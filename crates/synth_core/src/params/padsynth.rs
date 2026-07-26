//! PADsynth parameter types.
//!
//! Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/213-padsynth-synthesys-method.rst>
//! From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, NormalizedValue};

/// PADsynth parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PadSynthParam {
    /// Bandwidth of each partial in cents (controls "spread")
    Bandwidth(NormalizedValue),
    /// Harmonic tilt/rolloff per octave
    Tilt(NormalizedValue),
    /// Detune randomness (0 = pure harmonics, 1 = inharmonic)
    Detune(NormalizedValue),
    /// Base frequency
    BaseFreq(Hertz),
    /// Output level
    Level(NormalizedValue),
}

impl Default for PadSynthParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
