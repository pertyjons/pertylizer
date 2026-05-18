//! Song structure and arrangement.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::ids::{PatternId, SeqInstrumentId, TrackId};
use super::pattern::Pattern;
use super::time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
use super::track::SequencerTrack;
use synth_core::{Bpm, Gain, Semitones};

/// Tempo change event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TempoChange {
    /// Position of the tempo change.
    pub tick: Tick,
    /// New tempo in BPM.
    pub bpm: Bpm,
}

/// Time signature change event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TimeSignatureChange {
    /// Position of the time signature change.
    pub tick: Tick,
    /// New time signature.
    pub signature: TimeSignature,
}

/// A pattern placement in the arrangement.
#[must_use]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternPlacement {
    /// Which pattern to play.
    pub pattern_id: PatternId,
    /// Which track this is on.
    pub track_id: TrackId,
    /// Start position in the song.
    pub start: Tick,
    /// Transposition in semitones.
    pub transpose: Semitones,
    /// Volume scaling (1.0 = normal).
    pub gain: Gain,
    /// Optional per-placement length override. When `None`, the placement
    /// occupies exactly `pattern.length` ticks. When `Some(d)`, the
    /// placement extends/clips to that length; the engine still walks the
    /// pattern's note timeline up to `min(d, pattern.length)`.
    #[serde(default)]
    pub length_override: Option<Duration>,
}

impl PatternPlacement {
    /// Create a new pattern placement.
    pub fn new(pattern_id: PatternId, track_id: TrackId, start: Tick) -> Self {
        Self {
            pattern_id,
            track_id,
            start,
            transpose: Semitones::ZERO,
            gain: Gain::UNITY,
            length_override: None,
        }
    }

    /// Set transposition (builder pattern).
    pub fn with_transpose(mut self, semitones: Semitones) -> Self {
        self.transpose = semitones;
        self
    }

    /// Set gain (builder pattern).
    pub fn with_gain(mut self, gain: Gain) -> Self {
        self.gain = gain;
        self
    }

    /// Calculate end position. Respects `length_override` when set,
    /// otherwise uses the pattern's own length.
    pub fn end(&self, pattern_length: Duration) -> Tick {
        let len = self.length_override.unwrap_or(pattern_length);
        Tick(self.start.0 + len.0 as u64)
    }

    /// Effective length (override if set, otherwise the pattern's length).
    pub fn effective_length(&self, pattern_length: Duration) -> Duration {
        self.length_override.unwrap_or(pattern_length)
    }
}

/// A complete song with patterns, tracks, and arrangement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    /// Song name.
    pub name: String,
    /// Song author.
    pub author: String,

    // Pattern storage
    patterns: BTreeMap<PatternId, Pattern>,
    next_pattern_id: u32,

    // Track storage
    tracks: BTreeMap<TrackId, SequencerTrack>,
    next_track_id: u16,
    /// Display order for tracks (top → bottom). Maintained alongside
    /// `tracks`: created tracks append to the end, deleted ones are
    /// removed, and `reorder_track` mutates the order without touching
    /// the underlying map. `tracks()` iterates in this order.
    #[serde(default)]
    track_order: Vec<TrackId>,

    // Arrangement
    arrangement: Vec<PatternPlacement>,

    // Tempo and time signature
    tempo_changes: Vec<TempoChange>,
    time_signature_changes: Vec<TimeSignatureChange>,
    /// Default tempo (BPM).
    pub default_tempo: Bpm,
    /// Default time signature.
    pub default_time_signature: TimeSignature,
}

impl Song {
    /// Create a new empty song.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: String::new(),
            patterns: BTreeMap::new(),
            next_pattern_id: 0,
            tracks: BTreeMap::new(),
            next_track_id: 0,
            track_order: Vec::new(),
            arrangement: Vec::new(),
            tempo_changes: Vec::new(),
            time_signature_changes: Vec::new(),
            default_tempo: Bpm::new(120.0),
            default_time_signature: TimeSignature::COMMON,
        }
    }

    /// Set the author (builder pattern).
    #[must_use]
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }

    /// Set the default tempo (builder pattern).
    #[must_use]
    pub fn with_tempo(mut self, bpm: Bpm) -> Self {
        self.default_tempo = bpm;
        self
    }

    /// Set the default time signature (builder pattern).
    #[must_use]
    pub fn with_time_signature(mut self, sig: TimeSignature) -> Self {
        self.default_time_signature = sig;
        self
    }

    // === Pattern management ===

    /// Create a new pattern and add it to the song.
    pub fn create_pattern(&mut self, length: Duration) -> PatternId {
        let id = PatternId(self.next_pattern_id);
        self.next_pattern_id = self.next_pattern_id.saturating_add(1);
        self.patterns.insert(id, Pattern::new(id, length));
        id
    }

    /// Get a pattern by ID.
    #[must_use]
    pub fn pattern(&self, id: PatternId) -> Option<&Pattern> {
        self.patterns.get(&id)
    }

    /// Get a mutable pattern by ID.
    pub fn pattern_mut(&mut self, id: PatternId) -> Option<&mut Pattern> {
        self.patterns.get_mut(&id)
    }

    /// Get all patterns.
    pub fn patterns(&self) -> impl Iterator<Item = &Pattern> {
        self.patterns.values()
    }

    /// Get the number of patterns.
    #[must_use]
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    /// Delete a pattern.
    pub fn delete_pattern(&mut self, id: PatternId) -> Option<Pattern> {
        // Also remove from arrangement
        self.arrangement.retain(|p| p.pattern_id != id);
        self.patterns.remove(&id)
    }

    /// Insert a pre-built pattern under its existing id. Used by undo to
    /// restore a deleted pattern with its full notes and automation.
    /// Returns true if inserted, false if a pattern with that id already
    /// exists.
    pub fn insert_pattern(&mut self, pattern: Pattern) -> bool {
        if self.patterns.contains_key(&pattern.id) {
            return false;
        }
        // Keep `next_pattern_id` ahead of any restored id so later
        // create_pattern calls don't reuse the same slot.
        if pattern.id.0 >= self.next_pattern_id {
            self.next_pattern_id = pattern.id.0.saturating_add(1);
        }
        self.patterns.insert(pattern.id, pattern);
        true
    }

    /// Duplicate a pattern.
    pub fn duplicate_pattern(&mut self, id: PatternId) -> Option<PatternId> {
        let pattern = self.patterns.get(&id)?.clone();
        let new_id = PatternId(self.next_pattern_id);
        self.next_pattern_id = self.next_pattern_id.saturating_add(1);

        let mut new_pattern = Pattern::new(new_id, pattern.length);
        new_pattern.name = format!("{} (copy)", pattern.name);
        new_pattern.row_resolution = pattern.row_resolution;

        // Copy notes
        for note in pattern.notes() {
            let _ = new_pattern.insert_note(note.clone());
        }

        // Copy automation lanes
        new_pattern.automation = pattern.automation.clone();

        self.patterns.insert(new_id, new_pattern);
        Some(new_id)
    }

    // === Track management ===

    /// Create a new track.
    pub fn create_track(&mut self, name: impl Into<String>) -> TrackId {
        let id = TrackId(self.next_track_id);
        self.next_track_id = self.next_track_id.saturating_add(1);
        self.tracks.insert(id, SequencerTrack::new(id, name));
        self.track_order.push(id);
        id
    }

    /// Insert a pre-built track under its existing id. Used by undo to
    /// restore a deleted track. The track is appended to the display order
    /// (or its old position if provided via `at_index`). Returns true if
    /// inserted, false if the id already exists.
    pub fn insert_track(&mut self, track: SequencerTrack, at_index: Option<usize>) -> bool {
        if self.tracks.contains_key(&track.id) {
            return false;
        }
        if track.id.0 >= self.next_track_id {
            self.next_track_id = track.id.0.saturating_add(1);
        }
        let id = track.id;
        self.tracks.insert(id, track);
        let pos =
            at_index.map_or_else(|| self.track_order.len(), |i| i.min(self.track_order.len()));
        self.track_order.insert(pos, id);
        true
    }

    /// Insert a pre-built placement as-is. Used by undo to restore a
    /// removed placement with its transpose/gain/etc. preserved. Returns
    /// true if inserted, false if the pattern or track is missing.
    pub fn insert_placement(&mut self, placement: PatternPlacement) -> bool {
        if !self.patterns.contains_key(&placement.pattern_id)
            || !self.tracks.contains_key(&placement.track_id)
        {
            return false;
        }
        let start = placement.start;
        let pos = self.arrangement.partition_point(|p| p.start <= start);
        self.arrangement.insert(pos, placement);
        true
    }

    /// Move a track from one index to another in the display order.
    /// Returns true if the move happened. Repairs `track_order` first if
    /// it has drifted from `tracks` (e.g. an older serialized song).
    pub fn reorder_track(&mut self, from: usize, to: usize) -> bool {
        self.repair_track_order();
        if from >= self.track_order.len() || to >= self.track_order.len() || from == to {
            return false;
        }
        let id = self.track_order.remove(from);
        self.track_order.insert(to, id);
        true
    }

    /// Sync `track_order` with `tracks`: drop unknown ids, append missing
    /// ones in map order. Idempotent; no-op when already in sync.
    pub fn repair_track_order(&mut self) {
        self.track_order.retain(|id| self.tracks.contains_key(id));
        let missing: Vec<TrackId> = self
            .tracks
            .keys()
            .filter(|id| !self.track_order.contains(id))
            .copied()
            .collect();
        self.track_order.extend(missing);
    }

    /// Get the display-ordered list of track IDs.
    pub fn track_order(&self) -> &[TrackId] {
        &self.track_order
    }

    /// Get a track by ID.
    #[must_use]
    pub fn track(&self, id: TrackId) -> Option<&SequencerTrack> {
        self.tracks.get(&id)
    }

    /// Get a mutable track by ID.
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut SequencerTrack> {
        self.tracks.get_mut(&id)
    }

    /// Get all tracks in display order.
    ///
    /// Fast path (no allocation) when `track_order` is in sync with
    /// `tracks` — the case after every supported mutation. For older
    /// serialized songs with an empty/partial `track_order`, falls back
    /// to a one-shot `HashSet` to chain any missing tracks in map order.
    pub fn tracks(&self) -> impl Iterator<Item = &SequencerTrack> {
        use std::collections::HashSet;
        let in_sync = self.track_order.len() == self.tracks.len();
        let missing: HashSet<TrackId> = if in_sync {
            HashSet::new()
        } else {
            // Tracks present in the map but absent from track_order.
            self.tracks
                .keys()
                .filter(|id| !self.track_order.contains(id))
                .copied()
                .collect()
        };
        self.track_order
            .iter()
            .filter_map(|id| self.tracks.get(id))
            .chain(
                self.tracks
                    .iter()
                    .filter_map(move |(id, t)| missing.contains(id).then_some(t)),
            )
    }

    /// Get the number of tracks.
    #[must_use]
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Mutable iterator over all tracks, in `TrackId` order.
    ///
    /// Order is the underlying `BTreeMap`'s key order rather than display
    /// order; the borrow checker prevents borrowing `track_order` while
    /// holding a mutable reference to `tracks`. For order-sensitive mutation,
    /// iterate `track_order()` and call `track_mut(id)` per ID.
    pub fn tracks_mut(&mut self) -> impl Iterator<Item = &mut SequencerTrack> {
        self.tracks.values_mut()
    }

    /// Delete a track.
    pub fn delete_track(&mut self, id: TrackId) -> Option<SequencerTrack> {
        // Also remove placements on this track
        self.arrangement.retain(|p| p.track_id != id);
        self.track_order.retain(|t| *t != id);
        self.tracks.remove(&id)
    }

    /// Check if any track is soloed.
    #[must_use]
    pub fn any_solo(&self) -> bool {
        self.tracks.values().any(|t| t.solo)
    }

    /// Set `solo = true` on `target` and `solo = false` on every other track.
    ///
    /// No-op for `target` if it does not exist; other tracks are still
    /// cleared.
    pub fn set_solo_only(&mut self, target: TrackId) {
        for (id, track) in self.tracks.iter_mut() {
            track.solo = *id == target;
        }
    }

    // === Arrangement ===

    /// Place a pattern in the arrangement.
    ///
    /// Returns `false` if the pattern or track does not exist.
    pub fn place_pattern(&mut self, pattern_id: PatternId, track_id: TrackId, start: Tick) -> bool {
        if !self.patterns.contains_key(&pattern_id) || !self.tracks.contains_key(&track_id) {
            return false;
        }

        // Check for duplicate placement at same position and track
        let already_exists = self
            .arrangement
            .iter()
            .any(|p| p.start == start && p.track_id == track_id);
        if already_exists {
            return false;
        }

        let placement = PatternPlacement::new(pattern_id, track_id, start);

        // Insert sorted by start time
        let pos = self.arrangement.partition_point(|p| p.start <= start);
        self.arrangement.insert(pos, placement);
        true
    }

    /// Remove a placement at a specific position.
    pub fn remove_placement(
        &mut self,
        pattern_id: PatternId,
        track_id: TrackId,
        start: Tick,
    ) -> bool {
        let pos = self
            .arrangement
            .iter()
            .position(|p| p.pattern_id == pattern_id && p.track_id == track_id && p.start == start);

        if let Some(idx) = pos {
            let _ = self.arrangement.remove(idx);
            true
        } else {
            false
        }
    }

    /// Move a placement to a new position and/or track.
    pub fn move_placement(
        &mut self,
        pattern_id: PatternId,
        from_track: TrackId,
        from_start: Tick,
        to_track: TrackId,
        to_start: Tick,
    ) -> bool {
        let pos = self.arrangement.iter().position(|p| {
            p.pattern_id == pattern_id && p.track_id == from_track && p.start == from_start
        });

        if let Some(idx) = pos {
            self.arrangement[idx].track_id = to_track;
            self.arrangement[idx].start = to_start;
            // Re-sort by start time
            self.arrangement.sort_by_key(|p| p.start);
            true
        } else {
            false
        }
    }

    /// Set a placement's length override. Identified by
    /// `(pattern_id, track_id, start)`. Returns true if found and updated.
    pub fn set_placement_length(
        &mut self,
        pattern_id: PatternId,
        track_id: TrackId,
        start: Tick,
        length: Option<Duration>,
    ) -> bool {
        for p in &mut self.arrangement {
            if p.pattern_id == pattern_id && p.track_id == track_id && p.start == start {
                p.length_override = length;
                return true;
            }
        }
        false
    }

    /// Get all placements.
    pub fn arrangement(&self) -> &[PatternPlacement] {
        &self.arrangement
    }

    /// If `pattern_id` is currently playing through some placement at
    /// `current_tick`, return the pattern-relative tick. Otherwise `None`.
    #[must_use]
    pub fn pattern_playhead_for(
        &self,
        pattern_id: PatternId,
        current_tick: Tick,
    ) -> Option<PatternTick> {
        let pattern_length = self.pattern(pattern_id)?.length;
        self.arrangement.iter().find_map(|p| {
            if p.pattern_id != pattern_id {
                return None;
            }
            let end = p.end(pattern_length);
            if current_tick >= p.start && current_tick < end {
                #[allow(clippy::cast_possible_truncation)]
                Some(PatternTick((current_tick.0 - p.start.0) as u32))
            } else {
                None
            }
        })
    }

    /// Get placements in a time range.
    pub fn placements_in_range(
        &self,
        start: Tick,
        end: Tick,
    ) -> impl Iterator<Item = &PatternPlacement> {
        self.arrangement.iter().filter(move |p| {
            let pattern_end = self
                .patterns
                .get(&p.pattern_id)
                .map(|pat| p.end(pat.length))
                .unwrap_or(p.start);
            p.start < end && pattern_end > start
        })
    }

    /// Get placements on a specific track.
    pub fn placements_on_track(
        &self,
        track_id: TrackId,
    ) -> impl Iterator<Item = &PatternPlacement> {
        self.arrangement
            .iter()
            .filter(move |p| p.track_id == track_id)
    }

    /// Find the pattern playing at a given tick.
    /// Returns the pattern ID and the tick offset within that pattern.
    ///
    /// **Known limitation:** If multiple patterns overlap at the same tick,
    /// only the first match (by arrangement order) is returned.
    #[must_use]
    pub fn pattern_at_tick(&self, tick: Tick) -> Option<(PatternId, Tick)> {
        for placement in &self.arrangement {
            if let Some(pattern) = self.patterns.get(&placement.pattern_id) {
                let pattern_end = placement.end(pattern.length);
                if tick >= placement.start && tick < pattern_end {
                    let offset = Tick(tick.0.saturating_sub(placement.start.0));
                    return Some((placement.pattern_id, offset));
                }
            }
        }
        None
    }

    // === Tempo ===

    /// Set tempo at a position.
    pub fn set_tempo_at(&mut self, tick: Tick, bpm: Bpm) {
        // Remove existing at same tick
        self.tempo_changes.retain(|t| t.tick != tick);

        let change = TempoChange { tick, bpm };
        let pos = self.tempo_changes.partition_point(|t| t.tick <= tick);
        self.tempo_changes.insert(pos, change);
    }

    /// Get tempo at a position.
    #[must_use]
    pub fn tempo_at(&self, tick: Tick) -> Bpm {
        let pos = self.tempo_changes.partition_point(|t| t.tick <= tick);
        if pos > 0 {
            self.tempo_changes[pos - 1].bpm
        } else {
            self.default_tempo
        }
    }

    /// Get all tempo changes.
    #[must_use]
    pub fn tempo_changes(&self) -> &[TempoChange] {
        &self.tempo_changes
    }

    /// Clear all tempo changes.
    pub fn clear_tempo_changes(&mut self) {
        self.tempo_changes.clear();
    }

    /// Remove the tempo change at exactly `tick`. Returns true if one was
    /// removed. No-op for ticks that don't carry an explicit change.
    pub fn remove_tempo_change(&mut self, tick: Tick) -> bool {
        let before = self.tempo_changes.len();
        self.tempo_changes.retain(|t| t.tick != tick);
        self.tempo_changes.len() != before
    }

    // === Time signature ===

    /// Set time signature at a position.
    pub fn set_time_signature_at(&mut self, tick: Tick, sig: TimeSignature) {
        self.time_signature_changes.retain(|t| t.tick != tick);

        let change = TimeSignatureChange {
            tick,
            signature: sig,
        };
        let pos = self
            .time_signature_changes
            .partition_point(|t| t.tick <= tick);
        self.time_signature_changes.insert(pos, change);
    }

    /// Get time signature at a position.
    pub fn time_signature_at(&self, tick: Tick) -> TimeSignature {
        let pos = self
            .time_signature_changes
            .partition_point(|t| t.tick <= tick);
        if pos > 0 {
            self.time_signature_changes[pos - 1].signature
        } else {
            self.default_time_signature
        }
    }

    // === Time conversion ===

    /// Convert tick to seconds (handles tempo changes).
    #[must_use]
    pub fn tick_to_seconds(&self, target: Tick) -> f64 {
        let mut seconds = 0.0;
        let mut current_tick = Tick(0);
        let mut current_tempo = self.default_tempo;

        for change in &self.tempo_changes {
            if change.tick >= target {
                break;
            }

            // Time to this tempo change
            let ticks = change.tick.0 - current_tick.0;
            let beats = ticks as f64 / TICKS_PER_QUARTER as f64;
            seconds += beats * 60.0 / f64::from(current_tempo.as_f32());

            current_tick = change.tick;
            current_tempo = change.bpm;
        }

        // Remaining ticks
        let remaining_ticks = target.0 - current_tick.0;
        let remaining_beats = remaining_ticks as f64 / TICKS_PER_QUARTER as f64;
        seconds += remaining_beats * 60.0 / f64::from(current_tempo.as_f32());

        seconds
    }

    /// Convert seconds to tick (handles tempo changes).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn seconds_to_tick(&self, target_seconds: f64) -> Tick {
        let mut seconds = 0.0;
        let mut current_tick = Tick(0);
        let mut current_tempo = self.default_tempo;

        for change in &self.tempo_changes {
            let ticks = change.tick.0 - current_tick.0;
            let beats = ticks as f64 / TICKS_PER_QUARTER as f64;
            let tempo = f64::from(current_tempo.as_f32());
            let segment_seconds = beats * 60.0 / tempo;

            if seconds + segment_seconds >= target_seconds {
                // Target is in this segment
                let remaining_seconds = target_seconds - seconds;
                let remaining_beats = remaining_seconds * tempo / 60.0;
                let remaining_ticks = (remaining_beats * TICKS_PER_QUARTER as f64) as u64;
                return Tick(current_tick.0 + remaining_ticks);
            }

            seconds += segment_seconds;
            current_tick = change.tick;
            current_tempo = change.bpm;
        }

        // Target is after all tempo changes
        let tempo = f64::from(current_tempo.as_f32());
        let remaining_seconds = target_seconds - seconds;
        let remaining_beats = remaining_seconds * tempo / 60.0;
        let remaining_ticks = (remaining_beats * TICKS_PER_QUARTER as f64) as u64;
        Tick(current_tick.0 + remaining_ticks)
    }

    /// Remove patterns and tracks not referenced by any arrangement placement.
    /// Returns `(removed_pattern_names, removed_track_names, used_instrument_ids)`.
    /// The caller is responsible for removing unused instruments from the engine.
    pub fn remove_unused(
        &mut self,
    ) -> (
        Vec<String>,
        Vec<String>,
        std::collections::HashSet<SeqInstrumentId>,
    ) {
        use std::collections::HashSet;

        // Find used patterns and tracks from arrangement
        let mut used_patterns = HashSet::new();
        let mut used_tracks = HashSet::new();
        for p in &self.arrangement {
            used_patterns.insert(p.pattern_id);
            used_tracks.insert(p.track_id);
        }

        // Remove unused patterns
        let unused_pids: Vec<_> = self
            .patterns
            .keys()
            .filter(|id| !used_patterns.contains(id))
            .copied()
            .collect();
        let removed_patterns: Vec<String> = unused_pids
            .iter()
            .filter_map(|id| self.patterns.get(id).map(|p| p.name.clone()))
            .collect();
        for pid in unused_pids {
            self.patterns.remove(&pid);
        }

        // Remove unused tracks
        let unused_tids: Vec<_> = self
            .tracks
            .keys()
            .filter(|id| !used_tracks.contains(id))
            .copied()
            .collect();
        let removed_tracks: Vec<String> = unused_tids
            .iter()
            .filter_map(|id| self.tracks.get(id).map(|t| t.name.clone()))
            .collect();
        for tid in unused_tids {
            self.tracks.remove(&tid);
            self.track_order.retain(|t| *t != tid);
        }

        // Collect instrument IDs still in use (from tracks and notes)
        let mut used_instruments = HashSet::new();
        for track in self.tracks.values() {
            if let Some(inst) = track.instrument {
                used_instruments.insert(inst);
            }
        }
        for pattern in self.patterns.values() {
            for note in pattern.notes() {
                used_instruments.insert(note.instrument);
            }
        }

        (removed_patterns, removed_tracks, used_instruments)
    }

    /// Calculate total length based on arrangement.
    pub fn calculate_length(&self) -> Tick {
        self.arrangement
            .iter()
            .filter_map(|p| {
                self.patterns
                    .get(&p.pattern_id)
                    .map(|pat| p.end(pat.length))
            })
            .max()
            .unwrap_or(Tick(0))
    }

    /// Get length in seconds.
    #[must_use]
    pub fn length_seconds(&self) -> f64 {
        self.tick_to_seconds(self.calculate_length())
    }
}

impl Default for Song {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_song_creation() {
        let song = Song::new("Test Song")
            .with_author("Test Author")
            .with_tempo(Bpm::new(140.0));

        assert_eq!(song.name, "Test Song");
        assert_eq!(song.author, "Test Author");
        assert_eq!(song.default_tempo, Bpm::new(140.0));
    }

    #[test]
    fn test_pattern_management() {
        let mut song = Song::new("Test");

        let id = song.create_pattern(Duration(3840));
        assert!(song.pattern(id).is_some());
        assert_eq!(song.pattern_count(), 1);

        song.delete_pattern(id);
        assert!(song.pattern(id).is_none());
        assert_eq!(song.pattern_count(), 0);
    }

    #[test]
    fn test_track_management() {
        let mut song = Song::new("Test");

        let id = song.create_track("Lead");
        assert_eq!(song.track(id).unwrap().name, "Lead");
        assert_eq!(song.track_count(), 1);
    }

    #[test]
    fn test_arrangement() {
        let mut song = Song::new("Test");

        let pattern_id = song.create_pattern(Duration(3840));
        let track_id = song.create_track("Track 1");

        song.place_pattern(pattern_id, track_id, Tick(0));
        song.place_pattern(pattern_id, track_id, Tick(3840));

        assert_eq!(song.arrangement().len(), 2);
    }

    #[test]
    fn test_tick_to_seconds_constant_tempo() {
        let song = Song::new("Test");
        // 120 BPM = 2 beats/sec = 1920 ticks/sec
        let seconds = song.tick_to_seconds(Tick(1920));
        assert!((seconds - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_tick_to_seconds_with_tempo_change() {
        let mut song = Song::new("Test");
        song.set_tempo_at(Tick(960), Bpm::new(240.0)); // Double tempo after 1 beat

        // First beat: 0.5 sec (120 BPM)
        // Second beat: 0.25 sec (240 BPM)
        let seconds = song.tick_to_seconds(Tick(1920));
        assert!((seconds - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_tempo_at() {
        let mut song = Song::new("Test");
        assert_eq!(song.tempo_at(Tick(0)), Bpm::new(120.0));

        song.set_tempo_at(Tick(1000), Bpm::new(180.0));
        assert_eq!(song.tempo_at(Tick(500)), Bpm::new(120.0));
        assert_eq!(song.tempo_at(Tick(1500)), Bpm::new(180.0));
    }

    #[test]
    fn test_calculate_length() {
        let mut song = Song::new("Test");

        let pattern_id = song.create_pattern(Duration(3840));
        let track_id = song.create_track("Track");

        song.place_pattern(pattern_id, track_id, Tick(0));
        song.place_pattern(pattern_id, track_id, Tick(3840));

        assert_eq!(song.calculate_length().0, 7680);
    }

    #[test]
    fn test_move_placement() {
        let mut song = Song::new("Test");
        let pattern_id = song.create_pattern(Duration(960));
        let track1 = song.create_track("Track 1");
        let track2 = song.create_track("Track 2");

        song.place_pattern(pattern_id, track1, Tick(0));

        // Move to different track and position
        assert!(song.move_placement(pattern_id, track1, Tick(0), track2, Tick(1920)));
        assert_eq!(song.arrangement().len(), 1);
        assert_eq!(song.arrangement()[0].track_id, track2);
        assert_eq!(song.arrangement()[0].start, Tick(1920));

        // Move non-existent placement returns false
        assert!(!song.move_placement(pattern_id, track1, Tick(0), track2, Tick(0)));
    }

    #[test]
    fn test_placements_in_range() {
        let mut song = Song::new("Test");

        let pattern_id = song.create_pattern(Duration(960));
        let track_id = song.create_track("Track");

        song.place_pattern(pattern_id, track_id, Tick(0));
        song.place_pattern(pattern_id, track_id, Tick(1000));
        song.place_pattern(pattern_id, track_id, Tick(2000));

        let placements: Vec<_> = song.placements_in_range(Tick(500), Tick(1500)).collect();
        assert_eq!(placements.len(), 2); // First and second overlap with range
    }

    #[test]
    fn tracks_mut_iterates_every_track() {
        let mut song = Song::new("Test");
        let a = song.create_track("A");
        let b = song.create_track("B");
        let c = song.create_track("C");

        let mut seen: Vec<TrackId> = song.tracks_mut().map(|t| t.id).collect();
        seen.sort_by_key(|id| id.0);
        assert_eq!(seen, vec![a, b, c]);

        for track in song.tracks_mut() {
            track.mute = true;
        }
        assert!(song.tracks().all(|t| t.mute));
    }

    #[test]
    fn set_solo_only_isolates_target_and_clears_others() {
        let mut song = Song::new("Test");
        let a = song.create_track("A");
        let b = song.create_track("B");
        let c = song.create_track("C");

        song.track_mut(a).unwrap().solo = true;
        song.track_mut(b).unwrap().solo = true;

        song.set_solo_only(c);

        assert!(!song.track(a).unwrap().solo);
        assert!(!song.track(b).unwrap().solo);
        assert!(song.track(c).unwrap().solo);
        assert!(song.any_solo());
    }

    #[test]
    fn set_solo_only_with_unknown_target_clears_all() {
        let mut song = Song::new("Test");
        let a = song.create_track("A");
        song.track_mut(a).unwrap().solo = true;

        // TrackId(9999) does not exist; helper must still clear existing solos.
        song.set_solo_only(TrackId(9999));

        assert!(!song.track(a).unwrap().solo);
        assert!(!song.any_solo());
    }
}
