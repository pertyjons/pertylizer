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

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
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

// ============================================================================
// View adapter — data → display mapping
//
// These pure functions and the `TrackerColors` palette own every data→text and
// data→color decision; the render path below never formats a value or picks a
// color inline. Static cell glyphs are returned as `Cow::Borrowed` so the
// per-row hot path allocates only for genuinely dynamic cells (pitch, velocity,
// automation value). Mirrors the `render_cell_text` / `cell_color` split from
// the original tracker experiment, adapted to the note-list + voice-lane model.
// ============================================================================

/// Glyph for a voice lane with no note starting on this row.
const EMPTY_VOICE: &str = "\u{00b7}"; // ·
/// Placeholder for an automation lane with no value at this row.
const EMPTY_AUTOMATION: &str = "\u{2014}"; // —
/// Marker prepended to a note whose start tick falls between rows.
const OFF_GRID_MARK: &str = "~";
/// Playhead caret shown in the row gutter.
const PLAYHEAD_CARET: &str = "\u{25b6}"; // ▶
/// Legato marker.
const LEGATO_MARK: &str = "L";
/// Glide marker.
const GLIDE_MARK: &str = "G";
/// Per-note expression marker.
const EXPRESSION_MARK: &str = "\u{2022}"; // •

/// Zero-padded row index for the gutter (always dynamic).
fn row_number_text(row: usize) -> String {
    format!("{row:03}")
}

/// Note name as shown in a voice cell.
fn pitch_text(note: &PianoRollNote) -> String {
    format!("{}", note.pitch)
}

/// Velocity as a right-aligned 0–100 percentage, matching the piano roll.
fn velocity_text(note: &PianoRollNote) -> String {
    let pct = (note.velocity.as_f32() * 100.0).round() as u16;
    format!("{pct:>3}")
}

/// Automation value at a row: the sampled value to two decimals, or the static
/// placeholder when the lane has no points covering this row.
fn automation_value_text(value: Option<NormalizedValue>) -> Cow<'static, str> {
    value.map_or(Cow::Borrowed(EMPTY_AUTOMATION), |v| {
        Cow::Owned(format!("{:.2}", v.as_f32()))
    })
}

/// Palette for the tracker grid, snapshotted once per frame from the active
/// theme so the per-cell render path takes no theme-lock. `Color32` is `Copy`,
/// so the whole struct is cheap to capture into the row/cell closures.
#[derive(Clone, Copy)]
struct TrackerColors {
    header_bg: Color32,
    header_fg: Color32,
    playhead: Color32,
    row_beat: Color32,
    row_dim: Color32,
    empty: Color32,
    note: Color32,
    velocity: Color32,
    off_grid: Color32,
    legato: Color32,
    glide: Color32,
    expression: Color32,
    automation: Color32,
    automation_curve: Color32,
    /// Full-width tint behind the row the cursor is on.
    cursor_row: Color32,
    /// Fill behind the single cell the cursor occupies (drawn over `cursor_row`).
    cursor_cell: Color32,
    /// Outline around the cursor cell + accent for its column header.
    cursor_border: Color32,
}

impl TrackerColors {
    fn from_theme() -> Self {
        let c = &theme().colors;
        Self {
            header_bg: c.bg_dark,
            header_fg: c.accent_cyan,
            playhead: c.accent_yellow,
            row_beat: c.text_primary,
            row_dim: c.text_dim,
            empty: c.text_dim,
            note: c.text_primary,
            velocity: c.text_dim,
            off_grid: c.accent_orange,
            legato: c.accent_cyan,
            glide: c.accent_purple,
            expression: c.accent_green,
            automation: c.text_secondary,
            automation_curve: c.accent_cyan,
            cursor_row: c.bg_widget,
            cursor_cell: c.accent_primary.gamma_multiply(0.22),
            cursor_border: c.border_selected,
        }
    }

    /// Paint the cursor background for one cell: a row-wide tint plus, on the
    /// cursor cell itself, a translucent fill and outline. No-op off the cursor
    /// row. Drawn before the cell content so text/curves stay on top.
    fn paint_cursor(&self, ui: &egui::Ui, is_cursor_row: bool, is_cursor_cell: bool) {
        if !is_cursor_row {
            return;
        }
        let rect = ui.max_rect();
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, self.cursor_row);
        if is_cursor_cell {
            painter.rect_filled(rect, 0.0, self.cursor_cell);
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, self.cursor_border),
                egui::StrokeKind::Inside,
            );
        }
    }

    /// Gutter row-number color: playhead row, beat-aligned row, or off-beat.
    fn row_number(&self, is_playhead: bool, on_beat: bool) -> Color32 {
        if is_playhead {
            self.playhead
        } else if on_beat {
            self.row_beat
        } else {
            self.row_dim
        }
    }
}

// ============================================================================
// Cursor model (T2 editing scaffold)
//
// The cursor is a (row, column) position the user moves with arrow keys / clicks.
// T1 only *highlights* it; editing (T2) will read/write the cell under it. The
// column is stored as a flat selectable-column index (the row/time gutter is not
// selectable): `0..n_lanes` address voice lanes, `n_lanes..n_lanes+n_auto` address
// automation lanes. Lane and row counts vary per frame (the snapshot changes), so
// the persisted cursor is clamped to the current grid shape every frame.
// ============================================================================

/// A typed grid column, resolved from the cursor's flat index for the frame's
/// current lane layout. Voice/automation lanes are addressed by their own index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackerColumn {
    Voice(usize),
    Automation(usize),
}

/// Cursor position in the tracker grid. Persisted on `SequencerViewState` so it
/// survives across frames and view toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TrackerCursor {
    /// Row index (time step).
    pub row: usize,
    /// Flat selectable-column index (see module note); 0 = first voice lane.
    pub col: usize,
}

impl TrackerCursor {
    /// Clamp row/column into the current grid shape so a shrunk pattern or a
    /// removed lane can't leave the cursor pointing past the grid.
    fn clamp(&mut self, n_rows: usize, n_cols: usize) {
        self.row = self.row.min(n_rows.saturating_sub(1));
        self.col = self.col.min(n_cols.saturating_sub(1));
    }

    /// Move the cursor by `delta` rows, clamped to `[0, n_rows)`.
    fn move_rows(&mut self, delta: isize, n_rows: usize) {
        if n_rows == 0 {
            return;
        }
        let max = (n_rows - 1) as isize;
        self.row = (self.row as isize + delta).clamp(0, max) as usize;
    }

    /// Move the cursor by `delta` columns, clamped to `[0, n_cols)`.
    fn move_cols(&mut self, delta: isize, n_cols: usize) {
        if n_cols == 0 {
            return;
        }
        let max = (n_cols - 1) as isize;
        self.col = (self.col as isize + delta).clamp(0, max) as usize;
    }

    /// Resolve the flat column index into a typed voice/automation column for the
    /// frame's lane layout.
    fn resolved(&self, n_lanes: usize) -> TrackerColumn {
        if self.col < n_lanes {
            TrackerColumn::Voice(self.col)
        } else {
            TrackerColumn::Automation(self.col - n_lanes)
        }
    }
}

/// How far PageUp/PageDown jump, in rows. A musically natural page is a bar, but
/// the tracker doesn't carry rows-per-bar here, so a fixed block is used.
const PAGE_ROWS: isize = 16;

/// Keyboard navigation for the tracker cursor. Returns `true` if the cursor moved
/// this frame (so the caller can scroll it back into view). Keys are only handled
/// when no widget holds keyboard focus, so an inline rename keeps its arrows; the
/// movement keys are consumed so they don't also scroll the table.
///
/// T2 editing note: note/velocity entry and delete will hook in here, gated the
/// same way (no focused widget), writing at `cursor` via the pattern mutators.
fn handle_tracker_keys(
    ui: &egui::Ui,
    cursor: &mut TrackerCursor,
    n_rows: usize,
    n_cols: usize,
) -> bool {
    if ui.memory(|m| m.focused()).is_some() {
        return false;
    }
    use egui::{Key, Modifiers};
    let (up, down, left, right, page_up, page_down, home, end) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowLeft),
            i.consume_key(Modifiers::NONE, Key::ArrowRight),
            i.consume_key(Modifiers::NONE, Key::PageUp),
            i.consume_key(Modifiers::NONE, Key::PageDown),
            i.consume_key(Modifiers::NONE, Key::Home),
            i.consume_key(Modifiers::NONE, Key::End),
        )
    });

    let before = *cursor;
    if up {
        cursor.move_rows(-1, n_rows);
    }
    if down {
        cursor.move_rows(1, n_rows);
    }
    if left {
        cursor.move_cols(-1, n_cols);
    }
    if right {
        cursor.move_cols(1, n_cols);
    }
    if page_up {
        cursor.move_rows(-PAGE_ROWS, n_rows);
    }
    if page_down {
        cursor.move_rows(PAGE_ROWS, n_rows);
    }
    if home {
        cursor.row = 0;
    }
    if end {
        cursor.row = n_rows.saturating_sub(1);
    }
    *cursor != before
}

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
    let colors = TrackerColors::from_theme();

    // Shared toolbar row: pattern name + instrument selector + mini-transport.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            RichText::new(&data.pattern_name)
                .size(14.0)
                .color(colors.header_fg),
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

    // Cursor: clamp to the current grid, then apply keyboard navigation. The
    // selectable column space is the voice lanes followed by the automation lanes
    // (the row/time gutter is not selectable).
    let n_auto = data.automation_lanes.len();
    let n_cols = n_lanes + n_auto;
    view_state.tracker_cursor.clamp(n_rows, n_cols);
    let cursor_moved = handle_tracker_keys(ui, &mut view_state.tracker_cursor, n_rows, n_cols);
    let cursor = view_state.tracker_cursor;
    let cursor_kind = cursor.resolved(n_lanes);

    // Click-to-place: cells record their (row, col) here instead of borrowing
    // `view_state` into the table closures; applied after the table is built.
    let click_target: Cell<Option<(usize, usize)>> = Cell::new(None);

    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(44.0)); // row/time gutter
    for _ in 0..n_lanes {
        builder = builder.column(Column::initial(72.0).at_least(40.0));
    }
    for _ in 0..n_auto {
        builder = builder.column(Column::initial(80.0).at_least(52.0));
    }

    // Scroll target: the playhead while it's being followed, otherwise the cursor
    // when it just moved by keyboard (so navigation keeps the cursor on-screen).
    // `prow` is guarded against `n_rows` so a playhead resting exactly at the
    // pattern end can't ask the table to scroll to a non-existent row.
    let mut scroll_row = None;
    if is_playing
        && view_state.auto_follow_playhead
        && let Some(prow) = playhead_row
        && prow < n_rows
    {
        scroll_row = Some(prow);
    }
    if cursor_moved {
        scroll_row = Some(cursor.row);
    }
    if let Some(row) = scroll_row {
        builder = builder.scroll_to_row(row, Some(egui::Align::Center));
    }

    builder
        .header(row_h.max(20.0), |mut header| {
            // Header cells get an explicit accent color + background band so they
            // stand out from the body (the global theme forces all plain text to
            // `text_primary`, so a bare `ui.strong` would blend into the note cells).
            // The cursor's column header is tinted with the cursor accent.
            let header_label = |ui: &mut egui::Ui, text: String, active: bool| {
                let rect = ui.max_rect();
                ui.painter().rect_filled(rect, 0.0, colors.header_bg);
                let fg = if active {
                    colors.cursor_border
                } else {
                    colors.header_fg
                };
                ui.label(RichText::new(text).strong().color(fg))
            };
            header.col(|ui| {
                header_label(ui, "Row".to_string(), false);
            });
            for lane in 0..n_lanes {
                header.col(|ui| {
                    header_label(
                        ui,
                        format!("V{}", lane + 1),
                        cursor_kind == TrackerColumn::Voice(lane),
                    );
                });
            }
            for (ai, lane) in data.automation_lanes.iter().enumerate() {
                header.col(|ui| {
                    let name = lane.target.display_name();
                    header_label(
                        ui,
                        name.clone(),
                        cursor_kind == TrackerColumn::Automation(ai),
                    )
                    .on_hover_text(name);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, n_rows, |mut row| {
                let r = row.index();
                let row_tick = (r as u32) * tpr;
                let on_beat = row_tick.is_multiple_of(ticks_per_beat);
                let is_playhead = playhead_row == Some(r);
                let is_cursor_row = r == cursor.row;

                // Gutter: row number, beat emphasis, playhead marker. Clicking it
                // moves the cursor to this row, keeping the current column.
                let (_, gutter_resp) = row.col(|ui| {
                    colors.paint_cursor(ui, is_cursor_row, false);
                    let mut num = RichText::new(row_number_text(r))
                        .monospace()
                        .color(colors.row_number(is_playhead, on_beat));
                    if is_playhead || on_beat {
                        num = num.strong();
                    }
                    ui.horizontal(|ui| {
                        if is_playhead {
                            ui.label(RichText::new(PLAYHEAD_CARET).color(colors.playhead));
                        }
                        ui.label(num);
                    });
                });
                if gutter_resp.interact(egui::Sense::click()).clicked() {
                    click_target.set(Some((r, cursor.col)));
                }

                // Voice columns.
                for lane in 0..n_lanes {
                    let is_cursor_cell = is_cursor_row && cursor.col == lane;
                    let (_, resp) = row.col(|ui| {
                        colors.paint_cursor(ui, is_cursor_row, is_cursor_cell);
                        let hit = notes_by_start_row
                            .get(&r)
                            .and_then(|v| v.iter().find(|&&i| lane_of_note[i] == lane).copied());
                        match hit {
                            Some(idx) => draw_note_cell(ui, &data.notes[idx], tpr, &colors),
                            None => {
                                ui.label(
                                    RichText::new(EMPTY_VOICE).color(colors.empty).monospace(),
                                );
                            }
                        }
                    });
                    if resp.interact(egui::Sense::click()).clicked() {
                        click_target.set(Some((r, lane)));
                    }
                }

                // Automation columns: numeric value + per-cell curve segment behind it.
                for (ai, lane) in data.automation_lanes.iter().enumerate() {
                    let flat_col = n_lanes + ai;
                    let is_cursor_cell = is_cursor_row && cursor.col == flat_col;
                    let (_, resp) = row.col(|ui| {
                        colors.paint_cursor(ui, is_cursor_row, is_cursor_cell);
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
                                egui::Stroke::new(1.5, colors.automation_curve),
                            );
                        }
                        ui.label(
                            RichText::new(automation_value_text(top))
                                .color(colors.automation)
                                .monospace(),
                        );
                    });
                    if resp.interact(egui::Sense::click()).clicked() {
                        click_target.set(Some((r, flat_col)));
                    }
                }
            });
        });

    // Apply a click captured during the table pass (deferred so the closures don't
    // need a mutable borrow of `view_state`).
    if let Some((row, col)) = click_target.get() {
        view_state.tracker_cursor.row = row;
        view_state.tracker_cursor.col = col;
    }
}

/// Render one note into a voice cell: name, velocity, off-grid + expression markers.
/// All text and color come from the view adapter; this fn only places widgets.
fn draw_note_cell(ui: &mut egui::Ui, note: &PianoRollNote, tpr: u32, colors: &TrackerColors) {
    let off_grid = !note.start_tick.0.is_multiple_of(tpr);
    ui.horizontal(|ui| {
        if off_grid {
            ui.label(RichText::new(OFF_GRID_MARK).color(colors.off_grid))
                .on_hover_text(format!("Off-grid: tick {}", note.start_tick.0));
        }
        ui.label(
            RichText::new(pitch_text(note))
                .color(colors.note)
                .monospace(),
        );
        ui.label(
            RichText::new(velocity_text(note))
                .color(colors.velocity)
                .monospace(),
        );
        if note.legato {
            ui.label(RichText::new(LEGATO_MARK).color(colors.legato).small())
                .on_hover_text("Legato");
        }
        if note.glide.is_some() {
            ui.label(RichText::new(GLIDE_MARK).color(colors.glide).small())
                .on_hover_text("Glide");
        }
        if note.expression.is_some() {
            ui.label(
                RichText::new(EXPRESSION_MARK)
                    .color(colors.expression)
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
