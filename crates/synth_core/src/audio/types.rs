//! Core types for the audio abstraction layer.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// Sample rate in Hz for audio backend configuration.
///
/// This type uses `u32` to match hardware API conventions.
/// For DSP calculations, use `crate::types::SampleRate` (f32) which
/// provides methods like `nyquist()`, `period()`, and `samples_for()`.
///
/// Conversions between the two types are provided via `From` impls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SampleRate(pub u32);

impl SampleRate {
    pub const CD_QUALITY: Self = Self(44100);
    pub const DVD_QUALITY: Self = Self(48000);
    pub const STUDIO_QUALITY: Self = Self(96000);

    #[inline]
    pub fn as_f32(self) -> f32 {
        self.0 as f32
    }

    #[inline]
    pub fn as_f64(self) -> f64 {
        self.0 as f64
    }

    /// Convert samples to duration.
    #[inline]
    pub fn samples_to_duration(self, samples: u64) -> Duration {
        Duration::from_secs_f64(samples as f64 / self.as_f64())
    }

    /// Convert duration to samples.
    #[inline]
    pub fn duration_to_samples(self, duration: Duration) -> u64 {
        (duration.as_secs_f64() * self.as_f64()) as u64
    }
}

impl Default for SampleRate {
    fn default() -> Self {
        Self::DVD_QUALITY
    }
}

impl From<u32> for SampleRate {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<crate::types::SampleRate> for SampleRate {
    fn from(rate: crate::types::SampleRate) -> Self {
        Self(rate.as_f32() as u32)
    }
}

impl From<SampleRate> for crate::types::SampleRate {
    fn from(rate: SampleRate) -> Self {
        Self::new(rate.0 as f32)
    }
}

/// Buffer size in frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BufferSize(pub u32);

impl BufferSize {
    pub const TINY: Self = Self(64);
    pub const SMALL: Self = Self(128);
    pub const MEDIUM: Self = Self(256);
    pub const LARGE: Self = Self(512);
    pub const VERY_LARGE: Self = Self(1024);

    /// Calculate latency for this buffer size at given sample rate.
    #[inline]
    pub fn latency_at(self, sample_rate: SampleRate) -> Duration {
        Duration::from_secs_f64(self.0 as f64 / sample_rate.as_f64())
    }
}

impl Default for BufferSize {
    fn default() -> Self {
        Self::MEDIUM
    }
}

impl From<u32> for BufferSize {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

/// Number of audio channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ChannelCount {
    Mono,
    #[default]
    Stereo,
    Multi(u16),
}

impl ChannelCount {
    #[inline]
    pub fn count(self) -> u16 {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Multi(n) => n,
        }
    }
}

impl From<u16> for ChannelCount {
    fn from(value: u16) -> Self {
        match value {
            1 => Self::Mono,
            2 => Self::Stereo,
            n => Self::Multi(n),
        }
    }
}

/// Type of audio device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceType {
    Input,
    Output,
    Duplex,
}

/// Information about an audio device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Unique identifier for the device.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Type of device.
    pub device_type: DeviceType,
    /// Supported sample rates.
    pub supported_sample_rates: Vec<SampleRate>,
    /// Minimum buffer size.
    pub min_buffer_size: BufferSize,
    /// Maximum buffer size.
    pub max_buffer_size: BufferSize,
    /// Number of input channels.
    pub input_channels: ChannelCount,
    /// Number of output channels.
    pub output_channels: ChannelCount,
}

/// Stream configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StreamConfig {
    pub sample_rate: SampleRate,
    pub buffer_size: BufferSize,
    pub channels: ChannelCount,
}

/// Information about an active audio stream.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    /// Actual sample rate (may differ from requested).
    pub sample_rate: SampleRate,
    /// Actual buffer size.
    pub buffer_size: BufferSize,
    /// Number of channels.
    pub channels: ChannelCount,
    /// Estimated output latency.
    pub output_latency: Duration,
    /// Estimated input latency (if applicable).
    pub input_latency: Option<Duration>,
}

/// Context passed to audio callback.
#[derive(Debug, Clone, Copy)]
pub struct AudioCallbackContext {
    /// Sample rate of the stream.
    pub sample_rate: SampleRate,
    /// Number of frames in this callback.
    pub frames: usize,
    /// Number of channels.
    pub channels: u16,
    /// Time since stream started (in seconds).
    pub stream_time: f64,
    /// Total samples processed since stream started.
    pub sample_position: u64,
    /// Estimated output latency (in seconds).
    pub output_latency: f32,
}

/// Errors that can occur in the audio system.
#[derive(Debug, Error)]
pub enum AudioError {
    #[error("No audio backend available")]
    NoBackendAvailable,

    #[error("No default device available")]
    NoDefaultDevice,

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Unsupported configuration: {0}")]
    UnsupportedConfig(String),

    #[error("Stream creation failed: {0}")]
    StreamCreationFailed(String),

    #[error("Stream already running")]
    StreamAlreadyRunning,

    #[error("Stream not running")]
    StreamNotRunning,

    #[error("Backend error: {0}")]
    BackendError(String),

    #[error("Device disconnected")]
    DeviceDisconnected,

    #[error("Buffer underrun")]
    BufferUnderrun,

    #[error("Buffer overrun")]
    BufferOverrun,
}

/// Result type for audio operations.
pub type AudioResult<T> = Result<T, AudioError>;
