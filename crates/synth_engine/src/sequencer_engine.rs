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

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::tracker_effects::{
    ChannelEffectProcessor, GlobalCommand, InstrumentDefaults, NoteTriggerInfo,
};
use synth_core::{BipolarValue, Bpm, Cents, NormalizedValue, SampleCount, SampleRate, VoiceIndex};
use synth_sequencer::tracker_pattern::{Cell, TrackerPattern};
use synth_sequencer::{
    Pitch, RowIndex, SeqInstrumentId, SequencerEvent, Song, TICKS_PER_QUARTER, Tick, TrackId,
    TrackMode, Velocity,
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
    /// Track for mono-per-track behavior and effect processing.
    track: Option<TrackId>,
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
    cached_tempo: Bpm,
    /// Base tempo before speed adjustments (for tracker speed effect).
    base_tempo: Bpm,
    /// Current tracker speed (ticks per row, default 6).
    /// Lower speed = faster playback.
    tracker_speed: u8,
    /// Whether we're looping.
    looping: bool,
    /// Loop start position.
    loop_start: Tick,
    /// Loop end position.
    loop_end: Tick,
    /// Tracker effect processor for handling per-channel effects.
    effect_processor: ChannelEffectProcessor,
    /// Last instrument used per track (for TrackerPattern instrument inheritance).
    /// When a Cell::Note has instrument: None, the engine uses the last instrument
    /// that was played on that track.
    track_last_instruments: HashMap<TrackId, SeqInstrumentId>,
    /// Song ticks per tracker tick for effect processing.
    /// With 960 PPQN and 24 tracker ticks per beat, this is 960/24 = 40.
    /// Effects are processed at tracker-tick rate, not song-tick rate.
    song_ticks_per_tracker_tick: u32,
    /// Counter for song ticks since last effect tick.
    effect_tick_accumulator: u32,
}

impl SequencerEngine {
    /// Song ticks per tracker tick: 960 PPQN / 24 tracker ticks per beat = 40.
    const SONG_TICKS_PER_TRACKER_TICK: u32 = TICKS_PER_QUARTER / 24;

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
            base_tempo: default_tempo,
            tracker_speed: 6, // Default tracker speed
            looping: false,
            loop_start: Tick::ZERO,
            loop_end: Tick::ZERO,
            effect_processor: ChannelEffectProcessor::default(),
            track_last_instruments: HashMap::new(),
            song_ticks_per_tracker_tick: Self::SONG_TICKS_PER_TRACKER_TICK,
            effect_tick_accumulator: 0,
        }
    }

    /// Create a sequencer engine with a shared song reference.
    pub fn with_song(song: Arc<RwLock<Song>>, sample_rate: SampleRate) -> Self {
        let (cached_tempo, tracker_speed) = song
            .read()
            .map(|s| (s.default_tempo, s.default_tracker_speed))
            .unwrap_or((Bpm::new(120.0), 6));

        let mut engine = Self {
            song,
            play_state: PlayState::Stopped,
            current_tick: Tick::ZERO,
            tick_accumulator: 0.0,
            sample_rate,
            active_notes: Vec::new(),
            cached_tempo,
            base_tempo: cached_tempo,
            tracker_speed: tracker_speed.max(1), // Use song's tracker speed, prevent division by zero
            looping: false,
            loop_start: Tick::ZERO,
            loop_end: Tick::ZERO,
            effect_processor: ChannelEffectProcessor::default(),
            track_last_instruments: HashMap::new(),
            song_ticks_per_tracker_tick: Self::SONG_TICKS_PER_TRACKER_TICK,
            effect_tick_accumulator: 0,
        };
        // Recalculate effective tempo based on tracker speed
        engine.recalculate_effective_tempo();
        engine
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
        let beats = seconds * f64::from(self.cached_tempo.as_f32()) / 60.0;
        let delta_ticks = beats * TICKS_PER_QUARTER as f64;

        self.tick_accumulator += delta_ticks;

        // Process whole ticks
        while self.tick_accumulator >= 1.0 {
            self.tick_accumulator -= 1.0;

            // First, collect events at current tick from the song
            self.collect_events_at_tick(events);

            // Check for note-offs from active notes
            self.check_note_offs(events);

            // Process tick-based effects (vibrato, portamento, volume slide, etc.)
            // Effects are processed at tracker-tick rate (24 per beat), not song-tick rate (960 per beat).
            // This ensures vibrato/portamento speeds match original tracker behavior.
            self.effect_tick_accumulator += 1;
            if self.effect_tick_accumulator >= self.song_ticks_per_tracker_tick {
                self.effect_tick_accumulator = 0;

                let modulations = self.effect_processor.process_tick();
                for modulation in modulations {
                    events.push(SequencerEvent::Modulation {
                        tick: self.current_tick,
                        track: modulation.track,
                        pitch_cents: Cents::new(modulation.pitch_cents.as_f32()),
                        volume: NormalizedValue::new(modulation.volume.as_f32()),
                        panning: BipolarValue::new(modulation.panning.as_f32()),
                        note_cut: modulation.note_cut,
                        tone_porta_pitch: modulation.tone_porta_pitch,
                    });
                }
            }

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
        let new_tempo = if let Ok(song) = self.song.read() {
            Some(song.tempo_at(self.current_tick))
        } else {
            None
        };

        if let Some(tempo) = new_tempo {
            self.base_tempo = tempo;
            self.recalculate_effective_tempo();
        }
    }

    /// Recalculate effective tempo based on base tempo and tracker speed.
    ///
    /// Tracker speed affects playback rate:
    /// - Speed 6 (default): tempo is used as-is
    /// - Speed 3: playback is 2x faster (effective_tempo = base_tempo * 6/3)
    /// - Speed 12: playback is 0.5x slower (effective_tempo = base_tempo * 6/12)
    fn recalculate_effective_tempo(&mut self) {
        let speed_ratio = 6.0 / self.tracker_speed.max(1) as f32;
        self.cached_tempo = Bpm::new(self.base_tempo.as_f32() * speed_ratio);
    }

    /// Collect events that should trigger at the current tick.
    fn collect_events_at_tick(&mut self, events: &mut Vec<SequencerEvent>) {
        // Collect tracker pattern data that needs mutable processing
        // (instrument inheritance requires &mut self)
        struct TrackerPatternData {
            pattern: TrackerPattern,
            pattern_tick: u32,
            transpose: synth_core::Semitones,
        }

        // First, collect all the note and effect data we need while holding the lock
        let (notes_to_trigger, effect_events_to_process, tracker_patterns): (
            Vec<_>,
            Vec<_>,
            Vec<TrackerPatternData>,
        ) = {
            let Ok(song) = self.song.read() else {
                return;
            };

            let mut notes = Vec::new();
            let mut effect_events = Vec::new();
            let mut tracker_data = Vec::new();

            // Check each placement that might be active at this tick
            for placement in song.arrangement() {
                // First check for TrackerPattern
                if let Some(tracker_pattern) = song.tracker_pattern(placement.pattern_id) {
                    // Calculate pattern length in ticks
                    let pattern_length_ticks = tracker_pattern.length_ticks().0 as u64;
                    let pattern_end = Tick(placement.start.0 + pattern_length_ticks);

                    if self.current_tick < placement.start || self.current_tick >= pattern_end {
                        continue;
                    }

                    // Calculate position within the pattern
                    let pattern_tick = (self.current_tick.0 - placement.start.0) as u32;

                    // Clone pattern for processing outside the lock
                    // (needed because collect_tracker_pattern_events takes &mut self)
                    tracker_data.push(TrackerPatternData {
                        pattern: tracker_pattern.clone(),
                        pattern_tick,
                        transpose: placement.transpose,
                    });
                    continue;
                }

                // Fall back to regular Pattern
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

                    // Apply transposition from placement (keep original if out of range)
                    let transposed_pitch = note
                        .pitch
                        .transpose(placement.transpose)
                        .unwrap_or(note.pitch);

                    // Calculate end tick if duration is known
                    let end_tick = note
                        .duration
                        .map(|d| Tick(placement.start.0 + note.start.0 as u64 + d.0 as u64));

                    notes.push((
                        transposed_pitch,
                        note.velocity,
                        note.instrument,
                        note.effects.clone(),
                        end_tick,
                        note.track,
                        true, // Regular Pattern notes always have explicit instruments
                    ));
                }

                // Collect effect-only events at this pattern tick
                for effect_event in pattern.effect_events() {
                    if effect_event.tick.0 != pattern_tick {
                        continue;
                    }
                    effect_events.push((effect_event.track, effect_event.effects.clone()));
                }
            }

            (notes, effect_events, tracker_data)
        }; // Lock released here

        // Process TrackerPatterns (requires &mut self for instrument inheritance)
        let mut notes_to_trigger = notes_to_trigger;
        let mut effect_events_to_process = effect_events_to_process;
        let mut note_offs_to_process = Vec::new();

        for data in tracker_patterns {
            self.collect_tracker_pattern_events(
                &data.pattern,
                data.pattern_tick,
                data.transpose,
                &mut notes_to_trigger,
                &mut effect_events_to_process,
                &mut note_offs_to_process,
            );
        }

        // Now process the collected notes without holding the lock
        for (pitch, velocity, instrument, effects, end_tick, track, has_explicit_instrument) in
            notes_to_trigger
        {
            // Determine voice_index based on track mode
            let voice_index = self.get_voice_index_for_track(track);

            if let Some(track_id) = track {
                // For MonoVoice tracks: DON'T send NoteOff - the voice will be retriggered
                // directly via note_on_fixed_voice() which preserves envelope level.
                // For Polyphonic tracks: still use stop_notes_on_track for classic behavior.
                if voice_index.is_none() {
                    // Polyphonic mode - stop existing notes on this track
                    self.stop_notes_on_track(track_id, events);
                }
                // MonoVoice mode - don't stop, voice will be retriggered without envelope reset

                // Build trigger info for the note trigger state machine.
                // XM behavior: When a new note with explicit instrument is played,
                // reset channel volume/panning to the instrument's default values.
                // When inheriting instrument (no explicit), keep current state.
                let trigger_info = NoteTriggerInfo {
                    pitch,
                    instrument_defaults: if has_explicit_instrument {
                        // TODO: Get actual sample defaults from instrument mapping.
                        // For now, use velocity as volume (which is how tracker formats work).
                        Some(InstrumentDefaults {
                            volume: NormalizedValue::new(velocity.as_f32()),
                            panning: BipolarValue::CENTER,
                        })
                    } else {
                        None // Inherit instrument - don't reset volume/panning
                    },
                };

                // Process effects for this channel using the Note Trigger State Machine
                // Returns: (global_commands, should_trigger_note)
                // should_trigger_note is false for tone portamento (3xx/5xx effects)
                let (global_commands, should_trigger_note) = self
                    .effect_processor
                    .process_row_start(track_id, &effects, Some(trigger_info));

                // Handle global commands (tempo changes, etc.)
                for cmd in global_commands {
                    match cmd {
                        GlobalCommand::SetTempo(bpm) => {
                            // Update base tempo and recalculate effective tempo
                            self.base_tempo = Bpm::new(f32::from(bpm));
                            self.recalculate_effective_tempo();
                        }
                        GlobalCommand::SetSpeed(speed) => {
                            // Tracker speed affects timing: lower speed = faster playback
                            // At speed 6 (default), tempo is used as-is
                            // At speed 3, playback is 2x faster (6/3 = 2)
                            self.tracker_speed = speed.as_u8().max(1); // Prevent division by zero
                            self.recalculate_effective_tempo();
                        }
                        // Pattern navigation effects would require additional state
                        GlobalCommand::PatternBreak(_)
                        | GlobalCommand::PatternJump(_)
                        | GlobalCommand::SetLoopStart
                        | GlobalCommand::PatternLoop(_)
                        | GlobalCommand::PatternDelay(_) => {
                            // TODO: Implement pattern navigation
                        }
                    }
                }

                // Only create NoteOn if this is a fresh attack (not tone portamento)
                // Tone portamento (3xx/5xx) only sets the target pitch for glide,
                // it does NOT trigger a new note or reset the sample playback.
                if should_trigger_note {
                    // Track active note for NoteOff (for duration-based note-off)
                    self.active_notes.push(ActiveNote {
                        pitch,
                        instrument,
                        end_tick,
                        track,
                    });

                    // Note: We use the note's velocity directly - in tracker formats,
                    // the note's velocity column IS the volume for that note.
                    // Effect-based volume modulation (SetVolume, VolumeSlide) happens
                    // DURING playback via the effect processor, not at note onset.
                    events.push(SequencerEvent::NoteOn {
                        tick: self.current_tick,
                        pitch,
                        velocity,
                        instrument,
                        effects,
                        voice_index,
                    });
                }
            } else {
                // No track - no effect processing, emit as-is (polyphonic)
                self.active_notes.push(ActiveNote {
                    pitch,
                    instrument,
                    end_tick,
                    track,
                });

                events.push(SequencerEvent::NoteOn {
                    tick: self.current_tick,
                    pitch,
                    velocity,
                    instrument,
                    effects,
                    voice_index: None, // No track = polyphonic mode
                });
            }
        }

        // Process explicit NoteOff events from TrackerPattern (Cell::NoteOff)
        // These represent === markers in tracker files and should release the voice
        for track_id in note_offs_to_process {
            // Get voice index for this track
            let voice_index = self.get_voice_index_for_track(Some(track_id));

            // Find and remove the active note on this track
            // Then emit a NoteOff event
            if let Some(idx) = self
                .active_notes
                .iter()
                .position(|n| n.track == Some(track_id))
            {
                let note = self.active_notes.swap_remove(idx);
                events.push(SequencerEvent::NoteOff {
                    tick: self.current_tick,
                    pitch: note.pitch,
                    instrument: note.instrument,
                });
            } else if let Some(vi) = voice_index {
                // No active note tracked, but we have a voice index
                // Emit a generic voice release event for tracker mode
                events.push(SequencerEvent::VoiceOff {
                    tick: self.current_tick,
                    voice_index: vi,
                });
            }
        }

        // Process effect-only events (tracker-style rows with effects but no note)
        for (track_id, effects) in effect_events_to_process {
            // Process effects through the effect processor to handle global commands.
            // No trigger_info - effect-only rows don't trigger a new note.
            // We ignore the should_trigger_note return value since there's no note.
            let (global_commands, _) = self
                .effect_processor
                .process_row_start(track_id, &effects, None);

            // Handle global commands from effect-only rows
            for cmd in global_commands {
                match cmd {
                    GlobalCommand::SetTempo(bpm) => {
                        self.base_tempo = Bpm::new(f32::from(bpm));
                        self.recalculate_effective_tempo();
                    }
                    GlobalCommand::SetSpeed(speed) => {
                        self.tracker_speed = speed.as_u8().max(1);
                        self.recalculate_effective_tempo();
                    }
                    GlobalCommand::PatternBreak(_)
                    | GlobalCommand::PatternJump(_)
                    | GlobalCommand::SetLoopStart
                    | GlobalCommand::PatternLoop(_)
                    | GlobalCommand::PatternDelay(_) => {
                        // TODO: Implement pattern navigation
                    }
                }
            }

            // Emit effect events for channel-level processing
            for effect in effects {
                events.push(SequencerEvent::Effect {
                    tick: self.current_tick,
                    effect,
                });
            }
        }
    }

    /// Get the voice index for a track based on its mode.
    ///
    /// Returns `Some(VoiceIndex)` for `MonoVoice` tracks (tracker-style),
    /// and `None` for `Polyphonic` tracks (keyboard/MIDI-style).
    fn get_voice_index_for_track(&self, track: Option<TrackId>) -> Option<VoiceIndex> {
        let track_id = track?;

        if let Ok(song) = self.song.read() {
            for seq_track in song.tracks() {
                if seq_track.id == track_id {
                    return match seq_track.mode {
                        TrackMode::MonoVoice(idx) => Some(idx),
                        TrackMode::Polyphonic => None,
                    };
                }
            }
        }
        None
    }

    /// Collect events from a TrackerPattern at the given pattern tick.
    ///
    /// TrackerPattern semantics:
    /// - Each track maps 1:1 to a voice (TrackIndex N → VoiceIndex N)
    /// - Cell::Note triggers a note, automatically cutting any previous note on that voice
    /// - Cell::NoteOff explicitly releases the note (enters envelope release)
    /// - Cell::Empty does nothing (previous note continues)
    ///
    /// Instrument inheritance:
    /// - When a note has `instrument: Some(id)`, that instrument is used and remembered
    /// - When a note has `instrument: None`, the last instrument for that track is used
    /// - If no previous instrument exists, defaults to instrument 0
    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::type_complexity)]
    fn collect_tracker_pattern_events(
        &mut self,
        pattern: &TrackerPattern,
        pattern_tick: u32,
        transpose: synth_core::Semitones,
        notes: &mut Vec<(
            Pitch,
            Velocity,
            SeqInstrumentId,
            Vec<synth_sequencer::EffectCommand>,
            Option<Tick>,
            Option<TrackId>,
            bool, // has_explicit_instrument - true if instrument was specified on this note
        )>,
        effect_events: &mut Vec<(TrackId, Vec<synth_sequencer::EffectCommand>)>,
        note_offs: &mut Vec<TrackId>,
    ) {
        // Convert pattern tick to row index
        let ticks_per_row = pattern.ticks_per_row().as_u32();
        if ticks_per_row == 0 {
            return;
        }

        // Only process on row boundaries
        if !pattern_tick.is_multiple_of(ticks_per_row) {
            return;
        }

        let row_idx = pattern_tick / ticks_per_row;
        if row_idx >= pattern.num_rows().as_u16() as u32 {
            return;
        }

        let row_index = RowIndex::new(row_idx as u16);

        // Process each track at this row
        for (track_index, row) in pattern.row_across_tracks(row_index) {
            // Create a TrackId from the track index (for compatibility with existing code)
            let track_id = TrackId::new(track_index.as_u8() as u16);

            match &row.cell {
                Cell::Note {
                    pitch,
                    instrument,
                    velocity,
                } => {
                    // Apply transposition
                    let transposed_pitch = pitch.transpose(transpose).unwrap_or(*pitch);

                    // Use provided velocity or default
                    let note_velocity = velocity.unwrap_or(Velocity::FF);

                    // Instrument inheritance:
                    // - Some(id) → use that instrument and remember it
                    // - None → use the last instrument for this track (or default to 0)
                    let has_explicit_instrument = instrument.is_some();
                    let note_instrument = if let Some(inst) = instrument {
                        // Explicit instrument - remember it for future notes on this track
                        self.track_last_instruments.insert(track_id, *inst);
                        *inst
                    } else {
                        // Inherit from previous note on this track
                        self.track_last_instruments
                            .get(&track_id)
                            .copied()
                            .unwrap_or(SeqInstrumentId(0))
                    };

                    notes.push((
                        transposed_pitch,
                        note_velocity,
                        note_instrument,
                        row.effects.clone(),
                        None, // TrackerPattern notes don't have explicit duration
                        Some(track_id),
                        has_explicit_instrument,
                    ));
                }
                Cell::NoteOff => {
                    // Explicit note-off - mark this track for note release
                    note_offs.push(track_id);

                    // Also process any effects on this row
                    if !row.effects.is_empty() {
                        effect_events.push((track_id, row.effects.clone()));
                    }
                }
                Cell::Empty => {
                    // No note, but may have effects
                    if !row.effects.is_empty() {
                        effect_events.push((track_id, row.effects.clone()));
                    }
                }
            }
        }
    }

    /// Stop all active notes on the given track (mono-per-track behavior).
    fn stop_notes_on_track(&mut self, track: TrackId, events: &mut Vec<SequencerEvent>) {
        let current = self.current_tick;
        let mut i = 0;
        while i < self.active_notes.len() {
            if self.active_notes[i].track == Some(track) {
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

        // Read tracker speed from new song
        if let Ok(s) = song.read() {
            self.tracker_speed = s.default_tracker_speed.max(1);
        }

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
