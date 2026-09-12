//! Off-thread source-action admission and receipt collection.
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
pub(super) struct SourceQueue {
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
    pub(super) fn prepare(
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
    pub(super) fn offer(
        &mut self,
        clock: SampleTime,
        action: SessionSourceAction,
    ) -> Result<SessionSourceId, SessionError> {
        let at = action.at();
        let is_publication = matches!(action, SessionSourceAction::Publish { .. });
        if at < clock
            || self.last_offer.is_some_and(|(last, published)| {
                at < last || (at == last && published && !is_publication)
            })
        {
            return Err(SessionError::SourceOrder);
        }
        if self.held == self.slots.len() {
            return Err(SessionError::Full);
        }
        let serial = self
            .serial
            .checked_add(1)
            .ok_or(SessionError::IdentityExhausted)?;
        let id = SessionSourceId {
            generation: self.generation,
            serial,
        };
        let index = (self.head + self.held) % self.slots.len();
        self.slots[index] = Some(SourceEntry {
            id,
            action,
            outcome: None,
        });
        self.held += 1;
        self.serial = serial;
        self.last_offer = Some((at, is_publication));
        Ok(id)
    }
    pub(super) fn collect(&mut self) -> Option<SessionSourceReceipt> {
        if self.completed == 0 {
            return None;
        }
        let entry = self.slots.get_mut(self.head)?.take()?;
        let Some(outcome) = entry.outcome else {
            self.slots[self.head] = Some(entry);
            return None;
        };
        self.head = (self.head + 1) % self.slots.len();
        self.held -= 1;
        self.completed -= 1;
        Some(SessionSourceReceipt {
            id: entry.id,
            action: entry.action,
            outcome,
        })
    }
    pub(super) fn has_held(&self) -> bool {
        self.held != 0
    }
    pub(super) fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
    pub(super) fn close(&mut self) {
        for offset in self.completed..self.held {
            let index = (self.head + offset) % self.slots.len();
            if let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) {
                entry.outcome = Some(SessionSourceOutcome::Cancelled);
            }
        }
        self.completed = self.held;
    }
}
