//! Additive synthesis parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{NormalizedValue, Seconds};

/// Additive oscillator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AdditiveParam {
    /// Spectral tilt — rolloff per octave (0=flat, 1=steep).
    Tilt(NormalizedValue),
    /// Odd/even harmonic balance (0=odd only, 0.5=equal, 1=even only).
    OddEven(NormalizedValue),
    /// Brightness — high harmonic boost (0=dark, 1=bright).
    Brightness(NormalizedValue),
    /// Spectral stretch — inharmonicity (0=harmonic, 1=stretched).
    Stretch(NormalizedValue),
    /// Phase randomization amount on note-on.
    Randomize(NormalizedValue),
    /// Output level.
    Level(NormalizedValue),
    /// Per-oscillator glide (portamento) time in seconds (0 = follow the
    /// voice-level glide).
    GlideTime(Seconds),
}

impl Default for AdditiveParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::MAX)
    }
}
