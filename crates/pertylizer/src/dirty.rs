//! Whether the open project has unsaved changes.
//!
//! Dirty state used to be a `bool` set by scattered `mark_dirty()` calls, which
//! meant every editor had to remember to report itself. Several did not: edits
//! made through the sequencer, the grids, the mixer and the sample editor all
//! mutate shared state directly, so the project could be closed without a prompt
//! after real work had been done.
//!
//! The mechanism here inverts that. Instead of asking editors to announce their
//! edits, it reads the edit counters the state owners already maintain, and
//! compares them against a baseline captured at load/save time:
//!
//! | Counter | Covers |
//! |---|---|
//! | [`SharedSong::revision`](synth_sequencer::SharedSong::revision) | notes, patterns, placements, tempo, time signatures, automation, Note/Mod Grid graphs, tracks, mixer controls, return buses and sends |
//! | [`SharedGraph::version`](synth_engine::SharedGraph::version) | instruments, modules, parameters, connections, return/master effect chains |
//! | [`SampleLibrary::revision`](synth_sampler::SampleLibrary::revision) | sample import/delete/rename/metadata and destructive DSP edits |
//! | [`ProjectRevision::ui`] | patch-canvas state the other three do not own — module positions, group boxes, canvas size, instrument colour |
//!
//! A new editor added to any of those subsystems is covered automatically,
//! because it has to go through the same shared state to have any effect.
//!
//! # Undoing back to the saved state
//!
//! The counters are monotonic, so an undo is just another mutation and cannot
//! be recognised as a return to a previous point. The undo stack answers that
//! instead: `SynthApp::is_dirty` also treats the project as clean when the
//! stack is back at the depth it had when the project was saved.
//!
//! That shortcut is only sound while *every* mutation is undoable, so it is
//! disabled for the rest of the session as soon as a change is seen that did
//! not pass through the undo manager (`SynthApp::observe_untracked_mutation`).
//! Erring toward "dirty" costs an unnecessary save prompt; erring the other way
//! would silently discard work.

use synth_core::ContentRevision;

/// The edit counters of every subsystem that owns part of a project, captured
/// at one instant.
///
/// Compared for equality only: a difference in any field means something
/// changed. The individual values are meaningful solely against another
/// snapshot from the same session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub(crate) struct ProjectRevision {
    /// Sequencer song: patterns, tracks, automation, grids, mixer, return buses.
    pub song: ContentRevision,
    /// Engine graph: instruments, modules, parameters, connections, effects.
    pub graph: ContentRevision,
    /// Sample library: imports, deletions, metadata, destructive edits.
    pub samples: ContentRevision,
    /// Explicitly reported GUI-only edits — instrument properties and project
    /// metadata, which live in `InstrumentUiState` rather than shared state.
    pub ui: ContentRevision,
    /// Fingerprint of the patch-canvas layout across all instruments: module
    /// positions, group boxes and their persisted fields.
    ///
    /// A fingerprint rather than a counter because the canvas has around thirty
    /// mutation points (every drag, snap, group edit, collapse) and none of
    /// them reported themselves — dragging a module never marked the project
    /// dirty even though positions are saved. Deriving the value from the data
    /// itself means a new mutation point cannot be forgotten.
    pub layout: u64,
}

impl ProjectRevision {
    /// Whether this snapshot differs from `baseline` — i.e. whether there are
    /// unsaved changes relative to the last load or save.
    #[must_use]
    pub(crate) fn differs_from(self, baseline: Self) -> bool {
        self != baseline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revision(song: u64, graph: u64, samples: u64, ui: u64, layout: u64) -> ProjectRevision {
        ProjectRevision {
            song: ContentRevision::new(song),
            graph: ContentRevision::new(graph),
            samples: ContentRevision::new(samples),
            ui: ContentRevision::new(ui),
            layout,
        }
    }

    #[test]
    fn an_unchanged_snapshot_is_not_dirty() {
        let baseline = revision(3, 7, 1, 0, 99);
        assert!(!baseline.differs_from(baseline));
    }

    /// Each subsystem must be able to make the project dirty on its own — that
    /// is the whole point of tracking them separately. In particular `layout`
    /// covers dragging a module, which reported nothing at all before.
    #[test]
    fn a_change_in_any_subsystem_is_dirty() {
        let baseline = revision(3, 7, 1, 0, 99);
        for changed in [
            revision(4, 7, 1, 0, 99),
            revision(3, 8, 1, 0, 99),
            revision(3, 7, 2, 0, 99),
            revision(3, 7, 1, 1, 99),
            revision(3, 7, 1, 0, 100),
        ] {
            assert!(
                changed.differs_from(baseline),
                "{changed:?} must differ from {baseline:?}",
            );
        }
    }

    /// Saving captures a new baseline, which makes the same state clean again
    /// without any counter being reset.
    #[test]
    fn re_baselining_clears_dirtiness() {
        let baseline = revision(3, 7, 1, 0, 99);
        let after_edit = revision(4, 7, 1, 0, 99);
        assert!(after_edit.differs_from(baseline));

        let saved = after_edit;
        assert!(!after_edit.differs_from(saved));
    }
}
