//! Turing Machine module.
//!
//! A shift-register-based random note generator inspired by Music Thing Modular's
//! Turing Machine. Generates semi-random melodies that can be locked, mutated,
//! or fully random.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext,
    StepCount, TuringMachineParam, TuringScale, WidgetHint,
};
use synth_core::{MidiNote, PortName, SampleRate, Velocity};

/// Scale intervals for quantization.
const CHROMATIC: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const MAJOR: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR: [u8; 7] = [0, 2, 3, 5, 7, 8, 10];
const PENTATONIC: [u8; 5] = [0, 2, 4, 7, 9];

/// Turing Machine voice module.
#[derive(Clone)]
pub struct TuringMachine {
    // Parameters
    mutation_rate: NormalizedValue,
    range: NormalizedValue,
    scale: TuringScale,
    length: StepCount,

    // State
    shift_register: u16,
    sample_rate: SampleRate,
    step_counter: f32,
    samples_per_step: f32,
    current_cv: f32,
    gate_active: bool,

    // Simple PRNG state
    rng_state: u32,

    // Buffers
    pitch_buffer: AudioBuffer,
    gate_buffer: AudioBuffer,
}

impl TuringMachine {
    pub fn new() -> Self {
        Self {
            mutation_rate: NormalizedValue::CENTER,
            range: NormalizedValue::new(0.5),
            scale: TuringScale::default(),
            length: StepCount::new(16),

            shift_register: 0xACE1, // Initial seed
            sample_rate: SampleRate::DVD_QUALITY,
            step_counter: 0.0,
            samples_per_step: 5512.5,
            current_cv: 0.0,
            gate_active: false,

            rng_state: 12345,

            pitch_buffer: AudioBuffer::new(1024),
            gate_buffer: AudioBuffer::new(1024),
        }
    }

    /// Simple xorshift PRNG.
    #[inline]
    fn next_random(&mut self) -> f32 {
        self.rng_state ^= self.rng_state << 13;
        self.rng_state ^= self.rng_state >> 17;
        self.rng_state ^= self.rng_state << 5;
        (self.rng_state as f32) / (u32::MAX as f32)
    }

    /// Advance the shift register by one step.
    fn step(&mut self) {
        let mutation = self.mutation_rate.as_f32();
        let length = self.length.as_u8();
        let mask = if length >= 16 {
            0xFFFF
        } else {
            (1u16 << length) - 1
        };

        // Get the bit that's about to fall off
        let feedback_bit = (self.shift_register >> (length - 1)) & 1;

        // Mutation: probability of flipping the feedback bit
        let bit = if mutation < 0.01 {
            // Locked: always feed back the same bit
            feedback_bit
        } else if mutation > 0.99 {
            // Fully random
            if self.next_random() > 0.5 { 1 } else { 0 }
        } else {
            // Probabilistic mutation
            if self.next_random() < mutation {
                feedback_bit ^ 1 // Flip the bit
            } else {
                feedback_bit // Keep the bit
            }
        };

        // Shift and insert new bit
        self.shift_register = ((self.shift_register << 1) | bit) & mask;

        // Convert register to CV
        let raw_value = (self.shift_register as f32) / (mask as f32);
        self.current_cv = self.quantize_to_scale(raw_value);
        self.gate_active = true;
    }

    /// Quantize a 0.0-1.0 value to a scale.
    fn quantize_to_scale(&self, value: f32) -> f32 {
        let range_semitones = self.range.as_f32() * 24.0; // Up to 2 octaves
        let semitone = value * range_semitones;

        let intervals: &[u8] = match self.scale {
            TuringScale::Chromatic => &CHROMATIC,
            TuringScale::Major => &MAJOR,
            TuringScale::Minor => &MINOR,
            TuringScale::Pentatonic => &PENTATONIC,
        };

        if intervals.is_empty() {
            return 0.0;
        }

        // Find closest scale degree
        let octave = (semitone / 12.0).floor();
        let note_in_octave = semitone - octave * 12.0;

        let mut closest = intervals[0];
        let mut min_dist = f32::MAX;
        for &interval in intervals {
            let dist = (note_in_octave - interval as f32).abs();
            if dist < min_dist {
                min_dist = dist;
                closest = interval;
            }
        }

        // Return as V/Oct (1V = 1 octave)
        (octave * 12.0 + closest as f32) / 12.0
    }
}

impl Default for TuringMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for TuringMachine {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("turing_machine", "Turing Machine")
            .description("Shift-register random melody generator")
            .category(ModuleCategory::LFO)
            .tag("turing")
            .tag("generative")
            .tag("random")
            .parameter(
                ParameterDescriptor::float(
                    "mutation",
                    Param::TuringMachine(TuringMachineParam::MutationRate(NormalizedValue::CENTER)),
                    "Mutation",
                )
                .description("Mutation rate (0=locked, 0.5=evolving, 1=random)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "range",
                    Param::TuringMachine(TuringMachineParam::Range(NormalizedValue::new(0.5))),
                    "Range",
                )
                .description("Output pitch range")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "scale",
                    Param::TuringMachine(TuringMachineParam::Scale(TuringScale::Chromatic)),
                    "Scale",
                    TuringScale::to_choices(),
                )
                .description("Scale quantization")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    "length",
                    Param::TuringMachine(TuringMachineParam::Length(StepCount::new(16))),
                    "Length",
                )
                .description("Shift register length (8 or 16)")
                .range(8.0, 16.0)
                .default(16.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("clock", "Clock").description("External clock input"),
            )
            .port(PortDescriptor::audio_output("pitch", "Pitch").description("Pitch CV output"))
            .port(PortDescriptor::audio_output("gate", "Gate").description("Gate output"))
    }
}

impl PolyModule for TuringMachine {
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

        let clock = inputs.get(PortName::intern("clock"));

        // Calculate step timing from BPM
        let bpm = context.tempo.as_f32().max(20.0);
        self.samples_per_step = self.sample_rate.as_f32() * 60.0 / bpm / 4.0; // 16th notes

        for i in 0..num_samples {
            // Clock detection
            let clock_trigger = if let Some(clk) = clock {
                clk[i] > 0.5 && (i == 0 || clk[i - 1] <= 0.5)
            } else {
                false
            };

            let advance = if clock.is_some() {
                clock_trigger
            } else {
                self.step_counter += 1.0;
                if self.step_counter >= self.samples_per_step {
                    self.step_counter = 0.0;
                    true
                } else {
                    false
                }
            };

            if advance {
                self.step();
            }

            // Gate: short pulse on step
            let gate_phase = self.step_counter / self.samples_per_step;
            let gate_on = self.gate_active && gate_phase < 0.5;
            self.gate_buffer[i] = if gate_on { 1.0 } else { 0.0 };

            // Pitch CV (constant until next step)
            self.pitch_buffer[i] = self.current_cv;
        }

        if let Some(out) = outputs.get_mut(&PortName::PITCH) {
            out.copy_from(&self.pitch_buffer);
        }
        if let Some(out) = outputs.get_mut(&PortName::GATE) {
            out.copy_from(&self.gate_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::TuringMachine(p) = param {
            match p {
                TuringMachineParam::MutationRate(v) => self.mutation_rate = v,
                TuringMachineParam::Range(v) => self.range = v,
                TuringMachineParam::Scale(s) => self.scale = s,
                TuringMachineParam::Length(n) => {
                    self.length = StepCount::new(if n.as_u8() > 12 { 16 } else { 8 })
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::TuringMachine(p) = param {
            Some(match p {
                TuringMachineParam::MutationRate(_) => self.mutation_rate.as_f32(),
                TuringMachineParam::Range(_) => self.range.as_f32(),
                TuringMachineParam::Scale(_) => self.scale.index() as f32,
                TuringMachineParam::Length(_) => self.length.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::TuringMachine(TuringMachineParam::MutationRate(self.mutation_rate)),
            Param::TuringMachine(TuringMachineParam::Range(self.range)),
            Param::TuringMachine(TuringMachineParam::Scale(self.scale)),
            Param::TuringMachine(TuringMachineParam::Length(self.length)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::TuringMachine
    }

    fn reset(&mut self) {
        self.step_counter = 0.0;
        self.gate_active = false;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        self.reset();
    }

    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turing_machine_creation() {
        let tm = TuringMachine::new();
        assert_eq!(tm.length, StepCount::new(16));
    }

    #[test]
    fn test_turing_machine_step() {
        let mut tm = TuringMachine::new();
        tm.mutation_rate = NormalizedValue::new(0.0); // Locked
        let first_cv = {
            tm.step();
            tm.current_cv
        };
        // Step 16 times to cycle through register
        for _ in 0..16 {
            tm.step();
        }
        // Should repeat when locked
        assert!(
            (tm.current_cv - first_cv).abs() < 0.01,
            "Locked Turing Machine should repeat"
        );
    }
}
