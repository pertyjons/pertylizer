//! Fractal oscillator parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{NormalizedValue, Seconds};

/// Fractal oscillator parameter with typed value.
///
/// Controls for the Weierstrass-function-based stereo oscillator.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FractalOscParam {
    /// Roughness (a) — amplitude scaling per partial (0.0–0.99).
    Roughness(NormalizedValue),
    /// Fractal spacing (b) — frequency ratio between partials.
    /// GUI range 0.0–1.0 maps to actual range 1.01–10.0.
    FractalSpacing(NormalizedValue),
    /// Dispersion — static phase offset per partial for crest control.
    Dispersion(NormalizedValue),
    /// Stereo spread — even partials left, odd partials right.
    Spread(NormalizedValue),
    /// Output level.
    Level(NormalizedValue),
    /// Per-oscillator glide (portamento) time in seconds (0 = follow the
    /// voice-level glide).
    GlideTime(Seconds),
}

impl FractalOscParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Roughness(_) => "Roughness",
            Self::FractalSpacing(_) => "Spacing",
            Self::Dispersion(_) => "Dispersion",
            Self::Spread(_) => "Spread",
            Self::Level(_) => "Level",
            Self::GlideTime(_) => "Glide",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Roughness(v)
            | Self::FractalSpacing(v)
            | Self::Dispersion(v)
            | Self::Spread(v)
            | Self::Level(v) => v.as_f32(),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Roughness(_) => Self::Roughness(NormalizedValue::new(value)),
            Self::FractalSpacing(_) => Self::FractalSpacing(NormalizedValue::new(value)),
            Self::Dispersion(_) => Self::Dispersion(NormalizedValue::new(value)),
            Self::Spread(_) => Self::Spread(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }
}

impl Default for FractalOscParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::MAX)
    }
}
