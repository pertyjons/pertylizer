//! Quantum-boundary reset and one-quantum fade; owners remain in their two admitted slots.
use super::swap::SwapState;
use super::{
    AuditionId, AuditionOutcome, LiveInputError, LiveInputStream, Midi1Input, ReleaseCause,
    SwapOutcome, SwappingLiveStream,
};
use crate::{
    quantities::PreparedBytes,
    render::AudioBlockMut,
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor},
};
impl SwappingLiveStream {
    pub fn prepared_bytes(&self) -> PreparedBytes {
        PreparedBytes::measured(
            self.active_bytes.get()
                + self.secondary_bytes.get()
                + (self.scratch.len() * size_of::<f32>() + size_of::<Self>()) as u64,
        )
    }

    /// Admission borrows the transport cell; refusal retains its owning payload there.
    pub fn accept_prepared(
        &mut self,
        cell: &mut Option<super::PreparedLivePlan>,
    ) -> Result<bool, LiveInputError> {
        if !self.can_accept_prepared() {
            return Ok(false);
        }
        let Some(candidate) = cell.as_ref() else {
            return Ok(false);
        };
        if !candidate.fresh
            || candidate.profile != self.profile
            || candidate.live.entries.len() != self.active.entries.len()
            || candidate.live.sources.len() != self.active.sources.len()
            || candidate
                .live
                .sources
                .iter()
                .zip(&self.active.sources)
                .any(|(new, old)| new.generation != old.generation)
        {
            return Err(LiveInputError::Configuration);
        }
        if self
            .prepared_bytes()
            .get()
            .checked_add(candidate.bytes.get())
            .is_none_or(|bytes| bytes > self.ceiling.get())
        {
            return Err(LiveInputError::Bytes);
        }
        if let Some(candidate) = cell.take() {
            self.secondary = Some(candidate.live);
            self.secondary_bytes = candidate.bytes;
            self.state = SwapState::Pending;
            self.outcome = None;
        }
        Ok(true)
    }
    pub fn can_accept_prepared(&self) -> bool {
        self.state == SwapState::Vacant
            && !self.replacement_closed()
            && self.active.renderer.carry_frames() == 0
    }
    pub fn replacement_closed(&self) -> bool {
        self.failed || self.active.closed || self.active.end.is_some()
    }
    /// Moves only a retired owner with every result already collected. The caller
    /// must retain it or transfer it to an admitted off-thread reclamation slot.
    pub fn take_retired(&mut self) -> Option<super::PreparedLivePlan> {
        if self.state != SwapState::Retired
            || self
                .secondary
                .as_ref()
                .is_some_and(|old| old.entries.iter().any(Option::is_some))
        {
            return None;
        }
        let live = self.secondary.take()?;
        let bytes = self.secondary_bytes;
        self.secondary_bytes = crate::quantities::PreparedBytes::measured(0);
        self.state = SwapState::Vacant;
        Some(super::PreparedLivePlan {
            fresh: false,
            live,
            bytes,
            profile: self.profile,
        })
    }
    pub fn outcomes(&self) -> impl Iterator<Item = (AuditionId, AuditionOutcome)> + '_ {
        self.active
            .outcomes()
            .chain(self.secondary.iter().flat_map(LiveInputStream::outcomes))
    }
    pub fn clock(&self) -> SampleTime {
        self.active.clock()
    }
    pub fn update_parameter(
        &mut self,
        slot: crate::plan::ParameterSlot,
        version: super::UpdateVersion,
        value: crate::quantities::ParameterValue,
    ) -> Result<Option<super::UpdateVersion>, LiveInputError> {
        if self.state == SwapState::Pending {
            return Err(LiveInputError::Busy);
        }
        self.active.update_parameter(slot, version, value)
    }

    pub fn queue(
        &mut self,
        id: AuditionId,
        at: SampleTime,
        input: Midi1Input,
    ) -> Result<(), LiveInputError> {
        self.active.queue(id, at, input)
    }
    pub fn take_outcome(&mut self, id: AuditionId) -> Option<AuditionOutcome> {
        if let Some(outcome) = self.active.take_outcome(id) {
            return Some(outcome);
        }
        self.secondary.as_mut()?.take_outcome(id)
    }
    pub fn end_at(&mut self, at: SampleTime, cause: ReleaseCause) -> Result<(), LiveInputError> {
        self.active.end_at(at, cause)?;
        if let Some(secondary) = &mut self.secondary {
            match self.state {
                SwapState::Pending => {
                    secondary.interrupt();
                    self.state = SwapState::Retired;
                    self.outcome = Some(SwapOutcome::Cancelled);
                }
                SwapState::Fading => secondary.end_at(at, cause)?,
                SwapState::Vacant | SwapState::Retired => {}
            }
        }
        Ok(())
    }
    fn install(&mut self) -> Result<(), LiveInputError> {
        let at = self.active.clock();
        let Some(candidate) = &mut self.secondary else {
            return Err(LiveInputError::Configuration);
        };
        let end = at
            .checked_add(FrameCount::QUANTUM)
            .map_err(|_| LiveInputError::Shape)?;
        self.active.end_at(end, ReleaseCause::Stop)?;
        self.active.cancel_pending_parameters();
        for (new, old) in candidate.sources.iter_mut().zip(&self.active.sources) {
            new.serial = old.serial;
            new.pedal = old.pedal;
            new.bend = old.bend;
        }
        for (new, old) in candidate.held.iter_mut().zip(&self.active.held) {
            *new = old.filter(|held| held.down).map(|mut held| {
                held.identity = None;
                held
            });
        }
        // Unprocessed observations have no ingress/voice identity. Move them into
        // identical empty cells in the fresh owner; keep staged/completed receipts old.
        for (new, old) in candidate.entries.iter_mut().zip(&mut self.active.entries) {
            if old.is_some_and(|entry| entry.outcome.is_none() && entry.staged.is_none()) {
                *new = old.take();
            }
        }
        let anchor = StreamAnchor::new(at, PlanPosition::ZERO);
        candidate.renderer.install_stopped_clock(anchor);
        candidate.control.synchronize_replacement_anchor(anchor);
        // Advance the empty ingress horizon before admitting input in the fresh epoch.
        let mut publication = candidate
            .arbiter
            .open(at, 0)
            .map_err(crate::schedule::ScheduledRenderError::Publication)?;
        candidate
            .ingress
            .drain_into(&mut publication, candidate.renderer.diagnostics_mut(), at)
            .map_err(crate::schedule::ScheduledRenderError::Publication)?;
        core::mem::swap(&mut self.active, candidate);
        core::mem::swap(&mut self.active_bytes, &mut self.secondary_bytes);
        self.state = SwapState::Fading;
        self.fade_position = 0;
        self.outcome = Some(SwapOutcome::Installed {
            plan: self.active.control.plan_id(),
            at,
        });
        Ok(())
    }
    pub fn frames_until_boundary(&self) -> usize {
        let carry = self.active.renderer.carry_frames();
        if carry == 0 {
            QUANTUM_FRAMES as usize
        } else {
            carry
        }
    }
    pub fn render(&mut self, output: AudioBlockMut<'_>) -> Result<(), LiveInputError> {
        self.render_deferred(output)?;
        self.commit_outcomes();
        Ok(())
    }
    /// Host adapters splitting a callback must commit once after all segments succeed.
    /// On failure they silence the whole outer output and retain both owners.
    pub fn render_deferred(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), LiveInputError> {
        if self.failed
            || output.frames() == 0
            || output.layout() != self.profile.capabilities().channel_layout()
            || u64::try_from(output.frames()).map_or(true, |frames| {
                frames > self.profile.capabilities().maximum_block_size().as_u64()
            })
        {
            output.silence();
            return Err(LiveInputError::Shape);
        }
        let result = self.render_inner(output.reborrow());
        if result.is_err() {
            output.silence();
            self.recover_after_join();
        }
        result
    }
    pub fn commit_outcomes(&mut self) {
        self.active.commit_outcomes();
        if let Some(secondary) = &mut self.secondary {
            secondary.commit_outcomes();
        }
    }
    fn render_inner(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), LiveInputError> {
        while output.frames() > 0 {
            let carry = self.active.renderer.carry_frames();
            if carry == 0 && self.state == SwapState::Pending {
                self.install()?;
            }
            let frames = output.frames().min(if carry == 0 {
                QUANTUM_FRAMES as usize
            } else {
                carry
            });
            let (mut block, rest) = match output.split_at_frame(frames) {
                Ok((block, rest)) => (block, Some(rest)),
                Err(block) => (block, None),
            };
            self.active.render_deferred(block.reborrow())?;
            if self.state == SwapState::Fading {
                let channels = block.layout().channels();
                let samples = frames * channels;
                let Some(old) = &mut self.secondary else {
                    return Err(LiveInputError::Configuration);
                };
                old.render_deferred(
                    AudioBlockMut::new(&mut self.scratch[..samples], frames, block.layout())
                        .map_err(|_| LiveInputError::Shape)?,
                )?;
                for frame in 0..frames {
                    let position = u16::try_from(self.fade_position + frame + 1)
                        .map_err(|_| LiveInputError::Shape)?;
                    let blend = f32::from(position) / 64.0;
                    for channel in 0..channels {
                        let index = frame * channels + channel;
                        block.samples_mut()[index] = block.samples_mut()[index] * blend
                            + self.scratch[index] * (1.0 - blend);
                    }
                }
                self.fade_position += frames;
                if self.fade_position == QUANTUM_FRAMES as usize {
                    // The forced release must execute, not merely redeem its ingress hold.
                    let maximum =
                        usize::try_from(self.profile.capabilities().maximum_block_size().as_u64())
                            .map_err(|_| LiveInputError::Shape)?;
                    let mut remaining = QUANTUM_FRAMES as usize;
                    while remaining > 0 {
                        let count = remaining.min(maximum);
                        old.render_deferred(
                            AudioBlockMut::new(
                                &mut self.scratch[..count * channels],
                                count,
                                block.layout(),
                            )
                            .map_err(|_| LiveInputError::Shape)?,
                        )?;
                        remaining -= count;
                    }
                    if old.holds() != crate::quantities::EventCount::NONE
                        || old.renderer.has_replacement_obligations()
                    {
                        return Err(LiveInputError::Configuration);
                    }
                    self.state = SwapState::Retired;
                }
            }
            if let Some(rest) = rest {
                output = rest;
            } else {
                break;
            }
        }
        Ok(())
    }
    /// Terminal cancellation keeps both owners. May also be called by a callback on
    /// failure; only collection/destruction requires actual callback join.
    pub fn recover_after_join(&mut self) {
        self.failed = true;
        self.active.interrupt();
        if let Some(owner) = &mut self.secondary {
            owner.interrupt();
            self.state = SwapState::Retired;
        }
    }
}
impl LiveInputStream {
    pub fn has_sounding_obligations(&self) -> bool {
        self.renderer.has_replacement_obligations()
    }
    pub fn plan_id(&self) -> crate::plan::PlanId {
        self.control.plan_id()
    }
}
