//! Patch file format for saving and loading synthesizer configurations.
//!
//! Patches contain:
//! - Module configurations (type, position, parameters)
//! - Connections between modules
//! - Global settings (master volume, BPM, etc.)
//!
//! ## Parameter Format
//!
//! Parameters are stored as typed values that map to the engine's Param system:
//! - Waveforms: `"sine"`, `"triangle"`, `"sawtooth"`, `"square"`, `"pulse"`
//! - LFO Waveforms: `"sine"`, `"triangle"`, `"sawtooth"`, `"square"`, `"sample_and_hold"`
//! - Filter Modes: `"lowpass"`, `"highpass"`, `"bandpass"`, `"notch"`, `"peak"`
//! - Delay Modes: `"mono"`, `"stereo"`, `"ping_pong"`
//! - Distortion Modes: `"soft_clip"`, `"hard_clip"`, `"tube"`, `"foldback"`, `"bitcrush"`

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use serde::{Serialize, Deserialize};

use crate::engine::ModuleId;
use crate::engine::graph::Connection;

/// A complete synthesizer patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    /// Patch name.
    pub name: String,
    /// Author name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Patch version.
    #[serde(default = "default_version")]
    pub version: String,
    /// Description of the patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Detailed explanation of how the patch works.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Tags for categorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Module configurations.
    pub modules: Vec<ModuleState>,
    /// Connections between modules.
    pub connections: Vec<ConnectionState>,
    /// Global settings.
    #[serde(default)]
    pub settings: PatchSettings,
}

fn default_version() -> String {
    "1.0".to_string()
}

/// State of a single module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleState {
    /// Module ID (e.g., "osc-1", "flt-2").
    pub id: String,
    /// Module type (oscillator, filter, envelope, etc.).
    #[serde(rename = "type")]
    pub module_type: ModuleType,
    /// Position in the rack view.
    pub position: (f32, f32),
    /// Parameter values.
    #[serde(default)]
    pub parameters: HashMap<String, ParamValue>,
}

/// Module types that can be saved/loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleType {
    Oscillator,
    MathOscillator,
    SubOscillator,
    Noise,
    Filter,
    Envelope,
    Lfo,
    Amplifier,
    Mixer,
    StereoOutput,
    Delay,
    Reverb,
    Distortion,
    Chorus,
    // Visualizers
    Oscilloscope,
    LevelMeter,
}

impl ModuleType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Oscillator => "oscillator",
            Self::MathOscillator => "math_oscillator",
            Self::SubOscillator => "sub_oscillator",
            Self::Noise => "noise",
            Self::Filter => "filter",
            Self::Envelope => "envelope",
            Self::Lfo => "lfo",
            Self::Amplifier => "amplifier",
            Self::Mixer => "mixer",
            Self::StereoOutput => "stereo_output",
            Self::Delay => "delay",
            Self::Reverb => "reverb",
            Self::Distortion => "distortion",
            Self::Chorus => "chorus",
            Self::Oscilloscope => "oscilloscope",
            Self::LevelMeter => "level_meter",
        }
    }

    /// Get a short 3-letter prefix for module IDs.
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Oscillator => "osc",
            Self::MathOscillator => "mth",
            Self::SubOscillator => "sub",
            Self::Noise => "nse",
            Self::Filter => "flt",
            Self::Envelope => "env",
            Self::Lfo => "lfo",
            Self::Amplifier => "amp",
            Self::Mixer => "mix",
            Self::StereoOutput => "out",
            Self::Delay => "dly",
            Self::Reverb => "rev",
            Self::Distortion => "dst",
            Self::Chorus => "chr",
            Self::Oscilloscope => "scp",
            Self::LevelMeter => "mtr",
        }
    }
}

/// Parameter value for serialization.
/// Supports all types including string choices for waveforms, filter modes, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Choice(String),
}


/// Connection between two modules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionState {
    /// Source (module_id string like "osc-1", port_name).
    pub from: (String, String),
    /// Destination (module_id string, port_name).
    pub to: (String, String),
}

impl From<&Connection> for ConnectionState {
    fn from(c: &Connection) -> Self {
        Self {
            from: (c.from_module.to_string(), c.from_port.clone()),
            to: (c.to_module.to_string(), c.to_port.clone()),
        }
    }
}

impl ConnectionState {
    /// Convert to Connection, with an ID lookup function.
    /// Returns None if module IDs cannot be parsed.
    pub fn to_connection(&self) -> Option<Connection> {
        let from_id: ModuleId = self.from.0.parse().ok()?;
        let to_id: ModuleId = self.to.0.parse().ok()?;
        Some(Connection::new(
            from_id,
            &self.from.1,
            to_id,
            &self.to.1,
        ))
    }
}

/// Global patch settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchSettings {
    /// Master volume (0.0 - 1.0).
    #[serde(default = "default_master_volume")]
    pub master_volume: f32,
    /// Tempo in BPM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f32>,
    /// Octave offset for keyboard.
    #[serde(default)]
    pub octave_offset: i32,
    /// Glide/portamento time in seconds (0.0 = off).
    #[serde(default)]
    pub glide_time: f32,
}

fn default_master_volume() -> f32 {
    0.8
}

impl Default for PatchSettings {
    fn default() -> Self {
        Self {
            master_volume: 0.8,
            bpm: Some(120.0),
            octave_offset: 0,
            glide_time: 0.0,
        }
    }
}

impl Patch {
    /// Create a new empty patch.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: None,
            version: "1.0".to_string(),
            description: None,
            notes: None,
            tags: Vec::new(),
            modules: Vec::new(),
            connections: Vec::new(),
            settings: PatchSettings::default(),
        }
    }

    /// Load a patch from a JSON file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PatchError> {
        let content = fs::read_to_string(path.as_ref())
            .map_err(|e| PatchError::Io(e.to_string()))?;

        serde_json::from_str(&content)
            .map_err(|e| PatchError::Parse(e.to_string()))
    }

    /// Save the patch to a JSON file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PatchError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| PatchError::Serialize(e.to_string()))?;

        fs::write(path.as_ref(), content)
            .map_err(|e| PatchError::Io(e.to_string()))
    }

    /// Add a module to the patch.
    pub fn add_module(&mut self, module: ModuleState) {
        self.modules.push(module);
    }

    /// Add a connection to the patch.
    pub fn add_connection(&mut self, from_id: &str, from_port: &str, to_id: &str, to_port: &str) {
        self.connections.push(ConnectionState {
            from: (from_id.to_string(), from_port.to_string()),
            to: (to_id.to_string(), to_port.to_string()),
        });
    }
}

/// Errors that can occur when loading/saving patches.
#[derive(Debug, Clone)]
pub enum PatchError {
    Io(String),
    Parse(String),
    Serialize(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Parse(e) => write!(f, "Parse error: {}", e),
            Self::Serialize(e) => write!(f, "Serialize error: {}", e),
        }
    }
}

impl std::error::Error for PatchError {}

// ============================================================================
// MODULE BUILDER
// ============================================================================

/// Helper to create a module state with parameters.
pub struct ModuleBuilder {
    state: ModuleState,
}

impl ModuleBuilder {
    /// Create a new module with auto-generated ID based on type and instance number.
    /// Example: ModuleBuilder::new(1, ModuleType::Oscillator) creates ID "osc-1"
    pub fn new(instance: u16, module_type: ModuleType) -> Self {
        let id = format!("{}-{}", module_type.prefix(), instance);
        Self {
            state: ModuleState {
                id,
                module_type,
                position: (0.0, 0.0),
                parameters: HashMap::new(),
            },
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.state.position = (x, y);
        self
    }

    /// Set a float parameter.
    pub fn param_f(mut self, name: &str, value: f32) -> Self {
        self.state.parameters.insert(name.to_string(), ParamValue::Float(value));
        self
    }

    /// Set an integer parameter.
    pub fn param_i(mut self, name: &str, value: i32) -> Self {
        self.state.parameters.insert(name.to_string(), ParamValue::Int(value));
        self
    }

    /// Set a boolean parameter.
    pub fn param_b(mut self, name: &str, value: bool) -> Self {
        self.state.parameters.insert(name.to_string(), ParamValue::Bool(value));
        self
    }

    /// Set a choice/enum parameter (waveform, filter mode, etc.).
    pub fn param_choice(mut self, name: &str, value: &str) -> Self {
        self.state.parameters.insert(name.to_string(), ParamValue::Choice(value.to_string()));
        self
    }

    /// Convenience: set waveform for oscillator.
    pub fn waveform(self, wf: &str) -> Self {
        self.param_choice("waveform", wf)
    }

    /// Convenience: set filter mode.
    pub fn filter_mode(self, mode: &str) -> Self {
        self.param_choice("filter_type", mode)
    }

    /// Convenience: set delay mode.
    pub fn delay_mode(self, mode: &str) -> Self {
        self.param_choice("mode", mode)
    }

    /// Convenience: set distortion mode.
    pub fn distortion_mode(self, mode: &str) -> Self {
        self.param_choice("mode", mode)
    }

    /// Convenience: set algorithm for math oscillator.
    pub fn algorithm(self, algo: &str) -> Self {
        self.param_choice("algorithm", algo)
    }

    pub fn build(self) -> ModuleState {
        self.state
    }
}

// ============================================================================
// EXAMPLE PATCHES (re-exported from patches module)
// ============================================================================

pub use crate::patches::example_patches;
