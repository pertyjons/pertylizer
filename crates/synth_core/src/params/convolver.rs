//! Convolution reverb parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{DecayTrim, Milliseconds, NormalizedValue};

/// Impulse response type (mathematically generated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ImpulseResponse {
    #[default]
    Plate,
    Room,
    Spring,
    Hall,
}

impl ImpulseResponse {
    pub const ALL: [Self; 4] = [Self::Plate, Self::Room, Self::Spring, Self::Hall];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Plate => "Plate",
            Self::Room => "Room",
            Self::Spring => "Spring",
            Self::Hall => "Hall",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Room => "room",
            Self::Spring => "spring",
            Self::Hall => "hall",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Plate => "Plate reverb — dense, bright, smooth tail with no early reflections.",
            Self::Room => "Small-room ambience — short, natural reflections.",
            Self::Spring => "Spring reverb — boingy, resonant, vintage character.",
            Self::Hall => "Concert-hall reverb — long, lush, spacious decay.",
        }
    }

    #[must_use]
    pub fn from_index(idx: usize) -> Self {
        Self::ALL.get(idx).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|i| *i == self).unwrap_or(0)
    }
}

/// Convolution reverb parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ConvolverParam {
    /// Impulse response type.
    Ir(ImpulseResponse),
    /// Dry/wet mix.
    Mix(NormalizedValue),
    /// Pre-delay time.
    PreDelay(Milliseconds),
    /// Decay trim (shortens the IR tail).
    DecayTrim(DecayTrim),
    /// Brightness (one-pole LP on wet signal).
    Brightness(NormalizedValue),
    /// Enable dynamic convolution (amplitude-dependent IR crossfade).
    DynamicMode(NormalizedValue),
}

impl Default for ConvolverParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.3))
    }
}
