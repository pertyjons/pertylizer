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
use sha2::{Digest, Sha256};
use synth_core::audio::{AudioCallbackContext, AudioError, AudioProcessor, StreamInfo};
use synth_engine_v2::{
    plan::PlanId,
    profile::HostProfile,
    quantities::{ChannelLayout, SampleRate},
    render::AudioBlockMut,
    time::FrameCount,
};

use super::live::{SongAudio, SongEditError, SongPlayback, SongPlaybackError, prepare_song};
use super::render::{OutputPolicy, lower_project};

/// The largest callback the V1 engine accepts, and the V2 profile's maximum block.
pub const MAX_CALLBACK_FRAMES: usize = 4096;
/// V2 song playback renders stereo only; any other device layout refuses V2 mode.
const CHANNELS: usize = 2;
/// Sessions in flight between the halves; one active and one retiring is the steady state.
const SLOTS: usize = 4;

/// A queued V2 session installation. Zero means no rendered installation and the
/// maximum value is reserved for an indefinite mute, so neither is issued.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[must_use]
pub struct InstallSequence(u64);

impl InstallSequence {
    pub(crate) const NONE: Self = Self(0);
    const NEVER: Self = Self(u64::MAX);

    pub(crate) fn next(self) -> Option<Self> {
        self.0
            .checked_add(1)
            .filter(|value| *value < u64::MAX)
            .map(Self)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    fn from_rendered(value: u64) -> Option<Self> {
        (value != Self::NONE.0 && value != Self::NEVER.0).then_some(Self(value))
    }
}

/// Canonical JSON projection of the project fields passed to V2 lowering,
/// independent of the plan identity and callback installation sequence.
/// Non-finite raw floats serialize as `null`, so this is not a bitwise identity
/// for every possible in-memory `ProjectFile`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub(crate) struct LoweredProjectDigest([u8; 32]);

impl LoweredProjectDigest {
    /// Hash the project fields consumed by `lower_project`. Converting to a
    /// JSON value first gives map fields a stable key order.
    pub(crate) fn of(project: &crate::project::ProjectFile) -> Result<Self, serde_json::Error> {
        let value = serde_json::to_value((&project.instruments, &project.song, &project.global))?;
        let bytes = serde_json::to_vec(&value)?;
        let hash = Sha256::digest(bytes);
        let mut digest = [0; 32];
        digest.copy_from_slice(&hash);
        Ok(Self(digest))
    }
}

impl std::fmt::Display for LoweredProjectDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// A V2 plan and its lowering input, visible only after its installation has
/// completed a callback without being muted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub(crate) struct RenderedPlan {
    install: InstallSequence,
    plan: PlanId,
    project: LoweredProjectDigest,
}

impl RenderedPlan {
    pub(crate) const fn plan_id(self) -> PlanId {
        self.plan
    }

    #[cfg(any(feature = "gui-egui", test))]
    pub(crate) const fn project_digest(self) -> LoweredProjectDigest {
        self.project
    }
}

enum SwitchCommand {
    /// A session and its install sequence number, counted from 1 in queue order.
    Install(InstallSequence, Box<SongAudio>),
}

#[derive(Debug, thiserror::Error)]
enum InstallError {
    #[error("the V2 switch command ring is full")]
    Full,
    #[error("V2 installation identities are exhausted")]
    IdentityExhausted,
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
    /// Last installation whose session completed a successful callback. Only the
    /// callback writes it; control requests filter it before use.
    rendered_install: Arc<AtomicU64>,
    /// The sequence number of the active session's install.
    admitted: InstallSequence,
    scratch: Box<[f32]>,
}

/// A projection bracketed by the authoritative module/effect lineage counter.
struct EditSnapshot {
    project: crate::project::ProjectFile,
    lineage: Option<synth_core::ContentRevision>,
}

/// Metadata stays reversible until the owning running-edit receipt returns.
struct RunningPublication {
    install: InstallSequence,
    previous: Option<RenderedPlan>,
    omissions: Vec<String>,
    superseded: bool,
    lineage: Option<synth_core::ContentRevision>,
}

/// The control half, owned by the GUI thread.
#[must_use]
pub struct SongSwitch {
    commands: HeapProd<SwitchCommand>,
    retired: HeapCons<Box<SongAudio>>,
    playback: Option<SongPlayback>,
    identity_source: Option<Arc<synth_engine::EngineState>>,
    /// The lineage of the input used by the current control pair.
    identity_lineage: Option<synth_core::ContentRevision>,
    /// Metadata for the newest successfully queued session. Its install
    /// sequence prevents a previous callback from labelling this plan.
    latest_plan: Option<RenderedPlan>,
    /// `None` when the device reported a rate V2 cannot represent; V2 mode then refuses.
    rate: Option<SampleRate>,
    channels: usize,
    /// Whether the callback's slot holds a session once every queued command is admitted.
    occupied: bool,
    /// Shared with the callback: leave V2 mode, and silence V2 until a given install.
    remove: Arc<AtomicBool>,
    muted_until: Arc<AtomicU64>,
    rendered_install: Arc<AtomicU64>,
    /// The sequence number of the last queued install.
    sent: InstallSequence,
    /// Sessions the queued commands will retire and the control half has not yet collected.
    retiring: usize,
    /// An edit waits for transport acknowledgement or an earlier replacement.
    rebuild_pending: bool,
    /// One declined attempt waits for pause, Stop or a genuinely newer edit.
    running_declined: bool,
    /// Latest unsent Pause intent, retained while the replacement owns its credit.
    pause_pending: bool,
    /// Keep audible metadata until the matching owning receipt settles publication.
    pending_running: Option<RunningPublication>,
    /// Explicit Stop must retain fresh-start intent through switch-ring pressure.
    stop_pending: bool,
    /// The project was refused after an edit; V2 must not play, and the application leaves
    /// V2 mode through its own path, which resets V1 first (see [`Self::needs_disable`]).
    disable_pending: bool,
    notices: Vec<String>,
    /// What the current session's lowering could not represent; replaced on every rebuild
    /// and shown for as long as V2 mode is active.
    omissions: Vec<String>,
    #[cfg(test)]
    before_edit_publication: Option<Box<dyn FnOnce() + Send>>,
}

/// Wrap the V1 processor. The switch starts in V1 mode, and V2 mode refuses until the
/// stream's actual configuration is known through [`SongSwitch::set_stream`].
pub fn wrap<P: AudioProcessor>(v1: P) -> (SongSwitch, EngineSwitch<P>) {
    let (commands, command_reader) = HeapRb::new(SLOTS).split();
    let (retired, retired_reader) = HeapRb::new(SLOTS).split();
    let remove = Arc::new(AtomicBool::new(false));
    let muted_until = Arc::new(AtomicU64::new(0));
    let rendered_install = Arc::new(AtomicU64::new(InstallSequence::NONE.as_u64()));
    (
        SongSwitch {
            commands,
            retired: retired_reader,
            playback: None,
            identity_source: None,
            identity_lineage: None,
            latest_plan: None,
            rate: None,
            channels: 0,
            occupied: false,
            remove: Arc::clone(&remove),
            muted_until: Arc::clone(&muted_until),
            rendered_install: Arc::clone(&rendered_install),
            sent: InstallSequence::NONE,
            retiring: 0,
            rebuild_pending: false,
            running_declined: false,
            pause_pending: false,
            pending_running: None,
            stop_pending: false,
            disable_pending: false,
            notices: Vec::new(),
            omissions: Vec::new(),
            #[cfg(test)]
            before_edit_publication: None,
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
            rendered_install,
            admitted: InstallSequence::NONE,
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
        // Publish the absent session before lifting the control-side removal
        // request; observing `remove == false` must never expose its old ack.
        self.rendered_install
            .store(InstallSequence::NONE.as_u64(), Ordering::Release);
        self.remove.store(false, Ordering::Release);
        false
    }
}

impl<P: AudioProcessor> AudioProcessor for EngineSwitch<P> {
    fn process(&mut self, output: &mut [f32], context: &AudioCallbackContext) {
        self.admit_commands();
        let removing = self.admit_removal();
        let Some(song) = self.active.as_mut() else {
            self.rendered_install
                .store(InstallSequence::NONE.as_u64(), Ordering::Release);
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
        if removing || self.admitted.as_u64() < self.muted_until.load(Ordering::Acquire) {
            output.fill(0.0);
            self.rendered_install
                .store(InstallSequence::NONE.as_u64(), Ordering::Release);
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
        self.admitted = song.install_sequence();
        self.rendered_install.store(
            if rendered {
                self.admitted.as_u64()
            } else {
                InstallSequence::NONE.as_u64()
            },
            Ordering::Release,
        );
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
    /// Attach the same authoritative snapshot owner the GUI/MCP project builder reads.
    #[cfg(any(feature = "gui-egui", test))]
    pub(crate) fn track_identity_source(&mut self, source: Arc<synth_engine::EngineState>) {
        if self
            .identity_source
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &source))
        {
            return;
        }
        self.identity_source = Some(source);
        self.identity_lineage = None;
    }

    fn snapshot(&self, project: impl FnOnce() -> crate::project::ProjectFile) -> EditSnapshot {
        let before = self
            .identity_source
            .as_ref()
            .and_then(|source| source.shared_graph.identity_lineage());
        let project = project();
        let after = self
            .identity_source
            .as_ref()
            .and_then(|source| source.shared_graph.identity_lineage());
        EditSnapshot {
            project,
            lineage: before.filter(|_| before == after),
        }
    }

    fn lineage_allows(&self, input: Option<synth_core::ContentRevision>) -> bool {
        self.identity_source.as_ref().is_none_or(|source| {
            input.is_some()
                && self.identity_lineage == input
                && source.shared_graph.identity_lineage() == input
        })
    }

    /// The installation rendered by the last completed callback while it is still
    /// permitted by control's current mute/removal requests. This is not a device
    /// liveness signal: a stopped backend may never call the switch again.
    #[must_use]
    pub fn rendered_install(&self) -> Option<InstallSequence> {
        if self.remove.load(Ordering::Acquire) {
            return None;
        }
        let rendered = self.rendered_install.load(Ordering::Acquire);
        let muted_until = self.muted_until.load(Ordering::Acquire);
        (rendered >= muted_until)
            .then(|| InstallSequence::from_rendered(rendered))
            .flatten()
    }

    /// Return the newest queued plan only after a completed callback rendered
    /// that installation. A running replacement retains its previous audible
    /// metadata until the fade completes; a stopped replacement stays unnamed.
    #[must_use]
    pub fn rendered_plan_id(&self) -> Option<PlanId> {
        self.rendered_plan().map(RenderedPlan::plan_id)
    }

    /// The last completed audible installation and its serialized project
    /// input. This session-local identity is neither a canonical project
    /// revision nor a device-liveness indication.
    #[must_use]
    pub(crate) fn rendered_plan(&self) -> Option<RenderedPlan> {
        let rendered = self.rendered_install()?;
        self.playback.as_ref()?;
        self.latest_plan
            .filter(|plan| plan.install == rendered)
            .or_else(|| {
                self.pending_running
                    .as_ref()
                    .and_then(|pending| pending.previous)
                    .filter(|plan| plan.install == rendered)
            })
    }

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
        if self.playback.is_none() {
            return &[];
        }
        if let Some(pending) = &self.pending_running
            && self.rendered_install() == Some(pending.install)
        {
            return &pending.omissions;
        }
        &self.omissions
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
    ) -> Result<(SongPlayback, SongAudio, LoweredProjectDigest, Vec<String>), String> {
        let profile = self.profile()?;
        let digest = LoweredProjectDigest::of(project)
            .map_err(|error| format!("V2 lowering input cannot be identified: {error}"))?;
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
        Ok((playback, audio, digest, unrepresented))
    }

    /// Queue a session for the callback or return its capacity/identity refusal.
    fn install(
        &mut self,
        mut audio: SongAudio,
        plan: PlanId,
        project: LoweredProjectDigest,
    ) -> Result<(), InstallError> {
        let sequence = self.sent.next().ok_or(InstallError::IdentityExhausted)?;
        audio.set_install_sequence(sequence);
        if self
            .commands
            .try_push(SwitchCommand::Install(sequence, Box::new(audio)))
            .is_err()
        {
            return Err(InstallError::Full);
        }
        self.sent = sequence;
        self.latest_plan = Some(RenderedPlan {
            install: sequence,
            plan,
            project,
        });
        if self.occupied {
            self.retiring += 1;
        }
        self.occupied = true;
        Ok(())
    }

    /// Enter V2 mode with the project as it is now, stopped at the song start. The caller
    /// stops V1 first. On refusal V1 stays active and the reason is a notice.
    pub fn enable(&mut self, project: &crate::project::ProjectFile) -> bool {
        self.enable_with_lineage(project, None)
    }

    #[cfg(any(feature = "gui-egui", test))]
    pub(crate) fn enable_from(
        &mut self,
        project: impl FnOnce() -> crate::project::ProjectFile,
    ) -> bool {
        let snapshot = self.snapshot(project);
        self.enable_with_lineage(&snapshot.project, snapshot.lineage)
    }

    fn enable_with_lineage(
        &mut self,
        project: &crate::project::ProjectFile,
        lineage: Option<synth_core::ContentRevision>,
    ) -> bool {
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
            Ok((playback, audio, digest, unrepresented)) => {
                // Whatever silenced an earlier V2 mode ends with this session.
                self.muted_until.store(
                    self.sent.next().unwrap_or(InstallSequence::NEVER).as_u64(),
                    Ordering::Release,
                );
                if let Err(error) = self.install(audio, playback.plan_id(), digest) {
                    self.notices
                        .push(format!("V2 playback cannot switch: {error}"));
                    return false;
                }
                self.playback = Some(playback);
                self.identity_lineage = lineage;
                self.rebuild_pending = false;
                self.stop_pending = false;
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
        self.latest_plan = None;
        self.pending_running = None;
        self.pause_pending = false;
        self.running_declined = false;
        self.rebuild_pending = false;
        self.stop_pending = false;
        self.disable_pending = false;
    }

    /// Replace the session with a fresh one from the project, stopped at the song start.
    /// Returns whether the new session was queued. Ring pressure keeps the edit
    /// pending; a preparation or identity refusal mutes the old session.
    fn rebuild(&mut self, snapshot: &EditSnapshot) -> bool {
        let project = &snapshot.project;
        // After a refusal nothing may lift the silence; only leaving V2 mode ends it.
        if self.disable_pending {
            return false;
        }
        match self.prepare(project) {
            Ok((playback, audio, digest, unrepresented)) => {
                match self.install(audio, playback.plan_id(), digest) {
                    Ok(()) => {
                        self.playback = Some(playback);
                        self.pending_running = None;
                        self.pause_pending = false;
                        self.running_declined = false;
                        self.identity_lineage = snapshot.lineage;
                        self.rebuild_pending = false;
                        self.stop_pending = false;
                        self.omissions = unrepresented;
                        true
                    }
                    Err(InstallError::Full) => {
                        self.rebuild_pending = true;
                        false
                    }
                    Err(InstallError::IdentityExhausted) => {
                        self.disable_pending = true;
                        self.muted_until
                            .store(InstallSequence::NEVER.as_u64(), Ordering::Release);
                        self.notices.push(
                            "V2 playback stops: installation identities are exhausted".to_owned(),
                        );
                        false
                    }
                }
            }
            Err(reason) => {
                self.notices.push(format!(
                    "V2 playback stops: edited project preparation failed: {reason}"
                ));
                // The refused project must never play: V2 is silenced from the next callback,
                // whatever any ring holds. Leaving V2 mode is the application's, so that V1 is
                // reset before it is heard again.
                self.disable_pending = true;
                self.muted_until
                    .store(InstallSequence::NEVER.as_u64(), Ordering::Release);
                false
            }
        }
    }

    /// Wait for acknowledged pause, then try compatible stopped readmission.
    /// Unsupported graphs retain the explicit fresh-session fallback.
    fn apply_edit(&mut self, snapshot: &EditSnapshot) -> bool {
        let project = &snapshot.project;
        let Some(playback) = self.playback.as_mut() else {
            return false;
        };
        if !playback.edit_ready() {
            self.rebuild_pending = true;
            return false;
        }
        if playback
            .applied_position()
            .is_some_and(|position| position.as_u64() > 0)
            && self.lineage_allows(snapshot.lineage)
        {
            let attempt = (|| {
                let profile = self.profile().map_err(SongEditError::Preparation)?;
                let sequence = self.sent.next().ok_or_else(|| {
                    SongEditError::Preparation(InstallError::IdentityExhausted.to_string())
                })?;
                let digest = LoweredProjectDigest::of(project)
                    .map_err(|error| SongEditError::Preparation(error.to_string()))?;
                let lowered = lower_project(
                    &project.instruments,
                    &project.song,
                    &project.global,
                    profile,
                    OutputPolicy::Parity,
                );
                let omissions = lowered
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity() == super::Severity::Unrepresented)
                    .map(|d| format!("V2 playback omits: {:?}: {:?}", d.subject(), d.reason()))
                    .collect();
                let playback = self
                    .playback
                    .as_mut()
                    .ok_or_else(|| SongEditError::Preparation("V2 playback ended".to_owned()))?;
                let source = self.identity_source.clone();
                let expected = snapshot.lineage;
                #[cfg(test)]
                let before_publication = self.before_edit_publication.take();
                let plan =
                    playback.replace_compatible_guarded(lowered, profile, sequence, move || {
                        #[cfg(test)]
                        if let Some(hook) = before_publication {
                            hook();
                        }
                        source.as_ref().is_none_or(|source| {
                            expected.is_some() && source.shared_graph.identity_lineage() == expected
                        })
                    })?;
                self.sent = sequence;
                self.latest_plan = Some(RenderedPlan {
                    install: sequence,
                    plan,
                    project: digest,
                });
                self.omissions = omissions;
                self.rebuild_pending = false;
                Ok::<(), SongEditError>(())
            })();
            match attempt {
                Ok(()) => {
                    self.notices.push("V2 playback: compatible edit queued; the paused position will be kept after installation".to_owned());
                    return true;
                }
                Err(SongEditError::Busy) => {
                    self.rebuild_pending = true;
                    return false;
                }
                Err(SongEditError::Unsupported(reason)) => self.notices.push(format!("V2 playback: compatible paused edit unavailable ({reason}); preparing a restart from the song start")),
                Err(error) => {
                    self.notices.push(format!("V2 playback stops: paused edit preparation failed: {error}"));
                    self.disable_pending = true;
                    self.muted_until.store(InstallSequence::NEVER.as_u64(), Ordering::Release);
                    return false;
                }
            }
        }
        if self.rebuild(snapshot) {
            self.notices.push(
                "V2 playback: the edit is applied; playback restarts from the song start"
                    .to_owned(),
            );
            true
        } else {
            false
        }
    }

    fn reconcile_edit(&mut self) {
        let Some(playback) = self.playback.as_mut() else {
            return;
        };
        playback.collect();
        let Some((install, installed)) = playback.take_edit_completion() else {
            return;
        };
        let Some(pending) = self.pending_running.take() else {
            return;
        };
        if pending.install != install {
            self.pending_running = Some(pending);
            return;
        }
        if installed {
            self.identity_lineage = pending.lineage;
            self.omissions = pending.omissions;
        } else {
            self.latest_plan = pending.previous;
            self.rebuild_pending = true;
            self.running_declined = !pending.superseded;
            self.notices.push(if pending.superseded {
                "V2 playback: running edit declined; the previous plan continues while the newer edit is prepared"
            } else {
                "V2 playback: running edit declined; the previous plan continues, and the edit waits for pause or Stop"
            }.to_owned());
        }
    }

    fn apply_running(&mut self, snapshot: &EditSnapshot) -> bool {
        let result = (|| {
            let profile = self.profile().map_err(SongEditError::Preparation)?;
            let sequence = self.sent.next().ok_or_else(|| {
                SongEditError::Preparation(InstallError::IdentityExhausted.to_string())
            })?;
            let project = &snapshot.project;
            let digest = LoweredProjectDigest::of(project)
                .map_err(|error| SongEditError::Preparation(error.to_string()))?;
            let lowered = lower_project(
                &project.instruments,
                &project.song,
                &project.global,
                profile,
                OutputPolicy::Parity,
            );
            let omissions = lowered
                .diagnostics
                .iter()
                .filter(|d| d.severity() == super::Severity::Unrepresented)
                .map(|d| format!("V2 playback omits: {:?}: {:?}", d.subject(), d.reason()))
                .collect();
            let preserve = self.lineage_allows(snapshot.lineage);
            let source = self.identity_source.clone();
            let expected = snapshot.lineage;
            #[cfg(test)]
            let before_publication = self.before_edit_publication.take();
            let playback = self.playback.as_mut().ok_or(SongEditError::Busy)?;
            let (plan, kind) = playback.replace_running_guarded(
                lowered,
                profile,
                sequence,
                preserve,
                move || {
                    #[cfg(test)]
                    if let Some(hook) = before_publication {
                        hook();
                    }
                    source.as_ref().is_none_or(|source| {
                        expected.is_some() && source.shared_graph.identity_lineage() == expected
                    })
                },
            )?;
            self.pending_running = Some(RunningPublication {
                install: sequence,
                previous: self.latest_plan,
                omissions,
                superseded: false,
                lineage: snapshot.lineage,
            });
            self.sent = sequence;
            self.latest_plan = Some(RenderedPlan {
                install: sequence,
                plan,
                project: digest,
            });
            self.rebuild_pending = false;
            Ok::<_, SongEditError>(kind)
        })();
        match result {
            Ok(kind) => {
                self.notices.push(if kind == synth_engine_v2::host::session::transfer::replacement::PlanReplacementKind::RunningCompatible {
                    "V2 playback: compatible running edit queued to preserve sounding voices and their identities"
                } else {
                    "V2 playback: running edit queued with a one-quantum reset crossfade; crossing notes will be cut on installation"
                }.to_owned());
                true
            }
            Err(SongEditError::Busy) => false,
            Err(error) => {
                self.running_declined = true;
                self.notices.push(format!("V2 playback: running edit deferred ({error}); the previous plan continues until pause or Stop"));
                false
            }
        }
    }

    /// Apply a deferred edit; false while it cannot be installed yet.
    fn apply_pending(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) -> bool {
        if self.stop_pending {
            let snapshot = self.snapshot(project);
            return self.rebuild(&snapshot);
        }
        self.reconcile_edit();
        let running = self
            .playback
            .as_mut()
            .is_some_and(SongPlayback::running_edit_ready);
        let stopped = !running && self.playback.as_mut().is_some_and(SongPlayback::edit_ready);
        self.reconcile_edit();
        if running {
            if self.running_declined || self.pause_pending {
                return false;
            }
            let snapshot = self.snapshot(project);
            return self.apply_running(&snapshot);
        }
        if !stopped {
            return false;
        }
        let snapshot = self.snapshot(project);
        self.apply_edit(&snapshot)
    }

    /// Play from the start or from where V2 paused. A deferred edit is applied first.
    pub fn play(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) {
        self.pause_pending = false;
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
        if self.playback.as_ref().is_some_and(SongPlayback::is_editing) {
            self.notices
                .push("V2 playback is busy installing the paused edit; try again".to_owned());
            return;
        }
        if let Some(playback) = self.playback.as_mut()
            && let Err(error) = playback.play()
        {
            self.notices
                .push(format!("V2 playback could not start: {error}"));
        }
    }

    /// Pause where the song is; a pending edit retains the request until collection.
    pub fn pause(&mut self) {
        self.pause_pending = true;
        self.service_pause();
    }

    fn service_pause(&mut self) {
        self.reconcile_edit();
        if !self.pause_pending {
            return;
        }
        let Some(playback) = self.playback.as_mut() else {
            self.pause_pending = false;
            return;
        };
        if playback.is_editing() {
            return;
        }
        match playback.pause() {
            Ok(_) => self.pause_pending = false,
            Err(error) => {
                self.pause_pending = false;
                self.notices
                    .push(format!("V2 playback could not pause: {error}"));
            }
        }
    }

    /// Stop and return to the song start, with any deferred edit applied.
    pub fn stop(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) {
        if self.playback.is_none() || self.disable_pending {
            return;
        }
        self.stop_pending = true;
        // Silent from the next callback until the next install, the fresh session, is active;
        // a session queued earlier stays silent.
        self.muted_until.store(
            self.sent.next().unwrap_or(InstallSequence::NEVER).as_u64(),
            Ordering::Release,
        );
        // A refusal has its own notice and leaves V2 mode; retrying stop would not help.
        let snapshot = self.snapshot(project);
        if !self.rebuild(&snapshot) && !self.disable_pending {
            self.notices.push(
                "V2 playback is busy switching; stop again to return to the start".to_owned(),
            );
        }
    }

    /// Note an edit. Acknowledged playing can reset through a crossfade; paused
    /// edits use compatible readmission. `project` is called only when ready.
    pub fn project_changed(&mut self, project: impl FnOnce() -> crate::project::ProjectFile) {
        if self.playback.is_none() {
            return;
        }
        self.running_declined = false;
        if let Some(pending) = self.pending_running.as_mut() {
            pending.superseded = true;
        }
        self.rebuild_pending = true;
        if !self.apply_pending(project) && !self.running_declined {
            self.notices.push(
                "V2 playback: the edit remains pending; the previous plan remains active"
                    .to_owned(),
            );
        }
    }

    /// Call every GUI frame. Collects receipts and retired sessions, reports refusals,
    /// readmits deferred paused edits and prepares a fresh session after a fault.
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
        let edit_failed = playback.edit_failed();
        let faulted = playback.is_faulted();
        if faulted {
            let snapshot = self.snapshot(project);
            if self.rebuild(&snapshot) {
                self.notices.push(
                    "V2 playback faulted and was prepared again at the song start".to_owned(),
                );
            }
        } else if edit_failed {
            let snapshot = self.snapshot(project);
            if self.rebuild(&snapshot) {
                self.notices.push("V2 playback: a command or paused edit transfer failed; playback restarts from the song start".to_owned());
            }
        } else {
            self.reconcile_edit();
            self.service_pause();
            if self.rebuild_pending {
                // A failed install stays pending and is retried on the next poll.
                let _applied = self.apply_pending(project);
            }
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

#[cfg(test)]
#[path = "tests/app_edit.rs"]
mod edit_tests;
