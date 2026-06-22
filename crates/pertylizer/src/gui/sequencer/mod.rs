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
use synth_engine::{EngineCommand, EngineHandle, InstrumentId, RecordingState};
use synth_sequencer::{
    AutoInstrumentParam, AutomationPoint, AutomationTarget, CurveType, Duration as SeqDuration,
    ExpansionBuffer, Glide, GlideFrom, GlideInterp, Note, NoteExpression, NoteId, NoteLane,
    NoteName, NoteProcessor, Ornament, PatternId, PatternTick, Pitch, SeqInstrumentId, Song, Tick,
    TimeSignature, TrackId, Velocity, Vibrato, VibratoShape,
};

use crate::gui::input::KEY_MAP;
use crate::gui::theme::theme;
use crate::gui::widgets::toggle_button;

mod arrangement;
mod automation;
mod note_fx;
mod ornament;
mod piano_roll;
mod tracker;
mod transport;
use arrangement::{collect_arrangement_data, draw_arrangement};
use automation::{automation_point_at_pos, draw_automation_zone};
pub(crate) use note_fx::draw_note_fx_panel;
pub(crate) use ornament::{
    OrnamentEdit, draw_ornament_popup, ornament_detail, ornament_summary, ornament_tag,
};
pub(crate) use piano_roll::{collect_piano_roll_data, draw_piano_roll};
use piano_roll::{draw_automation_target_selector, draw_pattern_instrument_transport};
pub(crate) use tracker::draw_tracker;
use transport::{draw_ruler_labels, draw_transport_bar};

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
    /// Tracker-only view state (cursor, entry buffer, column count, Expr toggle,
    /// hovered row) — grouped; see `tracker::TrackerViewState`.
    tracker: tracker::TrackerViewState,
    /// Whether the right-docked Note FX rack inspector is shown for the opened
    /// pattern (toggled by the "Note FX" button in the piano-roll toolbar or the
    /// pattern view's editor-mode row).
    pub(crate) note_fx_panel_open: bool,
    /// Pre-edit snapshot of a note processor captured when an edit gesture begins
    /// (pattern, rack index, prior config), so a knob/slider drag collapses into
    /// a single `SetNoteProcessorConfig` undo entry on release. Discrete edits
    /// (combos, toggles) capture and finalize in the same frame.
    note_fx_edit_drag_start: Option<(PatternId, usize, NoteProcessor)>,
    /// Open per-note ornament editor popup (target note + baseline + working
    /// copy). `None` when the popup is closed; one coalesced undo entry is pushed
    /// when it closes.
    editing_ornament: Option<OrnamentEdit>,
    /// Whether the piano roll paints the note-processor expansion as faint ghost
    /// notes behind the source notes ("Ghosts" toggle).
    show_note_fx_ghosts: bool,
    /// Cached ghost-preview expansion (see [`GhostCache`]).
    ghost_cache: Option<GhostCache>,
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
            tracker: tracker::TrackerViewState::default(),
            note_fx_panel_open: false,
            note_fx_edit_drag_start: None,
            editing_ornament: None,
            show_note_fx_ghosts: false,
            ghost_cache: None,
        }
    }
}

impl SequencerViewState {
    /// Ghost-preview notes for `pattern_id` (the note-processor expansion). Reads
    /// a cached result, recomputing the per-tick sweep only when the source notes
    /// or rack changed. Returns an owned copy for the frame's painter; empty on a
    /// lock miss or when the pattern has nothing to expand.
    fn ghost_notes(&mut self, song: &Arc<RwLock<Song>>, pattern_id: PatternId) -> Vec<GhostNote> {
        let Some(s) = song.try_read() else {
            return self
                .ghost_cache
                .as_ref()
                .map_or_else(Vec::new, |c| c.ghosts.clone());
        };
        let Some(pattern) = s.pattern(pattern_id) else {
            self.ghost_cache = None;
            return Vec::new();
        };
        let notes = pattern.notes();
        let processors = pattern.processors();
        let fresh = self.ghost_cache.as_ref().is_some_and(|c| {
            c.pattern_id == pattern_id
                && c.length == pattern.length
                && c.notes.as_slice() == notes
                && c.processors.as_slice() == processors
        });
        if !fresh {
            self.ghost_cache = Some(GhostCache {
                pattern_id,
                length: pattern.length,
                notes: notes.to_vec(),
                processors: processors.to_vec(),
                ghosts: compute_ghosts(pattern, s.tempo_at(synth_sequencer::Tick(0))),
            });
        }
        self.ghost_cache
            .as_ref()
            .map_or_else(Vec::new, |c| c.ghosts.clone())
    }

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
/// Faint lavender for the note-processor ghost-preview overlay (drawn behind the
/// source notes).
const GHOST_NOTE_COLOR: Color32 = Color32::from_rgba_unmultiplied_const(170, 140, 230, 50);
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
    /// The note's per-note ornament, if any (drawn as a head marker in the piano
    /// roll, as a tag in the tracker ornament column).
    ornament: Option<Ornament>,
    /// Stored voice/column lane (the tracker's source of truth for which voice
    /// column a note lives in). The piano roll ignores it.
    lane: NoteLane,
}

/// One expanded note in the ghost-preview overlay (note-processor output drawn
/// faintly behind the source notes). The start tick is explicit here (unlike
/// `ExpandedNote`, whose start is the tick it was expanded at).
#[derive(Clone, Copy)]
struct GhostNote {
    start: PatternTick,
    pitch: Pitch,
    duration: Option<SeqDuration>,
}

/// Cached ghost-preview expansion for one pattern. Recomputed only when the
/// source notes or processor rack change (compared by value), since the sweep
/// over every tick is too costly to run each frame.
struct GhostCache {
    pattern_id: PatternId,
    /// Pattern length is part of the key: the sweep range is `0..length`, so a
    /// length change must invalidate even when notes + rack are unchanged.
    length: SeqDuration,
    notes: Vec<Note>,
    processors: Vec<NoteProcessor>,
    ghosts: Vec<GhostNote>,
}

/// Sweep the note-processor expansion over every tick of `pattern`, collecting
/// the generated notes for the ghost overlay. Empty when there is nothing to
/// expand (no rack, no ornaments). Capped to bound a pathological pattern; runs
/// on the UI thread (allocates), gated by the [`GhostCache`].
fn compute_ghosts(pattern: &synth_sequencer::Pattern, bpm: Bpm) -> Vec<GhostNote> {
    const MAX_GHOSTS: usize = 4096;
    // Nothing to preview without a rack or any ornament. `||` short-circuits so
    // the note scan is skipped when a rack is present.
    let has_work =
        !pattern.processors().is_empty() || pattern.notes().iter().any(|n| n.ornament.is_some());
    if !has_work {
        return Vec::new();
    }
    let mut ghosts = Vec::new();
    let mut buf = ExpansionBuffer::new();
    for t in 0..pattern.length.0 {
        let tick = PatternTick(t);
        pattern.expand_at_tick(tick, |_| true, bpm, &mut buf);
        for en in buf.notes() {
            ghosts.push(GhostNote {
                start: tick,
                pitch: en.pitch,
                duration: en.duration,
            });
            if ghosts.len() >= MAX_GHOSTS {
                return ghosts;
            }
        }
    }
    ghosts
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

/// Floor-snap a tick to a multiple of `step`. `step == 0` is a no-op.
fn snap_to_step(tick: u64, step: u64) -> u64 {
    tick.checked_div(step).map_or(tick, |q| q * step)
}

/// Quantize a `PatternTick` to a row boundary using the row's tick width.
fn quantize_tick(tick: PatternTick, ticks_per_row: u16) -> PatternTick {
    PatternTick(snap_to_step(tick.0 as u64, ticks_per_row as u64) as u32)
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
) -> std::collections::HashMap<InstrumentId, Color32> {
    instruments
        .iter()
        .filter_map(|inst| {
            inst.color
                .as_ref()
                .and_then(|hex| crate::gui::patch_editor::parse_hex_color(hex))
                .map(|c| (inst.id, c))
        })
        .collect()
}

/// Look up a cached instrument colour, falling back to `fallback` when
/// the instrument has no colour set.
fn cached_instrument_color(
    cache: &std::collections::HashMap<InstrumentId, Color32>,
    seq_id: SeqInstrumentId,
    fallback: Color32,
) -> Color32 {
    cache
        .get(&InstrumentId::from(seq_id))
        .copied()
        .unwrap_or(fallback)
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
        .any(|inst| inst.id == view_state.selected_instrument.into())
        && let Some(first) = instruments.first()
        && let Ok(seq_id) = SeqInstrumentId::try_from(first.id)
    {
        view_state.selected_instrument = seq_id;
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

        // Note FX rack inspector (right-docked; only when toggled on). Declared
        // before the bottom piano-roll panel so it spans the full height on the
        // right, beside both the arrangement and the roll.
        if view_state.note_fx_panel_open {
            egui::Panel::right("note_fx_panel")
                .resizable(true)
                .min_size(190.0)
                .default_size(290.0)
                .show_inside(ui, |ui| {
                    note_fx::draw_note_fx_panel(ui, song, view_state, undo_manager, pattern_id);
                });
        }

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
