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
pub mod instrument;
pub mod metering;
pub mod params;
pub mod sequencer_engine;
pub mod state;
pub mod synth_engine;
pub mod tracker_effects;
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
    EffectType, EngineCommand, EngineEvent, InstrumentParam, ModuleId, ModuleTypeId, PolyModule,
    PortId, VisualizerType,
};
pub use graph::{Connection, GraphError, ModuleGraph};
pub use instrument::{Instrument, InstrumentId, MidiChannel};
pub use sequencer_engine::{PlayState, SequencerEngine};
pub use state::*;
pub use synth_engine::{CommandSender, EngineHandle, SynthEngine};
pub use voice::{Voice, VoiceState};
pub use voice_allocator::{
    AllocationMode, AllocatorConfig, NotePriority, StealingStrategy, VoiceAllocator,
};

// Re-export typed parameter system
pub use params::{
    AmplifierParam,
    // Ports
    AudioPort,
    ChorusParam,
    CompressorParam,
    ControlPort,
    DelayMode,
    DelayParam,
    DistortionMode,
    DistortionParam,
    EnvelopeParam,
    EqParam,
    FilterMode,
    FilterParam,
    FlangerParam,
    FmMode,
    GranularParam,
    LevelMeterParam,
    LfoParam,
    LfoWaveform,
    LoopMode,
    MathAlgo,
    MathOscillatorParam,
    MixerParam,
    ModuleType,
    NoiseParam,
    NoiseType,
    // Module-specific params
    OscillatorParam,
    OscilloscopeParam,
    Param,
    PhaserParam,
    Port,
    ReverbParam,
    SamplePlayerParam,
    SubOscOctave,
    SubOscParam,
    SubOscWaveform,
    // Type-safe enums
    Waveform,
};

// Re-export GUI-Engine communication types
pub use connectivity::{
    ModuleConnectivityStatus, ModuleError, ModuleErrorKind, PortVisualState, VoiceStealReason,
};
pub use cpu_tracker::{ModuleCpuStats, ModuleCpuTracker, ModuleTiming, TimingBuffer};
pub use event_priority::{
    EventPriority, PrioritizedEventConsumer, PrioritizedEventProducer, TimestampedEvent,
    prioritized_event_channel,
};
pub use hub::{ClientHandle, ClientId, ClientPermissions, ClientType, EngineHub, HubError};
pub use shared_state::{
    AtomicF32, ConnectionSnapshot, ModuleStateSnapshot, SharedEngineState, SharedGraphState,
    SharedMeterState, SharedTransportState,
};
pub use transactions::{
    BatchBuilder, BatchResult, CommandBatch, TransactionId, TransactionalCommand,
};
pub use visual_state::{CableVisualState, MiniMeter, ModuleStyle, ModuleVisualState, Point};
