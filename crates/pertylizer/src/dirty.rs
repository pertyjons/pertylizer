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
//! | Term | Covers |
//! |---|---|
//! | [`SharedSong::revision`](synth_sequencer::SharedSong::revision) | notes, patterns, placements, tempo, time signatures, automation, Note/Mod Grid graphs, tracks, mixer controls, return buses and sends |
//! | [`SharedGraph::version`](synth_engine::SharedGraph::version) | instruments, modules, parameters, connections |
//! | [`SampleLibrary::revision`](synth_sampler::SampleLibrary::revision) | sample import/delete/rename/metadata and destructive DSP edits |
//! | [`ProjectRevision::ui`] | GUI-owned state the others do not hold — instrument properties and project metadata |
//! | [`ProjectRevision::layout`] | patch-canvas layout: module positions, group boxes and their persisted fields |
//! | [`ProjectRevision::global`] | master volume, keyboard octave, glide, the transport loop region, and the master / return-bus effect chains |
//!
//! A new editor added to any of those subsystems is covered automatically,
//! because it has to go through the same shared state to have any effect.
//!
//! The last two are fingerprints rather than counters, for the same reason: the
//! state they watch is reached from too many places for "remember to report" to
//! hold. The `global` row is the one that had to be added after the fact — the
//! effect chains in particular are *not* part of `SharedGraph`, they are
//! `RwLock`s on `EngineState`, so the graph version never saw them.
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
    /// Fingerprint of the project's global state: master volume, keyboard
    /// octave, glide time, the transport loop region, and the master and
    /// return-bus effect chains.
    ///
    /// These are all written by the save path ([`GlobalProjectState`] plus the
    /// transport loop) and none of them are owned by the four counters above.
    /// Master volume is an engine atomic; octave and glide are GUI widget
    /// state; the loop region is mirrored out of the sequencer engine rather
    /// than kept in the `Song`; and the effect chains are `RwLock`s on
    /// `EngineState`, *not* on the `SharedGraphState` whose version `graph`
    /// reads. So before this existed, lowering the master fader or adding a
    /// master reverb changed the file that would be written while the project
    /// still reported itself clean — no `*`, no autosave snapshot, and no
    /// prompt on close.
    ///
    /// A fingerprint for the same reason `layout` is one: these values are
    /// reached from many places (GUI, MCP, project load, undo), and deriving
    /// the answer from the data means none of them has to remember to report.
    ///
    /// [`GlobalProjectState`]: crate::project::GlobalProjectState
    pub global: u64,
}

impl ProjectRevision {
    /// Whether this snapshot differs from `baseline` — i.e. whether there are
    /// unsaved changes relative to the last load or save.
    #[must_use]
    pub(crate) fn differs_from(self, baseline: Self) -> bool {
        self != baseline
    }
}

/// Fingerprint the global state that the save path reads straight out of the
/// engine and the keyboard widget.
///
/// Mirrors what `project_apply::build_project_from_engine` persists, and must
/// be kept in step with it: a field added to `GlobalProjectState` without a
/// term here is a field the user can change without the project noticing.
/// [`tests::global`] covers each term one mutation at a time.
pub(crate) fn global_fingerprint(
    engine: &synth_engine::EngineState,
    octave_offset: i32,
    glide_time: synth_core::Seconds,
) -> u64 {
    use std::hash::{Hash, Hasher};
    use synth_core::ModuleParam as _;
    use synth_core::hash::splitmix64;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    // Floats: hash the bit pattern. "The fader is at the same value" is exactly
    // the equality this needs, and it is the same value the save path writes.
    hasher.write_u32(engine.master_volume.load().to_bits());
    hasher.write_u32(glide_time.as_f32().to_bits());
    octave_offset.hash(&mut hasher);

    let (loop_enabled, loop_start, loop_end) = engine.transport.loop_state();
    loop_enabled.hash(&mut hasher);
    loop_start.0.hash(&mut hasher);
    loop_end.0.hash(&mut hasher);

    /// Hash one effect chain in processing order — order is persisted, so a
    /// reorder has to register as a change.
    fn hash_effects(hasher: &mut impl Hasher, effects: &[synth_engine::ReturnEffectSnapshot]) {
        effects.len().hash(hasher);
        for fx in effects {
            fx.module_id.hash(hasher);
            fx.module_type.hash(hasher);
            fx.bypassed.hash(hasher);
            // Each slot in `parameters` is a distinct param of a known module
            // type, so position plus value identifies it. `as_f32` is the same
            // numeric view the save path serializes through `ParamValue`.
            fx.parameters.len().hash(hasher);
            for param in &fx.parameters {
                hasher.write_u32(param.as_f32().to_bits());
            }
        }
    }

    let return_buses = engine.return_bus_effects.read();
    return_buses.len().hash(&mut hasher);
    for bus in return_buses.iter() {
        bus.id.0.hash(&mut hasher);
        hash_effects(&mut hasher, &bus.effects);
    }
    drop(return_buses);

    hash_effects(&mut hasher, &engine.master_effects.read());

    splitmix64(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revision(
        song: u64,
        graph: u64,
        samples: u64,
        ui: u64,
        layout: u64,
        global: u64,
    ) -> ProjectRevision {
        ProjectRevision {
            song: ContentRevision::new(song),
            graph: ContentRevision::new(graph),
            samples: ContentRevision::new(samples),
            ui: ContentRevision::new(ui),
            layout,
            global,
        }
    }

    #[test]
    fn an_unchanged_snapshot_is_not_dirty() {
        let baseline = revision(3, 7, 1, 0, 99, 42);
        assert!(!baseline.differs_from(baseline));
    }

    /// Each subsystem must be able to make the project dirty on its own — that
    /// is the whole point of tracking them separately. In particular `layout`
    /// covers dragging a module, which reported nothing at all before, and
    /// `global` covers the master fader and the effect chains, which reported
    /// nothing either.
    #[test]
    fn a_change_in_any_subsystem_is_dirty() {
        let baseline = revision(3, 7, 1, 0, 99, 42);
        for changed in [
            revision(4, 7, 1, 0, 99, 42),
            revision(3, 8, 1, 0, 99, 42),
            revision(3, 7, 2, 0, 99, 42),
            revision(3, 7, 1, 1, 99, 42),
            revision(3, 7, 1, 0, 100, 42),
            revision(3, 7, 1, 0, 99, 43),
        ] {
            assert!(
                changed.differs_from(baseline),
                "{changed:?} must differ from {baseline:?}",
            );
        }
    }

    /// Every term of the global fingerprint, one mutation at a time.
    ///
    /// Each of these is written by the save path and owned by none of the four
    /// counters, so before the fingerprint existed each one could be changed
    /// with the project still reporting itself clean. A term dropped from
    /// `global_fingerprint` — or a field added to `GlobalProjectState` without
    /// one — fails here.
    mod global {
        use super::*;
        use synth_core::Seconds;
        use synth_engine::{EngineState, ReturnBusSnapshot, ReturnEffectSnapshot};

        /// The fingerprint of an untouched engine with no keyboard offset and
        /// no glide, which every case below starts from.
        fn baseline(engine: &EngineState) -> u64 {
            global_fingerprint(engine, 0, Seconds::ZERO)
        }

        fn reverb(decay: f32, bypassed: bool) -> ReturnEffectSnapshot {
            use synth_core::params::{Param, ReverbParam};
            ReturnEffectSnapshot {
                module_id: synth_engine::ModuleId::new(synth_core::ModuleType::Reverb, 1),
                module_type: synth_core::ModuleType::Reverb,
                parameters: vec![Param::Reverb(ReverbParam::Decay(
                    synth_core::NormalizedValue::new(decay),
                ))],
                bypassed,
            }
        }

        #[test]
        fn master_volume_changes_the_fingerprint() {
            let engine = EngineState::new();
            let before = baseline(&engine);

            engine.master_volume.store(0.38);

            assert_ne!(baseline(&engine), before, "the master fader is saved state");
        }

        #[test]
        fn the_keyboard_octave_changes_the_fingerprint() {
            let engine = EngineState::new();

            assert_ne!(
                global_fingerprint(&engine, 1, Seconds::ZERO),
                baseline(&engine),
            );
        }

        #[test]
        fn glide_time_changes_the_fingerprint() {
            let engine = EngineState::new();

            assert_ne!(
                global_fingerprint(&engine, 0, Seconds::new(0.25)),
                baseline(&engine),
            );
        }

        /// The loop region is mirrored out of the sequencer engine rather than
        /// kept in the `Song`, so the song revision never sees it.
        #[test]
        fn the_transport_loop_region_changes_the_fingerprint() {
            let engine = EngineState::new();
            let before = baseline(&engine);

            engine.transport.set_loop_state(
                synth_sequencer::Tick(0),
                synth_sequencer::Tick(1920),
                true,
            );

            assert_ne!(baseline(&engine), before);
        }

        /// Master and return effect chains live on `EngineState`, not on the
        /// `SharedGraphState` whose version the `graph` counter reads — adding
        /// a master reverb used to leave the project reading clean.
        #[test]
        fn adding_a_master_effect_changes_the_fingerprint() {
            let engine = EngineState::new();
            let before = baseline(&engine);

            engine.master_effects.write().push(reverb(0.5, false));

            assert_ne!(baseline(&engine), before);
        }

        #[test]
        fn editing_a_master_effect_parameter_changes_the_fingerprint() {
            let engine = EngineState::new();
            engine.master_effects.write().push(reverb(0.5, false));
            let before = baseline(&engine);

            engine.master_effects.write()[0] = reverb(0.9, false);

            assert_ne!(baseline(&engine), before, "knob values are saved state");
        }

        #[test]
        fn bypassing_a_master_effect_changes_the_fingerprint() {
            let engine = EngineState::new();
            engine.master_effects.write().push(reverb(0.5, false));
            let before = baseline(&engine);

            engine.master_effects.write()[0] = reverb(0.5, true);

            assert_ne!(baseline(&engine), before);
        }

        /// Chain order is persisted, so a reorder has to register even though
        /// the set of effects is unchanged.
        #[test]
        fn reordering_a_master_chain_changes_the_fingerprint() {
            let engine = EngineState::new();
            {
                let mut fx = engine.master_effects.write();
                fx.push(reverb(0.2, false));
                fx.push(reverb(0.8, false));
            }
            let before = baseline(&engine);

            engine.master_effects.write().swap(0, 1);

            assert_ne!(baseline(&engine), before);
        }

        #[test]
        fn return_bus_effects_change_the_fingerprint() {
            let engine = EngineState::new();
            let before = baseline(&engine);

            engine.return_bus_effects.write().push(ReturnBusSnapshot {
                id: synth_sequencer::ReturnBusId::new(1),
                effects: vec![reverb(0.5, false)],
            });

            assert_ne!(baseline(&engine), before);
        }

        /// Reading the engine must not itself look like an edit, or a freshly
        /// opened project would report unsaved changes immediately.
        #[test]
        fn the_fingerprint_is_stable_across_reads() {
            let engine = EngineState::new();
            engine.master_effects.write().push(reverb(0.5, false));

            assert_eq!(baseline(&engine), baseline(&engine));
        }
    }

    /// Saving captures a new baseline, which makes the same state clean again
    /// without any counter being reset.
    #[test]
    fn re_baselining_clears_dirtiness() {
        let baseline = revision(3, 7, 1, 0, 99, 42);
        let after_edit = revision(4, 7, 1, 0, 99, 42);
        assert!(after_edit.differs_from(baseline));

        let saved = after_edit;
        assert!(!after_edit.differs_from(saved));
    }
}
