//! AM Formantic Synthesis voice module.
//!
//! Generates vocal-like sounds using amplitude modulation with formant tables.
//! Three carrier oscillators run at formant frequencies (one per formant band),
//! each amplitude-modulated by a modulator at the note pitch. The vowel parameter
//! morphs between A/E/I/O/U formant tables.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/224-am-formantic-synthesis.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use std::collections::HashMap;

use synth_core::{AmFormantParam, ModuleType, Param};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity};

/// Number of formant bands (carrier oscillators).
const NUM_BANDS: usize = 3;
/// Number of vowels (A, E, I, O, U).
const NUM_VOWELS: usize = 5;

/// Formant frequencies for each vowel [vowel][band] in Hz.
const FORMANT_FREQ: [[f32; NUM_BANDS]; NUM_VOWELS] = [
    [800.0, 1150.0, 2900.0], // A (as in "father")
    [350.0, 2000.0, 2800.0], // E (as in "bed")
    [270.0, 2140.0, 3200.0], // I (as in "heed")
    [450.0, 800.0, 2830.0],  // O (as in "hot")
    [325.0, 700.0, 2530.0],  // U (as in "boot")
];

/// Formant amplitude weights per band (higher bands are quieter).
const FORMANT_GAIN: [[f32; NUM_BANDS]; NUM_VOWELS] = [
    [1.0, 0.5, 0.25],  // A
    [1.0, 0.5, 0.2],   // E
    [1.0, 0.35, 0.15], // I
    [1.0, 0.35, 0.2],  // O
    [1.0, 0.3, 0.15],  // U
];

/// AM Formantic Synthesis voice module.
///
/// Uses 3 carrier oscillators at formant frequencies, each amplitude-modulated
/// by a modulator oscillator running at the note pitch.
#[derive(Clone)]
pub struct AmFormant {
    // Parameters
    vowel: NormalizedValue,
    carrier_ratio: NormalizedValue,
    depth: NormalizedValue,
    level: NormalizedValue,

    // Oscillator state
    carrier_phases: [Phase; NUM_BANDS],
    modulator_phase: Phase,

    // Note tracking
    note_freq: Hertz,

    // Cached
    sample_rate: SampleRate,
    inv_sample_rate: f32,

    // Pre-allocated output buffer
    output_buffer: AudioBuffer,
}

impl AmFormant {
    pub fn new() -> Self {
        Self {
            vowel: NormalizedValue::MIN,
            carrier_ratio: NormalizedValue::new(0.5),
            depth: NormalizedValue::new(0.8),
            level: NormalizedValue::new(0.8),

            carrier_phases: [Phase::ZERO; NUM_BANDS],
            modulator_phase: Phase::ZERO,

            note_freq: Hertz::ZERO,

            sample_rate: SampleRate::DVD_QUALITY,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),

            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Interpolate formant frequencies and gains for the current vowel position.
    /// Returns ([freq; 3], [gain; 3]).
    #[inline]
    fn interpolated_formants(&self) -> ([f32; NUM_BANDS], [f32; NUM_BANDS]) {
        let pos = self.vowel.as_f32() * (NUM_VOWELS - 1) as f32;
        let idx = (pos as usize).min(NUM_VOWELS - 2);
        let frac = pos - idx as f32;

        let mut freqs = [0.0_f32; NUM_BANDS];
        let mut gains = [0.0_f32; NUM_BANDS];

        for band in 0..NUM_BANDS {
            freqs[band] =
                FORMANT_FREQ[idx][band] * (1.0 - frac) + FORMANT_FREQ[idx + 1][band] * frac;
            gains[band] =
                FORMANT_GAIN[idx][band] * (1.0 - frac) + FORMANT_GAIN[idx + 1][band] * frac;
        }

        (freqs, gains)
    }
}

impl Default for AmFormant {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for AmFormant {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("am_formant", "AM Formant")
            .description("AM formantic synthesis (vocal-like tones)")
            .category(ModuleCategory::Oscillator)
            .tag("formant")
            .tag("synthesis")
            .tag("vocal")
            .tag("am")
            .parameter(
                ParameterDescriptor::float(
                    "vowel",
                    Param::AmFormant(AmFormantParam::Vowel(NormalizedValue::MIN)),
                    "Vowel",
                )
                .description("Vowel morph (A -> E -> I -> O -> U)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "carrier_ratio",
                    Param::AmFormant(AmFormantParam::CarrierRatio(NormalizedValue::new(0.5))),
                    "Carrier",
                )
                .description("Scales carrier (formant) frequencies")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "depth",
                    Param::AmFormant(AmFormantParam::Depth(NormalizedValue::new(0.8))),
                    "Depth",
                )
                .description("AM modulation depth")
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::AmFormant(AmFormantParam::Level(NormalizedValue::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("AM formant output"))
    }
}

impl PolyModule for AmFormant {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.inv_sample_rate = 1.0 / context.sample_rate.as_f32();
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        // No note playing — output silence
        if self.note_freq.as_f32() <= 0.0 {
            for i in 0..num_samples {
                self.output_buffer[i] = 0.0;
            }
            if let Some(out) = outputs.get_mut(&PortName::OUT) {
                out.copy_from(&self.output_buffer);
            }
            return;
        }

        let (formant_freqs, formant_gains) = self.interpolated_formants();

        // CarrierRatio maps 0..1 to 0.25..4.0 (exponential scaling)
        let ratio = 0.25 * (16.0_f32).powf(self.carrier_ratio.as_f32());
        let depth = self.depth.as_f32();
        let level = self.level.as_f32();
        let inv_sr = self.inv_sample_rate;
        let mod_freq = self.note_freq.as_f32();

        // Compute per-band carrier frequency increments (scaled by ratio)
        let mut carrier_incs = [0.0_f32; NUM_BANDS];
        for band in 0..NUM_BANDS {
            carrier_incs[band] = formant_freqs[band] * ratio * inv_sr;
        }
        let mod_inc = mod_freq * inv_sr;

        // Normalization factor: sum of gains
        let gain_sum: f32 = formant_gains.iter().sum();
        let norm = if gain_sum > 1e-7 { 1.0 / gain_sum } else { 1.0 };

        for i in 0..num_samples {
            // Modulator: sine at note pitch
            let modulator = crate::math::fast_sin_turns(self.modulator_phase.as_f32());

            // AM envelope: 1.0 when depth=0 (no modulation), full AM when depth=1
            let am_envelope = 1.0 - depth + depth * (modulator * 0.5 + 0.5);

            // Sum carrier oscillators
            let mut sample = 0.0_f32;
            for band in 0..NUM_BANDS {
                let carrier = crate::math::fast_sin_turns(self.carrier_phases[band].as_f32());
                sample += carrier * formant_gains[band] * am_envelope;

                // Advance carrier phase
                let new_phase = self.carrier_phases[band].as_f32() + carrier_incs[band];
                self.carrier_phases[band] = Phase::new_unchecked(new_phase.fract());
            }

            // Normalize and apply level
            self.output_buffer[i] = sample * norm * level;

            // Advance modulator phase
            let new_mod = self.modulator_phase.as_f32() + mod_inc;
            self.modulator_phase = Phase::new_unchecked(new_mod.fract());
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::AmFormant(p) = param {
            match p {
                AmFormantParam::Vowel(v) => self.vowel = v,
                AmFormantParam::CarrierRatio(v) => self.carrier_ratio = v,
                AmFormantParam::Depth(v) => self.depth = v,
                AmFormantParam::Level(v) => self.level = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::AmFormant(p) = param {
            Some(match p {
                AmFormantParam::Vowel(_) => self.vowel.as_f32(),
                AmFormantParam::CarrierRatio(_) => self.carrier_ratio.as_f32(),
                AmFormantParam::Depth(_) => self.depth.as_f32(),
                AmFormantParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::AmFormant(AmFormantParam::Vowel(self.vowel)),
            Param::AmFormant(AmFormantParam::CarrierRatio(self.carrier_ratio)),
            Param::AmFormant(AmFormantParam::Depth(self.depth)),
            Param::AmFormant(AmFormantParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::AmFormant
    }

    fn reset(&mut self) {
        self.carrier_phases = [Phase::ZERO; NUM_BANDS];
        self.modulator_phase = Phase::ZERO;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.reset();
    }

    fn note_off(&mut self) {
        // Envelope controls release; nothing to do here
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.inv_sample_rate = 1.0 / sample_rate.as_f32();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_am_formant_creation() {
        let amf = AmFormant::new();
        assert!((amf.note_freq.as_f32() - 0.0).abs() < f32::EPSILON);
        assert_eq!(amf.carrier_phases, [Phase::ZERO; NUM_BANDS]);
    }

    #[test]
    fn test_am_formant_produces_sound() {
        let mut amf = AmFormant::new();
        amf.note_on(MidiNote::new(69), Velocity::new(0.8));

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(256));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        amf.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs[&PortName::OUT];
        let max = (0..256).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max > 0.01, "AM formant should produce sound, max={max}");
    }

    #[test]
    fn test_am_formant_silence_without_note() {
        let mut amf = AmFormant::new();

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        amf.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs[&PortName::OUT];
        let max = (0..64).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max < 0.001, "Should be silent without note_on, max={max}");
    }

    #[test]
    fn test_am_formant_vowel_morphing() {
        let mut amf = AmFormant::new();
        amf.note_on(MidiNote::new(60), Velocity::new(1.0));

        // Process with vowel A
        amf.vowel = NormalizedValue::MIN;
        let mut outputs_a = HashMap::new();
        outputs_a.insert(PortName::OUT, AudioBuffer::new(256));
        let context = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        amf.process(InputPorts::empty(), &mut outputs_a, &context);
        let sum_a: f32 = (0..256).map(|i| outputs_a[&PortName::OUT][i].abs()).sum();

        // Reset and process with vowel U
        amf.reset();
        amf.vowel = NormalizedValue::MAX;
        let mut outputs_u = HashMap::new();
        outputs_u.insert(PortName::OUT, AudioBuffer::new(256));
        amf.process(InputPorts::empty(), &mut outputs_u, &context);
        let sum_u: f32 = (0..256).map(|i| outputs_u[&PortName::OUT][i].abs()).sum();

        // Different vowels should produce different waveforms
        assert!(
            (sum_a - sum_u).abs() > 0.01,
            "Different vowels should produce different output: sum_a={sum_a}, sum_u={sum_u}"
        );
    }

    #[test]
    fn test_am_formant_params() {
        let mut amf = AmFormant::new();
        amf.set_param(Param::AmFormant(AmFormantParam::Depth(
            NormalizedValue::new(0.6),
        )));
        assert!((amf.depth.as_f32() - 0.6).abs() < 0.001);

        let val = amf
            .get_param(&Param::AmFormant(AmFormantParam::Depth(
                NormalizedValue::MIN,
            )))
            .unwrap_or(0.0);
        assert!((val - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_am_formant_output_bounded() {
        let mut amf = AmFormant::new();
        amf.note_on(MidiNote::new(60), Velocity::new(1.0));
        amf.level = NormalizedValue::MAX;
        amf.depth = NormalizedValue::MAX;

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(512));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(512),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        amf.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs[&PortName::OUT];
        let max = (0..512).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max <= 1.5, "Output should be reasonably bounded, max={max}");
    }
}
