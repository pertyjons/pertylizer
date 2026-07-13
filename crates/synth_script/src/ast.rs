//! Abstract syntax tree for YAMS.
//!
//! Produced by [`crate::parser`] from the token stream. Grouping parentheses
//! are *not* represented — precedence is encoded in the tree shape, so `(a + b)`
//! and the bare `a + b` (where context makes them equal) produce the same tree.
//! The formatter (decision #11) preserves author parentheses via the lossless
//! CST, not this AST.

pub use crate::lexer::DurationUnit;
use crate::span::Span;

/// A whole YAMS program. The **header** is declarations in any order: `src`
/// bindings, `arr` const tables, and `state` cells. The **body** is an ordered
/// list of statements (`let` locals and `s = expr` state assignments) whose
/// source order is significant for state — a `state` read returns the prior
/// value until its assignment runs. The program ends with a single `out`.
/// `output` is `None` when the source had no (valid) `out` statement — a
/// diagnostic was emitted in that case.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub bindings: Vec<Binding>,
    pub arrays: Vec<ArrayDecl>,
    pub states: Vec<StateDecl>,
    pub params: Vec<ParamDecl>,
    pub body: Vec<BodyStmt>,
    pub outputs: Vec<Output>,
}

/// An identifier with its source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// `src <alias> = <address>` — aliases a module member to a local name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub alias: Ident,
    pub address: ModuleRef,
    pub span: Span,
}

/// A module member address, e.g. `lfo-1.out` or `osc.detune`. `instance` is
/// `None` when the instance number is omitted (defaults to 1 at resolve time).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRef {
    pub module: String,
    pub instance: Option<u32>,
    pub member: String,
    pub span: Span,
}

/// `arr <name> = [ <expr>, … ]` — a read-only, compile-time-constant lookup
/// table. Each element must const-fold to an `f32`; the table is not a value (it
/// can only be indexed with `name[expr]` or measured with `len(name)`), so the
/// single-`f32` type system is preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayDecl {
    pub name: Ident,
    pub elements: Vec<Expr>,
    pub span: Span,
}

/// `let <name> = <expr>` — a named intermediate value.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub name: Ident,
    pub expr: Expr,
    pub span: Span,
}

/// `state <name> = <expr>` — declares a persistent per-voice state cell for
/// custom IIR/feedback. The initializer must be a compile-time constant; cells
/// reset to it on note-on (today only `0` is supported — seed a non-zero value
/// from `first_sample` in an audio-rate script).
#[derive(Debug, Clone, PartialEq)]
pub struct StateDecl {
    pub name: Ident,
    pub init: Expr,
    pub span: Span,
}

/// `param <name> = <default> [ [<min>, <max>] ] [ "<label>" [ "<tooltip>" ] ]` —
/// a user-facing knob (Script / AudioScript modules only). The default and
/// optional `[min, max]` bounds must const-fold; the two strings are optional
/// positional metadata (label, then tooltip). Read in-script by name.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: Ident,
    pub default: Expr,
    /// `Some((min, max))` when a `[min, max]` range is given, else `None`
    /// (defaults to `0..1`).
    pub range: Option<(Expr, Expr)>,
    pub label: Option<String>,
    pub tooltip: Option<String>,
    pub span: Span,
}

/// `<state-name> = <expr>` — write a declared `state` cell (compiles to
/// `Op::StoreState`). A body statement so its position is significant: the write
/// updates the cell for subsequent reads (the next sample, for an audio-rate IIR).
#[derive(Debug, Clone, PartialEq)]
pub struct Assign {
    pub name: Ident,
    pub expr: Expr,
    pub span: Span,
}

/// One ordered body statement: a `let` local or a `state` assignment. Order is
/// preserved by the compiler so straight-line program semantics give correct IIR.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyStmt {
    Local(Local),
    Assign(Assign),
}

/// Which output a statement writes. A bare `out` is `Mono` (duplicated to both
/// channels of a stereo audio script); `out.left` / `out.right` (audio-rate only)
/// are the stereo multi-out grammar; `out.pitch` / `out.vel` / `out.dur` /
/// `out.gate` (`note_event` only) are the note-event field grammar; `out1`..`out4`
/// (`Out(0..3)`, control-ports `Script` module only) are the numbered CV outputs
/// — a **bare** spelling like every other numbered port, not a dotted member. The
/// parser accepts every spelling regardless of dialect; the compiler validates
/// which are legal for the active dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutChannel {
    Mono,
    Left,
    Right,
    Pitch,
    Vel,
    Dur,
    Gate,
    /// A numbered CV output port `out1`..`out4`, 0-based slot `0..3`.
    Out(u8),
}

/// `out[.left|.right|.pitch|.vel|.dur|.gate] = <expr>` — one output statement. A
/// program has one mono `out`, or the `out.left`/`out.right` pair (audio-rate),
/// or one or more `out.<field>` note-event writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub channel: OutChannel,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `-x`
    Neg,
    /// `!x`
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

/// An expression node. `Error` is a recovery placeholder inserted where the
/// parser could not build a real node; the resolver skips it.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Numeric literal, carrying any duration unit for compile-time folding.
    Number {
        value: f64,
        unit: Option<DurationUnit>,
        span: Span,
    },
    /// A bare identifier — a `src` alias, a `let`, a macro/context var, or a
    /// constant. Resolved later.
    Var { name: String, span: Span },
    Unary {
        op: UnaryOp,
        rhs: Box<Expr>,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Ternary {
        cond: Box<Expr>,
        then: Box<Expr>,
        els: Box<Expr>,
        span: Span,
    },
    Call {
        name: String,
        args: Vec<Expr>,
        span: Span,
    },
    /// `name[index]` — index a const lookup table declared with `arr`. `name` is
    /// resolved to an array (not a scalar) by the compiler; the result is `f32`.
    Index {
        name: String,
        index: Box<Expr>,
        span: Span,
    },
    /// Parse-error placeholder.
    Error { span: Span },
}

impl Expr {
    /// The source span of this expression.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Number { span, .. }
            | Self::Var { span, .. }
            | Self::Unary { span, .. }
            | Self::Binary { span, .. }
            | Self::Ternary { span, .. }
            | Self::Call { span, .. }
            | Self::Index { span, .. }
            | Self::Error { span } => *span,
        }
    }
}
