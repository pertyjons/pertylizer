//! Oscilloscope visualization module.
//!
//! Displays the waveform in time domain. Pass-through module that
//! captures samples for display without modifying the signal.

use crate::engine::typed_params::{ModuleType, TypedParam, TypedValue, OscilloscopeParam};
use crate::modules::core::*;

use super::VisualizationBuffer;

/// Oscilloscope module for waveform visualization.
pub struct Oscilloscope {
    /// Shared buffer for GUI display.
    buffer: VisualizationBuffer,
    /// Time scale (samples to display).
    time_scale: f32,
    /// Vertical gain.
    gain: f32,
    /// Trigger level for stable display.
    trigger_level: f32,
    /// Is the display frozen.
    frozen: bool,
    /// Wet/dry mix (always 1.0 for pass-through).
    mix: f32,
    /// Sample rate.
    sample_rate: f32,
}

impl Oscilloscope {
    pub fn new() -> Self {
        Self {
            buffer: VisualizationBuffer::new(4096),
            time_scale: 1.0,
            gain: 1.0,
            trigger_level: 0.0,
            frozen: false,
            mix: 1.0,
            sample_rate: 48000.0,
        }
    }

    /// Get the visualization buffer for GUI access.
    pub fn visualization_buffer(&self) -> &VisualizationBuffer {
        &self.buffer
    }

    /// Get a clone of the visualization buffer for sharing.
    pub fn get_buffer_clone(&self) -> VisualizationBuffer {
        self.buffer.clone()
    }
    
    /// Set the visualization buffer (for sharing with GUI).
    pub fn set_buffer(&mut self, buffer: VisualizationBuffer) {
        self.buffer = buffer;
    }
}

impl Default for Oscilloscope {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Oscilloscope {
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            time_scale: self.time_scale,
            gain: self.gain,
            trigger_level: self.trigger_level,
            frozen: self.frozen,
            mix: self.mix,
            sample_rate: self.sample_rate,
        }
    }
}

impl Describable for Oscilloscope {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("oscilloscope", "Scope")
            .description("Oscilloscope waveform display")
            .category(ModuleCategory::Visualizer)
            .tag("visualizer")
            .tag("scope")
            .tag("waveform")
            .parameter(
                ParameterDescriptor::float(TypedParam::Oscilloscope(OscilloscopeParam::Time), "Time")
                    .description("Time scale (zoom)")
                    .range(0.1, 10.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Oscilloscope(OscilloscopeParam::Gain), "Gain")
                    .description("Vertical gain")
                    .range(0.1, 10.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Oscilloscope(OscilloscopeParam::Trigger), "Trig")
                    .description("Trigger level")
                    .range(-1.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output (pass-through)"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output (pass-through)"))
    }
}

impl EffectModule for Oscilloscope {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;

        // Pass-through: copy input to output
        output.copy_from_slice(input);

        // Capture samples for visualization (unless frozen)
        if !self.frozen {
            // Input is interleaved stereo [L, R, L, R, ...]
            let num_frames = input.len() / 2;
            let mut left_samples = Vec::with_capacity(num_frames);
            let mut right_samples = Vec::with_capacity(num_frames);
            
            for i in 0..num_frames {
                left_samples.push(input[i * 2] * self.gain);
                right_samples.push(input[i * 2 + 1] * self.gain);
            }
            
            self.buffer.write_samples(&left_samples, &right_samples);
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Oscilloscope(osc_param) = param {
            match osc_param {
                OscilloscopeParam::Time => {
                    if let Some(t) = value.as_float() {
                        self.time_scale = t.clamp(0.1, 10.0);
                    }
                }
                OscilloscopeParam::Gain => {
                    if let Some(g) = value.as_float() {
                        self.gain = g.clamp(0.1, 10.0);
                    }
                }
                OscilloscopeParam::Trigger => {
                    if let Some(t) = value.as_float() {
                        self.trigger_level = t.clamp(-1.0, 1.0);
                    }
                }
                OscilloscopeParam::Frozen => {
                    if let TypedValue::Bool(f) = value {
                        self.frozen = f;
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Oscilloscope(osc_param) = param {
            match osc_param {
                OscilloscopeParam::Time => Some(TypedValue::Float(self.time_scale)),
                OscilloscopeParam::Gain => Some(TypedValue::Float(self.gain)),
                OscilloscopeParam::Trigger => Some(TypedValue::Float(self.trigger_level)),
                OscilloscopeParam::Frozen => Some(TypedValue::Bool(self.frozen)),
            }
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.buffer = VisualizationBuffer::new(4096);
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = mix;
    }

    fn get_mix(&self) -> f32 {
        self.mix
    }

    fn tail_samples(&self) -> usize {
        0 // No tail - pass-through
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Oscilloscope
    }
}
