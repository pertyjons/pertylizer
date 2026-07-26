//! Drift Generator parameter types.
//!
//! Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/183-drift-generator.rst>
//! From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)

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

impl Default for DriftGeneratorParam {
    fn default() -> Self {
        Self::Rate(Hertz::new(0.2))
    }
}
