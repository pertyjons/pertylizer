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
}

impl Default for FilterParam {
    fn default() -> Self {
        Self::Mode(FilterMode::default())
    }
}
