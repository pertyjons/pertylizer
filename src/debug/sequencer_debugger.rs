//! Sequencer debugger for stepping through songs tick-by-tick.
//!
//! Allows offline analysis of event generation without real-time playback.

use std::sync::{Arc, RwLock};

use crate::engine::SequencerEngine;
use crate::sequencer::{Pitch, SeqInstrumentId, SequencerEvent, Song, Tick, TrackId};
use crate::types::{Bpm, SampleRate};

/// Playback state (re-exported from sequencer_engine).
pub use crate::engine::PlayState;

/// Source of an event (why it was generated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventSource {
    /// Note started at this tick.
    NoteStart,
    /// Note ended due to its duration.
    NoteDuration,
    /// Note was stopped by a new note on the same track (mono-per-track).
    MonoPerTrack,
    /// User/debugger manually stopped playback.
    ManualStop,
    /// Loop point was reached.
    LoopPoint,
}

/// An event with metadata about when and why it occurred.
#[derive(Debug, Clone)]
pub struct TimestampedEvent {
    /// Tick when the event occurred.
    pub tick: Tick,
    /// The actual sequencer event.
    pub event: SequencerEvent,
    /// Why this event was generated.
    pub source: EventSource,
}

impl TimestampedEvent {
    /// Create a new timestamped event.
    pub fn new(tick: Tick, event: SequencerEvent, source: EventSource) -> Self {
        Self {
            tick,
            event,
            source,
        }
    }
}

/// Information about an active note.
#[derive(Debug, Clone)]
pub struct ActiveNoteInfo {
    /// Pitch of the note.
    pub pitch: Pitch,
    /// Instrument playing the note.
    pub instrument: SeqInstrumentId,
    /// Track the note is on (for mono-per-track).
    pub track: Option<TrackId>,
    /// When the note started.
    pub started_at: Tick,
    /// When the note will end (if duration is known).
    pub will_end_at: Option<Tick>,
}

/// Snapshot of sequencer state at a point in time.
#[derive(Debug, Clone)]
pub struct SequencerSnapshot {
    /// Current tick position.
    pub current_tick: Tick,
    /// Current tempo.
    pub tempo: Bpm,
    /// Currently playing notes.
    pub active_notes: Vec<ActiveNoteInfo>,
    /// Current playback state.
    pub play_state: PlayState,
}

/// Debugger for stepping through a song tick-by-tick.
///
/// This allows offline analysis of event generation without real-time audio.
pub struct SequencerDebugger {
    /// The sequencer engine (configured for offline use).
    engine: SequencerEngine,
    /// Event log (all events generated so far).
    event_log: Vec<TimestampedEvent>,
    /// Shared song reference.
    song: Arc<RwLock<Song>>,
}

impl SequencerDebugger {
    /// Create a debugger for a song.
    pub fn new(song: Song) -> Self {
        let song_arc = Arc::new(RwLock::new(song));
        // Use a reasonable sample rate for offline processing
        let sample_rate = SampleRate::CD_QUALITY;
        let engine = SequencerEngine::with_song(Arc::clone(&song_arc), sample_rate);

        Self {
            engine,
            event_log: Vec::new(),
            song: song_arc,
        }
    }

    /// Create a debugger from a shared song reference.
    pub fn from_shared(song: Arc<RwLock<Song>>) -> Self {
        let sample_rate = SampleRate::CD_QUALITY;
        let engine = SequencerEngine::with_song(Arc::clone(&song), sample_rate);

        Self {
            engine,
            event_log: Vec::new(),
            song,
        }
    }

    /// Get the current tick position.
    pub fn current_tick(&self) -> Tick {
        self.engine.current_tick()
    }

    /// Get the current play state.
    pub fn play_state(&self) -> PlayState {
        self.engine.play_state()
    }

    /// Step forward by one tick, returning any events generated.
    pub fn step(&mut self) -> Vec<TimestampedEvent> {
        self.step_to(Tick(self.engine.current_tick().0 + 1))
    }

    /// Step forward to a specific tick, returning all events generated along the way.
    pub fn step_to(&mut self, target: Tick) -> Vec<TimestampedEvent> {
        // Ensure we're playing
        if self.engine.play_state() != PlayState::Playing {
            self.engine.play();
        }

        let mut events = Vec::new();
        let mut raw_events = Vec::new();

        // We need to step tick by tick to accurately capture events
        while self.engine.current_tick() < target {
            raw_events.clear();

            // Process one tick worth of samples
            // At 44100Hz and 120 BPM with 96 ticks per quarter note:
            // samples_per_tick = (44100 / (120/60)) / 96 ≈ 229.69 samples
            // We'll process enough samples to advance at least one tick
            let samples_per_tick = self.calculate_samples_per_tick();
            self.engine.process(
                crate::types::SampleCount::new(samples_per_tick),
                &mut raw_events,
            );

            // Convert raw events to timestamped events
            let current_tick = self.engine.current_tick();
            for event in &raw_events {
                let source = self.classify_event_source(event);
                let timestamped = TimestampedEvent::new(current_tick, event.clone(), source);
                events.push(timestamped.clone());
                self.event_log.push(timestamped);
            }
        }

        events
    }

    /// Step to the next event (skip empty ticks).
    pub fn step_to_next_event(&mut self) -> Option<Vec<TimestampedEvent>> {
        // Set a reasonable maximum to prevent infinite loops
        const MAX_TICKS: u64 = 100_000;

        for _ in 0..MAX_TICKS {
            let events = self.step();
            if !events.is_empty() {
                return Some(events);
            }

            // Check if we've reached the end of the song
            if let Ok(song) = self.song.read()
                && self.engine.current_tick() >= song.calculate_length()
            {
                return None;
            }
        }

        None
    }

    /// Reset to the beginning.
    pub fn reset(&mut self) {
        let events = self.engine.stop();

        // Log the stop events
        for event in events {
            let timestamped =
                TimestampedEvent::new(self.engine.current_tick(), event, EventSource::ManualStop);
            self.event_log.push(timestamped);
        }

        self.event_log.clear();
    }

    /// Get a snapshot of the current state.
    pub fn snapshot(&self) -> SequencerSnapshot {
        let tempo = if let Ok(song) = self.song.read() {
            song.tempo_at(self.engine.current_tick())
        } else {
            Bpm::new(120.0)
        };

        // We can't easily get active notes from the engine without exposing internals,
        // so we reconstruct from the event log
        let active_notes = self.reconstruct_active_notes();

        SequencerSnapshot {
            current_tick: self.engine.current_tick(),
            tempo,
            active_notes,
            play_state: self.engine.play_state(),
        }
    }

    /// Get all events generated so far.
    pub fn events(&self) -> &[TimestampedEvent] {
        &self.event_log
    }

    /// Get events of a specific type.
    pub fn note_on_events(&self) -> Vec<&TimestampedEvent> {
        self.event_log
            .iter()
            .filter(|e| e.event.is_note_on())
            .collect()
    }

    /// Get note-off events.
    pub fn note_off_events(&self) -> Vec<&TimestampedEvent> {
        self.event_log
            .iter()
            .filter(|e| e.event.is_note_off())
            .collect()
    }

    /// Get events at a specific tick.
    pub fn events_at(&self, tick: Tick) -> Vec<&TimestampedEvent> {
        self.event_log.iter().filter(|e| e.tick == tick).collect()
    }

    /// Get events for a specific instrument.
    pub fn events_for_instrument(&self, instrument: SeqInstrumentId) -> Vec<&TimestampedEvent> {
        self.event_log
            .iter()
            .filter(|e| e.event.instrument() == Some(instrument))
            .collect()
    }

    /// Generate a text summary of the event log.
    pub fn summarize(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!(
            "=== Sequencer Debug Summary (tick 0 to {}) ===",
            self.engine.current_tick().0
        ));
        lines.push(String::new());

        let note_ons = self.note_on_events().len();
        let note_offs = self.note_off_events().len();
        lines.push(format!("Total events: {}", self.event_log.len()));
        lines.push(format!("  NoteOn: {}", note_ons));
        lines.push(format!("  NoteOff: {}", note_offs));
        lines.push(String::new());

        // Group by tick
        let mut current_tick = Tick(u64::MAX);
        for event in &self.event_log {
            if event.tick != current_tick {
                current_tick = event.tick;
                lines.push(format!("Tick {}:", current_tick.0));
            }
            lines.push(format!("  {:?} ({:?})", event.event, event.source));
        }

        lines.join("\n")
    }

    // === Private helpers ===

    fn calculate_samples_per_tick(&self) -> usize {
        // samples_per_tick = sample_rate / (bpm/60 * ticks_per_beat)
        // At 44100Hz, 120 BPM, 96 ticks per quarter: 44100 / (2 * 96) ≈ 229.7
        let tempo = if let Ok(song) = self.song.read() {
            song.tempo_at(self.engine.current_tick())
        } else {
            Bpm::new(120.0)
        };

        let beats_per_second = tempo.as_f32() / 60.0;
        let ticks_per_second = beats_per_second * crate::sequencer::TICKS_PER_QUARTER as f32;
        let samples_per_tick = self.engine.sample_rate().as_f32() / ticks_per_second;

        // Always process at least one sample
        (samples_per_tick.ceil() as usize).max(1)
    }

    fn classify_event_source(&self, event: &SequencerEvent) -> EventSource {
        match event {
            SequencerEvent::NoteOn { .. } => EventSource::NoteStart,
            SequencerEvent::NoteOff { .. } => {
                // We could try to determine the source more precisely,
                // but that would require tracking more state
                EventSource::NoteDuration
            }
            SequencerEvent::Effect { .. } => EventSource::NoteStart, // Effects come with notes
            SequencerEvent::Parameter { .. } => EventSource::NoteStart,
        }
    }

    fn reconstruct_active_notes(&self) -> Vec<ActiveNoteInfo> {
        // Track which notes are currently active by (pitch, instrument)
        use std::collections::HashMap;
        let mut active: HashMap<(u8, SeqInstrumentId), ActiveNoteInfo> = HashMap::new();

        for event in &self.event_log {
            match &event.event {
                SequencerEvent::NoteOn {
                    pitch, instrument, ..
                } => {
                    let key = (pitch.as_midi(), *instrument);
                    active.insert(
                        key,
                        ActiveNoteInfo {
                            pitch: *pitch,
                            instrument: *instrument,
                            track: None, // We don't have this info from events
                            started_at: event.tick,
                            will_end_at: None,
                        },
                    );
                }
                SequencerEvent::NoteOff {
                    pitch, instrument, ..
                } => {
                    let key = (pitch.as_midi(), *instrument);
                    active.remove(&key);
                }
                _ => {}
            }
        }

        active.into_values().collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::sequencer::{Duration, PatternTick, Velocity};

    fn create_simple_song() -> Song {
        let mut song = Song::default();

        // Create a pattern with one note (4 beats = 384 ticks)
        let pattern_id = song.create_pattern(Duration(384));

        // Add a note to the pattern
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            pattern.add_note(
                PatternTick(0),
                Pitch::new(60).unwrap(),
                Velocity::MF,
                SeqInstrumentId(0),
            );
        }

        // Place pattern at tick 0
        let track_id = song.create_track("Track 1");
        song.place_pattern(pattern_id, track_id, Tick(0));

        song
    }

    #[test]
    fn test_debugger_step() {
        let song = create_simple_song();
        let mut dbg = SequencerDebugger::new(song);

        // Step forward
        let events = dbg.step_to(Tick(1));

        // Should have at least one NoteOn event
        let note_ons: Vec<_> = events.iter().filter(|e| e.event.is_note_on()).collect();
        assert!(!note_ons.is_empty(), "Expected NoteOn event at tick 0");
    }

    #[test]
    fn test_debugger_reset() {
        let song = create_simple_song();
        let mut dbg = SequencerDebugger::new(song);

        dbg.step_to(Tick(100));
        assert!(dbg.current_tick().0 >= 100);

        dbg.reset();
        assert_eq!(dbg.current_tick(), Tick::ZERO);
    }

    #[test]
    fn test_debugger_snapshot() {
        let song = create_simple_song();
        let mut dbg = SequencerDebugger::new(song);

        let snap1 = dbg.snapshot();
        assert_eq!(snap1.current_tick, Tick::ZERO);

        dbg.step_to(Tick(50));
        let snap2 = dbg.snapshot();
        assert!(snap2.current_tick.0 >= 50);
    }
}
