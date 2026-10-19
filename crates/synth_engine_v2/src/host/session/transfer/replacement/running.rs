//! Fresh-generation compiled-owner reset. No old minter snapshot is promoted.
mod hot;

use super::{
    PlanPublicationId, PlanReplacementError, PreparedPlanReplacement, REPLACEMENT_CREDITS,
};
use crate::{
    host::{
        HostError,
        session::{
            PlaybackState, SessionError, SessionOutcome,
            transfer::{SessionAudio, SessionControl, SessionSnapshot, SessionTransferLimits},
        },
    },
    plan::CompiledPlan,
    profile::HostProfile,
    quantities::{EventCount, HeldNoteCount, PreparedBytes},
    schedule::AdmittedCompiledStream,
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor},
};

/// Preparation policy for the default-off running-edit consumer. Producer shares are unchanged.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct RunningReplacementRequest {
    pub profile: HostProfile,
    pub transfer: SessionTransferLimits,
    /// None is an unbounded authored graph, not an inferred saved-song end.
    pub song_end: Option<PlanPosition>,
    /// Bounds position-dependent preparation work, not callback event density.
    pub maximum_timeline_events: EventCount,
    /// Try exact-program voice continuation; otherwise use the fresh reset path.
    pub mode: RunningEditMode,
}

/// The host's graph-identity permission and cold witness selection.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub enum RunningEditMode {
    Reset,
    ResetWithWitness(crate::host::session::transfer::ContinuationLimits),
    /// The host also binds its own unchanged end/loop policy; the core proves the
    /// compiled program and excludes loop/capture/external-source ownership.
    ContinueOrReset(crate::host::session::transfer::ContinuationLimits),
}
impl RunningEditMode {
    const fn witness_limits(self) -> Option<crate::host::session::transfer::ContinuationLimits> {
        match self {
            Self::Reset => None,
            Self::ResetWithWitness(limits) | Self::ContinueOrReset(limits) => Some(limits),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunningStage {
    Ready,
    Fading,
    Complete,
}

pub(super) struct RunningReset {
    pub(super) expected_anchor: StreamAnchor,
    pub(super) anchor: StreamAnchor,
    pub(super) target: PlaybackState,
    pub(super) stage: RunningStage,
    pub(super) faded: FrameCount,
    pub(super) scratch: Vec<f32>,
    pub(super) bytes: PreparedBytes,
}

/// On error the complete unpublished pair ends, including its private credit ledger.
fn initialize_at(
    control: &mut SessionControl,
    audio: &mut SessionAudio,
    anchor: StreamAnchor,
    target: PlaybackState,
    witness: Option<crate::host::session::transfer::ContinuationLimits>,
) -> Result<Option<crate::plan::ContinuationError>, HostError> {
    control.acknowledged = SessionSnapshot {
        clock: anchor.time(),
        playback: PlaybackState::Stopped(anchor.position()),
    };
    audio.runtime.state = control.acknowledged.playback;
    audio.renderer.install_stopped_clock(anchor);
    if target == PlaybackState::Stopped(anchor.position()) {
        control.control.synchronize_replacement_anchor(anchor);
        return Ok(None);
    }
    if target != PlaybackState::Playing(anchor) {
        return Err(SessionError::InitialActivation.into());
    }
    let mut play = control.prepare_play(anchor.time())?;
    // This copy is optional metadata for a later edit. Its failure leaves the
    // ordinary activation intact; report it to the host instead of disabling reset.
    let witness_error = match witness {
        Some(limits) => play.prepare_continuation_witness(limits).err(),
        None => None,
    };
    audio
        .enqueue(play)
        .map_err(|(_packet, error)| SessionError::from(error))?;
    audio
        .runtime
        .boundary(&mut audio.renderer, None)
        .map_err(SessionError::from)?;
    if !audio
        .runtime
        .scheduler
        .adopt_initial_at(&mut audio.renderer, anchor.time())
    {
        return Err(SessionError::InitialActivation.into());
    }
    audio.runtime.retain_adopted();
    let completed = audio
        .take_completed()
        .ok_or(SessionError::InitialActivation)?;
    let receipt = control
        .collect(completed)
        .map_err(|(_packet, error)| error)?;
    if !matches!(receipt.outcome, SessionOutcome::Applied { position } if position == anchor.position())
        || control.has_outstanding()
        || audio.has_retained_commands()
    {
        return Err(SessionError::InitialActivation.into());
    }
    audio.prepared_minter_obligations =
        control.control.has_replacement_obligations() || control.control.minter_mut().live() != 0;
    Ok(witness_error)
}

fn prepare_scratch(
    request: RunningReplacementRequest,
    controller_bytes: PreparedBytes,
) -> Result<(Vec<f32>, PreparedBytes), PlanReplacementError> {
    let samples = (QUANTUM_FRAMES as usize)
        .checked_mul(request.profile.capabilities().channel_layout().channels())
        .ok_or(PlanReplacementError::RunningBytes)?;
    let mut scratch = Vec::new();
    scratch
        .try_reserve_exact(samples)
        .map_err(|_| PlanReplacementError::RunningAllocation)?;
    scratch.resize(samples, 0.0);
    let bytes = scratch
        .capacity()
        .checked_mul(size_of::<f32>())
        .and_then(|bytes| bytes.checked_add(size_of::<RunningReset>()))
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(PlanReplacementError::RunningBytes)?;
    // Envelope shells are charged by control.bytes; this payload is separate.
    let allowance = request
        .transfer
        .control_bytes
        .get()
        .checked_sub(controller_bytes.get())
        .ok_or(PlanReplacementError::RunningBytes)?
        / REPLACEMENT_CREDITS as u64;
    if bytes > allowance {
        return Err(PlanReplacementError::RunningBytes);
    }
    Ok((scratch, PreparedBytes::measured(bytes)))
}

fn boundary_anchor(
    expected_anchor: StreamAnchor,
    at: SampleTime,
    acknowledged_clock: SampleTime,
) -> Result<StreamAnchor, PlanReplacementError> {
    if !at.as_u64().is_multiple_of(u64::from(QUANTUM_FRAMES)) || at < acknowledged_clock {
        return Err(PlanReplacementError::RunningBoundary);
    }
    let distance = at
        .difference(expected_anchor.time())
        .map_err(|_| PlanReplacementError::RunningBoundary)?;
    let frames =
        u64::try_from(distance.as_i64()).map_err(|_| PlanReplacementError::RunningBoundary)?;
    let position = expected_anchor
        .position()
        .checked_add(FrameCount::new(frames))
        .map_err(|_| PlanReplacementError::RunningBoundary)?;
    let _fade_end = at
        .checked_advance_quantum()
        .map_err(|_| PlanReplacementError::RunningBoundary)?;
    let _last_position = position
        .checked_add(FrameCount::QUANTUM)
        .map_err(|_| PlanReplacementError::RunningBoundary)?;
    Ok(StreamAnchor::new(at, position))
}

impl SessionControl {
    /// Prepare a running edit off-thread. Exact native continuation keeps the actual
    /// authority; other edits use a fresh owner with its initial activation collected.
    /// The boundary chooser runs after proof/copy/pair preparation. A late publication
    /// must be collected/destroyed, never retimed in place.
    pub fn prepare_running_replacement(
        &mut self,
        plan: CompiledPlan,
        stream: AdmittedCompiledStream,
        request: RunningReplacementRequest,
        choose_boundary: impl FnOnce(StreamAnchor) -> Option<SampleTime>,
    ) -> Result<Box<PreparedPlanReplacement>, PlanReplacementError> {
        let PlaybackState::Playing(expected_anchor) = self.acknowledged.playback else {
            return Err(PlanReplacementError::RunningTransport);
        };
        if self.closed || self.held != 0 || self.has_replacements() {
            return Err(PlanReplacementError::RunningTransport);
        }
        if self.control.has_replacement_obligations() {
            return Err(PlanReplacementError::NoteObligations);
        }
        let maximum = usize::try_from(request.maximum_timeline_events.get()).map_err(|_| {
            PlanReplacementError::RunningTimeline {
                limit: request.maximum_timeline_events,
            }
        })?;
        if maximum == 0 || stream.events().len() > maximum {
            return Err(PlanReplacementError::RunningTimeline {
                limit: request.maximum_timeline_events,
            });
        }
        let old = self.control.plan();
        if old.sample_rate() != plan.sample_rate()
            || old.channel_layout() != plan.channel_layout()
            || old.maximum_block_size() != plan.maximum_block_size()
        {
            return Err(PlanReplacementError::Geometry);
        }
        let index = self
            .replacement_credits
            .iter()
            .position(Option::is_none)
            .ok_or(PlanReplacementError::Full)?;
        let serial = self
            .publication_serial
            .checked_add(1)
            .ok_or(PlanReplacementError::IdentityExhausted)?;
        let mut continuation = match request.mode {
            RunningEditMode::ContinueOrReset(limits)
                if request
                    .profile
                    .limits()
                    .events()
                    .shares()
                    .session_event_share()
                    == self.session_share
                    && u32::try_from(self.credits.len()).ok()
                        == Some(request.transfer.session.commands.as_u32()) =>
            {
                self.try_continuation(&plan, &stream, limits)?
            }
            RunningEditMode::ContinueOrReset(_) => None,
            RunningEditMode::Reset | RunningEditMode::ResetWithWitness(_) => None,
        };
        let (mut control, mut audio) =
            Self::prepare_pair(plan, stream, request.profile, request.transfer, None)?;
        if control.control.has_replacement_obligations() {
            return Err(PlanReplacementError::NoteObligations);
        }
        let (scratch, bytes) = prepare_scratch(request, control.bytes)?;
        let at = choose_boundary(expected_anchor).ok_or(PlanReplacementError::RunningBoundary)?;
        let anchor = boundary_anchor(expected_anchor, at, self.acknowledged.clock)?;
        let position = anchor.position();
        let target = if request.song_end.is_some_and(|end| position >= end) {
            PlaybackState::Stopped(position)
        } else {
            PlaybackState::Playing(anchor)
        };
        if matches!(target, PlaybackState::Stopped(_)) {
            continuation = None;
        }
        let continuation_unavailable = if let Some(continuation) = continuation.as_mut() {
            continuation.boundary = at;
            None
        } else {
            initialize_at(
                &mut control,
                &mut audio,
                anchor,
                target,
                request.mode.witness_limits(),
            )?
        };
        let id = PlanPublicationId {
            generation: self.origin.generation,
            serial,
        };
        let candidate = Box::new(PreparedPlanReplacement {
            id,
            epoch: self.origin.epoch,
            plan: control.origin.plan,
            prepared_from_table: self.origin.table,
            old_plan: self.origin.plan,
            observation: None,
            observation_outcome: super::ObservationReplacementOutcome::NotAttached,
            expected_position: position,
            command_watermark: self.acknowledged_serial,
            control,
            audio,
            outcome: None,
            outgoing_table: None,
            migration: None,
            retired_compiled_notes: HeldNoteCount::NONE,
            running: if continuation.is_none() {
                Some(Box::new(RunningReset {
                    expected_anchor,
                    anchor,
                    target,
                    stage: RunningStage::Ready,
                    faded: FrameCount::new(0),
                    scratch,
                    bytes,
                }))
            } else {
                None
            },
            continuation: continuation.map(Box::new),
            continuation_unavailable,
        });
        self.replacement_credits[index] = Some(id);
        self.publication_serial = serial;
        Ok(candidate)
    }
}

#[cfg(test)]
fn prepare_initial(
    plan: CompiledPlan,
    stream: AdmittedCompiledStream,
    profile: HostProfile,
    limits: SessionTransferLimits,
    anchor: StreamAnchor,
) -> Result<(SessionControl, SessionAudio), HostError> {
    let (mut control, mut audio) = SessionControl::prepare(plan, stream, profile, limits)?;
    if control.control.has_replacement_obligations() {
        return Err(SessionError::FreshStreamRequired.into());
    }
    let _no_witness = initialize_at(
        &mut control,
        &mut audio,
        anchor,
        PlaybackState::Playing(anchor),
        None,
    )?;
    Ok((control, audio))
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod observation_tests;
