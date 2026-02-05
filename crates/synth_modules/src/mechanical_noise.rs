//! Mechanical Noise module for key and pedal sounds.
//!
//! Generates the mechanical sounds of acoustic piano action:
//! key clicks, hammer strikes, damper pedal noise, etc.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{Gain, Hertz, MidiNote, Milliseconds, NormalizedValue, SampleRate, Velocity};
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
    filter_state: f32,

    // Velocity for current note
    current_velocity: f32,

    // Sample rate
    sample_rate: SampleRate,

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

            filter_state: 0.0,
            current_velocity: 0.0,

            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Trigger a mechanical noise burst.
    fn trigger(&mut self, velocity: f32) {
        let vel_factor = 1.0 - self.velocity_sens.as_f32() * (1.0 - velocity);
        self.current_velocity = vel_factor;
        self.current_sample = 0;
        // Ensure at least 1 sample to prevent division by zero in generate_noise()
        self.envelope_samples =
            ((self.duration.as_f32() / 1000.0 * self.sample_rate.as_f32()) as usize).max(1);
        self.filter_state = 0.0;
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
        self.filter_state = self.filter_state * (1.0 - alpha) + noise * alpha;
        let filtered = self.filter_state;

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

        filtered * envelope * self.current_velocity * self.level.as_f32()
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
            .description("Key click and mechanical action sounds")
            .category(ModuleCategory::PhysicalModeling)
            .tag("noise")
            .tag("mechanical")
            .tag("click")
            .tag("piano")
            .parameter(
                ParameterDescriptor::choice(
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
                    Param::MechanicalNoise(MechanicalNoiseParam::Duration(Milliseconds::new(10.0))),
                    "Duration",
                )
                .description("Noise burst duration")
                .range(1.0, 100.0)
                .default(10.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::TimeSlider),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::MechanicalNoise(MechanicalNoiseParam::Cutoff(Hertz::new(3000.0))),
                    "Cutoff",
                )
                .description("Noise filter cutoff frequency")
                .range(100.0, 10000.0)
                .default(3000.0)
                .unit(ParameterUnit::Hertz)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::FrequencySlider),
            )
            .parameter(
                ParameterDescriptor::float(
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
                    Param::MechanicalNoise(MechanicalNoiseParam::Level(Gain::new(0.1))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.1)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Noise output"))
    }
}

impl PolyModule for MechanicalNoise {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        for i in 0..context.samples.as_usize() {
            self.output_buffer[i] = self.generate_noise();
        }

        if let Some(out) = outputs.get_mut("out") {
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

    fn reset(&mut self) {
        self.current_sample = self.envelope_samples;
        self.filter_state = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, velocity: Velocity) {
        // Trigger noise on key down (for KeyDown and Hammer types)
        if matches!(
            self.noise_type,
            MechanicalNoiseType::KeyDown | MechanicalNoiseType::Hammer
        ) {
            self.trigger(velocity.as_f32());
        }
    }

    fn note_off(&mut self) {
        // Trigger noise on key up (for KeyUp type)
        if matches!(self.noise_type, MechanicalNoiseType::KeyUp) {
            self.trigger(0.5); // Fixed velocity for key release
        }
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
