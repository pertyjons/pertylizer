//! ADSR Envelope generator module.
//!
//! Features:
//! - Standard ADSR envelope
//! - Exponential or linear curves
//! - Velocity sensitivity
//! - Retrigger modes

use std::collections::HashMap;

use crate::engine::typed_params::{
    TypedParam, TypedValue, EnvelopeParam, ModuleType,
};
use crate::modules::core::*;

/// Envelope stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvelopeStage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Envelope curve type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvelopeCurve {
    #[default]
    Linear,
    Exponential,
}

/// ADSR envelope generator.
#[derive(Clone)]
pub struct Envelope {
    // Parameters (times in seconds)
    attack: f32,
    decay: f32,
    sustain: f32,  // Level 0-1
    release: f32,
    
    // Curve type
    curve: EnvelopeCurve,
    
    // Velocity sensitivity
    velocity_sensitivity: f32,
    
    // State
    stage: EnvelopeStage,
    level: f32,
    velocity: f32,
    sample_rate: f32,
    
    // For exponential curves
    target_level: f32,
    
    // Output
    output_buffer: AudioBuffer,
}

impl Envelope {
    pub fn new() -> Self {
        Self {
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.3,
            curve: EnvelopeCurve::Exponential,
            velocity_sensitivity: 1.0,
            stage: EnvelopeStage::Idle,
            level: 0.0,
            velocity: 1.0,
            sample_rate: 48000.0,
            target_level: 0.0,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Get the current envelope stage.
    pub fn stage(&self) -> EnvelopeStage {
        self.stage
    }

    /// Check if the envelope is active (not idle).
    pub fn is_active(&self) -> bool {
        self.stage != EnvelopeStage::Idle
    }

    /// Trigger the envelope.
    pub fn trigger(&mut self, velocity: f32) {
        self.velocity = velocity;
        self.stage = EnvelopeStage::Attack;
        self.target_level = 1.0;
        // Optionally reset level to 0 for hard attack
        // self.level = 0.0;
    }

    /// Release the envelope.
    pub fn release(&mut self) {
        if self.stage != EnvelopeStage::Idle {
            self.stage = EnvelopeStage::Release;
            self.target_level = 0.0;
        }
    }

    /// Calculate the coefficient for exponential curves.
    #[inline]
    fn calculate_coefficient(time_seconds: f32, sample_rate: f32) -> f32 {
        if time_seconds <= 0.0 {
            1.0
        } else {
            let samples = time_seconds * sample_rate;
            // Time constant for ~99.3% of target in given time
            (-1.0 / samples).exp()
        }
    }

    /// Process a single sample.
    #[inline]
    fn process_sample(&mut self) -> f32 {
        let velocity_scale = 1.0 - self.velocity_sensitivity * (1.0 - self.velocity);

        match self.stage {
            EnvelopeStage::Idle => {
                self.level = 0.0;
            }

            EnvelopeStage::Attack => {
                match self.curve {
                    EnvelopeCurve::Linear => {
                        if self.attack > 0.0 {
                            let increment = 1.0 / (self.attack * self.sample_rate);
                            self.level += increment;
                        } else {
                            self.level = 1.0;
                        }
                    }
                    EnvelopeCurve::Exponential => {
                        // Handle zero/very short attack time
                        if self.attack <= 0.001 {
                            self.level = 1.0;
                        } else {
                            let coef = Self::calculate_coefficient(self.attack, self.sample_rate);
                            self.level = self.target_level + (self.level - self.target_level) * coef;
                        }
                    }
                }

                if self.level >= 0.999 {
                    self.level = 1.0;
                    self.stage = EnvelopeStage::Decay;
                    self.target_level = self.sustain;
                }
            }

            EnvelopeStage::Decay => {
                match self.curve {
                    EnvelopeCurve::Linear => {
                        if self.decay > 0.0 {
                            let decrement = (1.0 - self.sustain) / (self.decay * self.sample_rate);
                            self.level -= decrement;
                        } else {
                            self.level = self.sustain;
                        }
                    }
                    EnvelopeCurve::Exponential => {
                        // Handle zero/very short decay time
                        if self.decay <= 0.001 {
                            self.level = self.sustain;
                        } else {
                            let coef = Self::calculate_coefficient(self.decay, self.sample_rate);
                            self.level = self.target_level + (self.level - self.target_level) * coef;
                        }
                    }
                }

                if self.level <= self.sustain + 0.001 {
                    self.level = self.sustain;
                    self.stage = EnvelopeStage::Sustain;
                }
            }

            EnvelopeStage::Sustain => {
                self.level = self.sustain;
            }

            EnvelopeStage::Release => {
                match self.curve {
                    EnvelopeCurve::Linear => {
                        if self.release > 0.0 {
                            let decrement = self.sustain / (self.release * self.sample_rate);
                            self.level -= decrement;
                        } else {
                            self.level = 0.0;
                        }
                    }
                    EnvelopeCurve::Exponential => {
                        // Handle zero/very short release time
                        if self.release <= 0.001 {
                            self.level = 0.0;
                        } else {
                            let coef = Self::calculate_coefficient(self.release, self.sample_rate);
                            self.level = self.target_level + (self.level - self.target_level) * coef;
                        }
                    }
                }

                if self.level <= 0.001 {
                    self.level = 0.0;
                    self.stage = EnvelopeStage::Idle;
                }
            }
        }

        self.level * velocity_scale
    }
}

impl Default for Envelope {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Envelope {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("envelope", "ADSR")
            .description("ADSR envelope generator")
            .category(ModuleCategory::Envelope)
            .tag("envelope")
            .tag("adsr")
            .tag("modulation")
            .parameter(
                ParameterDescriptor::float(TypedParam::Envelope(EnvelopeParam::Attack), "Attack")
                    .description("Attack time")
                    .range(0.0, 10.0)
                    .default(0.01)
                    .unit(ParameterUnit::Seconds)
                    .widget(WidgetHint::TimeSlider)
                    .curve(ResponseCurve::Exponential),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Envelope(EnvelopeParam::Decay), "Decay")
                    .description("Decay time")
                    .range(0.0, 10.0)
                    .default(0.1)
                    .unit(ParameterUnit::Seconds)
                    .widget(WidgetHint::TimeSlider)
                    .curve(ResponseCurve::Exponential),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Envelope(EnvelopeParam::Sustain), "Sustain")
                    .description("Sustain level")
                    .range(0.0, 1.0)
                    .default(0.7)
                    .unit(ParameterUnit::Percent)
                    .widget(WidgetHint::Slider),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Envelope(EnvelopeParam::Release), "Release")
                    .description("Release time")
                    .range(0.0, 10.0)
                    .default(0.3)
                    .unit(ParameterUnit::Seconds)
                    .widget(WidgetHint::TimeSlider)
                    .curve(ResponseCurve::Exponential),
            )
            .port(PortDescriptor::gate_input("gate", "Gate").description("Gate input"))
            .port(PortDescriptor::control_input("velocity", "Vel").description("Velocity input"))
            .port(PortDescriptor::audio_output("out", "Out").description("Envelope output"))
    }
}

impl VoiceModule for Envelope {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        let gate_input = inputs.get("gate");
        let velocity_input = inputs.get("velocity");

        let mut prev_gate = 0.0f32;

        for i in 0..context.samples {
            // Check gate
            if let Some(gate) = gate_input {
                let gate_val = gate[i];
                
                // Rising edge = note on
                if gate_val > 0.5 && prev_gate <= 0.5 {
                    let vel = velocity_input.map(|v| v[i]).unwrap_or(1.0);
                    self.trigger(vel);
                }
                // Falling edge = note off
                else if gate_val <= 0.5 && prev_gate > 0.5 {
                    self.release();
                }
                
                prev_gate = gate_val;
            }

            self.output_buffer[i] = self.process_sample();
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Envelope(env_param) = param {
            match env_param {
                EnvelopeParam::Attack => {
                    if let Some(a) = value.as_float() {
                        self.attack = a.max(0.0);
                    }
                }
                EnvelopeParam::Decay => {
                    if let Some(d) = value.as_float() {
                        self.decay = d.max(0.0);
                    }
                }
                EnvelopeParam::Sustain => {
                    if let Some(s) = value.as_float() {
                        self.sustain = s.clamp(0.0, 1.0);
                    }
                }
                EnvelopeParam::Release => {
                    if let Some(r) = value.as_float() {
                        self.release = r.max(0.0);
                    }
                }
                // Curve parameters not yet implemented in base Envelope
                EnvelopeParam::AttackCurve | EnvelopeParam::DecayCurve | EnvelopeParam::ReleaseCurve => {}
            }
        }
    }
    
    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Envelope(env_param) = param {
            match env_param {
                EnvelopeParam::Attack => Some(TypedValue::Float(self.attack)),
                EnvelopeParam::Decay => Some(TypedValue::Float(self.decay)),
                EnvelopeParam::Sustain => Some(TypedValue::Float(self.sustain)),
                EnvelopeParam::Release => Some(TypedValue::Float(self.release)),
                _ => None,
            }
        } else {
            None
        }
    }
    
    fn module_type(&self) -> ModuleType {
        ModuleType::Envelope
    }

    fn reset(&mut self) {
        self.stage = EnvelopeStage::Idle;
        self.level = 0.0;
    }

    fn note_on(&mut self, _note: u8, velocity: f32) {
        self.trigger(velocity);
    }

    fn note_off(&mut self) {
        self.release();
    }

    fn box_clone(&self) -> Box<dyn VoiceModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_creation() {
        let env = Envelope::new();
        assert_eq!(env.stage, EnvelopeStage::Idle);
        assert!((env.level - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_envelope_trigger() {
        let mut env = Envelope::new();
        env.sample_rate = 48000.0;
        
        env.trigger(1.0);
        assert_eq!(env.stage, EnvelopeStage::Attack);
        
        // Process some samples
        for _ in 0..10000 {
            env.process_sample();
        }
        
        // Should be in sustain now
        assert!(env.stage == EnvelopeStage::Decay || env.stage == EnvelopeStage::Sustain);
    }

    #[test]
    fn test_envelope_release() {
        let mut env = Envelope::new();
        env.sample_rate = 48000.0;
        env.attack = 0.001;
        env.decay = 0.001;
        env.release = 0.1;
        
        env.trigger(1.0);
        
        // Process through attack and decay
        for _ in 0..1000 {
            env.process_sample();
        }
        
        env.release();
        assert_eq!(env.stage, EnvelopeStage::Release);
        
        // Process through release
        for _ in 0..50000 {
            env.process_sample();
        }
        
        assert_eq!(env.stage, EnvelopeStage::Idle);
        assert!(env.level < 0.01);
    }
}
