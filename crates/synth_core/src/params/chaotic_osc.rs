//! Chaotic Oscillator parameter types.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/184-rossler-and-lorenz-oscillators.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, NormalizedValue};

/// Chaotic system type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChaoticSystem {
    #[default]
    Rossler,
    Lorenz,
}

impl ChaoticSystem {
    pub const ALL: [Self; 2] = [Self::Rossler, Self::Lorenz];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Rossler => "Rössler",
            Self::Lorenz => "Lorenz",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Rossler => {
                "Rössler attractor — a single-scroll chaotic system with smooth, slowly evolving motion."
            }
            Self::Lorenz => {
                "Lorenz attractor — a two-lobe chaotic system with more turbulent motion."
            }
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Rossler => "rossler",
            Self::Lorenz => "lorenz",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|s| s == self).unwrap_or(0)
    }
}

/// Chaotic Oscillator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ChaoticOscParam {
    /// Which chaotic system to use
    System(ChaoticSystem),
    /// Speed of iteration (Hz-like rate)
    Rate(Hertz),
    /// Chaos amount — controls system parameters (0.0 to 1.0)
    Chaos(NormalizedValue),
    /// Output depth (0.0 to 1.0)
    Depth(NormalizedValue),
}

impl Default for ChaoticOscParam {
    fn default() -> Self {
        Self::System(ChaoticSystem::default())
    }
}
