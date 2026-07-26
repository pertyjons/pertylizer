//! Synth engine with voice allocation, graph processing, and sequencer.
//!
//! This crate provides the core audio engine:
//! - Voice allocation and management
//! - Module graph for signal routing
//! - Effect chains
//! - Command/event system for UI communication
//! - Sequencer engine
//! - Multi-GUI hub support

#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::similar_names)]

pub mod click_generator;
pub mod commands;
pub mod connectivity;
pub mod cpu_tracker;
pub mod effect_chain;
pub mod event_priority;
pub mod graph;
pub mod hub;
pub mod instrument;
pub mod metering;
pub mod mod_grid;
pub mod recording;
pub mod return_bus;
pub mod sequencer_engine;
pub mod shared_state;
pub mod state;
pub mod synth_engine;
pub mod transactions;
pub mod visual_state;
pub mod visualizers;
pub mod voice;
pub mod voice_allocator;

/// Wrap a song in editable control-thread state with lock-free audio snapshots.
#[must_use]
pub fn shared_song(song: synth_sequencer::Song) -> std::sync::Arc<synth_sequencer::SharedSong> {
    std::sync::Arc::new(synth_sequencer::SharedSong::new(song))
}

// Engine exports
pub use commands::{
    EffectType, EngineCommand, EngineEvent, InstrumentParam, ModuleId, ModuleTypeId, NoteEvent,
    PortId, ReorderDirection, VisualizerType,
};
pub use connectivity::{
    ModuleConnectivityStatus, ModuleError, ModuleErrorKind, PortVisualState, VoiceStealReason,
};
pub use cpu_tracker::{ModuleCpuStats, ModuleCpuTracker, ModuleTiming, TimingBuffer};
pub use effect_chain::EffectChain;
pub use event_priority::{
    EventPriority, PrioritizedEventConsumer, PrioritizedEventProducer, TimestampedEvent,
    prioritized_event_channel,
};
pub use graph::{Connection, GraphError, ModuleGraph};
pub use hub::{ClientHandle, ClientId, ClientPermissions, ClientType, EngineHub, HubError};
pub use instrument::{Instrument, InstrumentCategory, InstrumentId, MidiChannel};
pub use metering::MeteringSystem;
pub use recording::RecordingState;
pub use return_bus::ReturnBusChannel;
pub use sequencer_engine::{PlayState, SequencerEngine};
pub use shared_state::{
    AtomicF32, ConnectionSnapshot, InstrumentSnapshot, ModuleStateSnapshot, ReturnBusSnapshot,
    ReturnEffectSnapshot, SharedEngineState, SharedGraphState, SharedMeterState,
    SharedTransportState,
};
pub use state::{
    AtomicF64, AtomicU32, ChannelMeterBank, CommandSync, EngineState, MAX_METER_SLOTS, MeterState,
    NO_FOCUSED_INSTRUMENT, TransportState,
};
pub use synth_engine::{CommandSender, CpuStageBreakdown, EngineHandle, SynthEngine};
pub use transactions::{
    BatchBuilder, BatchResult, CommandBatch, TransactionId, TransactionalCommand,
};
pub use visual_state::{CableVisualState, MiniMeter, ModuleStyle, ModuleVisualState, Point};
pub use voice::{Voice, VoiceId, VoiceState};
pub use voice_allocator::{
    AllocationMode, AllocatorConfig, NotePriority, StealingStrategy, VoiceAllocator,
};

// Re-export param types from synth_core for convenience
pub use synth_core::{
    AmplifierParam, AudioPort, ChorusParam, CompressorParam, ControlPort, DelayMode, DelayParam,
    DistortionMode, DistortionParam, EnvelopeParam, EqParam, FilterMode, FilterParam, FlangerParam,
    FmMode, LevelMeterParam, LfoParam, LfoWaveform, MathAlgo, MathOscillatorParam, MixerParam,
    ModuleType, NoiseParam, NoiseType, OscillatorParam, OscilloscopeParam, Param, PhaserParam,
    Port, ReverbParam, SubOscOctave, SubOscParam, SubOscWaveform, Waveform,
};

#[cfg(test)]
mod shared_song_tests {
    use super::shared_song;
    use synth_sequencer::Song;

    #[test]
    fn shared_song_wraps_a_readable_song() {
        let shared = shared_song(Song::new("Demo"));
        assert_eq!(shared.read().name, "Demo");
        // A second handle shares the same lock (Arc clone, not a deep copy).
        let clone = std::sync::Arc::clone(&shared);
        clone.write().name = "Renamed".to_string();
        assert_eq!(shared.read().name, "Renamed");
    }
}
