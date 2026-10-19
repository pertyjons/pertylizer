//! Experimental live song playback through Core V2 (ADR-0077).
//!
//! A lowered project plays through the ordered session transport: [`SongPlayback`] is the
//! control half, [`SongAudio`] the audio-callback half. Commands cross in a bounded
//! single-producer ring and return in another, so the callback neither allocates, frees,
//! locks nor waits. The control half schedules each command a margin past the clock the
//! audio half last published, on a quantum boundary as the session requires.
//!
//! One owning lane handles compatible paused edits and running changes. Exact native
//! running continuation retains voices; other running edits reset with a Q-frame fade.
//! Explicit Stop prepares a fresh session; seek and loop refuse at the application.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use synth_engine_v2::{
    host::session::{
        PlaybackState, SessionCommandCapacity, SessionLimits, SessionOutcome,
        transfer::{
            CompletedSessionCommand, ContinuationLimits, PreparedSessionCommand, SessionAudio,
            SessionControl, SessionTransferLimits,
            replacement::{
                PlanReplacementError, PlanReplacementKind, PlanReplacementOutcome,
                PreparedPlanReplacement, RunningEditMode, RunningReplacementRequest,
            },
        },
    },
    plan::PlanId,
    profile::HostProfile,
    quantities::{EventCount, PreparedBytes},
    render::AudioBlockMut,
    schedule::{AdmittedCompiledStream, PlanEvent},
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime},
};
use thiserror::Error;

use super::{LoweredProject, Severity};

#[path = "live_observation.rs"]
mod observation;

/// Command cells in each direction; play and pause need one each, the rest is headroom.
const COMMAND_SLOTS: usize = 8;

/// The song-playback partition an accepted evidence record has selected (ADR-0054 clause 3,
/// ADR-0077 decision 6). EVD-0025 selected compiled 96 and session 128 as bounds enforced
/// before playback and the per-quantum cap 360, for this path alone; every other share is
/// unreachable here. A profile with any other value is not qualified.
pub const QUALIFIED_COMPILED_SHARE: u32 = 96;
pub const QUALIFIED_SESSION_SHARE: u32 = 128;
pub const QUALIFIED_EVENTS_PER_QUANTUM: u32 = 360;

/// Provisional cold copies of one adopted program, never a callback event quota.
fn continuation_limits() -> ContinuationLimits {
    ContinuationLimits {
        maximum_transition_records: synth_engine_v2::quantities::RecordCount::measured(16_384),
        maximum_stamped_events: EventCount::measured(16_384),
        witness_bytes: PreparedBytes::measured(4 * 1024 * 1024),
        transition_bytes: PreparedBytes::measured(4 * 1024 * 1024),
    }
}

/// Whether `profile` carries exactly the partition EVD-0025 qualified.
#[must_use]
pub fn partition_is_qualified(profile: &HostProfile) -> bool {
    let events = profile.limits().events();
    let shares = events.shares();
    shares.compiled_event_share().get() == QUALIFIED_COMPILED_SHARE
        && shares.session_event_share().get() == QUALIFIED_SESSION_SHARE
        && events.max_events_per_quantum().get() == QUALIFIED_EVENTS_PER_QUANTUM
}

#[derive(Debug, Error)]
pub enum SongPlaybackError {
    #[error("the project does not lower to V2: {0}")]
    Refused(String),
    #[error("V2 song playback waits for an accepted capacity reselection (ADR-0054, ADR-0077)")]
    CapacityUnqualified,
    #[error("V2 song preparation failed: {0}")]
    Preparation(String),
    #[error("the V2 session refused the command: {0}")]
    Command(String),
    #[error("the V2 command ring is full")]
    Full,
    #[error("the V2 song is stopping at its end; play again once it has stopped")]
    Ending,
    #[error("the V2 session faulted; the song must be prepared again")]
    Faulted,
}

/// Internal edit policy distinguishes transient, unsupported and fatal preparation.
#[derive(Debug, Error)]
pub(crate) enum SongEditError {
    #[error("the V2 session is waiting for acknowledged transport or edit collection")]
    Busy,
    #[error("the V2 edit path is unavailable: {0}")]
    Unsupported(String),
    #[error("V2 edit preparation failed: {0}")]
    Preparation(String),
}

/// Control half, owned by the application thread.
#[must_use]
pub struct SongPlayback {
    observers: observation::SongObservationControl,
    control: SessionControl,
    limits: SessionTransferLimits,
    edits: HeapProd<SongEdit>,
    edited: HeapCons<SongEdit>,
    edit_pending: Option<PlanPosition>,
    edit_completion: Option<(super::app::InstallSequence, bool)>,
    unresolved_edit: Option<SongEdit>,
    edit_failed: bool,
    commands: HeapProd<PreparedSessionCommand>,
    completed: HeapCons<CompletedSessionCommand>,
    clock: Arc<AtomicU64>,
    margin: u64,
    playing: bool,
    applied: Option<PlanPosition>,
    /// The arrangement's end, where playback stops by itself.
    end: PlanPosition,
    /// Commands submitted and not yet collected.
    outstanding: usize,
    /// The boundary of the outstanding stop at the arrangement's end.
    ending: Option<SampleTime>,
    /// Commands the session did not apply, oldest first, for the application to show.
    refusals: Vec<String>,
    /// Commands the audio half could not admit, returned for cancellation here.
    returned: HeapCons<PreparedSessionCommand>,
    /// Set by the audio half when its session faulted; the session never renders again.
    faulted: Arc<AtomicBool>,
    fault_reported: bool,
    /// Packets V2 refused to cancel or collect, kept with their credit rather than dropped.
    unresolved_commands: Vec<PreparedSessionCommand>,
    unresolved_completions: Vec<CompletedSessionCommand>,
    /// Every applied position in order, for the EVD-0025 measurement.
    #[cfg(test)]
    applied_log: Vec<PlanPosition>,
}

/// Audio half, owned by the callback.
#[must_use]
pub struct SongAudio {
    observers: super::observation::ObservationAudio,
    audio: SessionAudio,
    edits: HeapCons<SongEdit>,
    edited: HeapProd<SongEdit>,
    pending_edit: Option<SongEdit>,
    install: super::app::InstallSequence,
    maximum_frames: FrameCount,
    layout: synth_engine_v2::quantities::ChannelLayout,
    commands: HeapCons<PreparedSessionCommand>,
    completed: HeapProd<CompletedSessionCommand>,
    pending: Option<CompletedSessionCommand>,
    /// A command the session would not admit, waiting for room in the return ring.
    refused: Option<PreparedSessionCommand>,
    returns: HeapProd<PreparedSessionCommand>,
    clock: Arc<AtomicU64>,
    faulted: Arc<AtomicBool>,
}

/// One candidate at a time: its return cell is reserved until control collects it.
struct SongEdit {
    packet: Box<PreparedPlanReplacement>,
    install: super::app::InstallSequence,
}

/// Prepare a lowered project for live playback on `profile`, stopped at the song start.
///
/// The lowering must have been made for the same profile. A refused lowering names its
/// first refusal; an unqualified capacity partition refuses before anything is built.
pub fn prepare_song(
    lowered: LoweredProject,
    profile: HostProfile,
) -> Result<(SongPlayback, SongAudio), SongPlaybackError> {
    if !partition_is_qualified(&profile) {
        return Err(SongPlaybackError::CapacityUnqualified);
    }
    prepare_unqualified(lowered, profile)
}

/// [`prepare_song`] without the capacity gate, for tests and the reselection measurement.
pub(crate) fn prepare_unqualified(
    lowered: LoweredProject,
    profile: HostProfile,
) -> Result<(SongPlayback, SongAudio), SongPlaybackError> {
    if let Some(refusal) = lowered
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.severity() == Severity::Refused)
    {
        return Err(SongPlaybackError::Refused(format!(
            "{:?}: {:?}",
            refusal.subject(),
            refusal.reason()
        )));
    }
    let plan = lowered
        .plan
        .ok_or_else(|| SongPlaybackError::Refused("no plan was produced".to_owned()))?;
    // Offline events are engine times from a render anchored at plan position zero, so a
    // time is the same number of frames as its plan position.
    let events: Vec<PlanEvent> = lowered
        .events
        .iter()
        .map(|event| PlanEvent::new(PlanPosition::new(event.time().as_u64()), event.payload()))
        .collect();
    let stream = AdmittedCompiledStream::admit(&plan, &events)
        .map_err(|error| SongPlaybackError::Preparation(error.to_string()))?;
    let preparation =
        |error: &dyn std::fmt::Display| SongPlaybackError::Preparation(error.to_string());
    let limits = SessionTransferLimits {
        session: SessionLimits {
            commands: SessionCommandCapacity::new(
                u32::try_from(COMMAND_SLOTS).map_err(|error| preparation(&error))?,
            )
            .map_err(|error| preparation(&error))?,
            command_bytes: PreparedBytes::limit(16_384).map_err(|error| preparation(&error))?,
        },
        // Five core credits reserve whole-pair envelopes before assigning map bytes.
        // This cold metadata allowance changes no event share or callback capacity.
        control_bytes: PreparedBytes::limit(65_536).map_err(|error| preparation(&error))?,
    };
    let maximum = profile.capabilities().maximum_block_size().as_u64();
    let end = PlanPosition::new(lowered.lowered_frames.as_u64());
    let (control, audio) = SessionControl::prepare(plan, stream, profile, limits)
        .map_err(|error| preparation(&error))?;
    let (commands, command_reader) = HeapRb::new(COMMAND_SLOTS).split();
    let (completion_writer, completed) = HeapRb::new(COMMAND_SLOTS).split();
    let (returns, returned) = HeapRb::new(COMMAND_SLOTS).split();
    let (edits, edit_reader) = HeapRb::new(1).split();
    let (edit_writer, edited) = HeapRb::new(1).split();
    let clock = Arc::new(AtomicU64::new(0));
    let faulted = Arc::new(AtomicBool::new(false));
    let (observers, observer_audio) = observation::SongObservationControl::prepare(&profile)?;
    Ok((
        SongPlayback {
            observers,
            control,
            limits,
            edits,
            edited,
            edit_pending: None,
            edit_completion: None,
            unresolved_edit: None,
            edit_failed: false,
            commands,
            completed,
            clock: Arc::clone(&clock),
            // Two whole callbacks plus a quantum: the command is due after the callback
            // that may already be rendering when it is scheduled.
            margin: 2 * maximum + u64::from(QUANTUM_FRAMES),
            playing: false,
            applied: None,
            end,
            outstanding: 0,
            ending: None,
            refusals: Vec::new(),
            returned,
            faulted: Arc::clone(&faulted),
            fault_reported: false,
            unresolved_commands: Vec::new(),
            unresolved_completions: Vec::new(),
            #[cfg(test)]
            applied_log: Vec::new(),
        },
        SongAudio {
            observers: observer_audio,
            audio,
            edits: edit_reader,
            edited: edit_writer,
            pending_edit: None,
            install: super::app::InstallSequence::NONE,
            maximum_frames: profile.capabilities().maximum_block_size(),
            layout: profile.capabilities().channel_layout(),
            commands: command_reader,
            completed: completion_writer,
            pending: None,
            refused: None,
            returns,
            clock,
            faulted,
        },
    ))
}

impl SongPlayback {
    /// A pause request is insufficient: every ordinary command must be collected.
    pub(crate) fn edit_ready(&mut self) -> bool {
        self.collect();
        !self.is_faulted()
            && self.outstanding == 0
            && !self.observers.changing()
            && self.edit_pending.is_none()
            && self.unresolved_edit.is_none()
            && !self.edit_failed
            && matches!(self.control.snapshot().playback, PlaybackState::Stopped(_))
    }

    pub(crate) const fn is_editing(&self) -> bool {
        self.edit_pending.is_some() || self.unresolved_edit.is_some()
    }

    pub(crate) const fn edit_failed(&self) -> bool {
        self.edit_failed
    }

    #[cfg(test)]
    pub(crate) fn replace_compatible(
        &mut self,
        lowered: LoweredProject,
        profile: HostProfile,
        install: super::app::InstallSequence,
    ) -> Result<PlanId, SongEditError> {
        self.replace_compatible_guarded(lowered, profile, install, || true)
    }

    /// Preparation finishes before the host rechecks its input's identity lineage.
    pub(crate) fn replace_compatible_guarded(
        &mut self,
        lowered: LoweredProject,
        profile: HostProfile,
        install: super::app::InstallSequence,
        publish_allowed: impl FnOnce() -> bool,
    ) -> Result<PlanId, SongEditError> {
        if !self.edit_ready() {
            return Err(SongEditError::Busy);
        }
        if !partition_is_qualified(&profile) {
            return Err(SongEditError::Preparation(
                SongPlaybackError::CapacityUnqualified.to_string(),
            ));
        }
        if let Some(refusal) = lowered
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.severity() == Severity::Refused)
        {
            return Err(SongEditError::Preparation(format!(
                "{:?}: {:?}",
                refusal.subject(),
                refusal.reason()
            )));
        }
        let plan = lowered
            .plan
            .ok_or_else(|| SongEditError::Preparation("no edited plan".to_owned()))?;
        let observation = self.observers.prepare_edit(self.control.plan(), &plan);
        let events: Vec<_> = lowered
            .events
            .iter()
            .map(|event| PlanEvent::new(PlanPosition::new(event.time().as_u64()), event.payload()))
            .collect();
        let stream = AdmittedCompiledStream::admit(&plan, &events)
            .map_err(|error| SongEditError::Preparation(error.to_string()))?;
        let mut packet = self
            .control
            .prepare_compatible_replacement(plan, stream, profile, self.limits)
            .map_err(|error| match error {
                PlanReplacementError::Transport | PlanReplacementError::Full => SongEditError::Busy,
                PlanReplacementError::NoteObligations => {
                    SongEditError::Unsupported(error.to_string())
                }
                PlanReplacementError::Migration(
                    synth_engine_v2::render::GlobalMigrationError::Routing
                    | synth_engine_v2::render::GlobalMigrationError::UnidentifiedState { .. },
                ) => SongEditError::Unsupported(error.to_string()),
                _ => SongEditError::Preparation(error.to_string()),
            })?;
        let observation_mirror =
            match observation::SongObservationControl::attach(&mut packet, observation) {
                Ok(mirror) => mirror,
                Err(error) => {
                    if let Err((packet, cancellation)) = self.control.cancel_replacement(packet) {
                        self.unresolved_edit = Some(SongEdit { packet, install });
                        return Err(SongEditError::Preparation(cancellation.to_string()));
                    }
                    return Err(SongEditError::Preparation(error.to_string()));
                }
            };
        if !publish_allowed() {
            if let Err((packet, error)) = self.control.cancel_replacement(packet) {
                self.unresolved_edit = Some(SongEdit { packet, install });
                self.edit_failed = true;
                return Err(SongEditError::Preparation(error.to_string()));
            }
            return Err(SongEditError::Busy);
        }
        let plan = packet.plan_id();
        if let Err(edit) = self.edits.try_push(SongEdit { packet, install }) {
            match self.control.cancel_replacement(edit.packet) {
                Ok(_) => return Err(SongEditError::Busy),
                Err((packet, error)) => {
                    self.unresolved_edit = Some(SongEdit { packet, install });
                    return Err(SongEditError::Preparation(error.to_string()));
                }
            }
        }
        self.observers.published(observation_mirror);
        self.edit_pending = Some(PlanPosition::new(lowered.lowered_frames.as_u64()));
        Ok(plan)
    }

    /// Acknowledged playing, with no transport or edit receipt still in flight.
    pub(crate) fn running_edit_ready(&mut self) -> bool {
        self.collect();
        !self.is_faulted()
            && !self.edit_failed
            && self.outstanding == 0
            && !self.observers.changing()
            && !self.is_editing()
            && self.ending.is_none()
            && matches!(self.control.snapshot().playback, PlaybackState::Playing(_))
    }

    pub(crate) fn take_edit_completion(&mut self) -> Option<(super::app::InstallSequence, bool)> {
        self.edit_completion.take()
    }

    /// Prepare a bounded running edit; failure leaves the playing owner untouched.
    pub(crate) fn replace_running_guarded(
        &mut self,
        lowered: LoweredProject,
        profile: HostProfile,
        install: super::app::InstallSequence,
        preserve_graph_identity: bool,
        publish_allowed: impl FnOnce() -> bool,
    ) -> Result<(PlanId, PlanReplacementKind), SongEditError> {
        let preserve_graph_identity =
            preserve_graph_identity && lowered.lowered_frames.as_u64() == self.end.as_u64();
        if !self.running_edit_ready() {
            return Err(SongEditError::Busy);
        }
        if !partition_is_qualified(&profile) {
            return Err(SongEditError::Preparation(
                SongPlaybackError::CapacityUnqualified.to_string(),
            ));
        }
        if let Some(refusal) = lowered
            .diagnostics
            .iter()
            .find(|d| d.severity() == Severity::Refused)
        {
            return Err(SongEditError::Preparation(format!(
                "{:?}: {:?}",
                refusal.subject(),
                refusal.reason()
            )));
        }
        let plan = lowered
            .plan
            .ok_or_else(|| SongEditError::Preparation("no edited plan".to_owned()))?;
        let observation = if preserve_graph_identity {
            self.observers.prepare_edit(self.control.plan(), &plan)
        } else {
            self.observers.refuse_edit()
        };
        let events: Vec<_> = lowered
            .events
            .iter()
            .map(|event| PlanEvent::new(PlanPosition::new(event.time().as_u64()), event.payload()))
            .collect();
        let stream = AdmittedCompiledStream::admit(&plan, &events)
            .map_err(|error| SongEditError::Preparation(error.to_string()))?;
        let clock = Arc::clone(&self.clock);
        let margin = self.margin;
        let end = self.end;
        let mut packet = self
            .control
            .prepare_running_replacement(
                plan,
                stream,
                RunningReplacementRequest {
                    profile,
                    transfer: self.limits,
                    song_end: Some(PlanPosition::new(lowered.lowered_frames.as_u64())),
                    maximum_timeline_events: EventCount::measured(16_384),
                    mode: if preserve_graph_identity {
                        RunningEditMode::ContinueOrReset(continuation_limits())
                    } else {
                        RunningEditMode::ResetWithWitness(continuation_limits())
                    },
                },
                move |anchor| {
                    let quantum = u64::from(QUANTUM_FRAMES);
                    let earliest = clock.load(Ordering::Acquire).checked_add(margin)?;
                    let at = earliest.checked_add(quantum - 1)? / quantum * quantum;
                    let position = anchor
                        .position()
                        .as_u64()
                        .checked_add(at.checked_sub(anchor.time().as_u64())?)?;
                    // Leave the ordinary end-stop lookahead free of a frozen edit credit.
                    let stop_window = margin.checked_mul(4)?;
                    (position.checked_add(stop_window)? < end.as_u64())
                        .then_some(SampleTime::new(at))
                },
            )
            .map_err(|error| SongEditError::Unsupported(error.to_string()))?;
        let observation_mirror =
            match observation::SongObservationControl::attach(&mut packet, observation) {
                Ok(mirror) => mirror,
                Err(error) => {
                    if let Err((packet, cancellation)) = self.control.cancel_replacement(packet) {
                        self.unresolved_edit = Some(SongEdit { packet, install });
                        return Err(SongEditError::Preparation(cancellation.to_string()));
                    }
                    return Err(SongEditError::Preparation(error.to_string()));
                }
            };
        let kind = packet.kind();
        let continuation_unavailable = packet.continuation_unavailable();
        if kind == PlanReplacementKind::RunningCompatible && !publish_allowed() {
            if let Err((packet, error)) = self.control.cancel_replacement(packet) {
                self.unresolved_edit = Some(SongEdit { packet, install });
                self.edit_failed = true;
                return Err(SongEditError::Preparation(error.to_string()));
            }
            self.refusals.push(
                "graph identity changed during running preparation; the candidate was cancelled and the edit remains pending".to_owned(),
            );
            return Err(SongEditError::Busy);
        }
        let plan = packet.plan_id();
        if let Err(edit) = self.edits.try_push(SongEdit { packet, install }) {
            if let Err((packet, error)) = self.control.cancel_replacement(edit.packet) {
                self.unresolved_edit = Some(SongEdit { packet, install });
                self.edit_failed = true;
                return Err(SongEditError::Preparation(error.to_string()));
            }
            return Err(SongEditError::Busy);
        }
        if let Some(error) = continuation_unavailable {
            self.refusals.push(format!(
                "reset queued without a voice-continuation witness: {error}"
            ));
        }
        self.observers.published(observation_mirror);
        self.edit_pending = Some(PlanPosition::new(lowered.lowered_frames.as_u64()));
        Ok((plan, kind))
    }

    /// The immutable plan this control half and its paired audio half own.
    pub const fn plan_id(&self) -> PlanId {
        self.control.plan_id()
    }

    /// Whether the last acknowledged or submitted transport state is playing.
    #[must_use]
    pub const fn is_playing(&self) -> bool {
        self.playing
    }

    /// Whether the audio half's session faulted. A faulted session never renders again;
    /// the application prepares a fresh one.
    #[must_use]
    pub fn is_faulted(&self) -> bool {
        self.faulted.load(Ordering::Acquire)
    }

    /// Start or resume from the stopped position; returns the boundary it applies at.
    pub fn play(&mut self) -> Result<SampleTime, SongPlaybackError> {
        self.collect();
        if self.edit_failed {
            return Err(SongPlaybackError::Faulted);
        }
        if self.is_faulted() {
            return Err(SongPlaybackError::Faulted);
        }
        if self.ending.is_some() {
            return Err(SongPlaybackError::Ending);
        }
        let at = self.next_boundary();
        let mut packet = self
            .control
            .prepare_play(at)
            .map_err(|error| SongPlaybackError::Command(error.to_string()))?;
        let witness_error = packet
            .prepare_continuation_witness(continuation_limits())
            .err();
        self.submit(packet)?;
        if let Some(error) = witness_error {
            self.refusals.push(format!(
                "Play queued without a voice-continuation witness: {error}"
            ));
        }
        self.playing = true;
        Ok(at)
    }

    /// Stop at the next boundary, keeping the position for a later resume; returns the
    /// boundary it applies at.
    pub fn pause(&mut self) -> Result<SampleTime, SongPlaybackError> {
        self.collect();
        if self.edit_failed {
            return Err(SongPlaybackError::Faulted);
        }
        if self.is_faulted() {
            return Err(SongPlaybackError::Faulted);
        }
        // Already stopping at the end, which is at most a few callbacks away; an earlier
        // stop would be out of order for the session, so the pause is that stop.
        if let Some(end) = self.ending {
            self.playing = false;
            return Ok(end);
        }
        let at = self.next_boundary();
        let packet = self
            .control
            .prepare_stop(at)
            .map_err(|error| SongPlaybackError::Command(error.to_string()))?;
        self.submit(packet)?;
        self.playing = false;
        Ok(at)
    }

    /// Collect returned commands; returns how many had applied. A command the session did
    /// not apply is reported through [`Self::take_refusals`]. Once nothing is outstanding,
    /// the playing state is the session's acknowledged state, not the last request; a
    /// faulted session is stopped.
    pub fn collect(&mut self) -> usize {
        self.observers.poll();
        let mut applied = 0;
        if let Some(edit) = self.edited.try_pop() {
            let running = edit.packet.running_boundary().is_some();
            match self.control.collect_replacement(edit.packet) {
                Ok(receipt) => {
                    self.observers.edited(receipt.observation);
                    if let PlanReplacementOutcome::Installed(snapshot) = receipt.outcome {
                        if let Some(end) = self.edit_pending.take() {
                            self.end = end;
                        }
                        self.applied = match snapshot.playback {
                            PlaybackState::Stopped(position) => Some(position),
                            PlaybackState::Playing(anchor) => Some(anchor.position()),
                            PlaybackState::Unavailable => None,
                        };
                        self.edit_completion = Some((edit.install, true));
                    } else {
                        self.edit_pending = None;
                        self.edit_completion = Some((edit.install, false));
                        self.edit_failed = !running
                            || matches!(receipt.outcome, PlanReplacementOutcome::Interrupted(_));
                        self.refusals
                            .push(format!("V2 edit refused: {:?}", receipt.outcome));
                    }
                }
                Err((packet, error)) => {
                    self.unresolved_edit = Some(SongEdit {
                        packet,
                        install: edit.install,
                    });
                    self.edit_failed = true;
                    self.refusals
                        .push(format!("V2 paused edit collection failed: {error}"));
                }
            }
        }
        while let Some(packet) = self.returned.try_pop() {
            let boundary = packet.boundary();
            match self.control.cancel(packet) {
                Ok(_cancelled) => {
                    self.outstanding = self.outstanding.saturating_sub(1);
                    self.refusals.push(format!(
                        "{:?} at {} was not admitted by the audio half",
                        boundary.command,
                        boundary.at.as_u64()
                    ));
                }
                Err((packet, error)) => {
                    self.refusals.push(format!(
                        "a returned V2 command could not be cancelled: {error}"
                    ));
                    self.unresolved_commands.push(packet);
                    self.edit_failed = true;
                }
            }
        }
        while let Some(packet) = self.completed.try_pop() {
            match self.control.collect(packet) {
                Ok(receipt) => {
                    self.outstanding = self.outstanding.saturating_sub(1);
                    match receipt.outcome {
                        SessionOutcome::Applied { position } => {
                            self.applied = Some(position);
                            #[cfg(test)]
                            self.applied_log.push(position);
                            applied += 1;
                        }
                        outcome => self.refusals.push(format!(
                            "{:?} at {} was not applied: {outcome:?}",
                            receipt.boundary.command,
                            receipt.boundary.at.as_u64()
                        )),
                    }
                }
                Err((packet, error)) => {
                    self.refusals
                        .push(format!("a V2 receipt could not be collected: {error}"));
                    self.unresolved_completions.push(packet);
                    self.edit_failed = true;
                }
            }
        }
        if self.is_faulted() {
            // The session retains whatever it had admitted and never renders again, so no
            // outstanding command can complete; the state is stopped until re-preparation.
            if !self.fault_reported {
                self.fault_reported = true;
                self.refusals
                    .push("the V2 session faulted; the song must be prepared again".to_owned());
            }
            self.ending = None;
            self.playing = false;
        } else if self.outstanding == 0 {
            self.ending = None;
            self.playing = matches!(self.control.snapshot().playback, PlaybackState::Playing(_));
        }
        applied
    }

    /// Every command the session refused or cancelled since the last call, oldest first.
    pub fn take_refusals(&mut self) -> Vec<String> {
        std::mem::take(&mut self.refusals)
    }

    /// Call regularly from the control thread. Collects receipts and, when the acknowledged
    /// playback nears the arrangement's end, schedules the stop at that end (or at the
    /// earliest boundary still ahead, if the end has already passed).
    pub fn poll(&mut self) -> Result<(), SongPlaybackError> {
        self.collect();
        if self.is_faulted()
            || self.is_editing()
            || self.edit_failed
            || self.ending.is_some()
            || self.outstanding != 0
        {
            return Ok(());
        }
        let PlaybackState::Playing(anchor) = self.control.snapshot().playback else {
            return Ok(());
        };
        let remaining = self.end.as_u64().saturating_sub(anchor.position().as_u64());
        let end_at = anchor.time().as_u64().saturating_add(remaining);
        let clock = self.clock.load(Ordering::Acquire);
        if end_at > clock.saturating_add(4 * self.margin) {
            return Ok(());
        }
        let quantum = u64::from(QUANTUM_FRAMES);
        let at = SampleTime::new(end_at.div_ceil(quantum) * quantum).max(self.next_boundary());
        let packet = self
            .control
            .prepare_stop(at)
            .map_err(|error| SongPlaybackError::Command(error.to_string()))?;
        self.submit(packet)?;
        // Still playing until the session acknowledges the stop at the end.
        self.ending = Some(at);
        Ok(())
    }

    /// The song position of the last applied command: where playback started or stopped.
    #[must_use]
    pub const fn applied_position(&self) -> Option<PlanPosition> {
        self.applied
    }

    /// The first quantum boundary a margin past the audio half's published clock.
    fn next_boundary(&self) -> SampleTime {
        let quantum = u64::from(QUANTUM_FRAMES);
        let earliest = self.clock.load(Ordering::Acquire) + self.margin;
        SampleTime::new(earliest.div_ceil(quantum) * quantum)
    }

    fn submit(&mut self, packet: PreparedSessionCommand) -> Result<(), SongPlaybackError> {
        if let Err(packet) = self.commands.try_push(packet) {
            // The command never left this thread; cancelling returns its credit.
            if let Err((packet, error)) = self.control.cancel(packet) {
                // Kept with its credit rather than dropped, as in `collect`.
                self.unresolved_commands.push(packet);
                self.edit_failed = true;
                return Err(SongPlaybackError::Command(error.to_string()));
            }
            return Err(SongPlaybackError::Full);
        }
        self.outstanding += 1;
        Ok(())
    }
}

impl SongAudio {
    pub(crate) const fn install_sequence(&self) -> super::app::InstallSequence {
        self.install
    }

    pub(crate) fn set_install_sequence(&mut self, install: super::app::InstallSequence) {
        self.install = install;
    }

    /// Inject an early attempt to exercise the app's refused-receipt recovery.
    #[cfg(test)]
    pub(crate) fn attempt_edit_before_boundary(&mut self) {
        self.pending_edit = self.edits.try_pop();
        self.install_edit();
    }

    #[cfg(test)]
    pub(crate) fn edit_is_fading(&self) -> bool {
        self.pending_edit
            .as_ref()
            .is_some_and(|edit| edit.packet.is_rendering_transition())
    }

    #[cfg(test)]
    pub(crate) fn defer_edit_past_boundary(&mut self) {
        self.pending_edit = self.edits.try_pop();
        for _ in 0..40 {
            let mut output = [0.0; 512];
            assert!(
                self.render_session(
                    AudioBlockMut::new(
                        &mut output,
                        256,
                        synth_engine_v2::quantities::ChannelLayout::Stereo
                    )
                    .expect("block")
                )
            );
        }
    }

    fn install_edit(&mut self) {
        let Some(edit) = self.pending_edit.take() else {
            return;
        };
        let result = match (
            edit.packet.running_boundary().is_some(),
            self.observers.store_mut(),
        ) {
            (true, Some(store)) => self
                .audio
                .begin_running_replacement_observed(edit.packet, store),
            (false, Some(store)) => self.audio.install_replacement_observed(edit.packet, store),
            (true, None) => self.audio.begin_running_replacement(edit.packet),
            (false, None) => self.audio.install_replacement(edit.packet),
        };
        let packet = match result {
            Ok(packet) | Err((packet, _)) => packet,
        };
        if packet.outcome().is_none() {
            self.pending_edit = Some(SongEdit {
                packet,
                install: edit.install,
            });
            return;
        }
        if matches!(packet.outcome(), Some(PlanReplacementOutcome::Installed(_))) {
            self.install = edit.install;
        }
        if let Err(edit) = self.edited.try_push(SongEdit {
            packet,
            install: edit.install,
        }) {
            self.pending_edit = Some(edit);
        }
    }

    /// The session's high water for one producer class (EVD-0025's instrument).
    #[cfg(test)]
    pub(crate) const fn high_water(
        &self,
        class: synth_engine_v2::publish::ProducerClass,
    ) -> synth_engine_v2::quantities::EventCount {
        self.audio.high_water(class)
    }

    /// The session's high water of all external classes in one window.
    #[cfg(test)]
    pub(crate) const fn high_water_external_total(
        &self,
    ) -> synth_engine_v2::quantities::EventCount {
        self.audio.high_water_external_total()
    }

    /// Publication faults the session's renderer counted.
    #[cfg(test)]
    pub(crate) const fn publication_faults(&self) -> u64 {
        self.audio.publication_faults()
    }

    /// Render one callback. Admits queued commands first, returns completions after, and
    /// publishes the clock. A failure silences the block; the session stays faulted.
    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> bool {
        // Validate the original callback before any split. Splitting cannot make an
        // oversized callback admissible or conceal a terminal session fault.
        if u64::try_from(output.frames()).unwrap_or(u64::MAX) > self.maximum_frames.as_u64()
            || output.frames() == 0
            || output.layout() != self.layout
        {
            return self.render_edit(output);
        }
        let _observation_service = self.observers.service(self.audio.plan_id());
        if self.pending_edit.is_none() {
            self.pending_edit = self.edits.try_pop();
        }
        if self
            .pending_edit
            .as_ref()
            .is_some_and(|edit| edit.packet.outcome().is_some())
        {
            // A retained return is never attempted a second time.
            if let Some(edit) = self.pending_edit.take()
                && let Err(edit) = self.edited.try_push(edit)
            {
                self.pending_edit = Some(edit);
            }
        } else if self
            .pending_edit
            .as_ref()
            .is_some_and(|edit| edit.packet.is_rendering_transition())
        {
            return self.render_edit(output);
        } else if self.pending_edit.is_some() {
            let distance = self
                .pending_edit
                .as_ref()
                .filter(|edit| edit.packet.running_boundary().is_some())
                .map_or_else(
                    || self.audio.frames_until_plan_boundary(),
                    |edit| {
                        self.audio
                            .frames_until_running_boundary(&edit.packet)
                            .unwrap_or(FrameCount::new(0))
                    },
                );
            let carry = usize::try_from(distance.as_u64()).unwrap_or(usize::MAX);
            if carry == 0 {
                self.install_edit();
            } else if carry < output.frames()
                && let Ok((head, tail)) = output.reborrow().split_at_frame(carry)
            {
                let first = self.render_session(head);
                if first {
                    self.install_edit();
                }
                let second = self.render_edit(tail);
                if !first || !second {
                    output.silence();
                }
                return first && second;
            }
        }
        self.render_edit(output)
    }

    fn render_edit(&mut self, mut output: AudioBlockMut<'_>) -> bool {
        if !self
            .pending_edit
            .as_ref()
            .is_some_and(|edit| edit.packet.is_rendering_transition())
        {
            return self.render_session(output);
        }
        let Some(edit) = self.pending_edit.as_mut() else {
            return false;
        };
        let rendered = if let Some(store) = self.observers.store_mut() {
            self.audio.render_running_replacement_observed(
                &mut edit.packet,
                output.reborrow(),
                store,
            )
        } else {
            self.audio
                .render_running_replacement(&mut edit.packet, output.reborrow())
        }
        .is_ok();
        if !rendered {
            if let Some(store) = self.observers.store_mut() {
                store.end();
            }
            output.silence();
            self.faulted.store(true, Ordering::Release);
        } else if matches!(
            edit.packet.outcome(),
            Some(PlanReplacementOutcome::Installed(_))
        ) {
            self.install = edit.install;
        }
        self.clock
            .store(self.audio.clock().as_u64(), Ordering::Release);
        if edit.packet.outcome().is_some()
            && let Some(edit) = self.pending_edit.take()
            && let Err(edit) = self.edited.try_push(edit)
        {
            self.pending_edit = Some(edit);
        }
        rendered
    }

    fn render_session(&mut self, mut output: AudioBlockMut<'_>) -> bool {
        // A refused command, and every command queued behind it, goes back to the control
        // half for cancellation: once the session refuses one, order forbids the rest.
        if let Some(packet) = self.refused.take()
            && let Err(packet) = self.returns.try_push(packet)
        {
            self.refused = Some(packet);
        }
        let prefix = self.commands.occupied_len().min(COMMAND_SLOTS);
        for _ in 0..prefix {
            if self.refused.is_some() {
                break;
            }
            let Some(packet) = self.commands.try_pop() else {
                break;
            };
            if let Err((packet, _refusal)) = self.audio.enqueue(packet) {
                // The refusal travels back as the command itself; control reports it.
                if let Err(packet) = self.returns.try_push(packet) {
                    self.refused = Some(packet);
                }
                while !self.returns.is_full() {
                    let Some(queued) = self.commands.try_pop() else {
                        break;
                    };
                    if let Err(queued) = self.returns.try_push(queued) {
                        self.refused = Some(queued);
                        break;
                    }
                }
                break;
            }
        }
        let rendered = if let Some(store) = self.observers.store_mut() {
            self.audio.render_observed(output.reborrow(), Some(store))
        } else {
            self.audio.render(output.reborrow())
        }
        .is_ok();
        if !rendered {
            if let Some(store) = self.observers.store_mut() {
                store.end();
            }
            output.silence();
            self.faulted.store(true, Ordering::Release);
        }
        for _ in 0..COMMAND_SLOTS {
            if self.completed.is_full() {
                break;
            }
            let Some(packet) = self.pending.take().or_else(|| self.audio.take_completed()) else {
                break;
            };
            if let Err(packet) = self.completed.try_push(packet) {
                self.pending = Some(packet);
                break;
            }
        }
        self.clock
            .store(self.audio.clock().as_u64(), Ordering::Release);
        rendered
    }
}

#[cfg(test)]
#[path = "tests/evd_0025.rs"]
mod evd_0025;

#[cfg(test)]
mod tests {
    use super::*;

    /// A command delivered after its boundary is refused by the session. The refusal is
    /// surfaced, and the reported state follows the session rather than the request.
    #[test]
    fn a_late_command_is_surfaced_and_the_state_follows_the_session() {
        let project = crate::lowering::tests::live_fixture_project("subtractive-voice");
        let profile = crate::lowering::tests::live_fixture_profile();
        let lowered = super::super::lower_project(
            &project.instruments,
            &project.song,
            &project.global,
            profile,
            super::super::render::OutputPolicy::Parity,
        );
        let (mut control, mut audio) = prepare_unqualified(lowered, profile).expect("prepares");
        let block =
            usize::try_from(profile.capabilities().maximum_block_size().as_u64()).expect("frames");
        let mut output = vec![0.0; block * 2];
        for _ in 0..10 {
            assert!(
                audio.render(
                    AudioBlockMut::new(
                        &mut output,
                        block,
                        synth_engine_v2::quantities::ChannelLayout::Stereo
                    )
                    .expect("block")
                )
            );
        }
        // Scheduled at the long-passed boundary zero, as a stale clock would.
        let packet = control
            .control
            .prepare_play(SampleTime::ZERO)
            .expect("the control half still accepts the acknowledged boundary");
        control.submit(packet).expect("queued");
        control.playing = true;
        assert!(
            audio.render(
                AudioBlockMut::new(
                    &mut output,
                    block,
                    synth_engine_v2::quantities::ChannelLayout::Stereo
                )
                .expect("block")
            )
        );
        assert_eq!(control.collect(), 0, "nothing applied");
        assert_eq!(control.take_refusals().len(), 1, "the refusal is surfaced");
        assert!(!control.is_playing(), "the state follows the session");
    }

    /// After a session fault the audio half cannot admit a command; it returns the command,
    /// the control half cancels it and reports it, and the state stays stopped.
    #[test]
    fn a_command_the_audio_half_cannot_admit_returns_and_is_reported() {
        let project = crate::lowering::tests::live_fixture_project("subtractive-voice");
        let profile = crate::lowering::tests::live_fixture_profile();
        let lowered = super::super::lower_project(
            &project.instruments,
            &project.song,
            &project.global,
            profile,
            super::super::render::OutputPolicy::Parity,
        );
        let (mut control, mut audio) = prepare_unqualified(lowered, profile).expect("prepares");
        let block =
            usize::try_from(profile.capabilities().maximum_block_size().as_u64()).expect("frames");
        let stereo = synth_engine_v2::quantities::ChannelLayout::Stereo;
        let mut oversized = vec![9.0; (block + 1) * 2];
        assert!(
            !audio.render(AudioBlockMut::new(&mut oversized, block + 1, stereo).expect("block")),
            "an oversized callback faults the session"
        );
        assert!(oversized.iter().all(|sample| *sample == 0.0));
        assert!(matches!(control.play(), Err(SongPlaybackError::Faulted)));
        // A command submitted before the control half saw the fault, as a race would.
        let packet = control
            .control
            .prepare_play(SampleTime::new(1_024))
            .expect("the control half still accepts it");
        control.submit(packet).expect("queued");
        let mut output = vec![0.0; block * 2];
        assert!(!audio.render(AudioBlockMut::new(&mut output, block, stereo).expect("block")));
        assert_eq!(control.collect(), 0, "nothing applied");
        let refusals = control.take_refusals();
        assert_eq!(
            refusals.len(),
            2,
            "the fault and the returned command: {refusals:?}"
        );
        assert!(!control.is_playing());
        assert_eq!(control.outstanding, 0, "no command is stranded");
        assert!(control.unresolved_commands.is_empty());
    }

    /// A pause while the end stop is outstanding is that stop, not an out-of-order refusal.
    #[test]
    fn a_pause_near_the_end_is_the_end_stop() {
        let project = crate::lowering::tests::live_fixture_project("subtractive-voice");
        let profile = crate::lowering::tests::live_fixture_profile();
        let lowered = super::super::lower_project(
            &project.instruments,
            &project.song,
            &project.global,
            profile,
            super::super::render::OutputPolicy::Parity,
        );
        let end = lowered.lowered_frames.as_u64();
        let (mut control, mut audio) = prepare_unqualified(lowered, profile).expect("prepares");
        let block =
            usize::try_from(profile.capabilities().maximum_block_size().as_u64()).expect("frames");
        let stereo = synth_engine_v2::quantities::ChannelLayout::Stereo;
        let mut output = vec![0.0; block * 2];
        let mut render = |audio: &mut SongAudio| {
            assert!(audio.render(AudioBlockMut::new(&mut output, block, stereo).expect("block")));
        };
        render(&mut audio);
        let _at = control.play().expect("play");
        let mut guard = 0;
        while control.ending.is_none() {
            render(&mut audio);
            control.poll().expect("poll");
            guard += 1;
            assert!(guard < 1000, "the end stop is scheduled");
        }
        let scheduled = control.ending.expect("outstanding end stop");
        assert_eq!(control.pause().expect("pause near the end"), scheduled);
        assert!(!control.is_playing());
        while control.outstanding != 0 {
            render(&mut audio);
            control.poll().expect("poll");
        }
        assert!(control.take_refusals().is_empty(), "nothing was refused");
        assert!(control.applied_position().expect("stopped").as_u64() >= end);
        assert!(!control.is_playing());
    }

    /// A command the session admitted before a render fault is retained by the dead
    /// session; the control half reports the fault and stops, instead of waiting for it.
    #[test]
    fn a_fault_after_admission_stops_the_reported_state() {
        let project = crate::lowering::tests::live_fixture_project("subtractive-voice");
        let profile = crate::lowering::tests::live_fixture_profile();
        let lowered = super::super::lower_project(
            &project.instruments,
            &project.song,
            &project.global,
            profile,
            super::super::render::OutputPolicy::Parity,
        );
        let (mut control, mut audio) = prepare_unqualified(lowered, profile).expect("prepares");
        let block =
            usize::try_from(profile.capabilities().maximum_block_size().as_u64()).expect("frames");
        let stereo = synth_engine_v2::quantities::ChannelLayout::Stereo;
        let _at = control.play().expect("play");
        assert!(control.is_playing());
        let mut oversized = vec![9.0; (block + 1) * 2];
        assert!(
            !audio.render(AudioBlockMut::new(&mut oversized, block + 1, stereo).expect("block"))
        );
        assert!(control.is_faulted());
        assert_eq!(control.collect(), 0);
        assert!(!control.is_playing(), "a faulted session is stopped");
        assert_eq!(
            control.take_refusals().len(),
            1,
            "the fault is reported once"
        );
        control.poll().expect("poll after a fault");
        assert!(control.take_refusals().is_empty());
        assert!(matches!(control.pause(), Err(SongPlaybackError::Faulted)));
    }
}

#[cfg(test)]
#[path = "tests/paused_edit.rs"]
mod paused_edit;
