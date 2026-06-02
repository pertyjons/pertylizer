//! Voice Synth parameter types.
//!
//! Physically-inspired singing-voice source–filter model: a glottal pulse
//! (LF/Rosenberg family) excites a bank of formant resonators tuned to a vowel.
//! See `plans/voice-synth-plan.md` for the full design and phased roadmap.

use serde::{Deserialize, Serialize};

use crate::types::{Cents, Hertz, NormalizedValue};

/// Voice Synth parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VoiceSynthParam {
    /// Vowel morph position (0.0=A, 0.25=E, 0.5=I, 0.75=O, 1.0=U)
    Vowel(NormalizedValue),
    /// Formant shift / vocal-tract length (0.5=down an octave, 0.5→1.0 center, 1.0=up).
    /// Maps 0..1 to a 0.5..2.0 frequency scale; 0.5 = no shift.
    FormantShift(NormalizedValue),
    /// Aspiration / breath noise amount (0.0 = none)
    Breathiness(NormalizedValue),
    /// Glottal open quotient — soft/breathy (low) ↔ pressed (high)
    OpenQuotient(NormalizedValue),
    /// Spectral tilt — 0.0 bright, 1.0 dark (soft glottal closure)
    Tilt(NormalizedValue),
    /// Vibrato rate
    VibratoRate(Hertz),
    /// Vibrato depth in cents
    VibratoDepth(Cents),
    /// Choir size: number of decorrelated unison sub-voices (1..16)
    UnisonVoices(NormalizedValue),
    /// Unison detune spread in cents
    UnisonDetune(Cents),
    /// Unison stereo spread (0 = mono, 1 = full width)
    UnisonSpread(NormalizedValue),
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
            Self::Breathiness(_) => "Breathiness",
            Self::OpenQuotient(_) => "Open Quotient",
            Self::Tilt(_) => "Tilt",
            Self::VibratoRate(_) => "Vibrato Rate",
            Self::VibratoDepth(_) => "Vibrato Depth",
            Self::UnisonVoices(_) => "Unison Voices",
            Self::UnisonDetune(_) => "Unison Detune",
            Self::UnisonSpread(_) => "Unison Spread",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Breathiness(v)
            | Self::OpenQuotient(v)
            | Self::Tilt(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.as_f32(),
            Self::VibratoRate(hz) => hz.as_f32(),
            Self::VibratoDepth(c) | Self::UnisonDetune(c) => c.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::FormantShift(_) => Self::FormantShift(NormalizedValue::new(value)),
            Self::Breathiness(_) => Self::Breathiness(NormalizedValue::new(value)),
            Self::OpenQuotient(_) => Self::OpenQuotient(NormalizedValue::new(value)),
            Self::Tilt(_) => Self::Tilt(NormalizedValue::new(value)),
            Self::VibratoRate(_) => Self::VibratoRate(Hertz::new(value)),
            Self::VibratoDepth(_) => Self::VibratoDepth(Cents::new(value)),
            Self::UnisonVoices(_) => Self::UnisonVoices(NormalizedValue::new(value)),
            Self::UnisonDetune(_) => Self::UnisonDetune(Cents::new(value)),
            Self::UnisonSpread(_) => Self::UnisonSpread(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }
}

impl Default for VoiceSynthParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
