//! Typed MIDI 1 publication and immutable note-capture observations.

use crate::controller::BipolarLevel;
use crate::host::ConnectionGeneration;
use crate::ingress::IngressRefused;
use crate::quantities::{KeyIdentity, NoteVelocity, QuantityError};
use crate::time::{SampleTime, StreamEpoch, TimeSource};
use synth_core::MidiChannel;

use super::NoteCaptureError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct CaptureSessionId(pub(super) u64);
impl CaptureSessionId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct PublicationSequence(pub(super) u64);
impl PublicationSequence {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PerformedOccurrenceId {
    pub(super) source: ConnectionGeneration,
    pub(super) serial: u64,
}
impl PerformedOccurrenceId {
    pub const fn source(self) -> ConnectionGeneration {
        self.source
    }
    pub const fn serial(self) -> u64 {
        self.serial
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct CapturePassId(pub(super) u64);
impl CapturePassId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// A fixture target token; not Phase 10's canonical project identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct FixtureTargetId(u64);
impl FixtureTargetId {
    pub fn new(value: u64) -> Result<Self, NoteCaptureError> {
        if value == 0 {
            return Err(NoteCaptureError::InvalidTarget);
        }
        Ok(Self(value))
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// An expected fixture revision, retained without claiming a project transaction service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct FixtureRevision(u64);
impl FixtureRevision {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Midi1Event {
    NoteOn {
        key: KeyIdentity,
        velocity: NoteVelocity,
    },
    KeyRelease {
        key: KeyIdentity,
        velocity: NoteVelocity,
    },
    Sustain {
        down: bool,
    },
    PitchBend {
        value: BipolarLevel,
    },
}

/// Validated supported three-byte MIDI 1 input. Construction never clamps invalid bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct Midi1Input {
    pub(super) channel: MidiChannel,
    pub(super) event: Midi1Event,
}
impl Midi1Input {
    pub fn from_bytes(bytes: [u8; 3]) -> Result<Self, NoteCaptureError> {
        let [status, first, second] = bytes;
        if first > 127 || second > 127 {
            return Err(NoteCaptureError::InvalidMidi);
        }
        let event = match status & 0xf0 {
            0x80 => Midi1Event::KeyRelease {
                key: KeyIdentity::new(first)?,
                velocity: midi_velocity(second)?,
            },
            0x90 if second == 0 => Midi1Event::KeyRelease {
                key: KeyIdentity::new(first)?,
                velocity: NoteVelocity::SILENT,
            },
            0x90 => Midi1Event::NoteOn {
                key: KeyIdentity::new(first)?,
                velocity: midi_velocity(second)?,
            },
            0xb0 if first == 64 => Midi1Event::Sustain { down: second >= 64 },
            0xe0 => {
                let raw = u16::from(first) | (u16::from(second) << 7);
                let value = (f32::from(raw) - 8192.0) / if raw < 8192 { 8192.0 } else { 8191.0 };
                Midi1Event::PitchBend {
                    value: BipolarLevel::new(value)?,
                }
            }
            _ => return Err(NoteCaptureError::UnsupportedMidi),
        };
        // The status nibble proved the channel is 1..=16 before the legacy constructor.
        let channel = MidiChannel::new((status & 0x0f) + 1);
        Ok(Self { channel, event })
    }
    pub const fn channel(self) -> MidiChannel {
        self.channel
    }
    pub const fn event(self) -> Midi1Event {
        self.event
    }
}
fn midi_velocity(value: u8) -> Result<NoteVelocity, QuantityError> {
    NoteVelocity::new(f32::from(value) / 127.0)
}

/// This fixture records the supplied audition result; it does not publish to a renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditionTrace {
    /// Integrated live admission, resolved before the raw take can seal.
    Pending(crate::host::live::AuditionId),
    /// A final outcome in the separately owned audition stream's epoch.
    Resolved(crate::host::live::AuditionOutcome),
    NotOffered,
    Refused(IngressRefused),
    Executed(SampleTime),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct CaptureStamp {
    pub(super) epoch: StreamEpoch,
    pub(super) nominal: SampleTime,
    pub(super) published_at: SampleTime,
}
impl CaptureStamp {
    pub const fn epoch(self) -> StreamEpoch {
        self.epoch
    }
    pub const fn nominal(self) -> SampleTime {
        self.nominal
    }
    pub const fn published_at(self) -> SampleTime {
        self.published_at
    }
    pub const fn provenance(self) -> TimeSource {
        // Reuse ADR-0053's single constructor for simulated provenance.
        crate::ingress::PerformanceIngress::envelope_for(self.epoch, self.nominal).source()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct RecordedInput {
    pub(super) source: ConnectionGeneration,
    pub(super) sequence: PublicationSequence,
    pub(super) stamp: CaptureStamp,
    pub(super) input: Midi1Input,
    pub(super) occurrence: Option<PerformedOccurrenceId>,
    pub(super) audition: AuditionTrace,
    pub(super) timing_anomaly: bool,
}
impl RecordedInput {
    pub const fn source(self) -> ConnectionGeneration {
        self.source
    }
    pub const fn sequence(self) -> PublicationSequence {
        self.sequence
    }
    pub const fn stamp(self) -> CaptureStamp {
        self.stamp
    }
    pub const fn input(self) -> Midi1Input {
        self.input
    }
    pub const fn occurrence(self) -> Option<PerformedOccurrenceId> {
        self.occurrence
    }
    pub const fn audition(self) -> AuditionTrace {
        self.audition
    }
    pub const fn timing_anomaly(self) -> bool {
        self.timing_anomaly
    }
}

/// Complete state for the two controller classes this adapter supports, by MIDI channel.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ControllerSnapshot {
    pub(super) pedals: [bool; 16],
    pub(super) bends: [BipolarLevel; 16],
}
impl ControllerSnapshot {
    pub const fn neutral() -> Self {
        Self {
            pedals: [false; 16],
            bends: [BipolarLevel::ZERO; 16],
        }
    }
    pub fn with_input(mut self, input: Midi1Input) -> Result<Self, NoteCaptureError> {
        let at = usize::from(input.channel.as_index());
        match input.event {
            Midi1Event::Sustain { down } => self.pedals[at] = down,
            Midi1Event::PitchBend { value } => self.bends[at] = value,
            _ => return Err(NoteCaptureError::NotController),
        }
        Ok(self)
    }
    pub const fn pedals(&self) -> &[bool; 16] {
        &self.pedals
    }
    pub const fn bends(&self) -> &[BipolarLevel; 16] {
        &self.bends
    }
}
