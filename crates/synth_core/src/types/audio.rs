//! Additional audio domain types for type-safe processing.

use std::ops::{Add, Sub};

use serde::{Deserialize, Serialize};

use super::{Decibels, Gain};

/// Deprecated: Use Bpm instead.
#[deprecated(since = "0.33.0", note = "Use Bpm instead")]
pub type Tempo = super::Bpm;

/// Buffer index for delay lines and circular buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct BufferIndex(pub usize);

impl BufferIndex {
    /// Create a new buffer index.
    #[inline]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Zero index.
    pub const ZERO: Self = Self(0);

    /// Get the raw index.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// Advance by one, wrapping at buffer size.
    #[inline]
    pub fn advance(self, buffer_size: usize) -> Self {
        Self((self.0 + 1) % buffer_size)
    }

    /// Wrap to buffer size.
    #[inline]
    pub fn wrap(self, buffer_size: usize) -> Self {
        Self(self.0 % buffer_size)
    }

    /// Calculate read position for delay.
    #[inline]
    pub fn delay_read(self, delay_samples: usize, buffer_size: usize) -> Self {
        Self((self.0 + buffer_size - delay_samples) % buffer_size)
    }
}

impl Add<usize> for BufferIndex {
    type Output = Self;

    #[inline]
    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl Sub<usize> for BufferIndex {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: usize) -> Self::Output {
        Self(self.0.saturating_sub(rhs))
    }
}

impl From<usize> for BufferIndex {
    fn from(index: usize) -> Self {
        Self(index)
    }
}

impl From<BufferIndex> for usize {
    fn from(index: BufferIndex) -> Self {
        index.0
    }
}

/// Frame count (number of audio samples to process).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct FrameCount(pub usize);

impl FrameCount {
    /// Create a new frame count.
    #[inline]
    pub const fn new(count: usize) -> Self {
        Self(count)
    }

    /// Zero frames.
    pub const ZERO: Self = Self(0);

    /// Get the raw count.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// Convert to duration given a sample rate.
    #[inline]
    pub fn to_seconds(self, sample_rate: super::SampleRate) -> super::Seconds {
        super::Seconds::new(self.0 as f32 / sample_rate.as_f32())
    }
}

impl From<usize> for FrameCount {
    fn from(count: usize) -> Self {
        Self(count)
    }
}

impl From<FrameCount> for usize {
    fn from(count: FrameCount) -> Self {
        count.0
    }
}

/// Noise generator state (xorshift random state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct NoiseState(pub u32);

impl NoiseState {
    /// Create a new noise state with a seed.
    #[inline]
    pub const fn new(seed: u32) -> Self {
        Self(seed)
    }

    /// Default seed value.
    pub const DEFAULT: Self = Self(0x1234_5678);

    /// Get the raw state.
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Generate next random value using xorshift.
    #[inline]
    pub fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Generate next random value in 0..1 range.
    #[inline]
    pub fn next_unipolar(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32
    }
}

impl Default for NoiseState {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Filter state variable for IIR filters.
///
/// Used for one-pole lowpass, biquad states, etc.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(transparent)]
pub struct FilterState(pub f32);

impl FilterState {
    /// Create a new filter state.
    #[inline]
    pub const fn new(value: f32) -> Self {
        Self(value)
    }

    /// Zero state.
    pub const ZERO: Self = Self(0.0);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Reset to zero.
    #[inline]
    pub fn reset(&mut self) {
        self.0 = 0.0;
    }

    /// Apply denormal prevention.
    ///
    /// Adds a tiny DC offset to prevent denormal numbers which can
    /// cause severe performance degradation in filter processing.
    /// Call this after filter state updates to maintain performance.
    #[inline]
    pub fn flush_denormals(&mut self) {
        // Add tiny offset, then check if the value became denormal
        // If so, reset to zero. This is faster than adding offset every sample.
        if self.0.abs() < 1e-15 {
            self.0 = 0.0;
        }
    }

    /// Apply one-pole lowpass filter.
    #[inline]
    pub fn one_pole(&mut self, input: f32, coeff: f32) -> f32 {
        self.0 = input + (self.0 - input) * coeff;
        self.0
    }

    /// Process through first-order allpass filter.
    ///
    /// Implements: y[n] = coeff * (x[n] - y[n-1]) + y[n-1]
    /// The state stores the previous output (y[n-1]).
    /// Used in phasers for frequency-dependent phase shifting.
    #[inline]
    pub fn process_allpass(&mut self, input: f32, coeff: f32) -> f32 {
        let prev_output = self.0;
        let output = coeff * (input - prev_output) + prev_output;
        self.0 = output;
        output
    }

    /// High-pass one-pole filter.
    ///
    /// Returns the high-frequency content by subtracting lowpass from input.
    #[inline]
    pub fn one_pole_hp(&mut self, input: f32, coeff: f32) -> f32 {
        input - self.one_pole(input, coeff)
    }

    /// DC blocking filter.
    ///
    /// Removes DC offset from signal using a one-pole highpass.
    /// Typical coeff: 0.995 for 48kHz (approximately 10Hz cutoff)
    #[inline]
    pub fn dc_blocker(&mut self, input: f32, coeff: f32) -> f32 {
        let hp = input - self.0;
        self.0 = input * (1.0 - coeff) + self.0 * coeff;
        hp
    }

    /// Slew rate limiter.
    ///
    /// Limits how fast the output can change, useful for smoothing
    /// sudden parameter changes.
    ///
    /// # Arguments
    /// * `input` - Target value
    /// * `rise_rate` - Maximum increase per sample
    /// * `fall_rate` - Maximum decrease per sample
    #[inline]
    pub fn slew_limit(&mut self, input: f32, rise_rate: f32, fall_rate: f32) -> f32 {
        let delta = input - self.0;
        if delta > rise_rate {
            self.0 += rise_rate;
        } else if delta < -fall_rate {
            self.0 -= fall_rate;
        } else {
            self.0 = input;
        }
        self.0
    }

    /// Leaky integrator (accumulator with decay).
    ///
    /// Useful for envelope followers and RMS calculation.
    #[inline]
    pub fn leaky_integrate(&mut self, input: f32, attack: f32, release: f32) -> f32 {
        let coeff = if input > self.0 { attack } else { release };
        self.0 = input + (self.0 - input) * coeff;
        self.0
    }

    /// Soft knee saturation.
    ///
    /// Applies gentle saturation to prevent harsh clipping.
    #[inline]
    pub fn soft_saturate(value: f32, threshold: f32) -> f32 {
        if value.abs() < threshold {
            value
        } else {
            let sign = value.signum();
            let x = (value.abs() - threshold) / (1.0 - threshold);
            sign * (threshold + (1.0 - threshold) * x / (1.0 + x))
        }
    }
}

impl From<f32> for FilterState {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<FilterState> for f32 {
    fn from(state: FilterState) -> Self {
        state.0
    }
}

/// Amplitude for peak/RMS measurements.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Amplitude(pub f32);

impl Amplitude {
    /// Create a new amplitude value.
    #[inline]
    pub const fn new(value: f32) -> Self {
        Self(value)
    }

    /// Zero amplitude.
    pub const ZERO: Self = Self(0.0);

    /// Full scale.
    pub const FULL_SCALE: Self = Self(1.0);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to decibels.
    #[inline]
    pub fn to_db(self) -> Decibels {
        Decibels::from_linear(self.0)
    }

    /// Update with decay.
    #[inline]
    pub fn decay(&mut self, decay_factor: f32) {
        self.0 *= decay_factor;
    }

    /// Update to max of current and new value.
    #[inline]
    pub fn update_peak(&mut self, sample: f32) {
        self.0 = self.0.max(sample.abs());
    }
}

impl From<f32> for Amplitude {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<Amplitude> for f32 {
    fn from(amp: Amplitude) -> Self {
        amp.0
    }
}

/// Voice count for polyphony and chorus effects.
///
/// Represents the number of simultaneous voices in a module.
/// Typical range is 1-16 for polyphonic synths, 1-4 for chorus effects.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[repr(transparent)]
pub struct VoiceCount(pub u8);

impl VoiceCount {
    /// Create a new voice count, clamping to [1, 128].
    #[inline]
    pub fn new(count: u8) -> Self {
        Self(count.clamp(1, 128))
    }

    /// Create without clamping (for performance in hot paths).
    #[inline]
    pub const fn new_unchecked(count: u8) -> Self {
        Self(count)
    }

    /// Single voice (monophonic).
    pub const MONO: Self = Self(1);

    /// Dual voices.
    pub const DUAL: Self = Self(2);

    /// Four voices (common for chorus).
    pub const QUAD: Self = Self(4);

    /// Eight voices (default for synth).
    pub const OCTO: Self = Self(8);

    /// Sixteen voices.
    pub const SIXTEEN: Self = Self(16);

    /// Thirty-two voices.
    pub const THIRTYTWO: Self = Self(32);

    /// Maximum allocator voices.
    pub const MAX_ALLOCATOR: Self = Self(128);

    /// Get the raw voice count.
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get as usize for array indexing.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Clamp to a chorus-appropriate range (1-4).
    #[inline]
    pub fn clamp_chorus(self) -> Self {
        Self(self.0.clamp(1, 4))
    }

    /// Clamp to a synth polyphony range (1-16).
    #[inline]
    pub fn clamp_polyphony(self) -> Self {
        Self(self.0.clamp(1, 16))
    }

    /// Clamp to voice allocator range (1-128).
    #[inline]
    pub fn clamp_allocator(self) -> Self {
        Self(self.0.clamp(1, 128))
    }
}

impl From<u8> for VoiceCount {
    fn from(count: u8) -> Self {
        Self::new(count)
    }
}

impl From<u32> for VoiceCount {
    fn from(count: u32) -> Self {
        Self::new(count.min(128) as u8)
    }
}

impl From<usize> for VoiceCount {
    fn from(count: usize) -> Self {
        Self::new(count.min(128) as u8)
    }
}

impl From<VoiceCount> for u8 {
    fn from(count: VoiceCount) -> Self {
        count.0
    }
}

impl From<VoiceCount> for usize {
    fn from(count: VoiceCount) -> Self {
        count.0 as Self
    }
}

impl std::fmt::Display for VoiceCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// STEREO BALANCE
// ============================================================================

/// Stereo balance/pan position (-1.0 = full left, 0.0 = center, +1.0 = full right).
///
/// This is a specialized wrapper around `BipolarValue` for stereo positioning.
/// It provides convenience methods for pan-specific calculations.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct StereoBalance(pub super::BipolarValue);

impl StereoBalance {
    /// Create a new stereo balance.
    #[inline]
    pub fn new(value: f32) -> Self {
        Self(super::BipolarValue::new(value))
    }

    /// Full left (-1.0).
    pub const LEFT: Self = Self(super::BipolarValue::MIN);

    /// Center (0.0).
    pub const CENTER: Self = Self(super::BipolarValue::CENTER);

    /// Full right (+1.0).
    pub const RIGHT: Self = Self(super::BipolarValue::MAX);

    /// Get the raw value.
    #[inline]
    pub fn as_f32(self) -> f32 {
        self.0.as_f32()
    }

    /// Get the underlying BipolarValue.
    #[inline]
    pub fn as_bipolar(self) -> super::BipolarValue {
        self.0
    }

    /// Calculate left and right gains using constant power panning.
    ///
    /// Returns (left_gain, right_gain) where both are in 0.0-1.0 range.
    #[inline]
    #[must_use]
    pub fn gains(self) -> (Gain, Gain) {
        // Constant power panning using sin/cos
        let angle = (self.0.as_f32() + 1.0) * 0.25 * std::f32::consts::PI;
        let left = angle.cos();
        let right = angle.sin();
        (Gain::new(left), Gain::new(right))
    }

    /// Calculate left and right gains using linear panning.
    ///
    /// Returns (left_gain, right_gain) where both are in 0.0-1.0 range.
    #[inline]
    #[must_use]
    pub fn linear_gains(self) -> (Gain, Gain) {
        let pan = self.0.as_f32();
        let left = (1.0 - pan) * 0.5;
        let right = (1.0 + pan) * 0.5;
        (Gain::new(left), Gain::new(right))
    }

    /// Clamp to valid range.
    #[inline]
    pub fn clamp(self) -> Self {
        Self::new(self.0.as_f32())
    }
}

impl From<super::BipolarValue> for StereoBalance {
    fn from(value: super::BipolarValue) -> Self {
        Self(value)
    }
}

impl From<StereoBalance> for super::BipolarValue {
    fn from(balance: StereoBalance) -> Self {
        balance.0
    }
}

impl From<f32> for StereoBalance {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<StereoBalance> for f32 {
    fn from(balance: StereoBalance) -> Self {
        balance.as_f32()
    }
}

impl std::fmt::Display for StereoBalance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = self.0.as_f32();
        if value < -0.01 {
            write!(f, "L{:.0}%", -value * 100.0)
        } else if value > 0.01 {
            write!(f, "R{:.0}%", value * 100.0)
        } else {
            write!(f, "C")
        }
    }
}

// ============================================================================
// STEREO SAMPLE
// ============================================================================

/// A single stereo audio sample with left and right channels.
///
/// This type replaces `(f32, f32)` and `[f32; 2]` for stereo signals,
/// providing better ergonomics and type safety.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StereoSample {
    /// Left channel sample value.
    pub left: f32,
    /// Right channel sample value.
    pub right: f32,
}

impl StereoSample {
    /// Silence (both channels zero).
    pub const ZERO: Self = Self {
        left: 0.0,
        right: 0.0,
    };

    /// Create a new stereo sample from left and right values.
    #[inline]
    pub const fn new(left: f32, right: f32) -> Self {
        Self { left, right }
    }

    /// Create a stereo sample from a mono source (both channels equal).
    #[inline]
    pub const fn from_mono(mono: f32) -> Self {
        Self {
            left: mono,
            right: mono,
        }
    }

    /// Apply a gain factor to both channels.
    #[inline]
    pub fn apply_gain(self, gain: Gain) -> Self {
        let g = gain.as_f32();
        Self {
            left: self.left * g,
            right: self.right * g,
        }
    }

    /// Apply separate gains to left and right channels.
    #[inline]
    pub fn apply_stereo_gain(self, left_gain: Gain, right_gain: Gain) -> Self {
        Self {
            left: self.left * left_gain.as_f32(),
            right: self.right * right_gain.as_f32(),
        }
    }

    /// Apply panning using a StereoBalance.
    #[inline]
    pub fn apply_pan(self, pan: StereoBalance) -> Self {
        let (left_gain, right_gain) = pan.gains();
        self.apply_stereo_gain(left_gain, right_gain)
    }

    /// Convert to mono by averaging left and right channels.
    #[inline]
    pub fn to_mono(self) -> f32 {
        (self.left + self.right) * 0.5
    }

    /// Mix with another stereo sample.
    #[inline]
    pub fn mix(self, other: Self) -> Self {
        Self {
            left: self.left + other.left,
            right: self.right + other.right,
        }
    }

    /// Soft clip both channels to prevent harsh clipping.
    #[inline]
    pub fn soft_clip(self) -> Self {
        Self {
            left: self.left.tanh(),
            right: self.right.tanh(),
        }
    }

    /// Hard clip both channels to [-1, 1].
    #[inline]
    pub fn hard_clip(self) -> Self {
        Self {
            left: self.left.clamp(-1.0, 1.0),
            right: self.right.clamp(-1.0, 1.0),
        }
    }

    /// Get peak amplitude (max of absolute values).
    #[inline]
    pub fn peak(self) -> f32 {
        self.left.abs().max(self.right.abs())
    }
}

impl std::ops::Add for StereoSample {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            left: self.left + rhs.left,
            right: self.right + rhs.right,
        }
    }
}

impl std::ops::AddAssign for StereoSample {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.left += rhs.left;
        self.right += rhs.right;
    }
}

impl std::ops::Sub for StereoSample {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            left: self.left - rhs.left,
            right: self.right - rhs.right,
        }
    }
}

impl std::ops::Mul<f32> for StereoSample {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            left: self.left * rhs,
            right: self.right * rhs,
        }
    }
}

impl std::ops::MulAssign<f32> for StereoSample {
    #[inline]
    fn mul_assign(&mut self, rhs: f32) {
        self.left *= rhs;
        self.right *= rhs;
    }
}

impl From<(f32, f32)> for StereoSample {
    fn from((left, right): (f32, f32)) -> Self {
        Self { left, right }
    }
}

impl From<StereoSample> for (f32, f32) {
    fn from(sample: StereoSample) -> Self {
        (sample.left, sample.right)
    }
}

impl From<[f32; 2]> for StereoSample {
    fn from([left, right]: [f32; 2]) -> Self {
        Self { left, right }
    }
}

impl From<StereoSample> for [f32; 2] {
    fn from(sample: StereoSample) -> Self {
        [sample.left, sample.right]
    }
}

// ============================================================================
// CPU USAGE
// ============================================================================

/// CPU usage percentage (0.0 - 1.0).
///
/// Represents the fraction of available CPU time used by audio processing.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct CpuUsage(pub f32);

impl CpuUsage {
    /// Create a new CPU usage value, clamping to [0, 1].
    #[inline]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    /// Zero usage.
    pub const ZERO: Self = Self(0.0);

    /// Full usage (100%).
    pub const FULL: Self = Self(1.0);

    /// Get the raw value (0.0 - 1.0).
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Get as percentage (0 - 100).
    #[inline]
    pub fn as_percent(self) -> f32 {
        self.0 * 100.0
    }

    /// Check if usage is critical (> 80%).
    #[inline]
    pub fn is_critical(self) -> bool {
        self.0 > 0.8
    }

    /// Check if usage is warning level (> 60%).
    #[inline]
    pub fn is_warning(self) -> bool {
        self.0 > 0.6
    }
}

impl From<f32> for CpuUsage {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<CpuUsage> for f32 {
    fn from(usage: CpuUsage) -> Self {
        usage.0
    }
}

impl std::fmt::Display for CpuUsage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1}%", self.0 * 100.0)
    }
}

// ============================================================================
// STEREO LEVELS
// ============================================================================

/// Stereo level measurements for meters and visualization.
///
/// Contains separate amplitude measurements for left and right channels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StereoLevels {
    /// Left channel amplitude.
    pub left: Amplitude,
    /// Right channel amplitude.
    pub right: Amplitude,
}

impl StereoLevels {
    /// Create new stereo levels.
    #[inline]
    pub const fn new(left: Amplitude, right: Amplitude) -> Self {
        Self { left, right }
    }

    /// Zero levels (silence).
    pub const ZERO: Self = Self {
        left: Amplitude::ZERO,
        right: Amplitude::ZERO,
    };

    /// Create from mono level (both channels equal).
    #[inline]
    pub const fn from_mono(level: Amplitude) -> Self {
        Self {
            left: level,
            right: level,
        }
    }

    /// Get the maximum of left and right.
    #[inline]
    pub fn peak(self) -> Amplitude {
        Amplitude::new(self.left.as_f32().max(self.right.as_f32()))
    }

    /// Convert both channels to decibels.
    #[inline]
    pub fn to_db(self) -> (Decibels, Decibels) {
        (self.left.to_db(), self.right.to_db())
    }

    /// Apply decay to both channels.
    #[inline]
    pub fn decay(&mut self, factor: f32) {
        self.left.decay(factor);
        self.right.decay(factor);
    }

    /// Update with peak values from a stereo sample.
    #[inline]
    pub fn update_peak(&mut self, left: f32, right: f32) {
        self.left.update_peak(left);
        self.right.update_peak(right);
    }
}

impl From<(f32, f32)> for StereoLevels {
    fn from((left, right): (f32, f32)) -> Self {
        Self {
            left: Amplitude::new(left),
            right: Amplitude::new(right),
        }
    }
}

impl From<StereoLevels> for (f32, f32) {
    fn from(levels: StereoLevels) -> Self {
        (levels.left.as_f32(), levels.right.as_f32())
    }
}

// ============================================================================
// PATTERN INDEX
// ============================================================================

/// Index of a pattern in the song arrangement.
///
/// Zero-indexed position in the pattern order list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct PatternIndex(pub usize);

impl PatternIndex {
    /// Create a new pattern index.
    #[inline]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// First pattern.
    pub const ZERO: Self = Self(0);

    /// Get the raw index.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }

    /// Advance to next pattern.
    #[inline]
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// Go to previous pattern (saturating).
    #[inline]
    pub fn prev(self) -> Self {
        Self(self.0.saturating_sub(1))
    }
}

impl From<usize> for PatternIndex {
    fn from(index: usize) -> Self {
        Self(index)
    }
}

impl From<PatternIndex> for usize {
    fn from(index: PatternIndex) -> Self {
        index.0
    }
}

impl std::fmt::Display for PatternIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pattern {}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_count() {
        let voices = VoiceCount::QUAD;
        assert_eq!(voices.as_usize(), 4);

        let clamped = VoiceCount::new(10).clamp_chorus();
        assert_eq!(clamped.as_u8(), 4);
    }

    #[test]
    fn test_bpm() {
        use super::super::Bpm;
        let tempo = Bpm::new(120.0);
        // At 120 BPM, beat duration = 60/120 = 0.5s
        assert!((tempo.beat_duration().as_f32() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_buffer_index() {
        let idx = BufferIndex::new(5);
        let wrapped = idx.advance(8);
        assert_eq!(wrapped.as_usize(), 6);

        let idx2 = BufferIndex::new(7);
        let wrapped2 = idx2.advance(8);
        assert_eq!(wrapped2.as_usize(), 0);
    }

    #[test]
    fn test_noise_state() {
        let mut noise = NoiseState::DEFAULT;
        let v1 = noise.next();
        let v2 = noise.next();
        assert!(v1 >= -1.0 && v1 <= 1.0);
        assert!(v2 >= -1.0 && v2 <= 1.0);
        assert!((v1 - v2).abs() > 0.001); // Should be different
    }

    #[test]
    fn test_amplitude() {
        let mut amp = Amplitude::ZERO;
        amp.update_peak(0.5);
        assert!((amp.as_f32() - 0.5).abs() < 0.001);
        amp.update_peak(-0.8);
        assert!((amp.as_f32() - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_dc_blocker() {
        let mut state = FilterState::ZERO;
        // DC offset should be removed over time
        let mut output = 0.0;
        for _ in 0..1000 {
            output = state.dc_blocker(1.0, 0.995);
        }
        assert!(output.abs() < 0.1);
    }

    #[test]
    fn test_slew_limit() {
        let mut state = FilterState::new(0.0);
        let result = state.slew_limit(1.0, 0.1, 0.1);
        assert!((result - 0.1).abs() < 0.001);
    }

    #[test]
    fn test_soft_saturate() {
        // Below threshold - linear
        let v1 = FilterState::soft_saturate(0.5, 0.8);
        assert!((v1 - 0.5).abs() < 0.001);

        // Above threshold - saturated
        let v2 = FilterState::soft_saturate(1.5, 0.8);
        assert!(v2 < 1.5 && v2 > 0.8);
    }

    #[test]
    fn test_stereo_balance() {
        let center = StereoBalance::CENTER;
        let (l, r) = center.gains();
        // At center, both should be equal (constant power = sqrt(0.5) ≈ 0.707)
        assert!((l.as_f32() - r.as_f32()).abs() < 0.001);

        let left = StereoBalance::LEFT;
        let (l, r) = left.gains();
        assert!(l.as_f32() > 0.99); // Full left
        assert!(r.as_f32() < 0.01); // No right

        let right = StereoBalance::RIGHT;
        let (l, r) = right.gains();
        assert!(l.as_f32() < 0.01); // No left
        assert!(r.as_f32() > 0.99); // Full right
    }

    #[test]
    fn test_stereo_balance_display() {
        assert_eq!(format!("{}", StereoBalance::CENTER), "C");
        assert_eq!(format!("{}", StereoBalance::LEFT), "L100%");
        assert_eq!(format!("{}", StereoBalance::RIGHT), "R100%");
    }

    #[test]
    fn test_stereo_sample_new() {
        let sample = StereoSample::new(0.5, -0.5);
        assert!((sample.left - 0.5).abs() < 0.001);
        assert!((sample.right - (-0.5)).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_from_mono() {
        let sample = StereoSample::from_mono(0.7);
        assert!((sample.left - 0.7).abs() < 0.001);
        assert!((sample.right - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_apply_gain() {
        let sample = StereoSample::new(1.0, 0.5);
        let gained = sample.apply_gain(Gain::new(0.5));
        assert!((gained.left - 0.5).abs() < 0.001);
        assert!((gained.right - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_apply_pan() {
        let sample = StereoSample::from_mono(1.0);

        // Pan full left
        let left_panned = sample.apply_pan(StereoBalance::LEFT);
        assert!(left_panned.left > 0.99);
        assert!(left_panned.right < 0.01);

        // Pan full right
        let right_panned = sample.apply_pan(StereoBalance::RIGHT);
        assert!(right_panned.left < 0.01);
        assert!(right_panned.right > 0.99);
    }

    #[test]
    fn test_stereo_sample_to_mono() {
        let sample = StereoSample::new(0.8, 0.4);
        let mono = sample.to_mono();
        assert!((mono - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_mix() {
        let a = StereoSample::new(0.3, 0.5);
        let b = StereoSample::new(0.2, 0.1);
        let mixed = a.mix(b);
        assert!((mixed.left - 0.5).abs() < 0.001);
        assert!((mixed.right - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_hard_clip() {
        let sample = StereoSample::new(1.5, -2.0);
        let clipped = sample.hard_clip();
        assert!((clipped.left - 1.0).abs() < 0.001);
        assert!((clipped.right - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_peak() {
        let sample = StereoSample::new(0.3, -0.8);
        assert!((sample.peak() - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_add() {
        let a = StereoSample::new(0.3, 0.4);
        let b = StereoSample::new(0.1, 0.2);
        let sum = a + b;
        assert!((sum.left - 0.4).abs() < 0.001);
        assert!((sum.right - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_mul() {
        let sample = StereoSample::new(0.5, 0.8);
        let scaled = sample * 2.0;
        assert!((scaled.left - 1.0).abs() < 0.001);
        assert!((scaled.right - 1.6).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_from_tuple() {
        let sample: StereoSample = (0.5, 0.7).into();
        assert!((sample.left - 0.5).abs() < 0.001);
        assert!((sample.right - 0.7).abs() < 0.001);

        let tuple: (f32, f32) = sample.into();
        assert!((tuple.0 - 0.5).abs() < 0.001);
        assert!((tuple.1 - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_stereo_sample_from_array() {
        let sample: StereoSample = [0.3, 0.9].into();
        assert!((sample.left - 0.3).abs() < 0.001);
        assert!((sample.right - 0.9).abs() < 0.001);

        let arr: [f32; 2] = sample.into();
        assert!((arr[0] - 0.3).abs() < 0.001);
        assert!((arr[1] - 0.9).abs() < 0.001);
    }

    // === Tests for new types ===

    #[test]
    fn test_cpu_usage() {
        let usage = CpuUsage::new(0.75);
        assert!((usage.as_f32() - 0.75).abs() < 0.001);
        assert!((usage.as_percent() - 75.0).abs() < 0.1);
        assert!(usage.is_warning());
        assert!(!usage.is_critical());

        let critical = CpuUsage::new(0.9);
        assert!(critical.is_critical());

        // Test clamping
        let over = CpuUsage::new(1.5);
        assert!((over.as_f32() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_stereo_levels() {
        let levels = StereoLevels::new(Amplitude::new(0.8), Amplitude::new(0.6));
        assert!((levels.peak().as_f32() - 0.8).abs() < 0.001);

        let mono = StereoLevels::from_mono(Amplitude::new(0.5));
        assert!((mono.left.as_f32() - 0.5).abs() < 0.001);
        assert!((mono.right.as_f32() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_pattern_index() {
        let idx = PatternIndex::ZERO;
        assert_eq!(idx.as_usize(), 0);

        let next = idx.next();
        assert_eq!(next.as_usize(), 1);

        let prev = idx.prev();
        assert_eq!(prev.as_usize(), 0); // Saturating
    }

    #[test]
    fn test_voice_count_clamping() {
        // Test allocator range
        let voices = VoiceCount::new(200);
        assert_eq!(voices.as_u8(), 128);

        let zero = VoiceCount::new(0);
        assert_eq!(zero.as_u8(), 1);

        let valid = VoiceCount::new(64);
        assert_eq!(valid.as_u8(), 64);
    }
}
