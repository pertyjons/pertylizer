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
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
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

impl WaveshaperParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Curve(_) => "Curve",
            Self::Drive(_) => "Drive",
            Self::Mix(_) => "Mix",
            Self::Bias(_) => "Bias",
            Self::Symmetry(_) => "Symmetry",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Curve(c) => c.index() as f32,
            Self::Drive(v) | Self::Mix(v) => v.as_f32(),
            Self::Bias(b) | Self::Symmetry(b) => b.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Curve(_) => {
                Self::Curve(WaveshaperCurve::from_index(value as usize).unwrap_or_default())
            }
            Self::Drive(_) => Self::Drive(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Bias(_) => Self::Bias(BipolarValue::new(value)),
            Self::Symmetry(_) => Self::Symmetry(BipolarValue::new(value)),
        }
    }
}

impl Default for WaveshaperParam {
    fn default() -> Self {
        Self::Drive(NormalizedValue::new(0.3))
    }
}
