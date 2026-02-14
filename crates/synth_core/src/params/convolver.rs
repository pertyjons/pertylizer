//! Convolution reverb parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Milliseconds, NormalizedValue};

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
    DecayTrim(NormalizedValue),
    /// Brightness (one-pole LP on wet signal).
    Brightness(NormalizedValue),
}

impl ConvolverParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Ir(_) => "IR Type",
            Self::Mix(_) => "Mix",
            Self::PreDelay(_) => "Pre-Delay",
            Self::DecayTrim(_) => "Decay",
            Self::Brightness(_) => "Brightness",
        }
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Ir(ir) => ir.index() as f32,
            Self::Mix(v) | Self::DecayTrim(v) | Self::Brightness(v) => v.as_f32(),
            Self::PreDelay(ms) => ms.as_f32(),
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Ir(_) => Self::Ir(ImpulseResponse::from_index(value as usize)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::PreDelay(_) => Self::PreDelay(Milliseconds::new(value)),
            Self::DecayTrim(_) => Self::DecayTrim(NormalizedValue::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
        }
    }
}

impl Default for ConvolverParam {
    fn default() -> Self {
        Self::Mix(NormalizedValue::new(0.3))
    }
}
