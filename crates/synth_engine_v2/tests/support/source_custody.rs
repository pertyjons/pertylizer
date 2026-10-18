//! Shared non-shipping source custody used by the example host and private mixed tests.
use super::Midi1Event;
use super::{
    ConnectionGeneration, InputCaptureHalt, InputError, InputObservation, InputTick, PreparedBytes,
    SampleTime, SimulatedInputClock,
};
use ringbuf::HeapRb;
use std::{
    mem::size_of,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
use thiserror::Error;

use super::source_queue::{SourceQueueCommitError, SourceQueueReader, SourceQueueWriter};

pub const SOURCE_CELLS: usize = 16;
/// Ordinary source observations retain the previous 16-place flow limit.
/// Protected key and pedal releases may use the physical cells beyond it.
pub const SOURCE_BACKING_CELLS: usize = 128;
/// Held onsets each owning a protected release place; the next onset is refused.
pub const MAX_RESERVED_NOTES: usize = 64;
const MIDI_CHANNELS: usize = 16;
const MAX_RESERVED_PEDALS: usize = MIDI_CHANNELS;
const _: () =
    assert!(SOURCE_CELLS + MAX_RESERVED_NOTES + MAX_RESERVED_PEDALS <= SOURCE_BACKING_CELLS - 2);

/// A host-owned halt for source producers that no raw owner requests, so raw capture can
/// stop while its sources keep publishing. Requesting it neither allocates nor blocks, so
/// an audio callback may request it.
#[allow(
    dead_code,
    reason = "the example host's sources share its raw owner's halt"
)]
#[derive(Clone, Default)]
#[must_use]
pub struct SourceHostHalt {
    signal: Arc<AtomicBool>,
}

#[allow(
    dead_code,
    reason = "the example host's sources share its raw owner's halt"
)]
impl SourceHostHalt {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&self) {
        self.signal.store(true, Ordering::Release);
    }

    pub fn is_requested(&self) -> bool {
        self.signal.load(Ordering::Acquire)
    }
}

/// What stops a source producer. A producer fault requests it, so it is terminal for the
/// whole host either way.
#[derive(Clone)]
#[must_use]
pub enum SourceHalt {
    /// The raw owner's halt: raw capture interrupting itself also stops the source.
    Raw(InputCaptureHalt),
    /// The host's own halt: only the host stops the source, and a producer fault leaves
    /// the raw owner's halt to the host.
    Host(SourceHostHalt),
}

impl SourceHalt {
    fn is_requested(&self) -> bool {
        match self {
            Self::Raw(halt) => halt.is_requested(),
            Self::Host(halt) => halt.is_requested(),
        }
    }

    fn request_invalid(&self) {
        match self {
            Self::Raw(halt) => halt.request_invalid(),
            Self::Host(halt) => halt.request(),
        }
    }
}

impl From<InputCaptureHalt> for SourceHalt {
    fn from(halt: InputCaptureHalt) -> Self {
        Self::Raw(halt)
    }
}

impl From<SourceHostHalt> for SourceHalt {
    fn from(halt: SourceHostHalt) -> Self {
        Self::Host(halt)
    }
}

#[derive(Debug, Error, PartialEq)]
#[must_use]
pub enum SourceSendError {
    #[cfg(test)]
    #[error(
        "source ordinary queue capacity reached; retry this observation first if the source remains running"
    )]
    Retry(InputObservation),
    #[error("source halted; retain this observation for shutdown reporting")]
    Halted(InputObservation),
    #[error("source observation invalid before publication: {1}")]
    Invalid(InputObservation, InputError),
}

/// A source-local attempt exists before its observation enters the ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct SourceAttemptId {
    generation: ConnectionGeneration,
    serial: u64,
}

impl SourceAttemptId {
    pub fn generation(self) -> ConnectionGeneration {
        self.generation
    }

    pub fn serial(self) -> u64 {
        self.serial
    }
}

/// Custody identity for one observation accepted into a concrete source ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct SourceQueueId {
    generation: ConnectionGeneration,
    serial: u64,
}

impl SourceQueueId {
    pub fn generation(self) -> ConnectionGeneration {
        self.generation
    }

    pub fn serial(self) -> u64 {
        self.serial
    }
}

/// One queue identity and the nominal engine time validated for that packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct SourceQueueStamp {
    queue: SourceQueueId,
    mapped_at: SampleTime,
}

impl SourceQueueStamp {
    pub fn queue(self) -> SourceQueueId {
        self.queue
    }

    pub fn mapped_at(self) -> SampleTime {
        self.mapped_at
    }
}

/// The exact queued original remains paired with its checked stamp and raw result.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct SourceHandoff {
    stamp: SourceQueueStamp,
    observation: InputObservation,
}

impl SourceHandoff {
    pub fn stamp(self) -> SourceQueueStamp {
        self.stamp
    }

    pub fn observation(self) -> InputObservation {
        self.observation
    }
}

/// A source head may be committed only by the consumer that still owns it.
#[derive(Debug, Error, PartialEq)]
#[must_use]
pub enum SourceHeadError {
    #[error("source inbox has no head to commit")]
    Empty,
    #[error("source inbox head changed before commit")]
    Mismatched {
        expected: SourceHandoff,
        actual: SourceHandoff,
    },
}

#[derive(Debug)]
#[must_use]
pub struct SourceAccepted {
    attempt: SourceAttemptId,
    queue: SourceQueueId,
}

impl SourceAccepted {
    pub fn parts(self) -> (SourceAttemptId, SourceQueueId) {
        (self.attempt, self.queue)
    }
}

/// The producer keeps the authoritative pending original if this token is lost.
#[derive(Debug)]
#[must_use]
pub struct SourceRetryToken {
    attempt: SourceAttemptId,
    observation: InputObservation,
}

impl SourceRetryToken {
    pub fn attempt(&self) -> SourceAttemptId {
        self.attempt
    }

    pub fn observation(&self) -> InputObservation {
        self.observation
    }
}

#[derive(Debug, Error)]
#[must_use]
pub enum SourceOwnedSendError {
    #[error("source ordinary queue capacity reached; retry the owned token first")]
    Retry(SourceRetryToken),
    #[error("source halted with an unresolved owned retry token")]
    Halted(SourceRetryToken),
    #[error("retry token does not match this source's pending attempt")]
    Mismatched(SourceRetryToken),
    #[error(transparent)]
    Refused(#[from] SourceSendError),
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct SourceRetiredAttempt {
    attempt: SourceAttemptId,
    observation: InputObservation,
}

impl SourceRetiredAttempt {
    pub fn attempt(self) -> SourceAttemptId {
        self.attempt
    }

    pub fn observation(self) -> InputObservation {
        self.observation
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SourcePacket {
    id: SourceQueueId,
    observation: InputObservation,
    mapped_at: SampleTime,
}

#[derive(Clone, Copy)]
struct PendingOwned {
    retired: SourceRetiredAttempt,
    queue: SourceQueueId,
    tick: InputTick,
    arrival: Option<SampleTime>,
    nominal: SampleTime,
    reservation: ReservationChange,
}

#[derive(Clone, Copy)]
enum ReservationChange {
    Ordinary,
    NoteOn(usize),
    KeyUp(usize),
    PedalDown(usize),
    PedalUp(usize),
}

impl ReservationChange {
    fn is_protected_release(self) -> bool {
        matches!(self, Self::KeyUp(_) | Self::PedalUp(_))
    }
}

struct SourceReservations {
    /// Same-key repeats each own one release place; each key-up spends one.
    keys: [u8; MIDI_CHANNELS * 128],
    pedals: [bool; MAX_RESERVED_PEDALS],
    notes: usize,
}

impl Default for SourceReservations {
    fn default() -> Self {
        Self {
            keys: [0; MIDI_CHANNELS * 128],
            pedals: [false; MAX_RESERVED_PEDALS],
            notes: 0,
        }
    }
}

impl SourceReservations {
    fn change(&self, observation: InputObservation) -> Result<ReservationChange, InputError> {
        let InputObservation::Message { input, .. } = observation else {
            return Ok(ReservationChange::Ordinary);
        };
        let channel = usize::from(input.channel().as_u8() - 1);
        Ok(match input.event() {
            Midi1Event::NoteOn { key, .. } => {
                if self.notes == MAX_RESERVED_NOTES {
                    return Err(InputError::ProtectedCapacity);
                }
                ReservationChange::NoteOn(channel * 128 + key.as_index())
            }
            Midi1Event::KeyRelease { key, .. } => {
                let index = channel * 128 + key.as_index();
                if self.keys[index] == 0 {
                    ReservationChange::Ordinary
                } else {
                    ReservationChange::KeyUp(index)
                }
            }
            Midi1Event::Sustain { down: true } if !self.pedals[channel] => {
                ReservationChange::PedalDown(channel)
            }
            Midi1Event::Sustain { down: false } if self.pedals[channel] => {
                ReservationChange::PedalUp(channel)
            }
            Midi1Event::Sustain { .. } | Midi1Event::PitchBend { .. } => {
                ReservationChange::Ordinary
            }
        })
    }

    fn commit(&mut self, change: ReservationChange) {
        match change {
            ReservationChange::NoteOn(index) => {
                self.keys[index] += 1;
                self.notes += 1;
            }
            ReservationChange::KeyUp(index) => {
                self.keys[index] -= 1;
                self.notes -= 1;
            }
            ReservationChange::PedalDown(channel) => {
                self.pedals[channel] = true;
            }
            ReservationChange::PedalUp(channel) => {
                self.pedals[channel] = false;
            }
            ReservationChange::Ordinary => {}
        }
    }
}

#[must_use]
pub struct SourceProducer {
    halt: Box<SourceHalt>,
    queue: SourceQueueWriter<SourcePacket>,
    generation: ConnectionGeneration,
    serial: u64,
    attempt_serial: u64,
    time: Box<SourceTime>,
    reservations: Box<SourceReservations>,
    failure: Arc<OnceLock<SourceFailure>>,
    retry: Option<InputObservation>,
    owned_retry: Box<Option<PendingOwned>>,
}
#[derive(Clone, Copy)]
struct SourceFailure {
    observation: InputObservation,
    reason: InputError,
}
struct SourceTime {
    clock: SimulatedInputClock,
    last_tick: Option<InputTick>,
    last_arrival: Option<SampleTime>,
    frontier: SampleTime,
}
#[must_use]
pub struct SourceInbox {
    generation: ConnectionGeneration,
    queue: SourceQueueReader<SourcePacket>,
    closed: bool,
    failure: Arc<OnceLock<SourceFailure>>,
    failure_resolved: bool,
}

impl SourceInbox {
    /// `halt` is the raw owner's halt, shared, or a host halt raw capture never requests.
    pub fn prepare(
        generation: ConnectionGeneration,
        halt: impl Into<SourceHalt>,
        clock: SimulatedInputClock,
    ) -> (SourceProducer, Self) {
        let (writer, reader) = SourceQueueWriter::prepare(SOURCE_BACKING_CELLS);
        let failure = Arc::new(OnceLock::new());
        (
            SourceProducer {
                halt: Box::new(halt.into()),
                queue: writer,
                generation,
                serial: 0,
                attempt_serial: 0,
                time: Box::new(SourceTime {
                    clock,
                    last_tick: None,
                    last_arrival: None,
                    frontier: SampleTime::ZERO,
                }),
                reservations: Box::default(),
                failure: Arc::clone(&failure),
                retry: None,
                owned_retry: Box::new(None),
            },
            Self {
                generation,
                queue: reader,
                closed: false,
                failure,
                failure_resolved: false,
            },
        )
    }

    /// Charge both queue endpoints and their shared backing; all on non-audio threads.
    pub fn storage_bytes() -> PreparedBytes {
        PreparedBytes::measured(
            (size_of::<Self>()
                + size_of::<SourceProducer>()
                + size_of::<SourceHalt>()
                + size_of::<SourceTime>()
                + size_of::<SourceReservations>()
                + size_of::<Option<PendingOwned>>()
                + size_of::<OnceLock<SourceFailure>>()
                + size_of::<HeapRb<SourcePacket>>()
                + SOURCE_BACKING_CELLS * size_of::<SourcePacket>()
                + 256) as u64,
        )
    }

    /// Observe the exact queued head without releasing its ring place. The
    /// merger must resolve a producer failure before servicing any head.
    /// A later stage or command refusal may leave it here for a same-identity
    /// retry.
    pub fn peek_handoff(&self) -> Option<SourceHandoff> {
        self.queue.peek().map(|packet| SourceHandoff {
            stamp: SourceQueueStamp {
                queue: packet.id,
                mapped_at: packet.mapped_at,
            },
            observation: packet.observation,
        })
    }

    /// Release one ring place only for the exact head a downstream owner took.
    /// Call after that owner accepts the handoff or records its terminal
    /// disposition, and after resolving producer failure for the service pass.
    /// A wrong or stale handoff changes neither the head nor producer capacity.
    pub fn commit_handoff(
        &mut self,
        expected: SourceHandoff,
    ) -> Result<SourceHandoff, SourceHeadError> {
        let packet = SourcePacket {
            id: expected.stamp.queue,
            observation: expected.observation,
            mapped_at: expected.stamp.mapped_at,
        };
        let packet = match self.queue.commit_exact(packet) {
            Ok(packet) => packet,
            Err(SourceQueueCommitError::Empty) => return Err(SourceHeadError::Empty),
            Err(SourceQueueCommitError::Mismatched { actual }) => {
                return Err(SourceHeadError::Mismatched {
                    expected,
                    actual: SourceHandoff {
                        stamp: SourceQueueStamp {
                            queue: actual.id,
                            mapped_at: actual.mapped_at,
                        },
                        observation: actual.observation,
                    },
                });
            }
        };
        Ok(SourceHandoff {
            stamp: SourceQueueStamp {
                queue: packet.id,
                mapped_at: packet.mapped_at,
            },
            observation: packet.observation,
        })
    }

    pub(super) fn generation(&self) -> ConnectionGeneration {
        self.generation
    }
    pub(super) fn occupied_len(&self) -> usize {
        self.queue.occupied_len()
    }
    pub(super) fn unresolved_failure(&self) -> Option<(InputObservation, InputError)> {
        if self.failure_resolved {
            None
        } else {
            self.failure
                .get()
                .map(|failure| (failure.observation, failure.reason))
        }
    }
    pub(super) fn resolve_failure(&mut self) {
        self.failure_resolved = true;
    }

    /// Only an empty queue AFTER the producer joins permits source acknowledgement.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Consume the unique producer endpoint after its thread returns it. A nonempty
    /// inbox or an unresolved terminal refusal still owns source custody.
    pub fn close(&mut self, producer: SourceProducer) -> Result<(), SourceProducer> {
        if producer.generation != self.generation
            || !self.is_empty()
            || (self.failure.get().is_some() && !self.failure_resolved)
            || producer.owned_retry.is_some()
        {
            return Err(producer);
        }
        self.closed = true;
        Ok(())
    }
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}
impl SourceProducer {
    fn invalid(&self, observation: InputObservation, reason: InputError) -> SourceSendError {
        self.failure.get_or_init(|| SourceFailure {
            observation,
            reason,
        });
        self.halt.request_invalid();
        SourceSendError::Invalid(observation, reason)
    }

    fn check_order(
        &self,
        observation: InputObservation,
    ) -> Result<(InputTick, Option<SampleTime>, SampleTime), InputError> {
        let (tick, arrival) = match observation {
            InputObservation::Message { tick, arrival, .. } => (tick, Some(arrival)),
            InputObservation::Frontier { tick } => (tick, None),
        };
        let nominal = self.time.clock.map(tick)?;
        if self.time.last_tick.is_some_and(|last| tick < last) || nominal < self.time.frontier {
            return Err(InputError::Order);
        }
        if let Some(arrival) = arrival {
            if arrival < nominal {
                return Err(InputError::Future);
            }
            if self.time.last_arrival.is_some_and(|last| arrival < last) {
                return Err(InputError::Order);
            }
        } else if nominal <= self.time.frontier
            || self.time.last_arrival.is_some_and(|last| nominal <= last)
        {
            return Err(InputError::Order);
        }
        Ok((tick, arrival, nominal))
    }

    fn commit_time(&mut self, tick: InputTick, arrival: Option<SampleTime>, nominal: SampleTime) {
        self.time.last_tick = Some(tick);
        if let Some(arrival) = arrival {
            self.time.last_arrival = Some(arrival);
        } else {
            self.time.frontier = nominal;
        }
    }

    fn has_room(&self, change: ReservationChange) -> bool {
        // Each accepted onset and effective pedal-down owns one protected
        // release place. The preparation-time bound above makes every such
        // release fit even while all ordinary places are occupied.
        change.is_protected_release() || self.queue.occupied_len() < SOURCE_CELLS
    }

    /// Test-only value path. A full ring records its returned value as the pending
    /// retry. A different otherwise valid value before it is a terminal order fault.
    /// Halt ends publication and returns the original for an explicit report.
    #[cfg(test)]
    pub fn send(&mut self, observation: InputObservation) -> Result<(), SourceSendError> {
        self.send_identified(observation).map(|_| ())
    }

    /// Test-only value path; mint the queue identity only on a successful push.
    #[cfg(test)]
    pub fn send_identified(
        &mut self,
        observation: InputObservation,
    ) -> Result<SourceQueueId, SourceSendError> {
        if self.halt.is_requested() {
            return Err(SourceSendError::Halted(observation));
        }
        let (tick, arrival, nominal) = self
            .check_order(observation)
            .map_err(|error| self.invalid(observation, error))?;
        if self.owned_retry.is_some() {
            return Err(self.invalid(observation, InputError::Order));
        }
        if self.retry.is_some_and(|pending| pending != observation) {
            return Err(self.invalid(observation, InputError::Order));
        }
        let reservation = self
            .reservations
            .change(observation)
            .map_err(|error| self.invalid(observation, error))?;
        let Some(serial) = self.serial.checked_add(1) else {
            return Err(self.invalid(observation, InputError::IdentityExhausted));
        };
        let id = SourceQueueId {
            generation: self.generation,
            serial,
        };
        if !self.has_room(reservation) {
            self.retry = Some(observation);
            return Err(SourceSendError::Retry(observation));
        }
        if let Err(original) = self.queue.try_push(SourcePacket {
            id,
            observation,
            mapped_at: nominal,
        }) {
            self.retry = Some(original.observation);
            return Err(SourceSendError::Retry(original.observation));
        }
        self.serial = serial;
        self.reservations.commit(reservation);
        self.retry = None;
        self.commit_time(tick, arrival, nominal);
        Ok(id)
    }

    /// A first attempt owns an identity even when a full ring returns its token.
    pub fn submit_owned(
        &mut self,
        observation: InputObservation,
    ) -> Result<SourceAccepted, SourceOwnedSendError> {
        if self.halt.is_requested() {
            return Err(SourceSendError::Halted(observation).into());
        }
        let (tick, arrival, nominal) = self
            .check_order(observation)
            .map_err(|reason| self.invalid(observation, reason))?;
        if self.retry.is_some() || self.owned_retry.is_some() {
            return Err(self.invalid(observation, InputError::Order).into());
        }
        let reservation = self
            .reservations
            .change(observation)
            .map_err(|error| self.invalid(observation, error))?;
        let Some(attempt_serial) = self.attempt_serial.checked_add(1) else {
            return Err(self
                .invalid(observation, InputError::IdentityExhausted)
                .into());
        };
        let Some(queue_serial) = self.serial.checked_add(1) else {
            return Err(self
                .invalid(observation, InputError::IdentityExhausted)
                .into());
        };
        let attempt = SourceAttemptId {
            generation: self.generation,
            serial: attempt_serial,
        };
        let queue = SourceQueueId {
            generation: self.generation,
            serial: queue_serial,
        };
        self.attempt_serial = attempt_serial;
        if !self.has_room(reservation)
            || self
                .queue
                .try_push(SourcePacket {
                    id: queue,
                    observation,
                    mapped_at: nominal,
                })
                .is_err()
        {
            *self.owned_retry = Some(PendingOwned {
                retired: SourceRetiredAttempt {
                    attempt,
                    observation,
                },
                queue,
                tick,
                arrival,
                nominal,
                reservation,
            });
            return Err(SourceOwnedSendError::Retry(SourceRetryToken {
                attempt,
                observation,
            }));
        }
        self.serial = queue_serial;
        self.reservations.commit(reservation);
        self.commit_time(tick, arrival, nominal);
        Ok(SourceAccepted { attempt, queue })
    }

    /// A wrong token changes neither source; the caller can return it to its owner.
    pub fn retry_owned(
        &mut self,
        token: SourceRetryToken,
    ) -> Result<SourceAccepted, SourceOwnedSendError> {
        let Some(pending) = *self.owned_retry else {
            return Err(SourceOwnedSendError::Mismatched(token));
        };
        if pending.retired.attempt != token.attempt()
            || pending.retired.observation != token.observation()
        {
            return Err(SourceOwnedSendError::Mismatched(token));
        }
        if self.halt.is_requested() {
            return Err(SourceOwnedSendError::Halted(token));
        }
        if !self.has_room(pending.reservation)
            || self
                .queue
                .try_push(SourcePacket {
                    id: pending.queue,
                    observation: pending.retired.observation,
                    mapped_at: pending.nominal,
                })
                .is_err()
        {
            return Err(SourceOwnedSendError::Retry(token));
        }
        self.serial = pending.queue.serial;
        self.reservations.commit(pending.reservation);
        *self.owned_retry = None;
        self.commit_time(pending.tick, pending.arrival, pending.nominal);
        Ok(SourceAccepted {
            attempt: token.attempt(),
            queue: pending.queue,
        })
    }

    fn record_retirement(&self, pending: SourceRetiredAttempt) {
        self.failure.get_or_init(|| SourceFailure {
            observation: pending.observation,
            reason: InputError::SourceQueueFull,
        });
        self.halt.request_invalid();
    }

    /// Terminally retire a matching token. The original remains in the producer
    /// until this call, so loss of the token cannot silently release custody.
    pub fn retire_owned(
        &mut self,
        token: SourceRetryToken,
    ) -> Result<SourceRetiredAttempt, SourceRetryToken> {
        match *self.owned_retry {
            Some(pending)
                if pending.retired.attempt == token.attempt()
                    && pending.retired.observation == token.observation() =>
            {
                *self.owned_retry = None;
                self.record_retirement(pending.retired);
                Ok(pending.retired)
            }
            _ => Err(token),
        }
    }

    /// Joined recovery can report the original even if a worker lost its token.
    pub fn retire_pending(&mut self) -> Option<SourceRetiredAttempt> {
        let pending = self.owned_retry.take()?;
        self.record_retirement(pending.retired);
        Some(pending.retired)
    }

    #[cfg(test)]
    pub(super) fn set_serial_in_core_for_test(&mut self, serial: u64) {
        self.serial = serial;
    }

    #[cfg(test)]
    pub(super) fn set_attempt_serial_in_core_for_test(&mut self, serial: u64) {
        self.attempt_serial = serial;
    }
}
