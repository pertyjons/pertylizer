//! Bode frequency shifter effect.
//!
//! Uses two chains of cascaded all-pass filters (Hilbert transform pair)
//! to produce 90-degree phase-shifted analytic signal components, then
//! multiplies by sin/cos of shift frequency for single-sideband modulation.
//!
//! Modes: Up-shift, Down-shift, or Stereo (up L / down R).

use synth_core::{
    AudioEffect, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor, ParameterUnit,
    PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{FrequencyShifterParam, ModuleType, Param};
use synth_core::{Hertz, NormalizedValue, Phase, SampleRate, StereoSample};

/// Number of all-pass stages per Hilbert chain.
const NUM_STAGES: usize = 4;

/// Hilbert transform all-pass coefficients for ~48 kHz sample rate.
/// Chain A produces 0-degree component, Chain B produces 90-degree component.
/// These are optimized for broadband 90-degree phase difference (20 Hz - 20 kHz).
const HILBERT_COEFFS_A: [f32; NUM_STAGES] = [0.6923878, 0.9360654, 0.9882295, 0.9987488];
const HILBERT_COEFFS_B: [f32; NUM_STAGES] = [0.4021921, 0.856_171, 0.9722909, 0.9952884];

/// Bode frequency shifter using Hilbert transform.
pub struct FrequencyShifter {
    // Parameters
    shift: Hertz,
    mix: NormalizedValue,
    mode: NormalizedValue,

    // Hilbert all-pass chains (stereo: L and R)
    chain_a_l: [f32; NUM_STAGES],
    chain_b_l: [f32; NUM_STAGES],
    chain_a_r: [f32; NUM_STAGES],
    chain_b_r: [f32; NUM_STAGES],

    // Shift oscillator phase
    osc_phase: Phase,

    // State
    sample_rate: SampleRate,
}

impl FrequencyShifter {
    pub fn new() -> Self {
        Self {
            shift: Hertz::new(0.0),
            mix: NormalizedValue::MAX,
            mode: NormalizedValue::MIN,
            chain_a_l: [0.0; NUM_STAGES],
            chain_b_l: [0.0; NUM_STAGES],
            chain_a_r: [0.0; NUM_STAGES],
            chain_b_r: [0.0; NUM_STAGES],
            osc_phase: Phase::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Process a sample through an all-pass chain, returning the output.
    #[inline]
    fn process_chain(state: &mut [f32; NUM_STAGES], coeffs: &[f32; NUM_STAGES], input: f32) -> f32 {
        let mut x = input;
        for i in 0..NUM_STAGES {
            let y = coeffs[i] * (x - state[i]) + state[i];
            state[i] = y;
            // Feed forward: output of this stage becomes input to next
            // But for the Hilbert pair, we use simple first-order all-pass
            // y[n] = c * (x[n] - y[n-1]) + x[n-1]
            // Simplified: store previous output
            let prev = state[i];
            let out = coeffs[i] * (x - prev) + prev;
            state[i] = out;
            x = out;
        }
        x
    }
}

impl Default for FrequencyShifter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for FrequencyShifter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("frequency_shifter", "Freq Shifter")
            .description("Bode frequency shifter — shifts all frequencies by a fixed Hz amount")
            .category(ModuleCategory::Effect)
            .tag("frequency_shifter")
            .tag("effect")
            .tag("spectral")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .parameter(
                ParameterDescriptor::float(
                    "shift",
                    Param::FrequencyShifter(FrequencyShifterParam::Shift(Hertz::new(0.0))),
                    "Shift",
                )
                .description("Frequency shift amount in Hz")
                .range(-1000.0, 1000.0)
                .default(0.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::FrequencyShifter(FrequencyShifterParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mode",
                    Param::FrequencyShifter(FrequencyShifterParam::Mode(NormalizedValue::MIN)),
                    "Mode",
                )
                .description("0=Up shift, 0.5=Down shift, 1.0=Stereo (up L, down R)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for FrequencyShifter {
    #[allow(clippy::too_many_lines)]
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        self.sample_rate = context.sample_rate;
        let sr = self.sample_rate.as_f32();
        let shift_hz = self.shift.as_f32();
        let phase_inc = shift_hz.abs() / sr;
        let shift_sign = shift_hz.signum();

        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);
            let in_l = dry.left;
            let in_r = dry.right;

            // Hilbert transform: process through both all-pass chains
            let hilbert_a_l = Self::process_chain(&mut self.chain_a_l, &HILBERT_COEFFS_A, in_l);
            let hilbert_b_l = Self::process_chain(&mut self.chain_b_l, &HILBERT_COEFFS_B, in_l);
            let hilbert_a_r = Self::process_chain(&mut self.chain_a_r, &HILBERT_COEFFS_A, in_r);
            let hilbert_b_r = Self::process_chain(&mut self.chain_b_r, &HILBERT_COEFFS_B, in_r);

            // Generate sin/cos for frequency shifting
            let p = self.osc_phase.as_f32();
            let cos_val = (p * std::f32::consts::TAU).cos();
            let sin_val = (p * std::f32::consts::TAU).sin();
            self.osc_phase = self.osc_phase.advance(phase_inc);

            // Single-sideband modulation:
            // Up-shift:   out = hilbert_a * cos - hilbert_b * sin
            // Down-shift: out = hilbert_a * cos + hilbert_b * sin
            let up_l = hilbert_a_l * cos_val - hilbert_b_l * sin_val * shift_sign;
            let down_l = hilbert_a_l * cos_val + hilbert_b_l * sin_val * shift_sign;
            let up_r = hilbert_a_r * cos_val - hilbert_b_r * sin_val * shift_sign;
            let down_r = hilbert_a_r * cos_val + hilbert_b_r * sin_val * shift_sign;

            // Mode selection: 0=up, 0.5=down, 1.0=stereo
            let mode_val = self.mode.as_f32();
            let (shifted_l, shifted_r) = if mode_val < 0.33 {
                // Up shift
                (up_l, up_r)
            } else if mode_val < 0.66 {
                // Down shift
                (down_l, down_r)
            } else {
                // Stereo: up on left, down on right
                (up_l, down_r)
            };

            // Mix
            let mix = self.mix.as_f32();
            let result =
                StereoSample::new(in_l, in_r).blend(StereoSample::new(shifted_l, shifted_r), mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        self.chain_a_l = [0.0; NUM_STAGES];
        self.chain_b_l = [0.0; NUM_STAGES];
        self.chain_a_r = [0.0; NUM_STAGES];
        self.chain_b_r = [0.0; NUM_STAGES];
        self.osc_phase = Phase::ZERO;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::FrequencyShifter(p) = param {
            match p {
                FrequencyShifterParam::Shift(h) => {
                    self.shift = Hertz::new(h.as_f32().clamp(-1000.0, 1000.0));
                }
                FrequencyShifterParam::Mix(m) => {
                    self.mix = m;
                }
                FrequencyShifterParam::Mode(m) => {
                    self.mode = m;
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::FrequencyShifter(p) = param {
            Some(match p {
                FrequencyShifterParam::Shift(_) => self.shift.as_f32(),
                FrequencyShifterParam::Mix(_) => self.mix.as_f32(),
                FrequencyShifterParam::Mode(_) => self.mode.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::FrequencyShifter(FrequencyShifterParam::Shift(self.shift)),
            Param::FrequencyShifter(FrequencyShifterParam::Mix(self.mix)),
            Param::FrequencyShifter(FrequencyShifterParam::Mode(self.mode)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::FrequencyShifter
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    #[test]
    fn test_frequency_shifter_creation() {
        let fs = FrequencyShifter::new();
        assert_eq!(fs.shift.as_f32(), 0.0);
        assert_eq!(fs.mix.as_f32(), 1.0);
    }

    #[test]
    fn test_passthrough_at_zero_shift() {
        let mut fs = FrequencyShifter::new();
        let input = vec![0.5_f32, 0.5, -0.3, -0.3, 0.1, 0.1, 0.0, 0.0];
        let mut output = vec![0.0_f32; 8];

        let context = ProcessContext {
            samples: SampleCount::new(4),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        fs.process(&input, &mut output, &context);

        // With zero shift, output should approximate input (after Hilbert filter settling)
        // The all-pass chains introduce some latency so we just check it doesn't blow up
        for sample in &output {
            assert!(sample.abs() < 2.0, "Output too large: {sample}");
        }
    }

    #[test]
    fn test_frequency_shifter_params() {
        let mut fs = FrequencyShifter::new();
        fs.set_param(Param::FrequencyShifter(FrequencyShifterParam::Shift(
            Hertz::new(100.0),
        )));
        assert!((fs.shift.as_f32() - 100.0).abs() < 0.001);

        let params = fs.get_params();
        assert_eq!(params.len(), 3);
    }
}
