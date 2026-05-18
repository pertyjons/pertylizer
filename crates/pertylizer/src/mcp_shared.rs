//! Shared state between the MCP server and the GUI.
//!
//! `McpSharedState` is created in `main.rs` and shared via `Arc` to both
//! the `AppSynthBridge` (MCP side) and `SynthApp` (GUI side).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use synth_awe::AweState;

use crate::patch::{Author, Patch};
use crate::project::ProjectFile;
use synth_mcp::McpSessionRegistry;
use synth_sequencer::Song;

/// UI-refresh notification produced when MCP (or any non-GUI caller)
/// applies a project to the engine. The GUI consumes this on its next
/// frame to rebuild its UI mirrors via `refresh_ui_from_project` /
/// `refresh_ui_after_reset`. Stays `None` after a save — saves don't
/// change engine state so the UI has nothing to refresh.
pub enum ProjectRefresh {
    /// A project was loaded — the GUI must rebuild every UI mirror
    /// (instruments, patch editor canvases, AWE UI, keyboard octave, …).
    Loaded(Box<ProjectFile>),
    /// The project was reset to empty — equivalent to loading an empty
    /// `ProjectFile`. The GUI rebuilds its mirrors against the empty
    /// state.
    Reset,
}

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
    /// Serializes concurrent project I/O (save vs. load, two MCP clients
    /// racing). Held for the duration of one apply.
    pub project_io_lock: parking_lot::Mutex<()>,
    /// Bumped on every successful project apply (load / reset). The GUI
    /// observes increments to detect "something changed; refresh".
    pub project_revision: AtomicU64,
    /// UI-refresh queue populated by MCP/non-GUI loads, drained by the
    /// GUI on each frame. `None` between events.
    pub pending_project_refresh: Mutex<Option<ProjectRefresh>>,
    /// Path of the most recently loaded project, for the GUI title bar
    /// and "Save" → existing-path detection. Cleared on `new_project`.
    pub last_loaded_project_path: Mutex<Option<PathBuf>>,
    /// Result of the most recent project I/O op (load / save / new) so
    /// the GUI can surface errors in the status line. `None` before any
    /// op has run.
    pub last_project_io_status: Mutex<Option<Result<String, String>>>,
    /// Current AWE state (written by GUI each frame, read by MCP).
    pub awe_state: Mutex<AweState>,
    /// Pending AWE state change from MCP (consumed by GUI each frame).
    pub pending_awe_state: Mutex<Option<AweState>>,
    /// Free-text description of the AWE state's acoustic character.
    /// Lives outside `AweState` to avoid touching 36+ literal initializers
    /// in the preset table. Empty string == not set. Both GUI and MCP
    /// read/write this directly.
    pub awe_description: Arc<Mutex<String>>,
    /// Project author / composer metadata. Both GUI ("Project → Edit
    /// metadata…") and MCP (future `set_project_author`) read and write
    /// this directly. `None` when not set — file-save then emits a
    /// missing `author` field.
    pub author: Arc<Mutex<Option<Author>>>,
}

impl McpSharedState {
    #[must_use]
    pub fn new() -> Self {
        Self::with_song(Arc::new(parking_lot::RwLock::new(Song::new("Untitled"))))
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
            project_io_lock: parking_lot::Mutex::new(()),
            project_revision: AtomicU64::new(0),
            pending_project_refresh: Mutex::new(None),
            last_loaded_project_path: Mutex::new(None),
            last_project_io_status: Mutex::new(None),
            awe_state: Mutex::new(AweState::default()),
            pending_awe_state: Mutex::new(None),
            awe_description: Arc::new(Mutex::new(String::new())),
            author: Arc::new(Mutex::new(None)),
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
