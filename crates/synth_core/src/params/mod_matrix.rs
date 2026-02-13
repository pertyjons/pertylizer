//! Mod Matrix parameter types.
//!
//! The mod matrix provides 8 modulation slots, each routing a source
//! to a destination with a bipolar amount control.

use serde::{Deserialize, Serialize};

use crate::types::BipolarValue;

/// Maximum number of mod matrix slots.
pub const MOD_MATRIX_SLOTS: usize = 8;

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
}

impl ModSource {
    pub const ALL: [Self; 10] = [
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
            .map(|s| crate::module_traits::ChoiceOption::new(s.id(), s.name()))
            .collect()
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
    pub const ALL: [Self; 11] = [
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
        Self::FilterCutoff(1),
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::OscPitch(0) => "Osc 1 Pitch",
            Self::OscPitch(1) => "Osc 2 Pitch",
            Self::OscPitch(_) => "Osc Pitch",
            Self::OscLevel(0) => "Osc 1 Level",
            Self::OscLevel(_) => "Osc Level",
            Self::FilterCutoff(0) => "Filter 1 Cutoff",
            Self::FilterCutoff(1) => "Filter 2 Cutoff",
            Self::FilterCutoff(_) => "Filter Cutoff",
            Self::FilterResonance(0) => "Filter 1 Reso",
            Self::FilterResonance(_) => "Filter Reso",
            Self::AmpLevel(0) => "Amp Level",
            Self::AmpLevel(_) => "Amp Level",
            Self::AmpPan(0) => "Amp Pan",
            Self::AmpPan(_) => "Amp Pan",
            Self::LfoRate(0) => "LFO 1 Rate",
            Self::LfoRate(_) => "LFO Rate",
            Self::LfoDepth(0) => "LFO 1 Depth",
            Self::LfoDepth(_) => "LFO Depth",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OscPitch(0) => "osc1_pitch",
            Self::OscPitch(1) => "osc2_pitch",
            Self::OscPitch(_) => "osc_pitch",
            Self::OscLevel(0) => "osc1_level",
            Self::OscLevel(_) => "osc_level",
            Self::FilterCutoff(0) => "flt1_cutoff",
            Self::FilterCutoff(1) => "flt2_cutoff",
            Self::FilterCutoff(_) => "flt_cutoff",
            Self::FilterResonance(0) => "flt1_reso",
            Self::FilterResonance(_) => "flt_reso",
            Self::AmpLevel(0) => "amp_level",
            Self::AmpLevel(_) => "amp_level",
            Self::AmpPan(0) => "amp_pan",
            Self::AmpPan(_) => "amp_pan",
            Self::LfoRate(0) => "lfo1_rate",
            Self::LfoRate(_) => "lfo_rate",
            Self::LfoDepth(0) => "lfo1_depth",
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
            .map(|d| crate::module_traits::ChoiceOption::new(d.id(), d.name()))
            .collect()
    }
}

// ============================================================================
// MOD MATRIX PARAMETER ENUM (with typed values)
// ============================================================================

/// Mod matrix parameter with typed value.
///
/// Each slot (0-7) has source, destination, amount, and enabled state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ModMatrixParam {
    /// Source for slot N.
    SlotSource(u8, ModSource),
    /// Destination for slot N.
    SlotDestination(u8, ModDestination),
    /// Amount for slot N (-1.0 to +1.0).
    SlotAmount(u8, BipolarValue),
    /// Whether slot N is enabled.
    SlotEnabled(u8, bool),
}

impl ModMatrixParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SlotSource(a, _), Self::SlotSource(b, _)) => a == b,
            (Self::SlotDestination(a, _), Self::SlotDestination(b, _)) => a == b,
            (Self::SlotAmount(a, _), Self::SlotAmount(b, _)) => a == b,
            (Self::SlotEnabled(a, _), Self::SlotEnabled(b, _)) => a == b,
            _ => false,
        }
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::SlotSource(0, _) => "Slot 1 Source",
            Self::SlotDestination(0, _) => "Slot 1 Dest",
            Self::SlotAmount(0, _) => "Slot 1 Amount",
            Self::SlotEnabled(0, _) => "Slot 1 On",
            Self::SlotSource(1, _) => "Slot 2 Source",
            Self::SlotDestination(1, _) => "Slot 2 Dest",
            Self::SlotAmount(1, _) => "Slot 2 Amount",
            Self::SlotEnabled(1, _) => "Slot 2 On",
            Self::SlotSource(2, _) => "Slot 3 Source",
            Self::SlotDestination(2, _) => "Slot 3 Dest",
            Self::SlotAmount(2, _) => "Slot 3 Amount",
            Self::SlotEnabled(2, _) => "Slot 3 On",
            Self::SlotSource(3, _) => "Slot 4 Source",
            Self::SlotDestination(3, _) => "Slot 4 Dest",
            Self::SlotAmount(3, _) => "Slot 4 Amount",
            Self::SlotEnabled(3, _) => "Slot 4 On",
            Self::SlotSource(4, _) => "Slot 5 Source",
            Self::SlotDestination(4, _) => "Slot 5 Dest",
            Self::SlotAmount(4, _) => "Slot 5 Amount",
            Self::SlotEnabled(4, _) => "Slot 5 On",
            Self::SlotSource(5, _) => "Slot 6 Source",
            Self::SlotDestination(5, _) => "Slot 6 Dest",
            Self::SlotAmount(5, _) => "Slot 6 Amount",
            Self::SlotEnabled(5, _) => "Slot 6 On",
            Self::SlotSource(6, _) => "Slot 7 Source",
            Self::SlotDestination(6, _) => "Slot 7 Dest",
            Self::SlotAmount(6, _) => "Slot 7 Amount",
            Self::SlotEnabled(6, _) => "Slot 7 On",
            Self::SlotSource(7, _) => "Slot 8 Source",
            Self::SlotDestination(7, _) => "Slot 8 Dest",
            Self::SlotAmount(7, _) => "Slot 8 Amount",
            Self::SlotEnabled(7, _) => "Slot 8 On",
            Self::SlotSource(_, _) => "Slot Source",
            Self::SlotDestination(_, _) => "Slot Dest",
            Self::SlotAmount(_, _) => "Slot Amount",
            Self::SlotEnabled(_, _) => "Slot On",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
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
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
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

    /// Get the slot index for this parameter.
    pub fn slot(&self) -> u8 {
        match self {
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
