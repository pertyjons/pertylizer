//! ADSR Envelope generator module.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{
    BipolarValue, MidiNote, NormalizedValue, PortName, SampleRate, Seconds, Velocity,
};
use synth_core::{EnvelopeParam, ModuleType, Param};

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

impl EnvelopeStage {
    /// Convert stage to u32 for atomic storage.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Idle => 0,
            Self::Attack => 1,
            Self::Decay => 2,
            Self::Sustain => 3,
            Self::Release => 4,
        }
    }

    /// Convert u32 back to stage.
    #[must_use]
    pub const fn from_u32(val: u32) -> Self {
        match val {
            1 => Self::Attack,
            2 => Self::Decay,
            3 => Self::Sustain,
            4 => Self::Release,
            _ => Self::Idle,
        }
    }
}

// ============================================================================
// ENVELOPE POSITION BUFFER
// ============================================================================

/// Lock-free buffer for sharing envelope time position with GUI.
///
/// Stores the current stage and time elapsed in that stage for visualization.
/// All voices write to the same buffer - the GUI shows the most recent state.
#[derive(Debug, Default)]
pub struct EnvelopePositionBuffer {
    /// Current stage (0=Idle, 1=Attack, 2=Decay, 3=Sustain, 4=Release).
    stage: AtomicU32,
    /// Time elapsed in current stage (seconds, stored as f32 bits).
    time_in_stage: AtomicU32,
}

impl EnvelopePositionBuffer {
    /// Create a new position buffer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stage: AtomicU32::new(0),
            time_in_stage: AtomicU32::new(0.0f32.to_bits()),
        }
    }

    /// Set the envelope state (called from audio thread).
    pub fn set(&self, stage: EnvelopeStage, time_in_stage: f32) {
        self.stage.store(stage.as_u32(), Ordering::Relaxed);
        self.time_in_stage
            .store(time_in_stage.to_bits(), Ordering::Relaxed);
    }

    /// Get the envelope state (called from GUI thread).
    /// Returns (stage, time_in_stage).
    #[must_use]
    pub fn get(&self) -> (EnvelopeStage, Seconds) {
        let stage = EnvelopeStage::from_u32(self.stage.load(Ordering::Relaxed));
        let time = Seconds::new(f32::from_bits(self.time_in_stage.load(Ordering::Relaxed)));
        (stage, time)
    }

    /// Get just the stage.
    #[must_use]
    pub fn stage(&self) -> EnvelopeStage {
        EnvelopeStage::from_u32(self.stage.load(Ordering::Relaxed))
    }

    /// Get time in current stage (seconds).
    #[must_use]
    pub fn time_in_stage(&self) -> Seconds {
        Seconds::new(f32::from_bits(self.time_in_stage.load(Ordering::Relaxed)))
    }
}

impl Clone for EnvelopePositionBuffer {
    fn clone(&self) -> Self {
        Self {
            stage: AtomicU32::new(self.stage.load(Ordering::Relaxed)),
            time_in_stage: AtomicU32::new(self.time_in_stage.load(Ordering::Relaxed)),
        }
    }
}

// ============================================================================
// ADSR ENVELOPE
// ============================================================================

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
    /// Time elapsed in current stage (seconds).
    time_in_stage: Seconds,
    /// Position buffer for GUI visualization.
    position_buffer: Arc<EnvelopePositionBuffer>,
    /// Previous gate value for edge detection (persists across buffers).
    prev_gate: NormalizedValue,
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
            output_buffer: AudioBuffer::new(1024),
            time_in_stage: Seconds::ZERO,
            position_buffer: Arc::new(EnvelopePositionBuffer::new()),
            prev_gate: NormalizedValue::MIN,
        }
    }

    /// Get the position buffer for GUI sync.
    #[must_use]
    pub fn position_buffer(&self) -> Arc<EnvelopePositionBuffer> {
        Arc::clone(&self.position_buffer)
    }

    pub fn stage(&self) -> EnvelopeStage {
        self.stage
    }

    pub fn is_active(&self) -> bool {
        self.stage != EnvelopeStage::Idle
    }

    pub fn trigger(&mut self, velocity: Velocity) {
        self.velocity = NormalizedValue::new(velocity.as_f32());
        self.stage = EnvelopeStage::Attack;
        self.target_level = NormalizedValue::MAX;
        self.time_in_stage = Seconds::ZERO;
    }

    pub fn release(&mut self) {
        if self.stage != EnvelopeStage::Idle {
            self.stage = EnvelopeStage::Release;
            self.target_level = NormalizedValue::MIN;
            self.time_in_stage = Seconds::ZERO;
        }
    }

    /// Apply curve shaping to a base exponential coefficient.
    ///
    /// Negative curve = slower start (raise coeff), positive = faster start (lower coeff).
    #[inline]
    fn apply_curve(base_coef: f32, curve: f32) -> f32 {
        crate::math::apply_curve_shaping(base_coef, curve)
    }

    #[inline]
    fn process_sample(&mut self) -> f32 {
        let velocity_scale = crate::math::velocity_sensitivity(
            self.velocity.as_f32(),
            self.velocity_sensitivity.as_f32(),
        );

        let prev_stage = self.stage;

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
                    let effective_coef = Self::apply_curve(base_coef, self.attack_curve.as_f32());

                    let target = self.target_level.as_f32();
                    let current = self.level.as_f32();
                    let new_level = target + (current - target) * effective_coef;
                    self.level = NormalizedValue::new(new_level.clamp(0.0, 1.0));

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
                    let effective_coef = Self::apply_curve(base_coef, self.decay_curve.as_f32());

                    let new_level = sustain + (current - sustain) * effective_coef;
                    self.level = NormalizedValue::new(new_level.clamp(0.0, 1.0));

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
                    let effective_coef = Self::apply_curve(base_coef, self.release_curve.as_f32());

                    let new_level = current * effective_coef;
                    self.level = NormalizedValue::new(new_level.clamp(0.0, 1.0));

                    if self.level.as_f32() <= 0.001 {
                        self.level = NormalizedValue::MIN;
                        self.stage = EnvelopeStage::Idle;
                    }
                }
            }
        }

        // Update time tracking
        if self.stage != prev_stage {
            // Stage changed - start at one sample so the position buffer
            // never briefly reports zero on the transition sample.
            self.time_in_stage = Seconds::new(1.0 / self.sample_rate.as_f32());
        } else if self.stage != EnvelopeStage::Idle {
            // Increment time (1 sample)
            self.time_in_stage =
                Seconds::new(self.time_in_stage.as_f32() + 1.0 / self.sample_rate.as_f32());
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
                    "attack",
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
                    "decay",
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
                    "sustain",
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
                    "release",
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
                    "vel_sens",
                    Param::Envelope(EnvelopeParam::VelocitySensitivity(NormalizedValue::MAX)),
                    "Vel Sens",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "atk_curve",
                    Param::Envelope(EnvelopeParam::AttackCurve(BipolarValue::CENTER)),
                    "Atk Curve",
                )
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "dec_curve",
                    Param::Envelope(EnvelopeParam::DecayCurve(BipolarValue::CENTER)),
                    "Dec Curve",
                )
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "rel_curve",
                    Param::Envelope(EnvelopeParam::ReleaseCurve(BipolarValue::CENTER)),
                    "Rel Curve",
                )
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::gate_input("gate", "Gate")
                    .description("Starts/releases the envelope. Automatic from keyboard"),
            )
            .port(
                PortDescriptor::control_input("velocity", "Vel")
                    .description("Velocity. Automatic from keyboard"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description(
                "Envelope signal (0–1). Connect to: Filter Cutoff CV, Amplifier CV, Oscillator FM",
            ))
    }
}

impl PolyModule for Envelope {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let gate_reader = inputs.reader(PortName::GATE, 0.0);
        let velocity_reader = inputs.reader(PortName::VELOCITY, 1.0);

        for i in 0..context.samples.as_usize() {
            if gate_reader.is_connected() {
                let gate_val = gate_reader[i];
                if crate::math::rising_edge(gate_val, self.prev_gate.as_f32()) {
                    let vel = Velocity::new(velocity_reader[i]);
                    self.trigger(vel);
                } else if gate_val <= 0.5 && self.prev_gate.as_f32() > 0.5 {
                    self.release();
                }
                self.prev_gate = NormalizedValue::new(gate_val);
            }
            self.output_buffer[i] = self.process_sample();
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }

        // Update position buffer for GUI visualization (stage + time)
        self.position_buffer
            .set(self.stage, self.time_in_stage.as_f32());
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
        self.time_in_stage = Seconds::ZERO;
        self.prev_gate = NormalizedValue::MIN;
    }

    fn note_on(&mut self, _note: MidiNote, velocity: Velocity) {
        self.trigger(velocity);
    }

    fn note_off(&mut self) {
        self.release();
    }

    fn is_release_done(&self) -> bool {
        self.stage == EnvelopeStage::Idle
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
        env.trigger(Velocity::MAX);
        assert_eq!(env.stage, EnvelopeStage::Attack);
    }
}
