//! Frequency shifter parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, NormalizedValue};

/// Frequency shifter parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FrequencyShifterParam {
    /// Shift amount in Hz (-1000 to +1000)
    Shift(Hertz),
    /// Dry/wet mix (0.0 = dry, 1.0 = wet)
    Mix(NormalizedValue),
    /// Mode: 0=up, 0.5=down, 1.0=stereo (up L, down R)
    Mode(NormalizedValue),
}

impl FrequencyShifterParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Shift(_) => "Shift",
            Self::Mix(_) => "Mix",
            Self::Mode(_) => "Mode",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Shift(h) => h.as_f32(),
            Self::Mix(v) => v.as_f32(),
            Self::Mode(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Shift(_) => Self::Shift(Hertz::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Mode(_) => Self::Mode(NormalizedValue::new(value)),
        }
    }

    pub fn shift_default() -> Self {
        Self::Shift(Hertz::new(0.0))
    }
    pub fn mix_default() -> Self {
        Self::Mix(NormalizedValue::MAX)
    }
    pub fn mode_default() -> Self {
        Self::Mode(NormalizedValue::MIN)
    }
}

impl Default for FrequencyShifterParam {
    fn default() -> Self {
        Self::shift_default()
    }
}
