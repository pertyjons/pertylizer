//! Core traits and types for the module system.
//!
//! This module defines the fundamental abstractions for all synth modules:
//! - `Module` trait for audio processing
//! - `Describable` trait for UI introspection
//! - Parameter descriptors with widget hints
//! - Port definitions for routing

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::engine::ModuleTypeId;
use crate::engine::typed_params::{ModuleType as TypedModuleType, Param};
use crate::types::{Bpm, MidiNote, SampleRate};

// ============================================================================
// Buffer types
// ============================================================================

/// A buffer of audio samples.
#[derive(Clone, Default)]
pub struct AudioBuffer {
    samples: Vec<f32>,
}

impl AudioBuffer {
    /// Create a new buffer with the given size.
    pub fn new(size: usize) -> Self {
        Self {
            samples: vec![0.0; size],
        }
    }

    /// Get the buffer length.
    #[inline]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Clear the buffer to silence.
    #[inline]
    pub fn clear(&mut self) {
        self.samples.fill(0.0);
    }

    /// Get a slice of the samples.
    #[inline]
    pub fn as_slice(&self) -> &[f32] {
        &self.samples
    }

    /// Get a mutable slice of the samples.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        &mut self.samples
    }

    /// Resize the buffer.
    pub fn resize(&mut self, new_size: usize) {
        self.samples.resize(new_size, 0.0);
    }

    /// Copy from another buffer.
    pub fn copy_from(&mut self, other: &AudioBuffer) {
        let len = self.samples.len().min(other.samples.len());
        self.samples[..len].copy_from_slice(&other.samples[..len]);
    }

    /// Add another buffer to this one.
    pub fn add_from(&mut self, other: &AudioBuffer) {
        for (dst, src) in self.samples.iter_mut().zip(other.samples.iter()) {
            *dst += *src;
        }
    }

    /// Multiply by a scalar.
    pub fn scale(&mut self, factor: f32) {
        for sample in &mut self.samples {
            *sample *= factor;
        }
    }
}

impl std::ops::Index<usize> for AudioBuffer {
    type Output = f32;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.samples[index]
    }
}

impl std::ops::IndexMut<usize> for AudioBuffer {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.samples[index]
    }
}

// ============================================================================
// Zero-allocation port wrappers for realtime-safe processing
// ============================================================================

use crate::types::PortName;

/// Zero-allocation wrapper for looking up input buffers by port name.
///
/// Used in `PolyModule::process()` to avoid HashMap allocation every audio frame.
/// Provides O(n) linear search which is fast for the typical 1-4 input ports.
#[derive(Clone, Copy)]
pub struct InputPorts<'a>(&'a [(PortName, &'a AudioBuffer)]);

impl<'a> InputPorts<'a> {
    /// Create a new InputPorts wrapper from a slice.
    #[inline]
    pub fn new(ports: &'a [(PortName, &'a AudioBuffer)]) -> Self {
        Self(ports)
    }

    /// Create an empty InputPorts (no inputs connected).
    #[inline]
    pub fn empty() -> Self {
        Self(&[])
    }

    /// Get an input buffer by port name.
    ///
    /// Returns `None` if no input is connected to this port.
    /// O(n) linear search, but n is typically 1-4 ports.
    #[inline]
    pub fn get(&self, name: &str) -> Option<&AudioBuffer> {
        self.0
            .iter()
            .find(|(n, _)| n.as_str() == name)
            .map(|(_, buf)| *buf)
    }

    /// Check if any inputs are connected.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get the number of connected inputs.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

// ============================================================================
// Processing context
// ============================================================================

/// Context passed to modules during processing.
#[derive(Debug, Clone, Copy)]
pub struct ProcessContext {
    /// Sample rate (type-safe Hz).
    pub sample_rate: SampleRate,
    /// Number of samples to process.
    pub samples: usize,
    /// Current tempo (type-safe BPM).
    pub tempo: Bpm,
    /// Is transport playing.
    pub is_playing: bool,
    /// Current position in beats.
    pub position_beats: f64,
}

impl Default for ProcessContext {
    fn default() -> Self {
        Self {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: 256,
            tempo: Bpm::DEFAULT,
            is_playing: false,
            position_beats: 0.0,
        }
    }
}

// ============================================================================
// Parameter descriptors
// ============================================================================

/// Hint for what type of UI widget to use.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WidgetHint {
    /// Standard slider.
    Slider,
    /// Rotary knob.
    Knob,
    /// On/off toggle.
    Toggle,
    /// Dropdown menu.
    Dropdown,
    /// XY pad for 2D control.
    XYPad,
    /// Waveform selector with visual preview.
    WaveformSelector,
    /// ADSR envelope editor.
    EnvelopeEditor,
    /// Piano keyboard.
    PianoKeyboard,
    /// Custom waveform editor.
    WaveEditor,
    /// Frequency with logarithmic scale.
    FrequencySlider,
    /// Time value (ms or seconds).
    TimeSlider,
    /// Percentage (0-100%).
    PercentSlider,
    /// Decibel scale.
    DecibelSlider,
    /// Pan control (-1 to +1).
    PanKnob,
}

/// Response curve for parameter mapping.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ResponseCurve {
    /// Linear mapping.
    Linear,
    /// Logarithmic (good for frequency).
    Logarithmic,
    /// Exponential (good for time).
    Exponential,
    /// S-curve (smooth transitions).
    SCurve,
    /// Squared (good for volume).
    Squared,
}

impl ResponseCurve {
    /// Convert a normalized value (0.0-1.0) to the actual value range.
    pub fn denormalize(&self, normalized: f32, min: f32, max: f32) -> f32 {
        let n = normalized.clamp(0.0, 1.0);
        match self {
            Self::Linear => min + n * (max - min),
            Self::Logarithmic => {
                // For frequency-like parameters
                if min <= 0.0 {
                    min + n * (max - min) // Fall back to linear if min <= 0
                } else {
                    min * (max / min).powf(n)
                }
            }
            Self::Exponential => {
                // Fast start, slow end
                let curved = n * n;
                min + curved * (max - min)
            }
            Self::SCurve => {
                // Smooth S-curve using smoothstep
                let curved = n * n * (3.0 - 2.0 * n);
                min + curved * (max - min)
            }
            Self::Squared => {
                // Good for volume/amplitude
                let curved = n * n;
                min + curved * (max - min)
            }
        }
    }

    /// Convert an actual value to normalized (0.0-1.0).
    pub fn normalize(&self, value: f32, min: f32, max: f32) -> f32 {
        if (max - min).abs() < f32::EPSILON {
            return 0.0;
        }
        let clamped = value.clamp(min, max);
        match self {
            Self::Linear => (clamped - min) / (max - min),
            Self::Logarithmic => {
                if min <= 0.0 {
                    (clamped - min) / (max - min)
                } else {
                    (clamped / min).ln() / (max / min).ln()
                }
            }
            Self::Exponential => {
                let linear = (clamped - min) / (max - min);
                linear.sqrt()
            }
            Self::SCurve => {
                // Inverse smoothstep approximation using Newton-Raphson
                let linear = (clamped - min) / (max - min);
                // Simple approximation
                if linear <= 0.5 {
                    (0.5 * (2.0 * linear)).sqrt() * 0.5
                } else {
                    1.0 - (0.5 * (2.0 * (1.0 - linear))).sqrt() * 0.5
                }
            }
            Self::Squared => {
                let linear = (clamped - min) / (max - min);
                linear.sqrt()
            }
        }
    }
}

/// Unit type for parameter values.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ParameterUnit {
    None,
    Hertz,
    Decibels,
    Percent,
    Milliseconds,
    Seconds,
    Semitones,
    Cents,
    Octaves,
    Beats,
    BeatsPerMinute,
    Samples,
    Ratio,
}

impl ParameterUnit {
    /// Get the unit suffix string.
    pub fn suffix(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::Hertz => " Hz",
            Self::Decibels => " dB",
            Self::Percent => "%",
            Self::Milliseconds => " ms",
            Self::Seconds => " s",
            Self::Semitones => " st",
            Self::Cents => " ct",
            Self::Octaves => " oct",
            Self::Beats => " beats",
            Self::BeatsPerMinute => " BPM",
            Self::Samples => " smp",
            Self::Ratio => ":1",
        }
    }

    /// Format a value with this unit for display.
    pub fn format(&self, value: f32) -> String {
        match self {
            Self::Hertz => {
                if value >= 1000.0 {
                    format!("{:.2} kHz", value / 1000.0)
                } else {
                    format!("{:.1} Hz", value)
                }
            }
            Self::Decibels => format!("{:.1} dB", value),
            Self::Percent => format!("{:.0}%", value * 100.0),
            Self::Milliseconds => format!("{:.1} ms", value),
            Self::Seconds => format!("{:.2} s", value),
            Self::Semitones => format!("{:.0} st", value),
            Self::Cents => format!("{:.0} ct", value),
            Self::Octaves => format!("{:.1} oct", value),
            Self::Beats => format!("{:.1} beats", value),
            Self::BeatsPerMinute => format!("{:.0} BPM", value),
            Self::Samples => format!("{:.0} smp", value),
            Self::Ratio => format!("{:.1}:1", value),
            Self::None => format!("{:.2}", value),
        }
    }
}

/// A choice option for dropdown parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceOption {
    /// String ID (matches enum variant name).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
}

impl ChoiceOption {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// Complete description of a parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterDescriptor {
    /// Parameter with default value (identifies both type and default).
    pub id: Param,
    /// Display name.
    pub name: String,
    /// Description for tooltips.
    pub description: String,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
    /// Default value.
    pub default: f32,
    /// Unit type.
    pub unit: ParameterUnit,
    /// Widget hint for UI.
    pub widget_hint: WidgetHint,
    /// Response curve.
    pub response_curve: ResponseCurve,
    /// For choice parameters, the available options.
    pub choices: Option<Vec<ChoiceOption>>,
    /// Can this parameter be modulated.
    pub modulatable: bool,
}

impl ParameterDescriptor {
    /// Create a new float parameter descriptor.
    /// The id parameter should contain the default value.
    pub fn float(id: Param, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: String::new(),
            min: 0.0,
            max: 1.0,
            default: 0.5,
            unit: ParameterUnit::None,
            widget_hint: WidgetHint::Knob,
            response_curve: ResponseCurve::Linear,
            choices: None,
            modulatable: true,
        }
    }

    /// Create a choice parameter descriptor.
    /// The id parameter should contain the default choice value.
    pub fn choice(id: Param, name: impl Into<String>, choices: Vec<ChoiceOption>) -> Self {
        let max = (choices.len().saturating_sub(1)) as f32;
        Self {
            id,
            name: name.into(),
            description: String::new(),
            min: 0.0,
            max,
            default: 0.0,
            unit: ParameterUnit::None,
            widget_hint: WidgetHint::Dropdown,
            response_curve: ResponseCurve::Linear,
            choices: Some(choices),
            modulatable: false,
        }
    }

    // Builder methods
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    pub fn default(mut self, value: f32) -> Self {
        self.default = value;
        self
    }

    pub fn unit(mut self, unit: ParameterUnit) -> Self {
        self.unit = unit;
        self
    }

    pub fn widget(mut self, hint: WidgetHint) -> Self {
        self.widget_hint = hint;
        self
    }

    pub fn curve(mut self, curve: ResponseCurve) -> Self {
        self.response_curve = curve;
        self
    }

    pub fn modulatable(mut self, can_modulate: bool) -> Self {
        self.modulatable = can_modulate;
        self
    }

    /// Map a normalized value (0-1) to the parameter range.
    pub fn denormalize(&self, normalized: f32) -> f32 {
        let n = normalized.clamp(0.0, 1.0);
        match self.response_curve {
            ResponseCurve::Linear => self.min + n * (self.max - self.min),
            ResponseCurve::Logarithmic => {
                let min_log = self.min.max(0.001).ln();
                let max_log = self.max.ln();
                (min_log + n * (max_log - min_log)).exp()
            }
            ResponseCurve::Exponential => self.min + (n * n) * (self.max - self.min),
            ResponseCurve::SCurve => {
                let s = n * n * (3.0 - 2.0 * n);
                self.min + s * (self.max - self.min)
            }
            ResponseCurve::Squared => self.min + n.sqrt() * (self.max - self.min),
        }
    }

    /// Map a value to normalized (0-1).
    pub fn normalize(&self, value: f32) -> f32 {
        let v = value.clamp(self.min, self.max);
        match self.response_curve {
            ResponseCurve::Linear => (v - self.min) / (self.max - self.min),
            ResponseCurve::Logarithmic => {
                let min_log = self.min.max(0.001).ln();
                let max_log = self.max.ln();
                (v.ln() - min_log) / (max_log - min_log)
            }
            ResponseCurve::Exponential => ((v - self.min) / (self.max - self.min)).sqrt(),
            ResponseCurve::SCurve => {
                // Approximate inverse
                let n = (v - self.min) / (self.max - self.min);
                // Newton-Raphson would be better, but this is close enough
                n.sqrt()
            }
            ResponseCurve::Squared => {
                let n = (v - self.min) / (self.max - self.min);
                n * n
            }
        }
    }

    /// Format a value for display.
    pub fn format(&self, value: f32) -> String {
        if let Some(ref choices) = self.choices {
            let idx = value.round() as usize;
            if let Some(choice) = choices.get(idx) {
                return choice.name.clone();
            }
        }

        self.unit.format(value)
    }
}

// ============================================================================
// Port descriptors
// ============================================================================

/// Type of port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortType {
    /// Audio signal (typically -1 to +1).
    Audio,
    /// Control signal (typically 0 to 1 or -1 to +1).
    Control,
    /// Gate/trigger signal (0 or 1).
    Gate,
    /// MIDI data.
    Midi,
}

/// Direction of a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortDirection {
    Input,
    Output,
}

/// Description of a port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortDescriptor {
    /// Port name (unique within the module).
    pub name: String,
    /// Display label.
    pub label: String,
    /// Description.
    pub description: String,
    /// Port type.
    pub port_type: PortType,
    /// Direction.
    pub direction: PortDirection,
}

impl PortDescriptor {
    pub fn audio_input(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Audio,
            direction: PortDirection::Input,
        }
    }

    pub fn audio_output(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Audio,
            direction: PortDirection::Output,
        }
    }

    pub fn control_input(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Control,
            direction: PortDirection::Input,
        }
    }

    pub fn gate_input(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Gate,
            direction: PortDirection::Input,
        }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }
}

// ============================================================================
// Module descriptor
// ============================================================================

/// Category for organizing modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModuleCategory {
    Oscillator,
    Filter,
    Envelope,
    LFO,
    Amplifier,
    Effect,
    Utility,
    Sampler,
    Sequencer,
    Mixer,
    Output,
    Visualizer,
}

/// Complete description of a module for UI generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    /// Module type identifier.
    pub type_id: ModuleTypeId,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Category.
    pub category: ModuleCategory,
    /// All parameters.
    pub parameters: Vec<ParameterDescriptor>,
    /// All ports.
    pub ports: Vec<PortDescriptor>,
    /// Tags for search.
    pub tags: Vec<String>,
}

impl ModuleDescriptor {
    pub fn new(type_id: impl Into<ModuleTypeId>, name: impl Into<String>) -> Self {
        Self {
            type_id: type_id.into(),
            name: name.into(),
            description: String::new(),
            category: ModuleCategory::Utility,
            parameters: Vec::new(),
            ports: Vec::new(),
            tags: Vec::new(),
        }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn category(mut self, cat: ModuleCategory) -> Self {
        self.category = cat;
        self
    }

    pub fn parameter(mut self, param: ParameterDescriptor) -> Self {
        self.parameters.push(param);
        self
    }

    pub fn port(mut self, port: PortDescriptor) -> Self {
        self.ports.push(port);
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
}

// ============================================================================
// Module traits
// ============================================================================

/// Trait for self-describing modules.
pub trait Describable {
    /// Get the module descriptor for UI generation.
    fn descriptor(&self) -> ModuleDescriptor;
}

/// Trait for voice modules (oscillators, filters, envelopes).
///
/// Voice modules are instantiated per-voice in a polyphonic synth.
pub trait PolyModule: Describable + Send {
    /// Process audio.
    ///
    /// # Arguments
    /// * `inputs` - Zero-allocation wrapper for input buffers (use `inputs.get("port_name")`)
    /// * `outputs` - Map of output port name to buffer (to fill)
    /// * `context` - Processing context
    ///
    /// # Realtime Safety
    /// The `inputs` parameter uses `InputPorts` instead of `HashMap` to avoid
    /// heap allocation on every audio frame. This is critical for low-latency
    /// audio processing.
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    );

    /// Set a parameter value.
    /// The Param contains both the parameter type and its value.
    fn set_param(&mut self, param: Param);

    /// Get current parameter value.
    /// Pass a Param with any value to identify which parameter to get.
    /// Returns the current value as f32, or None if not found.
    fn get_param(&self, param: &Param) -> Option<f32>;

    /// Get all current parameters with their values.
    fn get_params(&self) -> Vec<Param>;

    /// Get the typed module type for this module.
    fn module_type(&self) -> TypedModuleType;

    /// Reset the module state.
    fn reset(&mut self);

    /// Trigger note on.
    fn note_on(&mut self, _note: MidiNote, _velocity: f32) {}

    /// Trigger note off.
    fn note_off(&mut self) {}

    /// Set the sample rate for this module.
    /// Called when the module is added to a graph or when the sample rate changes.
    fn set_sample_rate(&mut self, _sample_rate: f32) {
        // Default implementation does nothing.
        // Override in modules that need sample rate (oscillators, filters, etc.)
    }

    /// Clone into a boxed trait object.
    fn box_clone(&self) -> Box<dyn PolyModule>;
}

/// Trait for effect modules (delay, reverb, etc.).
///
/// Effect modules process the mixed output of all voices.
pub trait AudioEffect: Describable + Send {
    /// Process audio in-place or with separate input/output.
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext);

    /// Set a parameter value.
    /// The Param contains both the parameter type and its value.
    fn set_param(&mut self, param: Param);

    /// Get current parameter value.
    /// Pass a Param with any value to identify which parameter to get.
    /// Returns the current value as f32, or None if not found.
    fn get_param(&self, param: &Param) -> Option<f32>;

    /// Get all current parameters with their values.
    fn get_params(&self) -> Vec<Param>;

    /// Get the typed module type for this effect.
    fn module_type(&self) -> TypedModuleType;

    /// Reset the effect state (clear delay lines, etc.).
    fn reset(&mut self);

    /// Set wet/dry mix.
    fn set_mix(&mut self, mix: f32);

    /// Get current mix.
    fn get_mix(&self) -> f32;

    /// Get the tail length in samples (for reverbs, delays).
    fn tail_samples(&self) -> usize {
        0
    }

    /// Set the sample rate for this effect.
    fn set_sample_rate(&mut self, _sample_rate: f32) {
        // Default implementation does nothing.
        // Override in effects that need sample rate (delays, reverbs, etc.)
    }
}

// ============================================================================
// Waveform types - Re-exported from typed_params for single source of truth
// ============================================================================

// Re-export waveform and filter types from typed_params
pub use crate::engine::typed_params::{
    DelayMode, DistortionMode, FilterMode, LfoWaveform, LoopMode, MathAlgo, Waveform,
};

// Type alias for backward compatibility
pub type FilterType = FilterMode;

// Helper to create ChoiceOption lists from enums
impl Waveform {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|w| ChoiceOption::new(w.id(), w.name()))
            .collect()
    }
}

impl LfoWaveform {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|w| ChoiceOption::new(w.id(), w.name()))
            .collect()
    }
}

impl FilterMode {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|f| ChoiceOption::new(f.id(), f.name()))
            .collect()
    }
}

// Note: MathAlgo::to_choices is defined in engine/params/oscillators.rs
