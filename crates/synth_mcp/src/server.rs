//! MCP server implementation using rmcp.
//!
//! Defines tool handlers that delegate to the SynthBridge trait.

use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures_util::FutureExt;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CallToolResponse, CallToolResult, CompleteRequestParams, CompleteResult, CompletionInfo,
    ContentBlock, Implementation, JsonObject, ListResourceTemplatesResult, ListResourcesResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    Reference, Resource, ResourceContents, ResourceTemplate, ServerCapabilities, ServerInfo,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use synth_core::{
    BipolarValue, Bpm, Gain, InstrumentId, MidiChannel, MidiNote, NormalizedValue, Semitones,
};
use synth_sequencer::{
    ModGraphId, ModNodeId, NoteGraphId, NoteId, NoteModuleId, PatternId, PlacementLoopMode,
    ReturnBusId, Tick, TrackId,
};

use crate::bridge::SynthBridge;
use crate::error::McpBridgeError;
// Result types named in a tool's signature because it returns them typed
// (`Json<T>`); a string-returning tool never names its payload type. This list
// grows as TODO 6.7 converts more tools.
use crate::types::{
    AnalyzeArrangementResult, AnalyzeBassDrumLockResult, AnalyzeDrumGrooveResult,
    AnalyzeFormMapResult, AnalyzeHarmonicFunctionResult, AnalyzeHarmonyResult,
    AnalyzeHookStrengthResult, AnalyzeInstrumentRangeResult, AnalyzeMaskingMatrixResult,
    AnalyzeMasterChainResult, AnalyzeMixBusResult, AnalyzeNoteResult, AnalyzePatternResult,
    AnalyzeReturnBussesResult, AnalyzeSampleSpectrogramResult, AnalyzeSampleSpectrumResult,
    AnalyzeSectionResult, AnalyzeSpectrogramResult, AnalyzeSpectrumResult,
    AnalyzeTensionCurveResult, AnalyzeVelocityResponseResult, ApplyExamplePatchResult,
    AutoGainStageResult, AutomationLaneInfo, AutomationPointInfo, AutomationSummaryResult,
    AutomationTargetInfo, BatchExecResult, BatchResult, BuildInstrumentResult,
    CompareEnvelopesResult, CompareMixResult, CompareSpectraResult, ConnectionCheckResult,
    ConnectionInfo, CreateChordProgressionResult, DetailedSampleInfo, EngineStatus,
    ExamplePatchInfo, FindMotifsResult, FreezePatternResult, GenerateChordResult, GraphDiagnostic,
    GraphResult, InputDeviceInfo, InputStateInfo, InsertModuleResult, InstrumentInfo,
    InstrumentProfileResult, Listing, MasterVolumeInfo, MatrixRoutingInfo, ModGraphDetail,
    ModGraphInfo, ModTargetInfo, ModuleInfo, ModuleSearchResult, ModuleTypeBrief, ModuleTypeInfo,
    MutationItem, MutationResult, NoteGraphDetail, NoteGraphInfo, NoteInfo, NotePreviewInfo,
    OptimizeResult, ParameterInfo, PatternInfo, PlacementInfo, PortSignalTypeInfo,
    ProjectLintReport, QuantizeNotesToGridResult, QuantizeNotesToScaleResult,
    RebuildInstrumentResult, RenderToWavResult, ReturnBusInfo, ReturnEffectInfo, SampleInfo,
    SamplerStateInfo, SetSongResult, SimplifyAutomationResult, SongInfo, SuggestMusicFixesResult,
    TempoPoint, ToolOutcome, TrackInfo, TransposeNotesResult, UiSnapshot,
    ValidateInstrumentAudioResult, VersionInfo,
};

mod tools;

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

fn arrangement_tick(
    beat: Option<f32>,
    tick: Option<Tick>,
    field: &str,
) -> Result<crate::bridge::PlacementPosition, McpBridgeError> {
    match (beat, tick) {
        (None, None) => Err(McpBridgeError::Other(format!(
            "set at least one of {field}_beat or {field}_tick"
        ))),
        (None, Some(tick)) => Ok(crate::bridge::PlacementPosition::from_tick(tick)),
        (Some(beat), None) => {
            validate_range("arrangement beat", beat, 0.0, 9999.0)?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok(crate::bridge::PlacementPosition::from_tick(Tick(
                (beat * synth_sequencer::TICKS_PER_QUARTER as f32).round() as u64,
            )))
        }
        (Some(beat), Some(tick)) => {
            validate_range("arrangement beat", beat, 0.0, 9999.0)?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let beat_tick = Tick((beat * synth_sequencer::TICKS_PER_QUARTER as f32).round() as u64);
            if beat_tick == tick {
                Ok(crate::bridge::PlacementPosition::from_tick(tick))
            } else {
                Err(McpBridgeError::Other(format!(
                    "{field}_beat and {field}_tick refer to different positions"
                )))
            }
        }
    }
}

fn arrangement_length(
    beats: Option<f32>,
    ticks: Option<u32>,
) -> Result<Option<u32>, McpBridgeError> {
    match (beats, ticks) {
        (Some(_), Some(_)) => Err(McpBridgeError::Other(
            "set at most one of length_beats or length_ticks".to_string(),
        )),
        (None, None) => Ok(None),
        (None, Some(0)) => Err(McpBridgeError::Other(
            "length_ticks must be greater than zero".to_string(),
        )),
        (None, Some(ticks)) => Ok(Some(ticks)),
        (Some(beats), None) => {
            validate_range("length_beats", beats, 0.001, 1024.0)?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok(Some(
                (beats * synth_sequencer::TICKS_PER_QUARTER as f32).round() as u32,
            ))
        }
    }
}

fn placement_to_bridge(
    placement: PlacementInput,
) -> Result<crate::bridge::BridgePlacementData, McpBridgeError> {
    let start_tick = arrangement_tick(placement.start_beat, placement.start_tick, "start")?;
    let transpose = placement.transpose_semitones.unwrap_or(0.0);
    validate_range("transpose_semitones", transpose, -127.0, 127.0)?;
    let gain = placement.gain.unwrap_or(1.0);
    validate_range("gain", gain, 0.0, 2.0)?;
    Ok(crate::bridge::BridgePlacementData {
        pattern_id: placement.pattern_id,
        track_id: placement.track_id,
        start: start_tick,
        transpose_semitones: transpose,
        gain,
        length_ticks: arrangement_length(placement.length_beats, placement.length_ticks)?,
        loop_mode: placement.loop_mode.unwrap_or_default(),
    })
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

/// Run a blocking bridge call on a dedicated worker so the tokio executor stays
/// available for SSE keep-alives, then hand the payload back typed. Use for
/// every tool that performs offline rendering or other long-running synchronous
/// work.
///
/// On a direct call the payload reaches the caller as `structuredContent`
/// against a declared `outputSchema` and a failure becomes an `is_error` result
/// carrying the message. Through `dispatch_tools!` the same failure stays an
/// `Err(String)`, which is what the batch verdict reads.
fn run_blocking_typed<T, E, F>(f: F) -> Result<Json<T>, String>
where
    T: serde::Serialize + schemars::JsonSchema,
    E: std::fmt::Display,
    F: FnOnce() -> Result<T, E>,
{
    tokio::task::block_in_place(|| match f() {
        Ok(result) => Ok(Json(result)),
        Err(e) => Err(format!("Error: {e}")),
    })
}

fn validate_midi_note(_note: MidiNote) -> Result<(), McpBridgeError> {
    Ok(())
}

fn validate_velocity(velocity: u8) -> Result<(), McpBridgeError> {
    if velocity > 127 {
        return Err(McpBridgeError::InvalidVelocity(velocity));
    }
    Ok(())
}

fn validate_midi_channel(_channel: MidiChannel) -> Result<(), McpBridgeError> {
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
    pitch: MidiNote,
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
    pitch: Option<MidiNote>,
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
    midi_channel: Option<MidiChannel>,
    volume: Option<Gain>,
    pan: Option<BipolarValue>,
    modules: &[ModuleDefInput],
    connections: Option<&[ConnectionDefInput]>,
) -> Result<(), McpBridgeError> {
    validate_name("instrument", name)?;
    if let Some(ch) = midi_channel {
        validate_midi_channel(ch)?;
    }
    if let Some(v) = volume {
        validate_range("volume", v.as_f32(), 0.0, 2.0)?;
    }
    if let Some(p) = pan {
        validate_range("pan", p.as_f32(), -1.0, 1.0)?;
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
            validate_midi_note(MidiNote::new(p))?;
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

/// Collects one [`MutationItem`] per requested item, in request order.
///
/// **Push exactly once per requested item, in the order the request listed
/// them** — `index` is the push position, which is what makes it the request
/// position. Every handler already loops over its input array once and reports
/// each item exactly once, so this holds by construction; a handler that wants to
/// skip an item should still record it as failed.
pub(crate) struct Mutations<T> {
    items: Vec<MutationItem<T>>,
}

impl<T> Mutations<T> {
    /// An empty collector.
    fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// One collector for a request of `n` items.
    fn with_capacity(n: usize) -> Self {
        Self {
            items: Vec::with_capacity(n),
        }
    }

    /// This item landed, and the tool has nothing to name for it.
    fn ok(&mut self) {
        self.push(None, None);
    }

    /// This item landed and produced `value` — an id, a path, a label.
    fn named(&mut self, value: T) {
        self.push(Some(value), None);
    }

    /// This item did not land.
    fn failed(&mut self, error: String) {
        self.push(None, Some(error));
    }

    /// This item did not land, and it is item `index` of the request rather than
    /// the next one in push order — for a pre-flight validator that walks the
    /// array and refuses the whole call at the first bad entry, so the entries
    /// before it are never recorded.
    fn failed_at(&mut self, index: usize, error: String) {
        self.items.push(MutationItem {
            index,
            value: None,
            error: Some(error),
        });
    }

    fn push(&mut self, value: Option<T>, error: Option<String>) {
        self.items.push(MutationItem {
            index: self.items.len(),
            value,
            error,
        });
    }

    /// How many items landed.
    fn ok_count(&self) -> usize {
        self.items.iter().filter(|i| i.succeeded()).count()
    }

    /// One message per failed item, in request order — what [`batch_msg`] appends.
    fn errors(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|item| item.error.clone())
            .collect()
    }

    /// The payload, with [`batch_msg`]'s sentence as its `message`.
    ///
    /// For a tool whose item values are *not* strings (a created `InstrumentInfo`,
    /// an imported `SampleInfo`), so the sentence names counts only — the values
    /// themselves are right there in `items`.
    fn into_result(self, noun: &str) -> MutationResult<T> {
        let message = batch_msg(self.ok_count(), noun, &[], &self.errors());
        MutationResult {
            message,
            items: self.items,
        }
    }
}

impl Mutations<String> {
    /// The reply: [`batch_msg`]'s sentence in `content`, the per-item results in
    /// `structuredContent`.
    ///
    /// The prose is byte-identical to what these tools returned before they had a
    /// structured half — `batch_msg` is still handed the same three things, they
    /// are just derived from the items now instead of tracked beside them.
    fn reply(self, noun: &str) -> CallToolResult {
        let details: Vec<String> = self
            .items
            .iter()
            .filter_map(|item| item.value.clone())
            .collect();
        let message = batch_msg(self.ok_count(), noun, &details, &self.errors());
        mutation_reply(MutationResult {
            message,
            items: self.items,
        })
    }
}

/// How long a client may treat `tools/list` as fresh.
///
/// Ten minutes: long enough that reconnecting inside a working session does not
/// re-fetch 219 tools, short enough that a restart with a different
/// `disabled_tools` set is picked up rather than cached indefinitely. Note this
/// saves *transfer*, not context — the catalog still enters the model's prompt
/// once per session, which is TODO §6.13's separate problem.
const TOOL_CATALOG_TTL_MS: u64 = 10 * 60 * 1_000;

/// The `_meta` key a reply states its [`ToolOutcome`] under.
///
/// MCP has one boolean for "did this go wrong", and three answers are needed:
/// a call can complete, half-complete, or do nothing. `is_error` carries the
/// first distinction and this carries the rest, in the extension channel the
/// spec provides for exactly that.
pub const OUTCOME_META_KEY: &str = "pertylizer/outcome";

/// State a reply's verdict on the reply itself.
///
/// Every helper here goes through this, so "what did this call amount to" is
/// answered once, by the code that knows, and read everywhere else. Without it a
/// payload's partiality is only expressed *inside its own type* — as
/// `BuildInstrumentResult::partial_success`, `SetSongResult::errors`,
/// `FreezePatternResult::dropped_events` — which is invisible to anything
/// generic, and a half-built instrument inside a `rollback: true` batch was
/// reported as a clean success and left standing.
fn stamp_outcome(reply: &mut CallToolResult, outcome: ToolOutcome) {
    // A *partial* call is not an error result: the work that landed is real, and
    // the batch decides for itself whether "not complete" is grounds to restore.
    reply.is_error = Some(outcome == ToolOutcome::Failure);
    let mut meta = reply.meta.take().unwrap_or_default();
    if let Ok(value) = serde_json::to_value(outcome) {
        meta.0.insert(OUTCOME_META_KEY.to_string(), value);
    }
    reply.meta = Some(meta);
}

/// The verdict a reply states, if it states one.
fn stated_outcome(reply: &CallToolResult) -> Option<ToolOutcome> {
    reply
        .meta
        .as_ref()
        .and_then(|meta| meta.0.get(OUTCOME_META_KEY))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// A typed payload that can come back *incomplete*, with the verdict its handler
/// worked out from it.
///
/// The five results whose own type carries a "this is not the whole job" flag.
/// They keep their published schema and their JSON body; what they gain is a
/// verdict that something other than their own handler can read.
fn typed_reply<T: serde::Serialize>(payload: &T, outcome: ToolOutcome) -> CallToolResult {
    let text = to_json(payload);
    let mut reply = CallToolResult::success(vec![ContentBlock::text(text)]);
    reply.structured_content = serde_json::to_value(payload).ok();
    stamp_outcome(&mut reply, outcome);
    reply
}

/// A [`typed_reply`] tool's failure: the message, and *no* structured half.
///
/// These five publish their own result type as `output_schema`, so they cannot
/// answer [`action_failed`] on the way out: a `MutationResult` there does not
/// satisfy the schema their own catalog entry advertises, and a client that
/// validates `structuredContent` — which is what `mcp_wire_contract.rs` does to
/// every success reply — rejects the whole result and loses the error with it.
/// Omitting the field is the one shape that is always valid: the spec only
/// requires `structuredContent` to conform when it is present.
fn typed_failure(message: impl Into<String>) -> CallToolResult {
    let mut reply = CallToolResult::success(vec![ContentBlock::text(message.into())]);
    stamp_outcome(&mut reply, ToolOutcome::Failure);
    reply
}

/// Wrap a [`MutationResult`] as a reply: its sentence in `content`, itself in
/// `structuredContent`.
fn mutation_reply<T: serde::Serialize>(payload: MutationResult<T>) -> CallToolResult {
    let outcome = payload.outcome();
    let mut reply = CallToolResult::success(vec![ContentBlock::text(payload.message.clone())]);
    // Serialization of a struct of owned strings cannot fail; leaving the field
    // unset on the impossible branch degrades to a prose-only reply rather than
    // failing the call.
    reply.structured_content = serde_json::to_value(payload).ok();
    // Stated by the code that has the items in hand, rather than recovered
    // downstream from the sentence.
    stamp_outcome(&mut reply, outcome);
    reply
}

/// A one-item mutator's confirmation: it did the one thing it was asked to.
///
/// The ~50 tools that write their own sentence ("OK: master volume set to 0.80x")
/// rather than going through [`batch_msg`]. They say so explicitly instead of
/// having it read back off the sentence's leading marker — see [`ToolOutcome`].
fn action_ok(message: impl Into<String>) -> CallToolResult {
    let message = message.into();
    let mut items = Mutations::<String>::with_capacity(1);
    items.ok();
    mutation_reply(MutationResult {
        message,
        items: items.items,
    })
}

/// A one-item mutator's failure: it did not do the thing, and the message says
/// why.
///
/// The message lands in both halves — as the reply's text and as the item's
/// `error` — because for these tools the sentence *is* the error: there is no
/// second per-item message to report.
fn action_failed(message: impl Into<String>) -> CallToolResult {
    let message = message.into();
    let mut items = Mutations::<String>::with_capacity(1);
    items.failed(message.clone());
    mutation_reply(MutationResult {
        message,
        items: items.items,
    })
}

/// A one-shot mutator that acted on `n` items and wrote its own sentence.
///
/// `set_tempo_at` and `remove_tempo_at` each address N ticks and report one
/// sentence. [`action_ok`] would claim one item and under-report the call, so they
/// say how many. There is nothing per-item to name — the sentence already carries
/// the count — so the entries are bare.
///
/// `n` is the number of **requested** items, not of things that changed: `index`
/// is a request position, so a shorter list would silently re-point every entry
/// at the wrong request element.
fn action_ok_n(message: impl Into<String>, n: usize) -> CallToolResult {
    let message = message.into();
    let mut items = Mutations::<String>::with_capacity(n);
    for _ in 0..n {
        items.ok();
    }
    mutation_reply(MutationResult {
        message,
        items: items.items,
    })
}

/// A mutating tool's pre-flight refusal, naming the requested item that caused it.
///
/// The `index` is the whole point of a per-item result: it is what lets a caller
/// line a failure up against the request that produced it. A refusal that always
/// said `0` — which is what these reported before — points at the first item
/// however far down the array the bad one actually is, which is worse than saying
/// nothing, because it looks like an answer.
///
/// It carries the structured half for the same reason every other reply does: the
/// tool declares an `output_schema`, so a reply without `structuredContent`
/// contradicts its own catalog entry for any client that validates the payload.
fn action_rejected_at(index: usize, message: String) -> CallToolResult {
    let mut items = Mutations::<String>::with_capacity(1);
    items.failed_at(index, message.clone());
    mutation_reply(MutationResult {
        message,
        items: items.items,
    })
}

/// A pre-flight refusal that belongs to the call rather than to one item — a bad
/// scalar argument, a path that fails validation, a tool given no array at all.
fn action_rejected(message: String) -> CallToolResult {
    action_rejected_at(0, message)
}

/// The text a reply carries, for the batch path, which speaks strings.
fn reply_text(reply: &CallToolResult) -> String {
    reply
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The schema every [`action_reply`] answers against.
///
/// Named rather than spelled out at each `#[tool(output_schema = …)]`: the
/// mutating tools share one payload type, and repeating the turbofish eighty
/// times invites one of them to drift onto a different type unnoticed.
///
/// Built once and cloned by `Arc`: it is the *same* document for all 112 action
/// tools, and generating it per route left the router holding 112 identical
/// copies of it. `build_router` keeps the sharing intact by only replacing a
/// schema that `$ref`-inlining actually rewrote — so the inlining happens *here*,
/// once. `MutationResult<T>` `$ref`s its per-item type into `$defs`, and leaving
/// that for `build_router` would have it rewrite and re-`Arc` this document 112
/// times, which is exactly what the `OnceLock` exists to avoid.
fn action_output_schema() -> std::sync::Arc<JsonObject> {
    static SCHEMA: std::sync::OnceLock<std::sync::Arc<JsonObject>> = std::sync::OnceLock::new();
    std::sync::Arc::clone(SCHEMA.get_or_init(|| {
        let generated = rmcp::handler::server::tool::schema_for_output::<MutationResult<String>>();
        std::sync::Arc::new(inline_schema_refs(generated.as_ref()))
    }))
}

/// A reply's verdict, from the reply itself.
///
/// This is the whole of the server's failure classification, and it reads only
/// two things: the `is_error` flag every reply already carries, and — for a
/// mutator — the per-item results it published.
///
/// What it replaces is worth recording, because the bug class recurred three
/// times. The verdict used to be *inferred* from serialized output: a leading
/// `"Error:"` in the prose, plus a JSON-shape guess for the payloads that stayed
/// valid JSON when they failed. That guess read `ApplyExamplePatchResult`'s
/// non-fatal notes as a total failure, then `BuildInstrumentResult`'s and
/// `RebuildInstrumentResult`'s, each found by inspecting one more type; keying it
/// to three field names by hand fixed the symptom and left a predicate that a
/// rename would silently blind. Worse, it could not see trouble a handler
/// reported *inside* a success sentence — `"Script saved … but did not compile"`
/// read as a clean install.
///
/// Now every reply states its own verdict at the point that knows it:
/// `mutation_reply` stamps `is_error` from [`MutationResult::outcome`], and a
/// typed tool's failure is an `Err`, which rmcp renders as `is_error` itself. The
/// deserializations below are not a shape guess: each is one of our own result
/// types deserialized back into itself and asked its own predicate, needed only
/// because `CallToolResult` carries a `Value` rather than the `T` it came from.
///
/// Three types answer, because three can record a total failure inside an
/// otherwise-successful reply. [`MutationResult`] is one; the other two are
/// [`BatchResult`] and [`BatchExecResult`], which reach the client through
/// `Json<T>` — and `CallToolResult::structured` hardcodes `is_error: Some(false)`,
/// so a `update_note` whose every id was unknown, or a `batch_execute` whose every
/// op failed, has no other way to say so.
///
/// The `contains_key` gates only choose which type to try, so a large reader
/// payload is not deserialized three times over.
fn reply_outcome(structured: Option<&serde_json::Value>, is_error: bool) -> ToolOutcome {
    if is_error {
        return ToolOutcome::Failure;
    }
    let Some(value) = structured else {
        return ToolOutcome::Success;
    };
    let Some(map) = value.as_object() else {
        return ToolOutcome::Success;
    };
    // `MutationResult<Value>` deserializes whatever the item values were. Gated on
    // *both* its fields, not `message` alone: reader payloads carry a `message`
    // too (`ConnectionCheckResult`, `CompareMixResult`), and only the pair is
    // characteristic of a mutation reply.
    if map.contains_key("message")
        && map.contains_key("items")
        && let Ok(result) =
            serde_json::from_value::<MutationResult<serde_json::Value>>(value.clone())
    {
        return result.outcome();
    }
    if map.contains_key("succeeded") {
        if let Ok(result) = serde_json::from_value::<BatchResult>(value.clone()) {
            return result.outcome();
        }
        if let Ok(result) = serde_json::from_value::<BatchExecResult>(value.clone()) {
            return result.outcome();
        }
    }
    ToolOutcome::Success
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

// A real enum rather than a validated `String` so the closed set reaches the
// caller in the tool's `inputSchema` as a JSON Schema `enum` — MCP has no way to
// complete a *tool* argument, so the schema is the only place a client can learn
// the valid values without a discovery round trip. Rejecting a bad value stops
// being our hand-written check and becomes deserialization.
//
// Kept as a plain comment, not a doc comment: schemars publishes `///` as the
// type's `description`, and every word of it then ships to every client on every
// `tools/list`. Rationale for us, not for them.
/// A module category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename_all = "lowercase")]
pub enum ModuleCategoryFilter {
    Voice,
    Effect,
    Visualizer,
}

impl ModuleCategoryFilter {
    /// The wire spelling, which is what the bridge matches on.
    fn as_str(self) -> &'static str {
        match self {
            Self::Voice => "voice",
            Self::Effect => "effect",
            Self::Visualizer => "visualizer",
        }
    }
}

// See `ModuleCategoryFilter` above for why this is an enum, and why that
// reasoning is not a doc comment.
/// A port signal type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
#[schemars(rename_all = "lowercase")]
pub enum SignalTypeFilter {
    Audio,
    Control,
    Gate,
    Midi,
}

impl SignalTypeFilter {
    /// The wire spelling, which is what the bridge matches on.
    fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Control => "control",
            Self::Gate => "gate",
            Self::Midi => "midi",
        }
    }
}

/// URI template for the module-type resource, and the name of the variable
/// inside it. Shared by `list_resource_templates`, `read_resource` and
/// `complete`, so a completion cannot offer a value the reader would reject.
const MODULE_TYPE_URI_TEMPLATE: &str = "synth://module-types/{type_key}";
const MODULE_TYPE_URI_PREFIX: &str = "synth://module-types/";
const MODULE_TYPE_URI_VARIABLE: &str = "type_key";

/// The same three for the example-patch resource.
const PATCH_URI_TEMPLATE: &str = "synth://patches/{name}";
const PATCH_URI_PREFIX: &str = "synth://patches/";
const PATCH_URI_VARIABLE: &str = "name";

/// The slug an example patch is addressed by in its resource URI.
///
/// One definition because three places must agree on it: the URI
/// `list_resources` advertises, the lookup `read_resource` performs, and the
/// values `complete` offers.
fn patch_slug(name: &str) -> String {
    name.to_ascii_lowercase().replace(' ', "-")
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
    #[schemars(description = "Filter by module category")]
    pub category: Option<ModuleCategoryFilter>,
    #[schemars(description = "Filter to modules that have an input port of this signal type")]
    pub has_input_type: Option<SignalTypeFilter>,
    #[schemars(description = "Filter to modules that have an output port of this signal type")]
    pub has_output_type: Option<SignalTypeFilter>,
    #[schemars(
        description = "Text search in module name, description, and parameter names (case-insensitive)"
    )]
    pub query: Option<String>,
    #[schemars(
        description = "How many matches to return, best-first (default 20). Each match carries \
        full port/parameter detail, so a large limit is a large reply; `total_matched` always \
        reports how many there were. Use list_module_types for the whole catalog in compact form."
    )]
    pub limit: Option<usize>,
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
    pub note: MidiNote,
    #[schemars(
        description = "Velocity (0-127, where 127 = maximum)",
        range(min = 0, max = 127)
    )]
    pub velocity: u8,
    #[schemars(
        description = "MIDI channel (1-16, default 1)",
        range(min = 1, max = 16)
    )]
    pub channel: Option<MidiChannel>,
    #[schemars(description = "Optional instrument ID; when set, bypasses MIDI-channel routing")]
    pub instrument_id: Option<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOnParam {
    #[schemars(description = "Notes to trigger on (one or many — e.g. a whole chord in one call)")]
    pub notes: Vec<NoteOnInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOffInput {
    #[schemars(description = "MIDI note number (0-127)", range(min = 0, max = 127))]
    pub note: MidiNote,
    #[schemars(
        description = "MIDI channel (1-16, default 1)",
        range(min = 1, max = 16)
    )]
    pub channel: Option<MidiChannel>,
    #[schemars(description = "Optional instrument ID; when set, releases only that instrument")]
    pub instrument_id: Option<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOffParam {
    #[schemars(description = "Notes to release (one or many)")]
    pub notes: Vec<NoteOffInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetInputDeviceParam {
    #[schemars(
        description = "Device id/name from list_input_devices, or null for the default input"
    )]
    pub device_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct StopRecordingParam {
    #[schemars(description = "Optional name for the sample created from the recording")]
    pub name: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PreviewNoteParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(
        description = "MIDI note number (0-127, where 60 = middle C)",
        range(min = 0, max = 127)
    )]
    pub note: MidiNote,
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
    pub note: Option<MidiNote>,
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
    pub start_tick: Option<Tick>,
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
        description = "Release/effect tail captured after the requested range (default 1.0 seconds, max 30). The transport stops before this tail, so later arrangement events are not triggered."
    )]
    pub tail_seconds: Option<f32>,
    #[schemars(
        description = "Absolute tick to start rendering from (default 0 = song beginning)."
    )]
    pub start_tick: Option<Tick>,
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
    pub start_tick: Option<Tick>,
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
    pub start_tick: Option<Tick>,
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
    pub start_tick: Option<Tick>,
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
        description = "Enable the time-resolved (per-frame) distance (default true). It scores only target-energy frames by default, so sparse/staccato material ranks correctly instead of averaging over silence. Set false only when aggregate-only output is required."
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
    pub start_tick: Option<Tick>,
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
    pub start_tick: Option<Tick>,
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
    pub start_tick: Option<Tick>,
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
    pub start_tick: Option<Tick>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeSectionParam {
    #[schemars(description = "Absolute tick where the section starts (inclusive).")]
    pub start_tick: Tick,
    #[schemars(description = "Absolute tick where the section ends (exclusive).")]
    pub end_tick: Tick,
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
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub pattern_id: Option<PatternId>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0 (song beginning). Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub exclude_track_ids: Option<Vec<TrackId>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzePatternParam {
    #[schemars(description = "Pattern ID to analyze.")]
    pub pattern_id: PatternId,
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
    pub note: MidiNote,
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
    pub pattern_id: PatternId,
    #[schemars(
        description = "Signed semitone shift (e.g. +5 = up a fourth, -12 = down an octave)."
    )]
    pub semitones: Semitones,
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
    pub pattern_id: PatternId,
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
    pub pattern_id: PatternId,
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
    pub pattern_id: Option<PatternId>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0. Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<Tick>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeHarmonicFunctionParam {
    #[schemars(
        description = "Pattern ID to analyze. When set, the arrangement_* fields are ignored and analysis runs on that pattern's notes in pattern-relative ticks. Leave unset to analyze the arrangement instead."
    )]
    pub pattern_id: Option<PatternId>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0 (song beginning). Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub exclude_track_ids: Option<Vec<TrackId>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeArrangementParam {
    #[schemars(
        description = "Pattern ID to analyze. When set, the analyzer runs over the pattern's bars (in pattern-relative ticks) instead of the arrangement. arrangement_* fields are ignored when this is set."
    )]
    pub pattern_id: Option<PatternId>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0. Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub exclude_track_ids: Option<Vec<TrackId>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeFormMapParam {
    #[schemars(description = "Pattern ID. When set, the arrangement_* fields are ignored.")]
    pub pattern_id: Option<PatternId>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub exclude_track_ids: Option<Vec<TrackId>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindMotifsParam {
    #[schemars(description = "Pattern ID. When set, arrangement_* fields are ignored.")]
    pub pattern_id: Option<PatternId>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub exclude_track_ids: Option<Vec<TrackId>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeHookStrengthParam {
    #[schemars(description = "Pattern ID. When set, arrangement_* fields are ignored.")]
    pub pattern_id: Option<PatternId>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub exclude_track_ids: Option<Vec<TrackId>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeTensionCurveParam {
    #[schemars(description = "Pattern ID. When set, arrangement_* fields are ignored.")]
    pub pattern_id: Option<PatternId>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub exclude_track_ids: Option<Vec<TrackId>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SuggestMusicFixesParam {
    #[schemars(description = "Pattern ID. When set, arrangement_* fields are ignored.")]
    pub pattern_id: Option<PatternId>,
    #[schemars(description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0.")]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub exclude_track_ids: Option<Vec<TrackId>>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AnalyzeBassDrumLockParam {
    #[schemars(
        description = "Pattern ID to analyze. When set, the analyzer treats notes with GM kick MIDI numbers (35, 36) as kicks and everything else as bass. Useful for combined rhythm-section patterns. Ignored when arrangement_* fields are set."
    )]
    pub pattern_id: Option<PatternId>,
    #[schemars(
        description = "Arrangement-mode start tick (inclusive, absolute). Defaults to 0. Ignored when pattern_id is set."
    )]
    pub arrangement_start_tick: Option<Tick>,
    #[schemars(
        description = "Arrangement-mode end tick (exclusive, absolute). Defaults to the full arrangement length. Ignored when pattern_id is set."
    )]
    pub arrangement_end_tick: Option<Tick>,
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
    pub note: MidiNote,
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
    pub pattern_id: PatternId,
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
    pub track_id: TrackId,
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
    pub track_id: TrackId,
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
    pub volume: Option<Gain>,
    #[serde(default)]
    #[schemars(
        description = "Pan position (-1.0 = left, 0.0 = center, 1.0 = right). Omit to leave unchanged."
    )]
    pub pan: Option<BipolarValue>,
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
    pub channel: MidiChannel,
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
    pub bpm: Bpm,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TempoPointParam {
    #[schemars(description = "Absolute position in ticks (960 ticks = 1 quarter note)")]
    pub tick: Tick,
    #[schemars(description = "Tempo in BPM at this point (20-999)")]
    pub bpm: Bpm,
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
    pub ticks: Vec<Tick>,
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
    pub pattern_id: PatternId,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeletePatternsParam {
    #[schemars(
        description = "Pattern IDs to delete (one or many). Also removes all placements of each pattern."
    )]
    pub pattern_ids: Vec<PatternId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveNotesParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: PatternId,
    #[schemars(description = "Note IDs to remove (one or many)")]
    pub note_ids: Vec<NoteId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemovePlacementsParam {
    #[schemars(
        description = "Placements to remove (one or many), each identified by pattern_id, track_id, and exactly one of start_beat/start_tick"
    )]
    pub placements: Vec<PlacementLocatorInput>,
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
    pub pitch: MidiNote,
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
    pub pattern_id: PatternId,
    #[schemars(description = "Array of notes to add")]
    pub notes: Vec<NoteInput>,
}

/// A note update in a batch operation.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteUpdateInput {
    #[schemars(description = "Note ID to update")]
    pub note_id: NoteId,
    #[schemars(
        description = "New MIDI pitch (0-127), or null to keep current",
        range(min = 0, max = 127)
    )]
    pub pitch: Option<MidiNote>,
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
    pub pattern_id: PatternId,
    #[schemars(description = "Array of note updates")]
    pub updates: Vec<NoteUpdateInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReplaceNotesParam {
    #[schemars(description = "Pattern ID to replace notes in (clears existing notes first)")]
    pub pattern_id: PatternId,
    #[schemars(description = "Array of new notes to insert")]
    pub notes: Vec<NoteInput>,
}

// === Note Grid (pooled note-processing graphs) ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteGraphIdParam {
    #[schemars(description = "Note graph id (from list_note_graphs)")]
    pub graph_id: NoteGraphId,
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
    pub graph_ids: Vec<NoteGraphId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddNoteGraphModuleInput {
    #[schemars(description = "Note graph id to add the module to")]
    pub graph_id: NoteGraphId,
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
    pub graph_id: NoteGraphId,
    #[schemars(description = "Module id to replace (from get_note_graph)")]
    pub module_id: NoteModuleId,
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
    pub graph_id: NoteGraphId,
    #[schemars(description = "NoteScriptTransform module id (from get_note_graph)")]
    pub module_id: NoteModuleId,
    #[schemars(
        description = "YAMS note_event source. Runs per note (1:1). Read note_pitch/note_vel/note_dur/tick and value inputs in1..in4; assign out.pitch/out.vel/out.dur/out.gate. A negative out.vel drops the note; a negative out.dur restores 'plays until cut'. Empty source = pass-through. Example: `out.pitch = note_pitch + 12`."
    )]
    pub source: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveNoteGraphModuleParam {
    #[schemars(description = "Note graph id")]
    pub graph_id: NoteGraphId,
    #[schemars(description = "Module id to remove (from get_note_graph)")]
    pub module_id: NoteModuleId,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectNoteGraphInput {
    #[schemars(description = "Note graph id")]
    pub graph_id: NoteGraphId,
    #[schemars(description = "Source (output-side) module id")]
    pub from: NoteModuleId,
    #[schemars(description = "Destination (input-side) module id")]
    pub to: NoteModuleId,
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
pub struct GetNoteGraphParam {
    #[schemars(description = "The note graph to read")]
    pub graph_id: NoteGraphId,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteGraphMetadataInput {
    #[schemars(description = "Note graph ID to update")]
    pub graph_id: NoteGraphId,
    #[schemars(description = "Replacement name; omitted keeps the current name")]
    pub name: Option<String>,
    #[schemars(description = "Replacement description; empty clears it, omitted keeps it")]
    pub description: Option<String>,
    #[schemars(description = "Replacement #rrggbb color; null/omitted keeps the current color")]
    pub color: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteGraphMetadataParam {
    #[schemars(description = "Note graph metadata updates (one or many)")]
    pub items: Vec<SetNoteGraphMetadataInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatternNoteGraphInput {
    #[schemars(description = "Pattern id to bind")]
    pub pattern_id: PatternId,
    #[schemars(
        description = "Note graph id to bind, or null/omitted to clear the binding (the pattern's raw notes + per-note ornaments then play)."
    )]
    pub graph_id: Option<NoteGraphId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetPatternNoteGraphParam {
    #[schemars(description = "Pattern→graph bindings to set or clear (one or many).")]
    pub items: Vec<SetPatternNoteGraphInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteNoteGraphInput {
    #[schemars(description = "Pattern id containing the note")]
    pub pattern_id: PatternId,
    #[schemars(description = "Note id (from list_notes)")]
    pub note_id: NoteId,
    #[schemars(
        description = "Note graph id to bind for per-note articulation, or null/omitted to clear the binding."
    )]
    pub graph_id: Option<NoteGraphId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetNoteNoteGraphParam {
    #[schemars(description = "Per-note graph bindings to set or clear (one or many).")]
    pub items: Vec<SetNoteNoteGraphInput>,
}

// === Mod Grid (pooled control-rate modulator graphs) ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateModGraphParam {
    #[schemars(description = "Name for the new mod graph")]
    pub name: String,
    #[schemars(description = "Optional free-text description")]
    pub description: Option<String>,
    #[schemars(description = "Optional color as #rrggbb")]
    pub color: Option<String>,
    #[schemars(
        description = "Scope: 'global' (one always-on instance, default) or 'track' (one instance per assigned track; assign with assign_mod_graph)."
    )]
    pub scope: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteModGraphParam {
    #[schemars(description = "Mod graph ids to delete (one or many).")]
    pub graph_ids: Vec<ModGraphId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DuplicateModGraphParam {
    #[schemars(description = "Mod graph id to duplicate")]
    pub graph_id: ModGraphId,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModGraphScopeParam {
    #[schemars(description = "Mod graph id")]
    pub graph_id: ModGraphId,
    #[schemars(description = "New scope: 'global' or 'track'.")]
    pub scope: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AssignModGraphParam {
    #[schemars(description = "Mod graph id (must be 'track' scope to run)")]
    pub graph_id: ModGraphId,
    #[schemars(
        description = "Track ids to assign (replaces the current set; one running instance per track). Unknown ids are rejected."
    )]
    pub tracks: Vec<TrackId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AddModGraphNodeInput {
    #[schemars(description = "Mod graph id to add the node to")]
    pub graph_id: ModGraphId,
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
    pub graph_id: ModGraphId,
    #[schemars(description = "Node id to remove (drops every cable touching it)")]
    pub node_id: ModNodeId,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectModGraphInput {
    #[schemars(description = "Mod graph id")]
    pub graph_id: ModGraphId,
    #[schemars(description = "Source node id")]
    pub from: ModNodeId,
    #[schemars(description = "Source output port name (e.g. 'out')")]
    pub from_port: String,
    #[schemars(description = "Destination node id")]
    pub to: ModNodeId,
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
    pub graph_id: ModGraphId,
    #[schemars(description = "Existing node id to edit in place")]
    pub node_id: ModNodeId,
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
pub struct GetModGraphParam {
    #[schemars(description = "The mod graph to read")]
    pub graph_id: ModGraphId,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModGraphMetadataInput {
    #[schemars(description = "Mod graph ID to update")]
    pub graph_id: ModGraphId,
    #[schemars(description = "Replacement name; omitted keeps the current name")]
    pub name: Option<String>,
    #[schemars(description = "Replacement description; empty clears it, omitted keeps it")]
    pub description: Option<String>,
    #[schemars(description = "Replacement #rrggbb color; null/omitted keeps the current color")]
    pub color: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetModGraphMetadataParam {
    #[schemars(description = "Mod graph metadata updates (one or many)")]
    pub items: Vec<SetModGraphMetadataInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListModTargetsParam {
    #[schemars(
        description = "Restrict to one graph's routings, or null/omit for every graph's routings."
    )]
    pub graph_id: Option<ModGraphId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NoteOrnamentInput {
    #[schemars(description = "Pattern ID containing the note")]
    pub pattern_id: PatternId,
    #[schemars(description = "Note ID (from list_notes)")]
    pub note_id: NoteId,
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
    pub pattern_ids: Vec<PatternId>,
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
        track_id: Option<TrackId>,
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

/// Automation target accepted either as the canonical DSL string or as the
/// same structured object supported by `add_automation_points`.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum AutomationTargetSelector {
    Dsl(String),
    Structured(AutomationTargetInput),
}

impl AutomationTargetSelector {
    fn to_target_string(&self) -> String {
        match self {
            Self::Dsl(target) => target.clone(),
            Self::Structured(target) => target.to_target_string(),
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
    #[schemars(description = "Instrument ID (default 0 for instrument/module targets)")]
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
    pub curve_strength: Option<synth_sequencer::CurveStrength>,
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
    pub pattern_id: PatternId,
    #[schemars(description = "Automation points to add")]
    pub points: Vec<AutomationPointInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetAutomationPointsParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: PatternId,
    #[schemars(
        description = "Target DSL: instrument macro (Volume/Pan/FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release), module:<type>:<instance>:<param>, track:<param>[:<track_id>], or global:MasterVolume. From list_automation_lanes, pass the lane target string back verbatim."
    )]
    pub target: AutomationTargetSelector,
    #[schemars(description = "Instrument ID (default 0 for instrument/module targets)")]
    pub instrument_id: Option<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveAutomationPointsParam {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: PatternId,
    #[schemars(
        description = "Target DSL: instrument macro (Volume/Pan/FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release), module:<type>:<instance>:<param>, track:<param>[:<track_id>], or global:MasterVolume. From list_automation_lanes, pass the lane target string back verbatim."
    )]
    pub target: AutomationTargetSelector,
    #[schemars(description = "Instrument ID (default 0 for instrument/module targets)")]
    pub instrument_id: Option<InstrumentId>,
    #[schemars(description = "Beat positions of points to remove")]
    pub beats: Vec<f32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClearAutomationLaneInput {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: PatternId,
    #[schemars(
        description = "Target DSL: instrument macro (Volume/Pan/FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release), module:<type>:<instance>:<param>, track:<param>[:<track_id>], or global:MasterVolume. From list_automation_lanes, pass the lane target string back verbatim."
    )]
    pub target: AutomationTargetSelector,
    #[schemars(description = "Instrument ID (default 0 for instrument/module targets)")]
    pub instrument_id: Option<InstrumentId>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ClearAutomationLaneParam {
    #[schemars(description = "Automation lanes to clear (one or many)")]
    pub items: Vec<ClearAutomationLaneInput>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SimplifyAutomationParam {
    #[schemars(
        description = "Normalized error tolerance (0.0..1.0). A point is removed only when the surrounding segment reproduces its value within this margin, so it bounds the maximum change to the automation. Larger = more aggressive. Typical: 0.005–0.02.",
        range(min = 0.0, max = 1.0)
    )]
    pub tolerance: f32,
    #[schemars(description = "Restrict to one pattern (omit = every pattern in the song).")]
    pub pattern_id: Option<PatternId>,
    #[schemars(
        description = "Restrict to a single lane by target DSL (omit = every lane in scope). Same target strings list_automation_lanes returns."
    )]
    pub target: Option<AutomationTargetSelector>,
    #[schemars(
        description = "Instrument ID used to resolve an instrument/module `target` (default 0). Ignored when `target` is omitted."
    )]
    pub instrument_id: Option<InstrumentId>,
    #[serde(default)]
    #[schemars(
        description = "false (default) = dry-run: report the before/after counts and max error without changing anything. true = rewrite the lanes."
    )]
    pub apply: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScaleAutomationLaneInput {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: PatternId,
    #[schemars(description = "Target lane (e.g. 'module:flt:1:cutoff' or 'FilterCutoff')")]
    pub target: AutomationTargetSelector,
    #[schemars(description = "Instrument ID (default 0 for instrument/module targets)")]
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
    pub pattern_id: PatternId,
    #[schemars(description = "Target lane (e.g. 'module:flt:1:cutoff' or 'FilterCutoff')")]
    pub target: AutomationTargetSelector,
    #[schemars(description = "Instrument ID (default 0 for instrument/module targets)")]
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
    pub from_pattern_id: PatternId,
    #[schemars(description = "Source target lane")]
    pub from_target: AutomationTargetSelector,
    #[schemars(description = "Source instrument ID (default 0 for instrument/module targets)")]
    pub from_instrument_id: Option<InstrumentId>,
    #[schemars(description = "Destination pattern ID (may equal the source)")]
    pub to_pattern_id: PatternId,
    #[schemars(description = "Destination target lane")]
    pub to_target: AutomationTargetSelector,
    #[schemars(
        description = "Destination instrument ID (default 0 for instrument/module targets)"
    )]
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
    pub track_id: TrackId,
    #[serde(default)]
    #[schemars(description = "Volume (0.0 = silent, 1.0 = full). Omit to leave unchanged.")]
    pub volume: Option<NormalizedValue>,
    #[serde(default)]
    #[schemars(
        description = "Pan position (-1.0 = left, 0.0 = center, 1.0 = right). Omit to leave unchanged."
    )]
    pub pan: Option<BipolarValue>,
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
    pub track_id: TrackId,
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
    pub return_ids: Vec<ReturnBusId>,
}

/// One return bus's mixer update for `set_return_bus_mixer`. Every field except
/// `return_id` is optional; only the fields that are present are changed.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnBusMixerInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: ReturnBusId,
    #[serde(default)]
    #[schemars(description = "Volume (0.0 = silent, 1.0 = full). Omit to leave unchanged.")]
    pub volume: Option<NormalizedValue>,
    #[serde(default)]
    #[schemars(
        description = "Pan position (-1.0 = left, 0.0 = center, 1.0 = right). Omit to leave unchanged."
    )]
    pub pan: Option<BipolarValue>,
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
    pub return_id: ReturnBusId,
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
    pub return_id: ReturnBusId,
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
    pub return_id: ReturnBusId,
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
    pub track_id: TrackId,
    #[schemars(description = "Destination return bus ID")]
    pub return_id: ReturnBusId,
    #[schemars(
        description = "Send level: 0.0 = none, 1.0 = unity (the maximum). Values above 1.0 are rejected — boosted sends are not supported; raise the return-bus volume or add a second return instead.",
        range(min = 0.0, max = 1.0)
    )]
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
    pub track_id: TrackId,
    #[schemars(description = "Destination return bus ID the send targets")]
    pub return_id: ReturnBusId,
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
    pub return_id: ReturnBusId,
    #[schemars(
        description = "Effect type keys to add (one or many), in chain order (e.g. ['eq', 'rev']). Each accepts the prefix or display name (e.g. 'rev', 'delay', 'chorus', 'compressor', 'distortion'). Voice modules are rejected."
    )]
    pub effect_types: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RemoveReturnEffectsParam {
    #[schemars(description = "Return bus ID")]
    pub return_id: ReturnBusId,
    #[schemars(
        description = "Effect module-id strings to remove (one or many), e.g. ['rev-1'], from list_return_busses"
    )]
    pub module_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReturnEffectParamInput {
    #[schemars(description = "Return bus ID")]
    pub return_id: ReturnBusId,
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
    pub return_id: ReturnBusId,
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
    pub return_id: ReturnBusId,
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
    pub from_id: ReturnBusId,
    #[schemars(description = "Destination return bus ID")]
    pub to_id: ReturnBusId,
    #[schemars(
        description = "Send level: 0.0 = none, 1.0 = unity (the maximum). Values above 1.0 are rejected — raise the destination return-bus volume instead of boosting the send.",
        range(min = 0.0, max = 1.0)
    )]
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
    pub from_id: ReturnBusId,
    #[schemars(description = "Destination return bus ID the send targets")]
    pub to_id: ReturnBusId,
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
    pub volume: Gain,
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
    pub track_id: TrackId,
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
    pub track_ids: Vec<TrackId>,
}

// === Pattern management parameter structs ===

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RenamePatternInput {
    #[schemars(description = "Pattern ID")]
    pub pattern_id: PatternId,
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
    pub pattern_id: PatternId,
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
    pub pattern_ids: Vec<PatternId>,
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

/// One MSEG segment in a complete shape update.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct MsegSegmentInput {
    #[schemars(
        description = "Segment duration in seconds",
        range(min = 0.0, max = 60.0)
    )]
    pub time: f32,
    #[schemars(
        description = "Target level at the end of the segment",
        range(min = 0.0, max = 1.0)
    )]
    pub level: f32,
    #[schemars(
        description = "Curve shape: -1 logarithmic, 0 linear, +1 exponential",
        range(min = -1.0, max = 1.0)
    )]
    pub curve: f32,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SetMsegSegmentsParam {
    #[schemars(description = "Instrument ID (0 for default instrument)")]
    pub instrument_id: InstrumentId,
    #[schemars(description = "MSEG module ID (e.g. 'msg-1')")]
    pub module_id: String,
    #[schemars(description = "Complete MSEG shape, from 1 to 16 segments")]
    pub segments: Vec<MsegSegmentInput>,
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
    pub pattern_id: PatternId,
    #[schemars(description = "Track ID to place on")]
    pub track_id: TrackId,
    #[schemars(description = "Start position in beats; may accompany an agreeing start_tick")]
    pub start_beat: Option<f32>,
    #[schemars(
        description = "Exact start position in ticks; may accompany an agreeing start_beat"
    )]
    pub start_tick: Option<Tick>,
    #[serde(default)]
    #[schemars(description = "Placement transpose in semitones (default 0)")]
    pub transpose_semitones: Option<f32>,
    #[serde(default)]
    #[schemars(description = "Linear placement gain (default 1, range 0..2)")]
    pub gain: Option<f32>,
    #[serde(default)]
    #[schemars(
        description = "Optional placement length override in beats; mutually exclusive with length_ticks"
    )]
    pub length_beats: Option<f32>,
    #[serde(default)]
    #[schemars(
        description = "Optional exact placement length override in ticks; mutually exclusive with length_beats"
    )]
    pub length_ticks: Option<u32>,
    #[serde(default)]
    #[schemars(description = "Playback beyond the source pattern: repeat (default) or clip")]
    pub loop_mode: Option<PlacementLoopMode>,
}

/// The exact identity of an existing pattern placement.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlacementLocatorInput {
    #[schemars(description = "Pattern ID of the existing placement")]
    pub pattern_id: PatternId,
    #[schemars(description = "Track ID of the existing placement")]
    pub track_id: TrackId,
    #[schemars(description = "Start position in beats; may accompany an agreeing start_tick")]
    pub start_beat: Option<f32>,
    #[schemars(
        description = "Exact start position in ticks; may accompany an agreeing start_beat"
    )]
    pub start_tick: Option<Tick>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PlacementUpdateInput {
    #[schemars(description = "Pattern ID of the existing placement")]
    pub pattern_id: PatternId,
    #[schemars(description = "Track ID of the existing placement")]
    pub track_id: TrackId,
    #[schemars(
        description = "Existing start position in beats; may accompany an agreeing start_tick"
    )]
    pub start_beat: Option<f32>,
    #[schemars(
        description = "Exact existing start position in ticks; may accompany an agreeing start_beat"
    )]
    pub start_tick: Option<Tick>,
    #[schemars(description = "Move the placement to this track ID")]
    pub new_track_id: Option<TrackId>,
    #[schemars(
        description = "Move the placement to this beat; mutually exclusive with new_start_tick"
    )]
    pub new_start_beat: Option<f32>,
    #[schemars(
        description = "Move the placement to this exact tick; mutually exclusive with new_start_beat"
    )]
    pub new_start_tick: Option<Tick>,
    #[schemars(description = "Replace the placement transpose in semitones (range -127..127)")]
    pub transpose_semitones: Option<f32>,
    #[schemars(description = "Replace the linear placement gain (range 0..2)")]
    pub gain: Option<f32>,
    #[schemars(
        description = "Replace the length override in beats; mutually exclusive with length_ticks and clear_length_override"
    )]
    pub length_beats: Option<f32>,
    #[schemars(
        description = "Replace the exact length override in ticks; mutually exclusive with length_beats and clear_length_override"
    )]
    pub length_ticks: Option<u32>,
    #[serde(default)]
    #[schemars(description = "Remove the existing length override")]
    pub clear_length_override: bool,
    #[schemars(description = "Replace the playback mode: repeat or clip")]
    pub loop_mode: Option<PlacementLoopMode>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdatePlacementsParam {
    #[schemars(description = "Array of partial placement updates")]
    pub updates: Vec<PlacementUpdateInput>,
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
    #[schemars(
        description = "Start in beats; may accompany start_tick when both resolve to the same position"
    )]
    pub start_beat: Option<f32>,
    #[schemars(description = "Exact start tick; may accompany an agreeing start_beat")]
    pub start_tick: Option<Tick>,
    #[schemars(description = "Optional placement transposition in semitones (default 0)")]
    pub transpose_semitones: Option<f32>,
    #[schemars(description = "Optional linear placement gain (default 1)")]
    pub gain: Option<f32>,
    #[schemars(
        description = "Optional placement length in beats; mutually exclusive with length_ticks"
    )]
    pub length_beats: Option<f32>,
    #[schemars(
        description = "Optional exact placement length in ticks; mutually exclusive with length_beats"
    )]
    pub length_ticks: Option<u32>,
    #[serde(default)]
    #[schemars(description = "Playback beyond the source pattern: repeat (default) or clip")]
    pub loop_mode: Option<PlacementLoopMode>,
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
        description = "Parameters as {name: value}. Values can be numbers (440.0), strings for choices ('sawtooth'), or booleans. Parameter names are module-specific and must match exactly — call get_module_type_info(<module_type>) for the valid names/ranges/choices. An unknown name is skipped (not applied) and reported per-result in `errors` with `partial_success: true`; it does NOT fail the call, so check `partial_success` before treating the patch as complete."
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
    pub midi_channel: Option<MidiChannel>,
    #[schemars(description = "Volume (0.0-2.0, optional)")]
    pub volume: Option<Gain>,
    #[schemars(description = "Pan (-1.0 to 1.0, optional)")]
    pub pan: Option<BipolarValue>,
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
    pub midi_channel: Option<MidiChannel>,
    #[schemars(description = "Volume (0.0-2.0, optional)")]
    pub volume: Option<Gain>,
    #[schemars(description = "Pan (-1.0 to 1.0, optional)")]
    pub pan: Option<BipolarValue>,
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
    #[schemars(
        description = "Absolute file path for the project. `.ptz` (recommended) and `.json` are both accepted and preserved; any other extension is normalized to `.ptz`. A project that embeds samples is always written as a `.ptz.zip` bundle regardless of the requested extension. Loading auto-detects the format by content, so the extension never blocks a round-trip. save_project returns the actual path written."
    )]
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
    pub note: MidiNote,
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
        let mut router = Self::discovery_tool_router()
            + Self::instruments_tool_router()
            + Self::analysis_tool_router()
            + Self::sequencer_tool_router()
            + Self::automation_tool_router()
            + Self::mixing_tool_router()
            + Self::project_tool_router()
            + Self::samples_tool_router()
            + Self::audio_input_tool_router()
            + Self::batch_tool_router();
        for name in disabled_tools {
            router.disable_route(*name);
        }
        // rmcp generates each tool's schemas with schemars, which emits a
        // `$ref` into `$defs` for every nested struct (array item types, enums,
        // …). MCP clients that don't resolve `$ref` then render those as
        // `Array<unknown>`, hiding the required fields. Inline the refs once here
        // so every tool's schema is self-describing regardless of client.
        //
        // Output schemas need this just as badly: a `Json<Vec<T>>` tool publishes
        // `{"type":"array","items":{"$ref":"#/$defs/T"}}`, which is exactly the
        // `Array<unknown>` case for a non-resolving client.
        //
        // Only *replace* a schema that the inlining actually rewrote. A schema
        // with no `$defs` comes back unchanged, and re-wrapping it in a fresh
        // `Arc` would undo [`action_output_schema`]'s whole point: that one
        // shared `MutationResult` document — which inlines its own `$defs` when
        // it is built — stays one allocation instead of 112 identical copies.
        for route in router.map.values_mut() {
            if schema_has_defs(route.attr.input_schema.as_ref()) {
                let inlined = inline_schema_refs(route.attr.input_schema.as_ref());
                route.attr.input_schema = std::sync::Arc::new(inlined);
            }
            if let Some(output) = route.attr.output_schema.as_ref()
                && schema_has_defs(output.as_ref())
            {
                let inlined = inline_schema_refs(output.as_ref());
                route.attr.output_schema = Some(std::sync::Arc::new(inlined));
            }
        }
        router
    }
}

/// Recursively inline every local `$ref` (`#/$defs/…` or `#/definitions/…`) in a
/// JSON-Schema value, resolving names against `defs`. `path` holds the
/// definition names currently being expanded so reference cycles terminate: a
/// `$ref` that would revisit a name on the path — or points at an unknown
/// definition — is left intact and sets `retained`, telling the caller to keep
/// `$defs` as a fallback. Sibling keys on a `$ref` node (e.g. `description`) are
/// merged over the resolved definition.
fn inline_refs_value(
    value: &serde_json::Value,
    defs: &serde_json::Map<String, serde_json::Value>,
    path: &mut Vec<String>,
    retained: &mut bool,
) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref")
                && let Some(name) = reference
                    .strip_prefix("#/$defs/")
                    .or_else(|| reference.strip_prefix("#/definitions/"))
            {
                if path.iter().any(|p| p == name) {
                    *retained = true; // cycle — leave the $ref in place
                    return value.clone();
                }
                if let Some(target) = defs.get(name) {
                    path.push(name.to_string());
                    let mut resolved = inline_refs_value(target, defs, path, retained);
                    if let Value::Object(res) = &mut resolved {
                        for (k, v) in map {
                            if k != "$ref" {
                                res.insert(k.clone(), inline_refs_value(v, defs, path, retained));
                            }
                        }
                    }
                    path.pop();
                    return resolved;
                }
                *retained = true; // unknown definition — leave the $ref in place
                return value.clone();
            }
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), inline_refs_value(v, defs, path, retained));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| inline_refs_value(v, defs, path, retained))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Does this schema carry a `$defs` / `definitions` block — i.e. is there
/// anything for [`inline_schema_refs`] to inline? The predicate `build_router`
/// uses to skip a no-op rewrite; [`inline_schema_refs`] short-circuits on the
/// same condition, so the two must agree.
fn schema_has_defs(schema: &JsonObject) -> bool {
    schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(serde_json::Value::as_object)
        .is_some_and(|defs| !defs.is_empty())
}

/// Inline a schema's local `$ref`s and drop the now-redundant `$defs`, so
/// array/object item schemas are concrete for MCP clients that do not resolve
/// `$ref`. If any `$ref` could not be inlined (a reference cycle or a dangling
/// name), `$defs` is retained as a fallback so the schema stays valid.
fn inline_schema_refs(schema: &JsonObject) -> JsonObject {
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    if defs.is_empty() {
        return schema.clone();
    }
    let root = serde_json::Value::Object(schema.clone());
    let mut path: Vec<String> = Vec::new();
    let mut retained = false;
    let resolved = inline_refs_value(&root, &defs, &mut path, &mut retained);
    let mut out = match resolved {
        serde_json::Value::Object(map) => map,
        _ => return schema.clone(),
    };
    if !retained {
        out.remove("$defs");
        out.remove("definitions");
    }
    out
}

/// One dispatched operation's result, as its handler stated it.
///
/// The batch path used to speak `Result<String, String>`: every payload rendered
/// to text, and the verdict recovered afterwards by inspecting that text. This
/// carries the three things a batch reply actually needs, from the one place that
/// knows them.
pub(crate) struct DispatchedOp {
    /// Success, partial, or failure — never re-derived downstream.
    outcome: ToolOutcome,
    /// The payload as data, when the tool has one.
    structured: Option<serde_json::Value>,
    /// The text, when text is what the op has to say: a mutator's sentence, a
    /// document, or an error. `None` for a typed reader, whose payload is
    /// `structured` — serializing it into a message too would send it twice, which
    /// is the cost this branch declines to pay elsewhere (see `PROSE_TOOLS`).
    message: Option<String>,
}

impl DispatchedOp {
    /// A dispatch-level refusal — unknown tool, unparseable params, a panic. The
    /// operation never reached its handler.
    fn rejected(message: String) -> Self {
        Self {
            outcome: ToolOutcome::Failure,
            structured: None,
            message: Some(message),
        }
    }

    /// A handler's `Err`: it ran and refused.
    fn failed(message: String) -> Self {
        Self::rejected(message)
    }

    /// A reader's payload: the same value a direct call puts in
    /// `structuredContent`, and no text, because the payload *is* the answer.
    fn payload(structured: serde_json::Value) -> Self {
        Self {
            // A reader either produced its payload or returned `Err`; only a
            // mutator can land partially, and `reply_outcome` reads that from the
            // payload's own per-item results.
            outcome: reply_outcome(Some(&structured), false),
            structured: Some(structured),
            message: None,
        }
    }

    /// A document reader's reply: text, and nothing structured to carry.
    fn text(message: String) -> Self {
        Self {
            outcome: ToolOutcome::Success,
            structured: None,
            message: Some(message),
        }
    }

    /// A mutating tool's reply, which already states its own verdict.
    fn from_reply(reply: &CallToolResult) -> Self {
        let structured = reply.structured_content.clone();
        let text = reply_text(reply);
        // A mutator's sentence earns its place beside the items — it is the summary
        // a human reads, not the payload restated. A `typed_reply`'s text *is* the
        // payload restated, though: those five tools put `to_json(payload)` in
        // `content`, which is right for a direct call (every typed tool does, and a
        // text-only client needs it) and pure duplication inside a batch item that
        // already carries the value. Told apart by asking whether the text parses
        // back to the same JSON, so no marker has to be kept in step.
        let restates_payload = structured.as_ref().is_some_and(|value| {
            serde_json::from_str::<serde_json::Value>(&text).ok() == Some(value.clone())
        });
        Self {
            // What the handler said, when it said anything; the payload predicate
            // is the fallback for a reply built without our helpers.
            outcome: stated_outcome(reply).unwrap_or_else(|| {
                reply_outcome(structured.as_ref(), reply.is_error.unwrap_or(false))
            }),
            structured,
            message: (!restates_payload).then_some(text),
        }
    }

    /// What to log for this op — and what the warn-vs-debug split classifies.
    ///
    /// A typed tool has no `message`: its answer is data. When such a payload is
    /// the one recording the failure (an `import_sample` whose every path was bad,
    /// a `set_parameter` whose every id was unknown), reading `message` alone
    /// logged `error = ""` and the activity console lost the reason entirely — and
    /// a bridge "not found" reported that way missed the `warn` it had earned.
    /// The payload's own summary sentence is that reason; a payload without one
    /// falls back to its serialized form, capped so a large reader result cannot
    /// flood the bounded ring.
    fn log_detail(&self) -> String {
        if let Some(message) = &self.message {
            return message.clone();
        }
        let Some(value) = &self.structured else {
            return String::new();
        };
        value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map_or_else(
                || truncate_chars(&to_json(value), LOG_DETAIL_MAX),
                str::to_string,
            )
    }

    /// This op as its entry in the batch report.
    fn into_item(self, index: usize, tool: String) -> crate::types::BatchExecItemResult {
        crate::types::BatchExecItemResult {
            index,
            tool,
            status: self.outcome,
            structured: self.structured,
            message: self.message,
        }
    }

    /// What this op amounted to.
    ///
    /// The batch decides for itself what to do with `Partial`; at the tool layer
    /// it stays distinct from `Failure`, because a reply that half-applied is not
    /// an error result.
    const fn outcome(&self) -> ToolOutcome {
        self.outcome
    }

    /// Whether this op failed outright — nothing landed.
    fn is_failure(&self) -> bool {
        self.outcome == ToolOutcome::Failure
    }
}

/// Macro to generate dispatch arms for `batch_execute`.
///
/// Each arm deserializes the JSON params into the appropriate type and calls
/// the corresponding tool method, unwrapping the `Parameters` wrapper. The three
/// lists mirror the three reply shapes a handler can have.
macro_rules! dispatch_tools {
    ($self:expr, $tool:expr, $params:expr, $validate_only:expr, [
        $( $name:literal => $method:ident ( $ptype:ty ) ),* $(,)?
    ], typed: [
        $( $tname:literal => $tmethod:ident ( $tptype:ty ) ),* $(,)?
    ], action: [
        $( $aname:literal => $amethod:ident ( $aptype:ty ) ),* $(,)?
    ]) => {
        match $tool {
            // Tools returning `Result<String, String>` — the two whose whole
            // result is one document, so there is no structure to expose.
            $(
                $name => {
                    match serde_json::from_value::<$ptype>($params) {
                        // In validate-only (dry_run) mode the params parsed, so
                        // report the op as valid without invoking the handler —
                        // no state is touched.
                        Ok(p) => if $validate_only {
                            DispatchedOp::text(format!("dry_run OK: '{}' params valid", $name))
                        } else {
                            match $self.$method(Parameters(p)).await {
                                Ok(text) => DispatchedOp::text(text),
                                Err(e) => DispatchedOp::failed(e),
                            }
                        },
                        Err(e) => DispatchedOp::rejected(
                            format!("Error: invalid params for '{}': {}", $name, e)
                        ),
                    }
                }
            )*
            // Tools returning `Result<Json<T>, String>`: the payload crosses the
            // batch boundary as itself now, with the serialized form beside it for
            // a caller that wants text.
            $(
                $tname => {
                    match serde_json::from_value::<$tptype>($params) {
                        Ok(p) => if $validate_only {
                            DispatchedOp::text(format!("dry_run OK: '{}' params valid", $tname))
                        } else {
                            match $self.$tmethod(Parameters(p)).await {
                                Ok(payload) => match serde_json::to_value(&payload.0) {
                                    Ok(value) => DispatchedOp::payload(value),
                                    // A payload that cannot become a `Value` also
                                    // cannot become JSON text; report it rather
                                    // than losing the op silently.
                                    Err(e) => DispatchedOp::failed(format!(
                                        "Error: could not serialize '{}' result: {}", $tname, e
                                    )),
                                },
                                Err(e) => DispatchedOp::failed(e),
                            }
                        },
                        Err(e) => DispatchedOp::rejected(
                            format!("Error: invalid params for '{}': {}", $tname, e)
                        ),
                    }
                }
            )*
            // Tools returning `CallToolResult` — prose in `content`, a
            // `MutationResult` beside it. Both halves cross the boundary, so a
            // batched mutation reports its per-item results like a direct call.
            $(
                $aname => {
                    match serde_json::from_value::<$aptype>($params) {
                        Ok(p) => if $validate_only {
                            DispatchedOp::text(format!("dry_run OK: '{}' params valid", $aname))
                        } else {
                            DispatchedOp::from_reply(&$self.$amethod(Parameters(p)).await)
                        },
                        Err(e) => DispatchedOp::rejected(
                            format!("Error: invalid params for '{}': {}", $aname, e)
                        ),
                    }
                }
            )*
            // Registered with the router, deliberately absent from the lists
            // above: its reply is a base64 `audio/wav` content block, and a batch
            // renders each op to one payload value. Answered by name rather than
            // falling through to "unknown tool", which is untrue of a tool the
            // catalog lists and leaves the caller guessing at the fix.
            "preview_note" => DispatchedOp::rejected(
                "Error: 'preview_note' cannot run inside batch_execute — it answers audio, \
                 which a batch result cannot carry. Call it directly, or use 'analyze_note' \
                 for the same render as measurements."
                    .to_string(),
            ),
            _ => {
                // Every list contributes, or a typo of a tool's name would lose
                // its suggestion the moment that tool was converted.
                let known: &[&str] = &[ $( $name, )* $( $tname, )* $( $aname, )* ];
                let hint = synth_core::suggest::did_you_mean(
                    $tool,
                    known.iter().copied(),
                    synth_core::suggest::DEFAULT_MAX_HINTS,
                );
                DispatchedOp::rejected(format!("Error: unknown tool '{}'{}", $tool, hint))
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

/// Did this `Err` come from the dispatch table itself (unknown tool, params that
/// don't deserialize, a panic) rather than from a tool that ran and reported a
/// failure? A tool returning `Result<Json<T>, String>` answers through the same
/// `Err` channel `dispatch_tools!` uses for a rejection, so only the wording —
/// which the macro and the panic guard own — tells the two apart.
fn is_dispatch_rejection(msg: &str) -> bool {
    msg.starts_with("Error: unknown tool '")
        || msg.starts_with("Error: invalid params for '")
        || msg.starts_with("Error: tool '")
}

/// Character cap for a param summary line.
const SUMMARY_MAX: usize = 60;

/// Character cap for the log line describing a failed op, when the only thing
/// there is to describe it with is a serialized payload. Generous enough to carry
/// a per-item error list, short of a whole reader result.
const LOG_DETAIL_MAX: usize = 400;

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
    /// Dispatch a tool call by name with JSON params.
    ///
    /// Used by `batch_execute`, which needs each op's payload *and* its verdict
    /// for the per-op result and the stop-on-error / rollback accounting. Both
    /// come from the handler by way of [`DispatchedOp`]; nothing here re-reads a
    /// serialized result to work out what happened.
    async fn dispatch_tool(
        &self,
        tool: &str,
        params: serde_json::Value,
        validate_only: bool,
    ) -> DispatchedOp {
        let started = Instant::now();
        // Summarize the params before they're moved into the dispatch, so the
        // failure logs below show what the call did — e.g. `12 ops`,
        // `flt-1.cutoff=800` — without dumping full JSON.
        let param_summary = summarize_value(&params);
        // Isolate a panicking sub-op (e.g. a panic inside a `block_in_place`
        // bridge call) so one bad op surfaces as an error instead of killing
        // the tokio worker and dropping the session.
        let op = match run_catching_panic(
            self.session_id,
            tool,
            self.dispatch_tool_inner(tool, params, validate_only),
        )
        .await
        {
            Ok(op) => op,
            // Named and marked as a panic, not handed on bare: `is_dispatch_rejection`
            // keys the warn-vs-debug log split off this prefix, and the batch item's
            // `message` is all a caller gets to tell "the op panicked" from "the op
            // refused an argument".
            Err(panic_message) => {
                DispatchedOp::rejected(format!("Error: tool '{tool}' panicked: {panic_message}"))
            }
        };
        let elapsed_ms = started.elapsed().as_millis();
        // Counted whatever the verdict: a failed mutation may still have written
        // some of its items, and a rollback comparing sequences wants to err
        // towards "state moved".
        if !validate_only {
            self.note_call(tool);
        }
        match op.outcome {
            // A dispatch rejection (unknown tool, invalid params, panic) is always
            // worth a warn. A tool's *own* failure gets the warn/debug split:
            // `is_bridge_error` still warns (a missing pattern or a dead command
            // channel is worth seeing), but a plain validation refusal —
            // `"tolerance must be in 0..=1"`, an out-of-range velocity — drops to
            // debug rather than logging as if the dispatch had refused the call,
            // which in a large batch would flood the bounded activity-console ring.
            ToolOutcome::Failure => {
                let message = &op.log_detail();
                if is_dispatch_rejection(message) {
                    tracing::warn!(
                        session_id = self.session_id,
                        tool,
                        elapsed_ms,
                        error = %message,
                        "MCP batch dispatch rejected tool call"
                    );
                } else if is_bridge_error(message) {
                    tracing::warn!(
                        session_id = self.session_id,
                        tool,
                        elapsed_ms,
                        error = %message,
                        "MCP tool returned bridge/engine-state error"
                    );
                } else {
                    tracing::debug!(
                        session_id = self.session_id,
                        tool,
                        elapsed_ms,
                        error = %message,
                        "MCP tool returned validation error"
                    );
                }
            }
            // Some items landed and some did not. Logged at debug with the
            // sentence, which names the failures: this is not an error, but a
            // silently-partial write is the thing a caller most wants to find
            // afterwards in the console.
            ToolOutcome::Partial => {
                tracing::debug!(
                    session_id = self.session_id,
                    tool,
                    elapsed_ms,
                    params = %param_summary,
                    detail = %op.log_detail(),
                    "MCP batch sub-op partially succeeded"
                );
            }
            // A batch sub-op success. The whole `batch_execute` call is already
            // logged at info by `call_tool`, so keep the per-op line at trace: the
            // activity-console capture layer only keeps `debug`+, so a 1000-op
            // batch's successes don't flood (and evict the visible history from)
            // the bounded in-memory ring. Raise to trace on stderr with
            // `RUST_LOG=synth_mcp=trace` when needed.
            ToolOutcome::Success => {
                tracing::trace!(
                    session_id = self.session_id,
                    tool,
                    elapsed_ms,
                    params = %param_summary,
                    "MCP batch sub-op succeeded"
                );
            }
        }
        op
    }

    /// Whether `tool` is a read-only tool, from the annotation it publishes.
    ///
    /// Derived from the router rather than a list kept by hand: `read_only_hint`
    /// is already asserted against the read-only families by
    /// `tool_annotations_tests`, so there is one place to be wrong instead of two.
    /// An unknown name counts as mutating — a dispatch rejection changes nothing,
    /// and over-counting a mutation only makes a rollback more cautious.
    pub(crate) fn is_read_only_tool(&self, tool: &str) -> bool {
        self.tool_router
            .map
            .get(tool)
            .and_then(|route| route.attr.annotations.as_ref())
            .and_then(|annotations| annotations.read_only_hint)
            .unwrap_or(false)
    }

    /// Count a call against [`SynthBridge::mutation_seq`] unless it only reads.
    ///
    /// One chokepoint for both entry points: a direct `call_tool` and a batch
    /// sub-op. What it buys is that a rollback can tell its own writes from
    /// another client's — see `batch_execute`.
    fn note_call(&self, tool: &str) {
        if !self.is_read_only_tool(tool) {
            self.bridge.note_mutation();
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
        let op = self.dispatch_tool_inner(tool, params, false).await;
        // A typed op carries its answer as data now, so render it here rather than
        // on the wire: this is the *test* seam, and every caller of it parses JSON
        // out of the string.
        let text = op
            .message
            .clone()
            .unwrap_or_else(|| op.structured.as_ref().map_or_else(String::new, to_json));
        if op.is_failure() { Err(text) } else { Ok(text) }
    }

    /// Run `batch_execute` exactly as a client would, from a JSON param value —
    /// for tests of the dry_run / rollback orchestration.
    #[doc(hidden)]
    pub async fn batch_execute_for_test(&self, params: serde_json::Value) -> String {
        match serde_json::from_value::<BatchExecuteParam>(params) {
            Ok(p) => match self.batch_execute(Parameters(p)).await {
                Ok(report) => to_json(&report.0),
                Err(e) => e,
            },
            Err(e) => format!("Error: invalid batch params: {e}"),
        }
    }

    async fn dispatch_tool_inner(
        &self,
        tool: &str,
        params: serde_json::Value,
        validate_only: bool,
    ) -> DispatchedOp {
        dispatch_tools!(self, tool, params, validate_only, [
            // Still prose (`-> Result<String, String>`) — these two are exactly
            // `output_schema_tests::PROSE_TOOLS`, which holds the whole prose set.
            // The section headers that used to group this list left with their
            // tools for `typed:` / `action:` below; only the ones that still
            // have an entry are kept.

            // Module types & discovery
            "get_yams_reference" => get_yams_reference(NoParams),
            "get_project_schema" => get_project_schema(NoParams),

            // Parameters

            // Bake a pattern's bound note graph (or per-note ornaments) into
            // plain notes.

            // Note Grid (pooled note-processing graphs)

            // Mod Grid

            // Arrangement
        ], typed: [
            // Tools returning `Result<Json<T>, String>` — see TODO 6.7. Being
            // listed here is what keeps them reachable from `batch_execute`;
            // the dispatch-coverage test fails if a router-registered tool is
            // in neither list.
            "get_module_type_info" => get_module_type_info(GetModuleTypeInfoParam),
            "get_master_volume" => get_master_volume(NoParams),
            "list_module_types" => list_module_types(NoParams),
            "search_modules" => search_modules(SearchModulesParam),
            "get_note_graph" => get_note_graph(GetNoteGraphParam),
            "get_mod_graph" => get_mod_graph(GetModGraphParam),
            "analyze_note" => analyze_note(AnalyzeNoteParam),
            "create_note_graph" => create_note_graph(CreateNoteGraphParam),
            "duplicate_note_graph" => duplicate_note_graph(NoteGraphIdParam),
            "create_mod_graph" => create_mod_graph(CreateModGraphParam),
            "duplicate_mod_graph" => duplicate_mod_graph(DuplicateModGraphParam),
            "list_port_types" => list_port_types(NoParams),
            "set_mseg_segments" => set_mseg_segments(SetMsegSegmentsParam),
            "update_placement" => update_placement(UpdatePlacementsParam),
            "create_instrument" => create_instrument(CreateInstrumentParam),
            "import_sample" => import_sample(ImportSampleParam),
            "duplicate_sample" => duplicate_sample(SampleIdsParam),
            "validate_instrument_audio" => validate_instrument_audio(ValidateInstrumentAudioParam),
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
            "auto_gain_stage" => auto_gain_stage(AutoGainStageParam),
            "analyze_section" => analyze_section(AnalyzeSectionParam),
            "analyze_masking_matrix" => analyze_masking_matrix(AnalyzeMaskingMatrixParam),
            "analyze_instrument_range" => analyze_instrument_range(AnalyzeInstrumentRangeParam),
            "analyze_velocity_response" => analyze_velocity_response(AnalyzeVelocityResponseParam),
            "analyze_tension_curve" => analyze_tension_curve(AnalyzeTensionCurveParam),
            "suggest_music_fixes" => suggest_music_fixes(SuggestMusicFixesParam),
            "compare_mix_before_after" => compare_mix_before_after(CompareMixBeforeAfterParam),
            "generate_chord" => generate_chord(GenerateChordParam),
            "create_chord_progression_pattern" => create_chord_progression_pattern(CreateChordProgressionPatternParam),
            "transpose_notes" => transpose_notes(TransposeNotesParam),
            "quantize_notes_to_scale" => quantize_notes_to_scale(QuantizeNotesToScaleParam),
            "add_automation_points" => add_automation_points(AddAutomationPointsParam),
            "get_instrument_automation_targets" => get_instrument_automation_targets(InstrumentIdParam),
            "get_automation_points" => get_automation_points(GetAutomationPointsParam),
            "remove_automation_points" => remove_automation_points(RemoveAutomationPointsParam),
            "simplify_automation" => simplify_automation(SimplifyAutomationParam),
            "get_automation_summary" => get_automation_summary(GetAutomationSummaryParam),
            "get_module_info" => get_module_info(ModuleParam),
            "lint_project" => lint_project(NoParams),
            "check_connection" => check_connection(CheckConnectionParam),
            "insert_module_between" => insert_module_between(InsertModuleBetweenParam),
            "set_parameter" => set_parameter(SetParametersParam),
            "list_samples" => list_samples(ListSamplesParam),
            "get_sampler_state" => get_sampler_state(SamplerModuleParam),
            "add_note" => add_note(AddNotesParam),
            "update_note" => update_note(UpdateNotesParam),
            "replace_notes" => replace_notes(ReplaceNotesParam),
            "create_pattern" => create_pattern(CreatePatternsParam),
            "create_track" => create_track(CreateTracksParam),
            "place_pattern" => place_pattern(PlacePatternsParam),
            "analyze_harmony" => analyze_harmony(AnalyzeHarmonyParam),
            "analyze_pattern" => analyze_pattern(AnalyzePatternParam),
            "analyze_arrangement" => analyze_arrangement(AnalyzeArrangementParam),
            "analyze_form_map" => analyze_form_map(AnalyzeFormMapParam),
            "analyze_hook_strength" => analyze_hook_strength(AnalyzeHookStrengthParam),
            "quantize_notes_to_grid" => quantize_notes_to_grid(QuantizeNotesToGridParam),
            "analyze_drum_groove" => analyze_drum_groove(AnalyzeDrumGrooveParam),
            "analyze_bass_drum_lock" => analyze_bass_drum_lock(AnalyzeBassDrumLockParam),
            "analyze_harmonic_function" => analyze_harmonic_function(AnalyzeHarmonicFunctionParam),
            "list_input_devices" => list_input_devices(NoParams),
            "get_input_state" => get_input_state(NoParams),
            "stop_recording" => stop_recording(StopRecordingParam),
            "find_motifs" => find_motifs(FindMotifsParam),
            "list_example_patches" => list_example_patches(NoParams),
            "get_ui_snapshot" => get_ui_snapshot(InstrumentIdParam),
            "list_return_busses" => list_return_busses(NoParams),
            "list_master_effects" => list_master_effects(NoParams),
            "optimize_project" => optimize_project(NoParams),
            "get_sample_info" => get_sample_info(SampleIdParam),
            "list_instruments" => list_instruments(NoParams),
            "get_instrument_profiles" => get_instrument_profiles(NoParams),
            "get_instrument_info" => get_instrument_info(InstrumentIdParam),
            "list_modules" => list_modules(InstrumentIdParam),
            "get_connections" => get_connections(InstrumentIdParam),
            "get_mod_matrix_routings" => get_mod_matrix_routings(InstrumentIdParam),
            "get_parameter" => get_parameter(GetParameterParam),
            "get_engine_status" => get_engine_status(NoParams),
            "get_version" => get_version(NoParams),
            "get_graph_diagnostics" => get_graph_diagnostics(InstrumentIdParam),
            "list_automation_lanes" => list_automation_lanes(PatternIdParam),
            "get_song_info" => get_song_info(NoParams),
            "get_tempo_map" => get_tempo_map(NoParams),
            "list_patterns" => list_patterns(NoParams),
            "list_notes" => list_notes(PatternIdParam),
            "list_note_graphs" => list_note_graphs(NoParams),
            "list_mod_graphs" => list_mod_graphs(NoParams),
            "list_mod_targets" => list_mod_targets(ListModTargetsParam),
            "list_tracks" => list_tracks(NoParams),
            "list_arrangement" => list_arrangement(NoParams),
        ], action: [
            // Typed payloads whose own type says "incomplete" — they answer a
            // `CallToolResult` so the verdict travels with the reply.
            "build_instrument" => build_instrument(BuildInstrumentsParam),
            "apply_example_patch" => apply_example_patch(ApplyExamplePatchParam),
            "rebuild_instrument_preserve_automation" =>
                rebuild_instrument_preserve_automation(RebuildInstrumentParam),
            "freeze_pattern" => freeze_pattern(PatternIdParam),
            "set_song" => set_song(SetSongParam),
            // Tools answering with prose plus an `ActionResult` beside it.
            "set_input_device" => set_input_device(SetInputDeviceParam),
            "start_monitoring" => start_monitoring(NoParams),
            "stop_monitoring" => stop_monitoring(NoParams),
            "start_recording" => start_recording(NoParams),
            "clear_automation_lane" => clear_automation_lane(ClearAutomationLaneParam),
            "scale_automation_lane" => scale_automation_lane(ScaleAutomationLaneParam),
            "offset_automation_lane" => offset_automation_lane(OffsetAutomationLaneParam),
            "copy_automation_lane" => copy_automation_lane(CopyAutomationLaneParam),
            "set_mod_matrix_script" => set_mod_matrix_script(SetModMatrixScriptParam),
            "note_on" => note_on(NoteOnParam),
            "note_off" => note_off(NoteOffParam),
            "load_example_patch" => load_example_patch(LoadExamplePatchParam),
            "auto_layout" => auto_layout(NoParams),
            "add_module" => add_module(AddModulesParam),
            "remove_module" => remove_module(RemoveModulesParam),
            "clear_graph" => clear_graph(InstrumentIdParam),
            "delete_instrument" => delete_instrument(DeleteInstrumentsParam),
            "rename_instrument" => rename_instrument(RenameInstrumentParam),
            "set_instrument_description" => set_instrument_description(SetInstrumentDescriptionParam),
            "set_instrument_color" => set_instrument_color(SetInstrumentColorParam),
            "set_patch_color" => set_patch_color(SetPatchColorParam),
            "set_patch_description" => set_patch_description(SetPatchDescriptionParam),
            "set_module_description" => set_module_description(SetModuleDescriptionParam),
            "set_sample_description" => set_sample_description(SetSampleDescriptionParam),
            "set_sidechain_source" => set_sidechain_source(SetSidechainSourceParam),
            "set_instrument_mixer" => set_instrument_mixer(SetInstrumentMixerParam),
            "set_allocator_config" => set_allocator_config(SetAllocatorConfigParam),
            "set_instrument_midi_channel" => set_instrument_midi_channel(SetInstrumentMidiChannelParam),
            "set_instrument_category" => set_instrument_category(SetInstrumentCategoryParam),
            "disconnect" => disconnect(ConnectMultipleParam),
            "set_track_description" => set_track_description(SetTrackDescriptionParam),
            "set_track_color" => set_track_color(SetTrackColorParam),
            "set_track_mixer" => set_track_mixer(SetTrackMixerParam),
            "set_track_instrument" => set_track_instrument(SetTrackInstrumentParam),
            "rename_track" => rename_track(RenameTrackParam),
            "delete_track" => delete_track(DeleteTracksParam),
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
            "set_master_volume" => set_master_volume(SetMasterVolumeParam),
            "add_master_effect" => add_master_effect(AddMasterEffectsParam),
            "remove_master_effect" => remove_master_effect(RemoveMasterEffectsParam),
            "set_master_effect_parameter" => set_master_effect_parameter(SetMasterEffectParameterParam),
            "set_master_effect_enabled" => set_master_effect_enabled(SetMasterEffectEnabledParam),
            "reorder_master_effect" => reorder_master_effect(ReorderMasterEffectParam),
            "new_project" => new_project(NoParams),
            "save_project" => save_project(ProjectPathParam),
            "save_patch" => save_patch(SavePatchParam),
            "load_project" => load_project(ProjectPathParam),
            "delete_sample" => delete_sample(DeleteSamplesParam),
            "rename_sample" => rename_sample(RenameSampleParam),
            "set_sample_root_note" => set_sample_root_note(SetSampleRootNoteParam),
            "normalize_sample" => normalize_sample(SampleIdsParam),
            "reverse_sample" => reverse_sample(SampleIdsParam),
            "trim_sample_silence" => trim_sample_silence(SampleIdsParam),
            "set_sample_loop" => set_sample_loop(SetSampleLoopParam),
            "set_sample_crop" => set_sample_crop(SetSampleCropParam),
            "export_sample" => export_sample(ExportSampleParam),
            "assign_sample_to_module" => assign_sample_to_module(AssignSampleParam),
            "set_sampler_parameter" => set_sampler_parameter(SetSamplerParameterParam),
            "set_song_description" => set_song_description(SetSongDescriptionParam),
            "set_pattern_description" => set_pattern_description(SetPatternDescriptionParam),
            "set_song_tempo" => set_song_tempo(SetSongTempoParam),
            "set_tempo_at" => set_tempo_at(SetTempoAtParam),
            "remove_tempo_at" => remove_tempo_at(RemoveTempoAtParam),
            "set_transport_loop" => set_transport_loop(SetTransportLoopParam),
            "clear_transport_loop" => clear_transport_loop(NoParams),
            "set_song_name" => set_song_name(SetSongNameParam),
            "delete_pattern" => delete_pattern(DeletePatternsParam),
            "remove_note" => remove_note(RemoveNotesParam),
            "set_note_ornament" => set_note_ornament(SetNoteOrnamentParam),
            "set_note_graph_metadata" => set_note_graph_metadata(SetNoteGraphMetadataParam),
            "delete_note_graph" => delete_note_graph(DeleteNoteGraphParam),
            "add_note_graph_module" => add_note_graph_module(AddNoteGraphModuleParam),
            "set_note_graph_module" => set_note_graph_module(SetNoteGraphModuleParam),
            "set_note_graph_script" => set_note_graph_script(SetNoteGraphScriptParam),
            "remove_note_graph_module" => remove_note_graph_module(RemoveNoteGraphModuleParam),
            "connect_note_graph" => connect_note_graph(ConnectNoteGraphParam),
            "set_pattern_note_graph" => set_pattern_note_graph(SetPatternNoteGraphParam),
            "set_note_note_graph" => set_note_note_graph(SetNoteNoteGraphParam),
            "set_mod_graph_metadata" => set_mod_graph_metadata(SetModGraphMetadataParam),
            "delete_mod_graph" => delete_mod_graph(DeleteModGraphParam),
            "set_mod_graph_scope" => set_mod_graph_scope(SetModGraphScopeParam),
            "assign_mod_graph" => assign_mod_graph(AssignModGraphParam),
            "add_mod_graph_node" => add_mod_graph_node(AddModGraphNodeParam),
            "remove_mod_graph_node" => remove_mod_graph_node(RemoveModGraphNodeParam),
            "connect_mod_graph" => connect_mod_graph(ConnectModGraphParam),
            "disconnect_mod_graph" => disconnect_mod_graph(DisconnectModGraphParam),
            "set_mod_graph_node" => set_mod_graph_node(SetModGraphNodeParam),
            "remove_placement" => remove_placement(RemovePlacementsParam),
            "clear_pattern" => clear_pattern(ClearPatternParam),
            "rename_pattern" => rename_pattern(RenamePatternParam),
            "set_pattern_length" => set_pattern_length(SetPatternLengthParam),
            "duplicate_pattern" => duplicate_pattern(DuplicatePatternParam),
            "set_song_author" => set_song_author(SetSongAuthorParam),
            "set_song_time_signature" => set_song_time_signature(SetSongTimeSignatureParam),
            "seq_play" => seq_play(NoParams),
            "seq_stop" => seq_stop(NoParams),
            "seq_seek" => seq_seek(SeqSeekParam),
            "connect" => connect(ConnectMultipleParam),
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

impl SynthMcpServer {
    /// The running application's version, for the `initialize` handshake.
    ///
    /// Deliberately *not* `env!("CARGO_PKG_VERSION")`: that expands to
    /// `synth_mcp`'s own crate version (`0.1.0`), not the app's, so clients were
    /// told the wrong version on every connect. The bridge is the same source
    /// `get_version` reports from, so the two can no longer disagree.
    fn app_version(&self) -> String {
        self.bridge
            .get_version()
            .map_or_else(|_| "unknown".to_owned(), |info| info.version)
    }

    /// One-line summary of what this build can synthesize, counted from the live
    /// descriptor catalog rather than written out by hand — the previous fixed
    /// string claimed "35 voice modules, 21 effects" long after the real counts
    /// had moved on. Visualizer types are excluded: `add_module` rejects them
    /// over MCP, so they are not part of what a client can build with.
    fn describe_catalog(&self) -> String {
        let Ok(types) = self.bridge.list_module_types_brief() else {
            return "Modular synthesizer with MCP integration".to_owned();
        };
        let voice = types.iter().filter(|t| t.category == "voice").count();
        let effects = types.iter().filter(|t| t.category == "effect").count();
        format!(
            "Modular synthesizer with {voice} voice modules, {effects} effects, \
             and MCP integration"
        )
    }
}

// NOTE: this attribute must stay glued to the `impl ServerHandler` block below —
// it generates the tool routing (`list_tools`, and the `call_tool` our override
// delegates to). Slipping any other item between the two compiles cleanly but
// silently leaves `tools/list` empty.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for SynthMcpServer {
    /// Override the macro-generated `list_tools` to advertise a cache lifetime.
    ///
    /// `#[tool_handler]` fills the SEP-2549 hints as `ttlMs: 0` +
    /// `cacheScope: public` for clients on `2026-07-28` — "immediately stale, and
    /// anyone may serve it". Both halves are wrong here. The catalog is built once
    /// by `build_router` and cannot change while the process runs, and at
    /// 219 tools it is the largest thing this server ever sends; telling a client
    /// not to keep it means re-sending it on every reconnect.
    ///
    /// `Private` rather than `Public` because the list is *this instance's*: the
    /// `disabled_tools` argument means two Pertylizer processes can publish
    /// different catalogs, and a shared intermediary must not serve one to the
    /// other. The TTL is bounded for the same reason — a restart with a different
    /// tool set converges instead of being cached forever.
    ///
    /// The macro only generates a method the impl does not already define
    /// (`has_method` in `rmcp-macros`), which is what lets this and `call_tool`
    /// below coexist with it.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, ErrorData> {
        // Absent for older revisions, which have no field for it.
        let cacheable = context
            .protocol_version()
            .is_some_and(|version| version >= rmcp::model::ProtocolVersion::V_2026_07_28);
        Ok(rmcp::model::ListToolsResult {
            result_type: Some(rmcp::model::ResultType::COMPLETE),
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
            ttl_ms: cacheable.then_some(TOOL_CATALOG_TTL_MS),
            cache_scope: cacheable.then_some(rmcp::model::CacheScope::Private),
        })
    }

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
    ) -> Result<CallToolResponse, ErrorData> {
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
        // `batch_execute` counts its own sub-ops in `dispatch_tool`, so counting it
        // again here would make every batch look like one extra external mutation
        // to a *concurrent* rollback batch.
        if tool_name != "batch_execute" {
            self.note_call(&tool_name);
        }
        let elapsed_ms = started.elapsed().as_millis();

        // `call_tool` is the one choke point every top-level tool call passes
        // through (single calls don't go through `dispatch_tool` — that's the
        // batch sub-op path). Log each call at info so it lands in the console
        // at the default level; a tool-reported failure (in-band `Error:` text
        // or the `is_error` flag) is demoted to warn.
        match outcome {
            // Only a completed call carries a result to inspect. The other
            // `CallToolResponse` variants (`InputRequired`, `Task`) mean the call
            // has not produced an outcome yet, so there is nothing to judge as
            // failed — pass them through untouched.
            Ok(Ok(CallToolResponse::Complete(mut result))) => {
                // No inspection of the reply's *text*: `mutation_reply` stamped
                // `is_error` from the items, and a typed tool's `Err` is rendered
                // as `is_error` by rmcp.
                //
                // A typed tool that answers `Ok` with a payload recording a total
                // failure is the one case the flag cannot already be right: rmcp
                // builds those through `CallToolResult::structured`, which
                // hardcodes `is_error: Some(false)`. So the payload is asked —
                // by its own type's predicate, not by sniffing its text.
                let failed = result.is_error.unwrap_or(false)
                    || reply_outcome(result.structured_content.as_ref(), false)
                        == ToolOutcome::Failure;
                if failed {
                    result.is_error = Some(true);
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
                Ok(CallToolResponse::Complete(result))
            }
            Ok(Ok(response)) => {
                tracing::info!(
                    target: "synth_mcp::call",
                    session_id = self.session_id,
                    tool = %tool_name,
                    elapsed_ms,
                    params = %param_summary,
                    "MCP tool call did not complete in one round trip"
                );
                Ok(response)
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
                .enable_completions()
                .build(),
        )
        .with_server_info(
            Implementation::new("pertylizer", self.app_version())
                .with_title("Pertylizer")
                .with_description(self.describe_catalog())
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
             - `list_module_types` — compact catalog: type_key, name, category, gui_only for \
               every type. Cheap; start here to pick a `type_key`.\n\
             - `get_module_type_info` — get ports, parameters (with ranges/units/choices), and \
               signal flow hints for a single module type by key (e.g. 'osc', 'flt').\n\
             - `search_modules` — filter modules by category ('voice'/'effect'), port signal \
               type ('audio'/'control'/'gate'/'midi'), or text query. Use this to find modules \
               with specific capabilities.\n\
             - `list_port_types` — reference of all signal types with descriptions, value ranges, \
               and compatibility rules.\n\
             - `check_connection` — validate a proposed connection before making it. Reports \
               port direction errors, signal type incompatibilities, and lists available ports.\n\n\
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
                    format!("{MODULE_TYPE_URI_PREFIX}{}", info.type_key),
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
                let slug = patch_slug(&patch.name);
                let mut r = Resource::new(format!("{PATCH_URI_PREFIX}{slug}"), patch.name.clone());
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
            ResourceTemplate::new(MODULE_TYPE_URI_TEMPLATE, "Module Type")
                .with_description("Detailed info about a synth module type (ports, parameters)")
                .with_mime_type("application/json"),
            ResourceTemplate::new(PATCH_URI_TEMPLATE, "Example Patch")
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
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = &request.uri;

        if let Some(type_key) = uri.strip_prefix(MODULE_TYPE_URI_PREFIX) {
            // The brief listing decides existence and one type is then built.
            // The full listing would build ports, parameters and algorithm JSON
            // for all ~75 types just to return one of them — and it is also the
            // listing `complete` does *not* use, so sourcing existence from the
            // brief one keeps reader and completion over the same set.
            //
            // Matched exactly against the key: `get_module_type_info` also
            // accepts display names, but only the key is ever advertised in a
            // resource URI, so the reader stays as strict as it was.
            let known = self
                .bridge
                .list_module_types_brief()
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
            if !known.iter().any(|t| t.type_key == type_key) {
                return Err(ErrorData::resource_not_found(
                    format!("Module type '{type_key}' not found"),
                    None,
                ));
            }
            let info = self
                .bridge
                .get_module_type_info(type_key)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

            let json = to_json(&info);
            Ok(ReadResourceResult::new(vec![ResourceContents::text(json, uri.clone())]).into())
        } else if let Some(slug) = uri.strip_prefix(PATCH_URI_PREFIX) {
            // Look up patch by slug — match by converting name to slug
            let patches = self
                .bridge
                .list_example_patches()
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

            let patch_name = patches
                .iter()
                .find(|p| patch_slug(&p.name) == slug)
                .map(|p| p.name.clone())
                .ok_or_else(|| {
                    ErrorData::resource_not_found(format!("Patch '{slug}' not found"), None)
                })?;

            let data = self
                .bridge
                .get_example_patch(&patch_name)
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;

            let json = to_json(&data);
            Ok(ReadResourceResult::new(vec![ResourceContents::text(json, uri.clone())]).into())
        } else {
            Err(ErrorData::resource_not_found(
                format!("Unknown resource URI: {uri}"),
                None,
            ))
        }
    }

    /// Autocomplete a variable in one of our resource-template URIs.
    ///
    /// The protocol addresses completion at prompts and resource templates
    /// only — there is no `ref/tool`, in this spec version or the draft — so
    /// this reaches the two templates and nothing else. Tool arguments are
    /// served by their input schemas instead (see the `enum`s on the
    /// closed-set arguments) and by the near-miss hints their errors carry.
    ///
    /// Candidates are ranked by the same policy every hint uses, so what a
    /// completion offers first is what a typo would have been corrected to.
    /// An empty value lists everything, which is what a client opening a
    /// dropdown before typing expects, and is the one case the hint ranking
    /// deliberately answers with nothing.
    #[allow(clippy::unused_async)]
    async fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, ErrorData> {
        // We advertise no prompts, so a prompt reference has nothing to offer.
        let Reference::Resource(template) = &request.r#ref else {
            return Ok(CompleteResult::default());
        };

        let argument = &request.argument;
        let values = match template.uri.as_str() {
            MODULE_TYPE_URI_TEMPLATE if argument.name == MODULE_TYPE_URI_VARIABLE => {
                self.complete_module_type_key(&argument.value)
            }
            PATCH_URI_TEMPLATE if argument.name == PATCH_URI_VARIABLE => {
                self.complete_patch_slug(&argument.value)
            }
            _ => Vec::new(),
        };

        let total = values.len();
        let truncated: Vec<String> = values
            .into_iter()
            .take(CompletionInfo::MAX_VALUES)
            .collect();
        let has_more = total > truncated.len();
        // Cannot fail — the take() above is the cap `with_pagination` checks —
        // but surfaced rather than swallowed, so raising that cap can never
        // turn into a silently empty completion.
        let completion =
            CompletionInfo::with_pagination(truncated, u32::try_from(total).ok(), has_more)
                .map_err(|e| ErrorData::internal_error(e, None))?;
        Ok(CompleteResult::new(completion))
    }
}

/// One completion candidate: the value to offer, and a second spelling it is
/// also findable by (a module key and its display name, a patch slug and the
/// patch's real name).
type CompletionCandidate = (String, String);

/// The candidates matching what has been typed, best first.
///
/// Completion matches far more loosely than the near-miss hints, because it is
/// a *filter over a list the caller can see* rather than a guess at one: any
/// substring counts and there is no minimum length. Someone two characters into
/// a dropdown means "narrow this down", not "I may have misspelled something" —
/// the hint ranking answers `ba` with nothing, which is right for an error
/// message and useless for a dropdown over `sub-bass` and `spacey-bass`.
///
/// Only when nothing contains the text does it fall back to the shared
/// near-miss ranking, which is what still recovers an actual typo (`lmit`).
/// Both spellings are matched, so typing a name reaches a value keyed by
/// something else.
fn filter_completions(typed: &str, candidates: &[CompletionCandidate]) -> Vec<String> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return candidates.iter().map(|(value, _)| value.clone()).collect();
    }

    let mut ranked: Vec<(usize, &str)> = candidates
        .iter()
        .filter_map(|(value, alias)| {
            let rank = [value, alias]
                .into_iter()
                .filter_map(|spelling| {
                    let spelling = spelling.to_lowercase();
                    if spelling.starts_with(&typed) {
                        Some(0)
                    } else if spelling.contains(&typed) {
                        Some(1)
                    } else {
                        None
                    }
                })
                .min()?;
            Some((rank, value.as_str()))
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    if !ranked.is_empty() {
        return ranked
            .into_iter()
            .map(|(_, value)| value.to_string())
            .collect();
    }

    // Uncapped on purpose: `complete` is the one place that truncates, and it
    // counts `total` from what this returns. Capping here too would report a
    // `total` that had already been cut down to the cap, so `hasMore` would say
    // `false` over a list that really did have more.
    synth_core::suggest::similar_by(
        &typed,
        candidates.iter(),
        |(value, alias)| [value.as_str(), alias.as_str()],
        candidates.len(),
    )
    .into_iter()
    .map(|(value, _)| value.clone())
    .collect()
}

impl SynthMcpServer {
    /// Module type keys matching what has been typed so far.
    ///
    /// Findable by display name as well as key, then answered with the key: a
    /// caller part-way through typing `limit` has written neither a prefix nor
    /// a near miss of `lmt`, and only the name connects the two.
    fn complete_module_type_key(&self, typed: &str) -> Vec<String> {
        let Ok(types) = self.bridge.list_module_types_brief() else {
            return Vec::new();
        };
        let candidates: Vec<CompletionCandidate> = types
            .into_iter()
            .map(|info| (info.type_key, info.name))
            .collect();
        filter_completions(typed, &candidates)
    }

    /// Example-patch slugs matching what has been typed so far.
    ///
    /// Findable by the patch's real name too, since the slug is a rendering of
    /// that name and a caller is far likelier to know the name.
    fn complete_patch_slug(&self, typed: &str) -> Vec<String> {
        let Ok(patches) = self.bridge.list_example_patches() else {
            return Vec::new();
        };
        let candidates: Vec<CompletionCandidate> = patches
            .into_iter()
            .map(|patch| (patch_slug(&patch.name), patch.name))
            .collect();
        filter_completions(typed, &candidates)
    }
}

/// Convert input structs to bridge-level types.
fn convert_instrument_def(
    instrument_id: Option<InstrumentId>,
    name: String,
    midi_channel: Option<MidiChannel>,
    volume: Option<Gain>,
    pan: Option<BipolarValue>,
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
#[path = "server/tests/automation_target_input.rs"]
mod automation_target_input_tests;

#[cfg(test)]
#[path = "server/tests/batch_msg.rs"]
mod batch_msg_tests;

#[cfg(test)]
#[path = "server/tests/summarize_params.rs"]
mod summarize_params_tests;

#[cfg(test)]
#[path = "server/tests/panic_isolation.rs"]
mod panic_isolation_tests;

#[cfg(test)]
#[path = "server/tests/schema_range.rs"]
mod schema_range_tests;

#[cfg(test)]
#[path = "server/tests/tool_annotations.rs"]
mod tool_annotations_tests;

#[cfg(test)]
#[path = "server/tests/completions.rs"]
mod completions_tests;

#[cfg(test)]
#[path = "server/tests/schema_enum.rs"]
mod schema_enum_tests;

#[cfg(test)]
#[path = "server/tests/output_schema.rs"]
mod output_schema_tests;
