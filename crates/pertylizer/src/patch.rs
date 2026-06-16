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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use synth_core::{BipolarValue, Bpm, Gain, Param, ParameterDescriptor, Seconds, Semitones};
use synth_engine::instrument::InstrumentId;
use thiserror::Error;

use synth_core::ModuleType;
use synth_engine::ModuleId;
use synth_engine::graph::Connection;

/// Author information embedded in patch, project, and preset files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct Author {
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

impl From<&str> for Author {
    fn from(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }
}

impl Author {
    /// Returns true if all fields are empty.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
            && self.email.is_empty()
            && self.website.is_empty()
            && self.license.is_empty()
    }
}

/// A complete synthesizer patch.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Patch {
    /// Patch name.
    pub name: String,
    /// Author information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
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
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
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

/// State of a single module.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModuleState {
    /// Module ID (e.g., "osc-1", "flt-2").
    pub id: String,
    /// Module type (oscillator, filter, envelope, etc.).
    #[serde(rename = "type")]
    pub module_type: ModuleType,
    /// Position in the rack view.
    pub position: Position,
    /// Free-text per-instance note (e.g. "this LFO is the wobble modulator").
    /// Distinct from `ModuleDescriptor.description`, which documents the module
    /// *type* and is shared across all instances. Optional and skipped when
    /// empty, so existing projects/patches stay byte-identical and schema-valid.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Parameter values.
    #[serde(default)]
    pub parameters: BTreeMap<String, ParamValue>,
    /// Per-slot YAMS control scripts (Step 2), keyed by 1-based slot number
    /// (`"1"`..`"16"`, matching the `slot_N_*` parameter naming). Held **separate
    /// from `parameters`** because a script is canonical source text compiled on
    /// load — not a descriptor-driven `f32`/choice param — so it must dodge the
    /// descriptor-keyed schema. Optional and skipped when empty, so scriptless
    /// projects (every existing one) are byte-identical and still schema-valid.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scripts: BTreeMap<String, String>,
}

/// Unique ID for a group within a patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct GroupId(pub u32);

/// Hex color string for UI (e.g., "#RRGGBB" or "#RRGGBBAA").
pub type HexColor = String;

/// Parse a hex color string into `(r, g, b, a)` bytes.
///
/// Accepts `"#RRGGBB"` (alpha defaults to `FF`) or `"#RRGGBBAA"`, with or
/// without the leading `#`, and rejects any other length or non-hex-digit
/// content (including a leading `+`/`-` that `from_str_radix` would otherwise
/// allow). Lives here (not in the `gui-egui`-gated `gui` module) so both the
/// MCP bridge and the GUI parser share one definition of "valid" and cannot
/// drift apart.
#[must_use]
pub fn parse_hex_rgba(hex: &str) -> Option<(u8, u8, u8, u8)> {
    let s = hex.trim().trim_start_matches('#');
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match s.len() {
        6 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some((
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
                0xFF,
            ))
        }
        8 => {
            let v = u32::from_str_radix(s, 16).ok()?;
            Some((
                ((v >> 24) & 0xFF) as u8,
                ((v >> 16) & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                (v & 0xFF) as u8,
            ))
        }
        _ => None,
    }
}

/// Validate and normalize a hex color to canonical uppercase `"#RRGGBBAA"`.
///
/// Thin wrapper over [`parse_hex_rgba`]; produces the same canonical form the
/// GUI writes via `color32_to_hex`.
#[must_use]
pub fn normalize_hex_color(hex: &str) -> Option<HexColor> {
    let (r, g, b, a) = parse_hex_rgba(hex)?;
    Some(format!("#{r:02X}{g:02X}{b:02X}{a:02X}"))
}

/// A port exposed on a group boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExposedPortState {
    pub label: String,
    /// Module ID string (e.g., "osc-1").
    pub module_id: String,
    /// Port name string (e.g., "out", "cutoff").
    pub port: String,
}

/// Group of modules in the patch editor (UI-level metadata).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(rename_all = "snake_case")]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
///
/// **Variant order matters** for `#[serde(untagged)]`: Bool must come before
/// numeric types (JSON `true`/`false` would match numbers otherwise), and
/// Int before Float so integer JSON values deserialize as `Int` rather than `Float`.
///
/// `SampleId` is a struct variant so it serializes as `{"sample_id": N}` —
/// distinguishable from plain numbers/strings under `untagged`. Plain `f32`
/// can't represent a `u64` `SampleId` losslessly, so sampler assignments need
/// their own variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ParamValue {
    Bool(bool),
    Int(i32),
    Float(f32),
    Choice(String),
    SampleId { sample_id: u64 },
}

impl ParamValue {
    /// Convert to f32 for GUI/slider display. Lossy for `SampleId` (ids ≥ 2²⁴
    /// lose precision); use [`to_param`](Self::to_param) for the engine.
    pub fn to_f32(&self, desc: &ParameterDescriptor) -> f32 {
        match self {
            Self::Float(f) => *f,
            Self::Int(i) => *i as f32,
            Self::SampleId { sample_id } => *sample_id as f32,
            Self::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Choice(s) => desc
                .choices
                .as_ref()
                .and_then(|cs| cs.iter().position(|c| c.id == *s))
                .map(|i| i as f32)
                .unwrap_or(0.0),
        }
    }

    /// Convert to engine [`Param`], preserving full precision.
    ///
    /// Bypasses the f32 path for `SampleId` so u64 sample ids round-trip
    /// losslessly — `SamplerParam::SampleSelect` would otherwise truncate
    /// through the 24-bit f32 mantissa.
    pub fn to_param(&self, desc: &ParameterDescriptor) -> Param {
        if let Self::SampleId { sample_id } = self {
            return Param::Sampler(synth_core::params::SamplerParam::SampleSelect(
                synth_core::params::SampleId(*sample_id),
            ));
        }
        // Mod-matrix destination: parse the address string (accepting both the
        // new "flt-1.cutoff" form and legacy enum ids like "flt1_cutoff").
        if let (
            Param::ModMatrix(synth_core::params::ModMatrixParam::SlotDestination(slot, _)),
            Self::Choice(s),
        ) = (&desc.id, self)
        {
            return Param::ModMatrix(synth_core::params::ModMatrixParam::SlotDestination(
                *slot,
                synth_core::DestAddr::parse(s),
            ));
        }
        // Mod-matrix source: parse the address string (new "lfo-1.out" / macro id
        // / legacy ModSource id like "lfo1").
        if let (
            Param::ModMatrix(synth_core::params::ModMatrixParam::SlotSource(slot, _)),
            Self::Choice(s),
        ) = (&desc.id, self)
        {
            return Param::ModMatrix(synth_core::params::ModMatrixParam::SlotSource(
                *slot,
                synth_core::SrcAddr::parse(s),
            ));
        }
        desc.id.with_f32(self.to_f32(desc))
    }

    /// Build from an engine [`Param`], preserving the full sample id for
    /// `SampleSelect` (which can't survive the f32 round-trip). For choice
    /// parameters (those whose descriptor lists `choices`), emits the
    /// canonical string id rather than a numeric index.
    pub fn from_param(p: &Param, desc: &ParameterDescriptor) -> Self {
        if let Param::Sampler(synth_core::params::SamplerParam::SampleSelect(sid)) = p {
            return Self::SampleId { sample_id: sid.0 };
        }
        // Mod-matrix destinations serialize as a free-form address string
        // ("flt-1.cutoff"), not the legacy enum index — so they can target any
        // modulatable param on any module (S1.2).
        if let Param::ModMatrix(synth_core::params::ModMatrixParam::SlotDestination(_, dest)) = p {
            return Self::Choice(
                dest.map_or_else(|| "none".to_string(), |a| a.to_address_string()),
            );
        }
        // Mod-matrix sources likewise serialize as a free-form address string
        // (a macro id like "velocity", or "lfo-1.out") — so any module output /
        // macro is a usable source (S1.3).
        if let Param::ModMatrix(synth_core::params::ModMatrixParam::SlotSource(_, src)) = p {
            return Self::Choice(src.map_or_else(|| "none".to_string(), |a| a.to_address_string()));
        }
        if let Some(choice) = desc.choice_for_value(p.as_f32()) {
            return Self::Choice(choice.id.clone());
        }
        Self::Float(p.as_f32())
    }
}

// ============================================================================
// LEGACY MOD-MATRIX ADDRESS UPGRADE
// ============================================================================

/// Map each `ModuleType` to the sorted list of 1-based instance numbers present
/// in this instrument's module list. Drives the positional upgrade of legacy
/// mod-matrix ids in [`upgrade_legacy_mod_matrix`].
fn positional_instances(modules: &[ModuleState]) -> BTreeMap<ModuleType, Vec<u16>> {
    let mut map: BTreeMap<ModuleType, Vec<u16>> = BTreeMap::new();
    for m in modules {
        if let Some((prefix, inst)) = m.id.rsplit_once('-')
            && let (Some(mt), Ok(instance)) = (ModuleType::from_prefix(prefix), inst.parse::<u16>())
        {
            map.entry(mt).or_default().push(instance);
        }
    }
    for instances in map.values_mut() {
        instances.sort_unstable();
    }
    map
}

/// Upgrade one legacy positional source id (`"env2"`) to an absolute address
/// string (`"env-6.out"`) using this instrument's actual instances. Returns
/// `None` to leave the value unchanged — a new-form address, a macro, or an
/// unresolvable positional index.
fn upgrade_legacy_source(s: &str, instances: &BTreeMap<ModuleType, Vec<u16>>) -> Option<String> {
    // New-form addresses ("lfo-1.out") carry a '-' and never need upgrading.
    if s.contains('-') {
        return None;
    }
    let src = synth_core::ModSource::ALL
        .iter()
        .copied()
        .find(|x| x.id() == s)?;
    // `None` for macro sources (Velocity, ModWheel, …) — they have no instance.
    let (module_type, position) = src.module_type_and_position()?;
    let instance = *instances.get(&module_type)?.get(position)?;
    match synth_core::SrcAddr::from_mod_source(src)? {
        synth_core::SrcAddr::Module { name, .. } => Some(
            synth_core::SrcAddr::module(module_type, instance, name.as_str()).to_address_string(),
        ),
        synth_core::SrcAddr::Macro(_) => None,
    }
}

/// Upgrade one legacy positional destination id (`"osc1_pitch"`) to an absolute
/// address string. See [`upgrade_legacy_source`].
fn upgrade_legacy_dest(s: &str, instances: &BTreeMap<ModuleType, Vec<u16>>) -> Option<String> {
    if s.contains('-') {
        return None;
    }
    let dest = synth_core::ModDestination::ALL
        .iter()
        .copied()
        .find(|x| x.id() == s)?;
    let (module_type, position, param) = dest.module_target_position()?;
    let instance = *instances.get(&module_type)?.get(position)?;
    Some(synth_core::DestAddr::new(module_type, instance, param).to_address_string())
}

/// Upgrade legacy positional mod-matrix source/dest ids in `modules` to absolute
/// addresses, resolving each legacy 0-based positional index against *this
/// instrument's* real module instances.
///
/// The legacy [`ModSource`](synth_core::ModSource) /
/// [`ModDestination`](synth_core::ModDestination) enums addressed the i-th
/// module of a type *within the instrument*; the address form is an absolute
/// global instance. When earlier instruments climbed the instance counter (so
/// this instrument's envelopes are, say, `env-5`/`env-6`), naive parsing of
/// `"env2"` would point at a *different* instrument's `env-2` — or a
/// non-existent module — silently dropping the modulation. Resolving
/// positionally here keeps old projects sounding the same after the address
/// refactor. A no-op for canonical instances and already-upgraded addresses.
pub(crate) fn upgrade_legacy_mod_matrix(modules: &mut [ModuleState]) {
    let instances = positional_instances(modules);
    for module in modules.iter_mut() {
        if module.module_type != ModuleType::ModMatrix {
            continue;
        }
        for (key, value) in &mut module.parameters {
            let ParamValue::Choice(s) = value else {
                continue;
            };
            let upgraded = if key.ends_with("_source") {
                upgrade_legacy_source(s, &instances)
            } else if key.ends_with("_dest") {
                upgrade_legacy_dest(s, &instances)
            } else {
                None
            };
            if let Some(new_s) = upgraded {
                *s = new_s;
            }
        }
    }
}

/// Connection between two modules.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

/// Canvas size hint for the rack view.
///
/// Serializes as `{"width": ..., "height": ...}`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct CanvasSize {
    pub width: f32,
    pub height: f32,
}

impl CanvasSize {
    #[must_use]
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Global patch settings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PatchSettings {
    /// Master volume (0.0 - 1.0).
    #[serde(default = "default_master_volume")]
    #[schemars(with = "f32")]
    pub master_volume: Gain,
    /// Tempo in BPM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<f32>")]
    pub bpm: Option<Bpm>,
    /// Octave offset for keyboard.
    #[serde(default)]
    pub octave_offset: i32,
    /// Glide/portamento time in seconds (0.0 = off).
    #[serde(default)]
    #[schemars(with = "f32")]
    pub glide_time: Seconds,
    /// AWE (Acoustic World Engine) state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awe: Option<synth_awe::AweState>,
    /// Canvas size hint for the rack view.
    /// Used to restore the scroll area size when loading a patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_size: Option<CanvasSize>,
    /// Effect chain processing order (module IDs as strings, in chain order).
    ///
    /// The engine restores the effect chain to match this order on load.
    /// An empty list means the patch has no effect-chain modules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effect_chain_order: Vec<String>,
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
            effect_chain_order: Vec::new(),
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
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InstrumentState {
    /// Engine instrument ID.
    pub id: InstrumentId,
    /// Display name.
    pub name: String,
    /// MIDI channel (1-indexed, 1–16, clamped on deserialization).
    pub channel: u8,
    /// Volume (0.0 = silent, 1.0 = unity).
    #[schemars(with = "f32")]
    pub volume: Gain,
    /// Stereo pan (-1.0 = left, 0.0 = center, +1.0 = right).
    #[schemars(with = "f32")]
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
    #[schemars(with = "f32")]
    pub transpose: Semitones,
    /// Oversampling factor (1, 2, or 4).
    #[serde(default = "default_oversampling")]
    pub oversampling: u8,
    /// Category for visualization/routing, stored as the
    /// `InstrumentCategory::as_u8()` value. `0` = Uncategorized.
    #[serde(default)]
    pub category: u8,
    /// Free-text description of the instrument.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Optional accent color as hex string (e.g. "#FF8800FF").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<HexColor>,
    /// Voice allocation mode (Poly / Mono / Legato / Unison).
    #[serde(default)]
    pub allocation_mode: synth_engine::voice_allocator::AllocationMode,
    /// Strategy for stealing voices when all are busy.
    #[serde(default)]
    pub stealing_strategy: synth_engine::voice_allocator::StealingStrategy,
    /// Maximum polyphony for this instrument (1–128, default 8).
    #[serde(default = "default_max_voices")]
    #[schemars(with = "u8")]
    pub max_voices: synth_core::VoiceCount,
    /// Velocity → amplitude sensitivity (0 = constant volume, 1 = full dynamic).
    #[serde(default = "default_vel_amp_sens")]
    #[schemars(with = "f32")]
    pub velocity_amp_sensitivity: synth_core::NormalizedValue,
    /// Velocity → filter cutoff sensitivity (0 = none, 1 = full).
    #[serde(default)]
    #[schemars(with = "f32")]
    pub velocity_filter_sensitivity: synth_core::NormalizedValue,
    /// Sidechain source — when set, this instrument's compressors
    /// receive audio from the source instrument's output.
    /// Serialized as `u64` for forward-compat with non-Rust readers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain_source_id: Option<u64>,
    /// Full module graph for this instrument.
    pub patch: Patch,
}

impl<'de> Deserialize<'de> for InstrumentState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            id: InstrumentId,
            name: String,
            channel: u8,
            volume: Gain,
            pan: BipolarValue,
            muted: bool,
            solo: bool,
            #[serde(default = "default_key_range")]
            key_range: (u8, u8),
            #[serde(default)]
            transpose: Semitones,
            #[serde(default = "default_oversampling")]
            oversampling: u8,
            #[serde(default)]
            category: u8,
            #[serde(default)]
            description: String,
            #[serde(default)]
            color: Option<HexColor>,
            #[serde(default)]
            allocation_mode: synth_engine::voice_allocator::AllocationMode,
            #[serde(default)]
            stealing_strategy: synth_engine::voice_allocator::StealingStrategy,
            #[serde(default = "default_max_voices")]
            max_voices: synth_core::VoiceCount,
            #[serde(default = "default_vel_amp_sens")]
            velocity_amp_sensitivity: synth_core::NormalizedValue,
            #[serde(default)]
            velocity_filter_sensitivity: synth_core::NormalizedValue,
            #[serde(default)]
            sidechain_source_id: Option<u64>,
            patch: Patch,
        }

        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            id: raw.id,
            name: raw.name,
            channel: raw.channel.clamp(1, 16),
            volume: raw.volume,
            pan: raw.pan,
            muted: raw.muted,
            solo: raw.solo,
            key_range: raw.key_range,
            transpose: raw.transpose,
            oversampling: raw.oversampling,
            category: raw.category,
            description: raw.description,
            color: raw.color,
            allocation_mode: raw.allocation_mode,
            stealing_strategy: raw.stealing_strategy,
            max_voices: raw.max_voices,
            velocity_amp_sensitivity: raw.velocity_amp_sensitivity,
            velocity_filter_sensitivity: raw.velocity_filter_sensitivity,
            sidechain_source_id: raw.sidechain_source_id,
            patch: raw.patch,
        })
    }
}

fn default_key_range() -> (u8, u8) {
    (0, 127)
}

fn default_oversampling() -> u8 {
    1
}

fn default_max_voices() -> synth_core::VoiceCount {
    synth_core::VoiceCount::OCTO
}

fn default_vel_amp_sens() -> synth_core::NormalizedValue {
    synth_core::NormalizedValue::MAX
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
// AWE PRESET FILE
// ============================================================================

/// A saved AWE (Acoustic World Engine) preset file.
///
/// Contains full AWE state plus metadata (name, author, license, tags).
/// Serialized as JSON with `"file_type": "awe_preset"` discriminator.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AwePresetFile {
    /// Always `"awe_preset"` — distinguishes from patch and project files.
    pub file_type: String,
    /// Format version (currently `"1.0"`).
    #[serde(default = "default_version")]
    pub version: String,
    /// Preset name.
    pub name: String,
    /// Short description of the acoustic character.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Author information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    /// Tags for categorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Full AWE state.
    pub state: synth_awe::AweState,
}

impl AwePresetFile {
    /// Create a new AWE preset file.
    pub fn new(name: impl Into<String>, state: synth_awe::AweState) -> Self {
        Self {
            file_type: "awe_preset".to_string(),
            version: "1.0".to_string(),
            name: name.into(),
            description: String::new(),
            author: None,
            tags: Vec::new(),
            state,
        }
    }

    /// Load an AWE preset from a JSON file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PatchError> {
        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| PatchError::Io(e.to_string()))?;
        serde_json::from_str(&content).map_err(|e| PatchError::Parse(e.to_string()))
    }

    /// Save the AWE preset to a JSON file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), PatchError> {
        let content =
            serde_json::to_string_pretty(self).map_err(|e| PatchError::Serialize(e.to_string()))?;
        fs::write(path.as_ref(), content).map_err(|e| PatchError::Io(e.to_string()))
    }
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
                description: String::new(),
                parameters: BTreeMap::new(),
                scripts: BTreeMap::new(),
            },
        }
    }

    pub fn position(mut self, x: f32, y: f32) -> Self {
        self.state.position = Position::new(x, y);
        self
    }

    /// Set the per-instance description (free-text intent for this module).
    pub fn description(mut self, description: &str) -> Self {
        self.state.description = description.to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: a bare `ModuleState` with the given id and type, no parameters.
    fn module_state(id: &str, module_type: ModuleType) -> ModuleState {
        ModuleState {
            id: id.to_string(),
            module_type,
            position: Position::default(),
            description: String::new(),
            parameters: BTreeMap::new(),
            scripts: BTreeMap::new(),
        }
    }

    /// A per-slot YAMS script round-trips through JSON by its 1-based slot key,
    /// an empty `scripts` map is omitted (scriptless projects stay byte-identical),
    /// and an old project with no `scripts` field loads to an empty map.
    #[test]
    fn module_scripts_persist_round_trip() {
        let mut ms = module_state("mmx-1", ModuleType::ModMatrix);
        // Empty → skipped from the serialized form.
        let json_empty = serde_json::to_string(&ms).expect("serialize");
        assert!(
            !json_empty.contains("scripts"),
            "empty scripts must be skipped, got: {json_empty}"
        );

        // A script on slot 3 round-trips by its 1-based key.
        ms.scripts
            .insert("3".to_string(), "out = velocity * 0.5".to_string());
        let json = serde_json::to_string(&ms).expect("serialize");
        let back: ModuleState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            back.scripts.get("3").map(String::as_str),
            Some("out = velocity * 0.5")
        );

        // An old project lacking the field deserializes to an empty map (default).
        let legacy =
            r#"{"id":"mmx-1","type":"mod_matrix","position":{"x":0.0,"y":0.0},"parameters":{}}"#;
        let loaded: ModuleState = serde_json::from_str(legacy).expect("deserialize legacy");
        assert!(loaded.scripts.is_empty());
    }

    /// A per-instance `description` round-trips through JSON, an empty
    /// description is omitted from the serialized form (so existing
    /// projects stay byte-identical), and an old project with no
    /// `description` field loads to an empty string.
    #[test]
    fn module_description_persists_round_trip() {
        let mut ms = module_state("lfo-1", ModuleType::Lfo);
        // Empty → skipped from the serialized form.
        let json_empty = serde_json::to_string(&ms).expect("serialize");
        assert!(
            !json_empty.contains("description"),
            "empty description must be skipped, got: {json_empty}"
        );

        // A non-empty description round-trips.
        ms.description = "wobble modulator for the filter cutoff".to_string();
        let json = serde_json::to_string(&ms).expect("serialize");
        let back: ModuleState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.description, "wobble modulator for the filter cutoff");

        // An old project lacking the field deserializes to an empty string.
        let legacy = r#"{"id":"lfo-1","type":"lfo","position":{"x":0.0,"y":0.0},"parameters":{}}"#;
        let loaded: ModuleState = serde_json::from_str(legacy).expect("deserialize legacy");
        assert!(loaded.description.is_empty());

        // The builder sets it too.
        let built = ModuleBuilder::new(2, ModuleType::Lfo)
            .description("vibrato")
            .build();
        assert_eq!(built.description, "vibrato");
    }

    /// Legacy positional mod-matrix ids upgrade to the *positionally correct*
    /// instance, not a naive global one. An instrument whose envelopes climbed
    /// to env-5/env-6 (earlier instruments took 1-4) stores `"env2"` meaning its
    /// own second envelope — env-6 — so the upgrade must land on env-6, never
    /// the unrelated global env-2. Filter flt-3 / `"flt1_cutoff"` likewise.
    #[test]
    fn legacy_mod_matrix_upgrades_to_positional_instance() {
        let mut matrix = module_state("mmx-2", ModuleType::ModMatrix);
        matrix.parameters.insert(
            "slot_1_source".to_string(),
            ParamValue::Choice("env2".to_string()),
        );
        matrix.parameters.insert(
            "slot_1_dest".to_string(),
            ParamValue::Choice("flt1_cutoff".to_string()),
        );
        // A macro id and an already-upgraded address must pass through untouched.
        matrix.parameters.insert(
            "slot_2_source".to_string(),
            ParamValue::Choice("velocity".to_string()),
        );
        matrix.parameters.insert(
            "slot_3_source".to_string(),
            ParamValue::Choice("lfo-7.out".to_string()),
        );

        let mut modules = vec![
            module_state("env-5", ModuleType::Envelope),
            module_state("env-6", ModuleType::Envelope),
            module_state("flt-3", ModuleType::Filter),
            matrix,
        ];

        upgrade_legacy_mod_matrix(&mut modules);

        let params = &modules[3].parameters;
        assert_eq!(
            params.get("slot_1_source"),
            Some(&ParamValue::Choice("env-6.out".to_string())),
        );
        assert_eq!(
            params.get("slot_1_dest"),
            Some(&ParamValue::Choice("flt-3.cutoff".to_string())),
        );
        assert_eq!(
            params.get("slot_2_source"),
            Some(&ParamValue::Choice("velocity".to_string())),
        );
        assert_eq!(
            params.get("slot_3_source"),
            Some(&ParamValue::Choice("lfo-7.out".to_string())),
        );
    }

    /// On canonical instances (env-1/env-2) the positional upgrade is a no-op in
    /// effect — `"env2"` already maps to env-2.
    #[test]
    fn legacy_mod_matrix_upgrade_is_identity_on_canonical_instances() {
        let mut matrix = module_state("mmx-1", ModuleType::ModMatrix);
        matrix.parameters.insert(
            "slot_1_source".to_string(),
            ParamValue::Choice("env2".to_string()),
        );
        let mut modules = vec![
            module_state("env-1", ModuleType::Envelope),
            module_state("env-2", ModuleType::Envelope),
            matrix,
        ];

        upgrade_legacy_mod_matrix(&mut modules);

        assert_eq!(
            modules[2].parameters.get("slot_1_source"),
            Some(&ParamValue::Choice("env-2.out".to_string())),
        );
    }

    /// A mod-matrix destination serializes as a free-form address string and
    /// round-trips — including an *arbitrary* address (`osc-1.detune`) outside
    /// the 19 legacy roles — while a legacy id still upgrades on load.
    #[test]
    fn mod_matrix_arbitrary_destination_round_trips() {
        use synth_core::params::{ModDestination, ModMatrixParam};
        use synth_core::{DestAddr, ModuleType};

        let desc = ParameterDescriptor::choice(
            "slot_1_dest",
            Param::ModMatrix(ModMatrixParam::SlotDestination(0, None)),
            "Slot 1 Dest",
            ModDestination::to_choices(),
        );

        // Arbitrary address (not one of the 19 legacy roles).
        let addr = DestAddr::new(ModuleType::Oscillator, 1, "detune");
        let param = Param::ModMatrix(ModMatrixParam::SlotDestination(0, Some(addr)));
        let value = ParamValue::from_param(&param, &desc);
        assert!(
            matches!(&value, ParamValue::Choice(s) if s == "osc-1.detune"),
            "expected address string, got {value:?}"
        );
        assert_eq!(value.to_param(&desc), param);

        // A legacy enum id upgrades to the equivalent address on load.
        let legacy = ParamValue::Choice("flt1_cutoff".to_string());
        assert_eq!(
            legacy.to_param(&desc),
            Param::ModMatrix(ModMatrixParam::SlotDestination(
                0,
                DestAddr::from_mod_destination(ModDestination::FilterCutoff(0)),
            )),
        );

        // `none` round-trips to an unconfigured destination.
        assert_eq!(
            ParamValue::Choice("none".to_string()).to_param(&desc),
            Param::ModMatrix(ModMatrixParam::SlotDestination(0, None)),
        );
    }

    /// A mod-matrix source serializes as a free-form address string and
    /// round-trips — including an arbitrary module output (`lfo-3.out`, beyond
    /// the legacy 2-LFO ceiling) — while macro and legacy ids still work.
    #[test]
    fn mod_matrix_arbitrary_source_round_trips() {
        use synth_core::params::{ModMatrixParam, ModSource};
        use synth_core::{ModuleType, SrcAddr};

        let desc = ParameterDescriptor::choice(
            "slot_1_source",
            Param::ModMatrix(ModMatrixParam::SlotSource(0, None)),
            "Slot 1 Source",
            ModSource::to_choices(),
        );

        // A third LFO — impossible in the legacy 16-source enum.
        let addr = SrcAddr::module(ModuleType::Lfo, 3, "out");
        let param = Param::ModMatrix(ModMatrixParam::SlotSource(0, Some(addr)));
        let value = ParamValue::from_param(&param, &desc);
        assert!(
            matches!(&value, ParamValue::Choice(s) if s == "lfo-3.out"),
            "expected address string, got {value:?}"
        );
        assert_eq!(value.to_param(&desc), param);

        // A legacy id upgrades; a macro id stays a macro.
        assert_eq!(
            ParamValue::Choice("lfo1".to_string()).to_param(&desc),
            Param::ModMatrix(ModMatrixParam::SlotSource(
                0,
                SrcAddr::from_mod_source(ModSource::Lfo(0)),
            )),
        );
        assert_eq!(
            ParamValue::Choice("none".to_string()).to_param(&desc),
            Param::ModMatrix(ModMatrixParam::SlotSource(0, None)),
        );
    }

    #[test]
    fn test_awe_preset_file_round_trip() {
        let state = synth_awe::AweState::default();
        let mut preset = AwePresetFile::new("Test Preset", state);
        preset.description = "A test preset".to_string();
        preset.tags = vec!["test".to_string(), "demo".to_string()];
        preset.author = Some(Author {
            name: "Test Author".to_string(),
            license: "CC BY 4.0".to_string(),
            ..Author::default()
        });

        let json = serde_json::to_string_pretty(&preset).expect("serialize");
        let parsed: AwePresetFile = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.file_type, "awe_preset");
        assert_eq!(parsed.name, "Test Preset");
        assert_eq!(parsed.description, "A test preset");
        assert_eq!(parsed.tags, vec!["test", "demo"]);
        assert_eq!(
            parsed.author.as_ref().map(|a| a.name.as_str()),
            Some("Test Author")
        );
        assert_eq!(parsed.state.enabled, preset.state.enabled);
    }

    #[test]
    fn test_author_is_empty() {
        let author = Author::default();
        assert!(author.is_empty());

        let author = Author::from("Test");
        assert!(!author.is_empty());
    }

    #[test]
    fn test_patch_settings_effect_chain_order_round_trip() {
        let mut patch = Patch::new("Test");
        patch.settings.effect_chain_order = vec!["reverb-1".to_string(), "delay-2".to_string()];

        let json = serde_json::to_string_pretty(&patch).expect("serialize");
        assert!(
            json.contains("effect_chain_order"),
            "non-empty chain order should serialize"
        );

        let parsed: Patch = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            parsed.settings.effect_chain_order,
            vec!["reverb-1".to_string(), "delay-2".to_string()]
        );
    }

    #[test]
    fn test_patch_settings_empty_chain_order_skipped() {
        let patch = Patch::new("Test");
        let json = serde_json::to_string_pretty(&patch).expect("serialize");
        assert!(
            !json.contains("effect_chain_order"),
            "empty chain order should be skipped from JSON"
        );

        let parsed: Patch = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.settings.effect_chain_order.is_empty());
    }

    #[test]
    fn test_awe_preset_file_save_load() {
        let dir = std::env::temp_dir().join("pertylizer_test_awe");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_preset.json");

        let state = synth_awe::AweState::default();
        let preset = AwePresetFile::new("Save Load Test", state);
        preset.save(&path).expect("save");

        let loaded = AwePresetFile::load(&path).expect("load");
        assert_eq!(loaded.name, "Save Load Test");
        assert_eq!(loaded.file_type, "awe_preset");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
