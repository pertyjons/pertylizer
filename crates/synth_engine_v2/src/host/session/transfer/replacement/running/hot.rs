//! Exact-boundary fresh-owner exchange, fixed one-quantum fade and terminal custody.
use super::{RunningReset, RunningStage, SessionAudio, SessionSnapshot};
use crate::{
    host::session::{
        PlaybackState,
        transfer::replacement::{
            PlanReplacementError, PlanReplacementOutcome, PlanReplacementRefusal,
            PreparedPlanReplacement,
        },
    },
    render::AudioBlockMut,
    schedule::ScheduledRenderError,
    time::{FrameCount, QUANTUM_FRAMES},
};

impl SessionAudio {
    /// Output frames still belonging to the old owner before the requested B.
    pub fn frames_until_running_boundary(
        &self,
        candidate: &PreparedPlanReplacement,
    ) -> Result<FrameCount, PlanReplacementRefusal> {
        let at = candidate
            .running_boundary()
            .ok_or(PlanReplacementRefusal::NotFresh)?;
        if candidate.id.generation != self.origin.generation || candidate.epoch != self.origin.epoch
        {
            return Err(PlanReplacementRefusal::Origin);
        }
        if candidate.prepared_from_table != self.origin.table {
            return Err(PlanReplacementRefusal::MigrationOrigin);
        }
        if self.clock() > at {
            return Err(PlanReplacementRefusal::LateRunningBoundary {
                at,
                clock: self.clock(),
            });
        }
        let distance = at
            .as_u64()
            .checked_sub(self.clock().as_u64())
            .ok_or(PlanReplacementRefusal::NotFresh)?;
        let carry = u64::try_from(self.renderer.carry_frames())
            .map_err(|_| PlanReplacementRefusal::Carry)?;
        distance
            .checked_add(carry)
            .map(FrameCount::new)
            .ok_or(PlanReplacementRefusal::NotFresh)
    }

    fn running_refusal(
        &self,
        candidate: &PreparedPlanReplacement,
    ) -> Option<PlanReplacementRefusal> {
        let reset = candidate.running.as_ref()?;
        if candidate.id.generation != self.origin.generation || candidate.epoch != self.origin.epoch
        {
            return Some(PlanReplacementRefusal::Origin);
        }
        if self.runtime.closed || self.fault.is_some() || self.running_transition {
            return Some(PlanReplacementRefusal::Unavailable);
        }
        if candidate.id.serial <= self.publication_serial {
            return Some(PlanReplacementRefusal::Superseded);
        }
        if self.runtime.held != 0 || self.runtime.serial != candidate.command_watermark {
            return Some(PlanReplacementRefusal::CommandsChanged);
        }
        if self.origin.table != candidate.prepared_from_table {
            return Some(PlanReplacementRefusal::MigrationOrigin);
        }
        if self.runtime.state != PlaybackState::Playing(reset.expected_anchor) {
            return Some(PlanReplacementRefusal::PositionChanged);
        }
        if self.clock() > reset.anchor.time() {
            return Some(PlanReplacementRefusal::LateRunningBoundary {
                at: reset.anchor.time(),
                clock: self.clock(),
            });
        }
        if self.clock() != reset.anchor.time() || self.renderer.carry_frames() != 0 {
            return Some(PlanReplacementRefusal::Carry);
        }
        if self.renderer.has_readmission_obligations()
            || self.runtime.sources.is_some()
            || self.runtime.adopted_slot.is_some()
            || !self.runtime.scheduler.activation_exchange_empty()
        {
            return Some(PlanReplacementRefusal::NoteObligations);
        }
        if !self.running_successor_ready(candidate, reset) {
            return Some(PlanReplacementRefusal::NotFresh);
        }
        None
    }

    fn running_successor_ready(
        &self,
        candidate: &PreparedPlanReplacement,
        reset: &RunningReset,
    ) -> bool {
        let fresh = &candidate.audio;
        let playing = match reset.target {
            PlaybackState::Playing(anchor) if anchor == reset.anchor => true,
            PlaybackState::Stopped(position) if position == reset.anchor.position() => false,
            _ => return false,
        };
        let serial = u64::from(playing);
        let last = if playing {
            Some(reset.anchor.time())
        } else {
            None
        };
        let Some(samples) = (QUANTUM_FRAMES as usize)
            .checked_mul(fresh.renderer.plan().channel_layout().channels())
        else {
            return false;
        };
        if reset.scratch.len() != samples
            || candidate.control.held != 0
            || candidate.control.closed
            || candidate.control.serial != serial
            || candidate.control.acknowledged_serial != serial
            || candidate.control.last_offer != last
            || candidate.control.publication_serial != 0
            || candidate.control.acknowledged
                != (SessionSnapshot {
                    clock: reset.anchor.time(),
                    playback: reset.target,
                })
        {
            return false;
        }
        if reset.stage != RunningStage::Ready
            || reset.faded != FrameCount::new(0)
            || fresh.origin != candidate.control.origin
            || fresh.origin.generation == self.origin.generation
            || fresh.origin.epoch == self.origin.epoch
            || fresh.origin.table == self.origin.table
            || fresh.origin.plan != candidate.plan
            || fresh.clock() != reset.anchor.time()
            || fresh.renderer.carry_frames() != 0
            || fresh.runtime.state != reset.target
            || fresh.runtime.closed
            || fresh.fault.is_some()
            || fresh.running_transition
            || fresh.runtime.held != 0
            || fresh.runtime.adopted_slot.is_some()
            || fresh.runtime.serial != serial
            || fresh.runtime.last_offer != last
            || fresh.publication_serial != 0
            || !fresh.runtime.scheduler.initial_reset_ready(playing)
        {
            return false;
        }
        true
    }

    /// Begin only at B. On success the returned owning box is Fading, not complete;
    /// render through `render_running_replacement` and retain it until an outcome exists.
    pub fn begin_running_replacement(
        &mut self,
        mut candidate: Box<PreparedPlanReplacement>,
    ) -> Result<Box<PreparedPlanReplacement>, (Box<PreparedPlanReplacement>, PlanReplacementError)>
    {
        if candidate.continuation.is_some() {
            return self.begin_continuing_replacement(candidate);
        }
        if candidate.outcome.is_some()
            || candidate
                .running
                .as_ref()
                .is_none_or(|reset| reset.stage != RunningStage::Ready)
        {
            return Err((candidate, PlanReplacementError::State));
        }
        if let Ok(frames) = self.frames_until_running_boundary(&candidate)
            && frames.as_u64() != 0
        {
            // The owning value remains Ready; a timely host can wait without re-preparing.
            return Err((candidate, PlanReplacementError::State));
        }
        if let Some(refusal) = self.running_refusal(&candidate) {
            if refusal != PlanReplacementRefusal::Origin {
                self.publication_serial = self.publication_serial.max(candidate.id.serial);
            }
            candidate.outcome = Some(PlanReplacementOutcome::Refused(refusal));
            return Ok(candidate);
        }
        self.publication_serial = candidate.id.serial;
        candidate.outgoing_table = Some(self.origin.table);
        if let Some(reset) = candidate.running.as_mut() {
            reset.stage = RunningStage::Fading;
        }
        // Keep every fresh-generation serial/epoch/table and initial adoption debt.
        core::mem::swap(self, &mut candidate.audio);
        self.running_transition = true;
        Ok(candidate)
    }

    fn quiesce_faded_compiled(
        &mut self,
    ) -> Result<crate::quantities::HeldNoteCount, ScheduledRenderError> {
        if self.runtime.closed
            || self.fault.is_some()
            || self.runtime.held != 0
            || self.runtime.sources.is_some()
            || self.runtime.adopted_slot.is_some()
            || !self.runtime.scheduler.activation_exchange_empty()
            || self.renderer.carry_frames() != 0
            || self.renderer.has_readmission_obligations()
        {
            return Err(ScheduledRenderError::RunningReplacementState);
        }
        let retired = self.renderer.readmission_retired_notes();
        // This owner is never rendered or addressed again. Its isolated compiled
        // minter and registry end together at off-thread collection, not via a fake Stop.
        self.runtime.closed = true;
        self.runtime.state = PlaybackState::Unavailable;
        Ok(retired)
    }

    fn interrupt_running(
        &mut self,
        candidate: &mut PreparedPlanReplacement,
        error: ScheduledRenderError,
    ) {
        self.fault = Some(error);
        self.running_transition = false;
        self.runtime.closed = true;
        self.runtime.state = PlaybackState::Unavailable;
        candidate.retired_compiled_notes = candidate.audio.renderer.readmission_retired_notes();
        candidate.audio.runtime.closed = true;
        candidate.audio.runtime.state = PlaybackState::Unavailable;
        if let Some(reset) = candidate.running.as_mut() {
            reset.stage = RunningStage::Complete;
        }
        candidate.outcome = Some(PlanReplacementOutcome::Interrupted(SessionSnapshot {
            clock: self.clock(),
            playback: self.runtime.state,
        }));
    }

    /// While Fading this is the only render entry. Any failure silences the whole
    /// supplied outer block, closes the installed owner and retains both owners.
    pub fn render_running_replacement(
        &mut self,
        candidate: &mut PreparedPlanReplacement,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        if candidate.continuation.is_some() {
            return self.render_continuing_replacement(candidate, output);
        }
        if candidate
            .running
            .as_ref()
            .is_none_or(|reset| reset.stage != RunningStage::Fading)
            || candidate.control.origin != self.origin
            || candidate.outcome.is_some()
        {
            output.silence();
            let error = ScheduledRenderError::RunningReplacementState;
            if candidate.control.origin == self.origin && candidate.outgoing_table.is_some() {
                self.interrupt_running(candidate, error);
            } else {
                // A foreign payload cannot mutate its own outcome through this owner.
                self.fault = Some(error);
                self.runtime.closed = true;
            }
            return Err(error);
        }
        let result = self.render_running_inner(candidate, &mut output);
        if let Err(error) = result {
            output.silence();
            self.interrupt_running(candidate, error);
        }
        result
    }

    fn fade_position(
        &self,
        candidate: &PreparedPlanReplacement,
    ) -> Result<usize, ScheduledRenderError> {
        let reset = candidate
            .running
            .as_ref()
            .ok_or(ScheduledRenderError::RunningReplacementState)?;
        let old = &candidate.audio;
        let faded = usize::try_from(reset.faded.as_u64())
            .map_err(|_| ScheduledRenderError::RunningReplacementState)?;
        let quantum = QUANTUM_FRAMES as usize;
        if faded >= quantum {
            return Err(ScheduledRenderError::RunningReplacementState);
        }
        let clock = if faded == 0 {
            reset.anchor.time()
        } else {
            reset
                .anchor
                .time()
                .checked_advance_quantum()
                .map_err(|_| ScheduledRenderError::RunningReplacementState)?
        };
        let carry = if faded == 0 { 0 } else { quantum - faded };
        if self.runtime.closed
            || self.fault.is_some()
            || !self.running_transition
            || self.runtime.state != reset.target
            || self.clock() != clock
            || old.clock() != clock
            || self.renderer.carry_frames() != carry
            || old.renderer.carry_frames() != carry
        {
            return Err(ScheduledRenderError::RunningReplacementState);
        }
        if old.origin.generation != candidate.id.generation
            || old.origin.epoch != candidate.epoch
            || old.origin.table != candidate.prepared_from_table
            || old.runtime.serial != candidate.command_watermark
            || old.runtime.held != 0
            || old.runtime.closed
            || old.runtime.state != PlaybackState::Playing(reset.expected_anchor)
        {
            return Err(ScheduledRenderError::RunningReplacementState);
        }
        Ok(faded)
    }

    fn render_running_inner(
        &mut self,
        candidate: &mut PreparedPlanReplacement,
        output: &mut AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        self.renderer.validate_output(output)?;
        let faded = self.fade_position(candidate)?;
        let Some(reset) = candidate.running.as_mut() else {
            return Err(ScheduledRenderError::RunningReplacementState);
        };
        let quantum = QUANTUM_FRAMES as usize;
        let frames = output.frames().min(quantum - faded);
        let channels = output.layout().channels();
        let samples = frames
            .checked_mul(channels)
            .ok_or(ScheduledRenderError::RunningReplacementState)?;
        let old_samples = reset
            .scratch
            .get_mut(..samples)
            .ok_or(ScheduledRenderError::RunningReplacementState)?;
        let old = AudioBlockMut::new(old_samples, frames, output.layout())
            .map_err(|_| ScheduledRenderError::RunningReplacementState)?;
        candidate.audio.render(old)?;
        let mut head = output
            .window(0, frames)
            .ok_or(ScheduledRenderError::RunningReplacementState)?;
        self.render_owned(head.reborrow())?;
        blend(reset, &mut head, faded)?;
        reset.faded = FrameCount::new(
            u64::try_from(faded + frames)
                .map_err(|_| ScheduledRenderError::RunningReplacementState)?,
        );
        if faded + frames == quantum {
            candidate.retired_compiled_notes = candidate.audio.quiesce_faded_compiled()?;
            reset.stage = RunningStage::Complete;
            self.running_transition = false;
            candidate.outcome = Some(PlanReplacementOutcome::Installed(SessionSnapshot {
                clock: self.clock(),
                playback: self.runtime.state,
            }));
        }
        if frames < output.frames() {
            let tail = output
                .window(frames, output.frames() - frames)
                .ok_or(ScheduledRenderError::RunningReplacementState)?;
            self.render_owned(tail)?;
        }
        Ok(())
    }
}

fn blend(
    reset: &RunningReset,
    head: &mut AudioBlockMut<'_>,
    faded: usize,
) -> Result<(), ScheduledRenderError> {
    let frames = head.frames();
    let channels = head.layout().channels();
    let mixed = head.samples_mut();
    for frame in 0..frames {
        let step = u16::try_from(faded + frame + 1)
            .map_err(|_| ScheduledRenderError::RunningReplacementState)?;
        let total = u16::try_from(QUANTUM_FRAMES)
            .map_err(|_| ScheduledRenderError::RunningReplacementState)?;
        if step == total {
            // Preserve the successor's endpoint bits, including signed zero.
            continue;
        }
        let gain = f32::from(step) / f32::from(total);
        for channel in 0..channels {
            let index = frame * channels + channel;
            let old = reset
                .scratch
                .get(index)
                .copied()
                .ok_or(ScheduledRenderError::RunningReplacementState)?;
            let value = mixed
                .get_mut(index)
                .ok_or(ScheduledRenderError::RunningReplacementState)?;
            *value = old * (1.0 - gain) + *value * gain;
        }
    }
    Ok(())
}
