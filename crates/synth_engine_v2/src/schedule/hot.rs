//! The compiled scheduler's audio-thread path.
//!
//! Like `render/hot.rs`, this file may not allocate, lock, perform I/O, log, or panic.
//! The prepared list is immutable; only its cursor advances after a successful render.

use super::{CompiledEventScheduler, ScheduledRenderError};
use crate::render::{AudioBlockMut, PreparedRenderer, Renderer, TimedEvents};

impl CompiledEventScheduler {
    /// Render one actual host block with the prepared, bounded events its quanta contain.
    pub fn render(
        &mut self,
        renderer: &mut PreparedRenderer,
        output: AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        if self.epoch != renderer.epoch() {
            return Err(ScheduledRenderError::EpochMismatch {
                schedule: self.epoch,
                renderer: renderer.epoch(),
            });
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

        let due = self.events.get(self.next..end).unwrap_or(&[]);
        if let Err(error) = renderer.render(output, TimedEvents::new(due)) {
            return Err(ScheduledRenderError::Render(error));
        }
        self.next = end;
        Ok(())
    }
}
