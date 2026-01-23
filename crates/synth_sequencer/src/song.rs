//! Song structure and arrangement.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::ids::{PatternId, RowCount, TrackCount, TrackId};
use super::pattern::Pattern;
use super::time::{Duration, TICKS_PER_QUARTER, Tick, TimeSignature};
use super::track::SequencerTrack;
use super::tracker_pattern::TrackerPattern;
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

    /// Calculate end position based on pattern length.
    pub fn end(&self, pattern_length: Duration) -> Tick {
        Tick(self.start.0 + pattern_length.0 as u64)
    }
}

/// A complete song with patterns, tracks, and arrangement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Song {
    /// Song name.
    pub name: String,
    /// Song author.
    pub author: String,

    // Pattern storage (piano roll style)
    patterns: HashMap<PatternId, Pattern>,
    next_pattern_id: u32,

    // Tracker pattern storage (tracker style)
    #[serde(default)]
    tracker_patterns: HashMap<PatternId, TrackerPattern>,

    // Track storage
    tracks: HashMap<TrackId, SequencerTrack>,
    next_track_id: u16,

    // Arrangement
    arrangement: Vec<PatternPlacement>,

    // Tempo and time signature
    tempo_changes: Vec<TempoChange>,
    time_signature_changes: Vec<TimeSignatureChange>,
    /// Default tempo (BPM).
    pub default_tempo: Bpm,
    /// Default time signature.
    pub default_time_signature: TimeSignature,
    /// Default tracker speed (ticks per row, typically 6).
    /// Used for XM/MOD/S3M/IT modules where tempo is controlled by both BPM and speed.
    #[serde(default = "default_tracker_speed")]
    pub default_tracker_speed: u8,
}

/// Default tracker speed (6 ticks per row, like FastTracker 2).
fn default_tracker_speed() -> u8 {
    6
}

impl Song {
    /// Create a new empty song.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: String::new(),
            patterns: HashMap::new(),
            next_pattern_id: 0,
            tracker_patterns: HashMap::new(),
            tracks: HashMap::new(),
            next_track_id: 0,
            arrangement: Vec::new(),
            tempo_changes: Vec::new(),
            time_signature_changes: Vec::new(),
            default_tempo: Bpm::new(120.0),
            default_time_signature: TimeSignature::COMMON,
            default_tracker_speed: default_tracker_speed(),
        }
    }

    /// Set the author (builder pattern).
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }

    /// Set the default tempo (builder pattern).
    pub fn with_tempo(mut self, bpm: Bpm) -> Self {
        self.default_tempo = bpm;
        self
    }

    /// Set the default time signature (builder pattern).
    pub fn with_time_signature(mut self, sig: TimeSignature) -> Self {
        self.default_time_signature = sig;
        self
    }

    /// Set the default tracker speed (builder pattern).
    /// Speed is the number of ticks per row (typically 6 in FastTracker 2).
    pub fn with_tracker_speed(mut self, speed: u8) -> Self {
        self.default_tracker_speed = speed.max(1); // Prevent division by zero
        self
    }

    // === Pattern management ===

    /// Create a new pattern and add it to the song.
    pub fn create_pattern(&mut self, length: Duration) -> PatternId {
        let id = PatternId(self.next_pattern_id);
        self.next_pattern_id += 1;
        self.patterns.insert(id, Pattern::new(id, length));
        id
    }

    /// Get a pattern by ID.
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
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    /// Delete a pattern.
    pub fn delete_pattern(&mut self, id: PatternId) -> Option<Pattern> {
        // Also remove from arrangement
        self.arrangement.retain(|p| p.pattern_id != id);
        self.patterns.remove(&id)
    }

    /// Duplicate a pattern.
    pub fn duplicate_pattern(&mut self, id: PatternId) -> Option<PatternId> {
        let pattern = self.patterns.get(&id)?.clone();
        let new_id = PatternId(self.next_pattern_id);
        self.next_pattern_id += 1;

        let mut new_pattern = Pattern::new(new_id, pattern.length);
        new_pattern.name = format!("{} (copy)", pattern.name);
        new_pattern.row_resolution = pattern.row_resolution;

        // Copy notes
        for note in pattern.notes() {
            new_pattern.insert_note(note.clone());
        }

        self.patterns.insert(new_id, new_pattern);
        Some(new_id)
    }

    // === Tracker pattern management ===

    /// Create a new tracker pattern and add it to the song.
    pub fn create_tracker_pattern(
        &mut self,
        num_tracks: TrackCount,
        num_rows: RowCount,
    ) -> PatternId {
        let id = PatternId(self.next_pattern_id);
        self.next_pattern_id += 1;
        self.tracker_patterns
            .insert(id, TrackerPattern::new(id, num_tracks, num_rows));
        id
    }

    /// Add a tracker pattern directly (e.g., from import).
    pub fn add_tracker_pattern(&mut self, pattern: TrackerPattern) -> PatternId {
        let id = pattern.id();
        // Update next_pattern_id if needed
        if id.0 >= self.next_pattern_id {
            self.next_pattern_id = id.0 + 1;
        }
        self.tracker_patterns.insert(id, pattern);
        id
    }

    /// Get a tracker pattern by ID.
    pub fn tracker_pattern(&self, id: PatternId) -> Option<&TrackerPattern> {
        self.tracker_patterns.get(&id)
    }

    /// Get a mutable tracker pattern by ID.
    pub fn tracker_pattern_mut(&mut self, id: PatternId) -> Option<&mut TrackerPattern> {
        self.tracker_patterns.get_mut(&id)
    }

    /// Get all tracker patterns.
    pub fn tracker_patterns(&self) -> impl Iterator<Item = &TrackerPattern> {
        self.tracker_patterns.values()
    }

    /// Get the number of tracker patterns.
    pub fn tracker_pattern_count(&self) -> usize {
        self.tracker_patterns.len()
    }

    /// Check if a pattern ID refers to a tracker pattern.
    pub fn is_tracker_pattern(&self, id: PatternId) -> bool {
        self.tracker_patterns.contains_key(&id)
    }

    /// Delete a tracker pattern.
    pub fn delete_tracker_pattern(&mut self, id: PatternId) -> Option<TrackerPattern> {
        // Also remove from arrangement
        self.arrangement.retain(|p| p.pattern_id != id);
        self.tracker_patterns.remove(&id)
    }

    // === Track management ===

    /// Create a new track.
    pub fn create_track(&mut self, name: impl Into<String>) -> TrackId {
        let id = TrackId(self.next_track_id);
        self.next_track_id += 1;
        self.tracks.insert(id, SequencerTrack::new(id, name));
        id
    }

    /// Get a track by ID.
    pub fn track(&self, id: TrackId) -> Option<&SequencerTrack> {
        self.tracks.get(&id)
    }

    /// Get a mutable track by ID.
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut SequencerTrack> {
        self.tracks.get_mut(&id)
    }

    /// Get all tracks.
    pub fn tracks(&self) -> impl Iterator<Item = &SequencerTrack> {
        self.tracks.values()
    }

    /// Get the number of tracks.
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Delete a track.
    pub fn delete_track(&mut self, id: TrackId) -> Option<SequencerTrack> {
        // Also remove placements on this track
        self.arrangement.retain(|p| p.track_id != id);
        self.tracks.remove(&id)
    }

    /// Check if any track is soloed.
    pub fn any_solo(&self) -> bool {
        self.tracks.values().any(|t| t.solo)
    }

    // === Arrangement ===

    /// Place a pattern in the arrangement.
    pub fn place_pattern(&mut self, pattern_id: PatternId, track_id: TrackId, start: Tick) {
        let placement = PatternPlacement::new(pattern_id, track_id, start);

        // Insert sorted by start time
        let pos = self.arrangement.partition_point(|p| p.start <= start);
        self.arrangement.insert(pos, placement);
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
            self.arrangement.remove(idx);
            true
        } else {
            false
        }
    }

    /// Get all placements.
    pub fn arrangement(&self) -> &[PatternPlacement] {
        &self.arrangement
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
    pub fn pattern_at_tick(&self, tick: Tick) -> Option<(PatternId, Tick)> {
        // Find placement that contains this tick
        for placement in &self.arrangement {
            // Check regular patterns first
            if let Some(pattern) = self.patterns.get(&placement.pattern_id) {
                let pattern_end = placement.end(pattern.length);
                if tick >= placement.start && tick < pattern_end {
                    let offset = Tick(tick.0.saturating_sub(placement.start.0));
                    return Some((placement.pattern_id, offset));
                }
            }
            // Check tracker patterns
            else if let Some(tracker_pattern) = self.tracker_patterns.get(&placement.pattern_id) {
                let length = Duration(tracker_pattern.length_ticks().0);
                let pattern_end = placement.end(length);
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
    pub fn tempo_at(&self, tick: Tick) -> Bpm {
        self.tempo_changes
            .iter()
            .rev()
            .find(|t| t.tick <= tick)
            .map(|t| t.bpm)
            .unwrap_or(self.default_tempo)
    }

    /// Get all tempo changes.
    pub fn tempo_changes(&self) -> &[TempoChange] {
        &self.tempo_changes
    }

    /// Clear all tempo changes.
    pub fn clear_tempo_changes(&mut self) {
        self.tempo_changes.clear();
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
        self.time_signature_changes
            .iter()
            .rev()
            .find(|t| t.tick <= tick)
            .map(|t| t.signature)
            .unwrap_or(self.default_time_signature)
    }

    // === Time conversion ===

    /// Convert tick to seconds (handles tempo changes).
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
}
