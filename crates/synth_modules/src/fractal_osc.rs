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
use std::f32::consts::{FRAC_PI_2, TAU};

use synth_core::{
    AudioBuffer, Describable, FractalOscParam, Hertz, InputPorts, MidiNote, ModuleCategory,
    ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor, PolyModule,
    PortDescriptor, PortName, ProcessContext, SampleRate, Velocity, WidgetHint,
};

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

            output_buffer_left: AudioBuffer::new(1024),
            output_buffer_right: AudioBuffer::new(1024),
        }
    }

    /// Map normalized roughness (0–1) to clamped `a` range [0.0, 0.99].
    #[inline]
    fn actual_roughness(&self) -> f32 {
        (self.roughness.as_f32() * 0.99).clamp(0.0, 0.99)
    }

    /// Map normalized fractal spacing (0–1) to `b` range [1.01, 10.0].
    #[inline]
    fn actual_spacing(&self) -> f32 {
        let t = self.fractal_spacing.as_f32();
        (1.01 + t * (10.0 - 1.01)).clamp(1.01, 10.0)
    }

    /// Generate one stereo sample pair using the Weierstrass additive loop.
    ///
    /// All power computations are iterative (multiply-accumulate) to avoid
    /// expensive `powf` calls in the hot path.
    #[inline]
    fn process_sample(
        &mut self,
        freq: f32,
        a: f32,
        b: f32,
        dispersion: f32,
        spread: f32,
        nyquist: f32,
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

            // Convert to radians and compute sine
            let value = (phase_with_disp * TAU).sin();

            // Equal-power stereo panning:
            // Even partials pan left (-spread), odd partials pan right (+spread)
            let pan = if n % 2 == 0 { -spread } else { spread };
            let angle = ((pan + 1.0) * 0.5) * FRAC_PI_2;
            let gain_l = angle.cos();
            let gain_r = angle.sin();

            out_l += amp * value * gain_l;
            out_r += amp * value * gain_r;

            // Advance phase for this partial: phase += f_n / sample_rate
            self.phases[n] += f_n * self.inv_sample_rate;
            // Wrap phase to [0, 1) — single subtract is sufficient for audio rates
            if self.phases[n] >= 1.0 {
                self.phases[n] -= 1.0;
            }

            // Step iterative powers
            a_pow *= a;
            b_pow *= b;
        }

        // Normalize by total amplitude to keep consistent output level
        if sum_a > 0.0 {
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
                    Param::FractalOsc(FractalOscParam::Level(NormalizedValue::MAX)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("freq_cv", "Freq CV")
                    .description("Pitch modulation (1V/oct). Connect: LFO, Envelope"),
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

        let freq_cv = inputs.get(PortName::FREQ_CV);
        let level = self.level.as_f32();
        let nyquist = self.sample_rate.as_f32() * 0.5;

        // Map normalized params to DSP ranges
        let a = self.actual_roughness();
        let b = self.actual_spacing();
        let dispersion = self.dispersion.as_f32().clamp(0.0, 1.0);
        let spread = self.spread.as_f32().clamp(0.0, 1.0);

        let base_freq = self.note_freq.as_f32();

        for i in 0..num_samples {
            // Apply frequency CV (1V/oct)
            let freq = if let Some(cv) = freq_cv {
                base_freq * (2.0_f32).powf(cv[i])
            } else {
                base_freq
            };

            let (l, r) = self.process_sample(freq, a, b, dispersion, spread, nyquist);
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
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::FractalOsc
    }

    fn reset(&mut self) {
        self.phases.fill(0.0);
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
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

    #[test]
    fn test_fractal_osc_creation() {
        let osc = FractalOscillator::new();
        assert_eq!(osc.note_freq.as_f32(), 440.0);
        assert_eq!(osc.phases, [0.0; NUM_PARTIALS]);
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
