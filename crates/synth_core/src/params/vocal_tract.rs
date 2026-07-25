//! Vocal Tract parameter types.
//!
//! Articulatory Kelly–Lochbaum waveguide voice: a glottal source drives a
//! tube whose area profile (tongue constriction) shapes the formants.
//! See `plans/voice-synth-plan.md` (Phase 6) for the design.

use serde::{Deserialize, Serialize};

use crate::types::NormalizedValue;

/// Vocal Tract parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum VocalTractParam {
    /// Tongue / constriction position along the tract (0 = glottis, 1 = lips)
    Tongue(NormalizedValue),
    /// Constriction amount (0 = open tube, 1 = tight constriction)
    Constriction(NormalizedValue),
    /// Lip aperture / rounding (0 = rounded/closed, 1 = spread/open)
    Lips(NormalizedValue),
    /// Tract length / voice type (0 = short tract → soprano/child, 1 = long → bass)
    Length(NormalizedValue),
    /// Nasality / velar port opening (0 = oral, 1 = fully nasal)
    Nasality(NormalizedValue),
    /// Aspiration / breath noise at the glottis (0 = none)
    Breathiness(NormalizedValue),
    /// Output level
    Level(NormalizedValue),
}

impl Default for VocalTractParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
