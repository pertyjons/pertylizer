//! LPC Vocoder effect.
//!
//! Analyzes the spectral envelope of the input using Linear Predictive Coding
//! and applies it as an all-pole filter. Outputs mono (both channels receive
//! the same filtered signal) since LPC analysis operates on a mono mixdown.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Analysis/137-lpc-analysis-autocorrelation-levinson-durbin-recursion.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use synth_core::{
    AudioEffect, Describable, Milliseconds, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ParameterUnit, PortDescriptor, ProcessContext,
    SampleRate, StereoSample, VocoderParam, WidgetHint,
};

use crate::math::{MAX_LPC_ORDER, lpc_analysis_fixed};

/// Analysis buffer size (max window, ~46ms at 48kHz).
const MAX_WINDOW_SAMPLES: usize = 2048;

pub struct Vocoder {
    // Parameters
    order: NormalizedValue,
    window_size_ms: Milliseconds,
    mix: NormalizedValue,

    // Cached derived value
    cached_order: usize,

    // Analysis state
    analysis_buf: [f32; MAX_WINDOW_SAMPLES],
    analysis_pos: usize,
    analysis_len: usize,

    // LPC coefficients
    coeffs: [f32; MAX_LPC_ORDER],

    // All-pole filter state
    filter_state: [f32; MAX_LPC_ORDER],

    sample_rate: SampleRate,
}

impl Vocoder {
    pub fn new() -> Self {
        let order = NormalizedValue::new(0.5);
        Self {
            order,
            window_size_ms: Milliseconds::new(20.0),
            mix: NormalizedValue::MAX,
            cached_order: Self::compute_order(order),
            analysis_buf: [0.0; MAX_WINDOW_SAMPLES],
            analysis_pos: 0,
            analysis_len: 960, // 20ms at 48kHz
            coeffs: [0.0; MAX_LPC_ORDER],
            filter_state: [0.0; MAX_LPC_ORDER],
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Compute the effective LPC order from the normalized parameter (4-32).
    fn compute_order(v: NormalizedValue) -> usize {
        (4.0 + v.as_f32() * 28.0) as usize
    }

    /// Update analysis window length from milliseconds.
    fn update_window_len(&mut self) {
        let samples = (self.window_size_ms.as_f32() * 0.001 * self.sample_rate.as_f32()) as usize;
        self.analysis_len = samples.clamp(64, MAX_WINDOW_SAMPLES);
    }

    /// Apply all-pole filter to a single sample using current LPC coefficients.
    ///
    /// Belt-and-suspenders NaN guard: if numerical conditioning makes the
    /// filter blow up despite the Levinson-Durbin reflection-coefficient
    /// clamp upstream, reset the state and pass the dry sample through
    /// instead of letting ±∞ / NaN propagate to the output.
    #[inline]
    fn filter_sample(&mut self, input: f32) -> f32 {
        let order = self.cached_order;
        let mut output = input;
        for i in 0..order {
            output -= self.coeffs[i] * self.filter_state[i];
        }
        if !output.is_finite() {
            self.filter_state.fill(0.0);
            self.coeffs.fill(0.0);
            return input;
        }
        for i in (1..order).rev() {
            self.filter_state[i] = self.filter_state[i - 1];
        }
        self.filter_state[0] = output;
        output
    }
}

impl Default for Vocoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Vocoder {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("vocoder", "LPC Vocoder")
            .description("LPC spectral envelope vocoder effect")
            .category(ModuleCategory::Effect)
            .tag("vocoder")
            .tag("lpc")
            .tag("spectral")
            .parameter(
                ParameterDescriptor::float(
                    "order",
                    Param::Vocoder(VocoderParam::Order(NormalizedValue::new(0.5))),
                    "Order",
                )
                .description("LPC analysis order (4-32)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "window_size",
                    Param::Vocoder(VocoderParam::WindowSize(Milliseconds::new(20.0))),
                    "Window",
                )
                .description("Analysis window size")
                .range(5.0, 50.0)
                .default(20.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Vocoder(VocoderParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in_l", "In L"))
            .port(PortDescriptor::audio_input("in_r", "In R"))
            .port(PortDescriptor::audio_output("out_l", "Out L"))
            .port(PortDescriptor::audio_output("out_r", "Out R"))
    }
}

impl AudioEffect for Vocoder {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        self.sample_rate = context.sample_rate;
        let num_frames = context.samples.as_usize();
        let mix = self.mix.as_f32();

        for frame in 0..num_frames {
            let dry = StereoSample::read_frame(input, frame);
            let mono = dry.to_mono();

            if self.analysis_pos < self.analysis_len {
                self.analysis_buf[self.analysis_pos] = mono;
            }
            self.analysis_pos += 1;

            if self.analysis_pos >= self.analysis_len {
                self.analysis_pos = 0;
                lpc_analysis_fixed(
                    &self.analysis_buf[..self.analysis_len],
                    self.cached_order,
                    &mut self.coeffs,
                );
            }

            let filtered = self.filter_sample(mono);
            let wet = StereoSample::new(filtered, filtered);
            let result = dry.blend(wet, mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Vocoder(p) = param {
            match p {
                VocoderParam::Order(v) => {
                    self.order = v;
                    self.cached_order = Self::compute_order(v);
                }
                VocoderParam::WindowSize(ms) => {
                    self.window_size_ms = ms;
                    self.update_window_len();
                }
                VocoderParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Vocoder(p) = param {
            Some(match p {
                VocoderParam::Order(_) => self.order.as_f32(),
                VocoderParam::WindowSize(_) => self.window_size_ms.as_f32(),
                VocoderParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Vocoder(VocoderParam::Order(self.order)),
            Param::Vocoder(VocoderParam::WindowSize(self.window_size_ms)),
            Param::Vocoder(VocoderParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Vocoder
    }

    fn reset(&mut self) {
        self.analysis_buf.fill(0.0);
        self.analysis_pos = 0;
        self.coeffs.fill(0.0);
        self.filter_state.fill(0.0);
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.update_window_len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    fn ctx_with_rate(sr: u32, frames: usize) -> ProcessContext<'static> {
        ProcessContext {
            sample_rate: SampleRate::new(sr as f32),
            samples: SampleCount::new(frames),
            ..ProcessContext::default()
        }
    }

    #[test]
    fn vocoder_creation() {
        let v = Vocoder::new();
        assert_eq!(v.cached_order, 18); // 4 + 0.5*28 = 18
    }

    /// Regression: Vocoder + non-stationary input (impulse-like or
    /// formant-style decaying carriers) used to feed Levinson-Durbin a
    /// degenerate autocorrelation matrix → reflection coefficients with
    /// |k| ≥ 1 → unstable all-pole filter → ±∞ → NaN. Now the
    /// reflection coefficients are clamped and the filter has a NaN guard.
    #[test]
    fn vocoder_stable_on_decaying_carrier() {
        let mut v = Vocoder::new();
        v.set_sample_rate(SampleRate::new(44100.0));

        // Synthesize a formant-like decaying carrier: each phase cycle of
        // 200 samples carries a sin·exp(-t·k) burst (similar shape to the
        // MathOscillator "formant" algorithm that originally exposed this).
        let frames = 256;
        let mut input = vec![0.0f32; frames * 2];
        for f in 0..frames {
            let t = (f % 200) as f32 / 200.0;
            let s = (std::f32::consts::TAU * t * 11.0).sin() * (-t * 7.0).exp();
            input[f * 2] = s;
            input[f * 2 + 1] = s;
        }
        let mut output = vec![0.0f32; frames * 2];
        let context = ctx_with_rate(44100, frames);

        for _ in 0..200 {
            v.process(&input, &mut output, &context);
        }

        for sample in &output {
            assert!(
                sample.is_finite(),
                "Vocoder output non-finite on decaying carrier: {sample}"
            );
            // The clamped Levinson-Durbin keeps the filter strictly stable
            // (poles inside the unit circle), but a high-Q resonator on a
            // peaky carrier can still ring loud. We just want to catch
            // unbounded growth — anything finite within ~50× input is fine,
            // anything truly exploding will hit the NaN guard.
            assert!(
                sample.abs() < 50.0,
                "Vocoder output exploded on decaying carrier: |{sample}| ≥ 50"
            );
        }
    }

    /// Make sure a clean sine wave still passes through stably across both
    /// 44.1 and 48 kHz at high LPC order.
    #[test]
    fn vocoder_stable_on_sine() {
        for sr in [44100u32, 48000] {
            let mut v = Vocoder::new();
            v.set_sample_rate(SampleRate::new(sr as f32));
            v.set_param(Param::Vocoder(VocoderParam::Order(NormalizedValue::new(
                1.0,
            ))));
            v.set_param(Param::Vocoder(VocoderParam::WindowSize(Milliseconds::new(
                50.0,
            ))));

            let frames = 256;
            let mut input = vec![0.0f32; frames * 2];
            for f in 0..frames {
                let s = (std::f32::consts::TAU * 220.0 * f as f32 / sr as f32).sin() * 0.7;
                input[f * 2] = s;
                input[f * 2 + 1] = s;
            }
            let mut output = vec![0.0f32; frames * 2];
            let context = ctx_with_rate(sr, frames);

            for _ in 0..200 {
                v.process(&input, &mut output, &context);
            }

            for sample in &output {
                assert!(
                    sample.is_finite(),
                    "Vocoder output non-finite on sine at sr={sr}: {sample}"
                );
            }
        }
    }
}
