//! Delay effect module.
//!
//! Features:
//! - Stereo delay with independent L/R times
//! - Tempo sync option
//! - Ping-pong mode
//! - Feedback with soft limiting
//! - Modulation input for chorus-like effects

use crate::engine::typed_params::{TypedParam, TypedValue, DelayParam, ModuleType, DelayMode};
use crate::modules::core::*;

/// Maximum delay time in seconds.
const MAX_DELAY_SECONDS: f32 = 2.0;

// DelayMode is imported from typed_params via core
// Add to_choices helper for UI compatibility
impl DelayMode {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| ChoiceOption::new(m.id(), m.name()))
            .collect()
    }
}

/// Stereo delay effect.
pub struct Delay {
    // Parameters
    mode: DelayMode,
    time_left: f32,    // Seconds
    time_right: f32,   // Seconds (for stereo mode)
    feedback: f32,     // 0-1
    mix: f32,          // 0-1
    high_cut: f32,     // Hz (for feedback path)
    
    // Delay buffers
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    write_pos: usize,
    
    // Filter state for feedback
    filter_left: f32,
    filter_right: f32,
    
    // State
    sample_rate: f32,
}

impl Delay {
    pub fn new() -> Self {
        let buffer_size = (MAX_DELAY_SECONDS * 48000.0) as usize;
        Self {
            mode: DelayMode::Mono,
            time_left: 0.375,
            time_right: 0.5,
            feedback: 0.4,
            mix: 0.5,
            high_cut: 8000.0,
            buffer_left: vec![0.0; buffer_size],
            buffer_right: vec![0.0; buffer_size],
            write_pos: 0,
            filter_left: 0.0,
            filter_right: 0.0,
            sample_rate: 48000.0,
        }
    }

    /// Resize buffers for new sample rate.
    fn resize_buffers(&mut self) {
        let size = (MAX_DELAY_SECONDS * self.sample_rate) as usize;
        if self.buffer_left.len() != size {
            self.buffer_left.resize(size, 0.0);
            self.buffer_right.resize(size, 0.0);
            self.write_pos = 0;
        }
    }

    /// Read from delay buffer with linear interpolation.
    #[inline]
    fn read_interpolated(buffer: &[f32], write_pos: usize, delay_samples: f32) -> f32 {
        let len = buffer.len();
        let read_pos = (write_pos as f32 - delay_samples).rem_euclid(len as f32);
        let idx0 = (read_pos as usize) % len;  // Ensure within bounds
        let idx1 = (idx0 + 1) % len;
        let frac = read_pos - read_pos.floor();
        
        buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac
    }

    /// One-pole lowpass for feedback damping.
    #[inline]
    fn lowpass(input: f32, state: &mut f32, cutoff: f32, sample_rate: f32) -> f32 {
        let coef = (-std::f32::consts::TAU * cutoff / sample_rate).exp();
        *state = input * (1.0 - coef) + *state * coef;
        *state
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Delay {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("delay", "Delay")
            .description("Stereo delay with ping-pong mode")
            .category(ModuleCategory::Effect)
            .tag("delay")
            .tag("effect")
            .tag("time")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .port(PortDescriptor::control_input("time_cv", "Time CV").description("Delay time modulation"))
            .parameter(
                ParameterDescriptor::choice(TypedParam::Delay(DelayParam::Mode), "Mode", DelayMode::to_choices())
                    .description("Delay mode"),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Delay(DelayParam::Time), "Time")
                    .description("Delay time")
                    .range(0.001, MAX_DELAY_SECONDS)
                    .default(0.375)
                    .unit(ParameterUnit::Seconds)
                    .widget(WidgetHint::TimeSlider)
                    .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Delay(DelayParam::Feedback), "Feedback")
                    .description("Feedback amount")
                    .range(0.0, 0.95)
                    .default(0.4)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Delay(DelayParam::Mix), "Mix")
                    .description("Dry/wet mix")
                    .range(0.0, 1.0)
                    .default(0.5)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Delay(DelayParam::Damping), "Tone")
                    .description("Feedback high-cut filter")
                    .range(200.0, 20000.0)
                    .default(8000.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::FrequencySlider)
                    .curve(ResponseCurve::Logarithmic),
            )
    }
}

impl EffectModule for Delay {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        self.resize_buffers();

        let delay_samples_left = (self.time_left * self.sample_rate).min((self.buffer_left.len() - 1) as f32);
        let delay_samples_right = (self.time_right * self.sample_rate).min((self.buffer_right.len() - 1) as f32);

        let len = self.buffer_left.len();

        // Process assuming interleaved stereo
        let channels = 2;
        for frame in 0..context.samples {
            let idx_l = frame * channels;
            let idx_r = frame * channels + 1;
            
            let in_l = if idx_l < input.len() { input[idx_l] } else { 0.0 };
            let in_r = if idx_r < input.len() { input[idx_r] } else { in_l };

            // Read from delay buffers
            let delayed_l = Self::read_interpolated(&self.buffer_left, self.write_pos, delay_samples_left);
            let delayed_r = Self::read_interpolated(&self.buffer_right, self.write_pos, delay_samples_right);

            // Apply feedback filtering
            let fb_l = Self::lowpass(delayed_l, &mut self.filter_left, self.high_cut, self.sample_rate);
            let fb_r = Self::lowpass(delayed_r, &mut self.filter_right, self.high_cut, self.sample_rate);

            // Calculate feedback signal based on mode
            let (write_l, write_r) = match self.mode {
                DelayMode::Mono => {
                    let mono_in = (in_l + in_r) * 0.5;
                    let mono_fb = (fb_l + fb_r) * 0.5;
                    let write = mono_in + mono_fb * self.feedback;
                    (write, write)
                }
                DelayMode::Stereo => {
                    (
                        in_l + fb_l * self.feedback,
                        in_r + fb_r * self.feedback,
                    )
                }
                DelayMode::PingPong => {
                    // Cross-feed: left feedback goes to right, vice versa
                    (
                        in_l + fb_r * self.feedback,
                        in_r + fb_l * self.feedback,
                    )
                }
            };

            // Soft limit feedback to prevent runaway
            let write_l = write_l.tanh();
            let write_r = write_r.tanh();

            // Write to delay buffers
            self.buffer_left[self.write_pos] = write_l;
            self.buffer_right[self.write_pos] = write_r;

            // Advance write position
            self.write_pos = (self.write_pos + 1) % len;

            // Mix dry/wet
            if idx_l < output.len() {
                output[idx_l] = in_l * (1.0 - self.mix) + delayed_l * self.mix;
            }
            if idx_r < output.len() {
                output[idx_r] = in_r * (1.0 - self.mix) + delayed_r * self.mix;
            }
        }
    }

    fn reset(&mut self) {
        self.buffer_left.fill(0.0);
        self.buffer_right.fill(0.0);
        self.filter_left = 0.0;
        self.filter_right = 0.0;
        self.write_pos = 0;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    fn get_mix(&self) -> f32 {
        self.mix
    }

    fn tail_samples(&self) -> usize {
        // Estimate tail based on feedback
        let decay_time = self.time_left * (1.0 / (1.0 - self.feedback)).ln() / 3.0;
        (decay_time * self.sample_rate) as usize
    }
    
    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Delay(delay_param) = param {
            match delay_param {
                DelayParam::Mode => {
                    if let TypedValue::DelayMode(mode) = value {
                        // Direct assignment - same type now!
                        self.mode = mode;
                    }
                }
                DelayParam::Time => {
                    if let Some(t) = value.as_float() {
                        self.time_left = t.clamp(0.001, MAX_DELAY_SECONDS);
                        self.time_right = t.clamp(0.001, MAX_DELAY_SECONDS);
                    }
                }
                DelayParam::TimeLeft => {
                    if let Some(t) = value.as_float() {
                        self.time_left = t.clamp(0.001, MAX_DELAY_SECONDS);
                    }
                }
                DelayParam::TimeRight => {
                    if let Some(t) = value.as_float() {
                        self.time_right = t.clamp(0.001, MAX_DELAY_SECONDS);
                    }
                }
                DelayParam::Feedback => {
                    if let Some(f) = value.as_float() {
                        self.feedback = f.clamp(0.0, 0.95);
                    }
                }
                DelayParam::Mix => {
                    if let Some(m) = value.as_float() {
                        self.mix = m.clamp(0.0, 1.0);
                    }
                }
                DelayParam::Damping => {
                    if let Some(d) = value.as_float() {
                        self.high_cut = d.clamp(200.0, 20000.0);
                    }
                }
                DelayParam::TempoSync => {
                    // Not yet implemented
                }
            }
        }
    }
    
    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Delay(delay_param) = param {
            match delay_param {
                DelayParam::Mode => {
                    // Direct use - same type now!
                    Some(TypedValue::DelayMode(self.mode))
                }
                DelayParam::Time => Some(TypedValue::Float(self.time_left)),
                DelayParam::TimeLeft => Some(TypedValue::Float(self.time_left)),
                DelayParam::TimeRight => Some(TypedValue::Float(self.time_right)),
                DelayParam::Feedback => Some(TypedValue::Float(self.feedback)),
                DelayParam::Mix => Some(TypedValue::Float(self.mix)),
                DelayParam::Damping => Some(TypedValue::Float(self.high_cut)),
                DelayParam::TempoSync => None,
            }
        } else {
            None
        }
    }
    
    fn module_type(&self) -> ModuleType {
        ModuleType::Delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_creation() {
        let delay = Delay::new();
        assert_eq!(delay.mode, DelayMode::Mono);
        assert!((delay.time_left - 0.375).abs() < 0.001);
    }

    #[test]
    fn test_delay_no_explosion() {
        let mut delay = Delay::new();
        delay.sample_rate = 48000.0;
        delay.feedback = 0.9;
        delay.resize_buffers();

        let context = ProcessContext {
            sample_rate: 48000.0,
            samples: 256,
            ..Default::default()
        };

        let input = vec![1.0; 512];
        let mut output = vec![0.0; 512];

        // Process many times
        for _ in 0..100 {
            delay.process(&input, &mut output, &context);
        }

        // Check output is bounded
        for sample in &output {
            assert!(sample.abs() < 10.0, "Delay output exploded");
        }
    }
}
