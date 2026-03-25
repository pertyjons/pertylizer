//! Sample loading, playback, and manipulation for Pertylizer.
//!
//! This crate provides:
//! - Core sample types (`SampleId`, `SampleMeta`, `Sample`)
//! - WAV file I/O (`load_wav`, `save_wav`)
//! - Sample library for managing loaded samples
//! - Sample rate conversion (linear interpolation)

pub mod error;
pub mod library;
pub mod playback;
pub mod sample;
pub mod types;
pub mod wav;

pub use error::SampleError;
pub use library::SampleLibrary;
pub use playback::SamplePlayer;
pub use sample::Sample;
pub use types::*;
pub use wav::{load_wav, save_wav};
