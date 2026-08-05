//! The main synthesizer engine with polyphony support.
//!
//! This is the complete engine that uses:
//! - VoiceAllocator for polyphonic voice management
//! - ModuleGraph for signal routing within each voice
//! - Effect chain for post-voice processing

use arrayvec::ArrayVec;
use parking_lot::Mutex;
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::audio_input::AudioInputStream;
use crate::commands::{EngineCommand, EngineEvent, ModuleId, NoteEvent, PortId, ReorderDirection};
use crate::effect_chain::{EffectChain, EffectSlot};
use crate::graph::ModuleGraph;
use crate::instrument::{
    Instrument, InstrumentId, MidiChannelSelection, mix_stereo_faded, stereo_peak,
};
use crate::metering::MeteringSystem;
use crate::recording::RecordingState;
use crate::return_bus::ReturnBusChannel;
use crate::sequencer_engine::{PlayState, SequencerEngine};
use crate::state::{CommandSync, EngineState};
use crate::visualizers::{LevelMeter, Oscilloscope, SpectrumAnalyzer, VisualizationBuffer};
use synth_core::params::LfoWaveform;
use synth_core::{
    AudioBuffer, AudioCallbackContext, AudioProcessor, BeatPosition, BipolarValue, CcNumber,
    CpuUsage, EnvelopeParam, FilterParam, Gain, Hertz, MidiNote, ModuleType, NormalizedValue,
    Param, PolyModule as PolyModuleTrait, ProcessContext, SampleCount, SampleRate, Seconds,
    Semitones, StreamInfo, Velocity,
};
use synth_sequencer::{
    AutoInstrumentParam, AutomationTarget, Glide, GlideFrom, GlideInterp, GlobalParam,
    NoteExpression, ReturnBusId, SequencerEvent, TrackParam, VibratoShape,
};

/// Capacity of the engine's command ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct CommandCapacity(usize);

impl CommandCapacity {
    /// Default capacity, large enough for substantial patch-loading bursts.
    pub const DEFAULT: Self = Self(16_384);

    /// Create a non-zero command capacity.
    pub const fn new(capacity: usize) -> Option<Self> {
        if capacity == 0 {
            None
        } else {
            Some(Self(capacity))
        }
    }

    /// Return the number of commands the ring can hold.
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl Default for CommandCapacity {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Failure returned when a command cannot be enqueued before its deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandSendError {
    /// The audio thread did not free a command slot in time.
    #[error("command queue remained full until the send deadline")]
    Timeout,
}

/// Size of the event ring buffer.
const EVENT_BUFFER_SIZE: usize = 256;

/// Size of the return channel for modules that need to be dropped on main thread.
const RETURN_BUFFER_SIZE: usize = 256;

/// Size of the replaced-script return channel. A single `SetModScript` re-install
/// on a fully-allocated instrument hands back up to `1 + VoiceCount::MAX_ALLOCATOR`
/// (= 129: template voice graph + every live voice) replaced `Arc`s, and
/// `process_commands` can drain several such commands in one audio block before
/// the main thread next calls `cleanup_dropped_modules`. Size for ~15 full-
/// polyphony re-installs per drain so the ring never overflows into an
/// audio-thread `Arc` drop in realistic live editing.
const SCRIPT_TRASH_BUFFER_SIZE: usize = 2048;

/// Items returned from audio thread for main thread cleanup.
/// This prevents memory deallocation from happening on the audio thread.
pub enum DroppedItem {
    /// A voice module from the modular graph.
    Module(Box<dyn PolyModuleTrait>),
    /// An instrument with its voice allocator.
    Instrument(Box<Instrument>),
}

/// Wrapper for modules returned from audio thread for deferred cleanup.
/// This allows dropping to happen on the main thread to avoid
/// deallocations on the real-time audio thread.
pub struct DroppedModule(pub Box<dyn PolyModuleTrait>);

/// Ownership retired by a real-time pointer swap and destroyed by the control
/// thread when [`EngineHandle::cleanup_dropped_modules`] runs.
enum DeferredControlDrop {
    Song(Arc<synth_sequencer::SharedSong>),
    AudioInput(AudioInputStream),
}

/// Capacity permits shared by control-command producers and the deferred-drop
/// consumer. Reserving before enqueue guarantees that every pointer-swap
/// command has a free return-ring slot when the audio thread processes it.
struct DeferredDropSlots {
    available: AtomicUsize,
}

impl DeferredDropSlots {
    const fn new(capacity: CommandCapacity) -> Self {
        Self {
            available: AtomicUsize::new(capacity.as_usize()),
        }
    }

    fn try_reserve(&self) -> bool {
        self.available
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |available| {
                available.checked_sub(1)
            })
            .is_ok()
    }

    fn release(&self) {
        self.available.fetch_add(1, Ordering::Release);
    }
}

/// A clonable sender for engine commands.
///
/// This wrapper allows multiple sources (GUI, MIDI, automation) to send
/// commands to the engine. Uses Mutex for thread-safe access to the
/// ring buffer producer - the overhead is minimal since command sending
/// is infrequent compared to audio processing.
#[derive(Clone)]
pub struct CommandSender {
    producer: Arc<Mutex<ringbuf::HeapProd<EngineCommand>>>,
    state: Arc<EngineState>,
    /// Shared enqueue/drain counters. Bumped on every successful push so a
    /// reader can wait for corresponding DSP mutations to reach the audio
    /// thread. See [`CommandSync`].
    command_sync: Arc<CommandSync>,
    capacity: CommandCapacity,
    deferred_drop_slots: Arc<DeferredDropSlots>,
}

impl CommandSender {
    /// Create a command sender with synchronous control-snapshot publication.
    fn new(
        producer: ringbuf::HeapProd<EngineCommand>,
        state: Arc<EngineState>,
        capacity: CommandCapacity,
        deferred_drop_slots: Arc<DeferredDropSlots>,
    ) -> Self {
        Self {
            producer: Arc::new(Mutex::new(producer)),
            command_sync: Arc::clone(&state.command_sync),
            state,
            capacity,
            deferred_drop_slots,
        }
    }

    /// Capacity of the shared command ring.
    pub const fn capacity(&self) -> CommandCapacity {
        self.capacity
    }

    /// Send a command to the engine (non-blocking, may fail if queue full).
    pub fn send(&self, command: EngineCommand) -> bool {
        // Bump `enqueued` *inside* the producer lock, atomically with the push:
        // it must advance in lockstep with FIFO position so a save's
        // `wait_for_pending_commands` snapshot always accounts for every command
        // already ahead of the caller's in the ring. Bumping after releasing the
        // lock would let a concurrent sender's still-uncounted push drain first
        // and satisfy the wait prematurely (stale-graph read). See `CommandSync`.
        let mut producer = self.producer.lock();
        if !producer.is_full() {
            let reserved_drop = command_retires_control_ownership(&command);
            if reserved_drop && !self.deferred_drop_slots.try_reserve() {
                return false;
            }
            crate::control_snapshot::publish(&command, &self.state);
            let pushed = producer.try_push(command).is_ok();
            debug_assert!(pushed, "a sole producer lost a reserved command slot");
            if pushed {
                self.command_sync.note_enqueued();
            } else if reserved_drop {
                self.deferred_drop_slots.release();
            }
            pushed
        } else {
            false
        }
    }

    /// Send a command to the engine, blocking until there's space.
    /// Use this when loading patches or doing bulk operations.
    ///
    /// Uses bounded exponential backoff to avoid busy-waiting while still
    /// responding quickly when the queue has space. Returns after at most
    /// approximately 500 ms if the audio thread is not draining.
    pub fn send_blocking(&self, command: EngineCommand) -> Result<(), CommandSendError> {
        self.send_with_timeout(command, Duration::from_millis(500))
    }

    fn send_with_timeout(
        &self,
        command: EngineCommand,
        timeout: Duration,
    ) -> Result<(), CommandSendError> {
        const MAX_BACKOFF: Duration = Duration::from_millis(10);

        let deadline = Instant::now() + timeout;
        let mut backoff = Duration::ZERO;
        let mut cmd = command;

        loop {
            // Lock, push, and bump `enqueued` on success in one critical section
            // (see `send`); release the guard before any backoff sleep.
            {
                let mut producer = self.producer.lock();
                if !producer.is_full() {
                    let reserved_drop = command_retires_control_ownership(&cmd);
                    let has_drop_slot = !reserved_drop || self.deferred_drop_slots.try_reserve();
                    if has_drop_slot {
                        crate::control_snapshot::publish(&cmd, &self.state);
                        match producer.try_push(cmd) {
                            Ok(()) => {
                                self.command_sync.note_enqueued();
                                return Ok(());
                            }
                            Err(returned_command) => {
                                debug_assert!(
                                    false,
                                    "a sole producer lost a reserved command slot"
                                );
                                if reserved_drop {
                                    self.deferred_drop_slots.release();
                                }
                                cmd = returned_command;
                            }
                        }
                    }
                }
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(CommandSendError::Timeout);
            }

            if backoff.is_zero() {
                std::thread::yield_now();
                backoff = Duration::from_millis(1);
            } else {
                std::thread::sleep(backoff.min(deadline.saturating_duration_since(now)));
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

const fn command_retires_control_ownership(command: &EngineCommand) -> bool {
    matches!(
        command,
        EngineCommand::SetSong { .. }
            | EngineCommand::SetAudioInputConsumer { .. }
            | EngineCommand::ClearAudioInputConsumer
    )
}

/// Per-stage CPU usage snapshot for the status-bar breakdown tooltip. Each
/// field is a fraction of the per-buffer real-time budget (e.g. `0.09` = 9 %),
/// matching the units of the overall `total`.
#[derive(Debug, Clone, Copy, Default)]
#[must_use]
pub struct CpuStageBreakdown {
    /// All instrument/voice processing, including the channel- and return-bus stages.
    pub voices: CpuUsage,
    /// The user-added modular graph.
    pub module_graph: CpuUsage,
    /// The master effect chain.
    pub master_fx: CpuUsage,
    /// The Mod Grid control-rate pre-pass (all running instances).
    pub mod_grid: CpuUsage,
    /// Overall audio-callback load (the status-bar number).
    pub total: CpuUsage,
}

/// Handle for the UI to communicate with the engine.
pub struct EngineHandle {
    /// Send commands to the engine (clonable for sharing with MIDI, etc.).
    command_sender: CommandSender,
    /// Receive events from the engine.
    event_consumer: ringbuf::HeapCons<EngineEvent>,
    /// Receive dropped modules from audio thread (for main thread cleanup).
    return_consumer: ringbuf::HeapCons<DroppedModule>,
    /// Receive dropped instruments from audio thread (for main thread cleanup).
    instrument_return_consumer: ringbuf::HeapCons<Box<Instrument>>,
    /// Receive cleared automation dedup keys from the audio thread so their
    /// `ParamId(Arc<str>)` frees here, on the main thread, never on the audio
    /// thread (see `SequencerEngine::clear_automation_dedup`).
    automation_trash_consumer: ringbuf::HeapCons<AutomationTarget>,
    /// Receive replaced metadata strings for destruction off the audio thread.
    metadata_trash_consumer: ringbuf::HeapCons<String>,
    /// Receive replaced mod-matrix scripts from the audio thread so the old
    /// `Arc<BoundScript>` (bytecode + source text) frees here, on the main
    /// thread, never on the audio thread (see `handle_set_mod_script`).
    script_trash_consumer: ringbuf::HeapCons<Arc<synth_core::script::BoundScript>>,
    /// Receive replaced graph-node descriptors from the audio thread so the old
    /// `ModuleDescriptor` frees here, on the main thread (see `handle_set_mod_script`).
    descriptor_trash_consumer: ringbuf::HeapCons<Arc<synth_core::ModuleDescriptor>>,
    /// Receive replaced Mod Grid runtimes from the audio thread so the old
    /// runtime's modules free here, on the main thread, never on the audio
    /// thread (see the `SetModGrid` handler).
    mod_grid_trash_consumer: ringbuf::HeapCons<Box<crate::mod_grid::ModGridRuntime>>,
    /// Receive song and live-input objects retired by pointer swaps.
    deferred_control_drop_consumer: ringbuf::HeapCons<DeferredControlDrop>,
    deferred_drop_slots: Arc<DeferredDropSlots>,
    /// Shared state for reading meters, etc.
    pub state: Arc<EngineState>,
    /// Visualization buffers keyed by module ID (shared with engine via Arc).
    pub visualization_buffers: HashMap<ModuleId, Arc<VisualizationBuffer>>,
    /// Note event consumer for OSC telemetry.
    note_event_consumer: Option<ringbuf::HeapCons<NoteEvent>>,
}

impl EngineHandle {
    /// Send a command to the engine (non-blocking, may fail if queue full).
    pub fn send(&mut self, command: EngineCommand) -> bool {
        self.command_sender.send(command)
    }

    /// Send a command to the engine, blocking until there's space in the queue.
    /// Use this when loading patches or doing bulk operations.
    /// Returns an error if the audio thread does not free a slot before the
    /// bounded deadline.
    pub fn send_blocking(&mut self, command: EngineCommand) -> Result<(), CommandSendError> {
        self.command_sender.send_blocking(command)
    }

    /// Get a clonable command sender for sharing with other sources (MIDI, etc.).
    ///
    /// This allows multiple threads/sources to send commands to the engine.
    pub fn command_sender(&self) -> CommandSender {
        self.command_sender.clone()
    }

    /// Capacity of the command ring shared with this handle.
    pub const fn command_capacity(&self) -> CommandCapacity {
        self.command_sender.capacity()
    }

    /// Take the note event consumer for OSC telemetry.
    /// Returns `None` if already taken.
    pub fn take_note_event_consumer(&mut self) -> Option<ringbuf::HeapCons<NoteEvent>> {
        self.note_event_consumer.take()
    }

    /// Poll and drop any modules/instruments returned from the audio thread.
    /// Call this regularly from the main thread to clean up removed items.
    pub fn cleanup_dropped_modules(&mut self) {
        // Clean up dropped voice modules
        while self.return_consumer.try_pop().is_some() {
            // Module is dropped here on the main thread - no audio glitches!
        }
        // Clean up dropped instruments
        while self.instrument_return_consumer.try_pop().is_some() {
            // Instrument is dropped here on the main thread - no audio glitches!
        }
        // Clean up cleared automation dedup keys — their ParamId(Arc<str>) frees
        // here on the main thread, never on the audio thread.
        while self.automation_trash_consumer.try_pop().is_some() {}
        // Free replaced names, descriptions, and colors on the control thread.
        while self.metadata_trash_consumer.try_pop().is_some() {}
        // Clean up replaced mod-matrix scripts — their Arc<BoundScript> frees here
        // on the main thread, never on the audio thread.
        while self.script_trash_consumer.try_pop().is_some() {}
        // Clean up replaced graph-node descriptors — their String/Vec fields free
        // here on the main thread, never on the audio thread.
        while self.descriptor_trash_consumer.try_pop().is_some() {}
        // Clean up replaced Mod Grid runtimes — their `Box<dyn PolyModule>` DSP
        // frees here on the main thread, never on the audio thread.
        while self.mod_grid_trash_consumer.try_pop().is_some() {}
        while let Some(retired) = self.deferred_control_drop_consumer.try_pop() {
            match retired {
                DeferredControlDrop::Song(song) => drop(song),
                DeferredControlDrop::AudioInput(stream) => drop(stream),
            }
            self.deferred_drop_slots.release();
        }
    }

    /// Send a note on event to the default channel.
    pub fn note_on(&mut self, note: MidiNote, velocity: Velocity) -> bool {
        self.send(EngineCommand::NoteOn {
            note,
            velocity,
            channel: super::instrument::MidiChannelSelection::CH1,
            instrument_id: None,
        })
    }

    /// Send a note off event to the default channel.
    pub fn note_off(&mut self, note: MidiNote) -> bool {
        self.send(EngineCommand::NoteOff {
            note,
            channel: super::instrument::MidiChannelSelection::CH1,
            instrument_id: None,
        })
    }

    /// Send a note on event to a specific channel.
    pub fn note_on_channel(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        channel: super::instrument::MidiChannelSelection,
    ) -> bool {
        self.send(EngineCommand::NoteOn {
            note,
            velocity,
            channel,
            instrument_id: None,
        })
    }

    /// Send a note off event to a specific channel.
    pub fn note_off_channel(
        &mut self,
        note: MidiNote,
        channel: super::instrument::MidiChannelSelection,
    ) -> bool {
        self.send(EngineCommand::NoteOff {
            note,
            channel,
            instrument_id: None,
        })
    }

    /// Set a voice module parameter using the type-safe API.
    ///
    /// This targets a specific instrument's voice graph.
    pub fn set_voice_parameter(
        &mut self,
        instrument_id: crate::instrument::InstrumentId,
        target: crate::commands::PolyModule,
        param: synth_core::Param,
    ) -> bool {
        self.send(EngineCommand::SetVoiceParameter {
            instrument_id,
            target,
            param,
        })
    }

    /// Set an effect parameter using typed API.
    ///
    /// # Arguments
    /// * `instrument_id` - Target instrument's effect chain (None for future global master bus)
    /// * `module_id` - The id of the effect slot to modify
    /// * `param` - The parameter to set
    pub fn set_effect_parameter(
        &mut self,
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        param: synth_core::Param,
    ) -> bool {
        self.send(EngineCommand::SetEffectParameter {
            instrument_id,
            module_id,
            param,
        })
    }

    /// Set master volume.
    pub fn set_master_volume(&mut self, volume: Gain) -> bool {
        self.send(EngineCommand::SetMasterVolume(volume))
    }

    /// Poll for events from the engine.
    pub fn poll_event(&mut self) -> Option<EngineEvent> {
        self.event_consumer.try_pop()
    }

    /// Get the current peak meter values.
    pub fn peak_meters(&self) -> (synth_core::Amplitude, synth_core::Amplitude) {
        self.state.meters.get_peak()
    }

    /// Get the current RMS meter values.
    pub fn rms_meters(&self) -> (synth_core::Amplitude, synth_core::Amplitude) {
        self.state.meters.get_rms()
    }

    /// Post-fader peak for a channel (instrument), or 0.0 when it isn't
    /// currently audible/metered. Read each frame to drive a per-strip meter.
    #[must_use]
    pub fn channel_peak(&self, id: InstrumentId) -> f32 {
        self.state.channel_meters.peak_for(id.as_u64())
    }

    /// Post-fader peak for a return bus, or 0.0 when muted/absent.
    #[must_use]
    pub fn return_peak(&self, id: synth_sequencer::ReturnBusId) -> f32 {
        self.state.return_meters.peak_for(u64::from(id.0))
    }

    /// Get the current voice count.
    pub fn voice_count(&self) -> u32 {
        self.state.voice_count.load()
    }

    /// Get the current CPU usage.
    pub fn cpu_usage(&self) -> CpuUsage {
        CpuUsage::new(self.state.cpu_usage.load())
    }

    /// Whether this build reads the wall clock inside the audio callback to
    /// publish CPU diagnostics.
    pub const fn cpu_profiling_enabled() -> bool {
        cfg!(feature = "rt-profiling")
    }

    /// Get the per-stage CPU breakdown (each a fraction of the buffer budget,
    /// same units as [`Self::cpu_usage`]) for the status-bar tooltip.
    pub fn cpu_breakdown(&self) -> CpuStageBreakdown {
        CpuStageBreakdown {
            voices: CpuUsage::new(self.state.cpu_voices.load()),
            module_graph: CpuUsage::new(self.state.cpu_module_graph.load()),
            master_fx: CpuUsage::new(self.state.cpu_master_fx.load()),
            mod_grid: CpuUsage::new(self.state.cpu_mod_grid.load()),
            total: CpuUsage::new(self.state.cpu_usage.load()),
        }
    }

    /// Get the master volume.
    pub fn master_volume(&self) -> f32 {
        self.state.master_volume.load()
    }

    /// Get the current playback position in sequencer ticks.
    pub fn playback_ticks(&self) -> u64 {
        self.state.transport.get_ticks()
    }

    /// Check if the sequencer is playing.
    pub fn is_playing(&self) -> bool {
        self.state.transport.is_playing()
    }

    /// Add a visualization buffer for a module (Arc is shared with engine).
    pub fn add_visualization_buffer(
        &mut self,
        module_id: ModuleId,
        buffer: Arc<VisualizationBuffer>,
    ) {
        self.visualization_buffers.insert(module_id, buffer);
    }

    /// Get a visualization buffer for a module.
    pub fn get_visualization_buffer(&self, module_id: ModuleId) -> Option<&VisualizationBuffer> {
        self.visualization_buffers
            .get(&module_id)
            .map(|arc| arc.as_ref())
    }

    /// Remove a visualization buffer.
    pub fn remove_visualization_buffer(&mut self, module_id: ModuleId) {
        self.visualization_buffers.remove(&module_id);
    }

    /// Set the focused instrument for keyboard/MIDI input.
    /// When set, keyboard input (channel 0) goes only to this instrument.
    /// When None, traditional MIDI channel routing is used.
    pub fn set_focused_instrument(&mut self, instrument_id: Option<InstrumentId>) -> bool {
        self.send(EngineCommand::SetFocusedInstrument(instrument_id))
    }

    /// Get the currently focused instrument ID.
    /// Returns None if using traditional MIDI channel routing.
    pub fn get_focused_instrument(&self) -> Option<InstrumentId> {
        self.state.get_focused_instrument()
    }
}

/// MIDI CC number of the sustain (damper) pedal.
const CC_SUSTAIN_PEDAL: u8 = 64;

/// The main synthesizer engine with polyphony.
pub struct SynthEngine {
    /// Receive commands from UI.
    command_consumer: ringbuf::HeapCons<EngineCommand>,
    /// Send events to UI.
    event_producer: ringbuf::HeapProd<EngineEvent>,
    /// Send note events to OSC telemetry (lock-free, fire-and-forget).
    note_event_producer: ringbuf::HeapProd<NoteEvent>,
    /// Send removed modules back to UI for dropping on main thread.
    return_producer: ringbuf::HeapProd<DroppedModule>,
    /// Send removed instruments back to UI for dropping on main thread.
    instrument_return_producer: ringbuf::HeapProd<Box<Instrument>>,
    /// Send replaced instrument/module metadata to the main thread for drop.
    metadata_trash_producer: ringbuf::HeapProd<String>,
    /// Send replaced mod-matrix scripts back to UI for dropping on the main
    /// thread. `set_mod_script` runs during the command drain (audio thread), so
    /// the old `Arc<BoundScript>`'s final `free()` must not happen here.
    script_trash_producer: ringbuf::HeapProd<Arc<synth_core::script::BoundScript>>,
    /// Send replaced graph-node descriptors back to the main thread for dropping.
    /// A script edit that changes a module's `param` set swaps a fresh descriptor
    /// into each voice node (audio thread); the old `ModuleDescriptor` owns
    /// `String`s/`Vec`s whose free must not run in `process()`.
    descriptor_trash_producer: ringbuf::HeapProd<Arc<synth_core::ModuleDescriptor>>,
    /// Shared state.
    state: Arc<EngineState>,

    // === Instrument management (multitimbral) ===
    /// All synthesizer instruments (each with its own voice graph and allocator).
    instruments: Vec<Box<Instrument>>,

    // === Master effects ===
    /// Global master effect chain (processes mixed output from all instruments).
    /// Effects like master reverb, limiter, EQ go here.
    master_effects: EffectChain,

    // === Global module graph ===
    /// The global module graph for modular routing.
    /// Contains all user-added modules and their connections.
    module_graph: ModuleGraph,
    /// Whether to use the global module graph for processing.
    use_modular_routing: bool,

    // === Audio state ===
    sample_rate: f32,
    master_volume: f32,

    // === Buffers ===
    /// Main stereo mix buffer (interleaved L/R).
    mix_buffer: AudioBuffer,
    /// Output buffer for the global module graph.
    graph_output: AudioBuffer,
    /// Previous-callback output per instrument (interleaved stereo).
    /// Read by sidechain consumers; written after each instrument's
    /// `process()`. Pre-allocated on instrument add/remove to avoid
    /// audio-thread allocation. Introduces ~1 buffer of sidechain
    /// detection latency — acceptable for compressor envelope follow.
    prev_instrument_outputs: std::collections::HashMap<InstrumentId, AudioBuffer>,
    /// Per-track controls applied to each tagged voice before the shared
    /// instrument effect chain. The full `TrackId` space is pre-allocated and
    /// generation-marked, so refresh and lookup are allocation-free without a
    /// 65K-entry clear on every callback.
    track_controls: Box<[TrackControlSlot]>,
    track_control_generation: u64,

    // === Sends / returns (Phase 7) ===
    /// Return busses (effect-send destinations), each with its own effect
    /// chain and fader. Created/removed via commands (allocation happens there,
    /// off the steady-state hot path), processed after the channel-bus stage.
    /// The fader/mute is a per-block snapshot of the song definition.
    return_busses: Vec<ReturnBusChannel>,
    /// `ReturnBusId` → index into `return_busses`, rebuilt on every return-bus
    /// mutation (off the hot path) so `update_track_controls` resolves sends and
    /// faders in O(1) instead of scanning the vec each audio block.
    return_index: std::collections::HashMap<ReturnBusId, usize>,
    /// Per-instrument resolved send taps, refreshed each block alongside
    /// `track_controls`. Pre-allocated with `MAX_CHANNEL_SENDS` capacity per
    /// instrument so the audio thread never grows a vec. Keyed by engine id.
    channel_sends:
        std::collections::HashMap<InstrumentId, ArrayVec<ChannelSend, MAX_CHANNEL_SENDS>>,
    /// Resolved bus-to-bus send taps, one inner vec per return-bus index
    /// (parallel to `return_busses`). Refreshed each block in
    /// `update_track_controls`; inner vecs are cleared and refilled (no realloc
    /// in steady state). `target_index` is already resolved into `return_busses`.
    return_sends: Vec<Vec<ResolvedReturnSend>>,
    /// Return-bus processing order (indices into `return_busses`) such that every
    /// source is processed before its bus-to-bus targets. Recomputed each block
    /// by a Kahn topological sort; falls back to index order for any nodes left
    /// in a cycle (defensive — cycles are rejected when sends are created).
    return_order: Vec<usize>,
    /// Scratch indegree counts for the `return_order` topological sort, kept as a
    /// field so the sort reuses its allocation each block.
    return_indegree: Vec<u32>,
    /// Scratch buffer holding one return's post-fader output while it is summed
    /// into the master mix and tapped into bus-to-bus targets.
    return_scratch: AudioBuffer,

    // === Metering ===
    metering: MeteringSystem,

    // === Sequencer ===
    sequencer: SequencerEngine,
    /// Pre-allocated buffer for sequencer events (real-time safe).
    sequencer_event_buffer: Vec<SequencerEvent>,

    // === Recording ===
    recording: crate::recording::RecordingBuffer,
    click_generator: crate::click_generator::ClickGenerator,
    /// Saved loop state before recording started (start, end, enabled).
    pre_record_loop: Option<(synth_sequencer::Tick, synth_sequencer::Tick, bool)>,
    /// Pending recorded notes that failed to send (retry on next process cycle).
    pending_recorded_notes: Option<(
        synth_sequencer::PatternId,
        Vec<crate::recording::RecordedNote>,
        bool,
    )>,

    // === Audio input ===
    /// Live audio input and its lock-free asynchronous resampler.
    audio_input_stream: Option<AudioInputStream>,
    /// Pre-allocated buffer for audio input (stereo interleaved, block_size * 2).
    audio_input_buffer: Vec<f32>,

    // === Mod Grid ===
    /// Running mod-grid instances, swapped wholesale via `SetModGrid`.
    mod_grid: Box<crate::mod_grid::ModGridRuntime>,
    /// Scratch output buffer for driving each instance's `ModuleGraph::process`
    /// (the graph's designated output is unused — targets read named ports).
    mod_grid_scratch: AudioBuffer,
    /// Additive master-volume offset from the mod grid for this block. (The
    /// per-track and per-instrument offset accumulators live in `mod_grid` itself,
    /// pre-keyed off the audio thread, so the pre-pass never allocates.)
    grid_master_volume_offset: f32,
    /// Live MIDI CC state, read by Mod Grid `MidiCc` sources. Boxed to keep the
    /// engine struct small (the state is ~8 KB of fixed arrays).
    midi_cc: Box<crate::mod_grid::MidiCcState>,
    /// Sustain-pedal (CC64) state per MIDI channel (0..15). While a channel's
    /// pedal is down, its NoteOffs are deferred (see `sustained_notes`).
    sustain_pedal_down: [bool; 16],
    /// `sustained_notes[channel][note]` = a NoteOff arrived while the pedal was
    /// held, so the note is sustained until the pedal lifts. A fixed bitfield
    /// (never allocates on the audio thread, constant-time set/clear, and inherent
    /// deduplication); boxed to keep the engine struct small (~2 KB).
    sustained_notes: Box<[[bool; 128]; 16]>,
    /// Producer end of the mod-grid trash channel: the old runtime is pushed here
    /// on a `SetModGrid` swap so its modules are dropped off the audio thread.
    mod_grid_trash_producer: ringbuf::HeapProd<Box<crate::mod_grid::ModGridRuntime>>,
    /// A single-slot backstop for the (practically unreachable) case where the
    /// trash channel is full at swap time: the old runtime is parked here instead
    /// of being dropped on the audio thread, and flushed into the channel on the
    /// next swap. Guarantees no DSP is destructed on the audio thread.
    mod_grid_pending_drop: Option<Box<crate::mod_grid::ModGridRuntime>>,
    /// Producer for objects whose destructor must run on the control thread.
    deferred_control_drop_producer: ringbuf::HeapProd<DeferredControlDrop>,
    deferred_drop_slots: Arc<DeferredDropSlots>,
    // Wall-clock reads are deliberately absent from normal audio builds.
    #[cfg(feature = "rt-profiling")]
    callback_duration_sum: f32,
    #[cfg(feature = "rt-profiling")]
    callback_count: u32,
    #[cfg(feature = "rt-profiling")]
    stage_voices_sum: f32,
    #[cfg(feature = "rt-profiling")]
    stage_module_graph_sum: f32,
    #[cfg(feature = "rt-profiling")]
    stage_master_fx_sum: f32,
    #[cfg(feature = "rt-profiling")]
    stage_mod_grid_sum: f32,
}

impl SynthEngine {
    /// Create a new engine with default configuration.
    /// Create a new engine.
    ///
    /// The engine starts with no instruments — create them explicitly
    /// via [`EngineCommand::AddInstrument`].
    pub fn new() -> (Self, EngineHandle) {
        Self::with_command_capacity(CommandCapacity::DEFAULT)
    }

    /// Create an engine with an explicit command-ring capacity.
    ///
    /// Small capacities are useful for deterministic backpressure tests;
    /// production callers normally use [`Self::new`].
    pub fn with_command_capacity(command_capacity: CommandCapacity) -> (Self, EngineHandle) {
        let state = EngineState::new();

        // Create command ring buffer
        let command_rb = HeapRb::<EngineCommand>::new(command_capacity.as_usize());
        let (command_producer, command_consumer) = command_rb.split();

        // Create event ring buffer
        let event_rb = HeapRb::<EngineEvent>::new(EVENT_BUFFER_SIZE);
        let (event_producer, event_consumer) = event_rb.split();

        // Create note event ring buffer for OSC telemetry
        let note_event_rb = HeapRb::<NoteEvent>::new(EVENT_BUFFER_SIZE);
        let (note_event_producer, note_event_consumer) = note_event_rb.split();

        // Create return buffer for modules to be dropped on main thread
        let return_rb = HeapRb::<DroppedModule>::new(RETURN_BUFFER_SIZE);
        let (return_producer, return_consumer) = return_rb.split();

        // Create return buffer for instruments to be dropped on main thread
        let instrument_return_rb = HeapRb::<Box<Instrument>>::new(RETURN_BUFFER_SIZE);
        let (instrument_return_producer, instrument_return_consumer) = instrument_return_rb.split();

        // Create return buffer for cleared automation dedup keys, whose
        // `ParamId(Arc<str>)` must not run its (possibly final) drop on the
        // audio thread (see `SequencerEngine::clear_automation_dedup`).
        let automation_trash_rb = HeapRb::<AutomationTarget>::new(RETURN_BUFFER_SIZE);
        let (automation_trash_producer, automation_trash_consumer) = automation_trash_rb.split();

        // A command can retire at most one metadata string except graph clears,
        // whose total is bounded by the modules previously accepted through the
        // command queue. Match that queue so a full drain cannot overflow into
        // an audio-thread String drop.
        let metadata_trash_rb = HeapRb::<String>::new(command_capacity.as_usize());
        let (metadata_trash_producer, metadata_trash_consumer) = metadata_trash_rb.split();

        // Create return buffer for replaced mod-matrix scripts, whose
        // `Arc<BoundScript>` must not run its (possibly final) drop on the audio
        // thread (see `handle_set_mod_script`).
        let script_trash_rb =
            HeapRb::<Arc<synth_core::script::BoundScript>>::new(SCRIPT_TRASH_BUFFER_SIZE);
        let (script_trash_producer, script_trash_consumer) = script_trash_rb.split();

        // Return buffer for replaced graph-node descriptors (a script edit that
        // changes a module's param set swaps a fresh descriptor into every voice
        // node); same worst-case fan-out as the script trash (template + voices).
        let descriptor_trash_rb =
            HeapRb::<Arc<synth_core::ModuleDescriptor>>::new(SCRIPT_TRASH_BUFFER_SIZE);
        let (descriptor_trash_producer, descriptor_trash_consumer) = descriptor_trash_rb.split();

        // Trash channel for replaced Mod Grid runtimes (their DSP frees off the
        // audio thread). A small depth is plenty — swaps are user-paced.
        let mod_grid_trash_rb =
            HeapRb::<Box<crate::mod_grid::ModGridRuntime>>::new(RETURN_BUFFER_SIZE);
        let (mod_grid_trash_producer, mod_grid_trash_consumer) = mod_grid_trash_rb.split();
        let deferred_control_drop_rb =
            HeapRb::<DeferredControlDrop>::new(command_capacity.as_usize());
        let (deferred_control_drop_producer, deferred_control_drop_consumer) =
            deferred_control_drop_rb.split();
        let deferred_drop_slots = Arc::new(DeferredDropSlots::new(command_capacity));

        let mut engine = Self {
            command_consumer,
            event_producer,
            note_event_producer,
            return_producer,
            instrument_return_producer,
            metadata_trash_producer,
            script_trash_producer,
            descriptor_trash_producer,
            state: Arc::clone(&state),
            instruments: Vec::with_capacity(128),
            master_effects: EffectChain::new(),
            module_graph: ModuleGraph::new(),
            use_modular_routing: false,
            sample_rate: 48000.0,
            master_volume: 1.0,
            mix_buffer: AudioBuffer::new(512),
            graph_output: AudioBuffer::new(1024),
            prev_instrument_outputs: std::collections::HashMap::with_capacity(128),
            track_controls: vec![TrackControlSlot::default(); usize::from(u16::MAX) + 1]
                .into_boxed_slice(),
            track_control_generation: 0,
            return_busses: Vec::new(),
            return_index: std::collections::HashMap::new(),
            return_sends: Vec::new(),
            return_order: Vec::new(),
            return_indegree: Vec::new(),
            return_scratch: AudioBuffer::new(512),
            channel_sends: std::collections::HashMap::with_capacity(128),
            metering: MeteringSystem::new(synth_core::SampleRate::DVD_QUALITY),
            sequencer: SequencerEngine::new(synth_core::SampleRate::DVD_QUALITY),
            sequencer_event_buffer: Vec::with_capacity(128),
            recording: crate::recording::RecordingBuffer::new(),
            click_generator: crate::click_generator::ClickGenerator::new(
                synth_core::SampleRate::DVD_QUALITY,
            ),
            pre_record_loop: None,
            pending_recorded_notes: None,
            audio_input_stream: None,
            // Pre-allocate for up to MAX_BLOCK_SIZE stereo (interleaved) frames.
            audio_input_buffer: vec![0.0; synth_core::MAX_BLOCK_SIZE * 2],
            mod_grid: Box::default(),
            mod_grid_scratch: AudioBuffer::new(512),
            grid_master_volume_offset: 0.0,
            midi_cc: Box::default(),
            sustain_pedal_down: [false; 16],
            sustained_notes: Box::new([[false; 128]; 16]),
            mod_grid_trash_producer,
            mod_grid_pending_drop: None,
            deferred_control_drop_producer,
            deferred_drop_slots: Arc::clone(&deferred_drop_slots),
            #[cfg(feature = "rt-profiling")]
            callback_duration_sum: 0.0,
            #[cfg(feature = "rt-profiling")]
            callback_count: 0,
            #[cfg(feature = "rt-profiling")]
            stage_voices_sum: 0.0,
            #[cfg(feature = "rt-profiling")]
            stage_module_graph_sum: 0.0,
            #[cfg(feature = "rt-profiling")]
            stage_master_fx_sum: 0.0,
            #[cfg(feature = "rt-profiling")]
            stage_mod_grid_sum: 0.0,
        };

        // Hand the sequencer the producer end of the automation-trash channel so
        // its transport-reset clears free `ParamId` arcs off the audio thread.
        engine
            .sequencer
            .set_automation_trash(automation_trash_producer);

        let handle = EngineHandle {
            command_sender: CommandSender::new(
                command_producer,
                Arc::clone(&state),
                command_capacity,
                Arc::clone(&deferred_drop_slots),
            ),
            event_consumer,
            return_consumer,
            instrument_return_consumer,
            automation_trash_consumer,
            metadata_trash_consumer,
            script_trash_consumer,
            descriptor_trash_consumer,
            mod_grid_trash_consumer,
            deferred_control_drop_consumer,
            deferred_drop_slots,
            state,
            visualization_buffers: HashMap::new(),
            note_event_consumer: Some(note_event_consumer),
        };

        (engine, handle)
    }

    /// Find an effect slot by its module ID in a specific instrument's effect chain.
    fn find_effect_by_id(
        &mut self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
    ) -> Option<&mut EffectSlot> {
        self.instruments
            .iter_mut()
            .find(|i| i.id() == instrument_id)
            .and_then(|inst| inst.effect_chain_mut().find_effect_by_id(module_id))
    }

    /// Populate a voice graph with the default signal chain.
    ///
    /// This creates a basic subtractive synthesis signal chain:
    /// OSC1 + OSC2 -> Filter -> Amplifier
    /// with envelope modulation.
    #[cfg(test)]
    fn populate_default_voice_graph(graph: &mut ModuleGraph) {
        use synth_core::{
            AmplifierParam, Cents, EnvelopeParam, FilterParam, Hertz, LfoParam, LfoWaveform,
            OscillatorParam, Seconds as TypedSeconds, Waveform,
        };
        use synth_modules::{Amplifier, Envelope, Filter, Lfo, Oscillator};

        // Add oscillators with Spacey Bass preset defaults
        let osc1_id = graph.add_module(Box::new({
            let mut osc = Oscillator::new();
            osc.set_param(Param::Oscillator(OscillatorParam::Waveform(
                Waveform::Sawtooth,
            )));
            osc.set_param(Param::Oscillator(OscillatorParam::Level(Gain::new(0.6))));
            osc
        }));

        let osc2_id = graph.add_module(Box::new({
            let mut osc = Oscillator::new();
            osc.set_param(Param::Oscillator(OscillatorParam::Waveform(
                Waveform::Sawtooth,
            )));
            osc.set_param(Param::Oscillator(OscillatorParam::Level(Gain::new(0.5))));
            osc.set_param(Param::Oscillator(OscillatorParam::Detune(Cents::new(7.0))));
            osc
        }));

        // Add filter
        let filter_id = graph.add_module(Box::new({
            let mut filter = Filter::new();
            filter.set_param(Param::Filter(FilterParam::Cutoff(Hertz::new(400.0))));
            filter.set_param(Param::Filter(FilterParam::Resonance(NormalizedValue::new(
                0.4,
            ))));
            filter
        }));

        // Add amp envelope
        let amp_env_id = graph.add_module(Box::new({
            let mut env = Envelope::new();
            env.set_param(Param::Envelope(EnvelopeParam::Attack(TypedSeconds::new(
                0.005,
            ))));
            env.set_param(Param::Envelope(EnvelopeParam::Decay(TypedSeconds::new(
                0.2,
            ))));
            env.set_param(Param::Envelope(EnvelopeParam::Sustain(
                NormalizedValue::new(0.6),
            )));
            env.set_param(Param::Envelope(EnvelopeParam::Release(TypedSeconds::new(
                0.3,
            ))));
            env
        }));

        // Add filter envelope
        let filter_env_id = graph.add_module(Box::new({
            let mut env = Envelope::new();
            env.set_param(Param::Envelope(EnvelopeParam::Attack(TypedSeconds::new(
                0.001,
            ))));
            env.set_param(Param::Envelope(EnvelopeParam::Decay(TypedSeconds::new(
                0.3,
            ))));
            env.set_param(Param::Envelope(EnvelopeParam::Sustain(
                NormalizedValue::new(0.2),
            )));
            env.set_param(Param::Envelope(EnvelopeParam::Release(TypedSeconds::new(
                0.4,
            ))));
            env
        }));

        // Add LFO
        let _lfo_id = graph.add_module(Box::new({
            let mut lfo = Lfo::new();
            lfo.set_param(Param::Lfo(LfoParam::Rate(Hertz::new(0.3))));
            lfo.set_param(Param::Lfo(LfoParam::Depth(NormalizedValue::new(0.25))));
            lfo.set_param(Param::Lfo(LfoParam::Waveform(LfoWaveform::Sine)));
            lfo
        }));

        // Add amplifier
        let amp_id = graph.add_module(Box::new({
            let mut amp = Amplifier::new();
            amp.set_param(Param::Amplifier(AmplifierParam::Level(Gain::new(0.7))));
            amp
        }));

        // Connect: OSC1 -> Filter
        let _ = graph.connect(osc1_id, "out", filter_id, "in");
        // Connect: OSC2 -> Filter (signals are summed at filter input)
        let _ = graph.connect(osc2_id, "out", filter_id, "in");
        // Connect: Filter -> Amplifier
        let _ = graph.connect(filter_id, "out", amp_id, "in");
        // Connect: Amp Envelope -> Amplifier CV
        let _ = graph.connect(amp_env_id, "out", amp_id, "cv");
        // Connect: Filter Envelope -> Filter Cutoff CV
        let _ = graph.connect(filter_env_id, "out", filter_id, "cutoff_cv");
    }

    /// Process pending commands.
    fn process_commands(&mut self) {
        while let Some(command) = self.command_consumer.try_pop() {
            self.handle_command(command);
            // Publish drain progress *after* handle_command has mirrored its
            // effect into shared_graph, so a reader that waits on this count
            // (see CommandSync) sees the mirrored state too.
            self.state.command_sync.note_processed();
        }
    }

    /// Handle a single command by dispatching to specialized handlers.
    fn handle_command(&mut self, command: EngineCommand) {
        match command {
            // Instrument management
            EngineCommand::AddInstrument { instrument } => {
                self.handle_add_instrument(instrument);
            }
            EngineCommand::RemoveInstrument { instrument_id } => {
                self.handle_remove_instrument(instrument_id);
            }
            EngineCommand::RenameInstrument {
                instrument_id,
                name,
            } => {
                let discarded = if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.replace_name(name)
                } else {
                    name
                };
                Self::trash_metadata(&mut self.metadata_trash_producer, Some(discarded));
            }
            EngineCommand::SetInstrumentDescription {
                instrument_id,
                description,
            } => {
                let discarded = if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.replace_description(description)
                } else {
                    description
                };
                Self::trash_metadata(&mut self.metadata_trash_producer, Some(discarded));
            }
            EngineCommand::SetPatchDescription {
                instrument_id,
                description,
            } => {
                let discarded = if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.replace_patch_description(description)
                } else {
                    description
                };
                Self::trash_metadata(&mut self.metadata_trash_producer, discarded);
            }
            EngineCommand::SetInstrumentColor {
                instrument_id,
                color,
            } => {
                let discarded = if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.replace_color(color)
                } else {
                    color
                };
                Self::trash_metadata(&mut self.metadata_trash_producer, discarded);
            }
            EngineCommand::SetPatchColor {
                instrument_id,
                color,
            } => {
                let discarded = if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.replace_patch_color(color)
                } else {
                    color
                };
                Self::trash_metadata(&mut self.metadata_trash_producer, discarded);
            }
            EngineCommand::SetModuleDescription {
                instrument_id,
                module_id,
                description,
            } => {
                let discarded = if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.replace_module_description(module_id, description)
                } else {
                    description
                };
                Self::trash_metadata(&mut self.metadata_trash_producer, discarded);
            }
            EngineCommand::SetSidechainSource {
                instrument_id,
                source,
            } => {
                // Walk the proposed chain to reject any cycle. Cycles
                // are technically safe (previous-buffer semantics) but
                // produce surprising ducking patterns and obscure user
                // intent, so the engine rejects them outright.
                let creates_cycle = source.is_some_and(|src| {
                    Self::sidechain_chain_contains(&self.instruments, src, instrument_id)
                });
                if !creates_cycle
                    && let Some(inst) = self
                        .instruments
                        .iter_mut()
                        .find(|i| i.id() == instrument_id)
                {
                    inst.set_sidechain_source_id(source);
                }
            }
            EngineCommand::SetInstrumentParameter {
                instrument_id,
                param,
            } => {
                self.handle_set_instrument_param(instrument_id, param);
            }
            EngineCommand::SetInstrumentMidiChannel {
                instrument_id,
                channel,
            } => {
                self.handle_set_instrument_channel(instrument_id, channel);
            }
            EngineCommand::SetInstrumentEnabled {
                instrument_id,
                enabled,
            } => {
                self.handle_set_instrument_enabled(instrument_id, enabled);
            }
            EngineCommand::SetInstrumentCategory {
                instrument_id,
                category,
            } => {
                if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.set_category(category);
                }
            }
            EngineCommand::SetInstrumentSolo {
                instrument_id,
                solo,
            } => {
                self.handle_set_instrument_solo(instrument_id, solo);
            }

            // Return busses (effect sends)
            EngineCommand::CreateReturnBus { id } => {
                self.handle_create_return_bus(id);
            }
            EngineCommand::RemoveReturnBus { id } => {
                self.handle_remove_return_bus(id);
            }
            EngineCommand::ClearReturnBusses => {
                self.return_busses.clear();
                self.return_index.clear();
            }
            EngineCommand::ClearMasterEffects => {
                self.master_effects.clear();
            }
            EngineCommand::AddReturnEffect {
                return_id,
                id,
                effect,
            } => {
                self.handle_add_return_effect(return_id, id, effect);
            }
            EngineCommand::RemoveReturnEffect { return_id, id } => {
                if let Some(bus) = self.return_busses.iter_mut().find(|b| b.id() == return_id) {
                    bus.effect_chain_mut().remove_effect(id);
                }
            }
            EngineCommand::SetReturnEffectParameter {
                return_id,
                module_id,
                param,
            } => {
                if let Some(bus) = self.return_busses.iter_mut().find(|b| b.id() == return_id)
                    && let Some(slot) = bus.effect_chain_mut().find_effect_by_id(module_id)
                {
                    slot.effect.set_param(param);
                    slot.state = crate::effect_chain::EnabledState::Active;
                }
            }
            EngineCommand::SetReturnEffectEnabled {
                return_id,
                module_id,
                enabled,
            } => {
                if let Some(bus) = self.return_busses.iter_mut().find(|b| b.id() == return_id)
                    && let Some(slot) = bus.effect_chain_mut().find_effect_by_id(module_id)
                {
                    slot.state = crate::effect_chain::EnabledState::from(enabled);
                }
            }
            EngineCommand::SetReturnEffectChainOrder { return_id, order } => {
                if let Some(bus) = self.return_busses.iter_mut().find(|b| b.id() == return_id) {
                    bus.effect_chain_mut().set_slot_order(&order);
                }
            }
            EngineCommand::ReorderReturnEffect {
                return_id,
                module_id,
                direction,
            } => {
                if let Some(bus) = self.return_busses.iter_mut().find(|b| b.id() == return_id) {
                    match direction {
                        crate::commands::ReorderDirection::Up => {
                            bus.effect_chain_mut().move_slot_up(module_id);
                        }
                        crate::commands::ReorderDirection::Down => {
                            bus.effect_chain_mut().move_slot_down(module_id);
                        }
                    }
                }
            }

            // Note control
            EngineCommand::NoteOn {
                note,
                velocity,
                channel,
                instrument_id,
            } => {
                self.handle_note_on(note, velocity, channel, instrument_id);
            }
            EngineCommand::NoteOff {
                note,
                channel,
                instrument_id,
            } => {
                self.handle_note_off(note, channel, instrument_id);
            }
            EngineCommand::AllNotesOff => {
                self.handle_all_notes_off();
            }

            EngineCommand::ResetDsp => {
                self.handle_reset_dsp();
            }

            // MIDI controllers
            EngineCommand::PitchBend { value, channel } => {
                self.handle_pitch_bend(value, channel);
                self.push_note_event(NoteEvent::Cc {
                    cc: CcNumber::PITCH_BEND,
                    value: value.to_unipolar(),
                    channel,
                });
            }
            EngineCommand::ModWheel { value, channel } => {
                self.handle_mod_wheel(value, channel);
                // Keep the live CC state in sync so a Mod Grid `MidiCc` source on
                // CC1 sees the mod wheel too (the per-voice path above is separate).
                self.midi_cc
                    .set(channel.as_zero_indexed(), 1, value.as_f32());
                self.push_note_event(NoteEvent::Cc {
                    cc: CcNumber::MOD_WHEEL,
                    value,
                    channel,
                });
            }
            EngineCommand::ControlChange { channel, cc, value } => {
                // Feed the live CC state read by Mod Grid `MidiCc` sources (CC1
                // still arrives as `ModWheel` for the per-voice mod-wheel path,
                // which mirrors into the CC state too).
                self.midi_cc
                    .set(channel.as_zero_indexed(), cc, value.as_f32());
                // CC64 is the sustain pedal: while down, defer NoteOffs; on the
                // down→up edge, release every note held on this channel.
                if cc == CC_SUSTAIN_PEDAL {
                    let ch = usize::from(channel.as_zero_indexed());
                    let down = value.as_f32() >= 0.5;
                    let was_down = self.sustain_pedal_down[ch];
                    self.sustain_pedal_down[ch] = down;
                    if was_down && !down {
                        self.release_sustained_notes(channel);
                    }
                }
            }
            EngineCommand::Aftertouch { value, channel } => {
                self.handle_aftertouch(value, channel);
                self.push_note_event(NoteEvent::Cc {
                    cc: CcNumber::AFTERTOUCH,
                    value,
                    channel,
                });
            }
            EngineCommand::PolyAftertouch {
                note,
                value,
                channel,
            } => {
                self.handle_poly_aftertouch(note, value, channel);
            }

            // Global parameters
            EngineCommand::SetMasterVolume(vol) => {
                self.handle_set_master_volume(vol);
            }
            EngineCommand::SetGlideTime(time) => {
                self.handle_set_glide_time(time);
            }
            EngineCommand::SetFocusedInstrument(instrument_id) => {
                self.handle_set_focused_instrument(instrument_id);
            }

            // Voice/module parameters
            EngineCommand::SetVoiceParameter {
                instrument_id,
                target,
                param,
            } => {
                self.handle_set_voice_param(instrument_id, target, param);
            }
            EngineCommand::SetModuleParameter {
                instrument_id,
                module_id,
                param,
            } => {
                self.handle_set_module_param(instrument_id, module_id, param);
            }
            EngineCommand::SetModScript {
                instrument_id,
                module_id,
                slot,
                script,
                descriptor,
            } => {
                self.handle_set_mod_script(
                    instrument_id,
                    module_id,
                    slot as usize,
                    script,
                    descriptor,
                );
                // Refresh the shared snapshot so the new script is visible to the
                // save path (`ModuleStateSnapshot.scripts`).
            }

            // Reset/clear
            EngineCommand::Reset => {
                self.handle_reset();
            }
            EngineCommand::ClearAllModules => {
                self.handle_clear_all_modules();
            }

            // Effects
            EngineCommand::SetBypass {
                instrument_id,
                module,
                bypass,
            } => {
                self.handle_set_bypass(instrument_id, module, bypass);
            }
            EngineCommand::SetEffectParameter {
                instrument_id,
                module_id,
                param,
            } => {
                self.handle_set_effect_param(instrument_id, module_id, param);
            }
            EngineCommand::SetEffectEnabled {
                instrument_id,
                module_id,
                enabled,
            } => {
                self.handle_set_effect_enabled(instrument_id, module_id, enabled);
            }
            EngineCommand::AddVisualizer {
                instrument_id,
                id,
                visualizer_type,
                buffer,
            } => {
                self.handle_add_visualizer(instrument_id, id, visualizer_type, buffer);
            }
            EngineCommand::RemoveVisualizer { instrument_id, id } => {
                self.handle_remove_visualizer(instrument_id, id);
            }
            EngineCommand::AddEffectInstance {
                instrument_id,
                id,
                effect,
            } => {
                self.handle_add_effect_instance(instrument_id, id, effect);
            }
            EngineCommand::RemoveEffect { instrument_id, id } => {
                self.handle_remove_effect(instrument_id, id);
            }
            EngineCommand::ReorderEffect {
                instrument_id,
                module_id,
                direction,
            } => {
                self.handle_reorder_effect(instrument_id, module_id, direction);
            }
            EngineCommand::SetEffectChainOrder {
                instrument_id,
                order,
            } => {
                self.handle_set_effect_chain_order(instrument_id, &order);
            }

            // Modular routing
            EngineCommand::AddModuleInstance {
                instrument_id,
                id,
                module,
            } => {
                self.handle_add_module_instance(instrument_id, id, module);
            }
            EngineCommand::RemoveModule { instrument_id, id } => {
                self.handle_remove_module(instrument_id, id);
            }
            EngineCommand::Connect {
                instrument_id,
                from,
                to,
            } => {
                self.handle_connect(instrument_id, from, to);
            }
            EngineCommand::Disconnect {
                instrument_id,
                from,
                to,
            } => {
                self.handle_disconnect(instrument_id, from, to);
            }
            EngineCommand::DisconnectAll {
                instrument_id,
                module,
            } => {
                self.handle_disconnect_all(instrument_id, module);
            }

            // Transport control
            EngineCommand::Play => {
                // If recording is armed, start count-in
                if self.recording.state() == crate::recording::RecordingState::Armed {
                    if let Some(seek_to) = self.recording.start_playback() {
                        // Save current loop state before overriding
                        self.pre_record_loop = Some((
                            self.sequencer.loop_start(),
                            self.sequencer.loop_end(),
                            self.sequencer.is_looping(),
                        ));

                        // Set loop around the pattern region
                        if let Some((region_start, pattern_length)) = self.recording.target_info() {
                            let loop_end =
                                synth_sequencer::Tick(region_start.0 + pattern_length.0 as u64);
                            self.sequencer.set_loop(region_start, loop_end, true);
                            self.sync_loop_to_transport();
                        }

                        // Enable metronome during count-in
                        self.click_generator.set_enabled(true);
                        self.sequencer.play();
                        self.sequencer.seek_without_events(seek_to);
                        self.state.transport.set_playing(true);
                        self.state.transport.set_ticks(seek_to.0);
                        self.state
                            .transport
                            .set_recording_state(self.recording.state());
                    }
                } else {
                    self.sequencer.set_solo_pattern(None);
                    self.clear_preview();
                    self.sequencer.play();
                    self.state.transport.set_playing(true);
                    // Playback starts at the cursor when coming from a stop;
                    // surface that position immediately rather than next block.
                    self.state
                        .transport
                        .set_ticks(self.sequencer.current_tick().0);
                }
            }
            EngineCommand::Stop => {
                // Disarm/flush any active recording. Covers Armed too (not just
                // Capturing/CountIn): a Stop pressed while only armed must clear
                // the armed state instead of leaving recording stuck Armed.
                if self.recording.state() != crate::recording::RecordingState::Idle {
                    let pattern_id = self.recording.target_pattern();
                    let overdub = self.recording.is_overdub();
                    let notes = self.recording.disarm();
                    if let Some(pid) = pattern_id
                        && !notes.is_empty()
                    {
                        self.flush_recorded_notes(pid, notes, overdub);
                    }
                    self.state
                        .transport
                        .set_recording_state(RecordingState::Idle);
                    // Restore metronome to shared state
                    self.click_generator
                        .set_enabled(self.state.transport.is_metronome_on());
                    // Restore loop state from before recording
                    self.restore_pre_record_loop();
                }

                self.sequencer.set_solo_pattern(None);
                self.clear_preview();
                self.sequencer.stop_without_events();
                self.state.transport.set_playing(false);
                // Stop returns the playhead to the cursor (or to 0 on a second
                // press); mirror whatever position the sequencer settled on.
                self.state
                    .transport
                    .set_ticks(self.sequencer.current_tick().0);

                // Release all voices on all instruments and revert any
                // automation overrides to their base values.
                for instrument in &mut self.instruments {
                    instrument.all_notes_off();
                }
                self.clear_all_param_overrides();
            }
            EngineCommand::Pause => {
                self.sequencer.pause();
                self.state.transport.set_playing(false);
            }
            EngineCommand::Rewind => {
                self.sequencer
                    .seek_without_events(synth_sequencer::Tick::ZERO);
                self.sequencer.set_cursor(synth_sequencer::Tick::ZERO);
                self.state.transport.set_ticks(0);
            }
            EngineCommand::Seek { tick } => {
                // A seek marks the cursor too: this is the position Play starts
                // from and Stop returns to. Clicking the ruler is how the user
                // places the cursor.
                self.sequencer.seek_without_events(tick);
                self.sequencer.set_cursor(tick);
                self.state.transport.set_ticks(tick.0);
            }
            EngineCommand::SetLoop {
                start,
                end,
                enabled,
            } => {
                self.sequencer.set_loop(start, end, enabled);
                self.sync_loop_to_transport();
            }
            EngineCommand::SetRepeat { enabled } => {
                self.sequencer.set_repeat_song(enabled);
                self.sync_loop_to_transport();
            }
            EngineCommand::PlayPattern {
                pattern_id,
                instrument,
            } => {
                // Find pattern in arrangement and get boundaries
                let bounds =
                    Self::find_pattern_bounds(&self.sequencer.song().snapshot(), pattern_id);

                if let Some((start, end)) = bounds {
                    // Placed pattern — play through the arrangement at this region.
                    self.clear_preview();
                    self.sequencer.play();
                    self.sequencer.seek_without_events(start);
                    self.sequencer.set_loop(start, end, true);
                    self.sync_loop_to_transport();
                    self.state.transport.set_playing(true);
                    self.state.transport.set_ticks(start.0);
                } else {
                    // Orphan pattern — enter preview-mode and loop at pattern.length.
                    let length = self
                        .sequencer
                        .song()
                        .snapshot()
                        .pattern(pattern_id)
                        .map(|p| p.length);
                    if let Some(length) = length {
                        // max(1): a zero-length pattern would otherwise make the
                        // loop [0,0) wrap every tick and pin the playhead at 0.
                        let loop_end = synth_sequencer::Tick(u64::from(length.0.max(1)));
                        self.sequencer
                            .set_preview_pattern(Some((pattern_id, instrument)));
                        self.state.transport.set_preview_pattern(Some(pattern_id));
                        self.sequencer.play();
                        self.sequencer
                            .seek_without_events(synth_sequencer::Tick::ZERO);
                        self.sequencer
                            .set_loop(synth_sequencer::Tick::ZERO, loop_end, true);
                        self.sync_loop_to_transport();
                        self.state.transport.set_playing(true);
                        self.state.transport.set_ticks(0);
                    } else {
                        // Pattern not found or the song lock was momentarily
                        // unavailable. Fall back to plain playback (matching the
                        // pre-orphan behavior) instead of a silent no-op, and
                        // make sure we don't leave a stale preview target set.
                        self.clear_preview();
                        self.sequencer.play();
                        // play() now starts at the cursor; the pre-orphan
                        // fallback played from the start, so seek to 0.
                        self.sequencer
                            .seek_without_events(synth_sequencer::Tick::ZERO);
                        self.state.transport.set_playing(true);
                        self.state.transport.set_ticks(0);
                    }
                }
            }
            EngineCommand::PlayFromPattern { pattern_id } => {
                // Start playback from pattern start, no loop
                let bounds =
                    Self::find_pattern_bounds(&self.sequencer.song().snapshot(), pattern_id);

                if let Some((start, _end)) = bounds {
                    // Important: play() first to avoid it resetting current_tick to 0
                    self.sequencer.play();
                    self.sequencer.seek_without_events(start);
                    self.sequencer.set_loop(
                        synth_sequencer::Tick::ZERO,
                        synth_sequencer::Tick::ZERO,
                        false,
                    );
                    self.sync_loop_to_transport();
                    self.state.transport.set_playing(true);
                    self.state.transport.set_ticks(start.0);
                } else {
                    // Fallback: pattern not in arrangement, play from the start.
                    // play() now begins at the cursor, so seek to 0 explicitly to
                    // honor the "from beginning" intent.
                    self.sequencer.play();
                    self.sequencer
                        .seek_without_events(synth_sequencer::Tick::ZERO);
                    self.state.transport.set_playing(true);
                    self.state.transport.set_ticks(0);
                }
            }
            EngineCommand::SetSoloPattern(pattern) => {
                // Solo and orphan-preview are mutually exclusive playback modes
                // (the preview branch in collect_events_at_tick ignores solo).
                // Entering solo must leave preview so the solo toggle isn't a
                // dead control. Clearing solo (None) leaves preview untouched.
                if pattern.is_some() {
                    self.clear_preview();
                }
                self.sequencer.set_solo_pattern(pattern);
            }
            EngineCommand::SetPreviewPattern(preview) => {
                if preview.is_some() {
                    self.sequencer.set_solo_pattern(None);
                }
                self.sequencer.set_preview_pattern(preview);
                // Transport mirror is playhead-only — pattern id without instrument.
                self.state
                    .transport
                    .set_preview_pattern(preview.map(|(pattern_id, _)| pattern_id));
            }
            EngineCommand::SetSong { song } => {
                let previous = self.sequencer.set_song(song);
                self.defer_control_drop(DeferredControlDrop::Song(previous));
                // The new song's pattern ids are unrelated to the old one's;
                // a stale orphan-preview target must not survive the swap.
                self.clear_preview();
            }

            EngineCommand::SetModGrid { runtime } => {
                // Swap in the pre-built runtime as a plain `Box` pointer move — no
                // heap op on the audio thread (the incoming box is not deref-moved,
                // and the old box is pushed to the trash channel as-is, not re-
                // boxed). Its `Box<dyn PolyModule>` DSP frees off the audio thread
                // when the main thread drains that channel. If the channel is full
                // (never, in practice) the old box drops here — a rare, bounded
                // cost on a user-paced swap.
                let old = std::mem::replace(&mut self.mod_grid, runtime);
                // First flush any previously-parked box into the channel, then hand
                // over `old`. On a full channel the box is parked in the single-slot
                // backstop instead of being dropped here — no DSP is ever destructed
                // on the audio thread. (The offset accumulator slots were pre-keyed
                // in the runtime off the audio thread by the builder, so nothing to
                // insert here either.)
                if let Some(pending) = self.mod_grid_pending_drop.take()
                    && let Err(back) = self.mod_grid_trash_producer.try_push(pending)
                {
                    self.mod_grid_pending_drop = Some(back);
                }
                if let Err(back) = self.mod_grid_trash_producer.try_push(old) {
                    if self.mod_grid_pending_drop.is_none() {
                        // Common overflow: park it, flushed on the next swap.
                        self.mod_grid_pending_drop = Some(back);
                    } else {
                        // Degenerate double-overflow (both the channel and the
                        // single backstop are full) — only reachable if the main
                        // thread stopped draining entirely. Dropping here is the
                        // bounded last resort; a 1-deep slot can't hold two.
                        drop(back);
                    }
                }
            }

            // Recording commands
            EngineCommand::ArmRecord {
                pattern_id,
                track_id,
                region_start,
                pattern_length,
                ticks_per_bar,
                quantize_grid,
                overdub,
            } => {
                self.recording.arm(
                    pattern_id,
                    track_id,
                    region_start,
                    pattern_length,
                    ticks_per_bar,
                    overdub,
                );
                self.recording.set_quantize_grid(quantize_grid);
                self.state
                    .transport
                    .set_recording_state(self.recording.state());
            }
            EngineCommand::DisarmRecord => {
                let pattern_id = self.recording.target_pattern();
                let overdub = self.recording.is_overdub();
                let notes = self.recording.disarm();
                if let Some(pid) = pattern_id
                    && !notes.is_empty()
                {
                    self.flush_recorded_notes(pid, notes, overdub);
                }
                self.state
                    .transport
                    .set_recording_state(RecordingState::Idle);
                // Restore loop state from before recording
                self.restore_pre_record_loop();
                // Disarming an orphan recording must also leave preview-mode;
                // otherwise the engine keeps bypassing the arrangement and
                // looping the single pattern after recording has ended.
                self.clear_preview();
            }
            EngineCommand::SetMetronome(enabled) => {
                self.click_generator.set_enabled(enabled);
                self.state.transport.set_metronome(enabled);
            }
            EngineCommand::SetMetronomeVolume(vol) => {
                self.click_generator.set_volume(vol);
            }

            EngineCommand::SetTempo(bpm) => {
                self.state.transport.set_tempo(bpm);
            }

            EngineCommand::SetAudioInputConsumer {
                consumer,
                sample_rate,
            } => {
                let replacement = AudioInputStream::new(consumer, sample_rate);
                if let Some(previous) = self.audio_input_stream.replace(replacement) {
                    self.defer_control_drop(DeferredControlDrop::AudioInput(previous));
                } else {
                    self.deferred_drop_slots.release();
                }
            }
            EngineCommand::ClearAudioInputConsumer => {
                if let Some(previous) = self.audio_input_stream.take() {
                    self.defer_control_drop(DeferredControlDrop::AudioInput(previous));
                } else {
                    self.deferred_drop_slots.release();
                }
            }
            EngineCommand::LoadSampleData {
                instrument_id,
                module_id,
                data,
                channels,
                frame_count,
                root_note,
            } => {
                // Load sample data into the Sampler module in template + all active voices
                if let Some(instrument) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    // Template graph
                    if let Some(module) = instrument.voice_graph_mut().get_module_mut(module_id) {
                        module.load_sample_data(data.clone(), channels, frame_count, root_note);
                    }
                    // Active voices
                    for voice in instrument.allocator_mut().voices_mut() {
                        if let Some(module) = voice.graph.get_module_mut(module_id) {
                            module.load_sample_data(data.clone(), channels, frame_count, root_note);
                        }
                    }
                }
            }
        }
    }

    // ========================================================================
    // Pattern navigation helpers
    // ========================================================================

    /// Find the start and end tick for a pattern's first occurrence in the arrangement.
    /// Walk the sidechain chain starting at `start`, following
    /// `sidechain_source_id` on each visited instrument, and return
    /// true if `needle` is reachable. Bounded by the number of
    /// instruments so a corrupted chain cannot loop forever.
    fn sidechain_chain_contains(
        instruments: &[Box<Instrument>],
        start: InstrumentId,
        needle: InstrumentId,
    ) -> bool {
        let mut current = Some(start);
        for _ in 0..=instruments.len() {
            let Some(id) = current else {
                return false;
            };
            if id == needle {
                return true;
            }
            current = instruments
                .iter()
                .find(|i| i.id() == id)
                .and_then(|i| i.sidechain_source_id());
        }
        // Bound exhausted — there's a cycle but it doesn't include
        // `needle`. Returning false here is safe: the caller is only
        // asking whether *this* node would create a new cycle.
        false
    }

    fn find_pattern_bounds(
        song: &synth_sequencer::Song,
        pattern_id: synth_sequencer::PatternId,
    ) -> Option<(synth_sequencer::Tick, synth_sequencer::Tick)> {
        for placement in song.arrangement() {
            if placement.pattern_id == pattern_id
                && let Some(pattern) = song.pattern(pattern_id)
            {
                return Some((placement.start, placement.end(pattern.length)));
            }
        }
        None
    }

    // ========================================================================
    // Instrument management handlers
    // ========================================================================

    fn handle_add_instrument(&mut self, mut instrument: Box<Instrument>) {
        self.prev_instrument_outputs
            .insert(instrument.id(), instrument.take_sidechain_output());
        self.channel_sends.insert(instrument.id(), ArrayVec::new());
        self.instruments.push(instrument);
    }

    fn handle_remove_instrument(&mut self, instrument_id: InstrumentId) {
        if let Some(idx) = self
            .instruments
            .iter()
            .position(|p| p.id() == instrument_id)
        {
            let mut instrument = self.instruments.swap_remove(idx);
            // Drop this instrument's sidechain cache.
            if let Some(output) = self.prev_instrument_outputs.remove(&instrument_id) {
                instrument.restore_sidechain_output(output);
            }
            self.channel_sends.remove(&instrument_id);
            // Clear any sidechain references that pointed at this id.
            for inst in &mut self.instruments {
                if inst.sidechain_source_id() == Some(instrument_id) {
                    inst.set_sidechain_source_id(None);
                }
            }
            let _ = self.instrument_return_producer.try_push(instrument);
        }
    }

    // ========================================================================
    // Return-bus (effect-send) management handlers
    // ========================================================================

    /// Rebuild the `ReturnBusId` → index map from `return_busses`. Cheap (few
    /// busses) and only called on return-bus mutations, never the hot path.
    fn rebuild_return_index(&mut self) {
        self.return_index.clear();
        for (idx, bus) in self.return_busses.iter().enumerate() {
            self.return_index.insert(bus.id(), idx);
        }
    }

    fn handle_create_return_bus(&mut self, id: ReturnBusId) {
        // Re-using an existing id is a no-op so a project load is idempotent.
        if self.return_busses.iter().any(|b| b.id() == id) {
            return;
        }
        self.return_busses.push(ReturnBusChannel::new(id));
        self.rebuild_return_index();
    }

    fn handle_remove_return_bus(&mut self, id: ReturnBusId) {
        if let Some(idx) = self.return_busses.iter().position(|b| b.id() == id) {
            self.return_busses.remove(idx);
        }
        // Stale send taps resolve to no destination on the next
        // `update_track_controls` and are simply dropped.
        self.rebuild_return_index();
    }

    fn handle_add_return_effect(
        &mut self,
        return_id: ReturnBusId,
        id: ModuleId,
        effect: Box<dyn synth_core::AudioEffect>,
    ) {
        if let Some(bus) = self.return_busses.iter_mut().find(|b| b.id() == return_id) {
            bus.effect_chain_mut()
                .add_effect(id, effect, SampleRate::new(self.sample_rate));
        }
    }

    fn handle_set_instrument_param(
        &mut self,
        instrument_id: InstrumentId,
        param: crate::commands::InstrumentParam,
    ) {
        use crate::commands::InstrumentParam;

        let Some(instrument) = self
            .instruments
            .iter_mut()
            .find(|p| p.id() == instrument_id)
        else {
            return;
        };

        match param {
            InstrumentParam::Volume(vol) => instrument.set_volume(vol),
            InstrumentParam::Pan(pan) => instrument.set_pan(pan),
            InstrumentParam::GlideTime(time) => instrument.allocator_mut().set_glide_time(time),
            InstrumentParam::AllocationMode(mode) => instrument.allocator_mut().set_mode(mode),
            InstrumentParam::StealingStrategy(strategy) => {
                instrument.allocator_mut().set_stealing(strategy)
            }
            InstrumentParam::UnisonDetune(detune) => {
                instrument.allocator_mut().set_unison_detune(detune)
            }
            InstrumentParam::UnisonSpread(spread) => {
                instrument.allocator_mut().set_unison_spread(spread)
            }
            InstrumentParam::MaxVoices(count) => {
                // Stored in config only — no live resize (that would allocate on
                // the audio thread). Takes effect on the next voice-graph
                // reconstruction (e.g. project load). See `set_max_voices`.
                instrument.allocator_mut().set_max_voices(count);
            }
            InstrumentParam::VelocityAmpSensitivity(sens) => {
                instrument.set_velocity_amp_sensitivity(sens);
            }
            InstrumentParam::VelocityFilterSensitivity(sens) => {
                instrument.set_velocity_filter_sensitivity(sens);
            }
            InstrumentParam::Solo(solo) => instrument.set_solo(solo),
            InstrumentParam::KeyRange(range) => instrument.set_key_range(range),
            InstrumentParam::Transpose(semitones) => instrument.set_transpose(semitones),
            InstrumentParam::LearnState(state) => instrument.set_learn_state(state),
            InstrumentParam::OversamplingFactor(factor) => instrument.set_oversampling(factor),
        }
    }

    fn handle_set_instrument_channel(
        &mut self,
        instrument_id: InstrumentId,
        channel: MidiChannelSelection,
    ) {
        if let Some(instrument) = self
            .instruments
            .iter_mut()
            .find(|p| p.id() == instrument_id)
        {
            instrument.set_midi_channel(channel);
        }
    }

    fn handle_set_instrument_enabled(&mut self, instrument_id: InstrumentId, enabled: bool) {
        if let Some(instrument) = self
            .instruments
            .iter_mut()
            .find(|p| p.id() == instrument_id)
        {
            instrument.set_enabled(enabled);
        }
    }

    fn handle_set_instrument_solo(&mut self, instrument_id: InstrumentId, solo: bool) {
        if let Some(instrument) = self
            .instruments
            .iter_mut()
            .find(|p| p.id() == instrument_id)
        {
            instrument.set_solo(solo);
        }
    }

    // ========================================================================
    // Note control handlers
    // ========================================================================

    fn handle_note_on(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        channel: MidiChannelSelection,
        explicit_instrument: Option<InstrumentId>,
    ) {
        let channel_raw = channel.as_zero_indexed();
        let mut note_triggered = false;

        // A re-press reclaims a pedal-held note: drop any deferred release so
        // lifting the pedal won't cut off the newly-struck note.
        self.sustained_notes[usize::from(channel_raw)][usize::from(note.as_u8())] = false;

        // Check if there's a focused instrument for keyboard input (by InstrumentId)
        let focused_id = self.state.get_focused_instrument();

        for instrument in self.instruments.iter_mut() {
            if explicit_instrument.is_some_and(|id| instrument.id() != id) {
                continue;
            }
            // If focused instrument is set, only that instrument receives keyboard input
            // (Channel 0 is the default keyboard channel)
            if explicit_instrument.is_none()
                && let Some(focus_id) = focused_id
            {
                // Only send to focused instrument, and only for channel 0 (keyboard)
                if channel_raw == 0 && instrument.id() != focus_id {
                    continue;
                }
                // For other channels (e.g., external MIDI), use traditional routing
                if channel_raw != 0 && !instrument.responds_to_channel(channel_raw) {
                    continue;
                }
            } else if explicit_instrument.is_none() {
                // Traditional MIDI channel routing
                if !instrument.responds_to_channel(channel_raw) {
                    continue;
                }
            }

            // Check learn mode
            if instrument.handle_note_learn(note) {
                let _ = self.event_producer.try_push(EngineEvent::KeyRangeLearned {
                    instrument_id: instrument.id(),
                    key_range: instrument.key_range(),
                    learn_state: instrument.learn_state(),
                });
                continue;
            }

            if instrument.note_on(note, velocity).is_some() {
                note_triggered = true;
                if self
                    .note_event_producer
                    .try_push(NoteEvent::On {
                        note,
                        velocity,
                        instrument_id: instrument.id(),
                        category: instrument.category(),
                    })
                    .is_err()
                {
                    self.state
                        .event_drops
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }

        if explicit_instrument.is_none() && self.use_modular_routing {
            self.module_graph.note_on(note, velocity);
            note_triggered = true;
        }

        // Capture note for recording (after instrument routing so sound plays immediately)
        if self.recording.state() == crate::recording::RecordingState::Capturing
            && let Some(pitch) = synth_sequencer::Pitch::new(note.as_u8())
        {
            self.recording
                .note_on(pitch, velocity, self.sequencer.current_tick());
        }

        if note_triggered {
            let _ = self.event_producer.try_push(EngineEvent::NoteTriggered {
                note,
                velocity,
                channel,
            });
        }
    }

    fn handle_note_off(
        &mut self,
        note: MidiNote,
        channel: MidiChannelSelection,
        explicit_instrument: Option<InstrumentId>,
    ) {
        let channel_raw = channel.as_zero_indexed();

        // Sustain pedal held on this channel: defer the release until the pedal
        // lifts (a re-press reclaims the note in `handle_note_on`). Skip the whole
        // release — the note is still sounding, so no NoteOff event / recording.
        if self.sustain_pedal_down[usize::from(channel_raw)] {
            self.sustained_notes[usize::from(channel_raw)][usize::from(note.as_u8())] = true;
            return;
        }

        let focused_id = self.state.get_focused_instrument();

        for instrument in self.instruments.iter_mut() {
            if explicit_instrument.is_some_and(|id| instrument.id() != id) {
                continue;
            }
            // Same logic as note_on for focused instrument routing
            if explicit_instrument.is_none()
                && let Some(focus_id) = focused_id
            {
                if channel_raw == 0 && instrument.id() != focus_id {
                    continue;
                }
                if channel_raw != 0 && !instrument.responds_to_channel(channel_raw) {
                    continue;
                }
            } else if explicit_instrument.is_none() && !instrument.responds_to_channel(channel_raw)
            {
                continue;
            }
            instrument.note_off(note);
            if self
                .note_event_producer
                .try_push(NoteEvent::Off {
                    note,
                    instrument_id: instrument.id(),
                    category: instrument.category(),
                })
                .is_err()
            {
                self.state
                    .event_drops
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        if explicit_instrument.is_none() && self.use_modular_routing {
            self.module_graph.note_off();
        }

        // Capture note-off for recording
        if self.recording.state() == crate::recording::RecordingState::Capturing
            && let Some(pitch) = synth_sequencer::Pitch::new(note.as_u8())
        {
            self.recording
                .note_off(pitch, self.sequencer.current_tick());
        }

        let _ = self
            .event_producer
            .try_push(EngineEvent::NoteReleased { note, channel });
    }

    /// Push a note event to the OSC telemetry ring buffer, tracking drops.
    fn push_note_event(&mut self, event: NoteEvent) {
        if self.note_event_producer.try_push(event).is_err() {
            self.state
                .event_drops
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn handle_all_notes_off(&mut self) {
        for instrument in &mut self.instruments {
            instrument.all_notes_off();
        }
        if self.use_modular_routing {
            self.module_graph.reset();
        }
        // A panic / stop clears any pedal hold too — nothing is left deferred.
        *self.sustained_notes = [[false; 128]; 16];
        self.sustain_pedal_down = [false; 16];
        // Transport stop reverts automation overrides to their base values.
        self.clear_all_param_overrides();

        let _ = self.event_producer.try_push(EngineEvent::AllNotesReleased);
    }

    /// Release every note held by the sustain pedal on `channel` (called on the
    /// pedal's down→up edge). Clears each held bit *before* the real release — the
    /// pedal is already up, so `handle_note_off` won't re-defer. RT-safe: a fixed
    /// 128-entry scan, no allocation.
    fn release_sustained_notes(&mut self, channel: MidiChannelSelection) {
        let ch = usize::from(channel.as_zero_indexed());
        for note in 0u8..128 {
            if self.sustained_notes[ch][usize::from(note)] {
                self.sustained_notes[ch][usize::from(note)] = false;
                self.handle_note_off(MidiNote::new(note), channel, None);
            }
        }
    }

    /// Hand retired pointer-owned state to the control thread. A permit was
    /// reserved before the command entered the command ring, so this push cannot
    /// fail unless the reservation invariant is broken.
    fn defer_control_drop(&mut self, retired: DeferredControlDrop) {
        if let Err(retired) = self.deferred_control_drop_producer.try_push(retired) {
            debug_assert!(false, "reserved deferred-drop slot was unavailable");
            // Preserve RT safety even if a future change breaks the reservation
            // invariant: leaking is preferable to deallocating on the audio
            // thread. The debug assertion makes the defect loud in tests.
            std::mem::forget(retired);
            self.deferred_drop_slots.release();
        }
    }

    /// Hard-reset the per-instrument signal path's DSP state to silence instantly
    /// — every instrument's voices (envelopes/filters/oscillator phase), effect
    /// chain, and oversampling downsamplers, plus the master effect chain, every
    /// return bus's effect chain, and the modular graph. Unlike
    /// `handle_all_notes_off` (which only releases and lets tails ring), this
    /// zeroes delay lines and reverb buffers so a subsequent render starts from a
    /// clean slate — tail-proof isolation between offline renders. Real-time safe:
    /// only touches pre-allocated DSP buffers, no alloc/lock.
    ///
    /// Not reset (out of the offline single-instrument render path): the
    /// one-block sidechain previous-output buffer (it self-heals after one
    /// block, and the offline render uses no sidechain).
    fn handle_reset_dsp(&mut self) {
        for instrument in &mut self.instruments {
            instrument.reset_dsp();
        }
        self.master_effects.reset();
        for bus in &mut self.return_busses {
            bus.effect_chain_mut().reset();
        }
        if self.use_modular_routing {
            self.module_graph.reset();
        }
    }

    /// Revert every transient automation override (instruments + modular graph)
    /// to its base value. Invoked on any transport stop. Real-time safe.
    fn clear_all_param_overrides(&mut self) {
        for instrument in &mut self.instruments {
            instrument.clear_param_overrides();
        }
        if self.use_modular_routing {
            self.module_graph.clear_param_overrides();
        }
    }

    // ========================================================================
    // Recording helpers
    // ========================================================================

    /// Send recorded notes to the UI thread for writing into the target pattern.
    ///
    /// This avoids taking RwLock::write() on the audio thread.
    /// The `pattern_id` must be captured **before** calling `disarm()`, which clears the target.
    fn flush_recorded_notes(
        &mut self,
        pattern_id: synth_sequencer::PatternId,
        notes: Vec<crate::recording::RecordedNote>,
        overdub: bool,
    ) {
        if let Err(EngineEvent::RecordedNotesFlushed {
            pattern_id,
            notes,
            overdub,
        }) = self
            .event_producer
            .try_push(EngineEvent::RecordedNotesFlushed {
                pattern_id,
                notes,
                overdub,
            })
        {
            // Ring buffer full — save for retry on next process cycle
            // to avoid dropping the Vec on the audio thread.
            self.pending_recorded_notes = Some((pattern_id, notes, overdub));
        }
    }

    /// Restore loop state saved before recording started.
    fn restore_pre_record_loop(&mut self) {
        if let Some((start, end, enabled)) = self.pre_record_loop.take() {
            self.sequencer.set_loop(start, end, enabled);
            self.sync_loop_to_transport();
        }
    }

    /// Mirror the current sequencer loop region into `TransportState` so the
    /// GUI ruler markers and MCP `get_song_info` can observe it without
    /// touching the audio thread.
    fn sync_loop_to_transport(&self) {
        self.state.transport.set_loop_state(
            self.sequencer.loop_start(),
            self.sequencer.loop_end(),
            self.sequencer.is_looping(),
        );
    }

    // ========================================================================
    // MIDI controller handlers
    // ========================================================================

    /// Push per-track pitch overrides (`TrackParam::Pitch`) onto the active
    /// voices — the pitch sibling of [`Self::update_track_controls`]. Each
    /// voice carries the `TrackId` of the placement that spawned it, so pitch
    /// lands per voice (two tracks layering one instrument stay independent)
    /// and preview/live voices (no tag) sit at zero offset. The unconditional
    /// overwrite is load-bearing: it is what returns voices to 0 st when
    /// `track_auto` empties at a transport reset — do not add an is-empty
    /// early-out. RT-safe: read-only map lookups, no allocation.
    fn update_track_pitch(&mut self) {
        let track_auto = self.sequencer.track_auto();
        for instrument in &mut self.instruments {
            for voice in instrument.allocator_mut().voices_mut() {
                if !voice.is_active() {
                    continue;
                }
                let lane_pitch = voice
                    .track
                    .and_then(|t| track_auto.get(&t))
                    .and_then(|o| o.pitch)
                    .unwrap_or(Semitones::ZERO);
                // Add the Mod Grid's per-track pitch offset (semitones) on top of
                // the lane pitch — additive, and cleared each block so removing
                // the routing returns the voice to the lane/zero value.
                let grid_pitch = voice
                    .track
                    .and_then(|t| self.mod_grid.track_offsets.get(&t))
                    .map_or(Semitones::ZERO, |o| o.pitch);
                voice.track_pitch = lane_pitch + grid_pitch;
            }
        }
    }

    /// Mod-grid pre-pass: process every running instance's control-rate DSP once
    /// for this block and accumulate its routings as additive, block-constant
    /// offsets into `grid_track_offsets` / `grid_master_volume_offset`. Runs
    /// before instruments and before track-control composition.
    ///
    /// RT-safe: the runtime is pre-built (no allocation here — `mem::take` swaps
    /// with an empty `Vec`), no locks. Offsets are cleared and recomputed every
    /// block, so removing a routing returns its target to base with no latch.
    fn process_mod_grid(&mut self, context: &ProcessContext<'_>) {
        // Reset this block's accumulators unconditionally (load-bearing: an
        // emptied routing must fall back to base, like `update_track_pitch`).
        self.grid_master_volume_offset = 0.0;
        self.mod_grid.reset_offsets();
        if self.mod_grid.is_empty() {
            return;
        }
        // A module offset is applied here but *cleared* by the target voice's own
        // `process_audio`. An instrument that won't be processed this block (muted,
        // or not soloed while another is) would never clear it, so the additive
        // offset would latch and grow — mirror the two skip gates in `process()`
        // (mute inside `Instrument::process`, solo in the instrument loop) and
        // never write into an instrument that won't run this block.
        let any_soloed = self.instruments.iter().any(|i| i.is_solo());
        // Move the instances out to free the `self` borrow, so the DSP scratch
        // and the offset accumulators (disjoint fields) are freely reachable.
        // Block-size-independent audio-tap follower coefficient (once per block).
        let tap = crate::mod_grid::tap_coeff(context.samples, context.sample_rate);
        let mut instances = std::mem::take(&mut self.mod_grid.instances);
        for instance in &mut instances {
            // Cheap→module input injections: feed a Macro/Transport/AudioTap value
            // into a hosted module's control input *before* the graph processes.
            for i in 0..instance.injections.len() {
                let inj = &mut instance.injections[i];
                let (module, port) = (inj.module, inj.port);
                let value = match &inj.source {
                    crate::mod_grid::ModSource::Constant(v) => *v,
                    crate::mod_grid::ModSource::Transport(kind) => {
                        crate::mod_grid::transport_value(*kind, context)
                    }
                    crate::mod_grid::ModSource::InstrumentLevel(id) => {
                        let level = self
                            .instruments
                            .iter()
                            .find(|inst| inst.id() == *id)
                            .map_or(0.0, |inst| {
                                crate::mod_grid::buffer_level(inst.last_output_interleaved())
                            });
                        inj.follow(level, tap)
                    }
                    crate::mod_grid::ModSource::MasterLevel => {
                        let level = crate::mod_grid::buffer_level(self.mix_buffer.as_slice());
                        inj.follow(level, tap)
                    }
                    crate::mod_grid::ModSource::MidiCc { cc, channel } => {
                        self.midi_cc.get(*cc, *channel)
                    }
                    // A Dsp source is a real DSP cable, never an injection.
                    crate::mod_grid::ModSource::Dsp(..) => 0.0,
                };
                instance.dsp.set_input_injection_value(module, port, value);
            }
            instance.dsp.process(&mut self.mod_grid_scratch, context);
            for routing in &mut instance.targets {
                let cv = match &routing.source {
                    None => continue,
                    Some(crate::mod_grid::ModSource::Dsp(module, port)) => {
                        crate::mod_grid::read_source_value(
                            instance.dsp.get_module_output(*module, *port),
                        )
                    }
                    Some(crate::mod_grid::ModSource::Constant(v)) => *v,
                    Some(crate::mod_grid::ModSource::Transport(kind)) => {
                        crate::mod_grid::transport_value(*kind, context)
                    }
                    Some(crate::mod_grid::ModSource::InstrumentLevel(id)) => {
                        let level = self
                            .instruments
                            .iter()
                            .find(|i| i.id() == *id)
                            .map_or(0.0, |i| {
                                crate::mod_grid::buffer_level(i.last_output_interleaved())
                            });
                        routing.follow(level, tap)
                    }
                    Some(crate::mod_grid::ModSource::MasterLevel) => {
                        let level = crate::mod_grid::buffer_level(self.mix_buffer.as_slice());
                        routing.follow(level, tap)
                    }
                    Some(crate::mod_grid::ModSource::MidiCc { cc, channel }) => {
                        self.midi_cc.get(*cc, *channel)
                    }
                };
                let contribution = cv * routing.amount;
                if contribution == 0.0 {
                    continue;
                }
                match &routing.target {
                    AutomationTarget::Global(GlobalParam::MasterVolume) => {
                        self.grid_master_volume_offset += contribution;
                    }
                    AutomationTarget::Track {
                        track: Some(track),
                        param,
                    } => {
                        // `get_mut` only — the slot was pre-keyed in the runtime
                        // off the audio thread, so this never allocates. A missing
                        // slot is skipped.
                        if let Some(entry) = self.mod_grid.track_offsets.get_mut(track) {
                            match param {
                                TrackParam::Volume => entry.volume += contribution,
                                TrackParam::Pan => entry.pan += contribution,
                                TrackParam::Pitch => {
                                    entry.pitch =
                                        Semitones::new(entry.pitch.as_f32() + contribution);
                                }
                                // Mute is excluded from grid targets initially.
                                TrackParam::Mute => {}
                            }
                        }
                    }
                    // A module-param target: apply the offset additively to every
                    // active voice of the owning instrument (address interned
                    // off-thread as `dest_addr`; the voice clears it same-block).
                    AutomationTarget::Module { instrument, .. } => {
                        if let Some(addr) = routing.dest_addr {
                            self.apply_grid_module_offset(
                                *instrument,
                                addr,
                                contribution,
                                any_soloed,
                            );
                        }
                    }
                    // An instrument target: a module-backed param (FilterCutoff,
                    // ADSR, …) carries a `dest_addr` and uses the same per-voice
                    // path; Volume/Pan are channel-level and accumulate into the
                    // per-instrument offset (keyed by `InstrumentId`) folded
                    // in at the channel-bus stage.
                    AutomationTarget::Instrument { instrument, param } => {
                        if let Some(addr) = routing.dest_addr {
                            self.apply_grid_module_offset(
                                *instrument,
                                addr,
                                contribution,
                                any_soloed,
                            );
                        } else if let Some(entry) =
                            self.mod_grid.instrument_offsets.get_mut(instrument)
                        {
                            match param {
                                AutoInstrumentParam::Volume => entry.volume += contribution,
                                AutoInstrumentParam::Pan => entry.pan += contribution,
                                // Module-backed params took the `dest_addr` branch.
                                _ => {}
                            }
                        }
                    }
                    // Relative track targets are resolved (or dropped) at build
                    // time and never arrive here unresolved.
                    _ => {}
                }
            }
        }
        self.mod_grid.instances = instances;
    }

    /// Apply a grid module-param offset to every active voice of the instrument
    /// the `InstrumentId` maps to — the shared per-voice write path for
    /// `Module` and module-backed `Instrument` targets. Skips an instrument that
    /// won't process this block (muted, or not soloed while another is), or the
    /// additive offset would latch (the voice clears it only when it runs).
    fn apply_grid_module_offset(
        &mut self,
        instrument: synth_sequencer::InstrumentId,
        addr: synth_core::DestAddr,
        contribution: f32,
        any_soloed: bool,
    ) {
        let Some(inst) = self.instruments.iter_mut().find(|i| i.id() == instrument) else {
            return;
        };
        if inst.is_enabled() && (!any_soloed || inst.is_solo()) {
            for voice in inst.allocator_mut().voices_mut() {
                if voice.is_active() {
                    voice.apply_mod_offset_addr(addr, contribution);
                }
            }
        }
    }

    fn handle_pitch_bend(
        &mut self,
        value: synth_core::BipolarValue,
        channel: MidiChannelSelection,
    ) {
        let channel_raw = channel.as_zero_indexed();
        self.instruments
            .iter_mut()
            .filter(|inst| inst.responds_to_channel(channel_raw))
            .for_each(|inst| {
                inst.allocator_mut()
                    .voices_mut()
                    .iter_mut()
                    .for_each(|voice| voice.pitch_bend = value)
            });
    }

    fn handle_mod_wheel(&mut self, value: NormalizedValue, channel: MidiChannelSelection) {
        let channel_raw = channel.as_zero_indexed();
        self.instruments
            .iter_mut()
            .filter(|inst| inst.responds_to_channel(channel_raw))
            .for_each(|inst| {
                inst.allocator_mut()
                    .voices_mut()
                    .iter_mut()
                    .for_each(|voice| voice.mod_wheel = value)
            });
    }

    fn handle_aftertouch(&mut self, value: NormalizedValue, channel: MidiChannelSelection) {
        let channel_raw = channel.as_zero_indexed();
        self.instruments
            .iter_mut()
            .filter(|inst| inst.responds_to_channel(channel_raw))
            .for_each(|inst| {
                inst.allocator_mut()
                    .voices_mut()
                    .iter_mut()
                    .for_each(|voice| voice.aftertouch = value)
            });
    }

    fn handle_poly_aftertouch(
        &mut self,
        note: MidiNote,
        value: NormalizedValue,
        channel: MidiChannelSelection,
    ) {
        let channel_raw = channel.as_zero_indexed();
        self.instruments
            .iter_mut()
            .filter(|inst| inst.responds_to_channel(channel_raw))
            .for_each(|inst| {
                inst.allocator_mut()
                    .voices_mut()
                    .iter_mut()
                    .filter(|voice| voice.note() == Some(note))
                    .for_each(|voice| voice.poly_aftertouch = value)
            });
    }

    // ========================================================================
    // Global parameter handlers
    // ========================================================================

    fn handle_set_master_volume(&mut self, vol: Gain) {
        self.master_volume = vol.as_f32().clamp(0.0, 2.0);
        self.state.master_volume.store(self.master_volume);
    }

    fn handle_set_glide_time(&mut self, time: Seconds) {
        let time_secs = Seconds::new(time.as_f32().clamp(0.0, 5.0));
        for instrument in &mut self.instruments {
            instrument.allocator_mut().set_glide_time(time_secs);
        }
    }

    fn handle_set_focused_instrument(&mut self, instrument_id: Option<InstrumentId>) {
        // Store InstrumentId directly — stable across add/remove unlike a vec index.
        self.state.set_focused_instrument(instrument_id);
    }

    // ========================================================================
    // Voice/module parameter handlers
    // ========================================================================

    fn handle_set_voice_param(
        &mut self,
        instrument_id: InstrumentId,
        target: crate::commands::PolyModule,
        param: Param,
    ) {
        let module_id = target.module_id();

        if let Some(instrument) = self
            .instruments
            .iter_mut()
            .find(|i| i.id() == instrument_id)
        {
            instrument.voice_graph_mut().set_param(module_id, param);
            for voice in instrument.allocator_mut().voices_mut() {
                voice.graph.set_param(module_id, param);
            }
        }
    }

    fn handle_set_module_param(
        &mut self,
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        param: Param,
    ) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument.voice_graph_mut().set_param(module_id, param);
                    for voice in instrument.allocator_mut().voices_mut() {
                        voice.graph.set_param(module_id, param);
                    }
                }
            }
            None => {
                self.module_graph.set_param(module_id, param);
            }
        }
    }

    /// Install (or clear) a Mod Matrix slot's compiled control script, mirroring
    /// `handle_set_module_param`: apply to the instrument's template voice graph
    /// **and** every live voice so a held note picks it up immediately. The `Arc`
    /// is cloned per target (a cheap refcount bump, no bytecode copy).
    fn handle_set_mod_script(
        &mut self,
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        slot: usize,
        script: Option<std::sync::Arc<synth_core::script::BoundScript>>,
        descriptor: Option<Arc<synth_core::ModuleDescriptor>>,
    ) {
        // Every install replaces a slot's previous `Arc<BoundScript>` (template
        // voice graph + each live voice). Capture each replaced `Arc` and ship it
        // to the main thread instead of dropping it here, on the audio thread. When
        // a rebuilt descriptor is supplied (a script module whose `param` set may
        // have changed), share it into each graph node too (cheap `Arc` clone) and
        // ship the replaced descriptors to the deferred-drop channel.
        let script_trash = &mut self.script_trash_producer;
        let desc_trash = &mut self.descriptor_trash_producer;
        match instrument_id {
            Some(inst_id) => match self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                Some(instrument) => {
                    let replaced =
                        instrument
                            .voice_graph_mut()
                            .set_script(module_id, slot, script.clone());
                    Self::trash_script(script_trash, replaced);
                    if let Some(d) = &descriptor {
                        let old = instrument
                            .voice_graph_mut()
                            .set_node_descriptor(module_id, Arc::clone(d));
                        Self::trash_descriptor(desc_trash, old);
                    }
                    for voice in instrument.allocator_mut().voices_mut() {
                        let replaced = voice.graph.set_script(module_id, slot, script.clone());
                        Self::trash_script(script_trash, replaced);
                        if let Some(d) = &descriptor {
                            let old = voice.graph.set_node_descriptor(module_id, Arc::clone(d));
                            Self::trash_descriptor(desc_trash, old);
                        }
                    }
                }
                // Instrument not found (stale id — removed/renamed between the
                // command's enqueue and this drain). The script was never
                // installed and holds no other reference here, so dropping it
                // inline would free on the audio thread; route it to the trash
                // channel instead. (The `Some` arm above only ever *clones*
                // `script`, so its final drop is a non-last refcount decrement.)
                None => Self::trash_script(script_trash, script),
            },
            None => {
                let replaced = self.module_graph.set_script(module_id, slot, script);
                Self::trash_script(script_trash, replaced);
                if let Some(d) = descriptor {
                    let old = self.module_graph.set_node_descriptor(module_id, d);
                    Self::trash_descriptor(desc_trash, old);
                }
            }
        }
    }

    /// Ship a replaced graph-node descriptor to the main thread for a deferred
    /// drop (its `String`/`Vec` fields must not free on the audio thread). Full
    /// channel → drops here as a last resort, like [`trash_script`](Self::trash_script).
    fn trash_descriptor(
        producer: &mut ringbuf::HeapProd<Arc<synth_core::ModuleDescriptor>>,
        replaced: Option<Arc<synth_core::ModuleDescriptor>>,
    ) {
        if let Some(old) = replaced {
            let _ = producer.try_push(old);
        }
    }

    /// Ship a replaced mod-matrix script to the main thread for a deferred drop.
    /// If the trash channel is full, the `Arc` drops here (audio thread) as a
    /// last resort — the same degradation as the automation-trash path. The ring
    /// is sized (`SCRIPT_TRASH_BUFFER_SIZE`) for the worst-case per-command
    /// fan-out (template + every live voice) across several re-installs per drain,
    /// so it does not realistically fill between `cleanup_dropped_modules` drains.
    fn trash_script(
        producer: &mut ringbuf::HeapProd<Arc<synth_core::script::BoundScript>>,
        replaced: Option<Arc<synth_core::script::BoundScript>>,
    ) {
        if let Some(old) = replaced {
            let _ = producer.try_push(old);
        }
    }

    /// Defer destruction of replaced metadata until the control thread drains
    /// the return queues. The trash ring matches the command queue's capacity,
    /// so every metadata value accepted since the previous UI cleanup fits.
    fn trash_metadata(producer: &mut ringbuf::HeapProd<String>, discarded: Option<String>) {
        if let Some(value) = discarded {
            let _ = producer.try_push(value);
        }
    }

    // ========================================================================
    // Reset/clear handlers
    // ========================================================================

    /// Clear orphan-preview mode on both the sequencer and the shared transport
    /// state, keeping the two in sync. Safe to call when preview is already off.
    fn clear_preview(&mut self) {
        self.sequencer.set_preview_pattern(None);
        self.state.transport.set_preview_pattern(None);
    }

    fn handle_reset(&mut self) {
        for instrument in &mut self.instruments {
            instrument.panic();
            instrument.effect_chain_mut().reset();
        }
        self.master_effects.reset();
        // Reset must not leave the engine stuck in orphan-preview mode with a
        // pattern id that may no longer be valid.
        self.clear_preview();
    }

    fn handle_clear_all_modules(&mut self) {
        for instrument in &mut self.instruments {
            instrument.panic();
            instrument.set_enabled(false);
            instrument.voice_graph_mut().clear();
            instrument.effect_chain_mut().clear();
            for description in instrument.take_module_descriptions() {
                Self::trash_metadata(&mut self.metadata_trash_producer, Some(description));
            }
            instrument.rebuild_voices();
        }
        self.master_effects.clear();
        self.module_graph.clear();
        self.use_modular_routing = false;
    }

    // ========================================================================
    // Effect handlers
    // ========================================================================

    fn handle_set_bypass(
        &mut self,
        instrument_id: Option<InstrumentId>,
        module: ModuleId,
        bypass: bool,
    ) {
        if let Some(instrument_id) = instrument_id {
            if let Some(instrument) = self
                .instruments
                .iter_mut()
                .find(|instrument| instrument.id() == instrument_id)
            {
                Self::set_instrument_bypass(instrument, module, bypass);
            }
        } else {
            for instrument in &mut self.instruments {
                Self::set_instrument_bypass(instrument, module, bypass);
            }
        }
    }

    fn set_instrument_bypass(instrument: &mut Instrument, module: ModuleId, bypass: bool) {
        if let Some(slot) = instrument.effect_chain_mut().find_effect_by_id(module) {
            slot.state = crate::effect_chain::EnabledState::from(!bypass);
            return;
        }
        instrument.voice_graph_mut().set_bypass(module, bypass);
        for voice in instrument.allocator_mut().voices_mut() {
            voice.graph.set_bypass(module, bypass);
        }
    }

    fn handle_set_effect_param(
        &mut self,
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        param: Param,
    ) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(slot) = self.find_effect_by_id(inst_id, module_id) {
                    slot.effect.set_param(param);
                    slot.state = crate::effect_chain::EnabledState::Active;
                }
            }
            None => {
                if let Some(slot) = self.master_effects.find_effect_by_id(module_id) {
                    slot.effect.set_param(param);
                    slot.state = crate::effect_chain::EnabledState::Active;
                }
            }
        }
    }

    fn handle_set_effect_enabled(
        &mut self,
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        enabled: bool,
    ) {
        let state = crate::effect_chain::EnabledState::from(enabled);
        match instrument_id {
            Some(inst_id) => {
                if let Some(slot) = self.find_effect_by_id(inst_id, module_id) {
                    slot.state = state;
                }
            }
            None => {
                if let Some(slot) = self.master_effects.find_effect_by_id(module_id) {
                    slot.state = state;
                }
            }
        }
    }

    fn handle_add_visualizer(
        &mut self,
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        visualizer_type: crate::commands::VisualizerType,
        buffer: Arc<VisualizationBuffer>,
    ) {
        use crate::commands::VisualizerType;
        use synth_core::AudioEffect;

        let visualizer: Box<dyn AudioEffect> = match visualizer_type {
            VisualizerType::Oscilloscope => Box::new(Oscilloscope::new()),
            VisualizerType::LevelMeter => Box::new(LevelMeter::new()),
            VisualizerType::SpectrumAnalyzer => Box::new(SpectrumAnalyzer::new()),
        };

        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument
                        .effect_chain_mut()
                        .add_visualizer(id, visualizer, buffer);
                }
            }
            None => {
                self.master_effects.add_visualizer(id, visualizer, buffer);
            }
        }
    }

    fn handle_remove_visualizer(&mut self, instrument_id: Option<InstrumentId>, id: ModuleId) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument.effect_chain_mut().remove_visualizer(id);
                    let description = instrument.remove_module_description(id);
                    Self::trash_metadata(&mut self.metadata_trash_producer, description);
                }
            }
            None => {
                self.master_effects.remove_visualizer(id);
            }
        }
    }

    fn handle_add_effect_instance(
        &mut self,
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        effect: Box<dyn synth_core::AudioEffect>,
    ) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument.effect_chain_mut().add_effect(
                        id,
                        effect,
                        SampleRate::new(self.sample_rate),
                    );
                    let count = count_effects(instrument);
                    self.state.effect_count.store(count);
                }
            }
            None => {
                self.master_effects
                    .add_effect(id, effect, SampleRate::new(self.sample_rate));
            }
        }
    }

    fn handle_remove_effect(&mut self, instrument_id: Option<InstrumentId>, id: ModuleId) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument.effect_chain_mut().remove_effect(id);
                    let description = instrument.remove_module_description(id);
                    Self::trash_metadata(&mut self.metadata_trash_producer, description);
                    let count = count_effects(instrument);
                    self.state.effect_count.store(count);
                }
            }
            None => {
                self.master_effects.remove_effect(id);
            }
        }
    }

    fn effect_chain_for_mut(
        &mut self,
        instrument_id: Option<InstrumentId>,
    ) -> Option<&mut EffectChain> {
        match instrument_id {
            Some(inst_id) => self
                .instruments
                .iter_mut()
                .find(|i| i.id() == inst_id)
                .map(|i| i.effect_chain_mut()),
            None => Some(&mut self.master_effects),
        }
    }

    fn handle_reorder_effect(
        &mut self,
        instrument_id: Option<InstrumentId>,
        module_id: ModuleId,
        direction: ReorderDirection,
    ) {
        let Some(chain) = self.effect_chain_for_mut(instrument_id) else {
            return;
        };
        match direction {
            ReorderDirection::Up => {
                chain.move_slot_up(module_id);
            }
            ReorderDirection::Down => {
                chain.move_slot_down(module_id);
            }
        }
    }

    fn handle_set_effect_chain_order(
        &mut self,
        instrument_id: Option<InstrumentId>,
        order: &[ModuleId],
    ) {
        let Some(chain) = self.effect_chain_for_mut(instrument_id) else {
            return;
        };
        chain.set_slot_order(order);
    }

    // ========================================================================
    // Modular routing handlers
    // ========================================================================

    fn handle_add_module_instance(
        &mut self,
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        module: Box<dyn PolyModuleTrait>,
    ) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument.voice_graph_mut().add_module_with_id(id, module);
                    instrument.rebuild_voices();
                }
            }
            None => {
                self.module_graph.add_module_with_id(id, module);
                self.use_modular_routing = true;
            }
        }
    }

    fn handle_remove_module(&mut self, instrument_id: Option<InstrumentId>, id: ModuleId) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument.voice_graph_mut().remove_module(id);
                    let description = instrument.remove_module_description(id);
                    Self::trash_metadata(&mut self.metadata_trash_producer, description);
                    instrument.rebuild_voices();
                }
            }
            None => {
                if let Some(module) = self.module_graph.remove_module_and_return(id) {
                    let _ = self.return_producer.try_push(DroppedModule(module));
                }
            }
        }
    }

    fn handle_connect(&mut self, instrument_id: Option<InstrumentId>, from: PortId, to: PortId) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    if let Err(e) = instrument.voice_graph_mut().connect(
                        from.module,
                        from.port,
                        to.module,
                        to.port,
                    ) {
                        let _ = e;
                    } else {
                        instrument.rebuild_voices();
                    }
                }
            }
            None => {
                if let Err(e) =
                    self.module_graph
                        .connect(from.module, from.port, to.module, to.port)
                {
                    let _ = e;
                }
            }
        }
    }

    fn handle_disconnect(&mut self, instrument_id: Option<InstrumentId>, from: PortId, to: PortId) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id)
                    && instrument.voice_graph_mut().disconnect(
                        from.module,
                        from.port.as_str(),
                        to.module,
                        to.port.as_str(),
                    )
                {
                    instrument.rebuild_voices();
                }
            }
            None => {
                self.module_graph.disconnect(
                    from.module,
                    from.port.as_str(),
                    to.module,
                    to.port.as_str(),
                );
            }
        }
    }

    fn handle_disconnect_all(&mut self, instrument_id: Option<InstrumentId>, module: ModuleId) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument.voice_graph_mut().disconnect_all(module);
                    instrument.rebuild_voices();
                }
            }
            None => {
                self.module_graph.disconnect_all(module);
            }
        }
    }

    /// Process all active voices across all instruments and mix.
    ///
    /// Delegates to `Instrument::process` for each instrument, which handles:
    /// - Voice processing through the signal chain
    /// - Instrument volume and pan application
    /// - Mixing into the stereo output buffer
    ///
    /// ## Solo Logic
    /// If any instrument is soloed, only soloed instruments produce sound.
    /// Non-soloed instruments are skipped entirely (not just muted).
    /// Refresh the per-track [`TrackControl`] table from the sequencer `Song`,
    /// ready for per-voice application in [`Instrument::process`].
    ///
    /// Real-time safe: reads an immutable lock-free snapshot and allocates
    /// nothing. Instruments may be shared across tracks;
    /// each voice resolves the control for its own `TrackId`.
    fn update_track_controls(&mut self) {
        let song = self.sequencer.song().snapshot();
        self.track_control_generation = self.track_control_generation.wrapping_add(1).max(1);
        for sends in self.channel_sends.values_mut() {
            sends.clear();
        }
        let any_solo = song.any_solo();
        let track_auto = self.sequencer.track_auto();
        for track in song.tracks() {
            // Every track writes its own generation-marked control slot.
            if let Some(slot) = self.track_controls.get_mut(track.id.as_usize()) {
                // Live automation overrides the stored fader; absent → static.
                // The Mod Grid then adds its block-constant offset on top (lane
                // or base value + grid offset, clamped) — additive composition.
                let auto = track_auto.get(&track.id).copied().unwrap_or_default();
                let grid = self
                    .mod_grid
                    .track_offsets
                    .get(&track.id)
                    .copied()
                    .unwrap_or_default();
                let base_volume = auto.volume.unwrap_or(track.volume).as_f32();
                let base_pan = auto.pan.unwrap_or(track.pan).as_f32();
                slot.control = TrackControl {
                    volume: NormalizedValue::new(
                        (base_volume + grid.volume.as_f32()).clamp(0.0, 1.0),
                    ),
                    pan: BipolarValue::new((base_pan + grid.pan.as_f32()).clamp(-1.0, 1.0)),
                    audible: track.is_audible(any_solo) && !auto.muted.unwrap_or(false),
                };
                slot.generation = self.track_control_generation;
            }
            // Resolve this track's send taps to return-bus indices. Like the
            // fader, a shared instrument takes the last track's sends for the
            // block: clear unconditionally, then repopulate — so a track with no
            // sends sharing an instrument with one that has sends doesn't inherit
            // them. Sends to a missing return bus, or beyond `MAX_CHANNEL_SENDS`,
            // are dropped.
            if let Some(list) = self.channel_sends.get_mut(&track.instrument) {
                list.clear();
                for send in &track.sends {
                    if list.len() >= MAX_CHANNEL_SENDS {
                        break;
                    }
                    // A disabled send is a non-destructive bypass: skip resolving
                    // it so it contributes nothing this block, while the song keeps
                    // its level/tap point for when it is re-enabled.
                    if !send.enabled {
                        continue;
                    }
                    let Some(&return_index) = self.return_index.get(&send.target) else {
                        continue;
                    };
                    list.push(ChannelSend {
                        return_index,
                        level: send.level.as_f32(),
                        pre_fader: send.pre_fader,
                    });
                }
            }
        }
        // Snapshot the return-bus faders from the song into the runtime
        // channels (the song is the source of truth; the engine reads live,
        // mirroring per-track controls). A channel with no matching song def
        // keeps its previous fader and simply isn't driven.
        for def in song.return_busses() {
            if let Some(&idx) = self.return_index.get(&def.id) {
                self.return_busses[idx].set_fader(def.volume, def.pan, def.mute, def.solo);
            }
        }
    }

    /// Resolve each return's bus-to-bus `sends` into index-based taps and compute
    /// a topological processing order so every source is processed before its
    /// targets. Allocation-free in steady state: the per-return vecs and the sort
    /// scratch are cleared and refilled, growing only when return busses are added.
    ///
    /// Both the source return and each send target are resolved to a
    /// `return_busses` index via `return_index`, so this never assumes the song's
    /// return order matches the engine's. Re-reads the song itself (rather than
    /// borrowing one from the caller) so it can take `&mut self`; called from
    /// `process` right after `update_track_controls`.
    fn resolve_return_routing(&mut self) {
        let n = self.return_busses.len();
        // Size the scratch vecs to the return count (grows only when buses added).
        if self.return_sends.len() < n {
            self.return_sends.resize_with(n, Vec::new);
        }
        if self.return_indegree.len() < n {
            self.return_indegree.resize(n, 0);
        }
        for list in self.return_sends.iter_mut().take(n) {
            list.clear();
        }
        for d in self.return_indegree.iter_mut().take(n) {
            *d = 0;
        }
        // Default to plain index order; the topological sort below replaces it
        // when the song is readable.
        self.return_order.clear();
        for idx in 0..n {
            self.return_order.push(idx);
        }

        let song = self.sequencer.song().snapshot();

        // Resolve send targets and accumulate indegrees for the topological sort.
        for def in song.return_busses() {
            let Some(&src_idx) = self.return_index.get(&def.id) else {
                continue;
            };
            if src_idx >= n {
                continue;
            }
            for send in &def.sends {
                if !send.enabled {
                    continue;
                }
                let Some(&target_index) = self.return_index.get(&send.target) else {
                    continue;
                };
                if target_index >= n || target_index == src_idx {
                    continue;
                }
                self.return_sends[src_idx].push(ResolvedReturnSend {
                    target_index,
                    level: send.level.as_f32(),
                });
                self.return_indegree[target_index] += 1;
            }
        }

        // Kahn's algorithm: `return_order` doubles as the output list and the
        // work queue (head pointer `read`). Seed it with every indegree-0 node in
        // index order (stable), then relax outgoing edges.
        self.return_order.clear();
        for (idx, &deg) in self.return_indegree.iter().take(n).enumerate() {
            if deg == 0 {
                self.return_order.push(idx);
            }
        }
        let mut read = 0;
        while read < self.return_order.len() {
            let node = self.return_order[read];
            read += 1;
            for k in 0..self.return_sends[node].len() {
                let target = self.return_sends[node][k].target_index;
                self.return_indegree[target] -= 1;
                if self.return_indegree[target] == 0 {
                    self.return_order.push(target);
                }
            }
        }
        // Any nodes left out are in a cycle (should not happen — cycles are
        // rejected at creation). Append them in index order so they still render;
        // their back-edge taps into already-processed targets are simply dropped.
        if self.return_order.len() < n {
            for idx in 0..n {
                if !self.return_order.contains(&idx) {
                    self.return_order.push(idx);
                }
            }
        }
    }

    /// Apply `Global(..)` automation events for this block.
    ///
    /// `MasterVolume` writes the engine master gain (mirroring
    /// `handle_set_master_volume`: local field + shared atomic). Tempo and swing
    /// are intentionally not global automation params — both alter the playback
    /// time grid rather than a per-block value (tempo lives in the song's tempo
    /// map `Song::tempo_at`, swing in `Pattern::apply_swing`). Track automation
    /// is handled in the sequencer (see `SequencerEngine::track_auto`);
    /// instrument automation in `route_sequencer_events`.
    fn apply_global_automation(&mut self) {
        for event in &self.sequencer_event_buffer {
            if let SequencerEvent::Parameter {
                target: AutomationTarget::Global(param),
                value,
                ..
            } = event
            {
                match param {
                    GlobalParam::MasterVolume => {
                        let gain = value.as_f32().clamp(0.0, 2.0);
                        self.master_volume = gain;
                        self.state.master_volume.store(gain);
                    }
                }
            }
        }
    }

    fn process_voices(&mut self, context: &ProcessContext<'_>) {
        let num_channels = 2;
        let buffer_size = context.samples.as_usize() * num_channels;

        // Ensure mix buffer is sized correctly and cleared
        self.mix_buffer.resize(buffer_size);
        self.mix_buffer.clear();

        let mut active_count = 0u32;

        // Check if any instrument is soloed
        let any_soloed = self.instruments.iter().any(|i| i.is_solo());

        // Process each instrument into its channel bus (post-effect, pre-fader
        // signal left in the instrument's `effect_buffer`). The instrument no
        // longer writes the master mix; the bus stage below applies the channel
        // fader/pan and sums to `mix_buffer`.
        //
        // Sidechain routing uses *previous-callback* outputs from
        // `self.prev_instrument_outputs`. This introduces ~1 buffer of
        // sidechain detection latency (e.g. ~5 ms at 256-frame buffers /
        // 48 kHz) — well below typical compressor attack times. The
        // benefit is that processing order doesn't matter: A can
        // sidechain B and B can sidechain A without circular reads.
        // Per-track override map, read-only for the block — lets a stolen
        // note seed its track pitch at retrigger (disjoint field borrow from
        // `self.instruments`).
        let track_auto = self.sequencer.track_auto();
        let track_controls = TrackControlSnapshot {
            slots: &self.track_controls,
            generation: self.track_control_generation,
        };
        for instrument in &mut self.instruments {
            // Skip this instrument if:
            // - Any instrument is soloed AND this one is not soloed
            if any_soloed && !instrument.is_solo() {
                continue;
            }

            // Feed sidechain from the previous callback before process.
            if let Some(src_id) = instrument.sidechain_source_id()
                && let Some(prev) = self.prev_instrument_outputs.get(&src_id)
            {
                instrument.feed_sidechain_inputs(prev.as_slice());
            }

            active_count += instrument.process(context, track_auto, track_controls);
        }

        // Clear each return bus's send-accumulation buffer for this block before
        // the channel-bus stage taps into it.
        for bus in &mut self.return_busses {
            bus.prepare_block(buffer_size);
        }

        // Channel-bus stage: apply the shared instrument fader and sum into the
        // master mix, tapping configured sends into the return busses. Track
        // controls were already applied per voice before the shared effects.
        mix_channel_busses(
            &self.instruments,
            any_soloed,
            ChannelControls {
                grid_offsets: &self.mod_grid.instrument_offsets,
                sends: &self.channel_sends,
            },
            &mut self.return_busses,
            &mut self.mix_buffer,
            &self.state.channel_meters,
        );

        // Return-bus stage with bus-to-bus routing. Process each return in the
        // dependency order computed by `resolve_return_routing` (every source
        // before its targets): run its effect chain, render its post-fader output
        // into `return_scratch`, tap that into any target return busses (which
        // come later in the order, so their input buffers are still open), then
        // sum it into the master mix. With no bus-to-bus sends this reduces to the
        // previous in-order pass.
        self.return_scratch.resize(buffer_size);
        // Return solo: when any return is soloed, only soloed returns reach the
        // master mix. Bus-to-bus taps still flow (routing is solo-independent), so
        // a soloed return that is fed by other returns still hears them.
        let any_return_solo = self.return_busses.iter().any(ReturnBusChannel::is_soloed);
        for oi in 0..self.return_order.len() {
            let idx = self.return_order[oi];
            self.return_busses[idx].process_chain(context);
            let peak = {
                let scratch = &mut self.return_scratch.as_mut_slice()[..buffer_size];
                self.return_busses[idx].render_output(scratch)
            };
            // Bus-to-bus taps into the (not-yet-processed) target returns.
            for si in 0..self.return_sends[idx].len() {
                let send = self.return_sends[idx][si];
                let scratch = &self.return_scratch.as_slice()[..buffer_size];
                let dst = self.return_busses[send.target_index].input_mut();
                let m = dst.len().min(buffer_size);
                for j in 0..m {
                    dst[j] += scratch[j] * send.level;
                }
            }
            // Sum into the master mix (scratch is already silent when muted),
            // unless solo gates this return out of the final mix.
            if !any_return_solo || self.return_busses[idx].is_soloed() {
                let scratch = &self.return_scratch.as_slice()[..buffer_size];
                let dst = self.mix_buffer.as_mut_slice();
                let m = dst.len().min(buffer_size);
                for j in 0..m {
                    dst[j] += scratch[j];
                }
            }
            self.state
                .return_meters
                .publish(idx, u64::from(self.return_busses[idx].id().0), peak);
        }
        self.state.return_meters.set_count(self.return_busses.len());

        // Capture each instrument's post-effect-chain output for the
        // next callback. Pure copy into pre-allocated buffers — no allocs.
        for instrument in &self.instruments {
            if let Some(buf) = self.prev_instrument_outputs.get_mut(&instrument.id()) {
                let src = instrument.last_output_interleaved();
                if buf.len() < src.len() {
                    // Resize once at startup or after a buffer-size change.
                    buf.resize(src.len());
                }
                buf.as_mut_slice()[..src.len()].copy_from_slice(src);
            }
        }

        // Update total voice count across all instruments
        self.state.voice_count.store(active_count);
    }

    /// Process the global module graph.
    ///
    /// This processes user-added modules and mixes their output
    /// into the main mix buffer.
    fn process_module_graph(&mut self, context: &ProcessContext<'_>) {
        if !self.use_modular_routing || self.module_graph.is_empty() {
            return;
        }

        // Resize graph output buffer
        self.graph_output.resize(context.samples.as_usize());
        self.graph_output.clear();

        // Process the module graph
        self.module_graph.process(&mut self.graph_output, context);

        // Mix graph output into stereo mix buffer (mono to stereo)
        for i in 0..context.samples.as_usize() {
            let sample = self.graph_output[i];
            self.mix_buffer[i * 2] += sample;
            self.mix_buffer[i * 2 + 1] += sample;
        }
    }

    /// Process the global master effects chain.
    ///
    /// This processes effects like master reverb, limiter, EQ
    /// on the mixed output from all instruments.
    fn process_master_effects(&mut self, context: &ProcessContext<'_>) {
        if self.master_effects.is_empty() {
            return;
        }

        // Process master effects in place on the mix buffer
        self.master_effects.process(&mut self.mix_buffer, context);
    }

    /// Update metering.
    fn update_meters(&mut self, output: &[f32]) {
        self.metering
            .update(output, &self.state, &mut self.event_producer);
    }
}

impl Default for SynthEngine {
    fn default() -> Self {
        Self::new().0
    }
}

/// Owning-track fader state for one channel, snapshotted from the sequencer
/// `Song` each block. `NEUTRAL` (unity volume, centre pan, audible) composes to
/// the instrument's own fader unchanged, so an instrument with no owning track
/// behaves exactly as it did before Phase 2.
#[derive(Clone, Copy)]
pub(crate) struct TrackControl {
    pub(crate) volume: NormalizedValue,
    pub(crate) pan: BipolarValue,
    pub(crate) audible: bool,
}

#[derive(Clone, Copy)]
struct TrackControlSlot {
    control: TrackControl,
    generation: u64,
}

impl Default for TrackControlSlot {
    fn default() -> Self {
        Self {
            control: TrackControl::NEUTRAL,
            generation: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TrackControlSnapshot<'a> {
    slots: &'a [TrackControlSlot],
    generation: u64,
}

impl TrackControlSnapshot<'_> {
    pub(crate) fn get(self, track: Option<synth_sequencer::TrackId>) -> TrackControl {
        let Some(track) = track else {
            return TrackControl::NEUTRAL;
        };
        self.slots
            .get(track.as_usize())
            .filter(|slot| slot.generation == self.generation)
            .map_or(TrackControl::NEUTRAL, |slot| slot.control)
    }
}

impl TrackControl {
    const NEUTRAL: Self = Self {
        volume: NormalizedValue::MAX,
        pan: BipolarValue::CENTER,
        audible: true,
    };
}

/// Maximum number of send taps resolved per channel each block. Pre-allocated
/// so `update_track_controls` can refresh the per-channel send list without
/// growing a vec on the audio thread; extra sends beyond this cap are ignored.
const MAX_CHANNEL_SENDS: usize = 16;

/// A resolved send tap for one channel: the target return bus *index* (already
/// resolved from `ReturnBusId` to a position in `return_busses`), the send
/// level, and the pre/post-fader tap point. Built each block by
/// `update_track_controls` from the track's `sends`.
#[derive(Clone, Copy)]
struct ChannelSend {
    return_index: usize,
    level: f32,
    pre_fader: bool,
}

/// A resolved bus-to-bus send: the target return-bus *index* (resolved from
/// `ReturnBusId`) and the send level. Bus-to-bus sends are post-fader (they tap
/// the source return's post-fader output). Built each block by
/// `update_track_controls` from each return's `sends`.
#[derive(Clone, Copy)]
struct ResolvedReturnSend {
    target_index: usize,
    level: f32,
}

/// The per-instrument control maps the channel-bus stage reads together. Track
/// controls have already been applied per voice before the shared effect chain.
struct ChannelControls<'a> {
    /// Mod Grid per-instrument Volume/Pan offsets, keyed by `InstrumentId`
    /// (pre-keyed off the audio thread).
    grid_offsets:
        &'a std::collections::HashMap<InstrumentId, crate::mod_grid::GridInstrumentOffset>,
    sends: &'a std::collections::HashMap<InstrumentId, ArrayVec<ChannelSend, MAX_CHANNEL_SENDS>>,
}

/// Channel-bus stage: mix every instrument's channel into the master buffer.
///
/// Channel-strip model: each instrument is a channel whose post-effect,
/// **pre-fader** signal lives in its `effect_buffer` (read via
/// [`Instrument::last_output_interleaved`]). This stage applies the shared
/// instrument fader/pan, taps sends, and sums the result into `mix_buffer`.
///
/// Moving the shared instrument fader here (out of `Instrument::process`) gives
/// sends/returns one insertion point. The per-track fader is applied per voice
/// before the shared effect chain. The sidechain tap stays pre-instrument-fader:
/// this stage never writes gain back into `effect_buffer`.
///
/// Skips muted/non-soloed instruments. Track mute/solo has already been applied
/// to each tagged voice.
fn mix_channel_busses(
    instruments: &[Box<Instrument>],
    any_soloed: bool,
    controls: ChannelControls<'_>,
    return_busses: &mut [ReturnBusChannel],
    mix_buffer: &mut AudioBuffer,
    channel_meters: &crate::state::ChannelMeterBank,
) {
    let dst = mix_buffer.as_mut_slice();
    // Publish one meter slot per instrument, in order (silent channels read 0),
    // so the GUI can show a level on every strip.
    for (i, instrument) in instruments.iter().enumerate() {
        let soloed_out = any_soloed && !instrument.is_solo();
        let audible = !soloed_out && !instrument.mute_state().is_muted();

        let peak = if audible {
            // Apply the instrument fader. The Mod Grid's
            // per-instrument offset composes additively onto the instrument fader
            // (volume clamped to a valid gain, pan clamped by `BipolarValue`).
            let grid = controls
                .grid_offsets
                .get(&instrument.id())
                .copied()
                .unwrap_or_default();
            let pan = BipolarValue::new(instrument.pan().as_f32() + grid.pan.as_f32());
            let (pan_left, pan_right) = Gain::from_pan(pan);
            let volume = (instrument.volume().as_f32() + grid.volume.as_f32()).clamp(0.0, 2.0);
            let left_gain = pan_left.as_f32() * volume;
            let right_gain = pan_right.as_f32() * volume;

            let src = instrument.last_output_interleaved();

            // Send taps into return busses. Pre-fader taps the raw channel
            // signal; post-fader taps after the channel fader/pan (the common
            // case, so the send tracks the fader). Linear sum — soft-clip is
            // applied only on the return's own output, not per-tap.
            if let Some(sends) = controls.sends.get(&instrument.id()) {
                for send in sends {
                    if let Some(bus) = return_busses.get_mut(send.return_index) {
                        apply_send_tap(src, left_gain, right_gain, *send, bus.input_mut());
                    }
                }
            }

            let peak = stereo_peak(src, left_gain, right_gain);
            mix_stereo_faded(src, left_gain, right_gain, dst);
            peak
        } else {
            0.0
        };

        channel_meters.publish(i, instrument.id().as_u64(), peak);
    }
    channel_meters.set_count(instruments.len());
}

/// Sum one channel's send into a return bus's accumulation buffer.
///
/// `pre_fader` taps the raw channel signal (`src` scaled only by the send
/// level); otherwise the post-fader signal (`src` scaled by the channel
/// fader/pan gains *and* the send level) is summed — the common case where the
/// send level tracks the channel fader. Linear, no soft-clip (that is applied
/// once on the return's own output in [`ReturnBus::mix_into`]).
#[inline]
fn apply_send_tap(
    src: &[f32],
    left_gain: f32,
    right_gain: f32,
    send: ChannelSend,
    dst: &mut [f32],
) {
    let (l, r) = if send.pre_fader {
        (send.level, send.level)
    } else {
        (left_gain * send.level, right_gain * send.level)
    };
    let n = src.len().min(dst.len());
    let mut i = 0;
    while i + 1 < n {
        dst[i] += src[i] * l;
        dst[i + 1] += src[i + 1] * r;
        i += 2;
    }
}

/// Count the number of effects (excluding visualizers) in an instrument's effect chain.
#[allow(clippy::cast_possible_truncation)]
fn count_effects(instrument: &Instrument) -> u32 {
    instrument
        .effect_chain()
        .slots()
        .iter()
        .filter(|s| matches!(s, crate::effect_chain::ChainSlot::Effect(_)))
        .count() as u32
}

/// Resolve an `InstrumentId` to its index in `instruments`.
///
/// Falls back to index 0 (first instrument) so orphaned notes — those naming an
/// instrument that no longer exists — still produce sound.
fn resolve_instrument_index(
    id: &synth_sequencer::InstrumentId,
    instruments: &[Box<Instrument>],
) -> Option<usize> {
    if let Some(idx) = instruments.iter().position(|i| i.id() == *id) {
        return Some(idx);
    }
    // Fallback: first instrument (orphaned notes still produce sound)
    if instruments.is_empty() {
        None
    } else {
        Some(0)
    }
}

/// Resolve a per-note `NoteTrigger` from a `SequencerEvent::NoteOn`'s expression.
///
/// `glide.from` is converted to a target-relative semitone offset (so it is
/// transpose-invariant): relative `Semitones` pass through; an absolute `Pitch`
/// becomes `source_midi - target_midi`. RT-safe — no allocation.
fn note_trigger(
    legato: bool,
    glide: &Option<Glide>,
    expression: &Option<NoteExpression>,
    target: synth_sequencer::Pitch,
    track: Option<synth_sequencer::TrackId>,
) -> crate::voice::NoteTrigger {
    let glide_spec = glide.map(|g| {
        let from_offset = match g.from {
            GlideFrom::Semitones(s) => s,
            GlideFrom::Pitch(p) => {
                Semitones::new(f32::from(p.as_midi()) - f32::from(target.as_midi()))
            }
        };
        crate::voice::GlideSpec {
            from_offset,
            time: Seconds::from(g.time),
            stepped: matches!(g.interp, GlideInterp::Stepped),
        }
    });
    // Vibrato (taxonomy primitive 1): map the sequencer shape onto the engine LFO
    // and the fade-in time onto Seconds. Note-shape scalars (accent/gate/ghost/
    // probability) are already resolved sequencer-side, so they're ignored here.
    let vibrato_spec = expression
        .and_then(|e| e.vibrato)
        .map(|v| crate::voice::VibratoSpec {
            depth: v.depth,
            rate: v.rate,
            fade_in: Seconds::from(v.delay),
            // Map the serialized sequencer shape onto the shared synth_core LFO enum.
            shape: match v.shape {
                VibratoShape::Sine => LfoWaveform::Sine,
                VibratoShape::Triangle => LfoWaveform::Triangle,
                VibratoShape::Square => LfoWaveform::Square,
                VibratoShape::Saw => LfoWaveform::Sawtooth,
            },
        });
    crate::voice::NoteTrigger {
        legato,
        glide: glide_spec,
        vibrato: vibrato_spec,
        track,
    }
}

/// Route sequencer events to the appropriate instruments.
///
/// Resolves each event's instrument id to a vec index by scanning `instruments`.
/// Also pushes note events to the OSC telemetry ring buffer.
fn route_sequencer_events(
    events: &[SequencerEvent],
    instruments: &mut [Box<Instrument>],
    note_event_producer: &mut ringbuf::HeapProd<NoteEvent>,
    event_drops: &std::sync::atomic::AtomicU32,
) {
    for event in events {
        match event {
            SequencerEvent::NoteOn {
                pitch,
                velocity,
                instrument,
                legato,
                glide,
                expression,
                track,
                ..
            } => {
                let note = MidiNote::new(pitch.as_midi());
                let vel = *velocity;
                let trigger = note_trigger(*legato, glide, expression, *pitch, *track);

                if let Some(idx) = resolve_instrument_index(instrument, instruments) {
                    instruments[idx].note_on_expr(note, vel, trigger);

                    if note_event_producer
                        .try_push(NoteEvent::On {
                            note,
                            velocity: vel,
                            instrument_id: instruments[idx].id(),
                            category: instruments[idx].category(),
                        })
                        .is_err()
                    {
                        event_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            SequencerEvent::NoteOff {
                pitch, instrument, ..
            } => {
                let note = MidiNote::new(pitch.as_midi());

                if let Some(idx) = resolve_instrument_index(instrument, instruments) {
                    instruments[idx].note_off(note);

                    if note_event_producer
                        .try_push(NoteEvent::Off {
                            note,
                            instrument_id: instruments[idx].id(),
                            category: instruments[idx].category(),
                        })
                        .is_err()
                    {
                        event_drops.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            SequencerEvent::Parameter { target, value, .. } => {
                if let AutomationTarget::Instrument { instrument, param } = target
                    && let Some(inst) = instruments.iter_mut().find(|i| i.id() == *instrument)
                {
                    // The module-targeted params (FilterCutoff/Resonance, ADSR)
                    // resolve to the *first* module of the relevant type in the
                    // instrument graph (the A1 convention) and apply through the
                    // transient override path, denormalizing 0..1 via the
                    // descriptor range; those overrides revert to base on
                    // transport stop (`clear_all_param_overrides`). Volume/Pan are
                    // instrument-level channel state set directly (pre-existing
                    // behavior, not part of the override layer).
                    match param {
                        AutoInstrumentParam::Volume => {
                            inst.set_volume(Gain::new(value.as_f32()));
                        }
                        AutoInstrumentParam::Pan => {
                            // Map 0.0-1.0 to -1.0..1.0
                            inst.set_pan(BipolarValue::new(value.as_f32() * 2.0 - 1.0));
                        }
                        AutoInstrumentParam::FilterCutoff => inst.apply_normalized_override(
                            ModuleType::Filter,
                            |p| matches!(p, Param::Filter(FilterParam::Cutoff(_))),
                            |v| Param::Filter(FilterParam::Cutoff(Hertz::new(v))),
                            *value,
                        ),
                        AutoInstrumentParam::FilterResonance => inst.apply_normalized_override(
                            ModuleType::Filter,
                            |p| matches!(p, Param::Filter(FilterParam::Resonance(_))),
                            |v| Param::Filter(FilterParam::Resonance(NormalizedValue::new(v))),
                            *value,
                        ),
                        AutoInstrumentParam::Attack => inst.apply_normalized_override(
                            ModuleType::Envelope,
                            |p| matches!(p, Param::Envelope(EnvelopeParam::Attack(_))),
                            |v| Param::Envelope(EnvelopeParam::Attack(Seconds::new(v))),
                            *value,
                        ),
                        AutoInstrumentParam::Decay => inst.apply_normalized_override(
                            ModuleType::Envelope,
                            |p| matches!(p, Param::Envelope(EnvelopeParam::Decay(_))),
                            |v| Param::Envelope(EnvelopeParam::Decay(Seconds::new(v))),
                            *value,
                        ),
                        AutoInstrumentParam::Sustain => inst.apply_normalized_override(
                            ModuleType::Envelope,
                            |p| matches!(p, Param::Envelope(EnvelopeParam::Sustain(_))),
                            |v| Param::Envelope(EnvelopeParam::Sustain(NormalizedValue::new(v))),
                            *value,
                        ),
                        AutoInstrumentParam::Release => inst.apply_normalized_override(
                            ModuleType::Envelope,
                            |p| matches!(p, Param::Envelope(EnvelopeParam::Release(_))),
                            |v| Param::Envelope(EnvelopeParam::Release(Seconds::new(v))),
                            *value,
                        ),
                    }
                } else if let AutomationTarget::Module {
                    instrument,
                    module_type,
                    instance,
                    param_id,
                } = target
                    && let Some(inst) = instruments.iter_mut().find(|i| i.id() == *instrument)
                {
                    // Generic A2 target: the positional (module_type, instance)
                    // identity is rebuilt into a ModuleId, the parameter resolved
                    // by descriptor type_id, and the 0..1 value denormalized via
                    // that param's range. Reverted on stop (`handle_all_notes_off`).
                    inst.apply_module_param_override(
                        *module_type,
                        *instance,
                        param_id.as_str(),
                        *value,
                    );
                }
            }
        }
    }
}

#[cfg(feature = "rt-profiling")]
fn publish_cpu_profile(
    engine: &mut SynthEngine,
    elapsed: Duration,
    context: &AudioCallbackContext,
) {
    const PROFILE_WINDOW_CALLBACKS: u32 = 100;

    engine.callback_duration_sum += elapsed.as_secs_f32();
    engine.callback_count += 1;
    if engine.callback_count < PROFILE_WINDOW_CALLBACKS {
        return;
    }

    let callback_count = engine.callback_count as f32;
    let buffer_duration = context.frames as f32 / context.sample_rate.as_f32();
    engine
        .state
        .cpu_usage
        .store((engine.callback_duration_sum / callback_count) / buffer_duration);
    engine
        .state
        .cpu_voices
        .store((engine.stage_voices_sum / callback_count) / buffer_duration);
    engine
        .state
        .cpu_module_graph
        .store((engine.stage_module_graph_sum / callback_count) / buffer_duration);
    engine
        .state
        .cpu_master_fx
        .store((engine.stage_master_fx_sum / callback_count) / buffer_duration);
    engine
        .state
        .cpu_mod_grid
        .store((engine.stage_mod_grid_sum / callback_count) / buffer_duration);

    engine.callback_duration_sum = 0.0;
    engine.callback_count = 0;
    engine.stage_voices_sum = 0.0;
    engine.stage_module_graph_sum = 0.0;
    engine.stage_master_fx_sum = 0.0;
    engine.stage_mod_grid_sum = 0.0;
}

impl AudioProcessor for SynthEngine {
    fn process(&mut self, output: &mut [f32], context: &AudioCallbackContext) {
        #[cfg(feature = "rt-profiling")]
        let callback_started = Instant::now();

        // Retry sending pending recorded notes from previous cycle
        if let Some((pattern_id, notes, overdub)) = self.pending_recorded_notes.take() {
            self.flush_recorded_notes(pattern_id, notes, overdub);
        }

        // Process commands
        self.process_commands();

        let sample_count = SampleCount::new(context.frames);

        // Resample complete stereo input frames into the engine's hardware rate.
        // The stream keeps bounded latency while tracking independent device
        // clocks; no allocation or lock is taken here.
        let stereo_samples = context.frames * 2;
        // No resize — buffer is pre-allocated to 8192. If block > 4096 frames,
        // we silently cap to buffer size (avoids RT allocation).
        let mut audio_input_buffer = std::mem::take(&mut self.audio_input_buffer);
        let buf_cap = audio_input_buffer.len();
        let usable = stereo_samples.min(buf_cap);
        if let Some(ref mut stream) = self.audio_input_stream {
            stream.render(
                &mut audio_input_buffer[..usable],
                usable / 2,
                context.sample_rate,
            );
        } else {
            audio_input_buffer[..usable].fill(0.0);
        }
        let has_input = self.audio_input_stream.is_some();

        let audio_input_ref: Option<&[f32]> = if has_input {
            Some(&audio_input_buffer[..usable])
        } else {
            None
        };

        let process_context = ProcessContext {
            sample_rate: synth_core::SampleRate::new(context.sample_rate.as_f32()),
            samples: sample_count,
            tempo: self.state.transport.get_tempo(),
            is_playing: self.state.transport.is_playing(),
            position_beats: BeatPosition::new(self.state.transport.position_beats.load()),
            voice_start_time: synth_core::SamplePosition::ZERO,
            audio_input: audio_input_ref,
        };

        // Save tick before sequencer advances (for beat boundary detection)
        let prev_tick = self.sequencer.current_tick();
        let was_playing = self.sequencer.play_state() == PlayState::Playing;

        // Process sequencer events
        self.sequencer_event_buffer.clear();
        self.sequencer
            .process(sample_count, &mut self.sequencer_event_buffer);

        let curr_tick = self.sequencer.current_tick();

        self.state.transport.set_ticks(curr_tick.0);

        // If the sequencer auto-stopped inside process() (reached end of the
        // arrangement with looping disabled), mirror EngineCommand::Stop on
        // the transport side: clear is_playing and release all voices.
        if was_playing && self.sequencer.play_state() != PlayState::Playing {
            self.state.transport.set_playing(false);
            for instrument in &mut self.instruments {
                instrument.all_notes_off();
            }
            self.clear_all_param_overrides();
        }

        // Tick the recording state machine
        self.recording.tick(curr_tick);
        // Update shared recording state if it changed
        let rec_state = self.recording.state();
        if self.state.transport.recording_state() != rec_state {
            self.state.transport.set_recording_state(rec_state);
        }

        // Cache recording state for the checks below
        let rec_state_enum = self.recording.state();

        // Flush recorded notes at loop boundary so they play on next pass.
        // First flush uses the user's overdub setting (may clear pattern);
        // take_released_notes() sets loop_flushed so subsequent flushes always overdub.
        if rec_state_enum == crate::recording::RecordingState::Capturing
            && curr_tick.0 < prev_tick.0
        {
            let pattern_id = self.recording.target_pattern();
            let overdub = self.recording.is_overdub();
            let notes = self.recording.take_released_notes();
            if let Some(pid) = pattern_id
                && !notes.is_empty()
            {
                self.flush_recorded_notes(pid, notes, overdub);
            }
        }

        // Emit live recording preview only when notes changed (avoids per-buffer clone)
        if rec_state_enum == crate::recording::RecordingState::Capturing
            && let Some((_region_start, pattern_length)) = self.recording.target_info()
            && let Some((completed, held)) = self.recording.preview_snapshot()
        {
            let _ = self.event_producer.try_push(EngineEvent::RecordingPreview {
                completed,
                held,
                pattern_length,
            });
        }

        // Trigger metronome click on beat boundaries
        if self.click_generator.is_enabled() && curr_tick.0 > prev_tick.0 {
            let tpq = synth_sequencer::TICKS_PER_QUARTER as u64;
            if curr_tick.0 / tpq != prev_tick.0 / tpq {
                // Crossed a beat boundary — use cached ticks_per_bar for accent
                let ticks_per_bar = self.recording.ticks_per_bar().0 as u64;
                let accented = curr_tick.0 % ticks_per_bar < tpq;
                self.click_generator.trigger_click(accented);
            }
        }

        route_sequencer_events(
            &self.sequencer_event_buffer,
            &mut self.instruments,
            &mut self.note_event_producer,
            &self.state.event_drops,
        );

        // Apply global automation (master volume) for this block.
        self.apply_global_automation();

        // Mod Grid pre-pass: run the always-on control-rate modulator graphs
        // once and accumulate their additive offsets, before instruments read
        // them and before track-control composition folds the track offsets in.
        #[cfg(feature = "rt-profiling")]
        let stage_started = Instant::now();
        self.process_mod_grid(&process_context);
        #[cfg(feature = "rt-profiling")]
        {
            self.stage_mod_grid_sum += stage_started.elapsed().as_secs_f32();
        }

        // Refresh per-instrument track controls from the Song before the
        // channel-bus stage (inside process_voices) reads them. Track-fader
        // automation is folded in here via SequencerEngine::track_auto.
        self.update_track_controls();
        // Push per-track pitch overrides (TrackParam::Pitch) onto the active
        // voices — per voice via the track tag, never the shared strip.
        self.update_track_pitch();
        // Resolve bus-to-bus send routing + processing order before the return
        // stage (inside process_voices) consumes it.
        self.resolve_return_routing();

        #[cfg(feature = "rt-profiling")]
        let stage_started = Instant::now();
        self.process_voices(&process_context);
        #[cfg(feature = "rt-profiling")]
        {
            self.stage_voices_sum += stage_started.elapsed().as_secs_f32();
        }

        // Process modular graph (user-added modules)
        #[cfg(feature = "rt-profiling")]
        let stage_started = Instant::now();
        self.process_module_graph(&process_context);
        #[cfg(feature = "rt-profiling")]
        {
            self.stage_module_graph_sum += stage_started.elapsed().as_secs_f32();
        }

        // Process master effects (master bus: reverb, limiter, EQ, etc.)
        #[cfg(feature = "rt-profiling")]
        let stage_started = Instant::now();
        self.process_master_effects(&process_context);
        #[cfg(feature = "rt-profiling")]
        {
            self.stage_master_fx_sum += stage_started.elapsed().as_secs_f32();
        }
        self.audio_input_buffer = audio_input_buffer;

        // Mix metronome click into output
        self.click_generator
            .process(self.mix_buffer.as_mut_slice(), context.frames);

        // Process master-level visualizers after master effects (final signal)
        self.master_effects.process_visualizers(&self.mix_buffer);

        // Copy to output with master volume. The Mod Grid's block-constant
        // master-volume offset composes on top of the fader/lane value without
        // mutating the persistent `master_volume` (which would drift).
        let master_volume = (self.master_volume + self.grid_master_volume_offset).clamp(0.0, 2.0);
        let channels = context.channels as usize;
        for (i, frame) in output.chunks_mut(channels).enumerate() {
            let left = self
                .mix_buffer
                .as_slice()
                .get(i * 2)
                .copied()
                .unwrap_or(0.0);
            let right = self
                .mix_buffer
                .as_slice()
                .get(i * 2 + 1)
                .copied()
                .unwrap_or(left);

            if channels >= 2 {
                frame[0] = (left * master_volume).clamp(-1.0, 1.0);
                frame[1] = (right * master_volume).clamp(-1.0, 1.0);
            } else if channels == 1 {
                frame[0] = ((left + right) * 0.5 * master_volume).clamp(-1.0, 1.0);
            }
        }

        // Write final output to master scope for oscilloscope display
        self.state.master_scope.write_interleaved(output);

        // Update meters
        self.update_meters(output);

        // Update transport
        self.state.transport.advance(
            context.frames as u64,
            synth_core::SampleRate::new(self.sample_rate),
        );

        #[cfg(feature = "rt-profiling")]
        publish_cpu_profile(self, callback_started.elapsed(), context);
    }

    fn on_stream_start(&mut self, info: &StreamInfo) {
        self.sample_rate = info.sample_rate.as_f32();
        self.state.sample_rate.store(info.sample_rate.as_u32());
        let sr = SampleRate::new(self.sample_rate);
        self.metering.set_sample_rate(sr);
        self.sequencer.set_sample_rate(sr);
        self.click_generator.set_sample_rate(sr);
    }

    fn on_stream_stop(&mut self) {
        // Panic all instruments
        self.instruments.iter_mut().for_each(|inst| inst.panic());
    }

    fn on_error(&mut self, error: synth_core::AudioError) {
        let _ = error;
        let _ = self.event_producer.try_push(EngineEvent::BufferUnderrun);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[path = "synth_engine/tests.rs"]
mod tests;

/// Real-time allocation regression guard.
///
/// The project rule is that [`SynthEngine::process`] must never allocate on the
/// audio thread (see CLAUDE.md "Real-Time Safety"). This module installs a
/// counting global allocator for the unit-test binary and a
/// [`no_alloc`]-armed region that fails the test if a steady-state `process()`
/// call performs *any* heap allocation or deallocation — automating the RT rule
/// the project otherwise enforces by hand.
///
/// The counter is thread-local-armed, so it only observes allocations on the
/// thread running the guarded block; the constant allocations of the other unit
/// tests running in parallel are ignored. When disarmed the allocator is a plain
/// passthrough to the system allocator (one relaxed thread-local read per call).
#[cfg(test)]
#[path = "synth_engine/rt_alloc_guard.rs"]
mod rt_alloc_guard;
