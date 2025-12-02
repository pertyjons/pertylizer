//! Rack view - the main workspace for arranging modules.
//!
//! This module handles the canvas where modules are placed,
//! connections are drawn, and modules can be added/removed.
//! 
//! Modules are rendered as draggable, resizable windows with z-order support.
//! Cables are rendered in a foreground layer so they appear above modules.

use std::collections::{HashMap, HashSet, VecDeque};
use eframe::egui::{self, Color32, LayerId, Order, Pos2, Rect, Sense, Ui, Vec2};

use crate::engine::{EngineHandle, ModuleId};
use crate::engine::typed_params::Param;
use crate::engine::graph::Connection;
use crate::modules::core::{ModuleCategory, ModuleDescriptor};

use super::module_panel::{category_color, ModulePanelState, PortPosition};
use super::widgets::{colors, draw_cable, theme, PortDirection, PortType};

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
    pub from_port: String,
    pub from_position: Pos2,
    pub from_type: PortType,
    pub from_direction: PortDirection,
    /// Current mouse position.
    pub current_pos: Pos2,
}

/// The main rack view state.
pub struct RackView {
    /// All module panels.
    panels: HashMap<ModuleId, ModulePanelState>,
    /// All connections.
    connections: Vec<Connection>,
    /// Port positions (updated each frame).
    port_positions: HashMap<(ModuleId, String), PortPosition>,
    /// Currently selected module.
    selected_module: Option<ModuleId>,
    /// Connection being drawn.
    pending_connection: Option<PendingConnection>,
    /// Canvas offset for panning.
    canvas_offset: Vec2,
    /// Zoom level.
    zoom: f32,
    /// Module descriptors (cached).
    descriptors: HashMap<ModuleId, ModuleDescriptor>,
    /// Next position for new modules.
    next_module_pos: Pos2,
    /// Module z-order (front to back).
    z_order: Vec<ModuleId>,
    /// Module connectivity status (updated when connections change).
    connectivity: HashMap<ModuleId, ModuleConnectivity>,
}

impl RackView {
    pub fn new() -> Self {
        Self {
            panels: HashMap::new(),
            connections: Vec::new(),
            port_positions: HashMap::new(),
            selected_module: None,
            pending_connection: None,
            canvas_offset: Vec2::ZERO,
            zoom: 1.0,
            descriptors: HashMap::new(),
            next_module_pos: Pos2::new(50.0, 50.0),
            z_order: Vec::new(),
            connectivity: HashMap::new(),
        }
    }

    /// Add a module to the rack.
    pub fn add_module(&mut self, id: ModuleId, descriptor: ModuleDescriptor) {
        let position = self.next_module_pos;
        self.next_module_pos.x += 210.0;
        // Wrap to next row based on typical screen width
        if self.next_module_pos.x > 1200.0 {
            self.next_module_pos.x = 50.0;
            self.next_module_pos.y += 320.0;
        }

        // Initialize parameter values from defaults
        let mut param_values = HashMap::new();
        for param in &descriptor.parameters {
            param_values.insert(param.name.clone(), param.default);
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
            param_values.insert(param.name.clone(), param.default);
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
        self.selected_module = None;
        self.pending_connection = None;
        self.next_module_pos = Pos2::new(50.0, 50.0);
    }

    /// Get module data for saving.
    pub fn get_module_data(&self, id: ModuleId) -> Option<(ModuleDescriptor, Pos2, HashMap<String, f32>)> {
        let descriptor = self.descriptors.get(&id)?;
        let panel = self.panels.get(&id)?;
        Some((descriptor.clone(), panel.position, panel.param_values.clone()))
    }

    /// Remove a module from the rack.
    pub fn remove_module(&mut self, id: ModuleId) {
        self.panels.remove(&id);
        self.descriptors.remove(&id);
        self.z_order.retain(|&mid| mid != id);
        self.connectivity.remove(&id);
        // Remove connections involving this module
        self.connections
            .retain(|c| c.from_module != id && c.to_module != id);
        if self.selected_module == Some(id) {
            self.selected_module = None;
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

    /// Calculate the topological processing order of modules.
    /// Returns a vector of (ModuleId, position) tuples.
    pub fn calculate_processing_order(&self) -> Vec<(ModuleId, usize)> {
        use std::collections::{HashMap, VecDeque};

        let module_ids: Vec<ModuleId> = self.z_order.clone();
        if module_ids.is_empty() {
            return Vec::new();
        }

        // Build adjacency list and in-degree count
        let mut in_degree: HashMap<ModuleId, usize> = HashMap::new();
        let mut adjacency: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();

        for &id in &module_ids {
            in_degree.insert(id, 0);
            adjacency.insert(id, Vec::new());
        }

        for conn in &self.connections {
            if let Some(adj) = adjacency.get_mut(&conn.from_module) {
                adj.push(conn.to_module);
            }
            if let Some(deg) = in_degree.get_mut(&conn.to_module) {
                *deg += 1;
            }
        }

        // Kahn's algorithm for topological sort
        let mut queue: VecDeque<ModuleId> = VecDeque::new();
        for (&id, &deg) in &in_degree {
            if deg == 0 {
                queue.push_back(id);
            }
        }

        let mut order = Vec::new();
        while let Some(id) = queue.pop_front() {
            order.push(id);
            if let Some(neighbors) = adjacency.get(&id) {
                for &neighbor in neighbors {
                    if let Some(deg) = in_degree.get_mut(&neighbor) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(neighbor);
                        }
                    }
                }
            }
        }

        // Add any modules not in the order (disconnected)
        for &id in &module_ids {
            if !order.contains(&id) {
                order.push(id);
            }
        }

        // Return with positions
        order.into_iter().enumerate().map(|(pos, id)| (id, pos)).collect()
    }

    /// Get connected ports for a module.
    fn get_connected_ports(&self, module_id: ModuleId) -> Vec<String> {
        let mut ports = Vec::new();
        for conn in &self.connections {
            if conn.from_module == module_id {
                ports.push(conn.from_port.clone());
            }
            if conn.to_module == module_id {
                ports.push(conn.to_port.clone());
            }
        }
        ports
    }

    /// Draw the rack view.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        handle: &EngineHandle,
    ) -> RackViewResult {
        let mut result = RackViewResult::default();

        // Canvas background
        let canvas_rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(canvas_rect, Sense::click_and_drag());

        // Handle canvas panning with middle mouse
        if response.dragged_by(egui::PointerButton::Middle) {
            self.canvas_offset += response.drag_delta();
        }

        // Handle zoom with scroll (when not over a module)
        if response.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                self.zoom = (self.zoom + scroll * 0.001).clamp(0.5, 2.0);
            }
        }

        // Draw grid
        self.draw_grid(ui, canvas_rect);

        // Clear port positions for this frame
        self.port_positions.clear();

        // Collect data before mutable iteration
        let module_ids: Vec<_> = self.z_order.clone();
        let connected_ports_map: HashMap<ModuleId, Vec<String>> = module_ids
            .iter()
            .map(|&id| (id, self.get_connected_ports(id)))
            .collect();

        // Calculate processing order for display
        let processing_order: HashMap<ModuleId, usize> = self.calculate_processing_order()
            .into_iter()
            .collect();
        let total_modules = module_ids.len();

        // Track which module to bring to front
        let mut bring_to_front: Option<ModuleId> = None;

        // Draw modules as windows (in z-order)
        for module_id in &module_ids {
            let module_id = *module_id;
            let connected_ports = connected_ports_map.get(&module_id).cloned().unwrap_or_default();
            
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

            // Dim modules that aren't connected to output
            let opacity = match connectivity_status {
                ModuleConnectivity::Connected => 1.0,
                ModuleConnectivity::Orphaned => 0.6,
                ModuleConnectivity::Disconnected => 0.4,
            };

            let dimmed_accent = accent_color.gamma_multiply(opacity);

            let mut open = true;
            let window_id = egui::Id::new(("module_window", module_id.to_string()));

            // Create frame with dimming for disconnected modules
            let frame = egui::Frame::window(&ui.ctx().style())
                .stroke(egui::Stroke::new(
                    if is_selected { 2.0 } else { 1.0 },
                    if is_selected { dimmed_accent } else { dimmed_accent.gamma_multiply(0.5) }
                ))
                .fill(ui.ctx().style().visuals.window_fill().gamma_multiply(opacity));

            let window = egui::Window::new(&descriptor.name)
                .id(window_id)
                .open(&mut open)
                .collapsible(true)
                .resizable(true)
                .default_pos(panel_position + self.canvas_offset)
                .min_width(180.0)
                .min_height(100.0)
                .frame(frame);

            // Get processing info for this module
            let proc_position = processing_order.get(&module_id).copied();
            let is_source = self.is_source(module_id);
            let is_sink = self.is_sink(module_id);

            let window_response = window.show(ui.ctx(), |ui| {
                // Processing order and source/sink indicators
                ui.horizontal(|ui| {
                    // Accent color indicator
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 14.0), Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, dimmed_accent);

                    // Processing order number
                    if let Some(pos) = proc_position {
                        let order_text = format!("#{}", pos + 1);
                        ui.label(egui::RichText::new(order_text)
                            .small()
                            .color(colors::TEXT_DIM))
                            .on_hover_text(format!("Processing order: {} of {}", pos + 1, total_modules));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Connectivity status indicator
                        match connectivity_status {
                            ModuleConnectivity::Connected => {
                                ui.label(egui::RichText::new("●")
                                    .small()
                                    .color(Color32::from_rgb(100, 200, 100)))
                                    .on_hover_text("Connected to output");
                            }
                            ModuleConnectivity::Orphaned => {
                                ui.label(egui::RichText::new("○")
                                    .small()
                                    .color(Color32::from_rgb(200, 200, 100)))
                                    .on_hover_text("Has connections but not routed to output");
                            }
                            ModuleConnectivity::Disconnected => {
                                ui.label(egui::RichText::new("○")
                                    .small()
                                    .color(Color32::from_rgb(100, 100, 100)))
                                    .on_hover_text("No connections");
                            }
                        }

                        // Source indicator (no inputs)
                        if is_source {
                            ui.label(egui::RichText::new("▶")
                                .small()
                                .color(Color32::from_rgb(100, 200, 100)))
                                .on_hover_text("Source: No incoming connections");
                        }
                        // Sink indicator (no outputs)
                        if is_sink {
                            ui.label(egui::RichText::new("■")
                                .small()
                                .color(Color32::from_rgb(200, 100, 100)))
                                .on_hover_text("Sink: No outgoing connections");
                        }
                    });
                });

                ui.separator();

                // Draw ports section
                self.draw_ports_section(ui, module_id, &descriptor, &connected_ports);
                
                ui.separator();

                // Draw parameters - need to get panel_state here
                if let Some(panel_state) = self.panels.get_mut(&module_id) {
                    // Get visualization buffer for this module if it exists
                    let vis_buffer = handle.get_visualization_buffer(module_id);
                    let panel_result = draw_module_panel_params(ui, panel_state, &descriptor, accent_color, vis_buffer);

                    // Collect parameter changes
                    for param in panel_result.param_changes {
                        result.param_changes.push((module_id, param));
                    }
                }
            });

            // Handle window interaction
            if let Some(inner_response) = window_response {
                // Update position from window
                if let Some(new_pos) = ui.ctx().memory(|mem| {
                    mem.area_rect(window_id).map(|r| r.min)
                }) {
                    if let Some(panel_state) = self.panels.get_mut(&module_id) {
                        panel_state.position = new_pos - self.canvas_offset;
                    }
                }

                // Bring to front on click
                if inner_response.response.clicked() || inner_response.response.drag_started() {
                    self.selected_module = Some(module_id);
                    bring_to_front = Some(module_id);
                }
            }

            // Handle window close (delete module)
            if !open {
                result.modules_to_remove.push(module_id);
            }
        }

        // Apply z-order change
        if let Some(id) = bring_to_front {
            self.bring_to_front(id);
        }

        // Handle port interactions for connections
        self.handle_port_interactions(ui, &mut result);

        // Draw connections in foreground layer and handle cable removal
        let cables_to_remove = self.draw_connections_foreground(ui);
        if !cables_to_remove.is_empty() {
            for cable in cables_to_remove {
                self.connections.retain(|c| c != &cable);
                result.connections_to_remove.push(cable);
            }
            self.calculate_connectivity();
        }

        // Draw pending connection in foreground
        if let Some(ref pending) = self.pending_connection {
            let color = match pending.from_type {
                PortType::Audio => colors::CABLE_AUDIO,
                PortType::Control => colors::CABLE_CONTROL,
                PortType::Gate => colors::CABLE_GATE,
                PortType::Midi => colors::PORT_MIDI,
            };
            
            let painter = ui.ctx().layer_painter(LayerId::new(Order::Foreground, egui::Id::new("cables")));
            draw_cable(&painter, pending.from_position, pending.current_pos, color);
        }

        // Handle click on empty space to deselect
        if response.clicked() {
            self.selected_module = None;
        }

        // Cancel pending connection with escape (not right click - that's for cable removal now)
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) 
        {
            self.pending_connection = None;
        }

        result
    }

    /// Draw ports section with clickable ports.
    fn draw_ports_section(
        &mut self,
        ui: &mut Ui,
        module_id: ModuleId,
        descriptor: &ModuleDescriptor,
        connected_ports: &[String],
    ) {
        use crate::modules::core::PortDirection as CorePortDirection;

        ui.horizontal(|ui| {
            // Input ports
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("IN").size(theme().fonts.size_small).color(colors::TEXT_DIM));
                for port in descriptor.ports.iter().filter(|p| p.direction == CorePortDirection::Input) {
                    let port_type = convert_port_type(port.port_type);
                    let is_connected = connected_ports.contains(&port.name);
                    
                    ui.horizontal(|ui| {
                        let (response, center) = super::widgets::Port::new(port_type, PortDirection::Input)
                            .connected(is_connected)
                            .show(ui);
                        
                        // Store port position (screen coordinates)
                        self.port_positions.insert(
                            (module_id, port.name.clone()),
                            PortPosition {
                                module_id,
                                port_name: port.name.clone(),
                                position: center,
                                port_type,
                                direction: PortDirection::Input,
                            },
                        );
                        
                        ui.label(egui::RichText::new(&port.label).size(theme().fonts.size_small).color(colors::TEXT_SECONDARY));
                        
                        if response.hovered() && !port.description.is_empty() {
                            response.on_hover_text(&port.description);
                        }
                    });
                }
            });

            ui.add_space(20.0);

            // Output ports
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("OUT").size(theme().fonts.size_small).color(colors::TEXT_DIM));
                    for port in descriptor.ports.iter().filter(|p| p.direction == CorePortDirection::Output) {
                        let port_type = convert_port_type(port.port_type);
                        let is_connected = connected_ports.contains(&port.name);
                        
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&port.label).size(theme().fonts.size_small).color(colors::TEXT_SECONDARY));
                            
                            let (response, center) = super::widgets::Port::new(port_type, PortDirection::Output)
                                .connected(is_connected)
                                .show(ui);
                            
                            self.port_positions.insert(
                                (module_id, port.name.clone()),
                                PortPosition {
                                    module_id,
                                    port_name: port.name.clone(),
                                    position: center,
                                    port_type,
                                    direction: PortDirection::Output,
                                },
                            );
                            
                            if response.hovered() && !port.description.is_empty() {
                                response.on_hover_text(&port.description);
                            }
                        });
                    }
                });
            });
        });
    }

    fn draw_grid(&self, ui: &mut Ui, rect: Rect) {
        let painter = ui.painter();
        
        // Background
        painter.rect_filled(rect, 0.0, colors::BG_DARK);

        // Grid lines
        let grid_size = 50.0 * self.zoom;
        let offset_x = self.canvas_offset.x % grid_size;
        let offset_y = self.canvas_offset.y % grid_size;

        let grid_color = Color32::from_rgba_unmultiplied(60, 65, 75, 50);

        // Vertical lines
        let mut x = rect.left() + offset_x;
        while x < rect.right() {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                egui::Stroke::new(1.0, grid_color),
            );
            x += grid_size;
        }

        // Horizontal lines
        let mut y = rect.top() + offset_y;
        while y < rect.bottom() {
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                egui::Stroke::new(1.0, grid_color),
            );
            y += grid_size;
        }
    }

    /// Draw connections in a foreground layer so they appear above modules.
    /// Returns connections that should be removed (right-clicked).
    fn draw_connections_foreground(&self, ui: &Ui) -> Vec<Connection> {
        let painter = ui.ctx().layer_painter(LayerId::new(Order::Foreground, egui::Id::new("cables")));
        let mut to_remove = Vec::new();
        
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());
        let right_clicked = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));

        for connection in &self.connections {
            let from_key = (connection.from_module, connection.from_port.clone());
            let to_key = (connection.to_module, connection.to_port.clone());

            if let (Some(from_pos), Some(to_pos)) = (
                self.port_positions.get(&from_key),
                self.port_positions.get(&to_key),
            ) {
                let color = match from_pos.port_type {
                    PortType::Audio => colors::CABLE_AUDIO,
                    PortType::Control => colors::CABLE_CONTROL,
                    PortType::Gate => colors::CABLE_GATE,
                    PortType::Midi => colors::PORT_MIDI,
                };

                // Check if mouse is near this cable
                let is_hovered = pointer_pos.map(|p| {
                    point_near_bezier(p, from_pos.position, to_pos.position, 8.0)
                }).unwrap_or(false);

                // Draw cable with highlight if hovered
                let draw_color = if is_hovered {
                    Color32::from_rgb(255, 100, 100) // Red highlight when hovered
                } else {
                    color
                };
                
                draw_cable(&painter, from_pos.position, to_pos.position, draw_color);
                
                // Draw thicker outline when hovered for better visibility
                if is_hovered {
                    // Show tooltip
                    if let Some(pos) = pointer_pos {
                        let tooltip_pos = pos + Vec2::new(10.0, 10.0);
                        painter.text(
                            tooltip_pos,
                            egui::Align2::LEFT_TOP,
                            "Right-click to disconnect",
                            theme().fonts.normal(),
                            Color32::WHITE,
                        );
                    }
                    
                    // Remove on right click
                    if right_clicked {
                        to_remove.push(connection.clone());
                    }
                }
            }
        }
        
        to_remove
    }

    fn handle_port_interactions(&mut self, ui: &mut Ui, result: &mut RackViewResult) {
        let pointer_pos = ui.input(|i| i.pointer.interact_pos());

        // Check for port clicks
        for ((module_id, port_name), port_pos) in &self.port_positions {
            let port_rect = Rect::from_center_size(port_pos.position, Vec2::splat(20.0));

            if let Some(pos) = pointer_pos {
                if port_rect.contains(pos) {
                    // Check for click
                    if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary)) {
                        if let Some(ref pending) = self.pending_connection {
                            // Complete connection
                            if self.can_connect(pending, port_pos) {
                                let connection = if pending.from_direction == PortDirection::Output {
                                    Connection::new(
                                        pending.from_module,
                                        pending.from_port.clone(),
                                        *module_id,
                                        port_name.clone(),
                                    )
                                } else {
                                    Connection::new(
                                        *module_id,
                                        port_name.clone(),
                                        pending.from_module,
                                        pending.from_port.clone(),
                                    )
                                };
                                result.connections_to_add.push(connection);
                            }
                            self.pending_connection = None;
                        } else {
                            // Start new connection
                            self.pending_connection = Some(PendingConnection {
                                from_module: *module_id,
                                from_port: port_name.clone(),
                                from_position: port_pos.position,
                                from_type: port_pos.port_type,
                                from_direction: port_pos.direction,
                                current_pos: pos,
                            });
                        }
                    }
                }
            }
        }

        // Update pending connection position
        if let Some(ref mut pending) = self.pending_connection {
            if let Some(pos) = pointer_pos {
                pending.current_pos = pos;
            }
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
        let output_modules: Vec<ModuleId> = self.panels.keys()
            .filter(|&&id| {
                self.descriptors.get(&id)
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
            let has_any_connection = self.connections.iter()
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
        self.connectivity.get(&id).copied().unwrap_or(ModuleConnectivity::Disconnected)
    }
}

impl Default for RackView {
    fn default() -> Self {
        Self::new()
    }
}

/// Result from drawing the rack view.
#[derive(Default)]
pub struct RackViewResult {
    /// Parameter changes to send to engine.
    /// Each entry is (module_id, Param with value).
    pub param_changes: Vec<(ModuleId, Param)>,
    /// Modules to remove.
    pub modules_to_remove: Vec<ModuleId>,
    /// Connections to add.
    pub connections_to_add: Vec<Connection>,
    /// Connections to remove.
    #[allow(dead_code)]
    pub connections_to_remove: Vec<Connection>,
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
    vis_buffer: Option<&crate::visualizers::VisualizationBuffer>,
    _param_changes: &mut Vec<Param>,
) {
    // Check which type of visualizer based on module type id
    if descriptor.type_id.0 == "oscilloscope" {
        // Get gain from params by name
        let gain = state.param_values.get("Gain")
            .copied().unwrap_or(1.0);

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
            colors::ACCENT_CYAN,
        );

        if vis_buffer.is_none() {
            ui.label(egui::RichText::new("No signal")
                .small().color(colors::TEXT_DIM));
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

        super::widgets::draw_stereo_meter(
            ui,
            peak_l,
            peak_r,
            rms_l,
            rms_r,
            width,
            height,
        );

        if vis_buffer.is_none() {
            ui.label(egui::RichText::new("No signal")
                .small().color(colors::TEXT_DIM));
        }
    }
}

/// Draw only the parameters section of a module panel.
fn draw_module_panel_params(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    accent_color: Color32,
    vis_buffer: Option<&crate::visualizers::VisualizationBuffer>,
) -> PanelParamsResult {
    use crate::modules::core::WidgetHint;
    use super::widgets::{Knob, WaveformSelector};

    let mut param_changes = Vec::new();

    // For Visualizer modules, draw visualization FIRST (before parameters)
    if descriptor.category == ModuleCategory::Visualizer {
        draw_visualizer_display(ui, state, descriptor, vis_buffer, &mut param_changes);
        // Skip regular parameter drawing for visualizers - the display is the main UI
        return PanelParamsResult { param_changes };
    }

    // Group parameters by widget hint
    let waveform_params: Vec<_> = descriptor.parameters.iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::WaveformSelector))
        .collect();

    let knob_params: Vec<_> = descriptor.parameters.iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Knob | WidgetHint::FrequencySlider))
        .collect();

    let slider_params: Vec<_> = descriptor.parameters.iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Slider | WidgetHint::TimeSlider))
        .collect();

    let dropdown_params: Vec<_> = descriptor.parameters.iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Dropdown))
        .collect();

    let toggle_params: Vec<_> = descriptor.parameters.iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Toggle))
        .collect();

    // Draw waveform selectors first (most prominent)
    for param in &waveform_params {
        if let Some(ref choices) = param.choices {
            let current = state.param_values.get(&param.name).copied().unwrap_or(param.default);
            let mut selected = current.round() as usize;

            ui.label(egui::RichText::new(&param.name).size(theme().fonts.size_normal).color(colors::TEXT_SECONDARY));

            if WaveformSelector::new(&mut selected)
                .accent_color(accent_color)
                .show(ui)
            {
                state.param_values.insert(param.name.clone(), selected as f32);
                // Create new param with the selected value
                param_changes.push(param.id.with_f32(selected as f32));
            }

            // Show selected name
            if let Some(choice) = choices.get(selected) {
                ui.label(egui::RichText::new(&choice.name).size(theme().fonts.size_small).color(colors::TEXT_DIM));
            }
        }
    }

    // Draw sliders (for ADSR etc)
    for param in &slider_params {
        let current = state.param_values.get(&param.name).copied().unwrap_or(param.default);
        let mut value = current;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&param.name).size(theme().fonts.size_normal).color(colors::TEXT_SECONDARY));
            ui.add_space(4.0);

            // Use logarithmic slider for time parameters
            let is_time = matches!(param.widget_hint, WidgetHint::TimeSlider);

            let slider = if is_time && param.min > 0.0 {
                egui::Slider::new(&mut value, param.min..=param.max)
                    .logarithmic(true)
                    .suffix("s")
                    .min_decimals(3)
                    .max_decimals(3)
            } else {
                egui::Slider::new(&mut value, param.min..=param.max)
                    .min_decimals(2)
                    .max_decimals(2)
            };

            if ui.add(slider).changed() {
                state.param_values.insert(param.name.clone(), value);
                param_changes.push(param.id.with_f32(value));
            }
        });
    }

    // Draw ADSR visualization if this is an Envelope module
    if descriptor.category == ModuleCategory::Envelope {
        let attack = state.param_values.get("Attack").copied().unwrap_or(0.01);
        let decay = state.param_values.get("Decay").copied().unwrap_or(0.1);
        let sustain = state.param_values.get("Sustain").copied().unwrap_or(0.7);
        let release = state.param_values.get("Release").copied().unwrap_or(0.3);

        ui.add_space(4.0);
        // Use available width for ADSR curve
        let width = ui.available_width().clamp(100.0, 200.0);
        let height = (width * 0.4).clamp(40.0, 80.0);
        super::widgets::draw_adsr_curve(ui, attack, decay, sustain, release, width, height);
    }

    // Draw dropdowns (for non-waveform choices)
    for param in &dropdown_params {
        if let Some(ref choices) = param.choices {
            let current = state.param_values.get(&param.name).copied().unwrap_or(param.default);
            let mut selected = current.round() as usize;

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&param.name).size(theme().fonts.size_normal).color(colors::TEXT_SECONDARY));
                let text = choices.get(selected).map(|c| c.name.clone()).unwrap_or_else(|| "?".into());
                egui::ComboBox::from_id_salt(format!("{}-{}", descriptor.type_id.0, param.name))
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
                state.param_values.insert(param.name.clone(), selected as f32);
                // Create new param with the selected value
                param_changes.push(param.id.with_f32(selected as f32));
            }
        }
    }

    // Draw toggles
    if !toggle_params.is_empty() {
        ui.horizontal(|ui| {
            for param in &toggle_params {
                let current = state.param_values.get(&param.name).copied().unwrap_or(param.default);
                let mut checked = current > 0.5;
                if ui.checkbox(&mut checked, &param.name).changed() {
                    let new_val = if checked { 1.0 } else { 0.0 };
                    state.param_values.insert(param.name.clone(), new_val);
                    param_changes.push(param.id.with_f32(new_val));
                }
            }
        });
    }

    // Draw knobs in a wrapped layout that responds to window width
    if !knob_params.is_empty() {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            for param in &knob_params {
                ui.vertical(|ui| {
                    let current = state.param_values.get(&param.name).copied().unwrap_or(param.default);
                    let mut value = current;

                    Knob::new(&mut value, param.min, param.max)
                        .default(param.default)
                        .response_curve(param.response_curve)
                        .label(&param.name)
                        .size(theme().sizes.knob_size)
                        .accent_color(accent_color)
                        .show(ui);

                    if (value - current).abs() > f32::EPSILON {
                        state.param_values.insert(param.name.clone(), value);
                        param_changes.push(param.id.with_f32(value));
                    }

                    ui.label(egui::RichText::new(param.format(value)).size(theme().fonts.size_small).color(colors::TEXT_DIM));
                });
            }
        });
    }

    PanelParamsResult { param_changes }
}

/// Available modules that can be added.
pub struct ModulePalette;

// Re-export EffectType from commands for GUI use
pub use crate::engine::commands::EffectType;

/// Visualizer type for the add module submenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerType {
    Oscilloscope,
    LevelMeter,
}

/// Result from ModulePalette - either a category or a specific effect type.
#[derive(Debug, Clone, Copy)]
pub enum PaletteSelection {
    Category(ModuleCategory),
    MathOscillator,
    SubOscillator,
    Noise,
    Effect(EffectType),
    Visualizer(VisualizerType),
    StereoOutput,
}

impl ModulePalette {
    /// Show the module palette (for adding new modules).
    pub fn show(ui: &mut Ui) -> Option<PaletteSelection> {
        let mut selected = None;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Add Module:").color(colors::TEXT_SECONDARY));

            // Oscillator submenu (Basic, Math, Sub, Noise)
            let osc_color = category_color(ModuleCategory::Oscillator);
            ui.menu_button(
                egui::RichText::new("🎵 Oscillator").color(osc_color),
                |ui| {
                    if ui.button("🎵 Basic").clicked() {
                        selected = Some(PaletteSelection::Category(ModuleCategory::Oscillator));
                        ui.close();
                    }
                    if ui.button("🔢 Math").clicked() {
                        selected = Some(PaletteSelection::MathOscillator);
                        ui.close();
                    }
                    if ui.button("🔈 Sub").clicked() {
                        selected = Some(PaletteSelection::SubOscillator);
                        ui.close();
                    }
                    if ui.button("🌫 Noise").clicked() {
                        selected = Some(PaletteSelection::Noise);
                        ui.close();
                    }
                },
            );

            let other_categories = [
                (ModuleCategory::Filter, "Filter", "🔊"),
                (ModuleCategory::Envelope, "Envelope", "📈"),
                (ModuleCategory::LFO, "LFO", "〰"),
                (ModuleCategory::Amplifier, "VCA", "🔉"),
                (ModuleCategory::Mixer, "Mixer", "🎚"),
            ];

            for (category, name, icon) in other_categories {
                let color = category_color(category);
                let button = egui::Button::new(
                    egui::RichText::new(format!("{} {}", icon, name)).color(color),
                );

                if ui.add(button).clicked() {
                    selected = Some(PaletteSelection::Category(category));
                }
            }
            
            // Effect submenu
            let effect_color = category_color(ModuleCategory::Effect);
            ui.menu_button(
                egui::RichText::new("✨ Effect").color(effect_color),
                |ui| {
                    if ui.button("🔁 Delay").clicked() {
                        selected = Some(PaletteSelection::Effect(EffectType::Delay));
                        ui.close();
                    }
                    if ui.button("🌊 Reverb").clicked() {
                        selected = Some(PaletteSelection::Effect(EffectType::Reverb));
                        ui.close();
                    }
                    if ui.button("🔥 Distortion").clicked() {
                        selected = Some(PaletteSelection::Effect(EffectType::Distortion));
                        ui.close();
                    }
                    if ui.button("🎭 Chorus").clicked() {
                        selected = Some(PaletteSelection::Effect(EffectType::Chorus));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("🌀 Flanger").clicked() {
                        selected = Some(PaletteSelection::Effect(EffectType::Flanger));
                        ui.close();
                    }
                    if ui.button("🔄 Phaser").clicked() {
                        selected = Some(PaletteSelection::Effect(EffectType::Phaser));
                        ui.close();
                    }
                    if ui.button("📊 Compressor").clicked() {
                        selected = Some(PaletteSelection::Effect(EffectType::Compressor));
                        ui.close();
                    }
                    if ui.button("🎛 EQ").clicked() {
                        selected = Some(PaletteSelection::Effect(EffectType::Eq));
                        ui.close();
                    }
                },
            );
            
            // Visualizer submenu
            let viz_color = category_color(ModuleCategory::Visualizer);
            ui.menu_button(
                egui::RichText::new("📊 Visualizer").color(viz_color),
                |ui| {
                    if ui.button("📈 Oscilloscope").clicked() {
                        selected = Some(PaletteSelection::Visualizer(VisualizerType::Oscilloscope));
                        ui.close();
                    }
                    if ui.button("📊 Level Meter").clicked() {
                        selected = Some(PaletteSelection::Visualizer(VisualizerType::LevelMeter));
                        ui.close();
                    }
                },
            );

            // Stereo Output button
            let output_color = category_color(ModuleCategory::Output);
            let output_button = egui::Button::new(
                egui::RichText::new("🔈 Output").color(output_color),
            );
            if ui.add(output_button).clicked() {
                selected = Some(PaletteSelection::StereoOutput);
            }
        });

        selected
    }
}

/// Convert from core PortType to widget PortType.
fn convert_port_type(port_type: crate::modules::core::PortType) -> PortType {
    match port_type {
        crate::modules::core::PortType::Audio => PortType::Audio,
        crate::modules::core::PortType::Control => PortType::Control,
        crate::modules::core::PortType::Gate => PortType::Gate,
        crate::modules::core::PortType::Midi => PortType::Midi,
    }
}

/// Check if a point is near a bezier cable curve.
fn point_near_bezier(point: Pos2, from: Pos2, to: Pos2, threshold: f32) -> bool {
    // Control points for bezier (same as in draw_cable)
    let control_offset = (to.x - from.x).abs() * 0.5;
    let ctrl1 = Pos2::new(from.x + control_offset, from.y);
    let ctrl2 = Pos2::new(to.x - control_offset, to.y);
    
    // Sample points along the bezier curve and check distance
    let segments = 20;
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;
        
        let x = mt3 * from.x + 3.0 * mt2 * t * ctrl1.x + 3.0 * mt * t2 * ctrl2.x + t3 * to.x;
        let y = mt3 * from.y + 3.0 * mt2 * t * ctrl1.y + 3.0 * mt * t2 * ctrl2.y + t3 * to.y;
        
        let dist = ((point.x - x).powi(2) + (point.y - y).powi(2)).sqrt();
        if dist < threshold {
            return true;
        }
    }
    false
}
