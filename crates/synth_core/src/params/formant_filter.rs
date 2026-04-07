//! Formant Filter parameter types.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/110-formant-filter.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

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

impl FormantFilterParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Vowel(_) => "Vowel",
            Self::Cutoff(_) => "Cutoff",
            Self::Resonance(_) => "Resonance",
            Self::Mix(_) => "Mix",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v) => v.as_f32(),
            Self::Cutoff(hz) => hz.as_f32(),
            Self::Resonance(r) => r.as_f32(),
            Self::Mix(m) => m.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::Cutoff(_) => Self::Cutoff(Hertz::new(value)),
            Self::Resonance(_) => Self::Resonance(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }
}

impl Default for FormantFilterParam {
    fn default() -> Self {
        Self::Vowel(NormalizedValue::MIN)
    }
}
