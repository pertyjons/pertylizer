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
