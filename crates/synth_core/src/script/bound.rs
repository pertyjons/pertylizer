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
    /// Control rate in Hz (`sample_rate / block_size`), *not* the audio sample
    /// rate.
    Cr,
    /// Absolute transport position in beats (grows unbounded while playing).
    Beat,
    /// Phase within the current bar, `0..1` (4/4).
    BarPhase,
    /// Transport tempo in BPM.
    Tempo,
    /// `1.0` while the transport is running, else `0.0`.
    Playing,
    /// `1.0` only at sample 0 of the note's first block, else `0.0`. Audio-rate
    /// one-shot init pulse (an [`AudioScript`] runs one eval per sample, so the
    /// per-block `gate_on` would read `1` for the whole first block). Injected
    /// per-sample by the audio module; reads `0.0` at control rate.
    FirstSample,
}

/// Which audio input channel a per-sample source register reads. Mono `in`
/// aliases the left channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioInputChannel {
    /// `in` / `in_l` — the left (or mono) audio input.
    Left,
    /// `in_r` — the right audio input.
    Right,
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
    /// A per-sample audio input (audio-rate scripts only). Resolves to `0.0` as a
    /// block-constant placeholder; the [`AudioScript`] module overwrites this
    /// register each sample from its input port (see
    /// [`AudioBindings`](super::AudioBindings)).
    AudioIn(AudioInputChannel),
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
