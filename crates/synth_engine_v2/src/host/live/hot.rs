//! Fixed-custody audition admission and rendering; no owner is freed on this path.
use super::{
    AuditionId, AuditionOutcome, ConnectionGeneration, Entry, Held, LiveInputError,
    LiveInputStream, Midi1Input, MidiChannel, ReleaseCause,
};
use crate::{
    publish::ProducerClass,
    recording::notes::Midi1Event,
    render::{AudioBlockMut, EventPayload, Renderer, TimedEvent},
    schedule::ScheduledRenderError,
    time::{FrameCount, QUANTUM_FRAMES, SampleTime},
};

impl LiveInputStream {
    pub fn holds(&self) -> crate::quantities::EventCount {
        self.ingress.holds_outstanding()
    }

    pub fn queue(
        &mut self,
        id: AuditionId,
        nominal: SampleTime,
        input: Midi1Input,
    ) -> Result<(), LiveInputError> {
        let port = self
            .sources
            .iter()
            .position(|source| source.generation == id.source)
            .ok_or(LiveInputError::Identity)?;
        let source = &mut self.sources[port];
        if source.serial >= id.serial {
            return Err(LiveInputError::Identity);
        }
        let index = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(LiveInputError::Capacity)?;
        let cell = &mut self.entries[index];
        source.serial = id.serial;
        *cell = Some(Entry {
            id,
            nominal,
            input,
            staged: None,
            outcome: self.closed.then_some(AuditionOutcome::Cancelled),
        });
        Ok(())
    }

    /// The consumer must reconcile any raw capture annotation before returning this cell.
    pub fn take_outcome(&mut self, id: AuditionId) -> Option<AuditionOutcome> {
        for cell in &mut self.entries {
            if let Some(entry) = cell
                && entry.id == id
                && let Some(outcome) = entry.outcome
            {
                *cell = None;
                return Some(outcome);
            }
        }
        None
    }

    /// Called only for an applied ordered transport end, before rendering its span.
    pub fn end_at(&mut self, at: SampleTime, cause: ReleaseCause) -> Result<(), LiveInputError> {
        if at < self.clock() || !at.as_u64().is_multiple_of(u64::from(QUANTUM_FRAMES)) {
            return Err(LiveInputError::Shape);
        }
        if self.end.is_none_or(|(prior, _)| at < prior) {
            self.end = Some((at, cause));
        }
        Ok(())
    }

    /// After callback join, unresolved operations become cancellations, never executions.
    pub fn interrupt(&mut self) {
        self.closed = true;
        self.failed = true;
        self.cancel_parameters();
        for entry in self.entries.iter_mut().flatten() {
            if entry.outcome.is_none() {
                entry.outcome = Some(AuditionOutcome::Cancelled);
            }
            entry.staged = None;
        }
    }

    fn release(
        &mut self,
        at: SampleTime,
        scope: Option<(ConnectionGeneration, MidiChannel)>,
        cause: ReleaseCause,
    ) -> Result<(), LiveInputError> {
        let mut first = None;
        for held in self.held.iter().flatten() {
            if selected(held, scope) && held.identity.is_some() {
                first = held.identity;
                break;
            }
        }
        let Some(first) = first else {
            return Ok(());
        };
        let mut identities = [first; crate::ingress::RELEASE_GROUP_CAPACITY];
        let mut count = 0;
        for held in self.held.iter().flatten() {
            if selected(held, scope)
                && let Some(identity) = held.identity
            {
                let slot = identities
                    .get_mut(count)
                    .ok_or(LiveInputError::Configuration)?;
                *slot = identity;
                count += 1;
            }
        }
        self.control
            .offer_release_group(&mut self.ingress, at, &identities[..count], cause)?;
        for cell in &mut self.held {
            if let Some(held) = cell
                && selected(held, scope)
            {
                held.identity = None;
                if !held.down {
                    *cell = None;
                }
            }
        }
        Ok(())
    }

    fn input(&mut self, entry: Entry, at: SampleTime) -> Result<AuditionOutcome, LiveInputError> {
        let port = self
            .sources
            .iter()
            .position(|source| source.generation == entry.id.source)
            .ok_or(LiveInputError::Identity)?;
        let channel = entry.input.channel();
        let channel_index = usize::from(channel.as_index());
        let executed = AuditionOutcome::Executed {
            epoch: self.epoch(),
            at,
        };
        match entry.input.event() {
            Midi1Event::NoteOn { key, velocity } => {
                // Refused onsets retain a FIFO tombstone until their own key release.
                let index = self
                    .held
                    .iter()
                    .position(Option::is_none)
                    .ok_or(LiveInputError::Capacity)?;
                let slot = &mut self.held[index];
                let result =
                    self.control
                        .offer_note_on(&mut self.ingress, at, self.note, key, velocity);
                *slot = Some(Held {
                    id: entry.id,
                    channel,
                    key,
                    down: true,
                    identity: result.ok(),
                });
                if let Ok(identity) = result
                    && self.sources[port].bend[channel_index] != crate::quantities::Cents::ZERO
                {
                    self.control.offer_bend(
                        &mut self.ingress,
                        at,
                        identity,
                        self.sources[port].bend[channel_index],
                    )?;
                }
                Ok(match result {
                    Ok(_) => executed,
                    Err(refusal) => AuditionOutcome::Refused(refusal),
                })
            }
            Midi1Event::KeyRelease { key, .. } => {
                let mut selected: Option<(usize, u64)> = None;
                for (position, held) in self.held.iter().enumerate() {
                    if let Some(held) = held
                        && held.id.source == entry.id.source
                        && held.channel == channel
                        && held.key == key
                        && held.down
                        && selected.is_none_or(|(_, serial)| held.id.serial < serial)
                    {
                        selected = Some((position, held.id.serial));
                    }
                }
                let Some(cell) = selected.and_then(|(index, _)| self.held.get_mut(index)) else {
                    return Ok(AuditionOutcome::UnmatchedRelease);
                };
                let Some(held) = cell.as_mut() else {
                    return Err(LiveInputError::Identity);
                };
                held.down = false;
                let Some(identity) = held.identity else {
                    *cell = None;
                    return Ok(AuditionOutcome::NotSounded);
                };
                if !self.sources[port].pedal[channel_index] {
                    self.control
                        .offer_note_off(&mut self.ingress, at, identity)?;
                    *cell = None;
                }
                Ok(executed)
            }
            Midi1Event::Sustain { down } => {
                if !down {
                    self.release(
                        at,
                        Some((entry.id.source, channel)),
                        ReleaseCause::SustainLift,
                    )?;
                }
                self.sources[port].pedal[channel_index] = down;
                Ok(executed)
            }
            Midi1Event::PitchBend { value } => {
                let cents = crate::quantities::Cents::new(value.as_f32() * 200.0)
                    .map_err(|_| LiveInputError::Configuration)?;
                // Failure silences and terminates the whole callback, never reports a
                // partially applied channel bend as executed.
                for held in self.held.iter().flatten() {
                    if held.id.source == entry.id.source
                        && held.channel == channel
                        && let Some(identity) = held.identity
                    {
                        self.control
                            .offer_bend(&mut self.ingress, at, identity, cents)?;
                    }
                }
                self.sources[port].bend[channel_index] = cents;
                Ok(executed)
            }
        }
    }

    fn stage_quantum(&mut self) -> Result<(), LiveInputError> {
        let clock = self.clock();
        if !self.closed
            && let Some((at, cause)) = self.end
            && at <= clock
        {
            self.release(clock, None, cause)?;
            self.closed = true;
            self.cancel_pending_parameters();
        }
        if self.closed {
            for entry in self.entries.iter_mut().flatten() {
                if entry.outcome.is_none() && entry.staged.is_none() {
                    entry.outcome = Some(AuditionOutcome::Cancelled);
                }
            }
            return Ok(());
        }
        let end = clock
            .checked_add(FrameCount::QUANTUM)
            .map_err(|_| LiveInputError::Shape)?;
        for _ in 0..self.entries.len() {
            let mut selected: Option<(usize, Entry)> = None;
            for (index, entry) in self.entries.iter().enumerate() {
                let Some(entry) = entry else {
                    continue;
                };
                if entry.outcome.is_some() || entry.staged.is_some() || entry.nominal >= end {
                    continue;
                }
                let order = |entry: &Entry| {
                    (
                        entry.nominal,
                        self.sources
                            .iter()
                            .position(|source| source.generation == entry.id.source),
                        entry.id.serial,
                    )
                };
                if selected
                    .as_ref()
                    .is_none_or(|(_, prior)| order(entry) < order(prior))
                {
                    selected = Some((index, *entry));
                }
            }
            let Some((index, entry)) = selected else {
                break;
            };
            let outcome = self.input(entry, entry.nominal.max(clock))?;
            if let Some(Some(entry)) = self.entries.get_mut(index) {
                entry.staged = Some(outcome);
            }
        }
        Ok(())
    }

    pub fn render(&mut self, output: AudioBlockMut<'_>) -> Result<(), LiveInputError> {
        self.render_deferred(output)?;
        self.commit_outcomes();
        Ok(())
    }

    /// The enclosing host commits only after its complete callback succeeds.
    pub(super) fn render_deferred(
        &mut self,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), LiveInputError> {
        if self.failed {
            output.silence();
            return Err(LiveInputError::Closed);
        }
        if output.frames() == 0
            || output.layout() != self.renderer.plan().channel_layout()
            || u64::try_from(output.frames()).map_or(true, |frames| {
                frames > self.renderer.plan().maximum_block_size().as_u64()
            })
        {
            output.silence();
            return Err(LiveInputError::Shape);
        }
        let result = self.render_inner(output.reborrow());
        if result.is_err() {
            output.silence();
            self.interrupt();
        }
        result
    }

    pub(super) fn commit_outcomes(&mut self) {
        for entry in self.entries.iter_mut().flatten() {
            if let Some(outcome) = entry.staged.take() {
                entry.outcome = Some(outcome);
            }
        }
        for cell in &mut self.parameters {
            if let Some(version) = cell.staged.take() {
                cell.applied = Some(version);
            }
        }
    }

    fn render_inner(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), LiveInputError> {
        while output.frames() > 0 {
            let carry = self.renderer.carry_frames();
            if carry == 0 {
                self.stage_quantum()?;
            }
            let frames = output.frames().min(if carry == 0 {
                QUANTUM_FRAMES as usize
            } else {
                carry
            });
            let (block, rest) = match output.split_at_frame(frames) {
                Ok((block, rest)) => (block, Some(rest)),
                Err(block) => (block, None),
            };
            let clock = self.clock();
            let mut publication = self
                .arbiter
                .open(clock, usize::from(carry == 0))
                .map_err(ScheduledRenderError::Publication)?;
            if carry == 0 && !self.closed {
                for cell in &mut self.parameters {
                    if let Some((version, value)) = cell.pending {
                        publication
                            .charge(
                                ProducerClass::Session,
                                TimedEvent::new(
                                    crate::ingress::PerformanceIngress::envelope_for(
                                        self.renderer.epoch(),
                                        clock,
                                    ),
                                    EventPayload::SetParameter {
                                        slot: cell.slot,
                                        value,
                                    },
                                ),
                            )
                            .map_err(ScheduledRenderError::Publication)?;
                        cell.pending = None;
                        cell.staged = Some(version);
                    }
                }
            }
            self.ingress
                .drain_into(&mut publication, self.renderer.diagnostics_mut(), clock)
                .map_err(ScheduledRenderError::Publication)?;
            self.renderer
                .render(block, publication.seal().events())
                .map_err(ScheduledRenderError::Render)?;
            if let Some(rest) = rest {
                output = rest;
            } else {
                break;
            }
        }
        Ok(())
    }
}

fn selected(held: &Held, scope: Option<(ConnectionGeneration, MidiChannel)>) -> bool {
    scope.is_none_or(|(source, channel)| {
        held.id.source == source && held.channel == channel && !held.down
    })
}
