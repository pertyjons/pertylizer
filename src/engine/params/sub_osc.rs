//! Sub-oscillator parameter types.

use serde::{Deserialize, Serialize};

use crate::types::Gain;

// ============================================================================
// SUB-OSCILLATOR WAVEFORM ENUM
// ============================================================================

/// Sub-oscillator waveform types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SubOscWaveform {
    /// Pure sine - smooth, fundamental bass
    #[default]
    Sine,
    /// Square wave - punchy, full harmonics
    Square,
    /// 25% pulse - hollow, distinctive character
    Pulse25,
}

impl SubOscWaveform {
    pub const ALL: [Self; 3] = [Self::Sine, Self::Square, Self::Pulse25];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Square => "Square",
            Self::Pulse25 => "Pulse 25%",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Square => "square",
            Self::Pulse25 => "pulse25",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|w| w == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::modules::core::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|w| crate::modules::core::ChoiceOption::new(w.id(), w.name()))
            .collect()
    }
}

// ============================================================================
// SUB-OSCILLATOR OCTAVE ENUM
// ============================================================================

/// Sub-oscillator octave offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SubOscOctave {
    /// One octave down (-1)
    #[default]
    MinusOne,
    /// Two octaves down (-2)
    MinusTwo,
}

impl SubOscOctave {
    pub const ALL: [Self; 2] = [Self::MinusOne, Self::MinusTwo];

    pub fn name(&self) -> &'static str {
        match self {
            Self::MinusOne => "-1 Oct",
            Self::MinusTwo => "-2 Oct",
        }
    }

    /// Get the frequency divisor for this octave.
    pub fn divisor(self) -> f32 {
        match self {
            Self::MinusOne => 2.0,
            Self::MinusTwo => 4.0,
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|o| o == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::modules::core::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|o| {
                crate::modules::core::ChoiceOption::new(
                    if *o == Self::MinusOne {
                        "minus1"
                    } else {
                        "minus2"
                    },
                    o.name(),
                )
            })
            .collect()
    }
}

// ============================================================================
// SUB-OSCILLATOR PARAMETER ENUM (with typed values)
// ============================================================================

/// Sub-oscillator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SubOscParam {
    /// Waveform selection
    Waveform(SubOscWaveform),
    /// Octave transposition
    Octave(SubOscOctave),
    /// Output level (0.0 to 1.0)
    Level(Gain),
}

impl SubOscParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Waveform(_) => "Waveform",
            Self::Octave(_) => "Octave",
            Self::Level(_) => "Level",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Waveform(w) => w.index() as f32,
            Self::Octave(o) => o.index() as f32,
            Self::Level(g) => g.as_f32(),
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Waveform(_) => {
                Self::Waveform(SubOscWaveform::from_index(value as usize).unwrap_or_default())
            }
            Self::Octave(_) => {
                Self::Octave(SubOscOctave::from_index(value as usize).unwrap_or_default())
            }
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }

    /// Default templates
    pub fn waveform_default() -> Self {
        Self::Waveform(SubOscWaveform::default())
    }
    pub fn octave_default() -> Self {
        Self::Octave(SubOscOctave::default())
    }
    pub fn level_default() -> Self {
        Self::Level(Gain::new(0.5))
    }
}

impl Default for SubOscParam {
    fn default() -> Self {
        Self::Waveform(SubOscWaveform::default())
    }
}
