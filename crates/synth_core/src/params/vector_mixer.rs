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
