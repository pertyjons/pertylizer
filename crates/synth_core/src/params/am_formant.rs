//! AM Formantic Synthesis parameter types.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/224-am-formantic-synthesis.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use serde::{Deserialize, Serialize};

use crate::types::NormalizedValue;

/// AM Formantic Synthesis parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AmFormantParam {
    /// Vowel morph position (0.0=A, 0.25=E, 0.5=I, 0.75=O, 1.0=U)
    Vowel(NormalizedValue),
    /// Carrier frequency ratio
    CarrierRatio(NormalizedValue),
    /// Modulation depth
    Depth(NormalizedValue),
    /// Output level
    Level(NormalizedValue),
}

impl AmFormantParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Vowel(_) => "Vowel",
            Self::CarrierRatio(_) => "Carrier",
            Self::Depth(_) => "Depth",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v) | Self::CarrierRatio(v) | Self::Depth(v) | Self::Level(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::CarrierRatio(_) => Self::CarrierRatio(NormalizedValue::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }
}

impl Default for AmFormantParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
