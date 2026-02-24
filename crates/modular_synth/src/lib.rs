//! Modular Synthesizer Application.
//!
//! This crate provides the complete synthesizer application:
//! - GUI (egui-based)
//! - Audio backends (cpal)
//! - MIDI I/O
//! - Patch loading/saving
//! - File import (WAV samples)

#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::similar_names)]

pub mod audio;
#[cfg(feature = "debug-tools")]
pub mod debug;
#[cfg(feature = "gui-egui")]
pub mod gui;
pub mod io;
#[cfg(feature = "mcp")]
pub mod mcp_bridge;
pub mod patch;
pub mod patches;

// Re-export workspace crates
pub use synth_core;
pub use synth_dsp;
pub use synth_engine;
pub use synth_modules;
pub use synth_sequencer;

// Re-export commonly used items
pub use patch::{Patch, PatchModuleType};

// Re-export from synth_engine
pub use synth_engine::{
    CommandSender, EffectChain, EngineCommand, EngineEvent, EngineHandle, EngineHub, Instrument,
    InstrumentId, MidiChannel, ModuleGraph, ModuleId, PlayState, SequencerEngine, SynthEngine,
    VoiceAllocator,
};

// Re-export from synth_core
pub use synth_core::{
    AudioBackend, AudioCallbackContext, AudioError, AudioProcessor, AudioStream, ModuleType,
    NormalizedValue, Param, SampleRate, StreamInfo,
};
