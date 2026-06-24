//! GUI module for Pertylizer.
//!
//! This module provides the egui-based graphical interface for the synthesizer.
//!
//! # Modular UI Components
//!
//! - `widgets` - Reusable UI widgets (knobs, meters, ports, cables)
//! - `module_panel` - Renders individual synth modules
//! - `patch_editor` - Canvas for arranging modules and connections

#[cfg(feature = "gui-egui")]
pub mod egui_backend;

#[cfg(feature = "gui-egui")]
pub mod keyboard;

#[cfg(feature = "gui-egui")]
pub mod widgets;

#[cfg(feature = "gui-egui")]
pub mod module_panel;

#[cfg(feature = "gui-egui")]
pub mod patch_editor;

#[cfg(feature = "gui-egui")]
pub mod theme;

#[cfg(feature = "gui-egui")]
pub mod dialogs;

#[cfg(feature = "gui-egui")]
pub mod export_dialog;

#[cfg(feature = "gui-egui")]
pub mod patch_bridge;

#[cfg(feature = "gui-egui")]
pub mod auto_layout;

#[cfg(feature = "gui-egui")]
pub mod instrument_rack;

#[cfg(feature = "gui-egui")]
pub mod input;

#[cfg(feature = "gui-egui")]
pub mod views;

#[cfg(feature = "gui-egui")]
pub mod app;

#[cfg(feature = "gui-egui")]
pub mod awe_view;

#[cfg(feature = "gui-egui")]
pub mod panels;

#[cfg(feature = "gui-egui")]
pub mod sequencer;

#[cfg(feature = "gui-egui")]
pub mod pattern_view;

#[cfg(feature = "gui-egui")]
pub mod list_panel;

#[cfg(feature = "gui-egui")]
pub mod sample_view;

#[cfg(feature = "gui-egui")]
pub mod mixer_view;

#[cfg(feature = "gui-egui")]
pub mod welcome_view;

#[cfg(feature = "gui-egui")]
pub mod analyze;

#[cfg(feature = "gui-egui")]
pub(crate) mod clipboard;

use crate::audio::{AudioHostTrait, StreamConfig};
use std::error::Error;
use std::sync::Arc;
use synth_engine::{AllocatorConfig, EngineHandle, SynthEngine};

/// Result type for GUI operations.
pub type GuiResult<T> = Result<T, Box<dyn Error>>;

/// Configuration for the synthesizer GUI.
pub struct SynthGuiConfig {
    /// Window title.
    pub title: String,
    /// Voice allocator configuration.
    pub allocator_config: AllocatorConfig,
    /// Audio stream configuration.
    pub stream_config: StreamConfig,
    /// Shared synth session (module lifecycle).
    pub session: Arc<crate::session::SynthSession>,
    /// Shared song data for sequencer.
    pub song: Arc<parking_lot::RwLock<synth_sequencer::Song>>,
    /// Shared MCP state (if MCP feature enabled).
    #[cfg(feature = "mcp")]
    pub mcp_shared: Option<std::sync::Arc<crate::mcp_shared::McpSharedState>>,
    /// Shared OSC state (if OSC feature enabled).
    #[cfg(feature = "osc")]
    pub osc_shared: Option<synth_osc::OscSharedState>,
    /// Persistent application settings.
    pub settings: crate::io::settings::AppSettings,
    /// Shared sample library.
    pub sample_library: Arc<std::sync::RwLock<synth_sampler::SampleLibrary>>,
}

/// Trait that all GUI backends must implement.
pub trait GuiBackend {
    /// Returns the name of this backend.
    fn name(&self) -> &'static str;

    /// Run the GUI event loop.
    fn run(
        self: Box<Self>,
        engine: SynthEngine,
        handle: EngineHandle,
        host: Box<dyn AudioHostTrait>,
        config: SynthGuiConfig,
    ) -> GuiResult<()>;
}

/// Create the GUI backend.
#[cfg(feature = "gui-egui")]
pub fn create_backend() -> Box<dyn GuiBackend> {
    Box::new(egui_backend::EguiBackend::new())
}

// Re-exports for convenience
#[cfg(feature = "gui-egui")]
pub use egui_backend::EguiBackend;
