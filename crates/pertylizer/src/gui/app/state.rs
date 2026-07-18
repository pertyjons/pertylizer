//! Application state types for the synthesizer GUI.
//!
//! Contains state structures for master effects, navigation, and other UI elements.

use serde::{Deserialize, Serialize};

// ============================================================================
// NAVIGATION STATE
// ============================================================================

/// Main application view (tab-based navigation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AppView {
    /// Landing / welcome view (default). Always available, independent of
    /// whether any instrument exists.
    #[default]
    Home,
    /// Instrument rack and patch editor view.
    Rack,
    /// Note Grid view (pooled note-processing graphs: pool list + node canvas).
    NoteGraph,
    /// Mod Grid view (pooled control-rate modulator graphs: pool list + node canvas).
    ModGrid,
    /// Pattern browser and editor view (orphan patterns + full-window piano roll).
    Pattern,
    /// Sequencer view (piano roll / arrangement).
    Sequencer,
    /// Mixer view (per-channel faders, pan, sends, return busses, master).
    Mixer,
    /// Sample browser and editor view.
    Sample,
}

// Re-export effect types from views module to avoid duplication
pub use crate::gui::views::master_effects::{MasterEffectParams, MasterEffectUiState};
