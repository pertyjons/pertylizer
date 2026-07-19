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

use synth_core::VoicePitch;
use synth_core::{AmFormantParam, ModuleType, Param};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity,
};

use crate::formant_tables::{FORMANT_FREQ, FORMANT_GAIN, NUM_VOWELS};

/// Number of formant bands (carrier oscillators) — F1/F2/F3. AM synthesis uses
/// no bandwidths, and the shared tables' singer's-formant band is voice-synth
/// only, so this stays at the three speech formants.
const NUM_BANDS: usize = 3;

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
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    /// Cached interned port name for the Vowel CV input (interning locks an
    /// internal table, so it must not happen on the audio thread — see
    /// [`PortName::intern`]). Pitch CV uses the `PITCH_CV` compile-time constant.
    vowel_cv_port: PortName,

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
            mod_offsets: ParamModOffsets::new(),
            vowel_cv_port: PortName::intern("vowel_cv"),

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
            .port(
                PortDescriptor::control_input("pitch_cv", "Pitch CV").description(
                    "1V/oct pitch offset (octaves) on the AM modulator. Connect: LFO, Pitch",
                ),
            )
            .port(
                PortDescriptor::control_input("vowel_cv", "Vowel CV").description(
                    "Modulate vowel position (added to the Vowel knob). Connect: LFO, Envelope",
                ),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("AM formant output"))
    }
}

impl PolyModule for AmFormant {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
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

        // CV inputs. Vowel CV is control-rate: the formant tables are
        // interpolated once per block, so the CV is sampled at block start
        // (sample 0) and folded into the vowel position. Pitch CV (below) is
        // per-sample. Unconnected readers return 0.0 → no change.
        let vowel_cv = inputs.reader(self.vowel_cv_port, 0.0);
        let pitch_cv = inputs.reader(PortName::PITCH_CV, 0.0);
        let vowel_cv0 = if num_samples > 0 {
            vowel_cv.get(0)
        } else {
            0.0
        };

        // `vowel` is read inside interpolated_formants; apply its generic mod
        // offset and the Vowel CV just for that call, then restore.
        let saved_vowel = self.vowel;
        self.vowel = NormalizedValue::new(
            (self.mod_offsets.effective("vowel", self.vowel.as_f32()) + vowel_cv0).clamp(0.0, 1.0),
        );
        let (formant_freqs, formant_gains) = self.interpolated_formants();
        self.vowel = saved_vowel;

        // CarrierRatio maps 0..1 to 0.25..4.0 (exponential scaling)
        let ratio = 0.25
            * (16.0_f32).powf(
                self.mod_offsets
                    .effective("carrier_ratio", self.carrier_ratio.as_f32()),
            );
        let depth = self.mod_offsets.effective("depth", self.depth.as_f32());
        let level = self.mod_offsets.effective("level", self.level.as_f32());
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

            // Advance modulator phase. Pitch CV bends the AM modulator 1V/oct
            // per sample (the formant carriers are unaffected — they track the
            // vowel, not the note pitch).
            let inc = if pitch_cv.is_connected() {
                Hertz::new(mod_freq)
                    .apply_cv(BipolarValue::new(pitch_cv.get(i)))
                    .as_f32()
                    * inv_sr
            } else {
                mod_inc
            };
            let new_mod = self.modulator_phase.as_f32() + inc;
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

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.carrier_phases = [Phase::ZERO; NUM_BANDS];
        self.modulator_phase = Phase::ZERO;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.reset();
    }

    fn set_voice_pitch(&mut self, pitch: VoicePitch) {
        // `process` reads `note_freq` live as the AM modulator frequency, so
        // tracking the modulated note pitch (glide / vibrato / bend) is just
        // updating `note_freq`. Phase accumulates continuously — no click.
        self.note_freq = Hertz::new(Hertz::OSC_RANGE.clamp(pitch.played.as_f32()));
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
    use crate::voice_pitch_harness::{amdf_fundamental, render_mono};

    /// `set_voice_pitch` retunes the AM modulator (`note_freq`, read live by
    /// `process`): 2× voice pitch doubles the modulation rate; a static note
    /// holds. The pitch here is the AM rate over inharmonic formant carriers, so
    /// we measure the *envelope* (rectified + smoothed) — AMDF on the raw output
    /// would lock onto the carriers. Full depth makes the envelope periodic at
    /// the note rate.
    #[test]
    fn am_formant_tracks_voice_pitch() {
        let sr = SampleRate::DVD_QUALITY;
        let srf = sr.as_f32();
        let note = MidiNote::new(45); // A2 ≈ 110 Hz
        let f = note.to_frequency().as_f32();
        let cents = |est: f32, target: f32| 1200.0 * (est / target).log2();

        // Extract the AM envelope: rectify, then a 1-pole low-pass that smooths
        // the carrier ripple but follows the note-rate modulation.
        let envelope = |sig: &[f32]| -> Vec<f32> {
            let mut env = 0.0f32;
            sig.iter()
                .map(|&x| {
                    env += 0.02 * (x.abs() - env);
                    env
                })
                .collect()
        };

        let mut s = AmFormant::new();
        s.depth = NormalizedValue::new(1.0); // full AM → envelope dips to 0 each cycle
        s.note_on(note, Velocity::MAX);
        let stat = render_mono(&mut s, sr, 6, 1024, |_| {});
        let est_stat = amdf_fundamental(&envelope(&stat)[3072..], srf, f);
        assert!(cents(est_stat, f).abs() < 50.0, "static {est_stat}");

        let mut s2 = AmFormant::new();
        s2.depth = NormalizedValue::new(1.0);
        s2.note_on(note, Velocity::MAX);
        let up = render_mono(&mut s2, sr, 6, 1024, |m| {
            m.set_voice_pitch(VoicePitch::tracking(Hertz::new(f * 2.0)));
        });
        let est_up = amdf_fundamental(&envelope(&up)[3072..], srf, f * 2.0);
        assert!(cents(est_up, f * 2.0).abs() < 50.0, "2x {est_up}");
    }

    #[test]
    fn test_am_formant_creation() {
        let amf = AmFormant::new();
        assert!((amf.note_freq.as_f32() - 0.0).abs() < f32::EPSILON);
        assert_eq!(amf.carrier_phases, [Phase::ZERO; NUM_BANDS]);
    }

    /// `level` is a working mod destination via the generic store, and the
    /// transiently-applied `vowel` field is restored after the block.
    #[test]
    fn level_mod_offset_scales_output_and_vowel_restores() {
        let mut amf = AmFormant::new();
        let desc = amf.descriptor();
        amf.mod_offsets_mut().unwrap().populate(&desc);
        amf.note_on(MidiNote::A4, Velocity::MAX);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            ..ProcessContext::default()
        };
        fn peak(amf: &mut AmFormant, ctx: &ProcessContext) -> f32 {
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            amf.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut amf, &ctx);
        assert!(base > 1e-3, "base output present, got {base}");

        let vowel_before = amf.vowel.as_f32();
        amf.set_mod_offset("level", -0.8);
        amf.set_mod_offset("vowel", 0.5); // exercise the transient field
        let quieter = peak(&mut amf, &ctx);
        assert!(
            (amf.vowel.as_f32() - vowel_before).abs() < 1e-6,
            "vowel field must be restored after process"
        );
        assert!(
            quieter < base * 0.6,
            "level offset should reduce output: {quieter} vs {base}"
        );

        amf.clear_mod_offsets();
        let reverted = peak(&mut amf, &ctx);
        assert!(reverted > quieter, "clearing restores level");
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
