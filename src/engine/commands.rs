//! Commands that can be sent to the synth engine.
//!
//! These commands are sent from the UI thread to the audio thread
//! via a lock-free ring buffer.

use serde::{Deserialize, Serialize};
use std::fmt;

use super::typed_params::{TypedParam, TypedValue};

/// Unique identifier for a module instance.
/// 
/// Uses numeric IDs internally for efficiency (Copy trait).
/// Use `VoiceModule` enum for type-safe access to standard voice modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModuleId(pub u32);

impl ModuleId {
    pub const MASTER: Self = Self(0);

    pub fn new(id: u32) -> Self {
        Self(id)
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Try to get a descriptive name from VoiceModule
        if let Some(vm) = VoiceModule::from_module_id(*self) {
            write!(f, "{}", vm.id())
        } else {
            write!(f, "module-{}", self.0)
        }
    }
}

/// Identifies a module within a voice.
/// Used for type-safe parameter routing without magic numbers.
/// 
/// Each variant has a unique numeric ID and descriptive string name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VoiceModule {
    Oscillator1,
    Oscillator2,
    Filter,
    AmpEnvelope,
    FilterEnvelope,
    Lfo,
    Amplifier,
    Mixer,
}

impl VoiceModule {
    /// Get the module info: (numeric_id, string_id, internal_name)
    pub const fn info(&self) -> (u32, &'static str, &'static str) {
        match self {
            Self::Oscillator1    => (1, "oscillator-1", "osc1"),
            Self::Oscillator2    => (2, "oscillator-2", "osc2"),
            Self::Filter         => (3, "filter", "filter"),
            Self::AmpEnvelope    => (4, "amp-envelope", "amp_env"),
            Self::FilterEnvelope => (5, "filter-envelope", "filter_env"),
            Self::Lfo            => (6, "lfo", "lfo"),
            Self::Amplifier      => (7, "amplifier", "amp"),
            Self::Mixer          => (8, "mixer", "mixer"),
        }
    }
    
    /// Get the descriptive string ID (e.g., "oscillator-1").
    pub const fn id(&self) -> &'static str {
        self.info().1
    }
    
    /// Get the internal module name used by Voice (e.g., "osc1").
    pub const fn internal_name(&self) -> &'static str {
        self.info().2
    }
    
    /// Get the GUI ModuleId for this voice module.
    pub const fn module_id(&self) -> ModuleId {
        ModuleId(self.info().0)
    }
    
    /// Try to get VoiceModule from a ModuleId.
    pub fn from_module_id(id: ModuleId) -> Option<Self> {
        Self::ALL.iter().find(|v| v.module_id() == id).copied()
    }
    
    /// All voice modules.
    pub const ALL: &'static [VoiceModule] = &[
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
    pub fn create_module(&self) -> Option<Box<dyn crate::modules::VoiceModule>> {
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
    // === Note control ===
    /// Start a note.
    NoteOn {
        note: u8,
        velocity: f32,
        channel: u8,
    },

    /// Stop a note.
    NoteOff {
        note: u8,
        channel: u8,
    },

    /// All notes off.
    AllNotesOff,

    /// Pitch bend.
    PitchBend {
        value: f32, // -1.0 to 1.0
        channel: u8,
    },

    /// Aftertouch.
    Aftertouch {
        value: f32,
        channel: u8,
    },

    /// Per-note aftertouch.
    PolyAftertouch {
        note: u8,
        value: f32,
        channel: u8,
    },

    // === Parameter control ===
    /// Set a voice module parameter using the type-safe API.
    /// Use VoiceModule to identify which module within the voice to update.
    SetVoiceParameter {
        target: VoiceModule,
        param: super::typed_params::TypedParam,
        value: super::typed_params::TypedValue,
    },
    
    /// Set a parameter on a module in the global graph.
    SetModuleParameter {
        module_id: ModuleId,
        param: super::typed_params::TypedParam,
        value: super::typed_params::TypedValue,
    },

    // === Module control ===
    /// Add a pre-created module instance to the graph (real-time safe).
    /// The module is created in the GUI thread and sent via this command.
    AddModuleInstance {
        id: ModuleId,
        module: Box<dyn crate::modules::VoiceModule>,
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
    /// Set tempo in BPM.
    SetTempo(f32),

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

    /// Set master volume.
    SetMasterVolume(f32),

    /// Set glide/portamento time in seconds.
    SetGlideTime(f32),

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
        buffer: crate::visualizers::VisualizationBuffer,
    },

    /// Remove a visualizer module.
    RemoveVisualizer {
        id: ModuleId,
    },

    /// Add a pre-created effect instance to the effect chain (real-time safe).
    /// The effect is created in the GUI thread and sent via this command.
    AddEffectInstance {
        id: ModuleId,
        effect: Box<dyn crate::modules::EffectModule>,
    },
    
    /// Remove an effect from the effect chain.
    RemoveEffect {
        id: ModuleId,
    },
    
    /// Set an effect parameter using type-safe API.
    SetEffectParameter {
        effect_type: EffectType,
        param: super::typed_params::TypedParam,
        value: super::typed_params::TypedValue,
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
    ParameterChanged {
        module: ModuleId,
        param: TypedParam,
        value: TypedValue,
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
}

// Manual Debug implementation because Box<dyn VoiceModule> doesn't implement Debug
impl std::fmt::Debug for EngineCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
            Self::SetVoiceParameter { target, param, value } => {
                f.debug_struct("SetVoiceParameter")
                    .field("target", target)
                    .field("param", param)
                    .field("value", value)
                    .finish()
            }
            Self::SetModuleParameter { module_id, param, value } => {
                f.debug_struct("SetModuleParameter")
                    .field("module_id", module_id)
                    .field("param", param)
                    .field("value", value)
                    .finish()
            }
            Self::AddModuleInstance { id, .. } => {
                // Can't debug the module itself, just show the ID
                f.debug_struct("AddModuleInstance")
                    .field("id", id)
                    .field("module", &"<dyn VoiceModule>")
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
                    .field("effect", &"<dyn EffectModule>")
                    .finish()
            }
            Self::RemoveEffect { id } => {
                f.debug_struct("RemoveEffect").field("id", id).finish()
            }
            Self::SetEffectParameter { effect_type, param, value } => {
                f.debug_struct("SetEffectParameter")
                    .field("effect_type", effect_type)
                    .field("param", param)
                    .field("value", value)
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
