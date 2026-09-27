//! The bound mixed audio half's quantum-boundary operation.
//! The renderer and ended-note storage are prepared off-thread. This path does
//! not allocate, lock, perform I/O, log or drop an owning candidate.

use super::{
    MixedEffectiveEvents, MixedEffectiveTimeError, MixedOneShotAudio, MixedOneShotRenderError,
    MixedOneShotRenderReport, MixedStreamAudio,
};
use crate::{
    publish::ProducerClass,
    quantities::{EventCount, QuantumCount},
    render::{AudioBlockMut, EventEnvelope, Renderer, TimedEvent},
    time::{PlanPosition, QUANTUM_FRAMES, SampleTime},
};

impl MixedEffectiveEvents<'_> {
    /// The callback's checked read stays inside the purity scan's hot region.
    pub(super) fn shifted(
        &self,
        event_index: usize,
        event: TimedEvent,
    ) -> Result<TimedEvent, MixedEffectiveTimeError> {
        let envelope = event.envelope();
        let time = envelope.time().checked_add(self.shift).map_err(|_| {
            MixedEffectiveTimeError::EventTimeUnrepresentable {
                event_index,
                time: envelope.time(),
                shift: self.shift,
            }
        })?;
        Ok(TimedEvent::new(
            EventEnvelope::new(envelope.epoch(), time, envelope.source()),
            event.payload(),
        ))
    }

    pub(crate) fn get(
        &self,
        event_index: usize,
    ) -> Result<Option<TimedEvent>, MixedEffectiveTimeError> {
        match self.events.get(event_index).copied() {
            Some(event) => self.shifted(event_index, event).map(Some),
            None => Ok(None),
        }
    }
}

impl MixedStreamAudio {
    /// Move only the compiled musical mapping at the current render boundary.
    /// Scratch was sized from the bound compiled span off-thread. The returned
    /// anchor belongs with the retired compiled list; a later schedule owner
    /// must also publish complete scoped restoration in this same quantum.
    #[allow(dead_code)] // Only the private mixed callback calls this so far.
    pub(crate) fn adopt_compiled_boundary(
        &mut self,
        effective: SampleTime,
        position: PlanPosition,
    ) -> Result<crate::render::MixedBoundaryAdopted, crate::render::MixedBoundaryReleaseError> {
        self.renderer.adopt_mixed_compiled_boundary(
            self.partition.compiled_producer(),
            effective,
            position,
            &mut self.compiled_ended,
        )
    }
}

#[allow(dead_code)] // Private callback remains unreachable by production hosts.
impl MixedOneShotAudio {
    /// Read the private owner's state and charges without moving its capsule.
    pub(crate) fn report(&self) -> MixedOneShotRenderReport {
        MixedOneShotRenderReport {
            effective_anchor: self.effective_anchor,
            retired_anchor: self.capsule.retired_anchor,
            sequence: self.in_force,
            adopted: self.adopted,
            faulted: self.fault.is_some(),
            fault: self.fault,
            release_charged: self.release_charged,
            released_compiled: self.released_compiled,
            restoration_charged: self.restoration_charged,
            suffix_charged: self.suffix_charged,
            completed_quanta: self.completed_quanta,
            boundary_quantum_completed: self
                .adoption_after_quanta
                .is_some_and(|at| self.completed_quanta > at),
        }
    }

    /// Render the sealed private schedule in at most one quantum per publication.
    /// Every terminal failure silences the caller's complete block and retains the box.
    pub(crate) fn render_private(
        &mut self,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), MixedOneShotRenderError> {
        if self.fault.is_some() {
            output.silence();
            return Err(MixedOneShotRenderError::Faulted);
        }
        let renderer = &self.audio.renderer;
        if output.layout() != renderer.plan().channel_layout()
            || u64::try_from(output.frames()).map_or(true, |frames| {
                frames > renderer.plan().maximum_block_size().as_u64()
            })
        {
            return Err(MixedOneShotRenderError::OutputShape);
        }
        if output.frames() == 0 {
            return Ok(());
        }
        self.render_started = true;
        let result = self.render_private_inner(&mut output);
        if let Err(error) = result {
            self.fault = Some(error);
            match error {
                MixedOneShotRenderError::Publication(_) => {
                    self.audio.renderer.terminal_fault(&mut output);
                }
                MixedOneShotRenderError::Displacement(_) => {
                    self.audio.renderer.terminal_displacement_fault(&mut output);
                }
                _ => self.audio.renderer.terminal_mixed_fault(&mut output),
            }
            return Err(error);
        }
        Ok(())
    }

    fn render_private_inner(
        &mut self,
        output: &mut AudioBlockMut<'_>,
    ) -> Result<(), MixedOneShotRenderError> {
        let renderer = &self.audio.renderer;
        if renderer.plan().id() != self.capsule.plan
            || renderer.epoch() != self.capsule.epoch
            || renderer.table_id() != self.capsule.table
        {
            return Err(MixedOneShotRenderError::Pairing);
        }
        if renderer.diagnostics().needs_reprepare() {
            return Err(MixedOneShotRenderError::RendererFaulted);
        }
        let mut offset = 0;
        while offset < output.frames() {
            let carry = self.audio.renderer.carry_frames();
            let remaining = output.frames() - offset;
            let frames = remaining.min(if carry == 0 {
                QUANTUM_FRAMES as usize
            } else {
                carry
            });
            let Some(block) = output.window(offset, frames) else {
                return Err(MixedOneShotRenderError::InternalOutputWindow);
            };
            self.render_private_step(block, carry == 0)?;
            offset += frames;
        }
        Ok(())
    }

    fn render_private_step(
        &mut self,
        output: AudioBlockMut<'_>,
        opens_quantum: bool,
    ) -> Result<(), MixedOneShotRenderError> {
        let clock = self.audio.renderer.clock();
        if opens_quantum && !self.adopted && clock == self.timing.effective() {
            let adopted = self.audio.adopt_compiled_boundary(
                self.timing.effective(),
                self.effective_anchor.position(),
            )?;
            self.capsule.retired_anchor = Some(adopted.retired_anchor());
            self.capsule.retired_next = Some(self.next);
            self.released_compiled = adopted.released();
            core::mem::swap(&mut self.events, &mut self.capsule.events);
            self.next = 0;
            self.in_force = self.capsule.sequence;
            self.adopted = true;
            self.adoption_after_quanta = Some(self.completed_quanta);
        }

        let mut publication = self.arbiter.open(clock, usize::from(opens_quantum))?;
        let mut next = self.next;
        if opens_quantum {
            let restoration = if self.adopted {
                self.capsule
                    .restoration_count
                    .as_usize()
                    .ok_or(MixedOneShotRenderError::RestorationCount)?
            } else {
                0
            };
            if self.adopted && !self.release_charged {
                publication.charge_operation(ProducerClass::Session, clock)?;
                self.release_charged = true;
            }
            while let Some(stamped) = self.events.get(next).copied() {
                let event = if self.adopted {
                    MixedEffectiveEvents {
                        events: &self.events,
                        shift: self.timing.shift(),
                    }
                    .get(next)
                    .map_err(MixedOneShotRenderError::Displacement)?
                    .ok_or(MixedOneShotRenderError::MissingCandidateEvent { event_index: next })?
                } else {
                    stamped
                };
                let at = event.envelope().time();
                if at < clock {
                    return Err(MixedOneShotRenderError::MissedEvent { event: at, clock });
                }
                if at.quantum_index() != clock.quantum_index() {
                    break;
                }
                let class = if self.adopted && next < restoration {
                    ProducerClass::Session
                } else {
                    ProducerClass::Compiled
                };
                publication.charge(class, event)?;
                if class == ProducerClass::Session {
                    self.restoration_charged =
                        EventCount::measured(self.restoration_charged.get().saturating_add(1));
                } else if self.adopted {
                    self.suffix_charged =
                        EventCount::measured(self.suffix_charged.get().saturating_add(1));
                }
                next += 1;
            }
            #[cfg(test)]
            if let Some(event) = self.test_live
                && !self.test_live_spent
            {
                let at = event.envelope().time();
                if at < clock {
                    return Err(MixedOneShotRenderError::MissedEvent { event: at, clock });
                }
                if at.quantum_index() == clock.quantum_index() {
                    publication.charge(ProducerClass::Live, event)?;
                    self.test_live_spent = true;
                }
            }
        }
        self.audio
            .renderer
            .render(output, publication.seal().events())?;
        self.next = next;
        if opens_quantum {
            self.completed_quanta =
                QuantumCount::measured(self.completed_quanta.get().saturating_add(1));
        }
        Ok(())
    }
}
