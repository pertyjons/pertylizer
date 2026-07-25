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

impl Default for FooglersParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.5))
    }
}
