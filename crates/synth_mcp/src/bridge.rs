//! Bridge trait for connecting the MCP server to the synth engine.
//!
//! Implementors provide read access to engine state and write access
//! via the command sender. The trait uses primitive types to avoid
//! leaking synth_engine types into the MCP crate.
//!
//! # Design: raw primitives at the serialization boundary
//!
//! All bridge methods accept and return raw primitives (`f32`, `u8`, `u64`, etc.)
//! rather than domain newtypes (`Hertz`, `BipolarValue`, `NormalizedValue`, etc.).
//! This is intentional: the MCP protocol exchanges JSON, so the bridge sits at the
//! serialization boundary where type-safe wrappers add friction without safety.
//! Validation of ranges and semantics happens in `server.rs` (the MCP tool layer)
//! before values reach the bridge, and conversion to domain newtypes happens in the
//! bridge implementation (`mcp_bridge.rs` in the `pertylizer` crate).

use crate::error::McpBridgeError;
use crate::types::{
    ApplyExamplePatchResult, AudioPreview, AutoGainStageResult, AutomationLaneInfo,
    AutomationPointInfo, AutomationSummaryGroup, AutomationSummaryLane, AutomationSummaryResult,
    AutomationTargetInfo, AwePresetInfo, AweStateInfo, BatchResult, BuildInstrumentResult,
    ChordProgressionStep, ConnectionCheckResult, ConnectionInfo, CreateChordProgressionResult,
    DetailedSampleInfo, DiagnosticSeverity, EngineStatus, ExamplePatchInfo, GraphDiagnostic,
    InputDeviceInfo, InputStateInfo, InsertModuleResult, InstrumentInfo, InstrumentProfileResult,
    MatrixRoutingInfo, ModuleInfo, ModuleSearchResult, ModuleTypeBrief, ModuleTypeInfo, NoteInfo,
    NoteProcessorInfo, OptimizeResult, ParameterInfo, PatchResourceData, PatternInfo,
    PlacementInfo, ProjectLintEntry, ProjectLintReport, ProjectSchemaInfo, ReturnBusInfo,
    ReturnEffectInfo, SampleInfo, SamplerStateInfo, SetSongResult, SongInfo, TempoPoint, TrackInfo,
    UiSnapshot, VersionInfo,
};

// === Bridge-level data structures for batch operations ===

/// Per-note glide (portamento/glissando) for batch note operations.
///
/// Primitive fields only — the bridge implementation resolves these into the
/// sequencer's `Glide`/`GlideFrom`/`GlideInterp` types.
#[derive(Debug, Clone, Copy, Default)]
pub struct BridgeGlide {
    /// Glide source as a signed semitone offset relative to the note.
    pub from_semitones: Option<f32>,
    /// Glide source as an absolute MIDI pitch (takes precedence over `from_semitones`).
    pub from_pitch: Option<u8>,
    /// Glide time in milliseconds.
    pub time_ms: f32,
    /// `true` = stepped glissando, `false` = continuous portamento.
    pub stepped: bool,
}

/// Per-note vibrato for batch note operations (primitive fields only; the
/// bridge implementation resolves `shape` into the sequencer's `VibratoShape`).
#[derive(Debug, Clone, Default)]
pub struct BridgeVibrato {
    /// Peak pitch deviation in semitones.
    pub depth: f32,
    /// LFO rate in Hz.
    pub rate: f32,
    /// Depth fade-in time in milliseconds.
    pub delay_ms: f32,
    /// LFO shape token (`sine`/`triangle`/`square`/`saw`); defaults to sine.
    pub shape: Option<String>,
}

/// Per-note expression block for batch note operations.
#[derive(Debug, Clone, Default)]
pub struct BridgeExpression {
    /// Accent: velocity multiplier (1.0 = unchanged).
    pub accent: Option<f32>,
    /// Gate as a fraction of duration (0..1).
    pub gate: Option<f32>,
    /// Ghost (forced-soft) note.
    pub ghost: bool,
    /// Trigger probability (0..1).
    pub probability: Option<f32>,
    /// Optional per-note vibrato.
    pub vibrato: Option<BridgeVibrato>,
}

/// Note data for batch add/replace operations.
pub struct BridgeNoteData {
    /// MIDI pitch (0-127).
    pub pitch: u8,
    /// Start position in beats.
    pub start_beat: f32,
    /// Duration in beats.
    pub duration_beats: f32,
    /// Velocity (0-127).
    pub velocity: u8,
    /// Per-note tie/legato (no retrigger onto the successor).
    pub legato: bool,
    /// Optional per-note glide.
    pub glide: Option<BridgeGlide>,
    /// Optional per-note expression block.
    pub expression: Option<BridgeExpression>,
}

/// Note update for batch update operations.
pub struct BridgeNoteUpdate {
    /// Note ID to update.
    pub note_id: u64,
    /// New pitch, or None to keep current.
    pub pitch: Option<u8>,
    /// New start position in beats, or None to keep current.
    pub start_beat: Option<f32>,
    /// New duration in beats, or None to keep current.
    pub duration_beats: Option<f32>,
    /// New velocity, or None to keep current.
    pub velocity: Option<u8>,
}

/// Pattern data for batch create operations.
pub struct BridgePatternData {
    /// Pattern name.
    pub name: String,
    /// Length in beats.
    pub length_beats: f32,
    /// Optional inline notes.
    pub notes: Vec<BridgeNoteData>,
    /// Optional inline automation points.
    pub automation: Vec<BridgeAutomationPointData>,
}

/// Track data for batch create operations.
pub struct BridgeTrackData {
    /// Track name.
    pub name: String,
    /// Optional instrument ID.
    pub instrument_id: Option<u16>,
}

/// Placement data for batch arrange operations.
pub struct BridgePlacementData {
    /// Pattern ID.
    pub pattern_id: u32,
    /// Track ID.
    pub track_id: u16,
    /// Start position in beats.
    pub start_beat: f32,
}

/// Placement using array indices (for `set_song` where IDs don't exist yet).
pub struct BridgeSongPlacement {
    /// Index into the patterns array.
    pub pattern_index: usize,
    /// Index into the tracks array.
    pub track_index: usize,
    /// Start position in beats.
    pub start_beat: f32,
}

// === Bridge-level data structures for batch instrument building ===

/// Module definition for `build_instrument`.
pub struct BridgeModuleDef {
    /// Module type key (e.g. "osc", "flt", "dly").
    pub module_type: String,
    /// Parameters as (name, value) pairs.
    pub params: Vec<(String, BridgeParamValue)>,
}

/// Parameter value for bridge-level batch operations.
pub enum BridgeParamValue {
    /// Numeric value (f32).
    Number(f64),
    /// Choice/enum value (e.g. "sawtooth", "lowpass").
    Choice(String),
    /// Boolean value.
    Bool(bool),
}

/// Direction to move an effect within a return-bus effect chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnEffectMove {
    /// Move the effect one slot earlier in the processing order.
    Up,
    /// Move the effect one slot later in the processing order.
    Down,
}

/// Connection definition using array indices into the modules array.
pub struct BridgeConnectionDef {
    /// 0-based index of the source module in the modules array.
    pub from_index: usize,
    /// Source port name.
    pub from_port: String,
    /// 0-based index of the destination module in the modules array.
    pub to_index: usize,
    /// Destination port name.
    pub to_port: String,
}

/// Complete instrument definition for `build_instrument`.
pub struct BridgeInstrumentDef {
    /// Optional existing instrument ID to update (clears graph and rebuilds).
    pub instrument_id: Option<u64>,
    /// Instrument name.
    pub name: String,
    /// Optional MIDI channel (1-16).
    pub midi_channel: Option<u8>,
    /// Optional volume (0.0-2.0).
    pub volume: Option<f32>,
    /// Optional pan (-1.0 = left, 0.0 = center, 1.0 = right).
    pub pan: Option<f32>,
    /// Modules to create.
    pub modules: Vec<BridgeModuleDef>,
    /// Connections between modules (using array indices).
    pub connections: Vec<BridgeConnectionDef>,
}

/// Sample rate at which an offline analysis render runs.
///
/// `Full` (44.1 kHz) is the default and the only rate at which every metric is
/// trustworthy — it covers the full audible band (Nyquist 22 kHz). `Draft`
/// (22.05 kHz) roughly halves render time per buffer, which compounds across the
/// per-track renders of `analyze_section`/`analyze_masking_matrix`, but its
/// Nyquist is only 11 kHz, so the `high` energy band is truncated, `true_peak`
/// is less reliable, LUFS is biased (its K-weighting filters are tuned for
/// 44.1 kHz), and distortion-heavy patches alias more. Use `Draft` for quick
/// level/balance/RMS passes; use `Full` when LUFS accuracy, high-band,
/// true-peak, or saturation behavior matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderQuality {
    /// 22.05 kHz — ~2× faster per render, full-band metrics unreliable.
    Draft,
    /// 44.1 kHz — full audible band, all metrics trustworthy (default).
    #[default]
    Full,
}

impl RenderQuality {
    /// 22.05 kHz draft rate.
    pub const DRAFT_SAMPLE_RATE: u32 = 22_050;
    /// 44.1 kHz full rate (the analysis default).
    pub const FULL_SAMPLE_RATE: u32 = 44_100;

    /// Render sample rate in Hz for this quality.
    #[must_use]
    pub fn sample_rate(self) -> u32 {
        match self {
            Self::Draft => Self::DRAFT_SAMPLE_RATE,
            Self::Full => Self::FULL_SAMPLE_RATE,
        }
    }

    /// Parse the MCP string flag. `Some("draft")` → `Draft`; everything else
    /// (including `None` and unrecognized values) → `Full`, the safe default.
    #[must_use]
    pub fn parse(flag: Option<&str>) -> Self {
        match flag {
            Some(s) if s.eq_ignore_ascii_case("draft") => Self::Draft,
            _ => Self::Full,
        }
    }
}

/// Which optional stages of the signal chain an offline analysis render should
/// reconstruct on top of the dry instrument sum, plus the render sample rate.
///
/// The default (`AnalysisScope::default()`) preserves the historical behavior:
/// instruments and their own effect chains only, return busses summed dry, no
/// master processing, rendered at the full 44.1 kHz rate. Each effect flag opts
/// a stage back in; `render_sample_rate` selects the render resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnalysisScope {
    /// Load the master effect chain (master-bus limiter/EQ/compressor, …).
    pub master_effects: bool,
    /// Load each return bus's effect chain (else returns stay dry).
    pub return_effects: bool,
    /// Reconstruct AWE room simulation. Not yet implemented — requesting it
    /// emits a warning on the result and otherwise behaves as `false`.
    pub awe: bool,
    /// Sample rate the offline render runs at (see [`RenderQuality`]).
    pub render_sample_rate: u32,
}

impl Default for AnalysisScope {
    fn default() -> Self {
        Self {
            master_effects: false,
            return_effects: false,
            awe: false,
            render_sample_rate: RenderQuality::FULL_SAMPLE_RATE,
        }
    }
}

/// One side of a `compare_spectra` comparison: either an offline render
/// (optionally soloing one instrument over a window) or an imported sample / WAV
/// file. A `Some` `sample_id_or_path` selects the sample source; otherwise it is
/// a render.
#[derive(Debug, Clone, Default)]
pub struct SpectrumSource {
    /// `Some` → analyse this imported-sample id or WAV path. `None` → render.
    pub sample_id_or_path: Option<String>,
    /// (Render only) solo this instrument; `None` = full mix.
    pub instrument_id: Option<u16>,
    /// (Render only) absolute start tick (default 0).
    pub start_tick: Option<u64>,
    /// (Render only) seconds to render (default 10).
    pub duration_seconds: Option<f32>,
    /// (Sample only) offset into the decoded sample to start analysing, in ms
    /// (default 0). Lets the caller slide a window to a single voiced frame.
    pub start_ms: Option<f32>,
    /// Length of the analysis window in ms. For a sample, slices the decoded
    /// buffer (clamped + zero-padded past the end). For a render, overrides
    /// `duration_seconds`. `None` = whole sample / default render duration.
    pub window_len_ms: Option<f32>,
}

/// The optional time-resolved (per-frame) mode of
/// [`McpBridge::compare_spectra`]. When `enabled`, both sources are framed into
/// spectrograms and compared frame-by-frame with target-energy masking — the
/// framed distances are *added* to the aggregate result, not a replacement. The
/// aggregate scalar averages over the whole window, so on time-sparse / staccato
/// material this per-frame path is what carries the ranking signal. All fields
/// are ignored when `enabled` is `false` (the default, unchanged behaviour).
#[derive(Debug, Clone, Default)]
pub struct TimeResolvedOptions {
    /// Turn the framed path on. `false` = aggregate-only.
    pub enabled: bool,
    /// Frame hop in ms (`None` = the spectrogram default, 20 ms).
    pub hop_ms: Option<f32>,
    /// Analysed frame length in ms (`None` = the spectrogram default, 40 ms).
    pub frame_len_ms: Option<f32>,
    /// Mask frames on the target's per-frame energy (`true`, the default) or
    /// compare every paired frame (`false`).
    pub mask_target_energy: bool,
    /// Envelope-cross-correlate the two buffers and shift the candidate before
    /// framing (`true`, the default) or pair frames from sample 0 (`false`).
    pub align_envelope: bool,
    /// Maximum alignment search in ms (`None` = 250 ms). Ignored when
    /// `align_envelope` is `false`.
    pub align_max_ms: Option<f32>,
}

impl AnalysisScope {
    /// Build a scope from the optional MCP flags. `all` turns on every effect
    /// stage; the per-stage flags OR in on top of it. Every `None` effect flag
    /// resolves to `false`, so omitting them yields the dry default. `quality`
    /// selects the render resolution (`RenderQuality::default()` = full).
    #[must_use]
    pub fn from_flags(
        all: Option<bool>,
        master_effects: Option<bool>,
        return_effects: Option<bool>,
        awe: Option<bool>,
        quality: RenderQuality,
    ) -> Self {
        let all = all.unwrap_or(false);
        Self {
            master_effects: all || master_effects.unwrap_or(false),
            return_effects: all || return_effects.unwrap_or(false),
            awe: all || awe.unwrap_or(false),
            render_sample_rate: quality.sample_rate(),
        }
    }
}

/// Where to splice a new module into an instrument's audio signal path.
///
/// The voice graph is a DAG, not a linear chain, so "position" is expressed
/// relative to existing modules/cables rather than as a numeric index (which
/// would mean different things on instruments with different topologies).
#[derive(Debug, Clone)]
pub enum InsertAnchor {
    /// Splice the (unique) outgoing audio cable of this module id.
    After(String),
    /// Splice the (unique) incoming audio cable of this module id.
    Before(String),
    /// Splice the outgoing audio cable of the (unique) module of this type.
    AfterType(String),
    /// Splice the incoming audio cable of the (unique) module of this type.
    BeforeType(String),
    /// Splice this exact existing connection.
    Connection {
        from_module: String,
        from_port: String,
        to_module: String,
        to_port: String,
    },
    /// Default: splice the audio cable feeding the instrument's output module
    /// (i.e. insert at the very end of the audio path, just before output).
    BeforeOutput,
}

/// Find the single module whose type display name matches `canonical_name`.
/// Errors when none or more than one match (the caller must then disambiguate
/// with an explicit module id).
fn unique_module_of_type<'a>(
    canonical_name: &str,
    modules: &'a [ModuleInfo],
) -> Result<&'a str, McpBridgeError> {
    let ids: Vec<&str> = modules
        .iter()
        .filter(|m| m.module_type == canonical_name)
        .map(|m| m.id.as_str())
        .collect();
    match ids.as_slice() {
        [] => Err(McpBridgeError::Other(format!(
            "no module of type '{canonical_name}' in this instrument"
        ))),
        [id] => Ok(id),
        many => Err(McpBridgeError::Other(format!(
            "type '{canonical_name}' is ambiguous ({} modules: {}); use `after`/`before` with a specific module id",
            many.len(),
            many.join(", ")
        ))),
    }
}

/// Find the single outgoing audio cable from `module_id` (matching one of its
/// `audio_out_ports`). Errors when there is none, or when the path branches
/// (more than one) — the caller must then pass an explicit connection.
fn unique_audio_cable_from(
    module_id: &str,
    audio_out_ports: &[String],
    connections: &[ConnectionInfo],
) -> Result<ConnectionInfo, McpBridgeError> {
    let cables: Vec<&ConnectionInfo> = connections
        .iter()
        .filter(|c| c.from_module == module_id && audio_out_ports.contains(&c.from_port))
        .collect();
    match cables.as_slice() {
        [] => Err(McpBridgeError::Other(format!(
            "'{module_id}' has no outgoing audio cable to splice after; connect it first, \
             or pass an explicit from_module/from_port/to_module/to_port"
        ))),
        [c] => Ok((*c).clone()),
        many => Err(McpBridgeError::Other(format!(
            "the audio path branches at '{module_id}' ({} outgoing audio cables); disambiguate \
             with an explicit from_module/from_port/to_module/to_port. Cables: {}",
            many.len(),
            many.iter()
                .map(|c| format!(
                    "{}:{} → {}:{}",
                    c.from_module, c.from_port, c.to_module, c.to_port
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ))),
    }
}

/// Find the single incoming audio cable into `module_id` (matching one of its
/// `audio_in_ports`). Errors when there is none, or when more than one feeds in.
fn unique_audio_cable_into(
    module_id: &str,
    audio_in_ports: &[String],
    connections: &[ConnectionInfo],
) -> Result<ConnectionInfo, McpBridgeError> {
    let cables: Vec<&ConnectionInfo> = connections
        .iter()
        .filter(|c| c.to_module == module_id && audio_in_ports.contains(&c.to_port))
        .collect();
    match cables.as_slice() {
        [] => Err(McpBridgeError::Other(format!(
            "'{module_id}' has no incoming audio cable to splice before; connect it first, \
             or pass an explicit from_module/from_port/to_module/to_port"
        ))),
        [c] => Ok((*c).clone()),
        many => Err(McpBridgeError::Other(format!(
            "more than one audio cable feeds '{module_id}' ({}); disambiguate with an explicit \
             from_module/from_port/to_module/to_port. Cables: {}",
            many.len(),
            many.iter()
                .map(|c| format!(
                    "{}:{} → {}:{}",
                    c.from_module, c.from_port, c.to_module, c.to_port
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ))),
    }
}

/// Bridge between the MCP server and the synth engine.
///
/// All methods use primitive types. Conversion to domain types
/// (Hertz, MidiNote, Param, etc.) happens in the implementation.
pub trait SynthBridge: Send + Sync + 'static {
    // === Read operations ===

    /// List all instruments with basic info.
    fn list_instruments(&self) -> Result<Vec<InstrumentInfo>, McpBridgeError>;

    /// Auto-infer a profile (role, envelope shape, register, texture, ...)
    /// for every instrument that at least one of the song's tracks routes to.
    /// The same inference path that `analyze_harmony`'s `exclude_drums = true`
    /// default uses — but exposed directly so external tools can read or
    /// debug the classification.
    fn get_instrument_profiles(&self) -> Result<Vec<InstrumentProfileResult>, McpBridgeError>;

    /// Get detailed info for a single instrument.
    fn get_instrument_info(&self, instrument_id: u64) -> Result<InstrumentInfo, McpBridgeError>;

    /// List all modules in an instrument's voice graph.
    fn list_modules(&self, instrument_id: u64) -> Result<Vec<ModuleInfo>, McpBridgeError>;

    /// Get detailed info for a single module.
    fn get_module_info(
        &self,
        instrument_id: u64,
        module_id: &str,
    ) -> Result<ModuleInfo, McpBridgeError>;

    /// Get all connections in the voice graph.
    fn get_connections(&self, instrument_id: u64) -> Result<Vec<ConnectionInfo>, McpBridgeError>;

    /// Get all active Mod Matrix routings across every Mod Matrix module in
    /// the instrument. Returns slot rows with semantic source/destination
    /// IDs so callers don't have to decode the `Slot N Source` choice index
    /// by hand.
    fn get_mod_matrix_routings(
        &self,
        instrument_id: u64,
    ) -> Result<Vec<MatrixRoutingInfo>, McpBridgeError>;

    /// Install (or clear) a YAMS control script on a Mod Matrix slot (Step 2).
    /// `slot` is **1-based** (matching `get_mod_matrix_routings`). A non-empty
    /// `source` is compiled (a compile error is returned); an empty `source`
    /// clears the slot, reverting it to scalar `amount × source`.
    fn set_mod_matrix_script(
        &self,
        instrument_id: u64,
        module_id: &str,
        slot: u8,
        source: &str,
    ) -> Result<(), McpBridgeError>;

    /// Get a single parameter value.
    fn get_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
    ) -> Result<ParameterInfo, McpBridgeError>;

    /// Get engine-wide status (CPU, voices, meters, transport).
    fn get_engine_status(&self) -> Result<EngineStatus, McpBridgeError>;

    /// Get build/version info for the running application (version, build
    /// timestamp, git commit/branch/dirty state at build time).
    fn get_version(&self) -> Result<VersionInfo, McpBridgeError>;

    /// Run diagnostics on the graph and report issues.
    fn get_graph_diagnostics(
        &self,
        instrument_id: u64,
    ) -> Result<Vec<GraphDiagnostic>, McpBridgeError>;

    /// Return the authoritative on-disk `.pertyproj` JSON Schema plus the format
    /// and build versions. The implementor embeds the committed
    /// `schemas/project.schema.json` (the exact artifact external tools must
    /// match), so callers can validate or diff project files without re-deriving
    /// the schema and risking introspection-vs-disk drift.
    fn get_project_schema(&self) -> Result<ProjectSchemaInfo, McpBridgeError>;

    /// Run the graph diagnostics over every instrument and aggregate the results
    /// into one project-wide load-lint report — a single call that surfaces
    /// *behavioural* warnings (unconnected ports, silent voices, feedback loops)
    /// across the whole project, complementing the structural `get_project_schema`
    /// validation. Composed from `list_instruments` + `get_graph_diagnostics`, so
    /// every bridge gets it for free and there's a single source of diagnostic
    /// truth. Only instruments with an actionable diagnostic appear in `entries`.
    fn lint_project(&self) -> Result<ProjectLintReport, McpBridgeError> {
        let instruments = self.list_instruments()?;
        let mut per_instrument = Vec::with_capacity(instruments.len());
        for instrument in &instruments {
            match self.get_graph_diagnostics(instrument.id) {
                Ok(diagnostics) => {
                    per_instrument.push((instrument.id, instrument.name.clone(), diagnostics));
                }
                // The instrument vanished between the `list_instruments` snapshot
                // and this call (a concurrent GUI/MCP delete). Skip it rather than
                // aborting the whole report over a transient race.
                Err(McpBridgeError::InstrumentNotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(build_lint_report(per_instrument))
    }

    // === Instrument lifecycle ===

    /// Create a new instrument. Returns info about the created instrument.
    fn create_instrument(&self, name: &str) -> Result<InstrumentInfo, McpBridgeError>;

    /// Delete an instrument.
    fn delete_instrument(&self, instrument_id: u64) -> Result<(), McpBridgeError>;

    /// Rename an instrument.
    fn rename_instrument(&self, instrument_id: u64, name: &str) -> Result<(), McpBridgeError>;

    /// Set the free-text description / intent on an instrument. Pass `""`
    /// to clear. Never affects audio; surfaces in `InstrumentInfo` for
    /// later AI reads.
    fn set_instrument_description(
        &self,
        instrument_id: u64,
        description: &str,
    ) -> Result<(), McpBridgeError>;

    /// Set or clear an instrument's accent color. Accepts `"#RRGGBB"` /
    /// `"#RRGGBBAA"`; pass `""` to clear back to "auto" / default. Never affects
    /// audio; surfaces in `InstrumentInfo.color` and is persisted on save.
    fn set_instrument_color(&self, instrument_id: u64, color: &str) -> Result<(), McpBridgeError>;

    /// Set or clear the patch-level accent color on an instrument's currently
    /// loaded patch (distinct from `set_instrument_color`). Accepts
    /// `"#RRGGBB"` / `"#RRGGBBAA"`; pass `""` to clear. Travels with the patch
    /// when saved; surfaces in `InstrumentInfo.patch_color`.
    fn set_patch_color(&self, instrument_id: u64, color: &str) -> Result<(), McpBridgeError>;

    /// Set or clear the patch-level description on an instrument's
    /// currently-loaded patch. Pass `""` to clear (treated as `None`).
    fn set_patch_description(
        &self,
        instrument_id: u64,
        description: &str,
    ) -> Result<(), McpBridgeError>;

    /// Set or clear the free-text description on a specific module instance
    /// (intent for this module — e.g. "wobble LFO for the cutoff"). Distinct
    /// from the module *type* doc. Pass `""` to clear. Rejected if the module
    /// does not exist or the description exceeds the hard length limit.
    fn set_module_description(
        &self,
        instrument_id: u64,
        module_id: &str,
        description: &str,
    ) -> Result<(), McpBridgeError>;

    /// Set or clear the sidechain source instrument id. Pass `None` to
    /// disable sidechain routing into this instrument's compressors.
    /// Self-routing is rejected.
    fn set_sidechain_source(
        &self,
        instrument_id: u64,
        source: Option<u64>,
    ) -> Result<(), McpBridgeError>;

    /// Set instrument volume (0.0-2.0).
    fn set_instrument_volume(&self, instrument_id: u64, volume: f32) -> Result<(), McpBridgeError>;

    /// Set instrument pan (-1.0 = left, 0.0 = center, 1.0 = right).
    fn set_instrument_pan(&self, instrument_id: u64, pan: f32) -> Result<(), McpBridgeError>;

    /// Set instrument mute state.
    fn set_instrument_mute(&self, instrument_id: u64, muted: bool) -> Result<(), McpBridgeError>;

    /// Set instrument solo state.
    fn set_instrument_solo(&self, instrument_id: u64, solo: bool) -> Result<(), McpBridgeError>;

    /// Set instrument MIDI channel (1-16).
    fn set_instrument_midi_channel(
        &self,
        instrument_id: u64,
        channel: u8,
    ) -> Result<(), McpBridgeError>;

    /// Set instrument enabled state.
    fn set_instrument_enabled(
        &self,
        instrument_id: u64,
        enabled: bool,
    ) -> Result<(), McpBridgeError>;

    /// Set instrument category (for visualization routing).
    fn set_instrument_category(
        &self,
        instrument_id: u64,
        category: &str,
    ) -> Result<(), McpBridgeError>;

    /// Set the voice allocation mode. Accepts "Polyphonic", "Mono", "Legato",
    /// or "Unison" (case-sensitive).
    fn set_instrument_allocation_mode(
        &self,
        instrument_id: u64,
        mode: &str,
    ) -> Result<(), McpBridgeError>;

    /// Set the voice-stealing strategy. Accepts "None", "Oldest", "Quietest",
    /// "LowestPriority", or "SameNote" (case-sensitive).
    fn set_instrument_stealing_strategy(
        &self,
        instrument_id: u64,
        strategy: &str,
    ) -> Result<(), McpBridgeError>;

    /// Set the unison detune spread in cents (total across `Unison`-mode voices).
    fn set_instrument_unison_detune(
        &self,
        instrument_id: u64,
        cents: f32,
    ) -> Result<(), McpBridgeError>;

    /// Set the unison stereo spread (0.0 = centred .. 1.0 = full width).
    fn set_instrument_unison_spread(
        &self,
        instrument_id: u64,
        spread: f32,
    ) -> Result<(), McpBridgeError>;

    /// Set the maximum polyphony (1..=128). Takes effect on the next voice-graph
    /// rebuild (e.g. project load), not live.
    fn set_instrument_max_voices(
        &self,
        instrument_id: u64,
        max_voices: u32,
    ) -> Result<(), McpBridgeError>;

    // === Write operations ===

    /// Set a module parameter by name. The value may be a number, a boolean, or
    /// a string choice (resolved against the parameter's descriptor). Returns the
    /// parameter info with the actual value set.
    fn set_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
        value: BridgeParamValue,
    ) -> Result<ParameterInfo, McpBridgeError>;

    /// Send a MIDI note on.
    fn note_on(&self, note: u8, velocity: u8, channel: u8) -> Result<(), McpBridgeError>;

    /// Send a MIDI note off.
    fn note_off(&self, note: u8, channel: u8) -> Result<(), McpBridgeError>;

    // === Example patches ===

    /// List all available example patches grouped by category.
    fn list_example_patches(&self) -> Result<Vec<ExamplePatchInfo>, McpBridgeError>;

    /// Get full data for an example patch (modules, connections, parameters).
    fn get_example_patch(&self, name: &str) -> Result<PatchResourceData, McpBridgeError>;

    /// Queue an example patch for loading (GUI picks it up next frame).
    fn load_example_patch(&self, name: &str) -> Result<String, McpBridgeError>;

    /// Get a snapshot of the current UI layout (module positions, sizes, connections).
    fn get_ui_snapshot(&self, instrument_id: u64) -> Result<UiSnapshot, McpBridgeError>;

    /// Request auto-layout of modules in the patch view (GUI applies it next frame).
    fn request_auto_layout(&self) -> Result<String, McpBridgeError>;

    // === Module management ===

    /// List all available module types with their ports and parameters.
    fn list_module_types(&self) -> Result<Vec<ModuleTypeInfo>, McpBridgeError>;

    /// Compact catalog: just `{type_key, name, category}` per module type, for
    /// callers that only need to pick a type without the full port/parameter dump.
    fn list_module_types_brief(&self) -> Result<Vec<ModuleTypeBrief>, McpBridgeError>;

    /// Add a module to an instrument's voice graph. Returns confirmation message.
    fn add_module(&self, instrument_id: u64, module_type: &str) -> Result<String, McpBridgeError>;

    /// Remove a module from an instrument's voice graph.
    fn remove_module(&self, instrument_id: u64, module_id: &str) -> Result<(), McpBridgeError>;

    /// Connect two module ports.
    fn connect(
        &self,
        instrument_id: u64,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<(), McpBridgeError>;

    /// Disconnect two module ports.
    fn disconnect(
        &self,
        instrument_id: u64,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<(), McpBridgeError>;

    /// Clear the entire voice graph for an instrument (remove all modules and connections).
    fn clear_graph(&self, instrument_id: u64) -> Result<(), McpBridgeError>;

    /// Splice a new module of `module_type` into an existing audio cable,
    /// re-routing `source → new module → destination`.
    ///
    /// This is a convenience over manual add + disconnect + connect. The new
    /// module is wired through its first audio input and first audio output, so
    /// the type must carry audio (a pure modulator like an LFO is rejected).
    /// `anchor` selects which existing cable to cut; on any wiring failure the
    /// original cable is restored and the new module removed.
    ///
    /// Default trait method composed from the graph primitives — implementors
    /// get it for free.
    fn insert_module_between(
        &self,
        instrument_id: u64,
        module_type: &str,
        anchor: InsertAnchor,
    ) -> Result<InsertModuleResult, McpBridgeError> {
        use std::collections::HashMap;

        // The new module must carry audio through: pick its first audio in/out.
        let new_info = self.get_module_type_info(module_type)?;
        let audio_in = new_info
            .input_ports
            .iter()
            .find(|p| p.signal_type == "audio")
            .map(|p| p.name.clone())
            .ok_or_else(|| {
                McpBridgeError::Other(format!(
                    "module type '{module_type}' has no audio input port, so it can't be spliced into the signal path"
                ))
            })?;
        let audio_out = new_info
            .output_ports
            .iter()
            .find(|p| p.signal_type == "audio")
            .map(|p| p.name.clone())
            .ok_or_else(|| {
                McpBridgeError::Other(format!(
                    "module type '{module_type}' has no audio output port, so it can't be spliced into the signal path"
                ))
            })?;

        let modules = self.list_modules(instrument_id)?;
        let connections = self.get_connections(instrument_id)?;

        // Audio in/out port names per existing module id (cache per type).
        let mut type_cache: HashMap<String, ModuleTypeInfo> = HashMap::new();
        let mut audio_ports: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
        for m in &modules {
            if !type_cache.contains_key(&m.module_type) {
                let info = self.get_module_type_info(&m.module_type)?;
                type_cache.insert(m.module_type.clone(), info);
            }
            let info = &type_cache[&m.module_type];
            let ins = info
                .input_ports
                .iter()
                .filter(|p| p.signal_type == "audio")
                .map(|p| p.name.clone())
                .collect();
            let outs = info
                .output_ports
                .iter()
                .filter(|p| p.signal_type == "audio")
                .map(|p| p.name.clone())
                .collect();
            audio_ports.insert(m.id.clone(), (ins, outs));
        }

        // Resolve the anchor to the existing cable to splice.
        let target = match anchor {
            InsertAnchor::Connection {
                from_module,
                from_port,
                to_module,
                to_port,
            } => connections
                .iter()
                .find(|c| {
                    c.from_module == from_module
                        && c.from_port == from_port
                        && c.to_module == to_module
                        && c.to_port == to_port
                })
                .cloned()
                .ok_or_else(|| {
                    McpBridgeError::Other(format!(
                        "no existing connection {from_module}:{from_port} → {to_module}:{to_port} to splice"
                    ))
                })?,
            InsertAnchor::After(id) => {
                let outs = audio_ports
                    .get(&id)
                    .map(|(_, o)| o.clone())
                    .ok_or_else(|| McpBridgeError::ModuleNotFound(id.clone()))?;
                unique_audio_cable_from(&id, &outs, &connections)?
            }
            InsertAnchor::Before(id) => {
                let ins = audio_ports
                    .get(&id)
                    .map(|(i, _)| i.clone())
                    .ok_or_else(|| McpBridgeError::ModuleNotFound(id.clone()))?;
                unique_audio_cable_into(&id, &ins, &connections)?
            }
            InsertAnchor::AfterType(t) => {
                let canonical = self.get_module_type_info(&t)?.name;
                let id = unique_module_of_type(&canonical, &modules)?.to_string();
                let outs = audio_ports.get(&id).map(|(_, o)| o.clone()).unwrap_or_default();
                unique_audio_cable_from(&id, &outs, &connections)?
            }
            InsertAnchor::BeforeType(t) => {
                let canonical = self.get_module_type_info(&t)?.name;
                let id = unique_module_of_type(&canonical, &modules)?.to_string();
                let ins = audio_ports.get(&id).map(|(i, _)| i.clone()).unwrap_or_default();
                unique_audio_cable_into(&id, &ins, &connections)?
            }
            InsertAnchor::BeforeOutput => {
                let canonical = self.get_module_type_info("out")?.name;
                let id = unique_module_of_type(&canonical, &modules)?.to_string();
                let ins = audio_ports.get(&id).map(|(i, _)| i.clone()).unwrap_or_default();
                unique_audio_cable_into(&id, &ins, &connections)?
            }
        };

        // Create the new module, then discover its id by set-difference (robust
        // to the confirmation-message format).
        let before_ids: std::collections::HashSet<String> =
            modules.iter().map(|m| m.id.clone()).collect();
        self.add_module(instrument_id, module_type)?;
        let new_id = self
            .list_modules(instrument_id)?
            .into_iter()
            .map(|m| m.id)
            .find(|id| !before_ids.contains(id))
            .ok_or_else(|| {
                McpBridgeError::Other(
                    "could not determine the id of the newly added module".to_string(),
                )
            })?;

        // Splice: cut the old cable, route source → new → destination. Restore
        // the original wiring on any failure so a partial splice never lingers.
        let (fm, fp, tm, tp) = (
            target.from_module,
            target.from_port,
            target.to_module,
            target.to_port,
        );
        self.disconnect(instrument_id, &fm, &fp, &tm, &tp)?;
        if let Err(e) = self.connect(instrument_id, &fm, &fp, &new_id, &audio_in) {
            let _ = self.connect(instrument_id, &fm, &fp, &tm, &tp);
            let _ = self.remove_module(instrument_id, &new_id);
            return Err(e);
        }
        if let Err(e) = self.connect(instrument_id, &new_id, &audio_out, &tm, &tp) {
            let _ = self.disconnect(instrument_id, &fm, &fp, &new_id, &audio_in);
            let _ = self.connect(instrument_id, &fm, &fp, &tm, &tp);
            let _ = self.remove_module(instrument_id, &new_id);
            return Err(e);
        }

        Ok(InsertModuleResult {
            removed_connection: format!("{fm}:{fp} → {tm}:{tp}"),
            new_connections: vec![
                format!("{fm}:{fp} → {new_id}:{audio_in}"),
                format!("{new_id}:{audio_out} → {tm}:{tp}"),
            ],
            module_id: new_id,
        })
    }

    // === Sequencer: Song ===

    /// Get song info (name, tempo, length, pattern/track counts).
    fn get_song_info(&self) -> Result<SongInfo, McpBridgeError>;

    /// Set the song tempo in BPM.
    fn set_song_tempo(&self, bpm: f32) -> Result<(), McpBridgeError>;

    /// Add or replace tempo-map points as `(tick, bpm, ramp)`. Each point
    /// replaces any existing change at the same tick; `ramp` selects a linear
    /// ramp toward the next point (`true`) or a step change (`false`). This
    /// edits the tempo *map*, not the global default tempo (see
    /// [`Self::set_song_tempo`]).
    fn set_tempo_at(&self, points: &[(u64, f32, bool)]) -> Result<(), McpBridgeError>;

    /// Remove tempo-map points at the given absolute ticks. Returns how many
    /// were actually removed.
    fn remove_tempo_at(&self, ticks: &[u64]) -> Result<usize, McpBridgeError>;

    /// Read the full tempo map, sorted by tick. Does not include the global
    /// default tempo.
    fn get_tempo_map(&self) -> Result<Vec<TempoPoint>, McpBridgeError>;

    /// Set the song name.
    fn set_song_name(&self, name: &str) -> Result<(), McpBridgeError>;

    /// Set the transport loop region. When `enabled` is true, playback wraps
    /// from `end_beats` back to `start_beats`.
    fn set_transport_loop(
        &self,
        start_beats: f32,
        end_beats: f32,
        enabled: bool,
    ) -> Result<(), McpBridgeError>;

    /// Clear the transport loop region (disable wrapping).
    fn clear_transport_loop(&self) -> Result<(), McpBridgeError>;

    // === Sequencer: Patterns ===

    /// List all patterns in the song.
    fn list_patterns(&self) -> Result<Vec<PatternInfo>, McpBridgeError>;

    /// Create a new pattern with the given name and length in beats.
    fn create_pattern(&self, name: &str, length_beats: f32) -> Result<u32, McpBridgeError>;

    /// Delete a pattern by ID.
    fn delete_pattern(&self, pattern_id: u32) -> Result<(), McpBridgeError>;

    // === Sequencer: Notes ===

    /// List all notes in a pattern.
    fn list_notes(&self, pattern_id: u32) -> Result<Vec<NoteInfo>, McpBridgeError>;

    /// Add a note to a pattern. Returns the created note info.
    /// The note routes through its track's instrument at arrangement time.
    fn add_note(
        &self,
        pattern_id: u32,
        pitch: u8,
        start_beat: f32,
        duration_beats: f32,
        velocity: u8,
    ) -> Result<NoteInfo, McpBridgeError>;

    /// Remove a note from a pattern.
    fn remove_note(&self, pattern_id: u32, note_id: u64) -> Result<(), McpBridgeError>;

    /// Update a note's properties (only provided fields are changed). Returns updated note info.
    fn update_note(
        &self,
        pattern_id: u32,
        note_id: u64,
        pitch: Option<u8>,
        start_beat: Option<f32>,
        duration_beats: Option<f32>,
        velocity: Option<u8>,
    ) -> Result<NoteInfo, McpBridgeError>;

    // === Sequencer: Note processors (generative articulation rack) ===

    /// List a pattern's note-processor rack in execution order.
    fn list_note_processors(
        &self,
        pattern_id: u32,
    ) -> Result<Vec<NoteProcessorInfo>, McpBridgeError>;

    /// Add a note processor to a pattern's rack. `processor` is the
    /// externally-tagged `NoteProcessor` JSON (e.g.
    /// `{"Arpeggiator": {...}}`, `{"Chord": {...}}`); it is inserted at its
    /// canonical chain position. Returns the insertion index.
    fn add_note_processor(
        &self,
        pattern_id: u32,
        processor: serde_json::Value,
    ) -> Result<usize, McpBridgeError>;

    /// Replace the processor at `index` in place (config edit). `processor` is
    /// the same JSON shape `add_note_processor` accepts.
    fn set_note_processor(
        &self,
        pattern_id: u32,
        index: usize,
        processor: serde_json::Value,
    ) -> Result<(), McpBridgeError>;

    /// Remove the processor at `index` from a pattern's rack.
    fn remove_note_processor(&self, pattern_id: u32, index: usize) -> Result<(), McpBridgeError>;

    /// Bake the whole rack into concrete notes (Model-A freeze) and clear it.
    /// Returns the resulting note count.
    fn freeze_note_processors(&self, pattern_id: u32) -> Result<usize, McpBridgeError>;

    /// Set or clear a note's per-note timed-repeat ornament
    /// (flam/drag/ruff/roll/grace). `ornament` is the `Ornament` JSON to set,
    /// or `None`/`null` to clear it.
    fn set_note_ornament(
        &self,
        pattern_id: u32,
        note_id: u64,
        ornament: Option<serde_json::Value>,
    ) -> Result<(), McpBridgeError>;

    // === Sequencer: Tracks ===

    /// List all tracks in the song.
    fn list_tracks(&self) -> Result<Vec<TrackInfo>, McpBridgeError>;

    /// Create a new track. Returns the track ID.
    fn create_track(&self, name: &str, instrument_id: Option<u16>) -> Result<u16, McpBridgeError>;

    // === Sequencer: Arrangement ===

    /// Place a pattern on a track at a given beat position.
    fn place_pattern(
        &self,
        pattern_id: u32,
        track_id: u16,
        start_beat: f32,
    ) -> Result<(), McpBridgeError>;

    /// Remove a pattern placement.
    fn remove_placement(
        &self,
        pattern_id: u32,
        track_id: u16,
        start_beat: f32,
    ) -> Result<(), McpBridgeError>;

    /// List all pattern placements in the arrangement.
    fn list_arrangement(&self) -> Result<Vec<PlacementInfo>, McpBridgeError>;

    // === Sequencer: Transport ===

    /// Start sequencer playback.
    fn seq_play(&self) -> Result<(), McpBridgeError>;

    /// Stop sequencer playback.
    fn seq_stop(&self) -> Result<(), McpBridgeError>;

    /// Seek to a beat position.
    fn seq_seek(&self, beat: f32) -> Result<(), McpBridgeError>;

    // === Batch operations ===

    /// Add multiple notes to a pattern in one call.
    fn add_notes(
        &self,
        pattern_id: u32,
        notes: &[BridgeNoteData],
    ) -> Result<BatchResult, McpBridgeError>;

    /// Update multiple notes in a pattern in one call.
    fn update_notes(
        &self,
        pattern_id: u32,
        updates: &[BridgeNoteUpdate],
    ) -> Result<BatchResult, McpBridgeError>;

    /// Replace all notes in a pattern (clear + add).
    fn replace_notes(
        &self,
        pattern_id: u32,
        notes: &[BridgeNoteData],
    ) -> Result<BatchResult, McpBridgeError>;

    /// Clear all notes from a pattern. Returns the number of notes removed.
    fn clear_pattern(&self, pattern_id: u32) -> Result<usize, McpBridgeError>;

    /// Create multiple patterns (with optional inline notes).
    fn create_patterns(
        &self,
        patterns: &[BridgePatternData],
    ) -> Result<BatchResult, McpBridgeError>;

    /// Create multiple tracks.
    fn create_tracks(&self, tracks: &[BridgeTrackData]) -> Result<BatchResult, McpBridgeError>;

    /// Place multiple patterns in the arrangement.
    fn place_patterns(
        &self,
        placements: &[BridgePlacementData],
    ) -> Result<BatchResult, McpBridgeError>;

    /// Build a full song in one call (patterns + tracks + notes + arrangement).
    fn set_song(
        &self,
        name: &str,
        tempo: f32,
        patterns: &[BridgePatternData],
        tracks: &[BridgeTrackData],
        placements: &[BridgeSongPlacement],
    ) -> Result<SetSongResult, McpBridgeError>;

    // === Batch instrument building ===

    /// Build a complete instrument in one call: create instrument, add modules,
    /// set parameters, and wire connections.
    fn build_instrument(
        &self,
        spec: &BridgeInstrumentDef,
    ) -> Result<BuildInstrumentResult, McpBridgeError>;

    /// Build multiple instruments in one call.
    fn build_instruments(
        &self,
        specs: &[BridgeInstrumentDef],
    ) -> Result<Vec<BuildInstrumentResult>, McpBridgeError>;

    /// Rebuild an existing instrument's voice graph from `spec` while keeping
    /// its pattern automation pointed at the right modules. The instance
    /// counters are reset before the rebuild so the new graph numbers modules
    /// deterministically (1.. per type, in add order); wherever the module set
    /// is unchanged the ids match the old graph and the automation lanes stay
    /// valid. Lanes whose target module no longer exists are reported as
    /// orphaned and, if `drop_orphaned`, removed. Requires `spec.instrument_id`.
    /// The default impl errors — only bridges able to rebuild override it.
    fn rebuild_instrument_preserve_automation(
        &self,
        _spec: &BridgeInstrumentDef,
        _drop_orphaned: bool,
    ) -> Result<crate::types::RebuildInstrumentResult, McpBridgeError> {
        Err(McpBridgeError::Other(
            "instrument rebuild is not supported by this bridge".to_string(),
        ))
    }

    /// Apply a named example patch directly (bypassing GUI queue).
    /// If `instrument_id` is None, creates a new instrument.
    fn apply_example_patch(
        &self,
        instrument_id: Option<u64>,
        patch_name: &str,
    ) -> Result<ApplyExamplePatchResult, McpBridgeError>;

    // === Automation ===

    /// Add automation points to a pattern.
    fn add_automation_points(
        &self,
        pattern_id: u32,
        points: &[BridgeAutomationPointData],
    ) -> Result<BatchResult, McpBridgeError>;

    /// List all automation lanes in a pattern.
    fn list_automation_lanes(
        &self,
        pattern_id: u32,
    ) -> Result<Vec<AutomationLaneInfo>, McpBridgeError>;

    /// List the valid automation targets for an instrument: every automatable
    /// per-module parameter in its live graph (ready-to-use `module:` target
    /// strings) plus the instrument-level macros. Makes targets discoverable
    /// without reverse-engineering descriptor `type_id`s or instance indices.
    fn get_instrument_automation_targets(
        &self,
        instrument_id: u64,
    ) -> Result<Vec<AutomationTargetInfo>, McpBridgeError>;

    /// Get all automation points for a specific lane.
    fn get_automation_points(
        &self,
        pattern_id: u32,
        target: &str,
        instrument_id: u16,
    ) -> Result<Vec<AutomationPointInfo>, McpBridgeError>;

    /// Remove automation points at specific beats.
    fn remove_automation_points(
        &self,
        pattern_id: u32,
        target: &str,
        instrument_id: u16,
        beats: &[f32],
    ) -> Result<BatchResult, McpBridgeError>;

    /// Clear all points in an automation lane.
    fn clear_automation_lane(
        &self,
        pattern_id: u32,
        target: &str,
        instrument_id: u16,
    ) -> Result<usize, McpBridgeError>;

    /// Scale and/or offset every point's value in an existing automation lane,
    /// in place (tick + curve preserved). Each value becomes
    /// `clamp((value - pivot) * scale + pivot + offset, 0..1)`. Returns the
    /// number of points transformed. Errors if the lane does not exist.
    fn transform_automation_lane(
        &self,
        pattern_id: u32,
        target: &str,
        instrument_id: u16,
        scale: f32,
        pivot: f32,
        offset: f32,
    ) -> Result<usize, McpBridgeError>;

    /// Copy an automation lane's points to another pattern/target, optionally
    /// scaled (`clamp(value * scale + offset, 0..1)`); tick + curve preserved.
    /// When `clear_destination` is set the target lane is emptied first.
    /// Returns the number of points copied. Errors if the source lane is absent.
    #[allow(clippy::too_many_arguments)]
    fn copy_automation_lane(
        &self,
        from_pattern_id: u32,
        from_target: &str,
        from_instrument_id: u16,
        to_pattern_id: u32,
        to_target: &str,
        to_instrument_id: u16,
        scale: f32,
        offset: f32,
        clear_destination: bool,
    ) -> Result<usize, McpBridgeError>;

    /// Project-wide automation overview: every lane in every pattern, grouped
    /// by `"instrument"` (default), `"target"`, or `"pattern"`. Read-only.
    ///
    /// Default trait method composed from `list_patterns` + `list_automation_lanes`.
    fn get_automation_summary(
        &self,
        group_by: &str,
    ) -> Result<AutomationSummaryResult, McpBridgeError> {
        use std::collections::BTreeMap;

        let patterns = self.list_patterns()?;
        let mut all: Vec<AutomationSummaryLane> = Vec::new();
        for p in &patterns {
            for lane in self.list_automation_lanes(p.id)? {
                all.push(AutomationSummaryLane {
                    pattern_id: p.id,
                    pattern_name: p.name.clone(),
                    target: lane.target,
                    instrument_id: lane.instrument_id,
                    point_count: lane.point_count,
                });
            }
        }
        let total_lanes = all.len();
        let total_points: usize = all.iter().map(|l| l.point_count).sum();

        let mut grouped: BTreeMap<String, Vec<AutomationSummaryLane>> = BTreeMap::new();
        for lane in all {
            let key = match group_by {
                "target" => lane.target.clone(),
                "pattern" => format!("pattern {} ({})", lane.pattern_id, lane.pattern_name),
                // "instrument" and any other value fall back to instrument grouping.
                _ => lane.instrument_id.map_or_else(
                    || "(no instrument)".to_string(),
                    |id| format!("instrument {id}"),
                ),
            };
            grouped.entry(key).or_default().push(lane);
        }

        let groups = grouped
            .into_iter()
            .map(|(group, lanes)| AutomationSummaryGroup {
                group,
                lane_count: lanes.len(),
                point_count: lanes.iter().map(|l| l.point_count).sum(),
                lanes,
            })
            .collect();

        Ok(AutomationSummaryResult {
            group_by: group_by.to_string(),
            total_lanes,
            total_points,
            groups,
        })
    }

    // === Track control ===

    /// Set track volume (0.0-1.0).
    fn set_track_volume(&self, track_id: u16, volume: f32) -> Result<(), McpBridgeError>;

    /// Set track pan (-1.0 = left, 0.0 = center, 1.0 = right).
    fn set_track_pan(&self, track_id: u16, pan: f32) -> Result<(), McpBridgeError>;

    /// Mute or unmute a track.
    fn set_track_mute(&self, track_id: u16, muted: bool) -> Result<(), McpBridgeError>;

    /// Solo or unsolo a track.
    fn set_track_solo(&self, track_id: u16, solo: bool) -> Result<(), McpBridgeError>;

    /// Set the instrument assigned to a track (None to unassign).
    fn set_track_instrument(
        &self,
        track_id: u16,
        instrument_id: Option<u16>,
    ) -> Result<(), McpBridgeError>;

    /// Rename a track.
    fn rename_track(&self, track_id: u16, name: &str) -> Result<(), McpBridgeError>;

    /// Set a track's free-text description / intent (`""` clears it).
    fn set_track_description(&self, track_id: u16, description: &str)
    -> Result<(), McpBridgeError>;

    /// Set a track's display color from a `"#RRGGBB"` / `"#RRGGBBAA"` hex string.
    fn set_track_color(&self, track_id: u16, color: &str) -> Result<(), McpBridgeError>;

    /// Delete a track and its placements.
    fn delete_track(&self, track_id: u16) -> Result<(), McpBridgeError>;

    // === Return busses (effect sends) ===

    /// List all return busses.
    fn list_return_busses(&self) -> Result<Vec<ReturnBusInfo>, McpBridgeError>;

    /// Create a new return bus; returns its id.
    fn create_return_bus(&self, name: &str) -> Result<u16, McpBridgeError>;

    /// Delete a return bus and strip every track send that targeted it.
    fn delete_return_bus(&self, return_id: u16) -> Result<(), McpBridgeError>;

    /// Set a return bus's output volume (0.0-1.0).
    fn set_return_bus_volume(&self, return_id: u16, volume: f32) -> Result<(), McpBridgeError>;

    /// Set a return bus's output pan (-1.0 = left, 0.0 = center, 1.0 = right).
    fn set_return_bus_pan(&self, return_id: u16, pan: f32) -> Result<(), McpBridgeError>;

    /// Mute or unmute a return bus.
    fn set_return_bus_mute(&self, return_id: u16, muted: bool) -> Result<(), McpBridgeError>;

    /// Solo or unsolo a return bus. When any return is soloed, only soloed
    /// returns reach the master mix.
    fn set_return_bus_solo(&self, return_id: u16, solo: bool) -> Result<(), McpBridgeError>;

    /// Set a return bus's display color from a `"#RRGGBB"` / `"#RRGGBBAA"` hex string.
    fn set_return_bus_color(&self, return_id: u16, color: &str) -> Result<(), McpBridgeError>;

    /// Set a return bus's free-text description / intent (`""` clears it).
    fn set_return_bus_description(
        &self,
        return_id: u16,
        description: &str,
    ) -> Result<(), McpBridgeError>;

    /// Rename a return bus.
    fn rename_return_bus(&self, return_id: u16, name: &str) -> Result<(), McpBridgeError>;

    /// Add an insert effect to a return bus's effect chain. `effect_type` is a
    /// module-type token (e.g. "rev", "delay", "chorus"). Returns the created
    /// effect's module-id string (e.g. "rev-2"), which is unique within the bus.
    fn add_return_effect(
        &self,
        return_id: u16,
        effect_type: &str,
    ) -> Result<String, McpBridgeError>;

    /// Remove an insert effect from a return bus's effect chain by module id.
    fn remove_return_effect(&self, return_id: u16, module_id: &str) -> Result<(), McpBridgeError>;

    /// Set a parameter on a return-bus insert effect. `param_name` matches the
    /// effect descriptor's `type_id` or display name; `value` may be a number,
    /// boolean, or choice string. Returns the resolved parameter info.
    fn set_return_effect_parameter(
        &self,
        return_id: u16,
        module_id: &str,
        param_name: &str,
        value: BridgeParamValue,
    ) -> Result<ParameterInfo, McpBridgeError>;

    /// Enable or bypass a return-bus insert effect (`true` = active, `false` = bypassed).
    fn set_return_effect_enabled(
        &self,
        return_id: u16,
        module_id: &str,
        enabled: bool,
    ) -> Result<(), McpBridgeError>;

    /// Move a return-bus insert effect one slot up or down in the chain.
    fn reorder_return_effect(
        &self,
        return_id: u16,
        module_id: &str,
        direction: ReturnEffectMove,
    ) -> Result<(), McpBridgeError>;

    /// Add or update a track's send to a return bus (upsert by target).
    /// `enabled = false` bypasses the send non-destructively (keeps its level/tap).
    fn set_track_send(
        &self,
        track_id: u16,
        return_id: u16,
        level: f32,
        pre_fader: bool,
        enabled: bool,
    ) -> Result<(), McpBridgeError>;

    /// Remove a track's send to a return bus.
    fn remove_track_send(&self, track_id: u16, return_id: u16) -> Result<(), McpBridgeError>;

    /// Add or update a bus-to-bus send from one return bus into another (upsert
    /// by target). Rejected if it would create a routing cycle. `enabled = false`
    /// bypasses the send non-destructively.
    fn set_return_send(
        &self,
        from_id: u16,
        to_id: u16,
        level: f32,
        enabled: bool,
    ) -> Result<(), McpBridgeError>;

    /// Remove a bus-to-bus send from one return bus into another.
    fn remove_return_send(&self, from_id: u16, to_id: u16) -> Result<(), McpBridgeError>;

    // === Master bus ===

    /// Read the master output volume (0.0 = silent, 1.0 = unity; may exceed 1.0).
    fn get_master_volume(&self) -> Result<f32, McpBridgeError>;

    /// Set the master output volume (0.0 = silent, 1.0 = unity).
    fn set_master_volume(&self, volume: f32) -> Result<(), McpBridgeError>;

    /// List the master-bus insert effects in processing order.
    fn list_master_effects(&self) -> Result<Vec<ReturnEffectInfo>, McpBridgeError>;

    /// Add an insert effect to the master-bus effect chain. `effect_type` is a
    /// module-type token (e.g. "limiter", "eq"). Returns the new module-id string.
    fn add_master_effect(&self, effect_type: &str) -> Result<String, McpBridgeError>;

    /// Remove an insert effect from the master-bus effect chain by module id.
    fn remove_master_effect(&self, module_id: &str) -> Result<(), McpBridgeError>;

    /// Set a parameter on a master-bus insert effect. Returns the resolved info.
    fn set_master_effect_parameter(
        &self,
        module_id: &str,
        param_name: &str,
        value: BridgeParamValue,
    ) -> Result<ParameterInfo, McpBridgeError>;

    /// Enable or bypass a master-bus insert effect.
    fn set_master_effect_enabled(
        &self,
        module_id: &str,
        enabled: bool,
    ) -> Result<(), McpBridgeError>;

    /// Move a master-bus insert effect one slot up or down in the chain.
    fn reorder_master_effect(
        &self,
        module_id: &str,
        direction: ReturnEffectMove,
    ) -> Result<(), McpBridgeError>;

    // === Pattern management ===

    /// Rename a pattern.
    fn rename_pattern(&self, pattern_id: u32, name: &str) -> Result<(), McpBridgeError>;

    /// Set a pattern's free-text description / intent (`""` clears it).
    fn set_pattern_description(
        &self,
        pattern_id: u32,
        description: &str,
    ) -> Result<(), McpBridgeError>;

    /// Set pattern length in beats.
    fn set_pattern_length(&self, pattern_id: u32, length_beats: f32) -> Result<(), McpBridgeError>;

    /// Duplicate a pattern (notes + automation). Returns new pattern ID.
    fn duplicate_pattern(&self, pattern_id: u32) -> Result<u32, McpBridgeError>;

    // === Song metadata ===

    /// Set the song author.
    fn set_song_author(&self, author: &str) -> Result<(), McpBridgeError>;

    /// Set the song's free-text description / intent (`""` clears it).
    fn set_song_description(&self, description: &str) -> Result<(), McpBridgeError>;

    /// Set the song time signature.
    fn set_song_time_signature(&self, numerator: u8, denominator: u8)
    -> Result<(), McpBridgeError>;

    // === Batch parameter set ===

    /// Set multiple parameters at once.
    fn set_parameters(
        &self,
        instrument_id: u64,
        params: &[BridgeParamSet],
    ) -> Result<BatchResult, McpBridgeError>;

    // === Project management ===

    /// Reset to a new empty project.
    fn new_project(&self) -> Result<String, McpBridgeError>;

    /// Save the current project to a file.
    ///
    /// **Warning:** Performs file I/O. Must not be called from the audio thread.
    fn save_project(&self, path: &str) -> Result<String, McpBridgeError>;

    /// Save a single instrument as a standalone patch file.
    ///
    /// **Warning:** Performs file I/O. Must not be called from the audio thread.
    fn save_patch(&self, instrument_id: u64, path: &str) -> Result<String, McpBridgeError>;

    /// Load a project from a file.
    ///
    /// **Warning:** Performs file I/O. Must not be called from the audio thread.
    fn load_project(&self, path: &str) -> Result<String, McpBridgeError>;

    /// Optimize the project by removing unused patterns, tracks, and instruments.
    fn optimize_project(&self) -> Result<OptimizeResult, McpBridgeError>;

    // === Audio preview ===

    /// Render a short audio preview of a note played on the given instrument.
    ///
    /// Creates an offline engine, loads the instrument's current patch,
    /// plays the note, and returns WAV data.
    fn render_note_preview(
        &self,
        instrument_id: u64,
        note: u8,
        velocity: u8,
        duration_ms: u32,
        tail_ms: u32,
    ) -> Result<AudioPreview, McpBridgeError>;

    /// Render a note offline and analyze the resulting f32 audio buffer.
    ///
    /// Same render path as `render_note_preview` (octave offset applied,
    /// engine snapshot, etc.) but skips the WAV encoding and instead returns
    /// quantitative metrics: fundamental frequency, peak/RMS, DC offset,
    /// clip count, RMS and centroid envelopes over time, and top spectral
    /// peaks at attack / sustain / release.
    ///
    /// `expected_note` (when `Some`) anchors `expected_fundamental_hz` to that
    /// MIDI note instead of the actually-played one and narrows the
    /// fundamental search to ±tritone around it. Useful when the loudest
    /// spectral peak isn't the fundamental (sub-octave dominance, wave folding).
    ///
    /// `envelope_window_ms` (when `Some`) sets the block size of the RMS /
    /// centroid envelopes; defaults to 50 ms. Lower it (e.g. to 2–5 ms) to
    /// resolve fast attacks that a 50 ms window collapses into a single frame.
    #[allow(clippy::too_many_arguments)]
    fn analyze_note(
        &self,
        instrument_id: u64,
        note: u8,
        velocity: u8,
        duration_ms: u32,
        tail_ms: u32,
        expected_note: Option<u8>,
        envelope_window_ms: Option<f32>,
    ) -> Result<crate::types::AnalyzeNoteResult, McpBridgeError>;

    /// Symbolic harmonic analysis of a pattern or arrangement range.
    ///
    /// Walks notes in time order, groups overlapping notes into chord events
    /// on a configurable resolution, labels each event with a chord symbol
    /// when one matches a known template, and infers the most likely key.
    ///
    /// Scope is selected by which fields are `Some`:
    /// - `pattern_id = Some(p)` analyzes a single pattern (other args ignored).
    /// - otherwise analyzes the arrangement range
    ///   `[arrangement_start_tick, arrangement_end_tick)` across all tracks.
    ///   When both are `None`, the full arrangement is analyzed.
    ///
    /// `grouping_ticks` is the chord-detection resolution in ticks (default 960
    /// = one quarter note at the engine's PPQN).
    ///
    /// `exclude_drums` (default `true`) drops notes from tracks whose
    /// instrument has category `Drums`, so percussion MIDI pitches don't
    /// pollute chord identification. `exclude_track_ids` is an explicit list
    /// of track IDs to drop in addition. Both apply only to arrangement scope.
    fn analyze_harmony(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        grouping_ticks: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<crate::types::AnalyzeHarmonyResult, McpBridgeError>;

    /// Symbolic structural analysis of a single pattern.
    ///
    /// Reads the pattern's notes directly — no audio rendering, no engine
    /// snapshot needed. Reports density (notes per bar/beat, active ratio),
    /// pitch shape (range, mean, distinct count, duration-weighted pitch
    /// class histogram), velocity dynamics (min/max/mean/std/range),
    /// rhythmic structure (max/mean polyphony, distinct onsets/durations,
    /// inter-onset-interval stats, regularity score), and bar-level
    /// repetition.
    ///
    /// `length_bars` / `notes_per_bar` use the song's default time signature.
    fn analyze_pattern(
        &self,
        pattern_id: u32,
    ) -> Result<crate::types::AnalyzePatternResult, McpBridgeError>;

    /// Symbolic drum-feel diagnostics. Identifies drum tracks via the same
    /// `infer_all_profiles` path that `analyze_harmony`'s `exclude_drums`
    /// default uses, classifies each hit (kick / snare / hat / tom / cymbal /
    /// clap / other) via the General MIDI drum map, and reports backbeat
    /// strength, hat subdivision, ghost-note count, fill candidates, and
    /// bar-level repetition. Pure symbolic — no audio rendering.
    ///
    /// Scope is selected by which fields are `Some`:
    /// - `pattern_id = Some(p)` analyzes one pattern's notes (no drum-track
    ///   filtering — assumes the pattern is a drum pattern).
    /// - otherwise analyzes drum tracks in the arrangement range
    ///   `[arrangement_start_tick, arrangement_end_tick)`. When both are
    ///   `None`, the full arrangement is analyzed.
    fn analyze_drum_groove(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
    ) -> Result<crate::types::AnalyzeDrumGrooveResult, McpBridgeError>;

    /// Symbolic kick/bass-lock diagnostics. Pure symbolic — no audio
    /// rendering. In arrangement scope, identifies drum tracks (Role::Drums,
    /// confidence ≥ 0.6) and bass tracks (Role::Bass, confidence ≥ 0.6) via
    /// `infer_all_profiles`, then aligns kick onsets (GM MIDI 35/36) against
    /// bass note onsets within a tolerance and reports lock_score,
    /// coverage_score, kick-only / bass-only counts, and a bass-pitch
    /// stability summary.
    ///
    /// `onset_tolerance_ticks` defaults to 120 (±1/32-note at 960 PPQN);
    /// clamped to `[30, 960]`.
    ///
    /// In pattern scope, the analyzer treats every note with a kick-mapped
    /// MIDI number as a kick and every other note as bass — useful for
    /// rhythm-section patterns that are stored as a single combined pattern.
    fn analyze_bass_drum_lock(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        onset_tolerance_ticks: Option<u32>,
    ) -> Result<crate::types::AnalyzeBassDrumLockResult, McpBridgeError>;

    /// Tonal-function analysis on top of `analyze_harmony`. Runs the same
    /// chord-identification + key-inference pipeline, then annotates each
    /// chord event with a scale-degree Roman numeral, a function bucket
    /// (Tonic / Subdominant / Dominant / Other / Chromatic), and a 0..1
    /// tension score, and detects cadences (Authentic V → I, Plagal IV → I,
    /// Half — anything → V, Deceptive V → vi) on consecutive chord pairs.
    ///
    /// Parameters mirror `analyze_harmony`: pattern scope when
    /// `pattern_id = Some(p)`, arrangement range otherwise.
    fn analyze_harmonic_function(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        grouping_ticks: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<crate::types::AnalyzeHarmonicFunctionResult, McpBridgeError>;

    /// Section-level form diagnostic. Walks the analyzed scope one bar at a
    /// time, builds a per-bar feature vector (duration-weighted pitch-class
    /// histogram + density + mean velocity + active tracks), computes a
    /// cosine self-similarity matrix, merges adjacent similar bars into
    /// sections, and labels sections by first appearance (`"A"`, `"B"`,
    /// `"A'"` for near-matches). Returns per-bar features + per-section
    /// stats. Pure symbolic.
    #[allow(clippy::too_many_arguments)]
    fn analyze_arrangement(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        similarity_threshold: Option<f32>,
        section_min_bars: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<crate::types::AnalyzeArrangementResult, McpBridgeError>;

    /// Compact view of the same clustering as `analyze_arrangement`: one
    /// label per bar plus a run-length-compressed `form_string` like
    /// `"AABA"`. Same default thresholds.
    #[allow(clippy::too_many_arguments)]
    fn analyze_form_map(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        similarity_threshold: Option<f32>,
        section_min_bars: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<crate::types::AnalyzeFormMapResult, McpBridgeError>;

    /// Find recurring pitch-interval motifs in the scope. Converts each
    /// track's notes into signed semitone deltas, slides an n-gram window
    /// (lengths `min_interval_length..=max_interval_length`, defaults `3..=6`),
    /// and returns the top-N entries that appear at least `min_count` times
    /// (default 3). Transposition-invariant. `top_n` defaults to 10.
    #[allow(clippy::too_many_arguments)]
    fn find_motifs(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        min_interval_length: Option<u8>,
        max_interval_length: Option<u8>,
        min_count: Option<u32>,
        top_n: Option<u32>,
        max_occurrences_per_motif: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<crate::types::FindMotifsResult, McpBridgeError>;

    /// Hook-strength diagnostic. Runs the motif finder internally and
    /// reduces the result to a single `hook_score` in `[0, 1]` plus the
    /// strongest motif found.
    #[allow(clippy::too_many_arguments)]
    fn analyze_hook_strength(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        min_interval_length: Option<u8>,
        min_count: Option<u32>,
        max_occurrences_per_motif: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<crate::types::AnalyzeHookStrengthResult, McpBridgeError>;

    /// Render `duration_seconds` of the master bus offline and return
    /// mix-level metrics (LUFS-I, true peak proxy, RMS, crest factor, banded
    /// energy, stereo correlation, mid/side, mono-compatibility).
    ///
    /// Rendering starts at `start_tick` (defaults to 0). Duration is clamped
    /// by the renderer to an internal upper bound.
    /// When `include_per_track` is `Some(true)`, the bridge does N additional
    /// soloed renders (one per audible track that overlaps the rendered window)
    /// and returns a `TrackContribution` for each — same semantics as
    /// `analyze_section`. Cost scales linearly with the track count.
    fn analyze_mix_bus(
        &self,
        duration_seconds: f32,
        start_tick: Option<u64>,
        include_per_track: Option<bool>,
        scope: AnalysisScope,
    ) -> Result<crate::types::AnalyzeMixBusResult, McpBridgeError>;

    /// Render `duration_seconds` of the arrangement offline and write it to a
    /// 32-bit float WAV file at `path`, returning where it landed plus basic
    /// stats (sample rate, frame count, peak). Deterministic and offline, the
    /// same render `analyze_mix_bus` uses.
    ///
    /// When `instrument_id` is `Some`, only that instrument's tracks are soloed
    /// so the file contains a single instrument's contribution (a clean
    /// fingerprint for external spectral matching); `None` writes the full mix.
    /// `scope` selects which optional signal stages (master/return effects, AWE)
    /// the render includes, exactly as for `analyze_mix_bus`.
    fn render_to_wav(
        &self,
        path: String,
        duration_seconds: f32,
        start_tick: Option<u64>,
        instrument_id: Option<u16>,
        scope: AnalysisScope,
    ) -> Result<crate::types::RenderToWavResult, McpBridgeError>;

    /// Render `duration_seconds` of the arrangement offline and return a
    /// detailed spectrum: detected partials (frequency + amplitude + harmonic
    /// tagging), a voiced/unvoiced verdict, and timbre descriptors (centroid,
    /// flatness, rolloff, inharmonicity, odd/even ratio). Deterministic and
    /// offline, the same render `analyze_mix_bus` uses.
    ///
    /// `instrument_id` solos one instrument (clean single-source fingerprint);
    /// `f0_hint` sharpens the harmonic tagging; `max_partials` caps the partial
    /// list; `log_bins` (> 0) adds log-spaced magnitude bins for
    /// `compare_spectra`. `scope` selects the optional signal stages.
    #[allow(clippy::too_many_arguments)]
    fn analyze_spectrum(
        &self,
        duration_seconds: f32,
        start_tick: Option<u64>,
        instrument_id: Option<u16>,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        scope: AnalysisScope,
    ) -> Result<crate::types::AnalyzeSpectrumResult, McpBridgeError>;

    /// Like `analyze_spectrum` but returns a *spectrogram*: the requested window
    /// is rendered **once** and a sliding FFT produces one spectrum per `hop_ms`
    /// (analysing `window_len_ms` per frame), so the time evolution of a sound is
    /// visible in a single O(1)-render call. The per-frame `voiced` flag reads a
    /// source's pitched/noise alternation directly.
    #[allow(clippy::too_many_arguments)]
    fn analyze_spectrogram(
        &self,
        duration_seconds: f32,
        start_tick: Option<u64>,
        instrument_id: Option<u16>,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        hop_ms: Option<f32>,
        window_len_ms: Option<f32>,
        scope: AnalysisScope,
    ) -> Result<crate::types::AnalyzeSpectrogramResult, McpBridgeError>;

    /// Same detailed spectral analysis as `analyze_spectrum`, but over an
    /// imported sample or a WAV file on disk instead of a render — so a real
    /// reference recording (e.g. a SID render) can be fingerprinted in identical
    /// units and compared against a candidate patch.
    ///
    /// `sample_id_or_path` is either a numeric imported-sample id or a
    /// filesystem path to a WAV file.
    fn analyze_sample_spectrum(
        &self,
        sample_id_or_path: String,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        start_ms: Option<f32>,
        window_len_ms: Option<f32>,
    ) -> Result<crate::types::AnalyzeSampleSpectrumResult, McpBridgeError>;

    /// Per-frame spectrogram of an imported sample or WAV file, analysed at the
    /// file's NATIVE sample rate — the sample counterpart of `analyze_spectrogram`,
    /// so the time evolution of a real reference recording is visible in one call.
    fn analyze_sample_spectrogram(
        &self,
        sample_id_or_path: String,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        hop_ms: Option<f32>,
        window_len_ms: Option<f32>,
    ) -> Result<crate::types::AnalyzeSampleSpectrogramResult, McpBridgeError>;

    /// Compare two spectra (each a render or a sample) and return a distance plus
    /// a per-partial diff. `target` is the reference (e.g. a real SID render);
    /// `candidate` is the attempt (e.g. your patch). `missing_partials` names the
    /// partials strong in the target that the candidate lacks — the actionable
    /// guidance for a timbre-matching loop. Both sources are analysed with the
    /// same `f0_hint`/`max_partials`/`log_bins`/`mel_bands`; render sources use
    /// `scope`. `mel_bands` (default 40) sizes the log-mel filterbank behind
    /// `mel_l2_distance`. `time_resolved` optionally adds a per-frame masked
    /// distance (see [`TimeResolvedOptions`]) for time-sparse material the
    /// aggregate scalar averages flat.
    #[allow(clippy::too_many_arguments)]
    fn compare_spectra(
        &self,
        target: SpectrumSource,
        candidate: SpectrumSource,
        f0_hint: Option<f32>,
        max_partials: Option<u32>,
        log_bins: Option<u32>,
        mel_bands: Option<u32>,
        scope: AnalysisScope,
        time_resolved: TimeResolvedOptions,
    ) -> Result<crate::types::CompareSpectraResult, McpBridgeError>;

    /// Compare the amplitude *contours* (ADSR shape over time) of two sources —
    /// the time-domain counterpart of `compare_spectra`. Extracts an RMS
    /// envelope from each (render or sample/WAV), aligns them with dynamic time
    /// warping for a shape distance, and reports the per-side ADSR estimate plus
    /// attack transient. `envelope_window_ms` sets the RMS block size;
    /// `note_duration_ms` (applied to both) marks note-off for the release
    /// estimate; `transient_window_ms` sets the attack-transient span. Render
    /// sources use `scope`.
    #[allow(clippy::too_many_arguments)]
    fn compare_envelopes(
        &self,
        target: SpectrumSource,
        candidate: SpectrumSource,
        envelope_window_ms: Option<f32>,
        note_duration_ms: Option<u32>,
        transient_window_ms: Option<f32>,
        scope: AnalysisScope,
    ) -> Result<crate::types::CompareEnvelopesResult, McpBridgeError>;

    /// Incremental per-effect breakdown of the master bus. Renders the chain
    /// input (post-return mix, no master effects) once, then re-renders the
    /// master output with the chain truncated after each effect, so each
    /// effect's contribution (loudness/peak/width/dynamics delta) is isolated.
    /// The master effect chain is always reconstructed regardless of `scope`;
    /// `scope` only controls whether the return-bus wet signal feeds the chain
    /// input, the render sample rate, and AWE. Cost is O(effect_count) renders.
    fn analyze_master_chain(
        &self,
        duration_seconds: f32,
        start_tick: Option<u64>,
        scope: AnalysisScope,
    ) -> Result<crate::types::AnalyzeMasterChainResult, McpBridgeError>;

    /// Per-return-bus contribution to the master mix. Renders the full mix once,
    /// then re-renders with each return bus muted in turn; the deltas report
    /// what each return adds (loudness/peak/width). Return-bus effect chains are
    /// always reconstructed regardless of `scope`; `scope` only controls whether
    /// the master effect chain processes the sum, the render sample rate, and
    /// AWE. Cost is O(return_count) renders.
    fn analyze_return_busses(
        &self,
        duration_seconds: f32,
        start_tick: Option<u64>,
        scope: AnalysisScope,
    ) -> Result<crate::types::AnalyzeReturnBussesResult, McpBridgeError>;

    /// Capture or compare a mix-bus baseline. `action = "capture"` renders the
    /// mix and stores its metrics (plus the render window + scope) in-session;
    /// `action = "compare"` re-renders with the stored settings and returns the
    /// `current − baseline` deltas. Lets a caller A/B a change ("did this make
    /// the mix louder / peakier / wider?"). The baseline is transient — it
    /// lives only for the session and is never written to the project.
    fn compare_mix_before_after(
        &self,
        action: &str,
        duration_seconds: f32,
        start_tick: Option<u64>,
        label: Option<String>,
        scope: AnalysisScope,
    ) -> Result<crate::types::CompareMixResult, McpBridgeError>;

    /// Capture the current project state (instruments, modules, connections,
    /// effects, song — whatever is reflected in the latest published engine
    /// state) into an internal slot for a later
    /// [`restore_snapshot`](Self::restore_snapshot). Used by `batch_execute`
    /// rollback. Errors if a snapshot is already pending (concurrent rollback
    /// batches are unsupported). The default impl errors — only bridges able to
    /// snapshot the project override it.
    fn capture_snapshot(&self) -> Result<(), McpBridgeError> {
        Err(McpBridgeError::Other(
            "project snapshots are not supported by this bridge".to_string(),
        ))
    }

    /// Restore the project from the snapshot taken by
    /// [`capture_snapshot`](Self::capture_snapshot), clearing the slot. Errors
    /// if no snapshot was captured.
    fn restore_snapshot(&self) -> Result<(), McpBridgeError> {
        Err(McpBridgeError::Other(
            "project snapshots are not supported by this bridge".to_string(),
        ))
    }

    /// Discard any captured snapshot without restoring. No-op if none exists.
    fn clear_snapshot(&self) {}

    /// Measure the master mix (post master + return effects) and adjust the
    /// master fader toward `target_lufs` without pushing the true peak above
    /// `true_peak_ceiling_dbtp`. The master fader is post-effects, so the
    /// adjustment scales loudness and peak linearly — one render suffices, no
    /// iteration. Mutates the project's master volume.
    fn auto_gain_stage(
        &self,
        target_lufs: f32,
        true_peak_ceiling_dbtp: f32,
        duration_seconds: f32,
        start_tick: Option<u64>,
    ) -> Result<AutoGainStageResult, McpBridgeError>;

    /// Render an explicit arrangement range `[start_tick, end_tick)` offline
    /// and return the same metrics as `analyze_mix_bus`.
    ///
    /// When `include_per_track` is `Some(true)`, the bridge does N additional
    /// soloed renders (one per audible track that overlaps the range) and
    /// returns a `TrackContribution` for each so the caller can see which
    /// track owns which share of the mix's energy. Cost scales linearly with
    /// the track count of the section.
    fn analyze_section(
        &self,
        start_tick: u64,
        end_tick: u64,
        include_per_track: Option<bool>,
        scope: AnalysisScope,
    ) -> Result<crate::types::AnalyzeSectionResult, McpBridgeError>;

    /// Pairwise spectral-masking report for every audible track that
    /// overlaps `[start_tick, end_tick)`. Renders each audible track soloed
    /// (same renders as `analyze_section` with `include_per_track = true`)
    /// and computes per-band overlap on the resulting buffers — no extra
    /// renders are needed beyond the per-track set. Pairs are sorted by
    /// descending conflict score so the most contested combination appears
    /// first.
    fn analyze_masking_matrix(
        &self,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        top_pairs: Option<u32>,
        scope: AnalysisScope,
    ) -> Result<crate::types::AnalyzeMaskingMatrixResult, McpBridgeError>;

    /// Sweep an instrument across a MIDI note range, render-and-analyze each
    /// step via the same path as `analyze_note`, and return per-note metrics
    /// plus cross-step issues (silent notes, likely aliasing, lost pitch
    /// tracking, clipping, level spread). Use this to spot patches that work
    /// at C4 but fall apart at C6 or C2 before committing them. `step_semitones`
    /// defaults to 12 (one note per octave); reduce for higher resolution.
    #[allow(clippy::too_many_arguments)]
    fn analyze_instrument_range(
        &self,
        instrument_id: u64,
        low_note: u8,
        high_note: u8,
        step_semitones: Option<u8>,
        velocity: Option<u8>,
        duration_ms: Option<u32>,
        tail_ms: Option<u32>,
    ) -> Result<crate::types::AnalyzeInstrumentRangeResult, McpBridgeError>;

    /// Hold one MIDI note and sweep velocity across `[velocity_low,
    /// velocity_high]`. Returns per-velocity amplitude and brightness curves
    /// plus monotonicity / responsiveness flags — confirms the patch responds
    /// musically to velocity (or flags it as effectively velocity-deaf).
    /// `velocity_step` defaults to 16.
    #[allow(clippy::too_many_arguments)]
    fn analyze_velocity_response(
        &self,
        instrument_id: u64,
        note: u8,
        velocity_low: Option<u8>,
        velocity_high: Option<u8>,
        velocity_step: Option<u8>,
        duration_ms: Option<u32>,
        tail_ms: Option<u32>,
    ) -> Result<crate::types::AnalyzeVelocityResponseResult, McpBridgeError>;

    /// Bar-level tension curve over the analyzed scope. Builds per-bar
    /// rows from existing analyzers — harmonic_function for per-chord
    /// tension, bar_features for density/register/rhythmic activity,
    /// optionally a single offline render sliced per bar for loudness,
    /// brightness, band entropy, and stereo width.
    ///
    /// Returns per-bar values plus peak/trough/mean/std-dev summary, the
    /// section labels from `analyze_arrangement`, and warnings for
    /// chorus-doesn't-lift, build-peaks-too-early, drop-loses-low-end,
    /// and monotone-tension shape problems.
    ///
    /// `include_audio` defaults to true in arrangement scope and false in
    /// pattern scope; set explicitly to override. Audio mode does one
    /// full-range render of the scope and slices it per bar — cost is
    /// roughly equivalent to one `analyze_section` call.
    #[allow(clippy::too_many_arguments)]
    fn analyze_tension_curve(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        include_audio: Option<bool>,
        similarity_threshold: Option<f32>,
        section_min_bars: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<crate::types::AnalyzeTensionCurveResult, McpBridgeError>;

    /// Meta-analysis: runs the relevant analyzers over the scope, applies
    /// a rule set per category, and returns ranked fix suggestions with
    /// supporting evidence. Adds no new measurements — every suggestion
    /// references metrics already produced by harmony, harmonic_function,
    /// mix-bus, masking, drum-groove, bass-drum-lock, form, motifs / hook,
    /// tension-curve, or instrument-profile analyzers.
    ///
    /// `categories` filters which rule families run (`"harmony"`,
    /// `"mix"`, `"groove"`, `"arrangement"`, `"composition"`, `"patch"`).
    /// Empty list / `None` runs everything. `include_audio` mirrors
    /// `analyze_tension_curve` — set to false to skip the mix-bus /
    /// masking / audio-augmented tension-curve checks. `max_suggestions`
    /// defaults to 15.
    #[allow(clippy::too_many_arguments)]
    fn suggest_music_fixes(
        &self,
        pattern_id: Option<u32>,
        arrangement_start_tick: Option<u64>,
        arrangement_end_tick: Option<u64>,
        categories: Option<Vec<String>>,
        include_audio: Option<bool>,
        max_suggestions: Option<u32>,
        exclude_drums: Option<bool>,
        exclude_track_ids: Option<Vec<u16>>,
    ) -> Result<crate::types::SuggestMusicFixesResult, McpBridgeError>;

    // === Composition helpers (symbolic — no audio rendering) ===

    /// Parse a chord symbol (e.g. `"Cm7"`, `"F#maj7"`, `"Bbsus4"`) and
    /// return the MIDI notes for the requested voicing rooted at `octave`
    /// (scientific pitch notation, octave 4 = middle-C). `voicing` is one
    /// of `"close"`, `"drop2"`, `"drop3"`, `"open"` (default `"close"`).
    /// Pure symbolic — does not touch the song; the caller plays or places
    /// the returned notes with `note_on` / `add_note`.
    fn generate_chord(
        &self,
        symbol: &str,
        octave: i32,
        voicing: Option<&str>,
    ) -> Result<crate::types::GenerateChordResult, McpBridgeError>;

    /// Create a new pattern and fill it with a chord progression: each symbol
    /// in `chords` is voiced via [`generate_chord`] and placed as a block of
    /// notes spanning `beats_per_chord`, laid end to end. Returns the new
    /// pattern id and a per-chord breakdown.
    ///
    /// Default trait method composed from `create_pattern` + `generate_chord` +
    /// `add_notes`.
    fn create_chord_progression_pattern(
        &self,
        name: &str,
        chords: &[String],
        beats_per_chord: f32,
        octave: i32,
        voicing: Option<&str>,
        velocity: u8,
    ) -> Result<CreateChordProgressionResult, McpBridgeError> {
        if chords.is_empty() {
            return Err(McpBridgeError::Other(
                "progression has no chords".to_string(),
            ));
        }
        if !(beats_per_chord.is_finite() && beats_per_chord > 0.0) {
            return Err(McpBridgeError::Other(
                "beats_per_chord must be a positive number".to_string(),
            ));
        }

        let length_beats = chords.len() as f32 * beats_per_chord;
        let pattern_id = self.create_pattern(name, length_beats)?;

        let mut notes: Vec<BridgeNoteData> = Vec::new();
        let mut steps: Vec<ChordProgressionStep> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        for (i, symbol) in chords.iter().enumerate() {
            let start_beat = i as f32 * beats_per_chord;
            let chord = self.generate_chord(symbol, octave, voicing)?;
            for w in &chord.warnings {
                warnings.push(format!("{symbol}: {w}"));
            }
            for &pitch in &chord.notes {
                notes.push(BridgeNoteData {
                    pitch,
                    start_beat,
                    duration_beats: beats_per_chord,
                    velocity,
                    legato: false,
                    glide: None,
                    expression: None,
                });
            }
            steps.push(ChordProgressionStep {
                symbol: chord.symbol,
                start_beat,
                quality: chord.quality,
                voicing: chord.voicing,
                notes: chord.notes,
            });
        }

        let added = self.add_notes(pattern_id, &notes)?;
        Ok(CreateChordProgressionResult {
            pattern_id,
            length_beats,
            notes_added: added.succeeded,
            chords: steps,
            warnings,
        })
    }

    /// Transpose every note in `pattern_id` by `semitones` (signed). When
    /// both `scale_tonic` and `scale_name` are set, any note whose
    /// transposed pitch lands outside that scale is snapped to the nearest
    /// in-scale pitch using `tie_break` (`"up"` / `"down"` / `"nearest"`,
    /// default `"up"`).
    fn transpose_notes(
        &self,
        pattern_id: u32,
        semitones: i32,
        scale_tonic: Option<u8>,
        scale_name: Option<&str>,
        tie_break: Option<&str>,
    ) -> Result<crate::types::TransposeNotesResult, McpBridgeError>;

    /// Snap every note in `pattern_id` to the nearest pitch of the given
    /// key/scale. Notes already in scale are left untouched.
    fn quantize_notes_to_scale(
        &self,
        pattern_id: u32,
        scale_tonic: u8,
        scale_name: &str,
        tie_break: Option<&str>,
    ) -> Result<crate::types::QuantizeNotesToScaleResult, McpBridgeError>;

    /// Snap note start ticks in `pattern_id` to the requested `grid_ticks`
    /// grid with optional swing (`0..=1`), humanization (max ±tick jitter
    /// per note, seeded for reproducibility), and quantize strength
    /// (`0..=1`, where `1.0` is full snap, `0.5` is half-way).
    #[allow(clippy::too_many_arguments)]
    fn quantize_notes_to_grid(
        &self,
        pattern_id: u32,
        grid_ticks: u32,
        strength: Option<f32>,
        swing: Option<f32>,
        humanize_ticks: Option<u32>,
        humanize_seed: Option<u64>,
    ) -> Result<crate::types::QuantizeNotesToGridResult, McpBridgeError>;

    // === AWE (Acoustic World Engine) ===

    /// Get the current AWE state (room, material, all parameters, LFOs).
    fn get_awe_state(&self) -> Result<AweStateInfo, McpBridgeError>;

    /// Enable or disable the AWE engine.
    fn set_awe_enabled(&self, enabled: bool) -> Result<(), McpBridgeError>;

    /// Set or clear the AWE state's free-text description (the acoustic
    /// character of the current room / preset). Pass `""` to clear.
    fn set_awe_description(&self, description: &str) -> Result<(), McpBridgeError>;

    /// Set a single AWE parameter by name. Value interpretation depends on the parameter.
    fn set_awe_parameter(&self, name: &str, value: f64) -> Result<(), McpBridgeError>;

    /// Set the room shape. `dimensions` carries the shape-specific values
    /// (length, width, height, radius, …) in meters.
    fn set_awe_room_shape(
        &self,
        shape: synth_awe::RoomShapeKind,
        dimensions: &[f32],
    ) -> Result<(), McpBridgeError>;

    /// Set the wall material.
    fn set_awe_material(&self, material: synth_awe::MaterialKind) -> Result<(), McpBridgeError>;

    /// Load a named AWE preset. Returns the resulting state.
    fn set_awe_preset(&self, name: &str) -> Result<AweStateInfo, McpBridgeError>;

    /// List all available AWE presets.
    fn list_awe_presets(&self) -> Result<Vec<AwePresetInfo>, McpBridgeError>;

    /// Configure an AWE LFO (1-4).
    fn set_awe_lfo(
        &self,
        index: u8,
        rate: f32,
        amount: f32,
        target: synth_awe::AweLfoTarget,
    ) -> Result<(), McpBridgeError>;

    // === Sample library ===

    /// List all samples, optionally filtered by name substring.
    fn list_samples(&self, filter: Option<&str>) -> Result<Vec<SampleInfo>, McpBridgeError>;

    /// Import a WAV file into the sample library.
    fn import_sample(
        &self,
        path: &str,
        name: Option<&str>,
        root_note: Option<u8>,
    ) -> Result<SampleInfo, McpBridgeError>;

    /// Delete a sample by ID.
    fn delete_sample(&self, id: u64) -> Result<(), McpBridgeError>;

    /// Rename a sample.
    fn rename_sample(&self, id: u64, name: &str) -> Result<(), McpBridgeError>;

    /// Set a sample's free-text description / intent (`""` clears it).
    fn set_sample_description(&self, id: u64, description: &str) -> Result<(), McpBridgeError>;

    /// Set the root note for a sample.
    fn set_sample_root_note(&self, id: u64, note: u8) -> Result<(), McpBridgeError>;

    /// Normalize sample peak to 0 dB.
    fn normalize_sample(&self, id: u64) -> Result<(), McpBridgeError>;

    /// Reverse sample audio data.
    fn reverse_sample(&self, id: u64) -> Result<(), McpBridgeError>;

    /// Auto-trim silence from sample.
    fn trim_sample_silence(&self, id: u64) -> Result<(), McpBridgeError>;

    /// Get detailed info for a sample including audio statistics.
    fn get_sample_info(&self, id: u64) -> Result<DetailedSampleInfo, McpBridgeError>;

    /// Duplicate a sample (new ID, same data).
    fn duplicate_sample(&self, id: u64) -> Result<SampleInfo, McpBridgeError>;

    /// Set loop region for a sample. Pass None to disable looping.
    fn set_sample_loop(
        &self,
        id: u64,
        enabled: bool,
        start_seconds: Option<f64>,
        end_seconds: Option<f64>,
        crossfade_ms: Option<f64>,
    ) -> Result<(), McpBridgeError>;

    /// Set crop region for a sample. Pass None values to remove crop.
    fn set_sample_crop(
        &self,
        id: u64,
        start_seconds: Option<f64>,
        end_seconds: Option<f64>,
    ) -> Result<(), McpBridgeError>;

    /// Export a sample to a WAV file.
    fn export_sample(
        &self,
        id: u64,
        path: &str,
        bit_depth: Option<u8>,
    ) -> Result<(), McpBridgeError>;

    // === Sampler module ===

    /// Assign a sample to a Sampler module.
    fn assign_sample_to_module(
        &self,
        instrument_id: u64,
        module_id: &str,
        sample_id: u64,
    ) -> Result<(), McpBridgeError>;

    /// Get the current state of a Sampler module.
    fn get_sampler_state(
        &self,
        instrument_id: u64,
        module_id: &str,
    ) -> Result<SamplerStateInfo, McpBridgeError>;

    /// Set a Sampler module parameter by name.
    fn set_sampler_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
        value: &str,
    ) -> Result<(), McpBridgeError>;

    // === Audio input ===

    /// List available audio input devices.
    fn list_input_devices(&self) -> Result<Vec<InputDeviceInfo>, McpBridgeError>;

    /// Get current audio input state.
    fn get_input_state(&self) -> Result<InputStateInfo, McpBridgeError>;

    // === Discovery ===

    /// Get detailed info for a single module type by its type key (e.g. "osc", "flt").
    fn get_module_type_info(&self, type_key: &str) -> Result<ModuleTypeInfo, McpBridgeError>;

    /// Search module types by category, port requirements, or text query.
    ///
    /// A text query is tokenised and scored with field weights (name 10,
    /// tags 5, description 2, parameter 2); zero-score modules are dropped and
    /// the rest sorted best-first. When the query matched nothing, the result's
    /// `did_you_mean` carries edit-distance near-misses instead of an empty list.
    fn search_modules(
        &self,
        category: Option<&str>,
        has_input_type: Option<&str>,
        has_output_type: Option<&str>,
        query: Option<&str>,
    ) -> Result<ModuleSearchResult, McpBridgeError>;

    /// Check whether a connection between two ports is valid.
    fn check_connection(
        &self,
        instrument_id: u64,
        from_module: &str,
        from_port: &str,
        to_module: &str,
        to_port: &str,
    ) -> Result<ConnectionCheckResult, McpBridgeError>;
}

/// Interpolation curve for an automation point.
///
/// Mirrors the sequencer's `CurveType` without depending on the sequencer
/// crate. `Exponential` carries no strength here — it is supplied separately
/// via `curve_strength` so the schema stays a flat string enum (the AWE-style
/// pattern that advertises the valid values to MCP clients).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Default,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub enum CurveKind {
    /// Linear interpolation to the next point.
    #[default]
    Linear,
    /// Hold the value until the next point.
    Step,
    /// Exponential curve; strength comes from `curve_strength`.
    Exponential,
    /// Smoothstep S-curve.
    SCurve,
}

/// Automation point data for MCP bridge.
pub struct BridgeAutomationPointData {
    /// Instrument parameter name: "Volume", "Pan", "FilterCutoff", "FilterResonance",
    /// "Attack", "Decay", "Sustain", "Release".
    pub param: String,
    /// Instrument index (default 0).
    pub instrument_id: u16,
    /// Position in beats.
    pub beat: f32,
    /// Normalized value (0.0-1.0).
    pub value: f32,
    /// Interpolation curve.
    pub curve: CurveKind,
    /// Strength for `CurveKind::Exponential` (-127..=127); ignored otherwise.
    pub curve_strength: Option<i8>,
}

/// Parameter set for batch set_parameters operations.
pub struct BridgeParamSet {
    /// Module ID string (e.g. "osc-1").
    pub module_id: String,
    /// Parameter name.
    pub param_name: String,
    /// New value.
    pub value: f32,
}

/// Aggregate per-instrument diagnostics into a project-wide lint report: tally
/// every severity across all instruments, but list in `entries` only the
/// instruments carrying an *actionable* (`Warning`/`Error`) diagnostic — so an
/// empty project (whose instruments emit only an `Info` "graph is empty" note)
/// doesn't flood the report. Pure (no engine access) so `SynthBridge::lint_project`
/// stays a thin I/O wrapper and the aggregation is unit-testable on its own.
pub(crate) fn build_lint_report(
    per_instrument: Vec<(u64, String, Vec<GraphDiagnostic>)>,
) -> ProjectLintReport {
    let instruments_checked = per_instrument.len();
    let mut entries = Vec::new();
    let mut error_count = 0;
    let mut warning_count = 0;
    let mut info_count = 0;

    for (instrument_id, instrument_name, diagnostics) in per_instrument {
        let mut actionable = false;
        for diagnostic in &diagnostics {
            match diagnostic.severity {
                DiagnosticSeverity::Error => {
                    error_count += 1;
                    actionable = true;
                }
                DiagnosticSeverity::Warning => {
                    warning_count += 1;
                    actionable = true;
                }
                DiagnosticSeverity::Info => info_count += 1,
            }
        }
        if actionable {
            entries.push(ProjectLintEntry {
                instrument_id,
                instrument_name,
                diagnostics,
            });
        }
    }

    ProjectLintReport {
        instruments_checked,
        error_count,
        warning_count,
        info_count,
        entries,
    }
}

#[cfg(test)]
mod lint_report_tests {
    use super::build_lint_report;
    use crate::types::{DiagnosticSeverity, GraphDiagnostic};

    fn diag(severity: DiagnosticSeverity) -> GraphDiagnostic {
        GraphDiagnostic {
            severity,
            module_id: None,
            message: "x".to_string(),
        }
    }

    #[test]
    fn tallies_all_severities_but_entries_are_actionable_only() {
        let report = build_lint_report(vec![
            (
                1,
                "Bass".to_string(),
                vec![
                    diag(DiagnosticSeverity::Warning),
                    diag(DiagnosticSeverity::Error),
                ],
            ),
            // A clean instrument: counted in instruments_checked, absent from entries.
            (2, "Lead".to_string(), vec![]),
            // Info-only (e.g. empty graph): counted in info_count, but NOT in
            // entries — it isn't actionable.
            (3, "Pad".to_string(), vec![diag(DiagnosticSeverity::Info)]),
        ]);

        assert_eq!(report.instruments_checked, 3);
        assert_eq!(report.error_count, 1);
        assert_eq!(report.warning_count, 1);
        // info_count tallies the Info note even though instrument 3 is absent
        // from entries — so the counts are not derivable from entries alone.
        assert_eq!(report.info_count, 1);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].instrument_id, 1);
    }

    #[test]
    fn actionable_entry_keeps_its_info_diagnostics_for_context() {
        let report = build_lint_report(vec![(
            1,
            "Bass".to_string(),
            vec![
                diag(DiagnosticSeverity::Info),
                diag(DiagnosticSeverity::Error),
            ],
        )]);

        assert_eq!(report.entries.len(), 1);
        // The whole diagnostic list (Info included) rides along for context.
        assert_eq!(report.entries[0].diagnostics.len(), 2);
        assert_eq!(report.error_count, 1);
        assert_eq!(report.info_count, 1);
    }

    #[test]
    fn empty_project_is_clean() {
        let report = build_lint_report(vec![]);
        assert_eq!(report.instruments_checked, 0);
        assert_eq!(report.error_count, 0);
        assert_eq!(report.warning_count, 0);
        assert!(report.entries.is_empty());
    }
}

#[cfg(test)]
mod insert_anchor_tests {
    use super::{unique_audio_cable_from, unique_audio_cable_into, unique_module_of_type};
    use crate::types::{ConnectionInfo, ModuleInfo};

    fn module(id: &str, type_name: &str) -> ModuleInfo {
        ModuleInfo {
            id: id.to_string(),
            module_type: type_name.to_string(),
            name: id.to_string(),
            bypassed: false,
            description: String::new(),
            parameters: vec![],
            input_ports: vec![],
            output_ports: vec![],
            mod_matrix_routings: None,
            scripts: None,
        }
    }

    fn cable(fm: &str, fp: &str, tm: &str, tp: &str) -> ConnectionInfo {
        ConnectionInfo {
            from_module: fm.to_string(),
            from_port: fp.to_string(),
            to_module: tm.to_string(),
            to_port: tp.to_string(),
        }
    }

    // osc-1 → flt-1 → amp-1 → out-1, plus a CV side-cable env-1 → amp-1.
    fn graph() -> (Vec<ModuleInfo>, Vec<ConnectionInfo>) {
        let modules = vec![
            module("osc-1", "Oscillator"),
            module("flt-1", "Filter"),
            module("amp-1", "Amplifier"),
            module("env-1", "Envelope"),
            module("out-1", "Stereo Output"),
        ];
        let conns = vec![
            cable("osc-1", "output", "flt-1", "input"),
            cable("flt-1", "output", "amp-1", "input"),
            cable("amp-1", "output", "out-1", "input"),
            cable("env-1", "output", "amp-1", "cv"),
        ];
        (modules, conns)
    }

    #[test]
    fn unique_type_resolves_or_reports_ambiguity() {
        let (modules, _) = graph();
        assert_eq!(
            unique_module_of_type("Amplifier", &modules).unwrap(),
            "amp-1"
        );
        assert!(unique_module_of_type("Mixer", &modules).is_err()); // none
        let two = vec![module("flt-1", "Filter"), module("flt-2", "Filter")];
        assert!(unique_module_of_type("Filter", &two).is_err()); // ambiguous
    }

    #[test]
    fn cable_after_picks_the_audio_output_cable() {
        let (_, conns) = graph();
        let audio_out = vec!["output".to_string()];
        let c = unique_audio_cable_from("osc-1", &audio_out, &conns).unwrap();
        assert_eq!(
            (c.from_module.as_str(), c.to_module.as_str()),
            ("osc-1", "flt-1")
        );
        // A module with no outgoing audio cable errors.
        assert!(unique_audio_cable_from("out-1", &audio_out, &conns).is_err());
    }

    #[test]
    fn cable_before_ignores_cv_inputs() {
        let (_, conns) = graph();
        // amp-1 has an audio input ("input") and a CV input ("cv"); only
        // the audio one is a splice target, so resolution is unambiguous.
        let audio_in = vec!["input".to_string()];
        let c = unique_audio_cable_into("amp-1", &audio_in, &conns).unwrap();
        assert_eq!(
            (c.from_module.as_str(), c.to_module.as_str()),
            ("flt-1", "amp-1")
        );
    }

    #[test]
    fn cable_after_reports_branching() {
        let mut conns = graph().1;
        // Make osc-1 fan out to a second audio destination → branch.
        conns.push(cable("osc-1", "output", "amp-1", "input"));
        let audio_out = vec!["output".to_string()];
        assert!(unique_audio_cable_from("osc-1", &audio_out, &conns).is_err());
    }
}
