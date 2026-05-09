//! Shared state between the MCP server and the GUI.
//!
//! `McpSharedState` is created in `main.rs` and shared via `Arc` to both
//! the `AppSynthBridge` (MCP side) and `SynthApp` (GUI side).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use synth_awe::AweState;

use crate::patch::Patch;
use synth_mcp::McpSessionRegistry;
use synth_sequencer::Song;

/// Action requested by MCP that must be executed on the GUI thread.
pub enum ProjectAction {
    /// Reset to a new empty project.
    New,
    /// Save the current project to a file.
    Save(PathBuf),
    /// Load a project from a file.
    Load(PathBuf),
}

/// PNG bytes on success or a failure description, signalled by the GUI thread
/// in response to a screenshot request.
pub type ScreenshotResult = Result<Vec<u8>, String>;

/// Callback the MCP thread can invoke to wake up the egui update loop.
/// egui pauses its update loop when nothing interactive is happening, so
/// background requests (screenshot, etc.) need to nudge it back into a frame.
pub type RepaintSignal = Arc<dyn Fn() + Send + Sync>;

/// Shared state for communication between MCP bridge and GUI.
pub struct McpSharedState {
    /// Patch queued for loading by MCP (consumed by GUI each frame).
    pub pending_patch: Mutex<Option<(Patch, String)>>,
    /// Current UI layout snapshot (written by GUI, read by MCP).
    pub ui_layout: Mutex<UiLayoutData>,
    /// Shared song data for sequencer (read/written by MCP, read by engine).
    pub song: Arc<parking_lot::RwLock<Song>>,
    /// Whether the MCP HTTP server is listening.
    pub mcp_listening: AtomicBool,
    /// Registry of active MCP sessions with client identity info.
    pub mcp_sessions: McpSessionRegistry,
    /// Auto-layout requested by MCP (consumed by GUI each frame).
    pub pending_auto_layout: AtomicBool,
    /// Project action queued by MCP (consumed by GUI each frame).
    pub pending_project_action: Mutex<Option<ProjectAction>>,
    /// Result of the last project action, signaled via condvar.
    pub project_action_result: (Mutex<Option<Result<String, String>>>, Condvar),
    /// Current AWE state (written by GUI each frame, read by MCP).
    pub awe_state: Mutex<AweState>,
    /// Pending AWE state change from MCP (consumed by GUI each frame).
    pub pending_awe_state: Mutex<Option<AweState>>,
    /// Screenshot requested by MCP (consumed by GUI each frame).
    pub pending_screenshot: AtomicBool,
    /// Result of the last screenshot request, signaled via condvar.
    /// `Ok(png_bytes)` on success, `Err(message)` on encoding failure.
    pub screenshot_result: (Mutex<Option<ScreenshotResult>>, Condvar),
    /// Set by the GUI on first frame, called by MCP to wake the GUI thread
    /// when it has paused its update loop.
    pub repaint_signal: Mutex<Option<RepaintSignal>>,
    /// Frame counter incremented by the GUI each `ui()` call. Useful for
    /// diagnosing whether the GUI thread is actually rendering.
    pub gui_frame_counter: AtomicU64,
}

impl McpSharedState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending_patch: Mutex::new(None),
            ui_layout: Mutex::new(UiLayoutData::default()),
            song: Arc::new(parking_lot::RwLock::new(Song::new("Untitled"))),
            mcp_listening: AtomicBool::new(false),
            mcp_sessions: McpSessionRegistry::new(),
            pending_auto_layout: AtomicBool::new(false),
            pending_project_action: Mutex::new(None),
            project_action_result: (Mutex::new(None), Condvar::new()),
            awe_state: Mutex::new(AweState::default()),
            pending_awe_state: Mutex::new(None),
            pending_screenshot: AtomicBool::new(false),
            screenshot_result: (Mutex::new(None), Condvar::new()),
            repaint_signal: Mutex::new(None),
            gui_frame_counter: AtomicU64::new(0),
        }
    }

    /// Create with a pre-existing shared Song (so GUI and MCP share the same instance).
    #[must_use]
    pub fn with_song(song: Arc<parking_lot::RwLock<Song>>) -> Self {
        Self {
            pending_patch: Mutex::new(None),
            ui_layout: Mutex::new(UiLayoutData::default()),
            song,
            mcp_listening: AtomicBool::new(false),
            mcp_sessions: McpSessionRegistry::new(),
            pending_auto_layout: AtomicBool::new(false),
            pending_project_action: Mutex::new(None),
            project_action_result: (Mutex::new(None), Condvar::new()),
            awe_state: Mutex::new(AweState::default()),
            pending_awe_state: Mutex::new(None),
            pending_screenshot: AtomicBool::new(false),
            screenshot_result: (Mutex::new(None), Condvar::new()),
            repaint_signal: Mutex::new(None),
            gui_frame_counter: AtomicU64::new(0),
        }
    }

    /// Check if the MCP server is listening.
    pub fn is_listening(&self) -> bool {
        self.mcp_listening.load(Ordering::Relaxed)
    }

    /// Get the number of active MCP sessions.
    pub fn active_sessions(&self) -> usize {
        self.mcp_sessions.active_count()
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
