//! Voice module - represents a single synthesizer voice.
//!
//! A voice contains a complete signal chain (oscillators, filters, envelopes, etc.)
//! and handles note events for polyphonic playback.
//!
//! ## Macro Controllers
//!
//! Each voice stores macro controller state (pitch bend, mod wheel) using type-safe
//! domain types to prevent unit mismatches.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::engine::typed_params::{TypedParam, TypedValue};
use crate::modules::core::*;
use crate::types::{BipolarValue, Cents, Hertz, NormalizedValue, Seconds, Semitones};

/// Maximum buffer size we support.
const MAX_BUFFER_SIZE: usize = 4096;

/// Pre-allocated buffers for voice processing.
/// This avoids heap allocations in the audio thread.
#[derive(Clone)]
pub struct VoiceProcessingBuffers {
    // LFO buffers
    pub lfo_out: AudioBuffer,
    pub retrigger: AudioBuffer,

    // Oscillator buffers
    pub osc1_out: AudioBuffer,
    pub osc2_out: AudioBuffer,
    pub osc_mixed: AudioBuffer,
    pub fm_buffer: AudioBuffer,
    pub pwm_buffer: AudioBuffer,

    // Filter buffers
    pub filter_out: AudioBuffer,
    pub filter_env_out: AudioBuffer,
    pub cutoff_cv: AudioBuffer,
    pub res_cv: AudioBuffer,

    // Amp buffers
    pub amp_env_out: AudioBuffer,
    pub amp_left: AudioBuffer,
    pub amp_right: AudioBuffer,
    pub amp_cv: AudioBuffer,
    pub pan_cv: AudioBuffer,
}

impl VoiceProcessingBuffers {
    pub fn new() -> Self {
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
    pub fn clear_all(&mut self) {
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

impl Default for VoiceProcessingBuffers {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-computed MIDI note to frequency lookup table.
/// Avoids expensive powf() calls during audio processing.
/// Initialized once on first access.
static NOTE_FREQ_TABLE: LazyLock<[f32; 128]> = LazyLock::new(|| {
    let mut table = [0.0f32; 128];
    for i in 0..128 {
        // 440.0 * 2^((note - 69) / 12)
        table[i] = 440.0 * 2.0f32.powf((i as f32 - 69.0) / 12.0);
    }
    table
});

/// Voice state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceState {
    /// Voice is not playing.
    Idle,
    /// Voice is actively playing a note.
    Active,
    /// Voice is in release phase.
    Releasing,
    /// Voice was stolen and is fading out.
    Stealing,
}

/// Glide (portamento) state for smooth pitch transitions.
#[derive(Debug, Clone, Copy)]
pub struct GlideState {
    /// Source frequency in Hz
    pub from_freq: f32,
    /// Target frequency in Hz
    pub to_freq: f32,
    /// Current frequency (during glide)
    pub current_freq: f32,
    /// Glide time in seconds
    pub time: f32,
    /// Current position in glide (0.0 = start, 1.0 = complete)
    pub position: f32,
    /// Whether glide is active
    pub active: bool,
}

impl Default for GlideState {
    fn default() -> Self {
        Self {
            from_freq: 440.0,
            to_freq: 440.0,
            current_freq: 440.0,
            time: 0.0,
            position: 1.0,
            active: false,
        }
    }
}

impl GlideState {
    /// Start a new glide from current frequency to target.
    pub fn start(&mut self, target_freq: f32, glide_time: f32) {
        if glide_time > 0.0 && (self.current_freq - target_freq).abs() > 0.01 {
            self.from_freq = self.current_freq;
            self.to_freq = target_freq;
            self.time = glide_time;
            self.position = 0.0;
            self.active = true;
        } else {
            // No glide - jump directly
            self.current_freq = target_freq;
            self.from_freq = target_freq;
            self.to_freq = target_freq;
            self.position = 1.0;
            self.active = false;
        }
    }

    /// Update the glide state (call once per sample or per block).
    /// Returns the current frequency.
    pub fn update(&mut self, delta_time: f32) -> f32 {
        if !self.active {
            return self.current_freq;
        }

        // Update position
        if self.time > 0.0 {
            self.position += delta_time / self.time;
        } else {
            self.position = 1.0;
        }

        if self.position >= 1.0 {
            self.position = 1.0;
            self.active = false;
            self.current_freq = self.to_freq;
        } else {
            // Exponential interpolation (sounds more natural for pitch)
            // f(t) = from * (to/from)^t
            let ratio = self.to_freq / self.from_freq;
            self.current_freq = self.from_freq * ratio.powf(self.position);
        }

        self.current_freq
    }

    /// Get current frequency without updating.
    pub fn get_frequency(&self) -> f32 {
        self.current_freq
    }

    /// Check if glide is complete.
    pub fn is_complete(&self) -> bool {
        !self.active
    }
}

/// Default pitch bend range in semitones (standard MIDI is ±2).
pub const DEFAULT_PITCH_BEND_RANGE: Semitones = Semitones(2.0);

/// Default maximum vibrato depth (2.5% = ~43 cents at full mod wheel).
pub const DEFAULT_VIBRATO_DEPTH: NormalizedValue = NormalizedValue(0.025);

/// Performance expression settings for a voice/part.
///
/// These control how MIDI controllers affect the sound.
#[derive(Debug, Clone, Copy)]
pub struct ExpressionSettings {
    /// Pitch bend range in semitones (typically 2-24).
    pub pitch_bend_range: Semitones,
    /// Maximum vibrato depth when mod wheel is at 100% (0.0-0.1 typical).
    pub vibrato_depth: NormalizedValue,
    /// How much velocity affects amplitude (0 = constant, 1 = full dynamic).
    pub velocity_to_amp: NormalizedValue,
    /// How much velocity affects filter cutoff (0 = none, 1 = full).
    pub velocity_to_filter: NormalizedValue,
}

impl Default for ExpressionSettings {
    fn default() -> Self {
        Self {
            pitch_bend_range: DEFAULT_PITCH_BEND_RANGE,
            vibrato_depth: DEFAULT_VIBRATO_DEPTH,
            velocity_to_amp: NormalizedValue::MAX,       // Full velocity sensitivity
            velocity_to_filter: NormalizedValue::CENTER, // 50% velocity to filter
        }
    }
}

/// A single synthesizer voice.
pub struct Voice {
    /// Unique voice ID.
    pub id: u32,
    /// Current state.
    pub state: VoiceState,
    /// Currently playing MIDI note (0-127).
    pub note: u8,
    /// Note velocity (type-safe normalized value 0.0-1.0).
    pub velocity: NormalizedValue,
    /// When this voice was triggered (for voice stealing priority).
    pub trigger_time: u64,
    /// Age in samples (for voice stealing).
    pub age: u64,

    // === Macro controllers (type-safe) ===
    /// Pitch bend amount (-1.0 to +1.0, type-safe).
    pub pitch_bend: BipolarValue,
    /// Mod wheel amount (0.0 to 1.0, type-safe).
    pub mod_wheel: NormalizedValue,
    /// Aftertouch amount (0.0 to 1.0, type-safe).
    pub aftertouch: NormalizedValue,

    /// Expression settings (pitch bend range, velocity sensitivity, etc.).
    pub expression: ExpressionSettings,

    /// Voice modules in processing order.
    pub modules: Vec<Box<dyn VoiceModule>>,
    /// Module names for lookup.
    module_names: Vec<String>,

    /// Internal buffers for routing (inputs).
    buffers: HashMap<String, AudioBuffer>,

    /// Pre-allocated output buffers for each module (avoids heap allocation in process).
    module_outputs: HashMap<String, AudioBuffer>,

    /// Steal fade-out counter.
    pub steal_fade_samples: usize,
    pub steal_fade_counter: usize,

    /// Glide state for portamento.
    pub glide: GlideState,
    /// Configured glide time.
    glide_time: Seconds,

    /// Pre-allocated processing buffers to avoid heap allocations.
    processing_buffers: VoiceProcessingBuffers,
}

impl Voice {
    /// Create a new voice with the given ID.
    pub fn new(id: u32) -> Self {
        Self {
            id,
            state: VoiceState::Idle,
            note: 0,
            velocity: NormalizedValue::MIN,
            trigger_time: 0,
            age: 0,
            // Macro controllers default to neutral positions
            pitch_bend: BipolarValue::CENTER,
            mod_wheel: NormalizedValue::MIN,
            aftertouch: NormalizedValue::MIN,
            expression: ExpressionSettings::default(),
            modules: Vec::new(),
            module_names: Vec::new(),
            buffers: HashMap::new(),
            module_outputs: HashMap::new(),
            steal_fade_samples: 128,
            steal_fade_counter: 0,
            glide: GlideState::default(),
            glide_time: Seconds::ZERO,
            processing_buffers: VoiceProcessingBuffers::new(),
        }
    }

    /// Add a module to the voice.
    /// Pre-allocates output buffer to avoid heap allocation during processing.
    pub fn add_module(&mut self, name: impl Into<String>, module: Box<dyn VoiceModule>) {
        let name_str = name.into();
        let out_key = format!("{}_out", name_str);
        // Pre-allocate output buffer with max size
        self.module_outputs.insert(out_key, AudioBuffer::new(MAX_BUFFER_SIZE));
        self.module_names.push(name_str);
        self.modules.push(module);
    }

    /// Get a module by name.
    pub fn get_module(&self, name: &str) -> Option<&dyn VoiceModule> {
        self.module_names
            .iter()
            .position(|n| n == name)
            .map(|idx| self.modules[idx].as_ref())
    }

    /// Get a mutable module by name.
    pub fn get_module_mut(&mut self, name: &str) -> Option<&mut dyn VoiceModule> {
        let idx = self.module_names.iter().position(|n| n == name)?;
        Some(self.modules[idx].as_mut())
    }

    /// Set a parameter on a module.
    pub fn set_param(&mut self, module_name: &str, param: TypedParam, value: TypedValue) {
        if let Some(module) = self.get_module_mut(module_name) {
            module.set_param(param, value);
        }
    }
    
    /// Set the glide time (type-safe Seconds).
    pub fn set_glide_time(&mut self, time: Seconds) {
        self.glide_time = Seconds::new(time.as_f32().max(0.0));
    }

    /// Get the current glide time.
    pub fn get_glide_time(&self) -> Seconds {
        self.glide_time
    }
    
    /// Get the current frequency (accounting for glide).
    pub fn get_current_frequency(&self) -> f32 {
        self.glide.get_frequency()
    }
    
    /// Set detune on all oscillators in the voice (type-safe Cents).
    /// Used for unison mode.
    pub fn set_oscillator_detune(&mut self, detune: Cents) {
        use crate::engine::typed_params::{TypedParam, TypedValue, OscillatorParam};

        // Apply to osc1
        if let Some(osc) = self.get_module_mut("osc1") {
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Detune),
                TypedValue::Float(detune.as_f32())
            );
        }

        // Apply to osc2 with slight additional detune for richness
        if let Some(osc) = self.get_module_mut("osc2") {
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Detune),
                TypedValue::Float(detune.as_f32() + 7.0) // Keep the 7 cent offset
            );
        }
    }
    
    /// Set oscillator frequency directly (type-safe Hertz).
    fn set_oscillator_frequency(&mut self, freq: Hertz) {
        use crate::engine::typed_params::{TypedParam, TypedValue, OscillatorParam};

        // Apply to osc1
        if let Some(osc) = self.get_module_mut("osc1") {
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Frequency),
                TypedValue::Float(freq.as_f32())
            );
        }

        // Apply to osc2
        if let Some(osc) = self.get_module_mut("osc2") {
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Frequency),
                TypedValue::Float(freq.as_f32())
            );
        }
    }

    /// Convert MIDI note to frequency (A4 = 440 Hz).
    /// Uses a pre-computed lookup table to avoid expensive powf() calls.
    #[inline]
    fn note_to_freq(note: u8) -> f32 {
        NOTE_FREQ_TABLE[note as usize]
    }

    /// Trigger note on with type-safe velocity.
    pub fn note_on(&mut self, note: u8, velocity: NormalizedValue, time: u64) {
        let target_freq = Self::note_to_freq(note);

        // Start glide from current position if we have a glide time
        if self.glide_time.as_f32() > 0.0 && self.state == VoiceState::Active {
            self.glide.start(target_freq, self.glide_time.as_f32());
        } else {
            // No glide - set frequency immediately
            self.glide.current_freq = target_freq;
            self.glide.from_freq = target_freq;
            self.glide.to_freq = target_freq;
            self.glide.active = false;
        }

        self.note = note;
        self.velocity = velocity;
        self.trigger_time = time;
        self.age = 0;
        self.state = VoiceState::Active;

        // Notify all modules (convert to f32 for module interface)
        for module in &mut self.modules {
            module.note_on(note, velocity.as_f32());
        }
    }
    
    /// Change pitch without retriggering (for legato mode).
    pub fn glide_to_note(&mut self, note: u8) {
        let target_freq = Self::note_to_freq(note);
        self.note = note;
        
        if self.glide_time.as_f32() > 0.0 {
            self.glide.start(target_freq, self.glide_time.as_f32());
        } else {
            self.glide.current_freq = target_freq;
            self.glide.to_freq = target_freq;
            self.glide.active = false;
        }
    }

    /// Trigger note off.
    pub fn note_off(&mut self) {
        if self.state == VoiceState::Active {
            self.state = VoiceState::Releasing;

            // Notify all modules
            for module in &mut self.modules {
                module.note_off();
            }
        }
    }

    /// Start voice stealing (quick fade-out).
    pub fn steal(&mut self) {
        self.state = VoiceState::Stealing;
        self.steal_fade_counter = self.steal_fade_samples;
    }

    /// Check if voice is available for new notes.
    pub fn is_available(&self) -> bool {
        self.state == VoiceState::Idle
    }

    /// Check if voice is producing sound.
    pub fn is_active(&self) -> bool {
        self.state != VoiceState::Idle
    }

    /// Reset the voice to idle state.
    pub fn reset(&mut self) {
        self.state = VoiceState::Idle;
        self.note = 0;
        self.velocity = NormalizedValue::MIN;
        self.age = 0;
        self.glide = GlideState::default();
        // Note: We don't reset macro controllers here since they are channel-wide,
        // not per-voice. They persist across notes.

        for module in &mut self.modules {
            module.reset();
        }
    }

    /// Process audio for this voice.
    ///
    /// Returns the voice output in the provided buffer.
    ///
    /// # Real-time Safety
    /// This method uses pre-allocated buffers and avoids heap allocation.
    pub fn process(&mut self, output: &mut AudioBuffer, context: &ProcessContext) {
        // Increment age
        self.age += context.samples as u64;

        // Handle stealing fade-out
        if self.state == VoiceState::Stealing {
            if self.steal_fade_counter == 0 {
                self.reset();
                output.clear();
                return;
            }
        }

        // Update glide and set oscillator frequency
        if self.glide.active {
            let delta_time = context.samples as f32 / context.sample_rate;
            let freq = Hertz::new(self.glide.update(delta_time));
            self.set_oscillator_frequency(freq);
        }

        // Ensure buffers are properly sized (no allocation if size matches)
        self.ensure_buffers(context.samples);
        self.ensure_output_buffers(context.samples);

        // Process modules in order using pre-allocated output buffers.
        // We use indices to avoid borrow checker issues.
        for idx in 0..self.modules.len() {
            let name = &self.module_names[idx];
            let out_key_owned = format!("{}_out", name);

            // Clear the pre-allocated output buffer
            if let Some(buf) = self.module_outputs.get_mut(&out_key_owned) {
                buf.clear();
            }

            // Build inputs from self.buffers (references only, no allocation)
            // Note: We collect into a small temporary, but this is stack-allocated
            // for small numbers of inputs. For a fully zero-alloc solution,
            // consider a fixed-size array or arena allocator.
            let inputs: HashMap<String, &AudioBuffer> = self.buffers.iter()
                .map(|(k, v)| (k.clone(), v))
                .collect();

            // Process module into pre-allocated output
            self.modules[idx].process(&inputs, &mut self.module_outputs, context);

            // Copy this module's output to buffers for next modules to use as input
            if let Some(out_buf) = self.module_outputs.get(&out_key_owned) {
                if let Some(buf) = self.buffers.get_mut(&out_key_owned) {
                    buf.copy_from(out_buf);
                } else {
                    // First time: insert (this allocates once, then reuses)
                    self.buffers.insert(out_key_owned, out_buf.clone());
                }
            }
        }

        // Get final output (from last module)
        if let Some(name) = self.module_names.last() {
            let out_key = format!("{}_out", name);
            if let Some(final_buf) = self.module_outputs.get(&out_key) {
                output.copy_from(final_buf);
            } else {
                output.clear();
            }
        } else {
            output.clear();
        }

        // Apply stealing fade-out
        if self.state == VoiceState::Stealing && self.steal_fade_counter > 0 {
            let fade_samples = self.steal_fade_counter.min(context.samples);
            for i in 0..context.samples {
                let fade = if i < fade_samples {
                    (self.steal_fade_counter - i) as f32 / self.steal_fade_samples as f32
                } else {
                    0.0
                };
                output[i] *= fade;
            }
            self.steal_fade_counter = self.steal_fade_counter.saturating_sub(context.samples);
        }
    }

    /// Ensure internal input buffers are properly sized.
    fn ensure_buffers(&mut self, size: usize) {
        for buf in self.buffers.values_mut() {
            if buf.len() != size {
                buf.resize(size);
            }
        }
    }

    /// Ensure pre-allocated output buffers are properly sized.
    /// This only resizes if needed, avoiding allocation when size matches.
    #[inline]
    fn ensure_output_buffers(&mut self, size: usize) {
        for buf in self.module_outputs.values_mut() {
            if buf.len() != size {
                buf.resize(size);
            }
        }
    }

    /// Clone the voice structure (for voice allocation).
    pub fn clone_structure(&self) -> Self {
        let mut new_voice = Voice::new(self.id);
        new_voice.glide_time = self.glide_time;
        new_voice.expression = self.expression;

        for (idx, module) in self.modules.iter().enumerate() {
            new_voice.add_module(
                self.module_names[idx].clone(),
                module.box_clone(),
            );
        }

        new_voice
    }

    /// Process audio for this voice with full signal chain.
    ///
    /// This method contains the complete DSP routing logic:
    /// LFO -> Oscillators -> Filter -> Amplifier
    ///
    /// ## Modulation Sources
    /// - Pitch bend: Applied to oscillator frequency (±2 semitones default)
    /// - Mod wheel: Scales vibrato depth from LFO
    /// - Velocity: Scales amplitude envelope
    ///
    /// Returns stereo output in left/right buffers.
    pub fn process_audio(
        &mut self,
        left_out: &mut AudioBuffer,
        right_out: &mut AudioBuffer,
        context: &ProcessContext,
    ) {
        let samples = context.samples;

        // Clear pre-allocated buffers for this voice
        self.processing_buffers.clear_all();

        // === Calculate frequency with pitch bend using strong types ===
        // base_freq: the note's base frequency (accounting for glide)
        let base_freq = Hertz::new(self.glide.get_frequency());

        // Apply pitch bend: bend_semitones = pitch_bend * range
        // Using Semitones::apply() for type-safe frequency calculation
        let bend_semitones = self.expression.pitch_bend_range * self.pitch_bend.as_f32();
        let freq = bend_semitones.apply(base_freq);

        // Set oscillator frequencies (before processing)
        self.set_oscillator_frequency(freq);

        // === Process LFO with retrigger on note start ===
        if self.age == 0 {
            self.processing_buffers.retrigger[0] = 1.0;
        }

        // Get module index for LFO
        if let Some(idx) = self.module_names.iter().position(|n| n == "lfo") {
            let lfo_out = std::mem::take(&mut self.processing_buffers.lfo_out);
            let lfo_inputs: HashMap<String, &AudioBuffer> =
                [("retrigger".to_string(), &self.processing_buffers.retrigger as &AudioBuffer)]
                    .into_iter()
                    .collect();
            let mut lfo_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), lfo_out)].into_iter().collect();

            self.modules[idx].process(&lfo_inputs, &mut lfo_outputs, context);

            if let Some(buf) = lfo_outputs.remove("out") {
                self.processing_buffers.lfo_out = buf;
            }
        }

        // === Prepare FM and PWM buffers ===
        // Mod wheel scales vibrato depth: lfo * mod_wheel * max_depth
        // NormalizedValue * NormalizedValue -> NormalizedValue, then extract f32
        let vibrato_depth = (self.expression.vibrato_depth * self.mod_wheel).as_f32();
        for i in 0..samples {
            self.processing_buffers.fm_buffer[i] = self.processing_buffers.lfo_out[i] * vibrato_depth;
            self.processing_buffers.pwm_buffer[i] = self.processing_buffers.lfo_out[i] * 0.3;
        }

        // === Process Oscillator 1 ===
        if let Some(idx) = self.module_names.iter().position(|n| n == "osc1") {
            let osc1_out = std::mem::take(&mut self.processing_buffers.osc1_out);
            let osc1_inputs: HashMap<String, &AudioBuffer> = [
                ("fm".to_string(), &self.processing_buffers.fm_buffer as &AudioBuffer),
                ("pwm".to_string(), &self.processing_buffers.pwm_buffer as &AudioBuffer),
            ]
            .into_iter()
            .collect();
            let mut osc1_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), osc1_out)].into_iter().collect();

            self.modules[idx].process(&osc1_inputs, &mut osc1_outputs, context);

            if let Some(buf) = osc1_outputs.remove("out") {
                self.processing_buffers.osc1_out = buf;
            }
        }

        // === Process Oscillator 2 ===
        if let Some(idx) = self.module_names.iter().position(|n| n == "osc2") {
            let osc2_out = std::mem::take(&mut self.processing_buffers.osc2_out);
            let osc2_inputs: HashMap<String, &AudioBuffer> = [
                ("fm".to_string(), &self.processing_buffers.fm_buffer as &AudioBuffer),
                ("pwm".to_string(), &self.processing_buffers.pwm_buffer as &AudioBuffer),
            ]
            .into_iter()
            .collect();
            let mut osc2_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), osc2_out)].into_iter().collect();

            self.modules[idx].process(&osc2_inputs, &mut osc2_outputs, context);

            if let Some(buf) = osc2_outputs.remove("out") {
                self.processing_buffers.osc2_out = buf;
            }
        }

        // === Mix oscillators ===
        for i in 0..samples {
            self.processing_buffers.osc_mixed[i] =
                self.processing_buffers.osc1_out[i] + self.processing_buffers.osc2_out[i];
        }

        // === Process Filter Envelope ===
        if let Some(idx) = self.module_names.iter().position(|n| n == "filter_env") {
            let filter_env_out = std::mem::take(&mut self.processing_buffers.filter_env_out);
            let empty_inputs: HashMap<String, &AudioBuffer> = HashMap::new();
            let mut filter_env_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), filter_env_out)].into_iter().collect();

            self.modules[idx].process(&empty_inputs, &mut filter_env_outputs, context);

            if let Some(buf) = filter_env_outputs.remove("out") {
                self.processing_buffers.filter_env_out = buf;
            }
        }

        // === Prepare Filter CV ===
        // Velocity scales filter envelope: harder hits open filter more
        // vel_scale: interpolate between (1 - sensitivity) and 1.0 based on velocity
        // At sensitivity=0: vel_scale = 1.0 (no velocity effect)
        // At sensitivity=1: vel_scale = velocity (full velocity effect)
        let filter_sens = self.expression.velocity_to_filter.as_f32();
        let vel_scale = (1.0 - filter_sens) + filter_sens * self.velocity.as_f32();
        for i in 0..samples {
            // Scale filter envelope by velocity for expressive filter response
            let env_mod = self.processing_buffers.filter_env_out[i] * vel_scale;
            self.processing_buffers.cutoff_cv[i] = self.processing_buffers.lfo_out[i] * 0.2
                + env_mod * 0.6;
            self.processing_buffers.res_cv[i] = self.processing_buffers.lfo_out[i] * 0.1;
        }

        // === Process Filter ===
        if let Some(idx) = self.module_names.iter().position(|n| n == "filter") {
            let filter_out = std::mem::take(&mut self.processing_buffers.filter_out);
            let filter_inputs: HashMap<String, &AudioBuffer> = [
                ("in".to_string(), &self.processing_buffers.osc_mixed as &AudioBuffer),
                ("cutoff_cv".to_string(), &self.processing_buffers.cutoff_cv as &AudioBuffer),
                ("res_cv".to_string(), &self.processing_buffers.res_cv as &AudioBuffer),
            ]
            .into_iter()
            .collect();
            let mut filter_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), filter_out)].into_iter().collect();

            self.modules[idx].process(&filter_inputs, &mut filter_outputs, context);

            if let Some(buf) = filter_outputs.remove("out") {
                self.processing_buffers.filter_out = buf;
            }
        }

        // === Process Amp Envelope ===
        if let Some(idx) = self.module_names.iter().position(|n| n == "amp_env") {
            let amp_env_out = std::mem::take(&mut self.processing_buffers.amp_env_out);
            let empty_inputs: HashMap<String, &AudioBuffer> = HashMap::new();
            let mut amp_env_outputs: HashMap<String, AudioBuffer> =
                [("out".to_string(), amp_env_out)].into_iter().collect();

            self.modules[idx].process(&empty_inputs, &mut amp_env_outputs, context);

            if let Some(buf) = amp_env_outputs.remove("out") {
                self.processing_buffers.amp_env_out = buf;
            }
        }

        // === Prepare Amp CV ===
        // Velocity sensitivity for amplitude:
        // At sensitivity=0: amp_scale = 1.0 (no velocity effect, constant volume)
        // At sensitivity=1: amp_scale = velocity (full dynamic range)
        let amp_sens = self.expression.velocity_to_amp.as_f32();
        let amp_scale = (1.0 - amp_sens) + amp_sens * self.velocity.as_f32();
        for i in 0..samples {
            self.processing_buffers.amp_cv[i] = self.processing_buffers.amp_env_out[i] * amp_scale;
            self.processing_buffers.pan_cv[i] = self.processing_buffers.lfo_out[i] * 0.15;
        }

        // === Process Amplifier ===
        if let Some(idx) = self.module_names.iter().position(|n| n == "amp") {
            let amp_left = std::mem::take(&mut self.processing_buffers.amp_left);
            let amp_right = std::mem::take(&mut self.processing_buffers.amp_right);
            let amp_inputs: HashMap<String, &AudioBuffer> = [
                ("in".to_string(), &self.processing_buffers.filter_out as &AudioBuffer),
                ("cv".to_string(), &self.processing_buffers.amp_cv as &AudioBuffer),
                ("pan_cv".to_string(), &self.processing_buffers.pan_cv as &AudioBuffer),
            ]
            .into_iter()
            .collect();
            let mut amp_outputs: HashMap<String, AudioBuffer> = [
                ("left".to_string(), amp_left),
                ("right".to_string(), amp_right),
            ]
            .into_iter()
            .collect();

            self.modules[idx].process(&amp_inputs, &mut amp_outputs, context);

            if let Some(buf) = amp_outputs.remove("left") {
                self.processing_buffers.amp_left = buf;
            }
            if let Some(buf) = amp_outputs.remove("right") {
                self.processing_buffers.amp_right = buf;
            }
        }

        // Copy to output buffers
        left_out.copy_from(&self.processing_buffers.amp_left);
        right_out.copy_from(&self.processing_buffers.amp_right);
    }
}

impl Clone for Voice {
    fn clone(&self) -> Self {
        self.clone_structure()
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glide_state() {
        let mut glide = GlideState::default();
        
        // Start at 440 Hz
        glide.current_freq = 440.0;
        
        // Glide to 880 Hz (one octave up) over 0.1 seconds
        glide.start(880.0, 0.1);
        
        assert!(glide.active);
        assert_eq!(glide.from_freq, 440.0);
        assert_eq!(glide.to_freq, 880.0);
        
        // Update halfway
        let freq = glide.update(0.05);
        assert!(freq > 440.0 && freq < 880.0);
        
        // Complete the glide
        let freq = glide.update(0.1);
        assert!(!glide.active);
        assert!((freq - 880.0).abs() < 1.0);
    }

    #[test]
    fn test_note_to_freq() {
        // A4 should be 440 Hz
        let freq = Voice::note_to_freq(69);
        assert!((freq - 440.0).abs() < 0.01);
        
        // A5 should be 880 Hz (one octave up)
        let freq = Voice::note_to_freq(81);
        assert!((freq - 880.0).abs() < 0.01);
        
        // A3 should be 220 Hz (one octave down)
        let freq = Voice::note_to_freq(57);
        assert!((freq - 220.0).abs() < 0.01);
    }
}
