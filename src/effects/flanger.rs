//! Flanger effect with LFO-modulated delay.

use crate::engine::typed_params::{FlangerParam, ModuleType, TypedParam, TypedValue};
use crate::modules::{
    Describable, EffectModule, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PortDescriptor, ProcessContext, WidgetHint,
};

/// Maximum delay time in milliseconds.
const MAX_DELAY_MS: f32 = 20.0;

/// Flanger effect with modulated delay line and feedback.
pub struct Flanger {
    // Parameters
    rate: f32,     // LFO rate in Hz
    depth: f32,    // Modulation depth 0-1
    feedback: f32, // Feedback amount -1 to 1
    delay: f32,    // Base delay time in ms
    mix: f32,      // Dry/wet mix 0-1

    // Delay buffers (stereo)
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,

    // LFO state
    lfo_phase: f32,

    // Feedback state
    feedback_l: f32,
    feedback_r: f32,

    // State
    sample_rate: f32,
}

impl Flanger {
    pub fn new() -> Self {
        Self {
            rate: 0.3,
            depth: 0.7,
            feedback: 0.7,
            delay: 2.0,
            mix: 0.5,
            buffer_l: vec![0.0; 4800], // Will be resized
            buffer_r: vec![0.0; 4800],
            write_pos: 0,
            lfo_phase: 0.0,
            feedback_l: 0.0,
            feedback_r: 0.0,
            sample_rate: 48000.0,
        }
    }

    fn resize_buffers(&mut self) {
        let size = (MAX_DELAY_MS / 1000.0 * self.sample_rate) as usize + 1;
        if self.buffer_l.len() != size {
            self.buffer_l.resize(size, 0.0);
            self.buffer_r.resize(size, 0.0);
            self.write_pos = 0;
        }
    }

    /// Read from delay buffer with linear interpolation.
    #[inline]
    fn read_interpolated(buffer: &[f32], write_pos: usize, delay_samples: f32) -> f32 {
        let len = buffer.len();
        let read_pos = (write_pos as f32 - delay_samples).rem_euclid(len as f32);
        let idx0 = read_pos as usize % len;
        let idx1 = (idx0 + 1) % len;
        let frac = read_pos - read_pos.floor();
        buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac
    }
}

impl Default for Flanger {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Flanger {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("flanger", "Flanger")
            .description("Classic flanger effect with feedback")
            .category(ModuleCategory::Effect)
            .tag("flanger")
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
                ParameterDescriptor::float(TypedParam::Flanger(FlangerParam::Rate), "Rate")
                    .description("LFO rate")
                    .range(0.05, 5.0)
                    .default(0.3)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Flanger(FlangerParam::Depth), "Depth")
                    .description("Modulation depth")
                    .range(0.0, 1.0)
                    .default(0.7)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Flanger(FlangerParam::Feedback), "Feedback")
                    .description("Feedback amount")
                    .range(-0.95, 0.95)
                    .default(0.7)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Flanger(FlangerParam::Delay), "Delay")
                    .description("Base delay time")
                    .range(0.1, 10.0)
                    .default(2.0)
                    .unit(ParameterUnit::Milliseconds)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Flanger(FlangerParam::Mix), "Mix")
                    .description("Dry/wet mix")
                    .range(0.0, 1.0)
                    .default(0.5)
                    .widget(WidgetHint::Knob),
            )
    }
}

impl EffectModule for Flanger {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        self.resize_buffers();

        let phase_inc = self.rate / self.sample_rate;
        let max_mod_ms = self.delay.min(MAX_DELAY_MS - self.delay);

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

            // Calculate LFO value (triangle wave for smoother flanging)
            let lfo = if self.lfo_phase < 0.5 {
                self.lfo_phase * 4.0 - 1.0
            } else {
                3.0 - self.lfo_phase * 4.0
            };
            self.lfo_phase = (self.lfo_phase + phase_inc).rem_euclid(1.0);

            // Calculate modulated delay time
            let delay_ms = self.delay + lfo * max_mod_ms * self.depth;
            let delay_samples = (delay_ms / 1000.0 * self.sample_rate).max(1.0);

            // Write input + feedback to delay buffer
            self.buffer_l[self.write_pos] = in_l + self.feedback_l * self.feedback;
            self.buffer_r[self.write_pos] = in_r + self.feedback_r * self.feedback;

            // Read from delay with interpolation
            let wet_l = Self::read_interpolated(&self.buffer_l, self.write_pos, delay_samples);
            let wet_r = Self::read_interpolated(&self.buffer_r, self.write_pos, delay_samples);

            // Store for feedback
            self.feedback_l = wet_l;
            self.feedback_r = wet_r;

            // Advance write position
            self.write_pos = (self.write_pos + 1) % self.buffer_l.len();

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
        self.buffer_l.fill(0.0);
        self.buffer_r.fill(0.0);
        self.write_pos = 0;
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
        if let TypedParam::Flanger(flanger_param) = param {
            match flanger_param {
                FlangerParam::Rate => {
                    if let Some(r) = value.as_float() {
                        self.rate = r.clamp(0.05, 5.0);
                    }
                }
                FlangerParam::Depth => {
                    if let Some(d) = value.as_float() {
                        self.depth = d.clamp(0.0, 1.0);
                    }
                }
                FlangerParam::Feedback => {
                    if let Some(f) = value.as_float() {
                        self.feedback = f.clamp(-0.95, 0.95);
                    }
                }
                FlangerParam::Delay => {
                    if let Some(d) = value.as_float() {
                        self.delay = d.clamp(0.1, 10.0);
                    }
                }
                FlangerParam::Mix => {
                    if let Some(m) = value.as_float() {
                        self.mix = m.clamp(0.0, 1.0);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Flanger(flanger_param) = param {
            match flanger_param {
                FlangerParam::Rate => Some(TypedValue::Float(self.rate)),
                FlangerParam::Depth => Some(TypedValue::Float(self.depth)),
                FlangerParam::Feedback => Some(TypedValue::Float(self.feedback)),
                FlangerParam::Delay => Some(TypedValue::Float(self.delay)),
                FlangerParam::Mix => Some(TypedValue::Float(self.mix)),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Flanger
    }
}
