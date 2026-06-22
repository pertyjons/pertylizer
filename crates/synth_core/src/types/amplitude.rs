//! Amplitude and gain types for type-safe audio processing.

use std::f32::consts::FRAC_PI_4;
use std::ops::{Add, Mul, Sub};

use serde::{Deserialize, Serialize};

use super::{BipolarValue, Clampable, ValueRange};

/// Linear gain factor (amplitude multiplier).
///
/// A value of 1.0 means unity gain (no change).
/// Values > 1.0 amplify, values < 1.0 attenuate.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Gain(pub f32);

impl Gain {
    /// Create a new gain value.
    #[inline]
    pub const fn new(gain: f32) -> Self {
        Self(gain)
    }

    /// Unity gain (no change).
    pub const UNITY: Self = Self(1.0);

    /// Silence (mute).
    pub const MUTE: Self = Self(0.0);

    /// Bounds for an amplifier / mixer **level** control (0–2, default unity).
    ///
    /// `0` = silent, `1` = unity, `2` = +6 dB. Single source of truth shared by
    /// the amplifier/mixer descriptors `.value_range(...)` and their level
    /// apply-clamps — so they can't drift.
    pub const MIXER_RANGE: ValueRange = ValueRange::new(0.0, 2.0, 1.0);

    /// Get the raw gain value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to decibels.
    #[inline]
    pub fn to_db(self) -> Decibels {
        if self.0 > 0.0 {
            Decibels::new(20.0 * self.0.log10())
        } else {
            Decibels::NEG_INF
        }
    }

    /// Apply gain to a sample.
    #[inline]
    pub fn apply(self, sample: f32) -> f32 {
        sample * self.0
    }

    /// Combine two gains (multiply).
    #[inline]
    pub fn combine(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }

    /// Clamp to a reasonable range.
    #[inline]
    pub fn clamp_reasonable(self) -> Self {
        Self(self.0.clamp(0.0, 10.0))
    }

    /// Calculate left and right gains from a pan position using constant power panning.
    ///
    /// Pan: -1.0 = full left, 0.0 = center, 1.0 = full right.
    /// Uses sine/cosine curves to maintain constant perceived loudness.
    #[inline]
    pub fn from_pan(pan: BipolarValue) -> (Self, Self) {
        // Convert pan [-1, 1] to angle [0, π/2]
        let angle = (pan.as_f32() + 1.0) * FRAC_PI_4;
        let left = Self::new(angle.cos());
        let right = Self::new(angle.sin());
        (left, right)
    }
}

impl Default for Gain {
    fn default() -> Self {
        Self::UNITY
    }
}

impl Clampable for Gain {
    fn clamp(self) -> Self {
        Self(self.0.max(0.0))
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && self.0 >= 0.0
    }
}

impl Mul for Gain {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

impl Mul<f32> for Gain {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl From<f32> for Gain {
    fn from(gain: f32) -> Self {
        Self(gain)
    }
}

impl From<Gain> for f32 {
    fn from(gain: Gain) -> Self {
        gain.0
    }
}

impl std::fmt::Display for Gain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.2}x", self.0)
    }
}

/// Amplitude in decibels (logarithmic scale).
///
/// 0 dB = unity gain
/// -6 dB ≈ half amplitude
/// -20 dB = 0.1 amplitude
/// -∞ dB = silence
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Decibels(pub f32);

impl Decibels {
    /// Create a new dB value.
    #[inline]
    pub const fn new(db: f32) -> Self {
        Self(db)
    }

    /// Unity (0 dB).
    pub const UNITY: Self = Self(0.0);

    /// Negative infinity (silence).
    pub const NEG_INF: Self = Self(f32::NEG_INFINITY);

    /// Minimum displayable value.
    pub const MIN_DISPLAY: Self = Self(-60.0);

    /// Get the raw value in dB.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to linear gain.
    #[inline]
    pub fn to_gain(self) -> Gain {
        if self.0.is_finite() {
            Gain::new(10.0f32.powf(self.0 / 20.0))
        } else {
            Gain::MUTE
        }
    }

    /// Convert to linear amplitude value.
    #[inline]
    pub fn to_linear(self) -> f32 {
        self.to_gain().as_f32()
    }

    /// Create from linear amplitude.
    #[inline]
    pub fn from_linear(linear: f32) -> Self {
        if linear <= 0.0 {
            Self::NEG_INF
        } else {
            Self(20.0 * linear.log10())
        }
    }

    /// Clamp to a display range (-60 to +20 dB).
    #[inline]
    pub fn clamp_display(self) -> Self {
        Self(self.0.clamp(-60.0, 20.0))
    }

    /// Clamp to a typical mixer range (-∞ to +12 dB).
    #[inline]
    pub fn clamp_mixer(self) -> Self {
        if self.0 < -60.0 {
            Self::NEG_INF
        } else {
            Self(self.0.min(12.0))
        }
    }
}

/// Convert a linear magnitude to a normalized `0.0..=1.0` meter value.
///
/// The magnitude is converted to dBFS (`20·log10`) and then mapped linearly
/// over the dB range `[floor_db, 0]` onto `[0.0, 1.0]`, clamped at both ends.
/// `floor_db` is the dB value that maps to `0.0` and must be negative (e.g.
/// `-60.0` for a level meter, `-100.0` for a spectrum band). A magnitude of
/// `0` (or below) yields `0.0`.
#[inline]
#[must_use]
pub fn magnitude_to_normalized_db(magnitude: f32, floor_db: f32) -> f32 {
    // `max(0.0)` floors at silence and also collapses NaN to 0.0 (NaN guard:
    // `f32::max` returns the non-NaN operand), so a bad sample maps to 0.0
    // rather than propagating NaN through the clamp into render geometry.
    let db = Decibels::from_linear(magnitude.max(0.0)).as_f32();
    ((db - floor_db) / -floor_db).clamp(0.0, 1.0)
}

impl Default for Decibels {
    fn default() -> Self {
        Self::UNITY
    }
}

impl Clampable for Decibels {
    fn clamp(self) -> Self {
        self.clamp_display()
    }

    fn is_valid(&self) -> bool {
        // Negative infinity is valid (represents silence)
        !self.0.is_nan() && self.0 <= 100.0
    }
}

impl Add for Decibels {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Decibels {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl From<f32> for Decibels {
    fn from(db: f32) -> Self {
        Self(db)
    }
}

impl From<Decibels> for f32 {
    fn from(db: Decibels) -> Self {
        db.0
    }
}

impl From<Gain> for Decibels {
    fn from(gain: Gain) -> Self {
        gain.to_db()
    }
}

impl From<Decibels> for Gain {
    fn from(db: Decibels) -> Self {
        db.to_gain()
    }
}

impl std::fmt::Display for Decibels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == f32::NEG_INFINITY {
            write!(f, "-∞ dB")
        } else {
            write!(f, "{:+.1} dB", self.0)
        }
    }
}

/// Compression ratio (e.g., 4:1, 10:1).
///
/// A ratio of 1.0 means no compression, infinity would be limiting.
/// Typical values range from 1.0 (no compression) to 20.0 (heavy limiting).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Ratio(pub f32);

impl Ratio {
    /// Create a new compression ratio.
    #[inline]
    pub const fn new(ratio: f32) -> Self {
        Self(ratio)
    }

    /// No compression (1:1).
    pub const UNITY: Self = Self(1.0);

    /// Light compression (2:1).
    pub const LIGHT: Self = Self(2.0);

    /// Medium compression (4:1).
    pub const MEDIUM: Self = Self(4.0);

    /// Heavy compression (10:1).
    pub const HEAVY: Self = Self(10.0);

    /// Limiting (20:1).
    pub const LIMITING: Self = Self(20.0);

    /// Get the raw ratio value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Calculate the compressed output level.
    ///
    /// Given an input level in dB above threshold, returns the output level in dB.
    /// Formula: output = input / ratio
    #[inline]
    pub fn compress(&self, overshoot_db: f32) -> f32 {
        if self.0 > 0.0 {
            overshoot_db / self.0
        } else {
            overshoot_db
        }
    }

    /// Clamp to a typical compression range (1:1 to 20:1).
    #[inline]
    pub fn clamp_typical(self) -> Self {
        Self(self.0.clamp(1.0, 20.0))
    }

    /// Format as ratio string (e.g., "4:1").
    pub fn to_ratio_string(&self) -> String {
        if self.0 >= 20.0 {
            "∞:1".to_string()
        } else {
            format!("{:.1}:1", self.0)
        }
    }
}

impl Default for Ratio {
    fn default() -> Self {
        Self::MEDIUM
    }
}

impl Clampable for Ratio {
    fn clamp(self) -> Self {
        self.clamp_typical()
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && self.0 >= 1.0
    }
}

impl From<f32> for Ratio {
    fn from(ratio: f32) -> Self {
        Self(ratio.max(1.0))
    }
}

impl From<Ratio> for f32 {
    fn from(ratio: Ratio) -> Self {
        ratio.0
    }
}

impl std::fmt::Display for Ratio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_ratio_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magnitude_to_normalized_db() {
        // 0 dB (unity magnitude) maps to the top of the range.
        assert!((magnitude_to_normalized_db(1.0, -60.0) - 1.0).abs() < 1e-4);
        // The floor dB maps to 0.0 (-60 dB ≈ magnitude 0.001).
        assert!(magnitude_to_normalized_db(0.001, -60.0).abs() < 1e-3);
        // Midpoint: -30 dB over a -60 dB floor → 0.5.
        let half = magnitude_to_normalized_db(10f32.powf(-30.0 / 20.0), -60.0);
        assert!((half - 0.5).abs() < 1e-3);
        // Below the floor and silence both clamp to 0.0.
        assert_eq!(magnitude_to_normalized_db(0.0, -60.0), 0.0);
        assert_eq!(magnitude_to_normalized_db(-1.0, -60.0), 0.0);
        // Above unity clamps to 1.0.
        assert_eq!(magnitude_to_normalized_db(4.0, -60.0), 1.0);
        // NaN is sanitized to 0.0 rather than propagating.
        assert_eq!(magnitude_to_normalized_db(f32::NAN, -60.0), 0.0);
    }

    #[test]
    fn test_ratio_compression() {
        let ratio = Ratio::MEDIUM; // 4:1
        let compressed = ratio.compress(12.0); // 12dB overshoot
        assert!((compressed - 3.0).abs() < 0.001); // Should be 3dB
    }

    #[test]
    fn test_ratio_display() {
        assert_eq!(Ratio::MEDIUM.to_string(), "4.0:1");
        assert_eq!(Ratio::LIMITING.to_string(), "∞:1");
    }

    #[test]
    fn test_gain_to_db() {
        let unity = Gain::UNITY;
        let db = unity.to_db();
        assert!((db.0 - 0.0).abs() < 0.001);

        let half = Gain::new(0.5);
        let db = half.to_db();
        assert!((db.0 - (-6.02)).abs() < 0.1);
    }

    #[test]
    fn test_db_to_gain() {
        let unity = Decibels::UNITY;
        let gain = unity.to_gain();
        assert!((gain.0 - 1.0).abs() < 0.001);

        let half_amp = Decibels::new(-6.02);
        let gain = half_amp.to_gain();
        assert!((gain.0 - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_gain_apply() {
        let gain = Gain::new(0.5);
        let sample = 0.8;
        assert!((gain.apply(sample) - 0.4).abs() < 0.001);
    }

    #[test]
    fn test_db_display() {
        assert!(Decibels::NEG_INF.to_string().contains("-∞"));
        assert!(Decibels::new(3.0).to_string().contains("+3"));
    }
}
