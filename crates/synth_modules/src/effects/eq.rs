//! 3-band parametric equalizer with low shelf, mid peak, and high shelf.

use synth_core::{
    AudioEffect, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor, ParameterUnit,
    PortDescriptor, ProcessContext, StereoSample, WidgetHint,
};
use synth_core::{Decibels, Hertz, NormalizedValue, SampleRate};
use synth_core::{EqParam, ModuleType, Param};

/// Biquad filter coefficients (local to EQ, uses Direct Form I for stereo state).
#[derive(Clone, Copy, Default)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    /// Create from raw unnormalized coefficients, dividing all by a0.
    fn from_raw(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        let a0 = if a0.abs() < 1e-6 { 1e-6 } else { a0 };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

/// Biquad filter state (stereo).
#[derive(Clone, Copy, Default)]
struct BiquadState {
    x1_l: f32,
    x2_l: f32,
    y1_l: f32,
    y2_l: f32,
    x1_r: f32,
    x2_r: f32,
    y1_r: f32,
    y2_r: f32,
}

impl BiquadState {
    /// Process a stereo sample through the filter.
    #[inline]
    fn process(&mut self, in_l: f32, in_r: f32, c: &BiquadCoeffs) -> (f32, f32) {
        let out_l =
            c.b0 * in_l + c.b1 * self.x1_l + c.b2 * self.x2_l - c.a1 * self.y1_l - c.a2 * self.y2_l;
        let out_r =
            c.b0 * in_r + c.b1 * self.x1_r + c.b2 * self.x2_r - c.a1 * self.y1_r - c.a2 * self.y2_r;

        self.x2_l = self.x1_l;
        self.x1_l = in_l;
        self.y2_l = self.y1_l;
        self.y1_l = out_l;

        self.x2_r = self.x1_r;
        self.x1_r = in_r;
        self.y2_r = self.y1_r;
        self.y1_r = out_r;

        (out_l, out_r)
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// 3-band parametric EQ with low shelf, mid peak, and high shelf.
pub struct Eq {
    // Parameters
    low_freq: Hertz,
    low_gain: Decibels,
    mid_freq: Hertz,
    mid_gain: Decibels,
    mid_q: f32, // Q factor - dimensionless, kept as f32
    high_freq: Hertz,
    high_gain: Decibels,
    mix: NormalizedValue,

    // Filter state
    low_state: BiquadState,
    mid_state: BiquadState,
    high_state: BiquadState,

    // Cached coefficients
    low_coeffs: BiquadCoeffs,
    mid_coeffs: BiquadCoeffs,
    high_coeffs: BiquadCoeffs,

    // State
    sample_rate: SampleRate,
    coeffs_dirty: bool,
}

impl Eq {
    pub fn new() -> Self {
        let mut eq = Self {
            low_freq: Hertz::new(200.0),
            low_gain: Decibels::new(0.0),
            mid_freq: Hertz::new(1000.0),
            mid_gain: Decibels::new(0.0),
            mid_q: 1.0,
            high_freq: Hertz::new(4000.0),
            high_gain: Decibels::new(0.0),
            mix: NormalizedValue::MAX,
            low_state: BiquadState::default(),
            mid_state: BiquadState::default(),
            high_state: BiquadState::default(),
            low_coeffs: BiquadCoeffs::default(),
            mid_coeffs: BiquadCoeffs::default(),
            high_coeffs: BiquadCoeffs::default(),
            sample_rate: SampleRate::DVD_QUALITY,
            coeffs_dirty: true,
        };
        eq.update_coefficients();
        eq
    }

    /// Calculate low shelf filter coefficients.
    fn calc_low_shelf(&mut self) {
        let a = crate::math::db_to_eq_amplitude(self.low_gain.as_f32());
        let omega = crate::math::biquad_omega(self.low_freq, self.sample_rate);
        let cos_w0 = omega.cos_w0;
        let sin_w0 = omega.sin_w0;
        let shelf_slope = 0.9_f32;
        let alpha = crate::math::biquad_alpha_shelf(sin_w0, a, shelf_slope);
        let sqrt_a = a.sqrt();

        self.low_coeffs = BiquadCoeffs::from_raw(
            a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
            a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha),
            (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
            (a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha,
        );
    }

    /// Calculate peaking EQ filter coefficients.
    fn calc_mid_peak(&mut self) {
        let a = crate::math::db_to_eq_amplitude(self.mid_gain.as_f32());
        let omega = crate::math::biquad_omega(self.mid_freq, self.sample_rate);
        let cos_w0 = omega.cos_w0;
        let sin_w0 = omega.sin_w0;
        let alpha = crate::math::biquad_alpha_from_q(sin_w0, self.mid_q);

        self.mid_coeffs = BiquadCoeffs::from_raw(
            1.0 + alpha * a,
            -2.0 * cos_w0,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos_w0,
            1.0 - alpha / a,
        );
    }

    /// Calculate high shelf filter coefficients.
    fn calc_high_shelf(&mut self) {
        let a = crate::math::db_to_eq_amplitude(self.high_gain.as_f32());
        let omega = crate::math::biquad_omega(self.high_freq, self.sample_rate);
        let cos_w0 = omega.cos_w0;
        let sin_w0 = omega.sin_w0;
        let shelf_slope = 0.9_f32;
        let alpha = crate::math::biquad_alpha_shelf(sin_w0, a, shelf_slope);
        let sqrt_a = a.sqrt();

        self.high_coeffs = BiquadCoeffs::from_raw(
            a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
            a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha),
            (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
            (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha,
        );
    }

    /// Update all filter coefficients.
    fn update_coefficients(&mut self) {
        if self.coeffs_dirty {
            self.calc_low_shelf();
            self.calc_mid_peak();
            self.calc_high_shelf();
            self.coeffs_dirty = false;
        }
    }
}

impl Default for Eq {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Eq {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("eq", "EQ")
            .description("3-band parametric equalizer")
            .category(ModuleCategory::Effect)
            .tag("eq")
            .tag("effect")
            .tag("filter")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .parameter(
                ParameterDescriptor::float(
                    "low_freq",
                    Param::Eq(EqParam::LowFreq(Hertz::new(200.0))),
                    "Low Freq",
                )
                .description("Low shelf frequency")
                .range(20.0, 500.0)
                .default(200.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "low_gain",
                    Param::Eq(EqParam::LowGain(Decibels::new(0.0))),
                    "Low Gain",
                )
                .description("Low shelf gain")
                .range(-12.0, 12.0)
                .default(0.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mid_freq",
                    Param::Eq(EqParam::MidFreq(Hertz::new(1000.0))),
                    "Mid Freq",
                )
                .description("Mid band frequency")
                .range(200.0, 5000.0)
                .default(1000.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mid_gain",
                    Param::Eq(EqParam::MidGain(Decibels::new(0.0))),
                    "Mid Gain",
                )
                .description("Mid band gain")
                .range(-12.0, 12.0)
                .default(0.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mid_q",
                    Param::Eq(EqParam::MidQ(NormalizedValue::new(1.0))),
                    "Mid Q",
                )
                .description("Mid band Q factor")
                .range(0.1, 10.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "high_freq",
                    Param::Eq(EqParam::HighFreq(Hertz::new(4000.0))),
                    "High Freq",
                )
                .description("High shelf frequency")
                .range(1000.0, 16000.0)
                .default(4000.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "high_gain",
                    Param::Eq(EqParam::HighGain(Decibels::new(0.0))),
                    "High Gain",
                )
                .description("High shelf gain")
                .range(-12.0, 12.0)
                .default(0.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Eq(EqParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Eq {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        // Update sample rate and recalculate coefficients if needed
        if self.sample_rate != context.sample_rate {
            self.sample_rate = context.sample_rate;
            self.coeffs_dirty = true;
        }
        self.update_coefficients();

        // Process stereo interleaved
        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);
            let in_l = dry.left;
            let in_r = dry.right;

            // Process through 3 bands in series
            let (low_l, low_r) = self.low_state.process(in_l, in_r, &self.low_coeffs);
            let (mid_l, mid_r) = self.mid_state.process(low_l, low_r, &self.mid_coeffs);
            let (wet_l, wet_r) = self.high_state.process(mid_l, mid_r, &self.high_coeffs);

            // Mix dry/wet
            let mix = self.mix.as_f32();
            let result = StereoSample::new(in_l, in_r).blend(StereoSample::new(wet_l, wet_r), mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        self.low_state.reset();
        self.mid_state.reset();
        self.high_state.reset();
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Eq(eq_param) = param {
            match eq_param {
                EqParam::LowFreq(f) => {
                    self.low_freq = Hertz::new(f.as_f32().clamp(20.0, 500.0));
                    self.coeffs_dirty = true;
                }
                EqParam::LowGain(g) => {
                    self.low_gain = Decibels::new(g.as_f32().clamp(-12.0, 12.0));
                    self.coeffs_dirty = true;
                }
                EqParam::MidFreq(f) => {
                    self.mid_freq = Hertz::new(f.as_f32().clamp(200.0, 5000.0));
                    self.coeffs_dirty = true;
                }
                EqParam::MidGain(g) => {
                    self.mid_gain = Decibels::new(g.as_f32().clamp(-12.0, 12.0));
                    self.coeffs_dirty = true;
                }
                EqParam::MidQ(q) => {
                    self.mid_q = q.as_f32().clamp(0.1, 10.0);
                    self.coeffs_dirty = true;
                }
                EqParam::HighFreq(f) => {
                    self.high_freq = Hertz::new(f.as_f32().clamp(1000.0, 16000.0));
                    self.coeffs_dirty = true;
                }
                EqParam::HighGain(g) => {
                    self.high_gain = Decibels::new(g.as_f32().clamp(-12.0, 12.0));
                    self.coeffs_dirty = true;
                }
                EqParam::Mix(m) => {
                    self.mix = m;
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Eq(eq_param) = param {
            Some(match eq_param {
                EqParam::LowFreq(_) => self.low_freq.as_f32(),
                EqParam::LowGain(_) => self.low_gain.as_f32(),
                EqParam::MidFreq(_) => self.mid_freq.as_f32(),
                EqParam::MidGain(_) => self.mid_gain.as_f32(),
                EqParam::MidQ(_) => self.mid_q,
                EqParam::HighFreq(_) => self.high_freq.as_f32(),
                EqParam::HighGain(_) => self.high_gain.as_f32(),
                EqParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Eq(EqParam::LowFreq(self.low_freq)),
            Param::Eq(EqParam::LowGain(self.low_gain)),
            Param::Eq(EqParam::MidFreq(self.mid_freq)),
            Param::Eq(EqParam::MidGain(self.mid_gain)),
            Param::Eq(EqParam::MidQ(NormalizedValue::new(self.mid_q))),
            Param::Eq(EqParam::HighFreq(self.high_freq)),
            Param::Eq(EqParam::HighGain(self.high_gain)),
            Param::Eq(EqParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Eq
    }
}
