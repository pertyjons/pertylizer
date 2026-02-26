//! Sequencer engine for real-time playback.
//!
//! This module provides the runtime engine that converts stored song data
//! into real-time events at audio sample rate precision.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use synth_core::{Bpm, SampleCount, SampleRate};
use synth_sequencer::{
    AutomationTarget, PatternTick, Pitch, SeqInstrumentId, SequencerEvent, Song, TICKS_PER_QUARTER,
    Tick,
};

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
    /// The sequencer instrument playing the note.
    instrument: SeqInstrumentId,
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
    tick_accumulator: f64,
    /// Current sample rate.
    sample_rate: SampleRate,
    /// Active notes that need NoteOff events.
    active_notes: Vec<ActiveNote>,
    /// Cached tempo (BPM) to avoid locking song every sample.
    cached_tempo: Bpm,
    /// Whether we're looping.
    looping: bool,
    /// Loop start position.
    loop_start: Tick,
    /// Loop end position.
    loop_end: Tick,
    /// Last emitted automation values (for deduplication).
    last_automation_values: HashMap<AutomationTarget, f32>,
}

impl SequencerEngine {
    /// Create a new sequencer engine with the given sample rate.
    pub fn new(sample_rate: SampleRate) -> Self {
        let default_tempo = Bpm::new(120.0);
        Self {
            song: Arc::new(RwLock::new(Song::default())),
            play_state: PlayState::Stopped,
            current_tick: Tick::ZERO,
            tick_accumulator: 0.0,
            sample_rate,
            active_notes: Vec::new(),
            cached_tempo: default_tempo,
            looping: false,
            loop_start: Tick::ZERO,
            loop_end: Tick::ZERO,
            last_automation_values: HashMap::new(),
        }
    }

    /// Create a sequencer engine with a shared song reference.
    pub fn with_song(song: Arc<RwLock<Song>>, sample_rate: SampleRate) -> Self {
        let cached_tempo = song
            .read()
            .map(|s| s.default_tempo)
            .unwrap_or(Bpm::new(120.0));

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
            last_automation_values: HashMap::new(),
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
        self.last_automation_values.clear();

        events
    }

    /// Seek to a specific tick position.
    pub fn seek(&mut self, tick: Tick) -> Vec<SequencerEvent> {
        let events = self.release_all_notes();
        self.active_notes.clear();
        self.last_automation_values.clear();
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

    /// Process samples up to the next song tick boundary.
    ///
    /// Returns the number of samples consumed (the "chunk size").
    pub fn process_until_next_tick(
        &mut self,
        remaining_samples: usize,
        events: &mut Vec<SequencerEvent>,
    ) -> usize {
        if self.play_state != PlayState::Playing || remaining_samples == 0 {
            return 0;
        }

        let bpm = f64::from(self.cached_tempo.as_f32());
        if bpm <= 0.0 {
            return remaining_samples;
        }
        let samples_per_tick =
            f64::from(self.sample_rate.as_f32()) * 60.0 / (bpm * TICKS_PER_QUARTER as f64);

        // How many samples until the next whole tick?
        let samples_to_next_tick = (1.0 - self.tick_accumulator) * samples_per_tick;
        let chunk = (samples_to_next_tick.ceil() as usize)
            .max(1)
            .min(remaining_samples);

        // Advance the accumulator by the chunk's worth of ticks
        let seconds = chunk as f64 / f64::from(self.sample_rate.as_f32());
        let beats = seconds * bpm / 60.0;
        let delta_ticks = beats * TICKS_PER_QUARTER as f64;
        self.tick_accumulator += delta_ticks;

        // Process whole ticks (usually 0 or 1)
        while self.tick_accumulator >= 1.0 {
            self.tick_accumulator -= 1.0;
            self.collect_events_at_tick(events);
            self.check_note_offs(events);

            self.current_tick = Tick(self.current_tick.0 + 1);

            if self.looping && self.current_tick >= self.loop_end {
                self.release_all_notes_into(events);
                self.active_notes.clear();
                self.current_tick = self.loop_start;
            }

            self.update_cached_tempo();
        }

        chunk
    }

    /// Process a buffer of samples and append generated events.
    pub fn process(&mut self, samples: SampleCount, events: &mut Vec<SequencerEvent>) {
        let mut remaining = samples.as_usize();
        while remaining > 0 {
            let consumed = self.process_until_next_tick(remaining, events);
            if consumed == 0 {
                break;
            }
            remaining = remaining.saturating_sub(consumed);
        }
    }

    /// Update the cached tempo from the song.
    fn update_cached_tempo(&mut self) {
        if let Ok(song) = self.song.try_read() {
            self.cached_tempo = song.tempo_at(self.current_tick);
        }
    }

    /// Collect events that should trigger at the current tick.
    fn collect_events_at_tick(&mut self, events: &mut Vec<SequencerEvent>) {
        // Collect note and automation data while holding the lock
        let (notes_to_trigger, auto_events): (Vec<_>, Vec<_>) = {
            let Ok(song) = self.song.try_read() else {
                return;
            };

            let mut notes = Vec::new();
            let mut auto_vals = Vec::new();

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
                #[allow(clippy::cast_possible_truncation)]
                let pattern_tick = (self.current_tick.0 - placement.start.0) as u32;

                // Collect notes that start at this pattern tick
                for note in pattern.notes() {
                    if note.start.0 != pattern_tick {
                        continue;
                    }

                    let transposed_pitch = note
                        .pitch
                        .transpose(placement.transpose)
                        .unwrap_or(note.pitch);

                    let end_tick = note
                        .duration
                        .map(|d| Tick(placement.start.0 + note.start.0 as u64 + d.0 as u64));

                    notes.push((transposed_pitch, note.velocity, note.instrument, end_tick));
                }

                // Collect automation values at this tick
                for lane in &pattern.automation {
                    if let Some(value) = lane.value_at(PatternTick(pattern_tick)) {
                        auto_vals.push((lane.target.clone(), value));
                    }
                }
            }

            (notes, auto_vals)
        }; // Lock released here

        // Now process the collected notes without holding the lock
        for (pitch, velocity, instrument, end_tick) in notes_to_trigger {
            self.active_notes.push(ActiveNote {
                pitch,
                instrument,
                end_tick,
            });

            events.push(SequencerEvent::NoteOn {
                tick: self.current_tick,
                pitch,
                velocity,
                instrument,
            });
        }

        // Emit automation parameter events (deduplicated)
        for (target, value) in auto_events {
            let changed = self
                .last_automation_values
                .get(&target)
                .is_none_or(|last| (value - last).abs() > 0.001);

            if changed {
                self.last_automation_values.insert(target.clone(), value);
                events.push(SequencerEvent::Parameter {
                    tick: self.current_tick,
                    target,
                    value,
                });
            }
        }
    }

    /// Check for notes that should end at the current tick.
    fn check_note_offs(&mut self, events: &mut Vec<SequencerEvent>) {
        let current = self.current_tick;

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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use synth_sequencer::{Duration, PatternTick, SeqInstrumentId, Velocity};

    fn create_test_song() -> Song {
        let mut song = Song::new("Test").with_tempo(Bpm::new(120.0));

        // Create a pattern with one note
        let pattern_id = song.create_pattern(Duration::WHOLE);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            pattern.add_note(
                PatternTick(0),
                Pitch::new(60).unwrap(),
                Velocity::MF,
                SeqInstrumentId(0),
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

        let mut events = Vec::new();
        seq.process(SampleCount::new(480), &mut events);

        assert!(seq.current_tick().0 > 0);
    }

    #[test]
    fn test_note_events_generated() {
        let song = Arc::new(RwLock::new(create_test_song()));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);

        seq.play();

        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);

        let note_ons: Vec<_> = events.iter().filter(|e| e.is_note_on()).collect();
        assert!(
            !note_ons.is_empty(),
            "Expected NoteOn events, got {:?}",
            events
        );
    }

    #[test]
    fn test_stop_releases_notes() {
        let song = Arc::new(RwLock::new(create_test_song()));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);

        seq.play();

        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);

        let stop_events = seq.stop();
        let note_offs: Vec<_> = stop_events.iter().filter(|e| e.is_note_off()).collect();

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

        seq.set_loop(Tick(0), Tick(100), true);
        assert!(seq.is_looping());

        seq.play();

        let mut events = Vec::new();
        for _ in 0..100 {
            events.clear();
            seq.process(SampleCount::new(48), &mut events);
        }

        assert!(seq.current_tick().0 < 100);
    }
}
