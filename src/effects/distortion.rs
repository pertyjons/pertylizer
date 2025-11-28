//! Distortion effect module.
//!
//! Features:
//! - Multiple distortion types (soft clip, hard clip, foldback, bitcrush)
//! - Drive control
//! - Tone filter
//! - Mix control

use crate::engine::typed_params::{
    TypedParam, TypedValue, DistortionParam, ChorusParam, ModuleType,
    DistortionMode,
};
use crate::modules::core::*;

// DistortionMode is imported from typed_params
// Type alias for local use
pub type DistortionType = DistortionMode;

// Add to_choices helper for UI compatibility
impl DistortionMode {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|d| ChoiceOption::new(d.id(), d.name()))
            .collect()
    }
}

/// Distortion effect.
pub struct Distortion {
    // Parameters
    dist_type: DistortionType,
    drive: f32,      // 0-1 (maps to gain)
    tone: f32,       // 0-1 (lowpass cutoff)
    mix: f32,        // 0-1
    bit_depth: f32,  // For bitcrush (1-16)

    // Filter state
    filter_state: f32,

    // State
    sample_rate: f32,
}

impl Distortion {
    pub fn new() -> Self {
        Self {
            dist_type: DistortionType::SoftClip,
            drive: 0.5,
            tone: 0.8,
            mix: 1.0,
            bit_depth: 8.0,
            filter_state: 0.0,
            sample_rate: 48000.0,
        }
    }

    /// Apply distortion to a sample.
    #[inline]
    fn distort(&self, input: f32) -> f32 {
        // Apply drive (exponential curve for more usable range)
        let gain = 1.0 + self.drive * self.drive * 50.0;
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
        let cutoff = 200.0 + self.tone * self.tone * 15000.0;
        let coef = (-std::f32::consts::TAU * cutoff / self.sample_rate).exp();
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
                    TypedParam::Distortion(DistortionParam::Mode),
                    "Type",
                    DistortionType::to_choices(),
                )
                .description("Distortion algorithm"),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Distortion(DistortionParam::Drive), "Drive")
                    .description("Distortion amount")
                    .range(0.0, 1.0)
                    .default(0.5)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Distortion(DistortionParam::Tone), "Tone")
                    .description("Post-distortion filter")
                    .range(0.0, 1.0)
                    .default(0.8)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Distortion(DistortionParam::Mix), "Mix")
                    .description("Dry/wet mix")
                    .range(0.0, 1.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
    }
}

impl EffectModule for Distortion {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;

        for i in 0..input.len().min(output.len()) {
            let dry = input[i];
            let distorted = self.distort(dry);
            let filtered = self.apply_tone(distorted);
            output[i] = dry * (1.0 - self.mix) + filtered * self.mix;
        }
    }

    fn reset(&mut self) {
        self.filter_state = 0.0;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    fn get_mix(&self) -> f32 {
        self.mix
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Distortion(dist_param) = param {
            match dist_param {
                DistortionParam::Mode => {
                    if let TypedValue::DistortionMode(mode) = value {
                        // Direct assignment - same type now!
                        self.dist_type = mode;
                    }
                }
                DistortionParam::Drive => {
                    if let Some(d) = value.as_float() {
                        self.drive = d.clamp(0.0, 1.0);
                    }
                }
                DistortionParam::Tone => {
                    if let Some(t) = value.as_float() {
                        self.tone = t.clamp(0.0, 1.0);
                    }
                }
                DistortionParam::Mix => {
                    if let Some(m) = value.as_float() {
                        self.mix = m.clamp(0.0, 1.0);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Distortion(dist_param) = param {
            match dist_param {
                DistortionParam::Mode => {
                    // Direct use - same type now!
                    Some(TypedValue::DistortionMode(self.dist_type))
                }
                DistortionParam::Drive => Some(TypedValue::Float(self.drive)),
                DistortionParam::Tone => Some(TypedValue::Float(self.tone)),
                DistortionParam::Mix => Some(TypedValue::Float(self.mix)),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Distortion
    }
}

/// Chorus effect.
pub struct Chorus {
    // Parameters
    rate: f32,       // Hz
    depth: f32,      // 0-1
    mix: f32,        // 0-1
    voices: u32,     // 1-4

    // Delay buffer
    buffer: Vec<f32>,
    write_pos: usize,

    // LFO state (per voice)
    lfo_phases: [f32; 4],

    // State
    sample_rate: f32,
}

impl Chorus {
    const MAX_DELAY_MS: f32 = 50.0;

    pub fn new() -> Self {
        Self {
            rate: 0.5,
            depth: 0.5,
            mix: 0.5,
            voices: 2,
            buffer: vec![0.0; 48000], // Will be resized
            write_pos: 0,
            lfo_phases: [0.0, 0.25, 0.5, 0.75], // Spread phases
            sample_rate: 48000.0,
        }
    }

    fn resize_buffer(&mut self) {
        let size = (Self::MAX_DELAY_MS / 1000.0 * self.sample_rate) as usize;
        if self.buffer.len() != size {
            self.buffer.resize(size, 0.0);
            self.write_pos = 0;
        }
    }

    #[inline]
    fn read_interpolated(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        let read_pos = (self.write_pos as f32 - delay_samples).rem_euclid(len as f32);
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
                ParameterDescriptor::float(TypedParam::Chorus(ChorusParam::Rate), "Rate")
                    .description("LFO rate")
                    .range(0.1, 5.0)
                    .default(0.5)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Chorus(ChorusParam::Depth), "Depth")
                    .description("Modulation depth")
                    .range(0.0, 1.0)
                    .default(0.5)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Chorus(ChorusParam::Mix), "Mix")
                    .description("Dry/wet mix")
                    .range(0.0, 1.0)
                    .default(0.5)
                    .widget(WidgetHint::Knob),
            )
    }
}

impl EffectModule for Chorus {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        self.resize_buffer();

        let base_delay_ms = 7.0;
        let mod_depth_ms = self.depth * 5.0;
        let phase_inc = self.rate / self.sample_rate;

        for i in 0..input.len().min(output.len()) {
            let dry = input[i];

            // Write to delay buffer
            self.buffer[self.write_pos] = dry;

            // Sum chorus voices
            let mut wet = 0.0f32;
            for v in 0..self.voices as usize {
                let lfo = (self.lfo_phases[v] * std::f32::consts::TAU).sin();
                let delay_ms = base_delay_ms + mod_depth_ms * lfo;
                let delay_samples = delay_ms / 1000.0 * self.sample_rate;

                wet += self.read_interpolated(delay_samples);

                // Advance LFO
                self.lfo_phases[v] = (self.lfo_phases[v] + phase_inc).rem_euclid(1.0);
            }

            wet /= self.voices as f32;

            // Advance write position
            self.write_pos = (self.write_pos + 1) % self.buffer.len();

            output[i] = dry * (1.0 - self.mix) + wet * self.mix;
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    fn get_mix(&self) -> f32 {
        self.mix
    }
    
    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Chorus(chorus_param) = param {
            match chorus_param {
                ChorusParam::Rate => {
                    if let Some(r) = value.as_float() {
                        self.rate = r.clamp(0.1, 5.0);
                    }
                }
                ChorusParam::Depth => {
                    if let Some(d) = value.as_float() {
                        self.depth = d.clamp(0.0, 1.0);
                    }
                }
                ChorusParam::Mix => {
                    if let Some(m) = value.as_float() {
                        self.mix = m.clamp(0.0, 1.0);
                    }
                }
                ChorusParam::Voices => {
                    if let Some(v) = value.as_int() {
                        self.voices = (v as u32).clamp(1, 4);
                    }
                }
                _ => {}
            }
        }
    }
    
    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Chorus(chorus_param) = param {
            match chorus_param {
                ChorusParam::Rate => Some(TypedValue::Float(self.rate)),
                ChorusParam::Depth => Some(TypedValue::Float(self.depth)),
                ChorusParam::Mix => Some(TypedValue::Float(self.mix)),
                ChorusParam::Voices => Some(TypedValue::Int(self.voices as i32)),
                _ => None,
            }
        } else {
            None
        }
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
        dist.sample_rate = 48000.0;

        for dt in DistortionType::ALL {
            dist.dist_type = dt;
            let input = [0.5f32; 100];
            let mut output = [0.0f32; 100];

            let context = ProcessContext {
                sample_rate: 48000.0,
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
