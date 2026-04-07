//! Beat Detector parameter types.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Analysis/200-beat-detector-class.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, Milliseconds, NormalizedValue};

/// Beat Detector parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BeatDetectorParam {
    /// Detection sensitivity / threshold (0.0 to 1.0)
    Sensitivity(NormalizedValue),
    /// Low-pass filter frequency for envelope (Hz)
    FilterFreq(Hertz),
    /// Gate hold time (ms)
    HoldTime(Milliseconds),
}

impl BeatDetectorParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sensitivity(_) => "Sensitivity",
            Self::FilterFreq(_) => "Filter",
            Self::HoldTime(_) => "Hold",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Sensitivity(v) => v.as_f32(),
            Self::FilterFreq(hz) => hz.as_f32(),
            Self::HoldTime(ms) => ms.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Sensitivity(_) => Self::Sensitivity(NormalizedValue::new(value)),
            Self::FilterFreq(_) => Self::FilterFreq(Hertz::new(value)),
            Self::HoldTime(_) => Self::HoldTime(Milliseconds::new(value)),
        }
    }
}

impl Default for BeatDetectorParam {
    fn default() -> Self {
        Self::Sensitivity(NormalizedValue::new(0.5))
    }
}
