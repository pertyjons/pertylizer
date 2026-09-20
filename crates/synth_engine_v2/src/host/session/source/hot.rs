//! Source dispatch retains outcomes in fixed slots.
use super::{SessionSourceAction, SessionSourceOutcome, SourceQueue};
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
