//! Runtime events generated during playback.
//!
//! These events are NOT stored - they are generated in real-time from Pattern data.

use serde::{Deserialize, Serialize};
use synth_core::NormalizedValue;

use super::automation::AutomationTarget;
use super::ids::{InstrumentId, TrackId};
use super::note::{Glide, NoteExpression};
use super::pitch::{Pitch, Velocity};
use super::time::Tick;

/// Events generated during sequencer playback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SequencerEvent {
    /// Note on event.
    NoteOn {
        /// Absolute position in the song.
        tick: Tick,
        /// Pitch to play.
        pitch: Pitch,
        /// Velocity/attack strength.
        velocity: Velocity,
        /// Instrument to play.
        instrument: InstrumentId,
        /// Per-note tie/legato intent (taxonomy primitive 2). `Copy`/alloc-free
        /// so this event stays RT-safe to clone on the audio thread. Carried from
        /// the source `Note`; consumed by the engine in Phase B3.
        legato: bool,
        /// Per-note glide (portamento/glissando), `None` when absent. Any absolute
        /// `GlideFrom::Pitch` source is already transposed to the placement's key
        /// at emission, so the event is self-contained. Consumed in Phase B3.
        glide: Option<Glide>,
        /// Per-note expression block (vibrato + note-shape scalars), `None` when
        /// absent. `Copy`/alloc-free. Probability is already resolved at emission
        /// (the note is simply not emitted when it loses the roll), so the engine
        /// consumes only the remaining fields (note-shape scalars in C3, vibrato
        /// in C4).
        expression: Option<NoteExpression>,
        /// Track whose placement spawned this note, `None` for the
        /// placement-less preview and live input. The voice stores it so
        /// track-scoped state (`TrackParam::Pitch`, later per-voice faders)
        /// lands on exactly the voices playing on that track. `serde(default)`
        /// so any externally captured pre-platform event stream still parses
        /// (events are never persisted by the app itself).
        #[serde(default)]
        track: Option<TrackId>,
    },
    /// Note off event.
    NoteOff {
        /// Absolute position in the song.
        tick: Tick,
        /// Pitch to stop.
        pitch: Pitch,
        /// Instrument to stop.
        instrument: InstrumentId,
    },
    /// Parameter automation event.
    Parameter {
        /// Absolute position in the song.
        tick: Tick,
        /// Target parameter.
        target: AutomationTarget,
        /// Normalized value (0.0 - 1.0).
        value: NormalizedValue,
    },
}

impl SequencerEvent {
    /// Get the tick position of this event.
    pub fn tick(&self) -> Tick {
        match self {
            Self::NoteOn { tick, .. }
            | Self::NoteOff { tick, .. }
            | Self::Parameter { tick, .. } => *tick,
        }
    }

    /// Check if this is a note-on event.
    #[must_use]
    pub fn is_note_on(&self) -> bool {
        matches!(self, Self::NoteOn { .. })
    }

    /// Check if this is a note-off event.
    #[must_use]
    pub fn is_note_off(&self) -> bool {
        matches!(self, Self::NoteOff { .. })
    }

    /// Check if this is a parameter event.
    #[must_use]
    pub fn is_parameter(&self) -> bool {
        matches!(self, Self::Parameter { .. })
    }

    /// Sort priority for events at the same tick.
    ///
    /// `NoteOff` events come first (0) so notes are released before new ones start,
    /// then `NoteOn` (1), then `Parameter` (2).
    #[must_use]
    pub fn sort_priority(&self) -> u8 {
        match self {
            Self::NoteOff { .. } => 0,
            Self::NoteOn { .. } => 1,
            Self::Parameter { .. } => 2,
        }
    }

    /// Get the instrument ID if this is a note event.
    pub fn instrument(&self) -> Option<InstrumentId> {
        match self {
            Self::NoteOn { instrument, .. } => Some(*instrument),
            Self::NoteOff { instrument, .. } => Some(*instrument),
            _ => None,
        }
    }

    /// Get the pitch if this is a note event.
    pub fn pitch(&self) -> Option<Pitch> {
        match self {
            Self::NoteOn { pitch, .. } => Some(*pitch),
            Self::NoteOff { pitch, .. } => Some(*pitch),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_event_tick() {
        let event = SequencerEvent::NoteOn {
            tick: Tick(1000),
            pitch: Pitch::new(60).unwrap(),
            velocity: Velocity::MF,
            instrument: InstrumentId(0),
            legato: false,
            glide: None,
            expression: None,
            track: None,
        };
        assert_eq!(event.tick().0, 1000);
    }

    #[test]
    fn test_event_type_checks() {
        let note_on = SequencerEvent::NoteOn {
            tick: Tick(0),
            pitch: Pitch::new(60).unwrap(),
            velocity: Velocity::MF,
            instrument: InstrumentId(0),
            legato: false,
            glide: None,
            expression: None,
            track: None,
        };
        assert!(note_on.is_note_on());
        assert!(!note_on.is_note_off());

        let note_off = SequencerEvent::NoteOff {
            tick: Tick(960),
            pitch: Pitch::new(60).unwrap(),
            instrument: InstrumentId(0),
        };
        assert!(note_off.is_note_off());
        assert!(!note_off.is_note_on());
    }

    #[test]
    fn test_event_sorting() {
        let mut events = Vec::from([
            SequencerEvent::NoteOn {
                tick: Tick(500),
                pitch: Pitch::new(60).unwrap(),
                velocity: Velocity::MF,
                instrument: InstrumentId(0),
                legato: false,
                glide: None,
                expression: None,
                track: None,
            },
            SequencerEvent::NoteOn {
                tick: Tick(100),
                pitch: Pitch::new(62).unwrap(),
                velocity: Velocity::MF,
                instrument: InstrumentId(0),
                legato: false,
                glide: None,
                expression: None,
                track: None,
            },
            SequencerEvent::NoteOff {
                tick: Tick(300),
                pitch: Pitch::new(60).unwrap(),
                instrument: InstrumentId(0),
            },
        ]);

        events.sort_by_key(|e| (e.tick(), e.sort_priority()));

        let ticks: Vec<_> = events.iter().map(|e| e.tick().0).collect();
        assert_eq!(ticks, vec![100, 300, 500]);
    }

    #[test]
    fn test_event_instrument() {
        let note = SequencerEvent::NoteOn {
            tick: Tick(0),
            pitch: Pitch::new(60).unwrap(),
            velocity: Velocity::MF,
            instrument: InstrumentId(5),
            legato: false,
            glide: None,
            expression: None,
            track: None,
        };
        assert_eq!(note.instrument(), Some(InstrumentId(5)));
    }
}
