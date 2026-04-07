//! Drift Generator parameter types.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/183-drift-generator.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, NormalizedValue};

/// Drift Generator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DriftGeneratorParam {
    /// Drift rate — how fast the value wanders (Hz)
    Rate(Hertz),
    /// Drift depth — output amplitude (0.0 to 1.0)
    Depth(NormalizedValue),
    /// Smoothness — higher = smoother transitions (0.0 to 1.0)
    Smoothness(NormalizedValue),
}

impl DriftGeneratorParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Smoothness(_) => "Smoothness",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(d) => d.as_f32(),
            Self::Smoothness(s) => s.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Smoothness(_) => Self::Smoothness(NormalizedValue::new(value)),
        }
    }
}

impl Default for DriftGeneratorParam {
    fn default() -> Self {
        Self::Rate(Hertz::new(0.2))
    }
}
