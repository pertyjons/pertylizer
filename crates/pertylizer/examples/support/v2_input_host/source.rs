//! Concrete nonblocking producer custody; the merger resolves every popped observation.
use super::*;
use std::sync::OnceLock;
use synth_engine_v2::host::input::{InputTick, SimulatedInputClock};

const SOURCE_CELLS: usize = 16;
pub const SOURCE_OUTSTANDING: synth_engine_v2::quantities::EventCount =
    synth_engine_v2::quantities::EventCount::measured(64);

#[derive(Debug, Error, PartialEq)]
#[must_use]
pub enum SourceSendError {
    #[cfg(test)]
    #[error("source ring full; retry this observation first if the source remains running")]
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
    #[error("source ring full; retry the owned token first")]
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

#[derive(Clone, Copy)]
struct SourcePacket {
    id: SourceQueueId,
    observation: InputObservation,
}

#[derive(Clone, Copy)]
struct PendingOwned {
    retired: SourceRetiredAttempt,
    queue: SourceQueueId,
    tick: InputTick,
    arrival: Option<SampleTime>,
    nominal: SampleTime,
}

#[must_use]
pub struct SourceProducer {
    halt: InputCaptureHalt,
    queue: HeapProd<SourcePacket>,
    generation: ConnectionGeneration,
    serial: u64,
    attempt_serial: u64,
    time: Box<SourceTime>,
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
    queue: HeapCons<SourcePacket>,
    closed: bool,
    failure: Arc<OnceLock<SourceFailure>>,
    failure_resolved: bool,
}

impl SourceInbox {
    pub fn prepare(
        generation: ConnectionGeneration,
        halt: InputCaptureHalt,
        clock: SimulatedInputClock,
    ) -> (SourceProducer, Self) {
        let (writer, reader) = HeapRb::new(SOURCE_CELLS).split();
        let failure = Arc::new(OnceLock::new());
        (
            SourceProducer {
                halt,
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
                + size_of::<SourceTime>()
                + size_of::<Option<PendingOwned>>()
                + size_of::<OnceLock<SourceFailure>>()
                + size_of::<HeapRb<SourcePacket>>()
                + SOURCE_CELLS * size_of::<SourcePacket>()
                + 256) as u64,
        )
    }

    /// Register a terminal producer fault before any source queue is serviced.
    pub fn record_failure(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(InputOfferResult),
    ) {
        if !self.failure_resolved
            && let Some(failure) = self.failure.get().copied()
        {
            match control.source_failed_before_ring(
                self.generation,
                failure.observation,
                failure.reason,
            ) {
                Ok(()) => self.failure_resolved = true,
                Err(error) => {
                    receive(Err(InputOfferError::Refused(failure.observation, error)));
                    self.failure_resolved = true;
                }
            }
        }
    }

    /// A fixed prefix, including refusals after closure. A refusal returns the
    /// original; an accepted ID with a fault must never be retried.
    #[cfg(test)]
    pub fn service(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(InputOfferResult),
    ) {
        self.service_identified(control, |_, result| receive(result));
    }

    /// A queue ID accompanies every popped observation, including raw refusals.
    /// A pre-ring failure has no queue ID.
    pub fn service_identified(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(Option<SourceQueueId>, InputOfferResult),
    ) {
        self.record_failure(control, |result| receive(None, result));
        let prefix = self.queue.occupied_len();
        for _ in 0..prefix {
            let Some(packet) = self.queue.try_pop() else {
                break;
            };
            receive(
                Some(packet.id),
                control.offer(self.generation, packet.observation),
            );
        }
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
        let Some(serial) = self.serial.checked_add(1) else {
            return Err(self.invalid(observation, InputError::IdentityExhausted));
        };
        let id = SourceQueueId {
            generation: self.generation,
            serial,
        };
        if let Err(original) = self.queue.try_push(SourcePacket { id, observation }) {
            self.retry = Some(original.observation);
            return Err(SourceSendError::Retry(original.observation));
        }
        self.serial = serial;
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
        if let Err(original) = self.queue.try_push(SourcePacket {
            id: queue,
            observation,
        }) {
            *self.owned_retry = Some(PendingOwned {
                retired: SourceRetiredAttempt {
                    attempt,
                    observation: original.observation,
                },
                queue,
                tick,
                arrival,
                nominal,
            });
            return Err(SourceOwnedSendError::Retry(SourceRetryToken {
                attempt,
                observation: original.observation,
            }));
        }
        self.serial = queue_serial;
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
        if self
            .queue
            .try_push(SourcePacket {
                id: pending.queue,
                observation: pending.retired.observation,
            })
            .is_err()
        {
            return Err(SourceOwnedSendError::Retry(token));
        }
        self.serial = pending.queue.serial;
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
    pub(super) fn set_serial_for_test(&mut self, serial: u64) {
        self.serial = serial;
    }

    #[cfg(test)]
    pub(super) fn set_attempt_serial_for_test(&mut self, serial: u64) {
        self.attempt_serial = serial;
    }
}
