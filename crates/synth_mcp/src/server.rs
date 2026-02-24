//! MCP server implementation using rmcp.
//!
//! Defines tool handlers that delegate to the SynthBridge trait.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ServerInfo;
use rmcp::{ServerHandler, tool, tool_handler, tool_router};

use crate::bridge::SynthBridge;

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
    #[schemars(description = "New parameter value as a float")]
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

// === MCP Server ===

/// The MCP server that wraps a SynthBridge implementation.
#[derive(Clone)]
pub struct SynthMcpServer {
    bridge: Arc<dyn SynthBridge>,
    tool_router: ToolRouter<Self>,
}

impl SynthMcpServer {
    /// Create a new MCP server backed by the given bridge.
    pub fn new(bridge: Arc<dyn SynthBridge>) -> Self {
        Self {
            bridge,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_handler]
impl ServerHandler for SynthMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Modular synthesizer MCP server. Inspect and control the running synth: \
                 list modules, read parameters, change settings, play notes."
                    .into(),
            ),
            ..Default::default()
        }
    }
}

#[tool_router]
impl SynthMcpServer {
    #[tool(description = "List all instruments in the synth engine with their basic settings")]
    async fn list_instruments(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_instruments() {
            Ok(instruments) => serde_json::to_string_pretty(&instruments)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Get detailed information about a specific instrument including module count and effects"
    )]
    async fn get_instrument_info(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_instrument_info(params.0.instrument_id) {
            Ok(info) => serde_json::to_string_pretty(&info)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all modules in an instrument's voice graph with their types and names"
    )]
    async fn list_modules(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.list_modules(params.0.instrument_id) {
            Ok(modules) => serde_json::to_string_pretty(&modules)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
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
            Ok(info) => serde_json::to_string_pretty(&info)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Get all connections (cables) between modules in the voice graph")]
    async fn get_connections(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_connections(params.0.instrument_id) {
            Ok(conns) => serde_json::to_string_pretty(&conns)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Get the current value of a specific module parameter")]
    async fn get_parameter(&self, params: Parameters<GetParameterParam>) -> String {
        match self.bridge.get_parameter(
            params.0.instrument_id,
            &params.0.module_id,
            &params.0.param_name,
        ) {
            Ok(info) => serde_json::to_string_pretty(&info)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Get engine status: CPU usage, voice count, meters, transport state")]
    async fn get_engine_status(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.get_engine_status() {
            Ok(status) => serde_json::to_string_pretty(&status)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Run diagnostics on the module graph to find issues like disconnected modules or missing connections"
    )]
    async fn get_graph_diagnostics(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_graph_diagnostics(params.0.instrument_id) {
            Ok(diags) => serde_json::to_string_pretty(&diags)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Set a module parameter to a new value. Use list_modules and get_module_info to discover available parameters."
    )]
    async fn set_parameter(&self, params: Parameters<SetParameterParam>) -> String {
        match self.bridge.set_parameter(
            params.0.instrument_id,
            &params.0.module_id,
            &params.0.param_name,
            params.0.value,
        ) {
            Ok(()) => "OK".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "Play a MIDI note (note on). Use note=60 for middle C, velocity=100 for moderate strength."
    )]
    async fn note_on(&self, params: Parameters<NoteOnParam>) -> String {
        let channel = params.0.channel.unwrap_or(1);
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

    #[tool(description = "Stop a MIDI note (note off).")]
    async fn note_off(&self, params: Parameters<NoteOffParam>) -> String {
        let channel = params.0.channel.unwrap_or(1);
        match self.bridge.note_off(params.0.note, channel) {
            Ok(()) => format!("Note {} off (ch={})", params.0.note, channel),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all available example patches with their categories, descriptions, and tags"
    )]
    async fn list_example_patches(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_example_patches() {
            Ok(patches) => serde_json::to_string_pretty(&patches)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
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
        description = "Get a snapshot of the current UI layout: module positions, sizes, connections, and overlap analysis for debugging"
    )]
    async fn get_ui_snapshot(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.get_ui_snapshot(params.0.instrument_id) {
            Ok(snapshot) => serde_json::to_string_pretty(&snapshot)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(
        description = "List all available module types with their ports and parameters. Use the type_key to add modules with add_module."
    )]
    async fn list_module_types(&self, _params: Parameters<NoParams>) -> String {
        match self.bridge.list_module_types() {
            Ok(types) => serde_json::to_string_pretty(&types)
                .unwrap_or_else(|e| format!("Serialization error: {e}")),
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
        description = "Clear the entire voice graph for an instrument, removing all modules and connections. Use this to start from scratch."
    )]
    async fn clear_graph(&self, params: Parameters<InstrumentIdParam>) -> String {
        match self.bridge.clear_graph(params.0.instrument_id) {
            Ok(()) => "OK: graph cleared".to_string(),
            Err(e) => format!("Error: {e}"),
        }
    }

    #[tool(description = "Disconnect a cable between two module ports.")]
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
}
