//! Rack view - the main workspace for arranging modules.
//!
//! This module handles the canvas where modules are placed,
//! connections are drawn, and modules can be added/removed.
//!
//! Modules are rendered as draggable, resizable windows with z-order support.
//! Cables are rendered behind modules; hovered cables pop to the foreground.

use eframe::egui::{self, Color32, Id, LayerId, Order, Pos2, Rect, Sense, Ui, Vec2};
use egui_extras as _;
use egui_remixicon::icons as ri;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use synth_core::{ModMatrixGridSize, ModMatrixParam, ModuleType, Param};
use synth_core::{ModuleCategory, ModuleDescriptor, PortName, PortType};
use synth_engine::graph::Connection;
use synth_engine::{EngineHandle, ModuleId};

use crate::patch::{
    ConnectionState, ExposedPortState, GroupCategory, GroupId, GroupTemplate, HexColor,
    ModuleGroupState, ModuleState, ParamValue, Position,
};

use super::module_panel::{ModulePanelState, PortPosition, category_color};
use super::theme::theme;
use super::widgets::{
    CABLE_SPREAD, WidgetPortDirection, WidgetPortType, cable_color, closest_point_on_cable,
    draw_cable, draw_cable_dragging, draw_cable_highlighted, draw_flow_particles, point_near_cable,
};

/// Grid cell size in pixels. Used for grid drawing and snap-to-grid.
pub(crate) const GRID_SIZE: f32 = 32.0;
const GROUP_HEADER_HEIGHT: f32 = 24.0;
const GROUP_PORT_MARGIN: f32 = 12.0;
const GROUP_PADDING: f32 = 16.0;

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

fn collapsed_group_size(group: &ModuleGroup) -> Vec2 {
    let port_rows = group
        .exposed_inputs
        .len()
        .max(group.exposed_outputs.len())
        .max(1);
    let t = theme();
    let height = GROUP_HEADER_HEIGHT
        + GROUP_PORT_MARGIN * 2.0
        + port_rows as f32 * t.sizes.port_vertical_spacing;
    let height = height.max(t.sizes.module_min_height);
    // Collapsed groups only show "N modules" text — use a smaller content width
    let collapsed_content_width = 40.0;
    let ports_width = t.sizes.port_column_width * 2.0 + collapsed_content_width;
    let title_chars = group.name.chars().count() as f32;
    let title_width = 14.0 + title_chars * 7.0; // accent + approximate glyph width
    let header_actions = 2.0 * 20.0 + 8.0; // expand + delete buttons + spacing
    let header_width = title_width + header_actions;
    let width = ports_width.max(header_width);
    Vec2::new(width, height)
}

fn parse_hex_color(hex: &str) -> Option<Color32> {
    let s = hex.trim_start_matches('#');
    let bytes = match s.len() {
        6 => u32::from_str_radix(s, 16).ok().map(|v| (v << 8) | 0xFF),
        8 => u32::from_str_radix(s, 16).ok(),
        _ => None,
    }?;
    let r = ((bytes >> 24) & 0xFF) as u8;
    let g = ((bytes >> 16) & 0xFF) as u8;
    let b = ((bytes >> 8) & 0xFF) as u8;
    let a = (bytes & 0xFF) as u8;
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

fn color32_to_hex(color: Color32) -> HexColor {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.r(),
        color.g(),
        color.b(),
        color.a()
    )
}

fn group_toggle_icon_rect(rect: Rect) -> Rect {
    let size = Vec2::splat(16.0);
    Rect::from_min_size(Pos2::new(rect.max.x - size.x - 6.0, rect.min.y + 4.0), size)
}

fn group_menu_icon_rect(rect: Rect) -> Rect {
    let size = Vec2::splat(16.0);
    let toggle = group_toggle_icon_rect(rect);
    Rect::from_min_size(
        Pos2::new(toggle.min.x - size.x - 4.0, rect.min.y + 4.0),
        size,
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

/// A port exposed on a group boundary (UI-level).
#[derive(Debug, Clone)]
pub(crate) struct ExposedPort {
    pub label: String,
    pub module_id: ModuleId,
    pub port_name: PortName,
}

/// A group of modules in the patch editor (UI-level).
#[derive(Debug, Clone)]
pub(crate) struct ModuleGroup {
    pub id: GroupId,
    pub name: String,
    pub color: Option<HexColor>,
    pub members: Vec<ModuleId>,
    pub collapsed: bool,
    pub position: Pos2,
    pub exposed_inputs: Vec<ExposedPort>,
    pub exposed_outputs: Vec<ExposedPort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GroupPortKey {
    group_id: GroupId,
    module_id: ModuleId,
    port_name: PortName,
    direction: WidgetPortDirection,
}

#[derive(Debug, Clone)]
struct PortRenderInfo {
    module_id: ModuleId,
    port_name: PortName,
    label: String,
    description: String,
    port_type: WidgetPortType,
    is_connected: bool,
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

#[derive(Clone)]
struct GroupContextMenuState {
    group_id: GroupId,
    menu_pos: Pos2,
}

/// Actions requested by the group template UI.
#[derive(Debug, Clone, Copy)]
pub enum GroupTemplateAction {
    /// Open the template browser and insert at drop position (world coords).
    OpenBrowser { drop_pos: Pos2 },
    /// Save a specific group as a template.
    SaveGroup { group_id: GroupId },
}

struct GroupLayout {
    rects_world: HashMap<GroupId, Rect>,
    rects_screen: HashMap<GroupId, Rect>,
    hidden_modules: HashSet<ModuleId>,
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
#[derive(Clone, Copy)]
struct BgContextMenuState {
    /// Where to place the new module (world/logical coordinates).
    world_pos: Pos2,
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
    /// Exposed group port positions for collapsed groups (updated each frame).
    group_port_positions: HashMap<GroupPortKey, PortPosition>,
    /// Currently selected module.
    selected_module: Option<ModuleId>,
    /// Multi-selection for grouping.
    selected_modules: HashSet<ModuleId>,
    /// Currently selected group.
    selected_group: Option<GroupId>,
    /// In-progress rename buffer for group context menu.
    group_name_edit: Option<(GroupId, String)>,
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
    /// Module groups (UI-level metadata).
    groups: HashMap<GroupId, ModuleGroup>,
    /// Lookup: module -> group.
    module_to_group: HashMap<ModuleId, GroupId>,
    /// Next group id to allocate.
    next_group_id: u32,
    /// Modules that need to be repositioned (after auto-layout).
    /// When a module is in this set, we use current_pos() instead of default_pos().
    needs_reposition: HashSet<ModuleId>,
    /// The cable closest to the pointer (if any), updated each frame.
    hovered_cable: Option<Connection>,
    /// Right-click context menu on a port.
    port_context_menu: Option<PortContextMenuState>,
    /// Right-click context menu on background (or cable). Contains cable if hovered.
    bg_context_menu: Option<BgContextMenuState>,
    /// Right-click context menu on a group.
    group_context_menu: Option<GroupContextMenuState>,
    /// Minimum canvas size hint (restored from saved patch).
    min_canvas_size: Option<Vec2>,
    /// Cached effect chain order from previous frame (for change detection).
    prev_effect_chain_order: Vec<ModuleId>,
    /// When true, skip reading positions back from egui Area state.
    /// Set after loading a patch/project so saved positions aren't overwritten
    /// by stale egui Area rects during the same frame.
    suppress_position_readback: bool,
}

impl PatchEditor {
    pub fn new() -> Self {
        Self {
            panels: HashMap::new(),
            connections: Vec::new(),
            port_positions: HashMap::new(),
            group_port_positions: HashMap::new(),
            selected_module: None,
            selected_modules: HashSet::new(),
            selected_group: None,
            group_name_edit: None,
            pending_connection: None,
            descriptors: HashMap::new(),
            next_module_pos: Pos2::new(50.0, 50.0),
            z_order: Vec::new(),
            connectivity: HashMap::new(),
            bypassed: HashMap::new(),
            groups: HashMap::new(),
            module_to_group: HashMap::new(),
            next_group_id: 1,
            needs_reposition: HashSet::new(),
            hovered_cable: None,
            port_context_menu: None,
            bg_context_menu: None,
            group_context_menu: None,
            min_canvas_size: None,
            suppress_position_readback: false,
            prev_effect_chain_order: Vec::new(),
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
        self.group_port_positions.clear();
        self.z_order.clear();
        self.connectivity.clear();
        self.bypassed.clear();
        self.selected_module = None;
        self.selected_modules.clear();
        self.selected_group = None;
        self.group_name_edit = None;
        self.pending_connection = None;
        self.next_module_pos = Pos2::new(50.0, 50.0);
        self.groups.clear();
        self.module_to_group.clear();
        self.next_group_id = 1;
        self.group_context_menu = None;
        self.needs_reposition.clear();
        self.suppress_position_readback = true;
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
        // Clear saved canvas size hint so the canvas can shrink
        self.min_canvas_size = None;
        self.z_order.retain(|&mid| mid != id);
        self.connectivity.remove(&id);
        self.bypassed.remove(&id);
        self.selected_modules.remove(&id);
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
        // Remove from any group
        if let Some(group_id) = self.module_to_group.remove(&id)
            && let Some(group) = self.groups.get_mut(&group_id)
        {
            group.members.retain(|mid| *mid != id);
            group.exposed_inputs.retain(|p| p.module_id != id);
            group.exposed_outputs.retain(|p| p.module_id != id);
            if group.members.is_empty() {
                self.groups.remove(&group_id);
                if self.selected_group == Some(group_id) {
                    self.selected_group = None;
                }
            }
        }
        self.calculate_connectivity();
    }

    /// Load groups from patch data (clears existing groups).
    pub fn load_groups_from_patch(&mut self, groups: &[ModuleGroupState]) {
        self.groups.clear();
        self.module_to_group.clear();

        let mut max_id = 0u32;
        for g in groups {
            max_id = max_id.max(g.id.0);
            let mut members: Vec<ModuleId> = Vec::new();
            for member in &g.members {
                if let Ok(mid) = member.parse::<ModuleId>() {
                    if !self.panels.contains_key(&mid) {
                        continue;
                    }
                    if self.module_to_group.contains_key(&mid) {
                        continue; // Enforce exclusivity
                    }
                    members.push(mid);
                    self.module_to_group.insert(mid, g.id);
                }
            }
            if members.is_empty() {
                continue;
            }

            let member_set: HashSet<ModuleId> = members.iter().copied().collect();
            let exposed_inputs = Self::parse_exposed_ports(&g.exposed_inputs, &member_set);
            let exposed_outputs = Self::parse_exposed_ports(&g.exposed_outputs, &member_set);

            let group = ModuleGroup {
                id: g.id,
                name: g.name.clone(),
                color: g.color.clone(),
                members,
                collapsed: g.collapsed,
                position: Pos2::new(g.position.x, g.position.y),
                exposed_inputs,
                exposed_outputs,
            };
            self.groups.insert(g.id, group);
        }

        self.next_group_id = max_id.saturating_add(1).max(1);
    }

    /// Export groups for patch serialization.
    pub fn group_states(&self) -> Vec<ModuleGroupState> {
        let mut groups: Vec<ModuleGroupState> = self
            .groups
            .values()
            .map(|g| ModuleGroupState {
                id: g.id,
                name: g.name.clone(),
                color: g.color.clone(),
                members: g.members.iter().map(|m| m.to_string()).collect(),
                collapsed: g.collapsed,
                position: Position::new(g.position.x, g.position.y),
                exposed_inputs: g
                    .exposed_inputs
                    .iter()
                    .map(|p| ExposedPortState {
                        label: p.label.clone(),
                        module_id: p.module_id.to_string(),
                        port: String::from(p.port_name),
                    })
                    .collect(),
                exposed_outputs: g
                    .exposed_outputs
                    .iter()
                    .map(|p| ExposedPortState {
                        label: p.label.clone(),
                        module_id: p.module_id.to_string(),
                        port: String::from(p.port_name),
                    })
                    .collect(),
            })
            .collect();
        groups.sort_by_key(|g| g.id.0);
        groups
    }

    /// Get the display name for a group.
    pub fn group_name(&self, group_id: GroupId) -> Option<String> {
        self.groups.get(&group_id).map(|g| g.name.clone())
    }

    /// Insert a group with pre-defined members and exposed ports.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_group(
        &mut self,
        name: String,
        color: Option<HexColor>,
        mut members: Vec<ModuleId>,
        exposed_inputs: Vec<ExposedPort>,
        exposed_outputs: Vec<ExposedPort>,
        collapsed: bool,
        position: Pos2,
    ) -> GroupId {
        let id = self.allocate_group_id();
        members.sort_by_key(|m| m.to_string());

        for mid in &members {
            self.remove_from_group(*mid);
        }
        for mid in &members {
            self.module_to_group.insert(*mid, id);
        }

        let group = ModuleGroup {
            id,
            name,
            color,
            members,
            collapsed,
            position,
            exposed_inputs,
            exposed_outputs,
        };
        self.groups.insert(id, group);
        self.selected_group = Some(id);
        self.selected_modules.clear();
        self.selected_module = None;
        id
    }

    /// Build a reusable group template from an existing group.
    pub fn build_group_template(
        &self,
        group_id: GroupId,
        category: Option<GroupCategory>,
        description: Option<String>,
        author: Option<String>,
    ) -> Option<GroupTemplate> {
        let group = self.groups.get(&group_id)?;
        if group.members.is_empty() {
            return None;
        }

        let mut min = Pos2::new(f32::MAX, f32::MAX);
        for mid in &group.members {
            if let Some((_desc, pos, _params)) = self.get_module_data(*mid) {
                min.x = min.x.min(pos.x);
                min.y = min.y.min(pos.y);
            }
        }
        if !min.x.is_finite() || !min.y.is_finite() {
            return None;
        }

        let mut modules: Vec<ModuleState> = Vec::new();
        for mid in &group.members {
            if let Some((_desc, pos, params)) = self.get_module_data(*mid) {
                let mut param_map = BTreeMap::new();
                for (name, value) in params {
                    param_map.insert(name, ParamValue::Float(value));
                }
                modules.push(ModuleState {
                    id: mid.to_string(),
                    module_type: mid.module_type,
                    position: Position::new(pos.x - min.x, pos.y - min.y),
                    parameters: param_map,
                });
            }
        }

        let member_set: HashSet<ModuleId> = group.members.iter().copied().collect();
        let connections: Vec<ConnectionState> = self
            .connections
            .iter()
            .filter(|c| member_set.contains(&c.from_module) && member_set.contains(&c.to_module))
            .map(ConnectionState::from)
            .collect();

        let exposed_inputs = group
            .exposed_inputs
            .iter()
            .map(|p| ExposedPortState {
                label: p.label.clone(),
                module_id: p.module_id.to_string(),
                port: String::from(p.port_name),
            })
            .collect();
        let exposed_outputs = group
            .exposed_outputs
            .iter()
            .map(|p| ExposedPortState {
                label: p.label.clone(),
                module_id: p.module_id.to_string(),
                port: String::from(p.port_name),
            })
            .collect();

        Some(GroupTemplate {
            name: group.name.clone(),
            author,
            description,
            category,
            tags: Vec::new(),
            color: group.color.clone(),
            modules,
            connections,
            exposed_inputs,
            exposed_outputs,
        })
    }

    fn parse_exposed_ports(
        ports: &[ExposedPortState],
        members: &HashSet<ModuleId>,
    ) -> Vec<ExposedPort> {
        let mut result = Vec::new();
        let mut seen: HashSet<(ModuleId, PortName)> = HashSet::new();
        for p in ports {
            let Ok(mid) = p.module_id.parse::<ModuleId>() else {
                continue;
            };
            if !members.contains(&mid) {
                continue;
            }
            let port_name: PortName = p.port.clone().into();
            if !seen.insert((mid, port_name)) {
                continue;
            }
            result.push(ExposedPort {
                label: p.label.clone(),
                module_id: mid,
                port_name,
            });
        }
        result
    }

    fn group_of(&self, module_id: ModuleId) -> Option<GroupId> {
        self.module_to_group.get(&module_id).copied()
    }

    fn allocate_group_id(&mut self) -> GroupId {
        let id = GroupId(self.next_group_id);
        self.next_group_id = self.next_group_id.saturating_add(1);
        id
    }

    fn remove_from_group(&mut self, module_id: ModuleId) {
        if let Some(group_id) = self.module_to_group.remove(&module_id)
            && let Some(group) = self.groups.get_mut(&group_id)
        {
            group.members.retain(|mid| *mid != module_id);
            group.exposed_inputs.retain(|p| p.module_id != module_id);
            group.exposed_outputs.retain(|p| p.module_id != module_id);
            if group.members.is_empty() {
                self.groups.remove(&group_id);
                if self.selected_group == Some(group_id) {
                    self.selected_group = None;
                }
                if self
                    .group_context_menu
                    .as_ref()
                    .is_some_and(|m| m.group_id == group_id)
                {
                    self.group_context_menu = None;
                }
            }
        }
        self.refresh_exposed_for_module(module_id);
    }

    fn add_module_to_group(&mut self, group_id: GroupId, module_id: ModuleId) {
        if let Some(group) = self.groups.get_mut(&group_id)
            && !group.members.contains(&module_id)
        {
            group.members.push(module_id);
        }
        self.module_to_group.insert(module_id, group_id);
        self.refresh_exposed_for_module(module_id);
    }

    fn refresh_exposed_for_module(&mut self, module_id: ModuleId) {
        let connections: Vec<Connection> = self.connections.clone();
        for conn in &connections {
            if conn.from_module == module_id || conn.to_module == module_id {
                self.ensure_exposed_for_connection(conn);
            }
        }
    }

    fn create_group_from_selection(&mut self) -> Option<GroupId> {
        if self.selected_modules.is_empty() {
            return None;
        }
        let id = self.allocate_group_id();
        let mut members: Vec<ModuleId> = self.selected_modules.iter().copied().collect();
        members.sort_by_key(|m| m.to_string());

        for mid in &members {
            self.remove_from_group(*mid);
        }
        for mid in &members {
            self.module_to_group.insert(*mid, id);
        }

        let position = self.compute_group_default_position(&members);
        let members_for_refresh = members.clone();

        let group = ModuleGroup {
            id,
            name: format!("Group {}", id.0),
            color: None,
            members,
            collapsed: false,
            position,
            exposed_inputs: Vec::new(),
            exposed_outputs: Vec::new(),
        };
        self.groups.insert(id, group);
        for mid in &members_for_refresh {
            self.refresh_exposed_for_module(*mid);
        }
        self.selected_group = Some(id);
        Some(id)
    }

    fn compute_group_default_position(&self, members: &[ModuleId]) -> Pos2 {
        let mut min = Pos2::new(f32::MAX, f32::MAX);
        let mut any = false;
        for mid in members {
            if let Some(panel) = self.panels.get(mid) {
                let rect = Rect::from_min_size(panel.position, panel.size);
                min.x = min.x.min(rect.min.x);
                min.y = min.y.min(rect.min.y);
                any = true;
            }
        }
        if any { min } else { self.next_module_pos }
    }

    fn group_bounds_world(&self, group: &ModuleGroup) -> Option<Rect> {
        let mut rect: Option<Rect> = None;
        for mid in &group.members {
            if let Some(panel) = self.panels.get(mid) {
                let r = Rect::from_min_size(panel.position, panel.size);
                rect = Some(rect.map_or(r, |acc| acc.union(r)));
            }
        }
        let mut rect = rect?;
        rect.min.x -= GROUP_PADDING;
        rect.max.x += GROUP_PADDING;
        rect.min.y -= GROUP_PADDING + GROUP_HEADER_HEIGHT;
        rect.max.y += GROUP_PADDING;
        Some(rect)
    }

    fn port_widget_type(&self, module_id: ModuleId, port_name: PortName) -> WidgetPortType {
        if let Some(descriptor) = self.descriptors.get(&module_id)
            && let Some(port) = descriptor.ports.iter().find(|p| p.name == port_name)
        {
            return match port.port_type {
                PortType::Audio => WidgetPortType::Audio,
                PortType::Control => WidgetPortType::Control,
                PortType::Gate => WidgetPortType::Gate,
                PortType::Midi => WidgetPortType::Midi,
            };
        }
        WidgetPortType::Audio
    }

    fn compute_group_layout(&self, area_origin: Vec2, scroll_offset: Vec2) -> GroupLayout {
        let mut rects_world = HashMap::new();
        let mut rects_screen = HashMap::new();
        let mut hidden_modules: HashSet<ModuleId> = HashSet::new();

        let screen_offset = area_origin - scroll_offset;

        for group in self.groups.values() {
            let world_rect = if group.collapsed {
                Rect::from_min_size(group.position, collapsed_group_size(group))
            } else {
                match self.group_bounds_world(group) {
                    Some(r) => r,
                    None => continue,
                }
            };
            rects_world.insert(group.id, world_rect);
            let rect_screen = world_rect.translate(screen_offset);
            rects_screen.insert(group.id, rect_screen);

            if group.collapsed {
                hidden_modules.extend(group.members.iter().copied());
            }
        }

        GroupLayout {
            rects_world,
            rects_screen,
            hidden_modules,
        }
    }

    fn delete_group(&mut self, group_id: GroupId, result: &mut PatchEditorResult) {
        let Some(group) = self.groups.get(&group_id).cloned() else {
            return;
        };
        for mid in group.members {
            result.modules_to_remove.push(mid);
            self.remove_module(mid);
        }
        self.groups.remove(&group_id);
        if self.selected_group == Some(group_id) {
            self.selected_group = None;
        }
        if self
            .group_context_menu
            .as_ref()
            .is_some_and(|m| m.group_id == group_id)
        {
            self.group_context_menu = None;
        }
    }

    fn ungroup(&mut self, group_id: GroupId) {
        let Some(group) = self.groups.remove(&group_id) else {
            return;
        };
        for mid in group.members {
            self.module_to_group.remove(&mid);
        }
        if self.selected_group == Some(group_id) {
            self.selected_group = None;
        }
        if self
            .group_context_menu
            .as_ref()
            .is_some_and(|m| m.group_id == group_id)
        {
            self.group_context_menu = None;
        }
    }

    fn is_port_exposed(
        &self,
        group_id: GroupId,
        module_id: ModuleId,
        port_name: PortName,
        direction: WidgetPortDirection,
    ) -> bool {
        let Some(group) = self.groups.get(&group_id) else {
            return false;
        };
        let list = match direction {
            WidgetPortDirection::Input => &group.exposed_inputs,
            WidgetPortDirection::Output => &group.exposed_outputs,
        };
        list.iter()
            .any(|p| p.module_id == module_id && p.port_name == port_name)
    }

    fn expose_port(
        &mut self,
        group_id: GroupId,
        module_id: ModuleId,
        port_name: PortName,
        direction: WidgetPortDirection,
    ) -> bool {
        let label = self
            .port_label(module_id, port_name)
            .unwrap_or_else(|| port_name.to_string());
        let Some(group) = self.groups.get_mut(&group_id) else {
            return false;
        };
        let list = match direction {
            WidgetPortDirection::Input => &mut group.exposed_inputs,
            WidgetPortDirection::Output => &mut group.exposed_outputs,
        };
        if list
            .iter()
            .any(|p| p.module_id == module_id && p.port_name == port_name)
        {
            return false;
        }
        list.push(ExposedPort {
            label,
            module_id,
            port_name,
        });
        true
    }

    fn hide_port(
        &mut self,
        group_id: GroupId,
        module_id: ModuleId,
        port_name: PortName,
        direction: WidgetPortDirection,
    ) -> bool {
        let Some(group) = self.groups.get_mut(&group_id) else {
            return false;
        };
        let list = match direction {
            WidgetPortDirection::Input => &mut group.exposed_inputs,
            WidgetPortDirection::Output => &mut group.exposed_outputs,
        };
        let before = list.len();
        list.retain(|p| !(p.module_id == module_id && p.port_name == port_name));
        before != list.len()
    }

    fn has_external_connection_for_port(
        &self,
        group_id: GroupId,
        module_id: ModuleId,
        port_name: PortName,
        direction: WidgetPortDirection,
    ) -> bool {
        self.connections.iter().any(|c| {
            let (mid, p, other_mid) = match direction {
                WidgetPortDirection::Input => (c.to_module, c.to_port, c.from_module),
                WidgetPortDirection::Output => (c.from_module, c.from_port, c.to_module),
            };
            if mid != module_id || p != port_name {
                return false;
            }
            let other_group = self.group_of(other_mid);
            other_group != Some(group_id)
        })
    }

    fn ensure_exposed_for_connection(&mut self, connection: &Connection) {
        let from_group = self.group_of(connection.from_module);
        let to_group = self.group_of(connection.to_module);
        if from_group == to_group {
            return;
        }
        if let Some(gid) = from_group {
            let _ = self.expose_port(
                gid,
                connection.from_module,
                connection.from_port,
                WidgetPortDirection::Output,
            );
        }
        if let Some(gid) = to_group {
            let _ = self.expose_port(
                gid,
                connection.to_module,
                connection.to_port,
                WidgetPortDirection::Input,
            );
        }
    }

    fn port_label(&self, module_id: ModuleId, port_name: PortName) -> Option<String> {
        let descriptor = self.descriptors.get(&module_id)?;
        descriptor
            .ports
            .iter()
            .find(|p| p.name == port_name)
            .map(|p| p.label.clone())
    }

    fn group_color(&self, group: &ModuleGroup) -> Color32 {
        group
            .color
            .as_ref()
            .and_then(|c| parse_hex_color(c))
            .unwrap_or_else(|| theme().colors.accent_cyan)
    }

    fn is_hidden_internal_connection(&self, connection: &Connection) -> bool {
        let from_group = self.group_of(connection.from_module);
        let to_group = self.group_of(connection.to_module);
        if let Some(gid) = from_group
            && from_group == to_group
            && let Some(group) = self.groups.get(&gid)
        {
            return group.collapsed;
        }
        false
    }

    fn resolve_connection_endpoint(
        &self,
        module_id: ModuleId,
        port_name: PortName,
        other_module: ModuleId,
        direction: WidgetPortDirection,
    ) -> Option<PortPosition> {
        if let Some(group_id) = self.group_of(module_id)
            && self.group_of(other_module) != Some(group_id)
            && let Some(group) = self.groups.get(&group_id)
            && group.collapsed
        {
            let key = GroupPortKey {
                group_id,
                module_id,
                port_name,
                direction,
            };
            if let Some(pos) = self.group_port_positions.get(&key) {
                return Some(pos.clone());
            }
        }
        self.port_positions.get(&(module_id, port_name)).cloned()
    }

    /// Delete a module and bypass its signal chain connections.
    ///
    /// If the module has exactly one audio input and one audio output port,
    /// incoming connections are reconnected directly to outgoing connections
    /// so the signal chain isn't broken (A→B→C becomes A→C when B is deleted).
    fn bypass_and_remove(&mut self, module_id: ModuleId, result: &mut PatchEditorResult) {
        // Check if this module can be bypassed (one audio in, one audio out)
        if let Some(descriptor) = self.descriptors.get(&module_id).cloned() {
            let audio_inputs: Vec<_> = descriptor
                .ports
                .iter()
                .filter(|p| {
                    p.direction == synth_core::PortDirection::Input
                        && p.port_type == synth_core::PortType::Audio
                })
                .collect();
            let audio_outputs: Vec<_> = descriptor
                .ports
                .iter()
                .filter(|p| {
                    p.direction == synth_core::PortDirection::Output
                        && p.port_type == synth_core::PortType::Audio
                })
                .collect();

            if audio_inputs.len() == 1 && audio_outputs.len() == 1 {
                let in_port = &audio_inputs[0].name;
                let out_port = &audio_outputs[0].name;

                let incoming: Vec<_> = self
                    .connections
                    .iter()
                    .filter(|c| c.to_module == module_id && c.to_port == *in_port)
                    .cloned()
                    .collect();
                let outgoing: Vec<_> = self
                    .connections
                    .iter()
                    .filter(|c| c.from_module == module_id && c.from_port == *out_port)
                    .cloned()
                    .collect();

                // Create bypass connections (source → destination, skipping deleted module)
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
            }
        }

        // Remove all connections to/from this module
        let module_conns: Vec<_> = self
            .connections
            .iter()
            .filter(|c| c.from_module == module_id || c.to_module == module_id)
            .cloned()
            .collect();
        for c in &module_conns {
            result.connections_to_remove.push(*c);
        }
        self.connections
            .retain(|c| c.from_module != module_id && c.to_module != module_id);

        result.modules_to_remove.push(module_id);
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
            self.ensure_exposed_for_connection(&connection);
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

    /// Set the position of a module in the rack view.
    pub fn set_module_position(&mut self, module_id: ModuleId, position: Pos2) {
        if let Some(panel) = self.panels.get_mut(&module_id) {
            panel.position = position;
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
    /// Used to tell `ScrollArea` how large the content is and for persisting
    /// canvas size in patch files.
    pub fn content_size(&self) -> Vec2 {
        let mut max_x: f32 = 0.0;
        let mut max_y: f32 = 0.0;
        for panel in self.panels.values() {
            max_x = max_x.max(panel.position.x + 350.0);
            max_y = max_y.max(panel.position.y + 400.0);
        }
        for group in self.groups.values() {
            if group.collapsed {
                let size = collapsed_group_size(group);
                max_x = max_x.max(group.position.x + size.x + 100.0);
                max_y = max_y.max(group.position.y + size.y + 100.0);
            }
        }
        if self.panels.is_empty() && self.groups.is_empty() {
            return self.min_canvas_size.unwrap_or(Vec2::new(800.0, 600.0));
        }
        let mut size = Vec2::new(max_x + 100.0, max_y + 100.0);
        if let Some(min) = self.min_canvas_size {
            size = size.max(min);
        }
        size
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
        effect_chain_order: &[ModuleId],
    ) -> PatchEditorResult {
        let mut result = PatchEditorResult::default();

        // Re-align effect modules when the chain order changes
        // (effect added, removed, or reordered)
        if effect_chain_order != self.prev_effect_chain_order {
            self.align_effect_chain(effect_chain_order);
            self.prev_effect_chain_order = effect_chain_order.to_vec();
        }

        let content_size = self.content_size();

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

                // Draw tinted background zone behind effect modules
                self.draw_effect_zone(ui, scroll_rect);

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

        // Compute group layout (bounds + hidden modules) before drawing cables
        let group_layout = self.compute_group_layout(area_origin, scroll_offset);
        let mut new_group_port_positions: HashMap<GroupPortKey, PortPosition> = HashMap::new();

        // Collect module screen rects from egui memory (persisted from previous frame)
        let module_rects: Vec<Rect> = self
            .panels
            .keys()
            .filter_map(|mid| {
                if group_layout.hidden_modules.contains(mid) {
                    return None;
                }
                let wid = Id::new((instrument_id, "module_window", mid.to_string()));
                ui.ctx().memory(|mem| mem.area_rect(wid))
            })
            .collect();

        // Draw group frames on the scroll area's layer BEFORE cables.
        if let Some(layer_id) = scroll_layer_id {
            let clip = scroll_clip_rect.unwrap_or(visible_rect);
            self.draw_group_frames(ui, &group_layout, layer_id, clip);
        }

        // Handle interactions for expanded group frames.
        self.handle_group_interactions(ui, &group_layout, &module_rects);

        // Draw cables on the scroll area's layer BEFORE module Areas are created.
        // This uses the previous frame's port_positions — one frame delay is
        // imperceptible in an immediate-mode GUI.
        let time = ui.input(|i| i.time);
        if let Some(layer_id) = scroll_layer_id {
            let clip = scroll_clip_rect.unwrap_or(visible_rect);
            self.draw_connections(ui, time, layer_id, clip, &module_rects);
            // Draw effect chain cables (signal flow between effects)
            self.draw_effect_chain_cables(
                ui,
                layer_id,
                clip,
                effect_chain_order,
                area_origin,
                scroll_offset,
            );
        }

        // Draw collapsed group boxes (movable) after cables so they sit above.
        self.draw_collapsed_groups(
            ui,
            &group_layout,
            instrument_id,
            visible_rect,
            area_origin,
            scroll_offset,
            &mut result,
            &mut new_group_port_positions,
        );
        self.group_port_positions = new_group_port_positions;

        // Draw context menus (Foreground) — must happen after hover detection above
        self.draw_port_context_menu(ui, &mut result);
        self.draw_group_context_menu(ui, &mut result);
        if let Some(ref response) = canvas_response {
            self.draw_bg_context_menu(response, &mut result);
        }

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
            if group_layout.hidden_modules.contains(&module_id) {
                continue;
            }
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
            let is_selected = self.selected_modules.contains(&module_id);
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
            // (same layer as panels). The keyboard panel renders at Order::Middle
            // to ensure it always has input priority over module Areas.
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
                            ri::CLOSE_LINE,
                            egui::FontId::proportional(10.0),
                            close_color,
                        );

                        if close_resp.clicked() {
                            self.bypass_and_remove(module_id, &mut result);
                        }
                    });

                    // Request continuous repaint so the waveform updates
                    ui.ctx().request_repaint();
                    return;
                }

                frame.show(ui, |ui| {
                    // Title bar: name + status icons + close button (single row)
                    Self::draw_panel_header_row(
                        ui,
                        dimmed_accent,
                        &title,
                        Some(format!("ID: {module_id}")),
                        |ui| {
                            let t = theme();
                            let button_min_size = Vec2::new(20.0, 20.0);

                            // Source indicator (no inputs)
                            if is_source {
                                ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(ri::UPLOAD_2_FILL)
                                            .color(Color32::from_rgb(100, 200, 100))
                                            .size(10.0),
                                    )
                                    .frame(false)
                                    .min_size(Vec2::new(14.0, 20.0)),
                                )
                                .on_hover_text("Source Module\nGenerates signal (no incoming connections).");
                            }

                            // Sink indicator (no outputs)
                            if is_sink {
                                ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(ri::DOWNLOAD_2_FILL)
                                            .color(Color32::from_rgb(200, 100, 100))
                                            .size(10.0),
                                    )
                                    .frame(false)
                                    .min_size(Vec2::new(14.0, 20.0)),
                                )
                                .on_hover_text("Sink Module\nConsumes signal (no outgoing connections).");
                            }

                            // Connectivity status indicator
                            let (conn_icon, conn_color, conn_tooltip): (_, _, &str) = if is_global_module {
                                if descriptor.category == ModuleCategory::Utility {
                                    (ri::FLASHLIGHT_FILL, Color32::from_rgb(100, 180, 220), "Internal Routing\nRoutes modulation internally — no cables needed.")
                                } else {
                                    (ri::FLASHLIGHT_FILL, Color32::from_rgb(100, 180, 220), "Global Module\nProcessed automatically via effect chain.")
                                }
                            } else {
                                match connectivity_status {
                                    ModuleConnectivity::Connected => (
                                        ri::LINK,
                                        Color32::from_rgb(100, 200, 100),
                                        "Routed to Output\nAudio from this module reaches the output.",
                                    ),
                                    ModuleConnectivity::Orphaned => (
                                        ri::ERROR_WARNING_LINE,
                                        Color32::from_rgb(200, 200, 100),
                                        "Orphaned\nHas connections but signal doesn't reach output.\nConnect to a module that leads to Output.",
                                    ),
                                    ModuleConnectivity::Disconnected => (
                                        ri::LINK_UNLINK,
                                        Color32::from_rgb(100, 100, 100),
                                        "Disconnected\nNo cables connected.\nDrag from ports to create connections.",
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
                                (ri::VOLUME_MUTE_FILL, t.colors.text_dim)
                            } else {
                                (ri::VOLUME_UP_FILL, t.colors.accent_green)
                            };
                            let power_tooltip = if is_bypassed {
                                "Bypassed\nModule output is muted.\nClick to activate."
                            } else {
                                "Active\nModule is processing audio.\nClick to bypass."
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

                            // Effect chain reorder buttons (up/down arrows)
                            if let Some(chain_pos) =
                                effect_chain_order.iter().position(|id| *id == module_id)
                            {
                                let chain_btn_size = Vec2::new(18.0, 20.0);
                                let can_move_up = chain_pos > 0;
                                let can_move_down =
                                    chain_pos + 1 < effect_chain_order.len();
                                let chain_color = Color32::from_rgb(230, 160, 50);

                                // Up arrow
                                let up_color = if can_move_up {
                                    chain_color
                                } else {
                                    chain_color.gamma_multiply(0.3)
                                };
                                let up_resp = ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new(ri::ARROW_UP_S_LINE)
                                                .color(up_color)
                                                .size(12.0),
                                        )
                                        .frame(false)
                                        .min_size(chain_btn_size),
                                    )
                                    .on_hover_text("Move up in chain (process earlier)");
                                if up_resp.clicked() && can_move_up {
                                    result.reorder_effects.push((
                                        module_id,
                                        synth_engine::ReorderDirection::Up,
                                    ));
                                }

                                // Down arrow
                                let down_color = if can_move_down {
                                    chain_color
                                } else {
                                    chain_color.gamma_multiply(0.3)
                                };
                                let down_resp = ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new(ri::ARROW_DOWN_S_LINE)
                                                .color(down_color)
                                                .size(12.0),
                                        )
                                        .frame(false)
                                        .min_size(chain_btn_size),
                                    )
                                    .on_hover_text("Move down in chain (process later)");
                                if down_resp.clicked() && can_move_down {
                                    result.reorder_effects.push((
                                        module_id,
                                        synth_engine::ReorderDirection::Down,
                                    ));
                                }
                            }

                            // Close/delete button (always visible)
                            ui.separator();
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(ri::CLOSE_LINE)
                                            .color(t.colors.text_dim)
                                            .size(12.0),
                                    )
                                    .frame(false)
                                    .min_size(button_min_size),
                                )
                                .on_hover_text("Delete module")
                                .clicked()
                            {
                                open = false;
                            }
                        },
                    );

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
                                    egui::RichText::new(format!(
                                        "{} Effect Chain",
                                        ri::FLASHLIGHT_FILL
                                    ))
                                    .size(11.0)
                                    .color(theme().colors.text_dim),
                                );

                                // Chain position badge
                                if let Some(chain_pos) =
                                    effect_chain_order.iter().position(|id| *id == module_id)
                                {
                                    let chain_label = format!("#{}", chain_pos + 1);
                                    ui.label(
                                        egui::RichText::new(chain_label)
                                            .size(10.0)
                                            .color(Color32::from_rgb(230, 160, 50)),
                                    );
                                }
                            });
                            ui.label(
                                egui::RichText::new("Applied automatically after voice mixing")
                                    .size(9.0)
                                    .color(theme().colors.text_dim.gamma_multiply(0.7)),
                            );
                        } else {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} Visualizer",
                                        egui_remixicon::icons::SPECTRUM_FILL
                                    ))
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
            // Update logical position and size from screen rect, snapped to grid.
            // Skip during the frame a patch/project was just loaded — egui's Area rects
            // are stale and would overwrite the freshly loaded saved positions.
            if !self.suppress_position_readback
                && let Some(area_rect) = ui.ctx().memory(|mem| mem.area_rect(window_id))
                && let Some(panel_state) = self.panels.get_mut(&module_id)
            {
                let logical_pos = area_rect.min - area_origin + scroll_offset;
                panel_state.position = snap_to_grid(logical_pos);
                panel_state.size = area_rect.size();
            }

            // Bring to front on click
            if area_response.response.clicked() || area_response.response.drag_started() {
                let modifiers = ui.input(|i| i.modifiers);
                if modifiers.shift || modifiers.ctrl {
                    if self.selected_modules.contains(&module_id) {
                        self.selected_modules.remove(&module_id);
                    } else {
                        self.selected_modules.insert(module_id);
                    }
                } else {
                    self.selected_modules.clear();
                    self.selected_modules.insert(module_id);
                }
                self.selected_module = Some(module_id);
                self.selected_group = None;
                bring_to_front = Some(module_id);
            }

            if area_response.response.drag_stopped()
                && let Some(panel_state) = self.panels.get(&module_id)
            {
                let center = panel_state.position + panel_state.size / 2.0;
                let mut target_group: Option<GroupId> = None;
                for (gid, rect) in &group_layout.rects_world {
                    if rect.contains(center) {
                        target_group = Some(*gid);
                        break;
                    }
                }
                match target_group {
                    Some(gid) => {
                        if self.group_of(module_id) != Some(gid) {
                            self.remove_from_group(module_id);
                            self.add_module_to_group(gid, module_id);
                        }
                    }
                    None => {
                        if self.group_of(module_id).is_some() {
                            self.remove_from_group(module_id);
                        }
                    }
                }
            }

            // Handle close (delete module) — triggered by close button
            // If the module is in a signal chain, bypass it (reconnect around it)
            if !open {
                self.bypass_and_remove(module_id, &mut result);
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
            let painter = eframe::egui::Painter::new(
                ui.ctx().clone(),
                LayerId::new(Order::Background, egui::Id::new("cables_active")),
                visible_rect,
            );
            draw_cable_dragging(&painter, pending.from_position, pending.current_pos, color);
        }

        // Handle click on empty space to deselect
        if let Some(ref response) = canvas_response
            && response.clicked()
        {
            self.selected_module = None;
            self.selected_modules.clear();
            self.selected_group = None;
        }

        // Right-click on background (or cable) → capture state for context menu
        if let Some(ref response) = canvas_response
            && response.secondary_clicked()
            && self.port_context_menu.is_none()
            && let Some(screen_pos) = ui.input(|i| i.pointer.interact_pos())
        {
            // Convert screen position to world/logical position
            let world_pos = Pos2::new(
                screen_pos.x - area_origin.x + scroll_offset.x,
                screen_pos.y - area_origin.y + scroll_offset.y,
            );
            self.bg_context_menu = Some(BgContextMenuState {
                world_pos,
                cable: self.hovered_cable,
            });
        }

        // Cancel pending connection / close context menus with escape
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_connection = None;
            self.port_context_menu = None;
            self.bg_context_menu = None;
        }

        // Draw toolbar in foreground layer (always on top, positioned relative to visible area)

        // Clear suppress flag — next frame will resume normal position tracking
        self.suppress_position_readback = false;

        result
    }

    /// Draw a vertical column of ports (input or output side).
    fn draw_panel_header_row<F>(
        ui: &mut Ui,
        accent_color: Color32,
        title: &str,
        hover_text: Option<String>,
        actions: F,
    ) where
        F: FnOnce(&mut Ui),
    {
        ui.horizontal(|ui| {
            // Accent color indicator
            let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 14.0), Sense::hover());
            ui.painter().rect_filled(rect, 2.0, accent_color);

            // Title
            let response = ui.label(egui::RichText::new(title).strong().color(accent_color));
            if let Some(hover) = hover_text {
                response.on_hover_text(hover);
            }

            actions(ui);
        });

        ui.separator();

        // Gradient tint spanning full module panel, from frame edge to separator
        let margin = ui.spacing().window_margin;
        let module_rect = ui.max_rect();
        let gradient_rect = egui::Rect::from_min_max(
            egui::pos2(
                module_rect.left() - f32::from(margin.left),
                module_rect.top() - f32::from(margin.top),
            ),
            egui::pos2(
                module_rect.right() + f32::from(margin.right),
                ui.min_rect().bottom(),
            ),
        );
        let tint = accent_color.gamma_multiply(0.12);
        let transparent = Color32::TRANSPARENT;
        let painter = ui.painter();
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(gradient_rect.left_top(), tint);
        mesh.colored_vertex(gradient_rect.right_top(), transparent);
        mesh.colored_vertex(gradient_rect.right_bottom(), transparent);
        mesh.colored_vertex(gradient_rect.left_bottom(), transparent);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(mesh));
    }

    fn draw_port_column_with<F>(
        ui: &mut Ui,
        direction: WidgetPortDirection,
        ports: &[PortRenderInfo],
        pending_info: Option<(ModuleId, WidgetPortType, WidgetPortDirection)>,
        mut store_position: F,
    ) where
        F: FnMut(&PortRenderInfo, Pos2),
    {
        let t = theme();
        let col_width = t.sizes.port_column_width;
        let spacing = t.sizes.port_vertical_spacing;

        ui.vertical(|ui| {
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

            for port in ports {
                let is_highlighted = pending_info
                    .map(|(from_module, from_type, from_dir)| {
                        from_module != port.module_id
                            && from_dir != direction
                            && from_type == port.port_type
                    })
                    .unwrap_or(false);

                ui.vertical_centered(|ui| {
                    ui.allocate_ui(Vec2::new(col_width, spacing), |ui| {
                        ui.centered_and_justified(|ui| {
                            let (response, center) =
                                super::widgets::PortWidget::new(port.port_type, direction)
                                    .connected(port.is_connected)
                                    .highlighted(is_highlighted)
                                    .label(&port.label)
                                    .show(ui);

                            store_position(port, center);

                            if !port.description.is_empty() {
                                response.on_hover_text(&port.description);
                            }
                        });
                    });
                });
            }
        });
    }

    fn draw_port_column(
        &mut self,
        ui: &mut Ui,
        module_id: ModuleId,
        descriptor: &ModuleDescriptor,
        direction: WidgetPortDirection,
        connected_ports: &[PortName],
    ) {
        use synth_core::PortDirection as CorePortDirection;

        let core_dir = match direction {
            WidgetPortDirection::Input => CorePortDirection::Input,
            WidgetPortDirection::Output => CorePortDirection::Output,
        };

        let ports: Vec<PortRenderInfo> = descriptor
            .ports
            .iter()
            .filter(|p| p.direction == core_dir)
            .map(|p| PortRenderInfo {
                module_id,
                port_name: p.name,
                label: p.label.clone(),
                description: p.description.clone(),
                port_type: convert_port_type(p.port_type),
                is_connected: connected_ports.contains(&p.name),
            })
            .collect();

        let pending_info = self
            .pending_connection
            .as_ref()
            .map(|p| (p.from_module, p.from_type, p.from_direction));
        let port_positions = &mut self.port_positions;
        Self::draw_port_column_with(ui, direction, &ports, pending_info, |port, center| {
            port_positions.insert(
                (module_id, port.port_name),
                PortPosition {
                    module_id,
                    port_name: port.port_name,
                    position: center,
                    port_type: port.port_type,
                    direction,
                },
            );
        });
    }

    fn draw_group_port_column(
        &mut self,
        ui: &mut Ui,
        group: &ModuleGroup,
        direction: WidgetPortDirection,
        new_positions: &mut HashMap<GroupPortKey, PortPosition>,
    ) {
        let ports = match direction {
            WidgetPortDirection::Input => &group.exposed_inputs,
            WidgetPortDirection::Output => &group.exposed_outputs,
        };

        let ports: Vec<PortRenderInfo> = ports
            .iter()
            .map(|p| PortRenderInfo {
                module_id: p.module_id,
                port_name: p.port_name,
                label: p.label.clone(),
                description: String::new(),
                port_type: self.port_widget_type(p.module_id, p.port_name),
                is_connected: self.has_external_connection_for_port(
                    group.id,
                    p.module_id,
                    p.port_name,
                    direction,
                ),
            })
            .collect();

        let pending_info = self
            .pending_connection
            .as_ref()
            .map(|p| (p.from_module, p.from_type, p.from_direction));
        Self::draw_port_column_with(ui, direction, &ports, pending_info, |port, center| {
            new_positions.insert(
                GroupPortKey {
                    group_id: group.id,
                    module_id: port.module_id,
                    port_name: port.port_name,
                    direction,
                },
                PortPosition {
                    module_id: port.module_id,
                    port_name: port.port_name,
                    position: center,
                    port_type: port.port_type,
                    direction,
                },
            );
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

    /// Draw a tinted background zone behind effect (and visualizer) modules.
    fn draw_effect_zone(&self, ui: &mut Ui, scroll_rect: Rect) {
        let padding = GRID_SIZE;

        // Compute bounding box of effect/visualizer modules in logical coordinates
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut has_effects = false;

        for (id, panel) in &self.panels {
            let category = self.descriptors.get(id).map(|d| d.category);
            if matches!(
                category,
                Some(ModuleCategory::Effect | ModuleCategory::Visualizer)
            ) {
                has_effects = true;
                min_x = min_x.min(panel.position.x);
                min_y = min_y.min(panel.position.y);
                max_x = max_x.max(panel.position.x + panel.size.x);
                max_y = max_y.max(panel.position.y + panel.size.y);
            }
        }

        if !has_effects {
            return;
        }

        // Expand to grid-aligned bounds with padding
        let origin = scroll_rect.min.to_vec2();
        let zone_rect = Rect::from_min_max(
            Pos2::new(
                ((min_x - padding) / GRID_SIZE).floor() * GRID_SIZE + origin.x,
                ((min_y - padding) / GRID_SIZE).floor() * GRID_SIZE + origin.y,
            ),
            Pos2::new(
                ((max_x + padding) / GRID_SIZE).ceil() * GRID_SIZE + origin.x,
                ((max_y + padding) / GRID_SIZE).ceil() * GRID_SIZE + origin.y,
            ),
        );

        let painter = ui.painter();

        // Tinted background
        let fx_color = category_color(ModuleCategory::Effect);
        let tint = fx_color.gamma_multiply(0.06);
        painter.rect_filled(zone_rect, 4.0, tint);

        // Border
        painter.rect_stroke(
            zone_rect,
            4.0,
            egui::Stroke::new(1.0, fx_color.gamma_multiply(0.15)),
            egui::StrokeKind::Inside,
        );

        // Label in top-left corner
        let label_pos = zone_rect.min + egui::vec2(8.0, 4.0);
        painter.text(
            label_pos,
            egui::Align2::LEFT_TOP,
            format!("{} Effect Chain", ri::FLASHLIGHT_FILL),
            egui::FontId::proportional(11.0),
            fx_color.gamma_multiply(0.35),
        );
    }

    /// Draw effect chain cables showing signal flow between effects.
    ///
    /// These are drawn as vertical cables (top-to-bottom) between consecutive
    /// effects in chain order, with arrowheads showing direction.
    fn draw_effect_chain_cables(
        &self,
        ui: &Ui,
        bg_layer: LayerId,
        clip_rect: Rect,
        effect_chain_order: &[ModuleId],
        area_origin: Vec2,
        scroll_offset: Vec2,
    ) {
        if effect_chain_order.len() < 2 {
            return;
        }

        let painter = eframe::egui::Painter::new(ui.ctx().clone(), bg_layer, clip_rect);

        // Warm amber color for chain cables
        let chain_color = Color32::from_rgb(230, 160, 50);
        let chain_stroke = egui::Stroke::new(2.5, chain_color.gamma_multiply(0.7));
        let arrow_color = chain_color.gamma_multiply(0.85);

        // Draw cables between consecutive effects
        for pair in effect_chain_order.windows(2) {
            let from_id = pair[0];
            let to_id = pair[1];

            // Get module panel positions
            let (Some(from_panel), Some(to_panel)) =
                (self.panels.get(&from_id), self.panels.get(&to_id))
            else {
                continue;
            };

            // Calculate screen positions: bottom-center of source, top-center of destination
            let from_screen = Pos2::new(
                from_panel.position.x + from_panel.size.x * 0.5 + area_origin.x - scroll_offset.x,
                from_panel.position.y + from_panel.size.y + area_origin.y - scroll_offset.y,
            );
            let to_screen = Pos2::new(
                to_panel.position.x + to_panel.size.x * 0.5 + area_origin.x - scroll_offset.x,
                to_panel.position.y + area_origin.y - scroll_offset.y,
            );

            // Draw vertical cable segments
            let mid_y = (from_screen.y + to_screen.y) * 0.5;
            if (from_screen.x - to_screen.x).abs() < 2.0 {
                // Straight vertical line
                painter.line_segment([from_screen, to_screen], chain_stroke);
            } else {
                // Orthogonal: down, across, down
                painter.line_segment([from_screen, Pos2::new(from_screen.x, mid_y)], chain_stroke);
                painter.line_segment(
                    [
                        Pos2::new(from_screen.x, mid_y),
                        Pos2::new(to_screen.x, mid_y),
                    ],
                    chain_stroke,
                );
                painter.line_segment([Pos2::new(to_screen.x, mid_y), to_screen], chain_stroke);
            }

            // Draw arrowhead at destination
            let arrow_size = 6.0;
            let arrow_tip = to_screen;
            let arrow_left = Pos2::new(arrow_tip.x - arrow_size, arrow_tip.y - arrow_size * 1.5);
            let arrow_right = Pos2::new(arrow_tip.x + arrow_size, arrow_tip.y - arrow_size * 1.5);
            painter.add(egui::Shape::convex_polygon(
                vec![arrow_tip, arrow_left, arrow_right],
                arrow_color,
                egui::Stroke::NONE,
            ));
        }

        // Draw "IN" label/arrow above the first effect
        if let Some(first_panel) = self.panels.get(&effect_chain_order[0]) {
            let pos = Pos2::new(
                first_panel.position.x + first_panel.size.x * 0.5 + area_origin.x - scroll_offset.x,
                first_panel.position.y + area_origin.y - scroll_offset.y - 4.0,
            );
            let arrow_top = Pos2::new(pos.x, pos.y - 14.0);
            painter.line_segment([arrow_top, pos], chain_stroke);
            // Small arrowhead
            let s = 4.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos,
                    Pos2::new(pos.x - s, pos.y - s * 1.5),
                    Pos2::new(pos.x + s, pos.y - s * 1.5),
                ],
                arrow_color,
                egui::Stroke::NONE,
            ));
            painter.text(
                Pos2::new(pos.x, arrow_top.y - 2.0),
                egui::Align2::CENTER_BOTTOM,
                "IN",
                egui::FontId::proportional(9.0),
                chain_color.gamma_multiply(0.6),
            );
        }

        // Draw "OUT" label/arrow below the last effect
        if let Some(last_panel) = self
            .panels
            .get(effect_chain_order.last().unwrap_or(&effect_chain_order[0]))
        {
            let pos = Pos2::new(
                last_panel.position.x + last_panel.size.x * 0.5 + area_origin.x - scroll_offset.x,
                last_panel.position.y + last_panel.size.y + area_origin.y - scroll_offset.y + 4.0,
            );
            let arrow_bottom = Pos2::new(pos.x, pos.y + 14.0);
            painter.line_segment([pos, arrow_bottom], chain_stroke);
            let s = 4.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    arrow_bottom,
                    Pos2::new(arrow_bottom.x - s, arrow_bottom.y - s * 1.5),
                    Pos2::new(arrow_bottom.x + s, arrow_bottom.y - s * 1.5),
                ],
                arrow_color,
                egui::Stroke::NONE,
            ));
            painter.text(
                Pos2::new(pos.x, arrow_bottom.y + 2.0),
                egui::Align2::CENTER_TOP,
                "OUT",
                egui::FontId::proportional(9.0),
                chain_color.gamma_multiply(0.6),
            );
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
        let fg_painter = eframe::egui::Painter::new(
            ui.ctx().clone(),
            LayerId::new(Order::Background, egui::Id::new("cables_active_fg")),
            clip_rect,
        );

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
            if self.is_hidden_internal_connection(c) {
                continue;
            }
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
            if self.is_hidden_internal_connection(connection) {
                continue;
            }
            let idx = dest_index.entry(connection.to_module).or_default();
            let n = dest_count.get(&connection.to_module).copied().unwrap_or(1);
            let spread = (*idx as f32 - (n as f32 - 1.0) / 2.0) * CABLE_SPREAD;
            *dest_index.get_mut(&connection.to_module).unwrap_or(&mut 0) += 1;

            let from_pos = self.resolve_connection_endpoint(
                connection.from_module,
                connection.from_port,
                connection.to_module,
                WidgetPortDirection::Output,
            );
            let to_pos = self.resolve_connection_endpoint(
                connection.to_module,
                connection.to_port,
                connection.from_module,
                WidgetPortDirection::Input,
            );

            if let (Some(from_pos), Some(to_pos)) = (from_pos, to_pos) {
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

    fn draw_group_frames(&self, ui: &Ui, layout: &GroupLayout, layer_id: LayerId, clip_rect: Rect) {
        let painter = eframe::egui::Painter::new(ui.ctx().clone(), layer_id, clip_rect);
        for group in self.groups.values() {
            if group.collapsed {
                continue;
            }
            let Some(rect) = layout.rects_screen.get(&group.id) else {
                continue;
            };
            let base_color = self.group_color(group);
            let stroke_width = if self.selected_group == Some(group.id) {
                2.0
            } else {
                1.0
            };
            let stroke = egui::Stroke::new(stroke_width, base_color.gamma_multiply(0.6));
            painter.rect(
                *rect,
                6.0,
                Color32::from_rgba_unmultiplied(0, 0, 0, 0),
                stroke,
                egui::StrokeKind::Inside,
            );

            // Header strip
            let header_rect =
                Rect::from_min_size(rect.min, Vec2::new(rect.width(), GROUP_HEADER_HEIGHT));
            painter.rect_filled(header_rect, 6.0, base_color.gamma_multiply(0.15));
            painter.text(
                header_rect.min + Vec2::new(8.0, 4.0),
                egui::Align2::LEFT_TOP,
                &group.name,
                egui::FontId::proportional(12.0),
                base_color.gamma_multiply(0.9),
            );

            // Menu icon (⋯)
            let menu_rect = group_menu_icon_rect(*rect);
            painter.rect_stroke(
                menu_rect,
                3.0,
                egui::Stroke::new(1.0, base_color.gamma_multiply(0.5)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                menu_rect.center(),
                egui::Align2::CENTER_CENTER,
                ri::MORE_LINE,
                egui::FontId::proportional(12.0),
                base_color.gamma_multiply(0.9),
            );

            // Collapse icon
            let icon_rect = group_toggle_icon_rect(*rect);
            painter.rect_stroke(
                icon_rect,
                3.0,
                egui::Stroke::new(1.0, base_color.gamma_multiply(0.5)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                ri::SUBTRACT_LINE,
                egui::FontId::proportional(12.0),
                base_color.gamma_multiply(0.9),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_collapsed_groups(
        &mut self,
        ui: &Ui,
        layout: &GroupLayout,
        instrument_id: u64,
        visible_rect: Rect,
        area_origin: Vec2,
        scroll_offset: Vec2,
        result: &mut PatchEditorResult,
        new_positions: &mut HashMap<GroupPortKey, PortPosition>,
    ) {
        let group_ids: Vec<GroupId> = self.groups.keys().copied().collect();
        for group_id in group_ids {
            let Some(group) = self.groups.get(&group_id).cloned() else {
                continue;
            };
            if !group.collapsed {
                continue;
            }

            let rect_screen = layout
                .rects_screen
                .get(&group_id)
                .copied()
                .unwrap_or_else(|| {
                    let size = collapsed_group_size(&group);
                    Rect::from_min_size(group.position + area_origin - scroll_offset, size)
                });

            let area_id = Id::new((instrument_id, "group_box", group_id.0));
            let area = egui::Area::new(area_id)
                .order(Order::Background)
                .movable(true)
                .current_pos(rect_screen.min);

            let mut toggle_clicked = false;
            let mut delete_clicked = false;
            let mut menu_clicked = false;
            let mut menu_pos = Pos2::ZERO;
            let response = area.show(ui.ctx(), |ui| {
                ui.set_clip_rect(visible_rect);
                let base_color = self.group_color(&group);
                let stroke_width = if self.selected_group == Some(group_id) {
                    2.0
                } else {
                    1.0
                };
                let frame = egui::Frame::window(&ui.ctx().style())
                    .fill(ui.ctx().style().visuals.window_fill())
                    .stroke(egui::Stroke::new(
                        stroke_width,
                        base_color.gamma_multiply(0.6),
                    ))
                    .corner_radius(6.0);
                frame.show(ui, |ui| {
                    let button_min_size = Vec2::new(20.0, 20.0);
                    let t = theme();
                    Self::draw_panel_header_row(
                        ui,
                        base_color.gamma_multiply(0.9),
                        &group.name,
                        Some(format!("Group ID: {}", group_id.0)),
                        |ui| {
                            ui.separator();
                            let menu_resp = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(ri::MORE_LINE)
                                            .color(t.colors.text_dim)
                                            .size(12.0),
                                    )
                                    .frame(false)
                                    .min_size(button_min_size),
                                )
                                .on_hover_text("Group menu");
                            if menu_resp.clicked() {
                                menu_clicked = true;
                                menu_pos = menu_resp.rect.left_bottom();
                            }

                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(ri::ADD_LINE)
                                            .color(t.colors.text_dim)
                                            .size(12.0),
                                    )
                                    .frame(false)
                                    .min_size(button_min_size),
                                )
                                .on_hover_text("Expand group")
                                .clicked()
                            {
                                toggle_clicked = true;
                            }

                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(ri::CLOSE_LINE)
                                            .color(t.colors.text_dim)
                                            .size(12.0),
                                    )
                                    .frame(false)
                                    .min_size(button_min_size),
                                )
                                .on_hover_text("Delete group")
                                .clicked()
                            {
                                delete_clicked = true;
                            }
                        },
                    );

                    ui.horizontal(|ui| {
                        // Left port column (IN)
                        ui.vertical(|ui| {
                            ui.set_width(t.sizes.port_column_width);
                            self.draw_group_port_column(
                                ui,
                                &group,
                                WidgetPortDirection::Input,
                                new_positions,
                            );
                        });

                        // Content column
                        ui.vertical(|ui| {
                            ui.set_min_width(40.0);
                            ui.label(
                                egui::RichText::new(format!("{} modules", group.members.len()))
                                    .size(10.0)
                                    .color(t.colors.text_dim),
                            );
                            if group.exposed_inputs.is_empty() && group.exposed_outputs.is_empty() {
                                ui.label(
                                    egui::RichText::new("No exposed ports")
                                        .size(9.0)
                                        .color(t.colors.text_dim.gamma_multiply(0.7)),
                                );
                            }
                        });

                        // Right port column (OUT)
                        ui.vertical(|ui| {
                            ui.set_width(t.sizes.port_column_width);
                            self.draw_group_port_column(
                                ui,
                                &group,
                                WidgetPortDirection::Output,
                                new_positions,
                            );
                        });
                    });
                });
            });

            if delete_clicked {
                self.delete_group(group_id, result);
                continue;
            }

            if let Some(area_rect) = ui.ctx().memory(|mem| mem.area_rect(area_id))
                && let Some(group_mut) = self.groups.get_mut(&group_id)
            {
                let logical_pos = area_rect.min - area_origin + scroll_offset;
                group_mut.position = snap_to_grid(logical_pos);
            }

            if toggle_clicked && let Some(group_mut) = self.groups.get_mut(&group_id) {
                group_mut.collapsed = false;
                continue;
            }

            if menu_clicked {
                self.group_context_menu = Some(GroupContextMenuState { group_id, menu_pos });
            }

            if response.response.clicked() {
                self.selected_group = Some(group_id);
                self.selected_modules.clear();
                self.selected_module = None;
            }
            if response.response.double_clicked()
                && let Some(group_mut) = self.groups.get_mut(&group_id)
            {
                group_mut.collapsed = false;
            }
        }
    }

    fn handle_group_interactions(&mut self, ui: &Ui, layout: &GroupLayout, module_rects: &[Rect]) {
        let pointer_pos = ui.input(|i| i.pointer.interact_pos());
        let Some(pos) = pointer_pos else {
            return;
        };
        let over_module = module_rects.iter().any(|r| r.contains(pos));
        if over_module {
            return;
        }

        let mut target_group: Option<GroupId> = None;
        for (gid, rect) in &layout.rects_screen {
            if rect.contains(pos)
                && let Some(group) = self.groups.get(gid)
                && !group.collapsed
            {
                target_group = Some(*gid);
                break;
            }
        }

        let Some(group_id) = target_group else {
            return;
        };

        if let Some(rect) = layout.rects_screen.get(&group_id) {
            let primary_clicked =
                ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));

            // Menu icon (⋯) click
            let menu_rect = group_menu_icon_rect(*rect);
            if menu_rect.contains(pos) && primary_clicked {
                self.group_context_menu = Some(GroupContextMenuState {
                    group_id,
                    menu_pos: Pos2::new(menu_rect.left(), menu_rect.bottom()),
                });
                return;
            }

            // Collapse icon click
            let icon_rect = group_toggle_icon_rect(*rect);
            if icon_rect.contains(pos) && primary_clicked {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    group.collapsed = true;
                    if let Some(rect_world) = layout.rects_world.get(&group_id) {
                        group.position = rect_world.min;
                    }
                }
                return;
            }
        }

        if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary)) {
            self.selected_group = Some(group_id);
            self.selected_modules.clear();
            self.selected_module = None;
        }
        if ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        }) && let Some(group) = self.groups.get_mut(&group_id)
        {
            group.collapsed = true;
            if let Some(rect) = layout.rects_world.get(&group_id) {
                group.position = rect.min;
            }
        }
    }

    /// Draw the background right-click context menu for adding modules.
    #[allow(clippy::too_many_lines)]
    fn draw_bg_context_menu(&mut self, response: &egui::Response, result: &mut PatchEditorResult) {
        use egui_remixicon::icons as ri;
        let menu_state = self.bg_context_menu;
        let world_pos = menu_state.map(|s| s.world_pos).unwrap_or_default();
        let menu_cable = menu_state.and_then(|s| s.cable);
        let mut selected: Option<PaletteSelection> = None;
        let mut cable_action_taken = false;

        response.context_menu(|ui| {
            // --- Layout ---
            ui.label(
                egui::RichText::new("Layout")
                    .color(theme().colors.text_secondary)
                    .size(11.0),
            );
            if ui
                .button(format!("{} Auto Layout", ri::LAYOUT_GRID_FILL))
                .clicked()
            {
                result.request_auto_layout = true;
                ui.close();
            }

            ui.separator();

            if !self.selected_modules.is_empty() {
                if ui.button("Create group from selection").clicked() {
                    self.create_group_from_selection();
                    ui.close();
                }
                ui.separator();
            }

            if ui
                .button(format!("{} Insert Group Template", ri::FOLDER_ADD_LINE))
                .clicked()
            {
                result.group_template_action = Some(GroupTemplateAction::OpenBrowser {
                    drop_pos: world_pos,
                });
                ui.close();
            }

            ui.separator();

            // Cable actions (shown when right-clicking on a hovered cable)
            if let Some(connection) = menu_cable {
                if ui.button("Delete cable").clicked() {
                    self.connections.retain(|c| c != &connection);
                    result.connections_to_remove.push(connection);
                    self.calculate_connectivity();
                    cable_action_taken = true;
                    ui.close();
                }

                if ui.button("Insert Signal Monitor").clicked() {
                    self.connections.retain(|c| c != &connection);
                    result.connections_to_remove.push(connection);
                    result.insert_signal_monitor_at.push(connection);
                    self.calculate_connectivity();
                    cable_action_taken = true;
                    ui.close();
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
                egui::RichText::new(format!("{} Oscillator", ri::MUSIC_2_FILL)).color(osc_color),
                |ui| {
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Category(ModuleCategory::Oscillator),
                        &mut selected,
                    );
                    Self::bg_menu_item(ui, PaletteSelection::MathOscillator, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::SubOscillator, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::Noise, &mut selected);
                    ui.separator();
                    Self::bg_menu_item(ui, PaletteSelection::WavetableOsc, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::AdditiveOsc, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::GranularOsc, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::FractalOsc, &mut selected);
                },
            );

            // Simple categories (single module type each)
            for sel in [
                PaletteSelection::Category(ModuleCategory::Filter),
                PaletteSelection::Category(ModuleCategory::Envelope),
                PaletteSelection::Category(ModuleCategory::LFO),
                PaletteSelection::Category(ModuleCategory::Amplifier),
                PaletteSelection::Category(ModuleCategory::Mixer),
            ] {
                Self::bg_menu_item(ui, sel, &mut selected);
            }

            ui.separator();

            // Effect submenu
            let fx_color = category_color(ModuleCategory::Effect);
            ui.menu_button(
                egui::RichText::new(format!("{} Effect", ri::SPARKLING_FILL)).color(fx_color),
                |ui| {
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Delay),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Reverb),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Distortion),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Chorus),
                        &mut selected,
                    );
                    ui.separator();
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Flanger),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Phaser),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Compressor),
                        &mut selected,
                    );
                    Self::bg_menu_item(ui, PaletteSelection::Effect(EffectType::Eq), &mut selected);
                    ui.separator();
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Waveshaper),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::BbdDelay),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Limiter),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::MidSide),
                        &mut selected,
                    );
                    ui.separator();
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::Convolver),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::PhaseVocoder),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Effect(EffectType::FrequencyShifter),
                        &mut selected,
                    );
                },
            );

            // Visualizer submenu
            let viz_color = category_color(ModuleCategory::Visualizer);
            ui.menu_button(
                egui::RichText::new(format!("{} Visualizer", ri::SPECTRUM_FILL)).color(viz_color),
                |ui| {
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Visualizer(PaletteVisualizerType::Oscilloscope),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Visualizer(PaletteVisualizerType::LevelMeter),
                        &mut selected,
                    );
                    Self::bg_menu_item(
                        ui,
                        PaletteSelection::Visualizer(PaletteVisualizerType::SpectrumAnalyzer),
                        &mut selected,
                    );
                    ui.separator();
                    Self::bg_menu_item(ui, PaletteSelection::SignalMonitor, &mut selected);
                },
            );

            // Modulation submenu
            let mod_color = category_color(ModuleCategory::Utility);
            ui.menu_button(
                egui::RichText::new(format!("{} Modulation", ri::SWAP_FILL)).color(mod_color),
                |ui| {
                    Self::bg_menu_item(ui, PaletteSelection::RingMod, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::EnvelopeFollower, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::Mseg, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::KineticModulator, &mut selected);
                },
            );

            // Generative submenu
            let gen_color = category_color(ModuleCategory::LFO);
            ui.menu_button(
                egui::RichText::new(format!("{} Generative", ri::MAGIC_FILL)).color(gen_color),
                |ui| {
                    Self::bg_menu_item(ui, PaletteSelection::Euclidean, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::TuringMachine, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::RandomGates, &mut selected);
                },
            );

            // Physical submenu
            let phys_color = category_color(ModuleCategory::PhysicalModeling);
            ui.menu_button(
                egui::RichText::new(format!("{} Physical", ri::PIANO_FILL)).color(phys_color),
                |ui| {
                    Self::bg_menu_item(ui, PaletteSelection::KeyboardPanner, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::BodyResonance, &mut selected);
                    Self::bg_menu_item(ui, PaletteSelection::MechanicalNoise, &mut selected);
                },
            );

            ui.separator();

            // Output & Mod Matrix as direct buttons
            Self::bg_menu_item(ui, PaletteSelection::StereoOutput, &mut selected);
            Self::bg_menu_item(ui, PaletteSelection::ModMatrix, &mut selected);
        });

        if cable_action_taken {
            self.bg_context_menu = None;
            return;
        }

        if let Some(sel) = selected {
            result.context_add = Some((sel, world_pos, menu_cable));
            self.bg_context_menu = None;
        }

        // Clear stored state when context menu closes
        if !response.context_menu_opened() {
            self.bg_context_menu = None;
        }
    }

    /// Helper for background context menu items — uses shared palette_label for icon + color.
    fn bg_menu_item(ui: &mut Ui, selection: PaletteSelection, out: &mut Option<PaletteSelection>) {
        let (label, color) = palette_label(selection);
        if ui.button(egui::RichText::new(label).color(color)).clicked() {
            *out = Some(selection);
            ui.close();
        }
    }

    /// Draw the port right-click context menu for quick-adding connected modules.
    #[allow(clippy::too_many_lines)]
    fn draw_port_context_menu(&mut self, ui: &Ui, result: &mut PatchEditorResult) {
        let Some(ref state) = self.port_context_menu else {
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

        let mut open = true;
        egui::Popup::new(
            menu_id,
            ui.ctx().clone(),
            egui::PopupAnchor::Position(menu_pos),
            ui.layer_id(),
        )
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .style(egui::containers::menu::menu_style)
        .gap(0.0)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
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

            // Group exposure controls (if the module belongs to a group)
            if let Some(group_id) = self.group_of(target_module) {
                let is_exposed =
                    self.is_port_exposed(group_id, target_module, target_port, target_direction);
                if is_exposed {
                    let can_hide = !self.has_external_connection_for_port(
                        group_id,
                        target_module,
                        target_port,
                        target_direction,
                    );
                    let resp = ui.add_enabled(can_hide, egui::Button::new("Hide group port"));
                    if resp.clicked() {
                        let _ =
                            self.hide_port(group_id, target_module, target_port, target_direction);
                        close_menu = true;
                    } else if !can_hide {
                        resp.on_hover_text("Port has external connections");
                    }
                } else if ui.button("Expose as group port").clicked() {
                    let _ =
                        self.expose_port(group_id, target_module, target_port, target_direction);
                    close_menu = true;
                }
                ui.separator();
            }

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
                                    PaletteSelection::Category(ModuleCategory::Oscillator),
                                    PaletteSelection::SubOscillator,
                                    PaletteSelection::WavetableOsc,
                                    PaletteSelection::MathOscillator,
                                    PaletteSelection::AdditiveOsc,
                                    PaletteSelection::GranularOsc,
                                    PaletteSelection::FractalOsc,
                                    PaletteSelection::Noise,
                                    PaletteSelection::RingMod,
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
                                    PaletteSelection::Category(ModuleCategory::LFO),
                                    PaletteSelection::Category(ModuleCategory::Envelope),
                                    PaletteSelection::Mseg,
                                    PaletteSelection::KineticModulator,
                                    PaletteSelection::EnvelopeFollower,
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
                                    PaletteSelection::Euclidean,
                                    PaletteSelection::TuringMachine,
                                    PaletteSelection::RandomGates,
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
                                    PaletteSelection::Category(ModuleCategory::Filter),
                                    PaletteSelection::Category(ModuleCategory::Amplifier),
                                    PaletteSelection::Category(ModuleCategory::Mixer),
                                    PaletteSelection::SignalMonitor,
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
                                    PaletteSelection::Effect(EffectType::Delay),
                                    PaletteSelection::Effect(EffectType::Reverb),
                                    PaletteSelection::Effect(EffectType::Distortion),
                                    PaletteSelection::Effect(EffectType::Chorus),
                                    PaletteSelection::Effect(EffectType::Flanger),
                                    PaletteSelection::Effect(EffectType::Phaser),
                                    PaletteSelection::Effect(EffectType::Compressor),
                                    PaletteSelection::Effect(EffectType::Eq),
                                    PaletteSelection::Effect(EffectType::Waveshaper),
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
                                    PaletteSelection::Category(ModuleCategory::Amplifier),
                                    PaletteSelection::Category(ModuleCategory::Filter),
                                    PaletteSelection::Category(ModuleCategory::Oscillator),
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
                                    PaletteSelection::Category(ModuleCategory::Envelope),
                                    PaletteSelection::Category(ModuleCategory::Amplifier),
                                ],
                            );
                        }
                        WidgetPortType::Midi => {}
                    }
                }
            }
        });

        if close_menu || !open {
            self.port_context_menu = None;
        }
    }

    fn draw_group_context_menu(&mut self, ui: &Ui, result: &mut PatchEditorResult) {
        let Some(state) = self.group_context_menu.clone() else {
            return;
        };
        let Some(group) = self.groups.get(&state.group_id).cloned() else {
            self.group_context_menu = None;
            return;
        };

        let menu_id = egui::Id::new("group_context_menu");
        let mut open = true;
        let mut close_menu = false;
        let mut commit_name: Option<String> = None;
        let mut color_update: Option<Option<HexColor>> = None;
        let mut do_save_template = false;
        let mut do_ungroup = false;
        let mut do_delete = false;

        let mut name_buf = self
            .group_name_edit
            .take()
            .and_then(|(gid, name)| {
                if gid == state.group_id {
                    Some(name)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| group.name.clone());

        egui::Popup::new(
            menu_id,
            ui.ctx().clone(),
            egui::PopupAnchor::Position(state.menu_pos),
            ui.layer_id(),
        )
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .style(egui::containers::menu::menu_style)
        .gap(0.0)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.label(
                egui::RichText::new(&group.name)
                    .color(theme().colors.text_secondary)
                    .size(11.0),
            );
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Name");
                let resp = ui.text_edit_singleline(&mut name_buf);
                if (resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    && !name_buf.trim().is_empty()
                {
                    commit_name = Some(name_buf.trim().to_string());
                }
            });

            ui.horizontal(|ui| {
                ui.label("Color");
                let mut color = self.group_color(&group);
                let changed = egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut color,
                    egui::color_picker::Alpha::BlendOrAdditive,
                )
                .changed();
                if changed {
                    color_update = Some(Some(color32_to_hex(color)));
                }
                if ui.button("Clear").clicked() {
                    color_update = Some(None);
                }
            });

            ui.separator();

            if ui.button("Save group as template").clicked() {
                do_save_template = true;
                close_menu = true;
            }

            if ui.button("Ungroup").clicked() {
                do_ungroup = true;
                close_menu = true;
            }

            if ui.button("Delete group").clicked() {
                do_delete = true;
                close_menu = true;
            }
        });

        if let Some(new_name) = commit_name
            && let Some(g) = self.groups.get_mut(&state.group_id)
        {
            g.name = new_name;
        }

        if let Some(color_opt) = color_update
            && let Some(g) = self.groups.get_mut(&state.group_id)
        {
            g.color = color_opt;
        }

        if do_save_template {
            result.group_template_action = Some(GroupTemplateAction::SaveGroup {
                group_id: state.group_id,
            });
        }

        if do_ungroup {
            self.ungroup(state.group_id);
        }

        if do_delete {
            self.delete_group(state.group_id, result);
        }

        if !close_menu && open {
            self.group_name_edit = Some((state.group_id, name_buf));
        } else {
            self.group_name_edit = None;
        }

        if close_menu || !open {
            self.group_context_menu = None;
        }
    }

    /// Helper: render a list of menu buttons that push `QuickAddRequest`s.
    /// Uses shared `palette_label` for consistent icons and colors.
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
        items: &[PaletteSelection],
    ) {
        for &selection in items {
            let (label, color) = palette_label(selection);
            if ui.button(egui::RichText::new(label).color(color)).clicked() {
                result.quick_add_requests.push(QuickAddRequest {
                    selection,
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
                }
            }
        }

        // Check for group port clicks
        for port_pos in self.group_port_positions.values() {
            let port_rect = Rect::from_center_size(port_pos.position, Vec2::splat(20.0));

            if let Some(pos) = pointer_pos
                && port_rect.contains(pos)
            {
                if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary)) {
                    if let Some(ref pending) = self.pending_connection {
                        if self.can_connect(pending, port_pos) {
                            let connection =
                                if pending.from_direction == WidgetPortDirection::Output {
                                    Connection::new(
                                        pending.from_module,
                                        pending.from_port,
                                        port_pos.module_id,
                                        port_pos.port_name,
                                    )
                                } else {
                                    Connection::new(
                                        port_pos.module_id,
                                        port_pos.port_name,
                                        pending.from_module,
                                        pending.from_port,
                                    )
                                };
                            result.connections_to_add.push(connection);
                        }
                        self.pending_connection = None;
                    } else {
                        self.pending_connection = Some(PendingConnection {
                            from_module: port_pos.module_id,
                            from_port: port_pos.port_name,
                            from_position: port_pos.position,
                            from_type: port_pos.port_type,
                            from_direction: port_pos.direction,
                            current_pos: pos,
                        });
                    }
                }

                if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary)) {
                    self.port_context_menu = Some(PortContextMenuState {
                        module_id: port_pos.module_id,
                        port_name: port_pos.port_name,
                        port_type: port_pos.port_type,
                        direction: port_pos.direction,
                        menu_pos: pos,
                    });
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

    /// Get the set of currently multi-selected modules.
    pub fn selected_module_ids(&self) -> &HashSet<ModuleId> {
        &self.selected_modules
    }

    /// Collect the effective selection: multi-selection if non-empty,
    /// otherwise the single selected module (if any).
    pub fn effective_selection(&self) -> HashSet<ModuleId> {
        if !self.selected_modules.is_empty() {
            self.selected_modules.clone()
        } else if let Some(id) = self.selected_module {
            let mut set = HashSet::new();
            set.insert(id);
            set
        } else {
            HashSet::new()
        }
    }

    /// Extract `ModuleState` for each given module ID.
    ///
    /// Builds serializable module states from the current panel data,
    /// suitable for clipboard storage.
    pub fn extract_module_states(&self, ids: &HashSet<ModuleId>) -> Vec<ModuleState> {
        let mut states = Vec::new();
        for &id in ids {
            if let Some((_desc, position, gui_params)) = self.get_module_data(id) {
                let mut parameters = std::collections::BTreeMap::new();
                for (name, value) in &gui_params {
                    parameters.insert(name.clone(), ParamValue::Float(*value));
                }
                states.push(ModuleState {
                    id: id.to_string(),
                    module_type: id.module_type,
                    position: Position::new(position.x, position.y),
                    parameters,
                });
            }
        }
        states
    }

    /// Find all connections where both endpoints are in the given module set.
    pub fn internal_connections(&self, ids: &HashSet<ModuleId>) -> Vec<ConnectionState> {
        self.connections
            .iter()
            .filter(|c| ids.contains(&c.from_module) && ids.contains(&c.to_module))
            .map(ConnectionState::from)
            .collect()
    }

    /// Select a set of modules (replaces current multi-selection).
    pub fn select_modules(&mut self, ids: &HashSet<ModuleId>) {
        self.selected_modules = ids.clone();
        self.selected_module = ids.iter().next().copied();
    }

    /// Align effect chain modules in a vertical column, preserving their x-center
    /// and stacking them top-to-bottom in chain order with consistent spacing.
    fn align_effect_chain(&mut self, effect_chain_order: &[ModuleId]) {
        if effect_chain_order.is_empty() {
            return;
        }

        // Only consider modules that actually exist in panels
        let existing: Vec<ModuleId> = effect_chain_order
            .iter()
            .filter(|id| self.panels.contains_key(id))
            .copied()
            .collect();
        if existing.is_empty() {
            return;
        }

        // Use the average x-center of all effect modules as the column x
        let avg_x = existing
            .iter()
            .filter_map(|id| self.panels.get(id))
            .map(|p| p.position.x + p.size.x * 0.5)
            .sum::<f32>()
            / existing.len() as f32;

        // Find the topmost y as starting point
        let start_y = existing
            .iter()
            .filter_map(|id| self.panels.get(id))
            .map(|p| p.position.y)
            .fold(f32::MAX, f32::min);

        // Stack modules vertically in chain order
        let gap = GRID_SIZE;
        let mut y = start_y;
        for &id in &existing {
            if let Some(panel) = self.panels.get_mut(&id) {
                let x = avg_x - panel.size.x * 0.5;
                panel.position = snap_to_grid(Pos2::new(x, y));
                y = panel.position.y + panel.size.y + gap;
                self.needs_reposition.insert(id);
            }
        }
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

    /// Set a minimum canvas size hint (restored from a saved patch).
    pub fn set_min_canvas_size(&mut self, size: Vec2) {
        self.min_canvas_size = Some(size);
    }

    /// Apply automatic layout to modules based on signal flow.
    ///
    /// `available_rect` should be the area available for modules (excluding side panels).
    pub fn apply_auto_layout(
        &mut self,
        available_rect: egui::Rect,
        effect_chain_order: &[ModuleId],
    ) {
        use super::auto_layout::{LayoutConnection, ModuleInfo, calculate_layout_with_chain_order};

        // Clear saved canvas size hint — auto-layout determines the new bounds
        self.min_canvas_size = None;

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

        let result = calculate_layout_with_chain_order(
            &modules,
            &connections,
            layout_rect,
            effect_chain_order,
        );

        // Apply new positions and mark for repositioning
        for (module_id, position) in result.positions {
            if let Some(panel) = self.panels.get_mut(&module_id) {
                panel.position = position;
                // Mark this module to use current_pos() on next frame
                self.needs_reposition.insert(module_id);
            }
        }

        // Re-align effect chain into a clean vertical column after auto-layout
        self.align_effect_chain(effect_chain_order);
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
    /// Requests to open template browser or save group templates.
    pub group_template_action: Option<GroupTemplateAction>,
    /// Requests to reorder effects in the chain (module_id, direction).
    pub reorder_effects: Vec<(ModuleId, synth_engine::ReorderDirection)>,
}

impl PatchEditorResult {
    /// Returns true if any mutation occurred (parameter, module, or connection change).
    #[must_use]
    pub fn has_mutations(&self) -> bool {
        !self.param_changes.is_empty()
            || !self.modules_to_remove.is_empty()
            || !self.connections_to_add.is_empty()
            || !self.connections_to_remove.is_empty()
            || !self.bypass_toggles.is_empty()
            || !self.quick_add_requests.is_empty()
            || self.context_add.is_some()
            || !self.insert_signal_monitor_at.is_empty()
    }
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
    FractalOsc,
}

/// Get the display label (with Remix Icon) and category color for a palette selection.
fn palette_label(selection: PaletteSelection) -> (String, Color32) {
    use egui_remixicon::icons as ri;

    let (icon, text, color) = match selection {
        // Oscillators
        PaletteSelection::Category(ModuleCategory::Oscillator) => (
            ri::MUSIC_2_FILL,
            "Oscillator",
            category_color(ModuleCategory::Oscillator),
        ),
        PaletteSelection::MathOscillator => (
            ri::FUNCTION_FILL,
            "Math Osc",
            category_color(ModuleCategory::Oscillator),
        ),
        PaletteSelection::SubOscillator => (
            ri::VOLUME_DOWN_FILL,
            "Sub Osc",
            category_color(ModuleCategory::Oscillator),
        ),
        PaletteSelection::Noise => (
            ri::SIGNAL_WIFI_FILL,
            "Noise",
            category_color(ModuleCategory::Oscillator),
        ),
        PaletteSelection::WavetableOsc => (
            ri::BAR_CHART_FILL,
            "Wavetable",
            category_color(ModuleCategory::Oscillator),
        ),
        PaletteSelection::AdditiveOsc => (
            ri::MUSIC_FILL,
            "Additive",
            category_color(ModuleCategory::Oscillator),
        ),
        PaletteSelection::GranularOsc => (
            ri::RHYTHM_FILL,
            "Granular",
            category_color(ModuleCategory::Oscillator),
        ),
        PaletteSelection::FractalOsc => (
            ri::SEEDLING_FILL,
            "Fractal Osc",
            category_color(ModuleCategory::Oscillator),
        ),
        // Simple categories
        PaletteSelection::Category(ModuleCategory::Filter) => (
            ri::FILTER_FILL,
            "Filter",
            category_color(ModuleCategory::Filter),
        ),
        PaletteSelection::Category(ModuleCategory::Envelope) => (
            ri::LINE_CHART_FILL,
            "Envelope",
            category_color(ModuleCategory::Envelope),
        ),
        PaletteSelection::Category(ModuleCategory::LFO) => {
            (ri::PULSE_FILL, "LFO", category_color(ModuleCategory::LFO))
        }
        PaletteSelection::Category(ModuleCategory::Amplifier) => (
            ri::VOLUME_UP_FILL,
            "VCA",
            category_color(ModuleCategory::Amplifier),
        ),
        PaletteSelection::Category(ModuleCategory::Mixer) => (
            ri::EQUALIZER_FILL,
            "Mixer",
            category_color(ModuleCategory::Mixer),
        ),
        // Effects
        PaletteSelection::Effect(EffectType::Delay) => (
            ri::REPEAT_FILL,
            "Delay",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Reverb) => (
            ri::SURROUND_SOUND_FILL,
            "Reverb",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Distortion) => (
            ri::FIRE_FILL,
            "Distortion",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Chorus) => (
            ri::SPEED_FILL,
            "Chorus",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Flanger) => (
            ri::TORNADO_FILL,
            "Flanger",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Phaser) => (
            ri::LOOP_LEFT_FILL,
            "Phaser",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Compressor) => (
            ri::DASHBOARD_FILL,
            "Compressor",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Eq) => (
            ri::EQUALIZER_2_FILL,
            "EQ",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Waveshaper) => (
            ri::PULSE_FILL,
            "Waveshaper",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::BbdDelay) => (
            ri::DISC_FILL,
            "BBD Delay",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Limiter) => (
            ri::STOP_FILL,
            "Limiter",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::MidSide) => (
            ri::ARROW_LEFT_RIGHT_FILL,
            "Mid/Side",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::Convolver) => (
            ri::SOUND_MODULE_FILL,
            "Convolver",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::PhaseVocoder) => (
            ri::MICROSCOPE_FILL,
            "Phase Vocoder",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::FrequencyShifter) => (
            ri::SHUFFLE_FILL,
            "Freq Shifter",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::EnsembleChorus) => (
            ri::SPEED_FILL,
            "Ensemble Chorus",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::ShimmerReverb) => (
            ri::SPARKLING_2_FILL,
            "Shimmer Reverb",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::GranularFx) => (
            ri::RHYTHM_FILL,
            "Granular FX",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::SpectralBlur) => (
            ri::BLUR_OFF_FILL,
            "Spectral Blur",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::ModalResonator) => (
            ri::SOUND_MODULE_FILL,
            "Modal Resonator",
            category_color(ModuleCategory::Effect),
        ),
        PaletteSelection::Effect(EffectType::ReverseGateReverb) => (
            ri::REWIND_FILL,
            "Reverse/Gate",
            category_color(ModuleCategory::Effect),
        ),
        // Visualizers
        PaletteSelection::Visualizer(PaletteVisualizerType::Oscilloscope) => (
            ri::PULSE_LINE,
            "Oscilloscope",
            category_color(ModuleCategory::Visualizer),
        ),
        PaletteSelection::Visualizer(PaletteVisualizerType::LevelMeter) => (
            ri::BAR_CHART_2_FILL,
            "Level Meter",
            category_color(ModuleCategory::Visualizer),
        ),
        PaletteSelection::Visualizer(PaletteVisualizerType::SpectrumAnalyzer) => (
            ri::SPECTRUM_FILL,
            "Spectrum",
            category_color(ModuleCategory::Visualizer),
        ),
        PaletteSelection::SignalMonitor => (
            ri::SEARCH_LINE,
            "Signal Monitor",
            category_color(ModuleCategory::Visualizer),
        ),
        // Modulation
        PaletteSelection::RingMod => (
            ri::SWAP_FILL,
            "Ring Mod",
            category_color(ModuleCategory::Utility),
        ),
        PaletteSelection::EnvelopeFollower => (
            ri::LINE_CHART_FILL,
            "Env Follower",
            category_color(ModuleCategory::Utility),
        ),
        PaletteSelection::Mseg => (
            ri::NODE_TREE,
            "MSEG",
            category_color(ModuleCategory::Utility),
        ),
        PaletteSelection::KineticModulator => (
            ri::SPEED_UP_FILL,
            "Kinetic Mod",
            category_color(ModuleCategory::Utility),
        ),
        // Generative
        PaletteSelection::Euclidean => (
            ri::RECORD_CIRCLE_FILL,
            "Euclidean",
            category_color(ModuleCategory::LFO),
        ),
        PaletteSelection::TuringMachine => (
            ri::SHUFFLE_FILL,
            "Turing Machine",
            category_color(ModuleCategory::LFO),
        ),
        PaletteSelection::RandomGates => (
            ri::MAGIC_FILL,
            "Random Gates",
            category_color(ModuleCategory::LFO),
        ),
        // Physical modeling
        PaletteSelection::KeyboardPanner => (
            ri::PIANO_FILL,
            "Keyboard Panner",
            category_color(ModuleCategory::PhysicalModeling),
        ),
        PaletteSelection::BodyResonance => (
            ri::SOUND_MODULE_FILL,
            "Body Resonance",
            category_color(ModuleCategory::PhysicalModeling),
        ),
        PaletteSelection::MechanicalNoise => (
            ri::WRENCH_FILL,
            "Mechanical Noise",
            category_color(ModuleCategory::PhysicalModeling),
        ),
        // Output / Utility
        PaletteSelection::StereoOutput => (
            ri::SPEAKER_FILL,
            "Output",
            category_color(ModuleCategory::Output),
        ),
        PaletteSelection::ModMatrix => (
            ri::NODE_TREE,
            "Mod Matrix",
            category_color(ModuleCategory::Utility),
        ),
        // Fallback for any other category
        PaletteSelection::Category(cat) => (ri::SOUND_MODULE_FILL, "Module", category_color(cat)),
    };
    (format!("{icon} {text}"), color)
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
