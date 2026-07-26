//! LFO-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{BeatDivision, Hertz, NormalizedValue, Phase};

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
    /// Smooth random: continuous random modulation without discontinuities.
    /// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/269-smooth-random-lfo-generator.rst>
    SmoothRandom,
}

impl LfoWaveform {
    pub const ALL: [Self; 6] = [
        Self::Sine,
        Self::Triangle,
        Self::Sawtooth,
        Self::Square,
        Self::SampleAndHold,
        Self::SmoothRandom,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Sawtooth",
            Self::Square => "Square",
            Self::SampleAndHold => "Sample & Hold",
            Self::SmoothRandom => "Smooth Random",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Sine => "Smooth sinusoidal sweep up and down.",
            Self::Triangle => "Linear ramp up then down — even, symmetric motion.",
            Self::Sawtooth => "Linear ramp that resets sharply — ramp-style modulation.",
            Self::Square => "Instant switch between two levels — on/off, trill-like.",
            Self::SampleAndHold => "Random stepped value held each cycle — classic S&H jitter.",
            Self::SmoothRandom => {
                "Continuously interpolated random motion — drifting, with no sudden jumps."
            }
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Triangle => "triangle",
            Self::Sawtooth => "sawtooth",
            Self::Square => "square",
            Self::SampleAndHold => "sample_and_hold",
            Self::SmoothRandom => "smooth_random",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "sine" => Some(Self::Sine),
            "triangle" => Some(Self::Triangle),
            "sawtooth" => Some(Self::Sawtooth),
            "square" => Some(Self::Square),
            "sample_and_hold" | "s&h" => Some(Self::SampleAndHold),
            "smooth_random" | "random" => Some(Self::SmoothRandom),
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
// LFO PARAMETER ENUM (with typed values)
// ============================================================================

/// LFO parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LfoParam {
    /// LFO waveform
    Waveform(LfoWaveform),
    /// Rate in Hz
    Rate(Hertz),
    /// Modulation depth (0.0 to 1.0)
    Depth(NormalizedValue),
    /// Initial phase (0.0 to 1.0)
    Phase(Phase),
    /// Sync to tempo
    TempoSync(bool),
    /// Sync division in beats
    SyncDivision(BeatDivision),
    /// Retrigger on note
    Retrigger(bool),
}

impl LfoParam {
    /// Default templates
    pub fn waveform_default() -> Self {
        Self::Waveform(LfoWaveform::default())
    }
    pub fn rate_default() -> Self {
        Self::Rate(Hertz::new(1.0))
    }
    pub fn depth_default() -> Self {
        Self::Depth(NormalizedValue::new(1.0))
    }
    pub fn phase_default() -> Self {
        Self::Phase(Phase::ZERO)
    }
    pub fn tempo_sync_default() -> Self {
        Self::TempoSync(false)
    }
    pub fn sync_division_default() -> Self {
        Self::SyncDivision(BeatDivision::QUARTER)
    }
    pub fn retrigger_default() -> Self {
        Self::Retrigger(false)
    }
}

impl Default for LfoParam {
    fn default() -> Self {
        Self::Waveform(LfoWaveform::default())
    }
}
