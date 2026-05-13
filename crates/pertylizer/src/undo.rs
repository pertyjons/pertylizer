//! Undo/redo system for sequencer, module, and connection operations.
//!
//! The `UndoManager` stores actions on undo/redo stacks and returns them
//! to the caller for execution. The caller (GUI code) is responsible for
//! actually applying the inverse action via the existing session/song APIs.

use synth_engine::ModuleId;
use synth_engine::graph::Connection;
use synth_engine::instrument::InstrumentId;
use synth_sequencer::{
    Duration as SeqDuration, NoteId, PatternId, PatternTick, Pitch, Tick, TrackId, Velocity,
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
    pub(crate) instrument: synth_sequencer::SeqInstrumentId,
    pub(crate) track: Option<synth_sequencer::TrackId>,
}

impl From<&synth_sequencer::Note> for NoteSnapshot {
    fn from(note: &synth_sequencer::Note) -> Self {
        Self {
            id: note.id,
            start: note.start,
            duration: note.duration,
            pitch: note.pitch,
            velocity: note.velocity,
            instrument: note.instrument,
            track: note.track,
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
            instrument: synth_sequencer::SeqInstrumentId::new(0),
            track: None,
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
        assert!(matches!(undone, UndoAction::RemoveNote { .. }));
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
        assert!(matches!(redone, UndoAction::AddNote { .. }));
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
            assert!(matches!(actions[0], UndoAction::MoveNote { .. }));
            // Second should be inverse of AddNote (was first) => RemoveNote.
            assert!(matches!(actions[1], UndoAction::RemoveNote { .. }));
        } else {
            panic!("Expected Composite inverse");
        }
    }
}
