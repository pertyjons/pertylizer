//! Serializable response types for MCP tools.

use serde::Serialize;

/// Information about an instrument.
#[derive(Debug, Clone, Serialize)]
pub struct InstrumentInfo {
    /// Instrument ID.
    pub id: u64,
    /// Instrument name.
    pub name: String,
    /// Instrument category (e.g. "Drums", "Bass", "Pad", "Lead").
    pub category: String,
    /// MIDI channel (1-16).
    pub midi_channel: u8,
    /// Volume (0.0-2.0).
    pub volume: f32,
    /// Pan (-1.0 = left, 0.0 = center, 1.0 = right).
    pub pan: f32,
    /// Whether the instrument is enabled (not muted).
    pub enabled: bool,
    /// Whether the instrument is muted.
    pub muted: bool,
    /// Whether the instrument is soloed.
    pub solo: bool,
    /// Number of modules in the voice graph.
    pub module_count: usize,
    /// Number of effects in the chain.
    pub effect_count: usize,
}

/// Information about a module in the voice graph.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleInfo {
    /// Module ID string (e.g. "osc-1").
    pub id: String,
    /// Module type (e.g. "Oscillator").
    pub module_type: String,
    /// Human-readable name.
    pub name: String,
    /// Whether the module is bypassed.
    pub bypassed: bool,
    /// Current parameters.
    pub parameters: Vec<ParameterInfo>,
    /// Input port names.
    pub input_ports: Vec<String>,
    /// Output port names.
    pub output_ports: Vec<String>,
}

/// Information about a parameter.
#[derive(Debug, Clone, Serialize)]
pub struct ParameterInfo {
    /// Parameter name (e.g. "frequency").
    pub name: String,
    /// Current value as f32.
    pub value: f32,
    /// Human-readable display value (e.g. "440.0 Hz").
    pub display: String,
    /// Minimum allowed value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    /// Maximum allowed value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
    /// Default value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<f32>,
    /// Allowed choices (for choice/enum parameters).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
}

/// Information about a connection between modules.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionInfo {
    /// Source module ID.
    pub from_module: String,
    /// Source port name.
    pub from_port: String,
    /// Destination module ID.
    pub to_module: String,
    /// Destination port name.
    pub to_port: String,
}

/// Engine status information.
#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    /// CPU usage (0.0-1.0).
    pub cpu_usage: f32,
    /// Number of active voices.
    pub voice_count: u32,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Peak meters (left, right).
    pub peak_left: f32,
    pub peak_right: f32,
    /// RMS meters (left, right).
    pub rms_left: f32,
    pub rms_right: f32,
    /// Master volume (0.0-2.0).
    pub master_volume: f32,
    /// Transport tempo in BPM.
    pub tempo: f32,
    /// Whether transport is playing.
    pub is_playing: bool,
    /// Number of instruments.
    pub instrument_count: usize,
}

/// A diagnostic issue found in the graph.
#[derive(Debug, Clone, Serialize)]
pub struct GraphDiagnostic {
    /// Severity level.
    pub severity: DiagnosticSeverity,
    /// Module ID involved (if applicable).
    pub module_id: Option<String>,
    /// Description of the issue.
    pub message: String,
}

/// Severity of a diagnostic.
#[derive(Debug, Clone, Serialize)]
pub enum DiagnosticSeverity {
    /// Informational note.
    Info,
    /// Potential problem.
    Warning,
    /// Definite problem.
    Error,
}

/// Information about an example patch.
#[derive(Debug, Clone, Serialize)]
pub struct ExamplePatchInfo {
    /// Patch name.
    pub name: String,
    /// Category (e.g. "Bass", "Lead").
    pub category: String,
    /// Short description.
    pub description: String,
    /// Tags for searching.
    pub tags: Vec<String>,
    /// Number of modules.
    pub module_count: usize,
    /// Number of connections.
    pub connection_count: usize,
}

/// Snapshot of the current UI layout.
#[derive(Debug, Clone, Serialize)]
pub struct UiSnapshot {
    /// Name of the loaded patch.
    pub patch_name: String,
    /// All visible modules with position and size.
    pub modules: Vec<UiModuleInfo>,
    /// All connections between modules.
    pub connections: Vec<UiConnectionInfo>,
    /// Window size (width, height).
    pub window_size: (f32, f32),
    /// Module pairs that overlap each other.
    pub overlaps: Vec<UiOverlap>,
}

/// UI information about a single module.
#[derive(Debug, Clone, Serialize)]
pub struct UiModuleInfo {
    /// Module ID string (e.g. "osc-1").
    pub id: String,
    /// Module type (e.g. "Oscillator").
    pub module_type: String,
    /// Display name.
    pub name: String,
    /// Position (x, y) in the workspace.
    pub position: (f32, f32),
    /// Size (width, height).
    pub size: (f32, f32),
    /// Parameters as (name, display_value) pairs.
    pub parameters: Vec<(String, String)>,
}

/// Connection in the UI snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct UiConnectionInfo {
    /// Source module ID.
    pub from_module: String,
    /// Source port name.
    pub from_port: String,
    /// Destination module ID.
    pub to_module: String,
    /// Destination port name.
    pub to_port: String,
}

/// Information about a module parameter including range, unit, and choices.
#[derive(Debug, Clone, Serialize)]
pub struct ParamTypeInfo {
    /// Parameter name (use with set_parameter).
    pub name: String,
    /// Description of what this parameter does.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
    /// Default value.
    pub default: f32,
    /// Unit (e.g. "Hz", "dB", "%", "ms", "s").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// Allowed choices for enum/discrete parameters (id → display name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<ChoiceInfo>>,
}

/// A named choice for an enum/discrete parameter.
#[derive(Debug, Clone, Serialize)]
pub struct ChoiceInfo {
    /// Value to pass to set_parameter (float index: 0.0, 1.0, 2.0, ...).
    pub value: f32,
    /// Choice identifier string.
    pub id: String,
    /// Display name.
    pub name: String,
}

/// Information about a module port.
#[derive(Debug, Clone, Serialize)]
pub struct PortTypeInfo {
    /// Port name (use with connect/disconnect).
    pub name: String,
    /// Signal type: "audio", "control", "gate", or "midi".
    pub signal_type: String,
}

/// Information about an available module type.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleTypeInfo {
    /// Type key to pass to add_module (e.g. "oscillator", "filter").
    pub type_key: String,
    /// Display name (e.g. "Oscillator", "Math Oscillator").
    pub name: String,
    /// Module description.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Category: "voice", "effect", or "visualizer".
    pub category: String,
    /// Input ports with signal type info.
    pub input_ports: Vec<PortTypeInfo>,
    /// Output ports with signal type info.
    pub output_ports: Vec<PortTypeInfo>,
    /// Parameters with range, unit, and choice metadata.
    pub parameters: Vec<ParamTypeInfo>,
    /// Typical signal flow hint (e.g. "osc → filter → amp → out").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_flow_hint: Option<String>,
}

/// Two modules that overlap in the UI.
#[derive(Debug, Clone, Serialize)]
pub struct UiOverlap {
    /// First module ID.
    pub module_a: String,
    /// Second module ID.
    pub module_b: String,
    /// Area of overlap in square pixels.
    pub overlap_area: f32,
}

// === Batch operation types ===

/// Result for a single item in a batch operation.
#[derive(Debug, Clone, Serialize)]
pub struct BatchItemResult {
    /// Zero-based index of the item in the input array.
    pub index: usize,
    /// Whether this item succeeded.
    pub success: bool,
    /// Assigned ID on create, None on update/failure.
    pub id: Option<u64>,
    /// Error message if this item failed.
    pub error: Option<String>,
}

/// Aggregate result for a batch operation.
#[derive(Debug, Clone, Serialize)]
pub struct BatchResult {
    /// Total number of items in the batch.
    pub total: usize,
    /// Number of items that succeeded.
    pub succeeded: usize,
    /// Number of items that failed.
    pub failed: usize,
    /// Per-item results.
    pub items: Vec<BatchItemResult>,
}

/// Result of a `set_song` operation that builds a full song in one call.
#[derive(Debug, Clone, Serialize)]
pub struct SetSongResult {
    /// Number of patterns created.
    pub patterns_created: usize,
    /// Number of tracks created.
    pub tracks_created: usize,
    /// Number of notes added across all patterns.
    pub notes_added: usize,
    /// Number of arrangement placements created.
    pub placements_created: usize,
    /// Pattern IDs in the same order as the input array.
    pub pattern_ids: Vec<u32>,
    /// Track IDs in the same order as the input array.
    pub track_ids: Vec<u16>,
    /// Any errors that occurred during the operation.
    pub errors: Vec<String>,
}

// === Batch instrument building types ===

/// Result of building a single instrument via `build_instrument`.
#[derive(Debug, Clone, Serialize)]
pub struct BuildInstrumentResult {
    /// Assigned instrument ID.
    pub instrument_id: u64,
    /// Module IDs in the same order as the input modules array.
    pub module_ids: Vec<String>,
    /// Number of connections successfully created.
    pub connection_count: usize,
    /// Non-fatal errors encountered.
    pub errors: Vec<String>,
    /// Hint for the caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Result of applying an example patch via `apply_example_patch`.
#[derive(Debug, Clone, Serialize)]
pub struct ApplyExamplePatchResult {
    /// Instrument ID (created or reused).
    pub instrument_id: u64,
    /// Name of the patch that was applied.
    pub patch_name: String,
    /// Number of modules created.
    pub module_count: usize,
    /// Number of connections created.
    pub connection_count: usize,
    /// Non-fatal errors encountered.
    pub errors: Vec<String>,
}

// === Sequencer types ===

/// Information about the current song.
#[derive(Debug, Clone, Serialize)]
pub struct SongInfo {
    /// Song name.
    pub name: String,
    /// Song author.
    pub author: String,
    /// Default tempo in BPM.
    pub tempo: f32,
    /// Time signature as "numerator/denominator".
    pub time_signature: String,
    /// Total song length in seconds.
    pub length_seconds: f64,
    /// Number of patterns.
    pub pattern_count: usize,
    /// Number of tracks.
    pub track_count: usize,
}

/// Information about a pattern.
#[derive(Debug, Clone, Serialize)]
pub struct PatternInfo {
    /// Pattern ID.
    pub id: u32,
    /// Pattern name.
    pub name: String,
    /// Length in beats.
    pub length_beats: f32,
    /// Number of notes.
    pub note_count: usize,
}

/// Information about a note in a pattern.
#[derive(Debug, Clone, Serialize)]
pub struct NoteInfo {
    /// Note ID.
    pub id: u64,
    /// MIDI pitch (0-127).
    pub pitch: u8,
    /// Human-readable pitch name (e.g. "C4", "A#3").
    pub pitch_name: String,
    /// Start position in beats.
    pub start_beat: f32,
    /// Duration in beats.
    pub duration_beats: f32,
    /// Velocity (0-127).
    pub velocity: u8,
}

/// Information about a sequencer track.
#[derive(Debug, Clone, Serialize)]
pub struct TrackInfo {
    /// Track ID.
    pub id: u16,
    /// Track name.
    pub name: String,
    /// Instrument ID (if assigned).
    pub instrument_id: Option<u16>,
    /// Volume (0.0-1.0).
    pub volume: f32,
    /// Pan (-1.0 = left, 0.0 = center, 1.0 = right).
    pub pan: f32,
    /// Whether the track is muted.
    pub mute: bool,
    /// Whether the track is soloed.
    pub solo: bool,
}

/// Information about a pattern placement in the arrangement.
#[derive(Debug, Clone, Serialize)]
pub struct PlacementInfo {
    /// Pattern ID.
    pub pattern_id: u32,
    /// Track ID.
    pub track_id: u16,
    /// Start position in beats.
    pub start_beat: f32,
}

// === Automation types ===

/// Information about an automation lane in a pattern.
#[derive(Debug, Clone, Serialize)]
pub struct AutomationLaneInfo {
    /// Target parameter name (e.g. "Volume", "Pan").
    pub target: String,
    /// Instrument ID (if instrument-targeted).
    pub instrument_id: Option<u16>,
    /// Number of automation points.
    pub point_count: usize,
}

/// Information about an automation point.
#[derive(Debug, Clone, Serialize)]
pub struct AutomationPointInfo {
    /// Position in beats.
    pub beat: f32,
    /// Normalized value (0.0-1.0).
    pub value: f32,
    /// Interpolation curve type (Linear, Step, Exponential, SCurve).
    pub curve: String,
}

/// Full data for an example patch (used by MCP resources).
#[derive(Debug, Clone, Serialize)]
pub struct PatchResourceData {
    /// Patch name.
    pub name: String,
    /// Category (e.g. "Bass", "Lead").
    pub category: String,
    /// Short description.
    pub description: String,
    /// Tags for searching.
    pub tags: Vec<String>,
    /// Modules in the patch.
    pub modules: Vec<PatchModuleInfo>,
    /// Connections between modules.
    pub connections: Vec<UiConnectionInfo>,
}

/// Module info within a patch resource.
#[derive(Debug, Clone, Serialize)]
pub struct PatchModuleInfo {
    /// Module ID (e.g. "osc-1").
    pub id: String,
    /// Module type key.
    pub module_type: String,
    /// Parameters as (name, value) pairs.
    pub parameters: Vec<PatchParamInfo>,
}

/// A parameter name+value pair in a patch resource.
#[derive(Debug, Clone, Serialize)]
pub struct PatchParamInfo {
    /// Parameter name.
    pub name: String,
    /// Parameter value (numeric, string choice, or bool).
    pub value: PatchParamValue,
}

/// Parameter value variants for patch resources.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum PatchParamValue {
    /// Numeric value.
    Float(f32),
    /// Integer value.
    Int(i32),
    /// Boolean value.
    Bool(bool),
    /// Choice/enum value (e.g. "sawtooth").
    Choice(String),
}

/// Result of optimizing a project by removing unused items.
#[derive(Debug, Clone, Serialize)]
pub struct OptimizeResult {
    /// Names of removed patterns.
    pub removed_patterns: Vec<String>,
    /// Names of removed tracks.
    pub removed_tracks: Vec<String>,
    /// Names of removed instruments.
    pub removed_instruments: Vec<String>,
    /// Total number of items removed.
    pub total_removed: usize,
}

/// Audio preview data returned by offline rendering.
#[derive(Debug, Clone)]
pub struct AudioPreview {
    /// Raw WAV file bytes.
    pub wav_data: Vec<u8>,
    /// Sample rate used for rendering.
    pub sample_rate: u32,
    /// Duration in seconds (note + tail).
    pub duration_seconds: f32,
}

// === AWE (Acoustic World Engine) types ===

/// Full AWE state returned by `get_awe_state`.
#[derive(Debug, Clone, Serialize)]
pub struct AweStateInfo {
    /// Whether AWE is enabled.
    pub enabled: bool,
    /// Current room shape (e.g. "Box", "Cylinder", "Sphere").
    pub room_shape: String,
    /// Room dimensions as a human-readable string (e.g. "8.0 x 5.0 x 3.0 m").
    pub room_dimensions: String,
    /// Room length in meters (x-axis).
    pub room_length: f32,
    /// Room width in meters (y-axis).
    pub room_width: f32,
    /// Room height in meters (z-axis).
    pub room_height: f32,
    /// Room volume in cubic meters.
    pub room_volume: f32,
    /// Wall material name (e.g. "Concrete", "Wood", "Glass").
    pub material: String,
    /// Source position [x, y, z] in meters.
    pub source_position: [f32; 3],
    /// Listener position [x, y, z] in meters.
    pub listener_position: [f32; 3],
    /// Dry/wet mix (0.0 = fully dry, 1.0 = fully wet).
    pub dry_wet: f32,
    /// Early/late reflection balance (0.0 = early only, 1.0 = late only).
    pub early_late_balance: f32,
    /// Room mode resonance amount (0.0 = off, 1.0 = full).
    pub modes_amount: f32,
    /// Frequency warping (-1.0 to 1.0). Negative = lower modes, positive = higher modes.
    pub freq_warp: f32,
    /// Resonance boost for room modes (0.0-1.0).
    pub resonance_boost: f32,
    /// Tail stretch factor (0.5 = shorter, 1.0 = natural, 4.0 = longest).
    pub tail_stretch: f32,
    /// Portal amount (0.0 = off, 1.0 = full acoustic portal effect).
    pub portal_amount: f32,
    /// Pre-delay in milliseconds (0-200 ms delay before first reflection).
    pub pre_delay_ms: f32,
    /// FDN chorus modulation depth (0.0-1.0).
    pub modulation_depth: f32,
    /// FDN chorus modulation rate in Hz (0.01-20.0).
    pub modulation_rate: f32,
    /// Air absorption amount (0.0-1.0, high-frequency damping over distance).
    pub air_absorption: f32,
    /// Stereo width (0.0 = mono, 1.0 = full stereo).
    pub width: f32,
    /// High-cut frequency in Hz (200-20000).
    pub high_cut: f32,
    /// Low-cut frequency in Hz (20-2000).
    pub low_cut: f32,
    /// Temperature in Celsius (affects speed of sound).
    pub temperature: f32,
    /// Per-voice spatial processing enabled.
    pub spatial_enabled: bool,
    /// Note-to-position mapping ("Off", "LinearX", "LinearY", "Circular").
    pub note_mapping: String,
    /// LFO states (4 LFOs).
    pub lfos: Vec<AweLfoInfo>,
}

/// State of one AWE LFO.
#[derive(Debug, Clone, Serialize)]
pub struct AweLfoInfo {
    /// LFO index (1-4).
    pub index: u8,
    /// Rate in Hz (0.01-20.0).
    pub rate: f32,
    /// Modulation amount (0.0-1.0).
    pub amount: f32,
    /// Modulation target name.
    pub target: String,
}

/// Info about an AWE preset.
#[derive(Debug, Clone, Serialize)]
pub struct AwePresetInfo {
    /// Preset name.
    pub name: String,
    /// Short description of the acoustic character.
    pub description: String,
}

// ============================================================================
// SAMPLE TYPES
// ============================================================================

/// Information about a sample in the library.
#[derive(Debug, Clone, Serialize)]
pub struct SampleInfo {
    /// Sample ID.
    pub id: u64,
    /// Display name.
    pub name: String,
    /// Duration in seconds.
    pub duration_seconds: f64,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of channels (1=mono, 2=stereo).
    pub channels: u16,
    /// Total frames.
    pub frame_count: usize,
    /// Root MIDI note (if set).
    pub root_note: Option<u8>,
    /// Whether loop is enabled.
    pub loop_enabled: bool,
    /// Whether crop is set.
    pub has_crop: bool,
    /// Source type (recorded, imported, generated).
    pub source: String,
}

/// Detailed information about a sample including audio statistics.
#[derive(Debug, Clone, Serialize)]
pub struct DetailedSampleInfo {
    /// Basic sample info.
    #[serde(flatten)]
    pub info: SampleInfo,
    /// Peak amplitude (0.0 - 1.0+).
    pub peak_level: f32,
    /// RMS level.
    pub rms_level: f32,
    /// DC offset (average sample value).
    pub dc_offset: f32,
    /// Memory usage in bytes.
    pub memory_bytes: usize,
    /// Loop region start in seconds (if set).
    pub loop_start_seconds: Option<f64>,
    /// Loop region end in seconds (if set).
    pub loop_end_seconds: Option<f64>,
    /// Crop start in seconds (if set).
    pub crop_start_seconds: Option<f64>,
    /// Crop end in seconds (if set).
    pub crop_end_seconds: Option<f64>,
}

/// State of a Sampler module in the rack.
#[derive(Debug, Clone, Serialize)]
pub struct SamplerStateInfo {
    /// Currently assigned sample ID (0 if none).
    pub sample_id: u64,
    /// Name of assigned sample (empty if none).
    pub sample_name: String,
    /// Pitch tracking enabled.
    pub pitch_tracking: bool,
    /// Volume level (0.0 - 1.0).
    pub level: f32,
    /// Play mode: "one_shot", "sustain", or "loop".
    pub play_mode: String,
    /// Direction: "forward", "reverse", or "ping_pong".
    pub direction: String,
    /// Velocity sensitivity (0.0 - 1.0).
    pub velocity_sensitivity: f32,
    /// Fine tune in cents (-100 to +100).
    pub fine_tune: f32,
    /// Start offset (0.0 - 1.0).
    pub start_offset: f32,
}

// ============================================================================
// DISCOVERY TYPES
// ============================================================================

/// Information about a port signal type (returned by `list_port_types`).
#[derive(Debug, Clone, Serialize)]
pub struct PortSignalTypeInfo {
    /// Signal type identifier (matches `PortTypeInfo::signal_type`).
    pub signal_type: String,
    /// Human-readable description.
    pub description: String,
    /// Value range hint.
    pub value_range: String,
    /// Which other signal types this can connect to.
    pub compatible_with: Vec<String>,
}

/// Result of checking whether a connection between two ports is valid.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionCheckResult {
    /// Whether the connection is valid.
    pub valid: bool,
    /// Signal type of the source port (if found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_signal_type: Option<String>,
    /// Signal type of the destination port (if found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_signal_type: Option<String>,
    /// Explanation of why the connection is valid or invalid.
    pub message: String,
    /// Actionable hint for the caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

// ============================================================================
// BATCH EXECUTE TYPES
// ============================================================================

/// Result of a single operation in a `batch_execute` call.
#[derive(Debug, Clone, Serialize)]
pub struct BatchExecItemResult {
    /// Zero-based index of the operation in the input array.
    pub index: usize,
    /// Tool name that was called.
    pub tool: String,
    /// Whether the operation succeeded.
    pub success: bool,
    /// Result string (JSON or message) on success, error message on failure.
    pub result: String,
}

/// Aggregate result for a `batch_execute` call.
#[derive(Debug, Clone, Serialize)]
pub struct BatchExecResult {
    /// Total number of operations.
    pub total: usize,
    /// Number of operations that succeeded.
    pub succeeded: usize,
    /// Number of operations that failed.
    pub failed: usize,
    /// Per-operation results.
    pub results: Vec<BatchExecItemResult>,
}

/// Current audio input state.
#[derive(Debug, Clone, Serialize)]
pub struct InputStateInfo {
    /// Current state: "idle", "monitoring", or "recording".
    pub state: String,
    /// Current peak level (0.0 to 1.0+).
    pub peak_level: f32,
    /// Recording duration in seconds (0 if not recording).
    pub recorded_seconds: f64,
    /// Whether the input stream is active.
    pub is_active: bool,
}

/// Info about an audio input device.
#[derive(Debug, Clone, Serialize)]
pub struct InputDeviceInfo {
    /// Device name/ID.
    pub name: String,
    /// Number of input channels.
    pub input_channels: u16,
}
