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
    #[error("source ring full; retry this observation first if the source remains running")]
    Retry(InputObservation),
    #[error("source halted; retain this observation for shutdown reporting")]
    Halted(InputObservation),
    #[error("source observation invalid before publication: {1}")]
    Invalid(InputObservation, InputError),
}

#[must_use]
pub struct SourceProducer {
    halt: InputCaptureHalt,
    queue: HeapProd<InputObservation>,
    generation: ConnectionGeneration,
    time: Box<SourceTime>,
    failure: Arc<OnceLock<SourceFailure>>,
    retry: Option<InputObservation>,
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
    queue: HeapCons<InputObservation>,
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
                time: Box::new(SourceTime {
                    clock,
                    last_tick: None,
                    last_arrival: None,
                    frontier: SampleTime::ZERO,
                }),
                failure: Arc::clone(&failure),
                retry: None,
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
                + size_of::<OnceLock<SourceFailure>>()
                + size_of::<HeapRb<InputObservation>>()
                + SOURCE_CELLS * size_of::<InputObservation>()
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
    pub fn service(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(InputOfferResult),
    ) {
        self.record_failure(control, &mut receive);
        let prefix = self.queue.occupied_len();
        for _ in 0..prefix {
            let Some(observation) = self.queue.try_pop() else {
                break;
            };
            receive(control.offer(self.generation, observation));
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

    /// A full ring records its returned value as the pending retry. A different
    /// otherwise valid value before that retry is a terminal order fault.
    /// Halt ends publication and returns the original for an explicit report.
    pub fn send(&mut self, observation: InputObservation) -> Result<(), SourceSendError> {
        if self.halt.is_requested() {
            return Err(SourceSendError::Halted(observation));
        }
        let (tick, arrival, nominal) = self
            .check_order(observation)
            .map_err(|error| self.invalid(observation, error))?;
        if self.retry.is_some_and(|pending| pending != observation) {
            return Err(self.invalid(observation, InputError::Order));
        }
        if let Err(original) = self.queue.try_push(observation) {
            self.retry = Some(original);
            return Err(SourceSendError::Retry(original));
        }
        self.retry = None;
        self.time.last_tick = Some(tick);
        if let Some(arrival) = arrival {
            self.time.last_arrival = Some(arrival);
        } else {
            self.time.frontier = nominal;
        }
        Ok(())
    }
}
