//! Waveshaper effect module.
//!
//! Creative/experimental signal shaping with selectable curves:
//! - SoftClip: tanh saturation
//! - Asymmetric: different drive for positive/negative
//! - Fold: wavefolder (west-coast synthesis)
//! - Chebyshev: polynomial harmonic control
//! - SineFold: sin(x * drive * PI) for FM-like timbres
//! - Quantize: bit-reduction / lo-fi

use synth_core::{
    AudioEffect, BipolarValue, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ProcessContext, WidgetHint,
};
use synth_core::{ModuleType, NormalizedValue, Param, WaveshaperCurve, WaveshaperParam};

/// Waveshaper effect.
pub struct Waveshaper {
    curve: WaveshaperCurve,
    drive: NormalizedValue,
    mix: NormalizedValue,
    bias: BipolarValue,
    symmetry: BipolarValue,
}

impl Waveshaper {
    pub fn new() -> Self {
        Self {
            curve: WaveshaperCurve::SoftClip,
            drive: NormalizedValue::new(0.3),
            mix: NormalizedValue::MAX,
            bias: BipolarValue::CENTER,
            symmetry: BipolarValue::CENTER,
        }
    }

    /// Map normalized drive (0-1) to actual gain multiplier (1x-20x, exponential).
    #[inline]
    fn drive_gain(&self) -> f32 {
        crate::math::drive_gain(self.drive.as_f32(), 19.0)
    }

    /// Apply the selected waveshaping curve to a single sample.
    #[inline]
    fn shape(&self, input: f32) -> f32 {
        let gain = self.drive_gain();
        let bias = self.bias.as_f32();
        let sym = self.symmetry.as_f32();

        // Apply bias (DC offset before shaping)
        let biased = input + bias;

        // Apply drive
        let driven = biased * gain;

        match self.curve {
            WaveshaperCurve::SoftClip => crate::math::soft_clip(driven),

            WaveshaperCurve::Asymmetric => crate::math::asymmetric_soft_clip(driven, sym),

            WaveshaperCurve::Fold => crate::math::foldback(driven, 1.0),

            WaveshaperCurve::Chebyshev => {
                // Chebyshev polynomial T_n for harmonic generation
                // Use symmetry to blend between T2 (even harmonics) and T3 (odd harmonics)
                let x = driven.clamp(-1.0, 1.0);
                let t2 = crate::math::chebyshev_t2(x);
                let t3 = crate::math::chebyshev_t3(x);
                // Blend: sym=-1 -> pure T2, sym=0 -> equal, sym=+1 -> pure T3
                let blend = (sym + 1.0) * 0.5; // 0..1
                let chebyshev = t2 * (1.0 - blend) + t3 * blend;
                chebyshev.clamp(-1.0, 1.0)
            }

            WaveshaperCurve::SineFold => {
                // sin(x * drive * PI) - FM-like metallic character
                (driven * std::f32::consts::PI).sin()
            }

            WaveshaperCurve::Quantize => {
                // Quantize to N levels (drive controls resolution)
                // Map drive 0..1 to levels: low drive = many levels (subtle), high drive = few (harsh)
                let levels = (2.0 + (1.0 - self.drive.as_f32()) * 254.0).round();
                crate::math::quantize_signal(biased, levels)
            }
        }
    }
}

impl Default for Waveshaper {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Waveshaper {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("waveshaper", "Waveshaper")
            .description("Creative waveshaping effect with selectable curves")
            .category(ModuleCategory::Effect)
            .tag("waveshaper")
            .tag("effect")
            .tag("distortion")
            // No ports - effect chain modules are processed automatically
            .parameter(
                ParameterDescriptor::choice(
                    "curve",
                    Param::Waveshaper(WaveshaperParam::Curve(WaveshaperCurve::SoftClip)),
                    "Curve",
                    WaveshaperCurve::to_choices(),
                )
                .description("Waveshaping algorithm"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "drive",
                    Param::Waveshaper(WaveshaperParam::Drive(NormalizedValue::new(0.3))),
                    "Drive",
                )
                .description("Input gain before shaping (exponential 1x-20x)")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Waveshaper(WaveshaperParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "bias",
                    Param::Waveshaper(WaveshaperParam::Bias(BipolarValue::CENTER)),
                    "Bias",
                )
                .description("DC offset before shaping")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "symmetry",
                    Param::Waveshaper(WaveshaperParam::Symmetry(BipolarValue::CENTER)),
                    "Symmetry",
                )
                .description("Asymmetric distortion control")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Waveshaper {
    fn process(&mut self, input: &[f32], output: &mut [f32], _context: &ProcessContext) {
        for i in 0..input.len().min(output.len()) {
            let dry = input[i];
            let shaped = self.shape(dry);
            output[i] = self.mix.blend(dry, shaped);
        }
    }

    fn reset(&mut self) {
        // Waveshaper is stateless (no filter state to reset)
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Waveshaper(ws_param) = param {
            match ws_param {
                WaveshaperParam::Curve(c) => self.curve = c,
                WaveshaperParam::Drive(d) => self.drive = d,
                WaveshaperParam::Mix(m) => self.mix = m,
                WaveshaperParam::Bias(b) => self.bias = b,
                WaveshaperParam::Symmetry(s) => self.symmetry = s,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Waveshaper(ws_param) = param {
            Some(match ws_param {
                WaveshaperParam::Curve(_) => self.curve.index() as f32,
                WaveshaperParam::Drive(_) => self.drive.as_f32(),
                WaveshaperParam::Mix(_) => self.mix.as_f32(),
                WaveshaperParam::Bias(_) => self.bias.as_f32(),
                WaveshaperParam::Symmetry(_) => self.symmetry.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Waveshaper(WaveshaperParam::Curve(self.curve)),
            Param::Waveshaper(WaveshaperParam::Drive(self.drive)),
            Param::Waveshaper(WaveshaperParam::Mix(self.mix)),
            Param::Waveshaper(WaveshaperParam::Bias(self.bias)),
            Param::Waveshaper(WaveshaperParam::Symmetry(self.symmetry)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Waveshaper
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;
    use synth_core::SampleRate;

    #[test]
    fn test_waveshaper_all_curves_finite() {
        let mut ws = Waveshaper::new();

        for curve in WaveshaperCurve::ALL {
            ws.curve = curve;
            let input = [0.5f32; 100];
            let mut output = [0.0f32; 100];

            let context = ProcessContext {
                sample_rate: SampleRate::DVD_QUALITY,
                samples: SampleCount::new(100),
                ..Default::default()
            };

            ws.process(&input, &mut output, &context);

            for &sample in &output {
                assert!(sample.is_finite(), "Output not finite for {:?}", curve);
                assert!(sample.abs() <= 2.0, "Output too large for {:?}", curve);
            }
        }
    }

    #[test]
    fn test_waveshaper_silence_passthrough() {
        let mut ws = Waveshaper::new();
        let input = [0.0f32; 64];
        let mut output = [0.0f32; 64];

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(64),
            ..Default::default()
        };

        // With bias=0 and any curve, silence should produce near-silence
        ws.bias = BipolarValue::CENTER;
        ws.process(&input, &mut output, &context);

        for &sample in &output {
            assert!(
                sample.abs() < 0.01,
                "Silence should produce near-silence, got {}",
                sample
            );
        }
    }

    #[test]
    fn test_waveshaper_dry_wet_mix() {
        let mut ws = Waveshaper::new();
        ws.mix = NormalizedValue::new(0.0); // Fully dry
        ws.drive = NormalizedValue::new(0.8);

        let input = [0.5f32; 64];
        let mut output = [0.0f32; 64];

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(64),
            ..Default::default()
        };

        ws.process(&input, &mut output, &context);

        // Fully dry should pass through unchanged
        for &sample in &output {
            assert!(
                (sample - 0.5).abs() < 0.001,
                "Dry signal should pass through unchanged"
            );
        }
    }
}
