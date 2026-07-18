//! Drift Generator module.
//!
//! Smooth, bounded random modulation that mimics analog oscillator instability.
//! Produces natural pitch drift, filter wander, and amplitude variation.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/183-drift-generator.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{
    BipolarValue, Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity,
};
use synth_core::{DriftGeneratorParam, ModuleType, Param};

/// Drift Generator — smooth bounded random modulation source.
#[derive(Clone)]
pub struct DriftGenerator {
    rate: Hertz,
    depth: NormalizedValue,
    smoothness: NormalizedValue,
    sample_rate: SampleRate,
    /// Current drift value (bipolar -1..1)
    current: f32,
    /// Target drift value
    target: f32,
    /// Phase for controlling when to pick new target
    phase: Phase,
    /// Per-instance xorshift RNG state. Replaces the former global `fastrand`, so
    /// an offline render reproduces the live wander and a Mod Grid node's `seed`
    /// (or a voice index) decorrelates instances. Never zero (xorshift stalls at
    /// 0); seeded via [`PolyModule::set_seed`] / [`PolyModule::set_voice_index`].
    rng_state: u32,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    output_buffer: AudioBuffer,
}

impl DriftGenerator {
    pub fn new() -> Self {
        Self {
            rate: Hertz::new(0.2),
            depth: NormalizedValue::new(0.5),
            smoothness: NormalizedValue::new(0.7),
            sample_rate: SampleRate::DVD_QUALITY,
            current: 0.0,
            target: 0.0,
            phase: Phase::ZERO,
            rng_state: 0x9E37_79B9,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Generate a new random target using sine-warped random walk. Advances the
    /// per-instance RNG (RT-safe, deterministic — offline render matches live).
    #[inline]
    fn new_target(&mut self) -> f32 {
        let r = crate::math::xorshift32(&mut self.rng_state) * 2.0 - 1.0;
        // Sine-warp for more natural distribution (more time near center)
        (r * std::f32::consts::FRAC_PI_2).sin()
    }
}

impl Default for DriftGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for DriftGenerator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("drift_generator", "Drift Generator")
            .width(synth_core::ModuleWidth::Medium)
            .description("Smooth random modulation for analog-style drift")
            .category(ModuleCategory::LFO)
            .tag("drift")
            .tag("modulation")
            .tag("random")
            .parameter(
                ParameterDescriptor::float(
                    "rate",
                    Param::DriftGenerator(DriftGeneratorParam::Rate(Hertz::new(0.2))),
                    "Rate",
                )
                .description("How fast the drift wanders")
                .range(0.01, 5.0)
                .default(0.2)
                .widget(WidgetHint::Knob)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "depth",
                    Param::DriftGenerator(DriftGeneratorParam::Depth(NormalizedValue::new(0.5))),
                    "Depth",
                )
                .description("Output amplitude")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "smoothness",
                    Param::DriftGenerator(DriftGeneratorParam::Smoothness(NormalizedValue::new(
                        0.7,
                    ))),
                    "Smooth",
                )
                .description("Higher = smoother transitions")
                .range(0.0, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("rate_cv", "Rate CV")
                    .description("Modulate wander speed (exp FM). Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out").description(
                    "Drift signal (±depth). Connect to: Oscillator FM, Filter Cutoff CV",
                ),
            )
    }
}

impl PolyModule for DriftGenerator {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        // Generic mod offsets — per-block constants. `rate` is additionally a
        // per-sample CV destination (Rate CV) below; depth/smoothness stay block.
        let eff_rate = Hertz::new(self.mod_offsets.effective("rate", self.rate.as_f32()));
        let smoothness = self
            .mod_offsets
            .effective("smoothness", self.smoothness.as_f32());
        let depth = self.mod_offsets.effective("depth", self.depth.as_f32());

        // Smoothness controls the slew rate (0.0 = instant, 1.0 = very slow). The
        // phase increment and the one-pole coefficient both derive from the rate,
        // so the Rate CV recomputes them per sample; unconnected → block constants.
        let slew = 1.0 - smoothness * 0.999;
        let sr = self.sample_rate.as_f32();
        let coeff_of = |rate: Hertz| (slew * rate.as_f32() / sr * 100.0).clamp(0.0001, 1.0);
        let base_phase_inc = eff_rate.phase_increment(self.sample_rate);
        let base_coeff = coeff_of(eff_rate);
        let rate_cv = inputs.reader(PortName::RATE_CV, 0.0);

        for i in 0..context.samples.as_usize() {
            let (phase_inc, coeff) = if rate_cv.is_connected() {
                let r = eff_rate.apply_fm(BipolarValue::new(rate_cv.get(i)));
                (r.phase_increment(self.sample_rate), coeff_of(r))
            } else {
                (base_phase_inc, base_coeff)
            };

            let old_phase = self.phase.as_f32();
            self.phase = self.phase.advance(phase_inc);

            // Pick new target when phase wraps around
            if self.phase.as_f32() < old_phase {
                self.target = self.new_target();
            }

            // Smooth interpolation toward target (one-pole lowpass)
            self.current += (self.target - self.current) * coeff;

            self.output_buffer[i] = self.current * depth;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::DriftGenerator(p) = param {
            match p {
                DriftGeneratorParam::Rate(r) => self.rate = Hertz::new(r.as_f32().clamp(0.01, 5.0)),
                DriftGeneratorParam::Depth(d) => self.depth = d,
                DriftGeneratorParam::Smoothness(s) => self.smoothness = s,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::DriftGenerator(p) = param {
            Some(match p {
                DriftGeneratorParam::Rate(_) => self.rate.as_f32(),
                DriftGeneratorParam::Depth(_) => self.depth.as_f32(),
                DriftGeneratorParam::Smoothness(_) => self.smoothness.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::DriftGenerator(DriftGeneratorParam::Rate(self.rate)),
            Param::DriftGenerator(DriftGeneratorParam::Depth(self.depth)),
            Param::DriftGenerator(DriftGeneratorParam::Smoothness(self.smoothness)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::DriftGenerator
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.current = 0.0;
        self.target = 0.0;
        self.phase = Phase::ZERO;
    }

    fn set_seed(&mut self, seed: u64) {
        // Non-zero (xorshift stalls at 0). Used by the Mod Grid per-node seed.
        self.rng_state = (seed as u32).max(1);
    }

    fn set_voice_index(&mut self, voice_index: u32) {
        // Decorrelate per voice so a patch's drift isn't in lockstep across
        // voices (the graph already folds the module id into `voice_index`).
        self.rng_state = voice_index.max(1);
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

    /// `depth` is a working mod destination via the generic store: driving it to
    /// 0 silences the drift output, and clearing reverts.
    #[test]
    fn depth_mod_offset_scales_output() {
        let mut drift = DriftGenerator::new();
        let desc = drift.descriptor();
        drift.mod_offsets_mut().unwrap().populate(&desc);
        drift.depth = NormalizedValue::MAX;

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            ..ProcessContext::default()
        };
        // Seed a fixed drift value (current == target so it holds) to isolate the
        // depth scaling from the random walk.
        fn peak(drift: &mut DriftGenerator, ctx: &ProcessContext) -> f32 {
            drift.current = 0.6;
            drift.target = 0.6;
            drift.phase = Phase::ZERO;
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            drift.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut drift, &ctx);
        assert!(base > 1e-3, "base drift present, got {base}");

        drift.set_mod_offset("depth", -1.0);
        let silent = peak(&mut drift, &ctx);
        assert!(
            silent < base * 0.05,
            "depth→0 should silence drift: {silent}"
        );

        drift.clear_mod_offsets();
        assert!(peak(&mut drift, &ctx) > silent, "clearing restores depth");
    }

    /// The per-instance RNG makes the wander deterministic (offline == live) and
    /// seedable: the same seed reproduces the output, different seeds decorrelate.
    #[test]
    fn set_seed_is_deterministic_and_decorrelates() {
        let sum = |seed: u64| -> f32 {
            let mut drift = DriftGenerator::new();
            drift.set_seed(seed);
            drift.depth = NormalizedValue::MAX;
            // Fast rate so several targets are drawn within the window.
            drift.rate = Hertz::new(5.0);
            let ctx = ProcessContext {
                samples: synth_core::SampleCount::new(64000),
                ..ProcessContext::default()
            };
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(64000));
            drift.process(InputPorts::empty(), &mut outs, &ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i]).sum::<f32>()
        };
        assert_eq!(sum(42), sum(42), "same seed must reproduce the wander");
        assert!(
            (sum(42) - sum(999)).abs() > 1e-3,
            "different seeds must decorrelate the wander"
        );
    }

    #[test]
    fn test_drift_generator_output_bounded() {
        let mut drift = DriftGenerator::new();
        drift.depth = NormalizedValue::MAX;
        let context = ProcessContext::default();
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(256));
        let inputs = InputPorts::empty();

        // Run several buffers to let drift accumulate
        for _ in 0..100 {
            drift.process(inputs, &mut outputs, &context);
        }

        if let Some(out) = outputs.get(&PortName::OUT) {
            for i in 0..256 {
                assert!(
                    out[i] >= -1.0 && out[i] <= 1.0,
                    "Output out of bounds: {}",
                    out[i]
                );
            }
        }
    }
}
