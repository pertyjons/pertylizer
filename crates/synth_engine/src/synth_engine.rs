//! The main synthesizer engine with polyphony support.
//!
//! This is the complete engine that uses:
//! - VoiceAllocator for polyphonic voice management
//! - ModuleGraph for signal routing within each voice
//! - Effect chain for post-voice processing

use parking_lot::Mutex;
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::commands::{EngineCommand, EngineEvent, ModuleId, NoteEvent, PortId, ReorderDirection};
use crate::effect_chain::{EffectChain, EffectSlot};
use crate::graph::ModuleGraph;
use crate::instrument::{Instrument, InstrumentId, MidiChannel, soft_clip};
use crate::instrument_mapping::InstrumentMapping;
use crate::metering::MeteringSystem;
use crate::recording::RecordingState;
use crate::sequencer_engine::{PlayState, SequencerEngine};
use crate::shared_state::{ConnectionSnapshot, ModuleStateSnapshot};
use crate::state::EngineState;
use crate::visualizers::{LevelMeter, Oscilloscope, SpectrumAnalyzer, VisualizationBuffer};
use synth_awe::{AweEngine, SpatialContext, SpatialVoiceBank};
use synth_core::{
    AudioBuffer, AudioCallbackContext, AudioProcessor, BeatPosition, BipolarValue, CcNumber,
    EnvelopeParam, FilterParam, Gain, Hertz, MidiNote, ModuleType, NormalizedValue, Param,
    PolyModule as PolyModuleTrait, ProcessContext, SampleCount, SampleRate, Seconds, StreamInfo,
    Velocity,
};
use synth_sequencer::{AutoInstrumentParam, AutomationTarget, GlobalParam, SequencerEvent};

/// Size of the command ring buffer.
/// Large enough to handle patch loading (100+ modules with params/connections).
const COMMAND_BUFFER_SIZE: usize = 16384;

/// Size of the event ring buffer.
const EVENT_BUFFER_SIZE: usize = 256;

/// Size of the return channel for modules that need to be dropped on main thread.
const RETURN_BUFFER_SIZE: usize = 256;

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

/// A clonable sender for engine commands.
///
/// This wrapper allows multiple sources (GUI, MIDI, automation) to send
/// commands to the engine. Uses Mutex for thread-safe access to the
/// ring buffer producer - the overhead is minimal since command sending
/// is infrequent compared to audio processing.
#[derive(Clone)]
pub struct CommandSender {
    producer: Arc<Mutex<ringbuf::HeapProd<EngineCommand>>>,
}

impl CommandSender {
    /// Create a new CommandSender from a ring buffer producer.
    pub fn new(producer: ringbuf::HeapProd<EngineCommand>) -> Self {
        Self {
            producer: Arc::new(Mutex::new(producer)),
        }
    }

    /// Send a command to the engine (non-blocking, may fail if queue full).
    pub fn send(&self, command: EngineCommand) -> bool {
        self.producer.lock().try_push(command).is_ok()
    }

    /// Send a command to the engine, blocking until there's space.
    /// Use this when loading patches or doing bulk operations.
    ///
    /// Uses exponential backoff to avoid busy-waiting while still responding
    /// quickly when the queue has space. Max wait time is ~500ms before timeout.
    pub fn send_blocking(&self, command: EngineCommand) -> bool {
        // Exponential backoff: 0, 0, 1, 2, 4, 8, 16, 32, 64, 100ms (capped)
        // Total worst-case wait: ~500ms instead of 10s
        const BACKOFF_MILLIS: [u64; 10] = [0, 0, 1, 2, 4, 8, 16, 32, 64, 100];
        const MAX_ATTEMPTS: u32 = 50; // 50 attempts * avg ~10ms = ~500ms max

        let mut attempts = 0;
        let mut cmd = command;

        loop {
            match self.producer.lock().try_push(cmd) {
                Ok(()) => return true,
                Err(returned_cmd) => {
                    cmd = returned_cmd;
                    attempts += 1;
                    if attempts >= MAX_ATTEMPTS {
                        eprintln!("Command queue timeout after {attempts} attempts!");
                        return self.producer.lock().try_push(cmd).is_ok();
                    }
                    // Exponential backoff with cap at last value
                    let sleep_idx = (attempts as usize).min(BACKOFF_MILLIS.len() - 1);
                    let sleep_ms = BACKOFF_MILLIS[sleep_idx];
                    if sleep_ms > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
                    } else {
                        std::thread::yield_now();
                    }
                }
            }
        }
    }
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
    /// Returns false only if there's a timeout (deadlock protection).
    pub fn send_blocking(&mut self, command: EngineCommand) -> bool {
        self.command_sender.send_blocking(command)
    }

    /// Get a clonable command sender for sharing with other sources (MIDI, etc.).
    ///
    /// This allows multiple threads/sources to send commands to the engine.
    pub fn command_sender(&self) -> CommandSender {
        self.command_sender.clone()
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
    }

    /// Send a note on event to the default channel.
    pub fn note_on(&mut self, note: MidiNote, velocity: Velocity) -> bool {
        self.send(EngineCommand::NoteOn {
            note,
            velocity,
            channel: super::instrument::MidiChannel::CH1,
        })
    }

    /// Send a note off event to the default channel.
    pub fn note_off(&mut self, note: MidiNote) -> bool {
        self.send(EngineCommand::NoteOff {
            note,
            channel: super::instrument::MidiChannel::CH1,
        })
    }

    /// Send a note on event to a specific channel.
    pub fn note_on_channel(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        channel: super::instrument::MidiChannel,
    ) -> bool {
        self.send(EngineCommand::NoteOn {
            note,
            velocity,
            channel,
        })
    }

    /// Send a note off event to a specific channel.
    pub fn note_off_channel(
        &mut self,
        note: MidiNote,
        channel: super::instrument::MidiChannel,
    ) -> bool {
        self.send(EngineCommand::NoteOff { note, channel })
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

    /// Get the current voice count.
    pub fn voice_count(&self) -> u32 {
        self.state.voice_count.load()
    }

    /// Get the current CPU usage.
    pub fn cpu_usage(&self) -> f32 {
        self.state.cpu_usage.load()
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
    /// Shared state.
    state: Arc<EngineState>,

    // === Instrument management (multitimbral) ===
    /// All synthesizer instruments (each with its own voice graph and allocator).
    instruments: Vec<Box<Instrument>>,

    // === Master effects ===
    /// Global master effect chain (processes mixed output from all instruments).
    /// Effects like master reverb, limiter, EQ go here.
    master_effects: EffectChain,

    // === AWE (Acoustic World Engine) ===
    /// Room simulation engine, processed after master effects.
    awe_engine: AweEngine,
    /// Per-voice spatial audio bank (written by instruments, read by AWE).
    spatial_voice_bank: SpatialVoiceBank,

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
    /// Per-instrument owning-track controls (volume/pan/audibility), refreshed
    /// each block from the sequencer `Song` and consumed by the channel-bus
    /// stage (`mix_channel_busses`). Pre-allocated on instrument add/remove
    /// (mirrors `prev_instrument_outputs`) so the audio thread never grows it.
    /// Instruments with no owning track keep a `NEUTRAL` entry, which composes
    /// to the instrument's own fader unchanged.
    track_controls: std::collections::HashMap<InstrumentId, TrackControl>,

    // === Metering ===
    metering: MeteringSystem,

    // === Sequencer ===
    sequencer: SequencerEngine,
    /// Pre-allocated buffer for sequencer events (real-time safe).
    sequencer_event_buffer: Vec<SequencerEvent>,
    /// Stable mapping from sequencer instrument IDs to engine instrument IDs.
    /// Convention: `SeqInstrumentId(X)` ↔ `InstrumentId(X)`.
    instrument_mapping: InstrumentMapping,

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
    /// Consumer for live audio input (from AudioInputManager's engine ring buffer).
    audio_input_consumer: Option<ringbuf::HeapCons<f32>>,
    /// Pre-allocated buffer for audio input (stereo interleaved, block_size * 2).
    audio_input_buffer: Vec<f32>,

    // === Performance monitoring ===
    callback_duration_sum: f32,
    callback_count: u32,
}

impl SynthEngine {
    /// Create a new engine with default configuration.
    /// Create a new engine.
    ///
    /// The engine starts with no instruments — create them explicitly
    /// via [`EngineCommand::AddInstrument`].
    pub fn new() -> (Self, EngineHandle) {
        let state = EngineState::new();

        // Create command ring buffer
        let command_rb = HeapRb::<EngineCommand>::new(COMMAND_BUFFER_SIZE);
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

        let instrument_mapping = InstrumentMapping::new();

        let engine = Self {
            command_consumer,
            event_producer,
            note_event_producer,
            return_producer,
            instrument_return_producer,
            state: Arc::clone(&state),
            instruments: vec![],
            master_effects: EffectChain::new(),
            awe_engine: AweEngine::new(),
            spatial_voice_bank: SpatialVoiceBank::new(),
            module_graph: ModuleGraph::new(),
            use_modular_routing: false,
            sample_rate: 48000.0,
            master_volume: 1.0,
            mix_buffer: AudioBuffer::new(512),
            graph_output: AudioBuffer::new(1024),
            prev_instrument_outputs: std::collections::HashMap::new(),
            track_controls: std::collections::HashMap::new(),
            metering: MeteringSystem::new(synth_core::SampleRate::DVD_QUALITY),
            sequencer: SequencerEngine::new(synth_core::SampleRate::DVD_QUALITY),
            sequencer_event_buffer: Vec::with_capacity(128),
            instrument_mapping,
            recording: crate::recording::RecordingBuffer::new(),
            click_generator: crate::click_generator::ClickGenerator::new(
                synth_core::SampleRate::DVD_QUALITY,
            ),
            pre_record_loop: None,
            pending_recorded_notes: None,
            audio_input_consumer: None,
            audio_input_buffer: vec![0.0; 8192], // Pre-allocate for up to 4096 stereo frames
            callback_duration_sum: 0.0,
            callback_count: 0,
        };

        // Initialize shared state (empty — no instruments yet)
        engine.update_shared_instruments();

        let handle = EngineHandle {
            command_sender: CommandSender::new(command_producer),
            event_consumer,
            return_consumer,
            instrument_return_consumer,
            state,
            visualization_buffers: HashMap::new(),
            note_event_consumer: Some(note_event_consumer),
        };

        (engine, handle)
    }

    /// Rebuild all voices for all instruments.
    ///
    /// Each instrument uses its own voice_graph as the template.
    /// Call this after bulk operations that affect all instruments.
    #[allow(dead_code)] // Useful for future bulk operations
    fn rebuild_all_instrument_voices(&mut self) {
        self.instruments
            .iter_mut()
            .for_each(|inst| inst.rebuild_voices());
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
                if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.set_name(&name);
                }
                self.update_shared_instruments();
            }
            EngineCommand::SetInstrumentDescription {
                instrument_id,
                description,
            } => {
                if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.set_description(&description);
                }
                self.update_shared_instruments();
            }
            EngineCommand::SetPatchDescription {
                instrument_id,
                description,
            } => {
                if let Some(inst) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id() == instrument_id)
                {
                    inst.set_patch_description(description);
                }
                self.update_shared_instruments();
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
                self.update_shared_instruments();
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
                    self.update_shared_instruments();
                }
            }
            EngineCommand::SetInstrumentSolo {
                instrument_id,
                solo,
            } => {
                self.handle_set_instrument_solo(instrument_id, solo);
            }

            // Note control
            EngineCommand::NoteOn {
                note,
                velocity,
                channel,
            } => {
                self.handle_note_on(note, velocity, channel);
            }
            EngineCommand::NoteOff { note, channel } => {
                self.handle_note_off(note, channel);
            }
            EngineCommand::AllNotesOff => {
                self.handle_all_notes_off();
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
                self.push_note_event(NoteEvent::Cc {
                    cc: CcNumber::MOD_WHEEL,
                    value,
                    channel,
                });
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
                self.update_shared_graph(Some(instrument_id));
            }
            EngineCommand::SetModuleParameter {
                instrument_id,
                module_id,
                param,
            } => {
                self.handle_set_module_param(instrument_id, module_id, param);
                self.update_shared_graph(instrument_id);
            }

            // Reset/clear
            EngineCommand::Reset => {
                self.handle_reset();
            }
            EngineCommand::ClearAllModules => {
                self.handle_clear_all_modules();
                // Clear shared graph state too
                self.state.shared_graph.set_connections(Vec::new());
                self.state.shared_graph.set_processing_order(Vec::new());
                for m in self.state.shared_graph.get_all_modules() {
                    self.state.shared_graph.remove_module(m.instrument_id, m.id);
                }
                self.update_shared_instruments();
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
                self.update_shared_graph(instrument_id);
            }
            EngineCommand::RemoveModule { instrument_id, id } => {
                self.handle_remove_module(instrument_id, id);
                self.update_shared_graph(instrument_id);
            }
            EngineCommand::Connect {
                instrument_id,
                from,
                to,
            } => {
                self.handle_connect(instrument_id, from, to);
                self.update_shared_graph(instrument_id);
            }
            EngineCommand::Disconnect {
                instrument_id,
                from,
                to,
            } => {
                self.handle_disconnect(instrument_id, from, to);
                self.update_shared_graph(instrument_id);
            }
            EngineCommand::DisconnectAll {
                instrument_id,
                module,
            } => {
                self.handle_disconnect_all(instrument_id, module);
                self.update_shared_graph(instrument_id);
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
                        let _ = self.sequencer.seek(seek_to);
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
                let _ = self.sequencer.stop();
                self.state.transport.set_playing(false);
                self.state.transport.set_ticks(0);

                // Release all voices on all instruments
                for instrument in &mut self.instruments {
                    instrument.all_notes_off();
                }
            }
            EngineCommand::Pause => {
                self.sequencer.pause();
                self.state.transport.set_playing(false);
            }
            EngineCommand::Rewind => {
                let _ = self.sequencer.seek(synth_sequencer::Tick::ZERO);
                self.state.transport.set_ticks(0);
            }
            EngineCommand::Seek { tick } => {
                let _ = self.sequencer.seek(tick);
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
                let bounds = self
                    .sequencer
                    .song()
                    .try_read()
                    .and_then(|song| Self::find_pattern_bounds(&song, pattern_id));

                if let Some((start, end)) = bounds {
                    // Placed pattern — play through the arrangement at this region.
                    self.clear_preview();
                    self.sequencer.play();
                    let _ = self.sequencer.seek(start);
                    self.sequencer.set_loop(start, end, true);
                    self.sync_loop_to_transport();
                    self.state.transport.set_playing(true);
                    self.state.transport.set_ticks(start.0);
                } else {
                    // Orphan pattern — enter preview-mode and loop at pattern.length.
                    let length = self
                        .sequencer
                        .song()
                        .try_read()
                        .and_then(|song| song.pattern(pattern_id).map(|p| p.length));
                    if let Some(length) = length {
                        // max(1): a zero-length pattern would otherwise make the
                        // loop [0,0) wrap every tick and pin the playhead at 0.
                        let loop_end = synth_sequencer::Tick(u64::from(length.0.max(1)));
                        self.sequencer
                            .set_preview_pattern(Some((pattern_id, instrument)));
                        self.state.transport.set_preview_pattern(Some(pattern_id));
                        self.sequencer.play();
                        let _ = self.sequencer.seek(synth_sequencer::Tick::ZERO);
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
                        self.state.transport.set_playing(true);
                    }
                }
            }
            EngineCommand::PlayFromPattern { pattern_id } => {
                // Start playback from pattern start, no loop
                let bounds = self
                    .sequencer
                    .song()
                    .try_read()
                    .and_then(|song| Self::find_pattern_bounds(&song, pattern_id));

                if let Some((start, _end)) = bounds {
                    // Important: play() first to avoid it resetting current_tick to 0
                    self.sequencer.play();
                    let _ = self.sequencer.seek(start);
                    self.sequencer.set_loop(
                        synth_sequencer::Tick::ZERO,
                        synth_sequencer::Tick::ZERO,
                        false,
                    );
                    self.sync_loop_to_transport();
                    self.state.transport.set_playing(true);
                    self.state.transport.set_ticks(start.0);
                } else {
                    // Fallback: pattern not in arrangement, just play from beginning
                    self.sequencer.play();
                    self.state.transport.set_playing(true);
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
                self.sequencer.set_song(song);
                // The new song's pattern ids are unrelated to the old one's;
                // a stale orphan-preview target must not survive the swap.
                self.clear_preview();
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

            // AWE commands
            EngineCommand::SetAweParameter { param } => {
                self.awe_engine.set_param(param);
            }
            EngineCommand::SetAweEnabled { enabled } => {
                self.awe_engine.set_enabled(enabled);
            }
            EngineCommand::SetAweState { snapshot } => {
                self.awe_engine.apply_snapshot(snapshot);
            }
            EngineCommand::SetAudioInputConsumer { consumer } => {
                self.audio_input_consumer = Some(consumer);
            }
            EngineCommand::ClearAudioInputConsumer => {
                self.audio_input_consumer = None;
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

    fn handle_add_instrument(&mut self, instrument: Box<Instrument>) {
        // Register in mapping: SeqInstrumentId(id as u16) ↔ InstrumentId(id)
        #[allow(clippy::cast_possible_truncation)]
        let seq_id = synth_sequencer::SeqInstrumentId(instrument.id().as_u64() as u16);
        self.instrument_mapping.insert(seq_id, instrument.id());
        // Pre-allocate this instrument's sidechain output buffer so the
        // audio thread never grows the map. 4096 × 2 = max interleaved frame.
        self.prev_instrument_outputs
            .insert(instrument.id(), AudioBuffer::new(4096 * 2));
        // Pre-allocate the track-control entry (neutral until a track claims it).
        self.track_controls
            .insert(instrument.id(), TrackControl::NEUTRAL);
        self.instruments.push(instrument);
        self.update_shared_instruments();
    }

    fn handle_remove_instrument(&mut self, instrument_id: InstrumentId) {
        if let Some(idx) = self
            .instruments
            .iter()
            .position(|p| p.id() == instrument_id)
        {
            // Clean up shared graph data for this instrument
            self.state
                .shared_graph
                .remove_modules_for_instrument(instrument_id);
            self.state
                .shared_graph
                .set_connections_for_instrument(instrument_id, Vec::new());

            // Remove from mapping
            self.instrument_mapping.remove_by_engine_id(instrument_id);

            let instrument = self.instruments.swap_remove(idx);
            // Drop this instrument's sidechain cache and track-control entry.
            self.prev_instrument_outputs.remove(&instrument_id);
            self.track_controls.remove(&instrument_id);
            // Clear any sidechain references that pointed at this id.
            for inst in &mut self.instruments {
                if inst.sidechain_source_id() == Some(instrument_id) {
                    inst.set_sidechain_source_id(None);
                }
            }
            let _ = self.instrument_return_producer.try_push(instrument);
            self.update_shared_instruments();
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
            InstrumentParam::MaxVoices(_) => {
                // Cannot change max voices at runtime without reallocating
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
        self.update_shared_instruments();
    }

    fn handle_set_instrument_channel(&mut self, instrument_id: InstrumentId, channel: MidiChannel) {
        if let Some(instrument) = self
            .instruments
            .iter_mut()
            .find(|p| p.id() == instrument_id)
        {
            instrument.set_midi_channel(channel);
        }
        self.update_shared_instruments();
    }

    fn handle_set_instrument_enabled(&mut self, instrument_id: InstrumentId, enabled: bool) {
        if let Some(instrument) = self
            .instruments
            .iter_mut()
            .find(|p| p.id() == instrument_id)
        {
            instrument.set_enabled(enabled);
        }
        self.update_shared_instruments();
    }

    fn handle_set_instrument_solo(&mut self, instrument_id: InstrumentId, solo: bool) {
        if let Some(instrument) = self
            .instruments
            .iter_mut()
            .find(|p| p.id() == instrument_id)
        {
            instrument.set_solo(solo);
        }
        self.update_shared_instruments();
    }

    // ========================================================================
    // Note control handlers
    // ========================================================================

    fn handle_note_on(&mut self, note: MidiNote, velocity: Velocity, channel: MidiChannel) {
        let channel_raw = channel.as_zero_indexed();
        let mut note_triggered = false;

        // Check if there's a focused instrument for keyboard input (by InstrumentId)
        let focused_id = self.state.get_focused_instrument();

        for instrument in self.instruments.iter_mut() {
            // If focused instrument is set, only that instrument receives keyboard input
            // (Channel 0 is the default keyboard channel)
            if let Some(focus_id) = focused_id {
                // Only send to focused instrument, and only for channel 0 (keyboard)
                if channel_raw == 0 && instrument.id() != focus_id {
                    continue;
                }
                // For other channels (e.g., external MIDI), use traditional routing
                if channel_raw != 0 && !instrument.responds_to_channel(channel_raw) {
                    continue;
                }
            } else {
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

        if self.use_modular_routing {
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

    fn handle_note_off(&mut self, note: MidiNote, channel: MidiChannel) {
        let channel_raw = channel.as_zero_indexed();
        let focused_id = self.state.get_focused_instrument();

        for instrument in self.instruments.iter_mut() {
            // Same logic as note_on for focused instrument routing
            if let Some(focus_id) = focused_id {
                if channel_raw == 0 && instrument.id() != focus_id {
                    continue;
                }
                if channel_raw != 0 && !instrument.responds_to_channel(channel_raw) {
                    continue;
                }
            } else if !instrument.responds_to_channel(channel_raw) {
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

        if self.use_modular_routing {
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
            // Transport stop reverts automation overrides to their base values.
            instrument.clear_param_overrides();
        }

        if self.use_modular_routing {
            self.module_graph.reset();
            self.module_graph.clear_param_overrides();
        }

        let _ = self.event_producer.try_push(EngineEvent::AllNotesReleased);
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

    fn handle_pitch_bend(&mut self, value: synth_core::BipolarValue, channel: MidiChannel) {
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

    fn handle_mod_wheel(&mut self, value: NormalizedValue, channel: MidiChannel) {
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

    fn handle_aftertouch(&mut self, value: NormalizedValue, channel: MidiChannel) {
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
        channel: MidiChannel,
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
        let target_instruments: Box<dyn Iterator<Item = &mut Box<Instrument>> + '_> =
            if let Some(id) = instrument_id {
                Box::new(self.instruments.iter_mut().filter(move |i| i.id() == id))
            } else {
                Box::new(self.instruments.iter_mut())
            };

        // Capture the iterator's instrument_ids so we can also refresh the
        // shared_graph snapshot after mutating bypass state. Without this,
        // offline tooling (e.g. analyze_note) would still see the old bypass
        // state and render with the wrong topology.
        let mut touched: Vec<InstrumentId> = Vec::new();
        for instrument in target_instruments {
            touched.push(instrument.id());

            // Try effect chain first — match by ModuleId so duplicate effects
            // of the same type can be bypassed independently.
            if let Some(slot) = instrument.effect_chain_mut().find_effect_by_id(module) {
                slot.state = crate::effect_chain::EnabledState::from(!bypass);
                continue;
            }

            // Also set bypass on voice graph modules (osc, filter, env, LFO)
            instrument.voice_graph_mut().set_bypass(module, bypass);
            for voice in instrument.allocator_mut().voices_mut() {
                voice.graph.set_bypass(module, bypass);
            }
        }
        for inst_id in touched {
            self.update_shared_graph(Some(inst_id));
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
        // Mirror the change into shared_graph so offline tooling
        // (e.g. analyze_note) sees the new parameter and bypass state.
        self.update_shared_graph(instrument_id);
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
        // Mirror the change into shared_graph so offline tooling
        // (e.g. analyze_note) sees the new bypass state.
        self.update_shared_graph(instrument_id);
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
        self.update_shared_instruments();
        // Mirror the new effect slot into shared_graph so offline tooling
        // sees the latest chain composition and per-effect parameters.
        self.update_shared_graph(instrument_id);
    }

    fn handle_remove_effect(&mut self, instrument_id: Option<InstrumentId>, id: ModuleId) {
        match instrument_id {
            Some(inst_id) => {
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                    instrument.effect_chain_mut().remove_effect(id);
                    let count = count_effects(instrument);
                    self.state.effect_count.store(count);
                }
            }
            None => {
                self.master_effects.remove_effect(id);
            }
        }
        self.update_shared_instruments();
        // Drop the removed effect slot from shared_graph so offline tooling
        // does not keep rendering with a stale snapshot.
        self.update_shared_graph(instrument_id);
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
        self.update_shared_instruments();
        self.update_shared_graph(instrument_id);
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
        self.update_shared_instruments();
        self.update_shared_graph(instrument_id);
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

    // ========================================================================
    // Shared graph state update
    // ========================================================================

    /// Update shared graph state after a graph-changing command.
    ///
    /// Dispatches to the appropriate instrument or ignores global graph (not exposed via MCP yet).
    fn update_shared_graph(&self, instrument_id: Option<InstrumentId>) {
        if let Some(inst_id) = instrument_id
            && let Some(instrument) = self.instruments.iter().find(|i| i.id() == inst_id)
        {
            self.update_shared_graph_for_instrument(instrument);
        }
        // Global module graph (instrument_id == None) is not exposed yet
    }

    /// Update the shared graph state from an instrument's voice graph.
    ///
    /// Called after topology-changing commands (add/remove module, connect/disconnect)
    /// and parameter changes. Allocates (Vec, String) but only at user-interaction
    /// rate, not per-sample.
    fn update_shared_graph_for_instrument(&self, instrument: &Instrument) {
        let instrument_id = instrument.id();
        let graph = instrument.voice_graph();
        let shared = &self.state.shared_graph;

        // Build module snapshots from the voice graph
        let mut module_ids: Vec<ModuleId> = graph.module_ids().collect();
        // Sort by (type prefix, instance) for stable ordering without allocating Strings.
        module_ids.sort_by(|a, b| {
            a.module_type
                .prefix()
                .cmp(b.module_type.prefix())
                .then(a.instance.cmp(&b.instance))
        });

        // Clear and rebuild only modules for THIS instrument
        shared.remove_modules_for_instrument(instrument_id);

        for &id in &module_ids {
            if let Some(module) = graph.get_module(id) {
                let descriptor = module.descriptor();
                let mut snapshot = ModuleStateSnapshot::new(
                    id,
                    instrument_id,
                    module.module_type(),
                    descriptor.name.to_string(),
                );
                snapshot.parameters = module.get_params();
                snapshot.bypass_state = if graph.is_bypassed(id) {
                    synth_core::BypassState::Bypassed
                } else {
                    synth_core::BypassState::Active
                };
                shared.set_module(snapshot);
            }
        }

        // Also export effect chain slots so offline tooling (e.g. analyze_note)
        // can reproduce the full per-instrument signal flow, not just the voice
        // graph. Visualizers are skipped — they don't modify audio. Slot
        // ordering in the chain is preserved by passing it explicitly through
        // ConnectionSnapshot below.
        let chain = instrument.effect_chain();
        for slot in chain.slots() {
            if let crate::effect_chain::ChainSlot::Effect(effect_slot) = slot {
                let descriptor = effect_slot.effect.descriptor();
                let mut snapshot = ModuleStateSnapshot::new(
                    effect_slot.module_id,
                    instrument_id,
                    effect_slot.module_type,
                    descriptor.name.to_string(),
                );
                snapshot.parameters = effect_slot.effect.get_params();
                snapshot.bypass_state = if effect_slot.state.is_bypassed() {
                    synth_core::BypassState::Bypassed
                } else {
                    synth_core::BypassState::Active
                };
                shared.set_module(snapshot);
            }
        }

        // Build connection snapshots for this instrument
        let connections: Vec<ConnectionSnapshot> = graph
            .connections()
            .map(|c| {
                ConnectionSnapshot::new(
                    instrument_id,
                    c.from_module,
                    c.from_port,
                    c.to_module,
                    c.to_port,
                )
            })
            .collect();
        shared.set_connections_for_instrument(instrument_id, connections);

        // Update processing order
        shared.set_processing_order(graph.processing_order().to_vec());
    }

    /// Build and write instrument metadata snapshots to shared state.
    fn update_shared_instruments(&self) {
        let snapshots: Vec<crate::shared_state::InstrumentSnapshot> = self
            .instruments
            .iter()
            .map(|inst| {
                #[allow(clippy::cast_possible_truncation)]
                let seq_id = self
                    .instrument_mapping
                    .seq_id(inst.id())
                    .map_or(inst.id().as_u64() as u16, |s| s.0);
                let allocator_cfg = inst.allocator().config();
                crate::shared_state::InstrumentSnapshot {
                    id: inst.id(),
                    seq_instrument_id: seq_id,
                    name: inst.name().to_string(),
                    description: inst.description().to_string(),
                    patch_description: inst.patch_description().map(str::to_owned),
                    sidechain_source_id: inst.sidechain_source_id(),
                    category: inst.category(),
                    midi_channel: synth_core::MidiChannel::new(
                        inst.midi_channel().as_zero_indexed() + 1,
                    ),
                    volume: inst.volume(),
                    pan: inst.pan(),
                    enabled: inst.is_enabled(),
                    muted: !inst.is_enabled(),
                    solo: inst.is_solo(),
                    module_count: inst.voice_graph().len(),
                    effect_count: inst.effect_chain().slots().len(),
                    effect_chain_order: inst.effect_chain().slot_order(),
                    key_range: inst.key_range(),
                    transpose: inst.transpose(),
                    oversampling: inst.oversampling(),
                    allocation_mode: allocator_cfg.mode,
                    stealing_strategy: allocator_cfg.stealing,
                    max_voices: allocator_cfg.max_voices,
                    velocity_amp_sensitivity: inst.velocity_amp_sensitivity(),
                    velocity_filter_sensitivity: inst.velocity_filter_sensitivity(),
                }
            })
            .collect();
        *self.state.instrument_snapshots.write() = snapshots;
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
    /// Refresh the per-instrument [`TrackControl`] map from the sequencer
    /// `Song`, ready for the channel-bus stage.
    ///
    /// Real-time safe: `try_read()` only (on contention, last block's controls
    /// are kept); no allocation — every key is pre-allocated on instrument add,
    /// so this only resets and overwrites existing `Copy` values. Track →
    /// instrument resolution goes through `InstrumentMapping`. Instruments may
    /// be shared across tracks (intentional layering); when two tracks drive the
    /// same instrument the last one in `tracks()` order wins the fader for that
    /// block. Independent faders for a shared instrument would need per-voice
    /// tagging (channel-strip plan, Phase 8) — not built.
    fn update_track_controls(&mut self) {
        let Some(song) = self.sequencer.song().try_read() else {
            return;
        };
        for control in self.track_controls.values_mut() {
            *control = TrackControl::NEUTRAL;
        }
        let any_solo = song.any_solo();
        let track_auto = self.sequencer.track_auto();
        for track in song.tracks() {
            // Every track has an instrument; a shared instrument means a shared
            // fader (last track in iteration order wins for that block).
            let Some(engine_id) = self.instrument_mapping.engine_id(track.instrument) else {
                continue;
            };
            if let Some(control) = self.track_controls.get_mut(&engine_id) {
                // Live automation overrides the stored fader; absent → static.
                let auto = track_auto.get(&track.id).copied().unwrap_or_default();
                *control = TrackControl {
                    volume: auto.volume.unwrap_or(track.volume),
                    pan: auto.pan.unwrap_or(track.pan),
                    audible: track.is_audible(any_solo) && !auto.muted.unwrap_or(false),
                };
            }
        }
    }

    /// Apply `Global(..)` automation events for this block.
    ///
    /// `MasterVolume` writes the engine master gain (mirroring
    /// `handle_set_master_volume`: local field + shared atomic). `Tempo` is
    /// deferred to §2.1 — playback rate is driven by the sequencer's
    /// `cached_tempo` (from `Song::tempo_at`), not the transport atomic, so
    /// `set_tempo` alone would change the readout but not the render. `Swing`
    /// has no engine implementation. Track automation is handled in the
    /// sequencer (see `SequencerEngine::track_auto`); instrument automation in
    /// `route_sequencer_events`.
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
                    GlobalParam::Tempo | GlobalParam::Swing => {}
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

        // Prepare spatial context if per-voice spatial is active
        let spatial_enabled = self.awe_engine.enabled() && self.awe_engine.spatial_enabled();
        self.spatial_voice_bank.clear();

        let spatial_ctx = if spatial_enabled {
            let room = self.awe_engine.room();
            let snap = self.awe_engine.snapshot();
            Some(SpatialContext {
                mapping: self.awe_engine.note_mapping(),
                room_length: room.length(),
                room_width: room.width(),
                room_height: room.height(),
                listener_x: snap.listener_pos[0],
            })
        } else {
            None
        };

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

            active_count +=
                instrument.process(context, spatial_ctx.as_ref(), &mut self.spatial_voice_bank);
        }

        // Channel-bus stage: compose each channel's instrument fader with its
        // owning-track fader and sum into the master mix. Single insertion point
        // for track controls and sends/returns (Phase 7).
        mix_channel_busses(
            &self.instruments,
            any_soloed,
            &self.track_controls,
            &mut self.mix_buffer,
        );

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
struct TrackControl {
    volume: NormalizedValue,
    pan: BipolarValue,
    audible: bool,
}

impl TrackControl {
    const NEUTRAL: Self = Self {
        volume: NormalizedValue::MAX,
        pan: BipolarValue::CENTER,
        audible: true,
    };
}

/// Channel-bus stage: mix every instrument's channel into the master buffer.
///
/// Channel-strip model: each instrument is a channel whose post-effect,
/// **pre-fader** signal lives in its `effect_buffer` (read via
/// [`Instrument::last_output_interleaved`]). This stage composes the channel's
/// instrument fader with its owning-track fader, applies per-channel soft
/// clipping, and sums the result into `mix_buffer`:
/// - gain = `inst.volume × track.volume`
/// - pan  = `inst.pan + track.pan` (clamped to [-1, 1] by `BipolarValue::new`),
///   fed once through the constant-power pan law — additive, *not* two cascaded
///   pan stages (which would mis-handle hard-panned channels).
///
/// Moving the fader here (out of `Instrument::process`) makes the instrument a
/// pure sound source and gives a single insertion point for track controls and
/// sends/returns (Phase 7). The sidechain tap stays pre-fader: this stage never
/// writes gain back into `effect_buffer`.
///
/// Skips muted/non-soloed instruments (as in Phase 1) and, additionally,
/// channels whose owning track is inaudible (track mute / track-solo exclusion
/// via `SequencerTrack::is_audible`). Instruments with no track entry, or a
/// `NEUTRAL` entry, compose to their instrument-only fader unchanged.
fn mix_channel_busses(
    instruments: &[Box<Instrument>],
    any_soloed: bool,
    track_controls: &std::collections::HashMap<InstrumentId, TrackControl>,
    mix_buffer: &mut AudioBuffer,
) {
    let dst = mix_buffer.as_mut_slice();
    for instrument in instruments {
        if any_soloed && !instrument.is_solo() {
            continue;
        }
        if instrument.mute_state().is_muted() {
            continue;
        }

        let track = track_controls
            .get(&instrument.id())
            .copied()
            .unwrap_or(TrackControl::NEUTRAL);
        if !track.audible {
            continue;
        }

        // Compose instrument fader with track fader. Additive pan through one
        // constant-power law; multiplicative volume.
        let pan = BipolarValue::new(instrument.pan().as_f32() + track.pan.as_f32());
        let (pan_left, pan_right) = Gain::from_pan(pan);
        let volume = instrument.volume().as_f32() * track.volume.as_f32();
        let left_gain = pan_left.as_f32() * volume;
        let right_gain = pan_right.as_f32() * volume;

        let src = instrument.last_output_interleaved();
        let frames = src.len().min(dst.len());
        let mut i = 0;
        while i + 1 < frames {
            dst[i] += soft_clip(src[i] * left_gain);
            dst[i + 1] += soft_clip(src[i + 1] * right_gain);
            i += 2;
        }
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

/// Resolve a `SeqInstrumentId` to an instrument index using the stable mapping.
///
/// Falls back to index 0 (first instrument) if the mapping yields no match.
fn resolve_instrument_index(
    seq_id: &synth_sequencer::SeqInstrumentId,
    mapping: &InstrumentMapping,
    instruments: &[Box<Instrument>],
) -> Option<usize> {
    if let Some(engine_id) = mapping.engine_id(*seq_id)
        && let Some(idx) = instruments.iter().position(|i| i.id() == engine_id)
    {
        return Some(idx);
    }
    // Fallback: first instrument (orphaned notes still produce sound)
    if instruments.is_empty() {
        None
    } else {
        Some(0)
    }
}

/// Route sequencer events to the appropriate instruments.
///
/// Uses `InstrumentMapping` for stable lookup instead of vec-index casting.
/// Also pushes note events to the OSC telemetry ring buffer.
fn route_sequencer_events(
    events: &[SequencerEvent],
    instruments: &mut [Box<Instrument>],
    mapping: &InstrumentMapping,
    note_event_producer: &mut ringbuf::HeapProd<NoteEvent>,
    event_drops: &std::sync::atomic::AtomicU32,
) {
    for event in events {
        match event {
            SequencerEvent::NoteOn {
                pitch,
                velocity,
                instrument,
                ..
            } => {
                let note = MidiNote::new(pitch.as_midi());
                let vel = *velocity;

                if let Some(idx) = resolve_instrument_index(instrument, mapping, instruments) {
                    instruments[idx].note_on(note, vel);

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

                if let Some(idx) = resolve_instrument_index(instrument, mapping, instruments) {
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
                    && let Some(engine_id) = mapping.engine_id(*instrument)
                    && let Some(inst) = instruments.iter_mut().find(|i| i.id() == engine_id)
                {
                    // Module-targeted params resolve to the *first* module of
                    // the relevant type in the instrument graph (the A1
                    // convention) and apply through the transient override path,
                    // denormalizing 0..1 via the descriptor range. Overrides are
                    // reverted to base on transport stop (`handle_all_notes_off`).
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
                    && let Some(engine_id) = mapping.engine_id(*instrument)
                    && let Some(inst) = instruments.iter_mut().find(|i| i.id() == engine_id)
                {
                    // Generic A2 target: the positional (module_type, instance)
                    // identity is rebuilt into a ModuleId, the parameter resolved
                    // by descriptor type_id, and the 0..1 value denormalized via
                    // that param's range. Reverted on stop (`handle_all_notes_off`).
                    inst.apply_module_param_override(*module_type, *instance, param_id, *value);
                }
            }
        }
    }
}

impl AudioProcessor for SynthEngine {
    fn process(&mut self, output: &mut [f32], context: &AudioCallbackContext) {
        let start_time = Instant::now();

        // Retry sending pending recorded notes from previous cycle
        if let Some((pattern_id, notes, overdub)) = self.pending_recorded_notes.take() {
            self.flush_recorded_notes(pattern_id, notes, overdub);
        }

        // Process commands
        self.process_commands();

        let sample_count = SampleCount::new(context.frames);

        // Drain ALL available audio input samples into pre-allocated buffer.
        // We drain everything (not just stereo_samples) to handle clock drift:
        // if the input device is slightly faster than the output device, excess
        // samples accumulate. We keep only the most recent block worth of data.
        let stereo_samples = context.frames * 2;
        // No resize — buffer is pre-allocated to 8192. If block > 4096 frames,
        // we silently cap to buffer size (avoids RT allocation).
        let buf_cap = self.audio_input_buffer.len();
        let usable = stereo_samples.min(buf_cap);
        self.audio_input_buffer[..usable].fill(0.0);

        if let Some(ref mut consumer) = self.audio_input_consumer {
            // Drain everything available from the ring buffer to handle clock drift.
            // We keep only the most recent `usable` samples. If the input device is
            // slightly faster than the output, excess samples are discarded.
            let mut write_idx = 0;
            while let Some(sample) = consumer.try_pop() {
                if write_idx < usable {
                    self.audio_input_buffer[write_idx] = sample;
                } else {
                    // Overflow: shift buffer left and append at end (keep latest)
                    self.audio_input_buffer.copy_within(1..usable, 0);
                    self.audio_input_buffer[usable - 1] = sample;
                }
                write_idx += 1;
            }
        }
        let has_input = self.audio_input_consumer.is_some();

        // SAFETY: The audio_input_buffer is pre-allocated and not modified during
        // the remainder of this process() call. We extend its lifetime to decouple
        // it from `self` so that `self` can be borrowed mutably for instrument
        // processing while the buffer reference remains valid in ProcessContext.
        let audio_input_ref: Option<&[f32]> = if has_input {
            let ptr = self.audio_input_buffer.as_ptr();
            Some(unsafe { std::slice::from_raw_parts(ptr, usable) })
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
            &self.instrument_mapping,
            &mut self.note_event_producer,
            &self.state.event_drops,
        );

        // Apply global automation (master volume) for this block.
        self.apply_global_automation();

        // Refresh per-instrument track controls from the Song before the
        // channel-bus stage (inside process_voices) reads them. Track-fader
        // automation is folded in here via SequencerEngine::track_auto.
        self.update_track_controls();

        self.process_voices(&process_context);

        // Process modular graph (user-added modules)
        self.process_module_graph(&process_context);

        // Process master effects (master bus: reverb, limiter, EQ, etc.)
        self.process_master_effects(&process_context);

        // Process AWE (room simulation) after master effects
        if self.awe_engine.enabled() {
            let sr = SampleRate::new(self.sample_rate);
            if self.awe_engine.spatial_enabled() && self.spatial_voice_bank.active_count() > 0 {
                self.awe_engine.process_spatial(
                    self.mix_buffer.as_mut_slice(),
                    &self.spatial_voice_bank,
                    sr,
                );
            } else {
                self.awe_engine.process(self.mix_buffer.as_mut_slice(), sr);
            }
        }

        // Mix metronome click into output
        self.click_generator
            .process(self.mix_buffer.as_mut_slice(), context.frames);

        // Process master-level visualizers after AWE (so they show final signal)
        self.master_effects.process_visualizers(&self.mix_buffer);

        // Copy to output with master volume
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
                frame[0] = (left * self.master_volume).clamp(-1.0, 1.0);
                frame[1] = (right * self.master_volume).clamp(-1.0, 1.0);
            } else if channels == 1 {
                frame[0] = ((left + right) * 0.5 * self.master_volume).clamp(-1.0, 1.0);
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

        // Calculate CPU usage
        let elapsed = start_time.elapsed().as_secs_f32();
        let buffer_duration = context.frames as f32 / self.sample_rate;
        self.callback_duration_sum += elapsed;
        self.callback_count += 1;

        if self.callback_count >= 100 {
            let avg_duration = self.callback_duration_sum / self.callback_count as f32;
            let cpu_usage = avg_duration / buffer_duration;
            self.state.cpu_usage.store(cpu_usage);
            self.callback_duration_sum = 0.0;
            self.callback_count = 0;
        }
    }

    fn on_stream_start(&mut self, info: &StreamInfo) {
        self.sample_rate = info.sample_rate.as_f32();
        self.state.sample_rate.store(info.sample_rate.0);
        let sr = SampleRate::new(self.sample_rate);
        self.metering.set_sample_rate(sr);
        self.sequencer.set_sample_rate(sr);
        self.click_generator.set_sample_rate(sr);
        // AWE delay lines depend on sample rate — recalculate on next process()
        self.awe_engine.mark_geometry_dirty();
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
mod tests {
    use super::*;
    use crate::voice_allocator::{AllocationMode, AllocatorConfig, VoiceAllocator};
    use synth_core::{ModuleType, VoiceCount};

    /// Create a default instrument and add it to the engine via command.
    fn add_default_instrument(engine: &mut SynthEngine, handle: &mut EngineHandle) {
        let mut instrument =
            Instrument::with_config(InstrumentId::FIRST, "Default", AllocatorConfig::default());
        instrument.set_midi_channel(MidiChannel::CH1);
        SynthEngine::populate_default_voice_graph(instrument.voice_graph_mut());
        *instrument.allocator_mut() = VoiceAllocator::with_graph_template(
            instrument.allocator().config().clone(),
            instrument.voice_graph(),
        );
        handle.send(EngineCommand::AddInstrument {
            instrument: Box::new(instrument),
        });
        engine.process_commands();
    }

    fn add_instrument_with_config(
        engine: &mut SynthEngine,
        handle: &mut EngineHandle,
        config: AllocatorConfig,
    ) {
        let mut instrument =
            Instrument::with_config(InstrumentId::FIRST, "Default", config.clone());
        instrument.set_midi_channel(MidiChannel::CH1);
        SynthEngine::populate_default_voice_graph(instrument.voice_graph_mut());
        *instrument.allocator_mut() = VoiceAllocator::with_graph_template(
            instrument.allocator().config().clone(),
            instrument.voice_graph(),
        );
        handle.send(EngineCommand::AddInstrument {
            instrument: Box::new(instrument),
        });
        engine.process_commands();
    }

    #[test]
    fn test_engine_creation() {
        let (engine, handle) = SynthEngine::new();
        assert_eq!(engine.instruments.len(), 0);
        assert_eq!(handle.voice_count(), 0);
        assert!((handle.master_volume() - 1.0).abs() < 0.001);
        drop(engine);
    }

    #[test]
    fn test_polyphonic_notes() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let (mut engine, mut handle) = SynthEngine::new();
        add_instrument_with_config(&mut engine, &mut handle, config);

        // Send multiple notes
        handle.note_on(MidiNote::C4, Velocity::new(0.8));
        handle.note_on(MidiNote::new(64), Velocity::new(0.8));
        handle.note_on(MidiNote::new(67), Velocity::new(0.8));

        // Process commands
        engine.process_commands();

        // Should have 3 active voices across all instruments
        let total_active: usize = engine
            .instruments
            .iter()
            .map(|p| p.active_voice_count())
            .sum();
        assert_eq!(total_active, 3);
    }

    #[test]
    fn test_engine_starts_empty() {
        let (engine, _handle) = SynthEngine::new();
        assert_eq!(engine.instruments.len(), 0);
    }

    #[test]
    fn test_add_instrument_via_command() {
        let (mut engine, mut handle) = SynthEngine::new();
        add_default_instrument(&mut engine, &mut handle);

        assert_eq!(engine.instruments.len(), 1);
        assert_eq!(engine.instruments[0].id(), InstrumentId::FIRST);
        assert_eq!(engine.instruments[0].name(), "Default");
        assert_eq!(engine.instruments[0].midi_channel(), MidiChannel::CH1);
    }

    #[test]
    fn test_part_channel_routing() {
        let (mut engine, mut handle) = SynthEngine::new();
        add_default_instrument(&mut engine, &mut handle);

        // Send note on channel 1 - should be received
        handle.note_on_channel(
            MidiNote::C4,
            Velocity::new(0.8),
            crate::instrument::MidiChannel::CH1,
        );
        engine.process_commands();
        assert_eq!(engine.instruments[0].active_voice_count(), 1);

        // Send note on channel 2 - should NOT be received
        let ch2 = crate::instrument::MidiChannel::from_one_indexed(2).unwrap();
        handle.note_on_channel(MidiNote::new(64), Velocity::new(0.8), ch2);
        engine.process_commands();
        assert_eq!(engine.instruments[0].active_voice_count(), 1); // Still 1
    }

    /// Regression tests for dynamic routing.
    ///
    /// These tests verify that modules are correctly routed to either:
    /// - instrument.voice_graph (for polyphonic voice modules like Oscillator, Filter, etc.)
    /// - module_graph (for global effects like Reverb, Delay, etc.)
    mod dynamic_routing {
        use super::*;
        use crate::commands::{EngineCommand, ModuleId, PortId};
        use crate::instrument::InstrumentId;
        use synth_modules::{Filter, Oscillator};

        /// Test A: Polyphonic Allocation
        /// An Oscillator should be added to instrument's voice_graph, NOT to module_graph.
        #[test]
        fn test_oscillator_routed_to_voice_graph() {
            let (mut engine, mut handle) = SynthEngine::new();
            add_default_instrument(&mut engine, &mut handle);

            // Count existing oscillators in default instrument's voice graph (there are 2 by default)
            let initial_osc_count = engine.instruments[0]
                .voice_graph()
                .module_ids()
                .filter(|id| id.module_type == ModuleType::Oscillator)
                .count();

            // Create a new oscillator
            let osc_id = ModuleId::new(ModuleType::Oscillator, 10);
            let osc = Box::new(Oscillator::new());

            // Send command to add module to the default instrument
            handle.send(EngineCommand::AddModuleInstance {
                instrument_id: Some(InstrumentId::FIRST),
                id: osc_id,
                module: osc,
            });
            engine.process_commands();

            // Verify: Oscillator should be in instrument's voice_graph
            assert!(
                engine.instruments[0]
                    .voice_graph()
                    .get_module(osc_id)
                    .is_some(),
                "Oscillator should be in instrument's voice_graph"
            );

            // Verify: Oscillator should NOT be in module_graph
            assert!(
                engine.module_graph.get_module(osc_id).is_none(),
                "Oscillator should NOT be in module_graph"
            );

            // Verify: voice_graph oscillator count increased
            let final_osc_count = engine.instruments[0]
                .voice_graph()
                .module_ids()
                .filter(|id| id.module_type == ModuleType::Oscillator)
                .count();
            assert_eq!(final_osc_count, initial_osc_count + 1);
        }

        // Note: Effects (Reverb, Delay, etc.) don't implement PolyModule,
        // so they can't be added via AddModuleInstance. They use the separate
        // effect chain mechanism instead.

        /// Test B: Voice Propagation
        /// Adding a module to instrument's voice_graph should propagate to all its voices.
        #[test]
        fn test_voice_module_propagates_to_voices() {
            let config = AllocatorConfig {
                max_voices: VoiceCount::QUAD,
                mode: AllocationMode::Polyphonic,
                ..Default::default()
            };
            let (mut engine, mut handle) = SynthEngine::new();
            add_instrument_with_config(&mut engine, &mut handle, config);

            // Create a new filter with a unique ID
            let filter_id = ModuleId::new(ModuleType::Filter, 10);
            let filter = Box::new(Filter::new());

            // First, verify that voices don't have this filter yet
            for voice in engine.instruments[0].allocator().voices() {
                assert!(
                    voice.graph.get_module(filter_id).is_none(),
                    "Voice should not have filter_id before AddModuleInstance"
                );
            }

            // Send command to add module to default instrument
            handle.send(EngineCommand::AddModuleInstance {
                instrument_id: Some(InstrumentId::FIRST),
                id: filter_id,
                module: filter,
            });
            engine.process_commands();

            // Verify: All voices in the default instrument should have the new filter
            for (i, voice) in engine.instruments[0]
                .allocator()
                .voices()
                .iter()
                .enumerate()
            {
                assert!(
                    voice.graph.get_module(filter_id).is_some(),
                    "Voice {} should have filter_id after AddModuleInstance",
                    i
                );
            }
        }

        /// Test D: Voice module connections propagate to all voices
        #[test]
        fn test_voice_connection_propagates_to_voices() {
            let config = AllocatorConfig {
                max_voices: VoiceCount::DUAL,
                mode: AllocationMode::Polyphonic,
                ..Default::default()
            };
            let (mut engine, mut handle) = SynthEngine::new();
            add_instrument_with_config(&mut engine, &mut handle, config);

            // Add a new oscillator and amplifier to default instrument's voice graph
            let new_osc_id = ModuleId::new(ModuleType::Oscillator, 10);
            let new_amp_id = ModuleId::new(ModuleType::Amplifier, 10);

            handle.send(EngineCommand::AddModuleInstance {
                instrument_id: Some(InstrumentId::FIRST),
                id: new_osc_id,
                module: Box::new(Oscillator::new()),
            });
            handle.send(EngineCommand::AddModuleInstance {
                instrument_id: Some(InstrumentId::FIRST),
                id: new_amp_id,
                module: Box::new(synth_modules::Amplifier::new()),
            });
            engine.process_commands();

            // Connect new osc -> new amp in instrument's voice graph
            handle.send(EngineCommand::Connect {
                instrument_id: Some(InstrumentId::FIRST),
                from: PortId::new(new_osc_id, "out"),
                to: PortId::new(new_amp_id, "in"),
            });
            engine.process_commands();

            // Verify: instrument's voice_graph has the connection
            let template_connections: Vec<_> =
                engine.instruments[0].voice_graph().connections().collect();
            let has_connection = template_connections
                .iter()
                .any(|c| c.from_module == new_osc_id && c.to_module == new_amp_id);
            assert!(
                has_connection,
                "instrument's voice_graph should have the connection"
            );

            // Verify: All voices have the connection
            for (i, voice) in engine.instruments[0]
                .allocator()
                .voices()
                .iter()
                .enumerate()
            {
                let voice_connections: Vec<_> = voice.graph.connections().collect();
                let has_connection = voice_connections
                    .iter()
                    .any(|c| c.from_module == new_osc_id && c.to_module == new_amp_id);
                assert!(has_connection, "Voice {} should have the connection", i);
            }
        }

        /// Test E: ModuleType classification methods work correctly
        /// (Tests moved to src/engine/params/mod.rs - ModuleType now owns this logic)
        #[test]
        fn test_module_type_classification() {
            // Voice modules (should be true)
            assert!(ModuleType::Oscillator.is_voice_module());
            assert!(ModuleType::Filter.is_voice_module());
            assert!(ModuleType::StereoOutput.is_voice_module());

            // Effects (should be true for is_effect)
            assert!(ModuleType::Delay.is_effect());
            assert!(ModuleType::Reverb.is_effect());

            // Visualizers
            assert!(ModuleType::Oscilloscope.is_visualizer());
            assert!(ModuleType::LevelMeter.is_visualizer());

            // Global = !is_voice_module
            assert!(ModuleType::Delay.is_global());
            assert!(!ModuleType::Oscillator.is_global());
        }

        /// Test F: Remove voice module propagates to voices
        #[test]
        fn test_remove_voice_module_propagates() {
            let config = AllocatorConfig {
                max_voices: VoiceCount::DUAL,
                ..Default::default()
            };
            let (mut engine, mut handle) = SynthEngine::new();
            add_instrument_with_config(&mut engine, &mut handle, config);

            // Add a filter to default instrument
            let filter_id = ModuleId::new(ModuleType::Filter, 10);
            handle.send(EngineCommand::AddModuleInstance {
                instrument_id: Some(InstrumentId::FIRST),
                id: filter_id,
                module: Box::new(Filter::new()),
            });
            engine.process_commands();

            // Verify it exists
            assert!(
                engine.instruments[0]
                    .voice_graph()
                    .get_module(filter_id)
                    .is_some()
            );
            for voice in engine.instruments[0].allocator().voices() {
                assert!(voice.graph.get_module(filter_id).is_some());
            }

            // Remove it
            handle.send(EngineCommand::RemoveModule {
                instrument_id: Some(InstrumentId::FIRST),
                id: filter_id,
            });
            engine.process_commands();

            // Verify it's gone from instrument's voice_graph
            assert!(
                engine.instruments[0]
                    .voice_graph()
                    .get_module(filter_id)
                    .is_none()
            );

            // Verify it's gone from all voices
            for voice in engine.instruments[0].allocator().voices() {
                assert!(voice.graph.get_module(filter_id).is_none());
            }
        }
    }

    // --- Orphan-preview lifecycle (channel-strip plan §5) ------------------
    //
    // The preview target must be cleared on every transport reset; stale
    // preview surviving Stop/SetSong/solo was the v0.290.0 bug class. These
    // pin the command-handler teardown (previously untested).

    fn enter_preview(engine: &mut SynthEngine, handle: &mut EngineHandle) {
        handle.send(EngineCommand::SetPreviewPattern(Some((
            synth_sequencer::PatternId(0),
            synth_sequencer::SeqInstrumentId(0),
        ))));
        engine.process_commands();
        assert_eq!(
            engine.sequencer.preview_pattern(),
            Some(synth_sequencer::PatternId(0)),
            "precondition: preview should be active"
        );
    }

    #[test]
    fn stop_clears_orphan_preview() {
        let (mut engine, mut handle) = SynthEngine::new();
        enter_preview(&mut engine, &mut handle);
        handle.send(EngineCommand::Stop);
        engine.process_commands();
        assert_eq!(engine.sequencer.preview_pattern(), None);
    }

    #[test]
    fn set_song_clears_orphan_preview() {
        let (mut engine, mut handle) = SynthEngine::new();
        enter_preview(&mut engine, &mut handle);
        handle.send(EngineCommand::SetSong {
            song: std::sync::Arc::new(parking_lot::RwLock::new(synth_sequencer::Song::default())),
        });
        engine.process_commands();
        assert_eq!(engine.sequencer.preview_pattern(), None);
    }

    #[test]
    fn solo_pattern_and_preview_are_mutually_exclusive() {
        let (mut engine, mut handle) = SynthEngine::new();
        enter_preview(&mut engine, &mut handle);

        // Entering solo clears preview.
        handle.send(EngineCommand::SetSoloPattern(Some(
            synth_sequencer::PatternId(1),
        )));
        engine.process_commands();
        assert_eq!(engine.sequencer.preview_pattern(), None);
        assert_eq!(
            engine.sequencer.solo_pattern(),
            Some(synth_sequencer::PatternId(1))
        );

        // Entering preview clears solo.
        enter_preview(&mut engine, &mut handle);
        assert_eq!(engine.sequencer.solo_pattern(), None);
    }

    #[test]
    fn filter_cutoff_automation_dispatches_through_override() {
        use synth_core::{SampleCount, SampleRate};
        use synth_modules::{Filter, Oscillator};
        use synth_sequencer::{SeqInstrumentId, Tick};

        // Instrument graph: Osc -> Filter (sink). No voices allocated, so the
        // override lands on the template graph that we process directly.
        let mut instrument = Box::new(Instrument::new(InstrumentId::new(1), "test"));
        let g = instrument.voice_graph_mut();
        let osc_id = g.add_module(Box::new(Oscillator::new()));
        let flt_id = g.add_module(Box::new(Filter::new()));
        g.connect(osc_id, "out", flt_id, "in").unwrap();
        let mut instruments = vec![instrument];

        let mut mapping = InstrumentMapping::new();
        mapping.insert(SeqInstrumentId(7), InstrumentId::new(1));

        let note_rb = HeapRb::<NoteEvent>::new(16);
        let (mut note_prod, _note_cons) = note_rb.split();
        let drops = std::sync::atomic::AtomicU32::new(0);

        let ctx = ProcessContext {
            samples: SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        // Warm-up blocks first so the filter reaches steady state before
        // measuring (avoids start/retune transients dominating the energy).
        fn settled_energy(graph: &mut ModuleGraph, ctx: &ProcessContext<'_>) -> f32 {
            let mut out = AudioBuffer::new(256);
            for _ in 0..16 {
                graph.process(&mut out, ctx);
            }
            graph.process(&mut out, ctx);
            (0..256).map(|i| out[i] * out[i]).sum()
        }

        let base = settled_energy(instruments[0].voice_graph_mut(), &ctx);
        assert!(base > 1e-3, "expected audible base output, got {base}");

        // FilterCutoff automation at 0.0 -> 20 Hz cutoff via the dispatch path.
        let events = vec![SequencerEvent::Parameter {
            tick: Tick(0),
            target: AutomationTarget::Instrument {
                instrument: SeqInstrumentId(7),
                param: AutoInstrumentParam::FilterCutoff,
            },
            value: NormalizedValue::MIN,
        }];
        route_sequencer_events(&events, &mut instruments, &mapping, &mut note_prod, &drops);

        let low = settled_energy(instruments[0].voice_graph_mut(), &ctx);
        assert!(
            low < base * 0.25,
            "automation should have lowered the cutoff: {low} vs base {base}"
        );
    }

    #[test]
    fn module_automation_target_dispatches_through_override() {
        use synth_core::{SampleCount, SampleRate};
        use synth_modules::{Filter, Oscillator};
        use synth_sequencer::{SeqInstrumentId, Tick};

        // Osc -> Filter (sink); the filter is ModuleId(Filter, instance 1).
        let mut instrument = Box::new(Instrument::new(InstrumentId::new(1), "test"));
        let g = instrument.voice_graph_mut();
        let osc_id = g.add_module(Box::new(Oscillator::new()));
        let flt_id = g.add_module(Box::new(Filter::new()));
        g.connect(osc_id, "out", flt_id, "in").unwrap();
        let mut instruments = vec![instrument];

        let mut mapping = InstrumentMapping::new();
        mapping.insert(SeqInstrumentId(7), InstrumentId::new(1));

        let note_rb = HeapRb::<NoteEvent>::new(16);
        let (mut note_prod, _note_cons) = note_rb.split();
        let drops = std::sync::atomic::AtomicU32::new(0);

        let ctx = ProcessContext {
            samples: SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        fn settled_energy(graph: &mut ModuleGraph, ctx: &ProcessContext<'_>) -> f32 {
            let mut out = AudioBuffer::new(256);
            for _ in 0..16 {
                graph.process(&mut out, ctx);
            }
            graph.process(&mut out, ctx);
            (0..256).map(|i| out[i] * out[i]).sum()
        }

        let base = settled_energy(instruments[0].voice_graph_mut(), &ctx);
        assert!(base > 1e-3, "expected audible base output, got {base}");

        // Generic Module target: first Filter, "cutoff" param, 0.0 -> 20 Hz.
        let events = vec![SequencerEvent::Parameter {
            tick: Tick(0),
            target: AutomationTarget::Module {
                instrument: SeqInstrumentId(7),
                module_type: ModuleType::Filter,
                instance: 1,
                param_id: "cutoff".to_string(),
            },
            value: NormalizedValue::MIN,
        }];
        route_sequencer_events(&events, &mut instruments, &mapping, &mut note_prod, &drops);

        let low = settled_energy(instruments[0].voice_graph_mut(), &ctx);
        assert!(
            low < base * 0.25,
            "module automation should have lowered the cutoff: {low} vs base {base}"
        );
    }
}
