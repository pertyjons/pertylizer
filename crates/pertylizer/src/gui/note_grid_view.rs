//! The Note Grid view — pool list + Scene node canvas (plan §8.1).
//!
//! Mirrors the instrument Rack view: a left panel listing the project's pooled
//! note graphs (create / duplicate / rename / recolor / delete, usage counts)
//! and a central `egui::Scene` node canvas editing the selected graph — nodes
//! with `NoteStream` / `Value` ports, drag-to-connect cables, per-node config
//! editors reusing the Note FX card bodies.
//!
//! The plan's "make unique" action (§8.1) deliberately lives with the *host
//! binding* (the pattern editor's graph selector), not here — forking a shared
//! graph needs a host to repoint, and this view has no current-host context.
//!
//! Graphs live in the shared `Song` pool; every edit goes through
//! `song.write()` and the graph's own validating editors (`try_insert_node`,
//! `try_connect`, `remove_node` — which rebuild `processing_order` and reject
//! cycles / non-linear spines), then pushes a whole-graph snapshot undo entry.
//! Node *positions* persist on the graph (`NoteGraph::node_positions`, saved
//! with the project); nodes without one get deterministic auto-layout from
//! `processing_order`. Position writes are layout metadata: applied live
//! during a drag, deliberately not undo-tracked.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Vec2};
use egui_remixicon::icons as ri;

use synth_core::{ModuleCategory, ModuleType, ModuleWidth, NormalizedValue};
use synth_engine::ModuleId;
use synth_sequencer::{
    Arpeggiator, Chord, Duration as SeqDuration, EnvelopeTrigger, EuclideanGenerator, Humanize,
    LfoShape, MAX_DELAY_REPEATS, MAX_NOTE_DELAY_TICKS, MAX_RATCHET_SUBDIVISIONS,
    MAX_STEP_LFO_STEPS, NoteConnection, NoteDelay, NoteEnvelope, NoteGraph, NoteGraphId, NoteLfo,
    NoteModuleConfig, NoteModuleId, NoteName, NotePortType, NoteProcessor, NoteScriptTransform,
    Pitch, ProbabilityGate, Ratchet, ScaleQuantize, StepLfo, TrackColor, Velocity,
};

use crate::gui::auto_layout::{LayoutConnection, ModuleInfo, calculate_free_flow_layout};
use crate::gui::list_panel;
use crate::gui::node_canvas;
use crate::gui::scene_canvas;
use crate::gui::script_editor;
use crate::gui::sequencer::note_fx;
use crate::gui::theme::theme;
use crate::gui::toolbar;
use crate::gui::widgets::{
    CaptionTone, Knob, ModMarkers, ModuleCard, ModuleCardGeometry, ModuleColumn, ModulePort,
    ModulePortEndpoint, PortWidget, WidgetPortDirection, WidgetPortType, caption, count_drag_value,
    danger_button, dim_label, draw_cable, draw_cable_dragging, draw_cable_highlighted,
    draw_flow_particles, draw_module_port_column, draw_module_port_layout, enum_combo, expose,
    icon_button, knob_normalized, labeled_row, module_port_accessible_label, seed_reroll,
    submenu_button,
};
use crate::undo::{UndoAction, UndoManager};

/// Card height before the first frame has measured the real one.
const DEFAULT_NODE_HEIGHT: f32 = 140.0;
/// Gap between a node's IN/OUT port column and its content band (the rack uses
/// the same 4 px so the columns don't eat the body's width).
/// Hit radius (world units) for dropping a dragged wire on a port.
const WIRE_DROP_RADIUS: f32 = 16.0;

/// One end of a node's port set. `ValueIn` is indexed like
/// `NoteConnection::to_input`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum NodePort {
    StreamIn,
    StreamOut,
    ValueOut,
    ValueIn(u8),
}

impl ModulePortEndpoint for NodePort {
    fn widget_port_type(self) -> WidgetPortType {
        self.widget_type()
    }
}

impl NodePort {
    const fn is_output(self) -> bool {
        matches!(self, Self::StreamOut | Self::ValueOut)
    }

    const fn widget_type(self) -> WidgetPortType {
        match self {
            Self::StreamIn | Self::StreamOut => WidgetPortType::NoteStream,
            Self::ValueOut | Self::ValueIn(_) => WidgetPortType::Control,
        }
    }

    fn stable_id(self) -> String {
        match self {
            Self::StreamIn => "stream-in".to_string(),
            Self::StreamOut => "stream-out".to_string(),
            Self::ValueOut => "value-out".to_string(),
            Self::ValueIn(input) => format!("value-in-{input}"),
        }
    }
}

/// One node's port endpoint — its node id plus which port. The shared wire FSM
/// ([`node_canvas::wiring`]) keys pending wires, drop targets, and connection
/// validation on this pair.
type PortRef = (NoteModuleId, NodePort);

/// A port interaction gathered while drawing the nodes, resolved afterwards by
/// [`node_canvas::resolve_wire_events`].
type WireEvent = node_canvas::WireEvent<PortRef>;

/// Per-session view state (selection, search, canvas cameras, node positions).
#[derive(Default)]
pub(crate) struct NoteGridViewState {
    /// The graph loaded in the canvas.
    pub(crate) selected: Option<NoteGraphId>,
    search: String,
    /// Measured card sizes from the previous frame (drag hit-testing). Node
    /// *positions* live on the graph itself (`NoteGraph::node_positions`,
    /// persisted with the project); auto-layout fills the gaps per frame.
    sizes: HashMap<(NoteGraphId, NoteModuleId), Vec2>,
    /// Per-graph Scene camera.
    scene_rects: HashMap<NoteGraphId, Rect>,
    /// Port anchor positions recorded while drawing (world coords); cables are
    /// drawn from the previous frame's set, one frame of lag is imperceptible.
    port_positions: HashMap<PortRef, Pos2>,
    pending_wire: Option<node_canvas::WireDrag<PortRef>>,
    /// Pre-gesture graph snapshot for coalescing config-edit undo.
    edit_baseline: Option<(NoteGraphId, NoteGraph)>,
    /// Last rejected edit, shown in the canvas header until the next success.
    last_error: Option<String>,
    /// Rename-in-flight in the pool kebab: `(graph, buffer)`.
    rename: Option<(NoteGraphId, String)>,
    /// Open background context menu: `(world position, cable under pointer)`.
    bg_menu: Option<(Pos2, Option<NoteConnection>)>,
    /// Open graph-metadata edit window (the toolbar "Edit…" action), buffering
    /// the name so a rename coalesces into one undo entry.
    editing: Option<GraphEditState>,
    /// Open note-script editor popup (a `NoteScriptTransform` node's ƒx button),
    /// buffering the draft until Apply — the shared ƒx editor (§10.4).
    script_editing: Option<NoteScriptEditState>,
}

/// Buffered state for the note-script editor popup (§10.4): the node being
/// edited plus the in-progress draft, independent of the installed `source`
/// until Apply.
struct NoteScriptEditState {
    graph: NoteGraphId,
    node: NoteModuleId,
    draft: String,
    /// Whether the note_event reference window is open beside the editor.
    show_help: bool,
}

/// Buffered state for the graph-metadata edit window (name / description /
/// color). Mirrors the instrument and pattern edit windows.
struct GraphEditState {
    id: NoteGraphId,
    name: String,
    description: String,
}

/// A deferred edit to the selected graph, applied after the snapshot is drawn.
enum GraphEdit {
    AddNode(NoteModuleConfig, Pos2),
    RemoveNode(NoteModuleId),
    /// Live config edit — applied immediately, undo coalesced via
    /// `edit_baseline` (the Note FX rack idiom).
    SetNode(NoteModuleId, NoteModuleConfig),
    Connect(NoteConnection),
    Disconnect(NoteConnection),
}

/// Draw the whole Note Grid view: left pool panel + central node canvas.
pub(crate) fn draw_note_grid_view(
    ui: &mut egui::Ui,
    song: &Arc<synth_sequencer::SharedSong>,
    state: &mut NoteGridViewState,
    undo_manager: &mut UndoManager,
) {
    let selected_at_entry = state.selected;
    let pool = draw_pool_panel(ui, song, state, undo_manager);

    // Resolve the selection against the live pool (it may have been deleted or
    // the project replaced); fall back to the first graph. `None` = the pool
    // snapshot failed on a transient lock miss — keep the selection rather
    // than resetting the user's canvas over lock contention.
    if let Some(pool) = pool
        && state.selected.is_none_or(|id| !pool.contains(&id))
    {
        state.selected = pool.first().copied();
    }

    // Node ids are graph-local, so a wire or menu started on the previous
    // graph must not survive a selection change (its ids would alias
    // unrelated nodes in the new graph).
    if state.selected != selected_at_entry {
        state.pending_wire = None;
        state.bg_menu = None;
    }

    egui::CentralPanel::default().show(ui, |ui| {
        let Some(graph_id) = state.selected else {
            ui.vertical_centered(|ui| {
                ui.add_space(48.0);
                ui.label(
                    RichText::new("No note graphs yet — create one to start processing notes.")
                        .size(theme().fonts.size_normal)
                        .color(theme().colors.text_secondary),
                );
            });
            return;
        };
        draw_graph_canvas(ui, song, state, undo_manager, graph_id);
    });

    // The metadata edit window (toolbar "Edit…") and the note-script editor
    // popup are drawn at the ctx level so they survive the canvas's early returns.
    let ctx = ui.ctx().clone();
    draw_graph_edit_window(&ctx, song, state, undo_manager);
    draw_note_script_editor(&ctx, song, state, undo_manager);
}

// ============================================================================
// Left panel — the pool
// ============================================================================

/// A deferred pool-level edit collected while drawing the left panel.
enum PoolEdit {
    Create,
    Duplicate(NoteGraphId),
    Delete(NoteGraphId),
    Rename(NoteGraphId, String),
    Recolor(NoteGraphId, Option<TrackColor>),
}

/// One pool-panel row snapshot: id, name, color, description, usage count.
type PoolRow = (NoteGraphId, String, Option<TrackColor>, String, usize);

/// Draw the pool panel; returns the pool's graph ids, or `None` when the song
/// snapshot missed (transient lock contention — callers must not treat that
/// as an empty pool).
fn draw_pool_panel(
    ui: &mut egui::Ui,
    song: &Arc<synth_sequencer::SharedSong>,
    state: &mut NoteGridViewState,
    undo_manager: &mut UndoManager,
) -> Option<Vec<NoteGraphId>> {
    let rows: Option<Vec<PoolRow>> = song.try_read().map(|s| {
        s.note_graphs()
            .map(|g| {
                (
                    g.id,
                    g.name.clone(),
                    g.color,
                    g.description.clone(),
                    s.note_graph_usage(g.id),
                )
            })
            .collect()
    });
    let snapshot_ok = rows.is_some();
    let rows = rows.unwrap_or_default();

    let mut edit: Option<PoolEdit> = None;
    let mut clicked: Option<NoteGraphId> = None;

    egui::Panel::left("note_grid_pool_panel")
        .default_size(list_panel::DEFAULT_WIDTH)
        .min_size(list_panel::MIN_WIDTH)
        .show(ui, |ui| {
            egui::Panel::top("note_grid_pool_head").show(ui, |ui| {
                if list_panel::header(ui, ri::MIND_MAP, "Note Graphs", "New note graph") {
                    edit = Some(PoolEdit::Create);
                }
                list_panel::search_box(ui, &mut state.search);
            });

            let needle = state.search.to_lowercase();
            // Filter once per frame; the same list drives the rows and the
            // empty-state label.
            let shown: Vec<_> = rows
                .iter()
                .filter(|(_, name, ..)| needle.is_empty() || name.to_lowercase().contains(&needle))
                .collect();

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if shown.is_empty() {
                        list_panel::empty(ui, "No note graphs");
                    }
                    for (id, name, color, description, usage) in shown {
                        let is_active = state.selected == Some(*id);
                        let used = *usage > 0;

                        // Color swatch + row on one line; the row hosts the kebab.
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(10.0), Sense::empty());
                            let swatch = color.map_or(theme().colors.text_dim, |c| {
                                Color32::from_rgb(c.r, c.g, c.b)
                            });
                            ui.painter().rect_filled(rect.shrink(1.0), 2.0, swatch);

                            let rename = &mut state.rename;
                            let response = list_panel::row(
                                ui,
                                is_active,
                                name,
                                list_panel::row_text_color(is_active, used),
                                |ui| {
                                    // Rename: an auto-focused inline editor;
                                    // Enter / focus loss commits (Escape still
                                    // reverts via egui's TextEdit).
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
                                            // Commit + close only on a real
                                            // change, so tabbing into the other
                                            // menu items doesn't slam the menu.
                                            let new_name = buf.trim();
                                            if !new_name.is_empty() && new_name != name {
                                                edit = Some(PoolEdit::Rename(*id, new_name.into()));
                                                ui.close();
                                            }
                                            *rename = None;
                                        }
                                    }
                                    ui.separator();
                                    submenu_button(ui, (ri::PALETTE_LINE, "Color"), |ui| {
                                        // Swatch row from the shared presets,
                                        // plus a clear entry.
                                        ui.horizontal(|ui| {
                                            for &preset in TrackColor::presets() {
                                                let color =
                                                    crate::gui::sequencer::track_color_to_egui(
                                                        preset,
                                                    );
                                                let (rect, resp) = ui.allocate_exact_size(
                                                    Vec2::splat(16.0),
                                                    Sense::click(),
                                                );
                                                ui.painter().rect_filled(rect, 3.0, color);
                                                expose(
                                                    &resp,
                                                    egui::WidgetType::Button,
                                                    format!(
                                                        "color {:02X}{:02X}{:02X}",
                                                        preset.r, preset.g, preset.b
                                                    ),
                                                    None,
                                                );
                                                if resp.clicked() {
                                                    edit =
                                                        Some(PoolEdit::Recolor(*id, Some(preset)));
                                                    ui.close();
                                                }
                                            }
                                        });
                                        if ui.button("No color").clicked() {
                                            edit = Some(PoolEdit::Recolor(*id, None));
                                            ui.close();
                                        }
                                    });
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
                            let tip = if used {
                                format!(
                                    "{description}\nUsed by {usage} pattern{}",
                                    if *usage == 1 { "" } else { "s" }
                                )
                            } else {
                                format!("{description}\nUnused — no pattern binds this graph")
                            };
                            if response.on_hover_text(tip.trim()).clicked() && !is_active {
                                clicked = Some(*id);
                            }
                        });
                    }
                });
        });

    if let Some(id) = clicked {
        state.selected = Some(id);
    }

    // Ids created below are appended so the caller's selection resolution sees
    // the just-created graph as part of the pool (not "deleted → reset").
    let mut pool_ids: Option<Vec<NoteGraphId>> =
        snapshot_ok.then(|| rows.into_iter().map(|(id, ..)| id).collect());

    match edit {
        Some(PoolEdit::Create) => {
            let snapshot = {
                let mut song_w = song.write();
                let n = song_w.note_graphs().count() + 1;
                let id = song_w.create_note_graph(format!("Graph {n}"));
                song_w.note_graph(id).cloned()
            };
            if let Some(graph) = snapshot {
                state.selected = Some(graph.id);
                if let Some(pool) = &mut pool_ids {
                    pool.push(graph.id);
                }
                undo_manager.push(UndoAction::SetNoteGraph {
                    graph_id: graph.id,
                    old: None,
                    new: Some(graph),
                });
            }
        }
        Some(PoolEdit::Duplicate(src_id)) => {
            let snapshot = song.write().duplicate_note_graph(src_id);
            if let Some(graph) = snapshot {
                state.selected = Some(graph.id);
                if let Some(pool) = &mut pool_ids {
                    pool.push(graph.id);
                }
                undo_manager.push(UndoAction::SetNoteGraph {
                    graph_id: graph.id,
                    old: None,
                    new: Some(graph),
                });
            }
        }
        Some(PoolEdit::Delete(id)) => {
            let (removed, cleared) = {
                let mut song_w = song.write();
                let cleared: Vec<_> = song_w
                    .patterns()
                    .filter(|p| p.note_graph() == Some(id))
                    .map(|p| p.id)
                    .collect();
                (song_w.remove_note_graph(id), cleared)
            };
            if let Some(graph) = removed {
                // Bindings first, then the graph — the Composite inverse replays
                // in reverse, re-inserting the graph before re-binding patterns.
                let mut actions: Vec<UndoAction> = cleared
                    .into_iter()
                    .map(|pattern_id| UndoAction::SetPatternNoteGraph {
                        pattern_id,
                        old: Some(id),
                        new: None,
                    })
                    .collect();
                actions.push(UndoAction::SetNoteGraph {
                    graph_id: id,
                    old: Some(graph),
                    new: None,
                });
                undo_manager.push(UndoAction::Composite(actions));
            }
        }
        Some(PoolEdit::Rename(id, new_name)) => {
            with_graph_undo(song, undo_manager, id, |graph| {
                if !new_name.trim().is_empty() {
                    graph.name = new_name.trim().to_owned();
                }
            });
        }
        Some(PoolEdit::Recolor(id, color)) => {
            with_graph_undo(song, undo_manager, id, |graph| {
                graph.color = color;
            });
        }
        None => {}
    }

    // Pre-edit snapshot plus any ids created above; a just-deleted graph
    // resolves next frame.
    pool_ids
}

/// Mutate one graph under the write lock and push a snapshot undo entry when
/// the closure actually changed it. Used for metadata edits (rename, recolor).
fn with_graph_undo(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut UndoManager,
    id: NoteGraphId,
    mutate: impl FnOnce(&mut NoteGraph),
) {
    let Some((before, after)) = ({
        let mut song_w = song.write();
        song_w.note_graph_mut(id).map(|graph| {
            let before = graph.clone();
            mutate(graph);
            (before, graph.clone())
        })
    }) else {
        return;
    };
    if before != after {
        undo_manager.push(UndoAction::SetNoteGraph {
            graph_id: id,
            old: Some(before),
            new: Some(after),
        });
    }
}

/// The graph-metadata edit window (name / description / color), opened from the
/// toolbar "Edit…" action — the analog of the instrument and pattern edit
/// windows. Name commits as one coalesced undo entry on defocus/close;
/// description and color are utility metadata written live (description not
/// undo-tracked, matching pattern descriptions; color coalesces via
/// [`with_graph_undo`]).
fn draw_graph_edit_window(
    ctx: &egui::Context,
    song: &Arc<synth_sequencer::SharedSong>,
    state: &mut NoteGridViewState,
    undo_manager: &mut UndoManager,
) {
    let Some(edit) = state.editing.as_mut() else {
        return;
    };
    let graph_id = edit.id;
    // Drop the window if its graph is gone (deleted while open).
    if song
        .try_read()
        .map(|s| s.note_graph(graph_id).is_none())
        .unwrap_or(false)
    {
        state.editing = None;
        return;
    }

    let t = theme();
    let mut open = true;
    let mut name_done = false;
    let mut recolor: Option<Option<TrackColor>> = None;

    egui::Window::new(format!("{} Edit note graph", ri::EDIT_LINE))
        .id(egui::Id::new(("note_graph_edit_window", graph_id.0)))
        .open(&mut open)
        .resizable(true)
        .default_size([360.0, 260.0])
        .show(ctx, |ui| {
            ui.label(RichText::new("Name").color(t.colors.text_dim));
            if ui.text_edit_singleline(&mut edit.name).lost_focus() {
                name_done = true;
            }

            ui.add_space(t.spacing.md);

            ui.label(RichText::new("Description").color(t.colors.text_dim));
            if ui
                .add(
                    egui::TextEdit::multiline(&mut edit.description)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                // Utility metadata — written live, not undo-tracked (matching
                // pattern descriptions).
                if let Some(g) = song.write().note_graph_mut(graph_id) {
                    g.description = edit.description.clone();
                }
            }

            ui.add_space(t.spacing.md);

            ui.label(RichText::new("Color").color(t.colors.text_dim));
            ui.horizontal_wrapped(|ui| {
                for &preset in TrackColor::presets() {
                    let color = crate::gui::sequencer::track_color_to_egui(preset);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                    ui.painter().rect_filled(rect, 3.0, color);
                    expose(
                        &resp,
                        egui::WidgetType::Button,
                        format!("color {:02X}{:02X}{:02X}", preset.r, preset.g, preset.b),
                        None,
                    );
                    if resp.clicked() {
                        recolor = Some(Some(preset));
                    }
                }
                if ui.button("No color").clicked() {
                    recolor = Some(None);
                }
            });
        });

    if let Some(color) = recolor {
        with_graph_undo(song, undo_manager, graph_id, |g| g.color = color);
    }
    // Commit the buffered name once, on defocus or close; skip empties so a
    // cleared field can't blank the name. `with_graph_undo` no-ops if unchanged.
    if name_done || !open {
        let new_name = edit.name.trim().to_owned();
        if !new_name.is_empty() {
            with_graph_undo(song, undo_manager, graph_id, |g| g.name = new_name.clone());
        }
    }
    if !open {
        state.editing = None;
    }
}

/// An action the note-script popup requests this frame.
enum ScriptEditAction {
    /// Install the given source on the node.
    Apply(String),
    /// Reset the node to pass-through (empty source).
    Clear,
}

/// The shared ƒx script-editor popup for a `NoteScriptTransform` node (§10.4):
/// the shared code editor + live compile status, plus Format / Apply / Clear /
/// Close. It mirrors the Rack's Mod-Matrix / AudioScript ƒx popup and reuses its
/// editor atoms ([`script_editor`]). Apply installs the draft (undo-tracked,
/// recompiled); Clear resets it to pass-through. Drawn at the ctx level so it
/// survives the canvas's early returns; the draft lives in view state until
/// applied, independent of the node's installed `source`.
fn draw_note_script_editor(
    ctx: &egui::Context,
    song: &Arc<synth_sequencer::SharedSong>,
    state: &mut NoteGridViewState,
    undo_manager: &mut UndoManager,
) {
    let Some(edit) = state.script_editing.as_mut() else {
        return;
    };
    let graph_id = edit.graph;
    let node_id = edit.node;
    // Drop the popup if its node vanished (deleted, graph replaced, or the config
    // swapped to a non-script kind while the popup was open).
    let node_present = song
        .try_read()
        .map(|s| {
            s.note_graph(graph_id)
                .and_then(|g| g.nodes.get(&node_id))
                .is_some_and(|n| matches!(n, NoteModuleConfig::NoteScriptTransform(_)))
        })
        .unwrap_or(true);
    if !node_present {
        state.script_editing = None;
        return;
    }

    let mut open = true;
    let mut action: Option<ScriptEditAction> = None;
    // The Close button (and Clear) can't touch `open` — it is borrowed by
    // `.open(&mut open)` for the ✕ — so route those through a separate flag.
    let mut close_requested = false;
    egui::Window::new(format!("{} Note script — note_event", ri::CODE_LINE))
        .id(egui::Id::new(("note_script_editor", graph_id.0, node_id.0)))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(480.0)
        .min_width(320.0)
        .show(ctx, |ui| {
            script_editor::script_editor_header(
                ui,
                "Runs once per note: read note_pitch / note_vel / note_dur / tick and in1..in4; \
                 assign out.pitch / out.vel / out.dur / out.gate.",
                "note_event script reference",
                &mut edit.show_help,
            );

            let te_id = egui::Id::new(("note_script_text", graph_id.0, node_id.0));
            script_editor::script_code_editor(ui, te_id, &mut edit.draft, 12);

            // Live compile → status. Empty is a valid pass-through (not an error);
            // a non-empty draft compiles under the note_event dialect.
            let compiled_ok = if edit.draft.trim().is_empty() {
                dim_label(ui, "empty — use Clear to pass notes through");
                false
            } else {
                let status = crate::session::compile_note_event_script(&edit.draft).map(|_| ());
                let ok = status.is_ok();
                script_editor::script_status_line(ui, &status);
                ok
            };

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(compiled_ok, egui::Button::new("Format"))
                    .on_hover_text("Reformat the script (yamsfmt)")
                    .clicked()
                    && let Ok(formatted) = synth_script::format(&edit.draft)
                {
                    edit.draft = formatted;
                }
                if ui
                    .add_enabled(compiled_ok, egui::Button::new("Apply"))
                    .on_hover_text("Install this script on the node (keeps editing)")
                    .clicked()
                {
                    action = Some(ScriptEditAction::Apply(edit.draft.clone()));
                }
                if ui
                    .button("Clear")
                    .on_hover_text("Remove the script (passes notes through)")
                    .clicked()
                {
                    action = Some(ScriptEditAction::Clear);
                }
                if ui.button("Close").clicked() {
                    close_requested = true;
                }
            });
        });

    // The note_event reference, as a sibling window so it reads beside the
    // editor; its ✕ mirrors back to the Help toggle.
    if edit.show_help {
        let mut help_open = true;
        script_editor::draw_note_script_help_window(
            ctx,
            egui::Id::new(("note_script_help", graph_id.0, node_id.0)),
            &mut help_open,
        );
        if !help_open {
            edit.show_help = false;
        }
    }

    match action {
        Some(ScriptEditAction::Apply(source)) => {
            set_note_script(song, undo_manager, graph_id, node_id, source);
        }
        Some(ScriptEditAction::Clear) => {
            set_note_script(song, undo_manager, graph_id, node_id, String::new());
            close_requested = true;
        }
        None => {}
    }
    if !open || close_requested {
        state.script_editing = None;
    }
}

/// Install `source` on a `NoteScriptTransform` node and recompile — the GUI
/// counterpart of MCP `set_note_graph_script`, routed through the graph undo
/// path so a script change is one undo entry.
fn set_note_script(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut UndoManager,
    graph_id: NoteGraphId,
    node_id: NoteModuleId,
    source: String,
) {
    with_graph_undo(song, undo_manager, graph_id, |g| {
        if let Some(NoteModuleConfig::NoteScriptTransform(t)) = g.nodes.get_mut(&node_id) {
            t.source = source;
        }
        // The compiled program is `#[serde(skip)]`, so rebuild it from the new
        // source (the source is the single source of truth).
        crate::project_apply::recompile_graph_scripts(g);
    });
}

// ============================================================================
// Central canvas — the node editor
// ============================================================================

fn draw_graph_canvas(
    ui: &mut egui::Ui,
    song: &Arc<synth_sequencer::SharedSong>,
    state: &mut NoteGridViewState,
    undo_manager: &mut UndoManager,
    graph_id: NoteGraphId,
) {
    let t = theme();

    // Snapshot the graph + usage under one short read lock.
    let Some((mut graph, usage)) = song.try_read().and_then(|s| {
        Some((
            s.note_graph(graph_id)?.clone(),
            s.note_graph_usage(graph_id),
        ))
    }) else {
        dim_label(ui, "Graph unavailable…");
        return;
    };

    // Context bar: name + status on the left, Edit… / Auto Layout right-aligned
    // (mirrors the Rack patch toolbar).
    let mut open_edit = false;
    let mut auto_layout = false;
    toolbar::top(ui, "note_grid_toolbar", |ui| {
        ui.label(
            RichText::new(format!("{} {}", ri::MIND_MAP, graph.name))
                .size(t.fonts.size_heading)
                .color(t.colors.text_primary),
        );
        ui.label(
            RichText::new(if usage == 0 {
                "not bound to any pattern".to_owned()
            } else {
                format!(
                    "used by {usage} pattern{}",
                    if usage == 1 { "" } else { "s" }
                )
            })
            .color(t.colors.text_dim),
        );
        if let Some(err) = &state.last_error {
            ui.label(
                RichText::new(format!("{} {err}", ri::ERROR_WARNING_LINE))
                    .color(t.colors.accent_red),
            );
        }
        // Stream nodes off the active spine are inert (the evaluator skips
        // them) — tell the user why their node isn't doing anything.
        let inert = off_spine_stream_nodes(&graph);
        if inert > 0 {
            ui.label(
                RichText::new(format!(
                    "{} {inert} unwired node{} inactive — wire into the chain",
                    ri::ERROR_WARNING_LINE,
                    if inert == 1 { "" } else { "s" }
                ))
                .color(t.colors.accent_yellow),
            );
        }

        // Right-aligned actions: Auto Layout · Edit…
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button((
                    RichText::new(ri::EDIT_LINE).color(t.colors.text_secondary),
                    RichText::new("Edit…").color(t.colors.text_secondary),
                ))
                .on_hover_text("Edit this graph's name, description, and color")
                .clicked()
            {
                open_edit = true;
            }
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

    if open_edit {
        state.editing = Some(GraphEditState {
            id: graph_id,
            name: graph.name.clone(),
            description: graph.description.clone(),
        });
    }
    if auto_layout {
        // Discard the persisted positions so auto-layout takes over, and refit
        // the camera. Layout metadata — deliberately not undo-tracked (matching
        // node drags and the Rack's auto-layout). Clear the local snapshot too
        // so this same frame redraws with the fresh layout (no flicker).
        if let Some(g) = song.write().note_graph_mut(graph_id) {
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
    // A node drag in progress: persisted into the graph each frame (layout
    // metadata — no undo entry).
    let mut moved: Option<(NoteModuleId, Pos2)> = None;

    scene_canvas::scene().show(ui, &mut scene_rect, |ui| {
        let world_rect = ui.clip_rect();
        let canvas_bg = ui.interact(world_rect, ui.id().with("note_grid_bg"), Sense::click());
        expose(
            &canvas_bg,
            egui::WidgetType::Panel,
            "note grid canvas",
            None,
        );
        scene_canvas::draw_grid(ui, world_rect);

        // First-use affordance: an empty graph is just a grid otherwise.
        if graph.nodes.is_empty() {
            ui.painter().text(
                world_rect.center(),
                egui::Align2::CENTER_CENTER,
                "Right-click to add nodes",
                egui::FontId::proportional(theme().fonts.size_normal),
                theme().colors.text_dim,
            );
        }

        // Cables first (behind the nodes), from the previous frame's anchors.
        let hovered_cable = draw_cables(ui, state, &graph);

        // Nodes repopulate the port anchors for the next frame.
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

        // The live wire follows the pointer.
        if let Some(pending) = &state.pending_wire
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let world = scene_canvas::screen_to_world(ui, pointer);
            draw_cable_dragging(
                ui.painter(),
                pending.from_pos,
                world,
                port_color(pending.from.1),
            );
        }

        // Click on empty grid or Escape cancels a pending wire.
        if canvas_bg.clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.pending_wire = None;
        }

        // Right-click: cable delete + the add-node catalog.
        draw_bg_context_menu(ui, state, &canvas_bg, &graph, hovered_cable, &mut edit);
    });
    state.scene_rects.insert(graph_id, scene_rect);

    // Pinned zoom/fit controls (screen-space, outside the Scene).
    if let Some(action) = scene_canvas::view_controls(
        ui,
        egui::Id::new(("note_grid_view_controls", graph_id.0)),
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

    // Persist a live node drag (layout metadata — deliberately no undo).
    if let Some((node_id, pos)) = moved
        && let Some(graph) = song.write().note_graph_mut(graph_id)
    {
        graph.node_positions.insert(
            node_id,
            synth_sequencer::NodePosition { x: pos.x, y: pos.y },
        );
    }

    apply_graph_edit(song, undo_manager, state, &graph, edit);
    finalize_config_undo(song, undo_manager, state, any_dragged);
}

/// World rects of every laid-out node (camera framing).
fn node_rects<'a>(
    state: &'a NoteGridViewState,
    positions: &'a HashMap<NoteModuleId, Pos2>,
    graph: &'a NoteGraph,
) -> impl Iterator<Item = Rect> + 'a {
    graph.nodes.iter().filter_map(move |(id, config)| {
        let pos = positions.get(id)?;
        let size = state
            .sizes
            .get(&(graph.id, *id))
            .copied()
            .unwrap_or_else(|| {
                let margin = theme().sizes.module_compact_inner_margin;
                let geometry = ModuleCardGeometry::ported(node_width(config), margin);
                Vec2::new(geometry.outer_width, DEFAULT_NODE_HEIGHT)
            });
        Some(Rect::from_min_size(*pos, size))
    })
}

/// Stream nodes outside the active spine — inert at evaluation, surfaced as a
/// header warning so a silent node is explained.
fn off_spine_stream_nodes(graph: &NoteGraph) -> usize {
    graph
        .nodes
        .iter()
        .filter(|(id, config)| !config.is_value_source() && !graph.stream_spine.contains(id))
        .count()
}

/// Effective positions from the shared Sugiyama flow layout, overridden by any
/// manually persisted positions. The free graph layout uses the measured outer
/// card dimensions, including both port columns.
fn layout_positions(state: &NoteGridViewState, graph: &NoteGraph) -> HashMap<NoteModuleId, Pos2> {
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
            let compact_margin = theme().sizes.module_compact_inner_margin;
            let geometry = ModuleCardGeometry::ported(node_width(config), compact_margin);
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
    let mut positions: HashMap<NoteModuleId, Pos2> = flow
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

#[allow(clippy::too_many_arguments)]
fn draw_node(
    ui: &mut egui::Ui,
    state: &mut NoteGridViewState,
    graph: &NoteGraph,
    positions: &HashMap<NoteModuleId, Pos2>,
    node_id: NoteModuleId,
    config: &NoteModuleConfig,
    edit: &mut Option<GraphEdit>,
    moved: &mut Option<(NoteModuleId, Pos2)>,
    any_dragged: &mut bool,
    wire_events: &mut Vec<WireEvent>,
) {
    let key = (graph.id, node_id);
    let Some(mut pos) = positions.get(&node_id).copied() else {
        return;
    };
    // Rack `IN | body | OUT` layout: the content band keeps its bucket width and
    // the card grows by the two flanking port columns (+ their gaps).
    let compact_margin = theme().sizes.module_compact_inner_margin;
    let geometry = ModuleCardGeometry::ported(node_width(config), compact_margin);
    let size = state
        .sizes
        .get(&key)
        .copied()
        .unwrap_or_else(|| Vec2::new(geometry.outer_width, DEFAULT_NODE_HEIGHT));

    // Drag handle over the whole card, registered BEFORE the body so the
    // body's widgets (drawn on top) keep their own clicks/drags. Inside a
    // Scene, `drag_delta()` is already world-space (patch-editor idiom).
    let card_id = egui::Id::new(("note_grid_node", graph.id.0, node_id.0));
    let node_rect = Rect::from_min_size(pos, size);
    let response = ui.interact(node_rect, card_id.with("card"), Sense::click_and_drag());
    expose(
        &response,
        egui::WidgetType::Button,
        format!("{} node {}", node_name(config), node_id.0),
        None,
    );
    // Position changes are written to the graph after the frame (`moved`), so
    // they persist with the project and survive frames mid-drag.
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
        .inner_margin(compact_margin)
        .body_module_width(node_width(config));

    // Card content in a child Ui at the node's world position. Global-scope id
    // keeps inner widget ids independent of draw order (patch-editor idiom).
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id(card_id)
            .max_rect(Rect::from_min_size(
                pos,
                Vec2::new(geometry.outer_width, 600.0),
            ))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    card.show(&mut child, |card| {
        let description = graph.node_descriptions.get(&node_id).cloned();
        card.header(node_name(config), description, false, |ui| {
            if icon_button(ui, ri::CLOSE_LINE, theme().colors.text_dim, "Remove node").clicked() {
                propose_edit(edit, GraphEdit::RemoveNode(node_id));
            }
        });

        // Rack `IN | body | OUT` three-column layout: the input port column, the
        // config body sized to the bucket band, then the output port column
        // anchored to the right edge.
        draw_module_port_layout(
            card.body(),
            geometry.body_width,
            |column, ui| match column {
                ModuleColumn::Input => {
                    draw_note_port_column(ui, state, graph, node_id, config, true, wire_events);
                }
                ModuleColumn::Body => {
                    // Config editor into a working copy; a diff drives the live apply.
                    let mut cfg = config.clone();
                    let mut open_script = false;
                    edit_node_config(ui, node_id, &mut cfg, any_dragged, &mut open_script);
                    if cfg != *config {
                        propose_edit(edit, GraphEdit::SetNode(node_id, cfg));
                    }
                    // A `NoteScriptTransform` node's ƒx button requests the shared
                    // script editor popup, seeded from the node's live source (§10.4).
                    if open_script && let NoteModuleConfig::NoteScriptTransform(t) = config {
                        state.script_editing = Some(NoteScriptEditState {
                            graph: graph.id,
                            node: node_id,
                            draft: t.source().to_string(),
                            show_help: false,
                        });
                    }
                }
                ModuleColumn::Output => {
                    draw_note_port_column(ui, state, graph, node_id, config, false, wire_events);
                }
            },
        );

        // An arp / strummed chord rebuilds its stream from the source notes
        // through pitch transforms only, so an upstream script / gate / generator
        // is silently ignored — warn (the free canvas has no locked chain order).
        if graph.source_context_ignores_upstream(node_id) {
            ui.label(
                RichText::new(
                    "⚠ ignores upstream — reads source notes through pitch transforms only",
                )
                .color(theme().colors.accent_yellow)
                .size(10.0),
            );
        }
    });
    state.sizes.insert(key, child.min_rect().size());
}

/// A rack-style vertical port column for one side of a node (`is_input` picks
/// the IN vs OUT side): an `IN`/`OUT` header, a rail, and the side's ports
/// stacked as dots — the port name/type/description live in the hover tooltip,
/// exactly like the patch editor's shared module-port column. The column always
/// reserves its width so the body stays centred even when a side has no ports.
/// Anchors are recorded for cables; interactions become [`WireEvent`]s.
fn draw_note_port_column(
    ui: &mut egui::Ui,
    state: &mut NoteGridViewState,
    graph: &NoteGraph,
    node_id: NoteModuleId,
    config: &NoteModuleConfig,
    is_input: bool,
    wire_events: &mut Vec<WireEvent>,
) {
    // The ports on this side: (port, connected, name, description). A stream node
    // has one stream port per side; script / probability-gate inputs add their
    // value inputs; value sources emit a single `value out`.
    let mut ports: Vec<(NodePort, bool, String, &'static str)> = Vec::new();
    if is_input {
        if config.has_stream_input() {
            let connected = graph
                .connections
                .iter()
                .any(|c| c.port.is_stream() && c.to == node_id);
            ports.push((
                NodePort::StreamIn,
                connected,
                "notes in".into(),
                NOTES_IN_DESC,
            ));
        }
        for input in 0..config.value_input_count() {
            let connected = graph
                .connections
                .iter()
                .any(|c| !c.port.is_stream() && c.to == node_id && c.to_input == input);
            ports.push((
                NodePort::ValueIn(input),
                connected,
                value_input_name(config, input),
                value_input_desc(config),
            ));
        }
    } else if config.has_stream_output() {
        let connected = graph
            .connections
            .iter()
            .any(|c| c.port.is_stream() && c.from == node_id);
        ports.push((
            NodePort::StreamOut,
            connected,
            "notes out".into(),
            NOTES_OUT_DESC,
        ));
    } else if config.is_value_source() {
        let connected = graph
            .connections
            .iter()
            .any(|c| !c.port.is_stream() && c.from == node_id);
        ports.push((
            NodePort::ValueOut,
            connected,
            "value out".into(),
            value_out_desc(config),
        ));
    }

    let direction = if is_input {
        WidgetPortDirection::Input
    } else {
        WidgetPortDirection::Output
    };
    let ports: Vec<ModulePort<NodePort>> = ports
        .into_iter()
        .map(|(port, connected, label, description)| {
            let highlighted = state.pending_wire.as_ref().is_some_and(|pending| {
                open_connection(graph, pending.from, (node_id, port)).is_some()
            });
            let accessible_label = module_port_accessible_label(
                &format!("{} node {}", node_name(config), node_id.0),
                &port.stable_id(),
                &label,
                port.widget_type(),
                direction,
            );
            ModulePort::new(
                port,
                label,
                accessible_label,
                description,
                connected,
                highlighted,
                ModMarkers::default(),
            )
        })
        .collect();
    draw_module_port_column(ui, direction, &ports, |port, center, response| {
        state
            .port_positions
            .insert((node_id, port.endpoint()), center);
        node_canvas::push_port_event(wire_events, response, (node_id, port.endpoint()), center);
    });
}

/// The label for value input port `input` on `config`, matching the register the
/// node reads: `in1`..`in4` for a `NoteScriptTransform` (its four `Value`
/// registers), `threshold` for a `ProbabilityGate`'s single modulation input.
/// Falls back to `mod N` for any future node that grows a value input.
fn value_input_name(config: &NoteModuleConfig, input: u8) -> String {
    match config {
        NoteModuleConfig::ProbabilityGate(_) => "threshold".to_string(),
        NoteModuleConfig::NoteScriptTransform(_) => format!("in{}", input + 1),
        _ => format!("mod {}", input + 1),
    }
}

/// Hover description for the note-stream input / output ports (§10.2).
const NOTES_IN_DESC: &str = "The upstream note stream this node processes.";
const NOTES_OUT_DESC: &str = "The processed note stream, feeding downstream nodes.";

/// Hover description for a value-source node's output port (§10.2).
fn value_out_desc(config: &NoteModuleConfig) -> &'static str {
    match config {
        NoteModuleConfig::NoteLfo(_) => {
            "Tempo-synced LFO value (0..1) — patch into a downstream node's parameter."
        }
        NoteModuleConfig::StepLfo(_) => {
            "Step-sequenced value (0..1) — patch into a downstream node's parameter."
        }
        NoteModuleConfig::NoteEnvelope(_) => {
            "Envelope level (0..1) tracking note onsets — patch into a downstream node's parameter."
        }
        _ => "Modulation value (0..1) — patch into a downstream node's parameter.",
    }
}

/// Hover description for a node's value input port (§10.2).
fn value_input_desc(config: &NoteModuleConfig) -> &'static str {
    match config {
        NoteModuleConfig::ProbabilityGate(_) => "Modulates the pass-probability threshold (0..1).",
        NoteModuleConfig::NoteScriptTransform(_) => {
            "A Value input, readable in the script by this port's name (in1..in4)."
        }
        _ => "Modulation input (0..1).",
    }
}

/// Record a proposed edit for this frame (one edit applies per frame).
/// Structural edits outrank the config-diff `SetNode` a committing widget can
/// emit in the same frame — e.g. a DragValue losing focus to the Remove click
/// would otherwise shadow the removal.
fn propose_edit(slot: &mut Option<GraphEdit>, candidate: GraphEdit) {
    match slot {
        None => *slot = Some(candidate),
        Some(GraphEdit::SetNode(..)) if !matches!(candidate, GraphEdit::SetNode(..)) => {
            *slot = Some(candidate);
        }
        Some(_) => {}
    }
}

/// [`make_connection`] plus availability: the edge must not already exist,
/// the stream endpoints must be free (linearity), and a value input takes one
/// source. Mirrors what `try_connect` would reject, so ports never invite a
/// guaranteed-failing drop.
fn open_connection(
    graph: &NoteGraph,
    a: (NoteModuleId, NodePort),
    b: (NoteModuleId, NodePort),
) -> Option<NoteConnection> {
    let conn = make_connection(a, b)?;
    let occupied = if conn.port.is_stream() {
        graph
            .connections
            .iter()
            .any(|c| c.port.is_stream() && (c.to == conn.to || c.from == conn.from))
    } else {
        graph
            .connections
            .iter()
            .any(|c| !c.port.is_stream() && c.to == conn.to && c.to_input == conn.to_input)
    };
    (!occupied && !graph.connections.contains(&conn)).then_some(conn)
}

/// Orient two port endpoints into a typed connection, if they are compatible:
/// exactly one output, distinct nodes, stream→stream or value→value-input.
fn make_connection(
    a: (NoteModuleId, NodePort),
    b: (NoteModuleId, NodePort),
) -> Option<NoteConnection> {
    let (out, inp) = match (a.1.is_output(), b.1.is_output()) {
        (true, false) => (a, b),
        (false, true) => (b, a),
        _ => return None,
    };
    if out.0 == inp.0 {
        return None;
    }
    match (out.1, inp.1) {
        (NodePort::StreamOut, NodePort::StreamIn) => Some(NoteConnection::stream(out.0, inp.0)),
        (NodePort::ValueOut, NodePort::ValueIn(input)) => {
            Some(NoteConnection::value(out.0, inp.0, input))
        }
        _ => None,
    }
}

// ============================================================================
// Cables
// ============================================================================

/// Anchor keys for a connection's two ends.
const fn connection_ports(connection: &NoteConnection) -> (NodePort, NodePort) {
    if connection.port.is_stream() {
        (NodePort::StreamOut, NodePort::StreamIn)
    } else {
        (NodePort::ValueOut, NodePort::ValueIn(connection.to_input))
    }
}

fn port_color(port: NodePort) -> Color32 {
    PortWidget::new(port.widget_type()).color()
}

const fn connection_widget_type(connection: &NoteConnection) -> WidgetPortType {
    match connection.port {
        NotePortType::NoteStream => WidgetPortType::NoteStream,
        NotePortType::Value => WidgetPortType::Control,
        NotePortType::Gate => WidgetPortType::Gate,
    }
}

/// Draw every connection using the previous frame's anchors. Returns the
/// cable under the pointer, if any — per-frame derived data, so it is a
/// return value rather than view state.
fn draw_cables(
    ui: &egui::Ui,
    state: &NoteGridViewState,
    graph: &NoteGraph,
) -> Option<NoteConnection> {
    let time = ui.input(|i| i.time);
    let pointer_world = ui
        .input(|i| i.pointer.interact_pos())
        .map(|p| scene_canvas::screen_to_world(ui, p));

    let mut hovered_cable = None;
    for (index, connection) in graph.connections.iter().enumerate() {
        let (out_port, in_port) = connection_ports(connection);
        let (Some(&from), Some(&to)) = (
            state.port_positions.get(&(connection.from, out_port)),
            state.port_positions.get(&(connection.to, in_port)),
        ) else {
            continue; // first frame — anchors not recorded yet
        };
        // Small per-connection fan-out so parallel cables stay readable.
        let spread = (index % 4) as f32 * 6.0;
        let widget_type = connection_widget_type(connection);
        let color = crate::gui::widgets::cable_color(widget_type, 255);

        let hovered = pointer_world
            .is_some_and(|p| crate::gui::widgets::point_near_cable(p, from, to, 8.0, spread));
        if hovered {
            hovered_cable = Some(*connection);
            draw_cable_highlighted(ui.painter(), from, to, color, spread);
        } else {
            draw_cable(ui.painter(), from, to, color, spread);
            draw_flow_particles(ui.painter(), from, to, color, widget_type, time, spread);
        }
    }
    hovered_cable
}

// ============================================================================
// Background context menu — cable actions + the add-node catalog
// ============================================================================

/// The add-node palette: label + a default-configured instance. The
/// arpeggiator and strummed chord read the source notes through the spine's
/// upstream processors (the held-pitch seam, plan §5.B), wired into the graph
/// evaluator — so both process fully here, no longer pass-through.
fn node_catalog() -> [(&'static str, NoteModuleConfig); 12] {
    [
        (
            "Scale Quantize",
            NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize::default())),
        ),
        (
            "Chord",
            NoteModuleConfig::Processor(NoteProcessor::Chord(Chord::default())),
        ),
        (
            "Arpeggiator",
            NoteModuleConfig::Processor(NoteProcessor::Arpeggiator(Arpeggiator::default())),
        ),
        (
            "Humanize",
            NoteModuleConfig::Processor(NoteProcessor::Humanize(Humanize::default())),
        ),
        (
            "Euclidean Generator",
            NoteModuleConfig::Euclidean(EuclideanGenerator::default()),
        ),
        (
            "Probability Gate",
            NoteModuleConfig::ProbabilityGate(ProbabilityGate::default()),
        ),
        ("Note LFO", NoteModuleConfig::NoteLfo(NoteLfo::default())),
        ("Step LFO", NoteModuleConfig::StepLfo(StepLfo::default())),
        (
            "Note Envelope",
            NoteModuleConfig::NoteEnvelope(NoteEnvelope::default()),
        ),
        (
            "Script",
            NoteModuleConfig::NoteScriptTransform(NoteScriptTransform::new(
                "out.pitch = note_pitch",
            )),
        ),
        (
            "Delay / Echo",
            NoteModuleConfig::NoteDelay(NoteDelay::default()),
        ),
        ("Ratchet", NoteModuleConfig::Ratchet(Ratchet::default())),
    ]
}

fn draw_bg_context_menu(
    ui: &egui::Ui,
    state: &mut NoteGridViewState,
    canvas_bg: &egui::Response,
    graph: &NoteGraph,
    hovered_cable: Option<NoteConnection>,
    edit: &mut Option<GraphEdit>,
) {
    // Capture the world position (and hovered cable) at right-click time; the
    // menu stays open across frames while the pointer moves.
    if canvas_bg.secondary_clicked()
        && let Some(screen) = ui.input(|i| i.pointer.interact_pos())
    {
        state.bg_menu = Some((scene_canvas::screen_to_world(ui, screen), hovered_cable));
    }
    let Some((world_pos, menu_cable)) = state.bg_menu else {
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
        let at_cap = graph.node_count() >= synth_sequencer::MAX_NOTE_GRID_NODES;
        for (label, config) in node_catalog() {
            // Colour each entry by its node accent (same rack-category colour
            // logic as the nodes) so the "Add node" menu reads with the same
            // palette as the rack's category-coloured add menu — a leading
            // filled-circle swatch plus the tinted label.
            let accent = node_accent(&config);
            let entry = egui::Button::new(
                egui::RichText::new(format!("{}  {label}", ri::CHECKBOX_BLANK_CIRCLE_FILL))
                    .color(accent),
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

/// Apply the frame's single deferred edit under the write lock. Structural
/// edits push a whole-graph snapshot undo immediately; `SetNode` applies live
/// and leaves undo to [`finalize_config_undo`] (Note FX coalescing idiom).
fn apply_graph_edit(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut UndoManager,
    state: &mut NoteGridViewState,
    pre_snapshot: &NoteGraph,
    edit: Option<GraphEdit>,
) {
    let graph_id = pre_snapshot.id;
    let Some(edit) = edit else {
        return;
    };

    // Structural edits capture (before, after) for the undo entry; `None`
    // means the edit was rejected (error already recorded).
    let mut push_snapshot: Option<(NoteGraph, NoteGraph)> = None;

    match edit {
        GraphEdit::AddNode(config, world_pos) => {
            let mut song_w = song.write();
            let Some(graph) = song_w.note_graph_mut(graph_id) else {
                return;
            };
            let before = graph.clone();
            let node_id = graph.next_module_id();
            let auto_connect_from = config
                .has_stream_input()
                .then(|| graph.stream_output_node())
                .flatten();
            match graph.try_insert_node(node_id, config) {
                Ok(()) => {
                    // Keep the spine a single chain: hang a new stream consumer
                    // off the current tail. A rejection leaves the node unwired
                    // and is surfaced — a silently dangling node reads as "the
                    // graph stopped working".
                    state.last_error = None;
                    if let Some(tail) = auto_connect_from
                        && tail != node_id
                        && let Err(e) = graph.try_connect(NoteConnection::stream(tail, node_id))
                    {
                        state.last_error = Some(format!("added unwired: {e}"));
                    }
                    // Drop position (persisted layout metadata) — inside the
                    // same write, so it lands in the `after` undo snapshot.
                    let pos = scene_canvas::snap_to_grid(world_pos);
                    graph.node_positions.insert(
                        node_id,
                        synth_sequencer::NodePosition { x: pos.x, y: pos.y },
                    );
                    // Compile a just-added script node's program (a catalog node
                    // carries only its source) so it runs immediately, and so the
                    // compiled state is captured in the `after` undo snapshot.
                    crate::project_apply::recompile_graph_scripts(graph);
                    push_snapshot = Some((before, graph.clone()));
                }
                Err(e) => state.last_error = Some(e.to_string()),
            }
        }
        GraphEdit::RemoveNode(node_id) => {
            let mut song_w = song.write();
            let Some(graph) = song_w.note_graph_mut(graph_id) else {
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
            // Capture the pre-gesture baseline once per gesture (coalescing).
            if state
                .edit_baseline
                .as_ref()
                .is_none_or(|(gid, _)| *gid != graph_id)
            {
                state.edit_baseline = Some((graph_id, pre_snapshot.clone()));
            }
            let mut song_w = song.write();
            let Some(graph) = song_w.note_graph_mut(graph_id) else {
                return;
            };
            // Replacing an existing id never trips the node cap; an invalid
            // replacement (e.g. a config swap that breaks an edge) rolls back.
            match graph.try_insert_node(node_id, config) {
                // Recompile so a source edit's new program is installed on the
                // live node (the working-copy diff only carries the `source`).
                Ok(()) => crate::project_apply::recompile_graph_scripts(graph),
                Err(e) => state.last_error = Some(e.to_string()),
            }
        }
        GraphEdit::Connect(connection) => {
            let mut song_w = song.write();
            let Some(graph) = song_w.note_graph_mut(graph_id) else {
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
            let Some(graph) = song_w.note_graph_mut(graph_id) else {
                return;
            };
            let before = graph.clone();
            graph.connections.retain(|c| *c != connection);
            // Dropping an edge only relaxes constraints — rebuild can't fail.
            let _ = graph.rebuild_derived();
            push_snapshot = Some((before, graph.clone()));
            state.last_error = None;
        }
    }

    if let Some((before, after)) = push_snapshot
        && before != after
    {
        undo_manager.push(UndoAction::SetNoteGraph {
            graph_id,
            old: Some(before),
            new: Some(after),
        });
    }
}

/// Finalize a coalesced config-edit undo entry once no widget is being
/// dragged (the Note FX idiom): one undo entry per gesture, not per frame.
fn finalize_config_undo(
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut UndoManager,
    state: &mut NoteGridViewState,
    any_dragged: bool,
) {
    if any_dragged {
        return;
    }
    let Some((graph_id, old)) = state.edit_baseline.take() else {
        return;
    };
    let Some(song_r) = song.try_read() else {
        // Transient lock miss — keep the baseline for a retry next frame.
        state.edit_baseline = Some((graph_id, old));
        return;
    };
    let new = song_r.note_graph(graph_id).cloned();
    drop(song_r);
    if let Some(new) = new
        && new != old
    {
        undo_manager.push(UndoAction::SetNoteGraph {
            graph_id,
            old: Some(old),
            new: Some(new),
        });
    }
}

// ============================================================================
// Node metadata + per-kind config editors
// ============================================================================

fn node_name(config: &NoteModuleConfig) -> &'static str {
    match config {
        NoteModuleConfig::Processor(p) => note_fx::processor_name(p),
        NoteModuleConfig::Euclidean(_) => "Euclidean",
        NoteModuleConfig::ProbabilityGate(_) => "Probability Gate",
        NoteModuleConfig::NoteLfo(_) => "Note LFO",
        NoteModuleConfig::StepLfo(_) => "Step LFO",
        NoteModuleConfig::NoteEnvelope(_) => "Note Envelope",
        NoteModuleConfig::NoteScriptTransform(_) => "Script",
        NoteModuleConfig::NoteDelay(_) => "Delay / Echo",
        NoteModuleConfig::Ratchet(_) => "Ratchet",
    }
}

/// The node's accent colour. Colours are **keyed to the rack module-category
/// palette** ([`category_color`](crate::gui::module_panel::category_color)) so a
/// colour means roughly the same thing in the Note Grid as in the instrument
/// rack: **purple** = LFO / modulation source, **green** = envelope, **cyan** =
/// filter / time-based effect (pitch quantize, delay, ratchet), **red** =
/// sequencer / rhythm generator (Euclid, arp), **orange** = note
/// generator/source (chord), **grey** = utility / logic / script (humanize,
/// probability gate, script). This replaces an earlier ad-hoc scheme whose
/// colours clashed (Chord and Script both green, Scale Quantize and Delay both
/// cyan with no shared meaning).
fn node_accent(config: &NoteModuleConfig) -> Color32 {
    let t = theme();
    match config {
        NoteModuleConfig::Processor(p) => note_fx::processor_accent(p),
        // Rhythm/sequence generators → sequencer red.
        NoteModuleConfig::Euclidean(_) => t.colors.accent_red,
        // Time-based effects → effect cyan.
        NoteModuleConfig::NoteDelay(_) | NoteModuleConfig::Ratchet(_) => t.colors.accent_cyan,
        // LFO modulation sources → LFO purple.
        NoteModuleConfig::NoteLfo(_) | NoteModuleConfig::StepLfo(_) => t.colors.accent_purple,
        // Envelope → envelope green.
        NoteModuleConfig::NoteEnvelope(_) => t.colors.accent_green,
        // Logic / utility / script → utility grey.
        NoteModuleConfig::ProbabilityGate(_) | NoteModuleConfig::NoteScriptTransform(_) => {
            t.colors.text_secondary
        }
    }
}

/// The fixed card width bucket for a node kind — reuses the audio patch
/// editor's [`ModuleWidth`] so note nodes have defined, consistent sizes (the
/// body is pinned to the bucket minus the compact card margins). Buckets fit
/// each kind's widest non-wrapping row without overflow, measured in-app: the
/// Arpeggiator's `Octaves / Vel / Latch / Legato` row needs `ExtraLarge`;
/// combo/multi-field rows (Scale Quantize, Chord, Euclidean, LFOs) need
/// `Large`; the two-field Note Envelope fits `Medium`. The seed-carrying
/// Humanize and Probability Gate take `Medium` too: their `Seed` reroll row
/// widens with the seed's digit count, so a large seed overflows `Small`.
fn node_width(config: &NoteModuleConfig) -> ModuleWidth {
    match config {
        NoteModuleConfig::Processor(p) => match p {
            NoteProcessor::Humanize(_) => ModuleWidth::Medium,
            NoteProcessor::ScaleQuantize(_) | NoteProcessor::Chord(_) => ModuleWidth::Large,
            NoteProcessor::Arpeggiator(_) => ModuleWidth::ExtraLarge,
        },
        NoteModuleConfig::ProbabilityGate(_) => ModuleWidth::Medium,
        NoteModuleConfig::NoteEnvelope(_)
        | NoteModuleConfig::NoteDelay(_)
        | NoteModuleConfig::Ratchet(_) => ModuleWidth::Medium,
        NoteModuleConfig::Euclidean(_)
        | NoteModuleConfig::NoteLfo(_)
        | NoteModuleConfig::StepLfo(_)
        | NoteModuleConfig::NoteScriptTransform(_) => ModuleWidth::Large,
    }
}

/// Render the per-kind editor into `config` (a working copy the caller diffs).
/// A `NoteScriptTransform`'s source is edited in the shared popup, not inline, so
/// its body only *requests* the popup — the caller opens it, seeded from the live
/// source (§10.4).
fn edit_node_config(
    ui: &mut egui::Ui,
    node_id: NoteModuleId,
    config: &mut NoteModuleConfig,
    any_dragged: &mut bool,
    open_script: &mut bool,
) {
    // The Note FX editors salt their combo ids with a rack index; the node id
    // is the graph-side analog.
    let salt = node_id.0 as usize;
    // Computed before the `match` mut-borrows `config` (returns a `Copy` colour,
    // so the immutable borrow ends immediately); used by the script node body.
    let accent = node_accent(config);
    match config {
        NoteModuleConfig::Processor(p) => match p {
            NoteProcessor::ScaleQuantize(q) => note_fx::edit_scale_quantize(ui, salt, q),
            NoteProcessor::Chord(c) => note_fx::edit_chord(ui, salt, c, any_dragged),
            NoteProcessor::Arpeggiator(a) => note_fx::edit_arpeggiator(ui, salt, a, any_dragged),
            NoteProcessor::Humanize(h) => note_fx::edit_humanize(ui, h, any_dragged),
        },
        NoteModuleConfig::Euclidean(e) => edit_euclidean(ui, e, any_dragged),
        NoteModuleConfig::ProbabilityGate(g) => edit_probability_gate(ui, g, any_dragged),
        NoteModuleConfig::NoteLfo(l) => edit_note_lfo(ui, salt, l, any_dragged),
        NoteModuleConfig::StepLfo(s) => edit_step_lfo(ui, s, any_dragged),
        NoteModuleConfig::NoteEnvelope(e) => edit_note_envelope(ui, salt, e, any_dragged),
        NoteModuleConfig::NoteScriptTransform(t) => {
            note_script_node_body(ui, t, accent, open_script)
        }
        NoteModuleConfig::NoteDelay(d) => edit_note_delay(ui, d, any_dragged),
        NoteModuleConfig::Ratchet(r) => edit_ratchet(ui, r, any_dragged),
    }
}

/// The ratchet editor: subdivision length (ticks), retrigger count, and decay.
fn edit_ratchet(ui: &mut egui::Ui, r: &mut Ratchet, any_dragged: &mut bool) {
    labeled_row(ui, "Sub", |ui| {
        ticks_drag(ui, &mut r.sub_ticks, 1..=MAX_NOTE_DELAY_TICKS, any_dragged)
            .on_hover_text("Length of one subdivision, in ticks (960 = 1 beat).");
    });
    ui.horizontal(|ui| {
        count_drag_value(ui, &mut r.count, 1..=MAX_RATCHET_SUBDIVISIONS, any_dragged)
            .on_hover_text("Number of retriggers per note (max 16).");
        ui.label("hits");
    });
    ui.horizontal(|ui| {
        knob_normalized(ui, "Decay", &mut r.decay, any_dragged)
            .on_hover_text("Velocity falloff applied to each successive retrigger (0..1).");
    });
    dim_label(ui, "Subdivides each note into fast retriggers.");
}

/// The delay/echo editor: spacing (ticks), echo count, and per-echo feedback.
fn edit_note_delay(ui: &mut egui::Ui, d: &mut NoteDelay, any_dragged: &mut bool) {
    labeled_row(ui, "Time", |ui| {
        ticks_drag(
            ui,
            &mut d.delay_ticks,
            1..=MAX_NOTE_DELAY_TICKS,
            any_dragged,
        )
        .on_hover_text("Spacing between echoes, in ticks (960 = 1 beat).");
    });
    ui.horizontal(|ui| {
        count_drag_value(ui, &mut d.repeats, 1..=MAX_DELAY_REPEATS, any_dragged)
            .on_hover_text("Number of echoes after the dry note (max 16).");
        ui.label("echoes");
    });
    ui.horizontal(|ui| {
        knob_normalized(ui, "Feedback", &mut d.feedback, any_dragged)
            .on_hover_text("Velocity falloff applied to each successive echo (0..1).");
    });
    dim_label(ui, "Decaying repeats of the incoming onsets.");
}

/// The `NoteScriptTransform` node body, styled like the rack's script modules
/// (`draw_script_slot_row`): the `note` label, a truncated one-line preview of
/// the installed source, and a `ƒx` button that opens the shared editor popup
/// ([`draw_note_script_editor`], §10.4). The source is edited only in the popup,
/// so this body never mutates the config — it just requests the popup via `open`.
fn note_script_node_body(
    ui: &mut egui::Ui,
    t: &NoteScriptTransform,
    accent: Color32,
    open: &mut bool,
) {
    let colors = theme().colors;
    ui.horizontal(|ui| {
        caption(ui, "note", CaptionTone::Color(accent));
        // Truncated preview of the installed source (the popup owns editing).
        let src = t.source();
        let (preview, color) = if src.trim().is_empty() {
            ("— empty —".to_string(), colors.text_dim)
        } else {
            (script_editor::script_preview(src), colors.text_secondary)
        };
        caption(ui, preview, CaptionTone::Color(color));
        // ƒx is green when a working program is installed, grey otherwise — the
        // rack's script-module cue.
        let fx_color = if t.is_compiled() {
            colors.accent_green
        } else {
            colors.text_secondary
        };
        if ui
            .button(egui::RichText::new("ƒx").color(fx_color))
            .on_hover_text("Edit the note_event script")
            .clicked()
        {
            *open = true;
        }
    });
}

/// Drag a tick count with a ` tk` suffix (the shared `unit_drag_value`
/// preset over the `Duration` newtype); ORs `any_dragged` while held.
fn ticks_drag(
    ui: &mut egui::Ui,
    value: &mut SeqDuration,
    range: std::ops::RangeInclusive<u32>,
    any_dragged: &mut bool,
) -> egui::Response {
    let mut ticks = value.0;
    let resp = crate::gui::widgets::unit_drag_value(ui, &mut ticks, range, 1.0, " tk");
    *any_dragged |= resp.dragged();
    *value = SeqDuration(ticks);
    resp
}

fn edit_euclidean(ui: &mut egui::Ui, e: &mut EuclideanGenerator, any_dragged: &mut bool) {
    labeled_row(ui, "Steps", |ui| {
        let resp = ui
            .add(egui::DragValue::new(&mut e.steps).range(0..=64))
            .on_hover_text("Total steps in the Euclidean cycle.");
        *any_dragged |= resp.dragged();
        ui.label("Pulses");
        let resp = ui
            .add(egui::DragValue::new(&mut e.pulses).range(0..=64))
            .on_hover_text("Onsets spread as evenly as possible across the steps.");
        *any_dragged |= resp.dragged();
        ui.label("Rot");
        let resp = ui
            .add(egui::DragValue::new(&mut e.rotation).range(0..=64))
            .on_hover_text("Rotate the onset pattern by this many steps.");
        *any_dragged |= resp.dragged();
    });
    labeled_row(ui, "Step len", |ui| {
        ticks_drag(ui, &mut e.step_len, 1..=3840, any_dragged)
            .on_hover_text("Length of one step, in ticks (960 = 1 beat).");
    });
    labeled_row(ui, "Pitch", |ui| {
        let mut midi = e.pitch.as_midi();
        let resp = ui
            .add(egui::DragValue::new(&mut midi).range(0..=127))
            .on_hover_text("MIDI pitch of the generated notes.");
        *any_dragged |= resp.dragged();
        if let Some(p) = Pitch::new(midi) {
            e.pitch = p;
        }
        dim_label(ui, NoteName::from_midi(e.pitch.as_midi()).to_string());
        let mut vel = e.velocity.as_f32();
        let resp = Knob::new(&mut vel, 0.0, 1.0)
            .label("Vel")
            .show(ui)
            .on_hover_text("Velocity of the generated notes (0..1).");
        *any_dragged |= resp.dragged();
        e.velocity = Velocity::new(vel);
    });
}

fn edit_probability_gate(ui: &mut egui::Ui, g: &mut ProbabilityGate, any_dragged: &mut bool) {
    ui.horizontal(|ui| {
        knob_normalized(ui, "Prob", &mut g.probability, any_dragged)
            .on_hover_text("Chance each note passes (0 = drop all, 1 = keep all).");
    });
    labeled_row(ui, "Seed", |ui| {
        seed_reroll(ui, &mut g.seed, any_dragged);
    })
    .response
    .on_hover_text("Seed for the reproducible pass/block roll — reroll for a fresh pattern.");
    dim_label(
        ui,
        "A connected threshold input overrides this probability.",
    );
}

const LFO_SHAPES: [(LfoShape, &str); 4] = [
    (LfoShape::Sine, "Sine"),
    (LfoShape::Triangle, "Triangle"),
    (LfoShape::Saw, "Saw"),
    (LfoShape::Square, "Square"),
];

fn edit_note_lfo(ui: &mut egui::Ui, salt: usize, l: &mut NoteLfo, any_dragged: &mut bool) {
    labeled_row(ui, "Shape", |ui| {
        enum_combo(ui, (salt, "lfo_shape"), &mut l.shape, &LFO_SHAPES)
            .on_hover_text("LFO waveform.");
        ui.label("Period");
        ticks_drag(ui, &mut l.period, 0..=61_440, any_dragged)
            .on_hover_text("One full LFO cycle, in ticks (960 = 1 beat).");
    });
    ui.horizontal(|ui| {
        knob_normalized(ui, "Phase", &mut l.phase, any_dragged)
            .on_hover_text("Starting phase offset within the cycle (0..1).");
        knob_normalized(ui, "Depth", &mut l.depth, any_dragged)
            .on_hover_text("Output amount scaling the value (0..1).");
    });
}

fn edit_step_lfo(ui: &mut egui::Ui, s: &mut StepLfo, any_dragged: &mut bool) {
    labeled_row(ui, "Step len", |ui| {
        ticks_drag(ui, &mut s.step_len, 0..=3840, any_dragged)
            .on_hover_text("Length of one step, in ticks (960 = 1 beat).");
        ui.label(format!("{} steps", s.steps.len()));
        if s.steps.len() < MAX_STEP_LFO_STEPS
            && ui.small_button("+").on_hover_text("Add step").clicked()
        {
            s.steps.push(NormalizedValue::new(0.5));
        }
        if !s.steps.is_empty() && ui.small_button("−").on_hover_text("Remove step").clicked() {
            s.steps.pop();
        }
    });
    // One small vertical drag per step level.
    ui.horizontal_wrapped(|ui| {
        for step in &mut s.steps {
            let mut v = step.as_f32();
            let resp = ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(0.0..=1.0)
                        .speed(0.01)
                        .fixed_decimals(2),
                )
                .on_hover_text("Value emitted on this step (0..1).");
            *any_dragged |= resp.dragged();
            *step = NormalizedValue::new(v);
        }
    });
}

const ENVELOPE_TRIGGERS: [(EnvelopeTrigger, &str); 2] = [
    (EnvelopeTrigger::SourceOnset, "Source"),
    (EnvelopeTrigger::StreamOnset, "Stream"),
];

fn edit_note_envelope(
    ui: &mut egui::Ui,
    salt: usize,
    e: &mut NoteEnvelope,
    any_dragged: &mut bool,
) {
    labeled_row(ui, "Attack", |ui| {
        ticks_drag(ui, &mut e.attack, 0..=15_360, any_dragged)
            .on_hover_text("Rise time from 0 to peak, in ticks (960 = 1 beat).");
        ui.label("Decay");
        ticks_drag(ui, &mut e.decay, 0..=15_360, any_dragged)
            .on_hover_text("Fall time from peak back to 0, in ticks (960 = 1 beat).");
    });
    ui.horizontal(|ui| {
        knob_normalized(ui, "Peak", &mut e.peak, any_dragged)
            .on_hover_text("Peak level reached at the end of attack (0..1).");
    });
    labeled_row(ui, "Trigger", |ui| {
        enum_combo(
            ui,
            (salt, "env_trigger"),
            &mut e.trigger,
            &ENVELOPE_TRIGGERS,
        )
        .on_hover_text("What restarts the envelope: source-note onsets or transformed onsets.");
    });
    dim_label(
        ui,
        match e.trigger {
            EnvelopeTrigger::SourceOnset => "Retriggered by source-note onsets.",
            EnvelopeTrigger::StreamOnset => {
                "Retriggered by transformed onsets (e.g. per arp step) — costlier, \
                 and the look-back is capped at 1 beat past the last onset."
            }
        },
    );
}
