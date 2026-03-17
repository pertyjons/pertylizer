//! Phaser effect using cascaded all-pass filters.

use synth_core::{
    AllpassState, BipolarValue, Hertz, NormalizedValue, Phase, SampleRate, StereoSample,
};
use synth_core::{
    AudioEffect, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor, ParameterUnit,
    PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{ModuleType, Param, PhaserParam};

/// Number of all-pass filter stages (fixed at 6 for classic phaser sound).
const NUM_STAGES: usize = 6;

/// A single first-order all-pass filter stage.
#[derive(Clone, Copy, Default)]
struct AllPassStage {
    state: AllpassState,
}

impl AllPassStage {
    /// Process a sample through the all-pass filter.
    #[inline]
    fn process(&mut self, input: f32, coeff: f32) -> f32 {
        self.state.process(input, coeff)
    }

    fn reset(&mut self) {
        self.state.reset();
    }
}

/// Phaser effect with LFO-modulated all-pass filter stages.
pub struct Phaser {
    // Parameters
    rate: Hertz,
    depth: NormalizedValue,
    feedback: BipolarValue,
    center_freq: Hertz,
    mix: NormalizedValue,

    // All-pass filter stages (stereo)
    stages_l: [AllPassStage; NUM_STAGES],
    stages_r: [AllPassStage; NUM_STAGES],

    // LFO state
    lfo_phase: Phase,

    // Feedback state
    feedback_l: f32,
    feedback_r: f32,

    // State
    sample_rate: SampleRate,
}

impl Phaser {
    pub fn new() -> Self {
        Self {
            rate: Hertz::new(0.5),
            depth: NormalizedValue::new(0.7),
            feedback: BipolarValue::new(0.7),
            center_freq: Hertz::new(1000.0),
            mix: NormalizedValue::CENTER,
            stages_l: [AllPassStage::default(); NUM_STAGES],
            stages_r: [AllPassStage::default(); NUM_STAGES],
            lfo_phase: Phase::ZERO,
            feedback_l: 0.0,
            feedback_r: 0.0,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Calculate all-pass coefficient from frequency.
    #[inline]
    fn freq_to_coeff(&self, freq: Hertz) -> f32 {
        let tan_val = freq.to_tan_coeff(self.sample_rate);
        (tan_val - 1.0) / (tan_val + 1.0)
    }
}

impl Default for Phaser {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Phaser {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("phaser", "Phaser")
            .description("Classic phaser effect with LFO modulation")
            .category(ModuleCategory::Effect)
            .tag("phaser")
            .tag("effect")
            .tag("modulation")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            // Note: rate_cv port removed - AudioEffect trait doesn't support named CV inputs.
            // To add CV modulation, this effect would need to use PolyModule instead.
            .parameter(
                ParameterDescriptor::float(
                    "rate",
                    Param::Phaser(PhaserParam::Rate(Hertz::new(0.5))),
                    "Rate",
                )
                .description("LFO rate")
                .range(0.05, 5.0)
                .default(0.5)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "depth",
                    Param::Phaser(PhaserParam::Depth(NormalizedValue::new(0.7))),
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
                    Param::Phaser(PhaserParam::Feedback(BipolarValue::new(0.7))),
                    "Feedback",
                )
                .description("Feedback amount")
                .range(-0.95, 0.95)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "center_freq",
                    Param::Phaser(PhaserParam::CenterFreq(Hertz::new(1000.0))),
                    "Center Freq",
                )
                .description("Center frequency")
                .range(100.0, 4000.0)
                .default(1000.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Phaser(PhaserParam::Mix(NormalizedValue::CENTER)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Phaser {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        let phase_inc = self.rate.phase_increment(self.sample_rate);

        // Process stereo interleaved
        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);
            let in_l = dry.left;
            let in_r = dry.right;

            // Calculate LFO values (sine wave with stereo offset)
            let lfo_phase_l = self.lfo_phase.as_f32();
            let lfo_phase_r = (lfo_phase_l + 0.25).rem_euclid(1.0); // 90° offset for stereo width
            let lfo_l = (lfo_phase_l * std::f32::consts::TAU).sin();
            let lfo_r = (lfo_phase_r * std::f32::consts::TAU).sin();
            self.lfo_phase = self.lfo_phase.advance(phase_inc);

            // Calculate modulated frequencies for left and right
            let depth = self.depth.as_f32();
            let freq_mod_l_hz = self.center_freq.as_f32() * (1.0 + lfo_l * depth * 0.8);
            let freq_mod_r_hz = self.center_freq.as_f32() * (1.0 + lfo_r * depth * 0.8);
            let freq_mod_l =
                Hertz::new(freq_mod_l_hz.clamp(100.0, self.sample_rate.as_f32() * 0.45));
            let freq_mod_r =
                Hertz::new(freq_mod_r_hz.clamp(100.0, self.sample_rate.as_f32() * 0.45));

            // Calculate coefficients for all-pass filters
            let coeff_l = self.freq_to_coeff(freq_mod_l);
            let coeff_r = self.freq_to_coeff(freq_mod_r);

            // Process left channel through all-pass cascade
            let feedback = self.feedback.as_f32();
            let mut sample_l = in_l + self.feedback_l * feedback;
            for stage in &mut self.stages_l {
                sample_l = stage.process(sample_l, coeff_l);
            }
            self.feedback_l = sample_l;

            // Process right channel with offset phase for stereo width
            let mut sample_r = in_r + self.feedback_r * feedback;
            for stage in &mut self.stages_r {
                sample_r = stage.process(sample_r, coeff_r);
            }
            self.feedback_r = sample_r;

            // Mix dry/wet
            let mix = self.mix.as_f32();
            let result =
                StereoSample::new(in_l, in_r).blend(StereoSample::new(sample_l, sample_r), mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        for stage in &mut self.stages_l {
            stage.reset();
        }
        for stage in &mut self.stages_r {
            stage.reset();
        }
        self.feedback_l = 0.0;
        self.feedback_r = 0.0;
        self.lfo_phase = Phase::ZERO;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Phaser(phaser_param) = param {
            match phaser_param {
                PhaserParam::Rate(r) => {
                    self.rate = Hertz::new(r.as_f32().clamp(0.05, 5.0));
                }
                PhaserParam::Depth(d) => {
                    self.depth = d;
                }
                PhaserParam::Feedback(f) => {
                    self.feedback = BipolarValue::new(f.as_f32().clamp(-0.95, 0.95));
                }
                PhaserParam::Stages(_) => {
                    // Fixed at 6 stages, ignore
                }
                PhaserParam::CenterFreq(f) => {
                    self.center_freq = Hertz::new(f.as_f32().clamp(100.0, 4000.0));
                }
                PhaserParam::Mix(m) => {
                    self.mix = m;
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Phaser(phaser_param) = param {
            Some(match phaser_param {
                PhaserParam::Rate(_) => self.rate.as_f32(),
                PhaserParam::Depth(_) => self.depth.as_f32(),
                PhaserParam::Feedback(_) => self.feedback.as_f32(),
                PhaserParam::Stages(_) => NUM_STAGES as f32,
                PhaserParam::CenterFreq(_) => self.center_freq.as_f32(),
                PhaserParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Phaser(PhaserParam::Rate(self.rate)),
            Param::Phaser(PhaserParam::Depth(self.depth)),
            Param::Phaser(PhaserParam::Feedback(self.feedback)),
            Param::Phaser(PhaserParam::Stages(NUM_STAGES as u8)),
            Param::Phaser(PhaserParam::CenterFreq(self.center_freq)),
            Param::Phaser(PhaserParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Phaser
    }
}
