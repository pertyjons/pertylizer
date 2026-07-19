//! Vocal Tract — articulatory Kelly–Lochbaum waveguide voice (Phase 6c).
//!
//! A glottal pulse drives a `KellyLochbaumTract` waveguide whose area profile is
//! shaped by an articulatory area function; the formants fall out of the tube
//! physics rather than from a fixed filter bank. This is the *speech/
//! articulatory* engine — the complementary `VoiceSynth` module is the
//! source–filter singing/choir engine. Pick whichever fits per instrument.
//!
//! See `plans/voice-synth-plan.md` (Phase 6). Phase 6b builds a full multi-
//! section area function with three independently-controlled regions — a
//! tapered throat at rest, a Gaussian tongue constriction (position + amount),
//! and a lip aperture (rounding) — each addressable from a knob or CV. Phase 6c
//! adds source–tract (glottal) coupling (the glottal-end boundary stiffens as
//! the glottis closes, giving formant ripple a fixed reflector can't) and a
//! nasal side-branch: a velum-coupled nasal cavity opened by `Nasality` for
//! nasal consonants /m n ŋ/ and nasalised vowels. SATB presets follow in 6c.

use std::collections::HashMap;

use synth_core::VoicePitch;
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity};
use synth_core::{ModuleType, Param, VocalTractParam};
use synth_dsp::KellyLochbaumTract;

/// Nominal waveguide length (sections) — the "neutral" tract the length
/// control scales around. Kept at the historical 44 so a default patch (length
/// 0.5) sounds exactly as before the length control existed.
const N_NOMINAL_SECTIONS: usize = 44;
/// Shortest tract (fewest sections → highest formants: soprano / child).
const N_MIN_SECTIONS: usize = 34;
/// Longest tract (most sections → lowest formants: bass). Also the allocated
/// waveguide capacity; the active length never exceeds it.
const N_MAX_SECTIONS: usize = 54;
/// Waveguide steps per audio sample (raises the effective tract rate so the
/// formants land in the speech range).
const TRACT_OVERSAMPLE: usize = 2;
/// Glottal open quotient (fixed in 6a).
const GLOTTAL_OQ: f32 = 0.6;
/// Relative position of the glottal flow peak within the open phase.
const GLOTTAL_TP: f32 = 0.6;
/// Drive into the waveguide, and makeup at the lips.
const GLOTTAL_DRIVE: f32 = 0.06;
const OUTPUT_GAIN: f32 = 6.0;
/// Makeup gain on glottal breath noise.
const NOISE_GAIN: f32 = 1.5;
/// Non-zero xorshift seed.
const RNG_SEED: u32 = 0x9E37_79B9;

// --- Source–tract (glottal) coupling ---
/// Glottal-end reflection during the closed phase (near-rigid wall).
const GLOTTAL_R_CLOSED: f32 = 0.97;
/// Glottal-end reflection at peak opening (lossy coupling to the subglottal
/// tract). Lower than closed → the boundary "opens" as the glottis does.
const GLOTTAL_R_OPEN: f32 = 0.65;

// --- Area-function shape (diameters, arbitrary tract units) ---
/// Rest diameter of the wide oral cavity.
const REST_BASE: f32 = 1.6;
/// Rest diameter at the glottis end of the throat taper.
const REST_THROAT: f32 = 0.7;
/// Fraction of the tract (from the glottis) over which the throat opens up.
const THROAT_FRAC: f32 = 0.18;
/// Spatial width of the tongue constriction (larger = tighter/narrower hump).
const TONGUE_WIDTH: f32 = 6.0;
/// Maximum depth the tongue constriction subtracts from the rest diameter.
const TONGUE_DEPTH: f32 = 1.3;
/// Fraction of the tract (toward the lips) that the lip aperture controls.
const LIP_REGION: f32 = 0.85;
/// Diameter scale at the lips when fully rounded (0 lips param).
const LIP_ROUND: f32 = 0.25;
/// Smallest section diameter, so the tube never closes completely.
const MIN_DIAMETER: f32 = 0.05;
/// CV depth: how far ±full-scale CV moves an articulator (in 0..1 units).
const CV_DEPTH: f32 = 0.5;
/// Articulator change below this (0..1 units) skips a profile rebuild.
const ARTIC_EPS: f32 = 0.005;

// --- Nasal branch ---
/// Number of nasal-cavity waveguide sections.
const NOSE_SECTIONS: usize = 28;
/// Main-tract junction (the velum / soft palate) where the nose taps in, at the
/// nominal tract length.
const VELUM_INDEX: usize = 24;
/// Velar tap as a fraction of the (variable) tract length, so the nose stays at
/// the soft palate as the length control changes (24/44 at the nominal length).
const VELUM_FRAC: f32 = VELUM_INDEX as f32 / N_NOMINAL_SECTIONS as f32;
/// Rest diameter of the nasal cavity at its widest (mid-cavity).
const NOSE_BASE: f32 = 1.6;
/// Nasal diameter at the velar and nostril ends (a gentle taper at both ends
/// gives the cavity its anti-resonant character).
const NOSE_END: f32 = 0.9;

/// Hermite smoothstep on `t`, clamped to [0, 1].
#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Glottal flow over one normalized period `phase` ∈ [0, 1), open for `oq`.
/// Rosenberg-style two-piece pulse (shared shape with `VoiceSynth`).
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

/// Vocal Tract module — glottal source → Kelly–Lochbaum waveguide.
#[derive(Clone)]
pub struct VocalTract {
    // Parameters
    tongue: NormalizedValue,
    constriction: NormalizedValue,
    lips: NormalizedValue,
    length: NormalizedValue,
    nasality: NormalizedValue,
    breathiness: NormalizedValue,
    level: NormalizedValue,

    // Source state
    glottal_phase: Phase,
    prev_flow: f32,
    /// Glottal phase increment, ramped per sample toward `note_freq / sr` so a
    /// block-rate pitch change doesn't step the excitation amplitude (the
    /// excitation divides by it) — see `process`.
    current_inc: f32,
    rng_state: u32,

    // Effective (CV-modulated) articulator values — never written back to the
    // knobs, so connected CVs can't drift the stored values.
    current_tongue: f32,
    current_lips: f32,
    current_nasality: f32,

    // Waveguide
    tract: KellyLochbaumTract,

    // Note tracking
    note_freq: Hertz,

    // Cached
    inv_sample_rate: f32,
    /// Interned CV port names, interned once at construction so `process()`
    /// never has to call `PortName::intern` (which locks the global table).
    tongue_cv_port: PortName,
    lips_cv_port: PortName,
    nasality_cv_port: PortName,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    // Pre-allocated output buffer
    output_buffer: AudioBuffer,
}

impl VocalTract {
    pub fn new() -> Self {
        let mut v = Self {
            tongue: NormalizedValue::new(0.5),
            constriction: NormalizedValue::new(0.3),
            lips: NormalizedValue::new(0.5),
            length: NormalizedValue::new(0.5),
            nasality: NormalizedValue::new(0.0),
            breathiness: NormalizedValue::new(0.1),
            level: NormalizedValue::new(0.8),
            glottal_phase: Phase::ZERO,
            prev_flow: 0.0,
            current_inc: 1e-5,
            rng_state: RNG_SEED,
            current_tongue: 0.5,
            current_lips: 0.5,
            current_nasality: 0.0,
            tract: KellyLochbaumTract::new(N_MAX_SECTIONS),
            note_freq: Hertz::ZERO,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),
            tongue_cv_port: PortName::intern("tongue_cv"),
            lips_cv_port: PortName::intern("lips_cv"),
            nasality_cv_port: PortName::intern("nasality_cv"),
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        };
        v.build_nose();
        v.apply_length();
        v.rebuild_profile();
        v
    }

    /// Map the `length` knob (0..1) to an active section count in
    /// `[N_MIN_SECTIONS, N_MAX_SECTIONS]` — 0.5 lands on the nominal length.
    fn length_to_sections(&self) -> usize {
        let t = self.length.as_f32().clamp(0.0, 1.0);
        let span = (N_MAX_SECTIONS - N_MIN_SECTIONS) as f32;
        (N_MIN_SECTIONS as f32 + span * t).round() as usize
    }

    /// Apply the current `length` to the waveguide: set the active section count
    /// (the tract length, which scales all formants) and keep the velar tap at
    /// the soft palate. Cheap and real-time safe — no allocation.
    fn apply_length(&mut self) {
        let sections = self.length_to_sections();
        self.tract.set_active_len(sections);
        let velum = (sections as f32 * VELUM_FRAC).round() as usize;
        self.tract.set_velum_index(velum);
    }

    /// Attach the nasal cavity and give it a fixed tapered area profile. The
    /// branch stays sealed (velum closed) until `Nasality` opens it.
    fn build_nose(&mut self) {
        self.tract.enable_nose(NOSE_SECTIONS, VELUM_INDEX);
        for i in 0..NOSE_SECTIONS {
            let x = i as f32 / (NOSE_SECTIONS - 1) as f32;
            // Widest mid-cavity, tapering toward the velar and nostril ends.
            let taper = 1.0 - (2.0 * x - 1.0) * (2.0 * x - 1.0);
            self.tract
                .set_nose_diameter(i, NOSE_END + (NOSE_BASE - NOSE_END) * taper);
        }
        self.tract.update_nose_reflections();
        self.tract.set_velum(0.0);
    }

    /// One white-noise sample in [-1, 1] via xorshift32.
    #[inline]
    fn next_noise(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Rebuild the tract area profile from the three articulatory regions:
    /// a tapered throat at rest, a Gaussian tongue constriction (position +
    /// amount), and a lip aperture (rounding) at the mouth end.
    fn rebuild_profile(&mut self) {
        let tongue = self.current_tongue;
        let constriction = self
            .mod_offsets
            .effective("constriction", self.constriction.as_f32());
        // Lip aperture: 1 = spread (no narrowing), 0 = fully rounded.
        let lip_scale = LIP_ROUND + (1.0 - LIP_ROUND) * self.current_lips;
        let n = self.tract.active_len();
        for i in 0..n {
            let x = i as f32 / (n - 1) as f32;

            // Rest area: narrow throat near the glottis opening into the cavity.
            let rest = REST_THROAT + (REST_BASE - REST_THROAT) * smoothstep(x / THROAT_FRAC);

            // Tongue constriction: a Gaussian dip at the tongue position.
            let dist = (x - tongue) * TONGUE_WIDTH;
            let dip = constriction * TONGUE_DEPTH * (-(dist * dist)).exp();

            // Lip aperture: scale the mouth-end sections down when rounded.
            let lip_weight = smoothstep((x - LIP_REGION) / (1.0 - LIP_REGION));
            let d = (rest - dip) * (1.0 - lip_weight * (1.0 - lip_scale));

            self.tract.set_diameter(i, d.max(MIN_DIAMETER));
        }
        self.tract.update_reflections();
    }
}

impl Default for VocalTract {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for VocalTract {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("vocal_tract", "Vocal Tract")
            .description("Articulatory Kelly–Lochbaum waveguide voice (speech-grade)")
            .category(ModuleCategory::Oscillator)
            .tag("voice")
            .tag("vocal")
            .tag("waveguide")
            .tag("physical")
            .parameter(
                ParameterDescriptor::float(
                    "tongue",
                    Param::VocalTract(VocalTractParam::Tongue(NormalizedValue::new(0.5))),
                    "Tongue",
                )
                .description("Constriction position (0 = throat, 1 = lips)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "constriction",
                    Param::VocalTract(VocalTractParam::Constriction(NormalizedValue::new(0.3))),
                    "Constriction",
                )
                .description("Constriction amount (0 = open tube, 1 = tight)")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "lips",
                    Param::VocalTract(VocalTractParam::Lips(NormalizedValue::new(0.5))),
                    "Lips",
                )
                .description("Lip aperture (0 = rounded /o u/, 1 = spread /i e/)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "length",
                    Param::VocalTract(VocalTractParam::Length(NormalizedValue::new(0.5))),
                    "Length",
                )
                .description(
                    "Tract length / voice type (0 = short → soprano/child, \
                     0.5 = neutral, 1 = long → bass). Scales all formants.",
                )
                .range(0.0, 1.0)
                .default(0.5)
                // Structural: selects a discrete waveguide section count
                // (voice type), so it is not a continuous modulation target.
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "nasality",
                    Param::VocalTract(VocalTractParam::Nasality(NormalizedValue::new(0.0))),
                    "Nasality",
                )
                .description("Velar port opening (0 = oral, 1 = nasal /m n ŋ/)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "breathiness",
                    Param::VocalTract(VocalTractParam::Breathiness(NormalizedValue::new(0.1))),
                    "Breathiness",
                )
                .description("Aspiration / breath noise at the glottis")
                .range(0.0, 1.0)
                .default(0.1)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::VocalTract(VocalTractParam::Level(NormalizedValue::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("tongue_cv", "Tongue CV")
                    .description("Modulate constriction position. Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::control_input("lips_cv", "Lips CV")
                    .description("Modulate lip aperture / rounding. Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::control_input("nasality_cv", "Nasality CV")
                    .description("Modulate the velar (nasal) opening. Connect: LFO, Envelope"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Vocal tract output"))
    }
}

impl PolyModule for VocalTract {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
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

        let tongue_cv = inputs.reader(self.tongue_cv_port, 0.0);
        let lips_cv = inputs.reader(self.lips_cv_port, 0.0);
        let nasality_cv = inputs.reader(self.nasality_cv_port, 0.0);
        let tongue_cv_connected = tongue_cv.is_connected();
        let lips_cv_connected = lips_cv.is_connected();
        let nasality_cv_connected = nasality_cv.is_connected();

        // Generic mod-matrix offsets — articulator bases resolved once here.
        let tongue_base = self.mod_offsets.effective("tongue", self.tongue.as_f32());
        let lips_base = self.mod_offsets.effective("lips", self.lips.as_f32());
        let nasality_base = self
            .mod_offsets
            .effective("nasality", self.nasality.as_f32());

        // Without CV an articulator tracks its knob; rebuild once for this block.
        if !tongue_cv_connected {
            self.current_tongue = tongue_base;
        }
        if !lips_cv_connected {
            self.current_lips = lips_base;
        }
        if !nasality_cv_connected {
            self.current_nasality = nasality_base;
        }
        // Tract length is a structural per-voice control (no CV): apply it once
        // per block before rebuilding the area profile over the active length.
        self.apply_length();
        self.rebuild_profile();
        self.tract.set_velum(self.current_nasality);

        let inv_sr = self.inv_sample_rate;
        // Ramp the glottal increment across the block toward `note_freq/sr`
        // instead of snapping it: the excitation is `(flow - prev_flow)/inc`, so
        // a stepped `inc` under fast pitch modulation steps the excitation
        // amplitude — an audible click. One add/sample (below) smooths it.
        let target_inc = (self.note_freq.as_f32() * inv_sr).max(1e-5);
        // First block after note_on (sentinel `current_inc == 0`): snap exactly to
        // the rate-correct increment so the note starts on-pitch with no stale-rate
        // glide. `current_inc` is otherwise always ≥ 1e-5 (see the per-sample
        // `.max(1e-5)` below), so this only ever fires right after note_on.
        if self.current_inc <= 0.0 {
            self.current_inc = target_inc;
        }
        let d_inc = if num_samples > 0 {
            (target_inc - self.current_inc) / num_samples as f32
        } else {
            0.0
        };
        let level = self.mod_offsets.effective("level", self.level.as_f32());
        let breath = self
            .mod_offsets
            .effective("breathiness", self.breathiness.as_f32());
        let base_tongue = tongue_base;
        let base_lips = lips_base;
        let base_nasality = nasality_base;

        for i in 0..num_samples {
            // CVs move the articulators; rebuild only on a real change, and
            // drive the transient `current_*` values, never the stored knobs.
            if tongue_cv_connected || lips_cv_connected {
                let mut dirty = false;
                if tongue_cv_connected {
                    // `InputReader::get` sanitizes the raw CV (NaN/inf → 0); then clamp
                    // into the Kelly–Lochbaum-valid articulator range [0, 1].
                    let cv = tongue_cv.get(i);
                    let target = (base_tongue + cv * CV_DEPTH).clamp(0.0, 1.0);
                    if (target - self.current_tongue).abs() > ARTIC_EPS {
                        self.current_tongue = target;
                        dirty = true;
                    }
                }
                if lips_cv_connected {
                    let cv = lips_cv.get(i);
                    let target = (base_lips + cv * CV_DEPTH).clamp(0.0, 1.0);
                    if (target - self.current_lips).abs() > ARTIC_EPS {
                        self.current_lips = target;
                        dirty = true;
                    }
                }
                if dirty {
                    self.rebuild_profile();
                }
            }

            // The velum opening is cheap to set (no profile rebuild), so a
            // connected CV can drive it every sample.
            if nasality_cv_connected {
                let cv = nasality_cv.get(i);
                self.current_nasality = (base_nasality + cv * CV_DEPTH).clamp(0.0, 1.0);
                self.tract.set_velum(self.current_nasality);
            }

            // Per-sample glottal increment (ramped, not snapped — see above).
            self.current_inc = (self.current_inc + d_inc).max(1e-5);
            let inc = self.current_inc;

            // Glottal flow derivative (pitch-independent amplitude).
            let flow = glottal_flow(self.glottal_phase.as_f32(), GLOTTAL_OQ);
            let mut excitation = (flow - self.prev_flow) / inc;
            self.prev_flow = flow;

            // Source–tract coupling: the glottis is a near-rigid wall when shut
            // and a lossy opening to the subglottal tract when open, so its
            // reflection tracks the instantaneous opening (flow ∈ [0, 1]).
            let openness = flow.clamp(0.0, 1.0);
            self.tract.set_glottal_reflection(
                GLOTTAL_R_CLOSED + (GLOTTAL_R_OPEN - GLOTTAL_R_CLOSED) * openness,
            );

            if breath > 0.0 {
                excitation += self.next_noise() * breath * NOISE_GAIN;
            }

            // Drive the waveguide (oversampled) and read the radiated lip output.
            let drive = excitation * GLOTTAL_DRIVE;
            let mut lip = 0.0_f32;
            for _ in 0..TRACT_OVERSAMPLE {
                lip += self.tract.step(drive);
            }
            lip /= TRACT_OVERSAMPLE as f32;

            self.output_buffer[i] = crate::math::soft_clip(lip * OUTPUT_GAIN * level);

            self.glottal_phase = Phase::new_unchecked((self.glottal_phase.as_f32() + inc).fract());
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::VocalTract(p) = param {
            match p {
                VocalTractParam::Tongue(v) => {
                    self.tongue = v;
                    self.current_tongue = v.as_f32();
                }
                VocalTractParam::Constriction(v) => self.constriction = v,
                VocalTractParam::Lips(v) => {
                    self.lips = v;
                    self.current_lips = v.as_f32();
                }
                VocalTractParam::Length(v) => self.length = v,
                VocalTractParam::Nasality(v) => {
                    self.nasality = v;
                    self.current_nasality = v.as_f32();
                }
                VocalTractParam::Breathiness(v) => self.breathiness = v,
                VocalTractParam::Level(v) => self.level = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::VocalTract(p) = param {
            Some(match p {
                VocalTractParam::Tongue(_) => self.tongue.as_f32(),
                VocalTractParam::Constriction(_) => self.constriction.as_f32(),
                VocalTractParam::Lips(_) => self.lips.as_f32(),
                VocalTractParam::Length(_) => self.length.as_f32(),
                VocalTractParam::Nasality(_) => self.nasality.as_f32(),
                VocalTractParam::Breathiness(_) => self.breathiness.as_f32(),
                VocalTractParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::VocalTract(VocalTractParam::Tongue(self.tongue)),
            Param::VocalTract(VocalTractParam::Constriction(self.constriction)),
            Param::VocalTract(VocalTractParam::Lips(self.lips)),
            Param::VocalTract(VocalTractParam::Length(self.length)),
            Param::VocalTract(VocalTractParam::Nasality(self.nasality)),
            Param::VocalTract(VocalTractParam::Breathiness(self.breathiness)),
            Param::VocalTract(VocalTractParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::VocalTract
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.glottal_phase = Phase::ZERO;
        self.prev_flow = 0.0;
        self.rng_state = RNG_SEED;
        self.current_tongue = self.tongue.as_f32();
        self.current_lips = self.lips.as_f32();
        self.current_nasality = self.nasality.as_f32();
        self.tract.reset();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.reset();
        // Sentinel: the first `process()` block snaps `current_inc` exactly to the
        // rate-correct `note_freq/sr` (see process). Seeding here would bake a
        // possibly-stale `inv_sample_rate` — the engine only delivers the render
        // rate via `ProcessContext`, so a note triggered before the first process()
        // at a non-default rate would start the glottal increment at the wrong rate
        // (wrong excitation amplitude / a pop). A real increment is always
        // `freq/sr ≥ 1e-5 > 0`, so 0.0 unambiguously means "not yet seeded".
        self.current_inc = 0.0;
    }

    fn set_voice_pitch(&mut self, pitch: VoicePitch) {
        // Track the modulated note pitch (glide / vibrato / bend). `process`
        // ramps `current_inc` toward `note_freq/sr` across the block, so a
        // block-rate change here doesn't click.
        self.note_freq = Hertz::new(Hertz::OSC_RANGE.clamp(pitch.played.as_f32()));
    }

    fn note_off(&mut self) {
        // Amplitude envelope is handled by the host patch (Envelope → Amplifier).
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
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

    /// `set_voice_pitch` retunes the glottal source via `note_freq` (ramped into
    /// `current_inc` each block): 2× voice pitch doubles the rendered
    /// fundamental; a static note holds. Measured at A2 (formant-rich output).
    #[test]
    fn vocal_tract_tracks_voice_pitch() {
        let sr = SampleRate::DVD_QUALITY;
        let srf = sr.as_f32();
        let note = MidiNote::new(45); // A2 ≈ 110 Hz
        let f = note.to_frequency().as_f32();
        let cents = |est: f32, target: f32| 1200.0 * (est / target).log2();

        let mut s = VocalTract::new();
        s.note_on(note, Velocity::MAX);
        let stat = render_mono(&mut s, sr, 8, 1024, |_| {});
        let est_stat = amdf_fundamental(&stat[4096..], srf, f);
        assert!(cents(est_stat, f).abs() < 50.0, "static {est_stat}");

        let mut s2 = VocalTract::new();
        s2.note_on(note, Velocity::MAX);
        let up = render_mono(&mut s2, sr, 8, 1024, |m| {
            m.set_voice_pitch(VoicePitch::tracking(Hertz::new(f * 2.0)));
        });
        let est_up = amdf_fundamental(&up[4096..], srf, f * 2.0);
        assert!(cents(est_up, f * 2.0).abs() < 50.0, "2x {est_up}");
    }

    fn ctx(n: usize) -> ProcessContext<'static> {
        ProcessContext {
            samples: synth_core::SampleCount::new(n),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        }
    }

    fn render(setup: impl FnOnce(&mut VocalTract), n: usize) -> Vec<f32> {
        let mut v = VocalTract::new();
        setup(&mut v);
        v.note_on(MidiNote::new(50), Velocity::new(1.0));
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(n));
        v.process(InputPorts::empty(), &mut outputs, &ctx(n));
        (0..n).map(|i| outputs[&PortName::OUT][i]).collect()
    }

    #[test]
    fn test_vocal_tract_produces_sound() {
        let out = render(|_| {}, 2048);
        let max = out.iter().fold(0.0_f32, |m, &x| m.max(x.abs()));
        assert!(max > 0.01, "Vocal tract should produce sound, max={max}");
        assert!(max.is_finite() && max <= 1.5, "Output bounded, max={max}");
    }

    #[test]
    fn test_vocal_tract_silent_without_note() {
        let mut v = VocalTract::new();
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));
        v.process(InputPorts::empty(), &mut outputs, &ctx(64));
        let out = &outputs[&PortName::OUT];
        let max = (0..64).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max < 0.001, "Should be silent without note_on, max={max}");
    }

    /// Regression: the glottal increment must track the *render* sample rate, not
    /// the (stale) rate cached when `note_on` fired. Construct at the 48 kHz default,
    /// `note_on`, then render one block at 96 kHz: the increment snaps exactly to
    /// `note_freq / 96000`, not `note_freq / 48000` (the old stale-rate seed → pop).
    #[test]
    fn glottal_increment_follows_render_rate_not_note_on_rate() {
        let mut v = VocalTract::new();
        let note = MidiNote::new(57); // A3 ≈ 220 Hz
        v.note_on(note, Velocity::MAX);
        let f = note.to_frequency().as_f32();

        let n = 256;
        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(n));
        let ctx96 = ProcessContext {
            samples: synth_core::SampleCount::new(n),
            sample_rate: SampleRate::HIGH_RES,
            ..ProcessContext::default()
        };
        v.process(InputPorts::empty(), &mut outputs, &ctx96);

        let expected = f / SampleRate::HIGH_RES.as_f32();
        let wrong = f / SampleRate::DVD_QUALITY.as_f32();
        assert!(
            (v.current_inc - expected).abs() < 1e-7,
            "glottal increment {} should track render rate ({expected}), not note_on rate ({wrong})",
            v.current_inc
        );
        let b = &outputs[&PortName::OUT];
        assert!((0..n).all(|i| b[i].is_finite()), "output must stay finite");
    }

    #[test]
    fn test_vocal_tract_articulation_changes_output() {
        // Front vs back tongue position should produce different formants.
        let front: f32 = render(
            |v| {
                v.set_param(Param::VocalTract(VocalTractParam::Tongue(
                    NormalizedValue::new(0.8),
                )))
            },
            4096,
        )
        .iter()
        .map(|x| x.abs())
        .sum();
        let back: f32 = render(
            |v| {
                v.set_param(Param::VocalTract(VocalTractParam::Tongue(
                    NormalizedValue::new(0.2),
                )))
            },
            4096,
        )
        .iter()
        .map(|x| x.abs())
        .sum();
        assert!(
            (front - back).abs() > 0.01,
            "Tongue position should change output: front={front}, back={back}"
        );
    }

    #[test]
    fn test_vocal_tract_lip_rounding_changes_output() {
        // Rounded vs spread lips reshape the mouth aperture → different formants.
        let energy = |lips: f32| -> f32 {
            render(
                |v| {
                    v.set_param(Param::VocalTract(VocalTractParam::Lips(
                        NormalizedValue::new(lips),
                    )))
                },
                4096,
            )
            .iter()
            .map(|x| x.abs())
            .sum()
        };
        let rounded = energy(0.0);
        let spread = energy(1.0);
        assert!(
            (rounded - spread).abs() > 0.01,
            "Lip rounding should change output: rounded={rounded}, spread={spread}"
        );
    }

    #[test]
    fn test_vocal_tract_nasality_changes_output() {
        // Opening the velar port couples the nasal cavity → different timbre,
        // and the output must stay bounded.
        let energy = |nasality: f32| -> f32 {
            let out = render(
                |v| {
                    v.set_param(Param::VocalTract(VocalTractParam::Nasality(
                        NormalizedValue::new(nasality),
                    )))
                },
                4096,
            );
            let max = out.iter().fold(0.0_f32, |m, &x| m.max(x.abs()));
            assert!(
                max.is_finite() && max <= 1.5,
                "nasal output unbounded: {max}"
            );
            out.iter().map(|x| x.abs()).sum()
        };
        let oral = energy(0.0);
        let nasal = energy(1.0);
        assert!(
            (oral - nasal).abs() > 0.01,
            "Nasality should change output: oral={oral}, nasal={nasal}"
        );
    }

    #[test]
    fn test_vocal_tract_length_changes_output_and_active_len() {
        // A short tract (soprano/child) vs a long one (bass) changes the
        // formants, and the waveguide's active length reflects the control.
        let energy = |length: f32| -> f32 {
            let out = render(
                |v| {
                    v.set_param(Param::VocalTract(VocalTractParam::Length(
                        NormalizedValue::new(length),
                    )))
                },
                4096,
            );
            let max = out.iter().fold(0.0_f32, |m, &x| m.max(x.abs()));
            assert!(
                max.is_finite() && max <= 1.5,
                "length output unbounded: {max}"
            );
            out.iter().map(|x| x.abs()).sum()
        };
        let short = energy(0.0);
        let long = energy(1.0);
        assert!(
            (short - long).abs() > 0.01,
            "Tract length should change output: short={short}, long={long}"
        );

        // The mapping: 0 → shortest, 0.5 → nominal, 1 → longest.
        let active = |length: f32| {
            let mut v = VocalTract::new();
            v.set_param(Param::VocalTract(VocalTractParam::Length(
                NormalizedValue::new(length),
            )));
            v.apply_length();
            v.tract.active_len()
        };
        assert_eq!(active(0.0), N_MIN_SECTIONS);
        assert_eq!(active(0.5), N_NOMINAL_SECTIONS);
        assert_eq!(active(1.0), N_MAX_SECTIONS);
    }

    /// `level` used to be dropped (VocalTract never overrode set_mod_offset); it
    /// now scales the output through the generic store. Driving it to 0 silences
    /// the voice; clearing reverts.
    #[test]
    fn test_vocal_tract_level_mod_offset_scales_output() {
        let energy = |v: &mut VocalTract| {
            let mut outputs = HashMap::new();
            outputs.insert(PortName::OUT, AudioBuffer::new(2048));
            v.process(InputPorts::empty(), &mut outputs, &ctx(2048));
            (0..2048)
                .map(|i| outputs[&PortName::OUT][i].abs())
                .sum::<f32>()
        };

        let mut v = VocalTract::new();
        let desc = v.descriptor();
        v.mod_offsets_mut().unwrap().populate(&desc);

        v.note_on(MidiNote::new(50), Velocity::new(1.0));
        let base = energy(&mut v);
        assert!(base > 0.01, "baseline should produce sound, base={base}");

        v.set_mod_offset("level", -1.0); // level → 0 = silence
        v.note_on(MidiNote::new(50), Velocity::new(1.0));
        let quiet = energy(&mut v);
        assert!(
            quiet < base * 0.01,
            "level offset should silence output: base={base}, quiet={quiet}"
        );

        v.clear_mod_offsets();
        v.note_on(MidiNote::new(50), Velocity::new(1.0));
        let restored = energy(&mut v);
        assert!(
            (restored - base).abs() < base * 0.1,
            "clearing reverts level: base={base}, restored={restored}"
        );
    }

    #[test]
    fn test_vocal_tract_params() {
        let mut v = VocalTract::new();
        v.set_param(Param::VocalTract(VocalTractParam::Constriction(
            NormalizedValue::new(0.7),
        )));
        let got = v
            .get_param(&Param::VocalTract(VocalTractParam::Constriction(
                NormalizedValue::MIN,
            )))
            .unwrap_or(0.0);
        assert!((got - 0.7).abs() < 0.001);
    }
}
