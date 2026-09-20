//! Source dispatch retains outcomes in fixed slots.
use super::{
    SessionError, SessionSourceAction, SessionSourceId, SessionSourceOutcome, SessionSourceReceipt,
    SourceEntry, SourceQueue,
};
use crate::recording::notes::SimulatedNoteRecorder;
use crate::time::SampleTime;

#[derive(Clone, Copy)]
enum SourceCut {
    Before(SampleTime),
    FencesAt(SampleTime),
    Stopped,
}

impl SourceCut {
    fn includes(self, action: SessionSourceAction) -> bool {
        match self {
            Self::Before(until) => action.at() < until,
            Self::FencesAt(at) => {
                action.at() == at && matches!(action, SessionSourceAction::Fence { .. })
            }
            Self::Stopped => true,
        }
    }
}

impl SourceQueue {
    pub(crate) fn close(&mut self) {
        for offset in self.completed..self.held {
            let index = (self.head + offset) % self.slots.len();
            if let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) {
                entry.outcome = Some(SessionSourceOutcome::Cancelled);
            }
        }
        self.completed = self.held;
    }

    pub(crate) fn drain(
        &mut self,
        recorder: &mut SimulatedNoteRecorder,
        until: SampleTime,
        fences_only: bool,
    ) {
        let cut = if fences_only {
            SourceCut::FencesAt(until)
        } else {
            SourceCut::Before(until)
        };
        self.drain_prefix(recorder, cut);
    }

    /// After an ordered Stop, source progress is independent of the audio clock.
    /// Drain the finite admitted FIFO; each explicit action retains its own authority.
    pub(crate) fn drain_stopped(&mut self, recorder: &mut SimulatedNoteRecorder) {
        self.drain_prefix(recorder, SourceCut::Stopped);
    }

    fn drain_prefix(&mut self, recorder: &mut SimulatedNoteRecorder, cut: SourceCut) {
        while self.completed < self.held {
            let index = (self.head + self.completed) % self.slots.len();
            let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) else {
                return;
            };
            if !cut.includes(entry.action) {
                break;
            }
            let outcome = match entry.action {
                SessionSourceAction::Publish {
                    source,
                    stamp,
                    input,
                    audition,
                } => match recorder.publish(source, stamp, input, audition) {
                    Ok(receipt) => SessionSourceOutcome::Published(receipt),
                    Err(error) => SessionSourceOutcome::Refused(error),
                },
                SessionSourceAction::Fence {
                    source,
                    epoch,
                    frontier,
                } => {
                    match recorder
                        .source_sequence(source)
                        .and_then(|through| recorder.fence(source, epoch, frontier, through))
                    {
                        Ok(()) => SessionSourceOutcome::Fenced,
                        Err(error) => SessionSourceOutcome::Refused(error),
                    }
                }
            };
            entry.outcome = Some(outcome);
            self.completed += 1;
        }
    }
}

impl SourceQueue {
    pub(crate) fn offer(
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
    pub(crate) fn next_receipt_id(&self) -> Option<SessionSourceId> {
        if self.completed == 0 {
            return None;
        }
        let entry = self.slots.get(self.head)?.as_ref()?;
        entry.outcome.as_ref()?;
        Some(entry.id)
    }
    pub(crate) fn take_receipt(&mut self) -> Option<SessionSourceReceipt> {
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
}
