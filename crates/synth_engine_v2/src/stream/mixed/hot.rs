//! The bound mixed audio half's quantum-boundary operation.
//! The renderer, ended-note storage, both range minters and the compiled token table
//! are prepared off-thread. This path mints and releases compiled occurrences in that
//! prepared storage only; it does not allocate, lock, perform I/O, log or drop an
//! owning candidate.

use super::{
    MixedCompiledEvent, MixedCompiledPayload, MixedEffectiveEvents, MixedEffectiveTimeError,
    MixedNoteToken, MixedOneShotAudio, MixedOneShotRenderError, MixedOneShotRenderReport,
    MixedStreamAudio,
};
use crate::{
    identity::{NoteIdentity, Resolution},
    publish::ProducerClass,
    quantities::{EventCount, HeldNoteCount, QuantumCount},
    render::{AudioBlockMut, EventEnvelope, EventPayload, NoteEdge, Renderer, TimedEvent},
    time::{PlanPosition, QUANTUM_FRAMES, SampleTime},
};

impl MixedNoteToken {
    pub(crate) fn as_usize(self) -> usize {
        usize::from(self.0)
    }
}

impl MixedCompiledEvent {
    pub(crate) const fn new(envelope: EventEnvelope, payload: MixedCompiledPayload) -> Self {
        Self { envelope, payload }
    }

    pub(crate) const fn envelope(&self) -> EventEnvelope {
        self.envelope
    }

    pub(crate) const fn payload(&self) -> MixedCompiledPayload {
        self.payload
    }
}

impl MixedEffectiveEvents<'_> {
    /// The callback's checked read stays inside the purity scan's hot region.
    pub(super) fn shifted(
        &self,
        event_index: usize,
        event: MixedCompiledEvent,
    ) -> Result<MixedCompiledEvent, MixedEffectiveTimeError> {
        let envelope = event.envelope();
        let time = envelope.time().checked_add(self.shift).map_err(|_| {
            MixedEffectiveTimeError::EventTimeUnrepresentable {
                event_index,
                time: envelope.time(),
                shift: self.shift,
            }
        })?;
        Ok(MixedCompiledEvent::new(
            EventEnvelope::new(envelope.epoch(), time, envelope.source()),
            event.payload(),
        ))
    }

    pub(crate) fn get(
        &self,
        event_index: usize,
    ) -> Result<Option<MixedCompiledEvent>, MixedEffectiveTimeError> {
        match self.events.get(event_index).copied() {
            Some(event) => self.shifted(event_index, event).map(Some),
            None => Ok(None),
        }
    }
}

impl MixedStreamAudio {
    /// The occurrence an open compiled token names.
    fn compiled_identity(
        &self,
        token: MixedNoteToken,
    ) -> Result<NoteIdentity, MixedOneShotRenderError> {
        self.compiled_tokens
            .get(token.as_usize())
            .copied()
            .flatten()
            .ok_or(MixedOneShotRenderError::CompiledToken { token })
    }

    /// Give one tokenized compiled event the occurrence it names, as it publishes.
    ///
    /// A note-on mints by the compiled range's lowest-free rule at this point in the
    /// applied event order, so the identity depends on what has published, never on a
    /// candidate that was prepared, withdrawn or refused. Its release frees the index
    /// here, in the same order the renderer applies the edges.
    pub(super) fn publish_compiled(
        &mut self,
        event: MixedCompiledEvent,
    ) -> Result<TimedEvent, MixedOneShotRenderError> {
        let payload = match event.payload() {
            MixedCompiledPayload::Restore(scoped) => EventPayload::ScopedRestore(scoped),
            MixedCompiledPayload::On {
                token,
                slot,
                key,
                velocity,
            } => {
                if !matches!(self.compiled_tokens.get(token.as_usize()), Some(None)) {
                    return Err(MixedOneShotRenderError::CompiledToken { token });
                }
                let identity = self
                    .compiled_minter
                    .mint_keyed(slot, key)
                    .map_err(|source| MixedOneShotRenderError::CompiledMint { source })?;
                if let Some(cell) = self.compiled_tokens.get_mut(token.as_usize()) {
                    *cell = Some(identity);
                }
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On {
                        slot,
                        key,
                        velocity,
                    },
                }
            }
            MixedCompiledPayload::Off(token) => {
                let identity = self
                    .compiled_tokens
                    .get_mut(token.as_usize())
                    .and_then(Option::take)
                    .ok_or(MixedOneShotRenderError::CompiledToken { token })?;
                let resolution = self.compiled_minter.release(identity);
                if resolution != Resolution::Live {
                    return Err(MixedOneShotRenderError::CompiledRelease { resolution });
                }
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                }
            }
            MixedCompiledPayload::Expression(token, expression) => EventPayload::Expression {
                identity: self.compiled_identity(token)?,
                expression,
            },
            MixedCompiledPayload::Bend(token, cents) => EventPayload::Bend {
                identity: self.compiled_identity(token)?,
                cents,
            },
        };
        Ok(TimedEvent::new(event.envelope(), payload))
    }

    /// Release, in the audio-owned compiled minter, exactly the occurrences the
    /// renderer's boundary release ended, and close every token of the retired list.
    ///
    /// The minter and the registry must then agree that no compiled occurrence remains:
    /// a token the registry did not end, or an ended note the minter did not hold, is a
    /// terminal custody fault. The successor list's tokens start from this empty table.
    pub(super) fn end_compiled_custody(
        &mut self,
        released: HeldNoteCount,
    ) -> Result<(), MixedOneShotRenderError> {
        let ended = released
            .as_usize()
            .and_then(|count| self.compiled_ended.get(..count))
            .ok_or(MixedOneShotRenderError::CompiledCustody { live: released })?;
        for note in ended {
            let Some(note) = note else {
                return Err(MixedOneShotRenderError::CompiledCustody { live: released });
            };
            let resolution = self.compiled_minter.release(note.identity);
            if resolution != Resolution::Live {
                return Err(MixedOneShotRenderError::CompiledRelease { resolution });
            }
        }
        self.compiled_tokens.fill(None);
        let live = self.compiled_minter.live();
        if live != 0 {
            return Err(MixedOneShotRenderError::CompiledCustody {
                live: HeldNoteCount::measured(live),
            });
        }
        Ok(())
    }

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
        #[cfg(all(test, feature = "simulated-ingress"))]
        if self.invalid_ingress_command.is_some() {
            self.fault = Some(MixedOneShotRenderError::IngressCommandOrder);
            self.audio.renderer.terminal_mixed_fault(&mut output);
            return Err(MixedOneShotRenderError::IngressCommandOrder);
        }
        if output.frames() == 0 {
            return Ok(());
        }
        #[cfg(all(test, feature = "simulated-ingress"))]
        {
            self.test_ingress_inflight_len = 0;
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
        #[cfg(all(test, feature = "simulated-ingress"))]
        {
            self.test_ingress_inflight_len = 0;
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
            self.audio.end_compiled_custody(adopted.released())?;
            core::mem::swap(&mut self.events, &mut self.capsule.events);
            self.next = 0;
            self.in_force = self.capsule.sequence;
            self.adopted = true;
            self.adoption_after_quanta = Some(self.completed_quanta);
        }

        // Serve only at a quantum boundary. The command prefix is fixed before
        // opening publication, which borrows the arbiter through the render.
        #[cfg(all(test, feature = "simulated-ingress"))]
        if opens_quantum {
            self.service_test_ingress_queue();
            if self.invalid_ingress_command.is_some() {
                return Err(MixedOneShotRenderError::IngressCommandOrder);
            }
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
                let event = self.audio.publish_compiled(event)?;
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
            #[cfg(all(test, feature = "simulated-ingress"))]
            {
                let offset = self.test_ingress_inflight_len;
                let destination = self
                    .test_ingress_inflight
                    .get_mut(offset..)
                    .ok_or(MixedOneShotRenderError::IngressJournal)?;
                let before = self
                    .test_ingress
                    .copy_queue_for_test(destination)
                    .ok_or(MixedOneShotRenderError::IngressJournal)?;
                let bends_before = self.test_ingress.mixed_bend_queued();
                let charged = self.test_ingress.drain_into(
                    &mut publication,
                    self.audio.renderer.diagnostics_mut(),
                    clock,
                );
                self.test_ingress_inflight_len = offset
                    .saturating_add(before.saturating_sub(self.test_ingress.ring_len_for_test()));
                // Each bend entry that left the queue returns its credit to the merger.
                self.bend_credit_return
                    .give_back(super::MixedIngressQueueCount(
                        bends_before.saturating_sub(self.test_ingress.mixed_bend_queued()),
                    ));
                charged?;
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
        #[cfg(all(test, feature = "simulated-ingress"))]
        if opens_quantum && self.test_fail_after_ingress_at == Some(clock) {
            return Err(MixedOneShotRenderError::InjectedAfterIngress);
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
