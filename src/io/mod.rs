//! I/O operations for the synthesizer.
//!
//! This module provides:
//! - File system operations for patches and settings
//! - MIDI input handling via midir
//! - Sample loading and caching
//! - Song import from tracker files (MOD, XM, S3M, IT)

pub mod import;
mod midi;
mod patch_manager;
mod sample_manager;

pub use midi::{MidiError, MidiHandler};
pub use patch_manager::*;
pub use sample_manager::{SampleError, SampleManager, SampleResult};
