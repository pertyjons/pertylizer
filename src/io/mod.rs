//! I/O operations for the synthesizer.
//!
//! This module provides:
//! - File system operations for patches and settings
//! - MIDI input handling via midir

mod midi;
mod patch_manager;

pub use midi::{MidiError, MidiHandler};
pub use patch_manager::*;
