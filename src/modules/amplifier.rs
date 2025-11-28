//! Amplifier (VCA) module.
//!
//! Features:
//! - Level control with CV input
//! - Pan control
//! - Optional soft clipping

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_4;

use crate::engine::typed_params::{
    TypedParam, TypedValue, AmplifierParam, ModuleType,
};
use crate::modules::core::*;

/// Voltage Controlled Amplifier.
#[derive(Clone)]
pub struct Amplifier {
    // Parameters
    level: f32,
    pan: f32,  // -1 = left, 0 = center, +1 = right
    
    // Options
    soft_clip: bool,
    
    // State
    sample_rate: f32,
    
    // Outputs
    output_left: AudioBuffer,
    output_right: AudioBuffer,
}

impl Amplifier {
    pub fn new() -> Self {
        Self {
            level: 1.0,
            pan: 0.0,
            soft_clip: false,
            sample_rate: 48000.0,
            output_left: AudioBuffer::new(256),
            output_right: AudioBuffer::new(256),
        }
    }

    /// Calculate constant-power pan coefficients.
    #[inline]
    fn pan_coefficients(&self) -> (f32, f32) {
        // Constant power panning using sine/cosine
        let angle = (self.pan + 1.0) * FRAC_PI_4; // 0 to π/2
        let left = angle.cos();
        let right = angle.sin();
        (left, right)
    }

    /// Soft clipping function.
    #[inline]
    fn soft_clip(x: f32) -> f32 {
        x.tanh()
    }
}

impl Default for Amplifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Amplifier {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("amplifier", "VCA")
            .description("Voltage Controlled Amplifier with panning")
            .category(ModuleCategory::Amplifier)
            .tag("amplifier")
            .tag("vca")
            .tag("utility")
            .parameter(
                ParameterDescriptor::float(TypedParam::Amplifier(AmplifierParam::Level), "Level")
                    .description("Output level")
                    .range(0.0, 2.0)
                    .default(1.0)
                    .widget(WidgetHint::Slider),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Amplifier(AmplifierParam::Pan), "Pan")
                    .description("Stereo position")
                    .range(-1.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::PanKnob),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
            .port(PortDescriptor::control_input("cv", "CV").description("Level CV (0-1)"))
            .port(PortDescriptor::control_input("pan_cv", "Pan CV").description("Pan modulation"))
            .port(PortDescriptor::audio_output("left", "L").description("Left output"))
            .port(PortDescriptor::audio_output("right", "R").description("Right output"))
            .port(PortDescriptor::audio_output("out", "Out").description("Mono output"))
    }
}

impl VoiceModule for Amplifier {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_left.resize(context.samples);
        self.output_right.resize(context.samples);

        let audio_in = inputs.get("in");
        let cv_in = inputs.get("cv");
        let pan_cv = inputs.get("pan_cv");

        for i in 0..context.samples {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);
            
            // CV modulation (multiplicative)
            let cv = cv_in.map(|b| b[i]).unwrap_or(1.0);
            let effective_level = self.level * cv.max(0.0);
            
            // Pan modulation
            let effective_pan = if let Some(pan_mod) = pan_cv {
                (self.pan + pan_mod[i]).clamp(-1.0, 1.0)
            } else {
                self.pan
            };
            
            // Calculate pan coefficients
            let angle = (effective_pan + 1.0) * FRAC_PI_4;
            let pan_left = angle.cos();
            let pan_right = angle.sin();
            
            // Apply level and pan
            let mut left = input * effective_level * pan_left;
            let mut right = input * effective_level * pan_right;
            
            // Optional soft clipping
            if self.soft_clip {
                left = Self::soft_clip(left);
                right = Self::soft_clip(right);
            }
            
            self.output_left[i] = left;
            self.output_right[i] = right;
        }

        // Copy to outputs
        if let Some(left) = outputs.get_mut("left") {
            left.copy_from(&self.output_left);
        }
        if let Some(right) = outputs.get_mut("right") {
            right.copy_from(&self.output_right);
        }
        // Mono output = sum of left and right
        if let Some(out) = outputs.get_mut("out") {
            for i in 0..context.samples {
                out[i] = (self.output_left[i] + self.output_right[i]) * 0.5;
            }
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Amplifier(amp_param) = param {
            match amp_param {
                AmplifierParam::Level => {
                    if let Some(l) = value.as_float() {
                        self.level = l.clamp(0.0, 2.0);
                    }
                }
                AmplifierParam::Pan => {
                    if let Some(p) = value.as_float() {
                        self.pan = p.clamp(-1.0, 1.0);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Amplifier(amp_param) = param {
            match amp_param {
                AmplifierParam::Level => Some(TypedValue::Float(self.level)),
                AmplifierParam::Pan => Some(TypedValue::Float(self.pan)),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Amplifier
    }

    fn reset(&mut self) {
        self.output_left.clear();
        self.output_right.clear();
    }

    fn note_on(&mut self, _note: u8, _velocity: f32) {}
    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn VoiceModule> {
        Box::new(self.clone())
    }
}

/// Simple mixer for combining multiple audio sources.
#[derive(Clone)]
pub struct Mixer {
    // Per-channel levels
    levels: [f32; 8],
    master_level: f32,
    
    // Output
    output_buffer: AudioBuffer,
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            levels: [1.0; 8],
            master_level: 1.0,
            output_buffer: AudioBuffer::new(256),
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Mixer {
    fn descriptor(&self) -> ModuleDescriptor {
        let mut desc = ModuleDescriptor::new("mixer", "Mixer")
            .description("8-channel mixer")
            .category(ModuleCategory::Mixer)
            .tag("mixer")
            .tag("utility");

        // Add 8 input channels
        for i in 1..=8 {
            desc = desc.port(
                PortDescriptor::audio_input(format!("in{i}"), format!("In {i}"))
                    .description(format!("Input channel {i}")),
            );
        }

        desc = desc
            .parameter(
                ParameterDescriptor::float(TypedParam::Mixer(crate::engine::typed_params::MixerParam::Master), "Master")
                    .description("Master output level")
                    .range(0.0, 2.0)
                    .default(1.0)
                    .widget(WidgetHint::Slider),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Mixed output"));

        desc
    }
}

impl VoiceModule for Mixer {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.output_buffer.resize(context.samples);
        self.output_buffer.clear();

        // Sum all inputs
        for i in 1..=8 {
            let key = format!("in{i}");
            if let Some(input) = inputs.get(&key) {
                let level = self.levels[i - 1];
                for j in 0..context.samples {
                    self.output_buffer[j] += input[j] * level;
                }
            }
        }

        // Apply master level
        self.output_buffer.scale(self.master_level);

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Mixer(mixer_param) = param {
            use crate::engine::typed_params::MixerParam;
            match mixer_param {
                MixerParam::Master => {
                    if let Some(l) = value.as_float() {
                        self.master_level = l.clamp(0.0, 2.0);
                    }
                }
                MixerParam::Input1 => if let Some(l) = value.as_float() { self.levels[0] = l.clamp(0.0, 2.0); }
                MixerParam::Input2 => if let Some(l) = value.as_float() { self.levels[1] = l.clamp(0.0, 2.0); }
                MixerParam::Input3 => if let Some(l) = value.as_float() { self.levels[2] = l.clamp(0.0, 2.0); }
                MixerParam::Input4 => if let Some(l) = value.as_float() { self.levels[3] = l.clamp(0.0, 2.0); }
                MixerParam::Mute | MixerParam::Limit => {} // Not used by Mixer
            }
        }
    }
    
    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Mixer(mixer_param) = param {
            use crate::engine::typed_params::MixerParam;
            match mixer_param {
                MixerParam::Master => Some(TypedValue::Float(self.master_level)),
                MixerParam::Input1 => Some(TypedValue::Float(self.levels[0])),
                MixerParam::Input2 => Some(TypedValue::Float(self.levels[1])),
                MixerParam::Input3 => Some(TypedValue::Float(self.levels[2])),
                MixerParam::Input4 => Some(TypedValue::Float(self.levels[3])),
                MixerParam::Mute | MixerParam::Limit => None, // Not used by Mixer
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Mixer
    }

    fn reset(&mut self) {
        self.output_buffer.clear();
    }

    fn note_on(&mut self, _note: u8, _velocity: f32) {}
    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn VoiceModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amplifier_creation() {
        let amp = Amplifier::new();
        assert!((amp.level - 1.0).abs() < 0.001);
        assert!((amp.pan - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_constant_power_pan() {
        let mut amp = Amplifier::new();
        
        // Center: both channels equal
        amp.pan = 0.0;
        let (l, r) = amp.pan_coefficients();
        assert!((l - r).abs() < 0.01);
        
        // Left: left channel full, right silent
        amp.pan = -1.0;
        let (l, r) = amp.pan_coefficients();
        assert!(l > 0.99);
        assert!(r < 0.01);
        
        // Right: right channel full, left silent
        amp.pan = 1.0;
        let (l, r) = amp.pan_coefficients();
        assert!(l < 0.01);
        assert!(r > 0.99);
    }
}
