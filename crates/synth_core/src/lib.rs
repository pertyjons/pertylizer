//! Core types, traits, and audio abstractions for Pertylizer.
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
pub mod hash;
pub mod module_traits;
pub mod params;
pub mod script;
pub mod tuning;
pub mod types;

// Re-export all types at crate root for convenience
pub use audio::{
    AudioBackend, AudioCallbackContext, AudioError, AudioHost, AudioHostTrait, AudioProcessor,
    AudioResult, AudioStream, BufferSize, ChannelCount, DeviceInfo, DeviceType, MAX_BLOCK_SIZE,
    StreamConfig, StreamInfo,
};
pub use module_traits::*;
pub use params::*;
pub use types::*;

/// Convenience re-exports for code that works with parameters generically.
///
/// `use synth_core::prelude::*;` brings the parameter traits and the value-kind
/// classifier into scope so generic `T: ModuleParam` / `T: ScalarParam` code reads
/// cleanly. (Everything here is also reachable directly at the crate root.)
pub mod prelude {
    pub use crate::module_traits::{
        ModuleParam, ParamKind, ParameterUnit, ResponseCurve, ScalarParam,
    };
    pub use crate::params::Param;
}
