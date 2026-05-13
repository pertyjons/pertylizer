//! Serializable response types for MCP tools.

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Copy, Serialize)]
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
    /// True when `pitch_error_cents.abs() > 50.0` — fundamental is more
    /// than half a semitone away from the expected note.
    pub off_pitch: bool,
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

/// Mix-bus metrics common to `analyze_mix_bus` and `analyze_section`.
///
/// All `*_dbfs` fields use `-200.0` as a substitute for `-inf` so JSON
/// consumers don't have to special-case non-finite values. `lufs_integrated`
/// follows the same convention: silence reports `-200.0` rather than `-inf`.
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
    /// Overall RMS, linear.
    pub rms: f32,
    /// Overall RMS in dBFS.
    pub rms_dbfs: f32,
    /// Crest factor (peak_dBFS - rms_dBFS). Higher = more dynamic.
    pub crest_factor_db: f32,
    /// Integrated loudness (ITU-R BS.1770-4 LUFS).
    pub lufs_integrated: f32,
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
    /// Non-fatal warnings emitted during the render.
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
#[derive(Debug, Clone, Serialize)]
pub struct TrackContribution {
    /// Sequencer track ID.
    pub track_id: u16,
    /// Track name.
    pub track_name: String,
    /// Assigned instrument's seq ID (matches `InstrumentInfo.id` value).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument_id: Option<u16>,
    /// Sample peak across both channels of the soloed render, linear.
    pub peak: f32,
    /// Sample peak in dBFS (silence reported as -200.0).
    pub peak_dbfs: f32,
    /// Overall RMS of the soloed render, linear.
    pub rms: f32,
    /// Overall RMS in dBFS (silence reported as -200.0).
    pub rms_dbfs: f32,
    /// Integrated LUFS of the soloed render (silence reported as -200.0).
    pub lufs_integrated: f32,
    /// 4-band RMS energy of the soloed render (sub/low/mid/high).
    pub energy_bands: AnalyzeEnergyBands,
    /// Count of samples that hit the ±0.999 ceiling in the soloed render —
    /// pinpoints the actual offender when the master mix clips.
    pub clipped_samples: u32,
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
