//! Chaotic Oscillator module.
//!
//! Implements Rössler and Lorenz chaotic systems as modulation sources.
//! Produces complex, non-repeating signals for organic modulation.
//!
//! Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/184-rossler-and-lorenz-oscillators.rst>
//! From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{BipolarValue, Hertz, MidiNote, NormalizedValue, PortName, SampleRate, Velocity};
use synth_core::{ChaoticOscParam, ChaoticSystem, ModuleType, Param};

/// Chaotic Oscillator — Rössler and Lorenz systems.
#[derive(Clone)]
pub struct ChaoticOsc {
    system: ChaoticSystem,
    rate: Hertz,
    chaos: NormalizedValue,
    depth: NormalizedValue,
    sample_rate: SampleRate,
    // State variables (x, y, z)
    x: f32,
    y: f32,
    z: f32,
    /// Generic mod-matrix offsets (rate, chaos, depth). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    output_buffer: AudioBuffer,
    output_buffer_y: AudioBuffer,
    /// Cached interned name for the custom "out_y" port. Interned once in the
    /// constructor so `process()` never calls `PortName::intern` (which locks).
    out_y_name: PortName,
    /// Cached interned name for the "reset" gate input (interned once; see above).
    reset_port: PortName,
    /// Previous Reset-gate level for rising-edge detection (persists across blocks).
    prev_reset: f32,
}

impl ChaoticOsc {
    pub fn new() -> Self {
        Self {
            system: ChaoticSystem::Rossler,
            rate: Hertz::new(1.0),
            chaos: NormalizedValue::new(0.5),
            depth: NormalizedValue::MAX,
            sample_rate: SampleRate::DVD_QUALITY,
            x: 0.1,
            y: 0.0,
            z: 0.0,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
            output_buffer_y: AudioBuffer::new(1024),
            out_y_name: PortName::intern("out_y"),
            reset_port: PortName::intern("reset"),
            prev_reset: 0.0,
        }
    }

    /// Rössler attractor: dx/dt = -y - z, dy/dt = x + a*y, dz/dt = b + z*(x - c)
    #[inline]
    fn rossler_step(&mut self, dt: f32) {
        let a = 0.2;
        let b = 0.2;
        // c controls chaos: 4.0 = periodic, 5.7 = chaotic, 18.0 = highly chaotic
        let c = 4.0 + self.chaos.as_f32() * 14.0;

        let dx = (-self.y - self.z) * dt;
        let dy = (self.x + a * self.y) * dt;
        let dz = (b + self.z * (self.x - c)) * dt;

        self.x += dx;
        self.y += dy;
        self.z += dz;

        // Soft limit to prevent divergence
        self.x = crate::math::soft_clip(self.x * 0.05) * 20.0;
        self.y = crate::math::soft_clip(self.y * 0.05) * 20.0;
        self.z = crate::math::soft_clip(self.z * 0.02) * 50.0;
    }

    /// Lorenz attractor: dx/dt = s*(y-x), dy/dt = x*(r-z) - y, dz/dt = x*y - b*z
    #[inline]
    fn lorenz_step(&mut self, dt: f32) {
        let sigma = 10.0;
        let beta = 8.0 / 3.0;
        // rho controls chaos: 28.0 = standard chaotic regime
        let rho = 10.0 + self.chaos.as_f32() * 40.0;

        let dx = sigma * (self.y - self.x) * dt;
        let dy = (self.x * (rho - self.z) - self.y) * dt;
        let dz = (self.x * self.y - beta * self.z) * dt;

        self.x += dx;
        self.y += dy;
        self.z += dz;

        // Soft limit to prevent divergence
        self.x = crate::math::soft_clip(self.x * 0.03) * 33.0;
        self.y = crate::math::soft_clip(self.y * 0.03) * 33.0;
        self.z = crate::math::soft_clip(self.z * 0.02) * 50.0;
    }
}

impl Default for ChaoticOsc {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for ChaoticOsc {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("chaotic_osc", "Chaotic Osc")
            .description("Rössler/Lorenz chaotic systems for complex modulation")
            .category(ModuleCategory::LFO)
            .tag("chaos")
            .tag("modulation")
            .tag("random")
            .parameter(
                ParameterDescriptor::choice(
                    "system",
                    Param::ChaoticOsc(ChaoticOscParam::System(ChaoticSystem::Rossler)),
                    "System",
                    ChaoticSystem::to_choices(),
                )
                .description("Chaotic dynamical system"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "rate",
                    Param::ChaoticOsc(ChaoticOscParam::Rate(Hertz::new(1.0))),
                    "Rate",
                )
                .description("Iteration speed")
                .range(0.01, 20.0)
                .default(1.0)
                .widget(WidgetHint::Knob)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "chaos",
                    Param::ChaoticOsc(ChaoticOscParam::Chaos(NormalizedValue::new(0.5))),
                    "Chaos",
                )
                .description("System parameter (low=periodic, high=chaotic)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "depth",
                    Param::ChaoticOsc(ChaoticOscParam::Depth(NormalizedValue::MAX)),
                    "Depth",
                )
                .description("Output amplitude")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::gate_input("reset", "Reset")
                    .description("Rising edge re-seeds the attractor. Connect: Gate, Clock, LFO"),
            )
            .port(
                PortDescriptor::control_input("rate_cv", "Rate CV")
                    .description("Modulate iteration speed (exp FM). Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out").description(
                    "X-axis output (±depth). Connect to: Filter Cutoff CV, Oscillator FM",
                ),
            )
            .port(
                PortDescriptor::audio_output("out_y", "Y Out")
                    .description("Y-axis output (±depth). Second chaotic dimension"),
            )
    }
}

impl PolyModule for ChaoticOsc {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let sr = self.sample_rate.as_f32();
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);
        self.output_buffer_y.resize(num_samples);

        // Generic mod offsets (per-block constants). `chaos` is read deep inside
        // the per-system step functions, so apply its effective value to the
        // field for the block and restore after — the single loop has no early
        // return. rate/depth are read here and wrapped inline.
        let base_rate_hz = self.mod_offsets.effective("rate", self.rate.as_f32());
        let base_dt = base_rate_hz / sr;
        let depth = self.mod_offsets.effective("depth", self.depth.as_f32());
        let saved_chaos = self.chaos;
        self.chaos = NormalizedValue::new(self.mod_offsets.effective("chaos", self.chaos.as_f32()));

        // CV/gate inputs (unconnected readers return 0.0 → no change): Rate CV
        // exp-FMs the iteration speed per sample; a Reset rising edge re-seeds
        // the attractor.
        let rate_cv = inputs.reader(PortName::RATE_CV, 0.0);
        let reset_gate = inputs.reader(self.reset_port, 0.0);

        for i in 0..num_samples {
            // Reset on a rising edge of the gate input — re-seed the attractor.
            let g = reset_gate.get(i);
            if reset_gate.is_connected() && crate::math::rising_edge(g, self.prev_reset) {
                self.x = 0.1;
                self.y = 0.0;
                self.z = 0.0;
            }
            self.prev_reset = g;

            let dt = if rate_cv.is_connected() {
                Hertz::new(base_rate_hz)
                    .apply_fm(BipolarValue::new(rate_cv.get(i)))
                    .as_f32()
                    / sr
            } else {
                base_dt
            };
            match self.system {
                ChaoticSystem::Rossler => self.rossler_step(dt),
                ChaoticSystem::Lorenz => self.lorenz_step(dt),
            }

            // Normalize X and Y outputs to approximately ±1 range per sample
            let (norm_x, norm_y) = match self.system {
                ChaoticSystem::Rossler => (self.x / 12.0, self.y / 12.0),
                ChaoticSystem::Lorenz => (self.x / 20.0, self.y / 25.0),
            };

            self.output_buffer[i] = (norm_x * depth).clamp(-1.0, 1.0);
            self.output_buffer_y[i] = (norm_y * depth).clamp(-1.0, 1.0);
        }

        // Restore the base chaos so next block re-applies the offset from scratch.
        self.chaos = saved_chaos;

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }

        if let Some(out_y) = outputs.get_mut(&self.out_y_name) {
            out_y.copy_from(&self.output_buffer_y);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::ChaoticOsc(p) = param {
            match p {
                ChaoticOscParam::System(s) => self.system = s,
                ChaoticOscParam::Rate(r) => self.rate = Hertz::new(r.as_f32().clamp(0.01, 20.0)),
                ChaoticOscParam::Chaos(c) => self.chaos = c,
                ChaoticOscParam::Depth(d) => self.depth = d,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::ChaoticOsc(p) = param {
            Some(match p {
                ChaoticOscParam::System(_) => self.system.index() as f32,
                ChaoticOscParam::Rate(_) => self.rate.as_f32(),
                ChaoticOscParam::Chaos(_) => self.chaos.as_f32(),
                ChaoticOscParam::Depth(_) => self.depth.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::ChaoticOsc(ChaoticOscParam::System(self.system)),
            Param::ChaoticOsc(ChaoticOscParam::Rate(self.rate)),
            Param::ChaoticOsc(ChaoticOscParam::Chaos(self.chaos)),
            Param::ChaoticOsc(ChaoticOscParam::Depth(self.depth)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::ChaoticOsc
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.x = 0.1;
        self.y = 0.0;
        self.z = 0.0;
        self.prev_reset = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `depth` is a working mod destination via the generic store: a negative
    /// offset shrinks the output, and the transiently-applied `chaos` field is
    /// restored after each block (no drift).
    #[test]
    fn depth_mod_offset_scales_output_and_chaos_restores() {
        let mut osc = ChaoticOsc::new();
        let desc = osc.descriptor();
        osc.mod_offsets_mut().unwrap().populate(&desc);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            ..ProcessContext::default()
        };
        fn peak(osc: &mut ChaoticOsc, ctx: &ProcessContext) -> f32 {
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            osc.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut osc, &ctx);
        assert!(base > 1e-3, "base output present, got {base}");

        osc.set_mod_offset("depth", -0.8);
        let chaos_before = osc.chaos.as_f32();
        let quieter = peak(&mut osc, &ctx);
        osc.set_mod_offset("chaos", 0.5); // also exercise the transient field
        let _ = peak(&mut osc, &ctx);
        assert!(
            (osc.chaos.as_f32() - chaos_before).abs() < 1e-6,
            "chaos field must be restored after process"
        );
        assert!(
            quieter < base,
            "depth offset should reduce output: {quieter} vs {base}"
        );

        osc.clear_mod_offsets();
        let reverted = peak(&mut osc, &ctx);
        assert!(reverted > quieter, "clearing restores depth");
    }

    #[test]
    fn test_chaotic_osc_does_not_diverge() {
        let mut osc = ChaoticOsc::new();
        osc.chaos = NormalizedValue::MAX;
        osc.depth = NormalizedValue::MAX;
        let context = ProcessContext::default();
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(256));
        let inputs = InputPorts::empty();

        for _ in 0..1000 {
            osc.process(inputs, &mut outputs, &context);
        }

        if let Some(out) = outputs.get(&PortName::OUT) {
            for i in 0..256 {
                assert!(out[i].is_finite(), "Output diverged to non-finite");
                assert!(
                    out[i] >= -1.0 && out[i] <= 1.0,
                    "Output out of bounds: {}",
                    out[i]
                );
            }
        }
    }
}
