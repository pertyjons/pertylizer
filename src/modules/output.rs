//! Stereo Output Module - Final output destination for the module graph.
//!
//! The StereoOutput module serves as the final "sink" in the module graph.
//! It collects audio from connected modules and provides the final stereo
//! output with master volume, pan, and optional limiting.
//!
//! Unlike other modules, StereoOutput has no audio outputs - it's the end
//! of the signal chain. The processed audio is stored internally and can
//! be retrieved via the `get_output()` method.

use std::collections::HashMap;

use crate::engine::typed_params::{ModuleType, TypedParam, TypedValue, MixerParam};
use crate::modules::core::*;

/// Stereo output module - the final destination in the audio graph.
///
/// Features:
/// - Stereo input (left/right or mono summed to stereo)
/// - Master volume control
/// - Pan control
/// - Soft limiting to prevent clipping
/// - Peak/RMS metering (accessible via getters)
#[derive(Debug, Clone)]
pub struct StereoOutput {
    /// Master volume (0.0 - 1.0)
    master_level: f32,
    /// Master pan (-1.0 = left, 0.0 = center, 1.0 = right)
    pan: f32,
    /// Enable soft limiting
    limit_enabled: bool,
    /// Limiter threshold in dB (typically -0.1 to -3.0)
    limit_threshold: f32,
    /// Current peak level (left)
    peak_l: f32,
    /// Current peak level (right)
    peak_r: f32,
    /// Output buffer (interleaved stereo)
    output_buffer: Vec<f32>,
    /// Mute state
    muted: bool,
}

impl StereoOutput {
    /// Create a new stereo output with default settings.
    pub fn new() -> Self {
        Self {
            master_level: 0.8,
            pan: 0.0,
            limit_enabled: true,
            limit_threshold: -0.3, // dB
            peak_l: 0.0,
            peak_r: 0.0,
            output_buffer: Vec::new(),
            muted: false,
        }
    }

    /// Get the processed stereo output buffer (interleaved L, R, L, R...).
    pub fn get_output(&self) -> &[f32] {
        &self.output_buffer
    }

    /// Get the current peak levels (left, right).
    pub fn get_peak_levels(&self) -> (f32, f32) {
        (self.peak_l, self.peak_r)
    }

    /// Get the master level.
    pub fn get_master_level(&self) -> f32 {
        self.master_level
    }

    /// Set the master level.
    pub fn set_master_level(&mut self, level: f32) {
        self.master_level = level.clamp(0.0, 1.0);
    }

    /// Get mute state.
    pub fn is_muted(&self) -> bool {
        self.muted
    }

    /// Set mute state.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Apply soft limiting to prevent harsh clipping.
    fn soft_limit(&self, sample: f32) -> f32 {
        if !self.limit_enabled {
            return sample.clamp(-1.0, 1.0);
        }

        // Convert threshold from dB to linear
        let threshold = 10.0_f32.powf(self.limit_threshold / 20.0);
        
        // Soft knee limiting using tanh
        if sample.abs() > threshold {
            let sign = sample.signum();
            let excess = sample.abs() - threshold;
            let limited = threshold + (1.0 - threshold) * (excess / (1.0 + excess)).tanh();
            sign * limited
        } else {
            sample
        }
    }

    /// Calculate pan coefficients (equal power panning).
    fn pan_coefficients(&self) -> (f32, f32) {
        // Equal power panning law
        let angle = (self.pan + 1.0) * 0.25 * std::f32::consts::PI;
        (angle.cos(), angle.sin())
    }
}

impl Default for StereoOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for StereoOutput {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("stereo_output", "Stereo Output")
            .description("Master stereo output with volume, pan, and limiting")
            .category(ModuleCategory::Output)
            // Inputs - accepts both mono and stereo
            .port(PortDescriptor::audio_input("in", "In")
                .description("Mono audio input (summed to stereo)"))
            .port(PortDescriptor::audio_input("in_l", "In L")
                .description("Left channel input"))
            .port(PortDescriptor::audio_input("in_r", "In R")
                .description("Right channel input"))
            // No audio outputs - this is the final sink
            // Parameters
            .parameter(ParameterDescriptor::float(TypedParam::Mixer(MixerParam::Master), "Master Level")
                .description("Master output volume")
                .range(0.0, 1.0)
                .default(0.8)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Slider)
                .curve(ResponseCurve::Squared))
            .parameter(ParameterDescriptor::float(TypedParam::Amplifier(crate::engine::typed_params::AmplifierParam::Pan), "Pan")
                .description("Master stereo pan")
                .range(-1.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::PanKnob))
            .parameter(ParameterDescriptor::float(TypedParam::Mixer(MixerParam::Limit), "Limiter")
                .description("Enable soft limiter to prevent clipping")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Toggle))
            .parameter(ParameterDescriptor::float(TypedParam::Mixer(MixerParam::Mute), "Mute")
                .description("Mute output")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Toggle))
            .tag("output")
            .tag("master")
            .tag("stereo")
    }
}

impl VoiceModule for StereoOutput {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        // Resize output buffer if needed (interleaved stereo)
        let stereo_samples = context.samples * 2;
        if self.output_buffer.len() != stereo_samples {
            self.output_buffer.resize(stereo_samples, 0.0);
        }

        // Get input buffers
        let mono_in = inputs.get("in");
        let left_in = inputs.get("in_l");
        let right_in = inputs.get("in_r");

        // Calculate pan coefficients
        let (pan_l, pan_r) = self.pan_coefficients();

        // Reset peaks
        let mut peak_l = 0.0_f32;
        let mut peak_r = 0.0_f32;

        // Process each sample
        for i in 0..context.samples {
            // Get input samples
            let (mut left, mut right) = if let (Some(l), Some(r)) = (left_in, right_in) {
                // Stereo input
                (l[i], r[i])
            } else if let Some(mono) = mono_in {
                // Mono input - duplicate to both channels
                (mono[i], mono[i])
            } else {
                // No input - silence
                (0.0, 0.0)
            };

            // Apply mute
            if self.muted {
                left = 0.0;
                right = 0.0;
            } else {
                // Apply master level with pan
                left *= self.master_level * pan_l;
                right *= self.master_level * pan_r;

                // Apply soft limiting
                left = self.soft_limit(left);
                right = self.soft_limit(right);
            }

            // Store in interleaved output buffer
            self.output_buffer[i * 2] = left;
            self.output_buffer[i * 2 + 1] = right;

            // Track peaks
            peak_l = peak_l.max(left.abs());
            peak_r = peak_r.max(right.abs());
        }

        // Update peak meters with decay
        const DECAY: f32 = 0.95;
        self.peak_l = (self.peak_l * DECAY).max(peak_l);
        self.peak_r = (self.peak_r * DECAY).max(peak_r);

        // Also write to "out" port if present (for compatibility)
        // This allows the graph to read from this module's output
        if let Some(out_buf) = outputs.get_mut("out") {
            // Convert stereo to mono for graph routing (sum and normalize)
            for i in 0..context.samples.min(out_buf.len()) {
                let left = self.output_buffer[i * 2];
                let right = self.output_buffer.get(i * 2 + 1).copied().unwrap_or(left);
                out_buf[i] = (left + right) * 0.5;
            }
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        match param {
            TypedParam::Mixer(MixerParam::Master) => {
                if let Some(v) = value.as_float() {
                    self.master_level = v.clamp(0.0, 1.0);
                }
            }
            TypedParam::Mixer(MixerParam::Mute) => {
                if let Some(v) = value.as_float() {
                    self.muted = v > 0.5;
                }
            }
            TypedParam::Mixer(MixerParam::Limit) => {
                if let Some(v) = value.as_float() {
                    self.limit_enabled = v > 0.5;
                }
            }
            TypedParam::Amplifier(crate::engine::typed_params::AmplifierParam::Pan) => {
                if let Some(v) = value.as_float() {
                    self.pan = v.clamp(-1.0, 1.0);
                }
            }
            _ => {}
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        match param {
            TypedParam::Mixer(MixerParam::Master) => Some(TypedValue::Float(self.master_level)),
            TypedParam::Mixer(MixerParam::Mute) => Some(TypedValue::Float(if self.muted { 1.0 } else { 0.0 })),
            TypedParam::Mixer(MixerParam::Limit) => Some(TypedValue::Float(if self.limit_enabled { 1.0 } else { 0.0 })),
            TypedParam::Amplifier(crate::engine::typed_params::AmplifierParam::Pan) => Some(TypedValue::Float(self.pan)),
            _ => None,
        }
    }

    fn module_type(&self) -> ModuleType {
        // Treat as Mixer type for compatibility (Output category)
        ModuleType::Mixer
    }

    fn reset(&mut self) {
        self.peak_l = 0.0;
        self.peak_r = 0.0;
        self.output_buffer.clear();
    }

    fn box_clone(&self) -> Box<dyn VoiceModule> {
        Box::new(self.clone())
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stereo_output_creation() {
        let output = StereoOutput::new();
        assert_eq!(output.master_level, 0.8);
        assert_eq!(output.pan, 0.0);
        assert!(output.limit_enabled);
        assert!(!output.muted);
    }

    #[test]
    fn test_soft_limiting() {
        let output = StereoOutput::new();
        
        // Values below threshold should pass through
        assert!((output.soft_limit(0.5) - 0.5).abs() < 0.01);
        
        // Values above 1.0 should be limited
        let limited = output.soft_limit(2.0);
        assert!(limited <= 1.0);
        assert!(limited > 0.9);
    }

    #[test]
    fn test_pan_coefficients() {
        let mut output = StereoOutput::new();
        
        // Center pan should be equal
        output.pan = 0.0;
        let (l, r) = output.pan_coefficients();
        assert!((l - r).abs() < 0.01);
        
        // Full left should have higher left coefficient
        output.pan = -1.0;
        let (l, r) = output.pan_coefficients();
        assert!(l > r);
        
        // Full right should have higher right coefficient
        output.pan = 1.0;
        let (l, r) = output.pan_coefficients();
        assert!(r > l);
    }

    #[test]
    fn test_mute() {
        let mut output = StereoOutput::new();
        let context = ProcessContext::default();
        
        // Create test input
        let mut test_buf = AudioBuffer::new(context.samples);
        for i in 0..context.samples {
            test_buf[i] = 0.5;
        }
        
        let mut inputs = HashMap::new();
        inputs.insert("in".to_string(), &test_buf);
        let mut outputs = HashMap::new();
        
        // Process unmuted
        output.process(&inputs, &mut outputs, &context);
        let unmuted_output = output.get_output().to_vec();
        
        // Process muted
        output.set_muted(true);
        output.process(&inputs, &mut outputs, &context);
        let muted_output = output.get_output();
        
        // Muted should be silence
        assert!(muted_output.iter().all(|&s| s == 0.0));
        // Unmuted should have signal
        assert!(unmuted_output.iter().any(|&s| s != 0.0));
    }
}
