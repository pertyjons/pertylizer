//! Voice module - represents a single synthesizer voice.
//!
//! A voice contains a complete signal chain (oscillators, filters, envelopes, etc.)
//! and handles note events for polyphonic playback.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::engine::typed_params::{TypedParam, TypedValue};
use crate::modules::core::*;

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

/// A single synthesizer voice.
pub struct Voice {
    /// Unique voice ID.
    pub id: u32,
    /// Current state.
    pub state: VoiceState,
    /// Currently playing MIDI note (0-127).
    pub note: u8,
    /// Note velocity (0.0-1.0).
    pub velocity: f32,
    /// When this voice was triggered (for voice stealing priority).
    pub trigger_time: u64,
    /// Age in samples (for voice stealing).
    pub age: u64,

    /// Voice modules in processing order.
    pub modules: Vec<Box<dyn VoiceModule>>,
    /// Module names for lookup.
    module_names: Vec<String>,

    /// Internal buffers for routing.
    buffers: HashMap<String, AudioBuffer>,

    /// Steal fade-out counter.
    steal_fade_samples: usize,
    steal_fade_counter: usize,
    
    /// Glide state for portamento.
    glide: GlideState,
    /// Configured glide time (seconds).
    glide_time: f32,
}

impl Voice {
    /// Create a new voice with the given ID.
    pub fn new(id: u32) -> Self {
        Self {
            id,
            state: VoiceState::Idle,
            note: 0,
            velocity: 0.0,
            trigger_time: 0,
            age: 0,
            modules: Vec::new(),
            module_names: Vec::new(),
            buffers: HashMap::new(),
            steal_fade_samples: 128,
            steal_fade_counter: 0,
            glide: GlideState::default(),
            glide_time: 0.0,
        }
    }

    /// Add a module to the voice.
    pub fn add_module(&mut self, name: impl Into<String>, module: Box<dyn VoiceModule>) {
        self.module_names.push(name.into());
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
    
    /// Set the glide time in seconds.
    pub fn set_glide_time(&mut self, time: f32) {
        self.glide_time = time.max(0.0);
    }
    
    /// Get the current glide time.
    pub fn get_glide_time(&self) -> f32 {
        self.glide_time
    }
    
    /// Get the current frequency (accounting for glide).
    pub fn get_current_frequency(&self) -> f32 {
        self.glide.get_frequency()
    }
    
    /// Set detune on all oscillators in the voice (in cents).
    /// Used for unison mode.
    pub fn set_oscillator_detune(&mut self, detune_cents: f32) {
        use crate::engine::typed_params::{TypedParam, TypedValue, OscillatorParam};
        
        // Apply to osc1
        if let Some(osc) = self.get_module_mut("osc1") {
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Detune),
                TypedValue::Float(detune_cents)
            );
        }
        
        // Apply to osc2 with slight additional detune for richness
        if let Some(osc) = self.get_module_mut("osc2") {
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Detune),
                TypedValue::Float(detune_cents + 7.0) // Keep the 7 cent offset
            );
        }
    }
    
    /// Set oscillator frequency directly (used during glide).
    fn set_oscillator_frequency(&mut self, freq: f32) {
        use crate::engine::typed_params::{TypedParam, TypedValue, OscillatorParam};
        
        // Apply to osc1
        if let Some(osc) = self.get_module_mut("osc1") {
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Frequency),
                TypedValue::Float(freq)
            );
        }
        
        // Apply to osc2
        if let Some(osc) = self.get_module_mut("osc2") {
            osc.set_param(
                TypedParam::Oscillator(OscillatorParam::Frequency),
                TypedValue::Float(freq)
            );
        }
    }

    /// Convert MIDI note to frequency (A4 = 440 Hz).
    /// Uses a pre-computed lookup table to avoid expensive powf() calls.
    #[inline]
    fn note_to_freq(note: u8) -> f32 {
        NOTE_FREQ_TABLE[note as usize]
    }

    /// Trigger note on.
    pub fn note_on(&mut self, note: u8, velocity: f32, time: u64) {
        let target_freq = Self::note_to_freq(note);
        
        // Start glide from current position if we have a glide time
        if self.glide_time > 0.0 && self.state == VoiceState::Active {
            self.glide.start(target_freq, self.glide_time);
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

        // Notify all modules
        for module in &mut self.modules {
            module.note_on(note, velocity);
        }
    }
    
    /// Change pitch without retriggering (for legato mode).
    pub fn glide_to_note(&mut self, note: u8) {
        let target_freq = Self::note_to_freq(note);
        self.note = note;
        
        if self.glide_time > 0.0 {
            self.glide.start(target_freq, self.glide_time);
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
        self.velocity = 0.0;
        self.age = 0;
        self.glide = GlideState::default();

        for module in &mut self.modules {
            module.reset();
        }
    }

    /// Process audio for this voice.
    ///
    /// Returns the voice output in the provided buffer.
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
            let freq = self.glide.update(delta_time);
            self.set_oscillator_frequency(freq);
        }

        // Ensure buffers exist
        self.ensure_buffers(context.samples);

        // Process modules in order
        // For now, simple serial processing
        // A real implementation would use a graph-based approach

        let mut outputs: HashMap<String, AudioBuffer> = HashMap::new();

        // Create output buffer for each module
        for name in &self.module_names {
            outputs.insert(format!("{}_out", name), AudioBuffer::new(context.samples));
        }

        // Process each module
        for (idx, module) in self.modules.iter_mut().enumerate() {
            let name = &self.module_names[idx];

            // Clear and prepare output
            let out_key = format!("{}_out", name);
            if let Some(buf) = outputs.get_mut(&out_key) {
                buf.clear();
            }

            // Build inputs from self.buffers
            let inputs: HashMap<String, &AudioBuffer> = self.buffers.iter()
                .map(|(k, v)| (k.clone(), v))
                .collect();

            // Process
            module.process(&inputs, &mut outputs, context);

            // Make this module's output available as input for next modules
            if let Some(out_buf) = outputs.get(&out_key) {
                self.buffers.insert(out_key.clone(), out_buf.clone());
            }
        }

        // Get final output (from last module or specific output module)
        let final_output = self.module_names.last().and_then(|name| {
            outputs.get(&format!("{}_out", name))
        });

        if let Some(final_buf) = final_output {
            output.copy_from(final_buf);
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

    /// Ensure internal buffers are properly sized.
    fn ensure_buffers(&mut self, size: usize) {
        for buf in self.buffers.values_mut() {
            if buf.len() != size {
                buf.resize(size);
            }
        }
    }

    /// Clone the voice structure (for voice allocation).
    pub fn clone_structure(&self) -> Self {
        let mut new_voice = Voice::new(self.id);
        new_voice.glide_time = self.glide_time;
        
        for (idx, module) in self.modules.iter().enumerate() {
            new_voice.add_module(
                self.module_names[idx].clone(),
                module.box_clone(),
            );
        }

        new_voice
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
