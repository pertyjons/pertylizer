//! Undo/redo system for sequencer, module, and connection operations.
//!
//! The `UndoManager` stores actions on undo/redo stacks and returns them
//! to the caller for execution. The caller (GUI code) is responsible for
//! actually applying the inverse action via the existing session/song APIs.

use synth_core::Bpm;
use synth_engine::ModuleId;
use synth_engine::graph::Connection;
use synth_engine::instrument::InstrumentId;
use synth_sequencer::{
    Duration as SeqDuration, Glide, NoteExpression, NoteId, NoteLane, Ornament, PatternId,
    PatternTick, Pitch, Tick, TrackId, Velocity,
};

use crate::patch::{ConnectionState, ModuleState, ParamValue};

/// Maximum number of undo actions to keep in history.
const MAX_UNDO_HISTORY: usize = 100;

/// A single undoable action.
///
/// Each variant captures enough state to reverse the operation.
/// `Composite` groups multiple actions into a single undo step.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum UndoAction {
    // ── Sequencer note operations ──
    /// A note was added to a pattern.
    AddNote {
        pattern_id: PatternId,
        note: NoteSnapshot,
    },
    /// A note was removed from a pattern.
    RemoveNote {
        pattern_id: PatternId,
        note: NoteSnapshot,
    },
    /// A note was moved to a new start tick.
    MoveNote {
        pattern_id: PatternId,
        note_id: NoteId,
        old_start: PatternTick,
        new_start: PatternTick,
    },
    /// A note was resized to a new duration.
    ResizeNote {
        pattern_id: PatternId,
        note_id: NoteId,
        old_duration: Option<SeqDuration>,
        new_duration: Option<SeqDuration>,
    },
    /// A note was transposed to a new pitch.
    TransposeNote {
        pattern_id: PatternId,
        note_id: NoteId,
        old_pitch: Pitch,
        new_pitch: Pitch,
    },
    /// A note's velocity was changed.
    SetNoteVelocity {
        pattern_id: PatternId,
        note_id: NoteId,
        old_velocity: Velocity,
        new_velocity: Velocity,
    },
    /// Batch velocity change (scale velocities, multi-edit).
    /// Each tuple is `(note_id, old_velocity, new_velocity)`.
    SetVelocitiesBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, Velocity, Velocity)>,
    },
    /// Batch per-note legato/tie change (multi-edit).
    /// Each tuple is `(note_id, old_legato, new_legato)`.
    SetLegatoBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, bool, bool)>,
    },
    /// Batch per-note glide change (multi-edit).
    /// Each tuple is `(note_id, old_glide, new_glide)`.
    SetGlideBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, Option<Glide>, Option<Glide>)>,
    },
    /// Batch per-note expression-block change (multi-edit).
    /// Each tuple is `(note_id, old_expression, new_expression)`.
    SetExpressionBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, Option<NoteExpression>, Option<NoteExpression>)>,
    },
    /// Batch per-note voice-lane change (tracker column assignment / migration).
    /// Each tuple is `(note_id, old_lane, new_lane)`.
    SetLaneBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, NoteLane, NoteLane)>,
    },
    /// Batch per-note note-scope graph binding change (multi-edit, plan §2.1).
    /// Each tuple is `(note_id, old_graph, new_graph)`.
    SetNoteGraphBindingBatch {
        pattern_id: PatternId,
        changes: Vec<(
            NoteId,
            Option<synth_sequencer::NoteGraphId>,
            Option<synth_sequencer::NoteGraphId>,
        )>,
    },

    /// A pattern's note graph (or legacy per-note ornaments) was frozen — baked
    /// into plain notes and the source cleared. Carries the full pre-freeze
    /// pattern so
    /// the inverse restores notes + rack losslessly; applying it re-runs the
    /// (deterministic) freeze for redo.
    FreezePattern {
        pattern_id: PatternId,
        before: synth_sequencer::Pattern,
    },
    /// Restore a full pattern snapshot — the inverse of [`Self::FreezePattern`].
    RestorePattern {
        pattern_id: PatternId,
        snapshot: synth_sequencer::Pattern,
    },
    /// A note's per-note ornament was set, changed, or cleared.
    SetNoteOrnament {
        pattern_id: PatternId,
        note_id: NoteId,
        old: Option<Ornament>,
        new: Option<Ornament>,
    },

    // ── Pattern + track metadata ──
    /// A pattern was renamed.
    RenamePattern {
        pattern_id: PatternId,
        old_name: String,
        new_name: String,
    },
    /// A track was renamed.
    RenameTrack {
        track_id: TrackId,
        old_name: String,
        new_name: String,
    },
    /// A pattern's length was changed.
    SetPatternLength {
        pattern_id: PatternId,
        old_length: SeqDuration,
        new_length: SeqDuration,
    },

    // ── Tempo curve ──
    /// A tempo change at a specific tick was set, replaced, or removed. Each
    /// side carries `(bpm, ramp)` where `ramp` selects a linear ramp toward the
    /// next point vs a step change.
    ///
    /// `old = None` means there was no explicit change at this tick; `new = None`
    /// means the change was removed. The two together cover "Set tempo here…"
    /// (None → Some), edit / ramp toggle (Some(a) → Some(b)) and "Remove tempo
    /// change here" (Some → None).
    SetTempo {
        tick: Tick,
        old: Option<(Bpm, bool)>,
        new: Option<(Bpm, bool)>,
    },

    /// A tempo point was dragged in the lane: it moved from `old` to `new`, each
    /// `(tick, bpm, ramp)`. Applying removes the point at `old.0` and writes
    /// `new`; the inverse does the reverse.
    MoveTempo {
        old: (Tick, Bpm, bool),
        new: (Tick, Bpm, bool),
    },

    // ── Arrangement ──
    /// A placement was moved or transferred between tracks.
    MovePlacement {
        pattern_id: PatternId,
        old_track_id: TrackId,
        old_start: Tick,
        new_track_id: TrackId,
        new_start: Tick,
    },

    // ── Track / pattern delete (full state) ──
    /// A track was deleted. Restoring it brings back the full track plus
    /// every placement that referenced it.
    DeleteTrack {
        track: synth_sequencer::SequencerTrack,
        track_index: usize,
        placements: Vec<synth_sequencer::PatternPlacement>,
    },
    /// A track was restored from undo (inverse of DeleteTrack).
    AddTrack {
        track: synth_sequencer::SequencerTrack,
        track_index: usize,
        placements: Vec<synth_sequencer::PatternPlacement>,
    },
    /// A pattern was deleted. Restoring it brings back the full pattern
    /// (notes + automation) plus every placement that referenced it.
    DeletePattern {
        pattern: synth_sequencer::Pattern,
        placements: Vec<synth_sequencer::PatternPlacement>,
    },
    /// A pattern was restored from undo (inverse of DeletePattern).
    AddPattern {
        pattern: synth_sequencer::Pattern,
        placements: Vec<synth_sequencer::PatternPlacement>,
    },
    /// A placement was removed from the arrangement.
    RemovePlacement {
        placement: synth_sequencer::PatternPlacement,
    },
    /// A placement was inserted (inverse of RemovePlacement).
    InsertPlacement {
        placement: synth_sequencer::PatternPlacement,
    },
    /// A placement's length override changed (resize handle).
    SetPlacementLength {
        pattern_id: PatternId,
        track_id: TrackId,
        start: Tick,
        old_length: Option<SeqDuration>,
        new_length: Option<SeqDuration>,
    },

    // ── Automation ──
    /// An automation point was added.
    AddAutomationPoint {
        pattern_id: PatternId,
        target: synth_sequencer::AutomationTarget,
        tick: PatternTick,
        value: synth_core::NormalizedValue,
        curve: synth_sequencer::CurveType,
    },
    /// An automation point was removed.
    RemoveAutomationPoint {
        pattern_id: PatternId,
        target: synth_sequencer::AutomationTarget,
        tick: PatternTick,
        value: synth_core::NormalizedValue,
        curve: synth_sequencer::CurveType,
    },
    /// An automation point was moved.
    MoveAutomationPoint {
        pattern_id: PatternId,
        target: synth_sequencer::AutomationTarget,
        old_tick: PatternTick,
        old_value: synth_core::NormalizedValue,
        new_tick: PatternTick,
        new_value: synth_core::NormalizedValue,
        curve: synth_sequencer::CurveType,
    },
    /// An automation point's curve type was changed in place (same tick/value).
    SetAutomationPointCurve {
        pattern_id: PatternId,
        target: synth_sequencer::AutomationTarget,
        tick: PatternTick,
        value: synth_core::NormalizedValue,
        old_curve: synth_sequencer::CurveType,
        new_curve: synth_sequencer::CurveType,
    },
    /// A whole automation lane was added (e.g. a new empty tracker column).
    AddAutomationLane {
        pattern_id: PatternId,
        lane: synth_sequencer::AutomationLane,
    },
    /// A whole automation lane was removed (e.g. pruning empty tracker columns).
    /// Carries the full lane so undo restores its points too.
    RemoveAutomationLane {
        pattern_id: PatternId,
        lane: synth_sequencer::AutomationLane,
    },

    // ── Module operations ──
    /// A module was added to an instrument (connections captured for undo round-trip).
    AddModule {
        instrument_id: InstrumentId,
        module_state: ModuleState,
        connections: Vec<ConnectionState>,
    },
    /// A module was removed from an instrument (captures state + connections for undo).
    RemoveModule {
        instrument_id: InstrumentId,
        module_state: ModuleState,
        connections: Vec<ConnectionState>,
    },
    /// A module was moved to a new position.
    MoveModule {
        module_id: ModuleId,
        old_pos: (f32, f32),
        new_pos: (f32, f32),
    },
    /// A module parameter was changed.
    SetParameter {
        instrument_id: InstrumentId,
        module_id: ModuleId,
        param_name: String,
        old_value: ParamValue,
        new_value: ParamValue,
    },

    // ── Connection operations ──
    /// A connection was added.
    AddConnection {
        instrument_id: InstrumentId,
        connection: Connection,
    },
    /// A connection was removed.
    RemoveConnection {
        instrument_id: InstrumentId,
        connection: Connection,
    },

    // ── Note Grid (pooled note graphs) ──
    /// A pooled note graph was created (`old: None`), edited, or deleted
    /// (`new: None`) — full-snapshot on each side, like [`Self::FreezePattern`].
    /// Applying writes the `new` side into the `Song` pool. NB: like every
    /// snapshot action here, applying overwrites concurrent external (MCP)
    /// edits to the same object — the app-wide undo trade-off.
    SetNoteGraph {
        graph_id: synth_sequencer::NoteGraphId,
        old: Option<synth_sequencer::NoteGraph>,
        new: Option<synth_sequencer::NoteGraph>,
    },
    /// A Mod Grid graph was created (`old: None`), edited, or deleted
    /// (`new: None`) — full-snapshot on each side, mirroring [`Self::SetNoteGraph`].
    /// Applying writes the `new` side into the `Song` mod-grid pool.
    SetModGraph {
        graph_id: synth_sequencer::ModGraphId,
        old: Option<synth_sequencer::ModGraph>,
        new: Option<synth_sequencer::ModGraph>,
    },
    /// A pattern's note-graph binding changed.
    SetPatternNoteGraph {
        pattern_id: PatternId,
        old: Option<synth_sequencer::NoteGraphId>,
        new: Option<synth_sequencer::NoteGraphId>,
    },

    // ── Composite ──
    /// Multiple actions grouped as a single undo step.
    Composite(Vec<UndoAction>),
}

/// Snapshot of a note for undo/redo purposes.
///
/// Captures all fields needed to fully reconstruct a note.
#[derive(Debug, Clone)]
pub(crate) struct NoteSnapshot {
    pub(crate) id: NoteId,
    pub(crate) start: PatternTick,
    pub(crate) duration: Option<SeqDuration>,
    pub(crate) pitch: Pitch,
    pub(crate) velocity: Velocity,
    pub(crate) track: Option<synth_sequencer::TrackId>,
    pub(crate) lane: NoteLane,
}

impl From<&synth_sequencer::Note> for NoteSnapshot {
    fn from(note: &synth_sequencer::Note) -> Self {
        Self {
            id: note.id,
            start: note.start,
            duration: note.duration,
            pitch: note.pitch,
            velocity: note.velocity,
            track: note.track,
            lane: note.lane,
        }
    }
}

/// Stack-based undo/redo manager.
///
/// Stores actions and returns them on undo/redo. The caller is responsible
/// for actually executing the returned (inverse) action.
pub(crate) struct UndoManager {
    undo_stack: Vec<UndoAction>,
    redo_stack: Vec<UndoAction>,
}

impl UndoManager {
    /// Create a new empty undo manager.
    pub(crate) fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Push a new action onto the undo stack and clear the redo stack.
    pub(crate) fn push(&mut self, action: UndoAction) {
        self.redo_stack.clear();
        self.undo_stack.push(action);
        // Trim history if it exceeds the maximum.
        if self.undo_stack.len() > MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
    }

    /// Undo the most recent action.
    ///
    /// Pops from undo stack, pushes to redo stack, and returns the
    /// **inverse** action that should be executed to reverse the change.
    pub(crate) fn undo(&mut self) -> Option<UndoAction> {
        let original = self.undo_stack.pop()?;
        let inverse = Self::inverse(&original);
        self.redo_stack.push(original);
        Some(inverse)
    }

    /// Redo the most recently undone action.
    ///
    /// Pops from redo stack, pushes back to undo stack, and returns
    /// the **original** action that should be re-executed.
    pub(crate) fn redo(&mut self) -> Option<UndoAction> {
        let original = self.redo_stack.pop()?;
        self.undo_stack.push(original.clone());
        Some(original)
    }

    /// Whether there are actions available to undo.
    pub(crate) fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether there are actions available to redo.
    pub(crate) fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Clear all undo and redo history.
    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Create the inverse of an action (swaps old/new, add/remove).
    fn inverse(action: &UndoAction) -> UndoAction {
        match action {
            UndoAction::AddNote { pattern_id, note } => UndoAction::RemoveNote {
                pattern_id: *pattern_id,
                note: note.clone(),
            },
            UndoAction::RemoveNote { pattern_id, note } => UndoAction::AddNote {
                pattern_id: *pattern_id,
                note: note.clone(),
            },
            UndoAction::FreezePattern { pattern_id, before } => UndoAction::RestorePattern {
                pattern_id: *pattern_id,
                snapshot: before.clone(),
            },
            UndoAction::RestorePattern {
                pattern_id,
                snapshot,
            } => UndoAction::FreezePattern {
                pattern_id: *pattern_id,
                before: snapshot.clone(),
            },
            UndoAction::SetNoteOrnament {
                pattern_id,
                note_id,
                old,
                new,
            } => UndoAction::SetNoteOrnament {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old: *new,
                new: *old,
            },
            UndoAction::MoveNote {
                pattern_id,
                note_id,
                old_start,
                new_start,
            } => UndoAction::MoveNote {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old_start: *new_start,
                new_start: *old_start,
            },
            UndoAction::ResizeNote {
                pattern_id,
                note_id,
                old_duration,
                new_duration,
            } => UndoAction::ResizeNote {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old_duration: *new_duration,
                new_duration: *old_duration,
            },
            UndoAction::TransposeNote {
                pattern_id,
                note_id,
                old_pitch,
                new_pitch,
            } => UndoAction::TransposeNote {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old_pitch: *new_pitch,
                new_pitch: *old_pitch,
            },
            UndoAction::SetNoteVelocity {
                pattern_id,
                note_id,
                old_velocity,
                new_velocity,
            } => UndoAction::SetNoteVelocity {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old_velocity: *new_velocity,
                new_velocity: *old_velocity,
            },
            UndoAction::SetVelocitiesBatch {
                pattern_id,
                changes,
            } => UndoAction::SetVelocitiesBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetLegatoBatch {
                pattern_id,
                changes,
            } => UndoAction::SetLegatoBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetGlideBatch {
                pattern_id,
                changes,
            } => UndoAction::SetGlideBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetExpressionBatch {
                pattern_id,
                changes,
            } => UndoAction::SetExpressionBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetLaneBatch {
                pattern_id,
                changes,
            } => UndoAction::SetLaneBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetNoteGraphBindingBatch {
                pattern_id,
                changes,
            } => UndoAction::SetNoteGraphBindingBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::RenamePattern {
                pattern_id,
                old_name,
                new_name,
            } => UndoAction::RenamePattern {
                pattern_id: *pattern_id,
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            },
            UndoAction::RenameTrack {
                track_id,
                old_name,
                new_name,
            } => UndoAction::RenameTrack {
                track_id: *track_id,
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            },
            UndoAction::SetPatternLength {
                pattern_id,
                old_length,
                new_length,
            } => UndoAction::SetPatternLength {
                pattern_id: *pattern_id,
                old_length: *new_length,
                new_length: *old_length,
            },
            UndoAction::SetTempo { tick, old, new } => UndoAction::SetTempo {
                tick: *tick,
                old: *new,
                new: *old,
            },
            UndoAction::MoveTempo { old, new } => UndoAction::MoveTempo {
                old: *new,
                new: *old,
            },
            UndoAction::MovePlacement {
                pattern_id,
                old_track_id,
                old_start,
                new_track_id,
                new_start,
            } => UndoAction::MovePlacement {
                pattern_id: *pattern_id,
                old_track_id: *new_track_id,
                old_start: *new_start,
                new_track_id: *old_track_id,
                new_start: *old_start,
            },
            UndoAction::AddAutomationPoint {
                pattern_id,
                target,
                tick,
                value,
                curve,
            } => UndoAction::RemoveAutomationPoint {
                pattern_id: *pattern_id,
                target: target.clone(),
                tick: *tick,
                value: *value,
                curve: *curve,
            },
            UndoAction::RemoveAutomationPoint {
                pattern_id,
                target,
                tick,
                value,
                curve,
            } => UndoAction::AddAutomationPoint {
                pattern_id: *pattern_id,
                target: target.clone(),
                tick: *tick,
                value: *value,
                curve: *curve,
            },
            UndoAction::DeleteTrack {
                track,
                track_index,
                placements,
            } => UndoAction::AddTrack {
                track: track.clone(),
                track_index: *track_index,
                placements: placements.clone(),
            },
            UndoAction::AddTrack {
                track,
                track_index,
                placements,
            } => UndoAction::DeleteTrack {
                track: track.clone(),
                track_index: *track_index,
                placements: placements.clone(),
            },
            UndoAction::DeletePattern {
                pattern,
                placements,
            } => UndoAction::AddPattern {
                pattern: pattern.clone(),
                placements: placements.clone(),
            },
            UndoAction::AddPattern {
                pattern,
                placements,
            } => UndoAction::DeletePattern {
                pattern: pattern.clone(),
                placements: placements.clone(),
            },
            UndoAction::RemovePlacement { placement } => UndoAction::InsertPlacement {
                placement: placement.clone(),
            },
            UndoAction::InsertPlacement { placement } => UndoAction::RemovePlacement {
                placement: placement.clone(),
            },
            UndoAction::SetPlacementLength {
                pattern_id,
                track_id,
                start,
                old_length,
                new_length,
            } => UndoAction::SetPlacementLength {
                pattern_id: *pattern_id,
                track_id: *track_id,
                start: *start,
                old_length: *new_length,
                new_length: *old_length,
            },
            UndoAction::MoveAutomationPoint {
                pattern_id,
                target,
                old_tick,
                old_value,
                new_tick,
                new_value,
                curve,
            } => UndoAction::MoveAutomationPoint {
                pattern_id: *pattern_id,
                target: target.clone(),
                old_tick: *new_tick,
                old_value: *new_value,
                new_tick: *old_tick,
                new_value: *old_value,
                curve: *curve,
            },
            UndoAction::SetAutomationPointCurve {
                pattern_id,
                target,
                tick,
                value,
                old_curve,
                new_curve,
            } => UndoAction::SetAutomationPointCurve {
                pattern_id: *pattern_id,
                target: target.clone(),
                tick: *tick,
                value: *value,
                old_curve: *new_curve,
                new_curve: *old_curve,
            },
            UndoAction::AddAutomationLane { pattern_id, lane } => {
                UndoAction::RemoveAutomationLane {
                    pattern_id: *pattern_id,
                    lane: lane.clone(),
                }
            }
            UndoAction::RemoveAutomationLane { pattern_id, lane } => {
                UndoAction::AddAutomationLane {
                    pattern_id: *pattern_id,
                    lane: lane.clone(),
                }
            }
            UndoAction::AddModule {
                instrument_id,
                module_state,
                connections,
            } => UndoAction::RemoveModule {
                instrument_id: *instrument_id,
                module_state: module_state.clone(),
                connections: connections.clone(),
            },
            UndoAction::RemoveModule {
                instrument_id,
                module_state,
                connections,
            } => UndoAction::AddModule {
                instrument_id: *instrument_id,
                module_state: module_state.clone(),
                connections: connections.clone(),
            },
            UndoAction::MoveModule {
                module_id,
                old_pos,
                new_pos,
            } => UndoAction::MoveModule {
                module_id: *module_id,
                old_pos: *new_pos,
                new_pos: *old_pos,
            },
            UndoAction::SetParameter {
                instrument_id,
                module_id,
                param_name,
                old_value,
                new_value,
            } => UndoAction::SetParameter {
                instrument_id: *instrument_id,
                module_id: *module_id,
                param_name: param_name.clone(),
                old_value: new_value.clone(),
                new_value: old_value.clone(),
            },
            UndoAction::AddConnection {
                instrument_id,
                connection,
            } => UndoAction::RemoveConnection {
                instrument_id: *instrument_id,
                connection: *connection,
            },
            UndoAction::RemoveConnection {
                instrument_id,
                connection,
            } => UndoAction::AddConnection {
                instrument_id: *instrument_id,
                connection: *connection,
            },
            UndoAction::SetNoteGraph { graph_id, old, new } => UndoAction::SetNoteGraph {
                graph_id: *graph_id,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::SetModGraph { graph_id, old, new } => UndoAction::SetModGraph {
                graph_id: *graph_id,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::SetPatternNoteGraph {
                pattern_id,
                old,
                new,
            } => UndoAction::SetPatternNoteGraph {
                pattern_id: *pattern_id,
                old: *new,
                new: *old,
            },
            UndoAction::Composite(actions) => {
                // Reverse the order and invert each action.
                UndoAction::Composite(actions.iter().rev().map(Self::inverse).collect())
            }
        }
    }
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::assert_matches;

    fn test_pattern_id() -> PatternId {
        PatternId(1)
    }

    fn test_note_snapshot() -> NoteSnapshot {
        NoteSnapshot {
            id: NoteId(42),
            start: PatternTick(0),
            duration: Some(SeqDuration(960)),
            pitch: Pitch::MIDDLE_C,
            velocity: Velocity::MF,
            track: None,
            lane: NoteLane::ZERO,
        }
    }

    #[test]
    fn test_push_clears_redo_stack() {
        let mut mgr = UndoManager::new();
        let action = UndoAction::AddNote {
            pattern_id: test_pattern_id(),
            note: test_note_snapshot(),
        };
        mgr.push(action.clone());
        assert!(mgr.can_undo());

        // Undo puts inverse on redo stack.
        mgr.undo();
        assert!(mgr.can_redo());

        // Push new action clears redo.
        mgr.push(action);
        assert!(!mgr.can_redo());
    }

    #[test]
    fn test_undo_returns_action_and_pushes_inverse_to_redo() {
        let mut mgr = UndoManager::new();
        mgr.push(UndoAction::AddNote {
            pattern_id: test_pattern_id(),
            note: test_note_snapshot(),
        });

        let undone = mgr.undo().unwrap();
        // Undo returns the inverse: RemoveNote (to reverse AddNote).
        assert_matches!(undone, UndoAction::RemoveNote { .. });
        assert!(mgr.can_redo());
        assert!(!mgr.can_undo());
    }

    #[test]
    fn test_redo_returns_action_and_pushes_inverse_to_undo() {
        let mut mgr = UndoManager::new();
        mgr.push(UndoAction::AddNote {
            pattern_id: test_pattern_id(),
            note: test_note_snapshot(),
        });
        mgr.undo();

        let redone = mgr.redo().unwrap();
        // Redo returns the original action (AddNote) to re-execute.
        assert_matches!(redone, UndoAction::AddNote { .. });
        assert!(mgr.can_undo());
        assert!(!mgr.can_redo());
    }

    #[test]
    fn test_undo_empty_returns_none() {
        let mut mgr = UndoManager::new();
        assert!(mgr.undo().is_none());
    }

    #[test]
    fn test_redo_empty_returns_none() {
        let mut mgr = UndoManager::new();
        assert!(mgr.redo().is_none());
    }

    #[test]
    fn test_clear_empties_both_stacks() {
        let mut mgr = UndoManager::new();
        mgr.push(UndoAction::AddNote {
            pattern_id: test_pattern_id(),
            note: test_note_snapshot(),
        });
        mgr.undo();
        assert!(mgr.can_redo());

        mgr.clear();
        assert!(!mgr.can_undo());
        assert!(!mgr.can_redo());
    }

    #[test]
    fn test_max_history_trims_oldest() {
        let mut mgr = UndoManager::new();
        for i in 0..150 {
            mgr.push(UndoAction::MoveNote {
                pattern_id: test_pattern_id(),
                note_id: NoteId(i),
                old_start: PatternTick(0),
                new_start: PatternTick(i as u32),
            });
        }
        // Should be capped at MAX_UNDO_HISTORY.
        assert_eq!(mgr.undo_stack.len(), MAX_UNDO_HISTORY);
    }

    #[test]
    fn test_inverse_of_move_note_swaps_positions() {
        let action = UndoAction::MoveNote {
            pattern_id: test_pattern_id(),
            note_id: NoteId(1),
            old_start: PatternTick(0),
            new_start: PatternTick(100),
        };
        let inv = UndoManager::inverse(&action);
        if let UndoAction::MoveNote {
            old_start,
            new_start,
            ..
        } = inv
        {
            assert_eq!(old_start, PatternTick(100));
            assert_eq!(new_start, PatternTick(0));
        } else {
            panic!("Expected MoveNote inverse");
        }
    }

    #[test]
    fn test_inverse_of_set_automation_point_curve_swaps_curves() {
        use synth_sequencer::{AutomationTarget, CurveType, GlobalParam, PatternTick};
        let apply = UndoAction::SetAutomationPointCurve {
            pattern_id: PatternId(1),
            target: AutomationTarget::Global(GlobalParam::MasterVolume),
            tick: PatternTick(480),
            value: synth_core::NormalizedValue::new(0.5),
            old_curve: CurveType::Linear,
            new_curve: CurveType::Step,
        };
        let inv = UndoManager::inverse(&apply);
        if let UndoAction::SetAutomationPointCurve {
            old_curve,
            new_curve,
            tick,
            ..
        } = inv
        {
            // Inverse swaps the curves and keeps the same tick/value/target.
            assert_eq!(old_curve, CurveType::Step);
            assert_eq!(new_curve, CurveType::Linear);
            assert_eq!(tick, PatternTick(480));
        } else {
            panic!("Expected SetAutomationPointCurve inverse");
        }
    }

    #[test]
    fn test_inverse_of_set_tempo_apply_round_trips() {
        // Apply tempo at a tick that had no explicit change → inverse must be a remove.
        let apply = UndoAction::SetTempo {
            tick: Tick(960),
            old: None,
            new: Some((Bpm::new(140.0), false)),
        };
        let inv = UndoManager::inverse(&apply);
        if let UndoAction::SetTempo { tick, old, new } = inv {
            assert_eq!(tick, Tick(960));
            assert_eq!(old, Some((Bpm::new(140.0), false)));
            assert_eq!(new, None);
        } else {
            panic!("Expected SetTempo inverse");
        }
    }

    #[test]
    fn test_inverse_of_set_tempo_remove_round_trips() {
        // Remove an existing (ramp) tempo change → inverse must re-apply it verbatim.
        let remove = UndoAction::SetTempo {
            tick: Tick(1920),
            old: Some((Bpm::new(96.0), true)),
            new: None,
        };
        let inv = UndoManager::inverse(&remove);
        if let UndoAction::SetTempo { old, new, .. } = inv {
            assert_eq!(old, None);
            assert_eq!(new, Some((Bpm::new(96.0), true)));
        } else {
            panic!("Expected SetTempo inverse");
        }
    }

    #[test]
    fn test_inverse_of_move_tempo_swaps_endpoints() {
        // Dragging a point (tick + bpm change) → inverse must restore the origin.
        let mv = UndoAction::MoveTempo {
            old: (Tick(960), Bpm::new(120.0), false),
            new: (Tick(1440), Bpm::new(150.0), false),
        };
        let inv = UndoManager::inverse(&mv);
        if let UndoAction::MoveTempo { old, new } = inv {
            assert_eq!(old, (Tick(1440), Bpm::new(150.0), false));
            assert_eq!(new, (Tick(960), Bpm::new(120.0), false));
        } else {
            panic!("Expected MoveTempo inverse");
        }
    }

    #[test]
    fn test_inverse_of_composite_reverses_order() {
        let action = UndoAction::Composite(vec![
            UndoAction::AddNote {
                pattern_id: test_pattern_id(),
                note: test_note_snapshot(),
            },
            UndoAction::MoveNote {
                pattern_id: test_pattern_id(),
                note_id: NoteId(1),
                old_start: PatternTick(0),
                new_start: PatternTick(100),
            },
        ]);
        let inv = UndoManager::inverse(&action);
        if let UndoAction::Composite(actions) = inv {
            assert_eq!(actions.len(), 2);
            // First in inverse should be inverse of MoveNote (was second).
            assert_matches!(actions[0], UndoAction::MoveNote { .. });
            // Second should be inverse of AddNote (was first) => RemoveNote.
            assert_matches!(actions[1], UndoAction::RemoveNote { .. });
        } else {
            panic!("Expected Composite inverse");
        }
    }
}
