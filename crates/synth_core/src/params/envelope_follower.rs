//! Envelope follower parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{Milliseconds, NormalizedValue};

// ============================================================================
// ENVELOPE FOLLOWER PARAMETER ENUM (with typed values)
// ============================================================================

/// Envelope follower parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EnvelopeFollowerParam {
    /// Attack time — how fast the follower rises (0.1 - 100 ms)
    Attack(Milliseconds),
    /// Release time — how fast the follower falls (1.0 - 1000 ms)
    Release(Milliseconds),
    /// Sensitivity — output scaling (0.0 - 1.0)
    Sensitivity(NormalizedValue),
}

impl EnvelopeFollowerParam {
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Attack(_) => "Attack",
            Self::Release(_) => "Release",
            Self::Sensitivity(_) => "Sensitivity",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Attack(ms) => ms.as_f32(),
            Self::Release(ms) => ms.as_f32(),
            Self::Sensitivity(v) => v.as_f32(),
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Attack(_) => Self::Attack(Milliseconds::new(value)),
            Self::Release(_) => Self::Release(Milliseconds::new(value)),
            Self::Sensitivity(_) => Self::Sensitivity(NormalizedValue::new(value)),
        }
    }

    /// Default templates
    pub fn attack_default() -> Self {
        Self::Attack(Milliseconds::new(5.0))
    }
    pub fn release_default() -> Self {
        Self::Release(Milliseconds::new(50.0))
    }
    pub fn sensitivity_default() -> Self {
        Self::Sensitivity(NormalizedValue::new(0.5))
    }
}

impl Default for EnvelopeFollowerParam {
    fn default() -> Self {
        Self::attack_default()
    }
}
