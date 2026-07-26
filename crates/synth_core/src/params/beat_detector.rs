//! Beat Detector parameter types.
//!
//! Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Analysis/200-beat-detector-class.rst>
//! From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, Milliseconds, NormalizedValue};

/// Beat Detector parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BeatDetectorParam {
    /// Detection sensitivity / threshold (0.0 to 1.0)
    Sensitivity(NormalizedValue),
    /// Low-pass filter frequency for envelope (Hz)
    FilterFreq(Hertz),
    /// Gate hold time (ms)
    HoldTime(Milliseconds),
}

impl Default for BeatDetectorParam {
    fn default() -> Self {
        Self::Sensitivity(NormalizedValue::new(0.5))
    }
}
