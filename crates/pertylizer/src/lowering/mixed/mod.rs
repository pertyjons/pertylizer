//! Sound-only software input over the experimental mixed owner (ADR-0088).
//! The profile is provisional: no production capacity or physical timing is qualified.
use super::{ResolvedIdentities, graph::LiveNoteRange, render::lower_project_mixed};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use synth_core::{InstrumentId, ModuleType};
use synth_engine_v2::{
    host::{
        ConnectionGeneration, mixed_targets::MixedTargetAdmission,
        software_input::SoftwareInputPair,
    },
    profile::HostProfile,
    quantities::{EventCount, HeldNoteCount},
    recording::notes::Midi1Input,
    render::AudioBlockMut,
    schedule::{AdmittedCompiledStream, PlanEvent},
    stream::{
        MixedBridgeArmed, MixedBridgeControl, MixedBridgeResetAudio, MixedBridgeResetOwner,
        MixedJoinedStream, MixedPumpStep, MixedSourceHandoff, MixedSourcePacketId,
        MixedTransferCommand, MixedTransferConsumer, MixedTransferControlPort, MixedTransferFence,
        MixedTransferPlaces, MixedTransferProducer, MixedTransferQueue, MixedTransferResult,
    },
    time::{FrameCount, PlanPosition, SampleTime, StreamAnchor},
};

mod queue;
mod reset;
mod teardown;
mod transport;
use queue::{Reader, Ring, Writer};
pub use teardown::MixedRetirement;
use transport::{Operation, Request, RequestId};

type ControlPort =
    MixedTransferControlPort<Writer<MixedTransferCommand>, Reader<MixedTransferResult>>;
type Audio = MixedBridgeResetAudio<Reader<MixedTransferCommand>, Writer<MixedTransferResult>>;
type Owner = MixedBridgeResetOwner<Reader<MixedTransferCommand>, Writer<MixedTransferResult>>;
type RetiredOwner = synth_engine_v2::stream::MixedBridgeResetRetired<
    Reader<MixedTransferCommand>,
    Writer<MixedTransferResult>,
>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum MixedPlaybackError {
    #[error("mixed playback preparation refused: {0}")]
    Preparation(String),
    #[error("software input refused: {0}")]
    Input(String),
    #[error("mixed software owner identities are exhausted")]
    IdentityExhausted,
}

/// Bound for the lifetime of an armed owner; changing GUI selection cannot retarget it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) struct LiveBinding {
    instrument: InstrumentId,
    notes: HeldNoteCount,
    holds: EventCount,
}
impl LiveBinding {
    pub(crate) const fn software(instrument: InstrumentId) -> Self {
        Self {
            instrument,
            notes: HeldNoteCount::measured(4),
            holds: EventCount::measured(4),
        }
    }
    pub(crate) const fn instrument(self) -> InstrumentId {
        self.instrument
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LoopIntent {
    #[default]
    Keep,
    EnterWholeSong,
    Leave,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[must_use]
struct IntentRevision(u64);
impl IntentRevision {
    fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// Latest intent per independent transport dimension; Play cannot erase a rewind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TransportIntent {
    revision: IntentRevision,
    destination: Option<PlanPosition>,
    loop_mode: LoopIntent,
}

impl TransportIntent {
    fn is_empty(self) -> bool {
        self.destination.is_none() && self.loop_mode == LoopIntent::Keep
    }
    fn clear(&mut self) {
        self.destination = None;
        self.loop_mode = LoopIntent::Keep;
    }
}

#[derive(Clone, Copy)]
struct HeldKey {
    channel: synth_core::MidiChannel,
    key: synth_engine_v2::quantities::KeyIdentity,
    onset: MixedSourcePacketId,
}

struct Source {
    connection: ConnectionGeneration,
    serial: u64,
    frontier: SampleTime,
    pedals: [bool; 16],
    held: [Option<HeldKey>; 16],
}
impl Source {
    fn packet(&mut self) -> Result<MixedSourcePacketId, MixedPlaybackError> {
        let next = self
            .serial
            .checked_add(1)
            .ok_or(MixedPlaybackError::IdentityExhausted)?;
        let packet = MixedSourcePacketId::new(self.connection, next)
            .map_err(|_| MixedPlaybackError::IdentityExhausted)?;
        self.serial = next;
        Ok(packet)
    }
}

#[must_use]
pub(crate) struct MixedPlayback {
    bridge: MixedBridgeControl,
    port: ControlPort,
    // Keep the logical connections alive until explicit owner teardown.
    _connections: SoftwareInputPair,
    sources: [Source; 2],
    binding: LiveBinding,
    owner_install: super::app::InstallSequence,
    profile: HostProfile,
    omissions: Vec<String>,
    end: PlanPosition,
    clock: Arc<AtomicU64>,
    faulted: Arc<AtomicBool>,
    edit_completion: Option<(super::app::InstallSequence, bool)>,
    edit_metadata: Option<(Vec<String>, PlanPosition)>,
    reset_request: Option<Box<Request>>,
    reset_port: Option<ControlPort>,
    anchor: StreamAnchor,
    looping: Option<synth_engine_v2::transport::LoopInterval>,
    requests: HeapProd<Box<Request>>,
    completed: HeapCons<Box<Request>>,
    pending: Option<RequestId>,
    previous: Option<RequestId>,
    playing: bool,
    requested_playing: bool,
    intent: TransportIntent,
    resume: PlanPosition,
    notices: Vec<String>,
}

#[must_use]
pub(crate) struct MixedSongAudio {
    audio: Audio,
    clock: Arc<AtomicU64>,
    faulted: Arc<AtomicBool>,
    install: super::app::InstallSequence,
    owner_install: super::app::InstallSequence,
    requests: HeapCons<Box<Request>>,
    completed: HeapProd<Box<Request>>,
    pending: Option<Box<Request>>,
}
impl MixedSongAudio {
    pub(crate) fn render(&mut self, mut output: AudioBlockMut<'_>) -> bool {
        if self.faulted.load(Ordering::Acquire) {
            output.silence();
            return false;
        }
        if self.pending.is_none() {
            self.pending = self.requests.try_pop();
            if let Some(request) = &mut self.pending {
                request.before_render(&mut self.audio);
            }
        }
        let rendered = self.audio.render(output).is_ok();
        if !rendered {
            self.faulted.store(true, Ordering::Release);
        }
        self.clock
            .store(self.audio.clock().as_u64(), Ordering::Release);
        if let Some(request) = &mut self.pending {
            request.after_render(&mut self.audio, rendered);
        }
        if let Some(request) = &self.pending
            && let Operation::Continuation {
                install,
                packet: Some(packet),
                ..
            } = &request.operation
            && matches!(
                packet.outcome(),
                Some(
                    synth_engine_v2::stream::MixedContinuationOutcome::InstalledRendered { .. }
                        | synth_engine_v2::stream::MixedContinuationOutcome::InstalledFaulted { .. }
                )
            )
        {
            self.install = *install;
        }
        if let Some(request) = &self.pending
            && let Operation::Reset {
                install, retired, ..
            } = &request.operation
            && retired.is_some()
        {
            self.install = *install;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|request| request.complete)
            && let Some(request) = self.pending.take()
            && let Err(request) = self.completed.try_push(request)
        {
            self.pending = Some(request);
        }
        rendered
    }
    #[cfg(test)]
    pub(crate) fn stopped_position_for_test(&self) -> Option<PlanPosition> {
        self.audio.stopped_position()
    }
    pub(crate) fn refuse_output_shape(&self) {
        self.faulted.store(true, Ordering::Release);
    }
    pub(crate) const fn owner_install(&self) -> super::app::InstallSequence {
        self.owner_install
    }
    pub(crate) fn plan_id(&self) -> synth_engine_v2::plan::PlanId {
        self.audio.plan_id()
    }
    pub(crate) const fn install_sequence(&self) -> super::app::InstallSequence {
        self.install
    }
    pub(crate) fn set_install_sequence(&mut self, sequence: super::app::InstallSequence) {
        self.install = sequence;
        self.owner_install = sequence;
    }
}

fn preparation(error: impl std::fmt::Display) -> MixedPlaybackError {
    MixedPlaybackError::Preparation(error.to_string())
}

fn lower_targets(
    project: &crate::project::ProjectFile,
    profile: HostProfile,
    binding: LiveBinding,
) -> Result<(MixedTargetAdmission, Vec<String>, PlanPosition), MixedPlaybackError> {
    let live = LiveNoteRange::new(binding.notes, binding.holds)
        .ok_or_else(|| preparation("invalid live range"))?;
    let lowered = lower_project_mixed(project, profile, live);
    let omissions: Vec<String> = lowered
        .diagnostics
        .iter()
        .map(|diagnostic| format!("{:?}: {:?}", diagnostic.subject(), diagnostic.reason()))
        .collect();
    let plan = lowered.plan.ok_or_else(|| {
        preparation(format!(
            "project lowering refused: {}",
            omissions.join("; ")
        ))
    })?;
    let selected = project
        .instruments
        .iter()
        .find(|saved| saved.id == binding.instrument)
        .ok_or_else(|| preparation("the selected live instrument does not exist"))?;
    let identities =
        ResolvedIdentities::resolve(selected.id, &selected.patch.modules).map_err(preparation)?;
    let mut gates = identities
        .pairs()
        .filter(|(id, _)| id.module_type == ModuleType::Envelope)
        .map(|(_, node)| node);
    let node = gates
        .next()
        .ok_or_else(|| preparation("the live instrument has no envelope"))?;
    if gates.next().is_some() {
        return Err(preparation("the live instrument has ambiguous envelopes"));
    }
    let note = plan
        .resolve_note(node)
        .ok_or_else(|| preparation("the live envelope has no note target"))?;
    let events: Vec<_> = lowered
        .events
        .iter()
        .map(|event| PlanEvent::new(PlanPosition::new(event.time().as_u64()), event.payload()))
        .collect();
    let stream = AdmittedCompiledStream::admit(&plan, &events).map_err(preparation)?;
    let targets = MixedTargetAdmission::admit(plan, stream, note)
        .map_err(|failure| preparation(failure.reason()))?;
    Ok((
        targets,
        omissions,
        PlanPosition::new(lowered.lowered_frames.as_u64()),
    ))
}

pub(crate) fn prepare(
    project: &crate::project::ProjectFile,
    profile: HostProfile,
    binding: LiveBinding,
) -> Result<(MixedPlayback, MixedSongAudio), MixedPlaybackError> {
    let (binding_admission, omissions, end) = lower_targets(project, profile, binding)?;
    let anchor = StreamAnchor::new(SampleTime::new(0), PlanPosition::new(0));
    let joined = MixedJoinedStream::open(binding_admission, anchor)
        .map_err(|failure| preparation(failure.0))?;
    let prepared = joined
        .prepare_initial()
        .map_err(|failure| preparation(failure.0))?;
    // Initial installation is cold: adopt at sample zero without replaying a prefix.
    let start = SampleTime::new(0);
    let history = prepared
        .prepare_history(start, PlanPosition::new(0))
        .map_err(preparation)?;
    let suffix = prepared.prepare_suffix(history).map_err(preparation)?;
    let candidate = prepared.stamp_suffix(suffix).map_err(preparation)?;
    let connections = SoftwareInputPair::open().map_err(preparation)?;
    let pair = connections.sources();
    let depth = profile
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity()
        .as_usize()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| preparation("command capacity overflow"))?;
    let MixedBridgeArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = prepared
        .arm_software_bridge(
            candidate,
            &profile,
            &connections,
            MixedTransferFence {
                stage_cells: MixedTransferPlaces::new(32),
                held_onsets: MixedTransferPlaces::new(16),
                peer_wait: FrameCount::new(64),
            },
            Ring(HeapRb::new(depth)),
            Ring(HeapRb::new(depth)),
        )
        .map_err(|failure| preparation(failure.reason()))?;
    let clock = Arc::new(AtomicU64::new(0));
    let faulted = Arc::new(AtomicBool::new(false));
    let (requests, request_reader) = HeapRb::new(1).split();
    let (result_writer, completed) = HeapRb::new(1).split();
    Ok((
        MixedPlayback {
            bridge: control,
            port: control_port,
            _connections: connections,
            sources: [pair.first(), pair.second()].map(|connection| Source {
                connection,
                serial: 0,
                frontier: SampleTime::new(0),
                held: [None; 16],
                pedals: [false; 16],
            }),
            binding,
            owner_install: super::app::InstallSequence::NONE,
            profile,
            omissions,
            end,
            clock: Arc::clone(&clock),
            faulted: Arc::clone(&faulted),
            edit_completion: None,
            edit_metadata: None,
            reset_request: None,
            reset_port: None,
            anchor: StreamAnchor::new(start, PlanPosition::new(0)),
            looping: None,
            requests,
            completed,
            pending: None,
            previous: None,
            playing: true,
            requested_playing: true,
            intent: TransportIntent::default(),
            resume: PlanPosition::new(0),
            notices: Vec::new(),
        },
        MixedSongAudio {
            audio: MixedBridgeResetAudio::new(
                MixedBridgeResetOwner::new(audio, audio_port)
                    .map_err(|_| preparation("foreign audio port"))?,
            ),
            install: super::app::InstallSequence::NONE,
            owner_install: super::app::InstallSequence::NONE,
            requests: request_reader,
            completed: result_writer,
            pending: None,
            clock,
            faulted,
        },
    ))
}

impl MixedPlayback {
    pub(crate) const fn owner_install(&self) -> super::app::InstallSequence {
        self.owner_install
    }
    pub(crate) fn set_owner_install(&mut self, install: super::app::InstallSequence) {
        self.owner_install = install;
    }
    pub(crate) const fn binding(&self) -> LiveBinding {
        self.binding
    }
    pub(crate) fn omissions(&self) -> &[String] {
        &self.omissions
    }
    pub(crate) fn candidate_omissions(&self) -> &[String] {
        self.edit_metadata
            .as_ref()
            .map_or(&self.omissions, |(omissions, _)| omissions)
    }
    /// Arrival is an unmeasured dispatch estimate. The bridge reports its actual clamp.
    fn nominal(&self) -> Result<SampleTime, MixedPlaybackError> {
        self.bridge
            .audio_boundary()
            .checked_add(FrameCount::new(128))
            .map_err(preparation)
    }
    pub(crate) fn stage(
        &mut self,
        source: usize,
        input: Midi1Input,
    ) -> Result<(), MixedPlaybackError> {
        let nominal = self.nominal()?;
        let own = self
            .sources
            .get_mut(source)
            .ok_or_else(|| MixedPlaybackError::Input("unknown software source".to_owned()))?;
        use synth_engine_v2::recording::notes::Midi1Event;
        let key = match input.event() {
            Midi1Event::NoteOn { key, .. } | Midi1Event::KeyRelease { key, .. } => {
                Some((input.channel(), key))
            }
            Midi1Event::Sustain { .. } | Midi1Event::PitchBend { .. } => None,
        };
        let mut release = None;
        let mut onset = None;
        if let Some(key) = key {
            let existing = own
                .held
                .iter()
                .position(|entry| entry.is_some_and(|held| (held.channel, held.key) == key));
            match input.event() {
                Midi1Event::NoteOn { .. } => {
                    if existing.is_some() {
                        return Ok(());
                    }
                    onset = Some(own.held.iter().position(Option::is_none).ok_or_else(|| {
                        MixedPlaybackError::Input("software held-key capacity reached".to_owned())
                    })?);
                }
                Midi1Event::KeyRelease { .. } => {
                    let Some(index) = existing else {
                        return Ok(());
                    };
                    release = Some(index);
                }
                _ => {}
            }
        }
        let at = nominal.max(own.frontier);
        let packet = own.packet()?;
        let outcome = self
            .bridge
            .stage(MixedSourceHandoff { packet, at, input })
            .map_err(|error| MixedPlaybackError::Input(error.to_string()))?;
        own.frontier = at;
        if let Midi1Event::Sustain { down } = input.event() {
            own.pedals[usize::from(input.channel().as_u8() - 1)] = down;
        }
        // A split onset still owns its pairing tombstone until this key-up arrives.
        if let Some(index) = onset {
            own.held[index] = key.map(|(channel, key)| HeldKey {
                channel,
                key,
                onset: packet,
            });
        }
        if let Some(index) = release {
            own.held[index] = None;
        }
        match outcome {
            synth_engine_v2::stream::MixedStaged::Staged => Ok(()),
            // Reset already ended the sound. This paired physical release settles
            // its reset tombstone successfully and requires no further audio edge.
            synth_engine_v2::stream::MixedStaged::Split(
                synth_engine_v2::stream::MixedSplit::AfterRefusedOnset {
                    reason: synth_engine_v2::stream::MixedOnsetRefusal::Reset,
                    ..
                }
                | synth_engine_v2::stream::MixedSplit::AfterRefusedPedal {
                    reason: synth_engine_v2::stream::MixedOnsetRefusal::Reset,
                    ..
                },
            ) => Ok(()),
            other => Err(MixedPlaybackError::Input(format!("{other:?}"))),
        }
    }
    pub(crate) fn poll(&mut self) -> Result<(), MixedPlaybackError> {
        self.collect_operation()?;
        if self.faulted.load(Ordering::Acquire) {
            return Err(preparation("mixed callback faulted; prepare a new owner"));
        }
        self.publish_ready_reset()?;
        while let Some(report) = self.bridge.collect(&mut self.port).map_err(preparation)? {
            use synth_engine_v2::stream::MixedBridgeOutcome as Outcome;
            let success = matches!(
                report.outcome(),
                Outcome::NoteOn(Ok(_))
                    | Outcome::KeyUp { result: Ok(_), .. }
                    | Outcome::Bend { result: Ok(_), .. }
                    | Outcome::PedalDown { result: Ok(_), .. }
                    | Outcome::Lift { result: Ok(_), .. }
                    | Outcome::Panic(Ok(_))
            );
            if !success {
                self.notice(format!("Mixed input result: {:?}", report.outcome()));
            }
        }
        let nominal = self.nominal()?;
        for source in &mut self.sources {
            let at = nominal.max(source.frontier);
            let packet = source.packet()?;
            self.bridge
                .advance_frontier(packet, at)
                .map_err(preparation)?;
            source.frontier = at;
        }
        loop {
            match self.bridge.pump(&mut self.port).map_err(preparation)? {
                MixedPumpStep::Forwarded { .. } | MixedPumpStep::Panic { .. } => {}
                MixedPumpStep::Split { packet, split } => {
                    self.notice(format!("Mixed packet {packet:?}: {split:?}"))
                }
                MixedPumpStep::Idle(_) => break,
            }
        }
        if let Err(error) = self.service_intent() {
            self.notice(error.to_string());
        }
        Ok(())
    }
}

impl MixedPlayback {
    fn notice(&mut self, message: String) {
        if self.notices.len() < 16 {
            self.notices.push(message);
        } else if let Some(last) = self.notices.last_mut() {
            *last = "Additional mixed input refusals omitted".to_owned();
        }
    }
    pub(crate) fn release_source(&mut self, source: usize) -> Result<(), MixedPlaybackError> {
        let own = self
            .sources
            .get(source)
            .ok_or_else(|| preparation("unknown software source"))?;
        let held = own.held;
        let pedals = own.pedals;
        let mut first_error = None;
        for HeldKey { channel, key, .. } in held.into_iter().flatten() {
            let input = Midi1Input::from_bytes([0x80 | (channel.as_u8() - 1), key.as_u8(), 0])
                .map_err(preparation)?;
            if let Err(error) = self.stage(source, input) {
                first_error.get_or_insert(error);
            }
        }
        for (channel, down) in pedals.into_iter().enumerate() {
            if down {
                let status = u8::try_from(channel).map_err(preparation)?;
                let input = Midi1Input::from_bytes([0xb0 | status, 64, 0]).map_err(preparation)?;
                if let Err(error) = self.stage(source, input) {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    #[cfg(test)]
    pub(crate) fn reset_pending_for_test(&self) -> bool {
        self.reset_port.is_some()
    }
    #[cfg(test)]
    pub(crate) fn stopped_position_for_test(&self) -> Option<PlanPosition> {
        (!self.playing && self.pending.is_none()).then_some(self.resume)
    }
    #[cfg(test)]
    pub(crate) const fn anchor_time_for_test(&self) -> SampleTime {
        self.anchor.time()
    }
    #[cfg(test)]
    pub(crate) const fn anchor_position_for_test(&self) -> PlanPosition {
        self.anchor.position()
    }
    pub(crate) const fn is_playing(&self) -> bool {
        self.requested_playing
    }
    pub(crate) fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }
    fn boundary(&self) -> Result<SampleTime, MixedPlaybackError> {
        let margin = self
            .profile
            .capabilities()
            .maximum_block_size()
            .as_u64()
            .checked_mul(2)
            .and_then(|value| value.checked_add(64))
            .ok_or_else(|| preparation("transport margin overflow"))?;
        let after = self
            .bridge
            .audio_boundary()
            .checked_add(FrameCount::new(margin))
            .map_err(preparation)?;
        let rounded = after
            .as_u64()
            .checked_add(63)
            .map(|value| value / 64 * 64)
            .ok_or_else(|| preparation("transport clock exhausted"))?;
        Ok(SampleTime::new(rounded))
    }
    fn submit(&mut self, operation: Operation) -> Result<(), MixedPlaybackError> {
        if self.pending.is_some() {
            return Err(preparation("an owning operation is still pending"));
        }
        let id = self.next_request_id()?;
        let packet = Box::new(Request {
            id,
            operation,
            complete: false,
            rendered: None,
        });
        if let Err(packet) = self.requests.try_push(packet) {
            self.settle_close_request(*packet)?;
            return Err(preparation("operation ring is full"));
        }
        self.previous = Some(id);
        self.pending = Some(id);
        Ok(())
    }
    fn next_request_id(&self) -> Result<RequestId, MixedPlaybackError> {
        RequestId::next(self.previous).ok_or_else(|| {
            self.faulted.store(true, Ordering::Release);
            MixedPlaybackError::IdentityExhausted
        })
    }
    fn update_location(
        &mut self,
        destination: PlanPosition,
        loop_mode: LoopIntent,
    ) -> Result<(), MixedPlaybackError> {
        let revision = self.intent.revision.next().ok_or_else(|| {
            self.faulted.store(true, Ordering::Release);
            MixedPlaybackError::IdentityExhausted
        })?;
        self.intent = TransportIntent {
            revision,
            destination: Some(destination),
            loop_mode,
        };
        Ok(())
    }
    pub(crate) fn pause(&mut self, rewind: bool) -> Result<(), MixedPlaybackError> {
        if rewind {
            self.update_location(PlanPosition::ZERO, LoopIntent::Leave)?;
        }
        self.requested_playing = false;
        self.service_intent()
    }
    pub(crate) fn play(&mut self) -> Result<(), MixedPlaybackError> {
        self.requested_playing = true;
        self.service_intent()
    }
    pub(crate) const fn looping(&self) -> bool {
        self.looping.is_some()
    }
    pub(crate) fn loop_song(&mut self, enabled: bool) -> Result<(), MixedPlaybackError> {
        self.update_location(
            PlanPosition::ZERO,
            if enabled {
                LoopIntent::EnterWholeSong
            } else {
                LoopIntent::Leave
            },
        )?;
        if enabled {
            self.requested_playing = true;
        }
        self.service_intent()
    }
    pub(crate) fn seek(&mut self, position: PlanPosition) -> Result<(), MixedPlaybackError> {
        self.update_location(position, LoopIntent::Leave)?;
        self.service_intent()
    }
    fn service_intent(&mut self) -> Result<(), MixedPlaybackError> {
        if self.pending.is_some() {
            return Ok(());
        }
        if !self.requested_playing && self.playing {
            let at = self.boundary()?;
            return self.submit(Operation::Stop {
                at,
                accepted: false,
                refused: None,
                position: None,
            });
        }
        if self.intent.is_empty() && self.requested_playing == self.playing {
            return Ok(());
        }
        let result = self.prepare_transport_activation();
        match result {
            Ok(offer) => self.submit(Operation::Activation {
                offer: Some(offer),
                retired: None,
                refused: None,
                intent: self.intent,
            }),
            Err(error) => {
                // A cold unsupported destination is reported once, without stopping sound.
                self.intent.clear();
                self.requested_playing = self.playing;
                Err(error)
            }
        }
    }
    fn prepare_transport_activation(
        &self,
    ) -> Result<synth_engine_v2::stream::MixedActivationOffer, MixedPlaybackError> {
        let at = self.boundary()?;
        let position = self.intent.destination.unwrap_or(self.resume);
        let interval = match self.intent.loop_mode {
            LoopIntent::Keep => self.looping,
            LoopIntent::Leave => None,
            LoopIntent::EnterWholeSong => Some(
                synth_engine_v2::transport::LoopInterval::new(PlanPosition::ZERO, self.end)
                    .ok_or_else(|| preparation("the song has no loopable interval"))?,
            ),
        };
        let kind = if self.requested_playing && !self.playing {
            synth_engine_v2::stream::MixedActivationKind::Start
        } else {
            synth_engine_v2::stream::MixedActivationKind::Seek
        };
        match interval {
            Some(interval) => self
                .bridge
                .prepare_loop_activation(at, position, interval, at, kind),
            None => self.bridge.prepare_activation(at, position, at, kind),
        }
        .map_err(preparation)
    }
    pub(crate) fn editing(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn take_edit_completion(&mut self) -> Option<(super::app::InstallSequence, bool)> {
        self.edit_completion.take()
    }
    pub(crate) fn replace(
        &mut self,
        project: &crate::project::ProjectFile,
        install: super::app::InstallSequence,
        continuation_allowed: impl Fn() -> bool,
    ) -> Result<synth_engine_v2::plan::PlanId, MixedPlaybackError> {
        if self.pending.is_some() {
            return Err(preparation("owning operation pending"));
        }
        let (targets, omissions, end) = lower_targets(project, self.profile, self.binding)?;
        let plan = targets.plan().id();
        if !continuation_allowed() {
            self.prepare_reset(targets, install)?;
            self.edit_metadata = Some((omissions, end));
            return Ok(plan);
        }
        let clock = Arc::clone(&self.clock);
        let margin = self
            .profile
            .capabilities()
            .maximum_block_size()
            .as_u64()
            .checked_mul(2)
            .and_then(|value| value.checked_add(64))
            .ok_or_else(|| preparation("edit margin overflow"))?;
        let packet = match self.bridge.prepare_continuation(
            targets,
            move || {
                if !continuation_allowed() {
                    return None;
                }
                clock
                    .load(Ordering::Acquire)
                    .checked_add(margin)
                    .and_then(|at| at.checked_add(63))
                    .map(|at| SampleTime::new(at / 64 * 64))
            },
            synth_engine_v2::quantities::PreparedBytes::measured(4 * 1024 * 1024),
            synth_engine_v2::quantities::RecordCount::measured(16_384),
        ) {
            Ok(packet) => packet,
            Err(failure) => {
                let (_reason, targets) = *failure;
                self.prepare_reset(targets, install)?;
                self.edit_metadata = Some((omissions, end));
                return Ok(plan);
            }
        };
        self.submit(Operation::Continuation {
            packet: Some(packet),
            offered: false,
            refusal: None,
            install,
        })?;
        self.edit_metadata = Some((omissions, end));
        Ok(plan)
    }
    fn collect_operation(&mut self) -> Result<(), MixedPlaybackError> {
        let Some(mut request) = self.completed.try_pop() else {
            return Ok(());
        };
        if self.pending != Some(request.id) {
            return Err(preparation("foreign owning operation returned"));
        }
        self.pending = None;
        match &mut request.operation {
            Operation::Activation {
                retired,
                refused,
                intent,
                ..
            } => {
                if let Some(retired) = retired.take() {
                    let receipt = self
                        .bridge
                        .collect_activation(retired)
                        .map_err(|_| preparation("activation collection refused"))?;
                    if let synth_engine_v2::stream::MixedActivationOutcome::Adopted(adopted) =
                        receipt.outcome()
                    {
                        self.anchor = adopted.anchor();
                        self.looping = adopted.loop_interval();
                        if adopted.stopped() {
                            self.resume = adopted.anchor().position();
                        }
                        if self.intent.revision == intent.revision {
                            self.intent.clear();
                        }
                        self.playing = !adopted.stopped();
                    } else {
                        self.notices
                            .push(format!("Mixed activation: {:?}", receipt.outcome()));
                    }
                }
                if let Some(reason) = refused {
                    self.notices.push(format!("Mixed activation: {reason}"));
                }
            }
            Operation::Reset {
                owner,
                retired,
                refused,
                install,
                anchor,
                observed_stopped,
                ..
            } => {
                let applied = self.collect_reset(owner.take(), retired.take(), *refused, true)?;
                if applied {
                    if !self.playing {
                        let actual = observed_stopped.ok_or_else(|| {
                            preparation("stopped reset returned no actual audio cursor")
                        })?;
                        if actual != anchor.position() {
                            return Err(preparation(
                                "stopped reset audio cursor differs from its admitted anchor",
                            ));
                        }
                        self.resume = actual;
                    }
                    self.anchor = *anchor;
                    self.looping = None;
                }
                self.edit_completion = Some((*install, applied));
            }
            Operation::Continuation {
                packet,
                refusal,
                install,
                ..
            } => {
                let packet = packet
                    .take()
                    .ok_or_else(|| preparation("continuation owner did not return"))?;
                let returned = if refusal.is_some() {
                    self.bridge.cancel_continuation(packet)
                } else {
                    self.bridge.collect_continuation(packet)
                }
                .map_err(|(_, reason)| preparation(reason))?;
                let installed = matches!(
                    returned.outcome(),
                    Some(
                        synth_engine_v2::stream::MixedContinuationOutcome::InstalledRendered { .. }
                            | synth_engine_v2::stream::MixedContinuationOutcome::InstalledFaulted { .. }
                    )
                );
                self.edit_completion = Some((*install, installed));
                if !installed {
                    self.notice(format!("Mixed edit refused: {:?}", returned.outcome()));
                }
            }
            Operation::Stop {
                refused, position, ..
            } => {
                if let Some(reason) = refused {
                    self.notices.push(format!("Mixed Stop: {reason}"));
                } else {
                    self.playing = false;
                    self.resume =
                        position.ok_or_else(|| preparation("Stop returned no actual cursor"))?;
                }
            }
        }
        if let Some((_, applied)) = self.edit_completion
            && let Some((omissions, end)) = self.edit_metadata.take()
            && applied
        {
            self.omissions = omissions;
            self.end = end;
        }
        if request.rendered != Some(true) {
            return Err(preparation(
                "operation callback faulted; no rendered confirmation",
            ));
        }
        Ok(())
    }
    pub(crate) fn close(
        mut self,
        mut audio: MixedSongAudio,
    ) -> Result<MixedRetirement, MixedPlaybackError> {
        // Both halves are quiescent here; terminal failures destroy their allocations
        // cold and are reported as destruction, never successful joined settlement.
        self.settle_quiescent(&mut audio).map_err(|error| {
            preparation(format!(
                "terminal destruction after quiescence; joined settlement failed: {error}"
            ))
        })?;
        let (active, secondary) = audio.audio.into_owners();
        if secondary.is_some() {
            return Err(preparation("secondary reset owner requires settlement"));
        }
        let physical: Vec<_> = self
            .sources
            .iter()
            .flat_map(|source| source.held.iter().flatten().map(|key| key.onset))
            .collect();
        let (audio, port) = active.into_parts();
        let teardown = self
            .bridge
            .join(audio, Some(self.port), Some(port))
            .map_err(|failure| preparation(failure.reason()))?;
        MixedRetirement::account(&teardown, &physical)
    }
}
