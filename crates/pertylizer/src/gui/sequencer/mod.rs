//! Sequencer GUI module.
//!
//! Provides the sequencer view with transport controls, an arrangement timeline,
//! a piano roll with mouse interaction (draw, select, move, resize, delete notes),
//! and a GUI input source for sending `InputCommand`s to the sequencer engine.

use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui::{self, Color32, CursorIcon, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use synth_core::{BipolarValue, Bpm, Hertz, MidiNote, Milliseconds, NormalizedValue, Semitones};
use synth_engine::{EngineCommand, EngineHandle, InstrumentId, RecordingState};
use synth_sequencer::{
    AutoInstrumentParam, AutomationPoint, AutomationTarget, CurveType, Duration as SeqDuration,
    ExpansionBuffer, Glide, GlideFrom, GlideInterp, GlobalParam, Note, NoteExpression, NoteId,
    NoteLane, NoteName, NoteProcessor, Ornament, PatternId, PatternTick, Pitch, Song, Tick,
    TimeSignature, TrackId, TrackParam, Velocity, Vibrato, VibratoShape,
};

use crate::gui::input::KEY_MAP;
use crate::gui::theme::theme;
use crate::gui::widgets::{
    CaptionTone, caption, clickable_label, danger_button, dim_label, inline_editable_text,
    labeled_row, strong_label, toggle_button, tree_picker_button, unit_drag_value,
};

mod arrangement;
mod automation;
pub(crate) mod note_fx;
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
pub(crate) use tracker::{NoteGraphEdit, draw_tracker};
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
        /// The dragged point's curve, preserved across the move (a move must
        /// not reset the point's type to the default).
        curve: CurveType,
        /// Top Y of the stacked zone this point lives in. Captured at drag
        /// start so the value math stays correct even when the dragged lane
        /// isn't the focused one (drag works on any stacked lane, not just the
        /// focused zone's `auto_y`).
        zone_y: f32,
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
    /// Curve type applied to newly drawn automation points (the "brush"). A
    /// point's type is changed after the fact from its right-click menu.
    automation_curve: CurveType,
    /// `(pattern, first-used-instrument)` the working instrument was last
    /// auto-selected for. The working instrument snaps to the first instrument
    /// the pattern's tracks play through whenever the open pattern OR that
    /// first-used instrument changes (e.g. a placement is added/moved to a
    /// different track after the pattern was already open) — but not on every
    /// frame, so a manual change while the placement is unchanged still sticks.
    last_auto_instrument: Option<(PatternId, Option<InstrumentId>)>,
    /// The automation lane whose right-click context menu is open, if any:
    /// the lane target plus the point tick under the cursor (`Some` = a point
    /// was hit → curve/delete-point items; `None` = empty zone → lane-level
    /// items only). Both cases also offer "Delete lane". Set on secondary
    /// click, resolved against whichever stacked zone the click landed in.
    automation_ctx_point: Option<(AutomationTarget, Option<PatternTick>)>,
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
    /// When set, the settle-window scroll ignores the "only if off-screen" gate
    /// and always recenters the playhead. Set by the explicit jump buttons (Go
    /// to start / phrase ◀◀ ▶▶ / Go to playhead) so they always scroll the view
    /// to the new position — even when it is already roughly in view, and across
    /// the engine's asynchronous seek delay. Cleared when the settle window
    /// expires.
    force_reveal: bool,
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
    pub selected_instrument: InstrumentId,
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
    /// One-shot request from the Note FX panel's "edit" affordance: the backend
    /// takes this and switches to the Note Grid view with the graph loaded.
    pub(crate) jump_to_note_graph: Option<synth_sequencer::NoteGraphId>,
    /// One-shot request from an automation lane's provenance chip: the backend
    /// takes this and switches to the Mod Grid view with the graph loaded.
    pub(crate) jump_to_mod_graph: Option<synth_sequencer::ModGraphId>,
    /// Open per-note ornament editor popup (target note + baseline + working
    /// copy). `None` when the popup is closed; one coalesced undo entry is pushed
    /// when it closes.
    editing_ornament: Option<OrnamentEdit>,
    /// Open per-note note-graph picker popup (target note + baseline + working
    /// binding), driven by the tracker's Graph column. `None` when closed; one
    /// coalesced `SetNoteGraphBindingBatch` undo entry is pushed on close.
    editing_note_graph: Option<NoteGraphEdit>,
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
            automation_curve: CurveType::Linear,
            automation_ctx_point: None,
            last_auto_instrument: None,
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
            force_reveal: false,
            follow_settle_frames: 0,
            record_quantize: 0,
            overdub: true,
            recording_preview_completed: Vec::new(),
            recording_preview_held: Vec::new(),
            recording_preview_pattern_length: SeqDuration(0),
            selected_instrument: InstrumentId::new(0),
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
            jump_to_note_graph: None,
            jump_to_mod_graph: None,
            editing_ornament: None,
            editing_note_graph: None,
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
    fn ghost_notes(
        &mut self,
        song: &Arc<synth_sequencer::SharedSong>,
        pattern_id: PatternId,
    ) -> Vec<GhostNote> {
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
        // A bound graph takes precedence over the rack (like playback); resolve it
        // so its content is both the expansion source and part of the freshness key.
        let graph = pattern.note_graph().and_then(|gid| s.note_graph(gid));
        // Note-scope graphs bound to individual notes (plan §2.1). Their content
        // must be part of the freshness key so editing one invalidates the ghosts;
        // a binding *id* change is already covered by `notes`. Compared by
        // reference (no clone in the hot path); cloned only on a cache miss.
        let note_scope_refs = collect_note_scope_graphs(&s, notes);
        let fresh = self.ghost_cache.as_ref().is_some_and(|c| {
            c.pattern_id == pattern_id
                && c.length == pattern.length
                && c.notes.as_slice() == notes
                && c.processors.as_slice() == processors
                && c.graph.as_ref() == graph
                && c.note_scope_graphs
                    .iter()
                    .eq(note_scope_refs.iter().copied())
        });
        if !fresh {
            let ghosts = compute_ghosts(
                pattern,
                graph,
                s.note_graph_pool(),
                s.tempo_at(synth_sequencer::Tick(0)),
            );
            self.ghost_cache = Some(GhostCache {
                pattern_id,
                length: pattern.length,
                notes: notes.to_vec(),
                processors: processors.to_vec(),
                graph: graph.cloned(),
                note_scope_graphs: note_scope_refs.into_iter().cloned().collect(),
                ghosts,
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
        // A passive reveal is off-screen-gated; clear any forced bit left over
        // from a previous jump so it cannot contaminate this settle window
        // (`force` is opt-in per call via `reveal_playhead_force`).
        self.force_reveal = false;
    }

    /// Like [`Self::reveal_playhead`] but forces the settle-window scroll to
    /// recenter the marker even when it is already in view. For explicit
    /// "jump to here" actions (Go to start / phrase ◀◀ ▶▶ / Go to playhead),
    /// where the user expects the view to follow, not just the position.
    fn reveal_playhead_force(&mut self) {
        self.reveal_playhead();
        self.force_reveal = true;
    }
}

impl Default for SequencerViewState {
    fn default() -> Self {
        Self::new()
    }
}

/// The persistent id egui's `ScrollArea::id_salt(salt)` uses for its stored
/// `State`, so callers can `State::load`/`store` the exact same memory slot to
/// drive auto-follow scrolling.
///
/// egui 0.35 derives it as `ui.make_persistent_id(IdSalt::new(salt))`. Crucially
/// `IdSalt::new` hashes with different seeds than `Id::new`, so a plain
/// `Id::new(salt)` computes a *different* id and any forced scroll offset is
/// silently dropped — keep this in lockstep with `ScrollArea::id_salt`.
pub(super) fn scroll_state_id(ui: &egui::Ui, salt: &str) -> egui::Id {
    ui.make_persistent_id(egui::IdSalt::new(salt))
}

/// Whether the user manually scrolled the area since auto-follow last set its
/// horizontal offset. `expected` (the offset we requested) is clamped to the
/// area's real max first, because egui clamps the stored offset to the content:
/// a target past the end (playhead near the song end, or content narrower than
/// the viewport) is not a manual drag.
pub(super) fn user_scrolled_away<R>(
    scroll_output: &egui::scroll_area::ScrollAreaOutput<R>,
    expected: f32,
) -> bool {
    let max_x = (scroll_output.content_size.x - scroll_output.inner_rect.width()).max(0.0);
    (scroll_output.state.offset.x - expected.clamp(0.0, max_x)).abs() > MANUAL_SCROLL_EPS
}

/// Apply a rename to a pattern under a short write-lock and push a
/// `RenamePattern` undo entry. No-op if the name is unchanged or the pattern
/// no longer exists.
pub(crate) fn commit_pattern_rename(
    song: &Arc<synth_sequencer::SharedSong>,
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
    song: &Arc<synth_sequencer::SharedSong>,
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
/// Pixel slack between the auto-follow target offset and the ScrollArea's
/// actual offset before a divergence is treated as a manual user scroll.
const MANUAL_SCROLL_EPS: f32 = 2.0;
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
/// Height of the tempo-map lane, drawn between the ruler and the track rows.
const TEMPO_LANE_HEIGHT: f32 = 48.0;
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
    instrument_id: InstrumentId,
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
    instrument: InstrumentId,
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
    /// Tempo automation points: (tick, BPM, ramp). `ramp` = linear ramp toward
    /// the next point (accelerando/ritardando) vs a step change. Sorted by tick.
    tempo_changes: Vec<(u64, f32, bool)>,
    /// Default tempo (BPM) governing the timeline before the first tempo point.
    default_tempo: f32,
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
    /// The note's bound note-scope graph, if any (plan §2.1). Edited via the
    /// selection inspector's per-note graph selector.
    note_graph: Option<synth_sequencer::NoteGraphId>,
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
    /// The bound note graph (content, not just id) at cache time, so a graph
    /// edit — or a binding change — invalidates the preview.
    graph: Option<synth_sequencer::NoteGraph>,
    /// The distinct note-scope graphs (content) bound to this pattern's notes.
    /// A note's binding *id* change is already caught by `notes`; this also
    /// invalidates when a bound note-scope graph's *content* is edited (plan §2.1).
    note_scope_graphs: Vec<synth_sequencer::NoteGraph>,
    ghosts: Vec<GhostNote>,
}

/// Distinct note-scope graphs (deterministically ordered) bound to `notes`,
/// resolved from the pool — the content half of the ghost-cache key so a
/// note-scope graph edit invalidates the preview (plan §2.1). Returns borrows;
/// the caller clones only when it actually refreshes the cache.
fn collect_note_scope_graphs<'a>(
    song: &'a Song,
    notes: &[Note],
) -> Vec<&'a synth_sequencer::NoteGraph> {
    let mut ids: Vec<synth_sequencer::NoteGraphId> =
        notes.iter().filter_map(|n| n.note_graph).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.iter().filter_map(|&gid| song.note_graph(gid)).collect()
}

/// Sweep the note expansion over every tick of `pattern`, collecting the
/// generated notes for the ghost overlay. A bound note `graph` takes precedence
/// over the legacy rack (mirroring playback and the load-time migration, which
/// leaves every migrated pattern graph-bound with an empty rack). Per-note
/// note-scope graphs (plan §2.1) are resolved from `pool` exactly like playback.
/// Empty when there is nothing to expand (no graph nodes, no rack, no ornaments,
/// no note-scope bindings). Capped to bound a pathological pattern; runs on the
/// UI thread, gated by the [`GhostCache`].
fn compute_ghosts(
    pattern: &synth_sequencer::Pattern,
    graph: Option<&synth_sequencer::NoteGraph>,
    pool: &[synth_sequencer::NoteGraph],
    bpm: Bpm,
) -> Vec<GhostNote> {
    const MAX_GHOSTS: usize = 4096;
    let has_ornament = pattern.notes().iter().any(|n| n.ornament.is_some());
    // Per-note graph bindings (plan §2.1) also constitute work, even with no
    // pattern-scope graph / rack / ornament.
    let has_note_scope = pattern.notes().iter().any(|n| n.note_graph.is_some());
    let has_work = has_note_scope
        || match graph {
            Some(g) => g.node_count() > 0 || has_ornament,
            None => !pattern.processors().is_empty() || has_ornament,
        };
    if !has_work {
        return Vec::new();
    }
    let host = synth_sequencer::HostKey::from(pattern.id);
    let mut ghosts = Vec::new();
    let mut buf = ExpansionBuffer::new();
    // Note-scope resolves each note's bound graph during source seeding, exactly
    // like playback; its inner single-note expansion needs its own scratch.
    let mut note_scope_scratch = ExpansionBuffer::new();
    let mut ns_ctx = synth_sequencer::NoteScopeCtx {
        pool,
        scratch: &mut note_scope_scratch,
    };
    // Timing look-back pool (delay/echo re-running the upstream prefix at earlier
    // ticks); non-RT preview, so allocated locally like playback allocates its
    // engine field. Kept once, reused per tick.
    let mut lookback = synth_sequencer::lookback_pool();
    for t in 0..pattern.length.0 {
        let tick = PatternTick(t);
        match graph {
            Some(g) => {
                g.expand_at_tick(
                    pattern.notes(),
                    tick,
                    host,
                    bpm,
                    |_| true,
                    Some(&mut ns_ctx),
                    Some(&mut lookback),
                    &mut buf,
                );
            }
            None => pattern.expand_at_tick(tick, |_| true, bpm, Some(&mut ns_ctx), &mut buf),
        }
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
    track_overrides: Vec<InstrumentId>,
    /// Every track in the song, `(id, name)` — the cross-track lane targets
    /// (`Track { Some(id) }`) offered under the automation picker's submenu.
    all_tracks: Vec<(TrackId, String)>,
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
    seq_id: InstrumentId,
    fallback: Color32,
) -> Color32 {
    cache.get(&seq_id).copied().unwrap_or(fallback)
}

/// Send the engine an `ArmRecord` command for the given pattern, using the
/// placement on the user's selected track (or the first available
/// placement) as the recording region. Returns true if arm was sent.
fn arm_recording_for_pattern(
    handle: &mut EngineHandle,
    song: &Arc<synth_sequencer::SharedSong>,
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
    song: &Arc<synth_sequencer::SharedSong>,
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
    song: &Arc<synth_sequencer::SharedSong>,
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
    song: &Arc<synth_sequencer::SharedSong>,
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
    song: &Arc<synth_sequencer::SharedSong>,
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
        .any(|inst| inst.id == view_state.selected_instrument)
        && let Some(first) = instruments.first()
    {
        view_state.selected_instrument = first.id;
    }

    // Transport bar at the top
    let is_playing = super::toolbar::top(ui, "sequencer_transport", |ui| {
        draw_transport_bar(ui, handle, song, view_state)
    });

    // Request repaint during playback for smooth position updates. While
    // stopped, keep repainting for a few frames after a transport jump / seek /
    // stop so the timeline catches the engine's async playhead update and the
    // off-screen follow can scroll the marker into view.
    if is_playing {
        // `force_reveal` only applies to the stopped settle window; while
        // playing, continuous follow does the scrolling, so drop the bit here
        // so a jump made during playback can't leak into the settle window
        // after a later Pause.
        view_state.force_reveal = false;
        ctx.request_repaint();
    } else if view_state.follow_settle_frames > 0 {
        view_state.follow_settle_frames -= 1;
        if view_state.follow_settle_frames == 0 {
            view_state.force_reveal = false;
        }
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
                .show(ui, |ui| {
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
            .show(ui, |ui| {
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
                        None,
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
                        dim_label(ui, "Pattern not found");
                        view_state.close_piano_roll();
                        handle.send(EngineCommand::SetSoloPattern(None));
                        handle.send(EngineCommand::SetPreviewPattern(None));
                    }
                }
            });
    }

    // Main content: arrangement view
    egui::CentralPanel::default().show(ui, |ui| {
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
            dim_label(ui, "Song locked...");
        }
    });
}

/// Convert a sequencer track color to an egui Color32.
pub(crate) fn track_color_to_egui(color: synth_sequencer::TrackColor) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}
