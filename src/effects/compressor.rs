//! Dynamics compressor with adjustable threshold, ratio, attack, and release.

use crate::engine::typed_params::{CompressorParam, ModuleType, Param};
use crate::modules::{
    Describable, AudioEffect, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PortDescriptor, ProcessContext, WidgetHint,
};
use crate::types::{Decibels, Milliseconds, NormalizedValue, Ratio, SampleRate};

/// Compressor effect with envelope follower.
pub struct Compressor {
    // Parameters
    threshold: Decibels,
    ratio: Ratio,
    attack: Milliseconds,
    release: Milliseconds,
    makeup: Decibels,
    mix: NormalizedValue,

    // Envelope state
    envelope: f32,

    // State
    sample_rate: SampleRate,
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            threshold: Decibels::new(-20.0),
            ratio: Ratio::MEDIUM,
            attack: Milliseconds::new(10.0),
            release: Milliseconds::new(100.0),
            makeup: Decibels::new(0.0),
            mix: NormalizedValue::MAX,
            envelope: 0.0,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Calculate attack coefficient using type-safe Milliseconds.
    #[inline]
    fn attack_coeff(&self) -> f32 {
        let attack_secs = self.attack.to_seconds();
        (-1.0 / (attack_secs.as_f32() * self.sample_rate.as_f32())).exp()
    }

    /// Calculate release coefficient using type-safe Milliseconds.
    #[inline]
    fn release_coeff(&self) -> f32 {
        let release_secs = self.release.to_seconds();
        (-1.0 / (release_secs.as_f32() * self.sample_rate.as_f32())).exp()
    }

    /// Calculate gain reduction for a given input level.
    #[inline]
    fn compute_gain(&self, input_db: f32) -> f32 {
        let threshold = self.threshold.as_f32();
        if input_db > threshold {
            // Calculate gain reduction using type-safe Ratio
            let overshoot = input_db - threshold;
            let compressed = self.ratio.compress(overshoot);
            let gain_reduction = compressed - overshoot;
            Decibels::new(gain_reduction + self.makeup.as_f32()).to_linear()
        } else {
            self.makeup.to_linear()
        }
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Compressor {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("compressor", "Compressor")
            .description("Dynamics compressor with adjustable attack and release")
            .category(ModuleCategory::Effect)
            .tag("compressor")
            .tag("effect")
            .tag("dynamics")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .port(
                PortDescriptor::control_input("sidechain", "Sidechain")
                    .description("External sidechain input"),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Compressor(CompressorParam::Threshold(Decibels::new(-20.0))),
                    "Threshold",
                )
                .description("Compression threshold")
                .range(-60.0, 0.0)
                .default(-20.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Compressor(CompressorParam::Ratio(Ratio::MEDIUM)),
                    "Ratio",
                )
                .description("Compression ratio")
                .range(1.0, 20.0)
                .default(4.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Compressor(CompressorParam::Attack(Milliseconds::new(10.0))),
                    "Attack",
                )
                .description("Attack time")
                .range(0.1, 100.0)
                .default(10.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Compressor(CompressorParam::Release(Milliseconds::new(100.0))),
                    "Release",
                )
                .description("Release time")
                .range(10.0, 1000.0)
                .default(100.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Compressor(CompressorParam::Makeup(Decibels::new(0.0))),
                    "Makeup",
                )
                .description("Makeup gain")
                .range(0.0, 24.0)
                .default(0.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Compressor(CompressorParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix (parallel compression)")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Compressor {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;

        let attack_coeff = self.attack_coeff();
        let release_coeff = self.release_coeff();

        // Process stereo interleaved
        let channels = 2;
        for frame in 0..context.samples {
            let idx_l = frame * channels;
            let idx_r = frame * channels + 1;

            let in_l = if idx_l < input.len() { input[idx_l] } else { 0.0 };
            let in_r = if idx_r < input.len() {
                input[idx_r]
            } else {
                in_l
            };

            // Get peak level (stereo linked)
            let peak = in_l.abs().max(in_r.abs());
            let peak_db = Decibels::from_linear(peak).as_f32();

            // Envelope follower with attack/release
            let coeff = if peak_db > self.envelope {
                attack_coeff
            } else {
                release_coeff
            };
            self.envelope = coeff * self.envelope + (1.0 - coeff) * peak_db;

            // Calculate gain
            let gain = self.compute_gain(self.envelope);

            // Apply compression
            let wet_l = in_l * gain;
            let wet_r = in_r * gain;

            // Mix dry/wet (parallel compression)
            let mix = self.mix.as_f32();
            if idx_l < output.len() {
                output[idx_l] = in_l * (1.0 - mix) + wet_l * mix;
            }
            if idx_r < output.len() {
                output[idx_r] = in_r * (1.0 - mix) + wet_r * mix;
            }
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = NormalizedValue::new(mix);
    }

    fn get_mix(&self) -> f32 {
        self.mix.as_f32()
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Compressor(comp_param) = param {
            match comp_param {
                CompressorParam::Threshold(t) => {
                    self.threshold = Decibels::new(t.as_f32().clamp(-60.0, 0.0));
                }
                CompressorParam::Ratio(r) => {
                    self.ratio = Ratio::new(r.as_f32()).clamp_typical();
                }
                CompressorParam::Attack(a) => {
                    self.attack = Milliseconds::new(a.as_f32().clamp(0.1, 100.0));
                }
                CompressorParam::Release(r) => {
                    self.release = Milliseconds::new(r.as_f32().clamp(10.0, 1000.0));
                }
                CompressorParam::Makeup(m) => {
                    self.makeup = Decibels::new(m.as_f32().clamp(0.0, 24.0));
                }
                CompressorParam::Mix(m) => {
                    self.mix = m;
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Compressor(comp_param) = param {
            Some(match comp_param {
                CompressorParam::Threshold(_) => self.threshold.as_f32(),
                CompressorParam::Ratio(_) => self.ratio.as_f32(),
                CompressorParam::Attack(_) => self.attack.as_f32(),
                CompressorParam::Release(_) => self.release.as_f32(),
                CompressorParam::Makeup(_) => self.makeup.as_f32(),
                CompressorParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Compressor(CompressorParam::Threshold(self.threshold)),
            Param::Compressor(CompressorParam::Ratio(self.ratio)),
            Param::Compressor(CompressorParam::Attack(self.attack)),
            Param::Compressor(CompressorParam::Release(self.release)),
            Param::Compressor(CompressorParam::Makeup(self.makeup)),
            Param::Compressor(CompressorParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Compressor
    }
}
