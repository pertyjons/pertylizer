//! 3-band parametric equalizer with low shelf, mid peak, and high shelf.

use crate::engine::typed_params::{EqParam, ModuleType, TypedParam, TypedValue};
use crate::modules::{
    Describable, EffectModule, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PortDescriptor, ProcessContext, WidgetHint,
};

/// Biquad filter coefficients.
#[derive(Clone, Copy, Default)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
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
        let out_l = c.b0 * in_l + c.b1 * self.x1_l + c.b2 * self.x2_l
            - c.a1 * self.y1_l
            - c.a2 * self.y2_l;
        let out_r = c.b0 * in_r + c.b1 * self.x1_r + c.b2 * self.x2_r
            - c.a1 * self.y1_r
            - c.a2 * self.y2_r;

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
    low_freq: f32,  // Hz
    low_gain: f32,  // dB
    mid_freq: f32,  // Hz
    mid_gain: f32,  // dB
    mid_q: f32,     // Q factor
    high_freq: f32, // Hz
    high_gain: f32, // dB
    mix: f32,       // Dry/wet

    // Filter state
    low_state: BiquadState,
    mid_state: BiquadState,
    high_state: BiquadState,

    // Cached coefficients
    low_coeffs: BiquadCoeffs,
    mid_coeffs: BiquadCoeffs,
    high_coeffs: BiquadCoeffs,

    // State
    sample_rate: f32,
    coeffs_dirty: bool,
}

impl Eq {
    pub fn new() -> Self {
        let mut eq = Self {
            low_freq: 200.0,
            low_gain: 0.0,
            mid_freq: 1000.0,
            mid_gain: 0.0,
            mid_q: 1.0,
            high_freq: 4000.0,
            high_gain: 0.0,
            mix: 1.0,
            low_state: BiquadState::default(),
            mid_state: BiquadState::default(),
            high_state: BiquadState::default(),
            low_coeffs: BiquadCoeffs::default(),
            mid_coeffs: BiquadCoeffs::default(),
            high_coeffs: BiquadCoeffs::default(),
            sample_rate: 48000.0,
            coeffs_dirty: true,
        };
        eq.update_coefficients();
        eq
    }

    /// Calculate low shelf filter coefficients.
    fn calc_low_shelf(&mut self) {
        let a = 10.0_f32.powf(self.low_gain / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * self.low_freq / self.sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / 0.9 - 1.0) + 2.0).sqrt();
        let sqrt_a = a.sqrt();

        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha;
        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha);

        self.low_coeffs = BiquadCoeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        };
    }

    /// Calculate peaking EQ filter coefficients.
    fn calc_mid_peak(&mut self) {
        let a = 10.0_f32.powf(self.mid_gain / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * self.mid_freq / self.sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * self.mid_q);

        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;
        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;

        self.mid_coeffs = BiquadCoeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        };
    }

    /// Calculate high shelf filter coefficients.
    fn calc_high_shelf(&mut self) {
        let a = 10.0_f32.powf(self.high_gain / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * self.high_freq / self.sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / 0.9 - 1.0) + 2.0).sqrt();
        let sqrt_a = a.sqrt();

        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha;
        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha);

        self.high_coeffs = BiquadCoeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        };
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
                ParameterDescriptor::float(TypedParam::Eq(EqParam::LowFreq), "Low Freq")
                    .description("Low shelf frequency")
                    .range(20.0, 500.0)
                    .default(200.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Eq(EqParam::LowGain), "Low Gain")
                    .description("Low shelf gain")
                    .range(-12.0, 12.0)
                    .default(0.0)
                    .unit(ParameterUnit::Decibels)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Eq(EqParam::MidFreq), "Mid Freq")
                    .description("Mid band frequency")
                    .range(200.0, 5000.0)
                    .default(1000.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Eq(EqParam::MidGain), "Mid Gain")
                    .description("Mid band gain")
                    .range(-12.0, 12.0)
                    .default(0.0)
                    .unit(ParameterUnit::Decibels)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Eq(EqParam::MidQ), "Mid Q")
                    .description("Mid band Q factor")
                    .range(0.1, 10.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Eq(EqParam::HighFreq), "High Freq")
                    .description("High shelf frequency")
                    .range(1000.0, 16000.0)
                    .default(4000.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Eq(EqParam::HighGain), "High Gain")
                    .description("High shelf gain")
                    .range(-12.0, 12.0)
                    .default(0.0)
                    .unit(ParameterUnit::Decibels)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Eq(EqParam::Mix), "Mix")
                    .description("Dry/wet mix")
                    .range(0.0, 1.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
    }
}

impl EffectModule for Eq {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        // Update sample rate and recalculate coefficients if needed
        if (self.sample_rate - context.sample_rate).abs() > 1.0 {
            self.sample_rate = context.sample_rate;
            self.coeffs_dirty = true;
        }
        self.update_coefficients();

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

            // Process through 3 bands in series
            let (low_l, low_r) = self.low_state.process(in_l, in_r, &self.low_coeffs);
            let (mid_l, mid_r) = self.mid_state.process(low_l, low_r, &self.mid_coeffs);
            let (wet_l, wet_r) = self.high_state.process(mid_l, mid_r, &self.high_coeffs);

            // Mix dry/wet
            if idx_l < output.len() {
                output[idx_l] = in_l * (1.0 - self.mix) + wet_l * self.mix;
            }
            if idx_r < output.len() {
                output[idx_r] = in_r * (1.0 - self.mix) + wet_r * self.mix;
            }
        }
    }

    fn reset(&mut self) {
        self.low_state.reset();
        self.mid_state.reset();
        self.high_state.reset();
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    fn get_mix(&self) -> f32 {
        self.mix
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Eq(eq_param) = param {
            match eq_param {
                EqParam::LowFreq => {
                    if let Some(f) = value.as_float() {
                        self.low_freq = f.clamp(20.0, 500.0);
                        self.coeffs_dirty = true;
                    }
                }
                EqParam::LowGain => {
                    if let Some(g) = value.as_float() {
                        self.low_gain = g.clamp(-12.0, 12.0);
                        self.coeffs_dirty = true;
                    }
                }
                EqParam::MidFreq => {
                    if let Some(f) = value.as_float() {
                        self.mid_freq = f.clamp(200.0, 5000.0);
                        self.coeffs_dirty = true;
                    }
                }
                EqParam::MidGain => {
                    if let Some(g) = value.as_float() {
                        self.mid_gain = g.clamp(-12.0, 12.0);
                        self.coeffs_dirty = true;
                    }
                }
                EqParam::MidQ => {
                    if let Some(q) = value.as_float() {
                        self.mid_q = q.clamp(0.1, 10.0);
                        self.coeffs_dirty = true;
                    }
                }
                EqParam::HighFreq => {
                    if let Some(f) = value.as_float() {
                        self.high_freq = f.clamp(1000.0, 16000.0);
                        self.coeffs_dirty = true;
                    }
                }
                EqParam::HighGain => {
                    if let Some(g) = value.as_float() {
                        self.high_gain = g.clamp(-12.0, 12.0);
                        self.coeffs_dirty = true;
                    }
                }
                EqParam::Mix => {
                    if let Some(m) = value.as_float() {
                        self.mix = m.clamp(0.0, 1.0);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Eq(eq_param) = param {
            match eq_param {
                EqParam::LowFreq => Some(TypedValue::Float(self.low_freq)),
                EqParam::LowGain => Some(TypedValue::Float(self.low_gain)),
                EqParam::MidFreq => Some(TypedValue::Float(self.mid_freq)),
                EqParam::MidGain => Some(TypedValue::Float(self.mid_gain)),
                EqParam::MidQ => Some(TypedValue::Float(self.mid_q)),
                EqParam::HighFreq => Some(TypedValue::Float(self.high_freq)),
                EqParam::HighGain => Some(TypedValue::Float(self.high_gain)),
                EqParam::Mix => Some(TypedValue::Float(self.mix)),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Eq
    }
}
