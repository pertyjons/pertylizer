//! FOF (Formant Wave Function) parameter types.
//!
//! CHANT-style time-domain voice synthesis: formants are built directly from
//! overlapping granular formant-wave-function grains (Rodet, IRCAM 1984) rather
//! than by filtering a source like `VoiceSynth` does. See `plans/fof-plan.md`
//! for the full design and phased roadmap.

use serde::{Deserialize, Serialize};

use crate::types::NormalizedValue;

/// FOF parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FofParam {
    /// Vowel morph position (0.0=A, 0.25=E, 0.5=I, 0.75=O, 1.0=U)
    Vowel(NormalizedValue),
    /// Formant shift / vocal-tract length. Maps 0..1 to a 0.5..2.0 frequency
    /// scale; 0.5 = no shift (short tract = soprano, long = bass).
    FormantShift(NormalizedValue),
    /// Grain excitation time `tex` — the FOF skirt: 0 = sharp attack / wide
    /// bright skirt, 1 = soft attack / narrow duller skirt.
    Skirt(NormalizedValue),
    /// Output level
    Level(NormalizedValue),
}

impl FofParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Vowel(_) => "Vowel",
            Self::FormantShift(_) => "Formant Shift",
            Self::Skirt(_) => "Skirt",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v) | Self::FormantShift(v) | Self::Skirt(v) | Self::Level(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::FormantShift(_) => Self::FormantShift(NormalizedValue::new(value)),
            Self::Skirt(_) => Self::Skirt(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }
}

impl Default for FofParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
