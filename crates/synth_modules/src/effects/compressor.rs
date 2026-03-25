//! Dynamics compressor with adjustable threshold, ratio, attack, and release.

use synth_core::{
    AudioEffect, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor, ParameterUnit,
    PortDescriptor, ProcessContext, StereoSample, WidgetHint,
};
use synth_core::{CompressorParam, ModuleType, Param};
use synth_core::{Decibels, Hertz, Milliseconds, NormalizedValue, Ratio, SampleRate};

/// Compressor effect with envelope follower and optional sidechain.
pub struct Compressor {
    // Parameters
    threshold: Decibels,
    ratio: Ratio,
    attack: Milliseconds,
    release: Milliseconds,
    makeup: Decibels,
    mix: NormalizedValue,

    // Sidechain
    sidechain_enabled: bool,
    sidechain_filter_freq: Hertz,
    /// One-pole HPF state for sidechain filter.
    sc_filter_state: f32,

    // Sidechain input buffer (pre-allocated, filled externally before process)
    sidechain_buffer: Vec<f32>,
    sidechain_len: usize,

    // Envelope state
    envelope: f32,

    // State
    sample_rate: SampleRate,
}

impl Compressor {
    pub fn new() -> Self {
        Self {
            threshold: Decibels::new(-20.0),
            ratio: Ratio::MEDIUM,
            attack: Milliseconds::new(10.0),
            release: Milliseconds::new(100.0),
            makeup: Decibels::new(0.0),
            mix: NormalizedValue::MAX,
            sidechain_enabled: false,
            sidechain_filter_freq: Hertz::new(80.0),
            sc_filter_state: 0.0,
            sidechain_buffer: vec![0.0; 4096],
            sidechain_len: 0,
            envelope: 0.0,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Set the sidechain input buffer for the next process() call.
    /// The buffer should be interleaved stereo, same length as the main input.
    pub fn set_sidechain_input(&mut self, buffer: &[f32]) {
        let len = buffer.len().min(self.sidechain_buffer.len());
        self.sidechain_buffer[..len].copy_from_slice(&buffer[..len]);
        self.sidechain_len = len;
    }

    /// Calculate attack coefficient using type-safe Milliseconds.
    #[inline]
    fn attack_coeff(&self) -> f32 {
        let attack_secs = self.attack.to_seconds();
        crate::math::envelope_coeff(attack_secs.as_f32(), self.sample_rate.as_f32())
    }

    /// Calculate release coefficient using type-safe Milliseconds.
    #[inline]
    fn release_coeff(&self) -> f32 {
        let release_secs = self.release.to_seconds();
        crate::math::envelope_coeff(release_secs.as_f32(), self.sample_rate.as_f32())
    }

    /// Calculate gain reduction for a given input level.
    #[inline]
    fn compute_gain(&self, input_db: f32) -> f32 {
        let threshold = self.threshold.as_f32();
        if input_db > threshold {
            // Calculate gain reduction using type-safe Ratio
            let overshoot = input_db - threshold;
            let compressed = self.ratio.compress(overshoot);
            let gain_reduction = compressed - overshoot;
            Decibels::new(gain_reduction + self.makeup.as_f32()).to_linear()
        } else {
            self.makeup.to_linear()
        }
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Compressor {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("compressor", "Compressor")
            .description("Dynamics compressor with adjustable attack and release")
            .category(ModuleCategory::Effect)
            .tag("compressor")
            .tag("effect")
            .tag("dynamics")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .parameter(
                ParameterDescriptor::float(
                    "threshold",
                    Param::Compressor(CompressorParam::Threshold(Decibels::new(-20.0))),
                    "Threshold",
                )
                .description("Compression threshold")
                .range(-60.0, 0.0)
                .default(-20.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "ratio",
                    Param::Compressor(CompressorParam::Ratio(Ratio::MEDIUM)),
                    "Ratio",
                )
                .description("Compression ratio")
                .range(1.0, 20.0)
                .default(4.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "attack",
                    Param::Compressor(CompressorParam::Attack(Milliseconds::new(10.0))),
                    "Attack",
                )
                .description("Attack time")
                .range(0.1, 100.0)
                .default(10.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "release",
                    Param::Compressor(CompressorParam::Release(Milliseconds::new(100.0))),
                    "Release",
                )
                .description("Release time")
                .range(10.0, 1000.0)
                .default(100.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "makeup",
                    Param::Compressor(CompressorParam::Makeup(Decibels::new(0.0))),
                    "Makeup",
                )
                .description("Makeup gain")
                .range(0.0, 24.0)
                .default(0.0)
                .unit(ParameterUnit::Decibels)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Compressor(CompressorParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix (parallel compression)")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sidechain",
                    Param::Compressor(CompressorParam::SidechainEnabled(false)),
                    "Sidechain",
                )
                .description("Enable external sidechain input for detection")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sc_filter",
                    Param::Compressor(CompressorParam::SidechainFilter(Hertz::new(80.0))),
                    "SC Filter",
                )
                .description("Sidechain high-pass filter frequency")
                .range(20.0, 500.0)
                .default(80.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Compressor {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        self.sample_rate = context.sample_rate;

        let attack_coeff = self.attack_coeff();
        let release_coeff = self.release_coeff();

        // Sidechain HPF coefficient
        let sc_hpf_coeff = if self.sidechain_enabled && self.sidechain_filter_freq.as_f32() > 20.0 {
            let rc = 1.0 / (2.0 * std::f32::consts::PI * self.sidechain_filter_freq.as_f32());
            let dt = 1.0 / self.sample_rate.as_f32();
            rc / (rc + dt)
        } else {
            0.0 // No filtering
        };

        let use_sidechain = self.sidechain_enabled && self.sidechain_len > 0;

        // Process stereo interleaved
        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);
            let in_l = dry.left;
            let in_r = dry.right;

            // Determine detection signal: sidechain or input
            let detect_peak = if use_sidechain {
                let sc = StereoSample::read_frame(&self.sidechain_buffer, frame);
                let sc_mono = sc.left.abs().max(sc.right.abs());

                // Apply HPF to sidechain signal
                if sc_hpf_coeff > 0.0 {
                    let filtered = sc_hpf_coeff * (self.sc_filter_state + sc_mono);
                    self.sc_filter_state = filtered - sc_mono;
                    filtered.abs()
                } else {
                    sc_mono
                }
            } else {
                in_l.abs().max(in_r.abs())
            };

            let peak_db = Decibels::from_linear(detect_peak.max(1e-6)).as_f32();

            // Envelope follower with attack/release
            let coeff = if peak_db > self.envelope {
                attack_coeff
            } else {
                release_coeff
            };
            self.envelope = coeff * self.envelope + (1.0 - coeff) * peak_db;

            // Calculate gain
            let gain = self.compute_gain(self.envelope);

            // Apply compression to the MAIN input (not sidechain)
            let wet_l = in_l * gain;
            let wet_r = in_r * gain;

            // Mix dry/wet (parallel compression)
            let mix = self.mix.as_f32();
            let result = StereoSample::new(in_l, in_r).blend(StereoSample::new(wet_l, wet_r), mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.sc_filter_state = 0.0;
        self.sidechain_len = 0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Compressor(comp_param) = param {
            match comp_param {
                CompressorParam::Threshold(t) => {
                    self.threshold = Decibels::new(t.as_f32().clamp(-60.0, 0.0));
                }
                CompressorParam::Ratio(r) => {
                    self.ratio = Ratio::new(r.as_f32()).clamp_typical();
                }
                CompressorParam::Attack(a) => {
                    self.attack = Milliseconds::new(a.as_f32().clamp(0.1, 100.0));
                }
                CompressorParam::Release(r) => {
                    self.release = Milliseconds::new(r.as_f32().clamp(10.0, 1000.0));
                }
                CompressorParam::Makeup(m) => {
                    self.makeup = Decibels::new(m.as_f32().clamp(0.0, 24.0));
                }
                CompressorParam::Mix(m) => {
                    self.mix = m;
                }
                CompressorParam::SidechainEnabled(b) => {
                    self.sidechain_enabled = b;
                }
                CompressorParam::SidechainFilter(hz) => {
                    self.sidechain_filter_freq = Hertz::new(hz.as_f32().clamp(20.0, 500.0));
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Compressor(comp_param) = param {
            Some(match comp_param {
                CompressorParam::Threshold(_) => self.threshold.as_f32(),
                CompressorParam::Ratio(_) => self.ratio.as_f32(),
                CompressorParam::Attack(_) => self.attack.as_f32(),
                CompressorParam::Release(_) => self.release.as_f32(),
                CompressorParam::Makeup(_) => self.makeup.as_f32(),
                CompressorParam::Mix(_) => self.mix.as_f32(),
                CompressorParam::SidechainEnabled(_) => {
                    if self.sidechain_enabled {
                        1.0
                    } else {
                        0.0
                    }
                }
                CompressorParam::SidechainFilter(_) => self.sidechain_filter_freq.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Compressor(CompressorParam::Threshold(self.threshold)),
            Param::Compressor(CompressorParam::Ratio(self.ratio)),
            Param::Compressor(CompressorParam::Attack(self.attack)),
            Param::Compressor(CompressorParam::Release(self.release)),
            Param::Compressor(CompressorParam::Makeup(self.makeup)),
            Param::Compressor(CompressorParam::Mix(self.mix)),
            Param::Compressor(CompressorParam::SidechainEnabled(self.sidechain_enabled)),
            Param::Compressor(CompressorParam::SidechainFilter(self.sidechain_filter_freq)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Compressor
    }
}
