//! Spectrum Analyzer parameter types.

use serde::{Deserialize, Serialize};

use crate::types::Gain;

/// Spectrum analyzer visualizer parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SpectrumAnalyzerParam {
    /// Vertical gain scaling
    Gain(Gain),
}

impl SpectrumAnalyzerParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Gain(_) => "Gain",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Gain(g) => g.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Gain(_) => Self::Gain(Gain::new(value)),
        }
    }
}

impl Default for SpectrumAnalyzerParam {
    fn default() -> Self {
        Self::Gain(Gain::UNITY)
    }
}
