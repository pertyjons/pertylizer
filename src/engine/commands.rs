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

use super::instrument::{MidiChannel, InstrumentId, Instrument};
use super::typed_params::{ModuleType, Param};
use crate::types::{BipolarValue, Bpm, Gain, NormalizedValue, Seconds};

/// Unique identifier for a module instance.
///
/// Format: `{type}-{instance}` e.g., "osc-1", "flt-2", "env-1"
///
/// The type prefix is a 3-letter code from ModuleType::prefix().
/// The instance number starts at 1 and is unique per type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId {
    pub module_type: ModuleType,
    pub instance: u16,
}

impl ModuleId {
    /// Master output module ID.
    pub const MASTER: Self = Self { module_type: ModuleType::Mixer, instance: 0 };

    /// Create a new module ID.
    pub fn new(module_type: ModuleType, instance: u16) -> Self {
        Self { module_type, instance }
    }

    /// Create from legacy u32 format (for backwards compatibility).
    /// Maps old numeric IDs to typed IDs based on ranges:
    /// - 0: Master (mix-0)
    /// - 1-99: Oscillators
    /// - 100-199: Effects
    /// - 200-299: Visualizers
    /// - 300+: Outputs
    #[deprecated(note = "Use ModuleId::new(type, instance) instead")]
    pub fn from_legacy(id: u32) -> Self {
        match id {
            0 => Self::MASTER,
            1..=99 => Self::new(ModuleType::Oscillator, id as u16),
            100..=199 => Self::new(ModuleType::Delay, (id - 100) as u16),
            200..=299 => Self::new(ModuleType::Oscilloscope, (id - 200) as u16),
            300..=399 => Self::new(ModuleType::StereoOutput, (id - 300) as u16),
            _ => Self::new(ModuleType::Mixer, id as u16),
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
        let instance = parts[1].parse::<u16>()
            .map_err(|_| format!("Invalid instance number: {}", parts[1]))?;

        Ok(Self { module_type, instance })
    }
}

// Custom serialization as string "osc-1"
impl Serialize for ModuleId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ModuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let s = String::deserialize(deserializer)?;

        // Try new format first "osc-1"
        if let Ok(id) = s.parse::<ModuleId>() {
            return Ok(id);
        }

        // Fall back to legacy u32 format for old patches
        if let Ok(num) = s.parse::<u32>() {
            #[allow(deprecated)]
            return Ok(ModuleId::from_legacy(num));
        }

        Err(serde::de::Error::custom(format!("Invalid ModuleId: {}", s)))
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
            Self::Oscillator1    => (ModuleType::Oscillator, 1, "osc1"),
            Self::Oscillator2    => (ModuleType::Oscillator, 2, "osc2"),
            Self::Filter         => (ModuleType::Filter, 1, "filter"),
            Self::AmpEnvelope    => (ModuleType::Envelope, 1, "amp_env"),
            Self::FilterEnvelope => (ModuleType::Envelope, 2, "filter_env"),
            Self::Lfo            => (ModuleType::Lfo, 1, "lfo"),
            Self::Amplifier      => (ModuleType::Amplifier, 1, "amp"),
            Self::Mixer          => (ModuleType::Mixer, 1, "mixer"),
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
        ModuleId { module_type, instance }
    }

    /// Try to get PolyModule from a ModuleId.
    pub fn from_module_id(id: ModuleId) -> Option<Self> {
        Self::ALL.iter().find(|v| v.module_id() == id).copied()
    }
    
    /// All voice modules.
    pub const ALL: &'static [PolyModule] = &[
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
    pub fn create_module(&self) -> Option<Box<dyn crate::modules::PolyModule>> {
        use crate::modules::{Oscillator, Filter, Envelope, Lfo, Amplifier, Mixer};
        
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PortId {
    pub module: ModuleId,
    pub port: String,
}

impl PortId {
    pub fn new(module: ModuleId, port: impl Into<String>) -> Self {
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
    AddInstrument {
        instrument: Box<Instrument>,
    },

    /// Remove an instrument by ID.
    RemoveInstrument {
        instrument_id: InstrumentId,
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

    /// Enable or disable an instrument.
    SetInstrumentEnabled {
        instrument_id: InstrumentId,
        enabled: bool,
    },

    // === Note control ===
    /// Start a note with type-safe velocity.
    NoteOn {
        note: u8,
        /// Velocity as normalized value [0.0, 1.0].
        velocity: NormalizedValue,
        channel: MidiChannel,
    },

    /// Stop a note.
    NoteOff {
        note: u8,
        channel: MidiChannel,
    },

    /// All notes off.
    AllNotesOff,

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

    /// Aftertouch (channel pressure) - type-safe normalized value.
    Aftertouch {
        value: NormalizedValue,
        channel: MidiChannel,
    },

    /// Per-note aftertouch (polyphonic aftertouch) - type-safe normalized value.
    PolyAftertouch {
        note: u8,
        value: NormalizedValue,
        channel: MidiChannel,
    },

    // === Parameter control ===
    /// Set a voice module parameter using the type-safe API.
    /// Use PolyModule to identify which module within the voice to update.
    /// The Param contains both the parameter type and its value.
    SetVoiceParameter {
        target: PolyModule,
        param: Param,
    },

    /// Set a parameter on a module in the global graph.
    /// The Param contains both the parameter type and its value.
    SetModuleParameter {
        module_id: ModuleId,
        param: Param,
    },

    // === Module control ===
    /// Add a pre-created module instance to the graph (real-time safe).
    /// The module is created in the GUI thread and sent via this command.
    AddModuleInstance {
        id: ModuleId,
        module: Box<dyn crate::modules::PolyModule>,
    },

    /// Remove a module from the graph.
    RemoveModule {
        id: ModuleId,
    },

    /// Connect two ports.
    Connect {
        from: PortId,
        to: PortId,
    },

    /// Disconnect two ports.
    Disconnect {
        from: PortId,
        to: PortId,
    },

    /// Disconnect all connections from/to a module.
    DisconnectAll {
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

    // === Engine control ===
    /// Reset the engine state.
    Reset,

    /// Clear all modules (for patch loading).
    ClearAllModules,

    /// Set master volume (type-safe gain).
    SetMasterVolume(Gain),

    /// Set glide/portamento time (type-safe seconds).
    SetGlideTime(Seconds),

    /// Bypass a module.
    SetBypass {
        module: ModuleId,
        bypass: bool,
    },

    // === Visualizer control ===
    /// Add a visualizer module.
    /// The VisualizationBuffer is shared between engine and GUI via Arc.
    AddVisualizer {
        id: ModuleId,
        visualizer_type: VisualizerType,
        buffer: std::sync::Arc<crate::visualizers::VisualizationBuffer>,
    },

    /// Remove a visualizer module.
    RemoveVisualizer {
        id: ModuleId,
    },

    /// Add a pre-created effect instance to the effect chain (real-time safe).
    /// The effect is created in the GUI thread and sent via this command.
    AddEffectInstance {
        id: ModuleId,
        effect: Box<dyn crate::modules::AudioEffect>,
    },
    
    /// Remove an effect from the effect chain.
    RemoveEffect {
        id: ModuleId,
    },
    
    /// Set an effect parameter using type-safe API.
    /// The Param contains both the parameter type and its value.
    SetEffectParameter {
        effect_type: EffectType,
        param: Param,
    },
    
    /// Enable or disable an effect.
    SetEffectEnabled {
        effect_type: EffectType,
        enabled: bool,
    },
}

/// Type of visualizer to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerType {
    Oscilloscope,
    LevelMeter,
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
}

impl EffectType {
    /// Convert from typed_params::ModuleType if it's an effect.
    pub fn from_module_type(mt: super::typed_params::ModuleType) -> Option<Self> {
        use super::typed_params::ModuleType as MT;
        match mt {
            MT::Chorus => Some(Self::Chorus),
            MT::Delay => Some(Self::Delay),
            MT::Reverb => Some(Self::Reverb),
            MT::Distortion => Some(Self::Distortion),
            MT::Phaser => Some(Self::Phaser),
            MT::Flanger => Some(Self::Flanger),
            MT::Compressor => Some(Self::Compressor),
            MT::Eq => Some(Self::Eq),
            _ => None,
        }
    }
    
    /// Convert from patch::ModuleType if it's an effect.
    pub fn from_patch_module_type(mt: crate::patch::ModuleType) -> Option<Self> {
        use crate::patch::ModuleType as PMT;
        match mt {
            PMT::Chorus => Some(Self::Chorus),
            PMT::Delay => Some(Self::Delay),
            PMT::Reverb => Some(Self::Reverb),
            PMT::Distortion => Some(Self::Distortion),
            _ => None,
        }
    }
    
    /// Convert to typed_params::ModuleType.
    pub fn to_module_type(self) -> super::typed_params::ModuleType {
        use super::typed_params::ModuleType as MT;
        match self {
            Self::Chorus => MT::Chorus,
            Self::Delay => MT::Delay,
            Self::Reverb => MT::Reverb,
            Self::Distortion => MT::Distortion,
            Self::Phaser => MT::Phaser,
            Self::Flanger => MT::Flanger,
            Self::Compressor => MT::Compressor,
            Self::Eq => MT::Eq,
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
    /// Glide/portamento time (type-safe seconds).
    GlideTime(Seconds),
    /// Voice allocation mode.
    AllocationMode(super::voice_allocator::AllocationMode),
    /// Voice stealing strategy.
    StealingStrategy(super::voice_allocator::StealingStrategy),
    /// Maximum polyphony for this instrument.
    MaxVoices(usize),
    /// Velocity to amplitude sensitivity (0 = constant, 1 = full dynamic).
    VelocityAmpSensitivity(NormalizedValue),
    /// Velocity to filter cutoff sensitivity (0 = none, 1 = full).
    VelocityFilterSensitivity(NormalizedValue),
}

/// Events sent from the audio engine to the UI.
///
/// These are for visualization (meters, waveforms) and state updates.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// Peak meter update.
    PeakMeter {
        left: f32,
        right: f32,
    },

    /// RMS meter update.
    RmsMeter {
        left: f32,
        right: f32,
    },

    /// Voice activity.
    VoiceCount(u32),

    /// Module parameter changed (echo back).
    /// The Param contains both the parameter type and its value.
    ParameterChanged {
        module: ModuleId,
        param: Param,
    },

    /// CPU usage.
    CpuUsage(f32),

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
        /// MIDI note number (0-127).
        note: u8,
        /// Velocity as normalized value (0.0-1.0).
        velocity: f32,
        /// MIDI channel the note was triggered on.
        channel: MidiChannel,
    },

    /// A note was released by the engine.
    ///
    /// This event is sent when a note stops playing (enters release phase).
    /// The note may still be audible during the release, but the key should
    /// no longer appear "pressed" in the GUI.
    NoteReleased {
        /// MIDI note number (0-127).
        note: u8,
        /// MIDI channel the note was released on.
        channel: MidiChannel,
    },

    /// All notes were released (panic/all-notes-off).
    ///
    /// This is sent when AllNotesOff command is processed, allowing the
    /// GUI to clear all pressed keys at once.
    AllNotesReleased,
}

// Manual Debug implementation because Box<dyn PolyModule> doesn't implement Debug
impl std::fmt::Debug for EngineCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Instrument commands
            Self::AddInstrument { instrument } => {
                f.debug_struct("AddInstrument")
                    .field("instrument_id", &instrument.id())
                    .field("name", &instrument.name())
                    .finish()
            }
            Self::RemoveInstrument { instrument_id } => {
                f.debug_struct("RemoveInstrument").field("instrument_id", instrument_id).finish()
            }
            Self::SetInstrumentParameter { instrument_id, param } => {
                f.debug_struct("SetInstrumentParameter")
                    .field("instrument_id", instrument_id)
                    .field("param", param)
                    .finish()
            }
            Self::SetInstrumentMidiChannel { instrument_id, channel } => {
                f.debug_struct("SetInstrumentMidiChannel")
                    .field("instrument_id", instrument_id)
                    .field("channel", channel)
                    .finish()
            }
            Self::SetInstrumentEnabled { instrument_id, enabled } => {
                f.debug_struct("SetInstrumentEnabled")
                    .field("instrument_id", instrument_id)
                    .field("enabled", enabled)
                    .finish()
            }
            // Note commands
            Self::NoteOn { note, velocity, channel } => {
                f.debug_struct("NoteOn")
                    .field("note", note)
                    .field("velocity", velocity)
                    .field("channel", channel)
                    .finish()
            }
            Self::NoteOff { note, channel } => {
                f.debug_struct("NoteOff")
                    .field("note", note)
                    .field("channel", channel)
                    .finish()
            }
            Self::AllNotesOff => write!(f, "AllNotesOff"),
            Self::PitchBend { value, channel } => {
                f.debug_struct("PitchBend")
                    .field("value", value)
                    .field("channel", channel)
                    .finish()
            }
            Self::ModWheel { value, channel } => {
                f.debug_struct("ModWheel")
                    .field("value", value)
                    .field("channel", channel)
                    .finish()
            }
            Self::Aftertouch { value, channel } => {
                f.debug_struct("Aftertouch")
                    .field("value", value)
                    .field("channel", channel)
                    .finish()
            }
            Self::PolyAftertouch { note, value, channel } => {
                f.debug_struct("PolyAftertouch")
                    .field("note", note)
                    .field("value", value)
                    .field("channel", channel)
                    .finish()
            }
            Self::SetVoiceParameter { target, param } => {
                f.debug_struct("SetVoiceParameter")
                    .field("target", target)
                    .field("param", param)
                    .finish()
            }
            Self::SetModuleParameter { module_id, param } => {
                f.debug_struct("SetModuleParameter")
                    .field("module_id", module_id)
                    .field("param", param)
                    .finish()
            }
            Self::AddModuleInstance { id, .. } => {
                // Can't debug the module itself, just show the ID
                f.debug_struct("AddModuleInstance")
                    .field("id", id)
                    .field("module", &"<dyn PolyModule>")
                    .finish()
            }
            Self::RemoveModule { id } => {
                f.debug_struct("RemoveModule").field("id", id).finish()
            }
            Self::Connect { from, to } => {
                f.debug_struct("Connect")
                    .field("from", from)
                    .field("to", to)
                    .finish()
            }
            Self::Disconnect { from, to } => {
                f.debug_struct("Disconnect")
                    .field("from", from)
                    .field("to", to)
                    .finish()
            }
            Self::DisconnectAll { module } => {
                f.debug_struct("DisconnectAll").field("module", module).finish()
            }
            Self::SetTempo(t) => write!(f, "SetTempo({t})"),
            Self::Play => write!(f, "Play"),
            Self::Stop => write!(f, "Stop"),
            Self::Pause => write!(f, "Pause"),
            Self::Rewind => write!(f, "Rewind"),
            Self::Reset => write!(f, "Reset"),
            Self::ClearAllModules => write!(f, "ClearAllModules"),
            Self::SetMasterVolume(v) => write!(f, "SetMasterVolume({v})"),
            Self::SetGlideTime(t) => write!(f, "SetGlideTime({t})"),
            Self::SetBypass { module, bypass } => {
                f.debug_struct("SetBypass")
                    .field("module", module)
                    .field("bypass", bypass)
                    .finish()
            }
            Self::AddVisualizer { id, visualizer_type, .. } => {
                f.debug_struct("AddVisualizer")
                    .field("id", id)
                    .field("visualizer_type", visualizer_type)
                    .finish()
            }
            Self::RemoveVisualizer { id } => {
                f.debug_struct("RemoveVisualizer").field("id", id).finish()
            }
            Self::AddEffectInstance { id, .. } => {
                f.debug_struct("AddEffectInstance")
                    .field("id", id)
                    .field("effect", &"<dyn AudioEffect>")
                    .finish()
            }
            Self::RemoveEffect { id } => {
                f.debug_struct("RemoveEffect").field("id", id).finish()
            }
            Self::SetEffectParameter { effect_type, param } => {
                f.debug_struct("SetEffectParameter")
                    .field("effect_type", effect_type)
                    .field("param", param)
                    .finish()
            }
            Self::SetEffectEnabled { effect_type, enabled } => {
                f.debug_struct("SetEffectEnabled")
                    .field("effect_type", effect_type)
                    .field("enabled", enabled)
                    .finish()
            }
        }
    }
}
