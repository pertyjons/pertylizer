//! LPC Vocoder effect.
//!
//! Analyzes the spectral envelope of the input using Linear Predictive Coding
//! and applies it as an all-pole filter. Outputs mono (both channels receive
//! the same filtered signal) since LPC analysis operates on a mono mixdown.
//!
//! Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Analysis/137-lpc-analysis-autocorrelation-levinson-durbin-recursion.rst>
//! From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)

use synth_core::{
    AudioEffect, Describable, Milliseconds, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ParameterUnit, PortDescriptor, ProcessContext,
    SampleRate, StereoSample, VocoderParam, WidgetHint,
};

use crate::math::{MAX_LPC_ORDER, envelope_coeff, lpc_analysis_fixed, smooth_value};

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

    // Synthesis gain `G = sqrt(∏(1 - kᵢ²))` from the last analysis. Drives the
    // all-pole filter (`G·x[n] − Σ aₖ·y[n−k]`) so it doesn't apply the full
    // resonance boost of the modelled envelope. Unity until the first analysis.
    gain: f32,

    // Output level-match (loudness preservation) envelope followers. `G` cancels
    // the *average* prediction-gain boost, but this auto-vocoder still resonates
    // at its own formants (it filters the input through `1/A(z)` built from that
    // same input — the positive-feedback case), which can still boost some inputs.
    // Following the dry and wet envelopes and scaling the wet so it never exceeds
    // the dry level is the standard LPC gain-matching stage that bounds the output
    // to the input loudness regardless of input. Smoothed → no zipper noise.
    dry_env: f32,
    wet_env: f32,
    env_coeff: f32,

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
            gain: 1.0,
            dry_env: 0.0,
            wet_env: 0.0,
            // 10 ms loudness-follower time constant at the default rate; refreshed
            // by `update_window_len` whenever the sample rate changes.
            env_coeff: envelope_coeff(0.010, SampleRate::DVD_QUALITY.as_f32()),
            filter_state: [0.0; MAX_LPC_ORDER],
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Compute the effective LPC order from the normalized parameter (4-32).
    fn compute_order(v: NormalizedValue) -> usize {
        (4.0 + v.as_f32() * 28.0) as usize
    }

    /// Update analysis window length (and the loudness-follower coefficient)
    /// from the current sample rate.
    fn update_window_len(&mut self) {
        let samples = (self.window_size_ms.as_f32() * 0.001 * self.sample_rate.as_f32()) as usize;
        self.analysis_len = samples.clamp(64, MAX_WINDOW_SAMPLES);
        self.env_coeff = envelope_coeff(0.010, self.sample_rate.as_f32());
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
        // `y[n] = G·x[n] − Σ aₖ·y[n−k]`: the synthesis gain `G` compensates the
        // all-pole filter's resonance boost so the output tracks the input level.
        let mut output = self.gain * input;
        for i in 0..order {
            output -= self.coeffs[i] * self.filter_state[i];
        }
        if !output.is_finite() {
            self.filter_state.fill(0.0);
            self.coeffs.fill(0.0);
            self.gain = 1.0;
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
            .width(synth_core::ModuleWidth::Medium)
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
                self.gain = lpc_analysis_fixed(
                    &self.analysis_buf[..self.analysis_len],
                    self.cached_order,
                    &mut self.coeffs,
                );
            }

            let filtered = self.filter_sample(mono);

            // Loudness-match: follow the dry and wet envelopes and scale the wet
            // so it never exceeds the dry level. Bounds the auto-vocoder's
            // self-resonance to the input loudness for any input (the LPC gain
            // alone only compensates the average boost).
            self.dry_env = smooth_value(self.dry_env, mono.abs(), self.env_coeff);
            self.wet_env = smooth_value(self.wet_env, filtered.abs(), self.env_coeff);
            let level_match = if self.wet_env > 1e-4 {
                (self.dry_env / self.wet_env).min(1.0)
            } else {
                1.0
            };
            let matched = filtered * level_match;

            let wet = StereoSample::new(matched, matched);
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
        self.gain = 1.0;
        self.dry_env = 0.0;
        self.wet_env = 0.0;
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

    #[test]
    fn lpc_gain_attenuates_resonant_input() {
        // A near-periodic (highly predictable) signal yields a strong LPC
        // model → small residual energy → synthesis gain well below 1.0, which
        // is what cancels the all-pole filter's resonance boost.
        let n = 512;
        let sig: Vec<f32> = (0..n)
            .map(|i| (std::f32::consts::TAU * 8.0 * i as f32 / n as f32).sin() * 0.7)
            .collect();
        let mut coeffs = [0.0f32; MAX_LPC_ORDER];
        let gain = lpc_analysis_fixed(&sig, 18, &mut coeffs);
        assert!(gain.is_finite());
        assert!(
            (0.0..1.0).contains(&gain),
            "resonant input should give attenuating gain in (0,1), got {gain}"
        );
    }

    #[test]
    fn lpc_gain_is_unity_on_degenerate_input() {
        // Silence has no energy → no usable prediction → unity gain so the
        // filter is a clean passthrough rather than a divide-by-zero.
        let mut coeffs = [0.0f32; MAX_LPC_ORDER];
        let gain = lpc_analysis_fixed(&[0.0f32; 256], 18, &mut coeffs);
        assert_eq!(gain, 1.0);
        assert!(coeffs.iter().all(|&c| c == 0.0));
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
        let mut in_peak = 0.0f32;
        for f in 0..frames {
            let t = (f % 200) as f32 / 200.0;
            let s = (std::f32::consts::TAU * t * 11.0).sin() * (-t * 7.0).exp();
            input[f * 2] = s;
            input[f * 2 + 1] = s;
            in_peak = in_peak.max(s.abs());
        }
        let mut output = vec![0.0f32; frames * 2];
        let context = ctx_with_rate(44100, frames);

        let mut out_peak = 0.0f32;
        for _ in 0..200 {
            v.process(&input, &mut output, &context);
            for sample in &output {
                assert!(
                    sample.is_finite(),
                    "Vocoder output non-finite on decaying carrier: {sample}"
                );
                out_peak = out_peak.max(sample.abs());
            }
        }

        // Before the fix this peaky carrier drove the all-pole filter to ~25×
        // the input (clipping at 0 dB / going NaN). The synthesis gain `G`
        // cancels the average prediction-gain boost and the output loudness-match
        // pins the wet level to the dry envelope, so the output now tracks the
        // input level (~1.8× peak here, allowing for envelope-follower lag).
        // 2.5× is the musical bound from the original bug report; it still fails
        // hard on any return of the old explosion.
        assert!(
            out_peak < 2.5 * in_peak,
            "Vocoder resonance not tamed: out_peak {out_peak} ≥ 2.5× in_peak {in_peak}"
        );
    }

    /// A sustained periodic tone (the common vocoder carrier) must come out at
    /// roughly the input loudness — not the +19 dB boost the raw all-pole filter
    /// applied before the output loudness-match was added.
    #[test]
    fn vocoder_preserves_loudness_on_sustained_tone() {
        let mut v = Vocoder::new();
        v.set_sample_rate(SampleRate::new(44100.0));

        let frames = 256;
        let mut input = vec![0.0f32; frames * 2];
        let mut in_sq = 0.0f32;
        for f in 0..frames {
            let phase = (110.0 * f as f32 / 44100.0).fract();
            let s = (2.0 * phase - 1.0) * 0.5; // sawtooth
            input[f * 2] = s;
            input[f * 2 + 1] = s;
            in_sq += s * s;
        }
        let in_rms = (in_sq / frames as f32).sqrt();

        let mut output = vec![0.0f32; frames * 2];
        let context = ctx_with_rate(44100, frames);
        for _ in 0..200 {
            v.process(&input, &mut output, &context);
        }

        let out_rms = (output.iter().step_by(2).map(|s| s * s).sum::<f32>() / frames as f32).sqrt();
        let ratio = out_rms / in_rms;
        assert!(
            (0.3..=1.5).contains(&ratio),
            "Vocoder output loudness off: out/in RMS ratio {ratio} not in [0.3, 1.5]"
        );
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
