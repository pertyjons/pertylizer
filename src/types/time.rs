//! Time duration types for type-safe audio processing.

use super::{Clampable, Interpolate, SampleRate};

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

    /// Calculate one-pole filter coefficient for envelope smoothing.
    ///
    /// Returns `exp(-1.0 / (time * sample_rate))` which can be used as:
    /// `y = y * coeff + target * (1 - coeff)`
    #[inline]
    pub fn to_exp_coeff(self, sample_rate: SampleRate) -> f32 {
        if self.0 > 0.0 {
            (-1.0 / (self.0 * sample_rate.as_f32())).exp()
        } else {
            0.0 // Instant (no smoothing)
        }
    }

    /// Convert to sample count.
    #[inline]
    pub fn to_samples(self, sample_rate: SampleRate) -> usize {
        (self.0 * sample_rate.as_f32()).round() as usize
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

// Arithmetic operations via macros
crate::impl_additive!(Seconds);
crate::impl_scaling!(Seconds);
crate::impl_ratio!(Seconds);
crate::impl_float_conversions!(Seconds);

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

/// Tempo in beats per minute (BPM).
///
/// Standard range is 20-300 BPM for most music.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
#[repr(transparent)]
pub struct Bpm(pub f32);

impl Bpm {
    /// Create a new tempo value.
    #[inline]
    pub const fn new(bpm: f32) -> Self {
        Self(bpm)
    }

    /// Default tempo (120 BPM).
    pub const DEFAULT: Self = Self(120.0);

    /// Minimum typical tempo.
    pub const MIN: Self = Self(20.0);

    /// Maximum typical tempo.
    pub const MAX: Self = Self(300.0);

    /// Get the raw BPM value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to beat duration in seconds.
    #[inline]
    pub fn beat_duration(self) -> Seconds {
        if self.0 > 0.0 {
            Seconds::new(60.0 / self.0)
        } else {
            Seconds::new(0.5) // Fallback to 120 BPM
        }
    }

    /// Calculate samples per beat at given sample rate.
    #[inline]
    pub fn samples_per_beat(self, sample_rate: SampleRate) -> f32 {
        self.beat_duration().as_f32() * sample_rate.as_f32()
    }

    /// Calculate beats per sample at given sample rate.
    #[inline]
    pub fn beats_per_sample(self, sample_rate: SampleRate) -> f32 {
        self.0 / 60.0 / sample_rate.as_f32()
    }

    /// Clamp to typical tempo range.
    #[inline]
    pub fn clamp_typical(self) -> Self {
        Self(self.0.clamp(Self::MIN.0, Self::MAX.0))
    }
}

impl Clampable for Bpm {
    fn clamp(self) -> Self {
        self.clamp_typical()
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && self.0 > 0.0
    }
}

impl From<f32> for Bpm {
    fn from(bpm: f32) -> Self {
        Self(bpm)
    }
}

impl From<Bpm> for f32 {
    fn from(bpm: Bpm) -> Self {
        bpm.0
    }
}

impl std::fmt::Display for Bpm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1} BPM", self.0)
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
