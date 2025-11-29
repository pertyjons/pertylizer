//! Envelope-related parameter types.

use serde::{Deserialize, Serialize};

/// Envelope (ADSR) parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnvelopeParam {
    /// Attack time in seconds
    Attack,
    /// Decay time in seconds
    Decay,
    /// Sustain level (0.0 to 1.0)
    Sustain,
    /// Release time in seconds
    Release,
    /// Attack curve shape (-1.0 to 1.0, 0 = linear)
    AttackCurve,
    /// Decay curve shape
    DecayCurve,
    /// Release curve shape
    ReleaseCurve,
    /// Velocity sensitivity (0.0 = none, 1.0 = full)
    VelocitySensitivity,
}
