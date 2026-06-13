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
use parking_lot::RwLock;

use synth_engine::{EngineCommand, EngineHandle};
use synth_sequencer::{Duration as SeqDuration, PatternId, PatternTick, Song, Tick};

use crate::gui::instrument_rack::InstrumentUiState;
use crate::gui::sequencer::{
    SequencerViewState, collect_piano_roll_data, commit_pattern_rename, draw_piano_roll,
    draw_tracker,
};
use crate::gui::theme::theme;
use crate::undo::UndoManager;

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
/// [`SequencerViewState::opened_pattern`]; egui persists the Used/Orphans
/// `CollapsingHeader` open state internally, so this struct only carries the
/// search query and the editor mode toggle.
#[derive(Default)]
pub struct PatternViewState {
    pub search_query: String,
    pub editor_mode: PatternEditorMode,
}

// ============================================================================
// BROWSER DATA SNAPSHOT
// ============================================================================

/// One row in the pattern browser list.
#[derive(Debug, Clone)]
struct PatternBrowserRow {
    id: PatternId,
    name: String,
    /// Lowercased name, cached once for substring filtering and sort
    /// comparison — keeps the per-frame collector allocation-free in the
    /// sort comparator.
    name_lower: String,
    placement_count: u32,
    length_beats: f32,
}

/// Snapshot of the song's patterns partitioned by usage, filtered by search.
#[derive(Debug, Clone, Default)]
struct PatternBrowserData {
    used: Vec<PatternBrowserRow>,
    orphans: Vec<PatternBrowserRow>,
}

/// Build a `PatternBrowserData` snapshot from the shared `Song`.
///
/// Returns `None` if the song is currently write-locked (caller should skip
/// rendering this frame and try again next frame).
fn collect_pattern_browser_data(
    song: &Arc<RwLock<Song>>,
    query: &str,
) -> Option<PatternBrowserData> {
    let song = song.try_read()?;

    let mut counts: HashMap<PatternId, u32> = HashMap::new();
    for placement in song.arrangement() {
        *counts.entry(placement.pattern_id).or_insert(0) += 1;
    }

    let needle = query.to_lowercase();
    let mut data = PatternBrowserData::default();

    for pattern in song.patterns() {
        let name_lower = pattern.name.to_lowercase();
        if !needle.is_empty() && !name_lower.contains(&needle) {
            continue;
        }
        let count = counts.get(&pattern.id).copied().unwrap_or(0);
        let row = PatternBrowserRow {
            id: pattern.id,
            name: pattern.name.clone(),
            name_lower,
            placement_count: count,
            length_beats: pattern.length.as_beats(),
        };
        if count > 0 {
            data.used.push(row);
        } else {
            data.orphans.push(row);
        }
    }

    data.used.sort_by(|a, b| a.name_lower.cmp(&b.name_lower));
    data.orphans.sort_by(|a, b| a.name_lower.cmp(&b.name_lower));

    Some(data)
}

// ============================================================================
// VIEW ENTRY
// ============================================================================

/// Draw the full Pattern view (browser + piano roll).
pub(crate) fn draw_pattern_view(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    seq_view_state: &mut SequencerViewState,
    pattern_view_state: &mut PatternViewState,
    instruments: &[InstrumentUiState],
    undo_manager: &mut UndoManager,
) {
    draw_pattern_browser(ui, song, seq_view_state, pattern_view_state, undo_manager);

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(theme().colors.bg_panel))
        .show_inside(ui, |ui| {
            let Some(pattern_id) = seq_view_state.opened_pattern else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "Select a pattern on the left, or click + to create one.",
                        )
                        .color(theme().colors.text_dim),
                    );
                });
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

            // Editor-mode toggle: piano roll vs tracker (read-only in T1).
            ui.horizontal(|ui| {
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
            });
            ui.separator();

            match pattern_view_state.editor_mode {
                PatternEditorMode::Tracker => {
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
    song: &Arc<RwLock<Song>>,
    seq_view_state: &mut SequencerViewState,
    pattern_view_state: &mut PatternViewState,
    undo_manager: &mut UndoManager,
) {
    egui::Panel::left("pattern_browser")
        .default_size(220.0)
        .min_size(160.0)
        .show_inside(ui, |ui| {
            let t = theme();

            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new(format!("{} Patterns", ri::PIANO_FILL))
                        .color(t.colors.text_primary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(
                            egui::RichText::new(ri::ADD_LINE.to_string())
                                .color(t.colors.accent_primary),
                        )
                        .on_hover_text("New pattern")
                        .clicked()
                    {
                        let new_id = {
                            let mut song_w = song.write();
                            song_w.create_pattern(default_new_pattern_length())
                        };
                        seq_view_state.opened_pattern = Some(new_id);
                    }
                });
            });
            ui.separator();

            ui.add(
                egui::TextEdit::singleline(&mut pattern_view_state.search_query)
                    .hint_text("Search…")
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(t.spacing.xs);

            let Some(data) = collect_pattern_browser_data(song, &pattern_view_state.search_query)
            else {
                return;
            };

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    for (label, rows) in [("Used", &data.used[..]), ("Orphans", &data.orphans[..])]
                    {
                        draw_browser_section(ui, label, rows, song, seq_view_state, undo_manager);
                    }
                });
        });
}

fn draw_browser_section(
    ui: &mut egui::Ui,
    label: &str,
    rows: &[PatternBrowserRow],
    song: &Arc<RwLock<Song>>,
    seq_view_state: &mut SequencerViewState,
    undo_manager: &mut UndoManager,
) {
    let t = theme();
    egui::CollapsingHeader::new(
        egui::RichText::new(format!("{} ({})", label, rows.len()))
            .color(t.colors.text_secondary)
            .small(),
    )
    .id_salt(label)
    .default_open(true)
    .show(ui, |ui| {
        if rows.is_empty() {
            ui.label(
                egui::RichText::new("—")
                    .color(t.colors.text_dim)
                    .italics()
                    .small(),
            );
            return;
        }
        for row in rows {
            draw_browser_row(ui, row, song, seq_view_state, undo_manager);
        }
    });
}

fn draw_browser_row(
    ui: &mut egui::Ui,
    row: &PatternBrowserRow,
    song: &Arc<RwLock<Song>>,
    seq_view_state: &mut SequencerViewState,
    undo_manager: &mut UndoManager,
) {
    let t = theme();
    let is_selected = seq_view_state.opened_pattern == Some(row.id);
    let is_renaming = matches!(
        seq_view_state.editing_pattern_name.as_ref(),
        Some((id, _)) if *id == row.id
    );

    let fill = if is_selected {
        t.colors.bg_widget
    } else {
        egui::Color32::TRANSPARENT
    };
    let frame = egui::Frame::NONE
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .corner_radius(egui::CornerRadius::same(3));

    let response = frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if is_renaming {
                    if let Some((_, name_buf)) = seq_view_state.editing_pattern_name.as_mut() {
                        let resp = ui
                            .add(egui::TextEdit::singleline(name_buf).desired_width(f32::INFINITY));
                        if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            commit_pattern_rename(song, undo_manager, row.id, name_buf.clone());
                            seq_view_state.editing_pattern_name = None;
                        } else if !resp.has_focus() {
                            resp.request_focus();
                        }
                    }
                } else {
                    let name_text = if row.name.is_empty() {
                        format!("pattern-{}", row.id.0)
                    } else {
                        row.name.clone()
                    };
                    let name_color = if is_selected {
                        t.colors.accent_cyan
                    } else {
                        t.colors.text_primary
                    };
                    ui.add(
                        egui::Label::new(egui::RichText::new(name_text).color(name_color))
                            .truncate(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if row.placement_count > 0 {
                            ui.label(
                                egui::RichText::new(format!("{}×", row.placement_count))
                                    .color(t.colors.text_dim)
                                    .small(),
                            );
                        }
                        ui.label(
                            egui::RichText::new(format!("{:.0}b", row.length_beats))
                                .color(t.colors.text_dim)
                                .small(),
                        );
                    });
                }
            });
        })
        .response
        .interact(egui::Sense::click());

    if !is_renaming && response.clicked() {
        seq_view_state.opened_pattern = Some(row.id);
    }

    response.context_menu(|ui| {
        if ui.button("Rename").clicked() {
            seq_view_state.editing_pattern_name = Some((row.id, row.name.clone()));
            ui.close();
        }
        if ui.button("Duplicate").clicked() {
            let new_id = {
                let mut song_w = song.write();
                song_w.duplicate_pattern(row.id)
            };
            if let Some(new_id) = new_id {
                seq_view_state.opened_pattern = Some(new_id);
            }
            ui.close();
        }
        ui.separator();
        if ui
            .button(egui::RichText::new("Delete").color(t.colors.accent_red))
            .clicked()
        {
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
            ui.close();
        }
    });
}
