//! Ring Modulation module.
//!
//! Multiplies an input signal with an internal carrier oscillator to produce
//! sum and difference frequency sidebands. Classic ring mod effect used for
//! metallic tones, bell-like sounds, and atonal textures.
//!
//! Features:
//! - Internal carrier oscillator (sine, triangle, saw, square, pulse)
//! - Keyboard tracking for harmonic ring modulation
//! - Frequency ratio control for musical intervals
//! - Dry/wet mix

use std::collections::HashMap;
use std::f32::consts::TAU;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{
    BipolarValue, Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity, Waveform,
};
use synth_core::{ModuleType, Param, RingModParam};
use synth_dsp::oscillators::poly_blep;

/// Ring modulator voice module.
#[derive(Clone)]
pub struct RingMod {
    // Parameters
    carrier_freq: Hertz,
    carrier_waveform: Waveform,
    mix: NormalizedValue,
    freq_ratio: NormalizedValue,
    track_keyboard: NormalizedValue,

    // State
    carrier_phase: Phase,
    note_freq: Hertz,
    sample_rate: SampleRate,

    // Buffers
    output_buffer: AudioBuffer,
}

impl RingMod {
    pub fn new() -> Self {
        Self {
            carrier_freq: Hertz::new(440.0),
            carrier_waveform: Waveform::Sine,
            mix: NormalizedValue::new(0.5),
            freq_ratio: NormalizedValue::new(0.5),
            track_keyboard: NormalizedValue::MIN,
            carrier_phase: Phase::ZERO,
            note_freq: Hertz::A4,
            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Compute the effective carrier frequency based on parameters.
    #[inline]
    fn effective_carrier_freq(&self) -> Hertz {
        // Freq ratio maps 0.0-1.0 to 0.25x-4.0x multiplier
        let ratio = 0.25 * (16.0_f32).powf(self.freq_ratio.as_f32());

        // Keyboard tracking: interpolate between fixed and note-relative
        let tracking = self.track_keyboard.as_f32();
        let fixed = self.carrier_freq.as_f32() * ratio;
        let tracked = self.note_freq.as_f32() * ratio;

        Hertz::new(fixed * (1.0 - tracking) + tracked * tracking)
    }

    /// Generate a carrier sample at the given phase.
    #[inline]
    fn carrier_sample(&self, p: f32, dt: f32) -> f32 {
        match self.carrier_waveform {
            Waveform::Sine => (p * TAU).sin(),
            Waveform::Triangle => crate::math::triangle_wave(p),
            Waveform::Sawtooth => {
                let mut saw = 2.0 * p - 1.0;
                saw -= poly_blep(p, dt);
                saw
            }
            Waveform::Square | Waveform::Pulse => {
                let mut sq = if p < 0.5 { 1.0 } else { -1.0 };
                sq += poly_blep(p, dt);
                sq -= poly_blep((p + 0.5).rem_euclid(1.0), dt);
                sq
            }
            Waveform::DsfSaw => {
                // DSF saw approximation: fall back to naive saw for ring mod carrier
                2.0 * p - 1.0
            }
        }
    }
}

impl Default for RingMod {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for RingMod {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("ring_mod", "Ring Mod")
            .description("Ring modulator — multiplies input with internal carrier oscillator")
            .category(ModuleCategory::Oscillator)
            .tag("ring_mod")
            .tag("modulation")
            .tag("metallic")
            .parameter(
                ParameterDescriptor::float(
                    "carrier_freq",
                    Param::RingMod(RingModParam::CarrierFreq(Hertz::new(440.0))),
                    "Carrier Freq",
                )
                .description("Carrier oscillator frequency")
                .range(0.1, 20000.0)
                .default(440.0)
                .unit(ParameterUnit::Hertz)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "carrier_wave",
                    Param::RingMod(RingModParam::CarrierWaveform(Waveform::Sine)),
                    "Carrier Wave",
                    Waveform::ALL
                        .iter()
                        .map(|w| {
                            synth_core::ChoiceOption::new(w.id(), w.name())
                                .with_description(w.description())
                        })
                        .collect(),
                )
                .description("Carrier oscillator waveform")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::RingMod(RingModParam::Mix(NormalizedValue::new(0.5))),
                    "Mix",
                )
                .description("Dry/wet mix (0=dry, 1=ring mod)")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "freq_ratio",
                    Param::RingMod(RingModParam::FreqRatio(NormalizedValue::new(0.5))),
                    "Freq Ratio",
                )
                .description("Carrier frequency ratio (0.25x to 4.0x)")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "key_track",
                    Param::RingMod(RingModParam::TrackKeyboard(NormalizedValue::MIN)),
                    "Key Track",
                )
                .description("Keyboard tracking (0=fixed, 1=track note)")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Audio to modulate. Connect: Oscillator Out, Filter Out"),
            )
            .port(
                PortDescriptor::control_input("freq_cv", "Freq CV")
                    .description("Modulates carrier frequency. Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Ring-modulated output. Connect to: Amplifier In, Filter In"),
            )
    }
}

impl PolyModule for RingMod {
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
        let freq_cv = inputs.get(PortName::FREQ_CV);

        let carrier_freq = self.effective_carrier_freq();
        let mix = self.mix.as_f32();

        for i in 0..num_samples {
            let in_sample = input.map_or(0.0, |buf| buf[i]);

            // Apply frequency CV if connected
            let freq = if let Some(cv) = freq_cv {
                // CV scales freq exponentially: +1V = double freq
                carrier_freq.apply_cv(BipolarValue::new(cv[i]))
            } else {
                carrier_freq
            };

            let dt = freq.phase_increment(self.sample_rate);
            let p = self.carrier_phase.as_f32();

            let carrier = self.carrier_sample(p, dt);
            self.carrier_phase = self.carrier_phase.advance(dt);

            // Ring mod: input * carrier, with dry/wet mix
            let ring = in_sample * carrier;
            self.output_buffer[i] = crate::math::linear_mix(in_sample, ring, mix);
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::RingMod(p) = param {
            match p {
                RingModParam::CarrierFreq(h) => {
                    self.carrier_freq = Hertz::new(h.as_f32().clamp(0.1, 20000.0));
                }
                RingModParam::CarrierWaveform(w) => self.carrier_waveform = w,
                RingModParam::Mix(v) => self.mix = v,
                RingModParam::FreqRatio(v) => self.freq_ratio = v,
                RingModParam::TrackKeyboard(v) => self.track_keyboard = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::RingMod(p) = param {
            Some(match p {
                RingModParam::CarrierFreq(_) => self.carrier_freq.as_f32(),
                RingModParam::CarrierWaveform(_) => self.carrier_waveform.index() as f32,
                RingModParam::Mix(_) => self.mix.as_f32(),
                RingModParam::FreqRatio(_) => self.freq_ratio.as_f32(),
                RingModParam::TrackKeyboard(_) => self.track_keyboard.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::RingMod(RingModParam::CarrierFreq(self.carrier_freq)),
            Param::RingMod(RingModParam::CarrierWaveform(self.carrier_waveform)),
            Param::RingMod(RingModParam::Mix(self.mix)),
            Param::RingMod(RingModParam::FreqRatio(self.freq_ratio)),
            Param::RingMod(RingModParam::TrackKeyboard(self.track_keyboard)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::RingMod
    }

    fn reset(&mut self) {
        self.carrier_phase = Phase::ZERO;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.carrier_phase = Phase::ZERO;
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
    fn test_ring_mod_creation() {
        let rm = RingMod::new();
        assert_eq!(rm.carrier_freq.as_f32(), 440.0);
        assert_eq!(rm.carrier_waveform, Waveform::Sine);
    }

    #[test]
    fn test_ring_mod_silence_without_input() {
        let mut rm = RingMod::new();
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        rm.process(InputPorts::empty(), &mut outputs, &context);

        // Without input, output should be silent
        let out = &outputs[&PortName::OUT];
        for i in 0..64 {
            assert!(
                out[i].abs() < 0.001,
                "Expected silence without input, got {} at sample {}",
                out[i],
                i
            );
        }
    }

    #[test]
    fn test_ring_mod_params() {
        let mut rm = RingMod::new();
        rm.set_param(Param::RingMod(RingModParam::Mix(NormalizedValue::new(
            0.75,
        ))));
        assert!((rm.mix.as_f32() - 0.75).abs() < 0.001);

        let params = rm.get_params();
        assert_eq!(params.len(), 5);
    }
}
