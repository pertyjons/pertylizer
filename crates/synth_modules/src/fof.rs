//! FOF — CHANT-style Formant-Wave-Function voice module.
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
//! Phase 1: mono, single sound source, parameters `Vowel` / `FormantShift` /
//! `Skirt` / `Level`. Expressivity (Phase 2) and choir/unison (Phase 3) follow.
//! See `plans/fof-plan.md`. Vowel data is the shared `formant_tables`, the same
//! source `voice_synth` / `formant_filter` / `am_formant` use.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{Cents, Hertz, MidiNote, NormalizedValue, PortName, SampleRate, Velocity};
use synth_core::{FofParam, ModuleType, Param};

use crate::formant_tables::{FORMANT_BW, FORMANT_FREQ, FORMANT_GAIN, NUM_BANDS, NUM_VOWELS};

/// Maximum simultaneously-active grains per formant band (fixed ring — no heap
/// in `process()`).
///
/// Grain lifetime is set by the formant bandwidth (the decay rate `β = π·BW`),
/// *not* by pitch — so the number of overlapping grains is `lifetime × F0` and
/// grows with pitch. The ring must therefore be sized for the *highest* musical
/// F0 with the *narrowest effective* bandwidth, not the lowest. Worst case: the
/// narrowest vowel band (BW ≈ 50 Hz) at the lowest `Bandwidth` setting (×0.5 →
/// ≈ 25 Hz effective) gives lifetime ≈ ln(1/ENV_FLOOR)/(π·BW) ≈ 88 ms at the
/// −60 dB floor; at F0 ≈ 1 kHz (≈ C6, top of the soprano range) that is ≈ 88
/// grains. 96 covers that worst case. Above it the round-robin overwrites the
/// oldest (most-decayed) grain, so any artifact is bounded by its residual
/// envelope. (Phase 3 will track the active-grain range to bound the per-sample
/// scan once the choir multiplies this cost.)
const MAX_GRAINS: usize = 96;
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
/// Bandwidth-param → BW scale: 0.5 = ×1 (table), 0.0 = ×0.5, 1.0 = ×2.0.
#[inline]
fn bandwidth_scale(norm: f32) -> f32 {
    (2.0_f32).powf(2.0 * norm - 1.0)
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

/// Per-band formant target (centre frequency, decay rate, gain) for the current
/// vowel, used when triggering grains.
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

/// FOF module — CHANT-style granular formant voice (mono).
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
    level: NormalizedValue,

    // Voice state
    note_freq: synth_core::Hertz,
    /// Effective (CV-modulated) vowel position in 0..1, drives `band_targets`.
    current_vowel: f32,
    /// Glottal trigger phase in turns; wrapping past 1.0 fires a new grain set.
    trigger_phase: f32,
    /// Internal vibrato LFO phase in turns.
    vibrato_phase: f32,
    /// Breath-noise PRNG state (xorshift32).
    rng_state: u32,
    /// One-pole lowpass state shaping the breath noise.
    breath_lp: f32,
    grains: [[Grain; MAX_GRAINS]; NUM_BANDS],
    /// Round-robin write cursor per band (persists across blocks).
    cursor: [usize; NUM_BANDS],

    // Cached
    sample_rate: SampleRate,
    inv_sample_rate: f32,

    // Pre-allocated output buffer
    out_buffer: AudioBuffer,
}

impl Fof {
    pub fn new() -> Self {
        Self {
            vowel: NormalizedValue::MIN,
            formant_shift: NormalizedValue::new(0.5),
            skirt: NormalizedValue::new(0.3),
            bandwidth: NormalizedValue::new(0.5),
            breathiness: NormalizedValue::MIN,
            vibrato_rate: Hertz::new(5.5),
            vibrato_depth: Cents::ZERO,
            level: NormalizedValue::new(0.8),
            note_freq: synth_core::Hertz::ZERO,
            current_vowel: 0.0,
            trigger_phase: 0.0,
            vibrato_phase: 0.0,
            rng_state: RNG_SEED,
            breath_lp: 0.0,
            grains: [[Grain::default(); MAX_GRAINS]; NUM_BANDS],
            cursor: [0; NUM_BANDS],
            sample_rate: SampleRate::DVD_QUALITY,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),
            out_buffer: AudioBuffer::new(1024),
        }
    }

    /// Excitation time `tex` in samples for the current Skirt setting (≥ 1).
    #[inline]
    fn tex_samples(&self) -> u32 {
        let secs = TEX_MIN_SECS + self.skirt.as_f32() * (TEX_MAX_SECS - TEX_MIN_SECS);
        (secs * self.sample_rate.as_f32()).round().max(1.0) as u32
    }

    /// Compute the per-band grain targets for the given vowel position, applying
    /// the formant shift and the bandwidth scale.
    fn band_targets(&self, vowel: f32) -> [BandTarget; NUM_BANDS] {
        let pos = vowel * (NUM_VOWELS - 1) as f32;
        let idx = (pos as usize).min(NUM_VOWELS - 2);
        let frac = pos - idx as f32;
        let shift = crate::formant_tables::formant_shift_factor(self.formant_shift.as_f32());
        let bw_scale = bandwidth_scale(self.bandwidth.as_f32());
        let sr = self.sample_rate.as_f32();
        let inv_sr = self.inv_sample_rate;

        let mut targets = [BandTarget::default(); NUM_BANDS];
        for (band, target) in targets.iter_mut().enumerate() {
            let freq = FORMANT_FREQ[idx][band] * (1.0 - frac) + FORMANT_FREQ[idx + 1][band] * frac;
            let bw = FORMANT_BW[idx][band] * (1.0 - frac) + FORMANT_BW[idx + 1][band] * frac;
            let gain = FORMANT_GAIN[idx][band] * (1.0 - frac) + FORMANT_GAIN[idx + 1][band] * frac;

            let scaled = (freq * shift).clamp(20.0, sr * 0.45);
            let beta = std::f32::consts::PI * (bw * bw_scale).max(10.0);
            *target = BandTarget {
                phase_inc: scaled * inv_sr,
                decay: (-beta * inv_sr).exp(),
                amp: gain,
            };
        }
        targets
    }

    /// Fire one fresh grain per band from the given targets.
    fn trigger(&mut self, targets: &[BandTarget; NUM_BANDS], tex: u32) {
        for (band, ring) in self.grains.iter_mut().enumerate() {
            let slot = self.cursor[band];
            ring[slot] = Grain {
                active: true,
                phase: 0.0,
                phase_inc: targets[band].phase_inc,
                amp: targets[band].amp,
                env: 1.0,
                decay: targets[band].decay,
                age: 0,
                tex,
            };
            self.cursor[band] = (slot + 1) % MAX_GRAINS;
        }
    }

    /// Clear all active grains (used on reset / note start).
    fn clear_grains(&mut self) {
        self.grains = [[Grain::default(); MAX_GRAINS]; NUM_BANDS];
        self.cursor = [0; NUM_BANDS];
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
            .description("CHANT-style granular formant voice (FOF / formant wave functions)")
            .category(ModuleCategory::Oscillator)
            .tag("voice")
            .tag("vocal")
            .tag("formant")
            .tag("fof")
            .tag("chant")
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
            .port(PortDescriptor::audio_output("out", "Out").description("Voice output (mono)"))
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

        // No note playing — output silence.
        if self.note_freq.as_f32() <= 0.0 {
            for i in 0..num_samples {
                self.out_buffer[i] = 0.0;
            }
            self.write_output(outputs);
            return;
        }

        let pitch_cv = inputs.reader(PortName::intern("pitch_cv"), 0.0);
        let vowel_cv = inputs.reader(PortName::intern("vowel_cv"), 0.0);
        let breath_cv = inputs.reader(PortName::intern("breath_cv"), 0.0);
        let pitch_cv_connected = pitch_cv.is_connected();
        let vowel_cv_connected = vowel_cv.is_connected();

        let inv_sr = self.inv_sample_rate;
        let level = self.level.as_f32();
        let base_freq = self.note_freq.as_f32();
        let tex = self.tex_samples();
        let breath_base = self.breathiness.as_f32();
        let vib_depth_cents = self.vibrato_depth.as_f32();
        let vib_inc = self.vibrato_rate.as_f32() * inv_sr;

        if !vowel_cv_connected {
            self.current_vowel = self.vowel.as_f32();
        }
        // Grain targets feed only newly-triggered grains; recompute on vowel
        // change (vowel CV), not every sample.
        let mut targets = self.band_targets(self.current_vowel);
        let mut norm = grain_norm(&targets);
        let out_gain = FOF_GAIN * level;

        // One-pole breath lowpass coefficient (fixed cutoff).
        let breath_lp_coef = crate::math::one_pole_lp_coef(BREATH_LP_FC, inv_sr);

        // Fast path: no pitch CV and no vibrato → a constant trigger increment,
        // so the per-sample pitch transcendentals can be hoisted out entirely.
        let pitch_static = !pitch_cv_connected && vib_depth_cents == 0.0;
        let base_inc = (base_freq * inv_sr).clamp(MIN_TRIGGER_INC, MAX_TRIGGER_INC);

        for i in 0..num_samples {
            // Vowel CV: recompute grain targets (and their norm) on a shift.
            if vowel_cv_connected {
                let target_vowel =
                    (self.vowel.as_f32() + vowel_cv[i] * VOWEL_CV_DEPTH).clamp(0.0, 1.0);
                if (target_vowel - self.current_vowel).abs() > 0.001 {
                    self.current_vowel = target_vowel;
                    targets = self.band_targets(target_vowel);
                    norm = grain_norm(&targets);
                }
            }

            // Pitch: base ± vibrato (cents) ± pitch_cv (semitones). The increment
            // is clamped < 1.0 so at most one grain set fires per sample (a single
            // `-= 1.0` suffices) and a clamped, finite CV keeps `freq` finite.
            let inc = if pitch_static {
                base_inc
            } else {
                let vib_cents = vib_depth_cents * crate::math::fast_sin_turns(self.vibrato_phase);
                let pcv = if pitch_cv_connected {
                    pitch_cv[i].clamp(-MAX_PITCH_CV_SEMITONES, MAX_PITCH_CV_SEMITONES)
                } else {
                    0.0
                };
                let semis = pcv + vib_cents / 100.0;
                let freq = base_freq * crate::math::semitones_to_ratio(semis);
                (freq * inv_sr).clamp(MIN_TRIGGER_INC, MAX_TRIGGER_INC)
            };

            self.trigger_phase += inc;
            if self.trigger_phase >= 1.0 {
                self.trigger_phase -= 1.0;
                self.trigger(&targets, tex);
            }
            self.vibrato_phase = (self.vibrato_phase + vib_inc).fract();

            // Sum all active grains across every band (normalized voiced signal).
            let mut voiced = 0.0_f32;
            for ring in &mut self.grains {
                for grain in ring.iter_mut() {
                    if grain.active {
                        voiced += grain.next_sample();
                    }
                }
            }
            voiced *= norm;

            // Breath / aspiration: lowpass-shaped white noise, mixed by amount.
            // The filter/PRNG run every sample (no state freeze when breath = 0);
            // only the output is gated, so re-opening breath has no discontinuity.
            let breath = (breath_base + breath_cv[i]).clamp(0.0, 1.0);
            let n = crate::math::xorshift_noise(&mut self.rng_state);
            self.breath_lp += breath_lp_coef * (n - self.breath_lp);
            if breath > 0.0 {
                voiced += self.breath_lp * breath * NOISE_GAIN;
            }

            self.out_buffer[i] = crate::math::soft_clip(voiced * out_gain);
        }

        self.write_output(outputs);
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
            Param::Fof(FofParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Fof
    }

    fn reset(&mut self) {
        self.current_vowel = self.vowel.as_f32();
        self.trigger_phase = 0.0;
        self.vibrato_phase = 0.0;
        self.rng_state = RNG_SEED;
        self.breath_lp = 0.0;
        self.clear_grains();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.current_vowel = self.vowel.as_f32();
        // Start at 1.0 so the very first processed sample fires a grain set.
        self.trigger_phase = 1.0;
        self.vibrato_phase = 0.0;
        self.breath_lp = 0.0;
        // Re-seed the breath PRNG so re-triggers are deterministic (mirrors
        // reset()), but mix in the note so cloned/polyphonic voices on different
        // notes start with decorrelated breath rather than identical sequences.
        self.rng_state = (RNG_SEED ^ self.note_freq.as_f32().to_bits()) | 1;
        self.clear_grains();
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
    /// Copy the internal buffer to the `out` port if connected.
    fn write_output(&self, outputs: &mut HashMap<PortName, AudioBuffer>) {
        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.out_buffer);
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
        m
    }

    #[test]
    fn test_fof_creation() {
        let f = Fof::new();
        assert!((f.note_freq.as_f32() - 0.0).abs() < f32::EPSILON);
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
        f.note_on(MidiNote::new(60), Velocity::new(1.0));
        let mut out = outputs(1024);
        f.process(InputPorts::empty(), &mut out, &ctx(1024));

        let buf = &out[&PortName::OUT];
        let max = (0..1024).map(|i| buf[i].abs()).fold(0.0_f32, f32::max);
        assert!(max.is_finite() && max <= 1.5, "Output bounded, max={max}");
    }

    #[test]
    fn test_fof_vowel_morphing() {
        let render = |vowel: f32| {
            let mut f = Fof::new();
            f.set_param(Param::Fof(FofParam::Vowel(NormalizedValue::new(vowel))));
            f.note_on(MidiNote::new(48), Velocity::new(1.0));
            let mut out = outputs(2048);
            f.process(InputPorts::empty(), &mut out, &ctx(2048));
            (0..2048).map(|i| out[&PortName::OUT][i].abs()).sum::<f32>()
        };
        let a = render(0.0);
        let u = render(1.0);
        assert!(
            (a - u).abs() > 0.01,
            "Different vowels should differ: a={a}, u={u}"
        );
    }

    /// Worst case for the grain ring: a high F0 (C6) with the narrowest-band
    /// vowel (U) packs the most overlapping grains. Output must stay finite and
    /// bounded — a regression guard for grain-ring starvation / overwrite clicks.
    #[test]
    fn test_fof_high_pitch_bounded() {
        let mut f = Fof::new();
        f.set_param(Param::Fof(FofParam::Vowel(NormalizedValue::MAX)));
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

        f.set_param(Param::Fof(FofParam::VibratoDepth(Cents::new(40.0))));
        let vd = f
            .get_param(&Param::Fof(FofParam::VibratoDepth(Cents::ZERO)))
            .unwrap_or(0.0);
        assert!((vd - 40.0).abs() < 0.001);
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
        // With vibrato the trigger timing drifts vs a dead-steady reference, so
        // the rendered waveforms diverge in total energy / shape.
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
        // A connected vowel CV must shift the timbre away from the static vowel.
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
}
