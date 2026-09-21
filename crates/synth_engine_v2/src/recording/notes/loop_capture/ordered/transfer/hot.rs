//! Fixed-cell packet delivery and completion moves. No queue or thread implementation lives here.
use super::LoopSessionError;
use super::{
    Action, Delivery, LoopRecordingAudio, LoopTransferCompletion, LoopTransferError,
    LoopTransferOutcome, LoopTransferPacket, Pending,
};
use crate::{looping::LoopSnapshot, render::AudioBlockMut};

#[derive(Clone, Copy)]
enum ReadyReceipt {
    Command(crate::host::session::SessionCommandId),
    Source(crate::host::session::SessionSourceId),
}

impl LoopRecordingAudio {
    /// Reconcile mutable raw metadata before returning a live outcome credit.
    pub fn resolve_audition(
        &mut self,
        id: crate::host::live::AuditionId,
        outcome: crate::host::live::AuditionOutcome,
    ) -> Result<crate::quantities::EventCount, LoopSessionError> {
        self.session.resolve_audition(id, outcome)
    }

    pub(crate) const fn is_closed(&self) -> bool {
        self.session.commands.closed
    }

    pub(crate) fn halt(
        &mut self,
        reason: crate::recording::notes::CaptureStopReason,
    ) -> Result<(), LoopSessionError> {
        if !self.is_closed() {
            self.session.interrupt(reason)?;
        }
        Ok(())
    }

    pub fn applied_end(
        &self,
    ) -> Option<(
        crate::time::SampleTime,
        crate::host::session::SessionCommand,
    )> {
        self.session.applied_end()
    }

    pub const fn acknowledged(&self) -> LoopSnapshot {
        self.session.acknowledged()
    }

    /// Admit exactly the concrete host's captured FIFO prefix before rendering.
    /// Protocol refusal returns ownership; semantic refusal becomes a retained completion.
    pub fn enqueue(
        &mut self,
        packet: LoopTransferPacket,
    ) -> Result<(), (LoopTransferPacket, LoopTransferError)> {
        if packet.id.generation != self.generation {
            return Err((packet, LoopTransferError::Origin));
        }
        if self.session.commands.closed {
            return Err((packet, LoopTransferError::Closed));
        }
        if packet.id.serial <= self.last_serial {
            return Err((packet, LoopTransferError::Order));
        }
        let mut free = None;
        for (index, slot) in self.pending.iter().enumerate() {
            if slot.is_none() {
                free = Some(index);
                break;
            }
        }
        let Some(slot) = free.and_then(|index| self.pending.get_mut(index)) else {
            return Err((packet, LoopTransferError::Full));
        };
        let delivery = match packet.action {
            Action::Command(at, command) => match self.session.offer(at, command) {
                Ok(id) => Delivery::Command(id),
                Err(error) => Delivery::Refused(error),
            },
            Action::Source(action) => match self.session.offer_source(action) {
                Ok(id) => Delivery::Source(id),
                Err(error) => Delivery::Refused(error),
            },
        };
        self.last_serial = packet.id.serial;
        *slot = Some(Pending { packet, delivery });
        Ok(())
    }

    pub fn render(&mut self, output: AudioBlockMut<'_>) -> Result<(), LoopSessionError> {
        self.session.render(output)
    }

    /// A stopped source worker may make progress without another audio callback.
    /// The caller still needs exclusive ownership and the usual bounded ingress cut.
    pub fn drain_stopped_sources(&mut self) -> Result<(), LoopSessionError> {
        self.session.drain_stopped_sources()
    }

    /// Check destination capacity before taking; retain any failed queue push.
    /// The control credit remains outstanding while the completion is in transit.
    pub fn take_completed(&mut self) -> Option<LoopTransferCompletion> {
        // A refused delivery owns no serial slot. Every other variant is restored intact.
        for index in 0..self.pending.len() {
            let slot = self.pending.get_mut(index)?;
            if slot
                .as_ref()
                .is_some_and(|entry| matches!(entry.delivery, Delivery::Refused(_)))
            {
                match slot.take() {
                    Some(Pending {
                        packet,
                        delivery: Delivery::Refused(error),
                    }) => {
                        return Some(LoopTransferCompletion {
                            packet,
                            outcome: LoopTransferOutcome::Refused(error),
                            snapshot: self.acknowledged(),
                        });
                    }
                    other => *slot = other,
                }
            }
        }
        // Inspect the ready ID before consuming either authority. A missing mapping
        // must retain the serial receipt as well as its outstanding control credit.
        let ready = if let Some(id) = self.session.next_command_receipt_id() {
            ReadyReceipt::Command(id)
        } else {
            ReadyReceipt::Source(self.session.next_source_receipt_id()?)
        };
        for index in 0..self.pending.len() {
            let slot = self.pending.get_mut(index)?;
            let matches = slot
                .as_ref()
                .is_some_and(|entry| match (&entry.delivery, ready) {
                    (Delivery::Command(id), ReadyReceipt::Command(ready)) => *id == ready,
                    (Delivery::Source(id), ReadyReceipt::Source(ready)) => *id == ready,
                    _ => false,
                });
            if !matches {
                continue;
            }
            let Some(entry) = slot.take() else {
                continue;
            };
            let receipt = match ready {
                ReadyReceipt::Command(_) => self
                    .session
                    .take_command_receipt()
                    .map(LoopTransferOutcome::Command),
                ReadyReceipt::Source(_) => self
                    .session
                    .take_source_receipt()
                    .map(LoopTransferOutcome::Source),
            };
            let Some(receipt) = receipt else {
                *slot = Some(entry);
                return None;
            };
            return Some(LoopTransferCompletion {
                packet: entry.packet,
                outcome: receipt,
                snapshot: self.acknowledged(),
            });
        }
        None // Both authorities remain retained even if their private mapping is inconsistent.
    }
}
