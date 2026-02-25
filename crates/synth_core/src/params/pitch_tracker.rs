//! Pitch tracker parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, NormalizedValue};

/// Pitch tracker parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PitchTrackerParam {
    /// Gate threshold sensitivity (0.0-1.0)
    Sensitivity(NormalizedValue),
    /// Minimum trackable frequency (20-500 Hz)
    MinFreq(Hertz),
    /// Maximum trackable frequency (200-8000 Hz)
    MaxFreq(Hertz),
    /// Output smoothing amount (0.0-1.0)
    Smoothing(NormalizedValue),
}

impl PitchTrackerParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sensitivity(_) => "Sensitivity",
            Self::MinFreq(_) => "Min Freq",
            Self::MaxFreq(_) => "Max Freq",
            Self::Smoothing(_) => "Smoothing",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Sensitivity(v) => v.as_f32(),
            Self::MinFreq(h) => h.as_f32(),
            Self::MaxFreq(h) => h.as_f32(),
            Self::Smoothing(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Sensitivity(_) => Self::Sensitivity(NormalizedValue::new(value)),
            Self::MinFreq(_) => Self::MinFreq(Hertz::new(value)),
            Self::MaxFreq(_) => Self::MaxFreq(Hertz::new(value)),
            Self::Smoothing(_) => Self::Smoothing(NormalizedValue::new(value)),
        }
    }

    pub fn sensitivity_default() -> Self {
        Self::Sensitivity(NormalizedValue::new(0.5))
    }
    pub fn min_freq_default() -> Self {
        Self::MinFreq(Hertz::new(50.0))
    }
    pub fn max_freq_default() -> Self {
        Self::MaxFreq(Hertz::new(2000.0))
    }
    pub fn smoothing_default() -> Self {
        Self::Smoothing(NormalizedValue::new(0.3))
    }
}

impl Default for PitchTrackerParam {
    fn default() -> Self {
        Self::sensitivity_default()
    }
}
