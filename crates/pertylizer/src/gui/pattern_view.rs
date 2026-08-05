//! Pattern view — pattern browser + full-window piano roll for orphan and
//! placed patterns alike.
//!
//! Layout:
//! - Left SidePanel: pattern browser (search, [+], Used/Orphans groups)
//! - CentralPanel: full-window piano roll for the currently-opened pattern
//!
//! Selection (`SequencerViewState.opened_pattern`) is shared with the Seq
//! view's bottom-panel piano roll so switching tabs preserves the open
//! pattern.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui;
use egui_remixicon::icons as ri;

use synth_engine::{EngineCommand, EngineHandle};
use synth_sequencer::{Duration as SeqDuration, PatternId, PatternTick, Tick};

use crate::gui::instrument_rack::InstrumentUiState;
use crate::gui::list_panel;
use crate::gui::sequencer::{
    SequencerViewState, collect_piano_roll_data, commit_pattern_description, commit_pattern_rename,
    draw_note_fx_panel, draw_piano_roll, draw_tracker,
};
use crate::gui::theme::theme;
use crate::gui::widgets::{danger_button, empty_state, selectable_toggle};
use crate::undo::{UndoAction, UndoManager};

// ============================================================================
// VIEW STATE
// ============================================================================

/// Which editor renders the opened pattern in the Pattern tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatternEditorMode {
    /// Horizontal piano roll (the default).
    #[default]
    PianoRoll,
    /// Vertical, step/row-based tracker (read-only in T1).
    Tracker,
}

/// UI state owned by `SynthApp` for the Pattern tab.
///
/// The currently-opened pattern lives on
/// `SequencerViewState::opened_pattern`; egui persists the Used/Orphans
/// `CollapsingHeader` open state internally, so this struct only carries the
/// search query and the editor mode toggle.
#[derive(Default)]
pub struct PatternViewState {
    pub search_query: String,
    pub editor_mode: PatternEditorMode,
    /// Open pattern "Rename / edit…" popup, if any.
    pub editing: Option<PatternEditState>,
}

/// Buffers for the pattern edit popup (name + description), opened from a row's
/// kebab menu. Edits commit to the song on change.
pub struct PatternEditState {
    pub id: PatternId,
    pub name: String,
    pub description: String,
}

// ============================================================================
// BROWSER DATA SNAPSHOT
// ============================================================================

/// One row in the pattern browser list.
#[derive(Debug, Clone)]
struct PatternBrowserRow {
    id: PatternId,
    /// The stored name — may be empty (a freshly created pattern has none); the
    /// rename dialog edits this one.
    name: String,
    /// What the row shows and what the search box matches: `name`, or the
    /// `pattern-<id>` fallback for an unnamed pattern (so a search query can't
    /// hide every unnamed pattern).
    display_name: String,
    placement_count: u32,
    length_beats: f32,
}

/// Build the flat pattern list from the shared `Song`, in natural (song) order —
/// matching the Instruments and Samples lists. Unused patterns render dimmed in
/// place rather than being sorted to the bottom. Returns `None` if the song is
/// currently write-locked (caller should skip rendering this frame).
fn collect_pattern_browser_data(
    song: &Arc<synth_sequencer::SharedSong>,
) -> Option<Vec<PatternBrowserRow>> {
    let song = song.try_read()?;

    let mut counts: HashMap<PatternId, u32> = HashMap::new();
    for placement in song.arrangement() {
        *counts.entry(placement.pattern_id).or_insert(0) += 1;
    }

    let mut rows = Vec::new();

    for pattern in song.patterns() {
        let display_name = if pattern.name.is_empty() {
            format!("pattern-{}", pattern.id.0)
        } else {
            pattern.name.clone()
        };
        rows.push(PatternBrowserRow {
            id: pattern.id,
            name: pattern.name.clone(),
            display_name,
            placement_count: counts.get(&pattern.id).copied().unwrap_or(0),
            length_beats: pattern.length.as_beats(),
        });
    }

    Some(rows)
}

// ============================================================================
// VIEW ENTRY
// ============================================================================

/// Draw the full Pattern view (browser + piano roll).
pub(crate) fn draw_pattern_view(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    song: &Arc<synth_sequencer::SharedSong>,
    seq_view_state: &mut SequencerViewState,
    pattern_view_state: &mut PatternViewState,
    instruments: &[InstrumentUiState],
    undo_manager: &mut UndoManager,
) {
    draw_pattern_browser(ui, song, seq_view_state, pattern_view_state, undo_manager);
    draw_pattern_edit_window(&ui.ctx().clone(), song, pattern_view_state, undo_manager);

    // Note FX panel (right-docked; declared before the central panel so it spans
    // the full height to the right of both editors). Shown for the opened
    // pattern when toggled on — works for both the tracker and piano-roll
    // editor modes.
    if seq_view_state.note_fx_panel_open
        && let Some(pattern_id) = seq_view_state.opened_pattern
    {
        egui::Panel::right("note_fx_panel")
            .resizable(true)
            .min_size(190.0)
            .default_size(290.0)
            .show(ui, |ui| {
                draw_note_fx_panel(ui, song, seq_view_state, undo_manager, pattern_id);
            });
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(theme().colors.bg_panel))
        .show(ui, |ui| {
            let Some(pattern_id) = seq_view_state.opened_pattern else {
                empty_state(
                    ui,
                    "Select a pattern on the left, or click + to create one.",
                );
                return;
            };

            let Some(data) = collect_piano_roll_data(song, pattern_id) else {
                ui.label(egui::RichText::new("Pattern not found").color(theme().colors.text_dim));
                seq_view_state.close_piano_roll();
                handle.send(EngineCommand::SetSoloPattern(None));
                handle.send(EngineCommand::SetPreviewPattern(None));
                return;
            };

            let is_playing = handle.state.transport.is_playing();
            let current_tick = Tick(handle.state.transport.get_ticks());
            let preview_pid = handle.state.transport.preview_pattern();
            let pattern_playhead_tick = if preview_pid == Some(pattern_id) {
                song.try_read()
                    .and_then(|s| s.pattern(pattern_id).map(|p| p.length))
                    .map(|len| {
                        let length = u64::from(len.0.max(1));
                        #[allow(clippy::cast_possible_truncation)]
                        PatternTick((current_tick.0 % length) as u32)
                    })
            } else {
                song.try_read()
                    .and_then(|s| s.pattern_playhead_for(pattern_id, current_tick))
            };

            match pattern_view_state.editor_mode {
                PatternEditorMode::Tracker => {
                    // The tracker has no toolbar of its own, so the docked context
                    // bar here carries the mode selector + the Note FX toggle. (In
                    // piano-roll mode those live in the piano-roll's own row 1.)
                    crate::gui::toolbar::top(ui, "pattern_toolbar", |ui| {
                        ui.selectable_value(
                            &mut pattern_view_state.editor_mode,
                            PatternEditorMode::PianoRoll,
                            "Piano roll",
                        );
                        ui.selectable_value(
                            &mut pattern_view_state.editor_mode,
                            PatternEditorMode::Tracker,
                            "Tracker",
                        );
                        ui.separator();
                        let has_graph = song
                            .try_read()
                            .and_then(|s| s.pattern(pattern_id).map(|p| p.note_graph().is_some()))
                            .unwrap_or(false);
                        let label = if has_graph { "Note FX ●" } else { "Note FX" };
                        selectable_toggle(
                            ui,
                            &mut seq_view_state.note_fx_panel_open,
                            label,
                            "Show/hide the Note FX panel (bind a note graph to this pattern)",
                        );
                    });
                    draw_tracker(
                        ui,
                        &data,
                        pattern_playhead_tick,
                        is_playing,
                        handle,
                        song,
                        seq_view_state,
                        instruments,
                        undo_manager,
                    );
                }
                PatternEditorMode::PianoRoll => {
                    if !draw_piano_roll(
                        ui,
                        &data,
                        pattern_playhead_tick,
                        is_playing,
                        handle,
                        song,
                        seq_view_state,
                        instruments,
                        undo_manager,
                        Some(&mut pattern_view_state.editor_mode),
                    ) {
                        seq_view_state.close_piano_roll();
                        handle.send(EngineCommand::SetSoloPattern(None));
                        handle.send(EngineCommand::SetPreviewPattern(None));
                    }
                }
            }
        });
}

// ============================================================================
// BROWSER (left SidePanel)
// ============================================================================

/// 4 bars at the default time signature.
const fn default_new_pattern_length() -> SeqDuration {
    SeqDuration(SeqDuration::WHOLE.0 * 4)
}

fn draw_pattern_browser(
    ui: &mut egui::Ui,
    song: &Arc<synth_sequencer::SharedSong>,
    seq_view_state: &mut SequencerViewState,
    pattern_view_state: &mut PatternViewState,
    undo_manager: &mut UndoManager,
) {
    egui::Panel::left("pattern_browser")
        .default_size(list_panel::DEFAULT_WIDTH)
        .min_size(list_panel::MIN_WIDTH)
        .show(ui, |ui| {
            // Header + search pinned to the top.
            let mut add_clicked = false;
            egui::Panel::top("pattern_browser_head").show(ui, |ui| {
                add_clicked = list_panel::header(ui, ri::PIANO_FILL, "Patterns", "New pattern");
                list_panel::search_box(ui, &mut pattern_view_state.search_query);
            });
            if add_clicked {
                let (new_id, created) = {
                    let mut song_w = song.write();
                    let id = song_w.create_pattern(default_new_pattern_length());
                    let created = song_w.pattern(id).cloned();
                    (id, created)
                };
                seq_view_state.opened_pattern = Some(new_id);
                if let Some(pattern) = created {
                    // `AddPattern`'s inverse deletes it again, which is what
                    // undoing a creation means.
                    undo_manager.push(UndoAction::AddPattern {
                        pattern,
                        placements: Vec::new(),
                    });
                }
            }

            let Some(rows) = collect_pattern_browser_data(song) else {
                return;
            };

            list_panel::browser_rows(
                ui,
                &rows,
                &pattern_view_state.search_query,
                "No patterns",
                |row| row.display_name.as_str(),
                |ui, row| {
                    draw_browser_row(
                        ui,
                        row,
                        song,
                        seq_view_state,
                        &mut pattern_view_state.editing,
                        undo_manager,
                    );
                },
            );
        });
}

fn draw_browser_row(
    ui: &mut egui::Ui,
    row: &PatternBrowserRow,
    song: &Arc<synth_sequencer::SharedSong>,
    seq_view_state: &mut SequencerViewState,
    editing: &mut Option<PatternEditState>,
    undo_manager: &mut UndoManager,
) {
    let is_selected = seq_view_state.opened_pattern == Some(row.id);
    let used = row.placement_count > 0;

    let mut edit = false;
    let mut duplicate = false;
    let mut delete = false;

    // Usage tooltip — surfaces what the dimming means.
    let tip = if used {
        format!(
            "Used — {} placement{} · {:.0} beats",
            row.placement_count,
            if row.placement_count == 1 { "" } else { "s" },
            row.length_beats
        )
    } else {
        format!("Unused · {:.0} beats", row.length_beats)
    };
    let outcome = list_panel::browser_row(ui, is_selected, used, &row.display_name, tip, |ui| {
        if ui.button((ri::EDIT_LINE, "Rename / edit…")).clicked() {
            edit = true;
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            duplicate = true;
            ui.close();
        }
        ui.separator();
        if danger_button(ui, "Delete").clicked() {
            delete = true;
            ui.close();
        }
    });

    if edit {
        let description = song
            .try_read()
            .and_then(|s| s.pattern(row.id).map(|p| p.description.clone()))
            .unwrap_or_default();
        *editing = Some(PatternEditState {
            id: row.id,
            name: row.name.clone(),
            description,
        });
    } else if duplicate {
        let created = {
            let mut song_w = song.write();
            song_w
                .duplicate_pattern(row.id)
                .and_then(|id| song_w.pattern(id).cloned().map(|p| (id, p)))
        };
        if let Some((new_id, pattern)) = created {
            seq_view_state.opened_pattern = Some(new_id);
            undo_manager.push(UndoAction::AddPattern {
                pattern,
                placements: Vec::new(),
            });
        }
    } else if delete {
        let captured = {
            let mut song_w = song.write();
            let placements: Vec<_> = song_w
                .arrangement()
                .iter()
                .filter(|p| p.pattern_id == row.id)
                .cloned()
                .collect();
            song_w
                .delete_pattern(row.id)
                .map(|deleted| (deleted, placements))
        };
        if let Some((pat, plcs)) = captured {
            undo_manager.push(crate::undo::UndoAction::DeletePattern {
                pattern: pat,
                placements: plcs,
            });
            if seq_view_state.opened_pattern == Some(row.id) {
                seq_view_state.close_piano_roll();
            }
        }
    } else if outcome.clicked {
        seq_view_state.opened_pattern = Some(row.id);
    }
}

/// Pattern "Rename / edit…" popup (name + description), opened from a row's
/// kebab menu. Mirrors the instrument edit window. Commits on change.
fn draw_pattern_edit_window(
    ctx: &egui::Context,
    song: &Arc<synth_sequencer::SharedSong>,
    pattern_view_state: &mut PatternViewState,
    undo_manager: &mut UndoManager,
) {
    // Drop the popup if the target pattern is gone (e.g. deleted while open).
    let Some(id) = pattern_view_state.editing.as_ref().map(|e| e.id) else {
        return;
    };
    if song
        .try_read()
        .map(|s| s.pattern(id).is_none())
        .unwrap_or(false)
    {
        pattern_view_state.editing = None;
        return;
    }
    let Some(edit) = pattern_view_state.editing.as_mut() else {
        return;
    };
    let t = theme();
    let title = format!("{} Edit pattern", ri::EDIT_LINE);
    let mut open = true;
    let mut name_done = false;

    egui::Window::new(title)
        .id(egui::Id::new(("pattern_edit_window", id.0)))
        .open(&mut open)
        .resizable(true)
        .default_size([360.0, 260.0])
        .show(ctx, |ui| {
            ui.label(egui::RichText::new("Name").color(t.colors.text_dim));
            // Buffer the name in `edit.name` and commit once on defocus, so the
            // rename produces a single undo entry rather than one per keystroke.
            if ui.text_edit_singleline(&mut edit.name).lost_focus() {
                name_done = true;
            }

            ui.add_space(t.spacing.md);

            ui.label(egui::RichText::new("Description").color(t.colors.text_dim));
            if ui
                .add(
                    egui::TextEdit::multiline(&mut edit.description)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                commit_pattern_description(song, id, edit.description.clone());
            }
        });

    // Commit the buffered name once, on defocus or window close. Skip empties so
    // a cleared field can't blank the pattern name. `commit_pattern_rename` is a
    // no-op when unchanged, so calling on both paths is safe.
    let new_name = edit.name.clone();
    if (name_done || !open) && !new_name.is_empty() {
        commit_pattern_rename(song, undo_manager, id, new_name);
    }
    if !open {
        pattern_view_state.editing = None;
    }
}
