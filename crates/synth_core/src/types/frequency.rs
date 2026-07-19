//! Frequency types for type-safe audio processing.

use std::f32::consts::TAU;

use serde::{Deserialize, Serialize};

use super::{BipolarValue, Cents, Clampable, Interpolate, Seconds, Semitones, ValueRange};

/// Frequency in Hertz.
///
/// Represents oscillation rate - the number of cycles per second.
///
/// # Valid Range
/// Typically 0.0 to Nyquist (sample_rate / 2), but audio frequencies
/// are usually 20 Hz to 20,000 Hz.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Hertz(f32);

impl Hertz {
    /// Create a new frequency value.
    #[inline]
    pub const fn new(hz: f32) -> Self {
        Self(hz)
    }

    /// Zero frequency.
    pub const ZERO: Self = Self(0.0);

    /// Standard concert pitch A4.
    pub const A4: Self = Self(440.0);

    /// Middle C (C4).
    pub const C4: Self = Self(261.6256);

    /// Minimum audible frequency.
    pub const MIN_AUDIBLE: Self = Self(20.0);

    /// Maximum audible frequency.
    pub const MAX_AUDIBLE: Self = Self(20000.0);

    // === Musical note frequencies ===

    /// C0 (lowest piano note).
    pub const C0: Self = Self(16.352);
    /// C1.
    pub const C1: Self = Self(32.703);
    /// C2.
    pub const C2: Self = Self(65.406);
    /// C3.
    pub const C3: Self = Self(130.813);
    /// C5.
    pub const C5: Self = Self(523.251);
    /// C6.
    pub const C6: Self = Self(1046.502);
    /// C7.
    pub const C7: Self = Self(2093.005);
    /// C8 (highest piano note).
    pub const C8: Self = Self(4186.009);

    // === Range constants ===

    /// Bounds for an **oscillator base-frequency** control (1–20 000 Hz, A4).
    ///
    /// Single source of truth for the oscillator frequency descriptor
    /// `.value_range(...)` and its `set_param` apply-clamp.
    pub const OSC_RANGE: ValueRange = ValueRange::new(1.0, 20_000.0, 440.0);

    /// Bounds for a **crossover split-frequency** control (20–20 000 Hz, 1 kHz).
    ///
    /// Numerically equal to [`Self::FILTER_RANGE`] today but kept distinct: a
    /// crossover split is a separate control from a filter cutoff and must be
    /// free to diverge.
    pub const CROSSOVER_RANGE: ValueRange = ValueRange::new(20.0, 20_000.0, 1_000.0);

    /// Bounds for an LFO **rate** control (0.01–50 Hz, default 1 Hz).
    ///
    /// Single source of truth shared by the LFO descriptor `.value_range(...)`,
    /// the rate apply-clamps, and [`Self::clamp_lfo`] — so they can't drift.
    pub const LFO_RANGE: ValueRange = ValueRange::new(0.01, 50.0, 1.0);

    /// Bounds for a **filter cutoff** control (20–20 000 Hz, default 1 kHz).
    ///
    /// Single source of truth shared by the filter descriptors
    /// `.value_range(...)` and [`Self::clamp_filter`] — so they can't drift.
    pub const FILTER_RANGE: ValueRange = ValueRange::new(20.0, 20_000.0, 1_000.0);

    /// Sub-bass upper limit.
    pub const SUB_BASS: Self = Self(60.0);
    /// Bass upper limit.
    pub const BASS: Self = Self(250.0);
    /// Midrange upper limit.
    pub const MID: Self = Self(4000.0);
    /// Presence upper limit.
    pub const PRESENCE: Self = Self(6000.0);

    /// Get the raw value in Hz.
    #[inline]
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Get angular frequency (ω = 2πf).
    #[inline]
    #[must_use]
    pub fn angular(self) -> f32 {
        self.0 * TAU
    }

    /// Calculate phase increment per sample.
    #[inline]
    #[must_use]
    pub fn phase_increment(self, sample_rate: SampleRate) -> f32 {
        self.0 / sample_rate.0
    }

    /// Convert from MIDI note number.
    ///
    /// Uses standard 12-TET tuning with A4 = 440 Hz.
    #[inline]
    #[must_use]
    pub fn from_midi(note: u8) -> Self {
        Self(440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0))
    }

    /// Convert to approximate MIDI note number.
    #[inline]
    #[must_use]
    pub fn to_midi(self) -> f32 {
        if self.0 <= 0.0 {
            return 0.0;
        }
        69.0 + 12.0 * (self.0 / 440.0).log2()
    }

    /// Transpose by semitones.
    #[inline]
    #[must_use]
    pub fn transpose(self, semitones: Semitones) -> Self {
        Self(self.0 * (semitones.as_f32() / 12.0).exp2())
    }

    /// Transpose by octaves.
    #[inline]
    #[must_use]
    pub fn octave(self, octaves: i32) -> Self {
        Self(self.0 * 2.0f32.powi(octaves))
    }

    /// Apply exponential FM modulation (±2 octave range per unit CV).
    ///
    /// Computes `self * 2^(cv * 2)` — standard exponential FM scaling
    /// where `cv = 1.0` produces a 4x frequency multiplier (2 octaves up).
    #[inline]
    #[must_use]
    pub fn apply_fm(self, cv: BipolarValue) -> Self {
        Self((self.0 * (cv.as_f32() * 2.0).exp2()).clamp(0.01, 100_000.0))
    }

    /// Apply 1V/octave CV pitch tracking.
    ///
    /// Computes `self * 2^cv` — the Eurorack standard where
    /// each unit of CV shifts the frequency by one octave.
    #[inline]
    #[must_use]
    pub fn apply_cv(self, cv: BipolarValue) -> Self {
        Self(self.0 * cv.as_f32().exp2())
    }

    /// Clamp to audible range.
    #[inline]
    #[must_use]
    pub fn clamp_audible(self) -> Self {
        Self(self.0.clamp(20.0, 20000.0))
    }

    /// Clamp to a maximum (typically Nyquist).
    #[inline]
    #[must_use]
    pub fn clamp_max(self, max: Self) -> Self {
        Self(self.0.clamp(0.0, max.0))
    }

    /// Get the period in samples (sample_rate / freq).
    ///
    /// Useful for delay lines and oscillator period calculations.
    #[inline]
    #[must_use]
    pub fn period_samples(self, sample_rate: SampleRate) -> f32 {
        if self.0 > 0.0 {
            sample_rate.0 / self.0
        } else {
            f32::INFINITY
        }
    }

    /// Calculate tangent coefficient for filter design.
    ///
    /// Returns `tan(π * freq / sample_rate)`, used in bilinear transform
    /// for converting analog filter designs to digital.
    #[inline]
    #[must_use]
    pub fn to_tan_coeff(self, sample_rate: SampleRate) -> f32 {
        let clamped = self.0.clamp(0.0, sample_rate.as_f32() * 0.499);
        (std::f32::consts::PI * clamped / sample_rate.0).tan()
    }

    /// Calculate exponential decay coefficient.
    ///
    /// Returns `exp(-2π * freq / sample_rate)`, used for one-pole
    /// lowpass filters where the frequency represents the cutoff.
    #[inline]
    #[must_use]
    pub fn to_exp_coeff(self, sample_rate: SampleRate) -> f32 {
        (-TAU * self.0 / sample_rate.0).exp()
    }

    /// Clamp to LFO range.
    #[inline]
    #[must_use]
    pub fn clamp_lfo(self) -> Self {
        Self::new(Self::LFO_RANGE.clamp(self.0))
    }

    /// Clamp to filter range.
    #[inline]
    #[must_use]
    pub fn clamp_filter(self) -> Self {
        Self::new(Self::FILTER_RANGE.clamp(self.0))
    }

    /// Check if frequency is in audible range.
    #[inline]
    #[must_use]
    pub fn is_audible(self) -> bool {
        self.0 >= 20.0 && self.0 <= 20000.0
    }

    /// Get the frequency band this falls into.
    #[must_use]
    pub fn band(&self) -> FrequencyBand {
        if self.0 < 60.0 {
            FrequencyBand::SubBass
        } else if self.0 < 250.0 {
            FrequencyBand::Bass
        } else if self.0 < 4000.0 {
            FrequencyBand::Mid
        } else if self.0 < 6000.0 {
            FrequencyBand::Presence
        } else {
            FrequencyBand::Brilliance
        }
    }

    /// Calculate detune in cents between two frequencies.
    #[inline]
    #[must_use]
    pub fn cents_between(self, other: Self) -> Cents {
        if self.0 <= 0.0 {
            return Cents::ZERO;
        }
        Cents::new(1200.0 * (other.0 / self.0).log2())
    }
}

/// Frequency bands for EQ and analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyBand {
    /// Sub-bass (< 60 Hz).
    SubBass,
    /// Bass (60-250 Hz).
    Bass,
    /// Midrange (250-4000 Hz).
    Mid,
    /// Presence (4000-6000 Hz).
    Presence,
    /// Brilliance (> 6000 Hz).
    Brilliance,
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
        if self.0 > 0.0 && other.0 > 0.0 {
            Self(self.0 * (other.0 / self.0).powf(t))
        } else {
            self.lerp(other, t)
        }
    }
}

// Arithmetic operations via macros
crate::impl_additive!(Hertz);
crate::impl_scaling!(Hertz);
crate::impl_ratio!(Hertz);
crate::impl_float_conversions!(Hertz);

// Cross-type operators for ergonomic DSP code

/// Hertz / SampleRate = phase increment (f32).
///
/// This is the fundamental operation for oscillators:
/// ```ignore
/// let phase_inc = self.frequency / context.sample_rate;
/// ```
impl std::ops::Div<SampleRate> for Hertz {
    type Output = f32;

    #[inline]
    fn div(self, rhs: SampleRate) -> Self::Output {
        self.0 / rhs.0
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

/// Sample rate in samples per second for DSP calculations.
///
/// This type uses `f32` for efficient math operations in audio processing.
/// For audio backend configuration, use `crate::audio::types::SampleRate` (u32)
/// which matches hardware API conventions.
///
/// Conversions between the two types are provided via `From` impls in `audio::types`.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(from = "f32", into = "f32")]
#[repr(transparent)]
pub struct SampleRate(f32);

impl SampleRate {
    /// Create a new sample rate. Ensures the value is positive.
    #[inline]
    pub fn new(rate: f32) -> Self {
        Self(if rate > 0.0 { rate } else { 1.0 })
    }

    /// Create a sample rate without clamping.
    ///
    /// The caller must guarantee that `rate` is finite and greater than zero.
    #[must_use]
    pub const fn new_unchecked(rate: f32) -> Self {
        debug_assert!(
            rate.is_finite() && rate > 0.0,
            "sample rate must be positive"
        );
        Self(rate)
    }

    /// Common sample rates.
    pub const CD_QUALITY: Self = Self(44100.0);
    pub const DVD_QUALITY: Self = Self(48000.0);
    pub const HIGH_RES: Self = Self(96000.0);

    /// The highest sample rate the engine supports — the engine-wide ceiling.
    ///
    /// Real-time look-ahead / scratch buffers that must hold a worst-case number
    /// of samples (e.g. the limiter's look-ahead ring) size themselves from this
    /// rather than a hand-copied literal, so they can't desync if the ceiling moves.
    pub const MAX_SUPPORTED: Self = Self(192_000.0);

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
        (duration.as_f32() * self.0).round() as usize
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

impl std::fmt::Display for SampleRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 >= 1000.0 {
            write!(f, "{:.1} kHz", self.0 / 1000.0)
        } else {
            write!(f, "{:.0} Hz", self.0)
        }
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

        let e5 = a4.transpose(Semitones::new(7.0)); // Perfect fifth up
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
        assert!((period.as_f32() - 0.01).abs() < 0.0001);
    }
}
