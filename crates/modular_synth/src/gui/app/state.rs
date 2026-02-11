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
    /// Instrument rack and patch editor view (default).
    #[default]
    Rack,
}

// Re-export effect types from views module to avoid duplication
pub use crate::gui::views::master_effects::{MasterEffectParams, MasterEffectUiState};
