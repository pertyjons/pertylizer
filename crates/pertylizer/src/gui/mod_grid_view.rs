//! The Mod Grid view — pool list + Scene node canvas.
//!
//! A left panel lists the project's pooled control-rate modulator graphs
//! (create / duplicate / rename / delete, GLOBAL/TRACK scope chips) and a
//! central `egui::Scene` node canvas edits the selected graph: source nodes
//! (hosted control-rate modules + the cheap grid-native sources) with an OUT
//! port, and Target routing-sink nodes with an IN port, wired by drag-to-connect
//! cables. A Target node picks the automation target it writes and its depth.
//!
//! Graphs live in the shared `Song` pool; every edit goes through `song.write()`
//! and the graph's own validating editors (`try_insert_node`, `try_connect`,
//! `remove_node`), then pushes a whole-graph snapshot undo entry
//! ([`UndoAction::SetModGraph`]). The GUI's per-frame sync rebuilds the running
//! engine instances off the generation bump.
//!
//! Nodes expose their real named ports: a hosted module shows its descriptor
//! outputs and (non-MIDI) control inputs, cheap sources a single `out`, Target
//! sinks a single `in`. Cables can wire any source's output to a Target's input
//! or to a hosted module's control input (module→module modulation). A cheap
//! source (Macro/Transport/MidiCc/AudioTap) can only reach a Target — the engine
//! can't inject cheap values into the DSP graph.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Vec2};
use egui_remixicon::icons as ri;

use synth_core::{ModuleCategory, ModuleDescriptor, ModuleType, ModuleWidth};
use synth_engine::ModuleId;
use synth_sequencer::{
    AutoInstrumentParam, AutomationTarget, GlobalParam, InstrumentId, MAX_MOD_GRID_NODES,
    ModConnection, ModGraph, ModGraphId, ModGraphScope, ModNodeConfig, ModNodeId, ModTarget,
    ModuleNode, TrackId, TrackParam, TransportNode, TransportSource,
};

use crate::gui::auto_layout::{LayoutConnection, ModuleInfo, calculate_free_flow_layout};
use crate::gui::list_panel;
use crate::gui::module_panel::category_color;
use crate::gui::node_canvas;
use crate::gui::scene_canvas;
use crate::gui::theme::theme;
use crate::gui::toolbar;
use crate::gui::widgets::{
    ModMarkers, ModuleCard, ModuleCardGeometry, ModuleColumn, ModulePort, ModulePortEndpoint,
    PortWidget, WidgetPortDirection, WidgetPortType, danger_button, dim_label, draw_cable,
    draw_cable_dragging, draw_cable_highlighted, draw_module_port_column, draw_module_port_layout,
    draw_parameter_grid, expose, icon_button, module_port_accessible_label, tree_picker_button,
};
use crate::undo::{UndoAction, UndoManager};

/// Card height before the first frame has measured the real one.
const DEFAULT_NODE_HEIGHT: f32 = 120.0;
/// Compact graph-card margin shared with Note Grid.
const NODE_MARGIN: f32 = 6.0;
/// Hit radius (world units) for dropping a dragged wire on a port.
const WIRE_DROP_RADIUS: f32 = 16.0;

/// A node's port endpoint: node id + interned port name + direction. Copy so the
/// shared wire FSM can key on it. `is_output` distinguishes the two directions
/// of a same-named port (rare) and orients wiring.
type PortRef = (ModNodeId, synth_core::PortName, bool);
type WireEvent = node_canvas::WireEvent<PortRef>;

/// Immutable domain context needed to present one side of a Mod Grid node.
struct ModPortColumn<'a> {
    graph: &'a ModGraph,
    node_id: ModNodeId,
    config: &'a ModNodeConfig,
    ports: &'a [(synth_core::PortName, bool, String)],
}

impl ModulePortEndpoint for PortRef {
    fn widget_port_type(self) -> WidgetPortType {
        WidgetPortType::Control
    }
}

/// The ports a node exposes: `(name, is_output, label)`. Hosted modules take
/// theirs from the descriptor (outputs + non-MIDI inputs); cheap sources expose
/// a single `out`; Target sinks a single `in`.
fn node_ports(
    state: &mut ModGridViewState,
    config: &ModNodeConfig,
) -> Vec<(synth_core::PortName, bool, String)> {
    match config {
        ModNodeConfig::Module(m) => {
            let desc = state.descriptors.entry(m.module_type).or_insert_with(|| {
                crate::module_factory::create_voice_module(m.module_type).map(|(_, d)| d)
            });
            let Some(desc) = desc.as_ref() else {
                return vec![(synth_core::PortName::OUT, true, "out".into())];
            };
            desc.ports
                .iter()
                .filter(|p| {
                    p.direction == synth_core::PortDirection::Output
                        || p.port_type != synth_core::PortType::Midi
                })
                .map(|p| {
                    (
                        p.name,
                        p.direction == synth_core::PortDirection::Output,
                        p.label.clone(),
                    )
                })
                .collect()
        }
        ModNodeConfig::Target(_) => vec![(synth_core::PortName::IN, false, "in".into())],
        // Cheap grid-native sources: a single value output.
        _ => vec![(synth_core::PortName::OUT, true, "out".into())],
    }
}

/// Per-session view state (selection, search, cameras, node positions).
#[derive(Default)]
pub(crate) struct ModGridViewState {
    pub(crate) selected: Option<ModGraphId>,
    search: String,
    /// Measured card sizes from the previous frame (drag hit-testing).
    sizes: HashMap<(ModGraphId, ModNodeId), Vec2>,
    /// Per-graph Scene camera.
    scene_rects: HashMap<ModGraphId, Rect>,
    /// Port anchor positions recorded while drawing (world coords).
    port_positions: HashMap<PortRef, Pos2>,
    pending_wire: Option<node_canvas::WireDrag<PortRef>>,
    /// Pre-gesture graph snapshot for coalescing config-edit undo.
    edit_baseline: Option<(ModGraphId, ModGraph)>,
    /// Last rejected edit, shown in the canvas header until the next success.
    last_error: Option<String>,
    /// Rename-in-flight in the pool kebab: `(graph, buffer)`.
    rename: Option<(ModGraphId, String)>,
    /// Open background context menu: `(world position, cable under pointer)`.
    bg_menu: Option<(Pos2, Option<ModConnection>)>,
    /// Cached module descriptors, keyed by type, so the per-node param editor
    /// doesn't rebuild a module every frame.
    descriptors: HashMap<ModuleType, Option<ModuleDescriptor>>,
    /// The song's tracks `(id, name)`, snapshotted each frame — for the Audio
    /// Tap source picker and (later) target pickers.
    tracks: Vec<(TrackId, String)>,
    /// The instruments `(id, name)`, snapshotted each frame from the app (the
    /// `Song` has no instrument names) — for the Module-target picker.
    instruments: Vec<(InstrumentId, String)>,
    /// Per-instrument automatable module-param targets from the live descriptors
    /// — the per-module submenus of the Target picker (shared enumeration, so it
    /// matches MCP + the pattern-view lane picker). Refreshed only when the caller
    /// rebuilds it (the module graph changed), not every frame.
    module_groups: HashMap<InstrumentId, Vec<crate::module_targets::ModuleTargetGroup>>,
    /// The Mod Grid pre-pass CPU load (fraction of the buffer budget), shown in
    /// the canvas header. Snapshotted from the engine each frame.
    cpu_mod_grid: f32,
}

/// A deferred edit to the selected graph, applied after the snapshot is drawn.
enum GraphEdit {
    AddNode(ModNodeConfig, Pos2),
    RemoveNode(ModNodeId),
    /// Live config edit; undo coalesced via `edit_baseline`.
    SetNode(ModNodeId, ModNodeConfig),
    Connect(ModConnection),
    Disconnect(ModConnection),
}

/// Draw the whole Mod Grid view: left pool panel + central node canvas.
pub(crate) fn draw_mod_grid_view(
    ui: &mut egui::Ui,
    song: &Arc<synth_sequencer::SharedSong>,
    state: &mut ModGridViewState,
    undo_manager: &mut UndoManager,
    instruments: &[(InstrumentId, String)],
    module_groups: Option<HashMap<InstrumentId, Vec<crate::module_targets::ModuleTargetGroup>>>,
    cpu_mod_grid: f32,
) {
    // Snapshot the instrument list for the Module-target picker. The per-instrument
    // module targets are refreshed only when the caller rebuilds them (the module
    // graph changed); otherwise the previous frame's `state.module_groups` stands.
    state.instruments.clear();
    state.instruments.extend_from_slice(instruments);
    if let Some(groups) = module_groups {
        state.module_groups = groups;
    }
    state.cpu_mod_grid = cpu_mod_grid;

    let selected_at_entry = state.selected;
    let pool = draw_pool_panel(ui, song, state, undo_manager);

    if let Some(pool) = pool
        && state.selected.is_none_or(|id| !pool.contains(&id))
    {
        state.selected = pool.first().copied();
    }
    // Node ids are graph-local: a wire/menu started on another graph must not
    // survive a selection change.
    if state.selected != selected_at_entry {
        state.pending_wire = None;
        state.bg_menu = None;
    }

    egui::CentralPanel::default().show(ui, |ui| {
        let Some(graph_id) = state.selected else {
            ui.vertical_centered(|ui| {
                ui.add_space(48.0);
                ui.label(
                    RichText::new("No mod graphs yet — create one to modulate targets live.")
                        .size(theme().fonts.size_normal)
                        .color(theme().colors.text_secondary),
                );
            });
            return;
        };
        draw_graph_canvas(ui, song, state, undo_manager, graph_id);
    });
}

// ============================================================================
// Left panel — the pool
// ============================================================================

enum PoolEdit {
    Create,
    Duplicate(ModGraphId),
    Delete(ModGraphId),
    Rename(ModGraphId, String),
}

/// One pool row snapshot: id, name, scope, usage count.
type PoolRow = (ModGraphId, String, ModGraphScope, usize);

fn draw_pool_panel(
    ui: &mut egui::Ui,
    song: &Arc<synth_sequencer::SharedSong>,
    state: &mut ModGridViewState,
    undo_manager: &mut UndoManager,
) -> Option<Vec<ModGraphId>> {
    let rows: Option<Vec<PoolRow>> = song.try_read().map(|s| {
        s.mod_graphs()
            .map(|g| (g.id, g.name.clone(), g.scope, s.mod_graph_usage(g.id)))
            .collect()
    });
    let snapshot_ok = rows.is_some();
    let rows = rows.unwrap_or_default();

    let mut edit: Option<PoolEdit> = None;
    let mut clicked: Option<ModGraphId> = None;

    egui::Panel::left("mod_grid_pool_panel")
        .default_size(list_panel::DEFAULT_WIDTH)
        .min_size(list_panel::MIN_WIDTH)
        .show(ui, |ui| {
            egui::Panel::top("mod_grid_pool_head").show(ui, |ui| {
                if list_panel::header(ui, ri::PULSE_LINE, "Mod Graphs", "New mod graph") {
                    edit = Some(PoolEdit::Create);
                }
                list_panel::search_box(ui, &mut state.search);
            });

            let needle = state.search.to_lowercase();
            let shown: Vec<_> = rows
                .iter()
                .filter(|(_, name, ..)| needle.is_empty() || name.to_lowercase().contains(&needle))
                .collect();

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if shown.is_empty() {
                        list_panel::empty(ui, "No mod graphs");
                    }
                    for (id, name, scope, usage) in shown {
                        let is_active = state.selected == Some(*id);
                        ui.horizontal(|ui| {
                            // A GLOBAL/TRACK scope chip in the row.
                            let (chip, chip_col) = match scope {
                                ModGraphScope::Global => ("GLOBAL", theme().colors.accent_cyan),
                                ModGraphScope::Track => ("TRACK", theme().colors.accent_green),
                            };
                            ui.label(RichText::new(chip).size(9.0).color(chip_col));

                            let rename = &mut state.rename;
                            let response = list_panel::row(
                                ui,
                                is_active,
                                name,
                                list_panel::row_text_color(is_active, true),
                                |ui| {
                                    if rename.as_ref().is_none_or(|(rid, _)| rid != id) {
                                        *rename = Some((*id, name.clone()));
                                    }
                                    if let Some((_, buf)) = rename.as_mut() {
                                        let inline = crate::gui::widgets::inline_editable_text(
                                            ui,
                                            buf,
                                            false,
                                            |t| t,
                                        );
                                        if inline.ended {
                                            let new_name = buf.trim();
                                            if !new_name.is_empty() && new_name != name {
                                                edit = Some(PoolEdit::Rename(*id, new_name.into()));
                                                ui.close();
                                            }
                                            *rename = None;
                                        }
                                    }
                                    ui.separator();
                                    if ui.button((ri::FILE_COPY_LINE, "Duplicate")).clicked() {
                                        edit = Some(PoolEdit::Duplicate(*id));
                                        ui.close();
                                    }
                                    ui.separator();
                                    if danger_button(ui, format!("{} Delete…", ri::DELETE_BIN_LINE))
                                        .clicked()
                                    {
                                        edit = Some(PoolEdit::Delete(*id));
                                        ui.close();
                                    }
                                },
                            );
                            let tip = match scope {
                                ModGraphScope::Global => {
                                    "Global — one always-on instance".to_string()
                                }
                                ModGraphScope::Track => format!(
                                    "Track-scoped — assigned to {usage} track{}",
                                    if *usage == 1 { "" } else { "s" }
                                ),
                            };
                            if response.on_hover_text(tip).clicked() && !is_active {
                                clicked = Some(*id);
                            }
                        });
                    }
                });
        });

    if let Some(id) = clicked {
        state.selected = Some(id);
    }

    let mut pool_ids: Option<Vec<ModGraphId>> =
        snapshot_ok.then(|| rows.into_iter().map(|(id, ..)| id).collect());

    match edit {
        Some(PoolEdit::Create) => {
            let snapshot = {
                let mut song_w = song.write();
                let n = song_w.mod_graphs().count() + 1;
                let id = song_w.create_mod_graph(format!("Mod {n}"));
                song_w.mod_graph(id).cloned()
            };
            if let Some(graph) = snapshot {
                state.selected = Some(graph.id);
                if let Some(pool) = &mut pool_ids {
                    pool.push(graph.id);
                }
                undo_manager.push(UndoAction::SetModGraph {
                    graph_id: graph.id,
                    old: None,
                    new: Some(graph),
                });
            }
        }
        Some(PoolEdit::Duplicate(src_id)) => {
            let snapshot = song.write().duplicate_mod_graph(src_id);
            if let Some(graph) = snapshot {
                state.selected = Some(graph.id);
                if let Some(pool) = &mut pool_ids {
                    pool.push(graph.id);
                }
                undo_manager.push(UndoAction::SetModGraph {
                    graph_id: graph.id,
                    old: None,
                    new: Some(graph),
                });
            }
        }
        Some(PoolEdit::Delete(id)) => {
            let removed = song.write().remove_mod_graph(id);
            if let Some(graph) = removed {
                undo_manager.push(UndoAction::SetModGraph {
                    graph_id: id,
                    old: Some(graph),
                    new: None,
                });
            }
        }
        Some(PoolEdit::Rename(id, new_name)) => {
            with_graph_undo(song, undo_manager, id, |graph| {
                if !new_name.trim().is_empty() {
                    graph.name = new_name.trim().to_owned();
                }
            });
        }
        None => {}
    }

    pool_ids
}

/// Mutate one graph under the write lock and push a snapshot undo entry when the
/// closure actually changed it.
fn with_graph_undo(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut UndoManager,
    id: ModGraphId,
    mutate: impl FnOnce(&mut ModGraph),
) {
    let Some((before, after)) = ({
        let mut song_w = song.write();
        song_w.mod_graph_mut(id).map(|graph| {
            let before = graph.clone();
            mutate(graph);
            (before, graph.clone())
        })
    }) else {
        return;
    };
    if before != after {
        undo_manager.push(UndoAction::SetModGraph {
            graph_id: id,
            old: Some(before),
            new: Some(after),
        });
    }
}

// ============================================================================
// Canvas
// ============================================================================

fn draw_graph_canvas(
    ui: &mut egui::Ui,
    song: &Arc<synth_sequencer::SharedSong>,
    state: &mut ModGridViewState,
    undo_manager: &mut UndoManager,
    graph_id: ModGraphId,
) {
    let t = theme();

    // Snapshot the graph + the track list (for scope assignment + tap picker)
    // under one lock.
    let Some((graph, tracks)) = song.try_read().and_then(|s| {
        let graph = s.mod_graph(graph_id)?.clone();
        let tracks: Vec<(TrackId, String)> =
            s.tracks().map(|tr| (tr.id, tr.name.clone())).collect();
        Some((graph, tracks))
    }) else {
        dim_label(ui, "Graph unavailable…");
        return;
    };
    state.tracks.clone_from(&tracks);

    // Context bar: name + scope toggle + (Track) assignment editor.
    let mut scope_change: Option<ModGraphScope> = None;
    let mut assign_change: Option<Vec<TrackId>> = None;
    let mut auto_layout = false;
    toolbar::top(ui, "mod_grid_toolbar", |ui| {
        ui.label(
            RichText::new(format!("{} {}", ri::PULSE_LINE, graph.name))
                .size(t.fonts.size_heading)
                .color(t.colors.text_primary),
        );

        // Mod Grid pre-pass CPU load (all running instances), as a share of the
        // per-buffer budget. Amber past 25%, red past 50%.
        let cpu = state.cpu_mod_grid;
        let cpu_color = if cpu > 0.5 {
            t.colors.accent_red
        } else if cpu > 0.25 {
            t.colors.accent_yellow
        } else {
            t.colors.text_dim
        };
        ui.label(
            RichText::new(format!("{} {:.1}%", ri::CPU_LINE, cpu * 100.0))
                .size(t.fonts.size_small)
                .color(cpu_color),
        )
        .on_hover_text("Mod Grid CPU — the control-rate pre-pass for every running instance");

        // Scope segmented toggle.
        let mut is_track = graph.scope == ModGraphScope::Track;
        if ui
            .selectable_label(!is_track, "Global")
            .on_hover_text("One always-on instance")
            .clicked()
        {
            is_track = false;
            scope_change = Some(ModGraphScope::Global);
        }
        if ui
            .selectable_label(is_track, "Track")
            .on_hover_text("One instance per assigned track")
            .clicked()
        {
            scope_change = Some(ModGraphScope::Track);
        }

        // Track assignment editor (Track scope only).
        if graph.scope == ModGraphScope::Track {
            let label = if graph.assigned_tracks.is_empty() {
                format!("{} assign tracks", ri::ADD_LINE)
            } else {
                format!(
                    "{} {} tracks",
                    ri::CHECKBOX_MULTIPLE_LINE,
                    graph.assigned_tracks.len()
                )
            };
            ui.menu_button(label, |ui| {
                if tracks.is_empty() {
                    ui.label(RichText::new("no tracks").color(t.colors.text_dim));
                }
                let mut next = graph.assigned_tracks.clone();
                for (tid, name) in &tracks {
                    let mut on = next.contains(tid);
                    if ui.checkbox(&mut on, name).changed() {
                        if on {
                            if !next.contains(tid) {
                                next.push(*tid);
                            }
                        } else {
                            next.retain(|x| x != tid);
                        }
                        assign_change = Some(next.clone());
                    }
                }
            });
        }

        if let Some(err) = &state.last_error {
            ui.label(
                RichText::new(format!("{} {err}", ri::ERROR_WARNING_LINE))
                    .color(t.colors.accent_red),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button((
                    RichText::new(ri::LAYOUT_GRID_FILL).color(t.colors.text_secondary),
                    RichText::new("Auto Layout").color(t.colors.text_secondary),
                ))
                .on_hover_text("Tidy the node layout (discards manual positions)")
                .clicked()
            {
                auto_layout = true;
            }
        });
    });

    if let Some(scope) = scope_change {
        let before = graph.clone();
        let after = {
            let mut song_w = song.write();
            song_w.set_mod_graph_scope(graph_id, scope);
            song_w.mod_graph(graph_id).cloned()
        };
        if let Some(after) = after
            && before != after
        {
            undo_manager.push(UndoAction::SetModGraph {
                graph_id,
                old: Some(before),
                new: Some(after),
            });
        }
    }
    if let Some(tracks) = assign_change {
        let before = graph.clone();
        let after = {
            let mut song_w = song.write();
            song_w.assign_mod_graph(graph_id, &tracks);
            song_w.mod_graph(graph_id).cloned()
        };
        if let Some(after) = after
            && before != after
        {
            undo_manager.push(UndoAction::SetModGraph {
                graph_id,
                old: Some(before),
                new: Some(after),
            });
        }
    }

    let mut graph = graph;
    if auto_layout {
        if let Some(g) = song.write().mod_graph_mut(graph_id) {
            g.node_positions.clear();
        }
        graph.node_positions.clear();
        state.scene_rects.remove(&graph_id);
    }

    let positions = layout_positions(state, &graph);
    let visible_rect = ui.available_rect_before_wrap();
    let mut scene_rect = state
        .scene_rects
        .get(&graph_id)
        .copied()
        .unwrap_or_else(|| {
            scene_canvas::initial_scene_rect(node_rects(state, &positions, &graph), visible_rect)
        });

    let mut edit: Option<GraphEdit> = None;
    let mut any_dragged = false;
    let mut wire_events: Vec<WireEvent> = Vec::new();
    let mut moved: Option<(ModNodeId, Pos2)> = None;

    scene_canvas::scene().show(ui, &mut scene_rect, |ui| {
        let world_rect = ui.clip_rect();
        let canvas_bg = ui.interact(world_rect, ui.id().with("mod_grid_bg"), Sense::click());
        expose(&canvas_bg, egui::WidgetType::Panel, "mod grid canvas", None);
        scene_canvas::draw_grid(ui, world_rect);

        if graph.nodes.is_empty() {
            ui.painter().text(
                world_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Right-click to add nodes",
                egui::FontId::proportional(theme().fonts.size_normal),
                theme().colors.text_dim,
            );
        }

        let hovered_cable = draw_cables(ui, state, &graph);

        state.port_positions.clear();
        for (&node_id, config) in &graph.nodes {
            draw_node(
                ui,
                state,
                &graph,
                &positions,
                node_id,
                config,
                &mut edit,
                &mut moved,
                &mut any_dragged,
                &mut wire_events,
            );
        }

        node_canvas::resolve_wire_events(
            ui,
            &mut state.pending_wire,
            node_canvas::DropTargets {
                anchors: &state.port_positions,
                world_rect,
                drop_radius: WIRE_DROP_RADIUS,
            },
            wire_events,
            |from, to| open_connection(&graph, from, to),
            |connection| propose_edit(&mut edit, GraphEdit::Connect(connection)),
        );

        if let Some(pending) = &state.pending_wire
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let world = scene_canvas::screen_to_world(ui, pointer);
            draw_cable_dragging(ui.painter(), pending.from_pos, world, control_color());
        }

        if canvas_bg.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.pending_wire = None;
        }

        draw_bg_context_menu(ui, state, &canvas_bg, &graph, hovered_cable, &mut edit);
    });
    state.scene_rects.insert(graph_id, scene_rect);

    if let Some(action) = scene_canvas::view_controls(
        ui,
        egui::Id::new(("mod_grid_view_controls", graph_id.0)),
        visible_rect,
    ) {
        let current = state
            .scene_rects
            .get(&graph_id)
            .copied()
            .unwrap_or(scene_rect);
        let target = scene_canvas::apply_view_action(action, current, visible_rect, || {
            scene_canvas::initial_scene_rect(node_rects(state, &positions, &graph), visible_rect)
        });
        state.scene_rects.insert(graph_id, target);
    }

    // Persist a live node drag (layout metadata — no undo).
    if let Some((node_id, pos)) = moved
        && let Some(g) = song.write().mod_graph_mut(graph_id)
    {
        g.node_positions.insert(
            node_id,
            synth_sequencer::NodePosition { x: pos.x, y: pos.y },
        );
    }

    apply_graph_edit(song, undo_manager, state, &graph, edit);
    finalize_config_undo(song, undo_manager, state, any_dragged);
}

/// World rects of every laid-out node (camera framing).
fn node_rects<'a>(
    state: &'a ModGridViewState,
    positions: &'a HashMap<ModNodeId, Pos2>,
    graph: &'a ModGraph,
) -> impl Iterator<Item = Rect> + 'a {
    graph.nodes.keys().filter_map(move |id| {
        let pos = positions.get(id)?;
        let config = graph.nodes.get(id)?;
        let size = state
            .sizes
            .get(&(graph.id, *id))
            .copied()
            .unwrap_or_else(|| Vec2::new(node_geometry(config).outer_width, DEFAULT_NODE_HEIGHT));
        Some(Rect::from_min_size(*pos, size))
    })
}

/// Effective positions from the shared Sugiyama flow layout, overridden by any
/// manually persisted positions. Unlike Rack, no category zones are imposed.
fn layout_positions(state: &ModGridViewState, graph: &ModGraph) -> HashMap<ModNodeId, Pos2> {
    let mut domain_to_layout = HashMap::with_capacity(graph.nodes.len());
    let mut layout_to_domain = HashMap::with_capacity(graph.nodes.len());
    let modules: Vec<ModuleInfo> = graph
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, (&id, config))| {
            let instance = u16::try_from(index).ok()?;
            let layout_id = ModuleId::new(ModuleType::Oscillator, instance);
            domain_to_layout.insert(id, layout_id);
            layout_to_domain.insert(layout_id, id);
            let geometry = node_geometry(config);
            let size = state
                .sizes
                .get(&(graph.id, id))
                .copied()
                .unwrap_or_else(|| Vec2::new(geometry.outer_width, DEFAULT_NODE_HEIGHT));
            Some(ModuleInfo {
                id: layout_id,
                category: ModuleCategory::Oscillator,
                size,
            })
        })
        .collect();
    let connections: Vec<LayoutConnection> = graph
        .connections
        .iter()
        .filter_map(|connection| {
            Some(LayoutConnection {
                from_module: *domain_to_layout.get(&connection.from)?,
                to_module: *domain_to_layout.get(&connection.to)?,
            })
        })
        .collect();
    let flow = calculate_free_flow_layout(&modules, &connections);
    let mut positions: HashMap<ModNodeId, Pos2> = flow
        .positions
        .into_iter()
        .filter_map(|(layout_id, position)| Some((*layout_to_domain.get(&layout_id)?, position)))
        .collect();
    for (&id, position) in &graph.node_positions {
        positions.insert(id, Pos2::new(position.x, position.y));
    }
    positions
}

// ============================================================================
// Node cards
// ============================================================================

fn node_width(config: &ModNodeConfig) -> ModuleWidth {
    match config {
        // Keep hosted nodes aligned with the width declared by their Rack
        // descriptors. These are the module types offered by the Mod Grid.
        ModNodeConfig::Module(module) => match module.module_type {
            ModuleType::Lfo => ModuleWidth::Large,
            ModuleType::Mseg => ModuleWidth::ExtraLarge,
            ModuleType::EnvelopeFollower | ModuleType::Euclidean | ModuleType::RandomGates => {
                ModuleWidth::Medium
            }
            _ => ModuleWidth::Medium,
        },
        // Macro and Target contain full-width text/picker controls; Small and
        // ExtraSmall cannot contain them without overflowing the card body.
        ModNodeConfig::Macro(_) | ModNodeConfig::Target(_) => ModuleWidth::Medium,
        ModNodeConfig::Transport(_) | ModNodeConfig::MidiCc(_) | ModNodeConfig::AudioTap(_) => {
            ModuleWidth::Small
        }
    }
}

fn node_geometry(config: &ModNodeConfig) -> ModuleCardGeometry {
    ModuleCardGeometry::ported(node_width(config), NODE_MARGIN)
}

#[allow(clippy::too_many_arguments)]
fn draw_node(
    ui: &mut egui::Ui,
    state: &mut ModGridViewState,
    graph: &ModGraph,
    positions: &HashMap<ModNodeId, Pos2>,
    node_id: ModNodeId,
    config: &ModNodeConfig,
    edit: &mut Option<GraphEdit>,
    moved: &mut Option<(ModNodeId, Pos2)>,
    any_dragged: &mut bool,
    wire_events: &mut Vec<WireEvent>,
) {
    let key = (graph.id, node_id);
    let Some(mut pos) = positions.get(&node_id).copied() else {
        return;
    };
    let geometry = node_geometry(config);
    let size = state
        .sizes
        .get(&key)
        .copied()
        .unwrap_or_else(|| Vec2::new(geometry.outer_width, DEFAULT_NODE_HEIGHT));

    let card_id = egui::Id::new(("mod_grid_node", graph.id.0, node_id.0));
    let node_rect = Rect::from_min_size(pos, size);
    let response = ui.interact(node_rect, card_id.with("card"), Sense::click_and_drag());
    expose(
        &response,
        egui::WidgetType::Button,
        format!("{} node {}", node_name(config), node_id.0),
        None,
    );
    if response.dragged() {
        pos += response.drag_delta();
        *moved = Some((node_id, pos));
    }
    if response.drag_stopped() {
        pos = scene_canvas::snap_to_grid(pos);
        *moved = Some((node_id, pos));
    }
    if response.hovered() && state.pending_wire.is_none() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    let accent = node_accent(config);
    let card = ModuleCard::new(accent)
        .inner_margin(NODE_MARGIN)
        .body_module_width(node_width(config));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id(card_id)
            .max_rect(Rect::from_min_size(
                pos,
                Vec2::new(geometry.outer_width, 600.0),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let title = node_name(config);
    card.show(&mut child, |card| {
        let description = graph.node_descriptions.get(&node_id).cloned();
        card.header(&title, description, false, |ui| {
            if icon_button(ui, ri::CLOSE_LINE, theme().colors.text_dim, "Remove node").clicked() {
                propose_edit(edit, GraphEdit::RemoveNode(node_id));
            }
        });

        let ports = node_ports(state, config);
        draw_module_port_layout(
            card.body(),
            geometry.body_width,
            |column, ui| match column {
                ModuleColumn::Input => {
                    draw_port_column(
                        ui,
                        state,
                        ModPortColumn {
                            graph,
                            node_id,
                            config,
                            ports: &ports,
                        },
                        true,
                        wire_events,
                    );
                }
                ModuleColumn::Body => {
                    let mut cfg = config.clone();
                    edit_node_body(ui, state, &mut cfg, any_dragged);
                    if cfg != *config {
                        propose_edit(edit, GraphEdit::SetNode(node_id, cfg));
                    }
                }
                ModuleColumn::Output => {
                    draw_port_column(
                        ui,
                        state,
                        ModPortColumn {
                            graph,
                            node_id,
                            config,
                            ports: &ports,
                        },
                        false,
                        wire_events,
                    );
                }
            },
        );
    });
    state.sizes.insert(key, child.min_rect().size());
}

/// One side's port column (`is_input` picks IN vs OUT). Draws each of this
/// node's ports on that side as a dot; always reserves the column width so the
/// body stays centred even when a side has no ports.
fn draw_port_column(
    ui: &mut egui::Ui,
    state: &mut ModGridViewState,
    column: ModPortColumn<'_>,
    is_input: bool,
    wire_events: &mut Vec<WireEvent>,
) {
    let direction = if is_input {
        WidgetPortDirection::Input
    } else {
        WidgetPortDirection::Output
    };
    let owner = format!(
        "Mod Grid graph {}, {} node {}",
        column.graph.id.0,
        node_name(column.config),
        column.node_id.0
    );
    let side: Vec<ModulePort<PortRef>> = column
        .ports
        .iter()
        .filter(|(_, is_out, _)| *is_out != is_input)
        .map(|(port, is_output, label)| {
            let display_label = sentence_case(label);
            let endpoint = (column.node_id, *port, *is_output);
            let connected = column.graph.connections.iter().any(|c| {
                if *is_output {
                    c.from == column.node_id
                        && synth_core::PortName::from(c.from_port.as_str()) == *port
                } else {
                    c.to == column.node_id
                        && synth_core::PortName::from(c.to_port.as_str()) == *port
                }
            });
            let highlighted = state.pending_wire.as_ref().is_some_and(|pending| {
                open_connection(column.graph, pending.from, endpoint).is_some()
            });
            let accessible_label = module_port_accessible_label(
                &owner,
                port.as_str(),
                &display_label,
                WidgetPortType::Control,
                direction,
            );
            ModulePort::new(
                endpoint,
                display_label,
                accessible_label,
                "Mod Grid control signal",
                connected,
                highlighted,
                ModMarkers::default(),
            )
        })
        .collect();
    draw_module_port_column(ui, direction, &side, |port, center, response| {
        let endpoint = port.endpoint();
        state.port_positions.insert(endpoint, center);
        node_canvas::push_port_event(wire_events, response, endpoint, center);
    });
}

// ============================================================================
// Node body editors
// ============================================================================

fn sentence_case(label: &str) -> String {
    let mut chars = label.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn edit_node_body(
    ui: &mut egui::Ui,
    state: &mut ModGridViewState,
    cfg: &mut ModNodeConfig,
    any_dragged: &mut bool,
) {
    match cfg {
        ModNodeConfig::Module(m) => edit_module_body(ui, state, m, any_dragged),
        ModNodeConfig::Macro(m) => {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut m.name);
            });
            let mut value = m.value.as_f32();
            let r = ui.add(egui::Slider::new(&mut value, 0.0..=1.0).text("Value"));
            *any_dragged |= r.dragged();
            if r.changed() {
                m.value = synth_core::NormalizedValue::new(value);
            }
        }
        ModNodeConfig::Transport(tn) => {
            transport_combo(ui, tn);
        }
        ModNodeConfig::MidiCc(m) => {
            let mut cc = m.cc.as_u8();
            let r = ui.add(egui::DragValue::new(&mut cc).range(0..=127).prefix("CC "));
            *any_dragged |= r.dragged();
            if r.changed() {
                m.cc = synth_core::MidiCcNumber::new(cc).unwrap_or_default();
            }
            let mut omni = m.channel.is_none();
            if ui.checkbox(&mut omni, "Omni").changed() {
                m.channel = if omni { None } else { Some(0) };
            }
            if let Some(ch) = &mut m.channel {
                let r = ui.add(egui::DragValue::new(ch).range(0..=15).prefix("Ch "));
                *any_dragged |= r.dragged();
            }
        }
        ModNodeConfig::AudioTap(a) => audio_tap_combo(ui, state, a),
        ModNodeConfig::Target(target) => edit_target_body(ui, state, target, any_dragged),
    }
}

fn edit_module_body(
    ui: &mut egui::Ui,
    state: &mut ModGridViewState,
    m: &mut ModuleNode,
    any_dragged: &mut bool,
) {
    ui.label(
        RichText::new(m.module_type.name())
            .color(theme().colors.text_secondary)
            .size(11.0),
    );
    // Cache the descriptor so we don't rebuild the module every frame.
    let desc = state.descriptors.entry(m.module_type).or_insert_with(|| {
        crate::module_factory::create_voice_module(m.module_type).map(|(_, d)| d)
    });
    let Some(desc) = desc.as_ref() else {
        return;
    };
    // Use the same descriptor-driven controls as Rack: waveform icons, choice
    // dropdowns, toggles, sliders and knobs all follow the module's WidgetHint.
    // Reading a missing value falls back to the descriptor default without
    // mutating the graph merely because it was rendered.
    let changes = draw_parameter_grid(
        ui,
        desc,
        category_color(desc.category),
        |param| {
            m.params
                .get(&param.type_id)
                .copied()
                .unwrap_or_else(|| param.default_value())
        },
        |_, _| true,
        |_| ModMarkers::default(),
    );
    if !changes.is_empty() {
        *any_dragged |= ui.input(|input| input.pointer.primary_down());
    }
    for (param, value) in changes {
        m.params.insert(param.type_id.clone(), value);
    }
}

fn audio_tap_combo(
    ui: &mut egui::Ui,
    state: &ModGridViewState,
    a: &mut synth_sequencer::AudioTapNode,
) {
    use synth_sequencer::AudioTapSource;
    let label_for = |src: &AudioTapSource| match src {
        AudioTapSource::Master => "Master".to_string(),
        AudioTapSource::Track(tid) => state
            .tracks
            .iter()
            .find(|(id, _)| id == tid)
            .map_or_else(|| format!("Track {}", tid.0), |(_, n)| n.clone()),
    };
    ui.label(RichText::new("Smoothed level of").size(10.0));
    egui::ComboBox::from_id_salt("audio_tap_src")
        .selected_text(label_for(&a.source))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut a.source, AudioTapSource::Master, "Master");
            for (tid, name) in &state.tracks {
                ui.selectable_value(&mut a.source, AudioTapSource::Track(*tid), name);
            }
        });
}

fn transport_combo(ui: &mut egui::Ui, tn: &mut TransportNode) {
    let label = |s: TransportSource| match s {
        TransportSource::BeatPhase => "Beat phase",
        TransportSource::BarPhase => "Bar phase",
        TransportSource::Tempo => "Tempo",
        TransportSource::PositionBeats => "Position (beats)",
    };
    egui::ComboBox::from_id_salt("transport_src")
        .selected_text(label(tn.source))
        .show_ui(ui, |ui| {
            for s in [
                TransportSource::BeatPhase,
                TransportSource::BarPhase,
                TransportSource::Tempo,
                TransportSource::PositionBeats,
            ] {
                ui.selectable_value(&mut tn.source, s, label(s));
            }
        });
}

/// The Target sink editor: a hierarchical destination picker (the same nested
/// menu the pattern view's "Auto:" automation-target selector uses — This Track /
/// Global at the top, then per-instrument macros and per-module params) plus the
/// depth. Only grid-composable targets are offered (Track volume/pan/pitch, master
/// volume, instrument volume/pan, and any module's automatable params).
fn edit_target_body(
    ui: &mut egui::Ui,
    state: &mut ModGridViewState,
    target: &mut ModTarget,
    any_dragged: &mut bool,
) {
    let label = target.target.display_name();
    tree_picker_button(ui, "grid_target_dest", 180.0, label, |ui| {
        ui.set_min_width(180.0);
        // Owned snapshot of the current target for the selected-marks, so the
        // click handlers below can freely reassign `target.target`.
        let cur = target.target.clone();

        // This-track relative params (the default authoring form). Mute is
        // excluded — the grid can't modulate a bool.
        ui.menu_button("This Track", |ui| {
            ui.set_min_width(150.0);
            for param in [TrackParam::Volume, TrackParam::Pan, TrackParam::Pitch] {
                let t = AutomationTarget::Track { track: None, param };
                if ui
                    .selectable_label(cur == t, param.display_name())
                    .clicked()
                {
                    target.target = t;
                    ui.close();
                }
            }
        });

        // Global (master volume).
        ui.menu_button("Global", |ui| {
            ui.set_min_width(150.0);
            let t = AutomationTarget::Global(GlobalParam::MasterVolume);
            if ui.selectable_label(cur == t, "Master volume").clicked() {
                target.target = t;
                ui.close();
            }
        });

        // Per instrument: channel-level Volume/Pan macros, then one submenu per
        // automatable module (the shared `module_target_groups` enumeration, so
        // this matches the pattern-view picker's module list).
        if !state.instruments.is_empty() {
            ui.separator();
        }
        for (seq_id, name) in &state.instruments {
            ui.menu_button(format!("{}: {name}", seq_id.as_u64()), |ui| {
                ui.set_min_width(150.0);
                for param in [AutoInstrumentParam::Volume, AutoInstrumentParam::Pan] {
                    let t = AutomationTarget::Instrument {
                        instrument: *seq_id,
                        param,
                    };
                    if ui
                        .selectable_label(cur == t, param.display_name())
                        .clicked()
                    {
                        target.target = t;
                        ui.close();
                    }
                }
                if let Some(groups) = state.module_groups.get(seq_id) {
                    if !groups.is_empty() {
                        ui.separator();
                    }
                    for g in groups {
                        ui.menu_button(&g.label, |ui| {
                            ui.set_min_width(150.0);
                            for (type_id, pname) in &g.params {
                                let t = AutomationTarget::Module {
                                    instrument: *seq_id,
                                    module_type: g.module_id.module_type,
                                    instance: g.module_id.instance,
                                    param_id: type_id.as_str().into(),
                                };
                                if ui.selectable_label(cur == t, pname).clicked() {
                                    target.target = t;
                                    ui.close();
                                }
                            }
                        });
                    }
                }
            });
        }
    });

    ui.horizontal(|ui| {
        ui.label("Amount");
        let r = ui.add(egui::DragValue::new(&mut target.amount).speed(0.01));
        *any_dragged |= r.dragged();
    });
}

// ============================================================================
// Wiring
// ============================================================================

fn control_color() -> Color32 {
    PortWidget::new(WidgetPortType::Control).color()
}

fn propose_edit(slot: &mut Option<GraphEdit>, candidate: GraphEdit) {
    match slot {
        None => *slot = Some(candidate),
        Some(GraphEdit::SetNode(..)) if !matches!(candidate, GraphEdit::SetNode(..)) => {
            *slot = Some(candidate);
        }
        Some(_) => {}
    }
}

/// Orient two endpoints into a cable if compatible: exactly one output, distinct
/// nodes, and the input port not already driven. A cheap source (Macro/Transport/
/// AudioTap) feeding a hosted-module input is allowed — the engine injects it as a
/// block-constant value before the module processes (e.g. `Macro → LFO.rate_cv`).
fn open_connection(graph: &ModGraph, a: PortRef, b: PortRef) -> Option<ModConnection> {
    let (out, inp) = match (a.2, b.2) {
        (true, false) => (a, b),
        (false, true) => (b, a),
        _ => return None,
    };
    if out.0 == inp.0 {
        return None;
    }
    let conn = ModConnection::new(out.0, String::from(out.1), inp.0, String::from(inp.1));
    // Reject a second cable into an already-driven input port.
    let occupied = graph
        .connections
        .iter()
        .any(|c| c.to == inp.0 && synth_core::PortName::from(c.to_port.as_str()) == inp.1);
    (!occupied && !graph.connections.contains(&conn)).then_some(conn)
}

// ============================================================================
// Cables
// ============================================================================

/// Draw every cable using the previous frame's port anchors (resolved by the
/// cable's named ports, so module→module cables render too). A cable whose
/// endpoint port isn't drawn yet (first frame) is skipped. Returns the cable
/// under the pointer.
fn draw_cables(ui: &egui::Ui, state: &ModGridViewState, graph: &ModGraph) -> Option<ModConnection> {
    let pointer_world = ui
        .input(|i| i.pointer.interact_pos())
        .map(|p| scene_canvas::screen_to_world(ui, p));
    let color = crate::gui::widgets::cable_color(WidgetPortType::Control, 255);

    let mut hovered_cable = None;
    for (index, connection) in graph.connections.iter().enumerate() {
        let from_port = synth_core::PortName::from(connection.from_port.as_str());
        let to_port = synth_core::PortName::from(connection.to_port.as_str());
        let (Some(&from), Some(&to)) = (
            state
                .port_positions
                .get(&(connection.from, from_port, true)),
            state.port_positions.get(&(connection.to, to_port, false)),
        ) else {
            continue;
        };
        let spread = (index % 4) as f32 * 6.0;
        let hovered = pointer_world
            .is_some_and(|p| crate::gui::widgets::point_near_cable(p, from, to, 8.0, spread));
        if hovered {
            hovered_cable = Some(connection.clone());
            draw_cable_highlighted(ui.painter(), from, to, color, spread);
        } else {
            draw_cable(ui.painter(), from, to, color, spread);
        }
    }
    hovered_cable
}

// ============================================================================
// Background context menu — cable actions + the add-node catalog
// ============================================================================

/// The add-node palette: label + a default-configured instance.
fn node_catalog() -> Vec<(&'static str, ModNodeConfig)> {
    let module = |mt: ModuleType| {
        ModNodeConfig::Module(ModuleNode {
            module_type: mt,
            params: Default::default(),
            seed: None,
        })
    };
    vec![
        ("LFO", module(ModuleType::Lfo)),
        ("MSEG", module(ModuleType::Mseg)),
        ("Envelope Follower", module(ModuleType::EnvelopeFollower)),
        ("Euclidean", module(ModuleType::Euclidean)),
        ("Random Gates", module(ModuleType::RandomGates)),
        (
            "Transport",
            ModNodeConfig::Transport(TransportNode::default()),
        ),
        (
            "Macro",
            ModNodeConfig::Macro(synth_sequencer::MacroNode {
                name: "Macro".into(),
                value: synth_core::NormalizedValue::MIN,
            }),
        ),
        (
            "Audio Tap",
            ModNodeConfig::AudioTap(synth_sequencer::AudioTapNode {
                source: synth_sequencer::AudioTapSource::Master,
            }),
        ),
        (
            "MIDI CC",
            ModNodeConfig::MidiCc(synth_sequencer::MidiCcNode {
                cc: synth_core::MidiCcNumber::MOD_WHEEL,
                channel: None,
            }),
        ),
        (
            "Target →",
            ModNodeConfig::Target(ModTarget {
                target: AutomationTarget::Track {
                    track: None,
                    param: TrackParam::Volume,
                },
                amount: 0.25,
                combine: Default::default(),
            }),
        ),
    ]
}

fn draw_bg_context_menu(
    ui: &egui::Ui,
    state: &mut ModGridViewState,
    canvas_bg: &egui::Response,
    graph: &ModGraph,
    hovered_cable: Option<ModConnection>,
    edit: &mut Option<GraphEdit>,
) {
    if canvas_bg.secondary_clicked()
        && let Some(screen) = ui.input(|i| i.pointer.interact_pos())
    {
        state.bg_menu = Some((scene_canvas::screen_to_world(ui, screen), hovered_cable));
    }
    let Some((world_pos, menu_cable)) = state.bg_menu.clone() else {
        return;
    };

    canvas_bg.context_menu(|ui| {
        if let Some(connection) = menu_cable {
            if ui.button((ri::SCISSORS_CUT_LINE, "Delete cable")).clicked() {
                propose_edit(edit, GraphEdit::Disconnect(connection));
                ui.close();
            }
            ui.separator();
        }
        ui.label(
            RichText::new("Add node")
                .color(theme().colors.text_secondary)
                .size(11.0),
        );
        ui.separator();
        let at_cap = graph.node_count() >= MAX_MOD_GRID_NODES;
        for (label, config) in node_catalog() {
            let accent = node_accent(&config);
            let entry = egui::Button::new(
                RichText::new(format!("{}  {label}", ri::CHECKBOX_BLANK_CIRCLE_FILL)).color(accent),
            );
            if ui.add_enabled(!at_cap, entry).clicked() {
                propose_edit(edit, GraphEdit::AddNode(config, world_pos));
                ui.close();
            }
        }
    });

    if !canvas_bg.context_menu_opened() {
        state.bg_menu = None;
    }
}

// ============================================================================
// Applying edits + undo
// ============================================================================

fn apply_graph_edit(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut UndoManager,
    state: &mut ModGridViewState,
    pre_snapshot: &ModGraph,
    edit: Option<GraphEdit>,
) {
    let graph_id = pre_snapshot.id;
    let Some(edit) = edit else {
        return;
    };
    let mut push_snapshot: Option<(ModGraph, ModGraph)> = None;

    match edit {
        GraphEdit::AddNode(config, world_pos) => {
            let mut song_w = song.write();
            let Some(graph) = song_w.mod_graph_mut(graph_id) else {
                return;
            };
            let before = graph.clone();
            let node_id = graph.next_node_id();
            match graph.try_insert_node(node_id, config) {
                Ok(()) => {
                    state.last_error = None;
                    let pos = scene_canvas::snap_to_grid(world_pos);
                    graph.node_positions.insert(
                        node_id,
                        synth_sequencer::NodePosition { x: pos.x, y: pos.y },
                    );
                    push_snapshot = Some((before, graph.clone()));
                }
                Err(e) => state.last_error = Some(e.to_string()),
            }
        }
        GraphEdit::RemoveNode(node_id) => {
            let mut song_w = song.write();
            let Some(graph) = song_w.mod_graph_mut(graph_id) else {
                return;
            };
            let before = graph.clone();
            if graph.remove_node(node_id).is_some() {
                push_snapshot = Some((before, graph.clone()));
                state.sizes.remove(&(graph_id, node_id));
                state.last_error = None;
            }
        }
        GraphEdit::SetNode(node_id, config) => {
            if state
                .edit_baseline
                .as_ref()
                .is_none_or(|(gid, _)| *gid != graph_id)
            {
                state.edit_baseline = Some((graph_id, pre_snapshot.clone()));
            }
            let mut song_w = song.write();
            let Some(graph) = song_w.mod_graph_mut(graph_id) else {
                return;
            };
            if let Err(e) = graph.try_insert_node(node_id, config) {
                state.last_error = Some(e.to_string());
            }
        }
        GraphEdit::Connect(connection) => {
            let mut song_w = song.write();
            let Some(graph) = song_w.mod_graph_mut(graph_id) else {
                return;
            };
            let before = graph.clone();
            match graph.try_connect(connection) {
                Ok(()) => {
                    push_snapshot = Some((before, graph.clone()));
                    state.last_error = None;
                }
                Err(e) => state.last_error = Some(e.to_string()),
            }
        }
        GraphEdit::Disconnect(connection) => {
            let mut song_w = song.write();
            let Some(graph) = song_w.mod_graph_mut(graph_id) else {
                return;
            };
            let before = graph.clone();
            if graph.disconnect(&connection) {
                push_snapshot = Some((before, graph.clone()));
                state.last_error = None;
            }
        }
    }

    if let Some((before, after)) = push_snapshot
        && before != after
    {
        undo_manager.push(UndoAction::SetModGraph {
            graph_id,
            old: Some(before),
            new: Some(after),
        });
    }
}

/// Finalize a coalesced config-edit undo entry once no widget is being dragged:
/// one undo entry per gesture, not per frame.
fn finalize_config_undo(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut UndoManager,
    state: &mut ModGridViewState,
    any_dragged: bool,
) {
    if any_dragged {
        return;
    }
    let Some((graph_id, baseline)) = state.edit_baseline.take() else {
        return;
    };
    let after = song.read().mod_graph(graph_id).cloned();
    if let Some(after) = after
        && after != baseline
    {
        undo_manager.push(UndoAction::SetModGraph {
            graph_id,
            old: Some(baseline),
            new: Some(after),
        });
    }
}

// ============================================================================
// Node styling
// ============================================================================

fn node_name(config: &ModNodeConfig) -> String {
    match config {
        ModNodeConfig::Module(m) => m.module_type.name().to_string(),
        ModNodeConfig::Macro(m) => format!("Macro · {}", m.name),
        ModNodeConfig::Transport(_) => "Transport".to_string(),
        ModNodeConfig::MidiCc(m) => format!("MIDI CC {}", m.cc),
        ModNodeConfig::AudioTap(_) => "Audio Tap".to_string(),
        ModNodeConfig::Target(_) => "Target".to_string(),
    }
}

fn node_accent(config: &ModNodeConfig) -> Color32 {
    let c = &theme().colors;
    match config {
        ModNodeConfig::Module(_) => c.accent_cyan,
        ModNodeConfig::Macro(_) => c.accent_purple,
        ModNodeConfig::Transport(_) => c.accent_primary,
        ModNodeConfig::MidiCc(_) => c.accent_green,
        ModNodeConfig::AudioTap(_) => c.accent_orange,
        ModNodeConfig::Target(_) => c.accent_yellow,
    }
}
