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
