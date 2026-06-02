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
    /// Aspiration / breath noise at the glottis (0 = none)
    Breathiness(NormalizedValue),
    /// Output level
    Level(NormalizedValue),
}

impl VocalTractParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Tongue(_) => "Tongue",
            Self::Constriction(_) => "Constriction",
            Self::Lips(_) => "Lips",
            Self::Breathiness(_) => "Breathiness",
            Self::Level(_) => "Level",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Tongue(v)
            | Self::Constriction(v)
            | Self::Lips(v)
            | Self::Breathiness(v)
            | Self::Level(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Tongue(_) => Self::Tongue(NormalizedValue::new(value)),
            Self::Constriction(_) => Self::Constriction(NormalizedValue::new(value)),
            Self::Lips(_) => Self::Lips(NormalizedValue::new(value)),
            Self::Breathiness(_) => Self::Breathiness(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }
}

impl Default for VocalTractParam {
    fn default() -> Self {
        Self::Level(NormalizedValue::new(0.8))
    }
}
