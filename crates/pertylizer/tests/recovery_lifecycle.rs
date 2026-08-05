//! The recovery store across a simulated crash-and-relaunch cycle.
//!
//! The unit tests in `crate::recovery` cover each operation in isolation. These
//! walk the sequences that actually decide whether a user gets their work back,
//! because the failure modes here are all ordering mistakes — retiring a
//! snapshot a moment before offering it, offering one that a save already
//! superseded, or overwriting the crashed session's snapshot with the empty
//! document shown behind the prompt.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use pertylizer::project::{GlobalProjectState, ProjectFile};
use pertylizer::recovery::RecoveryStore;

fn store() -> (tempfile::TempDir, RecoveryStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = RecoveryStore::open_in(dir.path().join("recovery")).expect("open");
    (dir, store)
}

fn project(name: &str) -> ProjectFile {
    ProjectFile::new(
        Vec::new(),
        0,
        None,
        synth_sequencer::Song::new(name),
        GlobalProjectState::default(),
    )
}

fn at(secs: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
}

/// Edit, autosave, crash, relaunch: the work comes back.
#[test]
fn work_lost_to_a_crash_is_offered_on_the_next_launch() {
    let (_dir, store) = store();

    // Session one: an untitled project with unsaved edits, autosaved once.
    store
        .write_snapshot(
            &project("half-written song"),
            None,
            None,
            "Untitled",
            at(1_000),
        )
        .expect("autosave");

    // Session one dies here — no retire, because no save and no clean exit.

    // Session two: startup looks for recoverable work.
    let offered = store
        .find_recoverable(at(1_100))
        .expect("the crashed session's work must be offered");
    assert_eq!(offered.load().expect("load").song.name, "half-written song",);
}

/// Edit, autosave, then save properly and exit: nothing is offered next launch.
#[test]
fn work_that_was_saved_is_not_offered_again() {
    let (dir, store) = store();
    let manual = dir.path().join("song.ptz");

    store
        .write_snapshot(
            &project("in progress"),
            None,
            Some(&manual),
            "song",
            at(1_000),
        )
        .expect("autosave");

    // The user saves. The app writes the manual file and retires the snapshot.
    project("in progress").save(&manual).expect("manual save");
    store.retire(Some(&manual));

    assert!(
        store.find_recoverable(at(1_100)).is_none(),
        "saved work must not be offered for recovery",
    );
}

/// Declining the offer must actually remove the snapshot, or the same prompt
/// would reappear on every launch forever.
#[test]
fn declining_recovery_stops_the_offer_recurring() {
    let (_dir, store) = store();
    let entry = store
        .write_snapshot(&project("unwanted"), None, None, "Untitled", at(1_000))
        .expect("autosave");

    // What `decline_recovery` does: retire by path, then remove the files
    // directly (an untitled entry is keyed by a fixed id, not a path).
    store.retire(entry.meta.project_path.as_deref());
    let _ = std::fs::remove_file(&entry.snapshot_path);
    let _ = std::fs::remove_file(entry.snapshot_path.with_extension("json"));

    assert!(store.find_recoverable(at(1_100)).is_none());
    assert!(store.list().is_empty());
}

/// A snapshot from before a save made in another program (or another window)
/// must not be offered — recovering it would silently roll the file back.
#[test]
fn a_manual_save_made_elsewhere_wins_over_an_older_snapshot() {
    let (dir, store) = store();
    let manual = dir.path().join("song.ptz");

    store
        .write_snapshot(
            &project("old draft"),
            None,
            Some(&manual),
            "song",
            at(1_000),
        )
        .expect("autosave");
    // The manual file is written *now*, so its mtime is far newer than the
    // snapshot's recorded timestamp.
    project("newer save").save(&manual).expect("manual save");

    assert!(
        store.find_recoverable(SystemTime::now()).is_none(),
        "an older snapshot must never be offered over a newer manual save",
    );
}

/// Two projects crashing in separate sessions both survive, and the more recent
/// one is what gets offered first.
#[test]
fn the_newest_recoverable_project_is_offered_first() {
    let (_dir, store) = store();
    store
        .write_snapshot(
            &project("older"),
            None,
            Some(&PathBuf::from("/songs/older.ptz")),
            "older",
            at(1_000),
        )
        .expect("write older");
    store
        .write_snapshot(
            &project("newer"),
            None,
            Some(&PathBuf::from("/songs/newer.ptz")),
            "newer",
            at(2_000),
        )
        .expect("write newer");

    let offered = store.find_recoverable(at(3_000)).expect("offer");
    assert_eq!(offered.meta.project_name, "newer");
}

/// A snapshot is a full project, not a diff — recovering must reproduce the
/// content, not just the fact that something was unsaved.
#[test]
fn a_recovered_snapshot_round_trips_its_content() {
    let (_dir, store) = store();
    let mut original = project("detailed");
    original.global.octave_offset = 3;

    store
        .write_snapshot(&original, None, None, "detailed", at(1_000))
        .expect("write");

    let recovered = store
        .find_recoverable(at(1_100))
        .expect("offer")
        .load()
        .expect("load");
    assert_eq!(recovered.song.name, "detailed");
    assert_eq!(recovered.global.octave_offset, 3);
}

/// Repeated autosaves of one project must not grow the store — a long session
/// takes a snapshot every 30 seconds.
#[test]
fn repeated_autosaves_of_one_project_keep_one_snapshot() {
    let (_dir, store) = store();
    let path = PathBuf::from("/songs/long-session.ptz");

    for minute in 0..120 {
        store
            .write_snapshot(
                &project(&format!("take {minute}")),
                None,
                Some(&path),
                "long-session",
                at(1_000 + minute * 30),
            )
            .expect("autosave");
    }

    let entries = store.list();
    assert_eq!(entries.len(), 1, "one project must occupy one slot");
    assert_eq!(entries[0].load().expect("load").song.name, "take 119");
}
