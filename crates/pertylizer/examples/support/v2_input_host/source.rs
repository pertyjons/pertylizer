//! Concrete nonblocking producer custody; the merger resolves every popped observation.
use super::*;

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
}

#[must_use]
pub struct SourceProducer {
    halt: InputCaptureHalt,
    queue: HeapProd<InputObservation>,
    generation: ConnectionGeneration,
}
#[must_use]
pub struct SourceInbox {
    generation: ConnectionGeneration,
    queue: HeapCons<InputObservation>,
    closed: bool,
}

impl SourceInbox {
    pub fn prepare(
        generation: ConnectionGeneration,
        halt: InputCaptureHalt,
    ) -> (SourceProducer, Self) {
        let (writer, reader) = HeapRb::new(SOURCE_CELLS).split();
        (
            SourceProducer {
                halt,
                queue: writer,
                generation,
            },
            Self {
                generation,
                queue: reader,
                closed: false,
            },
        )
    }

    /// Charge both queue endpoints and their shared backing; all on non-audio threads.
    pub fn storage_bytes() -> PreparedBytes {
        PreparedBytes::measured(
            (size_of::<Self>()
                + size_of::<SourceProducer>()
                + size_of::<HeapRb<InputObservation>>()
                + SOURCE_CELLS * size_of::<InputObservation>()
                + 256) as u64,
        )
    }

    /// A fixed prefix, including refusals after closure. A refusal returns the
    /// original; an accepted ID with a fault must never be retried.
    pub fn service(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(InputOfferResult),
    ) {
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
    /// inbox must first deliver every queued observation or explicit refusal.
    pub fn close(&mut self, producer: SourceProducer) -> Result<(), SourceProducer> {
        if producer.generation != self.generation || !self.is_empty() {
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
    /// A full ring requires ordered retry; halt ends publication and retains the
    /// original observation for an explicit terminal report.
    pub fn send(&mut self, observation: InputObservation) -> Result<(), SourceSendError> {
        if self.halt.is_requested() {
            return Err(SourceSendError::Halted(observation));
        }
        self.queue
            .try_push(observation)
            .map_err(SourceSendError::Retry)?;
        Ok(())
    }
}
