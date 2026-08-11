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
use crate::display::DisplayName;
use crate::params::{ModuleType, Param};
pub use crate::types::{
    BeatPosition, Bpm, Hertz, MidiNote, NormalizedValue, SampleCount, SamplePosition, SampleRate,
    Semitones, ValueRange, Velocity,
};

/// The voice's per-block note pitch, **decomposed** so a pitch-tracking module
/// can either follow the finished pitch (default) or run its own glide.
///
/// The voice pre-computes all four fields once per block. A module that does not
/// opt into per-oscillator glide reads only [`played`](Self::played) and behaves
/// exactly as before. A module running its own portamento smooths toward
/// [`note_target`](Self::note_target) and re-applies [`expr`](Self::expr) on top,
/// so pitch-bend and per-note vibrato are never smoothed away by the glide.
///
/// `Copy`/alloc-free so it threads through the per-block broadcast without
/// allocation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoicePitch {
    /// Finished playing pitch: voice glide + pitch-bend + per-note vibrato folded
    /// in. Identical to the single frequency the voice handed its sound sources
    /// before this struct existed — the value every non-opt-in module reads.
    pub played: Hertz,
    /// Raw note target: the destination note frequency with **no** glide, bend or
    /// vibrato applied. A module gliding itself smooths toward this.
    pub note_target: Hertz,
    /// Pitch-bend + per-note vibrato + track pitch (`TrackParam::Pitch`
    /// automation) as a single additive semitone offset, to be applied *after*
    /// a module's own glide so expression rides on top un-smoothed. Note the
    /// magnitude: bend/vibrato are small (default bend range ±2 st) but track
    /// pitch spans ±48 st, so consumers must not assume a narrow range.
    pub expr: Semitones,
    /// The sounding MIDI note (before continuous modulation). Lets note-aware
    /// sources read the note without back-solving it from `played`.
    pub note: MidiNote,
}

impl VoicePitch {
    /// A voice pitch that simply tracks `freq`: no bend/vibrato, `note_target`
    /// equal to `played`. Used for note-on seeding and in tests where only the
    /// played pitch matters; `note` defaults to [`MidiNote::A4`].
    #[must_use]
    pub fn tracking(freq: Hertz) -> Self {
        Self {
            played: freq,
            note_target: freq,
            expr: Semitones::ZERO,
            note: MidiNote::A4,
        }
    }
}

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
pub struct ModuleTypeId(String);

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
    /// One waveform-preview toggle button per combinable waveform bit — the
    /// multi-select sibling of `WaveformSelector` for bool mask bits (the SID
    /// oscillator's waveform register). The parameter's `type_id` names the
    /// waveform shape to preview.
    WaveformToggle,
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
    /// Map a lowercase token to a unit, returning [`Self::None`] for anything
    /// unrecognized. Used by the YAMS `param … unit <token>` clause, so it stays
    /// forward-compatible (an unknown token is simply unitless, not an error).
    #[must_use]
    pub fn from_token(token: &str) -> Self {
        match token {
            "hz" => Self::Hertz,
            "db" => Self::Decibels,
            "percent" | "pct" => Self::Percent,
            "ms" => Self::Milliseconds,
            "s" | "sec" => Self::Seconds,
            "st" | "semitones" => Self::Semitones,
            "cents" => Self::Cents,
            "oct" | "octaves" => Self::Octaves,
            "beats" => Self::Beats,
            "bpm" => Self::BeatsPerMinute,
            "samples" => Self::Samples,
            "ratio" => Self::Ratio,
            _ => Self::None,
        }
    }

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

/// What kind of value a parameter holds — the single authoritative classifier,
/// derived from the engine variant's backing type via [`ScalarParam`], never
/// hand-declared. Emitted into `descriptors.json` and onto the MCP wire;
/// `Deserialize` is derived only because [`ParameterDescriptor`] (which carries a
/// `kind`) derives it — descriptors are code, not persisted, so it is never
/// actually read back from a file.
///
/// See `docs/param-kinds.md` for the per-variant audit and
/// `plans/param-type-system-plan.md` for the design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename_all = "lowercase")]
pub enum ParamKind {
    /// f32 within `range`, shaped by `response_curve`, displayed with `unit`.
    Continuous,
    /// Discrete integer (count or index). `range` carries min/max; step is 1,
    /// curve is linear, display is decimal-free.
    Integer,
    /// Two-state. Rendered as a toggle; serialized as a JSON bool.
    Bool,
    /// Finite named set; the value is an index into `choices`.
    Enum,
    /// Opaque id / address outside the numeric scale (sample id, mod-matrix
    /// address). Deliberately coarse — serialization and the picker widget stay
    /// variant-specific; `Reference` only flags "not a plain number".
    Reference,
}

impl ParamKind {
    /// Format a value for display, kind-aware (Phase 3). The single source of
    /// truth for `value → string`, shared by the descriptor and the GUI widgets:
    /// `Integer` is decimal-free, `Bool` reads `On`/`Off`, everything else uses the
    /// unit's own formatter.
    #[must_use]
    pub fn format(self, unit: ParameterUnit, value: f32) -> String {
        match self {
            Self::Integer => format!("{:.0}{}", value.round(), unit.suffix()),
            Self::Bool => {
                if value > 0.5 {
                    "On".to_string()
                } else {
                    "Off".to_string()
                }
            }
            // Continuous / Enum / Reference: defer to the unit's own formatter.
            // (Enum/Reference values rarely reach here — descriptors map them to a
            // choice name first — but a bare numeric fallback is harmless.)
            Self::Continuous | Self::Enum | Self::Reference => unit.format(value),
        }
    }
}

/// Type-derived metadata for every value type a `Param` variant can carry.
///
/// Implemented once per value type (newtypes, primitives, `bool`, references, and
/// each choice enum). The provided methods (`scalar_kind`/`scalar_unit`/
/// `scalar_curve`) are called on a **bound value** by the per-enum `kind()`/`unit()`
/// methods, so the metadata follows the *actual field type* and can never drift:
/// change a variant's field type and the resolved kind/unit changes with it.
///
/// `UNIT` is the natural display unit (overridable per descriptor). `DEFAULT_CURVE`
/// is **advisory only** — it is *not* auto-applied by the descriptor constructors
/// (a response curve is behavioral); it serves the Phase 2b curve audit.
pub trait ScalarParam {
    /// The value-kind this type maps to.
    const KIND: ParamKind;
    /// Natural display unit for this type.
    const UNIT: ParameterUnit;
    /// Suggested response curve (advisory; not auto-applied).
    const DEFAULT_CURVE: ResponseCurve;

    /// Kind of a bound value — drift-proof dispatch (resolves to `Self::KIND`).
    #[inline]
    #[must_use]
    fn scalar_kind(&self) -> ParamKind {
        Self::KIND
    }
    /// Unit of a bound value (resolves to `Self::UNIT`).
    #[inline]
    #[must_use]
    fn scalar_unit(&self) -> ParameterUnit {
        Self::UNIT
    }
    /// Suggested curve of a bound value (resolves to `Self::DEFAULT_CURVE`).
    #[inline]
    #[must_use]
    fn scalar_curve(&self) -> ResponseCurve {
        Self::DEFAULT_CURVE
    }
}

/// The uniform contract every module parameter enum (and the aggregate [`Param`])
/// provides: f32 round-tripping for the GUI, same-kind comparison, a display name,
/// and the value-kind metadata (`kind`/`unit`/`default_curve`).
///
/// This is the single definition point for the method set: each `*Param` enum
/// implements the complete contract directly, so generic code can use
/// `T: ModuleParam` and a new enum that omits a method is a compile error.
pub trait ModuleParam: Copy {
    /// Current value as an f32 (for GUI sliders / serialization).
    #[must_use]
    fn as_f32(&self) -> f32;
    /// This parameter with `value` applied (clamped/rounded to its type).
    #[must_use]
    fn with_f32(&self, value: f32) -> Self;
    /// Whether two params are the same kind (ignoring their values).
    #[must_use]
    fn same_kind(&self, other: &Self) -> bool;
    /// Human-readable parameter name.
    #[must_use]
    fn name(&self) -> &'static str;
    /// Value-kind classifier.
    #[must_use]
    fn kind(&self) -> ParamKind;
    /// Display unit.
    #[must_use]
    fn unit(&self) -> ParameterUnit;
    /// Suggested response curve (advisory).
    #[must_use]
    fn default_curve(&self) -> ResponseCurve;
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
    /// Value-kind classifier, seeded from `id.kind()` by the constructors so it
    /// can never drift from the engine's backing type. Recomputed in code, never
    /// read from persisted data.
    pub kind: ParamKind,
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
        let kind = id.kind();
        // Phase 2a: the display unit is derived from the parameter's value type
        // (`Hertz` → `Hz`, …), killing hand-typed unit drift. Override per descriptor
        // with `.unit()` for the rare legitimate case (e.g. `NormalizedValue` as
        // `Percent`). `response_curve` is NOT derived — that is behavioral (§14.6).
        let unit = id.unit();
        Self {
            type_id: type_id.into(),
            id,
            name: name.into(),
            description: String::new(),
            range: ValueRange::UNIT,
            unit,
            widget_hint: WidgetHint::Knob,
            response_curve: ResponseCurve::Linear,
            choices: None,
            modulatable: true,
            kind,
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
        let kind = id.kind();
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
            kind,
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
    /// A parameter is automatable iff it is a **numeric scalar** and
    /// **real-time-safe** to change per processing block. This requires a
    /// rampable kind — [`ParamKind::Continuous`] or [`ParamKind::Integer`]
    /// (integer lanes apply as stepped/sample-hold values through the same
    /// normalized pipeline; `with_f32` rounds — e.g. the SID chip registers,
    /// where per-frame PWM lanes are the core idiom) — and the [`modulatable`]
    /// flag (the "RT-safe" signal — module authors set it `false` for
    /// structural/sizing params such as unison voice count, pattern length,
    /// or step counts). Bool/enum/reference params are excluded by kind: they
    /// are discrete selections, not scalars on a range.
    ///
    /// This is a *descriptor-level eligibility* check: it reports whether a param
    /// is the right *kind* to automate, not whether the owning module currently
    /// honours an override for it. The transient override is implemented per
    /// module (Filter, Envelope, Amplifier, Oscillator, SidOscillator, …);
    /// automating an eligible param on any other module is a documented no-op
    /// until its override is implemented.
    ///
    /// [`modulatable`]: Self::modulatable
    #[must_use]
    pub fn is_automatable(&self) -> bool {
        self.modulatable && matches!(self.kind, ParamKind::Continuous | ParamKind::Integer)
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
        self.kind.format(self.unit, value)
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
        match self.kind {
            // Bool: accept any finite value; `with_f32` maps it via `> 0.5`.
            ParamKind::Bool => Ok(value),
            // Integer: round to nearest (lenient — a `4.3` from an automation/LFO
            // sweep is accepted, not rejected), then range-check the *rounded*
            // value. The rounded value is returned so the caller applies — and can
            // echo — exactly what took effect (Phase 5 / §14.1).
            ParamKind::Integer => {
                let rounded = value.round();
                if self.range.contains(rounded) {
                    Ok(rounded)
                } else {
                    Err(ParamValueError::OutOfRange {
                        value: rounded,
                        min: self.range.min,
                        max: self.range.max,
                    })
                }
            }
            // Continuous / Enum / Reference: range / choice-index check as before.
            _ => {
                if self.range.contains(value) {
                    Ok(value)
                } else {
                    Err(ParamValueError::OutOfRange {
                        value,
                        min: self.range.min,
                        max: self.range.max,
                    })
                }
            }
        }
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
    /// Every signal type in stable catalog order.
    pub const ALL: [Self; 4] = [Self::Audio, Self::Control, Self::Gate, Self::Midi];

    /// Stable lowercase identifier used by schemas and MCP discovery.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Control => "control",
            Self::Gate => "gate",
            Self::Midi => "midi",
        }
    }

    /// Canonical port-compatibility contract: whether a signal leaving an
    /// output port of `self` may drive an input port of type `dest`.
    ///
    /// This is the single source of truth for connection compatibility, shared
    /// by graph validation, the MCP `check_connection` validator, and the GUI
    /// patch editor's cable-drag highlighting. Audio and control are
    /// interchangeable, control may drive thresholded gate inputs, a gate may
    /// feed a control input, and MIDI is MIDI-only.
    #[must_use]
    pub fn can_drive(self, dest: Self) -> bool {
        matches!(
            (self, dest),
            (Self::Audio, Self::Audio)
                | (Self::Audio, Self::Control)
                | (Self::Control, Self::Audio)
                | (Self::Control, Self::Control)
                | (Self::Control, Self::Gate)
                | (Self::Gate, Self::Gate)
                | (Self::Gate, Self::Control)
                | (Self::Midi, Self::Midi)
        )
    }
}

impl std::fmt::Display for PortType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.id())
    }
}

/// Direction of a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortDirection {
    Input,
    Output,
}

impl PortDirection {
    /// Stable lowercase identifier used by descriptor catalogs.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

/// Numeric value range advertised for a port's nominal signal domain.
///
/// This is descriptive metadata, not an engine clamp. Ports accept the values
/// documented by [`PortValueDomain::accepted_values`]; the nominal range tells
/// patch builders which values a well-behaved source normally produces.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[must_use]
pub struct PortValueRange {
    min: f32,
    max: f32,
}

impl PortValueRange {
    pub const UNIPOLAR: Self = Self::new(0.0, 1.0);
    pub const BIPOLAR: Self = Self::new(-1.0, 1.0);

    const fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    #[must_use]
    pub const fn min(self) -> f32 {
        self.min
    }

    #[must_use]
    pub const fn max(self) -> f32 {
        self.max
    }
}

/// Value semantics for a module port.
///
/// Signal type answers whether a port carries audio, control, gate, or MIDI.
/// This domain separately documents which numeric values the port accepts and
/// the nominal range/unit a source should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortValueDomain {
    /// Any finite audio sample; nominal linear amplitude is -1 to +1.
    Audio,
    /// Any finite control value with no narrower universal nominal range.
    Control,
    /// Any finite normalized control value; nominal range is 0 to 1.
    Unipolar,
    /// Any finite normalized control value; nominal range is -1 to +1.
    Bipolar,
    /// Any finite gate value, thresholded at 0.5; conventional values are 0/1.
    Gate,
    /// Pitch measured in octaves (+1 doubles frequency); per-module clamped.
    Octaves,
    /// Pitch measured in semitones (+12 doubles frequency); per-module clamped.
    Semitones,
    /// Any finite additive offset in SID frequency-register units.
    SidFrequencyRegisterOffset,
    /// Any finite additive offset in SID pulse-width-register units.
    SidPulseWidthRegisterOffset,
    /// MIDI channel/system messages rather than numeric sample values.
    Midi,
}

impl PortValueDomain {
    /// Stable lowercase identifier used by descriptor catalogs.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Control => "control",
            Self::Unipolar => "unipolar",
            Self::Bipolar => "bipolar",
            Self::Gate => "gate",
            Self::Octaves => "octaves",
            Self::Semitones => "semitones",
            Self::SidFrequencyRegisterOffset => "sid_frequency_register_offset",
            Self::SidPulseWidthRegisterOffset => "sid_pulse_width_register_offset",
            Self::Midi => "midi",
        }
    }

    /// Exact value contract for discovery clients: which values an input port
    /// accepts, or equivalently which values an output port produces.
    #[must_use]
    pub const fn accepted_values(self) -> &'static str {
        match self {
            Self::Audio => {
                "Any finite f32 sample; values outside the nominal range are allowed but may clip downstream."
            }
            Self::Control | Self::Unipolar | Self::Bipolar => "Any finite f32 control value.",
            Self::Gate => "Any finite f32; values <= 0.5 are low and values > 0.5 are high.",
            Self::Octaves => {
                "Any finite f32 pitch value in octaves; +1.0 doubles and -1.0 halves frequency. Each module clamps the offset it applies (1V/oct oscillator inputs to ±1 octave); see the port description."
            }
            Self::Semitones => {
                "Any finite f32 pitch value in semitones; +12.0 doubles and -12.0 halves frequency. Each module clamps the offset it applies; see the port description."
            }
            Self::SidFrequencyRegisterOffset => {
                "Any finite f32 additive SID frequency-register offset; the effective register is clamped to its hardware range."
            }
            Self::SidPulseWidthRegisterOffset => {
                "Any finite f32 additive SID pulse-width-register offset; the effective register is clamped to its hardware range."
            }
            Self::Midi => "Valid MIDI channel or system messages.",
        }
    }

    /// Nominal range produced or expected by the port, when one is meaningful.
    #[must_use]
    pub const fn nominal_range(self) -> Option<PortValueRange> {
        match self {
            Self::Audio | Self::Bipolar => Some(PortValueRange::BIPOLAR),
            Self::Unipolar | Self::Gate => Some(PortValueRange::UNIPOLAR),
            Self::Control
            | Self::Octaves
            | Self::Semitones
            | Self::SidFrequencyRegisterOffset
            | Self::SidPulseWidthRegisterOffset
            | Self::Midi => None,
        }
    }

    /// Stable unit identifier for numeric discovery clients.
    #[must_use]
    pub const fn unit(self) -> Option<&'static str> {
        match self {
            Self::Audio => Some("linear_amplitude"),
            Self::Unipolar | Self::Bipolar | Self::Gate => Some("normalized"),
            Self::Octaves => Some("octaves"),
            Self::Semitones => Some("semitones"),
            Self::SidFrequencyRegisterOffset => Some("sid_frequency_register_units"),
            Self::SidPulseWidthRegisterOffset => Some("sid_pulse_width_register_units"),
            Self::Control | Self::Midi => None,
        }
    }
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
    /// Accepted values, nominal range, and unit for the signal.
    pub value_domain: PortValueDomain,
}

impl PortDescriptor {
    pub fn audio_input(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Audio,
            direction: PortDirection::Input,
            value_domain: PortValueDomain::Audio,
        }
    }

    pub fn audio_output(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Audio,
            direction: PortDirection::Output,
            value_domain: PortValueDomain::Audio,
        }
    }

    #[must_use]
    pub fn control_output(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Control,
            direction: PortDirection::Output,
            value_domain: PortValueDomain::Control,
        }
    }

    #[must_use]
    pub fn gate_output(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Gate,
            direction: PortDirection::Output,
            value_domain: PortValueDomain::Gate,
        }
    }

    pub fn control_input(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Control,
            direction: PortDirection::Input,
            value_domain: PortValueDomain::Control,
        }
    }

    pub fn gate_input(name: impl Into<PortName>, label: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            port_type: PortType::Gate,
            direction: PortDirection::Input,
            value_domain: PortValueDomain::Gate,
        }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Declare accepted values and the port's nominal signal range/unit.
    #[must_use]
    pub fn value_domain(mut self, value_domain: PortValueDomain) -> Self {
        self.value_domain = value_domain;
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

impl DisplayName for ModuleCategory {
    /// Signal-flow order (source → shaper → sink), not declaration order: this
    /// is what the patch editor's "Add module" menu shows, so a reader scans the
    /// categories the way a patch is built. Being the complete list is the point
    /// — a new variant cannot be silently missing from the menu.
    const ALL: &'static [Self] = &[
        Self::Oscillator,
        Self::Filter,
        Self::Envelope,
        Self::LFO,
        Self::Amplifier,
        Self::Mixer,
        Self::Effect,
        Self::Sampler,
        Self::Utility,
        Self::Sequencer,
        Self::PhysicalModeling,
        Self::Visualizer,
        Self::Output,
    ];

    /// The label the GUI shows for this category. A few read better than the
    /// variant name: `Utility` covers modulation utilities, `Sequencer` is the
    /// generative/pattern group, and `PhysicalModeling` is abbreviated so the
    /// menu row stays short.
    fn display_name(self) -> &'static str {
        match self {
            Self::Oscillator => "Oscillator",
            Self::Filter => "Filter",
            Self::Envelope => "Envelope",
            Self::LFO => "LFO",
            Self::Amplifier => "Amplifier",
            Self::Effect => "Effect",
            Self::Utility => "Modulation / Utility",
            Self::Sampler => "Sampler",
            Self::Sequencer => "Generative",
            Self::Mixer => "Mixer",
            Self::Output => "Output",
            Self::Visualizer => "Visualizer",
            Self::PhysicalModeling => "Physical",
        }
    }
}

impl ModuleCategory {
    /// Stable lowercase identifier used by descriptor catalogs.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Oscillator => "oscillator",
            Self::Filter => "filter",
            Self::Envelope => "envelope",
            Self::LFO => "lfo",
            Self::Amplifier => "amplifier",
            Self::Effect => "effect",
            Self::Utility => "utility",
            Self::Sampler => "sampler",
            Self::Sequencer => "sequencer",
            Self::Mixer => "mixer",
            Self::Output => "output",
            Self::Visualizer => "visualizer",
            Self::PhysicalModeling => "physical_modeling",
        }
    }
}

/// Fixed display-width bucket for a module's panel in the patch editor (and the
/// mixer's return-bus inserts). A module declares one of these instead of letting
/// its widest body row size it, so widths are deliberate, uniform, and known
/// before the body renders.
///
/// The pixel values are exact multiples of the editor's 32 px grid, so a module's
/// rendered width equals the grid cell it snaps to (no sub-grid gap) and the
/// auto-layout columns stay grid-aligned.
///
/// This is a pure GUI-layout concern: it is never serialized (the field on
/// [`ModuleDescriptor`] is `#[serde(skip)]`), and descriptors are always rebuilt
/// from code, so it needs no serde support.
/// Each bucket's content width (total minus the ~88 px of chrome a normal patch
/// module spends on its two 28 px port columns, item spacing, and inner margins)
/// is roughly `total − 88`, which works out to 1 / 2 / 3 / 4 / 5 knobs per row for
/// XS → XL. The scale is grid-aligned and chosen so even the smallest bucket holds
/// a usable control.
/// The variants are ordered smallest-to-largest, so the derived `Ord` compares
/// buckets by width (`ModuleWidth::Small < ModuleWidth::Large`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ModuleWidth {
    /// 160 px (5 grid cells, ~72 px content) — a single control: level meter,
    /// the inline signal monitor, one-knob utilities.
    ExtraSmall,
    /// 192 px (6 grid cells, ~104 px content) — 2 knobs/row: simple ≤3-param
    /// modules (amplifier, sub oscillator, basic utilities).
    Small,
    /// 256 px (8 grid cells, ~168 px content) — 3 knobs/row, the common case
    /// (most oscillators, filters, LFOs, mid-size effects).
    #[default]
    Medium,
    /// 352 px (11 grid cells, ~264 px content) — 4 knobs/row plus the bespoke
    /// editor bodies (envelope, oscilloscope, spectrum, mod matrix, sampler).
    Large,
    /// 448 px (14 grid cells, ~360 px content) — 5 knobs/row, the widest content
    /// (MSEG, large displays/editors).
    ExtraLarge,
}

impl ModuleWidth {
    /// Total module panel width in pixels (a multiple of the 32 px editor grid).
    #[must_use]
    pub const fn module_px(self) -> f32 {
        match self {
            Self::ExtraSmall => 160.0,
            Self::Small => 192.0,
            Self::Medium => 256.0,
            Self::Large => 352.0,
            Self::ExtraLarge => 448.0,
        }
    }
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
    /// Fixed display-width bucket for the module's panel. Not serialized — it is a
    /// code-declared GUI layout property, always rebuilt from the descriptor.
    #[serde(skip)]
    pub width: ModuleWidth,
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
            width: ModuleWidth::default(),
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

    /// Set the module's fixed display-width bucket (defaults to
    /// [`ModuleWidth::Medium`]).
    #[must_use]
    pub fn width(mut self, width: ModuleWidth) -> Self {
        self.width = width;
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

    /// Look a parameter up by its stable `type_id` or, failing that, its display
    /// name — matching case-insensitively with `_` and space interchangeable, so
    /// a patch file's `"key_track"` and a client's `"Key Track"` both land.
    ///
    /// The `type_id` is tried first and exhaustively: it is the identifier that
    /// can never be renamed, so a display name must never shadow it.
    #[must_use]
    pub fn find_parameter(&self, name: &str) -> Option<&ParameterDescriptor> {
        let needle = normalize_param_name(name);
        self.parameters
            .iter()
            .find(|parameter| normalize_param_name(&parameter.type_id) == needle)
            .or_else(|| {
                self.parameters
                    .iter()
                    .find(|parameter| normalize_param_name(&parameter.name) == needle)
            })
    }

    /// A hint naming the parameter `type_id`s closest to `needle`, or an empty
    /// string when none is close. Appends to a message that does not end in
    /// punctuation.
    ///
    /// The rejecting lookup is holding the entire list of names it would have
    /// accepted, so the caller should never have to spend a
    /// `get_module_type_info` round trip to learn that `cutof` was nearly
    /// `cutoff`.
    ///
    /// For a lookup that accepts display names too, use
    /// [`param_lookup_hint`](Self::param_lookup_hint) — offering a display name
    /// where only the `type_id` parses would be a hint the caller cannot act on.
    #[must_use]
    pub fn param_id_hint(&self, needle: &str) -> String {
        crate::suggest::did_you_mean(
            needle,
            self.parameters
                .iter()
                .map(|parameter| parameter.type_id.as_str()),
            crate::suggest::DEFAULT_MAX_HINTS,
        )
    }

    /// As [`param_id_hint`](Self::param_id_hint), but for the lookups that also
    /// accept a parameter's display name (see
    /// [`find_parameter`](Self::find_parameter)).
    ///
    /// Ranks **both** spellings and answers with the `type_id`, the same way
    /// [`ModuleType::suggest`](crate::ModuleType::suggest) ranks display names
    /// and answers with the prefix. The two spellings are equivalent to the
    /// lookup but not to the ranker, which counts plain edits: `"Key Tracking"`
    /// is 3 edits from the name `"Key Track"` and 4 from the `type_id`
    /// `"key_track"` — the `_`-for-space substitution is the edit that pushes it
    /// over the threshold — so ranking the ids alone answers a recoverable typo
    /// with silence.
    #[must_use]
    pub fn param_lookup_hint(&self, needle: &str) -> String {
        // Ranked per *parameter* rather than per spelling: a parameter that
        // matches by id and by name is one answer, and ranking the loose strings
        // would spend two of the three slots on it.
        let hits = crate::suggest::similar_by(
            needle,
            self.parameters.iter(),
            |parameter| [parameter.type_id.as_str(), parameter.name.as_str()],
            crate::suggest::DEFAULT_MAX_HINTS,
        );
        crate::suggest::hint_from(hits.into_iter().map(|parameter| parameter.type_id.as_str()))
    }
}

/// Lowercase and treat `_` as a space, so the `snake_case` `type_id` and the
/// spaced display name of the same parameter compare equal.
///
/// Public because [`find_parameter`](ModuleDescriptor::find_parameter) is not
/// the only lookup that must fold names this way — a caller matching runtime
/// parameters against descriptor entries by name needs the *same* folding, and
/// a second copy of this one-liner is exactly how two lookups end up accepting
/// different spellings.
#[must_use]
pub fn normalize_param_name(s: &str) -> String {
    s.to_lowercase().replace('_', " ")
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
    /// The [`Voice`](../../synth_engine) calls this every block with a
    /// [`VoicePitch`]: the finished [`played`](VoicePitch::played) pitch (base
    /// pitch, glide, pitch-bend and per-note vibrato combined) plus the
    /// decomposed [`note_target`](VoicePitch::note_target) and
    /// [`expr`](VoicePitch::expr) so a module can optionally run its own glide.
    /// Any module that is a pitched sound source tracking the played note —
    /// oscillators, the sampler, etc. — overrides so it follows continuous pitch
    /// modulation, not just the pitch latched at [`note_on`](Self::note_on).
    ///
    /// Most overrides simply read [`VoicePitch::played`] (identical to the former
    /// single-`Hertz` value). Default: no-op — effects, modulators and fixed-rate
    /// sources ignore the voice pitch.
    fn set_voice_pitch(&mut self, _pitch: VoicePitch) {}

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

    /// Re-seed this module's internal RNG so random-family modules can be
    /// decorrelated across instances (a Mod Grid graph assigned to several
    /// tracks, say). Default no-op — a deterministic module has nothing to seed.
    /// Implementors set their PRNG state deterministically from `seed` so an
    /// offline render reproduces the live result. Called off the audio thread at
    /// build time, before the module processes.
    fn set_seed(&mut self, _seed: u64) {}

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

    /// Evaluate a one-program **control-ports** script module (the `Script`
    /// module) from externally-resolved `sources`, caching up to four outputs
    /// (`out1..out4`) for [`process`](Self::process) to broadcast across their
    /// port buffers. Unlike [`eval_script_slot`](Self::eval_script_slot) (one
    /// value per rack slot, used by the Mod Matrix), this runs the module's single
    /// program once into a 4-output capture. The voice resolves the block-constant
    /// sources **and** the `in1..in4` port values first (holding `&graph`), then
    /// calls this with `&mut module`. Default: no-op (not a control-ports script
    /// module). Real-time safe.
    fn eval_control_multi(&mut self, _sources: &[f32], _ctx: &crate::script::EvalContext) {}

    /// The **effective value** (stored/automated base + this block's accumulated
    /// mod-offset) of a script module's user-declared `param` knob, by interned
    /// name. The voice reads it each block to fill the program's `LocalParam`
    /// source register — the block-constant knob value the script sees. Default
    /// `None` (not a script module, or no such knob). Real-time safe.
    fn effective_param(&self, _name: PortName) -> Option<f32> {
        None
    }

    /// Set the stable per-(voice, module) PRNG seed base for this module's hosted
    /// scripts, re-seeding their state. Called once per voice at allocation, after
    /// the voice id is assigned. Default: no-op (the module hosts no scripts).
    fn set_voice_index(&mut self, _voice_index: u32) {}

    /// Hand an audio-rate script module its source values for this block. The
    /// voice resolves block constants from the graph (macros, context vars,
    /// module addresses) and supplies the voice-frequency start/end range; the
    /// module overwrites its audio inputs, `note_hz`, and `first_sample`
    /// registers per sample inside `process`. Default: no-op (not an audio-rate
    /// script module). Real-time safe.
    fn set_audio_block_sources(
        &mut self,
        _sources: &[f32],
        _note_hz: crate::script::NoteFrequencyRange,
    ) {
    }

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

impl crate::params::SidModel {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| ChoiceOption::new(m.id(), m.name()).with_description(m.description()))
            .collect()
    }
}

impl crate::params::SidClock {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|c| ChoiceOption::new(c.id(), c.name()).with_description(c.description()))
            .collect()
    }
}

impl crate::params::SidQuality {
    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|q| ChoiceOption::new(q.id(), q.name()).with_description(q.description()))
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

    /// A descriptor shaped like the real filter's: a `type_id` in `snake_case`
    /// and a display name that is *not* a respelling of it.
    fn key_track_descriptor() -> ModuleDescriptor {
        ModuleDescriptor::new("flt", "Filter")
            .parameter(ParameterDescriptor::float(
                "cutoff",
                Param::Oscillator(crate::params::OscillatorParam::Frequency(
                    crate::Hertz::new(1000.0),
                )),
                "Cutoff",
            ))
            .parameter(ParameterDescriptor::float(
                "key_track",
                Param::Oscillator(crate::params::OscillatorParam::Detune(crate::Cents::ZERO)),
                "Key Track",
            ))
    }

    /// Both spellings resolve, and the stable `type_id` is what a caller may
    /// always rely on.
    #[test]
    fn find_parameter_accepts_the_type_id_and_the_display_name() {
        let descriptor = key_track_descriptor();
        for spelling in ["key_track", "Key Track", "KEY TRACK", "key track"] {
            assert_eq!(
                descriptor.find_parameter(spelling).map(|p| &p.type_id),
                Some(&"key_track".to_string()),
                "spelling {spelling}"
            );
        }
        assert!(descriptor.find_parameter("nope").is_none());
    }

    /// The reason the lookup hint ranks display names too: `"Key Tracking"` is
    /// 3 edits from the name `"Key Track"` and 4 from the `type_id`
    /// `"key_track"`, so ranking the ids alone answers a recoverable typo with
    /// silence. The answer is still the `type_id` — the spelling that always
    /// parses.
    #[test]
    fn param_lookup_hint_recovers_a_display_name_the_ids_cannot() {
        let descriptor = key_track_descriptor();
        assert_eq!(descriptor.param_id_hint("Key Tracking"), "");
        assert_eq!(
            descriptor.param_lookup_hint("Key Tracking"),
            ". Did you mean 'key_track'?"
        );
    }

    /// A parameter that ranks by both its id and its name must not fill the
    /// hint by itself.
    #[test]
    fn param_lookup_hint_offers_each_parameter_once() {
        let descriptor = key_track_descriptor();
        assert_eq!(
            descriptor.param_lookup_hint("cutof"),
            ". Did you mean 'cutoff'?"
        );
    }

    /// A wrong suggestion is worse than none.
    #[test]
    fn param_hints_stay_quiet_on_nonsense() {
        let descriptor = key_track_descriptor();
        assert_eq!(descriptor.param_id_hint("zzzzzz"), "");
        assert_eq!(descriptor.param_lookup_hint("zzzzzz"), "");
    }

    #[test]
    fn port_type_ids_and_compatibility_are_stable() {
        assert_eq!(
            PortType::ALL.map(PortType::id),
            ["audio", "control", "gate", "midi"]
        );

        assert!(PortType::Audio.can_drive(PortType::Audio));
        assert!(PortType::Audio.can_drive(PortType::Control));
        assert!(!PortType::Audio.can_drive(PortType::Gate));
        assert!(PortType::Control.can_drive(PortType::Audio));
        assert!(PortType::Control.can_drive(PortType::Control));
        assert!(PortType::Control.can_drive(PortType::Gate));
        assert!(PortType::Gate.can_drive(PortType::Control));
        assert!(PortType::Gate.can_drive(PortType::Gate));
        assert!(PortType::Midi.can_drive(PortType::Midi));
        assert!(!PortType::Midi.can_drive(PortType::Audio));
    }

    #[test]
    fn port_value_domains_distinguish_acceptance_from_nominal_range() {
        assert_eq!(PortValueDomain::Audio.id(), "audio");
        assert_eq!(
            PortValueDomain::Audio.nominal_range(),
            Some(PortValueRange::BIPOLAR)
        );
        assert!(
            PortValueDomain::Audio
                .accepted_values()
                .contains("outside the nominal range are allowed")
        );

        assert_eq!(PortValueDomain::Control.nominal_range(), None);
        assert_eq!(
            PortValueDomain::Unipolar.nominal_range(),
            Some(PortValueRange::UNIPOLAR)
        );
        assert_eq!(PortValueDomain::Octaves.unit(), Some("octaves"));
        assert!(PortValueDomain::Gate.accepted_values().contains("> 0.5"));

        let port =
            PortDescriptor::control_output("out", "Out").value_domain(PortValueDomain::Bipolar);
        assert_eq!(port.value_domain, PortValueDomain::Bipolar);
    }

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
    fn param_kind_and_unit_dispatch() {
        use crate::params::{ModMatrixParam, MsegParam, OscillatorParam, SampleId, SamplerParam};
        let freq = Param::Oscillator(OscillatorParam::Frequency(crate::Hertz::new(440.0)));
        assert_eq!(freq.kind(), ParamKind::Continuous);
        assert_eq!(freq.unit(), ParameterUnit::Hertz);
        // `default_curve()` is advisory (Phase 2b) — type-derived, not auto-applied.
        assert_eq!(freq.default_curve(), ResponseCurve::Logarithmic);
        assert_eq!(
            Param::Envelope(crate::params::EnvelopeParam::Attack(crate::Seconds::new(
                0.1
            )))
            .default_curve(),
            ResponseCurve::Exponential
        );
        assert_eq!(
            Param::Mseg(MsegParam::SegmentCount(4)).kind(),
            ParamKind::Integer
        );
        assert_eq!(
            Param::Mseg(MsegParam::LoopEnabled(true)).kind(),
            ParamKind::Bool
        );
        assert_eq!(
            Param::Sampler(SamplerParam::SampleSelect(SampleId::new(0))).kind(),
            ParamKind::Reference
        );
        assert_eq!(
            Param::ModMatrix(ModMatrixParam::SlotSource(0, None)).kind(),
            ParamKind::Reference
        );

        // Constructors seed `kind` from `id.kind()`.
        let d = ParameterDescriptor::float("seg", Param::Mseg(MsegParam::SegmentCount(4)), "Seg");
        assert_eq!(d.kind, ParamKind::Integer);
        assert!(d.choices.is_none());
    }

    #[test]
    fn param_kind_format_is_kind_aware() {
        // Integer: decimal-free, rounds, keeps the unit suffix.
        assert_eq!(ParamKind::Integer.format(ParameterUnit::None, 4.0), "4");
        assert_eq!(ParamKind::Integer.format(ParameterUnit::None, 3.7), "4");
        assert_eq!(
            ParamKind::Integer.format(ParameterUnit::Octaves, 2.0),
            "2 oct"
        );
        // Bool: On/Off.
        assert_eq!(ParamKind::Bool.format(ParameterUnit::None, 1.0), "On");
        assert_eq!(ParamKind::Bool.format(ParameterUnit::None, 0.0), "Off");
        // Continuous: defers to the unit formatter.
        assert_eq!(
            ParamKind::Continuous.format(ParameterUnit::Hertz, 440.0),
            "440.0 Hz"
        );
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
    fn validate_f32_is_kind_aware() {
        use crate::params::MsegParam;
        // Integer: rounds (lenient), then range-checks the rounded value.
        let int =
            ParameterDescriptor::float("segments", Param::Mseg(MsegParam::SegmentCount(4)), "Seg")
                .range(1.0, 16.0);
        assert_eq!(int.validate_f32(4.3), Ok(4.0)); // 4.3 → 4
        assert_eq!(int.validate_f32(15.6), Ok(16.0)); // rounds up, still in range
        assert!(int.validate_f32(20.0).is_err()); // out of range → rejected
        assert!(int.validate_f32(0.4).is_err()); // rounds to 0, below min 1

        // Bool: accepts any finite value (mapped via `> 0.5` downstream).
        let b = ParameterDescriptor::float(
            "loop_enabled",
            Param::Mseg(MsegParam::LoopEnabled(false)),
            "Loop",
        );
        assert_eq!(b.validate_f32(5.0), Ok(5.0));
        assert_eq!(b.validate_f32(0.0), Ok(0.0));
        assert_eq!(b.validate_f32(f32::NAN), Err(ParamValueError::NotFinite));
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
