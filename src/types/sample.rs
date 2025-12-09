//! Sample-related types for audio sampling.
//!
//! This module provides types for working with audio samples,
//! including sample values, positions, playback modes, and interpolation.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{MidiNote, SampleRate};

/// A single audio sample value in the range -1.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct SampleValue(pub f32);

impl SampleValue {
    /// Create a new sample value.
    #[inline]
    pub const fn new(value: f32) -> Self {
        Self(value)
    }

    /// Silence (0.0).
    pub const ZERO: Self = Self(0.0);

    /// Maximum positive value.
    pub const MAX: Self = Self(1.0);

    /// Maximum negative value.
    pub const MIN: Self = Self(-1.0);

    /// Get the raw f32 value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Linear interpolation between self and other.
    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self(self.0 + (other.0 - self.0) * t)
    }

    /// Clamp to valid range (-1.0 to 1.0).
    #[inline]
    pub fn clamp(self) -> Self {
        Self(self.0.clamp(-1.0, 1.0))
    }

    /// Scale by a factor.
    #[inline]
    pub fn scale(self, factor: f32) -> Self {
        Self(self.0 * factor)
    }
}

impl From<f32> for SampleValue {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<SampleValue> for f32 {
    fn from(value: SampleValue) -> Self {
        value.0
    }
}

/// An integer index into sample data.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct SampleIndex(pub usize);

impl SampleIndex {
    /// Create a new sample index.
    #[inline]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Zero index.
    pub const ZERO: Self = Self(0);

    /// Get the raw usize value.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl From<usize> for SampleIndex {
    fn from(index: usize) -> Self {
        Self(index)
    }
}

impl From<SampleIndex> for usize {
    fn from(index: SampleIndex) -> Self {
        index.0
    }
}

/// A fractional position within sample data for interpolation.
///
/// This is different from `SamplePosition` which is an integer position
/// in a buffer. `PlaybackPosition` supports fractional positions needed
/// for sample playback with pitch shifting and interpolation.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct PlaybackPosition(pub f64);

impl PlaybackPosition {
    /// Create a new playback position.
    #[inline]
    pub const fn new(position: f64) -> Self {
        Self(position)
    }

    /// Start position (0.0).
    pub const ZERO: Self = Self(0.0);

    /// Get the raw f64 value.
    #[inline]
    pub const fn as_f64(self) -> f64 {
        self.0
    }

    /// Get the integer part (sample index).
    #[inline]
    pub fn index(self) -> SampleIndex {
        SampleIndex(self.0 as usize)
    }

    /// Get the fractional part (for interpolation).
    #[inline]
    pub fn fraction(self) -> f32 {
        (self.0.fract()) as f32
    }

    /// Advance by a given amount.
    #[inline]
    pub fn advance(self, amount: f64) -> Self {
        Self(self.0 + amount)
    }

    /// Wrap around a given length.
    #[inline]
    pub fn wrap(self, length: usize) -> Self {
        if length == 0 {
            Self::ZERO
        } else {
            Self(self.0.rem_euclid(length as f64))
        }
    }
}

impl From<f64> for PlaybackPosition {
    fn from(position: f64) -> Self {
        Self(position)
    }
}

impl From<PlaybackPosition> for f64 {
    fn from(position: PlaybackPosition) -> Self {
        position.0
    }
}

/// Playback speed multiplier.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct PlaybackSpeed(pub f32);

impl PlaybackSpeed {
    /// Create a new playback speed.
    #[inline]
    pub const fn new(speed: f32) -> Self {
        Self(speed)
    }

    /// Normal speed (1.0).
    pub const NORMAL: Self = Self(1.0);

    /// Half speed (0.5).
    pub const HALF: Self = Self(0.5);

    /// Double speed (2.0).
    pub const DOUBLE: Self = Self(2.0);

    /// Get the raw f32 value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Clamp to valid range (0.01 to 10.0).
    #[inline]
    pub fn clamp(self) -> Self {
        Self(self.0.clamp(0.01, 10.0))
    }
}

impl Default for PlaybackSpeed {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl From<f32> for PlaybackSpeed {
    fn from(speed: f32) -> Self {
        Self(speed)
    }
}

impl From<PlaybackSpeed> for f32 {
    fn from(speed: PlaybackSpeed) -> Self {
        speed.0
    }
}

/// Channel configuration for samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ChannelMode {
    /// Mono sample (single channel).
    #[default]
    Mono,
    /// Stereo sample (left and right channels interleaved).
    Stereo,
}

impl ChannelMode {
    /// Number of channels.
    #[inline]
    pub const fn channel_count(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
        }
    }
}

/// A region within a sample for looping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LoopRegion {
    /// Start index of the loop.
    pub start: SampleIndex,
    /// End index of the loop (exclusive).
    pub end: SampleIndex,
}

impl LoopRegion {
    /// Create a new loop region.
    #[inline]
    pub const fn new(start: SampleIndex, end: SampleIndex) -> Self {
        Self { start, end }
    }

    /// Get the length of the loop region.
    #[inline]
    pub fn len(&self) -> usize {
        self.end.0.saturating_sub(self.start.0)
    }

    /// Check if the region is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// How the sample should loop (or not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LoopMode {
    /// Play once and stop.
    #[default]
    OneShot,
    /// Loop forward continuously.
    Forward(LoopRegion),
    /// Loop back and forth (ping-pong).
    PingPong(LoopRegion),
}

/// Interpolation method for sample playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Interpolation {
    /// No interpolation (nearest sample).
    Nearest,
    /// Linear interpolation between adjacent samples.
    #[default]
    Linear,
    /// Cubic interpolation for smoother results.
    Cubic,
}

/// Playback direction (for ping-pong looping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PlaybackDirection {
    /// Playing forward.
    #[default]
    Forward,
    /// Playing backward.
    Backward,
}

impl PlaybackDirection {
    /// Reverse the direction.
    #[inline]
    pub fn reverse(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
        }
    }

    /// Get the position delta sign.
    #[inline]
    pub fn sign(self) -> f64 {
        match self {
            Self::Forward => 1.0,
            Self::Backward => -1.0,
        }
    }
}

/// An audio sample with metadata.
///
/// Samples are immutable and can be safely shared across threads via `Arc`.
#[derive(Debug, Clone)]
pub struct Sample {
    /// Name of the sample (usually filename).
    pub name: String,
    /// Audio data as sample values.
    pub data: Arc<[SampleValue]>,
    /// Channel configuration.
    pub channels: ChannelMode,
    /// Original sample rate.
    pub sample_rate: SampleRate,
    /// Root note (pitch at normal speed).
    pub root_note: MidiNote,
    /// Loop mode.
    pub loop_mode: LoopMode,
}

impl Sample {
    /// Create a new sample.
    pub fn new(
        name: String,
        data: Vec<SampleValue>,
        channels: ChannelMode,
        sample_rate: SampleRate,
    ) -> Self {
        Self {
            name,
            data: data.into(),
            channels,
            sample_rate,
            root_note: MidiNote::C4,
            loop_mode: LoopMode::OneShot,
        }
    }

    /// Get the length of the sample in frames (not individual samples).
    #[inline]
    pub fn len(&self) -> SampleIndex {
        SampleIndex(self.data.len() / self.channels.channel_count())
    }

    /// Check if the sample is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Read a stereo sample at the given position with interpolation.
    ///
    /// For mono samples, the value is duplicated to both channels.
    /// Returns (left, right) sample values.
    pub fn read(
        &self,
        position: PlaybackPosition,
        interp: Interpolation,
    ) -> (SampleValue, SampleValue) {
        if self.is_empty() {
            return (SampleValue::ZERO, SampleValue::ZERO);
        }

        let frame_count = self.len().0;
        if frame_count == 0 {
            return (SampleValue::ZERO, SampleValue::ZERO);
        }

        match self.channels {
            ChannelMode::Mono => {
                let value = self.read_mono(position, interp, frame_count);
                (value, value)
            }
            ChannelMode::Stereo => self.read_stereo(position, interp, frame_count),
        }
    }

    /// Read a mono sample with interpolation.
    fn read_mono(
        &self,
        position: PlaybackPosition,
        interp: Interpolation,
        len: usize,
    ) -> SampleValue {
        let pos = position.0;
        let idx = pos as usize;

        if idx >= len {
            return SampleValue::ZERO;
        }

        match interp {
            Interpolation::Nearest => self.data[idx],
            Interpolation::Linear => {
                let frac = position.fraction();
                let s0 = self.data[idx];
                let s1 = if idx + 1 < len {
                    self.data[idx + 1]
                } else {
                    SampleValue::ZERO
                };
                s0.lerp(s1, frac)
            }
            Interpolation::Cubic => self.cubic_interp_mono(idx, position.fraction(), len),
        }
    }

    /// Read a stereo sample with interpolation.
    fn read_stereo(
        &self,
        position: PlaybackPosition,
        interp: Interpolation,
        frame_count: usize,
    ) -> (SampleValue, SampleValue) {
        let pos = position.0;
        let idx = pos as usize;

        if idx >= frame_count {
            return (SampleValue::ZERO, SampleValue::ZERO);
        }

        let base = idx * 2;

        match interp {
            Interpolation::Nearest => {
                let left = self.data.get(base).copied().unwrap_or(SampleValue::ZERO);
                let right = self
                    .data
                    .get(base + 1)
                    .copied()
                    .unwrap_or(SampleValue::ZERO);
                (left, right)
            }
            Interpolation::Linear => {
                let frac = position.fraction();
                let l0 = self.data.get(base).copied().unwrap_or(SampleValue::ZERO);
                let r0 = self
                    .data
                    .get(base + 1)
                    .copied()
                    .unwrap_or(SampleValue::ZERO);
                let l1 = self
                    .data
                    .get(base + 2)
                    .copied()
                    .unwrap_or(SampleValue::ZERO);
                let r1 = self
                    .data
                    .get(base + 3)
                    .copied()
                    .unwrap_or(SampleValue::ZERO);
                (l0.lerp(l1, frac), r0.lerp(r1, frac))
            }
            Interpolation::Cubic => self.cubic_interp_stereo(idx, position.fraction(), frame_count),
        }
    }

    /// Cubic interpolation for mono samples.
    fn cubic_interp_mono(&self, idx: usize, frac: f32, len: usize) -> SampleValue {
        let get = |i: isize| -> f32 {
            let clamped = i.clamp(0, (len - 1) as isize) as usize;
            self.data[clamped].0
        };

        let idx = idx as isize;
        let y0 = get(idx - 1);
        let y1 = get(idx);
        let y2 = get(idx + 1);
        let y3 = get(idx + 2);

        let t = frac;
        let t2 = t * t;
        let t3 = t2 * t;

        // Catmull-Rom spline
        let a0 = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
        let a1 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let a2 = -0.5 * y0 + 0.5 * y2;
        let a3 = y1;

        SampleValue(a0 * t3 + a1 * t2 + a2 * t + a3)
    }

    /// Cubic interpolation for stereo samples.
    fn cubic_interp_stereo(
        &self,
        idx: usize,
        frac: f32,
        frame_count: usize,
    ) -> (SampleValue, SampleValue) {
        let get = |i: isize, ch: usize| -> f32 {
            let clamped = i.clamp(0, (frame_count - 1) as isize) as usize;
            let data_idx = clamped * 2 + ch;
            self.data.get(data_idx).map(|v| v.0).unwrap_or(0.0)
        };

        let idx = idx as isize;
        let t = frac;
        let t2 = t * t;
        let t3 = t2 * t;

        let mut result = [0.0f32; 2];

        for ch in 0..2 {
            let y0 = get(idx - 1, ch);
            let y1 = get(idx, ch);
            let y2 = get(idx + 1, ch);
            let y3 = get(idx + 2, ch);

            // Catmull-Rom spline
            let a0 = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
            let a1 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let a2 = -0.5 * y0 + 0.5 * y2;
            let a3 = y1;

            result[ch] = a0 * t3 + a1 * t2 + a2 * t + a3;
        }

        (SampleValue(result[0]), SampleValue(result[1]))
    }

    /// Calculate the pitch ratio to play at a given note.
    ///
    /// Returns the speed multiplier needed to transpose from root_note to target_note.
    pub fn pitch_ratio(&self, target_note: MidiNote) -> f32 {
        let semitone_diff = target_note.0 as f32 - self.root_note.0 as f32;
        2.0f32.powf(semitone_diff / 12.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_value_lerp() {
        let a = SampleValue::new(0.0);
        let b = SampleValue::new(1.0);

        let mid = a.lerp(b, 0.5);
        assert!((mid.0 - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_playback_position_wrap() {
        let pos = PlaybackPosition::new(10.5);
        let wrapped = pos.wrap(8);
        assert!((wrapped.0 - 2.5).abs() < 0.001);
    }

    #[test]
    fn test_playback_direction() {
        let dir = PlaybackDirection::Forward;
        assert_eq!(dir.reverse(), PlaybackDirection::Backward);
        assert!((dir.sign() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_sample_read_mono() {
        let data: Vec<SampleValue> = vec![
            SampleValue::new(0.0),
            SampleValue::new(0.5),
            SampleValue::new(1.0),
            SampleValue::new(0.5),
        ];
        let sample = Sample::new(
            "test".to_string(),
            data,
            ChannelMode::Mono,
            SampleRate::CD_QUALITY,
        );

        let (l, r) = sample.read(PlaybackPosition::new(1.0), Interpolation::Nearest);
        assert!((l.0 - 0.5).abs() < 0.001);
        assert_eq!(l, r); // Mono should duplicate
    }

    #[test]
    fn test_sample_pitch_ratio() {
        let sample = Sample::new(
            "test".to_string(),
            vec![SampleValue::ZERO],
            ChannelMode::Mono,
            SampleRate::CD_QUALITY,
        );

        // Playing at root note should give ratio 1.0
        let ratio = sample.pitch_ratio(MidiNote::C4);
        assert!((ratio - 1.0).abs() < 0.001);

        // One octave up should give ratio 2.0
        let ratio = sample.pitch_ratio(MidiNote::new(72)); // C5
        assert!((ratio - 2.0).abs() < 0.001);
    }
}
