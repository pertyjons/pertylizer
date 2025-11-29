//! Dynamics compressor with adjustable threshold, ratio, attack, and release.

use crate::engine::typed_params::{CompressorParam, ModuleType, TypedParam, TypedValue};
use crate::modules::{
    Describable, EffectModule, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PortDescriptor, ProcessContext, WidgetHint,
};
use crate::types::{Decibels, NormalizedValue, SampleRate};

/// Compressor effect with envelope follower.
pub struct Compressor {
    // Parameters
    threshold: Decibels,
    ratio: f32,         // Compression ratio (1:1 to 20:1) - kept as f32
    attack_ms: f32,     // Attack time in ms - kept as f32
    release_ms: f32,    // Release time in ms - kept as f32
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
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup: Decibels::new(0.0),
            mix: NormalizedValue::MAX,
            envelope: 0.0,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Calculate attack coefficient.
    #[inline]
    fn attack_coeff(&self) -> f32 {
        (-1.0 / (self.attack_ms * 0.001 * self.sample_rate.as_f32())).exp()
    }

    /// Calculate release coefficient.
    #[inline]
    fn release_coeff(&self) -> f32 {
        (-1.0 / (self.release_ms * 0.001 * self.sample_rate.as_f32())).exp()
    }

    /// Calculate gain reduction for a given input level.
    #[inline]
    fn compute_gain(&self, input_db: f32) -> f32 {
        let threshold = self.threshold.as_f32();
        if input_db > threshold {
            // Calculate gain reduction
            let overshoot = input_db - threshold;
            let compressed = overshoot / self.ratio;
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
                    TypedParam::Compressor(CompressorParam::Threshold),
                    "Threshold",
                )
                .description("Compression threshold")
                .range(-60.0, 0.0)
                .default(-20.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Compressor(CompressorParam::Ratio), "Ratio")
                    .description("Compression ratio")
                    .range(1.0, 20.0)
                    .default(4.0)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    TypedParam::Compressor(CompressorParam::Attack),
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
                    TypedParam::Compressor(CompressorParam::Release),
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
                    TypedParam::Compressor(CompressorParam::Makeup),
                    "Makeup",
                )
                .description("Makeup gain")
                .range(0.0, 24.0)
                .default(0.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Compressor(CompressorParam::Mix), "Mix")
                    .description("Dry/wet mix (parallel compression)")
                    .range(0.0, 1.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
    }
}

impl EffectModule for Compressor {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = SampleRate::new(context.sample_rate);

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

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Compressor(comp_param) = param {
            match comp_param {
                CompressorParam::Threshold => {
                    if let Some(t) = value.as_float() {
                        self.threshold = Decibels::new(t.clamp(-60.0, 0.0));
                    }
                }
                CompressorParam::Ratio => {
                    if let Some(r) = value.as_float() {
                        self.ratio = r.clamp(1.0, 20.0);
                    }
                }
                CompressorParam::Attack => {
                    if let Some(a) = value.as_float() {
                        self.attack_ms = a.clamp(0.1, 100.0);
                    }
                }
                CompressorParam::Release => {
                    if let Some(r) = value.as_float() {
                        self.release_ms = r.clamp(10.0, 1000.0);
                    }
                }
                CompressorParam::Makeup => {
                    if let Some(m) = value.as_float() {
                        self.makeup = Decibels::new(m.clamp(0.0, 24.0));
                    }
                }
                CompressorParam::Mix => {
                    if let Some(m) = value.as_float() {
                        self.mix = NormalizedValue::new(m);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Compressor(comp_param) = param {
            match comp_param {
                CompressorParam::Threshold => Some(TypedValue::Float(self.threshold.as_f32())),
                CompressorParam::Ratio => Some(TypedValue::Float(self.ratio)),
                CompressorParam::Attack => Some(TypedValue::Float(self.attack_ms)),
                CompressorParam::Release => Some(TypedValue::Float(self.release_ms)),
                CompressorParam::Makeup => Some(TypedValue::Float(self.makeup.as_f32())),
                CompressorParam::Mix => Some(TypedValue::Float(self.mix.as_f32())),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Compressor
    }
}
