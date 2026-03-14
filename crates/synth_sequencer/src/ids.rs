//! Type-safe identifiers for sequencer entities.

use serde::{Deserialize, Serialize};

/// Unique identifier for a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PatternId(pub u32);

impl PatternId {
    /// Create a new pattern ID.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

// ============================================================================
// INDEX TYPES
// ============================================================================

/// Index for track within a pattern (0-255).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct TrackIndex(u8);

impl TrackIndex {
    /// First track (index 0).
    pub const ZERO: Self = Self(0);

    /// Create a new track index.
    #[inline]
    pub const fn new(index: u8) -> Self {
        Self(index)
    }

    /// Get the raw u8 value.
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get as usize for array indexing.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u8> for TrackIndex {
    fn from(index: u8) -> Self {
        Self(index)
    }
}

impl From<usize> for TrackIndex {
    fn from(index: usize) -> Self {
        Self(index.min(255) as u8)
    }
}

impl std::fmt::Display for TrackIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "T{:02}", self.0)
    }
}

/// Index for row within a pattern (0-65535).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct RowIndex(u16);

impl RowIndex {
    /// First row (index 0).
    pub const ZERO: Self = Self(0);

    /// Create a new row index.
    #[inline]
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    /// Get the raw u16 value.
    #[inline]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Get as usize for array indexing.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Get as u32 for calculations.
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }

    /// Advance to next row.
    #[inline]
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// Go to previous row (saturating).
    #[inline]
    #[must_use]
    pub const fn prev(self) -> Self {
        Self(self.0.saturating_sub(1))
    }

    /// Saturating addition.
    #[inline]
    #[must_use]
    pub const fn saturating_add(self, delta: u16) -> Self {
        Self(self.0.saturating_add(delta))
    }

    /// Saturating subtraction.
    #[inline]
    #[must_use]
    pub const fn saturating_sub(self, delta: u16) -> Self {
        Self(self.0.saturating_sub(delta))
    }
}

impl From<u16> for RowIndex {
    fn from(index: u16) -> Self {
        Self(index)
    }
}

impl From<usize> for RowIndex {
    fn from(index: usize) -> Self {
        Self(index.min(65535) as u16)
    }
}

impl std::fmt::Display for RowIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02X}", self.0)
    }
}

/// Number of tracks in a pattern (1-256).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TrackCount(u8);

impl TrackCount {
    /// Single track.
    pub const ONE: Self = Self(1);

    /// Four tracks.
    pub const FOUR: Self = Self(4);

    /// Eight tracks.
    pub const EIGHT: Self = Self(8);

    /// Sixteen tracks.
    pub const SIXTEEN: Self = Self(16);

    /// Create a new track count.
    #[inline]
    pub const fn new(count: u8) -> Self {
        Self(count)
    }

    /// Get the raw u8 value.
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get as usize for array sizing.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Iterate over all track indices.
    pub fn indices(self) -> impl Iterator<Item = TrackIndex> {
        (0..self.0).map(TrackIndex::new)
    }
}

impl Default for TrackCount {
    fn default() -> Self {
        Self::FOUR
    }
}

impl From<u8> for TrackCount {
    fn from(count: u8) -> Self {
        Self(count)
    }
}

impl From<usize> for TrackCount {
    fn from(count: usize) -> Self {
        Self(count.min(255) as u8)
    }
}

impl std::fmt::Display for TrackCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} tracks", self.0)
    }
}

/// Number of rows in a pattern (1-65536).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RowCount(u16);

impl RowCount {
    /// Standard 64 rows.
    pub const DEFAULT_64: Self = Self(64);

    /// 128 rows.
    pub const ROWS_128: Self = Self(128);

    /// 256 rows.
    pub const ROWS_256: Self = Self(256);

    /// Create a new row count.
    #[inline]
    pub const fn new(count: u16) -> Self {
        Self(count)
    }

    /// Get the raw u16 value.
    #[inline]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Get as usize for array sizing.
    #[inline]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Iterate over all row indices.
    pub fn indices(self) -> impl Iterator<Item = RowIndex> {
        (0..self.0).map(RowIndex::new)
    }
}

impl Default for RowCount {
    fn default() -> Self {
        Self::DEFAULT_64
    }
}

impl From<u16> for RowCount {
    fn from(count: u16) -> Self {
        Self(count)
    }
}

impl From<usize> for RowCount {
    fn from(count: usize) -> Self {
        Self(count.min(65535) as u16)
    }
}

impl std::fmt::Display for RowCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} rows", self.0)
    }
}

/// Ticks per row in song tick units (960 PPQN).
///
/// Controls how many song ticks pass before advancing to the next row.
/// Default: 240 song ticks/row = 4 rows per quarter note (16th note resolution).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TicksPerRow(u16);

impl TicksPerRow {
    /// Default: 240 song ticks per row.
    /// This gives 4 rows per quarter note (16th note resolution).
    pub const DEFAULT: Self = Self(240);

    /// Minimum (fastest): 40 song ticks per row.
    pub const MIN: Self = Self(40);

    /// Maximum (slowest): 1240 song ticks per row.
    pub const MAX: Self = Self(1240);

    /// Create a new ticks per row value from song ticks.
    #[inline]
    pub const fn new(song_ticks: u16) -> Self {
        Self(song_ticks)
    }

    /// Get the raw u16 value (song ticks).
    #[inline]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Get as u32 for calculations.
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }
}

impl Default for TicksPerRow {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<u16> for TicksPerRow {
    fn from(ticks: u16) -> Self {
        Self(ticks)
    }
}

/// Unique identifier for a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
