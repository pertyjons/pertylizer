//! LA (Linear Arithmetic) Synthesis module.
//!
//! Roland's LA synthesis technique: a short synthesized attack transient
//! (click, noise burst, pluck, or hammer) is crossfaded into a sustained
//! tone from an audio input. This gives natural-sounding attack characteristics
//! to any sustained sound.
//!
//! The module generates the attack transient internally and crossfades to
//! the input signal over a configurable time.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    LaSynthParam, MidiNote, Milliseconds, NormalizedValue, PortName, SampleRate, Velocity,
};
use synth_core::{ModuleType, Param};

/// LA Synthesis voice module.
#[derive(Clone)]
pub struct LaSynth {
    // Parameters
    attack_type: NormalizedValue,
    attack_time: Milliseconds,
    attack_level: NormalizedValue,
    crossfade_time: Milliseconds,
    brightness: NormalizedValue,

    // State
    sample_rate: SampleRate,
    phase: f32,          // attack transient phase (samples since note-on)
    attack_samples: u32, // attack duration in samples
    xfade_samples: u32,  // crossfade duration in samples
    total_samples: u32,  // attack + crossfade duration
    active: bool,        // whether in attack/crossfade phase
    noise_state: u32,    // simple noise PRNG state

    // One-pole lowpass for brightness
    filter_state: f32,
    /// Sample-rate-independent one-pole LP coefficient for the brightness
    /// filter, derived from a cutoff frequency (mapped from `brightness`) and
    /// the current sample rate. Recomputed in `update_times`.
    brightness_coef: f32,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    // Buffers
    output_buffer: AudioBuffer,
}

impl LaSynth {
    pub fn new() -> Self {
        let mut s = Self {
            attack_type: NormalizedValue::MIN,
            attack_time: Milliseconds::new(30.0),
            attack_level: NormalizedValue::new(0.7),
            crossfade_time: Milliseconds::new(50.0),
            brightness: NormalizedValue::new(0.5),
            sample_rate: SampleRate::DVD_QUALITY,
            phase: 0.0,
            attack_samples: 0,
            xfade_samples: 0,
            total_samples: 0,
            active: false,
            noise_state: 0x1234_5678,
            filter_state: 0.0,
            brightness_coef: 0.0,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        };
        s.update_times();
        s
    }

    fn update_times(&mut self) {
        #[allow(clippy::cast_possible_truncation)]
        {
            self.attack_samples = self.attack_time.to_samples(self.sample_rate) as u32;
            self.xfade_samples = self.crossfade_time.to_samples(self.sample_rate) as u32;
        }
        self.total_samples = self.attack_samples + self.xfade_samples;
        if self.total_samples == 0 {
            self.total_samples = 1;
        }
        self.update_brightness_coef();
    }

    /// Recompute the sample-rate-independent brightness one-pole coefficient.
    ///
    /// `brightness` is mapped exponentially to a cutoff frequency, then turned
    /// into a one-pole LP coefficient via `1 - exp(-2π·fc/sr)`. Computing the
    /// coefficient from the actual sample rate keeps the tone character
    /// constant across sample rates (a fixed per-sample alpha would not).
    fn update_brightness_coef(&mut self) {
        let cutoff_hz = crate::math::exponential_frequency_map(
            self.brightness.as_f32().clamp(0.0, 1.0),
            500.0,
            18_000.0,
        );
        let inv_sr = 1.0 / self.sample_rate.as_f32();
        self.brightness_coef = crate::math::one_pole_lp_coef(cutoff_hz, inv_sr);
    }

    /// Generate attack transient sample based on attack_type.
    #[inline]
    fn generate_attack(&mut self) -> f32 {
        let t = self.attack_type.as_f32();
        let level = self.attack_level.as_f32();

        let raw = if t < 0.25 {
            // Click: sharp impulse decaying exponentially
            let phase_norm = self.phase / self.attack_samples.max(1) as f32;
            (-phase_norm * 10.0).exp()
        } else if t < 0.5 {
            // Noise burst: filtered white noise with decay
            let phase_norm = self.phase / self.attack_samples.max(1) as f32;
            let noise = self.next_noise();
            noise * (-phase_norm * 5.0).exp()
        } else if t < 0.75 {
            // Pluck: decaying sine with descending pitch.
            //
            // Instantaneous frequency sweeps linearly from 1200 Hz (at
            // phase_norm=0) down to 800 Hz: f(t) = 800 + 400·(1 - phase_norm).
            // For a swept frequency the phase is the integral 2π·∫₀ᵗ f(τ) dτ,
            // NOT 2π·f(t)·t. For a linear sweep that integral equals
            // 2π·t·f_avg where f_avg = (f_start + f(t)) / 2 — using the
            // instantaneous f(t) directly would double the sweep depth.
            let phase_norm = self.phase / self.attack_samples.max(1) as f32;
            let freq_now = 800.0 + 400.0 * (1.0 - phase_norm); // descending pitch
            let freq_avg = (1200.0 + freq_now) * 0.5; // average over [0, t]
            let t_sec = self.phase / self.sample_rate.as_f32();
            (t_sec * freq_avg * std::f32::consts::TAU).sin() * (-phase_norm * 6.0).exp()
        } else {
            // Hammer: short broadband noise followed by resonant decay
            let phase_norm = self.phase / self.attack_samples.max(1) as f32;
            if phase_norm < 0.2 {
                // Initial impact
                self.next_noise() * (1.0 - phase_norm * 5.0)
            } else {
                // Resonant tail
                let t_sec = self.phase / self.sample_rate.as_f32();
                (t_sec * 300.0 * std::f32::consts::TAU).sin()
                    * (-(phase_norm - 0.2) * 8.0).exp()
                    * 0.5
            }
        };

        // Apply brightness filter (one-pole lowpass). The coefficient is
        // sample-rate-independent (derived from a cutoff frequency and the
        // actual sample rate in `update_brightness_coef`).
        self.filter_state += self.brightness_coef * (raw - self.filter_state);

        self.filter_state * level
    }

    /// Simple noise generator (no allocation).
    #[inline]
    fn next_noise(&mut self) -> f32 {
        // Xorshift32
        self.noise_state ^= self.noise_state << 13;
        self.noise_state ^= self.noise_state >> 17;
        self.noise_state ^= self.noise_state << 5;
        // Map to [-1, 1]
        (self.noise_state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

impl Default for LaSynth {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for LaSynth {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("la_synth", "LA Synth")
            .description(
                "Linear Arithmetic synthesis — attack transient crossfaded to sustain input",
            )
            .category(ModuleCategory::Oscillator)
            .tag("la_synth")
            .tag("synthesis")
            .tag("transient")
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Sustain signal input. Connect: Oscillator Out, Wavetable Out"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out").description(
                    "Output with attack transient. Connect to: Filter In, Amplifier In",
                ),
            )
            .parameter(
                ParameterDescriptor::float(
                    "attack_type",
                    Param::LaSynth(LaSynthParam::AttackType(NormalizedValue::MIN)),
                    "Attack Type",
                )
                .description("0=Click, 0.33=Noise burst, 0.66=Pluck, 1.0=Hammer")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "attack_time",
                    Param::LaSynth(LaSynthParam::AttackTime(Milliseconds::new(30.0))),
                    "Attack Time",
                )
                .description("Duration of attack transient")
                .range(1.0, 200.0)
                .default(30.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "attack_level",
                    Param::LaSynth(LaSynthParam::AttackLevel(NormalizedValue::new(0.7))),
                    "Attack Level",
                )
                .description("Level of attack transient")
                .range(0.0, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "x_fade_time",
                    Param::LaSynth(LaSynthParam::CrossfadeTime(Milliseconds::new(50.0))),
                    "X-Fade Time",
                )
                .description("Crossfade time from attack to sustain")
                .range(1.0, 500.0)
                .default(50.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "brightness",
                    Param::LaSynth(LaSynthParam::Brightness(NormalizedValue::new(0.5))),
                    "Brightness",
                )
                .description("Brightness filter on attack transient")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl PolyModule for LaSynth {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let input = inputs.get(PortName::IN);

        // All five params are read inside generate_attack / the timing math,
        // which run per sample in the loop below, so apply their effective
        // (modulated) values to the fields for the block and restore after. The
        // single loop has no early return. Timing params additionally need
        // update_times() to recompute the cached sample counts.
        let saved = (
            self.attack_type,
            self.attack_time,
            self.attack_level,
            self.crossfade_time,
            self.brightness,
        );
        self.attack_type = NormalizedValue::new(
            self.mod_offsets
                .effective("attack_type", self.attack_type.as_f32()),
        );
        self.attack_time = Milliseconds::new(
            self.mod_offsets
                .effective("attack_time", self.attack_time.as_f32()),
        );
        self.attack_level = NormalizedValue::new(
            self.mod_offsets
                .effective("attack_level", self.attack_level.as_f32()),
        );
        self.crossfade_time = Milliseconds::new(
            self.mod_offsets
                .effective("x_fade_time", self.crossfade_time.as_f32()),
        );
        self.brightness = NormalizedValue::new(
            self.mod_offsets
                .effective("brightness", self.brightness.as_f32()),
        );
        self.update_times();

        for i in 0..num_samples {
            let sustain = input.map_or(0.0, |buf| buf[i]);

            if self.active && (self.phase as u32) < self.total_samples {
                let sample_pos = self.phase as u32;

                if sample_pos < self.attack_samples {
                    // Pure attack phase
                    self.output_buffer[i] = self.generate_attack();
                } else {
                    // Crossfade phase
                    let xfade_pos = sample_pos - self.attack_samples;
                    let xfade_amt = xfade_pos as f32 / self.xfade_samples.max(1) as f32;
                    let xfade_amt = xfade_amt.clamp(0.0, 1.0);

                    let attack = self.generate_attack();
                    // Equal-power crossfade
                    let attack_gain = ((1.0 - xfade_amt) * std::f32::consts::FRAC_PI_2).sin();
                    let sustain_gain = (xfade_amt * std::f32::consts::FRAC_PI_2).sin();
                    self.output_buffer[i] = attack * attack_gain + sustain * sustain_gain;
                }

                self.phase += 1.0;

                if self.phase as u32 >= self.total_samples {
                    self.active = false;
                }
            } else {
                // Past attack/crossfade: pure sustain
                self.output_buffer[i] = sustain;
            }
        }

        // Restore base params + recompute the base sample counts.
        (
            self.attack_type,
            self.attack_time,
            self.attack_level,
            self.crossfade_time,
            self.brightness,
        ) = saved;
        self.update_times();

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::LaSynth(p) = param {
            match p {
                LaSynthParam::AttackType(v) => self.attack_type = v,
                LaSynthParam::AttackTime(v) => {
                    self.attack_time = Milliseconds::new(v.as_f32().clamp(1.0, 200.0));
                    self.update_times();
                }
                LaSynthParam::AttackLevel(v) => self.attack_level = v,
                LaSynthParam::CrossfadeTime(v) => {
                    self.crossfade_time = Milliseconds::new(v.as_f32().clamp(1.0, 500.0));
                    self.update_times();
                }
                LaSynthParam::Brightness(v) => {
                    self.brightness = v;
                    self.update_brightness_coef();
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::LaSynth(p) = param {
            Some(match p {
                LaSynthParam::AttackType(_) => self.attack_type.as_f32(),
                LaSynthParam::AttackTime(_) => self.attack_time.as_f32(),
                LaSynthParam::AttackLevel(_) => self.attack_level.as_f32(),
                LaSynthParam::CrossfadeTime(_) => self.crossfade_time.as_f32(),
                LaSynthParam::Brightness(_) => self.brightness.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::LaSynth(LaSynthParam::AttackType(self.attack_type)),
            Param::LaSynth(LaSynthParam::AttackTime(self.attack_time)),
            Param::LaSynth(LaSynthParam::AttackLevel(self.attack_level)),
            Param::LaSynth(LaSynthParam::CrossfadeTime(self.crossfade_time)),
            Param::LaSynth(LaSynthParam::Brightness(self.brightness)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::LaSynth
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.active = false;
        self.filter_state = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        self.phase = 0.0;
        self.active = true;
        self.filter_state = 0.0;
        self.update_times();
    }

    fn note_off(&mut self) {
        // Attack/crossfade continues until finished; sustain handled by amp envelope
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    #[test]
    fn test_la_synth_creation() {
        let la = LaSynth::new();
        assert_eq!(la.attack_type.as_f32(), 0.0);
        assert_eq!(la.attack_time.as_f32(), 30.0);
    }

    /// `brightness` is a working mod destination via the generic store: it
    /// changes the attack transient's one-pole cutoff. The transiently-applied
    /// fields are restored after the block (no drift).
    #[test]
    fn brightness_mod_offset_changes_attack_and_restores() {
        let mut la = LaSynth::new();
        let desc = la.descriptor();
        la.mod_offsets_mut().unwrap().populate(&desc);

        let ctx = ProcessContext {
            samples: SampleCount::new(64),
            ..ProcessContext::default()
        };
        // Render the attack transient (no sustain input) into a fresh buffer.
        fn attack_wave(la: &mut LaSynth, ctx: &ProcessContext) -> Vec<f32> {
            la.note_on(MidiNote::new(60), Velocity::MAX);
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(64));
            la.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i]).collect()
        }

        let base = attack_wave(&mut la, &ctx);
        let bright_before = la.brightness.as_f32();
        la.set_mod_offset("brightness", -0.4);
        let darker = attack_wave(&mut la, &ctx);
        assert!(
            (la.brightness.as_f32() - bright_before).abs() < 1e-6,
            "brightness field must be restored after process"
        );
        let diff: f32 = base.iter().zip(&darker).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            diff > 1e-3,
            "brightness mod should change the attack, diff = {diff}"
        );

        la.clear_mod_offsets();
        let reverted = attack_wave(&mut la, &ctx);
        let back: f32 = base.iter().zip(&reverted).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            back < 1e-3,
            "clearing reverts brightness, residual = {back}"
        );
    }

    #[test]
    fn test_passthrough_without_note() {
        let mut la = LaSynth::new();
        let num = 64;
        let mut in_buf = AudioBuffer::new(num);
        for i in 0..num {
            in_buf[i] = 0.5;
        }

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(num));

        let context = ProcessContext {
            samples: SampleCount::new(num),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        let input_refs = [(PortName::IN, &in_buf)];
        let inputs = InputPorts::new(&input_refs);
        la.process(inputs, &mut outputs, &context);

        // Without note_on, should pass through sustain
        let out = &outputs[&PortName::OUT];
        for i in 0..num {
            assert!(
                (out[i] - 0.5).abs() < 0.001,
                "Expected passthrough, got {} at {}",
                out[i],
                i
            );
        }
    }

    #[test]
    fn test_la_synth_params() {
        let mut la = LaSynth::new();
        la.set_param(Param::LaSynth(LaSynthParam::AttackLevel(
            NormalizedValue::new(0.9),
        )));
        assert!((la.attack_level.as_f32() - 0.9).abs() < 0.001);

        let params = la.get_params();
        assert_eq!(params.len(), 5);
    }
}
