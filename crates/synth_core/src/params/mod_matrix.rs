//! Mod Matrix parameter types.
//!
//! The mod matrix provides up to 16 modulation slots arranged in a grid.
//! Grid size is selectable (1x1, 2x2, 3x3, 4x4) giving 1, 4, 9, or 16 slots.
//! Each slot routes a source to a destination with a bipolar amount control.
//! A slot with Source=None is effectively inactive.

use serde::{Deserialize, Serialize};

use crate::types::BipolarValue;

/// Maximum number of mod matrix slots (4x4 grid).
pub const MAX_MOD_MATRIX_SLOTS: usize = 16;

// ============================================================================
// GRID SIZE
// ============================================================================

/// Grid size for the mod matrix display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ModMatrixGridSize {
    /// 1x1 grid (1 slot).
    Grid1x1,
    /// 2x2 grid (4 slots).
    #[default]
    Grid2x2,
    /// 3x3 grid (9 slots).
    Grid3x3,
    /// 4x4 grid (16 slots).
    Grid4x4,
}

impl ModMatrixGridSize {
    pub const ALL: [Self; 4] = [Self::Grid1x1, Self::Grid2x2, Self::Grid3x3, Self::Grid4x4];

    /// Number of active slots for this grid size.
    #[must_use]
    pub const fn slot_count(self) -> usize {
        match self {
            Self::Grid1x1 => 1,
            Self::Grid2x2 => 4,
            Self::Grid3x3 => 9,
            Self::Grid4x4 => 16,
        }
    }

    /// Grid dimension (columns = rows).
    #[must_use]
    pub const fn grid_dimension(self) -> usize {
        match self {
            Self::Grid1x1 => 1,
            Self::Grid2x2 => 2,
            Self::Grid3x3 => 3,
            Self::Grid4x4 => 4,
        }
    }

    /// Display name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Grid1x1 => "1\u{d7}1",
            Self::Grid2x2 => "2\u{d7}2",
            Self::Grid3x3 => "3\u{d7}3",
            Self::Grid4x4 => "4\u{d7}4",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Grid1x1 => "1\u{d7}1 grid — 1 modulation slot.",
            Self::Grid2x2 => "2\u{d7}2 grid — 4 modulation slots.",
            Self::Grid3x3 => "3\u{d7}3 grid — 9 modulation slots.",
            Self::Grid4x4 => "4\u{d7}4 grid — 16 modulation slots.",
        }
    }

    /// Identifier string.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Grid1x1 => "1x1",
            Self::Grid2x2 => "2x2",
            Self::Grid3x3 => "3x3",
            Self::Grid4x4 => "4x4",
        }
    }

    /// Create from index (0-3).
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    /// Get the index (0-3).
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|g| *g == self).unwrap_or(0)
    }

    /// Generate choices for GUI dropdown.
    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|g| {
                crate::module_traits::ChoiceOption::new(g.id(), g.name())
                    .with_description(g.description())
            })
            .collect()
    }
}

// ============================================================================
// MODULATION SOURCE
// ============================================================================

/// Modulation source for a mod matrix slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ModSource {
    /// No source (slot inactive).
    #[default]
    None,
    /// LFO output (index 0-based).
    Lfo(u8),
    /// Envelope output (index 0-based).
    Envelope(u8),
    /// Note velocity (0-1).
    Velocity,
    /// MIDI note number (normalized).
    NoteNumber,
    /// Channel aftertouch (0-1).
    Aftertouch,
    /// Mod wheel CC1 (0-1).
    ModWheel,
    /// Pitch bend (-1 to +1).
    PitchBend,
    /// Per-note polyphonic aftertouch (0-1).
    PolyAftertouch,
    /// Kinetic modulator position output (0-1 or -1 to 1).
    KineticPos,
    /// Kinetic modulator velocity output (-1 to 1).
    KineticVel,
    /// Kinetic modulator acceleration output (-1 to 1).
    KineticAcc,
    /// Envelope-follower output (0-1, index 0-based). Lets a follower drive a
    /// destination through the matrix without forming a graph cycle (a direct
    /// CV cable from the follower into a destination upstream of the
    /// follower's own audio input is rejected by the topo sort).
    EnvFollower(u8),
}

impl ModSource {
    pub const ALL: [Self; 16] = [
        Self::None,
        Self::Lfo(0),
        Self::Lfo(1),
        Self::Envelope(0),
        Self::Envelope(1),
        Self::Velocity,
        Self::NoteNumber,
        Self::Aftertouch,
        Self::ModWheel,
        Self::PitchBend,
        Self::PolyAftertouch,
        Self::KineticPos,
        Self::KineticVel,
        Self::KineticAcc,
        Self::EnvFollower(0),
        Self::EnvFollower(1),
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Lfo(0) => "LFO 1",
            Self::Lfo(1) => "LFO 2",
            Self::Lfo(_) => "LFO",
            Self::Envelope(0) => "Env 1",
            Self::Envelope(1) => "Env 2",
            Self::Envelope(_) => "Env",
            Self::Velocity => "Velocity",
            Self::NoteNumber => "Note",
            Self::Aftertouch => "Aftertouch",
            Self::ModWheel => "Mod Wheel",
            Self::PitchBend => "Pitch Bend",
            Self::PolyAftertouch => "Poly AT",
            Self::KineticPos => "Kinetic Pos",
            Self::KineticVel => "Kinetic Vel",
            Self::KineticAcc => "Kinetic Acc",
            Self::EnvFollower(0) => "Env Follower 1",
            Self::EnvFollower(1) => "Env Follower 2",
            Self::EnvFollower(_) => "Env Follower",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::None => "No source — the slot is inactive.",
            Self::Lfo(_) => "Low-frequency oscillator output.",
            Self::Envelope(_) => "Envelope generator output.",
            Self::Velocity => "Note-on velocity — how hard the key was struck.",
            Self::NoteNumber => "MIDI note number, normalized across the keyboard.",
            Self::Aftertouch => "Channel pressure applied after the key is held.",
            Self::ModWheel => "Mod wheel (MIDI CC 1).",
            Self::PitchBend => "Pitch-bend wheel (bipolar, centered at rest).",
            Self::PolyAftertouch => "Per-note polyphonic key pressure.",
            Self::KineticPos => "Kinetic Modulator position output.",
            Self::KineticVel => "Kinetic Modulator velocity (rate-of-change) output.",
            Self::KineticAcc => "Kinetic Modulator acceleration output.",
            Self::EnvFollower(_) => "Envelope-follower output tracking an audio signal's level.",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lfo(0) => "lfo1",
            Self::Lfo(1) => "lfo2",
            Self::Lfo(_) => "lfo",
            Self::Envelope(0) => "env1",
            Self::Envelope(1) => "env2",
            Self::Envelope(_) => "env",
            Self::Velocity => "velocity",
            Self::NoteNumber => "note",
            Self::Aftertouch => "aftertouch",
            Self::ModWheel => "mod_wheel",
            Self::PitchBend => "pitch_bend",
            Self::PolyAftertouch => "poly_aftertouch",
            Self::KineticPos => "kinetic_pos",
            Self::KineticVel => "kinetic_vel",
            Self::KineticAcc => "kinetic_acc",
            Self::EnvFollower(0) => "efl1",
            Self::EnvFollower(1) => "efl2",
            Self::EnvFollower(_) => "efl",
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|s| s == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|s| {
                crate::module_traits::ChoiceOption::new(s.id(), s.name())
                    .with_description(s.description())
            })
            .collect()
    }

    /// `(ModuleType, positional_index)` this source maps to, or `None` for
    /// implicit sources that are not module instances (`Velocity`,
    /// `NoteNumber`, `ModWheel`, `PitchBend`, `Aftertouch`, `PolyAftertouch`).
    /// Kinetic axes all resolve to `KineticModulator` position 0 — it's a
    /// single physical module emitting three outputs.
    ///
    /// `positional_index` is 0-based and refers to the i-th module of
    /// `ModuleType` in the *containing instrument's* voice graph, sorted by
    /// `ModuleId.instance`. Resolve to an actual `ModuleId` via the caller's
    /// per-instrument module enumeration — `ModuleId::new(mt, position + 1)`
    /// is only correct when the global instance counter happens to match
    /// positions (typical for the first instrument in a fresh project).
    pub fn module_type_and_position(&self) -> Option<(crate::ModuleType, usize)> {
        let pair = match self {
            Self::Lfo(i) => (crate::ModuleType::Lfo, *i as usize),
            Self::Envelope(i) => (crate::ModuleType::Envelope, *i as usize),
            Self::EnvFollower(i) => (crate::ModuleType::EnvelopeFollower, *i as usize),
            Self::KineticPos | Self::KineticVel | Self::KineticAcc => {
                (crate::ModuleType::KineticModulator, 0)
            }
            _ => return None,
        };
        Some(pair)
    }
}

// ============================================================================
// MODULATION DESTINATION
// ============================================================================

/// Modulation destination for a mod matrix slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ModDestination {
    /// No destination (slot inactive).
    #[default]
    None,
    /// Oscillator pitch in semitones (index 0-based).
    OscPitch(u8),
    /// Oscillator level (index 0-based).
    OscLevel(u8),
    /// Filter cutoff in semitones (index 0-based).
    FilterCutoff(u8),
    /// Filter resonance (index 0-based).
    FilterResonance(u8),
    /// Amplifier level (index 0-based).
    AmpLevel(u8),
    /// Amplifier pan (index 0-based).
    AmpPan(u8),
    /// LFO rate (index 0-based).
    LfoRate(u8),
    /// LFO depth (index 0-based).
    LfoDepth(u8),
}

impl ModDestination {
    pub const ALL: [Self; 19] = [
        Self::None,
        Self::OscPitch(0),
        Self::OscLevel(0),
        Self::FilterCutoff(0),
        Self::FilterResonance(0),
        Self::AmpLevel(0),
        Self::AmpPan(0),
        Self::LfoRate(0),
        Self::LfoDepth(0),
        Self::OscPitch(1),
        Self::OscLevel(1),
        Self::FilterCutoff(1),
        Self::FilterResonance(1),
        Self::AmpLevel(1),
        Self::AmpPan(1),
        Self::LfoRate(1),
        Self::LfoDepth(1),
        Self::OscPitch(2),
        Self::OscLevel(2),
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::OscPitch(0) => "Osc 1 Pitch",
            Self::OscPitch(1) => "Osc 2 Pitch",
            Self::OscPitch(2) => "Osc 3 Pitch",
            Self::OscPitch(_) => "Osc Pitch",
            Self::OscLevel(0) => "Osc 1 Level",
            Self::OscLevel(1) => "Osc 2 Level",
            Self::OscLevel(2) => "Osc 3 Level",
            Self::OscLevel(_) => "Osc Level",
            Self::FilterCutoff(0) => "Filter 1 Cutoff",
            Self::FilterCutoff(1) => "Filter 2 Cutoff",
            Self::FilterCutoff(_) => "Filter Cutoff",
            Self::FilterResonance(0) => "Filter 1 Reso",
            Self::FilterResonance(1) => "Filter 2 Reso",
            Self::FilterResonance(_) => "Filter Reso",
            Self::AmpLevel(0) => "Amp 1 Level",
            Self::AmpLevel(1) => "Amp 2 Level",
            Self::AmpLevel(_) => "Amp Level",
            Self::AmpPan(0) => "Amp 1 Pan",
            Self::AmpPan(1) => "Amp 2 Pan",
            Self::AmpPan(_) => "Amp Pan",
            Self::LfoRate(0) => "LFO 1 Rate",
            Self::LfoRate(1) => "LFO 2 Rate",
            Self::LfoRate(_) => "LFO Rate",
            Self::LfoDepth(0) => "LFO 1 Depth",
            Self::LfoDepth(1) => "LFO 2 Depth",
            Self::LfoDepth(_) => "LFO Depth",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::None => "No destination — the slot is inactive.",
            Self::OscPitch(_) => "Oscillator pitch, in semitones.",
            Self::OscLevel(_) => "Oscillator output level.",
            Self::FilterCutoff(_) => "Filter cutoff frequency, in semitones.",
            Self::FilterResonance(_) => "Filter resonance / Q.",
            Self::AmpLevel(_) => "Amplifier output level.",
            Self::AmpPan(_) => "Amplifier stereo pan position.",
            Self::LfoRate(_) => "LFO rate / speed.",
            Self::LfoDepth(_) => "LFO modulation depth.",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OscPitch(0) => "osc1_pitch",
            Self::OscPitch(1) => "osc2_pitch",
            Self::OscPitch(2) => "osc3_pitch",
            Self::OscPitch(_) => "osc_pitch",
            Self::OscLevel(0) => "osc1_level",
            Self::OscLevel(1) => "osc2_level",
            Self::OscLevel(2) => "osc3_level",
            Self::OscLevel(_) => "osc_level",
            Self::FilterCutoff(0) => "flt1_cutoff",
            Self::FilterCutoff(1) => "flt2_cutoff",
            Self::FilterCutoff(_) => "flt_cutoff",
            Self::FilterResonance(0) => "flt1_reso",
            Self::FilterResonance(1) => "flt2_reso",
            Self::FilterResonance(_) => "flt_reso",
            Self::AmpLevel(0) => "amp1_level",
            Self::AmpLevel(1) => "amp2_level",
            Self::AmpLevel(_) => "amp_level",
            Self::AmpPan(0) => "amp1_pan",
            Self::AmpPan(1) => "amp2_pan",
            Self::AmpPan(_) => "amp_pan",
            Self::LfoRate(0) => "lfo1_rate",
            Self::LfoRate(1) => "lfo2_rate",
            Self::LfoRate(_) => "lfo_rate",
            Self::LfoDepth(0) => "lfo1_depth",
            Self::LfoDepth(1) => "lfo2_depth",
            Self::LfoDepth(_) => "lfo_depth",
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|d| d == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|d| {
                crate::module_traits::ChoiceOption::new(d.id(), d.name())
                    .with_description(d.description())
            })
            .collect()
    }

    /// `(ModuleType, positional_index, param_name)` this destination maps
    /// to, or `None` for [`Self::None`]. `positional_index` is 0-based and
    /// refers to the i-th module of `ModuleType` in the containing
    /// instrument — resolve to an actual `ModuleId` via the caller's
    /// per-instrument module enumeration. See
    /// [`ModSource::module_type_and_position`] for the same convention.
    pub fn module_target_position(&self) -> Option<(crate::ModuleType, usize, &'static str)> {
        let (mt, i, param) = match self {
            Self::None => return None,
            Self::OscPitch(i) => (crate::ModuleType::Oscillator, *i, "pitch"),
            Self::OscLevel(i) => (crate::ModuleType::Oscillator, *i, "level"),
            Self::FilterCutoff(i) => (crate::ModuleType::Filter, *i, "cutoff"),
            Self::FilterResonance(i) => (crate::ModuleType::Filter, *i, "resonance"),
            Self::AmpLevel(i) => (crate::ModuleType::Amplifier, *i, "level"),
            Self::AmpPan(i) => (crate::ModuleType::Amplifier, *i, "pan"),
            Self::LfoRate(i) => (crate::ModuleType::Lfo, *i, "rate"),
            Self::LfoDepth(i) => (crate::ModuleType::Lfo, *i, "depth"),
        };
        Some((mt, i as usize, param))
    }
}

// ============================================================================
// DYNAMIC DESTINATION ADDRESS (S1.2)
// ============================================================================

/// A dynamic modulation destination: a specific module instance's parameter.
///
/// The ID-based address that supersedes the fixed [`ModDestination`] enum (S1.2),
/// letting a routing target **any** modulatable parameter on **any** module in the
/// patch — not just the 19 hardcoded roles.
///
/// Serialized as the canonical string `"<prefix>-<instance>.<param>"`
/// (e.g. `"flt-1.cutoff"`). [`DestAddr::parse`] also accepts the **legacy**
/// `ModDestination` ids (`"flt1_cutoff"`), so old projects auto-upgrade on load
/// without re-authoring. `param` reuses the interned [`PortName`](crate::PortName)
/// pool as a copyable parameter identifier (the param's descriptor `type_id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DestAddr {
    pub module_type: crate::ModuleType,
    /// 1-based instance, matching `ModuleId` (e.g. `"flt-1"` → 1).
    pub instance: u16,
    /// Interned parameter identifier (the descriptor `type_id`, e.g. `"cutoff"`).
    pub param: crate::PortName,
}

impl DestAddr {
    /// Build an address from a module type, 1-based instance, and param id.
    #[must_use]
    pub fn new(module_type: crate::ModuleType, instance: u16, param: &str) -> Self {
        Self {
            module_type,
            instance,
            param: crate::PortName::intern(param),
        }
    }

    /// Render to the canonical address string, e.g. `"flt-1.cutoff"`.
    #[must_use]
    pub fn to_address_string(&self) -> String {
        format!(
            "{}-{}.{}",
            self.module_type.prefix(),
            self.instance,
            self.param.as_str()
        )
    }

    /// Parse the canonical new-form address `"flt-1.cutoff"`.
    fn parse_new(s: &str) -> Option<Self> {
        let (module, param) = s.split_once('.')?;
        let (prefix, instance) = module.rsplit_once('-')?;
        if param.is_empty() {
            return None;
        }
        let module_type = crate::ModuleType::from_prefix(prefix)?;
        let instance: u16 = instance.parse().ok()?;
        Some(Self::new(module_type, instance, param))
    }

    /// Parse a stored destination string, accepting **both** the new address
    /// form and a **legacy** [`ModDestination`] id (`"flt1_cutoff"`). Returns
    /// `None` for an empty / `"none"` / unrecognized value.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() || s == "none" {
            return None;
        }
        if let Some(addr) = Self::parse_new(s) {
            return Some(addr);
        }
        // Legacy: map a `ModDestination` id to the equivalent address. The
        // legacy role index is 0-based; `ModuleId` instances are 1-based.
        let legacy = ModDestination::ALL.iter().copied().find(|d| d.id() == s)?;
        let (mt, pos, param) = legacy.module_target_position()?;
        Some(Self::new(mt, pos as u16 + 1, param))
    }
}

// ============================================================================
// MOD MATRIX PARAMETER ENUM (with typed values)
// ============================================================================

/// Slot name lookup table (1-indexed display names).
const SLOT_NAMES_SOURCE: [&str; 16] = [
    "Slot 1 Source",
    "Slot 2 Source",
    "Slot 3 Source",
    "Slot 4 Source",
    "Slot 5 Source",
    "Slot 6 Source",
    "Slot 7 Source",
    "Slot 8 Source",
    "Slot 9 Source",
    "Slot 10 Source",
    "Slot 11 Source",
    "Slot 12 Source",
    "Slot 13 Source",
    "Slot 14 Source",
    "Slot 15 Source",
    "Slot 16 Source",
];

const SLOT_NAMES_DEST: [&str; 16] = [
    "Slot 1 Dest",
    "Slot 2 Dest",
    "Slot 3 Dest",
    "Slot 4 Dest",
    "Slot 5 Dest",
    "Slot 6 Dest",
    "Slot 7 Dest",
    "Slot 8 Dest",
    "Slot 9 Dest",
    "Slot 10 Dest",
    "Slot 11 Dest",
    "Slot 12 Dest",
    "Slot 13 Dest",
    "Slot 14 Dest",
    "Slot 15 Dest",
    "Slot 16 Dest",
];

const SLOT_NAMES_AMOUNT: [&str; 16] = [
    "Slot 1 Amount",
    "Slot 2 Amount",
    "Slot 3 Amount",
    "Slot 4 Amount",
    "Slot 5 Amount",
    "Slot 6 Amount",
    "Slot 7 Amount",
    "Slot 8 Amount",
    "Slot 9 Amount",
    "Slot 10 Amount",
    "Slot 11 Amount",
    "Slot 12 Amount",
    "Slot 13 Amount",
    "Slot 14 Amount",
    "Slot 15 Amount",
    "Slot 16 Amount",
];

const SLOT_NAMES_ENABLED: [&str; 16] = [
    "Slot 1 Enabled",
    "Slot 2 Enabled",
    "Slot 3 Enabled",
    "Slot 4 Enabled",
    "Slot 5 Enabled",
    "Slot 6 Enabled",
    "Slot 7 Enabled",
    "Slot 8 Enabled",
    "Slot 9 Enabled",
    "Slot 10 Enabled",
    "Slot 11 Enabled",
    "Slot 12 Enabled",
    "Slot 13 Enabled",
    "Slot 14 Enabled",
    "Slot 15 Enabled",
    "Slot 16 Enabled",
];

/// Mod matrix parameter with typed value.
///
/// Each slot (0-15) has source, destination, amount, and enabled flag.
/// `GridSize` controls how many slots are visible and processed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ModMatrixParam {
    /// Grid size selector.
    GridSize(ModMatrixGridSize),
    /// Source for slot N.
    SlotSource(u8, ModSource),
    /// Destination for slot N.
    SlotDestination(u8, ModDestination),
    /// Amount for slot N (-1.0 to +1.0).
    SlotAmount(u8, BipolarValue),
    /// Enabled flag for slot N.
    SlotEnabled(u8, bool),
}

impl ModMatrixParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::GridSize(_), Self::GridSize(_)) => true,
            (Self::SlotSource(a, _), Self::SlotSource(b, _)) => a == b,
            (Self::SlotDestination(a, _), Self::SlotDestination(b, _)) => a == b,
            (Self::SlotAmount(a, _), Self::SlotAmount(b, _)) => a == b,
            (Self::SlotEnabled(a, _), Self::SlotEnabled(b, _)) => a == b,
            _ => false,
        }
    }

    /// Get the parameter name.
    #[allow(clippy::cast_possible_truncation)]
    pub fn name(&self) -> &'static str {
        match self {
            Self::GridSize(_) => "Grid Size",
            Self::SlotSource(i, _) => SLOT_NAMES_SOURCE
                .get(*i as usize)
                .copied()
                .unwrap_or("Slot Source"),
            Self::SlotDestination(i, _) => SLOT_NAMES_DEST
                .get(*i as usize)
                .copied()
                .unwrap_or("Slot Dest"),
            Self::SlotAmount(i, _) => SLOT_NAMES_AMOUNT
                .get(*i as usize)
                .copied()
                .unwrap_or("Slot Amount"),
            Self::SlotEnabled(i, _) => SLOT_NAMES_ENABLED
                .get(*i as usize)
                .copied()
                .unwrap_or("Slot Enabled"),
        }
    }

    /// Get the value as f32 (for GUI).
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::GridSize(g) => g.index() as f32,
            Self::SlotSource(_, s) => s.index() as f32,
            Self::SlotDestination(_, d) => d.index() as f32,
            Self::SlotAmount(_, a) => a.as_f32(),
            Self::SlotEnabled(_, e) => {
                if *e {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::GridSize(_) => Self::GridSize(ModMatrixGridSize::from_index(value as usize)),
            Self::SlotSource(slot, _) => {
                Self::SlotSource(*slot, ModSource::from_index(value as usize))
            }
            Self::SlotDestination(slot, _) => {
                Self::SlotDestination(*slot, ModDestination::from_index(value as usize))
            }
            Self::SlotAmount(slot, _) => Self::SlotAmount(*slot, BipolarValue::new(value)),
            Self::SlotEnabled(slot, _) => Self::SlotEnabled(*slot, value > 0.5),
        }
    }

    /// Get the slot index for this parameter (returns 0 for GridSize).
    pub fn slot(&self) -> u8 {
        match self {
            Self::GridSize(_) => 0,
            Self::SlotSource(s, _)
            | Self::SlotDestination(s, _)
            | Self::SlotAmount(s, _)
            | Self::SlotEnabled(s, _) => *s,
        }
    }
}

impl Default for ModMatrixParam {
    fn default() -> Self {
        Self::SlotSource(0, ModSource::default())
    }
}

#[cfg(test)]
mod dest_addr_tests {
    use super::*;
    use crate::ModuleType;

    #[test]
    fn parse_new_form() {
        let a = DestAddr::parse("flt-1.cutoff").unwrap();
        assert_eq!(a.module_type, ModuleType::Filter);
        assert_eq!(a.instance, 1);
        assert_eq!(a.param.as_str(), "cutoff");
    }

    #[test]
    fn parse_legacy_id_upgrades() {
        // Old ModDestination ids are accepted and mapped to the same address.
        // `flt1_cutoff` (1-based role) → flt-1.cutoff.
        assert_eq!(
            DestAddr::parse("flt1_cutoff"),
            Some(DestAddr::new(ModuleType::Filter, 1, "cutoff"))
        );
        // `osc2_pitch` → osc-2.pitch (second oscillator).
        assert_eq!(
            DestAddr::parse("osc2_pitch"),
            Some(DestAddr::new(ModuleType::Oscillator, 2, "pitch"))
        );
    }

    #[test]
    fn round_trips_through_string() {
        let a = DestAddr::new(ModuleType::Amplifier, 3, "pan");
        assert_eq!(a.to_address_string(), "amp-3.pan");
        assert_eq!(DestAddr::parse(&a.to_address_string()), Some(a));
    }

    #[test]
    fn none_and_garbage_parse_to_none() {
        assert_eq!(DestAddr::parse("none"), None);
        assert_eq!(DestAddr::parse(""), None);
        assert_eq!(DestAddr::parse("not_a_real_destination"), None);
        // A bad prefix in new-form falls through and is not a legacy id either.
        assert_eq!(DestAddr::parse("zzz-1.cutoff"), None);
    }
}
