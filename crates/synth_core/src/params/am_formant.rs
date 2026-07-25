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

impl Default for AmFormantParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
