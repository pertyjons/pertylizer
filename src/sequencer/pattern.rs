//! Pattern storage and manipulation.
//!
//! Patterns support two storage modes:
//! - **Note-based**: Piano roll style with Note objects (start time, duration, pitch)
//! - **Cell-based**: Tracker style with TrackCell grid (row × track)
//!
//! The cell-based representation is optimized for tracker-style editing.

use serde::{Deserialize, Serialize};

use super::automation::AutomationLane;
use super::effects::EffectCommand;
use super::ids::{NoteId, PatternId, SeqInstrumentId};
use super::note::Note;
use super::pitch::{Pitch, Velocity};
use super::time::{Duration, PatternTick};
use crate::types::Semitones;

// ============================================================================
// TrackCell - Cell-based tracker representation
// ============================================================================

/// A cell in a tracker-style pattern grid.
///
/// Each cell represents what happens at a specific row/track intersection.
/// This enum-based design allows for future expansion (e.g., pure automation rows).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum TrackCell {
    /// Empty cell - no event at this position.
    #[default]
    Empty,
    /// Note on event with pitch, instrument, and velocity.
    Note {
        /// The pitch to play.
        pitch: Pitch,
        /// Instrument number (0-255).
        instrument: u8,
        /// Velocity (normalized 0.0-1.0).
        velocity: Velocity,
    },
    /// Note off marker - stops the currently playing note.
    NoteOff,
    /// Effect-only cell (no note trigger).
    Effect {
        /// Effect command type (ProTracker style).
        command: u8,
        /// Effect parameter value.
        value: u8,
    },
}

impl TrackCell {
    /// Create a note cell.
    #[must_use]
    pub const fn note(pitch: Pitch, instrument: u8, velocity: Velocity) -> Self {
        Self::Note {
            pitch,
            instrument,
            velocity,
        }
    }

    /// Create a note-off cell.
    #[must_use]
    pub const fn note_off() -> Self {
        Self::NoteOff
    }

    /// Create an effect cell.
    #[must_use]
    pub const fn effect(command: u8, value: u8) -> Self {
        Self::Effect { command, value }
    }

    /// Check if this cell is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Check if this cell triggers a note.
    #[must_use]
    pub const fn is_note(&self) -> bool {
        matches!(self, Self::Note { .. })
    }

    /// Check if this cell is a note-off.
    #[must_use]
    pub const fn is_note_off(&self) -> bool {
        matches!(self, Self::NoteOff)
    }

    /// Get the pitch if this is a note cell.
    #[must_use]
    pub const fn pitch(&self) -> Option<Pitch> {
        match self {
            Self::Note { pitch, .. } => Some(*pitch),
            _ => None,
        }
    }

    /// Get the velocity if this is a note cell.
    #[must_use]
    pub const fn velocity(&self) -> Option<Velocity> {
        match self {
            Self::Note { velocity, .. } => Some(*velocity),
            _ => None,
        }
    }

    /// Get the instrument if this is a note cell.
    #[must_use]
    pub const fn instrument(&self) -> Option<u8> {
        match self {
            Self::Note { instrument, .. } => Some(*instrument),
            _ => None,
        }
    }
}

// ============================================================================
// TrackerGrid - Grid-based pattern storage
// ============================================================================

/// Grid-based storage for tracker-style patterns.
///
/// This is optimized for tracker-style editing where data is organized
/// by rows (time) and tracks (channels).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerGrid {
    /// Number of rows in this pattern.
    pub rows: u16,
    /// Number of tracks (channels).
    pub tracks: u8,
    /// Cell data: rows[row_idx][track_idx]
    /// Outer Vec = rows, Inner Vec = tracks per row.
    cells: Vec<Vec<TrackCell>>,
    /// Effect columns per track (typically 1-2).
    effects_per_track: Vec<Vec<EffectCommand>>,
}

impl TrackerGrid {
    /// Create a new empty grid.
    #[must_use]
    pub fn new(rows: u16, tracks: u8) -> Self {
        let cells = vec![vec![TrackCell::Empty; tracks as usize]; rows as usize];
        let effects_per_track = vec![Vec::new(); rows as usize * tracks as usize];
        Self {
            rows,
            tracks,
            cells,
            effects_per_track,
        }
    }

    /// Get a cell at the given position.
    #[must_use]
    pub fn get(&self, row: u16, track: u8) -> TrackCell {
        self.cells
            .get(row as usize)
            .and_then(|r| r.get(track as usize))
            .copied()
            .unwrap_or(TrackCell::Empty)
    }

    /// Set a cell at the given position.
    pub fn set(&mut self, row: u16, track: u8, cell: TrackCell) {
        if let Some(c) = self
            .cells
            .get_mut(row as usize)
            .and_then(|r| r.get_mut(track as usize))
        {
            *c = cell;
        }
    }

    /// Clear a cell (set to Empty).
    pub fn clear(&mut self, row: u16, track: u8) {
        self.set(row, track, TrackCell::Empty);
    }

    /// Get effects for a cell.
    #[must_use]
    pub fn effects(&self, row: u16, track: u8) -> &[EffectCommand] {
        let idx = row as usize * self.tracks as usize + track as usize;
        self.effects_per_track
            .get(idx)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Set effects for a cell.
    pub fn set_effects(&mut self, row: u16, track: u8, effects: Vec<EffectCommand>) {
        let idx = row as usize * self.tracks as usize + track as usize;
        if let Some(e) = self.effects_per_track.get_mut(idx) {
            *e = effects;
        }
    }

    /// Get a row as a slice.
    #[must_use]
    pub fn row(&self, row: u16) -> &[TrackCell] {
        self.cells
            .get(row as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Resize the grid (expanding adds empty cells, shrinking truncates).
    pub fn resize(&mut self, new_rows: u16, new_tracks: u8) {
        // Resize rows
        self.cells.resize(
            new_rows as usize,
            vec![TrackCell::Empty; new_tracks as usize],
        );

        // Resize each row's tracks
        for row in &mut self.cells {
            row.resize(new_tracks as usize, TrackCell::Empty);
        }

        // Resize effects storage
        self.effects_per_track
            .resize(new_rows as usize * new_tracks as usize, Vec::new());

        self.rows = new_rows;
        self.tracks = new_tracks;
    }

    /// Check if the grid is empty (all cells are Empty).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells
            .iter()
            .all(|row| row.iter().all(|c| c.is_empty()))
    }
}

/// Row resolution configuration for tracker view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowResolution {
    /// Number of rows in this pattern.
    pub rows: u16,
    /// Ticks per row.
    pub ticks_per_row: u16,
}

impl RowResolution {
    /// Standard tracker: 64 rows, 16 rows per bar at 4/4.
    pub fn standard_64() -> Self {
        Self {
            rows: 64,
            ticks_per_row: 240, // 4 bars * 960 ticks / 64 rows
        }
    }

    /// Short pattern: 16 rows.
    pub fn short_16() -> Self {
        Self {
            rows: 16,
            ticks_per_row: 240,
        }
    }

    /// High resolution: 128 rows.
    pub fn high_128() -> Self {
        Self {
            rows: 128,
            ticks_per_row: 120,
        }
    }

    /// Custom resolution.
    pub fn custom(rows: u16, ticks_per_row: u16) -> Self {
        Self {
            rows,
            ticks_per_row,
        }
    }

    /// Calculate pattern length in ticks.
    pub fn total_ticks(&self) -> Duration {
        Duration(self.rows as u32 * self.ticks_per_row as u32)
    }

    /// Convert row to tick.
    pub fn row_to_tick(&self, row: u16) -> PatternTick {
        PatternTick(row as u32 * self.ticks_per_row as u32)
    }

    /// Convert tick to row (rounded down).
    pub fn tick_to_row(&self, tick: PatternTick) -> u16 {
        (tick.0 / self.ticks_per_row as u32) as u16
    }

    /// Quantize tick to nearest row.
    pub fn quantize(&self, tick: PatternTick) -> PatternTick {
        let row = (tick.0 + self.ticks_per_row as u32 / 2) / self.ticks_per_row as u32;
        PatternTick(row * self.ticks_per_row as u32)
    }

    /// Quantize with strength (0.0 = no change, 1.0 = full quantize).
    pub fn quantize_with_strength(&self, tick: PatternTick, strength: f32) -> PatternTick {
        let quantized = self.quantize(tick);
        let diff = quantized.0 as f32 - tick.0 as f32;
        PatternTick((tick.0 as f32 + diff * strength.clamp(0.0, 1.0)) as u32)
    }
}

impl Default for RowResolution {
    fn default() -> Self {
        Self::standard_64()
    }
}

/// A pattern containing notes and automation.
///
/// Patterns support two representations:
/// - Note-based (piano roll style): `notes` Vec for free-form timing
/// - Grid-based (tracker style): `grid` TrackerGrid for row-aligned editing
///
/// The two representations can be synchronized via `sync_grid_from_notes()`
/// and `sync_notes_from_grid()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// Unique identifier.
    pub id: PatternId,
    /// Pattern name.
    pub name: String,
    /// Length in ticks.
    pub length: Duration,
    /// All notes, sorted by start tick (piano roll representation).
    notes: Vec<Note>,
    /// Automation lanes.
    pub automation: Vec<AutomationLane>,
    /// Row resolution for tracker view.
    pub row_resolution: RowResolution,
    /// Grid-based representation for tracker editing.
    /// This is lazily initialized when first accessed via `grid()` or `grid_mut()`.
    #[serde(default)]
    grid: Option<TrackerGrid>,
    /// Number of tracks in the grid.
    #[serde(default = "default_num_tracks")]
    num_tracks: u8,
    /// Next note ID counter.
    next_note_id: u64,
}

/// Default number of tracks for a pattern.
const fn default_num_tracks() -> u8 {
    4
}

impl Pattern {
    /// Create a new empty pattern.
    pub fn new(id: PatternId, length: Duration) -> Self {
        Self {
            id,
            name: String::new(),
            length,
            notes: Vec::new(),
            automation: Vec::new(),
            row_resolution: RowResolution::standard_64(),
            grid: None,
            num_tracks: default_num_tracks(),
            next_note_id: 0,
        }
    }

    /// Set the name (builder pattern).
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the row resolution and update length accordingly.
    #[must_use]
    pub fn with_row_resolution(mut self, resolution: RowResolution) -> Self {
        self.length = resolution.total_ticks();
        self.row_resolution = resolution;
        // Invalidate grid when resolution changes
        self.grid = None;
        self
    }

    /// Set the number of tracks.
    #[must_use]
    pub fn with_num_tracks(mut self, tracks: u8) -> Self {
        self.num_tracks = tracks;
        // Invalidate grid when track count changes
        self.grid = None;
        self
    }

    /// Get the number of tracks.
    #[must_use]
    pub const fn num_tracks(&self) -> u8 {
        self.num_tracks
    }

    /// Set the number of tracks.
    ///
    /// This invalidates the grid cache.
    pub fn set_num_tracks(&mut self, tracks: u8) {
        self.num_tracks = tracks;
        self.grid = None;
    }

    // === Grid access ===

    /// Get the tracker grid, creating it if necessary.
    ///
    /// The grid is lazily initialized and synchronized from notes.
    pub fn grid(&mut self) -> &TrackerGrid {
        if self.grid.is_none() {
            self.sync_grid_from_notes();
        }
        self.grid.as_ref().unwrap_or_else(|| {
            // This should never happen after sync, but provide a fallback
            static EMPTY: TrackerGrid = TrackerGrid {
                rows: 0,
                tracks: 0,
                cells: Vec::new(),
                effects_per_track: Vec::new(),
            };
            &EMPTY
        })
    }

    /// Get mutable access to the tracker grid.
    pub fn grid_mut(&mut self) -> &mut TrackerGrid {
        if self.grid.is_none() {
            self.sync_grid_from_notes();
        }
        self.grid
            .get_or_insert_with(|| TrackerGrid::new(self.row_resolution.rows, self.num_tracks))
    }

    /// Synchronize the grid from the notes Vec.
    ///
    /// This converts the piano roll representation to tracker grid format.
    pub fn sync_grid_from_notes(&mut self) {
        let rows = self.row_resolution.rows;
        let tracks = self.num_tracks;
        let mut grid = TrackerGrid::new(rows, tracks);

        // Track which track to use for each row (simple round-robin for polyphony)
        let mut track_usage: Vec<u8> = vec![0; rows as usize];

        for note in &self.notes {
            let row = self.row_resolution.tick_to_row(note.start);
            if row >= rows {
                continue;
            }

            // Find an available track for this row
            let track = track_usage[row as usize] % tracks;
            track_usage[row as usize] += 1;

            // Set the cell
            grid.set(
                row,
                track,
                TrackCell::Note {
                    pitch: note.pitch,
                    instrument: note.instrument.0 as u8,
                    velocity: note.velocity,
                },
            );

            // Set effects if any
            if !note.effects.is_empty() {
                grid.set_effects(row, track, note.effects.clone());
            }

            // Add note-off if duration is specified
            if let Some(end) = note.end() {
                let end_row = self.row_resolution.tick_to_row(end);
                if end_row < rows && end_row > row {
                    // Only set note-off if that cell is empty
                    if grid.get(end_row, track).is_empty() {
                        grid.set(end_row, track, TrackCell::NoteOff);
                    }
                }
            }
        }

        self.grid = Some(grid);
    }

    /// Synchronize notes from the grid.
    ///
    /// This converts the tracker grid to piano roll representation.
    /// Clears existing notes and rebuilds from grid data.
    pub fn sync_notes_from_grid(&mut self) {
        let Some(grid) = self.grid.take() else {
            return;
        };

        self.notes.clear();
        self.next_note_id = 0;

        let resolution = self.row_resolution;

        for row in 0..grid.rows {
            for track in 0..grid.tracks {
                let cell = grid.get(row, track);
                if let TrackCell::Note {
                    pitch,
                    instrument,
                    velocity,
                } = cell
                {
                    let start = resolution.row_to_tick(row);
                    let id = NoteId(self.next_note_id);
                    self.next_note_id += 1;

                    let mut note = Note::new(
                        id,
                        start,
                        pitch,
                        velocity,
                        SeqInstrumentId(instrument as u16),
                    );

                    // Look for note-off in subsequent rows
                    for end_row in (row + 1)..grid.rows {
                        let end_cell = grid.get(end_row, track);
                        if end_cell.is_note_off() || end_cell.is_note() {
                            let end_tick = resolution.row_to_tick(end_row);
                            note.duration = Some(Duration(end_tick.0 - start.0));
                            break;
                        }
                    }

                    // Copy effects
                    let effects = grid.effects(row, track);
                    if !effects.is_empty() {
                        note.effects = effects.to_vec();
                    }

                    self.notes.push(note);
                }
            }
        }

        // Sort notes by start tick
        self.notes.sort_by_key(|n| n.start);

        // Restore the grid
        self.grid = Some(grid);
    }

    /// Set a cell in the grid directly.
    ///
    /// This is the primary method for tracker-style editing.
    /// Note: Call `sync_notes_from_grid()` after batch edits to update the notes Vec.
    pub fn set_cell(&mut self, row: u16, track: u8, cell: TrackCell) {
        let grid = self.grid_mut();
        grid.set(row, track, cell);
    }

    /// Get a cell from the grid.
    pub fn get_cell(&mut self, row: u16, track: u8) -> TrackCell {
        self.grid().get(row, track)
    }

    // === Note ID generation ===

    /// Generate the next unique note ID.
    fn next_id(&mut self) -> NoteId {
        let id = NoteId(self.next_note_id);
        self.next_note_id += 1;
        id
    }

    // === Note manipulation ===

    /// Add a note (returns assigned ID).
    pub fn add_note(
        &mut self,
        start: PatternTick,
        pitch: Pitch,
        velocity: Velocity,
        instrument: SeqInstrumentId,
    ) -> NoteId {
        let id = self.next_id();
        let note = Note::new(id, start, pitch, velocity, instrument);

        // Insert sorted by start tick
        let pos = self.notes.partition_point(|n| n.start <= start);
        self.notes.insert(pos, note);
        id
    }

    /// Add a complete note (ID will be reassigned).
    pub fn insert_note(&mut self, mut note: Note) -> NoteId {
        note.id = self.next_id();
        let id = note.id;
        let pos = self.notes.partition_point(|n| n.start <= note.start);
        self.notes.insert(pos, note);
        id
    }

    /// Remove a note by ID.
    pub fn remove_note(&mut self, id: NoteId) -> Option<Note> {
        let pos = self.notes.iter().position(|n| n.id == id)?;
        Some(self.notes.remove(pos))
    }

    /// Get a note by ID.
    pub fn note(&self, id: NoteId) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// Get a mutable note by ID.
    pub fn note_mut(&mut self, id: NoteId) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    /// Get all notes (sorted).
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// Get notes within a tick range.
    pub fn notes_in_range(
        &self,
        start: PatternTick,
        end: PatternTick,
    ) -> impl Iterator<Item = &Note> {
        self.notes
            .iter()
            .filter(move |n| n.start < end && n.end().is_none_or(|e| e > start))
    }

    /// Get notes starting at a specific row.
    pub fn notes_at_row(&self, row: u16) -> impl Iterator<Item = &Note> {
        let tick = self.row_resolution.row_to_tick(row);
        let next_tick = self.row_resolution.row_to_tick(row + 1);
        self.notes
            .iter()
            .filter(move |n| n.start >= tick && n.start < next_tick)
    }

    /// Get notes at a specific pitch.
    pub fn notes_at_pitch(&self, pitch: Pitch) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(move |n| n.pitch == pitch)
    }

    // === Bulk operations ===

    /// Move a note to a new position.
    pub fn move_note(&mut self, id: NoteId, new_start: PatternTick) -> bool {
        if let Some(mut note) = self.remove_note(id) {
            let old_id = note.id;
            note.start = new_start;
            note.id = old_id; // Preserve ID
            self.notes
                .insert(self.notes.partition_point(|n| n.start <= new_start), note);
            // Decrement counter since we reused the ID
            self.next_note_id -= 1;
            true
        } else {
            false
        }
    }

    /// Resize a note.
    pub fn resize_note(&mut self, id: NoteId, new_duration: Duration) -> bool {
        if let Some(note) = self.note_mut(id) {
            note.duration = Some(new_duration);
            true
        } else {
            false
        }
    }

    /// Transpose a note.
    ///
    /// Returns false if the note doesn't exist or transposition would go out of range.
    pub fn transpose_note(&mut self, id: NoteId, semitones: Semitones) -> bool {
        if let Some(note) = self.note_mut(id)
            && let Some(new_pitch) = note.pitch.transpose(semitones)
        {
            note.pitch = new_pitch;
            return true;
        }
        false
    }

    /// Quantize all notes to the row grid.
    pub fn quantize_notes(&mut self) {
        for note in &mut self.notes {
            note.start = self.row_resolution.quantize(note.start);
        }
        // Re-sort after quantization
        self.notes.sort_by_key(|n| n.start);
    }

    /// Quantize with strength (0.0 = no change, 1.0 = full).
    pub fn quantize_notes_with_strength(&mut self, strength: f32) {
        let strength = strength.clamp(0.0, 1.0);
        for note in &mut self.notes {
            note.start = self
                .row_resolution
                .quantize_with_strength(note.start, strength);
        }
        self.notes.sort_by_key(|n| n.start);
    }

    /// Transpose all notes that can be transposed within valid range.
    ///
    /// Notes that would go out of range are left unchanged.
    pub fn transpose_all(&mut self, semitones: Semitones) {
        for note in &mut self.notes {
            if let Some(new_pitch) = note.pitch.transpose(semitones) {
                note.pitch = new_pitch;
            }
        }
    }

    /// Clear all notes.
    pub fn clear_notes(&mut self) {
        self.notes.clear();
    }

    /// Get number of notes.
    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    /// Check if pattern is empty.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.automation.iter().all(|l| l.is_empty())
    }

    // === Automation ===

    /// Add or get an automation lane for a target.
    pub fn get_or_create_automation(
        &mut self,
        target: super::automation::AutomationTarget,
    ) -> &mut AutomationLane {
        let pos = self.automation.iter().position(|l| l.target == target);
        let idx = match pos {
            Some(idx) => idx,
            None => {
                self.automation.push(AutomationLane::new(target.clone()));
                self.automation.len() - 1
            }
        };
        &mut self.automation[idx]
    }

    /// Get automation lane for a target.
    pub fn automation_lane(
        &self,
        target: &super::automation::AutomationTarget,
    ) -> Option<&AutomationLane> {
        self.automation.iter().find(|l| &l.target == target)
    }

    // === Event generation ===

    /// Generate runtime events for playback within a tick range.
    ///
    /// This converts the stored notes into NoteOn/NoteOff events that can be
    /// sent to the audio engine.
    pub fn generate_events(
        &self,
        pattern_start: super::time::Tick,
        range_start: super::time::Tick,
        range_end: super::time::Tick,
        transpose: Semitones,
        instrument_override: Option<SeqInstrumentId>,
    ) -> Vec<super::events::SequencerEvent> {
        use super::events::SequencerEvent;
        use super::time::Tick;

        let mut events = Vec::new();

        // Convert song-ticks to pattern-local ticks
        let local_start = if range_start.0 > pattern_start.0 {
            PatternTick((range_start.0 - pattern_start.0) as u32)
        } else {
            PatternTick(0)
        };
        let local_end =
            PatternTick(((range_end.0.saturating_sub(pattern_start.0)) as u32).min(self.length.0));

        for note in &self.notes {
            let instrument = instrument_override.unwrap_or(note.instrument);

            // Transpose pitch (keep original if out of range)
            let transposed_pitch = note.pitch.transpose(transpose).unwrap_or(note.pitch);

            // NoteOn
            if note.start >= local_start && note.start < local_end {
                let absolute_tick = Tick(pattern_start.0 + note.start.0 as u64);
                events.push(SequencerEvent::NoteOn {
                    tick: absolute_tick,
                    pitch: transposed_pitch,
                    velocity: note.velocity,
                    instrument,
                    effects: note.effects.clone(),
                });
            }

            // NoteOff
            if let Some(end) = note.end()
                && end > local_start
                && end <= local_end
            {
                let absolute_tick = Tick(pattern_start.0 + end.0 as u64);
                events.push(SequencerEvent::NoteOff {
                    tick: absolute_tick,
                    pitch: transposed_pitch,
                    instrument,
                });
            }
        }

        // Sort events by tick
        events.sort_by_key(|e| e.tick());
        events
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::sequencer::ids::NoteId;
    use crate::sequencer::note::Note;

    fn test_pattern() -> Pattern {
        Pattern::new(PatternId(0), Duration(3840))
    }

    #[test]
    fn test_pattern_creation() {
        let pattern = test_pattern();
        assert_eq!(pattern.length.0, 3840);
        assert!(pattern.notes.is_empty());
    }

    #[test]
    fn test_add_and_retrieve_note() {
        let mut pattern = test_pattern();
        let id = pattern.add_note(
            PatternTick(0),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );

        assert!(pattern.note(id).is_some());
        assert_eq!(pattern.note(id).unwrap().pitch.as_midi(), 60);
    }

    #[test]
    fn test_notes_stay_sorted() {
        let mut pattern = test_pattern();

        pattern.add_note(
            PatternTick(480),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );
        pattern.add_note(
            PatternTick(0),
            Pitch::new(62).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );
        pattern.add_note(
            PatternTick(240),
            Pitch::new(64).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );

        let ticks: Vec<_> = pattern.notes().iter().map(|n| n.start.0).collect();
        assert_eq!(ticks, vec![0, 240, 480]);
    }

    #[test]
    fn test_remove_note() {
        let mut pattern = test_pattern();
        let id = pattern.add_note(
            PatternTick(0),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );

        assert!(pattern.remove_note(id).is_some());
        assert!(pattern.note(id).is_none());
    }

    #[test]
    fn test_move_note() {
        let mut pattern = test_pattern();
        let id = pattern.add_note(
            PatternTick(0),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );

        assert!(pattern.move_note(id, PatternTick(480)));
        assert_eq!(pattern.note(id).unwrap().start.0, 480);
    }

    #[test]
    fn test_quantize_notes() {
        let mut pattern = test_pattern().with_row_resolution(RowResolution::standard_64());

        // Note at tick 120 (between rows 0 and 1 at 240 ticks/row)
        let id = pattern.add_note(
            PatternTick(120),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );

        pattern.quantize_notes();

        // Should quantize to row 0 (tick 0) or row 1 (tick 240)
        // 120 is exactly in the middle, so it goes to nearest (240 with rounding)
        let note = pattern.note(id).unwrap();
        assert_eq!(note.start.0, 240);
    }

    #[test]
    fn test_quantize_with_strength() {
        let mut pattern = test_pattern().with_row_resolution(RowResolution::standard_64());

        let id = pattern.add_note(
            PatternTick(120),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );

        // 50% quantization toward 240
        pattern.quantize_notes_with_strength(0.5);

        let note = pattern.note(id).unwrap();
        // 120 + 0.5 * (240 - 120) = 120 + 60 = 180
        assert_eq!(note.start.0, 180);
    }

    #[test]
    #[allow(clippy::similar_names)] // note1/note2/note3 and pattern/pattern2 are intentional
    fn test_notes_in_range() {
        let mut pattern = test_pattern();

        // Add notes with explicit durations so they have defined end times
        let note1 = Note::new(
            NoteId(0),
            PatternTick(100),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        )
        .with_duration(Duration(40)); // Ends at 140, before range
        let note2 = Note::new(
            NoteId(1),
            PatternTick(200),
            Pitch::new(62).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        )
        .with_duration(Duration(40)); // Starts at 200, within range
        let note3 = Note::new(
            NoteId(2),
            PatternTick(300),
            Pitch::new(64).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        )
        .with_duration(Duration(40)); // Starts at 300, after range

        pattern.insert_note(note1);
        pattern.insert_note(note2);
        pattern.insert_note(note3);

        // Range 150-250 should only include note2 (starts at 200)
        let notes: Vec<_> = pattern
            .notes_in_range(PatternTick(150), PatternTick(250))
            .collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].start.0, 200);

        // Test notes without duration (tracker-style, plays until next note off)
        let mut pattern2 = test_pattern();
        pattern2.add_note(
            PatternTick(100),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );
        pattern2.add_note(
            PatternTick(200),
            Pitch::new(62).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );

        // Notes without duration overlap with any range after their start
        let notes: Vec<_> = pattern2
            .notes_in_range(PatternTick(150), PatternTick(250))
            .collect();
        assert_eq!(notes.len(), 2); // Both notes overlap: first has no end, second starts in range
    }

    #[test]
    fn test_row_resolution() {
        let res = RowResolution::standard_64();
        assert_eq!(res.total_ticks().0, 64 * 240);
        assert_eq!(res.row_to_tick(4).0, 960);
        assert_eq!(res.tick_to_row(PatternTick(960)), 4);
    }

    // =========================================================================
    // TrackCell and TrackerGrid tests
    // =========================================================================

    #[test]
    fn test_track_cell_creation() {
        let empty = TrackCell::Empty;
        assert!(empty.is_empty());
        assert!(!empty.is_note());
        assert!(!empty.is_note_off());

        let note = TrackCell::note(Pitch::new(60).unwrap(), 1, Velocity::MF);
        assert!(!note.is_empty());
        assert!(note.is_note());
        assert_eq!(note.pitch(), Some(Pitch::new(60).unwrap()));
        assert_eq!(note.instrument(), Some(1));

        let note_off = TrackCell::note_off();
        assert!(note_off.is_note_off());
        assert!(!note_off.is_note());
    }

    #[test]
    fn test_tracker_grid_basic() {
        let mut grid = TrackerGrid::new(16, 4);
        assert_eq!(grid.rows, 16);
        assert_eq!(grid.tracks, 4);
        assert!(grid.is_empty());

        // Set a cell
        let note = TrackCell::note(Pitch::new(60).unwrap(), 0, Velocity::MF);
        grid.set(0, 0, note);
        assert!(!grid.is_empty());

        // Get the cell back
        let cell = grid.get(0, 0);
        assert!(cell.is_note());
        assert_eq!(cell.pitch(), Some(Pitch::new(60).unwrap()));

        // Clear the cell
        grid.clear(0, 0);
        assert!(grid.get(0, 0).is_empty());
        assert!(grid.is_empty());
    }

    #[test]
    fn test_tracker_grid_resize() {
        let mut grid = TrackerGrid::new(16, 4);

        // Set a note
        grid.set(
            0,
            0,
            TrackCell::note(Pitch::new(60).unwrap(), 0, Velocity::MF),
        );

        // Expand
        grid.resize(32, 8);
        assert_eq!(grid.rows, 32);
        assert_eq!(grid.tracks, 8);
        // Original note should still be there
        assert!(grid.get(0, 0).is_note());

        // New cells should be empty
        assert!(grid.get(20, 5).is_empty());
    }

    #[test]
    fn test_pattern_grid_sync() {
        let mut pattern = test_pattern().with_row_resolution(RowResolution::standard_64());

        // Add a note
        pattern.add_note(
            PatternTick(0), // Row 0
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        );

        // Get the grid (triggers sync from notes)
        let grid = pattern.grid();

        // Check that the note appears in the grid
        let cell = grid.get(0, 0);
        assert!(cell.is_note());
        assert_eq!(cell.pitch(), Some(Pitch::new(60).unwrap()));
    }

    #[test]
    fn test_pattern_set_cell() {
        let mut pattern = test_pattern().with_row_resolution(RowResolution::standard_64());

        // Set a cell directly
        pattern.set_cell(
            4,
            0,
            TrackCell::note(Pitch::new(64).unwrap(), 1, Velocity::FORTE),
        );

        // Sync back to notes
        pattern.sync_notes_from_grid();

        // Check that the note was created
        assert_eq!(pattern.note_count(), 1);
        let note = pattern.notes().first().unwrap();
        assert_eq!(note.pitch.as_midi(), 64);
        assert_eq!(note.start.0, 4 * 240); // Row 4 * ticks_per_row
    }

    #[test]
    fn test_grid_note_off_handling() {
        let mut pattern = test_pattern().with_row_resolution(RowResolution::standard_64());

        // Add a note with duration (4 rows = 960 ticks)
        let note = Note::new(
            NoteId(0),
            PatternTick(0),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        )
        .with_duration(Duration(960)); // 4 rows

        pattern.insert_note(note);

        // Sync to grid
        pattern.sync_grid_from_notes();

        // Check note-on at row 0
        let grid = pattern.grid();
        assert!(grid.get(0, 0).is_note());

        // Check note-off at row 4
        assert!(grid.get(4, 0).is_note_off());
    }
}
