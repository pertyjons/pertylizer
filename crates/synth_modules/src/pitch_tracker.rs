//! Pitch Tracker module.
//!
//! Detects the fundamental frequency of an audio input using
//! autocorrelation-based pitch detection. Outputs a control voltage
//! proportional to the detected pitch (1V/octave) and a gate signal
//! when pitch confidence is above the sensitivity threshold.
//!
//! Uses a pre-allocated ring buffer analyzed periodically.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{Hertz, MidiNote, NormalizedValue, PortName, SampleRate, Velocity};
use synth_core::{ModuleType, Param, PitchTrackerParam};

/// Size of the analysis ring buffer.
const RING_BUFFER_SIZE: usize = 2048;

/// How often we run pitch analysis (in samples).
const ANALYSIS_HOP: usize = 512;

/// Pitch tracker voice module.
#[derive(Clone)]
pub struct PitchTracker {
    // Parameters
    sensitivity: NormalizedValue,
    min_freq: Hertz,
    max_freq: Hertz,
    smoothing: NormalizedValue,

    // Ring buffer (pre-allocated)
    ring_buffer: Vec<f32>,
    write_pos: usize,
    hop_counter: usize,

    // Detected pitch state
    current_freq: f32,
    current_confidence: f32,
    smoothed_freq: f32,
    gate_open: bool,

    // State
    sample_rate: SampleRate,

    // Output buffers
    pitch_buffer: AudioBuffer,
    gate_buffer: AudioBuffer,
}

impl PitchTracker {
    pub fn new() -> Self {
        Self {
            sensitivity: NormalizedValue::new(0.5),
            min_freq: Hertz::new(50.0),
            max_freq: Hertz::new(2000.0),
            smoothing: NormalizedValue::new(0.3),
            ring_buffer: vec![0.0; RING_BUFFER_SIZE],
            write_pos: 0,
            hop_counter: 0,
            current_freq: 0.0,
            current_confidence: 0.0,
            smoothed_freq: 0.0,
            gate_open: false,
            sample_rate: SampleRate::DVD_QUALITY,
            pitch_buffer: AudioBuffer::new(1024),
            gate_buffer: AudioBuffer::new(1024),
        }
    }

    /// Run autocorrelation pitch detection on the ring buffer.
    fn analyze_pitch(&mut self) {
        let sr = self.sample_rate.as_f32();
        let min_lag = (sr / self.max_freq.as_f32()) as usize;
        let max_lag = (sr / self.min_freq.as_f32()).min(RING_BUFFER_SIZE as f32 / 2.0) as usize;

        if min_lag >= max_lag || max_lag >= RING_BUFFER_SIZE {
            return;
        }

        // Compute energy of the signal
        let mut energy = 0.0_f32;
        for i in 0..RING_BUFFER_SIZE {
            let s = self.ring_buffer[i];
            energy += s * s;
        }

        if energy < 1e-8 {
            self.current_confidence = 0.0;
            return;
        }

        // Normalized autocorrelation
        let mut best_lag = 0;
        let mut best_corr = 0.0_f32;

        for lag in min_lag..=max_lag {
            let mut correlation = 0.0_f32;
            let analysis_len = RING_BUFFER_SIZE - lag;
            for i in 0..analysis_len {
                let idx_a = (self.write_pos + i) % RING_BUFFER_SIZE;
                let idx_b = (self.write_pos + i + lag) % RING_BUFFER_SIZE;
                correlation += self.ring_buffer[idx_a] * self.ring_buffer[idx_b];
            }
            correlation /= analysis_len as f32;

            if correlation > best_corr {
                best_corr = correlation;
                best_lag = lag;
            }
        }

        // Parabolic interpolation for sub-sample accuracy
        if best_lag > min_lag && best_lag < max_lag {
            let prev = self.autocorr_at(best_lag - 1);
            let curr = self.autocorr_at(best_lag);
            let next = self.autocorr_at(best_lag + 1);

            let denom = prev - 2.0 * curr + next;
            if denom.abs() > 1e-10 {
                let delta = 0.5 * (prev - next) / denom;
                let refined_lag = best_lag as f32 + delta;
                if refined_lag > 0.0 {
                    self.current_freq = sr / refined_lag;
                }
            } else {
                self.current_freq = sr / best_lag as f32;
            }
        } else if best_lag > 0 {
            self.current_freq = sr / best_lag as f32;
        }

        // Confidence: normalized correlation relative to energy
        self.current_confidence = (best_corr / (energy / RING_BUFFER_SIZE as f32)).clamp(0.0, 1.0);

        // Update gate
        self.gate_open = self.current_confidence > self.sensitivity.as_f32();
    }

    /// Helper: compute autocorrelation at a given lag.
    #[inline]
    fn autocorr_at(&self, lag: usize) -> f32 {
        let analysis_len = RING_BUFFER_SIZE - lag;
        let mut correlation = 0.0_f32;
        for i in 0..analysis_len {
            let idx_a = (self.write_pos + i) % RING_BUFFER_SIZE;
            let idx_b = (self.write_pos + i + lag) % RING_BUFFER_SIZE;
            correlation += self.ring_buffer[idx_a] * self.ring_buffer[idx_b];
        }
        correlation / analysis_len as f32
    }

    /// Convert frequency to 1V/octave CV (relative to C4 = MIDI 60).
    #[inline]
    fn freq_to_cv(freq: f32) -> f32 {
        if freq <= 0.0 {
            return 0.0;
        }
        // C4 = 261.63 Hz = 0V, each octave = 1V
        (freq / 261.63).log2()
    }
}

impl Default for PitchTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for PitchTracker {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("pitch_tracker", "Pitch Tracker")
            .description("Autocorrelation pitch detector — outputs CV and gate from audio input")
            .category(ModuleCategory::Utility)
            .tag("pitch")
            .tag("tracker")
            .tag("cv")
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Audio input to track. Koppla: Oscillator Out, Mic input"),
            )
            .port(
                PortDescriptor::audio_output("pitch_cv", "Pitch CV")
                    .description("1V/oct pitch CV output. Koppla till: Oscillator Freq CV"),
            )
            .port(
                PortDescriptor::audio_output("gate", "Gate").description(
                    "Gate output (1.0 when pitch detected). Koppla till: Envelope Gate",
                ),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::PitchTracker(PitchTrackerParam::Sensitivity(NormalizedValue::new(0.5))),
                    "Sensitivity",
                )
                .description("Gate threshold — how confident the detection must be")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::PitchTracker(PitchTrackerParam::MinFreq(Hertz::new(50.0))),
                    "Min Freq",
                )
                .description("Minimum trackable frequency")
                .range(20.0, 500.0)
                .default(50.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::PitchTracker(PitchTrackerParam::MaxFreq(Hertz::new(2000.0))),
                    "Max Freq",
                )
                .description("Maximum trackable frequency")
                .range(200.0, 8000.0)
                .default(2000.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::PitchTracker(PitchTrackerParam::Smoothing(NormalizedValue::new(0.3))),
                    "Smoothing",
                )
                .description("Output smoothing amount")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
    }
}

impl PolyModule for PitchTracker {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.pitch_buffer.resize(num_samples);
        self.gate_buffer.resize(num_samples);

        let input = inputs.get(PortName::IN);

        let smooth = self.smoothing.as_f32();

        for i in 0..num_samples {
            let sample = input.map_or(0.0, |buf| buf[i]);

            // Write to ring buffer
            self.ring_buffer[self.write_pos] = sample;
            self.write_pos = (self.write_pos + 1) % RING_BUFFER_SIZE;

            // Run analysis every ANALYSIS_HOP samples
            self.hop_counter += 1;
            if self.hop_counter >= ANALYSIS_HOP {
                self.hop_counter = 0;
                self.analyze_pitch();
            }

            // Smooth the frequency output
            if self.gate_open && self.current_freq > 0.0 {
                self.smoothed_freq += (1.0 - smooth) * (self.current_freq - self.smoothed_freq);
            }

            // Output pitch CV (1V/octave)
            self.pitch_buffer[i] = Self::freq_to_cv(self.smoothed_freq);

            // Output gate
            self.gate_buffer[i] = if self.gate_open { 1.0 } else { 0.0 };
        }

        if let Some(out) = outputs.get_mut(&PortName::PITCH_CV) {
            out.copy_from(&self.pitch_buffer);
        }
        if let Some(out) = outputs.get_mut(&PortName::GATE) {
            out.copy_from(&self.gate_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::PitchTracker(p) = param {
            match p {
                PitchTrackerParam::Sensitivity(v) => self.sensitivity = v,
                PitchTrackerParam::MinFreq(h) => {
                    self.min_freq = Hertz::new(h.as_f32().clamp(20.0, 500.0));
                }
                PitchTrackerParam::MaxFreq(h) => {
                    self.max_freq = Hertz::new(h.as_f32().clamp(200.0, 8000.0));
                }
                PitchTrackerParam::Smoothing(v) => self.smoothing = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::PitchTracker(p) = param {
            Some(match p {
                PitchTrackerParam::Sensitivity(_) => self.sensitivity.as_f32(),
                PitchTrackerParam::MinFreq(_) => self.min_freq.as_f32(),
                PitchTrackerParam::MaxFreq(_) => self.max_freq.as_f32(),
                PitchTrackerParam::Smoothing(_) => self.smoothing.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::PitchTracker(PitchTrackerParam::Sensitivity(self.sensitivity)),
            Param::PitchTracker(PitchTrackerParam::MinFreq(self.min_freq)),
            Param::PitchTracker(PitchTrackerParam::MaxFreq(self.max_freq)),
            Param::PitchTracker(PitchTrackerParam::Smoothing(self.smoothing)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::PitchTracker
    }

    fn reset(&mut self) {
        self.ring_buffer.fill(0.0);
        self.write_pos = 0;
        self.hop_counter = 0;
        self.current_freq = 0.0;
        self.current_confidence = 0.0;
        self.smoothed_freq = 0.0;
        self.gate_open = false;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Pitch tracker is input-driven, not note-driven
    }

    fn note_off(&mut self) {
        // Nothing to do
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    #[test]
    fn test_pitch_tracker_creation() {
        let pt = PitchTracker::new();
        assert_eq!(pt.sensitivity.as_f32(), 0.5);
        assert_eq!(pt.min_freq.as_f32(), 50.0);
        assert_eq!(pt.max_freq.as_f32(), 2000.0);
    }

    #[test]
    fn test_freq_to_cv() {
        // C4 = 261.63 Hz should give ~0V
        let cv = PitchTracker::freq_to_cv(261.63);
        assert!(cv.abs() < 0.01, "C4 should be ~0V, got {cv}");

        // C5 = 523.26 Hz should give ~1V
        let cv = PitchTracker::freq_to_cv(523.26);
        assert!((cv - 1.0).abs() < 0.01, "C5 should be ~1V, got {cv}");
    }

    #[test]
    fn test_pitch_tracker_params() {
        let mut pt = PitchTracker::new();
        pt.set_param(Param::PitchTracker(PitchTrackerParam::Sensitivity(
            NormalizedValue::new(0.8),
        )));
        assert!((pt.sensitivity.as_f32() - 0.8).abs() < 0.001);

        let params = pt.get_params();
        assert_eq!(params.len(), 4);
    }

    #[test]
    fn test_silence_gives_no_gate() {
        let mut pt = PitchTracker::new();
        let num = 1024;
        let in_buf = AudioBuffer::new(num); // all zeros

        let mut outputs = HashMap::new();
        outputs.insert("pitch_cv".to_string(), AudioBuffer::new(num));
        outputs.insert("gate".to_string(), AudioBuffer::new(num));

        let context = ProcessContext {
            samples: SampleCount::new(num),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        let inputs = InputPorts::from_single("in", &in_buf);
        pt.process(inputs, &mut outputs, &context);

        // Gate should be closed for silence
        let gate = &outputs["gate"];
        for i in 0..num {
            assert!(
                gate[i] < 0.5,
                "Expected gate closed for silence, got {} at {}",
                gate[i],
                i
            );
        }
    }
}
