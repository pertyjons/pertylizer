//! ADSR Envelope generator module.

use std::collections::HashMap;

use crate::engine::typed_params::{EnvelopeParam, ModuleType, Param};
use crate::modules::core::*;
use crate::types::{BipolarValue, MidiNote, NormalizedValue, PortName, SampleRate, Seconds};

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
    attack: Seconds,
    decay: Seconds,
    sustain: NormalizedValue,
    release: Seconds,
    attack_curve: BipolarValue,
    decay_curve: BipolarValue,
    release_curve: BipolarValue,
    velocity_sensitivity: NormalizedValue,
    stage: EnvelopeStage,
    level: NormalizedValue,
    velocity: NormalizedValue,
    sample_rate: SampleRate,
    target_level: NormalizedValue,
    output_buffer: AudioBuffer,
}

impl Envelope {
    pub fn new() -> Self {
        Self {
            attack: Seconds::new(0.01),
            decay: Seconds::new(0.1),
            sustain: NormalizedValue::new(0.7),
            release: Seconds::new(0.3),
            attack_curve: BipolarValue::CENTER,
            decay_curve: BipolarValue::CENTER,
            release_curve: BipolarValue::CENTER,
            velocity_sensitivity: NormalizedValue::MAX,
            stage: EnvelopeStage::Idle,
            level: NormalizedValue::MIN,
            velocity: NormalizedValue::MAX,
            sample_rate: SampleRate::DVD_QUALITY,
            target_level: NormalizedValue::MIN,
            output_buffer: AudioBuffer::new(256),
        }
    }

    pub fn stage(&self) -> EnvelopeStage {
        self.stage
    }

    pub fn is_active(&self) -> bool {
        self.stage != EnvelopeStage::Idle
    }

    pub fn trigger(&mut self, velocity: f32) {
        self.velocity = NormalizedValue::new(velocity);
        self.stage = EnvelopeStage::Attack;
        self.target_level = NormalizedValue::MAX;
    }

    pub fn release(&mut self) {
        if self.stage != EnvelopeStage::Idle {
            self.stage = EnvelopeStage::Release;
            self.target_level = NormalizedValue::MIN;
        }
    }

    #[inline]
    fn process_sample(&mut self) -> f32 {
        let velocity_scale =
            1.0 - self.velocity_sensitivity.as_f32() * (1.0 - self.velocity.as_f32());

        match self.stage {
            EnvelopeStage::Idle => {
                self.level = NormalizedValue::MIN;
            }
            EnvelopeStage::Attack => {
                if self.attack.as_f32() <= 0.001 {
                    self.level = NormalizedValue::MAX;
                    self.stage = EnvelopeStage::Decay;
                    self.target_level = self.sustain;
                } else {
                    let base_coef = self.attack.to_exp_coeff(self.sample_rate);
                    let curve = self.attack_curve.as_f32();
                    let effective_coef = if curve.abs() < 0.01 {
                        base_coef
                    } else if curve < 0.0 {
                        base_coef.powf(1.0 + (-curve) * 3.0)
                    } else {
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
                if self.decay.as_f32() <= 0.001 {
                    self.level = self.sustain;
                    self.stage = EnvelopeStage::Sustain;
                } else {
                    let base_coef = self.decay.to_exp_coeff(self.sample_rate);
                    let sustain = self.sustain.as_f32();
                    let current = self.level.as_f32();
                    let curve = self.decay_curve.as_f32();
                    let effective_coef = if curve.abs() < 0.01 {
                        base_coef
                    } else if curve < 0.0 {
                        base_coef.powf(1.0 + (-curve) * 3.0)
                    } else {
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
                if self.release.as_f32() <= 0.001 {
                    self.level = NormalizedValue::MIN;
                    self.stage = EnvelopeStage::Idle;
                } else {
                    let base_coef = self.release.to_exp_coeff(self.sample_rate);
                    let current = self.level.as_f32();
                    let curve = self.release_curve.as_f32();
                    let effective_coef = if curve.abs() < 0.01 {
                        base_coef
                    } else if curve < 0.0 {
                        base_coef.powf(1.0 + (-curve) * 3.0)
                    } else {
                        base_coef.powf(1.0 / (1.0 + curve * 3.0))
                    };

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
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(EnvelopeParam::Attack(Seconds::new(0.01))),
                    "Attack",
                )
                .range(0.0, 10.0)
                .default(0.01)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::TimeSlider)
                .curve(ResponseCurve::Exponential),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(EnvelopeParam::Decay(Seconds::new(0.1))),
                    "Decay",
                )
                .range(0.0, 10.0)
                .default(0.1)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::TimeSlider)
                .curve(ResponseCurve::Exponential),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(EnvelopeParam::Sustain(NormalizedValue::new(0.7))),
                    "Sustain",
                )
                .range(0.0, 1.0)
                .default(0.7)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Slider),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(EnvelopeParam::Release(Seconds::new(0.3))),
                    "Release",
                )
                .range(0.0, 10.0)
                .default(0.3)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::TimeSlider)
                .curve(ResponseCurve::Exponential),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(EnvelopeParam::VelocitySensitivity(NormalizedValue::MAX)),
                    "Vel Sens",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(EnvelopeParam::AttackCurve(BipolarValue::CENTER)),
                    "Atk Curve",
                )
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(EnvelopeParam::DecayCurve(BipolarValue::CENTER)),
                    "Dec Curve",
                )
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Envelope(EnvelopeParam::ReleaseCurve(BipolarValue::CENTER)),
                    "Rel Curve",
                )
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::gate_input("gate", "Gate"))
            .port(PortDescriptor::control_input("velocity", "Vel"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}

impl PolyModule for Envelope {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let gate_input = inputs.get(PortName::GATE);
        let velocity_input = inputs.get(PortName::VELOCITY);
        let mut prev_gate = 0.0f32;

        for i in 0..context.samples.as_usize() {
            if let Some(gate) = gate_input {
                let gate_val = gate[i];
                if gate_val > 0.5 && prev_gate <= 0.5 {
                    let vel = velocity_input.map(|v| v[i]).unwrap_or(1.0);
                    self.trigger(vel);
                } else if gate_val <= 0.5 && prev_gate > 0.5 {
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

    fn set_param(&mut self, param: Param) {
        if let Param::Envelope(env_param) = param {
            match env_param {
                EnvelopeParam::Attack(a) => self.attack = Seconds::new(a.as_f32().max(0.0)),
                EnvelopeParam::Decay(d) => self.decay = Seconds::new(d.as_f32().max(0.0)),
                EnvelopeParam::Sustain(s) => self.sustain = s,
                EnvelopeParam::Release(r) => self.release = Seconds::new(r.as_f32().max(0.0)),
                EnvelopeParam::VelocitySensitivity(v) => self.velocity_sensitivity = v,
                EnvelopeParam::AttackCurve(c) => self.attack_curve = c,
                EnvelopeParam::DecayCurve(c) => self.decay_curve = c,
                EnvelopeParam::ReleaseCurve(c) => self.release_curve = c,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Envelope(env_param) = param {
            Some(match env_param {
                EnvelopeParam::Attack(_) => self.attack.as_f32(),
                EnvelopeParam::Decay(_) => self.decay.as_f32(),
                EnvelopeParam::Sustain(_) => self.sustain.as_f32(),
                EnvelopeParam::Release(_) => self.release.as_f32(),
                EnvelopeParam::VelocitySensitivity(_) => self.velocity_sensitivity.as_f32(),
                EnvelopeParam::AttackCurve(_) => self.attack_curve.as_f32(),
                EnvelopeParam::DecayCurve(_) => self.decay_curve.as_f32(),
                EnvelopeParam::ReleaseCurve(_) => self.release_curve.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Envelope(EnvelopeParam::Attack(self.attack)),
            Param::Envelope(EnvelopeParam::Decay(self.decay)),
            Param::Envelope(EnvelopeParam::Sustain(self.sustain)),
            Param::Envelope(EnvelopeParam::Release(self.release)),
            Param::Envelope(EnvelopeParam::VelocitySensitivity(
                self.velocity_sensitivity,
            )),
            Param::Envelope(EnvelopeParam::AttackCurve(self.attack_curve)),
            Param::Envelope(EnvelopeParam::DecayCurve(self.decay_curve)),
            Param::Envelope(EnvelopeParam::ReleaseCurve(self.release_curve)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Envelope
    }

    fn reset(&mut self) {
        self.stage = EnvelopeStage::Idle;
        self.level = NormalizedValue::MIN;
    }

    fn note_on(&mut self, _note: MidiNote, velocity: Velocity) {
        self.trigger(velocity.as_f32());
    }

    fn note_off(&mut self) {
        self.release();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
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
    }

    #[test]
    fn test_envelope_trigger() {
        let mut env = Envelope::new();
        env.sample_rate = SampleRate::DVD_QUALITY;
        env.trigger(1.0);
        assert_eq!(env.stage, EnvelopeStage::Attack);
    }
}
