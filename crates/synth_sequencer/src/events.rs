//! Runtime events generated during playback.
//!
//! These events are NOT stored - they are generated in real-time from Pattern data.

use serde::{Deserialize, Serialize};

use synth_core::NormalizedValue;

use super::automation::AutomationTarget;
use super::ids::SeqInstrumentId;
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
        instrument: SeqInstrumentId,
    },
    /// Note off event.
    NoteOff {
        /// Absolute position in the song.
        tick: Tick,
        /// Pitch to stop.
        pitch: Pitch,
        /// Instrument to stop.
        instrument: SeqInstrumentId,
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
    pub fn is_note_on(&self) -> bool {
        matches!(self, Self::NoteOn { .. })
    }

    /// Check if this is a note-off event.
    pub fn is_note_off(&self) -> bool {
        matches!(self, Self::NoteOff { .. })
    }

    /// Check if this is a parameter event.
    pub fn is_parameter(&self) -> bool {
        matches!(self, Self::Parameter { .. })
    }

    /// Get the instrument ID if this is a note event.
    pub fn instrument(&self) -> Option<SeqInstrumentId> {
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

/// Extension trait for sorting events.
pub trait EventSorting {
    /// Sort events by tick position.
    fn sort_by_tick(&mut self);
}

impl EventSorting for Vec<SequencerEvent> {
    fn sort_by_tick(&mut self) {
        self.sort_by_key(|e| e.tick());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use synth_core::NormalizedValue;

    #[test]
    fn test_event_tick() {
        let event = SequencerEvent::NoteOn {
            tick: Tick(1000),
            pitch: Pitch::new(60).unwrap(),
            velocity: Velocity::MF,
            instrument: SeqInstrumentId(0),
        };
        assert_eq!(event.tick().0, 1000);
    }

    #[test]
    fn test_event_type_checks() {
        let note_on = SequencerEvent::NoteOn {
            tick: Tick(0),
            pitch: Pitch::new(60).unwrap(),
            velocity: Velocity::MF,
            instrument: SeqInstrumentId(0),
        };
        assert!(note_on.is_note_on());
        assert!(!note_on.is_note_off());

        let note_off = SequencerEvent::NoteOff {
            tick: Tick(960),
            pitch: Pitch::new(60).unwrap(),
            instrument: SeqInstrumentId(0),
        };
        assert!(note_off.is_note_off());
        assert!(!note_off.is_note_on());
    }

    #[test]
    fn test_event_sorting() {
        let mut events = vec![
            SequencerEvent::NoteOn {
                tick: Tick(500),
                pitch: Pitch::new(60).unwrap(),
                velocity: Velocity::MF,
                instrument: SeqInstrumentId(0),
            },
            SequencerEvent::NoteOn {
                tick: Tick(100),
                pitch: Pitch::new(62).unwrap(),
                velocity: Velocity::MF,
                instrument: SeqInstrumentId(0),
            },
            SequencerEvent::NoteOff {
                tick: Tick(300),
                pitch: Pitch::new(60).unwrap(),
                instrument: SeqInstrumentId(0),
            },
        ];

        events.sort_by_tick();

        let ticks: Vec<_> = events.iter().map(|e| e.tick().0).collect();
        assert_eq!(ticks, vec![100, 300, 500]);
    }

    #[test]
    fn test_event_instrument() {
        let note = SequencerEvent::NoteOn {
            tick: Tick(0),
            pitch: Pitch::new(60).unwrap(),
            velocity: Velocity::MF,
            instrument: SeqInstrumentId(5),
        };
        assert_eq!(note.instrument(), Some(SeqInstrumentId(5)));

        let _unused = NormalizedValue::MIN; // keep import used
    }
}
