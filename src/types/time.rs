//! Time duration types for type-safe audio processing.

use std::ops::{Add, Div, Mul, Sub};

use super::{Clampable, Interpolate};

/// Duration in seconds.
///
/// Used for envelope times, delay times, etc.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
#[repr(transparent)]
pub struct Seconds(pub f32);

impl Seconds {
    /// Create a new time duration.
    #[inline]
    pub const fn new(secs: f32) -> Self {
        Self(secs)
    }

    /// Zero duration.
    pub const ZERO: Self = Self(0.0);

    /// One millisecond.
    pub const MS: Self = Self(0.001);

    /// Get the raw value in seconds.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to milliseconds.
    #[inline]
    pub fn as_millis(self) -> f32 {
        self.0 * 1000.0
    }

    /// Convert to frequency (cycles per second).
    #[inline]
    pub fn to_frequency(self) -> super::Hertz {
        if self.0 > 0.0 {
            super::Hertz::new(1.0 / self.0)
        } else {
            super::Hertz::new(f32::INFINITY)
        }
    }

    /// Create from milliseconds.
    #[inline]
    pub const fn from_millis(ms: f32) -> Self {
        Self(ms / 1000.0)
    }

    /// Clamp to a typical envelope range.
    #[inline]
    pub fn clamp_envelope(self) -> Self {
        Self(self.0.clamp(0.001, 30.0))
    }

    /// Clamp to a positive value.
    #[inline]
    pub fn clamp_positive(self) -> Self {
        Self(self.0.max(0.0))
    }
}

impl Clampable for Seconds {
    fn clamp(self) -> Self {
        Self(self.0.max(0.0))
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && self.0 >= 0.0
    }
}

impl Interpolate for Seconds {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self(self.0 + (other.0 - self.0) * t)
    }

    fn exp_lerp(self, other: Self, t: f32) -> Self {
        // Exponential interpolation for time
        if self.0 > 0.0 && other.0 > 0.0 {
            Self(self.0 * (other.0 / self.0).powf(t))
        } else {
            self.lerp(other, t)
        }
    }
}

// Arithmetic operations
impl Add for Seconds {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Seconds {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Mul<f32> for Seconds {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl Mul<Seconds> for f32 {
    type Output = Seconds;

    #[inline]
    fn mul(self, rhs: Seconds) -> Self::Output {
        Seconds(self * rhs.0)
    }
}

impl Div<f32> for Seconds {
    type Output = Self;

    #[inline]
    fn div(self, rhs: f32) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl Div<Seconds> for Seconds {
    type Output = f32;

    #[inline]
    fn div(self, rhs: Seconds) -> Self::Output {
        self.0 / rhs.0
    }
}

impl From<f32> for Seconds {
    fn from(secs: f32) -> Self {
        Self(secs)
    }
}

impl From<Seconds> for f32 {
    fn from(secs: Seconds) -> Self {
        secs.0
    }
}

impl std::fmt::Display for Seconds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 < 0.001 {
            write!(f, "{:.0} µs", self.0 * 1_000_000.0)
        } else if self.0 < 1.0 {
            write!(f, "{:.1} ms", self.0 * 1000.0)
        } else {
            write!(f, "{:.2} s", self.0)
        }
    }
}

/// Duration in milliseconds.
///
/// Convenience type for UI display and input.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
#[repr(transparent)]
pub struct Milliseconds(pub f32);

impl Milliseconds {
    /// Create a new duration in milliseconds.
    #[inline]
    pub const fn new(ms: f32) -> Self {
        Self(ms)
    }

    /// Get the raw value in milliseconds.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to Seconds.
    #[inline]
    pub fn to_seconds(self) -> Seconds {
        Seconds::new(self.0 / 1000.0)
    }
}

impl From<Seconds> for Milliseconds {
    fn from(secs: Seconds) -> Self {
        Self(secs.0 * 1000.0)
    }
}

impl From<Milliseconds> for Seconds {
    fn from(ms: Milliseconds) -> Self {
        Self(ms.0 / 1000.0)
    }
}

impl From<f32> for Milliseconds {
    fn from(ms: f32) -> Self {
        Self(ms)
    }
}

impl From<Milliseconds> for f32 {
    fn from(ms: Milliseconds) -> Self {
        ms.0
    }
}

impl std::fmt::Display for Milliseconds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1} ms", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seconds_to_frequency() {
        let period = Seconds::new(0.01); // 10ms
        let freq = period.to_frequency();
        assert!((freq.0 - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_milliseconds_conversion() {
        let ms = Milliseconds::new(500.0);
        let secs: Seconds = ms.into();
        assert!((secs.0 - 0.5).abs() < 0.0001);
    }

    #[test]
    fn test_seconds_display() {
        let s1 = Seconds::new(0.0001);
        assert!(s1.to_string().contains("µs"));

        let s2 = Seconds::new(0.5);
        assert!(s2.to_string().contains("ms"));

        let s3 = Seconds::new(2.5);
        assert!(s3.to_string().contains("s"));
    }
}
