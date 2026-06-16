//! Sequencer GUI module.
//!
//! Provides the sequencer view with transport controls, an arrangement timeline,
//! a piano roll with mouse interaction (draw, select, move, resize, delete notes),
//! and a GUI input source for sending `InputCommand`s to the sequencer engine.

use std::collections::HashSet;
use std::sync::Arc;

use parking_lot::RwLock;

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use synth_core::{BipolarValue, Bpm, Hertz, MidiNote, Milliseconds, NormalizedValue, Semitones};
use synth_engine::{EngineCommand, EngineHandle, RecordingState};
use synth_sequencer::{
    AutoInstrumentParam, AutomationPoint, AutomationTarget, CurveType, Duration as SeqDuration,
    Glide, GlideFrom, GlideInterp, NoteExpression, NoteId, NoteLane, NoteName, PatternId,
    PatternTick, Pitch, SeqInstrumentId, Song, Tick, TimeSignature, TrackId, Velocity, Vibrato,
    VibratoShape,
};

use crate::gui::input::KEY_MAP;
use crate::gui::theme::theme;
use crate::gui::widgets::toggle_button;

mod tracker;
pub(crate) use tracker::draw_tracker;

// ============================================================================
// EDIT TYPES
// ============================================================================

/// Active editing tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditTool {
    Select,
    Draw,
}

/// Which part of a note was hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HitZone {
    /// Note body (for moving).
    Body,
    /// Right edge (for resizing).
    RightEdge,
}

/// Active drag operation (preview only — Song is mutated on release).
#[derive(Debug, Clone)]
enum DragState {
    /// Drag-move a note.
    MoveNote {
        note_id: NoteId,
        original_tick: PatternTick,
        original_pitch: Pitch,
        current_tick: PatternTick,
        current_pitch: Pitch,
        /// Offset from note start to where the mouse grabbed (in ticks).
        grab_offset_ticks: SeqDuration,
    },
    /// Drag-resize a note (right edge).
    ResizeNote {
        note_id: NoteId,
        original_end_tick: PatternTick,
        current_end_tick: PatternTick,
    },
    /// Draw a new note by dragging (Draw tool on empty space).
    DrawNote {
        start_tick: PatternTick,
        pitch: Pitch,
        current_end_tick: PatternTick,
    },
    /// Selection rectangle (lasso).
    SelectRect { start_pos: Pos2, current_pos: Pos2 },
    /// Drag-move an automation point.
    DragAutomationPoint {
        target: AutomationTarget,
        original_tick: PatternTick,
        original_value: NormalizedValue,
        current_tick: PatternTick,
        current_value: NormalizedValue,
    },
    /// Drag-edit a velocity bar.
    DragVelocity {
        /// Note being velocity-edited (None = painting across multiple).
        last_note_id: Option<NoteId>,
        /// Original velocities captured the first time each note was
        /// touched, used to emit one composite undo on release.
        initial_velocities: std::collections::HashMap<NoteId, Velocity>,
    },
    /// Drag-move a pattern placement in the arrangement.
    DragPlacement {
        pattern_id: PatternId,
        track_id: TrackId,
        start_tick: Tick,
        /// Current drag tick position (snapped to grid).
        current_tick: Tick,
        /// Current target track ID.
        current_track_id: TrackId,
        /// Offset from placement start to where mouse grabbed (in ticks).
        grab_offset_ticks: Tick,
    },
    /// Drag-resize a placement's right edge (writes length_override).
    ResizePlacement {
        pattern_id: PatternId,
        track_id: TrackId,
        start_tick: Tick,
        /// Pattern length when the drag started — used to invert
        /// length_override on undo.
        original_length: SeqDuration,
        /// Current length while dragging (snapped to grid).
        current_length: SeqDuration,
    },
}

// ============================================================================
// CLIPBOARD
// ============================================================================

/// A note stored in the clipboard, with position relative to the selection origin.
#[derive(Debug, Clone)]
struct ClipboardNote {
    tick_offset: SeqDuration,
    pitch: Pitch,
    velocity: Velocity,
    duration: Option<SeqDuration>,
}

/// Piano roll clipboard for copy/paste operations.
#[derive(Debug, Clone, Default)]
struct Clipboard {
    notes: Vec<ClipboardNote>,
    /// Total width of the copied selection (max end - min start), in ticks.
    selection_width: SeqDuration,
}

// ============================================================================
// VIEW STATE
// ============================================================================

/// Pre-edit per-note glide snapshot captured at drag start (pattern + the
/// prior `(note_id, glide)` of each selected note), so a glide DragValue drag
/// collapses into a single `SetGlideBatch` undo entry on release.
type GlideDragStart = (PatternId, Vec<(NoteId, Option<Glide>)>);

/// Pre-edit per-note expression snapshot captured at drag start, so an
/// expression DragValue drag collapses into a single `SetExpressionBatch` undo
/// entry on release (mirrors [`GlideDragStart`]).
type ExprDragStart = (PatternId, Vec<(NoteId, Option<NoteExpression>)>);

/// Piano roll view state (persists across frames).
pub struct SequencerViewState {
    /// Clipboard for copy/paste operations.
    clipboard: Clipboard,
    /// Default velocity for newly drawn/recorded notes (0.0-1.0).
    pub default_velocity: Velocity,
    /// Quantization strength for the quantize button (0.0-1.0).
    quantize_strength: NormalizedValue,
    /// Swing amount (0.0-1.0, 0.0 = no swing).
    swing_amount: NormalizedValue,
    /// Velocity scale factor for scale-velocities operation (1–200%).
    velocity_scale_pct: u32,
    /// Draw-mode grid resolution in ticks (0 = use pattern default).
    draw_grid_resolution: u32,
    /// Draw-mode note length preset in ticks (0 = drag-to-length).
    draw_note_length: u32,
    /// Step entry mode enabled.
    step_entry_mode: bool,
    /// Step entry cursor position.
    step_cursor_tick: PatternTick,
    /// Currently opened pattern (None = piano roll closed). Shared between
    /// the Seq view's bottom panel and the Pattern tab — see
    /// `docs/pattern-tab-plan.md` §5.
    pub(crate) opened_pattern: Option<PatternId>,
    /// Currently selected notes.
    selected_notes: HashSet<NoteId>,
    /// Active edit tool.
    edit_tool: EditTool,
    /// Active drag operation.
    drag: Option<DragState>,
    /// Currently selected automation lane (None = automation zone hidden).
    selected_automation: Option<AutomationTarget>,
    /// Track currently being renamed (inline text edit).
    editing_track_name: Option<(TrackId, String)>,
    /// Pattern currently being renamed (inline text edit). Shared between
    /// the Seq view's piano-roll toolbar and the Pattern tab browser.
    pub(crate) editing_pattern_name: Option<(PatternId, String)>,
    /// Pattern whose description is being edited (inline text edit) in the
    /// piano-roll toolbar.
    pub(crate) editing_pattern_description: Option<(PatternId, String)>,
    /// Track whose description is being edited (inline text edit) in the
    /// track-properties popup.
    editing_track_description: Option<(TrackId, String)>,
    /// Repeat song (loop entire song).
    repeat_enabled: bool,
    /// Pattern solo: when true (default), mini-transport play isolates the
    /// open pattern from all other tracks/patterns. When false, the pattern
    /// loops together with everything else that overlaps its time range.
    /// Does not modify any track's persistent mute/solo state.
    pattern_solo: bool,
    /// Stored right-click position (captured at click time, before menu opens).
    context_menu_pos: Option<Pos2>,
    /// Track to highlight (set on right-click, cleared on primary click).
    highlighted_track: Option<TrackId>,
    /// Selected track for pattern follow mode.
    selected_track: Option<TrackId>,
    /// Arrangement timeline zoom level (1.0 = default).
    zoom_level: f32,
    /// Auto-scroll to follow playhead during playback.
    auto_follow_playhead: bool,
    /// Last scroll offset set by auto-follow (to detect manual scrolling).
    last_auto_scroll_offset: Option<f32>,
    /// Same, for the piano roll's own scroll area (both are drawn in the
    /// same frame, so they cannot share one expected-offset slot).
    pr_last_auto_scroll_offset: Option<f32>,
    /// Frames left to keep repainting after a transport jump / seek / stop while
    /// stopped, so the timeline catches the engine's async playhead update and
    /// can scroll the marker back into view (off-screen follow).
    follow_settle_frames: u8,
    /// Recording quantize grid in ticks (0=off, 960=1/4, 480=1/8, 240=1/16, 120=1/32).
    pub record_quantize: u32,
    /// Overdub mode: true = layer on existing notes, false = replace.
    pub overdub: bool,
    /// Live recording preview: completed notes.
    pub recording_preview_completed: Vec<synth_engine::recording::RecordedNote>,
    /// Live recording preview: held note starts (pitch, start_tick).
    pub recording_preview_held: Vec<(Pitch, PatternTick)>,
    /// Pattern length for preview rendering.
    pub recording_preview_pattern_length: SeqDuration,
    /// Selected instrument for new notes in the piano roll.
    pub selected_instrument: SeqInstrumentId,
    /// Pattern captured at recording arm time, and whether it was an orphan
    /// (no arrangement placement). Used to keep the armed-Play branch and the
    /// live-preview pitch fold tied to the pattern actually being recorded.
    pub(crate) recording_pattern: Option<PatternId>,
    /// Piano roll horizontal zoom (1.0 = default).
    pr_zoom_x: f32,
    /// Piano roll vertical zoom (1.0 = default).
    pr_zoom_y: f32,
    /// Original pattern length captured when a length-edit drag starts;
    /// used to emit a single undo action when the drag commits.
    pattern_length_drag_start: Option<(PatternId, SeqDuration)>,
    /// Original velocities captured when the selection-inspector velocity
    /// DragValue gains focus / starts dragging. Used to emit one composite
    /// undo entry covering the whole edit on release.
    inspector_vel_drag_start: Option<(PatternId, Vec<(NoteId, Velocity)>)>,
    /// Pre-edit per-note glide snapshot captured when a glide DragValue starts,
    /// so the whole drag collapses into one `SetGlideBatch` undo entry on release.
    inspector_glide_drag_start: Option<GlideDragStart>,
    /// Pre-edit per-note expression snapshot for the expression DragValues.
    inspector_expr_drag_start: Option<ExprDragStart>,
    /// Snap value (in ticks) for arrangement-view operations: placement
    /// create, placement drag-move, and placement resize. 0 = no snap.
    arrangement_snap_ticks: u32,
    /// Tap-tempo click timestamps (egui input time, seconds). Up to 4 are
    /// kept; entries older than ~2 s are dropped on the next tap.
    tap_tempo_times: Vec<f64>,
    /// Loop region — when both ends are set the engine loops between them.
    /// Mirrored to the engine via `EngineCommand::SetLoop`.
    loop_start_tick: Option<Tick>,
    loop_end_tick: Option<Tick>,
    /// Tracker-view cursor (row + flat column index). Persisted across frames and
    /// view toggles; navigated with arrow keys / clicks in `draw_tracker`. T1
    /// highlights it, T2 will edit the cell under it.
    tracker_cursor: tracker::TrackerCursor,
    /// In-progress numeric entry for the tracker automation cell under the cursor.
    /// `Some` while the user is typing a value (digits/`.`); committed on Enter,
    /// discarded on Esc or when the cursor leaves the cell. `None` when idle.
    tracker_value_buffer: Option<String>,
    /// User-requested minimum number of tracker voice columns. The actual column
    /// count is `max(derived_from_notes, this, 1)`; "Add voice column" raises it so
    /// an empty lane appears for entry, "Remove empty columns" lowers it. `0` = just
    /// derive from the notes.
    tracker_voice_columns: usize,
    /// Whether the tracker interleaves the per-note expression sub-columns
    /// (accent/gate/ghost/probability) after each voice column ("Expr" toggle).
    tracker_show_expression: bool,
}

impl SequencerViewState {
    pub fn new() -> Self {
        Self {
            clipboard: Clipboard::default(),
            default_velocity: Velocity::new(0.8),
            quantize_strength: NormalizedValue::new(1.0),
            swing_amount: NormalizedValue::new(0.0),
            velocity_scale_pct: 100,
            draw_grid_resolution: 0,
            draw_note_length: 0,
            step_entry_mode: false,
            step_cursor_tick: PatternTick::ZERO,
            opened_pattern: None,
            selected_notes: HashSet::new(),
            edit_tool: EditTool::Draw,
            drag: None,
            selected_automation: None,
            editing_track_name: None,
            editing_pattern_name: None,
            editing_pattern_description: None,
            editing_track_description: None,
            repeat_enabled: false,
            pattern_solo: true,
            context_menu_pos: None,
            highlighted_track: None,
            selected_track: None,
            zoom_level: 1.0,
            auto_follow_playhead: true,
            last_auto_scroll_offset: None,
            pr_last_auto_scroll_offset: None,
            follow_settle_frames: 0,
            record_quantize: 0,
            overdub: true,
            recording_preview_completed: Vec::new(),
            recording_preview_held: Vec::new(),
            recording_preview_pattern_length: SeqDuration(0),
            selected_instrument: SeqInstrumentId::new(0),
            recording_pattern: None,
            pr_zoom_x: 1.0,
            pr_zoom_y: 1.0,
            pattern_length_drag_start: None,
            inspector_vel_drag_start: None,
            inspector_glide_drag_start: None,
            inspector_expr_drag_start: None,
            // Default snap: 1 beat (1/4 note at TICKS_PER_QUARTER = 960).
            arrangement_snap_ticks: synth_sequencer::TICKS_PER_QUARTER,
            tap_tempo_times: Vec::new(),
            loop_start_tick: None,
            loop_end_tick: None,
            tracker_cursor: tracker::TrackerCursor::default(),
            tracker_value_buffer: None,
            tracker_voice_columns: 0,
            tracker_show_expression: true,
        }
    }
}

impl SequencerViewState {
    /// Get the effective grid resolution in ticks, falling back to pattern default.
    fn effective_grid(&self, fallback_ticks_per_row: u16) -> SeqDuration {
        SeqDuration(if self.draw_grid_resolution > 0 {
            self.draw_grid_resolution
        } else {
            fallback_ticks_per_row as u32
        })
    }

    /// Close the piano roll — clears the opened pattern, selection, and any
    /// in-flight drag. Called when the user clicks the piano-roll close
    /// button or when the open pattern is deleted from elsewhere.
    pub(crate) fn close_piano_roll(&mut self) {
        self.opened_pattern = None;
        self.selected_notes.clear();
        self.drag = None;
        self.recording_pattern = None;
    }

    /// Re-enable playhead follow and open the settle window so an off-screen
    /// marker is scrolled into view even while the transport is stopped.
    /// Every transport action that moves the playhead should call this.
    fn reveal_playhead(&mut self) {
        self.auto_follow_playhead = true;
        self.follow_settle_frames = FOLLOW_SETTLE_FRAMES;
    }
}

impl Default for SequencerViewState {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply a rename to a pattern under a short write-lock and push a
/// `RenamePattern` undo entry. No-op if the name is unchanged or the pattern
/// no longer exists.
pub(crate) fn commit_pattern_rename(
    song: &Arc<RwLock<Song>>,
    undo_manager: &mut crate::undo::UndoManager,
    pattern_id: PatternId,
    new_name: String,
) {
    let mut old_name: Option<String> = None;
    {
        let mut song_w = song.write();
        if let Some(pat) = song_w.pattern_mut(pattern_id)
            && pat.name != new_name
        {
            old_name = Some(pat.name.clone());
            pat.name = new_name.clone();
        }
    }
    if let Some(old) = old_name {
        undo_manager.push(crate::undo::UndoAction::RenamePattern {
            pattern_id,
            old_name: old,
            new_name,
        });
    }
}

/// Apply a description edit to a pattern under a short write-lock. No-op if the
/// description is unchanged or the pattern no longer exists. Description is a
/// utility metadata field and is not tracked by the undo system.
pub(crate) fn commit_pattern_description(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
    new_description: String,
) {
    let mut song_w = song.write();
    if let Some(pat) = song_w.pattern_mut(pattern_id)
        && pat.description != new_description
    {
        pat.description = new_description;
    }
}

// ============================================================================
// CONSTANTS
// ============================================================================

// --- Sequencer palette ---
/// Accent colour for the transport-loop ruler markers and status badge.
const LOOP_COLOR: Color32 = Color32::from_rgb(120, 220, 180);
/// Dimmed red used for the disabled/idle record button.
const DIM_RED: Color32 = Color32::from_rgb(120, 40, 40);
/// Red used for the "ARM" status label.
const ARM_RED: Color32 = Color32::from_rgb(180, 60, 60);
/// Fill for the selected track header.
const TRACK_HEADER_SELECTED_FILL: Color32 = Color32::from_rgba_premultiplied(80, 140, 220, 50);
/// Fill for a highlighted (non-selected) track header / row.
const TRACK_HIGHLIGHT_FILL: Color32 = Color32::from_rgba_premultiplied(80, 120, 200, 40);
/// Background for even arrangement track rows.
const TRACK_ROW_BG_EVEN: Color32 = Color32::from_rgba_premultiplied(40, 42, 46, 80);
/// Fallback colour for placement note miniatures.
const MINIATURE_FALLBACK: Color32 = Color32::from_rgb(180, 200, 230);
/// Stroke for the placement resize ghost.
const RESIZE_GHOST_STROKE: Color32 = Color32::from_rgb(255, 200, 120);
/// Fill for the placement drag ghost.
const DRAG_GHOST_FILL: Color32 = Color32::from_rgba_unmultiplied_const(120, 180, 255, 60);
/// Stroke for the placement drag ghost.
const DRAG_GHOST_STROKE: Color32 = Color32::from_rgb(120, 180, 255);
/// Number of frames to keep repainting after a transport jump / seek / stop
/// while the transport is stopped, so the off-screen follow picks up the
/// engine's asynchronous playhead update and scrolls the marker into view.
const FOLLOW_SETTLE_FRAMES: u8 = 8;
/// Colour for tempo-change markers on the ruler.
const TEMPO_MARKER: Color32 = Color32::from_rgb(255, 180, 80);
/// Fill for the step-entry mode banner.
const STEP_ENTRY_BANNER_FILL: Color32 = Color32::from_rgba_unmultiplied_const(255, 100, 255, 28);
/// Text colour for the step-entry banner label.
const STEP_ENTRY_TEXT: Color32 = Color32::from_rgb(255, 160, 255);
/// Piano-roll black-key background.
const PIANO_KEY_BLACK: Color32 = Color32::from_rgb(30, 30, 35);
/// Piano-roll white-key background.
const PIANO_KEY_WHITE: Color32 = Color32::from_rgb(55, 58, 65);
/// Grid background for C rows.
const GRID_BG_C: Color32 = Color32::from_rgba_premultiplied(50, 55, 65, 80);
/// Grid background for black-key rows.
const GRID_BG_BLACK: Color32 = Color32::from_rgba_premultiplied(25, 27, 30, 80);
/// Grid background for white-key rows.
const GRID_BG_WHITE: Color32 = Color32::from_rgba_premultiplied(35, 38, 42, 80);
/// Default colour for notes without an instrument colour.
const DEFAULT_NOTE_BLUE: Color32 = Color32::from_rgb(100, 180, 255);
/// Soft glow halo behind selected notes.
const NOTE_SELECTED_GLOW: Color32 = Color32::from_rgba_unmultiplied_const(140, 220, 255, 60);
/// Orange for recording-preview notes.
const RECORDING_PREVIEW_ORANGE: Color32 = Color32::from_rgb(255, 160, 60);
/// Fill for held recording-preview notes.
const RECORDING_PREVIEW_HELD_FILL: Color32 =
    Color32::from_rgba_unmultiplied_const(255, 160, 60, 140);
/// Fill for the note move-drag ghost.
const MOVE_GHOST_FILL: Color32 = Color32::from_rgba_unmultiplied_const(140, 210, 255, 100);
/// Stroke for the note move-drag ghost.
const MOVE_GHOST_STROKE: Color32 = Color32::from_rgba_unmultiplied_const(140, 210, 255, 180);
/// Fill for the draw-note preview.
const DRAW_NOTE_FILL: Color32 = Color32::from_rgba_unmultiplied_const(100, 220, 140, 120);
/// Stroke for the draw-note preview.
const DRAW_NOTE_STROKE: Color32 = Color32::from_rgba_unmultiplied_const(100, 220, 140, 200);
/// Fill for the rubber-band selection rectangle.
const SELECTION_RECT_FILL: Color32 = Color32::from_rgba_unmultiplied_const(100, 180, 255, 30);
/// Stroke for the rubber-band selection rectangle.
const SELECTION_RECT_STROKE: Color32 = Color32::from_rgba_unmultiplied_const(100, 180, 255, 150);
/// Background for the velocity-bar zone.
const VELOCITY_ZONE_BG: Color32 = Color32::from_rgba_premultiplied(20, 22, 26, 200);
/// Velocity-bar colour for selected notes.
const VELOCITY_BAR_SELECTED: Color32 = Color32::from_rgb(180, 230, 255);
/// Colour for the step-entry cursor line.
const STEP_CURSOR: Color32 = Color32::from_rgb(255, 100, 255);
/// Subtle white outline for hovered notes.
const NOTE_HOVER_OUTLINE: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 60);
/// Orange accent for the automation lane.
const AUTOMATION_ORANGE: Color32 = Color32::from_rgb(255, 160, 50);
/// Background for the automation zone.
const AUTOMATION_ZONE_BG: Color32 = Color32::from_rgba_premultiplied(18, 20, 24, 220);
/// Fill for the dragged automation point.
const AUTOMATION_ORANGE_FILL: Color32 = Color32::from_rgba_unmultiplied_const(255, 160, 50, 120);

/// Width of the track header panel (left side).
const TRACK_HEADER_WIDTH: f32 = 150.0;
/// Height of each track row in the arrangement.
const TRACK_ROW_HEIGHT: f32 = 64.0;
/// Height of the timeline ruler at the top.
const RULER_HEIGHT: f32 = 24.0;
/// Pixels per beat at default zoom.
const PIXELS_PER_BEAT: f32 = 40.0;
/// Minimum number of bars to show (even if song is shorter).
const MIN_VISIBLE_BARS: u32 = 8;
/// Padding inside pattern placement boxes.
const PLACEMENT_PADDING: f32 = 2.0;
/// Minimum zoom level for arrangement timeline.
const MIN_ZOOM: f32 = 0.25;
/// Maximum zoom level for arrangement timeline.
const MAX_ZOOM: f32 = 4.0;
/// Maximum miniature notes drawn per horizontal pixel of placement width.
/// More than ~2 notes per pixel is indistinguishable, so the draw loop
/// decimates evenly (notes are sorted by start tick) past this density.
const MINIATURE_NOTES_PER_PIXEL: f32 = 2.0;
// Piano roll constants
/// Minimum default height for the piano roll bottom panel.
const MIN_PIANO_ROLL_HEIGHT: f32 = 400.0;
/// Width of the keyboard column.
const KEY_WIDTH: f32 = 40.0;
/// Pixels per semitone (row height) at zoom 1.0.
const DEFAULT_NOTE_ROW_HEIGHT: f32 = 12.0;
/// Height of the velocity zone at the bottom.
const VELOCITY_ZONE_HEIGHT: f32 = 40.0;
/// Horizontal zoom: pixels per beat in the piano roll at zoom 1.0.
const DEFAULT_PR_PIXELS_PER_BEAT: f32 = 60.0;
/// Resize grab zone width (pixels from right edge).
const RESIZE_GRAB_ZONE: f32 = 10.0;
/// Height of the automation zone (below velocity zone).
const AUTOMATION_ZONE_HEIGHT: f32 = 80.0;
/// Radius for automation point circles.
const AUTOMATION_POINT_RADIUS: f32 = 4.0;

/// Grid resolution options: (display label, ticks). Uses `Duration` constants.
const GRID_RESOLUTIONS: &[(&str, u32)] = &[
    ("Auto", 0),
    ("1/4", SeqDuration::QUARTER.0),
    ("1/8", SeqDuration::EIGHTH.0),
    ("1/16", SeqDuration::SIXTEENTH.0),
    ("1/32", SeqDuration::THIRTY_SECOND.0),
];
/// Hit radius for automation point click detection.
const AUTOMATION_HIT_RADIUS: f32 = 8.0;

// ============================================================================
// SHARED TIMELINE RULER
// ============================================================================

/// Draw the shared timeline-ruler strip: a dark background with a running
/// per-bar number (1, 2, 3, …). Used by both the arrangement ruler and the
/// piano-roll ruler so the two "top bars" look identical. Callers overlay
/// their own grid lines, loop/tempo markers, playhead and bottom border.
fn draw_ruler_labels(
    painter: &egui::Painter,
    t: &crate::gui::theme::Theme,
    ruler_rect: Rect,
    total_bars: u32,
    ticks_per_bar: u64,
    tick_to_x: impl Fn(u64) -> f32,
) {
    painter.rect_filled(ruler_rect, 0.0, t.colors.bg_dark);
    for bar_idx in 0..total_bars {
        let x = tick_to_x(u64::from(bar_idx) * ticks_per_bar);
        painter.text(
            Pos2::new(x + 4.0, ruler_rect.min.y + 4.0),
            egui::Align2::LEFT_TOP,
            format!("{}", bar_idx + 1),
            egui::FontId::proportional(12.0),
            t.colors.text_secondary,
        );
    }
}

// ============================================================================
// TRANSPORT BAR
// ============================================================================

/// Draw the transport control bar.
///
/// Shows play/stop/pause buttons, position display (Bar:Beat:Tick), and tempo.
/// Returns true if playback is active (for repaint scheduling).
fn draw_transport_bar(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
) -> bool {
    use egui_remixicon::icons as ri;
    let t = theme();
    let is_playing = handle.state.transport.is_playing();
    let current_ticks = handle.state.transport.get_ticks();
    let current_tick = Tick(current_ticks);
    let tempo_f32 = handle.state.transport.get_tempo().as_f32();
    let rec_state = handle.state.transport.recording_state();
    let metro_on = handle.state.transport.is_metronome_on();

    // Read time signature and song name from the song (non-blocking).
    let (time_sig, song_name) = song
        .try_read()
        .map(|s| (s.time_signature_at(current_tick), s.name.clone()))
        .unwrap_or((TimeSignature::COMMON, String::new()));

    // Phrase boundaries are the sorted, de-duplicated start and end ticks of
    // every placement (plus the song start) — the musical anchors the ◀◀/▶▶
    // buttons jump between, so navigation follows the music even when the tune
    // is not aligned to the 4/4 bar grid. They are only needed when those
    // buttons are actually clicked, so build them lazily here rather than
    // allocating + sorting on every frame.
    let phrase_boundaries = || -> Vec<u64> {
        song.try_read()
            .map(|s| {
                let mut boundaries: Vec<u64> = vec![0];
                for p in s.arrangement().iter() {
                    boundaries.push(p.start.0);
                    if let Some(pat) = s.pattern(p.pattern_id) {
                        boundaries.push(p.end(pat.length).0);
                    }
                }
                boundaries.sort_unstable();
                boundaries.dedup();
                boundaries
            })
            .unwrap_or_else(|| vec![0])
    };

    let (bar, beat, tick) = current_tick.to_bar_beat_tick(time_sig);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        // Song name
        if !song_name.is_empty() {
            ui.label(RichText::new(&song_name).color(t.colors.accent_cyan));
            ui.separator();
        }

        // Go to start
        if ui
            .button(RichText::new(ri::SKIP_BACK_MINI_FILL).color(t.colors.text_primary))
            .on_hover_text("Go to start")
            .clicked()
        {
            handle.send(EngineCommand::Seek { tick: Tick::ZERO });
        }

        // Previous phrase — jump to the previous placement boundary (or, with
        // Shift, the previous 4/4 bar line). Boundary stepping follows the
        // music; bar stepping is the rigid grid for aligned songs.
        let ticks_per_bar = u64::from(time_sig.ticks_per_bar());
        let shift = ui.input(|i| i.modifiers.shift);
        if ui
            .button(RichText::new(ri::REWIND_MINI_FILL).color(t.colors.text_primary))
            .on_hover_text("Previous phrase (Shift: previous bar)")
            .clicked()
        {
            let prev = if shift && ticks_per_bar > 0 {
                // On a bar line → a full bar back; otherwise snap to this bar.
                if current_ticks.is_multiple_of(ticks_per_bar) {
                    current_ticks.saturating_sub(ticks_per_bar)
                } else {
                    (current_ticks / ticks_per_bar) * ticks_per_bar
                }
            } else {
                // Last boundary strictly before the playhead (sorted ascending).
                phrase_boundaries()
                    .iter()
                    .copied()
                    .rfind(|&b| b < current_ticks)
                    .unwrap_or(0)
            };
            handle.send(EngineCommand::Seek { tick: Tick(prev) });
            view_state.reveal_playhead();
        }

        // Play / Pause toggle. Play starts from the cursor (or resumes in
        // place after a pause); Pause freezes the playhead where it is.
        if is_playing {
            if ui
                .button(RichText::new(ri::PAUSE_FILL).color(t.colors.accent_yellow))
                .on_hover_text("Pause — hold position (Play resumes here)")
                .clicked()
            {
                handle.send(EngineCommand::Pause);
            }
        } else if ui
            .button(RichText::new(ri::PLAY_FILL).color(t.colors.accent_green))
            .on_hover_text("Play — from the cursor")
            .clicked()
        {
            handle.send(EngineCommand::Play);
            view_state.auto_follow_playhead = true;
        }

        // Next phrase — jump to the next placement boundary (or, with Shift,
        // the next 4/4 bar line).
        if ui
            .button(RichText::new(ri::SPEED_MINI_FILL).color(t.colors.text_primary))
            .on_hover_text("Next phrase (Shift: next bar)")
            .clicked()
        {
            let next = if shift && ticks_per_bar > 0 {
                (current_ticks / ticks_per_bar + 1) * ticks_per_bar
            } else {
                // First boundary strictly after the playhead; stay put if none.
                phrase_boundaries()
                    .iter()
                    .copied()
                    .find(|&b| b > current_ticks)
                    .unwrap_or(current_ticks)
            };
            handle.send(EngineCommand::Seek { tick: Tick(next) });
            view_state.reveal_playhead();
        }

        // Stop returns the playhead to the cursor; a second press once it is
        // already at the cursor rewinds to the start. Disabled only when
        // stopped at the very beginning (nothing to return to or rewind).
        let stop_enabled = is_playing || current_ticks > 0;
        if ui
            .add_enabled(
                stop_enabled,
                egui::Button::new(RichText::new(ri::STOP_FILL).color(if is_playing {
                    t.colors.accent_red
                } else {
                    t.colors.text_primary
                })),
            )
            .on_hover_text("Stop — return to cursor (again: to start)")
            .clicked()
        {
            handle.send(EngineCommand::Stop);
            view_state.reveal_playhead();
        }

        // Record button
        let has_pattern = view_state.opened_pattern.is_some();
        let dim_red = DIM_RED;
        let rec_color = match rec_state {
            RecordingState::Capturing => t.colors.accent_red,
            RecordingState::CountIn => {
                let blink = ((ui.input(|i| i.time) * 4.0) as u64).is_multiple_of(2);
                if blink {
                    t.colors.accent_red
                } else {
                    t.colors.text_dim
                }
            }
            RecordingState::Armed => {
                let blink = ((ui.input(|i| i.time) * 2.0) as u64).is_multiple_of(2);
                if blink { t.colors.accent_red } else { dim_red }
            }
            RecordingState::Idle => {
                if has_pattern {
                    dim_red
                } else {
                    t.colors.text_dim
                }
            }
        };
        let rec_btn = ui.add_enabled(
            has_pattern,
            egui::Button::new(RichText::new(ri::RECORD_CIRCLE_FILL).color(rec_color)),
        );
        if rec_btn
            .on_hover_text(match rec_state {
                RecordingState::Idle => {
                    if has_pattern {
                        "Arm recording"
                    } else {
                        "Open a pattern in the piano roll to arm recording"
                    }
                }
                _ => "Disarm recording",
            })
            .clicked()
        {
            if rec_state != RecordingState::Idle {
                handle.send(EngineCommand::DisarmRecord);
            } else if let Some(pattern_id) = view_state.opened_pattern {
                arm_recording_for_pattern(handle, song, view_state, pattern_id);
            }
        }
        // Request repaint during blinking states
        if matches!(rec_state, RecordingState::Armed | RecordingState::CountIn) {
            ui.request_repaint();
        }

        // Metronome toggle
        if toggle_button(ui, "M", metro_on)
            .on_hover_text(if metro_on {
                "Metronome off"
            } else {
                "Metronome on"
            })
            .clicked()
        {
            handle.send(EngineCommand::SetMetronome(!metro_on));
        }

        // Quantize button — cycles Off → 1/4 → 1/8 → 1/16 → 1/32
        let q_label = match view_state.record_quantize {
            960 => "Q:1/4",
            480 => "Q:1/8",
            240 => "Q:1/16",
            120 => "Q:1/32",
            _ => "Q",
        };
        if toggle_button(ui, q_label, view_state.record_quantize > 0)
            .on_hover_text(match view_state.record_quantize {
                960 => "Quantize: 1/4 note (click to cycle)",
                480 => "Quantize: 1/8 note (click to cycle)",
                240 => "Quantize: 1/16 note (click to cycle)",
                120 => "Quantize: 1/32 note (click to cycle)",
                _ => "Quantize: Off (click to cycle)",
            })
            .clicked()
        {
            view_state.record_quantize = match view_state.record_quantize {
                0 => 960,
                960 => 480,
                480 => 240,
                240 => 120,
                _ => 0,
            };
        }

        // Overdub toggle
        if toggle_button(ui, "OVR", view_state.overdub)
            .on_hover_text(if view_state.overdub {
                "Overdub on (click for replace)"
            } else {
                "Overdub off (click to layer)"
            })
            .clicked()
        {
            view_state.overdub = !view_state.overdub;
        }

        ui.separator();

        // Position display: Bar:Beat:Tick (1-based)
        let pos_text = format!("{:03}:{:02}:{:03}", bar + 1, beat + 1, tick);
        ui.label(
            RichText::new(pos_text)
                .family(egui::FontFamily::Monospace)
                .size(16.0)
                .color(if is_playing {
                    t.colors.accent_primary
                } else {
                    t.colors.text_primary
                }),
        );

        ui.separator();

        // Tempo
        let mut tempo_val = tempo_f32;
        let tempo_response = ui.add(
            egui::DragValue::new(&mut tempo_val)
                .range(20.0..=300.0)
                .speed(0.5)
                .fixed_decimals(1)
                .suffix(" BPM"),
        );
        if tempo_response.changed() {
            handle.send(EngineCommand::SetTempo(Bpm::new(tempo_val)));
        }

        // Tap tempo
        if ui
            .button(RichText::new("TAP").size(10.0))
            .on_hover_text("Tap to set tempo (average of last 4 clicks)")
            .clicked()
        {
            let now = ui.input(|i| i.time);
            // Clicks older than 2 s are treated as the start of a new tap series.
            view_state.tap_tempo_times.retain(|t| now - *t < 2.0);
            view_state.tap_tempo_times.push(now);
            if view_state.tap_tempo_times.len() > 4 {
                let drop = view_state.tap_tempo_times.len() - 4;
                view_state.tap_tempo_times.drain(0..drop);
            }
            if view_state.tap_tempo_times.len() >= 2 {
                let first = view_state.tap_tempo_times[0];
                let last = *view_state.tap_tempo_times.last().unwrap_or(&first);
                let intervals = view_state.tap_tempo_times.len() as f64 - 1.0;
                let avg_interval = (last - first) / intervals;
                if avg_interval > 0.0 {
                    let bpm = (60.0 / avg_interval) as f32;
                    let clamped = bpm.clamp(20.0, 300.0);
                    handle.send(EngineCommand::SetTempo(Bpm::new(clamped)));
                }
            }
        }

        ui.separator();

        // Time signature — click to edit
        let ts_btn = ui
            .add(
                egui::Button::new(
                    RichText::new(format!("{}/{}", time_sig.numerator, time_sig.denominator))
                        .color(t.colors.text_secondary),
                )
                .frame(false),
            )
            .on_hover_text("Click to change time signature");
        egui::Popup::from_toggle_button_response(&ts_btn).show(|ui| {
            ui.set_min_width(180.0);
            ui.label(RichText::new("Time signature").strong());
            ui.add_space(t.spacing.xs);
            let mut num = time_sig.numerator as i32;
            let mut den = time_sig.denominator as i32;
            let mut changed = false;
            ui.horizontal(|ui| {
                if ui
                    .add(egui::DragValue::new(&mut num).range(1..=32).speed(0.1))
                    .changed()
                {
                    changed = true;
                }
                ui.label("/");
                egui::ComboBox::from_id_salt("ts_den")
                    .selected_text(format!("{den}"))
                    .width(56.0)
                    .show_ui(ui, |ui| {
                        for &allowed in &[1_i32, 2, 4, 8, 16, 32] {
                            if ui
                                .selectable_label(den == allowed, format!("{allowed}"))
                                .clicked()
                            {
                                den = allowed;
                                changed = true;
                            }
                        }
                    });
            });
            if changed
                && let Ok(num_u8) = u8::try_from(num.clamp(1, 32))
                && let Ok(den_u8) = u8::try_from(den.clamp(1, 32))
            {
                let new_sig = TimeSignature::new(num_u8, den_u8);
                song.write().default_time_signature = new_sig;
            }
        });

        ui.separator();

        // Song repeat toggle
        let repeat_icon = if view_state.repeat_enabled {
            RichText::new(ri::REPEAT_FILL).color(t.colors.accent_primary)
        } else {
            RichText::new(ri::REPEAT_LINE).color(t.colors.text_dim)
        };
        if ui
            .button(repeat_icon)
            .on_hover_text(if view_state.repeat_enabled {
                "Disable song repeat"
            } else {
                "Repeat song"
            })
            .clicked()
        {
            view_state.repeat_enabled = !view_state.repeat_enabled;
            handle.send(EngineCommand::SetRepeat {
                enabled: view_state.repeat_enabled,
            });
        }

        // Without a status badge, a stale loop region quietly clips
        // playback — only the right-click menu reveals it exists.
        let (loop_enabled, loop_start, loop_end) = handle.state.transport.loop_state();
        if loop_enabled && loop_end.0 > loop_start.0 {
            let (s_bar, _, _) = loop_start.to_bar_beat_tick(time_sig);
            let (e_bar, _, _) = loop_end.to_bar_beat_tick(time_sig);
            let badge = format!("LOOP {}–{}", s_bar + 1, e_bar + 1);
            let resp = ui
                .add(
                    egui::Button::new(RichText::new(badge).color(LOOP_COLOR).strong()).frame(false),
                )
                .on_hover_text("Transport loop active — click to clear.");
            if resp.clicked() {
                handle.send(EngineCommand::SetLoop {
                    start: Tick::ZERO,
                    end: Tick::ZERO,
                    enabled: false,
                });
                view_state.loop_start_tick = None;
                view_state.loop_end_tick = None;
            }
            ui.separator();
        }

        ui.separator();

        // Status indicator
        match rec_state {
            RecordingState::Capturing => {
                ui.label(RichText::new("REC").color(t.colors.accent_red).strong());
            }
            RecordingState::CountIn => {
                ui.label(
                    RichText::new("COUNT-IN")
                        .color(t.colors.accent_red)
                        .strong(),
                );
            }
            RecordingState::Armed => {
                ui.label(RichText::new("ARM").color(ARM_RED));
            }
            RecordingState::Idle => {
                if is_playing {
                    ui.label(RichText::new("PLAYING").color(t.colors.meter_green));
                } else if current_ticks > 0 {
                    ui.label(RichText::new("PAUSED").color(t.colors.accent_yellow));
                } else {
                    ui.label(RichText::new("STOPPED").color(t.colors.text_dim));
                }
            }
        }
    });

    is_playing
}

// ============================================================================
// ARRANGEMENT VIEW
// ============================================================================

/// Snapshot of track info needed for rendering (avoids holding RwLock during paint).
struct TrackInfo {
    id: TrackId,
    name: String,
    description: String,
    color: Color32,
    track_color: synth_sequencer::TrackColor,
    volume: NormalizedValue,
    pan: BipolarValue,
    mute: bool,
    solo: bool,
    instrument_id: SeqInstrumentId,
}

/// Snapshot of a pattern in the song (for pattern management UI).
struct PatternInfo {
    id: PatternId,
    name: String,
    length_ticks: u32,
}

/// A miniature note rectangle for pattern preview.
struct NoteMiniature {
    /// Horizontal position as fraction of pattern length (0.0–1.0).
    start_frac: f32,
    /// Width as fraction of pattern length.
    duration_frac: f32,
    /// Vertical position as fraction of pitch range (0.0 = lowest, 1.0 = highest).
    pitch_frac: f32,
}

/// Snapshot of a pattern placement for rendering.
struct PlacementInfo {
    pattern_id: PatternId,
    track_id: TrackId,
    start_tick: u64,
    end_tick: u64,
    pattern_name: String,
    note_count: usize,
    color: Color32,
    /// Instrument of this placement's track — drives miniature colour.
    instrument: SeqInstrumentId,
    /// Length in beats (for tooltip).
    length_beats: f32,
    /// Note miniatures for preview drawing.
    note_miniatures: Vec<NoteMiniature>,
}

/// Collected song data for arrangement rendering.
struct ArrangementData {
    tracks: Vec<TrackInfo>,
    placements: Vec<PlacementInfo>,
    patterns: Vec<PatternInfo>,
    time_sig: TimeSignature,
    song_end_tick: u64,
    /// Tempo automation points: (tick, BPM). Sorted by tick.
    tempo_changes: Vec<(u64, f32)>,
}

/// Collect arrangement data from song (short read-lock, then release).
fn collect_arrangement_data(song: &Arc<RwLock<Song>>) -> Option<ArrangementData> {
    let song = song.try_read()?;

    let tracks: Vec<TrackInfo> = song
        .tracks()
        .map(|t| TrackInfo {
            id: t.id,
            name: t.name.clone(),
            description: t.description.clone(),
            color: track_color_to_egui(t.color),
            track_color: t.color,
            volume: t.volume,
            pan: t.pan,
            mute: t.mute,
            solo: t.solo,
            instrument_id: t.instrument,
        })
        .collect();

    let patterns: Vec<PatternInfo> = song
        .patterns()
        .map(|p| PatternInfo {
            id: p.id,
            name: p.name.clone(),
            length_ticks: p.length.0,
        })
        .collect();

    let mut song_end_tick: u64 = 0;
    let placements: Vec<PlacementInfo> = song
        .arrangement()
        .iter()
        .filter_map(|p| {
            let pattern = song.pattern(p.pattern_id)?;
            let end = p.end(pattern.length);
            if end.0 > song_end_tick {
                song_end_tick = end.0;
            }
            // Color and instrument from the track this placement belongs to.
            let track = song.track(p.track_id);
            let color = track
                .map(|t| track_color_to_egui(t.color))
                .unwrap_or(Color32::GRAY);
            let instrument = track.map(|t| t.instrument).unwrap_or_default();

            let length_beats = pattern.length.0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;

            // Build note miniatures for preview
            let notes = pattern.notes();
            let note_miniatures = if notes.is_empty() || pattern.length.0 == 0 {
                Vec::new()
            } else {
                let len = pattern.length.0 as f32;
                // Find pitch range for normalization
                let (min_pitch, max_pitch) = notes.iter().fold((127_u8, 0_u8), |(lo, hi), n| {
                    let p = n.pitch.as_midi();
                    (lo.min(p), hi.max(p))
                });
                let pitch_range = (max_pitch - min_pitch).max(1) as f32;

                // Bound the snapshot at what the draw loop could ever use:
                // a placement is at most length_beats × PIXELS_PER_BEAT ×
                // MAX_ZOOM pixels wide, and past MINIATURE_NOTES_PER_PIXEL
                // notes per pixel drawing is invisible. Decimate evenly past
                // that (notes are sorted by start tick) so a pathologically
                // dense pattern cannot blow up the per-frame snapshot cost.
                #[allow(clippy::cast_sign_loss)]
                let budget =
                    ((length_beats * PIXELS_PER_BEAT * MAX_ZOOM * MINIATURE_NOTES_PER_PIXEL)
                        as usize)
                        .max(1);
                let step = notes.len().div_ceil(budget).max(1);

                notes
                    .iter()
                    .step_by(step)
                    .map(|n| {
                        let dur = n.duration.map_or(len * 0.02, |d| d.0 as f32);
                        NoteMiniature {
                            start_frac: n.start.0 as f32 / len,
                            duration_frac: dur / len,
                            pitch_frac: (n.pitch.as_midi() - min_pitch) as f32 / pitch_range,
                        }
                    })
                    .collect()
            };

            Some(PlacementInfo {
                pattern_id: p.pattern_id,
                track_id: p.track_id,
                start_tick: p.start.0,
                end_tick: end.0,
                pattern_name: pattern.name.clone(),
                note_count: pattern.notes().len(),
                color,
                instrument,
                length_beats,
                note_miniatures,
            })
        })
        .collect();

    let time_sig = song.default_time_signature;
    let tempo_changes: Vec<(u64, f32)> = song
        .tempo_changes()
        .iter()
        .map(|tc| (tc.tick.0, tc.bpm.as_f32()))
        .collect();

    Some(ArrangementData {
        tracks,
        placements,
        patterns,
        time_sig,
        song_end_tick,
        tempo_changes,
    })
}

/// Arrangement view toolbar: zoom controls (out / 1x / in / fit), the snap
/// selector, and the follow-playhead toggle.
fn draw_arrangement_toolbar(
    ui: &mut egui::Ui,
    data: &ArrangementData,
    view_state: &mut SequencerViewState,
) {
    let t = theme();
    let beats_per_bar = data.time_sig.numerator as u64;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new("Zoom").color(t.colors.text_dim).size(10.0));
        if ui.small_button("-").on_hover_text("Zoom out").clicked() {
            view_state.zoom_level = (view_state.zoom_level * 0.8).clamp(MIN_ZOOM, MAX_ZOOM);
        }
        if ui.small_button("1x").on_hover_text("Reset zoom").clicked() {
            view_state.zoom_level = 1.0;
        }
        if ui.small_button("+").on_hover_text("Zoom in").clicked() {
            view_state.zoom_level = (view_state.zoom_level * 1.25).clamp(MIN_ZOOM, MAX_ZOOM);
        }
        if ui
            .small_button("Fit")
            .on_hover_text("Fit song to view")
            .clicked()
        {
            // Approximate: scale zoom so the whole song fits the visible width
            let visible_w = ui.available_width().max(200.0);
            let song_beats = (data.song_end_tick as f32
                / synth_sequencer::TICKS_PER_QUARTER as f32)
                .max(beats_per_bar as f32 * MIN_VISIBLE_BARS as f32);
            let needed = visible_w / song_beats.max(1.0) / PIXELS_PER_BEAT;
            view_state.zoom_level = needed.clamp(MIN_ZOOM, MAX_ZOOM);
        }

        ui.separator();
        ui.label(RichText::new("Snap").color(t.colors.text_dim).size(10.0));
        let snap_options = [
            ("Off", 0_u32),
            ("1/2 beat", synth_sequencer::TICKS_PER_QUARTER / 2),
            ("Beat", synth_sequencer::TICKS_PER_QUARTER),
            ("Bar", data.time_sig.ticks_per_bar()),
            ("2 bars", data.time_sig.ticks_per_bar() * 2),
        ];
        let snap_label = snap_options
            .iter()
            .find(|(_, ticks)| *ticks == view_state.arrangement_snap_ticks)
            .map_or("Custom", |(l, _)| l);
        egui::ComboBox::from_id_salt("arrangement_snap")
            .selected_text(snap_label)
            .width(80.0)
            .show_ui(ui, |ui| {
                for (label, ticks) in snap_options {
                    if ui
                        .selectable_label(view_state.arrangement_snap_ticks == ticks, label)
                        .clicked()
                    {
                        view_state.arrangement_snap_ticks = ticks;
                    }
                }
            });

        ui.separator();
        let follow_color = if view_state.auto_follow_playhead {
            t.colors.accent_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .small_button(RichText::new("Follow").color(follow_color))
            .on_hover_text("Auto-scroll to keep the playhead visible")
            .clicked()
        {
            if view_state.auto_follow_playhead {
                view_state.auto_follow_playhead = false;
            } else {
                // Bring an off-screen marker into view even while stopped.
                view_state.reveal_playhead();
            }
        }
        if ui
            .small_button("Go to playhead")
            .on_hover_text("Scroll the timeline to the current playhead position")
            .clicked()
        {
            view_state.reveal_playhead();
        }
    });
    ui.separator();
}

/// Draw the arrangement view with track headers and timeline.
/// Returns `Some(PatternId)` if a placement was double-clicked.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn draw_arrangement(
    ui: &mut egui::Ui,
    data: &ArrangementData,
    current_tick: u64,
    is_playing: bool,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    undo_manager: &mut crate::undo::UndoManager,
) -> Option<PatternId> {
    use egui_remixicon::icons as ri;
    let t = theme();
    let track_count = data.tracks.len();

    // Mirror the engine's loop region into view_state so MCP-set and
    // song-repeat loops surface as ruler markers without having to be
    // re-set via the right-click menu first.
    let (engine_loop_enabled, engine_loop_start, engine_loop_end) =
        handle.state.transport.loop_state();
    if engine_loop_enabled && engine_loop_end.0 > engine_loop_start.0 {
        view_state.loop_start_tick = Some(engine_loop_start);
        view_state.loop_end_tick = Some(engine_loop_end);
    }

    // ── Empty state: show "Add Track" button ──
    if track_count == 0 {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Empty song")
                    .size(16.0)
                    .color(t.colors.text_dim),
            );
            ui.add_space(t.spacing.md);
            if ui
                .button(
                    RichText::new(format!("{} Add Track", ri::ADD_LINE))
                        .color(t.colors.accent_green),
                )
                .clicked()
            {
                let _ = song.write().create_track("Track 1");
            }
        });
        return None;
    }

    // Calculate timeline extent
    let ticks_per_bar = data.time_sig.ticks_per_bar() as u64;
    let ticks_per_beat = data.time_sig.ticks_per_beat() as u64;
    let beats_per_bar = data.time_sig.numerator as u64;

    let pixels_per_beat = PIXELS_PER_BEAT * view_state.zoom_level;

    draw_arrangement_toolbar(ui, data, view_state);

    let song_bars = if ticks_per_bar > 0 {
        data.song_end_tick.div_ceil(ticks_per_bar) as u32
    } else {
        MIN_VISIBLE_BARS
    };
    let total_bars = song_bars.max(MIN_VISIBLE_BARS) + 2;
    let total_beats = total_bars as f32 * beats_per_bar as f32;
    let timeline_width = total_beats * pixels_per_beat;

    let mut double_clicked_pattern: Option<PatternId> = None;

    // ── Shared scroll state ──
    // The right-side timeline owns the vertical scroll. The left header panel
    // mirrors the same y-offset so headers stay aligned with their rows.
    let scroll_salt = "seq_scroll";
    let scroll_id = ui.make_persistent_id(egui::Id::new(scroll_salt));
    let header_v_offset = egui::scroll_area::State::load(ui.ctx(), scroll_id)
        .map(|s| s.offset.y)
        .unwrap_or(0.0);

    // ── Track header panel (left side, uses egui widgets) ──
    egui::Panel::left("seq_track_headers")
        .exact_size(TRACK_HEADER_WIDTH)
        .resizable(false)
        .show_inside(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;

            // Ruler corner placeholder (pinned, outside the scroll area)
            ui.allocate_space(Vec2::new(TRACK_HEADER_WIDTH, RULER_HEIGHT));

            egui::ScrollArea::vertical()
                .id_salt("seq_track_headers_scroll")
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .scroll_source(egui::scroll_area::ScrollSource::NONE)
                .vertical_scroll_offset(header_v_offset)
                .show(ui, |ui| {
                    for (i, track) in data.tracks.iter().enumerate() {
                        let is_selected = view_state.selected_track == Some(track.id);
                        let is_highlighted = view_state.highlighted_track == Some(track.id);
                        let bg = if is_selected {
                            TRACK_HEADER_SELECTED_FILL
                        } else if is_highlighted {
                            TRACK_HIGHLIGHT_FILL
                        } else if i % 2 == 0 {
                            t.colors.bg_module
                        } else {
                            t.colors.bg_panel
                        };

                        let frame = egui::Frame::new()
                            .fill(bg)
                            .stroke(if is_selected {
                                Stroke::new(1.0, t.colors.accent_cyan)
                            } else {
                                Stroke::NONE
                            })
                            .inner_margin(egui::Margin::symmetric(4, 2));

                        frame.show(ui, |ui| {
                            ui.set_min_height(TRACK_ROW_HEIGHT - 4.0);
                            ui.set_max_height(TRACK_ROW_HEIGHT - 4.0);
                            ui.set_min_width(ui.available_width());

                            // Color indicator + name row
                            ui.horizontal(|ui| {
                                // Color indicator
                                let (color_rect, _) = ui.allocate_exact_size(
                                    Vec2::new(4.0, TRACK_ROW_HEIGHT - 8.0),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(color_rect, 2.0, track.color);

                                ui.vertical(|ui| {
                                    // Track name (editable on click)
                                    if view_state.editing_track_name.as_ref().map(|(id, _)| *id)
                                        == Some(track.id)
                                    {
                                        if let Some((_, ref mut name_buf)) =
                                            view_state.editing_track_name
                                        {
                                            let resp = ui.add(
                                                egui::TextEdit::singleline(name_buf)
                                                    .desired_width(80.0)
                                                    .font(egui::FontId::proportional(12.0)),
                                            );
                                            if resp.lost_focus()
                                                || ui.input(|i| i.key_pressed(egui::Key::Enter))
                                            {
                                                let new_name = name_buf.clone();
                                                let tid = track.id;
                                                let old_name = track.name.clone();
                                                let mut applied = false;
                                                {
                                                    let mut song_w = song.write();
                                                    if let Some(t) = song_w.track_mut(tid)
                                                        && t.name != new_name
                                                    {
                                                        t.name = new_name.clone();
                                                        applied = true;
                                                    }
                                                }
                                                if applied {
                                                    undo_manager.push(
                                                        crate::undo::UndoAction::RenameTrack {
                                                            track_id: tid,
                                                            old_name,
                                                            new_name,
                                                        },
                                                    );
                                                }
                                                view_state.editing_track_name = None;
                                            } else if !resp.has_focus() {
                                                // Grab focus only on the first
                                                // frame so clicking elsewhere
                                                // commits naturally via
                                                // lost_focus.
                                                resp.request_focus();
                                            }
                                        }
                                    } else {
                                        let name_resp = ui.add(
                                            egui::Label::new(
                                                RichText::new(&track.name)
                                                    .size(12.0)
                                                    .color(t.colors.text_primary),
                                            )
                                            .sense(Sense::click()),
                                        );
                                        if name_resp.clicked() {
                                            view_state.selected_track = Some(track.id);
                                        }
                                        if name_resp.double_clicked() {
                                            view_state.editing_track_name =
                                                Some((track.id, track.name.clone()));
                                        }

                                        // Right-click context menu on track name
                                        name_resp.context_menu(|ui| {
                                            if ui.button("Rename").clicked() {
                                                view_state.editing_track_name =
                                                    Some((track.id, track.name.clone()));
                                                ui.close();
                                            }
                                            if ui
                                                .button(
                                                    RichText::new("Delete Track")
                                                        .color(t.colors.accent_red),
                                                )
                                                .clicked()
                                            {
                                                let mut captured: Option<(
                                                    synth_sequencer::SequencerTrack,
                                                    usize,
                                                    Vec<synth_sequencer::PatternPlacement>,
                                                )> = None;
                                                {
                                                    let mut song_w = song.write();
                                                    let idx = song_w
                                                        .tracks()
                                                        .position(|t| t.id == track.id)
                                                        .unwrap_or(0);
                                                    let placements: Vec<_> = song_w
                                                        .arrangement()
                                                        .iter()
                                                        .filter(|p| p.track_id == track.id)
                                                        .cloned()
                                                        .collect();
                                                    if let Some(deleted) =
                                                        song_w.delete_track(track.id)
                                                    {
                                                        captured = Some((deleted, idx, placements));
                                                    }
                                                }
                                                if let Some((trk, idx, plcs)) = captured {
                                                    undo_manager.push(
                                                        crate::undo::UndoAction::DeleteTrack {
                                                            track: trk,
                                                            track_index: idx,
                                                            placements: plcs,
                                                        },
                                                    );
                                                }
                                                ui.close();
                                            }
                                        });
                                    }

                                    // Instrument selector row
                                    let inst_label = instruments
                                        .iter()
                                        .find(|inst| inst.id.0 == track.instrument_id.0 as u64)
                                        .map_or_else(
                                            || "— (none) —".to_owned(),
                                            |inst| inst.name.clone(),
                                        );
                                    egui::ComboBox::from_id_salt(
                                        ui.id().with(("track_instr", track.id.0)),
                                    )
                                    .selected_text(RichText::new(&inst_label).size(11.0))
                                    .width(116.0)
                                    .show_ui(ui, |ui| {
                                        for inst in instruments {
                                            let seq_id = SeqInstrumentId::new(inst.id.0 as u16);
                                            let selected = track.instrument_id == seq_id;
                                            if ui.selectable_label(selected, &inst.name).clicked()
                                                && let mut song_w = song.write()
                                                && let Some(trk) = song_w.track_mut(track.id)
                                            {
                                                trk.instrument = seq_id;
                                            }
                                        }
                                    });

                                    // Mute / Solo row
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        // Mute button
                                        let m_color = if track.mute {
                                            t.colors.accent_red
                                        } else {
                                            t.colors.text_dim
                                        };
                                        if ui
                                            .button(RichText::new("M").size(10.0).color(m_color))
                                            .on_hover_text("Mute")
                                            .clicked()
                                            && let mut song_w = song.write()
                                            && let Some(trk) = song_w.track_mut(track.id)
                                        {
                                            trk.toggle_mute();
                                        }

                                        // Solo button
                                        let s_color = if track.solo {
                                            t.colors.accent_yellow
                                        } else {
                                            t.colors.text_dim
                                        };
                                        if ui
                                            .button(RichText::new("S").size(10.0).color(s_color))
                                            .on_hover_text("Solo")
                                            .clicked()
                                            && let mut song_w = song.write()
                                            && let Some(trk) = song_w.track_mut(track.id)
                                        {
                                            trk.toggle_solo();
                                        }

                                        // Reorder buttons (move up / down)
                                        let track_count_local = data.tracks.len();
                                        let up_enabled = i > 0;
                                        let down_enabled = i + 1 < track_count_local;
                                        if ui
                                            .add_enabled(
                                                up_enabled,
                                                egui::Button::new(
                                                    RichText::new(ri::ARROW_UP_S_LINE)
                                                        .size(10.0)
                                                        .color(t.colors.text_secondary),
                                                ),
                                            )
                                            .on_hover_text("Move track up")
                                            .clicked()
                                            && i > 0
                                        {
                                            song.write().reorder_track(i, i - 1);
                                        }
                                        if ui
                                            .add_enabled(
                                                down_enabled,
                                                egui::Button::new(
                                                    RichText::new(ri::ARROW_DOWN_S_LINE)
                                                        .size(10.0)
                                                        .color(t.colors.text_secondary),
                                                ),
                                            )
                                            .on_hover_text("Move track down")
                                            .clicked()
                                        {
                                            song.write().reorder_track(i, i + 1);
                                        }

                                        // Properties button (volume / pan / colour)
                                        let prop_btn = ui
                                            .button(
                                                RichText::new(ri::MORE_FILL)
                                                    .size(10.0)
                                                    .color(t.colors.text_secondary),
                                            )
                                            .on_hover_text(
                                                "Track properties (volume, pan, colour)",
                                            );
                                        egui::Popup::from_toggle_button_response(&prop_btn).show(
                                            |ui| {
                                                ui.set_min_width(220.0);
                                                ui.label(
                                                    RichText::new("Track properties").strong(),
                                                );
                                                ui.add_space(t.spacing.xs);

                                                // Volume
                                                let mut vol = track.volume.as_f32();
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new("Vol")
                                                            .color(t.colors.text_dim),
                                                    );
                                                    if ui
                                                        .add(
                                                            egui::Slider::new(&mut vol, 0.0..=1.0)
                                                                .show_value(true)
                                                                .fixed_decimals(2),
                                                        )
                                                        .changed()
                                                        && let mut song_w = song.write()
                                                        && let Some(trk) =
                                                            song_w.track_mut(track.id)
                                                    {
                                                        trk.volume = NormalizedValue::new(vol);
                                                    }
                                                });

                                                let mut pan_bi = track.pan.as_f32();
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new("Pan")
                                                            .color(t.colors.text_dim),
                                                    );
                                                    if ui
                                                        .add(
                                                            egui::Slider::new(
                                                                &mut pan_bi,
                                                                -1.0..=1.0,
                                                            )
                                                            .show_value(true)
                                                            .fixed_decimals(2),
                                                        )
                                                        .changed()
                                                        && let mut song_w = song.write()
                                                        && let Some(trk) =
                                                            song_w.track_mut(track.id)
                                                    {
                                                        trk.pan = BipolarValue::new(pan_bi);
                                                    }
                                                });

                                                // Description (utility metadata, no undo).
                                                // Buffered so edits survive the per-frame
                                                // snapshot rebuild; committed on lost focus.
                                                ui.add_space(t.spacing.xs);
                                                ui.label(
                                                    RichText::new("Description")
                                                        .color(t.colors.text_dim),
                                                );
                                                let editing_this = view_state
                                                    .editing_track_description
                                                    .as_ref()
                                                    .map(|(id, _)| *id)
                                                    == Some(track.id);
                                                if editing_this {
                                                    if let Some((_, ref mut desc_buf)) =
                                                        view_state.editing_track_description
                                                    {
                                                        let resp = ui.add(
                                                            egui::TextEdit::multiline(desc_buf)
                                                                .desired_rows(2)
                                                                .desired_width(f32::INFINITY)
                                                                .hint_text("Description"),
                                                        );
                                                        // Commit on every change so a popup
                                                        // dismissal (click-outside) can never
                                                        // strand an in-progress edit; lost_focus
                                                        // just ends the edit session.
                                                        if resp.changed() {
                                                            let new_desc = desc_buf.clone();
                                                            let mut song_w = song.write();
                                                            if let Some(trk) =
                                                                song_w.track_mut(track.id)
                                                                && trk.description != new_desc
                                                            {
                                                                trk.description = new_desc;
                                                            }
                                                        }
                                                        if resp.lost_focus() {
                                                            view_state.editing_track_description =
                                                                None;
                                                        } else if !resp.has_focus() {
                                                            resp.request_focus();
                                                        }
                                                    }
                                                } else {
                                                    let desc_text = if track.description.is_empty()
                                                    {
                                                        RichText::new("(click to add)")
                                                            .italics()
                                                            .color(t.colors.text_dim)
                                                    } else {
                                                        RichText::new(&track.description)
                                                            .color(t.colors.text_secondary)
                                                    };
                                                    if ui
                                                        .add(
                                                            egui::Label::new(desc_text)
                                                                .sense(Sense::click()),
                                                        )
                                                        .clicked()
                                                    {
                                                        view_state.editing_track_description = Some(
                                                            (track.id, track.description.clone()),
                                                        );
                                                    }
                                                }

                                                ui.add_space(t.spacing.xs);
                                                ui.label(
                                                    RichText::new("Colour")
                                                        .color(t.colors.text_dim),
                                                );
                                                ui.horizontal_wrapped(|ui| {
                                                    for preset in
                                                        synth_sequencer::TrackColor::presets()
                                                    {
                                                        let c = Color32::from_rgb(
                                                            preset.r, preset.g, preset.b,
                                                        );
                                                        let selected = track.track_color == *preset;
                                                        let stroke = if selected {
                                                            Stroke::new(2.0, t.colors.accent_cyan)
                                                        } else {
                                                            Stroke::NONE
                                                        };
                                                        let (rect, resp) = ui.allocate_exact_size(
                                                            Vec2::new(18.0, 18.0),
                                                            Sense::click(),
                                                        );
                                                        ui.painter().rect_filled(rect, 3.0, c);
                                                        ui.painter().rect_stroke(
                                                            rect,
                                                            3.0,
                                                            stroke,
                                                            egui::StrokeKind::Inside,
                                                        );
                                                        if resp.clicked()
                                                            && let mut song_w = song.write()
                                                            && let Some(trk) =
                                                                song_w.track_mut(track.id)
                                                        {
                                                            trk.color = *preset;
                                                        }
                                                    }
                                                });
                                            },
                                        );
                                    });
                                });
                            });
                        });
                    }

                    // "+" button to add track
                    ui.add_space(t.spacing.xs);
                    if ui
                        .button(
                            RichText::new(format!("{} Add Track", ri::ADD_LINE))
                                .size(11.0)
                                .color(t.colors.accent_green),
                        )
                        .clicked()
                    {
                        let mut song_w = song.write();
                        let count = song_w.track_count();
                        let _ = song_w.create_track(format!("Track {}", count + 1));
                    }
                });
        });

    // ── Timeline area (right side, uses painter for performance) ──
    // The scroll area's actual ID = ui.make_persistent_id(Id::new(salt))

    // Pre-set scroll offset for auto-follow before showing the scroll area.
    // While playing we continuously keep the playhead ~30% from the right edge.
    // While stopped we only scroll inside the settle window right after a
    // transport action (◀◀/▶▶ jump, ruler seek, stop-to-cursor, Go to
    // playhead), and only if the marker landed off-screen — outside that
    // window manual scrolling is never fought.
    if view_state.auto_follow_playhead && ticks_per_beat > 0 {
        let playhead_x_offset = current_tick as f32 / ticks_per_beat as f32 * pixels_per_beat;
        let visible_width = ui.available_width();
        let mut scroll_state =
            egui::scroll_area::State::load(ui.ctx(), scroll_id).unwrap_or_default();
        let current_offset = scroll_state.offset.x;

        let target_offset = if is_playing {
            // Continuous follow: keep the playhead ~30% from the right edge.
            Some((playhead_x_offset - visible_width * 0.7).max(0.0))
        } else if view_state.follow_settle_frames > 0 {
            // Settle window: if the marker is off-screen, re-center it ~30%
            // from the left so the music ahead of it stays visible. A small
            // slack keeps it from hugging the edge.
            let margin = pixels_per_beat;
            let off_screen = playhead_x_offset < current_offset + margin
                || playhead_x_offset > current_offset + visible_width - margin;
            off_screen.then(|| (playhead_x_offset - visible_width * 0.3).max(0.0))
        } else {
            // Stopped with no recent transport action: leave the user's
            // scroll position alone.
            None
        };

        if let Some(target_offset) = target_offset {
            scroll_state.offset.x = target_offset;
            scroll_state.store(ui.ctx(), scroll_id);
            view_state.last_auto_scroll_offset = Some(target_offset);
        }
    }

    let scroll_output = egui::ScrollArea::both()
        .id_salt(scroll_salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let total_size = Vec2::new(
                timeline_width,
                RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT,
            );
            let (response, painter) = ui.allocate_painter(total_size, Sense::click_and_drag());
            let painter_rect = response.rect;

            let tl_x = painter_rect.min.x;
            let tl_y = painter_rect.min.y;

            let tick_to_x = |tick_val: u64| -> f32 {
                if ticks_per_beat == 0 {
                    return tl_x;
                }
                let beats = tick_val as f32 / ticks_per_beat as f32;
                tl_x + beats * pixels_per_beat
            };

            // Helper: x position to tick
            let x_to_tick = |x: f32| -> u64 {
                if ticks_per_beat == 0 {
                    return 0;
                }
                let beats = (x - tl_x) / pixels_per_beat;
                (beats * ticks_per_beat as f32).max(0.0) as u64
            };

            // Single snap unit shared by placement create, drag, resize, and
            // loop-region — see `snap_to_step` for the underlying math.
            let snap_ticks = view_state.arrangement_snap_ticks as u64;
            let snap_tick = |tick: u64| snap_to_step(tick, snap_ticks);

            // Helper: y position to track row index
            let y_to_row = |y: f32| -> Option<usize> {
                let row_offset = y - (tl_y + RULER_HEIGHT);
                if row_offset < 0.0 || TRACK_ROW_HEIGHT <= 0.0 {
                    return None;
                }
                let idx = (row_offset / TRACK_ROW_HEIGHT) as usize;
                if idx < track_count { Some(idx) } else { None }
            };

            // ── Ruler (bar/beat numbers) ──
            let ruler_rect = Rect::from_min_size(
                Pos2::new(tl_x, tl_y),
                Vec2::new(timeline_width, RULER_HEIGHT),
            );
            draw_ruler_labels(
                &painter,
                &t,
                ruler_rect,
                total_bars,
                ticks_per_bar,
                tick_to_x,
            );

            // ── Full-height bar/beat grid lines ──
            for bar_idx in 0..total_bars {
                let bar_tick = bar_idx as u64 * ticks_per_bar;
                let x = tick_to_x(bar_tick);

                let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
                painter.line_segment(
                    [Pos2::new(x, tl_y + RULER_HEIGHT), Pos2::new(x, line_bottom)],
                    Stroke::new(1.0, t.colors.border),
                );

                for beat in 1..beats_per_bar {
                    let beat_tick = bar_tick + beat * ticks_per_beat;
                    let bx = tick_to_x(beat_tick);
                    painter.line_segment(
                        [
                            Pos2::new(bx, tl_y + RULER_HEIGHT),
                            Pos2::new(bx, line_bottom),
                        ],
                        Stroke::new(0.5, t.colors.border.gamma_multiply(0.4)),
                    );
                }
            }

            // ── Track row backgrounds ──
            for i in 0..track_count {
                let row_y = tl_y + RULER_HEIGHT + i as f32 * TRACK_ROW_HEIGHT;
                let is_highlighted = view_state.highlighted_track == Some(data.tracks[i].id);
                let bg = if is_highlighted {
                    TRACK_HIGHLIGHT_FILL
                } else if i % 2 == 0 {
                    TRACK_ROW_BG_EVEN
                } else {
                    Color32::TRANSPARENT
                };
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(tl_x, row_y),
                        Vec2::new(timeline_width, TRACK_ROW_HEIGHT),
                    ),
                    0.0,
                    bg,
                );
                // Row separator
                painter.line_segment(
                    [
                        Pos2::new(tl_x, row_y + TRACK_ROW_HEIGHT),
                        Pos2::new(tl_x + timeline_width, row_y + TRACK_ROW_HEIGHT),
                    ],
                    Stroke::new(0.5, t.colors.border),
                );
            }

            // ── Discoverability hint when no placements exist yet ──
            if data.placements.is_empty() && !data.tracks.is_empty() {
                let row_y = tl_y + RULER_HEIGHT + TRACK_ROW_HEIGHT * 0.5;
                painter.text(
                    Pos2::new(tl_x + 16.0, row_y),
                    egui::Align2::LEFT_CENTER,
                    "Double-click here to create a pattern",
                    egui::FontId::proportional(13.0),
                    t.colors.text_dim,
                );
            }

            // ── Pattern placements ──
            let mut placement_rects: Vec<(Rect, PatternId, TrackId, u64)> = Vec::new();
            // One-shot per-frame colour cache so mini-note rendering is O(1)
            // per note instead of O(notes × instruments) with a hex parse.
            let inst_color_cache = build_instrument_colour_cache(instruments);

            for placement in &data.placements {
                let Some(row_idx) = data.tracks.iter().position(|t| t.id == placement.track_id)
                else {
                    continue;
                };

                let x_start = tick_to_x(placement.start_tick);
                let x_end = tick_to_x(placement.end_tick);
                let row_y =
                    tl_y + RULER_HEIGHT + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
                let height = TRACK_ROW_HEIGHT - PLACEMENT_PADDING * 2.0;

                let rect = Rect::from_min_size(
                    Pos2::new(x_start, row_y),
                    Vec2::new((x_end - x_start).max(4.0), height),
                );

                placement_rects.push((
                    rect,
                    placement.pattern_id,
                    placement.track_id,
                    placement.start_tick,
                ));

                // Placement body uses the TRACK colour — uniform per track
                // so all placements on one track share the same row identity.
                let track_color = placement.color;

                let is_opened = view_state.opened_pattern == Some(placement.pattern_id);
                let fill_alpha = if is_opened { 140 } else { 100 };
                let fill = Color32::from_rgba_unmultiplied(
                    track_color.r(),
                    track_color.g(),
                    track_color.b(),
                    fill_alpha,
                );
                painter.rect_filled(rect, 3.0, fill);
                let stroke = if is_opened {
                    Stroke::new(2.0, t.colors.accent_cyan)
                } else {
                    Stroke::new(1.0, track_color)
                };
                painter.rect_stroke(rect, 3.0, stroke, egui::StrokeKind::Inside);

                let text_clip = Rect::from_min_max(
                    Pos2::new(rect.min.x + 4.0, rect.min.y),
                    Pos2::new(rect.max.x - 2.0, rect.max.y),
                );
                if text_clip.width() > 20.0 {
                    painter.with_clip_rect(text_clip).text(
                        Pos2::new(rect.min.x + 4.0, rect.min.y + 4.0),
                        egui::Align2::LEFT_TOP,
                        &placement.pattern_name,
                        egui::FontId::proportional(11.0),
                        t.colors.text_primary,
                    );
                }

                if !placement.note_miniatures.is_empty() {
                    let mini_top = rect.min.y + 18.0;
                    let mini_height = rect.max.y - mini_top - 2.0;
                    if mini_height > 4.0 {
                        let mini_width = rect.width() - 4.0;
                        let fallback = MINIATURE_FALLBACK;
                        let clipped = painter.with_clip_rect(rect);
                        // All notes in a placement play the placement's track instrument.
                        let inst_color = cached_instrument_color(
                            &inst_color_cache,
                            placement.instrument,
                            fallback,
                        );
                        // Pixel budget: drawing more notes than the box has
                        // horizontal pixels is invisible, so decimate evenly
                        // (notes are sorted by start tick, so every Nth note
                        // preserves the pattern's shape over its full length).
                        #[allow(clippy::cast_sign_loss)]
                        let budget = ((mini_width * MINIATURE_NOTES_PER_PIXEL) as usize).max(1);
                        let step = placement.note_miniatures.len().div_ceil(budget).max(1);
                        let note_color = Color32::from_rgba_unmultiplied(
                            inst_color.r(),
                            inst_color.g(),
                            inst_color.b(),
                            200,
                        );
                        for mini in placement.note_miniatures.iter().step_by(step) {
                            let nx = rect.min.x + 2.0 + mini.start_frac * mini_width;
                            let nw = (mini.duration_frac * mini_width).max(1.0);
                            let ny = mini_top + (1.0 - mini.pitch_frac) * (mini_height - 2.0);
                            clipped.rect_filled(
                                Rect::from_min_size(Pos2::new(nx, ny), Vec2::new(nw, 2.0)),
                                0.0,
                                note_color,
                            );
                        }
                    }
                }
            }

            // ── Double-click → open piano roll or create pattern ──
            if response.double_clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let mut hit_placement = false;
                for (rect, pattern_id, _, _) in &placement_rects {
                    if rect.contains(pos) {
                        double_clicked_pattern = Some(*pattern_id);
                        hit_placement = true;
                        break;
                    }
                }
                // Double-click on empty area → create pattern + place + open
                if !hit_placement && let Some(row_idx) = y_to_row(pos.y) {
                    let target_track = data.tracks[row_idx].id;
                    let click_tick = x_to_tick(pos.x);
                    let placement_tick = snap_tick(click_tick);
                    {
                        let mut song_w = song.write();
                        let new_pat_id = song_w.create_pattern(SeqDuration::WHOLE * 4);
                        song_w.place_pattern(new_pat_id, target_track, Tick(placement_tick));
                        double_clicked_pattern = Some(new_pat_id);
                    }
                }
            }

            // ── Hover hint + tooltip on placements ──
            if response.hovered()
                && let Some(pos) = ui.ctx().pointer_hover_pos()
            {
                let hovered_placement = data
                    .placements
                    .iter()
                    .zip(placement_rects.iter())
                    .find(|(_, (r, _, _, _))| r.contains(pos));

                if let Some((pl, _)) = hovered_placement {
                    ui.output_mut(|o| {
                        o.cursor_icon = CursorIcon::PointingHand;
                    });
                    // Tooltip with pattern info
                    let instr_name = data
                        .tracks
                        .iter()
                        .find(|t| t.id == pl.track_id)
                        .map(|t| t.instrument_id)
                        .and_then(|seq_id| {
                            instruments.iter().find(|inst| inst.id.0 == seq_id.0 as u64)
                        })
                        .map_or_else(|| "---".to_owned(), |inst| inst.name.clone());
                    let tip_name = pl.pattern_name.clone();
                    let tip_beats = pl.length_beats;
                    let tip_notes = pl.note_count;
                    response.clone().on_hover_ui(|ui: &mut egui::Ui| {
                        ui.label(
                            RichText::new(&tip_name)
                                .strong()
                                .color(t.colors.text_primary),
                        );
                        ui.label(format!("{tip_beats:.1} beats"));
                        ui.label(format!("{tip_notes} notes"));
                        ui.label(format!("Instrument: {instr_name}"));
                    });
                }
            }

            // ── Ctrl+scroll → timeline zoom ──
            if response.hovered() {
                let scroll_delta = ui.input(|i| {
                    if i.modifiers.ctrl || i.modifiers.command {
                        i.smooth_scroll_delta.y
                    } else {
                        0.0
                    }
                });
                if scroll_delta != 0.0 {
                    let factor = 1.0 + scroll_delta * 0.002;
                    view_state.zoom_level =
                        (view_state.zoom_level * factor).clamp(MIN_ZOOM, MAX_ZOOM);
                }
            }

            // ── Primary click: ruler seek or clear highlight ──
            if response.clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                if ruler_rect.contains(pos) {
                    // Click in ruler → seek to that position
                    let seek_tick = x_to_tick(pos.x);
                    handle.send(EngineCommand::Seek {
                        tick: Tick(seek_tick),
                    });
                    // Re-enable auto-follow on ruler click
                    view_state.reveal_playhead();
                } else {
                    view_state.highlighted_track = None;
                }
            }

            // ── Ruler hover: pointing hand cursor + indicator line ──
            if response.hovered()
                && let Some(pos) = ui.ctx().pointer_hover_pos()
                && ruler_rect.contains(pos)
            {
                ui.output_mut(|o| {
                    o.cursor_icon = CursorIcon::PointingHand;
                });
                // Draw subtle hover indicator line
                let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
                painter.line_segment(
                    [Pos2::new(pos.x, tl_y), Pos2::new(pos.x, line_bottom)],
                    Stroke::new(1.0, t.colors.text_dim.gamma_multiply(0.4)),
                );
            }

            // ── Capture right-click position + set highlighted track ──
            if response.secondary_clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                view_state.context_menu_pos = Some(pos);
                view_state.highlighted_track = y_to_row(pos.y).map(|i| data.tracks[i].id);
            }

            // ── Drag-to-move / resize placements ──
            const PLACEMENT_RESIZE_ZONE: f32 = 8.0;
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(pos) = response.interact_pointer_pos()
                && let Some((rect, pat_id, trk_id, start_tick)) =
                    placement_rects.iter().find(|(r, _, _, _)| r.contains(pos))
            {
                // Right-edge grab → resize. Body grab → move.
                if pos.x >= rect.max.x - PLACEMENT_RESIZE_ZONE {
                    if let Some(p) = data.placements.iter().find(|p| {
                        p.pattern_id == *pat_id
                            && p.track_id == *trk_id
                            && p.start_tick == *start_tick
                    }) {
                        let cur_len = SeqDuration((p.end_tick - p.start_tick) as u32);
                        view_state.drag = Some(DragState::ResizePlacement {
                            pattern_id: *pat_id,
                            track_id: *trk_id,
                            start_tick: Tick(*start_tick),
                            original_length: cur_len,
                            current_length: cur_len,
                        });
                    }
                } else {
                    let grab_tick = x_to_tick(pos.x);
                    view_state.drag = Some(DragState::DragPlacement {
                        pattern_id: *pat_id,
                        track_id: *trk_id,
                        start_tick: Tick(*start_tick),
                        current_tick: Tick(*start_tick),
                        current_track_id: *trk_id,
                        grab_offset_ticks: Tick(grab_tick.saturating_sub(*start_tick)),
                    });
                }
            }

            // ── Update drag state ──
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(pos) = response.interact_pointer_pos()
            {
                match &mut view_state.drag {
                    Some(DragState::DragPlacement {
                        current_tick,
                        current_track_id,
                        grab_offset_ticks,
                        ..
                    }) => {
                        let raw_tick = x_to_tick(pos.x).saturating_sub(grab_offset_ticks.0);
                        *current_tick = Tick(snap_tick(raw_tick));
                        if let Some(row_idx) = y_to_row(pos.y) {
                            *current_track_id = data.tracks[row_idx].id;
                        }
                    }
                    Some(DragState::ResizePlacement {
                        start_tick,
                        current_length,
                        ..
                    }) => {
                        let raw_end = x_to_tick(pos.x).max(start_tick.0 + 1);
                        let snapped_end = snap_tick(raw_end);
                        let end = snapped_end.max(start_tick.0 + 1);
                        *current_length = SeqDuration((end - start_tick.0).max(1) as u32);
                    }
                    _ => {}
                }
            }
            // ── Release drag → move placement / commit resize ──
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                match view_state.drag.take() {
                    Some(DragState::DragPlacement {
                        pattern_id,
                        track_id,
                        start_tick,
                        current_tick,
                        current_track_id,
                        ..
                    }) => {
                        if current_tick != start_tick || current_track_id != track_id {
                            let moved = song.write().move_placement(
                                pattern_id,
                                track_id,
                                start_tick,
                                current_track_id,
                                current_tick,
                            );
                            if moved {
                                undo_manager.push(crate::undo::UndoAction::MovePlacement {
                                    pattern_id,
                                    old_track_id: track_id,
                                    old_start: start_tick,
                                    new_track_id: current_track_id,
                                    new_start: current_tick,
                                });
                            }
                        }
                    }
                    Some(DragState::ResizePlacement {
                        pattern_id,
                        track_id,
                        start_tick,
                        original_length,
                        current_length,
                    }) => {
                        if current_length != original_length {
                            // Resolve the underlying pattern's native length so
                            // we can decide whether the override clears (== pattern.length).
                            let pattern_len = song
                                .read()
                                .pattern(pattern_id)
                                .map(|p| p.length)
                                .unwrap_or(current_length);
                            let new_override = if current_length == pattern_len {
                                None
                            } else {
                                Some(current_length)
                            };
                            // Old override is the original length unless it
                            // matched the pattern's native length too.
                            let old_override = if original_length == pattern_len {
                                None
                            } else {
                                Some(original_length)
                            };
                            song.write().set_placement_length(
                                pattern_id,
                                track_id,
                                start_tick,
                                new_override,
                            );
                            undo_manager.push(crate::undo::UndoAction::SetPlacementLength {
                                pattern_id,
                                track_id,
                                start: start_tick,
                                old_length: old_override,
                                new_length: new_override,
                            });
                        }
                    }
                    other => view_state.drag = other,
                }
            }

            // ── Draw resize ghost ──
            if let Some(DragState::ResizePlacement {
                track_id,
                start_tick,
                current_length,
                ..
            }) = &view_state.drag
                && let Some(row_idx) = data.tracks.iter().position(|t| t.id == *track_id)
            {
                let ghost_x = tick_to_x(start_tick.0);
                let ghost_end_x = tick_to_x(start_tick.0 + current_length.0 as u64);
                let ghost_y =
                    tl_y + RULER_HEIGHT + row_idx as f32 * TRACK_ROW_HEIGHT + PLACEMENT_PADDING;
                let ghost_rect = Rect::from_min_size(
                    Pos2::new(ghost_x, ghost_y),
                    Vec2::new(
                        (ghost_end_x - ghost_x).max(4.0),
                        TRACK_ROW_HEIGHT - PLACEMENT_PADDING * 2.0,
                    ),
                );
                painter.rect_stroke(
                    ghost_rect,
                    3.0,
                    Stroke::new(2.0, RESIZE_GHOST_STROKE),
                    egui::StrokeKind::Outside,
                );
            }

            // ── Draw drag ghost ──
            if let Some(DragState::DragPlacement {
                pattern_id,
                current_tick,
                current_track_id,
                ..
            }) = &view_state.drag
            {
                // Find original placement length
                if let Some(placement) =
                    data.placements.iter().find(|p| p.pattern_id == *pattern_id)
                {
                    let duration_ticks = placement.end_tick - placement.start_tick;
                    let ghost_x = tick_to_x(current_tick.0);
                    let ghost_end_x = tick_to_x(current_tick.0 + duration_ticks);
                    if let Some(row_idx) =
                        data.tracks.iter().position(|t| t.id == *current_track_id)
                    {
                        let ghost_y = tl_y
                            + RULER_HEIGHT
                            + row_idx as f32 * TRACK_ROW_HEIGHT
                            + PLACEMENT_PADDING;
                        let ghost_rect = Rect::from_min_size(
                            Pos2::new(ghost_x, ghost_y),
                            Vec2::new(
                                (ghost_end_x - ghost_x).max(4.0),
                                TRACK_ROW_HEIGHT - PLACEMENT_PADDING * 2.0,
                            ),
                        );
                        painter.rect_filled(ghost_rect, 3.0, DRAG_GHOST_FILL);
                        painter.rect_stroke(
                            ghost_rect,
                            3.0,
                            Stroke::new(1.5, DRAG_GHOST_STROKE),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }

            // ── Right-click context menu on timeline ──
            // Use stored position from secondary_clicked, not current hover
            let ctx_pos = view_state.context_menu_pos;
            response.context_menu(|ui| {
                ui.set_min_width(180.0);
                let hover_pos = ctx_pos.unwrap_or(ui.min_rect().min);

                // Right-click on the ruler shows loop-region commands.
                if ruler_rect.contains(hover_pos) {
                    let tick = Tick(x_to_tick(hover_pos.x));
                    let snapped = snap_tick(tick.0);
                    if ui.button("Set loop start here").clicked() {
                        view_state.loop_start_tick = Some(Tick(snapped));
                        if let Some(end) = view_state.loop_end_tick
                            && end.0 > snapped
                        {
                            handle.send(EngineCommand::SetLoop {
                                start: Tick(snapped),
                                end,
                                enabled: true,
                            });
                        }
                        ui.close();
                    }
                    if ui.button("Set loop end here").clicked() {
                        view_state.loop_end_tick = Some(Tick(snapped));
                        if let Some(start) = view_state.loop_start_tick
                            && snapped > start.0
                        {
                            handle.send(EngineCommand::SetLoop {
                                start,
                                end: Tick(snapped),
                                enabled: true,
                            });
                        }
                        ui.close();
                    }
                    if (view_state.loop_start_tick.is_some() || view_state.loop_end_tick.is_some())
                        && ui
                            .button(RichText::new("Clear loop").color(t.colors.accent_red))
                            .clicked()
                    {
                        view_state.loop_start_tick = None;
                        view_state.loop_end_tick = None;
                        handle.send(EngineCommand::SetLoop {
                            start: Tick::ZERO,
                            end: Tick::ZERO,
                            enabled: false,
                        });
                        ui.close();
                    }

                    ui.separator();

                    // Tempo automation at the clicked (snapped) tick.
                    let existing_bpm: Option<f32> = data
                        .tempo_changes
                        .iter()
                        .find(|(t, _)| *t == snapped)
                        .map(|(_, b)| *b);
                    let default_bpm = existing_bpm.unwrap_or_else(|| {
                        // Seed from the song's tempo at this position so
                        // dragging from a curve point feels stable.
                        song.try_read()
                            .map(|s| s.tempo_at(Tick(snapped)).as_f32())
                            .unwrap_or(120.0)
                    });
                    ui.menu_button("Set tempo here…", |ui| {
                        let mut bpm = default_bpm;
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut bpm)
                                    .range(20.0..=300.0)
                                    .speed(0.5)
                                    .fixed_decimals(1)
                                    .suffix(" BPM"),
                            );
                            if ui.button("Apply").clicked() {
                                let new_bpm = Bpm::new(bpm);
                                let old_bpm = existing_bpm.map(Bpm::new);
                                if old_bpm != Some(new_bpm) {
                                    song.write().set_tempo_at(Tick(snapped), new_bpm);
                                    undo_manager.push(crate::undo::UndoAction::SetTempo {
                                        tick: Tick(snapped),
                                        old_bpm,
                                        new_bpm: Some(new_bpm),
                                    });
                                }
                                ui.close();
                            }
                        });
                        if let Some(existing) = existing_bpm {
                            ui.separator();
                            if ui
                                .button(
                                    RichText::new("Remove tempo change here")
                                        .color(t.colors.accent_red),
                                )
                                .clicked()
                            {
                                if song.write().remove_tempo_change(Tick(snapped)) {
                                    undo_manager.push(crate::undo::UndoAction::SetTempo {
                                        tick: Tick(snapped),
                                        old_bpm: Some(Bpm::new(existing)),
                                        new_bpm: None,
                                    });
                                }
                                ui.close();
                            }
                        }
                    });
                    return;
                }

                // Check if right-click is on an existing placement
                let clicked_placement = placement_rects
                    .iter()
                    .find(|(r, _, _, _)| r.contains(hover_pos));

                if let Some((_, pat_id, trk_id, start_tick)) = clicked_placement {
                    let pat_id = *pat_id;
                    let trk_id = *trk_id;
                    let start_tick = *start_tick;

                    if ui.button("Open in Piano Roll").clicked() {
                        double_clicked_pattern = Some(pat_id);
                        ui.close();
                    }
                    if ui.button("Rename Pattern").clicked() {
                        // Get current name
                        let current_name = data
                            .patterns
                            .iter()
                            .find(|p| p.id == pat_id)
                            .map_or_else(String::new, |p| p.name.clone());
                        view_state.editing_pattern_name = Some((pat_id, current_name));
                        // The inline rename editor lives in the piano-roll
                        // toolbar — open it so the user can actually type.
                        double_clicked_pattern = Some(pat_id);
                        ui.close();
                    }

                    // Pattern length editing — free-input bars
                    ui.menu_button("Set Length…", |ui| {
                        let ticks_per_bar = data.time_sig.ticks_per_bar().max(1);
                        let current_len = data
                            .patterns
                            .iter()
                            .find(|p| p.id == pat_id)
                            .map_or(ticks_per_bar, |p| p.length_ticks);
                        let mut bars = (current_len / ticks_per_bar).max(1) as i32;
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::DragValue::new(&mut bars)
                                    .range(1..=64)
                                    .speed(0.1)
                                    .suffix(" bars"),
                            );
                            if ui.button("Apply").clicked() {
                                let new_len = SeqDuration(bars.max(1) as u32 * ticks_per_bar);
                                let mut applied: Option<SeqDuration> = None;
                                {
                                    let mut song_w = song.write();
                                    if let Some(pat) = song_w.pattern_mut(pat_id)
                                        && pat.length != new_len
                                    {
                                        applied = Some(pat.length);
                                        pat.length = new_len;
                                    }
                                }
                                if let Some(old) = applied {
                                    undo_manager.push(crate::undo::UndoAction::SetPatternLength {
                                        pattern_id: pat_id,
                                        old_length: old,
                                        new_length: new_len,
                                    });
                                }
                                ui.close();
                            }
                        });
                        ui.separator();
                        for bars_preset in [1_u32, 2, 4, 8, 16] {
                            if ui.button(format!("{bars_preset} bar(s)")).clicked() {
                                let new_len = SeqDuration(bars_preset * ticks_per_bar);
                                let mut applied: Option<SeqDuration> = None;
                                {
                                    let mut song_w = song.write();
                                    if let Some(pat) = song_w.pattern_mut(pat_id)
                                        && pat.length != new_len
                                    {
                                        applied = Some(pat.length);
                                        pat.length = new_len;
                                    }
                                }
                                if let Some(old) = applied {
                                    undo_manager.push(crate::undo::UndoAction::SetPatternLength {
                                        pattern_id: pat_id,
                                        old_length: old,
                                        new_length: new_len,
                                    });
                                }
                                ui.close();
                            }
                        }
                    });

                    if ui.button("Duplicate Pattern").clicked() {
                        {
                            let mut song_w = song.write();
                            if let Some(new_id) = song_w.duplicate_pattern(pat_id) {
                                let pattern_length = song_w
                                    .pattern(pat_id)
                                    .map_or(SeqDuration::WHOLE, |p| p.length);
                                song_w.place_pattern(
                                    new_id,
                                    trk_id,
                                    Tick(start_tick + pattern_length.0 as u64),
                                );
                            }
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Remove from Timeline").clicked() {
                        let mut captured: Option<synth_sequencer::PatternPlacement> = None;
                        {
                            let mut song_w = song.write();
                            if let Some(p) = song_w
                                .arrangement()
                                .iter()
                                .find(|p| {
                                    p.pattern_id == pat_id
                                        && p.track_id == trk_id
                                        && p.start.0 == start_tick
                                })
                                .cloned()
                            {
                                song_w.remove_placement(pat_id, trk_id, Tick(start_tick));
                                captured = Some(p);
                            }
                        }
                        if let Some(p) = captured {
                            undo_manager
                                .push(crate::undo::UndoAction::RemovePlacement { placement: p });
                        }
                        ui.close();
                    }
                    if ui
                        .button(RichText::new("Delete Pattern").color(t.colors.accent_red))
                        .clicked()
                    {
                        let mut captured: Option<(
                            synth_sequencer::Pattern,
                            Vec<synth_sequencer::PatternPlacement>,
                        )> = None;
                        {
                            let mut song_w = song.write();
                            let placements: Vec<_> = song_w
                                .arrangement()
                                .iter()
                                .filter(|p| p.pattern_id == pat_id)
                                .cloned()
                                .collect();
                            if let Some(deleted) = song_w.delete_pattern(pat_id) {
                                captured = Some((deleted, placements));
                            }
                        }
                        if let Some((pat, plcs)) = captured {
                            undo_manager.push(crate::undo::UndoAction::DeletePattern {
                                pattern: pat,
                                placements: plcs,
                            });
                        }
                        ui.close();
                    }
                } else {
                    // Clicked on empty area — figure out track + tick
                    let row_offset = hover_pos.y - (tl_y + RULER_HEIGHT);
                    let row_idx = if TRACK_ROW_HEIGHT > 0.0 {
                        (row_offset / TRACK_ROW_HEIGHT) as usize
                    } else {
                        0
                    };
                    let click_tick = x_to_tick(hover_pos.x);
                    let bar_tick = snap_tick(click_tick);

                    if row_idx < data.tracks.len() {
                        let target_track = data.tracks[row_idx].id;
                        ui.label(
                            RichText::new(format!(
                                "Bar {}",
                                bar_tick.checked_div(ticks_per_bar).map_or(1, |q| q + 1)
                            ))
                            .color(t.colors.text_dim),
                        );
                        ui.separator();

                        if ui
                            .button(format!("{} New Pattern Here", ri::ADD_LINE))
                            .clicked()
                        {
                            {
                                let mut song_w = song.write();
                                let new_pat_id = song_w.create_pattern(SeqDuration::WHOLE * 4);
                                song_w.place_pattern(new_pat_id, target_track, Tick(bar_tick));
                            }
                            ui.close();
                        }

                        // Place existing pattern submenu
                        if !data.patterns.is_empty() {
                            ui.menu_button("Place Existing Pattern", |ui| {
                                for pat in &data.patterns {
                                    let beats = pat.length_ticks as f32
                                        / synth_sequencer::TICKS_PER_QUARTER as f32;
                                    if ui
                                        .button(format!("{} ({:.0} beats)", pat.name, beats))
                                        .clicked()
                                    {
                                        song.write().place_pattern(
                                            pat.id,
                                            target_track,
                                            Tick(bar_tick),
                                        );
                                        ui.close();
                                    }
                                }
                            });
                        }
                    }
                }
            });

            // ── Loop region markers (in ruler + faint band over rows) ──
            if let (Some(loop_start), Some(loop_end)) =
                (view_state.loop_start_tick, view_state.loop_end_tick)
                && loop_end.0 > loop_start.0
            {
                let x_a = tick_to_x(loop_start.0);
                let x_b = tick_to_x(loop_end.0);
                let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
                let band_fill = Color32::from_rgba_unmultiplied(
                    LOOP_COLOR.r(),
                    LOOP_COLOR.g(),
                    LOOP_COLOR.b(),
                    24,
                );

                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(x_a, tl_y + RULER_HEIGHT),
                        Pos2::new(x_b, line_bottom),
                    ),
                    0.0,
                    band_fill,
                );

                for (x, dx) in [(x_a, 6.0), (x_b, -6.0)] {
                    painter.line_segment(
                        [Pos2::new(x, tl_y), Pos2::new(x, tl_y + RULER_HEIGHT)],
                        Stroke::new(2.0, LOOP_COLOR),
                    );
                    painter.line_segment(
                        [Pos2::new(x, tl_y + 4.0), Pos2::new(x + dx, tl_y + 4.0)],
                        Stroke::new(2.0, LOOP_COLOR),
                    );
                }
            }

            // ── Tempo change markers on the ruler ──
            if !data.tempo_changes.is_empty() {
                let tempo_color = TEMPO_MARKER;
                for (tick, bpm) in &data.tempo_changes {
                    let x = tick_to_x(*tick);
                    // Small flag: vertical tick + label "120.0".
                    painter.line_segment(
                        [Pos2::new(x, tl_y + 1.0), Pos2::new(x, tl_y + RULER_HEIGHT)],
                        Stroke::new(1.5, tempo_color),
                    );
                    painter.text(
                        Pos2::new(x + 2.0, tl_y + 12.0),
                        egui::Align2::LEFT_TOP,
                        format!("{bpm:.0}"),
                        egui::FontId::proportional(9.0),
                        tempo_color,
                    );
                }
            }

            // ── Playhead ──
            // The play-start / return position (the "cursor") is tracked by the
            // engine and drives Play / Stop, but it is intentionally not drawn
            // as a separate marker: while paused it sits behind the playhead and
            // reads as a misaligned "ghost". Stop simply snaps the playhead back
            // to it, the standard DAW behavior.
            let line_bottom = tl_y + RULER_HEIGHT + track_count as f32 * TRACK_ROW_HEIGHT;
            if current_tick > 0 || data.song_end_tick > 0 {
                let playhead_x = tick_to_x(current_tick);
                let line_top = tl_y;

                painter.line_segment(
                    [
                        Pos2::new(playhead_x, line_top),
                        Pos2::new(playhead_x, line_bottom),
                    ],
                    Stroke::new(2.0, t.colors.accent_primary),
                );

                let tri_size = 6.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(playhead_x - tri_size, line_top),
                        Pos2::new(playhead_x + tri_size, line_top),
                        Pos2::new(playhead_x, line_top + RULER_HEIGHT * 0.6),
                    ],
                    t.colors.accent_primary,
                    Stroke::NONE,
                ));
            }

            // Ruler bottom border
            painter.line_segment(
                [
                    Pos2::new(tl_x, tl_y + RULER_HEIGHT),
                    Pos2::new(tl_x + timeline_width, tl_y + RULER_HEIGHT),
                ],
                Stroke::new(1.0, t.colors.border),
            );
        });

    // Detect manual scrolling to disable auto-follow
    if is_playing {
        let actual_offset = scroll_output.state.offset.x;
        if let Some(expected) = view_state.last_auto_scroll_offset {
            // If user scrolled manually (offset differs significantly from what we set)
            if (actual_offset - expected).abs() > 2.0 {
                view_state.auto_follow_playhead = false;
                view_state.last_auto_scroll_offset = None;
            }
        }
    }

    // Re-enable auto-follow when playback starts from stopped
    if !is_playing {
        view_state.last_auto_scroll_offset = None;
    }

    double_clicked_pattern
}

// ============================================================================
// PIANO ROLL
// ============================================================================

/// Snapshot of a note for piano roll rendering.
struct PianoRollNote {
    note_id: NoteId,
    pitch: Pitch,
    start_tick: PatternTick,
    end_tick: Option<PatternTick>,
    velocity: Velocity,
    legato: bool,
    glide: Option<Glide>,
    expression: Option<NoteExpression>,
    /// Stored voice/column lane (the tracker's source of truth for which voice
    /// column a note lives in). The piano roll ignores it.
    lane: NoteLane,
}

/// Snapshot of a single automation point for rendering.
struct AutomationPointSnapshot {
    tick: PatternTick,
    value: NormalizedValue,
    curve: CurveType,
}

/// Snapshot of an automation lane for rendering.
struct AutomationLaneSnapshot {
    target: AutomationTarget,
    points: Vec<AutomationPointSnapshot>,
}

/// Collected data for piano roll rendering.
pub(crate) struct PianoRollData {
    pattern_name: String,
    pattern_description: String,
    pattern_id: PatternId,
    length_ticks: SeqDuration,
    ticks_per_row: u16,
    notes: Vec<PianoRollNote>,
    pitch_min: Pitch,
    pitch_max: Pitch,
    automation_lanes: Vec<AutomationLaneSnapshot>,
    time_sig: TimeSignature,
    /// Distinct track-level instrument overrides currently affecting this
    /// pattern (collected from every placement of the pattern). Empty when
    /// no host track has `track.instrument` set — in that case the
    /// per-note instrument is used at playback.
    track_overrides: Vec<SeqInstrumentId>,
}

/// Collect piano roll data from song (short read-lock, then release).
pub(crate) fn collect_piano_roll_data(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
) -> Option<PianoRollData> {
    let song = song.try_read()?;
    let pattern = song.pattern(pattern_id)?;

    let mut pitch_min = Pitch::MAX;
    let mut pitch_max = Pitch::MIN;

    let notes: Vec<PianoRollNote> = pattern
        .notes()
        .iter()
        .map(|n| {
            if n.pitch < pitch_min {
                pitch_min = n.pitch;
            }
            if n.pitch > pitch_max {
                pitch_max = n.pitch;
            }
            PianoRollNote {
                note_id: n.id,
                pitch: n.pitch,
                start_tick: n.start,
                end_tick: n.end(),
                velocity: n.velocity,
                legato: n.legato,
                glide: n.glide,
                expression: n.expression,
                lane: n.lane,
            }
        })
        .collect();

    // Default range if no notes
    if pitch_min > pitch_max {
        pitch_min = Pitch::new(48).unwrap_or(Pitch::MIDDLE_C); // C3
        pitch_max = Pitch::new(72).unwrap_or(Pitch::MIDDLE_C); // C5
    }

    let automation_lanes: Vec<AutomationLaneSnapshot> = pattern
        .automation
        .iter()
        .map(|lane| AutomationLaneSnapshot {
            target: lane.target.clone(),
            points: lane
                .points()
                .iter()
                .map(|p| AutomationPointSnapshot {
                    tick: p.tick,
                    value: p.value,
                    curve: p.curve,
                })
                .collect(),
        })
        .collect();

    let time_sig = song.default_time_signature;

    // Collect the distinct instruments this pattern's placements route through
    // (one per track). Used to show which instrument(s) actually play the
    // pattern. (Per-note instrument is no longer consulted at playback.)
    let mut track_overrides: Vec<SeqInstrumentId> = Vec::new();
    for placement in song.arrangement() {
        if placement.pattern_id == pattern_id
            && let Some(track) = song.track(placement.track_id)
            && !track_overrides.contains(&track.instrument)
        {
            track_overrides.push(track.instrument);
        }
    }

    Some(PianoRollData {
        pattern_name: if pattern.name.is_empty() {
            format!("Pattern {}", pattern_id.0)
        } else {
            pattern.name.clone()
        },
        pattern_description: pattern.description.clone(),
        pattern_id,
        length_ticks: pattern.length,
        ticks_per_row: song.row_resolution.ticks_per_row.as_u16(),
        notes,
        pitch_min,
        pitch_max,
        automation_lanes,
        time_sig,
        track_overrides,
    })
}

/// Find the note at the given position, returning its ID and which zone was hit.
#[allow(clippy::too_many_arguments)]
fn note_at_pos(
    notes: &[PianoRollNote],
    pos: Pos2,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    pitch_to_y: &dyn Fn(Pitch) -> f32,
    length_ticks: SeqDuration,
    view_pitch_min: Pitch,
    view_pitch_max: Pitch,
    note_row_height: f32,
) -> Option<(NoteId, HitZone)> {
    // Iterate in reverse so top-most (last drawn) notes are checked first
    for note in notes.iter().rev() {
        if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
            continue;
        }

        let y = pitch_to_y(note.pitch);
        let x_start = tick_to_x(note.start_tick);
        let x_end = match note.end_tick {
            Some(end) => tick_to_x(end),
            None => tick_to_x(length_ticks.as_pattern_tick()),
        };
        let note_width = (x_end - x_start).max(3.0);

        let note_rect = Rect::from_min_size(
            Pos2::new(x_start, y + 1.0),
            Vec2::new(note_width, note_row_height - 2.0),
        );

        // Expand tiny notes so they're easier to click
        let hit_rect = if note_width < RESIZE_GRAB_ZONE {
            note_rect.expand2(Vec2::new(2.0, 1.0))
        } else {
            note_rect
        };

        if hit_rect.contains(pos) {
            // Proportional grab zone: at most 30% of note width, so short notes remain movable
            let grab_zone = RESIZE_GRAB_ZONE.min(note_width * 0.3);
            let zone = if pos.x >= note_rect.max.x - grab_zone {
                HitZone::RightEdge
            } else {
                HitZone::Body
            };
            return Some((note.note_id, zone));
        }
    }
    None
}

/// Check if a note already exists at the given tick and pitch.
fn has_note_at(notes: &[PianoRollNote], tick: PatternTick, pitch: Pitch) -> bool {
    notes.iter().any(|n| {
        n.pitch == pitch
            && n.start_tick <= tick
            && n.end_tick.unwrap_or(PatternTick(u32::MAX)) > tick
    })
}

/// Floor-snap a tick to a multiple of `step`. `step == 0` is a no-op.
fn snap_to_step(tick: u64, step: u64) -> u64 {
    tick.checked_div(step).map_or(tick, |q| q * step)
}

/// Quantize a `PatternTick` to a row boundary using the row's tick width.
fn quantize_tick(tick: PatternTick, ticks_per_row: u16) -> PatternTick {
    PatternTick(snap_to_step(tick.0 as u64, ticks_per_row as u64) as u32)
}

/// Selection inspector row: shows pitch / start / length of the selected notes
/// (or "—" when they differ) and an editable velocity DragValue that batches an
/// undo entry on drag/focus release.
/// Default per-note vibrato installed when the inspector's Vibrato toggle is enabled.
fn default_inspector_vibrato() -> Vibrato {
    Vibrato {
        depth: Semitones::new(0.3),
        rate: Hertz::new(5.5),
        delay: Milliseconds::new(0.0),
        shape: VibratoShape::Sine,
    }
}

/// Apply a per-note expression edit to the selected notes (preserving each
/// note's *other* fields) and push one `SetExpressionBatch` undo entry. For
/// discrete (toggle) edits.
fn apply_expression_edit(
    song: &Arc<RwLock<Song>>,
    undo: &mut crate::undo::UndoManager,
    pid: PatternId,
    selected: &[&PianoRollNote],
    edit: impl Fn(&mut NoteExpression),
) {
    let mut changes: Vec<(NoteId, Option<NoteExpression>, Option<NoteExpression>)> = Vec::new();
    {
        let mut song_w = song.write();
        if let Some(pattern) = song_w.pattern_mut(pid) {
            for n in selected {
                let old = n.expression;
                let mut e = old.unwrap_or_default();
                edit(&mut e);
                // Collapse an all-default block back to None (no pointless storage/dot).
                let new = e.normalized();
                if new != old {
                    pattern.set_note_expression(n.note_id, new);
                    changes.push((n.note_id, old, new));
                }
            }
        }
    }
    if !changes.is_empty() {
        undo.push(crate::undo::UndoAction::SetExpressionBatch {
            pattern_id: pid,
            changes,
        });
    }
}

/// Live-apply a per-note expression edit (no undo) — used while an expression
/// DragValue is dragging; the drag collapses into one undo entry on release via
/// [`finish_expression_drag`]. Preserves each note's other fields.
fn live_expression_edit(
    song: &Arc<RwLock<Song>>,
    pid: PatternId,
    selected: &[&PianoRollNote],
    edit: impl Fn(&mut NoteExpression),
) {
    let mut song_w = song.write();
    if let Some(pattern) = song_w.pattern_mut(pid) {
        for n in selected {
            let mut e = n.expression.unwrap_or_default();
            edit(&mut e);
            // Collapse an all-default block back to None (no pointless storage/dot).
            pattern.set_note_expression(n.note_id, e.normalized());
        }
    }
}

/// On expression-DragValue release, diff the pre-drag snapshot against the now
/// current pattern state and push one `SetExpressionBatch` undo entry.
fn finish_expression_drag(
    song: &Arc<RwLock<Song>>,
    undo: &mut crate::undo::UndoManager,
    pid: PatternId,
    before: Vec<(NoteId, Option<NoteExpression>)>,
) {
    let mut changes = Vec::new();
    {
        let song_r = song.read();
        if let Some(pattern) = song_r.pattern(pid) {
            for (id, old) in before {
                let new = pattern.note(id).and_then(|n| n.expression);
                if new != old {
                    changes.push((id, old, new));
                }
            }
        }
    }
    if !changes.is_empty() {
        undo.push(crate::undo::UndoAction::SetExpressionBatch {
            pattern_id: pid,
            changes,
        });
    }
}

fn draw_piano_roll_selection_inspector(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    view_state: &mut SequencerViewState,
    song: &Arc<RwLock<Song>>,
    undo_manager: &mut crate::undo::UndoManager,
) {
    let t = theme();
    if !view_state.selected_notes.is_empty() {
        let selected: Vec<&PianoRollNote> = data
            .notes
            .iter()
            .filter(|n| view_state.selected_notes.contains(&n.note_id))
            .collect();
        if !selected.is_empty() {
            let first = selected[0];
            let pitches_equal = selected.iter().all(|n| n.pitch == first.pitch);
            let starts_equal = selected.iter().all(|n| n.start_tick == first.start_tick);
            let durations: Vec<Option<u32>> = selected
                .iter()
                .map(|n| n.end_tick.map(|e| e.0 - n.start_tick.0))
                .collect();
            let lengths_equal = durations.iter().all(|d| *d == durations[0]);
            let velocities_equal = selected
                .iter()
                .all(|n| (n.velocity.as_f32() - first.velocity.as_f32()).abs() < f32::EPSILON);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    RichText::new(format!("Selection ({})", selected.len()))
                        .color(t.colors.accent_yellow)
                        .strong(),
                );
                ui.separator();

                // Pitch
                ui.label(RichText::new("Pitch").color(t.colors.text_dim).size(10.0));
                let pitch_text = if pitches_equal {
                    let midi = first.pitch.as_midi();
                    let name = NoteName::from_midi(midi % 12);
                    let octave = (midi / 12) as i8 - 1;
                    format!("{name:?}{octave}")
                } else {
                    "—".to_owned()
                };
                ui.label(RichText::new(pitch_text).color(t.colors.text_primary));

                // Start
                ui.label(RichText::new("Start").color(t.colors.text_dim).size(10.0));
                let start_text = if starts_equal {
                    let beats =
                        first.start_tick.0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
                    format!("{beats:.2} beats")
                } else {
                    "—".to_owned()
                };
                ui.label(RichText::new(start_text).color(t.colors.text_primary));

                // Length
                ui.label(RichText::new("Len").color(t.colors.text_dim).size(10.0));
                let length_text = if lengths_equal {
                    match durations[0] {
                        Some(d) => {
                            let beats = d as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
                            format!("{beats:.2} beats")
                        }
                        None => "open".to_owned(),
                    }
                } else {
                    "—".to_owned()
                };
                ui.label(RichText::new(length_text).color(t.colors.text_primary));

                ui.label(RichText::new("Vel").color(t.colors.text_dim).size(10.0));
                let mut vel_pct = if velocities_equal {
                    (first.velocity.as_f32() * 100.0).round()
                } else {
                    50.0
                };
                let vel_resp = ui
                    .add(
                        egui::DragValue::new(&mut vel_pct)
                            .range(1.0..=100.0)
                            .speed(1.0)
                            .suffix(" %"),
                    )
                    .on_hover_text(if velocities_equal {
                        "Velocity of selected notes"
                    } else {
                        "Velocities differ — drag to set all to the same value"
                    });
                if vel_resp.drag_started() || vel_resp.gained_focus() {
                    view_state.inspector_vel_drag_start = Some((
                        data.pattern_id,
                        selected.iter().map(|n| (n.note_id, n.velocity)).collect(),
                    ));
                }
                if vel_resp.changed() {
                    let new_vel = Velocity::new((vel_pct / 100.0).clamp(0.01, 1.0));
                    let pid = data.pattern_id;
                    let ids: Vec<NoteId> = selected.iter().map(|n| n.note_id).collect();
                    let mut song_w = song.write();
                    if let Some(pattern) = song_w.pattern_mut(pid) {
                        for nid in ids {
                            pattern.set_note_velocity(nid, new_vel);
                        }
                    }
                }
                if (vel_resp.drag_stopped() || vel_resp.lost_focus())
                    && let Some((pid, before)) = view_state.inspector_vel_drag_start.take()
                    && pid == data.pattern_id
                {
                    let new_vel = Velocity::new((vel_pct / 100.0).clamp(0.01, 1.0));
                    let changes: Vec<(NoteId, Velocity, Velocity)> = before
                        .into_iter()
                        .filter_map(|(id, old)| (old != new_vel).then_some((id, old, new_vel)))
                        .collect();
                    if !changes.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::SetVelocitiesBatch {
                            pattern_id: data.pattern_id,
                            changes,
                        });
                    }
                }
            });

            // ── Per-note expression: tie/legato + glide (taxonomy primitive 2) ──
            let pid = data.pattern_id;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                // Tie / legato — discrete toggle, one undo entry per click.
                let mut legato = selected.iter().all(|n| n.legato);
                if ui
                    .checkbox(&mut legato, "Tie")
                    .on_hover_text("Legato: connect to the next note without retriggering")
                    .changed()
                {
                    let changes: Vec<(NoteId, bool, bool)> = selected
                        .iter()
                        .filter(|n| n.legato != legato)
                        .map(|n| (n.note_id, n.legato, legato))
                        .collect();
                    if !changes.is_empty() {
                        {
                            let mut song_w = song.write();
                            if let Some(pattern) = song_w.pattern_mut(pid) {
                                for (nid, _, new) in &changes {
                                    pattern.set_note_legato(*nid, *new);
                                }
                            }
                        }
                        undo_manager.push(crate::undo::UndoAction::SetLegatoBatch {
                            pattern_id: pid,
                            changes,
                        });
                    }
                }

                ui.separator();

                // Glide enable — discrete toggle; enabling installs a sensible default.
                let default_glide = Glide {
                    from: GlideFrom::Semitones(Semitones::new(-2.0)),
                    time: Milliseconds::new(100.0),
                    interp: GlideInterp::Continuous,
                };
                let mut glide_on =
                    !selected.is_empty() && selected.iter().all(|n| n.glide.is_some());
                if ui
                    .checkbox(&mut glide_on, "Glide")
                    .on_hover_text("Portamento/glissando into this note")
                    .changed()
                {
                    let new_glide = glide_on.then_some(default_glide);
                    let changes: Vec<(NoteId, Option<Glide>, Option<Glide>)> = selected
                        .iter()
                        .filter(|n| n.glide != new_glide)
                        .map(|n| (n.note_id, n.glide, new_glide))
                        .collect();
                    if !changes.is_empty() {
                        {
                            let mut song_w = song.write();
                            if let Some(pattern) = song_w.pattern_mut(pid) {
                                for (nid, _, new) in &changes {
                                    pattern.set_note_glide(*nid, *new);
                                }
                            }
                        }
                        undo_manager.push(crate::undo::UndoAction::SetGlideBatch {
                            pattern_id: pid,
                            changes,
                        });
                    }
                }

                // Glide parameters — shown when at least one selected note glides.
                if let Some(cur) = selected.iter().find_map(|n| n.glide) {
                    // The inspector expresses the source as a relative offset; an
                    // absolute Pitch source (MCP-set) collapses to the default.
                    let mut from_semis = match cur.from {
                        GlideFrom::Semitones(s) => s.as_f32(),
                        GlideFrom::Pitch(_) => -2.0,
                    };
                    let mut time_ms = cur.time.as_f32();
                    let mut stepped = matches!(cur.interp, GlideInterp::Stepped);

                    ui.label(RichText::new("From").color(t.colors.text_dim).size(10.0));
                    let from_resp = ui
                        .add(
                            egui::DragValue::new(&mut from_semis)
                                .range(-24.0..=24.0)
                                .speed(0.5)
                                .suffix(" st"),
                        )
                        .on_hover_text("Glide source, semitones relative to this note");

                    ui.label(RichText::new("Time").color(t.colors.text_dim).size(10.0));
                    let time_resp = ui
                        .add(
                            egui::DragValue::new(&mut time_ms)
                                .range(0.0..=2000.0)
                                .speed(2.0)
                                .suffix(" ms"),
                        )
                        .on_hover_text("Glide time");

                    let make = |from_semis: f32, time_ms: f32, stepped: bool| Glide {
                        from: GlideFrom::Semitones(Semitones::new(from_semis)),
                        time: Milliseconds::new(time_ms),
                        interp: if stepped {
                            GlideInterp::Stepped
                        } else {
                            GlideInterp::Continuous
                        },
                    };

                    // Capture pre-drag glide once, collapse the whole drag into one
                    // undo entry on release (mirrors the velocity DragValue).
                    if from_resp.drag_started()
                        || from_resp.gained_focus()
                        || time_resp.drag_started()
                        || time_resp.gained_focus()
                    {
                        view_state.inspector_glide_drag_start =
                            Some((pid, selected.iter().map(|n| (n.note_id, n.glide)).collect()));
                    }
                    // From/Time edits only touch notes that *already* glide — they
                    // never force glide onto a non-gliding note in a mixed selection
                    // (use the Glide checkbox for that). Multi-edit shares the one
                    // representative value, like the velocity inspector.
                    if from_resp.changed() || time_resp.changed() {
                        let g = make(from_semis, time_ms, stepped);
                        let mut song_w = song.write();
                        if let Some(pattern) = song_w.pattern_mut(pid) {
                            for n in selected.iter().filter(|n| n.glide.is_some()) {
                                pattern.set_note_glide(n.note_id, Some(g));
                            }
                        }
                    }
                    if (from_resp.drag_stopped()
                        || from_resp.lost_focus()
                        || time_resp.drag_stopped()
                        || time_resp.lost_focus())
                        && let Some((dpid, before)) = view_state.inspector_glide_drag_start.take()
                        && dpid == pid
                    {
                        let g = make(from_semis, time_ms, stepped);
                        let changes: Vec<(NoteId, Option<Glide>, Option<Glide>)> = before
                            .into_iter()
                            .filter_map(|(id, old)| {
                                // Only notes that already glided were edited above.
                                (old.is_some() && old != Some(g)).then_some((id, old, Some(g)))
                            })
                            .collect();
                        if !changes.is_empty() {
                            undo_manager.push(crate::undo::UndoAction::SetGlideBatch {
                                pattern_id: pid,
                                changes,
                            });
                        }
                    }

                    // Stepped (glissando) toggle — discrete, one undo entry.
                    if ui
                        .checkbox(&mut stepped, "Stepped")
                        .on_hover_text(
                            "Glissando: hold at chromatic steps instead of a smooth ramp",
                        )
                        .changed()
                    {
                        let g = make(from_semis, time_ms, stepped);
                        let changes: Vec<(NoteId, Option<Glide>, Option<Glide>)> = selected
                            .iter()
                            .filter(|n| n.glide.is_some() && n.glide != Some(g))
                            .map(|n| (n.note_id, n.glide, Some(g)))
                            .collect();
                        if !changes.is_empty() {
                            {
                                let mut song_w = song.write();
                                if let Some(pattern) = song_w.pattern_mut(pid) {
                                    for (nid, _, new) in &changes {
                                        pattern.set_note_glide(*nid, *new);
                                    }
                                }
                            }
                            undo_manager.push(crate::undo::UndoAction::SetGlideBatch {
                                pattern_id: pid,
                                changes,
                            });
                        }
                    }
                }
            });

            // ── Per-note expression block: accent / gate / ghost / probability ──
            // (taxonomy primitive 3 note-shape scalars + primitive 1 vibrato).
            // Each control edits only its own field, preserving the others per note.
            // Multi-edit semantics match the velocity inspector: controls display
            // the first selected note's value and an edit writes that field to all
            // selected (it does not preserve per-note variation of the edited
            // field — only of the *other* fields).
            let cur = selected
                .iter()
                .find_map(|n| n.expression)
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                // Accent (velocity ×). DragValue → live edit + one undo on release.
                ui.label(RichText::new("Accent").color(t.colors.text_dim).size(10.0));
                let mut accent = cur.accent.unwrap_or(1.0);
                let r = ui
                    .add(
                        egui::DragValue::new(&mut accent)
                            .range(0.0..=4.0)
                            .speed(0.02)
                            .fixed_decimals(2),
                    )
                    .on_hover_text("Velocity multiplier (1.0 = unchanged)");
                if r.drag_started() || r.gained_focus() {
                    view_state.inspector_expr_drag_start = Some((
                        pid,
                        selected.iter().map(|n| (n.note_id, n.expression)).collect(),
                    ));
                }
                if r.changed() {
                    // Returning to the neutral value (1.0×) clears the field so the
                    // block can collapse to None (no lingering expression dot).
                    let accent_field = (accent != 1.0).then_some(accent);
                    live_expression_edit(song, pid, &selected, |e| e.accent = accent_field);
                }
                if (r.drag_stopped() || r.lost_focus())
                    && let Some((dpid, before)) = view_state.inspector_expr_drag_start.take()
                    && dpid == pid
                {
                    finish_expression_drag(song, undo_manager, pid, before);
                }

                // Gate (% of duration, staccato/tenuto).
                ui.label(RichText::new("Gate").color(t.colors.text_dim).size(10.0));
                let mut gate_pct = cur.gate.map_or(100.0, |g| g.as_f32() * 100.0);
                let r = ui
                    .add(
                        egui::DragValue::new(&mut gate_pct)
                            .range(1.0..=100.0)
                            .speed(1.0)
                            .suffix(" %"),
                    )
                    .on_hover_text("Note length as a % of its duration (staccato)");
                if r.drag_started() || r.gained_focus() {
                    view_state.inspector_expr_drag_start = Some((
                        pid,
                        selected.iter().map(|n| (n.note_id, n.expression)).collect(),
                    ));
                }
                if r.changed() {
                    // Neutral gate (100% = full duration) clears the field.
                    let gate_field = (gate_pct < 100.0)
                        .then(|| NormalizedValue::new((gate_pct / 100.0).clamp(0.0, 1.0)));
                    live_expression_edit(song, pid, &selected, |e| e.gate = gate_field);
                }
                if (r.drag_stopped() || r.lost_focus())
                    && let Some((dpid, before)) = view_state.inspector_expr_drag_start.take()
                    && dpid == pid
                {
                    finish_expression_drag(song, undo_manager, pid, before);
                }

                // Probability (% chance to play).
                ui.label(RichText::new("Prob").color(t.colors.text_dim).size(10.0));
                let mut prob_pct = cur.probability.map_or(100.0, |p| p.as_f32() * 100.0);
                let r = ui
                    .add(
                        egui::DragValue::new(&mut prob_pct)
                            .range(0.0..=100.0)
                            .speed(1.0)
                            .suffix(" %"),
                    )
                    .on_hover_text("Chance this note plays (resolved at playback)");
                if r.drag_started() || r.gained_focus() {
                    view_state.inspector_expr_drag_start = Some((
                        pid,
                        selected.iter().map(|n| (n.note_id, n.expression)).collect(),
                    ));
                }
                if r.changed() {
                    // Neutral probability (100% = always plays) clears the field.
                    let prob_field = (prob_pct < 100.0)
                        .then(|| NormalizedValue::new((prob_pct / 100.0).clamp(0.0, 1.0)));
                    live_expression_edit(song, pid, &selected, |e| e.probability = prob_field);
                }
                if (r.drag_stopped() || r.lost_focus())
                    && let Some((dpid, before)) = view_state.inspector_expr_drag_start.take()
                    && dpid == pid
                {
                    finish_expression_drag(song, undo_manager, pid, before);
                }

                // Ghost (forced-soft) — discrete toggle.
                let mut ghost = cur.ghost;
                if ui
                    .checkbox(&mut ghost, "Ghost")
                    .on_hover_text("Force a soft (ghost) velocity")
                    .changed()
                {
                    apply_expression_edit(song, undo_manager, pid, &selected, |e| e.ghost = ghost);
                }
            });

            // ── Vibrato mini-control (taxonomy primitive 1): enable + depth/rate ──
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let mut vib_on = cur.vibrato.is_some();
                if ui
                    .checkbox(&mut vib_on, "Vibrato")
                    .on_hover_text("Per-note pitch vibrato")
                    .changed()
                {
                    let v = vib_on.then(default_inspector_vibrato);
                    apply_expression_edit(song, undo_manager, pid, &selected, |e| e.vibrato = v);
                }
                if let Some(v) = cur.vibrato {
                    ui.label(RichText::new("Depth").color(t.colors.text_dim).size(10.0));
                    let mut depth = v.depth.as_f32();
                    let rd = ui
                        .add(
                            egui::DragValue::new(&mut depth)
                                .range(0.0..=2.0)
                                .speed(0.01)
                                .suffix(" st"),
                        )
                        .on_hover_text("Vibrato depth (semitones)");
                    ui.label(RichText::new("Rate").color(t.colors.text_dim).size(10.0));
                    let mut rate = v.rate.as_f32();
                    let rr = ui
                        .add(
                            egui::DragValue::new(&mut rate)
                                .range(0.1..=20.0)
                                .speed(0.1)
                                .suffix(" Hz"),
                        )
                        .on_hover_text("Vibrato rate");
                    if rd.drag_started()
                        || rd.gained_focus()
                        || rr.drag_started()
                        || rr.gained_focus()
                    {
                        view_state.inspector_expr_drag_start = Some((
                            pid,
                            selected.iter().map(|n| (n.note_id, n.expression)).collect(),
                        ));
                    }
                    if rd.changed() || rr.changed() {
                        // Only notes that already have vibrato get depth/rate edits.
                        let mut song_w = song.write();
                        if let Some(pattern) = song_w.pattern_mut(pid) {
                            for n in selected
                                .iter()
                                .filter(|n| n.expression.is_some_and(|e| e.vibrato.is_some()))
                            {
                                let mut e = n.expression.unwrap_or_default();
                                if let Some(vib) = e.vibrato.as_mut() {
                                    vib.depth = Semitones::new(depth);
                                    vib.rate = Hertz::new(rate);
                                }
                                pattern.set_note_expression(n.note_id, Some(e));
                            }
                        }
                    }
                    if (rd.drag_stopped()
                        || rd.lost_focus()
                        || rr.drag_stopped()
                        || rr.lost_focus())
                        && let Some((dpid, before)) = view_state.inspector_expr_drag_start.take()
                        && dpid == pid
                    {
                        finish_expression_drag(song, undo_manager, pid, before);
                    }
                }
            });
            ui.separator();
        }
    }
}

/// Shared automation-target selector ComboBox (labelled "Auto:"). Sets
/// `view_state.selected_automation`. Lists existing lanes with points (badged when
/// they belong to another instrument), every instrument param, and the automatable
/// module params of the selected instrument. Used by both the piano-roll toolbar and
/// the tracker's "add automation column" control.
pub(crate) fn draw_automation_target_selector(
    ui: &mut egui::Ui,
    view_state: &mut SequencerViewState,
    data: &PianoRollData,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
) {
    ui.label(RichText::new("Auto:").color(theme().colors.text_dim));
    // Build label for ComboBox
    let auto_label = view_state
        .selected_automation
        .as_ref()
        .map_or_else(|| "None".to_owned(), AutomationTarget::display_name);

    egui::ComboBox::from_id_salt("auto_lane_select")
        .selected_text(&auto_label)
        .width(160.0)
        .show_ui(ui, |ui| {
            // "None" option to hide automation zone
            if ui
                .selectable_label(view_state.selected_automation.is_none(), "None")
                .clicked()
            {
                view_state.selected_automation = None;
            }

            // 1. Existing lanes with points (any instrument), with a
            //    badge when they belong to a different instrument.
            let mut shown_any_existing = false;
            for lane in &data.automation_lanes {
                if lane.points.is_empty() {
                    continue;
                }
                let target = &lane.target;
                let is_foreign = match target {
                    AutomationTarget::Instrument { instrument, .. }
                    | AutomationTarget::Module { instrument, .. } => {
                        *instrument != view_state.selected_instrument
                    }
                    _ => false,
                };
                let inst_name = match target {
                    AutomationTarget::Instrument { instrument, .. }
                    | AutomationTarget::Module { instrument, .. } => instruments
                        .iter()
                        .find(|inst| inst.id.0 == instrument.0 as u64)
                        .map(|inst| inst.name.clone()),
                    _ => None,
                };
                let base = target.display_name();
                let arrow = egui_remixicon::icons::ARROW_RIGHT_S_LINE;
                let label = if is_foreign {
                    match inst_name {
                        Some(name) => format!("* {base}  {arrow} {name}"),
                        None => format!("* {base}"),
                    }
                } else {
                    format!("* {base}")
                };
                let is_selected = view_state.selected_automation.as_ref() == Some(target);
                if ui.selectable_label(is_selected, &label).clicked() {
                    view_state.selected_automation = Some(target.clone());
                }
                shown_any_existing = true;
            }
            if shown_any_existing {
                ui.separator();
            }

            // 2. All instrument params for the currently selected
            //    instrument (empty + with-points alike), so the user
            //    can create new lanes.
            for param in AutoInstrumentParam::ALL {
                let target = AutomationTarget::Instrument {
                    instrument: view_state.selected_instrument,
                    param: *param,
                };
                let already_shown = data
                    .automation_lanes
                    .iter()
                    .any(|l| l.target == target && !l.points.is_empty());
                if already_shown {
                    continue;
                }
                let label = param.display_name().to_owned();
                let is_selected = view_state.selected_automation.as_ref() == Some(&target);
                if ui.selectable_label(is_selected, &label).clicked() {
                    view_state.selected_automation = Some(target);
                }
            }

            // 3. Per-module parameters of the selected instrument
            //    (generic A2 targets), filtered to the automatable
            //    allowlist. Lets the user automate any continuous,
            //    RT-safe module parameter, not just the fixed
            //    instrument-level set above.
            if let Some(inst) = instruments
                .iter()
                .find(|i| i.id.0 == view_state.selected_instrument.0 as u64)
            {
                let mut module_ids = inst.patch_editor.module_ids();
                module_ids.sort_unstable(); // deterministic (type, instance) order
                let mut shown_module_header = false;
                for module_id in module_ids {
                    let Some(desc) = inst.patch_editor.module_descriptor(module_id) else {
                        continue;
                    };
                    for param in &desc.parameters {
                        if !param.is_automatable() {
                            continue;
                        }
                        let target = AutomationTarget::Module {
                            instrument: view_state.selected_instrument,
                            module_type: module_id.module_type,
                            instance: module_id.instance,
                            param_id: param.type_id.as_str().into(),
                        };
                        // Skip params already shown as an existing lane above.
                        let already_shown = data
                            .automation_lanes
                            .iter()
                            .any(|l| l.target == target && !l.points.is_empty());
                        if already_shown {
                            continue;
                        }
                        if !shown_module_header {
                            ui.separator();
                            shown_module_header = true;
                        }
                        let label =
                            format!("{} {} · {}", desc.name, module_id.instance, param.name);
                        let is_selected = view_state.selected_automation.as_ref() == Some(&target);
                        if ui.selectable_label(is_selected, &label).clicked() {
                            view_state.selected_automation = Some(target);
                        }
                    }
                }
            }
        });
}

/// Shared pattern-editor controls rendered inline into the caller's toolbar row:
/// the working-instrument selector, the "track plays" badge, and the mini-transport
/// (play/pause, stop, record arm/disarm, pattern-solo). Both the piano roll and the
/// tracker call this so they show the same row without duplicating the
/// recording-arm logic.
fn draw_pattern_instrument_transport(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    is_playing: bool,
) {
    let t = theme();

    // Instrument selector for new notes
    {
        let selected_label = instruments
            .iter()
            .find(|inst| inst.id.0 == view_state.selected_instrument.0 as u64)
            .map_or_else(|| "---".to_owned(), |inst| inst.name.clone());
        egui::ComboBox::from_id_salt(ui.id().with("piano_roll_instrument"))
            .selected_text(RichText::new(&selected_label).size(12.0))
            .width(100.0)
            .show_ui(ui, |ui| {
                for inst in instruments {
                    let seq_id = SeqInstrumentId::new(inst.id.0 as u16);
                    let selected = view_state.selected_instrument == seq_id;
                    if ui.selectable_label(selected, &inst.name).clicked() {
                        view_state.selected_instrument = seq_id;
                    }
                }
            });

        // Badge: which instrument(s) the pattern's placements actually play
        // through. The selector above is only the working instrument (note
        // colour + preview); playback always routes via the track.
        if !data.track_overrides.is_empty() {
            let names: Vec<String> = data
                .track_overrides
                .iter()
                .map(|seq_id| {
                    instruments
                        .iter()
                        .find(|inst| inst.id.0 == seq_id.0 as u64)
                        .map_or_else(|| format!("#{}", seq_id.0), |inst| inst.name.clone())
                })
                .collect();
            let arrow = egui_remixicon::icons::ARROW_RIGHT_S_LINE;
            let badge_text = if names.len() == 1 {
                format!("{arrow} track plays: {}", names[0])
            } else {
                format!("{arrow} track plays: {}", names.join(", "))
            };
            ui.label(
                RichText::new(badge_text)
                    .size(10.0)
                    .color(t.colors.accent_yellow),
            )
            .on_hover_text(
                "Instrument(s) this pattern plays through where it is placed \
                    (set per track). The selector above is the working instrument \
                    for note colour and preview only.",
            );
        }
    }
    ui.separator();

    // Mini-transport controls
    {
        use egui_remixicon::icons as ri;
        ui.spacing_mut().item_spacing.x = 2.0;

        if is_playing {
            // Pause button
            if ui
                .button(
                    RichText::new(ri::PAUSE_FILL)
                        .size(12.0)
                        .color(t.colors.accent_yellow),
                )
                .on_hover_text("Pause")
                .clicked()
            {
                handle.send(EngineCommand::Pause);
            }
        } else {
            // Play pattern button — always loops the open pattern.
            // Pattern-solo toggle controls whether other tracks/patterns
            // are silenced during this playback.
            let play_tooltip = if view_state.pattern_solo {
                "Play pattern (solo — other tracks silent)"
            } else {
                "Play pattern (with other tracks)"
            };
            if ui
                .button(
                    RichText::new(ri::PLAY_FILL)
                        .size(12.0)
                        .color(t.colors.accent_green),
                )
                .on_hover_text(play_tooltip)
                .clicked()
            {
                // Only take the armed-Play path when recording is armed for
                // THIS pattern. Routing through `Play` runs the engine's
                // Armed → CountIn → Capturing flow (`PlayPattern` skips that
                // transition). If armed for a *different* pattern, fall
                // through and just audition the one on screen. Solo is not
                // applied while recording: the engine is either in orphan-
                // preview mode (solo ignored there) or the user wants to
                // hear surrounding context while overdubbing.
                let armed_for_this = handle.state.transport.recording_state()
                    == RecordingState::Armed
                    && view_state.recording_pattern == Some(data.pattern_id);
                if armed_for_this {
                    handle.send(EngineCommand::Play);
                } else {
                    let solo_target = if view_state.pattern_solo {
                        Some(data.pattern_id)
                    } else {
                        None
                    };
                    handle.send(EngineCommand::SetSoloPattern(solo_target));
                    handle.send(EngineCommand::PlayPattern {
                        pattern_id: data.pattern_id,
                        instrument: view_state.selected_instrument,
                    });
                }
                // Starting playback re-engages playhead follow (both editors), so a
                // prior manual scroll-away doesn't leave the view stuck off-screen.
                view_state.auto_follow_playhead = true;
            }
        }

        // Stop button — also clears the solo filter via the engine.
        if ui
            .button(
                RichText::new(ri::STOP_FILL)
                    .size(12.0)
                    .color(if is_playing {
                        t.colors.accent_red
                    } else {
                        t.colors.text_dim
                    }),
            )
            .on_hover_text("Stop")
            .clicked()
        {
            handle.send(EngineCommand::Stop);
        }

        // Mini record button — mirrors the global transport's record
        // arm/disarm behaviour so the user does not have to reach
        // back up while editing in the piano roll.
        let mini_rec_state = handle.state.transport.recording_state();
        let dim_red = DIM_RED;
        let mini_rec_color = match mini_rec_state {
            RecordingState::Capturing => t.colors.accent_red,
            RecordingState::CountIn | RecordingState::Armed => {
                let blink = ((ui.input(|i| i.time) * 2.0) as u64).is_multiple_of(2);
                if blink { t.colors.accent_red } else { dim_red }
            }
            RecordingState::Idle => dim_red,
        };
        if ui
            .button(
                RichText::new(ri::RECORD_CIRCLE_FILL)
                    .size(12.0)
                    .color(mini_rec_color),
            )
            .on_hover_text(match mini_rec_state {
                RecordingState::Idle => "Arm recording",
                _ => "Disarm recording",
            })
            .clicked()
        {
            if mini_rec_state != RecordingState::Idle {
                handle.send(EngineCommand::DisarmRecord);
            } else {
                arm_recording_for_pattern(handle, song, view_state, data.pattern_id);
            }
        }
        if matches!(
            mini_rec_state,
            RecordingState::Armed | RecordingState::CountIn
        ) {
            ui.ctx().request_repaint();
        }

        // Pattern solo toggle — when ON (default), pattern playback
        // isolates the open pattern. When OFF, other tracks/patterns
        // overlapping the pattern's time range also play.
        let solo_icon = if view_state.pattern_solo {
            RichText::new(ri::VOLUME_MUTE_FILL)
                .size(12.0)
                .color(t.colors.accent_yellow)
        } else {
            RichText::new(ri::VOLUME_UP_LINE)
                .size(12.0)
                .color(t.colors.text_dim)
        };
        let solo_resp = ui
            .button(solo_icon)
            .on_hover_text(if view_state.pattern_solo {
                "Solo: only this pattern plays — click to also hear other tracks"
            } else {
                "Other tracks audible — click to isolate this pattern"
            });
        if solo_resp.clicked() {
            view_state.pattern_solo = !view_state.pattern_solo;
            if is_playing {
                let solo_target = if view_state.pattern_solo {
                    Some(data.pattern_id)
                } else {
                    None
                };
                handle.send(EngineCommand::SetSoloPattern(solo_target));
            }
        }

        ui.spacing_mut().item_spacing.x = 8.0;
    }
}

/// Piano-roll coordinate transforms between grid pixels and `(tick, pitch)`.
///
/// Bundles the seven view scalars that the four transform closures used to
/// capture, so they travel as one `Copy` value instead of four `&dyn Fn`
/// arguments plus three loose scalars (collapsing
/// `handle_piano_roll_interaction`'s parameter list). The methods reproduce the
/// former closures verbatim.
#[derive(Clone, Copy)]
struct PianoRollCoords {
    /// Left edge of the note grid (x of tick 0), in screen pixels.
    grid_x: f32,
    /// Top edge of the note grid (y of the top visible row), in screen pixels.
    grid_y: f32,
    /// Pattern ticks per beat (quarter note).
    ticks_per_beat: u32,
    /// Horizontal scale: screen pixels per beat.
    pr_pixels_per_beat: f32,
    /// Height of one pitch row, in screen pixels.
    note_row_height: f32,
    /// Lowest visible pitch (clamps `y_to_pitch`).
    view_pitch_min: Pitch,
    /// Highest visible pitch (grid row 0).
    view_pitch_max: Pitch,
}

impl PianoRollCoords {
    /// Tick → x screen position.
    fn tick_to_x(&self, tick_val: PatternTick) -> f32 {
        if self.ticks_per_beat == 0 {
            return self.grid_x;
        }
        let beats = tick_val.0 as f32 / self.ticks_per_beat as f32;
        self.grid_x + beats * self.pr_pixels_per_beat
    }

    /// Pitch → y screen position (higher pitch = lower y, piano style).
    fn pitch_to_y(&self, pitch: Pitch) -> f32 {
        let row = self
            .view_pitch_max
            .as_midi()
            .saturating_sub(pitch.as_midi());
        self.grid_y + row as f32 * self.note_row_height
    }

    /// x screen position → tick (clamped at 0).
    fn x_to_tick(&self, x: f32) -> PatternTick {
        #[allow(clippy::cast_possible_truncation)]
        let tick =
            ((x - self.grid_x) / self.pr_pixels_per_beat * self.ticks_per_beat as f32).max(0.0);
        PatternTick(tick as u32)
    }

    /// y screen position → pitch (clamped to the visible range).
    fn y_to_pitch(&self, y: f32) -> Pitch {
        #[allow(clippy::cast_possible_truncation)]
        let row = ((y - self.grid_y) / self.note_row_height).floor().max(0.0) as u8;
        let midi = self
            .view_pitch_max
            .as_midi()
            .saturating_sub(row)
            .clamp(self.view_pitch_min.as_midi(), self.view_pitch_max.as_midi());
        Pitch::new(midi).unwrap_or(Pitch::MIDDLE_C)
    }
}

/// Draw the piano roll in a bottom panel.
/// Returns false if the close button was clicked.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(crate) fn draw_piano_roll(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    playhead_tick: Option<PatternTick>,
    is_playing: bool,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    undo_manager: &mut crate::undo::UndoManager,
) -> bool {
    let t = theme();
    let mut keep_open = true;

    // ── Toolbar ──
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        // Pattern name (editable via rename context menu)
        if view_state.editing_pattern_name.as_ref().map(|(id, _)| *id) == Some(data.pattern_id) {
            if let Some((_, ref mut name_buf)) = view_state.editing_pattern_name {
                let resp = ui.add(
                    egui::TextEdit::singleline(name_buf)
                        .desired_width(120.0)
                        .font(egui::FontId::proportional(14.0)),
                );
                if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit_pattern_rename(song, undo_manager, data.pattern_id, name_buf.clone());
                    view_state.editing_pattern_name = None;
                } else if !resp.has_focus() {
                    resp.request_focus();
                }
            }
        } else {
            let name_resp = ui.add(
                egui::Label::new(
                    RichText::new(&data.pattern_name)
                        .size(14.0)
                        .color(t.colors.accent_cyan),
                )
                .sense(Sense::click()),
            );
            if name_resp.double_clicked() {
                view_state.editing_pattern_name =
                    Some((data.pattern_id, data.pattern_name.clone()));
            }
        }

        // Pattern description (editable inline; utility metadata, no undo)
        if view_state
            .editing_pattern_description
            .as_ref()
            .map(|(id, _)| *id)
            == Some(data.pattern_id)
        {
            if let Some((_, ref mut desc_buf)) = view_state.editing_pattern_description {
                let resp = ui.add(
                    egui::TextEdit::singleline(desc_buf)
                        .desired_width(180.0)
                        .hint_text("Description")
                        .font(egui::FontId::proportional(12.0)),
                );
                // Commit on every change so switching patterns mid-edit can't
                // strand the buffer; lost_focus (incl. Enter on a singleline)
                // ends the edit session.
                if resp.changed() {
                    commit_pattern_description(song, data.pattern_id, desc_buf.clone());
                }
                if resp.lost_focus() {
                    view_state.editing_pattern_description = None;
                } else if !resp.has_focus() {
                    resp.request_focus();
                }
            }
        } else {
            let desc_text = if data.pattern_description.is_empty() {
                RichText::new("+ description")
                    .size(12.0)
                    .italics()
                    .color(t.colors.text_dim)
            } else {
                RichText::new(&data.pattern_description)
                    .size(12.0)
                    .color(t.colors.text_secondary)
            };
            let desc_resp = ui
                .add(egui::Label::new(desc_text).sense(Sense::click()))
                .on_hover_text("Double-click to edit pattern description");
            if desc_resp.double_clicked() {
                view_state.editing_pattern_description =
                    Some((data.pattern_id, data.pattern_description.clone()));
            }
        }

        // Pattern length (whole bars)
        {
            let ticks_per_bar = data.time_sig.ticks_per_bar().max(1);
            let mut bars = (data.length_ticks.0 / ticks_per_bar).max(1) as i32;
            let bars_resp = ui
                .add(
                    egui::DragValue::new(&mut bars)
                        .range(1..=64)
                        .speed(0.1)
                        .suffix(" bars"),
                )
                .on_hover_text("Pattern length in bars");
            if bars_resp.drag_started() || bars_resp.gained_focus() {
                view_state.pattern_length_drag_start = Some((data.pattern_id, data.length_ticks));
            }
            if bars_resp.changed() {
                let new_len = SeqDuration(bars.max(1) as u32 * ticks_per_bar);
                let pid = data.pattern_id;
                let mut song_w = song.write();
                if let Some(pat) = song_w.pattern_mut(pid) {
                    pat.length = new_len;
                }
            }
            if (bars_resp.drag_stopped() || bars_resp.lost_focus())
                && let Some((pid, old_len)) = view_state.pattern_length_drag_start.take()
            {
                let new_len = SeqDuration(bars.max(1) as u32 * ticks_per_bar);
                if pid == data.pattern_id && old_len != new_len {
                    undo_manager.push(crate::undo::UndoAction::SetPatternLength {
                        pattern_id: pid,
                        old_length: old_len,
                        new_length: new_len,
                    });
                }
            }
        }

        draw_pattern_instrument_transport(
            ui,
            data,
            handle,
            song,
            view_state,
            instruments,
            is_playing,
        );
        ui.separator();

        ui.label(
            RichText::new(format!("{} notes", data.notes.len())).color(t.colors.text_secondary),
        );

        if !view_state.selected_notes.is_empty() {
            ui.label(
                RichText::new(format!("{} selected", view_state.selected_notes.len()))
                    .color(t.colors.accent_yellow),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(RichText::new(egui_remixicon::icons::CLOSE_LINE).color(t.colors.accent_red))
                .on_hover_text("Close piano roll")
                .clicked()
            {
                keep_open = false;
            }

            let help_btn = ui
                .button(
                    RichText::new(egui_remixicon::icons::QUESTION_LINE).color(t.colors.text_dim),
                )
                .on_hover_text("Keyboard shortcuts");
            egui::Popup::from_toggle_button_response(&help_btn).show(|ui| {
                ui.set_min_width(320.0);
                ui.label(RichText::new("Piano-roll keyboard shortcuts").strong());
                ui.add_space(t.spacing.xs);
                egui::Grid::new("pr_shortcuts_grid")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        let rows: &[(&str, &str)] = &[
                            ("Space", "Play / Pause"),
                            ("Delete · Backspace", "Delete selected notes"),
                            ("Escape", "Clear selection · exit step entry"),
                            ("Ctrl+A", "Select all notes"),
                            ("Ctrl+C", "Copy selection"),
                            ("Ctrl+X", "Cut selection"),
                            ("Ctrl+V", "Paste at playhead"),
                            ("Ctrl+D", "Duplicate selection after"),
                            ("↑ · ↓", "Transpose ±1 semitone"),
                            ("Shift+↑ · Shift+↓", "Transpose ±1 octave"),
                            ("Ctrl+scroll", "Horizontal zoom"),
                            ("Ctrl+Shift+scroll", "Vertical zoom"),
                            (
                                "STEP key letters",
                                concat!("A·W·S·E·D·F·T·G·Y·H·U·J ", "\u{ea6e}", " C..B"),
                            ),
                        ];
                        for (k, d) in rows {
                            ui.label(RichText::new(*k).monospace().color(t.colors.accent_cyan));
                            ui.label(*d);
                            ui.end_row();
                        }
                    });
            });
        });
    });

    ui.add_space(t.spacing.xxs);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        // Tool selector
        let select_label = if view_state.edit_tool == EditTool::Select {
            RichText::new("Select").color(t.colors.accent_primary)
        } else {
            RichText::new("Select").color(t.colors.text_secondary)
        };
        if ui.button(select_label).clicked() {
            view_state.edit_tool = EditTool::Select;
        }

        let draw_label = if view_state.edit_tool == EditTool::Draw {
            RichText::new("Draw").color(t.colors.accent_primary)
        } else {
            RichText::new("Draw").color(t.colors.text_secondary)
        };
        if ui.button(draw_label).clicked() {
            view_state.edit_tool = EditTool::Draw;
        }

        ui.separator();

        // Automation lane selector (shared with the tracker view).
        draw_automation_target_selector(ui, view_state, data, instruments);

        ui.separator();

        // ── Grid resolution selector ──
        let grid_label = GRID_RESOLUTIONS
            .iter()
            .find(|(_, t)| *t == view_state.draw_grid_resolution)
            .map_or("Grid", |(l, _)| l);
        egui::ComboBox::from_id_salt("grid_res")
            .selected_text(grid_label)
            .width(50.0)
            .show_ui(ui, |ui| {
                for &(label, ticks) in GRID_RESOLUTIONS {
                    if ui
                        .selectable_label(view_state.draw_grid_resolution == ticks, label)
                        .clicked()
                    {
                        view_state.draw_grid_resolution = ticks;
                    }
                }
            });

        // ── Note length preset selector ──
        let len_label = GRID_RESOLUTIONS
            .iter()
            .find(|(_, t)| *t == view_state.draw_note_length)
            .map_or("L:Drag", |(l, _)| l);
        egui::ComboBox::from_id_salt("note_len")
            .selected_text(if view_state.draw_note_length == 0 {
                "L:Drag"
            } else {
                len_label
            })
            .width(55.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(view_state.draw_note_length == 0, "Drag")
                    .clicked()
                {
                    view_state.draw_note_length = 0;
                }
                for &(label, ticks) in &GRID_RESOLUTIONS[1..] {
                    if ui
                        .selectable_label(view_state.draw_note_length == ticks, label)
                        .clicked()
                    {
                        view_state.draw_note_length = ticks;
                    }
                }
            });

        // ── Default velocity ──
        ui.label(RichText::new("Vel:").color(t.colors.text_dim).size(10.0));
        let mut vel_pct = (view_state.default_velocity.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut vel_pct)
                    .range(1.0..=100.0)
                    .speed(1.0)
                    .suffix(" %"),
            )
            .changed()
        {
            view_state.default_velocity = Velocity::new((vel_pct / 100.0).max(0.01));
        }

        // ── Quantize button ──
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty(),
                egui::Button::new(RichText::new("Q").color(t.colors.accent_primary)),
            )
            .on_hover_text(format!(
                "Quantize selected (strength {:.0}%)",
                view_state.quantize_strength.as_f32() * 100.0
            ))
            .clicked()
        {
            let grid = view_state.effective_grid(data.ticks_per_row);
            let strength = view_state.quantize_strength;
            let selected = view_state.selected_notes.clone();
            batch_transform_with_undo(song, data.pattern_id, &selected, undo_manager, |pattern| {
                pattern.quantize_selected(&selected, grid, strength)
            });
        }

        // ── Quantize strength (small drag value) ──
        let mut q_str_pct = (view_state.quantize_strength.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut q_str_pct)
                    .range(0.0..=100.0)
                    .speed(1.0)
                    .suffix(" %")
                    .prefix("Str:"),
            )
            .on_hover_text("Quantize strength")
            .changed()
        {
            view_state.quantize_strength = NormalizedValue::new(q_str_pct / 100.0);
        }

        // ── Humanize button ──
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty(),
                egui::Button::new(RichText::new("H").color(t.colors.text_secondary)),
            )
            .on_hover_text("Humanize selected notes (random timing/velocity)")
            .clicked()
        {
            let selected = view_state.selected_notes.clone();
            batch_transform_with_undo(song, data.pattern_id, &selected, undo_manager, |pattern| {
                pattern.humanize_notes(&selected, SeqDuration(15), NormalizedValue::new(0.05));
            });
        }

        ui.separator();

        // ── Swing control ──
        ui.label(RichText::new("Sw:").color(t.colors.text_dim).size(10.0));
        let mut sw_pct = (view_state.swing_amount.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut sw_pct)
                    .range(0.0..=100.0)
                    .speed(1.0)
                    .suffix(" %"),
            )
            .on_hover_text("Swing amount (offset even subdivisions)")
            .changed()
        {
            view_state.swing_amount = NormalizedValue::new(sw_pct / 100.0);
        }
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty() && view_state.swing_amount.as_f32() > 0.0,
                egui::Button::new(RichText::new("Apply").color(t.colors.text_secondary)),
            )
            .on_hover_text("Apply swing to selected notes")
            .clicked()
        {
            let grid = view_state.effective_grid(data.ticks_per_row);
            let amount = view_state.swing_amount;
            let selected = view_state.selected_notes.clone();
            batch_transform_with_undo(song, data.pattern_id, &selected, undo_manager, |pattern| {
                pattern.apply_swing(&selected, grid, amount)
            });
        }

        // ── Scale velocities ──
        ui.add(
            egui::DragValue::new(&mut view_state.velocity_scale_pct)
                .range(1..=200_u32)
                .speed(1.0)
                .suffix(" %")
                .prefix("Vel×"),
        )
        .on_hover_text("Velocity scale factor");
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty() && view_state.velocity_scale_pct != 100,
                egui::Button::new(RichText::new("Scale").color(t.colors.text_secondary)),
            )
            .on_hover_text("Scale velocities of selected notes")
            .clicked()
        {
            let mut changes: Vec<(NoteId, Velocity, Velocity)> = Vec::new();
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                    for nid in &view_state.selected_notes {
                        if let Some(note) = pattern.note(*nid) {
                            changes.push((*nid, note.velocity, note.velocity));
                        }
                    }
                    pattern.scale_velocities(
                        &view_state.selected_notes,
                        view_state.velocity_scale_pct as f32 / 100.0,
                    );
                    for entry in &mut changes {
                        if let Some(note) = pattern.note(entry.0) {
                            entry.2 = note.velocity;
                        }
                    }
                }
            }
            if changes.iter().any(|(_, o, n)| o != n) {
                undo_manager.push(crate::undo::UndoAction::SetVelocitiesBatch {
                    pattern_id: data.pattern_id,
                    changes,
                });
            }
            view_state.velocity_scale_pct = 100;
        }

        // ── Step entry toggle ──
        let step_color = if view_state.step_entry_mode {
            t.colors.accent_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .button(RichText::new("STEP").color(step_color))
            .on_hover_text("Step entry mode")
            .clicked()
        {
            view_state.step_entry_mode = !view_state.step_entry_mode;
            if view_state.step_entry_mode {
                view_state.step_cursor_tick = PatternTick::ZERO;
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                view_state.pr_zoom_x = (view_state.pr_zoom_x * 1.25).min(4.0);
                view_state.pr_zoom_y = (view_state.pr_zoom_y * 1.25).min(3.0);
            }
            if ui.small_button("1x").on_hover_text("Reset zoom").clicked() {
                view_state.pr_zoom_x = 1.0;
                view_state.pr_zoom_y = 1.0;
            }
            if ui
                .small_button("-")
                .on_hover_text("Zoom out (Ctrl+scroll = horizontal, Ctrl+Shift+scroll = vertical)")
                .clicked()
            {
                view_state.pr_zoom_x = (view_state.pr_zoom_x * 0.8).max(0.25);
                view_state.pr_zoom_y = (view_state.pr_zoom_y * 0.8).max(0.5);
            }
            ui.label(RichText::new("Zoom").color(t.colors.text_dim).size(10.0));
        });
    });

    ui.separator();

    // ── Step entry banner ──
    if view_state.step_entry_mode {
        let banner_color = STEP_ENTRY_BANNER_FILL;
        egui::Frame::new()
            .fill(banner_color)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .corner_radius(2)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("STEP ENTRY")
                            .strong()
                            .color(STEP_ENTRY_TEXT),
                    );
                    ui.label(
                        RichText::new(
                            "Press keys to insert notes  (A·W·S·E·D·F·T·G·Y·H·U·J = C·C#·D·D#·E·F·F#·G·G#·A·A#·B)",
                        )
                        .size(11.0)
                        .color(t.colors.text_secondary),
                    );
                    ui.label(
                        RichText::new("· Esc disables").size(11.0).color(t.colors.text_dim),
                    );
                });
            });
        ui.separator();
    }

    draw_piano_roll_selection_inspector(ui, data, view_state, song, undo_manager);

    // ── Pitch range with margin ──
    // Fold live recording-preview pitches into the range so notes recorded
    // outside the committed range are visible immediately, not only after the
    // next loop wrap flushes them into the pattern. Only do this when the
    // preview belongs to the pattern on screen — the preview buffers are
    // global, so without this guard a recording into another pattern would
    // widen this one's vertical range to an empty band.
    let mut pitch_min = data.pitch_min;
    let mut pitch_max = data.pitch_max;
    if view_state.recording_pattern == Some(data.pattern_id) {
        for note in &view_state.recording_preview_completed {
            if note.pitch < pitch_min {
                pitch_min = note.pitch;
            }
            if note.pitch > pitch_max {
                pitch_max = note.pitch;
            }
        }
        for (pitch, _) in &view_state.recording_preview_held {
            if *pitch < pitch_min {
                pitch_min = *pitch;
            }
            if *pitch > pitch_max {
                pitch_max = *pitch;
            }
        }
    }

    let margin = 6;
    let view_pitch_min = pitch_min.saturating_sub(margin);
    let view_pitch_max = pitch_max.saturating_add(margin);
    let pitch_range = view_pitch_max.as_midi() - view_pitch_min.as_midi() + 1;

    // Zoom-scaled grid units (1.0 zoom = default constants).
    let note_row_height = DEFAULT_NOTE_ROW_HEIGHT * view_state.pr_zoom_y;
    let pr_pixels_per_beat = DEFAULT_PR_PIXELS_PER_BEAT * view_state.pr_zoom_x;

    let grid_height = pitch_range as f32 * note_row_height;
    let auto_height = if view_state.selected_automation.is_some() {
        AUTOMATION_ZONE_HEIGHT
    } else {
        0.0
    };
    let total_content_height = grid_height + VELOCITY_ZONE_HEIGHT + auto_height;

    // Timeline width: use max of pattern length and furthest note end
    let ticks_per_beat = synth_sequencer::TICKS_PER_QUARTER;
    let max_note_end = data
        .notes
        .iter()
        .filter_map(|n| n.end_tick)
        .max()
        .map_or(0, |t| t.0);
    let effective_ticks = data.length_ticks.0.max(max_note_end);
    let beats_in_pattern = if ticks_per_beat > 0 {
        effective_ticks as f32 / ticks_per_beat as f32
    } else {
        4.0
    };
    let grid_width = (beats_in_pattern * pr_pixels_per_beat).max(200.0);

    // ── Scrollable piano roll area ──
    // Use all available height (panel is resizable via TopBottomPanel)
    let scroll_max_height = ui.available_height().max(100.0);

    // Auto-follow playhead in piano roll during playback
    let pr_scroll_salt = "piano_roll_scroll";
    let pr_scroll_id = ui.make_persistent_id(egui::Id::new(pr_scroll_salt));
    if is_playing
        && view_state.auto_follow_playhead
        && let Some(pt) = playhead_tick
        && ticks_per_beat > 0
    {
        let playhead_beats = pt.0 as f32 / ticks_per_beat as f32;
        let playhead_x = KEY_WIDTH + playhead_beats * pr_pixels_per_beat;
        let visible_width = ui.available_width();
        let target_offset = (playhead_x - visible_width * 0.5).max(0.0);

        if let Some(mut scroll_state) = egui::scroll_area::State::load(ui.ctx(), pr_scroll_id) {
            scroll_state.offset.x = target_offset;
            scroll_state.store(ui.ctx(), pr_scroll_id);
            view_state.pr_last_auto_scroll_offset = Some(target_offset);
        }
    }

    let scroll_output = egui::ScrollArea::both()
        .id_salt(pr_scroll_salt)
        .max_height(scroll_max_height)
        .scroll_source(egui::scroll_area::ScrollSource {
            scroll_bar: true,
            drag: false, // Don't steal drag events — we handle them for note editing
            mouse_wheel: true,
        })
        .show(ui, |ui| {
            let total_size = Vec2::new(KEY_WIDTH + grid_width, RULER_HEIGHT + total_content_height);

            // Use allocate_rect with click_and_drag sense for mouse interaction
            let alloc_rect = Rect::from_min_size(ui.cursor().min, total_size);
            let response = ui.allocate_rect(alloc_rect, Sense::click_and_drag());
            let rect = response.rect;
            let painter = ui.painter_at(rect);

            // ── Ctrl+scroll → zoom (consumed before scroll area sees it) ──
            if response.hovered() {
                let (scroll_dy, ctrl, shift) = ui.input(|i| {
                    (
                        i.smooth_scroll_delta.y,
                        i.modifiers.ctrl || i.modifiers.command,
                        i.modifiers.shift,
                    )
                });
                if ctrl && scroll_dy != 0.0 {
                    let factor = 1.0 + scroll_dy * 0.002;
                    if shift {
                        view_state.pr_zoom_y = (view_state.pr_zoom_y * factor).clamp(0.5, 3.0);
                    } else {
                        view_state.pr_zoom_x = (view_state.pr_zoom_x * factor).clamp(0.25, 4.0);
                    }
                }
            }

            let origin = rect.min;
            let grid_x = origin.x + KEY_WIDTH;
            // Reserve a ruler strip at the top; the grid starts below it.
            let grid_y = origin.y + RULER_HEIGHT;

            // All four grid↔(tick,pitch) transforms live on this one value (see
            // `PianoRollCoords`). The closures below just delegate so the rest of
            // this function — and the interaction handler — keep their call shape.
            let coords = PianoRollCoords {
                grid_x,
                grid_y,
                ticks_per_beat,
                pr_pixels_per_beat,
                note_row_height,
                view_pitch_min,
                view_pitch_max,
            };
            let tick_to_x = |tick_val: PatternTick| coords.tick_to_x(tick_val);
            let pitch_to_y = |pitch: Pitch| coords.pitch_to_y(pitch);

            // Grid rect for checking if pointer is in the note grid area
            let grid_rect = Rect::from_min_size(
                Pos2::new(grid_x, grid_y),
                Vec2::new(grid_width, grid_height),
            );

            // ── Timeline ruler (bar numbers) ──
            // Top-left corner cell above the keyboard column.
            painter.rect_filled(
                Rect::from_min_size(origin, Vec2::new(KEY_WIDTH, RULER_HEIGHT)),
                0.0,
                t.colors.bg_dark,
            );
            let ruler_rect = Rect::from_min_size(
                Pos2::new(grid_x, origin.y),
                Vec2::new(grid_width, RULER_HEIGHT),
            );
            let ticks_per_bar = u64::from(data.time_sig.ticks_per_bar().max(1));
            let total_bars = effective_ticks.div_ceil(data.time_sig.ticks_per_bar().max(1)).max(1);
            draw_ruler_labels(&painter, &t, ruler_rect, total_bars, ticks_per_bar, |tick| {
                if ticks_per_beat == 0 {
                    grid_x
                } else {
                    grid_x + (tick as f32 / ticks_per_beat as f32) * pr_pixels_per_beat
                }
            });
            // Ruler bottom border.
            painter.line_segment(
                [
                    Pos2::new(grid_x, grid_y),
                    Pos2::new(grid_x + grid_width, grid_y),
                ],
                Stroke::new(1.0, t.colors.border),
            );

            // ── Keyboard (left column) ──
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(origin.x, grid_y), Vec2::new(KEY_WIDTH, grid_height)),
                0.0,
                t.colors.bg_dark,
            );

            for p in view_pitch_min.as_midi()..=view_pitch_max.as_midi() {
                let pitch = Pitch::new(p).unwrap_or(Pitch::MIDDLE_C);
                let y = pitch_to_y(pitch);
                let note_name = NoteName::from_midi(p % 12);
                let is_black = note_name.is_black_key();

                // Key background
                let key_color = if is_black {
                    PIANO_KEY_BLACK
                } else {
                    PIANO_KEY_WHITE
                };
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(origin.x, y),
                        Vec2::new(KEY_WIDTH, note_row_height),
                    ),
                    0.0,
                    key_color,
                );

                // Label on C notes
                if p % 12 == 0 {
                    let octave = (p / 12) as i8 - 1;
                    painter.text(
                        Pos2::new(origin.x + 4.0, y + 1.0),
                        egui::Align2::LEFT_TOP,
                        format!("C{octave}"),
                        egui::FontId::proportional(10.0),
                        t.colors.text_primary,
                    );
                }

                // Key border
                painter.line_segment(
                    [
                        Pos2::new(origin.x, y + note_row_height),
                        Pos2::new(origin.x + KEY_WIDTH, y + note_row_height),
                    ],
                    Stroke::new(0.5, t.colors.border.gamma_multiply(0.3)),
                );
            }

            // ── Note grid background ──
            for p in view_pitch_min.as_midi()..=view_pitch_max.as_midi() {
                let pitch = Pitch::new(p).unwrap_or(Pitch::MIDDLE_C);
                let y = pitch_to_y(pitch);
                let note_name = NoteName::from_midi(p % 12);
                let is_black = note_name.is_black_key();
                let is_c = p % 12 == 0;

                let bg = if is_c {
                    GRID_BG_C
                } else if is_black {
                    GRID_BG_BLACK
                } else {
                    GRID_BG_WHITE
                };

                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(grid_x, y),
                        Vec2::new(grid_width, note_row_height),
                    ),
                    0.0,
                    bg,
                );

                // Horizontal pitch row separator
                painter.line_segment(
                    [
                        Pos2::new(grid_x, y + note_row_height),
                        Pos2::new(grid_x + grid_width, y + note_row_height),
                    ],
                    Stroke::new(
                        if is_c { 0.8 } else { 0.3 },
                        t.colors.border.gamma_multiply(if is_c { 0.6 } else { 0.2 }),
                    ),
                );
            }

            // ── Vertical beat/sub-beat lines ──
            let beats_total = beats_in_pattern.ceil() as u32;
            let beats_per_bar_val = data.time_sig.numerator.max(1) as u32;
            for beat_idx in 0..=beats_total {
                let beat_tick = beat_idx * ticks_per_beat;
                let x = tick_to_x(PatternTick(beat_tick));
                let is_bar_line = beat_idx % beats_per_bar_val == 0;

                painter.line_segment(
                    [Pos2::new(x, grid_y), Pos2::new(x, grid_y + grid_height)],
                    Stroke::new(
                        if is_bar_line { 1.0 } else { 0.5 },
                        t.colors
                            .border
                            .gamma_multiply(if is_bar_line { 0.8 } else { 0.3 }),
                    ),
                );

                // Sub-beat lines (based on ticks_per_row)
                if data.ticks_per_row > 0 && !is_bar_line {
                    // Already drawing at beat level; skip sub-beats for readability
                }
            }

            // Sub-beat grid lines based on row resolution
            if data.ticks_per_row > 0 {
                let total_rows = data.length_ticks.0 / data.ticks_per_row as u32;
                for row in 0..total_rows {
                    let row_tick = row * data.ticks_per_row as u32;
                    // Skip if this aligns with a beat line (already drawn)
                    if row_tick.is_multiple_of(ticks_per_beat) {
                        continue;
                    }
                    let x = tick_to_x(PatternTick(row_tick));
                    painter.line_segment(
                        [Pos2::new(x, grid_y), Pos2::new(x, grid_y + grid_height)],
                        Stroke::new(0.3, t.colors.border.gamma_multiply(0.15)),
                    );
                }
            }

            // ── Notes ──
            let default_note_color = DEFAULT_NOTE_BLUE;
            // One-shot per-frame colour cache so each note's lookup is O(1).
            let note_color_cache = build_instrument_colour_cache(instruments);

            for note in &data.notes {
                if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
                    continue;
                }

                // Skip notes that are being dragged (draw ghost instead)
                let is_being_moved = matches!(
                    &view_state.drag,
                    Some(DragState::MoveNote { note_id, .. }) if *note_id == note.note_id
                );
                if is_being_moved {
                    continue;
                }

                let y = pitch_to_y(note.pitch);
                let x_start = tick_to_x(note.start_tick);

                let x_end = match note.end_tick {
                    Some(end) => tick_to_x(end),
                    None => {
                        // Open-ended: draw to pattern end or at least a visible width
                        tick_to_x(data.length_ticks.as_pattern_tick()).min(x_start + grid_width)
                    }
                };

                // Apply resize preview if this note is being resized
                let x_end = match &view_state.drag {
                    Some(DragState::ResizeNote {
                        note_id,
                        current_end_tick,
                        ..
                    }) if *note_id == note.note_id => tick_to_x(*current_end_tick),
                    _ => x_end,
                };

                let note_width = (x_end - x_start).max(3.0);
                let alpha = (note.velocity.as_f32() * 200.0 + 55.0).min(255.0) as u8;

                // Notes have no per-note instrument; colour by the instrument
                // the pattern plays through (its placement's track), falling
                // back to the working instrument for an unplaced pattern.
                let note_instrument = data
                    .track_overrides
                    .first()
                    .copied()
                    .unwrap_or(view_state.selected_instrument);
                let inst_color =
                    cached_instrument_color(&note_color_cache, note_instrument, default_note_color);

                let is_selected = view_state.selected_notes.contains(&note.note_id);

                let fill = Color32::from_rgba_unmultiplied(
                    inst_color.r(),
                    inst_color.g(),
                    inst_color.b(),
                    alpha,
                );

                let note_rect = Rect::from_min_size(
                    Pos2::new(x_start, y + 1.0),
                    Vec2::new(note_width, note_row_height - 2.0),
                );

                if is_selected {
                    // Soft glow halo behind the note (cyan tint).
                    let glow_rect = note_rect.expand(3.0);
                    painter.rect_filled(
                        glow_rect,
                        3.0,
                        NOTE_SELECTED_GLOW,
                    );
                }

                painter.rect_filled(note_rect, 2.0, fill);

                if is_selected {
                    // High-contrast outer outline (white) — clearly visible
                    // against any instrument colour or grid background.
                    painter.rect_stroke(
                        note_rect.expand(1.0),
                        3.0,
                        Stroke::new(2.0, Color32::WHITE),
                        egui::StrokeKind::Outside,
                    );
                    // Cyan inner accent for a recognisable selection colour.
                    painter.rect_stroke(
                        note_rect,
                        2.0,
                        Stroke::new(1.0, t.colors.accent_cyan),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    painter.rect_stroke(
                        note_rect,
                        2.0,
                        Stroke::new(0.5, inst_color),
                        egui::StrokeKind::Inside,
                    );
                }

                // Per-note expression markers (B4): a tie underline for legato
                // and a small ramp glyph at the left edge for glide.
                if note.legato {
                    let y_line = note_rect.max.y - 1.0;
                    painter.line_segment(
                        [
                            Pos2::new(note_rect.min.x + 1.0, y_line),
                            Pos2::new(note_rect.max.x - 1.0, y_line),
                        ],
                        Stroke::new(1.5, t.colors.accent_yellow),
                    );
                }
                if note.glide.is_some() {
                    // Diagonal ramp from bottom-left up into the note's leading edge.
                    let gx = note_rect.min.x;
                    painter.line_segment(
                        [
                            Pos2::new(gx + 0.5, note_rect.max.y - 1.0),
                            Pos2::new((gx + 6.0).min(note_rect.max.x), note_rect.min.y + 1.0),
                        ],
                        Stroke::new(1.5, t.colors.accent_cyan),
                    );
                }
                // Per-note expression (C5): a small dot at the top-right corner.
                if note.expression.is_some() && note_width > 5.0 {
                    painter.circle_filled(
                        Pos2::new(note_rect.max.x - 2.5, note_rect.min.y + 2.5),
                        1.5,
                        t.colors.accent_yellow,
                    );
                }

                // Open-ended indicator (gradient fade-out at right edge)
                if note.end_tick.is_none() && note_width > 8.0 {
                    let fade_rect = Rect::from_min_max(
                        Pos2::new(note_rect.max.x - 6.0, note_rect.min.y),
                        note_rect.max,
                    );
                    painter.rect_filled(
                        fade_rect,
                        0.0,
                        Color32::from_rgba_unmultiplied(
                            inst_color.r(),
                            inst_color.g(),
                            inst_color.b(),
                            alpha / 3,
                        ),
                    );
                }
            }

            // ── Recording preview notes (orange) ──
            if !view_state.recording_preview_completed.is_empty()
                || !view_state.recording_preview_held.is_empty()
            {
                let preview_color = RECORDING_PREVIEW_ORANGE;

                // Draw completed preview notes
                for note in &view_state.recording_preview_completed {
                    if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
                        continue;
                    }
                    let y = pitch_to_y(note.pitch);
                    let x_start = tick_to_x(note.start);
                    let x_end = tick_to_x(note.start + note.duration);
                    let note_width = (x_end - x_start).max(3.0);
                    let alpha = 180_u8;

                    let preview_rect = Rect::from_min_size(
                        Pos2::new(x_start, y + 1.0),
                        Vec2::new(note_width, note_row_height - 2.0),
                    );
                    painter.rect_filled(
                        preview_rect,
                        2.0,
                        Color32::from_rgba_unmultiplied(
                            preview_color.r(),
                            preview_color.g(),
                            preview_color.b(),
                            alpha,
                        ),
                    );
                    painter.rect_stroke(
                        preview_rect,
                        2.0,
                        Stroke::new(0.5, preview_color),
                        egui::StrokeKind::Inside,
                    );
                }

                // Draw held notes extending from start to current playhead
                if !view_state.recording_preview_held.is_empty() {
                    // Compute playhead position within pattern
                    let playhead_in_pattern = playhead_tick.map_or(0, |pt| pt.0);

                    for (pitch, start_tick) in &view_state.recording_preview_held {
                        if *pitch < view_pitch_min || *pitch > view_pitch_max {
                            continue;
                        }
                        let y = pitch_to_y(*pitch);
                        let x_start = tick_to_x(*start_tick);
                        let end = if playhead_in_pattern >= start_tick.0 {
                            PatternTick(playhead_in_pattern)
                        } else {
                            // Note wraps around pattern — draw to end
                            data.length_ticks.as_pattern_tick()
                        };
                        let x_end = tick_to_x(end);
                        let note_width = (x_end - x_start).max(3.0);

                        let held_rect = Rect::from_min_size(
                            Pos2::new(x_start, y + 1.0),
                            Vec2::new(note_width, note_row_height - 2.0),
                        );
                        painter.rect_filled(
                            held_rect,
                            2.0,
                            RECORDING_PREVIEW_HELD_FILL,
                        );
                        painter.rect_stroke(
                            held_rect,
                            2.0,
                            Stroke::new(0.5, preview_color),
                            egui::StrokeKind::Inside,
                        );
                    }
                }

                // Request repaint during recording for live updates
                ui.request_repaint();
            }

            // ── Ghost note for MoveNote drag ──
            if let Some(DragState::MoveNote {
                note_id,
                current_tick: drag_tick,
                current_pitch: drag_pitch,
                ..
            }) = &view_state.drag
            {
                // Find the original note data for velocity/duration
                if let Some(note) = data.notes.iter().find(|n| n.note_id == *note_id) {
                    let duration_ticks = note.end_tick.map_or(
                        data.length_ticks.as_pattern_tick() - note.start_tick,
                        |end| end - note.start_tick,
                    );
                    let y = pitch_to_y(*drag_pitch);
                    let x_start = tick_to_x(*drag_tick);
                    let x_end = tick_to_x(*drag_tick + duration_ticks);
                    let note_width = (x_end - x_start).max(3.0);

                    let ghost_rect = Rect::from_min_size(
                        Pos2::new(x_start, y + 1.0),
                        Vec2::new(note_width, note_row_height - 2.0),
                    );

                    // Semi-transparent ghost
                    painter.rect_filled(
                        ghost_rect,
                        2.0,
                        MOVE_GHOST_FILL,
                    );
                    painter.rect_stroke(
                        ghost_rect,
                        2.0,
                        Stroke::new(1.0, MOVE_GHOST_STROKE),
                        egui::StrokeKind::Inside,
                    );
                }
            }

            // ── DrawNote preview ──
            if let Some(DragState::DrawNote {
                start_tick,
                pitch,
                current_end_tick,
            }) = &view_state.drag
            {
                let y = pitch_to_y(*pitch);
                let x_start = tick_to_x(*start_tick);
                let x_end = tick_to_x(*current_end_tick);
                let note_width = (x_end - x_start).max(3.0);

                let draw_rect = Rect::from_min_size(
                    Pos2::new(x_start, y + 1.0),
                    Vec2::new(note_width, note_row_height - 2.0),
                );
                painter.rect_filled(
                    draw_rect,
                    2.0,
                    DRAW_NOTE_FILL,
                );
                painter.rect_stroke(
                    draw_rect,
                    2.0,
                    Stroke::new(1.0, DRAW_NOTE_STROKE),
                    egui::StrokeKind::Inside,
                );
            }

            // ── Selection rectangle ──
            if let Some(DragState::SelectRect {
                start_pos,
                current_pos,
            }) = &view_state.drag
            {
                let sel_rect = Rect::from_two_pos(*start_pos, *current_pos);
                painter.rect_filled(
                    sel_rect,
                    0.0,
                    SELECTION_RECT_FILL,
                );
                painter.rect_stroke(
                    sel_rect,
                    0.0,
                    Stroke::new(1.0, SELECTION_RECT_STROKE),
                    egui::StrokeKind::Inside,
                );
            }

            // ── Velocity bars (below grid) ──
            let vel_y = grid_y + grid_height;
            // Background
            painter.rect_filled(
                Rect::from_min_size(
                    Pos2::new(grid_x, vel_y),
                    Vec2::new(grid_width, VELOCITY_ZONE_HEIGHT),
                ),
                0.0,
                VELOCITY_ZONE_BG,
            );

            // Separator line
            painter.line_segment(
                [
                    Pos2::new(grid_x, vel_y),
                    Pos2::new(grid_x + grid_width, vel_y),
                ],
                Stroke::new(1.0, t.colors.border),
            );

            // Velocity label
            painter.text(
                Pos2::new(origin.x + 2.0, vel_y + 2.0),
                egui::Align2::LEFT_TOP,
                "VEL",
                egui::FontId::proportional(9.0),
                t.colors.text_dim,
            );

            // Velocity bars
            for note in &data.notes {
                let x = tick_to_x(note.start_tick);
                let bar_height = note.velocity.as_f32() * (VELOCITY_ZONE_HEIGHT - 4.0);
                let bar_y = vel_y + VELOCITY_ZONE_HEIGHT - bar_height - 2.0;

                let is_selected = view_state.selected_notes.contains(&note.note_id);
                let (vel_color, bar_w) = if is_selected {
                    (VELOCITY_BAR_SELECTED, 5.0)
                } else {
                    (velocity_color(note.velocity.as_f32()), 3.0)
                };
                let bar_rect = Rect::from_min_size(
                    Pos2::new(x - bar_w * 0.5, bar_y),
                    Vec2::new(bar_w, bar_height),
                );
                painter.rect_filled(bar_rect, 1.0, vel_color);
                if is_selected {
                    painter.rect_stroke(
                        bar_rect,
                        1.0,
                        Stroke::new(1.0, Color32::WHITE),
                        egui::StrokeKind::Outside,
                    );
                }
            }

            // ── Automation zone (below velocity) ──
            let auto_y = vel_y + VELOCITY_ZONE_HEIGHT;
            if let Some(selected_target) = &view_state.selected_automation {
                draw_automation_zone(
                    &painter,
                    data,
                    view_state,
                    selected_target,
                    grid_x,
                    auto_y,
                    grid_width,
                    &tick_to_x,
                    &t,
                );
            }

            // ── Playhead (only if this pattern is actually playing) ──
            if let Some(pattern_tick) = playhead_tick {
                let playhead_x = tick_to_x(pattern_tick);

                if playhead_x >= grid_x && playhead_x <= grid_x + grid_width {
                    // Line runs from the top of the ruler down through the grid.
                    painter.line_segment(
                        [
                            Pos2::new(playhead_x, origin.y),
                            Pos2::new(playhead_x, grid_y + total_content_height),
                        ],
                        Stroke::new(1.5, t.colors.accent_primary),
                    );
                    // Triangle marker in the ruler strip.
                    let tri_size = 6.0;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            Pos2::new(playhead_x - tri_size, origin.y),
                            Pos2::new(playhead_x + tri_size, origin.y),
                            Pos2::new(playhead_x, origin.y + RULER_HEIGHT * 0.6),
                        ],
                        t.colors.accent_primary,
                        Stroke::NONE,
                    ));
                }
            }

            // ── Step cursor ──
            if view_state.step_entry_mode {
                let cursor_x = tick_to_x(view_state.step_cursor_tick);
                if cursor_x >= grid_x && cursor_x <= grid_x + grid_width {
                    painter.line_segment(
                        [
                            Pos2::new(cursor_x, grid_y),
                            Pos2::new(cursor_x, grid_y + grid_height),
                        ],
                        Stroke::new(2.0, STEP_CURSOR),
                    );
                }
            }

            // ── Keyboard / grid separator ──
            painter.line_segment(
                [
                    Pos2::new(grid_x, grid_y),
                    Pos2::new(grid_x, grid_y + total_content_height),
                ],
                Stroke::new(1.0, t.colors.border),
            );

            // Automation zone rect (for hit-testing)
            let auto_rect = if view_state.selected_automation.is_some() {
                Some(Rect::from_min_size(
                    Pos2::new(grid_x, auto_y),
                    Vec2::new(grid_width, AUTOMATION_ZONE_HEIGHT),
                ))
            } else {
                None
            };

            // ── Hover and cursor ──
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                // Check automation zone hover first
                if auto_rect.is_some_and(|r| r.contains(pos)) {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                } else if grid_rect.contains(pos) {
                    let vp_min = view_pitch_min;
                    let vp_max = view_pitch_max;
                    let hit = note_at_pos(
                        &data.notes,
                        pos,
                        &tick_to_x,
                        &pitch_to_y,
                        data.length_ticks,
                        vp_min,
                        vp_max,
                        note_row_height,
                    );

                    match hit {
                        Some((_, HitZone::RightEdge)) => {
                            ui.ctx().set_cursor_icon(CursorIcon::ResizeEast);
                        }
                        Some((note_id, HitZone::Body)) => {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);

                            // Subtle hover highlight
                            if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                                let y = pitch_to_y(note.pitch);
                                let x_start = tick_to_x(note.start_tick);
                                let x_end = match note.end_tick {
                                    Some(end) => tick_to_x(end),
                                    None => tick_to_x(data.length_ticks.as_pattern_tick()),
                                };
                                let hover_rect = Rect::from_min_size(
                                    Pos2::new(x_start, y + 1.0),
                                    Vec2::new((x_end - x_start).max(3.0), note_row_height - 2.0),
                                );
                                painter.rect_stroke(
                                    hover_rect,
                                    2.0,
                                    Stroke::new(1.0, NOTE_HOVER_OUTLINE),
                                    egui::StrokeKind::Outside,
                                );

                                let midi = note.pitch.as_midi();
                                let pitch_name = NoteName::from_midi(midi % 12);
                                let octave = (midi / 12) as i8 - 1;
                                let beats = note.start_tick.0 as f32
                                    / synth_sequencer::TICKS_PER_QUARTER as f32;
                                let (bar0, beat0, tick0) =
                                    Tick(note.start_tick.0 as u64).to_bar_beat_tick(data.time_sig);
                                let bar = bar0 + 1;
                                let beat_in_bar = beat0 as f32
                                    + tick0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32
                                    + 1.0;
                                let length_text = match note.end_tick {
                                    Some(end) => {
                                        let dur_beats = (end.0 - note.start_tick.0) as f32
                                            / synth_sequencer::TICKS_PER_QUARTER as f32;
                                        format!("{dur_beats:.2} beats")
                                    }
                                    None => "open".to_owned(),
                                };
                                let vel_pct = (note.velocity.as_f32() * 100.0).round();
                                // Notes route through their placement's track instrument.
                                let note_instrument =
                                    data.track_overrides.first().copied().unwrap_or_default();
                                let inst_name = instruments
                                    .iter()
                                    .find(|inst| inst.id.0 == note_instrument.0 as u64)
                                    .map_or_else(
                                        || format!("#{}", note_instrument.0),
                                        |inst| inst.name.clone(),
                                    );
                                let tooltip_id = ui.id().with(("note_tip", note_id.0));
                                egui::Tooltip::always_open(
                                    ui.ctx().clone(),
                                    ui.layer_id(),
                                    tooltip_id,
                                    pos,
                                )
                                .at_pointer()
                                .show(|ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{pitch_name:?}{octave}  (MIDI {midi})"
                                        ))
                                        .strong(),
                                    );
                                    ui.label(format!(
                                        "Bar {bar}, beat {beat_in_bar:.2}  ({beats:.2} beats from start)"
                                    ));
                                    ui.label(format!("Length: {length_text}"));
                                    ui.label(format!("Velocity: {vel_pct}%"));
                                    ui.label(format!("Instrument: {inst_name}"));
                                });
                            }
                        }
                        None => {
                            if view_state.edit_tool == EditTool::Draw {
                                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                            }
                        }
                    }
                }
            }

            // ── Mouse interaction ──
            handle_piano_roll_interaction(
                &response,
                ui,
                data,
                song,
                view_state,
                handle,
                grid_rect,
                auto_rect,
                auto_y,
                &coords,
                undo_manager,
            );
        });

    // Detect manual scrolling to disable auto-follow, mirroring the
    // arrangement timeline: if the offset after the scroll area differs from
    // what auto-follow set, the user dragged the scrollbar — stop fighting.
    if is_playing {
        let actual_offset = scroll_output.state.offset.x;
        if let Some(expected) = view_state.pr_last_auto_scroll_offset
            && (actual_offset - expected).abs() > 2.0
        {
            view_state.auto_follow_playhead = false;
            view_state.pr_last_auto_scroll_offset = None;
        }
    } else {
        view_state.pr_last_auto_scroll_offset = None;
    }

    // ── Keyboard shortcuts ──
    let ctx = ui.ctx();
    if ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
        delete_selected_notes(
            song,
            data.pattern_id,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        view_state.selected_notes.clear();
        view_state.drag = None;
        view_state.step_entry_mode = false;
    }

    // ── Ctrl+A — select all notes ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
        view_state.selected_notes.clear();
        for note in &data.notes {
            view_state.selected_notes.insert(note.note_id);
        }
    }

    // ── Ctrl+C — copy selected notes ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
    }

    // ── Ctrl+X — cut selected notes ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::X))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
        delete_selected_notes(
            song,
            data.pattern_id,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Ctrl+V — paste at playhead or start ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V))
        && !view_state.clipboard.notes.is_empty()
    {
        let paste_tick = playhead_tick.unwrap_or(PatternTick(0));
        paste_clipboard_notes(
            song,
            data.pattern_id,
            &view_state.clipboard,
            paste_tick,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Ctrl+D — duplicate selected notes ──
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
        // Find min tick of selection
        let min_tick = data
            .notes
            .iter()
            .filter(|n| view_state.selected_notes.contains(&n.note_id))
            .map(|n| n.start_tick)
            .min()
            .unwrap_or(PatternTick::ZERO);
        let paste_tick = min_tick + view_state.clipboard.selection_width;
        paste_clipboard_notes(
            song,
            data.pattern_id,
            &view_state.clipboard,
            paste_tick,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Arrow Up/Down — transpose selected notes ──
    if !view_state.selected_notes.is_empty() {
        let shift = ctx.input(|i| i.modifiers.shift);
        let up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
        let down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
        if up || down {
            let semitones = match (up, shift) {
                (true, false) => Semitones::new(1.0),
                (true, true) => Semitones::new(12.0),
                (false, false) => Semitones::new(-1.0),
                (false, true) => Semitones::new(-12.0),
            };
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                    let mut composite = Vec::new();
                    for note_id in &view_state.selected_notes {
                        if let Some(note) = pattern.note(*note_id) {
                            let old_pitch = note.pitch;
                            if pattern.transpose_note(*note_id, semitones)
                                && let Some(transposed) = pattern.note(*note_id)
                            {
                                composite.push(crate::undo::UndoAction::TransposeNote {
                                    pattern_id: data.pattern_id,
                                    note_id: *note_id,
                                    old_pitch,
                                    new_pitch: transposed.pitch,
                                });
                            }
                        }
                    }
                    if !composite.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::Composite(composite));
                    }
                }
            }
        }
    }

    // ── Space — toggle play/pause ──
    if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
        if is_playing {
            handle.send(EngineCommand::Pause);
        } else {
            handle.send(EngineCommand::Play);
            view_state.auto_follow_playhead = true;
        }
    }

    // ── Step entry mode — keyboard piano inserts notes at cursor ──
    if view_state.step_entry_mode {
        let step_size = if view_state.draw_note_length > 0 {
            SeqDuration(view_state.draw_note_length)
        } else {
            view_state.effective_grid(data.ticks_per_row)
        };

        // Collect pressed key in a single input lock acquisition
        let pressed_note = ctx.input(|i| {
            if i.modifiers.command || i.modifiers.ctrl {
                return None;
            }
            KEY_MAP
                .iter()
                .find(|(key, _)| i.key_pressed(*key))
                .map(|(_, note)| *note)
        });
        let mut inserted = false;
        if let Some(pitch_val) = pressed_note
            && let Some(pitch) = Pitch::new(pitch_val)
            && let mut song_w = song.write()
            && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
        {
            let note_id = pattern.add_note(
                view_state.step_cursor_tick,
                pitch,
                view_state.default_velocity,
            );
            pattern.resize_note(note_id, step_size);
            if let Some(note) = pattern.note(note_id) {
                undo_manager.push(crate::undo::UndoAction::AddNote {
                    pattern_id: data.pattern_id,
                    note: note.into(),
                });
            }
            view_state.selected_notes.clear();
            view_state.selected_notes.insert(note_id);
            preview_note(handle, pitch, view_state.default_velocity);
            inserted = true;
        }
        if inserted {
            view_state.step_cursor_tick = view_state.step_cursor_tick + step_size;
            // Wrap around at pattern end
            if view_state.step_cursor_tick.0 >= data.length_ticks.0 {
                view_state.step_cursor_tick = PatternTick::ZERO;
            }
        }
    }

    keep_open
}

/// Handle mouse clicks and drags in the piano roll.
#[allow(clippy::too_many_arguments)]
fn handle_piano_roll_interaction(
    response: &egui::Response,
    ui: &egui::Ui,
    data: &PianoRollData,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    handle: &mut EngineHandle,
    grid_rect: Rect,
    auto_rect: Option<Rect>,
    auto_y: f32,
    coords: &PianoRollCoords,
    undo_manager: &mut crate::undo::UndoManager,
) {
    // Re-expose the bundled transforms under the names the body already uses, so
    // the handler logic is untouched by the parameter consolidation. The `&dyn
    // Fn` bindings keep the call shape for the sub-helpers that take closures.
    let tick_to_x_impl = |t: PatternTick| coords.tick_to_x(t);
    let pitch_to_y_impl = |p: Pitch| coords.pitch_to_y(p);
    let x_to_tick_impl = |x: f32| coords.x_to_tick(x);
    let y_to_pitch_impl = |y: f32| coords.y_to_pitch(y);
    let tick_to_x: &dyn Fn(PatternTick) -> f32 = &tick_to_x_impl;
    let pitch_to_y: &dyn Fn(Pitch) -> f32 = &pitch_to_y_impl;
    let x_to_tick: &dyn Fn(f32) -> PatternTick = &x_to_tick_impl;
    let y_to_pitch: &dyn Fn(f32) -> Pitch = &y_to_pitch_impl;
    let view_pitch_min = coords.view_pitch_min;
    let view_pitch_max = coords.view_pitch_max;
    let note_row_height = coords.note_row_height;

    let shift_held = ui.input(|i| i.modifiers.shift);

    // ── Automation click handling ──
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        let raw_tick = x_to_tick(pos.x);
        let tick = quantize_tick(raw_tick, data.ticks_per_row);
        let value =
            NormalizedValue::new((1.0 - (pos.y - auto_y) / AUTOMATION_ZONE_HEIGHT).clamp(0.0, 1.0));

        let new_point = AutomationPoint::new(tick, value);
        let curve = new_point.curve;
        {
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                let lane = pattern.get_or_create_automation(target.clone());
                lane.add_point(new_point);
            }
        }
        undo_manager.push(crate::undo::UndoAction::AddAutomationPoint {
            pattern_id: data.pattern_id,
            target,
            tick,
            value,
            curve,
        });
    }

    // ── Automation right-click (delete point) ──
    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        // Find the lane snapshot to hit-test against
        if let Some(lane) = data.automation_lanes.iter().find(|l| l.target == target)
            && let Some(idx) = automation_point_at_pos(lane, pos, tick_to_x, auto_y)
        {
            let pt = &lane.points[idx];
            let point_tick = pt.tick;
            let point_value = pt.value;
            let point_curve = pt.curve;
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                    && let Some(auto_lane) =
                        pattern.automation.iter_mut().find(|l| l.target == target)
                {
                    auto_lane.remove_point(point_tick);
                }
            }
            undo_manager.push(crate::undo::UndoAction::RemoveAutomationPoint {
                pattern_id: data.pattern_id,
                target,
                tick: point_tick,
                value: point_value,
                curve: point_curve,
            });
        }
    }

    // ── Click handling ──
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && grid_rect.contains(pos)
    {
        let vp_min = view_pitch_min;
        let vp_max = view_pitch_max;
        let hit = note_at_pos(
            &data.notes,
            pos,
            tick_to_x,
            pitch_to_y,
            data.length_ticks,
            vp_min,
            vp_max,
            note_row_height,
        );

        match hit {
            Some((note_id, _)) => {
                // Clicked on a note — select it and preview its sound
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    preview_note(handle, note.pitch, note.velocity);
                }
                if shift_held {
                    // Toggle in selection
                    if !view_state.selected_notes.remove(&note_id) {
                        view_state.selected_notes.insert(note_id);
                    }
                } else {
                    view_state.selected_notes.clear();
                    view_state.selected_notes.insert(note_id);
                }
            }
            None => {
                // Clicked on empty space
                if view_state.edit_tool == EditTool::Draw {
                    // Add a new note (only if no note already exists here)
                    let raw_tick = x_to_tick(pos.x);
                    let tick = quantize_tick(raw_tick, data.ticks_per_row);
                    let pitch_val = y_to_pitch(pos.y);

                    if !has_note_at(&data.notes, tick, pitch_val)
                        && let mut song_w = song.write()
                        && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                    {
                        let duration = if view_state.draw_note_length > 0 {
                            SeqDuration(view_state.draw_note_length)
                        } else {
                            SeqDuration((data.ticks_per_row as u32).max(1))
                        };
                        let note_id =
                            pattern.add_note(tick, pitch_val, view_state.default_velocity);
                        pattern.resize_note(note_id, duration);
                        if let Some(note) = pattern.note(note_id) {
                            undo_manager.push(crate::undo::UndoAction::AddNote {
                                pattern_id: data.pattern_id,
                                note: note.into(),
                            });
                        }
                        view_state.selected_notes.clear();
                        view_state.selected_notes.insert(note_id);
                        preview_note(handle, pitch_val, view_state.default_velocity);
                    }
                } else if !shift_held {
                    // Select tool on empty space — clear selection
                    view_state.selected_notes.clear();
                }
            }
        }
    }

    // ── Automation drag start ──
    if response.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        if let Some(lane) = data.automation_lanes.iter().find(|l| l.target == target)
            && let Some(idx) = automation_point_at_pos(lane, pos, tick_to_x, auto_y)
        {
            let pt = &lane.points[idx];
            view_state.drag = Some(DragState::DragAutomationPoint {
                target,
                original_tick: pt.tick,
                original_value: pt.value,
                current_tick: pt.tick,
                current_value: pt.value,
            });
        }
    }

    // ── Velocity drag start ──
    if response.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
    {
        let vel_y = grid_rect.max.y;
        let vel_rect = Rect::from_min_size(
            Pos2::new(grid_rect.min.x, vel_y),
            Vec2::new(grid_rect.width(), VELOCITY_ZONE_HEIGHT),
        );
        if vel_rect.contains(pos) {
            view_state.drag = Some(DragState::DragVelocity {
                last_note_id: None,
                initial_velocities: std::collections::HashMap::new(),
            });
        }
    }

    // ── Drag start ──
    // Use press_origin (where the click started) for hit-testing, not current pointer pos.
    // Without this, dragging straight up/down misses the note because the pointer
    // has already left the note rect by the time the drag threshold is reached.
    if response.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && grid_rect.contains(pos)
    {
        let vp_min = view_pitch_min;
        let vp_max = view_pitch_max;
        let hit = note_at_pos(
            &data.notes,
            pos,
            tick_to_x,
            pitch_to_y,
            data.length_ticks,
            vp_min,
            vp_max,
            note_row_height,
        );

        match hit {
            Some((note_id, HitZone::Body)) => {
                // Start moving the note
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    // If note has no explicit duration, lock it before move
                    // so the visual length is preserved
                    if note.end_tick.is_none() {
                        let implied_dur = (data.length_ticks.as_pattern_tick() - note.start_tick)
                            .max(SeqDuration(1));
                        {
                            let mut song_w = song.write();
                            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                                pattern.resize_note(note_id, implied_dur);
                            }
                        }
                    }
                    // Calculate where on the note the user grabbed
                    let grab_tick = x_to_tick(pos.x);
                    let grab_offset = grab_tick - note.start_tick;
                    view_state.drag = Some(DragState::MoveNote {
                        note_id,
                        original_tick: note.start_tick,
                        original_pitch: note.pitch,
                        current_tick: note.start_tick,
                        current_pitch: note.pitch,
                        grab_offset_ticks: grab_offset,
                    });
                    // Ensure note is selected
                    if !shift_held {
                        view_state.selected_notes.clear();
                    }
                    view_state.selected_notes.insert(note_id);
                }
            }
            Some((note_id, HitZone::RightEdge)) => {
                // Start resizing the note
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    let end_tick = note.end_tick.unwrap_or(data.length_ticks.as_pattern_tick());
                    view_state.drag = Some(DragState::ResizeNote {
                        note_id,
                        original_end_tick: end_tick,
                        current_end_tick: end_tick,
                    });
                }
            }
            None => match view_state.edit_tool {
                EditTool::Select => {
                    // Start selection rectangle
                    view_state.drag = Some(DragState::SelectRect {
                        start_pos: pos,
                        current_pos: pos,
                    });
                    if !shift_held {
                        view_state.selected_notes.clear();
                    }
                }
                EditTool::Draw => {
                    // Start drawing a new note by dragging (only if position is free)
                    let raw_tick = x_to_tick(pos.x);
                    let start_tick = quantize_tick(raw_tick, data.ticks_per_row);
                    let pitch = y_to_pitch(pos.y);
                    if !has_note_at(&data.notes, start_tick, pitch) {
                        let end_tick =
                            PatternTick(start_tick.0 + (data.ticks_per_row as u32).max(1));
                        view_state.drag = Some(DragState::DrawNote {
                            start_tick,
                            pitch,
                            current_end_tick: end_tick,
                        });
                    }
                }
            },
        }
    }

    // ── Drag update ──
    // Use pointer_latest_pos for smooth tracking during drags
    if response.dragged()
        && let Some(pos) = ui.pointer_latest_pos()
    {
        match &mut view_state.drag {
            Some(DragState::MoveNote {
                current_tick,
                current_pitch,
                grab_offset_ticks,
                ..
            }) => {
                let raw_tick = PatternTick(x_to_tick(pos.x).0.saturating_sub(grab_offset_ticks.0));
                *current_tick = quantize_tick(raw_tick, data.ticks_per_row);
                *current_pitch = y_to_pitch(pos.y);
            }
            Some(DragState::ResizeNote {
                current_end_tick, ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                let quantized = quantize_tick(raw_tick, data.ticks_per_row);
                *current_end_tick = PatternTick(quantized.0.max(1));
            }
            Some(DragState::DrawNote {
                start_tick,
                current_end_tick,
                ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                let quantized = quantize_tick(raw_tick, data.ticks_per_row);
                // End tick must be at least one row past start
                *current_end_tick = PatternTick(
                    quantized
                        .0
                        .max(start_tick.0 + (data.ticks_per_row as u32).max(1)),
                );
            }
            Some(DragState::SelectRect { current_pos, .. }) => {
                *current_pos = pos;
            }
            Some(DragState::DragAutomationPoint {
                current_tick,
                current_value,
                ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                *current_tick = quantize_tick(raw_tick, data.ticks_per_row);
                *current_value = NormalizedValue::new(
                    (1.0 - (pos.y - auto_y) / AUTOMATION_ZONE_HEIGHT).clamp(0.0, 1.0),
                );
            }
            Some(DragState::DragVelocity { .. }) => {
                // Velocity painting handled in real-time below
            }
            Some(DragState::DragPlacement { .. })
            | Some(DragState::ResizePlacement { .. })
            | None => {}
        }
    }

    // ── Velocity drag: apply velocity change in real-time ──
    if let Some(DragState::DragVelocity {
        last_note_id,
        initial_velocities,
    }) = &mut view_state.drag
        && let Some(pos) = ui.pointer_latest_pos()
    {
        // vel_y is grid_rect.max.y, VELOCITY_ZONE_HEIGHT below
        let vel_y = grid_rect.max.y;
        let new_vel = (1.0 - (pos.y - vel_y) / VELOCITY_ZONE_HEIGHT).clamp(0.01, 1.0);
        // Find note at this x position
        let click_tick = x_to_tick(pos.x);
        let nearest_note = data.notes.iter().min_by_key(|n| {
            let mid = n.start_tick.0 + n.end_tick.map_or(0, |e| (e.0 - n.start_tick.0) / 2);
            (mid as i64 - click_tick.0 as i64).unsigned_abs()
        });
        if let Some(note) = nearest_note {
            let note_x = tick_to_x(note.start_tick);
            if (note_x - pos.x).abs() < 15.0 {
                *last_note_id = Some(note.note_id);
                initial_velocities
                    .entry(note.note_id)
                    .or_insert(note.velocity);
                {
                    let mut song_w = song.write();
                    if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                        pattern.set_note_velocity(note.note_id, Velocity::new(new_vel));
                    }
                }
            }
        }
    }

    // ── Drag end ──
    if response.drag_stopped()
        && let Some(drag) = view_state.drag.take()
    {
        match drag {
            DragState::MoveNote {
                note_id,
                original_tick,
                original_pitch,
                current_tick,
                current_pitch,
                ..
            } => {
                // Apply move to song
                if (current_tick != original_tick || current_pitch != original_pitch)
                    && let mut song_w = song.write()
                    && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                {
                    let mut composite = Vec::new();
                    if current_tick != original_tick {
                        pattern.move_note(note_id, current_tick);
                        composite.push(crate::undo::UndoAction::MoveNote {
                            pattern_id: data.pattern_id,
                            note_id,
                            old_start: original_tick,
                            new_start: current_tick,
                        });
                    }
                    if current_pitch != original_pitch {
                        #[allow(clippy::cast_precision_loss)]
                        let delta =
                            current_pitch.as_midi() as f32 - original_pitch.as_midi() as f32;
                        pattern.transpose_note(note_id, Semitones::new(delta));
                        composite.push(crate::undo::UndoAction::TransposeNote {
                            pattern_id: data.pattern_id,
                            note_id,
                            old_pitch: original_pitch,
                            new_pitch: current_pitch,
                        });
                    }
                    if !composite.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::Composite(composite));
                    }
                }
            }
            DragState::ResizeNote {
                note_id,
                original_end_tick,
                current_end_tick,
                ..
            } => {
                // Apply resize to song
                if current_end_tick != original_end_tick
                    && let Some(note) = data.notes.iter().find(|n| n.note_id == note_id)
                {
                    let new_duration = (current_end_tick - note.start_tick).max(SeqDuration(1));
                    let old_duration = note.end_tick.map(|e| e - note.start_tick);
                    {
                        let mut song_w = song.write();
                        if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                            pattern.resize_note(note_id, new_duration);
                            undo_manager.push(crate::undo::UndoAction::ResizeNote {
                                pattern_id: data.pattern_id,
                                note_id,
                                old_duration: old_duration.map(|d| SeqDuration(d.0)),
                                new_duration: Some(new_duration),
                            });
                        }
                    }
                }
            }
            DragState::DrawNote {
                start_tick,
                pitch,
                current_end_tick,
            } => {
                // Create the note with the dragged duration (only if no duplicate)
                let duration = (current_end_tick - start_tick).max(SeqDuration(1));
                if !has_note_at(&data.notes, start_tick, pitch)
                    && let mut song_w = song.write()
                    && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                {
                    let note_id = pattern.add_note(start_tick, pitch, view_state.default_velocity);
                    pattern.resize_note(note_id, duration);
                    if let Some(added_note) = pattern.note(note_id) {
                        undo_manager.push(crate::undo::UndoAction::AddNote {
                            pattern_id: data.pattern_id,
                            note: added_note.into(),
                        });
                    }
                    view_state.selected_notes.clear();
                    view_state.selected_notes.insert(note_id);
                }
            }
            DragState::SelectRect {
                start_pos,
                current_pos,
                ..
            } => {
                // Resolve selection rectangle to notes
                let sel_rect = Rect::from_two_pos(start_pos, current_pos);
                let tick_start = x_to_tick(sel_rect.min.x);
                let tick_end = x_to_tick(sel_rect.max.x);
                let pitch_top = y_to_pitch(sel_rect.min.y);
                let pitch_bottom = y_to_pitch(sel_rect.max.y);
                let p_min = pitch_bottom.min(pitch_top);
                let p_max = pitch_bottom.max(pitch_top);

                for note in &data.notes {
                    let note_end = note.end_tick.unwrap_or(data.length_ticks.as_pattern_tick());
                    if note.start_tick < tick_end
                        && note_end > tick_start
                        && note.pitch >= p_min
                        && note.pitch <= p_max
                    {
                        view_state.selected_notes.insert(note.note_id);
                    }
                }
            }
            DragState::DragAutomationPoint {
                target,
                original_tick,
                original_value,
                current_tick,
                current_value,
            } => {
                // Apply automation point move
                if current_tick != original_tick
                    || (current_value.as_f32() - original_value.as_f32()).abs() > f32::EPSILON
                {
                    let new_point = AutomationPoint::new(current_tick, current_value);
                    let curve = new_point.curve;
                    {
                        let mut song_w = song.write();
                        if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                            let lane = pattern.get_or_create_automation(target.clone());
                            lane.remove_point(original_tick);
                            lane.add_point(new_point);
                        }
                    }
                    undo_manager.push(crate::undo::UndoAction::MoveAutomationPoint {
                        pattern_id: data.pattern_id,
                        target,
                        old_tick: original_tick,
                        old_value: original_value,
                        new_tick: current_tick,
                        new_value: current_value,
                        curve,
                    });
                }
            }
            // DragVelocity — applied in real-time; on release, capture the
            // before/after pairs into a single composite undo entry.
            DragState::DragVelocity {
                initial_velocities, ..
            } => {
                if !initial_velocities.is_empty() {
                    let mut changes: Vec<(NoteId, Velocity, Velocity)> = Vec::new();
                    {
                        let song_r = song.read();
                        if let Some(pattern) = song_r.pattern(data.pattern_id) {
                            for (note_id, old_vel) in &initial_velocities {
                                if let Some(note) = pattern.note(*note_id)
                                    && note.velocity != *old_vel
                                {
                                    changes.push((*note_id, *old_vel, note.velocity));
                                }
                            }
                        }
                    }
                    if !changes.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::SetVelocitiesBatch {
                            pattern_id: data.pattern_id,
                            changes,
                        });
                    }
                }
            }
            // Placement drag + resize are handled in the arrangement view.
            DragState::DragPlacement { .. } | DragState::ResizePlacement { .. } => {}
        }
    }
}

/// Find automation point at the given position (within hit radius).
fn automation_point_at_pos(
    lane: &AutomationLaneSnapshot,
    pos: Pos2,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    auto_y: f32,
) -> Option<usize> {
    let value_to_y =
        |val: f32| -> f32 { auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - val.clamp(0.0, 1.0)) };

    for (i, pt) in lane.points.iter().enumerate() {
        let px = tick_to_x(pt.tick);
        let py = value_to_y(pt.value.as_f32());
        let dist = ((pos.x - px).powi(2) + (pos.y - py).powi(2)).sqrt();
        if dist <= AUTOMATION_HIT_RADIUS {
            return Some(i);
        }
    }
    None
}

/// Draw the automation zone below the velocity zone.
#[allow(clippy::too_many_arguments)]
fn draw_automation_zone(
    painter: &egui::Painter,
    data: &PianoRollData,
    view_state: &SequencerViewState,
    selected_target: &AutomationTarget,
    grid_x: f32,
    auto_y: f32,
    grid_width: f32,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    t: &crate::gui::theme::Theme,
) {
    let auto_color = AUTOMATION_ORANGE; // Orange

    // Background
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(grid_x, auto_y),
            Vec2::new(grid_width, AUTOMATION_ZONE_HEIGHT),
        ),
        0.0,
        AUTOMATION_ZONE_BG,
    );

    // Separator line
    painter.line_segment(
        [
            Pos2::new(grid_x, auto_y),
            Pos2::new(grid_x + grid_width, auto_y),
        ],
        Stroke::new(1.0, t.colors.border),
    );

    // Label
    painter.text(
        Pos2::new(grid_x - KEY_WIDTH + 2.0, auto_y + 2.0),
        egui::Align2::LEFT_TOP,
        "AUTO",
        egui::FontId::proportional(9.0),
        t.colors.text_dim,
    );

    // Reference lines (25%, 50%, 75%)
    for frac in [0.25, 0.5, 0.75] {
        let ry = auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - frac);
        painter.line_segment(
            [Pos2::new(grid_x, ry), Pos2::new(grid_x + grid_width, ry)],
            Stroke::new(0.3, t.colors.border.gamma_multiply(0.3)),
        );
    }

    // Coordinate helpers
    let value_to_y =
        |val: f32| -> f32 { auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - val.clamp(0.0, 1.0)) };

    // Find the lane matching the selected target
    let lane = data
        .automation_lanes
        .iter()
        .find(|l| l.target == *selected_target);

    if let Some(lane) = lane {
        let points = &lane.points;

        if !points.is_empty() {
            // Draw flat extension before first point
            if let Some(first) = points.first() {
                let first_x = tick_to_x(first.tick);
                if first_x > grid_x {
                    let y = value_to_y(first.value.as_f32());
                    painter.line_segment(
                        [Pos2::new(grid_x, y), Pos2::new(first_x, y)],
                        Stroke::new(1.0, auto_color.gamma_multiply(0.5)),
                    );
                }
            }

            // Draw curves between consecutive points
            for [from, to] in points.array_windows() {
                let x_start = tick_to_x(from.tick);
                let x_end = tick_to_x(to.tick);
                let pixel_width = (x_end - x_start).max(1.0);

                // Sample the curve pixel by pixel
                let steps = (pixel_width as u32).max(2);
                let mut prev_pos = Pos2::new(x_start, value_to_y(from.value.as_f32()));

                for step in 1..=steps {
                    #[allow(clippy::cast_precision_loss)]
                    let frac = step as f32 / steps as f32;
                    let x = x_start + frac * (x_end - x_start);
                    let val =
                        from.curve
                            .interpolate(from.value, to.value, NormalizedValue::new(frac));
                    let y = value_to_y(val.as_f32());
                    let cur_pos = Pos2::new(x, y);

                    painter.line_segment([prev_pos, cur_pos], Stroke::new(1.5, auto_color));
                    prev_pos = cur_pos;
                }
            }

            // Draw flat extension after last point
            if let Some(last) = points.last() {
                let last_x = tick_to_x(last.tick);
                let grid_end_x = grid_x + grid_width;
                if last_x < grid_end_x {
                    let y = value_to_y(last.value.as_f32());
                    painter.line_segment(
                        [Pos2::new(last_x, y), Pos2::new(grid_end_x, y)],
                        Stroke::new(1.0, auto_color.gamma_multiply(0.5)),
                    );
                }
            }

            // Draw points
            for pt in points {
                let px = tick_to_x(pt.tick);
                let py = value_to_y(pt.value.as_f32());
                painter.circle_filled(Pos2::new(px, py), AUTOMATION_POINT_RADIUS, auto_color);
                painter.circle_stroke(
                    Pos2::new(px, py),
                    AUTOMATION_POINT_RADIUS,
                    Stroke::new(1.0, Color32::WHITE),
                );
            }
        }
    }

    // Draw drag preview ghost point
    if let Some(DragState::DragAutomationPoint {
        current_tick,
        current_value,
        target,
        ..
    }) = &view_state.drag
        && target == selected_target
    {
        let px = tick_to_x(*current_tick);
        let py = value_to_y(current_value.as_f32());
        painter.circle_filled(
            Pos2::new(px, py),
            AUTOMATION_POINT_RADIUS + 1.0,
            AUTOMATION_ORANGE_FILL,
        );
        painter.circle_stroke(
            Pos2::new(px, py),
            AUTOMATION_POINT_RADIUS + 1.0,
            Stroke::new(1.0, Color32::WHITE),
        );
    }
}

/// Play a short note preview (instant note-on + note-off).
fn preview_note(handle: &mut EngineHandle, pitch: Pitch, velocity: synth_core::Velocity) {
    handle.note_on(MidiNote::new(pitch.as_midi()), velocity);
    handle.note_off(MidiNote::new(pitch.as_midi()));
}

/// Build a one-shot cache of instrument id → parsed colour. Use once per
/// draw call before iterating notes or mini-notes so per-element lookup
/// is O(1) and the hex parse only happens once per instrument.
fn build_instrument_colour_cache(
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
) -> std::collections::HashMap<u64, Color32> {
    instruments
        .iter()
        .filter_map(|inst| {
            inst.color
                .as_ref()
                .and_then(|hex| crate::gui::patch_editor::parse_hex_color(hex))
                .map(|c| (inst.id.0, c))
        })
        .collect()
}

/// Look up a cached instrument colour, falling back to `fallback` when
/// the instrument has no colour set.
fn cached_instrument_color(
    cache: &std::collections::HashMap<u64, Color32>,
    seq_id: SeqInstrumentId,
    fallback: Color32,
) -> Color32 {
    cache.get(&(seq_id.0 as u64)).copied().unwrap_or(fallback)
}

/// Send the engine an `ArmRecord` command for the given pattern, using the
/// placement on the user's selected track (or the first available
/// placement) as the recording region. Returns true if arm was sent.
fn arm_recording_for_pattern(
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    pattern_id: PatternId,
) -> bool {
    // Look up either a real placement (placed pattern) or synthesize one
    // against pattern-tick 0 (orphan pattern). For orphans we also need to
    // tell the engine to enter preview-mode so playback loops the pattern
    // instead of the arrangement.
    let target = song.try_read().and_then(|s| {
        let mut best: Option<(Tick, SeqDuration, SeqDuration, TrackId)> = None;
        for p in s.arrangement() {
            if p.pattern_id == pattern_id {
                let pat = s.pattern(pattern_id)?;
                let tpb = SeqDuration(s.time_signature_at(p.start).ticks_per_bar());
                let is_selected_track = view_state.selected_track == Some(p.track_id);
                if best.is_none() || is_selected_track {
                    best = Some((p.start, pat.length, tpb, p.track_id));
                }
                if is_selected_track {
                    break;
                }
            }
        }
        if let Some(placed) = best {
            return Some((placed, false));
        }
        // Orphan: synthesize bounds against pattern-local tick 0.
        let pat = s.pattern(pattern_id)?;
        let tpb = SeqDuration(s.time_signature_at(Tick::ZERO).ticks_per_bar());
        Some(((Tick::ZERO, pat.length, tpb, TrackId(0)), true))
    });
    let Some(((region_start, pattern_length, ticks_per_bar, track_id), is_orphan)) = target else {
        return false;
    };
    view_state.recording_pattern = Some(pattern_id);
    if is_orphan {
        handle.send(EngineCommand::SetPreviewPattern(Some((
            pattern_id,
            view_state.selected_instrument,
        ))));
    } else {
        handle.send(EngineCommand::SetPreviewPattern(None));
    }
    handle.send(EngineCommand::ArmRecord {
        pattern_id,
        track_id,
        region_start,
        pattern_length,
        ticks_per_bar,
        quantize_grid: SeqDuration(view_state.record_quantize),
        overdub: view_state.overdub,
    });
    true
}

/// Apply a closure that mutates a set of notes in a pattern, capturing
/// per-note start/duration/velocity before and after the call and pushing
/// a single composite undo entry containing MoveNote / ResizeNote /
/// SetNoteVelocity per note that actually changed.
fn batch_transform_with_undo<F>(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
    note_ids: &HashSet<NoteId>,
    undo_manager: &mut crate::undo::UndoManager,
    apply: F,
) where
    F: FnOnce(&mut synth_sequencer::Pattern),
{
    if note_ids.is_empty() {
        return;
    }
    type Snapshot = (PatternTick, Option<SeqDuration>, Velocity);
    let mut before: std::collections::HashMap<NoteId, Snapshot> = std::collections::HashMap::new();
    {
        let mut song_w = song.write();
        let Some(pattern) = song_w.pattern_mut(pattern_id) else {
            return;
        };
        for nid in note_ids {
            if let Some(note) = pattern.note(*nid) {
                before.insert(*nid, (note.start, note.duration, note.velocity));
            }
        }
        apply(pattern);
        let mut composite: Vec<crate::undo::UndoAction> = Vec::new();
        for (nid, (old_start, old_dur, old_vel)) in &before {
            if let Some(note) = pattern.note(*nid) {
                if note.start != *old_start {
                    composite.push(crate::undo::UndoAction::MoveNote {
                        pattern_id,
                        note_id: *nid,
                        old_start: *old_start,
                        new_start: note.start,
                    });
                }
                if note.duration != *old_dur {
                    composite.push(crate::undo::UndoAction::ResizeNote {
                        pattern_id,
                        note_id: *nid,
                        old_duration: *old_dur,
                        new_duration: note.duration,
                    });
                }
                if note.velocity != *old_vel {
                    composite.push(crate::undo::UndoAction::SetNoteVelocity {
                        pattern_id,
                        note_id: *nid,
                        old_velocity: *old_vel,
                        new_velocity: note.velocity,
                    });
                }
            }
        }
        if !composite.is_empty() {
            undo_manager.push(crate::undo::UndoAction::Composite(composite));
        }
    }
}

fn delete_selected_notes(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
    selected: &mut HashSet<NoteId>,
    undo_manager: &mut crate::undo::UndoManager,
) {
    if selected.is_empty() {
        return;
    }
    {
        let mut song_w = song.write();
        if let Some(pattern) = song_w.pattern_mut(pattern_id) {
            let mut composite = Vec::new();
            for note_id in selected.iter() {
                if let Some(note) = pattern.note(*note_id) {
                    composite.push(crate::undo::UndoAction::RemoveNote {
                        pattern_id,
                        note: note.into(),
                    });
                }
                pattern.remove_note(*note_id);
            }
            if !composite.is_empty() {
                undo_manager.push(crate::undo::UndoAction::Composite(composite));
            }
        }
    }
    selected.clear();
}

/// Copy selected notes into the clipboard.
fn copy_selected_notes(
    data: &PianoRollData,
    selected: &HashSet<NoteId>,
    clipboard: &mut Clipboard,
) {
    clipboard.notes.clear();
    clipboard.selection_width = SeqDuration(0);

    let selected_notes: Vec<&PianoRollNote> = data
        .notes
        .iter()
        .filter(|n| selected.contains(&n.note_id))
        .collect();

    if selected_notes.is_empty() {
        return;
    }

    let min_tick = selected_notes
        .iter()
        .map(|n| n.start_tick)
        .min()
        .unwrap_or(PatternTick::ZERO);

    let max_end = selected_notes
        .iter()
        .map(|n| {
            n.end_tick
                .unwrap_or(PatternTick(n.start_tick.0 + data.ticks_per_row as u32))
        })
        .max()
        .unwrap_or(min_tick);

    clipboard.selection_width = max_end - min_tick;

    for note in &selected_notes {
        clipboard.notes.push(ClipboardNote {
            tick_offset: note.start_tick - min_tick,
            pitch: note.pitch,
            velocity: note.velocity,
            duration: note.end_tick.map(|e| e - note.start_tick),
        });
    }
}

/// Paste clipboard notes at a given tick position into the pattern.
fn paste_clipboard_notes(
    song: &Arc<RwLock<Song>>,
    pattern_id: PatternId,
    clipboard: &Clipboard,
    paste_tick: PatternTick,
    selected: &mut HashSet<NoteId>,
    undo_manager: &mut crate::undo::UndoManager,
) {
    if clipboard.notes.is_empty() {
        return;
    }

    selected.clear();

    {
        let mut song_w = song.write();
        if let Some(pattern) = song_w.pattern_mut(pattern_id) {
            let mut composite = Vec::new();
            for cn in &clipboard.notes {
                let tick = paste_tick + cn.tick_offset;
                let note_id = pattern.add_note(tick, cn.pitch, cn.velocity);
                if let Some(dur) = cn.duration {
                    pattern.resize_note(note_id, dur);
                }
                if let Some(note) = pattern.note(note_id) {
                    composite.push(crate::undo::UndoAction::AddNote {
                        pattern_id,
                        note: note.into(),
                    });
                }
                selected.insert(note_id);
            }
            if !composite.is_empty() {
                undo_manager.push(crate::undo::UndoAction::Composite(composite));
            }
        }
    }
}

/// Map velocity (0.0-1.0) to a color (green → yellow → red).
fn velocity_color(vel: f32) -> Color32 {
    if vel < 0.5 {
        let t = vel * 2.0;
        Color32::from_rgb(
            (80.0 + t * 175.0) as u8,
            (180.0 + t * 40.0) as u8,
            (80.0 - t * 40.0) as u8,
        )
    } else {
        let t = (vel - 0.5) * 2.0;
        Color32::from_rgb(
            (255.0) as u8,
            (220.0 - t * 140.0) as u8,
            (40.0 - t * 40.0) as u8,
        )
    }
}

// ============================================================================
// SEQUENCER VIEW (MAIN ENTRY)
// ============================================================================

/// Draw the full sequencer view (transport + arrangement + piano roll).
pub(crate) fn draw_sequencer_view(
    ui: &mut egui::Ui,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    undo_manager: &mut crate::undo::UndoManager,
) {
    let ctx = ui.ctx().clone();

    // Validate selected_instrument: if it no longer matches any instrument
    // (e.g. the user removed instruments while in a different view), fall
    // back to the first available one so new notes route to a real target.
    if !instruments
        .iter()
        .any(|inst| inst.id.0 == view_state.selected_instrument.0 as u64)
        && let Some(first) = instruments.first()
    {
        view_state.selected_instrument = SeqInstrumentId::new(first.id.0 as u16);
    }

    // Transport bar at the top
    let is_playing = egui::Panel::top("sequencer_transport")
        .show_inside(ui, |ui| draw_transport_bar(ui, handle, song, view_state))
        .inner;

    // Request repaint during playback for smooth position updates. While
    // stopped, keep repainting for a few frames after a transport jump / seek /
    // stop so the timeline catches the engine's async playhead update and the
    // off-screen follow can scroll the marker into view.
    if is_playing {
        ctx.request_repaint();
    } else if view_state.follow_settle_frames > 0 {
        view_state.follow_settle_frames -= 1;
        ctx.request_repaint();
    }

    // Read current playhead position (atomic, lock-free)
    let current_tick = handle.state.transport.get_ticks();

    // Collect song data (short read-lock, then release before rendering)
    let arrangement_data = collect_arrangement_data(song);

    // Pattern follow: when playing with a selected track, auto-switch to
    // whichever pattern is currently playing on that track.
    let playing_pattern_on_selected_track = if let Some(track_id) = view_state.selected_track
        && let Some(ad) = &arrangement_data
    {
        ad.placements.iter().find_map(|p| {
            if p.track_id == track_id && current_tick >= p.start_tick && current_tick < p.end_tick {
                Some(p.pattern_id)
            } else {
                None
            }
        })
    } else {
        None
    };

    if is_playing
        && let Some(pattern_id) = playing_pattern_on_selected_track
        && view_state.opened_pattern != Some(pattern_id)
    {
        view_state.opened_pattern = Some(pattern_id);
        view_state.selected_notes.clear();
        view_state.drag = None;
    }

    // Piano roll bottom panel (if a pattern is open)
    if let Some(pattern_id) = view_state.opened_pattern {
        let piano_roll_data = collect_piano_roll_data(song, pattern_id);

        // Calculate pattern-relative playhead tick. Preview-mode wins:
        // current_tick % pattern.length. Otherwise look for an active
        // placement in the arrangement snapshot.
        let preview_pid = handle.state.transport.preview_pattern();
        let pattern_playhead_tick: Option<PatternTick> = if preview_pid == Some(pattern_id) {
            song.try_read()
                .and_then(|s| s.pattern(pattern_id).map(|p| p.length))
                .map(|len| {
                    let length = u64::from(len.0.max(1));
                    #[allow(clippy::cast_possible_truncation)]
                    PatternTick((current_tick % length) as u32)
                })
        } else {
            arrangement_data.as_ref().and_then(|ad| {
                ad.placements.iter().find_map(|p| {
                    if p.pattern_id == pattern_id
                        && current_tick >= p.start_tick
                        && current_tick < p.end_tick
                    {
                        #[allow(clippy::cast_possible_truncation)]
                        Some(PatternTick((current_tick - p.start_tick) as u32))
                    } else {
                        None
                    }
                })
            })
        };

        // Use ~50% of available height for piano roll, with generous max
        let available_height = ctx.content_rect().height();
        let default_height = (available_height * 0.5).max(MIN_PIANO_ROLL_HEIGHT);

        egui::Panel::bottom("piano_roll")
            .resizable(true)
            .default_size(default_height)
            .min_size(150.0)
            .max_size(available_height - 100.0)
            .show_inside(ui, |ui| {
                if let Some(data) = &piano_roll_data {
                    if !draw_piano_roll(
                        ui,
                        data,
                        pattern_playhead_tick,
                        is_playing,
                        handle,
                        song,
                        view_state,
                        instruments,
                        undo_manager,
                    ) {
                        view_state.close_piano_roll();
                        // Closing the piano roll resumes normal multi-pattern playback.
                        handle.send(EngineCommand::SetSoloPattern(None));
                        handle.send(EngineCommand::SetPreviewPattern(None));
                    }
                } else {
                    // No snapshot this frame: either the pattern was deleted or
                    // the Song lock was momentarily unavailable. Only tear down
                    // the roll (and exit preview/solo) when the pattern is truly
                    // gone — a transient lock miss must not kill an in-progress
                    // recording. If even this read fails, keep the roll open and
                    // skip the frame.
                    let truly_gone = song
                        .try_read()
                        .is_some_and(|s| s.pattern(pattern_id).is_none());
                    if truly_gone {
                        ui.label(RichText::new("Pattern not found").color(theme().colors.text_dim));
                        view_state.close_piano_roll();
                        handle.send(EngineCommand::SetSoloPattern(None));
                        handle.send(EngineCommand::SetPreviewPattern(None));
                    }
                }
            });
    }

    // Main content: arrangement view
    egui::CentralPanel::default().show_inside(ui, |ui| {
        if let Some(data) = &arrangement_data {
            if let Some(pattern_id) = draw_arrangement(
                ui,
                data,
                current_tick,
                is_playing,
                handle,
                song,
                view_state,
                instruments,
                undo_manager,
            ) {
                // Clear selection when switching patterns
                if view_state.opened_pattern != Some(pattern_id) {
                    view_state.selected_notes.clear();
                    view_state.drag = None;
                }
                view_state.opened_pattern = Some(pattern_id);
            }
        } else {
            ui.label(RichText::new("Song locked...").color(theme().colors.text_dim));
        }
    });
}

/// Convert a sequencer track color to an egui Color32.
pub(crate) fn track_color_to_egui(color: synth_sequencer::TrackColor) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}
