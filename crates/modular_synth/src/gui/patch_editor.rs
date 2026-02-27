//! Rack view - the main workspace for arranging modules.
//!
//! This module handles the canvas where modules are placed,
//! connections are drawn, and modules can be added/removed.
//!
//! Modules are rendered as draggable, resizable windows with z-order support.
//! Cables are rendered behind modules; hovered cables pop to the foreground.

use eframe::egui::{self, Color32, Id, LayerId, Order, Pos2, Rect, Sense, Ui, Vec2};
use egui_extras as _;
use std::collections::{HashMap, HashSet, VecDeque};

use synth_core::{ModMatrixGridSize, ModMatrixParam, ModuleType, Param};
use synth_core::{ModuleCategory, ModuleDescriptor, PortName};
use synth_engine::graph::Connection;
use synth_engine::{EngineHandle, ModuleId};

use super::module_panel::{ModulePanelState, PortPosition, category_color};
use super::theme::theme;
use super::widgets::{
    CABLE_SPREAD, WidgetPortDirection, WidgetPortType, cable_color, closest_point_on_cable,
    draw_cable, draw_cable_dragging, draw_cable_highlighted, draw_flow_particles, point_near_cable,
};

/// Grid cell size in pixels. Used for grid drawing and snap-to-grid.
pub(crate) const GRID_SIZE: f32 = 50.0;

/// Trim sweep data to the last rising-edge crossing so the display
/// always shows complete waveform cycles (no visual gap at the end).
///
/// Scans backwards from the end to find the last point where the signal
/// crosses `threshold` from below. Returns a slice up to that point.
/// If no suitable crossing is found (e.g. sub-cycle data), returns all data.
fn trim_sweep_to_complete_cycles(samples: &[f32], threshold: f32) -> &[f32] {
    // Need at least a few samples, and don't trim more than 75%
    let min_len = samples.len() / 4;
    for i in (min_len.max(1)..samples.len()).rev() {
        if samples[i - 1] < threshold && samples[i] >= threshold {
            return &samples[..i];
        }
    }
    samples
}

/// Snap a position to the nearest grid point.
fn snap_to_grid(pos: Pos2) -> Pos2 {
    Pos2::new(
        (pos.x / GRID_SIZE).round() * GRID_SIZE,
        (pos.y / GRID_SIZE).round() * GRID_SIZE,
    )
}

/// Patch analysis: counts module types to enable smart display names and filtering.
///
/// Built once per frame from the current panels. Used for:
/// - Numbered module titles ("LFO 1" / "LFO 2" when 2+ LFOs, "LFO" when only 1)
/// - Filtering mod matrix dropdown choices (hide "LFO 2" source if only 1 LFO exists)
pub(crate) struct PatchAnalysis {
    /// How many of each module type exist.
    module_counts: HashMap<ModuleType, u16>,
}

impl PatchAnalysis {
    /// Build from current patch panels.
    fn from_panels(panels: &HashMap<ModuleId, ModulePanelState>) -> Self {
        let mut module_counts: HashMap<ModuleType, u16> = HashMap::new();
        for id in panels.keys() {
            *module_counts.entry(id.module_type).or_insert(0) += 1;
        }
        Self { module_counts }
    }

    /// Get count of a specific module type.
    fn count(&self, module_type: ModuleType) -> u16 {
        self.module_counts.get(&module_type).copied().unwrap_or(0)
    }

    /// Generate display name for a module.
    ///
    /// Always appends the instance number for consistency,
    /// e.g. "LFO 1", "Oscillator 1", even when only one exists.
    #[must_use]
    fn display_name(&self, module_id: ModuleId, base_name: &str) -> String {
        format!("{base_name} {}", module_id.instance)
    }
}

/// Module connectivity status for visualization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModuleConnectivity {
    /// Module is connected to an output (audio will be heard).
    Connected,
    /// Module has connections but doesn't reach an output.
    Orphaned,
    /// Module has no connections at all.
    #[default]
    Disconnected,
}

/// State for a pending connection being drawn.
#[derive(Clone, Debug)]
pub struct PendingConnection {
    /// Starting port info.
    pub from_module: ModuleId,
    pub from_port: PortName,
    pub from_position: Pos2,
    pub from_type: WidgetPortType,
    pub from_direction: WidgetPortDirection,
    /// Current mouse position.
    pub current_pos: Pos2,
}

/// State for a right-click context menu on a port.
#[derive(Clone)]
struct PortContextMenuState {
    module_id: ModuleId,
    port_name: PortName,
    port_type: WidgetPortType,
    direction: WidgetPortDirection,
    menu_pos: Pos2,
}

/// Request to create a new module and auto-connect it to an existing port.
#[derive(Debug, Clone)]
pub struct QuickAddRequest {
    /// What module to create.
    pub selection: PaletteSelection,
    /// The existing module/port to connect to.
    pub target_module: ModuleId,
    pub target_port: PortName,
    /// Direction of the target port (determines connection direction).
    pub target_direction: WidgetPortDirection,
    /// Screen position for placing the new module near the port.
    pub position: Pos2,
}

/// State for the unified background/cable right-click context menu.
#[derive(Clone)]
struct BgContextMenuState {
    /// Where to place the new module (world/logical coordinates).
    world_pos: Pos2,
    /// Where to draw the menu (screen coordinates).
    screen_pos: Pos2,
    /// If right-click was on a hovered cable, include it.
    cable: Option<Connection>,
}

/// The main rack view state.
#[derive(Clone)]
pub struct PatchEditor {
    /// All module panels.
    panels: HashMap<ModuleId, ModulePanelState>,
    /// All connections.
    connections: Vec<Connection>,
    /// Port positions (updated each frame).
    port_positions: HashMap<(ModuleId, PortName), PortPosition>,
    /// Currently selected module.
    selected_module: Option<ModuleId>,
    /// Connection being drawn.
    pending_connection: Option<PendingConnection>,
    /// Module descriptors (cached).
    descriptors: HashMap<ModuleId, ModuleDescriptor>,
    /// Next position for new modules.
    next_module_pos: Pos2,
    /// Module z-order (front to back).
    z_order: Vec<ModuleId>,
    /// Module connectivity status (updated when connections change).
    connectivity: HashMap<ModuleId, ModuleConnectivity>,
    /// Module bypass state (true = bypassed/off, false = active/on).
    bypassed: HashMap<ModuleId, bool>,
    /// Modules that need to be repositioned (after auto-layout).
    /// When a module is in this set, we use current_pos() instead of default_pos().
    needs_reposition: HashSet<ModuleId>,
    /// When true, auto-layout runs on the next frame that has a canvas_rect.
    needs_initial_layout: bool,
    /// The cable closest to the pointer (if any), updated each frame.
    hovered_cable: Option<Connection>,
    /// Right-click context menu on a port.
    port_context_menu: Option<PortContextMenuState>,
    /// Same-frame guard for port context menu.
    port_menu_just_opened: bool,
    /// Right-click context menu on background (or cable). Contains cable if hovered.
    bg_context_menu: Option<BgContextMenuState>,
    /// Same-frame guard for background context menu.
    bg_menu_just_opened: bool,
}

impl PatchEditor {
    pub fn new() -> Self {
        Self {
            panels: HashMap::new(),
            connections: Vec::new(),
            port_positions: HashMap::new(),
            selected_module: None,
            pending_connection: None,
            descriptors: HashMap::new(),
            next_module_pos: Pos2::new(50.0, 50.0),
            z_order: Vec::new(),
            connectivity: HashMap::new(),
            bypassed: HashMap::new(),
            needs_reposition: HashSet::new(),
            needs_initial_layout: true,
            hovered_cable: None,
            port_context_menu: None,
            port_menu_just_opened: false,
            bg_context_menu: None,
            bg_menu_just_opened: false,
        }
    }

    /// Add a module to the rack.
    pub fn add_module(&mut self, id: ModuleId, descriptor: ModuleDescriptor) {
        let position = self.next_module_pos;
        self.next_module_pos.x += 200.0; // module_min_width (180) + gap (20)
        // Wrap to next row based on typical screen width
        if self.next_module_pos.x > 1400.0 {
            self.next_module_pos.x = 50.0;
            self.next_module_pos.y += 250.0; // Generous row spacing for varying heights
        }

        // Initialize parameter values from defaults
        let mut param_values = HashMap::new();
        for param in &descriptor.parameters {
            param_values.insert(param.name.clone(), param.range.default);
        }

        let mut state = ModulePanelState::new(id, position);
        state.param_values = param_values;

        self.descriptors.insert(id, descriptor);
        self.panels.insert(id, state);
        self.z_order.push(id); // Add to front
        self.calculate_connectivity();
    }

    /// Add a module at a specific position.
    pub fn add_module_at(&mut self, id: ModuleId, descriptor: ModuleDescriptor, position: Pos2) {
        // Initialize parameter values from defaults
        let mut param_values = HashMap::new();
        for param in &descriptor.parameters {
            param_values.insert(param.name.clone(), param.range.default);
        }

        let mut state = ModulePanelState::new(id, position);
        state.param_values = param_values;

        self.descriptors.insert(id, descriptor);
        self.panels.insert(id, state);
        self.z_order.push(id);
        self.calculate_connectivity();
    }

    /// Clear all modules and connections.
    pub fn clear(&mut self) {
        self.panels.clear();
        self.descriptors.clear();
        self.connections.clear();
        self.port_positions.clear();
        self.z_order.clear();
        self.connectivity.clear();
        self.bypassed.clear();
        self.selected_module = None;
        self.pending_connection = None;
        self.next_module_pos = Pos2::new(50.0, 50.0);
    }

    /// Get module data for saving.
    pub fn get_module_data(
        &self,
        id: ModuleId,
    ) -> Option<(ModuleDescriptor, Pos2, HashMap<String, f32>)> {
        let descriptor = self.descriptors.get(&id)?;
        let panel = self.panels.get(&id)?;
        Some((
            descriptor.clone(),
            panel.position,
            panel.param_values.clone(),
        ))
    }

    /// Get the rendered size of a module panel.
    pub fn module_panel_size(&self, id: ModuleId) -> Option<Vec2> {
        self.panels.get(&id).map(|p| p.size)
    }

    /// Set envelope position buffer for a module (used for Envelope visualization).
    pub fn set_module_envelope_position(
        &mut self,
        id: ModuleId,
        buffer: std::sync::Arc<synth_modules::EnvelopePositionBuffer>,
    ) {
        if let Some(panel) = self.panels.get_mut(&id) {
            panel.envelope_position = Some(buffer);
        }
    }

    /// Remove a module from the rack.
    pub fn remove_module(&mut self, id: ModuleId) {
        self.panels.remove(&id);
        self.descriptors.remove(&id);
        self.z_order.retain(|&mid| mid != id);
        self.connectivity.remove(&id);
        self.bypassed.remove(&id);
        // Remove connections involving this module
        self.connections
            .retain(|c| c.from_module != id && c.to_module != id);
        if self.selected_module == Some(id) {
            self.selected_module = None;
        }
        // Cancel pending connection if it involves this module
        if self
            .pending_connection
            .as_ref()
            .is_some_and(|p| p.from_module == id)
        {
            self.pending_connection = None;
        }
        self.calculate_connectivity();
    }

    /// Bring a module to front.
    pub fn bring_to_front(&mut self, id: ModuleId) {
        self.z_order.retain(|&mid| mid != id);
        self.z_order.push(id);
    }

    /// Send a module to back.
    pub fn send_to_back(&mut self, id: ModuleId) {
        self.z_order.retain(|&mid| mid != id);
        self.z_order.insert(0, id);
    }

    /// Add a connection.
    pub fn add_connection(&mut self, connection: Connection) {
        if !self.connections.contains(&connection) {
            self.connections.push(connection);
            self.calculate_connectivity();
        }
    }

    /// Set a parameter value for a module in the rack view.
    /// The Param carries both the parameter type and value.
    pub fn set_parameter(&mut self, module_id: ModuleId, param: &Param, value: f32) {
        if let Some(panel) = self.panels.get_mut(&module_id) {
            panel.param_values.insert(param.name().to_string(), value);
        }
    }

    /// Set a parameter value by name (for patch loading compatibility).
    pub fn set_parameter_by_name(&mut self, module_id: ModuleId, param_name: &str, value: f32) {
        if let Some(panel) = self.panels.get_mut(&module_id) {
            panel.param_values.insert(param_name.to_string(), value);
        }
    }

    /// Remove a connection.
    #[allow(dead_code)]
    pub fn remove_connection(&mut self, connection: &Connection) {
        self.connections.retain(|c| c != connection);
        self.calculate_connectivity();
    }

    /// Get all connections.
    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    /// Check if a module is a "source" (no incoming audio connections).
    pub fn is_source(&self, module_id: ModuleId) -> bool {
        !self.connections.iter().any(|c| c.to_module == module_id)
    }

    /// Check if a module is a "sink" (no outgoing audio connections).
    pub fn is_sink(&self, module_id: ModuleId) -> bool {
        !self.connections.iter().any(|c| c.from_module == module_id)
    }

    /// Get connected ports for a module.
    fn get_connected_ports(&self, module_id: ModuleId) -> Vec<PortName> {
        let mut ports = Vec::new();
        for conn in &self.connections {
            if conn.from_module == module_id {
                ports.push(conn.from_port);
            }
            if conn.to_module == module_id {
                ports.push(conn.to_port);
            }
        }
        ports
    }

    /// Calculate the bounding box of all module positions + estimated size.
    /// Used to tell ScrollArea how large the content is.
    fn calculate_content_size(&self) -> Vec2 {
        if self.panels.is_empty() {
            return Vec2::new(800.0, 600.0);
        }
        let mut max_x: f32 = 0.0;
        let mut max_y: f32 = 0.0;
        for panel in self.panels.values() {
            max_x = max_x.max(panel.position.x + 350.0);
            max_y = max_y.max(panel.position.y + 400.0);
        }
        Vec2::new(max_x + 100.0, max_y + 100.0)
    }

    /// Draw the rack view.
    ///
    /// The `instrument_id` is used to namespace egui widget IDs, preventing
    /// collisions when multiple instruments have modules with the same ModuleId
    /// (e.g., both have "osc-1").
    pub fn show(
        &mut self,
        ui: &mut Ui,
        handle: &EngineHandle,
        instrument_id: u64,
    ) -> PatchEditorResult {
        let mut result = PatchEditorResult::default();
        let content_size = self.calculate_content_size();

        // Save the visible rect for toolbar positioning (before ScrollArea consumes it)
        let visible_rect = ui.available_rect_before_wrap();

        // Phase 1: ScrollArea for scrollbars and grid background.
        // We also capture the scroll area's layer_id — painting on that layer
        // later will render BEHIND module Areas (same Order::Background, but
        // the scroll layer is allocated first).
        let mut canvas_response = None;
        let mut scroll_layer_id = None;
        let mut scroll_clip_rect = None;
        let scroll_output = egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Allocate space for all content so ScrollArea shows scrollbars
                let scroll_rect =
                    Rect::from_min_size(ui.max_rect().min, content_size.max(ui.available_size()));
                canvas_response = Some(ui.allocate_rect(scroll_rect, Sense::click_and_drag()));

                // Draw grid
                self.draw_grid(ui, scroll_rect);

                // Save scroll area layer + clip rect for cable drawing (behind modules)
                scroll_layer_id = Some(ui.layer_id());
                scroll_clip_rect = Some(ui.clip_rect());

                scroll_rect
            });

        let scroll_rect = scroll_output.inner;
        let scroll_offset = scroll_output.state.offset;
        let area_origin = visible_rect.min.to_vec2();

        // Store the canvas rect for auto-layout calculations (in logical coordinates)
        result.canvas_rect = Some(scroll_rect);

        // Auto-layout on first frame (after startup or patch load)
        if self.needs_initial_layout && !self.panels.is_empty() {
            self.needs_initial_layout = false;
            self.apply_auto_layout(scroll_rect);
        }

        // Collect module screen rects from egui memory (persisted from previous frame)
        let module_rects: Vec<Rect> = self
            .panels
            .keys()
            .filter_map(|mid| {
                let wid = Id::new((instrument_id, "module_window", mid.to_string()));
                ui.ctx().memory(|mem| mem.area_rect(wid))
            })
            .collect();

        // Draw cables on the scroll area's layer BEFORE module Areas are created.
        // This uses the previous frame's port_positions — one frame delay is
        // imperceptible in an immediate-mode GUI.
        let time = ui.input(|i| i.time);
        if let Some(layer_id) = scroll_layer_id {
            let clip = scroll_clip_rect.unwrap_or(visible_rect);
            self.draw_connections(ui, time, layer_id, clip, &module_rects);
        }

        // Draw context menus (Foreground) — must happen after hover detection above
        self.draw_port_context_menu(ui, &mut result);
        self.draw_bg_context_menu(ui, &mut result);

        // Now clear port positions so modules can repopulate them for next frame
        self.port_positions.clear();

        // Build patch analysis for display names and mod matrix filtering
        let analysis = PatchAnalysis::from_panels(&self.panels);

        // Collect data before mutable iteration
        let module_ids: Vec<_> = self.z_order.clone();
        let connected_ports_map: HashMap<ModuleId, Vec<PortName>> = module_ids
            .iter()
            .map(|&id| (id, self.get_connected_ports(id)))
            .collect();

        // Constrain rect: prevent modules going to negative logical coords,
        // allow extending right/down (content grows to fit)
        let content_origin_screen = Pos2::new(
            visible_rect.min.x - scroll_offset.x,
            visible_rect.min.y - scroll_offset.y,
        );
        let constrain_rect =
            Rect::from_min_size(content_origin_screen, Vec2::new(20000.0, 20000.0));

        // Track which module to bring to front
        let mut bring_to_front: Option<ModuleId> = None;

        // Draw modules as windows (in z-order)
        for module_id in &module_ids {
            let module_id = *module_id;
            let connected_ports = connected_ports_map
                .get(&module_id)
                .cloned()
                .unwrap_or_default();

            let descriptor = match self.descriptors.get(&module_id) {
                Some(d) => d.clone(),
                None => continue,
            };

            // Get panel position before mutable borrow
            let panel_position = match self.panels.get(&module_id) {
                Some(s) => s.position,
                None => continue,
            };

            let accent_color = category_color(descriptor.category);
            let is_selected = self.selected_module == Some(module_id);
            let connectivity_status = self.get_connectivity(module_id);
            let is_bypassed = self.bypassed.get(&module_id).copied().unwrap_or(false);

            // Global modules (effects, visualizers) and internal routing modules (Utility
            // like Mod Matrix) are always "connected" — they work automatically without cables.
            let is_global_module = matches!(
                descriptor.category,
                ModuleCategory::Effect | ModuleCategory::Visualizer | ModuleCategory::Utility
            );

            // Dim modules that aren't connected to output, or are bypassed
            let opacity = if is_bypassed {
                0.4 // Heavily dimmed when bypassed
            } else if is_global_module {
                1.0 // Global modules are always active
            } else {
                match connectivity_status {
                    ModuleConnectivity::Connected => 1.0,
                    ModuleConnectivity::Orphaned => 0.6,
                    ModuleConnectivity::Disconnected => 0.4,
                }
            };

            let dimmed_accent = accent_color.gamma_multiply(opacity);

            let mut open = true;
            // Include instrument_id in the hash to prevent ID collisions across instruments
            let window_id = egui::Id::new((instrument_id, "module_window", module_id.to_string()));

            // Create frame with dimming for disconnected modules
            let frame = egui::Frame::window(&ui.ctx().style())
                .stroke(egui::Stroke::new(
                    if is_selected { 2.0 } else { 1.0 },
                    if is_selected {
                        dimmed_accent
                    } else {
                        dimmed_accent.gamma_multiply(0.5)
                    },
                ))
                .fill(
                    ui.ctx()
                        .style()
                        .visuals
                        .window_fill()
                        .gamma_multiply(opacity),
                );

            // Check if this module needs repositioning (after auto-layout)
            let needs_reposition = self.needs_reposition.contains(&module_id);

            let title = analysis.display_name(module_id, &descriptor.name);

            // Position in screen coordinates based on logical position and scroll offset
            let screen_pos = panel_position + area_origin - scroll_offset;

            // Use Area + Frame instead of Window so modules render at Order::Background
            // (same layer as panels). Panels rendered after CentralPanel paint over modules.
            let area = egui::Area::new(window_id)
                .order(Order::Background)
                .movable(true)
                .constrain_to(constrain_rect)
                .current_pos(screen_pos);

            // Get processing info for this module
            let is_source = self.is_source(module_id);
            let is_sink = self.is_sink(module_id);

            let is_inline_monitor = descriptor.type_id.0 == "inline_signal_monitor";

            let area_response = area.show(ui.ctx(), |ui| {
                // Clip to the visible scroll area so modules don't paint over panels
                ui.set_clip_rect(visible_rect);

                // Inline Signal Monitor: compact 100×50px with just oscilloscope + close button
                if is_inline_monitor {
                    let inline_frame = egui::Frame::new()
                        .fill(theme().colors.bg_dark)
                        .stroke(egui::Stroke::new(1.0, dimmed_accent.gamma_multiply(0.5)))
                        .corner_radius(4.0);
                    inline_frame.show(ui, |ui| {
                        let panel_width = 100.0;
                        let panel_height = 50.0;
                        ui.set_min_size(Vec2::new(panel_width, panel_height));
                        ui.set_max_size(Vec2::new(panel_width, panel_height));

                        // Draw ports at left/right edges
                        let port_col_w = 8.0;
                        ui.horizontal(|ui| {
                            // Left port (IN) — tiny dot
                            ui.vertical(|ui| {
                                ui.set_width(port_col_w);
                                ui.set_height(panel_height);
                                let center = ui.available_rect_before_wrap().center();
                                for port in &descriptor.ports {
                                    if port.direction == synth_core::PortDirection::Input {
                                        let port_rect = egui::Rect::from_center_size(
                                            center,
                                            Vec2::splat(8.0),
                                        );
                                        let port_resp = ui.allocate_rect(port_rect, Sense::click_and_drag());
                                        let port_color = if port_resp.hovered() {
                                            theme().colors.accent_cyan
                                        } else {
                                            theme().colors.accent_cyan.gamma_multiply(0.6)
                                        };
                                        ui.painter().circle_filled(center, 3.0, port_color);

                                        // Register port position for cables
                                        let screen_pos = center;
                                        let in_port_type = if port.port_type == synth_core::PortType::Audio {
                                            WidgetPortType::Audio
                                        } else {
                                            WidgetPortType::Control
                                        };
                                        self.port_positions.insert(
                                            (module_id, port.name),
                                            PortPosition {
                                                module_id,
                                                port_name: port.name,
                                                position: screen_pos,
                                                direction: WidgetPortDirection::Input,
                                                port_type: in_port_type,
                                            },
                                        );

                                        // Handle port interaction for cable dragging
                                        if port_resp.drag_started() {
                                            self.pending_connection = Some(PendingConnection {
                                                from_module: module_id,
                                                from_port: port.name,
                                                from_direction: WidgetPortDirection::Input,
                                                from_type: in_port_type,
                                                from_position: screen_pos,
                                                current_pos: screen_pos,
                                            });
                                        }
                                    }
                                }
                            });

                            // Oscilloscope content area
                            let scope_width = panel_width - port_col_w * 2.0;
                            let scope_height = panel_height;
                            ui.vertical(|ui| {
                                ui.set_width(scope_width);
                                let vis_buffer = handle.get_visualization_buffer(module_id);
                                let samples = if let Some(buffer) = vis_buffer {
                                    buffer.read_sweep().unwrap_or_default()
                                } else {
                                    (0..128)
                                        .map(|i| {
                                            let t = i as f32 / 128.0;
                                            (t * std::f32::consts::TAU * 3.0).sin() * 0.5
                                        })
                                        .collect()
                                };
                                let samples =
                                    trim_sweep_to_complete_cycles(&samples, 0.0);

                                super::widgets::draw_oscilloscope(
                                    ui,
                                    samples,
                                    scope_width,
                                    scope_height,
                                    1.0,
                                    theme().colors.accent_cyan,
                                );
                            });

                            // Right port (OUT) — tiny dot
                            ui.vertical(|ui| {
                                ui.set_width(port_col_w);
                                ui.set_height(panel_height);
                                let center = ui.available_rect_before_wrap().center();
                                for port in &descriptor.ports {
                                    if port.direction == synth_core::PortDirection::Output {
                                        let port_rect = egui::Rect::from_center_size(
                                            center,
                                            Vec2::splat(8.0),
                                        );
                                        let port_resp = ui.allocate_rect(port_rect, Sense::click_and_drag());
                                        let port_color = if port_resp.hovered() {
                                            theme().colors.accent_green
                                        } else {
                                            theme().colors.accent_green.gamma_multiply(0.6)
                                        };
                                        ui.painter().circle_filled(center, 3.0, port_color);

                                        let screen_pos = center;
                                        let out_port_type = if port.port_type == synth_core::PortType::Audio {
                                            WidgetPortType::Audio
                                        } else {
                                            WidgetPortType::Control
                                        };
                                        self.port_positions.insert(
                                            (module_id, port.name),
                                            PortPosition {
                                                module_id,
                                                port_name: port.name,
                                                position: screen_pos,
                                                direction: WidgetPortDirection::Output,
                                                port_type: out_port_type,
                                            },
                                        );

                                        if port_resp.drag_started() {
                                            self.pending_connection = Some(PendingConnection {
                                                from_module: module_id,
                                                from_port: port.name,
                                                from_direction: WidgetPortDirection::Output,
                                                from_type: out_port_type,
                                                from_position: screen_pos,
                                                current_pos: screen_pos,
                                            });
                                        }
                                    }
                                }
                            });
                        });

                        // Close button overlay (×) in top-right corner
                        let panel_rect = ui.min_rect();
                        let close_size = Vec2::new(14.0, 14.0);
                        let close_pos = Pos2::new(
                            panel_rect.right() - close_size.x - 1.0,
                            panel_rect.top() + 1.0,
                        );
                        let close_rect = Rect::from_min_size(close_pos, close_size);
                        let close_resp = ui.allocate_rect(close_rect, Sense::click());
                        let close_color = if close_resp.hovered() {
                            Color32::from_rgb(255, 100, 100)
                        } else {
                            Color32::from_rgba_premultiplied(200, 200, 200, 150)
                        };
                        ui.painter().text(
                            close_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "\u{2715}",
                            egui::FontId::proportional(10.0),
                            close_color,
                        );

                        if close_resp.clicked() {
                            // Reconnect cables around this monitor before removing
                            // Find incoming: something → this module "in"
                            let incoming: Vec<_> = self.connections.iter()
                                .filter(|c| c.to_module == module_id && c.to_port.as_str() == "in")
                                .cloned()
                                .collect();
                            // Find outgoing: this module "out" → something
                            let outgoing: Vec<_> = self.connections.iter()
                                .filter(|c| c.from_module == module_id && c.from_port.as_str() == "out")
                                .cloned()
                                .collect();

                            // Create bypass connections (from source directly to destination)
                            for inc in &incoming {
                                for out in &outgoing {
                                    let bypass = Connection::new(
                                        inc.from_module,
                                        inc.from_port,
                                        out.to_module,
                                        out.to_port,
                                    );
                                    self.connections.push(bypass);
                                    result.connections_to_add.push(bypass);
                                }
                            }

                            // Remove all connections to/from this module
                            let monitor_conns: Vec<_> = self.connections.iter()
                                .filter(|c| c.from_module == module_id || c.to_module == module_id)
                                .cloned()
                                .collect();
                            for c in &monitor_conns {
                                result.connections_to_remove.push(*c);
                            }
                            self.connections.retain(|c| c.from_module != module_id && c.to_module != module_id);

                            // Remove the module
                            open = false;
                            self.calculate_connectivity();
                        }
                    });

                    // Request continuous repaint so the waveform updates
                    ui.ctx().request_repaint();
                    return;
                }

                frame.show(ui, |ui| {
                    // Title bar: name + status icons + close button (single row)
                    ui.horizontal(|ui| {
                        // Accent color indicator
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(3.0, 14.0), Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, dimmed_accent);

                        // Module name
                        ui.label(egui::RichText::new(&title).strong().color(dimmed_accent))
                            .on_hover_text(format!("ID: {module_id}"));

                        let t = theme();
                        let button_min_size = Vec2::new(20.0, 20.0);

                        // Source indicator (no inputs)
                        if is_source {
                            ui.add(
                                egui::Button::new(
                                    egui::RichText::new("▸")
                                        .color(Color32::from_rgb(100, 200, 100))
                                        .size(10.0),
                                )
                                .frame(false)
                                .min_size(Vec2::new(14.0, 20.0)),
                            )
                            .on_hover_text("📤 Source Module\nGenerates signal (no incoming connections).\nOscillators, noise generators, etc.");
                        }

                        // Sink indicator (no outputs)
                        if is_sink {
                            ui.add(
                                egui::Button::new(
                                    egui::RichText::new("◼")
                                        .color(Color32::from_rgb(200, 100, 100))
                                        .size(10.0),
                                )
                                .frame(false)
                                .min_size(Vec2::new(14.0, 20.0)),
                            )
                            .on_hover_text("📥 Sink Module\nConsumes signal (no outgoing connections).\nOutput, visualizers, etc.");
                        }

                        // Connectivity status indicator
                        let (conn_icon, conn_color, conn_tooltip) = if is_global_module {
                            let tooltip =
                                if descriptor.category == ModuleCategory::Utility {
                                    "⚡ Internal Routing\nRoutes modulation internally — no cables needed."
                                } else {
                                    "⚡ Global Module\nProcessed automatically via effect chain."
                                };
                            ("◆", Color32::from_rgb(100, 180, 220), tooltip)
                        } else {
                            match connectivity_status {
                                ModuleConnectivity::Connected => (
                                    "◆",
                                    Color32::from_rgb(100, 200, 100),
                                    "🔗 Routed to Output\nAudio from this module reaches the output.",
                                ),
                                ModuleConnectivity::Orphaned => (
                                    "◇",
                                    Color32::from_rgb(200, 200, 100),
                                    "⚠ Orphaned\nHas connections but signal doesn't reach output.\nConnect to a module that leads to Output.",
                                ),
                                ModuleConnectivity::Disconnected => (
                                    "◇",
                                    Color32::from_rgb(100, 100, 100),
                                    "⊘ Disconnected\nNo cables connected.\nDrag from ports to create connections.",
                                ),
                            }
                        };
                        ui.add(
                            egui::Button::new(
                                egui::RichText::new(conn_icon)
                                    .color(conn_color)
                                    .size(12.0),
                            )
                            .frame(false)
                            .min_size(button_min_size),
                        )
                        .on_hover_text(conn_tooltip);

                        // Power/bypass button
                        let (power_icon, power_color) = if is_bypassed {
                            ("○", t.colors.text_dim)
                        } else {
                            ("●", t.colors.accent_green)
                        };
                        let power_tooltip = if is_bypassed {
                            "🔇 Bypassed\nModule output is muted.\nClick to activate."
                        } else {
                            "🔊 Active\nModule is processing audio.\nClick to bypass."
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(power_icon)
                                        .color(power_color)
                                        .size(14.0),
                                )
                                .frame(false)
                                .min_size(button_min_size),
                            )
                            .on_hover_text(power_tooltip)
                            .clicked()
                        {
                            let new_bypass_state = !is_bypassed;
                            self.bypassed.insert(module_id, new_bypass_state);
                            result.bypass_toggles.push((module_id, new_bypass_state));
                        }

                        // Close button (only for disconnected modules)
                        if connectivity_status == ModuleConnectivity::Disconnected {
                            ui.separator();
                            if ui.small_button("\u{2715}").clicked() {
                                open = false;
                            }
                        }
                    });

                    ui.separator();

                    // Check if this is a global module (no ports to show in columns)
                    let is_global = matches!(
                        descriptor.category,
                        ModuleCategory::Effect | ModuleCategory::Visualizer
                    );

                    if is_global {
                        // Global modules: full-width content, no port columns
                        let is_effect = descriptor.category == ModuleCategory::Effect;
                        if is_effect {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("⚡ Effect Chain")
                                        .size(11.0)
                                        .color(theme().colors.text_dim),
                                );
                            });
                            ui.label(
                                egui::RichText::new("Applied automatically after voice mixing")
                                    .size(9.0)
                                    .color(theme().colors.text_dim.gamma_multiply(0.7)),
                            );
                        } else {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("📊 Visualizer")
                                        .size(11.0)
                                        .color(theme().colors.text_dim),
                                );
                            });
                            ui.label(
                                egui::RichText::new("Displays final output signal")
                                    .size(9.0)
                                    .color(theme().colors.text_dim.gamma_multiply(0.7)),
                            );
                        }
                        ui.separator();

                        if let Some(panel_state) = self.panels.get_mut(&module_id) {
                            let vis_buffer = handle.get_visualization_buffer(module_id);
                            let panel_result = draw_module_panel_params(
                                ui,
                                panel_state,
                                &descriptor,
                                accent_color,
                                vis_buffer,
                                &analysis,
                            );
                            for param in panel_result.param_changes {
                                result.param_changes.push((module_id, param));
                            }
                        }
                    } else {
                        // Normal modules: three-column layout (IN ports | content | OUT ports)
                        let col_w = theme().sizes.port_column_width;

                        ui.horizontal(|ui| {
                            // Left port column (IN) - fixed width
                            ui.vertical(|ui| {
                                ui.set_width(col_w);
                                self.draw_port_column(
                                    ui,
                                    module_id,
                                    &descriptor,
                                    WidgetPortDirection::Input,
                                    &connected_ports,
                                );
                            });

                            // Content column - auto-width from content
                            ui.vertical(|ui| {
                                if let Some(panel_state) = self.panels.get_mut(&module_id) {
                                    let vis_buffer =
                                        handle.get_visualization_buffer(module_id);
                                    let panel_result = draw_module_panel_params(
                                        ui,
                                        panel_state,
                                        &descriptor,
                                        accent_color,
                                        vis_buffer,
                                        &analysis,
                                    );
                                    for param in panel_result.param_changes {
                                        result.param_changes.push((module_id, param));
                                    }
                                }
                            });

                            // Right port column (OUT) - fixed width
                            ui.vertical(|ui| {
                                ui.set_width(col_w);
                                self.draw_port_column(
                                    ui,
                                    module_id,
                                    &descriptor,
                                    WidgetPortDirection::Output,
                                    &connected_ports,
                                );
                            });
                        });
                    }
                });
            });

            // Handle area interaction — Area always returns InnerResponse (no Option)
            // Update logical position and size from screen rect, snapped to grid
            if let Some(area_rect) = ui.ctx().memory(|mem| mem.area_rect(window_id))
                && let Some(panel_state) = self.panels.get_mut(&module_id)
            {
                let logical_pos = area_rect.min - area_origin + scroll_offset;
                panel_state.position = snap_to_grid(logical_pos);
                panel_state.size = area_rect.size();
            }

            // Bring to front on click
            if area_response.response.clicked() || area_response.response.drag_started() {
                self.selected_module = Some(module_id);
                bring_to_front = Some(module_id);
            }

            // Handle close (delete module) — triggered by close button
            if !open {
                result.modules_to_remove.push(module_id);
            }

            // Clear reposition flag after this module has been drawn
            if needs_reposition {
                self.needs_reposition.remove(&module_id);
            }
        }

        // Apply z-order change
        if let Some(id) = bring_to_front {
            self.bring_to_front(id);
        }

        // Handle port interactions for connections
        self.handle_port_interactions(ui, &mut result);

        // Draw pending connection in foreground (less sag for responsive feel)
        if let Some(ref pending) = self.pending_connection {
            let color = cable_color(pending.from_type, 180);
            let painter = ui
                .ctx()
                .layer_painter(LayerId::new(Order::Foreground, egui::Id::new("cables")));
            draw_cable_dragging(&painter, pending.from_position, pending.current_pos, color);
        }

        // Handle click on empty space to deselect
        if let Some(ref response) = canvas_response
            && response.clicked()
        {
            self.selected_module = None;
        }

        // Right-click on background (or cable) → open unified context menu
        if let Some(ref response) = canvas_response
            && response.secondary_clicked()
            && !self.port_menu_just_opened
            && let Some(screen_pos) = ui.input(|i| i.pointer.interact_pos())
        {
            // Convert screen position to world/logical position
            let world_pos = Pos2::new(
                screen_pos.x - area_origin.x + scroll_offset.x,
                screen_pos.y - area_origin.y + scroll_offset.y,
            );
            self.bg_context_menu = Some(BgContextMenuState {
                world_pos,
                screen_pos,
                cable: self.hovered_cable,
            });
            self.bg_menu_just_opened = true;
        }

        // Cancel pending connection / close context menus with escape
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_connection = None;
            self.port_context_menu = None;
            self.bg_context_menu = None;
        }

        // Draw toolbar in foreground layer (always on top, positioned relative to visible area)
        self.draw_toolbar_foreground(ui, visible_rect, &mut result);

        result
    }

    /// Draw the toolbar overlay in the foreground layer.
    fn draw_toolbar_foreground(
        &self,
        ui: &mut Ui,
        canvas_rect: Rect,
        result: &mut PatchEditorResult,
    ) {
        use egui::{Area, Frame, Order};

        Area::new(egui::Id::new("patch_toolbar"))
            .order(Order::Foreground)
            .fixed_pos(canvas_rect.min + Vec2::new(10.0, 10.0))
            .show(ui.ctx(), |ui| {
                Frame::popup(ui.style())
                    .fill(theme().colors.bg_panel.gamma_multiply(0.95))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("📐 Auto Layout").clicked() {
                                result.request_auto_layout = true;
                            }
                        });
                    });
            });
    }

    /// Draw a vertical column of ports (input or output side).
    fn draw_port_column(
        &mut self,
        ui: &mut Ui,
        module_id: ModuleId,
        descriptor: &ModuleDescriptor,
        direction: WidgetPortDirection,
        connected_ports: &[PortName],
    ) {
        use synth_core::PortDirection as CorePortDirection;

        let t = theme();
        let col_width = t.sizes.port_column_width;
        let spacing = t.sizes.port_vertical_spacing;

        let core_dir = match direction {
            WidgetPortDirection::Input => CorePortDirection::Input,
            WidgetPortDirection::Output => CorePortDirection::Output,
        };

        let ports: Vec<_> = descriptor
            .ports
            .iter()
            .filter(|p| p.direction == core_dir)
            .collect();

        // Check if we have a pending connection for highlighting
        let pending_info = self
            .pending_connection
            .as_ref()
            .map(|p| (p.from_module, p.from_type, p.from_direction));

        ui.vertical(|ui| {
            // Small label at top
            let label = match direction {
                WidgetPortDirection::Input => "IN",
                WidgetPortDirection::Output => "OUT",
            };
            if !ports.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(label)
                            .size(8.0)
                            .color(t.colors.text_dim),
                    );
                });
            }

            // Draw each port centered in the column
            for port in &ports {
                let port_type = convert_port_type(port.port_type);
                let is_connected = connected_ports.contains(&port.name);

                let is_highlighted = pending_info
                    .map(|(from_module, from_type, from_dir)| {
                        from_module != module_id && from_dir != direction && from_type == port_type
                    })
                    .unwrap_or(false);

                ui.vertical_centered(|ui| {
                    ui.allocate_ui(Vec2::new(col_width, spacing), |ui| {
                        ui.centered_and_justified(|ui| {
                            let (response, center) =
                                super::widgets::PortWidget::new(port_type, direction)
                                    .connected(is_connected)
                                    .highlighted(is_highlighted)
                                    .show(ui);

                            // Store port position for cable rendering
                            self.port_positions.insert(
                                (module_id, port.name),
                                PortPosition {
                                    module_id,
                                    port_name: port.name,
                                    position: center,
                                    port_type,
                                    direction,
                                },
                            );

                            // Show label and description as tooltip
                            let tooltip = if port.description.is_empty() {
                                port.label.clone()
                            } else {
                                format!("{}: {}", port.label, port.description)
                            };
                            response.on_hover_text(tooltip);
                        });
                    });
                });
            }
        });
    }

    fn draw_grid(&self, ui: &mut Ui, rect: Rect) {
        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, 0.0, theme().colors.bg_dark);

        // Grid lines
        let grid_size = GRID_SIZE;

        let grid_color = Color32::from_rgba_unmultiplied(60, 65, 75, 50);

        // Vertical lines
        let mut x = rect.left();
        while x < rect.right() {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                egui::Stroke::new(1.0, grid_color),
            );
            x += grid_size;
        }

        // Horizontal lines
        let mut y = rect.top();
        while y < rect.bottom() {
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                egui::Stroke::new(1.0, grid_color),
            );
            y += grid_size;
        }
    }

    /// Draw cables behind modules (on the scroll area's layer). Hovered cables
    /// are drawn in the foreground layer so they appear above modules with glow.
    fn draw_connections(
        &mut self,
        ui: &Ui,
        time: f64,
        bg_layer: LayerId,
        clip_rect: Rect,
        module_rects: &[Rect],
    ) {
        let bg_painter = eframe::egui::Painter::new(ui.ctx().clone(), bg_layer, clip_rect);
        let fg_painter = ui
            .ctx()
            .layer_painter(LayerId::new(Order::Foreground, egui::Id::new("cables_fg")));

        let pointer_pos = ui.input(|i| i.pointer.hover_pos());

        // Track which cable the context menu targets so we highlight it
        let menu_target = self
            .bg_context_menu
            .as_ref()
            .and_then(|s| s.cable.as_ref())
            .cloned();

        // Compute spread offsets: group cables by destination module,
        // then spread cables within each group so they don't overlap.
        let mut dest_count: HashMap<ModuleId, usize> = HashMap::new();
        for c in &self.connections {
            *dest_count.entry(c.to_module).or_default() += 1;
        }
        let mut dest_index: HashMap<ModuleId, usize> = HashMap::new();

        // Pre-compute cable spreads and positions for nearest-cable detection
        struct CableInfo {
            index: usize,
            spread: f32,
            from_pos: Pos2,
            to_pos: Pos2,
            port_type: WidgetPortType,
        }
        let mut cable_infos: Vec<CableInfo> = Vec::new();

        for (i, connection) in self.connections.iter().enumerate() {
            let idx = dest_index.entry(connection.to_module).or_default();
            let n = dest_count.get(&connection.to_module).copied().unwrap_or(1);
            let spread = (*idx as f32 - (n as f32 - 1.0) / 2.0) * CABLE_SPREAD;
            *dest_index.get_mut(&connection.to_module).unwrap_or(&mut 0) += 1;

            let from_key = (connection.from_module, connection.from_port);
            let to_key = (connection.to_module, connection.to_port);

            if let (Some(from_pos), Some(to_pos)) = (
                self.port_positions.get(&from_key),
                self.port_positions.get(&to_key),
            ) {
                cable_infos.push(CableInfo {
                    index: i,
                    spread,
                    from_pos: from_pos.position,
                    to_pos: to_pos.position,
                    port_type: from_pos.port_type,
                });
            }
        }

        // Find the single nearest cable to the pointer (exclusive hover)
        let nearest_cable_idx: Option<usize> = pointer_pos.and_then(|p| {
            let over_module = module_rects.iter().any(|r| r.contains(p));

            let mut best_idx: Option<usize> = None;
            let mut best_dist = f32::MAX;

            for info in &cable_infos {
                let near_port = {
                    let to_from = (p - info.from_pos).length();
                    let to_to = (p - info.to_pos).length();
                    to_from < 15.0 || to_to < 15.0
                };

                if !near_port && over_module {
                    continue;
                }

                if point_near_cable(p, info.from_pos, info.to_pos, 20.0, info.spread) {
                    let snap = closest_point_on_cable(p, info.from_pos, info.to_pos, info.spread);
                    let dist = (p - snap).length();
                    if dist < best_dist {
                        best_dist = dist;
                        best_idx = Some(info.index);
                    }
                }
            }

            best_idx
        });

        // Update hovered_cable state for right-click handling
        self.hovered_cable = nearest_cable_idx.map(|i| self.connections[i]);

        // Draw all cables
        for info in &cable_infos {
            let connection = &self.connections[info.index];
            let color = cable_color(info.port_type, 180);

            let is_nearest = nearest_cable_idx == Some(info.index);
            let show_highlight = is_nearest || menu_target.as_ref() == Some(connection);

            if show_highlight {
                // Highlighted cable in foreground (above modules)
                draw_cable_highlighted(&fg_painter, info.from_pos, info.to_pos, color, info.spread);
            } else {
                // Normal cable behind modules
                draw_cable(&bg_painter, info.from_pos, info.to_pos, color, info.spread);
            }

            // Animated flow particles behind modules
            draw_flow_particles(
                &bg_painter,
                info.from_pos,
                info.to_pos,
                color,
                info.port_type,
                time,
                info.spread,
            );
        }
    }

    /// Draw the background right-click context menu for adding modules.
    #[allow(clippy::too_many_lines)]
    fn draw_bg_context_menu(&mut self, ui: &Ui, result: &mut PatchEditorResult) {
        let Some(ref menu_state) = self.bg_context_menu else {
            self.bg_menu_just_opened = false;
            return;
        };
        let world_pos = menu_state.world_pos;
        let menu_screen_pos = menu_state.screen_pos;
        let menu_cable = menu_state.cable;

        let menu_id = egui::Id::new("bg_context_menu");
        let mut selected: Option<PaletteSelection> = None;
        let mut cable_action_taken = false;

        let area_resp = egui::Area::new(menu_id)
            .order(Order::Foreground)
            .fixed_pos(menu_screen_pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .fill(theme().colors.bg_panel)
                    .show(ui, |ui| {
                        ui.set_min_width(150.0);

                        // Cable actions (shown when right-clicking on a hovered cable)
                        if let Some(connection) = menu_cable {
                            if ui.button("Delete cable").clicked() {
                                self.connections.retain(|c| c != &connection);
                                result.connections_to_remove.push(connection);
                                self.calculate_connectivity();
                                cable_action_taken = true;
                            }

                            if ui.button("Insert Signal Monitor").clicked() {
                                self.connections.retain(|c| c != &connection);
                                result.connections_to_remove.push(connection);
                                result.insert_signal_monitor_at.push(connection);
                                self.calculate_connectivity();
                                cable_action_taken = true;
                            }

                            ui.separator();

                            ui.label(
                                egui::RichText::new("Insert module...")
                                    .color(theme().colors.text_secondary)
                                    .size(11.0),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new("Add module")
                                    .color(theme().colors.text_secondary)
                                    .size(11.0),
                            );
                        }
                        ui.separator();

                        // Oscillator submenu
                        let osc_color = category_color(ModuleCategory::Oscillator);
                        ui.menu_button(
                            egui::RichText::new("🎵 Oscillator").color(osc_color),
                            |ui| {
                                Self::bg_menu_item(
                                    ui,
                                    "🎵 Basic",
                                    PaletteSelection::Category(ModuleCategory::Oscillator),
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🔢 Math",
                                    PaletteSelection::MathOscillator,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🔈 Sub",
                                    PaletteSelection::SubOscillator,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🌫 Noise",
                                    PaletteSelection::Noise,
                                    &mut selected,
                                );
                                ui.separator();
                                Self::bg_menu_item(
                                    ui,
                                    "📊 Wavetable",
                                    PaletteSelection::WavetableOsc,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🎶 Additive",
                                    PaletteSelection::AdditiveOsc,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🌾 Granular",
                                    PaletteSelection::GranularOsc,
                                    &mut selected,
                                );
                            },
                        );

                        // Simple categories (single module type each)
                        let categories = [
                            (ModuleCategory::Filter, "🔊 Filter"),
                            (ModuleCategory::Envelope, "📈 Envelope"),
                            (ModuleCategory::LFO, "〰 LFO"),
                            (ModuleCategory::Amplifier, "🔉 VCA"),
                            (ModuleCategory::Mixer, "🎚 Mixer"),
                        ];
                        for (cat, label) in categories {
                            let color = category_color(cat);
                            if ui.button(egui::RichText::new(label).color(color)).clicked() {
                                selected = Some(PaletteSelection::Category(cat));
                            }
                        }

                        ui.separator();

                        // Effect submenu
                        let fx_color = category_color(ModuleCategory::Effect);
                        ui.menu_button(egui::RichText::new("✨ Effect").color(fx_color), |ui| {
                            Self::bg_menu_item(
                                ui,
                                "🔁 Delay",
                                PaletteSelection::Effect(EffectType::Delay),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "🌊 Reverb",
                                PaletteSelection::Effect(EffectType::Reverb),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "🔥 Distortion",
                                PaletteSelection::Effect(EffectType::Distortion),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "🎭 Chorus",
                                PaletteSelection::Effect(EffectType::Chorus),
                                &mut selected,
                            );
                            ui.separator();
                            Self::bg_menu_item(
                                ui,
                                "🌀 Flanger",
                                PaletteSelection::Effect(EffectType::Flanger),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "🔄 Phaser",
                                PaletteSelection::Effect(EffectType::Phaser),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "📊 Compressor",
                                PaletteSelection::Effect(EffectType::Compressor),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "🎛 EQ",
                                PaletteSelection::Effect(EffectType::Eq),
                                &mut selected,
                            );
                            ui.separator();
                            Self::bg_menu_item(
                                ui,
                                "🔊 Waveshaper",
                                PaletteSelection::Effect(EffectType::Waveshaper),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "📼 BBD Delay",
                                PaletteSelection::Effect(EffectType::BbdDelay),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "🧱 Limiter",
                                PaletteSelection::Effect(EffectType::Limiter),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "↔ Mid/Side",
                                PaletteSelection::Effect(EffectType::MidSide),
                                &mut selected,
                            );
                            ui.separator();
                            Self::bg_menu_item(
                                ui,
                                "🏛 Convolver",
                                PaletteSelection::Effect(EffectType::Convolver),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "🔬 Phase Vocoder",
                                PaletteSelection::Effect(EffectType::PhaseVocoder),
                                &mut selected,
                            );
                            Self::bg_menu_item(
                                ui,
                                "🔀 Freq Shifter",
                                PaletteSelection::Effect(EffectType::FrequencyShifter),
                                &mut selected,
                            );
                        });

                        // Visualizer submenu
                        let viz_color = category_color(ModuleCategory::Visualizer);
                        ui.menu_button(
                            egui::RichText::new("📊 Visualizer").color(viz_color),
                            |ui| {
                                Self::bg_menu_item(
                                    ui,
                                    "📈 Oscilloscope",
                                    PaletteSelection::Visualizer(
                                        PaletteVisualizerType::Oscilloscope,
                                    ),
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "📊 Level Meter",
                                    PaletteSelection::Visualizer(PaletteVisualizerType::LevelMeter),
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "📊 Spectrum",
                                    PaletteSelection::Visualizer(
                                        PaletteVisualizerType::SpectrumAnalyzer,
                                    ),
                                    &mut selected,
                                );
                                ui.separator();
                                Self::bg_menu_item(
                                    ui,
                                    "🔍 Signal Monitor",
                                    PaletteSelection::SignalMonitor,
                                    &mut selected,
                                );
                            },
                        );

                        // Modulation submenu
                        let mod_color = category_color(ModuleCategory::Utility);
                        ui.menu_button(
                            egui::RichText::new("🔀 Modulation").color(mod_color),
                            |ui| {
                                Self::bg_menu_item(
                                    ui,
                                    "🔔 Ring Mod",
                                    PaletteSelection::RingMod,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "📈 Env Follower",
                                    PaletteSelection::EnvelopeFollower,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "📐 MSEG",
                                    PaletteSelection::Mseg,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🏃 Kinetic Mod",
                                    PaletteSelection::KineticModulator,
                                    &mut selected,
                                );
                            },
                        );

                        // Generative submenu
                        let gen_color = category_color(ModuleCategory::LFO);
                        ui.menu_button(
                            egui::RichText::new("🎲 Generative").color(gen_color),
                            |ui| {
                                Self::bg_menu_item(
                                    ui,
                                    "⊕ Euclidean",
                                    PaletteSelection::Euclidean,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🔀 Turing Machine",
                                    PaletteSelection::TuringMachine,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🎲 Random Gates",
                                    PaletteSelection::RandomGates,
                                    &mut selected,
                                );
                            },
                        );

                        // Physical submenu
                        let phys_color = category_color(ModuleCategory::PhysicalModeling);
                        ui.menu_button(
                            egui::RichText::new("🎹 Physical").color(phys_color),
                            |ui| {
                                Self::bg_menu_item(
                                    ui,
                                    "🎹 Keyboard Panner",
                                    PaletteSelection::KeyboardPanner,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🪵 Body Resonance",
                                    PaletteSelection::BodyResonance,
                                    &mut selected,
                                );
                                Self::bg_menu_item(
                                    ui,
                                    "🔧 Mechanical Noise",
                                    PaletteSelection::MechanicalNoise,
                                    &mut selected,
                                );
                            },
                        );

                        ui.separator();

                        // Output & Mod Matrix as direct buttons
                        let out_color = category_color(ModuleCategory::Output);
                        if ui
                            .button(egui::RichText::new("🔈 Output").color(out_color))
                            .clicked()
                        {
                            selected = Some(PaletteSelection::StereoOutput);
                        }
                        let util_color = category_color(ModuleCategory::Utility);
                        if ui
                            .button(egui::RichText::new("🔀 Mod Matrix").color(util_color))
                            .clicked()
                        {
                            selected = Some(PaletteSelection::ModMatrix);
                        }
                    });
            });

        if cable_action_taken {
            self.bg_context_menu = None;
            self.bg_menu_just_opened = false;
            return;
        }

        if let Some(sel) = selected {
            result.context_add = Some((sel, world_pos, menu_cable));
            self.bg_context_menu = None;
            self.bg_menu_just_opened = false;
            return;
        }

        // On the frame the menu was opened, skip the close check
        if self.bg_menu_just_opened {
            self.bg_menu_just_opened = false;
            return;
        }

        // Close on any click outside the menu area
        let menu_rect = area_resp.response.rect;
        let pointer_pos = ui.input(|i| i.pointer.interact_pos());
        let any_click = ui.input(|i| {
            i.pointer.button_clicked(egui::PointerButton::Primary)
                || i.pointer.button_clicked(egui::PointerButton::Secondary)
        });
        let clicked_outside =
            any_click && pointer_pos.map(|p| !menu_rect.contains(p)).unwrap_or(false);

        if clicked_outside {
            self.bg_context_menu = None;
        }
    }

    /// Helper for background context menu items.
    fn bg_menu_item(
        ui: &mut Ui,
        label: &str,
        selection: PaletteSelection,
        out: &mut Option<PaletteSelection>,
    ) {
        if ui.button(label).clicked() {
            *out = Some(selection);
            ui.close();
        }
    }

    /// Draw the port right-click context menu for quick-adding connected modules.
    #[allow(clippy::too_many_lines)]
    fn draw_port_context_menu(&mut self, ui: &Ui, result: &mut PatchEditorResult) {
        let Some(ref state) = self.port_context_menu else {
            self.port_menu_just_opened = false;
            return;
        };

        let menu_id = egui::Id::new("port_context_menu");
        let mut close_menu = false;

        // Clone what we need before mutable borrow
        let target_module = state.module_id;
        let target_port = state.port_name;
        let target_direction = state.direction;
        let port_type = state.port_type;
        let menu_pos = state.menu_pos;

        // Position: new module to the left of input ports, right of output ports
        let offset_x = match target_direction {
            WidgetPortDirection::Input => -220.0,
            WidgetPortDirection::Output => 220.0,
        };
        let new_module_pos = Pos2::new(menu_pos.x + offset_x, menu_pos.y - 50.0);

        let area_resp = egui::Area::new(menu_id)
            .order(Order::Foreground)
            .fixed_pos(menu_pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .fill(theme().colors.bg_panel)
                    .show(ui, |ui| {
                        ui.set_min_width(160.0);

                        let header = match target_direction {
                            WidgetPortDirection::Input => "Add source",
                            WidgetPortDirection::Output => "Add target",
                        };
                        ui.label(
                            egui::RichText::new(header)
                                .color(theme().colors.text_secondary)
                                .size(11.0),
                        );
                        ui.separator();

                        // Build menu items based on port type + direction
                        match target_direction {
                            WidgetPortDirection::Input => {
                                // Input port: show source modules
                                match port_type {
                                    WidgetPortType::Audio => {
                                        self.port_menu_items(
                                            ui,
                                            result,
                                            &mut close_menu,
                                            target_module,
                                            target_port,
                                            target_direction,
                                            new_module_pos,
                                            &[
                                                (
                                                    "Oscillator",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Oscillator,
                                                    ),
                                                ),
                                                ("Sub Osc", PaletteSelection::SubOscillator),
                                                ("Wavetable", PaletteSelection::WavetableOsc),
                                                ("Math Osc", PaletteSelection::MathOscillator),
                                                ("Additive", PaletteSelection::AdditiveOsc),
                                                ("Granular", PaletteSelection::GranularOsc),
                                                ("Noise", PaletteSelection::Noise),
                                                ("Ring Mod", PaletteSelection::RingMod),
                                            ],
                                        );
                                    }
                                    WidgetPortType::Control => {
                                        self.port_menu_items(
                                            ui,
                                            result,
                                            &mut close_menu,
                                            target_module,
                                            target_port,
                                            target_direction,
                                            new_module_pos,
                                            &[
                                                (
                                                    "LFO",
                                                    PaletteSelection::Category(ModuleCategory::LFO),
                                                ),
                                                (
                                                    "Envelope",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Envelope,
                                                    ),
                                                ),
                                                ("MSEG", PaletteSelection::Mseg),
                                                ("Kinetic Mod", PaletteSelection::KineticModulator),
                                                (
                                                    "Envelope Follower",
                                                    PaletteSelection::EnvelopeFollower,
                                                ),
                                            ],
                                        );
                                    }
                                    WidgetPortType::Gate => {
                                        self.port_menu_items(
                                            ui,
                                            result,
                                            &mut close_menu,
                                            target_module,
                                            target_port,
                                            target_direction,
                                            new_module_pos,
                                            &[
                                                ("Euclidean", PaletteSelection::Euclidean),
                                                ("Turing Machine", PaletteSelection::TuringMachine),
                                                ("Random Gates", PaletteSelection::RandomGates),
                                            ],
                                        );
                                    }
                                    WidgetPortType::Midi => {}
                                }
                            }
                            WidgetPortDirection::Output => {
                                // Output port: show destination modules
                                match port_type {
                                    WidgetPortType::Audio => {
                                        self.port_menu_items(
                                            ui,
                                            result,
                                            &mut close_menu,
                                            target_module,
                                            target_port,
                                            target_direction,
                                            new_module_pos,
                                            &[
                                                (
                                                    "Filter",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Filter,
                                                    ),
                                                ),
                                                (
                                                    "VCA",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Amplifier,
                                                    ),
                                                ),
                                                (
                                                    "Mixer",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Mixer,
                                                    ),
                                                ),
                                                ("Signal Monitor", PaletteSelection::SignalMonitor),
                                            ],
                                        );
                                        ui.separator();
                                        self.port_menu_items(
                                            ui,
                                            result,
                                            &mut close_menu,
                                            target_module,
                                            target_port,
                                            target_direction,
                                            new_module_pos,
                                            &[
                                                (
                                                    "Delay",
                                                    PaletteSelection::Effect(EffectType::Delay),
                                                ),
                                                (
                                                    "Reverb",
                                                    PaletteSelection::Effect(EffectType::Reverb),
                                                ),
                                                (
                                                    "Distortion",
                                                    PaletteSelection::Effect(
                                                        EffectType::Distortion,
                                                    ),
                                                ),
                                                (
                                                    "Chorus",
                                                    PaletteSelection::Effect(EffectType::Chorus),
                                                ),
                                                (
                                                    "Flanger",
                                                    PaletteSelection::Effect(EffectType::Flanger),
                                                ),
                                                (
                                                    "Phaser",
                                                    PaletteSelection::Effect(EffectType::Phaser),
                                                ),
                                                (
                                                    "Compressor",
                                                    PaletteSelection::Effect(
                                                        EffectType::Compressor,
                                                    ),
                                                ),
                                                ("EQ", PaletteSelection::Effect(EffectType::Eq)),
                                                (
                                                    "Waveshaper",
                                                    PaletteSelection::Effect(
                                                        EffectType::Waveshaper,
                                                    ),
                                                ),
                                            ],
                                        );
                                    }
                                    WidgetPortType::Control => {
                                        self.port_menu_items(
                                            ui,
                                            result,
                                            &mut close_menu,
                                            target_module,
                                            target_port,
                                            target_direction,
                                            new_module_pos,
                                            &[
                                                (
                                                    "VCA",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Amplifier,
                                                    ),
                                                ),
                                                (
                                                    "Filter",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Filter,
                                                    ),
                                                ),
                                                (
                                                    "Oscillator",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Oscillator,
                                                    ),
                                                ),
                                            ],
                                        );
                                    }
                                    WidgetPortType::Gate => {
                                        self.port_menu_items(
                                            ui,
                                            result,
                                            &mut close_menu,
                                            target_module,
                                            target_port,
                                            target_direction,
                                            new_module_pos,
                                            &[
                                                (
                                                    "Envelope",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Envelope,
                                                    ),
                                                ),
                                                (
                                                    "VCA",
                                                    PaletteSelection::Category(
                                                        ModuleCategory::Amplifier,
                                                    ),
                                                ),
                                            ],
                                        );
                                    }
                                    WidgetPortType::Midi => {}
                                }
                            }
                        }
                    });
            });

        // Same-frame guard
        if self.port_menu_just_opened {
            self.port_menu_just_opened = false;
            return;
        }

        // Close on click outside
        let menu_rect = area_resp.response.rect;
        let pointer_pos = ui.input(|i| i.pointer.interact_pos());
        let any_click = ui.input(|i| {
            i.pointer.button_clicked(egui::PointerButton::Primary)
                || i.pointer.button_clicked(egui::PointerButton::Secondary)
        });
        let clicked_outside =
            any_click && pointer_pos.map(|p| !menu_rect.contains(p)).unwrap_or(false);

        if close_menu || clicked_outside {
            self.port_context_menu = None;
        }
    }

    /// Helper: render a list of menu buttons that push `QuickAddRequest`s.
    #[allow(clippy::too_many_arguments)]
    fn port_menu_items(
        &self,
        ui: &mut Ui,
        result: &mut PatchEditorResult,
        close_menu: &mut bool,
        target_module: ModuleId,
        target_port: PortName,
        target_direction: WidgetPortDirection,
        position: Pos2,
        items: &[(&str, PaletteSelection)],
    ) {
        for (label, selection) in items {
            if ui.button(*label).clicked() {
                result.quick_add_requests.push(QuickAddRequest {
                    selection: *selection,
                    target_module,
                    target_port,
                    target_direction,
                    position,
                });
                *close_menu = true;
            }
        }
    }

    fn handle_port_interactions(&mut self, ui: &mut Ui, result: &mut PatchEditorResult) {
        let pointer_pos = ui.input(|i| i.pointer.interact_pos());

        // Check for port clicks
        for ((module_id, port_name), port_pos) in &self.port_positions {
            let port_rect = Rect::from_center_size(port_pos.position, Vec2::splat(20.0));

            if let Some(pos) = pointer_pos
                && port_rect.contains(pos)
            {
                // Check for click
                if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary)) {
                    if let Some(ref pending) = self.pending_connection {
                        // Complete connection
                        if self.can_connect(pending, port_pos) {
                            let connection =
                                if pending.from_direction == WidgetPortDirection::Output {
                                    Connection::new(
                                        pending.from_module,
                                        pending.from_port,
                                        *module_id,
                                        *port_name,
                                    )
                                } else {
                                    Connection::new(
                                        *module_id,
                                        *port_name,
                                        pending.from_module,
                                        pending.from_port,
                                    )
                                };
                            result.connections_to_add.push(connection);
                        }
                        self.pending_connection = None;
                    } else {
                        // Start new connection
                        self.pending_connection = Some(PendingConnection {
                            from_module: *module_id,
                            from_port: *port_name,
                            from_position: port_pos.position,
                            from_type: port_pos.port_type,
                            from_direction: port_pos.direction,
                            current_pos: pos,
                        });
                    }
                }

                // Right-click opens port context menu
                if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary)) {
                    self.port_context_menu = Some(PortContextMenuState {
                        module_id: *module_id,
                        port_name: *port_name,
                        port_type: port_pos.port_type,
                        direction: port_pos.direction,
                        menu_pos: pos,
                    });
                    self.port_menu_just_opened = true;
                }
            }
        }

        // Update pending connection position
        if let Some(ref mut pending) = self.pending_connection
            && let Some(pos) = pointer_pos
        {
            pending.current_pos = pos;
        }
    }

    fn can_connect(&self, pending: &PendingConnection, target: &PortPosition) -> bool {
        // Can't connect to same module
        if pending.from_module == target.module_id {
            return false;
        }

        // Must connect output to input or input to output
        if pending.from_direction == target.direction {
            return false;
        }

        // Must match port types
        if pending.from_type != target.port_type {
            return false;
        }

        true
    }

    /// Get selected module ID.
    #[allow(dead_code)]
    pub fn selected_module(&self) -> Option<ModuleId> {
        self.selected_module
    }

    /// Check if a module exists.
    #[allow(dead_code)]
    pub fn has_module(&self, id: ModuleId) -> bool {
        self.panels.contains_key(&id)
    }

    /// Get all module IDs.
    pub fn module_ids(&self) -> Vec<ModuleId> {
        self.panels.keys().copied().collect()
    }

    /// Get a module's descriptor.
    pub fn module_descriptor(&self, id: ModuleId) -> Option<&ModuleDescriptor> {
        self.descriptors.get(&id)
    }

    /// Calculate connectivity status for all modules.
    /// Uses BFS backwards from output modules to determine which modules
    /// are actually connected to audio output.
    pub fn calculate_connectivity(&mut self) {
        self.connectivity.clear();

        // Find all output modules (sinks that produce audio)
        let output_modules: Vec<ModuleId> = self
            .panels
            .keys()
            .filter(|&&id| {
                self.descriptors
                    .get(&id)
                    .map(|d| d.category == ModuleCategory::Output)
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        // Build reverse adjacency map (to -> from)
        let mut reverse_adj: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();
        for id in self.panels.keys() {
            reverse_adj.insert(*id, Vec::new());
        }
        for conn in &self.connections {
            if let Some(adj) = reverse_adj.get_mut(&conn.to_module) {
                adj.push(conn.from_module);
            }
        }

        // BFS backwards from output modules
        let mut connected: HashSet<ModuleId> = HashSet::new();
        let mut queue: VecDeque<ModuleId> = VecDeque::new();

        for &output_id in &output_modules {
            connected.insert(output_id);
            queue.push_back(output_id);
        }

        while let Some(id) = queue.pop_front() {
            if let Some(sources) = reverse_adj.get(&id) {
                for &source_id in sources {
                    if !connected.contains(&source_id) {
                        connected.insert(source_id);
                        queue.push_back(source_id);
                    }
                }
            }
        }

        // Determine connectivity status for each module
        for &id in self.panels.keys() {
            let has_any_connection = self
                .connections
                .iter()
                .any(|c| c.from_module == id || c.to_module == id);

            let status = if connected.contains(&id) {
                ModuleConnectivity::Connected
            } else if has_any_connection {
                ModuleConnectivity::Orphaned
            } else {
                ModuleConnectivity::Disconnected
            };

            self.connectivity.insert(id, status);
        }
    }

    /// Get connectivity status for a module.
    pub fn get_connectivity(&self, id: ModuleId) -> ModuleConnectivity {
        self.connectivity
            .get(&id)
            .copied()
            .unwrap_or(ModuleConnectivity::Disconnected)
    }

    /// Get bypass state for a module.
    /// Returns true if bypassed (off), false if active (on).
    pub fn is_bypassed(&self, id: ModuleId) -> bool {
        self.bypassed.get(&id).copied().unwrap_or(false)
    }

    /// Set bypass state for a module.
    /// `bypassed = true` means the module is off/bypassed.
    /// `bypassed = false` means the module is active/on.
    pub fn set_bypassed(&mut self, id: ModuleId, bypassed: bool) {
        self.bypassed.insert(id, bypassed);
    }

    /// Request auto-layout on the next frame (e.g. after loading a patch).
    pub fn request_initial_layout(&mut self) {
        self.needs_initial_layout = true;
    }

    /// Apply automatic layout to modules based on signal flow.
    ///
    /// `available_rect` should be the area available for modules (excluding side panels).
    pub fn apply_auto_layout(&mut self, available_rect: egui::Rect) {
        use super::auto_layout::{LayoutConnection, ModuleInfo, calculate_layout};

        // Collect module info
        let modules: Vec<ModuleInfo> = self
            .panels
            .iter()
            .filter_map(|(&id, panel)| {
                self.descriptors.get(&id).map(|desc| ModuleInfo {
                    id,
                    category: desc.category,
                    size: panel.size,
                })
            })
            .collect();

        // Collect connections
        let connections: Vec<LayoutConnection> = self
            .connections
            .iter()
            .map(|conn| LayoutConnection {
                from_module: conn.from_module,
                to_module: conn.to_module,
            })
            .collect();

        let layout_rect = available_rect;

        let result = calculate_layout(&modules, &connections, layout_rect);

        // Apply new positions and mark for repositioning
        for (module_id, position) in result.positions {
            if let Some(panel) = self.panels.get_mut(&module_id) {
                panel.position = position;
                // Mark this module to use current_pos() on next frame
                self.needs_reposition.insert(module_id);
            }
        }
    }
}

impl Default for PatchEditor {
    fn default() -> Self {
        Self::new()
    }
}

/// Result from drawing the rack view.
#[derive(Default)]
pub struct PatchEditorResult {
    /// Parameter changes to send to engine.
    /// Each entry is (module_id, Param with value).
    pub param_changes: Vec<(ModuleId, Param)>,
    /// Modules to remove.
    pub modules_to_remove: Vec<ModuleId>,
    /// Connections to add.
    pub connections_to_add: Vec<Connection>,
    /// Connections to remove.
    pub connections_to_remove: Vec<Connection>,
    /// Request auto-layout of modules.
    pub request_auto_layout: bool,
    /// The available canvas area (for auto-layout calculations).
    pub canvas_rect: Option<egui::Rect>,
    /// Bypass state toggles (module_id, new_bypass_state).
    /// true = bypassed (module is off), false = active (module is on).
    pub bypass_toggles: Vec<(ModuleId, bool)>,
    /// Connections where a Signal Monitor should be inserted.
    /// The backend creates the module and rewires: from→monitor→to.
    pub insert_signal_monitor_at: Vec<Connection>,
    /// Requests to create a new module and auto-connect to a port.
    pub quick_add_requests: Vec<QuickAddRequest>,
    /// Context menu request: add a module at a specific world position.
    /// Third value = cable to break and insert the module inline (if from cable context menu).
    pub context_add: Option<(PaletteSelection, Pos2, Option<Connection>)>,
}

/// Simplified panel result for parameters only.
pub struct PanelParamsResult {
    /// Parameter changes - each Param carries its own value.
    pub param_changes: Vec<Param>,
}

/// Draw visualizer display (oscilloscope or level meter).
/// This is called for Visualizer category modules to show the visualization prominently.
fn draw_visualizer_display(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    vis_buffer: Option<&synth_engine::visualizers::VisualizationBuffer>,
    _param_changes: &mut Vec<Param>,
) {
    // Check which type of visualizer based on module type id
    if descriptor.type_id.0 == "oscilloscope" {
        // Get gain from params by name
        let gain = state.param_values.get("Gain").copied().unwrap_or(1.0);

        // Get samples from visualization buffer if available
        let samples = if let Some(buffer) = vis_buffer {
            let (left, _right) = buffer.read_samples();
            // Downsample to 256 points for display
            let step = left.len().max(1) / 256;
            if step > 0 {
                left.into_iter().step_by(step.max(1)).take(256).collect()
            } else {
                left
            }
        } else {
            // Demo waveform if no buffer connected
            (0..256)
                .map(|i| {
                    let t = i as f32 / 256.0;
                    (t * std::f32::consts::TAU * 3.0).sin() * 0.5
                })
                .collect()
        };

        // Use available width, with min/max constraints
        let width = ui.available_width().clamp(120.0, 300.0);
        let height = (width * 0.5).clamp(60.0, 120.0);

        super::widgets::draw_oscilloscope(
            ui,
            &samples,
            width,
            height,
            gain,
            theme().colors.accent_cyan,
        );

        if vis_buffer.is_none() {
            ui.label(
                egui::RichText::new("No signal")
                    .small()
                    .color(theme().colors.text_dim),
            );
        }
    } else if descriptor.type_id.0 == "spectrum_analyzer" {
        // Get gain from params by name
        let gain = state.param_values.get("Gain").copied().unwrap_or(1.0);

        // Get magnitude data from visualization buffer
        let magnitudes = if let Some(buffer) = vis_buffer {
            let (left, _right) = buffer.read_samples();
            left
        } else {
            // Demo flat spectrum if no buffer connected
            vec![0.0; 256]
        };

        // Use available width, with min/max constraints
        let width = ui.available_width().clamp(120.0, 300.0);
        let height = (width * 0.5).clamp(60.0, 120.0);

        super::widgets::draw_spectrum_analyzer(
            ui,
            &magnitudes,
            width,
            height,
            gain,
            theme().colors.accent_green,
        );

        if vis_buffer.is_none() {
            ui.label(
                egui::RichText::new("No signal")
                    .small()
                    .color(theme().colors.text_dim),
            );
        }
    } else if descriptor.type_id.0 == "level_meter" {
        // Get levels from visualization buffer if available
        let (peak_l, peak_r, rms_l, rms_r) = if let Some(buffer) = vis_buffer {
            let (peak_l, peak_r) = buffer.get_peaks();
            let (rms_l, rms_r) = buffer.get_rms();
            (peak_l, peak_r, rms_l, rms_r)
        } else {
            // Demo levels if no buffer connected
            (0.0, 0.0, 0.0, 0.0)
        };

        // Use available width for meter, with constraints
        let width = ui.available_width().clamp(60.0, 120.0);
        let height = (width * 1.5).clamp(80.0, 150.0);

        super::widgets::draw_stereo_meter(ui, peak_l, peak_r, rms_l, rms_r, width, height);

        if vis_buffer.is_none() {
            ui.label(
                egui::RichText::new("No signal")
                    .small()
                    .color(theme().colors.text_dim),
            );
        }
    }
}

/// Draw only the parameters section of a module panel.
fn draw_module_panel_params(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    accent_color: Color32,
    vis_buffer: Option<&synth_engine::visualizers::VisualizationBuffer>,
    analysis: &PatchAnalysis,
) -> PanelParamsResult {
    use super::widgets::{EnvelopeEditor, Knob, WaveformSelector};
    use synth_core::WidgetHint;

    let mut param_changes = Vec::new();

    // For Visualizer modules, draw visualization FIRST (before parameters)
    if descriptor.category == ModuleCategory::Visualizer {
        draw_visualizer_display(ui, state, descriptor, vis_buffer, &mut param_changes);
        // Skip regular parameter drawing for visualizers - the display is the main UI
        return PanelParamsResult { param_changes };
    }

    // Special handling for Envelope modules - use interactive EnvelopeEditor
    if descriptor.category == ModuleCategory::Envelope {
        // Get current ADSR values
        let mut attack = state.param_values.get("Attack").copied().unwrap_or(0.01);
        let mut decay = state.param_values.get("Decay").copied().unwrap_or(0.1);
        let mut sustain = state.param_values.get("Sustain").copied().unwrap_or(0.7);
        let mut release = state.param_values.get("Release").copied().unwrap_or(0.3);

        // Get envelope playback position (lock-free)
        let envelope_pos = state.envelope_position.as_ref().map(|buf| buf.get());

        // Draw the interactive envelope editor
        ui.add_space(4.0);
        let width = ui.available_width().clamp(150.0, 250.0);
        let height = (width * 0.5).clamp(80.0, 120.0);

        let mut editor = EnvelopeEditor::new(&mut attack, &mut decay, &mut sustain, &mut release)
            .accent_color(accent_color)
            .size(width, height)
            .max_time(10.0);

        // Add playback position if available
        if let Some((stage, level)) = envelope_pos {
            editor = editor.playback_position(stage, level);
        }

        if let Some(changes) = editor.show(ui) {
            // Find parameter descriptors and push changes
            for param in &descriptor.parameters {
                if param.name == "Attack" && changes.attack.is_some() {
                    state.param_values.insert("Attack".to_string(), attack);
                    param_changes.push(param.id.with_f32(attack));
                }
                if param.name == "Decay" && changes.decay.is_some() {
                    state.param_values.insert("Decay".to_string(), decay);
                    param_changes.push(param.id.with_f32(decay));
                }
                if param.name == "Sustain" && changes.sustain.is_some() {
                    state.param_values.insert("Sustain".to_string(), sustain);
                    param_changes.push(param.id.with_f32(sustain));
                }
                if param.name == "Release" && changes.release.is_some() {
                    state.param_values.insert("Release".to_string(), release);
                    param_changes.push(param.id.with_f32(release));
                }
            }
        }

        ui.add_space(4.0);

        // Only show knob parameters for Envelope (Vel Sens, curves etc)
        let knob_params: Vec<_> = descriptor
            .parameters
            .iter()
            .filter(|p| matches!(p.widget_hint, WidgetHint::Knob))
            .collect();

        if !knob_params.is_empty() {
            const KNOBS_PER_ROW: usize = 5;
            for chunk in knob_params.chunks(KNOBS_PER_ROW) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    for param in chunk {
                        ui.vertical(|ui| {
                            let current = state
                                .param_values
                                .get(&param.name)
                                .copied()
                                .unwrap_or(param.range.default);
                            let mut value = current;

                            Knob::from_descriptor(&mut value, param)
                                .size(theme().sizes.knob_size)
                                .accent_color(accent_color)
                                .show(ui);

                            if (value - current).abs() > f32::EPSILON {
                                state.param_values.insert(param.name.clone(), value);
                                param_changes.push(param.id.with_f32(value));
                            }
                        });
                    }
                });
            }
        }

        return PanelParamsResult { param_changes };
    }

    // Special handling for Mod Matrix — custom grid rendering
    if descriptor.type_id.0 == "mod_matrix" {
        return draw_mod_matrix_grid(ui, state, descriptor, accent_color, analysis);
    }

    // Signal Monitor — draw oscilloscope display above parameters
    if descriptor.type_id.0 == "signal_monitor" {
        let gain = state.param_values.get("Gain").copied().unwrap_or(1.0);
        let trigger_level = state.param_values.get("Trig").copied().unwrap_or(0.5);

        let samples = if let Some(buffer) = vis_buffer {
            buffer.read_sweep().unwrap_or_default()
        } else {
            (0..256)
                .map(|i| {
                    let t = i as f32 / 256.0;
                    (t * std::f32::consts::TAU * 3.0).sin() * 0.5
                })
                .collect()
        };
        let threshold = trigger_level * 2.0 - 1.0;
        let samples = trim_sweep_to_complete_cycles(&samples, threshold);

        let width = ui.available_width().clamp(120.0, 300.0);
        let height = (width * 0.5).clamp(60.0, 120.0);

        super::widgets::draw_oscilloscope_with_trigger(
            ui,
            samples,
            width,
            height,
            gain,
            theme().colors.accent_cyan,
            Some(trigger_level),
        );

        if vis_buffer.is_none() {
            ui.label(
                egui::RichText::new("No signal")
                    .small()
                    .color(theme().colors.text_dim),
            );
        }
    }

    // Group parameters by widget hint (non-Envelope modules)
    let waveform_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::WaveformSelector))
        .collect();

    let knob_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| {
            matches!(
                p.widget_hint,
                WidgetHint::Knob | WidgetHint::FrequencySlider
            )
        })
        .collect();

    let slider_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Slider | WidgetHint::TimeSlider))
        .collect();

    let dropdown_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Dropdown))
        .collect();

    let toggle_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Toggle))
        .collect();

    // Draw waveform selectors first (most prominent)
    for param in &waveform_params {
        if let Some(ref choices) = param.choices {
            let current = state
                .param_values
                .get(&param.name)
                .copied()
                .unwrap_or(param.range.default);
            let mut selected = current.round() as usize;

            ui.label(
                egui::RichText::new(&param.name)
                    .size(theme().fonts.size_normal)
                    .color(theme().colors.text_secondary),
            );

            if WaveformSelector::new(&mut selected)
                .accent_color(accent_color)
                .show(ui)
            {
                state
                    .param_values
                    .insert(param.name.clone(), selected as f32);
                // Create new param with the selected value
                param_changes.push(param.id.with_f32(selected as f32));
            }

            // Show selected name
            if let Some(choice) = choices.get(selected) {
                ui.label(
                    egui::RichText::new(&choice.name)
                        .size(theme().fonts.size_small)
                        .color(theme().colors.text_dim),
                );
            }
        }
    }

    // Draw sliders (non-Envelope modules only)
    for param in &slider_params {
        let current = state
            .param_values
            .get(&param.name)
            .copied()
            .unwrap_or(param.range.default);
        let mut value = current;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&param.name)
                    .size(theme().fonts.size_normal)
                    .color(theme().colors.text_secondary),
            );
            ui.add_space(4.0);

            // Use logarithmic slider for time parameters
            let is_time = matches!(param.widget_hint, WidgetHint::TimeSlider);

            let slider = if is_time && param.range.min > 0.0 {
                egui::Slider::new(&mut value, param.range.min..=param.range.max)
                    .logarithmic(true)
                    .suffix("s")
                    .min_decimals(3)
                    .max_decimals(3)
            } else {
                egui::Slider::new(&mut value, param.range.min..=param.range.max)
                    .min_decimals(2)
                    .max_decimals(2)
            };

            if ui.add(slider).changed() {
                state.param_values.insert(param.name.clone(), value);
                param_changes.push(param.id.with_f32(value));
            }
        });
    }

    // Draw dropdowns (for non-waveform choices)
    for param in &dropdown_params {
        if let Some(ref choices) = param.choices {
            let current = state
                .param_values
                .get(&param.name)
                .copied()
                .unwrap_or(param.range.default);
            let mut selected = current.round() as usize;

            // Detect mod matrix source/destination dropdowns for filtering
            let is_mm_source = matches!(param.id, Param::ModMatrix(ModMatrixParam::SlotSource(..)));
            let is_mm_dest = matches!(
                param.id,
                Param::ModMatrix(ModMatrixParam::SlotDestination(..))
            );

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(&param.name)
                        .size(theme().fonts.size_normal)
                        .color(theme().colors.text_secondary),
                );
                let text = choices
                    .get(selected)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "?".into());
                egui::ComboBox::from_id_salt(format!("{}-{}", descriptor.type_id.0, param.name))
                    .selected_text(text)
                    .show_ui(ui, |ui| {
                        for (i, choice) in choices.iter().enumerate() {
                            // Filter mod matrix choices based on available modules
                            if (is_mm_source || is_mm_dest)
                                && !is_mod_choice_available(&choice.id, is_mm_source, analysis)
                            {
                                continue;
                            }
                            if ui.selectable_label(selected == i, &choice.name).clicked() {
                                selected = i;
                            }
                        }
                    });
            });

            if selected as f32 != current.round() {
                state
                    .param_values
                    .insert(param.name.clone(), selected as f32);
                // Create new param with the selected value
                param_changes.push(param.id.with_f32(selected as f32));
            }
        }
    }

    // Draw toggles
    if !toggle_params.is_empty() {
        ui.horizontal(|ui| {
            for param in &toggle_params {
                let current = state
                    .param_values
                    .get(&param.name)
                    .copied()
                    .unwrap_or(param.range.default);
                let mut checked = current > 0.5;
                if ui.checkbox(&mut checked, &param.name).changed() {
                    let new_val = if checked { 1.0 } else { 0.0 };
                    state.param_values.insert(param.name.clone(), new_val);
                    param_changes.push(param.id.with_f32(new_val));
                }
            }
        });
    }

    // Draw knobs in rows of max 5
    if !knob_params.is_empty() {
        ui.add_space(4.0);
        const KNOBS_PER_ROW: usize = 5;
        for chunk in knob_params.chunks(KNOBS_PER_ROW) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                for param in chunk {
                    ui.vertical(|ui| {
                        let current = state
                            .param_values
                            .get(&param.name)
                            .copied()
                            .unwrap_or(param.range.default);
                        let mut value = current;

                        Knob::from_descriptor(&mut value, param)
                            .size(theme().sizes.knob_size)
                            .accent_color(accent_color)
                            .show(ui);

                        if (value - current).abs() > f32::EPSILON {
                            state.param_values.insert(param.name.clone(), value);
                            param_changes.push(param.id.with_f32(value));
                        }
                    });
                }
            });
        }
    }

    PanelParamsResult { param_changes }
}

/// Draw mod matrix as a grid with size selector.
#[allow(clippy::too_many_lines)]
fn draw_mod_matrix_grid(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    accent_color: Color32,
    analysis: &PatchAnalysis,
) -> PanelParamsResult {
    use super::widgets::Knob;

    let mut param_changes = Vec::new();

    // --- Grid Size selector ---
    let grid_size_param = descriptor
        .parameters
        .iter()
        .find(|p| matches!(p.id, Param::ModMatrix(ModMatrixParam::GridSize(_))));

    let mut grid_size = ModMatrixGridSize::default();
    if let Some(gs_param) = grid_size_param
        && let Some(ref choices) = gs_param.choices
    {
        let current = state
            .param_values
            .get(&gs_param.name)
            .copied()
            .unwrap_or(gs_param.range.default);
        let mut selected = current.round() as usize;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Grid")
                    .size(theme().fonts.size_normal)
                    .color(theme().colors.text_secondary),
            );
            let text = choices
                .get(selected)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "?".into());
            egui::ComboBox::from_id_salt("mod_matrix_grid_size")
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for (i, choice) in choices.iter().enumerate() {
                        if ui.selectable_label(selected == i, &choice.name).clicked() {
                            selected = i;
                        }
                    }
                });
        });

        if selected as f32 != current.round() {
            state
                .param_values
                .insert(gs_param.name.clone(), selected as f32);
            param_changes.push(gs_param.id.with_f32(selected as f32));
        }

        grid_size = ModMatrixGridSize::from_index(selected);
    }

    let dim = grid_size.grid_dimension();
    let slot_count = grid_size.slot_count();

    ui.add_space(4.0);

    // Fixed slot width — avoid ui.available_width() which is unbounded in auto-sized Areas
    let grid_spacing = 4.0;
    let slot_width: f32 = 140.0;
    let combo_width = (slot_width - 20.0).max(60.0);

    // --- Grid of slots ---
    egui::Grid::new("mod_matrix_grid")
        .num_columns(dim)
        .spacing(egui::vec2(grid_spacing, grid_spacing))
        .show(ui, |ui| {
            for slot_idx in 0..slot_count {
                let slot_num = slot_idx + 1;

                // Find params for this slot
                let src_param = descriptor.parameters.iter().find(|p| {
                    matches!(p.id, Param::ModMatrix(ModMatrixParam::SlotSource(s, _)) if s as usize == slot_idx)
                });
                let dst_param = descriptor.parameters.iter().find(|p| {
                    matches!(p.id, Param::ModMatrix(ModMatrixParam::SlotDestination(s, _)) if s as usize == slot_idx)
                });
                let amt_param = descriptor.parameters.iter().find(|p| {
                    matches!(p.id, Param::ModMatrix(ModMatrixParam::SlotAmount(s, _)) if s as usize == slot_idx)
                });
                let en_param = descriptor.parameters.iter().find(|p| {
                    matches!(p.id, Param::ModMatrix(ModMatrixParam::SlotEnabled(s, _)) if s as usize == slot_idx)
                });

                ui.group(|ui| {
                    ui.set_width(slot_width);
                    ui.vertical(|ui| {
                        // Slot label
                        ui.label(
                            egui::RichText::new(format!("Slot {slot_num}"))
                                .size(theme().fonts.size_small)
                                .color(theme().colors.text_dim),
                        );

                        // Source ComboBox
                        if let Some(sp) = src_param
                            && let Some(ref choices) = sp.choices
                        {
                            let current = state
                                .param_values
                                .get(&sp.name)
                                .copied()
                                .unwrap_or(sp.range.default);
                            let mut selected = current.round() as usize;
                            let text = choices
                                .get(selected)
                                .map(|c| c.name.clone())
                                .unwrap_or_else(|| "?".into());
                            egui::ComboBox::from_id_salt(format!("mm_src_{slot_idx}"))
                                .selected_text(text)
                                .width(combo_width)
                                .show_ui(ui, |ui| {
                                    for (i, choice) in choices.iter().enumerate() {
                                        if !is_mod_choice_available(
                                            &choice.id, true, analysis,
                                        ) {
                                            continue;
                                        }
                                        if ui
                                            .selectable_label(selected == i, &choice.name)
                                            .clicked()
                                        {
                                            selected = i;
                                        }
                                    }
                                });
                            if selected as f32 != current.round() {
                                state
                                    .param_values
                                    .insert(sp.name.clone(), selected as f32);
                                param_changes.push(sp.id.with_f32(selected as f32));
                            }
                        }

                        // Destination ComboBox
                        if let Some(dp) = dst_param
                            && let Some(ref choices) = dp.choices
                        {
                            let current = state
                                .param_values
                                .get(&dp.name)
                                .copied()
                                .unwrap_or(dp.range.default);
                            let mut selected = current.round() as usize;
                            let text = choices
                                .get(selected)
                                .map(|c| c.name.clone())
                                .unwrap_or_else(|| "?".into());
                            egui::ComboBox::from_id_salt(format!("mm_dst_{slot_idx}"))
                                .selected_text(text)
                                .width(combo_width)
                                .show_ui(ui, |ui| {
                                    for (i, choice) in choices.iter().enumerate() {
                                        if !is_mod_choice_available(
                                            &choice.id, false, analysis,
                                        ) {
                                            continue;
                                        }
                                        if ui
                                            .selectable_label(selected == i, &choice.name)
                                            .clicked()
                                        {
                                            selected = i;
                                        }
                                    }
                                });
                            if selected as f32 != current.round() {
                                state
                                    .param_values
                                    .insert(dp.name.clone(), selected as f32);
                                param_changes.push(dp.id.with_f32(selected as f32));
                            }
                        }

                        // Amount knob + Enabled checkbox side by side
                        ui.horizontal(|ui| {
                            if let Some(ap) = amt_param {
                                let current = state
                                    .param_values
                                    .get(&ap.name)
                                    .copied()
                                    .unwrap_or(ap.range.default);
                                let mut value = current;
                                Knob::from_descriptor(&mut value, ap)
                                    .label("Amount")
                                    .size(theme().sizes.knob_size_small)
                                    .accent_color(accent_color)
                                    .show(ui);
                                if (value - current).abs() > f32::EPSILON {
                                    state.param_values.insert(ap.name.clone(), value);
                                    param_changes.push(ap.id.with_f32(value));
                                }
                            }

                            if let Some(ep) = en_param {
                                let current = state
                                    .param_values
                                    .get(&ep.name)
                                    .copied()
                                    .unwrap_or(ep.range.default);
                                let mut enabled = current > 0.5;
                                if ui.checkbox(&mut enabled, "").changed() {
                                    let val = if enabled { 1.0 } else { 0.0 };
                                    state.param_values.insert(ep.name.clone(), val);
                                    param_changes.push(ep.id.with_f32(val));
                                }
                            }
                        });
                        ui.add_space(2.0);
                    });
                });

                // End row after `dim` columns
                if (slot_idx + 1) % dim == 0 {
                    ui.end_row();
                }
            }
        });

    PanelParamsResult { param_changes }
}

/// Check if a mod matrix dropdown choice should be shown, based on available modules.
///
/// Hides choices that reference modules not present in the patch.
/// For example, "LFO 2" is hidden if only one LFO exists.
fn is_mod_choice_available(choice_id: &str, is_source: bool, analysis: &PatchAnalysis) -> bool {
    if is_source {
        match choice_id {
            "lfo1" => analysis.count(ModuleType::Lfo) >= 1,
            "lfo2" => analysis.count(ModuleType::Lfo) >= 2,
            "env1" => analysis.count(ModuleType::Envelope) >= 1,
            "env2" => analysis.count(ModuleType::Envelope) >= 2,
            _ => true, // none, velocity, note, aftertouch, mod_wheel, pitch_bend
        }
    } else {
        match choice_id {
            "osc1_pitch" | "osc1_level" => analysis.count(ModuleType::Oscillator) >= 1,
            "osc2_pitch" => analysis.count(ModuleType::Oscillator) >= 2,
            "flt1_cutoff" | "flt1_reso" => analysis.count(ModuleType::Filter) >= 1,
            "flt2_cutoff" => analysis.count(ModuleType::Filter) >= 2,
            "amp_level" | "amp_pan" => analysis.count(ModuleType::Amplifier) >= 1,
            "lfo1_rate" | "lfo1_depth" => analysis.count(ModuleType::Lfo) >= 1,
            _ => true, // none
        }
    }
}

// Re-export EffectType from commands for GUI use
pub use synth_engine::commands::EffectType;

/// Visualizer type for the add module submenu (palette selection).
///
/// For engine command types, see `engine::commands::VisualizerType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteVisualizerType {
    Oscilloscope,
    LevelMeter,
    SpectrumAnalyzer,
}

/// Result from ModulePalette - either a category or a specific effect type.
#[derive(Debug, Clone, Copy)]
pub enum PaletteSelection {
    Category(ModuleCategory),
    MathOscillator,
    SubOscillator,
    Noise,
    ModMatrix,
    Effect(EffectType),
    Visualizer(PaletteVisualizerType),
    StereoOutput,
    // Modulation / Utility
    RingMod,
    EnvelopeFollower,
    WavetableOsc,
    // Physical modeling
    KeyboardPanner,
    BodyResonance,
    MechanicalNoise,
    // New modules
    Mseg,
    AdditiveOsc,
    Euclidean,
    TuringMachine,
    RandomGates,
    GranularOsc,
    KineticModulator,
    SignalMonitor,
}

/// Convert from core PortType to widget PortType.
fn convert_port_type(port_type: synth_core::PortType) -> WidgetPortType {
    match port_type {
        synth_core::PortType::Audio => WidgetPortType::Audio,
        synth_core::PortType::Control => WidgetPortType::Control,
        synth_core::PortType::Gate => WidgetPortType::Gate,
        synth_core::PortType::Midi => WidgetPortType::Midi,
    }
}
