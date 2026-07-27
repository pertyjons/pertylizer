//! Recording buffer for capturing live note input into patterns.
//!
//! Captures NoteOn/Off events with timing, converts song ticks to pattern ticks.
//! Designed for real-time safety — no heap allocations during capture.

use synth_sequencer::{Duration, PatternId, PatternTick, Pitch, Tick, TrackId, Velocity};

/// Maximum number of simultaneously held notes during recording.
const MAX_HELD_NOTES: usize = 32;

/// Maximum number of recorded notes per recording session.
const MAX_RECORDED_NOTES: usize = 4096;

/// A note currently being held during recording.
#[derive(Debug, Clone, Copy)]
struct HeldNote {
    pitch: Pitch,
    velocity: Velocity,
    start_tick: PatternTick,
}

/// A completed recorded note ready to be written to a pattern.
#[derive(Debug, Clone, Copy)]
pub struct RecordedNote {
    pub pitch: Pitch,
    pub velocity: Velocity,
    pub start: PatternTick,
    pub duration: Duration,
}

/// Recording target information.
#[derive(Debug, Clone, Copy)]
struct RecordTarget {
    pattern_id: PatternId,
    #[expect(
        dead_code,
        reason = "recording retains its selected track for the pending track-aware commit path"
    )]
    track_id: TrackId,
    region_start: Tick,
    pattern_length: Duration,
    ticks_per_bar: Duration,
    /// Whether to layer notes on existing pattern (true) or replace (false).
    overdub: bool,
}

/// Recording state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    /// No recording active.
    Idle,
    /// Armed and waiting for playback to start.
    Armed,
    /// Count-in period (1 bar before recording starts).
    CountIn,
    /// Actively capturing notes.
    Capturing,
}

impl RecordingState {
    /// Convert to shared state u32 value.
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Idle => 0,
            Self::Armed => 1,
            Self::CountIn => 2,
            Self::Capturing => 3,
        }
    }

    /// Convert from shared state u32 value.
    pub fn from_u32(value: u32) -> Self {
        match value {
            1 => Self::Armed,
            2 => Self::CountIn,
            3 => Self::Capturing,
            _ => Self::Idle,
        }
    }
}

/// Buffer for recording live note input into patterns.
///
/// Pre-allocated for real-time safety — no heap allocations during capture.
/// Uses a spare-Vec swap strategy so `flush()` / `take_released_notes()` never
/// allocate on the audio thread. Capacity is ensured in `arm()` (called once
/// before recording starts).
pub(crate) struct RecordingBuffer {
    state: RecordingState,
    target: Option<RecordTarget>,
    held_notes: [Option<HeldNote>; MAX_HELD_NOTES],
    recorded_notes: Vec<RecordedNote>,
    /// Pre-allocated spare buffer swapped in by `flush()` / `take_released_notes()`
    /// so the active buffer is replaced without heap allocation.
    spare_notes: Vec<RecordedNote>,
    /// Tick at which the count-in ends and capturing begins.
    capture_start_tick: Tick,
    /// Quantization grid size (Duration(0) = no quantization).
    quantize_grid: Duration,
    /// Whether notes have been flushed at a loop boundary during this session.
    /// After the first flush, overdub is always true to preserve earlier passes.
    loop_flushed: bool,
    /// Number of held notes dropped because all slots were full.
    dropped_notes: u32,
    /// Set when a note is added or released; cleared after preview_snapshot sends.
    /// Avoids cloning recorded_notes every buffer when nothing changed.
    preview_dirty: bool,
}

impl RecordingBuffer {
    /// Create a new recording buffer.
    pub(crate) fn new() -> Self {
        Self {
            state: RecordingState::Idle,
            target: None,
            held_notes: [None; MAX_HELD_NOTES],
            recorded_notes: Vec::with_capacity(MAX_RECORDED_NOTES),
            spare_notes: Vec::with_capacity(MAX_RECORDED_NOTES),
            capture_start_tick: Tick::ZERO,
            quantize_grid: Duration(0),
            loop_flushed: false,
            dropped_notes: 0,
            preview_dirty: false,
        }
    }

    /// Get current recording state.
    pub(crate) fn state(&self) -> RecordingState {
        self.state
    }

    /// Get the target pattern ID (if armed or capturing).
    pub(crate) fn target_pattern(&self) -> Option<PatternId> {
        self.target.map(|t| t.pattern_id)
    }

    /// Number of notes dropped because held-note slots were full.
    #[cfg(test)]
    pub(crate) fn dropped_note_count(&self) -> u32 {
        self.dropped_notes
    }

    /// Set the quantization grid size (Duration(0) = no quantization).
    pub(crate) fn set_quantize_grid(&mut self, grid: Duration) {
        self.quantize_grid = grid;
    }

    /// Get recording target info: (region_start, pattern_length).
    pub(crate) fn target_info(&self) -> Option<(Tick, Duration)> {
        self.target.map(|t| (t.region_start, t.pattern_length))
    }

    /// Get a snapshot of current recording for live preview, if notes changed.
    ///
    /// Returns `Some((completed_notes, held_note_starts))` only when a note was
    /// added or released since the last snapshot. Returns `None` (no allocation)
    /// on the majority of buffer callbacks where nothing changed.
    /// Called at buffer-rate (~86Hz), not per-sample.
    #[allow(clippy::type_complexity)]
    pub(crate) fn preview_snapshot(
        &mut self,
    ) -> Option<(Vec<RecordedNote>, Vec<(Pitch, PatternTick)>)> {
        if !self.preview_dirty {
            return None;
        }
        self.preview_dirty = false;

        let completed = self.recorded_notes.clone();
        let mut held = Vec::with_capacity(MAX_HELD_NOTES);
        for h in self.held_notes.iter().flatten() {
            held.push((h.pitch, h.start_tick));
        }
        Some((completed, held))
    }

    /// Whether recorded notes should be layered rather than replacing the pattern.
    /// After the first loop-boundary flush, always returns true to preserve earlier passes.
    pub(crate) fn is_overdub(&self) -> bool {
        self.loop_flushed || self.target.is_none_or(|t| t.overdub)
    }

    /// Arm recording for a specific pattern.
    pub(crate) fn arm(
        &mut self,
        pattern_id: PatternId,
        track_id: TrackId,
        region_start: Tick,
        pattern_length: Duration,
        ticks_per_bar: Duration,
        overdub: bool,
    ) {
        self.state = RecordingState::Armed;
        self.target = Some(RecordTarget {
            pattern_id,
            track_id,
            region_start,
            pattern_length,
            ticks_per_bar,
            overdub,
        });
        self.clear();
    }

    /// Disarm recording, returning any captured notes.
    pub(crate) fn disarm(&mut self) -> Vec<RecordedNote> {
        let notes = self.flush();
        self.state = RecordingState::Idle;
        self.target = None;
        notes
    }

    /// Start playback while armed — begins count-in.
    /// Returns the tick position to seek to (1 bar before region start).
    /// Uses ticks_per_bar stored from arm() call.
    pub(crate) fn start_playback(&mut self) -> Option<Tick> {
        if self.state != RecordingState::Armed {
            return None;
        }
        let target = self.target?;
        self.state = RecordingState::CountIn;
        self.capture_start_tick = target.region_start;

        // Seek to 1 bar before region start
        let seek_to = Tick(
            target
                .region_start
                .0
                .saturating_sub(target.ticks_per_bar.0 as u64),
        );
        Some(seek_to)
    }

    /// Get cached ticks_per_bar (for metronome accent detection).
    pub(crate) fn ticks_per_bar(&self) -> Duration {
        self.target.map_or(Duration::WHOLE, |t| t.ticks_per_bar) // Default 4/4 at 960 PPQN
    }

    /// Tick the state machine — called each audio tick.
    /// Transitions CountIn → Capturing when we reach the region start.
    pub(crate) fn tick(&mut self, current_tick: Tick) {
        if self.state == RecordingState::CountIn && current_tick >= self.capture_start_tick {
            self.state = RecordingState::Capturing;
        }
    }

    /// Record a note-on event (only captures if in Capturing state).
    pub(crate) fn note_on(&mut self, pitch: Pitch, velocity: Velocity, song_tick: Tick) {
        if self.state != RecordingState::Capturing {
            return;
        }
        let pattern_tick = self.song_tick_to_pattern_tick(song_tick);

        // Find an empty slot
        for slot in &mut self.held_notes {
            if slot.is_none() {
                *slot = Some(HeldNote {
                    pitch,
                    velocity,
                    start_tick: pattern_tick,
                });
                self.preview_dirty = true;
                return;
            }
        }
        // All slots full — count the drop so the UI can warn the user
        self.dropped_notes += 1;
    }

    /// Record a note-off event (completes a held note).
    pub(crate) fn note_off(&mut self, pitch: Pitch, song_tick: Tick) {
        if self.state != RecordingState::Capturing {
            return;
        }
        let pattern_tick = self.song_tick_to_pattern_tick(song_tick);

        for slot in &mut self.held_notes {
            if let Some(held) = slot
                && held.pitch == pitch
            {
                let duration_ticks = pattern_tick.0.saturating_sub(held.start_tick.0);
                let duration = Duration(duration_ticks.max(1));

                if self.recorded_notes.len() < MAX_RECORDED_NOTES {
                    self.recorded_notes.push(RecordedNote {
                        pitch: held.pitch,
                        velocity: held.velocity,
                        start: held.start_tick,
                        duration,
                    });
                }
                *slot = None;
                self.preview_dirty = true;
                return;
            }
        }
    }

    /// Stop recording and return all captured notes.
    /// Completes any held notes with a default short duration.
    ///
    /// Real-time safe: swaps in a pre-allocated spare Vec instead of allocating.
    pub(crate) fn flush(&mut self) -> Vec<RecordedNote> {
        // Complete any still-held notes with minimum duration
        for slot in &mut self.held_notes {
            if let Some(held) = slot.take()
                && self.recorded_notes.len() < MAX_RECORDED_NOTES
            {
                self.recorded_notes.push(RecordedNote {
                    pitch: held.pitch,
                    velocity: held.velocity,
                    start: held.start_tick,
                    duration: Duration(240), // 16th note fallback
                });
            }
        }
        self.state = RecordingState::Idle;
        self.swap_and_take_notes()
    }

    /// Take all released notes without stopping recording.
    ///
    /// Used at loop boundaries to flush notes to the pattern mid-recording,
    /// so they play back on subsequent loop passes.
    /// Preserves held notes and recording state. Sets `loop_flushed` so
    /// subsequent `is_overdub()` calls always return true.
    ///
    /// **Call `is_overdub()` before this** if the first flush should respect
    /// the user's original replace/overdub choice.
    ///
    /// Real-time safe: swaps in a pre-allocated spare Vec instead of allocating.
    pub(crate) fn take_released_notes(&mut self) -> Vec<RecordedNote> {
        self.loop_flushed = true;
        self.swap_and_take_notes()
    }

    /// Swap recorded_notes with the pre-allocated spare and return the data.
    ///
    /// After the swap, `self.recorded_notes` is the (now cleared) spare with
    /// pre-allocated capacity, ready for the next recording pass.
    /// The returned Vec contains the captured notes and is consumed by the caller
    /// (sent to the UI thread via ring buffer, where it is eventually dropped).
    fn swap_and_take_notes(&mut self) -> Vec<RecordedNote> {
        // Swap spare in — recorded_notes becomes empty (spare), spare becomes data
        std::mem::swap(&mut self.recorded_notes, &mut self.spare_notes);
        // Clear the new recorded_notes (was spare) to prepare for next pass
        self.recorded_notes.clear();
        // Take the data out of spare; spare becomes a zero-capacity Vec (just pointer writes)
        std::mem::take(&mut self.spare_notes)
    }

    /// Convert a song tick to a pattern-relative tick.
    #[allow(clippy::cast_possible_truncation)] // pattern_tick fits in u32 after modulo
    fn song_tick_to_pattern_tick(&self, song_tick: Tick) -> PatternTick {
        let Some(target) = self.target else {
            return PatternTick::ZERO;
        };
        let relative = song_tick.0.saturating_sub(target.region_start.0);
        let pattern_tick = if target.pattern_length.0 > 0 {
            (relative % target.pattern_length.0 as u64) as u32
        } else {
            relative as u32
        };

        // Apply quantization if enabled
        let grid = self.quantize_grid.0;
        match (pattern_tick + grid / 2).checked_div(grid) {
            Some(quotient) => PatternTick(quotient * grid),
            None => PatternTick(pattern_tick),
        }
    }

    /// Clear all held notes and recorded notes.
    ///
    /// Called by `arm()` before a new recording session. Ensures both
    /// `recorded_notes` and `spare_notes` have pre-allocated capacity so
    /// that `flush()` / `take_released_notes()` never allocate during capture.
    /// The `reserve()` calls here are acceptable because `arm()` runs once
    /// before real-time recording begins.
    fn clear(&mut self) {
        self.held_notes = [None; MAX_HELD_NOTES];
        self.recorded_notes.clear();
        self.loop_flushed = false;
        self.dropped_notes = 0;
        self.preview_dirty = false;
        // Ensure both buffers have capacity for the next session.
        // This may allocate, but arm() is called before real-time capture starts.
        if self.recorded_notes.capacity() < MAX_RECORDED_NOTES {
            self.recorded_notes
                .reserve(MAX_RECORDED_NOTES - self.recorded_notes.capacity());
        }
        if self.spare_notes.capacity() < MAX_RECORDED_NOTES {
            self.spare_notes
                .reserve(MAX_RECORDED_NOTES - self.spare_notes.capacity());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn p(midi: u8) -> Pitch {
        Pitch::new(midi).unwrap()
    }

    fn vel(midi: u8) -> Velocity {
        Velocity::from_midi(midi)
    }

    #[test]
    fn test_arm_and_state() {
        let mut buf = RecordingBuffer::new();
        assert_eq!(buf.state(), RecordingState::Idle);

        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(3840),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        assert_eq!(buf.state(), RecordingState::Armed);
        assert_eq!(buf.target_pattern(), Some(PatternId::new(1)));
    }

    #[test]
    fn test_count_in_transition() {
        let mut buf = RecordingBuffer::new();
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(3840),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );

        let seek = buf.start_playback();
        assert_eq!(buf.state(), RecordingState::CountIn);
        assert_eq!(seek, Some(Tick(0))); // 3840 - 3840 = 0

        // Still in count-in
        buf.tick(Tick(1920));
        assert_eq!(buf.state(), RecordingState::CountIn);

        // Reaches region start → capturing
        buf.tick(Tick(3840));
        assert_eq!(buf.state(), RecordingState::Capturing);
    }

    #[test]
    fn test_note_recording() {
        let mut buf = RecordingBuffer::new();
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(0),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        buf.state = RecordingState::Capturing;

        buf.note_on(p(60), vel(100), Tick(0));
        buf.note_off(p(60), Tick(960));

        buf.note_on(p(64), vel(80), Tick(960));
        buf.note_off(p(64), Tick(1920));

        let notes = buf.flush();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].pitch, p(60));
        assert_eq!(notes[0].start, PatternTick(0));
        assert_eq!(notes[0].duration, Duration(960));
        assert_eq!(notes[1].pitch, p(64));
        assert_eq!(notes[1].start, PatternTick(960));
    }

    #[test]
    fn test_pattern_wrapping() {
        let mut buf = RecordingBuffer::new();
        // Pattern at tick 3840, length 3840 (1 bar)
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(3840),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        buf.state = RecordingState::Capturing;

        // Note at song tick 7680 = region_start + 3840 = wraps to pattern tick 0
        buf.note_on(p(60), vel(100), Tick(7680));
        buf.note_off(p(60), Tick(8640));

        let notes = buf.flush();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].start, PatternTick(0));
        assert_eq!(notes[0].duration, Duration(960));
    }

    #[test]
    fn test_disarm_returns_notes() {
        let mut buf = RecordingBuffer::new();
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(0),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        buf.state = RecordingState::Capturing;

        buf.note_on(p(60), vel(100), Tick(0));
        buf.note_off(p(60), Tick(480));

        let notes = buf.disarm();
        assert_eq!(notes.len(), 1);
        assert_eq!(buf.state(), RecordingState::Idle);
    }

    #[test]
    fn test_notes_ignored_during_count_in() {
        let mut buf = RecordingBuffer::new();
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(3840),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        let _ = buf.start_playback();

        // Note during count-in should be ignored
        buf.note_on(p(60), vel(100), Tick(1920));
        buf.note_off(p(60), Tick(2880));

        let notes = buf.flush();
        assert!(notes.is_empty());
    }

    #[test]
    fn test_dropped_note_counter() {
        let mut buf = RecordingBuffer::new();
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(0),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        buf.state = RecordingState::Capturing;
        assert_eq!(buf.dropped_note_count(), 0);

        // Fill all held-note slots
        for i in 0..MAX_HELD_NOTES as u8 {
            buf.note_on(p(60 + i), vel(100), Tick(0));
        }
        assert_eq!(buf.dropped_note_count(), 0);

        // Next note should be dropped
        buf.note_on(p(40), vel(100), Tick(0));
        assert_eq!(buf.dropped_note_count(), 1);

        buf.note_on(p(41), vel(100), Tick(0));
        assert_eq!(buf.dropped_note_count(), 2);

        // Counter resets on arm
        let _ = buf.flush();
        buf.arm(
            PatternId::new(2),
            TrackId::new(0),
            Tick(0),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        assert_eq!(buf.dropped_note_count(), 0);
    }

    #[test]
    fn test_flush_preserves_capacity() {
        let mut buf = RecordingBuffer::new();
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(0),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        buf.state = RecordingState::Capturing;

        buf.note_on(p(60), vel(100), Tick(0));
        buf.note_off(p(60), Tick(480));

        let notes = buf.flush();
        assert_eq!(notes.len(), 1);
        // After flush, recorded_notes should still have capacity (from spare)
        assert!(buf.recorded_notes.capacity() >= MAX_RECORDED_NOTES);
    }

    #[test]
    fn test_take_released_notes_preserves_capacity() {
        let mut buf = RecordingBuffer::new();
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(0),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        buf.state = RecordingState::Capturing;

        buf.note_on(p(60), vel(100), Tick(0));
        buf.note_off(p(60), Tick(480));

        let notes = buf.take_released_notes();
        assert_eq!(notes.len(), 1);
        // After take, recorded_notes should have capacity (from spare)
        assert!(buf.recorded_notes.capacity() >= MAX_RECORDED_NOTES);
    }

    #[test]
    fn test_held_notes_completed_on_flush() {
        let mut buf = RecordingBuffer::new();
        buf.arm(
            PatternId::new(1),
            TrackId::new(0),
            Tick(0),
            Duration::WHOLE,
            Duration::WHOLE,
            true,
        );
        buf.state = RecordingState::Capturing;

        // Start a note but never release it
        buf.note_on(p(60), vel(100), Tick(0));

        let notes = buf.flush();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].duration, Duration(240)); // Fallback 16th note
    }
}
