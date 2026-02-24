//! Bridge trait for connecting the MCP server to the synth engine.
//!
//! Implementors provide read access to engine state and write access
//! via the command sender. The trait uses primitive types to avoid
//! leaking synth_engine types into the MCP crate.

use crate::error::McpBridgeError;
use crate::types::{
    ConnectionInfo, EngineStatus, GraphDiagnostic, InstrumentInfo, ModuleInfo, ParameterInfo,
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
}
