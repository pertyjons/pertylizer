//! Error types for the MCP bridge.

/// Errors that can occur in the MCP bridge.
#[derive(Debug, thiserror::Error)]
pub enum McpBridgeError {
    /// Instrument not found.
    #[error("instrument not found: {0}")]
    InstrumentNotFound(u64),

    /// Module not found.
    #[error("module not found: {0}")]
    ModuleNotFound(String),

    /// Parameter not found.
    #[error("parameter not found: {0}")]
    ParameterNotFound(String),

    /// Command failed to send to the audio engine.
    #[error("failed to send {command} command to audio engine")]
    CommandSendFailed {
        /// Which command was being sent.
        command: &'static str,
    },

    /// Example patch not found.
    #[error("example patch not found: {0}")]
    PatchNotFound(String),

    /// Invalid module type.
    #[error("invalid module type: {0}")]
    InvalidModuleType(String),

    /// Port not found on module.
    #[error("port '{port}' not found on module '{module}', available: {available}")]
    PortNotFound {
        module: String,
        port: String,
        available: String,
    },

    /// Pattern not found.
    #[error("pattern not found: {0}")]
    PatternNotFound(u32),

    /// Note not found.
    #[error("note not found: {0}")]
    NoteNotFound(u64),

    /// Track not found.
    #[error("track not found: {0}")]
    TrackNotFound(u16),

    /// Return bus not found.
    #[error("return bus not found: {0}")]
    ReturnBusNotFound(u16),

    /// MIDI note out of valid range.
    #[error("invalid MIDI note {0}: must be 0-127")]
    InvalidMidiNote(u8),

    /// MIDI velocity out of valid range.
    #[error("invalid velocity {0}: must be 0-127")]
    InvalidVelocity(u8),

    /// MIDI channel out of valid range.
    #[error("invalid MIDI channel {0}: must be 1-16")]
    InvalidMidiChannel(u8),

    /// Numeric parameter out of valid range.
    #[error("{name} value {value} out of range: must be {min}..={max}")]
    ValueOutOfRange {
        name: &'static str,
        value: f32,
        min: f32,
        max: f32,
    },

    /// A module parameter value was rejected by descriptor-driven validation.
    #[error("invalid value for parameter '{name}': {source}")]
    InvalidParameterValue {
        /// The parameter that was being set.
        name: String,
        /// The underlying validation failure (range / finiteness).
        #[source]
        source: synth_core::ParamValueError,
    },

    /// Invalid instrument category string.
    #[error(
        "invalid instrument category '{0}': valid categories are Drums, Bass, Lead, Pad, Keys, Strings, Brass, Woodwind, FX, Vocal, Other"
    )]
    InvalidCategory(String),

    /// A parameter value (string choice or boolean) could not be applied.
    #[error("invalid value '{value}' for parameter '{name}': {detail}")]
    InvalidChoice {
        /// The parameter that was being set.
        name: String,
        /// The value the client supplied.
        value: String,
        /// Why it was rejected (valid choices, or that the parameter is numeric).
        detail: String,
    },

    /// Index out of bounds in a batch operation.
    #[error("{name} index {index} out of bounds: only {count} items available")]
    IndexOutOfBounds {
        name: &'static str,
        index: usize,
        count: usize,
    },

    /// Empty name not allowed.
    #[error("{kind} name must not be empty")]
    EmptyName { kind: &'static str },

    /// A free-text description exceeded the hard length limit.
    #[error("description too long: {len} chars (max {max})")]
    DescriptionTooLong {
        /// The length of the supplied description, in characters.
        len: usize,
        /// The hard maximum.
        max: usize,
    },

    /// Invalid AWE parameter name.
    #[error(
        "unknown AWE parameter '{0}'. Valid parameters: dry_wet, early_late_balance, modes_amount, \
         freq_warp, resonance_boost, tail_stretch, portal_amount, pre_delay, modulation_depth, \
         modulation_rate, air_absorption, width, high_cut, low_cut, temperature, source_x, \
         source_y, listener_x, listener_y"
    )]
    InvalidAweParameter(String),

    /// AWE preset not found.
    #[error("AWE preset '{0}' not found. Use list_awe_presets to see available presets.")]
    AwePresetNotFound(String),

    /// Invalid AWE LFO index.
    #[error("invalid LFO index {0}: must be 1-4")]
    InvalidLfoIndex(u8),

    /// A requested analysis window starts at or beyond the end of the audio.
    #[error(
        "analysis window starts at {start_ms}ms but only {available_ms}ms of audio is available"
    )]
    WindowOutOfBounds {
        /// Requested window start, in milliseconds.
        start_ms: f32,
        /// Total audio available, in milliseconds.
        available_ms: f32,
    },

    /// Generic error — use sparingly; prefer a specific variant when the failure
    /// mode is known so MCP clients can programmatically handle it.
    #[error("{0}")]
    Other(String),
}
