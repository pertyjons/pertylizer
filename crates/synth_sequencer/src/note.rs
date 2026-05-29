//! Note storage type.
//!
//! Notes are stored with start time and duration, not as separate on/off events.

use serde::{Deserialize, Serialize};
use synth_core::{Milliseconds, Semitones};

use super::ids::{NoteId, TrackId};
use super::pitch::{Pitch, Velocity};
use super::time::{Duration, PatternTick};

/// Interpolation type for a per-note glide.
///
/// The single taxonomy axis (roadmap: expression primitive 2) that separates a
/// continuous portamento from a stepped glissando. `Stepped` quantizes the glide
/// trajectory to semitones — the intentional-holds case, where smoothing would be
/// the bug (the inverse of the zipper-noise pitfall).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum GlideInterp {
    /// Continuous portamento (smooth pitch ramp).
    #[default]
    Continuous,
    /// Stepped glissando (quantized to semitones).
    Stepped,
}

/// Where a per-note glide starts from.
///
/// Either a signed semitone offset relative to the note's own pitch (the common
/// case) or an absolute source pitch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum GlideFrom {
    /// Signed semitone offset from this note's pitch (e.g. `-12.0` = an octave below).
    Semitones(Semitones),
    /// Absolute source pitch to glide from.
    Pitch(Pitch),
}

/// Per-note glide (portamento / glissando) — taxonomy primitive 2.
///
/// Drives the engine's existing `GlideState` (exponential portamento) from
/// per-note sequencer data. `Copy`/alloc-free so it can ride the audio-thread
/// `SequencerEvent::NoteOn` without heap allocation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Glide {
    /// Where the glide starts from.
    pub from: GlideFrom,
    /// Glide time (how long to reach this note's pitch).
    pub time: Milliseconds,
    /// Continuous (portamento) vs stepped (glissando).
    pub interp: GlideInterp,
}

/// A note in a pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// Track/channel for mono-per-track behavior.
    pub track: Option<TrackId>,
    /// Tie / legato intent: when set, this note connects to its successor without
    /// re-gating (taxonomy primitive 2). Additive; defaults to `false`.
    #[serde(default)]
    pub legato: bool,
    /// Per-note glide (portamento/glissando). Additive; defaults to `None`.
    #[serde(default)]
    pub glide: Option<Glide>,
}

impl Note {
    /// Create a new note with default duration.
    #[must_use]
    pub fn new(id: NoteId, start: PatternTick, pitch: Pitch, velocity: Velocity) -> Self {
        Self {
            id,
            start,
            duration: None,
            pitch,
            velocity,
            track: None,
            legato: false,
            glide: None,
        }
    }

    /// Set the duration (builder pattern).
    #[must_use]
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Set the track (builder pattern).
    #[must_use]
    pub fn with_track(mut self, track: TrackId) -> Self {
        self.track = Some(track);
        self
    }

    /// Set the legato/tie flag (builder pattern).
    #[must_use]
    pub fn with_legato(mut self, legato: bool) -> Self {
        self.legato = legato;
        self
    }

    /// Set the per-note glide (builder pattern).
    #[must_use]
    pub fn with_glide(mut self, glide: Glide) -> Self {
        self.glide = Some(glide);
        self
    }

    /// Get the end tick (None if duration is None).
    pub fn end(&self) -> Option<PatternTick> {
        self.duration.map(|d| self.start + d)
    }

    /// Check if the note is playing at the given tick.
    #[must_use]
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
    #[must_use]
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
        )
        .with_duration(Duration(200));

        assert!(!late_note.is_playing_at(PatternTick(50)));
        assert!(late_note.is_playing_at(PatternTick(100)));
        assert!(late_note.is_playing_at(PatternTick(200)));
        assert!(!late_note.is_playing_at(PatternTick(300)));
    }

    #[test]
    fn test_note_defaults_legato_glide() {
        let note = test_note();
        assert!(!note.legato);
        assert!(note.glide.is_none());
    }

    #[test]
    fn test_note_with_legato_and_glide() {
        let glide = Glide {
            from: GlideFrom::Semitones(Semitones::new(-12.0)),
            time: Milliseconds::new(50.0),
            interp: GlideInterp::Continuous,
        };
        let note = test_note().with_legato(true).with_glide(glide);
        assert!(note.legato);
        assert_eq!(note.glide, Some(glide));
    }

    #[test]
    fn test_note_serde_roundtrip_with_expression() {
        let glide = Glide {
            from: GlideFrom::Pitch(Pitch::new(48).unwrap()),
            time: Milliseconds::new(120.0),
            interp: GlideInterp::Stepped,
        };
        let note = test_note().with_legato(true).with_glide(glide);
        let json = serde_json::to_string(&note).unwrap();
        let back: Note = serde_json::from_str(&json).unwrap();
        assert_eq!(note, back);
    }

    #[test]
    fn test_note_deserializes_without_new_fields() {
        // Legacy JSON predating legato/glide must load with defaults (additive schema).
        let json = r#"{
            "id": 7,
            "start": 0,
            "duration": null,
            "pitch": 60,
            "velocity": 80,
            "track": null
        }"#;
        let note: Note = serde_json::from_str(json).unwrap();
        assert!(!note.legato);
        assert!(note.glide.is_none());
    }

    #[test]
    fn test_note_overlaps() {
        let note = Note::new(
            NoteId(0),
            PatternTick(100),
            Pitch::new(60).unwrap(),
            Velocity::MF,
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
