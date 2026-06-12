//! FOF — CHANT-style Formant-Wave-Function voice / choir module.
//!
//! Where `VoiceSynth` builds vowels by *filtering* a glottal source, FOF builds
//! them in the **time domain** from overlapping granular formant-wave-function
//! grains (FOF / *Fonction d'Onde Formantique*, Rodet, IRCAM 1984). Once per F0
//! period a fresh grain is fired for each formant band; each grain is that
//! formant's impulse response — a carrier sine at the formant centre frequency
//! under a raised-cosine attack into an exponential decay:
//!
//! ```text
//! grain(t) = A · env(t) · sin(2π·Fc·t)
//! env(t)   = ½(1 − cos(π·t/tex)) · e^(−β·t)   for t < tex   (attack)
//!          = e^(−β·t)                          for t ≥ tex   (decay)
//! ```
//!
//! `β = π·BW` sets the formant bandwidth (decay rate); `tex` (the **Skirt**
//! parameter) shapes the attack / high-frequency skirt. Pitch is the grain
//! trigger *rate* (F0); the vowel is the grain *shape* — the two are fully
//! decoupled, which is the whole point of FOF.
//!
//! Expressivity (Phase 2): formant `Bandwidth`, breath/aspiration noise,
//! internal vibrato, and `pitch_cv` / `vowel_cv` / `breath_cv` inputs.
//!
//! Choir (Phase 3): an internal bank of up to `MAX_UNISON` decorrelated
//! sub-voices — each its own granular FOF synth with detune, vibrato phase/rate,
//! formant jitter, onset stagger and stereo pan. Mono `out` is the un-panned
//! sum; `out_l`/`out_r` carry the stereo spread. See `plans/fof-plan.md`. Vowel
//! data is the shared `formant_tables`.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{Cents, Hertz, MidiNote, NormalizedValue, PortName, SampleRate, Velocity};
use synth_core::{FofParam, ModuleType, Param};

use crate::formant_tables::NUM_BANDS;

/// Maximum simultaneously-active grains per formant band, per sub-voice (fixed
/// ring — no heap in `process()`).
///
/// Grain lifetime is set by the formant bandwidth (the decay rate `β = π·BW`),
/// *not* by pitch — so the number of overlapping grains is `lifetime × F0` and
/// grows with pitch. The ring is sized for the *highest* musical F0 with the
/// *narrowest effective* bandwidth: the narrowest vowel band (BW ≈ 50 Hz) at
/// the lowest `Bandwidth` setting (×0.5 → ≈ 25 Hz effective) gives lifetime
/// ≈ ln(1/ENV_FLOOR)/(π·BW) ≈ 88 ms at the −60 dB floor; at F0 ≈ 1 kHz (≈ C6,
/// top of the soprano range) that is ≈ 88 grains. 96 covers it. Above that the
/// round-robin overwrites the oldest grain; under a single decay the oldest is
/// also the most-decayed, so the artifact is bounded by its residual envelope.
///
/// Per-band active-grain tracking (`count`) keeps the per-sample scan
/// proportional to the *live* grains. It assumes grains in a band retire oldest
/// first, which holds under a fixed decay. A mid-block vowel-CV sweep changes the
/// decay of *newly* fired grains, so a newer grain can briefly outlive an older
/// one; the reclaim then stops at the still-live old grain and `count` may
/// transiently over-estimate. That only costs extra scan iterations (dead slots
/// are skipped, never summed) and self-heals once the old grain retires — output
/// stays correct, and the cost never exceeds the naive full-ring scan.
const MAX_GRAINS: usize = 96;
/// Maximum number of unison sub-voices (fixed array — no heap in `process()`).
const MAX_UNISON: usize = 16;
/// Envelope level below which a grain is retired (−60 dB — inaudible, and keeps
/// the active-grain count bounded; see `MAX_GRAINS`).
const ENV_FLOOR: f32 = 1.0e-3;
/// Makeup gain applied to the summed grains before clipping. Kept ≤ 1 so that
/// overlapping grains from several F0 periods don't drive `soft_clip` hard.
const FOF_GAIN: f32 = 1.0;
/// Skirt → excitation time `tex`, in seconds: 0 = sharp/bright, 1 = soft/dull.
const TEX_MIN_SECS: f32 = 0.000_3;
const TEX_MAX_SECS: f32 = 0.003;
/// Lower bound on the per-sample trigger-phase increment — keeps the trigger
/// alive (and finite) without firing spurious grains during near-DC pitches.
const MIN_TRIGGER_INC: f32 = 1.0e-6;
/// Upper bound on the trigger-phase increment (< 1.0), so at most one grain set
/// fires per sample. 0.5 caps the trigger rate at the Nyquist frequency.
const MAX_TRIGGER_INC: f32 = 0.5;
/// Clamp on `pitch_cv` (semitones) so the effective F0 stays finite.
const MAX_PITCH_CV_SEMITONES: f32 = 60.0;
/// Non-zero PRNG seed (golden-ratio constant) for the breath-noise generator.
const RNG_SEED: u32 = 0x9E37_79B9;
/// Makeup gain applied to breath noise before it joins the voiced grains.
const NOISE_GAIN: f32 = 0.5;
/// Cutoff (Hz) of the one-pole lowpass that takes the harsh top off the breath.
const BREATH_LP_FC: f32 = 5000.0;
/// Vowel-CV modulation depth (matches `voice_synth`'s convention): a ±1 CV
/// shifts the vowel position by ±0.5 of the A→U range.
const VOWEL_CV_DEPTH: f32 = 0.5;
/// Maximum per-voice onset stagger, in seconds, for choir attack decorrelation.
const ONSET_MAX_SECS: f32 = 0.004;

/// Bandwidth-param → BW scale: 0.5 = ×1 (table), 0.0 = ×0.5, 1.0 = ×2.0.
#[inline]
fn bandwidth_scale(norm: f32) -> f32 {
    (2.0_f32).powf(2.0 * norm - 1.0)
}

/// Deterministic decorrelation hash in [0, 1) from (voice index, note, salt).
/// Reproducible and allocation-free — used instead of `Math.random`.
#[inline]
fn decorr_hash(voice: usize, note: f32, salt: f32) -> f32 {
    let x = voice as f32 * 0.618_034 + note * 0.019_3 + salt;
    let h = (x.sin() * 43758.547).abs();
    h - h.floor()
}

/// One active FOF grain — a decaying formant impulse response.
#[derive(Clone, Copy, Default)]
struct Grain {
    active: bool,
    /// Carrier phase in turns (1.0 = one cycle).
    phase: f32,
    /// Carrier phase increment per sample (Fc / sample_rate).
    phase_inc: f32,
    /// Grain amplitude (formant gain).
    amp: f32,
    /// Current exponential-decay envelope value (starts at 1.0).
    env: f32,
    /// Per-sample envelope decay multiplier (e^(−β / sample_rate)).
    decay: f32,
    /// Age in samples since the grain was triggered.
    age: u32,
    /// Attack length in samples (≥ 1).
    tex: u32,
}

impl Grain {
    /// Advance the grain one sample and return its contribution. Retires the
    /// grain once its envelope falls below `ENV_FLOOR`.
    #[inline]
    fn next_sample(&mut self) -> f32 {
        // Raised-cosine attack via `fast_sin_turns` (cos θ = sin(θ + π/2)).
        let attack = if self.age < self.tex {
            let frac = self.age as f32 / self.tex as f32;
            0.5 - 0.5 * crate::math::fast_sin_turns(0.25 + 0.5 * frac)
        } else {
            1.0
        };
        let out = self.amp * self.env * attack * crate::math::fast_sin_turns(self.phase);

        self.phase = (self.phase + self.phase_inc).fract();
        self.env *= self.decay;
        self.age = self.age.saturating_add(1);
        if self.env < ENV_FLOOR {
            self.active = false;
        }
        out
    }
}

/// Per-band formant target (centre frequency, decay rate, gain) for a vowel,
/// used when triggering grains.
#[derive(Clone, Copy, Default)]
struct BandTarget {
    phase_inc: f32,
    decay: f32,
    amp: f32,
}

/// Output normalization for a grain target set: `1 / Σ amp`, so the summed
/// formant gains stay near unity regardless of vowel.
#[inline]
fn grain_norm(targets: &[BandTarget; NUM_BANDS]) -> f32 {
    let gain_sum: f32 = targets.iter().map(|t| t.amp).sum();
    if gain_sum > 1.0e-7 {
        1.0 / gain_sum
    } else {
        1.0
    }
}

/// One decorrelated unison sub-voice: an independent granular FOF synth with its
/// own grain rings, trigger/vibrato phase, breath noise, detune, formant jitter,
/// onset and pan.
#[derive(Clone)]
struct SubVoice {
    grains: [[Grain; MAX_GRAINS]; NUM_BANDS],
    /// Per-band write cursor (one past the newest grain).
    head: [usize; NUM_BANDS],
    /// Per-band live-grain count (the scan iterates only this many).
    count: [usize; NUM_BANDS],
    trigger_phase: f32,
    vibrato_phase: f32,
    rng_state: u32,
    breath_lp: f32,

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
            grains: [[Grain::default(); MAX_GRAINS]; NUM_BANDS],
            head: [0; NUM_BANDS],
            count: [0; NUM_BANDS],
            trigger_phase: 0.0,
            vibrato_phase: 0.0,
            rng_state: RNG_SEED,
            breath_lp: 0.0,
            detune_cents: 0.0,
            vib_rate_mult: 1.0,
            formant_jitter: 1.0,
            pan_l: std::f32::consts::FRAC_1_SQRT_2,
            pan_r: std::f32::consts::FRAC_1_SQRT_2,
            onset_countdown: 0,
        }
    }

    /// Re-arm phases / state for a new note. `trigger_phase` / `vib_phase`
    /// decorrelate this voice from the others; grains are cleared (count = 0).
    fn restart(&mut self, seed: u32, trigger_phase: f32, vib_phase: f32, onset: u32) {
        self.head = [0; NUM_BANDS];
        self.count = [0; NUM_BANDS];
        self.trigger_phase = trigger_phase;
        self.vibrato_phase = vib_phase;
        self.rng_state = seed.max(1);
        self.breath_lp = 0.0;
        self.onset_countdown = onset;
    }

    /// Fire one fresh grain per band from the given targets.
    #[inline]
    fn trigger(&mut self, targets: &[BandTarget; NUM_BANDS], tex: u32) {
        for band in 0..NUM_BANDS {
            let head = self.head[band];
            self.grains[band][head] = Grain {
                active: true,
                phase: 0.0,
                phase_inc: targets[band].phase_inc,
                amp: targets[band].amp,
                env: 1.0,
                decay: targets[band].decay,
                age: 0,
                tex,
            };
            self.head[band] = (head + 1) % MAX_GRAINS;
            if self.count[band] < MAX_GRAINS {
                self.count[band] += 1;
            }
        }
    }

    /// Advance every live grain one sample and return their sum. The scan is
    /// bounded by the per-band live-grain `count`; retired grains are reclaimed
    /// from the oldest end. Grains in a band normally retire oldest-first (shared
    /// decay); under a mid-block decay change `count` may briefly over-estimate
    /// (see `MAX_GRAINS`), which only adds skipped-slot iterations, never error.
    #[inline]
    fn run_grains(&mut self) -> f32 {
        let mut sum = 0.0_f32;
        for band in 0..NUM_BANDS {
            let head = self.head[band];
            let count = self.count[band];
            for k in 0..count {
                let idx = (head + MAX_GRAINS - count + k) % MAX_GRAINS;
                if self.grains[band][idx].active {
                    sum += self.grains[band][idx].next_sample();
                }
            }
            // Reclaim retired grains from the oldest end.
            let mut c = self.count[band];
            while c > 0 {
                let oldest = (head + MAX_GRAINS - c) % MAX_GRAINS;
                if self.grains[band][oldest].active {
                    break;
                }
                c -= 1;
            }
            self.count[band] = c;
        }
        sum
    }
}

/// FOF module — CHANT-style granular formant voice / choir.
#[derive(Clone)]
pub struct Fof {
    // Parameters
    vowel: NormalizedValue,
    formant_shift: NormalizedValue,
    skirt: NormalizedValue,
    bandwidth: NormalizedValue,
    breathiness: NormalizedValue,
    vibrato_rate: Hertz,
    vibrato_depth: Cents,
    unison_voices: NormalizedValue,
    unison_detune: Cents,
    unison_spread: NormalizedValue,
    level: NormalizedValue,

    // Voice state
    note_freq: synth_core::Hertz,
    /// Effective (CV-modulated) vowel position in 0..1, drives `band_targets`.
    current_vowel: f32,
    voices: [SubVoice; MAX_UNISON],
    active_voices: usize,

    // Cached
    sample_rate: SampleRate,
    inv_sample_rate: f32,

    // Pre-allocated output buffers
    out_buffer: AudioBuffer,
    out_l_buffer: AudioBuffer,
    out_r_buffer: AudioBuffer,
}

impl Fof {
    pub fn new() -> Self {
        let mut f = Self {
            vowel: NormalizedValue::MIN,
            formant_shift: NormalizedValue::new(0.5),
            skirt: NormalizedValue::new(0.3),
            bandwidth: NormalizedValue::new(0.5),
            breathiness: NormalizedValue::MIN,
            vibrato_rate: Hertz::new(5.5),
            vibrato_depth: Cents::ZERO,
            unison_voices: NormalizedValue::MIN,
            unison_detune: Cents::new(15.0),
            unison_spread: NormalizedValue::new(0.7),
            level: NormalizedValue::new(0.8),
            note_freq: synth_core::Hertz::ZERO,
            current_vowel: 0.0,
            voices: std::array::from_fn(|_| SubVoice::new()),
            active_voices: 1,
            sample_rate: SampleRate::DVD_QUALITY,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),
            out_buffer: AudioBuffer::new(1024),
            out_l_buffer: AudioBuffer::new(1024),
            out_r_buffer: AudioBuffer::new(1024),
        };
        f.derive_decorrelation();
        f
    }

    /// Excitation time `tex` in samples for the current Skirt setting (≥ 1).
    #[inline]
    fn tex_samples(&self) -> u32 {
        let secs = TEX_MIN_SECS + self.skirt.as_f32() * (TEX_MAX_SECS - TEX_MIN_SECS);
        (secs * self.sample_rate.as_f32()).round().max(1.0) as u32
    }

    /// Number of active unison sub-voices (1..=MAX_UNISON).
    #[inline]
    fn unison_count(&self) -> usize {
        let n = 1 + (self.unison_voices.as_f32() * (MAX_UNISON - 1) as f32).round() as usize;
        n.clamp(1, MAX_UNISON)
    }

    /// Compute the per-band grain targets for the given vowel position, applying
    /// the formant shift, the bandwidth scale, and this voice's formant jitter.
    fn band_targets(&self, vowel: f32, jitter: f32) -> [BandTarget; NUM_BANDS] {
        let (freqs, bws, gains) = crate::formant_tables::interpolate_vowel(vowel);
        let scale =
            crate::formant_tables::formant_shift_factor(self.formant_shift.as_f32()) * jitter;
        let bw_scale = bandwidth_scale(self.bandwidth.as_f32());
        let sr = self.sample_rate.as_f32();
        let inv_sr = self.inv_sample_rate;

        let mut targets = [BandTarget::default(); NUM_BANDS];
        for (band, target) in targets.iter_mut().enumerate() {
            let scaled = (freqs[band] * scale).clamp(20.0, sr * 0.45);
            let beta = std::f32::consts::PI * (bws[band] * bw_scale).max(10.0);
            *target = BandTarget {
                phase_inc: scaled * inv_sr,
                decay: (-beta * inv_sr).exp(),
                amp: gains[band],
            };
        }
        targets
    }

    /// Recompute per-voice decorrelation (detune, vibrato rate, formant jitter,
    /// pan) for the active sub-voices. Deterministic from voice index + note, so
    /// it is idempotent and cheap to call every block.
    fn derive_decorrelation(&mut self) {
        let active = self.active_voices;
        let detune = self.unison_detune.as_f32();
        let spread = self.unison_spread.as_f32();
        let note = self.note_freq.as_f32();

        for (v, voice) in self.voices[..active].iter_mut().enumerate() {
            if active == 1 {
                voice.detune_cents = 0.0;
                voice.vib_rate_mult = 1.0;
                voice.formant_jitter = 1.0;
                voice.pan_l = std::f32::consts::FRAC_1_SQRT_2;
                voice.pan_r = std::f32::consts::FRAC_1_SQRT_2;
                continue;
            }
            voice.detune_cents = (decorr_hash(v, note, 1.0) * 2.0 - 1.0) * detune;
            voice.vib_rate_mult = 1.0 + (decorr_hash(v, note, 2.0) * 2.0 - 1.0) * 0.08;
            voice.formant_jitter = 1.0 + (decorr_hash(v, note, 3.0) * 2.0 - 1.0) * 0.03;
            let pos = (v as f32 / (active - 1) as f32) * 2.0 - 1.0;
            let (l, r) = crate::math::equal_power_pan(pos * spread);
            voice.pan_l = l;
            voice.pan_r = r;
        }
    }
}

impl Default for Fof {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Fof {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("fof", "FOF")
            .description(
                "CHANT-style granular formant voice / choir (FOF / formant wave functions)",
            )
            .category(ModuleCategory::Oscillator)
            .tag("voice")
            .tag("vocal")
            .tag("formant")
            .tag("fof")
            .tag("chant")
            .tag("choir")
            .tag("synthesis")
            .parameter(
                ParameterDescriptor::float(
                    "vowel",
                    Param::Fof(FofParam::Vowel(NormalizedValue::MIN)),
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
                    Param::Fof(FofParam::FormantShift(NormalizedValue::new(0.5))),
                    "Formant Shift",
                )
                .description("Vocal-tract length: <0.5 longer/darker, >0.5 shorter/brighter")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "skirt",
                    Param::Fof(FofParam::Skirt(NormalizedValue::new(0.3))),
                    "Skirt",
                )
                .description("Grain attack / formant skirt: 0 sharp & bright, 1 soft & dull")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "bandwidth",
                    Param::Fof(FofParam::Bandwidth(NormalizedValue::new(0.5))),
                    "Bandwidth",
                )
                .description("Formant bandwidth: 0.5 = natural, <0.5 narrower/sharper, >0.5 wider")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "breathiness",
                    Param::Fof(FofParam::Breathiness(NormalizedValue::MIN)),
                    "Breathiness",
                )
                .description("Aspiration / breath noise amount")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "vibrato_rate",
                    Param::Fof(FofParam::VibratoRate(Hertz::new(5.5))),
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
                    Param::Fof(FofParam::VibratoDepth(Cents::ZERO)),
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
                    Param::Fof(FofParam::UnisonVoices(NormalizedValue::MIN)),
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
                    Param::Fof(FofParam::UnisonDetune(Cents::new(15.0))),
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
                    Param::Fof(FofParam::UnisonSpread(NormalizedValue::new(0.7))),
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
                    Param::Fof(FofParam::Level(NormalizedValue::new(0.8))),
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

impl PolyModule for Fof {
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

        // No note playing — silence on all three ports.
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
        let pitch_cv_connected = pitch_cv.is_connected();
        let vowel_cv_connected = vowel_cv.is_connected();

        // Recompute active count + decorrelation for this block.
        self.active_voices = self.unison_count();
        self.derive_decorrelation();
        let active = self.active_voices;

        if !vowel_cv_connected {
            self.current_vowel = self.vowel.as_f32();
        }

        let inv_sr = self.inv_sample_rate;
        let level = self.level.as_f32();
        let base_freq = self.note_freq.as_f32();
        let tex = self.tex_samples();
        let breath_base = self.breathiness.as_f32();
        let vib_depth_cents = self.vibrato_depth.as_f32();
        let vib_inc = self.vibrato_rate.as_f32() * inv_sr;
        let unison_norm = 1.0 / (active as f32).sqrt();
        let out_gain = FOF_GAIN * level;
        let breath_lp_coef = crate::math::one_pole_lp_coef(BREATH_LP_FC, inv_sr);

        // Per-voice grain targets + normalization for the current vowel. Each
        // sub-voice has its own formant jitter, so targets differ per voice;
        // recomputed when the vowel position shifts (vowel CV).
        let mut targets = [[BandTarget::default(); NUM_BANDS]; MAX_UNISON];
        let mut norms = [1.0_f32; MAX_UNISON];
        for v in 0..active {
            targets[v] = self.band_targets(self.current_vowel, self.voices[v].formant_jitter);
            norms[v] = grain_norm(&targets[v]);
        }

        // Fast path: with neither pitch CV nor vibrato, each voice's trigger
        // increment is a per-voice constant (it depends only on the fixed detune),
        // so the per-sample `exp2` / `fast_sin_turns` hoist out entirely. Covers
        // both the solo voice and the common "fat static unison" choir.
        let vib_on = vib_depth_cents != 0.0;
        let static_inc: Option<[f32; MAX_UNISON]> = if !pitch_cv_connected && !vib_on {
            let mut incs = [0.0_f32; MAX_UNISON];
            for v in 0..active {
                let freq = base_freq * crate::math::cents_to_ratio(self.voices[v].detune_cents);
                incs[v] = (freq * inv_sr).clamp(MIN_TRIGGER_INC, MAX_TRIGGER_INC);
            }
            Some(incs)
        } else {
            None
        };

        for i in 0..num_samples {
            // Vowel CV: recompute per-voice targets/norms on a position shift.
            if vowel_cv_connected {
                let target_vowel =
                    (self.vowel.as_f32() + vowel_cv[i] * VOWEL_CV_DEPTH).clamp(0.0, 1.0);
                if (target_vowel - self.current_vowel).abs() > 0.001 {
                    self.current_vowel = target_vowel;
                    for v in 0..active {
                        targets[v] = self.band_targets(target_vowel, self.voices[v].formant_jitter);
                        norms[v] = grain_norm(&targets[v]);
                    }
                }
            }

            let pcv = if pitch_cv_connected {
                pitch_cv[i].clamp(-MAX_PITCH_CV_SEMITONES, MAX_PITCH_CV_SEMITONES)
            } else {
                0.0
            };
            let breath = (breath_base + breath_cv[i]).clamp(0.0, 1.0);

            let mut raw = 0.0_f32;
            let mut sum_l = 0.0_f32;
            let mut sum_r = 0.0_f32;

            for v in 0..active {
                let voice = &mut self.voices[v];

                // Staggered onset: stay silent (phases frozen) until armed.
                if voice.onset_countdown > 0 {
                    voice.onset_countdown -= 1;
                    continue;
                }

                // Pitch: base ± detune/vibrato (cents) ± pitch_cv (semitones).
                // The increment is clamped < 1.0 so at most one grain set fires
                // per sample (a single `-= 1.0` suffices) and a clamped, finite
                // CV keeps `freq` finite.
                let inc = if let Some(incs) = &static_inc {
                    incs[v]
                } else {
                    let vib_cents = if vib_on {
                        vib_depth_cents * crate::math::fast_sin_turns(voice.vibrato_phase)
                    } else {
                        0.0
                    };
                    let semis = pcv + (voice.detune_cents + vib_cents) / 100.0;
                    let freq = base_freq * crate::math::semitones_to_ratio(semis);
                    (freq * inv_sr).clamp(MIN_TRIGGER_INC, MAX_TRIGGER_INC)
                };

                voice.trigger_phase += inc;
                if voice.trigger_phase >= 1.0 {
                    voice.trigger_phase -= 1.0;
                    voice.trigger(&targets[v], tex);
                }
                // Advance the vibrato LFO only when it is actually in use.
                if vib_on {
                    voice.vibrato_phase =
                        (voice.vibrato_phase + vib_inc * voice.vib_rate_mult).fract();
                }

                // Sum this voice's live grains (normalized voiced signal).
                let mut voiced = voice.run_grains() * norms[v];

                // Breath / aspiration: lowpass-shaped white noise. The filter /
                // PRNG run every sample (no state freeze when breath = 0); only
                // the output is gated, so re-opening breath has no discontinuity.
                let n = crate::math::xorshift_noise(&mut voice.rng_state);
                voice.breath_lp += breath_lp_coef * (n - voice.breath_lp);
                if breath > 0.0 {
                    voiced += voice.breath_lp * breath * NOISE_GAIN;
                }

                raw += voiced;
                sum_l += voiced * voice.pan_l;
                sum_r += voiced * voice.pan_r;
            }

            self.out_buffer[i] = crate::math::soft_clip(raw * unison_norm * out_gain);
            self.out_l_buffer[i] = crate::math::soft_clip(sum_l * unison_norm * out_gain);
            self.out_r_buffer[i] = crate::math::soft_clip(sum_r * unison_norm * out_gain);
        }

        self.write_outputs(outputs);
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Fof(p) = param {
            match p {
                FofParam::Vowel(v) => {
                    self.vowel = v;
                    self.current_vowel = v.as_f32();
                }
                FofParam::FormantShift(v) => self.formant_shift = v,
                FofParam::Skirt(v) => self.skirt = v,
                FofParam::Bandwidth(v) => self.bandwidth = v,
                FofParam::Breathiness(v) => self.breathiness = v,
                FofParam::VibratoRate(hz) => self.vibrato_rate = hz,
                FofParam::VibratoDepth(c) => self.vibrato_depth = c,
                FofParam::UnisonVoices(v) => self.unison_voices = v,
                FofParam::UnisonDetune(c) => self.unison_detune = c,
                FofParam::UnisonSpread(v) => self.unison_spread = v,
                FofParam::Level(v) => self.level = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Fof(p) = param {
            Some(match p {
                FofParam::Vowel(_) => self.vowel.as_f32(),
                FofParam::FormantShift(_) => self.formant_shift.as_f32(),
                FofParam::Skirt(_) => self.skirt.as_f32(),
                FofParam::Bandwidth(_) => self.bandwidth.as_f32(),
                FofParam::Breathiness(_) => self.breathiness.as_f32(),
                FofParam::VibratoRate(_) => self.vibrato_rate.as_f32(),
                FofParam::VibratoDepth(_) => self.vibrato_depth.as_f32(),
                FofParam::UnisonVoices(_) => self.unison_voices.as_f32(),
                FofParam::UnisonDetune(_) => self.unison_detune.as_f32(),
                FofParam::UnisonSpread(_) => self.unison_spread.as_f32(),
                FofParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Fof(FofParam::Vowel(self.vowel)),
            Param::Fof(FofParam::FormantShift(self.formant_shift)),
            Param::Fof(FofParam::Skirt(self.skirt)),
            Param::Fof(FofParam::Bandwidth(self.bandwidth)),
            Param::Fof(FofParam::Breathiness(self.breathiness)),
            Param::Fof(FofParam::VibratoRate(self.vibrato_rate)),
            Param::Fof(FofParam::VibratoDepth(self.vibrato_depth)),
            Param::Fof(FofParam::UnisonVoices(self.unison_voices)),
            Param::Fof(FofParam::UnisonDetune(self.unison_detune)),
            Param::Fof(FofParam::UnisonSpread(self.unison_spread)),
            Param::Fof(FofParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Fof
    }

    fn reset(&mut self) {
        self.current_vowel = self.vowel.as_f32();
        for voice in &mut self.voices {
            *voice = SubVoice::new();
        }
        self.derive_decorrelation();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.active_voices = self.unison_count();
        self.current_vowel = self.vowel.as_f32();
        let active = self.active_voices;
        let note_hz = self.note_freq.as_f32();
        let onset_max = (ONSET_MAX_SECS * self.sample_rate.as_f32()).max(0.0);

        // Decorrelate the phases of EVERY voice (not just the active ones), so
        // raising UnisonVoices mid-note still finds the newly-activated voices
        // phase-staggered. Voice 0 starts at trigger phase 1.0 (fires on sample
        // 0); at active==1 `derive_decorrelation` also leaves it un-detuned and
        // centred, so a solo voice matches the single-voice path. (In a choir
        // voice 0 is detuned/panned like the rest — it is only special-cased for
        // the solo count.) Onset stagger applies only to voices active from this
        // note's attack. The breath PRNG is seeded per voice (mixing in the note)
        // so the choir's aspiration is decorrelated.
        for (v, voice) in self.voices.iter_mut().enumerate() {
            let (trigger_phase, vib_phase) = if v == 0 {
                // Start at 1.0 so the first processed sample fires a grain set.
                (1.0, 0.0)
            } else {
                (decorr_hash(v, note_hz, 6.0), decorr_hash(v, note_hz, 4.0))
            };
            let onset = if v == 0 || v >= active {
                0
            } else {
                (decorr_hash(v, note_hz, 5.0) * onset_max) as u32
            };
            let seed = (RNG_SEED ^ note_hz.to_bits()) ^ (v as u32 + 1).wrapping_mul(0x9E37_79B9);
            voice.restart(seed, trigger_phase, vib_phase, onset);
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

impl Fof {
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

    fn outputs(n: usize) -> HashMap<PortName, AudioBuffer> {
        let mut m = HashMap::new();
        m.insert(PortName::OUT, AudioBuffer::new(n));
        m.insert(PortName::OUT_L, AudioBuffer::new(n));
        m.insert(PortName::OUT_R, AudioBuffer::new(n));
        m
    }

    #[test]
    fn test_fof_creation() {
        let f = Fof::new();
        assert!((f.note_freq.as_f32() - 0.0).abs() < f32::EPSILON);
        assert_eq!(f.active_voices, 1);
    }

    #[test]
    fn test_fof_produces_sound() {
        let mut f = Fof::new();
        f.note_on(MidiNote::new(57), Velocity::new(0.8));
        let mut out = outputs(512);
        f.process(InputPorts::empty(), &mut out, &ctx(512));

        let buf = &out[&PortName::OUT];
        let max = (0..512).map(|i| buf[i].abs()).fold(0.0_f32, f32::max);
        assert!(max > 0.01, "FOF should produce sound, max={max}");
    }

    #[test]
    fn test_fof_silence_without_note() {
        let mut f = Fof::new();
        let mut out = outputs(64);
        f.process(InputPorts::empty(), &mut out, &ctx(64));

        let buf = &out[&PortName::OUT];
        let max = (0..64).map(|i| buf[i].abs()).fold(0.0_f32, f32::max);
        assert!(max < 0.001, "Should be silent without note_on, max={max}");
    }

    #[test]
    fn test_fof_output_bounded() {
        let mut f = Fof::new();
        f.set_param(Param::Fof(FofParam::Level(NormalizedValue::MAX)));
        f.set_param(Param::Fof(FofParam::Skirt(NormalizedValue::MAX)));
        f.set_param(Param::Fof(FofParam::Breathiness(NormalizedValue::MAX)));
        f.set_param(Param::Fof(FofParam::UnisonVoices(NormalizedValue::MAX)));
        f.note_on(MidiNote::new(60), Velocity::new(1.0));
        let mut out = outputs(1024);
        f.process(InputPorts::empty(), &mut out, &ctx(1024));

        for port in [PortName::OUT, PortName::OUT_L, PortName::OUT_R] {
            let buf = &out[&port];
            let max = (0..1024).map(|i| buf[i].abs()).fold(0.0_f32, f32::max);
            assert!(max.is_finite() && max <= 1.5, "Output bounded, max={max}");
        }
    }

    /// Worst case for the grain ring: a high F0 (C6) with the narrowest-band
    /// vowel (U) and the lowest Bandwidth packs the most overlapping grains.
    /// Output must stay finite and bounded.
    #[test]
    fn test_fof_high_pitch_bounded() {
        let mut f = Fof::new();
        f.set_param(Param::Fof(FofParam::Vowel(NormalizedValue::MAX)));
        f.set_param(Param::Fof(FofParam::Bandwidth(NormalizedValue::MIN)));
        f.set_param(Param::Fof(FofParam::Level(NormalizedValue::MAX)));
        f.note_on(MidiNote::new(84), Velocity::new(1.0));
        let mut out = outputs(2048);
        f.process(InputPorts::empty(), &mut out, &ctx(2048));

        let buf = &out[&PortName::OUT];
        let max = (0..2048).map(|i| buf[i].abs()).fold(0.0_f32, f32::max);
        assert!(
            max.is_finite() && max <= 1.5,
            "High-pitch output bounded, max={max}"
        );
        assert!(
            max > 0.01,
            "High-pitch should still produce sound, max={max}"
        );
    }

    /// A non-finite / absurdly large `pitch_cv` must not produce non-finite
    /// output — the increment clamp keeps the trigger phase and `freq` sane.
    #[test]
    fn test_fof_extreme_pitch_cv_finite() {
        let mut f = Fof::new();
        f.note_on(MidiNote::new(60), Velocity::new(1.0));

        let mut cv = AudioBuffer::new(256);
        for i in 0..256 {
            cv[i] = if i % 2 == 0 { f32::INFINITY } else { 1.0e30 };
        }
        let ports_data = [(PortName::intern("pitch_cv"), &cv)];
        let inputs = InputPorts::new(&ports_data);

        let mut out = outputs(256);
        f.process(inputs, &mut out, &ctx(256));

        let buf = &out[&PortName::OUT];
        assert!(
            (0..256).all(|i| buf[i].is_finite()),
            "Extreme pitch_cv must not yield non-finite output"
        );
    }

    fn total_energy(f: &mut Fof, n: usize) -> f32 {
        let mut out = outputs(n);
        f.process(InputPorts::empty(), &mut out, &ctx(n));
        (0..n).map(|i| out[&PortName::OUT][i].abs()).sum::<f32>()
    }

    #[test]
    fn test_fof_breathiness_changes_output() {
        let render = |breath: f32| {
            let mut f = Fof::new();
            f.set_param(Param::Fof(FofParam::Breathiness(NormalizedValue::new(
                breath,
            ))));
            f.note_on(MidiNote::new(50), Velocity::new(1.0));
            total_energy(&mut f, 1024)
        };
        let dry = render(0.0);
        let breathy = render(1.0);
        assert!(
            (dry - breathy).abs() > 0.01,
            "Breathiness should change output: dry={dry}, breathy={breathy}"
        );
    }

    #[test]
    fn test_fof_bandwidth_changes_output() {
        let render = |bw: f32| {
            let mut f = Fof::new();
            f.set_param(Param::Fof(FofParam::Bandwidth(NormalizedValue::new(bw))));
            f.note_on(MidiNote::new(48), Velocity::new(1.0));
            total_energy(&mut f, 2048)
        };
        let narrow = render(0.0);
        let wide = render(1.0);
        assert!(
            (narrow - wide).abs() > 0.01,
            "Bandwidth should change output: narrow={narrow}, wide={wide}"
        );
    }

    #[test]
    fn test_fof_vibrato_modulates_pitch() {
        let render = |depth: f32| {
            let mut f = Fof::new();
            f.set_param(Param::Fof(FofParam::VibratoRate(Hertz::new(6.0))));
            f.set_param(Param::Fof(FofParam::VibratoDepth(Cents::new(depth))));
            f.note_on(MidiNote::new(45), Velocity::new(1.0));
            total_energy(&mut f, 4096)
        };
        let steady = render(0.0);
        let vibrato = render(100.0);
        assert!(
            (steady - vibrato).abs() > 0.01,
            "Vibrato should modulate output: steady={steady}, vibrato={vibrato}"
        );
    }

    #[test]
    fn test_fof_vowel_cv_morphs() {
        let mut f_static = Fof::new();
        f_static.set_param(Param::Fof(FofParam::Vowel(NormalizedValue::MIN)));
        f_static.note_on(MidiNote::new(52), Velocity::new(1.0));
        let static_e = total_energy(&mut f_static, 2048);

        let mut f_cv = Fof::new();
        f_cv.set_param(Param::Fof(FofParam::Vowel(NormalizedValue::MIN)));
        f_cv.note_on(MidiNote::new(52), Velocity::new(1.0));
        let mut cv = AudioBuffer::new(2048);
        for i in 0..2048 {
            cv[i] = 1.0; // push the vowel fully toward U
        }
        let ports_data = [(PortName::intern("vowel_cv"), &cv)];
        let inputs = InputPorts::new(&ports_data);
        let mut out = outputs(2048);
        f_cv.process(inputs, &mut out, &ctx(2048));
        let cv_e = (0..2048).map(|i| out[&PortName::OUT][i].abs()).sum::<f32>();

        assert!(
            (static_e - cv_e).abs() > 0.01,
            "Vowel CV should change timbre: static={static_e}, cv={cv_e}"
        );
    }

    /// A solo voice pans to centre (L == R). A unison choir with spread
    /// decorrelates the channels, so |L − R| energy must grow.
    #[test]
    fn test_fof_unison_widens_stereo() {
        let stereo_diff = |voices: f32, spread: f32| {
            let mut f = Fof::new();
            f.set_param(Param::Fof(FofParam::UnisonVoices(NormalizedValue::new(
                voices,
            ))));
            f.set_param(Param::Fof(FofParam::UnisonSpread(NormalizedValue::new(
                spread,
            ))));
            f.set_param(Param::Fof(FofParam::UnisonDetune(Cents::new(30.0))));
            f.note_on(MidiNote::new(55), Velocity::new(1.0));
            let mut out = outputs(2048);
            f.process(InputPorts::empty(), &mut out, &ctx(2048));
            (0..2048)
                .map(|i| (out[&PortName::OUT_L][i] - out[&PortName::OUT_R][i]).abs())
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
    /// choir, not a phase-aligned coherent block.
    #[test]
    fn test_fof_unison_increase_mid_note() {
        let mut f = Fof::new();
        f.set_param(Param::Fof(FofParam::UnisonSpread(NormalizedValue::MAX)));
        f.set_param(Param::Fof(FofParam::UnisonDetune(Cents::new(30.0))));
        f.note_on(MidiNote::new(55), Velocity::new(1.0));

        let mut solo = outputs(512);
        f.process(InputPorts::empty(), &mut solo, &ctx(512));

        f.set_param(Param::Fof(FofParam::UnisonVoices(NormalizedValue::MAX)));
        let mut choir = outputs(512);
        f.process(InputPorts::empty(), &mut choir, &ctx(512));

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
    fn test_fof_params_roundtrip() {
        let mut f = Fof::new();
        f.set_param(Param::Fof(FofParam::FormantShift(NormalizedValue::new(
            0.7,
        ))));
        let got = f
            .get_param(&Param::Fof(FofParam::FormantShift(NormalizedValue::MIN)))
            .unwrap_or(0.0);
        assert!((got - 0.7).abs() < 0.001);

        f.set_param(Param::Fof(FofParam::UnisonDetune(Cents::new(25.0))));
        let det = f
            .get_param(&Param::Fof(FofParam::UnisonDetune(Cents::ZERO)))
            .unwrap_or(0.0);
        assert!((det - 25.0).abs() < 0.001);
    }
}
