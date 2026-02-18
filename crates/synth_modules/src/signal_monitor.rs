//! Signal Monitor module.
//!
//! A pass-through voice module that captures audio samples for
//! visualization without modifying the signal. Can be inserted
//! anywhere in the voice graph to inspect the waveform.
//!
//! Uses rising-edge trigger detection for stable waveform display.
//! Only one voice at a time can write to the visualization buffer —
//! a shared atomic generation counter ensures that when a new voice
//! triggers, the previous writer detects the generation mismatch
//! and yields. This prevents garbled multi-voice overlap.
//!
//! The visualization buffer is injected from the GUI layer via
//! `set_vis_sink()` since this crate cannot depend on `synth_engine`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use synth_core::{
    AudioBuffer, BipolarValue, Describable, Gain, InputPorts, ModuleCategory, ModuleDescriptor,
    ModuleType, NormalizedValue, Param, ParameterDescriptor, PolyModule, PortDescriptor, PortName,
    ProcessContext, SampleCount, SampleRate, Seconds, SignalMonitorParam, VisualizationSink,
    WidgetHint,
};

/// Signal Monitor — inline waveform visualizer for the voice graph.
///
/// Pass-through module: copies input to output without modification.
/// Captures gain-scaled samples to a shared visualization sink for GUI display.
/// Uses rising-edge trigger detection for a stable oscilloscope view.
///
/// **Polyphony handling:** All voice clones share a `sweep_generation` counter.
/// When a voice triggers a new sweep it bumps the generation and records
/// the new value locally. On every process call, the writing voice checks
/// whether the shared generation still matches its own — if another voice
/// bumped it, the old writer stops immediately.
pub struct SignalMonitor {
    /// Shared visualization sink (injected from GUI layer via Arc).
    /// All voice clones share the same sink via Arc.
    vis_sink: Option<Arc<dyn VisualizationSink>>,
    /// Shared sweep generation counter — incremented each time any voice
    /// starts a new sweep. The voice whose `my_sweep_gen` matches the
    /// current value is the sole writer.
    sweep_generation: Arc<AtomicU32>,
    /// The generation this voice claimed when it started its current sweep.
    /// Compared against `sweep_generation` to detect takeover.
    my_sweep_gen: u32,
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
}

impl SignalMonitor {
    #[must_use]
    pub fn new() -> Self {
        let sample_rate = SampleRate::DVD_QUALITY;
        let time_scale = Seconds::new(0.01);
        let display_samples =
            SampleCount::new((time_scale.as_f32() * sample_rate.as_f32()) as usize);

        Self {
            vis_sink: None,
            sweep_generation: Arc::new(AtomicU32::new(0)),
            my_sweep_gen: 0,
            time_scale,
            gain: Gain::UNITY,
            trigger_level: NormalizedValue::CENTER,
            frozen: false,
            prev_sample: BipolarValue::CENTER,
            triggered: false,
            capture_count: SampleCount::ZERO,
            display_samples,
            sample_rate,
            output_buffer: AudioBuffer::new(256),
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
    }

    /// Check whether this voice still owns the current sweep.
    #[inline]
    fn i_own_sweep(&self) -> bool {
        self.my_sweep_gen == self.sweep_generation.load(Ordering::Relaxed)
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
            // Arc clone: all voice clones share the same sink and generation counter
            vis_sink: self.vis_sink.clone(),
            sweep_generation: self.sweep_generation.clone(),
            // Generation 0 will never match after any voice has triggered
            my_sweep_gen: 0,
            time_scale: self.time_scale,
            gain: self.gain,
            trigger_level: self.trigger_level,
            frozen: self.frozen,
            prev_sample: self.prev_sample,
            triggered: false,
            capture_count: self.capture_count,
            display_samples: self.display_samples,
            sample_rate: self.sample_rate,
            output_buffer: self.output_buffer.clone(),
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
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
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
        if !self.frozen
            && let Some(ref sink) = self.vis_sink
        {
            // Map trigger_level (0.0-1.0) to bipolar range (-1.0 to 1.0)
            let threshold = self.trigger_level.as_f32() * 2.0 - 1.0;
            let gain = self.gain.as_f32();

            // Stack buffer for batching writes (no heap allocation)
            let mut left_buf = [0.0_f32; 256];
            let mut buf_pos = 0_usize;

            for i in 0..num_samples {
                let sample = input.map_or(0.0, |buf| buf[i]);

                if !self.triggered {
                    // Rising-edge detection
                    let prev = self.prev_sample.as_f32();
                    if prev < threshold && sample >= threshold {
                        // Claim the sweep by bumping the shared generation counter.
                        // fetch_add returns the OLD value, so our generation = old + 1.
                        let new_gen = self
                            .sweep_generation
                            .fetch_add(1, Ordering::Relaxed)
                            .wrapping_add(1);
                        self.my_sweep_gen = new_gen;
                        self.triggered = true;
                        self.capture_count = SampleCount::ZERO;
                    }
                }

                if self.triggered && self.i_own_sweep() {
                    if self.capture_count.as_usize() < self.display_samples.as_usize() {
                        left_buf[buf_pos] = sample * gain;
                        buf_pos += 1;
                        self.capture_count = SampleCount::new(self.capture_count.as_usize() + 1);

                        // Flush batch when buffer is full
                        if buf_pos >= left_buf.len() {
                            sink.write_vis_samples(&left_buf[..buf_pos], &left_buf[..buf_pos]);
                            buf_pos = 0;
                        }
                    } else {
                        // Sweep complete
                        self.triggered = false;
                    }
                } else if self.triggered {
                    // Another voice bumped the generation — we lost ownership, stop writing
                    self.triggered = false;
                }

                self.prev_sample = BipolarValue::new(sample);
            }

            // Flush remaining samples (only if we still own the sweep)
            if buf_pos > 0 && self.i_own_sweep() {
                sink.write_vis_samples(&left_buf[..buf_pos], &left_buf[..buf_pos]);
            }
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

        // Create input with a known signal
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
    fn test_sweep_generation_prevents_concurrent_writes() {
        let sm = SignalMonitor::new();
        let cloned = sm.clone();

        // Both share the same generation counter
        assert!(Arc::ptr_eq(&sm.sweep_generation, &cloned.sweep_generation));

        // Initially generation is 0, both have my_sweep_gen = 0
        // After first voice triggers, generation becomes 1
        // so the clone's my_sweep_gen (0) no longer matches
        sm.sweep_generation.fetch_add(1, Ordering::Relaxed);
        assert_eq!(sm.sweep_generation.load(Ordering::Relaxed), 1);
        assert_eq!(cloned.my_sweep_gen, 0);
        assert!(!cloned.i_own_sweep()); // clone lost ownership
    }

    #[test]
    fn test_signal_monitor_clone_shares_sink() {
        let sm = SignalMonitor::new();
        let cloned = sm.clone();
        assert!(sm.vis_sink.is_none());
        assert!(cloned.vis_sink.is_none());
    }
}
