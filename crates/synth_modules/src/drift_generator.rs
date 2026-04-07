//! Drift Generator module.
//!
//! Smooth, bounded random modulation that mimics analog oscillator instability.
//! Produces natural pitch drift, filter wander, and amplitude variation.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/183-drift-generator.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{DriftGeneratorParam, ModuleType, Param};
use synth_core::{Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity};

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
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Generate a new random target using sine-warped random walk.
    #[inline]
    fn new_target(&self) -> f32 {
        // Bounded random: use fastrand for RT-safe random
        let r = fastrand::f32() * 2.0 - 1.0;
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
                .unit(ParameterUnit::Hertz)
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
                PortDescriptor::audio_output("out", "Out").description(
                    "Drift signal (±depth). Connect to: Oscillator FM, Filter Cutoff CV",
                ),
            )
    }
}

impl PolyModule for DriftGenerator {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let phase_inc = self.rate.phase_increment(self.sample_rate);
        // Smoothness controls the slew rate (0.0 = instant, 1.0 = very slow)
        let slew = 1.0 - self.smoothness.as_f32() * 0.999;
        // One-pole coefficient: lower = smoother
        let coeff = slew * self.rate.as_f32() / self.sample_rate.as_f32() * 100.0;
        let coeff = coeff.clamp(0.0001, 1.0);

        for i in 0..context.samples.as_usize() {
            let old_phase = self.phase.as_f32();
            self.phase = self.phase.advance(phase_inc);

            // Pick new target when phase wraps around
            if self.phase.as_f32() < old_phase {
                self.target = self.new_target();
            }

            // Smooth interpolation toward target (one-pole lowpass)
            self.current += (self.target - self.current) * coeff;

            self.output_buffer[i] = self.current * self.depth.as_f32();
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

    fn reset(&mut self) {
        self.current = 0.0;
        self.target = 0.0;
        self.phase = Phase::ZERO;
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

    #[test]
    fn test_drift_generator_output_bounded() {
        let mut drift = DriftGenerator::new();
        drift.depth = NormalizedValue::MAX;
        let context = ProcessContext::test_default();
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(256));
        let inputs = InputPorts::empty();

        // Run several buffers to let drift accumulate
        for _ in 0..100 {
            drift.process(inputs.clone(), &mut outputs, &context);
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
