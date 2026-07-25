//! Sub-oscillator parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Gain, Seconds};

// ============================================================================
// SUB-OSCILLATOR WAVEFORM ENUM
// ============================================================================

/// Sub-oscillator waveform types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SubOscWaveform {
    /// Pure sine - smooth, fundamental bass
    #[default]
    Sine,
    /// Triangle wave - soft, rounded bass with a few odd harmonics
    Triangle,
    /// Sawtooth wave - bright, full harmonic spectrum
    Sawtooth,
    /// Square wave - punchy, full harmonics
    Square,
    /// 25% pulse - hollow, distinctive character
    Pulse25,
    /// Band-limited DSF sawtooth - bright but alias-free
    DsfSaw,
}

impl SubOscWaveform {
    pub const ALL: [Self; 6] = [
        Self::Sine,
        Self::Triangle,
        Self::Sawtooth,
        Self::Square,
        Self::Pulse25,
        Self::DsfSaw,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Saw",
            Self::Square => "Square",
            Self::Pulse25 => "Pulse 25%",
            Self::DsfSaw => "DSF Saw",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::Sine => "Pure sine — a smooth, clean fundamental for deep bass.",
            Self::Triangle => "Triangle — soft and rounded, with gentle odd harmonics.",
            Self::Sawtooth => "Sawtooth — bright, with the full harmonic series.",
            Self::Square => "Square wave — punchy, with full odd harmonics.",
            Self::Pulse25 => "25% pulse — a hollow, reedy character.",
            Self::DsfSaw => "DSF saw — bright like a sawtooth but band-limited (alias-free).",
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Triangle => "triangle",
            Self::Sawtooth => "sawtooth",
            Self::Square => "square",
            Self::Pulse25 => "pulse25",
            Self::DsfSaw => "dsf_saw",
        }
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|w| w == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|w| {
                crate::module_traits::ChoiceOption::new(w.id(), w.name())
                    .with_description(w.description())
            })
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

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    pub fn description(&self) -> &'static str {
        match self {
            Self::MinusOne => "One octave below the main oscillator.",
            Self::MinusTwo => "Two octaves below the main oscillator.",
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

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|o| {
                crate::module_traits::ChoiceOption::new(
                    if *o == Self::MinusOne {
                        "minus1"
                    } else {
                        "minus2"
                    },
                    o.name(),
                )
                .with_description(o.description())
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
    /// Per-oscillator glide (portamento) time in seconds (0 = follow the
    /// voice-level glide).
    GlideTime(Seconds),
}

impl SubOscParam {
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
