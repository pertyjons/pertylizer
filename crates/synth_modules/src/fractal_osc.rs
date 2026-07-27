//! Fractal oscillator based on the Weierstrass function.
//!
//! Generates stereo audio through additive synthesis where partial amplitudes
//! and frequencies follow geometric (fractal) scaling:
//!   W(t) = Σ a^n · sin(2π · b^n · freq · t + n · dispersion)
//!
//! Features:
//! - 64 partials with iterative power computation (no `powf` in hot loop)
//! - Anti-aliasing: partials above Nyquist are skipped
//! - Equal-power stereo panning via `spread` parameter
//! - Amplitude normalization for consistent output level
//! - 100% real-time safe: zero heap allocations in `process()`

use std::collections::HashMap;
use synth_core::VoicePitch;
use synth_core::{
    AudioBuffer, BipolarValue, Describable, FractalOscParam, Hertz, InputPorts, MidiNote,
    ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue, Param, ParamModOffsets,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, PortName, PortValueDomain,
    ProcessContext, SampleRate, Seconds, Velocity, WidgetHint,
};

use crate::osc_glide::OscGlide;

/// Maximum number of Weierstrass partials.
const NUM_PARTIALS: usize = 64;

/// Fractal oscillator using Weierstrass-function additive synthesis.
#[derive(Clone)]
pub struct FractalOscillator {
    // Parameters (GUI-facing, normalized 0–1)
    roughness: NormalizedValue,
    fractal_spacing: NormalizedValue,
    dispersion: NormalizedValue,
    spread: NormalizedValue,
    level: NormalizedValue,

    // State
    phases: [f32; NUM_PARTIALS],
    note_freq: Hertz,
    sample_rate: SampleRate,
    inv_sample_rate: f32,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    /// Per-oscillator glide (portamento); `0` glide time = follow the voice glide.
    glide: OscGlide,

    // Pre-allocated output buffers
    output_buffer_left: AudioBuffer,
    output_buffer_right: AudioBuffer,
}

impl FractalOscillator {
    pub fn new() -> Self {
        Self {
            roughness: NormalizedValue::new(0.5),
            fractal_spacing: NormalizedValue::new(0.11),
            dispersion: NormalizedValue::MIN,
            spread: NormalizedValue::new(0.5),
            level: NormalizedValue::MAX,

            phases: [0.0; NUM_PARTIALS],
            note_freq: Hertz::A4,
            sample_rate: SampleRate::DVD_QUALITY,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),
            mod_offsets: ParamModOffsets::new(),
            glide: OscGlide::new(),

            output_buffer_left: AudioBuffer::new(1024),
            output_buffer_right: AudioBuffer::new(1024),
        }
    }

    /// Map normalized roughness (0–1) to clamped `a` range [0.0, 0.99].
    #[inline]
    fn actual_roughness(norm: f32) -> f32 {
        (norm * 0.99).clamp(0.0, 0.99)
    }

    /// Map normalized fractal spacing (0–1) to `b` range [1.01, 10.0].
    #[inline]
    fn actual_spacing(norm: f32) -> f32 {
        (1.01 + norm * (10.0 - 1.01)).clamp(1.01, 10.0)
    }

    /// Generate one stereo sample pair using the Weierstrass additive loop.
    ///
    /// All power computations are iterative (multiply-accumulate) to avoid
    /// expensive `powf` calls in the hot path.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn process_sample(
        &mut self,
        freq: f32,
        a: f32,
        b: f32,
        dispersion: f32,
        nyquist: f32,
        even_pan_l: f32,
        even_pan_r: f32,
        odd_pan_l: f32,
        odd_pan_r: f32,
    ) -> (f32, f32) {
        // Early out: frequency out of valid range
        if freq <= 0.0 || freq >= nyquist {
            return (0.0, 0.0);
        }

        let mut out_l = 0.0_f32;
        let mut out_r = 0.0_f32;
        let mut sum_a = 0.0_f32;

        // Iterative power accumulators: a_pow = a^n, b_pow = b^n
        let mut a_pow = 1.0_f32;
        let mut b_pow = 1.0_f32;

        for n in 0..NUM_PARTIALS {
            // Partial frequency: freq * b^n
            let f_n = freq * b_pow;

            // Anti-aliasing: stop when partial exceeds Nyquist
            if f_n >= nyquist {
                break;
            }

            // Partial amplitude: a^n
            let amp = a_pow;
            sum_a += amp;

            // Weierstrass phase with dispersion offset (in cycles)
            let phase_with_disp = self.phases[n] + (n as f32) * dispersion;

            // Fast sine approximation (max error ~0.001, inaudible)
            let value = crate::math::fast_sin_turns(phase_with_disp);

            // Equal-power stereo panning (pre-computed):
            // Even partials pan left, odd partials pan right
            let (gain_l, gain_r) = if n % 2 == 0 {
                (even_pan_l, even_pan_r)
            } else {
                (odd_pan_l, odd_pan_r)
            };

            out_l += amp * value * gain_l;
            out_r += amp * value * gain_r;

            // Advance phase for this partial: phase += f_n / sample_rate
            self.phases[n] += f_n * self.inv_sample_rate;
            // Wrap phase to [0, 1) — use fract() to handle extreme frequencies
            if self.phases[n] >= 1.0 {
                self.phases[n] = self.phases[n].fract();
            }

            // Step iterative powers
            a_pow *= a;
            b_pow *= b;
        }

        // Normalize by total amplitude to keep consistent output level
        if sum_a > 1e-7 {
            out_l /= sum_a;
            out_r /= sum_a;
        }

        (out_l, out_r)
    }
}

impl Default for FractalOscillator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for FractalOscillator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("fractal_osc", "Fractal Osc")
            .description("Weierstrass fractal oscillator with stereo spread")
            .category(ModuleCategory::Oscillator)
            .tag("fractal")
            .tag("oscillator")
            .tag("additive")
            .tag("stereo")
            .parameter(
                ParameterDescriptor::float(
                    "roughness",
                    Param::FractalOsc(FractalOscParam::Roughness(NormalizedValue::new(0.5))),
                    "Roughness",
                )
                .description("Amplitude scaling per partial (fractal roughness)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "spacing",
                    Param::FractalOsc(FractalOscParam::FractalSpacing(NormalizedValue::new(0.11))),
                    "Spacing",
                )
                .description("Frequency ratio between partials (1.01–10.0)")
                .range(0.0, 1.0)
                .default(0.11)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "dispersion",
                    Param::FractalOsc(FractalOscParam::Dispersion(NormalizedValue::MIN)),
                    "Dispersion",
                )
                .description("Phase offset per partial for crest factor control")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "spread",
                    Param::FractalOsc(FractalOscParam::Spread(NormalizedValue::new(0.5))),
                    "Spread",
                )
                .description("Stereo spread (even partials left, odd right)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::FractalOsc(FractalOscParam::Level(NormalizedValue::MAX)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "glide_time",
                    Param::FractalOsc(FractalOscParam::GlideTime(Seconds::ZERO)),
                    "Glide",
                )
                .description("Per-oscillator portamento time (0 = follow the voice glide)")
                .range(0.0, 2.0)
                .default(0.0)
                .modulatable(false)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("freq_cv", "Freq CV")
                    .value_domain(PortValueDomain::Octaves)
                    .description(
                        "Pitch modulation (1V/oct), clamped to ±1 octave. \
                         Connect: LFO, Envelope",
                    ),
            )
            .port(
                PortDescriptor::audio_output("out_l", "Out L")
                    .description("Left stereo output. Connect to: Amplifier, Filter"),
            )
            .port(
                PortDescriptor::audio_output("out_r", "Out R")
                    .description("Right stereo output. Connect to: Amplifier, Filter"),
            )
    }
}

impl PolyModule for FractalOscillator {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.inv_sample_rate = 1.0 / context.sample_rate.as_f32();
        let num_samples = context.samples.as_usize();
        self.output_buffer_left.resize(num_samples);
        self.output_buffer_right.resize(num_samples);

        // Per-oscillator glide: with glide_time > 0 run our own portamento toward
        // the note target (bend/vibrato on top), overriding the voice glide.
        if let Some(glided) = self.glide.resolve(context.sample_rate, context.samples) {
            self.note_freq = Hertz::new(Hertz::OSC_RANGE.clamp(glided.as_f32()));
        }

        let freq_cv = inputs.get(PortName::FREQ_CV);
        let level = self.mod_offsets.effective("level", self.level.as_f32());
        let nyquist = self.sample_rate.as_f32() * 0.5;

        // Map normalized params (each through its generic mod offset) to DSP ranges
        let a = Self::actual_roughness(
            self.mod_offsets
                .effective("roughness", self.roughness.as_f32()),
        );
        let b = Self::actual_spacing(
            self.mod_offsets
                .effective("spacing", self.fractal_spacing.as_f32()),
        );
        let dispersion = self
            .mod_offsets
            .effective("dispersion", self.dispersion.as_f32())
            .clamp(0.0, 1.0);
        let spread = self
            .mod_offsets
            .effective("spread", self.spread.as_f32())
            .clamp(0.0, 1.0);

        // Pre-compute pan gains for even/odd partials (constant across samples)
        let (even_pan_l, even_pan_r) = crate::math::equal_power_pan(-spread);
        let (odd_pan_l, odd_pan_r) = crate::math::equal_power_pan(spread);

        for i in 0..num_samples {
            // Apply frequency CV (1V/oct)
            let freq = if let Some(cv) = freq_cv {
                self.note_freq
                    .apply_cv(BipolarValue::new(crate::math::sanitize_cv(cv[i])))
                    .as_f32()
            } else {
                self.note_freq.as_f32()
            };

            let (l, r) = self.process_sample(
                freq, a, b, dispersion, nyquist, even_pan_l, even_pan_r, odd_pan_l, odd_pan_r,
            );
            self.output_buffer_left[i] = l * level;
            self.output_buffer_right[i] = r * level;
        }

        if let Some(out_l) = outputs.get_mut(&PortName::OUT_L) {
            out_l.copy_from(&self.output_buffer_left);
        }
        if let Some(out_r) = outputs.get_mut(&PortName::OUT_R) {
            out_r.copy_from(&self.output_buffer_right);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::FractalOsc(p) = param {
            match p {
                FractalOscParam::Roughness(v) => self.roughness = v,
                FractalOscParam::FractalSpacing(v) => self.fractal_spacing = v,
                FractalOscParam::Dispersion(v) => self.dispersion = v,
                FractalOscParam::Spread(v) => self.spread = v,
                FractalOscParam::Level(v) => self.level = v,
                FractalOscParam::GlideTime(t) => self.glide.set_time(t),
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::FractalOsc(p) = param {
            Some(match p {
                FractalOscParam::Roughness(_) => self.roughness.as_f32(),
                FractalOscParam::FractalSpacing(_) => self.fractal_spacing.as_f32(),
                FractalOscParam::Dispersion(_) => self.dispersion.as_f32(),
                FractalOscParam::Spread(_) => self.spread.as_f32(),
                FractalOscParam::Level(_) => self.level.as_f32(),
                FractalOscParam::GlideTime(_) => self.glide.time().as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::FractalOsc(FractalOscParam::Roughness(self.roughness)),
            Param::FractalOsc(FractalOscParam::FractalSpacing(self.fractal_spacing)),
            Param::FractalOsc(FractalOscParam::Dispersion(self.dispersion)),
            Param::FractalOsc(FractalOscParam::Spread(self.spread)),
            Param::FractalOsc(FractalOscParam::Level(self.level)),
            Param::FractalOsc(FractalOscParam::GlideTime(self.glide.time())),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::FractalOsc
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.phases.fill(0.0);
        self.glide.reset();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
    }

    fn set_voice_pitch(&mut self, pitch: VoicePitch) {
        // `process` reads `note_freq` live each sample to scale the fractal
        // generator, so tracking the modulated note pitch is just updating
        // `note_freq`. Phase accumulates continuously — no click.
        self.glide.store(pitch);
        self.note_freq = Hertz::new(Hertz::OSC_RANGE.clamp(pitch.played.as_f32()));
    }

    fn note_off(&mut self) {
        // Nothing to do — envelope controls note release
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

    /// `set_voice_pitch` retunes the fractal generator via `note_freq` (read live
    /// by `process`): 2× voice pitch doubles the rendered fundamental; a static
    /// note holds. Measured at A2.
    #[test]
    fn fractal_osc_tracks_voice_pitch() {
        let sr = SampleRate::DVD_QUALITY;
        let srf = sr.as_f32();
        let note = MidiNote::new(45); // A2 ≈ 110 Hz
        let f = note.to_frequency().as_f32();
        let cents = |est: f32, target: f32| 1200.0 * (est / target).log2();

        // roughness = 0 collapses the Weierstrass sum to the n=0 partial only,
        // i.e. a pure sine at note_freq — cleanly measurable. The fractal upper
        // partials sit at the slightly-inharmonic freq·b^n, which has no clean
        // AMDF fundamental; pitch *tracking* is independent of roughness.
        let pure = || {
            let mut s = FractalOscillator::new();
            s.roughness = NormalizedValue::MIN;
            s.note_on(note, Velocity::MAX);
            s
        };

        let mut s = pure();
        let stat = render_mono(&mut s, sr, 4, 1024, |_| {});
        let est_stat = amdf_fundamental(&stat[2048..], srf, f);
        assert!(cents(est_stat, f).abs() < 50.0, "static {est_stat}");

        let mut s2 = pure();
        let up = render_mono(&mut s2, sr, 4, 1024, |m| {
            m.set_voice_pitch(VoicePitch::tracking(Hertz::new(f * 2.0)));
        });
        let est_up = amdf_fundamental(&up[2048..], srf, f * 2.0);
        assert!(cents(est_up, f * 2.0).abs() < 50.0, "2x {est_up}");
    }

    #[test]
    fn test_fractal_osc_creation() {
        let osc = FractalOscillator::new();
        assert_eq!(osc.note_freq.as_f32(), 440.0);
        assert_eq!(osc.phases, [0.0; NUM_PARTIALS]);
    }

    /// `level` is a working mod destination via the generic store: a negative
    /// offset reduces the output peak, and clearing restores it.
    #[test]
    fn level_mod_offset_scales_output() {
        let mut osc = FractalOscillator::new();
        let desc = osc.descriptor();
        osc.mod_offsets_mut().unwrap().populate(&desc);
        osc.note_on(MidiNote::A4, Velocity::MAX);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            ..ProcessContext::default()
        };
        fn peak(osc: &mut FractalOscillator, ctx: &ProcessContext) -> f32 {
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT_L, AudioBuffer::new(256));
            outs.insert(PortName::OUT_R, AudioBuffer::new(256));
            osc.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT_L];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut osc, &ctx);
        assert!(base > 1e-3, "base output present, got {base}");

        osc.set_mod_offset("level", -0.8);
        let quieter = peak(&mut osc, &ctx);
        assert!(
            quieter < base * 0.5,
            "level offset should reduce output: {quieter} vs {base}"
        );

        osc.clear_mod_offsets();
        assert!((peak(&mut osc, &ctx) - base).abs() < base * 0.1);
    }

    #[test]
    fn test_fractal_osc_produces_stereo_sound() {
        let mut osc = FractalOscillator::new();
        osc.note_on(MidiNote::new(69), Velocity::new(0.8));

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT_L, AudioBuffer::new(64));
        outputs.insert(PortName::OUT_R, AudioBuffer::new(64));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        osc.process(InputPorts::empty(), &mut outputs, &context);

        let out_l = &outputs[&PortName::OUT_L];
        let out_r = &outputs[&PortName::OUT_R];
        let max_l = (0..64).map(|i| out_l[i].abs()).fold(0.0_f32, f32::max);
        let max_r = (0..64).map(|i| out_r[i].abs()).fold(0.0_f32, f32::max);
        assert!(
            max_l > 0.01,
            "Fractal osc should produce left sound, max_l={max_l}"
        );
        assert!(
            max_r > 0.01,
            "Fractal osc should produce right sound, max_r={max_r}"
        );
    }

    #[test]
    fn test_fractal_osc_params() {
        let mut osc = FractalOscillator::new();
        osc.set_param(Param::FractalOsc(FractalOscParam::Roughness(
            NormalizedValue::new(0.8),
        )));
        assert!((osc.roughness.as_f32() - 0.8).abs() < 0.001);
    }

    #[test]
    fn test_fractal_osc_silence_at_nyquist() {
        let mut osc = FractalOscillator::new();
        // Set frequency above Nyquist: 48000/2 = 24000, so 25000 should be silent
        osc.note_freq = Hertz::new(25000.0);

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT_L, AudioBuffer::new(64));
        outputs.insert(PortName::OUT_R, AudioBuffer::new(64));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        osc.process(InputPorts::empty(), &mut outputs, &context);

        let out_l = &outputs[&PortName::OUT_L];
        let max = (0..64).map(|i| out_l[i].abs()).fold(0.0_f32, f32::max);
        assert!(max < 0.001, "Should be silent above Nyquist, but max={max}");
    }

    #[test]
    fn test_fractal_osc_normalization() {
        let mut osc = FractalOscillator::new();
        osc.note_on(MidiNote::new(60), Velocity::new(1.0));
        // High roughness = many active partials, normalization should keep output bounded
        osc.set_param(Param::FractalOsc(FractalOscParam::Roughness(
            NormalizedValue::new(0.95),
        )));

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT_L, AudioBuffer::new(256));
        outputs.insert(PortName::OUT_R, AudioBuffer::new(256));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        osc.process(InputPorts::empty(), &mut outputs, &context);

        let out_l = &outputs[&PortName::OUT_L];
        let max = (0..256).map(|i| out_l[i].abs()).fold(0.0_f32, f32::max);
        assert!(
            max <= 1.01,
            "Normalized output should be <= 1.0, but max={max}"
        );
    }
}
