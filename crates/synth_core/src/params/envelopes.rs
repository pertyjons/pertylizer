//! Envelope-related parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{BipolarValue, NormalizedValue, Seconds};

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
    /// Check if two parameters are the same kind (ignoring values).
    pub fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// Get the parameter name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Attack(_) => "Attack",
            Self::Decay(_) => "Decay",
            Self::Sustain(_) => "Sustain",
            Self::Release(_) => "Release",
            Self::AttackCurve(_) => "Attack Curve",
            Self::DecayCurve(_) => "Decay Curve",
            Self::ReleaseCurve(_) => "Release Curve",
            Self::VelocitySensitivity(_) => "Velocity Sens",
        }
    }

    /// Get the value as f32 (for GUI).
    pub fn as_f32(&self) -> f32 {
        match self {
            Self::Attack(t) => t.as_f32(),
            Self::Decay(t) => t.as_f32(),
            Self::Sustain(v) => v.as_f32(),
            Self::Release(t) => t.as_f32(),
            Self::AttackCurve(c) => c.as_f32(),
            Self::DecayCurve(c) => c.as_f32(),
            Self::ReleaseCurve(c) => c.as_f32(),
            Self::VelocitySensitivity(v) => v.as_f32(),
        }
    }

    /// Create the same parameter variant with a new f32 value (for GUI).
    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Attack(_) => Self::Attack(Seconds::new(value)),
            Self::Decay(_) => Self::Decay(Seconds::new(value)),
            Self::Sustain(_) => Self::Sustain(NormalizedValue::new(value)),
            Self::Release(_) => Self::Release(Seconds::new(value)),
            Self::AttackCurve(_) => Self::AttackCurve(BipolarValue::new(value)),
            Self::DecayCurve(_) => Self::DecayCurve(BipolarValue::new(value)),
            Self::ReleaseCurve(_) => Self::ReleaseCurve(BipolarValue::new(value)),
            Self::VelocitySensitivity(_) => Self::VelocitySensitivity(NormalizedValue::new(value)),
        }
    }

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
