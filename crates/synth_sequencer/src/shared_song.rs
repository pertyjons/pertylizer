//! Mutable control-thread song storage with lock-free audio snapshots.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::{ArcSwap, Guard};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use synth_core::ContentRevision;

use crate::Song;

/// Shared song state with atomically published immutable audio snapshots.
#[must_use]
pub struct SharedSong {
    editable: RwLock<Song>,
    snapshot: ArcSwap<Song>,
    /// Bumped once per write guard that actually mutated the song. This is the
    /// single choke point every song edit passes through — notes, patterns,
    /// placements, tempo, time signatures, automation, note/mod graphs, tracks,
    /// return buses and their sends all live in `Song` — so the application
    /// shell can derive "there are unsaved changes" from one counter instead of
    /// relying on every editor to remember to report itself.
    revision: AtomicU64,
}

impl SharedSong {
    pub fn new(mut song: Song) -> Self {
        // Derived processing orders are skipped by serialization. Build them
        // while the song is still owned by the control thread so swapping this
        // object into the audio engine is a pointer-only operation.
        song.rebuild_note_graphs();
        song.rebuild_mod_graphs();
        Self {
            snapshot: ArcSwap::from_pointee(song.clone()),
            editable: RwLock::new(song),
            revision: AtomicU64::new(ContentRevision::INITIAL.as_u64()),
        }
    }

    /// The current edit revision. Changes exactly when the song is mutated.
    ///
    /// Taking a write guard is not enough on its own: a guard that only reads,
    /// or that bails out before touching anything, leaves this untouched.
    pub fn revision(&self) -> ContentRevision {
        ContentRevision::new(self.revision.load(Ordering::Acquire))
    }

    pub fn read(&self) -> RwLockReadGuard<'_, Song> {
        self.editable.read()
    }

    pub fn try_read(&self) -> Option<RwLockReadGuard<'_, Song>> {
        self.editable.try_read()
    }

    pub fn write(&self) -> SharedSongWriteGuard<'_> {
        SharedSongWriteGuard {
            owner: self,
            guard: self.editable.write(),
            mutated: false,
        }
    }

    /// Pin the current immutable song without acquiring a lock.
    pub fn snapshot(&self) -> Guard<Arc<Song>> {
        self.snapshot.load()
    }
}

/// Write guard that publishes the completed edit when released.
pub struct SharedSongWriteGuard<'a> {
    owner: &'a SharedSong,
    guard: RwLockWriteGuard<'a, Song>,
    /// Set by [`DerefMut`] — the only way to reach a `&mut Song` through this
    /// guard, and therefore an exact record of whether the holder edited
    /// anything. Callers routinely take a write guard and then bail out (a
    /// missing pattern id, an empty selection); those must not count as edits,
    /// or the project would look unsaved after a no-op.
    mutated: bool,
}

impl Deref for SharedSongWriteGuard<'_> {
    type Target = Song;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl DerefMut for SharedSongWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.mutated = true;
        &mut self.guard
    }
}

impl Drop for SharedSongWriteGuard<'_> {
    fn drop(&mut self) {
        if !self.mutated {
            // Nothing was touched, so the published snapshot is still current.
            // Skipping the full `Song` clone here is why read-only uses of
            // `write()` are merely wasteful rather than expensive.
            return;
        }
        self.owner.snapshot.store(Arc::new(self.guard.clone()));
        self.owner.revision.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_publish_only_after_guard_release() {
        let shared = SharedSong::new(Song::new("before"));
        {
            let mut song = shared.write();
            song.name = "after".to_string();
            assert_eq!(shared.snapshot().name, "before");
        }
        assert_eq!(shared.snapshot().name, "after");
    }

    #[test]
    fn a_fresh_song_starts_at_the_initial_revision() {
        let shared = SharedSong::new(Song::new("x"));
        assert_eq!(shared.revision(), ContentRevision::INITIAL);
    }

    #[test]
    fn mutating_through_the_guard_advances_the_revision() {
        let shared = SharedSong::new(Song::new("x"));
        let before = shared.revision();

        shared.write().name = "y".to_string();

        assert_ne!(shared.revision(), before);
    }

    /// The guard must not report an edit just for existing — several call sites
    /// take a write lock and then return early without touching the song.
    #[test]
    fn a_write_guard_that_only_reads_leaves_the_revision_alone() {
        let shared = SharedSong::new(Song::new("x"));
        let before = shared.revision();

        {
            let song = shared.write();
            let _ = song.name.len();
        }

        assert_eq!(shared.revision(), before);
    }

    /// Each guard counts once, so a caller can tell two successive edits apart.
    #[test]
    fn successive_edits_each_advance_the_revision() {
        let shared = SharedSong::new(Song::new("x"));

        shared.write().name = "first".to_string();
        let after_first = shared.revision();
        shared.write().name = "second".to_string();

        assert_ne!(shared.revision(), after_first);
    }

    /// Read guards are a separate path entirely and must never look like edits.
    #[test]
    fn read_guards_leave_the_revision_alone() {
        let shared = SharedSong::new(Song::new("x"));
        let before = shared.revision();

        let _ = shared.read().name.len();
        let _ = shared.snapshot().name.len();

        assert_eq!(shared.revision(), before);
    }
}
