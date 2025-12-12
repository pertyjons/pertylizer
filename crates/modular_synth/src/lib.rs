//! Modular Synthesizer - Main application crate
//!
//! A flexible, modular audio synthesis system with:
//! - Abstract audio backend (supports cpal, with room for JACK, PortAudio, etc.)
//! - Lock-free UI communication
//! - Real-time safe audio processing
//! - Self-describing modules for dynamic UI generation

#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::too_many_lines)]

// Re-export workspace crates
pub use synth_core;
pub use synth_dsp;
pub use synth_engine;
pub use synth_modules;
pub use synth_sequencer;

// TODO: Move GUI, audio backends, I/O, patches, visualizers from src/
