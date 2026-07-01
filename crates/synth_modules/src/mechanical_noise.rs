//! Mechanical Noise module for key and pedal sounds.
//!
//! Generates the mechanical sounds of acoustic piano action:
//! key clicks, hammer strikes, damper pedal noise, etc.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, PortName, ProcessContext,
    ResponseCurve, WidgetHint,
};
use synth_core::{
    FilterState, Gain, Hertz, MidiNote, Milliseconds, NormalizedValue, SampleRate, Velocity,
};
use synth_core::{MechanicalNoiseParam, MechanicalNoiseType, ModuleType, Param};

/// Mechanical noise generator.
#[derive(Clone)]
pub struct MechanicalNoise {
    // Parameters
    noise_type: MechanicalNoiseType,
    duration: Milliseconds,
    cutoff: Hertz,
    velocity_sens: NormalizedValue,
    level: Gain,

    // Envelope state
    envelope_samples: usize,
    current_sample: usize,

    // Filter state for noise shaping
    filter_state: FilterState,

    // Velocity for current note (0.0-1.0 scaled factor)
    current_velocity: NormalizedValue,

    // Sample rate
    sample_rate: SampleRate,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    /// Previous Trigger-gate level for rising-edge detection (persists across blocks).
    prev_trigger: f32,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl MechanicalNoise {
    pub fn new() -> Self {
        Self {
            noise_type: MechanicalNoiseType::KeyDown,
            duration: Milliseconds::new(10.0),
            cutoff: Hertz::new(3000.0),
            velocity_sens: NormalizedValue::new(0.5),
            level: Gain::new(0.1),

            envelope_samples: 0,
            current_sample: 0,

            filter_state: FilterState::ZERO,
            current_velocity: NormalizedValue::MIN,

            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),
            prev_trigger: 0.0,
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Trigger a mechanical noise burst.
    fn trigger(&mut self, velocity: Velocity) {
        // vel_sens and duration are trigger-time params; apply their generic mod
        // offsets here (effective value at note-on), not per sample.
        let vel_sens = self
            .mod_offsets
            .effective("vel_sens", self.velocity_sens.as_f32());
        let vel_factor = 1.0 - vel_sens * (1.0 - velocity.as_f32());
        self.current_velocity = NormalizedValue::new(vel_factor);
        self.current_sample = 0;
        // Ensure at least 1 sample to prevent division by zero in generate_noise()
        let duration = Milliseconds::new(
            self.mod_offsets
                .effective("duration", self.duration.as_f32()),
        );
        self.envelope_samples = duration.to_samples(self.sample_rate).max(1);
        self.filter_state = FilterState::ZERO;
    }

    /// Generate filtered noise based on type.
    #[inline]
    fn generate_noise(&mut self) -> f32 {
        if self.current_sample >= self.envelope_samples {
            return 0.0;
        }

        // White noise base
        let noise = fastrand::f32() * 2.0 - 1.0;

        // Simple one-pole lowpass filter
        let cutoff_norm = (self.cutoff.as_f32() / self.sample_rate.as_f32()).min(0.5);
        let alpha = cutoff_norm; // Simplified coefficient
        self.filter_state =
            FilterState::new(self.filter_state.as_f32() * (1.0 - alpha) + noise * alpha);
        let filtered = self.filter_state.as_f32();

        // Envelope shape based on noise type
        let envelope = match self.noise_type {
            MechanicalNoiseType::KeyDown => {
                // Sharp attack, quick decay
                let progress = self.current_sample as f32 / self.envelope_samples as f32;
                let attack_end = 0.1;
                if progress < attack_end {
                    progress / attack_end
                } else {
                    (1.0 - (progress - attack_end) / (1.0 - attack_end)).powf(2.0)
                }
            }
            MechanicalNoiseType::KeyUp => {
                // Softer attack, longer decay
                let progress = self.current_sample as f32 / self.envelope_samples as f32;
                let attack_end = 0.2;
                if progress < attack_end {
                    progress / attack_end
                } else {
                    (1.0 - (progress - attack_end) / (1.0 - attack_end)).powf(1.5)
                }
            }
            MechanicalNoiseType::Pedal => {
                // Very short click
                let progress = self.current_sample as f32 / self.envelope_samples as f32;
                (1.0 - progress).powf(3.0)
            }
            MechanicalNoiseType::Hammer => {
                // Punchy attack
                let progress = self.current_sample as f32 / self.envelope_samples as f32;
                if progress < 0.05 {
                    progress / 0.05
                } else {
                    (1.0 - (progress - 0.05) / 0.95).powf(4.0)
                }
            }
        };

        self.current_sample += 1;

        filtered * envelope * self.current_velocity.as_f32() * self.level.as_f32()
    }
}

impl Default for MechanicalNoise {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for MechanicalNoise {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("mechanical_noise", "Mechanical Noise")
            .width(synth_core::ModuleWidth::Large)
            .description("Key click and mechanical action sounds")
            .category(ModuleCategory::PhysicalModeling)
            .tag("noise")
            .tag("mechanical")
            .tag("click")
            .tag("piano")
            .parameter(
                ParameterDescriptor::choice(
                    "type",
                    Param::MechanicalNoise(MechanicalNoiseParam::NoiseType(
                        MechanicalNoiseType::KeyDown,
                    )),
                    "Type",
                    MechanicalNoiseType::to_choices(),
                )
                .description("Type of mechanical noise")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    "duration",
                    Param::MechanicalNoise(MechanicalNoiseParam::Duration(Milliseconds::new(10.0))),
                    "Duration",
                )
                .description("Noise burst duration")
                .range(1.0, 100.0)
                .default(10.0)
                .widget(WidgetHint::TimeSlider),
            )
            .parameter(
                ParameterDescriptor::float(
                    "cutoff",
                    Param::MechanicalNoise(MechanicalNoiseParam::Cutoff(Hertz::new(3000.0))),
                    "Cutoff",
                )
                .description("Noise filter cutoff frequency")
                .range(100.0, 10000.0)
                .default(3000.0)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::FrequencySlider),
            )
            .parameter(
                ParameterDescriptor::float(
                    "vel_sens",
                    Param::MechanicalNoise(MechanicalNoiseParam::VelocitySens(
                        NormalizedValue::new(0.5),
                    )),
                    "Vel Sens",
                )
                .description("Velocity sensitivity")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::MechanicalNoise(MechanicalNoiseParam::Level(Gain::new(0.1))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.1)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::gate_input("trigger", "Trigger").description(
                    "Rising edge fires a burst (gate level = velocity). Connect: Gate, Clock",
                ),
            )
            .port(
                PortDescriptor::control_input("level_cv", "Level CV").description(
                    "Modulate output level (added to the Level knob). Connect: LFO, Envelope",
                ),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Noise output"))
    }
}

impl PolyModule for MechanicalNoise {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        // cutoff + level are read per-sample inside generate_noise; apply their
        // effective values to the fields for the block and restore after (single
        // loop, no early return). `level` is additionally a per-sample CV
        // destination (Level CV), folded into `self.level` before each sample.
        let saved = (self.cutoff, self.level);
        self.cutoff = Hertz::new(self.mod_offsets.effective("cutoff", self.cutoff.as_f32()));
        self.level = Gain::new(self.mod_offsets.effective("level", self.level.as_f32()));
        let base_level = self.level.as_f32();

        // Gate/CV inputs (unconnected readers return 0.0 → no change): a Trigger
        // rising edge fires a burst (gate level used as velocity), Level CV is
        // added to the per-sample output level.
        let trigger = inputs.reader(PortName::TRIGGER, 0.0);
        let level_cv = inputs.reader(PortName::LEVEL_CV, 0.0);

        for i in 0..context.samples.as_usize() {
            let g = trigger.get(i);
            if trigger.is_connected() && crate::math::rising_edge(g, self.prev_trigger) {
                self.trigger(Velocity::new(g.clamp(0.0, 1.0)));
            }
            self.prev_trigger = g;

            self.level = Gain::new((base_level + level_cv.get(i)).clamp(0.0, 1.0));
            self.output_buffer[i] = self.generate_noise();
        }

        (self.cutoff, self.level) = saved;

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::MechanicalNoise(p) = param {
            match p {
                MechanicalNoiseParam::NoiseType(t) => self.noise_type = t,
                MechanicalNoiseParam::Duration(d) => self.duration = d,
                MechanicalNoiseParam::Cutoff(f) => self.cutoff = f,
                MechanicalNoiseParam::VelocitySens(v) => self.velocity_sens = v,
                MechanicalNoiseParam::Level(g) => self.level = g,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::MechanicalNoise(p) = param {
            Some(match p {
                MechanicalNoiseParam::NoiseType(_) => self.noise_type.index() as f32,
                MechanicalNoiseParam::Duration(_) => self.duration.as_f32(),
                MechanicalNoiseParam::Cutoff(_) => self.cutoff.as_f32(),
                MechanicalNoiseParam::VelocitySens(_) => self.velocity_sens.as_f32(),
                MechanicalNoiseParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::MechanicalNoise(MechanicalNoiseParam::NoiseType(self.noise_type)),
            Param::MechanicalNoise(MechanicalNoiseParam::Duration(self.duration)),
            Param::MechanicalNoise(MechanicalNoiseParam::Cutoff(self.cutoff)),
            Param::MechanicalNoise(MechanicalNoiseParam::VelocitySens(self.velocity_sens)),
            Param::MechanicalNoise(MechanicalNoiseParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::MechanicalNoise
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.current_sample = self.envelope_samples;
        self.filter_state = FilterState::ZERO;
        self.prev_trigger = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, velocity: Velocity) {
        // Trigger noise on key down (for KeyDown and Hammer types)
        if matches!(
            self.noise_type,
            MechanicalNoiseType::KeyDown | MechanicalNoiseType::Hammer
        ) {
            self.trigger(velocity);
        }
    }

    fn note_off(&mut self) {
        // Trigger noise on key up (for KeyUp type)
        if matches!(self.noise_type, MechanicalNoiseType::KeyUp) {
            self.trigger(Velocity::new(0.5)); // Fixed velocity for key release
        }
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// `level` is a working mod destination via the generic store: driving it to
    /// 0 silences the burst, the base field is restored after the block, and
    /// clearing reverts.
    #[test]
    fn level_mod_offset_scales_output_and_restores() {
        let mut mn = MechanicalNoise::new();
        let desc = mn.descriptor();
        mn.mod_offsets_mut().unwrap().populate(&desc);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            ..ProcessContext::default()
        };
        fn burst_rms(mn: &mut MechanicalNoise, ctx: &ProcessContext) -> f32 {
            mn.note_on(MidiNote::A4, Velocity::MAX); // KeyDown default → triggers
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            mn.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i] * b[i]).sum::<f32>().sqrt()
        }

        let base = burst_rms(&mut mn, &ctx);
        assert!(base > 1e-4, "base burst present, got {base}");

        let level_before = mn.level.as_f32();
        mn.set_mod_offset("level", -1.0);
        let silent = burst_rms(&mut mn, &ctx);
        assert!(
            (mn.level.as_f32() - level_before).abs() < 1e-6,
            "level field must be restored after process"
        );
        assert!(
            silent < base * 0.05,
            "level→0 should silence the burst: {silent}"
        );

        mn.clear_mod_offsets();
        let reverted = burst_rms(&mut mn, &ctx);
        assert!(reverted > silent, "clearing restores level");
    }
}
