//! Time duration types for type-safe audio processing.

use serde::{Deserialize, Serialize};

use super::{Clampable, Interpolate, SampleRate};

/// Duration in seconds.
///
/// Used for envelope times, delay times, etc.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
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
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
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
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
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

/// Beat division for tempo-synced timing.
///
/// Represents note values as fractions of a whole note:
/// - 4.0 = whole note
/// - 2.0 = half note
/// - 1.0 = quarter note
/// - 0.5 = eighth note
/// - 0.25 = sixteenth note
/// - 0.125 = thirty-second note
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct BeatDivision(pub f32);

impl BeatDivision {
    /// Create a new beat division.
    #[inline]
    pub const fn new(beats: f32) -> Self {
        Self(beats)
    }

    /// Thirty-second note (1/32).
    pub const THIRTY_SECOND: Self = Self(0.125);

    /// Sixteenth note (1/16).
    pub const SIXTEENTH: Self = Self(0.25);

    /// Eighth note (1/8).
    pub const EIGHTH: Self = Self(0.5);

    /// Quarter note (1/4).
    pub const QUARTER: Self = Self(1.0);

    /// Half note (1/2).
    pub const HALF: Self = Self(2.0);

    /// Whole note.
    pub const WHOLE: Self = Self(4.0);

    /// Dotted quarter (1.5 beats).
    pub const DOTTED_QUARTER: Self = Self(1.5);

    /// Dotted eighth (0.75 beats).
    pub const DOTTED_EIGHTH: Self = Self(0.75);

    /// Triplet quarter (2/3 beat).
    pub const TRIPLET_QUARTER: Self = Self(2.0 / 3.0);

    /// Triplet eighth (1/3 beat).
    pub const TRIPLET_EIGHTH: Self = Self(1.0 / 3.0);

    /// Get the raw value in beats.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to duration at a given tempo.
    #[inline]
    pub fn to_duration(self, tempo: Bpm) -> Seconds {
        let beat_duration = tempo.beat_duration();
        Seconds::new(beat_duration.as_f32() * self.0)
    }

    /// Convert to sample count at a given tempo and sample rate.
    #[inline]
    pub fn to_samples(self, tempo: Bpm, sample_rate: SampleRate) -> usize {
        self.to_duration(tempo).to_samples(sample_rate)
    }

    /// Get dotted version (1.5x duration).
    #[inline]
    pub fn dotted(self) -> Self {
        Self(self.0 * 1.5)
    }

    /// Get triplet version (2/3 duration).
    #[inline]
    pub fn triplet(self) -> Self {
        Self(self.0 * (2.0 / 3.0))
    }

    /// Clamp to valid note division range (1/32 to 4 whole notes).
    #[inline]
    pub fn clamp_typical(self) -> Self {
        Self(self.0.clamp(0.125, 16.0))
    }

    /// Get the note name for display.
    pub fn name(&self) -> &'static str {
        match self.0 {
            x if (x - 0.125).abs() < 0.01 => "1/32",
            x if (x - 0.25).abs() < 0.01 => "1/16",
            x if (x - 0.5).abs() < 0.01 => "1/8",
            x if (x - 0.75).abs() < 0.01 => "1/8.",
            x if (x - 1.0).abs() < 0.01 => "1/4",
            x if (x - 1.5).abs() < 0.01 => "1/4.",
            x if (x - 2.0).abs() < 0.01 => "1/2",
            x if (x - 3.0).abs() < 0.01 => "1/2.",
            x if (x - 4.0).abs() < 0.01 => "1/1",
            _ => "custom",
        }
    }

    /// All common note divisions for UI dropdowns.
    pub const ALL_COMMON: [Self; 9] = [
        Self::THIRTY_SECOND,
        Self::SIXTEENTH,
        Self::EIGHTH,
        Self::DOTTED_EIGHTH,
        Self::QUARTER,
        Self::DOTTED_QUARTER,
        Self::HALF,
        Self::WHOLE,
        Self(8.0), // 2 bars
    ];
}

impl Default for BeatDivision {
    fn default() -> Self {
        Self::QUARTER
    }
}

impl From<f32> for BeatDivision {
    fn from(beats: f32) -> Self {
        Self(beats.max(0.0))
    }
}

impl From<BeatDivision> for f32 {
    fn from(div: BeatDivision) -> Self {
        div.0
    }
}

impl std::fmt::Display for BeatDivision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beat_division_to_duration() {
        let quarter = BeatDivision::QUARTER;
        let tempo = Bpm::new(120.0);
        let duration = quarter.to_duration(tempo);
        assert!((duration.as_f32() - 0.5).abs() < 0.001); // At 120 BPM, quarter = 0.5s
    }

    #[test]
    fn test_beat_division_dotted() {
        let eighth = BeatDivision::EIGHTH;
        let dotted = eighth.dotted();
        assert!((dotted.0 - 0.75).abs() < 0.001);
    }

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
