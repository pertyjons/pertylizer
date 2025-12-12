//! Core types, traits, and audio abstractions for modular synthesizer.
//!
//! This crate provides the foundational types used throughout the synthesizer:
//! - Type-safe audio domain types (Hertz, Gain, etc.)
//! - Module traits (PolyModule, AudioEffect, Describable)
//! - Audio processing traits (AudioProcessor, AudioBackend)

// Crate-level clippy allows for synth-appropriate exceptions
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::similar_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::suboptimal_flops)]
#![allow(clippy::float_cmp)]

pub mod audio;
pub mod module_traits;
pub mod params;
pub mod types;

// Re-export all types at crate root for convenience
pub use audio::{
    AudioBackend, AudioCallbackContext, AudioError, AudioHost, AudioHostTrait, AudioProcessor,
    AudioResult, AudioStream, BufferSize, ChannelCount, DeviceInfo, DeviceType, StreamConfig,
    StreamInfo,
};
pub use module_traits::*;
pub use params::*;
pub use types::*;
