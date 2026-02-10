//! Envelope types for multi-point envelopes.
//!
//! These types provide type-safe wrappers for envelope frame positions,
//! values, point indices, and fadeout rates.

use serde::{Deserialize, Serialize};

use super::{Clampable, Seconds};

// ============================================================================
// Envelope Types
// ============================================================================

/// Envelope frame position (0-65535 ticks).
///
/// Multi-point envelopes use frame-based timing where frames advance
/// at a rate of `BPM * 2 / 5` Hz (50 Hz at 125 BPM).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct EnvelopeFrame(pub u16);

impl EnvelopeFrame {
    /// Minimum frame (0).
    pub const MIN: Self = Self(0);

    /// Maximum frame (65535).
    pub const MAX: Self = Self(65535);

    /// Create a new envelope frame.
    #[inline]
    pub const fn new(frame: u16) -> Self {
        Self(frame)
    }

    /// Get the raw frame value.
    #[inline]
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Convert to seconds given a tick rate.
    ///
    /// Tick rate is typically `BPM * 2 / 5` (50 Hz at 125 BPM).
    #[inline]
    #[must_use]
    pub fn to_seconds(self, tick_rate: f32) -> Seconds {
        Seconds::new(self.0 as f32 / tick_rate.max(1.0))
    }
}

impl From<u16> for EnvelopeFrame {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for EnvelopeFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}f", self.0)
    }
}

/// Envelope value (0.0-1.0).
///
/// XM uses 0-64 internally, IT uses 0-64 for volume and 0-32 for pan.
/// This type normalizes to 0.0-1.0 for consistent processing.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct EnvelopeValue(pub f32);

impl EnvelopeValue {
    /// Minimum value (0.0).
    pub const MIN: Self = Self(0.0);

    /// Maximum value (1.0).
    pub const MAX: Self = Self(1.0);

    /// Center value (0.5).
    pub const CENTER: Self = Self(0.5);

    /// Create a new envelope value, clamping to [0.0, 1.0].
    #[inline]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    /// Create without clamping (for performance in hot paths).
    #[inline]
    pub const fn new_unchecked(value: f32) -> Self {
        Self(value)
    }

    /// Get the raw value.
    #[inline]
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Linear interpolation to another value.
    #[inline]
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(self.0 + (other.0 - self.0) * t)
    }
}

impl Clampable for EnvelopeValue {
    fn clamp(self) -> Self {
        Self(self.0.clamp(0.0, 1.0))
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && (0.0..=1.0).contains(&self.0)
    }
}

impl From<f32> for EnvelopeValue {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for EnvelopeValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.0}%", self.0 * 100.0)
    }
}

/// Envelope point index (0-24).
///
/// IT format supports up to 25 envelope points, XM supports 12.
/// Using IT's limit as the maximum.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct EnvelopePointIndex(pub u8);

impl EnvelopePointIndex {
    /// Minimum index (0).
    pub const MIN: Self = Self(0);

    /// Maximum index (24, IT limit of 25 points).
    pub const MAX: Self = Self(24);

    /// Create a new point index, clamping to [0, 24].
    #[inline]
    pub const fn new(index: u8) -> Self {
        Self(if index > 24 { 24 } else { index })
    }

    /// Get the raw index value.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get the index as usize for array indexing.
    #[inline]
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u8> for EnvelopePointIndex {
    fn from(value: u8) -> Self {
        Self::new(value)
    }
}

impl From<usize> for EnvelopePointIndex {
    fn from(value: usize) -> Self {
        Self::new(value.min(24) as u8)
    }
}

impl std::fmt::Display for EnvelopePointIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pt{}", self.0)
    }
}

/// Fadeout rate (0-65535).
///
/// Controls how quickly volume fades after note-off.
/// Higher values = faster fade. 0 = no fadeout.
/// XM fadeout: `volume -= fadeout / 32768` per tick.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct FadeoutRate(pub u16);

impl FadeoutRate {
    /// No fadeout (infinite sustain after release).
    pub const NONE: Self = Self(0);

    /// Default fadeout rate.
    pub const DEFAULT: Self = Self(256);

    /// Fast fadeout.
    pub const FAST: Self = Self(4096);

    /// Instant fadeout.
    pub const INSTANT: Self = Self(65535);

    /// Create a new fadeout rate.
    #[inline]
    pub const fn new(rate: u16) -> Self {
        Self(rate)
    }

    /// Get the raw rate value.
    #[inline]
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Convert to per-tick linear fade amount.
    ///
    /// Returns the amount to subtract from fadeout level (0.0-1.0) each tick.
    /// FT2 formula: `fadeoutVol -= fadeoutSpeed` where `fadeoutVol` range is 0..32768.
    /// Normalized: `fade_per_tick = rate / 32768.0`.
    #[inline]
    #[must_use]
    pub fn to_linear_fade_per_tick(self) -> f32 {
        self.0 as f32 / 32768.0
    }

    /// Convert to per-tick decay multiplier (legacy exponential model).
    ///
    /// Returns a value slightly less than 1.0 to multiply the volume each tick.
    #[inline]
    #[must_use]
    pub fn to_decay_per_tick(self) -> f32 {
        if self.0 == 0 {
            1.0
        } else {
            1.0 - (self.0 as f32 / 65536.0)
        }
    }

    /// Estimate time to silence in seconds at given tick rate.
    ///
    /// Uses linear fadeout model: ticks to silence = 32768 / rate.
    #[inline]
    #[must_use]
    pub fn estimated_duration(self, tick_rate: f32) -> Seconds {
        if self.0 == 0 {
            Seconds::new(f32::INFINITY)
        } else {
            let fade_per_tick = self.to_linear_fade_per_tick();
            if fade_per_tick <= 0.0 {
                Seconds::new(f32::INFINITY)
            } else {
                let ticks = 1.0 / fade_per_tick; // ticks = 32768 / rate
                Seconds::new(ticks / tick_rate.max(1.0))
            }
        }
    }
}

impl From<u16> for FadeoutRate {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for FadeoutRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == 0 {
            write!(f, "none")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_frame_conversions() {
        let frame = EnvelopeFrame::new(100);
        assert_eq!(frame.as_u16(), 100);

        // At 50 Hz tick rate, 100 frames = 2 seconds
        let secs = frame.to_seconds(50.0);
        assert!((secs.as_f32() - 2.0).abs() < 0.001);
    }

    #[test]
    fn envelope_value_clamping() {
        assert_eq!(EnvelopeValue::new(1.5).as_f32(), 1.0);
        assert_eq!(EnvelopeValue::new(-0.5).as_f32(), 0.0);
        assert_eq!(EnvelopeValue::new(0.5).as_f32(), 0.5);
    }

    #[test]
    fn fadeout_rate_linear() {
        // No fadeout should return 0.0 (no subtraction)
        assert_eq!(FadeoutRate::NONE.to_linear_fade_per_tick(), 0.0);

        // FadeoutRate(4096) should fade in 8 ticks: 4096/32768 = 0.125 per tick
        let fade = FadeoutRate::FAST.to_linear_fade_per_tick();
        assert!((fade - 0.125).abs() < 0.0001);

        // Estimated duration at 50 Hz: 1/0.125 = 8 ticks, 8/50 = 0.16s
        let dur = FadeoutRate::FAST.estimated_duration(50.0);
        assert!((dur.as_f32() - 0.16).abs() < 0.01);
    }

    #[test]
    fn fadeout_rate_decay() {
        // No fadeout should return 1.0
        assert_eq!(FadeoutRate::NONE.to_decay_per_tick(), 1.0);

        // Non-zero should be less than 1.0
        let decay = FadeoutRate::DEFAULT.to_decay_per_tick();
        assert!(decay < 1.0);
        assert!(decay > 0.99);
    }
}
