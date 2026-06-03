//! Tracker view — a vertical, step/row-based read-only lens on a single pattern,
//! toggleable against the horizontal piano roll (see `plans/tracker-view-plan.md`, T1).
//!
//! Rows = time steps (one per `ticks_per_row`). Columns = a row/time gutter, one per
//! polyphony/voice lane (all feeding the pattern's single instrument), and one per
//! automation lane. The automation curve is drawn behind each cell as a per-cell
//! segment, so adjacent cells join into a continuous curve without aligning an
//! overlay to the virtualized table geometry.
//!
//! T1 is **read-only**: it renders, it does not edit. It reuses the same
//! `PianoRollData` snapshot as the piano roll — a different lens, not a second store.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use parking_lot::RwLock;
use synth_core::NormalizedValue;
use synth_engine::EngineHandle;
use synth_sequencer::{PatternTick, Song};

use super::{
    AutomationPointSnapshot, PianoRollData, PianoRollNote, SequencerViewState,
    draw_pattern_instrument_transport,
};
use crate::gui::instrument_rack::InstrumentUiState;
use crate::gui::theme::theme;

/// Row height in pixels at zoom 1.0. Taller than a piano-roll semitone row because
/// each tracker row carries text.
const TRACKER_ROW_HEIGHT: f32 = 18.0;

/// Render the tracker view for one pattern. The grid is read-only (T1); the toolbar
/// row (instrument selector + mini-transport) is the same shared control the piano
/// roll uses.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_tracker(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    playhead_tick: Option<PatternTick>,
    is_playing: bool,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[InstrumentUiState],
) {
    let t = theme();

    // Shared toolbar row: pattern name + instrument selector + mini-transport.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            RichText::new(&data.pattern_name)
                .size(14.0)
                .color(t.colors.accent_cyan),
        );
        draw_pattern_instrument_transport(
            ui,
            data,
            handle,
            song,
            view_state,
            instruments,
            is_playing,
        );
    });
    ui.separator();

    let tpr = u32::from(data.ticks_per_row).max(1);
    let n_rows = (data.length_ticks.0.div_ceil(tpr)).max(1) as usize;
    let row_h = (TRACKER_ROW_HEIGHT * view_state.pr_zoom_y).clamp(12.0, 48.0);
    let ticks_per_beat = data.time_sig.ticks_per_beat().max(1);
    let playhead_row = playhead_tick.map(|p| (p.0 / tpr) as usize);

    // Keep repainting while playing so the playhead row and auto-follow track the
    // engine's async transport updates.
    if is_playing {
        ui.ctx().request_repaint();
    }

    // Assign each note to a voice lane (greedy interval coloring) and index notes by
    // their start row so each cell lookup is cheap.
    let (lane_of_note, n_lanes) = assign_voice_lanes(&data.notes, tpr);
    let mut notes_by_start_row: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, note) in data.notes.iter().enumerate() {
        let row = (note.start_tick.0 / tpr) as usize;
        notes_by_start_row.entry(row).or_default().push(idx);
    }

    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(44.0)); // row/time gutter
    for _ in 0..n_lanes {
        builder = builder.column(Column::initial(72.0).at_least(40.0));
    }
    for _ in 0..data.automation_lanes.len() {
        builder = builder.column(Column::initial(80.0).at_least(52.0));
    }

    // Auto-follow: keep the playhead row in view during playback. Guard `prow`
    // against `n_rows` so a playhead resting exactly at the pattern end can't ask
    // the table to scroll to a non-existent row.
    if is_playing
        && view_state.auto_follow_playhead
        && let Some(prow) = playhead_row
        && prow < n_rows
    {
        builder = builder.scroll_to_row(prow, Some(egui::Align::Center));
    }

    builder
        .header(row_h.max(20.0), |mut header| {
            // Header cells get an explicit accent color + background band so they
            // stand out from the body (the global theme forces all plain text to
            // `text_primary`, so a bare `ui.strong` would blend into the note cells).
            let header_label = |ui: &mut egui::Ui, text: String| {
                let rect = ui.max_rect();
                ui.painter().rect_filled(rect, 0.0, t.colors.bg_dark);
                ui.label(RichText::new(text).strong().color(t.colors.accent_cyan))
            };
            header.col(|ui| {
                header_label(ui, "Row".to_string());
            });
            for lane in 0..n_lanes {
                header.col(|ui| {
                    header_label(ui, format!("V{}", lane + 1));
                });
            }
            for lane in &data.automation_lanes {
                header.col(|ui| {
                    let name = lane.target.display_name();
                    header_label(ui, name.clone()).on_hover_text(name);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, n_rows, |mut row| {
                let r = row.index();
                let row_tick = (r as u32) * tpr;
                let on_beat = row_tick.is_multiple_of(ticks_per_beat);
                let is_playhead = playhead_row == Some(r);

                // Gutter: row number, beat emphasis, playhead marker.
                row.col(|ui| {
                    let mut num = RichText::new(format!("{r:03}")).monospace();
                    num = if is_playhead {
                        num.color(t.colors.accent_yellow).strong()
                    } else if on_beat {
                        num.color(t.colors.text_primary).strong()
                    } else {
                        num.color(t.colors.text_dim)
                    };
                    ui.horizontal(|ui| {
                        if is_playhead {
                            ui.label(RichText::new("\u{25b6}").color(t.colors.accent_yellow));
                        }
                        ui.label(num);
                    });
                });

                // Voice columns.
                for lane in 0..n_lanes {
                    row.col(|ui| {
                        let hit = notes_by_start_row
                            .get(&r)
                            .and_then(|v| v.iter().find(|&&i| lane_of_note[i] == lane).copied());
                        match hit {
                            Some(idx) => draw_note_cell(ui, &data.notes[idx], tpr),
                            None => {
                                ui.label(
                                    RichText::new("\u{00b7}")
                                        .color(t.colors.text_dim)
                                        .monospace(),
                                );
                            }
                        }
                    });
                }

                // Automation columns: numeric value + per-cell curve segment behind it.
                for lane in &data.automation_lanes {
                    row.col(|ui| {
                        let top = sample_at(&lane.points, PatternTick(row_tick));
                        let bot = sample_at(&lane.points, PatternTick(row_tick + tpr));
                        let rect = ui.max_rect();
                        if let (Some(a), Some(b)) = (top, bot) {
                            let x = |v: f32| rect.left() + v.clamp(0.0, 1.0) * rect.width();
                            ui.painter().line_segment(
                                [
                                    egui::pos2(x(a.as_f32()), rect.top()),
                                    egui::pos2(x(b.as_f32()), rect.bottom()),
                                ],
                                egui::Stroke::new(1.5, t.colors.accent_cyan),
                            );
                        }
                        let label = top.map_or_else(
                            || "\u{2014}".to_string(),
                            |v| format!("{:.2}", v.as_f32()),
                        );
                        ui.label(
                            RichText::new(label)
                                .color(t.colors.text_secondary)
                                .monospace(),
                        );
                    });
                }
            });
        });
}

/// Render one note into a voice cell: name, velocity, off-grid + expression markers.
fn draw_note_cell(ui: &mut egui::Ui, note: &PianoRollNote, tpr: u32) {
    let t = theme();
    let off_grid = !note.start_tick.0.is_multiple_of(tpr);
    // Velocity as a 0–100 percentage, matching how the piano roll shows it.
    let velocity_pct = (note.velocity.as_f32() * 100.0).round() as u16;
    ui.horizontal(|ui| {
        if off_grid {
            ui.label(RichText::new("~").color(t.colors.accent_orange))
                .on_hover_text(format!("Off-grid: tick {}", note.start_tick.0));
        }
        ui.label(
            RichText::new(format!("{}", note.pitch))
                .color(t.colors.text_primary)
                .monospace(),
        );
        ui.label(
            RichText::new(format!("{velocity_pct:>3}"))
                .color(t.colors.text_dim)
                .monospace(),
        );
        if note.legato {
            ui.label(RichText::new("L").color(t.colors.accent_cyan).small())
                .on_hover_text("Legato");
        }
        if note.glide.is_some() {
            ui.label(RichText::new("G").color(t.colors.accent_purple).small())
                .on_hover_text("Glide");
        }
        if note.expression.is_some() {
            ui.label(
                RichText::new("\u{2022}")
                    .color(t.colors.accent_green)
                    .small(),
            )
            .on_hover_text("Expression");
        }
    });
}

/// Greedy interval-coloring lane assignment: each note takes the lowest voice lane
/// free at its start tick. Read-only/computed for T1; T2 will store a stable lane
/// index on `Note`. Returns the per-note lane index (parallel to `notes`) and the
/// lane count (at least 1, so one empty voice column always shows).
fn assign_voice_lanes(notes: &[PianoRollNote], tpr: u32) -> (Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by_key(|&i| notes[i].start_tick.0);

    let mut lane_free_at: Vec<u32> = Vec::new(); // tick at which each lane frees
    let mut lane_of = vec![0usize; notes.len()];
    for &i in &order {
        let note = &notes[i];
        let start = note.start_tick.0;
        let end = note
            .end_tick
            .map_or_else(|| start.saturating_add(tpr), |e| e.0.max(start + 1));
        let lane = match lane_free_at.iter().position(|&free| free <= start) {
            Some(l) => {
                lane_free_at[l] = end;
                l
            }
            None => {
                lane_free_at.push(end);
                lane_free_at.len() - 1
            }
        };
        lane_of[i] = lane;
    }
    (lane_of, lane_free_at.len().max(1))
}

/// Interpolated automation value at `tick`, sampled from the snapshot points.
/// Mirrors `AutomationLane::value_at` (points are pre-sorted by tick) and reuses
/// `CurveType::interpolate`, so the curve shape matches the engine exactly.
fn sample_at(points: &[AutomationPointSnapshot], tick: PatternTick) -> Option<NormalizedValue> {
    if points.is_empty() {
        return None;
    }
    let idx = points.partition_point(|p| p.tick.0 <= tick.0);
    if idx == 0 {
        return Some(points[0].value);
    }
    if idx >= points.len() {
        return points.last().map(|p| p.value);
    }
    let before = &points[idx - 1];
    let after = &points[idx];
    if after.tick.0 == before.tick.0 {
        return Some(before.value);
    }
    let t = NormalizedValue::new(
        (tick.0 - before.tick.0) as f32 / (after.tick.0 - before.tick.0) as f32,
    );
    Some(before.curve.interpolate(before.value, after.value, t))
}
