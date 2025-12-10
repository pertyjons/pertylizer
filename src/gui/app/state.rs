//! Application state types for the synthesizer GUI.
//!
//! Contains state structures for master effects, navigation, and other UI elements.

use serde::{Deserialize, Serialize};

// Re-export TrackerViewState for convenience
pub use crate::sequencer::view::TrackerViewState;

// ============================================================================
// NAVIGATION STATE
// ============================================================================

/// Main application view (tab-based navigation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AppView {
    /// Instrument rack and patch editor view (default).
    #[default]
    Rack,
    /// Pattern/song sequencer view.
    Sequencer,
    /// Mixer view with channel strips.
    Mixer,
}

impl AppView {
    /// Get the display icon for this view.
    #[must_use]
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Rack => "🔌",
            Self::Sequencer => "🎹",
            Self::Mixer => "🎚",
        }
    }

    /// Get the display label for this view.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rack => "Rack",
            Self::Sequencer => "Sequencer",
            Self::Mixer => "Mixer",
        }
    }
}

/// Top panel drawer state (expandable panels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TopPanel {
    /// No panel open.
    #[default]
    None,
    /// MIDI settings panel.
    Midi,
    /// Engine/audio settings panel.
    Engine,
}

// Re-export effect types from views module to avoid duplication
pub use crate::gui::views::master_effects::{MasterEffectParams, MasterEffectUiState};
