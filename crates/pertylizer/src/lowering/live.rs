//! Experimental live song playback through Core V2 (ADR-0077).
//!
//! A lowered project plays through the ordered session transport: [`SongPlayback`] is the
//! control half, [`SongAudio`] the audio-callback half. Commands cross in a bounded
//! single-producer ring and return in another, so the callback neither allocates, frees,
//! locks nor waits. The control half schedules each command a margin past the clock the
//! audio half last published, on a quantum boundary as the session requires.
//!
//! Only play, pause and resume exist here. Stop-to-start and edits are a fresh
//! [`prepare_song`] installed while stopped by the caller; seek and loop refuse at the
//! application (ADR-0077 decision 2).
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
            CompletedSessionCommand, PreparedSessionCommand, SessionAudio, SessionControl,
            SessionTransferLimits,
        },
    },
    profile::HostProfile,
    quantities::PreparedBytes,
    render::AudioBlockMut,
    schedule::{AdmittedCompiledStream, PlanEvent},
    time::{PlanPosition, QUANTUM_FRAMES, SampleTime},
};
use thiserror::Error;

use super::{LoweredProject, Severity};

/// Command cells in each direction; play and pause need one each, the rest is headroom.
const COMMAND_SLOTS: usize = 8;

/// The song-playback partition an accepted evidence record has selected (ADR-0054 clause 3,
/// ADR-0077 decision 6). EVD-0025 selected compiled 96 and session 128 as bounds enforced
/// before playback and the per-quantum cap 360, for this path alone; every other share is
/// unreachable here. A profile with any other value is not qualified.
pub const QUALIFIED_COMPILED_SHARE: u32 = 96;
pub const QUALIFIED_SESSION_SHARE: u32 = 128;
pub const QUALIFIED_EVENTS_PER_QUANTUM: u32 = 360;

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

/// Control half, owned by the application thread.
#[must_use]
pub struct SongPlayback {
    control: SessionControl,
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
    audio: SessionAudio,
    commands: HeapCons<PreparedSessionCommand>,
    completed: HeapProd<CompletedSessionCommand>,
    pending: Option<CompletedSessionCommand>,
    /// A command the session would not admit, waiting for room in the return ring.
    refused: Option<PreparedSessionCommand>,
    returns: HeapProd<PreparedSessionCommand>,
    clock: Arc<AtomicU64>,
    faulted: Arc<AtomicBool>,
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
        control_bytes: PreparedBytes::limit(16_384).map_err(|error| preparation(&error))?,
    };
    let maximum = profile.capabilities().maximum_block_size().as_u64();
    let end = PlanPosition::new(lowered.lowered_frames.as_u64());
    let (control, audio) = SessionControl::prepare(plan, stream, profile, limits)
        .map_err(|error| preparation(&error))?;
    let (commands, command_reader) = HeapRb::new(COMMAND_SLOTS).split();
    let (completion_writer, completed) = HeapRb::new(COMMAND_SLOTS).split();
    let (returns, returned) = HeapRb::new(COMMAND_SLOTS).split();
    let clock = Arc::new(AtomicU64::new(0));
    let faulted = Arc::new(AtomicBool::new(false));
    Ok((
        SongPlayback {
            control,
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
            audio,
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
        if self.is_faulted() {
            return Err(SongPlaybackError::Faulted);
        }
        if self.ending.is_some() {
            return Err(SongPlaybackError::Ending);
        }
        let at = self.next_boundary();
        let packet = self
            .control
            .prepare_play(at)
            .map_err(|error| SongPlaybackError::Command(error.to_string()))?;
        self.submit(packet)?;
        self.playing = true;
        Ok(at)
    }

    /// Stop at the next boundary, keeping the position for a later resume; returns the
    /// boundary it applies at.
    pub fn pause(&mut self) -> Result<SampleTime, SongPlaybackError> {
        self.collect();
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
        let mut applied = 0;
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
        if self.is_faulted() || self.ending.is_some() || self.outstanding != 0 {
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
                return Err(SongPlaybackError::Command(error.to_string()));
            }
            return Err(SongPlaybackError::Full);
        }
        self.outstanding += 1;
        Ok(())
    }
}

impl SongAudio {
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
        let rendered = self.audio.render(output.reborrow()).is_ok();
        if !rendered {
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
