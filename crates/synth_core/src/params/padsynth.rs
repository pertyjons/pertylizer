//! PADsynth parameter types.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/213-padsynth-synthesys-method.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, NormalizedValue};

/// PADsynth parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PadSynthParam {
    /// Bandwidth of each partial in cents (controls "spread")
    Bandwidth(NormalizedValue),
    /// Harmonic tilt/rolloff per octave
    Tilt(NormalizedValue),
    /// Detune randomness (0 = pure harmonics, 1 = inharmonic)
    Detune(NormalizedValue),
    /// Base frequency
    BaseFreq(Hertz),
    /// Output level
    Level(NormalizedValue),
}

impl PadSynthParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Bandwidth(_) => "Bandwidth",
            Self::Tilt(_) => "Tilt",
            Self::Detune(_) => "Detune",
            Self::BaseFreq(_) => "Base Freq",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Bandwidth(v) | Self::Tilt(v) | Self::Detune(v) | Self::Level(v) => v.as_f32(),
            Self::BaseFreq(hz) => hz.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Bandwidth(_) => Self::Bandwidth(NormalizedValue::new(value)),
            Self::Tilt(_) => Self::Tilt(NormalizedValue::new(value)),
            Self::Detune(_) => Self::Detune(NormalizedValue::new(value)),
            Self::BaseFreq(_) => Self::BaseFreq(Hertz::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }
}

impl Default for PadSynthParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
