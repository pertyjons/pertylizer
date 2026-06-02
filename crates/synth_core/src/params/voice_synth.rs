//! Voice Synth parameter types.
//!
//! Physically-inspired singing-voice source–filter model: a glottal pulse
//! (LF/Rosenberg family) excites a bank of formant resonators tuned to a vowel.
//! See `plans/voice-synth-plan.md` for the full design and phased roadmap.

use serde::{Deserialize, Serialize};

use crate::types::NormalizedValue;

/// Voice Synth parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VoiceSynthParam {
    /// Vowel morph position (0.0=A, 0.25=E, 0.5=I, 0.75=O, 1.0=U)
    Vowel(NormalizedValue),
    /// Formant shift / vocal-tract length (0.5=down an octave, 0.5→1.0 center, 1.0=up).
    /// Maps 0..1 to a 0.5..2.0 frequency scale; 0.5 = no shift.
    FormantShift(NormalizedValue),
    /// Output level
    Level(NormalizedValue),
}

impl VoiceSynthParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Vowel(_) => "Vowel",
            Self::FormantShift(_) => "Formant Shift",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v) | Self::FormantShift(v) | Self::Level(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::FormantShift(_) => Self::FormantShift(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }
}

impl Default for VoiceSynthParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
