//! The application's experimental V2 song playback (ADR-0077): a switch around the V1
//! engine's audio processor, and its control half on the GUI thread.
//!
//! The V1 engine keeps receiving and processing every command the application sends; while
//! V2 mode is active its output is rendered into scratch and discarded, and the V2 session
//! supplies the audio. V2 has its own play, pause and stop; the V1 transport does not
//! drive it, so V2 never has to infer what V1 did. Stop prepares a fresh session at the
//! song start. Sessions cross to the callback and back through bounded rings, so the
//! callback neither allocates nor frees.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use synth_core::audio::{AudioCallbackContext, AudioError, AudioProcessor, StreamInfo};
use synth_engine_v2::{
    profile::HostProfile,
    quantities::{ChannelLayout, SampleRate},
    render::AudioBlockMut,
    time::FrameCount,
};

use super::live::{SongAudio, SongPlayback, SongPlaybackError, prepare_song};
use super::render::{OutputPolicy, lower_project};

/// The largest callback the V1 engine accepts, and the V2 profile's maximum block.
pub const MAX_CALLBACK_FRAMES: usize = 4096;
/// V2 song playback renders stereo only; any other device layout refuses V2 mode.
const CHANNELS: usize = 2;
/// Sessions in flight between the halves; one active and one retiring is the steady state.
const SLOTS: usize = 4;

enum SwitchCommand {
    /// A session and its install sequence number, counted from 1 in queue order.
    Install(u64, Box<SongAudio>),
}

/// The audio processor the application hands to its output stream.
#[must_use]
pub struct EngineSwitch<P: AudioProcessor> {
    v1: P,
    active: Option<Box<SongAudio>>,
    commands: HeapCons<SwitchCommand>,
    retired: HeapProd<Box<SongAudio>>,
    /// A session waiting for room in the retirement ring; never dropped here.
    retiring: Option<Box<SongAudio>>,
    /// The session V2 mode's removal took out, waiting for room in the retirement ring.
    removed: Option<Box<SongAudio>>,
    /// Set by the control half to leave V2 mode; it cannot fail to be delivered. While set,
    /// nothing is heard; it is honoured once every queued install has been admitted.
    remove: Arc<AtomicBool>,
    /// Set by the control half to silence V2 at once: V2 is heard only once the install
    /// with at least this sequence number is active, so a session queued before the
    /// silencing request cannot end it.
    muted_until: Arc<AtomicU64>,
    /// The sequence number of the active session's install.
    admitted: u64,
    scratch: Box<[f32]>,
}

/// The control half, owned by the GUI thread.
#[must_use]
pub struct SongSwitch {
    commands: HeapProd<SwitchCommand>,
    retired: HeapCons<Box<SongAudio>>,
    playback: Option<SongPlayback>,
    /// `None` when the device reported a rate V2 cannot represent; V2 mode then refuses.
    rate: Option<SampleRate>,
    channels: usize,
    /// Whether the callback's slot holds a session once every queued command is admitted.
    occupied: bool,
    /// Shared with the callback: leave V2 mode, and silence V2 until a given install.
    remove: Arc<AtomicBool>,
    muted_until: Arc<AtomicU64>,
    /// The sequence number of the last queued install.
    sent: u64,
    /// Sessions the queued commands will retire and the control half has not yet collected.
    retiring: usize,
    /// An edit arrived while playing; it is lowered at the next stop or pause.
    rebuild_pending: bool,
    /// The project was refused after an edit; V2 must not play, and the application leaves
    /// V2 mode through its own path, which resets V1 first (see [`Self::needs_disable`]).
    disable_pending: bool,
    notices: Vec<String>,
    /// What the current session's lowering could not represent; replaced on every rebuild
    /// and shown for as long as V2 mode is active.
    omissions: Vec<String>,
}

/// Wrap the V1 processor. The switch starts in V1 mode, and V2 mode refuses until the
/// stream's actual configuration is known through [`SongSwitch::set_stream`].
pub fn wrap<P: AudioProcessor>(v1: P) -> (SongSwitch, EngineSwitch<P>) {
    let (commands, command_reader) = HeapRb::new(SLOTS).split();
    let (retired, retired_reader) = HeapRb::new(SLOTS).split();
    let remove = Arc::new(AtomicBool::new(false));
    let muted_until = Arc::new(AtomicU64::new(0));
    (
        SongSwitch {
            commands,
            retired: retired_reader,
            playback: None,
            rate: None,
            channels: 0,
            occupied: false,
            remove: Arc::clone(&remove),
            muted_until: Arc::clone(&muted_until),
            sent: 0,
            retiring: 0,
            rebuild_pending: false,
            disable_pending: false,
            notices: Vec::new(),
            omissions: Vec::new(),
        },
        EngineSwitch {
            v1,
            active: None,
            commands: command_reader,
            retired,
            retiring: None,
            removed: None,
            remove,
            muted_until,
            admitted: 0,
            scratch: vec![0.0; MAX_CALLBACK_FRAMES * CHANNELS.max(8)].into_boxed_slice(),
        },
    )
}

impl<P: AudioProcessor> EngineSwitch<P> {
    fn retire(&mut self, session: Box<SongAudio>) {
        if let Err(session) = self.retired.try_push(session) {
            // Room returns when the control half collects; the session waits here.
            self.retiring = Some(session);
        }
    }

    fn admit_commands(&mut self) {
        if let Some(session) = self.retiring.take() {
            self.retire(session);
        }
        // A new command needs room to retire the session it replaces.
        while self.retiring.is_none() {
            let Some(command) = self.commands.try_pop() else {
                break;
            };
            let SwitchCommand::Install(sequence, session) = command;
            let previous = self.active.replace(session);
            self.admitted = sequence;
            if let Some(previous) = previous {
                self.retire(previous);
            }
        }
    }

    /// Leave V2 mode once every queued install has been admitted, so no session queued
    /// before the removal can return after it. Returns whether removal is still pending.
    fn admit_removal(&mut self) -> bool {
        if let Some(session) = self.removed.take()
            && let Err(session) = self.retired.try_push(session)
        {
            self.removed = Some(session);
        }
        if !self.remove.load(Ordering::Acquire) {
            return false;
        }
        if !self.commands.is_empty() || self.removed.is_some() {
            return true;
        }
        if let Some(session) = self.active.take()
            && let Err(session) = self.retired.try_push(session)
        {
            self.removed = Some(session);
        }
        self.remove.store(false, Ordering::Release);
        false
    }
}

impl<P: AudioProcessor> AudioProcessor for EngineSwitch<P> {
    fn process(&mut self, output: &mut [f32], context: &AudioCallbackContext) {
        self.admit_commands();
        let removing = self.admit_removal();
        let Some(song) = self.active.as_mut() else {
            self.v1.process(output, context);
            return;
        };
        // V1 still drains its commands and advances; its audio is not heard in V2 mode. It is
        // never handed a callback above the ceiling V2 prepared for, which V1 would meet by
        // growing its buffers on the audio thread; it skips that callback instead.
        let samples = output.len();
        if context.frames <= MAX_CALLBACK_FRAMES
            && let Some(scratch) = self.scratch.get_mut(..samples)
        {
            self.v1.process(scratch, context);
        }
        if removing || self.admitted < self.muted_until.load(Ordering::Acquire) {
            output.fill(0.0);
            return;
        }
        // An oversized callback reaches the session, whose terminal fault silences it and
        // asks for re-preparation (IO-INV-002); it is never skipped or split here.
        let rendered = usize::from(context.channels) == CHANNELS
            && AudioBlockMut::new(output, context.frames, ChannelLayout::Stereo)
                .is_ok_and(|block| song.render(block));
        if !rendered {
            output.fill(0.0);
        }
    }

    fn on_error(&mut self, error: AudioError) {
        self.v1.on_error(error);
    }

    fn on_stream_start(&mut self, info: &StreamInfo) {
        self.v1.on_stream_start(info);
    }

    fn on_stream_stop(&mut self) {
        self.v1.on_stream_stop();
    }
}

impl SongSwitch {
    /// Record the output stream's actual configuration, once it has started.
    pub fn set_stream(&mut self, stream: &StreamInfo) {
        // Device rates are far below 2^24, where every integer is exact in f32.
        #[allow(
            clippy::cast_precision_loss,
            reason = "an audio device rate fits f32 exactly"
        )]
        let rate = SampleRate::new(stream.sample_rate.as_u32() as f32).ok();
        self.rate = rate;
        self.channels = usize::from(stream.channels.count());
    }

    /// Whether V2 mode is active.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.playback.is_some()
    }

    /// Whether V2 has fully left the callback: not active, and every session it held has
    /// been retired and collected. Only then may MIDI reach V1 again.
    #[must_use]
    pub const fn is_released(&self) -> bool {
        self.playback.is_none() && !self.occupied && self.retiring == 0
    }

    /// Whether the V2 song is playing, as the session last acknowledged or was asked.
    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playback.as_ref().is_some_and(SongPlayback::is_playing)
    }

    /// What the active session omits, for a persistent indicator; empty outside V2 mode.
    #[must_use]
    pub fn omissions(&self) -> &[String] {
        if self.playback.is_some() {
            &self.omissions
        } else {
            &[]
        }
    }

    /// Notices for the application to show, oldest first.
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    /// The profile V2 renders with: the device rate, stereo, the V1 callback ceiling, and the
    /// engine's default partition, which EVD-0025 qualified.
    fn profile(&self) -> Result<HostProfile, String> {
        let rate = self.rate.ok_or_else(|| {
            "the output stream's sample rate is unknown or unusable for V2".to_owned()
        })?;
        if self.channels != CHANNELS {
            return Err(format!(
                "V2 playback renders stereo only; the device has {} channels",
                self.channels
            ));
        }
        HostProfile::harness(
            rate,
            FrameCount::new(MAX_CALLBACK_FRAMES as u64),
            ChannelLayout::Stereo,
        )
        .map_err(|error| error.to_string())
    }

    fn prepare(
        &self,
        project: &crate::project::ProjectFile,
    ) -> Result<(SongPlayback, SongAudio, Vec<String>), String> {
        let profile = self.profile()?;
        let lowered = lower_project(
            &project.instruments,
            &project.song,
            &project.global,
            profile,
            OutputPolicy::Parity,
        );
        // Behaviour the lowering could not represent plays without it; say what it was.
        let unrepresented = lowered
            .diagnostics
            .iter()
            .filter(|d| d.severity() == super::Severity::Unrepresented)
            .map(|d| format!("V2 playback omits: {:?}: {:?}", d.subject(), d.reason()))
            .collect();
        let (playback, audio) =
            prepare_song(lowered, profile).map_err(|error: SongPlaybackError| error.to_string())?;
        Ok((playback, audio, unrepresented))
    }

    /// Queue a session for the callback; false when the command ring is full.
    fn install(&mut self, audio: SongAudio) -> bool {
        let sequence = self.sent + 1;
        if self
            .commands
            .try_push(SwitchCommand::Install(sequence, Box::new(audio)))
            .is_err()
        {
            return false;
        }
        self.sent = sequence;
        if self.occupied {
            self.retiring += 1;
        }
        self.occupied = true;
        true
    }

    /// Enter V2 mode with the project as it is now, stopped at the song start. The caller
    /// stops V1 first. On refusal V1 stays active and the reason is a notice.
    pub fn enable(&mut self, project: &crate::project::ProjectFile) -> bool {
        self.collect_retired();
        if self.playback.is_some() {
            return true;
        }
        if self.remove.load(Ordering::Acquire) {
            self.notices
                .push("V2 playback is still switching off; try again".to_owned());
            return false;
        }
        match self.prepare(project) {
            Ok((playback, audio, unrepresented)) => {
                // Whatever silenced an earlier V2 mode ends with this session.
                self.muted_until.store(self.sent + 1, Ordering::Release);
                if !self.install(audio) {
                    self.notices
                        .push("V2 playback is busy switching; try again".to_owned());
                    return false;
                }
                self.playback = Some(playback);
                self.rebuild_pending = false;
                self.omissions = unrepresented;
                self.notices.push(
                    "V2 playback active: use the V2 play, pause and stop controls; the V1 \
                     transport does not affect V2; MIDI input is off until V2 mode ends \
                     (ADR-0022); seek, loop and pattern preview are not available"
                        .to_owned(),
                );
                true
            }
            Err(reason) => {
                self.notices.push(format!("V2 playback refused: {reason}"));
                false
            }
        }
    }

    /// Leave V2 mode. The removal cannot fail to reach the callback: from its next callback
    /// V2 is silent, and V1 is heard again once every queued install has been admitted.
    pub fn disable(&mut self) {
        if self.playback.is_none() {
            return;
        }
        self.remove.store(true, Ordering::Release);
        if self.occupied {
            self.retiring += 1;
        }
        self.occupied = false;
        self.playback = None;
        self.rebuild_pending = false;
        self.disable_pending = false;
    }

    /// Replace the session with a fresh one from the project, stopped at the song start.
    /// Returns whether the new session was queued; otherwise the old one stays and the
    /// rebuild stays pending.
    fn rebuild(&mut self, project: &crate::project::ProjectFile) -> bool {
        // After a refusal nothing may lift the silence; only leaving V2 mode ends it.
        if self.disable_pending {
            return false;
        }
        match self.prepare(project) {
            Ok((playback, audio, unrepresented)) => {
                if self.install(audio) {
                    self.playback = Some(playback);
                    self.rebuild_pending = false;
                    self.omissions = unrepresented;
                    true
                } else {
                    self.rebuild_pending = true;
                    false
                }
            }
            Err(reason) => {
                self.notices
                    .push(format!("V2 playback stops: the edited project {reason}"));
                // The refused project must never play: V2 is silenced from the next callback,
                // whatever any ring holds. Leaving V2 mode is the application's, so that V1 is
                // reset before it is heard again.
                self.disable_pending = true;
                self.muted_until.store(u64::MAX, Ordering::Release);
                false
            }
        }
    }

    /// Apply a deferred edit; false while it cannot be installed yet.
    fn apply_pending(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) -> bool {
        if self.rebuild(&project()) {
            self.notices.push(
                "V2 playback: the edit is applied; playback restarts from the song start"
                    .to_owned(),
            );
            true
        } else {
            false
        }
    }

    /// Play from the start or from where V2 paused. A deferred edit is applied first.
    pub fn play(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) {
        if self.disable_pending {
            self.notices
                .push("V2 playback is switching off after a refused edit".to_owned());
            return;
        }
        if let Some(playback) = self.playback.as_ref()
            && playback.is_playing()
        {
            return;
        }
        // An edit waiting to be applied must take effect before playing; the old session is
        // never started in its place.
        if self.rebuild_pending && !self.is_playing() && !self.apply_pending(project) {
            self.notices
                .push("V2 playback is busy applying the edit; try again".to_owned());
            return;
        }
        if let Some(playback) = self.playback.as_mut()
            && let Err(error) = playback.play()
        {
            self.notices
                .push(format!("V2 playback could not start: {error}"));
        }
    }

    /// Pause where the song is; play resumes there.
    pub fn pause(&mut self) {
        if let Some(playback) = self.playback.as_mut()
            && let Err(error) = playback.pause()
        {
            self.notices
                .push(format!("V2 playback could not pause: {error}"));
        }
    }

    /// Stop and return to the song start, with any deferred edit applied.
    pub fn stop(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) {
        if self.playback.is_none() || self.disable_pending {
            return;
        }
        // Silent from the next callback until the next install, the fresh session, is active;
        // a session queued earlier stays silent.
        self.muted_until.store(self.sent + 1, Ordering::Release);
        // A refusal has its own notice and leaves V2 mode; retrying stop would not help.
        if !self.rebuild(&project()) && !self.disable_pending {
            self.notices.push(
                "V2 playback is busy switching; stop again to return to the start".to_owned(),
            );
        }
    }

    /// Note that the project changed. Stopped, it is lowered now; playing, at the next
    /// play, pause or stop. `project` is only called when a rebuild happens.
    pub fn project_changed(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) {
        let Some(playback) = self.playback.as_ref() else {
            return;
        };
        if playback.is_playing() {
            if !self.rebuild_pending {
                self.notices
                    .push("V2 playback: the edit applies when playback stops or pauses".to_owned());
            }
            self.rebuild_pending = true;
        } else {
            self.rebuild(&project());
        }
    }

    /// Call every GUI frame. Collects receipts and retired sessions, reports refusals, and
    /// prepares a fresh session after a fault or, once paused, for a deferred edit.
    pub fn poll(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) {
        self.collect_retired();
        if self.disable_pending {
            // The refused project's session is muted; the application leaves V2 mode.
            return;
        }
        let Some(playback) = self.playback.as_mut() else {
            return;
        };
        if let Err(error) = playback.poll() {
            self.notices.push(format!("V2 playback: {error}"));
        }
        for refusal in playback.take_refusals() {
            self.notices.push(format!("V2 playback: {refusal}"));
        }
        let faulted = playback.is_faulted();
        let playing = playback.is_playing();
        if faulted {
            if self.rebuild(&project()) {
                self.notices.push(
                    "V2 playback faulted and was prepared again at the song start".to_owned(),
                );
            }
        } else if self.rebuild_pending && !playing {
            // A failed install stays pending and is retried on the next poll.
            let _applied = self.apply_pending(project);
        }
    }

    /// Destroy sessions the callback has retired, here on the GUI thread.
    fn collect_retired(&mut self) {
        while let Some(session) = self.retired.try_pop() {
            self.retiring = self.retiring.saturating_sub(1);
            drop(session);
        }
    }

    /// Whether the project was refused after an edit, so the application must leave V2 mode
    /// through its path that resets V1 first.
    #[must_use]
    pub const fn needs_disable(&self) -> bool {
        self.disable_pending
    }

    /// Whether the callback has room for another switch command.
    #[must_use]
    pub fn can_switch(&self) -> bool {
        !self.commands.is_full()
    }
}
