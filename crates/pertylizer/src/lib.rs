//! Pertylizer — a modular synthesizer.
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

pub mod activity_log;
pub mod analysis;
pub mod audio;
pub mod bundle;
pub mod composition;
pub mod group_templates;
#[cfg(feature = "gui-egui")]
pub mod gui;
pub mod harmony;
pub mod io;
#[cfg(feature = "mcp")]
pub mod mcp_bridge;
#[cfg(feature = "mcp")]
pub mod mcp_shared;
pub mod mod_grid_build;
pub mod module_factory;
pub mod module_targets;
pub mod panic_hook;
pub mod patch;
pub mod patches;
pub mod project;
pub mod project_apply;
pub mod session;
pub(crate) mod undo;

// Re-export workspace crates
pub use synth_core;
pub use synth_dsp;
pub use synth_engine;
pub use synth_modules;
pub use synth_sequencer;

// Re-export commonly used items
pub use patch::Patch;

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
