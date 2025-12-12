//! Phaser effect using cascaded all-pass filters.

use synth_core::{
    AudioEffect, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor, ParameterUnit,
    PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{BipolarValue, FilterState, Hertz, NormalizedValue, Phase, SampleRate};
use synth_core::{ModuleType, Param, PhaserParam};

/// Number of all-pass filter stages (fixed at 6 for classic phaser sound).
const NUM_STAGES: usize = 6;

/// A single first-order all-pass filter stage.
#[derive(Clone, Copy)]
struct AllPassStage {
    delay: FilterState,
}

impl Default for AllPassStage {
    fn default() -> Self {
        Self {
            delay: FilterState::ZERO,
        }
    }
}

impl AllPassStage {
    /// Process a sample through the all-pass filter.
    /// First-order all-pass: y[n] = coeff * (x[n] - y[n-1]) + y[n-1]
    /// The coefficient determines the center frequency of the phase shift.
    #[inline]
    fn process(&mut self, input: f32, coeff: f32) -> f32 {
        self.delay.process_allpass(input, coeff)
    }

    fn reset(&mut self) {
        self.delay.reset();
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
            .port(
                PortDescriptor::control_input("rate_cv", "Rate CV").description("Rate modulation"),
            )
            .parameter(
                ParameterDescriptor::float(
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
        let phase_inc = self.rate.as_f32() / self.sample_rate.as_f32();

        // Process stereo interleaved
        let channels = 2;
        for frame in 0..context.samples.as_usize() {
            let idx_l = frame * channels;
            let idx_r = frame * channels + 1;

            let in_l = if idx_l < input.len() {
                input[idx_l]
            } else {
                0.0
            };
            let in_r = if idx_r < input.len() {
                input[idx_r]
            } else {
                in_l
            };

            // Calculate LFO value (sine wave)
            let lfo = (self.lfo_phase.as_f32() * std::f32::consts::TAU).sin();
            self.lfo_phase = Phase::new((self.lfo_phase.as_f32() + phase_inc).rem_euclid(1.0));

            // Calculate modulated frequency
            let depth = self.depth.as_f32();
            let freq_mod_hz = self.center_freq.as_f32() * (1.0 + lfo * depth * 0.8);
            let freq_mod = Hertz::new(freq_mod_hz.clamp(100.0, self.sample_rate.as_f32() * 0.45));

            // Calculate coefficient for all-pass filters
            let coeff = self.freq_to_coeff(freq_mod);

            // Process left channel through all-pass cascade
            let feedback = self.feedback.as_f32();
            let mut sample_l = in_l + self.feedback_l * feedback;
            for stage in &mut self.stages_l {
                sample_l = stage.process(sample_l, coeff);
            }
            self.feedback_l = sample_l;

            // Process right channel (slightly offset phase for stereo width)
            let mut sample_r = in_r + self.feedback_r * feedback;
            for stage in &mut self.stages_r {
                sample_r = stage.process(sample_r, coeff);
            }
            self.feedback_r = sample_r;

            // Mix dry/wet
            let mix = self.mix.as_f32();
            if idx_l < output.len() {
                output[idx_l] = in_l * (1.0 - mix) + sample_l * mix;
            }
            if idx_r < output.len() {
                output[idx_r] = in_r * (1.0 - mix) + sample_r * mix;
            }
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
