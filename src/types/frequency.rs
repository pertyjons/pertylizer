//! Frequency types for type-safe audio processing.

use std::f32::consts::TAU;
use std::ops::{Add, Div, Mul, Sub};

use super::{Clampable, Interpolate, Seconds};

/// Frequency in Hertz.
///
/// Represents oscillation rate - the number of cycles per second.
///
/// # Valid Range
/// Typically 0.0 to Nyquist (sample_rate / 2), but audio frequencies
/// are usually 20 Hz to 20,000 Hz.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
#[repr(transparent)]
pub struct Hertz(pub f32);

impl Hertz {
    /// Create a new frequency value.
    #[inline]
    pub const fn new(hz: f32) -> Self {
        Self(hz)
    }

    /// Standard concert pitch A4.
    pub const A4: Self = Self(440.0);

    /// Middle C (C4).
    pub const C4: Self = Self(261.6256);

    /// Minimum audible frequency.
    pub const MIN_AUDIBLE: Self = Self(20.0);

    /// Maximum audible frequency.
    pub const MAX_AUDIBLE: Self = Self(20000.0);

    /// Get the raw value in Hz.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Get angular frequency (ω = 2πf).
    #[inline]
    pub fn angular(self) -> f32 {
        self.0 * TAU
    }

    /// Calculate phase increment per sample.
    #[inline]
    pub fn phase_increment(self, sample_rate: SampleRate) -> f32 {
        self.0 / sample_rate.0
    }

    /// Convert from MIDI note number.
    ///
    /// Uses standard 12-TET tuning with A4 = 440 Hz.
    #[inline]
    pub fn from_midi(note: u8) -> Self {
        Self(440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0))
    }

    /// Convert to approximate MIDI note number.
    #[inline]
    pub fn to_midi(self) -> f32 {
        69.0 + 12.0 * (self.0 / 440.0).log2()
    }

    /// Transpose by semitones.
    #[inline]
    pub fn transpose(self, semitones: f32) -> Self {
        Self(self.0 * (semitones / 12.0).exp2())
    }

    /// Transpose by octaves.
    #[inline]
    pub fn octave(self, octaves: i32) -> Self {
        Self(self.0 * 2.0f32.powi(octaves))
    }

    /// Clamp to audible range.
    #[inline]
    pub fn clamp_audible(self) -> Self {
        Self(self.0.clamp(20.0, 20000.0))
    }

    /// Clamp to a maximum (typically Nyquist).
    #[inline]
    pub fn clamp_max(self, max: Hertz) -> Self {
        Self(self.0.clamp(0.0, max.0))
    }
}

impl Clampable for Hertz {
    fn clamp(self) -> Self {
        Self(self.0.clamp(0.0, f32::MAX))
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && self.0 >= 0.0
    }
}

impl Hertz {
    /// Get the period (inverse of frequency).
    #[inline]
    pub fn period(&self) -> Seconds {
        if self.0 > 0.0 {
            Seconds::new(1.0 / self.0)
        } else {
            Seconds::new(f32::INFINITY)
        }
    }
}

impl Interpolate for Hertz {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self(self.0 + (other.0 - self.0) * t)
    }

    fn exp_lerp(self, other: Self, t: f32) -> Self {
        // Exponential interpolation is better for frequencies
        Self(self.0 * (other.0 / self.0).powf(t))
    }
}

// Arithmetic operations
impl Add for Hertz {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Hertz {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Mul<f32> for Hertz {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl Mul<Hertz> for f32 {
    type Output = Hertz;

    #[inline]
    fn mul(self, rhs: Hertz) -> Self::Output {
        Hertz(self * rhs.0)
    }
}

impl Div<f32> for Hertz {
    type Output = Self;

    #[inline]
    fn div(self, rhs: f32) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl Div<Hertz> for Hertz {
    type Output = f32;

    #[inline]
    fn div(self, rhs: Hertz) -> Self::Output {
        self.0 / rhs.0
    }
}

impl From<f32> for Hertz {
    fn from(hz: f32) -> Self {
        Self(hz)
    }
}

impl From<Hertz> for f32 {
    fn from(hz: Hertz) -> Self {
        hz.0
    }
}

impl std::fmt::Display for Hertz {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 >= 1000.0 {
            write!(f, "{:.2} kHz", self.0 / 1000.0)
        } else {
            write!(f, "{:.1} Hz", self.0)
        }
    }
}

/// Sample rate in samples per second.
///
/// This is essentially a frequency (Hz) but semantically different.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct SampleRate(pub f32);

impl SampleRate {
    /// Create a new sample rate.
    #[inline]
    pub const fn new(rate: f32) -> Self {
        Self(rate)
    }

    /// Common sample rates.
    pub const CD_QUALITY: Self = Self(44100.0);
    pub const DVD_QUALITY: Self = Self(48000.0);
    pub const HIGH_RES: Self = Self(96000.0);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Get the Nyquist frequency (half the sample rate).
    #[inline]
    pub fn nyquist(self) -> Hertz {
        Hertz::new(self.0 * 0.5)
    }

    /// Get the sample period in seconds.
    #[inline]
    pub fn period(self) -> Seconds {
        Seconds::new(1.0 / self.0)
    }

    /// Convert a time duration to sample count.
    #[inline]
    pub fn samples_for(self, duration: Seconds) -> usize {
        (duration.0 * self.0).round() as usize
    }
}

impl From<f32> for SampleRate {
    fn from(rate: f32) -> Self {
        Self(rate)
    }
}

impl From<SampleRate> for f32 {
    fn from(rate: SampleRate) -> Self {
        rate.0
    }
}

impl Default for SampleRate {
    fn default() -> Self {
        Self::DVD_QUALITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hertz_midi_conversion() {
        let a4 = Hertz::from_midi(69);
        assert!((a4.0 - 440.0).abs() < 0.001);

        let c4 = Hertz::from_midi(60);
        assert!((c4.0 - 261.63).abs() < 0.1);
    }

    #[test]
    fn test_hertz_transpose() {
        let a4 = Hertz::A4;
        let a5 = a4.octave(1);
        assert!((a5.0 - 880.0).abs() < 0.001);

        let e5 = a4.transpose(7.0); // Perfect fifth up
        assert!((e5.0 - 659.26).abs() < 0.1);
    }

    #[test]
    fn test_sample_rate_nyquist() {
        let sr = SampleRate::CD_QUALITY;
        let nyq = sr.nyquist();
        assert!((nyq.0 - 22050.0).abs() < 0.001);
    }

    #[test]
    fn test_hertz_period() {
        let freq = Hertz::new(100.0);
        let period = freq.period();
        assert!((period.0 - 0.01).abs() < 0.0001);
    }
}
