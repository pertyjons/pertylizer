//! Note storage type.
//!
//! Notes are stored with start time and duration, not as separate on/off events.

use serde::{Deserialize, Serialize};

use super::ids::{NoteId, SeqInstrumentId, TrackId};
use super::pitch::{Pitch, Velocity};
use super::time::{Duration, PatternTick};

/// A note in a pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// Unique ID for this note (for editing/selection).
    pub id: NoteId,
    /// Start position within the pattern.
    pub start: PatternTick,
    /// Duration in ticks.
    pub duration: Option<Duration>,
    /// Pitch.
    pub pitch: Pitch,
    /// Velocity/attack strength.
    pub velocity: Velocity,
    /// Instrument that plays this note.
    pub instrument: SeqInstrumentId,
    /// Track/channel for mono-per-track behavior.
    pub track: Option<TrackId>,
}

impl Note {
    /// Create a new note with default duration.
    pub fn new(
        id: NoteId,
        start: PatternTick,
        pitch: Pitch,
        velocity: Velocity,
        instrument: SeqInstrumentId,
    ) -> Self {
        Self {
            id,
            start,
            duration: None,
            pitch,
            velocity,
            instrument,
            track: None,
        }
    }

    /// Set the duration (builder pattern).
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Set the track (builder pattern).
    pub fn with_track(mut self, track: TrackId) -> Self {
        self.track = Some(track);
        self
    }

    /// Get the end tick (None if duration is None).
    pub fn end(&self) -> Option<PatternTick> {
        self.duration.map(|d| PatternTick(self.start.0 + d.0))
    }

    /// Check if the note is playing at the given tick.
    pub fn is_playing_at(&self, tick: PatternTick) -> bool {
        if tick < self.start {
            return false;
        }
        match self.end() {
            Some(end) => tick < end,
            None => true, // No explicit end, plays until cut
        }
    }

    /// Check if this note overlaps with another time range.
    pub fn overlaps(&self, start: PatternTick, end: PatternTick) -> bool {
        let note_end = self.end().unwrap_or(PatternTick(u32::MAX));
        self.start < end && note_end > start
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_note() -> Note {
        Note::new(
            NoteId(0),
            PatternTick(0),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        )
    }

    #[test]
    fn test_note_creation() {
        let note = test_note();
        assert_eq!(note.pitch.as_midi(), 60);
        assert_eq!(note.start.0, 0);
        assert!(note.duration.is_none());
    }

    #[test]
    fn test_note_with_duration() {
        let note = test_note().with_duration(Duration::QUARTER);
        assert_eq!(note.duration, Some(Duration::QUARTER));
        assert_eq!(note.end(), Some(PatternTick(960)));
    }

    #[test]
    fn test_note_is_playing_at() {
        // Note starts at tick 0 with quarter note duration (960 ticks)
        let note = test_note().with_duration(Duration::QUARTER);

        assert!(note.is_playing_at(PatternTick(0)));
        assert!(note.is_playing_at(PatternTick(480)));
        assert!(!note.is_playing_at(PatternTick(960)));
        assert!(!note.is_playing_at(PatternTick(1000)));

        let late_note = Note::new(
            NoteId(1),
            PatternTick(100),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        )
        .with_duration(Duration(200));

        assert!(!late_note.is_playing_at(PatternTick(50)));
        assert!(late_note.is_playing_at(PatternTick(100)));
        assert!(late_note.is_playing_at(PatternTick(200)));
        assert!(!late_note.is_playing_at(PatternTick(300)));
    }

    #[test]
    fn test_note_overlaps() {
        let note = Note::new(
            NoteId(0),
            PatternTick(100),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            SeqInstrumentId(0),
        )
        .with_duration(Duration(200));

        // Overlapping ranges
        assert!(note.overlaps(PatternTick(0), PatternTick(150)));
        assert!(note.overlaps(PatternTick(200), PatternTick(400)));
        assert!(note.overlaps(PatternTick(150), PatternTick(250)));

        // Non-overlapping ranges
        assert!(!note.overlaps(PatternTick(0), PatternTick(100)));
        assert!(!note.overlaps(PatternTick(300), PatternTick(400)));
    }
}
