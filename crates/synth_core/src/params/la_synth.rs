//! LA (Linear Arithmetic) synthesis parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Milliseconds, NormalizedValue};

/// LA synth parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LaSynthParam {
    /// Attack transient type: 0=click, 0.33=noise burst, 0.66=pluck, 1.0=hammer
    AttackType(NormalizedValue),
    /// Duration of attack transient (1-200 ms)
    AttackTime(Milliseconds),
    /// Level of attack transient (0.0-1.0)
    AttackLevel(NormalizedValue),
    /// Crossfade time from attack to sustain input (1-500 ms)
    CrossfadeTime(Milliseconds),
    /// Brightness filter on attack transient (0.0-1.0)
    Brightness(NormalizedValue),
}

impl LaSynthParam {
    pub fn attack_type_default() -> Self {
        Self::AttackType(NormalizedValue::MIN)
    }
    pub fn attack_time_default() -> Self {
        Self::AttackTime(Milliseconds::new(30.0))
    }
    pub fn attack_level_default() -> Self {
        Self::AttackLevel(NormalizedValue::new(0.7))
    }
    pub fn crossfade_time_default() -> Self {
        Self::CrossfadeTime(Milliseconds::new(50.0))
    }
    pub fn brightness_default() -> Self {
        Self::Brightness(NormalizedValue::new(0.5))
    }
}

impl Default for LaSynthParam {
    fn default() -> Self {
        Self::attack_type_default()
    }
}
