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
    PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{FofParam, ModuleType, Param};
use synth_core::{MidiNote, NormalizedValue, PortName, SampleRate, Velocity};

use crate::formant_tables::{FORMANT_BW, FORMANT_FREQ, FORMANT_GAIN, NUM_BANDS, NUM_VOWELS};

/// Maximum simultaneously-active grains per formant band (fixed ring — no heap
/// in `process()`).
///
/// Grain lifetime is set by the formant bandwidth (the decay rate `β = π·BW`),
/// *not* by pitch — so the number of overlapping grains is `lifetime × F0` and
/// grows with pitch. The ring must therefore be sized for the *highest* musical
/// F0 with the *narrowest* bandwidth, not the lowest. Worst case: the narrowest
/// vowel band (BW ≈ 50 Hz → lifetime ≈ ln(1/ENV_FLOOR)/(π·BW) ≈ 44 ms at the
/// −60 dB floor) at F0 ≈ 1 kHz (≈ C6, top of the soprano range) → ≈ 44 grains.
/// 64 covers up to ≈ 1.45 kHz. Above that the round-robin overwrites the oldest
/// (most-decayed) grain, so any artifact is bounded by its residual envelope.
const MAX_GRAINS: usize = 64;
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

/// FOF module — CHANT-style granular formant voice (mono, Phase 1).
#[derive(Clone)]
pub struct Fof {
    // Parameters
    vowel: NormalizedValue,
    formant_shift: NormalizedValue,
    skirt: NormalizedValue,
    level: NormalizedValue,

    // Voice state
    note_freq: synth_core::Hertz,
    /// Glottal trigger phase in turns; wrapping past 1.0 fires a new grain set.
    trigger_phase: f32,
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
            level: NormalizedValue::new(0.8),
            note_freq: synth_core::Hertz::ZERO,
            trigger_phase: 0.0,
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

    /// Compute the per-band grain targets for the current vowel + formant shift.
    fn band_targets(&self) -> [BandTarget; NUM_BANDS] {
        let pos = self.vowel.as_f32() * (NUM_VOWELS - 1) as f32;
        let idx = (pos as usize).min(NUM_VOWELS - 2);
        let frac = pos - idx as f32;
        let shift = crate::formant_tables::formant_shift_factor(self.formant_shift.as_f32());
        let sr = self.sample_rate.as_f32();
        let inv_sr = self.inv_sample_rate;

        let mut targets = [BandTarget::default(); NUM_BANDS];
        for (band, target) in targets.iter_mut().enumerate() {
            let freq = FORMANT_FREQ[idx][band] * (1.0 - frac) + FORMANT_FREQ[idx + 1][band] * frac;
            let bw = FORMANT_BW[idx][band] * (1.0 - frac) + FORMANT_BW[idx + 1][band] * frac;
            let gain = FORMANT_GAIN[idx][band] * (1.0 - frac) + FORMANT_GAIN[idx + 1][band] * frac;

            let scaled = (freq * shift).clamp(20.0, sr * 0.45);
            let beta = std::f32::consts::PI * bw.max(10.0);
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
        let pitch_cv_connected = pitch_cv.is_connected();
        let inv_sr = self.inv_sample_rate;
        let level = self.level.as_f32();
        let base_freq = self.note_freq.as_f32();
        let tex = self.tex_samples();
        let targets = self.band_targets();
        let gain_sum: f32 = targets.iter().map(|t| t.amp).sum();
        let norm = if gain_sum > 1.0e-7 {
            1.0 / gain_sum
        } else {
            1.0
        };
        let out_gain = norm * FOF_GAIN * level;
        // Trigger-phase increment when pitch CV is unconnected (hoisted constant).
        let base_inc = (base_freq * inv_sr).clamp(MIN_TRIGGER_INC, MAX_TRIGGER_INC);

        for i in 0..num_samples {
            // Trigger rate follows F0 (+ optional pitch CV in semitones). The
            // increment is clamped < 1.0 so at most one grain set fires per
            // sample (a single `-= 1.0` suffices) and a clamped, finite CV keeps
            // `freq` from overflowing to a non-finite value.
            let inc = if pitch_cv_connected {
                let cv = pitch_cv[i].clamp(-MAX_PITCH_CV_SEMITONES, MAX_PITCH_CV_SEMITONES);
                let freq = base_freq * crate::math::semitones_to_ratio(cv);
                (freq * inv_sr).clamp(MIN_TRIGGER_INC, MAX_TRIGGER_INC)
            } else {
                base_inc
            };
            self.trigger_phase += inc;
            if self.trigger_phase >= 1.0 {
                self.trigger_phase -= 1.0;
                self.trigger(&targets, tex);
            }

            // Sum all active grains across every band.
            let mut sample = 0.0_f32;
            for ring in &mut self.grains {
                for grain in ring.iter_mut() {
                    if grain.active {
                        sample += grain.next_sample();
                    }
                }
            }

            self.out_buffer[i] = crate::math::soft_clip(sample * out_gain);
        }

        self.write_output(outputs);
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Fof(p) = param {
            match p {
                FofParam::Vowel(v) => self.vowel = v,
                FofParam::FormantShift(v) => self.formant_shift = v,
                FofParam::Skirt(v) => self.skirt = v,
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
            Param::Fof(FofParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Fof
    }

    fn reset(&mut self) {
        self.trigger_phase = 0.0;
        self.clear_grains();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        // Start at 1.0 so the very first processed sample fires a grain set.
        self.trigger_phase = 1.0;
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
    }
}
