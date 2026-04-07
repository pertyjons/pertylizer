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

impl ChaoticOscParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::System(_) => "System",
            Self::Rate(_) => "Rate",
            Self::Chaos(_) => "Chaos",
            Self::Depth(_) => "Depth",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::System(s) => s.index() as f32,
            Self::Rate(hz) => hz.as_f32(),
            Self::Chaos(c) => c.as_f32(),
            Self::Depth(d) => d.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::System(_) => {
                Self::System(ChaoticSystem::from_index(value as usize).unwrap_or_default())
            }
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Chaos(_) => Self::Chaos(NormalizedValue::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
        }
    }
}

impl Default for ChaoticOscParam {
    fn default() -> Self {
        Self::System(ChaoticSystem::default())
    }
}
