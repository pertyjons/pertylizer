//! Generative module parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{NormalizedValue, StepCount};

// ============================================================================
// EUCLIDEAN SEQUENCER
// ============================================================================

/// Euclidean sequencer parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EuclideanParam {
    /// Number of steps (1-32).
    Steps(StepCount),
    /// Number of pulses (0-steps).
    Pulses(StepCount),
    /// Rotation offset (0-steps).
    Rotation(StepCount),
    /// Swing amount.
    Swing(NormalizedValue),
}

impl Default for EuclideanParam {
    fn default() -> Self {
        Self::Steps(StepCount::new(16))
    }
}

// ============================================================================
// TURING MACHINE
// ============================================================================

/// Scale type for Turing Machine quantization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TuringScale {
    #[default]
    Chromatic,
    Major,
    Minor,
    Pentatonic,
}

impl TuringScale {
    pub const ALL: [Self; 4] = [Self::Chromatic, Self::Major, Self::Minor, Self::Pentatonic];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Chromatic => "Chromatic",
            Self::Major => "Major",
            Self::Minor => "Minor",
            Self::Pentatonic => "Pentatonic",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Chromatic => "All twelve semitones — unquantized pitch range.",
            Self::Major => "Quantizes generated pitches to the major scale.",
            Self::Minor => "Quantizes generated pitches to the natural minor scale.",
            Self::Pentatonic => "Quantizes to the five-note pentatonic scale — always consonant.",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Chromatic => "chromatic",
            Self::Major => "major",
            Self::Minor => "minor",
            Self::Pentatonic => "pentatonic",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|s| s == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|s| {
                crate::module_traits::ChoiceOption::new(s.id(), s.name())
                    .with_description(s.description())
            })
            .collect()
    }
}

/// Turing Machine parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TuringMachineParam {
    /// Mutation rate (0=locked, 0.5=chaotic, 1=random).
    MutationRate(NormalizedValue),
    /// Output range.
    Range(NormalizedValue),
    /// Scale quantization.
    Scale(TuringScale),
    /// Number of register bits (8 or 16).
    Length(StepCount),
}

impl Default for TuringMachineParam {
    fn default() -> Self {
        Self::MutationRate(NormalizedValue::CENTER)
    }
}

// ============================================================================
// RANDOM GATES
// ============================================================================

/// Random Gates parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RandomGatesParam {
    /// Gate density (probability of trigger per step).
    Density(NormalizedValue),
    /// Random seed.
    Seed(u32),
    /// Probability of burst (multiple rapid gates).
    BurstProbability(NormalizedValue),
    /// Gate length.
    GateLength(NormalizedValue),
}

impl Default for RandomGatesParam {
    fn default() -> Self {
        Self::Density(NormalizedValue::CENTER)
    }
}
