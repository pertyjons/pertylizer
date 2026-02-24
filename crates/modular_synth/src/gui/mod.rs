//! GUI abstraction layer for the modular synthesizer.
//!
//! This module provides a framework-agnostic interface for building
//! synthesizer GUIs. Different backends (console, egui, iced, etc.)
//! implement the `GuiBackend` trait.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     GuiBackend trait                        │
//! │  - run()           Start the GUI event loop                 │
//! │  - name()          Backend identifier                       │
//! └─────────────────────────────────────────────────────────────┘
//!                              ▲
//!              ┌───────────────┼───────────────┐
//!              │               │               │
//!     ┌────────┴────────┐ ┌───┴────┐ ┌───────┴───────┐
//!     │ ConsoleBackend  │ │ Egui   │ │ (future)      │
//!     │ (text UI)       │ │Backend │ │ Iced, etc.    │
//!     └─────────────────┘ └────────┘ └───────────────┘
//! ```
//!
//! # Modular UI Components (egui)
//!
//! The egui backend is organized into reusable components:
//!
//! - `widgets` - Reusable UI widgets (knobs, meters, ports, cables)
//! - `module_panel` - Renders individual synth modules
//! - `patch_editor` - Canvas for arranging modules and connections

#[cfg(feature = "gui-console")]
pub mod console;

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

use crate::audio::{AudioHostTrait, StreamConfig};
use std::error::Error;
use synth_engine::{AllocatorConfig, EngineHandle, SynthEngine};

/// Result type for GUI operations.
pub type GuiResult<T> = Result<T, Box<dyn Error>>;

/// Configuration for the synthesizer GUI.
#[derive(Clone)]
pub struct SynthGuiConfig {
    /// Window title.
    pub title: String,
    /// Initial window width.
    pub width: u32,
    /// Initial window height.
    pub height: u32,
    /// Voice allocator configuration.
    pub allocator_config: AllocatorConfig,
    /// Audio stream configuration.
    pub stream_config: StreamConfig,
    /// Shared MCP state (if MCP feature enabled).
    #[cfg(feature = "mcp")]
    pub mcp_shared: Option<std::sync::Arc<crate::mcp_shared::McpSharedState>>,
}

impl Default for SynthGuiConfig {
    fn default() -> Self {
        Self {
            title: "Modular Synthesizer".to_string(),
            width: 1200,
            height: 800,
            allocator_config: AllocatorConfig::default(),
            stream_config: StreamConfig::default(),
            #[cfg(feature = "mcp")]
            mcp_shared: None,
        }
    }
}

/// Trait that all GUI backends must implement.
///
/// This provides a common interface for different GUI frameworks,
/// allowing easy switching between backends.
pub trait GuiBackend {
    /// Returns the name of this backend (e.g., "egui", "console").
    fn name(&self) -> &'static str;

    /// Run the GUI event loop.
    ///
    /// This method takes ownership of the audio system and runs
    /// until the user closes the application.
    fn run(
        self: Box<Self>,
        engine: SynthEngine,
        handle: EngineHandle,
        host: Box<dyn AudioHostTrait>,
        config: SynthGuiConfig,
    ) -> GuiResult<()>;
}

/// Available GUI backend types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuiType {
    /// Text-based console interface.
    Console,
    /// Egui-based graphical interface.
    #[default]
    Egui,
}

impl GuiType {
    /// Parse from command-line argument.
    pub fn from_arg(arg: &str) -> Option<Self> {
        match arg.to_lowercase().as_str() {
            "console" | "tui" | "text" => Some(Self::Console),
            "egui" | "gui" | "graphical" => Some(Self::Egui),
            _ => None,
        }
    }

    /// Get the name of this GUI type.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Console => "console",
            Self::Egui => "egui",
        }
    }
}

/// Create a GUI backend of the specified type.
///
/// Returns an error if the requested backend is not compiled in.
pub fn create_backend(gui_type: GuiType) -> GuiResult<Box<dyn GuiBackend>> {
    match gui_type {
        GuiType::Console => {
            #[cfg(feature = "gui-console")]
            {
                Ok(Box::new(console::ConsoleBackend::new()))
            }
            #[cfg(not(feature = "gui-console"))]
            {
                Err("Console GUI not compiled in. Enable 'gui-console' feature.".into())
            }
        }
        GuiType::Egui => {
            #[cfg(feature = "gui-egui")]
            {
                Ok(Box::new(egui_backend::EguiBackend::new()))
            }
            #[cfg(not(feature = "gui-egui"))]
            {
                Err("Egui GUI not compiled in. Enable 'gui-egui' feature.".into())
            }
        }
    }
}

/// Print available backends.
pub fn print_available_backends() {
    println!("Available GUI backends:");

    #[cfg(feature = "gui-console")]
    println!("  - console (text-based terminal interface)");

    #[cfg(feature = "gui-egui")]
    println!("  - egui (graphical interface) [default]");

    #[cfg(not(any(feature = "gui-console", feature = "gui-egui")))]
    println!("  (no backends compiled in)");
}

// Re-exports for convenience
#[cfg(feature = "gui-console")]
pub use console::ConsoleBackend;

#[cfg(feature = "gui-egui")]
pub use egui_backend::EguiBackend;
