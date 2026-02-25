//! Bridge trait for connecting the MCP server to the synth engine.
//!
//! Implementors provide read access to engine state and write access
//! via the command sender. The trait uses primitive types to avoid
//! leaking synth_engine types into the MCP crate.

use crate::error::McpBridgeError;
use crate::types::{
    ConnectionInfo, EngineStatus, ExamplePatchInfo, GraphDiagnostic, InstrumentInfo, ModuleInfo,
    ModuleTypeInfo, NoteInfo, ParameterInfo, PatternInfo, PlacementInfo, SongInfo, TrackInfo,
    UiSnapshot,
};

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
    fn add_note(
        &self,
        pattern_id: u32,
        pitch: u8,
        start_beat: f32,
        duration_beats: f32,
        velocity: u8,
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
}
