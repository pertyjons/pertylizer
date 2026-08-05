//! I/O operations for the synthesizer.
//!
//! This module provides:
//! - File system operations for patches and settings
//! - MIDI input handling via midir

pub mod atomic;
mod group_template_manager;
mod midi;
mod patch_manager;
pub mod settings;

pub use atomic::AtomicWriteError;
pub use group_template_manager::{GroupTemplateInfo, GroupTemplateManager, GroupTemplateSource};
pub use midi::{MidiError, MidiHandler};
pub use patch_manager::{PatchInfo, PatchManager};
pub use settings::AppSettings;
