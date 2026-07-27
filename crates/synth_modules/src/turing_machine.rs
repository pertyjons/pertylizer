//! Turing Machine module.
//!
//! A shift-register-based random note generator inspired by Music Thing Modular's
//! Turing Machine. Generates semi-random melodies that can be locked, mutated,
//! or fully random.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParamModOffsets, ParameterDescriptor, PolyModule, PortDescriptor,
    ProcessContext, StepCount, TuringMachineParam, TuringScale, WidgetHint,
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
    current_cv: NormalizedValue,
    gate_active: bool,

    // Simple PRNG state
    rng_state: u32,

    // Edge detection
    prev_clock: f32,

    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

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
            current_cv: NormalizedValue::MIN,
            gate_active: false,

            rng_state: 12345,

            prev_clock: 0.0,

            mod_offsets: ParamModOffsets::new(),

            pitch_buffer: AudioBuffer::new(1024),
            gate_buffer: AudioBuffer::new(1024),
        }
    }

    /// Simple xorshift PRNG.
    #[inline]
    fn next_random(&mut self) -> f32 {
        crate::math::xorshift32(&mut self.rng_state)
    }

    /// Advance the shift register by one step. `mutation` and `range` are the
    /// effective (mod-offset-applied) values, resolved once per block.
    fn step(&mut self, mutation: f32, range: f32) {
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
        self.current_cv = NormalizedValue::new(self.quantize_to_scale(raw_value, range));
        self.gate_active = true;
    }

    /// Quantize a 0.0-1.0 value to a scale. `range` is the effective output range.
    fn quantize_to_scale(&self, value: f32, range: f32) -> f32 {
        let range_semitones = range * 24.0; // Up to 2 octaves
        let semitone = value * range_semitones;

        let intervals: &[u8] = match self.scale {
            TuringScale::Chromatic => &CHROMATIC,
            TuringScale::Major => &MAJOR,
            TuringScale::Minor => &MINOR,
            TuringScale::Pentatonic => &PENTATONIC,
        };

        crate::math::quantize_to_scale(semitone, intervals)
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
                // Structural/sizing param (shift-register length): not ramp-able.
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::gate_input("clock", "Clock").description("External clock input"))
            .port(PortDescriptor::control_output("pitch", "Pitch").description("Pitch CV output"))
            .port(PortDescriptor::gate_output("gate", "Gate").description("Gate output"))
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

        let clock = inputs.get(PortName::CLOCK);

        // Generic mod offsets — per-block constants, resolved once here.
        let eff_mutation = self
            .mod_offsets
            .effective("mutation", self.mutation_rate.as_f32());
        let eff_range = self.mod_offsets.effective("range", self.range.as_f32());

        // Calculate step timing from BPM
        let bpm = context.tempo.as_f32().max(20.0);
        self.samples_per_step = crate::math::samples_per_16th(self.sample_rate.as_f32(), bpm);

        for i in 0..num_samples {
            // Clock detection
            let clock_trigger = if let Some(clk) = clock {
                let prev = if i == 0 { self.prev_clock } else { clk[i - 1] };
                crate::math::rising_edge(clk[i], prev)
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
                self.step(eff_mutation, eff_range);
            }

            // Gate: short pulse on step
            let gate_phase = self.step_counter / self.samples_per_step;
            let gate_on = self.gate_active && crate::math::gate_pulse(gate_phase, 0.5);
            self.gate_buffer[i] = if gate_on { 1.0 } else { 0.0 };

            // Pitch CV (constant until next step)
            self.pitch_buffer[i] = self.current_cv.as_f32();
        }

        // Persist last clock sample for edge detection across buffers
        if let Some(clk) = clock
            && num_samples > 0
        {
            self.prev_clock = clk[num_samples - 1];
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

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.step_counter = 0.0;
        self.gate_active = false;
    }

    fn set_seed(&mut self, seed: u64) {
        // Both the LFSR shift register and the xorshift RNG must be non-zero;
        // `| 1` guarantees it. `reset` leaves these untouched, so the seed sticks.
        self.shift_register = (seed as u16) | 1;
        self.rng_state = ((seed >> 16) as u32) | 1;
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
        let m = tm.mutation_rate.as_f32();
        let r = tm.range.as_f32();
        let first_cv = {
            tm.step(m, r);
            tm.current_cv
        };
        // Step 16 times to cycle through register
        for _ in 0..16 {
            tm.step(m, r);
        }
        // Should repeat when locked
        assert!(
            (tm.current_cv.as_f32() - first_cv.as_f32()).abs() < 0.01,
            "Locked Turing Machine should repeat"
        );
    }

    /// `range` is a working mod destination via the generic store: driving it to
    /// 0 collapses every quantized pitch to 0, and clearing reverts.
    #[test]
    fn range_mod_offset_collapses_pitch() {
        // Many samples so several free-running steps elapse within the block.
        let n = 64000;
        let render = |offset: f32| -> f32 {
            let mut tm = TuringMachine::new();
            let desc = tm.descriptor();
            tm.mod_offsets_mut().unwrap().populate(&desc);
            tm.mutation_rate = NormalizedValue::MAX; // keep the register churning
            if offset != 0.0 {
                tm.set_mod_offset("range", offset);
            }
            let ctx = ProcessContext {
                samples: synth_core::SampleCount::new(n),
                ..ProcessContext::default()
            };
            let mut outs = HashMap::new();
            outs.insert(PortName::PITCH, AudioBuffer::new(n));
            outs.insert(PortName::GATE, AudioBuffer::new(n));
            tm.process(InputPorts::empty(), &mut outs, &ctx);
            let b = &outs[&PortName::PITCH];
            (0..b.len()).map(|i| b[i].abs()).sum::<f32>()
        };

        let base = render(0.0);
        assert!(
            base > 0.0,
            "default range should produce pitch CV, got {base}"
        );

        let collapsed = render(-1.0); // range → 0
        assert_eq!(
            collapsed, 0.0,
            "range→0 should collapse pitch to 0: {collapsed}"
        );
    }
}
