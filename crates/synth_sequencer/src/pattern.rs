//! Pattern storage and manipulation.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::automation::AutomationLane;
use super::ids::{NoteId, PatternId, RowCount, RowIndex, TicksPerRow};
use super::note::Note;
use super::pitch::{Pitch, Velocity};
use super::time::{Duration, PatternTick};
use synth_core::{NormalizedValue, Semitones};

// ============================================================================
// RowResolution - Grid timing configuration
// ============================================================================

/// Row resolution configuration for pattern view.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RowResolution {
    /// Number of rows in this pattern.
    pub rows: RowCount,
    /// Ticks per row.
    pub ticks_per_row: TicksPerRow,
}

impl RowResolution {
    /// Standard: 64 rows, 16 rows per bar at 4/4.
    pub fn standard_64() -> Self {
        Self {
            rows: RowCount::new(64),
            ticks_per_row: TicksPerRow::new(240), // 4 bars * 960 ticks / 64 rows
        }
    }

    /// Short pattern: 16 rows.
    pub fn short_16() -> Self {
        Self {
            rows: RowCount::new(16),
            ticks_per_row: TicksPerRow::new(240),
        }
    }

    /// High resolution: 128 rows.
    pub fn high_128() -> Self {
        Self {
            rows: RowCount::new(128),
            ticks_per_row: TicksPerRow::new(120),
        }
    }

    /// Custom resolution.
    pub fn custom(rows: RowCount, ticks_per_row: TicksPerRow) -> Self {
        Self {
            rows,
            ticks_per_row,
        }
    }

    /// Calculate pattern length in ticks.
    pub fn total_ticks(&self) -> Duration {
        Duration(self.rows.as_u16() as u32 * self.ticks_per_row.as_u32())
    }

    /// Convert row to tick.
    pub fn row_to_tick(&self, row: RowIndex) -> PatternTick {
        PatternTick(row.as_u32() * self.ticks_per_row.as_u32())
    }

    /// Convert tick to row (rounded down).
    pub fn tick_to_row(&self, tick: PatternTick) -> RowIndex {
        RowIndex::new((tick.0 / self.ticks_per_row.as_u32()) as u16)
    }

    /// Quantize tick to nearest row.
    pub fn quantize(&self, tick: PatternTick) -> PatternTick {
        let tpr = self.ticks_per_row.as_u32();
        let row = (tick.0 + tpr / 2) / tpr;
        PatternTick(row * tpr)
    }

    /// Quantize with strength (0.0 = no change, 1.0 = full quantize).
    pub fn quantize_with_strength(
        &self,
        tick: PatternTick,
        strength: NormalizedValue,
    ) -> PatternTick {
        let quantized = self.quantize(tick);
        let diff = quantized.0 as f32 - tick.0 as f32;
        PatternTick((tick.0 as f32 + diff * strength.as_f32()).round() as u32)
    }
}

impl Default for RowResolution {
    fn default() -> Self {
        Self::standard_64()
    }
}

/// A pattern containing notes and automation.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Pattern {
    /// Unique identifier.
    pub id: PatternId,
    /// Pattern name.
    pub name: String,
    /// Free-text description capturing intent (e.g. "chorus drop, half-time
    /// feel"). Empty by default; readable/writable via MCP and GUI.
    #[serde(default)]
    pub description: String,
    /// Length in ticks.
    pub length: Duration,
    /// All notes, sorted by start tick.
    notes: Vec<Note>,
    /// Automation lanes.
    pub automation: Vec<AutomationLane>,
    /// Next note ID counter.
    next_note_id: u64,
}

impl Pattern {
    /// Create a new empty pattern.
    #[must_use]
    pub fn new(id: PatternId, length: Duration) -> Self {
        Self {
            id,
            name: String::new(),
            description: String::new(),
            length,
            notes: Vec::new(),
            automation: Vec::new(),
            next_note_id: 0,
        }
    }

    /// Set the name (builder pattern).
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
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

    /// Add a note (returns assigned ID). The note's instrument is determined
    /// by the track it plays on at arrangement time, not the note itself.
    pub fn add_note(&mut self, start: PatternTick, pitch: Pitch, velocity: Velocity) -> NoteId {
        let id = self.next_id();
        let note = Note::new(id, start, pitch, velocity);

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
    #[must_use]
    pub fn note(&self, id: NoteId) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// Get a mutable note by ID.
    pub fn note_mut(&mut self, id: NoteId) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    /// Get all notes (sorted).
    #[must_use]
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

    /// Get notes at a specific pitch.
    pub fn notes_at_pitch(&self, pitch: Pitch) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(move |n| n.pitch == pitch)
    }

    // === Bulk operations ===

    /// Move a note to a new position.
    pub fn move_note(&mut self, id: NoteId, new_start: PatternTick) -> bool {
        if let Some(mut note) = self.remove_note(id) {
            note.start = new_start;
            self.notes
                .insert(self.notes.partition_point(|n| n.start <= new_start), note);
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
    pub fn quantize_notes(&mut self, resolution: RowResolution) {
        for note in &mut self.notes {
            note.start = resolution.quantize(note.start);
        }
        // Re-sort after quantization
        self.notes.sort_by_key(|n| n.start);
    }

    /// Quantize with strength (0.0 = no change, 1.0 = full).
    pub fn quantize_notes_with_strength(
        &mut self,
        resolution: RowResolution,
        strength: NormalizedValue,
    ) {
        for note in &mut self.notes {
            note.start = resolution.quantize_with_strength(note.start, strength);
        }
        self.notes.sort_by_key(|n| n.start);
    }

    /// Set the velocity of a note.
    pub fn set_note_velocity(&mut self, id: NoteId, velocity: Velocity) -> bool {
        if let Some(note) = self.note_mut(id) {
            note.velocity = velocity;
            true
        } else {
            false
        }
    }

    /// Set the legato/tie flag of a note.
    pub fn set_note_legato(&mut self, id: NoteId, legato: bool) -> bool {
        if let Some(note) = self.note_mut(id) {
            note.legato = legato;
            true
        } else {
            false
        }
    }

    /// Set (or clear) the per-note glide.
    pub fn set_note_glide(&mut self, id: NoteId, glide: Option<super::note::Glide>) -> bool {
        if let Some(note) = self.note_mut(id) {
            note.glide = glide;
            true
        } else {
            false
        }
    }

    /// Set (or clear) the per-note expression block. An all-default block is
    /// normalized to `None` here (the central enforcement of "never persist an
    /// empty expression block"), so callers can't accidentally store one.
    pub fn set_note_expression(
        &mut self,
        id: NoteId,
        expression: Option<super::note::NoteExpression>,
    ) -> bool {
        if let Some(note) = self.note_mut(id) {
            note.expression = expression.and_then(super::note::NoteExpression::normalized);
            true
        } else {
            false
        }
    }

    /// Quantize only the selected notes to a custom grid with strength.
    pub fn quantize_selected(
        &mut self,
        note_ids: &HashSet<NoteId>,
        grid_ticks: Duration,
        strength: NormalizedValue,
    ) {
        if grid_ticks.0 == 0 {
            return;
        }
        let grid = grid_ticks.0;
        let str_f = strength.as_f32().clamp(0.0, 1.0);
        for note in &mut self.notes {
            if !note_ids.contains(&note.id) {
                continue;
            }
            let quantized = ((note.start.0 + grid / 2) / grid) * grid;
            let diff = quantized as f32 - note.start.0 as f32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let new_tick = (note.start.0 as f32 + diff * str_f).round() as u32;
            note.start = PatternTick(new_tick);
        }
        self.notes.sort_by_key(|n| n.start);
    }

    /// Apply humanization (random timing/velocity offsets) to selected notes.
    pub fn humanize_notes(
        &mut self,
        note_ids: &HashSet<NoteId>,
        timing_range: Duration,
        velocity_range: NormalizedValue,
    ) {
        let timing_ticks = timing_range.0;
        let vel_range = velocity_range.as_f32();
        for note in &mut self.notes {
            if !note_ids.contains(&note.id) {
                continue;
            }
            // Random timing offset: ±timing_range ticks
            let timing_offset = fastrand::i32(-(timing_ticks as i32)..=(timing_ticks as i32));
            #[allow(clippy::cast_sign_loss)]
            let new_tick = (note.start.0 as i64 + timing_offset as i64).max(0) as u32;
            note.start = PatternTick(new_tick.min(self.length.0.saturating_sub(1)));

            // Random velocity offset: ±velocity_range
            let vel_offset = fastrand::f32() * 2.0 * vel_range - vel_range;
            let new_vel = (note.velocity.as_f32() + vel_offset).clamp(0.01, 1.0);
            note.velocity = Velocity::new(new_vel);
        }
        self.notes.sort_by_key(|n| n.start);
    }

    /// Scale velocities of selected notes by a factor.
    pub fn scale_velocities(&mut self, note_ids: &HashSet<NoteId>, factor: f32) {
        for note in &mut self.notes {
            if !note_ids.contains(&note.id) {
                continue;
            }
            let new_vel = (note.velocity.as_f32() * factor).clamp(0.01, 1.0);
            note.velocity = Velocity::new(new_vel);
        }
    }

    /// Apply swing to selected notes by shifting even subdivisions.
    ///
    /// `grid_ticks` defines the base subdivision (e.g. 480 for 1/8 notes).
    /// `swing` ranges from 0.0 (no swing) to 1.0 (full shuffle).
    /// Even-numbered beats within each pair are delayed by `swing * grid_ticks / 2`.
    pub fn apply_swing(
        &mut self,
        note_ids: &HashSet<NoteId>,
        grid_ticks: Duration,
        swing: NormalizedValue,
    ) {
        let grid = grid_ticks.0;
        if grid == 0 {
            return;
        }
        let swing_f = swing.as_f32().clamp(0.0, 1.0);
        let max_shift = grid as f32 / 2.0;
        for note in &mut self.notes {
            if !note_ids.contains(&note.id) {
                continue;
            }
            // Determine which grid position this note is on
            let grid_pos = note.start.0 / grid;
            // Odd grid positions (the "even" subdivisions in musical terms) get shifted
            if grid_pos % 2 == 1 {
                let shift = (max_shift * swing_f) as u32;
                note.start =
                    PatternTick((note.start.0 + shift).min(self.length.0.saturating_sub(1)));
            }
        }
        self.notes.sort_by_key(|n| n.start);
    }

    /// Transpose all notes that can be transposed within valid range.
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
    #[must_use]
    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    /// Check if pattern is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.automation.iter().all(|l| l.is_empty())
    }

    /// Get a mutable note by its index.
    pub fn note_by_index_mut(&mut self, index: usize) -> Option<&mut Note> {
        self.notes.get_mut(index)
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
    #[must_use]
    pub fn automation_lane(
        &self,
        target: &super::automation::AutomationTarget,
    ) -> Option<&AutomationLane> {
        self.automation.iter().find(|l| &l.target == target)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ids::NoteId;
    use crate::note::Note;

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
        let id = pattern.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);

        assert!(pattern.note(id).is_some());
        assert_eq!(pattern.note(id).unwrap().pitch.as_midi(), 60);
    }

    #[test]
    fn test_notes_stay_sorted() {
        let mut pattern = test_pattern();

        let _ = pattern.add_note(PatternTick(480), Pitch::new(60).unwrap(), Velocity::MF);
        let _ = pattern.add_note(PatternTick(0), Pitch::new(62).unwrap(), Velocity::MF);
        let _ = pattern.add_note(PatternTick(240), Pitch::new(64).unwrap(), Velocity::MF);

        let ticks: Vec<_> = pattern.notes().iter().map(|n| n.start.0).collect();
        assert_eq!(ticks, vec![0, 240, 480]);
    }

    #[test]
    fn test_remove_note() {
        let mut pattern = test_pattern();
        let id = pattern.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);

        assert!(pattern.remove_note(id).is_some());
        assert!(pattern.note(id).is_none());
    }

    #[test]
    fn test_move_note() {
        let mut pattern = test_pattern();
        let id = pattern.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);

        assert!(pattern.move_note(id, PatternTick(480)));
        assert_eq!(pattern.note(id).unwrap().start.0, 480);
    }

    #[test]
    fn test_quantize_notes() {
        let resolution = RowResolution::standard_64();
        let mut pattern = test_pattern();

        let id = pattern.add_note(PatternTick(120), Pitch::new(60).unwrap(), Velocity::MF);

        pattern.quantize_notes(resolution);

        let note = pattern.note(id).unwrap();
        assert_eq!(note.start.0, 240);
    }

    #[test]
    fn test_quantize_with_strength() {
        let resolution = RowResolution::standard_64();
        let mut pattern = test_pattern();

        let id = pattern.add_note(PatternTick(120), Pitch::new(60).unwrap(), Velocity::MF);

        // 50% quantization toward 240
        pattern.quantize_notes_with_strength(resolution, NormalizedValue::CENTER);

        let note = pattern.note(id).unwrap();
        // 120 + 0.5 * (240 - 120) = 120 + 60 = 180
        assert_eq!(note.start.0, 180);
    }

    #[test]
    #[allow(clippy::similar_names)]
    fn test_notes_in_range() {
        let mut pattern = test_pattern();

        let note1 = Note::new(
            NoteId(0),
            PatternTick(100),
            Pitch::new(60).unwrap(),
            Velocity::MF,
        )
        .with_duration(Duration(40));
        let note2 = Note::new(
            NoteId(1),
            PatternTick(200),
            Pitch::new(62).unwrap(),
            Velocity::MF,
        )
        .with_duration(Duration(40));
        let note3 = Note::new(
            NoteId(2),
            PatternTick(300),
            Pitch::new(64).unwrap(),
            Velocity::MF,
        )
        .with_duration(Duration(40));

        let _ = pattern.insert_note(note1);
        let _ = pattern.insert_note(note2);
        let _ = pattern.insert_note(note3);

        let notes: Vec<_> = pattern
            .notes_in_range(PatternTick(150), PatternTick(250))
            .collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].start.0, 200);

        // Test notes without duration
        let mut pattern2 = test_pattern();
        let _ = pattern2.add_note(PatternTick(100), Pitch::new(60).unwrap(), Velocity::MF);
        let _ = pattern2.add_note(PatternTick(200), Pitch::new(62).unwrap(), Velocity::MF);

        let notes: Vec<_> = pattern2
            .notes_in_range(PatternTick(150), PatternTick(250))
            .collect();
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn test_row_resolution() {
        let res = RowResolution::standard_64();
        assert_eq!(res.total_ticks().0, 64 * 240);
        assert_eq!(res.row_to_tick(RowIndex::new(4)).0, 960);
        assert_eq!(res.tick_to_row(PatternTick(960)), RowIndex::new(4));
    }
}
