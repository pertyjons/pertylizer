//! MCP server implementation using rmcp.
//!
//! Defines tool handlers that delegate to the SynthBridge trait.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::FutureExt;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ListResourceTemplatesResult, ListResourcesResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, ServerCapabilities, ServerInfo,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use synth_core::InstrumentId;

use crate::bridge::SynthBridge;
use crate::error::McpBridgeError;

// === Helper functions ===

/// Reject paths containing `..` components to prevent directory traversal attacks.
fn validate_file_path(path: &str) -> Result<(), String> {
    use std::path::Path;
    let p = Path::new(path);
    for component in p.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Error: path must not contain '..' components".to_string());
        }
    }
    if !p.is_absolute() {
        return Err("Error: path must be absolute".to_string());
    }
    Ok(())
}

/// Serialize a value to pretty-printed JSON, returning an error string on failure.
fn to_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|e| format!("Serialization error: {e}"))
}

/// Extract a human-readable message from a caught panic payload
/// (`Box<dyn Any + Send>`), covering the common `&str` / `String` cases.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Poll a tool future, converting any panic into `Err(panic message)` instead
/// of letting it unwind into — and kill — the tokio worker thread (which would
/// drop the whole MCP session, surfacing as `404 Session not found` on the
/// next request). A panic inside a synchronous `block_in_place` bridge call
/// unwinds during this poll, so it is caught here. parking_lot locks don't
/// poison, so the bridge stays usable after a recovered panic.
///
/// On a caught panic the recovery is logged once here (so both the direct and
/// the batch entry points share one log site) and the panic message is returned
/// to the caller, which maps it onto its own error type.
async fn run_catching_panic<R>(
    session_id: u64,
    tool: &str,
    fut: impl std::future::Future<Output = R>,
) -> Result<R, String> {
    match AssertUnwindSafe(fut).catch_unwind().await {
        Ok(output) => Ok(output),
        Err(payload) => {
            let msg = panic_message(payload.as_ref());
            tracing::error!(
                session_id,
                tool,
                panic = %msg,
                "MCP tool handler panicked (recovered)"
            );
            Err(msg)
        }
    }
}

/// Run a blocking bridge call on a dedicated worker so the tokio executor
/// stays available for SSE keep-alives, then format the result as JSON (or
/// `"Error: …"` on failure). Use for every tool that performs offline
/// rendering or other long-running synchronous work.
fn run_blocking_json<T, E, F>(f: F) -> String
where
    T: serde::Serialize,
    E: std::fmt::Display,
    F: FnOnce() -> Result<T, E>,
{
    tokio::task::block_in_place(|| match f() {
        Ok(result) => to_json(&result),
        Err(e) => format!("Error: {e}"),
    })
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

/// Validate note fields that are always required (add_note, etc.).
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
    if pt.param.is_none() && pt.target.is_none() {
        return Err(McpBridgeError::Other(
            "automation point requires a 'param' string or a structured 'target'".to_string(),
        ));
    }
    validate_range("value", pt.value, 0.0, 1.0)?;
    validate_range("beat", pt.beat, 0.0, 9999.0)?;
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
    )?;
    // Glide fields are validated at the boundary too, so a bogus glide source is
    // a surfaced error rather than a silently-dropped glide (consistent with the
    // main pitch being validated).
    if let Some(g) = &n.glide {
        if let Some(p) = g.from_pitch {
            validate_midi_note(p)?;
        }
        if let Some(t) = g.time_ms {
            validate_range("glide.time_ms", t, 0.0, 60_000.0)?;
        }
    }
    if let Some(e) = &n.expression {
        if let Some(a) = e.accent {
            validate_range("expression.accent", a, 0.0, 16.0)?;
        }
        if let Some(g) = e.gate {
            validate_range("expression.gate", g, 0.0, 1.0)?;
        }
        if let Some(p) = e.probability {
            validate_range("expression.probability", p, 0.0, 1.0)?;
        }
        if let Some(v) = &e.vibrato {
            if let Some(d) = v.depth {
                validate_range("expression.vibrato.depth", d, 0.0, 48.0)?;
            }
            if let Some(r) = v.rate {
                validate_range("expression.vibrato.rate", r, 0.0, 100.0)?;
            }
            if let Some(ms) = v.delay_ms {
                validate_range("expression.vibrato.delay_ms", ms, 0.0, 60_000.0)?;
            }
            // Reject an unrecognized shape token rather than silently coercing it
            // to sine. Token set mirrors `VibratoShape::from_token` (synth_sequencer,
            // not reachable from this crate).
            if let Some(shape) = v.shape.as_deref() {
                let t = shape.trim();
                let known = [
                    "sine", "sin", "triangle", "tri", "square", "sqr", "saw", "sawtooth",
                ];
                if !known.iter().any(|k| t.eq_ignore_ascii_case(k)) {
                    return Err(McpBridgeError::InvalidChoice {
                        name: "expression.vibrato.shape".to_owned(),
                        value: shape.to_owned(),
                        detail: "expected one of: sine, triangle, square, saw".to_owned(),
                    });
                }
            }
        }
    }
    Ok(())
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

/// Summarise a batch of single-item mutations into one response string.
/// `noun` describes the items (e.g. "notes removed", "modules added").
/// `details` (optional, e.g. created module-ids) are listed when present, and
/// any per-item errors are appended. Full/partial success leads with `OK:`; a
/// whole-batch failure (nothing succeeded) leads with `Error:` so a caller that
/// gates on a leading marker can tell total failure from partial success.
fn batch_msg(ok_count: usize, noun: &str, details: &[String], errors: &[String]) -> String {
    // Total failure (nothing succeeded) must not read as success: a caller/script
    // that gates on a leading "Error:" would treat "OK: 0 …" as a pass. Lead with
    // a failure marker when every item failed, keep "OK:" for full/partial success.
    let leader = if ok_count == 0 && !errors.is_empty() {
        "Error:"
    } else {
        "OK:"
    };
    let mut out = format!("{leader} {ok_count} {noun}");
    if !details.is_empty() {
        out.push_str(&format!(" ({})", details.join(", ")));
    }
    if !errors.is_empty() {
        out.push_str(&format!("; {} failed: {}", errors.len(), errors.join("; ")));
    }
    out
}

/// Summarise a batch of mutations that each return a JSON object (created /
/// imported / duplicated entities) as a single valid-JSON response. Always emits
/// `{ "<noun>": [...successes...], "errors": [...] }` so callers can `JSON.parse`
/// the reply on full *and* partial success (unlike interleaving JSON with prose).
fn batch_json<T: serde::Serialize>(noun: &str, oks: &[T], errors: &[String]) -> String {
    let oks_value =
        serde_json::to_value(oks).unwrap_or_else(|_| serde_json::Value::Array(Vec::new()));
    to_json(&serde_json::json!({ noun: oks_value, "errors": errors }))
}

/// Classify a tool result string as a *total* failure, for `batch_execute`'s
/// stop-on-error / rollback gate. Prose results lead with `"Error:"` on failure
/// (and [`batch_msg`] now does so for whole-batch failures too). [`batch_json`]
/// results stay valid JSON, so their whole-batch failure is detected
/// structurally: a non-empty `"errors"` array with no successes (every other
/// array field empty) — the `{ "<noun>": [], "errors": [..] }` shape. Partial
/// success (some items landed) is *not* a failure, matching the prose path.
fn result_is_failure(result: &str) -> bool {
    if result.starts_with("Error:") {
        return true;
    }
    // Only batch_json emits an "errors" array, and it always starts with '{'.
    if !result.starts_with('{') {
        return false;
    }
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(result)
    else {
        return false;
    };
    let errors_nonempty = map
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|a| !a.is_empty());
    if !errors_nonempty {
        return false;
    }
    // Total failure only: no non-"errors" array carries any success.
    let any_success = map
        .iter()
        .any(|(k, v)| k != "errors" && v.as_array().is_some_and(|a| !a.is_empty()));
    !any_success
}

/// Convert a tagged [`ParamValueInput`] into the bridge's [`BridgeParamValue`],
/// rejecting non-finite numbers. Shared by the effect-parameter batch handlers.
fn param_value_to_bridge(
    value: ParamValueInput,
) -> Result<crate::bridge::BridgeParamValue, String> {
    match value {
        ParamValueInput::Number(n) => {
            if !n.is_finite() {
                return Err(format!(
                    "{}",
                    McpBridgeError::ValueOutOfRange {
                        name: "value",
                        value: n as f32,
                        min: f32::NEG_INFINITY,
                        max: f32::INFINITY,
                    }
                ));
            }
            Ok(crate::bridge::BridgeParamValue::Number(n))
        }
        ParamValueInput::Bool(b) => Ok(crate::bridge::BridgeParamValue::Bool(b)),
        ParamValueInput::Choice(s) => Ok(crate::bridge::BridgeParamValue::Choice(s)),
    }
}

/// Convert a [`McpBridgeError`] into the MCP [`ErrorData`] type used by tool handlers.
fn mcp_err(e: McpBridgeError) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// Distill a full `analyze_note` render into the compact go/no-go verdict
/// returned by `validate_instrument_audio`.
fn distill_audio_validation(
    r: &crate::types::AnalyzeNoteResult,
) -> crate::types::ValidateInstrumentAudioResult {
    // Audible floor ≈ -80 dBFS; "very quiet" advisory below ≈ -40 dBFS.
    const SILENCE_FLOOR: f32 = 1.0e-4;
    const QUIET_FLOOR: f32 = 0.01;

    let is_audible = r.peak_amplitude > SILENCE_FLOOR;
    let peak_dbfs = if r.peak_amplitude > 0.0 {
        20.0 * r.peak_amplitude.log10()
    } else {
        -200.0
    };
    let clipped = r.clipped_samples > 0;

    let mut warnings = Vec::new();
    if !is_audible {
        warnings.push("patch produced no audible signal (silent)".to_string());
    } else if r.peak_amplitude < QUIET_FLOOR {
        warnings.push(format!("very quiet: peak {peak_dbfs:.1} dBFS"));
    }
    if clipped {
        warnings.push(format!(
            "clipping: {} samples at fullscale",
            r.clipped_samples
        ));
    }
    if r.dc_offset.abs() > 0.01 {
        warnings.push(format!(
            "DC offset {:.3} — patch may lack a DC blocker",
            r.dc_offset
        ));
    }
    if is_audible && r.fundamental_hz > 0.0 && r.pitch_error_cents.abs() > 50.0 {
        warnings.push(format!(
            "pitch off by {:.0} cents from concert pitch",
            r.pitch_error_cents
        ));
    }

    let verdict = if !is_audible {
        "SILENT — no audio produced".to_string()
    } else if clipped {
        format!("audible but CLIPPING (peak {peak_dbfs:.1} dBFS)")
    } else if warnings.is_empty() {
        format!("OK — audible, clean (peak {peak_dbfs:.1} dBFS)")
    } else {
        format!("audible with advisories (peak {peak_dbfs:.1} dBFS)")
    };

    crate::types::ValidateInstrumentAudioResult {
        is_audible,
        verdict,
        peak_amplitude: r.peak_amplitude,
        peak_dbfs,
        rms_overall: r.rms_overall,
        clipped,
        clipped_samples: r.clipped_samples,
        fundamental_hz: r.fundamental_hz,
        pitch_error_cents: r.pitch_error_cents,
        dc_offset: r.dc_offset,
        note_played: r.note_played,
        warnings,
    }
}

/// Valid port signal type strings.
const VALID_SIGNAL_TYPES: &[&str] = &["audio", "control", "gate", "midi"];

/// Find strings similar to `needle` in `haystack` using simple edit distance.
fn find_similar(needle: &str, haystack: &[&str], max_results: usize) -> Vec<String> {
    let needle_lower = needle.to_lowercase();
    let mut scored: Vec<(&str, usize)> = haystack
        .iter()
        .filter_map(|&s| {
            let s_lower = s.to_lowercase();
            // Substring match gets priority
            if s_lower.contains(&needle_lower) || needle_lower.contains(&s_lower) {
                Some((s, 0))
            } else {
                let dist = edit_distance(&needle_lower, &s_lower);
                if dist <= 3 { Some((s, dist)) } else { None }
            }
        })
        .collect();
    scored.sort_by_key(|(_, d)| *d);
    scored
        .into_iter()
        .take(max_results)
        .map(|(s, _)| format!("'{s}'"))
        .collect()
}

/// Simple Levenshtein edit distance.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        dp[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[a.len()][b.len()]
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
        let count = self.session_count.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::info!(
            session_id = id,
            active_sessions = count,
            "MCP session opened (awaiting initialize)"
        );
        id
    }

    /// Fill in the client info for a session after the initialize handshake.
    fn set_client_info(&self, id: u64, info: McpSessionInfo) {
        tracing::info!(
            session_id = id,
            client = %info.client_name,
            client_version = %info.client_version,
            protocol = %info.protocol_version,
            "MCP client initialized"
        );
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.push(info);
        }
    }

    /// Remove a session when the client disconnects.
    fn unregister(&self, id: u64) {
        let prev = self.session_count.fetch_sub(1, Ordering::Relaxed);
        let active = prev.saturating_sub(1);
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.retain(|s| s.id != id);
        }
        tracing::info!(
            session_id = id,
            active_sessions = active,
            "MCP session closed"
        );
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

// === Discovery parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetModuleTypeInfoParam {
    #[schemars(
        description = "Module type key, e.g. 'osc', 'flt', 'env', 'lfo'. Use list_module_types to see all keys."
    )]
    pub type_key: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchModulesParam {
    #[schemars(description = "Filter by category: 'voice', 'effect', or 'visualizer'")]
    pub category: Option<String>,
    #[schemars(
        description = "Filter to modules that have an input port of this signal type: 'audio', 'control', 'gate', or 'midi'"
    )]
    pub has_input_type: Option<String>,
    #[schemars(
        description = "Filter to modules that have an output port of this signal type: 'audio', 'control', 'gate', or 'midi'"
    )]
    pub has_output_type: Option<String>,
    #[schemars(
        description = "Text search in module name, description, and parameter names (case-insensitive)"
    )]
    pub query: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListModuleTypesParam {
    #[schemars(
        description = "If true, return a compact list of just {type_key, name, category} per \
        module type instead of the full port/parameter catalog (which is hundreds of KB and can \
        exceed the tool-result token cap). Use brief to pick a type_key, then get_module_type_info \
        for that one type's full details. Default false."
    )]
    pub brief: Option<bool>,
}

/// Single from/to port pair for the `check_connection` validator.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CheckConnectionParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Source module ID, e.g. 'osc-1'")]
    pub from_module: String,
    #[schemars(description = "Source port name, e.g. 'out' ('output' is accepted as an alias)")]
    pub from_port: String,
    #[schemars(description = "Destination module ID, e.g. 'flt-1'")]
    pub to_module: String,
    #[schemars(description = "Destination port name, e.g. 'in' ('input' is accepted as an alias)")]
    pub to_port: String,
}

// === Batch execute parameter structs ===

/// A single operation in a `batch_execute` call.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchOperation {
    #[schemars(
        description = "Tool name to call, e.g. 'set_parameter', 'connect', 'set_song_tempo'"
    )]
    pub tool: String,
    #[schemars(description = "Parameters for the tool as a JSON object")]
    pub params: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchExecuteParam {
    #[schemars(
        description = "Array of operations to execute sequentially. Each has a 'tool' name and 'params' object."
    )]
    pub operations: Vec<BatchOperation>,
    #[schemars(
        description = "If true, stop on the first error. If false (default), continue and report all errors."
    )]
    pub stop_on_error: Option<bool>,
    #[schemars(
        description = "If true, validate every operation (tool name known + params parse) WITHOUT executing any of them, then report per-op validity. Nothing is mutated. Use this to pre-flight a batch before running it. Default false. dry_run takes precedence over rollback."
    )]
    pub dry_run: Option<bool>,
    #[schemars(
        description = "If true, snapshot the project before the batch and, if ANY operation fails, restore that snapshot so a failed batch is undone. Implies stop-on-error. Restore covers instruments, modules, connections, effects, and the song; it does NOT restore transport/playhead position, and sample data deleted mid-batch cannot be restored. Concurrent rollback batches are not supported (a second one errors). Default false. Ignored when dry_run is set."
    )]
    pub rollback: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentIdParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteInstrumentsParam {
    #[schemars(
        description = "Instrument IDs to delete (one or many). The default instrument (ID 0) cannot be deleted."
    )]
    pub instrument_ids: Vec<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ModuleParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Module ID string, e.g. 'osc-1', 'filter-1'")]
    pub module_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetParameterParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Module ID string, e.g. 'osc-1'")]
    pub module_id: String,
    #[schemars(description = "Parameter name, e.g. 'frequency', 'resonance'")]
    pub param_name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModMatrixScriptParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Mod Matrix module ID, e.g. 'mmx-1'")]
    pub module_id: String,
    #[schemars(description = "1-based slot number (1..=16), matching get_mod_matrix_routings")]
    pub slot: u8,
    #[schemars(
        description = "YAMS control-script source (e.g. 'src lfo = lfo-1.out\\nout = lfo * velocity'). The script's `out` becomes the slot's offset, replacing amount × source. An empty string clears the slot back to scalar behaviour."
    )]
    pub source: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOnInput {
    #[schemars(
        description = "MIDI note number (0-127, where 60 = middle C)",
        range(min = 0, max = 127)
    )]
    pub note: u8,
    #[schemars(
        description = "Velocity (0-127, where 127 = maximum)",
        range(min = 0, max = 127)
    )]
    pub velocity: u8,
    #[schemars(
        description = "MIDI channel (1-16, default 1)",
        range(min = 1, max = 16)
    )]
    pub channel: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOnParam {
    #[schemars(description = "Notes to trigger on (one or many — e.g. a whole chord in one call)")]
    pub notes: Vec<NoteOnInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOffInput {
    #[schemars(description = "MIDI note number (0-127)", range(min = 0, max = 127))]
    pub note: u8,
    #[schemars(
        description = "MIDI channel (1-16, default 1)",
        range(min = 1, max = 16)
    )]
    pub channel: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOffParam {
    #[schemars(description = "Notes to release (one or many)")]
    pub notes: Vec<NoteOffInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewNoteParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "MIDI note number (0-127, where 60 = middle C)",
        range(min = 0, max = 127)
    )]
    pub note: u8,
    #[schemars(
        description = "Velocity (0-127, where 127 = maximum)",
        range(min = 0, max = 127)
    )]
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
pub struct ValidateInstrumentAudioParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "MIDI note to test (default 60 = middle C)",
        range(min = 0, max = 127)
    )]
    pub note: Option<u8>,
    #[schemars(
        description = "Velocity (0-127, default 100)",
        range(min = 0, max = 127)
    )]
    pub velocity: Option<u8>,
    #[schemars(description = "Note hold time in milliseconds (default 500)")]
    pub duration_ms: Option<u32>,
    #[schemars(description = "Tail time after note-off in milliseconds (default 500)")]
    pub tail_ms: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeMixBusParam {
    #[schemars(
        description = "How many seconds of the master bus to render and analyze (default 10.0, max 300.0)."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "Absolute tick to start rendering from (default 0 = song beginning)."
    )]
    pub start_tick: Option<u64>,
    #[schemars(
        description = "When true, return a per-track contribution breakdown (peak, RMS, LUFS, banded energy, clipped sample count, and RMS share) for every audible track whose placements overlap the rendered window — the same breakdown analyze_section provides, but for a duration window from start_tick rather than an explicit [start,end) range. Costs one extra offline render per track, so leave off for fast master-only analysis. Default false."
    )]
    pub include_per_track: Option<bool>,
    #[schemars(
        description = "Include the full signal chain (master effects + return-bus effects) in the offline render. Shortcut for turning on every include_* flag below. Default false = dry instrument sum (per-instrument effects only), matching what the analysis has historically rendered."
    )]
    pub include_all: Option<bool>,
    #[schemars(
        description = "Load the master effect chain (master-bus limiter/EQ/compressor, etc.) into the offline render so the metrics reflect the processed master output rather than the raw instrument sum. Default false."
    )]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Load each return bus's effect chain (send/return reverbs, delays, …) into the offline render. When false the return busses are summed dry. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution: 'draft' (22.05 kHz, ~2x faster per render) or 'full' (44.1 kHz, default). Draft speeds up the render(s) — which compounds across per-track analyses — but its 11 kHz Nyquist truncates the 'high' energy band, weakens true_peak, biases LUFS (the K-weighting filters are tuned for 44.1 kHz), and aliases distortion-heavy patches. Use 'draft' only for quick level/balance/RMS passes; use 'full' when LUFS accuracy, high-frequency content, true peak, or saturation behavior matters. Unrecognized values fall back to 'full'."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenderToWavParam {
    #[schemars(
        description = "Absolute filesystem path to write the WAV file to (e.g. '/tmp/candidate.wav'). Written as 32-bit float, stereo, overwriting any existing file."
    )]
    pub path: String,
    #[schemars(
        description = "How many seconds of the arrangement to render (default 10.0, max 300.0)."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "Absolute tick to start rendering from (default 0 = song beginning)."
    )]
    pub start_tick: Option<u64>,
    #[schemars(
        description = "When set, solo only this instrument's tracks so the file contains that one instrument's contribution — a clean single-source fingerprint for external spectral matching. Omit for the full mix. Done against a clone, so your project's solo state is untouched."
    )]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(
        description = "Include the full signal chain (master effects + return-bus effects) in the render. Shortcut for every include_* flag below. Default false = dry instrument sum (per-instrument effects only)."
    )]
    pub include_all: Option<bool>,
    #[schemars(description = "Load the master effect chain into the render. Default false.")]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Load each return bus's effect chain into the render. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution: 'draft' (22.05 kHz) or 'full' (44.1 kHz, default). Use 'full' for spectral fingerprinting — 'draft' truncates everything above 11 kHz. Unrecognized values fall back to 'full'."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeSpectrumParam {
    #[schemars(
        description = "How many seconds of the arrangement to render and analyze (default 10.0, max 300.0). For a steady single-note fingerprint, solo the instrument and use a short window."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "Absolute tick to start rendering from (default 0 = song beginning)."
    )]
    pub start_tick: Option<u64>,
    #[schemars(
        description = "When set, solo only this instrument's tracks so the spectrum is that one instrument's contribution — a clean single-source fingerprint. Omit for the full mix. Done against a clone, so your project's solo state is untouched."
    )]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(
        description = "Approximate fundamental frequency in Hz. Restricts the pitch tracker's search to a fifth either side, killing octave errors and sharpening harmonic tagging (each partial's harmonic number + cents deviation). Optional."
    )]
    pub f0_hint: Option<f32>,
    #[schemars(
        description = "Maximum number of detected partials to return, descending by amplitude (default 48)."
    )]
    pub max_partials: Option<u32>,
    #[schemars(
        description = "When > 0, also return that many log-spaced magnitude bins (dB) spanning ~20 Hz to Nyquist. Needed for compare_spectra's broadband log-spectral distance. Default 0 = off."
    )]
    pub log_bins: Option<u32>,
    #[schemars(
        description = "Include the full signal chain (master effects + return-bus effects) in the render. Shortcut for every include_* flag below. Default false = dry instrument sum."
    )]
    pub include_all: Option<bool>,
    #[schemars(description = "Load the master effect chain into the render. Default false.")]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Load each return bus's effect chain into the render. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution: 'draft' (22.05 kHz) or 'full' (44.1 kHz, default). Use 'full' for spectral work — 'draft' truncates everything above 11 kHz. Unrecognized values fall back to 'full'."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeSpectrogramParam {
    #[schemars(
        description = "How many seconds of the arrangement to render and analyze (default 10.0, max 300.0)."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "Absolute tick to start rendering from (default 0 = song beginning)."
    )]
    pub start_tick: Option<u64>,
    #[schemars(
        description = "When set, solo only this instrument's tracks. Omit for the full mix. Done against a clone, so your project's solo state is untouched."
    )]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(
        description = "Approximate fundamental frequency in Hz, applied to every frame to sharpen harmonic tagging. Optional."
    )]
    pub f0_hint: Option<f32>,
    #[schemars(description = "Maximum partials per frame (default 48).")]
    pub max_partials: Option<u32>,
    #[schemars(
        description = "When > 0, add that many log-spaced magnitude bins (dB) per frame. Default 0 = off."
    )]
    pub log_bins: Option<u32>,
    #[schemars(
        description = "Hop between frame centres in milliseconds (default 20 ≈ one PAL video frame, the rate a SID voice switches waveform). Smaller = finer time resolution, more frames (capped at 4096)."
    )]
    pub hop_ms: Option<f32>,
    #[schemars(
        description = "Analysed window length per frame in milliseconds (default 40). Longer windows resolve closer partials but blur fast changes."
    )]
    pub window_len_ms: Option<f32>,
    #[schemars(
        description = "Include the full signal chain (master + return effects) in the render. Shortcut for the include_* flags. Default false = dry instrument sum."
    )]
    pub include_all: Option<bool>,
    #[schemars(description = "Load the master effect chain into the render. Default false.")]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Load each return bus's effect chain into the render. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution: 'draft' (22.05 kHz) or 'full' (44.1 kHz, default). Use 'full' for spectral work."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeSampleSpectrumParam {
    #[schemars(
        description = "Either a numeric imported-sample id (as returned by import_sample / list_samples) or a filesystem path to a WAV file. A bare integer is treated as a sample id first; to force a numeric-named file use a path form like './5.wav'. The sample is decoded at its native rate and downmixed to mono for analysis."
    )]
    pub sample_id_or_path: String,
    #[schemars(
        description = "Approximate fundamental frequency in Hz. Restricts the pitch tracker's search to a fifth either side, sharpening harmonic tagging. Optional."
    )]
    pub f0_hint: Option<f32>,
    #[schemars(
        description = "Maximum number of detected partials to return, descending by amplitude (default 48)."
    )]
    pub max_partials: Option<u32>,
    #[schemars(
        description = "When > 0, also return that many log-spaced magnitude bins (dB). Needed for compare_spectra's broadband log-spectral distance. Default 0 = off."
    )]
    pub log_bins: Option<u32>,
    #[schemars(
        description = "Offset into the decoded sample to start analysing, in milliseconds (default 0). Combine with window_len_ms to analyse a single frame of a time-varying sound — slide it in small (~5 ms) steps to land on a specific voiced/unvoiced frame."
    )]
    pub start_ms: Option<f32>,
    #[schemars(
        description = "Length of the analysis window in milliseconds. Slices the sample to [start_ms, start_ms+window_len_ms); a window past the end is zero-padded, and a start past the end is an error. Omit to analyse from start_ms to the end of the sample."
    )]
    pub window_len_ms: Option<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeSampleSpectrogramParam {
    #[schemars(
        description = "Either a numeric imported-sample id or a filesystem path to a WAV file. The audio is decoded at its native rate and downmixed to mono for analysis."
    )]
    pub sample_id_or_path: String,
    #[schemars(
        description = "Approximate fundamental frequency in Hz, applied to every frame to sharpen harmonic tagging. Optional."
    )]
    pub f0_hint: Option<f32>,
    #[schemars(description = "Maximum partials per frame (default 48).")]
    pub max_partials: Option<u32>,
    #[schemars(
        description = "When > 0, add that many log-spaced magnitude bins (dB) per frame. Default 0 = off."
    )]
    pub log_bins: Option<u32>,
    #[schemars(
        description = "Hop between frame centres in milliseconds (default 20 ≈ one PAL video frame, the rate a SID voice switches waveform). Smaller = finer time resolution, more frames (capped at 4096)."
    )]
    pub hop_ms: Option<f32>,
    #[schemars(
        description = "Analysed window length per frame in milliseconds (default 40). Longer windows resolve closer partials but blur fast changes."
    )]
    pub window_len_ms: Option<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SpectrumSourceParam {
    #[schemars(
        description = "Set to analyse an imported-sample id or a WAV file path instead of a render. When present this source is a sample; when omitted it is an offline render (see the render-only fields below)."
    )]
    pub sample_id_or_path: Option<String>,
    #[schemars(
        description = "(Render source only) solo this instrument so the source is that one instrument's contribution. Omit for the full mix."
    )]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(description = "(Render source only) absolute start tick (default 0).")]
    pub start_tick: Option<u64>,
    #[schemars(
        description = "(Render source only) how many seconds to render (default 10.0, max 300.0)."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "(Sample source only) offset into the decoded sample to start analysing, in ms (default 0). A render source addresses its window via start_tick instead. Use with window_len_ms to align one voiced frame against the other side."
    )]
    pub start_ms: Option<f32>,
    #[schemars(
        description = "Analysis window length in ms. For a sample source, slices the buffer (zero-padded past the end). For a render source, overrides duration_seconds. Set the SAME window on both sources to compare one frame against one frame — essential for time-varying targets (e.g. a SID voice switching waveform every ~20 ms), where you slide start_ms/start_tick in small steps to find the matching frame."
    )]
    pub window_len_ms: Option<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CompareSpectraParam {
    #[schemars(
        description = "The reference spectrum (e.g. a real SID render imported as a sample, or a WAV path). missing_partials lists what this target has that the candidate lacks."
    )]
    pub target: SpectrumSourceParam,
    #[schemars(
        description = "The candidate spectrum being matched against the target (e.g. your patch, as a soloed render)."
    )]
    pub candidate: SpectrumSourceParam,
    #[schemars(
        description = "Approximate fundamental frequency in Hz, applied to BOTH sources to sharpen harmonic tagging. Optional."
    )]
    pub f0_hint: Option<f32>,
    #[schemars(description = "Maximum partials per source for the partial diff (default 48).")]
    pub max_partials: Option<u32>,
    #[schemars(
        description = "Log-spaced bins per source for the broadband log_spectral_distance (default 128; forced on so the distance is always available)."
    )]
    pub log_bins: Option<u32>,
    #[schemars(
        description = "Log-mel filterbank bands per source for the perceptual mel_l2_distance (default 40 ≈ the standard MFCC filterbank; forced on so the distance is always available). More bands = finer perceptual resolution and a larger L2 magnitude; keep it fixed across a matching run so values stay comparable."
    )]
    pub mel_bands: Option<u32>,
    #[schemars(
        description = "Include the full signal chain (master + return effects) in any render source. Shortcut for the include_* flags. Default false = dry instrument sum."
    )]
    pub include_all: Option<bool>,
    #[schemars(description = "Load the master effect chain into render sources. Default false.")]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Load each return bus's effect chain into render sources. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution for render sources: 'draft' (22.05 kHz) or 'full' (44.1 kHz, default). Use 'full' for spectral work."
    )]
    pub render_quality: Option<String>,
    #[schemars(
        description = "Turn on the time-resolved (per-frame) distance. Default false = aggregate only. Set true for staccato / silence-dominated / time-varying material (e.g. a SID release tail): the aggregate distances average over the whole window and go blind to quiet-in-time content, while the framed path scores each frame on its own and ranks candidates. Adds time_resolved_lsd / time_resolved_mel_l2 / frames_compared / frames_masked / alignment_offset_ms / worst_frames to the result."
    )]
    pub time_resolved: Option<bool>,
    #[schemars(
        description = "(time_resolved only) frame hop in ms (default 20). Smaller = finer time resolution, more frames (capped at 4096 with a truncation warning)."
    )]
    pub hop_ms: Option<f32>,
    #[schemars(description = "(time_resolved only) analysed frame length in ms (default 40).")]
    pub frame_len_ms: Option<f32>,
    #[schemars(
        description = "(time_resolved only) which frames to score: 'target_energy' (default — compare only frames where the TARGET has energy, so silence/decay never averages in) or 'none' (compare every paired frame)."
    )]
    pub mask: Option<String>,
    #[schemars(
        description = "(time_resolved only) pre-alignment: 'envelope' (default — cross-correlate the two RMS envelopes and shift the candidate so onsets line up before framing; essential for staccato material where a ±60 ms offset flips the ranking) or 'none' (pair frames from sample 0). The chosen shift is reported as alignment_offset_ms."
    )]
    pub align: Option<String>,
    #[schemars(
        description = "(time_resolved only) maximum envelope-alignment search in ms (default 250). Ignored when align is 'none'."
    )]
    pub align_max_ms: Option<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CompareEnvelopesParam {
    #[schemars(
        description = "The reference contour (e.g. a real SID render imported as a sample, or a WAV path). Deltas are reported as candidate − target."
    )]
    pub target: SpectrumSourceParam,
    #[schemars(
        description = "The candidate contour being matched against the target (e.g. your patch, as a soloed render)."
    )]
    pub candidate: SpectrumSourceParam,
    #[schemars(
        description = "RMS-envelope block size in ms (default 5). Smaller resolves a faster attack but grows the DTW matrix; contours over 2048 windows are strided down for the warp (a warning is added)."
    )]
    pub envelope_window_ms: Option<f32>,
    #[schemars(
        description = "Held-note duration in ms, applied to BOTH sources to mark note-off for the release estimate. Omit if you don't know it (release_ms is then 0 and the whole buffer is treated as held); the DTW shape distance and attack/decay/sustain are unaffected."
    )]
    pub note_duration_ms: Option<u32>,
    #[schemars(
        description = "Attack-transient span in ms (default 20). The crest factor and energy-rise 'punch' metrics are measured over the first this-many ms of each source, so align the source start with the note onset (via start_tick / start_ms)."
    )]
    pub transient_window_ms: Option<f32>,
    #[schemars(
        description = "Include the full signal chain (master + return effects) in any render source. Shortcut for the include_* flags. Default false = dry instrument sum."
    )]
    pub include_all: Option<bool>,
    #[schemars(description = "Load the master effect chain into render sources. Default false.")]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Load each return bus's effect chain into render sources. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution for render sources: 'draft' (22.05 kHz) or 'full' (44.1 kHz, default)."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeMasterChainParam {
    #[schemars(
        description = "How many seconds of the master bus to render and analyze per stage (default 10.0, max 300.0)."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "Absolute tick to start rendering from (default 0 = song beginning)."
    )]
    pub start_tick: Option<u64>,
    #[schemars(
        description = "Feed the return-bus wet signal (send/return reverbs, delays, …) into the master-chain input. When false the return busses are summed dry. The master effect chain itself is ALWAYS measured regardless of this flag — it is the subject of the analysis. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution: 'draft' (22.05 kHz, ~2x faster per render) or 'full' (44.1 kHz, default). Draft compounds across the per-effect renders but truncates the 'high' band, weakens true_peak, and biases LUFS. Use 'full' when loudness/peak/saturation accuracy matters. Unrecognized values fall back to 'full'."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeReturnBussesParam {
    #[schemars(
        description = "How many seconds of the master bus to render and analyze (default 10.0, max 300.0)."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "Absolute tick to start rendering from (default 0 = song beginning)."
    )]
    pub start_tick: Option<u64>,
    #[schemars(
        description = "Also load the master effect chain so each return's contribution is measured through the processed master output rather than the raw pre-master sum. The return-bus effect chains themselves are ALWAYS reconstructed — they are the subject of the analysis. Default false."
    )]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Render resolution: 'draft' (22.05 kHz, ~2x faster per render) or 'full' (44.1 kHz, default). Draft compounds across the per-return renders but truncates the 'high' band, weakens true_peak, and biases LUFS. Use 'full' when loudness/peak accuracy matters. Unrecognized values fall back to 'full'."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CompareMixBeforeAfterParam {
    #[schemars(
        description = "Either 'capture' (render the mix now and store it as the baseline) or 'compare' (re-render and report current − baseline deltas). Capture first, make your change, then compare."
    )]
    pub action: String,
    #[schemars(
        description = "Capture only: how many seconds of the master bus to render (default 10.0, max 300.0). Ignored on compare, which reuses the baseline's window."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "Capture only: absolute tick to start rendering from (default 0). Ignored on compare."
    )]
    pub start_tick: Option<u64>,
    #[schemars(
        description = "Capture only: a label for the baseline (e.g. 'before EQ'). Defaults to 'baseline'. Ignored on compare."
    )]
    pub label: Option<String>,
    #[schemars(
        description = "Capture only: include the full signal chain (master + return effects) in the render. Shortcut for every include_* flag. Default false. The scope is stored and reused on compare."
    )]
    pub include_all: Option<bool>,
    #[schemars(description = "Capture only: load the master effect chain. Default false.")]
    pub include_master_effects: Option<bool>,
    #[schemars(description = "Capture only: load each return bus's effect chain. Default false.")]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Capture only: render resolution, 'draft' (22.05 kHz) or 'full' (44.1 kHz, default). Unrecognized values fall back to 'full'."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AutoGainStageParam {
    #[schemars(
        description = "Target integrated loudness in LUFS (e.g. -18, -14, -23). The master fader is adjusted to bring the measured loudness to this value."
    )]
    pub target_lufs: f32,
    #[schemars(
        description = "True-peak ceiling in dBTP that must not be breached (default -1.0). If hitting target_lufs would push the true peak above this, the gain is reduced and `limited_by` reports 'true_peak_ceiling'."
    )]
    pub true_peak_ceiling: Option<f32>,
    #[schemars(
        description = "How many seconds of the master bus to render and measure (default 10.0, max 300.0). Measured through the master + return effect chains at 44.1 kHz."
    )]
    pub duration_seconds: Option<f32>,
    #[schemars(
        description = "Absolute tick to start measuring from (default 0 = song beginning)."
    )]
    pub start_tick: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeSectionParam {
    #[schemars(description = "Absolute tick where the section starts (inclusive).")]
    pub start_tick: u64,
    #[schemars(description = "Absolute tick where the section ends (exclusive).")]
    pub end_tick: u64,
    #[schemars(
        description = "When true, return a per-track contribution breakdown (peak, RMS, LUFS, banded energy, clipped sample count, and RMS share) for every audible track whose placements overlap the section. Costs one extra offline render per track, so leave off for fast master-only analysis and turn on when investigating which track is clipping, dominating, or masking. Default false."
    )]
    pub include_per_track: Option<bool>,
    #[schemars(
        description = "Include the full signal chain (master effects + return-bus effects) in the offline render. Shortcut for every include_* flag below. Default false = dry instrument sum (per-instrument effects only)."
    )]
    pub include_all: Option<bool>,
    #[schemars(
        description = "Load the master effect chain into the offline render so the metrics reflect the processed master output. Default false."
    )]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Load each return bus's effect chain into the offline render. When false the return busses are summed dry. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution: 'draft' (22.05 kHz, ~2x faster per render) or 'full' (44.1 kHz, default). Draft speeds up the render(s) — which compounds across per-track analyses — but its 11 kHz Nyquist truncates the 'high' energy band, weakens true_peak, biases LUFS (the K-weighting filters are tuned for 44.1 kHz), and aliases distortion-heavy patches. Use 'draft' only for quick level/balance/RMS passes; use 'full' when LUFS accuracy, high-frequency content, true peak, or saturation behavior matters. Unrecognized values fall back to 'full'."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeMaskingMatrixParam {
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0 (song beginning)."
    )]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Maximum pairs to return, sorted by descending conflict_score (default 20, clamped to [1, 200]). The pair matrix is O(N²) in audible-track count, so the full list explodes the response size — only the top conflicts are usually actionable. `total_pair_count` in the result still reflects the full pair count."
    )]
    pub top_pairs: Option<u32>,
    #[schemars(
        description = "Include the full signal chain (master effects + return-bus effects) in the per-track offline renders. Shortcut for every include_* flag below. Default false = dry instrument sum (per-instrument effects only)."
    )]
    pub include_all: Option<bool>,
    #[schemars(
        description = "Load the master effect chain into the per-track offline renders. Default false."
    )]
    pub include_master_effects: Option<bool>,
    #[schemars(
        description = "Load each return bus's effect chain into the per-track offline renders. When false the return busses are summed dry. Default false."
    )]
    pub include_return_effects: Option<bool>,
    #[schemars(
        description = "Render resolution: 'draft' (22.05 kHz, ~2x faster per render) or 'full' (44.1 kHz, default). Draft speeds up the render(s) — which compounds across per-track analyses — but its 11 kHz Nyquist truncates the 'high' energy band, weakens true_peak, biases LUFS (the K-weighting filters are tuned for 44.1 kHz), and aliases distortion-heavy patches. Use 'draft' only for quick level/balance/RMS passes; use 'full' when LUFS accuracy, high-frequency content, true peak, or saturation behavior matters. Unrecognized values fall back to 'full'."
    )]
    pub render_quality: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeHarmonyParam {
    #[schemars(
        description = "Pattern ID to analyze. When set, the arrangement_* fields are ignored and analysis runs on that pattern's notes in pattern-relative ticks. Leave unset to analyze the arrangement instead."
    )]
    pub pattern_id: Option<u32>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0 (song beginning). Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Chord-detection window size in ticks (default 960 = one quarter note at 960 PPQN). Smaller values produce more, finer chord events; larger values aggregate."
    )]
    pub grouping_ticks: Option<u32>,
    #[schemars(
        description = "Exclude notes from tracks whose instrument has category 'Drums' (default true). Percussion MIDI pitches otherwise pollute chord identification — a hi-hat at MIDI 42 (F#) on top of an Am pad gets reported as F#m7b5. Has no effect in pattern scope. Set false to include all tracks regardless of category."
    )]
    pub exclude_drums: Option<bool>,
    #[schemars(
        description = "Explicit list of track IDs to exclude from chord identification, e.g. for tracks with category 'Uncategorized' that are nonetheless percussion. Combined with exclude_drums. Arrangement scope only."
    )]
    pub exclude_track_ids: Option<Vec<u16>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzePatternParam {
    #[schemars(description = "Pattern ID to analyze.")]
    pub pattern_id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeInstrumentRangeParam {
    #[schemars(description = "Instrument ID to sweep.")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "Lowest MIDI note in the sweep (0-127, inclusive).",
        range(min = 0, max = 127)
    )]
    pub low_note: u8,
    #[schemars(
        description = "Highest MIDI note in the sweep (0-127, inclusive).",
        range(min = 0, max = 127)
    )]
    pub high_note: u8,
    #[schemars(
        description = "Semitone gap between consecutive sweep steps (default 12 = one note per octave). Smaller values increase resolution at proportional render cost."
    )]
    pub step_semitones: Option<u8>,
    #[schemars(
        description = "Velocity to use for every step (1-127, default 100).",
        range(min = 1, max = 127)
    )]
    pub velocity: Option<u8>,
    #[schemars(
        description = "Note duration in ms (default 400). Held identical across steps so per-step amplitude/brightness curves stay comparable."
    )]
    pub duration_ms: Option<u32>,
    #[schemars(description = "Release tail in ms after note-off (default 200).")]
    pub tail_ms: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeVelocityResponseParam {
    #[schemars(description = "Instrument ID to test.")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "MIDI note to hold across the velocity sweep (0-127).",
        range(min = 0, max = 127)
    )]
    pub note: u8,
    #[schemars(
        description = "Lowest velocity in the sweep (1-127, default 1).",
        range(min = 1, max = 127)
    )]
    pub velocity_low: Option<u8>,
    #[schemars(
        description = "Highest velocity in the sweep (1-127, default 127).",
        range(min = 1, max = 127)
    )]
    pub velocity_high: Option<u8>,
    #[schemars(
        description = "Step size between consecutive velocities (default 16 → ~8 steps over 1-127)."
    )]
    pub velocity_step: Option<u8>,
    #[schemars(description = "Note duration in ms (default 400). Held identical across steps.")]
    pub duration_ms: Option<u32>,
    #[schemars(description = "Release tail in ms after note-off (default 200).")]
    pub tail_ms: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GenerateChordParam {
    #[schemars(
        description = "Chord symbol like 'C', 'Cm7', 'F#maj7', 'Bbsus4', 'Dm7b5', 'G7sus4', 'C5'. Accepts synonyms 'min'/'minor' (= 'm'), 'maj'/'major' (= ''), 'dim'/'aug'. Root may be uppercase or lowercase; '#' is sharp, 'b' after a letter is flat."
    )]
    pub symbol: String,
    #[schemars(
        description = "Octave in scientific pitch notation (-1 .. 9). Default 4 = middle-C octave."
    )]
    pub octave: Option<i32>,
    #[schemars(
        description = "Voicing to apply to the close-position chord. One of 'close' (default), 'drop2', 'drop3', 'open'. Voicings that need more notes than the chord has fall back to drop2 with a warning."
    )]
    pub voicing: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateChordProgressionPatternParam {
    #[schemars(description = "Name for the new pattern.")]
    pub name: String,
    #[schemars(
        description = "Chord symbols in playback order, e.g. ['Gm','F','Eb','D']. Each is parsed like generate_chord (synonyms, sharps/flats accepted)."
    )]
    pub chords: Vec<String>,
    #[schemars(
        description = "Beats each chord lasts, laid end to end (default 4.0 = one 4/4 bar). Pattern length = chords.len() * beats_per_chord."
    )]
    pub beats_per_chord: Option<f32>,
    #[schemars(
        description = "Octave in scientific pitch notation for the chord roots (default 4 = middle-C octave)."
    )]
    pub octave: Option<i32>,
    #[schemars(
        description = "Voicing applied to every chord: 'close' (default), 'drop2', 'drop3', 'open'."
    )]
    pub voicing: Option<String>,
    #[schemars(
        description = "Velocity for all placed notes (0-127, default 80).",
        range(min = 0, max = 127)
    )]
    pub velocity: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TransposeNotesParam {
    #[schemars(description = "Pattern to transpose.")]
    pub pattern_id: u32,
    #[schemars(
        description = "Signed semitone shift (e.g. +5 = up a fourth, -12 = down an octave)."
    )]
    pub semitones: i32,
    #[schemars(
        description = "Optional scale-constraint tonic (0..12, C = 0). When both scale_tonic and scale_name are set, any transposed pitch that lands off-scale is snapped to the nearest in-scale pitch."
    )]
    pub scale_tonic: Option<u8>,
    #[schemars(
        description = "Optional scale name: 'major', 'minor', 'harmonic_minor', 'melodic_minor', 'dorian', 'phrygian', 'lydian', 'mixolydian', 'locrian', 'pentatonic_major', 'pentatonic_minor', 'blues', 'chromatic'. Unknown names fall back to major."
    )]
    pub scale_name: Option<String>,
    #[schemars(
        description = "Tie-break direction for scale snap: 'up' (default — prefer higher pitch), 'down', or 'nearest'. Only meaningful when a scale is set."
    )]
    pub tie_break: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QuantizeNotesToScaleParam {
    #[schemars(description = "Pattern to operate on.")]
    pub pattern_id: u32,
    #[schemars(description = "Scale tonic pitch class (0..12, C = 0).")]
    pub scale_tonic: u8,
    #[schemars(
        description = "Scale template name (see transpose_notes for the full list). Unknown names fall back to major."
    )]
    pub scale_name: String,
    #[schemars(
        description = "Tie-break direction when two scale degrees are equidistant: 'up' (default), 'down', or 'nearest'."
    )]
    pub tie_break: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct QuantizeNotesToGridParam {
    #[schemars(description = "Pattern to operate on.")]
    pub pattern_id: u32,
    #[schemars(
        description = "Grid resolution in ticks (e.g. 240 = sixteenth at 960 PPQN, 480 = eighth, 960 = quarter). Must be > 0; pass 0 to return early without changes."
    )]
    pub grid_ticks: u32,
    #[schemars(
        description = "Quantize strength in 0..=1. 1.0 (default) = full snap, 0.5 = halfway between original and grid, 0.0 = no movement."
    )]
    pub strength: Option<f32>,
    #[schemars(
        description = "Swing amount 0..=1 (default 0). Even-indexed grid positions stay put; odd positions get pushed back by up to half the grid distance."
    )]
    pub swing: Option<f32>,
    #[schemars(
        description = "Maximum ±jitter applied per note after the grid snap, in ticks (default 0 = no humanization)."
    )]
    pub humanize_ticks: Option<u32>,
    #[schemars(
        description = "Seed for humanization (default 0). Same seed + same notes + same options → same output."
    )]
    pub humanize_seed: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeDrumGrooveParam {
    #[schemars(
        description = "Pattern ID to analyze as a drum pattern. When set, the arrangement_* fields are ignored and the analyzer treats every note in the pattern as a drum hit (no drum-track filtering)."
    )]
    pub pattern_id: Option<u32>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0. Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeHarmonicFunctionParam {
    #[schemars(
        description = "Pattern ID to analyze. When set, the arrangement_* fields are ignored and analysis runs on that pattern's notes in pattern-relative ticks. Leave unset to analyze the arrangement instead."
    )]
    pub pattern_id: Option<u32>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0 (song beginning). Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Chord-detection window size in ticks (default 960 = one quarter note at 960 PPQN, or 3840 = one bar in arrangement scope). Smaller values produce more, finer chord events; larger values aggregate."
    )]
    pub grouping_ticks: Option<u32>,
    #[schemars(
        description = "Exclude notes from tracks classified as Drums (default true), matching analyze_harmony's behaviour."
    )]
    pub exclude_drums: Option<bool>,
    #[schemars(
        description = "Explicit list of track IDs to exclude from chord identification, combined with exclude_drums. Arrangement scope only."
    )]
    pub exclude_track_ids: Option<Vec<u16>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeArrangementParam {
    #[schemars(
        description = "Pattern ID to analyze. When set, the analyzer runs over the pattern's bars (in pattern-relative ticks) instead of the arrangement. arrangement_* fields are ignored when this is set."
    )]
    pub pattern_id: Option<u32>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0. Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Cosine similarity above which two bars are treated as 'the same' for section labeling. Default 0.85 — lower for looser grouping, higher for stricter."
    )]
    pub similarity_threshold: Option<f32>,
    #[schemars(
        description = "Minimum bars a detected section must span. Sections shorter than this are merged with the longer of their neighbours. Default 2."
    )]
    pub section_min_bars: Option<u32>,
    #[schemars(
        description = "Exclude notes from tracks classified as Drums (default true). Drum tracks pollute pitch-class histograms and bias the similarity matrix. Arrangement scope only."
    )]
    pub exclude_drums: Option<bool>,
    #[schemars(
        description = "Explicit list of track IDs to exclude, combined with exclude_drums. Arrangement scope only."
    )]
    pub exclude_track_ids: Option<Vec<u16>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeFormMapParam {
    #[schemars(description = "Pattern ID. When set, the arrangement_* fields are ignored.")]
    pub pattern_id: Option<u32>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Cosine similarity above which two bars are treated as 'the same' for section labeling (default 0.85)."
    )]
    pub similarity_threshold: Option<f32>,
    #[schemars(
        description = "Minimum bars a section must span before short runs are merged (default 2)."
    )]
    pub section_min_bars: Option<u32>,
    #[schemars(
        description = "Exclude notes from tracks classified as Drums (default true). Arrangement scope only."
    )]
    pub exclude_drums: Option<bool>,
    #[schemars(
        description = "Explicit list of track IDs to exclude from feature extraction. Arrangement scope only."
    )]
    pub exclude_track_ids: Option<Vec<u16>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindMotifsParam {
    #[schemars(description = "Pattern ID. When set, arrangement_* fields are ignored.")]
    pub pattern_id: Option<u32>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Shortest motif length in intervals (= notes − 1). Default 3 = 4-note phrases. Clamped to [2, 12]."
    )]
    pub min_interval_length: Option<u8>,
    #[schemars(
        description = "Longest motif length in intervals. Default 6 = 7-note phrases. Clamped to [min_interval_length, 12]."
    )]
    pub max_interval_length: Option<u8>,
    #[schemars(
        description = "Minimum number of occurrences for a motif to be reported (default 3). Lower = more candidates, more noise."
    )]
    pub min_count: Option<u32>,
    #[schemars(
        description = "Maximum motifs returned, sorted by descending score = length × log2(1 + count). Default 10."
    )]
    pub top_n: Option<u32>,
    #[schemars(
        description = "Per-motif cap on the returned `occurrences` list (default 5, clamped to [0, 50]). The motif's `count` field is always the authoritative total; the list is just a sample of locations. Set 0 to omit the list entirely. Highly-repetitive hooks can hit 50+ occurrences and would otherwise blow up the response."
    )]
    pub max_occurrences_per_motif: Option<u32>,
    #[schemars(
        description = "Exclude drum tracks (default true). Drum hits don't have a melodic interval contour."
    )]
    pub exclude_drums: Option<bool>,
    #[schemars(description = "Explicit list of track IDs to exclude. Arrangement scope only.")]
    pub exclude_track_ids: Option<Vec<u16>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeHookStrengthParam {
    #[schemars(description = "Pattern ID. When set, arrangement_* fields are ignored.")]
    pub pattern_id: Option<u32>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Shortest motif length in intervals to consider for the hook score (default 3 = 4-note phrases). Shorter motifs are too generic to be hooks."
    )]
    pub min_interval_length: Option<u8>,
    #[schemars(
        description = "Minimum repeat count for a motif to count toward the hook score (default 3)."
    )]
    pub min_count: Option<u32>,
    #[schemars(
        description = "Per-motif cap on the returned `strongest_motif.occurrences` list (default 5, clamped to [0, 50]). The motif's `count` field is always the authoritative total. Set 0 to omit the list entirely."
    )]
    pub max_occurrences_per_motif: Option<u32>,
    #[schemars(description = "Exclude drum tracks (default true).")]
    pub exclude_drums: Option<bool>,
    #[schemars(description = "Explicit list of track IDs to exclude. Arrangement scope only.")]
    pub exclude_track_ids: Option<Vec<u16>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeTensionCurveParam {
    #[schemars(description = "Pattern ID. When set, arrangement_* fields are ignored.")]
    pub pattern_id: Option<u32>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "If true, render the scope once and slice the buffer per bar to compute loudness, brightness, band entropy, and stereo width axes. Defaults to true in arrangement scope and false in pattern scope. Audio mode costs roughly one `analyze_section` call."
    )]
    pub include_audio: Option<bool>,
    #[schemars(
        description = "Cosine-similarity threshold for the section clustering (same as `analyze_arrangement`). Default 0.85."
    )]
    pub similarity_threshold: Option<f32>,
    #[schemars(description = "Minimum section length in bars (default 2).")]
    pub section_min_bars: Option<u32>,
    #[schemars(description = "Exclude drum tracks from the melodic note stream (default true).")]
    pub exclude_drums: Option<bool>,
    #[schemars(description = "Explicit list of track IDs to exclude. Arrangement scope only.")]
    pub exclude_track_ids: Option<Vec<u16>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SuggestMusicFixesParam {
    #[schemars(description = "Pattern ID. When set, arrangement_* fields are ignored.")]
    pub pattern_id: Option<u32>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Categories to include. Subset of: 'harmony', 'mix', 'groove', 'arrangement', 'composition', 'patch'. Empty/null runs everything."
    )]
    pub categories: Option<Vec<String>>,
    #[schemars(
        description = "If true (default), run the audio-render-backed checks (mix-bus / masking / audio-augmented tension curve). Set to false for a faster pure-symbolic pass."
    )]
    pub include_audio: Option<bool>,
    #[schemars(description = "Maximum suggestions returned. Default 15. Clamped to [1, 50].")]
    pub max_suggestions: Option<u32>,
    #[schemars(description = "Exclude drum tracks from melodic-axis rules (default true).")]
    pub exclude_drums: Option<bool>,
    #[schemars(description = "Explicit list of track IDs to exclude. Arrangement scope only.")]
    pub exclude_track_ids: Option<Vec<u16>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeBassDrumLockParam {
    #[schemars(
        description = "Pattern ID to analyze. When set, the analyzer treats notes with GM kick MIDI numbers (35, 36) as kicks and everything else as bass. Useful for combined rhythm-section patterns. Ignored when arrangement_* fields are set."
    )]
    pub pattern_id: Option<u32>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0. Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<u64>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<u64>,
    #[schemars(
        description = "Maximum |Δtick| between a kick onset and a bass onset that still counts as a match. Default 120 (±1/32-note at 960 PPQN). Clamped to [30, 960]."
    )]
    pub onset_tolerance_ticks: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeNoteParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "MIDI note number (0-127, where 60 = middle C)",
        range(min = 0, max = 127)
    )]
    pub note: u8,
    #[schemars(
        description = "Velocity (0-127, where 127 = maximum)",
        range(min = 0, max = 127)
    )]
    pub velocity: u8,
    #[schemars(
        description = "Note duration in milliseconds (default 500). How long the note is held before release."
    )]
    pub duration_ms: Option<u32>,
    #[schemars(
        description = "Tail time in milliseconds after note-off (default 500). Extra time for release/reverb tails."
    )]
    pub tail_ms: Option<u32>,
    #[schemars(
        description = "Optional MIDI note for pitch-error measurement. When set, the fundamental detector restricts its search to ±tritone around this pitch so it isn't fooled by dominant sub-octaves (sub-bass) or strong upper harmonics (wave-folded patches). Defaults to the actually-played note (`note` shifted by the patch's octave_offset).",
        range(min = 0, max = 127)
    )]
    pub expected_note: Option<u8>,
    #[schemars(
        description = "RMS/centroid envelope block size in milliseconds (default 50). Lower it (e.g. 2–5 ms) to resolve fast attacks/decays that the default window collapses into one frame (a sub-window attack otherwise reports attack_ms = 0); raise it to smooth noisy contours. Clamped to [1, 5000].",
        range(min = 1, max = 5000)
    )]
    pub envelope_window_ms: Option<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct LoadExamplePatchParam {
    #[schemars(
        description = "Name of the example patch to load (case-insensitive), e.g. 'Acid Bass', 'Grand Piano'"
    )]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddModulesParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "Module types to add (one or many). Each accepts the short type key from list_module_types (e.g. 'osc', 'flt', 'amp'), the full name in snake_case (e.g. 'oscillator', 'ladder_filter'), or the display name (e.g. 'Oscillator', 'Ladder Filter'). Case-insensitive."
    )]
    pub module_types: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveModulesParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Module IDs to remove (one or many), e.g. ['osc-1', 'flt-2']")]
    pub module_ids: Vec<String>,
}

/// A single connection in a batch connect/disconnect call.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectionInput {
    #[schemars(description = "Source module ID, e.g. 'osc-1'")]
    pub from_module: String,
    #[schemars(description = "Source port name, e.g. 'out' ('output' is accepted as an alias)")]
    pub from_port: String,
    #[schemars(description = "Destination module ID, e.g. 'flt-1'")]
    pub to_module: String,
    #[schemars(description = "Destination port name, e.g. 'in' ('input' is accepted as an alias)")]
    pub to_port: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectMultipleParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Array of connections to make")]
    pub connections: Vec<ConnectionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InsertModuleBetweenParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "Type of module to insert (short key like 'flt', snake_case name, or display name). Must carry audio (have an audio input and output) — a pure modulator like an LFO is rejected."
    )]
    pub module_type: String,
    #[schemars(
        description = "Anchor: splice the outgoing audio cable of this module id, e.g. 'osc-1'. The new module goes right after it."
    )]
    pub after: Option<String>,
    #[schemars(
        description = "Anchor: splice the incoming audio cable of this module id, e.g. 'amp-1'. The new module goes right before it."
    )]
    pub before: Option<String>,
    #[schemars(
        description = "Anchor: like `after`, but addresses the single module of this type (e.g. 'osc'). Robust across instruments with differing instance numbers; errors if the type is ambiguous."
    )]
    pub after_type: Option<String>,
    #[schemars(
        description = "Anchor: like `before`, but addresses the single module of this type (e.g. 'amp'). Errors if the type is ambiguous."
    )]
    pub before_type: Option<String>,
    #[schemars(
        description = "Explicit-cable anchor (unambiguous escape hatch when the audio path branches): source module id. Requires from_port/to_module/to_port too."
    )]
    pub from_module: Option<String>,
    #[schemars(description = "Explicit-cable anchor: source port name (with from_module).")]
    pub from_port: Option<String>,
    #[schemars(description = "Explicit-cable anchor: destination module id (with from_module).")]
    pub to_module: Option<String>,
    #[schemars(description = "Explicit-cable anchor: destination port name (with from_module).")]
    pub to_port: Option<String>,
}

impl InsertModuleBetweenParam {
    /// Resolve the supplied fields into exactly one [`InsertAnchor`]. At most
    /// one anchor mode may be set; none defaults to "before the output module".
    fn resolve_anchor(&self) -> Result<crate::bridge::InsertAnchor, String> {
        use crate::bridge::InsertAnchor;

        let explicit = self.from_module.is_some()
            || self.from_port.is_some()
            || self.to_module.is_some()
            || self.to_port.is_some();
        let named = [
            &self.after,
            &self.before,
            &self.after_type,
            &self.before_type,
        ]
        .iter()
        .filter(|a| a.is_some())
        .count();

        if named + usize::from(explicit) > 1 {
            return Err(
                "specify at most one anchor: one of after / before / after_type / before_type, \
                 or the explicit from_module+from_port+to_module+to_port cable"
                    .to_string(),
            );
        }

        if explicit {
            match (&self.from_module, &self.from_port, &self.to_module, &self.to_port) {
                (Some(fm), Some(fp), Some(tm), Some(tp)) => Ok(InsertAnchor::Connection {
                    from_module: fm.clone(),
                    from_port: fp.clone(),
                    to_module: tm.clone(),
                    to_port: tp.clone(),
                }),
                _ => Err(
                    "the explicit cable anchor needs all four of from_module, from_port, to_module, to_port"
                        .to_string(),
                ),
            }
        } else if let Some(id) = &self.after {
            Ok(InsertAnchor::After(id.clone()))
        } else if let Some(id) = &self.before {
            Ok(InsertAnchor::Before(id.clone()))
        } else if let Some(t) = &self.after_type {
            Ok(InsertAnchor::AfterType(t.clone()))
        } else if let Some(t) = &self.before_type {
            Ok(InsertAnchor::BeforeType(t.clone()))
        } else {
            Ok(InsertAnchor::BeforeOutput)
        }
    }
}

// === Instrument lifecycle parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateInstrumentParam {
    #[schemars(description = "Names for the new instruments (one or many, created in order)")]
    pub names: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameInstrumentInput {
    #[schemars(description = "Instrument ID to rename")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "New name for the instrument")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameInstrumentParam {
    #[schemars(description = "Array of per-instrument renames")]
    pub items: Vec<RenameInstrumentInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentDescriptionInput {
    #[schemars(description = "Instrument ID to annotate")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "Free-text description of the instrument's intent / role. \
        Pass \"\" to clear. Never affects audio; surfaces in \
        list_instruments / get_instrument_info for later AI reads."
    )]
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentDescriptionParam {
    #[schemars(description = "Array of per-instrument description updates")]
    pub items: Vec<InstrumentDescriptionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentColorInput {
    #[schemars(description = "Instrument ID to recolor")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "Accent color as \"#RRGGBB\" or \"#RRGGBBAA\". Pass \"\" to clear back \
        to the default/auto tint. Lets you paint instruments so the mixer / arrangement is \
        visually scannable (e.g. red kick, blue pad). Surfaces in list_instruments / \
        get_instrument_info."
    )]
    pub color: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentColorParam {
    #[schemars(description = "Array of per-instrument color updates")]
    pub items: Vec<InstrumentColorInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PatchColorInput {
    #[schemars(description = "Instrument ID whose patch color to set")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "Patch-level accent color as \"#RRGGBB\" or \"#RRGGBBAA\" (distinct from \
        set_instrument_color — this travels with the patch when saved). Pass \"\" to clear. \
        Surfaces in list_instruments / get_instrument_info as patch_color."
    )]
    pub color: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatchColorParam {
    #[schemars(description = "Array of per-instrument patch-color updates")]
    pub items: Vec<PatchColorInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatchDescriptionParam {
    #[schemars(description = "Instrument ID whose patch description to set")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "Free-text patch description (what the patch is for / how it works). \
        Distinct from set_instrument_description, which captures per-instance song-role intent. \
        Pass \"\" to clear."
    )]
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ModuleDescriptionInput {
    #[schemars(description = "Instrument ID that owns the module")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Module ID, e.g. \"lfo-1\" or \"flt-2\"")]
    pub module_id: String,
    #[schemars(
        description = "Free-text per-instance description (what this specific module instance is \
        for — e.g. \"wobble LFO for the filter cutoff\"). Distinct from get_module_type_info, \
        which documents the module *type*. Pass \"\" to clear. Max 2000 characters."
    )]
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModuleDescriptionParam {
    #[schemars(
        description = "Array of per-module description updates. Each item is self-contained \
        ({instrument_id, module_id, description}), mirroring set_instrument_description, so one \
        call can annotate modules across different instruments."
    )]
    pub items: Vec<ModuleDescriptionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSongDescriptionParam {
    #[schemars(
        description = "Free-text description of the song's intent / mood / production notes. \
        Pass \"\" to clear. Surfaces in get_song_info."
    )]
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PatternDescriptionInput {
    #[schemars(description = "Pattern ID to annotate")]
    pub pattern_id: u32,
    #[schemars(
        description = "Free-text description of the pattern's intent (e.g. \"chorus drop, \
        half-time feel\"). Pass \"\" to clear. Surfaces in list_patterns."
    )]
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatternDescriptionParam {
    #[schemars(description = "Array of per-pattern description updates")]
    pub items: Vec<PatternDescriptionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackDescriptionInput {
    #[schemars(description = "Track ID to annotate")]
    pub track_id: u16,
    #[schemars(
        description = "Free-text description of the track's role (e.g. \"kick layer\", \
        \"sidechain source\"). Pass \"\" to clear. Surfaces in list_tracks."
    )]
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackDescriptionParam {
    #[schemars(description = "Array of per-track description updates")]
    pub items: Vec<TrackDescriptionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackColorInput {
    #[schemars(description = "Track ID to recolor")]
    pub track_id: u16,
    #[schemars(
        description = "Display color as \"#RRGGBB\" or \"#RRGGBBAA\" (alpha ignored). \
        Lets you paint the arrangement so it is visually scannable. Surfaces in list_tracks."
    )]
    pub color: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackColorParam {
    #[schemars(description = "Array of per-track color updates")]
    pub items: Vec<TrackColorInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SampleDescriptionInput {
    #[schemars(description = "Sample ID to annotate")]
    pub sample_id: u64,
    #[schemars(
        description = "Free-text description of the sample's intent / source. \
        Pass \"\" to clear. Surfaces in list_samples / get_sample_info."
    )]
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSampleDescriptionParam {
    #[schemars(description = "Array of per-sample description updates")]
    pub items: Vec<SampleDescriptionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SidechainSourceInput {
    #[schemars(description = "Instrument ID whose sidechain input to configure")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "Source instrument ID whose audio output feeds the sidechain. \
        Pass null (omit) to disable sidechain routing. Self-routing is rejected."
    )]
    #[serde(default)]
    pub source: Option<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSidechainSourceParam {
    #[schemars(description = "Array of per-instrument sidechain-source assignments")]
    pub items: Vec<SidechainSourceInput>,
}

/// One instrument's mixer update for `set_instrument_mixer`. Every field except
/// `instrument_id` is optional; only the fields that are present are changed.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentMixerInput {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: InstrumentId,
    #[serde(default)]
    #[schemars(
        description = "Volume level (0.0 = silent, 1.0 = unity, 2.0 = max). Omit to leave unchanged."
    )]
    pub volume: Option<f32>,
    #[serde(default)]
    #[schemars(
        description = "Pan position (-1.0 = left, 0.0 = center, 1.0 = right). Omit to leave unchanged."
    )]
    pub pan: Option<f32>,
    #[serde(default)]
    #[schemars(description = "Whether the instrument should be muted. Omit to leave unchanged.")]
    pub muted: Option<bool>,
    #[serde(default)]
    #[schemars(description = "Whether the instrument should be soloed. Omit to leave unchanged.")]
    pub solo: Option<bool>,
    #[serde(default)]
    #[schemars(description = "Whether the instrument should be enabled. Omit to leave unchanged.")]
    pub enabled: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentMixerParam {
    #[schemars(
        description = "Array of per-instrument mixer updates. Each entry sets any of \
        volume / pan / muted / solo / enabled on one instrument in a single call."
    )]
    pub items: Vec<InstrumentMixerInput>,
}

/// One instrument's voice-allocator update for `set_allocator_config`. Every
/// field except `instrument_id` is optional; only present fields are changed.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AllocatorConfigInput {
    #[schemars(description = "Instrument ID (0 for the default instrument)")]
    pub instrument_id: InstrumentId,
    #[serde(default)]
    #[schemars(
        description = "Voice allocation mode: Polyphonic, Mono, Legato, or Unison. Omit to leave unchanged."
    )]
    pub allocation_mode: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Voice-stealing strategy when all voices are busy: None, Oldest, Quietest, \
        LowestPriority, or SameNote. Omit to leave unchanged."
    )]
    pub stealing_strategy: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Unison detune spread in cents (0..100), total across all Unison-mode voices. \
        Only audible in Unison mode. Omit to leave unchanged."
    )]
    pub unison_detune: Option<f32>,
    #[serde(default)]
    #[schemars(
        description = "Unison stereo spread (0.0 = centred .. 1.0 = full L↔R width). Only audible \
        in Unison mode. Omit to leave unchanged."
    )]
    pub unison_spread: Option<f32>,
    #[serde(default)]
    #[schemars(
        description = "Maximum polyphony (1..=128). Applied on the next voice-graph rebuild / project \
        load, not live. Omit to leave unchanged."
    )]
    pub max_voices: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetAllocatorConfigParam {
    #[schemars(
        description = "Array of per-instrument voice-allocator updates. Each entry sets any of \
        allocation_mode / stealing_strategy / unison_detune / unison_spread / max_voices on one \
        instrument in a single call."
    )]
    pub items: Vec<AllocatorConfigInput>,
}

/// One instrument's MIDI-channel assignment for `set_instrument_midi_channel`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentMidiChannelInput {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "MIDI channel (1-16)", range(min = 1, max = 16))]
    pub channel: u8,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentMidiChannelParam {
    #[schemars(description = "Array of per-instrument MIDI-channel assignments")]
    pub items: Vec<InstrumentMidiChannelInput>,
}

/// One instrument's category assignment for `set_instrument_category`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentCategoryInput {
    #[schemars(description = "Instrument ID")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Category: Uncategorized, Drums, Bass, Pad, Lead, Arp, Keys, FX")]
    pub category: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInstrumentCategoryParam {
    #[schemars(description = "Array of per-instrument category assignments")]
    pub items: Vec<InstrumentCategoryInput>,
}

// === Sequencer parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSongTempoParam {
    #[schemars(description = "Tempo in BPM (e.g. 120.0)")]
    pub bpm: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TempoPointParam {
    #[schemars(description = "Absolute position in ticks (960 ticks = 1 quarter note)")]
    pub tick: u64,
    #[schemars(description = "Tempo in BPM at this point (20-999)")]
    pub bpm: f32,
    #[serde(default)]
    #[schemars(
        description = "When true, ramp linearly from this point's bpm toward the next point's bpm (accelerando/ritardando), reaching it at the next point. When false (default), a step change. A ramp with no following point holds constant."
    )]
    pub ramp: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTempoAtParam {
    #[schemars(
        description = "Tempo-map points to add or replace. A point replaces any existing change at the same tick."
    )]
    pub points: Vec<TempoPointParam>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveTempoAtParam {
    #[schemars(description = "Absolute ticks whose tempo change should be removed")]
    pub ticks: Vec<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTransportLoopParam {
    #[schemars(description = "Loop start in beats (1 beat = quarter note)")]
    pub start_beats: f32,
    #[schemars(description = "Loop end (exclusive) in beats; must be > start_beats")]
    pub end_beats: f32,
    #[schemars(description = "Whether the loop is enabled (true) or just stored (false)")]
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSongNameParam {
    #[schemars(description = "New song name")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PatternIdParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeletePatternsParam {
    #[schemars(
        description = "Pattern IDs to delete (one or many). Also removes all placements of each pattern."
    )]
    pub pattern_ids: Vec<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveNotesParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Note IDs to remove (one or many)")]
    pub note_ids: Vec<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemovePlacementsParam {
    #[schemars(
        description = "Placements to remove (one or many), each identified by pattern_id, track_id, and start_beat"
    )]
    pub placements: Vec<PlacementInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SeqSeekParam {
    #[schemars(description = "Beat position to seek to (0.0 = beginning)")]
    pub beat: f32,
}

// === Batch parameter structs ===

/// Per-note glide (portamento/glissando) for batch note operations.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GlideInput {
    #[schemars(
        description = "Glide source as a semitone offset relative to the note (negative = below). Use this OR from_pitch. Default -2."
    )]
    pub from_semitones: Option<f32>,
    #[schemars(
        description = "Glide source as an absolute MIDI pitch (0-127). Takes precedence over from_semitones.",
        range(min = 0, max = 127)
    )]
    pub from_pitch: Option<u8>,
    #[schemars(description = "Glide time in milliseconds. Default 100.")]
    pub time_ms: Option<f32>,
    #[schemars(
        description = "'continuous' (smooth portamento, default) or 'stepped' (chromatic glissando)."
    )]
    pub interp: Option<String>,
}

/// Per-note vibrato for batch note operations.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct VibratoInput {
    #[schemars(description = "Peak pitch deviation in semitones. Default 0.3.")]
    pub depth: Option<f32>,
    #[schemars(description = "LFO rate in Hz. Default 5.5.")]
    pub rate: Option<f32>,
    #[schemars(description = "Depth fade-in time in milliseconds. Default 0 (instant).")]
    pub delay_ms: Option<f32>,
    #[schemars(description = "LFO shape: 'sine' (default), 'triangle', 'square', or 'saw'.")]
    pub shape: Option<String>,
}

/// Per-note expression block for batch note operations.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExpressionInput {
    #[schemars(description = "Accent: velocity multiplier (1.0 = unchanged, >1 louder).")]
    pub accent: Option<f32>,
    #[schemars(description = "Gate length as a fraction of duration (0..1, staccato/tenuto).")]
    pub gate: Option<f32>,
    #[schemars(description = "Ghost note: force a soft velocity. Default false.")]
    pub ghost: Option<bool>,
    #[schemars(description = "Trigger probability (0..1). Default 1 (always plays).")]
    pub probability: Option<f32>,
    #[schemars(description = "Optional per-note pitch vibrato.")]
    pub vibrato: Option<VibratoInput>,
}

/// A note to add in a batch operation.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteInput {
    #[schemars(
        description = "MIDI pitch (0-127, where 60 = middle C)",
        range(min = 0, max = 127)
    )]
    pub pitch: u8,
    #[schemars(description = "Start position in beats (0.0 = beginning of pattern)")]
    pub start_beat: f32,
    #[schemars(description = "Duration in beats (1.0 = quarter note, 0.5 = eighth note)")]
    pub duration_beats: f32,
    #[schemars(
        description = "Velocity (0-127). Default 100 if omitted.",
        range(min = 0, max = 127)
    )]
    pub velocity: Option<u8>,
    #[schemars(
        description = "Tie/legato: mark THIS note as a continuation of its predecessor — it \
                       glides onto the still-active voice without retriggering (the flag sits on \
                       the later note of the tie). Default false."
    )]
    pub legato: Option<bool>,
    #[schemars(description = "Optional per-note glide (portamento/glissando) into this note.")]
    pub glide: Option<GlideInput>,
    #[schemars(
        description = "Optional per-note expression: accent, gate, ghost, probability, vibrato."
    )]
    pub expression: Option<ExpressionInput>,
}

/// Map a `NoteInput` (MCP JSON) to a bridge-level `BridgeNoteData`, resolving
/// the forgiving glide tokens (default time 100 ms, continuous interp).
fn note_input_to_bridge(n: &NoteInput) -> crate::bridge::BridgeNoteData {
    crate::bridge::BridgeNoteData {
        pitch: n.pitch,
        start_beat: n.start_beat,
        duration_beats: n.duration_beats,
        velocity: n.velocity.unwrap_or(100),
        legato: n.legato.unwrap_or(false),
        glide: n.glide.as_ref().map(|g| crate::bridge::BridgeGlide {
            from_semitones: g.from_semitones,
            from_pitch: g.from_pitch,
            time_ms: g.time_ms.unwrap_or(100.0),
            stepped: g.interp.as_deref().is_some_and(|s| {
                // Forgiving: accept common synonyms for the stepped articulation.
                let s = s.trim();
                s.eq_ignore_ascii_case("stepped")
                    || s.eq_ignore_ascii_case("step")
                    || s.eq_ignore_ascii_case("glissando")
                    || s.eq_ignore_ascii_case("gliss")
            }),
        }),
        expression: n
            .expression
            .as_ref()
            .map(|e| crate::bridge::BridgeExpression {
                accent: e.accent,
                gate: e.gate,
                ghost: e.ghost.unwrap_or(false),
                probability: e.probability,
                vibrato: e.vibrato.as_ref().map(|v| crate::bridge::BridgeVibrato {
                    depth: v.depth.unwrap_or(0.3),
                    rate: v.rate.unwrap_or(5.5),
                    delay_ms: v.delay_ms.unwrap_or(0.0),
                    shape: v.shape.clone(),
                }),
            }),
    }
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
    #[schemars(
        description = "New MIDI pitch (0-127), or null to keep current",
        range(min = 0, max = 127)
    )]
    pub pitch: Option<u8>,
    #[schemars(description = "New start position in beats, or null to keep current")]
    pub start_beat: Option<f32>,
    #[schemars(description = "New duration in beats, or null to keep current")]
    pub duration_beats: Option<f32>,
    #[schemars(
        description = "New velocity (0-127), or null to keep current",
        range(min = 0, max = 127)
    )]
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

// === Note Grid (pooled note-processing graphs) ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteGraphIdParam {
    #[schemars(description = "Note graph id (from list_note_graphs)")]
    pub graph_id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateNoteGraphParam {
    #[schemars(description = "Name for the new note graph")]
    pub name: String,
    #[schemars(description = "Optional free-text description")]
    pub description: Option<String>,
    #[schemars(description = "Optional color as #rrggbb")]
    pub color: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteNoteGraphParam {
    #[schemars(
        description = "Note graph ids to delete (one or many). Each delete clears every pattern reference to that graph."
    )]
    pub graph_ids: Vec<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddNoteGraphModuleInput {
    #[schemars(description = "Note graph id to add the module to")]
    pub graph_id: u32,
    #[schemars(
        description = "Module as externally-tagged NoteModuleConfig JSON: one of {\"Processor\":{...}} (ScaleQuantize/Chord/Arpeggiator/Humanize), {\"Euclidean\":{...}}, {\"ProbabilityGate\":{...}}, {\"NoteLfo\":{...}}, {\"StepLfo\":{...}}, {\"NoteEnvelope\":{...}} (optional \"trigger\": \"SourceOnset\"|\"StreamOnset\"), {\"NoteScriptTransform\":{\"source\":\"...\"}} (a YAMS note_event script — set/compile the source afterwards with set_note_graph_script), {\"NoteDelay\":{...}} (decaying echoes; repeats clamp to 16 at playback), or {\"Ratchet\":{...}} (subdivides notes into retriggers; count clamps to 16). Read existing modules with get_note_graph to see the exact shape."
    )]
    pub module: synth_sequencer::NoteModuleConfig,
    #[schemars(description = "Optional per-node pedagogical intent text")]
    pub description: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddNoteGraphModuleParam {
    #[schemars(description = "Modules to add (one or many). Returns each new module id.")]
    pub items: Vec<AddNoteGraphModuleInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteGraphModuleInput {
    #[schemars(description = "Note graph id")]
    pub graph_id: u32,
    #[schemars(description = "Module id to replace (from get_note_graph)")]
    pub module_id: u32,
    #[schemars(
        description = "Replacement module as externally-tagged NoteModuleConfig JSON (same shape as add_note_graph_module). The id and its connections are preserved."
    )]
    pub module: synth_sequencer::NoteModuleConfig,
    #[schemars(
        description = "Optional replacement per-node description; omitted keeps it unchanged"
    )]
    pub description: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteGraphModuleParam {
    #[schemars(description = "Modules to update (one or many)")]
    pub items: Vec<SetNoteGraphModuleInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteGraphScriptParam {
    #[schemars(description = "Note graph id")]
    pub graph_id: u32,
    #[schemars(description = "NoteScriptTransform module id (from get_note_graph)")]
    pub module_id: u32,
    #[schemars(
        description = "YAMS note_event source. Runs per note (1:1). Read note_pitch/note_vel/note_dur/tick and value inputs in1..in4; assign out.pitch/out.vel/out.dur/out.gate. A negative out.vel drops the note; a negative out.dur restores 'plays until cut'. Empty source = pass-through. Example: `out.pitch = note_pitch + 12`."
    )]
    pub source: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveNoteGraphModuleParam {
    #[schemars(description = "Note graph id")]
    pub graph_id: u32,
    #[schemars(description = "Module id to remove (from get_note_graph)")]
    pub module_id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectNoteGraphInput {
    #[schemars(description = "Note graph id")]
    pub graph_id: u32,
    #[schemars(description = "Source (output-side) module id")]
    pub from: u32,
    #[schemars(description = "Destination (input-side) module id")]
    pub to: u32,
    #[schemars(
        description = "Port: 'note_stream' (the linear spine — one input and one output per node), 'value', or 'gate' (modulation into a value input port). Defaults to 'note_stream'."
    )]
    pub port: Option<String>,
    #[schemars(
        description = "For value/gate edges, which value-input port of the target to feed (0-based). Defaults to 0."
    )]
    pub to_input: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectNoteGraphParam {
    #[schemars(
        description = "Connections to add (one or many). Each is validated for linearity, acyclicity, and endpoint types."
    )]
    pub items: Vec<ConnectNoteGraphInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetNoteGraphsParam {
    #[schemars(description = "Graph ids to read, or omit/null to read every graph")]
    pub graph_ids: Option<Vec<u32>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteGraphMetadataInput {
    pub graph_id: u32,
    pub name: Option<String>,
    pub description: Option<String>,
    #[schemars(description = "Replacement #rrggbb color; null/omitted keeps the current color")]
    pub color: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteGraphMetadataParam {
    pub items: Vec<SetNoteGraphMetadataInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatternNoteGraphInput {
    #[schemars(description = "Pattern id to bind")]
    pub pattern_id: u32,
    #[schemars(
        description = "Note graph id to bind, or null/omitted to clear the binding (the pattern's raw notes + per-note ornaments then play)."
    )]
    pub graph_id: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatternNoteGraphParam {
    #[schemars(description = "Pattern→graph bindings to set or clear (one or many).")]
    pub items: Vec<SetPatternNoteGraphInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteNoteGraphInput {
    #[schemars(description = "Pattern id containing the note")]
    pub pattern_id: u32,
    #[schemars(description = "Note id (from list_notes)")]
    pub note_id: u64,
    #[schemars(
        description = "Note graph id to bind for per-note articulation, or null/omitted to clear the binding."
    )]
    pub graph_id: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteNoteGraphParam {
    #[schemars(description = "Per-note graph bindings to set or clear (one or many).")]
    pub items: Vec<SetNoteNoteGraphInput>,
}

// === Mod Grid (pooled control-rate modulator graphs) ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ModGraphIdParam {
    #[schemars(description = "Mod graph id (from list_mod_graphs)")]
    pub graph_id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateModGraphParam {
    #[schemars(description = "Name for the new mod graph")]
    pub name: String,
    #[schemars(description = "Optional free-text description")]
    pub description: Option<String>,
    #[schemars(
        description = "Scope: 'global' (one always-on instance, default) or 'track' (one instance per assigned track; assign with assign_mod_graph)."
    )]
    pub scope: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteModGraphParam {
    #[schemars(description = "Mod graph ids to delete (one or many).")]
    pub graph_ids: Vec<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModGraphScopeParam {
    #[schemars(description = "Mod graph id")]
    pub graph_id: u32,
    #[schemars(description = "New scope: 'global' or 'track'.")]
    pub scope: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssignModGraphParam {
    #[schemars(description = "Mod graph id (must be 'track' scope to run)")]
    pub graph_id: u32,
    #[schemars(
        description = "Track ids to assign (replaces the current set; one running instance per track). Unknown ids are dropped."
    )]
    pub tracks: Vec<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddModGraphNodeInput {
    #[schemars(description = "Mod graph id to add the node to")]
    pub graph_id: u32,
    #[schemars(
        description = "Node as externally-tagged ModNodeConfig JSON. module_type uses the snake_case module key (e.g. 'lfo', 'mseg', 'envelope_follower'). Examples: {\"Module\":{\"module_type\":\"lfo\",\"params\":{\"rate\":2.0}}} (a hosted control-rate module), {\"Macro\":{\"name\":\"...\",\"value\":0.5}}, {\"Transport\":{\"source\":\"BeatPhase\"}}, {\"MidiCc\":{\"cc\":1}}, {\"AudioTap\":{\"source\":{\"Track\":0}}}, or {\"Target\":{\"target\":{\"Track\":{\"param\":\"Volume\"}},\"amount\":0.25}} (a routing sink; connect a source's out to its 'in' port). Read existing nodes with get_mod_graph to see the exact shape."
    )]
    pub node: synth_sequencer::ModNodeConfig,
    #[schemars(description = "Optional per-node pedagogical intent text")]
    pub description: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddModGraphNodeParam {
    #[schemars(description = "Nodes to add (one or many). Returns each new node id.")]
    pub items: Vec<AddModGraphNodeInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveModGraphNodeParam {
    #[schemars(description = "Mod graph id")]
    pub graph_id: u32,
    #[schemars(description = "Node id to remove (drops every cable touching it)")]
    pub node_id: u32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectModGraphInput {
    #[schemars(description = "Mod graph id")]
    pub graph_id: u32,
    #[schemars(description = "Source node id")]
    pub from: u32,
    #[schemars(description = "Source output port name (e.g. 'out')")]
    pub from_port: String,
    #[schemars(description = "Destination node id")]
    pub to: u32,
    #[schemars(description = "Destination input port name (e.g. 'in', 'rate_cv')")]
    pub to_port: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectModGraphParam {
    #[schemars(description = "Cables to add (one or many).")]
    pub items: Vec<ConnectModGraphInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DisconnectModGraphParam {
    #[schemars(
        description = "Cables to remove (one or many), each the exact (graph_id, from, from_port, to, to_port) of an existing cable — the inverse of connect_mod_graph."
    )]
    pub items: Vec<ConnectModGraphInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModGraphNodeInput {
    #[schemars(description = "Mod graph id")]
    pub graph_id: u32,
    #[schemars(description = "Existing node id to edit in place")]
    pub node_id: u32,
    #[schemars(
        description = "Replacement node as externally-tagged ModNodeConfig JSON (same shape as add_mod_graph_node). Keeps the node id and every cable touching it; the graph is re-validated (e.g. replacing a source that has outgoing cables with a Target is rejected). Read the current shape with get_mod_graph first."
    )]
    pub node: synth_sequencer::ModNodeConfig,
    #[schemars(
        description = "Optional replacement per-node description; omitted keeps it unchanged"
    )]
    pub description: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModGraphNodeParam {
    #[schemars(description = "Nodes to update (one or many)")]
    pub items: Vec<SetModGraphNodeInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetModGraphsParam {
    #[schemars(description = "Graph ids to read, or omit/null to read every graph")]
    pub graph_ids: Option<Vec<u32>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModGraphMetadataInput {
    pub graph_id: u32,
    pub name: Option<String>,
    pub description: Option<String>,
    #[schemars(description = "Replacement #rrggbb color; null/omitted keeps the current color")]
    pub color: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModGraphMetadataParam {
    pub items: Vec<SetModGraphMetadataInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListModTargetsParam {
    #[schemars(
        description = "Restrict to one graph's routings, or null/omit for every graph's routings."
    )]
    pub graph_id: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOrnamentInput {
    #[schemars(description = "Pattern ID containing the note")]
    pub pattern_id: u32,
    #[schemars(description = "Note ID (from list_notes)")]
    pub note_id: u64,
    #[schemars(
        description = "Ornament as JSON to set, or null/omitted to clear. Fields (all optional): count (total hits: flam 2, drag 3, ruff 4, roll N), spacing (ticks between hits), spacing_curve (Even/Accelerate/Decelerate), dynamics (Flat/Crescendo/Decrescendo), placement (LeadIn/OnBeat), pitch_offset (semitones for grace tones), grace_gate (0-1 grace length fraction)."
    )]
    pub ornament: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteOrnamentParam {
    #[schemars(description = "Array of per-note ornament updates (set or clear)")]
    pub items: Vec<NoteOrnamentInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClearPatternParam {
    #[schemars(description = "Pattern IDs to clear all notes from (one or many)")]
    pub pattern_ids: Vec<u32>,
}

/// A structured automation target — a typed alternative to the
/// `module:<type>:<instance>:<param>` DSL string accepted by `param`. Per-module
/// targets are validated against the instrument graph and the automatable
/// allowlist either way.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutomationTargetInput {
    /// A parameter on a specific module in the instrument's graph (e.g. the
    /// cutoff of filter instance 1).
    Module {
        /// Module type token: the short key, snake_case, or display name
        /// (e.g. "flt", "filter", "Filter") — see add_module.
        module_type: String,
        /// 1-based instance index within that module type.
        instance: u16,
        /// Descriptor parameter id (e.g. "cutoff"). See get_module_info.
        param_id: String,
    },
    /// An instrument-level macro: Volume, Pan, FilterCutoff, FilterResonance,
    /// Attack, Decay, Sustain, Release.
    Instrument {
        /// Macro name.
        param: String,
    },
    /// A track-scoped parameter: Volume, Pan, Mute, Pitch. Omit `track_id` for
    /// a host-track lane (follows whichever track the pattern is placed on —
    /// the usual form); set it to automate a specific track (cross-track).
    Track {
        /// Track parameter name.
        param: String,
        /// Target track id; omitted = the pattern's host track.
        track_id: Option<u16>,
    },
    /// A song-global parameter: MasterVolume.
    Global {
        /// Global parameter name.
        param: String,
    },
}

impl AutomationTargetInput {
    /// Render to the canonical target DSL string consumed by the bridge.
    fn to_target_string(&self) -> String {
        match self {
            Self::Module {
                module_type,
                instance,
                param_id,
            } => format!("module:{module_type}:{instance}:{param_id}"),
            Self::Instrument { param } => param.clone(),
            Self::Track {
                param,
                track_id: Some(id),
            } => format!("track:{param}:{id}"),
            Self::Track {
                param,
                track_id: None,
            } => format!("track:{param}"),
            Self::Global { param } => format!("global:{param}"),
        }
    }
}

/// An automation point to add.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AutomationPointInput {
    #[schemars(
        description = "Target as a DSL string: an instrument macro (Volume, Pan, FilterCutoff, FilterResonance, Attack, Decay, Sustain, Release); 'module:<type>:<instance>:<param>' (e.g. 'module:flt:1:cutoff'); a track lane 'track:<param>' (Volume/Pan/Mute/Pitch, host track) or 'track:<param>:<track_id>' (a specific track); or a global lane 'global:MasterVolume'. Provide this OR the structured 'target'."
    )]
    pub param: Option<String>,
    #[schemars(
        description = "Structured target (alternative to 'param'; takes precedence if both are given)."
    )]
    pub target: Option<AutomationTargetInput>,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(description = "Position in beats")]
    pub beat: f32,
    #[schemars(description = "Normalized value (0.0-1.0)")]
    pub value: f32,
    #[schemars(description = "Interpolation curve (default Linear)")]
    pub curve: Option<crate::bridge::CurveKind>,
    #[schemars(
        description = "Strength for the Exponential curve (-127..=127, negative = ease-in, positive = ease-out); ignored for other curves."
    )]
    pub curve_strength: Option<i8>,
}

impl AutomationPointInput {
    /// The effective target DSL string: the structured `target` wins, otherwise
    /// the `param` string (empty if neither is set, which downstream rejects).
    fn effective_target(&self) -> String {
        match &self.target {
            Some(t) => t.to_target_string(),
            None => self.param.clone().unwrap_or_default(),
        }
    }
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
    #[schemars(
        description = "Target DSL: instrument macro (Volume/Pan/FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release), module:<type>:<instance>:<param>, track:<param>[:<track_id>], or global:MasterVolume. From list_automation_lanes, pass the lane target string back verbatim."
    )]
    pub target: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveAutomationPointsParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(
        description = "Target DSL: instrument macro (Volume/Pan/FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release), module:<type>:<instance>:<param>, track:<param>[:<track_id>], or global:MasterVolume. From list_automation_lanes, pass the lane target string back verbatim."
    )]
    pub target: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(description = "Beat positions of points to remove")]
    pub beats: Vec<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClearAutomationLaneInput {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(
        description = "Target DSL: instrument macro (Volume/Pan/FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release), module:<type>:<instance>:<param>, track:<param>[:<track_id>], or global:MasterVolume. From list_automation_lanes, pass the lane target string back verbatim."
    )]
    pub target: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClearAutomationLaneParam {
    #[schemars(description = "Automation lanes to clear (one or many)")]
    pub items: Vec<ClearAutomationLaneInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScaleAutomationLaneInput {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Target lane (e.g. 'module:flt:1:cutoff' or 'FilterCutoff')")]
    pub target: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(
        description = "Multiplier applied to each point's value around the pivot (e.g. 0.8 = 20% less movement, 1.5 = more). Values are clamped to 0..1 afterwards."
    )]
    pub scale: f32,
    #[schemars(
        description = "Pivot the scaling is applied around, 0..1 (default 0.5 = lane midpoint). Points keep their distance-from-pivot times `scale`."
    )]
    pub pivot: Option<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScaleAutomationLaneParam {
    #[schemars(description = "Automation lanes to scale (one or many)")]
    pub items: Vec<ScaleAutomationLaneInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct OffsetAutomationLaneInput {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "Target lane (e.g. 'module:flt:1:cutoff' or 'FilterCutoff')")]
    pub target: String,
    #[schemars(description = "Instrument index (default 0)")]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(
        description = "Amount added to every point's value (e.g. -0.05 lowers the whole lane). Values are clamped to 0..1 afterwards."
    )]
    pub offset: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct OffsetAutomationLaneParam {
    #[schemars(description = "Automation lanes to offset (one or many)")]
    pub items: Vec<OffsetAutomationLaneInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CopyAutomationLaneInput {
    #[schemars(description = "Source pattern ID")]
    pub from_pattern_id: u32,
    #[schemars(description = "Source target lane")]
    pub from_target: String,
    #[schemars(description = "Source instrument index (default 0)")]
    pub from_instrument_id: Option<InstrumentId>,
    #[schemars(description = "Destination pattern ID (may equal the source)")]
    pub to_pattern_id: u32,
    #[schemars(description = "Destination target lane")]
    pub to_target: String,
    #[schemars(description = "Destination instrument index (default 0)")]
    pub to_instrument_id: Option<InstrumentId>,
    #[schemars(
        description = "Optional multiplier applied to copied values (default 1.0). Clamped to 0..1."
    )]
    pub scale: Option<f32>,
    #[schemars(
        description = "Optional amount added to copied values (default 0.0). Clamped to 0..1."
    )]
    pub offset: Option<f32>,
    #[schemars(
        description = "Clear the destination lane before copying (default false = merge points in)."
    )]
    pub clear_destination: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CopyAutomationLaneParam {
    #[schemars(description = "Automation-lane copy operations (one or many)")]
    pub items: Vec<CopyAutomationLaneInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetAutomationSummaryParam {
    #[schemars(
        description = "How to group lanes: 'instrument' (default), 'target', or 'pattern'."
    )]
    pub group_by: Option<String>,
}

// === Track control parameter structs ===

/// One track's mixer update for `set_track_mixer`. Every field except `track_id`
/// is optional; only the fields that are present are changed.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackMixerInput {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[serde(default)]
    #[schemars(description = "Volume (0.0 = silent, 1.0 = full). Omit to leave unchanged.")]
    pub volume: Option<f32>,
    #[serde(default)]
    #[schemars(
        description = "Pan position (-1.0 = left, 0.0 = center, 1.0 = right). Omit to leave unchanged."
    )]
    pub pan: Option<f32>,
    #[serde(default)]
    #[schemars(description = "Whether the track should be muted. Omit to leave unchanged.")]
    pub muted: Option<bool>,
    #[serde(default)]
    #[schemars(description = "Whether the track should be soloed. Omit to leave unchanged.")]
    pub solo: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackMixerParam {
    #[schemars(
        description = "Array of per-track mixer updates. Each entry sets any of \
        volume / pan / muted / solo on one track in a single call."
    )]
    pub items: Vec<TrackMixerInput>,
}

/// One track's instrument assignment for `set_track_instrument`. `instrument_id`
/// is required but nullable: a number assigns that instrument, `null` unassigns.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackInstrumentInput {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(
        description = "Instrument ID to drive this track. Pass null to unassign (the track plays nothing)."
    )]
    pub instrument_id: Option<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackInstrumentParam {
    #[schemars(description = "Array of per-track instrument assignments")]
    pub items: Vec<TrackInstrumentInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateReturnBusParam {
    #[schemars(
        description = "Names for the new return busses (e.g. \"Reverb\", \"Delay\"), created in order"
    )]
    pub names: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteReturnBusesParam {
    #[schemars(
        description = "Return bus IDs to delete (one or many). Also removes every track send that targeted each bus."
    )]
    pub return_ids: Vec<u16>,
}

/// One return bus's mixer update for `set_return_bus_mixer`. Every field except
/// `return_id` is optional; only the fields that are present are changed.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnBusMixerInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[serde(default)]
    #[schemars(description = "Volume (0.0 = silent, 1.0 = full). Omit to leave unchanged.")]
    pub volume: Option<f32>,
    #[serde(default)]
    #[schemars(
        description = "Pan position (-1.0 = left, 0.0 = center, 1.0 = right). Omit to leave unchanged."
    )]
    pub pan: Option<f32>,
    #[serde(default)]
    #[schemars(description = "Whether the return bus should be muted. Omit to leave unchanged.")]
    pub muted: Option<bool>,
    #[serde(default)]
    #[schemars(description = "Whether the return bus should be soloed. Omit to leave unchanged.")]
    pub solo: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetReturnBusMixerParam {
    #[schemars(
        description = "Array of per-return-bus mixer updates. Each entry sets any of \
        volume / pan / muted / solo on one return bus in a single call."
    )]
    pub items: Vec<ReturnBusMixerInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnBusColorInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[schemars(description = "Display color as \"#RRGGBB\" (or \"#RRGGBBAA\", alpha ignored)")]
    pub color: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetReturnBusColorParam {
    #[schemars(description = "Array of per-return-bus color updates")]
    pub items: Vec<ReturnBusColorInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnBusDescriptionInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[schemars(description = "Free-text description / intent (\"\" clears it)")]
    pub description: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetReturnBusDescriptionParam {
    #[schemars(description = "Array of per-return-bus description updates")]
    pub items: Vec<ReturnBusDescriptionInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameReturnBusInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[schemars(description = "New name")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameReturnBusParam {
    #[schemars(description = "Array of per-return-bus renames")]
    pub items: Vec<RenameReturnBusInput>,
}

/// Serde default for boolean fields that should default to `true` when omitted.
fn default_true() -> bool {
    true
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackSendInput {
    #[schemars(description = "Track ID (the channel sending)")]
    pub track_id: u16,
    #[schemars(description = "Destination return bus ID")]
    pub return_id: u16,
    #[schemars(description = "Send level (0.0 = none, 1.0 = unity)")]
    pub level: f32,
    #[serde(default)]
    #[schemars(
        description = "Tap point: true = pre-fader, false = post-fader (default). Post-fader follows the channel fader."
    )]
    pub pre_fader: bool,
    #[serde(default = "default_true")]
    #[schemars(
        description = "Whether the send is active (default true). false = non-destructive bypass: keeps the level/tap but contributes nothing to the return bus."
    )]
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetTrackSendParam {
    #[schemars(
        description = "Track sends to add or update (one or many; upsert by track_id+return_id target)"
    )]
    pub sends: Vec<TrackSendInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TrackSendRef {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(description = "Destination return bus ID the send targets")]
    pub return_id: u16,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveTrackSendsParam {
    #[schemars(
        description = "Track sends to remove (one or many), each a {track_id, return_id} pair"
    )]
    pub sends: Vec<TrackSendRef>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddReturnEffectsParam {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[schemars(
        description = "Effect type keys to add (one or many), in chain order (e.g. ['eq', 'rev']). Each accepts the prefix or display name (e.g. 'rev', 'delay', 'chorus', 'compressor', 'distortion'). Voice modules are rejected."
    )]
    pub effect_types: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveReturnEffectsParam {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[schemars(
        description = "Effect module-id strings to remove (one or many), e.g. ['rev-1'], from list_return_busses"
    )]
    pub module_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnEffectParamInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[schemars(description = "Effect module-id string (e.g. 'rev-1')")]
    pub module_id: String,
    #[schemars(
        description = "Parameter name (type_id or display name) — see the effect's parameters in list_return_busses"
    )]
    pub param_name: String,
    #[schemars(
        description = "New value: a number in the parameter's native range, a boolean for on/off, or a string for choice/enum parameters."
    )]
    pub value: ParamValueInput,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetReturnEffectParameterParam {
    #[schemars(description = "Return-bus effect parameter changes to apply (one or many)")]
    pub params: Vec<ReturnEffectParamInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnEffectEnabledInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[schemars(description = "Effect module-id string (e.g. 'rev-1')")]
    pub module_id: String,
    #[schemars(description = "true = active, false = bypassed")]
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetReturnEffectEnabledParam {
    #[schemars(description = "Return-bus effect enable/bypass toggles to apply (one or many)")]
    pub items: Vec<ReturnEffectEnabledInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReorderReturnEffectInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: u16,
    #[schemars(description = "Effect module-id string (e.g. 'rev-1')")]
    pub module_id: String,
    #[schemars(description = "Direction to move: 'up' (earlier in the chain) or 'down' (later)")]
    pub direction: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReorderReturnEffectParam {
    #[schemars(
        description = "Return-bus effect reorder moves to apply (one or many, applied in order)"
    )]
    pub items: Vec<ReorderReturnEffectInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnSendInput {
    #[schemars(description = "Source return bus ID (the one sending)")]
    pub from_id: u16,
    #[schemars(description = "Destination return bus ID")]
    pub to_id: u16,
    #[schemars(description = "Send level (0.0 = none, 1.0 = unity)")]
    pub level: f32,
    #[serde(default = "default_true")]
    #[schemars(
        description = "Whether the send is active (default true). false = non-destructive bypass."
    )]
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetReturnSendParam {
    #[schemars(
        description = "Bus-to-bus sends to add or update (one or many; upsert by from_id+to_id target)"
    )]
    pub sends: Vec<ReturnSendInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnSendRef {
    #[schemars(description = "Source return bus ID")]
    pub from_id: u16,
    #[schemars(description = "Destination return bus ID the send targets")]
    pub to_id: u16,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveReturnSendsParam {
    #[schemars(
        description = "Bus-to-bus sends to remove (one or many), each a {from_id, to_id} pair"
    )]
    pub sends: Vec<ReturnSendRef>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetMasterVolumeParam {
    #[schemars(description = "Master output volume (0.0 = silent, 1.0 = unity)")]
    pub volume: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddMasterEffectsParam {
    #[schemars(
        description = "Effect type keys to add (one or many), in chain order (e.g. ['eq', 'limiter']). Each accepts the prefix or display name (e.g. 'limiter', 'compressor', 'rev'). Voice modules are rejected."
    )]
    pub effect_types: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveMasterEffectsParam {
    #[schemars(
        description = "Effect module-id strings to remove (one or many), e.g. ['lim-1'], from list_master_effects"
    )]
    pub module_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MasterEffectParamInput {
    #[schemars(description = "Effect module-id string (e.g. 'lim-1')")]
    pub module_id: String,
    #[schemars(description = "Parameter name (type_id or display name); see list_master_effects")]
    pub param_name: String,
    #[schemars(
        description = "New value: a number in the parameter's native range, a boolean for on/off, or a string for choice/enum parameters."
    )]
    pub value: ParamValueInput,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetMasterEffectParameterParam {
    #[schemars(description = "Master-effect parameter changes to apply (one or many)")]
    pub params: Vec<MasterEffectParamInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MasterEffectEnabledInput {
    #[schemars(description = "Effect module-id string (e.g. 'lim-1')")]
    pub module_id: String,
    #[schemars(description = "true = active, false = bypassed")]
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetMasterEffectEnabledParam {
    #[schemars(description = "Master-effect enable/bypass toggles to apply (one or many)")]
    pub items: Vec<MasterEffectEnabledInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReorderMasterEffectInput {
    #[schemars(description = "Effect module-id string (e.g. 'lim-1')")]
    pub module_id: String,
    #[schemars(description = "Direction to move: 'up' (earlier in the chain) or 'down' (later)")]
    pub direction: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReorderMasterEffectParam {
    #[schemars(
        description = "Master-effect reorder moves to apply (one or many, applied in order)"
    )]
    pub items: Vec<ReorderMasterEffectInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameTrackInput {
    #[schemars(description = "Track ID")]
    pub track_id: u16,
    #[schemars(description = "New name for the track")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameTrackParam {
    #[schemars(description = "Array of per-track renames")]
    pub items: Vec<RenameTrackInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteTracksParam {
    #[schemars(
        description = "Track IDs to delete (one or many). Also removes each track's placements from the arrangement."
    )]
    pub track_ids: Vec<u16>,
}

// === Pattern management parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenamePatternInput {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "New name for the pattern")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenamePatternParam {
    #[schemars(description = "Array of per-pattern renames")]
    pub items: Vec<RenamePatternInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PatternLengthInput {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: u32,
    #[schemars(description = "New length in beats (e.g. 4.0 for one bar in 4/4)")]
    pub length_beats: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatternLengthParam {
    #[schemars(description = "Array of per-pattern length changes")]
    pub items: Vec<PatternLengthInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DuplicatePatternParam {
    #[schemars(description = "Pattern IDs to duplicate (one or many)")]
    pub pattern_ids: Vec<u32>,
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
        description = "New value: a number in the parameter's native range (440.0), a boolean for on/off, or a string for a choice/enum or an address (e.g. a waveform 'sawtooth', or a Mod Matrix slot_N_dest / slot_N_source address like 'spp-1.x' / 'lfo-1.out'). Use get_module_info / get_instrument_automation_targets to discover names and targets."
    )]
    pub value: ParamValueInput,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetParametersParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
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
    pub instrument_id: Option<InstrumentId>,
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
    pub instrument_id: Option<InstrumentId>,
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

/// Single instrument definition for batch build.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InstrumentDefInput {
    #[schemars(
        description = "Existing instrument ID to update. If omitted, creates a new instrument."
    )]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(description = "Instrument name")]
    pub name: String,
    #[schemars(
        description = "MIDI channel (1-16, optional)",
        range(min = 1, max = 16)
    )]
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
pub struct RebuildInstrumentParam {
    #[schemars(description = "Existing instrument ID to rebuild (required).")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Instrument name")]
    pub name: String,
    #[schemars(
        description = "MIDI channel (1-16, optional)",
        range(min = 1, max = 16)
    )]
    pub midi_channel: Option<u8>,
    #[schemars(description = "Volume (0.0-2.0, optional)")]
    pub volume: Option<f32>,
    #[schemars(description = "Pan (-1.0 to 1.0, optional)")]
    pub pan: Option<f32>,
    #[schemars(description = "Modules of the rebuilt voice graph")]
    pub modules: Vec<ModuleDefInput>,
    #[schemars(description = "Connections between modules (array indices)")]
    pub connections: Option<Vec<ConnectionDefInput>>,
    #[schemars(
        description = "If true, delete automation lanes orphaned by the rebuild (their target module no longer exists). If false (default), keep them and just report them — they stay dangling until the module is recreated or the lane is cleared."
    )]
    pub drop_orphaned: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ApplyExamplePatchParam {
    #[schemars(
        description = "Instrument ID to apply the patch to. If omitted, creates a new instrument."
    )]
    pub instrument_id: Option<InstrumentId>,
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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SavePatchParam {
    #[schemars(description = "ID of the instrument to export as a patch")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Absolute file path for the patch (.json)")]
    pub path: String,
}

// ============================================================================
// SAMPLE PARAMETER STRUCTS
// ============================================================================

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListSamplesParam {
    #[schemars(description = "Optional name filter (substring match, case-insensitive).")]
    pub name_filter: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ImportSampleInput {
    #[schemars(description = "Absolute file path to the WAV file to import.")]
    pub path: String,
    #[schemars(description = "Optional display name. Defaults to filename without extension.")]
    pub name: Option<String>,
    #[schemars(
        description = "Optional root MIDI note (0-127). 60 = C4 (middle C). Determines pitch mapping.",
        range(min = 0, max = 127)
    )]
    pub root_note: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ImportSampleParam {
    #[schemars(description = "WAV files to import (one or many).")]
    pub samples: Vec<ImportSampleInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SampleIdParam {
    #[schemars(description = "Sample ID. Use list_samples to find available IDs.")]
    pub sample_id: u64,
}

/// One-or-many sample IDs, for in-place sample edits (normalize / reverse / trim / duplicate).
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SampleIdsParam {
    #[schemars(description = "Sample IDs (one or many). Use list_samples to find available IDs.")]
    pub sample_ids: Vec<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteSamplesParam {
    #[schemars(
        description = "Sample IDs to delete (one or many). Use list_samples to find available IDs."
    )]
    pub sample_ids: Vec<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameSampleInput {
    #[schemars(description = "Sample ID.")]
    pub sample_id: u64,
    #[schemars(description = "New display name for the sample.")]
    pub name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenameSampleParam {
    #[schemars(description = "Array of per-sample renames")]
    pub items: Vec<RenameSampleInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SampleRootNoteInput {
    #[schemars(description = "Sample ID.")]
    pub sample_id: u64,
    #[schemars(
        description = "Root MIDI note (0-127). 60=C4, 48=C3, 72=C5.",
        range(min = 0, max = 127)
    )]
    pub note: u8,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSampleRootNoteParam {
    #[schemars(description = "Array of per-sample root-note assignments")]
    pub items: Vec<SampleRootNoteInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SampleLoopInput {
    #[schemars(description = "Sample ID.")]
    pub sample_id: u64,
    #[schemars(description = "Enable or disable looping.")]
    pub enabled: bool,
    #[schemars(description = "Loop start time in seconds (required when enabled=true).")]
    pub start_seconds: Option<f64>,
    #[schemars(description = "Loop end time in seconds (required when enabled=true).")]
    pub end_seconds: Option<f64>,
    #[schemars(description = "Crossfade duration in milliseconds at loop boundary (default: 0).")]
    pub crossfade_ms: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSampleLoopParam {
    #[schemars(description = "Array of per-sample loop settings")]
    pub items: Vec<SampleLoopInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SampleCropInput {
    #[schemars(description = "Sample ID.")]
    pub sample_id: u64,
    #[schemars(
        description = "Crop start time in seconds. Omit both start and end to remove crop."
    )]
    pub start_seconds: Option<f64>,
    #[schemars(description = "Crop end time in seconds. Omit both start and end to remove crop.")]
    pub end_seconds: Option<f64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSampleCropParam {
    #[schemars(description = "Array of per-sample crop settings")]
    pub items: Vec<SampleCropInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExportSampleInput {
    #[schemars(description = "Sample ID.")]
    pub sample_id: u64,
    #[schemars(description = "Absolute file path for the output WAV file.")]
    pub path: String,
    #[schemars(description = "Bit depth: 16, 24, or 32 (float). Default: 16.")]
    pub bit_depth: Option<u8>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExportSampleParam {
    #[schemars(description = "Samples to export to WAV files (one or many)")]
    pub samples: Vec<ExportSampleInput>,
}

// === Sampler module parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssignSampleInput {
    #[schemars(description = "Instrument ID.")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Sampler module ID (e.g. \"sam-1\"). Must be a Sampler module.")]
    pub module_id: String,
    #[schemars(description = "Sample ID to assign. Use list_samples to find IDs.")]
    pub sample_id: u64,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssignSampleParam {
    #[schemars(description = "Sample-to-sampler-module assignments (one or many)")]
    pub items: Vec<AssignSampleInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SamplerModuleParam {
    #[schemars(description = "Instrument ID.")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Sampler module ID (e.g. \"sam-1\").")]
    pub module_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SamplerParameterInput {
    #[schemars(description = "Instrument ID.")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "Sampler module ID (e.g. \"sam-1\").")]
    pub module_id: String,
    #[schemars(
        description = "Parameter name: pitch_tracking, level, play_mode, direction, \
                       velocity_sensitivity, fine_tune, start_offset."
    )]
    pub param_name: String,
    #[schemars(description = "Parameter value. Booleans: \"true\"/\"false\". \
                       Enums: \"one_shot\"/\"sustain\"/\"loop\" for play_mode, \
                       \"forward\"/\"reverse\"/\"ping_pong\" for direction. \
                       Numbers: float value as string.")]
    pub value: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetSamplerParameterParam {
    #[schemars(description = "Sampler parameter changes to apply (one or many)")]
    pub params: Vec<SamplerParameterInput>,
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
    /// When this server instance was constructed; used to report session
    /// lifetime in the disconnect log line.
    started_at: Instant,
}

impl SynthMcpServer {
    /// Create a new MCP server backed by the given bridge (no session tracking).
    pub fn new(bridge: Arc<dyn SynthBridge>) -> Self {
        Self::with_filter(bridge, &[])
    }

    /// Create a new MCP server with session tracking via a shared registry.
    pub fn with_registry(bridge: Arc<dyn SynthBridge>, registry: McpSessionRegistry) -> Self {
        Self::with_registry_and_filter(bridge, registry, &[])
    }

    /// Create a new MCP server with a list of tools hidden from the client.
    ///
    /// Disabled tools are excluded from `tools/list` and rejected at call time.
    /// Use this to expose a focused tool surface (e.g. read-only mode, or
    /// hiding categories like the sampler that don't apply to the deployment).
    pub fn with_filter(bridge: Arc<dyn SynthBridge>, disabled_tools: &[&'static str]) -> Self {
        Self {
            bridge,
            tool_router: Self::build_router(disabled_tools),
            registry: None,
            session_id: 0,
            started_at: Instant::now(),
        }
    }

    /// Like [`with_registry`](Self::with_registry) but also filters the tool
    /// list to hide the named tools from the client.
    pub fn with_registry_and_filter(
        bridge: Arc<dyn SynthBridge>,
        registry: McpSessionRegistry,
        disabled_tools: &[&'static str],
    ) -> Self {
        let session_id = registry.register();
        Self {
            bridge,
            tool_router: Self::build_router(disabled_tools),
            registry: Some(registry),
            session_id,
            started_at: Instant::now(),
        }
    }

    fn build_router(disabled_tools: &[&'static str]) -> ToolRouter<Self> {
        let mut router = Self::tool_router();
        for name in disabled_tools {
            router.disable_route(*name);
        }
        router
    }
}

/// Macro to generate dispatch arms for `batch_execute`.
///
/// Each arm deserializes the JSON params into the appropriate type and calls
/// the corresponding tool method, unwrapping the `Parameters` wrapper.
macro_rules! dispatch_tools {
    ($self:expr, $tool:expr, $params:expr, $validate_only:expr, [
        $( $name:literal => $method:ident ( $ptype:ty ) ),* $(,)?
    ]) => {
        match $tool {
            $(
                $name => {
                    match serde_json::from_value::<$ptype>($params) {
                        // In validate-only (dry_run) mode the params parsed, so
                        // report the op as valid without invoking the handler —
                        // no state is touched.
                        Ok(p) => if $validate_only {
                            Ok(format!("dry_run OK: '{}' params valid", $name))
                        } else {
                            Ok($self.$method(Parameters(p)).await)
                        },
                        Err(e) => Err(format!("Error: invalid params for '{}': {}", $name, e)),
                    }
                }
            )*
            _ => {
                let known: &[&str] = &[ $( $name ),* ];
                let similar = find_similar($tool, known, 3);
                let hint = if similar.is_empty() {
                    String::new()
                } else {
                    format!(". Did you mean {}?", similar.join(", "))
                };
                Err(format!("Error: unknown tool '{}'{}", $tool, hint))
            }
        }
    }
}

/// Classify a tool's `"Error: ..."` response: bridge/engine-state errors
/// (caller asked for an entity that does not exist, or the engine queue refused
/// the command) should surface at `warn!` so transient races are visible at the
/// default `info` filter. Pure validation rejections (bad enum, range, schema)
/// stay at `debug!`.
fn is_bridge_error(msg: &str) -> bool {
    msg.contains("not found") || msg.contains("failed to send")
}

/// Character cap for a param summary line.
const SUMMARY_MAX: usize = 60;

/// Truncate `s` to at most `max` characters, appending an ellipsis when cut.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A short, human-readable one-line summary of a tool call's params for the
/// success log / activity console — never the full JSON. Covers the common
/// shapes: `batch_execute` reports its op count, other array-shaped tools their
/// item count, and single-target tools a few `key=value` scalars.
///
/// Takes the argument object by reference so the hot `call_tool` path doesn't
/// clone a (potentially large) params map just to describe it.
fn summarize_params(obj: &serde_json::Map<String, serde_json::Value>) -> String {
    // batch_execute: the operation count is the useful summary.
    if let Some(ops) = obj.get("operations").and_then(|v| v.as_array()) {
        return format!("{} ops", ops.len());
    }
    // Other array-shaped tools (batch_json create_instrument / import_sample
    // / add_note …): report the first array field's length.
    if let Some((key, arr)) = obj.iter().find_map(|(k, v)| v.as_array().map(|a| (k, a))) {
        return truncate_chars(&format!("{key}={}", arr.len()), SUMMARY_MAX);
    }
    // Single-target tools: a few scalar fields as `key=value`.
    let mut parts = Vec::new();
    for (k, v) in obj {
        let scalar = match v {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        };
        if let Some(s) = scalar {
            parts.push(format!("{k}={s}"));
            if parts.len() >= 3 {
                break;
            }
        }
    }
    if !parts.is_empty() {
        return truncate_chars(&parts.join(", "), SUMMARY_MAX);
    }
    truncate_chars(&serde_json::to_string(obj).unwrap_or_default(), SUMMARY_MAX)
}

/// [`summarize_params`] for a whole params [`serde_json::Value`] (the
/// `dispatch_tool` path, where params aren't pre-split into an object).
fn summarize_value(params: &serde_json::Value) -> String {
    match params.as_object() {
        Some(obj) => summarize_params(obj),
        None => truncate_chars(&params.to_string(), SUMMARY_MAX),
    }
}

impl SynthMcpServer {
    /// Dispatch a tool call by name with JSON params, returning the result text
    /// and whether it represents a failure.
    ///
    /// Used by `batch_execute`, which needs the failure verdict for its
    /// success / stop-on-error / rollback accounting. We compute it here — the
    /// same `result_is_failure` classification that drives the log severity below
    /// — and hand it back so the caller doesn't re-parse the (possibly JSON)
    /// result string a second time.
    async fn dispatch_tool(
        &self,
        tool: &str,
        params: serde_json::Value,
        validate_only: bool,
    ) -> (String, bool) {
        let started = Instant::now();
        // Summarize the params before they're moved into the dispatch, so the
        // failure logs below show what the call did — e.g. `12 ops`,
        // `flt-1.cutoff=800` — without dumping full JSON.
        let param_summary = summarize_value(&params);
        // Isolate a panicking sub-op (e.g. a panic inside a `block_in_place`
        // bridge call) so one bad op surfaces as an error instead of killing
        // the tokio worker and dropping the session.
        let result = match run_catching_panic(
            self.session_id,
            tool,
            self.dispatch_tool_inner(tool, params, validate_only),
        )
        .await
        {
            Ok(r) => r,
            Err(msg) => Err(format!("Error: tool '{tool}' panicked: {msg}")),
        };
        let elapsed_ms = started.elapsed().as_millis();
        match result {
            // A hard dispatch rejection (unknown tool, invalid params, panic) is
            // always a failure and always worth a warn.
            Err(msg) => {
                tracing::warn!(
                    session_id = self.session_id,
                    tool,
                    elapsed_ms,
                    error = %msg,
                    "MCP batch dispatch rejected tool call"
                );
                (msg, true)
            }
            // Detect tool-reported failures structurally via `result_is_failure`
            // (the same gate the rollback/stop path uses), not a raw `"Error:"`
            // prefix: a `batch_json` tool (create_instrument / import_sample /
            // duplicate_sample) whose items *all* fail returns
            // `{ "<noun>": [], "errors": [..] }` — valid JSON with no leading
            // "Error:", which a prefix check would miss and log at `trace!`.
            Ok(s) if result_is_failure(&s) => {
                if is_bridge_error(&s) {
                    tracing::warn!(
                        session_id = self.session_id,
                        tool,
                        elapsed_ms,
                        error = %s,
                        "MCP tool returned bridge/engine-state error"
                    );
                } else {
                    tracing::debug!(
                        session_id = self.session_id,
                        tool,
                        elapsed_ms,
                        error = %s,
                        "MCP tool returned validation error"
                    );
                }
                (s, true)
            }
            Ok(s) => {
                // A batch sub-op success. The whole `batch_execute` call is
                // already logged at info by `call_tool`, so keep the per-op line
                // at trace: the activity-console capture layer only keeps
                // `debug`+, so a 1000-op batch's successes don't flood (and evict
                // the visible history from) the bounded in-memory ring. Sub-op
                // *failures* below stay at warn/debug and remain captured. Raise
                // to trace on stderr with `RUST_LOG=synth_mcp=trace` when needed.
                tracing::trace!(
                    session_id = self.session_id,
                    tool,
                    elapsed_ms,
                    params = %param_summary,
                    "MCP batch sub-op succeeded"
                );
                (s, false)
            }
        }
    }

    /// Names of every tool registered with the rmcp tool-router.
    ///
    /// Exposed for the `batch_execute` dispatch-coverage guard test, which
    /// asserts every router-registered tool is also reachable through the
    /// hand-maintained dispatch table below (so the two can't drift).
    #[doc(hidden)]
    #[must_use]
    pub fn router_tool_names(&self) -> Vec<String> {
        self.tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect()
    }

    /// Dispatch a tool by name exactly as `batch_execute` would — for tests.
    ///
    /// Pass a scalar `params` (e.g. `0`): it fails deserialization for every
    /// tool's parameter struct, so no tool body runs (avoiding side effects and
    /// blocking project-IO calls). The only observable distinction is whether
    /// the dispatch table knows the tool (`unknown tool` vs `invalid params`).
    #[doc(hidden)]
    pub async fn dispatch_tool_for_test(
        &self,
        tool: &str,
        params: serde_json::Value,
    ) -> Result<String, String> {
        self.dispatch_tool_inner(tool, params, false).await
    }

    /// Run `batch_execute` exactly as a client would, from a JSON param value —
    /// for tests of the dry_run / rollback orchestration.
    #[doc(hidden)]
    pub async fn batch_execute_for_test(&self, params: serde_json::Value) -> String {
        match serde_json::from_value::<BatchExecuteParam>(params) {
            Ok(p) => self.batch_execute(Parameters(p)).await,
            Err(e) => format!("Error: invalid batch params: {e}"),
        }
    }

    async fn dispatch_tool_inner(
        &self,
        tool: &str,
        params: serde_json::Value,
        validate_only: bool,
    ) -> Result<String, String> {
        dispatch_tools!(self, tool, params, validate_only, [
            // Read operations
            "list_instruments" => list_instruments(NoParams),
            "get_instrument_profiles" => get_instrument_profiles(NoParams),
            "get_instrument_info" => get_instrument_info(InstrumentIdParam),
            "list_modules" => list_modules(InstrumentIdParam),
            "get_module_info" => get_module_info(ModuleParam),
            "get_connections" => get_connections(InstrumentIdParam),
            "get_mod_matrix_routings" => get_mod_matrix_routings(InstrumentIdParam),
            "set_mod_matrix_script" => set_mod_matrix_script(SetModMatrixScriptParam),
            "get_parameter" => get_parameter(GetParameterParam),
            "get_engine_status" => get_engine_status(NoParams),
            "get_version" => get_version(NoParams),
            "get_graph_diagnostics" => get_graph_diagnostics(InstrumentIdParam),
            "get_project_schema" => get_project_schema(NoParams),
            "lint_project" => lint_project(NoParams),
            "get_ui_snapshot" => get_ui_snapshot(InstrumentIdParam),

            // Module types & discovery
            "list_module_types" => list_module_types(ListModuleTypesParam),
            "get_module_type_info" => get_module_type_info(GetModuleTypeInfoParam),
            "search_modules" => search_modules(SearchModulesParam),
            "list_port_types" => list_port_types(NoParams),
            "get_yams_reference" => get_yams_reference(NoParams),
            "check_connection" => check_connection(CheckConnectionParam),

            // Parameters
            "set_parameter" => set_parameter(SetParametersParam),

            // Notes
            "note_on" => note_on(NoteOnParam),
            "note_off" => note_off(NoteOffParam),

            // Example patches
            "list_example_patches" => list_example_patches(NoParams),
            "load_example_patch" => load_example_patch(LoadExamplePatchParam),
            "auto_layout" => auto_layout(NoParams),

            // Module management
            "add_module" => add_module(AddModulesParam),
            "remove_module" => remove_module(RemoveModulesParam),
            "connect" => connect(ConnectMultipleParam),
            "disconnect" => disconnect(ConnectMultipleParam),
            "clear_graph" => clear_graph(InstrumentIdParam),
            "insert_module_between" => insert_module_between(InsertModuleBetweenParam),

            // Instrument lifecycle
            "create_instrument" => create_instrument(CreateInstrumentParam),
            "delete_instrument" => delete_instrument(DeleteInstrumentsParam),
            "rename_instrument" => rename_instrument(RenameInstrumentParam),
            "set_instrument_description" => set_instrument_description(SetInstrumentDescriptionParam),
            "set_instrument_color" => set_instrument_color(SetInstrumentColorParam),
            "set_patch_color" => set_patch_color(SetPatchColorParam),
            "set_patch_description" => set_patch_description(SetPatchDescriptionParam),
            "set_module_description" => set_module_description(SetModuleDescriptionParam),
            "set_sidechain_source" => set_sidechain_source(SetSidechainSourceParam),
            "set_instrument_mixer" => set_instrument_mixer(SetInstrumentMixerParam),
            "set_allocator_config" => set_allocator_config(SetAllocatorConfigParam),
            "set_instrument_midi_channel" => set_instrument_midi_channel(SetInstrumentMidiChannelParam),
            "set_instrument_category" => set_instrument_category(SetInstrumentCategoryParam),

            // Song
            "get_song_info" => get_song_info(NoParams),
            "set_song_tempo" => set_song_tempo(SetSongTempoParam),
            "set_tempo_at" => set_tempo_at(SetTempoAtParam),
            "remove_tempo_at" => remove_tempo_at(RemoveTempoAtParam),
            "get_tempo_map" => get_tempo_map(NoParams),
            "set_song_name" => set_song_name(SetSongNameParam),
            "set_song_author" => set_song_author(SetSongAuthorParam),
            "set_song_description" => set_song_description(SetSongDescriptionParam),
            "set_song_time_signature" => set_song_time_signature(SetSongTimeSignatureParam),
            "set_transport_loop" => set_transport_loop(SetTransportLoopParam),
            "clear_transport_loop" => clear_transport_loop(NoParams),

            // Patterns
            "list_patterns" => list_patterns(NoParams),
            "delete_pattern" => delete_pattern(DeletePatternsParam),
            "rename_pattern" => rename_pattern(RenamePatternParam),
            "set_pattern_description" => set_pattern_description(SetPatternDescriptionParam),
            "set_pattern_length" => set_pattern_length(SetPatternLengthParam),
            "duplicate_pattern" => duplicate_pattern(DuplicatePatternParam),
            "create_pattern" => create_pattern(CreatePatternsParam),

            // Notes in patterns
            "list_notes" => list_notes(PatternIdParam),
            "remove_note" => remove_note(RemoveNotesParam),
            "add_note" => add_note(AddNotesParam),
            "update_note" => update_note(UpdateNotesParam),
            "replace_notes" => replace_notes(ReplaceNotesParam),
            "clear_pattern" => clear_pattern(ClearPatternParam),

            // Bake a pattern's bound note graph (or per-note ornaments) into
            // plain notes.
            "freeze_pattern" => freeze_pattern(PatternIdParam),
            "set_note_ornament" => set_note_ornament(SetNoteOrnamentParam),

            // Note Grid (pooled note-processing graphs)
            "list_note_graphs" => list_note_graphs(NoParams),
            "get_note_graph" => get_note_graph(NoteGraphIdParam),
            "get_note_graphs" => get_note_graphs(GetNoteGraphsParam),
            "create_note_graph" => create_note_graph(CreateNoteGraphParam),
            "set_note_graph_metadata" => set_note_graph_metadata(SetNoteGraphMetadataParam),
            "duplicate_note_graph" => duplicate_note_graph(NoteGraphIdParam),
            "delete_note_graph" => delete_note_graph(DeleteNoteGraphParam),
            "add_note_graph_module" => add_note_graph_module(AddNoteGraphModuleParam),
            "set_note_graph_module" => set_note_graph_module(SetNoteGraphModuleParam),
            "set_note_graph_script" => set_note_graph_script(SetNoteGraphScriptParam),
            "remove_note_graph_module" => remove_note_graph_module(RemoveNoteGraphModuleParam),
            "connect_note_graph" => connect_note_graph(ConnectNoteGraphParam),
            "set_pattern_note_graph" => set_pattern_note_graph(SetPatternNoteGraphParam),
            "set_note_note_graph" => set_note_note_graph(SetNoteNoteGraphParam),

            // Mod Grid
            "list_mod_graphs" => list_mod_graphs(NoParams),
            "get_mod_graph" => get_mod_graph(ModGraphIdParam),
            "get_mod_graphs" => get_mod_graphs(GetModGraphsParam),
            "create_mod_graph" => create_mod_graph(CreateModGraphParam),
            "set_mod_graph_metadata" => set_mod_graph_metadata(SetModGraphMetadataParam),
            "delete_mod_graph" => delete_mod_graph(DeleteModGraphParam),
            "set_mod_graph_scope" => set_mod_graph_scope(SetModGraphScopeParam),
            "assign_mod_graph" => assign_mod_graph(AssignModGraphParam),
            "add_mod_graph_node" => add_mod_graph_node(AddModGraphNodeParam),
            "remove_mod_graph_node" => remove_mod_graph_node(RemoveModGraphNodeParam),
            "set_mod_graph_node" => set_mod_graph_node(SetModGraphNodeParam),
            "connect_mod_graph" => connect_mod_graph(ConnectModGraphParam),
            "disconnect_mod_graph" => disconnect_mod_graph(DisconnectModGraphParam),
            "list_mod_targets" => list_mod_targets(ListModTargetsParam),

            // Tracks
            "list_tracks" => list_tracks(NoParams),
            "create_track" => create_track(CreateTracksParam),
            "set_track_mixer" => set_track_mixer(SetTrackMixerParam),
            "set_track_instrument" => set_track_instrument(SetTrackInstrumentParam),
            "rename_track" => rename_track(RenameTrackParam),
            "set_track_description" => set_track_description(SetTrackDescriptionParam),
            "set_track_color" => set_track_color(SetTrackColorParam),
            "delete_track" => delete_track(DeleteTracksParam),

            // Return busses (effect sends)
            "list_return_busses" => list_return_busses(NoParams),
            "create_return_bus" => create_return_bus(CreateReturnBusParam),
            "delete_return_bus" => delete_return_bus(DeleteReturnBusesParam),
            "set_return_bus_mixer" => set_return_bus_mixer(SetReturnBusMixerParam),
            "set_return_bus_color" => set_return_bus_color(SetReturnBusColorParam),
            "set_return_bus_description" => set_return_bus_description(SetReturnBusDescriptionParam),
            "rename_return_bus" => rename_return_bus(RenameReturnBusParam),
            "set_track_send" => set_track_send(SetTrackSendParam),
            "remove_track_send" => remove_track_send(RemoveTrackSendsParam),
            "set_return_send" => set_return_send(SetReturnSendParam),
            "remove_return_send" => remove_return_send(RemoveReturnSendsParam),
            "add_return_effect" => add_return_effect(AddReturnEffectsParam),
            "remove_return_effect" => remove_return_effect(RemoveReturnEffectsParam),
            "set_return_effect_parameter" => set_return_effect_parameter(SetReturnEffectParameterParam),
            "set_return_effect_enabled" => set_return_effect_enabled(SetReturnEffectEnabledParam),
            "reorder_return_effect" => reorder_return_effect(ReorderReturnEffectParam),
            "get_master_volume" => get_master_volume(NoParams),
            "set_master_volume" => set_master_volume(SetMasterVolumeParam),
            "list_master_effects" => list_master_effects(NoParams),
            "add_master_effect" => add_master_effect(AddMasterEffectsParam),
            "remove_master_effect" => remove_master_effect(RemoveMasterEffectsParam),
            "set_master_effect_parameter" => set_master_effect_parameter(SetMasterEffectParameterParam),
            "set_master_effect_enabled" => set_master_effect_enabled(SetMasterEffectEnabledParam),
            "reorder_master_effect" => reorder_master_effect(ReorderMasterEffectParam),

            // Arrangement
            "remove_placement" => remove_placement(RemovePlacementsParam),
            "list_arrangement" => list_arrangement(NoParams),
            "place_pattern" => place_pattern(PlacePatternsParam),

            // Automation
            "add_automation_points" => add_automation_points(AddAutomationPointsParam),
            "list_automation_lanes" => list_automation_lanes(PatternIdParam),
            "get_instrument_automation_targets" => get_instrument_automation_targets(InstrumentIdParam),
            "get_automation_points" => get_automation_points(GetAutomationPointsParam),
            "remove_automation_points" => remove_automation_points(RemoveAutomationPointsParam),
            "clear_automation_lane" => clear_automation_lane(ClearAutomationLaneParam),
            "scale_automation_lane" => scale_automation_lane(ScaleAutomationLaneParam),
            "offset_automation_lane" => offset_automation_lane(OffsetAutomationLaneParam),
            "copy_automation_lane" => copy_automation_lane(CopyAutomationLaneParam),
            "get_automation_summary" => get_automation_summary(GetAutomationSummaryParam),

            // Transport
            "seq_play" => seq_play(NoParams),
            "seq_stop" => seq_stop(NoParams),
            "seq_seek" => seq_seek(SeqSeekParam),

            // Build instruments
            "build_instrument" => build_instrument(BuildInstrumentsParam),
            "rebuild_instrument_preserve_automation" => rebuild_instrument_preserve_automation(RebuildInstrumentParam),
            "apply_example_patch" => apply_example_patch(ApplyExamplePatchParam),
            "set_song" => set_song(SetSongParam),

            // Project
            "new_project" => new_project(NoParams),
            "save_project" => save_project(ProjectPathParam),
            "save_patch" => save_patch(SavePatchParam),
            "load_project" => load_project(ProjectPathParam),
            "optimize_project" => optimize_project(NoParams),

            // Samples
            "list_samples" => list_samples(ListSamplesParam),
            "import_sample" => import_sample(ImportSampleParam),
            "delete_sample" => delete_sample(DeleteSamplesParam),
            "rename_sample" => rename_sample(RenameSampleParam),
            "set_sample_description" => set_sample_description(SetSampleDescriptionParam),
            "set_sample_root_note" => set_sample_root_note(SetSampleRootNoteParam),
            "normalize_sample" => normalize_sample(SampleIdsParam),
            "reverse_sample" => reverse_sample(SampleIdsParam),
            "trim_sample_silence" => trim_sample_silence(SampleIdsParam),
            "set_sample_loop" => set_sample_loop(SetSampleLoopParam),
            "set_sample_crop" => set_sample_crop(SetSampleCropParam),
            "export_sample" => export_sample(ExportSampleParam),
            "get_sample_info" => get_sample_info(SampleIdParam),
            "duplicate_sample" => duplicate_sample(SampleIdsParam),

            // Sampler module
            "assign_sample_to_module" => assign_sample_to_module(AssignSampleParam),
            "get_sampler_state" => get_sampler_state(SamplerModuleParam),
            "set_sampler_parameter" => set_sampler_parameter(SetSamplerParameterParam),

            // Audio input
            "list_input_devices" => list_input_devices(NoParams),
            "get_input_state" => get_input_state(NoParams),

            // Music analysis
            "analyze_harmony" => analyze_harmony(AnalyzeHarmonyParam),
            "analyze_pattern" => analyze_pattern(AnalyzePatternParam),
            "analyze_drum_groove" => analyze_drum_groove(AnalyzeDrumGrooveParam),
            "analyze_bass_drum_lock" => analyze_bass_drum_lock(AnalyzeBassDrumLockParam),
            "analyze_harmonic_function" => analyze_harmonic_function(AnalyzeHarmonicFunctionParam),
            "analyze_mix_bus" => analyze_mix_bus(AnalyzeMixBusParam),
            "render_to_wav" => render_to_wav(RenderToWavParam),
            "analyze_spectrum" => analyze_spectrum(AnalyzeSpectrumParam),
            "analyze_spectrogram" => analyze_spectrogram(AnalyzeSpectrogramParam),
            "analyze_sample_spectrum" => analyze_sample_spectrum(AnalyzeSampleSpectrumParam),
            "analyze_sample_spectrogram" => analyze_sample_spectrogram(AnalyzeSampleSpectrogramParam),
            "compare_spectra" => compare_spectra(CompareSpectraParam),
            "compare_envelopes" => compare_envelopes(CompareEnvelopesParam),
            "analyze_master_chain" => analyze_master_chain(AnalyzeMasterChainParam),
            "analyze_return_busses" => analyze_return_busses(AnalyzeReturnBussesParam),
            "compare_mix_before_after" => compare_mix_before_after(CompareMixBeforeAfterParam),
            "auto_gain_stage" => auto_gain_stage(AutoGainStageParam),
            "analyze_section" => analyze_section(AnalyzeSectionParam),
            "analyze_masking_matrix" => analyze_masking_matrix(AnalyzeMaskingMatrixParam),
            "analyze_instrument_range" => analyze_instrument_range(AnalyzeInstrumentRangeParam),
            "analyze_velocity_response" => analyze_velocity_response(AnalyzeVelocityResponseParam),
            "validate_instrument_audio" => validate_instrument_audio(ValidateInstrumentAudioParam),
            "analyze_arrangement" => analyze_arrangement(AnalyzeArrangementParam),
            "analyze_form_map" => analyze_form_map(AnalyzeFormMapParam),
            "find_motifs" => find_motifs(FindMotifsParam),
            "analyze_hook_strength" => analyze_hook_strength(AnalyzeHookStrengthParam),
            "analyze_tension_curve" => analyze_tension_curve(AnalyzeTensionCurveParam),
            "suggest_music_fixes" => suggest_music_fixes(SuggestMusicFixesParam),

            // Symbolic composition helpers
            "generate_chord" => generate_chord(GenerateChordParam),
            "create_chord_progression_pattern" => create_chord_progression_pattern(CreateChordProgressionPatternParam),
            "transpose_notes" => transpose_notes(TransposeNotesParam),
            "quantize_notes_to_scale" => quantize_notes_to_scale(QuantizeNotesToScaleParam),
            "quantize_notes_to_grid" => quantize_notes_to_grid(QuantizeNotesToGridParam),
        ])
    }
}

impl Drop for SynthMcpServer {
    fn drop(&mut self) {
        if let Some(registry) = &self.registry {
            let lifetime_ms = self.started_at.elapsed().as_millis();
            tracing::info!(
                session_id = self.session_id,
                lifetime_ms,
                "MCP server instance dropped"
            );
            registry.unregister(self.session_id);
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SynthMcpServer {
    /// Override the macro-generated `call_tool` to isolate panics. A tool body
    /// that panics (e.g. inside a `block_in_place` bridge call) would otherwise
    /// unwind into the tokio worker thread and kill it, taking the whole MCP
    /// session with it (observed as `404 Session not found` on the next
    /// request). Catching the unwind here turns it into a normal tool error.
    /// parking_lot locks don't poison, so the bridge stays usable afterwards.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let tool_name = request.name.clone();
        // Summarize args before `request` is moved into the call context, so the
        // per-call log line (and the activity-log console) shows what the call
        // did — e.g. `batch_execute (12 ops)`, `set_parameter (flt-1.cutoff=…)`.
        let param_summary = request
            .arguments
            .as_ref()
            .map_or_else(String::new, summarize_params);
        let started = Instant::now();
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let outcome =
            run_catching_panic(self.session_id, &tool_name, self.tool_router.call(tcc)).await;
        let elapsed_ms = started.elapsed().as_millis();

        // `call_tool` is the one choke point every top-level tool call passes
        // through (single calls don't go through `dispatch_tool` — that's the
        // batch sub-op path). Log each call at info so it lands in the console
        // at the default level; a tool-reported failure (in-band `Error:` text
        // or the `is_error` flag) is demoted to warn.
        match outcome {
            Ok(Ok(result)) => {
                let failed = result.is_error.unwrap_or(false)
                    || result
                        .content
                        .iter()
                        .any(|c| c.as_text().is_some_and(|t| result_is_failure(&t.text)));
                if failed {
                    tracing::warn!(
                        target: "synth_mcp::call",
                        session_id = self.session_id,
                        tool = %tool_name,
                        elapsed_ms,
                        params = %param_summary,
                        "MCP tool call failed"
                    );
                } else {
                    tracing::info!(
                        target: "synth_mcp::call",
                        session_id = self.session_id,
                        tool = %tool_name,
                        elapsed_ms,
                        params = %param_summary,
                        "MCP tool call"
                    );
                }
                Ok(result)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    target: "synth_mcp::call",
                    session_id = self.session_id,
                    tool = %tool_name,
                    elapsed_ms,
                    error = %e,
                    "MCP tool call errored"
                );
                Err(e)
            }
            Err(msg) => {
                tracing::warn!(
                    target: "synth_mcp::call",
                    session_id = self.session_id,
                    tool = %tool_name,
                    elapsed_ms,
                    error = %msg,
                    "MCP tool call panicked"
                );
                Err(ErrorData::internal_error(
                    format!("tool '{tool_name}' panicked: {msg}"),
                    None,
                ))
            }
        }
    }

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
             Envelope → amplifier cv (volume shaping), Envelope → filter cutoff_cv (filter sweep).\n\
             LFO → any cv input for modulation.\n\n\
             ## Building instruments\n\
             Use `build_instrument` for one-call instrument creation, or step-by-step: \
             `create_instrument` → `add_module` (multiple) → `set_parameter` → `connect`.\n\n\
             ## Discovery tools\n\
             Use these to understand available modules and valid connections before building:\n\
             - `get_module_type_info` — get ports, parameters (with ranges/units/choices), and \
               signal flow hints for a single module type by key (e.g. 'osc', 'flt'). \
               Lighter than `list_module_types` when you know which module you need.\n\
             - `search_modules` — filter modules by category ('voice'/'effect'), port signal \
               type ('audio'/'control'/'gate'/'midi'), or text query. Use this to find modules \
               with specific capabilities.\n\
             - `list_port_types` — reference of all signal types with descriptions, value ranges, \
               and compatibility rules.\n\
             - `check_connection` — validate a proposed connection before making it. Reports \
               port direction errors, signal type incompatibilities, and lists available ports.\n\
             - `list_module_types` — full catalog of all modules (use sparingly, large response).\n\n\
             ## Sequencer\n\
             Songs have **tracks** and **patterns**. Patterns contain notes and automation. \
             Patterns are placed on tracks in the **arrangement** timeline. \
             Use `create_pattern` → `add_note` → `create_track` → `place_pattern` to build songs.\n\n\
             ## Batch operations\n\
             Most mutating tools accept either a single item or an array, so you can create or \
             change many things in one call — prefer that over many single calls:\n\
             - `set_parameter`, `add_note`, `update_note`, `connect`, `create_pattern`, \
               `create_track`, `place_pattern`, `build_instrument`, and `set_song` (whole song).\n\
             - Generic: `batch_execute` — run up to 50 tool calls in a single request. \
               Accepts an array of `{tool, params}` objects, executes sequentially, \
               and returns per-item results. Use for cross-domain orchestration \
               (e.g. instrument setup + sequencer config in one call).",
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
                    tracing::warn!(
                        session_id = self.session_id,
                        "MCP client sent 'initialized' notification but peer_info was unavailable"
                    );
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
        let mut resources: Vec<Resource> = Vec::new();

        // Module type resources
        if let Ok(types) = self.bridge.list_module_types() {
            for info in &types {
                let mut r = Resource::new(
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
                resources.push(r);
            }
        }

        // Example patch resources
        if let Ok(patches) = self.bridge.list_example_patches() {
            for patch in &patches {
                let slug = patch.name.to_ascii_lowercase().replace(' ', "-");
                let mut r = Resource::new(format!("synth://patches/{slug}"), patch.name.clone());
                r.description = Some(format!(
                    "{}: {} | {} modules, {} connections",
                    patch.category, patch.description, patch.module_count, patch.connection_count
                ));
                r.mime_type = Some("application/json".into());
                resources.push(r);
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
            ResourceTemplate::new("synth://module-types/{type_key}", "Module Type")
                .with_description("Detailed info about a synth module type (ports, parameters)")
                .with_mime_type("application/json"),
            ResourceTemplate::new("synth://patches/{name}", "Example Patch")
                .with_description(
                    "Full patch data (modules, connections, parameters) for an example patch",
                )
                .with_mime_type("application/json"),
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
        description = "Auto-infer per-instrument profiles (role, envelope shape, pitch role, \
                       register, texture) for every instrument that at least one track routes to. \
                       Role values: drums, bass, lead, pad, pluck, keys, fx, unknown — each with a \
                       confidence in [0.0, 1.0] and a signal trail that explains the classification. \
                       Same inference path that `analyze_harmony`'s `exclude_drums = true` default \
                       uses; expose it directly to debug or override the classification. Manual \
                       `set_instrument_category` always wins (reports as `manual-override`)."
    )]
    async fn get_instrument_profiles(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_instrument_profiles() {
            Ok(profiles) => to_json(&profiles),
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
        description = "Get all active Mod Matrix routings across every Mod Matrix module in the instrument. Slot rows include semantic source IDs (e.g. 'lfo-1', 'env-2', or 'velocity'/'mod_wheel' for non-module sources) and dotted destination IDs (e.g. 'flt-1.cutoff'), plus amount in -1..1 and enabled flag. A slot with a YAMS control script (Step 2) also reports its `script` source text — then the offset is the script's output, not amount × source. Inactive slots (None → None, no script) are filtered out."
    )]
    async fn get_mod_matrix_routings(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_mod_matrix_routings(params.0.instrument_id) {
            Ok(routings) => to_json(&routings),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Install or clear a YAMS script on a Mod Matrix (`mmx-N`), Script (`scr-N`), OR AudioScript (`asc-N`) module slot (Step 2) — despite the name, `module_id` may be any of the three. With a non-empty `source`, the script is compiled (a compile error is returned with diagnostics) and its `out` value becomes the slot's output: a Mod Matrix slot's modulation offset (replacing amount × source), a Script module's `outN` port value, or an AudioScript module's per-sample audio-rate program. An empty `source` clears the slot back to scalar/silent behaviour (an `asc` with no script passes 0). `slot` is 1-based, matching get_mod_matrix_routings (Mod Matrix: 1..=16; Script module: 1..=8, driving out1..out8; AudioScript: slot 1). YAMS reads `src`-bound module outputs/params (e.g. `src lfo = lfo-1.out`) plus macros (velocity, mod_wheel, …) and context (gate, age, cr, sr, note_hz), and assigns a normalized value to `out`. Read installed scripts back via get_mod_matrix_routings (mmx) or get_module_info (scr/asc `scripts` array); see get_yams_reference for the language."
    )]
    async fn set_mod_matrix_script(&self, params: Parameters<SetModMatrixScriptParam>) -> String {
        let p = params.0;
        match self
            .bridge
            .set_mod_matrix_script(p.instrument_id, &p.module_id, p.slot, &p.source)
        {
            Ok(()) if p.source.trim().is_empty() => {
                format!("OK: cleared script on {} slot {}", p.module_id, p.slot)
            }
            Ok(()) => format!("OK: installed script on {} slot {}", p.module_id, p.slot),
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
        description = "Get build/version info for the running application: version, build \
                       timestamp (ISO 8601 / RFC 3339 UTC, e.g. 2026-07-03T14:30:00Z), git \
                       commit hash, branch, and whether the working tree had uncommitted \
                       changes at build time. Git fields are null when the binary was built \
                       outside a git checkout."
    )]
    async fn get_version(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_version() {
            Ok(info) => to_json(&info),
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
        description = "Return the authoritative on-disk JSON Schema for `.pertyproj` project files \
                       plus the build version that generated it. Use this to validate or diff project \
                       files against the exact committed schema — it avoids the introspection-vs-disk \
                       encoding drift you'd get from reading parameter values live (e.g. an enum reported \
                       numerically here but stored as a string on disk)."
    )]
    async fn get_project_schema(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_project_schema() {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Lint the whole project: run graph diagnostics over every instrument and \
                       aggregate them into one report. Surfaces behavioural issues schema validation \
                       can't — unconnected ports, silent voices, feedback loops, missing audio paths — \
                       per instrument, with total error/warning/info counts. A healthy project reports \
                       error_count = 0 and warning_count = 0. Use after loading a project or before export."
    )]
    async fn lint_project(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.lint_project() {
            Ok(report) => to_json(&report),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Play one or more MIDI notes (note on) — pass several to strike a whole chord in one call. Use note=60 for middle C, velocity=100 for moderate strength."
    )]
    async fn note_on(&self, params: Parameters<NoteOnParam>) -> String {
        for n in &params.0.notes {
            if let Err(e) = validate_midi_note(n.note) {
                return format!("Error: {e}");
            }
            if let Err(e) = validate_velocity(n.velocity) {
                return format!("Error: {e}");
            }
            if let Err(e) = validate_midi_channel(n.channel.unwrap_or(1)) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for n in &params.0.notes {
            let channel = n.channel.unwrap_or(1);
            match self.bridge.note_on(n.note, n.velocity, channel) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("note {}: {e}", n.note)),
            }
        }
        batch_msg(ok_count, "notes on", &[], &errors)
    }

    #[tool(
        description = "Stop one or more MIDI notes (note off). Use the same note numbers as the corresponding note_on."
    )]
    async fn note_off(&self, params: Parameters<NoteOffParam>) -> String {
        for n in &params.0.notes {
            if let Err(e) = validate_midi_note(n.note) {
                return format!("Error: {e}");
            }
            if let Err(e) = validate_midi_channel(n.channel.unwrap_or(1)) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for n in &params.0.notes {
            let channel = n.channel.unwrap_or(1);
            match self.bridge.note_off(n.note, channel) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("note {}: {e}", n.note)),
            }
        }
        batch_msg(ok_count, "notes off", &[], &errors)
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

        let audio = ContentBlock::audio(encoded, "audio/wav");

        let text = ContentBlock::text(format!(
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
        description = "Render a note offline and return quantitative analysis of the audio: detected fundamental, peak/RMS, DC offset, clip count, RMS and centroid envelopes over time, and top spectral peaks at attack/sustain/release. Use this instead of `preview_note` when you want metrics rather than audio bytes — far cheaper to inspect than a WAV roundtrip and gives consistent measurements across calls."
    )]
    async fn analyze_note(
        &self,
        params: Parameters<AnalyzeNoteParam>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_midi_note(params.0.note).map_err(mcp_err)?;
        validate_velocity(params.0.velocity).map_err(mcp_err)?;
        let duration_ms = params.0.duration_ms.unwrap_or(500);
        let tail_ms = params.0.tail_ms.unwrap_or(500);
        #[expect(clippy::cast_precision_loss, reason = "millisecond values fit in f32")]
        validate_range("duration_ms", duration_ms as f32, 1.0, 30000.0).map_err(mcp_err)?;
        #[expect(clippy::cast_precision_loss, reason = "millisecond values fit in f32")]
        validate_range("tail_ms", tail_ms as f32, 1.0, 30000.0).map_err(mcp_err)?;

        if let Some(expected) = params.0.expected_note {
            validate_midi_note(expected).map_err(mcp_err)?;
        }
        if let Some(window) = params.0.envelope_window_ms {
            validate_range("envelope_window_ms", window, 1.0, 5000.0).map_err(mcp_err)?;
        }

        let result = tokio::task::block_in_place(|| {
            self.bridge.analyze_note(
                params.0.instrument_id,
                params.0.note,
                params.0.velocity,
                duration_ms,
                tail_ms,
                params.0.expected_note,
                params.0.envelope_window_ms,
            )
        })
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

        let json = serde_json::to_string_pretty(&result)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Render N seconds of the master bus offline and return mix-level metrics: integrated/short-term-max/momentary-max LUFS (ITU-R BS.1770-4), sample peak in dBFS, true peak in dBTP (4× oversampled per BS.1770-4 Annex 2 — catches inter-sample overshoots that emerge after DA conversion), RMS in dBFS, crest factor, 4-band frequency-balance RMS energies (sub/low/mid/high), stereo correlation, mid/side RMS, stereo width, mono-compatibility score (0..1 — how well L+R survive a mono sum), and a clipped-sample count. Use this to judge whether a mix is balanced, too quiet/loud, narrow, anti-phase, or clipping (sample or inter-sample). LUFS-S requires ≥ 3 s of audio; shorter renders report -200.0 for that field. Renders the song from `start_tick` (default 0) for `duration_seconds` (default 10, max 300) using the engine snapshot — deterministic and offline. Pass `include_per_track: true` to also receive a per-track breakdown (one soloed render per audible track) so you can tell which track is responsible for clipping, dominant energy, or sub-bass — costs roughly O(track_count) extra render time. This is the same breakdown as analyze_section's, but keyed off a duration window rather than an explicit tick range."
    )]
    async fn analyze_mix_bus(&self, params: Parameters<AnalyzeMixBusParam>) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_mix_bus(
                duration,
                params.0.start_tick,
                params.0.include_per_track,
                scope,
            )
        })
    }

    #[tool(
        description = "Render the arrangement offline and write it to a 32-bit float stereo WAV file on disk, returning the path plus stats (sample_rate, frames, peak). Deterministic and offline — the same render analyze_mix_bus uses. Pass instrument_id to solo one instrument so the file is a clean single-source fingerprint (done against a clone, your project is untouched); omit it for the full mix. This is the building block for external timbre-matching: write your candidate patch to a WAV, then run your own FFT / spectral-distance against a reference WAV (e.g. a real SID render). Renders from start_tick (default 0) for duration_seconds (default 10, max 300). Use render_quality 'full' (default) for spectral work — 'draft' truncates everything above 11 kHz."
    )]
    async fn render_to_wav(&self, params: Parameters<RenderToWavParam>) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.render_to_wav(
                params.0.path.clone(),
                duration,
                params.0.start_tick,
                params.0.instrument_id,
                scope,
            )
        })
    }

    #[tool(
        description = "Detailed spectrum of an offline render: detected partials (frequency + amplitude + harmonic number + cents deviation), a voiced/unvoiced verdict, and timbre descriptors — spectral centroid (brightness), flatness (0 pure tone … 1 noise), rolloff, aggregate inharmonicity, and odd/even harmonic ratio. These separate timbres the 4-band analyze_mix_bus energy metric cannot: a plain triangle, a ring-modulated triangle, and a metallic carrier have near-identical 4-band energy but very different partial structure. Pass instrument_id to fingerprint one instrument in isolation (clone-based; your project is untouched); f0_hint to sharpen harmonic tagging; log_bins > 0 to add log-spaced magnitude bins for compare_spectra. The f0 detector (McLeod/NSDF) reports unvoiced (f0 null) for noise so noise frames don't emit a garbage fundamental. Renders from start_tick (default 0) for duration_seconds (default 10, max 300), deterministic and offline; use render_quality 'full' (default) for spectral work."
    )]
    async fn analyze_spectrum(&self, params: Parameters<AnalyzeSpectrumParam>) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_spectrum(
                duration,
                params.0.start_tick,
                params.0.instrument_id,
                params.0.f0_hint,
                params.0.max_partials,
                params.0.log_bins,
                scope,
            )
        })
    }

    #[tool(
        description = "Spectrogram of an offline render: the requested window is rendered ONCE and a sliding FFT returns one full spectrum (partials + voiced verdict + descriptors, same as analyze_spectrum) per hop_ms, analysing window_len_ms per frame. Use this when a sound's identity is its time evolution — e.g. a Commodore-64 SID voice whose spectrum switches every ~20 ms (pitched triangle frame vs chip-noise frame): the per-frame `voiced` flag reads that alternation directly. Far cheaper than calling analyze_spectrum many times — it is one render and O(1) MCP calls, not N. hop_ms defaults to 20 (≈ one PAL video frame), window_len_ms to 40; frames are capped at 4096. Renders from start_tick (default 0) for duration_seconds (default 10, max 300), deterministic and offline."
    )]
    async fn analyze_spectrogram(&self, params: Parameters<AnalyzeSpectrogramParam>) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_spectrogram(
                duration,
                params.0.start_tick,
                params.0.instrument_id,
                params.0.f0_hint,
                params.0.max_partials,
                params.0.log_bins,
                params.0.hop_ms,
                params.0.window_len_ms,
                scope,
            )
        })
    }

    #[tool(
        description = "Run the same detailed spectral analysis as analyze_spectrum, but over an imported sample or a WAV file on disk instead of a render — detected partials, voiced/unvoiced verdict, and timbre descriptors (centroid, flatness, rolloff, inharmonicity, odd/even ratio). Use this to fingerprint a real reference recording (e.g. a SID render written by sidplayfp, or any WAV) in exactly the same units as analyze_spectrum, then feed both into compare_spectra to drive a timbre-matching loop. sample_id_or_path is either a numeric imported-sample id or a path to a WAV file; the audio is analyzed at its native sample rate and downmixed to mono. Pass log_bins > 0 to enable the broadband distance in compare_spectra."
    )]
    async fn analyze_sample_spectrum(
        &self,
        params: Parameters<AnalyzeSampleSpectrumParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_sample_spectrum(
                params.0.sample_id_or_path.clone(),
                params.0.f0_hint,
                params.0.max_partials,
                params.0.log_bins,
                params.0.start_ms,
                params.0.window_len_ms,
            )
        })
    }

    #[tool(
        description = "Per-frame spectrogram of an imported sample or WAV file — the sample counterpart of analyze_spectrogram. Slides an FFT across the decoded audio at its NATIVE sample rate and returns one spectrum per hop (time_seconds + the same descriptor analyze_spectrum gives: partials, voiced verdict, centroid/flatness/rolloff/inharmonicity). Use it to see the time evolution of a real reference recording — e.g. a SID render alternating pitched/noise every ~20 ms — which a single aggregate analyze_sample_spectrum hides. Frames line up with analyze_spectrogram of the equivalent render so you can compare per-frame. sample_id_or_path is a numeric imported-sample id or a path to a WAV. hop_ms defaults to 20 (≈ one PAL frame), window_len_ms to 40; frame count is capped at 4096 (a warning is added on truncation). Deterministic and offline."
    )]
    async fn analyze_sample_spectrogram(
        &self,
        params: Parameters<AnalyzeSampleSpectrogramParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_sample_spectrogram(
                params.0.sample_id_or_path.clone(),
                params.0.f0_hint,
                params.0.max_partials,
                params.0.log_bins,
                params.0.hop_ms,
                params.0.window_len_ms,
            )
        })
    }

    #[tool(
        description = "Compare two spectra and return how far apart they are, and WHERE. Each side (target, candidate) is either a render (optionally soloing one instrument) or an imported sample / WAV file, so you can compare render↔render, render↔sample, or sample↔sample. Returns two distance scalars to minimise: log_spectral_distance (RMS dB difference over log-frequency bins) and mel_l2_distance (true L2 / Euclidean distance over a log-mel filterbank — perceptually weighted, tracks audible timbre change more closely; sized by mel_bands, default 40). Plus per-descriptor deltas (candidate − target): centroid_delta_hz (brightness), rolloff_delta_hz (filter-slope steepness — 12 vs 24 dB/oct), flatness_delta, inharmonicity_delta, and odd_even_ratio_delta_db (odd/even harmonic balance in dB — encodes pulse duty cycle, so use it to match pulse width); voicing_mismatch (a pitched-vs-noise gross mismatch) with its penalty reported separately in voicing_penalty_db (60 dB on a mismatch, else 0) rather than folded into log_spectral_distance — so the spectral scalar keeps ranking candidates even against a silent/unvoiced target window (add the two for the old combined score); and the high-value lists missing_partials (strong in the target, absent in the candidate — what your patch is failing to produce) and extra_partials (present in the candidate, not the target). This closes the timbre-matching loop: fingerprint a reference (analyze_sample_spectrum of a real SID render), measure your candidate, read missing_partials to know which frequencies to add, and watch the distances fall as you adjust parameters. NOTE: the default (aggregate) distances average the whole window into one spectrum, so on staccato / silence-dominated / time-varying material they go blind to quiet-in-time content (a decaying release tail) and stop ranking candidates — set time_resolved: true for those, which frames both sources, aligns them, masks on target energy, and returns per-frame time_resolved_lsd / worst_frames instead. Deterministic and offline."
    )]
    async fn compare_spectra(&self, params: Parameters<CompareSpectraParam>) -> String {
        let p = params.0;
        let scope = crate::bridge::AnalysisScope::from_flags(
            p.include_all,
            p.include_master_effects,
            p.include_return_effects,
            crate::bridge::RenderQuality::parse(p.render_quality.as_deref()),
        );
        let to_source = |s: &SpectrumSourceParam| crate::bridge::SpectrumSource {
            sample_id_or_path: s.sample_id_or_path.clone(),
            instrument_id: s.instrument_id,
            start_tick: s.start_tick,
            duration_seconds: s.duration_seconds,
            start_ms: s.start_ms,
            window_len_ms: s.window_len_ms,
        };
        let target = to_source(&p.target);
        let candidate = to_source(&p.candidate);
        // Mask/align default to on; only the explicit "none" string turns them off.
        let time_resolved = crate::bridge::TimeResolvedOptions {
            enabled: p.time_resolved.unwrap_or(false),
            hop_ms: p.hop_ms,
            frame_len_ms: p.frame_len_ms,
            mask_target_energy: p
                .mask
                .as_deref()
                .is_none_or(|m| !m.trim().eq_ignore_ascii_case("none")),
            align_envelope: p
                .align
                .as_deref()
                .is_none_or(|a| !a.trim().eq_ignore_ascii_case("none")),
            align_max_ms: p.align_max_ms,
        };
        run_blocking_json(move || {
            self.bridge.compare_spectra(
                target,
                candidate,
                p.f0_hint,
                p.max_partials,
                p.log_bins,
                p.mel_bands,
                scope,
                time_resolved,
            )
        })
    }

    #[tool(
        description = "Compare the amplitude CONTOURS (ADSR shape over time) of two sources — the time-domain counterpart of compare_spectra. FFT-based tools miss how a sound evolves; a SID voice's identity is largely its envelope (attack punch, decay, sustain, hard-restart click). Each side (target, candidate) is a render (optionally soloing one instrument) or an imported sample / WAV. Extracts an RMS envelope from each, peak-normalises them (shape is compared independent of loudness — use analyze_mix_bus for level), and aligns them with dynamic time warping. Returns: dtw_distance (the scalar to minimise — normalised warp distance between the contours, tolerant of small timing differences); a per-side breakdown (attack_ms, decay_ms, sustain_level, release_ms, plus attack-transient crest_factor_db and energy_rise_db — the 'punch' of the onset); and the candidate − target deltas for each. Use it to check your patch's envelope tracks a reference: watch dtw_distance fall as you tune ADSR, and read crest_factor_delta_db to see if you're missing the reference's attack punch. release_ms needs note_duration_ms; omit it and the shape distance still works. Deterministic and offline."
    )]
    async fn compare_envelopes(&self, params: Parameters<CompareEnvelopesParam>) -> String {
        let p = params.0;
        let scope = crate::bridge::AnalysisScope::from_flags(
            p.include_all,
            p.include_master_effects,
            p.include_return_effects,
            crate::bridge::RenderQuality::parse(p.render_quality.as_deref()),
        );
        let to_source = |s: &SpectrumSourceParam| crate::bridge::SpectrumSource {
            sample_id_or_path: s.sample_id_or_path.clone(),
            instrument_id: s.instrument_id,
            start_tick: s.start_tick,
            duration_seconds: s.duration_seconds,
            start_ms: s.start_ms,
            window_len_ms: s.window_len_ms,
        };
        let target = to_source(&p.target);
        let candidate = to_source(&p.candidate);
        run_blocking_json(move || {
            self.bridge.compare_envelopes(
                target,
                candidate,
                p.envelope_window_ms,
                p.note_duration_ms,
                p.transient_window_ms,
                scope,
            )
        })
    }

    #[tool(
        description = "Incremental per-effect breakdown of the master bus. Renders the chain input (post-return mix, before any master effect) once, then re-renders the master output with the chain truncated after each effect — so you can see exactly what each master effect does to the mix. Each stage reports the full mix metrics at that point plus the delta the effect introduced: lufs_delta, peak/true-peak/rms delta in dB, stereo_width_delta, crest_delta_db (negative = more compressed dynamics), and gain_reduction_db (positive = the effect attenuated level, e.g. a limiter). Use this to verify a master limiter is catching peaks, an EQ is shaping balance, or to find the effect that is crushing your dynamics or narrowing the image. The master effect chain is always reconstructed; pass `include_return_effects: true` to feed the return wet signal into the chain input. Costs one offline render per master effect plus one for the input — O(effect_count). Renders from `start_tick` (default 0) for `duration_seconds` (default 10, max 300), deterministic and offline."
    )]
    async fn analyze_master_chain(&self, params: Parameters<AnalyzeMasterChainParam>) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        // The master chain is always measured; only the surrounding stages are
        // optional. `from_flags` with master_effects=Some(true) forces it on.
        let scope = crate::bridge::AnalysisScope::from_flags(
            None,
            Some(true),
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge
                .analyze_master_chain(duration, params.0.start_tick, scope)
        })
    }

    #[tool(
        description = "Per-return-bus contribution to the master mix. Renders the full mix once, then re-renders with each return bus muted in turn (against a clone — your project is untouched), and reports how much each return adds: lufs_delta, peak/true-peak/rms delta in dB, and stereo_width_delta (all full − muted, so positive = the return makes the mix louder/wider/peakier). Use this to see which send effect (reverb, delay, …) is eating your headroom, widening the image, or contributing the most loudness. Because a return's wet signal cannot be cleanly soloed away from the dry track sum, the muted-difference is the honest contribution measure; returns sum in parallel, so each delta is that bus's marginal contribution. The return-bus effect chains are always reconstructed; pass `include_master_effects: true` to measure through the processed master output. Costs one offline render for the full mix plus one per return bus — O(return_count). Renders from `start_tick` (default 0) for `duration_seconds` (default 10, max 300), deterministic and offline."
    )]
    async fn analyze_return_busses(&self, params: Parameters<AnalyzeReturnBussesParam>) -> String {
        let duration = params.0.duration_seconds.unwrap_or(10.0);
        // Return-bus chains are always measured; only the surrounding stages are
        // optional. `from_flags` with return_effects=Some(true) forces them on.
        let scope = crate::bridge::AnalysisScope::from_flags(
            None,
            params.0.include_master_effects,
            Some(true),
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge
                .analyze_return_busses(duration, params.0.start_tick, scope)
        })
    }

    #[tool(
        description = "A/B a mix change. Call with action='capture' to render the current master mix and store it as a baseline, make your change (EQ, levels, effects, …), then call action='compare' to re-render and get the deltas: lufs_delta, peak/true-peak/rms delta in dB, crest_delta_db (positive = more dynamic), stereo_width_delta (positive = wider), mono_compat_delta. Compare re-renders with the exact same window and signal chain the baseline used, so the deltas reflect only your change. The baseline is per-session and is never written to the project; capturing again overwrites it. Use this to confirm a tweak did what you intended (e.g. 'did adding the limiter actually lower the true peak without crushing dynamics?')."
    )]
    async fn compare_mix_before_after(
        &self,
        params: Parameters<CompareMixBeforeAfterParam>,
    ) -> String {
        let p = params.0;
        let duration = p.duration_seconds.unwrap_or(10.0);
        let scope = crate::bridge::AnalysisScope::from_flags(
            p.include_all,
            p.include_master_effects,
            p.include_return_effects,
            crate::bridge::RenderQuality::parse(p.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge
                .compare_mix_before_after(&p.action, duration, p.start_tick, p.label, scope)
        })
    }

    #[tool(
        description = "Measure the master mix and set the master fader to reach a target loudness without breaching a true-peak ceiling. \
                       Renders the song (default 10 s) through the master + return effect chains at 44.1 kHz, measures integrated LUFS \
                       and true peak, then adjusts the master volume. The fader is post-effects, so loudness and peak scale linearly — \
                       no iteration. Returns measured vs. predicted LUFS/true-peak, the applied gain, old/new master volume, and \
                       `limited_by` (whether the target, the true-peak ceiling, or the fader range bound the result). Mutates master volume."
    )]
    async fn auto_gain_stage(&self, params: Parameters<AutoGainStageParam>) -> String {
        let p = params.0;
        if let Err(e) = validate_range("target_lufs", p.target_lufs, -60.0, 0.0) {
            return validation_err(e);
        }
        let ceiling = p.true_peak_ceiling.unwrap_or(-1.0);
        if let Err(e) = validate_range("true_peak_ceiling", ceiling, -24.0, 0.0) {
            return validation_err(e);
        }
        let duration = p.duration_seconds.unwrap_or(10.0);
        run_blocking_json(|| {
            self.bridge
                .auto_gain_stage(p.target_lufs, ceiling, duration, p.start_tick)
        })
    }

    #[tool(
        description = "Render an explicit arrangement range [start_tick, end_tick) offline and return the same mix-bus metrics as analyze_mix_bus (LUFS-I/S/M, sample peak, true peak in dBTP, RMS, crest, banded energy, stereo correlation, mid/side, mono-compatibility, clipped samples). Use this when you want to A/B verses vs. choruses, compare a buildup to a drop, or inspect a specific musical passage rather than a fixed-duration window from the song start. Pass `include_per_track: true` to also receive a per-track breakdown (one soloed render per audible track) so you can tell which track is responsible for clipping, dominant energy, or sub-bass — costs roughly O(track_count) extra render time. Per-track `metrics.peak`/`metrics.rms` include pan-law attenuation (-3 dB at center pan: a center-panned source with internal peak 1.0 reports ~0.7071). Per-track `pre_master_peak` analytically reverses the instrument's pan + volume attenuation from the per-channel peaks and reports the patch's internal signal peak directly, so you can see internal clipping that would otherwise be hidden by a quiet pan-down."
    )]
    async fn analyze_section(&self, params: Parameters<AnalyzeSectionParam>) -> String {
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_section(
                params.0.start_tick,
                params.0.end_tick,
                params.0.include_per_track,
                scope,
            )
        })
    }

    #[tool(
        description = "Pairwise spectral-masking report across every audible track in an arrangement range. Renders each audible track soloed once, then for every pair (a, b) compares their per-band RMS in the 4-band split (sub 0-100 Hz, low 100-500 Hz, mid 500-2000 Hz, high 2 kHz+) used elsewhere. Each pair carries the per-band overlap energy, the dominance margin in dB, an overall conflict_score in 0..=1, the dominant track id when one side leads by >6 dB on the worst-overlap band, and a textual hint such as 'Pad(2) masks Lead(3) in mid (500-2000 Hz)'. Pairs are returned sorted by descending conflict_score so the most contested combination appears first. Renders are O(track_count) (same as analyze_section with include_per_track=true); the pair matrix itself is in-memory and O(N²). Use when a section sounds muddy or when one element is being smothered and you need to know which other track is doing it."
    )]
    async fn analyze_masking_matrix(
        &self,
        params: Parameters<AnalyzeMaskingMatrixParam>,
    ) -> String {
        let scope = crate::bridge::AnalysisScope::from_flags(
            params.0.include_all,
            params.0.include_master_effects,
            params.0.include_return_effects,
            crate::bridge::RenderQuality::parse(params.0.render_quality.as_deref()),
        );
        run_blocking_json(|| {
            self.bridge.analyze_masking_matrix(
                params.0.arrangement_start_tick,
                params.0.arrangement_end_tick,
                params.0.top_pairs,
                scope,
            )
        })
    }

    #[tool(
        description = "Symbolic harmonic analysis of a pattern or arrangement range. Walks notes in time order, groups simultaneous notes into chord events, identifies chord symbols (e.g. Cm7, F7sus4), infers the most likely key via Krumhansl-Schmuckler correlation, and reports an in-key ratio, out-of-scale pitch classes, and a composite harmonic stability score. Pure symbolic — no audio rendering. Use to verify chord progressions, spot accidentally out-of-key notes, and reason about the harmonic shape of generated music. Pass `pattern_id` for one pattern, or omit it (with optional `arrangement_start_tick` / `arrangement_end_tick`) for an arrangement range."
    )]
    async fn analyze_harmony(&self, params: Parameters<AnalyzeHarmonyParam>) -> String {
        match self.bridge.analyze_harmony(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.grouping_ticks,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Symbolic structural analysis of a single pattern. Reports density (notes per bar/beat, active ratio), pitch shape (range, mean, distinct count, duration-weighted pitch-class histogram), velocity dynamics (min/max/mean/std/range), rhythm (max/mean polyphony, distinct onsets/durations, inter-onset-interval mean+std, regularity score), and bar-level repetition (distinct bar signatures, repetition score). Pure symbolic — no audio rendering. Use to verify whether a pattern is interesting (varied vs. flat, dense vs. sparse, repetitive vs. through-composed) without listening, and as a prerequisite for variation generation heuristics."
    )]
    async fn analyze_pattern(&self, params: Parameters<AnalyzePatternParam>) -> String {
        match self.bridge.analyze_pattern(params.0.pattern_id) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Sweep an instrument across a MIDI note range and run the same render-and-analyze pipeline as analyze_note at each step. Returns per-note metrics (fundamental, pitch error, pitch confidence, peak/RMS, centroid, clipped-sample count) plus cross-step issues (silent notes, likely-aliased notes — high centroid + low pitch confidence in the top octaves, lost pitch tracking — fundamental off by more than an octave, clipping notes, level spread in dB between loudest and quietest non-silent step). Use to catch patches that work at C4 in analyze_note and fall apart at C6 (aliasing) or C2 (energy loss). One render per step — `step_semitones` defaults to 12 (one note per octave); reduce for higher resolution, increase or limit `[low_note, high_note]` for cheaper sweeps."
    )]
    async fn analyze_instrument_range(
        &self,
        params: Parameters<AnalyzeInstrumentRangeParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_instrument_range(
                params.0.instrument_id,
                params.0.low_note,
                params.0.high_note,
                params.0.step_semitones,
                params.0.velocity,
                params.0.duration_ms,
                params.0.tail_ms,
            )
        })
    }

    #[tool(
        description = "Hold one MIDI note and sweep velocity across [velocity_low, velocity_high]. Returns per-velocity amplitude/brightness curves plus monotonicity flags (non_monotonic_amplitude_steps — adjacent pairs where peak fell as velocity rose, non_monotonic_centroid_steps — same for brightness) and a velocity_unresponsive flag (amplitude_range_db < 3 dB across the sweep). Use to confirm a patch actually responds to velocity in a musical way (rising amplitude, brighter filter at higher velocity) instead of being effectively velocity-deaf — common surprise on patches with the wrong envelope→amp routing."
    )]
    async fn analyze_velocity_response(
        &self,
        params: Parameters<AnalyzeVelocityResponseParam>,
    ) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_velocity_response(
                params.0.instrument_id,
                params.0.note,
                params.0.velocity_low,
                params.0.velocity_high,
                params.0.velocity_step,
                params.0.duration_ms,
                params.0.tail_ms,
            )
        })
    }

    #[tool(
        description = "Section-level form analysis. Walks the arrangement (or a single pattern's bars in pattern scope) one bar at a time, builds a duration-weighted pitch-class histogram + note-density + active-track feature row per bar, computes a cosine self-similarity matrix, and merges adjacent similar bars into sections. Sections that match a previously labeled section (similarity >= threshold) reuse its letter; near-matches get a prime (e.g. A'). Returns the per-bar feature rows, the detected sections with per-section stats, and the distinct section count. Pure symbolic — no audio rendering. Pair with `analyze_form_map` for the compact letter-string view. `exclude_drums` defaults to true."
    )]
    async fn analyze_arrangement(&self, params: Parameters<AnalyzeArrangementParam>) -> String {
        match self.bridge.analyze_arrangement(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.similarity_threshold,
            params.0.section_min_bars,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Compact view of the same section clustering as `analyze_arrangement`: one label per bar and a run-length-compressed form string like 'AABA' or 'ABACABA'. Cheaper to read for 'what's the structure of this song?' prompts. Uses the same default similarity threshold (0.85) and section_min_bars (2) merging. Empty bars (no melodic notes) appear as '·' in `bar_labels` and are skipped in the form string."
    )]
    async fn analyze_form_map(&self, params: Parameters<AnalyzeFormMapParam>) -> String {
        match self.bridge.analyze_form_map(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.similarity_threshold,
            params.0.section_min_bars,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Find recurring melodic motifs in the scope. Converts each track's notes into a pitch-interval sequence (signed semitone deltas between consecutive notes in time order, ignoring rests), slides an n-gram window across each track (lengths min_interval_length..=max_interval_length, defaults 3..=6), counts identical interval sequences, and returns the top_n motifs (default 10) that appear at least min_count times (default 3). Transposition-invariant — the same shape rooted at different pitches collapses to one entry. Each motif lists its interval sequence, count, and per-occurrence locations (track id, start tick, bar/beat, first pitch). Pure symbolic — no audio rendering. `exclude_drums` defaults to true."
    )]
    async fn find_motifs(&self, params: Parameters<FindMotifsParam>) -> String {
        match self.bridge.find_motifs(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.min_interval_length,
            params.0.max_interval_length,
            params.0.min_count,
            params.0.top_n,
            params.0.max_occurrences_per_motif,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Single-number 'does this song have a hook?' diagnostic. Runs `find_motifs` internally with min_interval_length (default 3) and min_count (default 3), then scores the result: hook_score = 0.5 × normalized_longest_motif_length + 0.3 × log2(1 + best_count) / log2(1 + total_notes) + 0.2 × coverage_ratio, clamped to [0, 1]. coverage_ratio is the fraction of melodic notes that participate in at least one qualifying motif. `strongest_motif` is the longest motif (ties broken by count) if any qualify; absent when the score is 0. Pure symbolic — no audio rendering."
    )]
    async fn analyze_hook_strength(&self, params: Parameters<AnalyzeHookStrengthParam>) -> String {
        match self.bridge.analyze_hook_strength(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.min_interval_length,
            params.0.min_count,
            params.0.max_occurrences_per_motif,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Bar-level tension curve over the scope. Builds per-bar rows from existing analyzers — harmonic_function for chord tension, bar_features for density/register/rhythmic activity, plus (in audio mode) a single offline render sliced per bar for loudness, brightness, band entropy, and stereo width. Returns per-bar values, the cluster-derived section labels (so the caller can map bars to A/B/A'), a peak/trough/mean/std-dev summary, and shape warnings: chorus reprises with lower energy, builds that peak too early, drops that lose low-end, and otherwise monotone curves. `include_audio` defaults to true in arrangement scope and false in pattern scope. No new measurements — pure synthesis on top of the existing analyzers."
    )]
    async fn analyze_tension_curve(&self, params: Parameters<AnalyzeTensionCurveParam>) -> String {
        run_blocking_json(|| {
            self.bridge.analyze_tension_curve(
                params.0.pattern_id,
                params.0.arrangement_start_tick,
                params.0.arrangement_end_tick,
                params.0.include_audio,
                params.0.similarity_threshold,
                params.0.section_min_bars,
                params.0.exclude_drums,
                params.0.exclude_track_ids,
            )
        })
    }

    #[tool(
        description = "Meta-analysis: runs the relevant analyzers across harmony, mix, groove, arrangement, composition, and patch categories, applies a rule set per category, and returns ranked fix suggestions with supporting evidence. No new measurements — every suggestion references metrics already produced by the underlying analyzer tools. `categories` is a subset of [harmony, mix, groove, arrangement, composition, patch] (empty/null = all). `include_audio` (default true) gates the mix-bus / masking / audio-augmented tension-curve checks. `max_suggestions` defaults to 15."
    )]
    async fn suggest_music_fixes(&self, params: Parameters<SuggestMusicFixesParam>) -> String {
        run_blocking_json(|| {
            self.bridge.suggest_music_fixes(
                params.0.pattern_id,
                params.0.arrangement_start_tick,
                params.0.arrangement_end_tick,
                params.0.categories,
                params.0.include_audio,
                params.0.max_suggestions,
                params.0.exclude_drums,
                params.0.exclude_track_ids,
            )
        })
    }

    #[tool(
        description = "Parse a chord symbol (e.g. 'Cm7', 'F#maj7', 'Bbsus4', 'G7sus4', 'C5') and return MIDI notes for the requested voicing rooted at `octave` (default 4 = middle-C octave). Voicings: 'close' (default — notes stacked above the root), 'drop2' (drop the 2nd-highest note an octave), 'drop3' (drop the 3rd-highest), 'open' (drop2+drop3 combined). Pure symbolic — does not touch the song; pair with `add_note` to place. Saves the AI from re-deriving chord intervals by hand on every progression."
    )]
    async fn generate_chord(&self, params: Parameters<GenerateChordParam>) -> String {
        match self.bridge.generate_chord(
            &params.0.symbol,
            params.0.octave.unwrap_or(4),
            params.0.voicing.as_deref(),
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create a new pattern and fill it with a chord progression in one call. Each symbol in `chords` is voiced \
                       like generate_chord and placed as a block of notes spanning `beats_per_chord` (default 4 = one 4/4 bar), \
                       laid end to end. Returns the new pattern id, total length, and a per-chord breakdown. Saves building \
                       pad/glue patterns chord-by-chord with generate_chord + add_notes."
    )]
    async fn create_chord_progression_pattern(
        &self,
        params: Parameters<CreateChordProgressionPatternParam>,
    ) -> String {
        let p = params.0;
        let velocity = p.velocity.unwrap_or(80);
        if velocity > 127 {
            return format!("Error: {}", McpBridgeError::InvalidVelocity(velocity));
        }
        match self.bridge.create_chord_progression_pattern(
            &p.name,
            &p.chords,
            p.beats_per_chord.unwrap_or(4.0),
            p.octave.unwrap_or(4),
            p.voicing.as_deref(),
            velocity,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Transpose every note in `pattern_id` by `semitones` (signed). Notes whose new pitch would leave the 0..127 MIDI range are left untouched and counted in `notes_out_of_range`. When both `scale_tonic` (0..12) and `scale_name` are set, transposed pitches that land off-scale are snapped to the nearest in-scale pitch using `tie_break` ('up'/'down'/'nearest', default 'up') — useful for staying diatonic when the AI shifts a phrase. Scale names: major, minor, harmonic_minor, melodic_minor, dorian, phrygian, lydian, mixolydian, locrian, pentatonic_major, pentatonic_minor, blues, chromatic. Replaces a 20-call sequence of update_note transposes."
    )]
    async fn transpose_notes(&self, params: Parameters<TransposeNotesParam>) -> String {
        match self.bridge.transpose_notes(
            params.0.pattern_id,
            params.0.semitones,
            params.0.scale_tonic,
            params.0.scale_name.as_deref(),
            params.0.tie_break.as_deref(),
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Snap every note in `pattern_id` to the nearest pitch of the given key/scale. Cleans up AI-generated material that drifted off-key. Returns notes_already_in_scale + notes_moved, mean and max absolute correction in semitones. `tie_break` ('up' default / 'down' / 'nearest') decides which way to snap when a pitch is equidistant from two scale degrees. Scale names: major, minor, harmonic_minor, melodic_minor, dorian, phrygian, lydian, mixolydian, locrian, pentatonic_major, pentatonic_minor, blues, chromatic."
    )]
    async fn quantize_notes_to_scale(
        &self,
        params: Parameters<QuantizeNotesToScaleParam>,
    ) -> String {
        match self.bridge.quantize_notes_to_scale(
            params.0.pattern_id,
            params.0.scale_tonic,
            &params.0.scale_name,
            params.0.tie_break.as_deref(),
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Snap note start ticks in `pattern_id` to a `grid_ticks` grid (240 = sixteenth at 960 PPQN, 480 = eighth, 960 = quarter) with optional swing (0..=1, even positions stay / odd push back by up to half-grid), humanization (max ±tick jitter per note), and quantize strength (0..=1; 1.0 = full snap, 0.5 = halfway between original and grid). Humanization is deterministic given the same `humanize_seed` (default 0) — reuse the seed to A/B compare different swing/strength settings without changing the jitter pattern. Returns notes_moved, mean and max tick deltas. Pure symbolic — no rendering."
    )]
    async fn quantize_notes_to_grid(&self, params: Parameters<QuantizeNotesToGridParam>) -> String {
        match self.bridge.quantize_notes_to_grid(
            params.0.pattern_id,
            params.0.grid_ticks,
            params.0.strength,
            params.0.swing,
            params.0.humanize_ticks,
            params.0.humanize_seed,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Symbolic drum-feel analysis. Classifies each drum note via the General MIDI drum map (kick / snare / closed-hat / open-hat / tom / cymbal / clap / other) and reports backbeat strength (snare hits landing on beats 2 and 4), hat subdivision (quarter / 8th / 16th / triplet_8th / triplet_16th / irregular / none), hat density per beat, ghost-note count (snare hits below half the loudest snare velocity), fill candidates (bars whose density exceeds 2× the mean), and bar-level repetition over drum notes. Pure symbolic — no audio rendering. Pass `pattern_id` to analyze one pattern as-is (no drum-track filtering); omit it (with optional `arrangement_start_tick` / `arrangement_end_tick`) to analyze every track auto-classified as Drums by `get_instrument_profiles` (confidence ≥ 0.6). Useful for answering 'why does this beat sound flat?' without listening."
    )]
    async fn analyze_drum_groove(&self, params: Parameters<AnalyzeDrumGrooveParam>) -> String {
        match self.bridge.analyze_drum_groove(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Symbolic kick/bass-lock diagnostics — answers 'does the bass actually work with the beat?' without listening. Identifies drum tracks (Role::Drums, conf ≥ 0.6) and bass tracks (Role::Bass, conf ≥ 0.6) via the same `infer_all_profiles` path that `analyze_harmony`'s drum filter uses, then aligns kick onsets (GM MIDI 35/36) against bass note onsets within `onset_tolerance_ticks` (default 120 = ±1/32-note at 960 PPQN). Reports `lock_score` (matched kicks / total kicks — how often the kick gets bass support), `coverage_score` (matched / total bass onsets — how often the bass has a kick beneath it), kick-only / bass-only counts, and a bass-pitch stability summary (most common pitch class on matched onsets and its share — high share = rooted bass, low share + many PCs = walking or melodic bass). Pass `pattern_id` to analyze a single combined rhythm-section pattern (kicks = GM kick MIDI, bass = everything else); omit it for arrangement scope across track-classified content."
    )]
    async fn analyze_bass_drum_lock(&self, params: Parameters<AnalyzeBassDrumLockParam>) -> String {
        match self.bridge.analyze_bass_drum_lock(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.onset_tolerance_ticks,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Tonal-function analysis on top of analyze_harmony. Runs the same chord-identification + key inference pipeline, then annotates each chord with a scale-degree Roman numeral (I, V7, IV, vii°, …), a function bucket (tonic / subdominant / dominant / other / chromatic), and a 0..1 tension score; detects cadences (authentic V → I, plagal IV → I, half — anything → V, deceptive V → vi) on consecutive chord pairs and reports a function distribution + tension-curve summary. Use this to reason about progression quality and direction — 'does this song actually resolve?' or 'where does the tension peak?'. Pass `pattern_id` for one pattern, or omit it (with optional `arrangement_start_tick` / `arrangement_end_tick`) for an arrangement range. `exclude_drums` defaults to true."
    )]
    async fn analyze_harmonic_function(
        &self,
        params: Parameters<AnalyzeHarmonicFunctionParam>,
    ) -> String {
        match self.bridge.analyze_harmonic_function(
            params.0.pattern_id,
            params.0.arrangement_start_tick,
            params.0.arrangement_end_tick,
            params.0.grouping_ticks,
            params.0.exclude_drums,
            params.0.exclude_track_ids,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
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
        description = "Request auto-layout of modules in the patch view. The GUI applies the layout on the next Rack-view frame, arranging modules by signal flow. If the user is in another view (AcousticWorld, Sequencer, Sample), the request stays pending until they return to Rack."
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
        description = "List all available module types. By default returns the full catalog \
        (every port + parameter per type) — this is hundreds of KB and can exceed the tool-result \
        token cap, so pass brief:true for a compact {type_key, name, category} list, then call \
        get_module_type_info for the one type you want. Use the type_key to add modules with \
        add_module."
    )]
    async fn list_module_types(&self, params: Parameters<ListModuleTypesParam>) -> String {
        if params.0.brief.unwrap_or(false) {
            return match self.bridge.list_module_types_brief() {
                Ok(types) => to_json(&types),
                Err(e) => format!("Error: {e}"),
            };
        }
        match self.bridge.list_module_types() {
            Ok(types) => to_json(&types),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add one or more modules to the instrument's voice graph. Modules appear in the GUI on the next frame. Returns the assigned module IDs (see also list_modules). GUI-only visualizer types (Oscilloscope/Meter/Spectrum) can't be added over MCP — they're flagged gui_only:true in list_module_types."
    )]
    async fn add_module(&self, params: Parameters<AddModulesParam>) -> String {
        let p = params.0;
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for module_type in &p.module_types {
            match self.bridge.add_module(p.instrument_id, module_type) {
                Ok(msg) => oks.push(msg),
                Err(e) => errors.push(format!("{module_type}: {e}")),
            }
        }
        batch_msg(oks.len(), "modules added", &oks, &errors)
    }

    #[tool(
        description = "Remove one or more modules from the instrument's voice graph and disconnect all their cables."
    )]
    async fn remove_module(&self, params: Parameters<RemoveModulesParam>) -> String {
        let p = params.0;
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for module_id in &p.module_ids {
            match self.bridge.remove_module(p.instrument_id, module_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{module_id}: {e}")),
            }
        }
        batch_msg(ok_count, "modules removed", &[], &errors)
    }

    #[tool(
        description = "Connect one or more module port pairs in one call. Returns the number of successful connections and any errors. \
                       Each connection specifies from_module:from_port → to_module:to_port. Port names must match the module's ports (typically 'out'/'in'); the aliases 'output'→'out' and 'input'→'in' are also accepted."
    )]
    async fn connect(&self, params: Parameters<ConnectMultipleParam>) -> String {
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

    #[tool(
        description = "Splice a new module into an existing audio cable in one call (add + disconnect + reconnect). \
                       Re-routes source → new module → destination through the new module's audio ports. \
                       Choose where with one anchor: `after`/`before` (a module id), `after_type`/`before_type` \
                       (a module type — robust across instruments), or the explicit from_module/from_port/to_module/to_port \
                       cable when the path branches. With no anchor it inserts at the end of the audio path, just before output. \
                       The module type must carry audio. On any wiring failure the original cable is restored."
    )]
    async fn insert_module_between(&self, params: Parameters<InsertModuleBetweenParam>) -> String {
        let p = params.0;
        let anchor = match p.resolve_anchor() {
            Ok(a) => a,
            Err(e) => return format!("Error: {e}"),
        };
        match self
            .bridge
            .insert_module_between(p.instrument_id, &p.module_type, anchor)
        {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Quick go/no-go check that an instrument actually produces audio: renders one test note offline \
                       and returns a compact verdict (is_audible, peak/RMS, clipping, fundamental, DC offset) plus warnings. \
                       Use to catch silent or broken patches before wiring them into a song. For full spectral detail use \
                       analyze_instrument_range or analyze_velocity_response."
    )]
    async fn validate_instrument_audio(
        &self,
        params: Parameters<ValidateInstrumentAudioParam>,
    ) -> String {
        let p = params.0;
        let note = p.note.unwrap_or(60);
        let velocity = p.velocity.unwrap_or(100);
        let duration_ms = p.duration_ms.unwrap_or(500);
        let tail_ms = p.tail_ms.unwrap_or(500);
        if note > 127 {
            return format!("Error: {}", McpBridgeError::InvalidMidiNote(note));
        }
        if velocity > 127 {
            return format!("Error: {}", McpBridgeError::InvalidVelocity(velocity));
        }
        match self.bridge.analyze_note(
            p.instrument_id,
            note,
            velocity,
            duration_ms,
            tail_ms,
            None,
            None,
        ) {
            Ok(r) => to_json(&distill_audio_validation(&r)),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Instrument lifecycle ===

    #[tool(
        description = "Create one or more instruments. Returns the array of created instrument infos, each with its assigned ID."
    )]
    async fn create_instrument(&self, params: Parameters<CreateInstrumentParam>) -> String {
        for name in &params.0.names {
            if let Err(e) = validate_name("instrument", name) {
                return format!("Error: {e}");
            }
        }
        let mut infos = Vec::new();
        let mut errors = Vec::new();
        for name in &params.0.names {
            match self.bridge.create_instrument(name) {
                Ok(info) => infos.push(info),
                Err(e) => errors.push(format!("'{name}': {e}")),
            }
        }
        batch_json("created", &infos, &errors)
    }

    #[tool(
        description = "Delete one or more instruments and all their modules. Cannot delete the default instrument (ID 0)."
    )]
    async fn delete_instrument(&self, params: Parameters<DeleteInstrumentsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.instrument_ids {
            match self.bridge.delete_instrument(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", id.as_u64())),
            }
        }
        batch_msg(ok_count, "instruments deleted", &[], &errors)
    }

    #[tool(
        description = "Rename one or more instruments. The name is shown in the UI instrument strip and track selector."
    )]
    async fn rename_instrument(&self, params: Parameters<RenameInstrumentParam>) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_name("instrument", &it.name) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_instrument(it.instrument_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.instrument_id)),
            }
        }
        batch_msg(ok_count, "instruments renamed", &[], &errors)
    }

    #[tool(
        description = "Set or clear the free-text description / intent on an instrument. \
        The description never affects audio and is read back via list_instruments / \
        get_instrument_info. Use it to record why an instrument exists, what role it plays \
        in the song, or any analysis notes you want a future agent (or human) to see. \
        Pass an empty string to clear."
    )]
    async fn set_instrument_description(
        &self,
        params: Parameters<SetInstrumentDescriptionParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_instrument_description(it.instrument_id, &it.description)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.instrument_id)),
            }
        }
        batch_msg(ok_count, "instrument descriptions set", &[], &errors)
    }

    #[tool(
        description = "Set or clear the accent color of one or more instruments from a \
        \"#RRGGBB\" / \"#RRGGBBAA\" hex string (pass \"\" to clear back to the default/auto \
        tint). Never affects audio; paints instruments so the mixer / arrangement is visually \
        scannable (e.g. red kick, blue pad, green bass) and is read back via list_instruments / \
        get_instrument_info. The color travels with the project on save."
    )]
    async fn set_instrument_color(&self, params: Parameters<SetInstrumentColorParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_instrument_color(it.instrument_id, &it.color)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.instrument_id)),
            }
        }
        batch_msg(ok_count, "instrument colors set", &[], &errors)
    }

    #[tool(
        description = "Set or clear the patch-level accent color of one or more instruments from a \
        \"#RRGGBB\" / \"#RRGGBBAA\" hex string (pass \"\" to clear). Distinct from \
        set_instrument_color: this color travels with the patch when it is saved/exported, so a \
        shared patch carries its own suggested tint. Never affects audio; read back via \
        list_instruments / get_instrument_info as patch_color."
    )]
    async fn set_patch_color(&self, params: Parameters<SetPatchColorParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_patch_color(it.instrument_id, &it.color) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.instrument_id)),
            }
        }
        batch_msg(ok_count, "patch colors set", &[], &errors)
    }

    #[tool(
        description = "Set or clear the patch-level description on an instrument's currently \
        loaded patch. This describes the *patch* (sound design intent, how it works, what it's \
        good for) and is distinct from set_instrument_description, which records the \
        instrument's per-instance role in the song. The patch description travels with the \
        patch when saved. Pass \"\" to clear."
    )]
    async fn set_patch_description(&self, params: Parameters<SetPatchDescriptionParam>) -> String {
        match self
            .bridge
            .set_patch_description(params.0.instrument_id, &params.0.description)
        {
            Ok(()) => {
                if params.0.description.is_empty() {
                    format!(
                        "OK: cleared patch description on instrument {}",
                        params.0.instrument_id
                    )
                } else {
                    format!(
                        "OK: set instrument {} patch description ({} chars)",
                        params.0.instrument_id,
                        params.0.description.chars().count()
                    )
                }
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set or clear the free-text description on one or more module instances \
        (what a particular module is for — e.g. \"wobble LFO for the filter cutoff\"). Takes an \
        array of self-contained {instrument_id, module_id, description} items, so a single call \
        can annotate modules across different instruments (mirrors set_instrument_description). \
        Distinct from get_module_type_info, which documents the module *type*. The description \
        travels with the patch when saved and is readable via get_module_info / list_modules. \
        Pass \"\" to clear an item. Max 2000 characters; an item is rejected if the module does \
        not exist."
    )]
    async fn set_module_description(
        &self,
        params: Parameters<SetModuleDescriptionParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_module_description(
                it.instrument_id,
                &it.module_id,
                &it.description,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}:{}: {e}", it.instrument_id, it.module_id)),
            }
        }
        batch_msg(ok_count, "module descriptions set", &[], &errors)
    }

    #[tool(
        description = "Set or clear the song's free-text description (intent / mood / production \
        notes). Pass \"\" to clear. Surfaces in get_song_info."
    )]
    async fn set_song_description(&self, params: Parameters<SetSongDescriptionParam>) -> String {
        match self.bridge.set_song_description(&params.0.description) {
            Ok(()) => format!(
                "OK: set song description ({} chars)",
                params.0.description.chars().count()
            ),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set or clear a pattern's free-text description (its musical intent, e.g. \
        \"chorus drop, half-time feel\"). Pass \"\" to clear. Surfaces in list_patterns."
    )]
    async fn set_pattern_description(
        &self,
        params: Parameters<SetPatternDescriptionParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_pattern_description(it.pattern_id, &it.description)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.pattern_id)),
            }
        }
        batch_msg(ok_count, "pattern descriptions set", &[], &errors)
    }

    #[tool(
        description = "Set or clear a track's free-text description (its role, e.g. \"kick layer\", \
        \"sidechain source\"). Pass \"\" to clear. Surfaces in list_tracks."
    )]
    async fn set_track_description(&self, params: Parameters<SetTrackDescriptionParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_track_description(it.track_id, &it.description)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.track_id)),
            }
        }
        batch_msg(ok_count, "track descriptions set", &[], &errors)
    }

    #[tool(
        description = "Set the display color of one or more tracks from a \"#RRGGBB\" / \"#RRGGBBAA\" hex string \
        (alpha ignored). Paints the arrangement so it is visually scannable. Surfaces in list_tracks."
    )]
    async fn set_track_color(&self, params: Parameters<SetTrackColorParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_track_color(it.track_id, &it.color) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.track_id)),
            }
        }
        batch_msg(ok_count, "track colors set", &[], &errors)
    }

    #[tool(
        description = "Set or clear a sample's free-text description (its intent / source). \
        Pass \"\" to clear. Surfaces in list_samples / get_sample_info."
    )]
    async fn set_sample_description(
        &self,
        params: Parameters<SetSampleDescriptionParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_sample_description(it.sample_id, &it.description)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "sample descriptions set", &[], &errors)
    }

    #[tool(
        description = "Set or clear the sidechain source on an instrument. When set, the \
        engine routes the source instrument's audio output into this instrument's \
        sidechain-capable modules (compressors with sidechain_enabled, envelope followers). \
        Use it for classic pumping/ducking — e.g. let a kick drum sidechain the pad. \
        Pass source = null (or omit) to disable. Self-routing is rejected."
    )]
    async fn set_sidechain_source(&self, params: Parameters<SetSidechainSourceParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_sidechain_source(it.instrument_id, it.source)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.instrument_id)),
            }
        }
        batch_msg(ok_count, "sidechain sources set", &[], &errors)
    }

    #[tool(
        description = "Set mixer state on one or more instruments in a single call. Each item \
        carries an instrument_id plus any of volume (0.0=silent, 1.0=unity, 2.0=max), pan \
        (-1.0=left..1.0=right), muted, solo, and enabled (disabled instruments skip all audio \
        processing — lighter than mute, which still processes but silences output). Omitted \
        fields are left unchanged. When any instrument is soloed, only soloed instruments sound."
    )]
    async fn set_instrument_mixer(&self, params: Parameters<SetInstrumentMixerParam>) -> String {
        // Validate all ranges up front so a bad value rejects the whole call.
        for it in &params.0.items {
            if let Some(v) = it.volume
                && let Err(e) = validate_range("volume", v, 0.0, 2.0)
            {
                return format!("Error: {e}");
            }
            if let Some(p) = it.pan
                && let Err(e) = validate_range("pan", p, -1.0, 1.0)
            {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            let id = it.instrument_id;
            let mut item_err: Option<String> = None;
            if let Some(v) = it.volume
                && let Err(e) = self.bridge.set_instrument_volume(id, v)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(p) = it.pan
                && let Err(e) = self.bridge.set_instrument_pan(id, p)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(m) = it.muted
                && let Err(e) = self.bridge.set_instrument_mute(id, m)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = it.solo
                && let Err(e) = self.bridge.set_instrument_solo(id, s)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(en) = it.enabled
                && let Err(e) = self.bridge.set_instrument_enabled(id, en)
            {
                item_err = Some(e.to_string());
            }
            match item_err {
                None => ok_count += 1,
                Some(e) => errors.push(format!("{}: {e}", id.as_u64())),
            }
        }
        batch_msg(ok_count, "instrument mixer updates applied", &[], &errors)
    }

    #[tool(
        description = "Set voice-allocator config on one or more instruments in a single call. Each \
        item carries an instrument_id plus any of: allocation_mode (Polyphonic | Mono | Legato | \
        Unison), stealing_strategy (None | Oldest | Quietest | LowestPriority | SameNote), \
        unison_detune (0..100 cents, audible only in Unison mode), unison_spread (0.0..1.0 stereo \
        width, audible only in Unison mode), and max_voices (1..=128; applied on the next voice-graph \
        rebuild / project load, not live). Omitted fields are left unchanged. Read the current \
        values back via get_instrument_info."
    )]
    async fn set_allocator_config(&self, params: Parameters<SetAllocatorConfigParam>) -> String {
        // Validate all numeric ranges up front so a bad value rejects the whole
        // call (mode/strategy strings are validated per-item by the bridge).
        for it in &params.0.items {
            if let Some(d) = it.unison_detune
                && let Err(e) = validate_range("unison_detune", d, 0.0, 100.0)
            {
                return format!("Error: {e}");
            }
            if let Some(s) = it.unison_spread
                && let Err(e) = validate_range("unison_spread", s, 0.0, 1.0)
            {
                return format!("Error: {e}");
            }
            if let Some(v) = it.max_voices
                && !(1..=128).contains(&v)
            {
                return format!("Error: max_voices must be in 1..=128, got {v}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            let id = it.instrument_id;
            let mut item_err: Option<String> = None;
            // An item that sets no fields still must name a real instrument,
            // otherwise it would report a phantom success. The field setters
            // below validate existence themselves, so only the all-omitted case
            // needs an explicit check.
            let sets_nothing = it.allocation_mode.is_none()
                && it.stealing_strategy.is_none()
                && it.unison_detune.is_none()
                && it.unison_spread.is_none()
                && it.max_voices.is_none();
            if sets_nothing && let Err(e) = self.bridge.get_instrument_info(id) {
                errors.push(format!("{}: {e}", id.as_u64()));
                continue;
            }
            if let Some(m) = &it.allocation_mode
                && let Err(e) = self.bridge.set_instrument_allocation_mode(id, m)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = &it.stealing_strategy
                && let Err(e) = self.bridge.set_instrument_stealing_strategy(id, s)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(d) = it.unison_detune
                && let Err(e) = self.bridge.set_instrument_unison_detune(id, d)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = it.unison_spread
                && let Err(e) = self.bridge.set_instrument_unison_spread(id, s)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(v) = it.max_voices
                && let Err(e) = self.bridge.set_instrument_max_voices(id, v)
            {
                item_err = Some(e.to_string());
            }
            match item_err {
                None => ok_count += 1,
                Some(e) => errors.push(format!("{}: {e}", id.as_u64())),
            }
        }
        batch_msg(ok_count, "allocator configs updated", &[], &errors)
    }

    #[tool(description = "Set the MIDI channel (1-16) for one or more instruments.")]
    async fn set_instrument_midi_channel(
        &self,
        params: Parameters<SetInstrumentMidiChannelParam>,
    ) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_midi_channel(it.channel) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_instrument_midi_channel(it.instrument_id, it.channel)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.instrument_id)),
            }
        }
        batch_msg(ok_count, "instrument MIDI channels set", &[], &errors)
    }

    #[tool(
        description = "Set the category of one or more instruments (for visualization routing). Categories: Uncategorized, Drums, Bass, Pad, Lead, Arp, Keys, FX."
    )]
    async fn set_instrument_category(
        &self,
        params: Parameters<SetInstrumentCategoryParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_instrument_category(it.instrument_id, &it.category)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.instrument_id)),
            }
        }
        batch_msg(ok_count, "instrument categories set", &[], &errors)
    }

    #[tool(
        description = "Disconnect one or more cables between module ports in one call. \
                       Each connection specifies from_module:from_port → to_module:to_port (same shape as connect)."
    )]
    async fn disconnect(&self, params: Parameters<ConnectMultipleParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for c in &params.0.connections {
            match self.bridge.disconnect(
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
        batch_msg(ok_count, "cables disconnected", &[], &errors)
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
        description = "Add or replace tempo-map points at absolute ticks (960 ticks per quarter note). Each point is a step by default, or set ramp=true for a linear accelerando/ritardando toward the next point. This edits the tempo MAP (position-specific tempo), NOT the global default tempo — use set_song_tempo for that. A point replaces any existing change at the same tick. Array-first: pass multiple points in one call. Inspect the map via get_tempo_map or get_song_info."
    )]
    async fn set_tempo_at(&self, params: Parameters<SetTempoAtParam>) -> String {
        for point in &params.0.points {
            if let Err(e) = validate_range("tempo", point.bpm, 20.0, 999.0) {
                return format!("Error: {e}");
            }
        }
        let points: Vec<(u64, f32, bool)> = params
            .0
            .points
            .iter()
            .map(|p| (p.tick, p.bpm, p.ramp))
            .collect();
        match self.bridge.set_tempo_at(&points) {
            Ok(()) => format!("OK: set {} tempo-map point(s)", points.len()),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Remove tempo-map points at the given absolute ticks. Returns how many were removed. Does not affect the global default tempo (set_song_tempo)."
    )]
    async fn remove_tempo_at(&self, params: Parameters<RemoveTempoAtParam>) -> String {
        match self.bridge.remove_tempo_at(&params.0.ticks) {
            Ok(n) => format!("OK: removed {n} tempo-map point(s)"),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get the tempo map: position-specific tempo changes, sorted by tick. Does not include the global default tempo — see get_song_info for that."
    )]
    async fn get_tempo_map(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_tempo_map() {
            Ok(map) => to_json(&map),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set the transport loop region in beats. When enabled, playback wraps from end_beats back to start_beats. Visible on the arrangement ruler. Use clear_transport_loop to disable, or get_song_info to inspect the current state."
    )]
    async fn set_transport_loop(&self, params: Parameters<SetTransportLoopParam>) -> String {
        if let Err(e) = validate_range("start_beats", params.0.start_beats, 0.0, 9999.0) {
            return validation_err(e);
        }
        if let Err(e) = validate_range("end_beats", params.0.end_beats, 0.0, 9999.0) {
            return validation_err(e);
        }
        match self.bridge.set_transport_loop(
            params.0.start_beats,
            params.0.end_beats,
            params.0.enabled,
        ) {
            Ok(()) => {
                if params.0.enabled {
                    format!(
                        "OK: transport loop {} -> {} beats (enabled)",
                        params.0.start_beats, params.0.end_beats
                    )
                } else {
                    "OK: transport loop stored (disabled)".to_string()
                }
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Clear the transport loop region. Equivalent to set_transport_loop with enabled=false; playback stops wrapping."
    )]
    async fn clear_transport_loop(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.clear_transport_loop() {
            Ok(()) => "OK: transport loop cleared".to_string(),
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
        description = "Delete one or more patterns by ID. Also removes all placements of each pattern."
    )]
    async fn delete_pattern(&self, params: Parameters<DeletePatternsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.pattern_ids {
            match self.bridge.delete_pattern(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "patterns deleted", &[], &errors)
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

    #[tool(description = "Remove one or more notes from a pattern by note ID.")]
    async fn remove_note(&self, params: Parameters<RemoveNotesParam>) -> String {
        let p = params.0;
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for note_id in &p.note_ids {
            match self.bridge.remove_note(p.pattern_id, *note_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("note {note_id}: {e}")),
            }
        }
        batch_msg(ok_count, "notes removed", &[], &errors)
    }

    // === Sequencer: pattern freeze ===

    #[tool(
        description = "Bake a pattern's note processing into concrete notes (Model-A freeze), for hand-editing. A bound note graph bakes (the binding is cleared; the pooled graph survives); otherwise per-note ornaments and note-scope articulation bake. DESTRUCTIVE: the generative setup cannot be un-baked — re-bind the graph to restore it. Returns the resulting note count."
    )]
    async fn freeze_pattern(&self, params: Parameters<PatternIdParam>) -> String {
        match self.bridge.freeze_pattern(params.0.pattern_id) {
            Ok((note_count, dropped)) => {
                // `dropped > 0` = a graph node hit the 128-event cap during the
                // bake; surface it so the overflow isn't silently swallowed.
                let warning = (dropped > 0).then(|| {
                    format!(
                        "{dropped} events dropped during freeze (a graph node hit the \
                         128-event cap)"
                    )
                });
                to_json(&serde_json::json!({
                    "pattern_id": params.0.pattern_id,
                    "note_count": note_count,
                    "dropped_events": dropped,
                    "warning": warning,
                }))
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set or clear per-note timed-repeat ornaments (flam/drag/ruff/roll/grace note) on one or more notes. Each item gives the Ornament JSON to set, or null to clear it. Ornaments expand each note into its figure at playback time."
    )]
    async fn set_note_ornament(&self, params: Parameters<SetNoteOrnamentParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.items {
            let (pattern_id, note_id) = (it.pattern_id, it.note_id);
            match self
                .bridge
                .set_note_ornament(pattern_id, note_id, it.ornament)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("pattern {pattern_id} note {note_id}: {e}")),
            }
        }
        batch_msg(ok_count, "note ornaments updated", &[], &errors)
    }

    // === Sequencer: Note Grid (pooled note-processing graphs) ===

    #[tool(
        description = "List every pooled Note Grid graph in summary form: id, name, description, color, module/connection counts, and how many patterns bind it."
    )]
    async fn list_note_graphs(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_note_graphs() {
            Ok(graphs) => to_json(&graphs),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get one Note Grid graph in full detail: its modules (in processing order, each with id/kind/config JSON) and connections (from/to/port/to_input)."
    )]
    async fn get_note_graph(&self, params: Parameters<NoteGraphIdParam>) -> String {
        match self.bridge.get_note_graph(params.0.graph_id) {
            Ok(detail) => to_json(&detail),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get full detail for selected Note Grid graphs, or every graph when graph_ids is omitted. Returns stable graph-id order and a per-graph detail/error result."
    )]
    async fn get_note_graphs(&self, params: Parameters<GetNoteGraphsParam>) -> String {
        let mut ids = match params.0.graph_ids {
            Some(ids) => ids,
            None => match self.bridge.list_note_graphs() {
                Ok(graphs) => graphs.into_iter().map(|graph| graph.id).collect(),
                Err(e) => return format!("Error: {e}"),
            },
        };
        ids.sort_unstable();
        ids.dedup();
        let results: Vec<_> = ids
            .into_iter()
            .map(|graph_id| match self.bridge.get_note_graph(graph_id) {
                Ok(detail) => serde_json::json!({"graph_id": graph_id, "detail": detail}),
                Err(e) => serde_json::json!({"graph_id": graph_id, "error": e.to_string()}),
            })
            .collect();
        to_json(&results)
    }

    #[tool(
        description = "Create an empty pooled Note Grid graph. Returns the new graph id. Add modules with add_note_graph_module, wire them with connect_note_graph, then bind it to a pattern with set_pattern_note_graph."
    )]
    async fn create_note_graph(&self, params: Parameters<CreateNoteGraphParam>) -> String {
        let p = params.0;
        match self
            .bridge
            .create_note_graph(p.name, p.description, p.color)
        {
            Ok(id) => to_json(&serde_json::json!({ "graph_id": id })),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Partially update name, description, and color for one or more Note Grid graphs."
    )]
    async fn set_note_graph_metadata(
        &self,
        params: Parameters<SetNoteGraphMetadataParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for item in params.0.items {
            match self.bridge.set_note_graph_metadata(
                item.graph_id,
                item.name,
                item.description,
                item.color,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("graph {}: {e}", item.graph_id)),
            }
        }
        batch_msg(ok_count, "note graph metadata updated", &[], &errors)
    }

    #[tool(
        description = "Duplicate a pooled Note Grid graph — nodes, connections, metadata, and editor layout — as '<name> copy'. Use before diverging a shared graph for one pattern (pair with set_pattern_note_graph to repoint). Returns the new graph's id."
    )]
    async fn duplicate_note_graph(&self, params: Parameters<NoteGraphIdParam>) -> String {
        match self.bridge.duplicate_note_graph(params.0.graph_id) {
            Ok(id) => to_json(&serde_json::json!({ "graph_id": id })),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Delete one or more pooled Note Grid graphs. DESTRUCTIVE: every pattern that binds a deleted graph is unbound (falls back to dry playback). Returns the per-graph count of patterns that were unbound."
    )]
    async fn delete_note_graph(&self, params: Parameters<DeleteNoteGraphParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for graph_id in params.0.graph_ids {
            match self.bridge.delete_note_graph(graph_id) {
                Ok(unbound) => oks.push(format!("graph {graph_id} (unbound {unbound} patterns)")),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "note graphs deleted", &oks, &errors)
    }

    #[tool(
        description = "Add one or more modules to Note Grid graphs. Each module is externally-tagged NoteModuleConfig JSON (Processor/Euclidean/ProbabilityGate/NoteLfo/StepLfo/NoteEnvelope/NoteScriptTransform/NoteDelay/Ratchet). For a NoteScriptTransform, add it with a source then compile it with set_note_graph_script. Returns each new module id."
    )]
    async fn add_note_graph_module(&self, params: Parameters<AddNoteGraphModuleParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for it in params.0.items {
            let graph_id = it.graph_id;
            let module = match serde_json::to_value(it.module) {
                Ok(module) => module,
                Err(e) => {
                    errors.push(format!("{graph_id}: {e}"));
                    continue;
                }
            };
            match self
                .bridge
                .add_note_graph_module(graph_id, module, it.description)
            {
                Ok(module_id) => oks.push(format!("graph {graph_id} @ module {module_id}")),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "note graph modules added", &oks, &errors)
    }

    #[tool(
        description = "Replace a Note Grid module's config in place (config edit), keeping its id and connections. A change that would orphan an existing connection is rejected."
    )]
    async fn set_note_graph_module(&self, params: Parameters<SetNoteGraphModuleParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for item in params.0.items {
            let module = match serde_json::to_value(item.module) {
                Ok(module) => module,
                Err(e) => {
                    errors.push(format!(
                        "graph {} module {}: {e}",
                        item.graph_id, item.module_id
                    ));
                    continue;
                }
            };
            match self.bridge.set_note_graph_module(
                item.graph_id,
                item.module_id,
                module,
                item.description,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "graph {} module {}: {e}",
                    item.graph_id, item.module_id
                )),
            }
        }
        batch_msg(ok_count, "note graph modules updated", &[], &errors)
    }

    #[tool(
        description = "Set a Note Grid NoteScriptTransform node's YAMS note_event source, compile it, and install the program. The script runs per note (1:1): read note_pitch/note_vel/note_dur/tick and value inputs in1..in4, assign out.pitch/out.vel/out.dur/out.gate. Returns the compile status; the source is always saved, and an empty source or a compile error leaves the node pass-through (the diagnostic is in the returned status). Add the node first with add_note_graph_module ({\"NoteScriptTransform\":{\"source\":\"\"}})."
    )]
    async fn set_note_graph_script(&self, params: Parameters<SetNoteGraphScriptParam>) -> String {
        let p = params.0;
        match self
            .bridge
            .set_note_graph_script(p.graph_id, p.module_id, p.source)
        {
            Ok(status) => status,
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Remove a module (and every connection touching it) from a Note Grid graph."
    )]
    async fn remove_note_graph_module(
        &self,
        params: Parameters<RemoveNoteGraphModuleParam>,
    ) -> String {
        let p = params.0;
        match self
            .bridge
            .remove_note_graph_module(p.graph_id, p.module_id)
        {
            Ok(()) => format!("Module {} removed from graph {}", p.module_id, p.graph_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Connect two Note Grid modules (one or many). port is 'note_stream' (the linear spine), 'value', or 'gate'; to_input selects the target's value-input port for modulation edges. Each connection is validated for linearity (one stream in/out per node), acyclicity, and endpoint types."
    )]
    async fn connect_note_graph(&self, params: Parameters<ConnectNoteGraphParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.items {
            let port = it.port.unwrap_or_else(|| "note_stream".to_string());
            let to_input = it.to_input.unwrap_or(0);
            match self
                .bridge
                .connect_note_graph(it.graph_id, it.from, it.to, port, to_input)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("graph {} {}→{}: {e}", it.graph_id, it.from, it.to)),
            }
        }
        batch_msg(ok_count, "note graph connections added", &[], &errors)
    }

    #[tool(
        description = "Bind patterns to Note Grid graphs (one or many). Set graph_id to bind, or null/omitted to clear the binding (the pattern's raw notes + per-note ornaments then play). A bound graph processes the pattern's notes at playback."
    )]
    async fn set_pattern_note_graph(&self, params: Parameters<SetPatternNoteGraphParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.items {
            let pattern_id = it.pattern_id;
            match self.bridge.set_pattern_note_graph(pattern_id, it.graph_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("pattern {pattern_id}: {e}")),
            }
        }
        batch_msg(
            ok_count,
            "pattern note-graph bindings updated",
            &[],
            &errors,
        )
    }

    #[tool(
        description = "Bind individual notes to Note Grid graphs for per-note articulation (flam / strum / arp / echo of one note), one or many. Set graph_id to bind, or null/omitted to clear. The note-scope graph runs on that note's material during source collection — before, and feeding, the pattern-scope graph / rack — and is decorrelated per note. Dangling graph ids are rejected."
    )]
    async fn set_note_note_graph(&self, params: Parameters<SetNoteNoteGraphParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.items {
            match self
                .bridge
                .set_note_note_graph(it.pattern_id, it.note_id, it.graph_id)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "pattern {} note {}: {e}",
                    it.pattern_id, it.note_id
                )),
            }
        }
        batch_msg(ok_count, "note note-graph bindings updated", &[], &errors)
    }

    // === Sequencer: Mod Grid (pooled control-rate modulator graphs) ===

    #[tool(
        description = "List every pooled Mod Grid graph in summary form (id, name, scope, assigned tracks, node/cable counts)."
    )]
    async fn list_mod_graphs(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_mod_graphs() {
            Ok(graphs) => to_json(&graphs),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Full detail of one Mod Grid graph: its nodes (with round-trippable ModNodeConfig JSON) and cables."
    )]
    async fn get_mod_graph(&self, params: Parameters<ModGraphIdParam>) -> String {
        match self.bridge.get_mod_graph(params.0.graph_id) {
            Ok(detail) => to_json(&detail),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get full detail for selected Mod Grid graphs, or every graph when graph_ids is omitted. Returns stable graph-id order and a per-graph detail/error result."
    )]
    async fn get_mod_graphs(&self, params: Parameters<GetModGraphsParam>) -> String {
        let mut ids = match params.0.graph_ids {
            Some(ids) => ids,
            None => match self.bridge.list_mod_graphs() {
                Ok(graphs) => graphs.into_iter().map(|graph| graph.id).collect(),
                Err(e) => return format!("Error: {e}"),
            },
        };
        ids.sort_unstable();
        ids.dedup();
        let results: Vec<_> = ids
            .into_iter()
            .map(|graph_id| match self.bridge.get_mod_graph(graph_id) {
                Ok(detail) => serde_json::json!({"graph_id": graph_id, "detail": detail}),
                Err(e) => serde_json::json!({"graph_id": graph_id, "error": e.to_string()}),
            })
            .collect();
        to_json(&results)
    }

    #[tool(
        description = "Create an empty pooled Mod Grid graph (a control-rate modulator graph whose outputs write into the automation target space). scope is 'global' (one always-on instance, default) or 'track'. Returns the new graph id."
    )]
    async fn create_mod_graph(&self, params: Parameters<CreateModGraphParam>) -> String {
        let p = params.0;
        match self.bridge.create_mod_graph(p.name, p.description, p.scope) {
            Ok(id) => to_json(&serde_json::json!({ "graph_id": id })),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Partially update name, description, and color for one or more Mod Grid graphs."
    )]
    async fn set_mod_graph_metadata(&self, params: Parameters<SetModGraphMetadataParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for item in params.0.items {
            match self.bridge.set_mod_graph_metadata(
                item.graph_id,
                item.name,
                item.description,
                item.color,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("graph {}: {e}", item.graph_id)),
            }
        }
        batch_msg(ok_count, "mod graph metadata updated", &[], &errors)
    }

    #[tool(
        description = "Delete one or more pooled Mod Grid graphs (removing their running instances)."
    )]
    async fn delete_mod_graph(&self, params: Parameters<DeleteModGraphParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for graph_id in params.0.graph_ids {
            match self.bridge.delete_mod_graph(graph_id) {
                Ok(()) => oks.push(format!("graph {graph_id}")),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "mod graphs deleted", &oks, &errors)
    }

    #[tool(
        description = "Set a Mod Grid graph's scope: 'global' (one always-on instance) or 'track' (one instance per assigned track). Switching to 'global' clears any track assignments."
    )]
    async fn set_mod_graph_scope(&self, params: Parameters<SetModGraphScopeParam>) -> String {
        let p = params.0;
        match self.bridge.set_mod_graph_scope(p.graph_id, p.scope) {
            Ok(()) => format!("Graph {} scope updated", p.graph_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Assign a track-scope Mod Grid graph to a set of tracks (one running instance per track; relative 'this track' targets resolve to each host). Replaces the current assignment; unknown track ids are dropped."
    )]
    async fn assign_mod_graph(&self, params: Parameters<AssignModGraphParam>) -> String {
        let p = params.0;
        match self.bridge.assign_mod_graph(p.graph_id, p.tracks) {
            Ok(()) => format!("Graph {} assignments updated", p.graph_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add one or more nodes to Mod Grid graphs. Each node is externally-tagged ModNodeConfig JSON: a hosted Module (lfo/mseg/envelope_follower/etc.), a Macro/Transport/MidiCc/AudioTap source, or a Target routing sink. Connect a source's output to a Target's 'in' port with connect_mod_graph. Returns each new node id."
    )]
    async fn add_mod_graph_node(&self, params: Parameters<AddModGraphNodeParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for it in params.0.items {
            let graph_id = it.graph_id;
            let node = match serde_json::to_value(it.node) {
                Ok(node) => node,
                Err(e) => {
                    errors.push(format!("{graph_id}: {e}"));
                    continue;
                }
            };
            match self
                .bridge
                .add_mod_graph_node(graph_id, node, it.description)
            {
                Ok(node_id) => oks.push(format!("graph {graph_id} @ node {node_id}")),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "mod graph nodes added", &oks, &errors)
    }

    #[tool(description = "Remove a Mod Grid node and every cable touching it.")]
    async fn remove_mod_graph_node(&self, params: Parameters<RemoveModGraphNodeParam>) -> String {
        let p = params.0;
        match self.bridge.remove_mod_graph_node(p.graph_id, p.node_id) {
            Ok(()) => format!("Node {} removed from graph {}", p.node_id, p.graph_id),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Connect one or more Mod Grid cables between named ports (e.g. a source module's 'out' to a Target node's 'in'). Validated: endpoints exist, a target isn't used as a source, single source per input port, no cycle."
    )]
    async fn connect_mod_graph(&self, params: Parameters<ConnectModGraphParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for it in params.0.items {
            let graph_id = it.graph_id;
            match self
                .bridge
                .connect_mod_graph(graph_id, it.from, it.from_port, it.to, it.to_port)
            {
                Ok(()) => oks.push(format!("graph {graph_id}: {} → {}", it.from, it.to)),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "mod graph cables added", &oks, &errors)
    }

    #[tool(
        description = "Remove one or more Mod Grid cables by their exact endpoints (the inverse of connect_mod_graph), leaving both nodes and every other cable intact. Use this to rewire a source without remove/re-add (which would drop the node's other cables and change its id)."
    )]
    async fn disconnect_mod_graph(&self, params: Parameters<DisconnectModGraphParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for it in params.0.items {
            let graph_id = it.graph_id;
            match self.bridge.disconnect_mod_graph(
                graph_id,
                it.from,
                it.from_port,
                it.to,
                it.to_port,
            ) {
                Ok(()) => oks.push(format!("graph {graph_id}: {} ↛ {}", it.from, it.to)),
                Err(e) => errors.push(format!("{graph_id}: {e}")),
            }
        }
        batch_msg(oks.len(), "mod graph cables removed", &oks, &errors)
    }

    #[tool(
        description = "Edit a Mod Grid node's config in place, keeping its id and every cable touching it (unlike remove + re-add, which changes the id and drops its cables). Use to change a Target's address/amount, a Macro's value, or a hosted module's params. The graph is re-validated. `node` is externally-tagged ModNodeConfig JSON."
    )]
    async fn set_mod_graph_node(&self, params: Parameters<SetModGraphNodeParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for item in params.0.items {
            let node = match serde_json::to_value(item.node) {
                Ok(node) => node,
                Err(e) => {
                    errors.push(format!(
                        "graph {} node {}: {e}",
                        item.graph_id, item.node_id
                    ));
                    continue;
                }
            };
            match self.bridge.set_mod_graph_node(
                item.graph_id,
                item.node_id,
                node,
                item.description,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "graph {} node {}: {e}",
                    item.graph_id, item.node_id
                )),
            }
        }
        batch_msg(ok_count, "mod graph nodes updated", &[], &errors)
    }

    #[tool(
        description = "List Mod Grid routing sinks — 'what writes to a target' — across all graphs, or just one when graph_id is set. The provenance answer to 'why is this parameter moving?'."
    )]
    async fn list_mod_targets(&self, params: Parameters<ListModTargetsParam>) -> String {
        match self.bridge.list_mod_targets(params.0.graph_id) {
            Ok(targets) => to_json(&targets),
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

    // === Sequencer: Arrangement ===

    #[tool(
        description = "Remove one or more pattern placements from the arrangement. Each placement is identified by its pattern_id, track_id, and start_beat."
    )]
    async fn remove_placement(&self, params: Parameters<RemovePlacementsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for pl in &params.0.placements {
            match self
                .bridge
                .remove_placement(pl.pattern_id, pl.track_id, pl.start_beat)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "pattern {} on track {} at {}: {e}",
                    pl.pattern_id, pl.track_id, pl.start_beat
                )),
            }
        }
        batch_msg(ok_count, "placements removed", &[], &errors)
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
        description = "Add one or more notes to a pattern in one call. Each note: pitch (MIDI 0-127, 60=C4), start_beat/duration_beats in beats, velocity (0-127)."
    )]
    async fn add_note(&self, params: Parameters<AddNotesParam>) -> String {
        for n in &params.0.notes {
            if let Err(e) = validate_note_input(n) {
                return validation_err(e);
            }
        }
        let notes: Vec<_> = params.0.notes.iter().map(note_input_to_bridge).collect();
        match self.bridge.add_notes(params.0.pattern_id, &notes) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Update one or more notes in a pattern in one call. Only provided fields are changed per note; null fields keep their current value."
    )]
    async fn update_note(&self, params: Parameters<UpdateNotesParam>) -> String {
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
        let notes: Vec<_> = params.0.notes.iter().map(note_input_to_bridge).collect();
        match self.bridge.replace_notes(params.0.pattern_id, &notes) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Clear all notes from one or more patterns. Returns the total number of notes removed."
    )]
    async fn clear_pattern(&self, params: Parameters<ClearPatternParam>) -> String {
        let mut ok_count = 0usize;
        let mut total = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.pattern_ids {
            match self.bridge.clear_pattern(*id) {
                Ok(count) => {
                    ok_count += 1;
                    total += count;
                }
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(
            ok_count,
            &format!("patterns cleared ({total} notes removed)"),
            &[],
            &errors,
        )
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
                param: pt.effective_target(),
                instrument_id: pt.instrument_id.unwrap_or_default(),
                beat: pt.beat,
                value: pt.value,
                curve: pt.curve.unwrap_or_default(),
                curve_strength: pt.curve_strength,
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

    #[tool(
        description = "List the valid automation targets for an instrument: every automatable per-module parameter in its graph (with ready-to-use 'module:<type>:<instance>:<param>' target strings, ranges, and units) plus the instrument-level macros. Use this to discover correct targets before adding automation points."
    )]
    async fn get_instrument_automation_targets(
        &self,
        params: Parameters<InstrumentIdParam>,
    ) -> String {
        match self
            .bridge
            .get_instrument_automation_targets(params.0.instrument_id)
        {
            Ok(targets) => to_json(&targets),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Get all automation points for a specific parameter lane in a pattern.")]
    async fn get_automation_points(&self, params: Parameters<GetAutomationPointsParam>) -> String {
        let p = params.0;
        match self.bridge.get_automation_points(
            p.pattern_id,
            &p.target,
            p.instrument_id.unwrap_or_default(),
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
            p.instrument_id.unwrap_or_default(),
            &p.beats,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Clear all automation points from one or more lanes (each a pattern + target + optional instrument index)."
    )]
    async fn clear_automation_lane(&self, params: Parameters<ClearAutomationLaneParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.clear_automation_lane(
                it.pattern_id,
                &it.target,
                it.instrument_id.unwrap_or_default(),
            ) {
                Ok(_count) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.pattern_id, it.target)),
            }
        }
        batch_msg(ok_count, "automation lanes cleared", &[], &errors)
    }

    #[tool(
        description = "Scale one or more automation lanes' values around a pivot, in place (tick + curve preserved). \
                       Makes a filter sweep (or any lane) more or less dramatic without re-entering points. \
                       value' = clamp((value - pivot) * scale + pivot, 0..1)."
    )]
    async fn scale_automation_lane(&self, params: Parameters<ScaleAutomationLaneParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.transform_automation_lane(
                it.pattern_id,
                &it.target,
                it.instrument_id.unwrap_or_default(),
                it.scale,
                it.pivot.unwrap_or(0.5),
                0.0,
            ) {
                Ok(_count) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.pattern_id, it.target)),
            }
        }
        batch_msg(ok_count, "automation lanes scaled", &[], &errors)
    }

    #[tool(
        description = "Shift one or more automation lanes' values by a constant, in place (tick + curve preserved). \
                       value' = clamp(value + offset, 0..1)."
    )]
    async fn offset_automation_lane(
        &self,
        params: Parameters<OffsetAutomationLaneParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.transform_automation_lane(
                it.pattern_id,
                &it.target,
                it.instrument_id.unwrap_or_default(),
                1.0,
                0.0,
                it.offset,
            ) {
                Ok(_count) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.pattern_id, it.target)),
            }
        }
        batch_msg(ok_count, "automation lanes offset", &[], &errors)
    }

    #[tool(
        description = "Copy one or more automation lanes' points to another pattern/target (tick + curve preserved), \
                       optionally scaled/offset. Useful for reusing filter motion between similar voices. \
                       By default points are merged into the destination; set clear_destination to replace."
    )]
    async fn copy_automation_lane(&self, params: Parameters<CopyAutomationLaneParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.copy_automation_lane(
                it.from_pattern_id,
                &it.from_target,
                it.from_instrument_id.unwrap_or_default(),
                it.to_pattern_id,
                &it.to_target,
                it.to_instrument_id.unwrap_or_default(),
                it.scale.unwrap_or(1.0),
                it.offset.unwrap_or(0.0),
                it.clear_destination.unwrap_or(false),
            ) {
                Ok(_count) => ok_count += 1,
                Err(e) => errors.push(format!("{} → {}: {e}", it.from_target, it.to_target)),
            }
        }
        batch_msg(ok_count, "automation lanes copied", &[], &errors)
    }

    #[tool(
        description = "Project-wide automation overview: every lane in every pattern, grouped by 'instrument' \
                       (default), 'target', or 'pattern'. Read-only — use to audit where automation lives without \
                       querying each pattern."
    )]
    async fn get_automation_summary(
        &self,
        params: Parameters<GetAutomationSummaryParam>,
    ) -> String {
        let group_by = params
            .0
            .group_by
            .unwrap_or_else(|| "instrument".to_string());
        if !matches!(group_by.as_str(), "instrument" | "target" | "pattern") {
            return "Error: group_by must be 'instrument', 'target', or 'pattern'".to_string();
        }
        match self.bridge.get_automation_summary(&group_by) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Track control ===

    #[tool(
        description = "Set mixer state on one or more tracks in a single call. Each item carries \
        a track_id plus any of volume (0.0=silent, 1.0=full, up to 2.0 for boost), pan \
        (-1.0=left..1.0=right), muted, and solo. Omitted fields are left unchanged. When any track \
        is soloed, only soloed tracks sound. To (un)assign a track's instrument, use set_track_instrument."
    )]
    async fn set_track_mixer(&self, params: Parameters<SetTrackMixerParam>) -> String {
        for it in &params.0.items {
            if let Some(v) = it.volume
                && let Err(e) = validate_range("volume", v, 0.0, 2.0)
            {
                return validation_err(e);
            }
            if let Some(p) = it.pan
                && let Err(e) = validate_range("pan", p, -1.0, 1.0)
            {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            let id = it.track_id;
            let mut item_err: Option<String> = None;
            if let Some(v) = it.volume
                && let Err(e) = self.bridge.set_track_volume(id, v)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(p) = it.pan
                && let Err(e) = self.bridge.set_track_pan(id, p)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(m) = it.muted
                && let Err(e) = self.bridge.set_track_mute(id, m)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = it.solo
                && let Err(e) = self.bridge.set_track_solo(id, s)
            {
                item_err = Some(e.to_string());
            }
            match item_err {
                None => ok_count += 1,
                Some(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "track mixer updates applied", &[], &errors)
    }

    #[tool(
        description = "Assign (or unassign) the instrument driving one or more tracks. Each item's \
        instrument_id is required: a number assigns that instrument, null unassigns (the track plays nothing)."
    )]
    async fn set_track_instrument(&self, params: Parameters<SetTrackInstrumentParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_track_instrument(it.track_id, it.instrument_id)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.track_id)),
            }
        }
        batch_msg(ok_count, "track instruments set", &[], &errors)
    }

    #[tool(
        description = "Rename one or more tracks. The name is shown in the sequencer track headers."
    )]
    async fn rename_track(&self, params: Parameters<RenameTrackParam>) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_name("track", &it.name) {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_track(it.track_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.track_id)),
            }
        }
        batch_msg(ok_count, "tracks renamed", &[], &errors)
    }

    #[tool(
        description = "Delete one or more tracks and all their placements from the arrangement."
    )]
    async fn delete_track(&self, params: Parameters<DeleteTracksParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.track_ids {
            match self.bridge.delete_track(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "tracks deleted", &[], &errors)
    }

    // === Return busses (effect sends) ===

    #[tool(
        description = "List all return busses (effect-send destinations) with their fader settings."
    )]
    async fn list_return_busses(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_return_busses() {
            Ok(busses) => to_json(&busses),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create one or more return busses (each a sub-mix with its own effect chain, fed by track sends). Returns the assigned IDs."
    )]
    async fn create_return_bus(&self, params: Parameters<CreateReturnBusParam>) -> String {
        for name in &params.0.names {
            if let Err(e) = validate_name("return bus", name) {
                return validation_err(e);
            }
        }
        let mut ids = Vec::new();
        let mut errors = Vec::new();
        for name in &params.0.names {
            match self.bridge.create_return_bus(name) {
                Ok(id) => ids.push(format!("{id} '{name}'")),
                Err(e) => errors.push(format!("'{name}': {e}")),
            }
        }
        batch_msg(ids.len(), "return busses created", &ids, &errors)
    }

    #[tool(
        description = "Delete one or more return busses and remove every track send that targeted them."
    )]
    async fn delete_return_bus(&self, params: Parameters<DeleteReturnBusesParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.return_ids {
            match self.bridge.delete_return_bus(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "return busses deleted", &[], &errors)
    }

    #[tool(
        description = "Set mixer state on one or more return busses in a single call. Each item \
        carries a return_id plus any of volume (0.0=silent..1.0=full), pan (-1.0=left..1.0=right), \
        muted, and solo. Omitted fields are left unchanged. When any return is soloed, only soloed \
        returns reach the master mix (bus-to-bus routing still flows)."
    )]
    async fn set_return_bus_mixer(&self, params: Parameters<SetReturnBusMixerParam>) -> String {
        for it in &params.0.items {
            // Return-bus volume is stored as NormalizedValue (clamps to [0, 1]).
            if let Some(v) = it.volume
                && let Err(e) = validate_range("volume", v, 0.0, 1.0)
            {
                return validation_err(e);
            }
            if let Some(p) = it.pan
                && let Err(e) = validate_range("pan", p, -1.0, 1.0)
            {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            let id = it.return_id;
            let mut item_err: Option<String> = None;
            if let Some(v) = it.volume
                && let Err(e) = self.bridge.set_return_bus_volume(id, v)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(p) = it.pan
                && let Err(e) = self.bridge.set_return_bus_pan(id, p)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(m) = it.muted
                && let Err(e) = self.bridge.set_return_bus_mute(id, m)
            {
                item_err = Some(e.to_string());
            }
            if item_err.is_none()
                && let Some(s) = it.solo
                && let Err(e) = self.bridge.set_return_bus_solo(id, s)
            {
                item_err = Some(e.to_string());
            }
            match item_err {
                None => ok_count += 1,
                Some(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "return bus mixer updates applied", &[], &errors)
    }

    #[tool(
        description = "Set the display color of one or more return busses from a \"#RRGGBB\" hex string."
    )]
    async fn set_return_bus_color(&self, params: Parameters<SetReturnBusColorParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_return_bus_color(it.return_id, &it.color) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.return_id)),
            }
        }
        batch_msg(ok_count, "return bus colors set", &[], &errors)
    }

    #[tool(
        description = "Set the free-text description / intent (\"\" clears it) on one or more return busses. Never affects audio."
    )]
    async fn set_return_bus_description(
        &self,
        params: Parameters<SetReturnBusDescriptionParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_return_bus_description(it.return_id, &it.description)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.return_id)),
            }
        }
        batch_msg(ok_count, "return bus descriptions set", &[], &errors)
    }

    #[tool(description = "Rename one or more return busses.")]
    async fn rename_return_bus(&self, params: Parameters<RenameReturnBusParam>) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_name("return bus", &it.name) {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_return_bus(it.return_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.return_id)),
            }
        }
        batch_msg(ok_count, "return busses renamed", &[], &errors)
    }

    #[tool(
        description = "Add or update one or more track effect sends to return busses (upsert by track+return target). pre_fader taps before the channel fader."
    )]
    async fn set_track_send(&self, params: Parameters<SetTrackSendParam>) -> String {
        // Stored as NormalizedValue (clamps to [0, 1]); validate to match.
        for s in &params.0.sends {
            if let Err(e) = validate_range("level", s.level, 0.0, 1.0) {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for s in &params.0.sends {
            match self.bridge.set_track_send(
                s.track_id,
                s.return_id,
                s.level,
                s.pre_fader,
                s.enabled,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "track {} → return {}: {e}",
                    s.track_id, s.return_id
                )),
            }
        }
        batch_msg(ok_count, "track sends set", &[], &errors)
    }

    #[tool(description = "Remove one or more track effect sends to return busses.")]
    async fn remove_track_send(&self, params: Parameters<RemoveTrackSendsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for s in &params.0.sends {
            match self.bridge.remove_track_send(s.track_id, s.return_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!(
                    "track {} → return {}: {e}",
                    s.track_id, s.return_id
                )),
            }
        }
        batch_msg(ok_count, "track sends removed", &[], &errors)
    }

    #[tool(
        description = "Add or update one or more bus-to-bus sends: route one return bus's output into another (e.g. a delay return into a reverb return). Upsert by from+to target; each is rejected if it would create a routing cycle."
    )]
    async fn set_return_send(&self, params: Parameters<SetReturnSendParam>) -> String {
        for s in &params.0.sends {
            if let Err(e) = validate_range("level", s.level, 0.0, 1.0) {
                return validation_err(e);
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for s in &params.0.sends {
            match self
                .bridge
                .set_return_send(s.from_id, s.to_id, s.level, s.enabled)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("return {} → return {}: {e}", s.from_id, s.to_id)),
            }
        }
        batch_msg(ok_count, "return sends set", &[], &errors)
    }

    #[tool(description = "Remove one or more bus-to-bus sends from one return bus into another.")]
    async fn remove_return_send(&self, params: Parameters<RemoveReturnSendsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for s in &params.0.sends {
            match self.bridge.remove_return_send(s.from_id, s.to_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("return {} → return {}: {e}", s.from_id, s.to_id)),
            }
        }
        batch_msg(ok_count, "return sends removed", &[], &errors)
    }

    // === Return-bus insert effects ===

    #[tool(
        description = "Add one or more insert effects to a return bus's effect chain, in order (e.g. put a reverb on a Reverb return). Each effect_type is a module-type key like 'rev', 'delay', 'chorus', 'eq', 'compressor'. Returns the new effects' module-ids (e.g. 'rev-1')."
    )]
    async fn add_return_effect(&self, params: Parameters<AddReturnEffectsParam>) -> String {
        let p = params.0;
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for effect_type in &p.effect_types {
            match self.bridge.add_return_effect(p.return_id, effect_type) {
                Ok(module_id) => oks.push(module_id),
                Err(e) => errors.push(format!("{effect_type}: {e}")),
            }
        }
        batch_msg(
            oks.len(),
            &format!("effects added to return bus {}", p.return_id),
            &oks,
            &errors,
        )
    }

    #[tool(
        description = "Remove one or more insert effects from a return bus's effect chain by their module-ids."
    )]
    async fn remove_return_effect(&self, params: Parameters<RemoveReturnEffectsParam>) -> String {
        let p = params.0;
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for module_id in &p.module_ids {
            match self.bridge.remove_return_effect(p.return_id, module_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{module_id}: {e}")),
            }
        }
        batch_msg(
            ok_count,
            &format!("effects removed from return bus {}", p.return_id),
            &[],
            &errors,
        )
    }

    #[tool(
        description = "Set parameters on return-bus insert effects (one or many). Each item gives return_id, module_id, param_name (type_id or display name) and value (number, boolean, or choice string). Use list_return_busses to discover effects and their parameters."
    )]
    async fn set_return_effect_parameter(
        &self,
        params: Parameters<SetReturnEffectParameterParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.params {
            let value = match param_value_to_bridge(it.value) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{}/{}: {e}", it.return_id, it.module_id));
                    continue;
                }
            };
            match self.bridge.set_return_effect_parameter(
                it.return_id,
                &it.module_id,
                &it.param_name,
                value,
            ) {
                Ok(_info) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        batch_msg(ok_count, "return effect parameters set", &[], &errors)
    }

    #[tool(
        description = "Enable or bypass one or more return-bus insert effects (enabled = false bypasses)."
    )]
    async fn set_return_effect_enabled(
        &self,
        params: Parameters<SetReturnEffectEnabledParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_return_effect_enabled(it.return_id, &it.module_id, it.enabled)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        batch_msg(ok_count, "return effect toggles applied", &[], &errors)
    }

    #[tool(
        description = "Move one or more return-bus insert effects up or down within their effect chain (direction: 'up' = earlier, 'down' = later). Moves are applied in array order."
    )]
    async fn reorder_return_effect(&self, params: Parameters<ReorderReturnEffectParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            let direction = match it.direction.trim().to_ascii_lowercase().as_str() {
                "up" => crate::bridge::ReturnEffectMove::Up,
                "down" => crate::bridge::ReturnEffectMove::Down,
                other => {
                    errors.push(format!(
                        "{}/{}: invalid direction '{other}', expected 'up' or 'down'",
                        it.return_id, it.module_id
                    ));
                    continue;
                }
            };
            match self
                .bridge
                .reorder_return_effect(it.return_id, &it.module_id, direction)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.return_id, it.module_id)),
            }
        }
        batch_msg(ok_count, "return effects reordered", &[], &errors)
    }

    // === Master bus ===

    #[tool(description = "Read the master output volume (0.0 = silent, 1.0 = unity).")]
    async fn get_master_volume(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_master_volume() {
            Ok(v) => format!("{v}"),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Set the master output volume (0.0 = silent, 1.0 = unity).")]
    async fn set_master_volume(&self, params: Parameters<SetMasterVolumeParam>) -> String {
        if let Err(e) = validate_range("volume", params.0.volume, 0.0, 4.0) {
            return validation_err(e);
        }
        match self.bridge.set_master_volume(params.0.volume) {
            Ok(()) => format!("OK: master volume set to {}", params.0.volume),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List the master-bus insert effects (the final effect chain applied to the full mix) in processing order, with parameters."
    )]
    async fn list_master_effects(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_master_effects() {
            Ok(effects) => to_json(&effects),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Add one or more insert effects to the master-bus effect chain, in order (applied to the full mix, e.g. a limiter or EQ on the master). Each effect_type is a module-type key. Returns the new module-ids."
    )]
    async fn add_master_effect(&self, params: Parameters<AddMasterEffectsParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for effect_type in &params.0.effect_types {
            match self.bridge.add_master_effect(effect_type) {
                Ok(module_id) => oks.push(module_id),
                Err(e) => errors.push(format!("{effect_type}: {e}")),
            }
        }
        batch_msg(oks.len(), "effects added to master bus", &oks, &errors)
    }

    #[tool(
        description = "Remove one or more insert effects from the master-bus effect chain by their module-ids."
    )]
    async fn remove_master_effect(&self, params: Parameters<RemoveMasterEffectsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for module_id in &params.0.module_ids {
            match self.bridge.remove_master_effect(module_id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{module_id}: {e}")),
            }
        }
        batch_msg(ok_count, "effects removed from master bus", &[], &errors)
    }

    #[tool(
        description = "Set a parameter on a master-bus insert effect. param_name is the parameter's type_id or display name; value is a number, boolean, or choice string. Use list_master_effects to discover effects and parameters."
    )]
    async fn set_master_effect_parameter(
        &self,
        params: Parameters<SetMasterEffectParameterParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in params.0.params {
            let value = match param_value_to_bridge(it.value) {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{}: {e}", it.module_id));
                    continue;
                }
            };
            match self
                .bridge
                .set_master_effect_parameter(&it.module_id, &it.param_name, value)
            {
                Ok(_info) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.module_id)),
            }
        }
        batch_msg(ok_count, "master effect parameters set", &[], &errors)
    }

    #[tool(
        description = "Enable or bypass one or more master-bus insert effects (enabled = false bypasses)."
    )]
    async fn set_master_effect_enabled(
        &self,
        params: Parameters<SetMasterEffectEnabledParam>,
    ) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_master_effect_enabled(&it.module_id, it.enabled)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.module_id)),
            }
        }
        batch_msg(ok_count, "master effect toggles applied", &[], &errors)
    }

    #[tool(
        description = "Move one or more master-bus insert effects up or down within the chain (direction: 'up' = earlier, 'down' = later). Moves are applied in array order."
    )]
    async fn reorder_master_effect(&self, params: Parameters<ReorderMasterEffectParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            let direction = match it.direction.trim().to_ascii_lowercase().as_str() {
                "up" => crate::bridge::ReturnEffectMove::Up,
                "down" => crate::bridge::ReturnEffectMove::Down,
                other => {
                    errors.push(format!(
                        "{}: invalid direction '{other}', expected 'up' or 'down'",
                        it.module_id
                    ));
                    continue;
                }
            };
            match self.bridge.reorder_master_effect(&it.module_id, direction) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.module_id)),
            }
        }
        batch_msg(ok_count, "master effects reordered", &[], &errors)
    }

    // === Pattern management ===

    #[tool(
        description = "Rename one or more patterns. The name is shown in the arrangement timeline and piano roll."
    )]
    async fn rename_pattern(&self, params: Parameters<RenamePatternParam>) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_name("pattern", &it.name) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_pattern(it.pattern_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.pattern_id)),
            }
        }
        batch_msg(ok_count, "patterns renamed", &[], &errors)
    }

    #[tool(
        description = "Set the length in beats of one or more patterns (e.g. 4.0 = one bar in 4/4, 8.0 = two bars)."
    )]
    async fn set_pattern_length(&self, params: Parameters<SetPatternLengthParam>) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_range("length_beats", it.length_beats, 0.001, 1024.0) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_pattern_length(it.pattern_id, it.length_beats)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.pattern_id)),
            }
        }
        batch_msg(ok_count, "pattern lengths set", &[], &errors)
    }

    #[tool(
        description = "Duplicate one or more patterns including all notes and automation. Returns the new pattern IDs."
    )]
    async fn duplicate_pattern(&self, params: Parameters<DuplicatePatternParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for id in &params.0.pattern_ids {
            match self.bridge.duplicate_pattern(*id) {
                Ok(new_id) => oks.push(format!("{id} → {new_id}")),
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(oks.len(), "patterns duplicated", &oks, &errors)
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
        description = "Set one or more module parameters in one call. Each entry is {module_id, param_name, value}; value is a number in the parameter's native range, a boolean, or a string for a choice/enum or an address (e.g. a Mod Matrix slot_N_dest of 'spp-1.x')."
    )]
    async fn set_parameter(&self, params: Parameters<SetParametersParam>) -> String {
        let p = params.0;
        for ps in &p.params {
            if let ParamValueInput::Number(n) = &ps.value
                && n.is_nan()
            {
                return format!(
                    "Error: NaN is not a valid value for parameter '{}' on module '{}'",
                    ps.param_name, ps.module_id
                );
            }
        }
        let param_sets: Vec<_> = p
            .params
            .into_iter()
            .map(|ps| crate::bridge::BridgeParamSet {
                module_id: ps.module_id,
                param_name: ps.param_name,
                value: match ps.value {
                    ParamValueInput::Number(n) => crate::bridge::BridgeParamValue::Number(n),
                    ParamValueInput::Bool(b) => crate::bridge::BridgeParamValue::Bool(b),
                    ParamValueInput::Choice(s) => crate::bridge::BridgeParamValue::Choice(s),
                },
            })
            .collect();
        match self.bridge.set_parameters(p.instrument_id, &param_sets) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create one or more patterns in one call, optionally with inline notes and automation. Returns per-pattern results with assigned IDs."
    )]
    async fn create_pattern(&self, params: Parameters<CreatePatternsParam>) -> String {
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
                    .map(note_input_to_bridge)
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
        description = "Create one or more tracks in one call. Optionally assign an instrument per track. Returns per-track results with assigned IDs."
    )]
    async fn create_track(&self, params: Parameters<CreateTracksParam>) -> String {
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
        description = "Place one or more patterns on tracks in the arrangement in one call. Each placement specifies pattern_id, track_id, and start_beat."
    )]
    async fn place_pattern(&self, params: Parameters<PlacePatternsParam>) -> String {
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
                notes: pat.notes.iter().map(note_input_to_bridge).collect(),
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
        if let Err(e) = validate_range("tempo", tempo, 20.0, 999.0) {
            return format!("Error: {e}");
        }
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
        if let Err(e) = validate_range("beat", params.0.beat, 0.0, 9999.0) {
            return format!("Error: {e}");
        }
        match self.bridge.seq_seek(params.0.beat) {
            Ok(()) => format!("OK: seeked to beat {}", params.0.beat),
            Err(e) => format!("Error: {e}"),
        }
    }

    // === Batch instrument building ===

    #[tool(
        description = "Build one or more complete instruments in one call. Each instrument has its own modules and connections; \
                       modules are referenced by 0-based array index in connections. Returns per-instrument results with instrument_id and module_ids. \
                       Port names must match the module's ports (osc/amp/out expose 'out'/'in'); the aliases 'output'→'out' and 'input'→'in' are also accepted. If every requested connection fails the whole call errors instead of returning a zero-connection instrument (a freshly-created instrument is rolled back, so no orphan is left). \
                       Example instrument: modules=[{module_type:'osc'},{module_type:'amp'},{module_type:'out'}], connections=[{from:0,from_port:'out',to:1,to_port:'in'},{from:1,from_port:'out',to:2,to_port:'in'}]"
    )]
    async fn build_instrument(&self, params: Parameters<BuildInstrumentsParam>) -> String {
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
        description = "Rebuild an existing instrument's voice graph (new modules/params/connections) while keeping its pattern automation working. Instance counters are reset before the rebuild so modules are numbered deterministically (1.. per type, in add order) — wherever the new module set matches the old, the module ids line up and their automation lanes stay valid automatically. Lanes whose target module no longer exists are reported as `orphaned_lanes`; set `drop_orphaned: true` to delete them, otherwise they are left dangling. Returns the rebuilt module ids, preserved-lane count, and the orphaned lanes. Use this instead of build_instrument when the instrument already has automation you don't want to lose. Note: matching is by module type + add-order, so reordering same-type modules can still re-point a lane."
    )]
    async fn rebuild_instrument_preserve_automation(
        &self,
        params: Parameters<RebuildInstrumentParam>,
    ) -> String {
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
        let drop_orphaned = p.drop_orphaned.unwrap_or(false);
        let spec = convert_instrument_def(
            Some(p.instrument_id),
            p.name,
            p.midi_channel,
            p.volume,
            p.pan,
            p.modules,
            p.connections,
        );
        match self
            .bridge
            .rebuild_instrument_preserve_automation(&spec, drop_orphaned)
        {
            Ok(result) => to_json(&result),
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
        if let Err(e) = validate_file_path(&params.0.path) {
            return e;
        }
        tokio::task::block_in_place(|| match self.bridge.save_project(&params.0.path) {
            Ok(msg) => format!("OK: {msg}"),
            Err(e) => format!("Error: {e}"),
        })
    }

    #[tool(
        description = "Save a single instrument as a standalone patch file (its modules, \
        connections, and patch metadata only — no song or other instruments). This is the \
        single-instrument format that load_project reads back, distinct from save_project which \
        writes the whole project. It waits (bounded) for graph mutations queued earlier in the \
        SAME batch_execute (add_module/connect) to be applied before reading the graph, so an \
        in-batch build-then-save captures the freshly-added modules/connections."
    )]
    async fn save_patch(&self, params: Parameters<SavePatchParam>) -> String {
        if let Err(e) = validate_file_path(&params.0.path) {
            return e;
        }
        tokio::task::block_in_place(|| {
            match self
                .bridge
                .save_patch(params.0.instrument_id, &params.0.path)
            {
                Ok(msg) => format!("OK: {msg}"),
                Err(e) => format!("Error: {e}"),
            }
        })
    }

    #[tool(
        description = "Load a project or patch file, replacing all current state. Supports both project files and single patch files."
    )]
    async fn load_project(&self, params: Parameters<ProjectPathParam>) -> String {
        if let Err(e) = validate_file_path(&params.0.path) {
            return e;
        }
        tokio::task::block_in_place(|| match self.bridge.load_project(&params.0.path) {
            Ok(msg) => format!("OK: {msg}"),
            Err(e) => format!("Error: {e}"),
        })
    }

    #[tool(
        description = "Optimize the project by removing unused patterns (not placed in arrangement), \
                       unused tracks (no placements), unused instruments (not referenced by any track or note), \
                       and unused samples (no `Sampler` module's `sample_select` references them). Pruning samples \
                       keeps the sample library empty when nothing uses it, which lets the next save stay on plain \
                       JSON instead of being forced into bundle format. Returns a summary of what was removed."
    )]
    async fn optimize_project(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.optimize_project() {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ========================================================================
    // SAMPLE LIBRARY TOOLS
    // ========================================================================

    #[tool(
        description = "List all samples in the sample library. Returns id, name, duration, channels, \
                       sample rate, root note, and source type for each sample. Use optional \
                       name_filter to search by name substring."
    )]
    async fn list_samples(&self, params: Parameters<ListSamplesParam>) -> String {
        match self.bridge.list_samples(params.0.name_filter.as_deref()) {
            Ok(samples) => to_json(&samples),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Import one or more WAV files into the sample library. Returns the array of new \
                       sample infos with assigned IDs. Each entry may override the name and set the root \
                       MIDI note (0-127, default 60=C4)."
    )]
    async fn import_sample(&self, params: Parameters<ImportSampleParam>) -> String {
        for s in &params.0.samples {
            if let Some(note) = s.root_note
                && let Err(e) = validate_midi_note(note)
            {
                return format!("Error: {e}");
            }
        }
        let mut infos = Vec::new();
        let mut errors = Vec::new();
        for s in &params.0.samples {
            match self
                .bridge
                .import_sample(&s.path, s.name.as_deref(), s.root_note)
            {
                Ok(info) => infos.push(info),
                Err(e) => errors.push(format!("'{}': {e}", s.path)),
            }
        }
        batch_json("imported", &infos, &errors)
    }

    #[tool(
        description = "Delete one or more samples from the library by ID. Use list_samples to find sample IDs."
    )]
    async fn delete_sample(&self, params: Parameters<DeleteSamplesParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.delete_sample(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "samples deleted", &[], &errors)
    }

    #[tool(description = "Rename one or more samples in the library.")]
    async fn rename_sample(&self, params: Parameters<RenameSampleParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.rename_sample(it.sample_id, &it.name) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "samples renamed", &[], &errors)
    }

    #[tool(
        description = "Set the root MIDI note for one or more samples (determines playback pitch mapping). \
                       Note 60 = C4 (middle C). Range: 0-127."
    )]
    async fn set_sample_root_note(&self, params: Parameters<SetSampleRootNoteParam>) -> String {
        for it in &params.0.items {
            if let Err(e) = validate_midi_note(it.note) {
                return format!("Error: {e}");
            }
        }
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_sample_root_note(it.sample_id, it.note) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "sample root notes set", &[], &errors)
    }

    #[tool(
        description = "Normalize peak level to 0 dB (maximum without clipping) for one or more samples."
    )]
    async fn normalize_sample(&self, params: Parameters<SampleIdsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.normalize_sample(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "samples normalized", &[], &errors)
    }

    #[tool(description = "Reverse the audio data in place for one or more samples.")]
    async fn reverse_sample(&self, params: Parameters<SampleIdsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.reverse_sample(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "samples reversed", &[], &errors)
    }

    #[tool(
        description = "Auto-trim silence from the start and end of one or more samples. Sets crop markers \
                       at the first and last audible frames (threshold: -40 dB)."
    )]
    async fn trim_sample_silence(&self, params: Parameters<SampleIdsParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.trim_sample_silence(*id) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_msg(ok_count, "samples trimmed", &[], &errors)
    }

    #[tool(
        description = "Get detailed information about a sample including peak level, RMS, DC offset, \
                       memory usage, and loop/crop regions in seconds."
    )]
    async fn get_sample_info(&self, params: Parameters<SampleIdParam>) -> String {
        match self.bridge.get_sample_info(params.0.sample_id) {
            Ok(info) => to_json(&info),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Create a copy of one or more samples, each with a new ID. The copy gets \" (copy)\" \
                       appended to its name. Returns the array of new sample infos."
    )]
    async fn duplicate_sample(&self, params: Parameters<SampleIdsParam>) -> String {
        let mut infos = Vec::new();
        let mut errors = Vec::new();
        for id in &params.0.sample_ids {
            match self.bridge.duplicate_sample(*id) {
                Ok(info) => infos.push(info),
                Err(e) => errors.push(format!("{id}: {e}")),
            }
        }
        batch_json("duplicated", &infos, &errors)
    }

    #[tool(
        description = "Set or disable the loop region for one or more samples. When enabled, provide start \
                       and end times in seconds. Optional crossfade in milliseconds smooths the \
                       loop boundary."
    )]
    async fn set_sample_loop(&self, params: Parameters<SetSampleLoopParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self.bridge.set_sample_loop(
                it.sample_id,
                it.enabled,
                it.start_seconds,
                it.end_seconds,
                it.crossfade_ms,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "sample loops set", &[], &errors)
    }

    #[tool(
        description = "Set or remove the crop region for one or more samples. Crop defines the audible \
                       portion. Omit start_seconds and end_seconds to remove the crop and use \
                       the full sample."
    )]
    async fn set_sample_crop(&self, params: Parameters<SetSampleCropParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .set_sample_crop(it.sample_id, it.start_seconds, it.end_seconds)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.sample_id)),
            }
        }
        batch_msg(ok_count, "sample crops updated", &[], &errors)
    }

    #[tool(
        description = "Export one or more samples to WAV files at the given paths. Crop region is applied \
                       if set. Bit depth: 16 (default), 24, or 32 (float)."
    )]
    async fn export_sample(&self, params: Parameters<ExportSampleParam>) -> String {
        let mut oks = Vec::new();
        let mut errors = Vec::new();
        for s in params.0.samples {
            match self.bridge.export_sample(s.sample_id, &s.path, s.bit_depth) {
                Ok(()) => oks.push(s.path),
                Err(e) => errors.push(format!("{}: {e}", s.sample_id)),
            }
        }
        batch_msg(oks.len(), "samples exported", &oks, &errors)
    }

    // ========================================================================
    // SAMPLER MODULE TOOLS
    // ========================================================================

    #[tool(
        description = "Assign a sample to a Sampler module in an instrument. The module must be \
                       of type 'sampler' (prefix 'sam'). Use list_samples for sample IDs and \
                       get_instrument_info for module IDs."
    )]
    async fn assign_sample_to_module(&self, params: Parameters<AssignSampleParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.items {
            match self
                .bridge
                .assign_sample_to_module(it.instrument_id, &it.module_id, it.sample_id)
            {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}: {e}", it.module_id)),
            }
        }
        batch_msg(ok_count, "samples assigned to modules", &[], &errors)
    }

    #[tool(
        description = "Get the current state of a Sampler module: assigned sample, pitch tracking, \
                       level, play mode, direction, velocity sensitivity, fine tune, start offset."
    )]
    async fn get_sampler_state(&self, params: Parameters<SamplerModuleParam>) -> String {
        match self
            .bridge
            .get_sampler_state(params.0.instrument_id, &params.0.module_id)
        {
            Ok(state) => to_json(&state),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set a parameter on a Sampler module. Parameters: pitch_tracking (true/false), \
                       level (0.0-1.0), play_mode (one_shot/sustain/loop), direction \
                       (forward/reverse/ping_pong), velocity_sensitivity (0.0-1.0), \
                       fine_tune (-100 to 100 cents), start_offset (0.0-1.0)."
    )]
    async fn set_sampler_parameter(&self, params: Parameters<SetSamplerParameterParam>) -> String {
        let mut ok_count = 0usize;
        let mut errors = Vec::new();
        for it in &params.0.params {
            match self.bridge.set_sampler_parameter(
                it.instrument_id,
                &it.module_id,
                &it.param_name,
                &it.value,
            ) {
                Ok(()) => ok_count += 1,
                Err(e) => errors.push(format!("{}/{}: {e}", it.module_id, it.param_name)),
            }
        }
        batch_msg(ok_count, "sampler parameters set", &[], &errors)
    }

    // ========================================================================
    // AUDIO INPUT TOOLS
    // ========================================================================

    #[tool(description = "List available audio input devices (microphones, line-in, etc.).")]
    async fn list_input_devices(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_input_devices() {
            Ok(devices) => to_json(&devices),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get the current audio input state: monitoring status, recording status, \
                       peak level, and recording duration."
    )]
    async fn get_input_state(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_input_state() {
            Ok(state) => to_json(&state),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ========================================================================
    // DISCOVERY TOOLS
    // ========================================================================

    #[tool(
        description = "Get detailed info for a single module type by its type key (e.g. 'osc', 'flt', 'env'). \
                       Returns ports, parameters with ranges/units/choices, and signal flow hints. \
                       Lighter than list_module_types when you already know which module you need."
    )]
    async fn get_module_type_info(&self, params: Parameters<GetModuleTypeInfoParam>) -> String {
        let type_key = params.0.type_key.trim();
        if type_key.is_empty() {
            return validation_err(McpBridgeError::EmptyName { kind: "type_key" });
        }
        match self.bridge.get_module_type_info(type_key) {
            Ok(info) => to_json(&info),
            Err(e) => {
                let hint = if matches!(e, McpBridgeError::InvalidModuleType(_)) {
                    match self.bridge.list_module_types() {
                        Ok(types) => {
                            let keys: Vec<&str> =
                                types.iter().map(|t| t.type_key.as_str()).collect();
                            let similar = find_similar(type_key, &keys, 3);
                            if similar.is_empty() {
                                "\nHint: use list_module_types to see all available type keys."
                                    .to_string()
                            } else {
                                format!(
                                    "\nHint: did you mean {}? Use list_module_types to see all.",
                                    similar.join(", ")
                                )
                            }
                        }
                        Err(_) => String::new(),
                    }
                } else {
                    String::new()
                };
                format!("Error: {e}{hint}")
            }
        }
    }

    #[tool(
        description = "Search available module types by category, port signal type, or text query. \
                       All filters are optional and combined with AND logic. Returns matching modules \
                       with full port/parameter details."
    )]
    async fn search_modules(&self, params: Parameters<SearchModulesParam>) -> String {
        let p = params.0;
        // Validate category if provided
        if let Some(ref cat) = p.category
            && !["voice", "effect", "visualizer"].contains(&cat.as_str())
        {
            return format!(
                "Error: invalid category '{}'. Valid categories: voice, effect, visualizer",
                cat
            );
        }
        // Validate signal types if provided
        for (name, val) in [
            ("has_input_type", &p.has_input_type),
            ("has_output_type", &p.has_output_type),
        ] {
            if let Some(st) = val
                && !VALID_SIGNAL_TYPES.contains(&st.as_str())
            {
                return format!(
                    "Error: invalid {name} '{}'. Valid signal types: audio, control, gate, midi",
                    st
                );
            }
        }
        match self.bridge.search_modules(
            p.category.as_deref(),
            p.has_input_type.as_deref(),
            p.has_output_type.as_deref(),
            p.query.as_deref(),
        ) {
            Ok(result) => {
                if result.modules.is_empty() {
                    let mut hint = "No modules matched your filters.".to_string();
                    if !result.did_you_mean.is_empty() {
                        hint.push_str(&format!(
                            " Did you mean: {}?",
                            result.did_you_mean.join(", ")
                        ));
                    } else if p.query.is_some() {
                        hint.push_str(" Try a broader text query or remove filters.");
                    }
                    hint
                } else {
                    let count = result.modules.len();
                    let json = to_json(&result.modules);
                    format!("{json}\n\n({count} module(s) matched)")
                }
            }
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all port signal types with descriptions, value ranges, and compatibility. \
                       Use this to understand which port types can connect to each other."
    )]
    async fn list_port_types(&self, _params: Parameters<NoParams>) -> String {
        use crate::types::PortSignalTypeInfo;
        let types = vec![
            PortSignalTypeInfo {
                signal_type: "audio".to_string(),
                description: "Audio-rate signal, processed sample-by-sample at the engine sample rate.".to_string(),
                value_range: "Typically -1.0 to +1.0 (can exceed for hot signals)".to_string(),
                compatible_with: vec!["audio".to_string(), "control".to_string()],
            },
            PortSignalTypeInfo {
                signal_type: "control".to_string(),
                description: "Control-rate signal for parameter modulation (pitch CV, filter cutoff CV, etc.).".to_string(),
                value_range: "0.0 to 1.0 (unipolar) or -1.0 to +1.0 (bipolar), depends on source".to_string(),
                compatible_with: vec!["audio".to_string(), "control".to_string()],
            },
            PortSignalTypeInfo {
                signal_type: "gate".to_string(),
                description: "Binary trigger signal for note on/off. High (1.0) = note held, low (0.0) = released.".to_string(),
                value_range: "0.0 or 1.0".to_string(),
                compatible_with: vec!["gate".to_string(), "control".to_string()],
            },
            PortSignalTypeInfo {
                signal_type: "midi".to_string(),
                description: "MIDI event data (note on/off, CC, pitch bend). Only connects to MIDI ports.".to_string(),
                value_range: "Structured MIDI events".to_string(),
                compatible_with: vec!["midi".to_string()],
            },
        ];
        to_json(&types)
    }

    #[tool(
        description = "Get the full YAMS (Yet Another Modulation Script) language reference as \
                       Markdown: grammar, statements (src/let/out, arr lookup tables), the function \
                       set, context variables, macros, and the array index / OOB rules. Read this \
                       before authoring a script for `set_mod_matrix_script`, which installs YAMS on \
                       a Mod Matrix OR Script (scr) module slot. Read back installed scripts via \
                       `get_module_info` (Script modules expose a `scripts` array) or \
                       `get_mod_matrix_routings` (Mod Matrix slots expose `script`)."
    )]
    async fn get_yams_reference(&self, _params: Parameters<NoParams>) -> String {
        synth_script::REFERENCE.to_string()
    }

    #[tool(
        description = "Check whether a connection between two module ports would be valid. \
                       Returns compatibility info and hints. Use this before connect to avoid errors."
    )]
    async fn check_connection(&self, params: Parameters<CheckConnectionParam>) -> String {
        let p = params.0;
        match self.bridge.check_connection(
            p.instrument_id,
            &p.from_module,
            &p.from_port,
            &p.to_module,
            &p.to_port,
        ) {
            Ok(result) => to_json(&result),
            Err(e) => format!("Error: {e}"),
        }
    }

    // ========================================================================
    // BATCH EXECUTE
    // ========================================================================

    #[tool(
        description = "Execute multiple tool calls in a single request to reduce round-trip latency. \
                       Operations run sequentially. Max 50 operations per batch. \
                       Cannot nest batch_execute inside a batch. \
                       Set `dry_run: true` to validate every operation (tool name + params) without \
                       executing any — nothing is mutated. Set `rollback: true` to make the batch \
                       all-or-nothing: the project is snapshotted first and restored if any operation fails."
    )]
    async fn batch_execute(&self, params: Parameters<BatchExecuteParam>) -> String {
        use crate::types::{BatchExecItemResult, BatchExecResult};

        let p = params.0;
        let dry_run = p.dry_run.unwrap_or(false);
        let rollback = p.rollback.unwrap_or(false) && !dry_run;
        // Rollback restores on the first failure, so executing past it is wasted
        // work that would only be undone — stop at the first error.
        let stop_on_error = p.stop_on_error.unwrap_or(false) || rollback;

        if p.operations.is_empty() {
            return "Error: operations array is empty".to_string();
        }
        if p.operations.len() > 50 {
            return format!(
                "Error: too many operations ({}). Maximum is 50 per batch.",
                p.operations.len()
            );
        }

        // Snapshot the project before mutating anything so a failed rollback
        // batch can be undone. Skipped for dry_run (nothing executes).
        if rollback && let Err(e) = self.bridge.capture_snapshot() {
            return format!("Error: could not capture rollback snapshot: {e}");
        }

        let capacity = p.operations.len();
        let mut results = Vec::with_capacity(capacity);
        let mut succeeded = 0usize;
        let mut failed = 0usize;

        for (i, op) in p.operations.into_iter().enumerate() {
            if op.tool == "batch_execute" {
                results.push(BatchExecItemResult {
                    index: i,
                    tool: op.tool,
                    success: false,
                    result: "Error: batch_execute cannot be nested".to_string(),
                });
                failed += 1;
                if stop_on_error {
                    break;
                }
                continue;
            }

            // `dispatch_tool` already classified the result (the same
            // `result_is_failure` gate that set its log severity), so use its
            // verdict directly rather than re-parsing the result string here.
            let (result, is_error) = self.dispatch_tool(&op.tool, op.params, dry_run).await;
            results.push(BatchExecItemResult {
                index: i,
                tool: op.tool,
                success: !is_error,
                result,
            });
            if is_error {
                failed += 1;
                if stop_on_error {
                    break;
                }
            } else {
                succeeded += 1;
            }
        }

        // Resolve the rollback snapshot: restore on any failure, else discard.
        let mut rolled_back = false;
        if rollback {
            if failed > 0 {
                match self.bridge.restore_snapshot() {
                    Ok(()) => rolled_back = true,
                    Err(e) => {
                        results.push(BatchExecItemResult {
                            index: results.len(),
                            tool: "<rollback>".to_string(),
                            success: false,
                            result: format!("Error: rollback failed: {e}"),
                        });
                    }
                }
            } else {
                self.bridge.clear_snapshot();
            }
        }

        to_json(&BatchExecResult {
            total: succeeded + failed,
            succeeded,
            failed,
            dry_run,
            rolled_back,
            results,
        })
    }
}

/// Convert input structs to bridge-level types.
fn convert_instrument_def(
    instrument_id: Option<InstrumentId>,
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
            param: pt.effective_target(),
            instrument_id: pt.instrument_id.unwrap_or_default(),
            beat: pt.beat,
            value: pt.value,
            curve: pt.curve.unwrap_or_default(),
            curve_strength: pt.curve_strength,
        })
        .collect()
}

#[cfg(test)]
mod automation_target_input_tests {
    use super::*;

    #[test]
    fn module_target_renders_canonical_dsl_string() {
        let t = AutomationTargetInput::Module {
            module_type: "flt".to_string(),
            instance: 1,
            param_id: "cutoff".into(),
        };
        assert_eq!(t.to_target_string(), "module:flt:1:cutoff");
    }

    #[test]
    fn instrument_target_renders_macro_name() {
        let t = AutomationTargetInput::Instrument {
            param: "FilterCutoff".to_string(),
        };
        assert_eq!(t.to_target_string(), "FilterCutoff");
    }

    fn point(param: Option<&str>, target: Option<AutomationTargetInput>) -> AutomationPointInput {
        AutomationPointInput {
            param: param.map(str::to_string),
            target,
            instrument_id: None,
            beat: 0.0,
            value: 0.5,
            curve: None,
            curve_strength: None,
        }
    }

    #[test]
    fn effective_target_prefers_structured_over_string() {
        let p = point(
            Some("Volume"),
            Some(AutomationTargetInput::Module {
                module_type: "flt".to_string(),
                instance: 2,
                param_id: "resonance".into(),
            }),
        );
        assert_eq!(p.effective_target(), "module:flt:2:resonance");
    }

    #[test]
    fn effective_target_falls_back_to_param_string() {
        assert_eq!(point(Some("Pan"), None).effective_target(), "Pan");
        assert_eq!(point(None, None).effective_target(), "");
    }

    #[test]
    fn validate_automation_point_requires_a_target() {
        assert!(validate_automation_point(&point(None, None)).is_err());
        assert!(validate_automation_point(&point(Some("Volume"), None)).is_ok());
    }
}

#[cfg(test)]
mod batch_msg_tests {
    use super::*;

    #[test]
    fn full_success_leads_with_ok() {
        assert_eq!(batch_msg(3, "widgets set", &[], &[]), "OK: 3 widgets set");
    }

    #[test]
    fn partial_success_leads_with_ok_and_lists_failures() {
        let msg = batch_msg(2, "widgets set", &[], &["boom".to_string()]);
        assert!(msg.starts_with("OK: 2 widgets set"), "got: {msg}");
        assert!(msg.contains("1 failed: boom"), "got: {msg}");
    }

    #[test]
    fn total_failure_leads_with_error_not_ok() {
        // Every item failed: the message must not read as success to a caller
        // that gates on a leading "Error:".
        let msg = batch_msg(0, "widgets set", &[], &["boom".to_string()]);
        assert!(msg.starts_with("Error: 0 widgets set"), "got: {msg}");
        assert!(
            !msg.starts_with("OK:"),
            "total failure must not lead with OK: {msg}"
        );
        assert!(msg.contains("1 failed: boom"), "got: {msg}");
    }

    #[test]
    fn empty_batch_leads_with_ok() {
        // Nothing attempted, nothing failed — still a benign success.
        assert_eq!(batch_msg(0, "widgets set", &[], &[]), "OK: 0 widgets set");
    }

    #[test]
    fn result_is_failure_flags_prose_and_json_total_failure() {
        // Prose leaders.
        assert!(result_is_failure("Error: nope"));
        assert!(result_is_failure(&batch_msg(
            0,
            "x",
            &[],
            &["e".to_string()]
        )));
        assert!(!result_is_failure("OK: 2 x; 1 failed: e"));
        assert!(!result_is_failure("OK: 3 x"));

        // batch_json: total failure (no successes) is a failure; partial/full is not.
        let total_fail = batch_json("created", &Vec::<u64>::new(), &["boom".to_string()]);
        assert!(result_is_failure(&total_fail), "got: {total_fail}");
        let partial = batch_json("created", &[1_u64], &["boom".to_string()]);
        assert!(
            !result_is_failure(&partial),
            "partial success is not a failure"
        );
        let full = batch_json("created", &[1_u64, 2], &[]);
        assert!(!result_is_failure(&full), "full success is not a failure");

        // A non-batch JSON blob with an empty errors list is not a failure.
        assert!(!result_is_failure(r#"{"created":[],"errors":[]}"#));
    }
}

#[cfg(test)]
mod summarize_params_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn batch_execute_reports_op_count() {
        let p = json!({ "operations": [ {"tool": "a"}, {"tool": "b"}, {"tool": "c"} ] });
        assert_eq!(summarize_value(&p), "3 ops");
    }

    #[test]
    fn array_shaped_tool_reports_field_len() {
        let p = json!({ "instruments": [ {}, {} ] });
        assert_eq!(summarize_value(&p), "instruments=2");
    }

    #[test]
    fn single_target_reports_scalar_fields() {
        // BTreeMap key order (default serde_json) is alphabetical.
        let p = json!({ "parameter": "cutoff", "value": 800 });
        assert_eq!(summarize_value(&p), "parameter=cutoff, value=800");
    }

    #[test]
    fn caps_scalar_fields_at_three() {
        let p = json!({ "a": 1, "b": 2, "c": 3, "d": 4 });
        // Alphabetical order, first three only.
        assert_eq!(summarize_value(&p), "a=1, b=2, c=3");
    }

    #[test]
    fn long_summary_is_truncated_with_ellipsis() {
        let long = "x".repeat(200);
        let p = json!({ "name": long });
        let out = summarize_value(&p);
        assert!(
            out.chars().count() <= 60,
            "got {} chars",
            out.chars().count()
        );
        assert!(out.ends_with('…'));
    }

    #[test]
    fn non_object_falls_back_to_json() {
        let p = json!("hello");
        assert_eq!(summarize_value(&p), "\"hello\"");
    }
}

#[cfg(test)]
mod panic_isolation_tests {
    use super::*;

    #[tokio::test]
    async fn passes_through_a_non_panicking_result() {
        let out = run_catching_panic(0, "test_tool", async { 42_u32 }).await;
        assert_eq!(out, Ok(42));
    }

    #[tokio::test]
    async fn recovers_a_str_panic_into_an_err_with_the_message() {
        let out: Result<(), String> =
            run_catching_panic(0, "test_tool", async { panic!("boom") }).await;
        assert_eq!(out, Err("boom".to_string()));
    }

    #[tokio::test]
    async fn recovers_a_string_panic_with_formatted_message() {
        let out: Result<(), String> =
            run_catching_panic(0, "test_tool", async { panic!("bad value {}", 7) }).await;
        assert_eq!(out, Err("bad value 7".to_string()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn recovers_a_panic_from_inside_block_in_place() {
        // The real failure mode: a panic raised inside a synchronous
        // `block_in_place` closure must unwind into this poll and be caught,
        // not propagate up to the worker thread.
        let out: Result<(), String> = run_catching_panic(0, "test_tool", async {
            tokio::task::block_in_place(|| panic!("inside block_in_place"));
        })
        .await;
        assert_eq!(out, Err("inside block_in_place".to_string()));
    }
}

#[cfg(test)]
mod schema_range_tests {
    use super::*;

    /// `#[schemars(range(...))]` on fixed-range numeric MCP fields must surface
    /// machine-readable `minimum`/`maximum` in the generated JSON schema, not
    /// just prose bounds in the description.
    #[test]
    fn fixed_range_fields_expose_min_max_in_schema() {
        // note/velocity: u8 with range(0, 127); channel: Option<u8> range(1, 16).
        let schema = serde_json::to_value(schemars::schema_for!(NoteOnInput))
            .expect("NoteOnInput schema serializes");
        let props = &schema["properties"];

        assert_eq!(props["note"]["maximum"], serde_json::json!(127), "note max");
        assert_eq!(props["note"]["minimum"], serde_json::json!(0), "note min");
        assert_eq!(
            props["velocity"]["maximum"],
            serde_json::json!(127),
            "velocity max"
        );
        assert_eq!(
            props["velocity"]["minimum"],
            serde_json::json!(0),
            "velocity min"
        );
        // channel is Option<u8>; the bounds attach to the inner number schema.
        let channel = serde_json::to_string(&props["channel"]).unwrap();
        assert!(
            channel.contains("\"maximum\":16") && channel.contains("\"minimum\":1"),
            "channel 1..=16 bounds missing: {channel}"
        );
    }

    #[test]
    fn batch_item_schemas_are_concrete() {
        fn schema_text<T: schemars::JsonSchema>() -> String {
            serde_json::to_string(&schemars::schema_for!(T)).unwrap()
        }

        let pattern = schema_text::<CreatePatternsParam>();
        assert!(pattern.contains("length_beats") && pattern.contains("start_beat"));
        let notes = schema_text::<AddNotesParam>();
        assert!(notes.contains("pitch") && notes.contains("duration_beats"));
        let tracks = schema_text::<CreateTracksParam>();
        assert!(tracks.contains("instrument_id"));
        let placements = schema_text::<PlacePatternsParam>();
        assert!(placements.contains("pattern_id") && placements.contains("track_id"));

        let note_modules = schema_text::<AddNoteGraphModuleParam>();
        assert!(
            note_modules.contains("graph_id")
                && note_modules.contains("ProbabilityGate")
                && note_modules.contains("NoteDelay")
        );
        let note_connections = schema_text::<ConnectNoteGraphParam>();
        assert!(note_connections.contains("from") && note_connections.contains("to_input"));
        let mod_nodes = schema_text::<AddModGraphNodeParam>();
        assert!(
            mod_nodes.contains("graph_id")
                && mod_nodes.contains("Macro")
                && mod_nodes.contains("Target")
        );
        let mod_connections = schema_text::<ConnectModGraphParam>();
        assert!(mod_connections.contains("from_port") && mod_connections.contains("to_port"));
    }
}
