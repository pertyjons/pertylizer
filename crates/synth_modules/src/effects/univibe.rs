//! Univibe effect.
//!
//! Emulation of the classic Univox Univibe pedal using 4 cascaded all-pass filters
//! with optocoupler (vactrol) modeling. Produces the characteristic swirling sound
//! of Hendrix-era recordings.
//!
//! Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Effects/277-univox-univibe-emulator.rst>
//! From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)

use synth_core::{
    AudioEffect, Describable, Hertz, ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue,
    Param, ParameterDescriptor, Phase, PortDescriptor, ProcessContext, ResponseCurve, SampleCount,
    SampleRate, StereoSample, UnivibeParam, WidgetHint,
};

/// Number of all-pass filter stages.
const NUM_STAGES: usize = 4;

/// Center frequencies for the 4 all-pass stages (inspired by original Univibe).
const STAGE_FREQS: [f32; NUM_STAGES] = [300.0, 600.0, 1200.0, 2400.0];

/// Single first-order all-pass filter.
///
/// Transposed direct-form II for the all-pass H(z) = (a + z⁻¹) / (1 + a·z⁻¹).
/// Pole at z = −a, so the filter is stable for |a| < 1 — which the bilinear-
/// derived `coeff` always satisfies (mathematically `tan(ω) − 1` and
/// `tan(ω) + 1` give a ratio in [−1, 1]).
///
/// The previous implementation used a different recurrence whose effective
/// state-update pole was `coeff·(1 − coeff)`. That magnitude exceeds 1 once
/// `coeff` drops below ~−0.62, which Univibe routinely lands in for the
/// 300/600/1200 Hz stages (negative coefficients near −0.92 / −0.84 / −0.71).
/// The state grew exponentially, overflowed f32 to ±∞ within ~200 samples,
/// and arithmetic on ∞ produced NaN — surfacing as runaway clipping in
/// `analyze_note` (peak 1.0, centroid pinned at Nyquist).
#[derive(Clone, Copy, Default)]
struct AllPass {
    state_l: f32,
    state_r: f32,
}

impl AllPass {
    #[inline]
    fn process_stereo(&mut self, input_l: f32, input_r: f32, coeff: f32) -> (f32, f32) {
        let out_l = coeff * input_l + self.state_l;
        self.state_l = input_l - coeff * out_l;

        let out_r = coeff * input_r + self.state_r;
        self.state_r = input_r - coeff * out_r;

        (out_l, out_r)
    }

    fn reset(&mut self) {
        self.state_l = 0.0;
        self.state_r = 0.0;
    }
}

/// Univibe — vintage phaser/vibrato effect.
pub struct Univibe {
    rate: Hertz,
    depth: NormalizedValue,
    feedback: NormalizedValue,
    mix: NormalizedValue,
    sample_rate: SampleRate,
    stages: [AllPass; NUM_STAGES],
    lfo_phase: Phase,
    feedback_l: f32,
    feedback_r: f32,
}

impl Univibe {
    pub fn new() -> Self {
        Self {
            rate: Hertz::new(2.0),
            depth: NormalizedValue::new(0.7),
            feedback: NormalizedValue::new(0.3),
            mix: NormalizedValue::new(0.5),
            sample_rate: SampleRate::DVD_QUALITY,
            stages: [AllPass::default(); NUM_STAGES],
            lfo_phase: Phase::ZERO,
            feedback_l: 0.0,
            feedback_r: 0.0,
        }
    }

    /// Vactrol-style LFO: asymmetric response (fast attack, slow release).
    #[inline]
    fn vactrol_lfo(phase: f32) -> f32 {
        // Skewed sine: compress the top half for a more "organic" sweep
        let raw = (phase * std::f32::consts::TAU).sin();
        // Vactrol curve: logarithmic response
        if raw > 0.0 {
            raw.sqrt()
        } else {
            -((-raw).sqrt())
        }
    }
}

impl Default for Univibe {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Univibe {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("univibe", "Univibe")
            .description("Classic Univox Univibe phaser/vibrato emulation")
            .category(ModuleCategory::Effect)
            .tag("univibe")
            .tag("effect")
            .tag("modulation")
            .tag("phaser")
            .port(PortDescriptor::audio_input("in_l", "In L"))
            .port(PortDescriptor::audio_input("in_r", "In R"))
            .port(PortDescriptor::audio_output("out_l", "Out L"))
            .port(PortDescriptor::audio_output("out_r", "Out R"))
            .parameter(
                ParameterDescriptor::float(
                    "rate",
                    Param::Univibe(UnivibeParam::Rate(Hertz::new(2.0))),
                    "Rate",
                )
                .description("LFO speed")
                .range(0.1, 10.0)
                .default(2.0)
                .widget(WidgetHint::Knob)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "depth",
                    Param::Univibe(UnivibeParam::Depth(NormalizedValue::new(0.7))),
                    "Depth",
                )
                .description("Modulation depth")
                .range(0.0, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "feedback",
                    Param::Univibe(UnivibeParam::Feedback(NormalizedValue::new(0.3))),
                    "Feedback",
                )
                .description("Feedback amount")
                .range(0.0, 0.95)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Univibe(UnivibeParam::Mix(NormalizedValue::new(0.5))),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Univibe {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        self.sample_rate = context.sample_rate;
        let sr = self.sample_rate.as_f32();
        let phase_inc = self.rate.phase_increment(self.sample_rate);
        let depth = self.depth.as_f32();
        let feedback = self.feedback.as_f32().min(0.95);
        let mix = self.mix.as_f32();

        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);

            // Generate vactrol-modulated LFO
            let lfo = Self::vactrol_lfo(self.lfo_phase.as_f32());
            self.lfo_phase = self.lfo_phase.advance(phase_inc);

            // Apply feedback
            let in_l = dry.left + self.feedback_l * feedback;
            let in_r = dry.right + self.feedback_r * feedback;

            let mut sample_l = in_l;
            let mut sample_r = in_r;

            // Process through 4 cascaded all-pass filters with modulated frequencies
            for (i, stage) in self.stages.iter_mut().enumerate() {
                // Each stage has a different center frequency, modulated by the LFO
                let base_freq = STAGE_FREQS[i];
                let mod_freq = base_freq * (1.0 + lfo * depth * 0.8);
                let mod_freq = mod_freq.clamp(20.0, sr * 0.45);

                // All-pass coefficient from frequency
                let omega = std::f32::consts::PI * mod_freq / sr;
                let t = omega.tan();
                let coeff = (t - 1.0) / (t + 1.0);

                let (out_l, out_r) = stage.process_stereo(sample_l, sample_r, coeff);
                sample_l = out_l;
                sample_r = out_r;
            }

            // Store feedback (soft-limited)
            self.feedback_l = crate::math::soft_clip(sample_l);
            self.feedback_r = crate::math::soft_clip(sample_r);

            // Mix
            let wet = StereoSample::new(sample_l, sample_r);
            let result = dry.blend(wet, mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Univibe(p) = param {
            match p {
                UnivibeParam::Rate(r) => self.rate = Hertz::new(r.as_f32().clamp(0.1, 10.0)),
                UnivibeParam::Depth(d) => self.depth = d,
                UnivibeParam::Feedback(f) => {
                    self.feedback = NormalizedValue::new(f.as_f32().clamp(0.0, 0.95));
                }
                UnivibeParam::Mix(m) => self.mix = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Univibe(p) = param {
            Some(match p {
                UnivibeParam::Rate(_) => self.rate.as_f32(),
                UnivibeParam::Depth(_) => self.depth.as_f32(),
                UnivibeParam::Feedback(_) => self.feedback.as_f32(),
                UnivibeParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Univibe(UnivibeParam::Rate(self.rate)),
            Param::Univibe(UnivibeParam::Depth(self.depth)),
            Param::Univibe(UnivibeParam::Feedback(self.feedback)),
            Param::Univibe(UnivibeParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Univibe
    }

    fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
        self.lfo_phase = Phase::ZERO;
        self.feedback_l = 0.0;
        self.feedback_r = 0.0;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn univibe_creation() {
        let univibe = Univibe::new();
        assert!((univibe.rate.as_f32() - 2.0).abs() < 0.001);
        assert_eq!(univibe.stages.len(), NUM_STAGES);
    }

    fn ctx_with_rate(sr: u32, frames: usize) -> ProcessContext<'static> {
        ProcessContext {
            sample_rate: SampleRate::new(sr as f32),
            samples: SampleCount::new(frames),
            ..ProcessContext::default()
        }
    }

    /// Regression: Univibe used to overflow to ±∞ within ~200 samples because
    /// the AllPass topology had an unstable pole at `coeff·(1 − coeff)` for
    /// negative `coeff`. Catch any future regression by running the effect
    /// with default-ish parameters across both 44.1 and 48 kHz and asserting
    /// finite output.
    #[test]
    fn univibe_stable_no_nan() {
        for sr in [44100u32, 48000] {
            let mut univibe = Univibe::new();
            univibe.set_sample_rate(SampleRate::new(sr as f32));

            let frames = 256;
            let context = ctx_with_rate(sr, frames);
            let input = vec![0.5f32; frames * 2];
            let mut output = vec![0.0f32; frames * 2];

            for _ in 0..400 {
                univibe.process(&input, &mut output, &context);
            }

            for sample in &output {
                assert!(
                    sample.is_finite(),
                    "Univibe output non-finite at sr={sr}: {sample}"
                );
                assert!(
                    sample.abs() < 4.0,
                    "Univibe output exploded at sr={sr}: |{sample}| ≥ 4"
                );
            }
        }
    }

    /// Cover the stress range — high feedback and depth — to make sure the
    /// stable topology holds even when the LFO pushes coefficients to their
    /// extremes.
    #[test]
    fn univibe_stable_extreme_params() {
        let mut univibe = Univibe::new();
        univibe.set_param(Param::Univibe(UnivibeParam::Feedback(
            NormalizedValue::new(0.95),
        )));
        univibe.set_param(Param::Univibe(UnivibeParam::Depth(NormalizedValue::new(
            1.0,
        ))));
        univibe.set_param(Param::Univibe(UnivibeParam::Rate(Hertz::new(10.0))));

        let frames = 256;
        let context = ctx_with_rate(44100, frames);
        let input = vec![1.0f32; frames * 2];
        let mut output = vec![0.0f32; frames * 2];

        for _ in 0..400 {
            univibe.process(&input, &mut output, &context);
        }

        for sample in &output {
            assert!(
                sample.is_finite(),
                "Univibe output non-finite at extreme params: {sample}"
            );
        }
    }
}
