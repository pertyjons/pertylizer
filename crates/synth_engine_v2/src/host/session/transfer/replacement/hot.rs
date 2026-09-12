//! Whole-pair installation at an empty-carry boundary. No owning value is dropped.

use super::{
    PlanReplacementError, PlanReplacementOutcome, PlanReplacementRefusal, PreparedPlanReplacement,
};
use crate::host::session::PlaybackState;
use crate::host::session::transfer::{SessionAudio, SessionSnapshot};
use crate::time::{FrameCount, QUANTUM_FRAMES, SampleTime, StreamAnchor};

impl SessionAudio {
    /// Drain this many old output frames before attempting a plan replacement.
    /// Between calls carry is at most Q, representable by FrameCount.
    pub fn frames_until_plan_boundary(&self) -> FrameCount {
        FrameCount::new(
            u64::try_from(self.renderer.carry_frames()).unwrap_or(u64::from(QUANTUM_FRAMES)),
        )
    }

    fn replacement_refusal(
        &self,
        candidate: &PreparedPlanReplacement,
    ) -> Option<PlanReplacementRefusal> {
        if candidate.id.generation != self.origin.generation || candidate.epoch != self.origin.epoch
        {
            return Some(PlanReplacementRefusal::Origin);
        }
        if self.runtime.closed || self.fault.is_some() {
            return Some(PlanReplacementRefusal::Unavailable);
        }
        if candidate.id.serial <= self.publication_serial {
            return Some(PlanReplacementRefusal::Superseded);
        }
        if self.runtime.held != 0 || self.runtime.serial != candidate.command_watermark {
            return Some(PlanReplacementRefusal::CommandsChanged);
        }
        if self.runtime.state != PlaybackState::Stopped(candidate.expected_position) {
            return Some(PlanReplacementRefusal::PositionChanged);
        }
        if self.renderer.carry_frames() != 0 {
            return Some(PlanReplacementRefusal::Carry);
        }
        // Preparation checked its authoritative base minter. A different current
        // table can only be an intervening replacement in this frozen interval:
        // commands would change the checked watermark. Its fresh minter snapshot
        // is therefore still valid, even before its new control is collected.
        if (candidate.prepared_from_table != self.origin.table && self.prepared_minter_obligations)
            || self.renderer.has_replacement_obligations()
        {
            return Some(PlanReplacementRefusal::NoteObligations);
        }
        if candidate.audio.renderer.clock() != SampleTime::ZERO
            || candidate.audio.renderer.carry_frames() != QUANTUM_FRAMES as usize
            || candidate.audio.runtime.held != 0
            || candidate.audio.fault.is_some()
        {
            return Some(PlanReplacementRefusal::NotFresh);
        }
        None
    }

    /// The host first reserves an owning return slot. Every attempted candidate
    /// becomes a retained Installed/Refused outcome. A protocol misuse returns the
    /// original box; neither route destroys plan, control, runtime or packet here.
    pub fn install_replacement(
        &mut self,
        mut candidate: Box<PreparedPlanReplacement>,
    ) -> Result<Box<PreparedPlanReplacement>, (Box<PreparedPlanReplacement>, PlanReplacementError)>
    {
        if candidate.outcome.is_some() {
            return Err((candidate, PlanReplacementError::State));
        }
        if let Some(reason) = self.replacement_refusal(&candidate) {
            if reason != PlanReplacementRefusal::Origin {
                self.publication_serial = self.publication_serial.max(candidate.id.serial);
            }
            candidate.outcome = Some(PlanReplacementOutcome::Refused(reason));
            return Ok(candidate);
        }
        let snapshot = SessionSnapshot {
            clock: self.renderer.clock(),
            playback: self.runtime.state,
        };
        candidate.outgoing_table = Some(self.origin.table);
        candidate
            .audio
            .renderer
            .install_stopped_clock(StreamAnchor::new(
                snapshot.clock,
                candidate.expected_position,
            ));
        candidate.audio.runtime.state = snapshot.playback;
        candidate.audio.runtime.serial = self.runtime.serial;
        candidate.audio.runtime.last_offer = self.runtime.last_offer;
        candidate.audio.publication_serial = candidate.id.serial;
        core::mem::swap(self, &mut candidate.audio);
        candidate.outcome = Some(PlanReplacementOutcome::Installed(snapshot));
        Ok(candidate)
    }
}
