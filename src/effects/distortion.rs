//! Distortion effect module.
//!
//! Features:
//! - Multiple distortion types (soft clip, hard clip, foldback, bitcrush)
//! - Drive control
//! - Tone filter
//! - Mix control

use crate::engine::typed_params::{
    DistortionMode, DistortionParam, ModuleType, Param,
};
use crate::modules::core::*;
use crate::types::{BufferIndex, Hertz, NormalizedValue, Phase, SampleRate, VoiceCount};

// Type alias for local use
pub type DistortionType = DistortionMode;

/// Distortion effect.
pub struct Distortion {
    // Parameters
    dist_type: DistortionType,
    drive: NormalizedValue,
    tone: NormalizedValue,
    mix: NormalizedValue,
    bit_depth: f32,  // For bitcrush (1-16) - not a normalized value

    // Filter state
    filter_state: f32,

    // State
    sample_rate: SampleRate,
}

impl Distortion {
    pub fn new() -> Self {
        Self {
            dist_type: DistortionType::SoftClip,
            drive: NormalizedValue::CENTER,
            tone: NormalizedValue::new(0.8),
            mix: NormalizedValue::MAX,
            bit_depth: 8.0,
            filter_state: 0.0,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Apply distortion to a sample.
    #[inline]
    fn distort(&self, input: f32) -> f32 {
        // Apply drive (exponential curve for more usable range)
        let drive = self.drive.as_f32();
        let gain = 1.0 + drive * drive * 50.0;
        let driven = input * gain;

        match self.dist_type {
            DistortionType::SoftClip => {
                // Tanh soft clipping
                driven.tanh()
            }

            DistortionType::HardClip => {
                // Simple hard clipping
                driven.clamp(-1.0, 1.0)
            }

            DistortionType::Foldback => {
                // Foldback distortion
                let threshold = 1.0;
                let mut x = driven;
                while x.abs() > threshold {
                    if x > threshold {
                        x = threshold - (x - threshold);
                    } else if x < -threshold {
                        x = -threshold - (x + threshold);
                    }
                }
                x
            }

            DistortionType::Bitcrush => {
                // Bit reduction
                let bits = self.bit_depth.max(1.0);
                let levels = 2.0f32.powf(bits);
                let quantized = (driven * levels).round() / levels;
                quantized.clamp(-1.0, 1.0)
            }

            DistortionType::Tube => {
                // Tube-like asymmetric soft clipping
                let x = driven;
                if x >= 0.0 {
                    1.0 - (-x).exp()
                } else {
                    -1.0 + x.exp()
                }
            }
        }
    }

    /// Apply tone filter (one-pole lowpass).
    #[inline]
    fn apply_tone(&mut self, input: f32) -> f32 {
        // Map tone parameter to cutoff frequency
        let tone = self.tone.as_f32();
        let cutoff = 200.0 + tone * tone * 15000.0;
        let coef = (-std::f32::consts::TAU * cutoff / self.sample_rate.as_f32()).exp();
        self.filter_state = input * (1.0 - coef) + self.filter_state * coef;
        self.filter_state
    }
}

impl Default for Distortion {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Distortion {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("distortion", "Distortion")
            .description("Multi-mode distortion effect")
            .category(ModuleCategory::Effect)
            .tag("distortion")
            .tag("effect")
            .tag("overdrive")
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
            .port(PortDescriptor::audio_output("out", "Out").description("Audio output"))
            .port(PortDescriptor::control_input("drive_cv", "Drive CV").description("Drive modulation"))
            .parameter(
                ParameterDescriptor::choice(
                    Param::Distortion(DistortionParam::Mode(DistortionMode::SoftClip)),
                    "Type",
                    DistortionMode::to_choices(),
                )
                .description("Distortion algorithm"),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Distortion(DistortionParam::Drive(NormalizedValue::CENTER)),
                    "Drive",
                )
                .description("Distortion amount")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Distortion(DistortionParam::Tone(NormalizedValue::new(0.8))),
                    "Tone",
                )
                .description("Post-distortion filter")
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Distortion(DistortionParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Distortion {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        let mix = self.mix.as_f32();

        for i in 0..input.len().min(output.len()) {
            let dry = input[i];
            let distorted = self.distort(dry);
            let filtered = self.apply_tone(distorted);
            output[i] = dry * (1.0 - mix) + filtered * mix;
        }
    }

    fn reset(&mut self) {
        self.filter_state = 0.0;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = NormalizedValue::new(mix);
    }

    fn get_mix(&self) -> f32 {
        self.mix.as_f32()
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Distortion(dist_param) = param {
            match dist_param {
                DistortionParam::Mode(mode) => {
                    self.dist_type = mode;
                }
                DistortionParam::Drive(d) => {
                    self.drive = d;
                }
                DistortionParam::Tone(t) => {
                    self.tone = t;
                }
                DistortionParam::Mix(m) => {
                    self.mix = m;
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Distortion(dist_param) = param {
            Some(match dist_param {
                DistortionParam::Mode(_) => self.dist_type.index() as f32,
                DistortionParam::Drive(_) => self.drive.as_f32(),
                DistortionParam::Tone(_) => self.tone.as_f32(),
                DistortionParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Distortion(DistortionParam::Mode(self.dist_type)),
            Param::Distortion(DistortionParam::Drive(self.drive)),
            Param::Distortion(DistortionParam::Tone(self.tone)),
            Param::Distortion(DistortionParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Distortion
    }
}

/// Chorus effect.
pub struct Chorus {
    // Parameters
    rate: Hertz,
    depth: NormalizedValue,
    mix: NormalizedValue,
    voices: VoiceCount,

    // Delay buffer
    buffer: Vec<f32>,
    write_pos: BufferIndex,

    // LFO state (per voice)
    lfo_phases: [Phase; 4],

    // State
    sample_rate: SampleRate,
}

impl Chorus {
    const MAX_DELAY_MS: f32 = 50.0;

    pub fn new() -> Self {
        Self {
            rate: Hertz::new(0.5),
            depth: NormalizedValue::CENTER,
            mix: NormalizedValue::CENTER,
            voices: VoiceCount::DUAL,
            buffer: vec![0.0; 48000], // Will be resized
            write_pos: BufferIndex::ZERO,
            lfo_phases: [
                Phase::new(0.0),
                Phase::new(0.25),
                Phase::new(0.5),
                Phase::new(0.75),
            ],
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    fn resize_buffer(&mut self) {
        let size = (Self::MAX_DELAY_MS / 1000.0 * self.sample_rate.as_f32()) as usize;
        if self.buffer.len() != size {
            self.buffer.resize(size, 0.0);
            self.write_pos = BufferIndex::ZERO;
        }
    }

    #[inline]
    fn read_interpolated(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        let read_pos = (self.write_pos.as_usize() as f32 - delay_samples).rem_euclid(len as f32);
        let idx0 = (read_pos as usize) % len;  // Ensure within bounds
        let idx1 = (idx0 + 1) % len;
        let frac = read_pos - read_pos.floor();
        self.buffer[idx0] * (1.0 - frac) + self.buffer[idx1] * frac
    }
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Chorus {
    fn descriptor(&self) -> ModuleDescriptor {
        use crate::engine::typed_params::ChorusParam;
        ModuleDescriptor::new("chorus", "Chorus")
            .description("Multi-voice chorus effect")
            .category(ModuleCategory::Effect)
            .tag("chorus")
            .tag("effect")
            .tag("modulation")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .port(PortDescriptor::control_input("rate_cv", "Rate CV").description("Rate modulation"))
            .parameter(
                ParameterDescriptor::float(
                    Param::Chorus(ChorusParam::Rate(Hertz::new(0.5))),
                    "Rate",
                )
                .description("LFO rate")
                .range(0.1, 5.0)
                .default(0.5)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Chorus(ChorusParam::Depth(NormalizedValue::CENTER)),
                    "Depth",
                )
                .description("Modulation depth")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Chorus(ChorusParam::Mix(NormalizedValue::CENTER)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Chorus {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        self.resize_buffer();

        let base_delay_ms = 7.0;
        let mod_depth_ms = self.depth.as_f32() * 5.0;
        let phase_inc = self.rate.as_f32() / self.sample_rate.as_f32();

        for i in 0..input.len().min(output.len()) {
            let dry = input[i];

            // Write to delay buffer
            self.buffer[self.write_pos.as_usize()] = dry;

            // Sum chorus voices
            let mut wet = 0.0f32;
            let voice_count = self.voices.as_usize();
            for v in 0..voice_count {
                let lfo = (self.lfo_phases[v].as_f32() * std::f32::consts::TAU).sin();
                let delay_ms = base_delay_ms + mod_depth_ms * lfo;
                let delay_samples = delay_ms / 1000.0 * self.sample_rate.as_f32();

                wet += self.read_interpolated(delay_samples);

                // Advance LFO
                self.lfo_phases[v] = Phase::new((self.lfo_phases[v].as_f32() + phase_inc).rem_euclid(1.0));
            }

            wet /= voice_count as f32;

            // Advance write position
            self.write_pos = self.write_pos.advance(self.buffer.len());

            let mix = self.mix.as_f32();
            output[i] = dry * (1.0 - mix) + wet * mix;
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = BufferIndex::ZERO;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = NormalizedValue::new(mix);
    }

    fn get_mix(&self) -> f32 {
        self.mix.as_f32()
    }

    fn set_param(&mut self, param: Param) {
        use crate::engine::typed_params::ChorusParam;
        if let Param::Chorus(chorus_param) = param {
            match chorus_param {
                ChorusParam::Rate(r) => {
                    self.rate = Hertz::new(r.as_f32().clamp(0.1, 5.0));
                }
                ChorusParam::Depth(d) => {
                    self.depth = d;
                }
                ChorusParam::Mix(m) => {
                    self.mix = m;
                }
                ChorusParam::Voices(v) => {
                    self.voices = VoiceCount::new(v.as_u8()).clamp_chorus();
                }
                _ => {}
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        use crate::engine::typed_params::ChorusParam;
        if let Param::Chorus(chorus_param) = param {
            Some(match chorus_param {
                ChorusParam::Rate(_) => self.rate.as_f32(),
                ChorusParam::Depth(_) => self.depth.as_f32(),
                ChorusParam::Mix(_) => self.mix.as_f32(),
                ChorusParam::Voices(_) => self.voices.as_u8() as f32,
                _ => return None,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        use crate::engine::typed_params::ChorusParam;
        vec![
            Param::Chorus(ChorusParam::Rate(self.rate)),
            Param::Chorus(ChorusParam::Depth(self.depth)),
            Param::Chorus(ChorusParam::Mix(self.mix)),
            Param::Chorus(ChorusParam::Voices(self.voices)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Chorus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distortion_types() {
        let mut dist = Distortion::new();
        dist.sample_rate = SampleRate::new(48000.0);

        for dt in DistortionType::ALL {
            dist.dist_type = dt;
            let input = [0.5f32; 100];
            let mut output = [0.0f32; 100];

            let context = ProcessContext {
                sample_rate: SampleRate::DVD_QUALITY,
                samples: 100,
                ..Default::default()
            };

            dist.process(&input, &mut output, &context);

            for &sample in &output {
                assert!(sample.is_finite(), "Output not finite for {:?}", dt);
                assert!(sample.abs() <= 2.0, "Output too large for {:?}", dt);
            }
        }
    }
}
