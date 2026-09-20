//! Fixed-storage source actions; preparation is off-thread and receipt moves are hot-safe.
mod hot;
mod types;
use super::SessionError;
use crate::host::ConnectionGeneration;
use crate::quantities::PreparedBytes;
use crate::time::SampleTime;
pub use types::*;

struct SourceEntry {
    id: SessionSourceId,
    action: SessionSourceAction,
    outcome: Option<SessionSourceOutcome>,
}
pub(crate) struct SourceQueue {
    generation: ConnectionGeneration,
    slots: Box<[Option<SourceEntry>]>,
    head: usize,
    held: usize,
    completed: usize,
    serial: u64,
    last_offer: Option<(SampleTime, bool)>,
    bytes: PreparedBytes,
}
impl SourceQueue {
    pub(crate) fn prepare(
        generation: ConnectionGeneration,
        limits: SessionSourceLimits,
    ) -> Result<Self, SessionError> {
        let count = usize::try_from(limits.actions.as_u32()).map_err(|_| SessionError::Layout)?;
        let bytes = count
            .checked_mul(size_of::<Option<SourceEntry>>())
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .ok_or(SessionError::Layout)?;
        let required = PreparedBytes::measured(bytes as u64);
        if required > limits.bytes {
            return Err(SessionError::ByteBudget {
                required,
                available: limits.bytes,
            });
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| SessionError::Allocation)?;
        slots.resize_with(count, || None);
        Ok(Self {
            generation,
            slots: slots.into_boxed_slice(),
            head: 0,
            held: 0,
            completed: 0,
            serial: 0,
            last_offer: None,
            bytes: required,
        })
    }
    pub(crate) fn collect(&mut self) -> Option<SessionSourceReceipt> {
        self.take_receipt()
    }
    pub(crate) fn capacity(&self) -> usize {
        self.slots.len()
    }
    pub(crate) fn has_held(&self) -> bool {
        self.held != 0
    }
    pub(crate) fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
}
