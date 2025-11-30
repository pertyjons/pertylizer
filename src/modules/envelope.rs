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
use crate::types::{Seconds, NormalizedValue, BipolarValue, SampleRate};

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

/// ADSR envelope generator.
#[derive(Clone)]
pub struct Envelope {
    // Parameters (times in seconds)
    attack: Seconds,
    decay: Seconds,
    sustain: NormalizedValue,
    release: Seconds,

    // Per-stage curve shapes (-1.0 = log, 0.0 = linear, 1.0 = exp)
    attack_curve: BipolarValue,
    decay_curve: BipolarValue,
    release_curve: BipolarValue,

    // Velocity sensitivity
    velocity_sensitivity: NormalizedValue,

    // State
    stage: EnvelopeStage,
    level: NormalizedValue,
    velocity: NormalizedValue,
    sample_rate: SampleRate,

    // For exponential curves
    target_level: NormalizedValue,

    // Output
    output_buffer: AudioBuffer,
}

impl Envelope {
    pub fn new() -> Self {
        Self {
            attack: Seconds::new(0.01),
            decay: Seconds::new(0.1),
            sustain: NormalizedValue::new(0.7),
            release: Seconds::new(0.3),
            // Default to standard exponential (curve = 0)
            // Negative values = punchy/logarithmic, Positive = slower/more gradual
            attack_curve: BipolarValue::CENTER,    // 0.0 = standard exponential
            decay_curve: BipolarValue::CENTER,     // 0.0 = standard exponential
            release_curve: BipolarValue::CENTER,   // 0.0 = standard exponential
            velocity_sensitivity: NormalizedValue::MAX,
            stage: EnvelopeStage::Idle,
            level: NormalizedValue::MIN,
            velocity: NormalizedValue::MAX,
            sample_rate: SampleRate::DVD_QUALITY,
            target_level: NormalizedValue::MIN,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Apply curve shaping to a linear ramp value (0.0 to 1.0).
    ///
    /// `curve`: -1.0 = logarithmic, 0.0 = linear, 1.0 = exponential
    /// `x`: input value in 0.0..1.0 range
    ///
    /// Returns shaped value in 0.0..1.0 range
    #[inline]
    fn apply_curve(x: f32, curve: f32) -> f32 {
        if curve.abs() < 0.01 {
            // Nearly linear - just return x
            x
        } else if curve > 0.0 {
            // Exponential curve (fast start, slow end for attack; slow start, fast end for decay)
            // Using power function: x^(1 + curve*3) for smooth exponential
            x.powf(1.0 + curve * 3.0)
        } else {
            // Logarithmic curve (slow start, fast end for attack; fast start, slow end for decay)
            // Using power function: x^(1 / (1 - curve*3))
            x.powf(1.0 / (1.0 - curve * 3.0))
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
        self.velocity = NormalizedValue::new(velocity);
        self.stage = EnvelopeStage::Attack;
        self.target_level = NormalizedValue::MAX;
        // Optionally reset level to 0 for hard attack
        // self.level = NormalizedValue::MIN;
    }

    /// Release the envelope.
    pub fn release(&mut self) {
        if self.stage != EnvelopeStage::Idle {
            self.stage = EnvelopeStage::Release;
            self.target_level = NormalizedValue::MIN;
        }
    }

    /// Process a single sample using per-stage curve shaping.
    ///
    /// The curve values modify the exponential coefficients:
    /// - Negative curves (-1 to 0): Logarithmic feel (punchy attack, fast initial decay)
    /// - Zero: Standard exponential envelope
    /// - Positive curves (0 to 1): Slower, more gradual transitions
    #[inline]
    fn process_sample(&mut self) -> f32 {
        let velocity_scale = 1.0 - self.velocity_sensitivity.as_f32() * (1.0 - self.velocity.as_f32());

        match self.stage {
            EnvelopeStage::Idle => {
                self.level = NormalizedValue::MIN;
            }

            EnvelopeStage::Attack => {
                // Handle zero/very short attack time
                if self.attack.as_f32() <= 0.001 {
                    self.level = NormalizedValue::MAX;
                    self.stage = EnvelopeStage::Decay;
                    self.target_level = self.sustain;
                } else {
                    // Get base coefficient and apply curve shaping
                    let base_coef = self.attack.to_exp_coeff(self.sample_rate);
                    let curve = self.attack_curve.as_f32();

                    // Curve modifies the coefficient:
                    // - Negative (log): faster approach (coef closer to 0)
                    // - Positive (exp): slower approach (coef closer to 1)
                    let effective_coef = if curve.abs() < 0.01 {
                        base_coef
                    } else if curve < 0.0 {
                        // Logarithmic: faster response (more punchy)
                        base_coef.powf(1.0 + (-curve) * 3.0)
                    } else {
                        // Exponential: slower response
                        base_coef.powf(1.0 / (1.0 + curve * 3.0))
                    };

                    let target = self.target_level.as_f32();
                    let current = self.level.as_f32();
                    let new_level = target + (current - target) * effective_coef;
                    self.level = NormalizedValue::new_unchecked(new_level);

                    if self.level.as_f32() >= 0.999 {
                        self.level = NormalizedValue::MAX;
                        self.stage = EnvelopeStage::Decay;
                        self.target_level = self.sustain;
                    }
                }
            }

            EnvelopeStage::Decay => {
                // Handle zero/very short decay time
                if self.decay.as_f32() <= 0.001 {
                    self.level = self.sustain;
                    self.stage = EnvelopeStage::Sustain;
                } else {
                    let base_coef = self.decay.to_exp_coeff(self.sample_rate);
                    let sustain = self.sustain.as_f32();
                    let current = self.level.as_f32();
                    let curve = self.decay_curve.as_f32();

                    // Curve modifies decay character:
                    // - Negative (log): Fast initial drop, slow tail (punchy drums)
                    // - Positive (exp): Slow initial, faster end (natural decay)
                    let effective_coef = if curve.abs() < 0.01 {
                        base_coef
                    } else if curve < 0.0 {
                        // Logarithmic: faster decay (more punchy)
                        base_coef.powf(1.0 + (-curve) * 3.0)
                    } else {
                        // Exponential: slower, more natural decay
                        base_coef.powf(1.0 / (1.0 + curve * 3.0))
                    };

                    let new_level = sustain + (current - sustain) * effective_coef;
                    self.level = NormalizedValue::new_unchecked(new_level.max(sustain));

                    if self.level.as_f32() <= sustain + 0.001 {
                        self.level = self.sustain;
                        self.stage = EnvelopeStage::Sustain;
                    }
                }
            }

            EnvelopeStage::Sustain => {
                self.level = self.sustain;
            }

            EnvelopeStage::Release => {
                // Handle zero/very short release time
                if self.release.as_f32() <= 0.001 {
                    self.level = NormalizedValue::MIN;
                    self.stage = EnvelopeStage::Idle;
                } else {
                    let base_coef = self.release.to_exp_coeff(self.sample_rate);
                    let current = self.level.as_f32();
                    let curve = self.release_curve.as_f32();

                    // Curve modifies release character:
                    // - Negative (log): Fast initial drop (gated feel)
                    // - Positive (exp): Slow, natural fade
                    let effective_coef = if curve.abs() < 0.01 {
                        base_coef
                    } else if curve < 0.0 {
                        // Logarithmic: faster release
                        base_coef.powf(1.0 + (-curve) * 3.0)
                    } else {
                        // Exponential: slower, natural release
                        base_coef.powf(1.0 / (1.0 + curve * 3.0))
                    };

                    // Release always targets zero
                    let new_level = current * effective_coef;
                    self.level = NormalizedValue::new_unchecked(new_level.max(0.0));

                    if self.level.as_f32() <= 0.001 {
                        self.level = NormalizedValue::MIN;
                        self.stage = EnvelopeStage::Idle;
                    }
                }
            }
        }

        self.level.as_f32() * velocity_scale
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
            .parameter(
                ParameterDescriptor::float(TypedParam::Envelope(EnvelopeParam::VelocitySensitivity), "Vel Sens")
                    .description("Velocity sensitivity (0 = ignore velocity, 1 = full response)")
                    .range(0.0, 1.0)
                    .default(1.0)
                    .unit(ParameterUnit::Percent)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Envelope(EnvelopeParam::AttackCurve), "Atk Curve")
                    .description("Attack curve (-1 = punchy/fast, 0 = standard, +1 = slow/gradual)")
                    .range(-1.0, 1.0)
                    .default(0.0)
                    .unit(ParameterUnit::None)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Envelope(EnvelopeParam::DecayCurve), "Dec Curve")
                    .description("Decay curve (-1 = punchy/fast, 0 = standard, +1 = slow/gradual)")
                    .range(-1.0, 1.0)
                    .default(0.0)
                    .unit(ParameterUnit::None)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Envelope(EnvelopeParam::ReleaseCurve), "Rel Curve")
                    .description("Release curve (-1 = gated/fast, 0 = standard, +1 = slow/natural)")
                    .range(-1.0, 1.0)
                    .default(0.0)
                    .unit(ParameterUnit::None)
                    .widget(WidgetHint::Knob),
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
        self.sample_rate = SampleRate::new(context.sample_rate);
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
                        self.attack = Seconds::new(a.max(0.0));
                    }
                }
                EnvelopeParam::Decay => {
                    if let Some(d) = value.as_float() {
                        self.decay = Seconds::new(d.max(0.0));
                    }
                }
                EnvelopeParam::Sustain => {
                    if let Some(s) = value.as_float() {
                        self.sustain = NormalizedValue::new(s);
                    }
                }
                EnvelopeParam::Release => {
                    if let Some(r) = value.as_float() {
                        self.release = Seconds::new(r.max(0.0));
                    }
                }
                EnvelopeParam::VelocitySensitivity => {
                    if let Some(v) = value.as_float() {
                        self.velocity_sensitivity = NormalizedValue::new(v);
                    }
                }
                EnvelopeParam::AttackCurve => {
                    if let Some(c) = value.as_float() {
                        self.attack_curve = BipolarValue::new(c);
                    }
                }
                EnvelopeParam::DecayCurve => {
                    if let Some(c) = value.as_float() {
                        self.decay_curve = BipolarValue::new(c);
                    }
                }
                EnvelopeParam::ReleaseCurve => {
                    if let Some(c) = value.as_float() {
                        self.release_curve = BipolarValue::new(c);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Envelope(env_param) = param {
            match env_param {
                EnvelopeParam::Attack => Some(TypedValue::Float(self.attack.as_f32())),
                EnvelopeParam::Decay => Some(TypedValue::Float(self.decay.as_f32())),
                EnvelopeParam::Sustain => Some(TypedValue::Float(self.sustain.as_f32())),
                EnvelopeParam::Release => Some(TypedValue::Float(self.release.as_f32())),
                EnvelopeParam::VelocitySensitivity => Some(TypedValue::Float(self.velocity_sensitivity.as_f32())),
                EnvelopeParam::AttackCurve => Some(TypedValue::Float(self.attack_curve.as_f32())),
                EnvelopeParam::DecayCurve => Some(TypedValue::Float(self.decay_curve.as_f32())),
                EnvelopeParam::ReleaseCurve => Some(TypedValue::Float(self.release_curve.as_f32())),
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
        self.level = NormalizedValue::MIN;
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
        assert!((env.level.as_f32() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_envelope_trigger() {
        let mut env = Envelope::new();
        env.sample_rate = SampleRate::DVD_QUALITY;

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
        env.sample_rate = SampleRate::DVD_QUALITY;
        env.attack = Seconds::new(0.001);
        env.decay = Seconds::new(0.001);
        env.release = Seconds::new(0.1);

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
        assert!(env.level.as_f32() < 0.01);
    }
}
