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
