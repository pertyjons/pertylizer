//! Beat Detector module.
//!
//! Envelope-following beat detection with Schmitt trigger gate output.
//! Useful for rhythmic gating and auto-tempo detection.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Analysis/200-beat-detector-class.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{BeatDetectorParam, ModuleType, Param};
use synth_core::{Hertz, MidiNote, Milliseconds, NormalizedValue, PortName, SampleRate, Velocity};

/// Beat Detector — envelope-based beat/transient detection.
#[derive(Clone)]
pub struct BeatDetector {
    sensitivity: NormalizedValue,
    filter_freq: Hertz,
    hold_time: Milliseconds,
    sample_rate: SampleRate,
    /// Envelope follower state
    envelope: f32,
    /// Previous envelope value for edge detection
    prev_envelope: f32,
    /// Gate state (0.0 or 1.0)
    gate: f32,
    /// Hold counter in samples
    hold_counter: u32,
    output_buffer: AudioBuffer,
    gate_buffer: AudioBuffer,
}

impl BeatDetector {
    pub fn new() -> Self {
        Self {
            sensitivity: NormalizedValue::new(0.5),
            filter_freq: Hertz::new(150.0),
            hold_time: Milliseconds::new(50.0),
            sample_rate: SampleRate::DVD_QUALITY,
            envelope: 0.0,
            prev_envelope: 0.0,
            gate: 0.0,
            hold_counter: 0,
            output_buffer: AudioBuffer::new(1024),
            gate_buffer: AudioBuffer::new(1024),
        }
    }
}

impl Default for BeatDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for BeatDetector {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("beat_detector", "Beat Detector")
            .description("Envelope-based beat/transient detector with gate output")
            .category(ModuleCategory::Utility)
            .tag("beat")
            .tag("detector")
            .tag("analysis")
            .parameter(
                ParameterDescriptor::float(
                    "sensitivity",
                    Param::BeatDetector(BeatDetectorParam::Sensitivity(NormalizedValue::new(0.5))),
                    "Sensitivity",
                )
                .description("Detection threshold")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "filter_freq",
                    Param::BeatDetector(BeatDetectorParam::FilterFreq(Hertz::new(150.0))),
                    "Filter",
                )
                .description("Envelope lowpass frequency")
                .range(20.0, 1000.0)
                .default(150.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "hold_time",
                    Param::BeatDetector(BeatDetectorParam::HoldTime(Milliseconds::new(50.0))),
                    "Hold",
                )
                .description("Gate hold time")
                .range(1.0, 500.0)
                .default(50.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Audio to analyze. Connect: any audio source"),
            )
            .port(
                PortDescriptor::audio_output("out", "Env Out")
                    .description("Envelope follower output (0-1)"),
            )
            .port(
                PortDescriptor::audio_output("gate", "Gate")
                    .description("Beat gate output (0 or 1)"),
            )
    }
}

impl PolyModule for BeatDetector {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());
        self.gate_buffer.resize(context.samples.as_usize());

        let audio_in = inputs.reader(PortName::IN, 0.0);

        // Envelope follower coefficient from filter frequency
        let coeff = self.filter_freq.to_exp_coeff(self.sample_rate);
        // Threshold from sensitivity (map 0..1 to usable range)
        let threshold_high = 1.0 - self.sensitivity.as_f32() * 0.9;
        let threshold_low = threshold_high * 0.6; // Schmitt trigger hysteresis
        let hold_samples = (self.hold_time.as_f32() * self.sample_rate.as_f32() / 1000.0) as u32;

        let gate_name = PortName::intern("gate");

        for i in 0..context.samples.as_usize() {
            let input = audio_in[i].abs();

            // One-pole lowpass envelope follower
            self.envelope += coeff * (input - self.envelope);

            // Schmitt trigger: detect rising edge of envelope
            if self.envelope > threshold_high && self.prev_envelope <= threshold_high {
                self.gate = 1.0;
                self.hold_counter = hold_samples;
            }

            // Hold gate for hold_time, then release when envelope drops below low threshold
            if self.hold_counter > 0 {
                self.hold_counter -= 1;
            } else if self.envelope < threshold_low {
                self.gate = 0.0;
            }

            self.prev_envelope = self.envelope;

            self.output_buffer[i] = self.envelope.clamp(0.0, 1.0);
            self.gate_buffer[i] = self.gate;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
        if let Some(gate_out) = outputs.get_mut(&gate_name) {
            gate_out.copy_from(&self.gate_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::BeatDetector(p) = param {
            match p {
                BeatDetectorParam::Sensitivity(s) => self.sensitivity = s,
                BeatDetectorParam::FilterFreq(f) => {
                    self.filter_freq = Hertz::new(f.as_f32().clamp(20.0, 1000.0));
                }
                BeatDetectorParam::HoldTime(h) => {
                    self.hold_time = Milliseconds::new(h.as_f32().clamp(1.0, 500.0));
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::BeatDetector(p) = param {
            Some(match p {
                BeatDetectorParam::Sensitivity(_) => self.sensitivity.as_f32(),
                BeatDetectorParam::FilterFreq(_) => self.filter_freq.as_f32(),
                BeatDetectorParam::HoldTime(_) => self.hold_time.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::BeatDetector(BeatDetectorParam::Sensitivity(self.sensitivity)),
            Param::BeatDetector(BeatDetectorParam::FilterFreq(self.filter_freq)),
            Param::BeatDetector(BeatDetectorParam::HoldTime(self.hold_time)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::BeatDetector
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.prev_envelope = 0.0;
        self.gate = 0.0;
        self.hold_counter = 0;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
