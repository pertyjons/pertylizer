//! Whole-pair installation at an empty-carry boundary. No owning value is dropped.

use super::{
    ObservationAdmission, ObservationReplacementOutcome, PlanReplacementError,
    PlanReplacementOutcome, PlanReplacementRefusal, PreparedPlanReplacement,
};
use crate::host::session::PlaybackState;
use crate::host::session::transfer::{SessionAudio, SessionSnapshot};
use crate::observe::{ObservationHandoffError, ObservationSubscriptions};
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
        if candidate.migration.is_some() && candidate.prepared_from_table != self.origin.table {
            return Some(PlanReplacementRefusal::MigrationOrigin);
        }
        if self.renderer.carry_frames() != 0 {
            return Some(PlanReplacementRefusal::Carry);
        }
        // Preparation checked its authoritative base minter. A different current
        // table can only be an intervening replacement in this frozen interval:
        // commands would change the checked watermark. Its fresh minter snapshot
        // is therefore still valid, even before its new control is collected.
        if (candidate.prepared_from_table != self.origin.table && self.prepared_minter_obligations)
            || self.renderer.has_readmission_obligations()
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
        candidate: Box<PreparedPlanReplacement>,
    ) -> Result<Box<PreparedPlanReplacement>, (Box<PreparedPlanReplacement>, PlanReplacementError)>
    {
        if candidate.observation.is_some() {
            return Err((candidate, PlanReplacementError::State));
        }
        self.install_replacement_inner(candidate)
    }

    /// Install stopped sound first, then hand observation to the actual successor.
    /// Sound refusal leaves the store untouched. Observation refusal ends its old
    /// labelled tail and never rejects the sound edit. Resume with `render_observed`;
    /// ordinary stopped idle quanta still produce no observation samples.
    pub fn install_replacement_observed(
        &mut self,
        candidate: Box<PreparedPlanReplacement>,
        observers: &mut ObservationSubscriptions,
    ) -> Result<Box<PreparedPlanReplacement>, (Box<PreparedPlanReplacement>, PlanReplacementError)>
    {
        let mut candidate = self.install_replacement_inner(candidate)?;
        if matches!(
            candidate.outcome,
            Some(PlanReplacementOutcome::Installed(_))
        ) {
            self.install_observation_handoff(&mut candidate, observers);
        }
        Ok(candidate)
    }

    /// Called only after the sound commit, by stopped and running owning entries.
    pub(super) fn install_observation_handoff(
        &self,
        candidate: &mut PreparedPlanReplacement,
        observers: &mut ObservationSubscriptions,
    ) {
        let result = match &candidate.observation {
            Some(ObservationAdmission::Prepared(prepared))
                if prepared.new_plan() == self.plan_id() =>
            {
                observers
                    .handoff(prepared)
                    .map(|()| prepared.successor_generation())
            }
            Some(ObservationAdmission::Prepared(_)) => Err(ObservationHandoffError::Origin),
            Some(ObservationAdmission::Refused(error)) => Err(*error),
            None => Err(ObservationHandoffError::NotPrepared),
        };
        candidate.observation_outcome = match result {
            Ok(generation) => ObservationReplacementOutcome::HandedOff {
                generation,
                kind: candidate.kind(),
            },
            Err(error) => {
                observers.end();
                ObservationReplacementOutcome::Ended(error)
            }
        };
    }

    fn install_replacement_inner(
        &mut self,
        mut candidate: Box<PreparedPlanReplacement>,
    ) -> Result<Box<PreparedPlanReplacement>, (Box<PreparedPlanReplacement>, PlanReplacementError)>
    {
        if candidate.outcome.is_some()
            || (candidate.running.is_some() || candidate.continuation.is_some())
        {
            return Err((candidate, PlanReplacementError::State));
        }
        if let Some(reason) = self.replacement_refusal(&candidate) {
            if reason != PlanReplacementRefusal::Origin {
                self.publication_serial = self.publication_serial.max(candidate.id.serial);
            }
            candidate.outcome = Some(PlanReplacementOutcome::Refused(reason));
            return Ok(candidate);
        }
        let retired_compiled_notes = self.renderer.readmission_retired_notes();
        if let Some(migration) = &candidate.migration
            && let Err(error) =
                migration.transfer_into(&mut self.renderer, &mut candidate.audio.renderer)
        {
            self.publication_serial = self.publication_serial.max(candidate.id.serial);
            candidate.outcome = Some(PlanReplacementOutcome::Refused(
                PlanReplacementRefusal::Migration(error),
            ));
            return Ok(candidate);
        }
        candidate.retired_compiled_notes = retired_compiled_notes;
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

impl PreparedPlanReplacement {
    pub const fn kind(&self) -> super::PlanReplacementKind {
        if self.continuation.is_some() {
            super::PlanReplacementKind::RunningCompatible
        } else if self.running.is_some() {
            super::PlanReplacementKind::RunningReset
        } else if self.migration.is_some() {
            super::PlanReplacementKind::StoppedCompatible
        } else {
            super::PlanReplacementKind::StoppedReset
        }
    }
    pub fn running_boundary(&self) -> Option<crate::time::SampleTime> {
        if let Some(running) = &self.running {
            return Some(running.anchor.time());
        }
        self.continuation
            .as_ref()
            .map(|continuation| continuation.boundary)
    }
    pub fn is_rendering_transition(&self) -> bool {
        self.continuation.as_ref().is_some_and(|continuation| {
            continuation.stage == super::continuation::ContinuationStage::Rendering
        }) || self
            .running
            .as_ref()
            .is_some_and(|running| running.stage == super::running::RunningStage::Fading)
    }
    pub fn transition_bytes(&self) -> crate::quantities::PreparedBytes {
        if let Some(continuation) = &self.continuation {
            return continuation.bytes;
        }
        self.running
            .as_ref()
            .map_or(crate::quantities::PreparedBytes::NONE, |running| {
                running.bytes
            })
    }
}
