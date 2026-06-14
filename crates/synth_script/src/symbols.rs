//! The predefined-identifier and built-in-function tables.
//!
//! These define the single namespace shared by everything the author can name
//! (decision #5): macros, context vars, constants, and functions. A `src`/`let`
//! name that collides with any of them is a compile error ([`is_reserved`]).

use synth_core::script::Builtin;

/// A per-voice macro input — always in scope, never bound with `src`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Macro {
    Velocity,
    ModWheel,
    Aftertouch,
    PitchBend,
    Note,
    PolyAt,
}

/// A per-voice context input filled by the engine each block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    Gate,
    GateOn,
    Age,
    /// Control rate in Hz (drives time-based stateful math; supplied as an input
    /// rather than a constant because it depends on the device sample rate).
    Sr,
}

/// A stateful built-in that carries per-voice register state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stateful {
    Lag,
    Slew,
    Sah,
    Accum,
    Delta,
    Phasor,
    Edge,
    Counter,
    Rand,
    White,
}

impl Stateful {
    /// Number of persistent state cells this op needs.
    #[must_use]
    pub fn state_cells(self) -> u16 {
        match self {
            Self::Sah | Self::Counter => 2,
            Self::Lag | Self::Slew | Self::Accum | Self::Delta | Self::Phasor | Self::Edge => 1,
            Self::Rand | Self::White => 0,
        }
    }
}

/// What a function name resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnKind {
    Stateless(Builtin),
    Stateful(Stateful),
}

/// A resolved function and its accepted argument count (inclusive range).
#[derive(Debug, Clone, Copy)]
pub struct FnSpec {
    pub kind: FnKind,
    pub min_arity: usize,
    pub max_arity: usize,
}

impl FnSpec {
    const fn fixed(kind: FnKind, arity: usize) -> Self {
        Self {
            kind,
            min_arity: arity,
            max_arity: arity,
        }
    }
}

/// Resolve a macro name.
#[must_use]
pub fn macro_from_name(name: &str) -> Option<Macro> {
    Some(match name {
        "velocity" => Macro::Velocity,
        "mod_wheel" => Macro::ModWheel,
        "aftertouch" => Macro::Aftertouch,
        "pitch_bend" => Macro::PitchBend,
        "note" => Macro::Note,
        "poly_at" => Macro::PolyAt,
        _ => return None,
    })
}

/// Resolve a context-variable name.
#[must_use]
pub fn context_from_name(name: &str) -> Option<Context> {
    Some(match name {
        "gate" => Context::Gate,
        "gate_on" => Context::GateOn,
        "age" => Context::Age,
        "sr" => Context::Sr,
        _ => return None,
    })
}

/// The value of a compile-time constant (`pi`, `tau`, `e`).
#[must_use]
pub fn constant_value(name: &str) -> Option<f32> {
    Some(match name {
        "pi" => core::f32::consts::PI,
        "tau" => core::f32::consts::TAU,
        "e" => core::f32::consts::E,
        _ => return None,
    })
}

/// Resolve a function name to its kind and arity.
#[must_use]
pub fn resolve_fn(name: &str) -> Option<FnSpec> {
    use Builtin as B;
    let stateless = |b: Builtin| FnSpec::fixed(FnKind::Stateless(b), b.arity());
    let stateful = |s: Stateful, lo: usize, hi: usize| FnSpec {
        kind: FnKind::Stateful(s),
        min_arity: lo,
        max_arity: hi,
    };
    Some(match name {
        "abs" => stateless(B::Abs),
        "sign" => stateless(B::Sign),
        "min" => stateless(B::Min),
        "max" => stateless(B::Max),
        "clamp" => stateless(B::Clamp),
        "floor" => stateless(B::Floor),
        "ceil" => stateless(B::Ceil),
        "round" => stateless(B::Round),
        "trunc" => stateless(B::Trunc),
        "quantize" => stateless(B::Quantize),
        "pow" => stateless(B::Pow),
        "sqrt" => stateless(B::Sqrt),
        "exp" => stateless(B::Exp),
        "log" => stateless(B::Log),
        "sin" => stateless(B::Sin),
        "cos" => stateless(B::Cos),
        "tan" => stateless(B::Tan),
        "atan" => stateless(B::Atan),
        "atan2" => stateless(B::Atan2),
        "lerp" | "mix" => stateless(B::Lerp),
        "smoothstep" => stateless(B::Smoothstep),
        "sigmoid" => stateless(B::Sigmoid),
        "gauss" => stateless(B::Gauss),
        "semis" => stateless(B::Semis),
        "mtof" => stateless(B::Mtof),
        "lag" => stateful(Stateful::Lag, 2, 2),
        "slew" => stateful(Stateful::Slew, 3, 3),
        "sah" => stateful(Stateful::Sah, 2, 2),
        "accum" => stateful(Stateful::Accum, 1, 1),
        "delta" => stateful(Stateful::Delta, 1, 1),
        "phasor" => stateful(Stateful::Phasor, 1, 1),
        "edge" => stateful(Stateful::Edge, 1, 1),
        "counter" => stateful(Stateful::Counter, 1, 1),
        "rand" => stateful(Stateful::Rand, 0, 2),
        "white" => stateful(Stateful::White, 0, 0),
        _ => return None,
    })
}

/// Whether `name` is a built-in (function, macro, context var, or constant) or a
/// keyword — i.e. cannot be bound by `src`/`let` (decision #5).
#[must_use]
pub fn is_reserved(name: &str) -> bool {
    matches!(name, "src" | "let" | "out")
        || resolve_fn(name).is_some()
        || macro_from_name(name).is_some()
        || context_from_name(name).is_some()
        || constant_value(name).is_some()
}
