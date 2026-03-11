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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use synth_core::{BipolarValue, Bpm, Gain, Seconds, Semitones};
use synth_engine::instrument::InstrumentId;
use thiserror::Error;

use synth_core::ModuleType;
use synth_engine::ModuleId;
use synth_engine::graph::Connection;

/// Author information embedded in a patch file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PatchAuthor {
    /// Full name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Email address.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// Website URL.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub website: String,
    /// License (e.g. "CC BY 4.0", "All rights reserved").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
}

impl From<&str> for PatchAuthor {
    fn from(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }
}

impl PatchAuthor {
    /// Returns true if all fields are empty.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
            && self.email.is_empty()
            && self.website.is_empty()
            && self.license.is_empty()
    }
}

/// Deserialize author from either a string (legacy) or an object (new format).
fn deserialize_author<'de, D>(deserializer: D) -> Result<Option<PatchAuthor>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct AuthorVisitor;

    impl<'de> de::Visitor<'de> for AuthorVisitor {
        type Value = Option<PatchAuthor>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or an author object")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Some(PatchAuthor {
                name: v.to_string(),
                ..PatchAuthor::default()
            }))
        }

        fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
            Ok(Some(PatchAuthor {
                name: v,
                ..PatchAuthor::default()
            }))
        }

        fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            let author = PatchAuthor::deserialize(de::value::MapAccessDeserializer::new(map))?;
            Ok(Some(author))
        }
    }

    deserializer.deserialize_any(AuthorVisitor)
}

/// A complete synthesizer patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patch {
    /// Patch name.
    pub name: String,
    /// Author information.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_author"
    )]
    pub author: Option<PatchAuthor>,
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
    /// Module groups (UI-level metadata).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<ModuleGroupState>,
    /// Global settings.
    #[serde(default)]
    pub settings: PatchSettings,
}

fn default_version() -> String {
    "1.0".to_string()
}

/// 2D position in the rack view, serialized as `{"x": ..., "y": ...}`.
///
/// Deserializes from both `{"x": ..., "y": ...}` (new) and `[x, y]` (legacy).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl Position {
    #[must_use]
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl From<(f32, f32)> for Position {
    fn from((x, y): (f32, f32)) -> Self {
        Self { x, y }
    }
}

impl<'de> Deserialize<'de> for Position {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        struct PositionVisitor;

        impl<'de> de::Visitor<'de> for PositionVisitor {
            type Value = Position;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("{\"x\": ..., \"y\": ...} or [x, y]")
            }

            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let x = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &"[x, y]"))?;
                let y = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(1, &"[x, y]"))?;
                Ok(Position { x, y })
            }

            fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                #[derive(Deserialize)]
                struct XY {
                    x: f32,
                    y: f32,
                }
                let xy = XY::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(Position { x: xy.x, y: xy.y })
            }
        }

        deserializer.deserialize_any(PositionVisitor)
    }
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
    pub position: Position,
    /// Parameter values.
    #[serde(default)]
    pub parameters: HashMap<String, ParamValue>,
}

/// Unique ID for a group within a patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub u32);

/// Hex color string for UI (e.g., "#RRGGBB" or "#RRGGBBAA").
pub type HexColor = String;

/// A port exposed on a group boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposedPortState {
    pub label: String,
    /// Module ID string (e.g., "osc-1").
    pub module_id: String,
    /// Port name string (e.g., "out", "cutoff").
    pub port: String,
}

/// Group of modules in the patch editor (UI-level metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleGroupState {
    pub id: GroupId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<HexColor>,
    /// Module IDs that belong to this group.
    pub members: Vec<String>,
    /// Whether the group is collapsed in the UI.
    pub collapsed: bool,
    /// Position of the group box when collapsed.
    pub position: Position,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposed_inputs: Vec<ExposedPortState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposed_outputs: Vec<ExposedPortState>,
}

/// Template category for grouping common building blocks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupCategory {
    Voice,
    Effect,
    #[default]
    Utility,
    Tutorial,
}

impl GroupCategory {
    pub const ALL: [GroupCategory; 4] = [
        GroupCategory::Voice,
        GroupCategory::Effect,
        GroupCategory::Utility,
        GroupCategory::Tutorial,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GroupCategory::Voice => "Voice",
            GroupCategory::Effect => "Effect",
            GroupCategory::Utility => "Utility",
            GroupCategory::Tutorial => "Tutorial",
        }
    }
}

/// Reusable group template stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupTemplate {
    /// Template name.
    pub name: String,
    /// Optional author metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional category (Voice / Effect / Utility / Tutorial).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<GroupCategory>,
    /// Tags for search/filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Optional group color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<HexColor>,
    /// Modules in the template (positions are relative to template origin).
    pub modules: Vec<ModuleState>,
    /// Connections between template modules.
    pub connections: Vec<ConnectionState>,
    /// Exposed input ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposed_inputs: Vec<ExposedPortState>,
    /// Exposed output ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposed_outputs: Vec<ExposedPortState>,
}

impl GroupTemplate {
    /// Create a new empty group template.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: None,
            description: None,
            category: None,
            tags: Vec::new(),
            color: None,
            modules: Vec::new(),
            connections: Vec::new(),
            exposed_inputs: Vec::new(),
            exposed_outputs: Vec::new(),
        }
    }

    /// Add a module to the template.
    pub fn add_module(&mut self, module: ModuleState) {
        self.modules.push(module);
    }

    /// Add a connection between modules in the template.
    pub fn add_connection(&mut self, from_id: &str, from_port: &str, to_id: &str, to_port: &str) {
        self.connections.push(ConnectionState {
            from: (from_id.to_string(), from_port.to_string()),
            to: (to_id.to_string(), to_port.to_string()),
        });
    }

    /// Expose an input port on the group boundary.
    pub fn expose_input(&mut self, label: &str, module_id: &str, port: &str) {
        self.exposed_inputs.push(ExposedPortState {
            label: label.to_string(),
            module_id: module_id.to_string(),
            port: port.to_string(),
        });
    }

    /// Expose an output port on the group boundary.
    pub fn expose_output(&mut self, label: &str, module_id: &str, port: &str) {
        self.exposed_outputs.push(ExposedPortState {
            label: label.to_string(),
            module_id: module_id.to_string(),
            port: port.to_string(),
        });
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
            from: (c.from_module.to_string(), c.from_port.into()),
            to: (c.to_module.to_string(), c.to_port.into()),
        }
    }
}

impl ConnectionState {
    /// Convert to Connection, with an ID lookup function.
    /// Returns None if module IDs cannot be parsed.
    pub fn to_connection(&self) -> Option<Connection> {
        let from_id: ModuleId = self.from.0.parse().ok()?;
        let to_id: ModuleId = self.to.0.parse().ok()?;
        Some(Connection::new(from_id, &self.from.1, to_id, &self.to.1))
    }
}

/// Global patch settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchSettings {
    /// Master volume (0.0 - 1.0).
    #[serde(default = "default_master_volume")]
    pub master_volume: Gain,
    /// Tempo in BPM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<Bpm>,
    /// Octave offset for keyboard.
    #[serde(default)]
    pub octave_offset: i32,
    /// Glide/portamento time in seconds (0.0 = off).
    #[serde(default)]
    pub glide_time: Seconds,
    /// AWE (Acoustic World Engine) state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awe: Option<synth_awe::AweState>,
    /// Canvas size hint (width, height) for the rack view.
    /// Used to restore the scroll area size when loading a patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_size: Option<(f32, f32)>,
}

fn default_master_volume() -> Gain {
    Gain::new(0.8)
}

impl Default for PatchSettings {
    fn default() -> Self {
        Self {
            master_volume: Gain::new(0.8),
            bpm: Some(Bpm::new(120.0)),
            octave_offset: 0,
            glide_time: Seconds::new(0.0),
            awe: None,
            canvas_size: None,
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
            groups: Vec::new(),
            settings: PatchSettings::default(),
        }
    }

    /// Load a patch from a JSON file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PatchError> {
        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| PatchError::Io(e.to_string()))?;

        serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))
    }

    /// Save the patch to a JSON file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PatchError> {
        let content =
            serde_json::to_string_pretty(self).map_err(|e| PatchError::Serialize(e.to_string()))?;

        fs::write(path.as_ref(), content).map_err(|e| PatchError::Io(e.to_string()))
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

    /// Add a module group to the patch.
    ///
    /// Automatically assigns a group ID based on the current number of groups.
    pub fn add_group(&mut self, name: &str, color: Option<&str>, members: &[&str]) {
        let id = GroupId(self.groups.len() as u32 + 1);
        // Compute average position of member modules for group placement.
        let (sum_x, sum_y, count) = self
            .modules
            .iter()
            .filter(|m| members.contains(&m.id.as_str()))
            .fold((0.0_f32, 0.0_f32, 0u32), |(sx, sy, c), m| {
                (sx + m.position.x, sy + m.position.y, c + 1)
            });
        let position = if count > 0 {
            Position::new(sum_x / count as f32, sum_y / count as f32)
        } else {
            Position::default()
        };
        self.groups.push(ModuleGroupState {
            id,
            name: name.to_string(),
            color: color.map(String::from),
            members: members.iter().map(|s| (*s).to_string()).collect(),
            collapsed: false,
            position,
            exposed_inputs: Vec::new(),
            exposed_outputs: Vec::new(),
        });
    }
}

// ============================================================================
// INSTRUMENT STATE (for project files)
// ============================================================================

/// Serializable state of a single instrument, used in project files.
///
/// Uses domain newtypes where they have `#[serde(transparent)]` (Gain,
/// BipolarValue, Semitones, InstrumentId). Fields that would change
/// serialization format (channel, key_range) or lack serde (oversampling)
/// remain as primitives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentState {
    /// Engine instrument ID.
    pub id: InstrumentId,
    /// Display name.
    pub name: String,
    /// MIDI channel (1-indexed, 1–16).
    pub channel: u8,
    /// Volume (0.0 = silent, 1.0 = unity).
    pub volume: Gain,
    /// Stereo pan (-1.0 = left, 0.0 = center, +1.0 = right).
    pub pan: BipolarValue,
    /// Whether the instrument is muted.
    pub muted: bool,
    /// Whether the instrument is soloed.
    pub solo: bool,
    /// Key range as (low, high) MIDI note numbers.
    #[serde(default = "default_key_range")]
    pub key_range: (u8, u8),
    /// Transpose offset in semitones.
    #[serde(default)]
    pub transpose: Semitones,
    /// Oversampling factor (1, 2, or 4).
    #[serde(default = "default_oversampling")]
    pub oversampling: u8,
    /// Full module graph for this instrument.
    pub patch: Patch,
}

fn default_key_range() -> (u8, u8) {
    (0, 127)
}

fn default_oversampling() -> u8 {
    1
}

/// Errors that can occur when loading/saving patches.
#[derive(Debug, Clone, Error)]
pub enum PatchError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Serialize error: {0}")]
    Serialize(String),
}

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
                position: Position::default(),
                parameters: HashMap::new(),
            },
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.state.position = Position::new(x, y);
        self
    }

    /// Set a float parameter.
    pub fn param_f(mut self, name: &str, value: f32) -> Self {
        self.state
            .parameters
            .insert(name.to_string(), ParamValue::Float(value));
        self
    }

    /// Set an integer parameter.
    pub fn param_i(mut self, name: &str, value: i32) -> Self {
        self.state
            .parameters
            .insert(name.to_string(), ParamValue::Int(value));
        self
    }

    /// Set a boolean parameter.
    pub fn param_b(mut self, name: &str, value: bool) -> Self {
        self.state
            .parameters
            .insert(name.to_string(), ParamValue::Bool(value));
        self
    }

    /// Set a choice/enum parameter (waveform, filter mode, etc.).
    pub fn param_choice(mut self, name: &str, value: &str) -> Self {
        self.state
            .parameters
            .insert(name.to_string(), ParamValue::Choice(value.to_string()));
        self
    }

    /// Convenience: set waveform for oscillator.
    pub fn waveform(self, wf: &str) -> Self {
        self.param_choice("waveform", wf)
    }

    /// Convenience: set filter mode.
    pub fn filter_mode(self, mode: &str) -> Self {
        self.param_choice("type", mode)
    }

    /// Convenience: set delay mode.
    pub fn delay_mode(self, mode: &str) -> Self {
        self.param_choice("mode", mode)
    }

    /// Convenience: set distortion mode.
    pub fn distortion_mode(self, mode: &str) -> Self {
        self.param_choice("type", mode)
    }

    /// Convenience: set filter model (standard, fluid, screamer, acid).
    pub fn filter_model(self, model: &str) -> Self {
        self.param_choice("model", model)
    }

    /// Convenience: set waveshaper curve.
    pub fn waveshaper_curve(self, curve: &str) -> Self {
        self.param_choice("curve", curve)
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

pub use crate::group_templates::categorized_group_templates;
pub use crate::patches::{categorized_patches, example_patches};
