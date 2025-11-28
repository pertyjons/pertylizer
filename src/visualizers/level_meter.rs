//! Level Meter visualization module.
//!
//! Displays audio levels with peak hold and RMS.
//! Pass-through module that measures levels without modifying the signal.

use crate::engine::typed_params::{ModuleType, TypedParam, TypedValue, LevelMeterParam};
use crate::modules::core::*;

use super::VisualizationBuffer;

/// Level meter module for audio level visualization.
pub struct LevelMeter {
    /// Shared buffer for GUI display.
    buffer: VisualizationBuffer,
    /// Peak hold time in seconds.
    peak_hold_time: f32,
    /// Show RMS or peak.
    show_rms: bool,
    /// Wet/dry mix (always 1.0 for pass-through).
    mix: f32,
    /// Sample rate.
    sample_rate: f32,
}

impl LevelMeter {
    pub fn new() -> Self {
        Self {
            buffer: VisualizationBuffer::new(1024),
            peak_hold_time: 1.0,
            show_rms: true,
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

impl Default for LevelMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for LevelMeter {
    fn clone(&self) -> Self {
        Self {
            buffer: self.buffer.clone(),
            peak_hold_time: self.peak_hold_time,
            show_rms: self.show_rms,
            mix: self.mix,
            sample_rate: self.sample_rate,
        }
    }
}

impl Describable for LevelMeter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("level_meter", "Meter")
            .description("Audio level meter with peak hold")
            .category(ModuleCategory::Visualizer)
            .tag("visualizer")
            .tag("meter")
            .tag("level")
            .parameter(
                ParameterDescriptor::float(TypedParam::LevelMeter(LevelMeterParam::PeakHold), "Hold")
                    .description("Peak hold time in seconds")
                    .range(0.1, 5.0)
                    .default(1.0)
                    .unit(ParameterUnit::Seconds)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output (pass-through)"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output (pass-through)"))
    }
}

impl EffectModule for LevelMeter {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;

        // Pass-through: copy input to output
        output.copy_from_slice(input);

        // Extract L/R channels and update levels
        let num_frames = input.len() / 2;
        let mut left_samples = Vec::with_capacity(num_frames);
        let mut right_samples = Vec::with_capacity(num_frames);
        
        for i in 0..num_frames {
            left_samples.push(input[i * 2]);
            right_samples.push(input[i * 2 + 1]);
        }
        
        // Update visualization buffer
        self.buffer.update_levels(&left_samples, &right_samples);
        self.buffer.write_samples(&left_samples, &right_samples);
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::LevelMeter(meter_param) = param {
            match meter_param {
                LevelMeterParam::PeakHold => {
                    if let Some(t) = value.as_float() {
                        self.peak_hold_time = t.clamp(0.1, 5.0);
                    }
                }
                LevelMeterParam::ShowRms => {
                    if let TypedValue::Bool(r) = value {
                        self.show_rms = r;
                    }
                }
                LevelMeterParam::DecayRate => {
                    // Not used currently
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::LevelMeter(meter_param) = param {
            match meter_param {
                LevelMeterParam::PeakHold => Some(TypedValue::Float(self.peak_hold_time)),
                LevelMeterParam::ShowRms => Some(TypedValue::Bool(self.show_rms)),
                LevelMeterParam::DecayRate => None, // Not used currently
            }
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.buffer.reset_peaks();
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
        ModuleType::LevelMeter
    }
}
