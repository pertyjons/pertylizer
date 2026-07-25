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

impl Default for VoiceSynthParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
