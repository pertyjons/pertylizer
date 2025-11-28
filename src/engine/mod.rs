//! Synthesizer engine module.
//!
//! This module contains the core audio processing engine with:
//! - Lock-free command queue for UI communication
//! - Shared state for meters and transport
//! - Real-time safe audio processing
//! - Voice management and polyphony
//! - Module graph for signal routing

pub mod commands;
pub mod state;
pub mod synth_engine;
pub mod voice;
pub mod voice_allocator;
pub mod graph;
pub mod typed_params;

pub use commands::*;
pub use state::*;
pub use synth_engine::{SynthEngine, EngineHandle};
pub use voice::{Voice, VoiceState};
pub use voice_allocator::{VoiceAllocator, AllocatorConfig, AllocationMode, StealingStrategy, NotePriority};
pub use graph::{ModuleGraph, Connection, GraphError};

// Re-export typed parameter system
pub use typed_params::{
    ModuleType, TypedParam, TypedValue, Port,
    // Module-specific params
    OscillatorParam, FilterParam, EnvelopeParam, LfoParam,
    AmplifierParam, MixerParam, DelayParam, ReverbParam,
    DistortionParam, ChorusParam, SamplePlayerParam, GranularParam,
    // Type-safe enums
    Waveform as TypedWaveform, LfoWaveform as TypedLfoWaveform,
    FilterMode, DelayMode, DistortionMode, LoopMode,
    // Ports
    AudioPort, ControlPort,
};
