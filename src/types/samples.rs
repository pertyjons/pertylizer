//! Sample count types for type-safe audio processing.

use std::ops::{Add, Div, Mul, Sub};

use super::{SampleRate, Seconds};

/// A count of audio samples.
///
/// Used for buffer sizes, delay lengths, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct SampleCount(pub usize);

impl SampleCount {
    /// Create a new sample count.
    #[inline]
    pub const fn new(count: usize) -> Self {
        Self(count)
    }

    /// Zero samples.
    pub const ZERO: Self = Self(0);

    /// Get the raw value.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// Convert to duration at a given sample rate.
    #[inline]
    pub fn to_seconds(self, sample_rate: SampleRate) -> Seconds {
        Seconds::new(self.0 as f32 / sample_rate.0)
    }

    /// Create from a duration at a given sample rate.
    #[inline]
    pub fn from_seconds(duration: Seconds, sample_rate: SampleRate) -> Self {
        Self((duration.0 * sample_rate.0).round() as usize)
    }

    /// Check if zero.
    #[inline]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Saturating subtraction.
    #[inline]
    pub const fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    /// Saturating addition.
    #[inline]
    pub const fn saturating_add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }
}

impl Add for SampleCount {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for SampleCount {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Mul<usize> for SampleCount {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: usize) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl Div<usize> for SampleCount {
    type Output = Self;

    #[inline]
    fn div(self, rhs: usize) -> Self::Output {
        Self(self.0 / rhs)
    }
}

/// SampleCount / SampleRate = Seconds.
///
/// Convert sample count to duration:
/// ```ignore
/// let duration = buffer_size / sample_rate;
/// ```
impl Div<SampleRate> for SampleCount {
    type Output = Seconds;

    #[inline]
    fn div(self, rhs: SampleRate) -> Self::Output {
        Seconds::new(self.0 as f32 / rhs.0)
    }
}

impl From<usize> for SampleCount {
    fn from(count: usize) -> Self {
        Self(count)
    }
}

impl From<SampleCount> for usize {
    fn from(count: SampleCount) -> Self {
        count.0
    }
}

impl std::fmt::Display for SampleCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} samples", self.0)
    }
}

/// Sample position in a buffer or stream.
///
/// Can be used for playback position, write position, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct SamplePosition(pub u64);

impl SamplePosition {
    /// Create a new sample position.
    #[inline]
    pub const fn new(pos: u64) -> Self {
        Self(pos)
    }

    /// Zero position (start).
    pub const ZERO: Self = Self(0);

    /// Get the raw value.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Advance by a count.
    #[inline]
    pub fn advance(self, count: SampleCount) -> Self {
        Self(self.0 + count.0 as u64)
    }

    /// Convert to time at a given sample rate.
    #[inline]
    pub fn to_seconds(self, sample_rate: SampleRate) -> Seconds {
        Seconds::new(self.0 as f32 / sample_rate.0)
    }

    /// Get position within a buffer (wrapping).
    #[inline]
    pub fn wrap(self, buffer_size: SampleCount) -> usize {
        (self.0 as usize) % buffer_size.0
    }
}

impl Add<SampleCount> for SamplePosition {
    type Output = Self;

    #[inline]
    fn add(self, rhs: SampleCount) -> Self::Output {
        Self(self.0 + rhs.0 as u64)
    }
}

impl Sub for SamplePosition {
    type Output = SampleCount;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        SampleCount((self.0 - rhs.0) as usize)
    }
}

impl From<u64> for SamplePosition {
    fn from(pos: u64) -> Self {
        Self(pos)
    }
}

impl From<SamplePosition> for u64 {
    fn from(pos: SamplePosition) -> Self {
        pos.0
    }
}

impl std::fmt::Display for SamplePosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "@{}", self.0)
    }
}

/// Block size (number of samples processed at once).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct BlockSize(pub usize);

impl BlockSize {
    /// Create a new block size.
    #[inline]
    pub const fn new(size: usize) -> Self {
        Self(size)
    }

    /// Common block sizes.
    pub const B64: Self = Self(64);
    pub const B128: Self = Self(128);
    pub const B256: Self = Self(256);
    pub const B512: Self = Self(512);
    pub const B1024: Self = Self(1024);

    /// Get the raw value.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// Convert to sample count.
    #[inline]
    pub const fn as_sample_count(self) -> SampleCount {
        SampleCount(self.0)
    }

    /// Calculate latency in seconds.
    #[inline]
    pub fn latency(self, sample_rate: SampleRate) -> Seconds {
        Seconds::new(self.0 as f32 / sample_rate.0)
    }

    /// Check if this is a power of two (optimal for FFT, etc.).
    #[inline]
    pub const fn is_power_of_two(self) -> bool {
        self.0.is_power_of_two()
    }
}

impl Default for BlockSize {
    fn default() -> Self {
        Self::B256
    }
}

impl From<usize> for BlockSize {
    fn from(size: usize) -> Self {
        Self(size)
    }
}

impl From<BlockSize> for usize {
    fn from(size: BlockSize) -> Self {
        size.0
    }
}

impl From<BlockSize> for SampleCount {
    fn from(size: BlockSize) -> Self {
        SampleCount(size.0)
    }
}

impl std::fmt::Display for BlockSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} samples/block", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_count_to_seconds() {
        let samples = SampleCount::new(48000);
        let sr = SampleRate::DVD_QUALITY;
        let secs = samples.to_seconds(sr);
        assert!((secs.0 - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_sample_count_from_seconds() {
        let duration = Seconds::new(0.5);
        let sr = SampleRate::DVD_QUALITY;
        let samples = SampleCount::from_seconds(duration, sr);
        assert_eq!(samples.0, 24000);
    }

    #[test]
    fn test_sample_position_wrap() {
        let pos = SamplePosition::new(1030);
        let wrapped = pos.wrap(SampleCount::new(1024));
        assert_eq!(wrapped, 6);
    }

    #[test]
    fn test_block_size_latency() {
        let block = BlockSize::B256;
        let sr = SampleRate::DVD_QUALITY;
        let latency = block.latency(sr);
        assert!((latency.as_millis() - 5.33).abs() < 0.1);
    }
}
