//! MCP server implementation using rmcp.
//!
//! Defines tool handlers that delegate to the SynthBridge trait.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    AnnotateAble, Annotated, CallToolResult, Content, Implementation, ListResourceTemplatesResult,
    ListResourcesResult, PaginatedRequestParams, RawAudioContent, RawContent, RawResource,
    RawResourceTemplate, ReadResourceRequestParams, ReadResourceResult, ResourceContents,
    ServerCapabilities, ServerInfo,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};

use crate::bridge::SynthBridge;
use crate::error::McpBridgeError;

// === Helper functions ===

/// Serialize a value to pretty-printed JSON, returning an error string on failure.
fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| format!("Serialization error: {e}"))
}

fn validate_midi_note(note: u8) -> Result<(), McpBridgeError> {
    if note > 127 {
        return Err(McpBridgeError::InvalidMidiNote(note));
    }
    Ok(())
}

fn validate_velocity(velocity: u8) -> Result<(), McpBridgeError> {
    if velocity > 127 {
        return Err(McpBridgeError::InvalidVelocity(velocity));
    }
    Ok(())
}

fn validate_midi_channel(channel: u8) -> Result<(), McpBridgeError> {
    if !(1..=16).contains(&channel) {
        return Err(McpBridgeError::InvalidMidiChannel(channel));
    }
    Ok(())
}

fn validate_range(
    name: &'static str,
    value: f32,
    min: f32,
    max: f32,
) -> Result<(), McpBridgeError> {
    if value < min || value > max || value.is_nan() {
        return Err(McpBridgeError::ValueOutOfRange {
            name,
            value,
            min,
            max,
        });
    }
    Ok(())
}

fn validate_name(kind: &'static str, name: &str) -> Result<(), McpBridgeError> {
    if name.trim().is_empty() {
        return Err(McpBridgeError::EmptyName { kind });
    }
    Ok(())
}

/// Valid automation curve names.
const VALID_CURVES: &[&str] = &["Linear", "Step", "Exponential", "SCurve"];

fn validate_curve(curve: &str) -> Result<(), McpBridgeError> {
    if !VALID_CURVES.contains(&curve) {
        return Err(McpBridgeError::InvalidCurve(curve.to_string()));
    }
    Ok(())
}

/// Validate note fields that are always required (add_note, add_notes, etc.).
fn validate_note_fields(
    pitch: u8,
    velocity: u8,
    start_beat: f32,
    duration_beats: f32,
) -> Result<(), McpBridgeError> {
    validate_midi_note(pitch)?;
    validate_velocity(velocity)?;
    validate_range("start_beat", start_beat, 0.0, 9999.0)?;
    validate_range("duration_beats", duration_beats, f32::MIN_POSITIVE, 9999.0)?;
    Ok(())
}

/// Validate optional note update fields.
fn validate_note_update_fields(
    pitch: Option<u8>,
    velocity: Option<u8>,
    start_beat: Option<f32>,
    duration_beats: Option<f32>,
) -> Result<(), McpBridgeError> {
    if let Some(p) = pitch {
        validate_midi_note(p)?;
    }
    if let Some(v) = velocity {
        validate_velocity(v)?;
    }
    if let Some(s) = start_beat {
        validate_range("start_beat", s, 0.0, 9999.0)?;
    }
    if let Some(d) = duration_beats {
        validate_range("duration_beats", d, f32::MIN_POSITIVE, 9999.0)?;
    }
    Ok(())
}

/// Validate automation point fields.
fn validate_automation_point(pt: &AutomationPointInput) -> Result<(), McpBridgeError> {
    validate_range("value", pt.value, 0.0, 1.0)?;
    validate_range("beat", pt.beat, 0.0, 9999.0)?;
    if let Some(ref curve) = pt.curve {
        validate_curve(curve)?;
    }
    Ok(())
}

/// Validate connection indices against the modules array length.
fn validate_connection_indices(
    connections: &[ConnectionDefInput],
    module_count: usize,
) -> Result<(), McpBridgeError> {
    for conn in connections {
        if conn.from >= module_count {
            return Err(McpBridgeError::IndexOutOfBounds {
                name: "from",
                index: conn.from,
                count: module_count,
            });
        }
        if conn.to >= module_count {
            return Err(McpBridgeError::IndexOutOfBounds {
                name: "to",
                index: conn.to,
                count: module_count,
            });
        }
    }
    Ok(())
}

/// Validate a time signature: numerator 1..=32, denominator is a power of 2 (1..=32).
fn validate_time_signature(numerator: u8, denominator: u8) -> Result<(), McpBridgeError> {
    if !(1..=32).contains(&numerator) {
        return Err(McpBridgeError::ValueOutOfRange {
            name: "numerator",
            value: f32::from(numerator),
            min: 1.0,
            max: 32.0,
        });
    }
    if denominator == 0 || !denominator.is_power_of_two() || denominator > 32 {
        return Err(McpBridgeError::Other(format!(
            "denominator must be a power of 2 (1,2,4,8,16,32), got {denominator}"
        )));
    }
    Ok(())
}

/// Validate fields common to `BuildInstrumentParam` / `InstrumentDefInput`.
fn validate_build_instrument_fields(
    name: &str,
    midi_channel: Option<u8>,
    volume: Option<f32>,
    pan: Option<f32>,
    modules: &[ModuleDefInput],
    connections: Option<&[ConnectionDefInput]>,
) -> Result<(), McpBridgeError> {
    validate_name("instrument", name)?;
    if let Some(ch) = midi_channel {
        validate_midi_channel(ch)?;
    }
    if let Some(v) = volume {
        validate_range("volume", v, 0.0, 2.0)?;
    }
    if let Some(p) = pan {
        validate_range("pan", p, -1.0, 1.0)?;
    }
    if let Some(conns) = connections {
        validate_connection_indices(conns, modules.len())?;
    }
    Ok(())
}

/// Validate a `NoteInput` (used in batch note operations).
fn validate_note_input(n: &NoteInput) -> Result<(), McpBridgeError> {
    validate_note_fields(
        n.pitch,
        n.velocity.unwrap_or(100),
        n.start_beat,
        n.duration_beats,
    )
}

/// Validate an `AutomationPointInput` slice.
fn validate_automation_points_input(points: &[AutomationPointInput]) -> Result<(), McpBridgeError> {
    for pt in points {
        validate_automation_point(pt)?;
    }
    Ok(())
}

/// Format a validation error for string-returning tool handlers.
fn validation_err(e: McpBridgeError) -> String {
    format!("Error: {e}")
}

/// Convert a [`McpBridgeError`] into the MCP [`ErrorData`] type used by tool handlers.
fn mcp_err(e: McpBridgeError) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

// === MCP session tracking ===

/// Information about a connected MCP client.
#[derive(Debug, Clone)]
pub struct McpSessionInfo {
    /// Unique session ID (monotonically increasing).
    pub id: u64,
    /// Client name (e.g. "claude-code", "cursor").
    pub client_name: String,
    /// Client version string.
    pub client_version: String,
    /// MCP protocol version the client speaks.
    pub protocol_version: String,
}

/// Shared registry of active MCP sessions, visible to the GUI.
#[derive(Debug, Clone)]
pub struct McpSessionRegistry {
    next_id: Arc<AtomicU64>,
    sessions: Arc<Mutex<Vec<McpSessionInfo>>>,
    /// Fast atomic counter for GUI (avoids locking the mutex every frame).
    session_count: Arc<AtomicUsize>,
}

impl McpSessionRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: Arc::new(AtomicU64::new(1)),
            sessions: Arc::new(Mutex::new(Vec::new())),
            session_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Allocate a new session ID and register a placeholder (client info filled in later).
    fn register(&self) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.session_count.fetch_add(1, Ordering::Relaxed);
        id
    }

    /// Fill in the client info for a session after the initialize handshake.
    fn set_client_info(&self, id: u64, info: McpSessionInfo) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.push(info);
            let _ = id; // id already embedded in info
        }
    }

    /// Remove a session when the client disconnects.
    fn unregister(&self, id: u64) {
        self.session_count.fetch_sub(1, Ordering::Relaxed);
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.retain(|s| s.id != id);
        }
    }

    /// Number of active sessions (lock-free).
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.session_count.load(Ordering::Relaxed)
    }

    /// Snapshot of all active sessions.
    #[must_use]
    pub fn sessions(&self) -> Vec<McpSessionInfo> {
        self.sessions
            .lock()
            .map_or_else(|_| Vec::new(), |s| s.clone())
    }

    /// The raw atomic counter, for backward-compatible sharing.
    #[must_use]
    pub fn count_arc(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.session_count)
    }
}

impl Default for McpSessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// === Parameter structs for tool inputs ===

/// Empty parameter struct for tools that take no arguments.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoParams {}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentIdParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ModuleParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
    #[schemars(description = "Module ID string, e.g. 'osc-1', 'filter-1'")]
    pub module_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetParameterParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
    #[schemars(description = "Module ID string, e.g. 'osc-1'")]
    pub module_id: String,
    #[schemars(description = "Parameter name, e.g. 'frequency', 'resonance'")]
    pub param_name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetParameterParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
    #[schemars(description = "Module ID string, e.g. 'osc-1'")]
    pub module_id: String,
    #[schemars(description = "Parameter name, e.g. 'frequency', 'resonance'")]
    pub param_name: String,
    #[schemars(
        description = "New value in the parameter's native range (e.g. 20.0-20000.0 for cutoff in Hz). Use list_module_types or get_module_info to discover valid ranges."
    )]
    pub value: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOnParam {
    #[schemars(description = "MIDI note number (0-127, where 60 = middle C)")]
    pub note: u8,
    #[schemars(description = "Velocity (0-127, where 127 = maximum)")]
    pub velocity: u8,
    #[schemars(description = "MIDI channel (1-16, default 1)")]
    pub channel: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOffParam {
    #[schemars(description = "MIDI note number (0-127)")]
    pub note: u8,
    #[schemars(description = "MIDI channel (1-16, default 1)")]
    pub channel: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewNoteParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
    #[schemars(description = "MIDI note number (0-127, where 60 = middle C)")]
    pub note: u8,
    #[schemars(description = "Velocity (0-127, where 127 = maximum)")]
    pub velocity: u8,
    #[schemars(
        description = "Note duration in milliseconds (default 500). How long the note is held before release."
    )]
    pub duration_ms: Option<u32>,
    #[schemars(
        description = "Tail time in milliseconds after note-off (default 500). Extra time for release/reverb tails."
    )]
    pub tail_ms: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LoadExamplePatchParam {
    #[schemars(
        description = "Name of the example patch to load (case-insensitive), e.g. 'Acid Bass', 'Grand Piano'"
    )]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddModuleParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
    #[schemars(
        description = "Module type key from list_module_types, e.g. 'oscillator', 'filter', 'amplifier'"
    )]
    pub module_type: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
    #[schemars(description = "Source module ID, e.g. 'osc-1'")]
    pub from_module: String,
    #[schemars(description = "Source port name, e.g. 'output'")]
    pub from_port: String,
    #[schemars(description = "Destination module ID, e.g. 'flt-1'")]
    pub to_module: String,
    #[schemars(description = "Destination port name, e.g. 'input'")]
    pub to_port: String,
}

/// A single connection in a batch connect call.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectionInput {
    #[schemars(description = "Source module ID, e.g. 'osc-1'")]
    pub from_module: String,
    #[schemars(description = "Source port name, e.g. 'out'")]
    pub from_port: String,
    #[schemars(description = "Destination module ID, e.g. 'flt-1'")]
    pub to_module: String,
    #[schemars(description = "Destination port name, e.g. 'input'")]
    pub to_port: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectMultipleParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
    #[schemars(description = "Array of connections to make")]
    pub connections: Vec<ConnectionInput>,
}

// === Instrument lifecycle parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateInstrumentParam {
    #[schemars(description = "Name for the new instrument")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameInstrumentParam {
    #[schemars(description = "Instrument ID to rename")]
    pub instrument_id: u64,
    #[schemars(description = "New name for the instrument")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentVolumeParam {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: u64,
    #[schemars(description = "Volume level (0.0 = silent, 1.0 = unity, 2.0 = max)")]
    pub volume: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentPanParam {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: u64,
    #[schemars(description = "Pan position (-1.0 = left, 0.0 = center, 1.0 = right)")]
    pub pan: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentMuteParam {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: u64,
    #[schemars(description = "Whether the instrument should be muted")]
    pub muted: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentSoloParam {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: u64,
    #[schemars(description = "Whether the instrument should be soloed")]
    pub solo: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentMidiChannelParam {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: u64,
    #[schemars(description = "MIDI channel (1-16)")]
    pub channel: u8,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentEnabledParam {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: u64,
    #[schemars(description = "Whether the instrument should be enabled")]
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentCategoryParam {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: u64,
    #[schemars(description = "Category: Uncategorized, Drums, Bass, Pad, Lead, Arp, Keys, FX")]
    pub category: String,
}

// === Sequencer parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSongTempoParam {
    #[schemars(description = "Tempo in BPM (e.g. 120.0)")]
    pub bpm: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSongNameParam {
    #[schemars(description = "New song name")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreatePatternParam {
    #[schemars(description = "Pattern name")]
    pub name: String,
    #[schemars(description = "Pattern length in beats (e.g. 4.0 for one bar in 4/4)")]
    pub length_beats: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PatternIdParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddNoteParam {
    #[schemars(description = "Pattern ID to add the note to")]
    pub pattern_id: u32,
    #[schemars(description = "MIDI pitch (0-127, where 60 = middle C)")]
    pub pitch: u8,
    #[schemars(description = "Start position in beats (0.0 = beginning of pattern)")]
    pub start_beat: f32,
    #[schemars(description = "Duration in beats (1.0 = quarter note, 0.5 = eighth note)")]
    pub duration_beats: f32,
    #[schemars(description = "Velocity (0-127, where 127 = maximum)")]
    pub velocity: u8,
    #[schemars(
        description = "Instrument index (default 0). During playback, the track's instrument overrides this when set."
    )]
    pub instrument_id: Option<u16>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveNoteParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Note ID to remove")]
    pub note_id: u64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateNoteParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Note ID to update")]
    pub note_id: u64,
    #[schemars(description = "New MIDI pitch (0-127), or null to keep current")]
    pub pitch: Option<u8>,
    #[schemars(description = "New start position in beats, or null to keep current")]
    pub start_beat: Option<f32>,
    #[schemars(description = "New duration in beats, or null to keep current")]
    pub duration_beats: Option<f32>,
    #[schemars(description = "New velocity (0-127), or null to keep current")]
    pub velocity: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateTrackParam {
    #[schemars(description = "Track name")]
    pub name: String,
    #[schemars(description = "Instrument ID to assign (optional)")]
    pub instrument_id: Option<u16>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlacePatternParam {
    #[schemars(description = "Pattern ID to place")]
    pub pattern_id: u32,
    #[schemars(description = "Track ID to place on")]
    pub track_id: u16,
    #[schemars(description = "Start position in beats")]
    pub start_beat: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SeqSeekParam {
    #[schemars(description = "Beat position to seek to (0.0 = beginning)")]
    pub beat: f32,
}

// === Batch parameter structs ===

/// A note to add in a batch operation.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteInput {
    #[schemars(description = "MIDI pitch (0-127, where 60 = middle C)")]
    pub pitch: u8,
    #[schemars(description = "Start position in beats (0.0 = beginning of pattern)")]
    pub start_beat: f32,
    #[schemars(description = "Duration in beats (1.0 = quarter note, 0.5 = eighth note)")]
    pub duration_beats: f32,
    #[schemars(description = "Velocity (0-127). Default 100 if omitted.")]
    pub velocity: Option<u8>,
    #[schemars(
        description = "Instrument index (default 0). During playback, the track's instrument overrides this when set."
    )]
    pub instrument_id: Option<u16>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddNotesParam {
    #[schemars(description = "Pattern ID to add notes to")]
    pub pattern_id: u32,
    #[schemars(description = "Array of notes to add")]
    pub notes: Vec<NoteInput>,
}

/// A note update in a batch operation.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteUpdateInput {
    #[schemars(description = "Note ID to update")]
    pub note_id: u64,
    #[schemars(description = "New MIDI pitch (0-127), or null to keep current")]
    pub pitch: Option<u8>,
    #[schemars(description = "New start position in beats, or null to keep current")]
    pub start_beat: Option<f32>,
    #[schemars(description = "New duration in beats, or null to keep current")]
    pub duration_beats: Option<f32>,
    #[schemars(description = "New velocity (0-127), or null to keep current")]
    pub velocity: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateNotesParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Array of note updates")]
    pub updates: Vec<NoteUpdateInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReplaceNotesParam {
    #[schemars(description = "Pattern ID to replace notes in (clears existing notes first)")]
    pub pattern_id: u32,
    #[schemars(description = "Array of new notes to insert")]
    pub notes: Vec<NoteInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClearPatternParam {
    #[schemars(description = "Pattern ID to clear all notes from")]
    pub pattern_id: u32,
}

/// An automation point to add.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AutomationPointInput {
    #[schemars(
        description = "Parameter: Volume, Pan, FilterCutoff, FilterResonance, Attack, Decay, Sustain, Release"
    )]
    pub param: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<u16>,
    #[schemars(description = "Position in beats")]
    pub beat: f32,
    #[schemars(description = "Normalized value (0.0-1.0)")]
    pub value: f32,
    #[schemars(description = "Interpolation curve: Linear (default), Step, Exponential, SCurve")]
    pub curve: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddAutomationPointsParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Automation points to add")]
    pub points: Vec<AutomationPointInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetAutomationPointsParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Target parameter name (e.g. Volume, Pan, FilterCutoff)")]
    pub target: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<u16>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveAutomationPointsParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Target parameter name (e.g. Volume, Pan)")]
    pub target: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<u16>,
    #[schemars(description = "Beat positions of points to remove")]
    pub beats: Vec<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClearAutomationLaneParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Target parameter name (e.g. Volume, Pan)")]
    pub target: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<u16>,
}

// === Track control parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackVolumeParam {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(description = "Volume (0.0 = silent, 1.0 = full)")]
    pub volume: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackPanParam {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(description = "Pan position (-1.0 = left, 0.0 = center, 1.0 = right)")]
    pub pan: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackMuteParam {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(description = "Whether the track should be muted")]
    pub muted: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackSoloParam {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(description = "Whether the track should be soloed")]
    pub solo: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackInstrumentParam {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(
        description = "Instrument ID to assign to this track. Omit or set to null to unassign."
    )]
    pub instrument_id: Option<u16>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameTrackParam {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(description = "New name for the track")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteTrackParam {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
}

// === Pattern management parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenamePatternParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "New name for the pattern")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatternLengthParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "New length in beats (e.g. 4.0 for one bar in 4/4)")]
    pub length_beats: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DuplicatePatternParam {
    #[schemars(description = "Pattern ID to duplicate")]
    pub pattern_id: u32,
}

// === Song metadata parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSongAuthorParam {
    #[schemars(description = "Author name")]
    pub author: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSongTimeSignatureParam {
    #[schemars(description = "Numerator (beats per bar, e.g. 4)")]
    pub numerator: u8,
    #[schemars(description = "Denominator (beat unit, e.g. 4 for quarter note)")]
    pub denominator: u8,
}

// === Batch parameter set structs ===

/// A parameter to set in a batch operation.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ParamSetInput {
    #[schemars(description = "Module ID (e.g. 'osc-1')")]
    pub module_id: String,
    #[schemars(description = "Parameter name (e.g. 'frequency', 'level')")]
    pub param_name: String,
    #[schemars(
        description = "New value in the parameter's native range (e.g. 440.0 for frequency in Hz). Use list_module_types for valid ranges."
    )]
    pub value: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetParametersParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: u64,
    #[schemars(description = "Array of parameters to set")]
    pub params: Vec<ParamSetInput>,
}

/// A pattern to create in a batch operation.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PatternInput {
    #[schemars(description = "Pattern name")]
    pub name: String,
    #[schemars(description = "Pattern length in beats (e.g. 4.0 for one bar in 4/4)")]
    pub length_beats: f32,
    #[schemars(description = "Optional array of notes to add immediately")]
    pub notes: Option<Vec<NoteInput>>,
    #[schemars(description = "Optional array of automation points to add")]
    pub automation: Option<Vec<AutomationPointInput>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreatePatternsParam {
    #[schemars(description = "Array of patterns to create")]
    pub patterns: Vec<PatternInput>,
}

/// A track to create in a batch operation.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackInput {
    #[schemars(description = "Track name")]
    pub name: String,
    #[schemars(description = "Instrument ID to assign (optional)")]
    pub instrument_id: Option<u16>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateTracksParam {
    #[schemars(description = "Array of tracks to create")]
    pub tracks: Vec<TrackInput>,
}

/// A pattern placement in the arrangement.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlacementInput {
    #[schemars(description = "Pattern ID to place")]
    pub pattern_id: u32,
    #[schemars(description = "Track ID to place on")]
    pub track_id: u16,
    #[schemars(description = "Start position in beats")]
    pub start_beat: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlacePatternsParam {
    #[schemars(description = "Array of placements to create")]
    pub placements: Vec<PlacementInput>,
}

/// Pattern definition for set_song.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SongPatternDef {
    #[schemars(description = "Pattern name")]
    pub name: String,
    #[schemars(description = "Pattern length in beats")]
    pub length_beats: f32,
    #[schemars(description = "Notes in this pattern")]
    pub notes: Vec<NoteInput>,
    #[schemars(description = "Optional automation points for this pattern")]
    pub automation: Option<Vec<AutomationPointInput>>,
}

/// Track definition for set_song.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SongTrackDef {
    #[schemars(description = "Track name")]
    pub name: String,
    #[schemars(description = "Instrument ID to assign (optional)")]
    pub instrument_id: Option<u16>,
}

/// Placement definition for set_song (uses array indices, not IDs).
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SongPlacementDef {
    #[schemars(
        description = "Index into the patterns array (0-based, refers to the pattern at this position)"
    )]
    pub pattern_index: usize,
    #[schemars(
        description = "Index into the tracks array (0-based, refers to the track at this position)"
    )]
    pub track_index: usize,
    #[schemars(description = "Start position in beats")]
    pub start_beat: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSongParam {
    #[schemars(description = "Song name")]
    pub name: String,
    #[schemars(description = "Tempo in BPM (default 120)")]
    pub tempo: Option<f32>,
    #[schemars(description = "Patterns to create (with their notes)")]
    pub patterns: Vec<SongPatternDef>,
    #[schemars(description = "Tracks to create")]
    pub tracks: Vec<SongTrackDef>,
    #[schemars(
        description = "Arrangement: place patterns on tracks. Uses array indices (not IDs) for pattern_index and track_index."
    )]
    pub placements: Vec<SongPlacementDef>,
}

// === Batch instrument building parameter structs ===

/// A parameter value: number, string choice, or boolean.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ParamValueInput {
    /// Numeric value (e.g. 440.0 for frequency).
    Number(f64),
    /// Boolean value.
    Bool(bool),
    /// String choice (e.g. "sawtooth" for waveform).
    Choice(String),
}

/// A module to create with optional parameters.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ModuleDefInput {
    #[schemars(
        description = "Module type key (prefix from list_module_types, e.g. 'osc', 'flt', 'amp', 'out', 'env', 'lfo', 'dly', 'rev')"
    )]
    pub module_type: String,
    #[schemars(
        description = "Parameters as {name: value}. Values can be numbers (440.0), strings for choices ('sawtooth'), or booleans. Use get_module_info to discover parameter names."
    )]
    pub params: Option<std::collections::BTreeMap<String, ParamValueInput>>,
}

/// A connection between modules using array indices.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectionDefInput {
    #[schemars(description = "Index of the source module in the modules array (0-based)")]
    pub from: usize,
    #[schemars(description = "Source port name (e.g. 'output', 'out')")]
    pub from_port: String,
    #[schemars(description = "Index of the destination module in the modules array (0-based)")]
    pub to: usize,
    #[schemars(description = "Destination port name (e.g. 'input', 'in', 'cutoff_mod')")]
    pub to_port: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BuildInstrumentParam {
    #[schemars(
        description = "Existing instrument ID to update. If provided, clears the instrument's graph and rebuilds it. If omitted, creates a new instrument."
    )]
    pub instrument_id: Option<u64>,
    #[schemars(description = "Instrument name")]
    pub name: String,
    #[schemars(description = "MIDI channel (1-16, optional)")]
    pub midi_channel: Option<u8>,
    #[schemars(description = "Volume (0.0-2.0, optional, default 1.0)")]
    pub volume: Option<f32>,
    #[schemars(description = "Pan (-1.0 to 1.0, optional, default 0.0)")]
    pub pan: Option<f32>,
    #[schemars(
        description = "Modules to create. Order matters — connections reference modules by array index."
    )]
    pub modules: Vec<ModuleDefInput>,
    #[schemars(
        description = "Connections between modules. Use 'from'/'to' as 0-based indices into the modules array."
    )]
    pub connections: Option<Vec<ConnectionDefInput>>,
}

/// Single instrument definition for batch build.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentDefInput {
    #[schemars(
        description = "Existing instrument ID to update. If omitted, creates a new instrument."
    )]
    pub instrument_id: Option<u64>,
    #[schemars(description = "Instrument name")]
    pub name: String,
    #[schemars(description = "MIDI channel (1-16, optional)")]
    pub midi_channel: Option<u8>,
    #[schemars(description = "Volume (0.0-2.0, optional)")]
    pub volume: Option<f32>,
    #[schemars(description = "Pan (-1.0 to 1.0, optional)")]
    pub pan: Option<f32>,
    #[schemars(description = "Modules to create")]
    pub modules: Vec<ModuleDefInput>,
    #[schemars(description = "Connections between modules (array indices)")]
    pub connections: Option<Vec<ConnectionDefInput>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BuildInstrumentsParam {
    #[schemars(description = "Array of instruments to create")]
    pub instruments: Vec<InstrumentDefInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ApplyExamplePatchParam {
    #[schemars(
        description = "Instrument ID to apply the patch to. If omitted, creates a new instrument."
    )]
    pub instrument_id: Option<u64>,
    #[schemars(
        description = "Name of the example patch (case-insensitive). Use list_example_patches to see available patches."
    )]
    pub patch_name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectPathParam {
    #[schemars(description = "Absolute file path for the project (.json)")]
    pub path: String,
}

// === AWE parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetAweEnabledParam {
    #[schemars(description = "Whether to enable AWE (Acoustic World Engine) room simulation")]
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetAweParameterParam {
    #[schemars(
        description = "AWE parameter name. Valid names: dry_wet (0-1, wet/dry mix), \
                       early_late_balance (0-1, early vs late reflections), \
                       modes_amount (0-1, room mode resonance strength), \
                       freq_warp (-1 to 1, shifts room mode frequencies), \
                       resonance_boost (0-1, emphasize room modes), \
                       tail_stretch (0.5-4.0, reverb tail length multiplier), \
                       portal_amount (0-1, acoustic portal effect), \
                       pre_delay (0-200, milliseconds before first reflection), \
                       modulation_depth (0-1, FDN chorus depth), \
                       modulation_rate (0.01-20.0, FDN chorus rate in Hz), \
                       air_absorption (0-1, high-frequency damping over distance), \
                       width (0-1, stereo width: 0=mono, 1=full stereo), \
                       high_cut (200-20000, high-cut frequency in Hz), \
                       low_cut (20-2000, low-cut frequency in Hz), \
                       temperature (-40 to 60, Celsius, affects speed of sound), \
                       source_x (meters, sound source X position in room), \
                       source_y (meters, sound source Y position in room), \
                       listener_x (meters, listener X position in room), \
                       listener_y (meters, listener Y position in room)"
    )]
    pub name: String,
    #[schemars(
        description = "New value for the parameter (range depends on parameter, see name description)"
    )]
    pub value: f64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetAweRoomShapeParam {
    #[schemars(
        description = "Room shape type: 'Box' (rectangular room), 'Cylinder' (tunnel/pipeline), \
                       'LShape' (two connected rectangles), 'Sphere' (spherical room), \
                       'Dome' (half-sphere), 'Tube' (open-ended cylinder, no end reflections)"
    )]
    pub shape: String,
    #[schemars(description = "Room dimensions in meters (depends on shape): \
                       Box: [length, width, height] (e.g. [8.0, 5.0, 3.0]). \
                       Cylinder: [radius, length] (e.g. [1.0, 20.0]). \
                       LShape: [length_a, width_a, length_b, width_b, height] (e.g. [8.0, 5.0, 6.0, 4.0, 3.0]). \
                       Sphere: [radius] (e.g. [5.0]). \
                       Dome: [radius] (e.g. [6.0]). \
                       Tube: [radius, length] (e.g. [1.5, 30.0]).")]
    pub dimensions: Vec<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetAweMaterialParam {
    #[schemars(description = "Wall material name. Available materials: \
                       Concrete (hard, dark, cold — low absorption), \
                       Wood (warm, balanced — medium absorption), \
                       Glass (bright, thin bass — reflects highs, absorbs lows), \
                       Metal (ultra-bright, ringing — minimal absorption), \
                       Fabric (very dark — high absorption, especially highs), \
                       Tile (hard, bright, clinical — like bathroom tiles), \
                       Marble (warmer hard surface — moderate absorption), \
                       Ice (crisp, noticeable HF absorption), \
                       Carpet (dead, absorbs everything — very high absorption), \
                       Water (murmuring, medium-dark — moderate absorption), \
                       Void (perfectly reflective — zero absorption, infinite reverb), \
                       Prism (extreme HF absorption with high diffusion), \
                       Plasma (strong LF damping with bright tail), \
                       Membrane (absorbs lows more than highs — non-physical), \
                       Nanogel (ultra-absorbent but highly diffusive)")]
    pub material: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetAwePresetParam {
    #[schemars(
        description = "AWE preset name (case-insensitive). Use list_awe_presets to see all available presets. \
                       Examples: 'Cathedral', 'Bathroom', 'Cave', 'Concert Hall', 'Dream', 'Small Studio'"
    )]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetAweLfoParam {
    #[schemars(description = "LFO index (1-4). AWE has 4 internal LFOs for parameter modulation.")]
    pub index: u8,
    #[schemars(
        description = "LFO rate in Hz (0.01-20.0). Lower = slow sweep, higher = vibrato-like."
    )]
    pub rate: f32,
    #[schemars(description = "Modulation amount (0.0-1.0). 0 = no modulation, 1 = maximum.")]
    pub amount: f32,
    #[schemars(
        description = "Modulation target: RoomLength, RoomWidth, SourceX, SourceY, ListenerX, \
                       ListenerY, DryWet, FreqWarp, EarlyLate, ModesAmount, ResonanceBoost, \
                       TailStretch, PortalAmount, PreDelay, ModulationDepth, ModulationRate, \
                       AirAbsorption, Width, HighCut, LowCut, Temperature"
    )]
    pub target: String,
}

// === MCP Server ===

/// The MCP server that wraps a SynthBridge implementation.
#[derive(Clone)]
pub struct SynthMcpServer {
    bridge: Arc<dyn SynthBridge>,
    tool_router: ToolRouter<Self>,
    /// Session registry shared with the GUI.
    registry: Option<McpSessionRegistry>,
    /// This session's unique ID within the registry.
    session_id: u64,
}

impl SynthMcpServer {
    /// Create a new MCP server backed by the given bridge (no session tracking).
    pub fn new(bridge: Arc<dyn SynthBridge>) -> Self {
        Self {
            bridge,
            tool_router: Self::tool_router(),
            registry: None,
            session_id: 0,
        }
    }

    /// Create a new MCP server with session tracking via a shared registry.
    pub fn with_registry(bridge: Arc<dyn SynthBridge>, registry: McpSessionRegistry) -> Self {
        let session_id = registry.register();
        Self {
            bridge,
            tool_router: Self::tool_router(),
            registry: Some(registry),
            session_id,
        }
    }
}

impl Drop for SynthMcpServer {
    fn drop(&mut self) {
        if let Some(registry) = &self.registry {
            registry.unregister(self.session_id);
        }
    }
}

#[tool_handler]
impl ServerHandler for SynthMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(
            Implementation::new("pertylizer", env!("CARGO_PKG_VERSION"))
                .with_title("Pertylizer")
                .with_description(
                    "Modular synthesizer with 35 voice modules, 21 effects, and MCP integration",
                )
                .with_website_url("https://github.com/pertyjons/pertylizer"),
        )
        .with_instructions(
            "Pertylizer MCP server. Inspect and control the running synth: \
             list modules, read parameters, change settings, play notes.\n\n\
             ## Architecture\n\
             Pertylizer is a modular synthesizer. Each **instrument** has a voice graph \
             (oscillators, filters, envelopes, amplifiers, mixers) and an effect chain \
             (delay, reverb, chorus, etc.).\n\n\
             ## Typical voice signal flow\n\
             `oscillator → filter → amplifier → output`\n\
             Envelope → amplifier cv_gain (volume shaping), Envelope → filter cutoff_cv (filter sweep).\n\
             LFO → any cv input for modulation.\n\n\
             ## Building instruments\n\
             Use `build_instrument` for one-call instrument creation, or step-by-step: \
             `create_instrument` → `add_module` (multiple) → `set_parameter` → `connect`.\n\
             Call `list_module_types` to discover available modules with parameter ranges, \
             units, port types, and signal flow hints.\n\n\
             ## Sequencer\n\
             Songs have **tracks** and **patterns**. Patterns contain notes and automation. \
             Patterns are placed on tracks in the **arrangement** timeline. \
             Use `create_pattern` → `add_notes` → `create_track` → `place_pattern` to build songs.\n\n\
             ## Batch operations\n\
             Prefer batch tools (`build_instrument`, `add_notes`, `connect_multiple`, \
             `create_patterns`, `place_patterns`) over repeated single calls for efficiency.\n\n\
             ## AWE (Acoustic World Engine)\n\
             AWE is a physics-based room simulation applied to the master output. It models \
             early reflections, late reverb (FDN), room modes (standing waves), and stereo \
             spatialisation based on room geometry, wall material, and source/listener positions.\n\n\
             **Typical workflow:**\n\
             1. `list_awe_presets` → browse available room presets\n\
             2. `set_awe_preset` → load a preset (also enables AWE)\n\
             3. `set_awe_parameter` → fine-tune individual parameters (dry_wet, tail_stretch, etc.)\n\
             4. `set_awe_room_shape` / `set_awe_material` → change room geometry or wall material\n\
             5. `set_awe_lfo` → add animated modulation (e.g. slowly sweep source position)\n\n\
             **Key concepts:** Room shape determines reflection patterns and standing waves. \
             Material controls frequency-dependent absorption (Metal = bright, Carpet = dark). \
             Source/listener positions affect early reflection timing and stereo image. \
             4 internal LFOs can modulate any parameter for evolving, animated spaces.\n\
             AWE must be enabled (`set_awe_enabled`) before you can hear its effect. \
             Use `get_awe_state` to inspect the current configuration at any time.",
        )
    }

    #[allow(clippy::unused_async)]
    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if let Some(registry) = &self.registry {
            let (client_name, client_version, protocol_version) =
                if let Some(peer_info) = context.peer.peer_info() {
                    (
                        peer_info.client_info.name.clone(),
                        peer_info.client_info.version.clone(),
                        peer_info.protocol_version.as_str().to_owned(),
                    )
                } else {
                    ("unknown".to_owned(), String::new(), String::new())
                };
            registry.set_client_info(
                self.session_id,
                McpSessionInfo {
                    id: self.session_id,
                    client_name,
                    client_version,
                    protocol_version,
                },
            );
        }
    }

    #[allow(clippy::unused_async)]
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut resources: Vec<Annotated<RawResource>> = Vec::new();

        // Module type resources
        if let Ok(types) = self.bridge.list_module_types() {
            for info in &types {
                let mut r = RawResource::new(
                    format!("synth://module-types/{}", info.type_key),
                    info.name.clone(),
                );
                r.description = Some(format!(
                    "Module type: {} | Ports: {} in, {} out | Params: {}",
                    info.category,
                    info.input_ports.len(),
                    info.output_ports.len(),
                    info.parameters.len()
                ));
                r.mime_type = Some("application/json".into());
                resources.push(Annotated::new(r, None));
            }
        }

        // Example patch resources
        if let Ok(patches) = self.bridge.list_example_patches() {
            for patch in &patches {
                let slug = patch.name.to_ascii_lowercase().replace(' ', "-");
                let mut r = RawResource::new(format!("synth://patches/{slug}"), patch.name.clone());
                r.description = Some(format!(
                    "{}: {} | {} modules, {} connections",
                    patch.category, patch.description, patch.module_count, patch.connection_count
                ));
                r.mime_type = Some("application/json".into());
                resources.push(Annotated::new(r, None));
            }
        }

        Ok(ListResourcesResult::with_all_items(resources))
    }

    #[allow(clippy::unused_async)]
    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let templates = vec![
            Annotated::new(
                RawResourceTemplate {
                    uri_template: "synth://module-types/{type_key}".into(),
                    name: "Module Type".into(),
                    title: None,
                    description: Some(
                        "Detailed info about a synth module type (ports, parameters)".into(),
                    ),
                    mime_type: Some("application/json".into()),
                    icons: None,
                },
                None,
            ),
            Annotated::new(
                RawResourceTemplate {
                    uri_template: "synth://patches/{name}".into(),
                    name: "Example Patch".into(),
                    title: None,
                    description: Some(
                        "Full patch data (modules, connections, parameters) for an example patch"
                            .into(),
                    ),
                    mime_type: Some("application/json".into()),
                    icons: None,
                },
                None,
            ),
        ];

        Ok(ListResourceTemplatesResult::with_all_items(templates))
    }

    #[allow(clippy::unused_async)]
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, ErrorData> {
        let uri = &request.uri;

        if let Some(type_key) = uri.strip_prefix("synth://module-types/") {
            // Look up module type
            let types = self
                .bridge
                .list_module_types()
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

            let info = types
                .iter()
                .find(|t| t.type_key == type_key)
                .ok_or_else(|| {
                    ErrorData::resource_not_found(
                        format!("Module type '{type_key}' not found"),
                        None,
                    )
                })?;

            let json = to_json(info);
            Ok(ReadResourceResult::new(vec![ResourceContents::text(
                json,
                uri.clone(),
            )]))
        } else if let Some(slug) = uri.strip_prefix("synth://patches/") {
            // Look up patch by slug — match by converting name to slug
            let patches = self
                .bridge
                .list_example_patches()
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

            let patch_name = patches
                .iter()
                .find(|p| p.name.to_ascii_lowercase().replace(' ', "-") == slug)
                .map(|p| p.name.clone())
                .ok_or_else(|| {
                    ErrorData::resource_not_found(format!("Patch '{slug}' not found"), None)
                })?;

            let data = self
                .bridge
                .get_example_patch(&patch_name)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

            let json = to_json(&data);
            Ok(ReadResourceResult::new(vec![ResourceContents::text(
                json,
                uri.clone(),
            )]))
        } else {
            Err(ErrorData::resource_not_found(
                format!("Unknown resource URI: {uri}"),
                None,
            ))
        }
    }
}

#[tool_router]
impl SynthMcpServer {
    #[tool(
        description = "List all instruments with ID, name, category, volume, pan, mute/solo state, and module/effect counts."
    )]
    async fn list_instruments(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_instruments() {
            Ok(instruments) => to_json(&instruments),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get detailed information about a specific instrument including module count and effects"
    )]
    async fn get_instrument_info(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_instrument_info(params.0.instrument_id) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all modules in an instrument's voice graph with their types and names"
    )]
    async fn list_modules(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.list_modules(params.0.instrument_id) {
            Ok(modules) => to_json(&modules),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get detailed info for a specific module including all parameters and port connections"
    )]
    async fn get_module_info(&self, params: Parameters<ModuleParam>) -> String {
        match self
            .bridge
            .get_module_info(params.0.instrument_id, &params.0.module_id)
        {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get all connections (cables) between modules in the voice graph. Returns from_module:from_port → to_module:to_port pairs."
    )]
    async fn get_connections(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_connections(params.0.instrument_id) {
            Ok(conns) => to_json(&conns),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get the current value of a specific module parameter. Returns name, raw value, formatted display string (e.g. '440 Hz'), and min/max/default range."
    )]
    async fn get_parameter(&self, params: Parameters<GetParameterParam>) -> String {
        match self.bridge.get_parameter(
            params.0.instrument_id,
            &params.0.module_id,
            &params.0.param_name,
        ) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get engine status: CPU usage (0.0-1.0), active voice count, peak/RMS meters (dB), sample rate, tempo, and whether sequencer is playing."
    )]
    async fn get_engine_status(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_engine_status() {
            Ok(status) => to_json(&status),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Run diagnostics on the module graph to find issues like disconnected modules or missing connections"
    )]
    async fn get_graph_diagnostics(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_graph_diagnostics(params.0.instrument_id) {
            Ok(diags) => to_json(&diags),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set a module parameter to a new value. Returns the parameter info with the actual value set (may differ from requested due to clamping). Use list_modules and get_module_info to discover available parameters."
    )]
    async fn set_parameter(&self, params: Parameters<SetParameterParam>) -> String {
        if params.0.value.is_nan() {
            return format!(
                "Error: {}",
                McpBridgeError::ValueOutOfRange {
                    name: "value",
                    value: params.0.value,
                    min: f32::NEG_INFINITY,
                    max: f32::INFINITY,
                }
            );
        }
        match self.bridge.set_parameter(
            params.0.instrument_id,
            &params.0.module_id,
            &params.0.param_name,
            params.0.value,
        ) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Play a MIDI note (note on). Use note=60 for middle C, velocity=100 for moderate strength."
    )]
    async fn note_on(&self, params: Parameters<NoteOnParam>) -> String {
        if let Err(e) = validate_midi_note(params.0.note) {
            return format!("Error: {e}");
        }
        if let Err(e) = validate_velocity(params.0.velocity) {
            return format!("Error: {e}");
        }
        let channel = params.0.channel.unwrap_or(1);
        if let Err(e) = validate_midi_channel(channel) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .note_on(params.0.note, params.0.velocity, channel)
        {
            Ok(()) => format!(
                "Note {} on (vel={}, ch={})",
                params.0.note, params.0.velocity, channel
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Stop a MIDI note (note off). Use the same note number as the corresponding note_on."
    )]
    async fn note_off(&self, params: Parameters<NoteOffParam>) -> String {
        if let Err(e) = validate_midi_note(params.0.note) {
            return format!("Error: {e}");
        }
        let channel = params.0.channel.unwrap_or(1);
        if let Err(e) = validate_midi_channel(channel) {
            return format!("Error: {e}");
        }
        match self.bridge.note_off(params.0.note, channel) {
            Ok(()) => format!("Note {} off (ch={})", params.0.note, channel),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Render an audio preview of a note played on an instrument. Returns a WAV audio clip of the instrument's current sound. Useful for hearing what a patch sounds like after making changes."
    )]
    async fn preview_note(
        &self,
        params: Parameters<PreviewNoteParam>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_midi_note(params.0.note).map_err(mcp_err)?;
        validate_velocity(params.0.velocity).map_err(mcp_err)?;
        let duration_ms = params.0.duration_ms.unwrap_or(500);
        let tail_ms = params.0.tail_ms.unwrap_or(500);
        #[expect(clippy::cast_precision_loss, reason = "millisecond values fit in f32")]
        validate_range("duration_ms", duration_ms as f32, 1.0, 30000.0).map_err(mcp_err)?;
        #[expect(clippy::cast_precision_loss, reason = "millisecond values fit in f32")]
        validate_range("tail_ms", tail_ms as f32, 1.0, 30000.0).map_err(mcp_err)?;

        let preview = self
            .bridge
            .render_note_preview(
                params.0.instrument_id,
                params.0.note,
                params.0.velocity,
                duration_ms,
                tail_ms,
            )
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&preview.wav_data);

        let audio = RawContent::Audio(RawAudioContent {
            data: encoded,
            mime_type: "audio/wav".to_string(),
        })
        .no_annotation();

        let text = Content::text(format!(
            "Audio preview: note {} vel {} on instrument {} ({:.1}s, {}Hz WAV, {} bytes)",
            params.0.note,
            params.0.velocity,
            params.0.instrument_id,
            preview.duration_seconds,
            preview.sample_rate,
            preview.wav_data.len(),
        ));

        Ok(CallToolResult::success(vec![text, audio]))
    }

    #[tool(
        description = "List all available example patches with their categories, descriptions, and tags"
    )]
    async fn list_example_patches(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_example_patches() {
            Ok(patches) => to_json(&patches),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Load an example patch by name. The GUI will update on the next frame. Use list_example_patches to see available patches."
    )]
    async fn load_example_patch(&self, params: Parameters<LoadExamplePatchParam>) -> String {
        match self.bridge.load_example_patch(&params.0.name) {
            Ok(msg) => msg,
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Request auto-layout of modules in the patch view. The GUI applies the layout on the next frame, arranging modules by signal flow."
    )]
    async fn auto_layout(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.request_auto_layout() {
            Ok(msg) => msg,
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get a snapshot of the current UI layout: module positions, sizes, connections, and overlap analysis for debugging"
    )]
    async fn get_ui_snapshot(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_ui_snapshot(params.0.instrument_id) {
            Ok(snapshot) => to_json(&snapshot),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all available module types with their ports and parameters. Use the type_key to add modules with add_module."
    )]
    async fn list_module_types(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_module_types() {
            Ok(types) => to_json(&types),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add a new module to the instrument's voice graph. The module appears in the GUI on the next frame. Use list_modules to discover the assigned module ID."
    )]
    async fn add_module(&self, params: Parameters<AddModuleParam>) -> String {
        match self
            .bridge
            .add_module(params.0.instrument_id, &params.0.module_type)
        {
            Ok(msg) => msg,
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Remove a module from the instrument's voice graph and disconnect all its cables."
    )]
    async fn remove_module(&self, params: Parameters<ModuleParam>) -> String {
        match self
            .bridge
            .remove_module(params.0.instrument_id, &params.0.module_id)
        {
            Ok(()) => format!("OK: removed {}", params.0.module_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Connect two module ports with a cable. Use list_modules or get_module_info to discover port names."
    )]
    async fn connect(&self, params: Parameters<ConnectParam>) -> String {
        match self.bridge.connect(
            params.0.instrument_id,
            &params.0.from_module,
            &params.0.from_port,
            &params.0.to_module,
            &params.0.to_port,
        ) {
            Ok(()) => format!(
                "OK: connected {}:{} → {}:{}",
                params.0.from_module, params.0.from_port, params.0.to_module, params.0.to_port
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Connect multiple module ports in one call. Returns the number of successful connections and any errors. \
                       Each connection specifies from_module:from_port → to_module:to_port."
    )]
    async fn connect_multiple(&self, params: Parameters<ConnectMultipleParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for c in &params.0.connections {
            match self.bridge.connect(
                params.0.instrument_id,
                &c.from_module,
                &c.from_port,
                &c.to_module,
                &c.to_port,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "{}:{} → {}:{}: {e}",
                    c.from_module, c.from_port, c.to_module, c.to_port
                )),
            }
        }
        if errors.is_empty() {
            format!("OK: {ok_count} connections made")
        } else {
            format!(
                "OK: {ok_count} connections made, {} errors: {}",
                errors.len(),
                errors.join("; ")
            )
        }
    }

    #[tool(
        description = "Clear the entire voice graph for an instrument, removing all modules and connections. Use this to start from scratch."
    )]
    async fn clear_graph(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.clear_graph(params.0.instrument_id) {
            Ok(()) => "OK: graph cleared".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Instrument lifecycle ===

    #[tool(
        description = "Create a new instrument. Returns the instrument info with its assigned ID."
    )]
    async fn create_instrument(&self, params: Parameters<CreateInstrumentParam>) -> String {
        if let Err(e) = validate_name("instrument", &params.0.name) {
            return format!("Error: {e}");
        }
        match self.bridge.create_instrument(&params.0.name) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Delete an instrument and all its modules. Cannot delete the default instrument (ID 0)."
    )]
    async fn delete_instrument(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.delete_instrument(params.0.instrument_id) {
            Ok(()) => format!("OK: deleted instrument {}", params.0.instrument_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Rename an instrument. The name is shown in the UI instrument strip and track selector."
    )]
    async fn rename_instrument(&self, params: Parameters<RenameInstrumentParam>) -> String {
        if let Err(e) = validate_name("instrument", &params.0.name) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .rename_instrument(params.0.instrument_id, &params.0.name)
        {
            Ok(()) => format!(
                "OK: renamed instrument {} to '{}'",
                params.0.instrument_id, params.0.name
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the volume of an instrument (0.0 = silent, 1.0 = unity gain, 2.0 = max)."
    )]
    async fn set_instrument_volume(&self, params: Parameters<SetInstrumentVolumeParam>) -> String {
        if let Err(e) = validate_range("volume", params.0.volume, 0.0, 2.0) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .set_instrument_volume(params.0.instrument_id, params.0.volume)
        {
            Ok(()) => format!(
                "OK: instrument {} volume set to {}",
                params.0.instrument_id, params.0.volume
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the pan position of an instrument (-1.0 = left, 0.0 = center, 1.0 = right)."
    )]
    async fn set_instrument_pan(&self, params: Parameters<SetInstrumentPanParam>) -> String {
        if let Err(e) = validate_range("pan", params.0.pan, -1.0, 1.0) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .set_instrument_pan(params.0.instrument_id, params.0.pan)
        {
            Ok(()) => format!(
                "OK: instrument {} pan set to {}",
                params.0.instrument_id, params.0.pan
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Mute or unmute an instrument.")]
    async fn set_instrument_mute(&self, params: Parameters<SetInstrumentMuteParam>) -> String {
        match self
            .bridge
            .set_instrument_mute(params.0.instrument_id, params.0.muted)
        {
            Ok(()) => {
                let state = if params.0.muted { "muted" } else { "unmuted" };
                format!("OK: instrument {} {state}", params.0.instrument_id)
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Solo or unsolo an instrument. When any instrument is soloed, only soloed instruments produce sound."
    )]
    async fn set_instrument_solo(&self, params: Parameters<SetInstrumentSoloParam>) -> String {
        match self
            .bridge
            .set_instrument_solo(params.0.instrument_id, params.0.solo)
        {
            Ok(()) => {
                let state = if params.0.solo { "soloed" } else { "unsoloed" };
                format!("OK: instrument {} {state}", params.0.instrument_id)
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Set the MIDI channel for an instrument (1-16).")]
    async fn set_instrument_midi_channel(
        &self,
        params: Parameters<SetInstrumentMidiChannelParam>,
    ) -> String {
        if let Err(e) = validate_midi_channel(params.0.channel) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .set_instrument_midi_channel(params.0.instrument_id, params.0.channel)
        {
            Ok(()) => format!(
                "OK: instrument {} MIDI channel set to {}",
                params.0.instrument_id, params.0.channel
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Enable or disable an instrument. Disabled instruments skip all audio processing (lighter than mute which still processes but silences output)."
    )]
    async fn set_instrument_enabled(
        &self,
        params: Parameters<SetInstrumentEnabledParam>,
    ) -> String {
        match self
            .bridge
            .set_instrument_enabled(params.0.instrument_id, params.0.enabled)
        {
            Ok(()) => {
                let state = if params.0.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                format!("OK: instrument {} {state}", params.0.instrument_id)
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the category of an instrument (for visualization routing). Categories: Uncategorized, Drums, Bass, Pad, Lead, Arp, Keys, FX."
    )]
    async fn set_instrument_category(
        &self,
        params: Parameters<SetInstrumentCategoryParam>,
    ) -> String {
        match self
            .bridge
            .set_instrument_category(params.0.instrument_id, &params.0.category)
        {
            Ok(()) => format!(
                "OK: instrument {} category set to {}",
                params.0.instrument_id, params.0.category
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Disconnect a cable between two module ports. Uses same from/to parameters as connect."
    )]
    async fn disconnect(&self, params: Parameters<ConnectParam>) -> String {
        match self.bridge.disconnect(
            params.0.instrument_id,
            &params.0.from_module,
            &params.0.from_port,
            &params.0.to_module,
            &params.0.to_port,
        ) {
            Ok(()) => format!(
                "OK: disconnected {}:{} → {}:{}",
                params.0.from_module, params.0.from_port, params.0.to_module, params.0.to_port
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Song ===

    #[tool(
        description = "Get song info: name, author, tempo, time signature, length, pattern/track counts"
    )]
    async fn get_song_info(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_song_info() {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the song tempo in BPM (typically 60-200, e.g. 120.0 for standard pop tempo)."
    )]
    async fn set_song_tempo(&self, params: Parameters<SetSongTempoParam>) -> String {
        if let Err(e) = validate_range("tempo", params.0.bpm, 20.0, 999.0) {
            return format!("Error: {e}");
        }
        match self.bridge.set_song_tempo(params.0.bpm) {
            Ok(()) => format!("OK: tempo set to {} BPM", params.0.bpm),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the song name. Shown in the transport bar and saved with the project."
    )]
    async fn set_song_name(&self, params: Parameters<SetSongNameParam>) -> String {
        if let Err(e) = validate_name("song", &params.0.name) {
            return format!("Error: {e}");
        }
        match self.bridge.set_song_name(&params.0.name) {
            Ok(()) => format!("OK: song name set to '{}'", params.0.name),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Patterns ===

    #[tool(
        description = "List all patterns in the song with their names, lengths, and note counts"
    )]
    async fn list_patterns(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_patterns() {
            Ok(patterns) => to_json(&patterns),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create a new pattern. length_beats: 4.0 = one bar in 4/4. Returns the pattern ID."
    )]
    async fn create_pattern(&self, params: Parameters<CreatePatternParam>) -> String {
        if let Err(e) = validate_name("pattern", &params.0.name) {
            return format!("Error: {e}");
        }
        if let Err(e) = validate_range("length_beats", params.0.length_beats, 0.001, 1024.0) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .create_pattern(&params.0.name, params.0.length_beats)
        {
            Ok(id) => format!("OK: created pattern {id} '{}'", params.0.name),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Delete a pattern by ID. Also removes all placements of this pattern.")]
    async fn delete_pattern(&self, params: Parameters<PatternIdParam>) -> String {
        match self.bridge.delete_pattern(params.0.pattern_id) {
            Ok(()) => format!("OK: deleted pattern {}", params.0.pattern_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Notes ===

    #[tool(
        description = "List all notes in a pattern. Returns note ID, MIDI pitch (0-127), pitch name (e.g. 'C4'), start/duration in beats, and velocity (0-127)."
    )]
    async fn list_notes(&self, params: Parameters<PatternIdParam>) -> String {
        match self.bridge.list_notes(params.0.pattern_id) {
            Ok(notes) => to_json(&notes),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add a note to a pattern. pitch: MIDI 0-127 (60=C4). start_beat/duration_beats in beats (1.0=quarter). velocity: 0-127."
    )]
    async fn add_note(&self, params: Parameters<AddNoteParam>) -> String {
        if let Err(e) = validate_note_fields(
            params.0.pitch,
            params.0.velocity,
            params.0.start_beat,
            params.0.duration_beats,
        ) {
            return validation_err(e);
        }
        match self.bridge.add_note(
            params.0.pattern_id,
            params.0.pitch,
            params.0.start_beat,
            params.0.duration_beats,
            params.0.velocity,
            params.0.instrument_id,
        ) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Remove a note from a pattern by note ID")]
    async fn remove_note(&self, params: Parameters<RemoveNoteParam>) -> String {
        match self
            .bridge
            .remove_note(params.0.pattern_id, params.0.note_id)
        {
            Ok(()) => format!(
                "OK: removed note {} from pattern {}",
                params.0.note_id, params.0.pattern_id
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Update a note's properties. Only provided fields are changed; null fields keep their current value."
    )]
    async fn update_note(&self, params: Parameters<UpdateNoteParam>) -> String {
        if let Err(e) = validate_note_update_fields(
            params.0.pitch,
            params.0.velocity,
            params.0.start_beat,
            params.0.duration_beats,
        ) {
            return validation_err(e);
        }
        match self.bridge.update_note(
            params.0.pattern_id,
            params.0.note_id,
            params.0.pitch,
            params.0.start_beat,
            params.0.duration_beats,
            params.0.velocity,
        ) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Tracks ===

    #[tool(
        description = "List all sequencer tracks with their names, instruments, and mute/solo state"
    )]
    async fn list_tracks(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_tracks() {
            Ok(tracks) => to_json(&tracks),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create a new sequencer track. Optionally assign an instrument and set a name. Returns the track ID."
    )]
    async fn create_track(&self, params: Parameters<CreateTrackParam>) -> String {
        if let Err(e) = validate_name("track", &params.0.name) {
            return validation_err(e);
        }
        match self
            .bridge
            .create_track(&params.0.name, params.0.instrument_id)
        {
            Ok(id) => format!("OK: created track {id} '{}'", params.0.name),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Arrangement ===

    #[tool(
        description = "Place a pattern on a track at a beat position in the arrangement timeline"
    )]
    async fn place_pattern(&self, params: Parameters<PlacePatternParam>) -> String {
        if let Err(e) = validate_range("start_beat", params.0.start_beat, 0.0, 9999.0) {
            return validation_err(e);
        }
        match self
            .bridge
            .place_pattern(params.0.pattern_id, params.0.track_id, params.0.start_beat)
        {
            Ok(()) => format!(
                "OK: placed pattern {} on track {} at beat {}",
                params.0.pattern_id, params.0.track_id, params.0.start_beat
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Remove a pattern placement from the arrangement by specifying the pattern_id, track_id, and start_beat of the placement to remove."
    )]
    async fn remove_placement(&self, params: Parameters<PlacePatternParam>) -> String {
        match self.bridge.remove_placement(
            params.0.pattern_id,
            params.0.track_id,
            params.0.start_beat,
        ) {
            Ok(()) => format!(
                "OK: removed placement of pattern {} from track {} at beat {}",
                params.0.pattern_id, params.0.track_id, params.0.start_beat
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all pattern placements in the song arrangement. Each placement maps a pattern to a track at a beat position."
    )]
    async fn list_arrangement(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_arrangement() {
            Ok(placements) => to_json(&placements),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Batch operations ===

    #[tool(
        description = "Add multiple notes to a pattern in one call. Much faster than calling add_note repeatedly."
    )]
    async fn add_notes(&self, params: Parameters<AddNotesParam>) -> String {
        for n in &params.0.notes {
            if let Err(e) = validate_note_input(n) {
                return validation_err(e);
            }
        }
        let notes: Vec<_> = params
            .0
            .notes
            .iter()
            .map(|n| crate::bridge::BridgeNoteData {
                pitch: n.pitch,
                start_beat: n.start_beat,
                duration_beats: n.duration_beats,
                velocity: n.velocity.unwrap_or(100),
                instrument_id: n.instrument_id,
            })
            .collect();
        match self.bridge.add_notes(params.0.pattern_id, &notes) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Update multiple notes in a pattern in one call. Only provided fields are changed per note."
    )]
    async fn update_notes(&self, params: Parameters<UpdateNotesParam>) -> String {
        for u in &params.0.updates {
            if let Err(e) =
                validate_note_update_fields(u.pitch, u.velocity, u.start_beat, u.duration_beats)
            {
                return validation_err(e);
            }
        }
        let updates: Vec<_> = params
            .0
            .updates
            .iter()
            .map(|u| crate::bridge::BridgeNoteUpdate {
                note_id: u.note_id,
                pitch: u.pitch,
                start_beat: u.start_beat,
                duration_beats: u.duration_beats,
                velocity: u.velocity,
            })
            .collect();
        match self.bridge.update_notes(params.0.pattern_id, &updates) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Replace all notes in a pattern: clears existing notes, then adds the new ones. Use for full pattern rewrites."
    )]
    async fn replace_notes(&self, params: Parameters<ReplaceNotesParam>) -> String {
        for n in &params.0.notes {
            if let Err(e) = validate_note_input(n) {
                return validation_err(e);
            }
        }
        let notes: Vec<_> = params
            .0
            .notes
            .iter()
            .map(|n| crate::bridge::BridgeNoteData {
                pitch: n.pitch,
                start_beat: n.start_beat,
                duration_beats: n.duration_beats,
                velocity: n.velocity.unwrap_or(100),
                instrument_id: n.instrument_id,
            })
            .collect();
        match self.bridge.replace_notes(params.0.pattern_id, &notes) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Clear all notes from a pattern. Returns the number of notes removed.")]
    async fn clear_pattern(&self, params: Parameters<ClearPatternParam>) -> String {
        match self.bridge.clear_pattern(params.0.pattern_id) {
            Ok(count) => format!(
                "OK: cleared {count} notes from pattern {}",
                params.0.pattern_id
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add automation points to a pattern. Each point specifies a parameter (e.g. Volume, Pan, FilterCutoff), position in beats, and a normalized value (0.0-1.0)."
    )]
    async fn add_automation_points(&self, params: Parameters<AddAutomationPointsParam>) -> String {
        for pt in &params.0.points {
            if let Err(e) = validate_automation_point(pt) {
                return validation_err(e);
            }
        }
        let p = params.0;
        let points: Vec<_> = p
            .points
            .into_iter()
            .map(|pt| crate::bridge::BridgeAutomationPointData {
                param: pt.param,
                instrument_id: pt.instrument_id.unwrap_or(0),
                beat: pt.beat,
                value: pt.value,
                curve: pt.curve.unwrap_or_else(|| "Linear".to_string()),
            })
            .collect();
        match self.bridge.add_automation_points(p.pattern_id, &points) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all automation lanes in a pattern with their target parameters and point counts."
    )]
    async fn list_automation_lanes(&self, params: Parameters<PatternIdParam>) -> String {
        match self.bridge.list_automation_lanes(params.0.pattern_id) {
            Ok(lanes) => to_json(&lanes),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Get all automation points for a specific parameter lane in a pattern.")]
    async fn get_automation_points(&self, params: Parameters<GetAutomationPointsParam>) -> String {
        let p = params.0;
        match self.bridge.get_automation_points(
            p.pattern_id,
            &p.target,
            p.instrument_id.unwrap_or(0),
        ) {
            Ok(points) => to_json(&points),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Remove automation points at specific beat positions from a lane.")]
    async fn remove_automation_points(
        &self,
        params: Parameters<RemoveAutomationPointsParam>,
    ) -> String {
        let p = params.0;
        match self.bridge.remove_automation_points(
            p.pattern_id,
            &p.target,
            p.instrument_id.unwrap_or(0),
            &p.beats,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Clear all automation points from a specific lane in a pattern.")]
    async fn clear_automation_lane(&self, params: Parameters<ClearAutomationLaneParam>) -> String {
        let p = params.0;
        match self.bridge.clear_automation_lane(
            p.pattern_id,
            &p.target,
            p.instrument_id.unwrap_or(0),
        ) {
            Ok(count) => format!("OK: cleared {count} points from {}", p.target),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Track control ===

    #[tool(
        description = "Set the volume of a track (0.0 = silent, 1.0 = full, up to 2.0 for boost)."
    )]
    async fn set_track_volume(&self, params: Parameters<SetTrackVolumeParam>) -> String {
        if let Err(e) = validate_range("volume", params.0.volume, 0.0, 2.0) {
            return validation_err(e);
        }
        match self
            .bridge
            .set_track_volume(params.0.track_id, params.0.volume)
        {
            Ok(()) => format!(
                "OK: track {} volume set to {}",
                params.0.track_id, params.0.volume
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the pan position of a track (-1.0 = left, 0.0 = center, 1.0 = right)."
    )]
    async fn set_track_pan(&self, params: Parameters<SetTrackPanParam>) -> String {
        if let Err(e) = validate_range("pan", params.0.pan, -1.0, 1.0) {
            return validation_err(e);
        }
        match self.bridge.set_track_pan(params.0.track_id, params.0.pan) {
            Ok(()) => format!(
                "OK: track {} pan set to {}",
                params.0.track_id, params.0.pan
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Mute or unmute a track.")]
    async fn set_track_mute(&self, params: Parameters<SetTrackMuteParam>) -> String {
        match self
            .bridge
            .set_track_mute(params.0.track_id, params.0.muted)
        {
            Ok(()) => {
                let state = if params.0.muted { "muted" } else { "unmuted" };
                format!("OK: track {} {state}", params.0.track_id)
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Solo or unsolo a track. When any track is soloed, only soloed tracks produce sound."
    )]
    async fn set_track_solo(&self, params: Parameters<SetTrackSoloParam>) -> String {
        match self.bridge.set_track_solo(params.0.track_id, params.0.solo) {
            Ok(()) => {
                let state = if params.0.solo { "soloed" } else { "unsoloed" };
                format!("OK: track {} {state}", params.0.track_id)
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the instrument assigned to a track. All notes on this track will play through the assigned instrument. \
                       Set instrument_id to null to unassign."
    )]
    async fn set_track_instrument(&self, params: Parameters<SetTrackInstrumentParam>) -> String {
        match self
            .bridge
            .set_track_instrument(params.0.track_id, params.0.instrument_id)
        {
            Ok(()) => {
                let inst = params
                    .0
                    .instrument_id
                    .map_or_else(|| "none".to_string(), |id| id.to_string());
                format!("OK: track {} instrument set to {inst}", params.0.track_id)
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Rename a track. The name is shown in the sequencer track headers.")]
    async fn rename_track(&self, params: Parameters<RenameTrackParam>) -> String {
        if let Err(e) = validate_name("track", &params.0.name) {
            return validation_err(e);
        }
        match self.bridge.rename_track(params.0.track_id, &params.0.name) {
            Ok(()) => format!(
                "OK: track {} renamed to '{}'",
                params.0.track_id, params.0.name
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Delete a track and all its placements from the arrangement.")]
    async fn delete_track(&self, params: Parameters<DeleteTrackParam>) -> String {
        match self.bridge.delete_track(params.0.track_id) {
            Ok(()) => format!("OK: deleted track {}", params.0.track_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Pattern management ===

    #[tool(
        description = "Rename a pattern. The name is shown in the arrangement timeline and piano roll."
    )]
    async fn rename_pattern(&self, params: Parameters<RenamePatternParam>) -> String {
        if let Err(e) = validate_name("pattern", &params.0.name) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .rename_pattern(params.0.pattern_id, &params.0.name)
        {
            Ok(()) => format!(
                "OK: pattern {} renamed to '{}'",
                params.0.pattern_id, params.0.name
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the length of a pattern in beats (e.g. 4.0 = one bar in 4/4, 8.0 = two bars)."
    )]
    async fn set_pattern_length(&self, params: Parameters<SetPatternLengthParam>) -> String {
        if let Err(e) = validate_range("length_beats", params.0.length_beats, 0.001, 1024.0) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .set_pattern_length(params.0.pattern_id, params.0.length_beats)
        {
            Ok(()) => format!(
                "OK: pattern {} length set to {} beats",
                params.0.pattern_id, params.0.length_beats
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Duplicate a pattern including all notes and automation. Returns the new pattern ID."
    )]
    async fn duplicate_pattern(&self, params: Parameters<DuplicatePatternParam>) -> String {
        match self.bridge.duplicate_pattern(params.0.pattern_id) {
            Ok(new_id) => format!(
                "OK: duplicated pattern {} as new pattern {new_id}",
                params.0.pattern_id
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Song metadata ===

    #[tool(description = "Set the song author name.")]
    async fn set_song_author(&self, params: Parameters<SetSongAuthorParam>) -> String {
        if let Err(e) = validate_name("author", &params.0.author) {
            return format!("Error: {e}");
        }
        match self.bridge.set_song_author(&params.0.author) {
            Ok(()) => format!("OK: song author set to '{}'", params.0.author),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Set the song time signature (e.g. 4/4, 3/4, 6/8).")]
    async fn set_song_time_signature(
        &self,
        params: Parameters<SetSongTimeSignatureParam>,
    ) -> String {
        if let Err(e) = validate_time_signature(params.0.numerator, params.0.denominator) {
            return validation_err(e);
        }
        match self
            .bridge
            .set_song_time_signature(params.0.numerator, params.0.denominator)
        {
            Ok(()) => format!(
                "OK: time signature set to {}/{}",
                params.0.numerator, params.0.denominator
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Batch parameter set ===

    #[tool(
        description = "Set multiple module parameters in one call. Faster than calling set_parameter repeatedly."
    )]
    async fn set_parameters(&self, params: Parameters<SetParametersParam>) -> String {
        let p = params.0;
        let param_sets: Vec<_> = p
            .params
            .into_iter()
            .map(|ps| crate::bridge::BridgeParamSet {
                module_id: ps.module_id,
                param_name: ps.param_name,
                value: ps.value,
            })
            .collect();
        match self.bridge.set_parameters(p.instrument_id, &param_sets) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create multiple patterns in one call, optionally with inline notes and automation. Returns per-pattern results with assigned IDs."
    )]
    async fn create_patterns(&self, params: Parameters<CreatePatternsParam>) -> String {
        for (i, pat) in params.0.patterns.iter().enumerate() {
            if let Err(e) = validate_name("pattern", &pat.name) {
                return validation_err(McpBridgeError::Other(format!("pattern[{i}]: {e}")));
            }
            if let Err(e) = validate_range("length_beats", pat.length_beats, 0.001, 1024.0) {
                return validation_err(McpBridgeError::Other(format!("pattern[{i}]: {e}")));
            }
            if let Some(ref notes) = pat.notes {
                for n in notes {
                    if let Err(e) = validate_note_input(n) {
                        return validation_err(McpBridgeError::Other(format!(
                            "pattern[{i}] note: {e}"
                        )));
                    }
                }
            }
            if let Some(ref auto) = pat.automation
                && let Err(e) = validate_automation_points_input(auto)
            {
                return validation_err(McpBridgeError::Other(format!(
                    "pattern[{i}] automation: {e}"
                )));
            }
        }
        let patterns: Vec<_> = params
            .0
            .patterns
            .into_iter()
            .map(|p| crate::bridge::BridgePatternData {
                name: p.name,
                length_beats: p.length_beats,
                notes: p
                    .notes
                    .unwrap_or_default()
                    .iter()
                    .map(|n| crate::bridge::BridgeNoteData {
                        pitch: n.pitch,
                        start_beat: n.start_beat,
                        duration_beats: n.duration_beats,
                        velocity: n.velocity.unwrap_or(100),
                        instrument_id: n.instrument_id,
                    })
                    .collect(),
                automation: convert_automation_points(p.automation),
            })
            .collect();
        match self.bridge.create_patterns(&patterns) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create multiple tracks in one call. Returns per-track results with assigned IDs."
    )]
    async fn create_tracks(&self, params: Parameters<CreateTracksParam>) -> String {
        for t in &params.0.tracks {
            if let Err(e) = validate_name("track", &t.name) {
                return validation_err(e);
            }
        }
        let tracks: Vec<_> = params
            .0
            .tracks
            .into_iter()
            .map(|t| crate::bridge::BridgeTrackData {
                name: t.name,
                instrument_id: t.instrument_id,
            })
            .collect();
        match self.bridge.create_tracks(&tracks) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Place multiple patterns in the arrangement in one call. Each placement specifies pattern_id, track_id, and start_beat."
    )]
    async fn place_patterns(&self, params: Parameters<PlacePatternsParam>) -> String {
        for p in &params.0.placements {
            if let Err(e) = validate_range("start_beat", p.start_beat, 0.0, 9999.0) {
                return validation_err(e);
            }
        }
        let placements: Vec<_> = params
            .0
            .placements
            .into_iter()
            .map(|p| crate::bridge::BridgePlacementData {
                pattern_id: p.pattern_id,
                track_id: p.track_id,
                start_beat: p.start_beat,
            })
            .collect();
        match self.bridge.place_patterns(&placements) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Build a complete song in one call: creates patterns (with notes and optional automation), tracks, and arrangement placements. \
                       Replaces the current song. Placements use array indices (pattern_index, track_index) since IDs are assigned during creation. \
                       Returns a summary with all assigned IDs."
    )]
    async fn set_song(&self, params: Parameters<SetSongParam>) -> String {
        let p = params.0;
        // Validate song name
        if let Err(e) = validate_name("song", &p.name) {
            return validation_err(e);
        }
        // Validate patterns: length, notes, automation
        for (i, pat) in p.patterns.iter().enumerate() {
            if let Err(e) = validate_range("length_beats", pat.length_beats, 0.001, 1024.0) {
                return validation_err(McpBridgeError::Other(format!("pattern[{i}]: {e}")));
            }
            for n in &pat.notes {
                if let Err(e) = validate_note_input(n) {
                    return validation_err(McpBridgeError::Other(format!(
                        "pattern[{i}] note: {e}"
                    )));
                }
            }
            if let Some(ref auto) = pat.automation
                && let Err(e) = validate_automation_points_input(auto)
            {
                return validation_err(McpBridgeError::Other(format!(
                    "pattern[{i}] automation: {e}"
                )));
            }
        }
        // Validate track names
        for (i, t) in p.tracks.iter().enumerate() {
            if let Err(e) = validate_name("track", &t.name) {
                return validation_err(McpBridgeError::Other(format!("track[{i}]: {e}")));
            }
        }
        // Validate placement indices and start_beat
        for (i, pl) in p.placements.iter().enumerate() {
            if pl.pattern_index >= p.patterns.len() {
                return validation_err(McpBridgeError::IndexOutOfBounds {
                    name: "pattern_index",
                    index: pl.pattern_index,
                    count: p.patterns.len(),
                });
            }
            if pl.track_index >= p.tracks.len() {
                return validation_err(McpBridgeError::IndexOutOfBounds {
                    name: "track_index",
                    index: pl.track_index,
                    count: p.tracks.len(),
                });
            }
            if let Err(e) = validate_range("start_beat", pl.start_beat, 0.0, 9999.0) {
                return validation_err(McpBridgeError::Other(format!("placement[{i}]: {e}")));
            }
        }
        let patterns: Vec<_> = p
            .patterns
            .into_iter()
            .map(|pat| crate::bridge::BridgePatternData {
                name: pat.name,
                length_beats: pat.length_beats,
                notes: pat
                    .notes
                    .iter()
                    .map(|n| crate::bridge::BridgeNoteData {
                        pitch: n.pitch,
                        start_beat: n.start_beat,
                        duration_beats: n.duration_beats,
                        velocity: n.velocity.unwrap_or(100),
                        instrument_id: n.instrument_id,
                    })
                    .collect(),
                automation: convert_automation_points(pat.automation),
            })
            .collect();
        let tracks: Vec<_> = p
            .tracks
            .into_iter()
            .map(|t| crate::bridge::BridgeTrackData {
                name: t.name,
                instrument_id: t.instrument_id,
            })
            .collect();
        let placements: Vec<_> = p
            .placements
            .into_iter()
            .map(|pl| crate::bridge::BridgeSongPlacement {
                pattern_index: pl.pattern_index,
                track_index: pl.track_index,
                start_beat: pl.start_beat,
            })
            .collect();
        let tempo = p.tempo.unwrap_or(120.0);
        match self
            .bridge
            .set_song(&p.name, tempo, &patterns, &tracks, &placements)
        {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Sequencer: Transport ===

    #[tool(
        description = "Start sequencer playback from the current position. Use seq_seek first to set position."
    )]
    async fn seq_play(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.seq_play() {
            Ok(()) => "OK: sequencer playing".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Stop sequencer playback and reset position")]
    async fn seq_stop(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.seq_stop() {
            Ok(()) => "OK: sequencer stopped".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Seek the sequencer to a beat position (0.0 = beginning)")]
    async fn seq_seek(&self, params: Parameters<SeqSeekParam>) -> String {
        match self.bridge.seq_seek(params.0.beat) {
            Ok(()) => format!("OK: seeked to beat {}", params.0.beat),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Batch instrument building ===

    #[tool(
        description = "Build a complete instrument in ONE call: creates the instrument, adds all modules, sets parameters, and wires connections. \
                       Modules are referenced by 0-based array index in connections. Returns instrument_id and module_ids. \
                       Example: modules=[{module_type:'osc'},{module_type:'amp'},{module_type:'out'}], connections=[{from:0,from_port:'output',to:1,to_port:'input'},{from:1,from_port:'output',to:2,to_port:'input'}]"
    )]
    async fn build_instrument(&self, params: Parameters<BuildInstrumentParam>) -> String {
        let p = params.0;
        if let Err(e) = validate_build_instrument_fields(
            &p.name,
            p.midi_channel,
            p.volume,
            p.pan,
            &p.modules,
            p.connections.as_deref(),
        ) {
            return validation_err(e);
        }
        let spec = convert_instrument_def(
            p.instrument_id,
            p.name,
            p.midi_channel,
            p.volume,
            p.pan,
            p.modules,
            p.connections,
        );
        match self.bridge.build_instrument(&spec) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Build multiple instruments in one call. Each instrument has its own modules and connections. \
                       Returns an array of results with instrument_id and module_ids per instrument."
    )]
    async fn build_instruments(&self, params: Parameters<BuildInstrumentsParam>) -> String {
        for (idx, inst) in params.0.instruments.iter().enumerate() {
            if let Err(e) = validate_build_instrument_fields(
                &inst.name,
                inst.midi_channel,
                inst.volume,
                inst.pan,
                &inst.modules,
                inst.connections.as_deref(),
            ) {
                return validation_err(McpBridgeError::Other(format!("instrument[{idx}]: {e}")));
            }
        }
        let specs: Vec<_> = params
            .0
            .instruments
            .into_iter()
            .map(|i| {
                convert_instrument_def(
                    i.instrument_id,
                    i.name,
                    i.midi_channel,
                    i.volume,
                    i.pan,
                    i.modules,
                    i.connections,
                )
            })
            .collect();
        match self.bridge.build_instruments(&specs) {
            Ok(results) => to_json(&results),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Apply a named example patch directly to an instrument, creating all modules, parameters, and connections. \
                       If instrument_id is omitted, creates a new instrument. Much faster than load_example_patch (no GUI queue). \
                       Use list_example_patches to see available patches."
    )]
    async fn apply_example_patch(&self, params: Parameters<ApplyExamplePatchParam>) -> String {
        match self
            .bridge
            .apply_example_patch(params.0.instrument_id, &params.0.patch_name)
        {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Project management ===

    #[tool(description = "Reset to a new empty project, clearing all instruments and song data.")]
    async fn new_project(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.new_project() {
            Ok(msg) => format!("OK: {msg}"),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Save the current project (all instruments, patches, song, arrangement) to a JSON file."
    )]
    async fn save_project(&self, params: Parameters<ProjectPathParam>) -> String {
        match self.bridge.save_project(&params.0.path) {
            Ok(msg) => format!("OK: {msg}"),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Load a project or patch file, replacing all current state. Supports both project files and single patch files."
    )]
    async fn load_project(&self, params: Parameters<ProjectPathParam>) -> String {
        match self.bridge.load_project(&params.0.path) {
            Ok(msg) => format!("OK: {msg}"),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Optimize the project by removing unused patterns (not placed in arrangement), \
                       unused tracks (no placements), and unused instruments (not referenced by any track or note). \
                       Returns a summary of what was removed."
    )]
    async fn optimize_project(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.optimize_project() {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === AWE (Acoustic World Engine) ===

    #[tool(
        description = "Get the current state of the Acoustic World Engine (AWE) — a physics-based room simulation \
                       that adds reverb, early reflections, and spatial effects. Returns room shape, material, \
                       all acoustic parameters, source/listener positions, LFO states, and whether AWE is enabled. \
                       Call this first to understand the current acoustic environment before making changes."
    )]
    async fn get_awe_state(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_awe_state() {
            Ok(state) => to_json(&state),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Enable or disable AWE (Acoustic World Engine). AWE must be enabled for room simulation \
                       to affect the audio output. When disabled, audio passes through dry."
    )]
    async fn set_awe_enabled(&self, params: Parameters<SetAweEnabledParam>) -> String {
        match self.bridge.set_awe_enabled(params.0.enabled) {
            Ok(()) => {
                let state = if params.0.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                format!("OK: AWE {state}")
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set a single AWE acoustic parameter by name. Use get_awe_state to see current values \
                       and valid ranges. Each parameter controls a different aspect of the room simulation — \
                       see the parameter name description for all options and their ranges."
    )]
    async fn set_awe_parameter(&self, params: Parameters<SetAweParameterParam>) -> String {
        match self
            .bridge
            .set_awe_parameter(&params.0.name, params.0.value)
        {
            Ok(()) => format!("OK: AWE {} = {}", params.0.name, params.0.value),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the AWE room shape and dimensions. The room geometry determines early reflection \
                       patterns, room modes (standing waves), and reverb character. \
                       Changing the room shape will clamp source/listener positions to fit the new geometry. \
                       Hint: small rooms (< 5m) sound tight and colored, large rooms (> 20m) sound spacious."
    )]
    async fn set_awe_room_shape(&self, params: Parameters<SetAweRoomShapeParam>) -> String {
        // Validate dimensions are positive
        for (i, &d) in params.0.dimensions.iter().enumerate() {
            if d <= 0.0 || d.is_nan() {
                return format!(
                    "Error: dimension[{i}] must be positive, got {d}. \
                     All room dimensions are in meters and must be > 0."
                );
            }
        }
        match self
            .bridge
            .set_awe_room_shape(&params.0.shape, &params.0.dimensions)
        {
            Ok(()) => format!("OK: AWE room shape set to {}", params.0.shape),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the AWE wall material. Materials determine how sound is absorbed and reflected \
                       at different frequencies, dramatically affecting the reverb character. \
                       Hard materials (Metal, Tile, Concrete) create bright, long reverb tails. \
                       Soft materials (Carpet, Fabric, Nanogel) create dark, short reverb. \
                       Exotic materials (Void, Prism, Plasma, Membrane) create non-physical effects."
    )]
    async fn set_awe_material(&self, params: Parameters<SetAweMaterialParam>) -> String {
        match self.bridge.set_awe_material(&params.0.material) {
            Ok(()) => format!("OK: AWE material set to {}", params.0.material),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Load a named AWE preset. Presets configure the complete room simulation \
                       (room shape, material, all acoustic parameters, positions) in one call. \
                       This also enables AWE. Use list_awe_presets to see all available presets. \
                       After loading a preset, use set_awe_parameter to fine-tune individual settings."
    )]
    async fn set_awe_preset(&self, params: Parameters<SetAwePresetParam>) -> String {
        match self.bridge.set_awe_preset(&params.0.name) {
            Ok(state) => to_json(&state),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all available AWE (Acoustic World Engine) presets with their names and descriptions. \
                       Presets range from realistic spaces (Cathedral, Concert Hall, Bathroom, Small Studio) \
                       to creative effects (Dream, Portal, Stargate) and exotic/sci-fi environments \
                       (EXT: Singularity, EXT: Plasma Storm, EXT: Nano Fog)."
    )]
    async fn list_awe_presets(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_awe_presets() {
            Ok(presets) => to_json(&presets),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Configure one of AWE's 4 internal LFOs. LFOs automatically modulate AWE parameters \
                       at a given rate, creating evolving, animated room effects. \
                       Example: set LFO 1 to slowly sweep the source position for a moving-source effect, \
                       or modulate tail_stretch for breathing reverb."
    )]
    async fn set_awe_lfo(&self, params: Parameters<SetAweLfoParam>) -> String {
        let p = params.0;
        if !(1..=4).contains(&p.index) {
            return format!(
                "Error: LFO index must be 1-4, got {}. AWE has 4 LFOs (LFO 1 through LFO 4).",
                p.index
            );
        }
        if let Err(e) = validate_range("rate", p.rate, 0.01, 20.0) {
            return format!("Error: {e}");
        }
        if let Err(e) = validate_range("amount", p.amount, 0.0, 1.0) {
            return format!("Error: {e}");
        }
        match self
            .bridge
            .set_awe_lfo(p.index, p.rate, p.amount, &p.target)
        {
            Ok(()) => format!(
                "OK: AWE LFO {} → {} at {:.2} Hz (amount {:.2})",
                p.index, p.target, p.rate, p.amount
            ),
            Err(e) => format!("Error: {e}"),
        }
    }
}

/// Convert input structs to bridge-level types.
fn convert_instrument_def(
    instrument_id: Option<u64>,
    name: String,
    midi_channel: Option<u8>,
    volume: Option<f32>,
    pan: Option<f32>,
    modules: Vec<ModuleDefInput>,
    connections: Option<Vec<ConnectionDefInput>>,
) -> crate::bridge::BridgeInstrumentDef {
    use crate::bridge::{
        BridgeConnectionDef, BridgeInstrumentDef, BridgeModuleDef, BridgeParamValue,
    };

    let bridge_modules = modules
        .into_iter()
        .map(|m| BridgeModuleDef {
            module_type: m.module_type,
            params: m
                .params
                .unwrap_or_default()
                .into_iter()
                .map(|(k, v)| {
                    let bv = match v {
                        ParamValueInput::Number(n) => BridgeParamValue::Number(n),
                        ParamValueInput::Bool(b) => BridgeParamValue::Bool(b),
                        ParamValueInput::Choice(s) => BridgeParamValue::Choice(s),
                    };
                    (k, bv)
                })
                .collect(),
        })
        .collect();

    let bridge_connections = connections
        .unwrap_or_default()
        .into_iter()
        .map(|c| BridgeConnectionDef {
            from_index: c.from,
            from_port: c.from_port,
            to_index: c.to,
            to_port: c.to_port,
        })
        .collect();

    BridgeInstrumentDef {
        instrument_id,
        name,
        midi_channel,
        volume,
        pan,
        modules: bridge_modules,
        connections: bridge_connections,
    }
}

/// Convert optional automation point inputs to bridge-level data.
fn convert_automation_points(
    points: Option<Vec<AutomationPointInput>>,
) -> Vec<crate::bridge::BridgeAutomationPointData> {
    points
        .unwrap_or_default()
        .into_iter()
        .map(|pt| crate::bridge::BridgeAutomationPointData {
            param: pt.param,
            instrument_id: pt.instrument_id.unwrap_or(0),
            beat: pt.beat,
            value: pt.value,
            curve: pt.curve.unwrap_or_else(|| "Linear".to_string()),
        })
        .collect()
}
