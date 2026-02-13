//! Wavetable oscillator parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Cents, Gain, NormalizedValue};

// ============================================================================
// WAVETABLE SELECT ENUM
// ============================================================================

/// Available wavetable banks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WavetableSelect {
    /// Sine -> Triangle -> Saw -> Square morph
    #[default]
    Basic,
    /// 1 to 32 harmonics additive
    Harmonics,
    /// Pulse width 50% -> 5%
    Pwm,
    /// Vowel formants a/e/i/o/u
    Formant,
    /// Mathematical waveforms
    Digital,
    /// Soft analog-style variants
    Warm,
}

impl WavetableSelect {
    pub const ALL: [Self; 6] = [
        Self::Basic,
        Self::Harmonics,
        Self::Pwm,
        Self::Formant,
        Self::Digital,
        Self::Warm,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::Harmonics => "Harmonics",
            Self::Pwm => "PWM",
            Self::Formant => "Formant",
            Self::Digital => "Digital",
            Self::Warm => "Warm",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Harmonics => "harmonics",
            Self::Pwm => "pwm",
            Self::Formant => "formant",
            Self::Digital => "digital",
            Self::Warm => "warm",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "basic" => Some(Self::Basic),
            "harmonics" => Some(Self::Harmonics),
            "pwm" => Some(Self::Pwm),
            "formant" => Some(Self::Formant),
            "digital" => Some(Self::Digital),
            "warm" => Some(Self::Warm),
            _ => None,
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
// WAVETABLE PARAMETER ENUM (with typed values)
// ============================================================================

/// Wavetable oscillator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WavetableParam {
    /// Wavetable bank selection
    Table(WavetableSelect),
    /// Scan position within the wavetable (0.0 - 1.0)
    Position(NormalizedValue),
    /// Detune in cents (-100 to +100)
    Detune(Cents),
    /// Octave offset (-2 to +2)
    Octave(i8),
    /// Output level (0.0 - 1.0)
    Level(Gain),
}

impl WavetableParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Table(_) => "Table",
            Self::Position(_) => "Position",
            Self::Detune(_) => "Detune",
            Self::Octave(_) => "Octave",
            Self::Level(_) => "Level",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Table(t) => t.index() as f32,
            Self::Position(v) => v.as_f32(),
            Self::Detune(c) => c.as_f32(),
            Self::Octave(o) => f32::from(*o),
            Self::Level(g) => g.as_f32(),
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    #[allow(clippy::cast_possible_truncation)]
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Table(_) => {
                Self::Table(WavetableSelect::from_index(value as usize).unwrap_or_default())
            }
            Self::Position(_) => Self::Position(NormalizedValue::new(value)),
            Self::Detune(_) => Self::Detune(Cents::new(value)),
            Self::Octave(_) => Self::Octave(value.round() as i8),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }

    /// Default templates
    pub fn table_default() -> Self {
        Self::Table(WavetableSelect::default())
    }
    pub fn position_default() -> Self {
        Self::Position(NormalizedValue::MIN)
    }
    pub fn detune_default() -> Self {
        Self::Detune(Cents::ZERO)
    }
    pub fn octave_default() -> Self {
        Self::Octave(0)
    }
    pub fn level_default() -> Self {
        Self::Level(Gain::new(0.8))
    }
}

impl Default for WavetableParam {
    fn default() -> Self {
        Self::table_default()
    }
}
