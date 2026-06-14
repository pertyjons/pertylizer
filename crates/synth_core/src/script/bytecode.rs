//! YAMS bytecode — the real-time instruction set and the immutable
//! [`CompiledScript`] the audio thread evaluates.
//!
//! A YAMS program compiles to a flat `Vec<Op>` for a **stack machine**: a
//! straight-line sequence (no loops, no recursion) over a fixed value stack, so
//! evaluation is a single `for` loop with O(1) call-stack depth (decision #8).
//! The non-RT compiler lives in `synth_script`; this crate holds only the type
//! the engine runs.

// ---- caps (decision #8; raisable later — only lowering is breaking) --------

/// Maximum bytecode length. With no loops/recursion this is the exact
/// worst-case instruction count, so the cap is a compile-time length gate.
pub const MAX_INSTRUCTIONS: usize = 256;
/// Maximum number of `src` source registers a script may read.
pub const MAX_SOURCES: usize = 32;
/// Maximum number of persistent per-voice state cells.
pub const MAX_STATE: usize = 16;
/// Maximum number of transient local (scratch) slots.
pub const MAX_LOCALS: usize = 16;
/// Maximum value-stack depth.
pub const MAX_STACK: usize = 64;
/// Maximum expression nesting depth (bounds the compiler's lowering recursion).
pub const MAX_NESTING_DEPTH: usize = 32;
/// Maximum source-text length in bytes.
pub const MAX_SOURCE_LEN: usize = 4096;

/// A built-in stateless function. Arity is fixed per function; the evaluator
/// pops that many operands (last argument on top) and pushes one result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Abs,
    Sign,
    Min,
    Max,
    Clamp,
    Floor,
    Ceil,
    Round,
    Trunc,
    Quantize,
    Pow,
    Sqrt,
    Exp,
    Log,
    Sin,
    Cos,
    Tan,
    Atan,
    Atan2,
    Lerp,
    Smoothstep,
    Sigmoid,
    Gauss,
    Semis,
    Mtof,
}

impl Builtin {
    /// Number of operands this function consumes from the stack.
    #[must_use]
    pub fn arity(self) -> usize {
        match self {
            Self::Abs
            | Self::Sign
            | Self::Floor
            | Self::Ceil
            | Self::Round
            | Self::Trunc
            | Self::Sqrt
            | Self::Exp
            | Self::Log
            | Self::Sin
            | Self::Cos
            | Self::Tan
            | Self::Atan
            | Self::Sigmoid
            | Self::Gauss
            | Self::Semis
            | Self::Mtof => 1,
            Self::Min | Self::Max | Self::Quantize | Self::Pow | Self::Atan2 => 2,
            Self::Clamp | Self::Lerp | Self::Smoothstep => 3,
        }
    }
}

/// One bytecode instruction. Operand-stack conventions are noted per variant;
/// "pop b, pop a" means `b` was on top (pushed last).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Push `constants[idx]`.
    PushConst(u16),
    /// Push the voice-provided source value `sources[idx]`.
    PushSource(u16),
    /// Push the transient local slot `locals[idx]`.
    LoadLocal(u16),
    /// Pop and store into the transient local slot `locals[idx]`.
    StoreLocal(u16),

    // Arithmetic — pop b, pop a, push (a op b).
    Add,
    Sub,
    Mul,
    /// Safe division: `a / 0 → 0`.
    Div,
    /// Safe remainder: `a % 0 → 0`.
    Rem,
    Pow,

    // Unary — pop a, push.
    Neg,
    Not,

    // Comparison / logic — pop b, pop a, push 1.0 / 0.0.
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,

    /// Ternary mux — pop els, pop then, pop cond; push `cond != 0 ? then : els`.
    /// Both arms are already evaluated (eager; decision: no short-circuit).
    Select,

    /// Apply a stateless built-in (pops `arity`, pushes one).
    Call(Builtin),

    // ---- stateful (the u16 is the base state-cell index) -----------------
    /// One-pole smoothing. Pop alpha, pop x. Coefficient `alpha` is supplied on
    /// the stack (precomputed constant, or computed from a dynamic time).
    Lag(u16),
    /// Slew limiter. Pop down, pop up, pop x (up/down in units per second).
    Slew(u16),
    /// Sample-and-hold on the rising edge of trig. Pop trig, pop x. Cells i, i+1.
    Sah(u16),
    /// Integrator. Pop x; state += x.
    Accum(u16),
    /// Change since the previous block. Pop x.
    Delta(u16),
    /// Own ramp `0→1` at the given rate (Hz). Pop rate.
    Phasor(u16),
    /// Rising-edge detector → 1.0 on a rising edge, else 0.0. Pop x.
    Edge(u16),
    /// Count rising edges of trig. Pop trig. Cells i, i+1.
    Counter(u16),
    /// Seeded uniform random in `[lo, hi)`. Pop hi, pop lo. Uses the per-voice PRNG.
    Rand,
}

/// An immutable, compiled YAMS program. Shared across voices behind an `Arc`;
/// per-voice mutable state lives in a separate [`super::RegisterFile`].
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledScript {
    pub(crate) code: Vec<Op>,
    pub(crate) constants: Vec<f32>,
    pub(crate) source_count: u16,
    pub(crate) state_count: u16,
}

impl CompiledScript {
    /// Build a compiled script from already-validated parts (the `synth_script`
    /// compiler enforces the caps before calling this).
    #[must_use]
    pub fn new(code: Vec<Op>, constants: Vec<f32>, source_count: u16, state_count: u16) -> Self {
        Self {
            code,
            constants,
            source_count,
            state_count,
        }
    }

    #[must_use]
    pub fn code(&self) -> &[Op] {
        &self.code
    }

    #[must_use]
    pub fn constants(&self) -> &[f32] {
        &self.constants
    }

    /// Number of source registers the voice must fill before [`Self::eval`].
    #[must_use]
    pub fn source_count(&self) -> u16 {
        self.source_count
    }

    /// Number of per-voice state cells this script uses.
    #[must_use]
    pub fn state_count(&self) -> u16 {
        self.state_count
    }
}
