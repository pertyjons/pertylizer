//! Core types for sample handling.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use synth_core::audio::SampleRate;
use synth_core::{ChannelCount, MidiNote, SampleCount};

/// Unique identifier for a sample in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct SampleId(pub u64);

impl SampleId {
    /// Create a new sample ID.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for SampleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sample:{}", self.0)
    }
}

/// Frame index within a sample buffer (absolute position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
#[must_use]
pub struct FrameIndex(pub usize);

impl FrameIndex {
    /// Create a new frame index.
    #[inline]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Start of sample.
    pub const ZERO: Self = Self(0);

    /// Get the raw value.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl std::fmt::Display for FrameIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame:{}", self.0)
    }
}

/// Fractional playback position within a sample (sub-frame precision).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct PlaybackPosition(pub f64);

impl PlaybackPosition {
    /// Create a new playback position.
    #[inline]
    pub const fn new(pos: f64) -> Self {
        Self(pos)
    }

    /// Start of sample.
    pub const ZERO: Self = Self(0.0);

    /// Get the integer frame index.
    #[inline]
    pub fn frame_index(self) -> usize {
        self.0 as usize
    }

    /// Get the fractional part for interpolation.
    #[inline]
    pub fn fraction(self) -> f64 {
        self.0.fract()
    }

    /// Advance by a speed amount.
    #[inline]
    pub fn advance(self, speed: PlaybackSpeed) -> Self {
        Self(self.0 + speed.0)
    }
}

/// Playback speed ratio (1.0 = original pitch, 2.0 = one octave up).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct PlaybackSpeed(pub f64);

impl PlaybackSpeed {
    /// Original speed (no pitch shift).
    pub const ORIGINAL: Self = Self(1.0);

    /// Calculate speed from MIDI note offset.
    /// `speed = 2^((target_note - root_note) / 12.0)`
    #[inline]
    pub fn from_note_offset(target: MidiNote, root: MidiNote) -> Self {
        let semitones = f64::from(target.0) - f64::from(root.0);
        Self(2.0_f64.powf(semitones / 12.0))
    }
}

/// How a sample was created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SampleSource {
    /// Recorded from audio input.
    Recorded,
    /// Imported from a WAV file.
    Imported { original_path: Option<PathBuf> },
    /// Generated programmatically.
    Generated,
}

/// Loop region within a sample.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LoopRegion {
    /// Loop start frame.
    pub start: FrameIndex,
    /// Loop end frame (exclusive).
    pub end: FrameIndex,
    /// Crossfade length in samples to avoid clicks.
    // Reserved for future crossfade implementation
    #[allow(dead_code)]
    pub crossfade: SampleCount,
}

/// Crop region — the audible portion of the full sample buffer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CropRegion {
    /// Crop start frame.
    pub start: FrameIndex,
    /// Crop end frame (exclusive).
    pub end: FrameIndex,
}

impl CropRegion {
    /// Length in frames.
    #[inline]
    pub fn len(self) -> usize {
        self.end.0.saturating_sub(self.start.0)
    }

    /// Whether the region is empty.
    #[inline]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// Playback mode for the sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlayMode {
    /// Play once, then stop.
    #[default]
    OneShot,
    /// Loop within the loop region.
    Loop,
    /// Play forward then backward, repeating.
    PingPong,
}

/// WAV bit depth for export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    /// 16-bit integer PCM.
    Int16,
    /// 24-bit integer PCM.
    Int24,
    /// 32-bit float.
    Float32,
}

/// A sample's metadata (does NOT contain audio data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleMeta {
    pub id: SampleId,
    pub name: String,
    /// Free-text description capturing intent (what the sample is for). Empty
    /// by default; readable/writable via MCP and GUI.
    #[serde(default)]
    pub description: String,
    pub sample_rate: SampleRate,
    pub channels: ChannelCount,
    pub frame_count: SampleCount,
    pub root_note: Option<MidiNote>,
    pub loop_region: Option<LoopRegion>,
    pub crop: Option<CropRegion>,
    pub source: SampleSource,
}

impl SampleMeta {
    /// Duration in seconds.
    #[inline]
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate.0 == 0 {
            return 0.0;
        }
        self.frame_count.as_usize() as f64 / f64::from(self.sample_rate.0)
    }
}
