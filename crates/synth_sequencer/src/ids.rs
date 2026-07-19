//! Type-safe identifiers for sequencer entities.

use serde::{Deserialize, Serialize};

macro_rules! impl_id_display {
    ($($type:ty),+ $(,)?) => {
        $(
            impl std::fmt::Display for $type {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    self.0.fmt(f)
                }
            }
        )+
    };
}

/// Unique identifier for a pattern.
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct PatternId(pub u32);

impl PatternId {
    /// Create a new pattern ID.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

/// Unique identifier for a Note Grid graph in the project pool.
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct NoteGraphId(pub u32);

impl NoteGraphId {
    /// Create a new note-graph ID.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

/// Unique identifier for a module (node) within a single [`NoteGraphId`] graph.
///
/// Only unique within its owning graph — different graphs reuse the same small
/// id space.
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct NoteModuleId(pub u32);

impl NoteModuleId {
    /// Create a new note-module ID.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

/// Unique identifier for a Mod Grid graph in the project pool.
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct ModGraphId(pub u32);

impl ModGraphId {
    /// Create a new mod-graph ID.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

/// Unique identifier for a node within a single [`ModGraphId`] graph.
///
/// Only unique within its owning graph — different graphs reuse the same small
/// id space.
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct ModNodeId(pub u32);

impl ModNodeId {
    /// Create a new mod-node ID.
    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }
}

// ============================================================================
// INDEX TYPES
// ============================================================================

/// Index for track within a pattern (0-255).
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
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
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get as usize for array indexing.
    #[inline]
    #[must_use]
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

/// Voice/polyphony lane a note occupies in the tracker grid (0-255).
///
/// Purely an editor/layout concept: it groups simultaneous notes into stable
/// tracker columns and is **not** consulted at playback. Defaults to lane 0, so
/// piano-roll-created notes and pre-lane projects all land in the first column.
/// Distinct from [`TrackId`] (mono-per-track routing) and from engine voice
/// allocation — this only describes a note's column in the tracker view.
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct NoteLane(u8);

impl NoteLane {
    /// First lane (index 0) — the default column.
    pub const ZERO: Self = Self(0);

    /// Create a new note lane.
    #[inline]
    pub const fn new(index: u8) -> Self {
        Self(index)
    }

    /// Get the raw u8 value.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get as usize for array/column indexing.
    #[inline]
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u8> for NoteLane {
    fn from(index: u8) -> Self {
        Self(index)
    }
}

impl From<usize> for NoteLane {
    fn from(index: usize) -> Self {
        Self(index.min(255) as u8)
    }
}

/// Index for row within a pattern (0-65535).
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
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
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Get as usize for array indexing.
    #[inline]
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    /// Get as u32 for calculations.
    #[inline]
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }

    /// Advance to next row.
    #[inline]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// Go to previous row (saturating).
    #[inline]
    pub const fn prev(self) -> Self {
        Self(self.0.saturating_sub(1))
    }

    /// Saturating addition.
    #[inline]
    pub const fn saturating_add(self, delta: u16) -> Self {
        Self(self.0.saturating_add(delta))
    }

    /// Saturating subtraction.
    #[inline]
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

/// Number of tracks in a pattern (1-255).
///
/// Clamped to a minimum of 1 — a pattern always has at least one track.
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
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

    /// Create a new track count (clamped to minimum 1).
    #[inline]
    pub const fn new(count: u8) -> Self {
        if count == 0 { Self(1) } else { Self(count) }
    }

    /// Get the raw u8 value.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get as usize for array sizing.
    #[inline]
    #[must_use]
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
        Self::new(count)
    }
}

impl From<usize> for TrackCount {
    fn from(count: usize) -> Self {
        Self::new(count.min(255) as u8)
    }
}

impl std::fmt::Display for TrackCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} tracks", self.0)
    }
}

/// Number of rows in a pattern (1-65535).
///
/// Clamped to a minimum of 1 — a pattern always has at least one row.
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct RowCount(u16);

impl RowCount {
    /// Standard 64 rows.
    pub const DEFAULT_64: Self = Self(64);

    /// 128 rows.
    pub const ROWS_128: Self = Self(128);

    /// 256 rows.
    pub const ROWS_256: Self = Self(256);

    /// Create a new row count (clamped to minimum 1).
    #[inline]
    pub const fn new(count: u16) -> Self {
        if count == 0 { Self(1) } else { Self(count) }
    }

    /// Get the raw u16 value.
    #[inline]
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Get as usize for array sizing.
    #[inline]
    #[must_use]
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
        Self::new(count)
    }
}

impl From<usize> for RowCount {
    fn from(count: usize) -> Self {
        Self::new(count.min(65535) as u16)
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
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
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
        // Guard against zero to prevent division-by-zero in tick_to_row / quantize
        if song_ticks < Self::MIN.0 {
            Self::MIN
        } else {
            Self(song_ticks)
        }
    }

    /// Get the raw u16 value (song ticks).
    #[inline]
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Get as u32 for calculations.
    #[inline]
    #[must_use]
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
#[must_use]
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub struct TrackId(pub u16);

impl TrackId {
    /// Create a new track ID.
    pub fn new(id: u16) -> Self {
        Self(id)
    }

    /// Return this ID as an array index.
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Unique identifier for an instrument.
///
/// Re-exported from `synth_core` so the sequencer data model, engine, project
/// format, MCP surface, and GUI all share one instrument-id namespace.
pub use synth_core::InstrumentId;

/// Unique identifier for a return bus (effect-send destination).
///
/// Return busses are an engine concept (a sub-mix with its own effect chain),
/// but tracks reference them by this id in their send taps, so the id lives in
/// the sequencer alongside the routing data it keys.
#[must_use]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct ReturnBusId(pub u16);

impl ReturnBusId {
    /// Create a new return-bus ID.
    pub fn new(id: u16) -> Self {
        Self(id)
    }
}

/// Unique identifier for a note within a pattern.
/// Used for selection, undo/redo, and editing operations.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NoteId(pub u64);

impl NoteId {
    /// Create a new note ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl_id_display!(
    PatternId,
    TrackId,
    ReturnBusId,
    NoteId,
    NoteGraphId,
    NoteModuleId,
    ModGraphId,
    ModNodeId,
);

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
