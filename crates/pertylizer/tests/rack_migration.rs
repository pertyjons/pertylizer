//! Load-time rack→graph migration, exercised on a real shipped project.
//!
//! The "Expression & Note-Processor Demo" fixture carries several patterns with
//! legacy `NoteProcessor` racks (arpeggiators, chords, humanize). Loading it and
//! running the migration (the same call `apply_project` makes on every load)
//! must fold each rack into a pooled Note Grid graph, leaving no pattern on the
//! retired rack path.

use pertylizer::project::ProjectFile;

#[test]
fn demo_fixture_racks_migrate_to_note_graphs() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/examples/projects/Expression & Note-Processor Demo.ptz"
    );
    let project = ProjectFile::load(path).expect("load demo fixture");
    let mut song = project.song;

    // Patterns carrying a legacy rack (and not already graph-bound) before load.
    let racked: Vec<_> = song
        .patterns()
        .filter(|p| !p.processors().is_empty() && p.note_graph().is_none())
        .map(|p| p.id)
        .collect();
    assert!(
        !racked.is_empty(),
        "fixture should ship note-processor racks to migrate"
    );
    let pool_before = song.note_graphs().count();

    let migrated = song.migrate_processor_racks_to_graphs();
    assert_eq!(migrated, racked.len(), "every racked pattern migrates");
    assert_eq!(
        song.note_graphs().count(),
        pool_before + racked.len(),
        "one new pooled graph per migrated rack"
    );

    // Every racked pattern is now bound to a real, non-empty graph with its rack
    // drained — nothing left on the legacy path.
    for pid in racked {
        let p = song.pattern(pid).expect("pattern still present");
        assert!(p.processors().is_empty(), "rack drained after migration");
        let gid = p.note_graph().expect("pattern now graph-bound");
        let graph = song.note_graph(gid).expect("bound graph resolves in pool");
        assert!(graph.node_count() > 0, "migrated graph has nodes");
    }

    // Idempotent: a re-run (e.g. a re-load of the just-migrated song) is a no-op.
    assert_eq!(song.migrate_processor_racks_to_graphs(), 0);
}
