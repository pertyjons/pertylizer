//! Rack view - the main workspace for arranging modules.
//!
//! This module handles the canvas where modules are placed,
//! connections are drawn, and modules can be added/removed.
//!
//! Modules are rendered as draggable, resizable windows with z-order support.
//! Cables are rendered behind modules; hovered cables pop to the foreground.

use eframe::egui::{self, Color32, Id, LayerId, Order, Pos2, Rect, Sense, Ui, Vec2};
use egui_remixicon::icons as ri;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use synth_core::{
    DestAddr, MacroSource, ModuleCategory, ModuleDescriptor, PortDirection, PortName, PortType,
    SrcAddr,
};
use synth_core::{ModMatrixParam, ModuleType, Param};
use synth_engine::graph::Connection;
use synth_engine::{EngineHandle, ModuleId};

use crate::audio::input::InputState;
use crate::patch::{
    ConnectionState, ExposedPortState, GroupCategory, GroupId, GroupTemplate, HexColor,
    ModuleGroupState, ModuleState, ParamValue, Position,
};

use super::module_panel::{ModulePanelState, PortPosition, category_color};
use super::theme::theme;
use super::widgets::{
    CABLE_SPREAD, ModRole, ModuleFrame, WidgetPortDirection, WidgetPortType, cable_color,
    closest_point_on_cable, draw_cable, draw_cable_dragging, draw_cable_highlighted,
    draw_flow_particles, draw_module_header, point_near_cable,
};

/// Grid cell size in pixels. Used for grid drawing and snap-to-grid.
pub(crate) const GRID_SIZE: f32 = 32.0;
const GROUP_HEADER_HEIGHT: f32 = 24.0;
const GROUP_PORT_MARGIN: f32 = 12.0;
const GROUP_PADDING: f32 = 16.0;

// --- Patch editor palette ---
/// Status: source module / routed-to-output (green "ok").
const STATUS_OK_GREEN: Color32 = Color32::from_rgb(100, 200, 100);
/// Status: sink module (red).
const STATUS_SINK_RED: Color32 = Color32::from_rgb(200, 100, 100);
/// Status: automation lane targets this module (amber).
const STATUS_AUTOMATED_AMBER: Color32 = Color32::from_rgb(220, 170, 90);
/// Status: internal routing / global module (blue).
const STATUS_ROUTING_BLUE: Color32 = Color32::from_rgb(100, 180, 220);
/// Status: orphaned — has connections but signal doesn't reach output (yellow).
const STATUS_ORPHANED_YELLOW: Color32 = Color32::from_rgb(200, 200, 100);
/// Status: disconnected — no cables (gray).
const STATUS_DISCONNECTED_GRAY: Color32 = Color32::from_rgb(100, 100, 100);
/// Effect-chain accent (warm amber) — reorder buttons, badge, chain cables.
const EFFECT_CHAIN_AMBER: Color32 = Color32::from_rgb(230, 160, 50);
/// Close button when hovered (red).
const CLOSE_BUTTON_HOVER_RED: Color32 = Color32::from_rgb(255, 100, 100);
/// Close button at rest (translucent light gray).
const CLOSE_BUTTON_IDLE: Color32 = Color32::from_rgba_premultiplied(200, 200, 200, 150);
/// Background grid line color (faint blue-gray).
const GRID_LINE_COLOR: Color32 = Color32::from_rgba_unmultiplied_const(60, 65, 75, 50);

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

pub(crate) fn parse_hex_color(hex: &str) -> Option<Color32> {
    let (r, g, b, a) = crate::patch::parse_hex_rgba(hex)?;
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

pub(crate) fn color32_to_hex(color: Color32) -> HexColor {
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

/// Paint a tinted, framed, labelled background zone around a set of panels.
/// Returns silently when no panels are supplied so callers can pass a
/// filtered iterator without an outer empty-check.
fn draw_module_zone<'a>(
    ui: &mut Ui,
    scroll_rect: Rect,
    panels: impl IntoIterator<Item = &'a ModulePanelState>,
    color: Color32,
    label: &str,
) {
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    let mut found = false;

    for panel in panels {
        found = true;
        min_x = min_x.min(panel.position.x);
        min_y = min_y.min(panel.position.y);
        max_x = max_x.max(panel.position.x + panel.size.x);
        max_y = max_y.max(panel.position.y + panel.size.y);
    }

    if !found {
        return;
    }

    let padding = GRID_SIZE;
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
    painter.rect_filled(zone_rect, 4.0, color.gamma_multiply(0.06));
    painter.rect_stroke(
        zone_rect,
        4.0,
        egui::Stroke::new(1.0, color.gamma_multiply(0.15)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        zone_rect.min + egui::vec2(8.0, 4.0),
        egui::Align2::LEFT_TOP,
        label,
        egui::FontId::proportional(11.0),
        color.gamma_multiply(0.35),
    );
}

/// Patch analysis: counts module types to enable smart display names and filtering.
///
/// Built once per frame from the current panels. Used for:
/// - Numbered module titles ("LFO 1" / "LFO 2" when 2+ LFOs, "LFO" when only 1)
/// - Filtering mod matrix dropdown choices (hide "LFO 2" source if only 1 LFO exists)
/// - Mod-matrix-reference lookups for the header badge (so source/destination
///   modules show that they're wired through the matrix even with zero cables)
pub(crate) struct PatchAnalysis {
    /// How many of each module type exist.
    module_counts: HashMap<ModuleType, u16>,
    /// Modules referenced as a Mod Matrix source, with the set of source
    /// port/param `name`s per module (S1.5b) — drives the per-knob source marker
    /// when a `name` matches a parameter `type_id` (a `detune` param); output
    /// ports like `out` have no knob, so they only roll up to the header badge.
    mod_matrix_sources: HashMap<ModuleId, HashSet<String>>,
    /// Modules referenced as a Mod Matrix destination, with the set of modulated
    /// parameter `type_id`s per module (S1.5a) — drives the per-knob marker; the
    /// keys alone roll up to the module-header badge.
    mod_matrix_destinations: HashMap<ModuleId, HashSet<String>>,
    /// Macros (`velocity`, `mod_wheel`, …) used as a Mod Matrix source (S1.5b).
    /// Macros have no `ModuleId` to badge, so they get the macro-source rail.
    mod_matrix_macros: HashSet<MacroSource>,
}

impl PatchAnalysis {
    /// Build from current patch panels.
    fn from_panels(panels: &HashMap<ModuleId, ModulePanelState>) -> Self {
        let mut module_counts: HashMap<ModuleType, u16> = HashMap::new();
        for id in panels.keys() {
            *module_counts.entry(id.module_type).or_insert(0) += 1;
        }

        let mut mod_matrix_sources: HashMap<ModuleId, HashSet<String>> = HashMap::new();
        let mut mod_matrix_destinations: HashMap<ModuleId, HashSet<String>> = HashMap::new();
        let mut mod_matrix_macros: HashSet<MacroSource> = HashSet::new();

        for (id, panel) in panels {
            if id.module_type != ModuleType::ModMatrix {
                continue;
            }

            // Routings are address-based (S1.5c): resolve each slot's source/dest
            // address (mirrored in `slot_addrs`) to the real module instance it
            // names, and flag it if that module is present. The legacy f32 index in
            // `param_values` can't represent arbitrary addresses (`lfo-3.out`), so
            // reading it here would miss exactly the targets the picker enables.
            for slot in 0..synth_core::MAX_MOD_MATRIX_SLOTS as u8 {
                let enabled_name = ModMatrixParam::SlotEnabled(slot, true).name();
                let enabled = panel
                    .param_values
                    .get(enabled_name)
                    .map(|v| *v != 0.0)
                    .unwrap_or(true);
                if !enabled {
                    continue;
                }

                let source_name = ModMatrixParam::SlotSource(slot, None).name();
                if let Some(addr) = panel.slot_addrs.get(source_name) {
                    match SrcAddr::parse(addr) {
                        Some(SrcAddr::Module {
                            module_type,
                            instance,
                            name,
                        }) => {
                            let mid = ModuleId::new(module_type, instance);
                            if panels.contains_key(&mid) {
                                // Owned `String` for the same reason as destinations:
                                // `mod_role_for_param` looks up by `&str` per knob per
                                // frame, so avoid a `PortName::intern` global lock.
                                mod_matrix_sources
                                    .entry(mid)
                                    .or_default()
                                    .insert(name.as_str().to_string());
                            }
                        }
                        // A macro source has no `ModuleId` to badge — record it for the
                        // macro-source rail instead.
                        Some(SrcAddr::Macro(m)) => {
                            mod_matrix_macros.insert(m);
                        }
                        None => {}
                    }
                }

                let dest_name = ModMatrixParam::SlotDestination(slot, None).name();
                if let Some(addr) = panel.slot_addrs.get(dest_name)
                    && let Some(dst) = DestAddr::parse(addr)
                {
                    let mid = ModuleId::new(dst.module_type, dst.instance);
                    if panels.contains_key(&mid) {
                        // Store the `type_id` as an owned `String`, not the interned
                        // `PortName`: `mod_role_for_param` is called per knob per
                        // frame with a `&str` type_id, and a `PortName` set would
                        // force `PortName::intern` (a global write-lock) on every
                        // lookup. The only cost here is one alloc per routing during
                        // the already-per-frame rebuild.
                        mod_matrix_destinations
                            .entry(mid)
                            .or_default()
                            .insert(dst.param.as_str().to_string());
                    }
                }
            }
        }

        Self {
            module_counts,
            mod_matrix_sources,
            mod_matrix_destinations,
            mod_matrix_macros,
        }
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

    /// `true` if any Mod Matrix slot routes from this module.
    fn is_mod_matrix_source(&self, module_id: ModuleId) -> bool {
        self.mod_matrix_sources.contains_key(&module_id)
    }

    /// `true` if any Mod Matrix slot routes to this module.
    fn is_mod_matrix_destination(&self, module_id: ModuleId) -> bool {
        self.mod_matrix_destinations.contains_key(&module_id)
    }

    /// `true` if a Mod Matrix slot reads this macro as its source (S1.5b) —
    /// drives the macro-source rail chip.
    fn is_macro_source(&self, macro_source: MacroSource) -> bool {
        self.mod_matrix_macros.contains(&macro_source)
    }

    /// The Mod Matrix role of a specific parameter on a module, for the per-knob
    /// marker (S1.5a/b). Both directions are tracked at parameter granularity: a
    /// source `name` that is a parameter (`detune`) marks that knob; a destination
    /// param marks its knob; a param that is both gets the `Both` glyph.
    /// `param_type_id` is the descriptor `type_id`.
    fn mod_role_for_param(&self, module_id: ModuleId, param_type_id: &str) -> Option<ModRole> {
        let is_source = self
            .mod_matrix_sources
            .get(&module_id)
            .is_some_and(|params| params.contains(param_type_id));
        let is_dest = self
            .mod_matrix_destinations
            .get(&module_id)
            .is_some_and(|params| params.contains(param_type_id));
        ModRole::from_flags(is_source, is_dest)
    }
}

/// A single addressing target for the Mod Matrix pickers (S1.5c).
struct ModAddrTarget {
    id: ModuleId,
    /// Display label, e.g. "Filter 1".
    label: String,
    /// Modulatable parameters as `(type_id, display label)` — destination picks.
    dest_params: Vec<(String, String)>,
    /// Output port names — source picks.
    source_ports: Vec<String>,
}

/// Addressing targets available to the Mod Matrix source/destination pickers.
///
/// Built once per frame from the patch's cached module descriptors. Where the
/// legacy 19-choice enum combos reached only ~2 roles per module, this catalog
/// lets a routing address **any** module's output port (source) or modulatable
/// parameter (destination), plus the six per-voice macros (S1.5c).
struct ModAddrCatalog {
    /// Targets sorted by module-type prefix then instance, for stable menus.
    modules: Vec<ModAddrTarget>,
}

impl ModAddrCatalog {
    /// Build from the patch editor's cached descriptors. The Mod Matrix itself is
    /// excluded — it is neither a modulation source nor a destination.
    fn from_descriptors(descriptors: &HashMap<ModuleId, ModuleDescriptor>) -> Self {
        let mut modules: Vec<ModAddrTarget> = descriptors
            .iter()
            .filter(|(id, _)| id.module_type != ModuleType::ModMatrix)
            .map(|(id, desc)| ModAddrTarget {
                id: *id,
                label: format!("{} {}", desc.name, id.instance),
                dest_params: desc
                    .parameters
                    .iter()
                    .filter(|p| p.is_automatable())
                    .map(|p| (p.type_id.clone(), p.name.clone()))
                    .collect(),
                source_ports: desc
                    .ports
                    .iter()
                    .filter(|p| p.direction == PortDirection::Output)
                    .map(|p| p.name.to_string())
                    .collect(),
            })
            .collect();
        modules.sort_by(|a, b| {
            (a.id.module_type.prefix(), a.id.instance)
                .cmp(&(b.id.module_type.prefix(), b.id.instance))
        });
        Self { modules }
    }

    /// Look up a target by module type + 1-based instance.
    fn target(&self, module_type: ModuleType, instance: u16) -> Option<&ModAddrTarget> {
        self.modules
            .iter()
            .find(|t| t.id.module_type == module_type && t.id.instance == instance)
    }

    /// Friendly label for a stored source address (`"lfo-1.out"` → "LFO 1 · out"),
    /// falling back to the raw address for a dangling / unknown reference.
    fn source_label(&self, addr: &str) -> String {
        match SrcAddr::parse(addr) {
            Some(SrcAddr::Macro(m)) => macro_label(m).to_string(),
            Some(SrcAddr::Module {
                module_type,
                instance,
                name,
            }) => {
                let base = self.target(module_type, instance).map_or_else(
                    || format!("{}-{instance}", module_type.prefix()),
                    |t| t.label.clone(),
                );
                format!("{base} · {}", name.as_str())
            }
            None => addr.to_string(),
        }
    }

    /// Friendly label for a stored destination address (`"flt-1.cutoff"` →
    /// "Filter 1 · Cutoff"), falling back to the raw address.
    fn dest_label(&self, addr: &str) -> String {
        let Some(d) = DestAddr::parse(addr) else {
            return addr.to_string();
        };
        let target = self.target(d.module_type, d.instance);
        let base = target.map_or_else(
            || format!("{}-{}", d.module_type.prefix(), d.instance),
            |t| t.label.clone(),
        );
        let param = target
            .and_then(|t| {
                t.dest_params
                    .iter()
                    .find(|(tid, _)| tid.as_str() == d.param.as_str())
            })
            .map_or_else(|| d.param.as_str().to_string(), |(_, name)| name.clone());
        format!("{base} · {param}")
    }
}

/// Display label for a modulation macro source.
fn macro_label(m: MacroSource) -> &'static str {
    match m {
        MacroSource::Velocity => "Velocity",
        MacroSource::NoteNumber => "Note",
        MacroSource::Aftertouch => "Aftertouch",
        MacroSource::ModWheel => "Mod Wheel",
        MacroSource::PitchBend => "Pitch Bend",
        MacroSource::PolyAftertouch => "Poly Aftertouch",
    }
}

/// Insert or remove a Mod Matrix slot-address mirror entry (S1.5c): `Some` stores
/// the canonical address, `None` drops the slot back to unconfigured.
fn sync_slot_addr(slot_addrs: &mut HashMap<String, String>, name: &str, addr: Option<String>) {
    match addr {
        Some(s) => {
            slot_addrs.insert(name.to_string(), s);
        }
        None => {
            slot_addrs.remove(name);
        }
    }
}

/// Render a Mod Matrix **source** picker. Returns `Some(selection)` only when the
/// user changed it (`None` inner value = cleared to no source).
fn mod_source_picker(
    ui: &mut Ui,
    id_salt: String,
    width: f32,
    current: Option<&str>,
    catalog: &ModAddrCatalog,
) -> Option<Option<SrcAddr>> {
    let mut result: Option<Option<SrcAddr>> = None;
    let text = current.map_or_else(|| "(none)".to_string(), |s| catalog.source_label(s));
    // Parse `current` once and compare structurally, instead of formatting every
    // candidate's address string per item per frame.
    let current_addr = current.and_then(SrcAddr::parse);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(text)
        .width(width)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "(none)").clicked() {
                result = Some(None);
            }
            for m in MacroSource::ALL {
                let addr = SrcAddr::Macro(m);
                if ui
                    .selectable_label(current_addr == Some(addr), macro_label(m))
                    .clicked()
                {
                    result = Some(Some(addr));
                }
            }
            for target in &catalog.modules {
                for port in &target.source_ports {
                    let addr = SrcAddr::module(target.id.module_type, target.id.instance, port);
                    let label = format!("{} · {port}", target.label);
                    if ui
                        .selectable_label(current_addr == Some(addr), label)
                        .clicked()
                    {
                        result = Some(Some(addr));
                    }
                }
            }
        });
    result
}

/// Render a Mod Matrix **destination** picker. Returns `Some(selection)` only when
/// the user changed it (`None` inner value = cleared to no destination).
fn mod_dest_picker(
    ui: &mut Ui,
    id_salt: String,
    width: f32,
    current: Option<&str>,
    catalog: &ModAddrCatalog,
) -> Option<Option<DestAddr>> {
    let mut result: Option<Option<DestAddr>> = None;
    let text = current.map_or_else(|| "(none)".to_string(), |s| catalog.dest_label(s));
    // Parse `current` once and compare structurally (see `mod_source_picker`).
    let current_addr = current.and_then(DestAddr::parse);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(text)
        .width(width)
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "(none)").clicked() {
                result = Some(None);
            }
            for target in &catalog.modules {
                for (type_id, label) in &target.dest_params {
                    let addr = DestAddr::new(target.id.module_type, target.id.instance, type_id);
                    let item = format!("{} · {label}", target.label);
                    if ui
                        .selectable_label(current_addr == Some(addr), item)
                        .clicked()
                    {
                        result = Some(Some(addr));
                    }
                }
            }
        });
    result
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
    /// Cached list of available samples (id, name) for sampler module dropdowns.
    sample_list: Vec<(u64, String)>,
    /// Cached effect chain order from previous frame (for change detection).
    prev_effect_chain_order: Vec<ModuleId>,
    /// Cached set of cable-less Mod Matrix attachment modules from the
    /// previous frame (for change detection). When this set changes,
    /// attachments are auto-stacked beneath the matrix module so they
    /// fall inside the Mod Matrix framed zone.
    prev_mod_matrix_attachments: Vec<ModuleId>,
    /// Cached connected ports per module (rebuilt when connections change).
    connected_ports_cache: HashMap<ModuleId, Vec<PortName>>,
    /// When true, skip reading positions back from egui Area state.
    /// Set after loading a patch/project so saved positions aren't overwritten
    /// by stale egui Area rects during the same frame.
    suppress_position_readback: bool,
    /// Modules that the in-progress cable drag must NOT connect to because the
    /// edge would form a cycle. Recomputed once per frame from a single graph
    /// traversal (see `recompute_drag_cycle_blocked`); empty when not dragging.
    drag_cycle_blocked: HashSet<ModuleId>,
    /// Open "Edit description" popup, if any (at most one at a time). Carries
    /// the target module and the in-progress draft until OK/Cancel.
    description_editor: Option<DescriptionEditorState>,
    /// Module whose read-only info popup (ⓘ) is open, if any.
    info_popup: Option<ModuleId>,
    /// Per-frame screen rect of each module's `Area`, captured during the
    /// render loop so the description / info popups can be positioned next to
    /// (not over) their module.
    module_rects: HashMap<ModuleId, egui::Rect>,
}

/// Transient state for the open "Edit description" popup. `draft` is the
/// in-progress text, independent of the installed description until applied.
#[derive(Clone)]
struct DescriptionEditorState {
    module_id: ModuleId,
    draft: String,
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
            sample_list: Vec::new(),
            suppress_position_readback: false,
            prev_effect_chain_order: Vec::new(),
            prev_mod_matrix_attachments: Vec::new(),
            connected_ports_cache: HashMap::new(),
            drag_cycle_blocked: HashSet::new(),
            description_editor: None,
            info_popup: None,
            module_rects: HashMap::new(),
        }
    }

    /// Update the cached sample list for sampler module dropdowns.
    pub fn set_sample_list(&mut self, list: Vec<(u64, String)>) {
        self.sample_list = list;
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
        self.connected_ports_cache.clear();
        self.suppress_position_readback = true;
        self.description_editor = None;
        self.info_popup = None;
        self.module_rects.clear();
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
        self.module_rects.remove(&id);
        // Close any description/info popup that targeted this module.
        if self.info_popup == Some(id) {
            self.info_popup = None;
        }
        if self
            .description_editor
            .as_ref()
            .is_some_and(|e| e.module_id == id)
        {
            self.description_editor = None;
        }
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
                    description: String::new(),
                    parameters: param_map,
                    scripts: std::collections::BTreeMap::new(),
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
        let relevant: Vec<Connection> = self
            .connections
            .iter()
            .filter(|c| c.from_module == module_id || c.to_module == module_id)
            .copied()
            .collect();
        for conn in &relevant {
            self.ensure_exposed_for_connection(conn);
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

    /// Move a collapsed group's box to `new_position` and shift every member
    /// panel by the same delta so the group's internal relative layout is
    /// preserved when the user expands it again.
    fn move_collapsed_group(&mut self, group_id: GroupId, new_position: Pos2) {
        let Some(group) = self.groups.get_mut(&group_id) else {
            return;
        };
        let delta = new_position - group.position;
        if delta == Vec2::ZERO {
            return;
        }
        group.position = new_position;
        let members = group.members.clone();
        for mid in members {
            if let Some(panel) = self.panels.get_mut(&mid) {
                panel.position += delta;
                self.needs_reposition.insert(mid);
            }
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

    /// Mirror engine parameter values into the panel's cache, skipping
    /// entries already at the engine value. Reconciling an engine→GUI
    /// round-trip of a mid-drag knob must not clobber the user's value,
    /// so equal values are a no-op.
    pub fn sync_module_params(&mut self, module_id: ModuleId, params: &[Param]) {
        let Some(panel) = self.panels.get_mut(&module_id) else {
            return;
        };
        for param in params {
            let name = param.name();

            // Mod Matrix routings are address-based; mirror the full address into
            // `slot_addrs` (the lossy f32 mirror below can only hold the legacy
            // enum index, so arbitrary addresses would vanish from the picker).
            match param {
                Param::ModMatrix(ModMatrixParam::SlotSource(_, src)) => {
                    sync_slot_addr(
                        &mut panel.slot_addrs,
                        name,
                        src.map(|a| a.to_address_string()),
                    );
                }
                Param::ModMatrix(ModMatrixParam::SlotDestination(_, dst)) => {
                    sync_slot_addr(
                        &mut panel.slot_addrs,
                        name,
                        dst.map(|a| a.to_address_string()),
                    );
                }
                _ => {}
            }

            let new_value = param.as_f32();
            match panel.param_values.get(name) {
                Some(current) if (*current - new_value).abs() <= f32::EPSILON => continue,
                _ => {
                    panel.param_values.insert(name.to_string(), new_value);
                }
            }
        }
    }

    /// Mirror a Mod Matrix module's installed control scripts (S2.4) into its
    /// panel so the expression editor and ƒx markers reflect them. `scripts` is
    /// the engine snapshot's map keyed by **1-based** slot string; the panel mirror
    /// (`slot_scripts`) keys by 0-based slot index. Clear-fill: a slot dropped from
    /// the snapshot (script cleared) vanishes from the mirror. The snapshot is the
    /// source of truth — `SetModScript` bumps the shared-graph version, so the
    /// version-gated reconcile re-runs with the script already published.
    pub fn sync_module_scripts(&mut self, module_id: ModuleId, scripts: &BTreeMap<String, String>) {
        let Some(panel) = self.panels.get_mut(&module_id) else {
            return;
        };
        panel.slot_scripts.clear();
        for (slot_key, source) in scripts {
            if let Ok(one_based) = slot_key.parse::<u8>()
                && one_based >= 1
            {
                panel.slot_scripts.insert(one_based - 1, source.clone());
            }
        }
    }

    /// Mirror a module's per-instance description from the engine snapshot into
    /// its panel so the info popup and the "Edit description" editor seed from
    /// the live value. Snapshot is the source of truth (`SetModuleDescription`
    /// bumps the shared-graph version, re-running the reconcile).
    pub fn sync_module_description(&mut self, module_id: ModuleId, description: &str) {
        if let Some(panel) = self.panels.get_mut(&module_id)
            && panel.description != description
        {
            panel.description = description.to_string();
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
        audio_input_snapshot: &AudioInputSnapshot,
        // Modules referenced by a sequencer automation lane (this instrument);
        // drawn with an "automated" header badge. Empty if none.
        automated_modules: &HashSet<ModuleId>,
    ) -> PatchEditorResult {
        let mut result = PatchEditorResult::default();

        // Precompute which modules the active cable drag may not target (cycle
        // guard), once per frame, so per-port highlighting is a cheap lookup.
        self.recompute_drag_cycle_blocked();

        self.realign_effect_chain_if_changed(effect_chain_order);
        let analysis = PatchAnalysis::from_panels(&self.panels);
        self.realign_mod_matrix_attachments_if_changed(&analysis);

        // Addressing targets for the Mod Matrix pickers (S1.5c). Built here, before
        // the `self.panels.get_mut` borrow below, so the picker can address every
        // other module without re-borrowing `self`. Skip the (allocating) build
        // entirely when the patch has no Mod Matrix — the catalog has no consumer.
        let has_mod_matrix = self
            .descriptors
            .keys()
            .any(|id| id.module_type == ModuleType::ModMatrix);
        let mod_catalog = if has_mod_matrix {
            ModAddrCatalog::from_descriptors(&self.descriptors)
        } else {
            ModAddrCatalog {
                modules: Vec::new(),
            }
        };

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
                self.draw_mod_matrix_zone(ui, scroll_rect, &analysis);

                // Save scroll area layer + clip rect for cable drawing (behind modules)
                scroll_layer_id = Some(ui.layer_id());
                scroll_clip_rect = Some(ui.clip_rect());

                scroll_rect
            });

        let scroll_offset = scroll_output.state.offset;
        let area_origin = visible_rect.min.to_vec2();

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
                ui.memory(|mem| mem.area_rect(wid))
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

        // Collect data before mutable iteration
        let module_ids: Vec<_> = self.z_order.clone();

        // Track which module to bring to front
        let mut bring_to_front: Option<ModuleId> = None;

        // Temporarily take descriptors out of self to allow immutable access
        // while self is mutably borrowed in the loop body.
        let descriptors = std::mem::take(&mut self.descriptors);

        // Draw modules as windows (in z-order)
        for module_id in &module_ids {
            let module_id = *module_id;
            if group_layout.hidden_modules.contains(&module_id) {
                continue;
            }
            let connected_ports = self
                .connected_ports_cache
                .get(&module_id)
                .cloned()
                .unwrap_or_default();

            let descriptor = match descriptors.get(&module_id) {
                Some(d) => d,
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

            // Dim modules that aren't connected to output, or are bypassed.
            // Matrix-referenced modules count as connected (routed via slot
            // instead of cable).
            let opacity = if is_bypassed {
                0.4
            } else if is_global_module
                || analysis.is_mod_matrix_source(module_id)
                || analysis.is_mod_matrix_destination(module_id)
            {
                1.0
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
            let frame = ModuleFrame::new(dimmed_accent)
                .selected(is_selected)
                .opacity(opacity)
                .build(&ui.global_style());

            // Check if this module needs repositioning (after auto-layout)
            let needs_reposition = self.needs_reposition.contains(&module_id);

            let title = analysis.display_name(module_id, &descriptor.name);

            // Position in screen coordinates based on logical position and scroll offset
            let screen_pos = panel_position + area_origin - scroll_offset;

            // Use Area + Frame instead of Window so modules render at Order::Background
            // (same layer as panels). The keyboard panel renders at Order::Middle
            // to ensure it always has input priority over module Areas.
            // `constrain_to(visible_rect)` also clips the Area's interact_rect
            // (`state.rect().intersect(constrain_rect)` in egui's Area::begin) so
            // modules drawn under surrounding panels don't steal hover/clicks
            // from them. `constrain(false)` keeps positions unclamped so
            // logical-x=0 modules still scroll under the panel visually.
            let area = egui::Area::new(window_id)
                .order(Order::Background)
                .movable(true)
                .constrain_to(visible_rect)
                .constrain(false)
                .current_pos(screen_pos);

            // Get processing info for this module
            let is_source = self.is_source(module_id);
            let is_sink = self.is_sink(module_id);
            let is_automated = automated_modules.contains(&module_id);

            let is_inline_monitor = descriptor.type_id.0 == "inline_signal_monitor";

            let area_response = area.show(ui.ctx(), |ui| {
                // Inline Signal Monitor: compact 100×50px with just oscilloscope + close button
                if is_inline_monitor {
                    self.draw_inline_monitor(
                        ui,
                        module_id,
                        descriptor,
                        dimmed_accent,
                        handle,
                        &mut result,
                    );
                    return;
                }

                frame.show(ui, |ui| {
                    // Title bar: name + status icons + close button (single row)
                    draw_module_header(
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
                                            .color(STATUS_OK_GREEN)
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
                                            .color(STATUS_SINK_RED)
                                            .size(10.0),
                                    )
                                    .frame(false)
                                    .min_size(Vec2::new(14.0, 20.0)),
                                )
                                .on_hover_text("Sink Module\nConsumes signal (no outgoing connections).");
                            }

                            // Automation indicator (referenced by a sequencer
                            // automation lane).
                            if is_automated {
                                ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(ri::PULSE_FILL)
                                            .color(STATUS_AUTOMATED_AMBER)
                                            .size(10.0),
                                    )
                                    .frame(false)
                                    .min_size(Vec2::new(14.0, 20.0)),
                                )
                                .on_hover_text("Automated\nA sequencer automation lane targets this module.");
                            }

                            // Connectivity status indicator
                            let (conn_icon, conn_color, conn_tooltip): (_, _, &str) = if is_global_module {
                                if descriptor.category == ModuleCategory::Utility {
                                    (ri::FLASHLIGHT_FILL, STATUS_ROUTING_BLUE, "Internal Routing\nRoutes modulation internally — no cables needed.")
                                } else {
                                    (ri::FLASHLIGHT_FILL, STATUS_ROUTING_BLUE, "Global Module\nProcessed automatically via effect chain.")
                                }
                            } else {
                                match connectivity_status {
                                    ModuleConnectivity::Connected => (
                                        ri::LINK,
                                        STATUS_OK_GREEN,
                                        "Routed to Output\nAudio from this module reaches the output.",
                                    ),
                                    ModuleConnectivity::Orphaned => (
                                        ri::ERROR_WARNING_LINE,
                                        STATUS_ORPHANED_YELLOW,
                                        "Orphaned\nHas connections but signal doesn't reach output.\nConnect to a module that leads to Output.",
                                    ),
                                    ModuleConnectivity::Disconnected => (
                                        ri::LINK_UNLINK,
                                        STATUS_DISCONNECTED_GRAY,
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

                            let is_matrix_source = analysis.is_mod_matrix_source(module_id);
                            let is_matrix_destination =
                                analysis.is_mod_matrix_destination(module_id);
                            if let Some(badge_role) =
                                ModRole::from_flags(is_matrix_source, is_matrix_destination)
                            {
                                let badge_color = t.colors.accent_purple;
                                // Share the icon mapping with the per-knob marker so
                                // the module roll-up and its parameters never show
                                // conflicting arrows; the tooltips stay module-level.
                                let badge_icon = badge_role.glyph();
                                let badge_tip = match (is_matrix_source, is_matrix_destination) {
                                    (true, true) => "Mod Matrix\nRouted as both source and destination via the Mod Matrix.",
                                    (true, false) => "Mod Matrix Source\nThis module drives one or more Mod Matrix slots.\nLook in the Mod Matrix module for slot details.",
                                    (false, true) => "Mod Matrix Destination\nA Mod Matrix slot modulates a parameter on this module.\nLook in the Mod Matrix module for slot details.",
                                    _ => "",
                                };
                                ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(badge_icon)
                                            .color(badge_color)
                                            .size(12.0),
                                    )
                                    .frame(false)
                                    .min_size(button_min_size),
                                )
                                .on_hover_text(badge_tip);
                            }

                            // Divider: everything to the left is a status
                            // indicator (not clickable); everything to the right
                            // (power, chain reorder, info, menu, close) is an
                            // interactive control.
                            ui.separator();

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
                                let chain_color = EFFECT_CHAIN_AMBER;

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

                            // Info (ⓘ) — toggles a read-only popup with the
                            // module's type documentation + this instance's note.
                            let info_open = self.info_popup == Some(module_id);
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(ri::INFORMATION_LINE)
                                            .color(if info_open {
                                                t.colors.text_secondary
                                            } else {
                                                t.colors.text_dim
                                            })
                                            .size(13.0),
                                    )
                                    .frame(false)
                                    .min_size(button_min_size),
                                )
                                .on_hover_text(
                                    "Module info\nType documentation + this instance's note.",
                                )
                                .clicked()
                            {
                                self.info_popup = if info_open { None } else { Some(module_id) };
                            }

                            // Overflow menu (⋯) — per-module actions. Currently
                            // just "Edit description"; built to grow. Uses a
                            // frameless custom button so it matches the other
                            // header icons (the default `menu_button` is boxed).
                            let mut open_desc_editor = false;
                            egui::containers::menu::MenuButton::from_button(
                                egui::Button::new(
                                    egui::RichText::new(ri::MORE_FILL)
                                        .color(t.colors.text_dim)
                                        .size(14.0),
                                )
                                .frame(false)
                                .min_size(button_min_size),
                            )
                            .ui(ui, |ui| {
                                if ui
                                    .button(format!("{}  Edit description…", ri::EDIT_LINE))
                                    .clicked()
                                {
                                    open_desc_editor = true;
                                    ui.close();
                                }
                            });
                            if open_desc_editor {
                                let draft = self
                                    .panels
                                    .get(&module_id)
                                    .map(|p| p.description.clone())
                                    .unwrap_or_default();
                                self.description_editor =
                                    Some(DescriptionEditorState { module_id, draft });
                            }

                            // Close/delete button (always visible). Grouped with
                            // the other interactive controls under the single
                            // divider above — no separate divider here.
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
                                            .color(EFFECT_CHAIN_AMBER),
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
                                descriptor,
                                accent_color,
                                vis_buffer,
                                &analysis,
                                &mod_catalog,
                                &self.sample_list,
                                audio_input_snapshot,
                            );
                            for param in panel_result.param_changes {
                                result.param_changes.push((module_id, param));
                            }
                            if panel_result.audio_input_action.is_some() {
                                result.audio_input_action = panel_result.audio_input_action;
                            }
                            for (slot, src) in panel_result.mod_script_actions {
                                result.mod_script_actions.push((module_id, slot, src));
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
                                    descriptor,
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
                                        descriptor,
                                        accent_color,
                                        vis_buffer,
                                        &analysis,
                                        &mod_catalog,
                                        &self.sample_list,
                                        audio_input_snapshot,
                                    );
                                    for param in panel_result.param_changes {
                                        result.param_changes.push((module_id, param));
                                    }
                                    if panel_result.audio_input_action.is_some() {
                                        result.audio_input_action =
                                            panel_result.audio_input_action;
                                    }
                                    for (slot, src) in panel_result.mod_script_actions {
                                        result
                                            .mod_script_actions
                                            .push((module_id, slot, src));
                                    }
                                }
                            });

                            // Right port column (OUT) - fixed width
                            ui.vertical(|ui| {
                                ui.set_width(col_w);
                                self.draw_port_column(
                                    ui,
                                    module_id,
                                    descriptor,
                                    WidgetPortDirection::Output,
                                    &connected_ports,
                                );
                            });
                        });
                    }
                });
            });

            // Remember the module's on-screen rect so the description / info
            // popups can be anchored beside it (not over it).
            if let Some(area_rect) = ui.memory(|mem| mem.area_rect(window_id)) {
                self.module_rects.insert(module_id, area_rect);
            }

            // Handle area interaction — Area always returns InnerResponse (no Option)
            // Update logical position and size from screen rect, snapped to grid.
            // Skip during the frame a patch/project was just loaded — egui's Area rects
            // are stale and would overwrite the freshly loaded saved positions.
            if !self.suppress_position_readback
                && let Some(area_rect) = ui.memory(|mem| mem.area_rect(window_id))
                && let Some(panel_state) = self.panels.get_mut(&module_id)
            {
                let logical_pos = area_rect.min - area_origin + scroll_offset;
                panel_state.position = snap_to_grid(logical_pos);
                panel_state.size = area_rect.size();
            }

            self.handle_module_interaction(
                ui,
                &area_response.response,
                module_id,
                &group_layout,
                &mut bring_to_front,
            );

            // Handle close (delete module) — triggered by close button.
            // If the module is in a signal chain, bypass it (reconnect around it).
            if !open {
                self.bypass_and_remove(module_id, &mut result);
            }

            // Clear reposition flag after this module has been drawn
            if needs_reposition {
                self.needs_reposition.remove(&module_id);
            }
        }

        // Restore descriptors after the render loop
        self.descriptors = descriptors;

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

        self.handle_canvas_background_input(ui, &canvas_response, area_origin, scroll_offset);

        // Macro-source rail (S1.5b): a fixed strip of macro chips above the
        // scrolling canvas. Only meaningful when a Mod Matrix can read them.
        if has_mod_matrix {
            self.draw_macro_source_rail(ui, instrument_id, visible_rect, &analysis);
        }

        // Module description / info popups (drawn last so they float above the
        // canvas; descriptors are already restored above so type docs resolve).
        self.draw_module_popups(ui, &mut result);

        // Clear suppress flag — next frame will resume normal position tracking
        self.suppress_position_readback = false;

        result
    }

    /// Draw the read-only info popup (ⓘ) and the "Edit description" editor when
    /// open. Both are floating windows anchored beside their module (via the
    /// per-frame `module_rects`); the editor pushes its result through
    /// `module_description_actions` on OK, the info popup is read-only.
    fn draw_module_popups(&mut self, ui: &mut Ui, result: &mut PatchEditorResult) {
        let ctx = ui.ctx().clone();
        let t = theme();

        // Read-only info popup: module id, type name + type documentation, and
        // this instance's note.
        if let Some(mid) = self.info_popup {
            let anchor = self
                .module_rects
                .get(&mid)
                .map(|r| r.right_top() + egui::vec2(12.0, 0.0));
            let (type_name, type_desc) = self
                .descriptors
                .get(&mid)
                .map(|d| (d.name.to_string(), d.description.to_string()))
                .unwrap_or_default();
            let instance_desc = self
                .panels
                .get(&mid)
                .map(|p| p.description.clone())
                .unwrap_or_default();
            let mut keep_open = true;
            let mut win = egui::Window::new(format!("{}  {mid}", ri::INFORMATION_LINE))
                .id(egui::Id::new(("module_info_popup", mid)))
                .collapsible(false)
                .resizable(false)
                .open(&mut keep_open);
            if let Some(pos) = anchor {
                win = win.default_pos(pos);
            }
            win.show(&ctx, |ui| {
                ui.label(
                    egui::RichText::new(format!("Type: {type_name}"))
                        .strong()
                        .color(t.colors.text_secondary),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new("Type documentation")
                        .size(t.fonts.size_small)
                        .color(t.colors.text_dim),
                );
                ui.label(if type_desc.is_empty() {
                    "(none)".to_string()
                } else {
                    type_desc
                });
                ui.separator();
                ui.label(
                    egui::RichText::new("Instance note")
                        .size(t.fonts.size_small)
                        .color(t.colors.text_dim),
                );
                ui.label(if instance_desc.is_empty() {
                    "(no description set)".to_string()
                } else {
                    instance_desc
                });
            });
            if !keep_open {
                self.info_popup = None;
            }
        }

        // "Edit description" editor: a small popup near the module with OK /
        // Cancel, styled like the expression editor.
        if let Some(mut editor) = self.description_editor.take() {
            let mid = editor.module_id;
            let anchor = self
                .module_rects
                .get(&mid)
                .map(|r| r.right_top() + egui::vec2(12.0, 0.0));
            let mut keep_open = true;
            let mut closed = false;
            let mut win = egui::Window::new(format!("{}  Edit description - {mid}", ri::EDIT_LINE))
                .id(egui::Id::new(("module_desc_editor", mid)))
                .collapsible(false)
                .resizable(true)
                .default_size(egui::vec2(360.0, 160.0))
                .min_width(260.0)
                .min_height(120.0)
                .open(&mut keep_open);
            if let Some(pos) = anchor {
                win = win.default_pos(pos);
            }
            win.show(&ctx, |ui| {
                ui.label(
                    egui::RichText::new("Per-instance note — what this specific module is for.")
                        .size(t.fonts.size_small)
                        .color(t.colors.text_secondary),
                );
                let reserved = 40.0;
                let editor_height = (ui.available_height() - reserved).max(60.0);
                ui.add_sized(
                    egui::vec2(ui.available_width(), editor_height),
                    egui::TextEdit::multiline(&mut editor.draft).desired_rows(3),
                );
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        result
                            .module_description_actions
                            .push((mid, editor.draft.clone()));
                        closed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        closed = true;
                    }
                });
            });
            // Persist the in-progress draft across frames unless dismissed.
            if keep_open && !closed {
                self.description_editor = Some(editor);
            }
        }
    }

    /// Draw the macro-source rail (S1.5b): a fixed strip of the six per-voice
    /// macro sources, each chip wearing the same purple source marker as a module
    /// header when a Mod Matrix slot reads it. Macros have no `ModuleId` to badge,
    /// so this rail is their home in the source topology. Anchored to the patch
    /// canvas's top-left, in the foreground above the scrolling module Areas.
    fn draw_macro_source_rail(
        &self,
        ui: &Ui,
        instrument_id: u64,
        visible_rect: Rect,
        analysis: &PatchAnalysis,
    ) {
        let t = theme();
        let inset = Vec2::splat(t.spacing.panel_padding);
        egui::Area::new(egui::Id::new(("macro_source_rail", instrument_id)))
            .order(Order::Foreground)
            .fixed_pos(visible_rect.min + inset)
            .constrain_to(visible_rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(t.colors.bg_panel)
                    .stroke(egui::Stroke::new(1.0, t.colors.border))
                    .corner_radius(4.0)
                    .inner_margin(t.spacing.widget_spacing)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new("Macros")
                                    .size(t.fonts.size_small)
                                    .color(t.colors.text_dim),
                            );
                            for m in MacroSource::ALL {
                                let active = analysis.is_macro_source(m);
                                let color = if active {
                                    t.colors.accent_purple
                                } else {
                                    t.colors.text_dim
                                };
                                let tip = if active {
                                    format!(
                                        "{}\nDrives one or more Mod Matrix slots.",
                                        macro_label(m)
                                    )
                                } else {
                                    format!("{}\nAvailable as a Mod Matrix source.", macro_label(m))
                                };
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    ui.label(
                                        egui::RichText::new(macro_label(m))
                                            .size(t.fonts.size_small)
                                            .color(color),
                                    );
                                    // Same Source glyph as the module-header badge,
                                    // shown only when the macro is actually wired.
                                    if active {
                                        ui.label(
                                            egui::RichText::new(ModRole::Source.glyph())
                                                .size(t.fonts.size_small)
                                                .color(t.colors.accent_purple),
                                        );
                                    }
                                })
                                .response
                                .on_hover_text(tip);
                            }
                        });
                    });
            });
    }

    /// Draw a vertical column of ports (input or output side).
    /// Handle a module Area's interaction: click/drag selects it (shift/ctrl
    /// toggles multi-select) and brings it to front; drag-stop re-parents it to
    /// the group under its center.
    fn handle_module_interaction(
        &mut self,
        ui: &mut Ui,
        response: &egui::Response,
        module_id: ModuleId,
        group_layout: &GroupLayout,
        bring_to_front: &mut Option<ModuleId>,
    ) {
        // Bring to front on click
        if response.clicked() || response.drag_started() {
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
            *bring_to_front = Some(module_id);
        }

        if response.drag_stopped()
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
    }

    /// Handle clicks on empty canvas: left-click deselects, right-click opens
    /// the background/cable context menu, and Escape cancels a pending
    /// connection and closes any open context menus.
    fn handle_canvas_background_input(
        &mut self,
        ui: &mut Ui,
        canvas_response: &Option<egui::Response>,
        area_origin: Vec2,
        scroll_offset: Vec2,
    ) {
        // Handle click on empty space to deselect
        if let Some(response) = canvas_response
            && response.clicked()
        {
            self.selected_module = None;
            self.selected_modules.clear();
            self.selected_group = None;
        }

        // Right-click on background (or cable) → capture state for context menu
        if let Some(response) = canvas_response
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
    }

    /// Render an inline signal-monitor module: a compact 100×50 oscilloscope
    /// with tiny IN/OUT ports and a close button. Registers port positions for
    /// cables and starts pending connections on port drag.
    fn draw_inline_monitor(
        &mut self,
        ui: &mut Ui,
        module_id: ModuleId,
        descriptor: &ModuleDescriptor,
        dimmed_accent: Color32,
        handle: &EngineHandle,
        result: &mut PatchEditorResult,
    ) {
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
                            let port_rect = egui::Rect::from_center_size(center, Vec2::splat(8.0));
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
                    let samples = trim_sweep_to_complete_cycles(&samples, 0.0);

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
                            let port_rect = egui::Rect::from_center_size(center, Vec2::splat(8.0));
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
                CLOSE_BUTTON_HOVER_RED
            } else {
                CLOSE_BUTTON_IDLE
            };
            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                ri::CLOSE_LINE,
                egui::FontId::proportional(10.0),
                close_color,
            );

            if close_resp.clicked() {
                self.bypass_and_remove(module_id, result);
            }
        });

        // Request continuous repaint so the waveform updates
        ui.request_repaint();
    }

    fn draw_port_column_with<F>(
        ui: &mut Ui,
        direction: WidgetPortDirection,
        ports: &[PortRenderInfo],
        pending_info: Option<(ModuleId, WidgetPortType, WidgetPortDirection)>,
        cycle_blocked: &HashSet<ModuleId>,
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

                let rail_x = ui.cursor().min.x + col_width * 0.5;
                let rail_top = ui.cursor().min.y + 3.0;
                let rail_bottom = rail_top + ports.len() as f32 * spacing - 6.0;
                ui.painter().line_segment(
                    [Pos2::new(rail_x, rail_top), Pos2::new(rail_x, rail_bottom)],
                    egui::Stroke::new(1.0, t.colors.border.gamma_multiply(0.55)),
                );
            }

            for port in ports {
                let is_highlighted = pending_info
                    .map(|(from_module, from_type, from_dir)| {
                        // Signal always flows output → input; pick whichever side
                        // is the output so directional compatibility is correct.
                        let (out_type, in_type) = if from_dir == WidgetPortDirection::Output {
                            (from_type, port.port_type)
                        } else {
                            (port.port_type, from_type)
                        };
                        from_module != port.module_id
                            && from_dir != direction
                            && out_type.can_drive(in_type)
                            && !cycle_blocked.contains(&port.module_id)
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
        // Cycle-blocked targets were computed once for this frame; the highlight
        // just looks each module up.
        let cycle_blocked = &self.drag_cycle_blocked;
        let port_positions = &mut self.port_positions;
        Self::draw_port_column_with(
            ui,
            direction,
            &ports,
            pending_info,
            cycle_blocked,
            |port, center| {
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
            },
        );
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
        // Group columns expose ports from several member modules; the shared
        // per-frame set already covers them all.
        let cycle_blocked = &self.drag_cycle_blocked;
        Self::draw_port_column_with(
            ui,
            direction,
            &ports,
            pending_info,
            cycle_blocked,
            |port, center| {
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
            },
        );
    }

    fn draw_grid(&self, ui: &mut Ui, rect: Rect) {
        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, 0.0, theme().colors.bg_dark);

        // Grid lines
        let grid_size = GRID_SIZE;

        let grid_color = GRID_LINE_COLOR;

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
        let panels = self.panels.iter().filter_map(|(id, p)| {
            let category = self.descriptors.get(id).map(|d| d.category);
            matches!(
                category,
                Some(ModuleCategory::Effect | ModuleCategory::Visualizer)
            )
            .then_some(p)
        });
        draw_module_zone(
            ui,
            scroll_rect,
            panels,
            category_color(ModuleCategory::Effect),
            &format!("{} Effect Chain", ri::FLASHLIGHT_FILL),
        );
    }

    /// Draw a tinted framed zone behind any Mod Matrix modules. The bounding
    /// box expands to include cable-less matrix attachments (modules wired
    /// only through a matrix slot, auto-stacked beneath the matrix by
    /// `align_mod_matrix_attachments`) so they sit visibly inside the same
    /// frame. Modules that *also* have cables stay in the voice-graph flow
    /// and the frame does not engulf them.
    fn draw_mod_matrix_zone(&self, ui: &mut Ui, scroll_rect: Rect, analysis: &PatchAnalysis) {
        if analysis.count(ModuleType::ModMatrix) == 0 {
            return;
        }
        // `prev_mod_matrix_attachments` was refreshed earlier this frame by
        // `realign_mod_matrix_attachments_if_changed`, so it matches the
        // current attachment set without a second walk over all panels.
        let matrix_panels = self
            .panels
            .iter()
            .filter(|(id, _)| id.module_type == ModuleType::ModMatrix)
            .map(|(_, p)| p);
        let attachment_panels = self
            .prev_mod_matrix_attachments
            .iter()
            .filter_map(|id| self.panels.get(id));
        draw_module_zone(
            ui,
            scroll_rect,
            matrix_panels.chain(attachment_panels),
            theme().colors.accent_purple,
            &format!("{} Mod Matrix", ri::PULSE_FILL),
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
        let chain_color = EFFECT_CHAIN_AMBER;
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
                Color32::TRANSPARENT,
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
            // Same input-routing guard as module Areas: clipping the
            // interact_rect to visible_rect via constrain_to (with
            // constrain(false) so positions aren't clamped) keeps the group
            // box from stealing hover/clicks from surrounding panels when
            // it's drawn outside the patch editor.
            let area = egui::Area::new(area_id)
                .order(Order::Background)
                .movable(true)
                .constrain_to(visible_rect)
                .constrain(false)
                .current_pos(rect_screen.min);

            let mut toggle_clicked = false;
            let mut delete_clicked = false;
            let mut menu_clicked = false;
            let mut menu_pos = Pos2::ZERO;
            let response = area.show(ui.ctx(), |ui| {
                let base_color = self.group_color(&group);
                let stroke_width = if self.selected_group == Some(group_id) {
                    2.0
                } else {
                    1.0
                };
                let frame = egui::Frame::window(&ui.global_style())
                    .fill(ui.global_style().visuals.window_fill())
                    .stroke(egui::Stroke::new(
                        stroke_width,
                        base_color.gamma_multiply(0.6),
                    ))
                    .corner_radius(6.0);
                frame.show(ui, |ui| {
                    let button_min_size = Vec2::new(20.0, 20.0);
                    let t = theme();
                    draw_module_header(
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

            if !self.suppress_position_readback
                && let Some(area_rect) = ui.memory(|mem| mem.area_rect(area_id))
            {
                let logical_pos = area_rect.min - area_origin + scroll_offset;
                self.move_collapsed_group(group_id, snap_to_grid(logical_pos));
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

            // Data-driven "Add module" menu: one submenu per category, populated
            // from the module catalog (which is built from `ALL_MODULE_TYPES`).
            // Adding a `ModuleType` variant and listing it in `ALL_MODULE_TYPES`
            // makes it appear here automatically — there is no hand-maintained
            // palette to drift out of sync with the enum. Every `ModuleCategory`
            // is listed below, so no category can be silently dropped either.
            const CATEGORY_ORDER: &[(ModuleCategory, &str)] = &[
                (ModuleCategory::Oscillator, "Oscillator"),
                (ModuleCategory::Filter, "Filter"),
                (ModuleCategory::Envelope, "Envelope"),
                (ModuleCategory::LFO, "LFO"),
                (ModuleCategory::Amplifier, "Amplifier"),
                (ModuleCategory::Mixer, "Mixer"),
                (ModuleCategory::Effect, "Effect"),
                (ModuleCategory::Sampler, "Sampler"),
                (ModuleCategory::Utility, "Modulation / Utility"),
                (ModuleCategory::Sequencer, "Generative"),
                (ModuleCategory::PhysicalModeling, "Physical"),
                (ModuleCategory::Visualizer, "Visualizer"),
                (ModuleCategory::Output, "Output"),
            ];
            for &(category, title) in CATEGORY_ORDER {
                // Skip a category with no modules rather than show an empty
                // submenu. (The closure below only runs when the submenu opens,
                // so presence must be checked up front.)
                if !module_catalog().iter().any(|(_, cat, _)| *cat == category) {
                    continue;
                }
                ui.menu_button(
                    egui::RichText::new(format!("{} {title}", category_icon(category)))
                        .color(category_color(category)),
                    |ui| {
                        for &(mt, cat, _) in module_catalog() {
                            if cat == category {
                                Self::bg_menu_item(ui, PaletteSelection::Module(mt), &mut selected);
                            }
                        }
                    },
                );
            }
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
                                    PaletteSelection::Sampler,
                                    PaletteSelection::AudioInput,
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

        // Port types must be compatible in the direction the signal flows
        // (output → input). Mirrors the engine's compatibility matrix so the
        // GUI never rejects a connection the engine would accept.
        let (out_type, in_type) = if pending.from_direction == WidgetPortDirection::Output {
            (pending.from_type, target.port_type)
        } else {
            (target.port_type, pending.from_type)
        };
        if !out_type.can_drive(in_type) {
            return false;
        }

        // Reject connections that would form a cycle — the engine silently drops
        // them (see Graph::would_create_cycle), so the GUI must not offer them.
        let (out_module, in_module) = if pending.from_direction == WidgetPortDirection::Output {
            (pending.from_module, target.module_id)
        } else {
            (target.module_id, pending.from_module)
        };
        if self.would_create_cycle(out_module, in_module) {
            return false;
        }

        true
    }

    /// Whether adding an edge `from → to` would create a cycle in the current
    /// connection graph. Mirrors `synth_engine::graph::Graph::would_create_cycle`
    /// (the engine silently rejects such edges). Used by `can_connect` at
    /// drop time; the per-frame highlight uses the precomputed
    /// `drag_cycle_blocked` set instead, which encodes the same rule in bulk.
    fn would_create_cycle(&self, from: ModuleId, to: ModuleId) -> bool {
        if from == to {
            return true; // Self-loop
        }

        let mut visited = HashSet::new();
        let mut stack = vec![to];
        while let Some(current) = stack.pop() {
            if current == from {
                return true; // Path to → from exists; the new edge closes a loop
            }
            if visited.insert(current) {
                for conn in &self.connections {
                    if conn.from_module == current {
                        stack.push(conn.to_module);
                    }
                }
            }
        }
        false
    }

    /// Recompute the set of modules the in-progress drag must not connect to
    /// because the edge would close a cycle. One graph traversal per frame
    /// (reachability from the drag's source) replaces the previous per-port,
    /// per-frame `would_create_cycle` DFS. Empty when no drag is active.
    ///
    /// Dragging from an *output* of `S` would add `S → m`, so any `m` that can
    /// already reach `S` (an ancestor of `S`) is blocked — walk edges backward.
    /// Dragging from an *input* of `S` would add `m → S`, so any `m` reachable
    /// from `S` (a descendant) is blocked — walk edges forward. `S` itself is
    /// always blocked (self-loop).
    fn recompute_drag_cycle_blocked(&mut self) {
        self.drag_cycle_blocked.clear();
        let Some(pending) = self.pending_connection.as_ref() else {
            return;
        };
        let source = pending.from_module;
        let walk_forward = pending.from_direction == WidgetPortDirection::Input;

        self.drag_cycle_blocked.insert(source);
        let mut stack = vec![source];
        while let Some(current) = stack.pop() {
            for conn in &self.connections {
                let neighbor = if walk_forward {
                    (conn.from_module == current).then_some(conn.to_module)
                } else {
                    (conn.to_module == current).then_some(conn.from_module)
                };
                if let Some(next) = neighbor
                    && self.drag_cycle_blocked.insert(next)
                {
                    stack.push(next);
                }
            }
        }
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
                    description: String::new(),
                    parameters,
                    scripts: std::collections::BTreeMap::new(),
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

    /// Treat `order` as the current baseline so the next
    /// `realign_effect_chain_if_changed()` call with the same order is a no-op.
    /// Used after loading a patch so manually placed effect positions survive.
    pub fn mark_effect_chain_aligned(&mut self, order: Vec<ModuleId>) {
        self.prev_effect_chain_order = order;
    }

    pub fn realign_effect_chain_if_changed(&mut self, effect_chain_order: &[ModuleId]) {
        if effect_chain_order == self.prev_effect_chain_order.as_slice() {
            return;
        }
        // Only realign on a true reorder — same set of effects in a new
        // sequence. Set-level differences (add/remove or partial snapshot
        // catching up after load) just sync the baseline without
        // repositioning, so panels keep their saved/auto-laid positions.
        let same_set = effect_chain_order.len() == self.prev_effect_chain_order.len()
            && effect_chain_order
                .iter()
                .all(|id| self.prev_effect_chain_order.contains(id));
        if same_set {
            self.align_effect_chain(effect_chain_order);
        }
        self.prev_effect_chain_order.clear();
        self.prev_effect_chain_order
            .extend_from_slice(effect_chain_order);
    }

    /// Cable-less voice-graph modules whose only routing is through a Mod
    /// Matrix slot (source or destination). Sorted by `ModuleId` for stable
    /// diffing against `prev_mod_matrix_attachments`.
    fn collect_mod_matrix_attachments(&self, analysis: &PatchAnalysis) -> Vec<ModuleId> {
        let mut attachments: Vec<ModuleId> = self
            .panels
            .keys()
            .filter(|id| {
                if id.module_type == ModuleType::ModMatrix {
                    return false;
                }
                let referenced =
                    analysis.is_mod_matrix_source(**id) || analysis.is_mod_matrix_destination(**id);
                if !referenced {
                    return false;
                }
                self.connected_ports_cache
                    .get(*id)
                    .map(|s| s.is_empty())
                    .unwrap_or(true)
            })
            .copied()
            .collect();
        attachments.sort();
        attachments
    }

    /// Treat the current attachments set as baseline so the next
    /// `realign_mod_matrix_attachments_if_changed()` call is a no-op.
    /// Use after loading a project so saved positions survive.
    pub fn mark_mod_matrix_attachments_aligned(&mut self) {
        let analysis = PatchAnalysis::from_panels(&self.panels);
        self.prev_mod_matrix_attachments = self.collect_mod_matrix_attachments(&analysis);
    }

    /// If the set of cable-less matrix-referenced modules changed (slot
    /// edited, module added/removed), stack them vertically below the
    /// anchor matrix module so the Mod Matrix framed zone naturally
    /// encloses them.
    fn realign_mod_matrix_attachments_if_changed(&mut self, analysis: &PatchAnalysis) {
        let current = self.collect_mod_matrix_attachments(analysis);
        if current == self.prev_mod_matrix_attachments {
            return;
        }
        self.align_mod_matrix_attachments(&current);
        self.prev_mod_matrix_attachments = current;
    }

    /// Stack matrix attachments vertically beneath the first Mod Matrix
    /// module on the canvas. No-op when no matrix exists or no
    /// attachments resolved (e.g., every referenced module also has audio
    /// cables and is therefore exempted by `collect_mod_matrix_attachments`).
    fn align_mod_matrix_attachments(&mut self, attachments: &[ModuleId]) {
        if attachments.is_empty() {
            return;
        }
        let anchor = self
            .panels
            .iter()
            .find(|(id, _)| id.module_type == ModuleType::ModMatrix)
            .map(|(_, p)| (p.position, p.size));
        let Some((anchor_pos, anchor_size)) = anchor else {
            return;
        };

        let gap = GRID_SIZE;
        let mut y = anchor_pos.y + anchor_size.y + gap;
        for &id in attachments {
            if let Some(panel) = self.panels.get_mut(&id) {
                let x = anchor_pos.x;
                panel.position = snap_to_grid(Pos2::new(x, y));
                y = panel.position.y + panel.size.y + gap;
                self.needs_reposition.insert(id);
            }
        }
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

        // Rebuild connected ports cache
        self.connected_ports_cache.clear();
        for conn in &self.connections {
            self.connected_ports_cache
                .entry(conn.from_module)
                .or_default()
                .push(conn.from_port);
            self.connected_ports_cache
                .entry(conn.to_module)
                .or_default()
                .push(conn.to_port);
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
    /// Module positions are placed in the canvas's logical coordinate system
    /// starting at `(GRID, GRID)`; the surrounding `ScrollArea` grows around
    /// the result via `content_size()`, so no screen-space rect is needed.
    ///
    /// Collapsed groups are treated as single layout nodes: the group's
    /// collapsed box gets a position from the layout, and member panels
    /// shift in lockstep so their internal relative layout is preserved.
    pub fn apply_auto_layout(&mut self, effect_chain_order: &[ModuleId]) {
        use super::auto_layout::{
            CollapsedGroupNode, LayoutConnection, ModuleInfo, calculate_layout_with_chain_order,
            prepare_layout_inputs,
        };

        // Clear saved canvas size hint — auto-layout determines the new bounds
        self.min_canvas_size = None;

        // Collect raw module info from every visible panel.
        let raw_modules: Vec<ModuleInfo> = self
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

        // Build a CollapsedGroupNode per collapsed group; track the
        // representative -> GroupId mapping so we can distribute the
        // representative's resulting position back to its members.
        let mut collapsed_nodes: Vec<CollapsedGroupNode> = Vec::new();
        let mut repr_to_group: HashMap<ModuleId, GroupId> = HashMap::new();
        for group in self.groups.values() {
            if !group.collapsed {
                continue;
            }
            let Some(&repr_id) = group
                .members
                .iter()
                .find(|mid| self.panels.contains_key(mid))
            else {
                continue;
            };
            let Some(descriptor) = self.descriptors.get(&repr_id) else {
                continue;
            };
            let category = descriptor.category;
            let members: Vec<ModuleId> = group
                .members
                .iter()
                .copied()
                .filter(|m| self.panels.contains_key(m))
                .collect();
            collapsed_nodes.push(CollapsedGroupNode {
                representative: repr_id,
                category,
                size: collapsed_group_size(group),
                members,
            });
            repr_to_group.insert(repr_id, group.id);
        }

        let raw_connections: Vec<LayoutConnection> = self
            .connections
            .iter()
            .map(|c| LayoutConnection {
                from_module: c.from_module,
                to_module: c.to_module,
            })
            .collect();

        let (modules, connections) =
            prepare_layout_inputs(&raw_modules, &raw_connections, &collapsed_nodes);

        let result = calculate_layout_with_chain_order(&modules, &connections, effect_chain_order);

        // Auto-layout is now self-contained: the result already places
        // effect-chain modules, so `align_effect_chain()` is no longer
        // run as a post-pass — it is retained only for explicit
        // user-triggered effect-chain reorders.
        for (module_id, position) in result.positions {
            if let Some(&group_id) = repr_to_group.get(&module_id) {
                self.move_collapsed_group(group_id, position);
            } else if let Some(panel) = self.panels.get_mut(&module_id) {
                panel.position = position;
                self.needs_reposition.insert(module_id);
            }
        }

        // Force the frame-loop realign to fire even when the attachment
        // set hasn't changed — auto-layout just scattered modulators into
        // the Modulation zone by category, but matrix attachments belong
        // beneath the matrix.
        self.prev_mod_matrix_attachments.clear();
        let analysis = PatchAnalysis::from_panels(&self.panels);
        self.realign_mod_matrix_attachments_if_changed(&analysis);
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
    /// Audio input action (monitoring/recording toggle from patch module).
    pub audio_input_action: Option<AudioInputAction>,
    /// Mod Matrix expression-editor actions (S2.4): `(module_id, 0-based slot,
    /// source)`. `Some(source)` installs/replaces a YAMS control script on the
    /// slot; `None` clears it. Routed to `session.set_mod_script` /
    /// `clear_mod_script` by the backend.
    pub mod_script_actions: Vec<(ModuleId, u8, Option<String>)>,
    /// Per-module-instance description edits from the "Edit description" popup:
    /// `(module_id, new_description)`. Empty string clears. Routed to
    /// `session.set_module_description` by the backend.
    pub module_description_actions: Vec<(ModuleId, String)>,
}

/// Actions from the Audio Input module panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioInputAction {
    StartMonitoring,
    StopMonitoring,
    StartRecording,
    StopRecording,
}

/// Snapshot of audio input state for rendering in the patch editor.
#[derive(Debug, Clone, Copy)]
pub struct AudioInputSnapshot {
    pub state: InputState,
    pub peak_level: f32,
    pub recorded_seconds: f64,
}

impl Default for AudioInputSnapshot {
    fn default() -> Self {
        Self {
            state: InputState::Idle,
            peak_level: 0.0,
            recorded_seconds: 0.0,
        }
    }
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
    /// Audio input action (if this is an AudioInput module).
    pub audio_input_action: Option<AudioInputAction>,
    /// Mod Matrix expression-editor actions (S2.4): `(0-based slot, source)`.
    /// `Some(source)` installs/replaces a YAMS control script; `None` clears the
    /// slot back to its scalar `Amount`. The caller pairs each with the module id
    /// and routes to `session.set_mod_script` / `clear_mod_script`.
    pub mod_script_actions: Vec<(u8, Option<String>)>,
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
            buffer.read_samples_into(&mut state.vis_buf_l, &mut state.vis_buf_r);
            // Downsample to 256 points for display
            let step = state.vis_buf_l.len().max(1) / 256;
            if step > 0 {
                state
                    .vis_buf_l
                    .iter()
                    .copied()
                    .step_by(step.max(1))
                    .take(256)
                    .collect()
            } else {
                state.vis_buf_l.clone()
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
        let demo_spectrum;
        let magnitudes: &[f32] = if let Some(buffer) = vis_buffer {
            buffer.read_samples_into(&mut state.vis_buf_l, &mut state.vis_buf_r);
            &state.vis_buf_l
        } else {
            // Demo flat spectrum if no buffer connected
            demo_spectrum = vec![0.0; 256];
            &demo_spectrum
        };

        // Use available width, with min/max constraints
        let width = ui.available_width().clamp(120.0, 300.0);
        let height = (width * 0.5).clamp(60.0, 120.0);

        super::widgets::draw_spectrum_analyzer(
            ui,
            magnitudes,
            width,
            height,
            gain,
            theme().colors.accent_green,
            48000.0,
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
#[allow(clippy::too_many_arguments)]
fn draw_module_panel_params(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    accent_color: Color32,
    vis_buffer: Option<&synth_engine::visualizers::VisualizationBuffer>,
    analysis: &PatchAnalysis,
    mod_catalog: &ModAddrCatalog,
    sample_list: &[(u64, String)],
    audio_input_snapshot: &AudioInputSnapshot,
) -> PanelParamsResult {
    use super::widgets::EnvelopeEditor;
    use synth_core::WidgetHint;

    let mut param_changes = Vec::new();
    let mut audio_input_action = None;
    // Per-knob Mod Matrix marker (S1.5a/b): a parameter is marked when a routing
    // targets this module's matching `type_id` (destination) or reads it as a
    // source param — the three-state direction comes from `mod_role_for_param`.
    let module_id = state.id;
    let mod_role =
        |p: &synth_core::ParameterDescriptor| analysis.mod_role_for_param(module_id, &p.type_id);

    // For Visualizer modules, draw visualization FIRST (before parameters)
    if descriptor.category == ModuleCategory::Visualizer {
        draw_visualizer_display(ui, state, descriptor, vis_buffer, &mut param_changes);
        // Skip regular parameter drawing for visualizers - the display is the main UI
        return PanelParamsResult {
            param_changes,
            audio_input_action: None,
            mod_script_actions: Vec::new(),
        };
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
        ui.add_space(theme().spacing.xs);
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

        ui.add_space(theme().spacing.xs);

        // Only show knob parameters for Envelope (Vel Sens, curves etc)
        let knob_params: Vec<_> = descriptor
            .parameters
            .iter()
            .filter(|p| matches!(p.widget_hint, WidgetHint::Knob))
            .collect();

        if !knob_params.is_empty() {
            let changes = super::widgets::draw_knobs(
                ui,
                &knob_params,
                accent_color,
                |p| {
                    state
                        .param_values
                        .get(&p.name)
                        .copied()
                        .unwrap_or(p.range.default)
                },
                mod_role,
            );
            for (param, value) in changes {
                state.param_values.insert(param.name.clone(), value);
                param_changes.push(param.id.with_f32(value));
            }
        }

        return PanelParamsResult {
            param_changes,
            audio_input_action: None,
            mod_script_actions: Vec::new(),
        };
    }

    // Special handling for Audio Input — monitoring and recording controls
    if descriptor.type_id.0 == "audio_input" {
        let t = theme();
        let input_state = audio_input_snapshot.state;
        let is_monitoring = input_state != InputState::Idle;
        let is_recording = input_state == InputState::Recording;

        // Monitor toggle button
        ui.add_space(t.spacing.xs);
        let monitor_icon = if is_monitoring {
            ri::MIC_FILL
        } else {
            ri::MIC_LINE
        };
        let monitor_color = if is_monitoring {
            t.colors.meter_green
        } else {
            t.colors.text_dim
        };
        if ui
            .button(
                egui::RichText::new(format!("{monitor_icon} Monitor"))
                    .color(monitor_color)
                    .size(11.0),
            )
            .clicked()
        {
            audio_input_action = Some(if is_monitoring {
                AudioInputAction::StopMonitoring
            } else {
                AudioInputAction::StartMonitoring
            });
        }

        // Record button (only enabled when monitoring)
        let rec_icon = if is_recording {
            ri::STOP_FILL
        } else {
            ri::RECORD_CIRCLE_FILL
        };
        let rec_color = if is_recording {
            t.colors.meter_red
        } else if is_monitoring {
            t.colors.text_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .add_enabled(
                is_monitoring,
                egui::Button::new(
                    egui::RichText::new(format!("{rec_icon} Rec"))
                        .color(rec_color)
                        .size(11.0),
                ),
            )
            .clicked()
        {
            audio_input_action = Some(if is_recording {
                AudioInputAction::StopRecording
            } else {
                AudioInputAction::StartRecording
            });
        }

        // Peak level meter
        if is_monitoring {
            let peak = audio_input_snapshot.peak_level;
            let bar_width = ui.available_width().min(120.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(bar_width, 6.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 2.0, t.colors.bg_dark);
            let fill_w = rect.width() * peak.clamp(0.0, 1.0);
            let fill_color = if peak > 0.9 {
                t.colors.meter_red
            } else if peak > 0.6 {
                t.colors.meter_yellow
            } else {
                t.colors.meter_green
            };
            painter.rect_filled(
                Rect::from_min_size(rect.min, Vec2::new(fill_w, rect.height())),
                2.0,
                fill_color,
            );
            ui.request_repaint();
        }

        // Recording timer
        if is_recording {
            let secs = audio_input_snapshot.recorded_seconds;
            let mins = (secs / 60.0) as u32;
            let remaining = secs - f64::from(mins) * 60.0;
            ui.label(
                egui::RichText::new(format!("{mins}:{remaining:04.1}"))
                    .color(t.colors.meter_red)
                    .monospace()
                    .size(10.0),
            );
        }

        ui.add_space(t.spacing.xs);
    }

    // Special handling for Sampler — sample selector dropdown
    if descriptor.type_id.0 == "sampler" && !sample_list.is_empty() {
        let current_id = state.param_values.get("Sample").copied().unwrap_or(0.0) as u64;
        let current_name = sample_list
            .iter()
            .find(|(id, _)| *id == current_id)
            .map(|(_, name)| name.as_str())
            .unwrap_or("(none)");

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Sample:").color(accent_color).small());
            egui::ComboBox::from_id_salt(format!("sampler_sample_select_{:?}", state.id))
                .selected_text(current_name)
                .width(120.0)
                .show_ui(ui, |ui| {
                    for &(id, ref name) in sample_list {
                        if ui.selectable_label(id == current_id, name).clicked() {
                            state.param_values.insert("Sample".to_string(), id as f32);
                            param_changes.push(synth_core::Param::sample_select(id));
                        }
                    }
                });
        });
        ui.add_space(theme().spacing.xs);
    }

    // Special handling for Mod Matrix — custom grid rendering
    if descriptor.type_id.0 == "mod_matrix" {
        return draw_mod_matrix_grid(ui, state, descriptor, accent_color, mod_catalog);
    }

    // Special handling for the Script module — a list of YAMS slots (one per
    // output port), each opening the shared expression editor.
    if descriptor.type_id.0 == "script" {
        return draw_script_module_grid(ui, state, accent_color);
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

    // Generic descriptor-driven parameter widgets, shared with the mixer's
    // return-bus inserts (see `widgets::draw_parameter_grid`). The patch editor
    // caches values per module and hides mod-matrix targets that aren't wired
    // up; those two concerns are the getter and choice filter, and the shared
    // renderer draws the rest.
    let changes = super::widgets::draw_parameter_grid(
        ui,
        descriptor,
        accent_color,
        |p| {
            state
                .param_values
                .get(&p.name)
                .copied()
                .unwrap_or(p.range.default)
        },
        // The Mod Matrix has its own picker path (early return above); every other
        // module's choices are always shown.
        |_p, _choice| true,
        mod_role,
    );
    for (param, value) in changes {
        state.param_values.insert(param.name.clone(), value);
        param_changes.push(param.id.with_f32(value));
    }

    PanelParamsResult {
        param_changes,
        audio_input_action,
        mod_script_actions: Vec::new(),
    }
}

/// Draw mod matrix as a grid with size selector.
#[allow(clippy::too_many_lines)]
fn draw_mod_matrix_grid(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    accent_color: Color32,
    catalog: &ModAddrCatalog,
) -> PanelParamsResult {
    use super::widgets::Knob;

    let mut param_changes = Vec::new();
    // Expression-editor actions collected this frame (S2.4). Applied by the
    // caller via `session.set_mod_script` / `clear_mod_script`.
    let mut mod_script_actions: Vec<(u8, Option<String>)> = Vec::new();
    // A slot whose ƒx button was clicked this frame — opens its editor popup.
    let mut open_editor_for: Option<u8> = None;

    // The grid selector is gone — the routing list is presented dynamically.
    // Show every *configured* routing (source or destination set) as a row, plus
    // one trailing empty row to add the next one. A slot is configured when it
    // carries a source or destination **address** (`slot_addrs`, S1.5c) — the
    // legacy f32 index can't represent arbitrary addresses. State lives in the 16
    // fixed slots; this is purely a derived presentation (no extra session state).
    let slot_addr = |slot_idx: usize, source: bool| -> Option<String> {
        let pid = if source {
            ModMatrixParam::SlotSource(slot_idx as u8, None)
        } else {
            ModMatrixParam::SlotDestination(slot_idx as u8, None)
        };
        state.slot_addrs.get(pid.name()).cloned()
    };
    let configured: Vec<usize> = (0..synth_core::MAX_MOD_MATRIX_SLOTS)
        .filter(|&i| slot_addr(i, true).is_some() || slot_addr(i, false).is_some())
        .collect();
    let first_free: Option<usize> =
        (0..synth_core::MAX_MOD_MATRIX_SLOTS).find(|&i| !configured.contains(&i));
    // (the `slot_addr` borrow of `state` ends here, before the mutable loop)

    // Rows to render: configured routings, then one empty "add" row.
    let mut rows: Vec<(usize, bool)> = configured.iter().map(|&i| (i, false)).collect();
    if let Some(free) = first_free {
        rows.push((free, true));
    }

    ui.add_space(theme().spacing.xs);

    // Fixed width — avoid ui.available_width() which is unbounded in auto-sized Areas
    let slot_width: f32 = 220.0;
    let combo_width = (slot_width - 20.0).max(60.0);

    // Slot to clear (its ✕ was clicked); applied after the render loop.
    let mut clear_idx: Option<usize> = None;

    ui.vertical(|ui| {
            for (slot_idx, is_add_row) in rows {
                let slot_num = slot_idx + 1;

                // Amount + enabled are still plain f32 params.
                let amt_param = descriptor.parameters.iter().find(|p| {
                    matches!(p.id, Param::ModMatrix(ModMatrixParam::SlotAmount(s, _)) if s as usize == slot_idx)
                });
                let en_param = descriptor.parameters.iter().find(|p| {
                    matches!(p.id, Param::ModMatrix(ModMatrixParam::SlotEnabled(s, _)) if s as usize == slot_idx)
                });

                // Source/destination are address-based (S1.5c). Read the stored
                // address strings (owned, so the borrow ends before the picker
                // writes back below) to drive the frame colour and the pickers.
                let src_name = ModMatrixParam::SlotSource(slot_idx as u8, None).name();
                let dst_name = ModMatrixParam::SlotDestination(slot_idx as u8, None).name();
                let src_addr = state.slot_addrs.get(src_name).cloned();
                let dst_addr = state.slot_addrs.get(dst_name).cloned();
                // Scripted slots (S2.4) override the scalar Amount with a YAMS
                // expression; read it (owned) here so the borrow ends before the row.
                let is_scripted = state.slot_scripts.contains_key(&(slot_idx as u8));
                let amount_val = amt_param.and_then(|p| {
                    state.param_values.get(&p.name).copied()
                        .or(Some(p.range.default))
                }).unwrap_or(0.0);
                let enabled = en_param
                    .and_then(|p| state.param_values.get(&p.name).copied().or(Some(p.range.default)))
                    .map(|v| v > 0.5)
                    .unwrap_or(true);

                let has_source = src_addr.is_some();
                let has_dest = dst_addr.is_some();
                // A scripted slot supplies its own sources (via the expression), so
                // it routes with only a destination. A scalar slot needs both ends.
                let fully_configured = has_dest && (has_source || is_scripted);

                // Slot state determines the frame stroke + the row tint:
                //  - Active (enabled + both ends): purple accent, full opacity
                //  - Disabled (enabled=false): dim grey frame, dimmed text
                //  - Half-configured (enabled but src or dst is None): orange
                //    warning frame so the user sees the slot won't route.
                let (frame_stroke_color, content_tint) = if !enabled {
                    (theme().colors.text_dim, theme().colors.text_dim)
                } else if !fully_configured {
                    (theme().colors.accent_orange, theme().colors.text_secondary)
                } else {
                    (theme().colors.accent_purple, theme().colors.text_primary)
                };

                let slot_frame = egui::Frame::group(ui.style())
                    .stroke(egui::Stroke::new(1.0, frame_stroke_color.gamma_multiply(0.6)))
                    .fill(frame_stroke_color.gamma_multiply(0.04));

                slot_frame.show(ui, |ui| {
                    ui.set_width(slot_width);
                    ui.vertical(|ui| {
                        // Row header: "Routing N" (or "New routing") + ✕ + state hint.
                        ui.horizontal(|ui| {
                            let header = if is_add_row {
                                "New routing".to_string()
                            } else {
                                format!("Routing {slot_num}")
                            };
                            ui.label(
                                egui::RichText::new(header)
                                    .size(theme().fonts.size_small)
                                    .color(content_tint),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Configured rows get a clear (remove) button.
                                    if !is_add_row
                                        && ui
                                            .button(
                                                egui::RichText::new(ri::DELETE_BIN_LINE)
                                                    .size(theme().fonts.size_small)
                                                    .color(theme().colors.text_secondary),
                                            )
                                            .on_hover_text("Remove this routing")
                                            .clicked()
                                    {
                                        clear_idx = Some(slot_idx);
                                    }
                                    if !is_add_row && (!enabled || !fully_configured) {
                                        let (icon, color, tip) = if !enabled {
                                            (
                                                ri::CLOSE_CIRCLE_LINE,
                                                theme().colors.text_dim,
                                                "Routing disabled",
                                            )
                                        } else if is_scripted {
                                            (
                                                ri::ALERT_LINE,
                                                theme().colors.accent_orange,
                                                "Expression has no effect - set a Destination",
                                            )
                                        } else {
                                            (
                                                ri::ALERT_LINE,
                                                theme().colors.accent_orange,
                                                "Routing has no effect - set both Source and Destination",
                                            )
                                        };
                                        ui.label(
                                            egui::RichText::new(icon)
                                                .size(theme().fonts.size_small)
                                                .color(color),
                                        )
                                        .on_hover_text(tip);
                                    }
                                },
                            );
                        });

                        // Source picker — any module output port or per-voice macro.
                        if let Some(sel) = mod_source_picker(
                            ui,
                            format!("mm_src_{slot_idx}"),
                            combo_width,
                            src_addr.as_deref(),
                            catalog,
                        ) {
                            sync_slot_addr(
                                &mut state.slot_addrs,
                                src_name,
                                sel.map(|a| a.to_address_string()),
                            );
                            param_changes.push(Param::ModMatrix(ModMatrixParam::SlotSource(
                                slot_idx as u8,
                                sel,
                            )));
                        }

                        // Signal-flow arrow with the modulation amount inline.
                        // Arrow direction (down + sign-marker) and colour map
                        // directly to slot state so a glance tells you whether
                        // the routing is active, polar, or inert.
                        let arrow_icon = if amount_val >= 0.0 {
                            ri::ARROW_DOWN_LINE
                        } else {
                            ri::ARROW_UP_LINE
                        };
                        let arrow_text = if is_scripted {
                            format!("{arrow_icon}  ƒx")
                        } else if amount_val.abs() > 0.001 {
                            format!("{arrow_icon}  {amount_val:+.2}")
                        } else {
                            arrow_icon.to_string()
                        };
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new(arrow_text)
                                    .size(theme().fonts.size_small)
                                    .color(frame_stroke_color),
                            );
                        });

                        // Destination picker — any modulatable param on any module.
                        if let Some(sel) = mod_dest_picker(
                            ui,
                            format!("mm_dst_{slot_idx}"),
                            combo_width,
                            dst_addr.as_deref(),
                            catalog,
                        ) {
                            sync_slot_addr(
                                &mut state.slot_addrs,
                                dst_name,
                                sel.map(|a| a.to_address_string()),
                            );
                            param_changes.push(Param::ModMatrix(ModMatrixParam::SlotDestination(
                                slot_idx as u8,
                                sel,
                            )));
                        }

                        // Amount knob + Active toggle + expression (ƒx) button.
                        ui.horizontal(|ui| {
                            if let Some(ap) = amt_param {
                                let current = amount_val;
                                let mut value = current;
                                // A scripted slot ignores the scalar Amount — show
                                // the knob disabled so it reads as overridden.
                                let knob_resp = ui
                                    .add_enabled_ui(!is_scripted, |ui| {
                                        Knob::from_descriptor(&mut value, ap)
                                            .label("Amount")
                                            .size(theme().sizes.knob_size_small)
                                            .accent_color(accent_color)
                                            .show(ui);
                                    })
                                    .response;
                                if is_scripted {
                                    knob_resp.on_hover_text(
                                        "Overridden by the expression (ƒx) - clear it to use Amount",
                                    );
                                } else if (value - current).abs() > f32::EPSILON {
                                    state.param_values.insert(ap.name.clone(), value);
                                    param_changes.push(ap.id.with_f32(value));
                                }
                            }

                            if let Some(ep) = en_param {
                                let mut is_on = enabled;
                                if ui
                                    .checkbox(&mut is_on, "Active")
                                    .on_hover_text("Enable this slot")
                                    .changed()
                                {
                                    let val = if is_on { 1.0 } else { 0.0 };
                                    state.param_values.insert(ep.name.clone(), val);
                                    param_changes.push(ep.id.with_f32(val));
                                }
                            }

                            // ƒx button - opens the expression editor for this slot.
                            // Lit purple when a script is installed (S2.4). Hidden on
                            // the trailing add-row (no slot to script yet).
                            if !is_add_row {
                                let fx_color = if is_scripted {
                                    theme().colors.accent_purple
                                } else {
                                    theme().colors.text_secondary
                                };
                                let tip = if is_scripted {
                                    "Edit the modulation expression (ƒx active)"
                                } else {
                                    "Replace Amount with a YAMS expression"
                                };
                                if ui
                                    .button(egui::RichText::new("ƒx").color(fx_color))
                                    .on_hover_text(tip)
                                    .clicked()
                                {
                                    open_editor_for = Some(slot_idx as u8);
                                }
                            }
                        });
                        ui.add_space(theme().spacing.xxs);
                    });
                });

                ui.add_space(theme().spacing.xs);
            }
        });

    // Apply a clear request: reset the slot to defaults (source/dest None,
    // amount 0, enabled) so it drops out of the configured list next frame and
    // starts fresh if the index is later reused by the add-row.
    if let Some(i) = clear_idx {
        let slot = i as u8;
        // Removing the routing also clears any installed expression and closes
        // its editor, so the slot reads as fully unconfigured next frame.
        if state.slot_scripts.contains_key(&slot) {
            mod_script_actions.push((slot, None));
        }
        if state.script_editor.as_ref().is_some_and(|e| e.slot == slot) {
            state.script_editor = None;
        }
        for (param, value) in [
            (
                Param::ModMatrix(ModMatrixParam::SlotSource(slot, None)),
                0.0,
            ),
            (
                Param::ModMatrix(ModMatrixParam::SlotDestination(slot, None)),
                0.0,
            ),
            (
                Param::ModMatrix(ModMatrixParam::SlotAmount(
                    slot,
                    synth_core::BipolarValue::CENTER,
                )),
                0.0,
            ),
            (
                Param::ModMatrix(ModMatrixParam::SlotEnabled(slot, true)),
                1.0,
            ),
        ] {
            // Drop the address mirror too, so the slot reads as unconfigured.
            if let Param::ModMatrix(
                ModMatrixParam::SlotSource(..) | ModMatrixParam::SlotDestination(..),
            ) = param
            {
                state.slot_addrs.remove(param.name());
            }
            state.param_values.insert(param.name().to_string(), value);
            param_changes.push(param);
        }
    }

    // Open the expression editor when an ƒx button was clicked, seeding the draft
    // from the slot's installed script (empty if none).
    if let Some(slot) = open_editor_for {
        let draft = state.slot_scripts.get(&slot).cloned().unwrap_or_default();
        state.script_editor = Some(super::module_panel::ScriptEditorState { slot, draft });
    }

    // Draw the expression-editor popup (S2.4) if one is open. A floating window so
    // the routing list stays compact. Compilation runs live (off the audio thread)
    // for the status line; Apply/Clear push actions the caller routes to the
    // session, which recompiles + installs the shared script.
    draw_slot_expression_editor(ui, state, &mut mod_script_actions);

    PanelParamsResult {
        param_changes,
        audio_input_action: None,
        mod_script_actions,
    }
}

/// Draw the shared per-slot YAMS expression-editor popup, reused by the Mod
/// Matrix and the Script module. Compiles live for the status line (off the
/// audio thread) and pushes `(slot, Some(src))` / `(slot, None)` actions the
/// caller routes to `session.set_mod_script` / `clear_mod_script`. No-op when no
/// slot's editor is open. The window is keyed by `state.id`, so each module's
/// editor is independent.
fn draw_slot_expression_editor(
    ui: &Ui,
    state: &mut ModulePanelState,
    mod_script_actions: &mut Vec<(u8, Option<String>)>,
) {
    let Some(mut editor) = state.script_editor.take() else {
        return;
    };
    let ctx = ui.ctx().clone();
    let mut keep_open = true;
    let mut closed_by_action = false;
    egui::Window::new(format!("Slot {} - Expression", editor.slot + 1))
        .id(egui::Id::new(("mm_expr_editor", state.id, editor.slot)))
        .collapsible(false)
        .resizable(true)
        .default_size(egui::vec2(520.0, 340.0))
        .min_width(320.0)
        .min_height(200.0)
        .open(&mut keep_open)
        .show(&ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "YAMS expression - assign `out`, e.g. `out = lfo-1.out * velocity`",
                )
                .size(theme().fonts.size_small)
                .color(theme().colors.text_secondary),
            );
            // The editor fills the window: it takes the full width and all the
            // height left after reserving room for the status line + button row,
            // so dragging the window corner grows the text area. Long scripts
            // scroll within the code editor.
            let reserved = 52.0;
            let editor_height = (ui.available_height() - reserved).max(80.0);
            ui.add_sized(
                egui::vec2(ui.available_width(), editor_height),
                egui::TextEdit::multiline(&mut editor.draft)
                    .code_editor()
                    .desired_rows(4),
            );

            // Live compile → status line (mirrors `session.set_mod_script`).
            let trimmed = editor.draft.trim();
            let status: Result<(), String> = if trimmed.is_empty() {
                Err("empty - Apply will clear the slot".to_string())
            } else {
                let (program, diags) =
                    synth_script::compile(&editor.draft, &synth_script::CompileOptions::default());
                if program.is_some() {
                    Ok(())
                } else {
                    let msg = diags
                        .iter()
                        .filter(|d| d.is_error())
                        .map(|d| d.message.clone())
                        .collect::<Vec<_>>()
                        .join("; ");
                    Err(if msg.is_empty() {
                        "compile error".to_string()
                    } else {
                        msg
                    })
                }
            };
            match &status {
                Ok(()) => {
                    ui.label(
                        egui::RichText::new(format!("{}  compiled", ri::CHECKBOX_CIRCLE_LINE))
                            .size(theme().fonts.size_small)
                            .color(theme().colors.accent_green),
                    );
                }
                Err(e) => {
                    ui.label(
                        egui::RichText::new(format!("{}  {e}", ri::ERROR_WARNING_LINE))
                            .size(theme().fonts.size_small)
                            .color(theme().colors.accent_orange),
                    );
                }
            }

            ui.horizontal(|ui| {
                // Format runs the canonical yamsfmt formatter and replaces the
                // draft with its output. Enabled only when the script is valid
                // (the formatter parses first; a broken script can't be formatted).
                if ui
                    .add_enabled(status.is_ok(), egui::Button::new("Format"))
                    .on_hover_text("Reformat the expression (yamsfmt)")
                    .clicked()
                    && let Ok(formatted) = synth_script::format(&editor.draft)
                {
                    editor.draft = formatted;
                }
                if ui
                    .add_enabled(status.is_ok(), egui::Button::new("Apply"))
                    .on_hover_text("Install this expression on the slot (keeps editing)")
                    .clicked()
                {
                    // Install but leave the popup open so the user can keep
                    // iterating (Close / ✕ dismisses it).
                    mod_script_actions.push((editor.slot, Some(editor.draft.clone())));
                }
                if ui
                    .button("Clear")
                    .on_hover_text("Remove the expression from this slot")
                    .clicked()
                {
                    mod_script_actions.push((editor.slot, None));
                    closed_by_action = true;
                }
                if ui.button("Close").clicked() {
                    closed_by_action = true;
                }
            });
        });
    // Persist the editor (with its in-progress draft) across frames unless the
    // window was closed via its ✕ or an action button.
    if keep_open && !closed_by_action {
        state.script_editor = Some(editor);
    }
}

/// A one-line, truncated preview of a slot's YAMS source for the panel row.
fn script_preview(src: &str) -> String {
    const MAX: usize = 24;
    let line = src.lines().next().unwrap_or("").trim();
    if line.chars().count() > MAX {
        let head: String = line.chars().take(MAX).collect();
        format!("{head}…")
    } else {
        line.to_string()
    }
}

/// Draw the Script module body: one row per output port (`out1`..`out8`), each
/// showing the installed YAMS source (truncated) and an ƒx button that opens the
/// shared expression editor for that slot. The output-port nipples themselves are
/// drawn by the descriptor-driven port column; this is only the editing UI.
fn draw_script_module_grid(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    accent_color: Color32,
) -> PanelParamsResult {
    let mut mod_script_actions: Vec<(u8, Option<String>)> = Vec::new();
    let mut open_editor_for: Option<u8> = None;
    let t = theme();

    for slot in 0u8..synth_modules::script_module::SCRIPT_MODULE_OUTPUTS as u8 {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("out{}", slot + 1))
                    .color(accent_color)
                    .small(),
            );
            let installed = state.slot_scripts.get(&slot);
            let (preview, color) = match installed {
                Some(src) => (script_preview(src), t.colors.text_secondary),
                None => ("— empty —".to_string(), t.colors.text_dim),
            };
            ui.label(egui::RichText::new(preview).color(color).small());

            let fx_color = if installed.is_some() {
                t.colors.accent_green
            } else {
                t.colors.text_secondary
            };
            if ui
                .button(egui::RichText::new("ƒx").color(fx_color))
                .on_hover_text("Edit this slot's YAMS expression")
                .clicked()
            {
                open_editor_for = Some(slot);
            }
        });
    }

    // Open the editor for a clicked slot, seeding the draft from the installed
    // script (empty for a fresh slot) — mirrors the Mod Matrix ƒx flow.
    if let Some(slot) = open_editor_for {
        let draft = state.slot_scripts.get(&slot).cloned().unwrap_or_default();
        state.script_editor = Some(super::module_panel::ScriptEditorState { slot, draft });
    }

    draw_slot_expression_editor(ui, state, &mut mod_script_actions);

    PanelParamsResult {
        param_changes: Vec::new(),
        audio_input_action: None,
        mod_script_actions,
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
    /// Any module type, selected directly. This is what the data-driven
    /// "Add module" menu emits — every entry in `ALL_MODULE_TYPES` is reachable
    /// through it, so a new `ModuleType` variant needs no new palette case.
    Module(ModuleType),
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
    // Scripting
    Script,
    // Sampler
    Sampler,
    AudioInput,
}

/// Cached catalog of every addable module type with its display name and
/// category, built once from [`ALL_MODULE_TYPES`](crate::module_factory::ALL_MODULE_TYPES).
///
/// This is the single source of truth for the "Add module" menu: a new
/// `ModuleType` variant listed in `ALL_MODULE_TYPES` appears automatically, so
/// the menu can never silently drift out of sync with the enum again. Built
/// lazily (constructing one throwaway instance per type) and memoized — the menu
/// renders every frame while open, so we must not rebuild it each time.
fn module_catalog() -> &'static [(ModuleType, ModuleCategory, String)] {
    static CATALOG: std::sync::OnceLock<Vec<(ModuleType, ModuleCategory, String)>> =
        std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        crate::module_factory::ALL_MODULE_TYPES
            .iter()
            .filter_map(|&mt| {
                crate::module_factory::get_descriptor(mt).map(|d| (mt, d.category, d.name))
            })
            .collect()
    })
}

/// Remix-icon glyph for a module category, used as the submenu header icon.
fn category_icon(category: ModuleCategory) -> &'static str {
    use egui_remixicon::icons as ri;
    match category {
        ModuleCategory::Oscillator => ri::MUSIC_2_FILL,
        ModuleCategory::Effect => ri::SPARKLING_FILL,
        ModuleCategory::Visualizer => ri::SPECTRUM_FILL,
        ModuleCategory::Utility => ri::SWAP_FILL,
        ModuleCategory::Sequencer => ri::MAGIC_FILL,
        ModuleCategory::PhysicalModeling => ri::PIANO_FILL,
        ModuleCategory::Sampler => ri::MUSIC_FILL,
        _ => ri::SOUND_MODULE_FILL,
    }
}

/// Get the display label (with Remix Icon) and category color for a palette selection.
fn palette_label(selection: PaletteSelection) -> (String, Color32) {
    use egui_remixicon::icons as ri;

    // Data-driven selections carry only the type; resolve name + colour from the
    // catalog rather than maintaining a per-variant arm here.
    if let PaletteSelection::Module(mt) = selection {
        let (name, category) = module_catalog()
            .iter()
            .find(|(t, _, _)| *t == mt)
            .map(|(_, c, n)| (n.clone(), *c))
            .unwrap_or_else(|| (format!("{mt:?}"), ModuleCategory::Utility));
        return (
            format!("{} {name}", category_icon(category)),
            category_color(category),
        );
    }

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
        // Sampler
        PaletteSelection::Sampler => (
            ri::MUSIC_FILL,
            "Sampler",
            category_color(ModuleCategory::Sampler),
        ),
        PaletteSelection::AudioInput => (
            ri::MIC_FILL,
            "Audio Input",
            category_color(ModuleCategory::Sampler),
        ),
        PaletteSelection::Script => (
            ri::FUNCTION_FILL,
            "Script",
            category_color(ModuleCategory::Utility),
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
        // Resolved by the early return above; this arm only satisfies the
        // exhaustiveness check.
        PaletteSelection::Module(_) => (
            ri::SOUND_MODULE_FILL,
            "Module",
            category_color(ModuleCategory::Utility),
        ),
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

#[cfg(test)]
mod patch_analysis_tests {
    use super::*;

    /// Seed a Mod Matrix panel's slot source/dest **addresses** (S1.5c), mirroring
    /// what `sync_module_params` writes from the engine. `slot_num` is 1-based.
    fn matrix_panel(slot_setups: &[(usize, &str, &str, bool)]) -> ModulePanelState {
        let mut state =
            ModulePanelState::new(ModuleId::new(ModuleType::ModMatrix, 1), Pos2::new(0.0, 0.0));
        for (slot_num, src, dst, enabled) in slot_setups {
            let slot = (*slot_num - 1) as u8;
            state.slot_addrs.insert(
                ModMatrixParam::SlotSource(slot, None).name().to_string(),
                (*src).to_string(),
            );
            state.slot_addrs.insert(
                ModMatrixParam::SlotDestination(slot, None)
                    .name()
                    .to_string(),
                (*dst).to_string(),
            );
            state.param_values.insert(
                ModMatrixParam::SlotEnabled(slot, true).name().to_string(),
                if *enabled { 1.0 } else { 0.0 },
            );
        }
        state
    }

    fn stub_panel(mt: ModuleType, instance: u16) -> (ModuleId, ModulePanelState) {
        let id = ModuleId::new(mt, instance);
        (id, ModulePanelState::new(id, Pos2::ZERO))
    }

    /// Address-based resolution: `env-6.out → flt-3.cutoff` flags exactly env-6
    /// and flt-3 (the real instances the addresses name), even at non-canonical
    /// instance numbers. Modules the addresses don't name — and the other
    /// envelope — must stay unflagged.
    #[test]
    fn analysis_marks_source_and_destination_by_address() {
        let mut panels = HashMap::new();
        panels.insert(
            ModuleId::new(ModuleType::ModMatrix, 2),
            matrix_panel(&[(1, "env-6.out", "flt-3.cutoff", true)]),
        );
        for (id, state) in [
            stub_panel(ModuleType::Envelope, 5),
            stub_panel(ModuleType::Envelope, 6),
            stub_panel(ModuleType::Filter, 3),
        ] {
            panels.insert(id, state);
        }

        let analysis = PatchAnalysis::from_panels(&panels);
        assert!(analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Envelope, 6)));
        assert!(analysis.is_mod_matrix_destination(ModuleId::new(ModuleType::Filter, 3)));
        // Per-parameter destination marker (S1.5a): only the addressed param
        // ("cutoff") is a destination on flt-3, not its other knobs.
        let flt3 = ModuleId::new(ModuleType::Filter, 3);
        assert_eq!(
            analysis.mod_role_for_param(flt3, "cutoff"),
            Some(ModRole::Destination)
        );
        assert_eq!(analysis.mod_role_for_param(flt3, "resonance"), None);
        // Instances the addresses don't name (and which aren't present) must not
        // be flagged.
        assert!(!analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Envelope, 2)));
        assert!(!analysis.is_mod_matrix_destination(ModuleId::new(ModuleType::Filter, 1)));
        // The other envelope is not the slot's source.
        assert!(!analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Envelope, 5)));
    }

    /// Disabled slots must drop out of the reference sets so toggling a
    /// slot off in the matrix UI clears the badge immediately.
    #[test]
    fn disabled_slot_clears_references() {
        let mut panels = HashMap::new();
        panels.insert(
            ModuleId::new(ModuleType::ModMatrix, 1),
            matrix_panel(&[(1, "lfo-1.out", "osc-1.pitch", false)]),
        );
        for (id, state) in [
            stub_panel(ModuleType::Lfo, 1),
            stub_panel(ModuleType::Oscillator, 1),
        ] {
            panels.insert(id, state);
        }
        let analysis = PatchAnalysis::from_panels(&panels);
        assert!(!analysis.is_mod_matrix_source(ModuleId::new(ModuleType::Lfo, 1)));
        assert!(!analysis.is_mod_matrix_destination(ModuleId::new(ModuleType::Oscillator, 1)));
    }

    /// The patch editor must refuse to highlight / allow a drag that would form
    /// a cycle, matching the engine's silent rejection. Graph: osc → amp → out.
    #[test]
    fn would_create_cycle_matches_engine() {
        let osc = ModuleId::new(ModuleType::Oscillator, 1);
        let amp = ModuleId::new(ModuleType::Amplifier, 1);
        let out = ModuleId::new(ModuleType::StereoOutput, 1);

        let mut editor = PatchEditor::new();
        editor
            .connections
            .push(Connection::new(osc, "out", amp, "in"));
        editor
            .connections
            .push(Connection::new(amp, "left", out, "in_l"));

        // Closing the loop back to an upstream module is a cycle.
        assert!(editor.would_create_cycle(out, osc));
        assert!(editor.would_create_cycle(amp, osc));
        // Self-loop.
        assert!(editor.would_create_cycle(amp, amp));
        // A normal downstream edge is fine (parallel edge, no loop).
        assert!(!editor.would_create_cycle(osc, out));
        assert!(!editor.would_create_cycle(osc, amp));
    }

    /// S2.4: the panel script mirror is snapshot-driven. `sync_module_scripts`
    /// maps the engine snapshot's 1-based slot keys to the panel's 0-based
    /// `slot_scripts`, and clear-fills (a slot absent from the snapshot — script
    /// cleared in the engine — drops from the mirror).
    #[test]
    fn sync_module_scripts_maps_one_based_snapshot_and_clear_fills() {
        let mut editor = PatchEditor::new();
        let id = ModuleId::new(ModuleType::ModMatrix, 1);
        editor
            .panels
            .insert(id, ModulePanelState::new(id, Pos2::ZERO));
        // A stale script that the next snapshot omits — must be dropped.
        editor
            .panels
            .get_mut(&id)
            .unwrap()
            .slot_scripts
            .insert(4, "out = 1".to_string());

        let mut snap = std::collections::BTreeMap::new();
        snap.insert("1".to_string(), "out = velocity".to_string());
        snap.insert("3".to_string(), "out = lfo-1.out".to_string());
        editor.sync_module_scripts(id, &snap);

        let scripts = &editor.panels[&id].slot_scripts;
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts.get(&0).map(String::as_str), Some("out = velocity"));
        assert_eq!(scripts.get(&2).map(String::as_str), Some("out = lfo-1.out"));
        assert!(
            !scripts.contains_key(&4),
            "clear-fill drops scripts absent from the snapshot"
        );

        // An empty snapshot clears the whole mirror.
        editor.sync_module_scripts(id, &std::collections::BTreeMap::new());
        assert!(editor.panels[&id].slot_scripts.is_empty());
    }

    /// The panel description mirror is snapshot-driven: `sync_module_description`
    /// copies the engine snapshot's value in (seeding the info popup + editor),
    /// and an empty snapshot value clears it. A missing panel is a no-op.
    #[test]
    fn sync_module_description_mirrors_snapshot() {
        let mut editor = PatchEditor::new();
        let id = ModuleId::new(ModuleType::Lfo, 1);
        editor
            .panels
            .insert(id, ModulePanelState::new(id, Pos2::ZERO));

        editor.sync_module_description(id, "wobble LFO for the cutoff");
        assert_eq!(editor.panels[&id].description, "wobble LFO for the cutoff");

        // A cleared snapshot value empties the mirror.
        editor.sync_module_description(id, "");
        assert!(editor.panels[&id].description.is_empty());

        // Syncing an unknown module is a no-op (no panic, no insert).
        let ghost = ModuleId::new(ModuleType::Lfo, 99);
        editor.sync_module_description(ghost, "ignored");
        assert!(!editor.panels.contains_key(&ghost));
    }

    /// The per-frame highlight set (`recompute_drag_cycle_blocked`) must agree
    /// with the per-edge `would_create_cycle` check that gates the actual drop.
    /// Graph: osc → amp → out.
    #[test]
    fn drag_cycle_blocked_matches_per_edge_check() {
        let osc = ModuleId::new(ModuleType::Oscillator, 1);
        let amp = ModuleId::new(ModuleType::Amplifier, 1);
        let out = ModuleId::new(ModuleType::StereoOutput, 1);

        let mut editor = PatchEditor::new();
        editor
            .connections
            .push(Connection::new(osc, "out", amp, "in"));
        editor
            .connections
            .push(Connection::new(amp, "left", out, "in_l"));

        let pending = |module, direction| PendingConnection {
            from_module: module,
            from_port: "p".into(),
            from_position: Pos2::ZERO,
            from_type: WidgetPortType::Audio,
            from_direction: direction,
            current_pos: Pos2::ZERO,
        };

        // Dragging from `out`'s OUTPUT blocks its ancestors (amp, osc) + itself.
        editor.pending_connection = Some(pending(out, WidgetPortDirection::Output));
        editor.recompute_drag_cycle_blocked();
        assert_eq!(
            editor.drag_cycle_blocked,
            HashSet::from([out, amp, osc]),
            "output drag blocks ancestors"
        );

        // Dragging from `osc`'s INPUT blocks its descendants (amp, out) + itself.
        editor.pending_connection = Some(pending(osc, WidgetPortDirection::Input));
        editor.recompute_drag_cycle_blocked();
        assert_eq!(
            editor.drag_cycle_blocked,
            HashSet::from([osc, amp, out]),
            "input drag blocks descendants"
        );

        // Dragging from `osc`'s OUTPUT has no ancestors — only the self-loop.
        editor.pending_connection = Some(pending(osc, WidgetPortDirection::Output));
        editor.recompute_drag_cycle_blocked();
        assert_eq!(editor.drag_cycle_blocked, HashSet::from([osc]));

        // No drag → empty.
        editor.pending_connection = None;
        editor.recompute_drag_cycle_blocked();
        assert!(editor.drag_cycle_blocked.is_empty());
    }
}
