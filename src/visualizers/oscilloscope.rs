//! Oscilloscope visualization module.
//!
//! Displays the waveform in time domain. Pass-through module that
//! captures samples for display without modifying the signal.

use crate::engine::typed_params::{ModuleType, OscilloscopeParam, Param};
use crate::modules::core::*;
use crate::types::{Gain, NormalizedValue, SampleRate, Seconds};

use super::VisualizationBuffer;

/// Oscilloscope module for waveform visualization.
pub struct Oscilloscope {
    /// Shared buffer for GUI display.
    buffer: VisualizationBuffer,
    /// Time scale (samples to display).
    time_scale: Seconds,
    /// Vertical gain.
    gain: Gain,
    /// Trigger level for stable display.
    trigger_level: NormalizedValue,
    /// Is the display frozen.
    frozen: bool,
    /// Wet/dry mix (always 1.0 for pass-through).
    mix: NormalizedValue,
    /// Sample rate.
    sample_rate: SampleRate,
}

impl Oscilloscope {
    pub fn new() -> Self {
        Self {
            buffer: VisualizationBuffer::new(4096),
            time_scale: Seconds::new(0.01),
            gain: Gain::UNITY,
            trigger_level: NormalizedValue::CENTER,
            frozen: false,
            mix: NormalizedValue::MAX,
            sample_rate: SampleRate::DVD_QUALITY,
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
                ParameterDescriptor::float(
                    Param::Oscilloscope(OscilloscopeParam::Time(Seconds::new(0.01))),
                    "Time",
                )
                .description("Time scale (zoom)")
                .range(0.001, 0.1)
                .default(0.01)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Oscilloscope(OscilloscopeParam::Gain(Gain::UNITY)),
                    "Gain",
                )
                .description("Vertical gain")
                .range(0.1, 10.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Oscilloscope(OscilloscopeParam::Trigger(NormalizedValue::CENTER)),
                    "Trig",
                )
                .description("Trigger level")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(
                PortDescriptor::audio_output("out_l", "Out L")
                    .description("Left output (pass-through)"),
            )
            .port(
                PortDescriptor::audio_output("out_r", "Out R")
                    .description("Right output (pass-through)"),
            )
    }
}

impl AudioEffect for Oscilloscope {
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
                left_samples.push(input[i * 2] * self.gain.as_f32());
                right_samples.push(input[i * 2 + 1] * self.gain.as_f32());
            }

            self.buffer.write_samples(&left_samples, &right_samples);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Oscilloscope(osc_param) = param {
            match osc_param {
                OscilloscopeParam::Time(t) => self.time_scale = t,
                OscilloscopeParam::Gain(g) => self.gain = g,
                OscilloscopeParam::Trigger(t) => self.trigger_level = t,
                OscilloscopeParam::Frozen(f) => self.frozen = f,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Oscilloscope(osc_param) = param {
            Some(match osc_param {
                OscilloscopeParam::Time(_) => self.time_scale.as_f32(),
                OscilloscopeParam::Gain(_) => self.gain.as_f32(),
                OscilloscopeParam::Trigger(_) => self.trigger_level.as_f32(),
                OscilloscopeParam::Frozen(_) => {
                    if self.frozen {
                        1.0
                    } else {
                        0.0
                    }
                }
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Oscilloscope(OscilloscopeParam::Time(self.time_scale)),
            Param::Oscilloscope(OscilloscopeParam::Gain(self.gain)),
            Param::Oscilloscope(OscilloscopeParam::Trigger(self.trigger_level)),
            Param::Oscilloscope(OscilloscopeParam::Frozen(self.frozen)),
        ]
    }

    fn reset(&mut self) {
        self.buffer = VisualizationBuffer::new(4096);
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        SampleCount::ZERO // No tail - pass-through
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Oscilloscope
    }
}
