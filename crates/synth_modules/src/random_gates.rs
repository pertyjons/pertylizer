//! Random Gates module.
//!
//! Generates probabilistic gate patterns with configurable density,
//! burst probability, and gate length.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParamModOffsets, ParameterDescriptor, PolyModule, PortDescriptor,
    ProcessContext, RandomGatesParam, WidgetHint,
};
use synth_core::{MidiNote, PortName, SampleRate, Velocity};

/// Random Gates voice module.
#[derive(Clone)]
pub struct RandomGates {
    // Parameters
    density: NormalizedValue,
    seed: u32,
    burst_probability: NormalizedValue,
    gate_length: NormalizedValue,

    // State
    rng_state: u32,
    /// Voice slot this instance drives, folded into the seed so a chord's voices
    /// get decorrelated gate patterns instead of firing in lockstep.
    voice_index: u32,
    sample_rate: SampleRate,
    step_counter: f32,
    samples_per_step: f32,
    gate_active: bool,
    gate_remaining: f32,
    burst_remaining: u8,
    cv_value: NormalizedValue,

    // Edge detection
    prev_clock: f32,

    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    // Buffers
    gate_buffer: AudioBuffer,
    cv_buffer: AudioBuffer,
}

impl RandomGates {
    pub fn new() -> Self {
        Self {
            density: NormalizedValue::CENTER,
            seed: 42,
            burst_probability: NormalizedValue::new(0.1),
            gate_length: NormalizedValue::CENTER,

            rng_state: 42,
            voice_index: 0,
            sample_rate: SampleRate::DVD_QUALITY,
            step_counter: 0.0,
            samples_per_step: 5512.5,
            gate_active: false,
            gate_remaining: 0.0,
            burst_remaining: 0,
            cv_value: NormalizedValue::MIN,

            prev_clock: 0.0,

            mod_offsets: ParamModOffsets::new(),

            gate_buffer: AudioBuffer::new(1024),
            cv_buffer: AudioBuffer::new(1024),
        }
    }

    /// Simple xorshift PRNG.
    #[inline]
    fn next_random(&mut self) -> f32 {
        crate::math::xorshift32(&mut self.rng_state)
    }

    /// Derive the RNG state from the `seed` param and this instance's voice slot,
    /// so a patch's voices decorrelate: voice 0 keeps the bare seed, the rest are
    /// spread by a golden-ratio hash. Kept non-zero (xorshift stalls at 0).
    #[inline]
    fn seeded_state(&self) -> u32 {
        self.seed
            .wrapping_add(self.voice_index.wrapping_mul(0x9E37_79B9))
            .max(1)
    }
}

impl Default for RandomGates {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for RandomGates {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("random_gates", "Random Gates")
            .description("Probabilistic gate generator with burst mode")
            .category(ModuleCategory::LFO)
            .tag("random")
            .tag("generative")
            .tag("gates")
            .parameter(
                ParameterDescriptor::float(
                    "density",
                    Param::RandomGates(RandomGatesParam::Density(NormalizedValue::CENTER)),
                    "Density",
                )
                .description("Probability of gate trigger per step")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "seed",
                    Param::RandomGates(RandomGatesParam::Seed(42)),
                    "Seed",
                )
                .description("Random seed for reproducibility")
                .range(0.0, 65535.0)
                .default(42.0)
                // Structural RNG seed: not a continuous modulation target.
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "burst",
                    Param::RandomGates(RandomGatesParam::BurstProbability(NormalizedValue::new(
                        0.1,
                    ))),
                    "Burst",
                )
                .description("Probability of triggering a burst")
                .range(0.0, 1.0)
                .default(0.1)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "gate_len",
                    Param::RandomGates(RandomGatesParam::GateLength(NormalizedValue::CENTER)),
                    "Gate Len",
                )
                .description("Gate length (short to long)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("clock", "Clock").description("External clock input"),
            )
            .port(PortDescriptor::audio_output("gate", "Gate").description("Gate output"))
            .port(PortDescriptor::audio_output("cv", "CV").description("Random CV output"))
    }
}

impl PolyModule for RandomGates {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.gate_buffer.resize(num_samples);
        self.cv_buffer.resize(num_samples);

        let clock = inputs.get(PortName::CLOCK);

        // Generic mod offsets — all per-block constants, resolved once here.
        let eff_density = self.mod_offsets.effective("density", self.density.as_f32());
        let eff_burst = self
            .mod_offsets
            .effective("burst", self.burst_probability.as_f32());
        let eff_gate_len = self
            .mod_offsets
            .effective("gate_len", self.gate_length.as_f32());

        // Calculate step timing from BPM (16th notes)
        let bpm = context.tempo.as_f32().max(20.0);
        self.samples_per_step = crate::math::samples_per_16th(self.sample_rate.as_f32(), bpm);

        // Gate length in samples
        let gate_samples = eff_gate_len * self.samples_per_step * 0.9 + 10.0;

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
                // Handle burst mode
                if self.burst_remaining > 0 {
                    self.burst_remaining -= 1;
                    self.gate_active = true;
                    self.gate_remaining = gate_samples;
                    self.cv_value = NormalizedValue::new(self.next_random());
                } else {
                    // Normal density check
                    let trigger = self.next_random() < eff_density;
                    if trigger {
                        self.gate_active = true;
                        self.gate_remaining = gate_samples;
                        self.cv_value = NormalizedValue::new(self.next_random());

                        // Check for burst
                        if self.next_random() < eff_burst {
                            self.burst_remaining = 2 + (self.next_random() * 3.0) as u8; // 2-4 burst gates
                        }
                    }
                }
            }

            // Decay gate
            if self.gate_remaining > 0.0 {
                self.gate_remaining -= 1.0;
                self.gate_active = self.gate_remaining > 0.0;
            } else {
                self.gate_active = false;
            }

            self.gate_buffer[i] = if self.gate_active { 1.0 } else { 0.0 };
            self.cv_buffer[i] = self.cv_value.as_f32();
        }

        // Persist last clock sample for edge detection across buffers
        if let Some(clk) = clock
            && num_samples > 0
        {
            self.prev_clock = clk[num_samples - 1];
        }

        if let Some(out) = outputs.get_mut(&PortName::GATE) {
            out.copy_from(&self.gate_buffer);
        }
        if let Some(out) = outputs.get_mut(&PortName::CV) {
            out.copy_from(&self.cv_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::RandomGates(p) = param {
            match p {
                RandomGatesParam::Density(v) => self.density = v,
                RandomGatesParam::Seed(s) => {
                    self.seed = s;
                    self.rng_state = self.seeded_state();
                }
                RandomGatesParam::BurstProbability(v) => self.burst_probability = v,
                RandomGatesParam::GateLength(v) => self.gate_length = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::RandomGates(p) = param {
            Some(match p {
                RandomGatesParam::Density(_) => self.density.as_f32(),
                RandomGatesParam::Seed(_) => self.seed as f32,
                RandomGatesParam::BurstProbability(_) => self.burst_probability.as_f32(),
                RandomGatesParam::GateLength(_) => self.gate_length.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::RandomGates(RandomGatesParam::Density(self.density)),
            Param::RandomGates(RandomGatesParam::Seed(self.seed)),
            Param::RandomGates(RandomGatesParam::BurstProbability(self.burst_probability)),
            Param::RandomGates(RandomGatesParam::GateLength(self.gate_length)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::RandomGates
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.step_counter = 0.0;
        self.gate_active = false;
        self.gate_remaining = 0.0;
        self.burst_remaining = 0;
        self.rng_state = self.seeded_state();
    }

    fn set_seed(&mut self, seed: u64) {
        // Fold the 64-bit seed into the 32-bit generator; `reset` re-derives
        // `rng_state` from `self.seed` (+ voice slot), so store the seed.
        self.seed = seed as u32;
        self.rng_state = self.seeded_state();
    }

    fn set_voice_index(&mut self, voice_index: u32) {
        // Decorrelate per voice (the graph folds the module id into `voice_index`)
        // so a chord's random gates aren't identical across voices — matches
        // DriftGenerator. TuringMachine is deliberately left in lockstep: its
        // identity is a single evolving shift-register sequence.
        self.voice_index = voice_index;
        self.rng_state = self.seeded_state();
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
    fn test_random_gates_creation() {
        let rg = RandomGates::new();
        assert_eq!(rg.seed, 42);
    }

    /// `density` is a working mod destination via the generic store: driving it
    /// to 0 stops all gate triggers, and clearing reverts.
    #[test]
    fn density_mod_offset_gates_output() {
        // Many samples so several free-running steps elapse within the block.
        let n = 64000;
        let render = |offset: f32| -> f32 {
            let mut rg = RandomGates::new();
            let desc = rg.descriptor();
            rg.mod_offsets_mut().unwrap().populate(&desc);
            if offset != 0.0 {
                rg.set_mod_offset("density", offset);
            }
            let ctx = ProcessContext {
                samples: synth_core::SampleCount::new(n),
                ..ProcessContext::default()
            };
            let mut outs = HashMap::new();
            outs.insert(PortName::GATE, AudioBuffer::new(n));
            outs.insert(PortName::CV, AudioBuffer::new(n));
            rg.process(InputPorts::empty(), &mut outs, &ctx);
            let b = &outs[&PortName::GATE];
            (0..b.len()).map(|i| b[i]).sum::<f32>()
        };

        let base = render(0.0);
        assert!(
            base > 0.0,
            "default density should produce gates, got {base}"
        );

        let silent = render(-1.0); // density → 0
        assert_eq!(silent, 0.0, "density→0 should produce no gates: {silent}");
    }

    #[test]
    fn test_random_gates_deterministic() {
        let mut rg1 = RandomGates::new();
        let mut rg2 = RandomGates::new();

        // Same seed should produce same sequence
        let v1 = rg1.next_random();
        let v2 = rg2.next_random();
        assert_eq!(v1, v2, "Same seed should produce same random values");
    }

    /// `set_seed` re-seeds the RNG so two instances decorrelate (the Mod Grid's
    /// per-node seed path), while the same seed stays reproducible.
    #[test]
    fn set_seed_decorrelates_and_reproduces() {
        let n = 64000;
        let cv_sum = |seed: u64| -> f32 {
            let mut rg = RandomGates::new();
            rg.set_seed(seed);
            let ctx = ProcessContext {
                samples: synth_core::SampleCount::new(n),
                ..ProcessContext::default()
            };
            let mut outs = HashMap::new();
            outs.insert(PortName::GATE, AudioBuffer::new(n));
            outs.insert(PortName::CV, AudioBuffer::new(n));
            rg.process(InputPorts::empty(), &mut outs, &ctx);
            let b = &outs[&PortName::CV];
            (0..b.len()).map(|i| b[i]).sum::<f32>()
        };
        assert!(
            (cv_sum(12345) - cv_sum(999)).abs() > 1e-6,
            "different seeds should decorrelate the CV output"
        );
        assert_eq!(
            cv_sum(777),
            cv_sum(777),
            "the same seed must reproduce the same sequence"
        );
    }

    /// Per-voice decorrelation: voices of one patch (same seed, different
    /// `voice_index`) produce independent gate/CV streams instead of firing in
    /// lockstep, while voice 0 keeps the bare-seed sequence (backward-compatible).
    #[test]
    fn voices_decorrelate_from_the_seed() {
        let n = 64000;
        let cv_sum = |voice: Option<u32>| -> f32 {
            let mut rg = RandomGates::new();
            if let Some(v) = voice {
                rg.set_voice_index(v);
            }
            let ctx = ProcessContext {
                samples: synth_core::SampleCount::new(n),
                ..ProcessContext::default()
            };
            let mut outs = HashMap::new();
            outs.insert(PortName::GATE, AudioBuffer::new(n));
            outs.insert(PortName::CV, AudioBuffer::new(n));
            rg.process(InputPorts::empty(), &mut outs, &ctx);
            let b = &outs[&PortName::CV];
            (0..b.len()).map(|i| b[i]).sum::<f32>()
        };
        assert!(
            (cv_sum(Some(0)) - cv_sum(Some(1))).abs() > 1e-6,
            "different voice indices should decorrelate the output"
        );
        assert!(
            (cv_sum(Some(1)) - cv_sum(Some(2))).abs() > 1e-6,
            "adjacent voices should decorrelate too"
        );
        assert_eq!(
            cv_sum(Some(0)),
            cv_sum(None),
            "voice 0 must match the un-indexed bare-seed sequence"
        );
    }
}
