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
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Waveform(_) => "Waveform",
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Phase(_) => "Phase",
            Self::TempoSync(_) => "Tempo Sync",
            Self::SyncDivision(_) => "Division",
            Self::Retrigger(_) => "Retrigger",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Waveform(w) => w.index() as f32,
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(d) => d.as_f32(),
            Self::Phase(p) => p.as_f32(),
            Self::TempoSync(b) => if *b { 1.0 } else { 0.0 },
            Self::SyncDivision(d) => d.as_f32(),
            Self::Retrigger(b) => if *b { 1.0 } else { 0.0 },
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Waveform(_) => Self::Waveform(
                LfoWaveform::from_index(value as usize).unwrap_or_default()
            ),
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Phase(_) => Self::Phase(Phase::new(value)),
            Self::TempoSync(_) => Self::TempoSync(value > 0.5),
            Self::SyncDivision(_) => Self::SyncDivision(BeatDivision::new(value)),
            Self::Retrigger(_) => Self::Retrigger(value > 0.5),
        }
    }

    /// Default templates
    pub fn waveform_default() -> Self { Self::Waveform(LfoWaveform::default()) }
    pub fn rate_default() -> Self { Self::Rate(Hertz::new(1.0)) }
    pub fn depth_default() -> Self { Self::Depth(NormalizedValue::new(1.0)) }
    pub fn phase_default() -> Self { Self::Phase(Phase::ZERO) }
    pub fn tempo_sync_default() -> Self { Self::TempoSync(false) }
    pub fn sync_division_default() -> Self { Self::SyncDivision(BeatDivision::QUARTER) }
    pub fn retrigger_default() -> Self { Self::Retrigger(false) }
}

impl Default for LfoParam {
    fn default() -> Self {
        Self::Waveform(LfoWaveform::default())
    }
}
