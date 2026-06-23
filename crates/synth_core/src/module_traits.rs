//! Core traits and types for the module system.
//!
//! This module defines the fundamental abstractions for all synth modules:
//! - `Module` trait for audio processing
//! - `Describable` trait for UI introspection
//! - Parameter descriptors with widget hints
//! - Port definitions for routing

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::ChannelCount;
use crate::params::{ModuleType, Param};
pub use crate::types::{
    BeatPosition, Bpm, Hertz, MidiNote, NormalizedValue, SampleCount, SamplePosition, SampleRate,
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

    /// Read sample `i`, coercing non-finite (NaN/Inf) values to `0.0`.
    ///
    /// This is the single sanitize boundary for direct CV-input cables: a NaN/Inf
    /// fed into DSP poisons feedback state and silences/explodes a voice, and a
    /// direct CV buffer is the only way one can enter (mod-matrix offsets are
    /// already clamped in `ParamModOffsets::effective`). **All modules reading a CV
    /// input must use `get(i)` rather than the raw `Index` (`reader[i]`)**, so the
    /// coercion can never be forgotten at a call site. The connected-default is
    /// already finite, so the branch only ever fires for live buffers.
    #[inline]
    #[must_use]
    pub fn get(&self, i: usize) -> f32 {
        let v = match self.buffer {
            Some(buf) => buf[i],
            None => self.default,
        };
        if v.is_finite() { v } else { 0.0 }
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
    /// Not rendered by the auto-renderer — module supplies its own UI.
    Hidden,
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

/// Error returned when a value fails validation against a [`ParameterDescriptor`].
///
/// This is the single descriptor-driven validation shared by every layer that
/// needs to reject bad input (the MCP boundary today). It draws on the same
/// `range` that drives JSON-Schema generation, the GUI, and MCP discovery.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum ParamValueError {
    /// The value was NaN or infinite.
    #[error("value is not finite (NaN or infinity)")]
    NotFinite,
    /// The value fell outside the parameter's `[min, max]` range.
    #[error("value {value} out of range: must be within {min}..={max}")]
    OutOfRange {
        /// The offending value.
        value: f32,
        /// Range minimum (inclusive).
        min: f32,
        /// Range maximum (inclusive).
        max: f32,
    },
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

    /// Whether this parameter may be used as a sequencer automation target.
    ///
    /// A parameter is automatable iff it is **continuous** and **real-time-safe**
    /// to change per processing block. This reuses the [`modulatable`] flag (the
    /// existing "continuous + RT-safe" signal — module authors set it `false` for
    /// structural/sizing params such as unison voice count, pattern length, or
    /// step counts) and additionally excludes `choice`/enum parameters, which are
    /// discrete selections (e.g. `FilterMode`, `Waveform`) and cannot be ramped.
    ///
    /// This is a *descriptor-level eligibility* check: it reports whether a param
    /// is the right *kind* to automate, not whether the owning module currently
    /// honours an override for it. In the A1/A2 first cut the transient override
    /// is implemented only for a subset of modules (Filter, Envelope, Amplifier,
    /// Oscillator); automating an eligible param on any other module is a
    /// documented no-op until its override is implemented.
    ///
    /// [`modulatable`]: Self::modulatable
    #[must_use]
    pub fn is_automatable(&self) -> bool {
        self.modulatable && self.choices.is_none()
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
        if let Some(choice) = self.choice_for_value(value) {
            return choice.name.clone();
        }
        self.unit.format(value)
    }

    /// Look up the `ChoiceOption` corresponding to a numeric parameter
    /// value. Returns `None` for non-choice params, out-of-range indices,
    /// or non-finite inputs.
    #[must_use]
    pub fn choice_for_value(&self, value: f32) -> Option<&ChoiceOption> {
        let choices = self.choices.as_ref()?;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let idx = value.round().max(0.0) as usize;
        choices.get(idx)
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

    /// Validate a raw `f32` value against this parameter's range.
    ///
    /// Returns the value unchanged on success, or a [`ParamValueError`]
    /// describing why it was rejected. For choice parameters the range is
    /// `0..=(choices.len() - 1)`, so an out-of-bounds choice index is rejected
    /// the same way as an out-of-range numeric value.
    pub fn validate_f32(&self, value: f32) -> Result<f32, ParamValueError> {
        if !value.is_finite() {
            return Err(ParamValueError::NotFinite);
        }
        if !self.range.contains(value) {
            return Err(ParamValueError::OutOfRange {
                value,
                min: self.range.min,
                max: self.range.max,
            });
        }
        Ok(value)
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

impl PortType {
    /// Canonical port-compatibility contract: whether a signal leaving an
    /// output port of `self` may drive an input port of type `dest`.
    ///
    /// This is the single source of truth for connection compatibility, shared
    /// by the MCP `check_connection` validator and the GUI patch editor's
    /// cable-drag highlighting. Audio and control are interchangeable, a gate
    /// may feed a control input, and MIDI is MIDI-only. Directional: `Gate →
    /// Control` is allowed but `Control → Gate` is not.
    #[must_use]
    pub fn can_drive(self, dest: Self) -> bool {
        matches!(
            (self, dest),
            (Self::Audio, Self::Audio)
                | (Self::Audio, Self::Control)
                | (Self::Control, Self::Audio)
                | (Self::Control, Self::Control)
                | (Self::Gate, Self::Gate)
                | (Self::Gate, Self::Control)
                | (Self::Midi, Self::Midi)
        )
    }
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

/// Generic per-module accumulator of Mod Matrix offsets — the default
/// `set_mod_offset` channel so a module gets every `modulatable` parameter
/// modulated without hand-writing a `match` arm per param (the scaling contract's
/// option A: normalized-through-range).
///
/// One entry per modulatable param, [`populate`](Self::populate)d **once off the
/// audio thread** from the module's descriptor (which caches each param's
/// `range`+`curve`, so the apply path never rebuilds the descriptor). The
/// hot-path ops ([`add`](Self::add)/[`clear`](Self::clear)/[`effective`](Self::effective))
/// are a linear scan of a tiny fixed `Vec` plus `f32` math — no allocation, no
/// lock, no panic — and run on the audio thread. A module that wants a *musical*
/// scale for a specific target (e.g. pitch in exact semitones) overrides
/// `set_mod_offset` for that target and delegates the rest here.
#[derive(Debug, Clone, Default)]
pub struct ParamModOffsets {
    entries: Vec<ParamOffset>,
}

#[derive(Debug, Clone)]
struct ParamOffset {
    /// Descriptor `type_id` — the module-agnostic param identifier the Mod Matrix
    /// addresses. Sourced from the descriptor on **both** the store side and the
    /// address side, so the two can never drift (no hand-typed literal).
    type_id: String,
    range: ValueRange,
    curve: ResponseCurve,
    /// Accumulated normalized offset for this block (`Σ amount × source`).
    offset: f32,
}

impl ParamModOffsets {
    /// Empty store (no modulatable params registered yet).
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Register one entry per `modulatable` param from `desc`, caching its
    /// `range`+`curve`. **Off the audio thread** (allocates) — the graph calls
    /// this when the module is added, since it already holds the descriptor.
    pub fn populate(&mut self, desc: &ModuleDescriptor) {
        self.entries.clear();
        self.entries.reserve(desc.parameters.len());
        for p in &desc.parameters {
            if p.modulatable {
                self.entries.push(ParamOffset {
                    type_id: p.type_id.clone(),
                    range: p.range,
                    curve: p.response_curve,
                    offset: 0.0,
                });
            }
        }
    }

    /// Accumulate a normalized offset for `type_id` (audio thread, RT-safe).
    /// Unknown / non-modulatable identifiers are ignored.
    pub fn add(&mut self, type_id: &str, value: f32) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.type_id == type_id) {
            e.offset += value;
        }
    }

    /// Zero every accumulated offset (called each block, RT-safe).
    pub fn clear(&mut self) {
        for e in &mut self.entries {
            e.offset = 0.0;
        }
    }

    /// `true` if any registered param currently carries a non-zero offset.
    /// Modules that cache derived state behind a dirty flag use this to force a
    /// recompute for the block while modulation is live (RT-safe linear scan).
    #[must_use]
    pub fn any_active(&self) -> bool {
        self.entries.iter().any(|e| e.offset != 0.0)
    }

    /// The effective native value for `type_id` given its `base` (already
    /// override-resolved) value: `denormalize(clamp(normalize(base) + offset))`
    /// through the param's own range+curve. Returns `base` unchanged when the
    /// param is unregistered or has no offset (RT-safe).
    #[must_use]
    pub fn effective(&self, type_id: &str, base: f32) -> f32 {
        match self.entries.iter().find(|e| e.type_id == type_id) {
            Some(e) if e.offset != 0.0 => {
                let norm = e.curve.normalize(base, e.range) + e.offset;
                e.curve.denormalize(norm.clamp(0.0, 1.0), e.range)
            }
            _ => base,
        }
    }
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

    /// Deliver the voice's per-block note pitch to a pitch-tracking sound source.
    ///
    /// The [`Voice`](../../synth_engine) calls this every block with the played
    /// note's *modulated* fundamental frequency (base pitch, glide, pitch-bend
    /// and per-note vibrato combined). Any module that is a pitched sound source
    /// tracking the played note — oscillators, the sampler, etc. — overrides so it
    /// follows continuous pitch modulation, not just the pitch latched at
    /// [`note_on`](Self::note_on).
    ///
    /// Default: no-op. Effects, modulators and fixed-rate sources ignore the
    /// voice pitch. This is the generalised replacement for the former
    /// oscillator-only `set_oscillator_frequency` path — pitch tracking is now a
    /// per-module capability rather than a hard-coded module-type special case.
    fn set_voice_pitch(&mut self, _freq: Hertz) {}

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
    /// `target` is a stable, module-agnostic parameter identifier — the param's
    /// descriptor `type_id` (e.g. `"cutoff"`, `"resonance"`, `"level"`, `"pan"`,
    /// `"rate"`, `"depth"`), plus `"pitch"` for additive oscillator pitch (which
    /// has no single knob). A module matches the identifiers it supports and
    /// ignores the rest. The offset is *additive* and accumulates across multiple
    /// routings to the same target until [`clear_mod_offsets`](Self::clear_mod_offsets).
    ///
    /// `value` is the routing contribution (`amount × source`); the module scales
    /// it into the parameter's own unit and clamps in `process()`.
    fn set_mod_offset(&mut self, target: &str, value: f32) {
        // Default: route through the generic store if the module exposes one
        // (normalized-through-range; populated from the descriptor). A module that
        // needs a musical per-target scale overrides this and may still delegate
        // the rest here. No store → no-op (legacy behaviour).
        if let Some(offsets) = self.mod_offsets_mut() {
            offsets.add(target, value);
        }
    }

    /// Clear all modulation offsets back to zero.
    fn clear_mod_offsets(&mut self) {
        if let Some(offsets) = self.mod_offsets_mut() {
            offsets.clear();
        }
    }

    /// Expose this module's generic [`ParamModOffsets`] store, if it has one.
    ///
    /// A module opts into descriptor-driven modulation of **every** `modulatable`
    /// param by holding a `ParamModOffsets` field and returning it here; the graph
    /// [`populate`](ParamModOffsets::populate)s it from the descriptor at add time,
    /// and `process()` reads effective values via
    /// [`effective`](ParamModOffsets::effective). Default `None` (no generic
    /// modulation; a module may still hand-implement `set_mod_offset`).
    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        None
    }

    /// Expose this module's modulation routings, if it is a Mod Matrix.
    ///
    /// Lets `Voice` read the routing list directly (source, destination, amount)
    /// rather than probing each field through the numeric `get_param` channel —
    /// the richer read path that lets a destination be an address rather than an
    /// `f32` index. Default `None` (the module has no routings).
    fn mod_routings(&self) -> Option<&[crate::ModRouting]> {
        None
    }

    /// Expose this module's per-slot compiled control scripts (YAMS), if it hosts
    /// any. Modules that host scripts embed a
    /// [`ScriptHost`](crate::script::ScriptHost) and return its
    /// [`slots`](crate::script::ScriptHost::slots) here. For the Mod Matrix this is
    /// parallel to [`mod_routings`](Self::mod_routings): when `scripts()[i]` is
    /// `Some`, slot `i`'s offset is the script's output instead of the scalar
    /// `source × amount` (decision #1). Shared immutably behind an `Arc`; the
    /// per-voice mutable state lives in the module's host. Default `None`.
    fn scripts(&self) -> Option<&[Option<std::sync::Arc<crate::script::BoundScript>>]> {
        None
    }

    /// Install (or clear, with `None`) the compiled control script for one slot.
    ///
    /// The `Arc<BoundScript>` cannot ride the numeric [`set_param`](Self::set_param)
    /// channel, so the load / authoring path sets it here directly. The script is
    /// *compiled* off the audio thread, but this install runs **on** the audio
    /// thread (command drain), so the implementation must **return** the script it
    /// replaces instead of dropping it — the engine routes the old `Arc` to the
    /// main thread for a deferred drop, keeping the (possibly final) `free()` off
    /// the real-time thread. Default: no-op slot, returns `None`.
    #[must_use = "the replaced script must be dropped off the audio thread"]
    fn set_script(
        &mut self,
        _slot: usize,
        _script: Option<std::sync::Arc<crate::script::BoundScript>>,
    ) -> Option<std::sync::Arc<crate::script::BoundScript>> {
        None
    }

    /// Evaluate one hosted script slot from externally-resolved `sources` and the
    /// slot's own per-voice state, returning the sanitized output offset.
    ///
    /// The engine resolves a script's source registers from the graph *first*
    /// (while holding `&graph`), then calls this with `&mut module` — dissolving
    /// the `&graph` / `&mut regs` borrow conflict. `None` if the slot is empty or
    /// the module hosts no scripts. Default: no-op, returns `None`. Real-time safe.
    fn eval_script_slot(
        &mut self,
        _slot: usize,
        _sources: &[f32],
        _ctx: &crate::script::EvalContext,
    ) -> Option<f32> {
        None
    }

    /// Set the stable per-(voice, module) PRNG seed base for this module's hosted
    /// scripts, re-seeding their state. Called once per voice at allocation, after
    /// the voice id is assigned. Default: no-op (the module hosts no scripts).
    fn set_voice_index(&mut self, _voice_index: u32) {}

    /// Apply a transient parameter override from sequencer automation.
    ///
    /// Unlike [`set_param`](Self::set_param), which writes the module's stored
    /// (base) value, an override is a transient layer applied on top of the base
    /// during `process()` and removed by
    /// [`clear_param_overrides`](Self::clear_param_overrides) on transport stop —
    /// the base param is **never** mutated by automation, so a project saved
    /// mid-playback still stores the base value.
    ///
    /// The `Param` carries both *which* parameter to override and the absolute
    /// value to use: an override **replaces** the base for that parameter while
    /// active. This is deliberately distinct from
    /// [`set_mod_offset`](Self::set_mod_offset), which is *additive* — automation
    /// is absolute, mod-matrix modulation is an offset.
    ///
    /// **Combine order when both drive one parameter** (the resolved "two
    /// controllers" rule): the effective value is
    /// `(override.unwrap_or(base)) + mod_offset` — i.e. the automation override
    /// replaces the base, then the mod-matrix offset is added *on top of the
    /// override*. Implementors must follow this order in `process()`.
    ///
    /// Default: no-op (module does not support automation overrides).
    fn set_param_override(&mut self, _param: Param) {
        // Default: no override support.
    }

    /// Clear all transient parameter overrides, reverting affected parameters to
    /// their base values. Called on transport stop (real-time safe).
    fn clear_param_overrides(&mut self) {
        // Default: nothing to clear.
    }

    /// Load sample audio data for playback (Sampler modules only).
    /// Default implementation does nothing.
    fn load_sample_data(
        &mut self,
        _data: std::sync::Arc<[f32]>,
        _channels: ChannelCount,
        _frame_count: usize,
        _root_note: MidiNote,
    ) {
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

    /// Feed audio from a sidechain source into this effect. Most effects
    /// ignore the input; `Compressor` overrides this to gate detection
    /// on the source signal. Called by the engine before `process` when
    /// the host instrument has `sidechain_source_id` set.
    fn set_sidechain_input(&mut self, _buffer: &[f32]) {
        // Default: ignore.
    }
}

// ============================================================================
// Waveform types - Re-exported from typed_params for single source of truth
// ============================================================================

// Re-export waveform and filter types from typed_params
pub use crate::params::{
    ChaoticSystem, DelayMode, DistortionMode, FilterMode, FilterModel, LfoWaveform, MathAlgo,
    Waveform,
};

// Helper to create ChoiceOption lists from enums
impl Waveform {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|w| ChoiceOption::new(w.id(), w.name()).with_description(w.description()))
            .collect()
    }
}

impl LfoWaveform {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|w| ChoiceOption::new(w.id(), w.name()).with_description(w.description()))
            .collect()
    }
}

impl FilterMode {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|f| ChoiceOption::new(f.id(), f.name()).with_description(f.description()))
            .collect()
    }
}

impl FilterModel {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| ChoiceOption::new(m.id(), m.name()).with_description(m.description()))
            .collect()
    }
}

impl ChaoticSystem {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|s| ChoiceOption::new(s.id(), s.name()).with_description(s.description()))
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
    use std::assert_matches;

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
    fn input_reader_get_sanitizes_non_finite_cv() {
        // The single sanitize boundary: a NaN/Inf in a connected CV buffer must read
        // back as a finite 0.0, while ordinary finite values pass through unchanged.
        let mut buf = AudioBuffer::new(4);
        buf[0] = f32::NAN;
        buf[1] = f32::INFINITY;
        buf[2] = f32::NEG_INFINITY;
        buf[3] = 0.75;

        let ports_data = [(PortName::CV, &buf)];
        let ports = InputPorts::new(&ports_data);
        let reader = ports.reader(PortName::CV, 0.0);

        assert_eq!(reader.get(0), 0.0, "NaN coerced to 0");
        assert_eq!(reader.get(1), 0.0, "+Inf coerced to 0");
        assert_eq!(reader.get(2), 0.0, "-Inf coerced to 0");
        assert_eq!(reader.get(3), 0.75, "finite value passes through");
        // An unconnected reader returns its (finite) default.
        let empty = InputPorts::empty();
        assert_eq!(empty.reader(PortName::CV, 0.0).get(7), 0.0);
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

    fn descriptor_with_range(min: f32, max: f32, default: f32) -> ParameterDescriptor {
        ParameterDescriptor::float(
            "test_param",
            Param::Oscillator(crate::params::OscillatorParam::Detune(crate::Cents::ZERO)),
            "Test Param",
        )
        .value_range(ValueRange::new(min, max, default))
    }

    #[test]
    fn validate_f32_accepts_in_range_and_bounds() {
        let pd = descriptor_with_range(-100.0, 100.0, 0.0);
        assert_eq!(pd.validate_f32(0.0), Ok(0.0));
        assert_eq!(pd.validate_f32(-100.0), Ok(-100.0)); // inclusive min
        assert_eq!(pd.validate_f32(100.0), Ok(100.0)); // inclusive max
    }

    #[test]
    fn validate_f32_rejects_out_of_range() {
        let pd = descriptor_with_range(0.0, 1.0, 0.5);
        assert_eq!(
            pd.validate_f32(1.5),
            Err(ParamValueError::OutOfRange {
                value: 1.5,
                min: 0.0,
                max: 1.0,
            })
        );
        assert_matches!(
            pd.validate_f32(-0.1),
            Err(ParamValueError::OutOfRange { .. })
        );
    }

    #[test]
    fn validate_f32_rejects_non_finite() {
        let pd = descriptor_with_range(0.0, 1.0, 0.5);
        assert_eq!(pd.validate_f32(f32::NAN), Err(ParamValueError::NotFinite));
        assert_eq!(
            pd.validate_f32(f32::INFINITY),
            Err(ParamValueError::NotFinite)
        );
    }

    #[test]
    fn validate_f32_treats_choice_index_as_range() {
        // choice() sets range 0..=(len-1); an out-of-bounds index is rejected.
        let choices = vec![
            ChoiceOption::new("a", "A"),
            ChoiceOption::new("b", "B"),
            ChoiceOption::new("c", "C"),
        ];
        let pd = ParameterDescriptor::choice(
            "mode",
            Param::Oscillator(crate::params::OscillatorParam::Detune(crate::Cents::ZERO)),
            "Mode",
            choices,
        );
        assert_eq!(pd.validate_f32(0.0), Ok(0.0));
        assert_eq!(pd.validate_f32(2.0), Ok(2.0));
        assert_matches!(
            pd.validate_f32(3.0),
            Err(ParamValueError::OutOfRange { .. })
        );
    }

    #[test]
    fn is_automatable_includes_continuous_excludes_choice_and_structural() {
        let detune =
            || Param::Oscillator(crate::params::OscillatorParam::Detune(crate::Cents::ZERO));

        // Continuous, modulatable float: automatable.
        assert!(ParameterDescriptor::float("cutoff", detune(), "Cutoff").is_automatable());

        // Structural/sizing float (opted out via modulatable(false)): excluded.
        assert!(
            !ParameterDescriptor::float("unison", detune(), "Unison")
                .modulatable(false)
                .is_automatable()
        );

        // Choice/enum param: excluded (discrete, not a ramp).
        let choice = ParameterDescriptor::choice(
            "mode",
            detune(),
            "Mode",
            vec![ChoiceOption::new("a", "A"), ChoiceOption::new("b", "B")],
        );
        assert!(!choice.is_automatable());
    }

    /// `ParamModOffsets` populates from a descriptor (modulatable params only),
    /// applies a normalized offset through each param's range, accumulates, and
    /// clears — the generic `set_mod_offset` channel (migration step 1).
    #[test]
    fn param_mod_offsets_populate_apply_accumulate_clear() {
        let dummy =
            || Param::Oscillator(crate::params::OscillatorParam::Detune(crate::Cents::ZERO));
        let desc = ModuleDescriptor::new("test", "Test")
            // Modulatable linear param 0..10.
            .parameter(ParameterDescriptor::float("amt", dummy(), "Amt").range(0.0, 10.0))
            // Non-modulatable: must NOT be registered.
            .parameter(
                ParameterDescriptor::float("fixed", dummy(), "Fixed")
                    .range(0.0, 1.0)
                    .modulatable(false),
            );

        let mut off = ParamModOffsets::new();
        off.populate(&desc);

        // No offset yet → base passes through unchanged.
        assert!((off.effective("amt", 5.0) - 5.0).abs() < 1e-4);

        // base 5 (norm 0.5 on 0..10 linear) + 0.25 → norm 0.75 → 7.5.
        off.add("amt", 0.25);
        assert!(
            (off.effective("amt", 5.0) - 7.5).abs() < 1e-4,
            "got {}",
            off.effective("amt", 5.0)
        );
        // Accumulates additively: +0.25 more → norm 1.0 → clamped → 10.0.
        off.add("amt", 0.25);
        assert!((off.effective("amt", 5.0) - 10.0).abs() < 1e-4);

        // A non-modulatable param was never registered → base unchanged.
        assert!((off.effective("fixed", 0.3) - 0.3).abs() < 1e-6);
        // An unknown id → base unchanged.
        assert!((off.effective("nope", 1.0) - 1.0).abs() < 1e-6);

        // Clear resets every offset.
        off.clear();
        assert!((off.effective("amt", 5.0) - 5.0).abs() < 1e-4);
    }
}
