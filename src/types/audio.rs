//! Additional audio domain types for type-safe processing.

use std::ops::{Add, Sub};

use serde::{Deserialize, Serialize};

use super::Decibels;

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
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
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
    /// Create a new voice count.
    #[inline]
    pub const fn new(count: u8) -> Self {
        Self(count)
    }

    /// Single voice (monophonic).
    pub const MONO: Self = Self(1);

    /// Dual voices.
    pub const DUAL: Self = Self(2);

    /// Four voices (common for chorus).
    pub const QUAD: Self = Self(4);

    /// Eight voices.
    pub const OCTO: Self = Self(8);

    /// Sixteen voices.
    pub const SIXTEEN: Self = Self(16);

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
}

impl From<u8> for VoiceCount {
    fn from(count: u8) -> Self {
        Self(count.max(1))
    }
}

impl From<u32> for VoiceCount {
    fn from(count: u32) -> Self {
        Self((count.clamp(1, 255)) as u8)
    }
}

impl From<VoiceCount> for u8 {
    fn from(count: VoiceCount) -> Self {
        count.0
    }
}

impl From<VoiceCount> for usize {
    fn from(count: VoiceCount) -> Self {
        count.0 as usize
    }
}

impl std::fmt::Display for VoiceCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
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
}
