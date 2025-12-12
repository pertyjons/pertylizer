//! Noise generator parameter types.

use serde::{Deserialize, Serialize};

use crate::types::Gain;

// ============================================================================
// NOISE TYPE ENUM
// ============================================================================

/// Noise color/type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum NoiseType {
    /// White noise - flat spectrum
    #[default]
    White,
    /// Pink noise - -3dB/octave
    Pink,
    /// Brown noise - -6dB/octave
    Brown,
    /// Blue noise - +3dB/octave
    Blue,
    /// Violet noise - +6dB/octave
    Violet,
}

impl NoiseType {
    pub const ALL: [Self; 5] = [
        Self::White,
        Self::Pink,
        Self::Brown,
        Self::Blue,
        Self::Violet,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::White => "White",
            Self::Pink => "Pink",
            Self::Brown => "Brown",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Pink => "pink",
            Self::Brown => "brown",
            Self::Blue => "blue",
            Self::Violet => "violet",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|t| t == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|t| crate::module_traits::ChoiceOption::new(t.id(), t.name()))
            .collect()
    }
}

// ============================================================================
// NOISE PARAMETER ENUM (with typed values)
// ============================================================================

/// Noise generator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum NoiseParam {
    /// Noise type/color
    Type(NoiseType),
    /// Output level (0.0 to 1.0)
    Level(Gain),
}

impl NoiseParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Type(_) => "Type",
            Self::Level(_) => "Level",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Type(t) => t.index() as f32,
            Self::Level(g) => g.as_f32(),
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Type(_) => Self::Type(NoiseType::from_index(value as usize).unwrap_or_default()),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }

    /// Default templates
    pub fn type_default() -> Self {
        Self::Type(NoiseType::default())
    }
    pub fn level_default() -> Self {
        Self::Level(Gain::UNITY)
    }
}

impl Default for NoiseParam {
    fn default() -> Self {
        Self::Type(NoiseType::default())
    }
}
