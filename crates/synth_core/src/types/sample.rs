//! Sample-related types for audio sampling.
//!
//! This module provides types for working with audio samples,
//! including sample values, positions, playback modes, and interpolation.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{MidiNote, SampleRate};

// ============================================================================
// SAMPLE VALUE
// ============================================================================

/// A single audio sample value in the range -1.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
#[must_use]
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

    /// Add another sample value.
    #[inline]
    pub fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
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

impl std::ops::Add for SampleValue {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

// ============================================================================
// SAMPLE INDEX
// ============================================================================

/// An integer index into sample data.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
#[must_use]
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

// ============================================================================
// PLAYBACK POSITION
// ============================================================================

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

// ============================================================================
// PLAYBACK SPEED
// ============================================================================

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

// ============================================================================
// PLAYBACK STATE
// ============================================================================

/// Current playback state of a sample player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PlaybackState {
    /// Not playing.
    #[default]
    Stopped,
    /// Currently playing.
    Playing,
}

impl PlaybackState {
    /// Check if currently playing.
    #[inline]
    pub const fn is_playing(self) -> bool {
        matches!(self, Self::Playing)
    }

    /// Check if stopped.
    #[inline]
    pub const fn is_stopped(self) -> bool {
        matches!(self, Self::Stopped)
    }
}

// ============================================================================
// CHANNEL MODE
// ============================================================================

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

// ============================================================================
// PLAYBACK DIRECTION
// ============================================================================

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
    #[must_use]
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

// ============================================================================
// LOOP MODE
// ============================================================================

/// Loop modes for sample playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LoopMode {
    #[default]
    Off,
    Forward,
    Backward,
    PingPong,
}

impl LoopMode {
    pub const ALL: [Self; 4] = [Self::Off, Self::Forward, Self::Backward, Self::PingPong];

    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Forward => "Forward",
            Self::Backward => "Backward",
            Self::PingPong => "Ping-Pong",
        }
    }

    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Forward => "forward",
            Self::Backward => "backward",
            Self::PingPong => "ping_pong",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "off" => Some(Self::Off),
            "forward" => Some(Self::Forward),
            "backward" => Some(Self::Backward),
            "ping_pong" => Some(Self::PingPong),
            _ => None,
        }
    }

    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    #[must_use]
    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    #[must_use]
    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
            .collect()
    }

    /// Check if looping is enabled.
    #[inline]
    #[must_use]
    pub const fn is_looping(self) -> bool {
        !matches!(self, Self::Off)
    }
}

// ============================================================================
// RELEASE MODE
// ============================================================================

/// Release behavior when note-off is received.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReleaseMode {
    /// Stop immediately on note-off.
    #[default]
    Immediate,
    /// Play to end of sample (ignoring loop) - for drums.
    PlayToEnd,
    /// Play to loop_end, then stop.
    PlayToLoop,
}

impl ReleaseMode {
    pub const ALL: [Self; 3] = [Self::Immediate, Self::PlayToEnd, Self::PlayToLoop];

    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Immediate => "Immediate",
            Self::PlayToEnd => "Play to End",
            Self::PlayToLoop => "Play to Loop",
        }
    }

    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::PlayToEnd => "play_to_end",
            Self::PlayToLoop => "play_to_loop",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "immediate" => Some(Self::Immediate),
            "play_to_end" => Some(Self::PlayToEnd),
            "play_to_loop" => Some(Self::PlayToLoop),
            _ => None,
        }
    }

    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    #[must_use]
    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|m| m == self).unwrap_or(0)
    }

    #[must_use]
    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
            .collect()
    }
}

// ============================================================================
// INTERPOLATION
// ============================================================================

/// Interpolation method for sample playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Interpolation {
    /// No interpolation (nearest sample). Fastest, lowest quality.
    Nearest,
    /// Linear interpolation between adjacent samples. Fast, decent quality.
    Linear,
    /// Cubic interpolation (Catmull-Rom spline). Good quality.
    #[default]
    Cubic,
    /// 4-point Hermite interpolation. Similar to cubic, smoother.
    Hermite,
    /// 4-point Lagrange interpolation. Mathematical precision.
    Lagrange,
    /// 8-tap windowed Sinc (Lanczos). High quality.
    Sinc8,
    /// 16-tap windowed Sinc (Lanczos). Highest quality, slowest.
    Sinc16,
}

impl Interpolation {
    /// All available interpolation modes.
    pub const ALL: [Self; 7] = [
        Self::Nearest,
        Self::Linear,
        Self::Cubic,
        Self::Hermite,
        Self::Lagrange,
        Self::Sinc8,
        Self::Sinc16,
    ];

    /// Get display name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Nearest => "Nearest",
            Self::Linear => "Linear",
            Self::Cubic => "Cubic",
            Self::Hermite => "Hermite",
            Self::Lagrange => "Lagrange",
            Self::Sinc8 => "Sinc 8",
            Self::Sinc16 => "Sinc 16",
        }
    }

    /// Get ID string.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Nearest => "nearest",
            Self::Linear => "linear",
            Self::Cubic => "cubic",
            Self::Hermite => "hermite",
            Self::Lagrange => "lagrange",
            Self::Sinc8 => "sinc8",
            Self::Sinc16 => "sinc16",
        }
    }

    /// Get index of this mode.
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    /// Create from index.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// Generate choice options for GUI.
    #[must_use]
    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| crate::module_traits::ChoiceOption::new(m.id(), m.name()))
            .collect()
    }

    /// Get the number of samples needed before the current position.
    #[inline]
    pub const fn samples_before(self) -> usize {
        match self {
            Self::Nearest | Self::Linear => 0,
            Self::Cubic | Self::Hermite | Self::Lagrange => 1,
            Self::Sinc8 => 3,
            Self::Sinc16 => 7,
        }
    }

    /// Get the number of samples needed after the current position.
    #[inline]
    pub const fn samples_after(self) -> usize {
        match self {
            Self::Nearest => 0,
            Self::Linear => 1,
            Self::Cubic | Self::Hermite | Self::Lagrange => 2,
            Self::Sinc8 => 4,
            Self::Sinc16 => 8,
        }
    }
}

// ============================================================================
// SAMPLE NAME
// ============================================================================

/// A sample name (typically from filename).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SampleName(pub String);

impl SampleName {
    /// Create a new sample name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Get the name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SampleName {
    fn from(name: String) -> Self {
        Self(name)
    }
}

impl From<&str> for SampleName {
    fn from(name: &str) -> Self {
        Self(name.to_string())
    }
}

impl std::fmt::Display for SampleName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ============================================================================
// WAVEFORM OVERVIEW
// ============================================================================

/// Pre-computed waveform overview for visualization.
///
/// Contains min/max peaks for efficient waveform display without
/// reading all sample data every frame.
///
/// Supports both mono and stereo samples:
/// - `peaks_left`: Always populated (min, max) pairs for left channel (or mono)
/// - `peaks_right`: Empty for mono, populated for stereo
#[derive(Debug, Clone)]
pub struct WaveformOverview {
    /// (min, max) pairs for left channel (or mono).
    pub peaks_left: Vec<(SampleValue, SampleValue)>,
    /// (min, max) pairs for right channel (empty if mono).
    pub peaks_right: Vec<(SampleValue, SampleValue)>,
    /// Number of source samples per peak.
    pub samples_per_peak: usize,
    /// Total number of frames in the source sample.
    pub total_frames: usize,
    /// Whether the source sample was stereo.
    pub is_stereo: bool,
}

impl WaveformOverview {
    /// Generate a waveform overview from a sample.
    ///
    /// # Arguments
    /// * `sample` - The source sample
    /// * `num_peaks` - Number of peak segments to generate (typically 100-500)
    #[must_use]
    pub fn generate(sample: &Sample, num_peaks: usize) -> Self {
        let frame_count = sample.len().as_usize();
        let num_peaks = num_peaks.max(1);
        let samples_per_peak = (frame_count / num_peaks).max(1);
        let is_stereo = matches!(sample.channels, ChannelMode::Stereo);

        let mut peaks_left = Vec::with_capacity(num_peaks);
        let mut peaks_right = if is_stereo {
            Vec::with_capacity(num_peaks)
        } else {
            Vec::new()
        };

        for i in 0..num_peaks {
            let start = i * samples_per_peak;
            let end = ((i + 1) * samples_per_peak).min(frame_count);

            if start >= frame_count {
                peaks_left.push((SampleValue::ZERO, SampleValue::ZERO));
                if is_stereo {
                    peaks_right.push((SampleValue::ZERO, SampleValue::ZERO));
                }
                continue;
            }

            let mut min_l = SampleValue::MAX;
            let mut max_l = SampleValue::MIN;
            let mut min_r = SampleValue::MAX;
            let mut max_r = SampleValue::MIN;

            for j in start..end {
                let (l, r) = sample.read(PlaybackPosition::new(j as f64), Interpolation::Nearest);

                if l.as_f32() < min_l.as_f32() {
                    min_l = l;
                }
                if l.as_f32() > max_l.as_f32() {
                    max_l = l;
                }
                if is_stereo {
                    if r.as_f32() < min_r.as_f32() {
                        min_r = r;
                    }
                    if r.as_f32() > max_r.as_f32() {
                        max_r = r;
                    }
                }
            }

            peaks_left.push((min_l, max_l));
            if is_stereo {
                peaks_right.push((min_r, max_r));
            }
        }

        Self {
            peaks_left,
            peaks_right,
            samples_per_peak,
            total_frames: frame_count,
            is_stereo,
        }
    }

    /// Get the left (or mono) peak at a normalized position (0.0 to 1.0).
    #[must_use]
    pub fn peak_left_at(&self, position: f32) -> Option<(SampleValue, SampleValue)> {
        if self.peaks_left.is_empty() {
            return None;
        }
        let index = (position * self.peaks_left.len() as f32) as usize;
        self.peaks_left
            .get(index.min(self.peaks_left.len() - 1))
            .copied()
    }

    /// Get the right peak at a normalized position (0.0 to 1.0).
    /// Returns None for mono samples or if position is out of range.
    #[must_use]
    pub fn peak_right_at(&self, position: f32) -> Option<(SampleValue, SampleValue)> {
        if self.peaks_right.is_empty() {
            return None;
        }
        let index = (position * self.peaks_right.len() as f32) as usize;
        self.peaks_right
            .get(index.min(self.peaks_right.len() - 1))
            .copied()
    }

    /// Get the peak at a normalized position (0.0 to 1.0).
    /// For stereo, returns the left channel peak.
    /// Provided for backwards compatibility.
    #[must_use]
    pub fn peak_at(&self, position: f32) -> Option<(SampleValue, SampleValue)> {
        self.peak_left_at(position)
    }

    /// Get the number of peaks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.peaks_left.len()
    }

    /// Check if the overview is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peaks_left.is_empty()
    }
}

// ============================================================================
// SAMPLE
// ============================================================================

/// Loop metadata for sample playback.
///
/// Loop points are stored as exact sample positions (u32) to avoid
/// precision loss that can cause clicks at loop boundaries.
#[derive(Debug, Clone, Copy, Default)]
pub struct SampleLoopInfo {
    /// Loop start position as exact sample index.
    pub loop_start: u32,
    /// Loop end position as exact sample index.
    pub loop_end: u32,
    /// Whether looping is enabled.
    pub enabled: bool,
    /// Ping-pong (bidirectional) loop.
    pub ping_pong: bool,
}

impl SampleLoopInfo {
    /// Create loop info from normalized positions (0.0-1.0) and sample length.
    ///
    /// Use this for backward compatibility with code that uses normalized values.
    #[must_use]
    pub fn from_normalized(start: f32, end: f32, sample_len: usize, ping_pong: bool) -> Self {
        let start_frame = (start.clamp(0.0, 1.0) * sample_len as f32) as u32;
        let end_frame = (end.clamp(0.0, 1.0) * sample_len as f32).min(sample_len as f32) as u32;
        Self {
            loop_start: start_frame,
            loop_end: end_frame.max(start_frame + 1), // Ensure at least 1 sample loop
            enabled: true,
            ping_pong,
        }
    }

    /// Get loop start as normalized value (0.0-1.0).
    #[must_use]
    pub fn normalized_start(&self, sample_len: usize) -> f32 {
        if sample_len == 0 {
            0.0
        } else {
            self.loop_start as f32 / sample_len as f32
        }
    }

    /// Get loop end as normalized value (0.0-1.0).
    #[must_use]
    pub fn normalized_end(&self, sample_len: usize) -> f32 {
        if sample_len == 0 {
            1.0
        } else {
            self.loop_end as f32 / sample_len as f32
        }
    }

    /// Get loop length in samples.
    #[must_use]
    pub fn length(&self) -> u32 {
        self.loop_end.saturating_sub(self.loop_start)
    }
}

/// An audio sample with metadata.
///
/// Samples are immutable and can be safely shared across threads via `Arc`.
#[derive(Debug, Clone)]
pub struct Sample {
    /// Name of the sample (usually filename).
    pub name: SampleName,
    /// Audio data as sample values.
    pub data: Arc<[SampleValue]>,
    /// Channel configuration.
    pub channels: ChannelMode,
    /// Original sample rate.
    pub sample_rate: SampleRate,
    /// Root note (pitch at normal speed).
    pub root_note: MidiNote,
    /// Loop information.
    pub loop_info: Option<SampleLoopInfo>,
    /// Default volume (0.0-1.0).
    pub default_volume: Option<f32>,
    /// Default panning (0.0=left, 0.5=center, 1.0=right).
    pub default_panning: Option<f32>,
}

impl Sample {
    /// Create a new sample.
    pub fn new(
        name: impl Into<SampleName>,
        data: Vec<SampleValue>,
        channels: ChannelMode,
        sample_rate: SampleRate,
    ) -> Self {
        Self {
            name: name.into(),
            data: data.into(),
            channels,
            sample_rate,
            root_note: MidiNote::C4,
            loop_info: None,
            default_volume: None,
            default_panning: None,
        }
    }

    /// Create a new sample with a specific root note.
    pub fn with_root_note(mut self, root_note: MidiNote) -> Self {
        self.root_note = root_note;
        self
    }

    /// Set loop information.
    pub fn with_loop_info(mut self, loop_info: SampleLoopInfo) -> Self {
        self.loop_info = Some(loop_info);
        self
    }

    /// Set default volume (0.0-1.0).
    pub fn with_default_volume(mut self, volume: f32) -> Self {
        self.default_volume = Some(volume.clamp(0.0, 1.0));
        self
    }

    /// Set default panning (0.0=left, 0.5=center, 1.0=right).
    pub fn with_default_panning(mut self, panning: f32) -> Self {
        self.default_panning = Some(panning.clamp(0.0, 1.0));
        self
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

    /// Get the duration in seconds.
    #[inline]
    pub fn duration_seconds(&self) -> f32 {
        self.len().as_usize() as f32 / self.sample_rate.as_f32()
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

    /// Read a stereo sample with loop-aware interpolation.
    ///
    /// When interpolation needs samples beyond loop boundaries, this method
    /// wraps indices back into the loop region instead of clamping to sample
    /// boundaries. This eliminates clicks at loop points caused by interpolation
    /// reading incorrect samples.
    ///
    /// For mono samples, the value is duplicated to both channels.
    /// Returns (left, right) sample values.
    pub fn read_looped(
        &self,
        position: PlaybackPosition,
        interp: Interpolation,
        loop_start: usize,
        loop_end: usize,
    ) -> (SampleValue, SampleValue) {
        if self.is_empty() {
            return (SampleValue::ZERO, SampleValue::ZERO);
        }

        let frame_count = self.len().0;
        if frame_count == 0 {
            return (SampleValue::ZERO, SampleValue::ZERO);
        }

        // Clamp loop bounds to valid range
        let loop_start = loop_start.min(frame_count.saturating_sub(1));
        let loop_end = loop_end.min(frame_count).max(loop_start + 1);

        match self.channels {
            ChannelMode::Mono => {
                let value =
                    self.read_mono_looped(position, interp, frame_count, loop_start, loop_end);
                (value, value)
            }
            ChannelMode::Stereo => {
                self.read_stereo_looped(position, interp, frame_count, loop_start, loop_end)
            }
        }
    }

    /// Read a mono sample with loop-aware interpolation.
    fn read_mono_looped(
        &self,
        position: PlaybackPosition,
        interp: Interpolation,
        len: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> SampleValue {
        let pos = position.0;
        let idx = pos as usize;

        if idx >= len {
            return SampleValue::ZERO;
        }

        // For simple interpolation modes, use standard read
        match interp {
            Interpolation::Nearest => self.data[idx],
            Interpolation::Linear => {
                let frac = position.fraction();
                let s0 = self.data[idx];
                let s1 = self.get_looped_sample_mono(idx + 1, len, loop_start, loop_end);
                s0.lerp(s1, frac)
            }
            Interpolation::Cubic => {
                self.cubic_interp_mono_looped(idx, position.fraction(), len, loop_start, loop_end)
            }
            Interpolation::Hermite => {
                self.hermite_interp_mono_looped(idx, position.fraction(), len, loop_start, loop_end)
            }
            Interpolation::Lagrange => self.lagrange_interp_mono_looped(
                idx,
                position.fraction(),
                len,
                loop_start,
                loop_end,
            ),
            Interpolation::Sinc8 => {
                self.sinc_interp_mono_looped(idx, position.fraction(), len, loop_start, loop_end, 4)
            }
            Interpolation::Sinc16 => {
                self.sinc_interp_mono_looped(idx, position.fraction(), len, loop_start, loop_end, 8)
            }
        }
    }

    /// Read a stereo sample with loop-aware interpolation.
    fn read_stereo_looped(
        &self,
        position: PlaybackPosition,
        interp: Interpolation,
        frame_count: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> (SampleValue, SampleValue) {
        let pos = position.0;
        let idx = pos as usize;

        if idx >= frame_count {
            return (SampleValue::ZERO, SampleValue::ZERO);
        }

        match interp {
            Interpolation::Nearest => {
                let base = idx * 2;
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
                let (l0, r0) =
                    self.get_looped_sample_stereo(idx, frame_count, loop_start, loop_end);
                let (l1, r1) =
                    self.get_looped_sample_stereo(idx + 1, frame_count, loop_start, loop_end);
                (l0.lerp(l1, frac), r0.lerp(r1, frac))
            }
            Interpolation::Cubic => self.cubic_interp_stereo_looped(
                idx,
                position.fraction(),
                frame_count,
                loop_start,
                loop_end,
            ),
            Interpolation::Hermite => self.hermite_interp_stereo_looped(
                idx,
                position.fraction(),
                frame_count,
                loop_start,
                loop_end,
            ),
            Interpolation::Lagrange => self.lagrange_interp_stereo_looped(
                idx,
                position.fraction(),
                frame_count,
                loop_start,
                loop_end,
            ),
            Interpolation::Sinc8 => self.sinc_interp_stereo_looped(
                idx,
                position.fraction(),
                frame_count,
                loop_start,
                loop_end,
                4,
            ),
            Interpolation::Sinc16 => self.sinc_interp_stereo_looped(
                idx,
                position.fraction(),
                frame_count,
                loop_start,
                loop_end,
                8,
            ),
        }
    }

    /// Get a mono sample with loop wrapping.
    #[inline]
    fn get_looped_sample_mono(
        &self,
        idx: usize,
        len: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> SampleValue {
        let wrapped_idx = if idx >= loop_end {
            // Wrap back into loop region
            let loop_len = loop_end.saturating_sub(loop_start).max(1);
            loop_start + (idx - loop_end) % loop_len
        } else if idx < loop_start && idx >= len {
            // Before loop start and out of bounds - clamp
            loop_start
        } else {
            idx.min(len.saturating_sub(1))
        };
        self.data
            .get(wrapped_idx)
            .copied()
            .unwrap_or(SampleValue::ZERO)
    }

    /// Get a stereo sample with loop wrapping.
    #[inline]
    fn get_looped_sample_stereo(
        &self,
        idx: usize,
        frame_count: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> (SampleValue, SampleValue) {
        let wrapped_idx = if idx >= loop_end {
            // Wrap back into loop region
            let loop_len = loop_end.saturating_sub(loop_start).max(1);
            loop_start + (idx - loop_end) % loop_len
        } else if idx >= frame_count {
            // Out of bounds - wrap to loop start
            loop_start
        } else {
            idx
        };
        let base = wrapped_idx * 2;
        let left = self.data.get(base).copied().unwrap_or(SampleValue::ZERO);
        let right = self
            .data
            .get(base + 1)
            .copied()
            .unwrap_or(SampleValue::ZERO);
        (left, right)
    }

    /// Cubic interpolation with loop wrapping for mono samples.
    fn cubic_interp_mono_looped(
        &self,
        idx: usize,
        frac: f32,
        len: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> SampleValue {
        let get = |offset: isize| -> f32 {
            let i = (idx as isize + offset).max(0) as usize;
            self.get_looped_sample_mono(i, len, loop_start, loop_end).0
        };

        let y0 = get(-1);
        let y1 = get(0);
        let y2 = get(1);
        let y3 = get(2);

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

    /// Cubic interpolation with loop wrapping for stereo samples.
    fn cubic_interp_stereo_looped(
        &self,
        idx: usize,
        frac: f32,
        frame_count: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> (SampleValue, SampleValue) {
        let get = |offset: isize, ch: usize| -> f32 {
            let i = (idx as isize + offset).max(0) as usize;
            let (l, r) = self.get_looped_sample_stereo(i, frame_count, loop_start, loop_end);
            if ch == 0 { l.0 } else { r.0 }
        };

        let t = frac;
        let t2 = t * t;
        let t3 = t2 * t;

        let mut result = [0.0f32; 2];

        for ch in 0..2 {
            let y0 = get(-1, ch);
            let y1 = get(0, ch);
            let y2 = get(1, ch);
            let y3 = get(2, ch);

            // Catmull-Rom spline
            let a0 = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
            let a1 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let a2 = -0.5 * y0 + 0.5 * y2;
            let a3 = y1;

            result[ch] = a0 * t3 + a1 * t2 + a2 * t + a3;
        }

        (SampleValue(result[0]), SampleValue(result[1]))
    }

    /// Hermite interpolation with loop wrapping for mono samples.
    fn hermite_interp_mono_looped(
        &self,
        idx: usize,
        frac: f32,
        len: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> SampleValue {
        let get = |offset: isize| -> f32 {
            let i = (idx as isize + offset).max(0) as usize;
            self.get_looped_sample_mono(i, len, loop_start, loop_end).0
        };

        let y0 = get(-1);
        let y1 = get(0);
        let y2 = get(1);
        let y3 = get(2);

        let t = frac;
        let t2 = t * t;
        let t3 = t2 * t;

        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);

        SampleValue(c0 + c1 * t + c2 * t2 + c3 * t3)
    }

    /// Hermite interpolation with loop wrapping for stereo samples.
    fn hermite_interp_stereo_looped(
        &self,
        idx: usize,
        frac: f32,
        frame_count: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> (SampleValue, SampleValue) {
        let get = |offset: isize, ch: usize| -> f32 {
            let i = (idx as isize + offset).max(0) as usize;
            let (l, r) = self.get_looped_sample_stereo(i, frame_count, loop_start, loop_end);
            if ch == 0 { l.0 } else { r.0 }
        };

        let t = frac;
        let t2 = t * t;
        let t3 = t2 * t;

        let mut result = [0.0f32; 2];

        for ch in 0..2 {
            let y0 = get(-1, ch);
            let y1 = get(0, ch);
            let y2 = get(1, ch);
            let y3 = get(2, ch);

            let c0 = y1;
            let c1 = 0.5 * (y2 - y0);
            let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);

            result[ch] = c0 + c1 * t + c2 * t2 + c3 * t3;
        }

        (SampleValue(result[0]), SampleValue(result[1]))
    }

    /// Lagrange interpolation with loop wrapping for mono samples.
    fn lagrange_interp_mono_looped(
        &self,
        idx: usize,
        frac: f32,
        len: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> SampleValue {
        let get = |offset: isize| -> f32 {
            let i = (idx as isize + offset).max(0) as usize;
            self.get_looped_sample_mono(i, len, loop_start, loop_end).0
        };

        let t = frac;
        let y0 = get(-1);
        let y1 = get(0);
        let y2 = get(1);
        let y3 = get(2);

        let l0 = (t * (t - 1.0) * (t - 2.0)) / -6.0;
        let l1 = ((t + 1.0) * (t - 1.0) * (t - 2.0)) / 2.0;
        let l2 = ((t + 1.0) * t * (t - 2.0)) / -2.0;
        let l3 = ((t + 1.0) * t * (t - 1.0)) / 6.0;

        SampleValue(y0 * l0 + y1 * l1 + y2 * l2 + y3 * l3)
    }

    /// Lagrange interpolation with loop wrapping for stereo samples.
    fn lagrange_interp_stereo_looped(
        &self,
        idx: usize,
        frac: f32,
        frame_count: usize,
        loop_start: usize,
        loop_end: usize,
    ) -> (SampleValue, SampleValue) {
        let get = |offset: isize, ch: usize| -> f32 {
            let i = (idx as isize + offset).max(0) as usize;
            let (l, r) = self.get_looped_sample_stereo(i, frame_count, loop_start, loop_end);
            if ch == 0 { l.0 } else { r.0 }
        };

        let t = frac;
        let l0 = (t * (t - 1.0) * (t - 2.0)) / -6.0;
        let l1 = ((t + 1.0) * (t - 1.0) * (t - 2.0)) / 2.0;
        let l2 = ((t + 1.0) * t * (t - 2.0)) / -2.0;
        let l3 = ((t + 1.0) * t * (t - 1.0)) / 6.0;

        let mut result = [0.0f32; 2];

        for ch in 0..2 {
            let y0 = get(-1, ch);
            let y1 = get(0, ch);
            let y2 = get(1, ch);
            let y3 = get(2, ch);

            result[ch] = y0 * l0 + y1 * l1 + y2 * l2 + y3 * l3;
        }

        (SampleValue(result[0]), SampleValue(result[1]))
    }

    /// Sinc interpolation with loop wrapping for mono samples.
    fn sinc_interp_mono_looped(
        &self,
        idx: usize,
        frac: f32,
        len: usize,
        loop_start: usize,
        loop_end: usize,
        half_taps: usize,
    ) -> SampleValue {
        let get = |offset: isize| -> f32 {
            let i = (idx as isize + offset).max(0) as usize;
            self.get_looped_sample_mono(i, len, loop_start, loop_end).0
        };

        let mut sum = 0.0f32;
        let mut weight_sum = 0.0f32;

        let taps = half_taps as isize;
        for i in -taps + 1..=taps {
            let x = frac - i as f32;
            let weight = lanczos_kernel(x, half_taps as f32);
            sum += get(i) * weight;
            weight_sum += weight;
        }

        if weight_sum.abs() > 1e-10 {
            SampleValue(sum / weight_sum)
        } else {
            SampleValue(get(0))
        }
    }

    /// Sinc interpolation with loop wrapping for stereo samples.
    fn sinc_interp_stereo_looped(
        &self,
        idx: usize,
        frac: f32,
        frame_count: usize,
        loop_start: usize,
        loop_end: usize,
        half_taps: usize,
    ) -> (SampleValue, SampleValue) {
        let get = |offset: isize, ch: usize| -> f32 {
            let i = (idx as isize + offset).max(0) as usize;
            let (l, r) = self.get_looped_sample_stereo(i, frame_count, loop_start, loop_end);
            if ch == 0 { l.0 } else { r.0 }
        };

        let mut sum = [0.0f32; 2];
        let mut weight_sum = 0.0f32;

        let taps = half_taps as isize;
        for i in -taps + 1..=taps {
            let x = frac - i as f32;
            let weight = lanczos_kernel(x, half_taps as f32);
            for ch in 0..2 {
                sum[ch] += get(i, ch) * weight;
            }
            weight_sum += weight;
        }

        if weight_sum.abs() > 1e-10 {
            (
                SampleValue(sum[0] / weight_sum),
                SampleValue(sum[1] / weight_sum),
            )
        } else {
            (SampleValue(get(0, 0)), SampleValue(get(0, 1)))
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
            Interpolation::Hermite => self.hermite_interp_mono(idx, position.fraction(), len),
            Interpolation::Lagrange => self.lagrange_interp_mono(idx, position.fraction(), len),
            Interpolation::Sinc8 => self.sinc_interp_mono(idx, position.fraction(), len, 4),
            Interpolation::Sinc16 => self.sinc_interp_mono(idx, position.fraction(), len, 8),
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
            Interpolation::Hermite => {
                self.hermite_interp_stereo(idx, position.fraction(), frame_count)
            }
            Interpolation::Lagrange => {
                self.lagrange_interp_stereo(idx, position.fraction(), frame_count)
            }
            Interpolation::Sinc8 => {
                self.sinc_interp_stereo(idx, position.fraction(), frame_count, 4)
            }
            Interpolation::Sinc16 => {
                self.sinc_interp_stereo(idx, position.fraction(), frame_count, 8)
            }
        }
    }

    /// Cubic interpolation (Catmull-Rom) for mono samples.
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

    /// Cubic interpolation (Catmull-Rom) for stereo samples.
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

    /// Hermite interpolation for mono samples.
    fn hermite_interp_mono(&self, idx: usize, frac: f32, len: usize) -> SampleValue {
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

        // Hermite interpolation coefficients
        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);

        SampleValue(c0 + c1 * t + c2 * t2 + c3 * t3)
    }

    /// Hermite interpolation for stereo samples.
    fn hermite_interp_stereo(
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

            let c0 = y1;
            let c1 = 0.5 * (y2 - y0);
            let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
            let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);

            result[ch] = c0 + c1 * t + c2 * t2 + c3 * t3;
        }

        (SampleValue(result[0]), SampleValue(result[1]))
    }

    /// Lagrange interpolation for mono samples.
    fn lagrange_interp_mono(&self, idx: usize, frac: f32, len: usize) -> SampleValue {
        let get = |i: isize| -> f32 {
            let clamped = i.clamp(0, (len - 1) as isize) as usize;
            self.data[clamped].0
        };

        let idx = idx as isize;
        let t = frac;

        let y0 = get(idx - 1);
        let y1 = get(idx);
        let y2 = get(idx + 1);
        let y3 = get(idx + 2);

        // 4-point Lagrange interpolation at points -1, 0, 1, 2
        let l0 = (t * (t - 1.0) * (t - 2.0)) / -6.0;
        let l1 = ((t + 1.0) * (t - 1.0) * (t - 2.0)) / 2.0;
        let l2 = ((t + 1.0) * t * (t - 2.0)) / -2.0;
        let l3 = ((t + 1.0) * t * (t - 1.0)) / 6.0;

        SampleValue(y0 * l0 + y1 * l1 + y2 * l2 + y3 * l3)
    }

    /// Lagrange interpolation for stereo samples.
    fn lagrange_interp_stereo(
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

        let l0 = (t * (t - 1.0) * (t - 2.0)) / -6.0;
        let l1 = ((t + 1.0) * (t - 1.0) * (t - 2.0)) / 2.0;
        let l2 = ((t + 1.0) * t * (t - 2.0)) / -2.0;
        let l3 = ((t + 1.0) * t * (t - 1.0)) / 6.0;

        let mut result = [0.0f32; 2];

        for ch in 0..2 {
            let y0 = get(idx - 1, ch);
            let y1 = get(idx, ch);
            let y2 = get(idx + 1, ch);
            let y3 = get(idx + 2, ch);

            result[ch] = y0 * l0 + y1 * l1 + y2 * l2 + y3 * l3;
        }

        (SampleValue(result[0]), SampleValue(result[1]))
    }

    /// Sinc interpolation with Lanczos window for mono samples.
    fn sinc_interp_mono(&self, idx: usize, frac: f32, len: usize, half_taps: usize) -> SampleValue {
        let get = |i: isize| -> f32 {
            let clamped = i.clamp(0, (len - 1) as isize) as usize;
            self.data[clamped].0
        };

        let idx = idx as isize;
        let mut sum = 0.0f32;
        let mut weight_sum = 0.0f32;

        let taps = half_taps as isize;
        for i in -taps + 1..=taps {
            let x = frac - i as f32;
            let weight = lanczos_kernel(x, half_taps as f32);
            sum += get(idx + i) * weight;
            weight_sum += weight;
        }

        if weight_sum.abs() > 1e-10 {
            SampleValue(sum / weight_sum)
        } else {
            SampleValue(get(idx))
        }
    }

    /// Sinc interpolation with Lanczos window for stereo samples.
    fn sinc_interp_stereo(
        &self,
        idx: usize,
        frac: f32,
        frame_count: usize,
        half_taps: usize,
    ) -> (SampleValue, SampleValue) {
        let get = |i: isize, ch: usize| -> f32 {
            let clamped = i.clamp(0, (frame_count - 1) as isize) as usize;
            let data_idx = clamped * 2 + ch;
            self.data.get(data_idx).map(|v| v.0).unwrap_or(0.0)
        };

        let idx = idx as isize;
        let mut sum = [0.0f32; 2];
        let mut weight_sum = 0.0f32;

        let taps = half_taps as isize;
        for i in -taps + 1..=taps {
            let x = frac - i as f32;
            let weight = lanczos_kernel(x, half_taps as f32);
            for ch in 0..2 {
                sum[ch] += get(idx + i, ch) * weight;
            }
            weight_sum += weight;
        }

        if weight_sum.abs() > 1e-10 {
            (
                SampleValue(sum[0] / weight_sum),
                SampleValue(sum[1] / weight_sum),
            )
        } else {
            (SampleValue(get(idx, 0)), SampleValue(get(idx, 1)))
        }
    }

    /// Calculate the pitch ratio to play at a given note.
    ///
    /// Returns the speed multiplier needed to transpose from root_note to target_note.
    pub fn pitch_ratio(&self, target_note: MidiNote) -> f32 {
        let semitone_diff = target_note.0 as f32 - self.root_note.0 as f32;
        2.0f32.powf(semitone_diff / 12.0)
    }
}

/// Lanczos kernel for sinc interpolation.
#[inline]
fn lanczos_kernel(x: f32, a: f32) -> f32 {
    if x.abs() < 1e-10 {
        1.0
    } else if x.abs() >= a {
        0.0
    } else {
        let pi_x = std::f32::consts::PI * x;
        let pi_x_a = pi_x / a;
        (pi_x.sin() / pi_x) * (pi_x_a.sin() / pi_x_a)
    }
}

// ============================================================================
// TESTS
// ============================================================================

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
    fn test_sample_value_add() {
        let a = SampleValue::new(0.3);
        let b = SampleValue::new(0.5);
        let sum = a + b;
        assert!((sum.0 - 0.8).abs() < 0.001);
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
    fn test_playback_state() {
        let state = PlaybackState::Stopped;
        assert!(state.is_stopped());
        assert!(!state.is_playing());

        let state = PlaybackState::Playing;
        assert!(state.is_playing());
        assert!(!state.is_stopped());
    }

    #[test]
    fn test_sample_name() {
        let name = SampleName::new("test_sample");
        assert_eq!(name.as_str(), "test_sample");
        assert_eq!(format!("{name}"), "test_sample");
    }

    #[test]
    fn test_sample_read_mono() {
        let data: Vec<SampleValue> = vec![
            SampleValue::new(0.0),
            SampleValue::new(0.5),
            SampleValue::new(1.0),
            SampleValue::new(0.5),
        ];
        let sample = Sample::new("test", data, ChannelMode::Mono, SampleRate::CD_QUALITY);

        let (l, r) = sample.read(PlaybackPosition::new(1.0), Interpolation::Nearest);
        assert!((l.0 - 0.5).abs() < 0.001);
        assert_eq!(l, r); // Mono should duplicate
    }

    #[test]
    fn test_sample_pitch_ratio() {
        let sample = Sample::new(
            "test",
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

    #[test]
    fn test_waveform_overview() {
        let data: Vec<SampleValue> = (0..1000)
            .map(|i| SampleValue::new((i as f32 * 0.01).sin()))
            .collect();
        let sample = Sample::new("test", data, ChannelMode::Mono, SampleRate::CD_QUALITY);

        let overview = WaveformOverview::generate(&sample, 100);
        assert_eq!(overview.len(), 100);
        assert!(!overview.is_empty());
        assert_eq!(overview.total_frames, 1000);

        // Check that we can get peaks
        let peak = overview.peak_at(0.5);
        assert!(peak.is_some());
    }

    #[test]
    fn test_interpolation_samples() {
        assert_eq!(Interpolation::Nearest.samples_before(), 0);
        assert_eq!(Interpolation::Linear.samples_after(), 1);
        assert_eq!(Interpolation::Sinc16.samples_before(), 7);
        assert_eq!(Interpolation::Sinc16.samples_after(), 8);
    }
}
