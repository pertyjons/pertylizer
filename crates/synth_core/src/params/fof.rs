//! FOF (Formant Wave Function) parameter types.
//!
//! CHANT-style time-domain voice synthesis: formants are built directly from
//! overlapping granular formant-wave-function grains (Rodet, IRCAM 1984) rather
//! than by filtering a source like `VoiceSynth` does. See `plans/fof-plan.md`
//! for the full design and phased roadmap.

use serde::{Deserialize, Serialize};

use crate::types::{Cents, Hertz, NormalizedValue};

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
    /// Formant bandwidth scale: 0.5 = table value, <0.5 narrower, >0.5 wider.
    Bandwidth(NormalizedValue),
    /// Aspiration / breath noise amount (0.0 = none)
    Breathiness(NormalizedValue),
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

impl Default for FofParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
