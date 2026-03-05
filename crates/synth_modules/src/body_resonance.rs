//! Body Resonance module for soundboard simulation.
//!
//! Simulates the resonant body of acoustic instruments like
//! piano soundboards, guitar bodies, etc.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{BodyResonanceParam, ModuleType, Param};
use synth_core::{FilterState, Hertz, MidiNote, NormalizedValue, PortName, SampleRate, Velocity};

/// Body resonance simulator using bandpass filters.
#[derive(Clone)]
pub struct BodyResonance {
    // Parameters
    frequency: Hertz,
    resonance: NormalizedValue,
    size: NormalizedValue,
    brightness: NormalizedValue,
    mix: NormalizedValue,

    // Filter states (three resonant modes, each with 2 state variables)
    filter1_state: [FilterState; 2],
    filter2_state: [FilterState; 2],
    filter3_state: [FilterState; 2],

    // Sample rate
    sample_rate: SampleRate,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl BodyResonance {
    pub fn new() -> Self {
        Self {
            frequency: Hertz::new(200.0),
            resonance: NormalizedValue::new(0.5),
            size: NormalizedValue::new(0.5),
            brightness: NormalizedValue::new(0.5),
            mix: NormalizedValue::new(0.3),

            filter1_state: [FilterState::ZERO; 2],
            filter2_state: [FilterState::ZERO; 2],
            filter3_state: [FilterState::ZERO; 2],

            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Process through a resonant bandpass filter (state variable filter).
    #[inline]
    fn process_resonator(
        state: &mut [FilterState; 2],
        input: f32,
        freq: Hertz,
        q: f32,
        sample_rate: SampleRate,
    ) -> f32 {
        // Coefficient calculation with dynamic Nyquist limit
        let sr = sample_rate.as_f32();
        let nyquist_limit = sr * 0.49;
        let f = (freq.as_f32().min(nyquist_limit)) / sr;
        let w = 2.0 * std::f32::consts::PI * f;
        let sin_w = w.sin();
        let alpha = sin_w / (2.0 * q.max(0.5));

        // Simple bandpass using biquad coefficients
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * w.cos();
        let a2 = 1.0 - alpha;
        let b0 = alpha;
        let b2 = -alpha;

        // Direct form II transposed
        let output = (b0 / a0) * input + state[0].as_f32();
        state[0] = FilterState::new((b2 / a0) * input - (a1 / a0) * output + state[1].as_f32());
        state[1] = FilterState::new(-(a2 / a0) * output);

        output
    }
}

impl Default for BodyResonance {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for BodyResonance {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("body_resonance", "Body Resonance")
            .description("Soundboard resonance simulation")
            .category(ModuleCategory::PhysicalModeling)
            .tag("body")
            .tag("resonance")
            .tag("soundboard")
            .tag("piano")
            .parameter(
                ParameterDescriptor::float(
                    Param::BodyResonance(BodyResonanceParam::Frequency(Hertz::new(200.0))),
                    "Freq",
                )
                .description("Primary resonant frequency")
                .range(50.0, 2000.0)
                .default(200.0)
                .unit(ParameterUnit::Hertz)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::FrequencySlider),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::BodyResonance(BodyResonanceParam::Resonance(NormalizedValue::new(0.5))),
                    "Resonance",
                )
                .description("Resonance amount (Q factor)")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::BodyResonance(BodyResonanceParam::Size(NormalizedValue::new(0.5))),
                    "Size",
                )
                .description("Body size (affects frequency spread)")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::BodyResonance(BodyResonanceParam::Brightness(NormalizedValue::new(0.5))),
                    "Bright",
                )
                .description("Brightness of resonance")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::BodyResonance(BodyResonanceParam::Mix(NormalizedValue::new(0.3))),
                    "Mix",
                )
                .description("Wet/dry mix")
                .range(0.0, 1.0)
                .default(0.3)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
            .port(PortDescriptor::audio_output("out", "Out").description("Processed output"))
    }
}

impl PolyModule for BodyResonance {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let input = inputs.get(PortName::IN);
        let base_freq = self.frequency.as_f32();
        let q = 0.5 + self.resonance.as_f32() * 10.0; // Q from 0.5 to 10.5
        let size = self.size.as_f32();
        let brightness = self.brightness.as_f32();
        let mix = self.mix.as_f32();

        // Calculate frequencies for three resonant modes
        // Smaller body = higher frequencies, larger spread
        let freq1 = Hertz::new(base_freq * (1.0 - size * 0.3));
        let freq2 = Hertz::new(base_freq * (1.0 + brightness * 0.5));
        let freq3 = Hertz::new(base_freq * (1.5 + size * 0.5));

        for i in 0..context.samples.as_usize() {
            let input_sample = input.map_or(0.0, |buf| buf[i]);

            // Process through three parallel resonators
            let r1 = Self::process_resonator(
                &mut self.filter1_state,
                input_sample,
                freq1,
                q,
                self.sample_rate,
            );
            let r2 = Self::process_resonator(
                &mut self.filter2_state,
                input_sample,
                freq2,
                q * 0.7,
                self.sample_rate,
            );
            let r3 = Self::process_resonator(
                &mut self.filter3_state,
                input_sample,
                freq3,
                q * 0.5,
                self.sample_rate,
            );

            // Mix resonators with different weights
            let resonance = r1 * 0.5 + r2 * 0.3 + r3 * 0.2;

            // Wet/dry mix
            self.output_buffer[i] = input_sample * (1.0 - mix) + resonance * mix;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::BodyResonance(p) = param {
            match p {
                BodyResonanceParam::Frequency(f) => self.frequency = f,
                BodyResonanceParam::Resonance(v) => self.resonance = v,
                BodyResonanceParam::Size(v) => self.size = v,
                BodyResonanceParam::Brightness(v) => self.brightness = v,
                BodyResonanceParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::BodyResonance(p) = param {
            Some(match p {
                BodyResonanceParam::Frequency(_) => self.frequency.as_f32(),
                BodyResonanceParam::Resonance(_) => self.resonance.as_f32(),
                BodyResonanceParam::Size(_) => self.size.as_f32(),
                BodyResonanceParam::Brightness(_) => self.brightness.as_f32(),
                BodyResonanceParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::BodyResonance(BodyResonanceParam::Frequency(self.frequency)),
            Param::BodyResonance(BodyResonanceParam::Resonance(self.resonance)),
            Param::BodyResonance(BodyResonanceParam::Size(self.size)),
            Param::BodyResonance(BodyResonanceParam::Brightness(self.brightness)),
            Param::BodyResonance(BodyResonanceParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::BodyResonance
    }

    fn reset(&mut self) {
        self.filter1_state = [FilterState::ZERO; 2];
        self.filter2_state = [FilterState::ZERO; 2];
        self.filter3_state = [FilterState::ZERO; 2];
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Could modulate body resonance based on note if desired
    }

    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
