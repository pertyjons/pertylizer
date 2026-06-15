//! The engine-facing binding layer between a [`CompiledScript`] and the voice.
//!
//! A YAMS program reads *bound sources* (module outputs/params, macros) and
//! *context vars* (gate, age, control rate). The bytecode only knows them as
//! numbered source registers; this module names what each register reads so the
//! voice can fill them. [`BoundScript`] is the immutable, `Arc`-shared half the
//! audio thread runs; the per-voice mutable [`super::RegisterFile`] is separate
//! (decision #4 — the script is never deep-cloned onto a `Copy` routing).

use crate::SrcAddr;
use crate::script::CompiledScript;

/// A per-voice context value the engine supplies each control block — the
/// real-time mirror of the compiler's `Context` symbol. Kept out of `SrcAddr`
/// because these have no module address; the voice provides them directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScriptContext {
    /// `1.0` while the note is held, else `0.0`.
    Gate,
    /// `1.0` only on the block the note started, else `0.0`.
    GateOn,
    /// Seconds since note-on.
    Age,
    /// Control rate in Hz.
    Sr,
}

/// One source register a [`BoundScript`] reads, in register order: the voice
/// resolves `inputs[i]` into the script's `sources[i]` before each `eval`.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptInput {
    /// A module output/param or macro, resolved exactly like a scalar routing's
    /// source. A dangling address reads `0.0` (disable-and-keep, decision #3).
    Source(SrcAddr),
    /// A per-voice context value supplied directly by the engine.
    Context(ScriptContext),
    /// An unresolvable binding (e.g. an unknown module prefix) — always `0.0`.
    /// Keeps the register slot so later indices stay aligned with the bytecode.
    Zero,
}

/// A compiled YAMS program paired with the addresses its source registers read.
///
/// Shared immutably across voices behind an `Arc`; the mutable per-voice state
/// (`lag`/`accum`/PRNG cells) lives in a separate [`super::RegisterFile`] sized
/// from `script.state_count()`. `source` is the canonical YAMS text, retained
/// for persistence (compiled on load) and inspection — never read on the audio
/// thread.
#[derive(Debug, Clone)]
pub struct BoundScript {
    /// The real-time bytecode program.
    pub script: CompiledScript,
    /// What each source register reads, indexed by register (`inputs.len()`
    /// equals `script.source_count()`).
    pub inputs: Vec<ScriptInput>,
    /// Canonical YAMS source text (persistence + inspection only).
    pub source: String,
}

impl BoundScript {
    /// Assemble from already-compiled parts.
    #[must_use]
    pub fn new(script: CompiledScript, inputs: Vec<ScriptInput>, source: String) -> Self {
        Self {
            script,
            inputs,
            source,
        }
    }
}
