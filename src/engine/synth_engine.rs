//! The main synthesizer engine with polyphony support.
//!
//! This is the complete engine that uses:
//! - VoiceAllocator for polyphonic voice management
//! - ModuleGraph for signal routing within each voice
//! - Effect chain for post-voice processing

use parking_lot::Mutex;
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::{AudioCallbackContext, AudioProcessor, StreamInfo};
use crate::engine::commands::{EngineCommand, EngineEvent, ModuleId};
use crate::engine::master_bus::{MasterBus, EffectSlot};
use crate::engine::graph::ModuleGraph;
use crate::engine::metering::MeteringSystem;
use crate::engine::params::{
    AmplifierParam, EnvelopeParam, FilterParam, LfoParam, LfoWaveform, ModuleType, OscillatorParam,
    Param, Waveform,
};
use crate::engine::sequencer_engine::SequencerEngine;
use crate::engine::state::EngineState;
use crate::engine::instrument::{MidiChannel, InstrumentId, Instrument};
use crate::engine::voice_allocator::{AllocatorConfig, VoiceAllocator};
use crate::types::{Gain, NormalizedValue, SampleCount, Seconds};
use crate::modules::{
    Amplifier, AudioBuffer, Envelope, Filter, Lfo, Oscillator, ProcessContext,
    PolyModule as PolyModuleTrait,
};
use crate::visualizers::{LevelMeter, Oscilloscope, VisualizationBuffer};

/// Size of the command ring buffer.
/// Large enough to handle patch loading (100+ modules with params/connections).
const COMMAND_BUFFER_SIZE: usize = 16384;

/// Size of the event ring buffer.
const EVENT_BUFFER_SIZE: usize = 256;

/// Size of the return channel for modules that need to be dropped on main thread.
const RETURN_BUFFER_SIZE: usize = 256;

/// Maximum buffer size we support.
const MAX_BUFFER_SIZE: usize = 4096;

/// Items returned from audio thread for main thread cleanup.
/// This prevents memory deallocation from happening on the audio thread.
pub enum DroppedItem {
    /// A voice module from the modular graph.
    Module(Box<dyn PolyModuleTrait>),
    /// An instrument with its voice allocator.
    Instrument(Box<Instrument>),
}

// SAFETY: PolyModule trait requires Send, and Instrument is Send
unsafe impl Send for DroppedItem {}

/// Wrapper for modules returned from audio thread for deferred cleanup.
/// This allows dropping to happen on the main thread to avoid
/// deallocations on the real-time audio thread.
pub struct DroppedModule(pub Box<dyn PolyModuleTrait>);

// SAFETY: PolyModule trait requires Send
unsafe impl Send for DroppedModule {}

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
    pub fn send_blocking(&self, command: EngineCommand) -> bool {
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 10000;
        let mut cmd = command;

        loop {
            match self.producer.lock().try_push(cmd) {
                Ok(()) => return true,
                Err(returned_cmd) => {
                    cmd = returned_cmd;
                    attempts += 1;
                    if attempts >= MAX_ATTEMPTS {
                        eprintln!("Command queue timeout after {} attempts!", MAX_ATTEMPTS);
                        return self.producer.lock().try_push(cmd).is_ok();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
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
    pub fn note_on(&mut self, note: u8, velocity: NormalizedValue) -> bool {
        self.send(EngineCommand::NoteOn {
            note,
            velocity,
            channel: super::instrument::MidiChannel::CH1,
        })
    }

    /// Send a note off event to the default channel.
    pub fn note_off(&mut self, note: u8) -> bool {
        self.send(EngineCommand::NoteOff {
            note,
            channel: super::instrument::MidiChannel::CH1,
        })
    }

    /// Send a note on event to a specific channel.
    pub fn note_on_channel(&mut self, note: u8, velocity: NormalizedValue, channel: super::instrument::MidiChannel) -> bool {
        self.send(EngineCommand::NoteOn {
            note,
            velocity,
            channel,
        })
    }

    /// Send a note off event to a specific channel.
    pub fn note_off_channel(&mut self, note: u8, channel: super::instrument::MidiChannel) -> bool {
        self.send(EngineCommand::NoteOff { note, channel })
    }

    /// Set a voice module parameter using the type-safe API.
    ///
    /// This targets a specific instrument's voice graph.
    pub fn set_voice_parameter(
        &mut self,
        instrument_id: crate::engine::instrument::InstrumentId,
        target: crate::engine::commands::PolyModule,
        param: crate::engine::typed_params::Param,
    ) -> bool {
        self.send(EngineCommand::SetVoiceParameter {
            instrument_id,
            target,
            param,
        })
    }

    /// Set an effect parameter using typed API.
    pub fn set_effect_parameter(
        &mut self,
        effect_type: crate::engine::commands::EffectType,
        param: crate::engine::typed_params::Param,
    ) -> bool {
        self.send(EngineCommand::SetEffectParameter {
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

    /// Add a visualization buffer for a module (Arc is shared with engine).
    pub fn add_visualization_buffer(&mut self, module_id: ModuleId, buffer: Arc<VisualizationBuffer>) {
        self.visualization_buffers.insert(module_id, buffer);
    }

    /// Get a visualization buffer for a module.
    pub fn get_visualization_buffer(&self, module_id: ModuleId) -> Option<&VisualizationBuffer> {
        self.visualization_buffers.get(&module_id).map(|arc| arc.as_ref())
    }

    /// Remove a visualization buffer.
    pub fn remove_visualization_buffer(&mut self, module_id: ModuleId) {
        self.visualization_buffers.remove(&module_id);
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
    /// Counter for generating unique instrument IDs.
    next_instrument_id: u64,

    // === Global module graph ===
    /// The global module graph for modular routing.
    /// Contains all user-added modules and their connections.
    module_graph: ModuleGraph,
    /// Whether to use the global module graph for processing.
    use_modular_routing: bool,

    // === Effect chain ===
    master_bus: MasterBus,

    // === Audio state ===
    sample_rate: f32,
    master_volume: f32,

    // === Buffers ===
    /// Temporary buffer for voice processing (used by module graph).
    voice_buffer: AudioBuffer,
    /// Main stereo mix buffer (interleaved L/R).
    mix_buffer: AudioBuffer,
    /// Output buffer for the global module graph.
    graph_output: AudioBuffer,

    // === Metering ===
    metering: MeteringSystem,

    // === Sequencer ===
    sequencer: SequencerEngine,

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

        // Create default instrument with OMNI channel (responds to all MIDI channels)
        let mut default_instrument = Instrument::with_config(InstrumentId::FIRST, "Default", config);
        default_instrument.set_midi_channel(MidiChannel::OMNI);

        // Populate the instrument's voice graph with default signal chain
        Self::populate_default_voice_graph(default_instrument.voice_graph_mut());

        // Initialize allocator with the instrument's voice graph
        *default_instrument.allocator_mut() = VoiceAllocator::with_graph_template(
            default_instrument.allocator().config().clone(),
            default_instrument.voice_graph(),
        );

        let engine = Self {
            command_consumer,
            event_producer,
            return_producer,
            instrument_return_producer,
            state: Arc::clone(&state),
            instruments: vec![Box::new(default_instrument)],
            next_instrument_id: 1, // 0 is used by default instrument
            module_graph: ModuleGraph::new(),
            use_modular_routing: false,
            master_bus: MasterBus::new(),
            sample_rate: 48000.0,
            master_volume: 1.0,
            voice_buffer: AudioBuffer::new(256),
            mix_buffer: AudioBuffer::new(512),
            graph_output: AudioBuffer::new(256),
            metering: MeteringSystem::new(48000.0),
            sequencer: SequencerEngine::new(crate::types::SampleRate::DVD_QUALITY),
            callback_duration_sum: 0.0,
            callback_count: 0,
        };

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

    /// Find an effect slot by its module type.
    fn find_effect_by_type(&mut self, module_type: ModuleType) -> Option<&mut EffectSlot> {
        self.master_bus.find_effect_by_type(module_type)
    }

    /// Check if a module type belongs to the voice signal chain (polyphonic).
    ///
    /// Voice modules are duplicated per voice and process within the voice allocator.
    /// Global modules (effects, visualizers) exist once and process after voice mixing.
    fn is_voice_module(module_type: ModuleType) -> bool {
        matches!(
            module_type,
            ModuleType::Oscillator
                | ModuleType::MathOscillator
                | ModuleType::SubOscillator
                | ModuleType::Noise
                | ModuleType::Filter
                | ModuleType::Envelope
                | ModuleType::Lfo
                | ModuleType::Amplifier
                | ModuleType::Mixer
                | ModuleType::StereoOutput
        )
    }

    /// Rebuild all voices for all instruments.
    ///
    /// Each instrument uses its own voice_graph as the template.
    /// Call this after bulk operations that affect all instruments.
    fn rebuild_all_instrument_voices(&mut self) {
        for instrument in &mut self.instruments {
            instrument.rebuild_voices();
        }
    }

    /// Find an effect slot by its module ID.
    fn find_effect_by_id(&mut self, module_id: ModuleId) -> Option<&mut EffectSlot> {
        self.master_bus.find_effect_by_id(module_id)
    }

    /// Populate a voice graph with the default signal chain.
    ///
    /// This creates a basic subtractive synthesis signal chain:
    /// OSC1 + OSC2 -> Filter -> Amplifier
    /// with envelope modulation.
    fn populate_default_voice_graph(graph: &mut ModuleGraph) {
        use crate::types::{Cents, Hertz, Seconds as TypedSeconds};

        // Add oscillators with Spacey Bass preset defaults
        let osc1_id = graph.add_module(Box::new({
            let mut osc = Oscillator::new();
            osc.set_param(Param::Oscillator(OscillatorParam::Waveform(Waveform::Sawtooth)));
            osc.set_param(Param::Oscillator(OscillatorParam::Level(Gain::new(0.6))));
            osc
        }));

        let osc2_id = graph.add_module(Box::new({
            let mut osc = Oscillator::new();
            osc.set_param(Param::Oscillator(OscillatorParam::Waveform(Waveform::Sawtooth)));
            osc.set_param(Param::Oscillator(OscillatorParam::Level(Gain::new(0.5))));
            osc.set_param(Param::Oscillator(OscillatorParam::Detune(Cents::new(7.0))));
            osc
        }));

        // Add filter
        let filter_id = graph.add_module(Box::new({
            let mut filter = Filter::new();
            filter.set_param(Param::Filter(FilterParam::Cutoff(Hertz::new(400.0))));
            filter.set_param(Param::Filter(FilterParam::Resonance(NormalizedValue::new(0.4))));
            filter
        }));

        // Add amp envelope
        let amp_env_id = graph.add_module(Box::new({
            let mut env = Envelope::new();
            env.set_param(Param::Envelope(EnvelopeParam::Attack(TypedSeconds::new(0.005))));
            env.set_param(Param::Envelope(EnvelopeParam::Decay(TypedSeconds::new(0.2))));
            env.set_param(Param::Envelope(EnvelopeParam::Sustain(NormalizedValue::new(0.6))));
            env.set_param(Param::Envelope(EnvelopeParam::Release(TypedSeconds::new(0.3))));
            env
        }));

        // Add filter envelope
        let filter_env_id = graph.add_module(Box::new({
            let mut env = Envelope::new();
            env.set_param(Param::Envelope(EnvelopeParam::Attack(TypedSeconds::new(0.001))));
            env.set_param(Param::Envelope(EnvelopeParam::Decay(TypedSeconds::new(0.3))));
            env.set_param(Param::Envelope(EnvelopeParam::Sustain(NormalizedValue::new(0.2))));
            env.set_param(Param::Envelope(EnvelopeParam::Release(TypedSeconds::new(0.4))));
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

    /// Handle a single command.
    fn handle_command(&mut self, command: EngineCommand) {
        match command {
            // === Instrument management commands ===
            EngineCommand::AddInstrument { instrument } => {
                self.instruments.push(instrument);
            }

            EngineCommand::RemoveInstrument { instrument_id } => {
                // Find and remove the instrument, sending it back to main thread for dropping
                // This is real-time safe: no deallocation happens on the audio thread
                if let Some(idx) = self.instruments.iter().position(|p| p.id() == instrument_id) {
                    let instrument = self.instruments.swap_remove(idx);
                    // Send to main thread for dropping - ignore if queue is full
                    let _ = self.instrument_return_producer.try_push(instrument);
                }
            }

            EngineCommand::SetInstrumentParameter { instrument_id, param } => {
                use crate::engine::commands::InstrumentParam;
                if let Some(instrument) = self.instruments.iter_mut().find(|p| p.id() == instrument_id) {
                    match param {
                        InstrumentParam::Volume(vol) => instrument.set_volume(vol),
                        InstrumentParam::Pan(pan) => instrument.set_pan(pan),
                        InstrumentParam::GlideTime(time) => instrument.allocator_mut().set_glide_time(time),
                        InstrumentParam::AllocationMode(mode) => instrument.allocator_mut().set_mode(mode),
                        InstrumentParam::StealingStrategy(strategy) => instrument.allocator_mut().set_stealing(strategy),
                        InstrumentParam::MaxVoices(_) => {
                            // Cannot change max voices at runtime without reallocating
                            // This would require recreating the allocator
                        }
                        InstrumentParam::VelocityAmpSensitivity(sens) => {
                            instrument.set_velocity_amp_sensitivity(sens);
                        }
                        InstrumentParam::VelocityFilterSensitivity(sens) => {
                            instrument.set_velocity_filter_sensitivity(sens);
                        }
                    }
                }
            }

            EngineCommand::SetInstrumentMidiChannel { instrument_id, channel } => {
                if let Some(instrument) = self.instruments.iter_mut().find(|p| p.id() == instrument_id) {
                    instrument.set_midi_channel(channel);
                }
            }

            EngineCommand::SetInstrumentEnabled { instrument_id, enabled } => {
                if let Some(instrument) = self.instruments.iter_mut().find(|p| p.id() == instrument_id) {
                    instrument.set_enabled(enabled);
                }
            }

            // === Note control - route to instruments by channel ===
            EngineCommand::NoteOn { note, velocity, channel } => {
                // Route to all instruments that respond to this channel
                let channel_raw = channel.as_zero_indexed();
                let velocity_f32 = velocity.as_f32();
                let mut note_triggered = false;

                for instrument in &mut self.instruments {
                    if instrument.responds_to_channel(channel_raw) {
                        if instrument.note_on(note, velocity_f32).is_some() {
                            note_triggered = true;
                        }
                    }
                }
                // Also trigger note on in the modular graph
                if self.use_modular_routing {
                    self.module_graph.note_on(note, velocity_f32);
                    note_triggered = true;
                }

                // Emit NoteTriggered event for GUI feedback
                if note_triggered {
                    let _ = self.event_producer.try_push(EngineEvent::NoteTriggered {
                        note,
                        velocity: velocity_f32,
                        channel,
                    });
                }
            }

            EngineCommand::NoteOff { note, channel } => {
                // Route to all instruments that respond to this channel
                let channel_raw = channel.as_zero_indexed();
                for instrument in &mut self.instruments {
                    if instrument.responds_to_channel(channel_raw) {
                        instrument.note_off(note);
                    }
                }
                // Also trigger note off in the modular graph
                if self.use_modular_routing {
                    self.module_graph.note_off();
                }

                // Emit NoteReleased event for GUI feedback
                let _ = self.event_producer.try_push(EngineEvent::NoteReleased {
                    note,
                    channel,
                });
            }

            EngineCommand::AllNotesOff => {
                // Release notes on all instruments
                for instrument in &mut self.instruments {
                    instrument.all_notes_off();
                }
                if self.use_modular_routing {
                    self.module_graph.reset();
                }

                // Emit AllNotesReleased event for GUI feedback
                let _ = self.event_producer.try_push(EngineEvent::AllNotesReleased);
            }

            EngineCommand::PitchBend { value, channel } => {
                // Apply pitch bend to all voices in instruments that respond to this channel
                let channel_raw = channel.as_zero_indexed();
                for instrument in &mut self.instruments {
                    if instrument.responds_to_channel(channel_raw) {
                        for voice in instrument.allocator_mut().voices_mut() {
                            voice.pitch_bend = value;
                        }
                    }
                }
            }

            EngineCommand::ModWheel { value, channel } => {
                // Apply mod wheel to all voices in instruments that respond to this channel
                let channel_raw = channel.as_zero_indexed();
                for instrument in &mut self.instruments {
                    if instrument.responds_to_channel(channel_raw) {
                        for voice in instrument.allocator_mut().voices_mut() {
                            voice.mod_wheel = value;
                        }
                    }
                }
            }

            EngineCommand::Aftertouch { value, channel } => {
                // Apply channel aftertouch to all voices in instruments that respond to this channel
                let channel_raw = channel.as_zero_indexed();
                for instrument in &mut self.instruments {
                    if instrument.responds_to_channel(channel_raw) {
                        for voice in instrument.allocator_mut().voices_mut() {
                            voice.aftertouch = value;
                        }
                    }
                }
            }

            EngineCommand::PolyAftertouch { note, value, channel } => {
                // Apply poly aftertouch to specific note in instruments that respond to this channel
                let channel_raw = channel.as_zero_indexed();
                for instrument in &mut self.instruments {
                    if instrument.responds_to_channel(channel_raw) {
                        for voice in instrument.allocator_mut().voices_mut() {
                            if voice.note == note {
                                voice.aftertouch = value;
                            }
                        }
                    }
                }
            }

            EngineCommand::SetMasterVolume(vol) => {
                // Clamp gain to reasonable range
                self.master_volume = vol.as_f32().clamp(0.0, 2.0);
                self.state.master_volume.store(self.master_volume);
            }

            EngineCommand::SetGlideTime(time) => {
                // Clamp to reasonable range: 0-5 second instruments
                let time_secs = Seconds::new(time.as_f32().clamp(0.0, 5.0));
                // Apply to all instruments
                for instrument in &mut self.instruments {
                    instrument.allocator_mut().set_glide_time(time_secs);
                }
            }

            EngineCommand::SetVoiceParameter { instrument_id, target, param } => {
                // Get the ModuleId from the PolyModule enum
                let module_id = target.module_id();

                // Find the target instrument and update its voice graph and voices
                if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == instrument_id) {
                    // Update the instrument's voice graph (so new voices get the new value)
                    instrument.voice_graph_mut().set_param(module_id, param.clone());

                    // Apply to all existing voices in this instrument
                    for voice in instrument.allocator_mut().voices_mut() {
                        voice.graph.set_param(module_id, param.clone());
                    }
                }
            }

            EngineCommand::SetModuleParameter { instrument_id, module_id, param } => {
                match instrument_id {
                    Some(inst_id) => {
                        // Target a specific instrument's voice graph
                        if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                            instrument.voice_graph_mut().set_param(module_id, param.clone());

                            // Apply to all existing voices in this instrument
                            for voice in instrument.allocator_mut().voices_mut() {
                                voice.graph.set_param(module_id, param.clone());
                            }
                        }
                    }
                    None => {
                        // Global module: update the global graph (master bus)
                        self.module_graph.set_param(module_id, param);
                    }
                }
            }

            EngineCommand::Reset => {
                // Panic all instruments
                for instrument in &mut self.instruments {
                    instrument.panic();
                }
                self.master_bus.reset();
            }

            EngineCommand::ClearAllModules => {
                // Clear all modules for patch loading - panic and disable all instruments
                // to prevent "ghost sound" from hardcoded voice templates
                for instrument in &mut self.instruments {
                    instrument.panic();
                    instrument.set_enabled(false);
                    // Clear each instrument's voice graph
                    instrument.voice_graph_mut().clear();
                    // Rebuild voices with the cleared graph
                    instrument.rebuild_voices();
                }
                self.master_bus.clear();
                self.module_graph.clear();
                self.use_modular_routing = false;
            }

            EngineCommand::SetBypass { module, bypass } => {
                // Use the module_type directly from ModuleId
                if let Some(slot) = self.find_effect_by_type(module.module_type) {
                    slot.enabled = !bypass;
                }
            }
            
            EngineCommand::SetEffectParameter { effect_type, param } => {
                // Convert EffectType to ModuleType and find the effect
                let mt = effect_type.to_module_type();
                if let Some(slot) = self.find_effect_by_type(mt) {
                    // Use typed API
                    slot.effect.set_param(param);
                    slot.enabled = true;
                }
            }
            
            EngineCommand::SetEffectEnabled { effect_type, enabled } => {
                let mt = effect_type.to_module_type();
                if let Some(slot) = self.find_effect_by_type(mt) {
                    slot.enabled = enabled;
                }
            }

            EngineCommand::AddVisualizer { id, visualizer_type, buffer } => {
                use crate::engine::commands::VisualizerType;
                use crate::modules::AudioEffect;

                let visualizer: Box<dyn AudioEffect> = match visualizer_type {
                    VisualizerType::Oscilloscope => Box::new(Oscilloscope::new()),
                    VisualizerType::LevelMeter => Box::new(LevelMeter::new()),
                };

                self.master_bus.add_visualizer(id, visualizer, buffer);
            }

            EngineCommand::RemoveVisualizer { id } => {
                self.master_bus.remove_visualizer(id);
            }

            EngineCommand::AddEffectInstance { id, effect } => {
                self.master_bus.add_effect(id, effect, self.sample_rate);
            }

            EngineCommand::RemoveEffect { id } => {
                self.master_bus.remove_effect(id);
            }

            // === Modular routing commands ===

            EngineCommand::AddModuleInstance { instrument_id, id, module } => {
                match instrument_id {
                    Some(inst_id) => {
                        // Add to specific instrument's voice graph
                        if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                            instrument.voice_graph_mut().add_module_with_id(id, module);
                            // Rebuild this instrument's voices with the updated graph
                            instrument.rebuild_voices();
                        }
                    }
                    None => {
                        // Global module: add to global graph (master bus)
                        self.module_graph.add_module_with_id(id, module);
                        self.use_modular_routing = true;
                    }
                }
            }

            EngineCommand::RemoveModule { instrument_id, id } => {
                match instrument_id {
                    Some(inst_id) => {
                        // Remove from specific instrument's voice graph
                        if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                            instrument.voice_graph_mut().remove_module(id);
                            // Rebuild this instrument's voices with the updated graph
                            instrument.rebuild_voices();
                        }
                    }
                    None => {
                        // Remove from global graph and send back to main thread for dropping
                        if let Some(module) = self.module_graph.remove_module_and_return(id) {
                            let _ = self.return_producer.try_push(DroppedModule(module));
                        }
                    }
                }
            }

            EngineCommand::Connect { instrument_id, from, to } => {
                match instrument_id {
                    Some(inst_id) => {
                        // Connect in specific instrument's voice graph
                        if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                            if let Err(e) = instrument.voice_graph_mut().connect(
                                from.module,
                                &from.port,
                                to.module,
                                &to.port,
                            ) {
                                eprintln!(
                                    "Voice graph connection failed: {:?}:{} -> {:?}:{} - {}",
                                    from.module, from.port, to.module, to.port, e
                                );
                            } else {
                                // Success: rebuild this instrument's voices
                                instrument.rebuild_voices();
                            }
                        }
                    }
                    None => {
                        // Connect in global graph
                        if let Err(e) = self.module_graph.connect(
                            from.module,
                            &from.port,
                            to.module,
                            &to.port,
                        ) {
                            eprintln!(
                                "Global graph connection failed: {:?}:{} -> {:?}:{} - {}",
                                from.module, from.port, to.module, to.port, e
                            );
                        }
                    }
                }
            }

            EngineCommand::Disconnect { instrument_id, from, to } => {
                match instrument_id {
                    Some(inst_id) => {
                        // Disconnect in specific instrument's voice graph
                        if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                            if instrument.voice_graph_mut().disconnect(
                                from.module,
                                &from.port,
                                to.module,
                                &to.port,
                            ) {
                                // Rebuild this instrument's voices
                                instrument.rebuild_voices();
                            }
                        }
                    }
                    None => {
                        // Disconnect in global graph
                        self.module_graph.disconnect(
                            from.module,
                            &from.port,
                            to.module,
                            &to.port,
                        );
                    }
                }
            }

            EngineCommand::DisconnectAll { instrument_id, module } => {
                match instrument_id {
                    Some(inst_id) => {
                        // Disconnect all in specific instrument's voice graph
                        if let Some(instrument) = self.instruments.iter_mut().find(|i| i.id() == inst_id) {
                            instrument.voice_graph_mut().disconnect_all(module);
                            // Rebuild this instrument's voices
                            instrument.rebuild_voices();
                        }
                    }
                    None => {
                        // Disconnect all in global graph
                        self.module_graph.disconnect_all(module);
                    }
                }
            }

            _ => {}
        }
    }

    /// Process all active voices across all instruments and mix.
    ///
    /// Delegates to `Instrument::process` for each instrument, which handles:
    /// - Voice processing through the signal chain
    /// - Instrument volume and pan application
    /// - Mixing into the stereo output buffer
    fn process_voices(&mut self, context: &ProcessContext) {
        let num_channels = 2;
        let buffer_size = context.samples * num_channels;

        // Ensure mix buffer is sized correctly and cleared
        self.mix_buffer.resize(buffer_size);
        self.mix_buffer.clear();

        let mut active_count = 0u32;

        // Process each instrument - delegate to Instrument::process
        for instrument in &mut self.instruments {
            active_count += instrument.process(&mut self.mix_buffer, context);
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
        self.graph_output.resize(context.samples);
        self.graph_output.clear();

        // Process the module graph
        self.module_graph.process(&mut self.graph_output, context);

        // Mix graph output into stereo mix buffer (mono to stereo)
        for i in 0..context.samples {
            let sample = self.graph_output[i];
            self.mix_buffer[i * 2] += sample;
            self.mix_buffer[i * 2 + 1] += sample;
        }
    }

    /// Process the effect chain.
    fn process_effects(&mut self, context: &ProcessContext) {
        self.master_bus.process(&mut self.mix_buffer, context);
    }

    /// Update metering.
    fn update_meters(&mut self, output: &[f32]) {
        self.metering.update(output, &self.state, &mut self.event_producer);
    }
}

impl Default for SynthEngine {
    fn default() -> Self {
        Self::new().0
    }
}

impl AudioProcessor for SynthEngine {
    fn process(&mut self, output: &mut [f32], context: &AudioCallbackContext) {
        let start_time = Instant::now();

        // Process commands
        self.process_commands();

        // Process sequencer with type-safe sample count
        let sample_count = SampleCount::new(context.frames);
        let sequencer_events = self.sequencer.process(sample_count);

        // Route sequencer events to the appropriate instruments
        // InstrumentId maps to instrument index (0 = first instrument, 1 = second instrument, etc.)
        for event in sequencer_events {
            match event {
                crate::sequencer::SequencerEvent::NoteOn { pitch, velocity, instrument, .. } => {
                    let note = pitch.as_midi();
                    let vel = velocity.as_f32();
                    let instrument_index = instrument.0 as usize;

                    // Trigger note on the matching instrument, or first instrument if index out of bounds
                    if let Some(target) = self.instruments.get_mut(instrument_index) {
                        target.note_on(note, vel);
                    } else if let Some(first) = self.instruments.first_mut() {
                        // Fallback to first instrument if instrument index is out of range
                        first.note_on(note, vel);
                    }
                }
                crate::sequencer::SequencerEvent::NoteOff { pitch, instrument, .. } => {
                    let note = pitch.as_midi();
                    let instrument_index = instrument.0 as usize;

                    // Trigger note off on the matching instrument
                    if let Some(target) = self.instruments.get_mut(instrument_index) {
                        target.note_off(note);
                    } else if let Some(first) = self.instruments.first_mut() {
                        first.note_off(note);
                    }
                }
                _ => {}
            }
        }

        let process_context = ProcessContext {
            sample_rate: context.sample_rate.as_f32(),
            samples: context.frames,
            tempo: 120.0,
            is_playing: self.state.transport.is_playing(),
            position_beats: self.state.transport.position_beats.load(),
        };

        // Process voices (built-in voice template)
        self.process_voices(&process_context);

        // Process modular graph (user-added modules)
        self.process_module_graph(&process_context);

        // Process effects
        self.process_effects(&process_context);

        // Copy to output with master volume
        let channels = context.channels as usize;
        for (i, frame) in output.chunks_mut(channels).enumerate() {
            let left = self.mix_buffer.as_slice().get(i * 2).copied().unwrap_or(0.0);
            let right = self.mix_buffer.as_slice().get(i * 2 + 1).copied().unwrap_or(left);

            if channels >= 2 {
                frame[0] = (left * self.master_volume).clamp(-1.0, 1.0);
                frame[1] = (right * self.master_volume).clamp(-1.0, 1.0);
            } else if channels == 1 {
                frame[0] = ((left + right) * 0.5 * self.master_volume).clamp(-1.0, 1.0);
            }
        }

        // Update meters
        self.update_meters(output);

        // Update transport
        self.state.transport.advance(context.frames as u64, self.sample_rate);

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
        self.sequencer.set_sample_rate(crate::types::SampleRate::new(self.sample_rate));
    }

    fn on_stream_stop(&mut self) {
        // Panic all instruments
        for instrument in &mut self.instruments {
            instrument.panic();
        }
    }

    fn on_error(&mut self, error: crate::audio::AudioError) {
        eprintln!("Audio error: {error}");
        let _ = self.event_producer.try_push(EngineEvent::BufferUnderrun);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::voice_allocator::AllocationMode;

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
            max_voices: 4,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let (mut engine, mut handle) = SynthEngine::with_config(config);

        // Send multiple notes
        handle.note_on(60, NormalizedValue::new(0.8));
        handle.note_on(64, NormalizedValue::new(0.8));
        handle.note_on(67, NormalizedValue::new(0.8));

        // Process commands
        engine.process_commands();

        // Should have 3 active voices across all instruments
        let total_active: usize = engine.instruments.iter()
            .map(|p| p.active_voice_count())
            .sum();
        assert_eq!(total_active, 3);
    }

    #[test]
    fn test_default_instrument_exists() {
        let (engine, _handle) = SynthEngine::new();

        // Engine should have one default instrument
        assert_eq!(engine.instruments.len(), 1);
        assert_eq!(engine.instruments[0].id(), crate::engine::instrument::InstrumentId::FIRST);
        assert_eq!(engine.instruments[0].name(), "Default");
        assert!(engine.instruments[0].midi_channel().is_omni());
    }

    #[test]
    fn test_part_channel_routing() {
        let (mut engine, mut handle) = SynthEngine::new();

        // Modify default instrument to listen to channel 1 only
        engine.instruments[0].set_midi_channel(
            crate::engine::instrument::MidiChannel::from_one_indexed(1).unwrap()
        );

        // Send note on channel 1 - should be received
        handle.note_on_channel(60, NormalizedValue::new(0.8), crate::engine::instrument::MidiChannel::CH1);
        engine.process_commands();
        assert_eq!(engine.instruments[0].active_voice_count(), 1);

        // Send note on channel 2 - should NOT be received
        let ch2 = crate::engine::instrument::MidiChannel::from_one_indexed(2).unwrap();
        handle.note_on_channel(64, NormalizedValue::new(0.8), ch2);
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
        use crate::engine::commands::{EngineCommand, ModuleId, PortId};
        use crate::engine::instrument::InstrumentId;
        use crate::modules::{Oscillator, Filter};

        /// Test A: Polyphonic Allocation
        /// An Oscillator should be added to instrument's voice_graph, NOT to module_graph.
        #[test]
        fn test_oscillator_routed_to_voice_graph() {
            let (mut engine, mut handle) = SynthEngine::new();

            // Count existing oscillators in default instrument's voice graph (there are 2 by default)
            let initial_osc_count = engine.instruments[0].voice_graph().module_ids()
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
                engine.instruments[0].voice_graph().get_module(osc_id).is_some(),
                "Oscillator should be in instrument's voice_graph"
            );

            // Verify: Oscillator should NOT be in module_graph
            assert!(
                engine.module_graph.get_module(osc_id).is_none(),
                "Oscillator should NOT be in module_graph"
            );

            // Verify: voice_graph oscillator count increased
            let final_osc_count = engine.instruments[0].voice_graph().module_ids()
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
                max_voices: 4,
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
            for (i, voice) in engine.instruments[0].allocator().voices().iter().enumerate() {
                assert!(
                    voice.graph.get_module(filter_id).is_some(),
                    "Voice {} should have filter_id after AddModuleInstance", i
                );
            }
        }

        /// Test D: Voice module connections propagate to all voices
        #[test]
        fn test_voice_connection_propagates_to_voices() {
            let config = AllocatorConfig {
                max_voices: 2,
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
                module: Box::new(crate::modules::Amplifier::new()),
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
            let template_connections: Vec<_> = engine.instruments[0].voice_graph().connections().collect();
            let has_connection = template_connections.iter().any(|c| {
                c.from_module == new_osc_id && c.to_module == new_amp_id
            });
            assert!(has_connection, "instrument's voice_graph should have the connection");

            // Verify: All voices have the connection
            for (i, voice) in engine.instruments[0].allocator().voices().iter().enumerate() {
                let voice_connections: Vec<_> = voice.graph.connections().collect();
                let has_connection = voice_connections.iter().any(|c| {
                    c.from_module == new_osc_id && c.to_module == new_amp_id
                });
                assert!(has_connection, "Voice {} should have the connection", i);
            }
        }

        /// Test E: is_voice_module correctly classifies module types
        #[test]
        fn test_is_voice_module_classification() {
            // Voice modules (should be true)
            assert!(SynthEngine::is_voice_module(ModuleType::Oscillator));
            assert!(SynthEngine::is_voice_module(ModuleType::MathOscillator));
            assert!(SynthEngine::is_voice_module(ModuleType::SubOscillator));
            assert!(SynthEngine::is_voice_module(ModuleType::Noise));
            assert!(SynthEngine::is_voice_module(ModuleType::Filter));
            assert!(SynthEngine::is_voice_module(ModuleType::Envelope));
            assert!(SynthEngine::is_voice_module(ModuleType::Lfo));
            assert!(SynthEngine::is_voice_module(ModuleType::Amplifier));
            assert!(SynthEngine::is_voice_module(ModuleType::Mixer));
            assert!(SynthEngine::is_voice_module(ModuleType::StereoOutput));

            // Global modules (should be false)
            assert!(!SynthEngine::is_voice_module(ModuleType::Delay));
            assert!(!SynthEngine::is_voice_module(ModuleType::Reverb));
            assert!(!SynthEngine::is_voice_module(ModuleType::Chorus));
            assert!(!SynthEngine::is_voice_module(ModuleType::Distortion));
            assert!(!SynthEngine::is_voice_module(ModuleType::Phaser));
            assert!(!SynthEngine::is_voice_module(ModuleType::Flanger));
            assert!(!SynthEngine::is_voice_module(ModuleType::Compressor));
            assert!(!SynthEngine::is_voice_module(ModuleType::Eq));
            assert!(!SynthEngine::is_voice_module(ModuleType::Oscilloscope));
            assert!(!SynthEngine::is_voice_module(ModuleType::LevelMeter));
        }

        /// Test F: Remove voice module propagates to voices
        #[test]
        fn test_remove_voice_module_propagates() {
            let config = AllocatorConfig {
                max_voices: 2,
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
            assert!(engine.instruments[0].voice_graph().get_module(filter_id).is_some());
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
            assert!(engine.instruments[0].voice_graph().get_module(filter_id).is_none());

            // Verify it's gone from all voices
            for voice in engine.instruments[0].allocator().voices() {
                assert!(voice.graph.get_module(filter_id).is_none());
            }
        }
    }
}
