//! Module panel state and port metadata.
//!
//! Holds the per-module UI state (`ModulePanelState`) and port-position
//! metadata (`PortPosition`) the patch editor uses for cable routing, plus
//! the category → accent-color mapping. The actual module rendering lives in
//! `patch_editor::PatchEditor::show`.

use eframe::egui::{Color32, Pos2, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

use synth_core::{ModuleCategory, PortName};
use synth_engine::ModuleId;
use synth_modules::EnvelopePositionBuffer;

use super::theme::theme;
use super::widgets::{WidgetPortDirection, WidgetPortType};

/// State for a module panel in the UI.
#[derive(Clone)]
pub struct ModulePanelState {
    /// Module ID.
    pub id: ModuleId,
    /// Position in the rack.
    pub position: Pos2,
    /// Last rendered size (used by auto-layout to avoid overlaps).
    pub size: Vec2,
    /// Cached parameter values (indexed by parameter name for lookup).
    /// Key is the parameter name from the descriptor.
    pub param_values: HashMap<String, f32>,
    /// Envelope position buffer for envelope modules (lock-free GUI sync).
    pub envelope_position: Option<Arc<EnvelopePositionBuffer>>,
    /// Reusable visualization sample buffers (avoids per-frame allocation).
    pub vis_buf_l: Vec<f32>,
    pub vis_buf_r: Vec<f32>,
}

impl ModulePanelState {
    pub fn new(id: ModuleId, position: Pos2) -> Self {
        Self {
            id,
            position,
            size: Vec2::new(250.0, 200.0),
            param_values: HashMap::new(),
            envelope_position: None,
            vis_buf_l: Vec::new(),
            vis_buf_r: Vec::new(),
        }
    }
}

/// Port position information for cable routing.
#[derive(Clone, Debug)]
pub struct PortPosition {
    pub module_id: ModuleId,
    pub port_name: PortName,
    pub position: Pos2,
    pub port_type: WidgetPortType,
    pub direction: WidgetPortDirection,
}

/// Get accent color for a module category.
pub fn category_color(category: ModuleCategory) -> Color32 {
    match category {
        ModuleCategory::Oscillator => theme().colors.accent_orange,
        ModuleCategory::Filter => theme().colors.accent_cyan,
        ModuleCategory::Envelope => theme().colors.accent_green,
        ModuleCategory::LFO => theme().colors.accent_purple,
        ModuleCategory::Amplifier => theme().colors.accent_yellow,
        ModuleCategory::Effect => theme().colors.accent_cyan,
        ModuleCategory::Utility => theme().colors.text_secondary,
        ModuleCategory::Sampler => theme().colors.accent_orange,
        ModuleCategory::Sequencer => theme().colors.accent_red,
        ModuleCategory::Mixer => theme().colors.accent_yellow,
        ModuleCategory::Output => theme().colors.accent_green,
        ModuleCategory::Visualizer => theme().colors.accent_purple,
        ModuleCategory::PhysicalModeling => theme().colors.accent_red,
    }
}
