//! Audio Input module — reads live audio input from `ProcessContext::audio_input`.
//!
//! This is a voice module that outputs live microphone/line-in audio.
//! All voice instances read the same immutable buffer from `ProcessContext`.
//! Only runs in active voices — the user must play a note to "open" the
//! voice and hear the input through the patch (same as the Noise module).

use std::collections::HashMap;

use synth_core::AmplifierParam;
use synth_core::{
    AudioBuffer, Describable, Gain, InputPorts, MidiNote, ModuleCategory, ModuleDescriptor,
    ModuleType, Param, ParamModOffsets, ParameterDescriptor, ParameterUnit, PolyModule,
    PortDescriptor, PortName, ProcessContext, SampleRate, Velocity, WidgetHint,
};

/// Audio input module — routes live audio input into the voice graph.
#[derive(Clone)]
pub struct AudioInput {
    /// Output level.
    level: Gain,
    /// Sample rate.
    sample_rate: SampleRate,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    /// Output buffer.
    output_buffer: AudioBuffer,
}

impl AudioInput {
    pub fn new() -> Self {
        Self {
            level: Gain::new(1.0),
            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }
}

impl Default for AudioInput {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for AudioInput {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("audio_input", "Audio Input")
            .description("Live audio input from microphone or line-in")
            .category(ModuleCategory::Sampler)
            .tag("input")
            .tag("source")
            .tag("live")
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::Amplifier(AmplifierParam::Level(Gain::new(1.0))),
                    "Level",
                )
                .description("Input level")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Live audio output"))
    }
}

impl PolyModule for AudioInput {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext<'_>,
    ) {
        let n_samples = context.samples.as_usize();
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(n_samples);

        let level = self.mod_offsets.effective("level", self.level.as_f32());

        if let Some(audio_input) = context.audio_input {
            // Mix stereo input to mono (L+R)/2 and apply level
            for i in 0..n_samples {
                let left_idx = i * 2;
                let right_idx = left_idx + 1;
                let left = if left_idx < audio_input.len() {
                    audio_input[left_idx]
                } else {
                    0.0
                };
                let right = if right_idx < audio_input.len() {
                    audio_input[right_idx]
                } else {
                    left
                };
                self.output_buffer[i] = (left + right) * 0.5 * level;
            }
        } else {
            // No input — silence
            for i in 0..n_samples {
                self.output_buffer[i] = 0.0;
            }
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Amplifier(AmplifierParam::Level(g)) = param {
            self.level = Gain::new(g.as_f32().clamp(0.0, 1.0));
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Amplifier(AmplifierParam::Level(_)) = param {
            Some(self.level.as_f32())
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![Param::Amplifier(AmplifierParam::Level(self.level))]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::AudioInput
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {}

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Audio input is always-on when voice is active
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

    /// `level` is a working mod destination via the generic store: a negative
    /// offset reduces the passed-through input level, and clearing reverts.
    #[test]
    fn level_mod_offset_scales_output() {
        let mut ai = AudioInput::new();
        let desc = ai.descriptor();
        ai.mod_offsets_mut().unwrap().populate(&desc);

        // 64 interleaved stereo samples (all 0.5) → 32 mono frames.
        let stereo = [0.5f32; 64];
        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(32),
            audio_input: Some(&stereo),
            ..ProcessContext::default()
        };
        fn peak(ai: &mut AudioInput, ctx: &ProcessContext) -> f32 {
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(32));
            ai.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut ai, &ctx);
        assert!(
            (base - 0.5).abs() < 1e-4,
            "unity level passes input, got {base}"
        );

        ai.set_mod_offset("level", -0.5);
        let quieter = peak(&mut ai, &ctx);
        assert!(
            quieter < base * 0.6,
            "level offset should reduce output: {quieter} vs {base}"
        );

        ai.clear_mod_offsets();
        assert!((peak(&mut ai, &ctx) - base).abs() < 1e-4);
    }
}
