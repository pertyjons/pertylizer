//! Filter-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{BipolarValue, Gain, Hertz, NormalizedValue};

// ============================================================================
// FILTER MODE ENUM
// ============================================================================

/// Filter types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum FilterMode {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Peak,
    LowShelf,
    HighShelf,
}

impl FilterMode {
    pub const ALL: [Self; 7] = [
        Self::Lowpass,
        Self::Highpass,
        Self::Bandpass,
        Self::Notch,
        Self::Peak,
        Self::LowShelf,
        Self::HighShelf,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Lowpass => "Lowpass",
            Self::Highpass => "Highpass",
            Self::Bandpass => "Bandpass",
            Self::Notch => "Notch",
            Self::Peak => "Peak",
            Self::LowShelf => "Low Shelf",
            Self::HighShelf => "High Shelf",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Lowpass => {
                "Passes frequencies below cutoff and attenuates above — removes brightness."
            }
            Self::Highpass => {
                "Passes frequencies above cutoff and attenuates below — removes body and rumble."
            }
            Self::Bandpass => {
                "Passes a band around cutoff, attenuating both sides — narrow and nasal."
            }
            Self::Notch => {
                "Attenuates a narrow band at cutoff, passing the rest — removes one frequency."
            }
            Self::Peak => "Boosts or cuts a band around cutoff — a resonant bell EQ.",
            Self::LowShelf => "Boosts or cuts everything below cutoff — a bass tilt.",
            Self::HighShelf => "Boosts or cuts everything above cutoff — a treble tilt.",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Lowpass => "lowpass",
            Self::Highpass => "highpass",
            Self::Bandpass => "bandpass",
            Self::Notch => "notch",
            Self::Peak => "peak",
            Self::LowShelf => "low_shelf",
            Self::HighShelf => "high_shelf",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "lowpass" | "lp" => Some(Self::Lowpass),
            "highpass" | "hp" => Some(Self::Highpass),
            "bandpass" | "bp" => Some(Self::Bandpass),
            "notch" => Some(Self::Notch),
            "peak" => Some(Self::Peak),
            "low_shelf" => Some(Self::LowShelf),
            "high_shelf" => Some(Self::HighShelf),
            _ => None,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }
}

// ============================================================================
// FILTER MODEL ENUM
// ============================================================================

/// Filter character model.
///
/// Selects the analog filter algorithm used for processing.
/// Standard uses the existing SVF, while Fluid/Screamer/Acid
/// provide distinct analog character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FilterModel {
    #[default]
    Standard,
    Fluid,
    Screamer,
    Acid,
    /// Karlsen fast ladder filter with constant Q.
    /// Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/240-karlsen-fast-ladder.rst
    Karlsen,
}

impl FilterModel {
    pub const ALL: [Self; 5] = [
        Self::Standard,
        Self::Fluid,
        Self::Screamer,
        Self::Acid,
        Self::Karlsen,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Fluid => "Fluid",
            Self::Screamer => "Screamer",
            Self::Acid => "Acid",
            Self::Karlsen => "Karlsen",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Standard => "Clean state-variable filter — neutral, transparent character.",
            Self::Fluid => "Morphing SVF — crossfades LP→BP→HP→Notch via the Morph control.",
            Self::Screamer => "Aggressive, overdriven voicing — screaming resonance.",
            Self::Acid => "TB-303-style ladder voicing — squelchy, acid resonance.",
            Self::Karlsen => {
                "Karlsen fast ladder with constant Q — warm, self-oscillating ladder tone."
            }
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Fluid => "fluid",
            Self::Screamer => "screamer",
            Self::Acid => "acid",
            Self::Karlsen => "karlsen",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "standard" => Some(Self::Standard),
            "fluid" => Some(Self::Fluid),
            "screamer" => Some(Self::Screamer),
            "acid" => Some(Self::Acid),
            "karlsen" => Some(Self::Karlsen),
            _ => None,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }
}

// ============================================================================
// FILTER PARAMETER ENUM (with typed values)
// ============================================================================

/// Filter parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FilterParam {
    /// Filter mode (Lowpass, Highpass, etc.)
    Mode(FilterMode),
    /// Cutoff frequency in Hz
    Cutoff(Hertz),
    /// Resonance/Q (0.0 to 1.0)
    Resonance(NormalizedValue),
    /// Keyboard tracking amount (0.0 to 1.0)
    KeyTracking(NormalizedValue),
    /// Drive/saturation amount (1.0 = unity, > 1.0 = overdrive)
    Drive(Gain),
    /// Envelope amount (-1.0 to 1.0)
    EnvAmount(BipolarValue),
    /// Cutoff CV input attenuverter (-1.0 to 1.0)
    CutoffMod(BipolarValue),
    /// Filter character model (Standard, Fluid, Screamer, Acid)
    Model(FilterModel),
    /// Morph control (Fluid: LP→BP→HP→Notch crossfade)
    Morph(NormalizedValue),
}

impl FilterParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Mode(_) => "Mode",
            Self::Cutoff(_) => "Cutoff",
            Self::Resonance(_) => "Resonance",
            Self::KeyTracking(_) => "Key Tracking",
            Self::Drive(_) => "Drive",
            Self::EnvAmount(_) => "Env Amount",
            Self::CutoffMod(_) => "Cutoff Mod",
            Self::Model(_) => "Model",
            Self::Morph(_) => "Morph",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Mode(m) => m.index() as f32,
            Self::Cutoff(hz) => hz.as_f32(),
            Self::Resonance(r) => r.as_f32(),
            Self::KeyTracking(kt) => kt.as_f32(),
            Self::Drive(d) => d.as_f32(),
            Self::EnvAmount(e) => e.as_f32(),
            Self::CutoffMod(c) => c.as_f32(),
            Self::Model(m) => m.index() as f32,
            Self::Morph(v) => v.as_f32(),
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Mode(_) => Self::Mode(FilterMode::from_index(value as usize).unwrap_or_default()),
            Self::Cutoff(_) => Self::Cutoff(Hertz::new(value)),
            Self::Resonance(_) => Self::Resonance(NormalizedValue::new(value)),
            Self::KeyTracking(_) => Self::KeyTracking(NormalizedValue::new(value)),
            Self::Drive(_) => Self::Drive(Gain::new(value)),
            Self::EnvAmount(_) => Self::EnvAmount(BipolarValue::new(value)),
            Self::CutoffMod(_) => Self::CutoffMod(BipolarValue::new(value)),
            Self::Model(_) => {
                Self::Model(FilterModel::from_index(value as usize).unwrap_or_default())
            }
            Self::Morph(_) => Self::Morph(NormalizedValue::new(value)),
        }
    }

    /// Default templates
    pub fn mode_default() -> Self {
        Self::Mode(FilterMode::default())
    }
    pub fn cutoff_default() -> Self {
        Self::Cutoff(Hertz::new(1000.0))
    }
    pub fn resonance_default() -> Self {
        Self::Resonance(NormalizedValue::new(0.0))
    }
    pub fn key_tracking_default() -> Self {
        Self::KeyTracking(NormalizedValue::new(0.0))
    }
    pub fn drive_default() -> Self {
        Self::Drive(Gain::UNITY)
    }
    pub fn env_amount_default() -> Self {
        Self::EnvAmount(BipolarValue::new(0.0))
    }
    pub fn cutoff_mod_default() -> Self {
        Self::CutoffMod(BipolarValue::MAX)
    }
    pub fn model_default() -> Self {
        Self::Model(FilterModel::Standard)
    }
    pub fn morph_default() -> Self {
        Self::Morph(NormalizedValue::MIN)
    }
}

impl Default for FilterParam {
    fn default() -> Self {
        Self::Mode(FilterMode::default())
    }
}
