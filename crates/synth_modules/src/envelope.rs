//! ADSR Envelope generator module.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve,
    WidgetHint,
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
    // Transient automation overrides (replace the base value while active,
    // cleared on transport stop; the base param is never mutated).
    override_attack: Option<Seconds>,
    override_decay: Option<Seconds>,
    override_sustain: Option<NormalizedValue>,
    override_release: Option<Seconds>,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
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
            override_attack: None,
            override_decay: None,
            override_sustain: None,
            override_release: None,
            mod_offsets: ParamModOffsets::new(),
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

        // Effective ADSR values: a transient automation override replaces the
        // base for the duration it is active; the base is left untouched.
        let attack = self.override_attack.unwrap_or(self.attack);
        let decay = self.override_decay.unwrap_or(self.decay);
        let sustain = self.override_sustain.unwrap_or(self.sustain);
        let release = self.override_release.unwrap_or(self.release);

        match self.stage {
            EnvelopeStage::Idle => {
                self.level = NormalizedValue::MIN;
            }
            EnvelopeStage::Attack => {
                if attack.as_f32() <= 0.001 {
                    self.level = NormalizedValue::MAX;
                    self.stage = EnvelopeStage::Decay;
                    self.target_level = sustain;
                } else {
                    let base_coef = attack.to_exp_coeff(self.sample_rate);
                    let effective_coef = Self::apply_curve(base_coef, self.attack_curve.as_f32());

                    let target = self.target_level.as_f32();
                    let current = self.level.as_f32();
                    let new_level = target + (current - target) * effective_coef;
                    self.level = NormalizedValue::new(new_level.clamp(0.0, 1.0));

                    if self.level.as_f32() >= 0.999 {
                        self.level = NormalizedValue::MAX;
                        self.stage = EnvelopeStage::Decay;
                        self.target_level = sustain;
                    }
                }
            }
            EnvelopeStage::Decay => {
                if decay.as_f32() <= 0.001 {
                    self.level = sustain;
                    self.stage = EnvelopeStage::Sustain;
                } else {
                    let base_coef = decay.to_exp_coeff(self.sample_rate);
                    let sustain_level = sustain.as_f32();
                    let current = self.level.as_f32();
                    let effective_coef = Self::apply_curve(base_coef, self.decay_curve.as_f32());

                    let new_level = sustain_level + (current - sustain_level) * effective_coef;
                    self.level = NormalizedValue::new(new_level.clamp(0.0, 1.0));

                    // Only snap to Sustain once the glide has converged from the
                    // decay side. Normal decay ramps DOWN from peak, so `current`
                    // sits above `sustain_level` and we finish when it gets close.
                    // If `sustain` was dynamically raised ABOVE the current level,
                    // `current` is below the target: glide smoothly UP toward it
                    // rather than instantly snapping (which would click).
                    if (self.level.as_f32() - sustain_level).abs() <= 0.001 {
                        self.level = sustain;
                        self.stage = EnvelopeStage::Sustain;
                    }
                }
            }
            EnvelopeStage::Sustain => {
                self.level = sustain;
            }
            EnvelopeStage::Release => {
                if release.as_f32() <= 0.001 {
                    self.level = NormalizedValue::MIN;
                    self.stage = EnvelopeStage::Idle;
                } else {
                    let base_coef = release.to_exp_coeff(self.sample_rate);
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
                .description("Attack time (silence to peak)")
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
                .description("Decay time (peak to sustain)")
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
                .description("Sustain level while gate is held")
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
                .description("Release time (gate-off to silence)")
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
                .description("How much note velocity scales the envelope (0 = ignore, 1 = full)")
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
                .description("Attack curve shape (-1 = logarithmic, 0 = linear, +1 = exponential)")
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
                .description("Decay curve shape (-1 = logarithmic, 0 = linear, +1 = exponential)")
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
                .description("Release curve shape (-1 = logarithmic, 0 = linear, +1 = exponential)")
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

        // The ADSR params + curves + vel_sens are read per sample inside
        // process_sample / trigger, so apply their generic mod offsets to the
        // fields for the block and restore after (single loop, no early return).
        // Sequencer automation overrides (override_*) still take precedence over
        // the mod offset for attack/decay/sustain/release, since process_sample
        // reads override-or-base.
        let saved = (
            self.attack,
            self.decay,
            self.sustain,
            self.release,
            self.velocity_sensitivity,
            self.attack_curve,
            self.decay_curve,
            self.release_curve,
        );
        self.attack = Seconds::new(self.mod_offsets.effective("attack", self.attack.as_f32()));
        self.decay = Seconds::new(self.mod_offsets.effective("decay", self.decay.as_f32()));
        self.sustain =
            NormalizedValue::new(self.mod_offsets.effective("sustain", self.sustain.as_f32()));
        self.release = Seconds::new(self.mod_offsets.effective("release", self.release.as_f32()));
        self.velocity_sensitivity = NormalizedValue::new(
            self.mod_offsets
                .effective("vel_sens", self.velocity_sensitivity.as_f32()),
        );
        self.attack_curve = BipolarValue::new(
            self.mod_offsets
                .effective("atk_curve", self.attack_curve.as_f32()),
        );
        self.decay_curve = BipolarValue::new(
            self.mod_offsets
                .effective("dec_curve", self.decay_curve.as_f32()),
        );
        self.release_curve = BipolarValue::new(
            self.mod_offsets
                .effective("rel_curve", self.release_curve.as_f32()),
        );

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

        // Restore the base params so next block re-applies offsets from scratch.
        (
            self.attack,
            self.decay,
            self.sustain,
            self.release,
            self.velocity_sensitivity,
            self.attack_curve,
            self.decay_curve,
            self.release_curve,
        ) = saved;

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

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
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

    fn set_param_override(&mut self, param: Param) {
        if let Param::Envelope(env_param) = param {
            match env_param {
                EnvelopeParam::Attack(a) => {
                    self.override_attack = Some(Seconds::new(a.as_f32().max(0.0)));
                }
                EnvelopeParam::Decay(d) => {
                    self.override_decay = Some(Seconds::new(d.as_f32().max(0.0)));
                }
                EnvelopeParam::Sustain(s) => self.override_sustain = Some(s),
                EnvelopeParam::Release(r) => {
                    self.override_release = Some(Seconds::new(r.as_f32().max(0.0)));
                }
                // Curves and velocity sensitivity are not automation targets here.
                _ => {}
            }
        }
    }

    fn clear_param_overrides(&mut self) {
        self.override_attack = None;
        self.override_decay = None;
        self.override_sustain = None;
        self.override_release = None;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;

    #[test]
    fn test_envelope_creation() {
        let env = Envelope::new();
        assert_eq!(env.stage, EnvelopeStage::Idle);
    }

    /// `sustain` is a working mod destination via the generic store: with the
    /// gate held high the output settles at the (modulated) sustain level, and
    /// the base field is restored after the block.
    #[test]
    fn sustain_mod_offset_changes_level_and_restores() {
        use std::collections::HashMap;
        let mut env = Envelope::new();
        let desc = env.descriptor();
        env.mod_offsets_mut().unwrap().populate(&desc);

        // ~6000 samples: long enough to pass attack (0.01s) + decay (0.1s) @ 48k.
        let n = 6000;
        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(n),
            ..ProcessContext::default()
        };
        fn settled(env: &mut Envelope, ctx: &ProcessContext, n: usize) -> f32 {
            env.reset();
            let mut gate = AudioBuffer::new(n);
            for i in 0..n {
                gate[i] = 1.0; // hold gate high
            }
            let in_ports = [(PortName::GATE, &gate)];
            let inputs = InputPorts::new(&in_ports);
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(n));
            env.process(inputs, &mut outs, ctx);
            outs[&PortName::OUT][n - 1]
        }

        let base = settled(&mut env, &ctx, n); // sustaining at a positive level
        assert!(base > 0.5, "envelope sustaining, got {base}");

        let sustain_before = env.sustain.as_f32();
        env.set_mod_offset("sustain", -0.4);
        let lowered = settled(&mut env, &ctx, n);
        assert!(
            (env.sustain.as_f32() - sustain_before).abs() < 1e-6,
            "sustain field must be restored after process"
        );
        assert!(
            lowered < base - 0.1,
            "sustain offset should lower the level: {lowered} vs {base}"
        );

        env.clear_mod_offsets();
        assert!(
            (settled(&mut env, &ctx, n) - base).abs() < 0.05,
            "clearing reverts sustain"
        );
    }

    #[test]
    fn test_envelope_trigger() {
        let mut env = Envelope::new();
        env.sample_rate = SampleRate::DVD_QUALITY;
        env.trigger(Velocity::MAX);
        assert_eq!(env.stage, EnvelopeStage::Attack);
    }

    #[test]
    fn test_envelope_param_override_replaces_base_and_reverts() {
        let mut env = Envelope::new();
        env.sample_rate = SampleRate::DVD_QUALITY;
        env.set_param(Param::Envelope(EnvelopeParam::Sustain(
            NormalizedValue::new(0.5),
        )));
        env.set_param(Param::Envelope(EnvelopeParam::Attack(Seconds::new(0.2))));

        // In the sustain stage the level tracks the base sustain.
        env.stage = EnvelopeStage::Sustain;
        let _ = env.process_sample();
        assert!((env.level.as_f32() - 0.5).abs() < 1e-6);

        // Override replaces the base while active.
        env.set_param_override(Param::Envelope(EnvelopeParam::Sustain(
            NormalizedValue::new(0.9),
        )));
        env.set_param_override(Param::Envelope(EnvelopeParam::Attack(Seconds::new(0.01))));
        env.stage = EnvelopeStage::Sustain;
        let _ = env.process_sample();
        assert!((env.level.as_f32() - 0.9).abs() < 1e-6);

        // Base params are never mutated by the override.
        assert!((env.sustain.as_f32() - 0.5).abs() < 1e-6);
        assert!((env.attack.as_f32() - 0.2).abs() < 1e-6);
        assert_matches!(env.override_attack, Some(a) if (a.as_f32() - 0.01).abs() < 1e-6);

        // Clearing reverts to the base.
        env.clear_param_overrides();
        env.stage = EnvelopeStage::Sustain;
        let _ = env.process_sample();
        assert!((env.level.as_f32() - 0.5).abs() < 1e-6);
        assert!(env.override_sustain.is_none());
        assert!(env.override_attack.is_none());
    }
}
