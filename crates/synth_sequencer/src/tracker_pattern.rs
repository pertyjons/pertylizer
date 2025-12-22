//! Tracker-style pattern with tracks and rows.
//!
//! This module provides a track-based pattern structure optimized for
//! tracker-style music (MOD, XM, IT, S3M formats).
//!
//! # Structure
//!
//! - [`TrackerPattern`] contains multiple [`Track`]s
//! - Each [`Track`] contains multiple [`Row`]s (one per pattern row)
//! - Each [`Row`] contains a [`Cell`] and optional effects
//!
//! # Voice Mapping
//!
//! Track index maps directly to voice index:
//! - Track 0 → Voice 0
//! - Track 1 → Voice 1
//! - etc.
//!
//! This means each track is monophonic - a new note on the same track
//! automatically cuts the previous note.

use serde::{Deserialize, Serialize};

use crate::effects::EffectCommand;
use crate::ids::{
    PatternId, RowCount, RowIndex, SeqInstrumentId, TicksPerRow, TrackCount, TrackIndex,
};
use crate::pitch::{Pitch, Velocity};
use crate::time::PatternTick;

// ============================================================================
// TrackerPattern
// ============================================================================

/// A tracker-style pattern with tracks and rows.
///
/// Each track maps to a single voice (`TrackIndex` N → `VoiceIndex` N).
/// All rows are explicitly stored, even empty ones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerPattern {
    id: PatternId,
    name: String,
    num_rows: RowCount,
    ticks_per_row: TicksPerRow,
    tracks: Vec<Track>,
}

impl TrackerPattern {
    /// Create a new empty pattern.
    #[must_use]
    pub fn new(id: PatternId, num_tracks: TrackCount, num_rows: RowCount) -> Self {
        Self {
            id,
            name: String::new(),
            num_rows,
            ticks_per_row: TicksPerRow::DEFAULT,
            tracks: (0..num_tracks.as_usize())
                .map(|_| Track::new(num_rows))
                .collect(),
        }
    }

    // === Accessors ===

    /// Get the pattern ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> PatternId {
        self.id
    }

    /// Get the pattern name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Set the pattern name.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Get the number of tracks.
    #[inline]
    #[must_use]
    pub fn num_tracks(&self) -> TrackCount {
        TrackCount::new(self.tracks.len() as u8)
    }

    /// Get the number of rows.
    #[inline]
    #[must_use]
    pub fn num_rows(&self) -> RowCount {
        self.num_rows
    }

    /// Get ticks per row.
    #[inline]
    #[must_use]
    pub fn ticks_per_row(&self) -> TicksPerRow {
        self.ticks_per_row
    }

    /// Set ticks per row.
    pub fn set_ticks_per_row(&mut self, ticks: TicksPerRow) {
        self.ticks_per_row = ticks;
    }

    /// Total pattern length in ticks.
    #[must_use]
    pub fn length_ticks(&self) -> PatternTick {
        PatternTick(self.num_rows.as_usize() as u32 * self.ticks_per_row.as_u32())
    }

    // === Row/Cell access ===

    /// Get a row at the specified position.
    ///
    /// # Panics
    /// Panics if track or row index is out of bounds.
    #[inline]
    #[must_use]
    pub fn get(&self, track: TrackIndex, row: RowIndex) -> &Row {
        &self.tracks[track.as_usize()].rows[row.as_usize()]
    }

    /// Get a mutable row at the specified position.
    ///
    /// # Panics
    /// Panics if track or row index is out of bounds.
    #[inline]
    pub fn get_mut(&mut self, track: TrackIndex, row: RowIndex) -> &mut Row {
        &mut self.tracks[track.as_usize()].rows[row.as_usize()]
    }

    /// Set the cell at the specified position.
    pub fn set_cell(&mut self, track: TrackIndex, row: RowIndex, cell: Cell) {
        self.tracks[track.as_usize()].rows[row.as_usize()].cell = cell;
    }

    /// Set effects at the specified position.
    pub fn set_effects(&mut self, track: TrackIndex, row: RowIndex, effects: Vec<EffectCommand>) {
        self.tracks[track.as_usize()].rows[row.as_usize()].effects = effects;
    }

    /// Add an effect at the specified position.
    pub fn add_effect(&mut self, track: TrackIndex, row: RowIndex, effect: EffectCommand) {
        self.tracks[track.as_usize()].rows[row.as_usize()]
            .effects
            .push(effect);
    }

    /// Clear the cell at the specified position (set to Empty).
    pub fn clear_cell(&mut self, track: TrackIndex, row: RowIndex) {
        self.tracks[track.as_usize()].rows[row.as_usize()].cell = Cell::Empty;
    }

    /// Clear effects at the specified position.
    pub fn clear_effects(&mut self, track: TrackIndex, row: RowIndex) {
        self.tracks[track.as_usize()].rows[row.as_usize()]
            .effects
            .clear();
    }

    // === Track access ===

    /// Get a track by index.
    #[inline]
    #[must_use]
    pub fn track(&self, index: TrackIndex) -> &Track {
        &self.tracks[index.as_usize()]
    }

    /// Get a mutable track by index.
    #[inline]
    pub fn track_mut(&mut self, index: TrackIndex) -> &mut Track {
        &mut self.tracks[index.as_usize()]
    }

    /// Get all tracks.
    #[must_use]
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    // === Iteration ===

    /// Iterate over all non-empty rows with their positions.
    ///
    /// Yields `(TrackIndex, RowIndex, &Row)` for each row that has
    /// a note, note-off, or effects.
    pub fn non_empty_rows(&self) -> impl Iterator<Item = (TrackIndex, RowIndex, &Row)> {
        self.tracks
            .iter()
            .enumerate()
            .flat_map(|(track_idx, track)| {
                track
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(move |(row_idx, row)| {
                        if !row.is_empty() {
                            Some((
                                TrackIndex::new(track_idx as u8),
                                RowIndex::new(row_idx as u16),
                                row,
                            ))
                        } else {
                            None
                        }
                    })
            })
    }

    /// Iterate over all rows at a specific row index across all tracks.
    pub fn row_across_tracks(&self, row: RowIndex) -> impl Iterator<Item = (TrackIndex, &Row)> {
        self.tracks
            .iter()
            .enumerate()
            .map(move |(track_idx, track)| {
                (
                    TrackIndex::new(track_idx as u8),
                    &track.rows[row.as_usize()],
                )
            })
    }

    /// Convert row index to tick position.
    #[inline]
    #[must_use]
    pub fn row_to_tick(&self, row: RowIndex) -> PatternTick {
        PatternTick(row.as_u32() * self.ticks_per_row.as_u32())
    }

    /// Convert tick position to row index (truncates).
    #[inline]
    #[must_use]
    pub fn tick_to_row(&self, tick: PatternTick) -> RowIndex {
        RowIndex::new((tick.0 / self.ticks_per_row.as_u32()) as u16)
    }
}

// ============================================================================
// Track
// ============================================================================

/// A single track in a pattern.
///
/// Each track maps to one voice. Contains all rows explicitly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    rows: Vec<Row>,
}

impl Track {
    /// Create a new track with empty rows.
    #[must_use]
    pub fn new(num_rows: RowCount) -> Self {
        Self {
            rows: vec![Row::default(); num_rows.as_usize()],
        }
    }

    /// Get a row by index.
    #[inline]
    #[must_use]
    pub fn row(&self, index: RowIndex) -> &Row {
        &self.rows[index.as_usize()]
    }

    /// Get a mutable row by index.
    #[inline]
    pub fn row_mut(&mut self, index: RowIndex) -> &mut Row {
        &mut self.rows[index.as_usize()]
    }

    /// Get all rows.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Get the number of rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Check if the track is empty (no rows).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

// ============================================================================
// Row
// ============================================================================

/// A single row in a track.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Row {
    /// Cell content (note, note-off, or empty).
    pub cell: Cell,

    /// Effects on this row.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectCommand>,
}

impl Row {
    /// Create a new row with a note.
    #[must_use]
    pub fn with_note(pitch: Pitch, instrument: SeqInstrumentId) -> Self {
        Self {
            cell: Cell::note(pitch, instrument),
            effects: Vec::new(),
        }
    }

    /// Check if this row is completely empty (no cell content, no effects).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cell.is_empty() && self.effects.is_empty()
    }

    /// Check if this row has a note.
    #[must_use]
    pub fn has_note(&self) -> bool {
        self.cell.is_note()
    }

    /// Check if this row has a note-off.
    #[must_use]
    pub fn has_note_off(&self) -> bool {
        self.cell.is_note_off()
    }

    /// Check if this row has effects.
    #[must_use]
    pub fn has_effects(&self) -> bool {
        !self.effects.is_empty()
    }
}

// ============================================================================
// Cell
// ============================================================================

/// Cell content in a tracker row.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub enum Cell {
    /// Empty cell - nothing happens, previous note continues playing.
    #[default]
    Empty,

    /// A note that triggers the voice.
    Note {
        /// Pitch to play.
        pitch: Pitch,

        /// Instrument for this note.
        ///
        /// - `Some(id)`: Load and play this instrument
        /// - `None`: Continue with previous instrument on this voice (inherit)
        ///
        /// In tracker formats (XM/MOD), instrument 0 often means "keep previous".
        /// During import, this is translated to `None`.
        instrument: Option<SeqInstrumentId>,

        /// Velocity for the note.
        ///
        /// - `Some(vel)`: Use this velocity
        /// - `None`: Use default (full volume) or let effects (Cxx) control
        velocity: Option<Velocity>,
    },

    /// Explicit note-off (`===` in tracker).
    ///
    /// **CRITICAL:** This cell should ONLY be created during import if the
    /// original file contains an explicit key-off marker.
    ///
    /// A new note on the same track automatically cuts the previous note -
    /// this should NOT generate a NoteOff cell. The voice system handles
    /// implicit cuts during playback.
    ///
    /// Playback behavior:
    /// - Voice enters release phase (envelope release)
    /// - Applies regardless of which instrument was playing
    NoteOff,
}

impl Cell {
    /// Create a note cell with instrument.
    #[must_use]
    pub fn note(pitch: Pitch, instrument: SeqInstrumentId) -> Self {
        Self::Note {
            pitch,
            instrument: Some(instrument),
            velocity: None,
        }
    }

    /// Create a note cell that inherits the previous instrument.
    ///
    /// Use this when the original tracker file has instrument = 0,
    /// meaning "continue with whatever instrument was playing".
    #[must_use]
    pub fn note_inherit_instrument(pitch: Pitch) -> Self {
        Self::Note {
            pitch,
            instrument: None,
            velocity: None,
        }
    }

    /// Create a note cell with full control over all fields.
    #[must_use]
    pub fn note_full(
        pitch: Pitch,
        instrument: Option<SeqInstrumentId>,
        velocity: Option<Velocity>,
    ) -> Self {
        Self::Note {
            pitch,
            instrument,
            velocity,
        }
    }

    /// Create an explicit note-off.
    ///
    /// **WARNING:** Only use this for explicit key-offs from the original file.
    /// Do NOT use for implicit note cuts when a new note starts.
    #[must_use]
    pub fn note_off() -> Self {
        Self::NoteOff
    }

    /// Check if this cell is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Check if this cell contains a note.
    #[inline]
    #[must_use]
    pub fn is_note(&self) -> bool {
        matches!(self, Self::Note { .. })
    }

    /// Check if this cell is a note-off.
    #[inline]
    #[must_use]
    pub fn is_note_off(&self) -> bool {
        matches!(self, Self::NoteOff)
    }

    /// Get pitch if this is a note.
    #[must_use]
    pub fn pitch(&self) -> Option<Pitch> {
        match self {
            Self::Note { pitch, .. } => Some(*pitch),
            _ => None,
        }
    }

    /// Get instrument if this is a note with explicit instrument.
    #[must_use]
    pub fn instrument(&self) -> Option<SeqInstrumentId> {
        match self {
            Self::Note { instrument, .. } => *instrument,
            _ => None,
        }
    }

    /// Get velocity if this is a note with explicit velocity.
    #[must_use]
    pub fn velocity(&self) -> Option<Velocity> {
        match self {
            Self::Note { velocity, .. } => *velocity,
            _ => None,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_creation() {
        let pattern = TrackerPattern::new(PatternId::new(0), TrackCount::new(4), RowCount::new(64));

        assert_eq!(pattern.num_tracks().as_u8(), 4);
        assert_eq!(pattern.num_rows().as_u16(), 64);
        assert_eq!(pattern.ticks_per_row().as_u16(), 240); // Default: 240 song ticks
        assert_eq!(pattern.length_ticks().0, 64 * 240); // 64 rows * 240 ticks
    }

    #[test]
    fn test_cell_access() {
        let mut pattern =
            TrackerPattern::new(PatternId::new(0), TrackCount::new(4), RowCount::new(64));

        let pitch = Pitch::new(60).unwrap();
        let instrument = SeqInstrumentId::new(1);

        pattern.set_cell(
            TrackIndex::new(0),
            RowIndex::new(0),
            Cell::note(pitch, instrument),
        );

        let row = pattern.get(TrackIndex::new(0), RowIndex::new(0));
        assert!(row.has_note());
        assert_eq!(row.cell.pitch(), Some(pitch));
        assert_eq!(row.cell.instrument(), Some(instrument));
    }

    #[test]
    fn test_note_off_is_explicit_only() {
        // This test documents the expected behavior:
        // NoteOff cells should only exist for explicit key-offs

        let mut pattern =
            TrackerPattern::new(PatternId::new(0), TrackCount::new(1), RowCount::new(4));

        let pitch1 = Pitch::new(60).unwrap();
        let pitch2 = Pitch::new(64).unwrap();
        let instrument = SeqInstrumentId::new(1);

        // Row 0: Note C-4
        pattern.set_cell(
            TrackIndex::ZERO,
            RowIndex::new(0),
            Cell::note(pitch1, instrument),
        );

        // Row 1: Empty (note continues)
        // (default, no action needed)

        // Row 2: Explicit note-off from original file
        pattern.set_cell(TrackIndex::ZERO, RowIndex::new(2), Cell::note_off());

        // Row 3: New note (NO note-off before this!)
        pattern.set_cell(
            TrackIndex::ZERO,
            RowIndex::new(3),
            Cell::note(pitch2, instrument),
        );

        // Verify structure
        assert!(pattern.get(TrackIndex::ZERO, RowIndex::new(0)).has_note());
        assert!(
            pattern
                .get(TrackIndex::ZERO, RowIndex::new(1))
                .cell
                .is_empty()
        );
        assert!(
            pattern
                .get(TrackIndex::ZERO, RowIndex::new(2))
                .has_note_off()
        );
        assert!(pattern.get(TrackIndex::ZERO, RowIndex::new(3)).has_note());
    }

    #[test]
    fn test_instrument_inheritance() {
        let pitch = Pitch::new(60).unwrap();

        // Explicit instrument
        let cell1 = Cell::note(pitch, SeqInstrumentId::new(5));
        assert_eq!(cell1.instrument(), Some(SeqInstrumentId::new(5)));

        // Inherit instrument
        let cell2 = Cell::note_inherit_instrument(pitch);
        assert_eq!(cell2.instrument(), None);
    }

    #[test]
    fn test_non_empty_rows_iteration() {
        let mut pattern =
            TrackerPattern::new(PatternId::new(0), TrackCount::new(2), RowCount::new(4));

        let pitch = Pitch::new(60).unwrap();
        let instrument = SeqInstrumentId::new(1);

        // Add some notes
        pattern.set_cell(
            TrackIndex::new(0),
            RowIndex::new(0),
            Cell::note(pitch, instrument),
        );
        pattern.set_cell(
            TrackIndex::new(1),
            RowIndex::new(2),
            Cell::note(pitch, instrument),
        );

        let non_empty: Vec<_> = pattern.non_empty_rows().collect();
        assert_eq!(non_empty.len(), 2);
        assert_eq!(non_empty[0].0.as_u8(), 0); // First track
        assert_eq!(non_empty[0].1.as_u16(), 0); // First row
        assert_eq!(non_empty[1].0.as_u8(), 1); // Second track
        assert_eq!(non_empty[1].1.as_u16(), 2); // Third row
    }

    #[test]
    fn test_row_to_tick_conversion() {
        let pattern = TrackerPattern::new(PatternId::new(0), TrackCount::new(1), RowCount::new(64));

        // Default is 240 song ticks per row (tracker speed 6)
        assert_eq!(pattern.row_to_tick(RowIndex::new(0)).0, 0);
        assert_eq!(pattern.row_to_tick(RowIndex::new(1)).0, 240);
        assert_eq!(pattern.row_to_tick(RowIndex::new(10)).0, 2400);

        // Test reverse conversion
        assert_eq!(pattern.tick_to_row(PatternTick(0)).as_u16(), 0);
        assert_eq!(pattern.tick_to_row(PatternTick(240)).as_u16(), 1);
        assert_eq!(pattern.tick_to_row(PatternTick(400)).as_u16(), 1); // Truncates
    }

    #[test]
    fn test_effects_on_row() {
        let mut pattern =
            TrackerPattern::new(PatternId::new(0), TrackCount::new(1), RowCount::new(4));

        pattern.add_effect(
            TrackIndex::ZERO,
            RowIndex::new(0),
            EffectCommand::SetVolume(64),
        );
        pattern.add_effect(
            TrackIndex::ZERO,
            RowIndex::new(0),
            EffectCommand::Vibrato { speed: 4, depth: 8 },
        );

        let row = pattern.get(TrackIndex::ZERO, RowIndex::new(0));
        assert!(row.has_effects());
        assert_eq!(row.effects.len(), 2);

        // Row with only effects is not empty
        assert!(!row.is_empty());
    }

    #[test]
    fn test_cell_equality() {
        let pitch = Pitch::new(60).unwrap();
        let instrument = SeqInstrumentId::new(1);

        let cell1 = Cell::note(pitch, instrument);
        let cell2 = Cell::note(pitch, instrument);
        let cell3 = Cell::note_off();

        assert_eq!(cell1, cell2);
        assert_ne!(cell1, cell3);
        assert_eq!(Cell::Empty, Cell::Empty);
    }
}
