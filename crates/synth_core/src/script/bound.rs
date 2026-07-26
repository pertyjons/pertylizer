//! The engine-facing binding layer between a [`CompiledScript`] and the voice.
//!
//! A YAMS program reads *bound sources* (module outputs/params, macros) and
//! *context vars* (gate, age, control rate). The bytecode only knows them as
//! numbered source registers; this module names what each register reads so the
//! voice can fill them. [`BoundScript`] is the immutable, `Arc`-shared half the
//! audio thread runs; the per-voice mutable [`super::RegisterFile`] is separate
//! (decision #4 — the script is never deep-cloned onto a `Copy` routing).

use crate::script::CompiledScript;
use crate::{PortName, SrcAddr};

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
    /// Audio sample rate in Hz (the device / render rate). Unlike [`Self::Cr`]
    /// this is the per-sample rate; `sr / cr` is the block size. Lets an
    /// audio-rate script be sample-rate-portable instead of hardcoding a rate.
    Sr,
    /// The voice's current playing frequency in Hz, including glide, pitch bend,
    /// and per-note vibrato (but not per-oscillator detune). Lets a scripted
    /// oscillator track the note faithfully — `phasor(note_hz)` — instead of
    /// `mtof(note)`, which sees only the raw note number.
    NoteHz,
    /// Absolute transport position in beats (grows unbounded while playing).
    Beat,
    /// Phase within the current bar, `0..1` (4/4).
    BarPhase,
    /// Transport tempo in BPM.
    Tempo,
    /// `1.0` while the transport is running, else `0.0`.
    Playing,
    /// `1.0` only at sample 0 of the note's first block, else `0.0`. Audio-rate
    /// one-shot init pulse (an `AudioScript` runs one eval per sample, so the
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

/// One incoming-note field a `note_event`-dialect script reads. The note-graph
/// consumer fills the matching source register from the note event before
/// [`CompiledScript::eval_note`](super::CompiledScript::eval_note); the mirror of
/// the compiler's `note_field` resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteField {
    /// `note_pitch` — the raw MIDI note number (0–127) of the source note,
    /// matching `mtof`'s convention.
    Pitch,
    /// `note_vel` — the source note's velocity, `0..1`.
    Vel,
    /// `note_dur` — the source note's duration in ticks. The consumer passes a
    /// `-1` sentinel for a note with no fixed duration ("plays until cut").
    Dur,
    /// `tick` — the current transport position in ticks.
    Tick,
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
    /// block-constant placeholder; the `AudioScript` module overwrites this
    /// register each sample from its input port (see
    /// [`AudioBindings`](super::AudioBindings)).
    AudioIn(AudioInputChannel),
    /// An incoming-note field (`note_event` scripts only): the source note's
    /// pitch/velocity/duration or the transport tick. Resolves to `0.0` as a
    /// placeholder; the note-graph consumer fills this register from the note
    /// event before [`eval_note`](super::CompiledScript::eval_note).
    NoteField(NoteField),
    /// A note-event `Value` modulation input `in1..in4` (`note_event` scripts
    /// only), indexed `0..3`. The consumer fills it from the module's matching
    /// `Value` input port.
    NoteInput(u8),
    /// A control-ports CV input `in1..in4` (the `Script` module only), indexed
    /// `0..3`. The voice fills it each block from the wired incoming graph
    /// connection to the matching `in{N}` port (previous-block value); an unwired
    /// port reads `0.0`. Distinct from [`Self::NoteInput`] so the two dialects
    /// resolve the same `in1..in4` token differently.
    ControlIn(u8),
    /// A user-declared `param` knob (`param drive = 0.5`), read in-script by its
    /// name. The voice fills this register each block from the module's stored
    /// (automated) knob value plus its accumulated mod-offset — a block constant,
    /// like a macro. The interned name matches the knob in the module's store.
    LocalParam(PortName),
    /// An unresolvable binding (e.g. an unknown module prefix) — always `0.0`.
    /// Keeps the register slot so later indices stay aligned with the bytecode.
    Zero,
}

/// One user-declared `param` knob from a script's header (`param drive = 0.5
/// [0, 1] "Drive" "tooltip"`). Rides immutably on the [`BoundScript`] — read
/// (never copied onto the audio thread) to build the module's descriptor and to
/// remap its fixed knob store on a script edit.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptParamDecl {
    /// Interned knob identifier — the in-script name, the persistence `type_id`,
    /// and the mod-matrix / cross-script address (`scr-1.drive`).
    pub name: PortName,
    /// Cached `name.as_str()` — a leaked `'static` from the intern pool, resolved
    /// once at compile time so the audio-thread mod-offset match is a lock-free
    /// `str` compare, never a per-block `PortName::as_str` read-lock.
    pub name_str: &'static str,
    /// Default value: the knob's initial position and its range default.
    pub default: f32,
    /// Range minimum (default `0.0` when no `[min, max]` is given).
    pub min: f32,
    /// Range maximum (default `1.0` when no `[min, max]` is given).
    pub max: f32,
    /// Optional display label (falls back to the identifier when absent).
    pub label: Option<String>,
    /// Optional tooltip shown on hover.
    pub tooltip: Option<String>,
    /// Display unit (drives the knob's value suffix); `None` when unset.
    pub unit: crate::ParameterUnit,
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
    /// User-declared `param` knobs (empty for a program with none). Read to build
    /// the host module's descriptor + knob store; never touched on the audio thread.
    pub params: Vec<ScriptParamDecl>,
}

impl BoundScript {
    /// Assemble from already-compiled parts (no declared params).
    #[must_use]
    pub fn new(script: CompiledScript, inputs: Vec<ScriptInput>, source: String) -> Self {
        Self {
            script,
            inputs,
            source,
            params: Vec::new(),
        }
    }

    /// Attach the program's declared `param` knobs (builder form so the many
    /// param-less `new` callers stay unchanged).
    #[must_use]
    pub fn with_params(mut self, params: Vec<ScriptParamDecl>) -> Self {
        self.params = params;
        self
    }
}
