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

impl AdditiveParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Tilt(_) => "Tilt",
            Self::OddEven(_) => "Odd/Even",
            Self::Brightness(_) => "Brightness",
            Self::Stretch(_) => "Stretch",
            Self::Randomize(_) => "Randomize",
            Self::Level(_) => "Level",
            Self::GlideTime(_) => "Glide",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Tilt(v)
            | Self::OddEven(v)
            | Self::Brightness(v)
            | Self::Stretch(v)
            | Self::Randomize(v)
            | Self::Level(v) => v.as_f32(),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Tilt(_) => Self::Tilt(NormalizedValue::new(value)),
            Self::OddEven(_) => Self::OddEven(NormalizedValue::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
            Self::Stretch(_) => Self::Stretch(NormalizedValue::new(value)),
            Self::Randomize(_) => Self::Randomize(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }
}

impl Default for AdditiveParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::MAX)
    }
}
