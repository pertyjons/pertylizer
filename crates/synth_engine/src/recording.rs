//! Recording buffer for capturing live note input into patterns.
//!
//! Captures NoteOn/Off events with timing, converts song ticks to pattern ticks.
//! Designed for real-time safety — no heap allocations during capture.

use synth_sequencer::{Duration, PatternId, PatternTick, Tick, TrackId};

/// Maximum number of simultaneously held notes during recording.
const MAX_HELD_NOTES: usize = 32;

/// Maximum number of recorded notes per recording session.
const MAX_RECORDED_NOTES: usize = 4096;

/// A note currently being held during recording.
#[derive(Debug, Clone, Copy)]
struct HeldNote {
    pitch: u8,
    velocity: u8,
    start_tick: PatternTick,
}

/// A completed recorded note ready to be written to a pattern.
#[derive(Debug, Clone, Copy)]
pub struct RecordedNote {
    pub pitch: u8,
    pub velocity: u8,
    pub start: PatternTick,
    pub duration: Duration,
}

/// Recording target information.
#[derive(Debug, Clone, Copy)]
struct RecordTarget {
    pattern_id: PatternId,
    #[allow(dead_code)] // Will be used for track-specific recording
    track_id: TrackId,
    region_start: Tick,
    pattern_length_ticks: u32,
    ticks_per_bar: u32,
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
pub(crate) struct RecordingBuffer {
    state: RecordingState,
    target: Option<RecordTarget>,
    held_notes: [Option<HeldNote>; MAX_HELD_NOTES],
    recorded_notes: Vec<RecordedNote>,
    /// Tick at which the count-in ends and capturing begins.
    capture_start_tick: Tick,
    /// Quantization grid size (0 = no quantization).
    quantize_grid: u32,
}

impl RecordingBuffer {
    /// Create a new recording buffer.
    pub(crate) fn new() -> Self {
        Self {
            state: RecordingState::Idle,
            target: None,
            held_notes: [None; MAX_HELD_NOTES],
            recorded_notes: Vec::with_capacity(MAX_RECORDED_NOTES),
            capture_start_tick: Tick::ZERO,
            quantize_grid: 0,
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

    /// Arm recording for a specific pattern.
    pub(crate) fn arm(
        &mut self,
        pattern_id: PatternId,
        track_id: TrackId,
        region_start: Tick,
        pattern_length_ticks: u32,
        ticks_per_bar: u32,
    ) {
        self.state = RecordingState::Armed;
        self.target = Some(RecordTarget {
            pattern_id,
            track_id,
            region_start,
            pattern_length_ticks,
            ticks_per_bar,
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
        let count_in_ticks = target.ticks_per_bar as u64;
        let seek_to = Tick(target.region_start.0.saturating_sub(count_in_ticks));
        Some(seek_to)
    }

    /// Get cached ticks_per_bar (for metronome accent detection).
    pub(crate) fn ticks_per_bar(&self) -> u32 {
        self.target.map_or(3840, |t| t.ticks_per_bar) // Default 4/4 at 960 PPQN
    }

    /// Tick the state machine — called each audio tick.
    /// Transitions CountIn → Capturing when we reach the region start.
    pub(crate) fn tick(&mut self, current_tick: Tick) {
        if self.state == RecordingState::CountIn && current_tick >= self.capture_start_tick {
            self.state = RecordingState::Capturing;
        }
    }

    /// Record a note-on event (only captures if in Capturing state).
    pub(crate) fn note_on(&mut self, pitch: u8, velocity: u8, song_tick: Tick) {
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
                return;
            }
        }
        // All slots full — drop the note silently
    }

    /// Record a note-off event (completes a held note).
    pub(crate) fn note_off(&mut self, pitch: u8, song_tick: Tick) {
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
                return;
            }
        }
    }

    /// Stop recording and return all captured notes.
    /// Completes any held notes with a default short duration.
    /// Uses `drain()` to preserve pre-allocated Vec capacity for real-time safety.
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
        // drain preserves capacity so next recording won't allocate on audio thread
        self.recorded_notes.drain(..).collect()
    }

    /// Convert a song tick to a pattern-relative tick.
    fn song_tick_to_pattern_tick(&self, song_tick: Tick) -> PatternTick {
        let Some(target) = self.target else {
            return PatternTick::ZERO;
        };
        let relative = song_tick.0.saturating_sub(target.region_start.0);
        let pattern_tick = if target.pattern_length_ticks > 0 {
            (relative % target.pattern_length_ticks as u64) as u32
        } else {
            relative as u32
        };

        // Apply quantization if enabled
        if self.quantize_grid > 0 {
            let rounded =
                ((pattern_tick + self.quantize_grid / 2) / self.quantize_grid) * self.quantize_grid;
            PatternTick(rounded)
        } else {
            PatternTick(pattern_tick)
        }
    }

    /// Clear all held notes and recorded notes.
    fn clear(&mut self) {
        self.held_notes = [None; MAX_HELD_NOTES];
        self.recorded_notes.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_arm_and_state() {
        let mut buf = RecordingBuffer::new();
        assert_eq!(buf.state(), RecordingState::Idle);

        buf.arm(PatternId::new(1), TrackId::new(0), Tick(3840), 3840, 3840);
        assert_eq!(buf.state(), RecordingState::Armed);
        assert_eq!(buf.target_pattern(), Some(PatternId::new(1)));
    }

    #[test]
    fn test_count_in_transition() {
        let mut buf = RecordingBuffer::new();
        buf.arm(PatternId::new(1), TrackId::new(0), Tick(3840), 3840, 3840);

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
        buf.arm(PatternId::new(1), TrackId::new(0), Tick(0), 3840, 3840);
        buf.state = RecordingState::Capturing;

        buf.note_on(60, 100, Tick(0));
        buf.note_off(60, Tick(960));

        buf.note_on(64, 80, Tick(960));
        buf.note_off(64, Tick(1920));

        let notes = buf.flush();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].pitch, 60);
        assert_eq!(notes[0].start, PatternTick(0));
        assert_eq!(notes[0].duration, Duration(960));
        assert_eq!(notes[1].pitch, 64);
        assert_eq!(notes[1].start, PatternTick(960));
    }

    #[test]
    fn test_pattern_wrapping() {
        let mut buf = RecordingBuffer::new();
        // Pattern at tick 3840, length 3840 (1 bar)
        buf.arm(PatternId::new(1), TrackId::new(0), Tick(3840), 3840, 3840);
        buf.state = RecordingState::Capturing;

        // Note at song tick 7680 = region_start + 3840 = wraps to pattern tick 0
        buf.note_on(60, 100, Tick(7680));
        buf.note_off(60, Tick(8640));

        let notes = buf.flush();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].start, PatternTick(0));
        assert_eq!(notes[0].duration, Duration(960));
    }

    #[test]
    fn test_disarm_returns_notes() {
        let mut buf = RecordingBuffer::new();
        buf.arm(PatternId::new(1), TrackId::new(0), Tick(0), 3840, 3840);
        buf.state = RecordingState::Capturing;

        buf.note_on(60, 100, Tick(0));
        buf.note_off(60, Tick(480));

        let notes = buf.disarm();
        assert_eq!(notes.len(), 1);
        assert_eq!(buf.state(), RecordingState::Idle);
    }

    #[test]
    fn test_notes_ignored_during_count_in() {
        let mut buf = RecordingBuffer::new();
        buf.arm(PatternId::new(1), TrackId::new(0), Tick(3840), 3840, 3840);
        let _ = buf.start_playback();

        // Note during count-in should be ignored
        buf.note_on(60, 100, Tick(1920));
        buf.note_off(60, Tick(2880));

        let notes = buf.flush();
        assert!(notes.is_empty());
    }

    #[test]
    fn test_held_notes_completed_on_flush() {
        let mut buf = RecordingBuffer::new();
        buf.arm(PatternId::new(1), TrackId::new(0), Tick(0), 3840, 3840);
        buf.state = RecordingState::Capturing;

        // Start a note but never release it
        buf.note_on(60, 100, Tick(0));

        let notes = buf.flush();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].duration, Duration(240)); // Fallback 16th note
    }
}
