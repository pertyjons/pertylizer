//! Phaser effect using cascaded all-pass filters.

use crate::engine::typed_params::{ModuleType, PhaserParam, TypedParam, TypedValue};
use crate::modules::{
    Describable, EffectModule, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PortDescriptor, ProcessContext, WidgetHint,
};

/// Number of all-pass filter stages (fixed at 6 for classic phaser sound).
const NUM_STAGES: usize = 6;

/// A single first-order all-pass filter stage.
#[derive(Clone, Copy, Default)]
struct AllPassStage {
    delay: f32,
}

impl AllPassStage {
    /// Process a sample through the all-pass filter.
    /// First-order all-pass: y[n] = a * (x[n] - y[n-1]) + x[n-1]
    /// The coefficient determines the center frequency of the phase shift.
    #[inline]
    fn process(&mut self, input: f32, coeff: f32) -> f32 {
        let output = coeff * (input - self.delay) + self.delay;
        self.delay = output;
        output
    }

    fn reset(&mut self) {
        self.delay = 0.0;
    }
}

/// Phaser effect with LFO-modulated all-pass filter stages.
pub struct Phaser {
    // Parameters
    rate: f32,        // LFO rate in Hz
    depth: f32,       // Modulation depth 0-1
    feedback: f32,    // Feedback amount -1 to 1
    center_freq: f32, // Center frequency in Hz
    mix: f32,         // Dry/wet mix 0-1

    // All-pass filter stages (stereo)
    stages_l: [AllPassStage; NUM_STAGES],
    stages_r: [AllPassStage; NUM_STAGES],

    // LFO state
    lfo_phase: f32,

    // Feedback state
    feedback_l: f32,
    feedback_r: f32,

    // State
    sample_rate: f32,
}

impl Phaser {
    pub fn new() -> Self {
        Self {
            rate: 0.5,
            depth: 0.7,
            feedback: 0.7,
            center_freq: 1000.0,
            mix: 0.5,
            stages_l: [AllPassStage::default(); NUM_STAGES],
            stages_r: [AllPassStage::default(); NUM_STAGES],
            lfo_phase: 0.0,
            feedback_l: 0.0,
            feedback_r: 0.0,
            sample_rate: 48000.0,
        }
    }

    /// Calculate all-pass coefficient from frequency.
    #[inline]
    fn freq_to_coeff(&self, freq: f32) -> f32 {
        let tan_val = (std::f32::consts::PI * freq / self.sample_rate).tan();
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
                ParameterDescriptor::float(TypedParam::Phaser(PhaserParam::Rate), "Rate")
                    .description("LFO rate")
                    .range(0.05, 5.0)
                    .default(0.5)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Phaser(PhaserParam::Depth), "Depth")
                    .description("Modulation depth")
                    .range(0.0, 1.0)
                    .default(0.7)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Phaser(PhaserParam::Feedback), "Feedback")
                    .description("Feedback amount")
                    .range(-0.95, 0.95)
                    .default(0.7)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    TypedParam::Phaser(PhaserParam::CenterFreq),
                    "Center Freq",
                )
                .description("Center frequency")
                .range(100.0, 4000.0)
                .default(1000.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Phaser(PhaserParam::Mix), "Mix")
                    .description("Dry/wet mix")
                    .range(0.0, 1.0)
                    .default(0.5)
                    .widget(WidgetHint::Knob),
            )
    }
}

impl EffectModule for Phaser {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        let phase_inc = self.rate / self.sample_rate;

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

            // Calculate LFO value (sine wave)
            let lfo = (self.lfo_phase * std::f32::consts::TAU).sin();
            self.lfo_phase = (self.lfo_phase + phase_inc).rem_euclid(1.0);

            // Calculate modulated frequency
            let freq_mod = self.center_freq * (1.0 + lfo * self.depth * 0.8);
            let freq_mod = freq_mod.clamp(100.0, self.sample_rate * 0.45);

            // Calculate coefficient for all-pass filters
            let coeff = self.freq_to_coeff(freq_mod);

            // Process left channel through all-pass cascade
            let mut sample_l = in_l + self.feedback_l * self.feedback;
            for stage in &mut self.stages_l {
                sample_l = stage.process(sample_l, coeff);
            }
            self.feedback_l = sample_l;

            // Process right channel (slightly offset phase for stereo width)
            let mut sample_r = in_r + self.feedback_r * self.feedback;
            for stage in &mut self.stages_r {
                sample_r = stage.process(sample_r, coeff);
            }
            self.feedback_r = sample_r;

            // Mix dry/wet
            if idx_l < output.len() {
                output[idx_l] = in_l * (1.0 - self.mix) + sample_l * self.mix;
            }
            if idx_r < output.len() {
                output[idx_r] = in_r * (1.0 - self.mix) + sample_r * self.mix;
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
        self.lfo_phase = 0.0;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    fn get_mix(&self) -> f32 {
        self.mix
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Phaser(phaser_param) = param {
            match phaser_param {
                PhaserParam::Rate => {
                    if let Some(r) = value.as_float() {
                        self.rate = r.clamp(0.05, 5.0);
                    }
                }
                PhaserParam::Depth => {
                    if let Some(d) = value.as_float() {
                        self.depth = d.clamp(0.0, 1.0);
                    }
                }
                PhaserParam::Feedback => {
                    if let Some(f) = value.as_float() {
                        self.feedback = f.clamp(-0.95, 0.95);
                    }
                }
                PhaserParam::Stages => {
                    // Fixed at 6 stages, ignore
                }
                PhaserParam::CenterFreq => {
                    if let Some(f) = value.as_float() {
                        self.center_freq = f.clamp(100.0, 4000.0);
                    }
                }
                PhaserParam::Mix => {
                    if let Some(m) = value.as_float() {
                        self.mix = m.clamp(0.0, 1.0);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Phaser(phaser_param) = param {
            match phaser_param {
                PhaserParam::Rate => Some(TypedValue::Float(self.rate)),
                PhaserParam::Depth => Some(TypedValue::Float(self.depth)),
                PhaserParam::Feedback => Some(TypedValue::Float(self.feedback)),
                PhaserParam::Stages => Some(TypedValue::Int(NUM_STAGES as i32)),
                PhaserParam::CenterFreq => Some(TypedValue::Float(self.center_freq)),
                PhaserParam::Mix => Some(TypedValue::Float(self.mix)),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Phaser
    }
}
