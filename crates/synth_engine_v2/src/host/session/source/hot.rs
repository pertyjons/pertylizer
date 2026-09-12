//! Source dispatch retains outcomes in fixed slots.
use super::{SessionSourceAction, SessionSourceOutcome, SourceQueue};
use crate::recording::notes::SimulatedNoteRecorder;
use crate::time::SampleTime;
impl SourceQueue {
    pub(crate) fn drain(
        &mut self,
        recorder: &mut SimulatedNoteRecorder,
        until: SampleTime,
        fences_only: bool,
    ) {
        while self.completed < self.held {
            let index = (self.head + self.completed) % self.slots.len();
            let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) else {
                return;
            };
            // Fence-prefix mode includes only fences exactly at the command time.
            // Full mode excludes the next quantum boundary.
            if if fences_only {
                entry.action.at() != until
                    || !matches!(entry.action, SessionSourceAction::Fence { .. })
            } else {
                entry.action.at() >= until
            } {
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
