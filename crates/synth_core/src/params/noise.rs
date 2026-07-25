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
    /// Clocked linear-feedback shift register noise - retro digital texture
    Lfsr,
    /// SID-inspired chip noise from a 23-bit shift register
    Chip,
}

impl NoiseType {
    pub const ALL: [Self; 7] = [
        Self::White,
        Self::Pink,
        Self::Brown,
        Self::Blue,
        Self::Violet,
        Self::Lfsr,
        Self::Chip,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::White => "White",
            Self::Pink => "Pink",
            Self::Brown => "Brown",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
            Self::Lfsr => "LFSR",
            Self::Chip => "Chip",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::White => "Flat spectrum, equal energy per frequency — bright, full hiss.",
            Self::Pink => "−3 dB/octave, equal energy per octave — natural, balanced hiss.",
            Self::Brown => "−6 dB/octave — dark, rumbling low-end noise.",
            Self::Blue => "+3 dB/octave — bright, airy hiss tilted toward the highs.",
            Self::Violet => "+6 dB/octave — very bright, sharp high-frequency noise.",
            Self::Lfsr => {
                "Deterministic linear-feedback shift register — gritty retro digital noise (NES-style)."
            }
            Self::Chip => {
                "SID-inspired 23-bit shift register — sparse, metallic chip noise, great for drums and FX."
            }
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Pink => "pink",
            Self::Brown => "brown",
            Self::Blue => "blue",
            Self::Violet => "violet",
            Self::Lfsr => "lfsr",
            Self::Chip => "chip",
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
            .map(|t| {
                crate::module_traits::ChoiceOption::new(t.id(), t.name())
                    .with_description(t.description())
            })
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
