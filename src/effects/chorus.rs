//! Chorus effect module.
//!
//! Features:
//! - Multi-voice chorus with phase-offset LFOs
//! - Adjustable rate and depth
//! - Smooth interpolated delay reading

use crate::engine::typed_params::{ChorusParam, ModuleType, Param};
use crate::modules::core::*;
use crate::types::{BufferIndex, Hertz, NormalizedValue, Phase, SampleRate, VoiceCount};

/// Chorus effect.
pub struct Chorus {
    // Parameters
    rate: Hertz,
    depth: NormalizedValue,
    mix: NormalizedValue,
    voices: VoiceCount,

    // Delay buffer
    buffer: Vec<f32>,
    write_pos: BufferIndex,

    // LFO state (per voice)
    lfo_phases: [Phase; 4],

    // State
    sample_rate: SampleRate,
}

impl Chorus {
    const MAX_DELAY_MS: f32 = 50.0;

    pub fn new() -> Self {
        Self {
            rate: Hertz::new(0.5),
            depth: NormalizedValue::CENTER,
            mix: NormalizedValue::CENTER,
            voices: VoiceCount::DUAL,
            buffer: vec![0.0; 48000], // Will be resized
            write_pos: BufferIndex::ZERO,
            lfo_phases: [
                Phase::new(0.0),
                Phase::new(0.25),
                Phase::new(0.5),
                Phase::new(0.75),
            ],
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    fn resize_buffer(&mut self) {
        let size = (Self::MAX_DELAY_MS / 1000.0 * self.sample_rate.as_f32()) as usize;
        if self.buffer.len() != size {
            self.buffer.resize(size, 0.0);
            self.write_pos = BufferIndex::ZERO;
        }
    }

    #[inline]
    fn read_interpolated(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        let read_pos = (self.write_pos.as_usize() as f32 - delay_samples).rem_euclid(len as f32);
        let idx0 = (read_pos as usize) % len;
        let idx1 = (idx0 + 1) % len;
        let frac = read_pos - read_pos.floor();
        self.buffer[idx0] * (1.0 - frac) + self.buffer[idx1] * frac
    }
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Chorus {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("chorus", "Chorus")
            .description("Multi-voice chorus effect")
            .category(ModuleCategory::Effect)
            .tag("chorus")
            .tag("effect")
            .tag("modulation")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .port(
                PortDescriptor::control_input("rate_cv", "Rate CV").description("Rate modulation"),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Chorus(ChorusParam::Rate(Hertz::new(0.5))),
                    "Rate",
                )
                .description("LFO rate")
                .range(0.1, 5.0)
                .default(0.5)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Chorus(ChorusParam::Depth(NormalizedValue::CENTER)),
                    "Depth",
                )
                .description("Modulation depth")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Chorus(ChorusParam::Mix(NormalizedValue::CENTER)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Chorus {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        self.resize_buffer();

        let base_delay_ms = 7.0;
        let mod_depth_ms = self.depth.as_f32() * 5.0;
        let phase_inc = self.rate.as_f32() / self.sample_rate.as_f32();

        for i in 0..input.len().min(output.len()) {
            let dry = input[i];

            // Write to delay buffer
            self.buffer[self.write_pos.as_usize()] = dry;

            // Sum chorus voices
            let mut wet = 0.0f32;
            let voice_count = self.voices.as_usize();
            for v in 0..voice_count {
                let lfo = (self.lfo_phases[v].as_f32() * std::f32::consts::TAU).sin();
                let delay_ms = base_delay_ms + mod_depth_ms * lfo;
                let delay_samples = delay_ms / 1000.0 * self.sample_rate.as_f32();

                wet += self.read_interpolated(delay_samples);

                // Advance LFO
                self.lfo_phases[v] =
                    Phase::new((self.lfo_phases[v].as_f32() + phase_inc).rem_euclid(1.0));
            }

            wet /= voice_count as f32;

            // Advance write position
            self.write_pos = self.write_pos.advance(self.buffer.len());

            let mix = self.mix.as_f32();
            output[i] = dry * (1.0 - mix) + wet * mix;
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = BufferIndex::ZERO;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = NormalizedValue::new(mix);
    }

    fn get_mix(&self) -> f32 {
        self.mix.as_f32()
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Chorus(chorus_param) = param {
            match chorus_param {
                ChorusParam::Rate(r) => {
                    self.rate = Hertz::new(r.as_f32().clamp(0.1, 5.0));
                }
                ChorusParam::Depth(d) => {
                    self.depth = d;
                }
                ChorusParam::Mix(m) => {
                    self.mix = m;
                }
                ChorusParam::Voices(v) => {
                    self.voices = VoiceCount::new(v.as_u8()).clamp_chorus();
                }
                _ => {}
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Chorus(chorus_param) = param {
            Some(match chorus_param {
                ChorusParam::Rate(_) => self.rate.as_f32(),
                ChorusParam::Depth(_) => self.depth.as_f32(),
                ChorusParam::Mix(_) => self.mix.as_f32(),
                ChorusParam::Voices(_) => self.voices.as_u8() as f32,
                _ => return None,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Chorus(ChorusParam::Rate(self.rate)),
            Param::Chorus(ChorusParam::Depth(self.depth)),
            Param::Chorus(ChorusParam::Mix(self.mix)),
            Param::Chorus(ChorusParam::Voices(self.voices)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Chorus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chorus_creation() {
        let chorus = Chorus::new();
        assert!((chorus.rate.as_f32() - 0.5).abs() < 0.001);
        assert_eq!(chorus.voices.as_u8(), 2);
    }

    #[test]
    fn test_chorus_no_nan() {
        let mut chorus = Chorus::new();

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: 256,
            ..Default::default()
        };

        let input = vec![0.5f32; 256];
        let mut output = vec![0.0f32; 256];

        for _ in 0..100 {
            chorus.process(&input, &mut output, &context);
        }

        for sample in &output {
            assert!(sample.is_finite(), "Chorus output is not finite");
        }
    }
}
