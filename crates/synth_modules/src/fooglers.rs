//! Fooglers (Weird Synthesis) module.
//!
//! Tiny 256-sample delay line with 2 controllable taps, high-frequency damping,
//! and feedback. Cheap CPU, rich modulated sounds — similar to physical modeling.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/11-weird-synthesis.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{FooglersParam, ModuleType, Param};
use synth_core::{MidiNote, NormalizedValue, PortName, SampleRate, Velocity};

/// Buffer size for the micro delay line.
const BUFFER_SIZE: usize = 256;

/// Fooglers — micro delay-line physical modeling synthesis.
#[derive(Clone)]
pub struct Fooglers {
    tap1: NormalizedValue,
    tap2: NormalizedValue,
    feedback: NormalizedValue,
    damping: NormalizedValue,
    level: NormalizedValue,
    sample_rate: SampleRate,
    /// Circular delay buffer (pre-allocated, fixed size).
    buffer: [f32; BUFFER_SIZE],
    /// Write position in buffer.
    write_pos: usize,
    /// One-pole lowpass state for damping.
    damp_state: f32,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    output_buffer: AudioBuffer,
}

impl Fooglers {
    pub fn new() -> Self {
        Self {
            tap1: NormalizedValue::new(0.3),
            tap2: NormalizedValue::new(0.7),
            feedback: NormalizedValue::new(0.8),
            damping: NormalizedValue::new(0.3),
            level: NormalizedValue::new(0.5),
            sample_rate: SampleRate::DVD_QUALITY,
            buffer: [0.0; BUFFER_SIZE],
            write_pos: 0,
            damp_state: 0.0,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }

    #[inline]
    fn read_tap(&self, position: f32) -> f32 {
        let delay = (position * (BUFFER_SIZE - 1) as f32) as usize;
        let idx = (self.write_pos + BUFFER_SIZE - delay) % BUFFER_SIZE;
        self.buffer[idx]
    }
}

impl Default for Fooglers {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Fooglers {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("fooglers", "Fooglers")
            .description("Micro delay-line weird synthesis (physical modeling)")
            .category(ModuleCategory::Oscillator)
            .tag("synthesis")
            .tag("physical")
            .tag("experimental")
            .parameter(
                ParameterDescriptor::float(
                    "tap1",
                    Param::Fooglers(FooglersParam::Tap1(NormalizedValue::new(0.3))),
                    "Tap 1",
                )
                .description("First tap position in delay line")
                .range(0.01, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "tap2",
                    Param::Fooglers(FooglersParam::Tap2(NormalizedValue::new(0.7))),
                    "Tap 2",
                )
                .description("Second tap position in delay line")
                .range(0.01, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "feedback",
                    Param::Fooglers(FooglersParam::Feedback(NormalizedValue::new(0.8))),
                    "Feedback",
                )
                .description("Feedback amount (sustain)")
                .range(0.0, 0.99)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "damping",
                    Param::Fooglers(FooglersParam::Damping(NormalizedValue::new(0.3))),
                    "Damping",
                )
                .description("High-frequency damping")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::Fooglers(FooglersParam::Level(NormalizedValue::new(0.5))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Excitation input. Connect: Noise, Oscillator"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Fooglers output"))
    }
}

impl PolyModule for Fooglers {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let audio_in = inputs.reader(PortName::IN, 0.0);
        // Generic mod offsets — all per-block constants, resolved once here.
        let feedback = self
            .mod_offsets
            .effective("feedback", self.feedback.as_f32())
            .min(0.99);
        let damp_coeff = self.mod_offsets.effective("damping", self.damping.as_f32());
        let level = self.mod_offsets.effective("level", self.level.as_f32());
        let tap1 = self.mod_offsets.effective("tap1", self.tap1.as_f32());
        let tap2 = self.mod_offsets.effective("tap2", self.tap2.as_f32());

        for i in 0..context.samples.as_usize() {
            let input = audio_in[i];

            // Read from two taps
            let tap1_val = self.read_tap(tap1);
            let tap2_val = self.read_tap(tap2);

            // Mix taps
            let tap_mix = (tap1_val + tap2_val) * 0.5;

            // Apply damping (one-pole lowpass)
            self.damp_state += damp_coeff * (tap_mix - self.damp_state);
            let damped = tap_mix * (1.0 - damp_coeff) + self.damp_state * damp_coeff;

            // Write to buffer: input + feedback
            let write_val = input + damped * feedback;
            self.buffer[self.write_pos] = crate::math::soft_clip(write_val);

            self.write_pos = (self.write_pos + 1) % BUFFER_SIZE;

            self.output_buffer[i] = damped * level;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Fooglers(p) = param {
            match p {
                FooglersParam::Tap1(v) => {
                    self.tap1 = NormalizedValue::new(v.as_f32().clamp(0.01, 1.0))
                }
                FooglersParam::Tap2(v) => {
                    self.tap2 = NormalizedValue::new(v.as_f32().clamp(0.01, 1.0))
                }
                FooglersParam::Feedback(v) => {
                    self.feedback = NormalizedValue::new(v.as_f32().clamp(0.0, 0.99));
                }
                FooglersParam::Damping(v) => self.damping = v,
                FooglersParam::Level(v) => self.level = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Fooglers(p) = param {
            Some(match p {
                FooglersParam::Tap1(_) => self.tap1.as_f32(),
                FooglersParam::Tap2(_) => self.tap2.as_f32(),
                FooglersParam::Feedback(_) => self.feedback.as_f32(),
                FooglersParam::Damping(_) => self.damping.as_f32(),
                FooglersParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Fooglers(FooglersParam::Tap1(self.tap1)),
            Param::Fooglers(FooglersParam::Tap2(self.tap2)),
            Param::Fooglers(FooglersParam::Feedback(self.feedback)),
            Param::Fooglers(FooglersParam::Damping(self.damping)),
            Param::Fooglers(FooglersParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Fooglers
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.buffer = [0.0; BUFFER_SIZE];
        self.write_pos = 0;
        self.damp_state = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Inject a click as excitation on note-on
        if self.write_pos < BUFFER_SIZE {
            self.buffer[self.write_pos] = 0.8;
        }
    }

    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `level` is a working mod destination via the generic store: a negative
    /// offset reduces the resonator output, and clearing restores it.
    #[test]
    fn level_mod_offset_scales_output() {
        let mut f = Fooglers::new();
        let desc = f.descriptor();
        f.mod_offsets_mut().unwrap().populate(&desc);
        f.note_on(MidiNote::A4, Velocity::MAX);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(512),
            ..ProcessContext::default()
        };
        fn rms(f: &mut Fooglers, ctx: &ProcessContext) -> f32 {
            // Excite with a steady input so the feedback network rings.
            let mut buf = AudioBuffer::new(512);
            for i in 0..512 {
                buf[i] = 0.3;
            }
            let in_ports = [(PortName::IN, &buf)];
            let inputs = InputPorts::new(&in_ports);
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(512));
            f.process(inputs, &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i] * b[i]).sum::<f32>().sqrt()
        }

        let base = rms(&mut f, &ctx);
        assert!(base > 1e-3, "base output present, got {base}");

        f.set_mod_offset("level", -0.9);
        let quieter = rms(&mut f, &ctx);
        assert!(
            quieter < base * 0.5,
            "level offset should reduce output: {quieter} vs {base}"
        );

        f.clear_mod_offsets();
        let reverted = rms(&mut f, &ctx);
        assert!(reverted > quieter, "clearing restores level");
    }
}
