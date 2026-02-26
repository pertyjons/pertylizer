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

use crate::commands::{EngineCommand, EngineEvent, ModuleId, PortId};
use crate::effect_chain::{EffectChain, EffectSlot};
use crate::graph::ModuleGraph;
use crate::instrument::{Instrument, InstrumentId, MidiChannel};
use crate::instrument_mapping::InstrumentMapping;
use crate::metering::MeteringSystem;
use crate::sequencer_engine::SequencerEngine;
use crate::shared_state::{ConnectionSnapshot, ModuleStateSnapshot};
use crate::state::EngineState;
use crate::visualizers::{LevelMeter, Oscilloscope, SpectrumAnalyzer, VisualizationBuffer};
use crate::voice_allocator::{AllocatorConfig, VoiceAllocator};
use synth_awe::{AweEngine, SpatialContext, SpatialVoiceBank};
use synth_core::{
    AmplifierParam, AudioBuffer, AudioCallbackContext, AudioProcessor, BeatPosition, BipolarValue,
    Bpm, EnvelopeParam, FilterParam, Gain, LfoParam, LfoWaveform, MidiNote, ModuleType,
    NormalizedValue, OscillatorParam, Param, PolyModule as PolyModuleTrait, ProcessContext,
    SampleCount, SampleRate, Seconds, StreamInfo, Velocity, Waveform,
};
use synth_modules::{Amplifier, Envelope, Filter, Lfo, Oscillator};
use synth_sequencer::{AutoInstrumentParam, AutomationTarget, SequencerEvent};

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
    /// * `effect_type` - The type of effect to modify
    /// * `param` - The parameter to set
    pub fn set_effect_parameter(
        &mut self,
        instrument_id: Option<InstrumentId>,
        effect_type: crate::commands::EffectType,
        param: synth_core::Param,
    ) -> bool {
        self.send(EngineCommand::SetEffectParameter {
            instrument_id,
            effect_type,
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
    pub fn peak_meters(&self) -> (f32, f32) {
        self.state.meters.get_peak()
    }

    /// Get the current RMS meter values.
    pub fn rms_meters(&self) -> (f32, f32) {
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

    /// Get the currently focused instrument index.
    /// Returns None if using traditional MIDI channel routing.
    pub fn get_focused_instrument(&self) -> Option<u32> {
        self.state.get_focused_instrument()
    }
}

/// The main synthesizer engine with polyphony.
pub struct SynthEngine {
    /// Receive commands from UI.
    command_consumer: ringbuf::HeapCons<EngineCommand>,
    /// Send events to UI.
    event_producer: ringbuf::HeapProd<EngineEvent>,
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

    // === Metering ===
    metering: MeteringSystem,

    // === Sequencer ===
    sequencer: SequencerEngine,
    /// Pre-allocated buffer for sequencer events (real-time safe).
    sequencer_event_buffer: Vec<SequencerEvent>,
    /// Stable mapping from sequencer instrument IDs to engine instrument IDs.
    /// Convention: `SeqInstrumentId(X)` ↔ `InstrumentId(X)`.
    instrument_mapping: InstrumentMapping,

    // === Performance monitoring ===
    callback_duration_sum: f32,
    callback_count: u32,
}

impl SynthEngine {
    /// Create a new engine with default configuration.
    pub fn new() -> (Self, EngineHandle) {
        Self::with_config(AllocatorConfig::default())
    }

    /// Create a new engine with custom configuration.
    pub fn with_config(config: AllocatorConfig) -> (Self, EngineHandle) {
        let state = EngineState::new();

        // Create command ring buffer
        let command_rb = HeapRb::<EngineCommand>::new(COMMAND_BUFFER_SIZE);
        let (command_producer, command_consumer) = command_rb.split();

        // Create event ring buffer
        let event_rb = HeapRb::<EngineEvent>::new(EVENT_BUFFER_SIZE);
        let (event_producer, event_consumer) = event_rb.split();

        // Create return buffer for modules to be dropped on main thread
        let return_rb = HeapRb::<DroppedModule>::new(RETURN_BUFFER_SIZE);
        let (return_producer, return_consumer) = return_rb.split();

        // Create return buffer for instruments to be dropped on main thread
        let instrument_return_rb = HeapRb::<Box<Instrument>>::new(RETURN_BUFFER_SIZE);
        let (instrument_return_producer, instrument_return_consumer) = instrument_return_rb.split();

        // Create default instrument on Channel 1 (strict channel separation)
        let mut default_instrument =
            Instrument::with_config(InstrumentId::FIRST, "Default", config);
        default_instrument.set_midi_channel(MidiChannel::CH1);

        // Populate the instrument's voice graph with default signal chain
        Self::populate_default_voice_graph(default_instrument.voice_graph_mut());

        // Initialize allocator with the instrument's voice graph
        *default_instrument.allocator_mut() = VoiceAllocator::with_graph_template(
            default_instrument.allocator().config().clone(),
            default_instrument.voice_graph(),
        );

        // Initialize instrument mapping with default instrument
        let mut instrument_mapping = InstrumentMapping::new();
        instrument_mapping.insert(synth_sequencer::SeqInstrumentId(0), InstrumentId::FIRST);

        let engine = Self {
            command_consumer,
            event_producer,
            return_producer,
            instrument_return_producer,
            state: Arc::clone(&state),
            instruments: vec![Box::new(default_instrument)],
            master_effects: EffectChain::new(),
            awe_engine: AweEngine::new(),
            spatial_voice_bank: SpatialVoiceBank::new(),
            module_graph: ModuleGraph::new(),
            use_modular_routing: false,
            sample_rate: 48000.0,
            master_volume: 1.0,
            mix_buffer: AudioBuffer::new(512),
            graph_output: AudioBuffer::new(1024),
            metering: MeteringSystem::new(48000.0),
            sequencer: SequencerEngine::new(synth_core::SampleRate::DVD_QUALITY),
            sequencer_event_buffer: Vec::with_capacity(128),
            instrument_mapping,
            callback_duration_sum: 0.0,
            callback_count: 0,
        };

        // Initialize instrument snapshots so MCP/GUI see the default instrument
        engine.update_shared_instruments();
        // Initialize shared graph for default instrument
        engine.update_shared_graph_for_instrument(&engine.instruments[0]);

        let handle = EngineHandle {
            command_sender: CommandSender::new(command_producer),
            event_consumer,
            return_consumer,
            instrument_return_consumer,
            state,
            visualization_buffers: HashMap::new(),
        };

        (engine, handle)
    }

    /// Find an effect slot by its module type in a specific instrument's effect chain.
    fn find_effect_by_type(
        &mut self,
        instrument_id: InstrumentId,
        module_type: ModuleType,
    ) -> Option<&mut EffectSlot> {
        self.instruments
            .iter_mut()
            .find(|i| i.id() == instrument_id)
            .and_then(|inst| inst.effect_chain_mut().find_effect_by_type(module_type))
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
    #[allow(dead_code)] // Useful for targeted effect updates
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
    fn populate_default_voice_graph(graph: &mut ModuleGraph) {
        use synth_core::{Cents, Hertz, Seconds as TypedSeconds};

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
            }
            EngineCommand::ModWheel { value, channel } => {
                self.handle_mod_wheel(value, channel);
            }
            EngineCommand::Aftertouch { value, channel } => {
                self.handle_aftertouch(value, channel);
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
                    self.state.shared_graph.remove_module(m.id);
                }
                self.update_shared_instruments();
            }

            // Effects
            EngineCommand::SetBypass { module, bypass } => {
                self.handle_set_bypass(module, bypass);
            }
            EngineCommand::SetEffectParameter {
                instrument_id,
                effect_type,
                param,
            } => {
                self.handle_set_effect_param(instrument_id, effect_type, param);
            }
            EngineCommand::SetEffectEnabled {
                instrument_id,
                effect_type,
                enabled,
            } => {
                self.handle_set_effect_enabled(instrument_id, effect_type, enabled);
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
                self.sequencer.play();
                self.state.transport.set_playing(true);
            }
            EngineCommand::Stop => {
                let _ = self.sequencer.stop();
                self.state.transport.set_playing(false);

                // Release all voices on all instruments
                // This is necessary because sequencer.stop() returns events that
                // would need to be processed, but we're not in the audio callback.
                // Direct voice release is more reliable.
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
            EngineCommand::PlayPattern { pattern_id } => {
                // Find pattern in arrangement and get boundaries
                let bounds = self
                    .sequencer
                    .song()
                    .read()
                    .ok()
                    .and_then(|song| Self::find_pattern_bounds(&song, pattern_id));

                if let Some((start, end)) = bounds {
                    // Important: play() first to avoid it resetting current_tick to 0
                    self.sequencer.play();
                    let _ = self.sequencer.seek(start);
                    self.sequencer.set_loop(start, end, true);
                    self.state.transport.set_playing(true);
                    self.state.transport.set_ticks(start.0);
                } else {
                    // Fallback: pattern not in arrangement, just play from beginning
                    self.sequencer.play();
                    self.state.transport.set_playing(true);
                }
            }
            EngineCommand::PlayFromPattern { pattern_id } => {
                // Start playback from pattern start, no loop
                let bounds = self
                    .sequencer
                    .song()
                    .read()
                    .ok()
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
                    self.state.transport.set_playing(true);
                    self.state.transport.set_ticks(start.0);
                } else {
                    // Fallback: pattern not in arrangement, just play from beginning
                    self.sequencer.play();
                    self.state.transport.set_playing(true);
                }
            }
            EngineCommand::SetSong { song } => {
                self.sequencer.set_song(song);
            }
            EngineCommand::SetTempo(bpm) => {
                self.state.transport.set_tempo(bpm.as_f32());
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
        }
    }

    // ========================================================================
    // Pattern navigation helpers
    // ========================================================================

    /// Find the start and end tick for a pattern's first occurrence in the arrangement.
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

        // Check if there's a focused instrument for keyboard input
        let focused_idx = self.state.get_focused_instrument();

        for (idx, instrument) in self.instruments.iter_mut().enumerate() {
            // If focused instrument is set, only that instrument receives keyboard input
            // (Channel 0 is the default keyboard channel)
            if let Some(focus_idx) = focused_idx {
                // Only send to focused instrument, and only for channel 0 (keyboard)
                if channel_raw == 0 && idx != focus_idx as usize {
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
            }
        }

        if self.use_modular_routing {
            self.module_graph.note_on(note, velocity);
            note_triggered = true;
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
        let focused_idx = self.state.get_focused_instrument();

        for (idx, instrument) in self.instruments.iter_mut().enumerate() {
            // Same logic as note_on for focused instrument routing
            if let Some(focus_idx) = focused_idx {
                if channel_raw == 0 && idx != focus_idx as usize {
                    continue;
                }
                if channel_raw != 0 && !instrument.responds_to_channel(channel_raw) {
                    continue;
                }
            } else if !instrument.responds_to_channel(channel_raw) {
                continue;
            }
            instrument.note_off(note);
        }

        if self.use_modular_routing {
            self.module_graph.note_off();
        }

        let _ = self
            .event_producer
            .try_push(EngineEvent::NoteReleased { note, channel });
    }

    fn handle_all_notes_off(&mut self) {
        for instrument in &mut self.instruments {
            instrument.all_notes_off();
        }

        if self.use_modular_routing {
            self.module_graph.reset();
        }

        let _ = self.event_producer.try_push(EngineEvent::AllNotesReleased);
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
        // Convert InstrumentId to u32 index for atomic storage
        let index = instrument_id.and_then(|id| {
            self.instruments
                .iter()
                .position(|i| i.id() == id)
                .map(|pos| pos as u32)
        });
        self.state.set_focused_instrument(index);
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

    fn handle_reset(&mut self) {
        for instrument in &mut self.instruments {
            instrument.panic();
            instrument.effect_chain_mut().reset();
        }
        self.master_effects.reset();
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

    fn handle_set_bypass(&mut self, module: ModuleId, bypass: bool) {
        // Try effect chain first
        for instrument in &mut self.instruments {
            if let Some(slot) = instrument
                .effect_chain_mut()
                .find_effect_by_type(module.module_type)
            {
                slot.state = crate::effect_chain::EnabledState::from(!bypass);
                return;
            }
        }

        // Also set bypass on voice graph modules (osc, filter, env, LFO)
        for instrument in &mut self.instruments {
            instrument.voice_graph_mut().set_bypass(module, bypass);
            for voice in instrument.allocator_mut().voices_mut() {
                voice.graph.set_bypass(module, bypass);
            }
        }
    }

    fn handle_set_effect_param(
        &mut self,
        instrument_id: Option<InstrumentId>,
        effect_type: crate::commands::EffectType,
        param: Param,
    ) {
        let mt = effect_type.to_module_type();
        match instrument_id {
            Some(inst_id) => {
                if let Some(slot) = self.find_effect_by_type(inst_id, mt) {
                    slot.effect.set_param(param);
                    slot.state = crate::effect_chain::EnabledState::Active;
                }
            }
            None => {
                if let Some(slot) = self.master_effects.find_effect_by_type(mt) {
                    slot.effect.set_param(param);
                    slot.state = crate::effect_chain::EnabledState::Active;
                }
            }
        }
    }

    fn handle_set_effect_enabled(
        &mut self,
        instrument_id: Option<InstrumentId>,
        effect_type: crate::commands::EffectType,
        enabled: bool,
    ) {
        let mt = effect_type.to_module_type();
        let state = crate::effect_chain::EnabledState::from(enabled);
        match instrument_id {
            Some(inst_id) => {
                if let Some(slot) = self.find_effect_by_type(inst_id, mt) {
                    slot.state = state;
                }
            }
            None => {
                if let Some(slot) = self.master_effects.find_effect_by_type(mt) {
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
                    let count = count_effects(instrument);
                    self.state.effect_count.store(count);
                }
            }
            None => {
                self.master_effects.remove_effect(id);
            }
        }
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
                        eprintln!(
                            "Voice graph connection failed: {:?}:{} -> {:?}:{} - {}",
                            from.module, from.port, to.module, to.port, e
                        );
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
                    eprintln!(
                        "Global graph connection failed: {:?}:{} -> {:?}:{} - {}",
                        from.module, from.port, to.module, to.port, e
                    );
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
        module_ids.sort_by_key(|id| format!("{id:?}"));

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
                crate::shared_state::InstrumentSnapshot {
                    id: inst.id(),
                    seq_instrument_id: seq_id,
                    name: inst.name().to_string(),
                    midi_channel: inst.midi_channel().as_one_indexed(),
                    volume: inst.volume().as_f32(),
                    pan: inst.pan().as_f32(),
                    enabled: inst.is_enabled(),
                    muted: !inst.is_enabled(),
                    solo: inst.is_solo(),
                    module_count: inst.voice_graph().len(),
                    effect_count: inst.effect_chain().slots().len(),
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
    fn process_voices(&mut self, context: &ProcessContext) {
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

        // Process each instrument - delegate to Instrument::process
        for instrument in &mut self.instruments {
            // Skip this instrument if:
            // - Any instrument is soloed AND this one is not soloed
            if any_soloed && !instrument.is_solo() {
                continue;
            }

            active_count += instrument.process(
                &mut self.mix_buffer,
                context,
                spatial_ctx.as_ref(),
                &mut self.spatial_voice_bank,
            );
        }

        // Update total voice count across all instruments
        self.state.voice_count.store(active_count);
    }

    /// Process the global module graph.
    ///
    /// This processes user-added modules and mixes their output
    /// into the main mix buffer.
    fn process_module_graph(&mut self, context: &ProcessContext) {
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
    fn process_master_effects(&mut self, context: &ProcessContext) {
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
fn route_sequencer_events(
    events: &[SequencerEvent],
    instruments: &mut [Box<Instrument>],
    mapping: &InstrumentMapping,
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
                }
            }
            SequencerEvent::NoteOff {
                pitch, instrument, ..
            } => {
                let note = MidiNote::new(pitch.as_midi());

                if let Some(idx) = resolve_instrument_index(instrument, mapping, instruments) {
                    instruments[idx].note_off(note);
                }
            }
            SequencerEvent::Parameter { target, value, .. } => {
                if let AutomationTarget::Instrument { instrument, param } = target
                    && let Some(engine_id) = mapping.engine_id(*instrument)
                    && let Some(inst) = instruments.iter_mut().find(|i| i.id() == engine_id)
                {
                    match param {
                        AutoInstrumentParam::Volume => {
                            inst.set_volume(Gain::new(*value));
                        }
                        AutoInstrumentParam::Pan => {
                            // Map 0.0-1.0 to -1.0..1.0
                            inst.set_pan(BipolarValue::new(*value * 2.0 - 1.0));
                        }
                        _ => {} // FilterCutoff etc. requires module routing (future)
                    }
                }
            }
        }
    }
}

impl AudioProcessor for SynthEngine {
    fn process(&mut self, output: &mut [f32], context: &AudioCallbackContext) {
        let start_time = Instant::now();

        // Process commands
        self.process_commands();

        let sample_count = SampleCount::new(context.frames);

        let process_context = ProcessContext {
            sample_rate: synth_core::SampleRate::new(context.sample_rate.as_f32()),
            samples: sample_count,
            tempo: Bpm::new(self.state.transport.get_tempo()),
            is_playing: self.state.transport.is_playing(),
            position_beats: BeatPosition::new(self.state.transport.position_beats.load()),
            voice_start_time: synth_core::SamplePosition::ZERO,
        };

        // Process sequencer events
        self.sequencer_event_buffer.clear();
        self.sequencer
            .process(sample_count, &mut self.sequencer_event_buffer);

        self.state
            .transport
            .set_ticks(self.sequencer.current_tick().0);

        route_sequencer_events(
            &self.sequencer_event_buffer,
            &mut self.instruments,
            &self.instrument_mapping,
        );

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
        self.state
            .transport
            .advance(context.frames as u64, self.sample_rate);

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
        self.metering.set_sample_rate(self.sample_rate);
        self.sequencer
            .set_sample_rate(synth_core::SampleRate::new(self.sample_rate));
        // AWE delay lines depend on sample rate — recalculate on next process()
        self.awe_engine.mark_geometry_dirty();
    }

    fn on_stream_stop(&mut self) {
        // Panic all instruments
        self.instruments.iter_mut().for_each(|inst| inst.panic());
    }

    fn on_error(&mut self, error: synth_core::AudioError) {
        eprintln!("Audio error: {error}");
        let _ = self.event_producer.try_push(EngineEvent::BufferUnderrun);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::voice_allocator::AllocationMode;
    use synth_core::VoiceCount;

    #[test]
    fn test_engine_creation() {
        let (engine, handle) = SynthEngine::new();
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
        let (mut engine, mut handle) = SynthEngine::with_config(config);

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
    fn test_default_instrument_exists() {
        let (engine, _handle) = SynthEngine::new();

        // Engine should have one default instrument on Channel 1
        assert_eq!(engine.instruments.len(), 1);
        assert_eq!(
            engine.instruments[0].id(),
            crate::instrument::InstrumentId::FIRST
        );
        assert_eq!(engine.instruments[0].name(), "Default");
        assert_eq!(engine.instruments[0].midi_channel(), MidiChannel::CH1);
    }

    #[test]
    fn test_part_channel_routing() {
        let (mut engine, mut handle) = SynthEngine::new();

        // Modify default instrument to listen to channel 1 only
        engine.instruments[0]
            .set_midi_channel(crate::instrument::MidiChannel::from_one_indexed(1).unwrap());

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
            let (mut engine, mut handle) = SynthEngine::with_config(config);

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
            let (mut engine, mut handle) = SynthEngine::with_config(config);

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
            let (mut engine, mut handle) = SynthEngine::with_config(config);

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
}
