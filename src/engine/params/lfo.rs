//! LFO-related parameter types.

use serde::{Deserialize, Serialize};

// ============================================================================
// LFO WAVEFORM ENUM
// ============================================================================

/// LFO waveform types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LfoWaveform {
    #[default]
    Sine,
    Triangle,
    Sawtooth,
    Square,
    SampleAndHold,
}

impl LfoWaveform {
    pub const ALL: [Self; 5] = [
        Self::Sine,
        Self::Triangle,
        Self::Sawtooth,
        Self::Square,
        Self::SampleAndHold,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Sawtooth",
            Self::Square => "Square",
            Self::SampleAndHold => "S&H",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Triangle => "triangle",
            Self::Sawtooth => "sawtooth",
            Self::Square => "square",
            Self::SampleAndHold => "sample_and_hold",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "sine" => Some(Self::Sine),
            "triangle" => Some(Self::Triangle),
            "sawtooth" => Some(Self::Sawtooth),
            "square" => Some(Self::Square),
            "sample_and_hold" | "s&h" => Some(Self::SampleAndHold),
            _ => None,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|w| w == self).unwrap_or(0)
    }
}

// ============================================================================
// LFO PARAMETER ENUM
// ============================================================================

/// LFO parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LfoParam {
    /// LFO waveform
    Waveform,
    /// Rate in Hz
    Rate,
    /// Modulation depth (0.0 to 1.0)
    Depth,
    /// Initial phase (0.0 to 1.0)
    Phase,
    /// Sync to tempo
    TempoSync,
    /// Sync division in beats (0.25 = 1/16, 0.5 = 1/8, 1.0 = 1/4, etc.)
    SyncDivision,
    /// Retrigger on note
    Retrigger,
}
