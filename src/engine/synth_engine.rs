//! The main synthesizer engine with polyphony support.
//!
//! This is the complete engine that uses:
//! - VoiceAllocator for polyphonic voice management
//! - ModuleGraph for signal routing within each voice
//! - Effect chain for post-voice processing

use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::{AudioCallbackContext, AudioProcessor, StreamInfo};
use crate::effects::{Delay, Reverb, Chorus};
use crate::engine::commands::{EngineCommand, EngineEvent, ModuleId};
use crate::engine::state::EngineState;
use crate::engine::voice::Voice;
use crate::engine::voice_allocator::{VoiceAllocator, AllocatorConfig};
use crate::engine::graph::ModuleGraph;
use crate::engine::typed_params::{
    ModuleType, TypedParam, TypedValue,
    OscillatorParam, FilterParam, EnvelopeParam, LfoParam, AmplifierParam,
    Waveform, LfoWaveform,
};
use crate::modules::{
    Oscillator, Filter, Envelope, Lfo, Amplifier,
    AudioBuffer, ProcessContext, EffectModule, VoiceModule as VoiceModuleTrait,
};
use crate::visualizers::{Oscilloscope, LevelMeter, VisualizationBuffer};

/// Size of the command ring buffer.
/// Large enough to handle patch loading (100+ modules with params/connections).
const COMMAND_BUFFER_SIZE: usize = 16384;

/// Size of the event ring buffer.
const EVENT_BUFFER_SIZE: usize = 256;

/// Size of the return channel for modules that need to be dropped on main thread.
const RETURN_BUFFER_SIZE: usize = 256;

/// Maximum buffer size we support.
const MAX_BUFFER_SIZE: usize = 4096;

/// Pre-allocated buffers for voice processing.
/// This avoids heap allocations in the audio thread.
struct VoiceProcessingBuffers {
    // LFO buffers
    lfo_out: AudioBuffer,
    retrigger: AudioBuffer,

    // Oscillator buffers
    osc1_out: AudioBuffer,
    osc2_out: AudioBuffer,
    osc_mixed: AudioBuffer,
    fm_buffer: AudioBuffer,
    pwm_buffer: AudioBuffer,

    // Filter buffers
    filter_out: AudioBuffer,
    filter_env_out: AudioBuffer,
    cutoff_cv: AudioBuffer,
    res_cv: AudioBuffer,

    // Amp buffers
    amp_env_out: AudioBuffer,
    amp_left: AudioBuffer,
    amp_right: AudioBuffer,
    amp_cv: AudioBuffer,
    pan_cv: AudioBuffer,
}

impl VoiceProcessingBuffers {
    fn new() -> Self {
        Self {
            lfo_out: AudioBuffer::new(MAX_BUFFER_SIZE),
            retrigger: AudioBuffer::new(MAX_BUFFER_SIZE),
            osc1_out: AudioBuffer::new(MAX_BUFFER_SIZE),
            osc2_out: AudioBuffer::new(MAX_BUFFER_SIZE),
            osc_mixed: AudioBuffer::new(MAX_BUFFER_SIZE),
            fm_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
            pwm_buffer: AudioBuffer::new(MAX_BUFFER_SIZE),
            filter_out: AudioBuffer::new(MAX_BUFFER_SIZE),
            filter_env_out: AudioBuffer::new(MAX_BUFFER_SIZE),
            cutoff_cv: AudioBuffer::new(MAX_BUFFER_SIZE),
            res_cv: AudioBuffer::new(MAX_BUFFER_SIZE),
            amp_env_out: AudioBuffer::new(MAX_BUFFER_SIZE),
            amp_left: AudioBuffer::new(MAX_BUFFER_SIZE),
            amp_right: AudioBuffer::new(MAX_BUFFER_SIZE),
            amp_cv: AudioBuffer::new(MAX_BUFFER_SIZE),
            pan_cv: AudioBuffer::new(MAX_BUFFER_SIZE),
        }
    }

    /// Clear all buffers for reuse.
    #[inline]
    fn clear_all(&mut self) {
        self.lfo_out.clear();
        self.retrigger.clear();
        self.osc1_out.clear();
        self.osc2_out.clear();
        self.osc_mixed.clear();
        self.fm_buffer.clear();
        self.pwm_buffer.clear();
        self.filter_out.clear();
        self.filter_env_out.clear();
        self.cutoff_cv.clear();
        self.res_cv.clear();
        self.amp_env_out.clear();
        self.amp_left.clear();
        self.amp_right.clear();
        self.amp_cv.clear();
        self.pan_cv.clear();
    }
}

/// Wrapper for modules returned from audio thread.
/// This allows dropping to happen on the main thread.
pub struct DroppedModule(pub Box<dyn VoiceModuleTrait>);

// SAFETY: VoiceModule trait requires Send
unsafe impl Send for DroppedModule {}

/// Handle for the UI to communicate with the engine.
pub struct EngineHandle {
    /// Send commands to the engine.
    command_producer: ringbuf::HeapProd<EngineCommand>,
    /// Receive events from the engine.
    event_consumer: ringbuf::HeapCons<EngineEvent>,
    /// Receive dropped modules from audio thread (for main thread cleanup).
    return_consumer: ringbuf::HeapCons<DroppedModule>,
    /// Shared state for reading meters, etc.
    pub state: Arc<EngineState>,
    /// Visualization buffers keyed by module ID (shared with engine via Arc).
    pub visualization_buffers: HashMap<ModuleId, Arc<VisualizationBuffer>>,
}

impl EngineHandle {
    /// Send a command to the engine (non-blocking, may fail if queue full).
    pub fn send(&mut self, command: EngineCommand) -> bool {
        self.command_producer.try_push(command).is_ok()
    }

    /// Send a command to the engine, blocking until there's space in the queue.
    /// Use this when loading patches or doing bulk operations.
    /// Returns false only if there's a timeout (deadlock protection).
    pub fn send_blocking(&mut self, command: EngineCommand) -> bool {
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 10000; // ~10 seconds at 1ms sleep
        let mut cmd = command;

        loop {
            match self.command_producer.try_push(cmd) {
                Ok(()) => return true,
                Err(returned_cmd) => {
                    cmd = returned_cmd; // ringbuf returns the value directly on error
                    attempts += 1;
                    if attempts >= MAX_ATTEMPTS {
                        eprintln!("Command queue timeout after {} attempts!", MAX_ATTEMPTS);
                        // Final attempt, drop the command if it fails
                        return self.command_producer.try_push(cmd).is_ok();
                    }
                    // Give the audio thread time to process commands
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }

    /// Poll and drop any modules returned from the audio thread.
    /// Call this regularly from the main thread to clean up removed modules.
    pub fn cleanup_dropped_modules(&mut self) {
        while self.return_consumer.try_pop().is_some() {
            // Module is dropped here on the main thread - no audio glitches!
        }
    }

    /// Send a note on event.
    pub fn note_on(&mut self, note: u8, velocity: f32) -> bool {
        self.send(EngineCommand::NoteOn {
            note,
            velocity,
            channel: 0,
        })
    }

    /// Send a note off event.
    pub fn note_off(&mut self, note: u8) -> bool {
        self.send(EngineCommand::NoteOff { note, channel: 0 })
    }

    /// Set a voice module parameter using the type-safe API.
    pub fn set_voice_parameter(
        &mut self,
        target: crate::engine::commands::VoiceModule,
        param: crate::engine::typed_params::TypedParam,
        value: crate::engine::typed_params::TypedValue,
    ) -> bool {
        self.send(EngineCommand::SetVoiceParameter {
            target,
            param,
            value,
        })
    }
    
    /// Set an effect parameter using typed API.
    pub fn set_effect_parameter(
        &mut self,
        effect_type: crate::engine::commands::EffectType,
        param: crate::engine::typed_params::TypedParam,
        value: crate::engine::typed_params::TypedValue,
    ) -> bool {
        self.send(EngineCommand::SetEffectParameter {
            effect_type,
            param,
            value,
        })
    }

    /// Set master volume.
    pub fn set_master_volume(&mut self, volume: f32) -> bool {
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

/// Effect slot in the effect chain.
struct EffectSlot {
    module_id: ModuleId,
    effect: Box<dyn EffectModule>,
    module_type: ModuleType,
    enabled: bool,
}

/// Visualizer slot with shared buffer.
struct VisualizerSlot {
    module_id: ModuleId,
    visualizer: Box<dyn EffectModule>,
    buffer: Arc<VisualizationBuffer>,
    enabled: bool,
}

/// The main synthesizer engine with polyphony.
pub struct SynthEngine {
    /// Receive commands from UI.
    command_consumer: ringbuf::HeapCons<EngineCommand>,
    /// Send events to UI.
    event_producer: ringbuf::HeapProd<EngineEvent>,
    /// Send removed modules back to UI for dropping on main thread.
    return_producer: ringbuf::HeapProd<DroppedModule>,
    /// Shared state.
    state: Arc<EngineState>,

    // === Voice management ===
    /// Voice allocator.
    voice_allocator: VoiceAllocator,
    /// Template module graph for voices.
    voice_template: ModuleGraph,

    // === Global module graph ===
    /// The global module graph for modular routing.
    /// Contains all user-added modules and their connections.
    module_graph: ModuleGraph,
    /// Whether to use the global module graph for processing.
    use_modular_routing: bool,

    // === Effect chain ===
    effects: Vec<EffectSlot>,

    // === Visualizers ===
    visualizers: Vec<VisualizerSlot>,

    // === Audio state ===
    sample_rate: f32,
    master_volume: f32,

    // === Buffers ===
    voice_buffer: AudioBuffer,
    mix_buffer: AudioBuffer,
    effect_buffer: AudioBuffer,
    graph_output: AudioBuffer,

    // === Pre-allocated voice processing buffers ===
    voice_buffers: VoiceProcessingBuffers,

    // === Metering ===
    meter_counter: usize,
    meter_interval: usize,
    peak_left: f32,
    peak_right: f32,
    rms_sum_left: f32,
    rms_sum_right: f32,

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

        // Create voice template with default signal chain
        let voice_template = Self::create_default_voice_template();

        // Create voice allocator with template
        let voice_allocator = VoiceAllocator::with_template(config, &Self::create_template_voice(&voice_template));

        // Create default effect chain
        // Each effect has a ModuleType for type-safe lookup
        let effects = vec![
            EffectSlot {
                module_id: ModuleId::new(ModuleType::Chorus, 1),
                effect: Box::new(Chorus::new()),
                module_type: ModuleType::Chorus,
                enabled: false,
            },
            EffectSlot {
                module_id: ModuleId::new(ModuleType::Delay, 1),
                effect: Box::new(Delay::new()),
                module_type: ModuleType::Delay,
                enabled: false,
            },
            EffectSlot {
                module_id: ModuleId::new(ModuleType::Reverb, 1),
                effect: Box::new(Reverb::new()),
                module_type: ModuleType::Reverb,
                enabled: false,
            },
            EffectSlot {
                module_id: ModuleId::new(ModuleType::Distortion, 1),
                effect: Box::new(crate::effects::Distortion::new()),
                module_type: ModuleType::Distortion,
                enabled: false,
            },
        ];

        let engine = Self {
            command_consumer,
            event_producer,
            return_producer,
            state: Arc::clone(&state),
            voice_allocator,
            voice_template,
            module_graph: ModuleGraph::new(),
            use_modular_routing: false,
            effects,
            visualizers: Vec::new(),
            sample_rate: 48000.0,
            master_volume: 1.0,
            voice_buffer: AudioBuffer::new(256),
            mix_buffer: AudioBuffer::new(512),
            effect_buffer: AudioBuffer::new(512),
            graph_output: AudioBuffer::new(256),
            voice_buffers: VoiceProcessingBuffers::new(),
            meter_counter: 0,
            meter_interval: 2048,
            peak_left: 0.0,
            peak_right: 0.0,
            rms_sum_left: 0.0,
            rms_sum_right: 0.0,
            callback_duration_sum: 0.0,
            callback_count: 0,
        };

        let handle = EngineHandle {
            command_producer,
            event_consumer,
            return_consumer,
            state,
            visualization_buffers: HashMap::new(),
        };

        (engine, handle)
    }

    /// Find an effect slot by its module type.
    fn find_effect_by_type(&mut self, module_type: ModuleType) -> Option<&mut EffectSlot> {
        self.effects.iter_mut().find(|slot| slot.module_type == module_type)
    }

    /// Find an effect slot by its module ID.
    fn find_effect_by_id(&mut self, module_id: ModuleId) -> Option<&mut EffectSlot> {
        self.effects.iter_mut().find(|slot| slot.module_id == module_id)
    }

    /// Create the default voice template graph.
    fn create_default_voice_template() -> ModuleGraph {
        let mut graph = ModuleGraph::new();

        // Add modules
        let osc1_id = graph.add_module(Box::new(Oscillator::new()));
        let _osc2_id = graph.add_module(Box::new({
            let mut osc = Oscillator::new();
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Detune),
                TypedValue::Float(7.0)
            );
            osc
        }));
        
        let filter_id = graph.add_module(Box::new(Filter::new()));
        let env_id = graph.add_module(Box::new(Envelope::new()));
        let filter_env_id = graph.add_module(Box::new({
            let mut env = Envelope::new();
            env.set_param(TypedParam::Envelope(EnvelopeParam::Attack), TypedValue::Float(0.001));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Decay), TypedValue::Float(0.3));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Sustain), TypedValue::Float(0.3));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Release), TypedValue::Float(0.5));
            env
        }));
        let amp_id = graph.add_module(Box::new(Amplifier::new()));
        let _lfo_id = graph.add_module(Box::new({
            let mut lfo = Lfo::new();
            lfo.set_param(TypedParam::Lfo(LfoParam::Rate), TypedValue::Float(0.5));
            lfo.set_param(TypedParam::Lfo(LfoParam::Depth), TypedValue::Float(0.3));
            lfo
        }));

        // Connect modules
        // OSC1 -> Filter
        let _ = graph.connect(osc1_id, "out", filter_id, "in");
        // OSC2 -> Filter (mixed)
        // Filter -> Amp
        let _ = graph.connect(filter_id, "out", amp_id, "in");
        // Amp Envelope -> Amp CV
        let _ = graph.connect(env_id, "out", amp_id, "cv");
        // Filter Envelope -> Filter Cutoff CV
        let _ = graph.connect(filter_env_id, "out", filter_id, "cutoff_cv");
        // LFO -> Filter Cutoff (subtle modulation)

        graph
    }

    /// Create a template voice from the graph.
    /// Default settings: Spacey bass sound
    fn create_template_voice(_graph: &ModuleGraph) -> Voice {
        let mut voice = Voice::new(0);
        
        // Oscillator 1 - Sawtooth, main bass sound
        voice.add_module("osc1", Box::new({
            let mut osc = Oscillator::new();
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Waveform),
                TypedValue::Waveform(Waveform::Sawtooth)
            );
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Level),
                TypedValue::Float(0.6)
            );
            osc
        }));
        
        // Oscillator 2 - Slightly detuned for thickness
        voice.add_module("osc2", Box::new({
            let mut osc = Oscillator::new();
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Waveform),
                TypedValue::Waveform(Waveform::Sawtooth)
            );
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Level),
                TypedValue::Float(0.5)
            );
            // Detune +7 cents for classic detuned sound
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Detune),
                TypedValue::Float(7.0)
            );
            osc
        }));
        
        // Filter - Low pass with moderate resonance
        voice.add_module("filter", Box::new({
            let mut filter = Filter::new();
            // Cutoff around 400 Hz for bass
            filter.set_param(
                TypedParam::Filter(FilterParam::Cutoff),
                TypedValue::Float(400.0)
            );
            // Some resonance for character
            filter.set_param(
                TypedParam::Filter(FilterParam::Resonance),
                TypedValue::Float(0.4)
            );
            filter
        }));
        
        // Amp Envelope - Punchy but smooth
        voice.add_module("amp_env", Box::new({
            let mut env = Envelope::new();
            env.set_param(TypedParam::Envelope(EnvelopeParam::Attack), TypedValue::Float(0.005));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Decay), TypedValue::Float(0.2));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Sustain), TypedValue::Float(0.6));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Release), TypedValue::Float(0.3));
            env
        }));
        
        // Filter Envelope - Opens up on attack
        voice.add_module("filter_env", Box::new({
            let mut env = Envelope::new();
            env.set_param(TypedParam::Envelope(EnvelopeParam::Attack), TypedValue::Float(0.001));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Decay), TypedValue::Float(0.3));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Sustain), TypedValue::Float(0.2));
            env.set_param(TypedParam::Envelope(EnvelopeParam::Release), TypedValue::Float(0.4));
            env
        }));
        
        // LFO - Slow modulation for movement
        voice.add_module("lfo", Box::new({
            let mut lfo = Lfo::new();
            lfo.set_param(TypedParam::Lfo(LfoParam::Rate), TypedValue::Float(0.3));
            lfo.set_param(TypedParam::Lfo(LfoParam::Depth), TypedValue::Float(0.25));
            lfo.set_param(
                TypedParam::Lfo(LfoParam::Waveform),
                TypedValue::LfoWaveform(LfoWaveform::Sine)
            );
            lfo
        }));
        
        // Amplifier
        voice.add_module("amp", Box::new({
            let mut amp = Amplifier::new();
            amp.set_param(
                TypedParam::Amplifier(AmplifierParam::Level),
                TypedValue::Float(0.7)
            );
            amp
        }));

        voice
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
            EngineCommand::NoteOn { note, velocity, channel: _ } => {
                self.voice_allocator.note_on(note, velocity);
                // Also trigger note on in the modular graph
                if self.use_modular_routing {
                    self.module_graph.note_on(note, velocity);
                }
            }

            EngineCommand::NoteOff { note, channel: _ } => {
                self.voice_allocator.note_off(note);
                // Also trigger note off in the modular graph
                if self.use_modular_routing {
                    self.module_graph.note_off();
                }
            }

            EngineCommand::AllNotesOff => {
                self.voice_allocator.all_notes_off();
                if self.use_modular_routing {
                    self.module_graph.reset();
                }
            }

            EngineCommand::SetMasterVolume(vol) => {
                self.master_volume = vol.clamp(0.0, 2.0);
                self.state.master_volume.store(self.master_volume);
            }

            EngineCommand::SetGlideTime(time) => {
                // Clamp to reasonable range: 0-5 seconds
                let time = time.clamp(0.0, 5.0);
                self.voice_allocator.set_glide_time(time);
            }

            EngineCommand::SetVoiceParameter { target, param, value } => {
                // Use VoiceModule enum to get the module name - no magic numbers!
                let module_name = target.internal_name();
                
                // Apply to all voices using the typed API
                for voice in self.voice_allocator.voices_mut() {
                    if let Some(module) = voice.get_module_mut(module_name) {
                        module.set_param(param.clone(), value.clone());
                    }
                }
            }
            
            EngineCommand::SetModuleParameter { module_id, param, value } => {
                // Set parameter on module in the global graph
                if let Some(module) = self.module_graph.get_module_mut(module_id) {
                    module.set_param(param, value);
                }
            }

            EngineCommand::Reset => {
                self.voice_allocator.panic();
                for effect in &mut self.effects {
                    effect.effect.reset();
                }
            }

            EngineCommand::ClearAllModules => {
                // Clear all modules for patch loading
                self.voice_allocator.panic();
                self.effects.clear();
                self.visualizers.clear();
                self.module_graph.clear();
                self.use_modular_routing = false;
            }

            EngineCommand::SetBypass { module, bypass } => {
                // Use the module_type directly from ModuleId
                if let Some(slot) = self.find_effect_by_type(module.module_type) {
                    slot.enabled = !bypass;
                }
            }
            
            EngineCommand::SetEffectParameter { effect_type, param, value } => {
                // Convert EffectType to ModuleType and find the effect
                let mt = effect_type.to_module_type();
                if let Some(slot) = self.find_effect_by_type(mt) {
                    // Use typed API
                    slot.effect.set_param(param, value);
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
                
                let visualizer: Box<dyn EffectModule> = match visualizer_type {
                    VisualizerType::Oscilloscope => Box::new(Oscilloscope::new()),
                    VisualizerType::LevelMeter => Box::new(LevelMeter::new()),
                };
                
                self.visualizers.push(VisualizerSlot {
                    module_id: id,
                    visualizer,
                    buffer,
                    enabled: true,
                });
            }

            EngineCommand::RemoveVisualizer { id } => {
                self.visualizers.retain(|slot| slot.module_id != id);
            }

            EngineCommand::AddEffectInstance { id, mut effect } => {
                // Set sample rate on the pre-created effect
                effect.set_sample_rate(self.sample_rate);
                
                // Get module type directly from effect (already typed_params::ModuleType)
                let module_type = effect.module_type();
                
                self.effects.push(EffectSlot {
                    module_id: id,
                    module_type,
                    effect,
                    enabled: true,
                });
            }

            EngineCommand::RemoveEffect { id } => {
                self.effects.retain(|slot| slot.module_id != id);
            }

            // === Modular routing commands ===

            EngineCommand::AddModuleInstance { id, module } => {
                self.module_graph.add_module_with_id(id, module);
                self.use_modular_routing = true;
            }

            EngineCommand::RemoveModule { id } => {
                // Remove module and send it back to main thread for dropping
                // This avoids deallocation on the audio thread
                if let Some(module) = self.module_graph.remove_module_and_return(id) {
                    let _ = self.return_producer.try_push(DroppedModule(module));
                }
            }

            EngineCommand::Connect { from, to } => {
                if let Err(e) = self.module_graph.connect(
                    from.module,
                    &from.port,
                    to.module,
                    &to.port,
                ) {
                    eprintln!(
                        "Connection failed: {:?}:{} -> {:?}:{} - {}",
                        from.module, from.port, to.module, to.port, e
                    );
                }
            }

            EngineCommand::Disconnect { from, to } => {
                self.module_graph.disconnect(
                    from.module,
                    &from.port,
                    to.module,
                    &to.port,
                );
            }

            EngineCommand::DisconnectAll { module } => {
                self.module_graph.disconnect_all(module);
            }

            _ => {}
        }
    }

    /// Process all active voices and mix.
    fn process_voices(&mut self, context: &ProcessContext) {
        let num_channels = 2;
        let buffer_size = context.samples * num_channels;
        let samples = context.samples;

        // Ensure buffers are sized correctly
        self.mix_buffer.resize(buffer_size);
        self.mix_buffer.clear();
        self.voice_buffer.resize(samples);

        let mut active_count = 0u32;

        // Process each voice
        for voice in self.voice_allocator.voices_mut() {
            if !voice.is_active() {
                continue;
            }

            active_count += 1;
            self.voice_buffer.clear();

            // Clear pre-allocated buffers for this voice
            self.voice_buffers.clear_all();

            // Note frequency (one octave down for bass)
            let freq = 440.0 * 2.0f32.powf((voice.note as f32 - 69.0 - 12.0) / 12.0);
            let velocity = voice.velocity;
            let voice_age = voice.age;

            // Set oscillator frequencies using typed API
            if let Some(osc) = voice.get_module_mut("osc1") {
                osc.set_param(
                    TypedParam::Oscillator(OscillatorParam::Frequency),
                    TypedValue::Float(freq)
                );
            }
            if let Some(osc) = voice.get_module_mut("osc2") {
                osc.set_param(
                    TypedParam::Oscillator(OscillatorParam::Frequency),
                    TypedValue::Float(freq)
                );
            }

            // === Process LFO with retrigger on note start ===
            if voice_age == 0 {
                self.voice_buffers.retrigger[0] = 1.0;
            }

            // Use pre-allocated buffers with HashMap (small allocation for HashMap itself,
            // but AudioBuffer data is pre-allocated)
            let lfo_inputs: HashMap<String, &AudioBuffer> =
                [("retrigger".to_string(), &self.voice_buffers.retrigger as &AudioBuffer)]
                .into_iter().collect();

            let mut lfo_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), std::mem::take(&mut self.voice_buffers.lfo_out))]
                .into_iter().collect();

            if let Some(lfo) = voice.get_module_mut("lfo") {
                lfo.process(&lfo_inputs, &mut lfo_outputs, context);
            }

            // Take buffer back
            if let Some(buf) = lfo_outputs.remove("out") {
                self.voice_buffers.lfo_out = buf;
            }

            // === Process Oscillator 1 with FM and PWM ===
            for i in 0..samples {
                self.voice_buffers.fm_buffer[i] = self.voice_buffers.lfo_out[i] * 0.025;
                self.voice_buffers.pwm_buffer[i] = self.voice_buffers.lfo_out[i] * 0.3;
            }

            let osc1_inputs: HashMap<String, &AudioBuffer> = [
                ("fm".to_string(), &self.voice_buffers.fm_buffer as &AudioBuffer),
                ("pwm".to_string(), &self.voice_buffers.pwm_buffer as &AudioBuffer),
            ].into_iter().collect();

            let mut osc1_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), std::mem::take(&mut self.voice_buffers.osc1_out))]
                .into_iter().collect();

            if let Some(osc) = voice.get_module_mut("osc1") {
                osc.process(&osc1_inputs, &mut osc1_outputs, context);
            }

            if let Some(buf) = osc1_outputs.remove("out") {
                self.voice_buffers.osc1_out = buf;
            }

            // === Process Oscillator 2 ===
            let osc2_inputs: HashMap<String, &AudioBuffer> = [
                ("fm".to_string(), &self.voice_buffers.fm_buffer as &AudioBuffer),
                ("pwm".to_string(), &self.voice_buffers.pwm_buffer as &AudioBuffer),
            ].into_iter().collect();

            let mut osc2_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), std::mem::take(&mut self.voice_buffers.osc2_out))]
                .into_iter().collect();

            if let Some(osc) = voice.get_module_mut("osc2") {
                osc.process(&osc2_inputs, &mut osc2_outputs, context);
            }

            if let Some(buf) = osc2_outputs.remove("out") {
                self.voice_buffers.osc2_out = buf;
            }

            // === Mix oscillators ===
            for i in 0..samples {
                self.voice_buffers.osc_mixed[i] = self.voice_buffers.osc1_out[i] + self.voice_buffers.osc2_out[i];
            }

            // === Process Filter Envelope ===
            let empty_inputs: HashMap<String, &AudioBuffer> = HashMap::new();
            let mut filter_env_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), std::mem::take(&mut self.voice_buffers.filter_env_out))]
                .into_iter().collect();

            if let Some(env) = voice.get_module_mut("filter_env") {
                env.process(&empty_inputs, &mut filter_env_outputs, context);
            }

            if let Some(buf) = filter_env_outputs.remove("out") {
                self.voice_buffers.filter_env_out = buf;
            }

            // === Process Filter ===
            for i in 0..samples {
                self.voice_buffers.cutoff_cv[i] = self.voice_buffers.lfo_out[i] * 0.2 + self.voice_buffers.filter_env_out[i] * 0.6;
                self.voice_buffers.res_cv[i] = self.voice_buffers.lfo_out[i] * 0.1;
            }

            let filter_inputs: HashMap<String, &AudioBuffer> = [
                ("in".to_string(), &self.voice_buffers.osc_mixed as &AudioBuffer),
                ("cutoff_cv".to_string(), &self.voice_buffers.cutoff_cv as &AudioBuffer),
                ("res_cv".to_string(), &self.voice_buffers.res_cv as &AudioBuffer),
            ].into_iter().collect();

            let mut filter_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), std::mem::take(&mut self.voice_buffers.filter_out))]
                .into_iter().collect();

            if let Some(filter) = voice.get_module_mut("filter") {
                filter.process(&filter_inputs, &mut filter_outputs, context);
            }

            if let Some(buf) = filter_outputs.remove("out") {
                self.voice_buffers.filter_out = buf;
            }

            // === Process Amp Envelope ===
            let mut amp_env_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), std::mem::take(&mut self.voice_buffers.amp_env_out))]
                .into_iter().collect();

            if let Some(env) = voice.get_module_mut("amp_env") {
                env.process(&empty_inputs, &mut amp_env_outputs, context);
            }

            if let Some(buf) = amp_env_outputs.remove("out") {
                self.voice_buffers.amp_env_out = buf;
            }

            // === Process Amplifier ===
            for i in 0..samples {
                self.voice_buffers.amp_cv[i] = self.voice_buffers.amp_env_out[i] * velocity;
                self.voice_buffers.pan_cv[i] = self.voice_buffers.lfo_out[i] * 0.15;
            }

            let amp_inputs: HashMap<String, &AudioBuffer> = [
                ("in".to_string(), &self.voice_buffers.filter_out as &AudioBuffer),
                ("cv".to_string(), &self.voice_buffers.amp_cv as &AudioBuffer),
                ("pan_cv".to_string(), &self.voice_buffers.pan_cv as &AudioBuffer),
            ].into_iter().collect();

            let mut amp_outputs: HashMap<String, AudioBuffer> = [
                ("left".to_string(), std::mem::take(&mut self.voice_buffers.amp_left)),
                ("right".to_string(), std::mem::take(&mut self.voice_buffers.amp_right)),
            ].into_iter().collect();

            if let Some(amp) = voice.get_module_mut("amp") {
                amp.process(&amp_inputs, &mut amp_outputs, context);
            }

            if let Some(buf) = amp_outputs.remove("left") {
                self.voice_buffers.amp_left = buf;
            }
            if let Some(buf) = amp_outputs.remove("right") {
                self.voice_buffers.amp_right = buf;
            }

            // Mix stereo output into mix buffer
            for i in 0..samples {
                self.mix_buffer[i * 2] += self.voice_buffers.amp_left[i];
                self.mix_buffer[i * 2 + 1] += self.voice_buffers.amp_right[i];
            }
        }

        // Update voice count
        self.state.voice_count.store(active_count);

        // Advance allocator time
        self.voice_allocator.advance_time(context.samples as u64);
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
        self.effect_buffer.resize(context.samples * 2);

        for slot in &mut self.effects {
            if slot.enabled {
                // Copy mix to effect buffer
                self.effect_buffer.copy_from(&self.mix_buffer);
                
                // Process effect (in-place)
                slot.effect.process(
                    self.effect_buffer.as_slice(),
                    self.mix_buffer.as_mut_slice(),
                    context,
                );
            }
        }

        // Process visualizers (they don't modify the signal, just capture it)
        self.process_visualizers(context);
    }

    /// Process visualizers - capture audio data for display.
    /// Uses interleaved methods to avoid heap allocation in audio thread.
    fn process_visualizers(&mut self, _context: &ProcessContext) {
        for slot in &mut self.visualizers {
            if !slot.enabled {
                continue;
            }

            // Write directly from interleaved mix buffer - NO ALLOCATION!
            slot.buffer.write_interleaved(self.mix_buffer.as_slice());
            slot.buffer.update_levels_interleaved(self.mix_buffer.as_slice());
        }
    }

    /// Update metering.
    fn update_meters(&mut self, output: &[f32]) {
        let channels = 2;
        
        for frame in output.chunks(channels) {
            let left = frame.get(0).copied().unwrap_or(0.0);
            let right = frame.get(1).copied().unwrap_or(left);

            self.peak_left = self.peak_left.max(left.abs());
            self.peak_right = self.peak_right.max(right.abs());
            self.rms_sum_left += left * left;
            self.rms_sum_right += right * right;
        }

        self.meter_counter += output.len() / channels;

        if self.meter_counter >= self.meter_interval {
            self.state.meters.update_peak(self.peak_left, self.peak_right);

            let rms_left = (self.rms_sum_left / self.meter_counter as f32).sqrt();
            let rms_right = (self.rms_sum_right / self.meter_counter as f32).sqrt();
            self.state.meters.update_rms(rms_left, rms_right);

            let _ = self.event_producer.try_push(EngineEvent::PeakMeter {
                left: self.peak_left,
                right: self.peak_right,
            });

            self.peak_left = 0.0;
            self.peak_right = 0.0;
            self.rms_sum_left = 0.0;
            self.rms_sum_right = 0.0;
            self.meter_counter = 0;
        }
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
        self.meter_interval = (self.sample_rate / 24.0) as usize;
    }

    fn on_stream_stop(&mut self) {
        self.voice_allocator.panic();
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
        handle.note_on(60, 0.8);
        handle.note_on(64, 0.8);
        handle.note_on(67, 0.8);

        // Process commands
        engine.process_commands();

        // Should have 3 active voices
        assert_eq!(engine.voice_allocator.active_voice_count(), 3);
    }
}
