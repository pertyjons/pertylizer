//! Sequencer engine for real-time playback.
//!
//! This module provides the runtime engine that converts stored song data
//! into real-time events at audio sample rate precision.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use synth_core::{BipolarValue, Bpm, NormalizedValue, SampleCount, SampleRate, Semitones};
use synth_sequencer::{
    AutomationTarget, ExpandedNote, ExpansionBuffer, Glide, GlideFrom, NoteExpression, PatternId,
    PatternTick, Pitch, SeqInstrumentId, SequencerEvent, Song, TICKS_PER_QUARTER, Tick, TrackId,
    TrackParam, Velocity,
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

/// A note collected at the current tick, before NoteOn emission.
///
/// `Copy`/alloc-free so it lives in the pre-allocated `scratch_notes` buffer
/// without per-tick heap churn. Carries the per-note expression (`legato`,
/// `glide`) so it can ride the emitted `SequencerEvent::NoteOn`.
#[derive(Debug, Clone, Copy)]
struct PendingNote {
    pitch: Pitch,
    velocity: Velocity,
    instrument: SeqInstrumentId,
    end_tick: Option<Tick>,
    /// Per-note tie/legato intent (taxonomy primitive 2).
    legato: bool,
    /// Per-note glide; any absolute `GlideFrom::Pitch` is already transposed to
    /// the placement key.
    glide: Option<Glide>,
    /// Per-note expression block (vibrato + note-shape scalars). Probability has
    /// already been resolved before this note was collected.
    expression: Option<NoteExpression>,
}

/// Golden-ratio odd constant — the SplitMix64 increment (same role as the
/// mixers in `synth_modules::math`; kept local because this is a stateless
/// sequencer-side hash, not the streaming `xorshift32` RNG).
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// Deterministic pseudo-random value in `[0, 1)` from a 64-bit seed.
///
/// SplitMix64 finalizer — pure arithmetic, RT-safe (no RNG state, no syscall,
/// no allocation), and reproducible for a given seed. Used to resolve per-note
/// trigger probability at emission so the audio thread never calls an RNG (the
/// project's no-`Math.random` posture). The finalizer mixes structured input
/// thoroughly, so callers may feed a plain xor of their key fields.
#[allow(clippy::cast_precision_loss)]
fn deterministic_unit(seed: u64) -> f32 {
    let mut z = seed.wrapping_add(SPLITMIX_GAMMA);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // Top 24 bits → [0, 1); 2^24 = 16_777_216 (exactly representable in f32).
    ((z >> 40) as f32) / 16_777_216.0
}

/// Ghost-note velocity scale (forced-soft articulation).
const GHOST_VELOCITY_SCALE: f32 = 0.4;

/// Apply the note-shape velocity scalars to a base velocity: `accent` (a
/// multiplier, >1 louder) and `ghost` (forced soft). Resolved sequencer-side so
/// the emitted `NoteOn` already carries the final velocity — the engine consumes
/// only vibrato from the expression block.
fn shaped_velocity(base: Velocity, expr: Option<NoteExpression>) -> Velocity {
    let Some(e) = expr else {
        return base;
    };
    let mut factor = e.accent.unwrap_or(1.0);
    if e.ghost {
        factor *= GHOST_VELOCITY_SCALE;
    }
    // Defensive: a non-finite accent (only reachable via a programmatic/MCP write,
    // never normal JSON) must not propagate NaN into the audio path — `Velocity::new`
    // clamp does not sanitize NaN.
    if !factor.is_finite() || (factor - 1.0).abs() < f32::EPSILON {
        return base;
    }
    Velocity::new(base.0 * factor)
}

/// Apply the `gate` note-shape scalar to a duration in ticks (staccato/tenuto).
/// `None` → unchanged. Clamps to ≥1 tick so the note keeps a non-zero end_tick:
/// a zero-length note (end == onset) would be eligible for an immediate same-tick
/// NoteOff in `check_note_offs`, which is an articulation we never want here.
///
/// `gate` is **ignored on a legato note** (`legato == true`): a tie wants its
/// full duration so the placement-boundary coalesce can sustain across it —
/// shortening it would silently sever the legato join. This keeps the three
/// note-shape scalars consistent under legato (accent/ghost apply via the
/// glided voice's velocity; gate defers to the tie).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn shaped_duration_ticks(duration_ticks: u32, expr: Option<NoteExpression>, legato: bool) -> u32 {
    match expr.and_then(|e| e.gate) {
        Some(gate) if !legato => ((duration_ticks as f32) * gate.as_f32()).round().max(1.0) as u32,
        _ => duration_ticks,
    }
}

/// Shape one expanded note into the engine-side pending note — the single
/// post-expansion path shared by the preview and arrangement branches so note
/// shaping can never diverge between audition and playback. Applies the
/// placement transpose (zero in preview) to the pitch and to any absolute
/// glide source (a relative-semitone glide is key-independent and passes
/// through; an absolute source that would leave MIDI range under transpose
/// drops the glide rather than emit a desynced source/target pair), the
/// gate/legato duration shaping anchored at `start_tick` (the absolute song
/// tick of the note's onset), and the accent/ghost velocity shaping.
fn make_pending_note(
    expanded: ExpandedNote,
    instrument: SeqInstrumentId,
    transpose: Semitones,
    start_tick: u64,
) -> PendingNote {
    let pitch = expanded
        .pitch
        .transpose(transpose)
        .unwrap_or(expanded.pitch);
    let end_tick = expanded.duration.map(|d| {
        Tick(
            start_tick
                + u64::from(shaped_duration_ticks(
                    d.0,
                    expanded.expression,
                    expanded.legato,
                )),
        )
    });
    let glide = expanded.glide.and_then(|g| match g.from {
        GlideFrom::Pitch(p) => p.transpose(transpose).map(|tp| Glide {
            from: GlideFrom::Pitch(tp),
            ..g
        }),
        GlideFrom::Semitones(_) => Some(g),
    });
    PendingNote {
        pitch,
        velocity: shaped_velocity(expanded.velocity, expanded.expression),
        instrument,
        end_tick,
        legato: expanded.legato,
        glide,
        expression: expanded.expression,
    }
}

/// Decide whether a note plays this occurrence, given its per-note probability.
///
/// `None` probability → always plays. The roll is seeded by the song tick, the
/// note id, and a `roll_nonce` that increments on every loop-wrap. The nonce is
/// what makes a *looped* section re-roll each pass — `current_tick` resets to
/// `loop_start` on wrap, so without it the same note would roll identically
/// every loop. The nonce resets with the transport, so playback from a given
/// start is still fully reproducible. The three fields are rotated apart before
/// the SplitMix finalizer decorrelates them.
fn note_passes_probability(note: &synth_sequencer::Note, tick: Tick, roll_nonce: u64) -> bool {
    match note.expression.and_then(|e| e.probability) {
        None => true,
        Some(prob) => {
            let seed = tick.0 ^ note.id.0.rotate_left(32) ^ roll_nonce.rotate_left(17);
            deterministic_unit(seed) < prob.as_f32()
        }
    }
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
    /// Cursor: the play-start / return position. `play()` from a stopped
    /// transport starts here, and `stop()` returns the playhead here (rather
    /// than always to 0). Set by `set_cursor` (driven by ruler clicks /
    /// `EngineCommand::Seek`). A second `stop()` once the playhead already
    /// rests on the cursor rewinds the cursor and playhead to the song start.
    cursor_tick: Tick,
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
    scratch_notes: Vec<PendingNote>,
    /// Pre-allocated scratch buffer for automation collection (avoids per-tick allocation).
    scratch_automation: Vec<(AutomationTarget, NormalizedValue)>,
    /// Pre-allocated bounded buffer for note-processor expansion (Model B).
    /// Every pattern's per-tick notes flow through this, rack or not, so there
    /// is a single collection path. Hard-capped; see the overflow policy on
    /// `synth_sequencer::ExpansionBuffer`.
    scratch_expansion: ExpansionBuffer,
    /// Total expanded note events dropped by the overflow cap since the last
    /// transport reset. The audio thread only counts (it must not log);
    /// diagnostics layers read this via `expansion_drops()`.
    expansion_drops: u64,
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
    /// Per-note-probability roll nonce. Increments on every loop-wrap so a
    /// looped section re-rolls each pass; resets to 0 with the transport so
    /// playback from a given start is reproducible.
    roll_nonce: u64,
}

impl SequencerEngine {
    /// Create a new sequencer engine with the given sample rate.
    pub fn new(sample_rate: SampleRate) -> Self {
        let default_tempo = Bpm::new(120.0);
        Self {
            song: Arc::new(RwLock::new(Song::default())),
            play_state: PlayState::Stopped,
            current_tick: Tick::ZERO,
            cursor_tick: Tick::ZERO,
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
            scratch_notes: Vec::with_capacity(synth_sequencer::MAX_EXPANSION_EVENTS_PER_TICK),
            scratch_automation: Vec::with_capacity(16),
            scratch_expansion: ExpansionBuffer::new(),
            expansion_drops: 0,
            solo_pattern: None,
            preview_pattern: None,
            preview_instrument: SeqInstrumentId(0),
            roll_nonce: 0,
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
            cursor_tick: Tick::ZERO,
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
            scratch_notes: Vec::with_capacity(synth_sequencer::MAX_EXPANSION_EVENTS_PER_TICK),
            scratch_automation: Vec::with_capacity(16),
            scratch_expansion: ExpansionBuffer::new(),
            expansion_drops: 0,
            solo_pattern: None,
            preview_pattern: None,
            preview_instrument: SeqInstrumentId(0),
            roll_nonce: 0,
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

    /// Get the cursor (play-start / return) position.
    pub fn cursor_tick(&self) -> Tick {
        self.cursor_tick
    }

    /// Set the cursor (play-start / return) position. This is where `play()`
    /// starts from when stopped and where `stop()` returns the playhead.
    pub fn set_cursor(&mut self, tick: Tick) {
        self.cursor_tick = tick;
    }

    /// Start playback. From a stopped transport, playback begins at the cursor
    /// (the marked start position); from a paused transport it resumes in place.
    pub fn play(&mut self) {
        if self.play_state == PlayState::Stopped {
            self.current_tick = self.cursor_tick;
            self.tick_accumulator = 0.0;
            self.roll_nonce = 0;
        }
        self.play_state = PlayState::Playing;
        self.update_cached_state();
    }

    /// Pause playback at current position.
    pub fn pause(&mut self) {
        self.play_state = PlayState::Paused;
    }

    /// Stop playback and return the playhead to the cursor. Pressing stop a
    /// second time once the playhead already rests on the cursor rewinds both
    /// the cursor and the playhead to the song start.
    pub fn stop(&mut self) -> Vec<SequencerEvent> {
        let was_stopped = self.play_state == PlayState::Stopped;
        let at_cursor = self.current_tick == self.cursor_tick;
        self.play_state = PlayState::Stopped;
        if was_stopped && at_cursor {
            // Second stop at the cursor → rewind to the song start.
            self.cursor_tick = Tick::ZERO;
            self.current_tick = Tick::ZERO;
        } else {
            // Return the playhead to the marked start position.
            self.current_tick = self.cursor_tick;
        }
        self.tick_accumulator = 0.0;
        self.roll_nonce = 0;
        self.expansion_drops = 0;

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
        // Reset the probability roll nonce so a seek-then-play is reproducible.
        self.roll_nonce = 0;
        self.expansion_drops = 0;
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
                // Advance the probability roll nonce so a looped section re-rolls
                // each pass (current_tick resets here, so it can't supply variation).
                self.roll_nonce = self.roll_nonce.wrapping_add(1);
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
                self.current_tick = self.cursor_tick;
                self.tick_accumulator = 0.0;
                self.roll_nonce = 0;
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
                // NB: preview/audition deliberately does NOT apply per-note
                // probability (gate `|_| true`) — auditioning a pattern should
                // always sound the note so the user can hear/edit it;
                // probability is a performance feature, applied only in
                // arrangement playback. The processor rack DOES apply, so the
                // preview sounds like playback will.
                pattern.expand_at_tick(
                    PatternTick(pattern_tick),
                    |_| true,
                    &mut self.scratch_expansion,
                );
                self.expansion_drops += u64::from(self.scratch_expansion.dropped());
                // Preview has no placement, so transpose is zero (a no-op for
                // pitch and glide alike).
                for i in 0..self.scratch_expansion.notes().len() {
                    let expanded = self.scratch_expansion.notes()[i];
                    self.scratch_notes.push(make_pending_note(
                        expanded,
                        self.preview_instrument,
                        Semitones::new(0.0),
                        self.current_tick.0,
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

                // Collect (and Model-B expand) notes that start at this pattern
                // tick. The gate resolves per-note trigger probability here
                // (sequencer-side, deterministic), so a losing roll simply omits
                // the note — the audio thread never runs an RNG. Absolute song
                // tick seeds it, so a looped section varies roll-to-roll yet
                // stays reproducible. The pattern's processor rack then runs in
                // pattern space (placement transpose applies after).
                let current_tick = self.current_tick;
                let roll_nonce = self.roll_nonce;
                pattern.expand_at_tick(
                    PatternTick(pattern_tick),
                    |note| note_passes_probability(note, current_tick, roll_nonce),
                    &mut self.scratch_expansion,
                );
                self.expansion_drops += u64::from(self.scratch_expansion.dropped());

                for i in 0..self.scratch_expansion.notes().len() {
                    let expanded = self.scratch_expansion.notes()[i];
                    self.scratch_notes.push(make_pending_note(
                        expanded,
                        track_instrument,
                        placement.transpose,
                        placement.start.0 + u64::from(pattern_tick),
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
        //
        // The per-note `legato`/`glide` fields are *carried* here but not yet
        // acted upon: Phase B3 generalises this same-pitch coalesce into a
        // per-note-legato superset (suppress NoteOff+NoteOn across *any*
        // successor, gliding when the pitch differs) and drives the engine's
        // GlideState. For now they only ride the emitted event.
        for i in 0..self.scratch_notes.len() {
            let PendingNote {
                pitch,
                velocity,
                instrument,
                end_tick,
                legato,
                glide,
                expression,
            } = self.scratch_notes[i];

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
                legato,
                glide,
                expression,
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

    /// Note-processor expansion events dropped by the per-tick overflow cap
    /// since the last transport reset (stop/seek). The audio thread only
    /// counts; surfacing/logging is the diagnostics layer's job.
    pub fn expansion_drops(&self) -> u64 {
        self.expansion_drops
    }

    /// Set a new song.
    pub fn set_song(&mut self, song: Arc<RwLock<Song>>) {
        let _ = self.stop();
        // A new song is unrelated to the old one's timeline: reset both the
        // playhead and the cursor (play-start / return position) to the start so
        // Play does not resume at a stale position carried over from the
        // previous song (e.g. when the swap happens mid-playback).
        self.current_tick = Tick::ZERO;
        self.cursor_tick = Tick::ZERO;
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

    #[test]
    fn deterministic_unit_is_in_range_and_reproducible() {
        for seed in [0u64, 1, 42, 1_000_000, u64::MAX, 0x9E37_79B9_7F4A_7C15] {
            let v = deterministic_unit(seed);
            assert!((0.0..1.0).contains(&v), "seed {seed} → {v} out of [0,1)");
            assert_eq!(v, deterministic_unit(seed), "must be reproducible");
        }
    }

    #[test]
    fn shaped_velocity_accent_ghost_and_absent() {
        use synth_sequencer::NoteExpression;
        let base = Velocity::new(0.5);
        let expr = |accent: Option<f32>, ghost: bool| NoteExpression {
            vibrato: None,
            accent,
            gate: None,
            ghost,
            probability: None,
        };
        // No expression → unchanged.
        assert_eq!(shaped_velocity(base, None), base);
        // Accent 1.5× → 0.75.
        assert!((shaped_velocity(base, Some(expr(Some(1.5), false))).0 - 0.75).abs() < 1e-6);
        // Ghost → 0.5 * 0.4 = 0.2.
        assert!((shaped_velocity(base, Some(expr(None, true))).0 - 0.2).abs() < 1e-6);
        // Accent + ghost compose: 0.5 * 2.0 * 0.4 = 0.4.
        assert!((shaped_velocity(base, Some(expr(Some(2.0), true))).0 - 0.4).abs() < 1e-6);
        // Accent that overshoots clamps to 1.0.
        assert!((shaped_velocity(base, Some(expr(Some(10.0), false))).0 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn shaped_duration_gate_and_floor() {
        use synth_sequencer::NoteExpression;
        let expr = |gate: Option<f32>| {
            gate.map(|g| NoteExpression {
                vibrato: None,
                accent: None,
                gate: Some(synth_core::NormalizedValue::new(g)),
                ghost: false,
                probability: None,
            })
        };
        // No gate → unchanged.
        assert_eq!(shaped_duration_ticks(960, None, false), 960);
        assert_eq!(shaped_duration_ticks(960, expr(None), false), 960);
        // Half gate → 480.
        assert_eq!(shaped_duration_ticks(960, expr(Some(0.5)), false), 480);
        // Zero gate → clamped to 1 tick (never coincident NoteOff/NoteOn).
        assert_eq!(shaped_duration_ticks(960, expr(Some(0.0)), false), 1);
        // Legato ignores gate (the tie keeps full duration).
        assert_eq!(shaped_duration_ticks(960, expr(Some(0.25)), true), 960);
    }

    #[test]
    fn probability_gate_endpoints_and_absent() {
        use synth_sequencer::{Note, NoteExpression, NoteId};
        let mk = |prob: Option<f32>| {
            let mut n = Note::new(
                NoteId(7),
                PatternTick(0),
                Pitch::new(60).unwrap(),
                Velocity::MF,
            );
            if let Some(p) = prob {
                n = n.with_expression(NoteExpression {
                    vibrato: None,
                    accent: None,
                    gate: None,
                    ghost: false,
                    probability: Some(synth_core::NormalizedValue::new(p)),
                });
            }
            n
        };
        // No expression / no probability → always plays.
        assert!(note_passes_probability(&mk(None), Tick(0), 0));
        // p = 1.0 → always plays; p = 0.0 → never plays (for any tick + nonce).
        for t in [0u64, 123, 99_999] {
            assert!(note_passes_probability(&mk(Some(1.0)), Tick(t), 0));
            assert!(!note_passes_probability(&mk(Some(0.0)), Tick(t), 7));
        }
        // The roll nonce varies the result at a FIXED tick (the looped-section
        // case, where current_tick resets every pass) — so a p=0.5 note does not
        // play-or-skip identically forever. Assert both outcomes occur across nonces.
        let half = mk(Some(0.5));
        let rolls: Vec<bool> = (0..32)
            .map(|nonce| note_passes_probability(&half, Tick(480), nonce))
            .collect();
        assert!(
            rolls.iter().any(|&b| b) && rolls.iter().any(|&b| !b),
            "nonce must vary the roll across loop passes: {rolls:?}"
        );
    }

    /// A song with one note in a pattern that is NOT placed in the arrangement
    /// (an "orphan" pattern). Returns the song + the orphan pattern id.
    fn create_orphan_song() -> (Song, PatternId) {
        let mut song = Song::new("Orphan").with_tempo(Bpm::new(120.0));
        let pattern_id = song.create_pattern(Duration::WHOLE);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            let _ = pattern.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
        }
        // Deliberately no create_track / place_pattern — the pattern is orphan.
        (song, pattern_id)
    }

    fn create_test_song() -> Song {
        let mut song = Song::new("Test").with_tempo(Bpm::new(120.0));

        // Create a pattern with one note
        let pattern_id = song.create_pattern(Duration::WHOLE);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            let _ = pattern.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
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
    fn test_cursor_play_stop_semantics() {
        let song = Arc::new(RwLock::new(create_test_song()));
        let mut seq = SequencerEngine::with_song(song, SampleRate::DVD_QUALITY);

        // Mark a cursor, then play: playback starts at the cursor, not 0.
        let cursor = Tick(1920);
        seq.set_cursor(cursor);
        seq.play();
        assert_eq!(seq.current_tick(), cursor, "play starts at the cursor");

        // Advance, then pause: the playhead holds where it is.
        let mut events = Vec::new();
        seq.process(SampleCount::new(5000), &mut events);
        let paused_at = seq.current_tick();
        assert!(paused_at > cursor, "playhead advanced past the cursor");
        seq.pause();
        assert_eq!(seq.current_tick(), paused_at, "pause holds position");

        // Resume from a pause continues in place (not from the cursor).
        seq.play();
        assert_eq!(seq.current_tick(), paused_at, "play resumes in place");

        // Stop returns the playhead to the cursor.
        let _ = seq.stop();
        assert_eq!(seq.current_tick(), cursor, "stop returns to the cursor");

        // A second stop at the cursor rewinds cursor and playhead to the start.
        let _ = seq.stop();
        assert_eq!(
            seq.current_tick(),
            Tick::ZERO,
            "second stop rewinds to start"
        );
        assert_eq!(
            seq.cursor_tick(),
            Tick::ZERO,
            "second stop clears the cursor"
        );
    }

    #[test]
    fn test_legato_across_placement_boundary_skips_retrigger() {
        // Two back-to-back placements of a same-pitch sustained note: the
        // sequencer must coalesce the boundary NoteOff+NoteOn into one voice.
        let mut song = Song::new("Legato").with_tempo(Bpm::new(120.0));
        let pattern_len = Duration::WHOLE;
        let pattern_id = song.create_pattern(pattern_len);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            let nid = pattern.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
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

    // --- Orphan-preview mode (channel-strip plan §5) -----------------------
    //
    // These pin the orphan-preview branch (auditioning an unplaced pattern),
    // which had a history of lifecycle bugs (v0.290.0) and no test coverage.
    // The scratch-track rework was assessed and closed as not-worth-it; this
    // hardens the mechanism that was kept.

    #[test]
    fn orphan_preview_emits_notes_through_preview_instrument() {
        let (song, pattern_id) = create_orphan_song();
        let mut seq =
            SequencerEngine::with_song(Arc::new(RwLock::new(song)), SampleRate::DVD_QUALITY);

        let preview_inst = SeqInstrumentId(7);
        seq.set_preview_pattern(Some((pattern_id, preview_inst)));
        seq.play();

        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);

        let note_ons: Vec<_> = events.iter().filter(|e| e.is_note_on()).collect();
        assert!(
            !note_ons.is_empty(),
            "orphan preview should emit NoteOn for the pattern's notes: {events:?}"
        );
        // No track context exists in preview — notes must route through the
        // explicitly supplied preview instrument.
        assert!(
            note_ons
                .iter()
                .all(|e| e.instrument() == Some(preview_inst)),
            "preview NoteOns must use the preview instrument {preview_inst:?}: {events:?}"
        );
    }

    #[test]
    fn orphan_pattern_is_silent_without_preview() {
        // Same orphan pattern, but preview not enabled: with no arrangement
        // placement the normal loop emits nothing.
        let (song, _pattern_id) = create_orphan_song();
        let mut seq =
            SequencerEngine::with_song(Arc::new(RwLock::new(song)), SampleRate::DVD_QUALITY);
        seq.play();

        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);

        assert!(
            events.iter().all(|e| !e.is_note_on()),
            "an unplaced pattern must be silent unless previewed: {events:?}"
        );
    }

    #[test]
    fn clearing_preview_silences_the_orphan_pattern() {
        let (song, pattern_id) = create_orphan_song();
        let mut seq =
            SequencerEngine::with_song(Arc::new(RwLock::new(song)), SampleRate::DVD_QUALITY);

        seq.set_preview_pattern(Some((pattern_id, SeqInstrumentId(3))));
        assert_eq!(seq.preview_pattern(), Some(pattern_id));

        seq.set_preview_pattern(None);
        assert_eq!(seq.preview_pattern(), None);

        seq.play();
        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);
        assert!(
            events.iter().all(|e| !e.is_note_on()),
            "after clearing preview the orphan pattern must be silent: {events:?}"
        );
    }

    #[test]
    fn zero_length_preview_pattern_does_not_panic() {
        // Regression guard for the zero-length orphan (v0.290.0): the modulo in
        // the preview branch must not divide by zero.
        let mut song = Song::new("ZeroLen").with_tempo(Bpm::new(120.0));
        let pattern_id = song.create_pattern(Duration(0));
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            let _ = pattern.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
        }
        let mut seq =
            SequencerEngine::with_song(Arc::new(RwLock::new(song)), SampleRate::DVD_QUALITY);
        seq.set_preview_pattern(Some((pattern_id, SeqInstrumentId(0))));
        seq.play();

        let mut events = Vec::new();
        // Must return without panicking (div-by-zero / out-of-range modulo).
        seq.process(SampleCount::new(480), &mut events);
    }

    // --- Note-processor expansion (NP1) -------------------------------------

    fn note_on_pitches(events: &[SequencerEvent]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|e| match e {
                SequencerEvent::NoteOn { pitch, .. } => Some(pitch.as_midi()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn processor_rack_expands_in_arrangement_playback() {
        use synth_sequencer::{NoteProcessor, PitchClass, ScaleMask, ScaleQuantize};
        // A C#4 note through a C-major ScaleQuantize rack must emit D4.
        let mut song = Song::new("Rack").with_tempo(Bpm::new(120.0));
        let pattern_id = song.create_pattern(Duration::WHOLE);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            let _ = pattern.add_note(PatternTick(0), Pitch::new(61).unwrap(), Velocity::MF);
            let _ = pattern.add_processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
                root: PitchClass::new(0),
                mask: ScaleMask::MAJOR,
            }));
        }
        let track_id = song.create_track("T");
        song.place_pattern(pattern_id, track_id, Tick::ZERO);

        let mut seq =
            SequencerEngine::with_song(Arc::new(RwLock::new(song)), SampleRate::DVD_QUALITY);
        seq.play();
        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);

        assert_eq!(
            note_on_pitches(&events),
            vec![62],
            "C#4 must be snapped to D4 by the rack: {events:?}"
        );
        assert_eq!(seq.expansion_drops(), 0);
    }

    #[test]
    fn processor_rack_applies_in_orphan_preview() {
        use synth_sequencer::{NoteProcessor, PitchClass, ScaleMask, ScaleQuantize};
        // Preview must sound like playback will: the rack applies there too.
        let (mut song, pattern_id) = create_orphan_song();
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            pattern.clear_notes();
            let _ = pattern.add_note(PatternTick(0), Pitch::new(66).unwrap(), Velocity::MF);
            let _ = pattern.add_processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
                root: PitchClass::new(0),
                mask: ScaleMask::MAJOR,
            }));
        }
        let mut seq =
            SequencerEngine::with_song(Arc::new(RwLock::new(song)), SampleRate::DVD_QUALITY);
        seq.set_preview_pattern(Some((pattern_id, SeqInstrumentId(1))));
        seq.play();

        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);
        // F#4 (66) snaps up to G4 (67) in C major.
        assert!(
            note_on_pitches(&events).contains(&67),
            "preview must apply the rack: {events:?}"
        );
    }

    #[test]
    fn expansion_overflow_drops_and_counts() {
        use synth_sequencer::MAX_EXPANSION_EVENTS_PER_TICK;
        // More simultaneous note-ons than the per-tick cap: the overflow policy
        // drops the newest and counts them — no panic, no allocation growth.
        let excess = 10_usize;
        let mut song = Song::new("Overflow").with_tempo(Bpm::new(120.0));
        let pattern_id = song.create_pattern(Duration::WHOLE);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            for i in 0..(MAX_EXPANSION_EVENTS_PER_TICK + excess) {
                #[allow(clippy::cast_possible_truncation)]
                let pitch = Pitch::new((i % 120) as u8).unwrap();
                let _ = pattern.add_note(PatternTick(0), pitch, Velocity::MF);
            }
        }
        let track_id = song.create_track("T");
        song.place_pattern(pattern_id, track_id, Tick::ZERO);

        let mut seq =
            SequencerEngine::with_song(Arc::new(RwLock::new(song)), SampleRate::DVD_QUALITY);
        seq.play();
        let mut events = Vec::new();
        seq.process(SampleCount::new(1000), &mut events);

        // Same-pitch duplicates coalesce via the legato extend, so count the
        // drop counter rather than emitted events.
        assert_eq!(seq.expansion_drops(), excess as u64);
        let _ = seq.stop();
        assert_eq!(seq.expansion_drops(), 0, "stop resets the drop counter");
    }
}
