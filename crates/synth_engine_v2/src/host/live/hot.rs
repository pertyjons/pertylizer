//! Fixed-custody audition admission and rendering; no owner is freed on this path.
use super::{
    AuditionId, AuditionOutcome, ConnectionGeneration, Entry, Held, LiveInputError,
    LiveInputStream, Midi1Input, MidiChannel, PreviewHeld, ReleaseCause,
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

    /// A note-on needs both a result entry and a held-occurrence cell. An
    /// earlier queued key release or sustain lift can free the latter only when
    /// its ordering and pedal state prove the cell available before this onset.
    /// Later queued events must preserve every pending onset's held-cell credit.
    /// The check uses prepared storage without allocation. Its worst-case scan
    /// is O(q² × (s + 1)) for quota q and source count s.
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
        if self.sources[port].serial >= id.serial {
            return Err(LiveInputError::Identity);
        }
        let index = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(LiveInputError::Capacity)?;
        if !self.closed
            && self
                .end
                .is_none_or(|(end, _)| nominal.max(self.clock()) < end)
        {
            // A collected result may free an entry while its key still owns a held cell.
            let occupied = self.held.iter().flatten().count();
            let pending = self
                .entries
                .iter()
                .flatten()
                .filter(|entry| {
                    entry.outcome.is_none()
                        && entry.staged.is_none()
                        && self
                            .end
                            .is_none_or(|(end, _)| entry.nominal.max(self.clock()) < end)
                        && matches!(entry.input.event(), Midi1Event::NoteOn { .. })
                })
                .count();
            if occupied >= self.held.len().saturating_sub(pending)
                && !self.can_stage_pending(id, nominal, input)?
            {
                return Err(LiveInputError::Capacity);
            }
        }
        let cell = &mut self.entries[index];
        self.sources[port].serial = id.serial;
        *cell = Some(Entry {
            id,
            nominal,
            input,
            staged: None,
            outcome: self.closed.then_some(AuditionOutcome::Cancelled),
        });
        Ok(())
    }

    fn can_stage_pending(
        &mut self,
        id: AuditionId,
        nominal: SampleTime,
        input: Midi1Input,
    ) -> Result<bool, LiveInputError> {
        self.preview.fill(None);
        for (preview, held) in self.preview.iter_mut().zip(&self.held) {
            *preview = held.map(|held| PreviewHeld {
                id: held.id,
                channel: held.channel,
                key: held.key,
                down: held.down,
                identity_possible: held.identity.is_some(),
            });
        }

        let source_rank = self
            .sources
            .iter()
            .position(|source| source.generation == id.source)
            .ok_or(LiveInputError::Identity)?;
        let candidate = Entry {
            id,
            nominal,
            input,
            staged: None,
            outcome: None,
        };
        let candidate_order = (nominal, source_rank, id.serial);
        let mut last = None;
        loop {
            let mut next = last
                .is_none_or(|prior| candidate_order > prior)
                .then_some((candidate_order, candidate));
            for entry in self.entries.iter().flatten() {
                if entry.outcome.is_some()
                    || entry.staged.is_some()
                    || self
                        .end
                        .is_some_and(|(end, _)| entry.nominal.max(self.clock()) >= end)
                {
                    continue;
                }
                let rank = self
                    .sources
                    .iter()
                    .position(|source| source.generation == entry.id.source)
                    .ok_or(LiveInputError::Identity)?;
                let order = (entry.nominal, rank, entry.id.serial);
                if last.is_some_and(|prior| order <= prior) {
                    continue;
                }
                if next.as_ref().is_none_or(|(prior, _)| order < *prior) {
                    next = Some((order, *entry));
                }
            }
            let Some((order, entry)) = next else {
                break;
            };
            match entry.input.event() {
                Midi1Event::NoteOn { key, .. } => {
                    let mut vacant = None;
                    for (index, cell) in self.preview.iter().enumerate() {
                        if cell.is_none() {
                            vacant = Some(index);
                            break;
                        }
                    }
                    let Some(index) = vacant else {
                        return Ok(false);
                    };
                    self.preview[index] = Some(PreviewHeld {
                        id: entry.id,
                        channel: entry.input.channel(),
                        key,
                        down: true,
                        identity_possible: true,
                    });
                }
                Midi1Event::KeyRelease { key, .. } => {
                    let mut selected: Option<(usize, u64)> = None;
                    for (index, cell) in self.preview.iter().enumerate() {
                        if let Some(held) = cell
                            && held.down
                            && held.id.source == entry.id.source
                            && held.channel == entry.input.channel()
                            && held.key == key
                            && selected.is_none_or(|(_, serial)| held.id.serial < serial)
                        {
                            selected = Some((index, held.id.serial));
                        }
                    }
                    if let Some((index, _)) = selected {
                        let port = order.1;
                        let channel = entry.input.channel();
                        let mut pedal_down =
                            self.sources[port].pedal[usize::from(channel.as_index())];
                        let mut last_pedal = None;
                        for pending in self.entries.iter().flatten() {
                            if pending.outcome.is_some()
                                || pending.staged.is_some()
                                || pending.id.source != entry.id.source
                                || pending.input.channel() != channel
                            {
                                continue;
                            }
                            let pending_order = (pending.nominal, port, pending.id.serial);
                            if pending_order < order
                                && last_pedal.is_none_or(|prior| pending_order > prior)
                                && let Midi1Event::Sustain { down } = pending.input.event()
                            {
                                pedal_down = down;
                                last_pedal = Some(pending_order);
                            }
                        }
                        if candidate.id.source == entry.id.source
                            && candidate.input.channel() == channel
                            && candidate_order < order
                            && last_pedal.is_none_or(|prior| candidate_order > prior)
                            && let Midi1Event::Sustain { down } = candidate.input.event()
                        {
                            pedal_down = down;
                        }
                        if let Some(held) = self.preview[index].as_mut() {
                            held.down = false;
                            if !pedal_down || !held.identity_possible {
                                self.preview[index] = None;
                            }
                        }
                    }
                }
                Midi1Event::Sustain { down: false } => {
                    for cell in &mut self.preview {
                        if cell.is_some_and(|held| {
                            held.id.source == entry.id.source
                                && held.channel == entry.input.channel()
                                && !held.down
                        }) {
                            *cell = None;
                        }
                    }
                }
                Midi1Event::Sustain { down: true } | Midi1Event::PitchBend { .. } => {}
            }
            last = Some(order);
        }
        Ok(true)
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
