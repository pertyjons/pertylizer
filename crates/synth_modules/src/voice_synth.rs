//! Voice Synth — physically-inspired singing-voice / choir module.
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
//! Choir (Phase 3): an internal bank of up to `MAX_UNISON` decorrelated
//! sub-voices — each with its own detune, vibrato phase/rate, formant jitter,
//! onset stagger and stereo pan. The pseudo-random amplitude beating between
//! sub-voices is what the ear hears as an ensemble. Mono `out` is the
//! un-panned sum; `out_l`/`out_r` carry the stereo spread.
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

use crate::formant_tables::NUM_BANDS;
use crate::voice_common::{self, MAX_UNISON, ONSET_MAX_SECS, RNG_SEED, VoiceSpread};

/// Relative position of the glottal flow peak within the open phase.
const GLOTTAL_TP: f32 = 0.6;
/// Makeup gain applied to breath noise before it joins the glottal excitation.
const NOISE_GAIN: f32 = 2.0;

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

/// One decorrelated unison sub-voice: an independent glottal source + formant
/// bank with its own detune, vibrato, formant jitter, onset and pan.
#[derive(Clone)]
struct SubVoice {
    glottal_phase: Phase,
    prev_flow: f32,
    vibrato_phase: Phase,
    tilt_state: f32,
    rng_state: u32,
    states: [BandpassState; NUM_BANDS],
    coeffs: [BandpassCoeffs; NUM_BANDS],
    gains: [f32; NUM_BANDS],

    // Per-voice decorrelation (recomputed at block start).
    detune_cents: f32,
    vib_rate_mult: f32,
    formant_jitter: f32,
    pan_l: f32,
    pan_r: f32,
    onset_countdown: u32,
}

impl SubVoice {
    fn new() -> Self {
        Self {
            glottal_phase: Phase::ZERO,
            prev_flow: 0.0,
            vibrato_phase: Phase::ZERO,
            tilt_state: 0.0,
            rng_state: RNG_SEED,
            states: [BandpassState::default(); NUM_BANDS],
            coeffs: [BandpassCoeffs::default(); NUM_BANDS],
            gains: [1.0, 0.5, 0.25, 0.15],
            detune_cents: 0.0,
            vib_rate_mult: 1.0,
            formant_jitter: 1.0,
            pan_l: std::f32::consts::FRAC_1_SQRT_2,
            pan_r: std::f32::consts::FRAC_1_SQRT_2,
            onset_countdown: 0,
        }
    }

    /// Re-arm phases/state for a new note. `glottal_phase`/`vib_phase`
    /// decorrelate this voice from the others.
    fn restart(&mut self, seed: u32, glottal_phase: f32, vib_phase: f32, onset: u32) {
        self.glottal_phase = Phase::new_unchecked(glottal_phase);
        self.prev_flow = 0.0;
        self.vibrato_phase = Phase::new_unchecked(vib_phase);
        self.tilt_state = 0.0;
        self.rng_state = seed;
        self.onset_countdown = onset;
        for s in &mut self.states {
            s.reset();
        }
    }

    /// One white-noise sample in [-1, 1] via xorshift32.
    #[inline]
    fn next_noise(&mut self) -> f32 {
        crate::math::xorshift_noise(&mut self.rng_state)
    }

    /// Recompute formant coefficients for the current vowel position, applying
    /// this voice's formant jitter and the shared formant shift.
    fn update_coeffs(&mut self, vowel_pos: f32, shift: f32, sample_rate: f32) {
        let (freqs, bws, gains) = crate::formant_tables::interpolate_vowel(vowel_pos);
        let scale = shift * self.formant_jitter;

        for band in 0..NUM_BANDS {
            let scaled_freq = (freqs[band] * scale).clamp(20.0, sample_rate * 0.45);
            self.coeffs[band] = BandpassCoeffs::new(scaled_freq, bws[band].max(10.0), sample_rate);
            self.gains[band] = gains[band];
        }
    }
}

/// Voice Synth module (glottal source → formant bank, with expressivity + choir).
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
    unison_voices: NormalizedValue,
    unison_detune: Cents,
    unison_spread: NormalizedValue,
    level: NormalizedValue,

    // Shared state
    /// Effective (CV-modulated) vowel position in 0..1, drives `update_coeffs`.
    current_vowel: f32,
    voices: [SubVoice; MAX_UNISON],
    active_voices: usize,
    note_freq: Hertz,

    // Cached
    sample_rate: SampleRate,
    inv_sample_rate: f32,

    // Pre-allocated output buffers
    out_buffer: AudioBuffer,
    out_l_buffer: AudioBuffer,
    out_r_buffer: AudioBuffer,
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
            unison_voices: NormalizedValue::MIN,
            unison_detune: Cents::new(15.0),
            unison_spread: NormalizedValue::new(0.7),
            level: NormalizedValue::new(0.8),
            current_vowel: 0.0,
            voices: std::array::from_fn(|_| SubVoice::new()),
            active_voices: 1,
            note_freq: Hertz::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),
            out_buffer: AudioBuffer::new(1024),
            out_l_buffer: AudioBuffer::new(1024),
            out_r_buffer: AudioBuffer::new(1024),
        };
        v.derive_decorrelation();
        v.voices[0].update_coeffs(v.current_vowel, v.shift_factor(), v.sample_rate.as_f32());
        v
    }

    /// Map the normalized formant-shift parameter to a frequency scale factor.
    /// 0.0 → 0.5 (octave down), 0.5 → 1.0 (no shift), 1.0 → 2.0 (octave up).
    #[inline]
    fn shift_factor(&self) -> f32 {
        crate::formant_tables::formant_shift_factor(self.formant_shift.as_f32())
    }

    /// Glottal open quotient in 0.3..0.9 (0.5 normalized → 0.6).
    #[inline]
    fn open_quotient(&self) -> f32 {
        0.3 + self.open_quotient.as_f32() * 0.6
    }

    /// Number of active unison sub-voices (1..=MAX_UNISON).
    #[inline]
    fn unison_count(&self) -> usize {
        voice_common::unison_count(self.unison_voices.as_f32(), MAX_UNISON)
    }

    /// Recompute per-voice decorrelation (detune, vibrato rate, formant jitter,
    /// pan) for the active sub-voices. Deterministic from voice index + note,
    /// so it is idempotent and cheap to call every block.
    fn derive_decorrelation(&mut self) {
        let active = self.active_voices;
        let detune = self.unison_detune.as_f32();
        let spread = self.unison_spread.as_f32();
        let note = self.note_freq.as_f32();

        for (v, voice) in self.voices[..active].iter_mut().enumerate() {
            let s = if active == 1 {
                VoiceSpread::solo()
            } else {
                VoiceSpread::derive(v, active, note, detune, spread)
            };
            voice.detune_cents = s.detune_cents;
            voice.vib_rate_mult = s.vib_rate_mult;
            voice.formant_jitter = s.formant_jitter;
            voice.pan_l = s.pan_l;
            voice.pan_r = s.pan_r;
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
            .description("Physically-inspired singing voice / choir (glottal source → formants)")
            .category(ModuleCategory::Oscillator)
            .tag("voice")
            .tag("vocal")
            .tag("formant")
            .tag("choir")
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
                    "unison_voices",
                    Param::VoiceSynth(VoiceSynthParam::UnisonVoices(NormalizedValue::MIN)),
                    "Unison Voices",
                )
                .description("Choir size: 1 (solo) to 16 decorrelated voices")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "unison_detune",
                    Param::VoiceSynth(VoiceSynthParam::UnisonDetune(Cents::new(15.0))),
                    "Unison Detune",
                )
                .description("Pitch spread across unison voices")
                .range(0.0, 50.0)
                .default(15.0)
                .unit(ParameterUnit::Cents)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "unison_spread",
                    Param::VoiceSynth(VoiceSynthParam::UnisonSpread(NormalizedValue::new(0.7))),
                    "Unison Spread",
                )
                .description("Stereo width of the choir (0 = mono, 1 = full)")
                .range(0.0, 1.0)
                .default(0.7)
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
            .port(PortDescriptor::audio_output("out", "Out").description("Voice output (mono sum)"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Stereo left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Stereo right output"))
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
        self.out_buffer.resize(num_samples);
        self.out_l_buffer.resize(num_samples);
        self.out_r_buffer.resize(num_samples);

        // No note playing — output silence on all three ports.
        if self.note_freq.as_f32() <= 0.0 {
            for i in 0..num_samples {
                self.out_buffer[i] = 0.0;
                self.out_l_buffer[i] = 0.0;
                self.out_r_buffer[i] = 0.0;
            }
            self.write_outputs(outputs);
            return;
        }

        let pitch_cv = inputs.reader(PortName::intern("pitch_cv"), 0.0);
        let vowel_cv = inputs.reader(PortName::intern("vowel_cv"), 0.0);
        let breath_cv = inputs.reader(PortName::intern("breath_cv"), 0.0);
        let vowel_cv_connected = vowel_cv.is_connected();

        // Recompute active count + decorrelation for this block.
        self.active_voices = self.unison_count();
        self.derive_decorrelation();
        let active = self.active_voices;

        if !vowel_cv_connected {
            self.current_vowel = self.vowel.as_f32();
        }

        let inv_sr = self.inv_sample_rate;
        let sr = self.sample_rate.as_f32();
        let shift = self.shift_factor();
        let level = self.level.as_f32();
        let oq = self.open_quotient();
        let base_freq = self.note_freq.as_f32();
        let breath_base = self.breathiness.as_f32();
        let vib_depth_cents = self.vibrato_depth.as_f32();
        let vib_inc = self.vibrato_rate.as_f32() * inv_sr;
        let unison_norm = 1.0 / (active as f32).sqrt();

        // Spectral tilt: one-pole lowpass; tilt == 0 is a true bypass.
        let tilt_amt = self.tilt.as_f32();
        let tilt_active = tilt_amt > 0.0;
        let tilt_fc = 10000.0 * (1.0 - tilt_amt) + 1200.0 * tilt_amt;
        let tilt_coef = crate::math::one_pole_lp_coef(tilt_fc, inv_sr);

        // Initial coefficients for this block.
        for voice in self.voices[..active].iter_mut() {
            voice.update_coeffs(self.current_vowel, shift, sr);
        }
        let gain_sum: f32 = self.voices[0].gains.iter().sum();
        let norm = if gain_sum > 1e-7 { 1.0 / gain_sum } else { 1.0 };

        for i in 0..num_samples {
            // Vowel CV modulation (shared, gated coefficient recompute).
            if vowel_cv_connected {
                let target = (self.vowel.as_f32() + vowel_cv[i] * 0.5).clamp(0.0, 1.0);
                if (target - self.current_vowel).abs() > 0.001 {
                    self.current_vowel = target;
                    for voice in self.voices[..active].iter_mut() {
                        voice.update_coeffs(target, shift, sr);
                    }
                }
            }

            let pcv = pitch_cv[i];
            let breath = (breath_base + breath_cv[i]).clamp(0.0, 1.0);

            let mut raw = 0.0_f32;
            let mut sum_l = 0.0_f32;
            let mut sum_r = 0.0_f32;

            for voice in self.voices[..active].iter_mut() {
                // Staggered onset: stay silent (phases frozen) until armed.
                if voice.onset_countdown > 0 {
                    voice.onset_countdown -= 1;
                    continue;
                }

                // Pitch: base ± detune/vibrato (cents) ± pitch_cv (semitones).
                let vib_cents =
                    vib_depth_cents * crate::math::fast_sin_turns(voice.vibrato_phase.as_f32());
                let semis = pcv + (voice.detune_cents + vib_cents) / 100.0;
                let freq = base_freq * (2.0_f32).powf(semis / 12.0);
                let inc = (freq * inv_sr).max(1e-5);

                // Glottal flow derivative (pitch-independent amplitude).
                let flow = glottal_flow(voice.glottal_phase.as_f32(), oq);
                let mut excitation = (flow - voice.prev_flow) / inc;
                voice.prev_flow = flow;

                // Breath / aspiration noise.
                if breath > 0.0 {
                    excitation += voice.next_noise() * breath * NOISE_GAIN;
                }

                // Spectral tilt (one-pole lowpass on the source); bypass at 0.
                let source = if tilt_active {
                    voice.tilt_state += tilt_coef * (excitation - voice.tilt_state);
                    voice.tilt_state
                } else {
                    excitation
                };

                // Sum parallel formant resonators.
                let mut voiced = 0.0_f32;
                for band in 0..NUM_BANDS {
                    voiced += voice.coeffs[band].process(source, &mut voice.states[band])
                        * voice.gains[band];
                }
                voiced *= norm;

                raw += voiced;
                sum_l += voiced * voice.pan_l;
                sum_r += voiced * voice.pan_r;

                // Advance phases, wrapping at their period boundaries.
                voice.glottal_phase =
                    Phase::new_unchecked((voice.glottal_phase.as_f32() + inc).fract());
                voice.vibrato_phase = Phase::new_unchecked(
                    (voice.vibrato_phase.as_f32() + vib_inc * voice.vib_rate_mult).fract(),
                );
            }

            self.out_buffer[i] = crate::math::soft_clip(raw * unison_norm * level);
            self.out_l_buffer[i] = crate::math::soft_clip(sum_l * unison_norm * level);
            self.out_r_buffer[i] = crate::math::soft_clip(sum_r * unison_norm * level);
        }

        self.write_outputs(outputs);
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
                VoiceSynthParam::UnisonVoices(v) => self.unison_voices = v,
                VoiceSynthParam::UnisonDetune(c) => self.unison_detune = c,
                VoiceSynthParam::UnisonSpread(v) => self.unison_spread = v,
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
                VoiceSynthParam::UnisonVoices(_) => self.unison_voices.as_f32(),
                VoiceSynthParam::UnisonDetune(_) => self.unison_detune.as_f32(),
                VoiceSynthParam::UnisonSpread(_) => self.unison_spread.as_f32(),
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
            Param::VoiceSynth(VoiceSynthParam::UnisonVoices(self.unison_voices)),
            Param::VoiceSynth(VoiceSynthParam::UnisonDetune(self.unison_detune)),
            Param::VoiceSynth(VoiceSynthParam::UnisonSpread(self.unison_spread)),
            Param::VoiceSynth(VoiceSynthParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::VoiceSynth
    }

    fn reset(&mut self) {
        self.current_vowel = self.vowel.as_f32();
        for voice in &mut self.voices {
            *voice = SubVoice::new();
        }
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.active_voices = self.unison_count();
        self.current_vowel = self.vowel.as_f32();
        let active = self.active_voices;
        let note_hz = self.note_freq.as_f32();
        let onset_max = (ONSET_MAX_SECS * self.sample_rate.as_f32()).max(0.0);

        // Decorrelate the phases of EVERY voice (not just the currently active
        // ones), so raising UnisonVoices mid-note still finds the newly
        // activated voices phase-staggered. Voice 0 stays the clean reference
        // so a solo voice exactly matches the single-voice signal path. Onset
        // stagger only applies to voices active from this note's attack.
        for (v, voice) in self.voices.iter_mut().enumerate() {
            let (glottal_phase, vib_phase) = if v == 0 {
                (0.0, 0.0)
            } else {
                (
                    voice_common::decorr_hash(v, note_hz, 6.0),
                    voice_common::decorr_hash(v, note_hz, 4.0),
                )
            };
            let onset = if v == 0 || v >= active {
                0
            } else {
                voice_common::onset_samples(v, note_hz, onset_max)
            };
            let seed = RNG_SEED ^ (v as u32 + 1).wrapping_mul(0x9E37_79B9);
            voice.restart(seed.max(1), glottal_phase, vib_phase, onset);
        }
        self.derive_decorrelation();
    }

    fn note_off(&mut self) {
        // Amplitude envelope is handled by the host patch (Envelope → Amplifier).
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.inv_sample_rate = 1.0 / sample_rate.as_f32();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

impl VoiceSynth {
    /// Copy the internal buffers to whichever output ports are connected.
    fn write_outputs(&self, outputs: &mut HashMap<PortName, AudioBuffer>) {
        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.out_buffer);
        }
        if let Some(out_l) = outputs.get_mut(&PortName::OUT_L) {
            out_l.copy_from(&self.out_l_buffer);
        }
        if let Some(out_r) = outputs.get_mut(&PortName::OUT_R) {
            out_r.copy_from(&self.out_r_buffer);
        }
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

    fn outputs_stereo(n: usize) -> HashMap<PortName, AudioBuffer> {
        let mut m = HashMap::new();
        m.insert(PortName::OUT, AudioBuffer::new(n));
        m.insert(PortName::OUT_L, AudioBuffer::new(n));
        m.insert(PortName::OUT_R, AudioBuffer::new(n));
        m
    }

    #[test]
    fn test_voice_synth_creation() {
        let v = VoiceSynth::new();
        assert!((v.note_freq.as_f32() - 0.0).abs() < f32::EPSILON);
        assert_eq!(v.active_voices, 1);
    }

    #[test]
    fn test_voice_synth_produces_sound() {
        let mut v = VoiceSynth::new();
        v.note_on(MidiNote::new(57), Velocity::new(0.8));
        let mut outputs = outputs_stereo(512);
        v.process(InputPorts::empty(), &mut outputs, &ctx(512));

        let out = &outputs[&PortName::OUT];
        let max = (0..512).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max > 0.01, "Voice synth should produce sound, max={max}");
    }

    #[test]
    fn test_voice_synth_silence_without_note() {
        let mut v = VoiceSynth::new();
        let mut outputs = outputs_stereo(64);
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
        let mut out_a = outputs_stereo(1024);
        v.process(InputPorts::empty(), &mut out_a, &ctx(1024));
        let sum_a: f32 = (0..1024).map(|i| out_a[&PortName::OUT][i].abs()).sum();

        v.note_on(MidiNote::new(48), Velocity::new(1.0));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::Vowel(
            NormalizedValue::MAX,
        )));
        let mut out_u = outputs_stereo(1024);
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
        v.set_param(Param::VoiceSynth(VoiceSynthParam::Breathiness(
            NormalizedValue::MAX,
        )));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::VibratoDepth(
            Cents::new(100.0),
        )));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::Tilt(
            NormalizedValue::MAX,
        )));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::UnisonVoices(
            NormalizedValue::MAX,
        )));
        v.note_on(MidiNote::new(60), Velocity::new(1.0));

        let mut outputs = outputs_stereo(512);
        v.process(InputPorts::empty(), &mut outputs, &ctx(512));

        for port in [PortName::OUT, PortName::OUT_L, PortName::OUT_R] {
            let out = &outputs[&port];
            let max = (0..512).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
            assert!(max.is_finite() && max <= 1.5, "Output bounded, max={max}");
        }
    }

    #[test]
    fn test_voice_synth_breathiness_changes_output() {
        let render = |breath: f32| {
            let mut v = VoiceSynth::new();
            v.note_on(MidiNote::new(50), Velocity::new(1.0));
            v.set_param(Param::VoiceSynth(VoiceSynthParam::Breathiness(
                NormalizedValue::new(breath),
            )));
            let mut outputs = outputs_stereo(1024);
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

    /// A solo voice pans to center (L == R). A unison choir with spread
    /// decorrelates the channels, so |L - R| energy must grow.
    #[test]
    fn test_voice_synth_unison_widens_stereo() {
        let stereo_diff = |voices: f32, spread: f32| {
            let mut v = VoiceSynth::new();
            v.set_param(Param::VoiceSynth(VoiceSynthParam::UnisonVoices(
                NormalizedValue::new(voices),
            )));
            v.set_param(Param::VoiceSynth(VoiceSynthParam::UnisonSpread(
                NormalizedValue::new(spread),
            )));
            v.set_param(Param::VoiceSynth(VoiceSynthParam::UnisonDetune(
                Cents::new(30.0),
            )));
            v.note_on(MidiNote::new(55), Velocity::new(1.0));
            let mut outputs = outputs_stereo(2048);
            v.process(InputPorts::empty(), &mut outputs, &ctx(2048));
            (0..2048)
                .map(|i| (outputs[&PortName::OUT_L][i] - outputs[&PortName::OUT_R][i]).abs())
                .sum::<f32>()
        };
        let solo = stereo_diff(0.0, 0.7);
        let choir = stereo_diff(1.0, 1.0);
        assert!(
            solo < 1e-3,
            "Solo voice should be centered (L==R), got {solo}"
        );
        assert!(
            choir > solo + 0.1,
            "Unison choir should widen stereo: solo={solo}, choir={choir}"
        );
    }

    /// Raising UnisonVoices mid-note must still yield a decorrelated (wide)
    /// choir, not a phase-aligned coherent block. Regression test for the
    /// mid-note activation bug.
    #[test]
    fn test_voice_synth_unison_increase_mid_note() {
        let mut v = VoiceSynth::new();
        v.set_param(Param::VoiceSynth(VoiceSynthParam::UnisonSpread(
            NormalizedValue::MAX,
        )));
        v.set_param(Param::VoiceSynth(VoiceSynthParam::UnisonDetune(
            Cents::new(30.0),
        )));
        v.note_on(MidiNote::new(55), Velocity::new(1.0));

        // First block: solo (centered).
        let mut solo = outputs_stereo(512);
        v.process(InputPorts::empty(), &mut solo, &ctx(512));

        // Raise to full choir mid-note (no new note_on).
        v.set_param(Param::VoiceSynth(VoiceSynthParam::UnisonVoices(
            NormalizedValue::MAX,
        )));
        let mut choir = outputs_stereo(512);
        v.process(InputPorts::empty(), &mut choir, &ctx(512));

        let diff: f32 = (0..512)
            .map(|i| (choir[&PortName::OUT_L][i] - choir[&PortName::OUT_R][i]).abs())
            .sum();
        let max = (0..512)
            .map(|i| choir[&PortName::OUT][i].abs())
            .fold(0.0_f32, f32::max);
        assert!(
            diff > 0.1,
            "Mid-note unison increase should decorrelate channels, diff={diff}"
        );
        assert!(max.is_finite() && max <= 1.5, "Output bounded, max={max}");
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

        v.set_param(Param::VoiceSynth(VoiceSynthParam::UnisonDetune(
            Cents::new(25.0),
        )));
        let det = v
            .get_param(&Param::VoiceSynth(VoiceSynthParam::UnisonDetune(
                Cents::ZERO,
            )))
            .unwrap_or(0.0);
        assert!((det - 25.0).abs() < 0.001);
    }
}
