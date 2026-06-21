//! Abstract syntax tree for YAMS.
//!
//! Produced by [`crate::parser`] from the token stream. Grouping parentheses
//! are *not* represented — precedence is encoded in the tree shape, so `(a + b)`
//! and the bare `a + b` (where context makes them equal) produce the same tree.
//! The formatter (decision #11) preserves author parentheses via the lossless
//! CST, not this AST.

pub use crate::lexer::DurationUnit;
use crate::span::Span;

/// A whole YAMS program: zero or more `src` bindings, zero or more `arr`
/// const-table declarations, zero or more `let` locals, and a single `out`.
/// `output` is `None` when the source had no (valid) `out` statement — a
/// diagnostic was emitted in that case.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub bindings: Vec<Binding>,
    pub arrays: Vec<ArrayDecl>,
    pub locals: Vec<Local>,
    pub output: Option<Output>,
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

/// `out [name] = <expr>` — the single output. `name` is reserved for the future
/// multi-output direction and is unused in v1.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: Option<Ident>,
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
