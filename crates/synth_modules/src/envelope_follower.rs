//! Envelope Follower module.
//!
//! Tracks the amplitude of an input signal and produces a smooth
//! control signal (0.0-1.0). Useful for making one sound's dynamics
//! control another parameter (sidechain-style effects, auto-wah, etc.).
//!
//! Uses a one-pole filter with separate attack and release coefficients
//! for smooth, musical envelope tracking.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve,
    WidgetHint,
};
use synth_core::{EnvelopeFollowerParam, ModuleType, Param};
use synth_core::{
    FilterState, MidiNote, Milliseconds, NormalizedValue, PortName, SampleRate, Velocity,
};

/// Envelope follower voice module.
#[derive(Clone)]
pub struct EnvelopeFollower {
    // Parameters
    attack: Milliseconds,
    release: Milliseconds,
    sensitivity: NormalizedValue,

    // State
    envelope: FilterState,
    sample_rate: SampleRate,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    // Buffers
    output_buffer: AudioBuffer,
}

impl EnvelopeFollower {
    pub fn new() -> Self {
        Self {
            attack: Milliseconds::new(5.0),
            release: Milliseconds::new(50.0),
            sensitivity: NormalizedValue::new(0.5),
            envelope: FilterState::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }
}

impl Default for EnvelopeFollower {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for EnvelopeFollower {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("envelope_follower", "Env Follower")
            .width(synth_core::ModuleWidth::Medium)
            .description("Envelope follower — tracks input amplitude as a control signal")
            .category(ModuleCategory::Utility)
            .tag("envelope")
            .tag("follower")
            .tag("utility")
            .tag("dynamics")
            .parameter(
                ParameterDescriptor::float(
                    "attack",
                    Param::EnvelopeFollower(EnvelopeFollowerParam::Attack(Milliseconds::new(5.0))),
                    "Attack",
                )
                .description("How fast the follower rises")
                .range(0.1, 100.0)
                .default(5.0)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "release",
                    Param::EnvelopeFollower(EnvelopeFollowerParam::Release(Milliseconds::new(
                        50.0,
                    ))),
                    "Release",
                )
                .description("How fast the follower falls")
                .range(1.0, 1000.0)
                .default(50.0)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sensitivity",
                    Param::EnvelopeFollower(EnvelopeFollowerParam::Sensitivity(
                        NormalizedValue::new(0.5),
                    )),
                    "Sensitivity",
                )
                .description("Output gain/sensitivity")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Audio input to follow"))
            .port(
                PortDescriptor::control_output("out", "Out")
                    .description("Envelope output (0.0-1.0)"),
            )
    }
}

impl PolyModule for EnvelopeFollower {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let input = inputs.get(PortName::IN);

        // Generic mod offsets — all per-block constants, resolved once here.
        let attack_coeff =
            Milliseconds::new(self.mod_offsets.effective("attack", self.attack.as_f32()))
                .to_exp_coeff(self.sample_rate);
        let release_coeff =
            Milliseconds::new(self.mod_offsets.effective("release", self.release.as_f32()))
                .to_exp_coeff(self.sample_rate);
        let sensitivity_scale = self
            .mod_offsets
            .effective("sensitivity", self.sensitivity.as_f32())
            * 4.0;

        for i in 0..num_samples {
            let in_sample = input.map_or(0.0, |buf| buf[i]);
            let rectified = in_sample.abs();

            let current = self.envelope.as_f32();
            let coeff = if rectified > current {
                attack_coeff
            } else {
                release_coeff
            };

            let new_env = coeff * current + (1.0 - coeff) * rectified;
            self.envelope = FilterState::new(new_env);

            // Scale by sensitivity and clamp to 0.0-1.0
            self.output_buffer[i] = (new_env * sensitivity_scale).min(1.0);
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::EnvelopeFollower(p) = param {
            match p {
                EnvelopeFollowerParam::Attack(ms) => {
                    self.attack = Milliseconds::new(ms.as_f32().clamp(0.1, 100.0));
                }
                EnvelopeFollowerParam::Release(ms) => {
                    self.release = Milliseconds::new(ms.as_f32().clamp(1.0, 1000.0));
                }
                EnvelopeFollowerParam::Sensitivity(v) => self.sensitivity = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::EnvelopeFollower(p) = param {
            Some(match p {
                EnvelopeFollowerParam::Attack(_) => self.attack.as_f32(),
                EnvelopeFollowerParam::Release(_) => self.release.as_f32(),
                EnvelopeFollowerParam::Sensitivity(_) => self.sensitivity.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::EnvelopeFollower(EnvelopeFollowerParam::Attack(self.attack)),
            Param::EnvelopeFollower(EnvelopeFollowerParam::Release(self.release)),
            Param::EnvelopeFollower(EnvelopeFollowerParam::Sensitivity(self.sensitivity)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::EnvelopeFollower
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.envelope = FilterState::ZERO;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Optionally reset envelope on note-on for consistent attack
        self.envelope = FilterState::ZERO;
    }

    fn note_off(&mut self) {
        // Nothing to do
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sensitivity` is a working mod destination via the generic store: it
    /// scales the tracked output level, and clearing reverts.
    #[test]
    fn sensitivity_mod_offset_scales_output() {
        let mut ef = EnvelopeFollower::new();
        let desc = ef.descriptor();
        ef.mod_offsets_mut().unwrap().populate(&desc);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(512),
            ..ProcessContext::default()
        };
        fn level(ef: &mut EnvelopeFollower, ctx: &ProcessContext) -> f32 {
            ef.reset();
            let mut buf = AudioBuffer::new(512);
            for i in 0..512 {
                buf[i] = 0.1; // small steady input, well below the sensitivity clamp
            }
            let in_ports = [(PortName::IN, &buf)];
            let inputs = InputPorts::new(&in_ports);
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(512));
            ef.process(inputs, &mut outs, ctx);
            outs[&PortName::OUT][511]
        }

        let base = level(&mut ef, &ctx);
        assert!(base > 1e-3, "base tracks input, got {base}");

        ef.set_mod_offset("sensitivity", -0.4);
        let lower = level(&mut ef, &ctx);
        assert!(
            lower < base * 0.9,
            "sensitivity offset should lower output: {lower} vs {base}"
        );

        ef.clear_mod_offsets();
        assert!((level(&mut ef, &ctx) - base).abs() < base * 0.05);
    }

    #[test]
    fn test_envelope_follower_creation() {
        let ef = EnvelopeFollower::new();
        assert!((ef.attack.as_f32() - 5.0).abs() < 0.01);
        assert!((ef.release.as_f32() - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_envelope_follower_silence() {
        let mut ef = EnvelopeFollower::new();
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        ef.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs[&PortName::OUT];
        for i in 0..64 {
            assert!(
                out[i].abs() < 0.001,
                "Expected silence, got {} at sample {}",
                out[i],
                i
            );
        }
    }

    #[test]
    fn test_envelope_follower_params() {
        let mut ef = EnvelopeFollower::new();
        ef.set_param(Param::EnvelopeFollower(EnvelopeFollowerParam::Attack(
            Milliseconds::new(20.0),
        )));
        assert!((ef.attack.as_f32() - 20.0).abs() < 0.01);

        let params = ef.get_params();
        assert_eq!(params.len(), 3);
    }
}
