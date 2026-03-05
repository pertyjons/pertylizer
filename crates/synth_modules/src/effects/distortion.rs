//! Distortion effect module.
//!
//! Features:
//! - Multiple distortion types (soft clip, hard clip, foldback, bitcrush)
//! - Drive control
//! - Tone filter
//! - Mix control

use synth_core::{
    AudioEffect, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ProcessContext, WidgetHint,
};
use synth_core::{DistortionMode, DistortionParam, ModuleType, Param};
use synth_core::{FilterState, Hertz, NormalizedValue, SampleRate};

// Type alias for local use
pub type DistortionType = DistortionMode;

/// Distortion effect.
pub struct Distortion {
    // Parameters
    dist_type: DistortionType,
    drive: NormalizedValue,
    tone: NormalizedValue,
    mix: NormalizedValue,
    bit_depth: f32, // For bitcrush (1-16) - not a normalized value

    // Filter state
    filter_state: FilterState,

    // State
    sample_rate: SampleRate,
}

impl Distortion {
    pub fn new() -> Self {
        Self {
            dist_type: DistortionType::SoftClip,
            drive: NormalizedValue::CENTER,
            tone: NormalizedValue::new(0.8),
            mix: NormalizedValue::MAX,
            bit_depth: 8.0,
            filter_state: FilterState::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Apply distortion to a sample.
    #[inline]
    fn distort(&self, input: f32) -> f32 {
        // Apply drive (exponential curve for more usable range)
        let drive = self.drive.as_f32();
        let gain = 1.0 + drive * drive * 50.0;
        let driven = input * gain;

        match self.dist_type {
            DistortionType::SoftClip => {
                // Tanh soft clipping
                driven.tanh()
            }

            DistortionType::HardClip => {
                // Simple hard clipping
                driven.clamp(-1.0, 1.0)
            }

            DistortionType::Foldback => {
                // Foldback distortion with iteration limit for real-time safety
                let threshold = 1.0;
                let mut x = driven;
                for _ in 0..16 {
                    if x > threshold {
                        x = threshold - (x - threshold);
                    } else if x < -threshold {
                        x = -threshold - (x + threshold);
                    } else {
                        break;
                    }
                }
                x.clamp(-threshold, threshold)
            }

            DistortionType::Bitcrush => {
                // Bit reduction
                let bits = self.bit_depth.max(1.0);
                let levels = 2.0f32.powf(bits);
                let quantized = (driven * levels).round() / levels;
                quantized.clamp(-1.0, 1.0)
            }

            DistortionType::Tube => {
                // Tube-like asymmetric soft clipping
                let x = driven;
                if x >= 0.0 {
                    1.0 - (-x).exp()
                } else {
                    -1.0 + x.exp()
                }
            }
        }
    }

    /// Apply tone filter (one-pole lowpass).
    #[inline]
    fn apply_tone(&mut self, input: f32) -> f32 {
        // Map tone parameter to cutoff frequency
        let tone = self.tone.as_f32();
        let cutoff = Hertz::new(200.0 + tone * tone * 15000.0);
        let coef = cutoff.to_exp_coeff(self.sample_rate);
        self.filter_state =
            FilterState::new(input * (1.0 - coef) + self.filter_state.as_f32() * coef);
        self.filter_state.as_f32()
    }
}

impl Default for Distortion {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Distortion {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("distortion", "Distortion")
            .description("Multi-mode distortion effect")
            .category(ModuleCategory::Effect)
            .tag("distortion")
            .tag("effect")
            .tag("overdrive")
            // No ports - effect chain modules are processed automatically
            .parameter(
                ParameterDescriptor::choice(
                    Param::Distortion(DistortionParam::Mode(DistortionMode::SoftClip)),
                    "Type",
                    DistortionMode::to_choices(),
                )
                .description("Distortion algorithm"),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Distortion(DistortionParam::Drive(NormalizedValue::CENTER)),
                    "Drive",
                )
                .description("Distortion amount")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Distortion(DistortionParam::Tone(NormalizedValue::new(0.8))),
                    "Tone",
                )
                .description("Post-distortion filter")
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Distortion(DistortionParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Distortion(DistortionParam::BitDepth(NormalizedValue::CENTER)),
                    "Bit Depth",
                )
                .description("Bit depth for bitcrush mode (1-16 bits)")
                .range(1.0, 16.0)
                .default(8.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Distortion {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        for i in 0..input.len().min(output.len()) {
            let dry = input[i];
            let distorted = self.distort(dry);
            let filtered = self.apply_tone(distorted);
            output[i] = self.mix.blend(dry, filtered);
        }
    }

    fn reset(&mut self) {
        self.filter_state = FilterState::ZERO;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Distortion(dist_param) = param {
            match dist_param {
                DistortionParam::Mode(mode) => {
                    self.dist_type = mode;
                }
                DistortionParam::Drive(d) => {
                    self.drive = d;
                }
                DistortionParam::Tone(t) => {
                    self.tone = t;
                }
                DistortionParam::Mix(m) => {
                    self.mix = m;
                }
                DistortionParam::BitDepth(b) => {
                    // Convert normalized 0-1 to bit depth 1-16
                    self.bit_depth = 1.0 + b.as_f32() * 15.0;
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Distortion(dist_param) = param {
            Some(match dist_param {
                DistortionParam::Mode(_) => self.dist_type.index() as f32,
                DistortionParam::Drive(_) => self.drive.as_f32(),
                DistortionParam::Tone(_) => self.tone.as_f32(),
                DistortionParam::Mix(_) => self.mix.as_f32(),
                // Return actual bit depth value (1-16), not normalized
                DistortionParam::BitDepth(_) => self.bit_depth,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        // Convert bit_depth (1-16) to normalized (0-1)
        let bit_depth_normalized = NormalizedValue::new((self.bit_depth - 1.0) / 15.0);
        vec![
            Param::Distortion(DistortionParam::Mode(self.dist_type)),
            Param::Distortion(DistortionParam::Drive(self.drive)),
            Param::Distortion(DistortionParam::Tone(self.tone)),
            Param::Distortion(DistortionParam::Mix(self.mix)),
            Param::Distortion(DistortionParam::BitDepth(bit_depth_normalized)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Distortion
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    #[test]
    fn test_distortion_types() {
        let mut dist = Distortion::new();
        dist.sample_rate = SampleRate::new(48000.0);

        for dt in DistortionType::ALL {
            dist.dist_type = dt;
            let input = [0.5f32; 100];
            let mut output = [0.0f32; 100];

            let context = ProcessContext {
                sample_rate: SampleRate::DVD_QUALITY,
                samples: SampleCount::new(100),
                ..Default::default()
            };

            dist.process(&input, &mut output, &context);

            for &sample in &output {
                assert!(sample.is_finite(), "Output not finite for {:?}", dt);
                assert!(sample.abs() <= 2.0, "Output too large for {:?}", dt);
            }
        }
    }
}
