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
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
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
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Compute one-pole coefficient from time in ms.
    #[inline]
    fn time_to_coeff(ms: Milliseconds, sample_rate: SampleRate) -> f32 {
        let time_secs = ms.as_f32() * 0.001;
        if time_secs < 0.000_01 {
            return 0.0; // Instant
        }
        (-1.0 / (time_secs * sample_rate.as_f32())).exp()
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
            .description("Envelope follower — tracks input amplitude as a control signal")
            .category(ModuleCategory::Utility)
            .tag("envelope")
            .tag("follower")
            .tag("utility")
            .tag("dynamics")
            .parameter(
                ParameterDescriptor::float(
                    Param::EnvelopeFollower(EnvelopeFollowerParam::Attack(Milliseconds::new(5.0))),
                    "Attack",
                )
                .description("How fast the follower rises")
                .range(0.1, 100.0)
                .default(5.0)
                .unit(ParameterUnit::Milliseconds)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::EnvelopeFollower(EnvelopeFollowerParam::Release(Milliseconds::new(
                        50.0,
                    ))),
                    "Release",
                )
                .description("How fast the follower falls")
                .range(1.0, 1000.0)
                .default(50.0)
                .unit(ParameterUnit::Milliseconds)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
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
                PortDescriptor::audio_output("out", "Out").description("Envelope output (0.0-1.0)"),
            )
    }
}

impl PolyModule for EnvelopeFollower {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let input = inputs.get(PortName::IN);

        let attack_coeff = Self::time_to_coeff(self.attack, self.sample_rate);
        let release_coeff = Self::time_to_coeff(self.release, self.sample_rate);
        let sensitivity_scale = self.sensitivity.as_f32() * 4.0;

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

        if let Some(out) = outputs.get_mut("out") {
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
        outputs.insert("out".to_string(), AudioBuffer::new(64));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        ef.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs["out"];
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
