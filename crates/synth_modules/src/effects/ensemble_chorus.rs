//! Juno-style BBD ensemble chorus effect.
//!
//! Features:
//! - 2-3 voice BBD chorus with inverted LFO phases for stereo spread
//! - One-pole tone filter in delay path
//! - Subtle BBD clock noise
//! - Mid/side stereo width processing

use synth_core::NoiseState;
use synth_core::{
    AudioEffect, Describable, EnsembleChorusParam, Hertz, Milliseconds, ModuleCategory,
    ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor, ParameterUnit,
    Phase, ProcessContext, SampleRate, VoiceCount, WidgetHint,
};

/// Maximum delay in milliseconds (for buffer sizing).
const MAX_DELAY_MS: f32 = 25.0;
/// Maximum number of chorus voices.
const MAX_VOICES: usize = 3;

/// Juno-style BBD ensemble chorus.
pub struct EnsembleChorus {
    // Parameters
    rate: Hertz,
    depth: Milliseconds,
    base_delay: Milliseconds,
    mix: NormalizedValue,
    tone: NormalizedValue,
    noise_amt: NormalizedValue,
    stereo_width: NormalizedValue,
    voices: VoiceCount,

    // Per-voice delay buffers (L and R)
    buffers_l: [Vec<f32>; MAX_VOICES],
    buffers_r: [Vec<f32>; MAX_VOICES],
    write_pos: usize,

    // LFO state per voice
    lfo_phases: [Phase; MAX_VOICES],

    // One-pole tone filter state (per voice, per channel)
    tone_state_l: [f32; MAX_VOICES],
    tone_state_r: [f32; MAX_VOICES],

    // Noise generator
    noise: NoiseState,

    sample_rate: SampleRate,
}

impl EnsembleChorus {
    pub fn new() -> Self {
        let buf_size = (MAX_DELAY_MS / 1000.0 * 48000.0) as usize + 2;
        Self {
            rate: Hertz::new(0.6),
            depth: Milliseconds::new(1.2),
            base_delay: Milliseconds::new(12.0),
            mix: NormalizedValue::CENTER,
            tone: NormalizedValue::new(0.4),
            noise_amt: NormalizedValue::new(0.1),
            stereo_width: NormalizedValue::MAX,
            voices: VoiceCount::DUAL,

            buffers_l: [
                vec![0.0; buf_size],
                vec![0.0; buf_size],
                vec![0.0; buf_size],
            ],
            buffers_r: [
                vec![0.0; buf_size],
                vec![0.0; buf_size],
                vec![0.0; buf_size],
            ],
            write_pos: 0,

            lfo_phases: [Phase::new(0.0), Phase::new(0.333), Phase::new(0.667)],

            tone_state_l: [0.0; MAX_VOICES],
            tone_state_r: [0.0; MAX_VOICES],

            noise: NoiseState::DEFAULT,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    fn resize_buffers(&mut self) {
        let size = (MAX_DELAY_MS / 1000.0 * self.sample_rate.as_f32()) as usize + 2;
        for v in 0..MAX_VOICES {
            if self.buffers_l[v].len() != size {
                self.buffers_l[v].resize(size, 0.0);
                self.buffers_r[v].resize(size, 0.0);
            }
        }
        self.write_pos = 0;
    }

    #[inline]
    fn read_interpolated(buffer: &[f32], write_pos: usize, delay_samples: f32) -> f32 {
        let len = buffer.len();
        let read_pos = (write_pos as f32 - delay_samples).rem_euclid(len as f32);
        let idx0 = (read_pos as usize) % len;
        let idx1 = (idx0 + 1) % len;
        let frac = read_pos - read_pos.floor();
        buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac
    }
}

impl Default for EnsembleChorus {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for EnsembleChorus {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("ensemble_chorus", "Ensemble Chorus")
            .description("Juno-style BBD ensemble chorus with stereo width")
            .category(ModuleCategory::Effect)
            .tag("chorus")
            .tag("effect")
            .tag("ensemble")
            .parameter(
                ParameterDescriptor::float(
                    Param::EnsembleChorus(EnsembleChorusParam::Rate(Hertz::new(0.6))),
                    "Rate",
                )
                .description("LFO rate")
                .range(0.2, 1.5)
                .default(0.6)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::EnsembleChorus(EnsembleChorusParam::Depth(Milliseconds::new(1.2))),
                    "Depth",
                )
                .description("Modulation depth in ms")
                .range(0.2, 2.5)
                .default(1.2)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::EnsembleChorus(EnsembleChorusParam::BaseDelay(Milliseconds::new(12.0))),
                    "Base Delay",
                )
                .description("Base delay time in ms")
                .range(5.0, 20.0)
                .default(12.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::EnsembleChorus(EnsembleChorusParam::Tone(NormalizedValue::new(0.4))),
                    "Tone",
                )
                .description("Lowpass tone in delay path")
                .range(0.0, 1.0)
                .default(0.4)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::EnsembleChorus(EnsembleChorusParam::Noise(NormalizedValue::new(0.1))),
                    "Noise",
                )
                .description("BBD clock noise amount")
                .range(0.0, 1.0)
                .default(0.1)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::EnsembleChorus(EnsembleChorusParam::StereoWidth(NormalizedValue::MAX)),
                    "Stereo Width",
                )
                .description("Stereo spread via mid/side")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::EnsembleChorus(EnsembleChorusParam::Voices(VoiceCount::DUAL)),
                    "Voices",
                )
                .description("Number of chorus voices (2 or 3)")
                .range(2.0, 3.0)
                .default(2.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::EnsembleChorus(EnsembleChorusParam::Mix(NormalizedValue::CENTER)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for EnsembleChorus {
    #[allow(clippy::too_many_lines)]
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        let sr = self.sample_rate.as_f32();
        let phase_inc = self.rate.as_f32() / sr;
        let depth_samples = self.depth.as_f32() / 1000.0 * sr;
        let base_samples = self.base_delay.as_f32() / 1000.0 * sr;
        let voice_count = (self.voices.as_u8() as usize).clamp(2, MAX_VOICES);

        // Tone filter coefficient: map tone 0..1 to cutoff 1000..8000 Hz
        let fc = 1000.0 + self.tone.as_f32() * 7000.0;
        let tone_coeff = (-std::f32::consts::TAU * fc / sr).exp();

        let noise_level = self.noise_amt.as_f32() * 0.005;
        let mix = self.mix.as_f32();
        let width = self.stereo_width.as_f32();

        let channels = 2;
        for frame in 0..context.samples.as_usize() {
            let idx_l = frame * channels;
            let idx_r = frame * channels + 1;

            let dry_l = if idx_l < input.len() {
                input[idx_l]
            } else {
                0.0
            };
            let dry_r = if idx_r < input.len() {
                input[idx_r]
            } else {
                0.0
            };

            let mut wet_l = 0.0f32;
            let mut wet_r = 0.0f32;

            for v in 0..voice_count {
                let lfo = (self.lfo_phases[v].as_f32() * std::f32::consts::TAU).sin();

                // Write dry signal into per-voice delay buffers
                self.buffers_l[v][self.write_pos] = dry_l;
                self.buffers_r[v][self.write_pos] = dry_r;

                // Inverted phase for L/R
                let delay_l = base_samples + depth_samples * lfo;
                let delay_r = base_samples - depth_samples * lfo;

                let mut sample_l =
                    Self::read_interpolated(&self.buffers_l[v], self.write_pos, delay_l.max(1.0));
                let mut sample_r =
                    Self::read_interpolated(&self.buffers_r[v], self.write_pos, delay_r.max(1.0));

                // One-pole tone filter
                self.tone_state_l[v] =
                    self.tone_state_l[v] * tone_coeff + sample_l * (1.0 - tone_coeff);
                sample_l = self.tone_state_l[v];

                self.tone_state_r[v] =
                    self.tone_state_r[v] * tone_coeff + sample_r * (1.0 - tone_coeff);
                sample_r = self.tone_state_r[v];

                // Add BBD noise
                if noise_level > 0.0 {
                    sample_l += self.noise.next() * noise_level;
                    sample_r += self.noise.next() * noise_level;
                }

                wet_l += sample_l;
                wet_r += sample_r;

                // Advance LFO
                self.lfo_phases[v] =
                    Phase::new((self.lfo_phases[v].as_f32() + phase_inc).rem_euclid(1.0));
            }

            wet_l /= voice_count as f32;
            wet_r /= voice_count as f32;

            // Stereo width via mid/side
            let mid = (wet_l + wet_r) * 0.5;
            let side = (wet_l - wet_r) * 0.5 * width;
            wet_l = mid + side;
            wet_r = mid - side;

            // Advance write position
            let buf_len = self.buffers_l[0].len();
            self.write_pos = (self.write_pos + 1) % buf_len;

            // Mix
            if idx_l < output.len() {
                output[idx_l] = dry_l * (1.0 - mix) + wet_l * mix;
            }
            if idx_r < output.len() {
                output[idx_r] = dry_r * (1.0 - mix) + wet_r * mix;
            }
        }
    }

    fn reset(&mut self) {
        for v in 0..MAX_VOICES {
            self.buffers_l[v].fill(0.0);
            self.buffers_r[v].fill(0.0);
            self.tone_state_l[v] = 0.0;
            self.tone_state_r[v] = 0.0;
        }
        self.write_pos = 0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }
    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::EnsembleChorus(p) = param {
            match p {
                EnsembleChorusParam::Rate(v) => self.rate = Hertz::new(v.as_f32().clamp(0.2, 1.5)),
                EnsembleChorusParam::Depth(v) => {
                    self.depth = Milliseconds::new(v.as_f32().clamp(0.2, 2.5));
                }
                EnsembleChorusParam::BaseDelay(v) => {
                    self.base_delay = Milliseconds::new(v.as_f32().clamp(5.0, 20.0));
                }
                EnsembleChorusParam::Mix(v) => self.mix = v,
                EnsembleChorusParam::Tone(v) => self.tone = v,
                EnsembleChorusParam::Noise(v) => self.noise_amt = v,
                EnsembleChorusParam::StereoWidth(v) => self.stereo_width = v,
                EnsembleChorusParam::Voices(v) => {
                    self.voices = VoiceCount::new(v.as_u8().clamp(2, 3));
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::EnsembleChorus(p) = param {
            Some(match p {
                EnsembleChorusParam::Rate(_) => self.rate.as_f32(),
                EnsembleChorusParam::Depth(_) => self.depth.as_f32(),
                EnsembleChorusParam::BaseDelay(_) => self.base_delay.as_f32(),
                EnsembleChorusParam::Mix(_) => self.mix.as_f32(),
                EnsembleChorusParam::Tone(_) => self.tone.as_f32(),
                EnsembleChorusParam::Noise(_) => self.noise_amt.as_f32(),
                EnsembleChorusParam::StereoWidth(_) => self.stereo_width.as_f32(),
                EnsembleChorusParam::Voices(_) => self.voices.as_u8() as f32,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::EnsembleChorus(EnsembleChorusParam::Rate(self.rate)),
            Param::EnsembleChorus(EnsembleChorusParam::Depth(self.depth)),
            Param::EnsembleChorus(EnsembleChorusParam::BaseDelay(self.base_delay)),
            Param::EnsembleChorus(EnsembleChorusParam::Tone(self.tone)),
            Param::EnsembleChorus(EnsembleChorusParam::Noise(self.noise_amt)),
            Param::EnsembleChorus(EnsembleChorusParam::StereoWidth(self.stereo_width)),
            Param::EnsembleChorus(EnsembleChorusParam::Voices(self.voices)),
            Param::EnsembleChorus(EnsembleChorusParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::EnsembleChorus
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.resize_buffers();
    }
}
