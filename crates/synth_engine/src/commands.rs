//! Commands that can be sent to the synth engine.
//!
//! These commands are sent from the UI thread to the audio thread
//! via a lock-free ring buffer.
//!
//! All commands use type-safe domain types (Gain, Bpm, NormalizedValue, etc.)
//! instead of primitive f32 values to prevent unit mismatches at compile time.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use super::instrument::{
    Instrument, InstrumentCategory, InstrumentId, KeyRange, LearnState, MidiChannel,
};
use synth_core::{
    Amplitude, BipolarValue, Bpm, CcNumber, Cents, CpuUsage, Gain, MidiNote, NormalizedValue,
    PortName, Seconds, Semitones, Velocity, VoiceCount,
};
use synth_core::{ModuleType, Param};
use synth_sequencer::{PatternId, ReturnBusId, SeqInstrumentId, Tick, TrackId};

/// Unique identifier for a module instance.
///
/// Format: `{type}-{instance}` e.g., "osc-1", "flt-2", "env-1"
///
/// The type prefix is a 3-letter code from ModuleType::prefix().
/// The instance number starts at 1 and is unique per type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId {
    pub module_type: ModuleType,
    pub instance: u16,
}

impl ModuleId {
    /// Master output module ID.
    pub const MASTER: Self = Self {
        module_type: ModuleType::Mixer,
        instance: 0,
    };

    /// Create a new module ID.
    pub fn new(module_type: ModuleType, instance: u16) -> Self {
        Self {
            module_type,
            instance,
        }
    }

    /// Get the type prefix (e.g., "osc", "flt").
    pub fn prefix(&self) -> &'static str {
        self.module_type.prefix()
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.module_type.prefix(), self.instance)
    }
}

impl FromStr for ModuleId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 2 {
            return Err(format!("Invalid ModuleId format: {}", s));
        }

        let module_type = ModuleType::from_prefix(parts[0])
            .ok_or_else(|| format!("Unknown module type prefix: {}", parts[0]))?;
        let instance = parts[1]
            .parse::<u16>()
            .map_err(|_| format!("Invalid instance number: {}", parts[1]))?;

        Ok(Self {
            module_type,
            instance,
        })
    }
}

// Custom serialization as string "osc-1"
impl Serialize for ModuleId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ModuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<Self>().map_err(serde::de::Error::custom)
    }
}

/// Identifies a module within a voice.
/// Used for type-safe parameter routing without magic numbers.
///
/// Each variant has a unique numeric ID and descriptive string name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PolyModule {
    Oscillator1,
    Oscillator2,
    Filter,
    AmpEnvelope,
    FilterEnvelope,
    Lfo,
    Amplifier,
    Mixer,
}

impl PolyModule {
    /// Get the module info: (module_type, instance, internal_name)
    pub const fn info(&self) -> (ModuleType, u16, &'static str) {
        match self {
            Self::Oscillator1 => (ModuleType::Oscillator, 1, "osc1"),
            Self::Oscillator2 => (ModuleType::Oscillator, 2, "osc2"),
            Self::Filter => (ModuleType::Filter, 1, "filter"),
            Self::AmpEnvelope => (ModuleType::Envelope, 1, "amp_env"),
            Self::FilterEnvelope => (ModuleType::Envelope, 2, "filter_env"),
            Self::Lfo => (ModuleType::Lfo, 1, "lfo"),
            Self::Amplifier => (ModuleType::Amplifier, 1, "amp"),
            Self::Mixer => (ModuleType::Mixer, 1, "mixer"),
        }
    }

    /// Get the descriptive string ID (e.g., "osc-1").
    pub fn id(&self) -> String {
        self.module_id().to_string()
    }

    /// Get the internal module name used by Voice (e.g., "osc1").
    pub const fn internal_name(&self) -> &'static str {
        self.info().2
    }

    /// Get the GUI ModuleId for this voice module.
    pub const fn module_id(&self) -> ModuleId {
        let (module_type, instance, _) = self.info();
        ModuleId {
            module_type,
            instance,
        }
    }

    /// Try to get PolyModule from a ModuleId.
    pub fn from_module_id(id: ModuleId) -> Option<Self> {
        Self::ALL.iter().find(|v| v.module_id() == id).copied()
    }

    /// All voice modules.
    pub const ALL: &[Self] = &[
        Self::Oscillator1,
        Self::Oscillator2,
        Self::Filter,
        Self::AmpEnvelope,
        Self::FilterEnvelope,
        Self::Lfo,
        Self::Amplifier,
        Self::Mixer,
    ];
}

/// Unique identifier for a module type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModuleTypeId(pub String);

impl ModuleTypeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Well-known module type IDs.
    pub const OSCILLATOR: &'static str = "oscillator";
    pub const FILTER: &'static str = "filter";
    pub const ENVELOPE: &'static str = "envelope";
    pub const LFO: &'static str = "lfo";
    pub const AMPLIFIER: &'static str = "amplifier";
    pub const MIXER: &'static str = "mixer";
    pub const DELAY: &'static str = "delay";
    pub const REVERB: &'static str = "reverb";
    pub const CHORUS: &'static str = "chorus";
    pub const DISTORTION: &'static str = "distortion";

    /// Create a module instance from this type ID.
    pub fn create_module(&self) -> Option<Box<dyn synth_core::PolyModule>> {
        use synth_modules::{Amplifier, Envelope, Filter, Lfo, Mixer, Oscillator};

        match self.0.as_str() {
            Self::OSCILLATOR => Some(Box::new(Oscillator::new())),
            Self::FILTER => Some(Box::new(Filter::new())),
            Self::ENVELOPE => Some(Box::new(Envelope::new())),
            Self::LFO => Some(Box::new(Lfo::new())),
            Self::AMPLIFIER => Some(Box::new(Amplifier::new())),
            Self::MIXER => Some(Box::new(Mixer::new())),
            _ => None,
        }
    }
}

impl<S: AsRef<str>> From<S> for ModuleTypeId {
    fn from(s: S) -> Self {
        Self(s.as_ref().to_string())
    }
}

/// Connection point identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortId {
    pub module: ModuleId,
    pub port: PortName,
}

impl PortId {
    pub fn new(module: ModuleId, port: impl Into<PortName>) -> Self {
        Self {
            module,
            port: port.into(),
        }
    }
}

/// Commands sent from UI to the audio engine.
///
/// These commands are processed in the audio callback, so they must
/// be designed to be handled without blocking.
pub enum EngineCommand {
    // === Instrument management ===
    /// Add a new instrument (pre-created in GUI thread for real-time safety).
    AddInstrument { instrument: Box<Instrument> },

    /// Remove an instrument by ID.
    RemoveInstrument { instrument_id: InstrumentId },

    /// Rename an instrument.
    RenameInstrument {
        instrument_id: InstrumentId,
        name: String,
    },

    /// Set the free-text description / intent on an instrument. Never
    /// affects audio; surfaces in `InstrumentSnapshot` for MCP/GUI reads.
    SetInstrumentDescription {
        instrument_id: InstrumentId,
        description: String,
    },

    /// Set or clear the patch-level description on an instrument. This
    /// describes the *patch* (sound design intent) rather than the
    /// instrument's per-instance role — both are kept separately so a
    /// patch can be shared/exported with its own narrative.
    SetPatchDescription {
        instrument_id: InstrumentId,
        description: Option<String>,
    },

    /// Set or clear the accent color on an instrument. Never affects audio;
    /// surfaces in `InstrumentSnapshot` for MCP/GUI reads and is persisted into
    /// `InstrumentState.color`. `None` clears back to "auto" / default.
    SetInstrumentColor {
        instrument_id: InstrumentId,
        color: Option<String>,
    },

    /// Set or clear the patch-level accent color on an instrument's currently
    /// loaded patch (distinct from `SetInstrumentColor`). Travels with the
    /// patch when saved; surfaces in `InstrumentSnapshot.patch_color`. `None`
    /// clears.
    SetPatchColor {
        instrument_id: InstrumentId,
        color: Option<String>,
    },

    /// Set or clear the free-text description on a specific module instance
    /// within an instrument's patch. Never affects audio; surfaces in
    /// `ModuleStateSnapshot` for MCP/GUI reads and persists with the patch.
    /// `description = None` (or empty) clears it.
    SetModuleDescription {
        instrument_id: InstrumentId,
        module_id: ModuleId,
        description: Option<String>,
    },

    /// Set or clear the sidechain source for an instrument. When set,
    /// the engine routes the source instrument's output into this
    /// instrument's compressors / sidechain-capable modules with
    /// previous-callback semantics. `source = None` disables.
    SetSidechainSource {
        instrument_id: InstrumentId,
        source: Option<InstrumentId>,
    },

    /// Set a parameter on a specific instrument.
    SetInstrumentParameter {
        instrument_id: InstrumentId,
        param: InstrumentParam,
    },

    /// Set the MIDI channel for an instrument.
    SetInstrumentMidiChannel {
        instrument_id: InstrumentId,
        channel: MidiChannel,
    },

    /// Enable or disable an instrument (mute).
    SetInstrumentEnabled {
        instrument_id: InstrumentId,
        enabled: bool,
    },

    /// Set instrument category for visualization routing.
    SetInstrumentCategory {
        instrument_id: InstrumentId,
        category: InstrumentCategory,
    },

    /// Solo an instrument.
    /// When any instrument is soloed, only soloed instruments produce sound.
    SetInstrumentSolo {
        instrument_id: InstrumentId,
        solo: bool,
    },

    // === Return busses (effect sends) ===
    /// Create the engine-side runtime channel for a return bus. The bus's
    /// definition (name, fader) lives in the song; the engine reads the fader
    /// live each block. Re-using an existing id is a no-op (ids come from the
    /// song / project load).
    CreateReturnBus { id: ReturnBusId },

    /// Remove a return bus's runtime channel by id. Send taps that referenced
    /// it become inert (resolved to no destination on the next block).
    RemoveReturnBus { id: ReturnBusId },

    /// Remove *all* return-bus runtime channels (and their effect chains).
    /// Used to reset return state before (re)loading a project so a load
    /// starts from a clean slate rather than stacking onto leftover channels.
    ClearReturnBusses,

    /// Add a pre-created effect instance to a return bus's effect chain
    /// (real-time safe: the effect is built off the audio thread).
    AddReturnEffect {
        return_id: ReturnBusId,
        id: ModuleId,
        effect: Box<dyn synth_core::AudioEffect>,
    },

    /// Remove an effect from a return bus's effect chain.
    RemoveReturnEffect {
        return_id: ReturnBusId,
        id: ModuleId,
    },

    /// Set a parameter on an effect in a return bus's effect chain.
    SetReturnEffectParameter {
        return_id: ReturnBusId,
        module_id: ModuleId,
        param: Param,
    },

    /// Enable or bypass an effect in a return bus's effect chain.
    SetReturnEffectEnabled {
        return_id: ReturnBusId,
        module_id: ModuleId,
        enabled: bool,
    },

    /// Move an effect up or down within a return bus's effect chain.
    ReorderReturnEffect {
        return_id: ReturnBusId,
        module_id: ModuleId,
        direction: ReorderDirection,
    },

    /// Remove every master-bus effect. Used before (re)loading a project so the
    /// master chain starts empty rather than stacking onto the previous project's
    /// effects (mirrors `ClearReturnBusses`).
    ClearMasterEffects,

    // === Note control ===
    /// Start a note with type-safe velocity.
    NoteOn {
        note: MidiNote,
        /// Note velocity.
        velocity: Velocity,
        channel: MidiChannel,
    },

    /// Stop a note.
    NoteOff {
        note: MidiNote,
        channel: MidiChannel,
    },

    /// All notes off.
    AllNotesOff,

    /// Hard-reset the DSP state in the per-instrument signal path to silence
    /// **instantly** — every instrument's voices (envelopes/filters/oscillator
    /// phase), its effect chain, its oversampling downsamplers, plus the master
    /// effect chain, every return bus's effect chain, and the modular graph.
    /// Unlike [`Self::AllNotesOff`] (which only triggers the release phase and
    /// lets tails ring), this zeroes delay lines and reverb buffers so the next
    /// render starts from a clean slate — tail-proof isolation between offline
    /// renders. Real-time safe.
    ///
    /// Not reset (out of scope — not in the offline single-instrument render
    /// path): the one-block sidechain previous-output buffer (self-heals after
    /// one block).
    ResetDsp,

    /// Pitch bend (type-safe bipolar value).
    /// GUI/MIDI handler converts raw MIDI (0-16383) to BipolarValue (-1.0 to 1.0).
    PitchBend {
        value: BipolarValue,
        channel: MidiChannel,
    },

    /// Mod wheel (CC1) - type-safe normalized value.
    /// GUI/MIDI handler converts raw MIDI (0-127) to NormalizedValue (0.0 to 1.0).
    ModWheel {
        value: NormalizedValue,
        channel: MidiChannel,
    },

    /// A generic MIDI Control Change. Feeds the engine's live CC state that Mod
    /// Grid `MidiCc` sources read. `cc` is the 0..127 controller number; `value`
    /// is the normalized 0..1 amount. (CC1 also arrives as [`Self::ModWheel`] for
    /// the per-voice mod-wheel path; both update the CC state.)
    ControlChange {
        channel: MidiChannel,
        cc: u8,
        value: NormalizedValue,
    },

    /// Aftertouch (channel pressure) - type-safe normalized value.
    Aftertouch {
        value: NormalizedValue,
        channel: MidiChannel,
    },

    /// Per-note aftertouch (polyphonic aftertouch) - type-safe normalized value.
    PolyAftertouch {
        note: MidiNote,
        value: NormalizedValue,
        channel: MidiChannel,
    },

    // === Parameter control ===
    /// Set a voice module parameter using the type-safe API.
    /// Use PolyModule to identify which module within the voice to update.
    /// The Param contains both the parameter type and its value.
    /// Requires instrument_id to target the correct instrument's voice graph.
    SetVoiceParameter {
        instrument_id: InstrumentId,
        target: PolyModule,
        param: Param,
    },

    /// Set a parameter on a module.
    /// - `instrument_id: Some(id)` - Target an instrument's voice graph
    /// - `instrument_id: None` - Target the global module graph (master bus)
    SetModuleParameter {
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        param: Param,
    },

    /// Install (or clear, with `None`) a Mod Matrix slot's compiled YAMS
    /// control script (Step 2). Compiled off the audio thread (on load /
    /// authoring); the audio thread only stores the shared `Arc`, applying it to
    /// the module template and every voice. `slot` is the 0-based routing index.
    /// - `instrument_id: Some(id)` - target an instrument's voice graph
    /// - `instrument_id: None` - target the global module graph (master bus)
    SetModScript {
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        slot: u8,
        script: Option<std::sync::Arc<synth_core::script::BoundScript>>,
        /// A prebuilt fresh [`ModuleDescriptor`] to swap into the module's graph
        /// nodes (template + every voice), for a script module whose `param` set
        /// may have changed. Built off the audio thread (the descriptor allocates);
        /// the audio thread only shares it in per node (cheap `Arc` clone) and
        /// defers the replaced ones' drop. `None` for scripts that never change
        /// their descriptor (the Mod Matrix `scr` scripts).
        descriptor: Option<std::sync::Arc<synth_core::ModuleDescriptor>>,
    },

    // === Module control ===
    /// Add a pre-created module instance to a graph (real-time safe).
    /// - `instrument_id: Some(id)` - Add to an instrument's voice graph
    /// - `instrument_id: None` - Add to the global module graph (master bus)
    AddModuleInstance {
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        module: Box<dyn synth_core::PolyModule>,
    },

    /// Remove a module from a graph.
    /// - `instrument_id: Some(id)` - Remove from an instrument's voice graph
    /// - `instrument_id: None` - Remove from the global module graph
    RemoveModule {
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
    },

    /// Connect two ports.
    /// - `instrument_id: Some(id)` - Connect within an instrument's voice graph
    /// - `instrument_id: None` - Connect within the global module graph
    Connect {
        instrument_id: Option<InstrumentId>,
        from: PortId,
        to: PortId,
    },

    /// Disconnect two ports.
    /// - `instrument_id: Some(id)` - Disconnect within an instrument's voice graph
    /// - `instrument_id: None` - Disconnect within the global module graph
    Disconnect {
        instrument_id: Option<InstrumentId>,
        from: PortId,
        to: PortId,
    },

    /// Disconnect all connections from/to a module.
    /// - `instrument_id: Some(id)` - Disconnect within an instrument's voice graph
    /// - `instrument_id: None` - Disconnect within the global module graph
    DisconnectAll {
        instrument_id: Option<InstrumentId>,
        module: ModuleId,
    },

    // === Transport control ===
    /// Set tempo (type-safe BPM).
    SetTempo(Bpm),

    /// Start playback.
    Play,

    /// Stop playback.
    Stop,

    /// Pause playback.
    Pause,

    /// Reset to beginning.
    Rewind,

    /// Seek to a specific tick position during playback.
    Seek { tick: Tick },

    /// Set loop boundaries and enable/disable looping.
    SetLoop {
        start: Tick,
        end: Tick,
        enabled: bool,
    },

    /// Enable/disable song repeat (loop entire song from start to end).
    SetRepeat { enabled: bool },

    /// Play only the specified pattern in a loop.
    /// Finds pattern in arrangement, sets loop boundaries, and starts playing.
    /// `instrument` is used only when the pattern has no placement (orphan
    /// preview); placed patterns route through their track's instrument.
    PlayPattern {
        pattern_id: PatternId,
        instrument: SeqInstrumentId,
    },

    /// Start playback from the beginning of a specific pattern.
    /// Does not loop - plays through the entire song from that position.
    PlayFromPattern { pattern_id: PatternId },

    /// Solo a single pattern during playback (piano-roll preview): only
    /// notes from placements with this `pattern_id` are emitted. Pass
    /// `None` to clear and resume normal multi-pattern playback. The
    /// engine also clears this state automatically on global `Play`/`Stop`.
    SetSoloPattern(Option<PatternId>),

    /// Enable preview-mode for an orphan pattern (no arrangement placement).
    /// When `Some(id)` the engine bypasses arrangement playback entirely and
    /// schedules notes from this pattern using `pattern_tick = current_tick
    /// % pattern.length`. Mute/solo and track-instrument routing do not
    /// apply (no track is involved); notes play through the supplied
    /// `instrument`. Pass `None` to clear. Cleared automatically by global
    /// `Play` (unarmed) and `Stop`.
    SetPreviewPattern(Option<(PatternId, SeqInstrumentId)>),

    // === Engine control ===
    /// Reset the engine state.
    Reset,

    /// Clear all modules (for patch loading).
    ClearAllModules,

    /// Set master volume (type-safe gain).
    SetMasterVolume(Gain),

    /// Set glide/portamento time (type-safe seconds).
    SetGlideTime(Seconds),

    /// Set the focused instrument for keyboard/MIDI input.
    /// When set, keyboard input goes only to this instrument, ignoring MIDI channels.
    /// When None, traditional MIDI channel routing is used.
    SetFocusedInstrument(Option<InstrumentId>),

    /// Bypass a module in a specific instrument (or all instruments if `None`).
    SetBypass {
        instrument_id: Option<InstrumentId>,
        module: ModuleId,
        bypass: bool,
    },

    // === Visualizer control ===
    /// Add a visualizer module to an instrument's effect chain.
    /// The VisualizationBuffer is shared between engine and GUI via Arc.
    /// - `instrument_id: Some(id)` - Add to a specific instrument's effect chain
    /// - `instrument_id: None` - Target the global master effect chain
    AddVisualizer {
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        visualizer_type: VisualizerType,
        buffer: std::sync::Arc<crate::visualizers::VisualizationBuffer>,
    },

    /// Remove a visualizer module from an instrument's effect chain.
    /// - `instrument_id: Some(id)` - Remove from a specific instrument's effect chain
    /// - `instrument_id: None` - Target the global master effect chain
    RemoveVisualizer {
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
    },

    /// Add a pre-created effect instance to an instrument's effect chain (real-time safe).
    /// The effect is created in the GUI thread and sent via this command.
    /// - `instrument_id: Some(id)` - Add to a specific instrument's effect chain
    /// - `instrument_id: None` - Target the global master effect chain
    AddEffectInstance {
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        effect: Box<dyn synth_core::AudioEffect>,
    },

    /// Remove an effect from an instrument's effect chain.
    /// - `instrument_id: Some(id)` - Remove from a specific instrument's effect chain
    /// - `instrument_id: None` - Target the global master effect chain
    RemoveEffect {
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
    },

    /// Reorder an effect/visualizer in the chain (move up or down).
    /// - `instrument_id: Some(id)` - Target a specific instrument's effect chain
    /// - `instrument_id: None` - Target the global master effects
    ReorderEffect {
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        direction: ReorderDirection,
    },

    /// Reorder the entire effect chain to match a given module ID sequence.
    ///
    /// IDs listed in `order` are moved to the front in the given order; any
    /// slots not mentioned keep their relative order and follow afterwards.
    /// Unknown IDs in `order` are ignored. Used by patch/project load to
    /// restore explicit effect-chain order.
    SetEffectChainOrder {
        instrument_id: Option<InstrumentId>,
        order: Vec<ModuleId>,
    },

    /// Set an effect parameter using type-safe API.
    /// The Param contains both the parameter type and its value.
    /// - `instrument_id: Some(id)` - Target a specific instrument's effect chain
    /// - `instrument_id: None` - Target the global master effect chain
    ///
    /// The effect is identified by its `ModuleId` so that multiple effects
    /// of the same type in a single chain can be targeted independently.
    SetEffectParameter {
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        param: Param,
    },

    /// Enable or disable an effect.
    /// - `instrument_id: Some(id)` - Target a specific instrument's effect chain
    /// - `instrument_id: None` - Target the global master effect chain
    ///
    /// The effect is identified by its `ModuleId` so that multiple effects
    /// of the same type in a single chain can be targeted independently.
    SetEffectEnabled {
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        enabled: bool,
    },

    /// Set the song for the sequencer.
    ///
    /// The song is shared via `Arc<RwLock<Song>>` for thread-safe access.
    /// The sequencer will stop and reset when a new song is set.
    SetSong {
        song: std::sync::Arc<parking_lot::RwLock<synth_sequencer::Song>>,
    },

    /// Replace the running Mod Grid instances wholesale. Built off the audio
    /// thread (the modules allocate) from the current `Song` mod-graph pool and
    /// shipped pre-assembled; the audio thread only swaps the box in. Sent when
    /// `Song::mod_grid_generation` changes.
    SetModGrid {
        runtime: Box<crate::mod_grid::ModGridRuntime>,
    },

    // === Recording ===
    /// Arm recording for a specific pattern.
    ArmRecord {
        pattern_id: PatternId,
        track_id: TrackId,
        region_start: Tick,
        pattern_length: synth_sequencer::Duration,
        /// Ticks per bar for count-in calculation (avoids song lock on audio thread).
        ticks_per_bar: synth_sequencer::Duration,
        /// Quantization grid (Duration(0) = off, Duration(480) = 1/8, etc.).
        quantize_grid: synth_sequencer::Duration,
        /// Overdub mode: true = layer on existing notes, false = replace.
        overdub: bool,
    },

    /// Disarm recording (flushes any captured notes first).
    DisarmRecord,

    /// Enable or disable the metronome.
    SetMetronome(bool),

    /// Set metronome volume.
    SetMetronomeVolume(Gain),

    // === Audio Input ===
    /// Set the audio input consumer (from AudioInputManager's engine ring buffer).
    /// Sent once when input monitoring starts.
    SetAudioInputConsumer { consumer: ringbuf::HeapCons<f32> },

    /// Clear the audio input consumer (when monitoring stops).
    ClearAudioInputConsumer,

    // === Sample Loading ===
    /// Load sample audio data into a Sampler module.
    /// Sent by GUI when the user selects a sample from the dropdown.
    LoadSampleData {
        instrument_id: InstrumentId,
        module_id: ModuleId,
        data: std::sync::Arc<[f32]>,
        channels: synth_core::ChannelCount,
        frame_count: usize,
        root_note: synth_core::MidiNote,
    },
}

/// Direction for reordering an effect in the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReorderDirection {
    /// Move earlier in the chain (toward index 0 / top).
    Up,
    /// Move later in the chain (toward end / bottom).
    Down,
}

/// Type of visualizer to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerType {
    Oscilloscope,
    LevelMeter,
    SpectrumAnalyzer,
}

/// Type of effect module.
/// Used for type-safe effect parameter routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectType {
    Chorus,
    Delay,
    Reverb,
    Distortion,
    Phaser,
    Flanger,
    Compressor,
    Eq,
    Waveshaper,
    BbdDelay,
    MidSide,
    Limiter,
    Convolver,
    PhaseVocoder,
    FrequencyShifter,
    EnsembleChorus,
    ShimmerReverb,
    GranularFx,
    SpectralBlur,
    ModalResonator,
    ReverseGateReverb,
}

impl EffectType {
    /// Convert from typed_params::ModuleType if it's an effect.
    pub fn from_module_type(mt: synth_core::ModuleType) -> Option<Self> {
        use synth_core::ModuleType as MT;
        match mt {
            MT::Chorus => Some(Self::Chorus),
            MT::Delay => Some(Self::Delay),
            MT::Reverb => Some(Self::Reverb),
            MT::Distortion => Some(Self::Distortion),
            MT::Phaser => Some(Self::Phaser),
            MT::Flanger => Some(Self::Flanger),
            MT::Compressor => Some(Self::Compressor),
            MT::Eq => Some(Self::Eq),
            MT::Waveshaper => Some(Self::Waveshaper),
            MT::BbdDelay => Some(Self::BbdDelay),
            MT::MidSide => Some(Self::MidSide),
            MT::Limiter => Some(Self::Limiter),
            MT::Convolver => Some(Self::Convolver),
            MT::PhaseVocoder => Some(Self::PhaseVocoder),
            MT::FrequencyShifter => Some(Self::FrequencyShifter),
            MT::EnsembleChorus => Some(Self::EnsembleChorus),
            MT::ShimmerReverb => Some(Self::ShimmerReverb),
            MT::GranularFx => Some(Self::GranularFx),
            MT::SpectralBlur => Some(Self::SpectralBlur),
            MT::ModalResonator => Some(Self::ModalResonator),
            MT::ReverseGateReverb => Some(Self::ReverseGateReverb),
            _ => None,
        }
    }

    /// Convert to typed_params::ModuleType.
    pub fn to_module_type(self) -> synth_core::ModuleType {
        use synth_core::ModuleType as MT;
        match self {
            Self::Chorus => MT::Chorus,
            Self::Delay => MT::Delay,
            Self::Reverb => MT::Reverb,
            Self::Distortion => MT::Distortion,
            Self::Phaser => MT::Phaser,
            Self::Flanger => MT::Flanger,
            Self::Compressor => MT::Compressor,
            Self::Eq => MT::Eq,
            Self::Waveshaper => MT::Waveshaper,
            Self::BbdDelay => MT::BbdDelay,
            Self::MidSide => MT::MidSide,
            Self::Limiter => MT::Limiter,
            Self::Convolver => MT::Convolver,
            Self::PhaseVocoder => MT::PhaseVocoder,
            Self::FrequencyShifter => MT::FrequencyShifter,
            Self::EnsembleChorus => MT::EnsembleChorus,
            Self::ShimmerReverb => MT::ShimmerReverb,
            Self::GranularFx => MT::GranularFx,
            Self::SpectralBlur => MT::SpectralBlur,
            Self::ModalResonator => MT::ModalResonator,
            Self::ReverseGateReverb => MT::ReverseGateReverb,
        }
    }
}

/// Part-specific parameters that can be set via commands.
/// All parameters use type-safe domain types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstrumentParam {
    /// Instrument volume (type-safe gain).
    Volume(Gain),
    /// Instrument pan (type-safe bipolar value: -1.0 left to +1.0 right).
    Pan(BipolarValue),
    /// Solo state - when any instrument is soloed, only soloed instruments play.
    Solo(bool),
    /// Glide/portamento time (type-safe seconds).
    GlideTime(Seconds),
    /// Voice allocation mode.
    AllocationMode(super::voice_allocator::AllocationMode),
    /// Voice stealing strategy.
    StealingStrategy(super::voice_allocator::StealingStrategy),
    /// Unison detune spread (total cents across all `Unison`-mode voices).
    UnisonDetune(Cents),
    /// Unison stereo spread (0..1) for `Unison`-mode voices.
    UnisonSpread(NormalizedValue),
    /// Maximum polyphony for this instrument.
    MaxVoices(VoiceCount),
    /// Velocity to amplitude sensitivity (0 = constant, 1 = full dynamic).
    VelocityAmpSensitivity(NormalizedValue),
    /// Velocity to filter cutoff sensitivity (0 = none, 1 = full).
    VelocityFilterSensitivity(NormalizedValue),
    /// Key range for keyboard splitting.
    KeyRange(KeyRange),
    /// Transpose offset in semitones (-24 to +24).
    Transpose(Semitones),
    /// MIDI learn state machine.
    LearnState(LearnState),
    /// Oversampling factor (Off/2x/4x).
    OversamplingFactor(synth_dsp::OversamplingFactor),
}

/// Events sent from the audio engine to the UI.
///
/// These are for visualization (meters, waveforms) and state updates.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Peak meter update.
    PeakMeter { left: Amplitude, right: Amplitude },

    /// RMS meter update.
    RmsMeter { left: Amplitude, right: Amplitude },

    /// Voice activity (current active voice count, can be 0).
    VoiceCount(u32),

    /// Module parameter changed (echo back).
    /// The Param contains both the parameter type and its value.
    ParameterChanged { module: ModuleId, param: Param },

    /// CPU usage.
    CpuUsage(CpuUsage),

    /// Buffer underrun occurred.
    BufferUnderrun,

    /// Envelope stage changed.
    EnvelopeStage {
        module: ModuleId,
        stage: u8, // 0=idle, 1=attack, 2=decay, 3=sustain, 4=release
    },

    /// Waveform data for oscilloscope.
    WaveformData {
        /// Downsampled waveform data.
        data: Vec<f32>,
    },

    /// A note was successfully triggered by the engine.
    ///
    /// This event is sent whenever a note starts playing, regardless of
    /// the source (MIDI, sequencer, or UI). Use this to update the GUI
    /// keyboard to show which notes the engine is actually playing.
    NoteTriggered {
        /// MIDI note number.
        note: MidiNote,
        /// Note velocity.
        velocity: Velocity,
        /// MIDI channel the note was triggered on.
        channel: MidiChannel,
    },

    /// A note was released by the engine.
    ///
    /// This event is sent when a note stops playing (enters release phase).
    /// The note may still be audible during the release, but the key should
    /// no longer appear "pressed" in the GUI.
    NoteReleased {
        /// MIDI note number.
        note: MidiNote,
        /// MIDI channel the note was released on.
        channel: MidiChannel,
    },

    /// All notes were released (panic/all-notes-off).
    ///
    /// This is sent when AllNotesOff command is processed, allowing the
    /// GUI to clear all pressed keys at once.
    AllNotesReleased,

    /// Key range was learned from MIDI input.
    ///
    /// This event is sent when an instrument in learn mode receives a note
    /// and updates its key range. The GUI should update its state accordingly.
    KeyRangeLearned {
        /// The instrument that learned the key range.
        instrument_id: InstrumentId,
        /// The new key range.
        key_range: KeyRange,
        /// The new learn state (usually Idle or WaitingForHighNote).
        learn_state: LearnState,
    },

    /// Live preview of recording in progress.
    ///
    /// Sent each audio buffer callback during Capturing state.
    /// The UI renders these as orange preview notes in the piano roll.
    RecordingPreview {
        /// Completed notes recorded so far.
        completed: Vec<crate::recording::RecordedNote>,
        /// Currently held notes (pitch, start tick).
        held: Vec<(synth_sequencer::Pitch, synth_sequencer::PatternTick)>,
        /// Pattern length for wrapping calculations.
        pattern_length: synth_sequencer::Duration,
    },

    /// Recorded notes flushed from the audio thread.
    ///
    /// The UI thread should write these notes into the target pattern.
    /// This avoids taking RwLock::write() on the audio thread.
    RecordedNotesFlushed {
        /// Target pattern to write notes into.
        pattern_id: PatternId,
        /// The recorded notes.
        notes: Vec<crate::recording::RecordedNote>,
        /// Whether to layer on existing notes (true) or replace them (false).
        overdub: bool,
    },
}

/// Lightweight note event for OSC telemetry (audio thread → OSC thread).
///
/// This is `Copy` and small, suitable for lock-free ring buffer transport
/// without cloning expensive `EngineEvent` variants.
#[derive(Debug, Clone, Copy)]
pub enum NoteEvent {
    /// A note was triggered.
    On {
        note: MidiNote,
        velocity: Velocity,
        instrument_id: InstrumentId,
        category: InstrumentCategory,
    },
    /// A note was released.
    Off {
        note: MidiNote,
        instrument_id: InstrumentId,
        category: InstrumentCategory,
    },
    /// A MIDI controller change (CC1=mod wheel, CC128=pitch bend, CC129=aftertouch).
    Cc {
        /// MIDI CC number (0–127 standard, 128+ for pseudo-CCs).
        cc: CcNumber,
        /// Normalized value (0.0–1.0).
        value: NormalizedValue,
        channel: MidiChannel,
    },
}

// Manual Debug implementation because Box<dyn PolyModule> doesn't implement Debug
impl std::fmt::Debug for EngineCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Instrument commands
            Self::AddInstrument { instrument } => f
                .debug_struct("AddInstrument")
                .field("instrument_id", &instrument.id())
                .field("name", &instrument.name())
                .finish(),
            Self::RemoveInstrument { instrument_id } => f
                .debug_struct("RemoveInstrument")
                .field("instrument_id", instrument_id)
                .finish(),
            Self::RenameInstrument {
                instrument_id,
                name,
            } => f
                .debug_struct("RenameInstrument")
                .field("instrument_id", instrument_id)
                .field("name", name)
                .finish(),
            Self::SetInstrumentDescription {
                instrument_id,
                description,
            } => f
                .debug_struct("SetInstrumentDescription")
                .field("instrument_id", instrument_id)
                .field("description", description)
                .finish(),
            Self::SetPatchDescription {
                instrument_id,
                description,
            } => f
                .debug_struct("SetPatchDescription")
                .field("instrument_id", instrument_id)
                .field("description", description)
                .finish(),
            Self::SetInstrumentColor {
                instrument_id,
                color,
            } => f
                .debug_struct("SetInstrumentColor")
                .field("instrument_id", instrument_id)
                .field("color", color)
                .finish(),
            Self::SetPatchColor {
                instrument_id,
                color,
            } => f
                .debug_struct("SetPatchColor")
                .field("instrument_id", instrument_id)
                .field("color", color)
                .finish(),
            Self::SetModuleDescription {
                instrument_id,
                module_id,
                description,
            } => f
                .debug_struct("SetModuleDescription")
                .field("instrument_id", instrument_id)
                .field("module_id", module_id)
                .field("description", description)
                .finish(),
            Self::SetSidechainSource {
                instrument_id,
                source,
            } => f
                .debug_struct("SetSidechainSource")
                .field("instrument_id", instrument_id)
                .field("source", source)
                .finish(),
            Self::SetInstrumentParameter {
                instrument_id,
                param,
            } => f
                .debug_struct("SetInstrumentParameter")
                .field("instrument_id", instrument_id)
                .field("param", param)
                .finish(),
            Self::SetInstrumentMidiChannel {
                instrument_id,
                channel,
            } => f
                .debug_struct("SetInstrumentMidiChannel")
                .field("instrument_id", instrument_id)
                .field("channel", channel)
                .finish(),
            Self::SetInstrumentEnabled {
                instrument_id,
                enabled,
            } => f
                .debug_struct("SetInstrumentEnabled")
                .field("instrument_id", instrument_id)
                .field("enabled", enabled)
                .finish(),
            Self::SetInstrumentCategory {
                instrument_id,
                category,
            } => f
                .debug_struct("SetInstrumentCategory")
                .field("instrument_id", instrument_id)
                .field("category", category)
                .finish(),
            Self::SetInstrumentSolo {
                instrument_id,
                solo,
            } => f
                .debug_struct("SetInstrumentSolo")
                .field("instrument_id", instrument_id)
                .field("solo", solo)
                .finish(),
            // Note commands
            Self::NoteOn {
                note,
                velocity,
                channel,
            } => f
                .debug_struct("NoteOn")
                .field("note", note)
                .field("velocity", velocity)
                .field("channel", channel)
                .finish(),
            Self::NoteOff { note, channel } => f
                .debug_struct("NoteOff")
                .field("note", note)
                .field("channel", channel)
                .finish(),
            Self::AllNotesOff => write!(f, "AllNotesOff"),
            Self::ResetDsp => write!(f, "ResetDsp"),
            Self::PitchBend { value, channel } => f
                .debug_struct("PitchBend")
                .field("value", value)
                .field("channel", channel)
                .finish(),
            Self::ModWheel { value, channel } => f
                .debug_struct("ModWheel")
                .field("value", value)
                .field("channel", channel)
                .finish(),
            Self::ControlChange { channel, cc, value } => f
                .debug_struct("ControlChange")
                .field("channel", channel)
                .field("cc", cc)
                .field("value", value)
                .finish(),
            Self::Aftertouch { value, channel } => f
                .debug_struct("Aftertouch")
                .field("value", value)
                .field("channel", channel)
                .finish(),
            Self::PolyAftertouch {
                note,
                value,
                channel,
            } => f
                .debug_struct("PolyAftertouch")
                .field("note", note)
                .field("value", value)
                .field("channel", channel)
                .finish(),
            Self::SetVoiceParameter {
                instrument_id,
                target,
                param,
            } => f
                .debug_struct("SetVoiceParameter")
                .field("instrument_id", instrument_id)
                .field("target", target)
                .field("param", param)
                .finish(),
            Self::SetModuleParameter {
                instrument_id,
                module_id,
                param,
            } => f
                .debug_struct("SetModuleParameter")
                .field("instrument_id", instrument_id)
                .field("module_id", module_id)
                .field("param", param)
                .finish(),
            Self::SetModScript {
                instrument_id,
                module_id,
                slot,
                script,
                descriptor,
            } => f
                .debug_struct("SetModScript")
                .field("instrument_id", instrument_id)
                .field("module_id", module_id)
                .field("slot", slot)
                // Show the source text, not the bytecode.
                .field("script", &script.as_ref().map(|s| s.source.as_str()))
                .field("rebuilt_descriptor", &descriptor.is_some())
                .finish(),
            Self::AddModuleInstance {
                instrument_id, id, ..
            } => {
                // Can't debug the module itself, just show the ID
                f.debug_struct("AddModuleInstance")
                    .field("instrument_id", instrument_id)
                    .field("id", id)
                    .field("module", &"<dyn PolyModule>")
                    .finish()
            }
            Self::RemoveModule { instrument_id, id } => f
                .debug_struct("RemoveModule")
                .field("instrument_id", instrument_id)
                .field("id", id)
                .finish(),
            Self::Connect {
                instrument_id,
                from,
                to,
            } => f
                .debug_struct("Connect")
                .field("instrument_id", instrument_id)
                .field("from", from)
                .field("to", to)
                .finish(),
            Self::Disconnect {
                instrument_id,
                from,
                to,
            } => f
                .debug_struct("Disconnect")
                .field("instrument_id", instrument_id)
                .field("from", from)
                .field("to", to)
                .finish(),
            Self::DisconnectAll {
                instrument_id,
                module,
            } => f
                .debug_struct("DisconnectAll")
                .field("instrument_id", instrument_id)
                .field("module", module)
                .finish(),
            Self::SetTempo(bpm) => write!(f, "SetTempo({bpm})"),
            Self::Play => write!(f, "Play"),
            Self::Stop => write!(f, "Stop"),
            Self::Pause => write!(f, "Pause"),
            Self::Rewind => write!(f, "Rewind"),

            Self::Seek { tick } => write!(f, "Seek({tick:?})"),
            Self::SetLoop {
                start,
                end,
                enabled,
            } => f
                .debug_struct("SetLoop")
                .field("start", start)
                .field("end", end)
                .field("enabled", enabled)
                .finish(),
            Self::SetRepeat { enabled } => write!(f, "SetRepeat({enabled})"),
            Self::PlayPattern { pattern_id, .. } => write!(f, "PlayPattern({pattern_id:?})"),
            Self::PlayFromPattern { pattern_id } => write!(f, "PlayFromPattern({pattern_id:?})"),
            Self::SetSoloPattern(p) => write!(f, "SetSoloPattern({p:?})"),
            Self::SetPreviewPattern(p) => write!(f, "SetPreviewPattern({p:?})"),
            Self::Reset => write!(f, "Reset"),
            Self::ClearAllModules => write!(f, "ClearAllModules"),
            Self::SetMasterVolume(v) => write!(f, "SetMasterVolume({v})"),
            Self::SetGlideTime(t) => write!(f, "SetGlideTime({t})"),
            Self::SetFocusedInstrument(id) => write!(f, "SetFocusedInstrument({id:?})"),
            Self::SetBypass {
                instrument_id,
                module,
                bypass,
            } => f
                .debug_struct("SetBypass")
                .field("instrument_id", instrument_id)
                .field("module", module)
                .field("bypass", bypass)
                .finish(),
            Self::AddVisualizer {
                instrument_id,
                id,
                visualizer_type,
                ..
            } => f
                .debug_struct("AddVisualizer")
                .field("instrument_id", instrument_id)
                .field("id", id)
                .field("visualizer_type", visualizer_type)
                .finish(),
            Self::RemoveVisualizer { instrument_id, id } => f
                .debug_struct("RemoveVisualizer")
                .field("instrument_id", instrument_id)
                .field("id", id)
                .finish(),
            Self::AddEffectInstance {
                instrument_id, id, ..
            } => f
                .debug_struct("AddEffectInstance")
                .field("instrument_id", instrument_id)
                .field("id", id)
                .field("effect", &"<dyn AudioEffect>")
                .finish(),
            Self::RemoveEffect { instrument_id, id } => f
                .debug_struct("RemoveEffect")
                .field("instrument_id", instrument_id)
                .field("id", id)
                .finish(),
            Self::ReorderEffect {
                instrument_id,
                module_id,
                direction,
            } => f
                .debug_struct("ReorderEffect")
                .field("instrument_id", instrument_id)
                .field("module_id", module_id)
                .field("direction", direction)
                .finish(),
            Self::SetEffectChainOrder {
                instrument_id,
                order,
            } => f
                .debug_struct("SetEffectChainOrder")
                .field("instrument_id", instrument_id)
                .field("order", order)
                .finish(),
            Self::SetEffectParameter {
                instrument_id,
                module_id,
                param,
            } => f
                .debug_struct("SetEffectParameter")
                .field("instrument_id", instrument_id)
                .field("module_id", module_id)
                .field("param", param)
                .finish(),
            Self::SetEffectEnabled {
                instrument_id,
                module_id,
                enabled,
            } => f
                .debug_struct("SetEffectEnabled")
                .field("instrument_id", instrument_id)
                .field("module_id", module_id)
                .field("enabled", enabled)
                .finish(),
            Self::ArmRecord {
                pattern_id,
                track_id,
                region_start,
                pattern_length,
                ticks_per_bar,
                quantize_grid,
                overdub,
            } => f
                .debug_struct("ArmRecord")
                .field("pattern_id", pattern_id)
                .field("track_id", track_id)
                .field("region_start", region_start)
                .field("pattern_length", pattern_length)
                .field("ticks_per_bar", ticks_per_bar)
                .field("quantize_grid", quantize_grid)
                .field("overdub", overdub)
                .finish(),
            Self::DisarmRecord => write!(f, "DisarmRecord"),
            Self::SetMetronome(enabled) => write!(f, "SetMetronome({enabled})"),
            Self::SetMetronomeVolume(vol) => write!(f, "SetMetronomeVolume({vol})"),
            Self::SetSong { .. } => write!(f, "SetSong"),
            Self::SetModGrid { .. } => write!(f, "SetModGrid"),
            Self::SetAudioInputConsumer { .. } => f.debug_struct("SetAudioInputConsumer").finish(),
            Self::ClearAudioInputConsumer => write!(f, "ClearAudioInputConsumer"),
            Self::LoadSampleData {
                instrument_id,
                module_id,
                ..
            } => f
                .debug_struct("LoadSampleData")
                .field("instrument_id", instrument_id)
                .field("module_id", module_id)
                .finish(),
            Self::CreateReturnBus { id } => {
                f.debug_struct("CreateReturnBus").field("id", id).finish()
            }
            Self::RemoveReturnBus { id } => {
                f.debug_struct("RemoveReturnBus").field("id", id).finish()
            }
            Self::ClearReturnBusses => write!(f, "ClearReturnBusses"),
            Self::AddReturnEffect { return_id, id, .. } => f
                .debug_struct("AddReturnEffect")
                .field("return_id", return_id)
                .field("id", id)
                .finish(),
            Self::RemoveReturnEffect { return_id, id } => f
                .debug_struct("RemoveReturnEffect")
                .field("return_id", return_id)
                .field("id", id)
                .finish(),
            Self::SetReturnEffectParameter {
                return_id,
                module_id,
                param,
            } => f
                .debug_struct("SetReturnEffectParameter")
                .field("return_id", return_id)
                .field("module_id", module_id)
                .field("param", param)
                .finish(),
            Self::SetReturnEffectEnabled {
                return_id,
                module_id,
                enabled,
            } => f
                .debug_struct("SetReturnEffectEnabled")
                .field("return_id", return_id)
                .field("module_id", module_id)
                .field("enabled", enabled)
                .finish(),
            Self::ReorderReturnEffect {
                return_id,
                module_id,
                direction,
            } => f
                .debug_struct("ReorderReturnEffect")
                .field("return_id", return_id)
                .field("module_id", module_id)
                .field("direction", direction)
                .finish(),
            Self::ClearMasterEffects => write!(f, "ClearMasterEffects"),
        }
    }
}

// Both enums cross the UI↔audio thread boundary over `ringbuf` lock-free
// channels (see the command/event ring buffers in `handle`/`lib`). Ringbuf only
// requires `Send` when the producer and consumer are split across threads, and
// that bound is checked deep inside its generics — so a variant that
// accidentally captured a non-`Send` payload (an `Rc`, a `*mut`) would fail with
// an opaque error far from here. These compile-time asserts pin the invariant at
// the type definition and give a clear failure site instead. Zero runtime cost.
static_assertions::assert_impl_all!(EngineCommand: Send);
static_assertions::assert_impl_all!(EngineEvent: Send);
// `'static` too (no borrowed data): the channels outlive any single call, so a
// captured reference would be unsound. `assert_impl_all!` can't express a
// lifetime bound, so check it with a zero-cost monomorphized helper.
const _: () = {
    const fn assert_static<T: 'static>() {}
    assert_static::<EngineCommand>();
    assert_static::<EngineEvent>();
};
