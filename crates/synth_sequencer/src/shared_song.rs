//! Mutable control-thread song storage with lock-free audio snapshots.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use arc_swap::{ArcSwap, Guard};
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::Song;

/// Shared song state with atomically published immutable audio snapshots.
#[must_use]
pub struct SharedSong {
    editable: RwLock<Song>,
    snapshot: ArcSwap<Song>,
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
        }
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
}

impl Deref for SharedSongWriteGuard<'_> {
    type Target = Song;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl DerefMut for SharedSongWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl Drop for SharedSongWriteGuard<'_> {
    fn drop(&mut self) {
        self.owner.snapshot.store(Arc::new(self.guard.clone()));
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
}
