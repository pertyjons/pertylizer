//! Normalized value types for type-safe audio processing.

use std::ops::{Add, Mul, Neg, Sub};

use super::Clampable;

/// A value normalized to the range [0.0, 1.0].
///
/// Used for:
/// - Knob positions
/// - Envelope levels
/// - Mix amounts
/// - Pulse width
/// - Any parameter where 0 = min and 1 = max
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct NormalizedValue(pub f32);

impl NormalizedValue {
    /// Create a new normalized value, clamping to [0, 1].
    #[inline]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    /// Create without clamping (for performance in hot paths).
    ///
    /// # Safety
    /// Caller must ensure value is in [0, 1].
    #[inline]
    pub const fn new_unchecked(value: f32) -> Self {
        Self(value)
    }

    /// Minimum value (0).
    pub const MIN: Self = Self(0.0);

    /// Maximum value (1).
    pub const MAX: Self = Self(1.0);

    /// Center value (0.5).
    pub const CENTER: Self = Self(0.5);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Scale to a range [min, max].
    #[inline]
    pub fn scale(self, min: f32, max: f32) -> f32 {
        min + self.0 * (max - min)
    }

    /// Scale logarithmically (useful for frequencies).
    #[inline]
    pub fn scale_log(self, min: f32, max: f32) -> f32 {
        min * (max / min).powf(self.0)
    }

    /// Create from a value in range [min, max].
    #[inline]
    pub fn from_range(value: f32, min: f32, max: f32) -> Self {
        if max > min {
            Self::new((value - min) / (max - min))
        } else {
            Self::CENTER
        }
    }

    /// Convert to bipolar range [-1, 1].
    #[inline]
    pub fn to_bipolar(self) -> BipolarValue {
        BipolarValue::new(self.0 * 2.0 - 1.0)
    }

    /// Invert (1 - value).
    #[inline]
    pub fn invert(self) -> Self {
        Self(1.0 - self.0)
    }

    /// Linear interpolation to another value.
    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(self.0 + (other.0 - self.0) * t)
    }

    /// Smooth step (S-curve).
    #[inline]
    pub fn smoothstep(self) -> Self {
        let t = self.0;
        Self(t * t * (3.0 - 2.0 * t))
    }

    /// Apply a curve (power function).
    #[inline]
    pub fn curve(self, exponent: f32) -> Self {
        Self::new(self.0.powf(exponent))
    }
}

impl Default for NormalizedValue {
    fn default() -> Self {
        Self::MIN
    }
}

impl Clampable for NormalizedValue {
    fn clamp(self) -> Self {
        Self(self.0.clamp(0.0, 1.0))
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && self.0 >= 0.0 && self.0 <= 1.0
    }
}

impl Mul for NormalizedValue {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

impl Mul<f32> for NormalizedValue {
    type Output = f32;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        self.0 * rhs
    }
}

impl From<f32> for NormalizedValue {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<NormalizedValue> for f32 {
    fn from(value: NormalizedValue) -> Self {
        value.0
    }
}

impl std::fmt::Display for NormalizedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.0}%", self.0 * 100.0)
    }
}

/// A value normalized to the range [-1.0, 1.0].
///
/// Used for:
/// - Audio samples
/// - LFO output
/// - Modulation amounts
/// - Pan position
/// - Any bipolar signal
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct BipolarValue(pub f32);

impl BipolarValue {
    /// Create a new bipolar value, clamping to [-1, 1].
    #[inline]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(-1.0, 1.0))
    }

    /// Create without clamping (for performance in hot paths).
    ///
    /// # Safety
    /// Caller must ensure value is in [-1, 1].
    #[inline]
    pub const fn new_unchecked(value: f32) -> Self {
        Self(value)
    }

    /// Minimum value (-1).
    pub const MIN: Self = Self(-1.0);

    /// Maximum value (1).
    pub const MAX: Self = Self(1.0);

    /// Center value (0).
    pub const CENTER: Self = Self(0.0);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Scale to a range centered on the center value.
    /// At -1: center - amount
    /// At  0: center
    /// At +1: center + amount
    #[inline]
    pub fn scale(self, center: f32, amount: f32) -> f32 {
        center + self.0 * amount
    }

    /// Convert to unipolar range [0, 1].
    #[inline]
    pub fn to_unipolar(self) -> NormalizedValue {
        NormalizedValue::new((self.0 + 1.0) * 0.5)
    }

    /// Get the absolute value.
    #[inline]
    pub fn abs(self) -> NormalizedValue {
        NormalizedValue::new(self.0.abs())
    }

    /// Invert (negate).
    #[inline]
    pub fn invert(self) -> Self {
        Self(-self.0)
    }

    /// Linear interpolation.
    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(self.0 + (other.0 - self.0) * t)
    }

    /// Rectify (full-wave: absolute value).
    #[inline]
    pub fn rectify_full(self) -> NormalizedValue {
        NormalizedValue::new(self.0.abs())
    }

    /// Rectify (half-wave: only positive).
    #[inline]
    pub fn rectify_half(self) -> NormalizedValue {
        NormalizedValue::new(self.0.max(0.0))
    }
}

impl Default for BipolarValue {
    fn default() -> Self {
        Self::CENTER
    }
}

impl Clampable for BipolarValue {
    fn clamp(self) -> Self {
        Self(self.0.clamp(-1.0, 1.0))
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && self.0 >= -1.0 && self.0 <= 1.0
    }
}

impl Neg for BipolarValue {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Add for BipolarValue {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.0 + rhs.0)
    }
}

impl Sub for BipolarValue {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.0 - rhs.0)
    }
}

impl Mul<f32> for BipolarValue {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self::new(self.0 * rhs)
    }
}

impl From<f32> for BipolarValue {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<BipolarValue> for f32 {
    fn from(value: BipolarValue) -> Self {
        value.0
    }
}

impl std::fmt::Display for BipolarValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:+.2}", self.0)
    }
}

/// Phase value in range [0.0, 1.0), wrapping.
///
/// Used for oscillator phase, LFO position, etc.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct Phase(pub f32);

impl Phase {
    /// Create a new phase value, wrapping to [0, 1).
    #[inline]
    pub fn new(value: f32) -> Self {
        Self(value.rem_euclid(1.0))
    }

    /// Create without wrapping (for performance).
    #[inline]
    pub const fn new_unchecked(value: f32) -> Self {
        Self(value)
    }

    /// Zero phase.
    pub const ZERO: Self = Self(0.0);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Advance phase by an increment, wrapping.
    #[inline]
    pub fn advance(self, increment: f32) -> Self {
        Self::new(self.0 + increment)
    }

    /// Get phase in radians [0, 2π).
    #[inline]
    pub fn as_radians(self) -> f32 {
        self.0 * std::f32::consts::TAU
    }

    /// Get sine value at this phase.
    #[inline]
    pub fn sin(self) -> f32 {
        self.as_radians().sin()
    }

    /// Get cosine value at this phase.
    #[inline]
    pub fn cos(self) -> f32 {
        self.as_radians().cos()
    }

    /// Reset to zero.
    #[inline]
    pub fn reset(&mut self) {
        self.0 = 0.0;
    }
}

impl Default for Phase {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Add<f32> for Phase {
    type Output = Self;

    #[inline]
    fn add(self, rhs: f32) -> Self::Output {
        Self::new(self.0 + rhs)
    }
}

impl From<f32> for Phase {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<Phase> for f32 {
    fn from(phase: Phase) -> Self {
        phase.0
    }
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.0}°", self.0 * 360.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_scale() {
        let v = NormalizedValue::new(0.5);
        assert!((v.scale(0.0, 100.0) - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_normalized_clamping() {
        let v = NormalizedValue::new(1.5);
        assert!((v.0 - 1.0).abs() < 0.001);

        let v = NormalizedValue::new(-0.5);
        assert!((v.0 - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_bipolar_to_unipolar() {
        let v = BipolarValue::new(0.0);
        assert!((v.to_unipolar().0 - 0.5).abs() < 0.001);

        let v = BipolarValue::MIN;
        assert!((v.to_unipolar().0 - 0.0).abs() < 0.001);

        let v = BipolarValue::MAX;
        assert!((v.to_unipolar().0 - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_phase_wrapping() {
        let p = Phase::new(1.5);
        assert!((p.0 - 0.5).abs() < 0.001);

        let p = Phase::new(-0.25);
        assert!((p.0 - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_phase_advance() {
        let p = Phase::new(0.9);
        let p = p.advance(0.2);
        assert!((p.0 - 0.1).abs() < 0.001);
    }
}
