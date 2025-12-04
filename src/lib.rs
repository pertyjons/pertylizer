//! Modular Synthesizer
//!
//! A flexible, modular audio synthesis system with:
//! - Abstract audio backend (supports cpal, with room for JACK, PortAudio, etc.)
//! - Lock-free UI communication
//! - Real-time safe audio processing
//! - Self-describing modules for dynamic UI generation
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                          UI                                  │
//! │  - Reads ModuleDescriptors for widget generation            │
//! │  - Sends EngineCommands via lock-free queue                 │
//! │  - Reads meters/state via atomics                           │
//! └──────────────┬──────────────────────────┬───────────────────┘
//!                │ Commands                  │ State reads
//!                ▼                           │
//! ┌──────────────────────────────────────────▼──────────────────┐
//! │                      SynthEngine                             │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
//! │  │ CommandQueue│  │ ModuleGraph │  │ StateSnapshot       │  │
//! │  │ (lock-free) │  │ (future)    │  │ (atomic values)     │  │
//! │  └─────────────┘  └─────────────┘  └─────────────────────┘  │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │ AudioProcessor trait
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │                      AudioHost                               │
//! │                 (manages streams)                            │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │ AudioBackend trait
//!              ┌────────────┼────────────┐
//!              ▼            ▼            ▼
//!         ┌─────────┐  ┌─────────┐  ┌─────────┐
//!         │  Cpal   │  │  Null   │  │ (future)│
//!         │ Backend │  │ Backend │  │  JACK   │
//!         └─────────┘  └─────────┘  └─────────┘
//! ```
//!
//! # Quick Start
//!
//! ```ignore
//! use modular_synth::{audio, engine};
//!
//! // Create the synth engine
//! let (engine, mut handle) = engine::SynthEngine::new();
//!
//! // Create audio host with default backend
//! let mut host = audio::default_host()?;
//!
//! // Start audio
//! let config = audio::StreamConfig::default();
//! host.start_output(None, &config, engine)?;
//!
//! // Play a note
//! handle.note_on(60, 0.8); // Middle C, velocity 0.8
//!
//! // ... later
//! handle.note_off(60);
//!
//! // Check meters
//! let (peak_l, peak_r) = handle.peak_meters();
//! println!("Peak: L={peak_l:.2} R={peak_r:.2}");
//!
//! // Stop when done
//! host.stop()?;
//! ```
//!
//! # Rust 2024 Features Used
//!
//! This crate uses several Rust 2024 features:
//! - Let chains in if/while expressions
//! - Improved pattern matching
//! - Enhanced error handling

// Temporarily allow missing docs during development
#![allow(missing_docs)]
#![warn(clippy::all)]
#![allow(clippy::module_inception)]

pub mod audio;
pub mod engine;
pub mod modules;
pub mod effects;
pub mod visualizers;
pub mod ui;
pub mod gui;
pub mod patch;
pub mod patches;
pub mod sequencer;
pub mod types;
pub mod io;

#[cfg(test)]
mod tests;

// Re-export commonly used items
pub use audio::{AudioHost, AudioHostTrait, AudioProcessor, StreamConfig};
pub use engine::{EngineCommand, EngineHandle, SynthEngine};

// Re-export voice management
pub use engine::{
    Voice, VoiceState, VoiceAllocator, AllocatorConfig, 
    AllocationMode, StealingStrategy, NotePriority,
    ModuleGraph, Connection, GraphError,
};

// Re-export module types
pub use modules::{
    Oscillator, Filter, LadderFilter, Envelope, Lfo, Amplifier, Mixer,
    Waveform, FilterType, LfoWaveform,
    ModuleDescriptor, ParameterDescriptor, PortDescriptor,
    PolyModule, AudioEffect, Describable,
    AudioBuffer, ProcessContext,
    ModuleCategory, WidgetHint, ParameterUnit,
};

// Re-export effects
pub use effects::{Delay, Reverb, Distortion, Chorus};

// Re-export visualizers
pub use visualizers::{Oscilloscope, LevelMeter, VisualizationBuffer};

// Re-export UI abstractions
pub use ui::{UiState, UiEvent, SynthEvent, ParameterWidget, ModulePanel};

// Re-export GUI abstractions
pub use gui::{GuiBackend, GuiType, SynthGuiConfig, create_backend, print_available_backends};

// Re-export patch system
pub use patch::{Patch, PatchError, ModuleState, ModuleType, ConnectionState, PatchSettings, example_patches};

// Re-export I/O
pub use io::{PatchManager, PatchInfo};

// Re-export sequencer types
pub use sequencer::{
    // Core types
    Song, Pattern, SequencerTrack, Note,
    // Time types
    Tick, PatternTick, Duration, TimeSignature, TICKS_PER_QUARTER,
    // Pitch/velocity
    Pitch, Velocity, NoteName,
    // IDs
    PatternId, TrackId, InstrumentId, NoteId,
    // Automation
    AutomationLane, AutomationPoint, AutomationTarget, CurveType,
    // Effects
    EffectCommand,
    // Events and input
    SequencerEvent, InputCommand, InputSource,
    // View helpers
    TrackerRow, TrackerCell, TrackerViewConfig,
};
