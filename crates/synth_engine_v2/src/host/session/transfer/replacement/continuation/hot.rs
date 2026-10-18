//! Exact-boundary plan/list exchange without replacing actual DSP or identity authority.
use super::{ContinuationStage, ContinuingReplacement};
use crate::{
    host::session::{
        PlaybackState,
        transfer::{
            SessionAudio, SessionSnapshot,
            replacement::{
                PlanReplacementError, PlanReplacementOutcome, PlanReplacementRefusal,
                PreparedPlanReplacement,
            },
        },
    },
    render::AudioBlockMut,
    schedule::ScheduledRenderError,
};

impl SessionAudio {
    fn continuing_refusal(
        &self,
        candidate: &PreparedPlanReplacement,
        continuation: &ContinuingReplacement,
    ) -> Option<PlanReplacementRefusal> {
        if candidate.id.generation != self.origin.generation || candidate.epoch != self.origin.epoch
        {
            return Some(PlanReplacementRefusal::Origin);
        }
        if self.runtime.closed || self.fault.is_some() || self.running_transition {
            return Some(PlanReplacementRefusal::Unavailable);
        }
        if self.origin.table != candidate.prepared_from_table
            || self.origin.plan != continuation.proof.old
        {
            return Some(PlanReplacementRefusal::MigrationOrigin);
        }
        if candidate.id.serial <= self.publication_serial {
            return Some(PlanReplacementRefusal::Superseded);
        }
        if self.runtime.held != 0 || self.runtime.serial != candidate.command_watermark {
            return Some(PlanReplacementRefusal::CommandsChanged);
        }
        if self.runtime.state != PlaybackState::Playing(continuation.anchor) {
            return Some(PlanReplacementRefusal::PositionChanged);
        }
        if self.clock() > continuation.boundary {
            return Some(PlanReplacementRefusal::LateRunningBoundary {
                at: continuation.boundary,
                clock: self.clock(),
            });
        }
        if self.clock() != continuation.boundary || self.renderer.carry_frames() != 0 {
            return Some(PlanReplacementRefusal::Carry);
        }
        let Some(witness) = continuation.witness.as_ref() else {
            return Some(PlanReplacementRefusal::NotFresh);
        };
        if self.runtime.sources.is_some()
            || self.runtime.adopted_slot.is_some()
            || self.renderer.has_readmission_obligations()
            || self.renderer.table_id() != self.origin.table
        {
            return Some(PlanReplacementRefusal::NoteObligations);
        }
        if continuation.stage != ContinuationStage::Ready
            || witness.plan != continuation.proof.new
            || witness.epoch != self.origin.epoch
            || witness.table != self.origin.table
            || witness.anchor != continuation.anchor
            || continuation.events.len() != witness.events.len()
            || candidate.plan != continuation.proof.new
            || !continuation.retired_catch_up.is_empty()
            || self.runtime.session_share != candidate.audio.runtime.session_share
            || self.runtime.slots.len() != candidate.audio.runtime.slots.len()
            || self.runtime.command_bytes != candidate.audio.runtime.command_bytes
            || !self.runtime.scheduler.continuation_ready(
                self.origin.epoch,
                self.origin.table,
                witness.sequence,
                continuation.events.len(),
            )
        {
            return Some(PlanReplacementRefusal::NotFresh);
        }
        if let Err(error) = self
            .renderer
            .continuation_preflight(&candidate.audio.renderer, &continuation.proof)
        {
            return Some(PlanReplacementRefusal::Continuation(error));
        }
        None
    }

    pub(crate) fn begin_continuing_replacement(
        &mut self,
        mut candidate: Box<PreparedPlanReplacement>,
    ) -> Result<Box<PreparedPlanReplacement>, (Box<PreparedPlanReplacement>, PlanReplacementError)>
    {
        let Some(continuation) = candidate.continuation.as_ref() else {
            return Err((candidate, PlanReplacementError::State));
        };
        if candidate.outcome.is_some() || continuation.stage != ContinuationStage::Ready {
            return Err((candidate, PlanReplacementError::State));
        }
        if !self.runtime.closed
            && self.fault.is_none()
            && let Ok(frames) = self.frames_until_running_boundary(&candidate)
            && frames.as_u64() != 0
        {
            return Err((candidate, PlanReplacementError::State));
        }
        if let Some(refusal) = self.continuing_refusal(&candidate, continuation) {
            if refusal != PlanReplacementRefusal::Origin {
                self.publication_serial = self.publication_serial.max(candidate.id.serial);
            }
            candidate.outcome = Some(PlanReplacementOutcome::Refused(refusal));
            return Ok(candidate);
        }
        let Some(continuation) = candidate.continuation.as_mut() else {
            return Err((candidate, PlanReplacementError::State));
        };
        // All source/destination extents and the immutable proof were checked. The
        // actual renderer/runtime stay in place; old Arc/Vec owners move to custody.
        candidate.outgoing_table = Some(self.origin.table);
        self.renderer
            .install_continuation(&mut candidate.audio.renderer, &continuation.proof);
        self.runtime
            .scheduler
            .continue_event_owners(&mut continuation.events, &mut continuation.retired_catch_up);
        self.origin.plan = continuation.proof.new;
        self.publication_serial = candidate.id.serial;
        self.running_transition = true;
        continuation.stage = ContinuationStage::Rendering;
        Ok(candidate)
    }

    fn interrupt_continuation(
        &mut self,
        candidate: &mut PreparedPlanReplacement,
        error: ScheduledRenderError,
    ) {
        self.fault = Some(error);
        self.runtime.closed = true;
        self.runtime.state = PlaybackState::Unavailable;
        self.running_transition = false;
        if let Some(continuation) = candidate.continuation.as_mut() {
            continuation.stage = ContinuationStage::Complete;
        }
        candidate.outcome = Some(PlanReplacementOutcome::Interrupted(SessionSnapshot {
            clock: self.clock(),
            playback: self.runtime.state,
        }));
    }

    pub(crate) fn render_continuing_replacement(
        &mut self,
        candidate: &mut PreparedPlanReplacement,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        let belongs = candidate.id.generation == self.origin.generation
            && candidate.epoch == self.origin.epoch
            && candidate.outgoing_table == Some(self.origin.table)
            && candidate
                .continuation
                .as_ref()
                .is_some_and(|continuation| self.origin.plan == continuation.proof.new);
        let ready = belongs
            && self.running_transition
            && candidate.outcome.is_none()
            && !self.runtime.closed
            && self.fault.is_none()
            && candidate.continuation.as_ref().is_some_and(|continuation| {
                continuation.stage == ContinuationStage::Rendering
                    && self.runtime.state == PlaybackState::Playing(continuation.anchor)
            });
        if !ready {
            output.silence();
            let error = ScheduledRenderError::RunningReplacementState;
            if belongs {
                self.interrupt_continuation(candidate, error);
            } else {
                self.fault = Some(error);
                self.runtime.closed = true;
            }
            return Err(error);
        }
        let result = self.render_owned(output.reborrow());
        if let Err(error) = result {
            output.silence();
            self.interrupt_continuation(candidate, error);
        } else if output.frames() != 0 {
            self.running_transition = false;
            if let Some(continuation) = candidate.continuation.as_mut() {
                continuation.stage = ContinuationStage::Complete;
            }
            candidate.outcome = Some(PlanReplacementOutcome::Installed(SessionSnapshot {
                clock: self.clock(),
                playback: self.runtime.state,
            }));
        }
        result
    }
}
