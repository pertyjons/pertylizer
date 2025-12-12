//! Type-safe identifiers for sequencer entities.

use serde::{Deserialize, Serialize};

/// Unique identifier for a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PatternId(pub u32);

impl PatternId {
    /// Create a new pattern ID.
    pub fn new(id: u32) -> Self {
        Self(id)
    }
}

/// Unique identifier for a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackId(pub u16);

impl TrackId {
    /// Create a new track ID.
    pub fn new(id: u16) -> Self {
        Self(id)
    }
}

/// Unique identifier for an instrument in the sequencer.
///
/// This is a compact u16 ID used for pattern data storage.
/// For engine-level instrument identification, see `engine::instrument::InstrumentId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SeqInstrumentId(pub u16);

impl SeqInstrumentId {
    /// Create a new sequencer instrument ID.
    pub fn new(id: u16) -> Self {
        Self(id)
    }
}

/// Unique identifier for a note within a pattern.
/// Used for selection, undo/redo, and editing operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NoteId(pub u64);

impl NoteId {
    /// Create a new note ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_equality() {
        let id1 = PatternId(1);
        let id2 = PatternId(1);
        let id3 = PatternId(2);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(NoteId(1));
        set.insert(NoteId(2));
        set.insert(NoteId(1)); // Duplicate

        assert_eq!(set.len(), 2);
    }
}
