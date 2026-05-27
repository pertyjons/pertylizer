//! Sequencer engine for real-time playback.
//!
//! This module provides the runtime engine that converts stored song data
//! into real-time events at audio sample rate precision.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use synth_core::{BipolarValue, Bpm, NormalizedValue, SampleCount, SampleRate};
use synth_sequencer::{
    AutomationTarget, PatternId, PatternTick, Pitch, SeqInstrumentId, SequencerEvent, Song,
    TICKS_PER_QUARTER, Tick, TrackId, TrackParam, Velocity,
};

/// Minimum change threshold for automation value deduplication.
/// Values changing less than this are considered unchanged and won't emit events.
const AUTOMATION_DEDUP_THRESHOLD: f32 = 0.001;

/// Live automation overrides for one track's fader, applied *over* the track's
/// stored (static) volume/pan/mute. A `None` field means "no automation —
/// use the static Song value". Maintained by the sequencer as it collects
/// automation and consumed by `SynthEngine::update_track_controls`.
///
/// Track automation is handled here (not via emitted `SequencerEvent`s)
/// because the override map must be cleared in lock-step with
/// `last_automation_values` at every transport reset — including the
/// loop-wrap / auto-stop branches that run on the audio thread and are
/// invisible to the engine's command layer.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrackAutoOverride {
    /// Automated track volume (normalized 0.0–1.0).
    pub volume: Option<NormalizedValue>,
    /// Automated track pan (bipolar -1.0..1.0).
    pub pan: Option<BipolarValue>,
    /// Automated track mute (`true` = silenced).
    pub muted: Option<bool>,
}

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
    /// Cached song length (end tick of last placement) used to auto-stop
    /// playback when looping is disabled.
    cached_song_length: Tick,
    /// Whether we're looping.
    looping: bool,
    /// Loop start position.
    loop_start: Tick,
    /// Loop end position.
    loop_end: Tick,
    /// Last emitted automation values (for deduplication).
    last_automation_values: HashMap<AutomationTarget, NormalizedValue>,
    /// Live per-track automation overrides, consumed by the channel-bus stage.
    /// Cleared alongside `last_automation_values` at every transport reset.
    track_auto: HashMap<TrackId, TrackAutoOverride>,
    /// Pre-allocated scratch buffer for note collection (avoids per-tick allocation).
    scratch_notes: Vec<(Pitch, Velocity, SeqInstrumentId, Option<Tick>)>,
    /// Pre-allocated scratch buffer for automation collection (avoids per-tick allocation).
    scratch_automation: Vec<(AutomationTarget, NormalizedValue)>,
    /// When set, only emit notes from placements whose pattern id matches.
    /// Used by the piano-roll preview to audition a single pattern in isolation.
    /// Cleared by global `Play`/`Stop` and by the GUI when the piano roll closes.
    solo_pattern: Option<PatternId>,
    /// When set, the engine bypasses arrangement playback entirely and loops
    /// this single pattern (orphan-preview mode). Notes are scheduled at
    /// `current_tick % pattern.length`. Mute/solo and track-instrument routing
    /// do not apply (no track is involved). Used by REC arm on orphan patterns
    /// and by `PlayPattern` when no placement exists. Cleared by global `Play`
    /// (unarmed) and `Stop`.
    preview_pattern: Option<PatternId>,
    /// Instrument that orphan-preview notes play through (no track context
    /// exists in preview mode). Set together with `preview_pattern`.
    preview_instrument: SeqInstrumentId,
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
            active_notes: Vec::with_capacity(64),
            cached_tempo: default_tempo,
            cached_song_length: Tick::ZERO,
            looping: false,
            loop_start: Tick::ZERO,
            loop_end: Tick::ZERO,
            last_automation_values: HashMap::with_capacity(32),
            track_auto: HashMap::with_capacity(32),
            scratch_notes: Vec::with_capacity(64),
            scratch_automation: Vec::with_capacity(16),
            solo_pattern: None,
            preview_pattern: None,
            preview_instrument: SeqInstrumentId(0),
        }
    }

    /// Create a sequencer engine with a shared song reference.
    pub fn with_song(song: Arc<RwLock<Song>>, sample_rate: SampleRate) -> Self {
        let (cached_tempo, cached_song_length) = {
            let s = song.read();
            (s.default_tempo, s.calculate_length())
        };

        Self {
            song,
            play_state: PlayState::Stopped,
            current_tick: Tick::ZERO,
            tick_accumulator: 0.0,
            sample_rate,
            active_notes: Vec::with_capacity(64),
            cached_tempo,
            cached_song_length,
            looping: false,
            loop_start: Tick::ZERO,
            loop_end: Tick::ZERO,
            last_automation_values: HashMap::with_capacity(32),
            track_auto: HashMap::with_capacity(32),
            scratch_notes: Vec::with_capacity(64),
            scratch_automation: Vec::with_capacity(16),
            solo_pattern: None,
            preview_pattern: None,
            preview_instrument: SeqInstrumentId(0),
        }
    }

    /// Set the solo-pattern filter. When `Some(id)`, only notes from
    /// placements whose pattern id matches are emitted; all other patterns
    /// stay silent. Pass `None` to resume normal multi-pattern playback.
    pub fn set_solo_pattern(&mut self, pattern: Option<PatternId>) {
        self.solo_pattern = pattern;
    }

    /// Get the current solo-pattern filter.
    pub fn solo_pattern(&self) -> Option<PatternId> {
        self.solo_pattern
    }

    /// Enable or disable orphan-preview mode for a single pattern. `Some((id,
    /// instrument))` previews the pattern through `instrument` (preview has no
    /// track context); `None` clears preview mode.
    pub fn set_preview_pattern(&mut self, preview: Option<(PatternId, SeqInstrumentId)>) {
        match preview {
            Some((pattern, instrument)) => {
                self.preview_pattern = Some(pattern);
                self.preview_instrument = instrument;
            }
            None => self.preview_pattern = None,
        }
    }

    /// Get the current preview-pattern target, if any.
    pub fn preview_pattern(&self) -> Option<PatternId> {
        self.preview_pattern
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
        self.update_cached_state();
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
        self.track_auto.clear();

        events
    }

    /// Seek to a specific tick position.
    pub fn seek(&mut self, tick: Tick) -> Vec<SequencerEvent> {
        let events = self.release_all_notes();
        self.active_notes.clear();
        self.last_automation_values.clear();
        self.track_auto.clear();
        self.current_tick = tick;
        self.tick_accumulator = 0.0;
        self.update_cached_state();
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

    /// Enable song repeat: sets loop from beginning to song end.
    /// Disabling clears the loop.
    pub fn set_repeat_song(&mut self, enabled: bool) {
        if enabled {
            let song_end = self
                .song
                .try_read()
                .map(|s| s.calculate_length())
                .unwrap_or(Tick::ZERO);
            if song_end > Tick::ZERO {
                self.set_loop(Tick::ZERO, song_end, true);
            }
        } else {
            self.set_loop(Tick::ZERO, Tick::ZERO, false);
        }
    }

    /// Get the loop start position.
    pub fn loop_start(&self) -> Tick {
        self.loop_start
    }

    /// Get the loop end position.
    pub fn loop_end(&self) -> Tick {
        self.loop_end
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
                self.last_automation_values.clear();
                self.track_auto.clear();
                self.current_tick = self.loop_start;
            } else if !self.looping
                && self.cached_song_length > Tick::ZERO
                && self.current_tick >= self.cached_song_length
            {
                // Reached end of arrangement — auto-stop. Mirrors
                // EngineCommand::Stop: release notes, reset to start,
                // transition to Stopped. The audio thread observes the
                // play_state change and clears transport.is_playing +
                // all_notes_off (see SynthEngine::process).
                self.release_all_notes_into(events);
                self.active_notes.clear();
                self.last_automation_values.clear();
                self.track_auto.clear();
                self.play_state = PlayState::Stopped;
                self.current_tick = Tick::ZERO;
                self.tick_accumulator = 0.0;
                break;
            }

            self.update_cached_state();
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

    /// Refresh the cached tempo and song length from the song.
    fn update_cached_state(&mut self) {
        if let Some(song) = self.song.try_read() {
            self.cached_tempo = song.tempo_at(self.current_tick);
            self.cached_song_length = song.calculate_length();
        }
    }

    /// Collect events that should trigger at the current tick.
    fn collect_events_at_tick(&mut self, events: &mut Vec<SequencerEvent>) {
        // Clear scratch buffers (reuses existing heap allocation)
        self.scratch_notes.clear();
        self.scratch_automation.clear();

        // Collect note and automation data while holding the lock.
        //
        // Preview-mode (orphan pattern) bypasses the arrangement entirely
        // and loops a single pattern at `current_tick % pattern.length`.
        if let Some(preview_id) = self.preview_pattern {
            let Some(song) = self.song.try_read() else {
                return;
            };
            if let Some(pattern) = song.pattern(preview_id) {
                let length_ticks = u64::from(pattern.length.0.max(1));
                #[allow(clippy::cast_possible_truncation)]
                let pattern_tick = (self.current_tick.0 % length_ticks) as u32;
                for note in pattern.notes() {
                    if note.start.0 != pattern_tick {
                        continue;
                    }
                    let end_tick = note
                        .duration
                        .map(|d| Tick(self.current_tick.0 + u64::from(d.0)));
                    self.scratch_notes.push((
                        note.pitch,
                        note.velocity,
                        self.preview_instrument,
                        end_tick,
                    ));
                }
                for lane in &pattern.automation {
                    if let Some(value) = lane.value_at(PatternTick(pattern_tick)) {
                        self.scratch_automation.push((lane.target.clone(), value));
                    }
                }
            }
        } else {
            let Some(song) = self.song.try_read() else {
                return;
            };

            let any_solo = song.any_solo();

            for placement in song.arrangement() {
                // Solo-pattern filter (piano-roll preview): drop everything
                // that isn't the chosen pattern. Track mute/solo still
                // applies on top of this so muted tracks remain silent.
                if let Some(solo) = self.solo_pattern
                    && placement.pattern_id != solo
                {
                    continue;
                }

                // Skip muted/non-soloed tracks
                let Some(track) = song.track(placement.track_id) else {
                    continue;
                };
                if !track.is_audible(any_solo) {
                    continue;
                }

                let Some(pattern) = song.pattern(placement.pattern_id) else {
                    continue;
                };

                // length_override gives clip semantics when set.
                let placement_len = placement.length_override.unwrap_or(pattern.length);
                let pattern_end = Tick(placement.start.0 + placement_len.0 as u64);
                if self.current_tick < placement.start || self.current_tick >= pattern_end {
                    continue;
                }

                // Calculate position within the pattern
                #[allow(clippy::cast_possible_truncation)]
                let pattern_tick = (self.current_tick.0 - placement.start.0) as u32;

                // Every track routes to an instrument; the track is the sole
                // source (per-note `note.instrument` is vestigial — Phase 4).
                let track_instrument = track.instrument;

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

                    let effective_instrument = track_instrument;

                    self.scratch_notes.push((
                        transposed_pitch,
                        note.velocity,
                        effective_instrument,
                        end_tick,
                    ));
                }

                // Collect automation values at this tick
                for lane in &pattern.automation {
                    if let Some(value) = lane.value_at(PatternTick(pattern_tick)) {
                        self.scratch_automation.push((lane.target.clone(), value));
                    }
                }
            }
        } // Lock released here

        // Legato across placement boundaries: if a new note shares
        // (pitch, instrument) with an active note ending at this exact tick,
        // extend the active note instead of emitting NoteOff+NoteOn. The
        // extended end_tick keeps `check_note_offs` below from firing the
        // pending NoteOff, so the voice sustains across the boundary.
        for i in 0..self.scratch_notes.len() {
            let (pitch, velocity, instrument, end_tick) = self.scratch_notes[i];

            let extending_idx = self.active_notes.iter().position(|n| {
                n.pitch == pitch
                    && n.instrument == instrument
                    && n.end_tick == Some(self.current_tick)
            });

            if let Some(idx) = extending_idx {
                self.active_notes[idx].end_tick = end_tick;
                continue;
            }

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
        for i in 0..self.scratch_automation.len() {
            let (ref target, value) = self.scratch_automation[i];
            let changed = self.last_automation_values.get(target).is_none_or(|last| {
                (value.as_f32() - last.as_f32()).abs() > AUTOMATION_DEDUP_THRESHOLD
            });

            if changed {
                self.last_automation_values.insert(target.clone(), value);
                if let AutomationTarget::Track { track, param } = target {
                    // Track automation updates the override map directly rather
                    // than emitting an event — the channel-bus stage reads it.
                    let entry = self.track_auto.entry(*track).or_default();
                    match param {
                        TrackParam::Volume => entry.volume = Some(value),
                        TrackParam::Pan => {
                            // Map normalized 0.0-1.0 to bipolar -1.0..1.0.
                            entry.pan = Some(BipolarValue::new(value.as_f32() * 2.0 - 1.0));
                        }
                        TrackParam::Mute => entry.muted = Some(value.as_f32() >= 0.5),
                        TrackParam::Solo => {} // cross-track concept — deferred
                    }
                } else {
                    events.push(SequencerEvent::Parameter {
                        tick: self.current_tick,
                        target: target.clone(),
                        value,
                    });
                }
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

    /// Live per-track automation overrides (volume/pan/mute), to be composed
    /// over the static track fader by the channel-bus stage. Entries are
    /// cleared on every transport reset, so an absent entry means "no live
    /// automation — use the stored track value".
    pub fn track_auto(&self) -> &HashMap<TrackId, TrackAutoOverride> {
        &self.track_auto
    }

    /// Set a new song.
    pub fn set_song(&mut self, song: Arc<RwLock<Song>>) {
        let _ = self.stop();
        self.song = song;
        self.update_cached_state();
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
            let _ = pattern.add_note(
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
    fn test_legato_across_placement_boundary_skips_retrigger() {
        // Two back-to-back placements of a same-pitch sustained note: the
        // sequencer must coalesce the boundary NoteOff+NoteOn into one voice.
        let mut song = Song::new("Legato").with_tempo(Bpm::new(120.0));
        let pattern_len = Duration::WHOLE;
        let pattern_id = song.create_pattern(pattern_len);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            let nid = pattern.add_note(
                PatternTick(0),
                Pitch::new(60).unwrap(),
                Velocity::MF,
                SeqInstrumentId(0),
            );
            if let Some(note) = pattern.note_mut(nid) {
                note.duration = Some(pattern_len);
            }
        }
        let track_id = song.create_track("T");
        song.place_pattern(pattern_id, track_id, Tick::ZERO);
        song.place_pattern(pattern_id, track_id, Tick(pattern_len.0 as u64));

        let song = Arc::new(RwLock::new(song));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);
        seq.play();

        // 5s of audio covers both placements (4s) plus the final NoteOff at
        // tick 7680 — 4s would land it exactly on the trailing edge.
        let mut events = Vec::new();
        let frames = (SampleRate::DVD_QUALITY.as_f32() * 5.0).round() as usize;
        seq.process(SampleCount::new(frames), &mut events);

        let note_ons = events.iter().filter(|e| e.is_note_on()).count();
        let note_offs = events.iter().filter(|e| e.is_note_off()).count();

        assert_eq!(
            note_ons, 1,
            "expected exactly 1 NoteOn (the second placement's NoteOn should be \
             coalesced into the sustaining voice): {events:?}"
        );
        assert_eq!(
            note_offs, 1,
            "expected exactly 1 NoteOff (only at the final note end, not at the \
             placement boundary): {events:?}"
        );
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
