//! Voice Synth — physically-inspired singing-voice module.
//!
//! Source–filter model: a glottal pulse (Rosenberg/LF family) excites a bank of
//! parallel formant resonators tuned to a morphable vowel (A→E→I→O→U). A
//! `FormantShift` parameter scales all formant centers together to model
//! vocal-tract length (the basis for SATB voice types in a later phase).
//!
//! Expressivity (Phase 2): breath/aspiration noise, glottal open quotient,
//! spectral tilt, internal vibrato, and `pitch_cv` / `vowel_cv` / `breath_cv`
//! modulation inputs.
//!
//! See `plans/voice-synth-plan.md` for the full design and phased roadmap.
//! The formant tables and bandpass below are copied from `formant_filter` /
//! `am_formant` (same per-module-private pattern those modules use); a shared
//! `formant_tables` extraction is deferred — see the plan's open questions.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{Cents, Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity};
use synth_core::{ModuleType, Param, VoiceSynthParam};

/// Number of formant bands (parallel bandpass resonators).
const NUM_BANDS: usize = 3;
/// Number of vowels (A, E, I, O, U).
const NUM_VOWELS: usize = 5;

/// Relative position of the glottal flow peak within the open phase.
const GLOTTAL_TP: f32 = 0.6;
/// Makeup gain applied to breath noise before it joins the glottal excitation.
const NOISE_GAIN: f32 = 2.0;
/// Non-zero PRNG seed (golden-ratio constant) for the breath-noise generator.
const RNG_SEED: u32 = 0x9E37_79B9;

/// Formant frequencies for each vowel [vowel][band] in Hz.
const FORMANT_FREQ: [[f32; NUM_BANDS]; NUM_VOWELS] = [
    [800.0, 1150.0, 2900.0], // A (as in "father")
    [350.0, 2000.0, 2800.0], // E (as in "bed")
    [270.0, 2140.0, 3200.0], // I (as in "heed")
    [450.0, 800.0, 2830.0],  // O (as in "hot")
    [325.0, 700.0, 2530.0],  // U (as in "boot")
];

/// Formant bandwidths for each vowel [vowel][band] in Hz.
const FORMANT_BW: [[f32; NUM_BANDS]; NUM_VOWELS] = [
    [80.0, 90.0, 120.0],  // A
    [60.0, 100.0, 120.0], // E
    [60.0, 90.0, 100.0],  // I
    [70.0, 80.0, 100.0],  // O
    [50.0, 60.0, 170.0],  // U
];

/// Formant gains (linear) for each vowel [vowel][band].
const FORMANT_GAIN: [[f32; NUM_BANDS]; NUM_VOWELS] = [
    [1.0, 0.5, 0.25],  // A
    [1.0, 0.5, 0.2],   // E
    [1.0, 0.35, 0.15], // I
    [1.0, 0.35, 0.2],  // O
    [1.0, 0.3, 0.15],  // U
];

/// 2nd-order bandpass filter state (per band).
#[derive(Clone, Copy, Default)]
struct BandpassState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BandpassState {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Bandpass filter coefficients (constant-skirt, RBJ-style).
#[derive(Clone, Copy, Default)]
struct BandpassCoeffs {
    b0: f32,
    a1: f32,
    a2: f32,
}

impl BandpassCoeffs {
    fn new(center: f32, bandwidth: f32, sample_rate: f32) -> Self {
        let omega = 2.0 * std::f32::consts::PI * center / sample_rate;
        let sin_w = omega.sin();
        let cos_w = omega.cos();
        let alpha = sin_w * (bandwidth * std::f32::consts::PI / sample_rate).tanh();

        let a0_inv = 1.0 / (1.0 + alpha);
        Self {
            b0: alpha * a0_inv,
            a1: -2.0 * cos_w * a0_inv,
            a2: (1.0 - alpha) * a0_inv,
        }
    }

    #[inline]
    fn process(&self, input: f32, state: &mut BandpassState) -> f32 {
        let output = self.b0 * (input - state.x2) - self.a1 * state.y1 - self.a2 * state.y2;
        state.x2 = state.x1;
        state.x1 = input;
        state.y2 = state.y1;
        state.y1 = output;
        output
    }
}

/// Glottal flow over one normalized period `phase` ∈ [0, 1), open for `oq`.
///
/// Rosenberg-style two-piece pulse: rises to a peak at `GLOTTAL_TP` of the open
/// phase, falls back to zero at the glottal-closure instant, then stays closed.
#[inline]
fn glottal_flow(phase: f32, oq: f32) -> f32 {
    if phase >= oq {
        return 0.0;
    }
    let x = phase / oq;
    if x < GLOTTAL_TP {
        0.5 * (1.0 - (std::f32::consts::PI * x / GLOTTAL_TP).cos())
    } else {
        (0.5 * std::f32::consts::PI * (x - GLOTTAL_TP) / (1.0 - GLOTTAL_TP)).cos()
    }
}

/// Voice Synth module (glottal source → formant bank, with expressivity).
#[derive(Clone)]
pub struct VoiceSynth {
    // Parameters
    vowel: NormalizedValue,
    formant_shift: NormalizedValue,
    breathiness: NormalizedValue,
    open_quotient: NormalizedValue,
    tilt: NormalizedValue,
    vibrato_rate: Hertz,
    vibrato_depth: Cents,
    level: NormalizedValue,

    // Glottal source state
    glottal_phase: Phase,
    prev_flow: f32,
    vibrato_phase: Phase,
    rng_state: u32,
    tilt_state: f32,

    // Formant bank state
    /// Effective (CV-modulated) vowel position in 0..1, drives `update_coeffs`.
    current_vowel: f32,
    states: [BandpassState; NUM_BANDS],
    coeffs: [BandpassCoeffs; NUM_BANDS],
    gains: [f32; NUM_BANDS],

    // Note tracking
    note_freq: Hertz,

    // Cached
    sample_rate: SampleRate,
    inv_sample_rate: f32,

    // Pre-allocated output buffer
    output_buffer: AudioBuffer,
}

impl VoiceSynth {
    pub fn new() -> Self {
        let mut v = Self {
            vowel: NormalizedValue::MIN,
            formant_shift: NormalizedValue::new(0.5),
            breathiness: NormalizedValue::MIN,
            open_quotient: NormalizedValue::new(0.5),
            tilt: NormalizedValue::MIN,
            vibrato_rate: Hertz::new(5.5),
            vibrato_depth: Cents::ZERO,
            level: NormalizedValue::new(0.8),
            glottal_phase: Phase::ZERO,
            prev_flow: 0.0,
            vibrato_phase: Phase::ZERO,
            rng_state: RNG_SEED,
            tilt_state: 0.0,
            current_vowel: 0.0,
            states: [BandpassState::default(); NUM_BANDS],
            coeffs: [BandpassCoeffs::default(); NUM_BANDS],
            gains: [1.0, 0.5, 0.25],
            note_freq: Hertz::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),
            output_buffer: AudioBuffer::new(1024),
        };
        v.update_coeffs();
        v
    }

    /// Map the normalized formant-shift parameter to a frequency scale factor.
    /// 0.0 → 0.5 (octave down), 0.5 → 1.0 (no shift), 1.0 → 2.0 (octave up).
    #[inline]
    fn shift_factor(&self) -> f32 {
        (2.0_f32).powf(2.0 * self.formant_shift.as_f32() - 1.0)
    }

    /// Glottal open quotient in 0.3..0.9 (0.5 normalized → 0.6).
    #[inline]
    fn open_quotient(&self) -> f32 {
        0.3 + self.open_quotient.as_f32() * 0.6
    }

    /// One white-noise sample in [-1, 1] via xorshift32 (allocation-free).
    #[inline]
    fn next_noise(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Recompute formant coefficients from `current_vowel`, shift and rate.
    /// Cheap (3 biquads) — called at block start and when `vowel_cv` moves.
    fn update_coeffs(&mut self) {
        let pos = self.current_vowel * (NUM_VOWELS - 1) as f32;
        let idx = (pos as usize).min(NUM_VOWELS - 2);
        let frac = pos - idx as f32;

        let shift = self.shift_factor();
        let sr = self.sample_rate.as_f32();

        for band in 0..NUM_BANDS {
            let freq = FORMANT_FREQ[idx][band] * (1.0 - frac) + FORMANT_FREQ[idx + 1][band] * frac;
            let bw = FORMANT_BW[idx][band] * (1.0 - frac) + FORMANT_BW[idx + 1][band] * frac;
            let gain = FORMANT_GAIN[idx][band] * (1.0 - frac) + FORMANT_GAIN[idx + 1][band] * frac;

            let scaled_freq = (freq * shift).clamp(20.0, sr * 0.45);
            self.coeffs[band] = BandpassCoeffs::new(scaled_freq, bw.max(10.0), sr);
            self.gains[band] = gain;
        }
    }
}

impl Default for VoiceSynth {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for VoiceSynth {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("voice_synth", "Voice Synth")
            .description("Physically-inspired singing voice (glottal source → formants)")
            .category(ModuleCategory::Oscillator)
            .tag("voice")
            .tag("vocal")
            .tag("formant")
            .tag("synthesis")
            .parameter(
                ParameterDescriptor::float(
                    "vowel",
                    Param::VoiceSynth(VoiceSynthParam::Vowel(NormalizedValue::MIN)),
                    "Vowel",
                )
                .description("Vowel morph (A → E → I → O → U)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "formant_shift",
                    Param::VoiceSynth(VoiceSynthParam::FormantShift(NormalizedValue::new(0.5))),
                    "Formant Shift",
                )
                .description("Vocal-tract length: <0.5 longer/darker, >0.5 shorter/brighter")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "breathiness",
                    Param::VoiceSynth(VoiceSynthParam::Breathiness(NormalizedValue::MIN)),
                    "Breathiness",
                )
                .description("Aspiration / breath noise amount")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "open_quotient",
                    Param::VoiceSynth(VoiceSynthParam::OpenQuotient(NormalizedValue::new(0.5))),
                    "Open Quotient",
                )
                .description("Glottal open quotient: low = pressed, high = soft/breathy")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "tilt",
                    Param::VoiceSynth(VoiceSynthParam::Tilt(NormalizedValue::MIN)),
                    "Tilt",
                )
                .description("Spectral tilt: 0 bright, 1 dark (soft glottal closure)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "vibrato_rate",
                    Param::VoiceSynth(VoiceSynthParam::VibratoRate(Hertz::new(5.5))),
                    "Vibrato Rate",
                )
                .description("Vibrato LFO rate")
                .range(0.0, 12.0)
                .default(5.5)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "vibrato_depth",
                    Param::VoiceSynth(VoiceSynthParam::VibratoDepth(Cents::ZERO)),
                    "Vibrato Depth",
                )
                .description("Vibrato depth in cents")
                .range(0.0, 100.0)
                .default(0.0)
                .unit(ParameterUnit::Cents)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::VoiceSynth(VoiceSynthParam::Level(NormalizedValue::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("pitch_cv", "Pitch CV")
                    .description("Pitch offset in semitones. Connect: LFO, Envelope, Pitch bend"),
            )
            .port(
                PortDescriptor::control_input("vowel_cv", "Vowel CV")
                    .description("Modulate vowel position. Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::control_input("breath_cv", "Breath CV")
                    .description("Modulate breathiness. Connect: LFO, Envelope"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Voice output"))
    }
}

impl PolyModule for VoiceSynth {
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

        // No note playing — output silence.
        if self.note_freq.as_f32() <= 0.0 {
            for i in 0..num_samples {
                self.output_buffer[i] = 0.0;
            }
            if let Some(out) = outputs.get_mut(&PortName::OUT) {
                out.copy_from(&self.output_buffer);
            }
            return;
        }

        let pitch_cv = inputs.reader(PortName::intern("pitch_cv"), 0.0);
        let vowel_cv = inputs.reader(PortName::intern("vowel_cv"), 0.0);
        let breath_cv = inputs.reader(PortName::intern("breath_cv"), 0.0);
        let vowel_cv_connected = vowel_cv.is_connected();

        // Refresh formant coefficients for this block (vowel/shift/rate may change).
        if !vowel_cv_connected {
            self.current_vowel = self.vowel.as_f32();
        }
        self.update_coeffs();

        let inv_sr = self.inv_sample_rate;
        let level = self.level.as_f32();
        let oq = self.open_quotient();
        let base_freq = self.note_freq.as_f32();
        let breath_base = self.breathiness.as_f32();
        let vib_depth_cents = self.vibrato_depth.as_f32();
        let vib_inc = self.vibrato_rate.as_f32() * inv_sr;

        let gain_sum: f32 = self.gains.iter().sum();
        let norm = if gain_sum > 1e-7 { 1.0 / gain_sum } else { 1.0 };

        // Spectral tilt: one-pole lowpass, cutoff darkens as tilt rises.
        // tilt == 0 is a true bypass (bright, sample-rate independent); above
        // that the cutoff sweeps down from ~10 kHz to ~1.2 kHz.
        let tilt_amt = self.tilt.as_f32();
        let tilt_active = tilt_amt > 0.0;
        let tilt_fc = 10000.0 * (1.0 - tilt_amt) + 1200.0 * tilt_amt;
        let tilt_coef =
            (1.0 - (-2.0 * std::f32::consts::PI * tilt_fc * inv_sr).exp()).clamp(0.0, 1.0);

        for i in 0..num_samples {
            // Vowel CV modulation (gated coefficient recompute, like FormantFilter).
            if vowel_cv_connected {
                let target = (self.vowel.as_f32() + vowel_cv[i] * 0.5).clamp(0.0, 1.0);
                if (target - self.current_vowel).abs() > 0.001 {
                    self.current_vowel = target;
                    self.update_coeffs();
                }
            }

            // Pitch: base ± vibrato (cents) ± pitch_cv (semitones).
            let vib_cents =
                vib_depth_cents * crate::math::fast_sin_turns(self.vibrato_phase.as_f32());
            let semis = pitch_cv[i] + vib_cents / 100.0;
            let freq = base_freq * (2.0_f32).powf(semis / 12.0);
            let inc = (freq * inv_sr).max(1e-5);

            // Glottal flow derivative w.r.t. phase — pitch-independent amplitude,
            // and the closure corner supplies the harmonic-rich excitation.
            let flow = glottal_flow(self.glottal_phase.as_f32(), oq);
            let mut excitation = (flow - self.prev_flow) / inc;
            self.prev_flow = flow;

            // Breath / aspiration noise, shaped by the formant bank alongside the
            // glottal source.
            let breath = (breath_base + breath_cv[i]).clamp(0.0, 1.0);
            if breath > 0.0 {
                excitation += self.next_noise() * breath * NOISE_GAIN;
            }

            // Spectral tilt (one-pole lowpass on the source); bypassed at tilt 0.
            let source = if tilt_active {
                self.tilt_state += tilt_coef * (excitation - self.tilt_state);
                self.tilt_state
            } else {
                excitation
            };

            // Sum parallel formant resonators.
            let mut voiced = 0.0_f32;
            for band in 0..NUM_BANDS {
                voiced +=
                    self.coeffs[band].process(source, &mut self.states[band]) * self.gains[band];
            }
            voiced *= norm;

            self.output_buffer[i] = crate::math::soft_clip(voiced * level);

            // Advance phases, wrapping at their period boundaries.
            self.glottal_phase = Phase::new_unchecked((self.glottal_phase.as_f32() + inc).fract());
            self.vibrato_phase =
                Phase::new_unchecked((self.vibrato_phase.as_f32() + vib_inc).fract());
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::VoiceSynth(p) = param {
            match p {
                VoiceSynthParam::Vowel(v) => {
                    self.vowel = v;
                    self.current_vowel = v.as_f32();
                }
                VoiceSynthParam::FormantShift(v) => self.formant_shift = v,
                VoiceSynthParam::Breathiness(v) => self.breathiness = v,
                VoiceSynthParam::OpenQuotient(v) => self.open_quotient = v,
                VoiceSynthParam::Tilt(v) => self.tilt = v,
                VoiceSynthParam::VibratoRate(hz) => self.vibrato_rate = hz,
                VoiceSynthParam::VibratoDepth(c) => self.vibrato_depth = c,
                VoiceSynthParam::Level(v) => self.level = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::VoiceSynth(p) = param {
            Some(match p {
                VoiceSynthParam::Vowel(_) => self.vowel.as_f32(),
                VoiceSynthParam::FormantShift(_) => self.formant_shift.as_f32(),
                VoiceSynthParam::Breathiness(_) => self.breathiness.as_f32(),
                VoiceSynthParam::OpenQuotient(_) => self.open_quotient.as_f32(),
                VoiceSynthParam::Tilt(_) => self.tilt.as_f32(),
                VoiceSynthParam::VibratoRate(_) => self.vibrato_rate.as_f32(),
                VoiceSynthParam::VibratoDepth(_) => self.vibrato_depth.as_f32(),
                VoiceSynthParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::VoiceSynth(VoiceSynthParam::Vowel(self.vowel)),
            Param::VoiceSynth(VoiceSynthParam::FormantShift(self.formant_shift)),
            Param::VoiceSynth(VoiceSynthParam::Breathiness(self.breathiness)),
            Param::VoiceSynth(VoiceSynthParam::OpenQuotient(self.open_quotient)),
            Param::VoiceSynth(VoiceSynthParam::Tilt(self.tilt)),
            Param::VoiceSynth(VoiceSynthParam::VibratoRate(self.vibrato_rate)),
            Param::VoiceSynth(VoiceSynthParam::VibratoDepth(self.vibrato_depth)),
            Param::VoiceSynth(VoiceSynthParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::VoiceSynth
    }

    fn reset(&mut self) {
        self.glottal_phase = Phase::ZERO;
        self.prev_flow = 0.0;
        self.vibrato_phase = Phase::ZERO;
        self.rng_state = RNG_SEED;
        self.tilt_state = 0.0;
        self.current_vowel = self.vowel.as_f32();
        for state in &mut self.states {
            state.reset();
        }
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.reset();
    }

    fn note_off(&mut self) {
        // Amplitude envelope is handled by the host patch (Envelope → Amplifier).
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.inv_sample_rate = 1.0 / sample_rate.as_f32();
        self.update_coeffs();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(n: usize) -> ProcessContext<'static> {
        ProcessContext {
            samples: synth_core::SampleCount::new(n),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        }
    }

    #[test]
    fn test_voice_synth_creation() {
        let v = VoiceSynth::new();
        assert!((v.note_freq.as_f32() - 0.0).abs() < f32::EPSILON);
        assert_eq!(v.glottal_phase, Phase::ZERO);
    }

    #[test]
    fn test_voice_synth_produces_sound() {
        let mut v = VoiceSynth::new();
        v.note_on(MidiNote::new(57), Velocity::new(0.8));

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(512));
        v.process(InputPorts::empty(), &mut outputs, &ctx(512));

        let out = &outputs[&PortName::OUT];
        let max = (0..512).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max > 0.01, "Voice synth should produce sound, max={max}");
    }

    #[test]
    fn test_voice_synth_silence_without_note() {
        let mut v = VoiceSynth::new();
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));
        v.process(InputPorts::empty(), &mut outputs, &ctx(64));

        let out = &outputs[&PortName::OUT];
        let max = (0..64).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max < 0.001, "Should be silent without note_on, max={max}");
    }

    #[test]
    fn test_voice_synth_vowel_morphing() {
        let mut v = VoiceSynth::new();
        v.note_on(MidiNote::new(48), Velocity::new(1.0));

        v.set_param(Param::VoiceSynth(VoiceSynthParam::Vowel(
            NormalizedValue::MIN,
        )));
        let mut out_a = HashMap::new();
        out_a.insert(PortName::OUT, AudioBuffer::new(1024));
        v.process(InputPorts::empty(), &mut out_a, &ctx(1024));
        let sum_a: f32 = (0..1024).map(|i| out_a[&PortName::OUT][i].abs()).sum();

        v.note_on(MidiNote::new(48), Velocity::new(1.0));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::Vowel(
            NormalizedValue::MAX,
        )));
        let mut out_u = HashMap::new();
        out_u.insert(PortName::OUT, AudioBuffer::new(1024));
        v.process(InputPorts::empty(), &mut out_u, &ctx(1024));
        let sum_u: f32 = (0..1024).map(|i| out_u[&PortName::OUT][i].abs()).sum();

        assert!(
            (sum_a - sum_u).abs() > 0.01,
            "Different vowels should differ: sum_a={sum_a}, sum_u={sum_u}"
        );
    }

    #[test]
    fn test_voice_synth_output_bounded() {
        let mut v = VoiceSynth::new();
        v.note_on(MidiNote::new(60), Velocity::new(1.0));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::Level(
            NormalizedValue::MAX,
        )));
        // Drive every expressivity path hard.
        v.set_param(Param::VoiceSynth(VoiceSynthParam::Breathiness(
            NormalizedValue::MAX,
        )));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::VibratoDepth(
            Cents::new(100.0),
        )));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::Tilt(
            NormalizedValue::MAX,
        )));

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(512));
        v.process(InputPorts::empty(), &mut outputs, &ctx(512));

        let out = &outputs[&PortName::OUT];
        let max = (0..512).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(
            max.is_finite() && max <= 1.5,
            "Output should be bounded, max={max}"
        );
    }

    #[test]
    fn test_voice_synth_breathiness_changes_output() {
        let render = |breath: f32| {
            let mut v = VoiceSynth::new();
            v.note_on(MidiNote::new(50), Velocity::new(1.0));
            v.set_param(Param::VoiceSynth(VoiceSynthParam::Breathiness(
                NormalizedValue::new(breath),
            )));
            let mut outputs = HashMap::new();
            outputs.insert(PortName::OUT, AudioBuffer::new(1024));
            v.process(InputPorts::empty(), &mut outputs, &ctx(1024));
            (0..1024)
                .map(|i| outputs[&PortName::OUT][i].abs())
                .sum::<f32>()
        };
        let dry = render(0.0);
        let breathy = render(1.0);
        assert!(
            (dry - breathy).abs() > 0.01,
            "Breathiness should change output: dry={dry}, breathy={breathy}"
        );
    }

    #[test]
    fn test_voice_synth_params() {
        let mut v = VoiceSynth::new();
        v.set_param(Param::VoiceSynth(VoiceSynthParam::FormantShift(
            NormalizedValue::new(0.7),
        )));
        let got = v
            .get_param(&Param::VoiceSynth(VoiceSynthParam::FormantShift(
                NormalizedValue::MIN,
            )))
            .unwrap_or(0.0);
        assert!((got - 0.7).abs() < 0.001);

        v.set_param(Param::VoiceSynth(VoiceSynthParam::VibratoRate(Hertz::new(
            6.0,
        ))));
        let rate = v
            .get_param(&Param::VoiceSynth(VoiceSynthParam::VibratoRate(
                Hertz::ZERO,
            )))
            .unwrap_or(0.0);
        assert!((rate - 6.0).abs() < 0.001);
    }
}
