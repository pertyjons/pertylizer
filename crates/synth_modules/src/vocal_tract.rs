//! Vocal Tract — articulatory Kelly–Lochbaum waveguide voice (Phase 6a).
//!
//! A glottal pulse drives a `KellyLochbaumTract` waveguide whose area profile is
//! shaped by a tongue constriction; the formants fall out of the tube physics
//! rather than from a fixed filter bank. This is the *speech/articulatory*
//! engine — the complementary `VoiceSynth` module is the source–filter
//! singing/choir engine. Pick whichever fits per instrument.
//!
//! See `plans/voice-synth-plan.md` (Phase 6). Phase 6a is a working single-tract
//! prototype (tongue constriction + breath); lips, nasal branch and presets
//! follow in 6b/6c.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity};
use synth_core::{ModuleType, Param, VocalTractParam};
use synth_dsp::KellyLochbaumTract;

/// Number of waveguide sections (≈ vocal-tract discretisation).
const N_SECTIONS: usize = 44;
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
    breathiness: NormalizedValue,
    level: NormalizedValue,

    // Source state
    glottal_phase: Phase,
    prev_flow: f32,
    rng_state: u32,

    // Effective (CV-modulated) tongue position — never written back to the
    // `tongue` knob, so a connected tongue_cv can't drift the stored value.
    current_tongue: f32,

    // Waveguide
    tract: KellyLochbaumTract,

    // Note tracking
    note_freq: Hertz,

    // Cached
    inv_sample_rate: f32,

    // Pre-allocated output buffer
    output_buffer: AudioBuffer,
}

impl VocalTract {
    pub fn new() -> Self {
        let mut v = Self {
            tongue: NormalizedValue::new(0.5),
            constriction: NormalizedValue::new(0.3),
            breathiness: NormalizedValue::new(0.1),
            level: NormalizedValue::new(0.8),
            glottal_phase: Phase::ZERO,
            prev_flow: 0.0,
            rng_state: RNG_SEED,
            current_tongue: 0.5,
            tract: KellyLochbaumTract::new(N_SECTIONS),
            note_freq: Hertz::ZERO,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),
            output_buffer: AudioBuffer::new(1024),
        };
        v.rebuild_profile();
        v
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

    /// Rebuild the tract area profile: a wide rest tube with a tongue
    /// constriction whose position and depth come from the parameters.
    fn rebuild_profile(&mut self) {
        let tongue = self.current_tongue;
        let constriction = self.constriction.as_f32();
        let n = self.tract.sections();
        for i in 0..n {
            let x = i as f32 / (n - 1) as f32;
            let base = 1.5;
            let dist = (x - tongue) * 6.0;
            let bump = constriction * 1.3 * (-(dist * dist)).exp();
            self.tract.set_diameter(i, (base - bump).max(0.05));
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

        let tongue_cv = inputs.reader(PortName::intern("tongue_cv"), 0.0);
        let tongue_cv_connected = tongue_cv.is_connected();

        // Without CV the effective tongue tracks the knob; rebuild for this block.
        if !tongue_cv_connected {
            self.current_tongue = self.tongue.as_f32();
        }
        self.rebuild_profile();

        let inv_sr = self.inv_sample_rate;
        let inc = (self.note_freq.as_f32() * inv_sr).max(1e-5);
        let level = self.level.as_f32();
        let breath = self.breathiness.as_f32();
        let base_tongue = self.tongue.as_f32();

        for i in 0..num_samples {
            // Tongue CV moves the constriction; rebuild only on a real change.
            // Drives `current_tongue`, never the stored `tongue` knob.
            if tongue_cv_connected {
                let target = (base_tongue + tongue_cv[i] * 0.5).clamp(0.0, 1.0);
                if (target - self.current_tongue).abs() > 0.005 {
                    self.current_tongue = target;
                    self.rebuild_profile();
                }
            }

            // Glottal flow derivative (pitch-independent amplitude).
            let flow = glottal_flow(self.glottal_phase.as_f32(), GLOTTAL_OQ);
            let mut excitation = (flow - self.prev_flow) / inc;
            self.prev_flow = flow;

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
            Param::VocalTract(VocalTractParam::Breathiness(self.breathiness)),
            Param::VocalTract(VocalTractParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::VocalTract
    }

    fn reset(&mut self) {
        self.glottal_phase = Phase::ZERO;
        self.prev_flow = 0.0;
        self.rng_state = RNG_SEED;
        self.current_tongue = self.tongue.as_f32();
        self.tract.reset();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.reset();
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
