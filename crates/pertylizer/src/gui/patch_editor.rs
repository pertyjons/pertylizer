//! Rack view - the main workspace for arranging modules.
//!
//! This module handles the canvas where modules are placed,
//! connections are drawn, and modules can be added/removed.
//!
//! Modules are rendered as draggable, resizable windows with z-order support.
//! Cables are rendered behind modules; hovered cables pop to the foreground.

use eframe::egui::{self, Color32, Order, Pos2, Rect, Sense, Ui, Vec2};
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
    CaptionTone, ModRole, ModuleFrame, WidgetPortDirection, WidgetPortType, cable_color, caption,
    draw_cable_dragging, draw_module_header,
};

mod canvas;

mod groups;
use groups::collapsed_group_size;

mod popups;
use popups::draw_slot_expression_editor;

mod wiring;

mod header;
mod node;
mod ports;
mod selection;

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

/// Convert a screen-space position to scene/world coordinates. Inside the
/// `egui::Scene` closure the raw `ui.input` pointer is global/screen, but the
/// manual cable / port / group / background hit-tests compare against world
/// coordinates — egui only transforms the pointer for real widget interactions
/// (knobs, buttons), not for these lookups. Returns `screen` unchanged when the
/// layer has no transform (e.g. before the first Scene frame).
fn screen_to_world(ui: &Ui, screen: Pos2) -> Pos2 {
    ui.ctx()
        .layer_transform_to_global(ui.layer_id())
        .map_or(screen, |t| t.inverse() * screen)
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

/// Paint a tinted, framed, labelled background zone around a set of panels.
/// Returns silently when no panels are supplied so callers can pass a
/// filtered iterator without an outer empty-check.
fn draw_module_zone<'a>(
    ui: &mut Ui,
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

    // Panel positions are already world-space; the scene layer applies the
    // pan/zoom transform at paint time. (Pre-Scene this added the scroll
    // viewport origin, which now double-offsets the zone off into the camera.)
    let padding = GRID_SIZE;
    let zone_rect = Rect::from_min_max(
        Pos2::new(
            ((min_x - padding) / GRID_SIZE).floor() * GRID_SIZE,
            ((min_y - padding) / GRID_SIZE).floor() * GRID_SIZE,
        ),
        Pos2::new(
            ((max_x + padding) / GRID_SIZE).ceil() * GRID_SIZE,
            ((max_y + padding) / GRID_SIZE).ceil() * GRID_SIZE,
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

/// The Script-module output slots a YAMS script references, found by compiling it
/// and resolving its `scr-N.outM` source addresses. Only Script outputs are
/// returned — they are the sole scripted slot exposing an addressable output a
/// later script can read back, so they are the only edges that can close a latent
/// feedback loop (LFO / macro / context sources can't). A script that fails to
/// compile yields an empty set (the live editor already flags the compile error).
fn script_output_refs(src: &str) -> HashSet<(ModuleId, u8)> {
    if src.trim().is_empty() {
        return HashSet::new();
    }
    let (program, _diags) = synth_script::compile(src, &synth_script::CompileOptions::default());
    let Some(program) = program else {
        return HashSet::new();
    };
    // `into_bound` needs an owned source string only for persistence/inspection;
    // we read `inputs` and discard it, so an empty string is fine here.
    script_refs_from_inputs(&program.into_bound(String::new()).inputs)
}

/// The Script-module output slots referenced by an already-compiled script's
/// `inputs`. Split out from [`script_output_refs`] so a caller that already
/// compiled the source (the live editor's status line) can extract refs without
/// recompiling.
fn script_refs_from_inputs(inputs: &[synth_core::script::ScriptInput]) -> HashSet<(ModuleId, u8)> {
    let mut refs = HashSet::new();
    for input in inputs {
        if let synth_core::script::ScriptInput::Source(SrcAddr::Module {
            module_type: ModuleType::Script,
            instance,
            name,
        }) = input
            && let Some(slot) = synth_modules::script_module::output_port_slot(name.as_str())
        {
            refs.insert((ModuleId::new(ModuleType::Script, *instance), slot as u8));
        }
    }
    refs
}

/// Latent script→script feedback edges across the whole patch, for the ƒx
/// editor's loop warning (§3.5). YAMS sources are address-based and resolved with
/// a one-block latency — they bypass the graph's cable cycle-detection
/// (`drag_cycle_blocked`), so a script reading its own (or a downstream script's)
/// output forms a delayed feedback path the cable checks never see. The delay
/// makes this safe at runtime (no infinite loop / stack overflow — YAMS bytecode
/// is straight-line); the warning exists purely so the user isn't surprised by
/// the one-block feedback.
///
/// Nodes are `(Script ModuleId, 0-based slot)` — the only scripted slot with an
/// addressable output. A Mod Matrix slot can read a script but exposes no output,
/// so it can never close a loop and is not a node. Built only while an expression
/// editor is open (it compiles every installed script).
struct ScriptDepGraph {
    /// node → the Script output slots its installed script reads.
    edges: HashMap<(ModuleId, u8), HashSet<(ModuleId, u8)>>,
}

/// Per-slot compiled-source-reference cache: `(module, slot) → (script text,
/// referenced Script outputs)`. Lets [`ScriptDepGraph::from_panels_cached`] skip
/// recompiling a slot whose text is unchanged.
type ScriptRefCache = HashMap<(ModuleId, u8), (String, HashSet<(ModuleId, u8)>)>;

impl ScriptDepGraph {
    /// Build from the installed scripts of every Script-module panel, reusing
    /// `cache` so a slot whose script text is unchanged is not recompiled — the
    /// per-frame cost while the ƒx editor is open then scales with *changed*
    /// scripts, not all of them.
    fn from_panels_cached(
        panels: &HashMap<ModuleId, ModulePanelState>,
        cache: &mut ScriptRefCache,
    ) -> Self {
        let mut edges = HashMap::new();
        for (id, panel) in panels {
            if id.module_type != ModuleType::Script {
                continue;
            }
            for (slot, src) in &panel.slot_scripts {
                let key = (*id, *slot);
                let refs = match cache.get(&key) {
                    Some((cached_src, cached_refs)) if cached_src == src => cached_refs.clone(),
                    _ => {
                        let r = script_output_refs(src);
                        cache.insert(key, (src.clone(), r.clone()));
                        r
                    }
                };
                if !refs.is_empty() {
                    edges.insert(key, refs);
                }
            }
        }
        Self { edges }
    }

    /// A human-readable warning if installing the script whose extracted
    /// `draft_refs` are given on `(module_id, slot)` would make that Script slot
    /// read its own output back (self-reference) or sit on a script→script cycle —
    /// both resolve with a one-block delay. `None` when the edited module is not a
    /// Script module, or no loop is formed. `draft_refs` come from the live draft
    /// (not its installed script), so the warning updates as the user types; the
    /// caller passes them already-extracted to avoid a redundant recompile.
    fn cycle_warning(
        &self,
        module_id: ModuleId,
        slot: u8,
        draft_refs: &HashSet<(ModuleId, u8)>,
    ) -> Option<String> {
        if module_id.module_type != ModuleType::Script {
            return None;
        }
        let start = (module_id, slot);
        // Direct self-reference is the simplest loop — name it explicitly.
        if draft_refs.contains(&start) {
            return Some(format!(
                "feeds back on itself (scr-{}.out{} reads its own output) — \
                 resolved 1 block late",
                module_id.instance,
                slot + 1,
            ));
        }
        let path = self.find_cycle_path(start, draft_refs)?;
        let chain = path
            .iter()
            .map(|(id, s)| format!("scr-{}.out{}", id.instance, s + 1))
            .collect::<Vec<_>>()
            .join(" → ");
        Some(format!(
            "forms a feedback cycle ({chain}) — resolved 1 block late"
        ))
    }

    /// DFS for a path `start → … → start`, using `start`'s edges from
    /// `start_refs` (the live draft) and every other node's installed edges.
    /// Returns the cycle as a node sequence that begins and ends at `start`, or
    /// `None` if acyclic. The graph is tiny (a handful of script slots), so plain
    /// recursion with a global visited set is ample.
    fn find_cycle_path(
        &self,
        start: (ModuleId, u8),
        start_refs: &HashSet<(ModuleId, u8)>,
    ) -> Option<Vec<(ModuleId, u8)>> {
        let mut path = vec![start];
        let mut visited = HashSet::new();
        visited.insert(start);
        for &next in start_refs {
            if let Some(found) = self.dfs_to(start, next, &mut path, &mut visited) {
                return Some(found);
            }
        }
        None
    }

    /// Recursive helper for [`Self::find_cycle_path`]. Returns the closed cycle
    /// path when `node` can reach `start`. `visited` is global: a node that can't
    /// reach `start` down one branch can't down another either, so it is never
    /// retried.
    fn dfs_to(
        &self,
        start: (ModuleId, u8),
        node: (ModuleId, u8),
        path: &mut Vec<(ModuleId, u8)>,
        visited: &mut HashSet<(ModuleId, u8)>,
    ) -> Option<Vec<(ModuleId, u8)>> {
        if node == start {
            let mut cycle = path.clone();
            cycle.push(start);
            return Some(cycle);
        }
        if !visited.insert(node) {
            return None;
        }
        path.push(node);
        if let Some(neighbours) = self.edges.get(&node) {
            for &next in neighbours {
                if let Some(found) = self.dfs_to(start, next, path, visited) {
                    return Some(found);
                }
            }
        }
        path.pop();
        None
    }
}

/// A single addressing target for the Mod Matrix pickers (S1.5c).
struct ModAddrTarget {
    id: ModuleId,
    /// Display label, e.g. "Filter 1".
    label: String,
    /// Automatable parameters as `(type_id, display label)`. Used both as
    /// destination picks (modulate this param) and as **source** picks — a script
    /// or routing can read the live value (`flt-1.cutoff`), normalized 0..1 via the
    /// descriptor range/curve by `resolve_param_source`.
    params: Vec<(String, String)>,
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
                params: desc
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
                let target = self.target(module_type, instance);
                let base = target.map_or_else(
                    || format!("{}-{instance}", module_type.prefix()),
                    |t| t.label.clone(),
                );
                // A source can be an output port (`out`) or a param (`cutoff`);
                // show the param's friendly name when the address matches one.
                let member = target
                    .and_then(|t| {
                        t.params
                            .iter()
                            .find(|(tid, _)| tid.as_str() == name.as_str())
                    })
                    .map_or_else(|| name.as_str().to_string(), |(_, label)| label.clone());
                format!("{base} · {member}")
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
                t.params
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

/// A fixed-width `menu_button` shell for the Mod Matrix source/destination tree
/// pickers, styled like the Script ƒx "Select input" picker (a plain button that
/// opens a nested menu) rather than a ComboBox. `label` is the current selection
/// shown on the button; the justified layout stretches it to `width` so the slot
/// grid stays aligned; `id_salt` keeps slots with identical labels from colliding.
/// `contents` builds the dropdown tree (mutating its own captured result).
fn tree_picker_menu(
    ui: &mut Ui,
    id_salt: &str,
    width: f32,
    label: String,
    contents: impl FnOnce(&mut Ui),
) {
    ui.push_id(id_salt, |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(width, ui.spacing().interact_size.y),
            egui::Layout::top_down_justified(egui::Align::Min),
            |ui| {
                ui.menu_button(label, contents);
            },
        );
    });
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
    tree_picker_menu(ui, &id_salt, width, text, |ui| {
        if ui.selectable_label(current.is_none(), "(none)").clicked() {
            result = Some(None);
            ui.close();
        }
        // Tree: module → output ports + params (a param reads the live value,
        // `flt-1.cutoff`). Mirrors the Script ƒx "Select input" menu so both
        // source pickers share one shape.
        for target in &catalog.modules {
            if target.source_ports.is_empty() && target.params.is_empty() {
                continue;
            }
            ui.menu_button(&target.label, |ui| {
                for port in &target.source_ports {
                    let addr = SrcAddr::module(target.id.module_type, target.id.instance, port);
                    if ui
                        .selectable_label(current_addr == Some(addr), port.as_str())
                        .clicked()
                    {
                        result = Some(Some(addr));
                        ui.close();
                    }
                }
                if !target.source_ports.is_empty() && !target.params.is_empty() {
                    ui.separator();
                }
                // Addressed by `type_id`, labelled with the friendly name.
                // Skip a param whose name collides with an output port:
                // `resolve_source` resolves the port first, so the param would
                // be unreachable under this address (and double-highlight).
                for (type_id, param_label) in &target.params {
                    if target.source_ports.iter().any(|p| p == type_id) {
                        continue;
                    }
                    let addr = SrcAddr::module(target.id.module_type, target.id.instance, type_id);
                    if ui
                        .selectable_label(current_addr == Some(addr), param_label.as_str())
                        .clicked()
                    {
                        result = Some(Some(addr));
                        ui.close();
                    }
                }
            });
        }
        ui.separator();
        ui.menu_button("Macros", |ui| {
            for m in MacroSource::ALL {
                let addr = SrcAddr::Macro(m);
                if ui
                    .selectable_label(current_addr == Some(addr), macro_label(m))
                    .clicked()
                {
                    result = Some(Some(addr));
                    ui.close();
                }
            }
        });
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
    tree_picker_menu(ui, &id_salt, width, text, |ui| {
        if ui.selectable_label(current.is_none(), "(none)").clicked() {
            result = Some(None);
            ui.close();
        }
        // Tree: module → modulatable params, mirroring the source picker so
        // both Mod Matrix dropdowns share one shape.
        for target in &catalog.modules {
            if target.params.is_empty() {
                continue;
            }
            ui.menu_button(&target.label, |ui| {
                for (type_id, label) in &target.params {
                    let addr = DestAddr::new(target.id.module_type, target.id.instance, type_id);
                    if ui
                        .selectable_label(current_addr == Some(addr), label.as_str())
                        .clicked()
                    {
                        result = Some(Some(addr));
                        ui.close();
                    }
                }
            });
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

/// Explicit canvas interaction state — one mutually-exclusive mode at a time.
///
/// This is the patch-editor interaction FSM: marquee selection, multi-node drag,
/// and cable drag are now all variants here instead of being spread across a
/// `pending_connection: Option<…>` field plus ad-hoc booleans, so "two modes at
/// once" states are unrepresentable.
#[derive(Debug, Clone, Default)]
enum CanvasInteraction {
    #[default]
    Idle,
    /// Left-drag marquee selection. Both points are in **world** coords so the
    /// rectangle survives a mid-drag pan/zoom.
    RubberBand { start: Pos2, current: Pos2 },
    /// Dragging one or more module cards together. The set is captured at
    /// drag-start (so changing the selection mid-drag can't affect it); every
    /// card moves by the same per-frame `drag_delta`, which preserves their
    /// relative layout without tracking per-card grab offsets.
    DraggingNodes { ids: Vec<ModuleId> },
    /// Dragging a cable out from a port. Holds the in-progress connection
    /// (source port + live cursor pos). Mutually exclusive with the other
    /// interactions by construction — that is the point of folding the old
    /// `pending_connection` field into this state machine.
    DraggingWire(PendingConnection),
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
    /// Current canvas interaction (rubber-band marquee; `Idle` otherwise).
    canvas_interaction: CanvasInteraction,
    /// Currently selected group.
    selected_group: Option<GroupId>,
    /// In-progress rename buffer for group context menu.
    group_name_edit: Option<(GroupId, String)>,
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
    /// The visible world bounds of the patch canvas — the `egui::Scene` camera.
    /// `Scene::show` mutates this on pan/zoom; `None` until the first frame, which
    /// frames the existing modules. Replaces the old `ScrollArea` scroll offset +
    /// `area_origin` bookkeeping (positions are now world coordinates directly).
    scene_rect: Option<Rect>,
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
    /// Per-script-slot compiled source references, keyed by `(module, slot)` →
    /// `(script text, referenced Script outputs)`. Lets the feedback-loop graph
    /// (§3.5) rebuild each frame the ƒx editor is open without recompiling every
    /// script — only a slot whose text changed is re-extracted. See
    /// `build_script_graph`.
    script_ref_cache: ScriptRefCache,
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
    /// Measured world size of each collapsed group box, read back from the
    /// rendered child `Ui` and reused (one-frame lag) as the drag/select
    /// interact rect next frame — the same trick the module cards use, so the
    /// whole visible box is grabbable even when the static size estimate is off.
    collapsed_group_sizes: HashMap<GroupId, Vec2>,
}

/// Transient state for the open "Edit description" popup. `draft` is the
/// in-progress text, independent of the installed description until applied.
#[derive(Clone)]
struct DescriptionEditorState {
    module_id: ModuleId,
    draft: String,
}

/// The per-module inputs a header's action row needs: which module, its
/// descriptor, and the status flags it renders as badges. Bundled to keep
/// [`PatchEditor::draw_module_header_actions`]'s signature small.
struct ModuleHeaderCtx<'a> {
    module_id: ModuleId,
    descriptor: &'a ModuleDescriptor,
    is_source: bool,
    is_sink: bool,
    is_automated: bool,
    is_bypassed: bool,
    is_global_module: bool,
    connectivity: ModuleConnectivity,
}

/// The per-module inputs a module body needs to draw its ports and parameter
/// panel. Frame-level inputs (analysis, catalog, script graph, …) are the same
/// for every module in the loop; bundling them keeps
/// [`PatchEditor::draw_module_body`]'s signature small.
struct ModuleBodyCtx<'a> {
    module_id: ModuleId,
    descriptor: &'a ModuleDescriptor,
    accent_color: Color32,
    connected_ports: &'a [PortName],
    analysis: &'a PatchAnalysis,
    mod_catalog: &'a ModAddrCatalog,
    script_graph: Option<&'a ScriptDepGraph>,
    effect_chain_order: &'a [ModuleId],
    audio_input_snapshot: &'a AudioInputSnapshot,
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
            canvas_interaction: CanvasInteraction::Idle,
            selected_group: None,
            group_name_edit: None,
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
            scene_rect: None,
            sample_list: Vec::new(),
            prev_effect_chain_order: Vec::new(),
            prev_mod_matrix_attachments: Vec::new(),
            connected_ports_cache: HashMap::new(),
            script_ref_cache: HashMap::new(),
            drag_cycle_blocked: HashSet::new(),
            description_editor: None,
            info_popup: None,
            module_rects: HashMap::new(),
            collapsed_group_sizes: HashMap::new(),
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
        self.canvas_interaction = CanvasInteraction::Idle; // also cancels any wire drag
        self.selected_group = None;
        self.group_name_edit = None;
        self.next_module_pos = Pos2::new(50.0, 50.0);
        self.groups.clear();
        self.module_to_group.clear();
        self.next_group_id = 1;
        self.group_context_menu = None;
        self.needs_reposition.clear();
        self.connected_ports_cache.clear();
        self.description_editor = None;
        self.info_popup = None;
        self.module_rects.clear();
        self.collapsed_group_sizes.clear();
        // Reframe the camera on the next show() — otherwise the previous patch's
        // pan/zoom would persist across project/instrument loads (the modules of
        // the newly loaded patch can sit in a different world region entirely).
        self.scene_rect = None;
    }

    /// The in-progress cable drag, if the canvas is currently dragging a wire.
    /// (Wire drag lives in the `CanvasInteraction` FSM; these accessors keep the
    /// call sites reading the way the old `pending_connection` field did.)
    fn pending_connection(&self) -> Option<&PendingConnection> {
        match &self.canvas_interaction {
            CanvasInteraction::DraggingWire(p) => Some(p),
            _ => None,
        }
    }

    /// Mutable view of the in-progress cable drag (e.g. to update the live cursor).
    fn pending_connection_mut(&mut self) -> Option<&mut PendingConnection> {
        match &mut self.canvas_interaction {
            CanvasInteraction::DraggingWire(p) => Some(p),
            _ => None,
        }
    }

    /// Begin (or replace) a cable drag.
    fn start_wire_drag(&mut self, pending: PendingConnection) {
        self.canvas_interaction = CanvasInteraction::DraggingWire(pending);
    }

    /// Cancel a cable drag, leaving other interaction states untouched.
    fn cancel_wire_drag(&mut self) {
        if matches!(self.canvas_interaction, CanvasInteraction::DraggingWire(_)) {
            self.canvas_interaction = CanvasInteraction::Idle;
        }
    }

    /// Frame the existing modules (padded) for the first `Scene` frame, honoring
    /// the `scene_rect` doc contract ("frames the existing modules"). Falls back
    /// to the viewport at the world origin when there are no modules yet.
    fn initial_scene_rect(&self, visible_rect: Rect) -> Rect {
        let mut bounds: Option<Rect> = None;
        for panel in self.panels.values() {
            let r = Rect::from_min_size(panel.position, panel.size);
            bounds = Some(bounds.map_or(r, |b| b.union(r)));
        }
        bounds.map_or_else(
            || Rect::from_min_size(Pos2::ZERO, visible_rect.size()),
            |b| b.expand(80.0),
        )
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
        // Cancel a cable drag if it started from this module
        if self
            .pending_connection()
            .is_some_and(|p| p.from_module == id)
        {
            self.cancel_wire_drag();
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

        // Addressing targets for the Mod Matrix pickers (S1.5c) and the Script
        // module's "Select input" picker (§3.5). Built here, before the
        // `self.panels.get_mut` borrow below, so the pickers can address every
        // other module without re-borrowing `self`. Skip the (allocating) build
        // entirely when nothing consumes it — neither a Mod Matrix nor a Script
        // module is present.
        let needs_catalog = self
            .descriptors
            .keys()
            .any(|id| matches!(id.module_type, ModuleType::ModMatrix | ModuleType::Script));
        let mod_catalog = if needs_catalog {
            ModAddrCatalog::from_descriptors(&self.descriptors)
        } else {
            ModAddrCatalog {
                modules: Vec::new(),
            }
        };

        // Script feedback-loop graph for the ƒx editor's warning (§3.5). Only
        // built while an expression editor is open (the warning has no other
        // consumer), and via the cache so unchanged scripts aren't recompiled
        // each frame the popup stays open.
        let editor_open = self.panels.values().any(|p| p.script_editor.is_some());
        let script_graph = if editor_open {
            Some(ScriptDepGraph::from_panels_cached(
                &self.panels,
                &mut self.script_ref_cache,
            ))
        } else {
            None
        };

        // Save the visible rect for toolbar / popup positioning (before the Scene).
        let visible_rect = ui.available_rect_before_wrap();

        // egui::Scene — a pannable/zoomable WORLD-space canvas. Module positions
        // ARE world coordinates; the Scene owns the pan/zoom transform, and the
        // scene layer id makes group/cable painting land behind the nodes.
        let mut scene_rect = match self.scene_rect {
            Some(r) => r,
            None => self.initial_scene_rect(visible_rect),
        };

        let _scene_output = egui::Scene::new()
            .zoom_range(egui::Rangef::new(0.2, 2.0))
            // Plan 4c: only RIGHT-drag pans the canvas, freeing LEFT-drag on the
            // empty grid for rubber-band selection. Pan is still also available via
            // scroll / trackpad, so no pan path is lost. Right-CLICK (no movement)
            // still opens the rack/cable context menu — egui distinguishes a
            // secondary click from a secondary drag.
            .drag_pan_buttons(egui::containers::DragPanButtons::SECONDARY)
            .show(ui, &mut scene_rect, |ui| {
                // Everything below draws in WORLD coordinates on the scene layer.
                let world_rect = ui.clip_rect();
                // Background response for deselect + the right-click rack menu.
                // CLICK-only on purpose: a drag-sensing widget here would capture
                // the pointer and starve the Scene's own pan response. Rubber-band
                // (left-drag) is instead read from the Scene's background response
                // (`ui.response()`) in `handle_canvas_background_input`.
                let canvas_response =
                    Some(ui.interact(world_rect, ui.id().with("canvas_bg"), Sense::click()));

                // Draw grid + tinted background zones.
                self.draw_grid(ui, world_rect);
                self.draw_effect_zone(ui);
                self.draw_monitors_zone(ui);
                self.draw_mod_matrix_zone(ui, &analysis);

                // The scene layer carries the pan/zoom transform; painting cables and
                // group frames on it puts them behind the nodes (drawn later).
                let scene_layer_id = ui.layer_id();
                let scene_clip_rect = ui.clip_rect();

                // Compute group layout (bounds + hidden modules) before drawing cables
                let group_layout = self.compute_group_layout();
                let mut new_group_port_positions: HashMap<GroupPortKey, PortPosition> =
                    HashMap::new();

                // Module world rects, straight from each panel's stored position + size
                // (no more reading egui Area memory back).
                let module_rects: Vec<Rect> = self
                    .panels
                    .values()
                    .filter_map(|p| {
                        if group_layout.hidden_modules.contains(&p.id) {
                            return None;
                        }
                        Some(Rect::from_min_size(p.position, p.size))
                    })
                    .collect();

                // Draw group frames on the scene layer BEFORE cables.
                self.draw_group_frames(ui, &group_layout, scene_layer_id, scene_clip_rect);

                // Handle interactions for expanded group frames.
                self.handle_group_interactions(ui, &group_layout, &module_rects);

                // Draw cables on the scene layer BEFORE the nodes. This uses the
                // previous frame's port_positions — one frame delay is imperceptible.
                let time = ui.input(|i| i.time);
                self.draw_connections(ui, time, scene_layer_id, scene_clip_rect, &module_rects);
                // Draw effect chain cables (signal flow between effects)
                self.draw_effect_chain_cables(
                    ui,
                    scene_layer_id,
                    scene_clip_rect,
                    effect_chain_order,
                );

                // Draw collapsed group boxes (movable) after cables so they sit above.
                self.draw_collapsed_groups(
                    ui,
                    instrument_id,
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
                    let (panel_position, panel_size) = match self.panels.get(&module_id) {
                        Some(s) => (s.position, s.size),
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
                        ModuleCategory::Effect
                            | ModuleCategory::Visualizer
                            | ModuleCategory::Utility
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
                    let window_id =
                        egui::Id::new((instrument_id, "module_window", module_id.to_string()));
                    // Create frame with dimming for disconnected modules
                    let frame = ModuleFrame::new(dimmed_accent)
                        .selected(is_selected)
                        .opacity(opacity)
                        .build(&ui.global_style());

                    // Check if this module needs repositioning (after auto-layout)
                    let needs_reposition = self.needs_reposition.contains(&module_id);

                    // Header shows the stable module id (e.g. "nse-1"); the
                    // human name + category live in the hover tooltip below.
                    let title = module_id.to_string();
                    let tooltip = {
                        let mut t = analysis.display_name(module_id, &descriptor.name);
                        t.push_str(&format!(
                            "\n{} · {}",
                            module_id,
                            category_label(descriptor.category)
                        ));
                        if !descriptor.description.is_empty() {
                            t.push_str("\n\n");
                            t.push_str(&descriptor.description);
                        }
                        t
                    };

                    // Get processing info for this module
                    let is_source = self.is_source(module_id);
                    let is_sink = self.is_sink(module_id);
                    let is_automated = automated_modules.contains(&module_id);
                    let is_inline_monitor = descriptor.type_id.0 == "inline_signal_monitor";

                    // Place the module at its WORLD position — no egui::Area (an Area
                    // inside a Scene would not inherit the layer transform). Own the drag
                    // with a click_and_drag interact over the card, registered BEFORE the
                    // body so the body's buttons/knobs (drawn on top) keep their own
                    // clicks/drags. `drag_delta()` is already world-space inside a Scene.
                    let node_rect = Rect::from_min_size(panel_position, panel_size);
                    // Distinct id from the child Ui's id-space (which is seeded with
                    // window_id below) so the card drag handle never collides with a
                    // body widget.
                    let node_response =
                        ui.interact(node_rect, window_id.with("card"), Sense::click_and_drag());
                    // Capture the drag set at drag-start: the whole selection if
                    // this card is part of it, otherwise just this card. Stored in
                    // the interaction FSM so a mid-drag selection change can't
                    // affect it.
                    if node_response.drag_started() {
                        let ids = if self.selected_modules.contains(&module_id) {
                            self.selected_modules.iter().copied().collect()
                        } else {
                            vec![module_id]
                        };
                        self.canvas_interaction = CanvasInteraction::DraggingNodes { ids };
                    }
                    if node_response.dragged() {
                        let delta = node_response.drag_delta();
                        // Move every card in the drag set by the same delta
                        // (preserves relative layout). Falls back to this card if
                        // the FSM state was lost for any reason.
                        let ids: Vec<ModuleId> = match &self.canvas_interaction {
                            CanvasInteraction::DraggingNodes { ids } => ids.clone(),
                            _ => vec![module_id],
                        };
                        for id in ids {
                            if let Some(p) = self.panels.get_mut(&id) {
                                p.position += delta;
                            }
                        }
                    }
                    if node_response.drag_stopped() {
                        // Snap the grabbed card to the grid, then shift the rest of
                        // the set by the same correction so the layout stays intact.
                        let ids: Vec<ModuleId> = match &self.canvas_interaction {
                            CanvasInteraction::DraggingNodes { ids } => ids.clone(),
                            _ => vec![module_id],
                        };
                        if let Some(before) = self.panels.get(&module_id).map(|p| p.position) {
                            let correction = snap_to_grid(before) - before;
                            for id in ids {
                                if let Some(p) = self.panels.get_mut(&id) {
                                    p.position += correction;
                                }
                            }
                        }
                        self.canvas_interaction = CanvasInteraction::Idle;
                    }
                    // Cursor affordance: a grab hand over a draggable card when
                    // idle. `hovered()` is false over inner widgets (knobs/buttons,
                    // drawn on top), so they keep their own cursors; the active-drag
                    // cursors (Grabbing/Crosshair) are set later from the FSM state.
                    if node_response.hovered()
                        && matches!(self.canvas_interaction, CanvasInteraction::Idle)
                    {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    let node_pos = self
                        .panels
                        .get(&module_id)
                        .map_or(panel_position, |p| p.position);

                    // Card content in a child Ui sized to the module's fixed-width
                    // bucket (`ModuleWidth::module_px`); the frame body is pinned to
                    // the bucket below. `min_rect()` is read back as the actual size.
                    let module_w = descriptor.width.module_px();
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            // Seed the child Ui's id with the module's window_id as a
                            // GLOBAL scope so the id is `Id::new(window_id)` — stable per
                            // module and INDEPENDENT of draw order. A plain `id_salt`
                            // (non-global) folds in the parent's `next_auto_id_salt` (a
                            // draw-order counter), so every module's inner widget ids
                            // shift whenever the z-order changes (bring-to-front), which
                            // is what triggered the red egui id-clash overlay. The old
                            // egui::Area's window_id was effectively a stable global id
                            // too. egui 0.35: `.id(id)` is the shortcut for the old
                            // `.id_salt(id).global_scope(true)`.
                            .id(window_id)
                            .max_rect(Rect::from_min_size(node_pos, Vec2::new(module_w, 600.0)))
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    {
                        let ui = &mut child;
                        if is_inline_monitor {
                            self.draw_inline_monitor(
                                ui,
                                module_id,
                                descriptor,
                                dimmed_accent,
                                handle,
                                &mut result,
                            );
                        } else {
                            frame.show(ui, |ui| {
                                // Pin the body to the fixed width bucket up front, so
                                // the header and 3-column body lay out at a deliberate,
                                // grid-aligned width instead of stretching to their
                                // widest row. (8 px = ModuleFrame's inner margin/side.)
                                ui.set_width(module_w - 16.0);

                                // Title bar: name + status icons + close button (single row)
                                draw_module_header(
                                    ui,
                                    dimmed_accent,
                                    &title,
                                    Some(tooltip),
                                    false,
                                    |ui| {
                                        self.draw_module_header_actions(
                                            ui,
                                            ModuleHeaderCtx {
                                                module_id,
                                                descriptor,
                                                is_source,
                                                is_sink,
                                                is_automated,
                                                is_bypassed,
                                                is_global_module,
                                                connectivity: connectivity_status,
                                            },
                                            &analysis,
                                            effect_chain_order,
                                            &mut result,
                                            &mut open,
                                        );
                                    },
                                );

                                self.draw_module_body(
                                    ui,
                                    ModuleBodyCtx {
                                        module_id,
                                        descriptor,
                                        accent_color,
                                        connected_ports: &connected_ports,
                                        analysis: &analysis,
                                        mod_catalog: &mod_catalog,
                                        script_graph: script_graph.as_ref(),
                                        effect_chain_order,
                                        audio_input_snapshot,
                                    },
                                    handle,
                                    &mut result,
                                );
                            });
                        }
                    }

                    // Read the frame's actual (world) size back.
                    let actual_rect = child.min_rect();
                    if let Some(p) = self.panels.get_mut(&module_id) {
                        p.size = actual_rect.size();
                    }
                    // Store the SCREEN rect (world → screen) so the screen-space
                    // info / description popups, drawn after the Scene, anchor
                    // beside the module instead of at the raw world position.
                    let screen_rect = ui
                        .ctx()
                        .layer_transform_to_global(ui.layer_id())
                        .map_or(actual_rect, |t| t * actual_rect);
                    self.module_rects.insert(module_id, screen_rect);

                    self.handle_module_interaction(
                        ui,
                        &node_response,
                        module_id,
                        &group_layout,
                        &mut bring_to_front,
                    );

                    // Right-click on a module's empty body opens the rack context menu.
                    node_response.context_menu(|ui| {
                        let (selected, _) =
                            self.bg_context_menu_contents(ui, &mut result, panel_position, None);
                        if let Some(sel) = selected {
                            result.context_add = Some((sel, panel_position, None));
                        }
                    });

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

                // Draw the in-progress cable on the scene layer so it tracks the
                // world-space ports under pan/zoom.
                if let Some(pending) = self.pending_connection() {
                    let color = cable_color(pending.from_type, 180);
                    draw_cable_dragging(
                        ui.painter(),
                        pending.from_position,
                        pending.current_pos,
                        color,
                    );
                }

                self.handle_canvas_background_input(ui, &canvas_response);
            });
        self.scene_rect = Some(scene_rect);

        // Macro-source rail (S1.5b): a fixed SCREEN-space strip — drawn OUTSIDE the
        // Scene so it never pans/zooms with the canvas.
        if analysis.count(ModuleType::ModMatrix) > 0 {
            self.draw_macro_source_rail(ui, instrument_id, visible_rect, &analysis);
        }

        // Module description / info popups — also screen-space, drawn after the
        // Scene so they float above the canvas at a readable size.
        self.draw_module_popups(ui, &mut result);

        result
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
                            caption(ui, "Macros", CaptionTone::Dim);
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
                                    caption(ui, macro_label(m), CaptionTone::Color(color));
                                    // Same Source glyph as the module-header badge,
                                    // shown only when the macro is actually wired.
                                    if active {
                                        caption(
                                            ui,
                                            ModRole::Source.glyph(),
                                            CaptionTone::Color(t.colors.accent_purple),
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

    /// Helper for background context menu items — uses shared palette_label for icon + color.
    fn bg_menu_item(ui: &mut Ui, selection: PaletteSelection, out: &mut Option<PaletteSelection>) {
        let (label, color) = palette_label(selection);
        if ui.button(egui::RichText::new(label).color(color)).clicked() {
            *out = Some(selection);
            ui.close();
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
    /// Clipboard requests from the rack's right-click menu. The backend owns the
    /// clipboard, so it consumes these after `show()` returns: copy/cut act on the
    /// current selection, paste drops the clipboard at an offset.
    pub request_copy: bool,
    pub request_cut: bool,
    pub request_paste: bool,
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
        let width = ui.available_width().clamp(150.0, 340.0);
        let height = (width * 0.5).clamp(80.0, 150.0);

        super::widgets::draw_oscilloscope(
            ui,
            &samples,
            width,
            height,
            gain,
            theme().colors.accent_cyan,
        );

        if vis_buffer.is_none() {
            caption(ui, "No signal", CaptionTone::Dim);
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
        let width = ui.available_width().clamp(150.0, 340.0);
        let height = (width * 0.5).clamp(80.0, 150.0);

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
            caption(ui, "No signal", CaptionTone::Dim);
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
        let width = ui.available_width().clamp(72.0, 140.0);
        let height = (width * 1.5).clamp(100.0, 170.0);

        super::widgets::draw_stereo_meter(
            ui,
            peak_l,
            peak_r,
            rms_l,
            rms_r,
            egui::vec2(width, height),
            true,
        );

        if vis_buffer.is_none() {
            caption(ui, "No signal", CaptionTone::Dim);
        }
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
    script_graph: Option<&ScriptDepGraph>,
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
                            caption(ui, header, CaptionTone::Color(content_tint));
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
                                        caption(ui, icon, CaptionTone::Color(color))
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
                            caption(ui, arrow_text, CaptionTone::Color(frame_stroke_color));
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
        state.script_editor = Some(super::module_panel::ScriptEditorState {
            slot,
            draft,
            ..Default::default()
        });
    }

    // Draw the expression-editor popup (S2.4) if one is open. A floating window so
    // the routing list stays compact. Compilation runs live (off the audio thread)
    // for the status line; Apply/Clear push actions the caller routes to the
    // session, which recompiles + installs the shared script.
    draw_slot_expression_editor(ui, state, script_graph, catalog, &mut mod_script_actions);

    PanelParamsResult {
        param_changes,
        audio_input_action: None,
        mod_script_actions,
    }
}

/// One input chosen from the ƒx editor's "Select input" picker.
enum PickedInput {
    /// A macro or context identifier (`velocity`, `beat`) — always in scope, so
    /// it splices in bare at the cursor.
    Bare(String),
    /// A module output address (`lfo-1.out`) — needs a `src` binding, so it
    /// splices in a variable plus (if absent) the binding line.
    ModuleSource(String),
}

/// Replace the char range `sel` (a sorted `(start, end)`; `None` → end of draft)
/// with `text`. An empty range is a plain insertion at the caret; a non-empty one
/// replaces the selection, matching how typing over a selection behaves. Returns
/// the char index just past the inserted text, for caret restore.
fn insert_at_cursor(draft: &mut String, sel: Option<(usize, usize)>, text: &str) -> usize {
    let char_len = draft.chars().count();
    let (start, end) = sel.unwrap_or((char_len, char_len));
    let start = start.min(char_len);
    let end = end.min(char_len).max(start);
    let byte = |idx: usize| {
        draft
            .char_indices()
            .nth(idx)
            .map_or(draft.len(), |(b, _)| b)
    };
    draft.replace_range(byte(start)..byte(end), text);
    start + text.chars().count()
}

/// A YAMS-legal variable name derived from a source address: `lfo-1.out` →
/// `lfo1_out` (drop `-`, `.` → `_`). A leading digit gets an `_` prefix since
/// YAMS identifiers are alpha-led.
fn derive_src_var(addr: &str) -> String {
    let mut s: String = addr
        .chars()
        .filter(|c| *c != '-')
        .map(|c| if c == '.' { '_' } else { c })
        .collect();
    if s.starts_with(|c: char| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

/// If a `src <name> = <addr>` line already binds `addr`, return its variable
/// name so the picker reuses it instead of adding a duplicate binding.
fn existing_binding_var(draft: &str, addr: &str) -> Option<String> {
    draft.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("src ")?;
        let (name, value) = rest.split_once('=')?;
        // Strip a trailing `# comment` so `src x = lfo-1.out  # main` still
        // counts as binding `lfo-1.out` (and isn't duplicated by the picker).
        let value = value.split('#').next().unwrap_or(value);
        (value.trim() == addr).then(|| name.trim().to_string())
    })
}

/// Every name bound by a `src`/`let` line in the draft — the set a new binding
/// must avoid colliding with (YAMS rejects duplicate names).
fn bound_names(draft: &str) -> HashSet<String> {
    draft
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("src ")
                .or_else(|| line.strip_prefix("let "))?;
            let (name, _) = rest.split_once('=')?;
            Some(name.trim().to_string())
        })
        .collect()
}

/// A binding name for `addr` that doesn't collide with an existing `src`/`let`
/// name: the derived name, or it with a `_N` suffix. Prevents the picker from
/// emitting two bindings with the same name (which the compiler would reject).
fn unique_src_var(draft: &str, addr: &str) -> String {
    let taken = bound_names(draft);
    let base = derive_src_var(addr);
    if !taken.contains(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}_{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Splice a module-output reference into `draft`: ensure a `src <var> = <addr>`
/// binding exists (reuse an existing one, else prepend a fresh, collision-free
/// binding as the first line), then insert `<var>` over the selection `sel` so it
/// lands where the user is typing. Returns the new caret char index.
fn insert_module_source(draft: &mut String, sel: Option<(usize, usize)>, addr: &str) -> usize {
    // Reuse an existing binding for this address — just insert its variable.
    if let Some(var) = existing_binding_var(draft, addr) {
        return insert_at_cursor(draft, sel, &var);
    }
    let var = unique_src_var(draft, addr);
    let line = format!("src {var} = {addr}\n");
    // Insert the variable at the selection first (indices refer to the current
    // draft), then prepend the binding and shift the caret past it.
    let new_cursor = insert_at_cursor(draft, sel, &var);
    draft.insert_str(0, &line);
    new_cursor + line.chars().count()
}

/// Move the egui multiline caret to char index `idx` next frame, so an inserted
/// snippet leaves the cursor at its end rather than wherever it was.
fn set_text_caret(ctx: &egui::Context, id: egui::Id, idx: usize) {
    if let Some(mut st) = egui::text_edit::TextEditState::load(ctx, id) {
        let cursor = egui::text::CCursor::new(idx);
        st.cursor
            .set_char_range(Some(egui::text::CCursorRange::one(cursor)));
        st.store(ctx, id);
    }
}

/// The "Select input" tree picker for the ƒx editor: modules → output ports and
/// params, plus the macro and context sources. Returns the chosen input (if any)
/// for the caller to splice into the draft. Sourced from the same catalog as the
/// Mod Matrix pickers (S1.5c), so it stays in sync with what `resolve_source` binds.
fn draw_select_input_menu(ui: &mut Ui, catalog: &ModAddrCatalog) -> Option<PickedInput> {
    let mut picked = None;
    ui.menu_button("Select input", |ui| {
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                // Module output ports and params — each becomes a `src` binding.
                // A param source (`flt-1.cutoff`) reads the live value normalized
                // 0..1 via the descriptor range/curve (resolve_param_source),
                // exactly like a Mod Matrix source.
                for target in &catalog.modules {
                    if target.source_ports.is_empty() && target.params.is_empty() {
                        continue;
                    }
                    ui.menu_button(&target.label, |ui| {
                        for port in &target.source_ports {
                            if ui.button(port).clicked() {
                                let addr = SrcAddr::module(
                                    target.id.module_type,
                                    target.id.instance,
                                    port,
                                )
                                .to_address_string();
                                picked = Some(PickedInput::ModuleSource(addr));
                                ui.close();
                            }
                        }
                        if !target.source_ports.is_empty() && !target.params.is_empty() {
                            ui.separator();
                        }
                        // Params: address by `type_id` (what resolve_param_source
                        // matches), label with the friendly name. Skip a param
                        // whose name collides with an output port — `resolve_source`
                        // resolves the port first, so it would be unreachable.
                        for (type_id, label) in &target.params {
                            if target.source_ports.iter().any(|p| p == type_id) {
                                continue;
                            }
                            if ui.button(label).clicked() {
                                let addr = SrcAddr::module(
                                    target.id.module_type,
                                    target.id.instance,
                                    type_id,
                                )
                                .to_address_string();
                                picked = Some(PickedInput::ModuleSource(addr));
                                ui.close();
                            }
                        }
                    });
                }
                ui.separator();
                // Macros and context vars are bare identifiers (no `src` needed).
                ui.menu_button("Macros", |ui| {
                    for (name, label) in synth_script::symbols::MACRO_CATALOG {
                        if ui.button(format!("{label}  ({name})")).clicked() {
                            picked = Some(PickedInput::Bare((*name).to_string()));
                            ui.close();
                        }
                    }
                });
                ui.menu_button("Context", |ui| {
                    for (name, label) in synth_script::symbols::CONTEXT_CATALOG {
                        if ui.button(format!("{label}  ({name})")).clicked() {
                            picked = Some(PickedInput::Bare((*name).to_string()));
                            ui.close();
                        }
                    }
                });
            });
    });
    picked
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
    script_graph: Option<&ScriptDepGraph>,
    catalog: &ModAddrCatalog,
) -> PanelParamsResult {
    let mut mod_script_actions: Vec<(u8, Option<String>)> = Vec::new();
    let mut open_editor_for: Option<u8> = None;
    let t = theme();
    // Match the output-port column's row pitch so each slot row lines up with
    // its `outN` nipple.
    let row_height = t.sizes.port_vertical_spacing;

    // Column header: the OUT port column starts with an "OUT" label that takes
    // up the top strip, so without a matching header here the slot rows would
    // sit half a row above their nipples. This 8px header restores alignment.
    ui.label(
        egui::RichText::new("SLOTS")
            .size(8.0)
            .color(t.colors.text_dim),
    );

    for slot in 0u8..synth_modules::script_module::SCRIPT_MODULE_OUTPUTS as u8 {
        ui.horizontal(|ui| {
            // Pin the row to the port pitch (24px) and centre its contents, so
            // the rows neither drift from nor sit off-centre to the nipples.
            ui.set_min_height(row_height);
            caption(
                ui,
                format!("out{}", slot + 1),
                CaptionTone::Color(accent_color),
            );
            let installed = state.slot_scripts.get(&slot);
            let (preview, color) = match installed {
                Some(src) => (script_preview(src), t.colors.text_secondary),
                None => ("— empty —".to_string(), t.colors.text_dim),
            };
            caption(ui, preview, CaptionTone::Color(color));

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
        state.script_editor = Some(super::module_panel::ScriptEditorState {
            slot,
            draft,
            ..Default::default()
        });
    }

    draw_slot_expression_editor(ui, state, script_graph, catalog, &mut mod_script_actions);

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
/// Human-readable label for a module category (used in module hover tooltips).
fn category_label(category: ModuleCategory) -> &'static str {
    match category {
        ModuleCategory::Oscillator => "Oscillator",
        ModuleCategory::Filter => "Filter",
        ModuleCategory::Envelope => "Envelope",
        ModuleCategory::LFO => "LFO",
        ModuleCategory::Amplifier => "Amplifier",
        ModuleCategory::Effect => "Effect",
        ModuleCategory::Utility => "Utility",
        ModuleCategory::Sampler => "Sampler",
        ModuleCategory::Sequencer => "Sequencer",
        ModuleCategory::Mixer => "Mixer",
        ModuleCategory::Output => "Output",
        ModuleCategory::Visualizer => "Visualizer",
        ModuleCategory::PhysicalModeling => "Physical Modeling",
    }
}

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
            .push(Connection::new(amp, "out_l", out, "in_l"));

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
            .push(Connection::new(amp, "out_l", out, "in_l"));

        let pending = |module, direction| PendingConnection {
            from_module: module,
            from_port: "p".into(),
            from_position: Pos2::ZERO,
            from_type: WidgetPortType::Audio,
            from_direction: direction,
            current_pos: Pos2::ZERO,
        };

        // Dragging from `out`'s OUTPUT blocks its ancestors (amp, osc) + itself.
        editor.start_wire_drag(pending(out, WidgetPortDirection::Output));
        editor.recompute_drag_cycle_blocked();
        assert_eq!(
            editor.drag_cycle_blocked,
            HashSet::from([out, amp, osc]),
            "output drag blocks ancestors"
        );

        // Dragging from `osc`'s INPUT blocks its descendants (amp, out) + itself.
        editor.start_wire_drag(pending(osc, WidgetPortDirection::Input));
        editor.recompute_drag_cycle_blocked();
        assert_eq!(
            editor.drag_cycle_blocked,
            HashSet::from([osc, amp, out]),
            "input drag blocks descendants"
        );

        // Dragging from `osc`'s OUTPUT has no ancestors — only the self-loop.
        editor.start_wire_drag(pending(osc, WidgetPortDirection::Output));
        editor.recompute_drag_cycle_blocked();
        assert_eq!(editor.drag_cycle_blocked, HashSet::from([osc]));

        // No drag → empty.
        editor.canvas_interaction = CanvasInteraction::Idle;
        editor.recompute_drag_cycle_blocked();
        assert!(editor.drag_cycle_blocked.is_empty());
    }

    /// A Script-module panel carrying `(0-based slot, source)` YAMS scripts.
    fn script_panel(instance: u16, slots: &[(u8, &str)]) -> (ModuleId, ModulePanelState) {
        let id = ModuleId::new(ModuleType::Script, instance);
        let mut state = ModulePanelState::new(id, Pos2::ZERO);
        for (slot, src) in slots {
            state.slot_scripts.insert(*slot, (*src).to_string());
        }
        (id, state)
    }

    fn graph_of(panels: Vec<(ModuleId, ModulePanelState)>) -> ScriptDepGraph {
        ScriptDepGraph::from_panels_cached(&panels.into_iter().collect(), &mut HashMap::new())
    }

    /// Run the loop warning for a live `draft` (compiles it to refs first), as the
    /// editor does each frame.
    fn warn(graph: &ScriptDepGraph, module: ModuleId, slot: u8, draft: &str) -> Option<String> {
        graph.cycle_warning(module, slot, &script_output_refs(draft))
    }

    /// A script reading its own output is flagged as a self-reference. The draft
    /// drives the check, so it fires before the script is even installed.
    #[test]
    fn cycle_warning_flags_self_reference() {
        let scr = ModuleId::new(ModuleType::Script, 1);
        let graph = graph_of(vec![script_panel(1, &[])]);
        let warning = warn(&graph, scr, 0, "src me = scr-1.out1\nout = me * 0.5")
            .expect("self-reference must warn");
        assert!(warning.contains("feeds back on itself"), "{warning}");
    }

    /// scr-1.out1 → scr-2.out1 → scr-1.out1 is a two-node cycle. scr-2's edge is
    /// installed; scr-1's edge comes from the live draft.
    #[test]
    fn cycle_warning_flags_two_script_cycle() {
        let scr1 = ModuleId::new(ModuleType::Script, 1);
        let graph = graph_of(vec![
            script_panel(1, &[]),
            script_panel(2, &[(0, "src a = scr-1.out1\nout = a")]),
        ]);
        let warning =
            warn(&graph, scr1, 0, "src b = scr-2.out1\nout = b").expect("cycle must warn");
        assert!(warning.contains("feedback cycle"), "{warning}");
        assert!(
            warning.contains("scr-1.out1") && warning.contains("scr-2.out1"),
            "{warning}"
        );
    }

    /// An acyclic chain (scr-1 reads scr-2, scr-2 reads nothing back) is fine.
    #[test]
    fn cycle_warning_silent_on_acyclic_chain() {
        let scr1 = ModuleId::new(ModuleType::Script, 1);
        let graph = graph_of(vec![
            script_panel(1, &[]),
            script_panel(2, &[(0, "out = 0.5")]),
        ]);
        assert!(warn(&graph, scr1, 0, "src b = scr-2.out1\nout = b").is_none());
    }

    /// Referencing a non-script source (an LFO) never forms a script cycle, so no
    /// warning — only `scr-N.outM` edges count.
    #[test]
    fn cycle_warning_ignores_non_script_sources() {
        let scr1 = ModuleId::new(ModuleType::Script, 1);
        let graph = graph_of(vec![script_panel(1, &[])]);
        assert!(warn(&graph, scr1, 0, "src l = lfo-1.out\nout = l").is_none());
    }

    /// A Mod Matrix slot exposes no addressable output, so it can never close a
    /// loop even when its script reads one — the warning is Script-module only.
    #[test]
    fn cycle_warning_only_for_script_modules() {
        let mm = ModuleId::new(ModuleType::ModMatrix, 1);
        let graph = graph_of(vec![script_panel(1, &[(0, "src m = scr-1.out1\nout = m")])]);
        assert!(warn(&graph, mm, 0, "src s = scr-1.out1\nout = s").is_none());
    }

    #[test]
    fn insert_at_cursor_splices_at_index() {
        let mut s = "out = ".to_string();
        // Cursor at end → append; returns the new caret past the text.
        let caret = insert_at_cursor(&mut s, Some((6, 6)), "velocity");
        assert_eq!(s, "out = velocity");
        assert_eq!(caret, 14);
        // `None` cursor falls back to the end.
        let mut s2 = "abc".to_string();
        assert_eq!(insert_at_cursor(&mut s2, None, "X"), 4);
        assert_eq!(s2, "abcX");
        // An out-of-range index clamps to the end rather than panicking.
        let mut s3 = "ab".to_string();
        insert_at_cursor(&mut s3, Some((99, 99)), "!");
        assert_eq!(s3, "ab!");
        // A non-empty range REPLACES the selection (like typing over it).
        let mut s4 = "out = lfo".to_string();
        let caret = insert_at_cursor(&mut s4, Some((6, 9)), "velocity");
        assert_eq!(s4, "out = velocity");
        assert_eq!(caret, 14);
    }

    #[test]
    fn derive_src_var_sanitizes_address() {
        assert_eq!(derive_src_var("lfo-1.out"), "lfo1_out");
        assert_eq!(derive_src_var("scr-2.out1"), "scr2_out1");
    }

    #[test]
    fn existing_binding_var_finds_reusable_binding() {
        let draft = "src lfo = lfo-1.out\nout = lfo * 0.5";
        assert_eq!(
            existing_binding_var(draft, "lfo-1.out").as_deref(),
            Some("lfo")
        );
        assert_eq!(existing_binding_var(draft, "env-1.out"), None);
        // A trailing comment on the binding line is ignored when matching.
        let commented = "src lfo = lfo-1.out  # main lfo\nout = lfo";
        assert_eq!(
            existing_binding_var(commented, "lfo-1.out").as_deref(),
            Some("lfo")
        );
    }

    #[test]
    fn insert_module_source_prepends_binding_then_inserts_var() {
        // No existing binding: prepend `src <var> = <addr>` and insert the var.
        let mut draft = "out = ".to_string();
        let caret = insert_module_source(&mut draft, Some((6, 6)), "lfo-1.out");
        assert_eq!(draft, "src lfo1_out = lfo-1.out\nout = lfo1_out");
        // Caret lands past the inserted variable (shifted by the prepended line).
        assert_eq!(&draft[..caret], "src lfo1_out = lfo-1.out\nout = lfo1_out");
    }

    #[test]
    fn insert_module_source_reuses_existing_binding() {
        // A binding for this address already exists → reuse its var, no prepend.
        let mut draft = "src l = lfo-1.out\nout = ".to_string();
        let end = draft.chars().count();
        insert_module_source(&mut draft, Some((end, end)), "lfo-1.out");
        assert_eq!(draft, "src l = lfo-1.out\nout = l");
    }

    #[test]
    fn insert_module_source_avoids_name_collision() {
        // The derived name `lfo1_out` is already taken by a *different* address;
        // the new binding must get a suffix so the script stays compilable.
        let mut draft = "src lfo1_out = env-2.out\nout = ".to_string();
        let end = draft.chars().count();
        insert_module_source(&mut draft, Some((end, end)), "lfo-1.out");
        assert!(draft.contains("src lfo1_out_2 = lfo-1.out"), "{draft}");
        assert!(draft.ends_with("lfo1_out_2"), "{draft}");
    }
}
