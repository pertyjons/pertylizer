//! The predefined-identifier and built-in-function tables.
//!
//! These define the single namespace shared by everything the author can name
//! (decision #5): macros, context vars, constants, and functions. A `src`/`let`
//! name that collides with any of them is a compile error ([`is_reserved`]).

use synth_core::script::{AudioInputChannel, Builtin};

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
    /// Control rate in Hz (`sample_rate / block_size`), *not* the audio sample
    /// rate. Drives time-based stateful math; supplied as an input rather than a
    /// constant because it depends on the device sample rate.
    Cr,
    /// Absolute transport position in beats (e.g. `sin(beat * tau)` is a
    /// tempo-locked sine). Grows unbounded while playing.
    Beat,
    /// Phase within the current bar, `0..1` (4/4). Wraps every bar.
    BarPhase,
    /// Transport tempo in BPM (e.g. `tempo / 60` is beats per second).
    Tempo,
    /// `1.0` while the transport is running, else `0.0`.
    Playing,
    /// `1.0` only at sample 0 of the note (audio-rate one-shot init). Resolved
    /// only in audio-rate scripts; not offered in the control-script picker.
    FirstSample,
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
    RandSmooth,
    Pulse,
}

impl Stateful {
    /// Number of persistent state cells this op needs, given how many arguments
    /// the call site passed. Most ops are fixed, but the synced overloads of
    /// `phasor`/`accum` allocate a second cell to remember the previous block's
    /// sync/reset value for edge detection (the 1-arg forms stay at one cell).
    #[must_use]
    pub fn state_cells(self, n_args: usize) -> u16 {
        match self {
            Self::Sah | Self::Counter => 2,
            // Synced overload (rate/x + sync/reset) needs phase/sum + prev-trig.
            Self::Accum | Self::Phasor => {
                if n_args >= 2 {
                    2
                } else {
                    1
                }
            }
            // `pulse` desugars to an `edge(...)` over the transport, one cell.
            Self::Lag | Self::Slew | Self::Delta | Self::Edge | Self::Pulse => 1,
            // Interpolated value noise: phase + segment start + segment target.
            Self::RandSmooth => 3,
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

/// Every macro identifier paired with a friendly label, for editor "select
/// input" pickers. Kept beside [`macro_from_name`] so a new macro is added in
/// both places — `macro_catalog_resolves` asserts every entry resolves.
pub const MACRO_CATALOG: &[(&str, &str)] = &[
    ("velocity", "Velocity"),
    ("mod_wheel", "Mod Wheel"),
    ("aftertouch", "Aftertouch"),
    ("pitch_bend", "Pitch Bend"),
    ("note", "Note number"),
    ("poly_at", "Poly Aftertouch"),
];

/// Every context-variable identifier paired with a friendly label, for editor
/// "select input" pickers. Kept beside [`context_from_name`] — see
/// `context_catalog_resolves`.
pub const CONTEXT_CATALOG: &[(&str, &str)] = &[
    ("gate", "Gate (0/1)"),
    ("gate_on", "Gate note-on edge"),
    ("age", "Voice age (s)"),
    ("cr", "Control rate (Hz)"),
    ("beat", "Transport beat"),
    ("bar_phase", "Bar phase (0..1)"),
    ("tempo", "Tempo (BPM)"),
    ("playing", "Transport playing"),
];

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

/// Resolve an audio-input identifier to its channel. Audio-rate scripts only —
/// the compiler gates these behind [`CompileOptions::audio_rate`]; in a
/// control-rate script they are reserved but unresolvable. Mono `in` reads left.
#[must_use]
pub fn audio_in_channel(name: &str) -> Option<AudioInputChannel> {
    Some(match name {
        "in" | "in_l" => AudioInputChannel::Left,
        "in_r" => AudioInputChannel::Right,
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
        "cr" => Context::Cr,
        "beat" => Context::Beat,
        "bar_phase" => Context::BarPhase,
        "tempo" => Context::Tempo,
        "playing" => Context::Playing,
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
        "tanh" => stateless(B::Tanh),
        "lerp" | "mix" => stateless(B::Lerp),
        "smoothstep" => stateless(B::Smoothstep),
        "sigmoid" => stateless(B::Sigmoid),
        "gauss" => stateless(B::Gauss),
        "semis" => stateless(B::Semis),
        "mtof" => stateless(B::Mtof),
        "unipolar" => stateless(B::Unipolar),
        "bipolar" => stateless(B::Bipolar),
        "lag" => stateful(Stateful::Lag, 2, 2),
        "slew" => stateful(Stateful::Slew, 3, 3),
        "sah" => stateful(Stateful::Sah, 2, 2),
        "accum" => stateful(Stateful::Accum, 1, 2),
        "delta" => stateful(Stateful::Delta, 1, 1),
        "phasor" => stateful(Stateful::Phasor, 1, 2),
        "edge" => stateful(Stateful::Edge, 1, 1),
        "counter" => stateful(Stateful::Counter, 1, 1),
        "rand" => stateful(Stateful::Rand, 0, 2),
        "rand_smooth" => stateful(Stateful::RandSmooth, 1, 1),
        "pulse" => stateful(Stateful::Pulse, 1, 1),
        "white" => stateful(Stateful::White, 0, 0),
        _ => return None,
    })
}

/// Whether `name` is a built-in (function, macro, context var, or constant) or a
/// keyword — i.e. cannot be bound by `src`/`let` (decision #5).
#[must_use]
pub fn is_reserved(name: &str) -> bool {
    // `table_lin`/`scale_snap` take an array argument and are lowered to dedicated
    // opcodes in the compiler (not `resolve_fn`), so they are reserved by name.
    matches!(
        name,
        "src" | "let"
            | "out"
            | "arr"
            | "state"
            | "len"
            | "table_lin"
            | "scale_snap"
            // Audio-rate identifiers — reserved in every mode (resolved only when
            // `audio_rate`), so a control script cannot bind over them.
            | "in"
            | "in_l"
            | "in_r"
            | "first_sample"
    ) || resolve_fn(name).is_some()
        || macro_from_name(name).is_some()
        || context_from_name(name).is_some()
        || constant_value(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every label catalog entry must resolve through its `*_from_name`, so the
    /// editor pickers can never offer an identifier the compiler rejects.
    #[test]
    fn macro_catalog_resolves() {
        for (name, _label) in MACRO_CATALOG {
            assert!(macro_from_name(name).is_some(), "macro {name} unresolved");
        }
    }

    #[test]
    fn context_catalog_resolves() {
        for (name, _label) in CONTEXT_CATALOG {
            assert!(
                context_from_name(name).is_some(),
                "context {name} unresolved"
            );
        }
    }
}
