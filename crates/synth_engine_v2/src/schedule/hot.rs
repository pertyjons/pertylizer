//! The compiled scheduler's audio-thread path.
//!
//! Like `render/hot.rs`, this file may not allocate, lock, perform I/O, log, or panic. The
//! prepared list is immutable; only its cursor advances after a successful render.
//!
//! **Compiled events reach the renderer through the arbiter, not around it.** An earlier
//! revision handed the renderer a borrowed slice of the prepared list directly. That made
//! ADR-0046 clause 2's "the only normal path that constructs renderer input" false for the
//! one producer Phase 2 already had, and it made an equivalence test between this path and
//! a published one meaningless — the two would not share a boundary. Every due event is now
//! charged to `ProducerClass::Compiled` and the sealed batch is what the renderer sees.

use super::{CompiledEventScheduler, ScheduledRenderError};
use crate::publish::{ProducerClass, PublicationArbiter};
use crate::render::{AudioBlockMut, PreparedRenderer, Renderer};

impl CompiledEventScheduler {
    /// Render one actual host block with the prepared, bounded events its quanta contain.
    ///
    /// The arbiter is a parameter rather than owned state because ADR-0046 clause 2 admits
    /// exactly **one** of them: every producer for a stream publishes into the same store,
    /// and a scheduler holding its own would be a second arbiter wearing another name.
    pub fn render(
        &mut self,
        renderer: &mut PreparedRenderer,
        arbiter: &mut PublicationArbiter,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        if self.epoch != renderer.epoch() {
            return Err(ScheduledRenderError::EpochMismatch {
                schedule: self.epoch,
                renderer: renderer.epoch(),
            });
        }

        // A faulted epoch publishes nothing further. Without this the scheduler would call
        // `terminal_fault` again on every retry, incrementing the attributable counter past
        // one and making "the stream ended once, for this reason" unreadable. `render`'s own
        // first check catches a retry that reaches it, but publication happens before that.
        if renderer.diagnostics().needs_reprepare() {
            // Delegating rather than filling here: `render`'s own first check silences the
            // block and returns the same error, and duplicating that would be a second
            // place for the two to disagree about what a dead epoch looks like.
            return renderer
                .render(output, crate::render::TimedEvents::EMPTY)
                .map_err(ScheduledRenderError::Render);
        }

        // The arbiter this schedule adopted, latched on first use. A second store
        // would pass every capacity bound while restarting the high-water history.
        match self.arbiter {
            Some(latched) if latched != arbiter.id() => {
                return Err(ScheduledRenderError::ForeignArbiter {
                    latched,
                    offered: arbiter.id(),
                });
            }
            Some(_) => {}
            None => self.arbiter = Some(arbiter.id()),
        }

        let clock = renderer.clock();
        if let Some(event) = self.events.get(self.next)
            && event.envelope().time() < clock
        {
            return Err(ScheduledRenderError::MissedEvent {
                event: event.envelope().time(),
                clock,
            });
        }

        let quanta = renderer.quanta_needed_for(output.frames());
        let admissible = self
            .max_events_per_quantum
            .as_usize()
            .unwrap_or(0)
            .saturating_mul(quanta);
        let Ok(quantum_count) = u64::try_from(quanta) else {
            return Err(ScheduledRenderError::CallSpanUnrepresentable { clock });
        };
        let Some(end_quantum) = clock.quantum_index().checked_add(quantum_count) else {
            return Err(ScheduledRenderError::CallSpanUnrepresentable { clock });
        };

        let mut end = self.next;
        while let Some(event) = self.events.get(end) {
            if event.envelope().time().quantum_index() >= end_quantum {
                break;
            }
            if end.saturating_sub(self.next) >= admissible {
                return Err(ScheduledRenderError::EventWindowTooDense {
                    clock,
                    presented_at_least: admissible.saturating_add(1),
                    admissible,
                });
            }
            end = end.saturating_add(1);
        }

        // Publication first, and a failure here ends the stream rather than the call.
        // Clause 7 is explicit that a share overrun is a contract violation and not a load
        // condition, so there is nothing to retry and the terminal response is taken before
        // the caller learns about it.
        let mut publication = match arbiter.open(clock, quanta) {
            Ok(publication) => publication,
            Err(fault) => {
                renderer.terminal_fault(&mut output);
                return Err(ScheduledRenderError::Publication(fault));
            }
        };

        let mut index = self.next;
        while index < end {
            let Some(event) = self.events.get(index).copied() else {
                break;
            };
            if let Err(fault) = publication.charge(ProducerClass::Compiled, event) {
                renderer.terminal_fault(&mut output);
                return Err(ScheduledRenderError::Publication(fault));
            }
            index = index.saturating_add(1);
        }

        let batch = publication.seal();
        if let Err(error) = renderer.render(output, batch.events()) {
            return Err(ScheduledRenderError::Render(error));
        }
        self.next = end;
        Ok(())
    }
}
