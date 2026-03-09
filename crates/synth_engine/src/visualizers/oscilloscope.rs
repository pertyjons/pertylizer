//! Oscilloscope visualization module.
//!
//! Displays the waveform in time domain. Pass-through module that
//! captures samples for display without modifying the signal.

use synth_core::*;
use synth_core::{BipolarValue, Gain, NormalizedValue, SampleCount, SampleRate, Seconds};
use synth_core::{ModuleType, OscilloscopeParam, Param};

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
    /// Previous left sample for rising-edge detection.
    prev_sample_l: BipolarValue,
    /// Whether we are currently capturing a triggered sweep.
    triggered: bool,
    /// How many samples have been captured in the current sweep.
    capture_count: SampleCount,
    /// Total samples to capture per sweep (time_scale * sample_rate).
    display_samples: SampleCount,
}

impl Oscilloscope {
    pub fn new() -> Self {
        let sample_rate = SampleRate::DVD_QUALITY;
        let time_scale = Seconds::new(0.01);
        let display_samples =
            SampleCount::new((time_scale.as_f32() * sample_rate.as_f32()) as usize);

        Self {
            buffer: VisualizationBuffer::new(4096),
            time_scale,
            gain: Gain::UNITY,
            trigger_level: NormalizedValue::CENTER,
            frozen: false,
            mix: NormalizedValue::MAX,
            sample_rate,
            prev_sample_l: BipolarValue::CENTER,
            triggered: false,
            capture_count: SampleCount::ZERO,
            display_samples,
        }
    }

    /// Recalculate display_samples from time_scale and sample_rate.
    fn update_display_samples(&mut self) {
        let samples = (self.time_scale.as_f32() * self.sample_rate.as_f32()) as usize;
        self.display_samples = SampleCount::new(samples.max(1));
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
            prev_sample_l: self.prev_sample_l,
            triggered: self.triggered,
            capture_count: self.capture_count,
            display_samples: self.display_samples,
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
                    "time",
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
                    "gain",
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
                    "trig",
                    Param::Oscilloscope(OscilloscopeParam::Trigger(NormalizedValue::CENTER)),
                    "Trig",
                )
                .description("Trigger level")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
        // No ports - visualizers receive the final output signal automatically
    }
}

impl AudioEffect for Oscilloscope {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;

        // Pass-through: copy input to output
        output.copy_from_slice(input);

        // Capture samples for visualization with trigger detection (unless frozen)
        if !self.frozen {
            let threshold = self.trigger_level.as_f32() * 2.0 - 1.0;
            let gain = self.gain.as_f32();
            let num_frames = input.len() / 2;

            // Use fixed-size stack buffer to avoid heap allocation
            let mut left_buf = [0.0_f32; 256];
            let mut right_buf = [0.0_f32; 256];
            let mut buf_pos = 0_usize;

            for i in 0..num_frames {
                let left = input[i * 2];

                if !self.triggered {
                    let prev = self.prev_sample_l.as_f32();
                    if prev < threshold && left >= threshold {
                        self.triggered = true;
                        self.capture_count = SampleCount::ZERO;
                    }
                }

                if self.triggered {
                    if self.capture_count.as_usize() < self.display_samples.as_usize() {
                        left_buf[buf_pos] = left * gain;
                        right_buf[buf_pos] = input[i * 2 + 1] * gain;
                        buf_pos += 1;
                        self.capture_count = SampleCount::new(self.capture_count.as_usize() + 1);

                        if buf_pos >= left_buf.len() {
                            self.buffer
                                .write_samples(&left_buf[..buf_pos], &right_buf[..buf_pos]);
                            buf_pos = 0;
                        }
                    } else {
                        self.triggered = false;
                    }
                }

                self.prev_sample_l = BipolarValue::new(left);
            }

            // Flush remaining samples
            if buf_pos > 0 {
                self.buffer
                    .write_samples(&left_buf[..buf_pos], &right_buf[..buf_pos]);
            }
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Oscilloscope(osc_param) = param {
            match osc_param {
                OscilloscopeParam::Time(t) => {
                    self.time_scale = t;
                    self.update_display_samples();
                }
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
        self.prev_sample_l = BipolarValue::CENTER;
        self.triggered = false;
        self.capture_count = SampleCount::ZERO;
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
