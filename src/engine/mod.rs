//! Synthesizer engine module.
//!
//! This module contains the core audio processing engine with:
//! - Lock-free command queue for UI communication
//! - Shared state for meters and transport
//! - Real-time safe audio processing
//! - Voice management and polyphony
//! - Module graph for signal routing
//! - Multi-GUI support with priority-based event distribution
//! - Per-module CPU tracking
//! - Transactional command batching

pub mod commands;
pub mod effect_chain;
pub mod graph;
pub mod metering;
pub mod params;
pub mod part;
pub mod sequencer_engine;
pub mod state;
pub mod synth_engine;
pub mod voice;
pub mod voice_allocator;

// Backwards compatibility alias
pub use params as typed_params;

// GUI-Engine communication modules (spec implementation)
pub mod connectivity;
pub mod cpu_tracker;
pub mod event_priority;
pub mod hub;
pub mod shared_state;
pub mod transactions;
pub mod visual_state;

pub use commands::{
    ModuleId, VoiceModule, ModuleTypeId, PortId, EngineCommand, VisualizerType, EffectType,
    PartParam, EngineEvent,
};
pub use state::*;
pub use synth_engine::{SynthEngine, EngineHandle};
pub use sequencer_engine::{SequencerEngine, PlayState};
pub use voice::{Voice, VoiceState};
pub use voice_allocator::{VoiceAllocator, AllocatorConfig, AllocationMode, StealingStrategy, NotePriority};
pub use graph::{ModuleGraph, Connection, GraphError};
pub use part::{PartId, MidiChannel, SynthPart};

// Re-export typed parameter system
pub use params::{
    ModuleType, TypedParam, TypedValue, Port,
    // Module-specific params
    OscillatorParam, MathOscillatorParam, FilterParam, EnvelopeParam, LfoParam,
    AmplifierParam, MixerParam, DelayParam, ReverbParam,
    DistortionParam, ChorusParam, PhaserParam, FlangerParam, CompressorParam, EqParam,
    SamplePlayerParam, GranularParam, OscilloscopeParam, LevelMeterParam,
    // Type-safe enums
    Waveform as TypedWaveform, LfoWaveform as TypedLfoWaveform,
    FilterMode, DelayMode, DistortionMode, LoopMode, MathAlgo,
    // Ports
    AudioPort, ControlPort,
};

// Re-export GUI-Engine communication types
pub use connectivity::{
    ModuleConnectivityStatus, VoiceStealReason, ModuleErrorKind, ModuleError, PortVisualState,
};
pub use event_priority::{
    EventPriority, TimestampedEvent, PrioritizedEventProducer, PrioritizedEventConsumer,
    prioritized_event_channel,
};
pub use shared_state::{
    AtomicF32, MeterState, TransportState, ModuleStateSnapshot, ConnectionSnapshot,
    SharedGraphState, SharedEngineState,
};
pub use visual_state::{
    ModuleStyle, ModuleVisualState, Point, CableVisualState, MiniMeter,
};
pub use cpu_tracker::{
    ModuleCpuStats, ModuleCpuTracker, ModuleTiming, TimingBuffer,
};
pub use hub::{
    ClientId, ClientType, ClientPermissions, EngineHub, ClientHandle, HubError,
};
pub use transactions::{
    TransactionId, TransactionalCommand, CommandBatch, BatchBuilder, BatchResult,
};
