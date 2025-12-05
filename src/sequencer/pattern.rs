//! Pattern storage and manipulation.

use serde::{Deserialize, Serialize};

use super::automation::AutomationLane;
use super::ids::{InstrumentId, NoteId, PatternId};
use super::note::Note;
use super::pitch::{Pitch, Velocity};
use super::time::{Duration, PatternTick};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// Unique identifier.
    pub id: PatternId,
    /// Pattern name.
    pub name: String,
    /// Length in ticks.
    pub length: Duration,
    /// All notes, sorted by start tick.
    notes: Vec<Note>,
    /// Automation lanes.
    pub automation: Vec<AutomationLane>,
    /// Row resolution for tracker view.
    pub row_resolution: RowResolution,
    /// Next note ID counter.
    next_note_id: u64,
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
            next_note_id: 0,
        }
    }

    /// Set the name (builder pattern).
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the row resolution and update length accordingly.
    pub fn with_row_resolution(mut self, resolution: RowResolution) -> Self {
        self.length = resolution.total_ticks();
        self.row_resolution = resolution;
        self
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
        instrument: InstrumentId,
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
    pub fn transpose_note(&mut self, id: NoteId, semitones: i8) -> bool {
        if let Some(note) = self.note_mut(id) {
            note.pitch = note.pitch.transpose(semitones);
            true
        } else {
            false
        }
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

    /// Transpose all notes.
    pub fn transpose_all(&mut self, semitones: i8) {
        for note in &mut self.notes {
            note.pitch = note.pitch.transpose(semitones);
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
        match pos {
            Some(idx) => &mut self.automation[idx],
            None => {
                self.automation.push(AutomationLane::new(target.clone()));
                self.automation.last_mut().unwrap()
            }
        }
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
        transpose: i8,
        instrument_override: Option<InstrumentId>,
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

            // Transpose pitch
            let transposed_midi =
                (note.pitch.as_midi() as i16 + transpose as i16).clamp(0, 127) as u8;
            let transposed_pitch = Pitch::new(transposed_midi).unwrap_or(note.pitch);

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
            InstrumentId(0),
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
            InstrumentId(0),
        );
        pattern.add_note(
            PatternTick(0),
            Pitch::new(62).unwrap(),
            Velocity::MF,
            InstrumentId(0),
        );
        pattern.add_note(
            PatternTick(240),
            Pitch::new(64).unwrap(),
            Velocity::MF,
            InstrumentId(0),
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
            InstrumentId(0),
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
            InstrumentId(0),
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
            InstrumentId(0),
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
            InstrumentId(0),
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
            InstrumentId(0),
        )
        .with_duration(Duration(40)); // Ends at 140, before range
        let note2 = Note::new(
            NoteId(1),
            PatternTick(200),
            Pitch::new(62).unwrap(),
            Velocity::MF,
            InstrumentId(0),
        )
        .with_duration(Duration(40)); // Starts at 200, within range
        let note3 = Note::new(
            NoteId(2),
            PatternTick(300),
            Pitch::new(64).unwrap(),
            Velocity::MF,
            InstrumentId(0),
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
            InstrumentId(0),
        );
        pattern2.add_note(
            PatternTick(200),
            Pitch::new(62).unwrap(),
            Velocity::MF,
            InstrumentId(0),
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
}
