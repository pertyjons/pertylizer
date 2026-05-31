//! Serializable response types for MCP tools.

use serde::{Deserialize, Serialize};

/// Information about an instrument.
#[derive(Debug, Clone, Serialize)]
pub struct InstrumentInfo {
    /// Instrument ID.
    pub id: u64,
    /// Instrument name.
    pub name: String,
    /// Free-text description / intent — `""` when not set. Skipped from
    /// JSON when empty so older clients don't see surprising new fields.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Patch-level description, separate from the per-instrument
    /// description above. Skipped from JSON when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_description: Option<String>,
    /// Sidechain source instrument id, or `None` when no sidechain is
    /// configured. Skipped from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidechain_source_id: Option<u64>,
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
    /// Output port names. For Mod Matrix modules this is `["matrix"]` — a
    /// virtual port marker so callers don't mistake the matrix for a dead
    /// module just because it has no audio/CV cables.
    pub output_ports: Vec<String>,
    /// Active modulation routings carried by this module. Populated only
    /// for Mod Matrix modules; absent on every other module type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_matrix_routings: Option<Vec<MatrixRoutingInfo>>,
}

/// One row of a Mod Matrix's routing table.
#[derive(Debug, Clone, Serialize)]
pub struct MatrixRoutingInfo {
    /// 1-based slot index (1..=16).
    pub slot: u8,
    /// Source semantic ID (e.g. `"lfo-1"`, `"env-2"`, or for non-module
    /// sources the source key `"velocity"`, `"mod_wheel"`, etc.).
    pub source: String,
    /// Source display name (e.g. `"LFO 1"`, `"Velocity"`).
    pub source_name: String,
    /// Destination as `"module-id.param"` (e.g. `"flt-1.cutoff"`).
    pub destination: String,
    /// Destination display name (e.g. `"Filter 1 Cutoff"`).
    pub destination_name: String,
    /// Modulation amount in `-1.0..=1.0`.
    pub amount: f32,
    /// Whether the slot is enabled (an enabled slot can still be inactive
    /// if source or destination is `None`).
    pub enabled: bool,
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
    /// Stable descriptor identifier (snake_case, e.g. "cutoff"). This is the
    /// `param_id` used to build a `module:<type>:<instance>:<param_id>`
    /// automation target — distinct from the human-readable `name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    /// Whether this parameter may be a sequencer automation target (continuous
    /// + RT-safe, non-enum). Mirrors `ParameterDescriptor::is_automatable`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_automatable: Option<bool>,
    /// Value-mapping curve (e.g. "Linear", "Logarithmic", "Exponential"),
    /// mirroring `ParameterDescriptor::response_curve`. Tells a client how the
    /// native value maps onto a 0..1 control so it can place sliders/automation
    /// sensibly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_curve: Option<String>,
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

/// Result of `insert_module_between`: a new module spliced into an existing
/// audio cable.
#[derive(Debug, Clone, Serialize)]
pub struct InsertModuleResult {
    /// ID of the newly created module (e.g. "flt-2").
    pub module_id: String,
    /// The cable that was cut to make room, as `"from:port → to:port"`.
    pub removed_connection: String,
    /// The two cables that now route source → new module → destination.
    pub new_connections: Vec<String>,
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

/// Per-instrument diagnostics within a project-wide load-lint pass. Present only
/// for instruments that have at least one *actionable* (`Warning`/`Error`)
/// diagnostic; when included, carries **all** of that instrument's diagnostics
/// (including any `Info` notes) for context.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectLintEntry {
    /// Instrument the diagnostics belong to.
    pub instrument_id: u64,
    /// Instrument name (for human-readable reports).
    pub instrument_name: String,
    /// All graph diagnostics found for this instrument (any severity).
    pub diagnostics: Vec<GraphDiagnostic>,
}

/// Project-wide load-lint report: runs the graph diagnostics over every
/// instrument and aggregates the results. Surfaces *behavioural* warnings
/// (unconnected ports, silent voices, feedback loops, …) that schema validation
/// alone can't catch. A clean project has `error_count == 0 && warning_count == 0`,
/// which lets a CI gate or a post-load check assert health in one call.
///
/// `entries` lists only instruments with an actionable (`Warning`/`Error`)
/// diagnostic, so a fresh project full of empty instruments (which emit only an
/// `Info` "graph is empty" note) doesn't flood it. The counts are project-wide
/// totals over *every* instrument, so they are **not** derivable from `entries`
/// alone — `info_count` in particular includes notes from instruments that never
/// appear in `entries`.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectLintReport {
    /// Number of instruments inspected.
    pub instruments_checked: usize,
    /// Total `Error`-severity diagnostics across all instruments.
    pub error_count: usize,
    /// Total `Warning`-severity diagnostics across all instruments.
    pub warning_count: usize,
    /// Total `Info`-severity diagnostics across all instruments (including those
    /// omitted from `entries` because they have no actionable diagnostic).
    pub info_count: usize,
    /// One entry per instrument with at least one `Warning`/`Error` diagnostic.
    pub entries: Vec<ProjectLintEntry>,
}

/// The authoritative on-disk JSON Schema for `.pertyproj` project files, paired
/// with the format and build versions.
///
/// Returned by `get_project_schema`. The `schema` is the exact committed
/// `schemas/project.schema.json` (embedded at build time and passed through
/// verbatim), not a live re-derived copy — so external tooling diffing against it
/// sees zero introspection-vs-disk drift.
///
/// To detect a *format* change, pin `schema_format_version`: it bumps only when
/// the on-disk project format changes. `app_version` is a build stamp that moves
/// every release, so it is **not** a reliable format-change signal.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectSchemaInfo {
    /// File name of the schema artifact (e.g. `"project.schema.json"`).
    pub schema_file: String,
    /// The `.pertyproj` format version — bumped only when the on-disk format
    /// changes. Pin this for format-change detection.
    pub schema_format_version: String,
    /// Build version of the application (a release stamp; changes every release,
    /// so not a format-change signal — use `schema_format_version` for that).
    pub app_version: String,
    /// The full JSON Schema document, passed through verbatim from the committed
    /// artifact (no parse-then-reserialize round-trip).
    pub schema: Box<serde_json::value::RawValue>,
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
    /// What this choice does, for discovery and tooltips.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
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
    /// Free-text description / intent — `""` when not set. Skipped from JSON
    /// when empty. Set via `set_song_description`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
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
    /// Transport loop active. When true, playback wraps from `loop_end_beats`
    /// back to `loop_start_beats`. Set via `set_transport_loop`.
    pub transport_loop_enabled: bool,
    /// Transport loop start in beats. Only meaningful when
    /// `transport_loop_enabled` is true.
    pub transport_loop_start_beats: f32,
    /// Transport loop end (exclusive) in beats. Only meaningful when
    /// `transport_loop_enabled` is true.
    pub transport_loop_end_beats: f32,
}

/// Information about a pattern.
#[derive(Debug, Clone, Serialize)]
pub struct PatternInfo {
    /// Pattern ID.
    pub id: u32,
    /// Pattern name.
    pub name: String,
    /// Free-text description / intent — `""` when not set. Skipped from JSON
    /// when empty. Set via `set_pattern_description`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
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
    /// Free-text description / intent (e.g. "kick layer") — `""` when not set.
    /// Skipped from JSON when empty. Set via `set_track_description`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Track color as "#RRGGBB". Set via `set_track_color`.
    pub color: String,
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
    /// Effect sends from this track to return busses. Empty when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<SendInfo>,
}

/// An effect send from a track to a return bus.
#[derive(Debug, Clone, Serialize)]
pub struct SendInfo {
    /// Destination return-bus ID.
    pub target: u16,
    /// Send level (0.0 = none, 1.0 = unity).
    pub level: f32,
    /// `true` = pre-fader tap, `false` = post-fader.
    pub pre_fader: bool,
    /// `true` = active, `false` = bypassed (kept but contributes nothing).
    pub enabled: bool,
}

/// Information about a return bus (effect-send destination).
#[derive(Debug, Clone, Serialize)]
pub struct ReturnBusInfo {
    /// Return-bus ID (referenced by track sends).
    pub id: u16,
    /// Display name.
    pub name: String,
    /// Output fader level (0.0-1.0).
    pub volume: f32,
    /// Output pan (-1.0 = left, 0.0 = center, 1.0 = right).
    pub pan: f32,
    /// Whether the bus is muted.
    pub mute: bool,
    /// Whether the bus is soloed.
    pub solo: bool,
    /// Display color as "#RRGGBB".
    pub color: String,
    /// Free-text description / intent — `""` when not set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Insert effects in processing order (the bus's effect chain). Edited via
    /// `add_return_effect` / `remove_return_effect` / `set_return_effect_parameter`
    /// / `set_return_effect_enabled` / `reorder_return_effect`.
    pub effects: Vec<ReturnEffectInfo>,
    /// Bus-to-bus send taps from this return into other return busses. Edited via
    /// `set_return_send` / `remove_return_send`.
    pub sends: Vec<ReturnSendInfo>,
}

/// A bus-to-bus send tap (one return feeding another).
#[derive(Debug, Clone, Serialize)]
pub struct ReturnSendInfo {
    /// Destination return-bus ID.
    pub target: u16,
    /// Send level (0.0 = none, 1.0 = unity).
    pub level: f32,
    /// `true` = active, `false` = bypassed.
    pub enabled: bool,
}

/// One insert effect on a return bus's effect chain.
#[derive(Debug, Clone, Serialize)]
pub struct ReturnEffectInfo {
    /// Module-id string (e.g. "rev-1"), unique within the bus's chain. Pass this
    /// to the per-effect tools (`remove_return_effect`, `set_return_effect_parameter`).
    pub module_id: String,
    /// Effect type key (e.g. "rev", "dly") — accepted by `add_return_effect`.
    pub effect_type: String,
    /// Whether the effect is currently bypassed.
    pub bypassed: bool,
    /// Current parameter values, in descriptor order.
    pub parameters: Vec<ReturnEffectParamInfo>,
}

/// One parameter on a return-bus insert effect.
#[derive(Debug, Clone, Serialize)]
pub struct ReturnEffectParamInfo {
    /// Display name (e.g. "Mix").
    pub name: String,
    /// Stable type id — pass this as `param_name` to `set_return_effect_parameter`.
    pub type_id: String,
    /// Current value in the parameter's native units.
    pub value: f32,
    /// Formatted value with unit (e.g. "0.35", "250 ms").
    pub display: String,
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

/// A valid automation target for an instrument, returned by
/// `get_instrument_automation_targets`. The `target` string is ready to pass to
/// the automation tools (`add/get/remove/clear_automation_*`).
#[derive(Debug, Clone, Serialize)]
pub struct AutomationTargetInfo {
    /// Target string for the automation tools (e.g. "module:flt:4:cutoff" or
    /// "FilterCutoff").
    pub target: String,
    /// "module" for a per-module parameter, "instrument" for a macro.
    pub kind: String,
    /// Module id string (e.g. "flt-4") — module targets only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_id: Option<String>,
    /// Descriptor `param_id` (snake_case) — module targets only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_id: Option<String>,
    /// Human-readable parameter name (e.g. "Cutoff").
    pub display_name: String,
    /// Unit (e.g. "Hertz", "Seconds"), when the parameter has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Minimum value (module targets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f32>,
    /// Maximum value (module targets).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f32>,
    /// Response curve (e.g. "Linear", "Logarithmic") — module targets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_curve: Option<String>,
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

/// One automation lane in the project-wide `get_automation_summary`.
#[derive(Debug, Clone, Serialize)]
pub struct AutomationSummaryLane {
    /// Pattern the lane lives in.
    pub pattern_id: u32,
    /// Pattern name (for human-readable reports).
    pub pattern_name: String,
    /// Automation target string (e.g. "module:flt:1:cutoff" or "Volume").
    pub target: String,
    /// Instrument the lane targets, when instrument-scoped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument_id: Option<u16>,
    /// Number of points in the lane.
    pub point_count: usize,
}

/// One group (by instrument / target / pattern) in `get_automation_summary`.
#[derive(Debug, Clone, Serialize)]
pub struct AutomationSummaryGroup {
    /// Group key (depends on `group_by`).
    pub group: String,
    /// Number of lanes in this group.
    pub lane_count: usize,
    /// Total automation points across the group.
    pub point_count: usize,
    /// The lanes in this group.
    pub lanes: Vec<AutomationSummaryLane>,
}

/// Project-wide automation overview returned by `get_automation_summary`.
#[derive(Debug, Clone, Serialize)]
pub struct AutomationSummaryResult {
    /// How the lanes were grouped ("instrument", "target", or "pattern").
    pub group_by: String,
    /// Total automation lanes across all patterns.
    pub total_lanes: usize,
    /// Total automation points across all patterns.
    pub total_points: usize,
    /// Grouped lanes, sorted by group key.
    pub groups: Vec<AutomationSummaryGroup>,
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
    /// Sample id — kept lossless as u64. The MCP resource view used to
    /// down-cast to `Int(i32)`, which silently truncated ids ≥ 2³¹.
    SampleId(u64),
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
    /// Names of removed samples (no `Sampler` module's `sample_select`
    /// referenced them). Pruning samples keeps `SampleLibrary` empty
    /// when nothing uses it, which in turn lets the next save use the
    /// plain JSON format instead of being forced into a bundle.
    pub removed_samples: Vec<String>,
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

/// Single spectral peak (frequency and magnitude).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AnalyzeSpectrumPeak {
    /// Peak frequency in Hz (parabolically interpolated).
    pub freq_hz: f32,
    /// Magnitude in dB relative to the loudest peak in the window
    /// (loudest = 0 dB, quieter peaks negative).
    pub magnitude_db: f32,
}

/// Spectral energy split into 4 frequency bands. Use `sub`/`low` to confirm
/// "is this actually a bass" vs `mid`/`high` for "leads / brightness". Each
/// value is an RMS amplitude in 0.0..~1.0; not all four sum to `rms_overall`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AnalyzeEnergyBands {
    /// 0-100 Hz energy (sub).
    pub sub: f32,
    /// 100-500 Hz energy (low/mid bass).
    pub low: f32,
    /// 500-2000 Hz energy (presence).
    pub mid: f32,
    /// 2000+ Hz energy (brilliance).
    pub high: f32,
}

/// Harmonic structure of a tonal signal at a known fundamental. Tells you
/// whether a saw is clean, a square is really squarey, whether tube
/// saturation has added even-order content, etc.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AnalyzeHarmonicContent {
    /// Total harmonic distortion in dB (power ratio of harmonics to
    /// fundamental). More negative = cleaner. -120 dB ≈ silent / no harmonics.
    pub thd_db: f32,
    /// Power ratio of odd harmonics to even harmonics in dB. Positive =
    /// odd-dominant (square / clean clipping); negative = even-dominant
    /// (asymmetric clipping, tube saturation, fold offsets). 0.0 when
    /// either side is empty.
    pub odd_even_ratio_db: f32,
    /// Count of harmonics above the noise floor (capped at 20).
    pub n_harmonics: u32,
}

/// Estimated ADSR-like envelope shape, derived from `rms_envelope`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct AnalyzeEnvelopeEstimate {
    /// Time from start to RMS peak, in ms.
    pub attack_ms: f32,
    /// Time from peak down to ~120 % of sustain level, in ms.
    pub decay_ms: f32,
    /// Average sustain RMS divided by peak RMS (0.0..1.0).
    pub sustain_level: f32,
    /// Time from note-off until RMS falls below 5 % of peak, in ms.
    pub release_ms: f32,
}

/// Boolean quality flags. Cheap one-glance status for an LLM agent.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct AnalyzeFlags {
    /// True when `peak_amplitude < 0.005` — patch produced essentially
    /// nothing audible.
    pub silent: bool,
    /// True when `clipped_samples > 0` — the engine output reached
    /// f32 fullscale at least once.
    pub clipping: bool,
    /// True when `|dc_offset| > 0.01` — patch is leaking DC.
    pub has_dc_offset: bool,
    /// True when `peak_amplitude < 0.05` — quieter than typical bus level,
    /// likely needs a gain boost.
    pub low_output: bool,
    /// True when the detector locked onto a pitch more than half a semitone
    /// from the expected note *with confidence*. Mutually exclusive with
    /// `pitch_unreliable`: this one fires only when the reported
    /// fundamental is trustworthy enough to act on.
    pub off_pitch: bool,
    /// True when the pitch detector couldn't pick a clear fundamental —
    /// formant-heavy, sub-bass-dominant, or atonal/noisy content.
    /// `fundamental_hz` should not be trusted; see `off_pitch` for the
    /// "locked on the wrong note" case.
    pub pitch_unreliable: bool,
}

/// Which signal `fundamental_hz` (and the spectral metrics derived from
/// the same buffer) was computed on. Lets the caller distinguish the
/// "true mono" case from the synthesized phase-robust signal used for
/// stereo input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnalysisSignalMode {
    /// Mono input — analysis ran on the single channel.
    Mono,
    /// Stereo input — analysis ran on a synthetic max(|L|,|R|) signal
    /// to survive anti-phase content. The synthetic signal preserves
    /// each frame's sign from the channel with the larger magnitude.
    MaxAbsStereo,
}

/// Compact go/no-go verdict on whether an instrument actually produces audio.
/// Returned by `validate_instrument_audio` — a thin distillation of the full
/// `analyze_note` render for the common "is this patch alive?" check.
#[derive(Debug, Clone, Serialize)]
pub struct ValidateInstrumentAudioResult {
    /// Whether the rendered note produced audible signal (peak above the
    /// silence floor, ~-80 dBFS).
    pub is_audible: bool,
    /// One-line human-readable verdict.
    pub verdict: String,
    /// Peak absolute amplitude across the render (1.0 = full scale).
    pub peak_amplitude: f32,
    /// Peak in dBFS (`-inf` rendered as a large negative number for silence).
    pub peak_dbfs: f32,
    /// Overall RMS amplitude.
    pub rms_overall: f32,
    /// Whether any samples reached the f32 fullscale ceiling.
    pub clipped: bool,
    /// Count of clipped samples (>= 0.999).
    pub clipped_samples: u32,
    /// Detected fundamental in Hz (0.0 if silent/atonal).
    pub fundamental_hz: f32,
    /// Detected fundamental vs concert pitch, in cents (positive = sharp).
    pub pitch_error_cents: f32,
    /// Mean sample value (DC offset); non-zero hints at a missing DC blocker.
    pub dc_offset: f32,
    /// MIDI note rendered (after the patch's octave offset).
    pub note_played: u8,
    /// Non-fatal advisories (very quiet, clipping, DC offset, pitch off, …).
    pub warnings: Vec<String>,
}

/// Quantitative analysis of a rendered note. Returned by `analyze_note`.
///
/// All metrics are computed offline on the f32 audio buffer with no WAV
/// roundtrip. Designed so an LLM agent can reason about a patch's behavior
/// without downloading or decoding audio. Includes per-window pitch tracking,
/// stereo correlation, banded energy, harmonic structure, an ADSR-like
/// envelope estimate, centroid trend, and quick boolean flags.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeNoteResult {
    /// MIDI note as requested by the caller.
    pub note_requested: u8,
    /// MIDI note actually played, after the patch's `octave_offset` was
    /// applied. Differs from `note_requested` when the patch sets a non-zero
    /// offset (e.g. bass patches with offset = -2).
    pub note_played: u8,
    /// Velocity used (0-127).
    pub velocity: u8,
    /// Sample rate of the rendered buffer in Hz.
    pub sample_rate: u32,
    /// Total render duration in seconds (note + tail).
    pub duration_seconds: f32,
    /// Detected fundamental frequency in Hz from the steady-state region of
    /// the note. Computed on the buffer described by `analysis_signal_mode`
    /// — for stereo input that's the synthetic max(|L|,|R|) signal, not the
    /// L+R mono mix, so anti-phase tonal content survives. Returns 0.0 for
    /// silent or out-of-range signals.
    pub fundamental_hz: f32,
    /// Which signal `fundamental_hz` and the spectral metrics
    /// (`spectrum_*`, `energy_bands`, `harmonic_content`, `pitch_envelope`,
    /// `centroid_envelope`) were derived from. For stereo input that's a
    /// synthetic max(|L|,|R|) buffer; for mono input it's the single
    /// channel directly.
    pub analysis_signal_mode: AnalysisSignalMode,
    /// Pitch detection on the left channel only. `None` for mono input.
    /// Use together with `fundamental_right` to spot wide-stereo patches
    /// where L and R carry different fundamentals and the pooled
    /// `fundamental_hz` is misleading.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fundamental_left: Option<f32>,
    /// Pitch detection on the right channel only. `None` for mono input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fundamental_right: Option<f32>,
    /// Confidence in `fundamental_left` in `0.0..=1.0`. Same prominence-based
    /// metric as `pitch_confidence`. `None` for mono input; `0.0` means no
    /// reliable detectable peak. Low values mean the channel is noisy/atonal
    /// even when `fundamental_left` reports a non-zero number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fundamental_left_confidence: Option<f32>,
    /// Confidence in `fundamental_right` in `0.0..=1.0`. See
    /// `fundamental_left_confidence`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fundamental_right_confidence: Option<f32>,
    /// Concert-pitch frequency (Hz) of `note_played`. Use as a reference for
    /// `fundamental_hz` to spot detuning, octave errors, and pitch drift.
    pub expected_fundamental_hz: f32,
    /// Detected fundamental in cents relative to `expected_fundamental_hz`
    /// (positive = sharp, negative = flat). 0.0 if either pitch is 0.
    pub pitch_error_cents: f32,
    /// Peak absolute amplitude across the whole render (1.0 = full scale).
    pub peak_amplitude: f32,
    /// Overall RMS amplitude of the whole render.
    pub rms_overall: f32,
    /// Mean of all samples (DC offset). Should be near zero in well-designed
    /// patches; non-zero hints at a missing DC blocker.
    pub dc_offset: f32,
    /// Count of samples whose absolute value reached or exceeded 0.999 —
    /// non-zero indicates clipping into the f32 fullscale ceiling.
    pub clipped_samples: u32,
    /// Window length in milliseconds used for `rms_envelope` and
    /// `centroid_envelope`.
    pub envelope_window_ms: f32,
    /// RMS amplitude per non-overlapping window of `envelope_window_ms`.
    pub rms_envelope: Vec<f32>,
    /// Spectral centroid in Hz per non-overlapping window of
    /// `envelope_window_ms`. A proxy for filter cutoff motion.
    pub centroid_envelope: Vec<f32>,
    /// Top spectrum peaks (up to 8) at the early/attack region (~50 ms in).
    pub spectrum_attack: Vec<AnalyzeSpectrumPeak>,
    /// Top spectrum peaks (up to 8) at the sustained mid-region.
    pub spectrum_sustain: Vec<AnalyzeSpectrumPeak>,
    /// Top spectrum peaks (up to 8) during the release tail. Empty when the
    /// release window has fully decayed below the spectral floor — that's
    /// not an error, just "the patch is silent by then".
    pub spectrum_release: Vec<AnalyzeSpectrumPeak>,
    /// Per-window fundamental frequency in Hz. Uses a longer window than
    /// `rms_envelope` / `centroid_envelope` (see `pitch_envelope_window_ms`)
    /// because FFT bin resolution scales with window length, and 50 ms is
    /// too coarse to track bass fundamentals stably. 0.0 for silent windows.
    /// Use this to detect pitch ramping or drift.
    pub pitch_envelope: Vec<f32>,
    /// Window length in milliseconds used for `pitch_envelope`. Differs from
    /// `envelope_window_ms` because pitch detection needs more samples per
    /// window than amplitude tracking does — typically 200 ms.
    pub pitch_envelope_window_ms: f32,
    /// Pearson correlation between L and R channels (-1..+1). 1.0 means
    /// the patch is mono routed to both outs; lower values indicate true
    /// stereo content.
    pub stereo_correlation: f32,
    /// RMS energy split into sub / low / mid / high bands.
    pub energy_bands: AnalyzeEnergyBands,
    /// Harmonic structure measured at the detected fundamental. Returns
    /// zeros when `fundamental_hz` is 0.
    pub harmonic_content: AnalyzeHarmonicContent,
    /// Estimated ADSR-like envelope shape derived from `rms_envelope`.
    pub envelope_estimate: AnalyzeEnvelopeEstimate,
    /// Linear-regression slope of `centroid_envelope` over the held-note
    /// region, in Hz/second. Positive = filter opening over the note,
    /// negative = closing.
    pub centroid_trend_hz_per_sec: f32,
    /// Quick boolean flags an agent can branch on without reading the
    /// raw numeric fields.
    pub flags: AnalyzeFlags,
    /// Per-channel peak amplitudes (left, right). Present when the render
    /// was stereo. Useful for catching anti-phase content that cancels in
    /// the mono mix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_right: Option<f32>,
    /// Per-channel RMS amplitudes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rms_left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rms_right: Option<f32>,
    /// Per-channel DC offsets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dc_left: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dc_right: Option<f32>,
    /// Per-channel clipped-sample counts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipped_left: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipped_right: Option<u32>,
    /// Mid (sum/2) channel RMS. High values mean energy is centered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mid_rms: Option<f32>,
    /// Side ((L−R)/2) channel RMS. High values mean energy is wide / decorrelated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side_rms: Option<f32>,
    /// Stereo width estimate in 0..1: `side_rms / (mid_rms + side_rms)`.
    /// 0 = mono (all energy in mid), ~0.5 = typical stereo, 1 = anti-phase
    /// or fully decorrelated (all energy in side). Returns 0 when both
    /// mid and side RMS are below ~1e-9 (silent). Complementary to
    /// `stereo_correlation`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo_width: Option<f32>,
    /// Confidence in the detected fundamental, in 0.0..1.0. Computed from
    /// the prominence of the loudest in-range bin over the next-loudest
    /// non-adjacent peak. Low values mean the spectrum is noisy or has
    /// multiple competing peaks; treat `pitch_error_cents` with skepticism
    /// when this is below ~0.3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch_confidence: Option<f32>,
    /// Number of trailing rms_envelope/centroid_envelope windows that were
    /// trimmed from `centroid_envelope` because they decayed to noise. The
    /// raw `rms_envelope` is no longer trimmed; this counter tells you how
    /// much of the tail was suppressed in the centroid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trimmed_tail_windows: Option<u32>,
    /// Time offset (ms) at which the attack-spectrum window starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attack_window_start_ms: Option<f32>,
    /// Time offset (ms) at which the sustain-spectrum window starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sustain_window_start_ms: Option<f32>,
    /// Time offset (ms) at which the release-spectrum window starts.
    /// Anchored relative to the absolute render start (i.e., 0 = first
    /// sample), so values larger than `duration_ms` are post-note-off.
    /// Never slips backward past the note-off boundary to fit a full
    /// window when the tail is short — the slice is allowed to be
    /// shorter than 100 ms instead. Tail too short for the 25 ms offset
    /// pegs this at the end of the render.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_window_start_ms: Option<f32>,
    /// Non-fatal warnings collected during the offline render — module
    /// instantiation failures, skipped modules, missing connections, etc.
    /// An empty vector is omitted from the JSON.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

// === AWE (Acoustic World Engine) types ===

/// Full AWE state returned by `get_awe_state`.
#[derive(Debug, Clone, Serialize)]
pub struct AweStateInfo {
    /// Whether AWE is enabled.
    pub enabled: bool,
    /// Free-text description of the acoustic character. Mirrors
    /// `AwePresetFile.description` when an AWE preset is loaded; can
    /// also be set directly via `set_awe_description`. Skipped from
    /// JSON when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
    /// Free-text description / intent — `""` when not set. Skipped from JSON
    /// when empty. Set via `set_sample_description`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
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

// ---------------------------------------------------------------------------
// analyze_harmony result types
// ---------------------------------------------------------------------------

/// What was analyzed by `analyze_harmony` — either a single pattern or an
/// arrangement range.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HarmonyScope {
    /// A pattern, identified by its `PatternId` value.
    Pattern { pattern_id: u32 },
    /// An arrangement range in absolute ticks (`[start_tick, end_tick)`).
    Arrangement { start_tick: u64, end_tick: u64 },
}

/// One identified chord event inside `AnalyzeHarmonyResult.chords`.
///
/// The window `[start_tick, end_tick)` is the grouping window the analyzer
/// used; chord identification is based on every pitch that sounded inside it.
#[derive(Debug, Clone, Serialize)]
pub struct HarmonyChordEvent {
    /// 1-indexed bar number at the event start, using the song's time
    /// signature at `start_tick`. For pattern scope, computed against the
    /// song's default time signature; bars are 1-indexed within the pattern.
    pub start_bar: u32,
    /// 1-indexed beat within `start_bar`.
    pub start_beat: u32,
    /// Window start in absolute ticks (or pattern-relative ticks for pattern
    /// scope).
    pub start_tick: u64,
    /// Window end (exclusive).
    pub end_tick: u64,
    /// Unique MIDI notes (0-127) that sounded at any point in the window,
    /// sorted ascending. Empty windows are omitted.
    pub midi_notes: Vec<u8>,
    /// Chord symbol e.g. `"Cm7"`, `"F#maj7"`, `"Bbsus4"`. `None` when the
    /// pitch set didn't match any known chord template (e.g. a single note
    /// or a dyad that isn't a power chord).
    pub symbol: Option<String>,
    /// Root pitch class (0..12, C = 0). `None` when `symbol` is `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<u8>,
    /// Quality name e.g. `"minor7"`, `"sus4"`. `None` when `symbol` is `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// True when every note in the window belongs to the inferred key's scale.
    pub in_key: bool,
}

/// Inferred key entry. `analyze_harmony` returns one top match plus up to two
/// runner-ups.
#[derive(Debug, Clone, Serialize)]
pub struct HarmonyKeyEstimate {
    /// Tonic pitch class (0..12, C = 0).
    pub tonic: u8,
    /// Tonic name e.g. `"C"`, `"F#"`.
    pub tonic_name: String,
    /// `"major"` or `"minor"`.
    pub mode: String,
    /// Combined human-readable label e.g. `"C major"`, `"F# minor"`.
    pub label: String,
    /// Krumhansl-Schmuckler Pearson correlation in `-1.0..=1.0`. Higher = better fit.
    pub correlation: f32,
}

/// Aggregate counters for `analyze_harmony`.
#[derive(Debug, Clone, Serialize)]
pub struct HarmonyStats {
    /// Total notes considered.
    pub total_notes: u32,
    /// Number of non-empty grouping windows.
    pub chord_event_count: u32,
    /// Distinct chord symbols across all events.
    pub distinct_chord_count: u32,
    /// How many events were identified to a chord symbol.
    pub identified_chord_count: u32,
    /// Lowest MIDI note encountered, or 0 if no notes.
    pub pitch_range_low: u8,
    /// Highest MIDI note encountered, or 0 if no notes.
    pub pitch_range_high: u8,
    /// Mean polyphony across non-empty windows (1.0 = monophonic).
    pub avg_polyphony: f32,
    /// Grouping resolution the analyzer used (ticks per window).
    pub grouping_ticks: u32,
}

/// Quantitative harmonic analysis of a pattern or arrangement range. Returned
/// by `analyze_harmony`.
///
/// Pure symbolic — no audio rendering involved. Walks notes in time order,
/// groups overlapping notes into chord events on a configurable resolution,
/// labels each event with a chord symbol when one matches, and infers the
/// most likely key using Krumhansl-Schmuckler key-profile correlation.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeHarmonyResult {
    /// What was analyzed (pattern id or arrangement range).
    pub scope: HarmonyScope,
    /// Sequence of chord events in time order.
    pub chords: Vec<HarmonyChordEvent>,
    /// Most likely key, or `None` for empty input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inferred_key: Option<HarmonyKeyEstimate>,
    /// Top alternative keys (up to 2) after the inferred one.
    pub key_candidates: Vec<HarmonyKeyEstimate>,
    /// Pitch-class histogram, weighted by note duration (in ticks). Index 0 = C.
    pub pitch_class_histogram: [f32; 12],
    /// Fraction of note-weight (by duration) inside the inferred key's scale
    /// (0.0..=1.0). 0.0 if `inferred_key` is `None`.
    pub in_key_ratio: f32,
    /// Pitch classes (0..12) with non-zero weight that fall outside the inferred
    /// key's scale. Empty if `inferred_key` is `None`.
    pub out_of_scale_pitch_classes: Vec<u8>,
    /// Composite 0.0..=1.0 score combining in-key ratio, chord-identification
    /// ratio, and key-correlation strength. Higher = more harmonically settled.
    pub harmonic_stability_score: f32,
    /// Counters and summary stats.
    pub stats: HarmonyStats,
    /// Warnings encountered during analysis (e.g. empty patterns, range with
    /// no notes). Empty when analysis ran cleanly.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// analyze_mix_bus / analyze_section result types
// ---------------------------------------------------------------------------

/// Mix-bus metrics common to `analyze_mix_bus` and `analyze_section`, also
/// embedded inside `TrackContribution` for the soloed per-track variant.
///
/// All `*_dbfs` / `*_dbtp` fields use `-200.0` as a substitute for `-inf` so
/// JSON consumers don't have to special-case non-finite values. All `lufs_*`
/// fields follow the same convention. `lufs_short_term_max` is `-200.0` for
/// buffers shorter than 3 s (the short-term window length).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct MixBusMetrics {
    /// Sample rate the buffer was rendered at.
    pub sample_rate: u32,
    /// Total render duration in seconds.
    pub duration_seconds: f32,
    /// Sample peak across both channels, linear.
    pub peak: f32,
    /// Sample peak in dBFS.
    pub peak_dbfs: f32,
    /// Sample peak of the left channel only, linear.
    pub peak_left: f32,
    /// Sample peak of the right channel only, linear.
    pub peak_right: f32,
    /// Inter-sample (true) peak across both channels, linear — computed via
    /// 4× polyphase oversampling per ITU-R BS.1770-4 Annex 2. Always ≥ `peak`;
    /// catches overshoots that emerge only after DA conversion or lossy
    /// encoding.
    pub true_peak: f32,
    /// True peak in dBTP (dB true peak).
    pub true_peak_dbtp: f32,
    /// Overall RMS, linear.
    pub rms: f32,
    /// Overall RMS in dBFS.
    pub rms_dbfs: f32,
    /// Crest factor (peak_dBFS - rms_dBFS). Higher = more dynamic.
    pub crest_factor_db: f32,
    /// Integrated loudness (ITU-R BS.1770-4 LUFS-I).
    pub lufs_integrated: f32,
    /// Maximum momentary loudness (LUFS-M) — peak over single 400 ms
    /// K-weighted blocks across the buffer.
    pub lufs_momentary_max: f32,
    /// Maximum short-term loudness (LUFS-S) — peak over 3 s K-weighted sliding
    /// windows stepped every 100 ms. `-200.0` for buffers shorter than 3 s.
    pub lufs_short_term_max: f32,
    /// 4-band RMS energy on the mono mix-down (sub/low/mid/high).
    pub energy_bands: AnalyzeEnergyBands,
    /// Pearson correlation between L and R channels, [-1.0, 1.0].
    pub stereo_correlation: f32,
    /// RMS of the (L+R)/2 component.
    pub mid_rms: f32,
    /// RMS of the (L-R)/2 component.
    pub side_rms: f32,
    /// Stereo width = side_rms / mid_rms (0.0 = mono).
    pub stereo_width: f32,
    /// Mono-compatibility score, 0.0..=1.0. 1.0 = perfectly mono-summable,
    /// 0.0 = full anti-phase cancellation.
    pub mono_compat: f32,
    /// Count of samples that hit the ±0.999 ceiling.
    pub clipped_samples: u32,
}

/// Output of `analyze_mix_bus`. Renders `duration_seconds` of the master bus
/// starting from `start_tick` (defaults to 0) and returns mix-level metrics.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeMixBusResult {
    /// 1-indexed bar number at `start_tick`, using the song's time signature
    /// at that tick.
    pub start_bar: u32,
    /// 1-indexed beat within `start_bar`.
    pub start_beat: u32,
    /// 1-indexed bar number at `end_tick`.
    pub end_bar: u32,
    /// 1-indexed beat within `end_bar`.
    pub end_beat: u32,
    /// Tick where the render started.
    pub start_tick: u64,
    /// Tick where the render ended (exclusive).
    pub end_tick: u64,
    /// Mix-bus metrics from the rendered window.
    pub metrics: MixBusMetrics,
    /// Per-track contribution breakdown, one entry per audible track whose
    /// placements overlap the rendered window. Empty when the caller did not
    /// request a breakdown. Each entry comes from a separate offline render
    /// with only that track soloed; the cost is therefore O(N) in the number
    /// of tracks covering the window. Same semantics as
    /// `AnalyzeSectionResult.per_track`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub per_track: Vec<TrackContribution>,
    /// Human-readable description of exactly what signal chain was measured —
    /// the master fader value plus which optional stages (master/return effects,
    /// AWE) were included and the render sample rate. Removes ambiguity about
    /// whether the metrics are pre- or post-master.
    pub signal_chain: String,
    /// Non-fatal warnings emitted during the render.
    pub warnings: Vec<String>,
}

/// Result of `auto_gain_stage`: the master fader was adjusted to bring the mix
/// toward a target loudness without breaching a true-peak ceiling.
#[derive(Debug, Clone, Serialize)]
pub struct AutoGainStageResult {
    /// Integrated LUFS measured at the current master setting (post master +
    /// return effects).
    pub measured_lufs: f32,
    /// True peak (dBTP) measured at the current master setting.
    pub measured_true_peak_dbtp: f32,
    /// Requested target integrated loudness (LUFS).
    pub target_lufs: f32,
    /// True-peak ceiling that was respected (dBTP).
    pub true_peak_ceiling_dbtp: f32,
    /// Gain applied to the master fader, in dB.
    pub applied_gain_db: f32,
    /// Master fader value before the adjustment (linear gain).
    pub previous_master_volume: f32,
    /// Master fader value after the adjustment (linear gain).
    pub new_master_volume: f32,
    /// Predicted integrated LUFS after the adjustment (linear, since the master
    /// fader is post-effects).
    pub predicted_lufs: f32,
    /// Predicted true peak (dBTP) after the adjustment.
    pub predicted_true_peak_dbtp: f32,
    /// Which constraint bound the result: `"target_lufs"` (hit the target),
    /// `"true_peak_ceiling"` (target would clip, peak-limited instead), or
    /// `"master_volume_range"` (clamped to the 0..2 fader range).
    pub limited_by: String,
    /// Non-fatal warnings emitted during the measurement render.
    pub warnings: Vec<String>,
}

/// Per-track contribution to a section's master mix. Returned in
/// `AnalyzeSectionResult.per_track` when the caller asks for a breakdown.
///
/// Each entry is the result of re-rendering the same `[start_tick, end_tick)`
/// range with one track soloed, so it shows what that track sounds like in
/// isolation (with its own track gain / pan / effects). Use this to answer
/// "which track is responsible for the chorus clipping?" or "which track owns
/// the sub-bass energy?".
///
/// `metrics.peak` / `metrics.peak_dbfs` / `metrics.rms` / `metrics.rms_dbfs`
/// reflect the track's contribution to the master mix, **including pan-law
/// attenuation** (constant-power pan: -3 dB on each channel for a center-panned
/// track). A center-panned mono source with internal peak 1.0 therefore reports
/// `metrics.peak ≈ 0.7071`.
///
/// `pre_master_peak` is the same instrument signal measured *before* track
/// volume and pan-law are applied — i.e. what the patch itself is producing.
/// Lets you spot a patch that clips internally even when the resulting master
/// contribution looks safe. Equivalent to running `analyze_note` against the
/// instrument and taking the max output peak across the section's notes.
#[derive(Debug, Clone, Serialize)]
pub struct TrackContribution {
    /// Sequencer track ID.
    pub track_id: u16,
    /// Track name.
    pub track_name: String,
    /// Assigned instrument's seq ID (matches `InstrumentInfo.id` value).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument_id: Option<u16>,
    /// Mix-bus metrics of the soloed render (with the track's own volume and
    /// pan applied — includes pan-law attenuation; see struct docs).
    pub metrics: MixBusMetrics,
    /// Internal-signal peak before track volume and pan-law are applied,
    /// linear. Equivalent to "what the patch is outputting on its own".
    pub pre_master_peak: f32,
    /// `pre_master_peak` in dBFS. Silence reported as -200.0.
    pub pre_master_peak_dbfs: f32,
    /// Fraction of the summed-track RMS that this track contributes,
    /// `0.0..=1.0`. Sums to ~1.0 across all returned tracks; quick way to spot
    /// dominant elements without comparing absolute RMS values.
    pub rms_share: f32,
}

/// Output of `analyze_section`. Same metrics as `analyze_mix_bus` but the
/// tick range is explicit. Optionally includes a per-track breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeSectionResult {
    /// 1-indexed bar number at `start_tick`.
    pub start_bar: u32,
    /// 1-indexed beat within `start_bar`.
    pub start_beat: u32,
    /// 1-indexed bar number at `end_tick`.
    pub end_bar: u32,
    /// 1-indexed beat within `end_bar`.
    pub end_beat: u32,
    /// Tick range that was analyzed (inclusive start, exclusive end).
    pub start_tick: u64,
    pub end_tick: u64,
    /// Mix-bus metrics for the section.
    pub metrics: MixBusMetrics,
    /// Per-track contribution breakdown, one entry per audible track whose
    /// placements overlap the section. Empty when the caller did not request
    /// a breakdown. Each entry comes from a separate offline render with only
    /// that track soloed; the cost is therefore O(N) in the number of tracks
    /// covering the section.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub per_track: Vec<TrackContribution>,
    /// Human-readable description of exactly what signal chain was measured
    /// (master fader + included optional stages + render sample rate). See
    /// `AnalyzeMixBusResult.signal_chain`.
    pub signal_chain: String,
    /// Non-fatal warnings emitted during the render.
    pub warnings: Vec<String>,
}

/// One stage of the master effect chain, measured at that effect's output.
/// Returned in `AnalyzeMasterChainResult.stages` in chain order.
///
/// `metrics` is the master bus measured with the chain truncated *after* this
/// effect. The `*_delta` fields are the change this effect introduced versus
/// the previous stage's output (the chain input for the first effect), so a
/// limiter shows a negative `peak_delta_db`, an EQ boost a positive
/// `rms_delta_db`, and a widener a positive `stereo_width_delta`.
#[derive(Debug, Clone, Serialize)]
pub struct MasterEffectStage {
    /// Effect module id (type prefix + instance, e.g. `lim:1`).
    pub module_id: String,
    /// Effect type prefix (e.g. `lim`, `eq`, `comp`).
    pub effect_type: String,
    /// Whether the effect was bypassed (disabled) in the live chain. A bypassed
    /// effect passes signal through unchanged, so its deltas are ~0.
    pub bypassed: bool,
    /// Master-bus metrics measured at this effect's output.
    pub metrics: MixBusMetrics,
    /// Integrated-LUFS change this effect introduced (post − pre).
    pub lufs_delta: f32,
    /// Sample-peak change in dB (post − pre). Negative = the effect lowered the peak.
    pub peak_delta_db: f32,
    /// True-peak change in dBTP (post − pre).
    pub true_peak_delta_db: f32,
    /// RMS change in dB (post − pre).
    pub rms_delta_db: f32,
    /// Stereo-width change (post − pre); positive = wider.
    pub stereo_width_delta: f32,
    /// Crest-factor change in dB (post − pre); negative = more compressed dynamics.
    pub crest_delta_db: f32,
    /// Net RMS attenuation in dB (pre − post): positive when the effect reduced
    /// level (dynamics/limiting), negative when it added gain. Convenience
    /// inverse of `rms_delta_db`.
    pub gain_reduction_db: f32,
}

/// Output of `analyze_master_chain`: an incremental, per-effect breakdown of
/// the master bus. The chain input (post-return mix, before any master effect)
/// is measured once, then the chain is rendered truncated after each effect so
/// every stage's contribution can be isolated. Costs one offline render per
/// master effect plus one for the input — O(effect_count).
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeMasterChainResult {
    /// 1-indexed bar number at `start_tick`.
    pub start_bar: u32,
    /// 1-indexed beat within `start_bar`.
    pub start_beat: u32,
    /// 1-indexed bar number at `end_tick`.
    pub end_bar: u32,
    /// 1-indexed beat within `end_bar`.
    pub end_beat: u32,
    /// Tick where the render started.
    pub start_tick: u64,
    /// Tick where the render ended (exclusive).
    pub end_tick: u64,
    /// Master-chain input metrics: the post-return mix before any master effect.
    pub input_metrics: MixBusMetrics,
    /// Final master output metrics (full chain). Equals `input_metrics` when the
    /// chain is empty.
    pub output_metrics: MixBusMetrics,
    /// One entry per master effect, in chain order. Empty when the master chain
    /// has no effects.
    pub stages: Vec<MasterEffectStage>,
    /// Human-readable description of exactly what signal chain was measured.
    /// See `AnalyzeMixBusResult.signal_chain`.
    pub signal_chain: String,
    /// Non-fatal warnings emitted during the renders.
    pub warnings: Vec<String>,
}

/// One return bus's contribution to the master mix, measured by A/B: the master
/// is rendered with every return active, then again with this one return muted.
/// The `*_delta` fields are `full − muted`, so a positive value means the
/// return makes the mix louder / wider / peakier.
///
/// A return's wet signal cannot be cleanly soloed away from the dry track sum
/// (master = dry track outputs + return wet outputs, and muting all tracks
/// would also starve the sends feeding the return), so this muted-difference is
/// the honest measure of what the return adds.
///
/// Each delta is the bus's *marginal* contribution: the change from removing it
/// entirely. Without bus-to-bus sends the returns sum in parallel and the
/// deltas are independent. With bus-to-bus sends they are NOT — muting a return
/// that feeds another also removes the signal it routed downstream, so its
/// delta absorbs that downstream contribution and the per-return deltas no
/// longer sum to the full mix (the result carries a warning when this applies).
#[derive(Debug, Clone, Serialize)]
pub struct ReturnBusContribution {
    /// Return bus id.
    pub return_id: u16,
    /// Return bus name.
    pub return_name: String,
    /// Integrated-LUFS the return adds to the master (full − muted).
    pub lufs_delta: f32,
    /// Sample-peak change in dB the return adds (full − muted).
    pub peak_delta_db: f32,
    /// True-peak change in dBTP the return adds (full − muted).
    pub true_peak_delta_db: f32,
    /// RMS change in dB the return adds (full − muted).
    pub rms_delta_db: f32,
    /// Stereo-width change the return adds (full − muted); positive = wider.
    pub stereo_width_delta: f32,
}

/// Output of `analyze_return_busses`: the full master mix plus each return
/// bus's marginal contribution, measured by muting one return at a time. Costs
/// one offline render for the full mix plus one per return bus —
/// O(return_count).
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeReturnBussesResult {
    /// 1-indexed bar number at `start_tick`.
    pub start_bar: u32,
    /// 1-indexed beat within `start_bar`.
    pub start_beat: u32,
    /// 1-indexed bar number at `end_tick`.
    pub end_bar: u32,
    /// 1-indexed beat within `end_bar`.
    pub end_beat: u32,
    /// Tick where the render started.
    pub start_tick: u64,
    /// Tick where the render ended (exclusive).
    pub end_tick: u64,
    /// Full master mix metrics with every return bus active.
    pub full_metrics: MixBusMetrics,
    /// One entry per return bus, in declared order.
    pub returns: Vec<ReturnBusContribution>,
    /// Human-readable description of exactly what signal chain was measured.
    /// See `AnalyzeMixBusResult.signal_chain`.
    pub signal_chain: String,
    /// Non-fatal warnings emitted during the renders.
    pub warnings: Vec<String>,
}

/// Per-band overlap report between two tracks. One entry per
/// `AnalyzeEnergyBands` band; values are linear RMS amplitudes pulled from
/// each track's soloed render.
///
/// `dominance_db = 20·log10(max(a,b) / min(a,b))` — magnitude of the louder
/// track's lead in this band, regardless of direction. Combine with
/// `track_a_energy` vs. `track_b_energy` to know *which* track is louder.
/// Reported as `200.0` when one side is silent (avoids `+inf` in JSON).
#[derive(Debug, Clone, Serialize)]
pub struct BandOverlap {
    /// `"sub"` | `"low"` | `"mid"` | `"high"`.
    pub band: String,
    /// Band lower frequency edge in Hz (inclusive).
    pub freq_low_hz: f32,
    /// Band upper frequency edge in Hz (exclusive, or Nyquist for `"high"`).
    pub freq_high_hz: f32,
    /// Linear RMS of track A in this band.
    pub track_a_energy: f32,
    /// Linear RMS of track B in this band.
    pub track_b_energy: f32,
    /// `min(track_a_energy, track_b_energy)` — the energy two tracks compete
    /// for in this band. High overlap is a masking candidate.
    pub overlap_energy: f32,
    /// `20·log10(max / min)` — how many dB louder the dominant track is.
    /// Clamped to `200.0` for the silent-vs-non-silent case.
    pub dominance_db: f32,
}

/// One pair of tracks with their per-band overlap and an optional textual
/// hint. Pairs are unordered (`track_a_id < track_b_id`); `dominant_track_id`
/// flags which side wins overall on the band that has the highest overlap.
#[derive(Debug, Clone, Serialize)]
pub struct MaskingPair {
    pub track_a_id: u16,
    pub track_a_name: String,
    pub track_b_id: u16,
    pub track_b_name: String,
    /// One entry per band (sub / low / mid / high), in that order.
    pub bands: Vec<BandOverlap>,
    /// `sum(overlap_energy) / sum(max(a,b))` across bands, `0.0..=1.0`.
    /// 0 = tracks share no audible energy; 1 = identical spectral envelope.
    /// Pairs are returned sorted by descending `conflict_score`.
    pub conflict_score: f32,
    /// The id of the track that dominates the highest-overlap band when the
    /// dominance margin exceeds 6 dB. `None` means the pair is in even
    /// competition (or both sides are essentially silent in shared bands).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dominant_track_id: Option<u16>,
    /// Free-form hint summarizing the worst band, e.g. `"Pad(2) masks
    /// Lead(3) in mid (500-2000 Hz)"`. `None` when the highest-overlap band
    /// is below the audibility threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Output of `analyze_masking_matrix`. Pairwise spectral overlap report
/// across every audible track in the section. Reuses the same soloed per-
/// track renders as `analyze_section` with `include_per_track = true`, so the
/// audio cost is one offline render per audible track; the pair matrix
/// itself is computed in-memory from the per-track band energies.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeMaskingMatrixResult {
    pub start_bar: u32,
    pub start_beat: u32,
    pub end_bar: u32,
    pub end_beat: u32,
    pub start_tick: u64,
    pub end_tick: u64,
    /// Number of audible tracks that overlapped the section. The pair
    /// count is `track_count·(track_count − 1) / 2`.
    pub track_count: u32,
    /// Total unordered pair count before any `top_pairs` truncation.
    pub total_pair_count: u32,
    /// One entry per unordered pair, sorted by descending `conflict_score`,
    /// truncated to the caller's `top_pairs` (default 20).
    pub pairs: Vec<MaskingPair>,
    /// Non-fatal warnings emitted during the render.
    pub warnings: Vec<String>,
}

/// Auto-inferred profile for one instrument. Mirrors the internal
/// `analysis::InstrumentProfile`; enums travel as snake_case strings so the
/// MCP crate stays free of pertylizer-side types.
#[derive(Debug, Clone, Serialize)]
pub struct InstrumentProfileResult {
    /// Sequencer instrument id (matches `SeqInstrumentId.0`).
    pub instrument_id: u16,
    pub instrument_name: String,
    /// Inferred role plus confidence and the signal trail that produced it.
    pub role: RoleInferenceResult,
    /// `"percussive" | "plucked" | "sustained" | "evolving" | "unknown"`.
    pub envelope_shape: String,
    /// `"tonal" | "atonal" | "mixed" | "unused"`.
    pub pitch_role: String,
    /// `"sub" | "bass" | "mid" | "high" | "full_range" | "unused"`.
    pub register: String,
    /// `"monophonic" | "polyphonic" | "chordal" | "unused"`.
    pub texture: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleInferenceResult {
    /// `"drums" | "bass" | "lead" | "pad" | "pluck" | "keys" | "fx" | "unknown"`.
    pub role: String,
    /// `0.0..=1.0`. The `analyze_harmony` drum filter triggers at `>= 0.6`.
    pub confidence: f32,
    /// Trail of signals (name, graph, envelope, pattern, manual, decision)
    /// that contributed to the classification.
    pub signals: Vec<ProfileSignalResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileSignalResult {
    /// `"name" | "graph" | "envelope" | "pattern" | "manual" | "decision"`.
    pub axis: String,
    /// Concrete detail, e.g. `"kick"`, `"noise-no-osc"`, `"oneshot-sampler"`,
    /// `"percussive"`, `"manual-override"`.
    pub detail: String,
}

// ---------------------------------------------------------------------------
// analyze_pattern result types
// ---------------------------------------------------------------------------

/// Density-related metrics for a pattern.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PatternDensity {
    /// Average notes per bar over the pattern's length, using the song's
    /// default time signature.
    pub notes_per_bar: f32,
    /// Average notes per beat.
    pub notes_per_beat: f32,
    /// Fraction of the pattern's tick span covered by at least one sounding
    /// note (0.0..=1.0). 1.0 = something always playing; 0.0 = silence.
    pub active_ratio: f32,
}

/// Pitch-shape metrics for a pattern.
#[derive(Debug, Clone, Serialize)]
pub struct PatternPitch {
    /// Lowest MIDI note encountered (0 when the pattern has no notes).
    pub low: u8,
    /// Highest MIDI note encountered (0 when the pattern has no notes).
    pub high: u8,
    /// `high - low`, in semitones.
    pub range_semitones: u8,
    /// Duration-weighted mean MIDI pitch.
    pub mean: f32,
    /// Number of distinct MIDI pitches in the pattern.
    pub distinct_count: u32,
    /// Duration-weighted pitch-class histogram, normalized so the values sum
    /// to 1.0 when the pattern has notes (else all zero). Index 0 = C.
    pub class_histogram: [f32; 12],
}

/// Velocity-dynamics metrics for a pattern.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PatternVelocity {
    /// Lowest velocity (0..=1.0).
    pub min: f32,
    /// Highest velocity.
    pub max: f32,
    /// Arithmetic mean.
    pub mean: f32,
    /// Population standard deviation (0 when fewer than 2 notes).
    pub std_dev: f32,
    /// `max - min`.
    pub range: f32,
}

/// Rhythmic-structure metrics for a pattern.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PatternRhythm {
    /// Maximum simultaneous voices observed at any tick.
    pub max_polyphony: u32,
    /// Mean simultaneous-voice count, weighted by the time at least one note
    /// was sounding. `1.0` for strictly monophonic patterns.
    pub mean_polyphony: f32,
    /// True when no two notes overlap in time.
    pub is_monophonic: bool,
    /// Number of distinct onset tick positions.
    pub distinct_onset_count: u32,
    /// Number of distinct note durations.
    pub distinct_duration_count: u32,
    /// Mean inter-onset interval across distinct onsets (ticks).
    pub mean_ioi_ticks: f32,
    /// Standard deviation of inter-onset intervals (ticks). Zero on a
    /// perfectly regular grid.
    pub ioi_std_ticks: f32,
    /// `1.0 - clamp(ioi_std / ioi_mean, 0, 1)`. `1.0` = perfectly regular
    /// grid; `0.0` for fewer than two distinct onsets.
    pub regularity_score: f32,
}

/// Bar-level self-similarity metrics for a pattern.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PatternRepetition {
    /// Number of distinct bar-content signatures across the pattern. Bars are
    /// hashed by `(onset_quantized_to_32nd, midi_note)`; durations and
    /// velocities are ignored.
    pub distinct_bars: u32,
    /// `ceil(length_ticks / ticks_per_bar)`.
    pub total_bars: u32,
    /// `0.0..=1.0`. `1.0` = every bar carries the same notes; `0.0` = every
    /// bar unique. `0.0` for single-bar patterns (no repetition to measure)
    /// — accompanied by a warning.
    pub bar_repetition_score: f32,
}

/// Output of `analyze_pattern`.
///
/// Pure symbolic — no audio rendering. Reads a pattern's notes directly and
/// reports density, pitch shape, velocity dynamics, rhythmic structure, and
/// bar-level repetition.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzePatternResult {
    /// Pattern ID that was analyzed.
    pub pattern_id: u32,
    /// Pattern name (may be empty).
    pub pattern_name: String,
    /// Authored pattern length, in ticks.
    pub length_ticks: u32,
    /// Authored pattern length expressed in bars under the song's default
    /// time signature (may be fractional).
    pub length_bars: f32,
    /// Time-signature numerator used for `length_bars` / `notes_per_bar`.
    pub time_signature_numerator: u8,
    /// Time-signature denominator used for `length_bars` / `notes_per_bar`.
    pub time_signature_denominator: u8,
    /// Total note count after dropping out-of-bounds notes.
    pub note_count: u32,
    pub density: PatternDensity,
    pub pitch: PatternPitch,
    pub velocity: PatternVelocity,
    pub rhythm: PatternRhythm,
    pub repetition: PatternRepetition,
    /// Warnings encountered during analysis (empty pattern, out-of-bounds
    /// notes, single-bar patterns where bar repetition isn't meaningful, …).
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// analyze_drum_groove result types
// ---------------------------------------------------------------------------

/// Per-component note counts inside the analyzed scope.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DrumComposition {
    pub kick: u32,
    pub snare: u32,
    pub hat_closed: u32,
    pub hat_open: u32,
    pub tom: u32,
    pub cymbal: u32,
    pub clap: u32,
    /// Anything outside the General MIDI percussion map (Cowbell, Tambourine,
    /// or custom drum maps the user defined themselves).
    pub other: u32,
}

/// Backbeat = snare hits landing on beats 2 and 4 (in 4-beat bars). `strength`
/// is the fraction of expected backbeat positions that actually carry a snare
/// hit (1.0 = tight, 0.0 = no backbeat).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DrumBackbeat {
    /// 0.0..=1.0.
    pub strength: f32,
    /// Total number of expected backbeat slots across the scope.
    pub expected_backbeats: u32,
    /// How many of those slots had a snare hit within tolerance.
    pub matched_backbeats: u32,
    /// Snare hits that did NOT land near a backbeat slot.
    pub off_backbeat_snares: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DrumHat {
    /// `"quarter" | "8th" | "16th" | "triplet_8th" | "triplet_16th" |
    /// "irregular" | "none"`. `"none"` when no hat hits were found.
    pub subdivision: String,
    pub hat_density_per_beat: f32,
    pub hat_count: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DrumGhostNotes {
    pub count: u32,
    /// Velocity threshold (0..=1) below which a snare hit is counted as a
    /// ghost note. Reported so callers can sanity-check the heuristic.
    pub velocity_threshold: f32,
}

/// Fill detection per bar. A bar is flagged as a "fill candidate" when its
/// drum-note density exceeds a fixed multiple of the mean drum density across
/// the whole range.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DrumFills {
    pub fill_bar_count: u32,
    pub density_threshold: f32,
    pub mean_density_per_bar: f32,
}

/// Bar-level self-similarity over drum notes.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct DrumRepetition {
    pub distinct_bars: u32,
    pub total_bars: u32,
    /// `0.0..=1.0`. `1.0` = every bar carries the same drum hits; `0.0` =
    /// every bar unique. `0.0` for fewer than 2 bars of drum activity.
    pub bar_repetition_score: f32,
}

/// One drum track that contributed notes to the analysis (arrangement scope
/// only — `analyze_drum_groove` lists every track classified as Drums with
/// confidence >= 0.6 by [`crate::types::InstrumentProfileResult`]).
#[derive(Debug, Clone, Serialize)]
pub struct DrumTrackInfo {
    pub track_id: u16,
    pub track_name: String,
    pub instrument_id: u16,
    pub instrument_name: String,
    pub drum_confidence: f32,
}

/// Output of `analyze_drum_groove`.
///
/// Pure symbolic — drum-feel diagnostics built on top of
/// `analysis::infer_all_profiles`. Reports backbeat strength, hat subdivision,
/// ghost-note count, fill candidates, and bar-level repetition. Use this to
/// answer "why does this beat sound flat?" without listening.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeDrumGrooveResult {
    /// What was analyzed (pattern id or arrangement range).
    pub scope: HarmonyScope,
    /// 1-indexed bar number at scope start (arrangement scope) or 1 (pattern
    /// scope).
    pub start_bar: u32,
    /// 1-indexed beat within `start_bar`.
    pub start_beat: u32,
    /// 1-indexed bar number at scope end.
    pub end_bar: u32,
    /// 1-indexed beat within `end_bar`.
    pub end_beat: u32,
    /// Span in ticks the analyzer actually walked.
    pub length_ticks: u32,
    /// `length_ticks` expressed in bars under the scope time signature.
    pub length_bars: f32,
    pub time_signature_numerator: u8,
    pub time_signature_denominator: u8,
    /// Total drum-note count contributing to the analysis.
    pub total_drum_notes: u32,
    /// Drum tracks the analyzer pulled notes from. Empty in pattern scope.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub drum_tracks: Vec<DrumTrackInfo>,
    pub composition: DrumComposition,
    pub backbeat: DrumBackbeat,
    pub hat: DrumHat,
    pub ghost_notes: DrumGhostNotes,
    pub fills: DrumFills,
    pub repetition: DrumRepetition,
    /// Warnings encountered during analysis (empty scope, no drum tracks
    /// found, single-bar drum activity, …). Empty when analysis ran cleanly.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// analyze_bass_drum_lock result types
// ---------------------------------------------------------------------------

/// One bass-only track that contributed notes to a bass/drum-lock analysis.
/// Reported alongside the drum tracks so callers can see what the analyzer
/// pulled notes from. Mirrors `DrumTrackInfo` but for bass.
#[derive(Debug, Clone, Serialize)]
pub struct BassTrackInfo {
    pub track_id: u16,
    pub track_name: String,
    pub instrument_id: u16,
    pub instrument_name: String,
    pub bass_confidence: f32,
}

/// Onset-level kick/bass alignment metrics. The two scores answer different
/// questions: `lock_score` is "how often does the kick get bass support?";
/// `coverage_score` is "how often does the bass have a kick beneath it?".
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct BassDrumAlignment {
    pub matched_onsets: u32,
    pub kick_only: u32,
    pub bass_only: u32,
    /// `matched_onsets / kick_onset_count` (0.0..=1.0). Higher = tighter
    /// lock. 0.0 when there are no kicks.
    pub lock_score: f32,
    /// `matched_onsets / bass_onset_count` (0.0..=1.0). Higher = bass mostly
    /// supported by kicks. 0.0 when there are no bass onsets.
    pub coverage_score: f32,
}

/// Bass-pitch stability on the matched (kick + bass) onsets. The bass usually
/// plays the chord root on the kick — high `on_kick_root_share` is the
/// fingerprint of a "rooted" bass line; low share + many distinct PCs is the
/// fingerprint of a walking or melodic bass.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BassPitchStability {
    /// Most common bass pitch class on matched onsets (0 = C, 11 = B).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_kick_root_pc: Option<u8>,
    /// Name of that pitch class for human-readable output (e.g. `"C"`,
    /// `"F#"`). Skipped from JSON when no matched onsets exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_kick_root_name: Option<String>,
    /// Fraction of matched onsets that hit `on_kick_root_pc` (0.0..=1.0).
    pub on_kick_root_share: f32,
    /// Distinct pitch classes the bass plays on matched onsets.
    pub distinct_pcs_on_kick: u32,
    /// Distinct pitch classes the bass plays across the entire scope.
    pub distinct_pcs_total: u32,
    /// Mean MIDI pitch of the bass across all onsets.
    pub mean_bass_midi: f32,
}

/// Output of `analyze_bass_drum_lock`.
///
/// Pure symbolic kick/bass relationship diagnostics. Identifies drum tracks
/// (for kicks: GM MIDI 35/36) and bass tracks (`infer_all_profiles` → Role
/// `Bass`) the same way `analyze_harmony`'s drum filter does, then walks
/// onsets and reports onset alignment, kick/bass solo counts, and a bass-
/// pitch stability summary.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeBassDrumLockResult {
    pub scope: HarmonyScope,
    pub start_bar: u32,
    pub start_beat: u32,
    pub end_bar: u32,
    pub end_beat: u32,
    pub length_ticks: u32,
    pub length_bars: f32,
    pub time_signature_numerator: u8,
    pub time_signature_denominator: u8,
    /// Drum tracks the analyzer pulled kicks from. Empty in pattern scope.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub drum_tracks: Vec<DrumTrackInfo>,
    /// Bass tracks the analyzer pulled notes from. Empty in pattern scope.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub bass_tracks: Vec<BassTrackInfo>,
    pub kick_onset_count: u32,
    pub bass_onset_count: u32,
    /// Maximum |Δtick| between a kick and a bass onset that still counts as
    /// a match. Reported so callers can see the tolerance band the
    /// metrics were computed against; clamped to `[120, 960]` internally.
    pub onset_tolerance_ticks: u32,
    pub alignment: BassDrumAlignment,
    pub bass_pitch: BassPitchStability,
    /// Warnings encountered during analysis. Empty when analysis ran cleanly.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// analyze_harmonic_function result types
// ---------------------------------------------------------------------------

/// One chord event annotated with its tonal function. Mirrors
/// `analysis::ChordFunction`; enums travel as snake_case strings so the MCP
/// crate stays free of pertylizer-side types.
#[derive(Debug, Clone, Serialize)]
pub struct ChordFunctionEvent {
    /// Chord symbol, e.g. `"Cm7"`. `"?"` when no chord was identified.
    pub symbol: String,
    /// 1-indexed bar number at the chord-event start, using the song's time
    /// signature at `start_tick`.
    pub start_bar: u32,
    /// 1-indexed beat within `start_bar`.
    pub start_beat: u32,
    /// Window start in absolute ticks (or pattern-relative for pattern scope).
    pub start_tick: u64,
    /// Window end (exclusive).
    pub end_tick: u64,
    /// Scale degree (1..=7 for diatonic, omitted for chromatic / unidentified
    /// chords).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_degree: Option<u8>,
    /// Roman numeral with quality decoration, e.g. `"V7"`, `"ii7"`, `"vii°"`.
    /// For chromatic roots the analyzer emits altered numerals like `"bII"`.
    /// `"?"` when no chord was identified at this position.
    pub roman_numeral: String,
    /// `"tonic" | "subdominant" | "dominant" | "other" | "chromatic"`.
    pub function: String,
    /// 0.0..=1.0. Per-chord tension contribution (function-based + extra
    /// from dominant 7th / diminished qualities).
    pub tension: f32,
    /// `chord.in_key` from `analyze_harmony` — true when every pitch in the
    /// window belongs to the inferred key's scale.
    pub in_key: bool,
    /// `"authentic" | "plagal" | "half_cadence" | "deceptive"` when this
    /// chord closes a cadence with the previous chord. Skipped otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cadence: Option<String>,
}

/// Cadence event — one entry per detected cadence ending in the chord stream.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct HarmonicCadenceEvent {
    /// Index into `AnalyzeHarmonicFunctionResult.chords` of the closing
    /// chord of the cadence.
    pub chord_index: u32,
    /// `"authentic" | "plagal" | "half_cadence" | "deceptive"`.
    pub kind: HarmonicCadenceKind,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarmonicCadenceKind {
    Authentic,
    Plagal,
    HalfCadence,
    Deceptive,
}

/// Per-function counts across the chord stream.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct FunctionDistribution {
    pub tonic: u32,
    pub subdominant: u32,
    pub dominant: u32,
    pub other: u32,
    pub chromatic: u32,
}

/// Tension-curve summary.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct TensionStats {
    /// Mean per-chord tension across the analyzed range (0.0..=1.0).
    pub mean: f32,
    /// Highest per-chord tension in the range.
    pub peak: f32,
    /// Lowest per-chord tension in the range.
    pub trough: f32,
    /// Standard deviation of per-chord tension (0 for ≤ 1 chord).
    pub std_dev: f32,
}

/// Output of `analyze_harmonic_function`.
///
/// Builds on `analyze_harmony`: same scope (pattern or arrangement range),
/// same key inference. For each chord event, the analyzer assigns a scale-
/// degree Roman numeral, a tonal-function bucket, and a tension score;
/// cadences are detected on consecutive chord pairs.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeHarmonicFunctionResult {
    pub scope: HarmonyScope,
    /// Inferred key tonic name (`"C"`, `"F#"`, …) and mode (`"major"` /
    /// `"minor"`). `None` for empty input or when the analyzer could not
    /// infer a key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<HarmonyKeyEstimate>,
    /// Chord events in time order, each annotated with function + Roman
    /// numeral + tension.
    pub chords: Vec<ChordFunctionEvent>,
    /// Detected cadences. Each entry points into `chords`.
    pub cadences: Vec<HarmonicCadenceEvent>,
    pub function_distribution: FunctionDistribution,
    pub tension: TensionStats,
    /// Warnings emitted by the underlying `analyze_harmony` call AND by the
    /// function-analysis layer.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// analyze_instrument_range result types
// ---------------------------------------------------------------------------

/// One step in a per-note sweep across a MIDI range. Captures the subset of
/// `AnalyzeNoteResult` that's useful for spotting per-note breakage — full
/// per-note `AnalyzeNoteResult` blobs would blow the response budget on a
/// 60-note sweep.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct InstrumentRangeStep {
    /// MIDI note requested.
    pub note: u8,
    /// MIDI note actually played, after the patch's `octave_offset` was
    /// applied. Differs from `note` for bass patches with offset != 0.
    pub note_played: u8,
    /// Concert-pitch frequency of `note_played`.
    pub expected_hz: f32,
    /// Detected fundamental in Hz. `0.0` when the renderer was silent or no
    /// peak survived the spectral floor.
    pub fundamental_hz: f32,
    /// Detected pitch error in cents (positive = sharp). `0.0` when either
    /// `fundamental_hz` or `expected_hz` is zero.
    pub pitch_error_cents: f32,
    /// Confidence in `fundamental_hz` in `0.0..=1.0` (prominence-based).
    /// Low values mean the spectrum is noisy or has competing peaks.
    pub pitch_confidence: f32,
    /// Peak absolute amplitude across the whole render (1.0 = full scale).
    pub peak_amplitude: f32,
    /// Overall RMS amplitude.
    pub rms_overall: f32,
    /// Spectral centroid (Hz) at the sustained mid-region. A proxy for
    /// brightness; sharp jumps across the range hint at aliasing.
    pub centroid_hz: f32,
    /// Count of samples whose absolute value reached or exceeded 0.999.
    pub clipped_samples: u32,
    /// True when `peak_amplitude` is below the silence floor (≈ −60 dBFS).
    /// The patch did not produce audible output at this note.
    pub silent: bool,
    /// True when `pitch_confidence < 0.3` AND the centroid is above
    /// half the Nyquist. Signals likely aliasing in the top octaves.
    pub likely_aliased: bool,
    /// True when the pitch tracker reported `fundamental_hz < expected_hz / 2`
    /// or `> expected_hz * 2`, when `|pitch_error_cents| > 1200`, or when
    /// confidence dropped below `0.20`. Catches patches that fall apart at
    /// the extremes — large detune *or* hopeless pitch tracking both count.
    pub pitch_lost: bool,
    /// True when `pitch_confidence < 0.10`. The fundamental and centroid
    /// measurements at this step are noise-level — readings should not be
    /// trusted even if `pitch_lost` is false. Stricter than `pitch_lost`
    /// because it answers a different question ("is the reading usable?"
    /// vs "did the patch break?").
    pub pitch_unreliable: bool,
}

/// Cross-range warnings derived from a `Vec<InstrumentRangeStep>`. Each flag
/// surfaces a specific bug class an agent would otherwise have to spot by
/// eyeballing the per-step list.
#[derive(Debug, Clone, Default, Serialize)]
pub struct InstrumentRangeIssues {
    /// Notes (by MIDI number) where `silent == true`. Empty when the patch
    /// produced audible output at every step.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub silent_notes: Vec<u8>,
    /// Notes where `likely_aliased == true`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub aliased_notes: Vec<u8>,
    /// Notes where `pitch_lost == true`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub pitch_lost_notes: Vec<u8>,
    /// Notes where `pitch_unreliable == true` — confidence under 0.10, so the
    /// fundamental and centroid columns for that step are not meaningful.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub pitch_unreliable_notes: Vec<u8>,
    /// Notes where `clipped_samples > 0`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub clipping_notes: Vec<u8>,
    /// `peak_amplitude` at the loudest non-silent step minus the quietest
    /// non-silent step, in dB. Large values indicate the patch's level
    /// varies wildly across its range (a known mix-bus surprise source).
    /// `0.0` when fewer than two non-silent steps were found.
    pub level_spread_db: f32,
}

/// Output of `analyze_instrument_range`. Sweeps an instrument across a MIDI
/// range, runs the same render-and-analyze pipeline as `analyze_note` at each
/// step, and returns per-step metrics plus a cross-step issue summary.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeInstrumentRangeResult {
    /// Instrument that was swept.
    pub instrument_id: u64,
    /// Velocity used at every step.
    pub velocity: u8,
    /// Lowest MIDI note in the sweep (`low_note`).
    pub low_note: u8,
    /// Highest MIDI note in the sweep (`high_note`).
    pub high_note: u8,
    /// Semitone gap between consecutive steps.
    pub step_semitones: u8,
    /// Note duration in ms (forwarded to the renderer).
    pub duration_ms: u32,
    /// Tail in ms after note-off (forwarded to the renderer).
    pub tail_ms: u32,
    /// One entry per swept note, in ascending MIDI order.
    pub steps: Vec<InstrumentRangeStep>,
    /// Cross-step warnings.
    pub issues: InstrumentRangeIssues,
    /// Non-fatal warnings collected during the sweep (renderer failures on
    /// individual notes, malformed parameters, etc.).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// analyze_velocity_response result types
// ---------------------------------------------------------------------------

/// One step in a velocity sweep at a fixed MIDI note.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct VelocityResponseStep {
    /// Velocity used for this step (1..=127).
    pub velocity: u8,
    /// Peak absolute amplitude across the whole render.
    pub peak_amplitude: f32,
    /// Overall RMS amplitude.
    pub rms_overall: f32,
    /// Spectral centroid (Hz) at the sustained mid-region. Should generally
    /// rise with velocity on patches with velocity → filter routing.
    pub centroid_hz: f32,
    /// Count of clipped samples at this velocity.
    pub clipped_samples: u32,
}

/// Cross-velocity diagnostics derived from a `Vec<VelocityResponseStep>`.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct VelocityResponseIssues {
    /// `peak_amplitude` at the loudest step minus the quietest, in dB.
    /// `0.0` when fewer than two non-silent steps were found. Large values
    /// = strongly velocity-sensitive; values < 6 dB = patch is barely
    /// responsive to velocity.
    pub amplitude_range_db: f32,
    /// Number of adjacent step pairs where peak_amplitude decreased as
    /// velocity increased. `0` for a clean monotonic response. Non-zero
    /// values indicate a non-musical velocity curve.
    pub non_monotonic_amplitude_steps: u32,
    /// Number of adjacent step pairs where centroid_hz decreased as
    /// velocity increased. Centroid is allowed to be flat (patch lacks
    /// velocity → filter modulation) but should not invert.
    pub non_monotonic_centroid_steps: u32,
    /// True when the patch is effectively velocity-insensitive
    /// (`amplitude_range_db < 3.0`) — flagged because patches that ignore
    /// velocity often surprise users.
    pub velocity_unresponsive: bool,
    /// True when the patch is musically compressed by velocity
    /// (`3.0 <= amplitude_range_db < 10.0`). The patch responds to velocity
    /// but barely — sits between "unresponsive" and "normal".
    pub velocity_compressed_response: bool,
    /// True when at least half of the centroid transitions invert AND the
    /// last step's centroid is below the first step's — velocity makes the
    /// patch *darker* instead of brighter. Almost always a routing mistake.
    pub velocity_brightness_inverted: bool,
}

/// Output of `analyze_velocity_response`. Holds one MIDI note across a swept
/// velocity range and returns per-velocity amplitude / brightness curves plus
/// monotonicity / responsiveness flags.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeVelocityResponseResult {
    pub instrument_id: u64,
    /// MIDI note held throughout the sweep.
    pub note: u8,
    /// Lowest velocity in the sweep.
    pub velocity_low: u8,
    /// Highest velocity in the sweep.
    pub velocity_high: u8,
    /// Step size between consecutive velocities.
    pub velocity_step: u8,
    /// Note duration in ms (forwarded to the renderer).
    pub duration_ms: u32,
    /// Tail in ms after note-off (forwarded to the renderer).
    pub tail_ms: u32,
    /// One entry per swept velocity, in ascending order.
    pub steps: Vec<VelocityResponseStep>,
    /// Cross-step diagnostics.
    pub issues: VelocityResponseIssues,
    /// Non-fatal warnings collected during the sweep.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Group B — symbolic composition helper results
// ---------------------------------------------------------------------------

/// Output of `generate_chord`. Pure symbolic — returns MIDI notes for a
/// requested chord symbol so the caller can place them with `add_note`.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateChordResult {
    /// Chord symbol echoed back.
    pub symbol: String,
    /// Root pitch class (0..12, C = 0).
    pub root_pitch_class: u8,
    /// Matched quality (`"major"`, `"minor7"`, `"sus4"`, …).
    pub quality: String,
    /// Suffix that matched (`""`, `"m7"`, `"sus4"`, …).
    pub suffix: String,
    /// Voicing that was actually applied (may differ from the request when
    /// the chord has fewer notes than the voicing needs).
    pub voicing: String,
    /// Generated MIDI notes in ascending order.
    pub notes: Vec<u8>,
    /// Non-fatal warnings (voicing fallback, MIDI clamping, …).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// One chord placed by `create_chord_progression_pattern`.
#[derive(Debug, Clone, Serialize)]
pub struct ChordProgressionStep {
    /// Chord symbol as supplied (e.g. "Gm").
    pub symbol: String,
    /// Beat where the chord starts in the pattern.
    pub start_beat: f32,
    /// Matched chord quality (e.g. "minor", "major7").
    pub quality: String,
    /// Voicing actually applied.
    pub voicing: String,
    /// MIDI notes placed for this chord.
    pub notes: Vec<u8>,
}

/// Result of `create_chord_progression_pattern`.
#[derive(Debug, Clone, Serialize)]
pub struct CreateChordProgressionResult {
    /// ID of the newly created pattern.
    pub pattern_id: u32,
    /// Total pattern length in beats.
    pub length_beats: f32,
    /// Total notes placed across all chords.
    pub notes_added: usize,
    /// Per-chord breakdown in playback order.
    pub chords: Vec<ChordProgressionStep>,
    /// Non-fatal warnings (voicing fallback, MIDI clamping, …), prefixed by symbol.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Output of `transpose_notes`. Pure symbolic — mutates a single pattern in
/// place and returns aggregate stats.
#[derive(Debug, Clone, Serialize)]
pub struct TransposeNotesResult {
    pub pattern_id: u32,
    /// Signed semitone shift requested.
    pub semitones: i32,
    /// Number of notes in the pattern before the call.
    pub notes_in: u32,
    /// Notes whose new pitch landed inside the valid MIDI range and were
    /// updated. Notes whose new pitch would have fallen outside are left
    /// untouched and counted in `notes_out_of_range`.
    pub notes_transposed: u32,
    pub notes_out_of_range: u32,
    /// Notes whose raw transposed pitch was outside the requested scale and
    /// snapped to the nearest in-scale pitch. `0` when no scale was given.
    pub notes_snapped_to_scale: u32,
    /// Echo of the scale constraint that was applied (when any).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scale_tonic_pitch_class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scale_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Output of `quantize_notes_to_scale`.
#[derive(Debug, Clone, Serialize)]
pub struct QuantizeNotesToScaleResult {
    pub pattern_id: u32,
    /// Scale tonic (0..12, C = 0).
    pub scale_tonic_pitch_class: u8,
    /// Scale template that was applied (may differ from the requested name
    /// when an unknown name fell back to major).
    pub scale_name: String,
    pub notes_in: u32,
    pub notes_already_in_scale: u32,
    pub notes_moved: u32,
    /// Mean absolute correction in semitones across `notes_moved`. `0.0`
    /// when no notes were moved.
    pub mean_correction_semitones: f32,
    /// Largest single-note correction in semitones.
    pub max_correction_semitones: u8,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Output of `quantize_notes_to_grid`.
#[derive(Debug, Clone, Serialize)]
pub struct QuantizeNotesToGridResult {
    pub pattern_id: u32,
    /// Grid resolution (in ticks) the snap was performed against.
    pub grid_ticks: u32,
    /// Quantize strength that was applied (0..1).
    pub strength: f32,
    /// Swing amount (0..1) — even-indexed grid positions stay, odd
    /// positions push back by up to half the grid distance.
    pub swing: f32,
    /// Maximum ±jitter applied to each note after the grid snap.
    pub humanize_ticks: u32,
    /// Seed used for humanization. Returning this lets callers reproduce a
    /// particular pass.
    pub humanize_seed: u64,
    pub notes_in: u32,
    pub notes_moved: u32,
    /// Mean absolute tick delta across `notes_moved`.
    pub mean_delta_ticks: f32,
    pub max_delta_ticks: u32,
    /// Pattern length in ticks at the time of the call (echoed for context).
    pub pattern_length_ticks: u32,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Group C — form & motif analysis result types
// ---------------------------------------------------------------------------

/// Per-bar feature summary shared by `analyze_arrangement` and
/// `analyze_form_map`. Compact projection of every bar in the analyzed scope:
/// note density, distinct pitch classes, dominant pitch class, mean velocity,
/// and the set of tracks that contributed notes to the bar. Lets the caller
/// inspect the raw matrix the section clustering ran on.
#[derive(Debug, Clone, Serialize)]
pub struct BarFeatureSummary {
    /// 1-indexed bar number in the scope (1 = first bar of the analyzed range).
    pub bar: u32,
    /// Number of melodic notes that started inside this bar.
    pub note_count: u32,
    /// Distinct pitch classes used in the bar (0..=12).
    pub distinct_pitch_classes: u8,
    /// Dominant pitch class (0..12) — duration-weighted argmax of the bar's
    /// pitch-class histogram. Omitted for empty bars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dominant_pitch_class: Option<u8>,
    /// Mean velocity (0..=1) across notes that started in the bar.
    pub mean_velocity: f32,
    /// Track IDs that had at least one note start in this bar. Empty in
    /// pattern scope (single pattern = single virtual track).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub active_track_ids: Vec<u16>,
}

/// One detected section in `analyze_arrangement` — a contiguous bar range
/// labeled by clustering similar adjacent bars and matching against
/// previously seen labels.
#[derive(Debug, Clone, Serialize)]
pub struct SectionSpan {
    /// Section label assigned by clustering (`"A"`, `"B"`, `"A'"`, …). Labels
    /// reflect first-appearance order; primes mark near-matches to an earlier
    /// label that fell just under the equality threshold.
    pub label: String,
    /// 1-indexed start bar (inclusive) of the section in the analyzed scope.
    pub start_bar: u32,
    /// 1-indexed end bar (inclusive) of the section.
    pub end_bar: u32,
    /// Section length in bars.
    pub length_bars: u32,
    /// Mean notes per bar across the section.
    pub mean_notes_per_bar: f32,
    /// Mean distinct pitch-class count per bar across the section.
    pub mean_distinct_pitch_classes: f32,
    /// Mean velocity across notes in the section (0..=1).
    pub mean_velocity: f32,
    /// Distinct track IDs that contributed notes anywhere inside the section.
    /// Empty in pattern scope.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub active_track_ids: Vec<u16>,
}

/// Output of `analyze_arrangement`. Section-level structural diagnostic built
/// from bar features and self-similarity clustering: detects repeating
/// sections, labels them, and reports per-section stats.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeArrangementResult {
    pub scope: HarmonyScope,
    pub start_bar: u32,
    pub start_beat: u32,
    pub end_bar: u32,
    pub end_beat: u32,
    pub length_ticks: u64,
    pub length_bars: u32,
    pub time_signature_numerator: u8,
    pub time_signature_denominator: u8,
    /// Cosine-similarity threshold above which two adjacent bars merge into
    /// one section, and above which a new section is considered equivalent
    /// to a previously labeled one.
    pub similarity_threshold: f32,
    /// Per-bar feature rows (1-indexed bars, length = `length_bars`).
    pub bars: Vec<BarFeatureSummary>,
    /// Detected sections in time order. Sections never overlap and cover
    /// every bar of the scope.
    pub sections: Vec<SectionSpan>,
    /// Number of distinct section labels found (`"A"` and `"A'"` count as
    /// the same root label).
    pub distinct_section_count: u32,
    /// Warnings (empty scope, fewer than 2 bars, ambiguous clustering, …).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Output of `analyze_form_map`. Compact view of the same clustering as
/// `analyze_arrangement` — one label per bar, plus a coalesced run-length
/// "form string" (`"AABA"`, `"ABACABA"`, …) that captures song structure at
/// a glance.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeFormMapResult {
    pub scope: HarmonyScope,
    pub start_bar: u32,
    pub start_beat: u32,
    pub end_bar: u32,
    pub end_beat: u32,
    pub length_bars: u32,
    pub time_signature_numerator: u8,
    pub time_signature_denominator: u8,
    pub similarity_threshold: f32,
    /// One label per bar in time order. Length = `length_bars`. `"·"` for
    /// empty bars (no melodic notes).
    pub bar_labels: Vec<String>,
    /// Run-length compression of `bar_labels`: each adjacent run of the
    /// same label collapses to a single character. Example: `["A", "A",
    /// "B", "A"]` → `"ABA"` (with section_min_bars merging applied first).
    pub form_string: String,
    /// Section spans (same as `analyze_arrangement.sections`) so the caller
    /// can recover where each form-string letter starts and ends without
    /// re-running clustering.
    pub sections: Vec<SectionSpan>,
    pub distinct_section_count: u32,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// One occurrence of a motif inside the analyzed scope.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct MotifOccurrence {
    /// Track that the motif starts on. `0` in pattern scope.
    pub track_id: u16,
    /// Absolute start tick of the first note in the motif. Pattern-relative
    /// in pattern scope.
    pub start_tick: u64,
    /// 1-indexed bar number at the motif start.
    pub start_bar: u32,
    /// 1-indexed beat within `start_bar`.
    pub start_beat: u32,
    /// MIDI pitch of the first note in the motif — useful for spotting
    /// transposed restatements without having to re-derive the contour.
    pub first_pitch: u8,
}

/// One motif candidate — a recurring sequence of pitch intervals.
#[derive(Debug, Clone, Serialize)]
pub struct MotifEntry {
    /// Length in intervals (i.e. one less than the number of notes).
    pub length: u8,
    /// Signed pitch deltas in semitones, in occurrence order. Length =
    /// `length`. Encoded as `i16` for JSON readability — single notes never
    /// hit i16 bounds.
    pub intervals: Vec<i16>,
    /// Number of times this exact interval sequence appears in the scope.
    pub count: u32,
    /// Where the motif occurs. Sorted by `(track_id, start_tick)`.
    pub occurrences: Vec<MotifOccurrence>,
}

/// Output of `find_motifs`. Lists the top-N pitch-interval n-grams that
/// recur at least `min_count` times across the scope. Transposition-
/// invariant — the same melodic shape rooted at different pitches collapses
/// to one entry.
#[derive(Debug, Clone, Serialize)]
pub struct FindMotifsResult {
    pub scope: HarmonyScope,
    pub start_bar: u32,
    pub start_beat: u32,
    pub end_bar: u32,
    pub end_beat: u32,
    /// Echoed search parameters so the caller can audit what was scanned.
    pub min_interval_length: u8,
    pub max_interval_length: u8,
    pub min_count: u32,
    /// Number of melodic notes considered (drum tracks excluded by default).
    pub total_notes: u32,
    /// Motifs sorted by descending `score = length * log2(1 + count)`,
    /// truncated to `top_n`. Empty when no n-gram met `min_count`.
    pub motifs: Vec<MotifEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// Output of `analyze_hook_strength`. Single-number diagnostic answering
/// "does this song actually have a hook?" — high score = a clear, long,
/// repeating motif covers a meaningful share of the melodic notes; low
/// score = melody noodles without a memorable shape.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeHookStrengthResult {
    pub scope: HarmonyScope,
    pub start_bar: u32,
    pub start_beat: u32,
    pub end_bar: u32,
    pub end_beat: u32,
    pub total_notes: u32,
    /// Hook score in `0..=1`. Combines longest-recurring motif length,
    /// repeat count, and the fraction of melodic notes that participate in
    /// some repeating motif. `0.0` when no motif meets the threshold.
    pub hook_score: f32,
    /// Fraction of melodic notes that belong to at least one motif of
    /// length ≥ `min_interval_length` that recurs ≥ `min_count` times.
    pub coverage_ratio: f32,
    /// Strongest motif found (longest × repeat count tie-break). Skipped
    /// when no motif met the threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strongest_motif: Option<MotifEntry>,
    pub min_interval_length: u8,
    pub min_count: u32,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Group D — meta-analysis result types
// ---------------------------------------------------------------------------

/// Per-bar entry in `analyze_tension_curve`. All scalar fields are normalized
/// to `[0, 1]` unless noted, so the caller can plot them on a single axis or
/// feed them into a higher-level fix-ranker without extra work.
///
/// Audio-derived fields (`loudness_score`, `brightness`, `band_entropy`,
/// `stereo_width_score`) are `None` when the caller asked for the cheap
/// symbolic-only mode (or when the scope was too short to render).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TensionCurveBar {
    /// 1-indexed bar number inside the analyzed scope.
    pub bar: u32,
    /// Mean per-chord tension across chord windows that overlap the bar
    /// (from `analyze_harmonic_function`). `0.0` when no chord touches the
    /// bar.
    pub harmonic_tension: f32,
    /// Fraction of in-bar chord-window-ticks marked out of key (proxy for
    /// vertical dissonance). `0.0` when no chord touches the bar.
    pub dissonance: f32,
    /// `note_count / 16`, clamped to 1. Saturates for very busy bars so
    /// arpeggios don't dominate the composite score.
    pub density_score: f32,
    /// `(mean_midi - 36) / 60`, clamped — places C2 near 0 and C7 near 1.
    pub register_score: f32,
    /// Distinct 16th-note onset positions / 16, clamped. Picks up
    /// syncopation without being inflated by chordal multi-note onsets.
    pub rhythmic_activity: f32,
    /// Mean velocity (0..=1) across notes that start in the bar.
    pub mean_velocity: f32,
    /// Distinct track IDs that contributed notes to the bar.
    pub active_track_count: u32,
    /// Loudness score: LUFS-M (or RMS dBFS for very short bars) mapped from
    /// `[-50, -10]` dB onto `[0, 1]`. Omitted in symbolic-only mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loudness_score: Option<f32>,
    /// (mid + high) / total energy. Higher = brighter. Omitted in
    /// symbolic-only mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f32>,
    /// Shannon entropy across the 4 energy bands, normalized by `ln(4)`.
    /// 1.0 = perfectly flat spectrum; 0.0 = single-band signal. Omitted in
    /// symbolic-only mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band_entropy: Option<f32>,
    /// `side_rms / mid_rms`, clamped to `[0, 1]`. Omitted in symbolic-only
    /// mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereo_width_score: Option<f32>,
    /// Final composite tension in `[0, 1]`. Symbolic-only blend is 35 %
    /// harmonic + 15 % dissonance + 20 % density + 10 % register + 20 %
    /// rhythm. Audio-augmented blend keeps 60 % of the symbolic score and
    /// adds 20 % loudness + 12 % brightness + 8 % band entropy.
    pub composite_tension: f32,
}

/// Summary statistics over the per-bar `composite_tension` series.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TensionCurveSummary {
    /// 1-indexed bar with the highest composite tension. `None` for empty
    /// scopes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_bar: Option<u32>,
    pub peak_value: f32,
    /// 1-indexed bar with the lowest composite tension *among bars that
    /// have content* — purely empty bars are skipped so an intro of rests
    /// doesn't always win. `None` when every bar is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trough_bar: Option<u32>,
    pub trough_value: f32,
    pub mean: f32,
    /// Standard deviation across the composite-tension series.
    pub std_dev: f32,
}

/// Output of `analyze_tension_curve`. Bar-level tension diagnostic built
/// from the existing harmony/dynamics/spectral analyzers — flags shape
/// issues like a chorus that does not lift, a build that peaks too early,
/// drops that lose low-end energy, and otherwise monotone curves.
#[derive(Debug, Clone, Serialize)]
pub struct AnalyzeTensionCurveResult {
    pub scope: HarmonyScope,
    pub start_bar: u32,
    pub start_beat: u32,
    pub end_bar: u32,
    pub end_beat: u32,
    pub length_bars: u32,
    pub time_signature_numerator: u8,
    pub time_signature_denominator: u8,
    /// True when the result includes audio-derived axes (loudness,
    /// brightness, band entropy, stereo width). Driven by `include_audio` at
    /// call time and whether the scope was renderable.
    pub has_audio: bool,
    /// Per-bar tension breakdown.
    pub bars: Vec<TensionCurveBar>,
    /// Detected sections (same clustering used by `analyze_arrangement`) so
    /// the caller can map bars back to A/B/A' labels without re-running
    /// form analysis.
    pub sections: Vec<SectionSpan>,
    pub summary: TensionCurveSummary,
    /// Cross-bar / cross-section warnings: chorus-doesn't-lift,
    /// build-peaks-too-early, drop-loses-low-end, monotone-tension.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}

/// One ranked suggestion from `suggest_music_fixes`. Severity is in
/// `[0, 1]`; the caller can decide how aggressively to act on each entry.
#[derive(Debug, Clone, Serialize)]
pub struct FixSuggestion {
    /// Stable rule identifier, e.g. `"harmony.no_key_inferred"`. Lets the
    /// caller suppress specific rules in a follow-up call without parsing
    /// the title.
    pub id: String,
    /// One of `"harmony"`, `"mix"`, `"groove"`, `"arrangement"`,
    /// `"composition"`, `"patch"`.
    pub category: String,
    /// Severity in `[0, 1]`. Used to sort the list — higher = more urgent.
    pub severity: f32,
    /// Short headline ("Chorus reprise has lower energy than the original").
    pub title: String,
    /// Two-to-three sentence detail with concrete action language.
    pub detail: String,
    /// Numeric / textual evidence that triggered the rule. One entry per
    /// supporting measurement, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub evidence: Vec<String>,
}

/// Output of `suggest_music_fixes`. Packages diagnostics from harmony,
/// harmonic function, mix-bus, masking, groove, bass/drum-lock, form,
/// motifs/hook, tension curve, and patch analyzers into a ranked list of
/// concrete edits. No new measurements — the underlying analyzers are
/// authoritative and surface their full output through their own MCP
/// tools.
#[derive(Debug, Clone, Serialize)]
pub struct SuggestMusicFixesResult {
    pub scope: HarmonyScope,
    pub start_bar: u32,
    pub start_beat: u32,
    pub end_bar: u32,
    pub end_beat: u32,
    pub length_bars: u32,
    /// Whether the bridge ran the audio-render-backed analyzers (mix bus,
    /// masking, audio-augmented tension curve). False when the caller
    /// asked for symbolic-only mode.
    pub include_audio: bool,
    /// Category filters that were honored. Empty when no filter was
    /// supplied (all categories ran).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub categories: Vec<String>,
    /// Ranked suggestions, severity descending. Truncated to
    /// `max_suggestions` (default 15).
    pub suggestions: Vec<FixSuggestion>,
    /// Rule IDs that were considered but produced no suggestions (passed
    /// thresholds), so the caller can confirm what was checked.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub rules_clean: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
}
