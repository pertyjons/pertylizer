//! Euclidean Sequencer module.
//!
//! Generates rhythmic patterns using the Björklund algorithm to distribute
//! pulses evenly across steps. Outputs gate and accent CV signals.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, EuclideanParam, InputPorts, ModuleCategory, ModuleDescriptor,
    ModuleType, NormalizedValue, Param, ParameterDescriptor, PolyModule, PortDescriptor,
    ProcessContext, StepCount, WidgetHint,
};
use synth_core::{MidiNote, PortName, SampleRate, Velocity};

/// Maximum number of steps.
const MAX_STEPS: usize = 32;

/// Euclidean sequencer voice module.
#[derive(Clone)]
pub struct Euclidean {
    // Parameters
    steps: StepCount,
    pulses: StepCount,
    rotation: StepCount,
    swing: NormalizedValue,

    // State
    pattern: [bool; MAX_STEPS],
    current_step: usize,
    sample_rate: SampleRate,
    gate_active: bool,
    samples_per_step: f32,
    sample_counter: f32,

    // Buffers
    gate_buffer: AudioBuffer,
    accent_buffer: AudioBuffer,
}

impl Euclidean {
    pub fn new() -> Self {
        let mut s = Self {
            steps: StepCount::new(16),
            pulses: StepCount::new(4),
            rotation: StepCount::new(0),
            swing: NormalizedValue::MIN,

            pattern: [false; MAX_STEPS],
            current_step: 0,
            sample_rate: SampleRate::DVD_QUALITY,
            gate_active: false,
            samples_per_step: 5512.5, // 120 BPM, 16th notes
            sample_counter: 0.0,

            gate_buffer: AudioBuffer::new(1024),
            accent_buffer: AudioBuffer::new(1024),
        };
        s.rebuild_pattern();
        s
    }

    /// Björklund algorithm to generate Euclidean rhythm.
    fn rebuild_pattern(&mut self) {
        let steps = self.steps.as_usize();
        let pulses = self.pulses.as_usize().min(steps);
        let rotation = self.rotation.as_usize() % steps.max(1);

        // Clear pattern
        self.pattern.fill(false);

        if steps == 0 || pulses == 0 {
            return;
        }

        if pulses >= steps {
            // All steps are pulses
            for slot in self.pattern.iter_mut().take(steps) {
                *slot = true;
            }
            return;
        }

        // Björklund algorithm
        let mut counts = vec![0u32; steps];
        let mut remainders = vec![0u32; steps];
        let mut level = 0;

        remainders[0] = pulses as u32;
        let mut divisor = (steps - pulses) as u32;

        loop {
            counts[level] = divisor / remainders[level];
            let new_remainder = divisor % remainders[level];
            divisor = remainders[level];
            remainders[level + 1] = new_remainder;
            level += 1;
            if remainders[level] <= 1 {
                break;
            }
        }
        counts[level] = divisor;

        // Build pattern from the tree
        let mut idx = 0;
        fn build(
            pattern: &mut [bool; MAX_STEPS],
            idx: &mut usize,
            level: i32,
            counts: &[u32],
            remainders: &[u32],
            _max_level: usize,
        ) {
            if level == -1 {
                if *idx < MAX_STEPS {
                    pattern[*idx] = false;
                    *idx += 1;
                }
            } else if level == -2 {
                if *idx < MAX_STEPS {
                    pattern[*idx] = true;
                    *idx += 1;
                }
            } else {
                let l = level as usize;
                for _ in 0..counts[l] {
                    build(pattern, idx, level - 1, counts, remainders, _max_level);
                }
                if remainders[l] != 0 {
                    build(pattern, idx, level - 2, counts, remainders, _max_level);
                }
            }
        }

        build(
            &mut self.pattern,
            &mut idx,
            level as i32,
            &counts,
            &remainders,
            level,
        );

        // Apply rotation
        if rotation > 0 {
            let mut rotated = [false; MAX_STEPS];
            for i in 0..steps {
                rotated[i] = self.pattern[(i + steps - rotation) % steps];
            }
            self.pattern = rotated;
        }
    }
}

impl Default for Euclidean {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Euclidean {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("euclidean", "Euclidean")
            .description("Euclidean rhythm sequencer — distributes pulses evenly across steps")
            .category(ModuleCategory::LFO)
            .tag("euclidean")
            .tag("generative")
            .tag("sequencer")
            .parameter(
                ParameterDescriptor::float(
                    "steps",
                    Param::Euclidean(EuclideanParam::Steps(StepCount::new(16))),
                    "Steps",
                )
                .description("Number of steps in the pattern")
                .range(1.0, 32.0)
                .default(16.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pulses",
                    Param::Euclidean(EuclideanParam::Pulses(StepCount::new(4))),
                    "Pulses",
                )
                .description("Number of active pulses")
                .range(0.0, 32.0)
                .default(4.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "rotation",
                    Param::Euclidean(EuclideanParam::Rotation(StepCount::new(0))),
                    "Rotation",
                )
                .description("Pattern rotation offset")
                .range(0.0, 31.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "swing",
                    Param::Euclidean(EuclideanParam::Swing(NormalizedValue::MIN)),
                    "Swing",
                )
                .description("Swing amount")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("clock", "Clock").description("External clock input"),
            )
            .port(PortDescriptor::audio_output("gate", "Gate").description("Gate output"))
            .port(PortDescriptor::audio_output("accent", "Accent").description("Accent CV output"))
    }
}

impl PolyModule for Euclidean {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.gate_buffer.resize(num_samples);
        self.accent_buffer.resize(num_samples);

        let clock = inputs.get(PortName::intern("clock"));

        // Calculate samples per step from BPM (16th notes by default)
        let bpm = context.tempo.as_f32().max(20.0);
        self.samples_per_step = self.sample_rate.as_f32() * 60.0 / bpm / 4.0;

        let steps = self.steps.as_usize();
        let swing_amount = self.swing.as_f32() * 0.33; // Max 33% swing

        for i in 0..num_samples {
            // Check for external clock rising edge
            let clock_trigger = if let Some(clk) = clock {
                clk[i] > 0.5 && (i == 0 || clk[i - 1] <= 0.5)
            } else {
                false
            };

            // Internal clock or external
            let advance = if clock.is_some() {
                clock_trigger
            } else {
                // Apply swing to even steps
                let swing_offset = if self.current_step % 2 == 1 {
                    swing_amount * self.samples_per_step
                } else {
                    0.0
                };
                self.sample_counter += 1.0;
                if self.sample_counter >= self.samples_per_step + swing_offset {
                    self.sample_counter = 0.0;
                    true
                } else {
                    false
                }
            };

            if advance && steps > 0 {
                self.current_step = (self.current_step + 1) % steps;
                self.gate_active = self.pattern[self.current_step];
            }

            // Gate output (short gate, ~50% duty cycle)
            let gate_phase = self.sample_counter / self.samples_per_step;
            let gate_on = self.gate_active && gate_phase < 0.5;
            self.gate_buffer[i] = if gate_on { 1.0 } else { 0.0 };

            // Accent: stronger on downbeat (step 0) and every 4th pulse
            let accent = if gate_on && self.current_step.is_multiple_of(4) {
                1.0
            } else if gate_on {
                0.6
            } else {
                0.0
            };
            self.accent_buffer[i] = accent;
        }

        if let Some(out) = outputs.get_mut(&PortName::GATE) {
            out.copy_from(&self.gate_buffer);
        }
        if let Some(out) = outputs.get_mut(&PortName::ACCENT) {
            out.copy_from(&self.accent_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Euclidean(p) = param {
            match p {
                EuclideanParam::Steps(n) => {
                    self.steps = StepCount::new(n.as_u8().clamp(1, 32));
                    self.rebuild_pattern();
                }
                EuclideanParam::Pulses(n) => {
                    self.pulses = StepCount::new(n.as_u8().min(32));
                    self.rebuild_pattern();
                }
                EuclideanParam::Rotation(n) => {
                    self.rotation = StepCount::new(n.as_u8().min(31));
                    self.rebuild_pattern();
                }
                EuclideanParam::Swing(v) => self.swing = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Euclidean(p) = param {
            Some(match p {
                EuclideanParam::Steps(_) => self.steps.as_f32(),
                EuclideanParam::Pulses(_) => self.pulses.as_f32(),
                EuclideanParam::Rotation(_) => self.rotation.as_f32(),
                EuclideanParam::Swing(_) => self.swing.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Euclidean(EuclideanParam::Steps(self.steps)),
            Param::Euclidean(EuclideanParam::Pulses(self.pulses)),
            Param::Euclidean(EuclideanParam::Rotation(self.rotation)),
            Param::Euclidean(EuclideanParam::Swing(self.swing)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Euclidean
    }

    fn reset(&mut self) {
        self.current_step = 0;
        self.sample_counter = 0.0;
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
    fn test_euclidean_pattern_4_16() {
        let mut seq = Euclidean::new();
        seq.steps = StepCount::new(16);
        seq.pulses = StepCount::new(4);
        seq.rebuild_pattern();

        let active: usize = seq.pattern[..16].iter().filter(|&&x| x).count();
        assert_eq!(active, 4, "E(4,16) should have 4 pulses");
    }

    #[test]
    fn test_euclidean_pattern_all_pulses() {
        let mut seq = Euclidean::new();
        seq.steps = StepCount::new(8);
        seq.pulses = StepCount::new(8);
        seq.rebuild_pattern();

        let active: usize = seq.pattern[..8].iter().filter(|&&x| x).count();
        assert_eq!(active, 8, "E(8,8) should have all 8 pulses");
    }
}
