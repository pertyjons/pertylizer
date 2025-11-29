//! Filter-related parameter types.

use serde::{Deserialize, Serialize};

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
// FILTER PARAMETER ENUM
// ============================================================================

/// Filter parameters - only valid for filter modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterParam {
    /// Filter mode (Lowpass, Highpass, etc.)
    Mode,
    /// Cutoff frequency in Hz
    Cutoff,
    /// Resonance/Q (0.0 to 1.0, or actual Q value)
    Resonance,
    /// Keyboard tracking amount (0.0 to 1.0)
    KeyTracking,
    /// Drive/saturation amount
    Drive,
    /// Envelope amount (-1.0 to 1.0, scales envelope CV input)
    EnvAmount,
}
