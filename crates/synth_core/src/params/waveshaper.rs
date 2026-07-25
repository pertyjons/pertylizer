//! Waveshaper effect parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{BipolarValue, NormalizedValue};

// ============================================================================
// WAVESHAPER CURVE ENUM
// ============================================================================

/// Waveshaper curves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum WaveshaperCurve {
    #[default]
    SoftClip,
    Asymmetric,
    Fold,
    Chebyshev,
    SineFold,
    Quantize,
}

impl WaveshaperCurve {
    pub const ALL: [Self; 6] = [
        Self::SoftClip,
        Self::Asymmetric,
        Self::Fold,
        Self::Chebyshev,
        Self::SineFold,
        Self::Quantize,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::SoftClip => "Soft Clip",
            Self::Asymmetric => "Asymmetric",
            Self::Fold => "Fold",
            Self::Chebyshev => "Chebyshev",
            Self::SineFold => "Sine Fold",
            Self::Quantize => "Quantize",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::SoftClip => "Smooth saturation — adds warm harmonics without harsh edges.",
            Self::Asymmetric => {
                "Different shaping for the positive and negative halves — even harmonics, tube-like."
            }
            Self::Fold => "Wavefolding — reflects peaks back for complex, bright harmonics.",
            Self::Chebyshev => "Chebyshev polynomial — generates a specific harmonic series.",
            Self::SineFold => "Sine-based folding — smooth, metallic folded timbres.",
            Self::Quantize => {
                "Steps the signal into discrete levels — bitcrush-style digital grit."
            }
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::SoftClip => "soft_clip",
            Self::Asymmetric => "asymmetric",
            Self::Fold => "fold",
            Self::Chebyshev => "chebyshev",
            Self::SineFold => "sine_fold",
            Self::Quantize => "quantize",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| {
                crate::module_traits::ChoiceOption::new(m.id(), m.name())
                    .with_description(m.description())
            })
            .collect()
    }
}

// ============================================================================
// WAVESHAPER PARAMETER ENUM (with typed values)
// ============================================================================

/// Waveshaper effect parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WaveshaperParam {
    Curve(WaveshaperCurve),
    Drive(NormalizedValue),
    Mix(NormalizedValue),
    Bias(BipolarValue),
    Symmetry(BipolarValue),
}

impl Default for WaveshaperParam {
    fn default() -> Self {
        Self::Drive(NormalizedValue::new(0.3))
    }
}
