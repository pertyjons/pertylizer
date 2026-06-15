//! Math Oscillator module with 19 different mathematical algorithms.
//!
//! Features:
//! - 18 different synthesis algorithms from FM to chaos
//! - Phase-based, iterative/chaotic, and buffer-based algorithms
//! - Three general-purpose parameters (A, B, C) per algorithm
//! - Internal state for chaos attractors and physical modeling

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve,
    WidgetHint,
};
use synth_core::{
    BipolarValue, BufferIndex, FrameCount, Gain, Hertz, MidiNote, NormalizedValue, Phase, PortName,
    SampleRate, Velocity,
};
use synth_core::{MathAlgo, MathOscillatorParam, ModuleType, Param};

/// Maximum delay line size for Karplus-Strong (enough for ~20Hz at 48kHz)
const MAX_DELAY_SIZE: usize = 4800;

/// A math-based oscillator with 19 synthesis algorithms.
#[derive(Clone)]
pub struct MathOscillator {
    // Parameters
    algo: MathAlgo,
    frequency: Hertz,
    /// Base frequency before FM modulation (set by note_on or set_param).
    base_frequency: Hertz,
    var_a: NormalizedValue,
    var_b: NormalizedValue,
    var_c: NormalizedValue,
    level: Gain,

    // Standard state
    phase: Phase,
    sample_rate: SampleRate,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    // Chaos/Lorenz state (raw f32 - mathematical state, not audio domain)
    lorenz_x: f32,
    lorenz_y: f32,
    lorenz_z: f32,

    // Feedback/Logistic state
    last_sample: f32,
    logistic_x: f32,

    // Karplus-Strong state
    delay_line: Vec<f32>,
    write_pos: BufferIndex,
    burst_remaining: FrameCount,

    // Bytebeat state
    time_counter: u32,

    // Shepard-tone per-layer phase accumulators
    shepard_phases: [f32; 6],

    // Output buffer
    output_buffer: AudioBuffer,
}

impl MathOscillator {
    pub fn new() -> Self {
        Self {
            algo: MathAlgo::SineFM,
            frequency: Hertz::A4,
            base_frequency: Hertz::A4,
            var_a: NormalizedValue::CENTER,
            var_b: NormalizedValue::CENTER,
            var_c: NormalizedValue::CENTER,
            level: Gain::UNITY,
            phase: Phase::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),
            lorenz_x: 0.1,
            lorenz_y: 0.0,
            lorenz_z: 0.0,
            last_sample: 0.0,
            logistic_x: 0.5,
            delay_line: vec![0.0; MAX_DELAY_SIZE],
            write_pos: BufferIndex::ZERO,
            burst_remaining: FrameCount::ZERO,
            time_counter: 0,
            shepard_phases: [0.0; 6],
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Set frequency from MIDI note.
    pub fn set_note(&mut self, note: MidiNote) {
        self.frequency = note.to_frequency();
        self.base_frequency = self.frequency;
    }

    /// Generate white noise sample using fastrand (thread-local, lock-free).
    #[inline]
    fn noise(&self) -> f32 {
        // Convert [0, 1) to [-1, 1)
        fastrand::f32() * 2.0 - 1.0
    }

    /// Convenience for tests/internal callers: generate using the current
    /// stored `var_a/b/c` (no mod offset, no CV).
    #[cfg(test)]
    fn generate_sample_from_vars(&mut self) -> f32 {
        let (a, b, c) = (
            self.var_a.as_f32(),
            self.var_b.as_f32(),
            self.var_c.as_f32(),
        );
        self.generate_sample(a, b, c)
    }

    /// Generate a sample using the current algorithm.
    #[inline]
    #[allow(clippy::many_single_char_names)] // Mathematical variables: t, a, b, c
    fn generate_sample(&mut self, a: f32, b: f32, c: f32) -> f32 {
        let t = self.phase.as_f32();
        let dt = self.frequency.phase_increment(self.sample_rate);

        let sample = match self.algo {
            // ================================================================
            // CATEGORY A: Phase-based (Stateless)
            // ================================================================
            MathAlgo::SineFM => {
                // Basic FM synthesis: carrier modulated by modulator
                let mod_index = a * 5.0;
                let mod_ratio = 1.0 + b * 7.0;
                (TAU * t + mod_index * (TAU * t * mod_ratio).sin()).sin()
            }

            MathAlgo::TanChaos => {
                // Tan distortion with noise
                let tan_freq = 1.0 + b * 3.0;
                let tan_val = (TAU * t * tan_freq).tan().clamp(-10.0, 10.0) * 0.1;
                let noise_amt = c * self.noise() * 0.3;
                (tan_val * a + noise_amt).clamp(-1.0, 1.0)
            }

            MathAlgo::SuperSaw => {
                // Multiple detuned sawtooth waves
                let spread = a * 0.05;
                let num_saws = 3 + (b * 4.0) as i32;
                let mut sum = 0.0;
                for i in 0..num_saws {
                    let detune = (i as f32 - (num_saws as f32 - 1.0) / 2.0) * spread;
                    let saw_phase = (t + detune).rem_euclid(1.0);
                    sum += 2.0 * saw_phase - 1.0;
                }
                sum / num_saws as f32
            }

            MathAlgo::BitWise => {
                // Digital glitch / bytebeat style
                let t_int = (t * 256.0 * (1.0 + b * 15.0)) as u32;
                let shift_a = (a * 8.0) as u32 + 1;
                let shift_b = (c * 8.0) as u32 + 1;
                let val = (t_int.wrapping_mul(t_int >> shift_a | t_int >> shift_b)) & 255;
                (val as f32 / 128.0) - 1.0
            }

            MathAlgo::WaveFolder => {
                // West coast style wave folding
                let input = (TAU * t).sin();
                let fold_amt = 1.0 + a * 8.0;
                let offset = b * 2.0 - 1.0;
                ((input + offset) * fold_amt).sin()
            }

            MathAlgo::Formant => {
                // FOF-style vocal formant grain. The bare `sin·exp(-d·t)`
                // form aliases badly at typical decay rates: at t=1 the
                // envelope is still ~exp(-d) and jumps to 1.0 at the next
                // phase wrap, producing a slope discontinuity that bleeds
                // up to Nyquist, plus a strong DC bias from the asymmetric
                // first positive lobe. Three shaping terms fix both:
                // half-cosine attack and release ramps that force the
                // grain to start and end at zero with zero slope, and an
                // exponential decay normalized to reach exactly 0 at t=1.
                let formant_freq = 1.0 + a * 20.0;
                let decay_rate = 1.0 + b * 15.0;
                let attack_len = 0.1;
                let release_start = 0.9;
                let attack = if t < attack_len {
                    0.5 - 0.5 * (PI * t / attack_len).cos()
                } else {
                    1.0
                };
                let release = if t > release_start {
                    let r = (t - release_start) / (1.0 - release_start);
                    0.5 + 0.5 * (PI * r).cos()
                } else {
                    1.0
                };
                let raw_env = (-t * decay_rate).exp();
                let end_env = (-decay_rate).exp();
                let env = ((raw_env - end_env) / (1.0 - end_env)).max(0.0);
                let carrier = (TAU * t * formant_freq).sin();
                carrier * env * attack * release
            }

            MathAlgo::PhaseDist => {
                // Casio CZ style phase distortion
                let dist_amt = 0.1 + a * 0.9;
                let bent_phase = if t < 0.5 {
                    (t / 0.5).powf(dist_amt) * 0.5
                } else {
                    1.0 - ((1.0 - t) / 0.5).powf(dist_amt) * 0.5
                };
                (TAU * bent_phase).sin()
            }

            MathAlgo::Metallic => {
                // Ring modulation for metallic tones
                let ratio = 1.0 + a * 10.0;
                let mix = b;
                let carrier = (TAU * t).sin();
                let modulator = (TAU * t * ratio).sin();
                carrier * (1.0 - mix) + carrier * modulator * mix
            }

            MathAlgo::Fractal => {
                // Weierstrass-like fractal function
                let num_harmonics = 3 + (a * 5.0) as i32;
                let base = 2.0 + b * 1.5;
                let mut sum = 0.0;
                let mut amp = 1.0;
                let mut freq = 1.0;
                for _ in 0..num_harmonics {
                    sum += amp * (TAU * t * freq).cos();
                    amp *= 0.5;
                    freq *= base;
                }
                sum * 0.5
            }

            MathAlgo::Chebyshev => {
                // Chebyshev polynomial waveshaping
                let order = 2.0 + a * 8.0;
                let input = (TAU * t).sin();
                // Chebyshev polynomial: cos(n * acos(x))
                (order * input.clamp(-1.0, 1.0).acos()).cos()
            }

            MathAlgo::Walsh => {
                // Walsh function synthesis (sum of square waves with binary periods)
                let num_terms = 2 + (a * 6.0) as i32;
                let mut sum = 0.0;
                let mut normalization = 0.0;
                for i in 0..num_terms {
                    let period = 1 << i;
                    let walsh_t = (t * period as f32).floor() as i32;
                    let sign = if walsh_t % 2 == 0 { 1.0 } else { -1.0 };
                    let weight = 1.0 / (i + 1) as f32;
                    sum += sign * weight;
                    normalization += weight;
                }
                sum / normalization * b * 2.0
            }

            MathAlgo::Pulsar => {
                // Pulsar synthesis (windowed sine bursts)
                let duty = 0.1 + a * 0.4;
                let window_shape = b;
                if t < duty {
                    let local_t = t / duty;
                    let window = if window_shape < 0.5 {
                        // Hann window
                        0.5 * (1.0 - (TAU * local_t).cos())
                    } else {
                        // Gaussian-ish
                        let x = local_t * 2.0 - 1.0;
                        (-x * x * 4.0).exp()
                    };
                    (TAU * local_t).sin() * window
                } else {
                    0.0
                }
            }

            MathAlgo::Shepard => {
                // Shepard tone — six logarithmically-spaced sine layers spanning
                // six octaves. Each layer carries its own phase accumulator so the
                // sines integrate at their true frequency (the parent `t` is one
                // wrapping phase ramp and cannot drive multiple frequencies at
                // once without emitting the partial-cycle DC bias / clicks the
                // earlier implementation produced).
                //
                // - param_a (`a`): sweep position in [0, 1). External modulation
                //   advances it; the algorithm wraps via `rem_euclid` so any
                //   sawtooth/LFO range produces a continuous rise (or fall).
                // - param_b (`b`): Gaussian envelope width (0.15..0.65,
                //   default 0.4 at b=0.5). Centered at `pos = 0.5` so the bottom
                //   and top registers fade out near the wrap boundaries —
                //   that's what makes the rise sound continuous rather than
                //   discontinuous when a register snaps from top to bottom.
                const NUM_LAYERS: usize = 6;
                let sweep = a.rem_euclid(1.0);
                let center = 0.5;
                let width = 0.15 + b * 0.5;
                let two_w_sq = 2.0 * width * width;
                let base_freq = self.frequency.as_f32();
                let inv_sr = 1.0 / self.sample_rate.as_f32();
                let mut sum = 0.0;
                let mut weight = 0.0;
                for i in 0..NUM_LAYERS {
                    let layer_offset = i as f32 / NUM_LAYERS as f32;
                    let pos = (layer_offset + sweep).rem_euclid(1.0);
                    let centered = pos - center;
                    let amp = (-(centered * centered) / two_w_sq).exp();
                    let freq_mult = 2.0f32.powf(pos * NUM_LAYERS as f32 - 3.0);
                    let phase_inc = base_freq * freq_mult * inv_sr;
                    self.shepard_phases[i] = (self.shepard_phases[i] + phase_inc).rem_euclid(1.0);
                    sum += amp * (TAU * self.shepard_phases[i]).sin();
                    weight += amp;
                }
                if weight > 1e-6 { sum / weight } else { 0.0 }
            }

            // ================================================================
            // CATEGORY B: Iterative/Chaotic (Stateful)
            // ================================================================
            MathAlgo::Bytebeat => {
                // Classic bytebeat: t * ((t>>A) | (t>>B))
                let t = self.time_counter;
                self.time_counter = self.time_counter.wrapping_add(1);
                let shift_a = (a * 12.0) as u32 + 1;
                let shift_b = (b * 12.0) as u32 + 1;
                let val = t.wrapping_mul((t >> shift_a) | (t >> shift_b)) & 255;
                (val as f32 / 128.0) - 1.0
            }

            MathAlgo::Lorenz => {
                // Lorenz attractor chaos
                let sigma = 10.0;
                let rho = 28.0;
                let beta = 8.0 / 3.0;
                let speed = 0.001 + a * 0.01;
                let dt_chaos = speed;

                let dx = sigma * (self.lorenz_y - self.lorenz_x);
                let dy = self.lorenz_x * (rho - self.lorenz_z) - self.lorenz_y;
                let dz = self.lorenz_x * self.lorenz_y - beta * self.lorenz_z;

                self.lorenz_x += dx * dt_chaos;
                self.lorenz_y += dy * dt_chaos;
                self.lorenz_z += dz * dt_chaos;

                // Clamp Lorenz values to prevent divergence
                self.lorenz_x = self.lorenz_x.clamp(-50.0, 50.0);
                self.lorenz_y = self.lorenz_y.clamp(-50.0, 50.0);
                self.lorenz_z = self.lorenz_z.clamp(0.0, 100.0);

                // Normalize output (Lorenz x typically ranges -20 to 20)
                (self.lorenz_x * 0.05 * b).clamp(-1.0, 1.0)
            }

            MathAlgo::Logistic => {
                // Logistic map chaos
                let r = 3.0 + a; // r from 3.0 to 4.0 (chaotic region)
                self.logistic_x = (r * self.logistic_x * (1.0 - self.logistic_x)).clamp(0.0, 1.0);
                // Center and scale output
                (self.logistic_x * 2.0 - 1.0) * b
            }

            MathAlgo::FeedbackFM => {
                // Self-modulating FM
                let feedback_amt = a * 3.0;
                let sample = (TAU * t + self.last_sample * feedback_amt).sin();
                self.last_sample = sample;
                sample
            }

            MathAlgo::Vosim => {
                // VOSIM: Voice simulation via squared sine pulses
                // A = formant freq multiplier (1-20x), B = decay per pulse, C = number of pulses
                let num_pulses = 1 + (c * 5.0) as i32; // 1-6 pulses
                let formant = 1.0 + a * 19.0; // formant 1x-20x base freq
                let decay = 0.3 + b * 0.69; // decay 0.3-0.99

                // Each pulse occupies 1/num_pulses of the period
                let pulse_width = 1.0 / num_pulses as f32;
                let pulse_index = (t / pulse_width) as i32;
                let local_t = (t / pulse_width).fract();

                if pulse_index < num_pulses {
                    // Squared sine pulse with exponential decay
                    let sin_val = (TAU * local_t * formant).sin();
                    let envelope = decay.powi(pulse_index);
                    sin_val * sin_val * envelope
                } else {
                    0.0
                }
            }

            // ================================================================
            // CATEGORY C: Buffer-based
            // ================================================================
            MathAlgo::KarplusStrong => {
                // Karplus-Strong physical modeling
                let delay_samples =
                    (self.sample_rate.as_f32() / self.frequency.as_f32().max(1.0)).round() as usize;
                let delay_samples = delay_samples.clamp(1, MAX_DELAY_SIZE - 1);

                // Read from delay line
                let read_pos = self.write_pos.delay_read(delay_samples, MAX_DELAY_SIZE);
                let val = self.delay_line[read_pos.as_usize()];

                // Low-pass filter (averaging)
                let prev_pos = read_pos.delay_read(1, MAX_DELAY_SIZE);
                let damping = 0.9 + a * 0.09; // 0.9 to 0.99
                let filtered = (val + self.delay_line[prev_pos.as_usize()]) * 0.5 * damping;

                // Add burst noise if triggered
                let burst_noise = if self.burst_remaining.as_usize() > 0 {
                    self.burst_remaining = FrameCount::new(self.burst_remaining.as_usize() - 1);
                    self.noise() * b
                } else {
                    0.0
                };

                // Write to delay line
                let output = filtered + burst_noise;
                self.delay_line[self.write_pos.as_usize()] = output;
                self.write_pos = self.write_pos.advance(MAX_DELAY_SIZE);

                output
            }
        };

        // Advance phase
        self.phase = self.phase.advance(dt);

        // Final clamp (level applied by the caller after modulation).
        sample.clamp(-1.0, 1.0)
    }

    /// Reset chaos/iteration state for deterministic note starts.
    fn reset_state(&mut self) {
        self.phase = Phase::ZERO;
        self.lorenz_x = 0.1;
        self.lorenz_y = 0.0;
        self.lorenz_z = 0.0;
        self.logistic_x = 0.5;
        self.last_sample = 0.0;
        self.time_counter = 0;
        self.shepard_phases = [0.0; 6];
    }

    /// Trigger a burst for Karplus-Strong.
    fn trigger_burst(&mut self) {
        let delay_samples = (self.sample_rate.as_f32() / self.frequency.as_f32()).round() as usize;
        self.burst_remaining = FrameCount::new(delay_samples.min(MAX_DELAY_SIZE));
        // Clear delay line for clean start
        self.delay_line.fill(0.0);
        self.write_pos = BufferIndex::ZERO;
    }
}

impl Default for MathOscillator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for MathOscillator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("math_oscillator", "Math Oscillator")
            .description("Advanced oscillator with 19 mathematical synthesis algorithms")
            .category(ModuleCategory::Oscillator)
            .tag("oscillator")
            .tag("math")
            .tag("chaos")
            .tag("fm")
            // Parameters
            .parameter(
                ParameterDescriptor::choice(
                    "algorithm",
                    Param::MathOscillator(MathOscillatorParam::Algorithm(MathAlgo::SineFM)),
                    "Algorithm",
                    MathAlgo::to_choices(),
                )
                .description("Synthesis algorithm")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    "frequency",
                    Param::MathOscillator(MathOscillatorParam::Frequency(Hertz::A4)),
                    "Frequency",
                )
                .description("Base frequency")
                .range(20.0, 20000.0)
                .default(440.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::FrequencySlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "param_a",
                    Param::MathOscillator(MathOscillatorParam::ParamA(NormalizedValue::CENTER)),
                    "Param A",
                )
                .description("Algorithm parameter A")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "param_b",
                    Param::MathOscillator(MathOscillatorParam::ParamB(NormalizedValue::CENTER)),
                    "Param B",
                )
                .description("Algorithm parameter B")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "param_c",
                    Param::MathOscillator(MathOscillatorParam::ParamC(NormalizedValue::CENTER)),
                    "Param C",
                )
                .description("Algorithm parameter C")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::MathOscillator(MathOscillatorParam::Level(Gain::UNITY)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            // Ports
            .port(
                PortDescriptor::control_input("fm", "FM")
                    .description("Modulates pitch. Connect: LFO, Envelope, another Oscillator"),
            )
            .port(
                PortDescriptor::control_input("param_a", "Mod A")
                    .description("Modulates Param A. Connect: LFO, Envelope, Kinetic Modulator"),
            )
            .port(
                PortDescriptor::control_input("param_b", "Mod B")
                    .description("Modulates Param B. Connect: LFO, Envelope, Kinetic Modulator"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Audio output. Connect to: Amplifier In, Filter In"),
            )
    }
}

impl PolyModule for MathOscillator {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        // Get modulation inputs
        let fm_input = inputs.get(PortName::FM);
        let mod_a = inputs.get(PortName::PARAM_A);
        let mod_b = inputs.get(PortName::PARAM_B);

        // Generic mod-matrix offsets, resolved once per block. frequency folds
        // into the FM base; param_a/b/c form the base the per-sample Mod A/B CV
        // adds on top of; level scales the output.
        let eff_base_freq = Hertz::new(
            self.mod_offsets
                .effective("frequency", self.base_frequency.as_f32()),
        );
        let base_a = self.mod_offsets.effective("param_a", self.var_a.as_f32());
        let base_b = self.mod_offsets.effective("param_b", self.var_b.as_f32());
        let c = self.mod_offsets.effective("param_c", self.var_c.as_f32());
        let level = self.mod_offsets.effective("level", self.level.as_f32());

        for i in 0..context.samples.as_usize() {
            // Apply FM relative to the (modulated) base frequency.
            self.frequency = if let Some(fm) = fm_input {
                eff_base_freq.apply_fm(BipolarValue::new(fm[i]))
            } else {
                eff_base_freq
            };

            // Per-sample Mod A/B CV adds on top of the modulated param base.
            let a = mod_a.map_or(base_a, |ma| (base_a + ma[i] * 0.5).clamp(0.0, 1.0));
            let b = mod_b.map_or(base_b, |mb| (base_b + mb[i] * 0.5).clamp(0.0, 1.0));

            self.output_buffer[i] = self.generate_sample(a, b, c) * level;
        }

        // Copy to output
        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::MathOscillator(math_param) = param {
            match math_param {
                MathOscillatorParam::Algorithm(algo) => {
                    self.algo = algo;
                    self.reset_state();
                }
                MathOscillatorParam::Frequency(f) => {
                    self.frequency = Hertz::new(f.as_f32().clamp(20.0, 20000.0));
                    self.base_frequency = self.frequency;
                }
                MathOscillatorParam::ParamA(v) => self.var_a = v,
                MathOscillatorParam::ParamB(v) => self.var_b = v,
                MathOscillatorParam::ParamC(v) => self.var_c = v,
                MathOscillatorParam::Level(l) => self.level = l,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::MathOscillator(math_param) = param {
            Some(match math_param {
                MathOscillatorParam::Algorithm(_) => self.algo.index() as f32,
                MathOscillatorParam::Frequency(_) => self.frequency.as_f32(),
                MathOscillatorParam::ParamA(_) => self.var_a.as_f32(),
                MathOscillatorParam::ParamB(_) => self.var_b.as_f32(),
                MathOscillatorParam::ParamC(_) => self.var_c.as_f32(),
                MathOscillatorParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::MathOscillator(MathOscillatorParam::Algorithm(self.algo)),
            Param::MathOscillator(MathOscillatorParam::Frequency(self.frequency)),
            Param::MathOscillator(MathOscillatorParam::ParamA(self.var_a)),
            Param::MathOscillator(MathOscillatorParam::ParamB(self.var_b)),
            Param::MathOscillator(MathOscillatorParam::ParamC(self.var_c)),
            Param::MathOscillator(MathOscillatorParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::MathOscillator
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.reset_state();
        self.delay_line.fill(0.0);
        self.write_pos = BufferIndex::ZERO;
        self.burst_remaining = FrameCount::ZERO;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.set_note(note);
        self.reset_state();

        // For Karplus-Strong, trigger a burst
        if self.algo == MathAlgo::KarplusStrong {
            self.trigger_burst();
        }
    }

    fn note_off(&mut self) {
        // Most algorithms don't need special handling
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_math_oscillator_creation() {
        let osc = MathOscillator::new();
        assert_eq!(osc.algo, MathAlgo::SineFM);
        assert!((osc.frequency.as_f32() - 440.0).abs() < 0.001);
    }

    /// `level` is a working mod destination: a negative offset reduces the
    /// rendered output peak, and clearing restores it.
    #[test]
    fn level_mod_offset_scales_output() {
        let mut osc = MathOscillator::new();
        let desc = osc.descriptor();
        osc.mod_offsets_mut().unwrap().populate(&desc);
        osc.note_on(MidiNote::A4, Velocity::MAX);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            ..ProcessContext::default()
        };
        fn peak(osc: &mut MathOscillator, ctx: &ProcessContext) -> f32 {
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            osc.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut osc, &ctx);
        assert!(base > 0.01, "base output present, got {base}");

        osc.set_mod_offset("level", -0.8);
        let quieter = peak(&mut osc, &ctx);
        assert!(
            quieter < base * 0.5,
            "level mod should reduce output: {quieter} vs {base}"
        );

        osc.clear_mod_offsets();
        assert!((peak(&mut osc, &ctx) - base).abs() < base * 0.1);
    }

    #[test]
    fn test_all_algorithms_produce_output() {
        let mut osc = MathOscillator::new();
        osc.sample_rate = SampleRate::DVD_QUALITY;

        for algo in MathAlgo::ALL {
            osc.algo = algo;
            osc.reset_state();

            // For Karplus-Strong, we need to trigger a burst
            if algo == MathAlgo::KarplusStrong {
                osc.trigger_burst();
            }

            // Generate 100 samples and check they're in range
            let mut has_nonzero = false;
            for _ in 0..100 {
                let sample = osc.generate_sample_from_vars();
                assert!(
                    (-2.0..=2.0).contains(&sample),
                    "Algorithm {:?} produced out-of-range sample: {}",
                    algo,
                    sample
                );
                if sample.abs() > 0.001 {
                    has_nonzero = true;
                }
            }

            // Most algorithms should produce non-zero output
            // (Pulsar might be zero if phase is in silent region)
            if algo != MathAlgo::Pulsar {
                assert!(has_nonzero, "Algorithm {:?} produced only silence", algo);
            }
        }
    }

    #[test]
    fn test_note_to_frequency() {
        let mut osc = MathOscillator::new();
        osc.set_note(MidiNote::A4); // A4
        assert!((osc.frequency.as_f32() - 440.0).abs() < 0.001);

        osc.set_note(MidiNote::C4); // C4
        assert!((osc.frequency.as_f32() - 261.63).abs() < 1.0);
    }

    /// Render N samples of the current configuration, returning peak / RMS / mean.
    fn render_metrics(osc: &mut MathOscillator, n: usize) -> (f32, f32, f32) {
        let mut peak: f32 = 0.0;
        let mut sum_sq = 0.0_f64;
        let mut sum = 0.0_f64;
        for _ in 0..n {
            let s = osc.generate_sample_from_vars();
            peak = peak.max(s.abs());
            sum_sq += (s as f64) * (s as f64);
            sum += s as f64;
        }
        let rms = (sum_sq / n as f64).sqrt() as f32;
        let mean = (sum / n as f64) as f32;
        (peak, rms, mean)
    }

    #[test]
    fn shepard_dc_low_across_sweep() {
        // The previous broken implementation leaked +0.06 of DC at param_a = 0
        // (lowest register dominated by a partial-sine ramp). With per-layer
        // phase accumulators the long-term mean of every sweep position must
        // sit comfortably below the analyzer's 0.01 has_dc_offset threshold.
        let mut osc = MathOscillator::new();
        osc.algo = MathAlgo::Shepard;
        osc.sample_rate = SampleRate::DVD_QUALITY;
        osc.set_note(MidiNote::new(36));

        // Three seconds — same window analyze_note uses for the audit.
        let samples = osc.sample_rate.as_f32() as usize * 3;

        for sweep_norm in [0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            osc.reset_state();
            osc.var_a = NormalizedValue::new(sweep_norm);
            osc.var_b = NormalizedValue::CENTER; // default width
            let (peak, _rms, mean) = render_metrics(&mut osc, samples);
            assert!(
                mean.abs() < 0.005,
                "Shepard DC at param_a={sweep_norm} too high: {mean}"
            );
            assert!(
                (0.05..=1.0).contains(&peak),
                "Shepard peak at param_a={sweep_norm} out of audible range: {peak}"
            );
        }
    }

    #[test]
    fn shepard_layer_phase_increments_track_sweep() {
        // Hallmark of a real Shepard: each layer integrates at its own
        // frequency, and the layer-to-frequency assignment shifts with
        // sweep. The broken implementation reused the parent oscillator's
        // phase for every layer, so the per-layer phase increments were
        // effectively constant in sweep. With the fix, advancing sweep by
        // 1/6 cyclically permutes the layer frequencies — so the *set* of
        // observed phase increments stays the same, but the assignment to
        // layer index i changes.
        let mut osc = MathOscillator::new();
        osc.algo = MathAlgo::Shepard;
        osc.sample_rate = SampleRate::DVD_QUALITY;
        osc.set_note(MidiNote::new(36));

        fn observe_increments(osc: &mut MathOscillator, sweep: f32) -> [f32; 6] {
            osc.reset_state();
            osc.var_a = NormalizedValue::new(sweep);
            osc.var_b = NormalizedValue::CENTER;
            // After one sample each layer's phase has advanced by exactly
            // one phase increment (initial phases are 0).
            let _ = osc.generate_sample_from_vars();
            osc.shepard_phases
        }

        let inc_a = observe_increments(&mut osc, 0.0);
        let inc_b = observe_increments(&mut osc, 1.0 / 6.0);

        // Layer i at sweep=1/6 sits at register pos=(i+1)/6 mod 1 → its
        // increment must match layer (i+1) % 6 at sweep=0 (which is at the
        // same register pos). Confirms the cyclic register-shift property
        // and rules out a regression where layer frequencies were tied to
        // layer index alone.
        for i in 0..6 {
            let mapped = (i + 1) % 6;
            let diff = (inc_b[i] - inc_a[mapped]).abs();
            assert!(
                diff < 1e-6,
                "layer {i} at sweep=1/6 should match layer {mapped} at sweep=0: {} vs {}",
                inc_b[i],
                inc_a[mapped],
            );
        }

        // And at least one layer increment must differ between sweep=0 and
        // some intermediate sweep position (e.g. 1/12) — proves the layers
        // aren't pinned to fixed frequencies.
        let inc_c = observe_increments(&mut osc, 1.0 / 12.0);
        let mut max_diff = 0.0_f32;
        for i in 0..6 {
            max_diff = max_diff.max((inc_c[i] - inc_a[i]).abs());
        }
        assert!(
            max_diff > 1e-5,
            "layer increments are sweep-insensitive (broken-algo regression)"
        );
    }

    #[test]
    fn formant_grain_is_dc_free_and_bounded() {
        // The bare `sin·exp(-d·t)` grain produced ~0.30 DC and a slope
        // discontinuity at phase wrap that aliased up to Nyquist. After
        // adding the half-cosine attack and the end-normalized envelope,
        // a long render at the example-patch defaults (a=0.5, b=0.4) must
        // sit below the analyzer's 0.005 has_dc_offset threshold and stay
        // strictly inside ±1.
        let mut osc = MathOscillator::new();
        osc.algo = MathAlgo::Formant;
        osc.sample_rate = SampleRate::DVD_QUALITY;
        osc.var_a = NormalizedValue::new(0.5);
        osc.var_b = NormalizedValue::new(0.4);
        osc.var_c = NormalizedValue::new(0.5);

        for &note in &[57_u8, 60, 69] {
            osc.reset_state();
            osc.set_note(MidiNote::new(note));
            let samples = osc.sample_rate.as_f32() as usize;
            let (peak, _rms, mean) = render_metrics(&mut osc, samples);
            assert!(
                mean.abs() < 0.005,
                "Formant DC at note={note} too high: {mean}"
            );
            assert!(peak <= 1.0, "Formant peak at note={note} clips: {peak}");
            assert!(peak > 0.05, "Formant peak at note={note} too quiet: {peak}");
        }
    }

    #[test]
    fn shepard_finite_and_bounded() {
        // Sweep param_a across the full unit interval at every sample for an
        // extended render and confirm we never produce non-finite or
        // out-of-range samples (regression for partial-cycle DC clamping).
        let mut osc = MathOscillator::new();
        osc.algo = MathAlgo::Shepard;
        osc.sample_rate = SampleRate::DVD_QUALITY;
        osc.set_note(MidiNote::new(36));
        osc.reset_state();

        let sr = osc.sample_rate.as_f32() as usize;
        for s in 0..(sr * 2) {
            osc.var_a = NormalizedValue::new((s as f32 / sr as f32).rem_euclid(1.0));
            osc.var_b = NormalizedValue::new(((s as f32 / sr as f32) * 0.5 + 0.25).clamp(0.0, 1.0));
            let v = osc.generate_sample_from_vars();
            assert!(v.is_finite(), "non-finite at s={s}");
            assert!(v.abs() <= 1.0, "out of range at s={s}: {v}");
        }
    }
}
