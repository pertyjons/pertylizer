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

impl EuclideanParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Steps(_) => "Steps",
            Self::Pulses(_) => "Pulses",
            Self::Rotation(_) => "Rotation",
            Self::Swing(_) => "Swing",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Steps(n) | Self::Pulses(n) | Self::Rotation(n) => n.as_f32(),
            Self::Swing(v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Steps(_) => Self::Steps(StepCount::new((value.round() as u8).clamp(1, 32))),
            Self::Pulses(_) => Self::Pulses(StepCount::new((value.round() as u8).min(32))),
            Self::Rotation(_) => Self::Rotation(StepCount::new((value.round() as u8).min(31))),
            Self::Swing(_) => Self::Swing(NormalizedValue::new(value)),
        }
    }
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

impl TuringMachineParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::MutationRate(_) => "Mutation",
            Self::Range(_) => "Range",
            Self::Scale(_) => "Scale",
            Self::Length(_) => "Length",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::MutationRate(v) | Self::Range(v) => v.as_f32(),
            Self::Scale(s) => s.index() as f32,
            Self::Length(n) => n.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::MutationRate(_) => Self::MutationRate(NormalizedValue::new(value)),
            Self::Range(_) => Self::Range(NormalizedValue::new(value)),
            Self::Scale(_) => {
                Self::Scale(TuringScale::from_index(value as usize).unwrap_or_default())
            }
            Self::Length(_) => Self::Length(StepCount::new(if value > 12.0 { 16 } else { 8 })),
        }
    }
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

impl RandomGatesParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Density(_) => "Density",
            Self::Seed(_) => "Seed",
            Self::BurstProbability(_) => "Burst",
            Self::GateLength(_) => "Gate Len",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Density(v) | Self::BurstProbability(v) | Self::GateLength(v) => v.as_f32(),
            Self::Seed(s) => *s as f32,
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Density(_) => Self::Density(NormalizedValue::new(value)),
            Self::Seed(_) => Self::Seed(value.round() as u32),
            Self::BurstProbability(_) => Self::BurstProbability(NormalizedValue::new(value)),
            Self::GateLength(_) => Self::GateLength(NormalizedValue::new(value)),
        }
    }
}

impl Default for RandomGatesParam {
    fn default() -> Self {
        Self::Density(NormalizedValue::CENTER)
    }
}
