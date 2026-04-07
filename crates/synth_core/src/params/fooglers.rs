//! Fooglers (Weird Synthesis) parameter types.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/11-weird-synthesis.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use serde::{Deserialize, Serialize};

use crate::types::NormalizedValue;

/// Fooglers parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FooglersParam {
    /// Tap 1 position (0.0 to 1.0, as fraction of buffer)
    Tap1(NormalizedValue),
    /// Tap 2 position (0.0 to 1.0, as fraction of buffer)
    Tap2(NormalizedValue),
    /// Feedback amount (0.0 to 0.99)
    Feedback(NormalizedValue),
    /// High-frequency damping (0.0 = no damping, 1.0 = heavy)
    Damping(NormalizedValue),
    /// Output level
    Level(NormalizedValue),
}

impl FooglersParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Tap1(_) => "Tap 1",
            Self::Tap2(_) => "Tap 2",
            Self::Feedback(_) => "Feedback",
            Self::Damping(_) => "Damping",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Tap1(v)
            | Self::Tap2(v)
            | Self::Feedback(v)
            | Self::Damping(v)
            | Self::Level(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Tap1(_) => Self::Tap1(NormalizedValue::new(value)),
            Self::Tap2(_) => Self::Tap2(NormalizedValue::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }
}

impl Default for FooglersParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.5))
    }
}
