//! One publication and one fixed DSP quantum, with chronological loop boundaries.

use super::{
    CompiledLoopStream, HeldToken, LoopBoundary, LoopEvent, LoopFault, LoopPassId, LoopPayload,
    LoopProgram, LoopRenderControl, LoopSnapshot, LoopSource, NoteToken, PassProgram,
};
use crate::{
    identity::{NoteIdentity, Resolution},
    publish::{ProducerClass, Publication},
    render::{
        AudioBlockMut, EventEnvelope, EventPayload, NoteEdge, Renderer, TimedEvent, TimedEvents,
    },
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, TimeSource},
};

impl LoopPassId {
    fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl LoopSource {
    fn active_program(&self) -> &LoopProgram {
        match self.program {
            PassProgram::Entry => &self.initial,
            PassProgram::Repeating => &self.repeating,
        }
    }

    fn charge(
        &self,
        publication: &mut Publication<'_>,
        class: ProducerClass,
        at: SampleTime,
        payload: EventPayload,
    ) -> Result<(), LoopFault> {
        publication
            .charge(
                class,
                TimedEvent::new(
                    EventEnvelope::new(self.control.epoch(), at, TimeSource::Compiled),
                    payload,
                ),
            )
            .map_err(LoopFault::Publication)
    }

    fn restore(&self, publication: &mut Publication<'_>, at: SampleTime) -> Result<(), LoopFault> {
        publication
            .charge_operation(ProducerClass::Session, at)
            .map_err(LoopFault::Publication)?;
        for payload in &self.active_program().catch_up {
            self.charge(publication, ProducerClass::Session, at, *payload)?;
        }
        Ok(())
    }

    fn release_identity(&mut self, identity: NoteIdentity) -> Result<(), LoopFault> {
        let producer = self.producer.ok_or(LoopFault::PreparedState)?;
        match self.control.minter_mut().release_for(producer, identity) {
            Resolution::Live => Ok(()),
            other => Err(LoopFault::Release(other)),
        }
    }

    fn wrap(&mut self, publication: &mut Publication<'_>, at: SampleTime) -> Result<(), LoopFault> {
        let next = self.pass.next().ok_or(LoopFault::PassExhausted)?;
        // Only the live compact prefix is visited. A long pass with many already
        // released tokens does not turn an empty wrap into a scan of all tokens.
        for index in 0..self.held_len {
            let held = self
                .held
                .get_mut(index)
                .and_then(Option::take)
                .ok_or(LoopFault::PreparedState)?;
            let identity = self
                .tokens
                .get_mut(held.token.0)
                .and_then(Option::take)
                .ok_or(LoopFault::PreparedState)?;
            if identity != held.identity {
                return Err(LoopFault::PreparedState);
            }
            self.charge(
                publication,
                ProducerClass::Session,
                at,
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                },
            )?;
            self.release_identity(identity)?;
        }
        self.held_len = 0;
        let slot = self
            .boundaries
            .get_mut(self.boundary_len)
            .ok_or(LoopFault::PreparedState)?;
        *slot = Some(LoopBoundary {
            epoch: self.control.epoch(),
            at,
            previous: self.pass,
            next,
            interval: self.interval,
        });
        self.boundary_len += 1;
        self.pass = next;
        self.position = self.interval.start();
        self.program = PassProgram::Repeating;
        self.cursor = 0;
        self.restore(publication, at)
    }

    fn identity(&self, token: NoteToken) -> Result<NoteIdentity, LoopFault> {
        self.tokens
            .get(token.0)
            .copied()
            .flatten()
            .ok_or(LoopFault::PreparedState)
    }

    fn emit(
        &mut self,
        publication: &mut Publication<'_>,
        at: SampleTime,
        event: LoopEvent,
    ) -> Result<(), LoopFault> {
        let payload = match event.payload {
            LoopPayload::Direct(payload) => payload,
            LoopPayload::On {
                token,
                slot,
                key,
                velocity,
            } => {
                if self
                    .tokens
                    .get(token.0)
                    .ok_or(LoopFault::PreparedState)?
                    .is_some()
                {
                    return Err(LoopFault::PreparedState);
                }
                let producer = self.producer.ok_or(LoopFault::PreparedState)?;
                let identity = self
                    .control
                    .minter_mut()
                    .mint_keyed(producer, slot, key)
                    .map_err(LoopFault::Identity)?;
                *self
                    .tokens
                    .get_mut(token.0)
                    .ok_or(LoopFault::PreparedState)? = Some(identity);
                *self
                    .held
                    .get_mut(self.held_len)
                    .ok_or(LoopFault::PreparedState)? = Some(HeldToken { token, identity });
                self.held_len += 1;
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On {
                        slot,
                        key,
                        velocity,
                    },
                }
            }
            LoopPayload::Off(token) => {
                let identity = self.identity(token)?;
                let mut found = None;
                for index in 0..self.held_len {
                    if self
                        .held
                        .get(index)
                        .copied()
                        .flatten()
                        .is_some_and(|held| held.token == token)
                    {
                        found = Some(index);
                        break;
                    }
                }
                let index = found.ok_or(LoopFault::PreparedState)?;
                let last = self
                    .held_len
                    .checked_sub(1)
                    .ok_or(LoopFault::PreparedState)?;
                let held = self.held.get_mut(last).and_then(Option::take);
                *self.held.get_mut(index).ok_or(LoopFault::PreparedState)? =
                    if index == last { None } else { held };
                self.held_len = last;
                *self
                    .tokens
                    .get_mut(token.0)
                    .ok_or(LoopFault::PreparedState)? = None;
                self.release_identity(identity)?;
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                }
            }
            LoopPayload::Expression(token, expression) => EventPayload::Expression {
                identity: self.identity(token)?,
                expression,
            },
            LoopPayload::Bend(token, cents) => EventPayload::Bend {
                identity: self.identity(token)?,
                cents,
            },
        };
        self.charge(publication, ProducerClass::Compiled, at, payload)
    }

    fn quantum(
        &mut self,
        clock: SampleTime,
        publication: &mut Publication<'_>,
    ) -> Result<[PlanPosition; QUANTUM_FRAMES as usize], LoopFault> {
        let mut positions = [self.position; QUANTUM_FRAMES as usize];
        for frame in 0..QUANTUM_FRAMES as usize {
            let at = clock
                .checked_add(FrameCount::new(frame as u64))
                .map_err(LoopFault::Time)?;
            if self.position == self.interval.end() {
                self.wrap(publication, at)?;
            }
            if !self.started {
                self.restore(publication, at)?;
                self.started = true;
            }
            *positions.get_mut(frame).ok_or(LoopFault::PreparedState)? = self.position;
            let offset = FrameCount::new(
                self.position
                    .as_u64()
                    .checked_sub(self.active_program().start.as_u64())
                    .ok_or(LoopFault::PreparedState)?,
            );
            while let Some(event) = self.active_program().events.get(self.cursor).copied() {
                if event.offset > offset {
                    break;
                }
                if event.offset < offset {
                    return Err(LoopFault::PreparedState);
                }
                self.emit(publication, at, event)?;
                self.cursor += 1;
            }
            self.position = self
                .position
                .checked_add(FrameCount::new(1))
                .map_err(LoopFault::Time)?;
        }
        Ok(positions)
    }
}

impl CompiledLoopStream {
    pub fn snapshot(&self) -> LoopSnapshot {
        LoopSnapshot {
            epoch: self.source.control.epoch(),
            plan: self.source.control.plan_id(),
            clock: self.renderer.clock(),
            pass: self.source.pass,
            position: self.source.position,
        }
    }

    /// Ordered boundaries from the last call, discarded at the next render.
    /// This borrowed observation is not a concurrent recording boundary queue.
    pub fn boundaries(&self) -> impl Iterator<Item = &LoopBoundary> {
        self.source.boundaries[..self.source.boundary_len]
            .iter()
            .flatten()
    }

    /// Render a full host callback through fixed quanta. Any terminal source or
    /// render fault silences the entire outer block and all later calls.
    pub fn render(&mut self, output: AudioBlockMut<'_>) -> Result<(), LoopFault> {
        self.render_controlled(output, |_| Ok(LoopRenderControl::PLAY))
    }

    pub(crate) fn render_controlled(
        &mut self,
        mut output: AudioBlockMut<'_>,
        mut control: impl FnMut(LoopSnapshot) -> Result<LoopRenderControl, LoopFault>,
    ) -> Result<(), LoopFault> {
        if let Some(fault) = self.fault {
            output.silence();
            return Err(fault);
        }
        let quanta = match self.renderer.validate_output(&mut output) {
            Ok(quanta) => quanta,
            Err(error) if self.renderer.diagnostics().needs_reprepare() => {
                return self.fail(&mut output, LoopFault::Render(error));
            }
            Err(error) => return Err(LoopFault::Render(error)),
        };
        self.source.boundary_len = 0;
        let extent = (quanta as u64)
            .checked_mul(u64::from(QUANTUM_FRAMES))
            .ok_or(LoopFault::PreparedState)?;
        if let Err(error) = self.renderer.clock().checked_add(FrameCount::new(extent)) {
            return self.fail(&mut output, LoopFault::Time(error));
        }
        let mut delivered = 0;
        while delivered < output.frames() {
            let carry = self.renderer.carry_frames();
            let frames = (output.frames() - delivered).min(if carry == 0 {
                QUANTUM_FRAMES as usize
            } else {
                carry
            });
            let Some(window) = output.window(delivered, frames) else {
                return self.fail(&mut output, LoopFault::PreparedState);
            };
            let result = if carry != 0 {
                self.renderer
                    .render(window, TimedEvents::EMPTY)
                    .map_err(LoopFault::Render)
            } else {
                match control(self.snapshot()) {
                    Ok(action) if action.playing => self.render_quantum(window),
                    Ok(action) => self.render_stopped(window, action),
                    Err(error) => Err(error),
                }
            };
            if let Err(fault) = result {
                return self.fail(&mut output, fault);
            }
            delivered += frames;
        }
        Ok(())
    }

    fn render_stopped(
        &mut self,
        output: AudioBlockMut<'_>,
        action: LoopRenderControl,
    ) -> Result<(), LoopFault> {
        let clock = self.renderer.clock();
        let mut publication = self
            .arbiter
            .open(clock, 1)
            .map_err(LoopFault::Publication)?;
        for _ in 0..action.idle_operations.get() {
            publication
                .charge_operation(ProducerClass::Session, clock)
                .map_err(LoopFault::Publication)?;
        }
        let _batch = publication.seal();
        self.renderer.render_idle(output).map_err(LoopFault::Render)
    }

    fn render_quantum(&mut self, output: AudioBlockMut<'_>) -> Result<(), LoopFault> {
        let mut publication = self
            .arbiter
            .open(self.renderer.clock(), 1)
            .map_err(LoopFault::Publication)?;
        let positions = self
            .source
            .quantum(self.renderer.clock(), &mut publication)?;
        let batch = publication.seal();
        self.renderer
            .render_loop_quantum(output, batch.events(), &positions)
            .map_err(LoopFault::Render)
    }

    fn fail(&mut self, output: &mut AudioBlockMut<'_>, fault: LoopFault) -> Result<(), LoopFault> {
        self.fault = Some(fault);
        self.source.boundary_len = 0;
        self.renderer.terminal_loop_fault(output, fault);
        Err(fault)
    }
}
