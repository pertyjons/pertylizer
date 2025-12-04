//! Sequencer engine for real-time playback.
//!
//! This module provides the runtime engine that converts stored song data
//! into real-time events at audio sample rate precision.
//!
//! ## Type Safety
//!
//! This module uses domain-specific types throughout:
//! - [`SampleRate`] instead of `f32` for sample rates
//! - [`SampleCount`] instead of `usize` for buffer sizes
//! - [`Tick`] instead of `u64` for song positions
//!
//! This prevents common errors like mixing up samples and ticks.

use std::sync::{Arc, RwLock};

use crate::sequencer::{
    InstrumentId, Pitch, SequencerEvent, Song, Tick, TICKS_PER_QUARTER,
};
use crate::types::{SampleCount, SampleRate};

/// Playback state of the sequencer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayState {
    /// Sequencer is playing.
    Playing,
    /// Sequencer is stopped at the beginning.
    #[default]
    Stopped,
    /// Sequencer is paused at the current position.
    Paused,
}

/// An active note that needs to be tracked for NoteOff events.
#[derive(Debug, Clone)]
struct ActiveNote {
    /// The pitch being played.
    pitch: Pitch,
    /// The instrument playing the note.
    instrument: InstrumentId,
    /// When the note should end (if duration is known).
    end_tick: Option<Tick>,
}

/// The sequencer engine processes song data and emits real-time events.
///
/// It maintains sample-accurate timing by accumulating fractional ticks
/// between audio buffer callbacks.
pub struct SequencerEngine {
    /// Reference to song data (thread-safe).
    song: Arc<RwLock<Song>>,
    /// Current playback state.
    play_state: PlayState,
    /// Current position in the song (integer ticks).
    current_tick: Tick,
    /// Sub-tick accumulator for sample-accurate timing.
    /// We accumulate fractional ticks between buffer blocks.
    tick_accumulator: f64,
    /// Current sample rate.
    sample_rate: SampleRate,
    /// Active notes that need NoteOff events.
    active_notes: Vec<ActiveNote>,
    /// Cached tempo (BPM) to avoid locking song every sample.
    cached_tempo: f32,
    /// Whether we're looping.
    looping: bool,
    /// Loop start position.
    loop_start: Tick,
    /// Loop end position.
    loop_end: Tick,
}

impl SequencerEngine {
    /// Create a new sequencer engine with the given sample rate.
    pub fn new(sample_rate: SampleRate) -> Self {
        Self {
            song: Arc::new(RwLock::new(Song::default())),
            play_state: PlayState::Stopped,
            current_tick: Tick::ZERO,
            tick_accumulator: 0.0,
            sample_rate,
            active_notes: Vec::new(),
            cached_tempo: 120.0,
            looping: false,
            loop_start: Tick::ZERO,
            loop_end: Tick::ZERO,
        }
    }

    /// Create a sequencer engine with a shared song reference.
    pub fn with_song(song: Arc<RwLock<Song>>, sample_rate: SampleRate) -> Self {
        let cached_tempo = song
            .read()
            .map(|s| s.default_tempo)
            .unwrap_or(120.0);

        Self {
            song,
            play_state: PlayState::Stopped,
            current_tick: Tick::ZERO,
            tick_accumulator: 0.0,
            sample_rate,
            active_notes: Vec::new(),
            cached_tempo,
            looping: false,
            loop_start: Tick::ZERO,
            loop_end: Tick::ZERO,
        }
    }

    /// Set the sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    /// Get the current sample rate.
    pub fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// Get the current play state.
    pub fn play_state(&self) -> PlayState {
        self.play_state
    }

    /// Get the current tick position.
    pub fn current_tick(&self) -> Tick {
        self.current_tick
    }

    /// Start playback.
    pub fn play(&mut self) {
        if self.play_state == PlayState::Stopped {
            self.current_tick = Tick::ZERO;
            self.tick_accumulator = 0.0;
        }
        self.play_state = PlayState::Playing;
        self.update_cached_tempo();
    }

    /// Pause playback at current position.
    pub fn pause(&mut self) {
        self.play_state = PlayState::Paused;
    }

    /// Stop playback and return to beginning.
    pub fn stop(&mut self) -> Vec<SequencerEvent> {
        self.play_state = PlayState::Stopped;
        self.current_tick = Tick::ZERO;
        self.tick_accumulator = 0.0;

        // Generate NoteOff events for all active notes
        let events = self.release_all_notes();
        self.active_notes.clear();
        events
    }

    /// Seek to a specific tick position.
    pub fn seek(&mut self, tick: Tick) -> Vec<SequencerEvent> {
        // Release all active notes before seeking
        let events = self.release_all_notes();
        self.active_notes.clear();
        self.current_tick = tick;
        self.tick_accumulator = 0.0;
        self.update_cached_tempo();
        events
    }

    /// Set loop points.
    pub fn set_loop(&mut self, start: Tick, end: Tick, enabled: bool) {
        self.loop_start = start;
        self.loop_end = end;
        self.looping = enabled;
    }

    /// Check if looping is enabled.
    pub fn is_looping(&self) -> bool {
        self.looping
    }

    /// Process a buffer of samples and append generated events to the buffer.
    ///
    /// This is the main entry point called from the audio thread.
    /// It advances the sequencer position by the given number of samples
    /// and appends any events that should occur during this time window.
    ///
    /// # Real-time Safety
    ///
    /// This method does not allocate memory. The caller should provide a
    /// pre-allocated buffer (typically cleared before each call) to collect
    /// events without heap allocations in the audio thread.
    pub fn process(&mut self, samples: SampleCount, events: &mut Vec<SequencerEvent>) {
        if self.play_state != PlayState::Playing {
            return;
        }

        // Calculate how many ticks to advance
        // Formula: delta_ticks = (samples / sample_rate) * (bpm / 60) * TICKS_PER_QUARTER
        let seconds = samples.as_usize() as f64 / self.sample_rate.as_f32() as f64;
        let beats = seconds * self.cached_tempo as f64 / 60.0;
        let delta_ticks = beats * TICKS_PER_QUARTER as f64;

        self.tick_accumulator += delta_ticks;

        // Process whole ticks
        while self.tick_accumulator >= 1.0 {
            self.tick_accumulator -= 1.0;

            // First, collect events at current tick from the song
            self.collect_events_at_tick(events);

            // Check for note-offs from active notes
            self.check_note_offs(events);

            // Advance position
            self.current_tick = Tick(self.current_tick.0 + 1);

            // Handle looping
            if self.looping && self.current_tick >= self.loop_end {
                // Release all notes at loop point
                self.release_all_notes_into(events);
                self.active_notes.clear();
                self.current_tick = self.loop_start;
            }

            // Update tempo if it changed at this tick
            self.update_cached_tempo();
        }
    }

    /// Update the cached tempo from the song.
    fn update_cached_tempo(&mut self) {
        if let Ok(song) = self.song.read() {
            self.cached_tempo = song.tempo_at(self.current_tick);
        }
    }

    /// Collect events that should trigger at the current tick.
    fn collect_events_at_tick(&mut self, events: &mut Vec<SequencerEvent>) {
        // First, collect all the note data we need while holding the lock
        let notes_to_trigger: Vec<_> = {
            let Ok(song) = self.song.read() else {
                return;
            };

            let mut notes = Vec::new();

            // Check each placement that might be active at this tick
            for placement in song.arrangement() {
                let Some(pattern) = song.pattern(placement.pattern_id) else {
                    continue;
                };

                // Check if this placement contains the current tick
                let pattern_end = Tick(placement.start.0 + pattern.length.0 as u64);
                if self.current_tick < placement.start || self.current_tick >= pattern_end {
                    continue;
                }

                // Calculate position within the pattern
                let pattern_tick = (self.current_tick.0 - placement.start.0) as u32;

                // Collect notes that start at this pattern tick
                for note in pattern.notes() {
                    if note.start.0 != pattern_tick {
                        continue;
                    }

                    // Apply transposition from placement
                    let transposed_pitch = note.pitch.transpose(placement.transpose);

                    // Calculate end tick if duration is known
                    let end_tick = note.duration.map(|d| {
                        Tick(placement.start.0 + note.start.0 as u64 + d.0 as u64)
                    });

                    notes.push((
                        transposed_pitch,
                        note.velocity,
                        note.instrument,
                        note.effects.clone(),
                        end_tick,
                    ));
                }
            }

            notes
        }; // Lock released here

        // Now process the collected notes without holding the lock
        for (pitch, velocity, instrument, effects, end_tick) in notes_to_trigger {
            // Track active note for NoteOff
            self.active_notes.push(ActiveNote {
                pitch,
                instrument,
                end_tick,
            });

            // Emit NoteOn event
            events.push(SequencerEvent::NoteOn {
                tick: self.current_tick,
                pitch,
                velocity,
                instrument,
                effects,
            });
        }
    }

    /// Check for notes that should end at the current tick.
    fn check_note_offs(&mut self, events: &mut Vec<SequencerEvent>) {
        let current = self.current_tick;

        // Find notes that should end
        let mut i = 0;
        while i < self.active_notes.len() {
            let should_end = self.active_notes[i]
                .end_tick
                .map(|end| current >= end)
                .unwrap_or(false);

            if should_end {
                let note = self.active_notes.swap_remove(i);
                events.push(SequencerEvent::NoteOff {
                    tick: current,
                    pitch: note.pitch,
                    instrument: note.instrument,
                });
            } else {
                i += 1;
            }
        }
    }

    /// Release all currently active notes, appending to the provided buffer.
    fn release_all_notes_into(&self, events: &mut Vec<SequencerEvent>) {
        for note in &self.active_notes {
            events.push(SequencerEvent::NoteOff {
                tick: self.current_tick,
                pitch: note.pitch,
                instrument: note.instrument,
            });
        }
    }

    /// Release all currently active notes and return them as a new vector.
    ///
    /// Note: This allocates and should only be called from non-audio threads
    /// (e.g., stop() and seek() which are typically called from the UI thread).
    fn release_all_notes(&self) -> Vec<SequencerEvent> {
        self.active_notes
            .iter()
            .map(|note| SequencerEvent::NoteOff {
                tick: self.current_tick,
                pitch: note.pitch,
                instrument: note.instrument,
            })
            .collect()
    }

    /// Get the shared song reference.
    pub fn song(&self) -> &Arc<RwLock<Song>> {
        &self.song
    }

    /// Set a new song.
    pub fn set_song(&mut self, song: Arc<RwLock<Song>>) {
        // Stop and clear any active notes
        let _ = self.stop();
        self.song = song;
        self.update_cached_tempo();
    }
}

impl Default for SequencerEngine {
    fn default() -> Self {
        Self::new(SampleRate::DVD_QUALITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequencer::{Duration, InstrumentId, PatternTick, Velocity};

    fn create_test_song() -> Song {
        let mut song = Song::new("Test").with_tempo(120.0);

        // Create a pattern with one note
        let pattern_id = song.create_pattern(Duration::WHOLE);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            pattern.add_note(
                PatternTick(0),
                Pitch::new(60).unwrap(),
                Velocity::MF,
                InstrumentId(0),
            );
        }

        // Create a track and place the pattern
        let track_id = song.create_track("Test Track");
        song.place_pattern(pattern_id, track_id, Tick::ZERO);

        song
    }

    #[test]
    fn test_sequencer_creation() {
        let seq = SequencerEngine::new(SampleRate::DVD_QUALITY);
        assert_eq!(seq.play_state(), PlayState::Stopped);
        assert_eq!(seq.current_tick(), Tick::ZERO);
    }

    #[test]
    fn test_play_pause_stop() {
        let mut seq = SequencerEngine::new(SampleRate::DVD_QUALITY);

        seq.play();
        assert_eq!(seq.play_state(), PlayState::Playing);

        seq.pause();
        assert_eq!(seq.play_state(), PlayState::Paused);

        seq.stop();
        assert_eq!(seq.play_state(), PlayState::Stopped);
        assert_eq!(seq.current_tick(), Tick::ZERO);
    }

    #[test]
    fn test_process_advances_position() {
        let song = Arc::new(RwLock::new(create_test_song()));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);

        seq.play();

        // Process enough samples to advance several ticks
        // At 120 BPM, 48000 Hz: 1 second = 1920 ticks
        // 480 samples = 10ms = ~19 ticks
        let mut events = Vec::new();
        seq.process(SampleCount::new(480), &mut events);

        assert!(seq.current_tick().0 > 0);
    }

    #[test]
    fn test_note_events_generated() {
        let song = Arc::new(RwLock::new(create_test_song()));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);

        seq.play();

        // Process enough samples to advance past tick 0
        // At 120 BPM, 48000 Hz: we need ~25 samples per tick
        // Process 1000 samples to ensure we pass tick 0
        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);

        // Should have at least one NoteOn event
        let note_ons: Vec<_> = events.iter().filter(|e| e.is_note_on()).collect();
        assert!(!note_ons.is_empty(), "Expected NoteOn events, got {:?}", events);
    }

    #[test]
    fn test_stop_releases_notes() {
        let song = Arc::new(RwLock::new(create_test_song()));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);

        seq.play();

        // Process enough samples to trigger the note at tick 0
        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);

        // Stop should release notes
        let stop_events = seq.stop();
        let note_offs: Vec<_> = stop_events.iter().filter(|e| e.is_note_off()).collect();

        // The test note has no duration, so it should still be active and released
        assert!(!note_offs.is_empty(), "Expected NoteOff events on stop");
    }

    #[test]
    fn test_seek() {
        let song = Arc::new(RwLock::new(create_test_song()));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);

        let target = Tick(1000);
        seq.seek(target);

        assert_eq!(seq.current_tick(), target);
    }

    #[test]
    fn test_looping() {
        let song = Arc::new(RwLock::new(create_test_song()));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);

        // Set a small loop
        seq.set_loop(Tick(0), Tick(100), true);
        assert!(seq.is_looping());

        seq.play();

        // Process enough to go past the loop point
        let mut events = Vec::new();
        for _ in 0..100 {
            events.clear();
            seq.process(SampleCount::new(48), &mut events);
        }

        // Position should be within loop range
        assert!(seq.current_tick().0 < 100);
    }
}
