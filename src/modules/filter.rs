//! State Variable Filter module.
//!
//! Features:
//! - Multiple filter types (LP, HP, BP, Notch, Peak, Shelving)
//! - Resonance up to self-oscillation
//! - Cutoff and resonance modulation inputs
//! - Key tracking

use std::collections::HashMap;
use std::f32::consts::PI;

use crate::engine::typed_params::{TypedParam, TypedValue, FilterParam, ModuleType};
use crate::modules::core::*;

/// State Variable Filter with multiple modes.
#[derive(Clone)]
pub struct Filter {
    // Parameters
    filter_type: FilterType,
    cutoff: f32,        // Hz
    resonance: f32,     // 0.0 - 1.0
    key_tracking: f32,  // 0.0 - 1.0
    env_amount: f32,    // -1.0 to 1.0 (scales envelope CV)

    // State
    sample_rate: f32,

    // SVF state variables
    ic1eq: f32,
    ic2eq: f32,

    // For key tracking
    base_note: u8,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl Filter {
    pub fn new() -> Self {
        Self {
            filter_type: FilterType::Lowpass,
            cutoff: 1000.0,
            resonance: 0.0,
            key_tracking: 0.0,
            env_amount: 1.0,  // Full positive envelope amount by default
            sample_rate: 48000.0,
            ic1eq: 0.0,
            ic2eq: 0.0,
            base_note: 60,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Calculate the effective cutoff frequency with key tracking.
    fn effective_cutoff(&self) -> f32 {
        let tracking_offset = (self.base_note as f32 - 60.0) * self.key_tracking * 100.0;
        // exp2 is faster than powf(2.0, x)
        let tracked = self.cutoff * (tracking_offset / 1200.0).exp2();
        tracked.clamp(20.0, self.sample_rate * 0.49)
    }

    /// Process a single sample through the SVF.
    #[inline]
    fn process_sample(&mut self, input: f32, cutoff_mod: f32, res_mod: f32) -> f32 {
        // Calculate coefficients
        let cutoff = (self.effective_cutoff() * (1.0 + cutoff_mod)).clamp(20.0, self.sample_rate * 0.49);
        let resonance = (self.resonance + res_mod).clamp(0.0, 1.0);
        
        // SVF coefficients
        let g = (PI * cutoff / self.sample_rate).tan();
        let k = 2.0 - 2.0 * resonance; // k = 2 - 2*Q, where Q ranges from 0.5 to inf
        
        // SVF equations (Cytomic/Vadim Zavalishin style)
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        
        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;
        
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        
        // Output based on filter type
        match self.filter_type {
            FilterType::Lowpass => v2,
            FilterType::Highpass => input - k * v1 - v2,
            FilterType::Bandpass => v1,
            FilterType::Notch => input - k * v1,
            FilterType::Peak => {
                let lp = v2;
                let hp = input - k * v1 - v2;
                lp - hp
            }
            FilterType::LowShelf => {
                // Simplified shelf - mix of input and lowpass
                let lp = v2;
                input * 0.5 + lp * 0.5
            }
            FilterType::HighShelf => {
                // Simplified shelf - mix of input and highpass
                let hp = input - k * v1 - v2;
                input * 0.5 + hp * 0.5
            }
        }
    }
}

impl Default for Filter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Filter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("filter", "Filter")
            .description("State Variable Filter with multiple modes")
            .category(ModuleCategory::Filter)
            .tag("filter")
            .tag("svf")
            // Parameters
            .parameter(
                ParameterDescriptor::choice(
                    TypedParam::Filter(FilterParam::Mode),
                    "Type",
                    FilterType::to_choices(),
                )
                .description("Filter type"),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Filter(FilterParam::Cutoff), "Cutoff")
                    .description("Cutoff frequency")
                    .range(20.0, 20000.0)
                    .default(1000.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::FrequencySlider)
                    .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Filter(FilterParam::Resonance), "Resonance")
                    .description("Filter resonance (Q)")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Filter(FilterParam::KeyTracking), "Key Track")
                    .description("Keyboard tracking amount")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .unit(ParameterUnit::Percent)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Filter(FilterParam::EnvAmount), "Env Amt")
                    .description("Envelope modulation amount (-1 to +1, scales cutoff CV)")
                    .range(-1.0, 1.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
            // Ports
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
            .port(PortDescriptor::control_input("cutoff_cv", "Cutoff CV").description("Cutoff modulation (scaled by Env Amount)"))
            .port(PortDescriptor::control_input("res_cv", "Res CV").description("Resonance modulation"))
            .port(PortDescriptor::audio_output("out", "Out").description("Filtered output"))
    }
}

impl VoiceModule for Filter {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        let audio_in = inputs.get("in");
        let cutoff_cv = inputs.get("cutoff_cv");
        let res_cv = inputs.get("res_cv");

        for i in 0..context.samples {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);
            // Scale cutoff CV by env_amount (allows negative/inverted envelopes)
            let cutoff_mod = cutoff_cv.map(|b| b[i] * self.env_amount).unwrap_or(0.0);
            let res_mod = res_cv.map(|b| b[i]).unwrap_or(0.0);

            self.output_buffer[i] = self.process_sample(input, cutoff_mod, res_mod);
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Filter(filter_param) = param {
            match filter_param {
                FilterParam::Mode => {
                    if let TypedValue::FilterMode(mode) = value {
                        // Direct assignment - FilterType is now alias to FilterMode
                        self.filter_type = mode;
                    }
                }
                FilterParam::Cutoff => {
                    if let Some(c) = value.as_float() {
                        self.cutoff = c.clamp(20.0, 20000.0);
                    }
                }
                FilterParam::Resonance => {
                    if let Some(r) = value.as_float() {
                        self.resonance = r.clamp(0.0, 1.0);
                    }
                }
                FilterParam::KeyTracking => {
                    if let Some(k) = value.as_float() {
                        self.key_tracking = k.clamp(0.0, 1.0);
                    }
                }
                FilterParam::Drive => {
                    // Not implemented in basic SVF filter
                }
                FilterParam::EnvAmount => {
                    if let Some(e) = value.as_float() {
                        self.env_amount = e.clamp(-1.0, 1.0);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Filter(filter_param) = param {
            match filter_param {
                FilterParam::Mode => {
                    // Direct use - same type now!
                    Some(TypedValue::FilterMode(self.filter_type))
                }
                FilterParam::Cutoff => Some(TypedValue::Float(self.cutoff)),
                FilterParam::Resonance => Some(TypedValue::Float(self.resonance)),
                FilterParam::KeyTracking => Some(TypedValue::Float(self.key_tracking)),
                FilterParam::Drive => None,
                FilterParam::EnvAmount => Some(TypedValue::Float(self.env_amount)),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Filter
    }

    fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    fn note_on(&mut self, note: u8, _velocity: f32) {
        self.base_note = note;
    }

    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn VoiceModule> {
        Box::new(self.clone())
    }
}

/// Moog-style 24dB/oct ladder filter.
#[derive(Clone)]
pub struct LadderFilter {
    // Parameters
    cutoff: f32,
    resonance: f32,
    drive: f32,
    
    // State
    sample_rate: f32,
    stage: [f32; 4],
    delay: [f32; 4],
    
    // Output
    output_buffer: AudioBuffer,
}

impl LadderFilter {
    pub fn new() -> Self {
        Self {
            cutoff: 1000.0,
            resonance: 0.0,
            drive: 0.0,
            sample_rate: 48000.0,
            stage: [0.0; 4],
            delay: [0.0; 4],
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Soft clipping saturation.
    #[inline]
    fn saturate(x: f32) -> f32 {
        x.tanh()
    }

    /// Process a single sample.
    #[inline]
    fn process_sample(&mut self, input: f32) -> f32 {
        let cutoff = self.cutoff.clamp(20.0, self.sample_rate * 0.49);
        let g = (PI * cutoff / self.sample_rate).tan();
        let k = self.resonance * 4.0;
        
        // Drive/saturation
        let driven = if self.drive > 0.0 {
            Self::saturate(input * (1.0 + self.drive * 3.0))
        } else {
            input
        };
        
        // Feedback with delay compensation
        let feedback = k * self.delay[3];
        let input_with_fb = driven - feedback;
        
        // Four cascaded one-pole lowpass filters
        for i in 0..4 {
            let prev = if i == 0 { input_with_fb } else { self.stage[i - 1] };
            self.stage[i] = (prev - self.delay[i]) * g / (1.0 + g) + self.delay[i];
            self.delay[i] = self.stage[i];
            
            // Add saturation between stages for character
            if self.drive > 0.0 {
                self.stage[i] = Self::saturate(self.stage[i]);
            }
        }
        
        self.stage[3]
    }
}

impl Default for LadderFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for LadderFilter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("ladder_filter", "Ladder Filter")
            .description("24dB/oct Moog-style ladder filter")
            .category(ModuleCategory::Filter)
            .tag("filter")
            .tag("moog")
            .tag("ladder")
            .parameter(
                ParameterDescriptor::float(TypedParam::Filter(FilterParam::Cutoff), "Cutoff")
                    .description("Cutoff frequency")
                    .range(20.0, 20000.0)
                    .default(1000.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::FrequencySlider)
                    .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Filter(FilterParam::Resonance), "Resonance")
                    .description("Filter resonance")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Filter(FilterParam::Drive), "Drive")
                    .description("Saturation amount")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In"))
            .port(PortDescriptor::control_input("cutoff_cv", "Cutoff CV"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}

impl VoiceModule for LadderFilter {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        let audio_in = inputs.get("in");
        let cutoff_cv = inputs.get("cutoff_cv");

        for i in 0..context.samples {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);
            
            // Apply cutoff modulation
            if let Some(cv) = cutoff_cv {
                let mod_amount = cv[i];
                // Exponential modulation (exp2 is faster than powf)
                self.cutoff = (self.cutoff * (mod_amount * 4.0).exp2()).clamp(20.0, 20000.0);
            }

            self.output_buffer[i] = self.process_sample(input);
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Filter(filter_param) = param {
            match filter_param {
                FilterParam::Cutoff => {
                    if let Some(c) = value.as_float() {
                        self.cutoff = c.clamp(20.0, 20000.0);
                    }
                }
                FilterParam::Resonance => {
                    if let Some(r) = value.as_float() {
                        self.resonance = r.clamp(0.0, 1.0);
                    }
                }
                FilterParam::Drive => {
                    if let Some(d) = value.as_float() {
                        self.drive = d.clamp(0.0, 1.0);
                    }
                }
                _ => {}
            }
        }
    }
    
    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Filter(filter_param) = param {
            match filter_param {
                FilterParam::Cutoff => Some(TypedValue::Float(self.cutoff)),
                FilterParam::Resonance => Some(TypedValue::Float(self.resonance)),
                FilterParam::Drive => Some(TypedValue::Float(self.drive)),
                _ => None,
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Filter
    }

    fn reset(&mut self) {
        self.stage = [0.0; 4];
        self.delay = [0.0; 4];
    }

    fn note_on(&mut self, _note: u8, _velocity: f32) {}
    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn VoiceModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_creation() {
        let filter = Filter::new();
        assert_eq!(filter.filter_type, FilterType::Lowpass);
        assert!((filter.cutoff - 1000.0).abs() < 0.001);
    }

    #[test]
    fn test_filter_stability() {
        let mut filter = Filter::new();
        filter.sample_rate = 48000.0;
        filter.cutoff = 100.0;
        filter.resonance = 0.99;

        // Process some samples, check for stability
        for _ in 0..1000 {
            let out = filter.process_sample(0.5, 0.0, 0.0);
            assert!(out.is_finite(), "Filter output is not finite");
            assert!(out.abs() < 100.0, "Filter output exploded");
        }
    }
}
