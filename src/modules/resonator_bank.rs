//! Resonator Bank module for sympathetic resonance.
//!
//! Simulates sympathetic string resonance found in pianos and other
//! stringed instruments. Multiple tuned resonators respond to input.

use std::collections::HashMap;

use crate::engine::typed_params::{ModuleType, Param, ResonatorBankParam};
use crate::modules::core::*;
use crate::types::{Gain, MidiNote, NormalizedValue, SampleRate};

/// Number of resonator strings in the bank.
const MAX_STRINGS: usize = 12;

/// Maximum delay size per string.
const MAX_DELAY_SIZE: usize = 2048;

/// A single resonating string.
#[derive(Clone)]
struct ResonatorString {
    delay_line: [f32; MAX_DELAY_SIZE],
    write_pos: usize,
    delay_length: usize,
    fractional: f32,
    filter_state: f32,
    frequency: f32,
    active: bool,
}

impl ResonatorString {
    fn new() -> Self {
        Self {
            delay_line: [0.0; MAX_DELAY_SIZE],
            write_pos: 0,
            delay_length: 100,
            fractional: 0.0,
            filter_state: 0.0,
            frequency: 440.0,
            active: false,
        }
    }

    fn set_frequency(&mut self, freq: f32, sample_rate: f32) {
        self.frequency = freq.max(20.0).min(20000.0);
        let period = sample_rate / self.frequency;
        self.delay_length = (period as usize).min(MAX_DELAY_SIZE - 1).max(1);
        self.fractional = period - self.delay_length as f32;
        self.active = true;
    }

    fn reset(&mut self) {
        self.delay_line.fill(0.0);
        self.filter_state = 0.0;
        self.write_pos = 0;
    }

    #[inline]
    fn process(&mut self, input: f32, resonance: f32, damping: f32) -> f32 {
        if !self.active {
            return 0.0;
        }

        // Read with interpolation
        let read_pos = (self.write_pos + MAX_DELAY_SIZE - self.delay_length) % MAX_DELAY_SIZE;
        let next_pos = (read_pos + 1) % MAX_DELAY_SIZE;
        let delayed = self.delay_line[read_pos] * (1.0 - self.fractional)
            + self.delay_line[next_pos] * self.fractional;

        // Damping filter
        let filtered = self.filter_state * damping + delayed * (1.0 - damping);
        self.filter_state = filtered;

        // Feedback with input coupling
        let feedback = (filtered + input * resonance) * 0.995;
        self.delay_line[self.write_pos] = feedback.clamp(-1.0, 1.0);
        self.write_pos = (self.write_pos + 1) % MAX_DELAY_SIZE;

        filtered
    }
}

impl Default for ResonatorString {
    fn default() -> Self {
        Self::new()
    }
}

/// Bank of sympathetic resonators.
#[derive(Clone)]
pub struct ResonatorBank {
    // Parameters
    string_count: u8,
    resonance: NormalizedValue,
    detune: f32,
    decay_mult: NormalizedValue,
    level: Gain,

    // Resonator strings
    strings: [ResonatorString; MAX_STRINGS],

    // Sample rate
    sample_rate: SampleRate,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl ResonatorBank {
    pub fn new() -> Self {
        Self {
            string_count: 3,
            resonance: NormalizedValue::new(0.5),
            detune: 0.0,
            decay_mult: NormalizedValue::new(0.5),
            level: Gain::new(0.3),

            strings: std::array::from_fn(|_| ResonatorString::new()),

            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Set up sympathetic strings based on a fundamental frequency.
    fn setup_strings(&mut self, fundamental: f32) {
        let sr = self.sample_rate.as_f32();
        let count = self.string_count as usize;
        let detune_cents = self.detune;

        for (i, string) in self.strings.iter_mut().enumerate() {
            if i < count {
                // Set up harmonically related frequencies with detuning
                let harmonic = (i + 1) as f32;
                let detune_ratio = 2.0_f32.powf((detune_cents * (i as f32)) / 1200.0);
                let freq = fundamental * harmonic * detune_ratio;
                string.set_frequency(freq, sr);
            } else {
                string.active = false;
            }
        }
    }
}

impl Default for ResonatorBank {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for ResonatorBank {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("resonator_bank", "Resonator Bank")
            .description("Sympathetic resonance modeling with multiple strings")
            .category(ModuleCategory::PhysicalModeling)
            .tag("resonator")
            .tag("sympathetic")
            .tag("physical")
            .tag("piano")
            .parameter(
                ParameterDescriptor::float(
                    Param::ResonatorBank(ResonatorBankParam::StringCount(3)),
                    "Strings",
                )
                .description("Number of resonating strings")
                .range(1.0, 12.0)
                .default(3.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ResonatorBank(ResonatorBankParam::Resonance(NormalizedValue::new(0.5))),
                    "Resonance",
                )
                .description("Sympathetic resonance amount")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ResonatorBank(ResonatorBankParam::Detune(0.0)),
                    "Detune",
                )
                .description("Detuning between strings (cents)")
                .range(-50.0, 50.0)
                .default(0.0)
                .unit(ParameterUnit::Cents)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ResonatorBank(ResonatorBankParam::DecayMult(NormalizedValue::new(0.5))),
                    "Decay",
                )
                .description("Decay time multiplier")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ResonatorBank(ResonatorBankParam::Level(Gain::new(0.3))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.3)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
            .port(PortDescriptor::audio_output("out", "Out").description("Resonance output"))
    }
}

impl PolyModule for ResonatorBank {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        let input = inputs.get("in");
        let resonance = self.resonance.as_f32();
        let damping = 0.3 + self.decay_mult.as_f32() * 0.6;
        let level = self.level.as_f32();

        for i in 0..context.samples {
            let input_sample = input.map_or(0.0, |buf| buf[i]);

            let mut output = 0.0;
            for string in &mut self.strings {
                output += string.process(input_sample, resonance, damping);
            }

            // Normalize by string count and apply level
            let count = self.string_count.max(1) as f32;
            self.output_buffer[i] = (output / count) * level;
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::ResonatorBank(p) = param {
            match p {
                ResonatorBankParam::StringCount(n) => {
                    self.string_count = n.clamp(1, MAX_STRINGS as u8);
                }
                ResonatorBankParam::Resonance(v) => self.resonance = v,
                ResonatorBankParam::Detune(d) => self.detune = d,
                ResonatorBankParam::DecayMult(v) => self.decay_mult = v,
                ResonatorBankParam::Level(g) => self.level = g,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::ResonatorBank(p) = param {
            Some(match p {
                ResonatorBankParam::StringCount(_) => f32::from(self.string_count),
                ResonatorBankParam::Resonance(_) => self.resonance.as_f32(),
                ResonatorBankParam::Detune(_) => self.detune,
                ResonatorBankParam::DecayMult(_) => self.decay_mult.as_f32(),
                ResonatorBankParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::ResonatorBank(ResonatorBankParam::StringCount(self.string_count)),
            Param::ResonatorBank(ResonatorBankParam::Resonance(self.resonance)),
            Param::ResonatorBank(ResonatorBankParam::Detune(self.detune)),
            Param::ResonatorBank(ResonatorBankParam::DecayMult(self.decay_mult)),
            Param::ResonatorBank(ResonatorBankParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::ResonatorBank
    }

    fn reset(&mut self) {
        for string in &mut self.strings {
            string.reset();
        }
    }

    fn note_on(&mut self, note: MidiNote, _velocity: f32) {
        // Set up strings based on played note
        let freq = 440.0 * 2.0_f32.powf((f32::from(note.as_u8()) - 69.0) / 12.0);
        self.setup_strings(freq);
    }

    fn note_off(&mut self) {
        // Let resonances decay naturally
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = SampleRate::new(sample_rate);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
