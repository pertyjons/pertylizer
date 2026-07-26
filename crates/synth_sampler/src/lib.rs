#![forbid(unsafe_code)]

//! Sample loading, playback, and manipulation for Pertylizer.
//!
//! This crate provides:
//! - Core sample types (`SampleId`, `SampleMeta`, `Sample`)
//! - WAV file I/O (`load_wav`, `save_wav`)
//! - Sample library for managing loaded samples
//! - Sample rate conversion (linear interpolation)

mod error;
mod library;
mod playback;
mod sample;
mod types;
mod wav;

pub use error::SampleError;
pub use library::SampleLibrary;
pub use playback::{PlaybackState, SamplePlayer};
pub use sample::Sample;
pub use types::{
    BitDepth, CropRegion, FrameIndex, LoopRegion, PlayMode, PlaybackPosition, PlaybackSpeed,
    SampleId, SampleMeta, SampleSource,
};
pub use wav::{load_wav, save_wav};
