//! Bridge trait for connecting the MCP server to the synth engine.
//!
//! Implementors provide read access to engine state and write access
//! via the command sender. The trait uses primitive types to avoid
//! leaking synth_engine types into the MCP crate.

use crate::error::McpBridgeError;
use crate::types::{
    ApplyExamplePatchResult, AutomationLaneInfo, AutomationPointInfo, BatchResult,
    BuildInstrumentResult, ConnectionInfo, EngineStatus, ExamplePatchInfo, GraphDiagnostic,
    InstrumentInfo, ModuleInfo, ModuleTypeInfo, NoteInfo, ParameterInfo, PatternInfo,
    PlacementInfo, SetSongResult, SongInfo, TrackInfo, UiSnapshot,
};

// === Bridge-level data structures for batch operations ===

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
    /// Optional instrument ID (SeqInstrumentId). Default 0 = first instrument.
    /// During playback, the track's instrument overrides this when set.
    pub instrument_id: Option<u16>,
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
    /// Instrument name.
    pub name: String,
    /// Optional MIDI channel (1-16).
    pub midi_channel: Option<u8>,
    /// Optional volume (0.0-2.0).
    pub volume: Option<f32>,
    /// Optional pan (-1.0 to 1.0).
    pub pan: Option<f32>,
    /// Modules to create.
    pub modules: Vec<BridgeModuleDef>,
    /// Connections between modules (using array indices).
    pub connections: Vec<BridgeConnectionDef>,
}

/// Bridge between the MCP server and the synth engine.
///
/// All methods use primitive types. Conversion to domain types
/// (Hertz, MidiNote, Param, etc.) happens in the implementation.
pub trait SynthBridge: Send + Sync + 'static {
    // === Read operations ===

    /// List all instruments with basic info.
    fn list_instruments(&self) -> Result<Vec<InstrumentInfo>, McpBridgeError>;

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

    /// Get a single parameter value.
    fn get_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
    ) -> Result<ParameterInfo, McpBridgeError>;

    /// Get engine-wide status (CPU, voices, meters, transport).
    fn get_engine_status(&self) -> Result<EngineStatus, McpBridgeError>;

    /// Run diagnostics on the graph and report issues.
    fn get_graph_diagnostics(
        &self,
        instrument_id: u64,
    ) -> Result<Vec<GraphDiagnostic>, McpBridgeError>;

    // === Instrument lifecycle ===

    /// Create a new instrument. Returns info about the created instrument.
    fn create_instrument(&self, name: &str) -> Result<InstrumentInfo, McpBridgeError>;

    /// Delete an instrument.
    fn delete_instrument(&self, instrument_id: u64) -> Result<(), McpBridgeError>;

    /// Rename an instrument.
    fn rename_instrument(&self, instrument_id: u64, name: &str) -> Result<(), McpBridgeError>;

    /// Set instrument volume (0.0-2.0).
    fn set_instrument_volume(&self, instrument_id: u64, volume: f32) -> Result<(), McpBridgeError>;

    /// Set instrument pan (-1.0 to 1.0).
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

    // === Write operations ===

    /// Set a module parameter by name.
    fn set_parameter(
        &self,
        instrument_id: u64,
        module_id: &str,
        param_name: &str,
        value: f32,
    ) -> Result<(), McpBridgeError>;

    /// Send a MIDI note on.
    fn note_on(&self, note: u8, velocity: u8, channel: u8) -> Result<(), McpBridgeError>;

    /// Send a MIDI note off.
    fn note_off(&self, note: u8, channel: u8) -> Result<(), McpBridgeError>;

    // === Example patches ===

    /// List all available example patches grouped by category.
    fn list_example_patches(&self) -> Result<Vec<ExamplePatchInfo>, McpBridgeError>;

    /// Queue an example patch for loading (GUI picks it up next frame).
    fn load_example_patch(&self, name: &str) -> Result<String, McpBridgeError>;

    /// Get a snapshot of the current UI layout (module positions, sizes, connections).
    fn get_ui_snapshot(&self, instrument_id: u64) -> Result<UiSnapshot, McpBridgeError>;

    // === Module management ===

    /// List all available module types with their ports and parameters.
    fn list_module_types(&self) -> Result<Vec<ModuleTypeInfo>, McpBridgeError>;

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

    // === Sequencer: Song ===

    /// Get song info (name, tempo, length, pattern/track counts).
    fn get_song_info(&self) -> Result<SongInfo, McpBridgeError>;

    /// Set the song tempo in BPM.
    fn set_song_tempo(&self, bpm: f32) -> Result<(), McpBridgeError>;

    /// Set the song name.
    fn set_song_name(&self, name: &str) -> Result<(), McpBridgeError>;

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

    /// Add a note to a pattern. Returns the new note ID.
    /// `instrument_id` defaults to 0 if None. Track instrument overrides during playback.
    fn add_note(
        &self,
        pattern_id: u32,
        pitch: u8,
        start_beat: f32,
        duration_beats: f32,
        velocity: u8,
        instrument_id: Option<u16>,
    ) -> Result<u64, McpBridgeError>;

    /// Remove a note from a pattern.
    fn remove_note(&self, pattern_id: u32, note_id: u64) -> Result<(), McpBridgeError>;

    /// Update a note's properties (only provided fields are changed).
    fn update_note(
        &self,
        pattern_id: u32,
        note_id: u64,
        pitch: Option<u8>,
        start_beat: Option<f32>,
        duration_beats: Option<f32>,
        velocity: Option<u8>,
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

    // === Track control ===

    /// Set track volume (0.0-1.0).
    fn set_track_volume(&self, track_id: u16, volume: f32) -> Result<(), McpBridgeError>;

    /// Set track pan (0.0-1.0, 0.5=center).
    fn set_track_pan(&self, track_id: u16, pan: f32) -> Result<(), McpBridgeError>;

    /// Mute or unmute a track.
    fn set_track_mute(&self, track_id: u16, muted: bool) -> Result<(), McpBridgeError>;

    /// Solo or unsolo a track.
    fn set_track_solo(&self, track_id: u16, solo: bool) -> Result<(), McpBridgeError>;

    /// Rename a track.
    fn rename_track(&self, track_id: u16, name: &str) -> Result<(), McpBridgeError>;

    /// Delete a track and its placements.
    fn delete_track(&self, track_id: u16) -> Result<(), McpBridgeError>;

    // === Pattern management ===

    /// Rename a pattern.
    fn rename_pattern(&self, pattern_id: u32, name: &str) -> Result<(), McpBridgeError>;

    /// Set pattern length in beats.
    fn set_pattern_length(&self, pattern_id: u32, length_beats: f32) -> Result<(), McpBridgeError>;

    /// Duplicate a pattern (notes + automation). Returns new pattern ID.
    fn duplicate_pattern(&self, pattern_id: u32) -> Result<u32, McpBridgeError>;

    // === Song metadata ===

    /// Set the song author.
    fn set_song_author(&self, author: &str) -> Result<(), McpBridgeError>;

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
    /// Interpolation curve: "Linear", "Step", "Exponential", "SCurve".
    pub curve: String,
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
