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
use synth_core::{Milliseconds, NormalizedValue, Semitones};
use synth_engine::EngineHandle;
use synth_sequencer::{
    AutomationLane, AutomationPoint, AutomationTarget, CurveType, Duration as SeqDuration,
    ExpansionBuffer, Glide, GlideFrom, GlideInterp, NoteExpression, NoteId, NoteLane, PatternId,
    PatternTick, Pitch, Song,
};

use super::{
    AutomationPointSnapshot, OrnamentEdit, PianoRollData, PianoRollNote, SequencerViewState,
    draw_automation_target_selector, draw_ornament_popup, draw_pattern_instrument_transport,
    ornament_detail, ornament_tag, preview_note,
};
use crate::gui::input::KEY_MAP;
use crate::gui::instrument_rack::InstrumentUiState;
use crate::gui::theme::theme;
use crate::undo::{UndoAction, UndoManager};

/// Row height in pixels at zoom 1.0. Taller than a piano-roll semitone row because
/// each tracker row carries text.
const TRACKER_ROW_HEIGHT: f32 = 18.0;

/// Vertical-zoom (`pr_zoom_y`) bounds for the tracker's `+`/`1x`/`-` controls.
/// Shared with the piano roll's `pr_zoom_y`; the per-step factor is multiplicative.
const TRACKER_ZOOM_MIN: f32 = 0.5;
const TRACKER_ZOOM_MAX: f32 = 3.0;
const TRACKER_ZOOM_STEP: f32 = 1.2;

/// A cell context-menu action, recorded during the table pass and applied
/// afterwards so the menu closures don't need a mutable borrow of the song/undo.
enum CtxAction {
    DeleteNote(NoteId),
    SetLegato(NoteId, bool),
    SetGlide(NoteId, Option<Glide>),
    EditOrnament(NoteId),
    ClearOrnament(NoteId),
    ClearExprField(NoteId, ExprField),
    ClearAutoPoint {
        target: AutomationTarget,
        tick: PatternTick,
        value: NormalizedValue,
        curve: CurveType,
    },
    DeleteAutoLane(AutomationTarget),
}

/// The default glide installed when toggling glide on from the context menu
/// (mirrors the piano-roll inspector's default).
fn default_glide() -> Glide {
    Glide {
        from: GlideFrom::Semitones(Semitones::new(-2.0)),
        time: Milliseconds::new(100.0),
        interp: GlideInterp::Continuous,
    }
}

/// Tracker-only view state, grouped on `SequencerViewState` as one `tracker`
/// field. Persists across frames and view toggles.
pub(crate) struct TrackerViewState {
    /// Cursor (row + flat column index); navigated with arrows / clicks.
    pub cursor: TrackerCursor,
    /// In-progress numeric entry for the cell under the cursor (`Some` while
    /// typing; committed on Enter, dropped on Esc / when the cursor leaves).
    pub value_buffer: Option<String>,
    /// User-requested minimum voice-column count (the actual count is
    /// `max(derived_from_notes, this, 1)`); "Add voice column" raises it.
    pub voice_columns: usize,
    /// Whether the per-note expression sub-columns are interleaved ("Expr").
    pub show_expression: bool,
    /// Row under the mouse last frame, tinted this frame as a hover highlight.
    pub hovered_row: Option<usize>,
}

impl Default for TrackerViewState {
    fn default() -> Self {
        Self {
            cursor: TrackerCursor::default(),
            value_buffer: None,
            voice_columns: 0,
            show_expression: true,
            hovered_row: None,
        }
    }
}

/// Upper bound on voice columns. Far above any realistic single-instrument
/// polyphony, and well under `NoteLane`'s u8 ceiling (255) so a column index can
/// never saturate `NoteLane::from(usize)` and silently collapse two columns onto
/// one stored lane.
const MAX_TRACKER_VOICE_COLUMNS: usize = 32;

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

/// Multi-line hover tooltip for a voice cell: pitch, velocity, the note's true
/// start tick (with an off-grid flag, since the row is quantized), duration, and
/// legato/glide flags.
fn voice_tooltip(note: &PianoRollNote, tpr: u32) -> String {
    let vel = (note.velocity.as_f32() * 100.0).round() as u16;
    let off_grid = !note.start_tick.0.is_multiple_of(tpr);
    let mut s = format!(
        "{}  ·  vel {vel}%\nstart tick {}{}",
        note.pitch,
        note.start_tick.0,
        if off_grid { "  (off-grid)" } else { "" },
    );
    if let Some(end) = note.end_tick {
        s.push_str(&format!(
            "\nduration {} ticks",
            end.0.saturating_sub(note.start_tick.0)
        ));
    }
    if note.legato {
        s.push_str("\nlegato");
    }
    if note.glide.is_some() {
        s.push_str("\nglide");
    }
    s
}

/// Automation value at a row: the sampled value to two decimals, or the static
/// placeholder when the lane has no points covering this row.
fn automation_value_text(value: Option<NormalizedValue>) -> Cow<'static, str> {
    value.map_or(Cow::Borrowed(EMPTY_AUTOMATION), |v| {
        Cow::Owned(format!("{:.2}", v.as_f32()))
    })
}

/// Placeholder for an expression field with no value (on a row that *has* a note).
const EXPR_UNSET: &str = "--";

/// Expression scalars are shown and entered as percent-ish integers (the stored
/// value ×100): accent 1.2 ↔ "120", gate/probability 0.5 ↔ "50". One definition for
/// both the display (`expr_field_text`) and parse (`set_expr_field`) sides.
const EXPR_DISPLAY_SCALE: f32 = 100.0;

/// One expression field of a note, as shown in its sub-column. Assumes the row
/// carries a note; the caller renders a blank cell when it does not. Scalars are
/// shown as 0–100-ish integers (accent/gate/probability ×100); ghost is a dot.
fn expr_field_text(expr: Option<&NoteExpression>, field: ExprField) -> Cow<'static, str> {
    let Some(e) = expr else {
        return Cow::Borrowed(EXPR_UNSET);
    };
    let pct = |v: f32| Cow::Owned(format!("{:.0}", v * EXPR_DISPLAY_SCALE));
    match field {
        ExprField::Accent => e.accent.map_or(Cow::Borrowed(EXPR_UNSET), pct),
        ExprField::Gate => e
            .gate
            .map_or(Cow::Borrowed(EXPR_UNSET), |g| pct(g.as_f32())),
        ExprField::Ghost => Cow::Borrowed(if e.ghost { EXPRESSION_MARK } else { EXPR_UNSET }),
        ExprField::Probability => e
            .probability
            .map_or(Cow::Borrowed(EXPR_UNSET), |p| pct(p.as_f32())),
    }
}

/// One note-processor's read-only output column (NP lanes, T3.2). One stage per
/// processor in the rack, in execution order; each shows the **cumulative**
/// expansion *after* that processor, so comparing adjacent stages reveals what each
/// processor contributes (the source → after-quantize → after-chord → after-arp
/// chain). Computed offline via `expand_at_tick_through`.
///
/// Row-resolution, by design: sampled only at `row * ticks_per_row`, so generated
/// events that land *between* rows (a strum offset, an arp step finer than a row) are
/// not shown — consistent with how the voice cells key off a note's start row.
struct NpStage {
    /// Column header — the processor kind (`scale_quantize`/`chord`/`arp`/`humanize`).
    label: String,
    /// Cumulative expanded pitches after this stage, parallel to rows `0..n_rows`.
    rows: Vec<Vec<Pitch>>,
}

/// Run the rack offline and build one [`NpStage`] per processor (cumulative output
/// after each stage). Returns an empty `Vec` (cheaply) when the rack is empty — the
/// common case — so the per-row expansion cost is only paid when there is a rack to
/// show. Non-blocking `try_read`; a contended lock just skips the overlay this frame.
/// `expand_at_tick_through` is pure/deterministic, so calling it off the audio thread
/// is safe.
fn compute_np_stages(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
    tpr: u32,
    n_rows: usize,
) -> Vec<NpStage> {
    let Some(song) = song.try_read() else {
        return Vec::new();
    };
    let Some(pattern) = song.pattern(pattern_id) else {
        return Vec::new();
    };
    let n_proc = pattern.processors().len();
    if n_proc == 0 {
        return Vec::new();
    }
    let bpm = song.tempo_at(synth_sequencer::Tick(0));
    let mut buf = ExpansionBuffer::new();
    let mut stages = Vec::with_capacity(n_proc);
    for p in 0..n_proc {
        let label = pattern.processors()[p].kind().to_string();
        let mut rows = Vec::with_capacity(n_rows);
        for r in 0..n_rows {
            let tick = PatternTick((r as u32) * tpr);
            // Cumulative output through processors `0..=p`.
            pattern.expand_at_tick_through(tick, |_| true, bpm, p + 1, &mut buf);
            rows.push(buf.notes().iter().map(|n| n.pitch).collect());
        }
        stages.push(NpStage { label, rows });
    }
    stages
}

/// Compact display of one row's NP output: up to three pitches, then `+N`.
fn np_cell_text(pitches: &[Pitch]) -> String {
    const SHOWN: usize = 3;
    if pitches.is_empty() {
        return String::new();
    }
    let mut s = pitches
        .iter()
        .take(SHOWN)
        .map(|p| format!("{p}"))
        .collect::<Vec<_>>()
        .join(" ");
    if pitches.len() > SHOWN {
        s.push_str(&format!(" +{}", pitches.len() - SHOWN));
    }
    s
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
    /// Divider line drawn at the top edge of a row that starts a new bar.
    bar_line: Color32,
    /// Faint tint behind the row under the mouse pointer.
    hover_row: Color32,
}

/// Per-cell row-chrome flags for [`TrackerColors::paint_cell_chrome`].
#[derive(Clone, Copy)]
struct CellChrome {
    cursor_row: bool,
    cursor_cell: bool,
    bar_start: bool,
    hovered: bool,
    /// The tracker would respond to arrow keys now (no other widget holds focus);
    /// when false the cursor outline draws dimmed.
    active: bool,
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
            bar_line: c.border,
            hover_row: Color32::from_white_alpha(10),
        }
    }

    /// Paint one cell's row chrome before its content: the cursor row tint + cell
    /// outline (on the cursor row/cell), and a divider line at the top edge of a
    /// bar-start row. Each cell paints its own slice, so the bar line is
    /// continuous across the row despite the table's per-cell clipping.
    fn paint_cell_chrome(&self, ui: &egui::Ui, chrome: CellChrome) {
        let rect = ui.max_rect();
        let painter = ui.painter();
        if chrome.hovered && !chrome.cursor_row {
            painter.rect_filled(rect, 0.0, self.hover_row);
        }
        if chrome.cursor_row {
            painter.rect_filled(rect, 0.0, self.cursor_row);
            if chrome.cursor_cell {
                painter.rect_filled(rect, 0.0, self.cursor_cell);
                // Dim the outline when the tracker isn't the active input (some
                // other widget holds keyboard focus), so the cursor reads as idle.
                let border = if chrome.active {
                    self.cursor_border
                } else {
                    self.cursor_border.gamma_multiply(0.4)
                };
                painter.rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0, border),
                    egui::StrokeKind::Inside,
                );
            }
        }
        if chrome.bar_start {
            painter.line_segment(
                [rect.left_top(), rect.right_top()],
                egui::Stroke::new(1.0, self.bar_line),
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

/// A per-note expression sub-column (the four scalar fields of `NoteExpression`;
/// vibrato is excluded — too rich for a single cell, so the note cell keeps its
/// `•` marker for it). Ordered as displayed left-to-right after a voice column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExprField {
    Accent,
    Gate,
    Ghost,
    Probability,
}

impl ExprField {
    /// In display order, matching `EXPR_FIELDS`.
    const ALL: [Self; 4] = [Self::Accent, Self::Gate, Self::Ghost, Self::Probability];

    /// Short column header.
    fn header(self) -> &'static str {
        match self {
            Self::Accent => "Acc",
            Self::Gate => "Gat",
            Self::Ghost => "Gho",
            Self::Probability => "Prb",
        }
    }

    /// Header tooltip — what the field is and how it's entered.
    fn description(self) -> &'static str {
        match self {
            Self::Accent => {
                "Accent — velocity multiplier (entered ×100: 120 = 1.2×). Enter commits, Delete clears."
            }
            Self::Gate => {
                "Gate — note length as a % of its duration (50 = half). Enter commits, Delete clears."
            }
            Self::Ghost => "Ghost — forced-quiet note. Enter/Space toggles, Delete clears.",
            Self::Probability => {
                "Probability — chance the note plays, % (80 = 0.8). Enter commits, Delete clears."
            }
        }
    }
}

/// Number of expression sub-columns shown per voice lane when the "Expr" toggle is
/// on (one per `ExprField::ALL`).
const EXPR_FIELDS: usize = ExprField::ALL.len();

/// A typed grid column, resolved from the cursor's flat index for the frame's
/// current layout. Voice lanes, their optional expression sub-columns, and
/// automation lanes are each addressed by their own index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackerColumn {
    Voice(usize),
    /// Per-note ornament cell for a voice lane (edited via the shared popup).
    Ornament(usize),
    Expr(usize, ExprField),
    Automation(usize),
}

/// Flat selectable-column index where voice lane `lane`'s group begins (its voice
/// cell; expression sub-cells, if shown, follow at `base + 1 + field_index`).
/// Called with `lane == n_lanes` it yields the first automation column. The one
/// definition of the per-voice column layout, shared by `resolved` (decode) and the
/// render/edit paths (encode) so they can't disagree.
fn voice_group_base(lane: usize, cols_per_voice: usize) -> usize {
    lane * cols_per_voice
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

    /// Resolve the flat column index into a typed column for the frame's layout.
    /// Each voice lane owns a contiguous group of `cols_per_voice`: sub-column 0 is
    /// the voice cell, 1 is the always-present ornament cell, and 2.. are the
    /// optional expression sub-columns; the automation lanes follow the groups.
    fn resolved(&self, n_lanes: usize, cols_per_voice: usize) -> TrackerColumn {
        let n_voice_cols = voice_group_base(n_lanes, cols_per_voice);
        if self.col < n_voice_cols {
            let lane = self.col / cols_per_voice;
            let sub = self.col % cols_per_voice;
            match sub {
                0 => TrackerColumn::Voice(lane),
                1 => TrackerColumn::Ornament(lane),
                _ => TrackerColumn::Expr(lane, ExprField::ALL[sub - 2]),
            }
        } else {
            TrackerColumn::Automation(self.col - n_voice_cols)
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

/// Editing keys for the tracker cursor: note entry (computer-keyboard piano) and
/// delete. Quantized to the row grid — the cursor row maps to `row * tpr`, so every
/// write lands on a step. Writes go straight through the pattern mutators with an
/// `UndoAction`, exactly like the piano roll, so undo/redo and audio pick them up
/// for free. Returns `true` if the cursor advanced (so the caller scrolls it back
/// into view). Gated on no focused widget, mirroring `handle_tracker_keys`.
///
/// Lane handling: a note entered in voice column V is stored on lane V. The first
/// lane-assigning insert into a not-yet-organized pattern (all notes on lane 0)
/// first migrates the current greedy display layout into the notes' stored lanes —
/// bundled into the same undo step — so the columns don't reshuffle on the next
/// frame. Edits on an automation column are ignored here (that is T3 numeric entry).
#[allow(clippy::too_many_arguments)]
fn handle_tracker_edit_keys(
    ui: &egui::Ui,
    data: &PianoRollData,
    song: &Arc<RwLock<Song>>,
    handle: &mut EngineHandle,
    undo_manager: &mut UndoManager,
    view_state: &mut SequencerViewState,
    n_lanes: usize,
    cols_per_voice: usize,
    n_rows: usize,
    tpr: u32,
    lane_of_note: &[usize],
    notes_by_start_row: &HashMap<usize, Vec<usize>>,
) -> bool {
    if ui.memory(|m| m.focused()).is_some() {
        return false;
    }
    let cursor = view_state.tracker.cursor;
    match cursor.resolved(n_lanes, cols_per_voice) {
        TrackerColumn::Voice(lane) => {
            // Leaving an automation cell drops any half-typed value.
            view_state.tracker.value_buffer = None;
            edit_voice_cell(
                ui,
                data,
                song,
                handle,
                undo_manager,
                view_state,
                lane,
                cursor.row,
                n_rows,
                tpr,
                lane_of_note,
                notes_by_start_row,
            )
        }
        TrackerColumn::Ornament(lane) => {
            view_state.tracker.value_buffer = None;
            edit_ornament_cell(
                ui,
                data,
                song,
                undo_manager,
                view_state,
                lane,
                cursor.row,
                lane_of_note,
                notes_by_start_row,
            )
        }
        TrackerColumn::Expr(lane, field) => edit_expression_cell(
            ui,
            data,
            song,
            undo_manager,
            view_state,
            lane,
            field,
            cursor.row,
            lane_of_note,
            notes_by_start_row,
        ),
        TrackerColumn::Automation(ai) => edit_automation_cell(
            ui,
            data,
            song,
            undo_manager,
            view_state,
            ai,
            cursor.row,
            tpr,
        ),
    }
}

/// Per-note ornament cell: Enter opens the shared ornament popup for the note
/// under the cursor (baseline = its current ornament); Delete/Backspace clears
/// the ornament. The popup window itself is rendered once per frame by
/// `draw_tracker`. Returns false (never moves the cursor).
#[allow(clippy::too_many_arguments)]
fn edit_ornament_cell(
    ui: &egui::Ui,
    data: &PianoRollData,
    song: &Arc<RwLock<Song>>,
    undo_manager: &mut UndoManager,
    view_state: &mut SequencerViewState,
    lane: usize,
    row: usize,
    lane_of_note: &[usize],
    notes_by_start_row: &HashMap<usize, Vec<usize>>,
) -> bool {
    let Some(idx) = notes_by_start_row
        .get(&row)
        .into_iter()
        .flatten()
        .find(|&&i| lane_of_note[i] == lane)
        .copied()
    else {
        return false;
    };
    let note_id = data.notes[idx].note_id;
    let current = data.notes[idx].ornament;

    let (enter, delete, backspace) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace),
        )
    });

    if enter {
        // Open the shared popup; baseline = the note's current ornament.
        view_state.editing_ornament = Some(OrnamentEdit {
            pattern_id: data.pattern_id,
            note_id,
            before: current,
            current,
        });
    } else if (delete || backspace) && current.is_some() {
        {
            let mut song_w = song.write();
            if let Some(n) = song_w
                .pattern_mut(data.pattern_id)
                .and_then(|p| p.note_mut(note_id))
            {
                n.ornament = None;
            }
        }
        undo_manager.push(UndoAction::SetNoteOrnament {
            pattern_id: data.pattern_id,
            note_id,
            old: current,
            new: None,
        });
    }
    false
}

/// Note entry + delete for a voice cell. A piano key inserts a note at the cursor
/// row on the cursor's voice lane; Delete/Backspace removes the note under the
/// cursor. See `handle_tracker_edit_keys` for the lane-migration rationale.
#[allow(clippy::too_many_arguments)]
fn edit_voice_cell(
    ui: &egui::Ui,
    data: &PianoRollData,
    song: &Arc<RwLock<Song>>,
    handle: &mut EngineHandle,
    undo_manager: &mut UndoManager,
    view_state: &mut SequencerViewState,
    lane: usize,
    row: usize,
    n_rows: usize,
    tpr: u32,
    lane_of_note: &[usize],
    notes_by_start_row: &HashMap<usize, Vec<usize>>,
) -> bool {
    let row_tick = PatternTick(row as u32 * tpr);

    // ── Delete: remove the note under the cursor. ──
    let delete = ui.input_mut(|i| {
        i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
            || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
    });
    if delete {
        let target = notes_by_start_row
            .get(&row)
            .and_then(|v| v.iter().find(|&&i| lane_of_note[i] == lane).copied());
        if let Some(idx) = target {
            let note_id = data.notes[idx].note_id;
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                && let Some(removed) = pattern.note(note_id).map(Into::into)
            {
                pattern.remove_note(note_id);
                undo_manager.push(UndoAction::RemoveNote {
                    pattern_id: data.pattern_id,
                    note: removed,
                });
            }
        }
        return false;
    }

    // ── Note entry: a piano key inserts a note at the cursor's voice column. ──
    let pressed_note = ui.input(|i| {
        if i.modifiers.command || i.modifiers.ctrl {
            return None;
        }
        KEY_MAP
            .iter()
            .find(|(key, _)| i.key_pressed(*key))
            .map(|(_, note)| *note)
    });
    let Some(pitch) = pressed_note.and_then(Pitch::new) else {
        return false;
    };

    // Don't stack a second note on an occupied voice cell: a single-note cell can't
    // show or delete an overlapping duplicate cleanly, and overwriting via
    // remove+add would break redo (re-adding reassigns the note id). Re-typing a
    // cell is therefore Delete-then-enter.
    let occupied = notes_by_start_row
        .get(&row)
        .is_some_and(|v| v.iter().any(|&i| lane_of_note[i] == lane));
    if occupied {
        return false;
    }

    let new_lane = NoteLane::from(lane);
    // Migrate the greedy display layout into stored lanes before the first
    // lane-assigning insert, so existing notes keep their columns. Only runs when
    // the pattern isn't yet lane-organized; `lane_of_note` is the greedy layout here.
    let mut migration: Vec<(synth_sequencer::NoteId, NoteLane, NoteLane)> = Vec::new();
    if !is_lane_organized(&data.notes) {
        for (i, note) in data.notes.iter().enumerate() {
            let greedy = NoteLane::from(lane_of_note[i]);
            if greedy != note.lane {
                migration.push((note.note_id, note.lane, greedy));
            }
        }
    }

    let mut song_w = song.write();
    let Some(pattern) = song_w.pattern_mut(data.pattern_id) else {
        return false;
    };
    for (id, _old, new) in &migration {
        pattern.set_note_lane(*id, *new);
    }
    let note_id = pattern.add_note(row_tick, pitch, view_state.default_velocity);
    pattern.resize_note(note_id, SeqDuration(tpr));
    pattern.set_note_lane(note_id, new_lane);
    let add = pattern.note(note_id).map(|n| UndoAction::AddNote {
        pattern_id: data.pattern_id,
        note: n.into(),
    });
    drop(song_w);

    if let Some(add) = add {
        let mut actions: Vec<UndoAction> = Vec::new();
        if !migration.is_empty() {
            actions.push(UndoAction::SetLaneBatch {
                pattern_id: data.pattern_id,
                changes: migration,
            });
        }
        actions.push(add);
        push_actions(undo_manager, actions);
    }
    preview_note(handle, pitch, view_state.default_velocity);

    // Advance the cursor down one row (vertical tracker), wrapping at the end.
    let next = row + 1;
    view_state.tracker.cursor.row = if next >= n_rows { 0 } else { next };
    true
}

/// Numeric entry + delete for an automation cell. Typing digits/`.` builds a value
/// in `tracker_value_buffer`; Enter commits it as a point at the cursor row's tick
/// (quantized), Esc discards, Delete removes the point at the row. Replacing an
/// existing point keeps its curve and uses `MoveAutomationPoint` so undo restores
/// the old value (a new point uses `AddAutomationPoint`). Never advances the cursor.
#[allow(clippy::too_many_arguments)]
fn edit_automation_cell(
    ui: &egui::Ui,
    data: &PianoRollData,
    song: &Arc<RwLock<Song>>,
    undo_manager: &mut UndoManager,
    view_state: &mut SequencerViewState,
    ai: usize,
    row: usize,
    tpr: u32,
) -> bool {
    let Some(lane) = data.automation_lanes.get(ai) else {
        view_state.tracker.value_buffer = None;
        return false;
    };
    let tick = PatternTick(row as u32 * tpr);
    let target = lane.target.clone();
    // Existing point exactly on this row (value + curve), if any.
    let existing = lane
        .points
        .iter()
        .find(|p| p.tick == tick)
        .map(|p| (p.value, p.curve));

    let (enter, esc, delete, backspace) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace),
        )
    });

    if esc {
        view_state.tracker.value_buffer = None;
        return false;
    }

    if delete {
        view_state.tracker.value_buffer = None;
        if let Some((value, curve)) = existing {
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                && let Some(auto_lane) = pattern.automation.iter_mut().find(|l| l.target == target)
            {
                auto_lane.remove_point(tick);
            }
            drop(song_w);
            undo_manager.push(UndoAction::RemoveAutomationPoint {
                pattern_id: data.pattern_id,
                target,
                tick,
                value,
                curve,
            });
        }
        return false;
    }

    if enter {
        if let Some(buf) = view_state.tracker.value_buffer.take()
            && let Ok(parsed) = buf.parse::<f32>()
        {
            let value = NormalizedValue::new(parsed.clamp(0.0, 1.0));
            // Preserve an existing point's curve on replace; new points get the
            // default curve.
            let curve = existing.map_or_else(CurveType::default, |(_, c)| c);
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                let auto_lane = pattern.get_or_create_automation(target.clone());
                let mut point = AutomationPoint::new(tick, value);
                point.curve = curve;
                auto_lane.add_point(point);
            }
            drop(song_w);
            undo_manager.push(match existing {
                Some((old_value, _)) => UndoAction::MoveAutomationPoint {
                    pattern_id: data.pattern_id,
                    target,
                    old_tick: tick,
                    old_value,
                    new_tick: tick,
                    new_value: value,
                    curve,
                },
                None => UndoAction::AddAutomationPoint {
                    pattern_id: data.pattern_id,
                    target,
                    tick,
                    value,
                    curve,
                },
            });
        }
        return false;
    }

    if backspace {
        if let Some(buf) = view_state.tracker.value_buffer.as_mut() {
            buf.pop();
        }
        return false;
    }
    accumulate_digit_buffer(ui, &mut view_state.tracker.value_buffer);
    false
}

/// Append any digit / `.` characters typed this frame to the shared numeric entry
/// buffer (created on first input), capped at 8 chars. Shared by the automation and
/// expression numeric cells.
fn accumulate_digit_buffer(ui: &egui::Ui, buffer: &mut Option<String>) {
    let typed: String = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.chars().filter(|c| c.is_ascii_digit() || *c == '.')),
                _ => None,
            })
            .flatten()
            .collect()
    });
    if !typed.is_empty() {
        let buf = buffer.get_or_insert_with(String::new);
        if buf.len() < 8 {
            buf.push_str(&typed);
        }
    }
}

/// Write one `NoteExpression` scalar from its displayed (×100) value, or clear it
/// when `displayed` is `None`. Accent is a raw multiplier (clamped 0–4×); gate and
/// probability are normalized 0..1. Ghost is a flag handled by the caller.
fn set_expr_field(expr: &mut NoteExpression, field: ExprField, displayed: Option<f32>) {
    // Descale the displayed (×100) value once; each field then just clamps to its
    // own domain.
    let v = displayed.map(|d| d / EXPR_DISPLAY_SCALE);
    let norm = |x: f32| NormalizedValue::new(x.clamp(0.0, 1.0));
    match field {
        ExprField::Accent => expr.accent = v.map(|x| x.clamp(0.0, 4.0)),
        ExprField::Gate => expr.gate = v.map(norm),
        ExprField::Probability => expr.probability = v.map(norm),
        ExprField::Ghost => {}
    }
}

/// Editing for an expression sub-cell at the cursor. Only acts when a note carries
/// this (row, lane). Numeric fields (accent/gate/probability) use the same digit
/// buffer as automation entry — Enter commits, Esc discards, Delete clears the
/// field. Ghost is a flag toggled by Enter/Space, cleared by Delete. Every write
/// goes through `set_note_expression` (normalizing an empty block to `None`) with a
/// `SetExpressionBatch` undo step. Never advances the cursor.
#[allow(clippy::too_many_arguments)]
fn edit_expression_cell(
    ui: &egui::Ui,
    data: &PianoRollData,
    song: &Arc<RwLock<Song>>,
    undo_manager: &mut UndoManager,
    view_state: &mut SequencerViewState,
    lane: usize,
    field: ExprField,
    row: usize,
    lane_of_note: &[usize],
    notes_by_start_row: &HashMap<usize, Vec<usize>>,
) -> bool {
    // The note this expression cell belongs to (none → nothing to edit).
    let Some(idx) = notes_by_start_row
        .get(&row)
        .into_iter()
        .flatten()
        .find(|&&i| lane_of_note[i] == lane)
        .copied()
    else {
        view_state.tracker.value_buffer = None;
        return false;
    };
    let note_id = data.notes[idx].note_id;
    let old_expr = data.notes[idx].expression;

    let (enter, esc, delete, backspace, space) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Delete),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace),
            i.consume_key(egui::Modifiers::NONE, egui::Key::Space),
        )
    });

    // Apply a mutated block (normalize empty → None) with one undo step.
    let mut commit = |expr: NoteExpression| {
        let new = expr.normalized();
        {
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                pattern.set_note_expression(note_id, new);
            }
        }
        undo_manager.push(UndoAction::SetExpressionBatch {
            pattern_id: data.pattern_id,
            changes: vec![(note_id, old_expr, new)],
        });
    };

    if esc {
        view_state.tracker.value_buffer = None;
        return false;
    }

    // Ghost: a boolean flag — Enter/Space toggles, Delete clears. No value buffer.
    if field == ExprField::Ghost {
        view_state.tracker.value_buffer = None;
        if enter || space {
            let mut e = old_expr.unwrap_or_default();
            e.ghost = !e.ghost;
            commit(e);
        } else if delete && let Some(mut e) = old_expr {
            e.ghost = false;
            commit(e);
        }
        return false;
    }

    // Numeric fields: Delete clears, Enter commits the typed buffer, digits build it.
    if delete {
        view_state.tracker.value_buffer = None;
        if let Some(mut e) = old_expr {
            set_expr_field(&mut e, field, None);
            commit(e);
        }
        return false;
    }
    if enter {
        if let Some(buf) = view_state.tracker.value_buffer.take()
            && let Ok(parsed) = buf.parse::<f32>()
        {
            let mut e = old_expr.unwrap_or_default();
            set_expr_field(&mut e, field, Some(parsed));
            commit(e);
        }
        return false;
    }
    if backspace {
        if let Some(buf) = view_state.tracker.value_buffer.as_mut() {
            buf.pop();
        }
        return false;
    }
    accumulate_digit_buffer(ui, &mut view_state.tracker.value_buffer);
    false
}

/// "Remove empty columns": compact note lanes so there are no gaps, drop any
/// trailing empty voice columns the user added, and remove automation lanes with no
/// points. Voice compaction renumbers stored lanes densely (when lane-organized);
/// the requested column minimum is lowered to the occupied-lane count. All the
/// resulting mutations are bundled into a single undo step.
fn clean_empty_columns(
    data: &PianoRollData,
    song: &Arc<RwLock<Song>>,
    undo_manager: &mut UndoManager,
    view_state: &mut SequencerViewState,
    lane_of_note: &[usize],
) {
    let mut actions: Vec<UndoAction> = Vec::new();

    // ── Voice lanes: compact stored lanes densely (organized patterns only). ──
    // Distinct occupied display lanes, ascending → dense 0..k remap by position.
    let mut occupied: Vec<usize> = lane_of_note.to_vec();
    occupied.sort_unstable();
    occupied.dedup();
    if is_lane_organized(&data.notes) {
        let mut changes: Vec<(synth_sequencer::NoteId, NoteLane, NoteLane)> = Vec::new();
        for note in &data.notes {
            let old = note.lane.as_usize();
            // `occupied` lists every used lane, so the position is always found.
            if let Some(new) = occupied.iter().position(|&l| l == old)
                && new != old
            {
                changes.push((note.note_id, note.lane, NoteLane::from(new)));
            }
        }
        if !changes.is_empty() {
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                for (id, _old, new) in &changes {
                    pattern.set_note_lane(*id, *new);
                }
            }
            drop(song_w);
            actions.push(UndoAction::SetLaneBatch {
                pattern_id: data.pattern_id,
                changes,
            });
        }
    }
    view_state.tracker.voice_columns = occupied.len();

    // ── Automation lanes: remove the ones with no points. ──
    let empty_targets: Vec<_> = data
        .automation_lanes
        .iter()
        .filter(|l| l.points.is_empty())
        .map(|l| l.target.clone())
        .collect();
    if !empty_targets.is_empty() {
        let mut song_w = song.write();
        if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
            for target in &empty_targets {
                if let Some(removed) = pattern.remove_automation_lane(target) {
                    actions.push(UndoAction::RemoveAutomationLane {
                        pattern_id: data.pattern_id,
                        lane: removed,
                    });
                }
            }
        }
    }

    // Bundle every mutation into one undo step (note compaction + lane removals are
    // disjoint, so their order within the composite doesn't matter).
    push_actions(undo_manager, actions);
}

/// Push a batch of undo actions as a single undo step: nothing on empty, the lone
/// action directly, otherwise a `Composite`. Keeps undo granularity consistent
/// across the tracker's bundled edits.
fn push_actions(undo_manager: &mut UndoManager, actions: Vec<UndoAction>) {
    match actions.len() {
        0 => {}
        1 => {
            if let Some(action) = actions.into_iter().next() {
                undo_manager.push(action);
            }
        }
        _ => undo_manager.push(UndoAction::Composite(actions)),
    }
}

/// Render the tracker view for one pattern. Voice cells accept note entry and
/// delete at the cursor; the toolbar row (instrument selector + mini-transport) is
/// the same shared control the piano roll uses.
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
    undo_manager: &mut UndoManager,
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
    // Column controls: add an empty voice column, add an automation column for the
    // selected target, or prune empty columns.
    let (add_voice, add_auto, clean_cols) = ui
        .horizontal(|ui| {
            let add_v = ui
                .button("+ Voice")
                .on_hover_text("Add an empty voice column for note entry")
                .clicked();
            ui.separator();
            draw_automation_target_selector(ui, view_state, data, instruments);
            let add_a = ui
                .button("+ Auto")
                .on_hover_text("Add an automation column for the selected target")
                .clicked();
            ui.separator();
            let clean = ui
                .button("Clean")
                .on_hover_text(
                    "Remove empty columns: compact voice lanes to close gaps and drop \
                     empty automation lanes",
                )
                .clicked();
            ui.separator();
            ui.toggle_value(&mut view_state.tracker.show_expression, "Expr")
                .on_hover_text(
                    "Show per-note expression sub-columns (accent / gate / ghost / probability)",
                );
            ui.separator();
            ui.menu_button("?", |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.label(RichText::new("Tracker — keys & mouse").strong());
                ui.separator();
                ui.label("Navigate: arrows move the cursor; PageUp/Down jump; Home/End to ends.");
                ui.label("Notes: a piano key (A W S E D F …) inserts a note at the cursor; Delete removes it.");
                ui.label("Expression (Acc/Gat/Prb): type digits + Enter (×100; 120 = 1.2×); Delete clears.");
                ui.label("Ghost (Gho): Enter/Space toggles; Delete clears.");
                ui.label("Ornament (Orn): Enter opens the editor; Delete clears.");
                ui.label("Automation: type a value 0–1 + Enter to set a point at the cursor row; Delete removes it.");
                ui.label("Right-click a cell or column header for more actions (delete, clear, legato/glide…).");
                ui.label("Zoom: the + / 1x / - buttons (also scales the cell text).");
            })
            .response
            .on_hover_text("Keyboard & mouse cheat-sheet");
            // Vertical-zoom controls, right-aligned. In a right-to-left layout the
            // first widget is rightmost, so add in reverse to read "+ 1x -".
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("-")
                    .on_hover_text("Zoom out (row height)")
                    .clicked()
                {
                    view_state.pr_zoom_y = (view_state.pr_zoom_y / TRACKER_ZOOM_STEP)
                        .clamp(TRACKER_ZOOM_MIN, TRACKER_ZOOM_MAX);
                }
                if ui.small_button("1x").on_hover_text("Reset zoom").clicked() {
                    view_state.pr_zoom_y = 1.0;
                }
                if ui
                    .small_button("+")
                    .on_hover_text("Zoom in (row height)")
                    .clicked()
                {
                    view_state.pr_zoom_y = (view_state.pr_zoom_y * TRACKER_ZOOM_STEP)
                        .clamp(TRACKER_ZOOM_MIN, TRACKER_ZOOM_MAX);
                }
            });
            (add_v, add_a, clean)
        })
        .inner;
    ui.separator();

    let tpr = u32::from(data.ticks_per_row).max(1);
    let n_rows = (data.length_ticks.0.div_ceil(tpr)).max(1) as usize;
    let row_h = (TRACKER_ROW_HEIGHT * view_state.pr_zoom_y).clamp(12.0, 48.0);
    // Scale the monospace cell text with the zoom so it neither clips nor shrinks
    // away. Set on `ui` before the table builds; the table's cell uis inherit it.
    let cell_font_size = (11.0 * view_state.pr_zoom_y).clamp(9.0, 20.0);
    if let Some(font) = ui
        .style_mut()
        .text_styles
        .get_mut(&egui::TextStyle::Monospace)
    {
        font.size = cell_font_size;
    }
    // Cell/header text is plain `ui.label`s, which default to *selectable* text and
    // therefore sense `click_and_drag`. A selectable label sits on top of the cell
    // background and steals the click — so clicking (or right-clicking) directly on
    // the text would never reach the cell response. Disable label selection table-wide
    // (cell uis inherit this style) so the text is click-through to the cell.
    ui.style_mut().interaction.selectable_labels = false;
    let ticks_per_beat = data.time_sig.ticks_per_beat().max(1);
    let ticks_per_bar = data.time_sig.ticks_per_bar().max(1);
    let playhead_row = playhead_tick.map(|p| (p.0 / tpr) as usize);

    // Keep repainting while playing so the playhead row and auto-follow track the
    // engine's async transport updates.
    if is_playing {
        ui.ctx().request_repaint();
    }

    // Note-processor output columns (read-only, one per processor). Empty — and so
    // no columns — unless the pattern has a processor rack.
    let np_stages = compute_np_stages(song, data.pattern_id, tpr, n_rows);

    // Assign each note to a voice lane and index notes by their start row so each
    // cell lookup is cheap. The shown column count honors the user's requested
    // minimum (Add voice column) on top of what the notes need.
    let (lane_of_note, derived_lanes) = display_lanes(&data.notes, tpr);
    let n_lanes = derived_lanes.max(view_state.tracker.voice_columns).max(1);

    // Column add/prune (applied to the song/view; reflected next frame).
    if add_voice {
        view_state.tracker.voice_columns = (n_lanes + 1).min(MAX_TRACKER_VOICE_COLUMNS);
    }
    if add_auto && let Some(target) = view_state.selected_automation.clone() {
        // Materialize the selected target as an empty lane (a new column), unless
        // it already exists.
        if !data.automation_lanes.iter().any(|l| l.target == target) {
            let lane = AutomationLane::new(target);
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                pattern.add_automation_lane(lane.clone());
            }
            drop(song_w);
            undo_manager.push(UndoAction::AddAutomationLane {
                pattern_id: data.pattern_id,
                lane,
            });
        }
    }
    if clean_cols {
        clean_empty_columns(data, song, undo_manager, view_state, &lane_of_note);
    }

    let mut notes_by_start_row: HashMap<usize, Vec<usize>> = HashMap::new();
    for (idx, note) in data.notes.iter().enumerate() {
        let row = (note.start_tick.0 / tpr) as usize;
        notes_by_start_row.entry(row).or_default().push(idx);
    }

    // Cursor: clamp to the current grid, then apply keyboard navigation. The
    // selectable column space is each voice lane's group (the voice cell + its
    // optional expression sub-cells) followed by the automation lanes (the row/time
    // gutter is not selectable). `cols_per_voice` is the width of one voice group.
    let show_expr = view_state.tracker.show_expression;
    // Voice + always-present ornament cell, plus the optional expression fields.
    let cols_per_voice = 2 + if show_expr { EXPR_FIELDS } else { 0 };
    let n_auto = data.automation_lanes.len();
    let n_cols = n_lanes * cols_per_voice + n_auto;
    let cursor_before_nav = view_state.tracker.cursor;
    view_state.tracker.cursor.clamp(n_rows, n_cols);
    let nav_moved = handle_tracker_keys(ui, &mut view_state.tracker.cursor, n_rows, n_cols);
    if view_state.tracker.cursor != cursor_before_nav {
        // Any cursor shift — keyboard nav OR a clamp from a reshaped grid (lane
        // added/removed) — discards a half-typed automation value: it belonged to
        // the cell we just left, and committing it to the new cell would be wrong.
        view_state.tracker.value_buffer = None;
    }
    let edit_moved = handle_tracker_edit_keys(
        ui,
        data,
        song,
        handle,
        undo_manager,
        view_state,
        n_lanes,
        cols_per_voice,
        n_rows,
        tpr,
        &lane_of_note,
        &notes_by_start_row,
    );
    let cursor_moved = nav_moved || edit_moved;
    let cursor = view_state.tracker.cursor;
    let cursor_kind = cursor.resolved(n_lanes, cols_per_voice);
    // In-progress automation value entry, snapshotted for the (immutable) cell
    // closures so they can show it in the cursor cell without borrowing view_state.
    let entry_buffer = view_state.tracker.value_buffer.clone();

    // Click-to-place: cells record their (row, col) here instead of borrowing
    // `view_state` into the table closures; applied after the table is built.
    let click_target: Cell<Option<(usize, usize)>> = Cell::new(None);

    // Row-hover highlight: any cell whose response contains the pointer records
    // its row here (cells span the table width, so this is X+Y exact); the result
    // is stored for *next* frame's tint (one-frame lag is invisible).
    let prev_hovered_row = view_state.tracker.hovered_row;
    let hovered_row: Cell<Option<usize>> = Cell::new(None);
    // A cell context-menu action, applied after the table (like `click_target`).
    let ctx_action: Cell<Option<CtxAction>> = Cell::new(None);
    // The cursor is "active" exactly when the tracker would respond to arrow keys:
    // no other widget holds keyboard focus (the same gate `handle_tracker_keys`
    // uses). When false (e.g. an inline rename has focus), the cursor draws dimmed.
    let cursor_active = ui.memory(|m| m.focused()).is_none();

    // Manual wheel scrolling during playback breaks lock-follow, mirroring the
    // piano roll's scroll-away detection. The table virtualizes rows and hides its
    // scroll offset, so (unlike the piano roll's offset delta) we key off raw scroll
    // input. Read before `TableBuilder::new(ui)` borrows `ui`. Re-enabled when
    // playback restarts (the transport Play button sets `auto_follow_playhead`).
    if is_playing
        && view_state.auto_follow_playhead
        && ui.input(|i| i.smooth_scroll_delta.y.abs() > 0.5)
    {
        view_state.auto_follow_playhead = false;
    }

    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        // Cells default to `Sense::hover()`, so cell/header responses never report
        // `clicked()` / `secondary_clicked()` and `.context_menu()` + `.on_hover_text()`
        // silently no-op. Sense clicks table-wide (propagates to header + body strips).
        // `Sense::CLICK` is the non-focusable variant: cells must not steal keyboard
        // focus, or the cursor's arrow-key gate (`focused().is_none()`) would break.
        .sense(egui::Sense::CLICK)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(44.0)); // row/time gutter
    for _ in 0..n_lanes {
        builder = builder.column(Column::initial(72.0).at_least(40.0)); // voice
        builder = builder.column(Column::initial(40.0).at_least(30.0)); // ornament
        if show_expr {
            for _ in 0..EXPR_FIELDS {
                builder = builder.column(Column::initial(34.0).at_least(26.0)); // expr field
            }
        }
    }
    for _ in 0..n_auto {
        builder = builder.column(Column::initial(80.0).at_least(52.0));
    }
    for _ in &np_stages {
        builder = builder.column(Column::initial(96.0).at_least(56.0)); // NP stage output
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
                header_label(ui, "Row".to_string(), false)
                    .on_hover_text("Step (row) number — each row is one ticks-per-row step");
            });
            for lane in 0..n_lanes {
                header.col(|ui| {
                    header_label(
                        ui,
                        format!("V{}", lane + 1),
                        cursor_kind == TrackerColumn::Voice(lane),
                    )
                    .on_hover_text(format!(
                        "Voice lane {} — notes feeding the pattern's instrument",
                        lane + 1
                    ));
                });
                header.col(|ui| {
                    header_label(ui, "Orn".to_string(), cursor_kind == TrackerColumn::Ornament(lane))
                        .on_hover_text(
                            "Per-note ornament — Enter edits (flam/drag/ruff/roll/grace), Delete clears",
                        );
                });
                if show_expr {
                    for field in ExprField::ALL {
                        header.col(|ui| {
                            header_label(
                                ui,
                                field.header().to_string(),
                                cursor_kind == TrackerColumn::Expr(lane, field),
                            )
                            .on_hover_text(field.description());
                        });
                    }
                }
            }
            for (ai, lane) in data.automation_lanes.iter().enumerate() {
                let name = lane.target.display_name();
                // Attach the tooltip + menu to the whole column cell, not the inner
                // label (a tiny text-sized rect): the cell senses clicks, the label
                // does not.
                let (_, resp) = header.col(|ui| {
                    header_label(ui, name.clone(), cursor_kind == TrackerColumn::Automation(ai));
                });
                let target = &lane.target;
                resp.on_hover_text(format!(
                    "Automation: {name} — per-row value 0..1; type to set a point, Delete to clear. Right-click to delete the lane."
                ))
                .context_menu(|ui| {
                    if ui.button("Delete lane").clicked() {
                        ctx_action.set(Some(CtxAction::DeleteAutoLane(target.clone())));
                        ui.close();
                    }
                });
            }
            for (si, stage) in np_stages.iter().enumerate() {
                header.col(|ui| {
                    header_label(ui, stage.label.clone(), false).on_hover_text(format!(
                        "Note processor {} output (cumulative after this stage)",
                        si + 1
                    ));
                });
            }
        })
        .body(|body| {
            body.rows(row_h, n_rows, |mut row| {
                let r = row.index();
                let row_tick = (r as u32) * tpr;
                let on_beat = row_tick.is_multiple_of(ticks_per_beat);
                // A divider above each bar boundary (not above row 0 = table top).
                let is_bar_start = r > 0 && row_tick.is_multiple_of(ticks_per_bar);
                let is_playhead = playhead_row == Some(r);
                let is_cursor_row = r == cursor.row;
                let is_hovered_row = prev_hovered_row == Some(r);

                // Gutter: row number, beat emphasis, playhead marker. Clicking it
                // moves the cursor to this row, keeping the current column.
                let (_, gutter_resp) = row.col(|ui| {
                    colors.paint_cell_chrome(ui, CellChrome { cursor_row: is_cursor_row, cursor_cell: false, bar_start: is_bar_start, hovered: is_hovered_row, active: cursor_active });
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
                if gutter_resp.clicked() {
                    click_target.set(Some((r, cursor.col)));
                }
                if gutter_resp.contains_pointer() {
                    hovered_row.set(Some(r));
                }

                // Voice columns (each optionally followed by its expression cells).
                for lane in 0..n_lanes {
                    let voice_col = voice_group_base(lane, cols_per_voice);
                    // The (first) note starting on this row in this lane, plus how
                    // many extra share it — shared by the voice cell and its
                    // expression sub-cells (no per-cell allocation).
                    let mut hits = notes_by_start_row
                        .get(&r)
                        .into_iter()
                        .flatten()
                        .filter(|&&i| lane_of_note[i] == lane);
                    let first_note = hits.next().copied();
                    let extra = hits.count();

                    let is_cursor_cell = is_cursor_row && cursor.col == voice_col;
                    let (_, resp) = row.col(|ui| {
                        colors.paint_cell_chrome(ui, CellChrome { cursor_row: is_cursor_row, cursor_cell: is_cursor_cell, bar_start: is_bar_start, hovered: is_hovered_row, active: cursor_active });
                        // Usually 0 or 1 notes; more than one means notes share a
                        // (row, lane) — draw the first and mark the rest with a "+N"
                        // overflow glyph rather than silently hiding them.
                        match first_note {
                            Some(idx) => {
                                draw_note_cell(ui, &data.notes[idx], tpr, &colors);
                                if extra > 0 {
                                    ui.label(
                                        RichText::new(format!("+{extra}"))
                                            .color(colors.off_grid)
                                            .small(),
                                    )
                                    .on_hover_text(format!(
                                        "{extra} more note(s) share this voice lane on this row"
                                    ));
                                }
                            }
                            None => {
                                ui.label(
                                    RichText::new(EMPTY_VOICE).color(colors.empty).monospace(),
                                );
                            }
                        }
                    });
                    if resp.clicked() {
                        click_target.set(Some((r, voice_col)));
                    }
                    if resp.contains_pointer() {
                        hovered_row.set(Some(r));
                    }
                    if let Some(idx) = first_note {
                        let n = &data.notes[idx];
                        let note_id = n.note_id;
                        let legato = n.legato;
                        let has_glide = n.glide.is_some();
                        resp.context_menu(|ui| {
                            if ui.button("Delete note").clicked() {
                                ctx_action.set(Some(CtxAction::DeleteNote(note_id)));
                                ui.close();
                            }
                            let mut leg = legato;
                            if ui.checkbox(&mut leg, "Legato").changed() {
                                ctx_action.set(Some(CtxAction::SetLegato(note_id, leg)));
                                ui.close();
                            }
                            let mut gl = has_glide;
                            if ui.checkbox(&mut gl, "Glide").changed() {
                                ctx_action
                                    .set(Some(CtxAction::SetGlide(note_id, gl.then(default_glide))));
                                ui.close();
                            }
                        });
                        resp.on_hover_text(voice_tooltip(n, tpr));
                    }

                    // Ornament cell (always present, sub-column 1): a compact tag
                    // for the note's ornament; Enter edits via the shared popup.
                    let orn_col = voice_col + 1;
                    let is_orn_cursor = is_cursor_row && cursor.col == orn_col;
                    let (_, orn_resp) = row.col(|ui| {
                        colors.paint_cell_chrome(ui, CellChrome { cursor_row: is_cursor_row, cursor_cell: is_orn_cursor, bar_start: is_bar_start, hovered: is_hovered_row, active: cursor_active });
                        match first_note.and_then(|idx| data.notes[idx].ornament) {
                            Some(o) => {
                                ui.label(
                                    RichText::new(ornament_tag(o))
                                        .color(colors.expression)
                                        .monospace(),
                                );
                            }
                            None => {
                                ui.label(
                                    RichText::new(EXPR_UNSET).color(colors.empty).monospace(),
                                );
                            }
                        }
                    });
                    if orn_resp.clicked() {
                        click_target.set(Some((r, orn_col)));
                    }
                    if orn_resp.contains_pointer() {
                        hovered_row.set(Some(r));
                    }
                    if let Some(idx) = first_note {
                        let note_id = data.notes[idx].note_id;
                        let orn = data.notes[idx].ornament;
                        orn_resp.context_menu(|ui| {
                            if ui.button("Edit ornament…").clicked() {
                                ctx_action.set(Some(CtxAction::EditOrnament(note_id)));
                                ui.close();
                            }
                            if orn.is_some() && ui.button("Clear ornament").clicked() {
                                ctx_action.set(Some(CtxAction::ClearOrnament(note_id)));
                                ui.close();
                            }
                        });
                        if let Some(o) = orn {
                            orn_resp.on_hover_text(ornament_detail(o));
                        }
                    }

                    // Expression sub-columns: the note's scalar fields, read-only
                    // (T3.1a). Blank when no note carries this (row, lane).
                    if show_expr {
                        for (fi, field) in ExprField::ALL.into_iter().enumerate() {
                            let flat = voice_col + 2 + fi;
                            let is_cc = is_cursor_row && cursor.col == flat;
                            let (_, resp) = row.col(|ui| {
                                colors.paint_cell_chrome(ui, CellChrome { cursor_row: is_cursor_row, cursor_cell: is_cc, bar_start: is_bar_start, hovered: is_hovered_row, active: cursor_active });
                                let Some(idx) = first_note else { return };
                                // While typing a value into this (numeric) cell, show
                                // the entry buffer with a caret; otherwise the value.
                                if is_cc
                                    && field != ExprField::Ghost
                                    && let Some(buf) = entry_buffer.as_deref()
                                {
                                    ui.label(
                                        RichText::new(format!("{buf}_"))
                                            .color(colors.cursor_border)
                                            .monospace(),
                                    );
                                    return;
                                }
                                let expr = data.notes[idx].expression.as_ref();
                                let text = expr_field_text(expr, field);
                                let set = text != EXPR_UNSET;
                                ui.label(
                                    RichText::new(text)
                                        .color(if set { colors.automation } else { colors.empty })
                                        .monospace(),
                                );
                            });
                            if resp.clicked() {
                                click_target.set(Some((r, flat)));
                            }
                            if resp.contains_pointer() {
                                hovered_row.set(Some(r));
                            }
                            if let Some(idx) = first_note {
                                let note_id = data.notes[idx].note_id;
                                resp.context_menu(|ui| {
                                    if ui.button(format!("Clear {}", field.header())).clicked() {
                                        ctx_action
                                            .set(Some(CtxAction::ClearExprField(note_id, field)));
                                        ui.close();
                                    }
                                });
                            }
                        }
                    }
                }

                // Automation columns: numeric value + per-cell curve segment behind it.
                for (ai, lane) in data.automation_lanes.iter().enumerate() {
                    let flat_col = voice_group_base(n_lanes, cols_per_voice) + ai;
                    let is_cursor_cell = is_cursor_row && cursor.col == flat_col;
                    let (_, resp) = row.col(|ui| {
                        colors.paint_cell_chrome(ui, CellChrome { cursor_row: is_cursor_row, cursor_cell: is_cursor_cell, bar_start: is_bar_start, hovered: is_hovered_row, active: cursor_active });
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
                        // While typing a value into this cell, show the entry buffer
                        // with a caret; otherwise show the sampled value, emphasized
                        // when an actual point sits on this row (vs. interpolation).
                        if is_cursor_cell && let Some(buf) = entry_buffer.as_deref() {
                            ui.label(
                                RichText::new(format!("{buf}_"))
                                    .color(colors.cursor_border)
                                    .monospace(),
                            );
                        } else {
                            let has_point = lane.points.iter().any(|p| p.tick.0 == row_tick);
                            let mut text = RichText::new(automation_value_text(top)).monospace();
                            text = if has_point {
                                text.color(colors.automation_curve).strong()
                            } else {
                                text.color(colors.automation)
                            };
                            ui.label(text);
                        }
                    });
                    if resp.clicked() {
                        click_target.set(Some((r, flat_col)));
                    }
                    if resp.contains_pointer() {
                        hovered_row.set(Some(r));
                    }
                    if let Some((value, curve)) = lane
                        .points
                        .iter()
                        .find(|p| p.tick.0 == row_tick)
                        .map(|p| (p.value, p.curve))
                    {
                        let target = &lane.target;
                        resp.context_menu(|ui| {
                            if ui.button("Clear point").clicked() {
                                ctx_action.set(Some(CtxAction::ClearAutoPoint {
                                    target: target.clone(),
                                    tick: PatternTick(row_tick),
                                    value,
                                    curve,
                                }));
                                ui.close();
                            }
                        });
                    }
                }

                // Note-processor output columns (read-only, non-selectable, one per
                // processor stage). Clicking parks the cursor on this row.
                for stage in &np_stages {
                    let (_, resp) = row.col(|ui| {
                        colors.paint_cell_chrome(ui, CellChrome { cursor_row: is_cursor_row, cursor_cell: false, bar_start: is_bar_start, hovered: is_hovered_row, active: cursor_active });
                        if let Some(pitches) = stage.rows.get(r) {
                            ui.label(
                                RichText::new(np_cell_text(pitches))
                                    .color(colors.expression)
                                    .monospace(),
                            );
                        }
                    });
                    if resp.clicked() {
                        click_target.set(Some((r, cursor.col)));
                    }
                    if resp.contains_pointer() {
                        hovered_row.set(Some(r));
                    }
                }
            });
        });

    // Store the row the pointer landed on this frame for next frame's hover tint.
    view_state.tracker.hovered_row = hovered_row.get();

    // Apply a click captured during the table pass (deferred so the closures don't
    // need a mutable borrow of `view_state`).
    if let Some((row, col)) = click_target.get() {
        if (row, col) != (view_state.tracker.cursor.row, view_state.tracker.cursor.col) {
            view_state.tracker.value_buffer = None;
        }
        view_state.tracker.cursor.row = row;
        view_state.tracker.cursor.col = col;
    }

    // Apply a context-menu action captured during the table pass (mutates the
    // song + records undo; EditOrnament must run before the popup below).
    apply_ctx_action(
        ctx_action.take(),
        data.pattern_id,
        song,
        view_state,
        undo_manager,
    );

    // Shared per-note ornament popup (open while `editing_ornament` is set —
    // e.g. Enter on an ornament cell above).
    draw_ornament_popup(ui, song, view_state, undo_manager);
}

/// Apply a tracker cell context-menu action: mutate the song and push the
/// matching undo entry (or open the ornament editor for `EditOrnament`).
fn apply_ctx_action(
    action: Option<CtxAction>,
    pattern_id: PatternId,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    undo_manager: &mut UndoManager,
) {
    match action {
        None => {}
        Some(CtxAction::DeleteNote(id)) => {
            let mut song_w = song.write();
            if let Some(p) = song_w.pattern_mut(pattern_id)
                && let Some(removed) = p.note(id).map(Into::into)
            {
                p.remove_note(id);
                undo_manager.push(UndoAction::RemoveNote {
                    pattern_id,
                    note: removed,
                });
            }
        }
        Some(CtxAction::SetLegato(id, new)) => {
            let mut song_w = song.write();
            if let Some(p) = song_w.pattern_mut(pattern_id)
                && let Some(old) = p.note(id).map(|n| n.legato)
            {
                p.set_note_legato(id, new);
                undo_manager.push(UndoAction::SetLegatoBatch {
                    pattern_id,
                    changes: vec![(id, old, new)],
                });
            }
        }
        Some(CtxAction::SetGlide(id, new)) => {
            let mut song_w = song.write();
            if let Some(p) = song_w.pattern_mut(pattern_id)
                && let Some(old) = p.note(id).map(|n| n.glide)
            {
                p.set_note_glide(id, new);
                undo_manager.push(UndoAction::SetGlideBatch {
                    pattern_id,
                    changes: vec![(id, old, new)],
                });
            }
        }
        Some(CtxAction::EditOrnament(id)) => {
            if let Some(s) = song.try_read() {
                let current = s
                    .pattern(pattern_id)
                    .and_then(|p| p.note(id))
                    .and_then(|n| n.ornament);
                drop(s);
                view_state.editing_ornament = Some(OrnamentEdit {
                    pattern_id,
                    note_id: id,
                    before: current,
                    current,
                });
            }
        }
        Some(CtxAction::ClearOrnament(id)) => {
            let mut old = None;
            {
                let mut song_w = song.write();
                if let Some(n) = song_w.pattern_mut(pattern_id).and_then(|p| p.note_mut(id)) {
                    old = n.ornament;
                    n.ornament = None;
                }
            }
            if old.is_some() {
                undo_manager.push(UndoAction::SetNoteOrnament {
                    pattern_id,
                    note_id: id,
                    old,
                    new: None,
                });
            }
        }
        Some(CtxAction::ClearExprField(id, field)) => {
            let mut change = None;
            {
                let mut song_w = song.write();
                if let Some(p) = song_w.pattern_mut(pattern_id)
                    && let Some(old) = p.note(id).map(|n| n.expression)
                {
                    let mut e = old.unwrap_or_default();
                    set_expr_field(&mut e, field, None);
                    let new = e.normalized();
                    if new != old {
                        p.set_note_expression(id, new);
                        change = Some((id, old, new));
                    }
                }
            }
            if let Some(c) = change {
                undo_manager.push(UndoAction::SetExpressionBatch {
                    pattern_id,
                    changes: vec![c],
                });
            }
        }
        Some(CtxAction::ClearAutoPoint {
            target,
            tick,
            value,
            curve,
        }) => {
            let mut removed = false;
            {
                let mut song_w = song.write();
                if let Some(lane) = song_w
                    .pattern_mut(pattern_id)
                    .and_then(|p| p.automation.iter_mut().find(|l| l.target == target))
                {
                    removed = lane.remove_point(tick).is_some();
                }
            }
            if removed {
                undo_manager.push(UndoAction::RemoveAutomationPoint {
                    pattern_id,
                    target,
                    tick,
                    value,
                    curve,
                });
            }
        }
        Some(CtxAction::DeleteAutoLane(target)) => {
            let mut removed = None;
            {
                let mut song_w = song.write();
                if let Some(p) = song_w.pattern_mut(pattern_id) {
                    removed = p.remove_automation_lane(&target);
                }
            }
            if let Some(lane) = removed {
                undo_manager.push(UndoAction::RemoveAutomationLane { pattern_id, lane });
            }
        }
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

/// Lane assignment for display. Stored `Note.lane` is the tracker's source of
/// truth for columns, but legacy patterns (built in the piano roll) have every
/// note on lane 0, where stacking them all in V1 would hide notes. So: if **any**
/// note carries a non-zero lane the pattern is "lane-organized" and we render by
/// the stored lane; if **all** notes are on lane 0 we fall back to greedy
/// interval-coloring so legacy content spreads readably. The first lane-assigning
/// edit (T2) migrates the greedy layout into stored lanes, after which this stays
/// on the stored-lane branch with no visible jump.
///
/// Returns the per-note lane index (parallel to `notes`) and the lane count (at
/// least 1, so one empty voice column always shows).
fn display_lanes(notes: &[PianoRollNote], tpr: u32) -> (Vec<usize>, usize) {
    if is_lane_organized(notes) {
        let lane_of: Vec<usize> = notes.iter().map(|n| n.lane.as_usize()).collect();
        // `map_or(1, m+1)` already floors at 1 (empty → 1, any max → ≥1).
        let n_lanes = lane_of.iter().copied().max().map_or(1, |m| m + 1);
        (lane_of, n_lanes)
    } else {
        assign_voice_lanes(notes, tpr)
    }
}

/// Whether a pattern is "lane-organized": any note carries a non-zero stored lane.
/// Both the render-layout decision (`display_lanes`) and the edit-migration decision
/// (`handle_tracker_edit_keys`) key off this single predicate so they can't disagree.
fn is_lane_organized(notes: &[PianoRollNote]) -> bool {
    notes.iter().any(|n| n.lane.as_u8() != 0)
}

/// Greedy interval-coloring lane assignment: each note takes the lowest voice lane
/// free at its start tick. Used as the read-only fallback for not-yet-lane-organized
/// patterns (see `display_lanes`) and as the seed the first tracker edit migrates
/// into stored lanes. Returns the per-note lane index (parallel to `notes`) and the
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
