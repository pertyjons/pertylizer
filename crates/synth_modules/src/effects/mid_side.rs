//! Mid-Side processing effect module.
//!
//! Features:
//! - Encode stereo to mid/side representation
//! - Independent mid and side gain controls
//! - Stereo width control (0=mono, 1=normal, 2=extra wide)
//! - Dry/wet mix

use synth_core::{
    AudioEffect, BipolarValue, Decibels, Describable, MidSideParam, ModuleCategory,
    ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor, ParameterUnit,
    ProcessContext, StereoSample, WidgetHint,
};

/// Mid-Side stereo processing effect.
///
/// Encodes stereo audio into mid (sum) and side (difference) components,
/// applies independent gain and width control, then decodes back to stereo.
pub struct MidSide {
    /// Stereo width: NormalizedValue 0.0-1.0 maps to actual width 0.0-2.0.
    width: NormalizedValue,
    /// Mid channel gain in dB.
    mid_gain: Decibels,
    /// Side channel gain in dB.
    side_gain: Decibels,
    /// Stereo field rotation (-1.0 to 1.0 maps to -180° to +180°).
    rotation: BipolarValue,
    /// Dry/wet mix.
    mix: NormalizedValue,
}

impl MidSide {
    /// Default width (0.5 normalized = 1.0 actual = normal stereo).
    const DEFAULT_WIDTH: NormalizedValue = NormalizedValue::CENTER;

    #[must_use]
    pub fn new() -> Self {
        Self {
            width: Self::DEFAULT_WIDTH,
            mid_gain: Decibels::UNITY,
            side_gain: Decibels::UNITY,
            rotation: BipolarValue::CENTER,
            mix: NormalizedValue::MAX,
        }
    }

    /// Map normalized width (0.0-1.0) to actual width (0.0-2.0).
    #[inline]
    #[must_use]
    fn actual_width(&self) -> f32 {
        self.width.as_f32() * 2.0
    }
}

impl Default for MidSide {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for MidSide {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("mid_side", "Mid/Side")
            .description("Mid-Side stereo processing with width and gain controls")
            .category(ModuleCategory::Effect)
            .tag("mid_side")
            .tag("effect")
            .tag("stereo")
            // No ports - effect chain modules are processed automatically
            .parameter(
                ParameterDescriptor::float(
                    "width",
                    Param::MidSide(MidSideParam::Width(Self::DEFAULT_WIDTH)),
                    "Width",
                )
                .description("Stereo width (0=mono, center=normal, max=extra wide)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mid_gain",
                    Param::MidSide(MidSideParam::MidGain(Decibels::UNITY)),
                    "Mid Gain",
                )
                .description("Mid channel gain")
                .range(-12.0, 12.0)
                .default(0.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "side_gain",
                    Param::MidSide(MidSideParam::SideGain(Decibels::UNITY)),
                    "Side Gain",
                )
                .description("Side channel gain")
                .range(-12.0, 12.0)
                .default(0.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "rotation",
                    Param::MidSide(MidSideParam::Rotation(BipolarValue::CENTER)),
                    "Rotation",
                )
                .description("Stereo field rotation (-180° to +180°)")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::MidSide(MidSideParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for MidSide {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        let mix = self.mix.as_f32();
        let width = self.actual_width();
        let mid_gain_linear = self.mid_gain.to_linear();
        let side_gain_linear = self.side_gain.to_linear();

        // Stereo rotation: 2D rotation matrix
        // Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Effects/255-stereo-field-rotation-via-transformation-matrix.rst
        let rotation_angle = self.rotation.as_f32() * std::f32::consts::PI; // -π to +π
        let cos_r = rotation_angle.cos();
        let sin_r = rotation_angle.sin();

        for frame in 0..context.samples.as_usize() {
            // Read input as stereo sample
            let dry = StereoSample::read_frame(input, frame);

            // Encode to mid/side
            let (mid, side) = crate::math::mid_side_encode(dry.left, dry.right);

            // Apply mid and side gains
            let mid = mid * mid_gain_linear;
            let side = side * side_gain_linear;

            // Apply width and decode back to stereo
            let (wet_l, wet_r) = crate::math::mid_side_decode(mid, side, width);

            // Apply stereo rotation (2D rotation matrix)
            let rot_l = wet_l * cos_r - wet_r * sin_r;
            let rot_r = wet_l * sin_r + wet_r * cos_r;
            let wet = StereoSample::new(rot_l, rot_r);

            // Mix dry/wet
            let result = dry.blend(wet, mix);

            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        // Mid-Side processing is stateless (no buffers to clear)
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::MidSide(ms_param) = param {
            match ms_param {
                MidSideParam::Width(w) => self.width = w,
                MidSideParam::MidGain(g) => {
                    self.mid_gain = Decibels::new(g.as_f32().clamp(-12.0, 12.0));
                }
                MidSideParam::SideGain(g) => {
                    self.side_gain = Decibels::new(g.as_f32().clamp(-12.0, 12.0));
                }
                MidSideParam::Rotation(r) => self.rotation = r,
                MidSideParam::Mix(m) => self.mix = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::MidSide(ms_param) = param {
            Some(match ms_param {
                MidSideParam::Width(_) => self.width.as_f32(),
                MidSideParam::MidGain(_) => self.mid_gain.as_f32(),
                MidSideParam::SideGain(_) => self.side_gain.as_f32(),
                MidSideParam::Rotation(_) => self.rotation.as_f32(),
                MidSideParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::MidSide(MidSideParam::Width(self.width)),
            Param::MidSide(MidSideParam::MidGain(self.mid_gain)),
            Param::MidSide(MidSideParam::SideGain(self.side_gain)),
            Param::MidSide(MidSideParam::Rotation(self.rotation)),
            Param::MidSide(MidSideParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::MidSide
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::{SampleCount, SampleRate};

    fn make_context(samples: usize) -> ProcessContext {
        ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(samples),
            ..Default::default()
        }
    }

    #[test]
    fn test_mid_side_creation() {
        let ms = MidSide::new();
        assert!((ms.width.as_f32() - 0.5).abs() < 0.001);
        assert!((ms.mid_gain.as_f32()).abs() < 0.001);
        assert!((ms.side_gain.as_f32()).abs() < 0.001);
        assert!((ms.mix.as_f32() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_mono_collapse() {
        // Width = 0.0 (normalized 0.0 -> actual 0.0) should collapse to mono
        let mut ms = MidSide::new();
        ms.width = NormalizedValue::new(0.0);

        let num_frames = 64;
        // Stereo signal: left=0.8, right=0.2
        let mut input = vec![0.0f32; num_frames * 2];
        for i in 0..num_frames {
            input[i * 2] = 0.8;
            input[i * 2 + 1] = 0.2;
        }
        let mut output = vec![0.0f32; num_frames * 2];

        ms.process(&input, &mut output, &make_context(num_frames));

        // With width=0, output should be mono (L=R=mid)
        for i in 0..num_frames {
            let left = output[i * 2];
            let right = output[i * 2 + 1];
            assert!(
                (left - right).abs() < 0.001,
                "Width 0 should produce mono: L={left}, R={right}"
            );
            // mid = (0.8 + 0.2) * 0.5 = 0.5
            assert!(
                (left - 0.5).abs() < 0.001,
                "Mono signal should be the mid component: {left}"
            );
        }
    }

    #[test]
    fn test_normal_stereo_passthrough() {
        // Width = 0.5 (normalized) -> actual 1.0 = normal stereo
        let mut ms = MidSide::new();
        // Default width is 0.5 (center), gains are 0 dB, mix is 1.0

        let num_frames = 64;
        let mut input = vec![0.0f32; num_frames * 2];
        for i in 0..num_frames {
            input[i * 2] = 0.7;
            input[i * 2 + 1] = 0.3;
        }
        let mut output = vec![0.0f32; num_frames * 2];

        ms.process(&input, &mut output, &make_context(num_frames));

        // With width=1.0 and 0dB gains, output should equal input
        for i in 0..num_frames {
            assert!(
                (output[i * 2] - 0.7).abs() < 0.001,
                "Normal width should pass through L: {}",
                output[i * 2]
            );
            assert!(
                (output[i * 2 + 1] - 0.3).abs() < 0.001,
                "Normal width should pass through R: {}",
                output[i * 2 + 1]
            );
        }
    }

    #[test]
    fn test_extra_wide() {
        // Width = 1.0 (normalized) -> actual 2.0 = extra wide
        let mut ms = MidSide::new();
        ms.width = NormalizedValue::MAX;

        let num_frames = 64;
        let mut input = vec![0.0f32; num_frames * 2];
        for i in 0..num_frames {
            input[i * 2] = 0.8;
            input[i * 2 + 1] = 0.2;
        }
        let mut output = vec![0.0f32; num_frames * 2];

        ms.process(&input, &mut output, &make_context(num_frames));

        // mid = 0.5, side = 0.3, width = 2.0
        // L = 0.5 + 0.3*2.0 = 1.1
        // R = 0.5 - 0.3*2.0 = -0.1
        for i in 0..num_frames {
            assert!(
                (output[i * 2] - 1.1).abs() < 0.001,
                "Extra wide L: {}",
                output[i * 2]
            );
            assert!(
                (output[i * 2 + 1] - (-0.1)).abs() < 0.001,
                "Extra wide R: {}",
                output[i * 2 + 1]
            );
        }
    }

    #[test]
    fn test_dry_wet_mix() {
        let mut ms = MidSide::new();
        ms.mix = NormalizedValue::new(0.0); // Fully dry
        ms.width = NormalizedValue::new(0.0); // Would collapse to mono if wet

        let num_frames = 32;
        let mut input = vec![0.0f32; num_frames * 2];
        for i in 0..num_frames {
            input[i * 2] = 0.9;
            input[i * 2 + 1] = 0.1;
        }
        let mut output = vec![0.0f32; num_frames * 2];

        ms.process(&input, &mut output, &make_context(num_frames));

        // Fully dry should pass through unchanged
        for i in 0..num_frames {
            assert!(
                (output[i * 2] - 0.9).abs() < 0.001,
                "Dry signal should pass through L"
            );
            assert!(
                (output[i * 2 + 1] - 0.1).abs() < 0.001,
                "Dry signal should pass through R"
            );
        }
    }

    #[test]
    fn test_mid_gain_boost() {
        let mut ms = MidSide::new();
        ms.mid_gain = Decibels::new(6.0); // ~2x boost to mid

        let num_frames = 32;
        // Mono signal (L=R) so side=0
        let input = vec![0.5f32; num_frames * 2];
        let mut output = vec![0.0f32; num_frames * 2];

        ms.process(&input, &mut output, &make_context(num_frames));

        // mid = 0.5, side = 0, mid_gain ~2x -> output ~1.0
        let expected = 0.5 * Decibels::new(6.0).to_linear();
        for i in 0..num_frames {
            assert!(
                (output[i * 2] - expected).abs() < 0.01,
                "Mid gain boost L: {} expected {}",
                output[i * 2],
                expected
            );
        }
    }

    #[test]
    fn test_silence_passthrough() {
        let mut ms = MidSide::new();
        let num_frames = 64;
        let input = vec![0.0f32; num_frames * 2];
        let mut output = vec![0.0f32; num_frames * 2];

        ms.process(&input, &mut output, &make_context(num_frames));

        for &sample in &output {
            assert!(
                sample.abs() < 0.001,
                "Silence should produce silence, got {sample}"
            );
        }
    }

    #[test]
    fn test_output_is_finite() {
        let mut ms = MidSide::new();
        ms.mid_gain = Decibels::new(12.0);
        ms.side_gain = Decibels::new(12.0);
        ms.width = NormalizedValue::MAX;

        let num_frames = 256;
        let mut input = vec![0.0f32; num_frames * 2];
        for i in 0..num_frames {
            let t = i as f32 / num_frames as f32;
            input[i * 2] = (t * std::f32::consts::TAU * 5.0).sin();
            input[i * 2 + 1] = (t * std::f32::consts::TAU * 5.0 + 1.0).sin();
        }
        let mut output = vec![0.0f32; num_frames * 2];

        ms.process(&input, &mut output, &make_context(num_frames));

        for &sample in &output {
            assert!(sample.is_finite(), "Output must be finite");
            assert!(sample.abs() < 100.0, "Output should not explode");
        }
    }

    #[test]
    fn test_set_get_params() {
        let mut ms = MidSide::new();

        ms.set_param(Param::MidSide(MidSideParam::Width(NormalizedValue::new(
            0.75,
        ))));
        ms.set_param(Param::MidSide(MidSideParam::MidGain(Decibels::new(3.0))));
        ms.set_param(Param::MidSide(MidSideParam::SideGain(Decibels::new(-6.0))));
        ms.set_param(Param::MidSide(MidSideParam::Mix(NormalizedValue::new(0.8))));

        let width = ms.get_param(&Param::MidSide(MidSideParam::Width(NormalizedValue::MIN)));
        assert!((width.unwrap_or(0.0) - 0.75).abs() < 0.001);

        let mid_g = ms.get_param(&Param::MidSide(MidSideParam::MidGain(Decibels::UNITY)));
        assert!((mid_g.unwrap_or(0.0) - 3.0).abs() < 0.001);

        let side_g = ms.get_param(&Param::MidSide(MidSideParam::SideGain(Decibels::UNITY)));
        assert!((side_g.unwrap_or(0.0) - (-6.0)).abs() < 0.001);

        let mix = ms.get_param(&Param::MidSide(MidSideParam::Mix(NormalizedValue::MIN)));
        assert!((mix.unwrap_or(0.0) - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_gain_clamping() {
        let mut ms = MidSide::new();

        // Setting gain beyond range should clamp
        ms.set_param(Param::MidSide(MidSideParam::MidGain(Decibels::new(20.0))));
        assert!((ms.mid_gain.as_f32() - 12.0).abs() < 0.001);

        ms.set_param(Param::MidSide(MidSideParam::SideGain(Decibels::new(-20.0))));
        assert!((ms.side_gain.as_f32() - (-12.0)).abs() < 0.001);
    }
}
