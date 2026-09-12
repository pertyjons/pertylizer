//! Copy successful render observations into fixed, never-reused journal cells.

use super::{JournaledLoopStream, LoopJournalEnd, LoopJournalEndReason};
use crate::{looping::LoopFault, render::AudioBlockMut};

impl JournaledLoopStream {
    /// Retain boundaries only after the whole call succeeds. A retryable output
    /// shape refusal changes nothing; a terminal fault closes at the previous
    /// successful frontier, even if the failed renderer partially advanced.
    pub fn render(&mut self, output: AudioBlockMut<'_>) -> Result<(), LoopFault> {
        match self.stream.render(output) {
            Ok(()) => {
                if self.end.is_none() {
                    for boundary in self.stream.boundaries() {
                        if let Some(cell) = self.boundaries.get_mut(self.boundary_len) {
                            *cell = Some(*boundary);
                            self.boundary_len += 1;
                        } else {
                            self.end = Some(LoopJournalEnd {
                                epoch: boundary.epoch,
                                at: boundary.at,
                                pass: boundary.previous,
                                reason: LoopJournalEndReason::PassLimit,
                            });
                            break;
                        }
                    }
                }
                self.acknowledged = self.stream.snapshot();
                Ok(())
            }
            Err(error) => {
                if let Some(fault) = self.stream.fault() {
                    let _retained_end = self.close(LoopJournalEndReason::RenderFault(fault));
                }
                Err(error)
            }
        }
    }

    /// End observation at the last successful frontier, without requiring another
    /// render call. The exclusive owner may call this after its callback joins.
    /// It does not assert a source fence or turn rendered time into delivered time.
    pub fn finish(&mut self) -> LoopJournalEnd {
        self.close(LoopJournalEndReason::Finished)
    }

    fn close(&mut self, reason: LoopJournalEndReason) -> LoopJournalEnd {
        if let Some(end) = self.end {
            return end;
        }
        let end = LoopJournalEnd {
            epoch: self.acknowledged.epoch,
            at: self.acknowledged.clock,
            pass: self.acknowledged.pass,
            reason,
        };
        self.end = Some(end);
        end
    }
}
