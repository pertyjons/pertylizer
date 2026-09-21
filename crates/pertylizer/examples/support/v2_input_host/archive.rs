//! Bounded off-thread ownership of earlier takes across explicit fresh attempts.
use super::*;
use synth_engine_v2::{quantities::CaptureResultCount, time::StreamEpoch};

enum Slot {
    Reserved(StreamEpoch),
    Retained(Box<InputCaptureSession>),
}
#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("retained-take count or byte ceiling exceeded")]
    Full,
    #[error("attempt epoch is already reserved or retained")]
    Duplicate,
    #[error("attempt has no reservation in this archive")]
    Foreign,
    #[error("take still owns unresolved results or sources")]
    Unresolved,
    #[error("retained-take storage allocation failed")]
    Allocation,
}
#[must_use]
pub struct RetainedRuns {
    slots: Box<[Option<Slot>]>,
    bytes: PreparedBytes,
    per_attempt: PreparedBytes,
}
impl RetainedRuns {
    /// `per_attempt` is the host's aggregate admitted ceiling for one complete
    /// renderer, journal, recorder, input owners and their transfer storage.
    /// One extra candidate is charged for preparation; reserve its epoch before
    /// starting it. A full archive refuses activation and retains earlier results.
    pub fn prepare(
        count: CaptureResultCount,
        per_attempt: PreparedBytes,
        ceiling: PreparedBytes,
    ) -> Result<Self, ArchiveError> {
        let count = usize::try_from(count.get()).map_err(|_| ArchiveError::Full)?;
        let inline = count
            .checked_mul(size_of::<Option<Slot>>() + size_of::<InputCaptureSession>())
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(ArchiveError::Full)?;
        let bytes = per_attempt
            .get()
            .checked_mul(count as u64 + 1)
            .and_then(|n| n.checked_add(inline))
            .filter(|n| *n <= ceiling.get())
            .map(PreparedBytes::measured)
            .ok_or(ArchiveError::Full)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| ArchiveError::Allocation)?;
        // Include any allocator overgrant in the declared descriptor ceiling.
        if slots.capacity() != count {
            return Err(ArchiveError::Full);
        }
        slots.resize_with(count, || None);
        Ok(Self {
            slots: slots.into_boxed_slice(),
            bytes,
            per_attempt,
        })
    }
    pub const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
    fn reserve(&mut self, epoch: StreamEpoch) -> Result<(), ArchiveError> {
        if self.slots.iter().flatten().any(|slot| match slot {
            Slot::Reserved(id) => *id == epoch,
            Slot::Retained(owner) => owner.acknowledged().epoch == epoch,
        }) {
            return Err(ArchiveError::Duplicate);
        }
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ArchiveError::Full)?;
        *slot = Some(Slot::Reserved(epoch));
        Ok(())
    }

    #[cfg(test)]
    pub fn reserve_for_test(&mut self, epoch: StreamEpoch) -> Result<(), ArchiveError> {
        self.reserve(epoch)
    }

    pub fn admit(
        &mut self,
        prepared: &super::prepare::PreparedAttempt,
    ) -> Result<(), ArchiveError> {
        if prepared.bytes() > self.per_attempt {
            return Err(ArchiveError::Full);
        }
        self.reserve(prepared.epoch())
    }

    /// Preparation failure has no running callback and releases only its reservation.
    pub fn cancel_preparation(&mut self, epoch: StreamEpoch) -> Result<(), ArchiveError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| matches!(slot, Some(Slot::Reserved(id)) if *id == epoch))
            .ok_or(ArchiveError::Foreign)?;
        *slot = None;
        Ok(())
    }
    pub fn retain(
        &mut self,
        owner: InputCaptureSession,
    ) -> Result<(), Box<(InputCaptureSession, ArchiveError)>> {
        if !owner.retirement_ready() || owner.result().is_err() {
            return Err(Box::new((owner, ArchiveError::Unresolved)));
        }
        let epoch = owner.acknowledged().epoch;
        let Some(slot) = self
            .slots
            .iter_mut()
            .find(|slot| matches!(slot, Some(Slot::Reserved(id)) if *id == epoch))
        else {
            return Err(Box::new((owner, ArchiveError::Foreign)));
        };
        *slot = Some(Slot::Retained(Box::new(owner)));
        Ok(())
    }
    /// Collection transfers ownership; no implicit discard or re-use of a take's IDs.
    pub fn take(&mut self, epoch: StreamEpoch) -> Option<InputCaptureSession> {
        let slot = self.slots.iter_mut().find(|slot| matches!(slot, Some(Slot::Retained(owner)) if owner.acknowledged().epoch == epoch))?;
        match slot.take()? {
            Slot::Retained(owner) => Some(*owner),
            Slot::Reserved(_) => None,
        }
    }
}
