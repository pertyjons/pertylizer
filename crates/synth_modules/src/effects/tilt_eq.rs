//! Tilt EQ effect.
//!
//! Single-knob equalizer that tilts the frequency response around a center point.
//! Positive = brighter, negative = darker.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/267-simple-tilt-equalizer.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use synth_core::{
    AudioEffect, BipolarValue, Describable, Hertz, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, PortDescriptor, ProcessContext, ResponseCurve,
    SampleCount, SampleRate, StereoSample, TiltEqParam, WidgetHint,
};

/// Tilt EQ — single-knob spectral balance control.
pub struct TiltEq {
    tilt: BipolarValue,
    center_freq: Hertz,
    mix: NormalizedValue,
    sample_rate: SampleRate,
    /// One-pole filter states (L/R)
    lp_state_l: f32,
    lp_state_r: f32,
}

impl TiltEq {
    pub fn new() -> Self {
        Self {
            tilt: BipolarValue::CENTER,
            center_freq: Hertz::new(800.0),
            mix: NormalizedValue::MAX,
            sample_rate: SampleRate::DVD_QUALITY,
            lp_state_l: 0.0,
            lp_state_r: 0.0,
        }
    }
}

impl Default for TiltEq {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for TiltEq {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("tilt_eq", "Tilt EQ")
            .width(synth_core::ModuleWidth::Small)
            .description("Single-knob spectral tilt equalizer")
            .category(ModuleCategory::Effect)
            .tag("eq")
            .tag("effect")
            .tag("tilt")
            .port(PortDescriptor::audio_input("in_l", "In L"))
            .port(PortDescriptor::audio_input("in_r", "In R"))
            .port(PortDescriptor::audio_output("out_l", "Out L"))
            .port(PortDescriptor::audio_output("out_r", "Out R"))
            .parameter(
                ParameterDescriptor::float(
                    "tilt",
                    Param::TiltEq(TiltEqParam::Tilt(BipolarValue::CENTER)),
                    "Tilt",
                )
                .description("Spectral tilt (-1=dark, 0=flat, +1=bright)")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "center_freq",
                    Param::TiltEq(TiltEqParam::CenterFreq(Hertz::new(800.0))),
                    "Center",
                )
                .description("Pivot frequency")
                .range(100.0, 8000.0)
                .default(800.0)
                .widget(WidgetHint::Knob)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::TiltEq(TiltEqParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for TiltEq {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        self.sample_rate = context.sample_rate;

        // One-pole lowpass coefficient from center frequency
        let sr = self.sample_rate.as_f32();
        let omega = (self.center_freq.as_f32() * std::f32::consts::TAU / sr).min(0.99);
        let lp_coeff = omega / (1.0 + omega);

        // Tilt gain: positive boosts high, cuts low; negative does the opposite
        let tilt = self.tilt.as_f32();
        let gain_low = 1.0 - tilt.max(0.0); // Cut lows when tilt > 0
        let gain_high = 1.0 + tilt.max(0.0); // Boost highs when tilt > 0
        let gain_low = gain_low + tilt.min(0.0).abs(); // Boost lows when tilt < 0
        let gain_high = gain_high - tilt.min(0.0).abs(); // Cut highs when tilt < 0

        let mix = self.mix.as_f32();

        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);

            // One-pole LP filter
            self.lp_state_l += lp_coeff * (dry.left - self.lp_state_l);
            self.lp_state_r += lp_coeff * (dry.right - self.lp_state_r);

            // Split into low and high
            let low_l = self.lp_state_l;
            let high_l = dry.left - low_l;
            let low_r = self.lp_state_r;
            let high_r = dry.right - low_r;

            // Apply tilt gains
            let wet = StereoSample::new(
                low_l * gain_low + high_l * gain_high,
                low_r * gain_low + high_r * gain_high,
            );

            let result = dry.blend(wet, mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::TiltEq(p) = param {
            match p {
                TiltEqParam::Tilt(t) => self.tilt = t,
                TiltEqParam::CenterFreq(f) => {
                    self.center_freq = Hertz::new(f.as_f32().clamp(100.0, 8000.0));
                }
                TiltEqParam::Mix(m) => self.mix = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::TiltEq(p) = param {
            Some(match p {
                TiltEqParam::Tilt(_) => self.tilt.as_f32(),
                TiltEqParam::CenterFreq(_) => self.center_freq.as_f32(),
                TiltEqParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::TiltEq(TiltEqParam::Tilt(self.tilt)),
            Param::TiltEq(TiltEqParam::CenterFreq(self.center_freq)),
            Param::TiltEq(TiltEqParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::TiltEq
    }

    fn reset(&mut self) {
        self.lp_state_l = 0.0;
        self.lp_state_r = 0.0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        SampleCount::ZERO
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }
}
