//! Crash recovery: periodic snapshots of unsaved work, kept beside the user's
//! own files rather than on top of them.
//!
//! # Why snapshots do not go to the project file
//!
//! An autosave that wrote to the project the user opened would silently commit
//! edits they had not agreed to keep — the "close without saving" escape hatch
//! would stop working. So every snapshot lands in a private recovery directory
//! and the manual file is only ever touched by an explicit save.
//!
//! # When recovery is offered
//!
//! A snapshot is *retired* — deleted — whenever the work it protects is no
//! longer at risk: after a successful manual save, and when the user explicitly
//! discards their changes. So a snapshot still present at startup means the
//! previous run ended with unsaved edits, whether from a crash, a power loss, or
//! a kill. That is exactly the set of cases where recovery should be offered,
//! and it needs no lock file or shutdown flag to detect.
//!
//! As a second guard, [`RecoveryEntry::supersedes_manual_save`] compares the
//! snapshot against the manual file it belongs to, so a stale snapshot left over
//! from a save that happened outside the app is not offered on top of newer
//! work.
//!
//! # Bounds
//!
//! Recovery storage is capped by both age ([`MAX_AGE`]) and count
//! ([`MAX_ENTRIES`]), pruned on startup. Without that, every abandoned untitled
//! project would leave a snapshot behind forever.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::project::ProjectFile;

/// Snapshots older than this are pruned unread. Long enough to survive a
/// weekend away from the machine, short enough that the directory does not
/// accumulate indefinitely.
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// At most this many snapshots are kept; the oldest are pruned first. Bounds
/// the directory when many projects are abandoned inside [`MAX_AGE`].
const MAX_ENTRIES: usize = 20;

/// Extension for the snapshot's metadata sidecar.
const META_EXTENSION: &str = "json";

/// Errors from reading or writing the recovery store.
///
/// Autosave failures are reported to the user without interrupting them and
/// never change dirty state — a snapshot that could not be written is a reason
/// to keep the work marked unsaved, not to pretend it is safe.
#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    /// No platform directory to store snapshots in.
    #[error("Cannot determine a recovery directory: {0}")]
    NoDirectory(&'static str),
    /// The recovery directory could not be created.
    #[error("Cannot create recovery directory {dir}: {source}")]
    CreateDirectory {
        /// The directory that could not be created.
        dir: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// Writing the snapshot or its metadata failed.
    #[error("Cannot write recovery snapshot: {0}")]
    Write(String),
    /// Reading a snapshot back failed.
    ///
    /// The cause is carried as text rather than a `source` field: `thiserror`
    /// reserves that name for a nested `Error`, and the underlying failure here
    /// is a `PatchError` already formatted for display.
    #[error("Cannot read recovery snapshot {path}: {reason}")]
    Read {
        /// The snapshot that could not be read.
        path: PathBuf,
        /// What went wrong.
        reason: String,
    },
}

/// What a snapshot is a snapshot *of*, recorded alongside it.
///
/// Without this the recovery prompt could only offer an anonymous file. The
/// original path is what lets a recovered document be reconnected to the
/// project it came from, and what [`RecoveryEntry::supersedes_manual_save`]
/// compares against.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryMeta {
    /// The manual project file these edits belong to, if the project has ever
    /// been saved. `None` for an untitled project, which has nothing on disk to
    /// compare against or return to.
    pub project_path: Option<PathBuf>,
    /// Display name, so the prompt can say what is being recovered even for an
    /// untitled project.
    pub project_name: String,
    /// When the snapshot was taken, as seconds since the Unix epoch.
    pub saved_at_unix: u64,
}

impl RecoveryMeta {
    /// The snapshot's age relative to `now`, or zero if it is somehow in the
    /// future (a clock adjustment between the write and this read).
    #[must_use]
    pub fn age(&self, now: SystemTime) -> Duration {
        let taken = SystemTime::UNIX_EPOCH + Duration::from_secs(self.saved_at_unix);
        now.duration_since(taken).unwrap_or(Duration::ZERO)
    }
}

/// One recoverable snapshot on disk: the project data plus its metadata.
#[derive(Debug, Clone)]
pub struct RecoveryEntry {
    /// The snapshot file itself.
    pub snapshot_path: PathBuf,
    /// What it is a snapshot of.
    pub meta: RecoveryMeta,
}

impl RecoveryEntry {
    /// Whether this snapshot holds work that is *not* already in the manual
    /// project file — the question the recovery prompt actually needs answered.
    ///
    /// An untitled project has no manual file, so its snapshot is by definition
    /// the only copy. A snapshot older than the file it belongs to describes
    /// work that has since been saved (or superseded by a save made elsewhere),
    /// and offering it would invite the user to overwrite newer work with older.
    #[must_use]
    pub fn supersedes_manual_save(&self) -> bool {
        let Some(manual) = &self.meta.project_path else {
            return true;
        };
        let Ok(manual_modified) = std::fs::metadata(manual).and_then(|m| m.modified()) else {
            // The manual file is gone or unreadable, so the snapshot is all
            // that is left of this project.
            return true;
        };
        let snapshot_taken = SystemTime::UNIX_EPOCH + Duration::from_secs(self.meta.saved_at_unix);
        snapshot_taken > manual_modified
    }

    /// Whether the snapshot is a ZIP bundle with embedded samples rather than
    /// plain JSON.
    ///
    /// The two share a filename, so this sniffs the file's magic. A caller that
    /// gets `true` must load through [`crate::bundle::load_bundle`] so the
    /// samples come back with the project; [`Self::load`] reads the JSON form
    /// only.
    #[must_use]
    pub fn load_is_bundle(&self) -> bool {
        crate::bundle::is_zip_file(&self.snapshot_path)
    }

    /// Load the snapshot's project data.
    ///
    /// Only valid for a plain-JSON snapshot — check [`Self::load_is_bundle`]
    /// first, or the embedded samples are silently dropped.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::Read`] if the file is missing or unparseable.
    pub fn load(&self) -> Result<ProjectFile, RecoveryError> {
        ProjectFile::load(&self.snapshot_path).map_err(|e| RecoveryError::Read {
            path: self.snapshot_path.clone(),
            reason: e.to_string(),
        })
    }
}

/// The on-disk recovery directory.
///
/// Holds one snapshot plus one metadata sidecar per project, keyed by a stable
/// id derived from the project's path so reopening the same project reuses its
/// slot instead of accumulating copies.
#[derive(Debug, Clone)]
pub struct RecoveryStore {
    dir: PathBuf,
}

impl RecoveryStore {
    /// Open the platform recovery directory, creating it if needed.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError`] if no data directory can be determined or the
    /// directory cannot be created.
    pub fn open() -> Result<Self, RecoveryError> {
        let base = dirs::data_dir()
            .or_else(dirs::home_dir)
            .ok_or(RecoveryError::NoDirectory(
                "could not determine home directory",
            ))?;
        Self::open_in(base.join("pertylizer").join("recovery"))
    }

    /// Open a specific directory as the recovery store, creating it if needed.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::CreateDirectory`] if the directory cannot be
    /// created.
    pub fn open_in(dir: PathBuf) -> Result<Self, RecoveryError> {
        std::fs::create_dir_all(&dir).map_err(|source| RecoveryError::CreateDirectory {
            dir: dir.clone(),
            source,
        })?;
        Ok(Self { dir })
    }

    /// The snapshot id for a project, stable across sessions.
    ///
    /// Derived from the manual file's path so reopening a project overwrites
    /// its own snapshot rather than adding another. An untitled project has no
    /// path to key on and gets a fixed slot: there can only be one untitled
    /// project open at a time, and pinning it means a crash-and-relaunch cycle
    /// does not leave a trail of orphans.
    ///
    /// Hashed with [`splitmix64`](synth_core::hash::splitmix64) rather than
    /// `DefaultHasher`, whose algorithm is explicitly unspecified across Rust
    /// releases. A toolchain upgrade would silently re-key every project's slot:
    /// existing snapshots would stop being found by their own project, linger
    /// until pruned, and a project could then be offered its own stale copy.
    /// Nothing here needs collision resistance, only stability over time.
    #[must_use]
    fn snapshot_id(project_path: Option<&Path>) -> String {
        let Some(path) = project_path else {
            return "untitled".to_string();
        };
        // Fold the path's bytes through the mixer one at a time. Cheap: a path
        // is short and this runs once per snapshot, not per frame.
        let digest = path
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .fold(0u64, |acc, byte| {
                synth_core::hash::splitmix64(acc ^ u64::from(*byte))
            });
        format!("{digest:016x}")
    }

    /// Path of the snapshot file for a project.
    fn snapshot_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.ptz"))
    }

    /// Path of the metadata sidecar for a project.
    fn meta_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.{META_EXTENSION}"))
    }

    /// Write a snapshot of `project`, replacing this project's previous one.
    ///
    /// `library` is the project's sample library. When it holds anything the
    /// snapshot is written as a ZIP bundle with the samples embedded, exactly
    /// as a manual save would: a plain-JSON snapshot carries no audio, so
    /// recovering one would hand the user their project back with every sample
    /// gone — including recorded and generated ones that exist nowhere else.
    /// The filename does not change; [`RecoveryEntry::load_is_bundle`] sniffs
    /// the ZIP magic instead.
    ///
    /// Both files are written atomically, and the snapshot is written *before*
    /// the metadata: a crash between the two leaves a snapshot with no sidecar,
    /// which [`Self::list`] skips, rather than a sidecar promising data that is
    /// not there.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::Write`] if either file cannot be written.
    pub fn write_snapshot(
        &self,
        project: &ProjectFile,
        library: Option<&synth_sampler::SampleLibrary>,
        project_path: Option<&Path>,
        project_name: &str,
        now: SystemTime,
    ) -> Result<RecoveryEntry, RecoveryError> {
        let id = Self::snapshot_id(project_path);
        let snapshot_path = self.snapshot_path(&id);

        match library.filter(|lib| !lib.is_empty()) {
            Some(lib) => crate::bundle::save_bundle(project, lib, &snapshot_path),
            None => project.save(&snapshot_path),
        }
        .map_err(|e| RecoveryError::Write(e.to_string()))?;

        let meta = RecoveryMeta {
            project_path: project_path.map(Path::to_path_buf),
            project_name: project_name.to_string(),
            saved_at_unix: now
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs(),
        };
        let json =
            serde_json::to_string_pretty(&meta).map_err(|e| RecoveryError::Write(e.to_string()))?;
        crate::io::atomic::write(&self.meta_path(&id), json.as_bytes())
            .map_err(|e| RecoveryError::Write(e.to_string()))?;

        Ok(RecoveryEntry {
            snapshot_path,
            meta,
        })
    }

    /// Delete the snapshot for a project, if any.
    ///
    /// Called when the work it protects is no longer at risk — after a
    /// successful manual save, or when the user discards their changes. Missing
    /// files are not an error: retiring a snapshot that was never written is the
    /// normal case for a project saved without ever going dirty.
    pub fn retire(&self, project_path: Option<&Path>) {
        let id = Self::snapshot_id(project_path);
        for path in [self.snapshot_path(&id), self.meta_path(&id)] {
            if let Err(e) = std::fs::remove_file(&path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(
                    target: "pertylizer::recovery",
                    path = %path.display(),
                    error = %e,
                    "could not retire recovery snapshot",
                );
            }
        }
    }

    /// Every snapshot in the store that has both its data and its metadata,
    /// newest first.
    #[must_use]
    pub fn list(&self) -> Vec<RecoveryEntry> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut found: Vec<RecoveryEntry> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let meta_path = entry.path();
                if meta_path.extension()?.to_str()? != META_EXTENSION {
                    return None;
                }
                let id = meta_path.file_stem()?.to_str()?.to_string();
                let snapshot_path = self.snapshot_path(&id);
                if !snapshot_path.is_file() {
                    // A snapshot that never got its sidecar written, or whose
                    // data was removed by hand. Nothing to recover.
                    return None;
                }
                let raw = std::fs::read_to_string(&meta_path).ok()?;
                let meta: RecoveryMeta = serde_json::from_str(&raw).ok()?;
                Some(RecoveryEntry {
                    snapshot_path,
                    meta,
                })
            })
            .collect();
        // Newest first, so callers can take the head and so pruning by count
        // drops the oldest.
        found.sort_by_key(|entry| std::cmp::Reverse(entry.meta.saved_at_unix));
        found
    }

    /// Delete snapshots that are too old or beyond the count cap, and report
    /// how many went.
    ///
    /// Run at startup, before offering recovery, so an abandoned project's
    /// snapshot does not linger indefinitely.
    pub fn prune(&self, now: SystemTime) -> usize {
        let entries = self.list();
        let mut removed = 0;
        for (index, entry) in entries.iter().enumerate() {
            let too_old = entry.meta.age(now) > MAX_AGE;
            let beyond_cap = index >= MAX_ENTRIES;
            if too_old || beyond_cap {
                self.retire(entry.meta.project_path.as_deref());
                // An untitled entry keys on a fixed id rather than a path, so
                // retire by path would miss it; remove its files directly.
                let _ = std::fs::remove_file(&entry.snapshot_path);
                let _ = std::fs::remove_file(entry.snapshot_path.with_extension(META_EXTENSION));
                removed += 1;
            }
        }
        removed
    }

    /// The snapshot worth offering to recover on startup, if any.
    ///
    /// Prunes first, then returns the newest snapshot that still holds work not
    /// present in its manual file.
    #[must_use]
    pub fn find_recoverable(&self, now: SystemTime) -> Option<RecoveryEntry> {
        let pruned = self.prune(now);
        if pruned > 0 {
            tracing::info!(
                target: "pertylizer::recovery",
                count = pruned,
                "pruned stale recovery snapshots",
            );
        }
        self.list()
            .into_iter()
            .find(RecoveryEntry::supersedes_manual_save)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::GlobalProjectState;

    fn store() -> (tempfile::TempDir, RecoveryStore) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = RecoveryStore::open_in(dir.path().join("recovery")).expect("open store");
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

    /// A fixed point far enough from the epoch that ages and comparisons behave
    /// like real timestamps.
    fn now() -> SystemTime {
        at(1_700_000_000)
    }

    #[test]
    fn a_snapshot_round_trips() {
        let (_dir, store) = store();
        let path = PathBuf::from("/songs/track.ptz");

        store
            .write_snapshot(&project("track"), None, Some(&path), "track", now())
            .expect("write");

        let entries = store.list();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].meta.project_path.as_deref(),
            Some(path.as_path())
        );
        assert_eq!(entries[0].load().expect("load").song.name, "track");
        assert!(
            !entries[0].load_is_bundle(),
            "a project with no samples stays plain JSON",
        );
    }

    /// A project with samples must snapshot its audio too. A JSON-only snapshot
    /// would hand the user back a song with every sample gone — including
    /// recorded ones that exist nowhere else on disk.
    #[test]
    fn a_snapshot_of_a_project_with_samples_keeps_the_samples() {
        use synth_sampler::{Sample, SampleLibrary, SampleMeta, SampleSource};

        let (_dir, store) = store();
        let mut library = SampleLibrary::new();
        library.add(Sample::new(
            SampleMeta {
                id: synth_sampler::SampleId::new(0),
                name: "blip".to_string(),
                description: String::new(),
                sample_rate: synth_core::audio::DeviceSampleRate::new(44_100),
                channels: synth_core::ChannelCount::Mono,
                frame_count: synth_core::SampleCount::new(4),
                root_note: None,
                loop_region: None,
                crop: None,
                source: SampleSource::Generated,
            },
            vec![0.0_f32, 0.5, -0.5, 0.0].into(),
        ));

        let entry = store
            .write_snapshot(&project("sampled"), Some(&library), None, "sampled", now())
            .expect("write");

        assert!(
            entry.load_is_bundle(),
            "a project with samples must snapshot as a bundle",
        );
        let mut restored = SampleLibrary::new();
        let recovered = crate::bundle::load_bundle(&entry.snapshot_path, &mut restored)
            .expect("the snapshot must load as a bundle");
        assert_eq!(recovered.song.name, "sampled");
        assert_eq!(restored.len(), 1, "the sample must come back with the song");
    }

    /// Reopening the same project must reuse its slot, or every session would
    /// leave another copy behind.
    #[test]
    fn re_snapshotting_a_project_replaces_its_previous_snapshot() {
        let (_dir, store) = store();
        let path = PathBuf::from("/songs/track.ptz");

        store
            .write_snapshot(&project("first"), None, Some(&path), "track", at(1_000))
            .expect("first");
        store
            .write_snapshot(&project("second"), None, Some(&path), "track", at(2_000))
            .expect("second");

        let entries = store.list();
        assert_eq!(entries.len(), 1, "one project, one snapshot");
        assert_eq!(entries[0].load().expect("load").song.name, "second");
    }

    /// Different projects get separate slots.
    #[test]
    fn separate_projects_get_separate_snapshots() {
        let (_dir, store) = store();

        store
            .write_snapshot(
                &project("a"),
                None,
                Some(Path::new("/songs/a.ptz")),
                "a",
                now(),
            )
            .expect("a");
        store
            .write_snapshot(
                &project("b"),
                None,
                Some(Path::new("/songs/b.ptz")),
                "b",
                now(),
            )
            .expect("b");

        assert_eq!(store.list().len(), 2);
    }

    /// A successful manual save retires the snapshot, so the next launch has
    /// nothing to offer.
    #[test]
    fn retiring_removes_the_snapshot() {
        let (_dir, store) = store();
        let path = PathBuf::from("/songs/track.ptz");
        store
            .write_snapshot(&project("track"), None, Some(&path), "track", now())
            .expect("write");

        store.retire(Some(&path));

        assert!(store.list().is_empty());
        assert!(
            store.find_recoverable(now()).is_none(),
            "nothing should be offered after a save",
        );
    }

    /// Retiring a project that never had a snapshot is the normal case for a
    /// save with no unsaved changes, and must not error or log noise.
    #[test]
    fn retiring_a_missing_snapshot_is_harmless() {
        let (_dir, store) = store();
        store.retire(Some(Path::new("/songs/never-saved.ptz")));
        assert!(store.list().is_empty());
    }

    /// The core promise: a snapshot left behind by a crash is offered on the
    /// next launch.
    #[test]
    fn a_leftover_snapshot_is_offered_for_recovery() {
        let (_dir, store) = store();
        // No manual file on disk at all — an untitled project, the case where
        // the snapshot is the only copy of the work.
        store
            .write_snapshot(&project("unsaved work"), None, None, "Untitled", now())
            .expect("write");

        let found = store.find_recoverable(now()).expect("must offer recovery");
        assert_eq!(found.load().expect("load").song.name, "unsaved work");
    }

    /// A snapshot older than the project file it belongs to describes work that
    /// has since been saved. Offering it would walk newer work backwards.
    #[test]
    fn a_snapshot_older_than_its_manual_file_is_not_offered() {
        let (dir, store) = store();
        let manual = dir.path().join("track.ptz");
        project("saved").save(&manual).expect("save manual");

        // Snapshot timestamped well before the manual file, which was just
        // written and therefore has a current mtime.
        store
            .write_snapshot(&project("stale"), None, Some(&manual), "track", at(1_000))
            .expect("write");

        assert!(
            store.find_recoverable(now()).is_none(),
            "a stale snapshot must not be offered over a newer manual save",
        );
    }

    /// If the manual file is gone, the snapshot is the only surviving copy and
    /// must still be offered.
    #[test]
    fn a_snapshot_whose_manual_file_vanished_is_still_offered() {
        let (_dir, store) = store();
        store
            .write_snapshot(
                &project("orphan"),
                None,
                Some(Path::new("/nowhere/deleted.ptz")),
                "deleted",
                now(),
            )
            .expect("write");

        assert!(store.find_recoverable(now()).is_some());
    }

    /// Abandoned snapshots must not accumulate forever.
    #[test]
    fn snapshots_older_than_the_age_limit_are_pruned() {
        let (_dir, store) = store();
        store
            .write_snapshot(&project("ancient"), None, None, "Untitled", at(1_000))
            .expect("write");

        let removed = store.prune(at(1_000) + MAX_AGE + Duration::from_secs(1));

        assert_eq!(removed, 1);
        assert!(store.list().is_empty());
    }

    /// A snapshot inside the age limit survives pruning.
    #[test]
    fn recent_snapshots_survive_pruning() {
        let (_dir, store) = store();
        store
            .write_snapshot(&project("fresh"), None, None, "Untitled", now())
            .expect("write");

        assert_eq!(store.prune(now() + Duration::from_secs(60)), 0);
        assert_eq!(store.list().len(), 1);
    }

    /// The count cap bounds the directory even when every snapshot is recent.
    #[test]
    fn the_entry_count_is_capped() {
        let (_dir, store) = store();
        for i in 0..(MAX_ENTRIES + 5) {
            store
                .write_snapshot(
                    &project(&format!("p{i}")),
                    None,
                    Some(&PathBuf::from(format!("/songs/p{i}.ptz"))),
                    &format!("p{i}"),
                    // Distinct timestamps so "oldest first" is well defined.
                    at(1_000 + i as u64),
                )
                .expect("write");
        }
        assert_eq!(store.list().len(), MAX_ENTRIES + 5);

        store.prune(at(2_000));

        assert_eq!(store.list().len(), MAX_ENTRIES);
    }

    /// Pruning by count must keep the newest work, not whatever the filesystem
    /// happened to list first.
    #[test]
    fn pruning_by_count_keeps_the_newest_snapshots() {
        let (_dir, store) = store();
        for i in 0..(MAX_ENTRIES + 3) {
            store
                .write_snapshot(
                    &project(&format!("p{i}")),
                    None,
                    Some(&PathBuf::from(format!("/songs/p{i}.ptz"))),
                    &format!("p{i}"),
                    at(1_000 + i as u64),
                )
                .expect("write");
        }

        store.prune(at(2_000));

        let survivors = store.list();
        let oldest_survivor = survivors
            .iter()
            .map(|e| e.meta.saved_at_unix)
            .min()
            .expect("survivors");
        assert_eq!(
            oldest_survivor,
            1_000 + 3,
            "the three oldest snapshots should have been the ones pruned",
        );
    }

    /// A snapshot with no metadata sidecar cannot be described to the user, so
    /// it is skipped rather than offered as an unlabelled file.
    #[test]
    fn a_snapshot_without_metadata_is_ignored() {
        let (_dir, store) = store();
        store
            .write_snapshot(&project("x"), None, None, "Untitled", now())
            .expect("write");
        std::fs::remove_file(store.meta_path("untitled")).expect("remove meta");

        assert!(store.list().is_empty());
        assert!(store.find_recoverable(now()).is_none());
    }

    #[test]
    fn age_is_zero_for_a_snapshot_from_the_future() {
        let meta = RecoveryMeta {
            project_path: None,
            project_name: "x".to_string(),
            saved_at_unix: 2_000,
        };
        assert_eq!(meta.age(at(1_000)), Duration::ZERO);
    }
}
