//! Spectrum Analyzer parameter types.

use serde::{Deserialize, Serialize};

use crate::types::Gain;

/// Spectrum analyzer visualizer parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SpectrumAnalyzerParam {
    /// Vertical gain scaling
    Gain(Gain),
}

impl Default for SpectrumAnalyzerParam {
    fn default() -> Self {
        Self::Gain(Gain::UNITY)
    }
}
