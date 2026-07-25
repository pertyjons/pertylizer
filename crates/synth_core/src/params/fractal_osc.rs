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

impl Default for FractalOscParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::MAX)
    }
}
