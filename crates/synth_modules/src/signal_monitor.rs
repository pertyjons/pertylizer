//! Signal Monitor module.
//!
//! A pass-through voice module that captures audio samples for
//! visualization without modifying the signal. Can be inserted
//! anywhere in the voice graph to inspect the waveform.
//!
//! Uses rising-edge trigger detection for stable waveform display.
//!
//! **Key design:** Each voice independently captures sweeps into a
//! local pre-allocated buffer. When a sweep completes, it is flushed
//! via `write_sweep()` which uses "newest voice wins" arbitration
//! based on `voice_start_time` from the `ProcessContext`. This avoids
//! the polyphony collision problem where multi-block sweeps were
//! discarded due to generation counter bumps from other voices.
//!
//! The visualization buffer is injected from the GUI layer via
//! `set_vis_sink()` since this crate cannot depend on `synth_engine`.

use std::collections::HashMap;
use std::sync::Arc;

use synth_core::{
    AudioBuffer, BipolarValue, Describable, Gain, InputPorts, ModuleCategory, ModuleDescriptor,
    ModuleType, NormalizedValue, Param, ParameterDescriptor, PolyModule, PortDescriptor, PortName,
    ProcessContext, SampleCount, SampleRate, Seconds, SignalMonitorParam, VisualizationSink,
    WidgetHint,
};

/// Signal Monitor — inline waveform visualizer for the voice graph.
///
/// Pass-through module: copies input to output without modification.
/// Captures samples to a shared visualization sink for GUI display.
/// Uses rising-edge trigger detection for a stable oscilloscope view.
///
/// **Polyphony handling:** Each voice captures sweeps independently.
/// At flush time, `write_sweep()` uses `voice_start_time` for
/// "newest voice wins" arbitration — no shared generation counter needed.
pub struct SignalMonitor {
    /// Shared visualization sink (injected from GUI layer via Arc).
    /// All voice clones share the same sink via Arc.
    vis_sink: Option<Arc<dyn VisualizationSink>>,
    /// Time scale — how many seconds of audio to display.
    time_scale: Seconds,
    /// Vertical gain for the display.
    gain: Gain,
    /// Trigger threshold (0.0–1.0, mapped to bipolar range for detection).
    trigger_level: NormalizedValue,
    /// Freeze the display (stop capturing new samples).
    frozen: bool,
    /// Previous sample value for rising-edge detection.
    prev_sample: BipolarValue,
    /// Whether we are currently capturing a triggered sweep.
    triggered: bool,
    /// How many samples have been captured in the current sweep.
    capture_count: SampleCount,
    /// Total samples to capture per sweep (time_scale * sample_rate).
    display_samples: SampleCount,
    /// Current sample rate.
    sample_rate: SampleRate,
    /// Pre-allocated output buffer.
    output_buffer: AudioBuffer,
    /// Pre-allocated local sweep buffer. Accumulated during a sweep and
    /// flushed to the vis sink only when the sweep completes AND this
    /// voice still owns the current generation. No heap allocation during
    /// process().
    sweep_buffer: Vec<f32>,
}

impl SignalMonitor {
    #[must_use]
    pub fn new() -> Self {
        let sample_rate = SampleRate::DVD_QUALITY;
        let time_scale = Seconds::new(0.01);
        let display_samples = (time_scale.as_f32() * sample_rate.as_f32()) as usize;

        let mut sweep_buffer = Vec::new();
        sweep_buffer.resize(display_samples.max(1), 0.0);

        Self {
            vis_sink: None,
            time_scale,
            gain: Gain::UNITY,
            trigger_level: NormalizedValue::CENTER,
            frozen: false,
            prev_sample: BipolarValue::CENTER,
            triggered: false,
            capture_count: SampleCount::ZERO,
            display_samples: SampleCount::new(display_samples.max(1)),
            sample_rate,
            output_buffer: AudioBuffer::new(1024),
            sweep_buffer,
        }
    }

    /// Inject a visualization sink (called from the GUI layer after creation).
    ///
    /// The sink is typically an `Arc<VisualizationBuffer>` from `synth_engine`,
    /// coerced to `Arc<dyn VisualizationSink>`.
    pub fn set_vis_sink(&mut self, sink: Arc<dyn VisualizationSink>) {
        self.vis_sink = Some(sink);
    }

    /// Recalculate display_samples from time_scale and sample_rate.
    fn update_display_samples(&mut self) {
        let samples = (self.time_scale.as_f32() * self.sample_rate.as_f32()) as usize;
        self.display_samples = SampleCount::new(samples.max(1));
        // Ensure sweep_buffer has capacity (no allocation during process)
        self.sweep_buffer.resize(samples.max(1), 0.0);
    }
}

impl Default for SignalMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SignalMonitor {
    fn clone(&self) -> Self {
        Self {
            // Arc clone: all voice clones share the same sink
            vis_sink: self.vis_sink.clone(),
            time_scale: self.time_scale,
            gain: self.gain,
            trigger_level: self.trigger_level,
            frozen: self.frozen,
            prev_sample: self.prev_sample,
            triggered: false,
            capture_count: SampleCount::ZERO,
            display_samples: self.display_samples,
            sample_rate: self.sample_rate,
            output_buffer: self.output_buffer.clone(),
            sweep_buffer: self.sweep_buffer.clone(),
        }
    }
}

impl Describable for SignalMonitor {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("signal_monitor", "Sig Mon")
            .description("Signal Monitor — inline waveform display (pass-through)")
            .category(ModuleCategory::Utility)
            .tag("visualizer")
            .tag("scope")
            .tag("monitor")
            .tag("utility")
            .parameter(
                ParameterDescriptor::float(
                    Param::SignalMonitor(SignalMonitorParam::Time(Seconds::new(0.01))),
                    "Time",
                )
                .description("Time scale (zoom)")
                .range(0.001, 0.1)
                .default(0.01)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SignalMonitor(SignalMonitorParam::Gain(Gain::UNITY)),
                    "Gain",
                )
                .description("Vertical gain")
                .range(0.1, 10.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::SignalMonitor(SignalMonitorParam::Trigger(NormalizedValue::CENTER)),
                    "Trig",
                )
                .description("Trigger level")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Audio att visa. Koppla in valfri signal för att se vågformen"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Audio output (pass-through)"),
            )
    }
}

impl PolyModule for SignalMonitor {
    #[allow(clippy::too_many_lines)]
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let input = inputs.get(PortName::IN);

        // Pass-through: copy input to output
        for i in 0..num_samples {
            let sample = input.map_or(0.0, |buf| buf[i]);
            self.output_buffer[i] = sample;
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }

        // Capture samples for visualization (unless frozen or no sink)
        if self.frozen || self.vis_sink.is_none() {
            return;
        }

        let threshold = self.trigger_level.as_f32() * 2.0 - 1.0;
        let max_samples = self.display_samples.as_usize();

        for i in 0..num_samples {
            let sample = input.map_or(0.0, |buf| buf[i]);

            if !self.triggered {
                // Rising-edge detection
                let prev = self.prev_sample.as_f32();
                if prev < threshold && sample >= threshold {
                    self.triggered = true;
                    self.capture_count = SampleCount::ZERO;
                }
            }

            if self.triggered {
                let idx = self.capture_count.as_usize();
                if idx < max_samples && idx < self.sweep_buffer.len() {
                    // Store raw sample — gain is applied in the GUI
                    self.sweep_buffer[idx] = sample;
                    self.capture_count = SampleCount::new(idx + 1);
                }

                if self.capture_count.as_usize() >= max_samples {
                    // Sweep complete — flush via write_sweep (newest voice wins)
                    if let Some(ref sink) = self.vis_sink {
                        let count = max_samples.min(self.sweep_buffer.len());
                        sink.write_sweep(
                            &self.sweep_buffer[..count],
                            context.voice_start_time.as_u64(),
                        );
                    }
                    self.triggered = false;
                }
            }

            self.prev_sample = BipolarValue::new(sample);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::SignalMonitor(p) = param {
            match p {
                SignalMonitorParam::Time(t) => {
                    self.time_scale = t;
                    self.update_display_samples();
                }
                SignalMonitorParam::Gain(g) => self.gain = g,
                SignalMonitorParam::Trigger(t) => self.trigger_level = t,
                SignalMonitorParam::Frozen(f) => self.frozen = f,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::SignalMonitor(p) = param {
            Some(match p {
                SignalMonitorParam::Time(_) => self.time_scale.as_f32(),
                SignalMonitorParam::Gain(_) => self.gain.as_f32(),
                SignalMonitorParam::Trigger(_) => self.trigger_level.as_f32(),
                SignalMonitorParam::Frozen(_) => {
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
            Param::SignalMonitor(SignalMonitorParam::Time(self.time_scale)),
            Param::SignalMonitor(SignalMonitorParam::Gain(self.gain)),
            Param::SignalMonitor(SignalMonitorParam::Trigger(self.trigger_level)),
            Param::SignalMonitor(SignalMonitorParam::Frozen(self.frozen)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SignalMonitor
    }

    fn reset(&mut self) {
        self.prev_sample = BipolarValue::CENTER;
        self.triggered = false;
        self.capture_count = SampleCount::ZERO;
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.update_display_samples();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signal_monitor_creation() {
        let sm = SignalMonitor::new();
        assert!((sm.time_scale.as_f32() - 0.01).abs() < 0.001);
        assert!((sm.gain.as_f32() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_signal_monitor_pass_through() {
        let mut sm = SignalMonitor::new();
        let mut outputs = HashMap::new();
        outputs.insert("out".to_string(), AudioBuffer::new(64));

        let mut input_buf = AudioBuffer::new(64);
        for i in 0..64 {
            input_buf[i] = (i as f32) / 64.0;
        }

        let context = ProcessContext {
            samples: SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        let inputs = InputPorts::from_single("in", &input_buf);
        sm.process(inputs, &mut outputs, &context);

        let out = &outputs["out"];
        for i in 0..64 {
            assert!(
                (out[i] - (i as f32) / 64.0).abs() < 0.001,
                "Pass-through failed at sample {}",
                i
            );
        }
    }

    #[test]
    fn test_signal_monitor_params() {
        let mut sm = SignalMonitor::new();
        sm.set_param(Param::SignalMonitor(SignalMonitorParam::Gain(Gain::new(
            2.0,
        ))));
        assert!((sm.gain.as_f32() - 2.0).abs() < 0.01);

        let params = sm.get_params();
        assert_eq!(params.len(), 4);
    }

    #[test]
    fn test_sweep_buffer_preallocated() {
        let sm = SignalMonitor::new();
        // Buffer should be pre-allocated to display_samples size
        assert_eq!(sm.sweep_buffer.len(), sm.display_samples.as_usize());
        assert!(sm.sweep_buffer.len() > 0);
    }

    #[test]
    fn test_signal_monitor_clone_shares_sink() {
        let sm = SignalMonitor::new();
        let cloned = sm.clone();
        assert!(sm.vis_sink.is_none());
        assert!(cloned.vis_sink.is_none());
    }
}
