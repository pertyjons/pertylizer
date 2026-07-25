//! Envelope-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{BipolarValue, NormalizedValue, Seconds, TimeScale};

/// Envelope (ADSR) parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum EnvelopeParam {
    /// Attack time in seconds
    Attack(Seconds),
    /// Decay time in seconds
    Decay(Seconds),
    /// Sustain level (0.0 to 1.0)
    Sustain(NormalizedValue),
    /// Release time in seconds
    Release(Seconds),
    /// Global multiplier applied to attack, decay, and release times
    TimeScale(TimeScale),
    /// Attack curve shape (-1.0 to 1.0, 0 = linear)
    AttackCurve(BipolarValue),
    /// Decay curve shape
    DecayCurve(BipolarValue),
    /// Release curve shape
    ReleaseCurve(BipolarValue),
    /// Velocity sensitivity (0.0 = none, 1.0 = full)
    VelocitySensitivity(NormalizedValue),
}

impl EnvelopeParam {
    /// Default templates
    pub fn attack_default() -> Self {
        Self::Attack(Seconds::new(0.01))
    }
    pub fn decay_default() -> Self {
        Self::Decay(Seconds::new(0.1))
    }
    pub fn sustain_default() -> Self {
        Self::Sustain(NormalizedValue::new(0.7))
    }
    pub fn release_default() -> Self {
        Self::Release(Seconds::new(0.3))
    }
    pub const fn time_scale_default() -> Self {
        Self::TimeScale(TimeScale::UNITY)
    }
    pub fn attack_curve_default() -> Self {
        Self::AttackCurve(BipolarValue::new(0.0))
    }
    pub fn decay_curve_default() -> Self {
        Self::DecayCurve(BipolarValue::new(0.0))
    }
    pub fn release_curve_default() -> Self {
        Self::ReleaseCurve(BipolarValue::new(0.0))
    }
    pub fn velocity_sensitivity_default() -> Self {
        Self::VelocitySensitivity(NormalizedValue::new(0.5))
    }
}

impl Default for EnvelopeParam {
    fn default() -> Self {
        Self::Attack(Seconds::new(0.01))
    }
}
