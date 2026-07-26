//! Formant Filter parameter types.
//!
//! Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Filters/110-formant-filter.rst>
//! From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, NormalizedValue};

/// Formant Filter parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FormantFilterParam {
    /// Vowel morph position (0.0=A, 0.25=E, 0.5=I, 0.75=O, 1.0=U)
    Vowel(NormalizedValue),
    /// Cutoff frequency (scales formant frequencies)
    Cutoff(Hertz),
    /// Resonance / formant sharpness (0.0 to 1.0)
    Resonance(NormalizedValue),
    /// Dry/wet mix (0.0 = dry, 1.0 = wet)
    Mix(NormalizedValue),
}

impl Default for FormantFilterParam {
    fn default() -> Self {
        Self::Vowel(NormalizedValue::MIN)
    }
}
