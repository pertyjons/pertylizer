//! Core traits and types for the module system.
//!
//! This module defines the fundamental abstractions for all synth modules:
//! - `Module` trait for audio processing
//! - `Describable` trait for UI introspection
//! - Parameter descriptors with widget hints
//! - Port definitions for routing

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::params::{ModuleType, Param};
pub use crate::types::{
    BeatPosition, Bpm, MidiNote, NormalizedValue, SampleCount, SamplePosition, SampleRate,
    ValueRange, Velocity,
};

// ============================================================================
// Module Type ID
// ============================================================================

/// Unique identifier for a module type.
///
/// This is a string-based type identifier used for:
/// - Serialization/deserialization of module configurations
/// - Factory pattern for creating module instances
/// - Module registration
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModuleTypeId(pub String);

impl ModuleTypeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Well-known module type IDs.
    pub const OSCILLATOR: &'static str = "oscillator";
    pub const FILTER: &'static str = "filter";
    pub const ENVELOPE: &'static str = "envelope";
    pub const LFO: &'static str = "lfo";
    pub const AMPLIFIER: &'static str = "amplifier";
    pub const MIXER: &'static str = "mixer";
    pub const DELAY: &'static str = "delay";
    pub const REVERB: &'static str = "reverb";
    pub const CHORUS: &'static str = "chorus";
    pub const DISTORTION: &'static str = "distortion";
}

impl<S: AsRef<str>> From<S> for ModuleTypeId {
    fn from(s: S) -> Self {
        Self(s.as_ref().to_string())
    }
}

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
    pub fn copy_from(&mut self, other: &Self) {
        let len = self.samples.len().min(other.samples.len());
        self.samples[..len].copy_from_slice(&other.samples[..len]);
    }

    /// Add another buffer to this one.
    pub fn add_from(&mut self, other: &Self) {
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
///
/// Supports two storage modes: reference slices `&[(PortName, &AudioBuffer)]`
/// and owned slices `&[(PortName, AudioBuffer)]`, avoiding heap allocation
/// when the graph already stores owned buffers.
#[derive(Clone, Copy)]
pub struct InputPorts<'a>(InputPortsInner<'a>);

#[derive(Clone, Copy)]
enum InputPortsInner<'a> {
    /// Slice of (name, reference) pairs — used by tests and external callers.
    Refs(&'a [(PortName, &'a AudioBuffer)]),
    /// Slice of (name, owned buffer) pairs — used by the audio graph to avoid
    /// allocating a temporary Vec of references every frame.
    Owned(&'a [(PortName, AudioBuffer)]),
}

impl<'a> InputPorts<'a> {
    /// Create a new `InputPorts` wrapper from a slice of references.
    #[inline]
    pub fn new(ports: &'a [(PortName, &'a AudioBuffer)]) -> Self {
        Self(InputPortsInner::Refs(ports))
    }

    /// Create a new `InputPorts` wrapper from a slice of owned buffers.
    ///
    /// This avoids heap-allocating a temporary `Vec<(PortName, &AudioBuffer)>`
    /// on every audio frame, which is critical for real-time safety.
    #[inline]
    pub fn from_owned(ports: &'a [(PortName, AudioBuffer)]) -> Self {
        Self(InputPortsInner::Owned(ports))
    }

    /// Create an empty InputPorts (no inputs connected).
    #[inline]
    pub fn empty() -> Self {
        Self(InputPortsInner::Refs(&[]))
    }

    /// Get an input buffer by port name.
    ///
    /// Returns `None` if no input is connected to this port.
    /// O(n) linear search, but n is typically 1-4 ports.
    ///
    /// Uses direct `u32` comparison via `PortName` for maximum speed
    /// (no string comparison, no locking).
    #[inline]
    pub fn get(&self, name: PortName) -> Option<&'a AudioBuffer> {
        match self.0 {
            InputPortsInner::Refs(ports) => {
                ports.iter().find(|(n, _)| *n == name).map(|(_, buf)| *buf)
            }
            InputPortsInner::Owned(ports) => {
                ports.iter().find(|(n, _)| *n == name).map(|(_, buf)| buf)
            }
        }
    }

    /// Get an input buffer by port name string (convenience method).
    ///
    /// **Note:** For hot paths, prefer `get(PortName::IN)` with constants.
    /// This method is provided for backwards compatibility.
    #[inline]
    pub fn get_str(&self, name: &str) -> Option<&'a AudioBuffer> {
        match self.0 {
            InputPortsInner::Refs(ports) => ports
                .iter()
                .find(|(n, _)| n.as_str() == name)
                .map(|(_, buf)| *buf),
            InputPortsInner::Owned(ports) => ports
                .iter()
                .find(|(n, _)| n.as_str() == name)
                .map(|(_, buf)| buf),
        }
    }

    /// Check if any inputs are connected.
    #[inline]
    pub fn is_empty(&self) -> bool {
        match &self.0 {
            InputPortsInner::Refs(ports) => ports.is_empty(),
            InputPortsInner::Owned(ports) => ports.is_empty(),
        }
    }

    /// Get the number of connected inputs.
    #[inline]
    pub fn len(&self) -> usize {
        match &self.0 {
            InputPortsInner::Refs(ports) => ports.len(),
            InputPortsInner::Owned(ports) => ports.len(),
        }
    }

    /// Get a zero-cost reader for a port with a default value.
    ///
    /// Returns an [`InputReader`] that can be indexed directly, eliminating the
    /// `.map(|b| b[i]).unwrap_or(default)` pattern.
    ///
    /// # Example
    /// ```ignore
    /// let fm = inputs.reader(PortName::FM, 0.0);
    /// let cv = inputs.reader(PortName::CV, 1.0);
    /// for i in 0..samples {
    ///     output[i] = input[i] * cv[i] + fm[i];
    /// }
    /// ```
    #[inline]
    pub fn reader(&self, name: PortName, default: f32) -> InputReader<'a> {
        let buffer = self.get(name);
        InputReader { buffer, default }
    }
}

/// Zero-cost reader for a single input port.
///
/// When the port is connected, indexes into the `AudioBuffer`.
/// When unconnected, returns the default value for any index.
/// This eliminates the `.map(|b| b[i]).unwrap_or(default)` pattern.
///
/// Created via [`InputPorts::reader()`].
#[must_use]
#[derive(Clone, Copy)]
pub struct InputReader<'a> {
    buffer: Option<&'a AudioBuffer>,
    default: f32,
}

impl<'a> InputReader<'a> {
    /// Check if this port is connected.
    #[inline]
    pub fn is_connected(&self) -> bool {
        self.buffer.is_some()
    }

    /// Get the underlying buffer slice, if connected.
    #[inline]
    pub fn as_slice(&self) -> Option<&'a [f32]> {
        self.buffer.map(AudioBuffer::as_slice)
    }
}

impl std::ops::Index<usize> for InputReader<'_> {
    type Output = f32;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        match self.buffer {
            Some(buf) => &buf[index],
            None => &self.default,
        }
    }
}

// ============================================================================
// Processing context
// ============================================================================

/// Context passed to modules during processing.
#[derive(Debug, Clone, Copy)]
pub struct ProcessContext<'a> {
    /// Sample rate (type-safe Hz).
    pub sample_rate: SampleRate,
    /// Number of samples to process (type-safe).
    pub samples: SampleCount,
    /// Current tempo (type-safe BPM).
    pub tempo: Bpm,
    /// Is transport playing.
    pub is_playing: bool,
    /// Current position in beats (type-safe).
    pub position_beats: BeatPosition,
    /// Start time of the voice that is processing (for sweep arbitration).
    pub voice_start_time: SamplePosition,
    /// Live audio input buffer for the current block (`None` if no input active).
    pub audio_input: Option<&'a [f32]>,
}

impl Default for ProcessContext<'_> {
    fn default() -> Self {
        Self {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(256),
            tempo: Bpm::DEFAULT,
            is_playing: false,
            position_beats: BeatPosition::ZERO,
            voice_start_time: SamplePosition::ZERO,
            audio_input: None,
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
    #[must_use]
    pub fn denormalize(&self, normalized: f32, range: ValueRange) -> f32 {
        let n = normalized.clamp(0.0, 1.0);
        let min = range.min;
        let max = range.max;
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
                // Slow start, fast end (exponential growth)
                let base = std::f32::consts::E;
                let curved = (base.powf(n) - 1.0) / (base - 1.0);
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
    #[must_use]
    pub fn normalize(&self, value: f32, range: ValueRange) -> f32 {
        let min = range.min;
        let max = range.max;
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
                let base = std::f32::consts::E;
                (linear * (base - 1.0) + 1.0).ln()
            }
            Self::SCurve => {
                // Inverse smoothstep via Newton-Raphson iteration
                let linear = (clamped - min) / (max - min);
                let mut t = linear;
                for _ in 0..4 {
                    let f = 3.0 * t * t - 2.0 * t * t * t - linear;
                    let df = 6.0 * t - 6.0 * t * t;
                    if df.abs() > 1e-10 {
                        t -= f / df;
                    }
                    t = t.clamp(0.0, 1.0);
                }
                t
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
    /// Stable identifier used as key in JSON project files.
    /// Must never change once set — renaming breaks saved projects.
    pub type_id: String,
    /// Parameter with default value (identifies both type and default).
    pub id: Param,
    /// Display name (shown in UI, safe to rename).
    pub name: String,
    /// Description for tooltips.
    pub description: String,
    /// Value range (min, max, default).
    pub range: ValueRange,
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
    ///
    /// - `type_id`: Stable identifier used as key in JSON project files. Must
    ///   never change once set — renaming breaks saved projects.
    /// - `id`: The `Param` variant with its default value.
    /// - `name`: Display name shown in the UI (safe to rename freely).
    pub fn float(type_id: impl Into<String>, id: Param, name: impl Into<String>) -> Self {
        Self {
            type_id: type_id.into(),
            id,
            name: name.into(),
            description: String::new(),
            range: ValueRange::UNIT,
            unit: ParameterUnit::None,
            widget_hint: WidgetHint::Knob,
            response_curve: ResponseCurve::Linear,
            choices: None,
            modulatable: true,
        }
    }

    /// Create a choice parameter descriptor.
    ///
    /// - `type_id`: Stable identifier used as key in JSON project files.
    /// - `id`: The `Param` variant with its default choice value.
    /// - `name`: Display name shown in the UI.
    pub fn choice(
        type_id: impl Into<String>,
        id: Param,
        name: impl Into<String>,
        choices: Vec<ChoiceOption>,
    ) -> Self {
        let max = (choices.len().saturating_sub(1)) as f32;
        let default = id.as_f32();
        Self {
            type_id: type_id.into(),
            id,
            name: name.into(),
            description: String::new(),
            range: ValueRange::new(0.0, max, default),
            unit: ParameterUnit::None,
            widget_hint: WidgetHint::Dropdown,
            response_curve: ResponseCurve::Linear,
            choices: Some(choices),
            modulatable: false,
        }
    }

    // Builder methods
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set the value range (min, max, default).
    #[must_use]
    pub fn value_range(mut self, range: ValueRange) -> Self {
        self.range = range;
        self
    }

    /// Set min and max, keeping current default (clamped if needed).
    #[must_use]
    pub fn range(mut self, min: f32, max: f32) -> Self {
        let default = self.range.default.clamp(min, max);
        self.range = ValueRange::new(min, max, default);
        self
    }

    /// Set the default value (within current min/max).
    #[must_use]
    pub fn default(mut self, value: f32) -> Self {
        self.range = self
            .range
            .with_default(value.clamp(self.range.min, self.range.max));
        self
    }

    #[must_use]
    pub fn unit(mut self, unit: ParameterUnit) -> Self {
        self.unit = unit;
        self
    }

    #[must_use]
    pub fn widget(mut self, hint: WidgetHint) -> Self {
        self.widget_hint = hint;
        self
    }

    #[must_use]
    pub fn curve(mut self, curve: ResponseCurve) -> Self {
        self.response_curve = curve;
        self
    }

    #[must_use]
    pub fn modulatable(mut self, can_modulate: bool) -> Self {
        self.modulatable = can_modulate;
        self
    }

    /// Map a normalized value (0-1) to the parameter range.
    #[must_use]
    pub fn denormalize(&self, normalized: f32) -> f32 {
        self.response_curve.denormalize(normalized, self.range)
    }

    /// Map a value to normalized (0-1).
    #[must_use]
    pub fn normalize(&self, value: f32) -> f32 {
        self.response_curve.normalize(value, self.range)
    }

    /// Format a value for display.
    #[must_use]
    pub fn format(&self, value: f32) -> String {
        if let Some(ref choices) = self.choices {
            let idx = value.round() as usize;
            if let Some(choice) = choices.get(idx) {
                return choice.name.clone();
            }
        }

        self.unit.format(value)
    }

    // Convenience accessors for backwards compatibility
    /// Get the minimum value.
    #[inline]
    #[must_use]
    pub fn min(&self) -> f32 {
        self.range.min
    }

    /// Get the maximum value.
    #[inline]
    #[must_use]
    pub fn max(&self) -> f32 {
        self.range.max
    }

    /// Get the default value.
    #[inline]
    #[must_use]
    pub fn default_value(&self) -> f32 {
        self.range.default
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
    /// Port name (unique within the module, interned for zero-allocation).
    pub name: PortName,
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
    pub fn audio_input(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Audio,
            direction: PortDirection::Input,
        }
    }

    pub fn audio_output(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Audio,
            direction: PortDirection::Output,
        }
    }

    pub fn control_input(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Control,
            direction: PortDirection::Input,
        }
    }

    pub fn gate_input(name: impl Into<PortName>, label: impl Into<String>) -> Self {
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
    PhysicalModeling,
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
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext<'_>,
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
    fn module_type(&self) -> ModuleType;

    /// Reset the module state.
    fn reset(&mut self);

    /// Trigger note on.
    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}

    /// Trigger note off.
    fn note_off(&mut self) {}

    /// Check if this module has finished its release phase.
    /// Returns `true` for modules that don't have a release phase (default).
    /// Envelope-type modules should return `false` while still releasing.
    fn is_release_done(&self) -> bool {
        true
    }

    /// Set the sample rate for this module.
    /// Called when the module is added to a graph or when the sample rate changes.
    fn set_sample_rate(&mut self, _sample_rate: SampleRate) {
        // Default implementation does nothing.
        // Override in modules that need sample rate (oscillators, filters, etc.)
    }

    /// Apply a modulation offset from the mod matrix.
    ///
    /// `dest_index` identifies which parameter to modulate (module-specific):
    /// - Oscillator: 0 = pitch (semitones), 1 = level
    /// - Filter: 0 = cutoff (semitones), 1 = resonance
    /// - Amplifier: 0 = level, 1 = pan
    /// - LFO: 0 = rate, 1 = depth
    fn set_mod_offset(&mut self, _dest_index: u8, _value: f32) {
        // Default: no modulation support
    }

    /// Clear all modulation offsets back to zero.
    fn clear_mod_offsets(&mut self) {
        // Default: nothing to clear
    }

    /// Clone into a boxed trait object.
    fn box_clone(&self) -> Box<dyn PolyModule>;
}

/// Trait for effect modules (delay, reverb, etc.).
///
/// Effect modules process the mixed output of all voices.
pub trait AudioEffect: Describable + Send {
    /// Process audio in-place or with separate input/output.
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>);

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
    fn module_type(&self) -> ModuleType;

    /// Reset the effect state (clear delay lines, etc.).
    fn reset(&mut self);

    /// Set wet/dry mix (0.0 = dry, 1.0 = wet).
    fn set_mix(&mut self, mix: NormalizedValue);

    /// Get current mix.
    fn get_mix(&self) -> NormalizedValue;

    /// Get the tail length in samples (for reverbs, delays).
    fn tail_samples(&self) -> SampleCount {
        SampleCount::ZERO
    }

    /// Set the sample rate for this effect.
    fn set_sample_rate(&mut self, _sample_rate: SampleRate) {
        // Default implementation does nothing.
        // Override in effects that need sample rate (delays, reverbs, etc.)
    }
}

// ============================================================================
// Waveform types - Re-exported from typed_params for single source of truth
// ============================================================================

// Re-export waveform and filter types from typed_params
pub use crate::params::{
    DelayMode, DistortionMode, FilterMode, FilterModel, LfoWaveform, MathAlgo, Waveform,
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

impl FilterModel {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| ChoiceOption::new(m.id(), m.name()))
            .collect()
    }
}

// Note: MathAlgo::to_choices is defined in engine/params/oscillators.rs

// ============================================================================
// VISUALIZATION SINK TRAIT
// ============================================================================

/// Trait for writing visualization samples from the audio thread.
///
/// Implemented by `VisualizationBuffer` in `synth_engine`. Used by
/// voice-level modules (like `SignalMonitor`) that need to send
/// waveform data to the GUI without a direct dependency on `synth_engine`.
pub trait VisualizationSink: Send + Sync {
    /// Write left and right channel samples for visualization.
    ///
    /// Must be non-blocking from the audio thread (use `try_lock` internally).
    fn write_vis_samples(&self, left: &[f32], right: &[f32]);

    /// Write a complete sweep for triggered oscilloscope display.
    ///
    /// The `voice_start_time` is used for arbitration: only the most recently
    /// started voice wins. Returns `true` if the sweep was accepted.
    ///
    /// Must be non-blocking from the audio thread (use `try_lock` internally).
    fn write_sweep(&self, _samples: &[f32], _voice_start_time: u64) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_reader_connected() {
        let mut buf = AudioBuffer::new(4);
        buf[0] = 1.0;
        buf[1] = 2.0;
        buf[2] = 3.0;
        buf[3] = 4.0;

        let ports_data = [(PortName::IN, &buf)];
        let ports = InputPorts::new(&ports_data);
        let reader = ports.reader(PortName::IN, 0.0);

        assert!(reader.is_connected());
        assert_eq!(reader[0], 1.0);
        assert_eq!(reader[1], 2.0);
        assert_eq!(reader[2], 3.0);
        assert_eq!(reader[3], 4.0);
        assert!(reader.as_slice().is_some());
        assert_eq!(reader.as_slice().unwrap().len(), 4);
    }

    #[test]
    fn test_input_reader_unconnected() {
        let ports = InputPorts::empty();
        let reader = ports.reader(PortName::FM, 0.5);

        assert!(!reader.is_connected());
        assert_eq!(reader[0], 0.5);
        assert_eq!(reader[99], 0.5);
        assert!(reader.as_slice().is_none());
    }

    #[test]
    fn test_input_reader_default_zero() {
        let ports = InputPorts::empty();
        let reader = ports.reader(PortName::IN, 0.0);

        assert_eq!(reader[0], 0.0);
    }

    #[test]
    fn test_input_reader_default_one() {
        let ports = InputPorts::empty();
        let reader = ports.reader(PortName::CV, 1.0);

        assert_eq!(reader[0], 1.0);
    }
}
