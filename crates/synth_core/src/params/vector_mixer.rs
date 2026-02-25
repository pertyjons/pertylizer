//! Vector mixer parameter types.

use serde::{Deserialize, Serialize};

use crate::types::BipolarValue;

/// Vector mixer parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VectorMixerParam {
    /// X position (-1.0 = left, +1.0 = right)
    X(BipolarValue),
    /// Y position (-1.0 = bottom, +1.0 = top)
    Y(BipolarValue),
}

impl VectorMixerParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::X(_) => "X",
            Self::Y(_) => "Y",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::X(v) => v.as_f32(),
            Self::Y(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::X(_) => Self::X(BipolarValue::new(value)),
            Self::Y(_) => Self::Y(BipolarValue::new(value)),
        }
    }

    pub fn x_default() -> Self {
        Self::X(BipolarValue::CENTER)
    }
    pub fn y_default() -> Self {
        Self::Y(BipolarValue::CENTER)
    }
}

impl Default for VectorMixerParam {
    fn default() -> Self {
        Self::x_default()
    }
}
