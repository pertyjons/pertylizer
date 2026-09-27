//! Independent, quota-sized handoff to the Core V2 live renderer.
use super::*;
use synth_engine_v2::{
    host::{
        input::{InputTick, SimulatedInputClock},
        live::{AuditionId, AuditionOutcome, LiveInputError, SwappingLiveStream},
    },
    ingress::ReleaseCause,
    ir::{
        ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations,
        PortId, SignalDomain,
    },
    profile::HostProfile,
    quantities::{Amplitude, EventCount, Frequency, HeldNoteCount, NormalizedLevel, Seconds},
    recording::notes::{AuditionTrace, Midi1Input},
    tuning::PreparedTuning,
};

#[derive(Clone, Copy)]
struct Packet {
    id: AuditionId,
    at: SampleTime,
    input: Midi1Input,
}

#[derive(Clone, Copy)]
pub(super) struct PreparedPacket {
    source: ConnectionGeneration,
    port: usize,
    serial: u64,
    tick: InputTick,
    input: Midi1Input,
}

pub struct AuditionControl {
    pub swaps: super::swaps::SwapControl,
    queue: HeapProd<Packet>,
    _backing: Arc<HeapRb<Packet>>,
    settled: HeapProd<AuditionId>,
    outcomes: HeapCons<(AuditionId, AuditionOutcome)>,
    _settled: Arc<HeapRb<AuditionId>>,
    _outcomes: Arc<HeapRb<(AuditionId, AuditionOutcome)>>,
    outstanding: usize,
    sources: [ConnectionGeneration; 2],
    clocks: [SimulatedInputClock; 2],
    serials: [u64; 2],
}
pub struct AuditionAudio {
    queue: HeapCons<Packet>,
    renderer: SwappingLiveStream,
    swaps: super::swaps::SwapAudio,
    settled: HeapCons<AuditionId>,
    outcomes: HeapProd<(AuditionId, AuditionOutcome)>,
    ready: Box<[Option<AuditionId>]>,
    scratch: Box<[f32]>,
    layout: synth_engine_v2::quantities::ChannelLayout,
    maximum_frames: usize,
    end_seen: bool,
    finished: bool,
    refused: Option<Packet>,
}
impl AuditionControl {
    /// `clocks` are the exact values passed to the two core input owners.
    pub(super) fn prepare(
        profile: HostProfile,
        sources: [ConnectionGeneration; 2],
        clocks: [SimulatedInputClock; 2],
    ) -> Result<(Self, AuditionAudio, PreparedBytes), Box<dyn std::error::Error>> {
        let graph = live_graph()?;
        let quota = EventCount::measured(2 * super::source::SOURCE_OUTSTANDING.get());
        let renderer = SwappingLiveStream::prepare(
            &graph,
            profile,
            NodeId::new(2),
            &sources,
            &[],
            quota,
            PreparedBytes::measured(16_100_000),
        )?;
        if renderer.prepared_bytes() > super::swaps::PAYLOAD_BYTES {
            return Err(LiveInputError::Bytes.into());
        }
        let (swaps, swap_audio) = super::swaps::SwapControl::prepare(profile, sources, quota);
        let mut bytes = super::swaps::SwapControl::bytes().get();
        let count = usize::try_from(quota.get())?;
        let frames = usize::try_from(profile.capabilities().maximum_block_size().as_u64())?;
        let samples = frames
            .checked_mul(profile.capabilities().channel_layout().channels())
            .ok_or("live scratch overflow")?;
        let extra = samples
            .checked_mul(size_of::<f32>())
            .and_then(|n| n.checked_add(count * size_of::<Packet>()))
            .and_then(|n| {
                n.checked_add(
                    size_of::<Self>()
                        + size_of::<AuditionAudio>()
                        + size_of::<HeapRb<Packet>>()
                        + 512,
                )
            })
            .ok_or("live host layout overflow")?;
        bytes = bytes
            .checked_add(u64::try_from(extra)?)
            .ok_or("live host layout overflow")?;
        let backing = Arc::new(HeapRb::new(count));
        let (writer, reader) = Arc::clone(&backing).split();
        let settled = Arc::new(HeapRb::new(count));
        let outcomes = Arc::new(HeapRb::new(count));
        let (settle_writer, settle_reader) = Arc::clone(&settled).split();
        let (outcome_writer, outcome_reader) = Arc::clone(&outcomes).split();
        bytes = bytes
            .checked_add(u64::try_from(
                count
                    * (size_of::<AuditionId>()
                        + size_of::<Option<AuditionId>>()
                        + size_of::<(AuditionId, AuditionOutcome)>())
                    + size_of::<HeapRb<AuditionId>>()
                    + size_of::<HeapRb<(AuditionId, AuditionOutcome)>>()
                    + 512,
            )?)
            .ok_or("live result layout overflow")?;
        Ok((
            Self {
                swaps,
                queue: writer,
                _backing: backing,
                settled: settle_writer,
                outcomes: outcome_reader,
                _settled: settled,
                _outcomes: outcomes,
                outstanding: 0,
                sources,
                clocks,
                serials: [0; 2],
            },
            AuditionAudio {
                queue: reader,
                renderer,
                swaps: swap_audio,
                settled: settle_reader,
                outcomes: outcome_writer,
                ready: vec![None; count].into_boxed_slice(),
                scratch: vec![0.0; samples].into_boxed_slice(),
                layout: profile.capabilities().channel_layout(),
                maximum_frames: frames,
                end_seen: false,
                finished: false,
                refused: None,
            },
            PreparedBytes::measured(bytes),
        ))
    }
    /// A pure reservation check. The core classifies invalid source time.
    pub(super) fn preflight(
        &self,
        source: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<Option<PreparedPacket>, InputError> {
        let InputObservation::Message { tick, input, .. } = observation else {
            return Ok(None);
        };
        let port = self
            .sources
            .iter()
            .position(|generation| *generation == source)
            .ok_or(InputError::Stale)?;
        if self.outstanding >= self.queue.capacity().get() {
            return Err(InputError::Full);
        }
        let serial = self.serials[port]
            .checked_add(1)
            .ok_or(InputError::IdentityExhausted)?;
        let _id = AuditionId::new(source, serial).map_err(|_| InputError::IdentityExhausted)?;
        Ok(Some(PreparedPacket {
            source,
            port,
            serial,
            tick,
            input,
        }))
    }
    /// Commit follows raw acceptance; failure retains an accepted core ID.
    pub(super) fn commit(&mut self, prepared: PreparedPacket) -> Result<AuditionTrace, InputError> {
        let at = self.clocks[prepared.port].map(prepared.tick)?;
        let id = AuditionId::new(prepared.source, prepared.serial)
            .map_err(|_| InputError::IdentityExhausted)?;
        self.queue
            .try_push(Packet {
                id,
                at,
                input: prepared.input,
            })
            .map_err(|_| InputError::Full)?;
        self.serials[prepared.port] = prepared.serial;
        self.outstanding += 1;
        Ok(AuditionTrace::Pending(id))
    }
    pub fn settle(&mut self, trace: AuditionTrace) -> Result<(), InputError> {
        if let AuditionTrace::Pending(id) = trace {
            // Each unsettled ID still owns an outstanding credit, so a full ring
            // is unreachable under normal credit accounting. Keep the error for
            // fault injection and any future change to that accounting.
            self.settled.try_push(id).map_err(|_| InputError::Full)?;
        }
        Ok(())
    }
    pub fn collect(&mut self) -> Option<(AuditionId, AuditionOutcome)> {
        let outcome = self.outcomes.try_pop()?;
        self.outstanding -= 1;
        Some(outcome)
    }
    #[cfg(test)]
    pub(super) fn custody(&self) -> ([u64; 2], usize, usize) {
        (self.serials, self.outstanding, self.queue.occupied_len())
    }
    #[cfg(test)]
    /// Inject a ring state that normal outstanding-credit accounting excludes.
    pub(super) fn fill_settlements(&mut self, source: ConnectionGeneration) {
        for serial in 1..=self.settled.capacity().get() {
            let id = AuditionId::new(source, u64::try_from(serial).unwrap()).unwrap();
            self.settled.try_push(id).unwrap();
        }
    }
}
impl AuditionAudio {
    pub fn validate(&self, output: &AudioBlockMut<'_>) -> Result<(), LiveInputError> {
        if output.layout() != self.layout
            || output.frames() == 0
            || output.frames() > self.maximum_frames
            || output
                .frames()
                .checked_mul(output.layout().channels())
                .is_none_or(|n| n > self.scratch.len())
        {
            return Err(LiveInputError::Shape);
        }
        Ok(())
    }
    fn drain(&mut self) -> Result<(), LiveInputError> {
        if self.refused.is_some() {
            return Err(LiveInputError::Closed);
        }
        let prefix = self.queue.occupied_len();
        for _ in 0..prefix {
            let Some(packet) = self.queue.try_pop() else {
                break;
            };
            if let Err(error) = self.renderer.queue(packet.id, packet.at, packet.input) {
                self.refused = Some(packet);
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn render(
        &mut self,
        output: &mut AudioBlockMut<'_>,
        end: Option<(SampleTime, SessionCommand)>,
    ) -> Result<(), LiveInputError> {
        self.drain()?;
        if !self.end_seen
            && let Some((at, command)) = end
        {
            self.renderer.end_at(
                at,
                if command == SessionCommand::Panic {
                    ReleaseCause::Panic
                } else {
                    ReleaseCause::Stop
                },
            )?;
            self.end_seen = true;
        }
        let frames = output.frames();
        let samples = frames * output.layout().channels();
        let scratch = self
            .scratch
            .get_mut(..samples)
            .ok_or(LiveInputError::Shape)?;
        let mut done = 0;
        while done < frames {
            self.swaps.service(&mut self.renderer)?;
            let count = (frames - done).min(self.renderer.frames_until_boundary());
            let channels = output.layout().channels();
            self.renderer.render_deferred(
                AudioBlockMut::new(
                    &mut scratch[done * channels..(done + count) * channels],
                    count,
                    output.layout(),
                )
                .map_err(|_| LiveInputError::Shape)?,
            )?;
            done += count;
        }
        self.renderer.commit_outcomes();
        self.swaps.acknowledge(&self.renderer);
        for (sample, live) in output.samples_mut().iter_mut().zip(scratch) {
            *sample += *live;
        }
        Ok(())
    }
    pub fn reconcile(&mut self, core: &mut InputCaptureAudio) -> Result<(), HostError> {
        let prefix = self.settled.occupied_len();
        for _ in 0..prefix {
            let Some(cell) = self.ready.iter_mut().find(|cell| cell.is_none()) else {
                return Err(LiveInputError::Capacity.into());
            };
            *cell = self.settled.try_pop();
        }
        let mut resolved = 0;
        for ready in &mut self.ready {
            if resolved == 8 {
                break;
            }
            let Some(id) = *ready else {
                continue;
            };
            if self.outcomes.is_full() {
                break;
            }
            let outcome = self
                .renderer
                .outcomes()
                .find(|(candidate, _)| *candidate == id)
                .map(|(_, outcome)| outcome);
            if let Some(outcome) = outcome {
                let _resolved_records = core.resolve_audition(id, outcome)?;
                self.outcomes
                    .try_push((id, outcome))
                    .map_err(|_| LiveInputError::Capacity)?;
                if self.renderer.take_outcome(id).is_none() {
                    return Err(LiveInputError::Identity.into());
                }
                *ready = None;
                resolved += 1;
            }
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<(), LiveInputError> {
        self.renderer.recover_after_join();
        if let Some(packet) = self.refused.take()
            && let Err(error) = self.renderer.queue(packet.id, packet.at, packet.input)
        {
            self.refused = Some(packet);
            return Err(error);
        }
        let prefix = self.queue.occupied_len();
        for _ in 0..prefix {
            let Some(packet) = self.queue.try_pop() else {
                break;
            };
            if let Err(error) = self.renderer.queue(packet.id, packet.at, packet.input) {
                self.refused = Some(packet);
                return Err(error);
            }
        }
        self.finished = true;
        Ok(())
    }
    pub fn is_finished(&self) -> bool {
        self.finished && self.refused.is_none() && self.queue.is_empty()
    }
    pub fn outcomes(&self) -> impl Iterator<Item = (AuditionId, AuditionOutcome)> + '_ {
        self.renderer.outcomes()
    }
    pub fn clock(&self) -> SampleTime {
        self.renderer.clock()
    }
    #[cfg(test)]
    pub(super) fn free_settlement_slot(&mut self) -> Option<AuditionId> {
        self.settled.try_pop()
    }
    #[cfg(test)]
    /// Reproduce the owner locations after a renderer refusal without filling
    /// its entire held-note table through the raw capture fixture.
    pub(super) fn hold_refused_after(&mut self, accepted: usize) -> Result<(), LiveInputError> {
        for _ in 0..accepted {
            let packet = self.queue.try_pop().ok_or(LiveInputError::Identity)?;
            self.renderer.queue(packet.id, packet.at, packet.input)?;
        }
        self.refused = Some(self.queue.try_pop().ok_or(LiveInputError::Identity)?);
        Ok(())
    }
    #[cfg(test)]
    pub(super) fn duplicate_refused_id(&mut self, serial: u64) -> Result<(), LiveInputError> {
        let packet = self.refused.as_mut().ok_or(LiveInputError::Identity)?;
        packet.id = AuditionId::new(packet.id.source(), serial)?;
        Ok(())
    }
}

pub fn live_graph() -> Result<GraphIr, Box<dyn std::error::Error>> {
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0)?,
                amplitude: Amplitude::new(0.05)?,
            },
            ExecutionScope::Voice,
        )
        .node(
            NodeId::new(2),
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(NodeId::new(3), IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(NodeId::new(4), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(3), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (NodeId::new(2), PortId::FIRST),
            (NodeId::new(3), synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (NodeId::new(3), PortId::FIRST),
            (NodeId::new(4), PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(ExecutionScope::Voice, PreparedTuning::equal_temperament()?)
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(4),
                simultaneous_holds: EventCount::measured(4),
            }],
            ..PlanDeclarations::default()
        })
        .build()?;
    Ok(graph)
}
