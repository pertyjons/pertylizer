//! Phase 9 Stage 6's concrete two-source bridge, through the public API only.
//!
//! Two real source rings (the example host's `SourceInbox` and `SourceProducer`, included
//! from `tests/support`) feed one real raw owner, an `InputCaptureControl` split from a
//! prepared capture session, and the bridge's bounded two-source stage. The host offers
//! each ring head to raw capture while it runs and hands the same packet, named by its
//! source ring's ID, to the bridge, which forwards commands through ADR-0078's host
//! queues to the mixed audio half. The raw owner's own packets travel through a second
//! pair of host rings to its `InputCaptureAudio`, which every host callback renders
//! beside the mixed audio half, and control collects the raw owner's `InputReceipt`s.
//!
//! Every test settles each source message handed to the bridge exactly once for sound:
//! by the stage call, by a pump split, by a collected report, or in joined teardown.
//! Joined teardown also returns exactly one raw receipt for every raw admission, whose
//! `matched_onset` is its admission-time link. Where raw linked a key-up, the bridge
//! paired it with the same onset on its own.
//!
//! The host has two halts. A terminal fault, mixed or source, stops the whole host: it
//! requests the sources' host halt, then the raw owner's, and a terminal mixed callback
//! does so before raw renders that callback, so capture ends with sound (rule 1.8). A
//! source refusal, and a raw admission refusal while raw capture runs, are reported to
//! the bridge. Raw capture that interrupts itself, as when its recorder refuses a late
//! packet, stops capture for both sources and nothing else: the host offers raw nothing
//! more and keeps handing both sources' packets to the bridge. It is not reported to the
//! bridge, which would charge the waiting source for its peer's silence.
#![cfg(feature = "mixed-transfer-rehearsal")]

mod common;

#[path = "support/callback_probe.rs"]
mod callback_probe;

#[path = "support/frozen_capacity.rs"]
mod frozen_capacity;

#[path = "support/instrument_inserts.rs"]
mod instrument_inserts;

#[path = "support/saved_project_host.rs"]
mod saved_project_host;

// The quality gate runs this binary with the rehearsal feature. Include the selector's
// arithmetic controls here too, so CI executes them rather than only type-checking them.
#[path = "mixed_capacity_selection.rs"]
mod capacity_selection_controls;

#[allow(
    dead_code,
    reason = "the shared source ring is exercised here through its owned path"
)]
#[path = "support/source_queue.rs"]
mod source_queue;

#[allow(
    dead_code,
    reason = "the shared source custody is exercised here through its owned path"
)]
#[path = "support/source_custody.rs"]
mod source_custody;

use std::cell::RefCell;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use source_custody::{
    SourceHandoff, SourceHostHalt, SourceInbox, SourceOwnedSendError, SourceProducer,
    SourceRetryToken,
};
use synth_core::MidiChannel;
use synth_engine_v2::{
    compile::{RenderConfig, compile},
    host::{
        ConnectionGeneration, EndpointId,
        input::{
            InputCapacity, InputCaptureAudio, InputCaptureControl, InputCaptureError,
            InputCaptureHalt, InputCaptureSession, InputError, InputEventId, InputLimits,
            InputObservation, InputOutcome, InputRate, InputReceipt, InputReuniteError, InputTick,
            InputTickSpan, SimulatedInputClock, SimulatedNoteInput,
        },
        mixed_targets::MixedTargetAdmission,
        session::{
            LoopRecordingSession, LoopSessionError, SessionCommand, SessionCommandCapacity,
            SessionError, SessionLimits, SessionOutcome, SessionSourceCapacity,
            SessionSourceLimits, SessionSourceOutcome,
            loop_transfer::{
                LoopTransferCompletion, LoopTransferError, LoopTransferId, LoopTransferOutcome,
                LoopTransferPacket,
            },
        },
    },
    identity::NoteIdentity,
    ir::{
        ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations,
        PortId, SignalDomain,
    },
    looping::{CompiledLoopStream, LoopPassId, LoopSettings},
    plan::CompiledPlan,
    profile::{CaptureLimits, CaptureLimitsInput, HostProfile, RecordingLimits},
    quantities::{
        Amplitude, CapturePassCount, CaptureResultCount, CaptureSourceCount, ChannelLayout,
        EventCount, Frequency, HeldNoteCount, KeyIdentity, NormalizedLevel, NoteVelocity,
        PreparedBytes, ProjectionTickCount, Seconds, TrackedInputNoteCount,
    },
    recording::notes::{
        CaptureDisposition, CaptureMode, CaptureQuantization, ControllerSnapshot, FixtureRevision,
        FixtureTargetId, Midi1Event, Midi1Input, MusicalInterval, NoteCaptureError,
        loop_capture::{LoopCaptureSession, LoopNoteArmInput},
    },
    render::AudioBlockMut,
    schedule::{AdmittedCompiledStream, CompiledPayload, PlanEvent},
    stream::{
        MixedActivationAdopted, MixedActivationCollectError, MixedActivationKind,
        MixedActivationLoop, MixedActivationOffer, MixedActivationOutcome, MixedActivationReceipt,
        MixedActivationRefusal, MixedActivationRetired, MixedActivationSlot, MixedBridgeArmed,
        MixedBridgeControl, MixedBridgeError, MixedBridgeFault, MixedBridgeOutcome,
        MixedBridgePanicEnd, MixedBridgePanicRefusal, MixedBridgeReport, MixedBridgeResetAudio,
        MixedBridgeResetEnd, MixedBridgeResetOwner, MixedBridgeResetPhase, MixedBridgeResetRefusal,
        MixedBridgeResetRenderError, MixedBridgeResetRetired, MixedBridgeResetState,
        MixedBridgeRetiredControl, MixedBridgeRetiredTeardown, MixedBridgeStopAccepted,
        MixedBridgeTeardown, MixedBridgeWait, MixedCommandPlace, MixedJoinedPrepared,
        MixedJoinedStream, MixedKeyUpRetired, MixedOnsetRefusal, MixedPumpStep, MixedRetiredHandle,
        MixedSourceCustody, MixedSourceFault, MixedSourceHandoff, MixedSourcePacketId, MixedSplit,
        MixedStaged, MixedStampedCandidate, MixedTransferArmError, MixedTransferAudio,
        MixedTransferAudioPort, MixedTransferCommand, MixedTransferConsumer,
        MixedTransferControlPort, MixedTransferCustody, MixedTransferFault, MixedTransferFence,
        MixedTransferHandleState, MixedTransferPedalDown, MixedTransferPlaces,
        MixedTransferProducer, MixedTransferQueue, MixedTransferRenderError, MixedTransferResult,
        MixedTransferSources, MixedTransferSourcesError,
    },
    tempo::{Bpm, MusicalTick, TempoMap},
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor},
    transport::{ActivationSequence, LoopInterval},
};

const SOURCE_NODE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
/// The largest callback any test renders.
const BLOCK: u64 = 1_024;
const Q: usize = QUANTUM_FRAMES as usize;
const FIRST: usize = 0;
const SECOND: usize = 1;

// ---------------------------------------------------------------------------------------
// Host queues: a preallocated lock-free SPSC ring that can lie about its capacity or
// lose one accepted packet, to inject host faults.

/// Immediate push-side peaks are qualified only for the serial source/control/audio
/// schedule. A concurrent consumer may pop before the occupancy load.
#[derive(Clone)]
struct QueuePeak(Arc<AtomicUsize>);

impl QueuePeak {
    fn new() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }
    fn observe(&self, occupied: usize) {
        let _previous = self.0.fetch_max(occupied, Ordering::Relaxed);
    }
    fn get(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.0.load(Ordering::Relaxed))
    }
}

#[derive(Clone)]
struct OwnerQueuePeaks {
    commands: QueuePeak,
    results: QueuePeak,
}

struct ObservedProducer<T> {
    ring: HeapProd<T>,
    peak: QueuePeak,
}
impl<T> ObservedProducer<T> {
    fn try_push(&mut self, packet: T) -> Result<(), T> {
        let result = self.ring.try_push(packet);
        if result.is_ok() {
            self.peak.observe(self.ring.occupied_len());
        }
        result
    }
    fn is_full(&self) -> bool {
        self.ring.is_full()
    }
}

impl<T> MixedTransferProducer<T> for ObservedProducer<T> {
    fn try_push(&mut self, packet: T) -> Result<(), T> {
        ObservedProducer::try_push(self, packet)
    }
}

#[derive(Clone, Copy, Debug)]
struct CellPeaks {
    occupied: MixedTransferPlaces,
    protected: MixedTransferPlaces,
    charged: MixedTransferPlaces,
}
impl CellPeaks {
    fn empty() -> Self {
        Self {
            occupied: MixedTransferPlaces::NONE,
            protected: MixedTransferPlaces::NONE,
            charged: MixedTransferPlaces::NONE,
        }
    }
    fn observe(&mut self, occupied: usize, protected: usize) {
        let charged = occupied
            .checked_add(protected)
            .expect("allocated cell counts fit");
        self.occupied = MixedTransferPlaces::new(self.occupied.get().max(occupied));
        self.protected = MixedTransferPlaces::new(self.protected.get().max(protected));
        self.charged = MixedTransferPlaces::new(self.charged.get().max(charged));
    }
}

struct HostQueue<T> {
    ring: HeapRb<T>,
    peak: QueuePeak,
    reported_capacity: Option<usize>,
    lose_push: Option<usize>,
}

impl<T> HostQueue<T> {
    fn new(capacity: usize) -> Self {
        Self {
            ring: HeapRb::new(capacity),
            peak: QueuePeak::new(),
            reported_capacity: None,
            lose_push: None,
        }
    }
}

struct HostProducer<T> {
    ring: HeapProd<T>,
    peak: QueuePeak,
    lose_push: Option<usize>,
    pushes: usize,
}

struct HostConsumer<T>(HeapCons<T>);

impl<T> MixedTransferQueue<T> for HostQueue<T> {
    type Producer = HostProducer<T>;
    type Consumer = HostConsumer<T>;

    fn capacity(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(
            self.reported_capacity
                .unwrap_or_else(|| self.ring.capacity().get()),
        )
    }

    fn occupied(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.ring.occupied_len())
    }

    fn split(self) -> (Self::Producer, Self::Consumer) {
        let (producer, consumer) = self.ring.split();
        (
            HostProducer {
                ring: producer,
                peak: self.peak,
                lose_push: self.lose_push,
                pushes: 0,
            },
            HostConsumer(consumer),
        )
    }
}

impl<T> MixedTransferProducer<T> for HostProducer<T> {
    fn try_push(&mut self, packet: T) -> Result<(), T> {
        self.pushes += 1;
        if self.lose_push == Some(self.pushes) {
            // A deliberately unqualified queue: it accepts and then loses this packet.
            drop(packet);
            return Ok(());
        }
        let result = self.ring.try_push(packet);
        if result.is_ok() {
            self.peak.observe(self.ring.occupied_len());
        }
        result
    }
}

impl<T> MixedTransferConsumer<T> for HostConsumer<T> {
    fn try_pop(&mut self) -> Option<T> {
        self.0.try_pop()
    }
}

type Commands = HostQueue<MixedTransferCommand>;
type Results = HostQueue<MixedTransferResult>;
type ControlPort =
    MixedTransferControlPort<HostProducer<MixedTransferCommand>, HostConsumer<MixedTransferResult>>;
type AudioPort =
    MixedTransferAudioPort<HostConsumer<MixedTransferCommand>, HostProducer<MixedTransferResult>>;

// ---------------------------------------------------------------------------------------
// The bound mixed plan.

fn profile() -> HostProfile {
    common::profile(BLOCK, ChannelLayout::Mono)
}

/// The admitted command/result bound: the profile's live ingress depth.
fn depth() -> usize {
    profile()
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity()
        .as_usize()
        .expect("depth fits")
}

/// Places each bridge host queue must report: the admitted bound and Panic's protected
/// place beyond it.
fn places() -> usize {
    depth() + 1
}

fn key(raw: u8) -> KeyIdentity {
    KeyIdentity::new(raw).expect("valid key")
}

fn at(frames: u64) -> SampleTime {
    SampleTime::new(frames)
}

/// One compiled producer and one live producer of four notes and four holds, through one
/// pitched voice graph. The compiled note sounds from frame 64 to the end, so a silent
/// callback can only be a terminal one.
fn plan() -> CompiledPlan {
    plan_with(0.25, profile())
}

fn plan_with(amplitude: f32, profile: HostProfile) -> CompiledPlan {
    let compiled = NoteProducerDeclaration {
        compiled: true,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::NONE,
    };
    let live = NoteProducerDeclaration {
        compiled: false,
        simultaneous_notes: HeldNoteCount::measured(4),
        simultaneous_holds: EventCount::measured(4),
    };
    let ir = GraphIr::builder()
        .node(
            SOURCE_NODE,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(amplitude).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE_NODE, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(ExecutionScope::Voice, common::twelve_tet())
        .declaring(PlanDeclarations {
            note_producers: vec![compiled, live],
            held_notes: HeldNoteCount::measured(6),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a readable mixed graph");
    common::admit(&ir, profile)
}

/// The bound schedule and an activation candidate at `requested` that replays a
/// compiled note held across the boundary.
fn prepared_at(requested: SampleTime) -> (MixedJoinedPrepared, MixedStampedCandidate) {
    prepared_plan_at(plan(), requested)
}

fn prepared_plan_at(
    plan: CompiledPlan,
    requested: SampleTime,
) -> (MixedJoinedPrepared, MixedStampedCandidate) {
    let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
    let events = [PlanEvent::new(
        PlanPosition::new(10),
        CompiledPayload::NoteOn {
            slot: note,
            key: key(57),
            velocity: NoteVelocity::FULL,
        },
    )];
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("admitted stream");
    let binding = MixedTargetAdmission::admit(plan, stream, note).expect("disjoint targets");
    let prepared = MixedJoinedStream::open(
        binding,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("joined owner")
    .prepare_initial()
    .expect("sealed initial schedule");
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamped candidate");
    (prepared, candidate)
}

fn prepared() -> (MixedJoinedPrepared, MixedStampedCandidate) {
    prepared_at(at(64))
}

// ---------------------------------------------------------------------------------------
// The raw owner: a prepared two-input capture session, split for concurrent ownership.

/// One tick per frame, so a source tick is its nominal stamp.
fn clock(epoch: synth_engine_v2::time::StreamEpoch) -> SimulatedInputClock {
    SimulatedInputClock::new(
        epoch,
        SampleTime::ZERO,
        InputTick::new(0),
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).expect("rate"),
        InputTickSpan::new(0),
    )
}

struct RawOwner {
    /// Off-thread preparation order; handles survive each owner's joined teardown.
    queue_owners: RefCell<Vec<OwnerQueuePeaks>>,
    control: InputCaptureControl,
    /// Moved into the host callback, which renders it beside the mixed audio half.
    audio: Option<InputCaptureAudio>,
    halt: InputCaptureHalt,
    generations: [ConnectionGeneration; 2],
    clocks: [SimulatedInputClock; 2],
}

fn raw_owner(cells: u32) -> RawOwner {
    let rate = profile().capabilities().sample_rate();
    raw_owner_at(cells, rate, FrameCount::new(BLOCK))
}

fn raw_owner_at(
    cells: u32,
    rate: synth_engine_v2::quantities::SampleRate,
    max_block: FrameCount,
) -> RawOwner {
    raw_owner_with_limits(
        cells,
        rate,
        max_block,
        SessionCommandCapacity::new(4).expect("commands"),
        SessionSourceCapacity::new(256).expect("actions"),
    )
}

fn raw_owner_with_limits(
    cells: u32,
    rate: synth_engine_v2::quantities::SampleRate,
    max_block: FrameCount,
    commands: SessionCommandCapacity,
    actions: SessionSourceCapacity,
) -> RawOwner {
    raw_owner_with_budgets(
        cells,
        rate,
        max_block,
        commands,
        actions,
        PreparedBytes::measured(262_144),
    )
}

fn raw_owner_with_budgets(
    cells: u32,
    rate: synth_engine_v2::quantities::SampleRate,
    max_block: FrameCount,
    commands: SessionCommandCapacity,
    actions: SessionSourceCapacity,
    action_bytes: PreparedBytes,
) -> RawOwner {
    let capture_profile =
        HostProfile::harness(rate, max_block, ChannelLayout::Mono).expect("capture profile");
    let graph = GraphIr::builder()
        .node(NodeId::new(1), IrNodeKind::Silence, ExecutionScope::Global)
        .node(NodeId::new(2), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(2), PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("silent capture graph");
    let plan = compile(&graph, &RenderConfig::new(capture_profile))
        .into_plan()
        .expect("capture plan");
    let events = AdmittedCompiledStream::admit(&plan, &[]).expect("no compiled events");
    let tempo = TempoMap::new(Bpm::new(120.0).expect("bpm"), &[], rate).expect("tempo");
    let interval = MusicalInterval::new(
        MusicalTick::ZERO,
        MusicalTick::new(4 * u64::from(synth_engine_v2::tempo::TICKS_PER_QUARTER)),
    )
    .expect("one bar");
    let end = tempo.position_of(interval.end()).expect("bar end");
    let stream = CompiledLoopStream::prepare(
        plan,
        events,
        capture_profile,
        LoopSettings::new(
            LoopInterval::new(PlanPosition::ZERO, end).expect("loop"),
            PlanPosition::ZERO,
            PreparedBytes::measured(1_000_000),
        )
        .expect("loop settings"),
    )
    .expect("capture loop");
    let limits = RecordingLimits::new(
        HeldNoteCount::limit(128).expect("held"),
        EventCount::limit(256).expect("events"),
    )
    .expect("recording limits")
    .with_capture(
        CaptureLimits::new(CaptureLimitsInput {
            max_tracked_input_notes: TrackedInputNoteCount::limit(128).expect("tracked"),
            max_capture_sources: CaptureSourceCount::limit(2).expect("sources"),
            max_capture_passes: CapturePassCount::limit(32).expect("passes"),
            max_pending_capture_results: CaptureResultCount::limit(1).expect("results"),
            max_capture_bytes: PreparedBytes::measured(2_097_152),
            max_audio_capture_frames: FrameCount::new(1),
            max_projection_ticks: ProjectionTickCount::limit(4_096).expect("ticks"),
            capture_lateness_allowance: FrameCount::ZERO,
        })
        .expect("capture limits"),
    )
    .expect("recording limits with capture");
    let mut capture = LoopCaptureSession::prepare(stream, limits, PreparedBytes::measured(8192))
        .expect("capture session");
    let epoch = capture.initial().epoch;
    let mut inputs = Vec::new();
    let mut sources = Vec::new();
    let mut generations = Vec::new();
    for port in 0..2 {
        let mut input = SimulatedNoteInput::new(
            EndpointId::new(format!("bridge-{port}")).expect("endpoint"),
            InputLimits {
                cells: InputCapacity::new(cells).expect("cells"),
                bytes: PreparedBytes::measured(1 << 20),
            },
        )
        .expect("input");
        let generation = input.begin().expect("generation");
        input.prepare(generation, clock(epoch)).expect("clock");
        sources.push(
            input
                .bind_capture(generation, &mut capture, ControllerSnapshot::neutral())
                .expect("bound source"),
        );
        generations.push(generation);
        inputs.push(input);
    }
    let _ticket = capture
        .arm_at(
            LoopNoteArmInput {
                target: FixtureTargetId::new(1).expect("target"),
                expected_revision: FixtureRevision::new(1),
                interval,
                mode: CaptureMode::Overdub,
                quantization: CaptureQuantization::Off,
                tempo,
            },
            &sources,
            SampleTime::ZERO,
        )
        .expect("armed capture");
    let session = LoopRecordingSession::prepare(
        capture,
        SessionLimits {
            commands,
            command_bytes: PreparedBytes::measured(16_384),
        },
        SessionSourceLimits {
            actions,
            bytes: action_bytes,
        },
    )
    .expect("recording session");
    let owner = InputCaptureSession::prepare(
        session,
        inputs.into_boxed_slice(),
        PreparedBytes::measured(8192),
    )
    .expect("capture owner");
    let (mut control, audio, halt) = owner
        .split(PreparedBytes::measured(1 << 20))
        .expect("split raw owner");
    for &generation in &generations {
        control.start_input(generation).expect("started input");
    }
    RawOwner {
        queue_owners: RefCell::new(Vec::new()),
        control,
        audio: Some(audio),
        halt,
        generations: [generations[0], generations[1]],
        clocks: [clock(epoch), clock(epoch)],
    }
}

// ---------------------------------------------------------------------------------------
// The concrete host: two source rings, the raw owner, the bridge and its queues.

#[test]
fn protected_key_and_pedal_releases_enter_a_saturated_source_ring() {
    let raw = raw_owner(64);
    let (mut producer, mut inbox) =
        SourceInbox::prepare(raw.generations[FIRST], raw.halt.clone(), raw.clocks[FIRST]);
    let mut observations = vec![message(100, note_on(60)), message(101, sustain(true))];
    for tick in 102..116 {
        observations.push(message(tick, bend(8192)));
    }
    assert_eq!(observations.len(), source_custody::SOURCE_CELLS);
    for observation in &observations {
        assert!(producer.submit_owned(*observation).is_ok());
    }
    assert_eq!(inbox.occupied_len(), source_custody::SOURCE_CELLS);

    let key_up = message(116, note_off(60));
    let lift = message(117, sustain(false));
    assert!(producer.submit_owned(key_up).is_ok());
    assert!(producer.submit_owned(lift).is_ok());
    assert_eq!(inbox.occupied_len(), source_custody::SOURCE_CELLS + 2);

    let ordinary = message(118, bend(8192));
    let retry = match producer.submit_owned(ordinary) {
        Err(SourceOwnedSendError::Retry(token)) => token,
        other => panic!("ordinary traffic must wait behind protected releases: {other:?}"),
    };
    let mut observed = Vec::new();
    while let Some(head) = inbox.peek_handoff() {
        observed.push(head.observation());
        assert_eq!(inbox.commit_handoff(head), Ok(head));
    }
    assert_eq!(observed, [observations, vec![key_up, lift]].concat());
    assert_eq!(inbox.occupied_len(), 0);
    assert!(producer.retry_owned(retry).is_ok());
    assert_eq!(
        inbox.peek_handoff().map(SourceHandoff::observation),
        Some(ordinary)
    );
}

#[test]
fn source_ring_keeps_all_eighty_protected_releases_under_ordinary_pressure() {
    let raw = raw_owner(64);
    let (mut producer, mut inbox) =
        SourceInbox::prepare(raw.generations[FIRST], raw.halt.clone(), raw.clocks[FIRST]);
    let mut tick = 100_u64;
    for key in 0..64_u8 {
        assert!(producer.submit_owned(message(tick, note_on(key))).is_ok());
        let head = inbox.peek_handoff().expect("accepted onset head");
        assert_eq!(inbox.commit_handoff(head), Ok(head));
        tick += 1;
    }
    for channel in 0..16_u8 {
        let down = Midi1Input::from_bytes([0xb0 + channel, 64, 127]).expect("pedal down");
        assert!(producer.submit_owned(message(tick, down)).is_ok());
        let head = inbox.peek_handoff().expect("accepted pedal head");
        assert_eq!(inbox.commit_handoff(head), Ok(head));
        tick += 1;
    }
    assert_eq!(inbox.occupied_len(), 0);
    for _ in 0..source_custody::SOURCE_CELLS {
        assert!(producer.submit_owned(message(tick, bend(8192))).is_ok());
        tick += 1;
    }
    for key in 0..64_u8 {
        assert!(producer.submit_owned(message(tick, note_off(key))).is_ok());
        tick += 1;
    }
    for channel in 0..16_u8 {
        let up = Midi1Input::from_bytes([0xb0 + channel, 64, 0]).expect("pedal lift");
        assert!(producer.submit_owned(message(tick, up)).is_ok());
        tick += 1;
    }
    assert_eq!(inbox.occupied_len(), source_custody::SOURCE_CELLS + 80);
    let next = message(tick, bend(8192));
    assert!(matches!(
        producer.submit_owned(next),
        Err(SourceOwnedSendError::Retry(_))
    ));
}

#[test]
fn retrying_an_ordinary_packet_keeps_fifo_before_its_protected_key_up() {
    let raw = raw_owner(64);
    let (mut producer, mut inbox) =
        SourceInbox::prepare(raw.generations[FIRST], raw.halt.clone(), raw.clocks[FIRST]);
    assert!(producer.submit_owned(message(100, note_on(60))).is_ok());
    for tick in 101..116 {
        assert!(producer.submit_owned(message(tick, bend(8192))).is_ok());
    }
    let ordinary = message(116, bend(12_288));
    let token = match producer.submit_owned(ordinary) {
        Err(SourceOwnedSendError::Retry(token)) => token,
        other => panic!("ordinary packet should wait for its FIFO place: {other:?}"),
    };
    let first = inbox.peek_handoff().expect("onset head");
    assert_eq!(inbox.commit_handoff(first), Ok(first));
    assert_eq!(first.observation(), message(100, note_on(60)));
    assert!(producer.retry_owned(token).is_ok());
    let key_up = message(117, note_off(60));
    assert!(producer.submit_owned(key_up).is_ok());
    let mut observed = Vec::new();
    while let Some(head) = inbox.peek_handoff() {
        observed.push(head.observation());
        assert_eq!(inbox.commit_handoff(head), Ok(head));
    }
    assert_eq!(observed[15], ordinary);
    assert_eq!(observed[16], key_up);
    assert!(!raw.halt.is_requested());
}

#[test]
fn a_sixty_fifth_held_note_reports_a_protected_overrun() {
    let mut raw = raw_owner(64);
    let (mut producer, mut inbox) =
        SourceInbox::prepare(raw.generations[FIRST], raw.halt.clone(), raw.clocks[FIRST]);
    for key in 0..64_u8 {
        assert!(
            producer
                .submit_owned(message(100 + u64::from(key), note_on(key)))
                .is_ok()
        );
        let head = inbox.peek_handoff().expect("accepted onset");
        assert_eq!(inbox.commit_handoff(head), Ok(head));
    }
    let excess = message(164, note_on(64));
    assert!(matches!(
        producer.submit_owned(excess),
        Err(SourceOwnedSendError::Refused(
            source_custody::SourceSendError::Invalid(original, InputError::ProtectedCapacity)
        )) if original == excess
    ));
    assert_eq!(
        inbox.unresolved_failure(),
        Some((excess, InputError::ProtectedCapacity))
    );
    assert_eq!(
        raw.control.record_pre_ring_failure(
            raw.generations[FIRST],
            excess,
            InputError::ProtectedCapacity
        ),
        Ok(())
    );
}

#[test]
fn source_reservation_overrun_is_not_reported_as_retry_expiry() {
    let mut rig = Rig::new(Setup::default());
    for key in 0..64_u8 {
        assert!(
            rig.producer(FIRST)
                .submit_owned(message(100 + u64::from(key), note_on(key)))
                .is_ok()
        );
        let head = rig.inboxes[FIRST].peek_handoff().expect("source head");
        assert_eq!(rig.inboxes[FIRST].commit_handoff(head), Ok(head));
    }
    let excess = message(164, note_on(64));
    assert!(matches!(
        rig.producer(FIRST).submit_owned(excess),
        Err(SourceOwnedSendError::Refused(
            source_custody::SourceSendError::Invalid(_, InputError::ProtectedCapacity)
        ))
    ));
    rig.stage_all();
    assert_eq!(
        rig.pre_ring,
        vec![(FIRST, excess, InputError::ProtectedCapacity)]
    );
    assert_eq!(rig.pre_ring_recorded, [Ok(())]);
    assert_eq!(
        rig.control.fault(),
        Some(MixedBridgeFault::Source {
            connection: rig.generation(FIRST),
            fault: MixedSourceFault::PreRing(InputError::ProtectedCapacity),
        })
    );
}

fn note_on(raw: u8) -> Midi1Input {
    Midi1Input::from_bytes([0x90, raw, 100]).expect("note-on")
}

fn note_off(raw: u8) -> Midi1Input {
    Midi1Input::from_bytes([0x80, raw, 0]).expect("note-off")
}

fn bend(value: u16) -> Midi1Input {
    let bytes = [0xe0, (value & 0x7f) as u8, (value >> 7) as u8];
    Midi1Input::from_bytes(bytes).expect("pitch bend")
}

fn sustain(down: bool) -> Midi1Input {
    channel_sustain(0, down)
}

/// A note-on on the zero-based MIDI channel `channel`.
fn channel_note_on(channel: u8, raw: u8) -> Midi1Input {
    Midi1Input::from_bytes([0x90 + channel, raw, 100]).expect("note-on")
}

fn channel_note_off(channel: u8, raw: u8) -> Midi1Input {
    Midi1Input::from_bytes([0x80 + channel, raw, 0]).expect("note-off")
}

fn channel_sustain(channel: u8, down: bool) -> Midi1Input {
    Midi1Input::from_bytes([0xb0 + channel, 64, if down { 127 } else { 0 }]).expect("sustain")
}

fn message(frames: u64, input: Midi1Input) -> InputObservation {
    InputObservation::Message {
        tick: InputTick::new(frames),
        arrival: at(frames),
        input,
    }
}

fn frontier(frames: u64) -> InputObservation {
    InputObservation::Frontier {
        tick: InputTick::new(frames),
    }
}

/// How one source message handed to the bridge was settled, or where it was at teardown.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Settled {
    /// The stage call's own return.
    Stage(Result<MixedStaged, MixedBridgeError>),
    /// A pump split.
    Pump(MixedSplit),
    /// A collected report.
    Report(MixedBridgeReport),
    /// Still held by the bridge at joined teardown.
    Held(MixedSourceCustody),
}

/// What raw capture did with one source packet the host handed on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RawAdmission {
    /// Raw capture admitted it with this ID and admission-time `matched_onset` link.
    Admitted {
        id: InputEventId,
        link: Option<InputEventId>,
    },
    /// Raw capture had already interrupted itself, so the host offered it nothing.
    Interrupted,
}

/// One source packet the host handed on, message or frontier, with its two identities
/// kept apart: the source ring's, which sound uses, and raw capture's outcome.
#[derive(Clone, Copy, Debug)]
struct Admitted {
    source: usize,
    packet: MixedSourcePacketId,
    observation: InputObservation,
    raw: RawAdmission,
}

impl Admitted {
    const fn is_message(&self) -> bool {
        matches!(self.observation, InputObservation::Message { .. })
    }

    const fn raw_id(&self) -> Option<InputEventId> {
        match self.raw {
            RawAdmission::Admitted { id, .. } => Some(id),
            RawAdmission::Interrupted => None,
        }
    }

    fn at(&self, source: usize, frames: u64) -> bool {
        self.source == source
            && matches!(self.observation, InputObservation::Message { tick, .. } if tick == InputTick::new(frames))
    }
}

/// The source ID of `source`'s message stamped `frames`.
fn packet_in(admitted: &[Admitted], source: usize, frames: u64) -> MixedSourcePacketId {
    admitted
        .iter()
        .find(|admitted| admitted.at(source, frames))
        .map(|admitted| admitted.packet)
        .unwrap_or_else(|| panic!("source {source} handed on a message at {frames}"))
}

fn admitted_in(admitted: &[Admitted], packet: MixedSourcePacketId) -> Admitted {
    admitted
        .iter()
        .find(|admitted| admitted.packet == packet)
        .copied()
        .unwrap_or_else(|| panic!("packet {packet:?} was never handed on"))
}

fn report_in(
    settled: &[(MixedSourcePacketId, Settled)],
    packet: MixedSourcePacketId,
) -> Option<MixedBridgeReport> {
    settled.iter().find_map(|(id, settled)| match settled {
        Settled::Report(report) if *id == packet => Some(*report),
        _ => None,
    })
}

/// What the raw owner finally did with one source packet, from its `InputReceipt`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RawState {
    /// The recorder published the message with this capture disposition.
    Published(CaptureDisposition),
    /// The recorder applied the frontier.
    Fenced,
    /// The recorder refused it.
    Refused,
    /// It was never delivered: teardown or a halt cancelled it.
    Cancelled,
    /// Raw capture had interrupted itself before the packet reached it.
    NotOffered,
}

/// One collected raw receipt, reduced to the fields correlation needs.
#[derive(Debug)]
struct RawReceipt {
    id: InputEventId,
    matched_onset: Option<InputEventId>,
    observation: InputObservation,
    state: RawState,
    /// Keep the recorder's exact outcome, including its refusal reason. The compact
    /// state above is only for the cross-consumer custody ledger.
    outcome: InputOutcome,
}

use synth_engine_v2::host::mixed_input::{
    MixedInputRefusal, MixedRawHaltClass as RawHaltClass, MixedRawPolicy, MixedRawPublisher,
    MixedRawSendRefusal, MixedRoutedSound,
};

#[test]
fn terminal_raw_outcome_dominates_late_in_either_completion_order() {
    assert_eq!(
        raw_halt_class(&InputOutcome::Delivered(SessionSourceOutcome::Refused(
            NoteCaptureError::PastBoundary,
        ))),
        Some(RawHaltClass::Late)
    );
    assert_eq!(
        raw_halt_class(&InputOutcome::Delivered(SessionSourceOutcome::Refused(
            NoteCaptureError::TrackerFull,
        ))),
        Some(RawHaltClass::Terminal)
    );
    assert_eq!(
        RawHaltClass::Late.combine(RawHaltClass::Terminal),
        RawHaltClass::Terminal
    );
    assert_eq!(
        RawHaltClass::Terminal.combine(RawHaltClass::Late),
        RawHaltClass::Terminal
    );
}

fn raw_halt_class(outcome: &InputOutcome) -> Option<RawHaltClass> {
    synth_engine_v2::host::mixed_input::classify_raw_outcome(outcome)
}

impl From<InputReceipt> for RawReceipt {
    fn from(receipt: InputReceipt) -> Self {
        let state = match &receipt.outcome {
            InputOutcome::Delivered(SessionSourceOutcome::Published(published)) => {
                RawState::Published(published.capture)
            }
            InputOutcome::Delivered(SessionSourceOutcome::Fenced) => RawState::Fenced,
            InputOutcome::Delivered(SessionSourceOutcome::Refused(_))
            | InputOutcome::Refused(_) => RawState::Refused,
            InputOutcome::Delivered(SessionSourceOutcome::Cancelled) | InputOutcome::Cancelled => {
                RawState::Cancelled
            }
        };
        Self {
            id: receipt.id,
            matched_onset: receipt.matched_onset,
            observation: receipt.observation,
            state,
            outcome: receipt.outcome,
        }
    }
}

/// The audio thread's whole callback: the raw owner's audio half and its rings beside the
/// mixed audio half and its wrapper. Both render every callback over the same frames.
struct HostCallback {
    maximum_block: FrameCount,
    mixed_layout: ChannelLayout,
    mixed: Option<MixedHostAudio>,
    raw: InputCaptureAudio,
    raw_halt: InputCaptureHalt,
    /// The sources' halt, which raw capture never requests.
    host_halt: SourceHostHalt,
    /// A raw render or packet-preparation fault has no reliable receipt classifier.
    raw_aux_terminal: Arc<AtomicBool>,
    /// The first exact raw render error, retained with the callback until join.
    raw_terminal_error: Option<LoopSessionError>,
    raw_command_peak: QueuePeak,
    raw_action_peak: QueuePeak,
    raw_usage_error: Option<synth_engine_v2::render::RehearsalUsageError>,
    packets: HeapCons<LoopTransferPacket>,
    completions: ObservedProducer<LoopTransferCompletion>,
    /// A packet the raw audio half refused; FIFO stops behind it until teardown.
    refused: Option<LoopTransferPacket>,
    /// A completion the full completion ring refused; it goes first next time.
    failed: Option<LoopTransferCompletion>,
    /// The raw owner's own output: its graph is silent and the host does not mix it.
    scratch: Vec<f32>,
}

enum MixedHostAudio {
    Direct(Box<MixedTransferAudio>, AudioPort),
    Reset(
        Box<
            MixedBridgeResetAudio<
                HostConsumer<MixedTransferCommand>,
                HostProducer<MixedTransferResult>,
            >,
        >,
    ),
}

impl MixedHostAudio {
    fn render(&mut self, output: AudioBlockMut<'_>) -> Result<(), MixedHostRenderError> {
        match self {
            Self::Direct(audio, port) => audio
                .render(port, output)
                .map_err(MixedHostRenderError::Direct),
            Self::Reset(wrapper) => wrapper.render(output).map_err(MixedHostRenderError::Reset),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MixedHostRenderError {
    Direct(MixedTransferRenderError),
    Reset(MixedBridgeResetRenderError),
    Host(MixedTransferRenderError),
}

impl MixedHostRenderError {
    fn is_output_shape(self) -> bool {
        matches!(
            self,
            Self::Direct(MixedTransferRenderError::OutputShape)
                | Self::Reset(MixedBridgeResetRenderError::OutputShape)
                | Self::Host(MixedTransferRenderError::OutputShape)
        )
    }

    /// Older direct-owner fault fixtures compare transfer-shaped host failures.
    fn transfer_view(self) -> MixedTransferRenderError {
        match self {
            Self::Direct(error) | Self::Host(error) => error,
            Self::Reset(_) => panic!("the direct-owner fixture received a reset error"),
        }
    }
}

/// One host callback's two results.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rendered {
    mixed: Result<(), MixedHostRenderError>,
    raw_ok: bool,
}

impl HostCallback {
    fn record_raw_render_error(&mut self, error: LoopSessionError, samples: &mut [f32]) -> bool {
        if matches!(error, LoopSessionError::Session(SessionError::Closed)) {
            return false;
        }
        if self.raw_terminal_error.is_none() {
            self.raw_terminal_error = Some(error);
        }
        self.raw_aux_terminal.store(true, Ordering::Release);
        self.host_halt.request();
        self.raw_halt.request_invalid();
        samples.fill(0.0);
        true
    }

    fn admit_raw(&mut self) {
        while self.refused.is_none() {
            let Some(packet) = self.packets.try_pop() else {
                return;
            };
            if let Err((packet, _)) = self.raw.enqueue(packet) {
                self.refused = Some(packet);
            }
            match self.raw.rehearsal_queue_occupancy() {
                Ok((commands, actions)) => {
                    self.raw_command_peak.observe(commands.get() as usize);
                    self.raw_action_peak.observe(actions.get() as usize);
                }
                Err(error) => {
                    self.raw_usage_error.get_or_insert(error);
                }
            }
        }
    }

    fn flush_raw(&mut self) {
        if let Some(completion) = self.failed.take()
            && let Err(completion) = self.completions.try_push(completion)
        {
            self.failed = Some(completion);
            return;
        }
        while !self.completions.is_full() {
            let Some(completion) = self.raw.take_completed() else {
                return;
            };
            if let Err(completion) = self.completions.try_push(completion) {
                self.failed = Some(completion);
                return;
            }
        }
    }

    /// Make raw handoff progress while the simulated clock grants no callback.
    /// Completed packets can outlive a full completion ring; draining that ring on
    /// control must not require another render to publish the remaining completions.
    fn service_between_callbacks(&mut self) {
        self.admit_raw();
        self.flush_raw();
    }

    /// Render the mixed half into `samples`, then the raw owner over the same frames. A
    /// terminal mixed callback halts the whole host first, sources and then raw, so the
    /// raw owner observes it in this same callback. A raw render fault silences this
    /// complete callback and publishes the host halt before the call returns.
    fn render(&mut self, samples: &mut [f32]) -> Rendered {
        if self.host_halt.is_requested() {
            samples.fill(0.0);
            return Rendered {
                mixed: Err(MixedHostRenderError::Host(
                    MixedTransferRenderError::RemoteHalt,
                )),
                raw_ok: false,
            };
        }
        let frames = samples.len() / self.mixed_layout.channels();
        self.admit_raw();
        let Ok(output) = AudioBlockMut::new(samples, frames, self.mixed_layout) else {
            samples.fill(0.0);
            return Rendered {
                mixed: Err(MixedHostRenderError::Host(
                    MixedTransferRenderError::OutputShape,
                )),
                raw_ok: false,
            };
        };
        let mixed = self.mixed.as_mut().expect("mixed owner").render(output);
        if mixed.is_err_and(|error| !error.is_output_shape()) {
            self.host_halt.request();
            self.raw_halt.request_invalid();
        }
        let Some(scratch) = self.scratch.get_mut(..frames) else {
            samples.fill(0.0);
            let terminal = match self.raw.synchronize_halt() {
                Ok(()) => false,
                Err(error) => self.record_raw_render_error(error, samples),
            };
            self.flush_raw();
            return Rendered {
                mixed: if terminal {
                    Err(MixedHostRenderError::Host(
                        MixedTransferRenderError::RemoteHalt,
                    ))
                } else {
                    mixed.and(Err(MixedHostRenderError::Host(
                        MixedTransferRenderError::OutputShape,
                    )))
                },
                raw_ok: !terminal && mixed.is_ok(),
            };
        };
        let raw_ok = if let Ok(raw) = AudioBlockMut::new(scratch, frames, ChannelLayout::Mono) {
            match self.raw.render(raw) {
                Ok(()) => true,
                Err(error) => {
                    if self.record_raw_render_error(error, samples) {
                        self.flush_raw();
                        return Rendered {
                            mixed: Err(MixedHostRenderError::Host(
                                MixedTransferRenderError::RemoteHalt,
                            )),
                            raw_ok: false,
                        };
                    }
                    false
                }
            }
        } else {
            self.host_halt.request();
            self.raw_halt.request_invalid();
            samples.fill(0.0);
            false
        };
        self.flush_raw();
        Rendered { mixed, raw_ok }
    }
}

struct Rig {
    mixed_layout: ChannelLayout,
    raw: RawOwner,
    producers: [Option<SourceProducer>; 2],
    inboxes: [SourceInbox; 2],
    retries: [Option<SourceRetryToken>; 2],
    control: MixedBridgeControl,
    /// Both audio halves; a threaded test moves the callback to its audio thread.
    callback: Option<HostCallback>,
    control_port: ControlPort,
    /// The raw packet ring's control end and a packet a full ring refused.
    raw_packets: ObservedProducer<LoopTransferPacket>,
    raw_unsent: Option<LoopTransferPacket>,
    raw_publisher: MixedRawPublisher,
    /// Optional control-side custody trace, recorded only after a successful ring push.
    /// Prefix tests use it to distinguish raw admission from delivery to the callback.
    raw_queued_inputs: Option<Vec<InputEventId>>,
    raw_completions: HeapCons<LoopTransferCompletion>,
    raw_packet_peak: QueuePeak,
    raw_completion_peak: QueuePeak,
    raw_command_peak: QueuePeak,
    raw_action_peak: QueuePeak,
    source_peaks: [MixedTransferPlaces; 2],
    stage_peaks: [CellPeaks; 2],
    raw_input_peaks: [CellPeaks; 2],
    callback_observations: Option<Vec<callback_probe::Observation>>,
    /// While set, step 2b sends raw nothing: a stalled raw path whose packets reach raw
    /// audio only after it rendered their stamps. Sound never reads it.
    raw_withheld: bool,
    /// A collected late recorder outcome is the only raw self-halt this first host
    /// shape treats as raw-only. Other causes must stop the whole host.
    raw_policy: MixedRawPolicy,
    /// Raw errors without a trustworthy receipt classification, published by either
    /// the callback or the control packet-preparation path.
    raw_aux_terminal: Arc<AtomicBool>,
    raw_aux_reported: bool,
    raw_packet_errors: Vec<InputCaptureError>,
    /// The sources' halt. Only a host-wide terminal fault requests it, always before the
    /// raw owner's halt. A raw halt without it awaits classification from its receipts.
    host_halt: SourceHostHalt,
    /// Every source packet the host handed on, messages and frontiers, in ring order.
    admitted: Vec<Admitted>,
    /// Every frontier handed to the bridge and its answer.
    frontiers: Vec<(MixedSourcePacketId, Result<(), MixedBridgeError>)>,
    /// Raw refusals while raw capture ran: the source, the original and the reason.
    raw_refused: Vec<(usize, InputObservation, InputError)>,
    /// Pre-ring producer failures the host reported.
    pre_ring: Vec<(usize, InputObservation, InputError)>,
    /// The raw owner's answer to each reported pre-ring failure.
    pre_ring_recorded: Vec<Result<(), InputError>>,
    /// Observations a source producer refused because the host had halted.
    halted_sends: Vec<(usize, InputObservation)>,
    /// Raw receipts collected while the host ran.
    receipts: Vec<RawReceipt>,
    /// Raw control errors: packet preparation and completion collection.
    raw_errors: Vec<String>,
    /// Command outcomes are distinct from source receipts and retain their raw IDs.
    raw_command_outcomes: Vec<(LoopTransferId, LoopTransferOutcome)>,
    raw_unexpected_command: bool,
    raw_join_in_progress: bool,
    raw_play_id: LoopTransferId,
    /// A collection refusal returns the exact completion; it remains owned here.
    raw_collect_refusals: Vec<Box<(LoopTransferCompletion, LoopTransferError)>>,
    raw_cancel_refusals: Vec<(LoopTransferPacket, LoopTransferError)>,
    raw_receipt_errors: Vec<(ConnectionGeneration, InputError)>,
    settled: Vec<(MixedSourcePacketId, Settled)>,
    /// Collected host Panic reports, which name no source packet.
    panics: Vec<MixedBridgeReport>,
    forwarded: Vec<MixedPumpStep>,
    errors: Vec<MixedBridgeError>,
    waits: Vec<MixedBridgeWait>,
    rendered: Vec<f32>,
    callbacks: Vec<(usize, usize, Result<(), MixedHostRenderError>)>,
    raw_callbacks: Vec<bool>,
    /// Whether source packets reach the bridge; see [`Setup::bridge`].
    bridge: bool,
    /// Every committed reset's retired control and old port, oldest first, until its
    /// owner retires from the wrapper and joins.
    reset_old: Vec<(MixedBridgeRetiredControl, ControlPort)>,
    /// Retired owners joined while the host ran, oldest first.
    retired_resets: Vec<MixedBridgeRetiredTeardown>,
}

struct Setup {
    cells: usize,
    peer_wait: u64,
    commands: Commands,
    results: Results,
    raw_cells: u32,
    raw_ring: MixedTransferPlaces,
    raw_completion_ring: MixedTransferPlaces,
    raw_commands: SessionCommandCapacity,
    raw_actions: SessionSourceCapacity,
    raw_action_bytes: PreparedBytes,
    /// The held onsets per source the bridge is told about; `None` declares the source
    /// ring's own bound.
    declared_holds: Option<usize>,
    /// Whether the host hands source packets to the bridge at all. Without it the host
    /// is a raw-only reference: the same rings, raw owner and callbacks, no sound path.
    bridge: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            cells: 8,
            peer_wait: 4_096,
            commands: HostQueue::new(places()),
            results: HostQueue::new(places()),
            raw_cells: 136,
            raw_ring: MixedTransferPlaces::new(RAW_RING),
            raw_completion_ring: MixedTransferPlaces::new(RAW_RING),
            raw_commands: SessionCommandCapacity::new(4).expect("commands"),
            raw_actions: SessionSourceCapacity::new(256).expect("actions"),
            raw_action_bytes: PreparedBytes::measured(262_144),
            declared_holds: None,
            bridge: true,
        }
    }
}

/// Every onset a source ring accepts reserves its release place, up to this bound.
fn source_holds() -> MixedTransferPlaces {
    MixedTransferPlaces::new(source_custody::MAX_RESERVED_NOTES)
}

fn arm_at(
    raw: &RawOwner,
    setup: Setup,
    requested: SampleTime,
) -> MixedBridgeArmed<Commands, Results> {
    let (prepared, candidate) = prepared_at(requested);
    arm_prepared(raw, setup, prepared, candidate, profile())
}

fn arm_prepared(
    raw: &RawOwner,
    setup: Setup,
    prepared: MixedJoinedPrepared,
    candidate: MixedStampedCandidate,
    sound_profile: HostProfile,
) -> MixedBridgeArmed<Commands, Results> {
    let held_onsets = setup
        .declared_holds
        .map_or_else(source_holds, MixedTransferPlaces::new);
    raw.queue_owners.borrow_mut().push(OwnerQueuePeaks {
        commands: setup.commands.peak.clone(),
        results: setup.results.peak.clone(),
    });
    prepared
        .arm_bridge_rehearsal(
            candidate,
            &sound_profile,
            MixedTransferSources::new(raw.generations[FIRST], raw.generations[SECOND])
                .expect("distinct sources"),
            MixedTransferFence {
                stage_cells: MixedTransferPlaces::new(setup.cells),
                held_onsets,
                peer_wait: FrameCount::new(setup.peer_wait),
            },
            setup.commands,
            setup.results,
        )
        .expect("bridge arms")
}

/// Places in each raw packet and completion ring.
const RAW_RING: usize = 64;

impl Rig {
    fn new(setup: Setup) -> Self {
        let (prepared, candidate) = prepared();
        Self::with_prepared(setup, prepared, candidate, profile())
    }

    fn with_prepared(
        setup: Setup,
        prepared: MixedJoinedPrepared,
        candidate: MixedStampedCandidate,
        sound_profile: HostProfile,
    ) -> Self {
        let mut raw = raw_owner_with_budgets(
            setup.raw_cells,
            sound_profile.capabilities().sample_rate(),
            sound_profile.capabilities().maximum_block_size(),
            setup.raw_commands,
            setup.raw_actions,
            setup.raw_action_bytes,
        );
        let raw_ring = setup.raw_ring.get();
        let raw_completion_ring = setup.raw_completion_ring.get();
        let bridge = setup.bridge;
        let MixedBridgeArmed {
            control,
            audio,
            control_port,
            audio_port,
        } = arm_prepared(&raw, setup, prepared, candidate, sound_profile);
        let mixed_layout = sound_profile.capabilities().channel_layout();
        let host_halt = SourceHostHalt::new();
        let (first, first_inbox) =
            SourceInbox::prepare(raw.generations[FIRST], host_halt.clone(), raw.clocks[FIRST]);
        let (second, second_inbox) = SourceInbox::prepare(
            raw.generations[SECOND],
            host_halt.clone(),
            raw.clocks[SECOND],
        );
        let raw_packet_peak = QueuePeak::new();
        let raw_completion_peak = QueuePeak::new();
        let raw_command_peak = QueuePeak::new();
        let raw_action_peak = QueuePeak::new();
        let (raw_packets, packets) = HeapRb::new(raw_ring).split();
        let (completions, raw_completions) = HeapRb::new(raw_completion_ring).split();
        let mut raw_packets = ObservedProducer {
            ring: raw_packets,
            peak: raw_packet_peak.clone(),
        };
        let completions = ObservedProducer {
            ring: completions,
            peak: raw_completion_peak.clone(),
        };
        let raw_aux_terminal = Arc::new(AtomicBool::new(false));
        // The raw recorder plays from the first sample, as the mixed half does.
        let play = raw
            .control
            .prepare_command(SampleTime::ZERO, SessionCommand::Play)
            .expect("raw play");
        let raw_play_id = play.id();
        assert!(raw_packets.try_push(play).is_ok(), "an empty ring");
        let callback = HostCallback {
            maximum_block: sound_profile.capabilities().maximum_block_size(),
            mixed_layout,
            mixed: Some(MixedHostAudio::Direct(Box::new(audio), audio_port)),
            raw: raw.audio.take().expect("raw audio half"),
            raw_halt: raw.halt.clone(),
            host_halt: host_halt.clone(),
            raw_aux_terminal: Arc::clone(&raw_aux_terminal),
            raw_terminal_error: None,
            raw_command_peak: raw_command_peak.clone(),
            raw_action_peak: raw_action_peak.clone(),
            raw_usage_error: None,
            packets,
            completions,
            refused: None,
            failed: None,
            scratch: vec![0.0; BLOCK as usize],
        };
        let raw_publisher =
            MixedRawPublisher::new(&raw.control, control.sources()).expect("bound raw publisher");
        let raw_policy =
            MixedRawPolicy::new(&raw.control, control.sources()).expect("bound raw classification");
        let mut rig = Self {
            mixed_layout,
            raw,
            producers: [Some(first), Some(second)],
            inboxes: [first_inbox, second_inbox],
            retries: [None, None],
            control,
            callback: Some(callback),
            control_port,
            raw_packets,
            raw_unsent: None,
            raw_publisher,
            raw_queued_inputs: None,
            raw_completions,
            raw_packet_peak,
            raw_completion_peak,
            raw_command_peak,
            raw_action_peak,
            source_peaks: [MixedTransferPlaces::NONE; 2],
            stage_peaks: [CellPeaks::empty(); 2],
            raw_input_peaks: [CellPeaks::empty(); 2],
            callback_observations: None,
            raw_withheld: false,
            raw_policy,
            raw_aux_terminal,
            raw_aux_reported: false,
            raw_packet_errors: Vec::new(),
            host_halt,
            admitted: Vec::new(),
            frontiers: Vec::new(),
            raw_refused: Vec::new(),
            pre_ring: Vec::new(),
            pre_ring_recorded: Vec::new(),
            halted_sends: Vec::new(),
            receipts: Vec::new(),
            raw_errors: Vec::new(),
            raw_command_outcomes: Vec::new(),
            raw_unexpected_command: false,
            raw_join_in_progress: false,
            raw_play_id,
            raw_collect_refusals: Vec::new(),
            raw_cancel_refusals: Vec::new(),
            raw_receipt_errors: Vec::new(),
            settled: Vec::new(),
            panics: Vec::new(),
            forwarded: Vec::new(),
            errors: Vec::new(),
            waits: Vec::new(),
            rendered: Vec::new(),
            callbacks: Vec::new(),
            raw_callbacks: Vec::new(),
            bridge,
            reset_old: Vec::new(),
            retired_resets: Vec::new(),
        };
        // Each input's start fence must reach raw audio before the first callback.
        rig.forward_raw();
        rig.observe_raw_inputs();
        rig
    }

    fn generation(&self, source: usize) -> ConnectionGeneration {
        self.raw.generations[source]
    }

    fn producer(&mut self, source: usize) -> &mut SourceProducer {
        self.producers[source].as_mut().expect("producer present")
    }

    /// Source thread `source` publishes one observation. A full ring keeps the owned
    /// token for a retry; a halted host keeps the observation for teardown; any other
    /// refusal is a test failure.
    fn send(&mut self, source: usize, observation: InputObservation) {
        assert!(
            self.retries[source].is_none(),
            "retry the owned token first"
        );
        match self.producer(source).submit_owned(observation) {
            Ok(_) => {}
            Err(SourceOwnedSendError::Retry(token)) => self.retries[source] = Some(token),
            Err(SourceOwnedSendError::Refused(source_custody::SourceSendError::Halted(
                observation,
            ))) => self.halted_sends.push((source, observation)),
            Err(other) => panic!("source {source} refused {observation:?}: {other}"),
        }
        self.source_peaks[source] = MixedTransferPlaces::new(
            self.source_peaks[source]
                .get()
                .max(self.inboxes[source].occupied_len()),
        );
    }

    fn send_all(&mut self, source: usize, observations: &[InputObservation]) {
        for &observation in observations {
            self.send(source, observation);
        }
    }

    /// Only a collected late recorder outcome may interrupt raw capture alone.
    fn raw_interrupted(&self) -> bool {
        self.raw.halt.is_requested()
            && !self.host_halt.is_requested()
            && self
                .raw_policy
                .decision()
                .is_some_and(|decision| decision == RawHaltClass::Late)
    }

    /// A host-wide terminal fault: stop both sources, then raw capture.
    fn halt_host(&self) {
        self.host_halt.request();
        self.raw.halt.request_invalid();
    }

    /// Reconcile a raw halt after collecting all currently available receipts, and
    /// before offering another source packet. Unknown causes are source-neutral.
    fn reconcile_raw_halt(&mut self) {
        if self.raw_aux_terminal.load(Ordering::Acquire) {
            self.raw_policy.terminal_failure();
            self.halt_host();
            if !self.raw_aux_reported {
                self.raw_aux_reported = true;
                if self.bridge {
                    self.errors.push(self.control.report_raw_self_halt());
                }
            }
            return;
        }
        if self.raw.halt.is_requested() && !self.host_halt.is_requested() && !self.raw_interrupted()
        {
            self.raw_policy.terminal_failure();
            self.halt_host();
            if self.bridge {
                self.errors.push(self.control.report_raw_self_halt());
            }
        }
    }

    /// Steps 1 and 2 of the control loop for both sources: resolve producer failures,
    /// then offer every ring head to raw while raw capture runs and hand the packet to
    /// the bridge under its source ID. Once raw capture has interrupted itself, the
    /// host offers raw nothing for either source and still hands every packet on. Step
    /// 2b then sends every raw packet the raw owner can prepare.
    fn stage_all(&mut self) {
        // The previous callback may have requested raw halt. Settle its exact result
        // before deciding whether the halt is the one allowed raw-only cause.
        self.collect_raw();
        self.reconcile_raw_halt();
        for source in [FIRST, SECOND] {
            let generation = self.generation(source);
            if let Some((observation, reason)) = self.inboxes[source].unresolved_failure() {
                let recorded =
                    self.raw
                        .control
                        .record_pre_ring_failure(generation, observation, reason);
                self.pre_ring_recorded.push(recorded);
                self.inboxes[source].resolve_failure();
                self.halt_host();
                let fault = if reason == InputError::SourceQueueFull {
                    MixedSourceFault::RetryExpired
                } else {
                    MixedSourceFault::PreRing(reason)
                };
                self.pre_ring.push((source, observation, reason));
                if self.bridge {
                    self.errors
                        .push(self.control.report_source_fault(generation, fault));
                }
            }
            while let Some(head) = self.inboxes[source].peek_handoff() {
                if !self.hand_on(source, head) {
                    break;
                }
                let committed = self.inboxes[source].commit_handoff(head);
                assert_eq!(committed, Ok(head), "the sole consumer commits its head");
            }
        }
        self.forward_raw();
    }

    /// One ring head: raw first while it runs, then the bridge. A raw refusal while raw
    /// capture runs is terminal for the host, and the packet goes nowhere else.
    fn hand_on(&mut self, source: usize, head: SourceHandoff) -> bool {
        self.reconcile_raw_halt();
        if self.host_halt.is_requested() {
            return false;
        }
        let generation = self.generation(source);
        let observation = head.observation();
        let queue = head.stamp().queue();
        let packet = MixedSourcePacketId::new(queue.generation(), queue.serial())
            .expect("the source ring issues nonzero serials");
        if self.bridge {
            return self.hand_on_mixed(source, packet, observation);
        }
        let raw = if self.raw_interrupted() {
            RawAdmission::Interrupted
        } else {
            match self.raw.control.offer_observation(generation, observation) {
                Ok(id) => RawAdmission::Admitted {
                    id,
                    link: self
                        .raw
                        .control
                        .matched_onset(id)
                        .expect("admission-time link"),
                },
                // Raw capture interrupted itself between the check and the offer.
                Err((_, InputError::State)) if self.raw.halt.is_requested() => {
                    self.reconcile_raw_halt();
                    if self.host_halt.is_requested() {
                        return false;
                    }
                    RawAdmission::Interrupted
                }
                Err((original, error)) => {
                    self.raw_refused.push((source, original, error));
                    self.halt_host();
                    if self.bridge {
                        self.errors.push(self.control.report_source_fault(
                            generation,
                            MixedSourceFault::RawAdmission(error),
                        ));
                    }
                    return true;
                }
            }
        };
        self.observe_raw_inputs();
        self.admitted.push(Admitted {
            source,
            packet,
            observation,
            raw,
        });
        true
    }

    fn hand_on_mixed(
        &mut self,
        source: usize,
        packet: MixedSourcePacketId,
        observation: InputObservation,
    ) -> bool {
        let routed = self
            .control
            .route_raw_first(&mut self.raw.control, &self.raw_policy, packet, observation)
            .map_err(|refusal| *refusal);
        match routed {
            Ok(receipt) => {
                self.observe_raw_inputs();
                let raw = receipt.raw.map_or(RawAdmission::Interrupted, |admission| {
                    RawAdmission::Admitted {
                        id: admission.id(),
                        link: admission.matched_onset(),
                    }
                });
                self.admitted.push(Admitted {
                    source,
                    packet,
                    observation,
                    raw,
                });
                match receipt.sound {
                    MixedRoutedSound::Message(staged) => {
                        self.observe_stage();
                        if staged != Ok(MixedStaged::Staged) {
                            self.settled.push((packet, Settled::Stage(staged)));
                        }
                    }
                    MixedRoutedSound::Frontier(advanced) => self.frontiers.push((packet, advanced)),
                }
                true
            }
            Err(MixedInputRefusal::Raw {
                observation,
                error,
                fault,
            }) => {
                self.raw_policy.terminal_failure();
                self.raw_refused.push((source, observation, error));
                self.errors.push(fault);
                self.halt_host();
                true
            }
            Err(MixedInputRefusal::Halted { fault, .. }) => {
                self.raw_policy.terminal_failure();
                self.errors.push(fault);
                self.halt_host();
                false
            }
            Err(error @ MixedInputRefusal::Metadata { .. }) => {
                panic!("fresh raw admission metadata failed: {error}")
            }
        }
    }

    /// A preparation error has no reliable late classification from its receipt.
    fn record_raw_packet_error(&mut self, error: InputCaptureError) {
        self.raw_errors.push(format!("prepare: {error:?}"));
        self.raw_packet_errors.push(error);
        self.raw_aux_terminal.store(true, Ordering::Release);
        self.reconcile_raw_halt();
    }

    /// Step 2b: send the retained packet first, then every packet the raw owner prepares,
    /// until the ring is full or the raw owner has none eligible. A withheld raw path
    /// sends nothing.
    fn forward_raw(&mut self) {
        if self.raw_withheld {
            return;
        }
        loop {
            let packet = match self.raw_unsent.take() {
                Some(packet) => packet,
                None => match self.raw.control.next_packet() {
                    Ok(Some(packet)) => packet,
                    Ok(None) => return,
                    Err(error) => {
                        self.record_raw_packet_error(error);
                        return;
                    }
                },
            };
            let input = self
                .raw_queued_inputs
                .as_ref()
                .and_then(|_| self.raw.control.packet_input_id(&packet));
            match self
                .raw_publisher
                .send(&self.raw.control, &mut self.raw_packets, packet)
            {
                Ok(()) => {}
                Err(MixedRawSendRefusal::Full(packet)) => {
                    self.raw_unsent = Some(packet);
                    return;
                }
                Err(MixedRawSendRefusal::Invalid { packet, error }) => {
                    self.raw_unsent = Some(packet);
                    self.record_raw_packet_error(InputCaptureError::Input(error));
                    return;
                }
            }
            if let (Some(queued), Some(input)) = (&mut self.raw_queued_inputs, input) {
                queued.push(input);
            }
        }
    }

    fn settle_raw(&mut self, completion: LoopTransferCompletion) {
        match self.raw.control.collect(completion) {
            Ok(Some((id, outcome))) => {
                let prior_halt = self.host_halt.is_requested() || self.raw.halt.is_requested();
                let applied_play = id == self.raw_play_id
                    && matches!(
                        &outcome,
                        LoopTransferOutcome::Command(receipt)
                            if receipt.boundary.command == SessionCommand::Play
                                && matches!(receipt.outcome, SessionOutcome::Applied { .. })
                    );
                let cancelled_play = id == self.raw_play_id
                    && matches!(
                        &outcome,
                        LoopTransferOutcome::Command(receipt)
                            if receipt.boundary.command == SessionCommand::Play
                                && receipt.outcome == SessionOutcome::Cancelled
                    )
                    && (prior_halt || self.raw_join_in_progress);
                self.raw_command_outcomes.push((id, outcome));
                if !applied_play && !cancelled_play {
                    self.raw_unexpected_command = true;
                    self.raw_aux_terminal.store(true, Ordering::Release);
                    self.reconcile_raw_halt();
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.raw_errors.push(format!("collect: {:?}", error.1));
                self.raw_collect_refusals.push(error);
                self.raw_aux_terminal.store(true, Ordering::Release);
                self.reconcile_raw_halt();
            }
        }
    }

    /// Step 4b: collect raw completions, then every raw receipt now complete.
    fn collect_raw(&mut self) {
        while let Some(completion) = self.raw_completions.try_pop() {
            self.settle_raw(completion);
        }
        for source in [FIRST, SECOND] {
            let generation = self.generation(source);
            loop {
                let receipt = match self.raw.control.collect_input(generation) {
                    Ok(Some(receipt)) => receipt,
                    Ok(None) => break,
                    Err(error) => {
                        self.raw_receipt_errors.push((generation, error));
                        self.raw_aux_terminal.store(true, Ordering::Release);
                        break;
                    }
                };
                if raw_halt_class(&receipt.outcome).is_some() {
                    let known_admission = self
                        .admitted
                        .iter()
                        .any(|admitted| admitted.raw_id() == Some(receipt.id));
                    if known_admission {
                        self.raw_policy
                            .observe(receipt.id.generation(), &receipt.outcome)
                            .expect("correlated raw outcome");
                    } else {
                        self.raw_policy.terminal_failure();
                    }
                }
                self.receipts.push(receipt.into());
            }
        }
        self.reconcile_raw_halt();
    }

    /// Step 3: pump until the bridge is idle or faults.
    fn pump(&mut self) {
        if !self.bridge {
            return;
        }
        loop {
            let step = self.control.pump(&mut self.control_port);
            self.observe_stage();
            match step {
                Ok(MixedPumpStep::Idle(wait)) => {
                    self.waits.push(wait);
                    return;
                }
                Ok(MixedPumpStep::Split { packet, split }) => {
                    self.settled.push((packet, Settled::Pump(split)));
                }
                Ok(step) => self.forwarded.push(step),
                Err(error) => {
                    self.errors.push(error);
                    return;
                }
            }
        }
    }

    /// Step 4: collect every raw completion and receipt, then every bridge result.
    fn collect(&mut self) {
        self.collect_raw();
        if !self.bridge {
            return;
        }
        for (retired, port) in &mut self.reset_old {
            loop {
                match retired.collect(port) {
                    Ok(Some(report)) => self.settled.push((
                        report.packet().expect("a retired arm carries no Panic"),
                        Settled::Report(report),
                    )),
                    Ok(None) => break,
                    Err(error) => {
                        self.errors.push(error);
                        return;
                    }
                }
            }
        }
        loop {
            match self.control.collect(&mut self.control_port) {
                Ok(Some(report)) => match report.packet() {
                    Some(packet) => self.settled.push((packet, Settled::Report(report))),
                    None => self.panics.push(report),
                },
                Ok(None) => return,
                Err(error) => {
                    self.errors.push(error);
                    return;
                }
            }
        }
    }

    fn observe_stage(&mut self) {
        for source in [FIRST, SECOND] {
            let (entries, protected) = self
                .control
                .lane_charge(self.generation(source))
                .expect("bound lane");
            self.stage_peaks[source].observe(entries.get(), protected.get());
        }
    }

    fn observe_raw_inputs(&mut self) {
        for source in [FIRST, SECOND] {
            let pressure = self
                .raw
                .control
                .input(self.generation(source))
                .expect("bound raw input")
                .pressure();
            self.raw_input_peaks[source].observe(
                pressure.occupied().as_usize(),
                pressure.release_reservations().as_usize(),
            );
        }
    }

    fn service(&mut self) {
        self.stage_all();
        self.pump();
        self.collect();
    }

    fn render(&mut self, frames: usize) -> Result<(), MixedHostRenderError> {
        let mut samples = vec![0.25_f32; frames * self.mixed_layout.channels()];
        let callback = self.callback.as_mut().expect("callback present");
        let rendered = if let Some(observations) = self.callback_observations.as_mut() {
            assert!(
                observations.len() < observations.capacity(),
                "callback observations must be preallocated"
            );
            let (rendered, observation) = callback_probe::observe(|| callback.render(&mut samples));
            observations.push(observation);
            rendered
        } else {
            callback.render(&mut samples)
        };
        self.callbacks
            .push((self.output_frames(), frames, rendered.mixed));
        self.raw_callbacks.push(rendered.raw_ok);
        self.rendered.extend(samples);
        self.reconcile_raw_halt();
        rendered.mixed
    }

    fn output_frames(&self) -> usize {
        self.rendered.len() / self.mixed_layout.channels()
    }

    fn frame_samples(&self, range: std::ops::Range<usize>) -> &[f32] {
        let channels = self.mixed_layout.channels();
        &self.rendered[range.start * channels..range.end * channels]
    }

    fn render_all(&mut self, partition: &[usize]) {
        for &frames in partition {
            let _rendered = self.render(frames);
        }
    }

    fn report_of(&self, packet: MixedSourcePacketId) -> Option<MixedBridgeReport> {
        report_in(&self.settled, packet)
    }

    fn packet_of(&self, source: usize, frames: u64) -> MixedSourcePacketId {
        packet_in(&self.admitted, source, frames)
    }

    /// Live release holds the direct mixed owner's ingress store has outstanding.
    fn holds(&self) -> EventCount {
        match self
            .callback
            .as_ref()
            .expect("callback present")
            .mixed
            .as_ref()
            .expect("mixed owner present")
        {
            MixedHostAudio::Direct(audio, _) => audio.live_holds_outstanding(),
            MixedHostAudio::Reset(_) => panic!("a reset wrapper keeps its owners' stores"),
        }
    }

    /// Render the mixed half's first callback, which opens its output one quantum behind
    /// the engine clock and serves no command yet.
    fn prime(&mut self) {
        self.render(Q).expect("start raw capture and mixed plan");
        self.collect();
    }

    /// Stage, pump and collect, render one quantum so audio serves every forwarded
    /// command, then collect its results.
    fn step(&mut self) {
        self.service();
        self.render(Q).expect("callback");
        self.collect();
    }

    fn messages(&self) -> impl Iterator<Item = &Admitted> {
        self.admitted
            .iter()
            .filter(|admitted| admitted.is_message())
    }

    fn settled_of(&self, packet: MixedSourcePacketId) -> Vec<Settled> {
        self.settled
            .iter()
            .filter(|(id, _)| *id == packet)
            .map(|(_, settled)| *settled)
            .collect()
    }

    /// The documented join order: producers return and retire any owned retry, the
    /// callback stops, the raw owner settles and reunites, control joins, then each
    /// inbox is drained and closed. Returns the teardown, every raw receipt and every
    /// observation still in a ring or producer, which no raw owner saw.
    fn join(mut self) -> Joined {
        let mut retired = Vec::new();
        for source in [FIRST, SECOND] {
            if let Some(token) = self.retries[source].take() {
                retired.push(
                    self.producer(source)
                        .retire_owned(token)
                        .expect("matching token")
                        .observation(),
                );
            }
            if let Some(pending) = self.producer(source).retire_pending() {
                retired.push(pending.observation());
            }
        }
        let mut callback = self.callback.take().expect("callback returned");
        let halted = self.settle_raw_for_join(&mut callback);
        let host_halted = self.host_halt.is_requested();
        let HostCallback {
            mixed,
            raw: raw_audio,
            raw_terminal_error,
            raw_usage_error,
            ..
        } = callback;
        let queue_owners = self.raw.queue_owners.into_inner();
        let (raw_receipts, raw_join_error) =
            reunite_raw(raw_audio, self.raw.control, self.raw.generations);
        let (mixed, port) = match mixed.expect("mixed owner returned") {
            MixedHostAudio::Direct(mixed, port) => {
                assert!(self.reset_old.is_empty());
                (*mixed, port)
            }
            MixedHostAudio::Reset(mut wrapper) => {
                // Without a committed reset the active owner is still the old one, and the
                // test took any refused successor back itself. Only the latest committed
                // reset can still hold its old owner in the wrapper.
                if !self.reset_old.is_empty() {
                    let retired = wrapper.take_retired().expect("fade completed");
                    join_retired(&mut self.reset_old, &mut self.retired_resets, retired);
                }
                assert!(self.reset_old.is_empty(), "every retired arm joined");
                let (active, secondary) = (*wrapper).into_owners();
                assert!(secondary.is_none());
                active.into_parts()
            }
        };
        let teardown = self
            .control
            .join(mixed, Some(self.control_port), Some(port))
            .expect("bound wrappers join");
        let mut unexamined = Vec::new();
        for source in [FIRST, SECOND] {
            if let Some((_, reason)) = self.inboxes[source].unresolved_failure() {
                assert_eq!(reason, InputError::SourceQueueFull);
                self.inboxes[source].resolve_failure();
            }
            while let Some(head) = self.inboxes[source].peek_handoff() {
                unexamined.push((source, head.observation()));
                let committed = self.inboxes[source].commit_handoff(head);
                assert_eq!(committed, Ok(head), "drained head");
            }
            let producer = self.producers[source].take().expect("producer present");
            assert!(
                self.inboxes[source].close(producer).is_ok(),
                "an empty inbox with a resolved producer closes"
            );
        }
        for (packet, custody) in teardown.held() {
            self.settled.push((*packet, Settled::Held(*custody)));
        }
        let mut receipts = self.receipts;
        receipts.extend(raw_receipts);
        Joined {
            queue_owners,
            raw_packet_peak: self.raw_packet_peak,
            raw_completion_peak: self.raw_completion_peak,
            raw_command_peak: self.raw_command_peak,
            raw_action_peak: self.raw_action_peak,
            source_peaks: self.source_peaks,
            stage_peaks: self.stage_peaks,
            raw_input_peaks: self.raw_input_peaks,
            mixed_layout: self.mixed_layout,
            teardown,
            admitted: self.admitted,
            settled: self.settled,
            receipts,
            raw_halted: halted,
            host_halted,
            raw_errors: self.raw_errors,
            raw_command_outcomes: self.raw_command_outcomes,
            raw_unexpected_command: self.raw_unexpected_command,
            raw_play_id: self.raw_play_id,
            raw_collect_refusals: self.raw_collect_refusals,
            raw_cancel_refusals: self.raw_cancel_refusals,
            raw_receipt_errors: self.raw_receipt_errors,
            raw_join_error,
            raw_packet_errors: self.raw_packet_errors,
            raw_render_error: raw_terminal_error,
            raw_usage_error,
            errors: self.errors,
            unexamined,
            retired,
            halted_sends: self.halted_sends,
            rendered: self.rendered,
            bridge: self.bridge,
            retired_reset: self.retired_resets,
        }
    }

    /// Take the retired old owner back from the wrapper between callbacks, collect what
    /// its old port still holds and join it with its own retired control off-thread. The
    /// wrapper's secondary slot is then empty for the next reset.
    fn retire_reset(&mut self) {
        let retired = callback_probe::without_heap(|| self.reset_wrapper().take_retired())
            .expect("fade completed");
        self.collect();
        join_retired(&mut self.reset_old, &mut self.retired_resets, retired);
    }

    /// The raw owner's settlement after callback access ends: raw capture that has not
    /// halted stops, every completion is collected and every undelivered packet is
    /// cancelled. Returns whether raw capture had halted before join, by a host-wide
    /// fault or by interrupting itself.
    fn settle_raw_for_join(&mut self, callback: &mut HostCallback) -> bool {
        let halted = callback.raw_halt.is_requested();
        self.raw_join_in_progress = true;
        if !halted {
            self.raw.halt.request_stop();
        }
        if let Err(error) = callback.raw.synchronize_halt() {
            self.raw_errors.push(format!("halt: {error:?}"));
        }
        while let Some(completion) = self
            .raw_completions
            .try_pop()
            .or_else(|| callback.failed.take())
            .or_else(|| callback.raw.take_completed())
        {
            self.settle_raw(completion);
        }
        let mut unsent: Vec<_> = callback.refused.take().into_iter().collect();
        while let Some(packet) = callback.packets.try_pop() {
            unsent.push(packet);
        }
        unsent.extend(self.raw_unsent.take());
        for packet in unsent {
            if let Err((packet, error)) = self.raw.control.cancel(packet) {
                self.raw_errors.push(format!("cancel: {error:?}"));
                self.raw_cancel_refusals.push((packet, error));
            }
        }
        halted
    }
}

/// Join one retired owner with the retired control its reset's commit returned.
fn join_retired(
    reset_old: &mut Vec<(MixedBridgeRetiredControl, ControlPort)>,
    joined: &mut Vec<MixedBridgeRetiredTeardown>,
    retired: MixedBridgeResetRetired<
        HostConsumer<MixedTransferCommand>,
        HostProducer<MixedTransferResult>,
    >,
) {
    let position = reset_old
        .iter()
        .position(|(control, _)| control.receipt() == retired.receipt())
        .expect("the retired owner's reset was committed");
    let (control, old_port) = reset_old.remove(position);
    let (old_audio, old_audio_port) = retired.into_owner().into_parts();
    let teardown = control
        .join(old_audio, Some(old_port), Some(old_audio_port))
        .expect("retired owner joins");
    joined.push(teardown);
}

/// Owning failure from reunion or a later joined-owner operation.
enum RawJoinError {
    Reunion(Box<InputReuniteError>),
    Joined {
        owner: Box<InputCaptureSession>,
        error: InputCaptureError,
    },
}

/// Reunite the settled raw owner, acknowledge both sources quiescent, finalize the take
/// and collect every remaining receipt.
fn reunite_raw(
    audio: InputCaptureAudio,
    control: InputCaptureControl,
    generations: [ConnectionGeneration; 2],
) -> (Vec<RawReceipt>, Option<RawJoinError>) {
    let mut owner = match audio.reunite(control) {
        Ok(owner) => owner,
        Err(error) => return (Vec::new(), Some(RawJoinError::Reunion(error))),
    };
    for generation in generations {
        if let Err(error) = owner.acknowledge_input_quiescence(generation) {
            return (
                Vec::new(),
                Some(RawJoinError::Joined {
                    owner: Box::new(owner),
                    error,
                }),
            );
        }
    }
    if let Err(error) = owner.finalize() {
        return (
            Vec::new(),
            Some(RawJoinError::Joined {
                owner: Box::new(owner),
                error,
            }),
        );
    }
    let mut receipts = Vec::new();
    for generation in generations {
        loop {
            match owner.collect_input(generation) {
                Ok(Some(receipt)) => receipts.push(receipt.into()),
                Ok(None) => break,
                Err(error) => {
                    return (
                        receipts,
                        Some(RawJoinError::Joined {
                            owner: Box::new(owner),
                            error,
                        }),
                    );
                }
            }
        }
    }
    (receipts, None)
}

struct Joined {
    queue_owners: Vec<OwnerQueuePeaks>,
    raw_packet_peak: QueuePeak,
    raw_completion_peak: QueuePeak,
    raw_command_peak: QueuePeak,
    raw_action_peak: QueuePeak,
    source_peaks: [MixedTransferPlaces; 2],
    stage_peaks: [CellPeaks; 2],
    raw_input_peaks: [CellPeaks; 2],
    mixed_layout: ChannelLayout,
    teardown: MixedBridgeTeardown,
    admitted: Vec<Admitted>,
    settled: Vec<(MixedSourcePacketId, Settled)>,
    /// Every raw receipt: those collected while running, then those joined teardown
    /// collected.
    receipts: Vec<RawReceipt>,
    /// Whether raw capture had halted before join stopped it: a host-wide fault, or raw
    /// capture interrupting itself.
    raw_halted: bool,
    /// Whether a host-wide terminal fault halted the sources, including during join.
    host_halted: bool,
    raw_errors: Vec<String>,
    raw_command_outcomes: Vec<(LoopTransferId, LoopTransferOutcome)>,
    raw_unexpected_command: bool,
    raw_play_id: LoopTransferId,
    raw_collect_refusals: Vec<Box<(LoopTransferCompletion, LoopTransferError)>>,
    raw_cancel_refusals: Vec<(LoopTransferPacket, LoopTransferError)>,
    raw_receipt_errors: Vec<(ConnectionGeneration, InputError)>,
    raw_join_error: Option<RawJoinError>,
    raw_packet_errors: Vec<InputCaptureError>,
    raw_render_error: Option<LoopSessionError>,
    raw_usage_error: Option<synth_engine_v2::render::RehearsalUsageError>,
    errors: Vec<MixedBridgeError>,
    unexamined: Vec<(usize, InputObservation)>,
    retired: Vec<InputObservation>,
    halted_sends: Vec<(usize, InputObservation)>,
    rendered: Vec<f32>,
    bridge: bool,
    /// Every retired owner's joined teardown, oldest first.
    retired_reset: Vec<MixedBridgeRetiredTeardown>,
}

impl Joined {
    fn frame_samples(&self, range: std::ops::Range<usize>) -> &[f32] {
        let channels = self.mixed_layout.channels();
        &self.rendered[range.start * channels..range.end * channels]
    }

    /// Rule 1.10: every message handed to the bridge is in exactly one sound place, and
    /// every raw admission has exactly one raw receipt that names its own observation and
    /// its admission-time link. Where raw linked a key-up, sound paired it on its own
    /// with that same onset: raw admitted every earlier packet of the source, so both
    /// pairings saw the same sequence.
    fn assert_each_packet_once(&self) {
        assert!(
            self.raw_errors.is_empty(),
            "raw host errors: {:?}",
            self.raw_errors
        );
        assert!(self.raw_packet_errors.is_empty());
        assert!(self.raw_render_error.is_none());
        assert!(self.raw_usage_error.is_none());
        assert!(self.raw_collect_refusals.is_empty());
        assert!(self.raw_cancel_refusals.is_empty());
        assert!(self.raw_receipt_errors.is_empty());
        match &self.raw_join_error {
            None => {}
            Some(RawJoinError::Reunion(error)) => {
                panic!("raw reunion retained its owners: {:?}", error.error());
            }
            Some(RawJoinError::Joined { owner, error }) => {
                panic!(
                    "raw joined owner failed after {:?}: {error:?}",
                    owner.acknowledged()
                );
            }
        }
        assert!(!self.raw_unexpected_command);
        if !self.raw_halted && !self.host_halted && !self.rendered.is_empty() {
            assert!(matches!(
                self.raw_command_outcomes.as_slice(),
                [(id, LoopTransferOutcome::Command(receipt))]
                    if *id == self.raw_play_id
                        && receipt.boundary.command == SessionCommand::Play
                        && matches!(receipt.outcome, SessionOutcome::Applied { .. })
            ));
        }
        if self.bridge {
            for admitted in self
                .admitted
                .iter()
                .filter(|admitted| admitted.is_message())
            {
                let places: Vec<_> = self
                    .settled
                    .iter()
                    .filter(|(packet, _)| *packet == admitted.packet)
                    .collect();
                assert_eq!(
                    places.len(),
                    1,
                    "packet {:?} ({:?}) is in {places:?}",
                    admitted.packet,
                    admitted.observation
                );
            }
        }
        for (packet, _) in &self.settled {
            assert!(
                admitted_in(&self.admitted, *packet).is_message(),
                "settled packet {packet:?} was never handed on as a message"
            );
        }
        for (onset, _) in self.teardown.open() {
            assert!(onset.is_some(), "every merger handle names its onset");
        }
        for old in &self.retired_reset {
            assert!(
                old.unclassified().is_empty(),
                "old reset handles are classified"
            );
            assert!(
                old.unmatched().is_empty(),
                "reset receipt matches old handles"
            );
        }
        self.assert_sound_pairs_as_raw_links();
        for admitted in &self.admitted {
            let RawAdmission::Admitted { id, link } = admitted.raw else {
                continue;
            };
            let receipts: Vec<_> = self
                .receipts
                .iter()
                .filter(|receipt| receipt.id == id)
                .collect();
            assert_eq!(receipts.len(), 1, "raw {id:?} has {receipts:?}");
            assert_eq!(receipts[0].observation, admitted.observation);
            assert_eq!(
                receipts[0].matched_onset, link,
                "the receipt's link is the admission-time link"
            );
        }
        // Starting an input admits its start fence, which no ring carried.
        let start_fence = |receipt: &RawReceipt| {
            receipt.id.serial() == 1
                && receipt.observation
                    == InputObservation::Frontier {
                        tick: InputTick::new(0),
                    }
        };
        let mut starts = Vec::new();
        for receipt in &self.receipts {
            if self
                .admitted
                .iter()
                .any(|admitted| admitted.raw_id() == Some(receipt.id))
            {
                continue;
            }
            assert!(
                start_fence(receipt),
                "receipt {receipt:?} names no admission"
            );
            starts.push(receipt.id.generation());
        }
        let mut generations: Vec<_> = self
            .admitted
            .iter()
            .filter_map(|admitted| admitted.raw_id().map(InputEventId::generation))
            .collect();
        generations.sort_unstable_by_key(|generation| format!("{generation:?}"));
        generations.dedup();
        starts.sort_unstable_by_key(|generation| format!("{generation:?}"));
        assert!(
            generations
                .iter()
                .all(|generation| starts.contains(generation)),
            "every admitting input returned its start fence: {starts:?}"
        );
        assert_eq!(starts.len(), 2, "one start fence per input");
    }

    /// Each settled key-up's sound pairing, `None` when unpaired, equals raw's
    /// admission-time link for every key-up raw admitted.
    fn assert_sound_pairs_as_raw_links(&self) {
        for (packet, settled) in &self.settled {
            let paired = match settled {
                Settled::Report(report) => match report.outcome() {
                    MixedBridgeOutcome::KeyUp { onset, .. } => Some(onset),
                    _ => continue,
                },
                Settled::Stage(Ok(MixedStaged::Split(split))) | Settled::Pump(split) => match split
                {
                    MixedSplit::AfterRefusedOnset { onset, .. } => Some(*onset),
                    MixedSplit::Unpaired => None,
                    _ => continue,
                },
                _ => continue,
            };
            let RawAdmission::Admitted { link, .. } = admitted_in(&self.admitted, *packet).raw
            else {
                continue;
            };
            let raw_onset = paired.map(|onset| {
                admitted_in(&self.admitted, onset)
                    .raw_id()
                    .expect("raw admitted every packet before a raw-admitted key-up")
            });
            assert_eq!(link, raw_onset, "key-up {packet:?} pairs as raw links it");
        }
    }

    /// What raw capture did with `packet`: its receipt's state, or `NotOffered`.
    fn raw_state(&self, packet: MixedSourcePacketId) -> RawState {
        let Some(raw) = admitted_in(&self.admitted, packet).raw_id() else {
            return RawState::NotOffered;
        };
        self.receipts
            .iter()
            .find(|receipt| receipt.id == raw)
            .map(|receipt| receipt.state)
            .unwrap_or_else(|| panic!("raw {raw:?} has no receipt"))
    }

    fn sound(&self, packet: MixedSourcePacketId) -> Settled {
        self.settled
            .iter()
            .find(|(id, _)| *id == packet)
            .map(|(_, settled)| *settled)
            .unwrap_or_else(|| panic!("packet {packet:?} has no sound place"))
    }

    fn packet_of(&self, source: usize, frames: u64) -> MixedSourcePacketId {
        packet_in(&self.admitted, source, frames)
    }

    fn report_of(&self, packet: MixedSourcePacketId) -> Option<MixedBridgeReport> {
        report_in(&self.settled, packet)
    }

    /// Every message's two final states, raw and sound, in ring order.
    fn ledger(&self) -> Vec<(MixedSourcePacketId, RawState, Settled)> {
        self.admitted
            .iter()
            .filter(|admitted| admitted.is_message())
            .map(|admitted| {
                (
                    admitted.packet,
                    self.raw_state(admitted.packet),
                    self.sound(admitted.packet),
                )
            })
            .collect()
    }
}

/// Everything raw capture produced in one run, with raw IDs renamed `r<source>#<serial>`
/// so two runs compare.
fn raw_trace(admitted: &[Admitted], receipts: &[RawReceipt]) -> Vec<String> {
    // A start fence was never admitted through a ring; its input's other admissions
    // name its source.
    let name = |raw: InputEventId| {
        let admitted = admitted
            .iter()
            .find(|admitted| admitted.packet.connection() == raw.generation())
            .expect("an input with admissions");
        format!("r{}#{}", admitted.source, raw.serial())
    };
    let mut trace: Vec<_> = receipts
        .iter()
        .map(|receipt| {
            format!(
                "{} {:?} link {:?} {:?}",
                name(receipt.id),
                receipt.observation,
                receipt.matched_onset.map(name),
                receipt.state
            )
        })
        .collect();
    trace.sort();
    trace
}

fn silent(samples: &[f32]) -> bool {
    samples.iter().all(|sample| *sample == 0.0)
}

fn identity_of(report: MixedBridgeReport) -> NoteIdentity {
    match report.outcome() {
        MixedBridgeOutcome::NoteOn(Ok(identity)) => identity,
        other => panic!("not an accepted onset: {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------
// Smoke: two sources, real rings, raw links and both stamps.

#[test]
fn two_real_source_rings_feed_raw_and_the_bridge_into_settled_reports() {
    let mut rig = Rig::new(Setup::default());
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(300, bend(12_288)),
            message(400, note_off(60)),
            frontier(2_000),
        ],
    );
    rig.send_all(
        SECOND,
        &[
            message(150, note_on(64)),
            message(500, note_off(64)),
            frontier(2_000),
        ],
    );
    rig.service();
    assert_eq!(rig.forwarded.len(), 5, "{:?}", rig.waits);
    rig.render_all(&[1_024]);
    rig.collect();
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let on = rig
        .report_of(rig.packet_of(FIRST, 100))
        .expect("onset report");
    let peer = rig
        .report_of(rig.packet_of(SECOND, 150))
        .expect("peer onset");
    let up = rig
        .report_of(rig.packet_of(FIRST, 400))
        .expect("key-up report");
    assert_eq!(
        (on.nominal_at(), on.stamp(), on.executed_at()),
        (at(100), at(100), at(100))
    );
    assert!(on.id() < peer.id(), "stamp order across sources");
    assert_eq!(
        up.outcome(),
        MixedBridgeOutcome::KeyUp {
            onset: rig.packet_of(FIRST, 100),
            result: Ok(MixedKeyUpRetired::Released(identity_of(on))),
        }
    );
    assert!(!silent(&rig.rendered));
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(matches!(
        joined.raw_command_outcomes.as_slice(),
        [(id, LoopTransferOutcome::Command(receipt))]
            if *id == joined.raw_play_id
                && receipt.boundary.command == SessionCommand::Play
                && matches!(receipt.outcome, SessionOutcome::Applied { .. })
    ));
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert!(joined.teardown.held().is_empty());
}

#[test]
fn a_foreign_raw_completion_returns_with_its_owner_and_stops_the_host() {
    let mut donor = Rig::new(Setup::default());
    let mut completion = None;
    for _ in 0..8 {
        donor.render(64).expect("donor callback");
        if let Some(ready) = donor.raw_completions.try_pop() {
            completion = Some(ready);
            break;
        }
    }
    let completion = completion.expect("donor Play completion");
    let mut host = Rig::new(Setup::default());
    host.settle_raw(completion);
    assert!(host.host_halt.is_requested());
    let mut joined = host.join();
    assert_eq!(joined.errors.len(), 1);
    assert_eq!(joined.raw_collect_refusals.len(), 1);
    let (completion, reason) = *joined
        .raw_collect_refusals
        .pop()
        .expect("foreign completion retained");
    assert_eq!(reason, LoopTransferError::Origin);
    donor.settle_raw(completion);
    donor.collect_raw();
    assert!(donor.raw_collect_refusals.is_empty());
    donor.join().assert_each_packet_once();
}

#[test]
fn an_uncollected_raw_credit_returns_both_owners_for_recovery() {
    let mut rig = Rig::new(Setup::default());
    let mut completion = None;
    for _ in 0..8 {
        rig.render(64).expect("raw callback");
        if let Some(ready) = rig.raw_completions.try_pop() {
            completion = Some(ready);
            break;
        }
    }
    let completion = completion.expect("Play completion withheld");
    let joined = rig.join();
    let Some(RawJoinError::Reunion(error)) = joined.raw_join_error else {
        panic!("unresolved transfer must return its owners");
    };
    assert!(matches!(
        error.error(),
        InputCaptureError::Transfer(LoopTransferError::Outstanding)
    ));
    let (mut control, audio) = error.into_split().expect("split owners retained");
    assert!(matches!(control.collect(completion), Ok(Some(_))));
    assert!(
        audio.reunite(control).is_ok(),
        "the retained owners recover"
    );
}

#[test]
fn a_refused_raw_cancel_returns_the_packet_to_its_original_owner() {
    let mut donor = Rig::new(Setup::default());
    let packet = donor
        .callback
        .as_mut()
        .expect("donor callback")
        .packets
        .try_pop()
        .expect("donor Play packet");
    let mut host = Rig::new(Setup::default());
    host.raw_unsent = Some(packet);
    let mut joined = host.join();
    assert_eq!(joined.raw_cancel_refusals.len(), 1);
    let (packet, reason) = joined
        .raw_cancel_refusals
        .pop()
        .expect("foreign packet retained");
    assert_eq!(reason, LoopTransferError::Origin);
    assert!(donor.raw.control.cancel(packet).is_ok());
    let donor = donor.join();
    assert!(donor.raw_join_error.is_none());
}

#[test]
fn joined_stop_can_cancel_an_admitted_unrendered_raw_play() {
    let mut rig = Rig::new(Setup::default());
    rig.callback.as_mut().expect("callback").admit_raw();
    let joined = rig.join();
    assert!(!joined.host_halted);
    assert!(!joined.raw_unexpected_command);
    assert!(matches!(
        joined.raw_command_outcomes.as_slice(),
        [(id, LoopTransferOutcome::Command(receipt))]
            if *id == joined.raw_play_id
                && receipt.boundary.command == SessionCommand::Play
                && receipt.outcome == SessionOutcome::Cancelled
    ));
    assert!(joined.raw_join_error.is_none());
    assert!(joined.errors.is_empty());
}

#[test]
fn redundant_pedal_edges_and_unpaired_key_ups_are_split_for_sound_and_kept_by_raw() {
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    // A lift while up, an unpaired key-up, and a second down while down each settle at
    // the stage and spend nothing; the effective down and lift are forwarded.
    rig.send_all(
        FIRST,
        &[
            message(100, sustain(false)),
            message(110, note_off(70)),
            message(120, sustain(true)),
            message(130, sustain(true)),
            message(140, sustain(false)),
            message(150, sustain(false)),
            frontier(1_000),
        ],
    );
    rig.send(SECOND, frontier(1_000));
    rig.service();
    for (frames, split) in [
        (100, MixedSplit::RedundantPedal),
        (110, MixedSplit::Unpaired),
        (130, MixedSplit::RedundantPedal),
        (150, MixedSplit::RedundantPedal),
    ] {
        assert_eq!(
            rig.settled_of(rig.packet_of(FIRST, frames)),
            [Settled::Stage(Ok(MixedStaged::Split(split)))],
            "message at {frames}"
        );
    }
    assert_eq!(rig.forwarded.len(), 2, "only the effective down and lift");
    rig.render(Q).expect("serving callback");
    rig.collect();
    let down = rig.packet_of(FIRST, 120);
    assert_eq!(
        rig.report_of(down).map(|report| report.outcome()),
        Some(MixedBridgeOutcome::PedalDown {
            channel: MidiChannel::CH1,
            result: Ok(MixedTransferPedalDown::Pressed),
        })
    );
    let lift = rig.report_of(rig.packet_of(FIRST, 140)).expect("lift");
    assert!(matches!(
        lift.outcome(),
        MixedBridgeOutcome::Lift { down: paired, result: Ok(lifted), .. }
            if paired == down && lifted.count().get() == 0
    ));
    // Raw records each packet once its own callback renders past the stamp.
    rig.render_all(&[1_024]);
    rig.collect();
    assert!(rig.raw_refused.is_empty() && rig.errors.is_empty());
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.pedals().is_empty());
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw keeps {packet:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Rule 1.1: raw capture is never refused or delayed by the sound path; a silent peer is
// fenced after `W` for sound only.

#[test]
fn a_silent_peer_never_delays_raw_capture_and_is_fenced_after_its_wait_bound() {
    const WAIT: u64 = 256;
    let mut rig = Rig::new(Setup {
        cells: 4,
        peer_wait: WAIT,
        ..Setup::default()
    });
    // The first source floods eight onsets then their key-ups; the second sends nothing,
    // not even a frontier. The host services the ring after every observation.
    for note in 0..8_u8 {
        rig.send(
            FIRST,
            message(100 + 10 * u64::from(note), note_on(60 + note)),
        );
        rig.stage_all();
    }
    for note in 0..8_u8 {
        rig.send(
            FIRST,
            message(200 + 10 * u64::from(note), note_off(60 + note)),
        );
        rig.stage_all();
    }
    rig.pump();
    assert!(
        rig.admitted
            .iter()
            .all(|admitted| admitted.raw_id().is_some()),
        "every observation was raw-admitted"
    );
    assert_eq!(rig.admitted.len(), 16);
    assert!(rig.raw_refused.is_empty());
    assert_eq!(
        rig.waits.last(),
        Some(&MixedBridgeWait::Peer {
            source: rig.generation(SECOND)
        })
    );
    // Two onsets and their key-ups hold the four cells; the other six onsets were split
    // at once and their key-ups consumed by the tombstones.
    let staged = rig
        .admitted
        .iter()
        .filter(|admitted| rig.settled_of(admitted.packet).is_empty())
        .count();
    assert_eq!(staged, 4);
    for note in 2..8_u8 {
        let onset = rig.packet_of(FIRST, 100 + 10 * u64::from(note));
        let key_up = rig.packet_of(FIRST, 200 + 10 * u64::from(note));
        assert_eq!(
            rig.settled_of(onset),
            [Settled::Stage(Ok(MixedStaged::Split(
                MixedSplit::StageFull
            )))]
        );
        assert_eq!(
            rig.settled_of(key_up),
            [Settled::Stage(Ok(MixedStaged::Split(
                MixedSplit::AfterRefusedOnset {
                    onset,
                    reason: MixedOnsetRefusal::StageFull,
                }
            )))]
        );
    }
    let charge = rig.control.lane_charge(rig.generation(FIRST));
    assert_eq!(
        charge,
        Some((MixedTransferPlaces::new(4), MixedTransferPlaces::NONE))
    );

    // Until the audio boundary passes `W` the peer is not fenced and nothing forwards.
    while rig.control.audio_boundary() <= at(WAIT) {
        rig.pump();
        assert!(rig.forwarded.is_empty(), "forwarded before the fence");
        rig.render(Q).expect("unfenced callback");
    }
    rig.pump();
    let fenced_at = rig.control.audio_boundary();
    assert_eq!(fenced_at, at(WAIT + Q as u64), "the first boundary past W");
    assert_eq!(
        rig.forwarded.len(),
        4,
        "the fenced peer no longer blocks sound"
    );
    rig.render(Q).expect("serving callback");
    rig.collect();
    assert!(
        rig.errors.is_empty(),
        "peer silence is no fault: {:?}",
        rig.errors
    );
    for note in 0..2_u8 {
        let onset = rig.report_of(rig.packet_of(FIRST, 100 + 10 * u64::from(note)));
        let key_up = rig.report_of(rig.packet_of(FIRST, 200 + 10 * u64::from(note)));
        let (onset, key_up) = (onset.expect("onset report"), key_up.expect("key-up report"));
        for report in [onset, key_up] {
            assert_eq!(
                report.stamp(),
                fenced_at,
                "the offer uses the audio boundary"
            );
            assert_eq!(
                report.executed_at(),
                fenced_at,
                "executed at the fencing boundary"
            );
            let delay = report.executed_at().as_u64() - report.nominal_at().as_u64();
            assert!(
                delay <= WAIT + Q as u64,
                "the silent peer delayed sound by {delay}, beyond W and one callback"
            );
        }
        assert_eq!(
            key_up.outcome(),
            MixedBridgeOutcome::KeyUp {
                onset: onset.packet().expect("a source report"),
                result: Ok(MixedKeyUpRetired::Released(identity_of(onset))),
            }
        );
    }
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(joined.teardown.fault(), None);
    assert!(joined.teardown.tombstones().is_empty());
}

/// Ten 1024-frame rounds from both sources: two overlapping notes each, a bend, their
/// key-ups and a frontier that lets raw deliver the round before it is rendered.
fn round(source: usize, round: u64) -> Vec<InputObservation> {
    let base = 1_024 * round + 64 + 10 * source as u64;
    let low = 48 + 24 * source as u8 + round as u8;
    let high = low + 12;
    vec![
        message(base, note_on(low)),
        message(base + 50, bend(8_192 + 256 * round as u16)),
        message(base + 100, note_on(high)),
        message(base + 400, note_off(low)),
        message(base + 500, note_off(high)),
        frontier(1_024 * round + 700),
    ]
}

/// One run of the rounds. `collect_sound` false never collects a bridge result, so the
/// command ring saturates; raw completions and receipts are collected either way.
fn raw_run(setup: Setup, collect_sound: bool) -> (Joined, Vec<MixedBridgeWait>) {
    let mut rig = Rig::new(setup);
    for number in 0..10 {
        rig.send_all(FIRST, &round(FIRST, number));
        rig.send_all(SECOND, &round(SECOND, number));
        rig.stage_all();
        rig.pump();
        rig.render(1_024)
            .expect("no sound shortage faults a callback");
        if collect_sound {
            rig.collect();
        } else {
            rig.collect_raw();
        }
    }
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    assert!(
        rig.raw_callbacks.iter().all(|ok| *ok),
        "raw rendered every callback"
    );
    let waits = rig.waits.clone();
    let joined = rig.join();
    joined.assert_each_packet_once();
    (joined, waits)
}

#[test]
fn raw_capture_is_identical_without_a_bridge_and_beside_a_starved_one() {
    let (reference, _) = raw_run(
        Setup {
            bridge: false,
            ..Setup::default()
        },
        true,
    );
    let (ample, _) = raw_run(
        Setup {
            cells: 64,
            ..Setup::default()
        },
        true,
    );
    // Two cells per lane, and no result is ever collected.
    let (starved, waits) = raw_run(
        Setup {
            cells: 2,
            ..Setup::default()
        },
        false,
    );
    let raw = raw_trace(&reference.admitted, &reference.receipts);
    assert_eq!(raw_trace(&ample.admitted, &ample.receipts), raw);
    assert_eq!(raw_trace(&starved.admitted, &starved.receipts), raw);
    // Every message was captured on time and every frontier applied: raw was never
    // refused or delayed, whatever the sound path did.
    for receipt in &starved.receipts {
        let expected = match receipt.observation {
            InputObservation::Message { .. } => RawState::Published(CaptureDisposition::Recorded),
            InputObservation::Frontier { .. } => RawState::Fenced,
        };
        assert_eq!(receipt.state, expected, "{receipt:?}");
    }
    assert!(!reference.raw_halted && !starved.raw_halted);
    // The starved bridge really was short: the stage split onsets and consumed their
    // key-ups, and the uncollected ring made heads wait, while the ample one split none.
    let splits = |joined: &Joined| {
        joined
            .ledger()
            .iter()
            .filter(|(_, _, sound)| {
                matches!(
                    sound,
                    Settled::Stage(Ok(MixedStaged::Split(
                        MixedSplit::StageFull | MixedSplit::AfterRefusedOnset { .. }
                    )))
                )
            })
            .count()
    };
    assert!(splits(&starved) > 0, "the starved stage split onsets");
    assert_eq!(splits(&ample), 0);
    assert!(waits.contains(&MixedBridgeWait::Commands), "{waits:?}");
    assert!(
        starved
            .ledger()
            .iter()
            .any(|(_, _, sound)| matches!(sound, Settled::Held(MixedSourceCustody::Staged))),
        "heads were still waiting at teardown"
    );
}

// ---------------------------------------------------------------------------------------
// The publication window: a future head waits in its stage cell, not in the ingress queue.

#[test]
fn a_head_beyond_the_next_publication_window_waits_in_its_cell_and_executes_on_time() {
    let window = FrameCount::new(BLOCK);
    assert!(
        profile().limits().events().forward_event_horizon() > FrameCount::new(4 * BLOCK),
        "this head lies inside the horizon, so only the window can hold it back"
    );
    let mut rig = Rig::new(Setup::default());
    let start = rig.control.audio_boundary();
    let far = start.as_u64() + 3 * BLOCK + 100;
    rig.send_all(
        FIRST,
        &[
            message(far, note_on(60)),
            message(far + 10, note_off(60)),
            frontier(far + 20),
        ],
    );
    rig.send(SECOND, frontier(far + 20));
    rig.service();
    assert!(rig.forwarded.is_empty(), "nothing entered the command ring");
    assert_eq!(
        rig.waits.last(),
        Some(&MixedBridgeWait::Window {
            stamp: at(far),
            end: start.checked_add(window).expect("representable"),
        })
    );
    assert_eq!(
        rig.control.lane_charge(rig.generation(FIRST)),
        Some((MixedTransferPlaces::new(2), MixedTransferPlaces::NONE)),
        "the onset and its key-up wait in their cells"
    );
    let mut waited = 0;
    while rig.forwarded.is_empty() {
        rig.render(BLOCK as usize).expect("callback");
        let boundary = rig.control.audio_boundary();
        rig.pump();
        let end = boundary.checked_add(window).expect("representable");
        if rig.forwarded.is_empty() {
            assert!(at(far) >= end, "a head inside the window was held back");
            waited += 1;
        } else {
            assert!(at(far) < end, "forwarded before its window opened");
        }
        assert!(waited < 8, "the window must open");
    }
    assert!(waited >= 2, "the head waited across callbacks");
    rig.render_all(&[BLOCK as usize, BLOCK as usize]);
    rig.collect();
    let on = rig
        .report_of(rig.packet_of(FIRST, far))
        .expect("onset report");
    let up = rig
        .report_of(rig.packet_of(FIRST, far + 10))
        .expect("key-up report");
    for (report, nominal) in [(on, far), (up, far + 10)] {
        assert_eq!(
            (report.nominal_at(), report.stamp(), report.executed_at()),
            (at(nominal), at(nominal), at(nominal)),
            "a waiting head still executes at its own stamp"
        );
    }
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(joined.teardown.transfer().counters().beyond_horizon(), 0);
    for packet in [
        on.packet().expect("a source report"),
        up.packet().expect("a source report"),
    ] {
        assert_eq!(
            joined.raw_state(packet),
            RawState::Published(CaptureDisposition::Recorded)
        );
    }
}

// ---------------------------------------------------------------------------------------
// Bounded custody: a source beyond the declared holds is terminal and named.

#[test]
fn an_onset_beyond_the_declared_holds_is_terminal_and_raw_keeps_the_rest() {
    let mut rig = Rig::new(Setup {
        cells: 2,
        declared_holds: Some(3),
        ..Setup::default()
    });
    rig.send(SECOND, frontier(5_000));
    // One onset forwards and protects its key-up place; two more are refused at the full
    // stage and become tombstones, reaching the declared three held onsets.
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(110, note_on(61)),
            message(120, note_on(62)),
            frontier(200),
        ],
    );
    rig.service();
    rig.render(256).expect("callback");
    rig.collect();
    assert_eq!(
        rig.control.tombstone_count(rig.generation(FIRST)),
        Some(MixedTransferPlaces::new(2))
    );
    rig.send_all(FIRST, &[message(300, note_on(63)), frontier(400)]);
    rig.stage_all();
    let beyond = rig.packet_of(FIRST, 300);
    let fault = MixedBridgeFault::SourceHolds {
        connection: rig.generation(FIRST),
        onset: beyond,
    };
    assert_eq!(
        rig.settled_of(beyond),
        [Settled::Stage(Err(MixedBridgeError::Fault(fault)))]
    );
    rig.render(256)
        .expect_err("the terminal fault silences the next callback");
    assert!(silent(rig.frame_samples(256..rig.output_frames())));
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(joined.teardown.fault(), Some(fault));
    assert_eq!(
        joined.teardown.tombstones().len(),
        2,
        "no tombstone was dropped"
    );
    for frames in [100, 110, 120] {
        assert_eq!(
            joined.raw_state(joined.packet_of(FIRST, frames)),
            RawState::Published(CaptureDisposition::Recorded),
            "raw captured the onsets sound refused"
        );
    }
    assert_eq!(joined.raw_state(beyond), RawState::Cancelled);
}

// ---------------------------------------------------------------------------------------
// Rule 1.2: every onset admitted with its release's place keeps that place.

#[test]
fn saturated_stage_commands_and_results_still_release_every_accepted_onset() {
    let mut rig = Rig::new(Setup {
        cells: 10,
        ..Setup::default()
    });
    assert_eq!(depth(), 32, "the fixture saturates a 32-place command ring");
    // Sixteen onsets alternate between the sources, so each forwards as soon as its
    // peer's next message passes it. Together they fill the ring: sixteen commands and
    // sixteen reserved key-up places.
    for note in 0..8_u8 {
        let base = 10 + 2 * u64::from(note);
        rig.send(FIRST, message(base, note_on(60 + note)));
        rig.service();
        rig.send(SECOND, message(base + 1, note_on(80 + note)));
        rig.service();
    }
    // A ninth first-source onset finds no command place, and stays staged at the head.
    rig.send(FIRST, message(30, note_on(68)));
    rig.service();
    rig.send(SECOND, frontier(31));
    rig.service();
    assert_eq!(rig.forwarded.len(), 16);
    assert_eq!(rig.waits.last(), Some(&MixedBridgeWait::Commands));
    // The first lane is now full: eight protected key-up places, the waiting onset and
    // its own. A tenth onset is split, but every key-up still takes its reserved place.
    rig.send(FIRST, message(32, note_on(69)));
    for note in 0..10_u8 {
        rig.send(FIRST, message(40 + u64::from(note), note_off(60 + note)));
    }
    for note in 0..8_u8 {
        rig.send(SECOND, message(50 + u64::from(note), note_off(80 + note)));
    }
    rig.send(FIRST, frontier(10_000));
    rig.send(SECOND, frontier(10_000));
    rig.stage_all();
    let tenth = rig.packet_of(FIRST, 32);
    assert_eq!(
        rig.settled_of(tenth),
        [Settled::Stage(Ok(MixedStaged::Split(
            MixedSplit::StageFull
        )))]
    );
    assert_eq!(
        rig.control.lane_charge(rig.generation(FIRST)),
        Some((MixedTransferPlaces::new(10), MixedTransferPlaces::NONE)),
        "the waiting onset and nine key-ups fill the lane"
    );
    for admitted in rig.admitted.clone() {
        if let InputObservation::Message { input, .. } = admitted.observation
            && matches!(input.event(), Midi1Event::KeyRelease { .. })
            && admitted.packet != rig.packet_of(FIRST, 49)
        {
            assert!(
                rig.settled_of(admitted.packet).is_empty(),
                "a key-up of a staged onset is never refused at the stage"
            );
        }
    }
    // Audio serves the sixteen commands; their results stay uncollected, so the ring
    // stays saturated and the head keeps waiting.
    rig.render(Q).expect("serving callback");
    rig.pump();
    assert_eq!(rig.forwarded.len(), 16);
    assert_eq!(rig.waits.last(), Some(&MixedBridgeWait::Commands));
    // Collection frees the places; everything drains in stamp order.
    for _ in 0..8 {
        rig.collect();
        rig.pump();
        let rendered = rig.render(Q);
        assert!(
            rendered.is_ok(),
            "draining callback: {rendered:?}, raw halt {:?}, errors {:?}, receipts {:?}",
            rig.raw_policy.decision(),
            rig.errors,
            rig.receipts
                .iter()
                .map(|receipt| (&receipt.id, &receipt.outcome))
                .collect::<Vec<_>>()
        );
    }
    rig.collect();
    assert!(
        rig.errors.is_empty(),
        "no shortage is terminal: {:?}",
        rig.errors
    );
    assert!(
        rig.callbacks
            .iter()
            .all(|(_, _, rendered)| rendered.is_ok()),
        "sound-path shortages never fault a callback"
    );
    let mut accepted = 0;
    for admitted in rig.admitted.clone() {
        let packet = admitted.packet;
        let Some(report) = rig.report_of(packet) else {
            continue;
        };
        if let MixedBridgeOutcome::NoteOn(result) = report.outcome() {
            let key_up = rig
                .settled
                .iter()
                .find_map(|(_, settled)| match settled {
                    Settled::Report(up) => match up.outcome() {
                        MixedBridgeOutcome::KeyUp { onset, result } if onset == packet => {
                            Some(result)
                        }
                        _ => None,
                    },
                    _ => None,
                })
                .expect("every forwarded onset's key-up was reported");
            match result {
                Ok(identity) => {
                    accepted += 1;
                    assert_eq!(key_up, Ok(MixedKeyUpRetired::Released(identity)));
                }
                Err(refused) => assert_eq!(key_up, Ok(MixedKeyUpRetired::AfterRefusal(refused))),
            }
        }
    }
    assert_eq!(accepted, 4, "audio accepted the four live holds");
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert_eq!(joined.teardown.transfer().counters().non_monotone(), 0);
}

// ---------------------------------------------------------------------------------------
// Rule 1.3: a stage that refuses an onset consumes its key-up, paired by same-key FIFO.

#[test]
fn a_refused_then_accepted_onset_on_one_key_pairs_each_key_up_with_its_own_onset() {
    let mut rig = Rig::new(Setup {
        cells: 2,
        ..Setup::default()
    });
    rig.send(SECOND, frontier(5_000));
    // Key 61 fills the first lane, so the first onset of key 60 is refused at the stage.
    rig.send_all(
        FIRST,
        &[message(100, note_on(61)), message(110, note_on(60))],
    );
    rig.stage_all();
    let refused = rig.packet_of(FIRST, 110);
    assert_eq!(
        rig.settled_of(refused),
        [Settled::Stage(Ok(MixedStaged::Split(
            MixedSplit::StageFull
        )))]
    );
    // Key 61's key-up uses its onset's protected cell; both forward and free the lane.
    rig.send(FIRST, message(120, note_off(61)));
    rig.service();
    assert_eq!(rig.forwarded.len(), 2);
    // A second onset of key 60 is accepted; same-key FIFO pairs the first key-up with the
    // refused onset and the second with the accepted one, as the raw owner links them.
    rig.send_all(
        FIRST,
        &[
            message(130, note_on(60)),
            message(140, note_off(60)),
            message(150, note_off(60)),
            frontier(5_000),
        ],
    );
    rig.service();
    let accepted = rig.packet_of(FIRST, 130);
    let first_up = rig.packet_of(FIRST, 140);
    let second_up = rig.packet_of(FIRST, 150);
    assert_eq!(
        rig.settled_of(first_up),
        [Settled::Stage(Ok(MixedStaged::Split(
            MixedSplit::AfterRefusedOnset {
                onset: refused,
                reason: MixedOnsetRefusal::StageFull,
            }
        )))]
    );
    rig.render_all(&[512, 512]);
    rig.collect();
    let onset = rig.report_of(accepted).expect("accepted onset report");
    let identity = identity_of(onset);
    let key_up = rig.report_of(second_up).expect("second key-up report");
    assert_eq!(
        key_up.outcome(),
        MixedBridgeOutcome::KeyUp {
            onset: accepted,
            result: Ok(MixedKeyUpRetired::Released(identity)),
        },
        "the second key-up ends the second note"
    );
    assert_eq!(key_up.executed_at(), at(150));
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.tombstones().is_empty());
    assert!(
        joined.teardown.open().is_empty(),
        "the second note did not stay held"
    );
}

// ---------------------------------------------------------------------------------------
// Rules 1.4 and 1.5a: sustain through both source rings. A key-up under its scope's
// pedal redeems no hold; the scope's one lift redeems every sustained hold at once.

/// One source's sustain script, one message per quantum: a sustained key-up, a second
/// sustained note, the first key again under the pedal, one lift, and a key-up after it.
fn sustain_script() -> [(u64, Midi1Input); 8] {
    [
        (100, note_on(60)),
        (170, sustain(true)),
        (230, note_off(60)),
        (300, note_on(62)),
        (360, note_off(62)),
        (420, note_on(60)),
        (490, sustain(false)),
        (550, note_off(60)),
    ]
}

#[test]
fn sustain_holds_key_ups_until_one_lift_redeems_every_sustained_hold() {
    // The same script with no sound path is the raw reference.
    let reference = {
        let mut rig = Rig::new(Setup {
            bridge: false,
            ..Setup::default()
        });
        rig.prime();
        rig.send(SECOND, frontier(5_000));
        for (frames, input) in sustain_script() {
            rig.send(FIRST, message(frames, input));
            rig.step();
        }
        rig.render_all(&[1_024]);
        rig.collect();
        let joined = rig.join();
        joined.assert_each_packet_once();
        raw_trace(&joined.admitted, &joined.receipts)
    };
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    rig.send(SECOND, frontier(5_000));
    let holds = [1, 1, 1, 2, 2, 3, 1, 0];
    for ((frames, input), holds) in sustain_script().into_iter().zip(holds) {
        rig.send(FIRST, message(frames, input));
        rig.step();
        assert!(
            rig.report_of(rig.packet_of(FIRST, frames)).is_some(),
            "audio served the message at {frames}: {:?}",
            rig.waits
        );
        assert_eq!(
            rig.holds(),
            EventCount::measured(holds),
            "live holds after the message at {frames}"
        );
    }
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let outcome = |frames| {
        rig.report_of(rig.packet_of(FIRST, frames))
            .expect("served")
            .outcome()
    };
    let first = rig.packet_of(FIRST, 100);
    let second = rig.packet_of(FIRST, 300);
    let again = rig.packet_of(FIRST, 420);
    let down = rig.packet_of(FIRST, 170);
    assert_eq!(
        outcome(170),
        MixedBridgeOutcome::PedalDown {
            channel: MidiChannel::CH1,
            result: Ok(MixedTransferPedalDown::Pressed),
        }
    );
    for (frames, onset) in [(230, first), (360, second)] {
        assert_eq!(
            outcome(frames),
            MixedBridgeOutcome::KeyUp {
                onset,
                result: Ok(MixedKeyUpRetired::Sustained),
            },
            "the key-up at {frames} writes no edge"
        );
    }
    let again_identity = identity_of(rig.report_of(again).expect("repeated key"));
    assert_ne!(
        again_identity,
        identity_of(rig.report_of(first).expect("first key")),
        "the repeated key is a new occurrence beside the sustained one"
    );
    let MixedBridgeOutcome::Lift {
        channel,
        down: paired,
        result: Ok(lifted),
    } = outcome(490)
    else {
        panic!("the lift was refused: {:?}", outcome(490));
    };
    assert_eq!((channel, paired), (MidiChannel::CH1, down));
    let mut ended: Vec<_> = lifted.onsets().collect();
    ended.sort_unstable_by_key(|onset| onset.serial());
    assert_eq!(
        ended,
        [first, second],
        "one group ends both sustained notes and not the held key"
    );
    assert_eq!(
        outcome(550),
        MixedBridgeOutcome::KeyUp {
            onset: again,
            result: Ok(MixedKeyUpRetired::Released(again_identity)),
        },
        "a key-up after the lift releases its own note"
    );
    rig.render_all(&[1_024]);
    rig.collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty(), "no hold is stranded");
    assert!(joined.teardown.pedals().is_empty());
    assert_eq!(
        joined.teardown.transfer().holds_outstanding(),
        EventCount::NONE
    );
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert_eq!(
        raw_trace(&joined.admitted, &joined.receipts),
        reference,
        "carrying sustain for sound changes no raw outcome"
    );
}

#[test]
fn a_lift_ends_only_its_own_source_and_channel_sustained_notes() {
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    let script = [
        (FIRST, 100, channel_note_on(0, 60)),
        (SECOND, 110, channel_note_on(0, 64)),
        (FIRST, 120, channel_note_on(1, 62)),
        (FIRST, 130, channel_sustain(0, true)),
        (SECOND, 140, channel_sustain(0, true)),
        (FIRST, 150, channel_sustain(1, true)),
        (FIRST, 160, channel_note_off(0, 60)),
        (SECOND, 170, channel_note_off(0, 64)),
        (FIRST, 180, channel_note_off(1, 62)),
    ];
    for (source, frames, input) in script {
        rig.send(source, message(frames, input));
    }
    rig.send(FIRST, frontier(300));
    rig.send(SECOND, frontier(300));
    rig.step();
    assert_eq!(
        rig.holds(),
        EventCount::measured(3),
        "three sustained notes keep their holds"
    );
    for (source, frames) in [(FIRST, 160), (SECOND, 170), (FIRST, 180)] {
        assert!(matches!(
            rig.report_of(rig.packet_of(source, frames))
                .expect("key-up")
                .outcome(),
            MixedBridgeOutcome::KeyUp {
                result: Ok(MixedKeyUpRetired::Sustained),
                ..
            }
        ));
    }
    // Lift the three scopes one at a time: the first source's first channel shares a
    // channel with the second source and a source with the first source's second
    // channel, and ends neither.
    for (source, channel, frames, onset, remaining) in [
        (FIRST, 0, 400, (FIRST, 100), 2),
        (SECOND, 0, 500, (SECOND, 110), 1),
        (FIRST, 1, 600, (FIRST, 120), 0),
    ] {
        rig.send(source, message(frames, channel_sustain(channel, false)));
        rig.send(FIRST, frontier(frames + 50));
        rig.send(SECOND, frontier(frames + 50));
        rig.step();
        let lift = rig.report_of(rig.packet_of(source, frames)).expect("lift");
        let MixedBridgeOutcome::Lift {
            result: Ok(lifted), ..
        } = lift.outcome()
        else {
            panic!("lift at {frames} was refused: {lift:?}");
        };
        assert_eq!(
            lifted.onsets().collect::<Vec<_>>(),
            [rig.packet_of(onset.0, onset.1)],
            "the lift at {frames} ends exactly its scope's note"
        );
        assert_eq!(rig.holds(), EventCount::measured(remaining));
    }
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.open().is_empty());
    assert!(joined.teardown.pedals().is_empty());
    assert_eq!(
        joined.teardown.transfer().holds_outstanding(),
        EventCount::NONE
    );
}

#[test]
fn a_full_stage_lane_keeps_its_protected_key_up_and_lift_and_refuses_a_new_down_whole() {
    let mut rig = Rig::new(Setup {
        cells: 4,
        ..Setup::default()
    });
    // The silent peer keeps every head staged while the onset and the effective down
    // fill the lane with their own cells and protected places.
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(110, sustain(true)),
            message(120, bend(12_288)),
            message(130, note_off(60)),
            message(140, sustain(false)),
            message(150, sustain(true)),
            message(160, note_on(62)),
            message(170, note_off(62)),
            message(180, sustain(false)),
        ],
    );
    rig.stage_all();
    let refused_down = rig.packet_of(FIRST, 150);
    let refused_onset = rig.packet_of(FIRST, 160);
    let split = |split| [Settled::Stage(Ok(MixedStaged::Split(split)))];
    for frames in [120, 150, 160] {
        assert_eq!(
            rig.settled_of(rig.packet_of(FIRST, frames)),
            split(MixedSplit::StageFull),
            "ordinary pressure refuses the message at {frames}"
        );
    }
    assert_eq!(
        rig.settled_of(rig.packet_of(FIRST, 170)),
        split(MixedSplit::AfterRefusedOnset {
            onset: refused_onset,
            reason: MixedOnsetRefusal::StageFull,
        })
    );
    assert_eq!(
        rig.settled_of(rig.packet_of(FIRST, 180)),
        split(MixedSplit::AfterRefusedPedal {
            down: refused_down,
            reason: MixedOnsetRefusal::StageFull,
        }),
        "the refused down's tombstone consumes its lift"
    );
    for frames in [130, 140] {
        assert!(
            rig.settled_of(rig.packet_of(FIRST, frames)).is_empty(),
            "the protected release at {frames} is staged"
        );
    }
    assert_eq!(
        rig.control.lane_charge(rig.generation(FIRST)),
        Some((MixedTransferPlaces::new(4), MixedTransferPlaces::NONE)),
        "the onset, down, key-up and lift fill the lane"
    );
    rig.send(FIRST, frontier(1_000));
    rig.send(SECOND, frontier(1_000));
    rig.step();
    rig.step();
    let onset = rig.packet_of(FIRST, 100);
    assert_eq!(
        rig.report_of(rig.packet_of(FIRST, 130))
            .expect("key-up")
            .outcome(),
        MixedBridgeOutcome::KeyUp {
            onset,
            result: Ok(MixedKeyUpRetired::Sustained),
        }
    );
    assert!(matches!(
        rig.report_of(rig.packet_of(FIRST, 140)).expect("lift").outcome(),
        MixedBridgeOutcome::Lift { result: Ok(lifted), .. }
            if lifted.onsets().eq([onset])
    ));
    assert_eq!(rig.holds(), EventCount::NONE);
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.tombstones().is_empty());
    assert!(joined.teardown.pedals().is_empty());
    assert!(joined.teardown.open().is_empty());
}

#[test]
fn uncollected_results_never_refuse_a_sustained_key_up_or_its_lift() {
    let mut rig = Rig::new(Setup {
        cells: 40,
        ..Setup::default()
    });
    assert_eq!(depth(), 32, "the fixture saturates a 32-place command ring");
    rig.prime();
    // The onset and the down charge four places: two commands, a key-up place and a lift
    // place. Fourteen second-source onsets take the other twenty-eight.
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(101, sustain(true)),
            frontier(200),
        ],
    );
    for note in 0..14_u8 {
        rig.send(SECOND, message(102 + u64::from(note), note_on(70 + note)));
    }
    rig.service();
    assert_eq!(rig.forwarded.len(), 16, "{:?}", rig.waits);
    // Audio serves all sixteen; their results stay uncollected, so every place stays
    // charged.
    rig.render(Q).expect("serving callback");
    assert_eq!(rig.holds(), EventCount::measured(4), "four live holds");
    rig.send_all(
        FIRST,
        &[
            message(300, note_off(60)),
            message(310, sustain(false)),
            frontier(1_000),
        ],
    );
    rig.send(SECOND, message(320, bend(12_288)));
    rig.stage_all();
    rig.pump();
    assert_eq!(
        rig.forwarded.len(),
        18,
        "the key-up and lift take their reserved places"
    );
    assert_eq!(
        rig.waits.last(),
        Some(&MixedBridgeWait::Commands),
        "an ordinary bend waits for a place"
    );
    rig.render(Q).expect("serving callback");
    assert_eq!(
        rig.holds(),
        EventCount::measured(3),
        "the lift redeemed the sustained hold"
    );
    rig.collect();
    let onset = rig.packet_of(FIRST, 100);
    assert_eq!(
        rig.report_of(rig.packet_of(FIRST, 300))
            .expect("key-up")
            .outcome(),
        MixedBridgeOutcome::KeyUp {
            onset,
            result: Ok(MixedKeyUpRetired::Sustained),
        }
    );
    assert!(matches!(
        rig.report_of(rig.packet_of(FIRST, 310)).expect("lift").outcome(),
        MixedBridgeOutcome::Lift { result: Ok(lifted), .. } if lifted.onsets().eq([onset])
    ));
    for note in 0..14_u8 {
        rig.send(SECOND, message(400 + u64::from(note), note_off(70 + note)));
    }
    rig.send(FIRST, frontier(2_000));
    rig.send(SECOND, frontier(2_000));
    for _ in 0..6 {
        rig.step();
    }
    assert!(rig.report_of(rig.packet_of(SECOND, 320)).is_some());
    assert_eq!(rig.holds(), EventCount::NONE);
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert!(joined.teardown.pedals().is_empty());
    assert_eq!(joined.teardown.transfer().counters().non_monotone(), 0);
}

#[test]
fn a_quiescent_full_command_ring_refuses_a_pedal_down_whole_and_consumes_its_staged_lift() {
    let mut rig = Rig::new(Setup {
        cells: 40,
        ..Setup::default()
    });
    // Thirty-one onsets reserve every place but one, so a down can never take its own
    // place and its lift's.
    for note in 0..16_u8 {
        let base = 10 + 2 * u64::from(note);
        rig.send(FIRST, message(base, note_on(30 + note)));
        if note < 15 {
            rig.send(SECOND, message(base + 1, note_on(60 + note)));
        }
        rig.service();
    }
    // The lift is staged before the down meets the ring, so the refusal moves into the
    // lift's own cell.
    rig.send_all(
        SECOND,
        &[message(41, sustain(true)), message(43, sustain(false))],
    );
    rig.stage_all();
    rig.send(FIRST, frontier(100));
    let down = rig.packet_of(SECOND, 41);
    for _ in 0..8 {
        rig.step();
        if !rig.settled_of(down).is_empty() {
            break;
        }
    }
    assert!(
        rig.waits.contains(&MixedBridgeWait::Commands),
        "while any command was outstanding the down waited"
    );
    assert_eq!(
        rig.settled_of(down),
        [Settled::Pump(MixedSplit::CommandsFull)]
    );
    assert_eq!(
        rig.settled_of(rig.packet_of(SECOND, 43)),
        [Settled::Pump(MixedSplit::AfterRefusedPedal {
            down,
            reason: MixedOnsetRefusal::CommandsFull,
        })]
    );
    assert_eq!(rig.forwarded.len(), 31);
    for note in 0..16_u8 {
        rig.send(FIRST, message(200 + u64::from(note), note_off(30 + note)));
        if note < 15 {
            rig.send(SECOND, message(200 + u64::from(note), note_off(60 + note)));
        }
    }
    rig.send(FIRST, frontier(1_000));
    rig.send(SECOND, frontier(1_000));
    for _ in 0..4 {
        rig.step();
    }
    assert_eq!(rig.holds(), EventCount::NONE);
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert!(joined.teardown.tombstones().is_empty());
    assert!(joined.teardown.pedals().is_empty());
}

#[test]
fn a_stopped_reset_retires_sustained_notes_and_consumes_the_old_lift() {
    const B: SampleTime = SampleTime::new(448);
    let mut rig = Rig::new(Setup::default());
    let callback = rig.callback.as_mut().expect("callback");
    let MixedHostAudio::Direct(audio, port) = callback.mixed.take().expect("direct owner") else {
        panic!("fresh rig is direct");
    };
    let owner = MixedBridgeResetOwner::new(*audio, port).expect("matching audio port");
    let mut wrapper = MixedBridgeResetAudio::new(owner);
    wrapper.schedule_stop(at(192)).expect("compiled Stop");
    callback.mixed = Some(MixedHostAudio::Reset(Box::new(wrapper)));
    rig.render(Q).expect("start raw capture and mixed plan");
    rig.collect();

    // The key-up is served under the pedal before the reset: the note is sustained.
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(110, sustain(true)),
            message(120, note_off(60)),
            frontier(150),
        ],
    );
    rig.send(SECOND, frontier(150));
    rig.service();
    while rig.control.audio_boundary() < at(320) {
        rig.render(Q).expect("whole callback");
        rig.collect();
    }
    let onset = rig.packet_of(FIRST, 100);
    let down = rig.packet_of(FIRST, 110);
    assert_eq!(
        rig.report_of(rig.packet_of(FIRST, 120))
            .expect("sustained key-up")
            .outcome(),
        MixedBridgeOutcome::KeyUp {
            onset,
            result: Ok(MixedKeyUpRetired::Sustained),
        }
    );

    let MixedBridgeArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = arm_at(&rig.raw, Setup::default(), B);
    rig.control.seal_reset(control, B).expect("sealed bridge");
    assert_eq!(rig.control.reset_ready(), Some(B));
    let successor = MixedBridgeResetOwner::new(audio, audio_port).expect("successor port");
    let Some(MixedHostAudio::Reset(wrapper)) =
        rig.callback.as_mut().expect("callback").mixed.as_mut()
    else {
        panic!("reset wrapper");
    };
    wrapper
        .offer_reset(successor, B)
        .expect("stopped owner accepts reset");

    // The lift arrives while sealed; it waits in its cell for the commit.
    rig.send_all(FIRST, &[message(420, sustain(false)), frontier(480)]);
    rig.send(SECOND, frontier(480));
    rig.service();
    assert_eq!(
        rig.waits.last(),
        Some(&MixedBridgeWait::ResetSealed { at: B })
    );
    while rig.control.audio_boundary() < B {
        rig.render(Q).expect("whole callback");
        rig.collect();
    }
    rig.render(8).expect("open reset boundary");
    let Some(MixedHostAudio::Reset(wrapper)) =
        rig.callback.as_mut().expect("callback").mixed.as_mut()
    else {
        panic!("reset wrapper");
    };
    let MixedBridgeResetState::Fading { receipt, .. } = wrapper.state() else {
        panic!("reset did not enter its fade: {:?}", wrapper.state());
    };
    let ended: Vec<_> = receipt.ended().collect();
    assert!(
        matches!(ended.as_slice(), [MixedBridgeResetEnd::Retired(_)]),
        "the reset group retires the sustained cell: {ended:?}"
    );
    let old = rig
        .control
        .commit_reset(receipt)
        .expect("commit bridge handoff");
    rig.reset_old
        .push((old, std::mem::replace(&mut rig.control_port, control_port)));

    // The successor's pedal starts up: a fresh down sustains a fresh note there.
    rig.send_all(
        FIRST,
        &[
            message(530, sustain(true)),
            message(540, note_on(62)),
            message(550, note_off(62)),
            message(560, sustain(false)),
            frontier(640),
        ],
    );
    rig.send(SECOND, frontier(640));
    rig.service();
    rig.render(56).expect("fade remainder");
    rig.collect();
    for _ in 0..3 {
        rig.service();
        rig.render(Q).expect("later callback");
        rig.collect();
    }

    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(
        !joined.host_halted && !joined.raw_halted && joined.errors.is_empty(),
        "errors: {:?}",
        joined.errors
    );
    assert_eq!(
        joined.sound(joined.packet_of(FIRST, 420)),
        Settled::Pump(MixedSplit::AfterRefusedPedal {
            down,
            reason: MixedOnsetRefusal::Reset,
        }),
        "the old lift never reaches the successor"
    );
    let new_down = joined
        .report_of(joined.packet_of(FIRST, 530))
        .expect("new down");
    assert_eq!(
        new_down.outcome(),
        MixedBridgeOutcome::PedalDown {
            channel: MidiChannel::CH1,
            result: Ok(MixedTransferPedalDown::Pressed),
        }
    );
    assert!(new_down.executed_at() >= B);
    let new_onset = joined.packet_of(FIRST, 540);
    assert!(matches!(
        joined.report_of(joined.packet_of(FIRST, 550)).expect("new key-up").outcome(),
        MixedBridgeOutcome::KeyUp { onset: found, result: Ok(MixedKeyUpRetired::Sustained) }
            if found == new_onset
    ));
    assert!(matches!(
        joined.report_of(joined.packet_of(FIRST, 560)).expect("new lift").outcome(),
        MixedBridgeOutcome::Lift { result: Ok(lifted), .. } if lifted.onsets().eq([new_onset])
    ));
    let [retired] = joined.retired_reset.as_slice() else {
        panic!("one retired owner joined");
    };
    assert!(matches!(
        retired.handles(),
        [(packet, MixedRetiredHandle::Ended(MixedBridgeResetEnd::Retired(_)))]
            if *packet == onset
    ));
    assert_eq!(
        retired.bridge().transfer().holds_outstanding(),
        EventCount::NONE,
        "the reset group redeemed the sustained hold"
    );
    assert!(retired.bridge().held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert!(joined.teardown.pedals().is_empty());
    assert_eq!(
        joined.teardown.transfer().holds_outstanding(),
        EventCount::NONE
    );
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw packet {packet:?} did not record across reset"
        );
    }
}

// ---------------------------------------------------------------------------------------
// R1: a fenced peer's message executes at the monotone execution stamp.

#[test]
fn a_fenced_peer_message_executes_behind_the_release_it_follows_with_both_stamps() {
    const WAIT: u64 = 128;
    let mut rig = Rig::new(Setup {
        peer_wait: WAIT,
        ..Setup::default()
    });
    // The second source is silent while the first plays one note.
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(300, note_off(60)),
            frontier(350),
        ],
    );
    rig.stage_all();
    while rig.control.audio_boundary() <= at(WAIT) {
        rig.pump();
        rig.render(Q).expect("callback");
    }
    rig.pump();
    rig.render(Q).expect("serving callback");
    rig.collect();
    let first_off = rig
        .report_of(rig.packet_of(FIRST, 300))
        .expect("first source key-up");
    let rig_first_on = rig.packet_of(FIRST, 100);
    assert_eq!(rig.control.last_stamp(), Some(at(300)));
    // Now the fenced peer delivers an onset stamped before that forwarded key-up.
    rig.send_all(
        SECOND,
        &[
            message(150, note_on(64)),
            message(400, note_off(64)),
            frontier(500),
        ],
    );
    rig.send(FIRST, frontier(500));
    let execution_boundary = rig.control.audio_boundary();
    assert!(
        execution_boundary < at(300),
        "the merger fence must dominate"
    );
    rig.service();
    rig.render_all(&[Q, Q, Q, Q]);
    rig.collect();
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let late = rig
        .report_of(rig.packet_of(SECOND, 150))
        .expect("late onset");
    assert_eq!(late.nominal_at(), at(150), "raw keeps the nominal stamp");
    assert_eq!(
        late.stamp(),
        at(300).max(execution_boundary),
        "the offer respects both the merger and audio boundaries"
    );
    assert!(late.executed_at() >= first_off.executed_at());
    assert!(matches!(late.outcome(), MixedBridgeOutcome::NoteOn(Ok(_))));
    let late_off = rig
        .report_of(rig.packet_of(SECOND, 400))
        .expect("late key-up");
    assert_eq!(
        late_off.outcome(),
        MixedBridgeOutcome::KeyUp {
            onset: late.packet().expect("a source report"),
            result: Ok(MixedKeyUpRetired::Released(identity_of(late))),
        }
    );
    assert!(late_off.executed_at() >= late.executed_at());
    let joined = rig.join();
    joined.assert_each_packet_once();
    // Split outcomes: raw capture keeps its own frontier law. The silent peer held raw
    // delivery back until its frontier arrived, so the recorder refused both onsets as
    // late and interrupted capture, while the sound path played both notes. The raw
    // interruption is not charged to the bridge or the host: that would blame the
    // waiting source for its peer's silence.
    assert!(
        joined.raw_halted && !joined.host_halted,
        "raw interrupted itself under its own late rule"
    );
    assert!(joined.halted_sends.is_empty(), "no source was halted");
    assert_eq!(
        joined.raw_state(late.packet().expect("a source report")),
        RawState::Refused
    );
    assert_eq!(joined.raw_state(rig_first_on), RawState::Refused);
    assert_eq!(
        joined.teardown.fault(),
        None,
        "peer silence is no bridge fault"
    );
    let counters = joined.teardown.transfer().counters();
    assert_eq!(
        counters.non_monotone(),
        0,
        "no forwarded packet met the store's fence"
    );
    assert_eq!(counters.beyond_horizon(), 0);
}

// ---------------------------------------------------------------------------------------
// Raw-only interruption: a late raw refusal stops capture for both sources and nothing
// else. Sound keeps staging both sources under their source IDs.

/// One round of the interruption script for `source`; round 2 sends nothing. Two onsets
/// of key 60 on the first source are held across the interruption and released after
/// it, and both sources start and end notes once capture has stopped.
fn interruption_script(round: u64, source: usize) -> Vec<InputObservation> {
    match (round, source) {
        (0, FIRST) => vec![
            message(100, note_on(60)),
            message(200, note_on(62)),
            message(300, note_off(62)),
            frontier(700),
        ],
        (0, _) => vec![message(150, note_on(64)), frontier(700)],
        (1, FIRST) => vec![
            message(1_124, note_on(60)),
            message(1_200, bend(12_288)),
            frontier(1_724),
        ],
        (1, _) => vec![
            message(1_150, note_off(64)),
            message(1_174, note_on(65)),
            frontier(1_724),
        ],
        (3, FIRST) => vec![
            message(3_172, note_off(60)),
            message(3_272, note_off(60)),
            message(3_372, bend(8_192)),
            frontier(3_796),
        ],
        (3, _) => vec![
            message(3_222, note_off(65)),
            message(3_322, note_on(67)),
            frontier(3_796),
        ],
        (4, FIRST) => vec![frontier(4_820)],
        (4, _) => vec![message(4_196, note_off(67)), frontier(4_820)],
        _ => Vec::new(),
    }
}

/// Six 1024-frame rounds of the interruption script. `late` withholds every raw packet
/// in round 1, so raw audio receives them in round 2, after it rendered their stamps.
/// Nothing else differs, so the run without it is the ample-raw reference.
fn interruption_run(late: bool) -> (Trace, Joined) {
    let mut rig = Rig::new(Setup::default());
    for round in 0..6 {
        rig.raw_withheld = late && round == 1;
        for source in [FIRST, SECOND] {
            rig.send_all(source, &interruption_script(round, source));
        }
        rig.service();
        if late && round == 3 {
            assert!(rig.raw_interrupted(), "the real late receipt was collected");
            let raw = rig
                .raw_publisher
                .interrupted_publication(&rig.raw.control, &rig.raw_policy)
                .expect("classified raw-only proof");
            assert!(
                raw.through() < rig.control.audio_boundary(),
                "raw ended before current sound"
            );
            let sound = rig.control.publication().expect("fresh source promises");
            let (mut sender, mut receiver) = rig.control.publication_mailbox();
            sender.publish(sound, raw).expect("publish interruption");
            let (sound, raw) = receiver.try_read().expect("complete mode copy");
            assert!(raw.is_interrupted());
            let mut gate = rig.control.publication_gate();
            let MixedHostAudio::Direct(audio, _) = rig
                .callback
                .as_ref()
                .expect("callback")
                .mixed
                .as_ref()
                .expect("mixed")
            else {
                panic!("direct fixture");
            };
            audio
                .offer_publication(&mut gate, sound, raw)
                .expect("same source lifetimes");
            audio
                .check_publication(&mut gate, FrameCount::QUANTUM)
                .expect("raw-only interruption cannot close an otherwise ready sound prefix");
        }
        rig.render(BLOCK as usize)
            .expect("raw capture never faults a sound callback");
        rig.collect();
    }
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    assert!(
        rig.frontiers.iter().all(|(_, advanced)| advanced.is_ok()),
        "{:?}",
        rig.frontiers
    );
    let trace = trace(&rig, None);
    let joined = rig.join();
    joined.assert_each_packet_once();
    (trace, joined)
}

#[test]
fn a_late_raw_refusal_stops_capture_for_both_sources_while_sound_matches_ample_raw() {
    let (reference, ample) = interruption_run(false);
    let (trace, late) = interruption_run(true);

    // Raw: the ample reference captured every message on time and never halted.
    assert!(!ample.raw_halted && !ample.host_halted);
    for (packet, state, _) in ample.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "{packet:?}"
        );
    }
    // The withheld round reached raw late, the recorder refused it, and capture stopped
    // for both sources. No source, host or bridge fault followed.
    assert!(
        late.raw_halted && !late.host_halted,
        "raw interrupted itself"
    );
    assert_eq!(late.teardown.fault(), None);
    assert!(late.halted_sends.is_empty(), "both sources kept sending");
    let mut refused = 0;
    let mut continued = [false; 2];
    for admitted in late
        .admitted
        .iter()
        .filter(|admitted| admitted.is_message())
    {
        let InputObservation::Message { tick, .. } = admitted.observation else {
            continue;
        };
        let state = late.raw_state(admitted.packet);
        match tick.as_u64() {
            0..1_024 => assert_eq!(state, RawState::Published(CaptureDisposition::Recorded)),
            1_024..2_048 => {
                assert!(admitted.raw_id().is_some(), "raw ran when it was offered");
                assert!(
                    matches!(
                        state,
                        RawState::Refused
                            | RawState::Published(CaptureDisposition::Late)
                            | RawState::Cancelled
                    ),
                    "{admitted:?} {state:?}"
                );
                refused += usize::from(state == RawState::Refused);
            }
            _ => {
                assert_eq!(admitted.raw, RawAdmission::Interrupted, "{admitted:?}");
                assert_eq!(state, RawState::NotOffered);
                continued[admitted.source] = true;
            }
        }
    }
    assert!(refused > 0, "the recorder refused a late packet");
    let late_refusals: Vec<_> = late
        .receipts
        .iter()
        .filter(|receipt| {
            matches!(
                &receipt.outcome,
                InputOutcome::Refused(LoopSessionError::Session(SessionError::SourceOrder))
            )
        })
        .collect();
    assert!(
        !late_refusals.is_empty(),
        "the exact late reason is retained"
    );
    for receipt in late_refusals {
        assert!(
            late.admitted.iter().any(|admitted| {
                admitted.raw_id() == Some(receipt.id) && admitted.observation == receipt.observation
            }),
            "the refused receipt names its original source packet"
        );
    }
    assert_eq!(
        continued,
        [true, true],
        "both sources reached sound after the stop"
    );

    // Sound: the same reports, splits, identities and samples as the ample reference.
    assert_eq!(trace, reference);
    assert_eq!(late.rendered.len(), ample.rendered.len());
    assert!(
        late.rendered
            .iter()
            .zip(&ample.rendered)
            .all(|(late, ample)| late.to_bits() == ample.to_bits()),
        "raw capture changed the sound"
    );
    assert!(late.teardown.held().is_empty());
    assert!(late.teardown.open().is_empty(), "no note stayed held");
    assert!(late.teardown.tombstones().is_empty());
    // Notes held across the stop release by their source IDs alone, and the two key-ups
    // of key 60 end the two key-60 notes first in, first out.
    let report = |source, frames| {
        late.report_of(late.packet_of(source, frames))
            .unwrap_or_else(|| panic!("source {source} at {frames} was reported"))
    };
    for (source, onset, key_up) in [
        (FIRST, 100, 3_172),
        (FIRST, 1_124, 3_272),
        (SECOND, 150, 1_150),
        (SECOND, 1_174, 3_222),
        (SECOND, 3_322, 4_196),
    ] {
        let onset = report(source, onset);
        assert_eq!(
            report(source, key_up).outcome(),
            MixedBridgeOutcome::KeyUp {
                onset: onset.packet().expect("a source report"),
                result: Ok(MixedKeyUpRetired::Released(identity_of(onset))),
            },
            "source {source} key-up at {key_up}"
        );
    }
}

#[test]
fn a_raw_self_halt_without_a_collected_late_reason_stops_the_whole_host() {
    let mut rig = Rig::new(Setup::default());
    rig.raw.halt.request_invalid();
    rig.service();
    assert!(rig.host_halt.is_requested());
    assert_eq!(rig.raw_policy.decision(), Some(RawHaltClass::Terminal));
    rig.raw_policy
        .observe(
            rig.generation(FIRST),
            &InputOutcome::Delivered(SessionSourceOutcome::Refused(
                NoteCaptureError::PastBoundary,
            )),
        )
        .expect("late outcome arriving after terminal classification");
    assert_eq!(rig.raw_policy.decision(), Some(RawHaltClass::Terminal));
    assert_eq!(
        rig.render(64),
        Err(MixedHostRenderError::Host(
            MixedTransferRenderError::RemoteHalt
        ))
    );
    assert!(rig.rendered.iter().all(|sample| *sample == 0.0));
    let joined = rig.join();
    assert!(joined.host_halted && joined.raw_halted);
    assert_eq!(joined.teardown.fault(), Some(MixedBridgeFault::RawSelfHalt));
}

#[test]
fn a_raw_self_halt_with_a_waiting_peer_packet_keeps_source_custody() {
    let mut rig = Rig::new(Setup::default());
    let peer = message(32, note_on(64));
    rig.send(SECOND, peer);
    rig.raw.halt.request_invalid();
    rig.service();
    assert!(rig.host_halt.is_requested());
    assert!(
        rig.raw_refused.is_empty(),
        "the peer did not cause the raw halt"
    );
    let joined = rig.join();
    assert_eq!(joined.teardown.fault(), Some(MixedBridgeFault::RawSelfHalt));
    assert!(joined.unexamined.contains(&(SECOND, peer)));
}

#[test]
fn a_late_classification_cannot_hide_direct_raw_errors() {
    let mut render_fault = Rig::new(Setup::default());
    render_fault
        .raw_policy
        .observe(
            render_fault.generation(FIRST),
            &InputOutcome::Delivered(SessionSourceOutcome::Refused(
                NoteCaptureError::PastBoundary,
            )),
        )
        .expect("own raw source");
    render_fault.raw.halt.request_invalid();
    let mut samples = [0.25_f32; 64];
    assert!(
        render_fault
            .callback
            .as_mut()
            .expect("callback")
            .record_raw_render_error(
                LoopSessionError::Session(SessionError::SourceOrder),
                &mut samples,
            )
    );
    assert!(samples.iter().all(|sample| *sample == 0.0));
    render_fault.reconcile_raw_halt();
    let joined = render_fault.join();
    assert!(joined.host_halted);
    assert_eq!(joined.teardown.fault(), Some(MixedBridgeFault::RawSelfHalt));
    assert!(matches!(
        joined.raw_render_error,
        Some(LoopSessionError::Session(SessionError::SourceOrder))
    ));

    let mut prepare_fault = Rig::new(Setup::default());
    prepare_fault
        .raw_policy
        .observe(
            prepare_fault.generation(FIRST),
            &InputOutcome::Delivered(SessionSourceOutcome::Refused(
                NoteCaptureError::PastBoundary,
            )),
        )
        .expect("own raw source");
    prepare_fault.raw.halt.request_invalid();
    prepare_fault.record_raw_packet_error(InputCaptureError::Recording(LoopSessionError::Session(
        SessionError::SourceOrder,
    )));
    let joined = prepare_fault.join();
    assert!(joined.host_halted);
    assert_eq!(joined.teardown.fault(), Some(MixedBridgeFault::RawSelfHalt));
    assert!(matches!(
        joined.raw_packet_errors.as_slice(),
        [InputCaptureError::Recording(LoopSessionError::Session(
            SessionError::SourceOrder
        ))]
    ));
}

#[test]
fn both_sources_can_fill_their_declared_raw_hold_entitlements_together() {
    let mut rig = Rig::new(Setup::default());
    for note in 0..64_u8 {
        let stamp = u64::from(note) * 64 + 1;
        rig.send(FIRST, message(stamp, note_on(60 + note)));
        rig.send(SECOND, message(stamp, note_on(60 + note)));
        rig.send(FIRST, frontier(stamp + 1));
        rig.send(SECOND, frontier(stamp + 1));
        rig.service();
        let rendered = rig.render(Q);
        assert!(
            rendered.is_ok(),
            "legal hold {note} rendered {rendered:?}, errors {:?}, raw decision {:?}",
            rig.errors,
            rig.raw_policy.decision()
        );
        rig.collect();
        assert!(!rig.raw.halt.is_requested(), "raw hold {note}");
        assert!(!rig.host_halt.is_requested(), "host hold {note}");
    }
    rig.render(Q).expect("flush the raw owner's final quantum");
    rig.collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(
        joined
            .receipts
            .iter()
            .filter(|receipt| receipt.state == RawState::Published(CaptureDisposition::Recorded))
            .count(),
        128,
        "states: {:?}",
        joined
            .receipts
            .iter()
            .take(8)
            .map(|receipt| receipt.state)
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------------------
// Arm refusals return every input untouched.

#[test]
fn arm_refuses_duplicate_sources_and_unusable_lanes_or_pairing_tables_with_inputs_returned() {
    let raw = raw_owner(64);
    assert_eq!(
        MixedTransferSources::new(raw.generations[FIRST], raw.generations[FIRST]).unwrap_err(),
        MixedTransferSourcesError::Duplicate
    );
    let sources =
        MixedTransferSources::new(raw.generations[FIRST], raw.generations[SECOND]).unwrap();
    assert_eq!(
        (sources.first(), sources.second()),
        (raw.generations[FIRST], raw.generations[SECOND])
    );
    let fence = |cells, holds| MixedTransferFence {
        stage_cells: MixedTransferPlaces::new(cells),
        held_onsets: MixedTransferPlaces::new(holds),
        peer_wait: FrameCount::new(64),
    };
    let (mut owner, mut candidate) = prepared();
    let mut commands: Commands = HostQueue::new(places());
    let mut results: Results = HostQueue::new(places());
    for cells in [0, 1, 257] {
        let refusal = owner
            .arm_bridge_rehearsal(
                candidate,
                &profile(),
                sources,
                fence(cells, 64),
                commands,
                results,
            )
            .expect_err("unusable lane");
        assert_eq!(
            refusal.reason(),
            MixedTransferArmError::StageCells {
                cells: MixedTransferPlaces::new(cells),
                maximum: MixedTransferPlaces::new(256),
            }
        );
        (owner, candidate, commands, results) = refusal.into_parts();
    }
    for holds in [0, 65_537] {
        let refusal = owner
            .arm_bridge_rehearsal(
                candidate,
                &profile(),
                sources,
                fence(2, holds),
                commands,
                results,
            )
            .expect_err("unusable pairing table");
        assert_eq!(
            refusal.reason(),
            MixedTransferArmError::HeldOnsets {
                onsets: MixedTransferPlaces::new(holds),
                maximum: MixedTransferPlaces::new(65_536),
            }
        );
        (owner, candidate, commands, results) = refusal.into_parts();
    }
    // A queue of exactly the admitted bound has no place for Panic's protected command
    // or result.
    let undersized: Commands = HostQueue::new(depth());
    let refusal = owner
        .arm_bridge_rehearsal(
            candidate,
            &profile(),
            sources,
            fence(2, 64),
            undersized,
            results,
        )
        .expect_err("undersized command queue");
    assert_eq!(
        refusal.reason(),
        MixedTransferArmError::CommandCapacity {
            required: MixedTransferPlaces::new(places()),
            reported: MixedTransferPlaces::new(depth()),
        }
    );
    let (owner, candidate, _undersized, results) = refusal.into_parts();
    let undersized: Results = HostQueue::new(depth());
    let refusal = owner
        .arm_bridge_rehearsal(
            candidate,
            &profile(),
            sources,
            fence(2, 64),
            commands,
            undersized,
        )
        .expect_err("undersized result queue");
    assert_eq!(
        refusal.reason(),
        MixedTransferArmError::ResultCapacity {
            required: MixedTransferPlaces::new(places()),
            reported: MixedTransferPlaces::new(depth()),
        }
    );
    let (owner, candidate, commands, _undersized) = refusal.into_parts();
    let armed = owner
        .arm_bridge_rehearsal(
            candidate,
            &profile(),
            sources,
            fence(2, 64),
            commands,
            results,
        )
        .expect("the returned inputs arm");
    assert_eq!(armed.control.fault(), None);
}

// ---------------------------------------------------------------------------------------
// Rule 1.8: one closed list of terminal causes; each silences the whole host from the
// callback that observes it, and joined teardown still finds every source ID.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Cause {
    /// The host hands the bridge a stamp behind its source's frontier.
    BridgeOrder,
    /// A source thread offers an observation stamped before its own frontier.
    PreRingOrder,
    /// A source ring stays full past the retry deadline and the producer retires.
    RetryExpired,
    /// The raw owner refuses a ring packet for capacity while raw capture runs.
    RawAdmission,
    /// The source hands over more held onsets than its bridge arm declared.
    SourceHolds,
    /// A source packet of an unbound connection reaches the bridge.
    ForeignSource,
    /// The host command queue loses an accepted command.
    LostCommand,
    /// The host result queue loses an accepted result.
    LostResult,
}

struct Terminal {
    joined: Joined,
    fault: Option<MixedBridgeFault>,
    callbacks: Vec<(usize, usize, Result<(), MixedTransferRenderError>)>,
    raw_refused: Vec<(usize, InputObservation, InputError)>,
    errors: Vec<MixedBridgeError>,
    foreign: Option<ConnectionGeneration>,
    generations: [ConnectionGeneration; 2],
}

/// The first packet of a connection neither bridge source is bound to.
fn foreign_packet() -> MixedSourcePacketId {
    let mut outsider = SimulatedNoteInput::new(
        EndpointId::new("outsider".to_owned()).expect("endpoint"),
        InputLimits {
            cells: InputCapacity::new(8).expect("cells"),
            bytes: PreparedBytes::measured(1 << 16),
        },
    )
    .expect("outsider input");
    MixedSourcePacketId::new(outsider.begin().expect("generation"), 1)
        .expect("nonzero packet serial")
}

fn terminal(cause: Cause, partition: &[usize]) -> Terminal {
    assert_eq!(partition.iter().sum::<usize>(), 256);
    let mut setup = Setup::default();
    match cause {
        Cause::LostCommand => setup.commands.lose_push = Some(3),
        Cause::LostResult => setup.results.lose_push = Some(3),
        Cause::RawAdmission => setup.raw_cells = 8,
        Cause::SourceHolds => setup.declared_holds = Some(1),
        _ => {}
    }
    let mut rig = Rig::new(setup);
    let generations = [rig.generation(FIRST), rig.generation(SECOND)];
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(300)]);
    rig.send_all(SECOND, &[message(120, note_on(64)), frontier(300)]);
    rig.service();
    assert_eq!(rig.forwarded.len(), 2);
    rig.render_all(partition);
    assert!(
        rig.callbacks.iter().all(|(_, _, result)| result.is_ok()),
        "the first round succeeds"
    );
    rig.collect();
    let mut foreign = None;
    match cause {
        Cause::BridgeOrder => {
            let input = note_on(61);
            let id = rig
                .raw
                .control
                .offer_observation(generations[FIRST], message(400, input))
                .expect("raw admits the message at its own time");
            // A host defect: the next source packet's handoff names a stamp
            // behind the source's frontier.
            let packet =
                MixedSourcePacketId::new(generations[FIRST], 3).expect("nonzero packet serial");
            rig.admitted.push(Admitted {
                source: FIRST,
                packet,
                observation: message(400, input),
                raw: RawAdmission::Admitted { id, link: None },
            });
            let staged = rig.control.stage(MixedSourceHandoff {
                packet,
                at: at(50),
                input,
            });
            rig.settled.push((packet, Settled::Stage(staged)));
        }
        Cause::PreRingOrder => {
            let refused = rig.producer(FIRST).submit_owned(message(200, note_on(61)));
            assert!(matches!(
                refused,
                Err(SourceOwnedSendError::Refused(
                    source_custody::SourceSendError::Invalid(_, InputError::Order)
                ))
            ));
        }
        Cause::RetryExpired => {
            for tick in 301..=316 {
                rig.send(FIRST, frontier(tick));
            }
            rig.send(FIRST, frontier(317));
            let token = rig.retries[FIRST]
                .take()
                .expect("a full ring returns its token");
            let retired = rig
                .producer(FIRST)
                .retire_owned(token)
                .expect("the producer's own token");
            assert_eq!(retired.observation(), frontier(317));
        }
        Cause::RawAdmission => {
            for note in 0..32_u8 {
                rig.send(FIRST, message(400 + u64::from(note), note_on(61 + note)));
                rig.stage_all();
                if !rig.raw_refused.is_empty() {
                    break;
                }
            }
            assert!(!rig.raw_refused.is_empty(), "the raw owner filled");
        }
        Cause::SourceHolds => {
            rig.send(FIRST, message(400, note_on(61)));
            rig.stage_all();
            assert!(matches!(
                rig.control.fault(),
                Some(MixedBridgeFault::SourceHolds { .. })
            ));
        }
        Cause::ForeignSource => {
            let packet = foreign_packet();
            foreign = Some(packet.connection());
            let staged = rig.control.stage(MixedSourceHandoff {
                packet,
                at: at(500),
                input: note_on(61),
            });
            assert_eq!(
                staged,
                Err(MixedBridgeError::Fault(MixedBridgeFault::ForeignSource {
                    generation: packet.connection(),
                }))
            );
        }
        Cause::LostCommand | Cause::LostResult => {
            rig.send_all(
                FIRST,
                &[
                    message(400, note_on(61)),
                    message(410, note_off(60)),
                    frontier(600),
                ],
            );
            rig.send(SECOND, frontier(600));
        }
    }
    rig.service();
    rig.render_all(partition);
    rig.collect();
    rig.render_all(partition);
    rig.render_all(partition);
    let fault = rig.control.fault();
    let callbacks = rig
        .callbacks
        .iter()
        .map(|&(start, frames, result)| {
            (
                start,
                frames,
                result.map_err(MixedHostRenderError::transfer_view),
            )
        })
        .collect();
    let raw_refused = rig.raw_refused.clone();
    let errors = rig.errors.clone();
    Terminal {
        joined: rig.join(),
        fault,
        callbacks,
        raw_refused,
        errors,
        foreign,
        generations,
    }
}

impl Terminal {
    /// From the first failing callback on, every callback failed and was silent;
    /// earlier callbacks succeeded, and the successful prefix sounded.
    fn first_failing(&self) -> (usize, MixedTransferRenderError) {
        let failing = self
            .callbacks
            .iter()
            .position(|(_, _, rendered)| rendered.is_err())
            .expect("the fault reached the audio half");
        for (index, &(start, frames, rendered)) in self.callbacks.iter().enumerate() {
            let samples = self.joined.frame_samples(start..start + frames);
            if index < failing {
                assert!(rendered.is_ok(), "callback {index} succeeds");
                if start >= 128 {
                    assert!(
                        !silent(samples),
                        "callback {index} sounds after both onsets"
                    );
                }
            } else {
                assert!(
                    rendered.is_err() && silent(samples),
                    "callback {index} is silent"
                );
            }
        }
        assert!(
            !silent(self.joined.frame_samples(0..self.callbacks[failing].0)),
            "the successful prefix sounds"
        );
        let (start, _, rendered) = self.callbacks[failing];
        (start, rendered.expect_err("failing callback"))
    }

    fn assert_terminal_raw_custody(&self, start: usize) {
        assert!(
            self.joined.raw_halted && self.joined.host_halted,
            "terminal fault halts capture and both sources"
        );
        for receipt in &self.joined.receipts {
            let tick = match receipt.observation {
                InputObservation::Message { tick, .. } | InputObservation::Frontier { tick } => {
                    tick
                }
            };
            let expected = match receipt.observation {
                _ if tick >= InputTick::new(start as u64) => RawState::Cancelled,
                InputObservation::Message { .. } => {
                    RawState::Published(CaptureDisposition::Recorded)
                }
                InputObservation::Frontier { .. } => RawState::Fenced,
            };
            assert_eq!(receipt.state, expected, "{receipt:?}");
        }
    }
}

#[test]
fn every_terminal_cause_silences_the_host_from_its_callback_and_keeps_raw_custody() {
    for cause in [
        Cause::BridgeOrder,
        Cause::PreRingOrder,
        Cause::RetryExpired,
        Cause::RawAdmission,
        Cause::SourceHolds,
        Cause::ForeignSource,
        Cause::LostCommand,
        Cause::LostResult,
    ] {
        let run = terminal(cause, &[256]);
        let first = run.generations[FIRST];
        let (start, error) = run.first_failing();
        let expected = match cause {
            Cause::BridgeOrder => MixedBridgeFault::SourceOrder {
                connection: first,
                at: at(50),
                frontier: at(300),
            },
            Cause::PreRingOrder => MixedBridgeFault::Source {
                connection: first,
                fault: MixedSourceFault::PreRing(InputError::Order),
            },
            Cause::RetryExpired => MixedBridgeFault::Source {
                connection: first,
                fault: MixedSourceFault::RetryExpired,
            },
            Cause::RawAdmission => MixedBridgeFault::Source {
                connection: first,
                fault: MixedSourceFault::RawAdmission(run.raw_refused[0].2),
            },
            Cause::SourceHolds => MixedBridgeFault::SourceHolds {
                connection: first,
                onset: run.joined.packet_of(FIRST, 400),
            },
            Cause::ForeignSource => MixedBridgeFault::ForeignSource {
                generation: run.foreign.expect("foreign generation"),
            },
            Cause::LostCommand => MixedBridgeFault::Transfer(MixedTransferFault::Remote),
            Cause::LostResult => MixedBridgeFault::Transfer(MixedTransferFault::ResultProtocol {
                received: run
                    .joined
                    .teardown
                    .transfer()
                    .invalid_result()
                    .expect("the out-of-order result is retained"),
                expected: run.joined.teardown.transfer().lost().first().copied(),
            }),
        };
        assert_eq!(run.fault, Some(expected), "{cause:?}");
        assert_eq!(run.joined.teardown.fault(), Some(expected), "{cause:?}");
        // The first cause wins: every later call or report observes the halt.
        let faults = run
            .errors
            .iter()
            .filter(|error| matches!(error, MixedBridgeError::Fault(_)))
            .count();
        assert!(faults <= 1, "{cause:?} reported {:?}", run.errors);
        assert!(
            run.errors
                .iter()
                .all(|error| matches!(error, MixedBridgeError::Fault(fault) | MixedBridgeError::Halted(fault) if *fault == expected)),
            "{cause:?} reported {:?}",
            run.errors
        );
        let expected_error = match cause {
            Cause::LostCommand => MixedTransferRenderError::CommandOrder,
            _ => MixedTransferRenderError::RemoteHalt,
        };
        assert_eq!(error, expected_error, "{cause:?}");
        // Control-detected causes before the second callback silence it; a lost result
        // is found by the collection after it.
        let expected_start = if cause == Cause::LostResult { 512 } else { 256 };
        assert_eq!(start, expected_start, "{cause:?}");
        run.joined.assert_each_packet_once();
        // One terminal scope: capture ends exactly where sound does. Everything raw
        // stamped before the failing callback was captured; everything else, including
        // packets raw already held for that callback, was cancelled.
        run.assert_terminal_raw_custody(start);
        let custody = run.joined.teardown.transfer().custody();
        if matches!(cause, Cause::LostCommand | Cause::LostResult) {
            assert_eq!(custody, MixedTransferCustody::Lost, "{cause:?}");
            let lost = run.joined.teardown.transfer().lost();
            assert_eq!(lost.len(), 1);
            assert!(
                run.joined
                    .teardown
                    .held()
                    .iter()
                    .any(|(_, custody)| matches!(
                        custody,
                        MixedSourceCustody::Command {
                            id,
                            place: MixedCommandPlace::Lost
                        } if *id == lost[0]
                    ))
            );
        } else {
            assert_eq!(custody, MixedTransferCustody::Complete, "{cause:?}");
        }
    }
}

#[test]
fn every_terminal_cause_keeps_its_first_fault_quantum_across_callback_partitions() {
    let partitions: [&[usize]; 4] = [&[256], &[64, 64, 64, 64], &[128, 128], &[1, 37, 90, 128]];
    for cause in [
        Cause::BridgeOrder,
        Cause::PreRingOrder,
        Cause::RetryExpired,
        Cause::RawAdmission,
        Cause::SourceHolds,
        Cause::ForeignSource,
        Cause::LostCommand,
        Cause::LostResult,
    ] {
        let reference = terminal(cause, partitions[0]);
        let reference_fault = reference.fault.expect("terminal fault");
        let reference_start = reference.first_failing().0;
        reference.joined.assert_each_packet_once();
        reference.assert_terminal_raw_custody(reference_start);
        for &partition in &partitions[1..] {
            let run = terminal(cause, partition);
            let fault = run.fault.expect("terminal fault");
            match (reference_fault, fault) {
                (
                    MixedBridgeFault::SourceOrder {
                        at: reference_at,
                        frontier: reference_frontier,
                        ..
                    },
                    MixedBridgeFault::SourceOrder {
                        connection,
                        at,
                        frontier,
                    },
                ) => {
                    assert_eq!(connection, run.generations[FIRST]);
                    assert_eq!((at, frontier), (reference_at, reference_frontier));
                }
                (
                    MixedBridgeFault::Source {
                        fault: reference_cause,
                        ..
                    },
                    MixedBridgeFault::Source {
                        connection,
                        fault: actual,
                    },
                ) => {
                    assert_eq!(connection, run.generations[FIRST]);
                    assert_eq!(actual, reference_cause);
                }
                (
                    MixedBridgeFault::ForeignSource { .. },
                    MixedBridgeFault::ForeignSource { generation },
                ) => assert_eq!(Some(generation), run.foreign),
                (
                    MixedBridgeFault::SourceHolds {
                        onset: expected, ..
                    },
                    MixedBridgeFault::SourceHolds { connection, onset },
                ) => {
                    assert_eq!(connection, run.generations[FIRST]);
                    assert_eq!(onset.serial(), expected.serial());
                }
                (MixedBridgeFault::Transfer(expected), MixedBridgeFault::Transfer(actual)) => {
                    assert_eq!(actual, expected)
                }
                _ => panic!("{cause:?} {partition:?}: {fault:?} differs from {reference_fault:?}"),
            }
            let start = run.first_failing().0;
            assert_eq!(start, reference_start, "{cause:?} {partition:?}");
            run.joined.assert_each_packet_once();
            run.assert_terminal_raw_custody(start);
        }
    }
}

#[test]
fn an_oversized_terminal_callback_reports_the_halt_before_shape_and_stops_raw() {
    let mut rig = Rig::new(Setup::default());
    rig.send(FIRST, message(100, note_on(60)));
    rig.stage_all();
    let raw = admitted_in(&rig.admitted, rig.packet_of(FIRST, 100))
        .raw_id()
        .expect("raw capture runs");
    let source = rig.generation(FIRST);
    assert_eq!(
        rig.control
            .report_source_fault(source, MixedSourceFault::RetryExpired),
        MixedBridgeError::Fault(MixedBridgeFault::Source {
            connection: source,
            fault: MixedSourceFault::RetryExpired,
        })
    );
    assert_eq!(
        rig.render(BLOCK as usize + 1),
        Err(MixedHostRenderError::Direct(
            MixedTransferRenderError::RemoteHalt
        ))
    );
    assert_eq!(rig.raw_callbacks.last(), Some(&false));
    assert!(rig.host_halt.is_requested() && rig.raw.halt.is_requested());
    assert!(
        rig.raw_completions.occupied_len() > 0,
        "the raw halt produced a completion in this callback"
    );
    rig.collect_raw();
    assert_eq!(
        rig.receipts
            .iter()
            .find(|receipt| receipt.id == raw)
            .map(|receipt| receipt.state),
        Some(RawState::Cancelled)
    );
    assert!(silent(&rig.rendered));
}

// ---------------------------------------------------------------------------------------
// Rule 1.9: one script under four callback partitions. The control schedule is fixed to
// whole 1024-frame rounds, as the rule requires for collector timing.

/// Everything a run produced that must not depend on the partition. Raw IDs, command
/// IDs and identities are renamed by first appearance, since each run issues fresh
/// connection generations and a fresh identity table.
#[derive(Debug, PartialEq)]
struct Trace {
    reports: Vec<String>,
    splits: Vec<String>,
    fault: Option<MixedBridgeFault>,
    failing: Option<(usize, MixedTransferRenderError)>,
    held: Vec<String>,
    /// Every raw receipt, which joined teardown adds.
    raw: Vec<String>,
}

fn trace(rig: &Rig, joined: Option<&MixedBridgeTeardown>) -> Trace {
    let mut identities: Vec<NoteIdentity> = Vec::new();
    let mut name = |identity: NoteIdentity| {
        let index = identities
            .iter()
            .position(|seen| *seen == identity)
            .unwrap_or_else(|| {
                identities.push(identity);
                identities.len() - 1
            });
        format!("n{index}")
    };
    let packet_name = |packet: MixedSourcePacketId| {
        let admitted = admitted_in(&rig.admitted, packet);
        format!("s{}#{}", admitted.source, packet.serial())
    };
    let anonymous = |text: String| {
        text.replace(&format!("{:?}", rig.generation(FIRST)), "first")
            .replace(&format!("{:?}", rig.generation(SECOND)), "second")
    };
    let mut reports = Vec::new();
    let mut splits = Vec::new();
    for (packet, settled) in &rig.settled {
        match settled {
            Settled::Report(report) => {
                let outcome = match report.outcome() {
                    MixedBridgeOutcome::NoteOn(Ok(identity)) => format!("on {}", name(identity)),
                    MixedBridgeOutcome::NoteOn(Err(refused)) => format!("on refused {refused:?}"),
                    MixedBridgeOutcome::KeyUp { onset, result } => {
                        let retired = match result {
                            Ok(MixedKeyUpRetired::Released(identity)) => {
                                format!("released {}", name(identity))
                            }
                            other => format!("{other:?}"),
                        };
                        format!("up {} {retired}", packet_name(onset))
                    }
                    other => format!("{other:?}"),
                };
                reports.push(format!(
                    "{} #{} {} {} {} {outcome}",
                    packet_name(*packet),
                    report.id().get(),
                    report.nominal_at(),
                    report.stamp(),
                    report.executed_at(),
                ));
            }
            other => splits.push(anonymous(format!("{} {other:?}", packet_name(*packet)))),
        }
    }
    let failing = rig.callbacks.iter().find_map(|&(start, _, rendered)| {
        rendered.err().map(|error| (start, error.transfer_view()))
    });
    let held = joined.map_or_else(Vec::new, |teardown| {
        teardown
            .held()
            .iter()
            .map(|(packet, custody)| format!("{} {custody:?}", packet_name(*packet)))
            .collect()
    });
    Trace {
        reports,
        splits,
        fault: rig.control.fault().map(|fault| match fault {
            MixedBridgeFault::Transfer(fault) => MixedBridgeFault::Transfer(fault),
            other => other,
        }),
        failing,
        held,
        raw: Vec::new(),
    }
}

/// Three rounds of control then 1024 rendered frames each, then one silent round. The
/// third round's first command is lost by the host queue, so audio faults at the first
/// boundary of that round, 2048, whatever the partition.
fn partitioned(partition: &[usize]) -> (Trace, Vec<f32>) {
    assert_eq!(partition.iter().sum::<usize>(), 1_024);
    let mut setup = Setup {
        cells: 4,
        ..Setup::default()
    };
    setup.commands.lose_push = Some(11);
    let mut rig = Rig::new(setup);
    let rounds: [(&[InputObservation], &[InputObservation]); 3] = [
        (
            &[
                message(100, note_on(60)),
                message(300, bend(12_288)),
                message(600, note_off(60)),
                frontier(1_000),
            ],
            &[
                message(200, note_on(64)),
                message(400, note_on(65)),
                frontier(1_000),
            ],
        ),
        (
            &[
                message(1_124, note_on(62)),
                message(1_400, note_on(63)),
                message(1_500, note_on(66)),
                message(1_600, note_off(66)),
                message(1_700, note_off(62)),
                frontier(2_024),
            ],
            &[
                message(1_200, note_off(64)),
                message(1_300, note_off(65)),
                frontier(2_024),
            ],
        ),
        (
            &[message(2_148, note_off(63)), frontier(3_048)],
            &[message(2_200, note_on(67)), frontier(3_048)],
        ),
    ];
    for (first, second) in rounds {
        rig.send_all(FIRST, first);
        rig.send_all(SECOND, second);
        rig.service();
        rig.render_all(partition);
        rig.collect();
    }
    rig.render_all(partition);
    rig.collect();
    let rendered = rig.rendered.clone();
    let partial = trace(&rig, None);
    let callbacks = rig.callbacks.clone();
    let joined = rig.join();
    joined.assert_each_packet_once();
    // Rebuild the trace with the teardown's held source IDs named the same way.
    let held = joined
        .teardown
        .held()
        .iter()
        .map(|(packet, custody)| {
            let admitted = admitted_in(&joined.admitted, *packet);
            format!("s{}#{} {custody:?}", admitted.source, packet.serial())
        })
        .collect();
    drop(callbacks);
    let raw = raw_trace(&joined.admitted, &joined.receipts);
    (
        Trace {
            held,
            raw,
            ..partial
        },
        rendered,
    )
}

#[test]
fn one_script_keeps_identities_outcomes_and_fault_quantum_across_partitions() {
    let irregular = [1, 63, 130, 7, 300, 2, 64, 457];
    let partitions: [Vec<usize>; 4] = [
        vec![1_024],
        vec![Q; 1_024 / Q],
        vec![256; 4],
        irregular.to_vec(),
    ];
    let (reference, audio) = partitioned(&partitions[0]);
    assert_eq!(
        reference.failing,
        Some((2_048, MixedTransferRenderError::CommandOrder)),
        "the lost command faults audio at the third round's first boundary"
    );
    assert_eq!(
        reference.fault,
        Some(MixedBridgeFault::Transfer(MixedTransferFault::Remote))
    );
    assert!(
        reference
            .splits
            .iter()
            .any(|split| split.contains("StageFull")),
        "the script exercises a stage split: {:?}",
        reference.splits
    );
    assert!(!silent(&audio[..2_048]), "the first two rounds sound");
    assert!(
        reference
            .raw
            .iter()
            .any(|receipt| receipt.contains("Published(Recorded)"))
            && reference
                .raw
                .iter()
                .any(|receipt| receipt.contains("Cancelled")),
        "raw captured the first rounds and cancelled the faulted one: {:?}",
        reference.raw
    );
    assert!(
        silent(&audio[2_048..]),
        "every sample from the fault on is silent"
    );
    for partition in &partitions[1..] {
        let (run, samples) = partitioned(partition);
        assert_eq!(run, reference, "partition {partition:?} changed the trace");
        assert!(
            samples[..2_048]
                .iter()
                .zip(&audio[..2_048])
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "partition {partition:?} changed the audio of the successful callbacks"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Rule 1.10: a fault with packets in every custody state; teardown classifies each.

#[test]
fn a_fault_with_packets_in_every_custody_state_finds_each_source_id_exactly_once() {
    // The result queue really holds two results while reporting the admitted places, so
    // audio's third uncollected result push fails: a terminal `ResultPush`.
    let mut setup = Setup {
        cells: 6,
        ..Setup::default()
    };
    setup.results.reported_capacity = Some(places());
    setup.results.ring = HeapRb::new(2);
    let mut rig = Rig::new(setup);
    // Round one: one onset is served and its result settled; it stays open.
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(200)]);
    rig.send_all(SECOND, &[frontier(200)]);
    rig.service();
    rig.render(256).expect("first callback");
    rig.collect();
    let settled_onset = rig
        .report_of(rig.packet_of(FIRST, 100))
        .expect("settled onset");
    // Round two is staged whole before the pump: the open onset's protected cell and two
    // more onsets leave room for the bend but not for the onsets at 320 and 340, which
    // leave tombstones. Four commands go out; the key-up at 700 waits for the second
    // source's frontier in its cell.
    rig.send_all(
        FIRST,
        &[
            message(300, note_on(61)),
            message(310, note_on(62)),
            message(320, note_on(63)),
            message(330, bend(4_096)),
            message(340, note_on(64)),
            frontier(650),
            message(700, note_off(61)),
        ],
    );
    rig.send_all(SECOND, &[message(305, note_on(70)), frontier(600)]);
    rig.service();
    let tombstoned = rig.packet_of(FIRST, 320);
    assert_eq!(
        rig.settled_of(tombstoned),
        [Settled::Stage(Ok(MixedStaged::Split(
            MixedSplit::StageFull
        )))]
    );
    assert_eq!(rig.forwarded.len(), 5);
    // Source-side custody the raw owner never sees: a full ring and one owned retry.
    for tick in 701..=716 {
        rig.send(FIRST, frontier(tick));
    }
    rig.send(FIRST, frontier(717));
    assert!(rig.retries[FIRST].is_some(), "the producer owns a retry");
    // Audio pushes results 2 and 3, fails on 4 and leaves command 5 unserved. The
    // terminal callback halts the raw owner and both producers with it.
    rig.render(256).expect_err("the third result push fails");
    rig.send(SECOND, frontier(800));
    rig.collect();
    let fault = rig.control.fault();
    let joined = rig.join();
    // Raw classification: the onset captured before the fault is recorded; everything
    // after it, including the packets raw already held for the failing callback, was
    // cancelled when the terminal callback halted capture before raw rendered it.
    assert!(joined.raw_halted, "the terminal callback halted capture");
    for (packet, state, sound) in joined.ledger() {
        let expected = if packet == settled_onset.packet().expect("a source report") {
            RawState::Published(CaptureDisposition::Recorded)
        } else {
            RawState::Cancelled
        };
        assert_eq!(state, expected, "packet {packet:?} with sound {sound:?}");
    }
    assert_eq!(
        joined.teardown.transfer().audio_fault(),
        Some(MixedTransferRenderError::ResultPush)
    );
    assert_eq!(
        fault,
        Some(MixedBridgeFault::Transfer(MixedTransferFault::Remote))
    );
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    joined.assert_each_packet_once();
    let held = joined.teardown.held();
    let place = |frames: u64| {
        let packet = joined
            .admitted
            .iter()
            .find(|admitted| admitted.at(FIRST, frames) || admitted.at(SECOND, frames))
            .expect("admitted")
            .packet;
        held.iter()
            .find(|(held, _)| *held == packet)
            .map(|(_, custody)| *custody)
    };
    let command = |frames: u64| match place(frames) {
        Some(MixedSourceCustody::Command { place, .. }) => place,
        other => panic!("{frames} is not a command: {other:?}"),
    };
    assert_eq!(command(300), MixedCommandPlace::Uncollected);
    assert_eq!(command(305), MixedCommandPlace::Uncollected);
    assert_eq!(command(310), MixedCommandPlace::PendingResult);
    assert_eq!(command(330), MixedCommandPlace::Unserved);
    assert_eq!(place(700), Some(MixedSourceCustody::Staged));
    assert_eq!(place(100), None, "the settled onset left the bridge");
    let second_tombstone = joined.packet_of(FIRST, 340);
    assert_eq!(
        joined.teardown.tombstones(),
        [
            (tombstoned, MixedOnsetRefusal::StageFull),
            (second_tombstone, MixedOnsetRefusal::StageFull),
        ]
    );
    // Every merger handle is named by its onset: the settled one and the three in flight.
    let open: Vec<_> = joined
        .teardown
        .open()
        .iter()
        .map(|(onset, state)| (onset.expect("named onset"), *state))
        .collect();
    assert!(open.contains(&(
        settled_onset.packet().expect("a source report"),
        MixedTransferHandleState::Open
    )));
    assert_eq!(open.len(), 4);
    // Ring and producer custody the raw owner never saw, and an observation the halted
    // host refused at its producer.
    assert_eq!(joined.unexamined.len(), 16);
    assert_eq!(joined.retired, [frontier(717)]);
    assert_eq!(joined.halted_sends, [(SECOND, frontier(800))]);
}

#[test]
fn a_failed_command_push_leaves_its_source_id_unsent_and_the_older_ones_unserved() {
    let mut setup = Setup::default();
    setup.commands.reported_capacity = Some(places());
    setup.commands.ring = HeapRb::new(2);
    let mut rig = Rig::new(setup);
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(200, note_on(61)),
            message(300, note_on(62)),
            frontier(400),
        ],
    );
    rig.send(SECOND, frontier(400));
    rig.service();
    assert_eq!(rig.forwarded.len(), 2);
    // The pump that met the failed push reports the fault; later calls see the halt.
    assert!(matches!(
        rig.errors.as_slice(),
        [
            MixedBridgeError::Fault(MixedBridgeFault::Transfer(
                MixedTransferFault::CommandPush { .. }
            )),
            MixedBridgeError::Halted(_),
        ]
    ));
    rig.render(256).expect_err("the halt silences audio");
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    let places: Vec<_> = joined
        .teardown
        .held()
        .iter()
        .map(|(_, custody)| *custody)
        .collect();
    assert!(matches!(
        places.as_slice(),
        [
            MixedSourceCustody::Command {
                place: MixedCommandPlace::Unserved,
                ..
            },
            MixedSourceCustody::Command {
                place: MixedCommandPlace::Unserved,
                ..
            },
            MixedSourceCustody::Command {
                place: MixedCommandPlace::Unsent,
                ..
            },
        ]
    ));
    // The unsent onset's merger handle is still named by its source onset.
    assert_eq!(joined.teardown.open().len(), 3);
    assert!(
        joined
            .teardown
            .open()
            .iter()
            .all(|(onset, _)| onset.is_some())
    );
}

// ---------------------------------------------------------------------------------------
// A quiescent full command ring splits the selected onset rather than deadlocking it in
// front of the key-ups whose places it is waiting for.

#[test]
fn a_quiescent_full_command_ring_splits_the_selected_onset_and_consumes_its_key_up() {
    let mut rig = Rig::new(Setup {
        cells: 40,
        ..Setup::default()
    });
    // Thirty-two onsets: thirty-one reserve every place but one, so the
    // thirty-second can never take its own place and its key-up's.
    for note in 0..16_u8 {
        let base = 10 + 2 * u64::from(note);
        rig.send(FIRST, message(base, note_on(30 + note)));
        rig.send(SECOND, message(base + 1, note_on(60 + note)));
        rig.service();
    }
    // The last onset's pairing place is already redeemed when its key-up reaches the
    // stage. A later command-ring split must use that staged key-up's own cell.
    rig.send(SECOND, message(43, note_off(75)));
    rig.stage_all();
    rig.send(FIRST, frontier(100));
    // Each round serves and collects every outstanding command before the next pump,
    // so the ring is quiescent whenever the merger's places run out.
    let last = rig.packet_of(SECOND, 41);
    for _ in 0..8 {
        rig.service();
        rig.render(256).expect("callback");
        rig.collect();
        if !rig.settled_of(last).is_empty() {
            break;
        }
    }
    // While any command was outstanding the head waited; only a quiescent ring splits.
    assert!(rig.waits.contains(&MixedBridgeWait::Commands));
    assert_eq!(
        rig.settled_of(last),
        [Settled::Pump(MixedSplit::CommandsFull)]
    );
    assert_eq!(rig.forwarded.len(), 31);
    // Its already-staged key-up carries the refusal; every other key-up keeps its place.
    for note in 0..16_u8 {
        rig.send(FIRST, message(200 + u64::from(note), note_off(30 + note)));
        if note != 15 {
            rig.send(SECOND, message(200 + u64::from(note), note_off(60 + note)));
        }
    }
    rig.send(FIRST, frontier(1_000));
    rig.send(SECOND, frontier(1_000));
    for _ in 0..4 {
        rig.service();
        rig.render(256).expect("callback");
        rig.collect();
    }
    let last_up = rig.packet_of(SECOND, 43);
    assert_eq!(
        rig.settled_of(last_up),
        [Settled::Pump(MixedSplit::AfterRefusedOnset {
            onset: last,
            reason: MixedOnsetRefusal::CommandsFull,
        })]
    );
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert!(joined.teardown.tombstones().is_empty());
}

// ---------------------------------------------------------------------------------------
// Threads: each source thread owns only its producer, the audio thread only the audio
// half and its wrapper, and the control thread everything else.

fn source_script(source: usize) -> Vec<InputObservation> {
    let mut script = Vec::new();
    let offset = 7 * source as u64;
    for note in 0..12_u64 {
        let base = 100 + 50 * note + offset;
        let raw = 48 + 12 * source as u8 + note as u8;
        script.push(message(base, note_on(raw)));
        if note % 3 == 0 {
            script.push(message(base + 10, bend(8_192 + 512 * note as u16)));
        }
        script.push(message(base + 20, note_off(raw)));
        script.push(frontier(base + 40));
    }
    script.push(frontier(10_000));
    script
}

#[test]
fn source_audio_and_control_threads_join_in_order_with_complete_custody() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    let mut rig = Rig::new(Setup {
        cells: 6,
        peer_wait: 1 << 20,
        ..Setup::default()
    });
    let stop = Arc::new(AtomicBool::new(false));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    // The simulated source clock is locked to audio: a callback renders only frames whose
    // raw packets control has already sent, as a real host's source timestamps would
    // guarantee. Without it the free-running thread would make raw delivery late.
    let paced = Arc::new(AtomicU64::new(0));
    let mut callback = rig.callback.take().expect("host callback");
    let audio_stop = Arc::clone(&stop);
    let audio_paced = Arc::clone(&paced);
    let audio_thread = std::thread::spawn(move || {
        let mut callbacks = 0_usize;
        let mut samples = vec![0.0_f32; 256];
        while !audio_stop.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < deadline,
                "threaded host progress deadline"
            );
            let next = 256 * (callbacks as u64 + 1);
            if next > audio_paced.load(Ordering::Acquire) {
                // Between callbacks the audio half still takes raw packets off its ring.
                callback.service_between_callbacks();
                std::thread::yield_now();
                continue;
            }
            let rendered = callback.render(&mut samples);
            assert_eq!(
                rendered,
                Rendered {
                    mixed: Ok(()),
                    raw_ok: true
                },
                "fault-free callback"
            );
            callbacks += 1;
            std::thread::yield_now();
        }
        (callback, callbacks)
    });
    let sources: Vec<_> = [FIRST, SECOND]
        .into_iter()
        .map(|source| {
            let mut producer = rig.producers[source].take().expect("producer");
            std::thread::spawn(move || {
                for observation in source_script(source) {
                    let mut attempt = producer.submit_owned(observation);
                    while let Err(SourceOwnedSendError::Retry(token)) = attempt {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "source retry deadline"
                        );
                        std::thread::yield_now();
                        attempt = producer.retry_owned(token);
                    }
                    let (_attempt, _queue) = attempt
                        .expect("the source ring accepts every observation")
                        .parts();
                }
                producer
            })
        })
        .collect();
    // Everything both sources' raw frontiers cover has been sent to raw once nothing is
    // left unsent.
    let pace = |rig: &Rig| {
        if rig.raw_unsent.is_some() {
            return;
        }
        let frontier = |source| {
            rig.admitted
                .iter()
                .filter_map(|admitted| match admitted.observation {
                    InputObservation::Frontier { tick } if admitted.source == source => {
                        Some(tick.as_u64())
                    }
                    _ => None,
                })
                .max()
                .unwrap_or(0)
        };
        paced.store(frontier(FIRST).min(frontier(SECOND)), Ordering::Release);
    };
    while !sources.iter().all(std::thread::JoinHandle::is_finished) {
        rig.service();
        pace(&rig);
        std::thread::yield_now();
    }
    for (source, handle) in sources.into_iter().enumerate() {
        rig.producers[source] = Some(handle.join().expect("source thread"));
    }
    let expected: usize = [FIRST, SECOND]
        .into_iter()
        .map(|source| {
            source_script(source)
                .iter()
                .filter(|observation| matches!(observation, InputObservation::Message { .. }))
                .count()
        })
        .sum();
    // Sound settles when audio serves a command; raw settles once audio has rendered the
    // message's stamp. Both must finish before the audio thread stops.
    let raw_settled = |rig: &Rig| {
        rig.receipts
            .iter()
            .filter(|receipt| matches!(receipt.observation, InputObservation::Message { .. }))
            .count()
    };
    while (rig.settled.len() < expected || raw_settled(&rig) < expected)
        && !audio_thread.is_finished()
    {
        rig.service();
        pace(&rig);
        assert!(rig.errors.is_empty(), "{:?}", rig.errors);
        std::thread::yield_now();
    }
    stop.store(true, Ordering::Release);
    let (callback, callbacks) = audio_thread.join().expect("audio thread");
    assert!(callbacks > 0);
    rig.callback = Some(callback);
    assert_eq!(rig.messages().count(), expected);
    assert!(rig.raw_refused.is_empty());
    // Raw pacing does not pace mixed acceptance: control may submit enough short
    // notes before publication to fill the bounded ingress queue. A named slot
    // drop is legal, but its exact tombstone must consume exactly its own key-up.
    for (packet, settled) in &rig.settled {
        let Settled::Report(onset_report) = settled else {
            continue;
        };
        if let MixedBridgeOutcome::KeyUp { onset, .. } = onset_report.outcome() {
            let paired_onset = rig
                .report_of(onset)
                .expect("every reported key-up has its onset report");
            assert!(
                matches!(paired_onset.outcome(), MixedBridgeOutcome::NoteOn(_)),
                "key-up {packet:?} must name a reported onset, not another outcome"
            );
        }
        let MixedBridgeOutcome::NoteOn(onset_result) = onset_report.outcome() else {
            continue;
        };
        let paired: Vec<_> = rig
            .settled
            .iter()
            .filter_map(|(_, settled)| {
                let Settled::Report(report) = settled else {
                    return None;
                };
                match report.outcome() {
                    MixedBridgeOutcome::KeyUp { onset, result } if onset == *packet => Some(result),
                    _ => None,
                }
            })
            .collect();
        assert_eq!(
            paired.len(),
            1,
            "one exact reported key-up for onset {packet:?}"
        );
        let expected = match onset_result {
            Ok(identity) => MixedKeyUpRetired::Released(identity),
            Err(
                refused @ synth_engine_v2::ingress::IngressRefused::Dropped {
                    resource: synth_engine_v2::ingress::ExhaustedResource::Slot,
                },
            ) => MixedKeyUpRetired::AfterRefusal(refused),
            Err(other) => panic!("unexpected threaded onset refusal for {packet:?}: {other:?}"),
        };
        assert_eq!(
            paired[0],
            Ok(expected),
            "exact paired retirement for onset {packet:?}"
        );
    }
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(joined.teardown.fault(), None);
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert!(joined.unexamined.is_empty() && joined.retired.is_empty());
    assert!(!joined.raw_halted, "raw capture ran to the join's stop");
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "packet {packet:?} was captured on time beside the sound path"
        );
    }
}

/// Exercise the stopped-owner reset with the same two source rings and raw recorder as
/// the ordinary bridge run. The direct-owner reference receives identical observations;
/// the whole-callback runs also share their callback boundaries.
fn common_host_reset_run(reset: bool, irregular: bool) -> (Vec<u32>, Vec<String>) {
    common_host_reset_with_plan(reset, irregular, plan())
}

fn common_host_reset_with_plan(
    reset: bool,
    irregular: bool,
    successor_plan: CompiledPlan,
) -> (Vec<u32>, Vec<String>) {
    const B: SampleTime = SampleTime::new(448);
    let mut rig = Rig::new(Setup::default());
    if reset {
        let callback = rig.callback.as_mut().expect("callback");
        let MixedHostAudio::Direct(audio, port) = callback.mixed.take().expect("direct owner")
        else {
            panic!("fresh rig is direct");
        };
        let owner = MixedBridgeResetOwner::new(*audio, port).expect("matching audio port");
        let mut wrapper = MixedBridgeResetAudio::new(owner);
        wrapper.schedule_stop(at(192)).expect("compiled Stop");
        callback.mixed = Some(MixedHostAudio::Reset(Box::new(wrapper)));
    }

    rig.render(Q).expect("start raw capture and mixed plan");
    rig.collect();

    rig.send(FIRST, message(100, note_on(60)));
    rig.send(FIRST, frontier(150));
    rig.send(SECOND, frontier(150));
    rig.service();
    while rig.control.audio_boundary() < at(320) {
        if irregular {
            rig.render(7).expect("first callback part");
            rig.render(57).expect("second callback part");
        } else {
            rig.render(Q).expect("whole callback");
        }
        rig.collect();
    }
    assert_eq!(rig.control.audio_boundary(), at(320));
    assert_eq!(
        rig.output_frames(),
        384,
        "the mixed output lags by one quantum"
    );

    let mut next_port = None;
    if reset {
        let MixedBridgeArmed {
            control,
            audio,
            control_port,
            audio_port,
        } = {
            let (prepared, candidate) = prepared_plan_at(successor_plan, B);
            arm_prepared(&rig.raw, Setup::default(), prepared, candidate, profile())
        };
        rig.control.seal_reset(control, B).expect("sealed bridge");
        assert_eq!(rig.control.reset_ready(), Some(B));
        let successor = MixedBridgeResetOwner::new(audio, audio_port).expect("successor port");
        let callback = rig.callback.as_mut().expect("callback");
        let Some(MixedHostAudio::Reset(wrapper)) = callback.mixed.as_mut() else {
            panic!("reset wrapper");
        };
        wrapper
            .offer_reset(successor, B)
            .expect("stopped owner accepts reset");
        next_port = Some(control_port);
    }

    // The second source's onset enters while the first source promises nothing before
    // 480. The reset bridge stages it during the seal; raw capture still receives it.
    rig.send(FIRST, frontier(480));
    rig.send(SECOND, message(420, note_on(62)));
    rig.send(SECOND, frontier(480));
    rig.service();
    if reset {
        assert_eq!(
            rig.waits.last(),
            Some(&MixedBridgeWait::ResetSealed { at: B })
        );
    }
    while rig.control.audio_boundary() < B {
        if irregular {
            rig.render(9).expect("first callback part");
            rig.render(55).expect("second callback part");
        } else {
            rig.render(Q).expect("whole callback");
        }
        rig.collect();
    }
    assert_eq!(rig.output_frames(), 512, "the reset opens at output 512");
    rig.render(8).expect("open reset boundary");
    let reset_receipt = if reset {
        let callback = rig.callback.as_mut().expect("callback");
        let Some(MixedHostAudio::Reset(wrapper)) = callback.mixed.as_mut() else {
            panic!("reset wrapper");
        };
        let MixedBridgeResetState::Fading { receipt, .. } = wrapper.state() else {
            panic!("reset did not enter its fade: {:?}", wrapper.state());
        };
        let ended: Vec<_> = receipt.ended().collect();
        assert!(matches!(
            ended.as_slice(),
            [MixedBridgeResetEnd::Tombstoned(_)]
        ));
        let old = rig
            .control
            .commit_reset(receipt)
            .expect("commit bridge handoff");
        rig.reset_old.push((
            old,
            std::mem::replace(&mut rig.control_port, next_port.expect("successor port")),
        ));
        Some(receipt)
    } else {
        None
    };

    rig.send(FIRST, message(530, note_off(60)));
    rig.send(FIRST, message(540, note_on(60)));
    rig.send(FIRST, message(550, note_off(60)));
    rig.send(FIRST, frontier(610));
    rig.send(SECOND, message(570, note_off(62)));
    rig.send(SECOND, frontier(610));
    rig.service();
    // Finish the old owner's fade and serve both sources' post-reset key-ups.
    if irregular {
        rig.render(5).expect("fade part");
        rig.render(51).expect("fade remainder");
    } else {
        rig.render(56).expect("fade remainder");
    }
    rig.collect();
    for _ in 0..3 {
        rig.service();
        if irregular {
            rig.render(11).expect("later callback part");
            rig.render(53).expect("later callback remainder");
        } else {
            rig.render(Q).expect("later callback");
        }
        rig.collect();
    }

    let audio: Vec<_> = rig.rendered.iter().map(|sample| sample.to_bits()).collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(
        !joined.host_halted && !joined.raw_halted,
        "host={}, raw={}, errors={:?}, raw={:?}, receipts={:?}",
        joined.host_halted,
        joined.raw_halted,
        joined.errors,
        raw_trace(&joined.admitted, &joined.receipts),
        joined.receipts
    );
    assert!(
        joined.errors.is_empty(),
        "bridge errors: {:?}",
        joined.errors
    );
    assert!(joined.teardown.held().is_empty());
    assert_eq!(joined.teardown.fault(), None);
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw packet {packet:?} did not record across reset"
        );
    }
    let old_on = joined.packet_of(FIRST, 100);
    let old_off = joined.packet_of(FIRST, 530);
    let sealed_on = joined.packet_of(SECOND, 420);
    let new_on = joined.packet_of(FIRST, 540);
    let new_off = joined.packet_of(FIRST, 550);
    let sealed_off = joined.packet_of(SECOND, 570);
    assert!(matches!(
        joined.report_of(old_on).expect("old onset").outcome(),
        MixedBridgeOutcome::NoteOn(Ok(_))
    ));
    for packet in [sealed_on, new_on] {
        let report = joined.report_of(packet).expect("successor onset");
        assert!(matches!(
            report.outcome(),
            MixedBridgeOutcome::NoteOn(Ok(_))
        ));
        if reset {
            assert!(report.executed_at() >= B);
        }
    }
    for (up, onset) in [(new_off, new_on), (sealed_off, sealed_on)] {
        assert!(matches!(
            joined.report_of(up).expect("successor key-up").outcome(),
            MixedBridgeOutcome::KeyUp { onset: found, result: Ok(_) } if found == onset
        ));
    }
    if let Some(receipt) = reset_receipt {
        assert_eq!(receipt.at(), B);
        assert_eq!(receipt.ended_at(), at(512));
        assert_eq!(
            joined.sound(old_off),
            Settled::Stage(Ok(MixedStaged::Split(MixedSplit::AfterRefusedOnset {
                onset: old_on,
                reason: MixedOnsetRefusal::Reset,
            })))
        );
        let [retired] = joined.retired_reset.as_slice() else {
            panic!("one retired owner joined");
        };
        assert_eq!(retired.receipt(), receipt);
        assert!(retired.bridge().held().is_empty());
        assert_eq!(retired.bridge().fault(), None);
        assert!(matches!(
            retired.handles(),
            [(packet, MixedRetiredHandle::Ended(MixedBridgeResetEnd::Tombstoned(_)))]
                if *packet == old_on
        ));
    } else {
        assert!(joined.retired_reset.is_empty());
        assert!(joined.report_of(old_off).is_some());
    }
    (audio, raw_trace(&joined.admitted, &joined.receipts))
}

#[test]
fn stopped_reset_preserves_two_source_raw_capture_and_partitioned_sound_custody() {
    let (direct_audio, raw_reference) = common_host_reset_run(false, false);
    let (whole_audio, whole_raw) = common_host_reset_run(true, false);
    let (irregular_audio, irregular_raw) = common_host_reset_run(true, true);
    // The mixed output lags its engine clock by one quantum. Output [0, 256)
    // precedes the Stop at engine sample 192; output [448, 512) is the old
    // owner's final quantum before the reset at engine sample 448.
    assert_eq!(
        &whole_audio[..256],
        &direct_audio[..256],
        "the compiled sound before Stop changed"
    );
    for (name, window) in [
        (
            "old owner after Stop and before reset",
            &whole_audio[448..512],
        ),
        ("successor after reset fade", &whole_audio[576..640]),
    ] {
        assert!(
            window.iter().any(|bits| bits & 0x7fff_ffff != 0),
            "{name} was silent"
        );
    }
    assert_eq!(whole_raw, raw_reference, "reset changed raw capture");
    assert_eq!(
        irregular_raw, raw_reference,
        "partition changed raw capture"
    );
    assert_eq!(
        irregular_audio, whole_audio,
        "partition changed reset audio"
    );
}

// ---------------------------------------------------------------------------------------
// Later compiled activation through the concrete host.

/// Render through `partition`'s callback sizes in turn until the output reaches `to`'s
/// engine sample one quantum later, each callback cut short at that point. Every
/// partition then reaches the same host operation points with the same audio boundary.
fn advance(rig: &mut Rig, to: SampleTime, partition: &[usize], next: &mut usize) {
    let output = usize::try_from(to.as_u64()).expect("fits") + Q;
    while rig.output_frames() < output {
        let chunk = partition[*next % partition.len()].min(output - rig.output_frames());
        *next += 1;
        rig.render(chunk).expect("whole callback");
        rig.collect();
    }
    assert_eq!(rig.control.audio_boundary(), to);
}

impl Rig {
    fn direct_audio(&mut self) -> &mut MixedTransferAudio {
        match self
            .callback
            .as_mut()
            .expect("callback present")
            .mixed
            .as_mut()
            .expect("mixed owner present")
        {
            MixedHostAudio::Direct(audio, _) => audio,
            MixedHostAudio::Reset(_) => panic!("this host renders its owner directly"),
        }
    }
}

fn adopted(slot: MixedActivationSlot) -> (MixedActivationReceipt, MixedActivationAdopted) {
    match slot {
        MixedActivationSlot::Retired(receipt) => match receipt.outcome() {
            MixedActivationOutcome::Adopted(adopted) => (receipt, adopted),
            other => panic!("the candidate did not adopt: {other:?}"),
        },
        other => panic!("no retired candidate: {other:?}"),
    }
}

/// One script on the two-source host, with or without its compiled transport. With it:
/// a seek at 256 adopts beside two held live notes; a Start at 512 resumes the program a
/// Stop at 384 suspended, after the pending Stop first refused it; a seek offered for 704
/// is cancelled by a Stop at 640 and stays uncollected until joined teardown. A foreign
/// offer, a duplicate, an uncollected retired candidate and a stale one are each refused
/// and handed back whole; a foreign control refuses a retired candidate and hands it
/// back. Live note ingress and raw capture run throughout.
fn activation_run(activate: bool, partition: &[usize]) -> (Vec<u32>, Vec<String>) {
    fn require_send<T: Send>() {}
    require_send::<MixedActivationOffer>();
    require_send::<MixedActivationRetired>();

    let initial = ActivationSequence::INITIAL.next().expect("armed sequence");
    let seek = initial.next().expect("seek sequence");
    let start = seek.next().expect("start sequence");
    let mut next = 0;
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    assert_eq!(rig.control.audio_boundary(), SampleTime::ZERO);
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(150)]);
    rig.send_all(SECOND, &[message(110, note_on(64)), frontier(150)]);
    rig.service();
    advance(&mut rig, at(192), partition, &mut next);

    // The seek is prepared and sealed by control, then carried to audio by the host.
    let other = arm_at(&rig.raw, Setup::default(), at(64));
    let mut other_control = other.control;
    let mut stale = None;
    if activate {
        let offer = rig
            .control
            .prepare_activation(
                at(256),
                PlanPosition::ZERO,
                at(256),
                MixedActivationKind::Seek,
            )
            .expect("seek prepares");
        assert_eq!(offer.supersedes(), initial);
        let duplicate = rig
            .control
            .prepare_activation(
                at(256),
                PlanPosition::ZERO,
                at(256),
                MixedActivationKind::Seek,
            )
            .expect("an identical seek prepares from the same in-force sequence");
        let foreign = other_control
            .prepare_activation(
                at(256),
                PlanPosition::ZERO,
                at(256),
                MixedActivationKind::Seek,
            )
            .expect("another transfer's seek prepares");
        let audio = rig.direct_audio();
        audio.offer_activation(offer).expect("audio takes the seek");
        let (reason, duplicate) = audio
            .offer_activation(duplicate)
            .expect_err("one slot holds one candidate");
        assert_eq!(reason, MixedActivationRefusal::AlreadyPending);
        let (reason, foreign) = audio
            .offer_activation(foreign)
            .expect_err("another transfer's offer is refused");
        assert_eq!(reason, MixedActivationRefusal::ForeignTransfer);
        assert_eq!(
            foreign.effective(),
            at(256),
            "the foreign offer came back whole"
        );
        assert_eq!(
            audio.activation(),
            MixedActivationSlot::Pending {
                supersedes: initial,
                effective: at(256),
                kind: MixedActivationKind::Seek,
                loop_interval: None,
            }
        );
        stale = Some(duplicate);
    }
    rig.send_all(SECOND, &[message(300, note_off(64)), frontier(330)]);
    rig.send(FIRST, frontier(330));
    rig.service();
    advance(&mut rig, at(320), partition, &mut next);

    if activate {
        let audio = rig.direct_audio();
        let (receipt, seeked) = adopted(audio.activation());
        assert_eq!(receipt.sequence(), seek);
        assert_eq!(seeked.effective(), at(256));
        assert_eq!(seeked.anchor().time(), at(256));
        assert_eq!(seeked.anchor().position(), PlanPosition::ZERO);
        assert!(!seeked.stopped());
        assert_eq!(
            seeked.released(),
            HeldNoteCount::measured(1),
            "the boundary release ends the compiled note and no live one"
        );
        let (reason, stale_offer) = audio
            .offer_activation(stale.take().expect("duplicate"))
            .expect_err("the retired seek still holds the slot");
        assert_eq!(reason, MixedActivationRefusal::RetiredUncollected);
        let retired = audio.take_retired_activation().expect("retired seek");
        assert_eq!(audio.activation(), MixedActivationSlot::Empty);
        let refused = other_control
            .collect_activation(retired)
            .expect_err("another control refuses the retired seek");
        assert_eq!(
            refused.reason(),
            MixedActivationCollectError::ForeignTransfer
        );
        let collected = rig
            .control
            .collect_activation(refused.into_retired())
            .expect("its own control collects it");
        assert_eq!(collected, receipt);
        let (reason, _stale) = rig
            .direct_audio()
            .offer_activation(stale_offer)
            .expect_err("a candidate of the superseded sequence is stale");
        assert_eq!(
            reason,
            MixedActivationRefusal::Superseded {
                supersedes: initial,
                in_force: seek,
            }
        );
        // The stale offer is dropped here, off the audio thread; it minted nothing.

        assert_eq!(
            rig.direct_audio().schedule_stop(at(384)),
            Ok(MixedBridgeStopAccepted::Scheduled)
        );
        let offer = rig
            .control
            .prepare_activation(
                at(512),
                PlanPosition::new(10),
                at(512),
                MixedActivationKind::Start,
            )
            .expect("Start prepares");
        let (reason, offer) = rig
            .direct_audio()
            .offer_activation(offer)
            .expect_err("a pending Stop before the boundary refuses the Start");
        assert_eq!(
            reason,
            MixedActivationRefusal::StopPending {
                stop: at(384),
                effective: at(512),
            }
        );
        stale = Some(offer);
    }
    drop(other_control);
    advance(&mut rig, at(448), partition, &mut next);

    if activate {
        rig.direct_audio()
            .offer_activation(stale.take().expect("returned Start"))
            .expect("the stopped program takes the same Start");
    }
    advance(&mut rig, at(576), partition, &mut next);
    assert_eq!(
        rig.holds(),
        EventCount::measured(1),
        "the first source's note is held"
    );

    if activate {
        let audio = rig.direct_audio();
        let (receipt, started) = adopted(audio.activation());
        assert_eq!(receipt.kind(), MixedActivationKind::Start);
        assert_eq!(receipt.supersedes(), seek);
        assert_eq!(started.effective(), at(512));
        assert_eq!(
            started.released(),
            HeldNoteCount::NONE,
            "Stop already ended it"
        );
        assert!(!started.stopped(), "Start resumes the program");
        let retired = audio.take_retired_activation().expect("retired Start");
        assert_eq!(
            rig.control.collect_activation(retired).expect("collected"),
            receipt
        );
        let offer = rig
            .control
            .prepare_activation(
                at(704),
                PlanPosition::ZERO,
                at(704),
                MixedActivationKind::Seek,
            )
            .expect("later seek prepares");
        assert_eq!(offer.supersedes(), start);
        let audio = rig.direct_audio();
        audio.offer_activation(offer).expect("audio takes the seek");
        assert_eq!(
            audio.schedule_stop(at(640)),
            Ok(MixedBridgeStopAccepted::Scheduled)
        );
    }
    rig.send_all(FIRST, &[message(720, note_off(60)), frontier(750)]);
    rig.send(SECOND, frontier(750));
    rig.service();
    advance(&mut rig, at(832), partition, &mut next);
    if activate {
        let MixedActivationSlot::Retired(receipt) = rig.direct_audio().activation() else {
            panic!("the Stop retires the pending seek");
        };
        assert_eq!(
            receipt.outcome(),
            MixedActivationOutcome::Cancelled { stop: at(640) }
        );
    }
    rig.service();
    advance(&mut rig, at(960), partition, &mut next);
    assert_eq!(rig.holds(), EventCount::NONE);

    let audio: Vec<_> = rig.rendered.iter().map(|sample| sample.to_bits()).collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(
        !joined.host_halted && !joined.raw_halted && joined.errors.is_empty(),
        "errors: {:?}",
        joined.errors
    );
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert_eq!(joined.teardown.fault(), None);
    let transfer = joined.teardown.transfer();
    assert_eq!(transfer.custody(), MixedTransferCustody::Complete);
    assert_eq!(transfer.holds_outstanding(), EventCount::NONE);
    if activate {
        assert!(
            matches!(
                transfer.activation(),
                MixedActivationSlot::Retired(receipt)
                    if receipt.outcome() == MixedActivationOutcome::Cancelled { stop: at(640) }
            ),
            "joined teardown names the uncollected cancelled seek"
        );
    } else {
        assert_eq!(transfer.activation(), MixedActivationSlot::Empty);
    }
    for (source, on, off) in [(FIRST, 100, 720), (SECOND, 110, 300)] {
        let onset = joined
            .report_of(joined.packet_of(source, on))
            .expect("onset");
        let identity = identity_of(onset);
        assert!(
            matches!(
                joined.report_of(joined.packet_of(source, off)).expect("key-up").outcome(),
                MixedBridgeOutcome::KeyUp { result: Ok(MixedKeyUpRetired::Released(ended)), .. }
                    if ended == identity
            ),
            "the live note held across activation ends at its own key-up"
        );
    }
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw packet {packet:?} did not record across activation"
        );
    }
    (audio, raw_trace(&joined.admitted, &joined.receipts))
}

#[test]
fn later_compiled_activation_crosses_the_two_source_host_beside_live_notes_and_raw_capture() {
    let (reference_audio, reference_raw) = activation_run(false, &[Q]);
    let (whole_audio, whole_raw) = activation_run(true, &[Q]);
    let (irregular_audio, irregular_raw) = activation_run(true, &[37, 91, 13, 5, 110]);
    assert_eq!(whole_raw, reference_raw, "activation changed raw capture");
    assert_eq!(
        irregular_raw, reference_raw,
        "partition changed raw capture"
    );
    assert_eq!(irregular_audio.len(), whole_audio.len());
    assert_eq!(
        irregular_audio
            .iter()
            .zip(&whole_audio)
            .position(|(a, b)| a != b),
        None,
        "partition changed activation audio"
    );
    // Output lags the engine clock by one quantum. Before the seek at engine sample 256
    // the activations change nothing; after it the compiled note restarts.
    assert_eq!(whole_audio[..320], reference_audio[..320]);
    assert_ne!(whole_audio[320..448], reference_audio[320..448]);
    // While the program is stopped, engine [384, 512), only the held live note sounds.
    assert!(
        whole_audio[464..576]
            .iter()
            .any(|bits| bits & 0x7fff_ffff != 0),
        "the held live note was silenced by Stop"
    );
}

// ---------------------------------------------------------------------------------------
// A later loop through the concrete host.

/// The loop `[0, 28)`: the compiled note at plan 10 sounds from offset 10 of each pass.
const LOOP_END: u64 = 28;
/// The loop is entered at plan 0 at this engine sample, and left by a seek at `EXIT`.
const LOOP_ENTRY: u64 = 256;
const LOOP_EXIT: u64 = 448;
/// The note's offset into a pass; each wrap ends it and the pass restarts it here.
const LOOP_ONSET: u64 = 10;

fn loop_interval() -> LoopInterval {
    LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(LOOP_END)).expect("loop")
}

/// The wraps the loop plays before its exit: 284 and 312 inside the quantum at 256,
/// then 340, 368, 396 and 424.
fn loop_wraps() -> Vec<u64> {
    (1..)
        .map(|pass| LOOP_ENTRY + pass * LOOP_END)
        .take_while(|wrap| *wrap < LOOP_EXIT)
        .collect()
}

/// Where each compiled pass begins: the entry, every wrap, and the exit seek to plan 0.
/// The note is silent for the first `LOOP_ONSET` frames after each.
fn loop_pass_starts() -> Vec<u64> {
    std::iter::once(LOOP_ENTRY)
        .chain(loop_wraps())
        .chain(std::iter::once(LOOP_EXIT))
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoopMode {
    /// No compiled transport change: the note sounds from 64 to the end.
    Reference,
    /// A compiled Stop at the loop's entry: from there only live notes sound.
    Stop,
    /// Enter the loop at `LOOP_ENTRY` and leave it with an ordinary seek at `LOOP_EXIT`.
    Loop,
}

/// One script on the two-source host. Live notes: the first source holds key 60 from
/// 100 until after the exit; the second plays key 64 from 110 to 300, across the entry
/// and between two wraps, and key 67 from 330 to 400, inside the loop. Raw capture
/// records every packet throughout.
fn loop_run(mode: LoopMode, partition: &[usize]) -> (Vec<u32>, Vec<String>) {
    let initial = ActivationSequence::INITIAL.next().expect("armed sequence");
    let entered = initial.next().expect("loop sequence");
    let exited = entered.next().expect("exit sequence");
    let interval = loop_interval();
    let mut next = 0;
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(150)]);
    rig.send_all(SECOND, &[message(110, note_on(64)), frontier(150)]);
    rig.service();
    advance(&mut rig, at(192), partition, &mut next);

    match mode {
        LoopMode::Reference => {}
        LoopMode::Stop => assert_eq!(
            rig.direct_audio().schedule_stop(at(LOOP_ENTRY)),
            Ok(MixedBridgeStopAccepted::Scheduled)
        ),
        LoopMode::Loop => {
            assert!(
                rig.control
                    .prepare_loop_activation(
                        at(LOOP_ENTRY),
                        PlanPosition::new(LOOP_END + 1),
                        interval,
                        at(LOOP_ENTRY),
                        MixedActivationKind::Seek,
                    )
                    .is_err(),
                "an entry outside the loop is refused"
            );
            let offer = rig
                .control
                .prepare_loop_activation(
                    at(LOOP_ENTRY),
                    PlanPosition::ZERO,
                    interval,
                    at(LOOP_ENTRY),
                    MixedActivationKind::Seek,
                )
                .expect("the loop entry prepares and is admitted");
            assert_eq!(offer.supersedes(), initial);
            assert_eq!(offer.loop_interval(), Some(interval));
            let audio = rig.direct_audio();
            audio
                .offer_activation(offer)
                .expect("audio takes the entry");
            assert_eq!(
                audio.activation(),
                MixedActivationSlot::Pending {
                    supersedes: initial,
                    effective: at(LOOP_ENTRY),
                    kind: MixedActivationKind::Seek,
                    loop_interval: Some(interval),
                }
            );
            let state: MixedActivationLoop = audio.compiled_loop();
            assert_eq!(state.interval(), None, "nothing loops before the entry");
            assert_eq!(state.pass(), None);
            assert_eq!(state.wraps(), EventCount::NONE);
            assert_eq!(state.wrap_releases(), HeldNoteCount::NONE);
        }
    }
    rig.send_all(
        SECOND,
        &[
            message(300, note_off(64)),
            message(330, note_on(67)),
            frontier(350),
        ],
    );
    rig.send(FIRST, frontier(350));
    rig.service();
    advance(&mut rig, at(320), partition, &mut next);

    if mode == LoopMode::Loop {
        let audio = rig.direct_audio();
        let (receipt, entry) = adopted(audio.activation());
        assert_eq!(receipt.sequence(), entered);
        assert_eq!(receipt.kind(), MixedActivationKind::Seek);
        assert!(!receipt.late());
        assert_eq!(entry.effective(), at(LOOP_ENTRY));
        assert_eq!(
            entry.anchor(),
            StreamAnchor::new(at(LOOP_ENTRY), PlanPosition::ZERO)
        );
        assert_eq!(
            entry.released(),
            HeldNoteCount::measured(1),
            "the entry's boundary release ends the compiled note and no live one"
        );
        assert!(!entry.stopped());
        assert_eq!(entry.loop_interval(), Some(interval));
        assert_eq!(entry.retired_pass(), None, "the armed program did not loop");
        // Two wraps, at 284 and 312, inside the quantum the loop entered at.
        let state = audio.compiled_loop();
        assert_eq!(state.interval(), Some(interval));
        assert_eq!(state.pass().map(LoopPassId::as_u64), Some(3));
        assert_eq!(state.wraps(), EventCount::measured(2));
        assert_eq!(state.wrap_releases(), HeldNoteCount::measured(2));
        let retired = audio.take_retired_activation().expect("retired entry");
        assert_eq!(
            rig.control.collect_activation(retired).expect("collected"),
            receipt
        );
        // The exit is an ordinary seek, prepared against the loop now in force.
        let exit = rig
            .control
            .prepare_activation(
                at(LOOP_EXIT),
                PlanPosition::ZERO,
                at(LOOP_EXIT),
                MixedActivationKind::Seek,
            )
            .expect("the exit seek prepares");
        assert_eq!(exit.supersedes(), entered);
        assert_eq!(exit.loop_interval(), None);
        rig.direct_audio()
            .offer_activation(exit)
            .expect("audio takes the exit");
    }
    rig.send_all(SECOND, &[message(400, note_off(67)), frontier(470)]);
    rig.send(FIRST, frontier(470));
    rig.service();
    advance(&mut rig, at(512), partition, &mut next);
    assert_eq!(
        rig.holds(),
        EventCount::measured(1),
        "the first source's note is still held after the exit"
    );

    if mode == LoopMode::Loop {
        let audio = rig.direct_audio();
        let (receipt, exit) = adopted(audio.activation());
        assert_eq!(receipt.sequence(), exited);
        assert_eq!(receipt.supersedes(), entered);
        assert_eq!(exit.effective(), at(LOOP_EXIT));
        assert_eq!(
            exit.anchor(),
            StreamAnchor::new(at(LOOP_EXIT), PlanPosition::ZERO)
        );
        assert_eq!(
            exit.released(),
            HeldNoteCount::measured(1),
            "the exit ends the note the seventh pass restarted at 434"
        );
        assert_eq!(exit.loop_interval(), None);
        assert_eq!(exit.retired_pass().map(LoopPassId::as_u64), Some(7));
        let state = audio.compiled_loop();
        assert_eq!(state.interval(), None);
        assert_eq!(state.pass(), None);
        let wraps = loop_wraps().len();
        assert_eq!(wraps, 6);
        assert_eq!(state.wraps(), EventCount::measured(6));
        assert_eq!(state.wrap_releases(), HeldNoteCount::measured(6));
        let retired = audio.take_retired_activation().expect("retired exit");
        assert_eq!(
            rig.control.collect_activation(retired).expect("collected"),
            receipt
        );
    }
    rig.send_all(FIRST, &[message(560, note_off(60)), frontier(600)]);
    rig.send(SECOND, frontier(600));
    rig.service();
    advance(&mut rig, at(640), partition, &mut next);
    assert_eq!(rig.holds(), EventCount::NONE);
    if mode == LoopMode::Loop {
        assert_eq!(
            rig.direct_audio().compiled_loop().wraps(),
            EventCount::measured(6),
            "no wrap after the exit"
        );
    }

    let audio: Vec<_> = rig.rendered.iter().map(|sample| sample.to_bits()).collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    joined.assert_sound_pairs_as_raw_links();
    assert!(
        !joined.host_halted && !joined.raw_halted && joined.errors.is_empty(),
        "errors: {:?}",
        joined.errors
    );
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert_eq!(joined.teardown.fault(), None);
    let transfer = joined.teardown.transfer();
    assert_eq!(transfer.custody(), MixedTransferCustody::Complete);
    assert_eq!(transfer.holds_outstanding(), EventCount::NONE);
    assert_eq!(transfer.activation(), MixedActivationSlot::Empty);
    for (source, on, off) in [(FIRST, 100, 560), (SECOND, 110, 300), (SECOND, 330, 400)] {
        let identity = identity_of(
            joined
                .report_of(joined.packet_of(source, on))
                .expect("onset"),
        );
        assert!(
            matches!(
                joined.report_of(joined.packet_of(source, off)).expect("key-up").outcome(),
                MixedBridgeOutcome::KeyUp { result: Ok(MixedKeyUpRetired::Released(ended)), .. }
                    if ended == identity
            ),
            "the live note at {on} ends at its own key-up across the loop"
        );
    }
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw packet {packet:?} did not record across the loop"
        );
    }
    (audio, raw_trace(&joined.admitted, &joined.receipts))
}

#[test]
fn a_later_loop_wraps_twice_in_one_quantum_and_exits_by_seek_beside_held_live_notes() {
    let (reference_audio, reference_raw) = loop_run(LoopMode::Reference, &[Q]);
    let (stop_audio, stop_raw) = loop_run(LoopMode::Stop, &[Q]);
    let (whole_audio, whole_raw) = loop_run(LoopMode::Loop, &[Q]);
    let (irregular_audio, irregular_raw) = loop_run(LoopMode::Loop, &[37, 91, 13, 5, 110]);
    assert_eq!(stop_raw, reference_raw, "Stop changed raw capture");
    assert_eq!(whole_raw, reference_raw, "the loop changed raw capture");
    assert_eq!(
        irregular_raw, reference_raw,
        "partition changed raw capture"
    );
    assert_eq!(irregular_audio.len(), whole_audio.len());
    assert_eq!(
        irregular_audio
            .iter()
            .zip(&whole_audio)
            .position(|(a, b)| a != b),
        None,
        "partition changed loop audio"
    );
    // Output index `i` holds engine sample `i - 64`. Before the entry nothing changed.
    let output = |engine: u64| usize::try_from(engine).expect("fits") + Q;
    assert_eq!(
        whole_audio[..output(LOOP_ENTRY)],
        reference_audio[..output(LOOP_ENTRY)]
    );
    // Wherever the entry, a wrap or the exit has the compiled note silent, the output
    // is exactly the live notes Stop leaves sounding: every wrap lands on its sample and
    // leaves the held live voices untouched. Between those gaps each pass's note sounds
    // over the live voices, until the next pass starts or, after the exit, to the end.
    let starts = loop_pass_starts();
    for (index, &from) in starts.iter().enumerate() {
        let gap = output(from)..output(from + LOOP_ONSET);
        assert_eq!(
            whole_audio[gap.clone()],
            stop_audio[gap.clone()],
            "the gap after {from} differs from the live-only output"
        );
        assert!(
            whole_audio[gap.clone()]
                .iter()
                .any(|bits| bits & 0x7fff_ffff != 0),
            "the held live notes were silent in the gap after {from}"
        );
        let until = starts.get(index + 1).copied().unwrap_or(640);
        let sounding = output(from + LOOP_ONSET)..output(until);
        assert_ne!(
            whole_audio[sounding.clone()],
            stop_audio[sounding.clone()],
            "the pass from {from} did not sound its compiled note"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Rules 1.5a and 2.4: one host Panic through the bridge's protected cell and place.

/// What one Panic script produced that must not depend on the callback partition.
struct PanicRun {
    audio: Vec<u32>,
    raw: Vec<String>,
    reports: Vec<String>,
}

/// The host Panic reports a run collected, with ended onsets named by source and serial.
fn panic_trace(rig: &Rig) -> Vec<String> {
    rig.panics
        .iter()
        .map(|report| {
            let outcome = match report.outcome() {
                MixedBridgeOutcome::Panic(Ok(panicked)) => {
                    let ended: Vec<_> = panicked
                        .ended()
                        .map(|end| {
                            let onset = end.onset();
                            let source = admitted_in(&rig.admitted, onset).source;
                            let kind = match end {
                                MixedBridgePanicEnd::Retired(_) => "retired",
                                MixedBridgePanicEnd::Tombstoned(_) => "tombstoned",
                            };
                            format!("s{source}#{} {kind}", onset.serial())
                        })
                        .collect();
                    format!("ended {ended:?} at {}", panicked.executed_at())
                }
                other => format!("{other:?}"),
            };
            format!(
                "panic #{} {} {} {} {outcome}",
                report.id().get(),
                report.nominal_at(),
                report.stamp(),
                report.executed_at()
            )
        })
        .collect()
}

/// The sorted ended onsets of the run's only Panic report, and that report.
fn only_panic(rig: &Rig) -> (MixedBridgeReport, Vec<MixedBridgePanicEnd>) {
    let [report] = rig.panics.as_slice() else {
        panic!("one Panic report: {:?}", rig.panics);
    };
    assert_eq!(report.packet(), None, "a host Panic names no source packet");
    let MixedBridgeOutcome::Panic(Ok(panicked)) = report.outcome() else {
        panic!("the Panic was refused: {report:?}");
    };
    assert_eq!(panicked.executed_at(), report.executed_at());
    let mut ended: Vec<_> = panicked.ended().collect();
    ended.sort_unstable_by_key(|end| {
        let onset = end.onset();
        (admitted_in(&rig.admitted, onset).source, onset.serial())
    });
    assert_eq!(panicked.count().get(), ended.len());
    (*report, ended)
}

/// Every source report, then every Panic report, with the run's fresh connection
/// generations and identity tables named alike.
fn normalized_reports(rig: &Rig) -> Vec<String> {
    let mut reports = trace(rig, None).reports;
    reports.extend(panic_trace(rig));
    reports
        .into_iter()
        .map(|report| {
            let mut report = report
                .replace(&format!("{:?}", rig.generation(FIRST)), "first")
                .replace(&format!("{:?}", rig.generation(SECOND)), "second");
            while let Some(start) = report.find("TableId(") {
                let end = report[start..]
                    .find(')')
                    .map_or(report.len() - 1, |end| start + end);
                report.replace_range(start..=end, "table");
            }
            report
        })
        .collect()
}

fn audible(bits: &[u32]) -> bool {
    bits.iter().any(|bits| bits & 0x7fff_ffff != 0)
}

/// One two-source script, with or without a host Panic stamped 200. The first source
/// holds key 60 from 100 and plays key 62 at 200, the Panic's own stamp. The second
/// source sustains key 64 under its pedal from 130. After the Panic the first source's
/// key-ups arrive at 270 and 300; the second source plays key 67 from 280 to 400 and
/// lifts its pedal at 290. A compiled seek at 384 replays the compiled note from plan 0
/// in both runs, and raw capture records every packet throughout.
fn panic_run(panic: bool, partition: &[usize]) -> PanicRun {
    let mut next = 0;
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(150)]);
    rig.send_all(
        SECOND,
        &[
            message(110, sustain(true)),
            message(120, note_on(64)),
            message(130, note_off(64)),
            frontier(150),
        ],
    );
    rig.service();
    advance(&mut rig, at(192), partition, &mut next);
    assert_eq!(
        rig.holds(),
        EventCount::measured(2),
        "key 60 held, 64 sustained"
    );
    let offer = rig
        .control
        .prepare_activation(
            at(384),
            PlanPosition::ZERO,
            at(384),
            MixedActivationKind::Seek,
        )
        .expect("the seek prepares");
    rig.direct_audio()
        .offer_activation(offer)
        .expect("audio takes the seek");

    if panic {
        assert_eq!(rig.control.panic(at(200)), Ok(()));
        assert_eq!(rig.control.panic_pending(), Some(at(200)));
    }
    rig.send_all(FIRST, &[message(200, note_on(62)), frontier(260)]);
    rig.send(SECOND, frontier(260));
    rig.service();
    if panic {
        let steps: Vec<_> = rig
            .forwarded
            .iter()
            .filter_map(|step| match step {
                MixedPumpStep::Forwarded { packet, stamp, .. } => Some((Some(*packet), *stamp)),
                MixedPumpStep::Panic { stamp, .. } => Some((None, *stamp)),
                _ => None,
            })
            .skip(4)
            .collect();
        assert_eq!(
            steps,
            [(Some(rig.packet_of(FIRST, 200)), at(200)), (None, at(200))],
            "a source edge at the Panic's stamp goes first"
        );
    }
    advance(&mut rig, at(256), partition, &mut next);
    if panic {
        assert_eq!(
            rig.holds(),
            EventCount::NONE,
            "the Panic redeemed all three live holds at 200"
        );
        let state = rig.direct_audio().panic_state();
        assert_eq!(state.compiled_ended(), HeldNoteCount::measured(1));
        assert_eq!(state.compiled_groups(), EventCount::measured(1));
        assert_eq!(state.compiled_tombstones(), HeldNoteCount::measured(1));
        assert_eq!(state.parked(), None);
        let (report, ended) = only_panic(&rig);
        assert_eq!(
            (report.nominal_at(), report.stamp(), report.executed_at()),
            (at(200), at(200), at(200))
        );
        assert_eq!(
            ended,
            [
                MixedBridgePanicEnd::Tombstoned(rig.packet_of(FIRST, 100)),
                MixedBridgePanicEnd::Tombstoned(rig.packet_of(FIRST, 200)),
                MixedBridgePanicEnd::Retired(rig.packet_of(SECOND, 120)),
            ],
            "one group ends both sources' held, tied and sustained notes"
        );
        let tied = rig
            .report_of(rig.packet_of(FIRST, 200))
            .expect("tied onset");
        assert!(tied.id() < report.id(), "the tied onset precedes the Panic");
        assert_eq!(
            rig.control.panic_pending(),
            None,
            "collection frees the cell"
        );
    } else {
        assert_eq!(rig.holds(), EventCount::measured(3));
    }
    rig.send_all(
        FIRST,
        &[
            message(270, note_off(60)),
            message(300, note_off(62)),
            frontier(450),
        ],
    );
    rig.send_all(
        SECOND,
        &[
            message(280, note_on(67)),
            message(290, sustain(false)),
            message(400, note_off(67)),
            frontier(450),
        ],
    );
    rig.service();
    advance(&mut rig, at(576), partition, &mut next);
    let audio = rig.direct_audio();
    let (receipt, seek) = adopted(audio.activation());
    assert_eq!(seek.effective(), at(384));
    assert_eq!(
        seek.released(),
        HeldNoteCount::measured(if panic { 0 } else { 1 }),
        "the seek's boundary release finds the compiled note only without the Panic"
    );
    let retired = audio.take_retired_activation().expect("retired seek");
    assert_eq!(
        rig.control.collect_activation(retired).expect("collected"),
        receipt
    );
    assert_eq!(rig.holds(), EventCount::NONE);
    let outcome = |source, frames| {
        rig.report_of(rig.packet_of(source, frames))
            .expect("served")
            .outcome()
    };
    let identity =
        |source, frames| identity_of(rig.report_of(rig.packet_of(source, frames)).expect("onset"));
    for (on, off) in [(100, 270), (200, 300)] {
        let ended = identity(FIRST, on);
        let expected = if panic {
            MixedKeyUpRetired::AfterPanic(ended)
        } else {
            MixedKeyUpRetired::Released(ended)
        };
        assert_eq!(
            outcome(FIRST, off),
            MixedBridgeOutcome::KeyUp {
                onset: rig.packet_of(FIRST, on),
                result: Ok(expected),
            },
            "the key-up at {off}"
        );
    }
    let MixedBridgeOutcome::Lift {
        result: Ok(lifted), ..
    } = outcome(SECOND, 290)
    else {
        panic!("the lift was refused: {:?}", outcome(SECOND, 290));
    };
    assert_eq!(
        lifted.onsets().collect::<Vec<_>>(),
        if panic {
            vec![]
        } else {
            vec![rig.packet_of(SECOND, 120)]
        },
        "the Panic left the pedal down and retired the sustained note"
    );
    assert_eq!(
        outcome(SECOND, 400),
        MixedBridgeOutcome::KeyUp {
            onset: rig.packet_of(SECOND, 280),
            result: Ok(MixedKeyUpRetired::Released(identity(SECOND, 280))),
        },
        "a note after the Panic sounds until its own key-up"
    );
    if panic {
        let state = rig.direct_audio().panic_state();
        assert_eq!(state.live_released_after_panic(), EventCount::measured(2));
        assert_eq!(state.compiled_ended(), HeldNoteCount::measured(1));
        assert_eq!(state.compiled_released_after_panic(), EventCount::NONE);
    } else {
        assert!(rig.panics.is_empty());
    }
    let reports = normalized_reports(&rig);
    let audio: Vec<_> = rig.rendered.iter().map(|sample| sample.to_bits()).collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(
        !joined.host_halted && !joined.raw_halted && joined.errors.is_empty(),
        "errors: {:?}",
        joined.errors
    );
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty(), "no handle is stranded");
    assert!(joined.teardown.pedals().is_empty());
    assert_eq!(joined.teardown.panic(), None);
    assert_eq!(joined.teardown.fault(), None);
    let transfer = joined.teardown.transfer();
    assert_eq!(transfer.custody(), MixedTransferCustody::Complete);
    assert_eq!(transfer.holds_outstanding(), EventCount::NONE);
    assert_eq!(transfer.parked(), None);
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw packet {packet:?} did not record"
        );
    }
    PanicRun {
        audio,
        raw: raw_trace(&joined.admitted, &joined.receipts),
        reports,
    }
}

#[test]
fn panic_ends_both_sources_and_the_compiled_note_at_its_sample_while_raw_capture_continues() {
    let reference = panic_run(false, &[Q]);
    let whole = panic_run(true, &[Q]);
    let irregular = panic_run(true, &[37, 91, 13, 5, 110]);
    assert_eq!(whole.raw, reference.raw, "the Panic changed raw capture");
    assert_eq!(
        irregular.raw, reference.raw,
        "partition changed raw capture"
    );
    assert_eq!(
        irregular.reports, whole.reports,
        "partition changed reports"
    );
    assert_eq!(irregular.audio.len(), whole.audio.len());
    assert_eq!(
        irregular
            .audio
            .iter()
            .zip(&whole.audio)
            .position(|(a, b)| a != b),
        None,
        "partition changed the Panic's audio"
    );
    // Output index `i` holds engine sample `i - 64`.
    let output = |engine: u64| usize::try_from(engine).expect("fits") + Q;
    assert_eq!(
        whole.audio[..output(200)],
        reference.audio[..output(200)],
        "nothing changed before the Panic"
    );
    assert!(
        !audible(&whole.audio[output(200)..output(280)]),
        "every live and compiled note fell silent at the Panic's sample"
    );
    assert!(audible(&reference.audio[output(200)..output(280)]));
    assert!(
        audible(&whole.audio[output(280)..output(384)]),
        "a note after the Panic sounds"
    );
    assert!(
        audible(&whole.audio[output(394)..output(576)]),
        "the compiled transport kept running: the seek replays its note"
    );
}

#[test]
fn panic_waits_for_an_unfenced_peer_and_orders_by_nominal_stamp_across_sources() {
    let mut rig = Rig::new(Setup {
        peer_wait: 256,
        ..Setup::default()
    });
    let mut next = 0;
    rig.prime();
    let peer = MixedBridgeWait::Peer {
        source: rig.generation(SECOND),
    };
    rig.send(FIRST, frontier(1_000));
    assert_eq!(rig.control.panic(at(300)), Ok(()));
    rig.service();
    assert_eq!(
        rig.waits.last(),
        Some(&peer),
        "a silent peer holds the Panic"
    );
    // A peer whose frontier only reaches the Panic's stamp may still send at that stamp.
    rig.send(SECOND, frontier(300));
    rig.service();
    assert_eq!(rig.waits.last(), Some(&peer));
    assert!(rig.forwarded.is_empty());
    assert_eq!(rig.control.panic_pending(), Some(at(300)));
    // Its onset at the Panic's stamp goes first, and the Panic ends it.
    rig.send_all(SECOND, &[message(300, note_on(64)), frontier(350)]);
    rig.service();
    assert!(
        matches!(
            rig.forwarded.as_slice(),
            [
                MixedPumpStep::Forwarded { packet, stamp: onset, .. },
                MixedPumpStep::Panic { stamp, .. },
            ] if *packet == rig.packet_of(SECOND, 300) && *onset == at(300) && *stamp == at(300)
        ),
        "{:?}",
        rig.forwarded
    );
    advance(&mut rig, at(384), &[Q], &mut next);
    let (report, ended) = only_panic(&rig);
    assert_eq!(report.executed_at(), at(300));
    assert_eq!(
        ended,
        [MixedBridgePanicEnd::Tombstoned(rig.packet_of(SECOND, 300))]
    );
    assert_eq!(rig.holds(), EventCount::NONE);

    // A second Panic at 600 waits for the peer, whose frontier is 350, until the peer is
    // fenced: the audio boundary passes 350 + 256.
    assert_eq!(rig.control.panic(at(600)), Ok(()));
    rig.service();
    assert_eq!(rig.waits.last(), Some(&peer));
    advance(&mut rig, at(576), &[Q], &mut next);
    rig.service();
    assert_eq!(
        rig.waits.last(),
        Some(&peer),
        "the peer is not fenced at 576"
    );
    advance(&mut rig, at(640), &[Q], &mut next);
    rig.service();
    assert!(
        matches!(rig.forwarded.last(), Some(MixedPumpStep::Panic { stamp, .. }) if *stamp == at(640)),
        "the fenced peer no longer holds the Panic: {:?}",
        rig.forwarded
    );
    // The fenced peer's later onset, stamped before the Panic, executes behind it and
    // keeps sounding.
    rig.send_all(SECOND, &[message(500, note_on(66)), frontier(650)]);
    rig.service();
    advance(&mut rig, at(704), &[Q], &mut next);
    let second = rig.panics.last().copied().expect("second Panic");
    assert_eq!(
        (second.nominal_at(), second.stamp(), second.executed_at()),
        (at(600), at(640), at(640))
    );
    assert!(matches!(
        second.outcome(),
        MixedBridgeOutcome::Panic(Ok(panicked)) if panicked.count().get() == 0
    ));
    let late = rig
        .report_of(rig.packet_of(SECOND, 500))
        .expect("late onset");
    assert!(late.id() > second.id());
    assert_eq!(
        (late.nominal_at(), late.stamp()),
        (at(500), at(640)),
        "the fenced onset keeps both stamps"
    );
    let late_identity = identity_of(late);
    assert_eq!(
        rig.holds(),
        EventCount::measured(1),
        "the late onset survives"
    );
    rig.send_all(
        SECOND,
        &[
            message(700, note_off(64)),
            message(710, note_off(66)),
            frontier(800),
        ],
    );
    rig.service();
    advance(&mut rig, at(832), &[Q], &mut next);
    let outcome = |frames| {
        rig.report_of(rig.packet_of(SECOND, frames))
            .expect("key-up")
            .outcome()
    };
    assert!(matches!(
        outcome(700),
        MixedBridgeOutcome::KeyUp {
            result: Ok(MixedKeyUpRetired::AfterPanic(_)),
            ..
        }
    ));
    assert_eq!(
        outcome(710),
        MixedBridgeOutcome::KeyUp {
            onset: rig.packet_of(SECOND, 500),
            result: Ok(MixedKeyUpRetired::Released(late_identity)),
        }
    );
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.open().is_empty());
    assert_eq!(joined.teardown.panic(), None);
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

/// Sixteen onsets and their sixteen reserved key-up places charge every ordinary place;
/// audio serves them, every result stays uncollected and every live hold is out. A
/// later first-source onset at 60 waits at its lane's head for command places, and the
/// second source's frontier is `peer`.
fn saturated(peer: u64) -> Rig {
    let mut rig = Rig::new(Setup {
        cells: 10,
        ..Setup::default()
    });
    assert_eq!(depth(), 32, "the fixture saturates a 32-place command ring");
    rig.prime();
    for note in 0..8_u8 {
        let base = 10 + 2 * u64::from(note);
        rig.send(FIRST, message(base, note_on(60 + note)));
        rig.service();
        rig.send(SECOND, message(base + 1, note_on(80 + note)));
        rig.service();
    }
    rig.send(FIRST, message(60, note_on(70)));
    rig.send(SECOND, frontier(peer));
    rig.service();
    assert_eq!(rig.forwarded.len(), 16);
    assert_eq!(rig.waits.last(), Some(&MixedBridgeWait::Commands));
    rig.render(Q).expect("serving callback");
    assert_eq!(rig.holds(), EventCount::measured(4));
    rig.pump();
    assert_eq!(rig.waits.last(), Some(&MixedBridgeWait::Commands));
    rig
}

#[test]
fn an_earlier_source_head_waiting_for_places_holds_the_panic_until_a_collection() {
    let mut rig = saturated(100);
    rig.send(FIRST, frontier(100));
    rig.stage_all();
    assert_eq!(rig.control.panic(at(80)), Ok(()));
    rig.pump();
    assert_eq!(rig.forwarded.len(), 16, "order comes before capacity");
    assert_eq!(rig.waits.last(), Some(&MixedBridgeWait::Commands));
    // Two collected results give the onset its two places; the Panic follows it.
    for _ in 0..2 {
        let report = rig
            .control
            .collect(&mut rig.control_port)
            .expect("collect")
            .expect("a served onset");
        let packet = report.packet().expect("a source report");
        rig.settled.push((packet, Settled::Report(report)));
    }
    rig.pump();
    assert!(
        matches!(
            &rig.forwarded[16..],
            [
                MixedPumpStep::Forwarded { packet, .. },
                MixedPumpStep::Panic { stamp, .. },
            ] if *packet == rig.packet_of(FIRST, 60) && *stamp == at(80)
        ),
        "{:?}",
        &rig.forwarded[16..]
    );
    rig.render(Q).expect("the Panic's callback");
    rig.collect();
    let (panic, ended) = only_panic(&rig);
    let held = rig
        .report_of(rig.packet_of(FIRST, 60))
        .expect("the earlier onset");
    assert!(held.id() < panic.id(), "the earlier onset executes first");
    assert!(
        matches!(held.outcome(), MixedBridgeOutcome::NoteOn(Err(_))),
        "every live hold was out, so audio refused it: {held:?}"
    );
    assert_eq!(ended.len(), 4, "the Panic ends the four accepted notes");
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
}

#[test]
fn panic_takes_its_protected_place_with_commands_results_and_holds_saturated() {
    let mut rig = saturated(60);
    assert_eq!(rig.control.panic(at(40)), Ok(()));
    rig.pump();
    assert!(
        matches!(rig.forwarded.last(), Some(MixedPumpStep::Panic { stamp, .. }) if *stamp == at(64)),
        "the Panic takes its protected place at once: {:?}",
        rig.forwarded
    );
    assert_eq!(
        rig.waits.last(),
        Some(&MixedBridgeWait::Commands),
        "the Panic took no ordinary place"
    );
    rig.render(Q).expect("the Panic's callback");
    assert_eq!(rig.holds(), EventCount::NONE, "the Panic ended every hold");

    rig.collect();
    let (report, ended) = only_panic(&rig);
    assert_eq!(
        (report.nominal_at(), report.stamp(), report.executed_at()),
        (at(40), at(64), at(64))
    );
    let sounding = [(FIRST, 10), (FIRST, 12), (SECOND, 11), (SECOND, 13)];
    assert_eq!(
        ended,
        sounding
            .map(|(source, frames)| MixedBridgePanicEnd::Tombstoned(rig.packet_of(source, frames))),
        "only the four accepted notes sound; refusal tombstones are not members"
    );
    // Every key-up follows: the four ended notes' are consumed after the Panic, the others
    // after their audio refusals, and the waiting onset sounds behind the Panic.
    for note in 0..8_u8 {
        rig.send(FIRST, message(100 + u64::from(note), note_off(60 + note)));
        rig.send(SECOND, message(110 + u64::from(note), note_off(80 + note)));
    }
    rig.send(FIRST, message(120, note_off(70)));
    rig.send(FIRST, frontier(10_000));
    rig.send(SECOND, frontier(10_000));
    for _ in 0..8 {
        rig.service();
        rig.render(Q).expect("draining callback");
        rig.collect();
    }
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    for note in 0..8_u8 {
        for (source, on, off) in [
            (FIRST, 10 + 2 * u64::from(note), 100 + u64::from(note)),
            (SECOND, 11 + 2 * u64::from(note), 110 + u64::from(note)),
        ] {
            let onset = rig.report_of(rig.packet_of(source, on)).expect("onset");
            let key_up = rig.report_of(rig.packet_of(source, off)).expect("key-up");
            let MixedBridgeOutcome::KeyUp { result, .. } = key_up.outcome() else {
                panic!("not a key-up: {key_up:?}");
            };
            match onset.outcome() {
                MixedBridgeOutcome::NoteOn(Ok(identity)) => {
                    assert_eq!(result, Ok(MixedKeyUpRetired::AfterPanic(identity)));
                }
                MixedBridgeOutcome::NoteOn(Err(refused)) => {
                    assert_eq!(result, Ok(MixedKeyUpRetired::AfterRefusal(refused)));
                }
                other => panic!("not an onset: {other:?}"),
            }
        }
    }
    let waited = rig
        .report_of(rig.packet_of(FIRST, 60))
        .expect("waiting onset");
    assert!(waited.id() > report.id());
    assert_eq!(
        rig.report_of(rig.packet_of(FIRST, 120))
            .expect("its key-up")
            .outcome(),
        MixedBridgeOutcome::KeyUp {
            onset: rig.packet_of(FIRST, 60),
            result: Ok(MixedKeyUpRetired::Released(identity_of(waited))),
        }
    );
    assert_eq!(
        rig.direct_audio().panic_state().live_released_after_panic(),
        EventCount::measured(4)
    );
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.held().is_empty());
    assert!(joined.teardown.open().is_empty());
    assert_eq!(
        joined.teardown.transfer().holds_outstanding(),
        EventCount::NONE
    );
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

#[test]
fn a_second_panic_is_refused_at_the_bridge_until_collected_and_a_new_quantum_opens() {
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    rig.send(FIRST, frontier(1_000));
    rig.send(SECOND, frontier(1_000));
    rig.stage_all();
    assert_eq!(rig.control.panic(at(100)), Ok(()));
    let pending = Err(MixedBridgePanicRefusal::Pending { at: at(100) });
    assert_eq!(rig.control.panic(at(120)), pending, "staged");
    rig.pump();
    assert!(matches!(
        rig.forwarded.as_slice(),
        [MixedPumpStep::Panic { stamp, .. }] if *stamp == at(100)
    ));
    assert_eq!(rig.control.panic(at(120)), pending, "forwarded");
    rig.render(Q).expect("callback");
    rig.render(Q).expect("the Panic's callback");
    assert_eq!(rig.control.panic(at(120)), pending, "served, uncollected");
    rig.collect();
    let (first, _) = only_panic(&rig);
    assert_eq!(first.executed_at(), at(100));
    assert_eq!(rig.control.panic_pending(), None);
    // Collection frees the cell, but audio has not opened the quantum after 100 yet.
    assert_eq!(
        rig.control.panic(at(90)),
        Err(MixedBridgePanicRefusal::BehindPanic {
            at: at(90),
            previous: at(100)
        })
    );
    assert_eq!(
        rig.control.panic(at(120)),
        Err(MixedBridgePanicRefusal::SameServiceQuantum { previous: at(100) })
    );
    rig.pump();
    assert_eq!(rig.forwarded.len(), 1, "no refusal accepted anything");
    assert_eq!(rig.control.panic_pending(), None);
    rig.render(Q).expect("the next quantum opens");
    assert_eq!(rig.control.panic(at(120)), Ok(()));
    rig.pump();
    assert!(matches!(
        rig.forwarded.last(),
        Some(MixedPumpStep::Panic { stamp, .. }) if *stamp == at(192)
    ));
    rig.render(Q).expect("the second Panic's callback");
    rig.collect();
    assert_eq!(rig.panics.len(), 2);
    let second = rig.panics[1];
    assert_eq!(
        (second.nominal_at(), second.stamp(), second.executed_at()),
        (at(120), at(192), at(192))
    );
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(joined.teardown.panic(), None);
}

#[test]
fn a_parked_staged_or_uncollected_panic_at_join_keeps_its_custody() {
    // Forwarded inside the window but stamped in a later quantum: audio parks it.
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    rig.send(FIRST, frontier(2_000));
    rig.send(SECOND, frontier(2_000));
    assert_eq!(rig.control.panic(at(500)), Ok(()));
    rig.service();
    let Some(&MixedPumpStep::Panic { id, stamp }) = rig.forwarded.last() else {
        panic!("forwarded Panic: {:?}", rig.forwarded);
    };
    assert_eq!(stamp, at(500));
    rig.render(Q).expect("callback");
    assert_eq!(rig.direct_audio().panic_state().parked(), Some(at(500)));
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(
        joined.teardown.panic(),
        Some(MixedSourceCustody::Command {
            id,
            place: MixedCommandPlace::Parked
        })
    );
    let transfer = joined.teardown.transfer();
    assert_eq!(transfer.custody(), MixedTransferCustody::Complete);
    assert_eq!(transfer.parked(), Some(id));
    assert_eq!(transfer.outstanding(), [id]);

    // Accepted and still waiting for a silent peer.
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    rig.send(FIRST, frontier(2_000));
    assert_eq!(rig.control.panic(at(500)), Ok(()));
    rig.service();
    assert!(rig.forwarded.is_empty());
    let joined = rig.join();
    assert_eq!(joined.teardown.panic(), Some(MixedSourceCustody::Staged));
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );

    // Served, with its result never collected.
    let mut rig = Rig::new(Setup::default());
    rig.prime();
    rig.send(FIRST, frontier(2_000));
    rig.send(SECOND, frontier(2_000));
    assert_eq!(rig.control.panic(at(100)), Ok(()));
    rig.service();
    let Some(&MixedPumpStep::Panic { id, .. }) = rig.forwarded.last() else {
        panic!("forwarded Panic: {:?}", rig.forwarded);
    };
    rig.render(Q).expect("callback");
    rig.render(Q).expect("the Panic's callback");
    let joined = rig.join();
    assert_eq!(
        joined.teardown.panic(),
        Some(MixedSourceCustody::Command {
            id,
            place: MixedCommandPlace::Uncollected
        })
    );
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

// ---------------------------------------------------------------------------------------
// Item 2 step 2: a due Panic cancels a sealed stopped-owner reset; a later one follows it.

/// The reset boundary both Panic-precedence scripts seal.
const RESET_B: u64 = 448;

/// Every partition a reset-precedence script runs in: the profile's largest callback,
/// one quantum, eight frames and an irregular split. Control acts at the same samples in
/// each, because [`advance_output`] cuts a callback wherever the script acts.
const RESET_PARTITIONS: [&[usize]; 4] = [&[1_024], &[Q], &[8], &[37, 91, 13, 5, 110]];

impl Rig {
    /// Move the direct mixed owner into a reset wrapper and schedule its compiled Stop.
    fn wrap_for_reset(&mut self, stop: SampleTime) {
        let callback = self.callback.as_mut().expect("callback");
        let MixedHostAudio::Direct(audio, port) = callback.mixed.take().expect("direct owner")
        else {
            panic!("fresh rig is direct");
        };
        let owner = MixedBridgeResetOwner::new(*audio, port).expect("matching audio port");
        let mut wrapper = MixedBridgeResetAudio::new(owner);
        wrapper.schedule_stop(stop).expect("compiled Stop");
        callback.mixed = Some(MixedHostAudio::Reset(Box::new(wrapper)));
    }

    fn reset_wrapper(
        &mut self,
    ) -> &mut MixedBridgeResetAudio<
        HostConsumer<MixedTransferCommand>,
        HostProducer<MixedTransferResult>,
    > {
        match self
            .callback
            .as_mut()
            .expect("callback present")
            .mixed
            .as_mut()
            .expect("mixed owner present")
        {
            MixedHostAudio::Reset(wrapper) => wrapper,
            MixedHostAudio::Direct(..) => panic!("this host renders through a reset wrapper"),
        }
    }

    /// Arm a successor bridge for `B`, seal it and offer its audio half to the wrapper.
    /// Returns the successor's control port.
    fn seal_and_offer(&mut self, b: SampleTime) -> ControlPort {
        let MixedBridgeArmed {
            control,
            audio,
            control_port,
            audio_port,
        } = arm_at(&self.raw, Setup::default(), b);
        self.control.seal_reset(control, b).expect("sealed bridge");
        assert_eq!(self.control.reset_ready(), Some(b));
        let owner = MixedBridgeResetOwner::new(audio, audio_port).expect("successor port");
        self.reset_wrapper()
            .offer_reset(owner, b)
            .expect("the stopped owner accepts the sealed successor");
        control_port
    }
}

/// Render through `partition` until the output holds engine sample `to - 1`, cutting the
/// last callback there, without reading the bridge's boundary: after an applied reset the
/// old transfer's boundary stays at `B`.
fn advance_output(rig: &mut Rig, to: SampleTime, partition: &[usize], next: &mut usize) {
    let output = usize::try_from(to.as_u64()).expect("fits") + Q;
    while rig.output_frames() < output {
        let chunk = partition[*next % partition.len()].min(output - rig.output_frames());
        *next += 1;
        rig.render(chunk).expect("whole callback");
        rig.collect();
    }
}

/// What one reset-precedence script produced that must not depend on the partition or,
/// for a cancelled reset, on whether a reset was sealed at all.
#[derive(Debug, PartialEq)]
struct PrecedenceRun {
    audio: Vec<u32>,
    raw: Vec<String>,
    reports: Vec<String>,
    /// The joined old owner's counts: holds outstanding, open handles, pedals, held
    /// source IDs, outstanding commands and the Panic's custody.
    old_owner: String,
}

/// One two-source script on a stopped owner, with or without a reset sealed at `B`.
/// The first source holds key 60 from 100; the second sustains key 64 under its pedal
/// from 110. At 384 a reset is sealed and its successor offered, or not. Then the first
/// source plays key 62 at 400 and the second key 67 at 430, both staged while sealed,
/// and a host Panic stamped 420 is accepted before audio reaches `B`. The Panic cancels
/// the sealed reset, so both runs must be identical: it ends 60, 62 and 64 at 420, and
/// 67 sounds until its own key-up.
fn cancel_run(seal: bool, partition: &[usize]) -> PrecedenceRun {
    let b = at(RESET_B);
    let mut next = 0;
    let mut rig = Rig::new(Setup::default());
    rig.wrap_for_reset(at(192));
    rig.prime();
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(150)]);
    rig.send_all(
        SECOND,
        &[
            message(110, sustain(true)),
            message(120, note_on(64)),
            message(130, note_off(64)),
            frontier(150),
        ],
    );
    rig.service();
    advance(&mut rig, at(384), partition, &mut next);
    let successor_port = seal.then(|| rig.seal_and_offer(b));

    rig.send_all(FIRST, &[message(400, note_on(62)), frontier(440)]);
    rig.send_all(SECOND, &[message(430, note_on(67)), frontier(440)]);
    rig.stage_all();
    if seal {
        assert_eq!(
            rig.control.pump(&mut rig.control_port),
            Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: b })),
            "both onsets wait while sealed"
        );
    }
    let panic = at(420);
    assert_eq!(rig.control.panic(panic), Ok(()));
    assert_eq!(
        rig.control.reset_phase(),
        if seal {
            MixedBridgeResetPhase::Cancelled { at: b, panic }
        } else {
            MixedBridgeResetPhase::Open
        }
    );
    rig.service();
    let steps: Vec<_> = rig
        .forwarded
        .iter()
        .filter_map(|step| match step {
            MixedPumpStep::Forwarded { packet, stamp, .. } => Some((Some(*packet), *stamp)),
            MixedPumpStep::Panic { stamp, .. } => Some((None, *stamp)),
            _ => None,
        })
        .skip(4)
        .collect();
    assert_eq!(
        steps,
        [
            (Some(rig.packet_of(FIRST, 400)), at(400)),
            (None, panic),
            (Some(rig.packet_of(SECOND, 430)), at(430)),
        ],
        "the earlier onset, the Panic, then the later onset, each at its nominal stamp"
    );
    advance(&mut rig, at(512), partition, &mut next);
    if seal {
        assert_eq!(
            rig.reset_wrapper().state(),
            MixedBridgeResetState::Refused {
                at: b,
                reason: MixedBridgeResetRefusal::PanicCancelled,
            }
        );
    } else {
        assert_eq!(rig.reset_wrapper().state(), MixedBridgeResetState::Empty);
    }
    let (report, ended) = only_panic(&rig);
    assert_eq!(
        (report.nominal_at(), report.stamp(), report.executed_at()),
        (panic, panic, panic),
        "accepted and pumped before audio reached 420, the Panic is sample-exact"
    );
    assert_eq!(
        ended,
        [
            MixedBridgePanicEnd::Tombstoned(rig.packet_of(FIRST, 100)),
            MixedBridgePanicEnd::Tombstoned(rig.packet_of(FIRST, 400)),
            MixedBridgePanicEnd::Retired(rig.packet_of(SECOND, 120)),
        ],
        "the old owner ends the held, the staged earlier and the sustained note once"
    );

    rig.send_all(
        FIRST,
        &[
            message(520, note_off(60)),
            message(530, note_off(62)),
            frontier(700),
        ],
    );
    rig.send_all(
        SECOND,
        &[
            message(540, sustain(false)),
            message(600, note_off(67)),
            frontier(700),
        ],
    );
    rig.service();
    advance(&mut rig, at(768), partition, &mut next);
    let outcome = |source, frames| {
        rig.report_of(rig.packet_of(source, frames))
            .expect("served")
            .outcome()
    };
    for (on, off) in [(100, 520), (400, 530)] {
        assert!(
            matches!(
                outcome(FIRST, off),
                MixedBridgeOutcome::KeyUp {
                    result: Ok(MixedKeyUpRetired::AfterPanic(_)),
                    ..
                }
            ),
            "the key-up at {off} finds its note already ended by the Panic"
        );
        assert_eq!(rig.settled_of(rig.packet_of(FIRST, on)).len(), 1);
    }
    assert!(matches!(
        outcome(SECOND, 540),
        MixedBridgeOutcome::Lift { result: Ok(lifted), .. } if lifted.onsets().count() == 0
    ));
    assert!(
        matches!(
            outcome(SECOND, 600),
            MixedBridgeOutcome::KeyUp {
                result: Ok(MixedKeyUpRetired::Released(_)),
                ..
            }
        ),
        "the later staged onset sounds until its own key-up"
    );

    let reports = normalized_reports(&rig);
    let audio: Vec<_> = rig.rendered.iter().map(|sample| sample.to_bits()).collect();
    let successor = if seal {
        let refused = rig
            .reset_wrapper()
            .take_refused()
            .expect("the refused successor");
        assert_eq!(refused.reason(), MixedBridgeResetRefusal::PanicCancelled);
        let owner = refused.into_owner();
        let next = rig
            .control
            .abandon_reset(&owner)
            .expect("a cancelled reset is abandoned");
        assert_eq!(rig.control.reset_phase(), MixedBridgeResetPhase::Open);
        Some((next, owner, successor_port.expect("successor port")))
    } else {
        None
    };
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(
        !joined.host_halted && !joined.raw_halted && joined.errors.is_empty(),
        "errors: {:?}",
        joined.errors
    );
    assert!(joined.retired_reset.is_empty());
    let teardown = &joined.teardown;
    let transfer = teardown.transfer();
    assert_eq!(transfer.custody(), MixedTransferCustody::Complete);
    let old_owner = format!(
        "holds {:?} open {} pedals {} held {} outstanding {} panic {:?}",
        transfer.holds_outstanding(),
        teardown.open().len(),
        teardown.pedals().len(),
        teardown.held().len(),
        transfer.outstanding().len(),
        teardown.panic(),
    );
    if let Some((next, owner, port)) = successor {
        let (audio, audio_port) = owner.into_parts();
        let successor = next
            .join(audio, Some(port), Some(audio_port))
            .expect("the abandoned successor joins");
        assert_eq!(successor.panic(), None, "the successor never had the Panic");
        assert!(successor.held().is_empty() && successor.open().is_empty());
        let transfer = successor.transfer();
        assert_eq!(transfer.custody(), MixedTransferCustody::Complete);
        assert!(transfer.outstanding().is_empty() && transfer.handles().is_empty());
    }
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw packet {packet:?} did not record"
        );
    }
    PrecedenceRun {
        audio,
        raw: raw_trace(&joined.admitted, &joined.receipts),
        reports,
        old_owner,
    }
}

#[test]
fn a_due_panic_cancels_the_sealed_reset_and_ends_the_old_owners_notes_once_in_every_partition() {
    let reference = cancel_run(false, &[Q]);
    // Output index `i` holds engine sample `i - 64`.
    let b_output = usize::try_from(RESET_B).expect("fits") + Q;
    assert!(
        audible(&reference.audio[b_output..]),
        "the later onset sounds after B"
    );
    for partition in RESET_PARTITIONS {
        assert_eq!(
            cancel_run(false, partition),
            reference,
            "partition {partition:?} changed the run without a reset"
        );
        assert_eq!(
            cancel_run(true, partition),
            reference,
            "partition {partition:?}: the cancelled reset changed sound, reports, raw \
             capture or the old owner's teardown"
        );
    }
}

/// What one applied-reset Panic script produced that must not depend on the partition.
#[derive(Debug, PartialEq)]
struct AppliedRun {
    audio: Vec<u32>,
    raw: Vec<String>,
    reports: Vec<String>,
    retired: String,
}

/// The first source holds key 60 from 100; the second sustains key 64 from 110. A reset
/// sealed at 384 is applied at `B`, with key 62 staged on the first source at 440 while
/// sealed. A host Panic stamped 450, inside `B`'s quantum, is accepted only after the
/// callback that opened `B`: audio already acquired the seal, so the cancel loses and the
/// reset commits. The `Reset` group ends 60 and 64 at `B + Q`; the Panic reaches only the
/// successor, ending key 62 there. Neither the onset nor the Panic is sample-exact: both
/// were pumped after audio opened `B`, so both execute at the successor's first open
/// sample, 512.
fn applied_panic_run(partition: &[usize]) -> AppliedRun {
    let b = at(RESET_B);
    let mut next = 0;
    let mut rig = Rig::new(Setup::default());
    rig.wrap_for_reset(at(192));
    rig.prime();
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(150)]);
    rig.send_all(
        SECOND,
        &[
            message(110, sustain(true)),
            message(120, note_on(64)),
            message(130, note_off(64)),
            frontier(150),
        ],
    );
    rig.service();
    advance(&mut rig, at(384), partition, &mut next);
    let successor_port = rig.seal_and_offer(b);
    rig.send_all(FIRST, &[message(440, note_on(62)), frontier(470)]);
    rig.send(SECOND, frontier(470));
    rig.service();
    advance_output(&mut rig, at(456), partition, &mut next);
    let MixedBridgeResetState::Fading { receipt, .. } = rig.reset_wrapper().state() else {
        panic!("the reset did not apply: {:?}", rig.reset_wrapper().state());
    };

    // Due inside `B`'s quantum, but audio decided first: the reset stays sealed.
    let panic = at(450);
    assert_eq!(rig.control.panic(panic), Ok(()));
    assert_eq!(
        rig.control.reset_phase(),
        MixedBridgeResetPhase::Sealed { at: b }
    );
    rig.pump();
    assert_eq!(
        rig.waits.last(),
        Some(&MixedBridgeWait::ResetSealed { at: b })
    );
    let retired = rig.control.commit_reset(receipt).expect("commit");
    rig.reset_old.push((
        retired,
        std::mem::replace(&mut rig.control_port, successor_port),
    ));
    rig.service();
    let opened = at(512);
    let steps: Vec<_> = rig
        .forwarded
        .iter()
        .filter_map(|step| match step {
            MixedPumpStep::Forwarded { packet, stamp, .. } => Some((Some(*packet), *stamp)),
            MixedPumpStep::Panic { stamp, .. } => Some((None, *stamp)),
            _ => None,
        })
        .skip(4)
        .collect();
    assert_eq!(
        steps,
        [(Some(rig.packet_of(FIRST, 440)), opened), (None, opened)],
        "pumped after B opened, both go to the successor at its first open sample"
    );
    advance_output(&mut rig, at(576), partition, &mut next);
    let (report, ended) = only_panic(&rig);
    assert_eq!(
        (report.nominal_at(), report.stamp(), report.executed_at()),
        (panic, opened, opened),
        "accepted after audio opened B, the Panic is late, not sample-exact"
    );
    assert_eq!(
        ended,
        [MixedBridgePanicEnd::Tombstoned(rig.packet_of(FIRST, 440))],
        "the Panic ends only the successor's note"
    );
    let after_fade: Vec<_> = rig
        .frame_samples((512 + Q)..(576 + Q))
        .iter()
        .map(|sample| sample.to_bits())
        .collect();
    assert!(
        !audible(&after_fade),
        "the Reset group and the Panic leave nothing sounding from B + Q"
    );

    rig.send_all(
        FIRST,
        &[
            message(600, note_off(60)),
            message(610, note_off(62)),
            frontier(700),
        ],
    );
    rig.send_all(SECOND, &[message(620, sustain(false)), frontier(700)]);
    rig.service();
    advance_output(&mut rig, at(768), partition, &mut next);
    let joined_reports = normalized_reports(&rig);
    let audio: Vec<_> = rig.rendered.iter().map(|sample| sample.to_bits()).collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(
        !joined.host_halted && !joined.raw_halted && joined.errors.is_empty(),
        "errors: {:?}",
        joined.errors
    );
    assert_eq!(
        joined.sound(joined.packet_of(FIRST, 600)),
        Settled::Stage(Ok(MixedStaged::Split(MixedSplit::AfterRefusedOnset {
            onset: joined.packet_of(FIRST, 100),
            reason: MixedOnsetRefusal::Reset,
        }))),
        "the old key-up never reaches the successor"
    );
    assert!(matches!(
        joined
            .report_of(joined.packet_of(FIRST, 610))
            .expect("new key-up")
            .outcome(),
        MixedBridgeOutcome::KeyUp {
            result: Ok(MixedKeyUpRetired::AfterPanic(_)),
            ..
        }
    ));
    let [retired] = joined.retired_reset.as_slice() else {
        panic!("one retired owner joined");
    };
    assert_eq!(
        retired.bridge().panic(),
        None,
        "the old arm never had the Panic"
    );
    let handles: Vec<_> = retired
        .handles()
        .iter()
        .map(|(packet, _)| *packet)
        .collect();
    assert_eq!(
        handles,
        [joined.packet_of(FIRST, 100), joined.packet_of(SECOND, 120)],
        "the Reset group ended each old note once"
    );
    assert!(
        retired
            .handles()
            .iter()
            .all(|(_, handle)| matches!(handle, MixedRetiredHandle::Ended(_)))
    );
    assert_eq!(
        retired.bridge().transfer().holds_outstanding(),
        EventCount::NONE
    );
    assert_eq!(joined.teardown.panic(), None);
    assert_eq!(
        joined.teardown.transfer().holds_outstanding(),
        EventCount::NONE
    );
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw packet {packet:?} did not record across reset"
        );
    }
    AppliedRun {
        audio,
        raw: raw_trace(&joined.admitted, &joined.receipts),
        reports: joined_reports,
        retired: format!("{:?}", retired.handles().len()),
    }
}

#[test]
fn a_panic_after_an_applied_reset_reaches_only_the_successor_in_every_partition() {
    let reference = applied_panic_run(&[Q]);
    for partition in RESET_PARTITIONS {
        assert_eq!(
            applied_panic_run(partition),
            reference,
            "partition {partition:?} changed the applied reset's Panic"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Repeated successor plans: two committed resets and a later activation on each successor.

/// The second reset's boundary.
const RESET_C: u64 = 1_024;

/// How one repeated-reset script runs.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Repeated {
    /// The same sources on the first owner alone: no reset, Stop or activation.
    Reference,
    /// Both resets and both successor activations, and no stray offer.
    Quiet,
    /// As `Quiet`, plus every stray offer: each is refused and handed back whole.
    Probed,
}

/// What one repeated-reset run produced that must not depend on the partition or on the
/// refused stray offers.
#[derive(Debug, PartialEq)]
struct RepeatedRun {
    audio: Vec<u32>,
    raw: Vec<String>,
    reports: Vec<String>,
    splits: Vec<String>,
    /// Each retired owner's handles, holds, activation slot and custody, oldest first.
    retired: Vec<String>,
    /// The final owner's teardown summary.
    last: String,
}

/// Service the host at every quantum boundary up to `to`, rendering through `partition`
/// in between, so every partition stages, pumps and collects at the same samples.
fn serve_to(rig: &mut Rig, to: SampleTime, partition: &[usize], next: &mut usize) {
    loop {
        let engine = u64::try_from(rig.output_frames() - Q).expect("fits");
        if engine >= to.as_u64() {
            return;
        }
        rig.service();
        let step = (engine / BLOCK_Q + 1) * BLOCK_Q;
        advance_output(rig, at(step.min(to.as_u64())), partition, next);
    }
}

const BLOCK_Q: u64 = QUANTUM_FRAMES as u64;

/// One retired or final owner's custody, with source packets named by source and serial.
fn owner_summary(rig_admitted: &[Admitted], teardown: &MixedBridgeTeardown) -> String {
    let transfer = teardown.transfer();
    let open: Vec<_> = teardown
        .open()
        .iter()
        .map(|(packet, state)| {
            let name = packet.map(|packet| {
                format!(
                    "s{}#{}",
                    admitted_in(rig_admitted, packet).source,
                    packet.serial()
                )
            });
            format!("{name:?} {state:?}")
        })
        .collect();
    format!(
        "custody {:?} holds {:?} open {open:?} pedals {} held {} outstanding {} \
         activation {:?} panic {:?} fault {:?}",
        transfer.custody(),
        transfer.holds_outstanding(),
        teardown.pedals().len(),
        teardown.held().len(),
        transfer.outstanding().len(),
        transfer.activation(),
        teardown.panic(),
        teardown.fault(),
    )
}

/// Two committed resets on the two-source host, each followed by a later activation on
/// its successor (draft Items 2 and 3).
///
/// Owner A stops at 192. The first source holds key 60 from 100 until 1100, across both
/// resets. The second source presses its pedal at 110 and plays key 64 from 120 to 130,
/// so 64 is sustained; it plays key 67 at 430 while the first reset is sealed for 448
/// and lifts its pedal at 500. Successor B takes over at 448: the old lift is consumed
/// as a reset tombstone, a Start at 640 runs B's compiled program, a Stop at 768 stops
/// it, and key 67 is still held when the second reset is sealed for 1024. Successor C
/// takes over at 1024 and seeks at 1216; the key-ups of 60 and 67 are consumed as reset
/// tombstones there, and C plays key 62 from 1150 to 1300.
///
/// The stray offers, all refused whole: A's seek offered while the first reset is
/// pending, an offer prepared between `B` and the commit, A's seek again after the
/// commit, though it supersedes the very sequence B holds in force, and a Start
/// prepared against stopped B, offered while the second reset is pending and again
/// after its commit (rule 3.5).
fn repeated_reset_run(mode: Repeated, partition: &[usize]) -> RepeatedRun {
    let reset = mode != Repeated::Reference;
    let probe = mode == Repeated::Probed;
    let initial = ActivationSequence::INITIAL.next().expect("armed sequence");
    let started = initial.next().expect("B's Start sequence");
    let b_seek = started.next().expect("B's seek sequence");
    let (b1, b2) = (at(RESET_B), at(RESET_C));
    let mut next = 0;
    let mut rig = Rig::new(Setup::default());
    if reset {
        rig.wrap_for_reset(at(192));
    }
    rig.prime();
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(150)]);
    rig.send_all(
        SECOND,
        &[
            message(110, sustain(true)),
            message(120, note_on(64)),
            message(130, note_off(64)),
            frontier(150),
        ],
    );
    serve_to(&mut rig, at(384), partition, &mut next);

    // The first reset, sealed for `B1` and pending from 384.
    let mut stale_a = None;
    let mut b_port = None;
    if reset {
        let offer = rig
            .control
            .prepare_activation(
                at(640),
                PlanPosition::ZERO,
                at(640),
                MixedActivationKind::Seek,
            )
            .expect("A's seek prepares");
        assert_eq!(offer.supersedes(), initial);
        b_port = Some(rig.seal_and_offer(b1));
        if probe {
            let (reason, offer) = rig
                .reset_wrapper()
                .offer_activation(offer)
                .expect_err("a pending reset wins at the caller boundary");
            assert_eq!(reason, MixedActivationRefusal::ResetPending { at: b1 });
            assert_eq!(rig.reset_wrapper().activation(), MixedActivationSlot::Empty);
            stale_a = Some(offer);
        }
    }
    rig.send_all(SECOND, &[message(430, note_on(67)), frontier(470)]);
    rig.send(FIRST, frontier(470));
    serve_to(&mut rig, at(456), partition, &mut next);

    if reset {
        let MixedBridgeResetState::Fading { receipt, .. } = rig.reset_wrapper().state() else {
            panic!(
                "the first reset did not apply: {:?}",
                rig.reset_wrapper().state()
            );
        };
        assert_eq!(receipt.at(), b1);
        assert_eq!(
            receipt.ended().count(),
            2,
            "A ends held 60 and sustained 64"
        );
        if probe {
            // Prepared between `B` and the commit, it still names A's transfer.
            let window = rig
                .control
                .prepare_activation(
                    at(640),
                    PlanPosition::ZERO,
                    at(640),
                    MixedActivationKind::Seek,
                )
                .expect("the window offer prepares against A");
            let (reason, window) = rig
                .reset_wrapper()
                .offer_activation(window)
                .expect_err("the active successor refuses A's offer");
            assert_eq!(reason, MixedActivationRefusal::ForeignTransfer);
            assert_eq!(window.effective(), at(640), "returned whole");
        }
        let old = rig.control.commit_reset(receipt).expect("first commit");
        rig.reset_old.push((
            old,
            std::mem::replace(&mut rig.control_port, b_port.take().expect("B's port")),
        ));
    }
    rig.send_all(SECOND, &[message(530, sustain(false)), frontier(600)]);
    rig.send(FIRST, frontier(600));
    serve_to(&mut rig, at(576), partition, &mut next);

    if reset {
        assert!(matches!(
            rig.reset_wrapper().state(),
            MixedBridgeResetState::Retired(_)
        ));
        rig.retire_reset();
        assert_eq!(rig.reset_wrapper().state(), MixedBridgeResetState::Empty);
        let start = rig
            .control
            .prepare_activation(
                at(640),
                PlanPosition::ZERO,
                at(640),
                MixedActivationKind::Start,
            )
            .expect("B's Start prepares");
        assert_eq!(start.supersedes(), initial);
        if let Some(offer) = stale_a.take() {
            // The sharp case: A's seek supersedes the very sequence B holds in force.
            assert_eq!(offer.supersedes(), start.supersedes());
            let (reason, offer) = rig
                .reset_wrapper()
                .offer_activation(offer)
                .expect_err("A's seek is foreign to B");
            assert_eq!(reason, MixedActivationRefusal::ForeignTransfer);
            assert_eq!(
                (offer.effective(), offer.kind()),
                (at(640), MixedActivationKind::Seek),
                "returned whole"
            );
        }
        rig.reset_wrapper()
            .offer_activation(start)
            .expect("B takes its own Start");
    }
    serve_to(&mut rig, at(648), partition, &mut next);

    let mut kept = None;
    let mut c_port = None;
    if reset {
        let (receipt, adopted_start) = adopted(rig.reset_wrapper().activation());
        assert_eq!(receipt.kind(), MixedActivationKind::Start);
        assert_eq!(receipt.supersedes(), initial);
        assert_eq!(receipt.sequence(), started);
        assert_eq!(adopted_start.effective(), at(640));
        assert!(!adopted_start.stopped(), "the Start runs B's program");
        assert_eq!(
            adopted_start.released(),
            HeldNoteCount::NONE,
            "a reset successor starts stopped with no compiled note sounding"
        );
        let retired = rig
            .reset_wrapper()
            .take_retired_activation()
            .expect("B's retired Start");
        assert_eq!(
            rig.control.collect_activation(retired).expect("collected"),
            receipt
        );
        // B's compiled note sounds from 650; the seek restarts it beside held key 67.
        let seek = rig
            .control
            .prepare_activation(
                at(704),
                PlanPosition::ZERO,
                at(704),
                MixedActivationKind::Seek,
            )
            .expect("B's seek prepares");
        assert_eq!(seek.supersedes(), started);
        rig.reset_wrapper()
            .offer_activation(seek)
            .expect("B takes its own seek");
    }
    serve_to(&mut rig, at(712), partition, &mut next);
    if reset {
        let (receipt, seeked) = adopted(rig.reset_wrapper().activation());
        assert_eq!(receipt.kind(), MixedActivationKind::Seek);
        assert_eq!(seeked.effective(), at(704));
        assert!(!seeked.stopped());
        assert_eq!(
            seeked.released(),
            HeldNoteCount::measured(1),
            "the boundary release ends B's compiled note and not held key 67"
        );
        let retired = rig
            .reset_wrapper()
            .take_retired_activation()
            .expect("B's retired seek");
        assert_eq!(
            rig.control.collect_activation(retired).expect("collected"),
            receipt
        );
        assert_eq!(
            rig.reset_wrapper().schedule_stop(at(768)),
            Ok(MixedBridgeStopAccepted::Scheduled)
        );
    }
    rig.send_all(FIRST, &[frontier(900)]);
    rig.send_all(SECOND, &[frontier(900)]);
    serve_to(&mut rig, at(960), partition, &mut next);

    // The second reset, sealed for `B2` with key 67 still held by B.
    if reset {
        let start = rig
            .control
            .prepare_activation(
                at(1_088),
                PlanPosition::ZERO,
                at(1_088),
                MixedActivationKind::Start,
            )
            .expect("a Start prepares against stopped B");
        assert_eq!(start.supersedes(), b_seek);
        c_port = Some(rig.seal_and_offer(b2));
        if probe {
            let (reason, start) = rig
                .reset_wrapper()
                .offer_activation(start)
                .expect_err("a pending reset wins at the caller boundary");
            assert_eq!(reason, MixedActivationRefusal::ResetPending { at: b2 });
            kept = Some(start);
        }
    }
    rig.send_all(FIRST, &[frontier(1_000)]);
    rig.send_all(SECOND, &[frontier(1_000)]);
    serve_to(&mut rig, at(1_032), partition, &mut next);

    if reset {
        let MixedBridgeResetState::Fading { receipt, .. } = rig.reset_wrapper().state() else {
            panic!(
                "the second reset did not apply: {:?}",
                rig.reset_wrapper().state()
            );
        };
        assert_eq!(receipt.at(), b2);
        assert!(
            matches!(
                receipt.ended().collect::<Vec<_>>().as_slice(),
                [MixedBridgeResetEnd::Tombstoned(_)]
            ),
            "B ends only key 67"
        );
        let old = rig.control.commit_reset(receipt).expect("second commit");
        rig.reset_old.push((
            old,
            std::mem::replace(&mut rig.control_port, c_port.take().expect("C's port")),
        ));
    }
    rig.send_all(
        FIRST,
        &[
            message(1_120, note_off(60)),
            message(1_150, note_on(62)),
            message(1_300, note_off(62)),
            frontier(1_400),
        ],
    );
    rig.send_all(SECOND, &[message(1_130, note_off(67)), frontier(1_400)]);
    serve_to(&mut rig, at(1_152), partition, &mut next);

    if reset {
        rig.retire_reset();
        if let Some(start) = kept.take() {
            // Rule 3.5: the reset while stopped invalidated B's prepared Start.
            let (reason, start) = rig
                .reset_wrapper()
                .offer_activation(start)
                .expect_err("B's Start is foreign to C");
            assert_eq!(reason, MixedActivationRefusal::ForeignTransfer);
            assert_eq!(start.effective(), at(1_088), "returned whole");
        }
        let seek = rig
            .control
            .prepare_activation(
                at(1_216),
                PlanPosition::ZERO,
                at(1_216),
                MixedActivationKind::Seek,
            )
            .expect("C's seek prepares");
        assert_eq!(seek.supersedes(), initial);
        rig.reset_wrapper()
            .offer_activation(seek)
            .expect("C takes its own seek");
    }
    serve_to(&mut rig, at(1_280), partition, &mut next);
    if reset {
        let (receipt, seeked) = adopted(rig.reset_wrapper().activation());
        assert_eq!(receipt.kind(), MixedActivationKind::Seek);
        assert_eq!(seeked.effective(), at(1_216));
        assert!(
            seeked.stopped(),
            "C's program stays stopped across its seek"
        );
        assert_eq!(
            seeked.released(),
            HeldNoteCount::NONE,
            "stopped C sounds no compiled note, and held key 62 is not released"
        );
        let retired = rig
            .reset_wrapper()
            .take_retired_activation()
            .expect("C's retired seek");
        assert_eq!(
            rig.control.collect_activation(retired).expect("collected"),
            receipt
        );
    }
    serve_to(&mut rig, at(1_472), partition, &mut next);
    for source in [FIRST, SECOND] {
        assert_eq!(
            rig.control.tombstone_count(rig.generation(source)),
            Some(MixedTransferPlaces::new(0)),
            "every reset tombstone of source {source} was consumed"
        );
    }

    let reports = normalized_reports(&rig);
    let splits = trace(&rig, None).splits;
    let audio: Vec<_> = rig.rendered.iter().map(|sample| sample.to_bits()).collect();
    let failed: Vec<_> = rig
        .callbacks
        .iter()
        .filter(|(_, _, rendered)| rendered.is_err())
        .take(3)
        .copied()
        .collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(
        !joined.host_halted && !joined.raw_halted && joined.errors.is_empty(),
        "{mode:?}: host {} raw {} errors {:?} callbacks {failed:?}",
        joined.host_halted,
        joined.raw_halted,
        joined.errors
    );
    let teardown = &joined.teardown;
    assert!(teardown.held().is_empty() && teardown.open().is_empty());
    assert_eq!(teardown.fault(), None);
    assert_eq!(teardown.transfer().holds_outstanding(), EventCount::NONE);
    assert_eq!(teardown.transfer().activation(), MixedActivationSlot::Empty);
    assert_eq!(
        teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "raw packet {packet:?} did not record"
        );
    }
    let packet = |source, frames| joined.packet_of(source, frames);
    let refused_up = |onset| {
        Settled::Stage(Ok(MixedStaged::Split(MixedSplit::AfterRefusedOnset {
            onset,
            reason: MixedOnsetRefusal::Reset,
        })))
    };
    let new_on = packet(FIRST, 1_150);
    assert!(matches!(
        joined.report_of(packet(FIRST, 1_300)).expect("C's key-up").outcome(),
        MixedBridgeOutcome::KeyUp { onset, result: Ok(MixedKeyUpRetired::Released(_)) }
            if onset == new_on
    ));
    let retired: Vec<_> = joined
        .retired_reset
        .iter()
        .map(|old| {
            let handles: Vec<_> = old
                .handles()
                .iter()
                .map(|(packet, handle)| {
                    format!(
                        "s{}#{} {handle:?}",
                        admitted_in(&joined.admitted, *packet).source,
                        packet.serial()
                    )
                    .replace(&format!("{:?}", old.receipt().at()), "B")
                })
                .collect();
            format!(
                "at {} handles {} {}",
                old.receipt().at(),
                handles.len(),
                owner_summary(&joined.admitted, old.bridge())
            )
        })
        .collect();
    if reset {
        assert_eq!(
            joined.sound(packet(FIRST, 1_120)),
            refused_up(packet(FIRST, 100))
        );
        assert_eq!(
            joined.sound(packet(SECOND, 1_130)),
            refused_up(packet(SECOND, 430))
        );
        assert!(
            matches!(
                joined.sound(packet(SECOND, 530)),
                Settled::Stage(Ok(MixedStaged::Split(MixedSplit::AfterRefusedPedal {
                    down,
                    reason: MixedOnsetRefusal::Reset,
                })))
                    | Settled::Pump(MixedSplit::AfterRefusedPedal {
                        down,
                        reason: MixedOnsetRefusal::Reset,
                    }) if down == packet(SECOND, 110)
            ),
            "A's lift never reaches B: {:?}",
            joined.sound(packet(SECOND, 530))
        );
        let [a, b] = joined.retired_reset.as_slice() else {
            panic!("two retired owners joined");
        };
        assert_eq!((a.receipt().at(), b.receipt().at()), (b1, b2));
        let ended = |old: &MixedBridgeRetiredTeardown| {
            let mut ended: Vec<_> = old
                .handles()
                .iter()
                .map(|(packet, handle)| {
                    let kind = match handle {
                        MixedRetiredHandle::Ended(MixedBridgeResetEnd::Tombstoned(_)) => {
                            "tombstoned"
                        }
                        MixedRetiredHandle::Ended(MixedBridgeResetEnd::Retired(_)) => "retired",
                        other => panic!("an old handle was not ended by its reset: {other:?}"),
                    };
                    (*packet, kind)
                })
                .collect();
            ended.sort_unstable_by_key(|(packet, _)| packet.serial());
            ended
        };
        let mut a_ended = ended(a);
        a_ended.sort_unstable_by_key(|(packet, _)| admitted_in(&joined.admitted, *packet).source);
        assert_eq!(
            a_ended,
            [
                (packet(FIRST, 100), "tombstoned"),
                (packet(SECOND, 120), "retired")
            ],
            "A's Reset group ends held 60 and sustained 64, once"
        );
        assert_eq!(
            ended(b),
            [(packet(SECOND, 430), "tombstoned")],
            "B's Reset group ends only 67; 60 never reached B"
        );
        for old in [a, b] {
            let transfer = old.bridge().transfer();
            assert_eq!(transfer.holds_outstanding(), EventCount::NONE);
            assert_eq!(transfer.activation(), MixedActivationSlot::Empty);
            assert_eq!(transfer.custody(), MixedTransferCustody::Complete);
            assert!(old.bridge().held().is_empty());
            assert_eq!(old.bridge().fault(), None);
        }
        let on_67 = joined.report_of(packet(SECOND, 430)).expect("67 onset");
        assert!(matches!(on_67.outcome(), MixedBridgeOutcome::NoteOn(Ok(_))));
        assert!(on_67.executed_at() >= b1, "67 sounds in B, not A");
    } else {
        assert!(joined.retired_reset.is_empty());
    }
    let last = owner_summary(&joined.admitted, &joined.teardown);
    RepeatedRun {
        audio,
        raw: raw_trace(&joined.admitted, &joined.receipts),
        reports,
        splits,
        retired,
        last,
    }
}

#[test]
fn repeated_resets_carry_successor_activations_and_hand_off_live_obligations_in_every_partition() {
    let reference = repeated_reset_run(Repeated::Reference, &[Q]);
    let quiet = repeated_reset_run(Repeated::Quiet, &[Q]);
    assert_eq!(quiet.raw, reference.raw, "the resets changed raw capture");
    // Output index `i` holds engine sample `i - 64`. Each successor sounds after its fade.
    for (name, from, to) in [("B", 640, 704), ("C", 1_216, 1_280)] {
        assert!(
            audible(&quiet.audio[from + Q..to + Q]),
            "successor {name} was silent after its activation"
        );
    }
    for partition in RESET_PARTITIONS {
        assert_eq!(
            repeated_reset_run(Repeated::Reference, partition).raw,
            reference.raw,
            "partition {partition:?} changed the reference raw capture"
        );
        assert_eq!(
            repeated_reset_run(Repeated::Quiet, partition),
            quiet,
            "partition {partition:?} changed sound, reports, raw capture or custody"
        );
        assert_eq!(
            repeated_reset_run(Repeated::Probed, partition),
            quiet,
            "partition {partition:?}: a refused stray offer changed the run"
        );
    }
}

#[test]
fn serial_queue_observers_retain_push_stage_and_raw_reservation_peaks() {
    let mut rig = Rig::new(Setup {
        raw_ring: MixedTransferPlaces::new(32),
        ..Setup::default()
    });
    assert!(rig.callback_observations.is_none());
    rig.prime();
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(101, note_on(62)),
            frontier(200),
        ],
    );
    rig.send_all(
        SECOND,
        &[
            message(110, note_on(64)),
            message(111, note_on(67)),
            frontier(200),
        ],
    );
    rig.service();
    rig.render(256).expect("onset callbacks");
    rig.collect();
    rig.send_all(
        FIRST,
        &[
            message(400, note_off(60)),
            message(401, note_off(62)),
            frontier(450),
        ],
    );
    rig.send_all(
        SECOND,
        &[
            message(410, note_off(64)),
            message(411, note_off(67)),
            frontier(450),
        ],
    );
    rig.service();
    rig.render(256).expect("release callbacks");
    rig.collect();
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(joined.queue_owners.len(), 1);
    assert_eq!(joined.queue_owners[0].commands.get().get(), 4);
    assert_eq!(joined.queue_owners[0].results.get().get(), 4);
    assert_eq!(joined.raw_packet_peak.get().get(), 6);
    // Primed output retains Play plus two startup frontiers until this prefix.
    assert_eq!(joined.raw_completion_peak.get().get(), 9);
    assert_eq!(joined.raw_command_peak.get().get(), 1);
    assert_eq!(joined.raw_action_peak.get().get(), 8);
    for source in [FIRST, SECOND] {
        assert_eq!(joined.source_peaks[source].get(), 3);
        assert_eq!(joined.stage_peaks[source].charged.get(), 4);
        assert_eq!(joined.raw_input_peaks[source].charged.get(), 6);
    }
}

/// Exercise the raw owner's actual admission and release-link storage. No sound
/// entitlement is consulted, and every admitted item is recovered at joined stop.
fn raw_reserved_key_control(
    config: &frozen_capacity::FrozenHostConfig,
    keys: u8,
    refuse_last: bool,
) {
    let raw = raw_owner_with_budgets(
        u32::try_from(config.raw_inputs.get()).expect("configured raw cells"),
        config.profile.capabilities().sample_rate(),
        config.profile.capabilities().maximum_block_size(),
        config.raw_commands,
        config.raw_actions,
        config.raw_action_bytes,
    );
    raw_reserved_key_control_owner(raw, keys, refuse_last);
}

fn raw_reserved_key_control_owner(mut raw: RawOwner, keys: u8, refuse_last: bool) {
    let mut accepted = Vec::new();
    'sources: for source in [FIRST, SECOND] {
        let generation = raw.generations[source];
        assert_eq!(
            raw.control
                .input(generation)
                .expect("input")
                .pressure()
                .occupied()
                .as_usize(),
            1,
            "capture binding retains one startup frontier"
        );
        let mut onsets = Vec::new();
        for note in 0..keys {
            let observation = message(100 + u64::from(note), note_on(note));
            let result = raw.control.offer_observation(generation, observation);
            if refuse_last && note + 1 == keys {
                assert_eq!(result, Err((observation, InputError::ProtectedCapacity)));
                // Actual raw overrun is terminal; do not fabricate later releases
                // on this negative path. Joined stop returns its admitted prefix.
                break 'sources;
            }
            onsets.push(result.expect("raw onset independent of sound G4"));
        }
        let pressure = raw.control.input(generation).expect("input").pressure();
        assert_eq!(pressure.release_reservations().as_usize(), onsets.len());
        for (note, onset) in onsets.into_iter().enumerate() {
            let note = u8::try_from(note).expect("MIDI note");
            let id = raw
                .control
                .offer_observation(generation, message(200 + u64::from(note), note_off(note)))
                .expect("reserved key-up despite retained onsets");
            assert_eq!(raw.control.matched_onset(id).expect("link"), Some(onset));
            accepted.push((id, onset));
        }
        assert_eq!(
            raw.control
                .input(generation)
                .expect("input")
                .pressure()
                .release_reservations()
                .as_usize(),
            0
        );
        let _frontier = raw
            .control
            .offer_observation(generation, frontier(300))
            .expect("frontier remains admissible");
    }
    raw.halt.request_stop();
    let mut audio = raw.audio.take().expect("audio");
    audio.synchronize_halt().expect("joined raw stop");
    let (receipts, error) = reunite_raw(audio, raw.control, raw.generations);
    assert!(error.is_none(), "all raw custody rejoins");
    for (id, onset) in accepted {
        let matches: Vec<_> = receipts.iter().filter(|receipt| receipt.id == id).collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].matched_onset, Some(onset));
    }
}

#[test]
fn raw_reserved_keys_control_prepares_every_reference_configuration() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let config = frozen_capacity::reference_host(
            synth_engine_v2::quantities::SampleRate::new(hz).expect("rate"),
        )
        .expect("reference");
        raw_reserved_key_control(&config, 64, false);
    }
}

#[test]
fn raw_sixty_four_keys_per_source_preserve_all_keyups_and_frontiers() {
    raw_reserved_key_control_owner(raw_owner(136), 64, false);
}

#[test]
fn raw_admission_floor_controls_use_real_guards_not_rounded_selection_minus_one() {
    // Before the second onset: startup1 + onset1 + reservation1 =3;
    // onset's four-cell guard makes7 the exact boundary. Six refuses it.
    raw_reserved_key_control_owner(raw_owner(7), 2, false);
    raw_reserved_key_control_owner(raw_owner(6), 2, true);
    // Before onset64: startup1 +63*(onset1+reservation1)=127;
    // therefore131 admits and130 refuses.136 remains the preserved entitlement.
    raw_reserved_key_control_owner(raw_owner(131), 64, false);
    raw_reserved_key_control_owner(raw_owner(130), 64, true);
}

#[test]
fn raw_session_queue_limits_bound_real_prepared_credits_at_n_and_n_minus_one() {
    for capacity in [3, 4] {
        let mut raw = raw_owner_with_limits(
            136,
            profile().capabilities().sample_rate(),
            FrameCount::new(BLOCK),
            SessionCommandCapacity::new(capacity).expect("commands"),
            SessionSourceCapacity::new(256).expect("actions"),
        );
        let mut audio = raw.audio.take().expect("audio");
        for index in 0..capacity {
            let packet = raw
                .control
                .prepare_command(at(64 * (1 + u64::from(index))), SessionCommand::Stop)
                .expect("admitted command credit");
            audio.enqueue(packet).expect("real raw command queue");
            assert_eq!(
                audio.rehearsal_queue_occupancy().expect("usage").0.get(),
                index + 1
            );
        }
        let refused = raw.control.prepare_command(at(640), SessionCommand::Stop);
        assert!(
            matches!(
                refused,
                Err(InputCaptureError::Recording(LoopSessionError::Session(
                    SessionError::Full
                )))
            ),
            "N+1 has no command credit: {refused:?}"
        );
        raw.halt.request_stop();
        audio.synchronize_halt().expect("stop");
        assert_eq!(
            audio
                .rehearsal_queue_occupancy()
                .expect("retained completed slots")
                .0
                .get(),
            capacity
        );
        while let Some(completion) = audio.take_completed() {
            let _outcome = raw
                .control
                .collect(completion)
                .expect("exact command credit returned");
        }
        assert_eq!(
            audio.rehearsal_queue_occupancy().expect("drained slots"),
            (EventCount::NONE, EventCount::NONE)
        );
        let (_, error) = reunite_raw(audio, raw.control, raw.generations);
        assert!(error.is_none());
    }
    // The preserved default entitlement is256 source actions.255 must stop the
    // same256-action prepared prefix at255, with the remainder still in raw input.
    for capacity in [255, 256] {
        let mut raw = raw_owner_with_limits(
            136,
            profile().capabilities().sample_rate(),
            FrameCount::new(BLOCK),
            SessionCommandCapacity::new(4).expect("commands"),
            SessionSourceCapacity::new(capacity).expect("actions"),
        );
        for source in [FIRST, SECOND] {
            for index in 1..128 {
                let _id = raw
                    .control
                    .offer_observation(raw.generations[source], frontier(index))
                    .expect("retained frontier prefix");
            }
        }
        let mut audio = raw.audio.take().expect("audio");
        for expected in 1..=capacity {
            let packet = raw
                .control
                .next_packet()
                .expect("raw preparation")
                .expect("accepted source credit");
            audio.enqueue(packet).expect("real raw source queue");
            assert_eq!(
                audio.rehearsal_queue_occupancy().expect("usage").1.get(),
                expected
            );
        }
        assert!(raw.control.next_packet().expect("credit limit").is_none());
        raw.halt.request_stop();
        audio.synchronize_halt().expect("stop");
        assert_eq!(
            audio
                .rehearsal_queue_occupancy()
                .expect("retained completed slots")
                .1
                .get(),
            capacity
        );
        while let Some(completion) = audio.take_completed() {
            let _outcome = raw
                .control
                .collect(completion)
                .expect("exact source credit returned");
        }
        assert_eq!(
            audio.rehearsal_queue_occupancy().expect("drained slots"),
            (EventCount::NONE, EventCount::NONE)
        );
        let (_, error) = reunite_raw(audio, raw.control, raw.generations);
        assert!(error.is_none());
    }
}

#[test]
fn raw_ring_n_minus_one_retains_the_exact_packet_and_completion_under_backpressure() {
    // Frozen baseline entitlement N=64: Play plus two start fences and61
    // independently admitted source frontiers. Both peers advance to60 so the
    // raw merger can prepare the complete prefix before any callback. The 64th item remains owned when
    // the corresponding physical ring has only 63 places; backpressure is not
    // misreported as a source-admission refusal.
    for (packets, completions) in [(64, 64), (63, 64), (64, 63)] {
        let mut rig = Rig::new(Setup {
            raw_ring: MixedTransferPlaces::new(packets),
            raw_completion_ring: MixedTransferPlaces::new(completions),
            ..Setup::default()
        });
        for tick in 1..=60 {
            rig.send(FIRST, frontier(tick));
            rig.stage_all();
        }
        rig.send(SECOND, frontier(60));
        rig.stage_all();
        let RawAdmission::Admitted { id: tail_input, .. } =
            rig.admitted.last().expect("tail source frontier").raw
        else {
            panic!("raw admitted tail");
        };
        let returned_packet = rig.raw_unsent.as_ref().map(LoopTransferPacket::id);
        assert_eq!(rig.raw_packet_peak.get().get(), packets);
        assert_eq!(rig.raw_unsent.is_some(), packets == 63);
        // First callback consumes priming; the next executes the complete prefix.
        rig.render(Q).expect("priming");
        rig.render(Q).expect("raw prefix");
        assert_eq!(
            rig.raw_completion_peak.get().get(),
            completions.min(packets)
        );
        let callback = rig.callback.as_mut().expect("callback");
        // flush_raw checks capacity before taking. A full completion ring leaves
        // its exact last completion inside raw, rather than inventing a failed push.
        let retained_completion = callback.raw.take_completed();
        assert_eq!(retained_completion.is_some(), completions == 63);
        let returned_completion = retained_completion.as_ref().map(LoopTransferCompletion::id);
        callback.failed = retained_completion;
        let joined = rig.join();
        if let Some(id) = returned_packet {
            assert_eq!(id.serial(), 64, "exact last prepared packet returned");
        }
        if let Some(id) = returned_completion {
            assert_eq!(id.serial(), 64, "exact last prepared completion retained");
        }
        let tail: Vec<_> = joined
            .receipts
            .iter()
            .filter(|receipt| receipt.id == tail_input)
            .collect();
        assert_eq!(
            tail.len(),
            1,
            "last source observation returns exactly once"
        );
        assert_eq!(tail[0].observation, frontier(60));
        assert_eq!(
            tail[0].state,
            if packets == 63 {
                RawState::Cancelled
            } else {
                RawState::Fenced
            }
        );
        joined.assert_each_packet_once();
        assert!(joined.raw_errors.is_empty() && joined.raw_packet_errors.is_empty());
        assert!(joined.raw_collect_refusals.is_empty() && joined.raw_cancel_refusals.is_empty());
        assert!(joined.raw_receipt_errors.is_empty() && joined.raw_join_error.is_none());
    }
}

#[test]
fn an_edited_mixed_plan_changes_only_successor_audio_and_preserves_raw_and_reset_custody() {
    let (original, raw) = common_host_reset_run(true, false);
    let (edited, edited_raw) =
        common_host_reset_with_plan(true, false, plan_with(0.125, profile()));
    let (partitioned, partitioned_raw) =
        common_host_reset_with_plan(true, true, plan_with(0.125, profile()));
    assert_eq!(edited_raw, raw);
    assert_eq!(partitioned_raw, raw);
    assert_eq!(partitioned, edited);
    assert_eq!(&edited[..512], &original[..512]);
    assert!(original[576..].iter().any(|bits| bits & 0x7fff_ffff != 0));
    for (old, new) in original[576..].iter().zip(&edited[576..]) {
        assert_eq!(f32::from_bits(*new), f32::from_bits(*old) * 0.5);
    }
}

#[test]
fn mixed_reset_refuses_a_rate_change_and_returns_the_complete_successor() {
    let mut rig = Rig::new(Setup::default());
    rig.wrap_for_reset(at(192));
    rig.prime();
    rig.send(FIRST, frontier(2_000));
    rig.send(SECOND, frontier(2_000));
    rig.service();
    let mut next = 0;
    advance_output(&mut rig, at(320), &[64], &mut next);
    let other = HostProfile::harness(
        synth_engine_v2::quantities::SampleRate::new(96_000.0).expect("rate"),
        FrameCount::new(BLOCK),
        ChannelLayout::Mono,
    )
    .expect("profile");
    let (prepared, candidate) = prepared_plan_at(plan_with(0.25, other), at(448));
    let MixedBridgeArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = arm_prepared(&rig.raw, Setup::default(), prepared, candidate, other);
    rig.control.seal_reset(control, at(448)).expect("seal");
    let successor = MixedBridgeResetOwner::new(audio, audio_port).expect("bound owner");
    let (reason, successor) = rig
        .reset_wrapper()
        .offer_reset(successor, at(448))
        .expect_err("rate change requires device re-prepare");
    assert_eq!(reason, MixedBridgeResetRefusal::ForeignSuccessor);
    let unused = rig
        .control
        .abandon_reset(&successor)
        .expect("abandon retains control");
    let (audio, audio_port) = successor.into_parts();
    let joined = unused
        .join(audio, Some(control_port), Some(audio_port))
        .expect("complete refused successor custody");
    assert_eq!(joined.transfer().custody(), MixedTransferCustody::Complete);
    rig.join().assert_each_packet_once();
}
