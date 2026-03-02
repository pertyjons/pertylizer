//! Core types for the audio abstraction layer.
//!
//! Re-exports from synth_core for use by the audio backends.

pub use synth_core::{
    AudioCallbackContext, AudioError, AudioResult, BufferSize, ChannelCount, DeviceInfo,
    DeviceType, StreamConfig, StreamInfo,
};

// Note: This module also re-exports SampleRate from synth_core::audio,
// which is the u32-based version for hardware API compatibility.
// For DSP calculations, use synth_core::SampleRate (f32-based).
pub use synth_core::audio::SampleRate;
