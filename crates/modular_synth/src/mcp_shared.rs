//! Shared state between the MCP server and the GUI.
//!
//! `McpSharedState` is created in `main.rs` and shared via `Arc` to both
//! the `AppSynthBridge` (MCP side) and `SynthApp` (GUI side).

use std::sync::Mutex;

use crate::patch::Patch;

/// Shared state for communication between MCP bridge and GUI.
pub struct McpSharedState {
    /// Patch queued for loading by MCP (consumed by GUI each frame).
    pub pending_patch: Mutex<Option<(Patch, String)>>,
    /// Current UI layout snapshot (written by GUI, read by MCP).
    pub ui_layout: Mutex<UiLayoutData>,
}

impl McpSharedState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending_patch: Mutex::new(None),
            ui_layout: Mutex::new(UiLayoutData::default()),
        }
    }
}

impl Default for McpSharedState {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of the current UI layout (positions, sizes, parameters).
#[derive(Debug, Clone, Default)]
pub struct UiLayoutData {
    /// Name of the currently loaded patch.
    pub patch_name: String,
    /// All modules with their visual state.
    pub modules: Vec<ModuleLayout>,
    /// All connections between modules.
    pub connections: Vec<ConnectionLayout>,
    /// Window size (width, height).
    pub window_size: (f32, f32),
}

/// Visual state of a single module in the UI.
#[derive(Debug, Clone)]
pub struct ModuleLayout {
    /// Module ID string (e.g. "osc-1").
    pub id: String,
    /// Module type name (e.g. "Oscillator").
    pub module_type: String,
    /// Human-readable display name (e.g. "Osc 1").
    pub name: String,
    /// Position in the workspace (x, y).
    pub position: (f32, f32),
    /// Rendered size (width, height).
    pub size: (f32, f32),
    /// Parameters as (name, display_value) pairs.
    pub parameters: Vec<(String, String)>,
}

/// A connection between two module ports.
#[derive(Debug, Clone)]
pub struct ConnectionLayout {
    /// Source module ID.
    pub from_module: String,
    /// Source port name.
    pub from_port: String,
    /// Destination module ID.
    pub to_module: String,
    /// Destination port name.
    pub to_port: String,
}
