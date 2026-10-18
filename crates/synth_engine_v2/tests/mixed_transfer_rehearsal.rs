//! ADR-0078's host-owned mixed transfer rehearsal, through the public API only.
//!
//! This binary links the library without `cfg(test)`, so it proves the rehearsal is
//! reachable by a non-test host behind `mixed-transfer-rehearsal` alone. The lock-free
//! host queue is `ringbuf`'s preallocated SPSC ring; allocation is counted around the
//! complete callback by the crate-internal fixtures, which own the counting allocator.
#![cfg(feature = "mixed-transfer-rehearsal")]

mod common;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use synth_core::MidiChannel;
use synth_engine_v2::{
    host::mixed_targets::MixedTargetAdmission,
    ingress::{ExhaustedResource, IngressRefused},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations},
    ir::{PortId, SignalDomain},
    plan::CompiledPlan,
    profile::HostProfile,
    quantities::{
        Amplitude, Cents, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity,
        NormalizedLevel, NoteVelocity, Seconds,
    },
    render::AudioBlockMut,
    schedule::{AdmittedCompiledStream, CompiledPayload, PlanEvent},
    stream::{
        MixedJoinedPrepared, MixedJoinedStream, MixedKeyUpHandle, MixedKeyUpRetired,
        MixedSourceBend, MixedSourceKeyUp, MixedSourceNoteOn, MixedStampedCandidate,
        MixedTransferArmError, MixedTransferArmed, MixedTransferAudio, MixedTransferAudioPort,
        MixedTransferCollectError, MixedTransferCommand, MixedTransferConsumer,
        MixedTransferControl, MixedTransferControlPort, MixedTransferCustody, MixedTransferFault,
        MixedTransferHandleState, MixedTransferJoinError, MixedTransferOutcome,
        MixedTransferPlaces, MixedTransferProducer, MixedTransferQueue, MixedTransferRefusal,
        MixedTransferRenderError, MixedTransferReport, MixedTransferResult,
        MixedTransferSubmitError, MixedTransferTeardown,
    },
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor},
};

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const BLOCK: u64 = 512;
const TOTAL: usize = 1_024;
const Q: usize = QUANTUM_FRAMES as usize;

// ---------------------------------------------------------------------------------------
// Host queues.

/// A qualified host queue: a preallocated lock-free SPSC ring. Its reports can be made
/// to lie, and its producer can lose one accepted packet, to inject host faults.
struct HostQueue<T> {
    ring: HeapRb<T>,
    reported_capacity: Option<usize>,
    reported_occupied: Option<usize>,
    lose_push: Option<usize>,
}

impl<T> HostQueue<T> {
    fn new(capacity: usize) -> Self {
        Self {
            ring: HeapRb::new(capacity),
            reported_capacity: None,
            reported_occupied: None,
            lose_push: None,
        }
    }

    /// A ring of `real` places that reports `reported`.
    fn lying(real: usize, reported: usize) -> Self {
        Self {
            reported_capacity: Some(reported),
            ..Self::new(real)
        }
    }
}

struct HostProducer<T> {
    ring: HeapProd<T>,
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
        MixedTransferPlaces::new(
            self.reported_occupied
                .unwrap_or_else(|| self.ring.occupied_len()),
        )
    }

    fn split(self) -> (Self::Producer, Self::Consumer) {
        let (producer, consumer) = self.ring.split();
        (
            HostProducer {
                ring: producer,
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
        self.ring.try_push(packet)
    }
}

impl<T> MixedTransferConsumer<T> for HostConsumer<T> {
    fn try_pop(&mut self) -> Option<T> {
        self.0.try_pop()
    }
}

/// A fault-injection queue the test can reach into, to move packets between transfers or
/// reorder them. It locks, so it is never used where callback purity is measured.
struct Tap<T> {
    packets: Arc<Mutex<VecDeque<T>>>,
    capacity: usize,
}

impl<T> Clone for Tap<T> {
    fn clone(&self) -> Self {
        Self {
            packets: Arc::clone(&self.packets),
            capacity: self.capacity,
        }
    }
}

impl<T> Tap<T> {
    fn new(capacity: usize) -> Self {
        Self {
            packets: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    fn take(&self) -> Option<T> {
        self.packets.lock().expect("tap").pop_front()
    }

    fn put(&self, packet: T) {
        self.packets.lock().expect("tap").push_back(packet);
    }

    fn len(&self) -> usize {
        self.packets.lock().expect("tap").len()
    }
}

impl<T> MixedTransferQueue<T> for Tap<T> {
    type Producer = Self;
    type Consumer = Self;

    fn capacity(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.capacity)
    }

    fn occupied(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.len())
    }

    fn split(self) -> (Self, Self) {
        (self.clone(), self)
    }
}

impl<T> MixedTransferProducer<T> for Tap<T> {
    fn try_push(&mut self, packet: T) -> Result<(), T> {
        let mut packets = self.packets.lock().expect("tap");
        if packets.len() >= self.capacity {
            return Err(packet);
        }
        packets.push_back(packet);
        Ok(())
    }
}

impl<T> MixedTransferConsumer<T> for Tap<T> {
    fn try_pop(&mut self) -> Option<T> {
        self.take()
    }
}

type Commands = HostQueue<MixedTransferCommand>;
type Results = HostQueue<MixedTransferResult>;
type Armed = MixedTransferArmed<Commands, Results>;

// ---------------------------------------------------------------------------------------
// The bound plan and initial one-shot candidate.

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

fn horizon() -> FrameCount {
    profile().limits().events().forward_event_horizon()
}

fn key(raw: u8) -> KeyIdentity {
    KeyIdentity::new(raw).expect("valid key")
}

fn at(frames: u64) -> SampleTime {
    SampleTime::new(frames)
}

/// One compiled producer and one live producer of two notes and two holds, compiled
/// first, through one pitched voice graph.
fn plan() -> CompiledPlan {
    let compiled = NoteProducerDeclaration {
        compiled: true,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::NONE,
    };
    let live = NoteProducerDeclaration {
        compiled: false,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::measured(2),
    };
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
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
            (SOURCE, PortId::FIRST),
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
            held_notes: HeldNoteCount::measured(4),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a readable mixed graph");
    common::admit(&ir, profile())
}

/// The bound initial schedule and a first-activation candidate requested at 64 that
/// replays a compiled note held across the boundary.
fn prepared() -> (MixedJoinedPrepared, MixedStampedCandidate) {
    let plan = plan();
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
        .prepare_history(at(64), PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamped candidate");
    (prepared, candidate)
}

fn arm_with(commands: Commands, results: Results) -> Armed {
    let (prepared, candidate) = prepared();
    prepared
        .arm_transfer_rehearsal(candidate, &profile(), commands, results)
        .expect("transfer arms")
}

fn arm() -> Armed {
    arm_with(HostQueue::new(depth()), HostQueue::new(depth()))
}

fn note_on(frames: u64, raw: u8) -> MixedSourceNoteOn {
    MixedSourceNoteOn {
        at: at(frames),
        channel: MidiChannel::CH1,
        key: key(raw),
        velocity: NoteVelocity::FULL,
    }
}

fn bend(frames: u64, cents: f32) -> MixedSourceBend {
    MixedSourceBend {
        at: at(frames),
        channel: MidiChannel::CH1,
        cents: Cents::new(cents).expect("finite cents"),
    }
}

fn key_up(frames: u64, handle: MixedKeyUpHandle) -> MixedSourceKeyUp {
    MixedSourceKeyUp {
        at: at(frames),
        handle,
    }
}

/// One callback of `frames` frames.
fn callback<C, R>(
    audio: &mut MixedTransferAudio,
    port: &mut MixedTransferAudioPort<C, R>,
    frames: usize,
) -> (Vec<f32>, Result<(), MixedTransferRenderError>)
where
    C: MixedTransferConsumer<MixedTransferCommand>,
    R: MixedTransferProducer<MixedTransferResult>,
{
    let mut samples = vec![0.25_f32; frames];
    let output = AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("block");
    let rendered = audio.render(port, output);
    (samples, rendered)
}

fn silent(samples: &[f32]) -> bool {
    samples.iter().all(|sample| *sample == 0.0)
}

fn collect_all<C, R>(
    control: &mut MixedTransferControl,
    port: &mut MixedTransferControlPort<C, R>,
) -> Vec<MixedTransferReport>
where
    C: MixedTransferProducer<MixedTransferCommand>,
    R: MixedTransferConsumer<MixedTransferResult>,
{
    let mut reports = Vec::new();
    while let Some(report) = control.collect(port).expect("ordered results") {
        reports.push(report);
    }
    reports
}

fn join(armed: Armed) -> MixedTransferTeardown {
    let MixedTransferArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = armed;
    control
        .join(audio, Some(control_port), Some(audio_port))
        .expect("bound wrappers join")
}

fn ids(list: &[synth_engine_v2::stream::MixedTransferCommandId]) -> Vec<u64> {
    list.iter().map(|id| id.get()).collect()
}

// ---------------------------------------------------------------------------------------
// Fault-free transfer.

/// The live performance this fixture plays beside the compiled note.
struct Performance {
    bend: bool,
}

/// Submit a note-on, an optional bend and the exact key-up before the first callback,
/// render `partition`, then collect every result and join.
fn perform(partition: &[usize], performance: &Performance) -> (Vec<f32>, MixedTransferTeardown) {
    let MixedTransferArmed {
        mut control,
        mut audio,
        mut control_port,
        mut audio_port,
    } = arm();
    let (on, handle) = control
        .submit_note_on(&mut control_port, note_on(200, 69))
        .expect("note-on enters the host queue");
    assert_eq!(on.get(), 1);
    if performance.bend {
        let bent = control
            .submit_bend(&mut control_port, bend(300, 200.0))
            .expect("bend enters the host queue");
        assert_eq!(bent.get(), 2);
    }
    let up = control
        .submit_key_up(&mut control_port, key_up(600, handle))
        .expect("the key-up uses the onset's reserved place");
    assert_eq!(control.frontier(), Some(at(600)));

    let mut samples = Vec::with_capacity(TOTAL);
    for &frames in partition {
        let (block, rendered) = callback(&mut audio, &mut audio_port, frames);
        rendered.expect("fault-free callback");
        samples.extend(block);
    }
    assert_eq!(samples.len(), TOTAL);

    let reports = collect_all(&mut control, &mut control_port);
    assert_eq!(reports.len(), if performance.bend { 3 } else { 2 });
    let MixedTransferOutcome::NoteOn {
        handle: onset_handle,
        result: Ok(identity),
    } = reports[0].outcome()
    else {
        panic!("the note-on is accepted: {:?}", reports[0]);
    };
    assert_eq!(onset_handle, handle);
    assert_eq!(reports[0].id(), on);
    assert_eq!(reports[0].at(), at(200), "results keep the nominal stamp");
    assert_eq!(
        reports[0].executed_at(),
        at(200),
        "a command served before its stamp executes at it"
    );
    if performance.bend {
        assert_eq!(reports[1].at(), at(300));
        let MixedTransferOutcome::Bend {
            channel,
            result: Ok(applied),
        } = reports[1].outcome()
        else {
            panic!("the bend applies: {:?}", reports[1]);
        };
        assert_eq!(channel, MidiChannel::CH1);
        assert_eq!(
            applied.members().get(),
            1,
            "the sounding live note takes the bend"
        );
    }
    let last = reports.last().expect("key-up result");
    assert_eq!(last.id(), up);
    assert_eq!(last.at(), at(600));
    assert_eq!(
        last.outcome(),
        MixedTransferOutcome::KeyUp {
            handle,
            result: Ok(MixedKeyUpRetired::Released(identity)),
        }
    );
    assert_eq!(control.fault(), None);
    let teardown = control
        .join(audio, Some(control_port), Some(audio_port))
        .expect("join");
    (samples, teardown)
}

#[test]
fn fault_free_transfer_is_callback_partition_invariant_and_joins_with_complete_custody() {
    let performance = Performance { bend: true };
    let irregular = [1, 63, 130, 7, 300, 2, 64, 457];
    assert_eq!(irregular.iter().sum::<usize>(), TOTAL);
    let partitions: [Vec<usize>; 4] = [
        vec![BLOCK as usize; TOTAL / BLOCK as usize],
        vec![Q; TOTAL / Q],
        vec![256; TOTAL / 256],
        irregular.to_vec(),
    ];
    let (reference, teardown) = perform(&partitions[0], &performance);
    assert!(!silent(&reference), "the compiled and live notes sound");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert!(teardown.outstanding().is_empty());
    assert!(teardown.handles().is_empty());
    assert_eq!(teardown.release_places(), MixedTransferPlaces::NONE);
    assert!(
        teardown
            .sounding_live()
            .expect("renderer snapshot")
            .is_empty()
    );
    assert_eq!(teardown.control_fault(), None);
    assert_eq!(teardown.audio_fault(), None);
    assert_eq!(teardown.counters().beyond_horizon(), 0);
    for partition in &partitions[1..] {
        let (samples, teardown) = perform(partition, &performance);
        assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
        assert!(
            samples
                .iter()
                .zip(&reference)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "callback partition {partition:?} changed the output"
        );
    }

    // The bend is heard from its own stamp and not one sample earlier.
    let (unbent, _) = perform(&partitions[0], &Performance { bend: false });
    let first_difference = reference
        .iter()
        .zip(&unbent)
        .position(|(a, b)| a.to_bits() != b.to_bits())
        .expect("the bend is audible");
    assert!(
        first_difference >= 300,
        "the bend was heard at {first_difference}, before its stamp"
    );
    assert!(
        first_difference < 600,
        "the bend is heard before the key-up"
    );
}

#[test]
fn joined_teardown_reports_a_live_note_until_its_future_key_up_is_published() {
    let MixedTransferArmed {
        mut control,
        mut audio,
        mut control_port,
        mut audio_port,
    } = arm();
    let (_, handle) = control
        .submit_note_on(&mut control_port, note_on(10, 69))
        .expect("onset enters the command queue");
    let (_, rendered) = callback(&mut audio, &mut audio_port, 128);
    rendered.expect("onset callback");
    let reports = collect_all(&mut control, &mut control_port);
    let MixedTransferOutcome::NoteOn {
        result: Ok(identity),
        ..
    } = reports[0].outcome()
    else {
        panic!("the onset must have a sounding identity");
    };

    let key_up_id = control
        .submit_key_up(&mut control_port, key_up(300, handle))
        .expect("future key-up uses its reservation");
    let (_, rendered) = callback(&mut audio, &mut audio_port, 128);
    rendered.expect("key-up service callback");
    let reports = collect_all(&mut control, &mut control_port);
    assert_eq!(reports[0].id(), key_up_id);
    assert!(matches!(
        reports[0].outcome(),
        MixedTransferOutcome::KeyUp {
            result: Ok(MixedKeyUpRetired::Released(_)),
            ..
        }
    ));

    let teardown = control
        .join(audio, Some(control_port), Some(audio_port))
        .expect("joined teardown");
    assert!(teardown.handles().is_empty());
    assert_eq!(
        teardown.sounding_live().expect("renderer snapshot"),
        [identity]
    );
}

#[test]
fn the_audio_half_and_its_wrapper_render_on_another_thread() {
    let MixedTransferArmed {
        mut control,
        audio,
        mut control_port,
        audio_port,
    } = arm();
    let (_, handle) = control
        .submit_note_on(&mut control_port, note_on(100, 64))
        .expect("note-on");
    let up = control
        .submit_key_up(&mut control_port, key_up(400, handle))
        .expect("key-up");
    assert_eq!(up.get(), 2);
    let (audio, audio_port, rendered) = std::thread::spawn(move || {
        let (mut audio, mut audio_port) = (audio, audio_port);
        let mut rendered = Vec::new();
        for _ in 0..TOTAL / Q {
            let (block, result) = callback(&mut audio, &mut audio_port, Q);
            result.expect("callback");
            rendered.extend(block);
        }
        (audio, audio_port, rendered)
    })
    .join()
    .expect("audio thread");
    assert!(!silent(&rendered));
    assert_eq!(collect_all(&mut control, &mut control_port).len(), 2);
    let teardown = control
        .join(audio, Some(control_port), Some(audio_port))
        .expect("join");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
}

// ---------------------------------------------------------------------------------------
// Arm.

#[test]
fn arm_refuses_undersized_or_occupied_queues_and_returns_both_unchanged() {
    let depth = depth();
    let cases: [(Commands, Results, MixedTransferArmError); 4] = [
        (
            HostQueue::new(depth - 1),
            HostQueue::new(depth),
            MixedTransferArmError::CommandCapacity {
                required: MixedTransferPlaces::new(depth),
                reported: MixedTransferPlaces::new(depth - 1),
            },
        ),
        (
            HostQueue::new(depth),
            HostQueue::new(depth - 1),
            MixedTransferArmError::ResultCapacity {
                required: MixedTransferPlaces::new(depth),
                reported: MixedTransferPlaces::new(depth - 1),
            },
        ),
        (
            HostQueue {
                reported_occupied: Some(1),
                ..HostQueue::new(depth)
            },
            HostQueue::new(depth),
            MixedTransferArmError::CommandOccupied {
                occupied: MixedTransferPlaces::new(1),
            },
        ),
        (
            HostQueue::new(depth),
            HostQueue {
                reported_occupied: Some(2),
                ..HostQueue::new(depth)
            },
            MixedTransferArmError::ResultOccupied {
                occupied: MixedTransferPlaces::new(2),
            },
        ),
    ];
    for (commands, results, expected) in cases {
        let (command_capacity, result_capacity) = (
            commands.ring.capacity().get(),
            results.ring.capacity().get(),
        );
        let (prepared, candidate) = prepared();
        let refusal = prepared
            .arm_transfer_rehearsal(candidate, &profile(), commands, results)
            .expect_err("arm refuses");
        assert_eq!(refusal.reason(), expected);
        let (prepared, candidate, commands, results) = refusal.into_parts();
        assert_eq!(commands.ring.capacity().get(), command_capacity);
        assert_eq!(results.ring.capacity().get(), result_capacity);
        assert!(commands.ring.is_empty() && results.ring.is_empty());
        // The returned owner and candidate still arm with qualified queues.
        let armed = prepared
            .arm_transfer_rehearsal(
                candidate,
                &profile(),
                HostQueue::new(depth),
                HostQueue::new(depth),
            )
            .expect("retry arms");
        assert_eq!(join(armed).custody(), MixedTransferCustody::Complete);
    }
}

// ---------------------------------------------------------------------------------------
// Physical push failures.

#[test]
fn a_failed_onset_push_retains_its_packet_handle_and_key_up_place() {
    // The real ring holds one command although it reports the admitted depth.
    let mut armed = arm_with(HostQueue::lying(1, depth()), HostQueue::new(depth()));
    let control = &mut armed.control;
    let first = control
        .submit_bend(&mut armed.control_port, bend(100, 50.0))
        .expect("the bend fills the real ring");
    assert_eq!(first.get(), 1);
    let Err(MixedTransferSubmitError::PushFailed { id, handle }) =
        control.submit_note_on(&mut armed.control_port, note_on(150, 64))
    else {
        panic!("a lying queue's full ring is a terminal push failure");
    };
    assert_eq!(id.get(), 2);
    let handle = handle.expect("the onset's handle is retained");
    assert_eq!(
        control.fault(),
        Some(MixedTransferFault::CommandPush { id })
    );
    assert_eq!(
        control.frontier(),
        Some(at(150)),
        "the observation was consumed"
    );

    // Later submits and collections observe the halt without minting or popping.
    let late = note_on(160, 65);
    let Err(MixedTransferSubmitError::Refused { reason, original }) =
        control.submit_note_on(&mut armed.control_port, late)
    else {
        panic!("a halted transfer refuses");
    };
    assert_eq!((reason, original), (MixedTransferRefusal::Halted, late));
    assert_eq!(
        control.collect(&mut armed.control_port),
        Err(MixedTransferCollectError::Halted(
            MixedTransferFault::CommandPush { id }
        ))
    );

    // The callback that observes the fault, and every later one, is silent.
    let (block, rendered) = callback(&mut armed.audio, &mut armed.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::RemoteHalt));
    assert!(silent(&block));
    let (block, rendered) = callback(&mut armed.audio, &mut armed.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::Faulted));
    assert!(silent(&block));

    let teardown = join(armed);
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert_eq!(ids(teardown.outstanding()), [1, 2]);
    assert_eq!(teardown.unsent(), Some(id));
    assert_eq!(ids(teardown.unserved()), [1], "no command was served");
    assert_eq!(
        teardown.handles(),
        [(handle, MixedTransferHandleState::Open)]
    );
    assert_eq!(
        teardown.release_places(),
        MixedTransferPlaces::new(1),
        "the key-up place stays reserved"
    );
}

#[test]
fn a_failed_key_up_push_moves_its_reserved_place_to_the_pending_command() {
    let mut armed = arm_with(HostQueue::lying(1, depth()), HostQueue::new(depth()));
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(100, 64))
        .expect("note-on fills the real ring");
    let Err(MixedTransferSubmitError::PushFailed { id, handle: failed }) = armed
        .control
        .submit_key_up(&mut armed.control_port, key_up(200, handle))
    else {
        panic!("the key-up push fails");
    };
    assert_eq!((id.get(), failed), (2, Some(handle)));
    let teardown = join(armed);
    assert_eq!(teardown.unsent(), Some(id));
    assert_eq!(
        teardown.handles(),
        [(handle, MixedTransferHandleState::Releasing(id))]
    );
    assert_eq!(
        teardown.release_places(),
        MixedTransferPlaces::NONE,
        "the reserved place became the unsent key-up's charge, not a second charge"
    );
    assert_eq!(ids(teardown.outstanding()), [1, 2]);
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
}

#[test]
fn a_failed_result_push_keeps_the_exact_result_and_serves_no_younger_command() {
    // The real result ring holds one result although it reports the admitted depth.
    let mut armed = arm_with(HostQueue::new(depth()), HostQueue::lying(1, depth()));
    for (frames, raw) in [(10, 60), (20, 62), (30, 64)] {
        let _accepted = armed
            .control
            .submit_note_on(&mut armed.control_port, note_on(frames, raw))
            .expect("note-on");
    }
    let (block, rendered) = callback(&mut armed.audio, &mut armed.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::ResultPush));
    assert!(silent(&block), "the observing callback is silent");
    assert_eq!(
        armed.control.collect(&mut armed.control_port),
        Err(MixedTransferCollectError::Halted(
            MixedTransferFault::Remote
        )),
        "collection observes the halt without popping the queued result"
    );
    let teardown = join(armed);
    assert_eq!(
        teardown.audio_fault(),
        Some(MixedTransferRenderError::ResultPush)
    );
    assert_eq!(ids(teardown.uncollected()), [1]);
    assert_eq!(teardown.pending_result().map(|id| id.get()), Some(2));
    assert_eq!(
        ids(teardown.unserved()),
        [3],
        "no younger command was served"
    );
    assert_eq!(ids(teardown.outstanding()), [1, 2, 3]);
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
}

// ---------------------------------------------------------------------------------------
// Foreign wrappers, handles and packets.

#[test]
fn a_foreign_control_wrapper_or_handle_faults_before_time_or_queue_checks() {
    let mut a = arm();
    let mut b = arm();
    let (_, handle_a) = a
        .control
        .submit_note_on(&mut a.control_port, note_on(500, 60))
        .expect("A's note-on");
    let _accepted = b
        .control
        .submit_note_on(&mut b.control_port, note_on(500, 62))
        .expect("B's note-on");

    // A backward-stamped key-up with A's handle is an owner fault at B, not a time fault.
    let crossed = key_up(100, handle_a);
    let Err(MixedTransferSubmitError::Refused { reason, original }) =
        b.control.submit_key_up(&mut b.control_port, crossed)
    else {
        panic!("B refuses A's handle");
    };
    assert_eq!(
        (reason, original),
        (MixedTransferRefusal::ForeignHandle, crossed)
    );
    assert_eq!(b.control.fault(), Some(MixedTransferFault::ForeignHandle));
    assert_eq!(
        b.control.frontier(),
        Some(at(500)),
        "B's frontier is unchanged"
    );

    // A's control wrapper lent to a halted B returns the halt; A keeps running.
    let Err(MixedTransferSubmitError::Refused { reason, .. }) = b
        .control
        .submit_note_on(&mut a.control_port, note_on(600, 64))
    else {
        panic!("B is halted");
    };
    assert_eq!(reason, MixedTransferRefusal::Halted);
    assert_eq!(a.control.fault(), None);

    // A fresh C faults on A's wrapper before touching A's queue.
    let mut c = arm();
    let foreign = note_on(10, 65);
    let Err(MixedTransferSubmitError::Refused { reason, original }) =
        c.control.submit_note_on(&mut a.control_port, foreign)
    else {
        panic!("C refuses A's wrapper");
    };
    assert_eq!(
        (reason, original),
        (MixedTransferRefusal::ForeignEndpoint, foreign)
    );
    assert_eq!(c.control.frontier(), None);
    assert_eq!(
        c.control.collect(&mut c.control_port),
        Err(MixedTransferCollectError::Halted(
            MixedTransferFault::ForeignEndpoint
        ))
    );

    // A's audio wrapper lent to C's callback: terminal, silent, A's queue untouched.
    let (block, rendered) = callback(&mut c.audio, &mut a.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::RemoteHalt));
    assert!(silent(&block));
    let mut d = arm();
    let (block, rendered) = callback(&mut d.audio, &mut a.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::ForeignEndpoint));
    assert!(silent(&block));
    assert_eq!(
        d.control.collect(&mut d.control_port),
        Err(MixedTransferCollectError::Halted(
            MixedTransferFault::Remote
        ))
    );

    // A still serves its own command through its own wrappers.
    let (_, rendered) = callback(&mut a.audio, &mut a.audio_port, 128);
    rendered.expect("A is unaffected");
    let reports = collect_all(&mut a.control, &mut a.control_port);
    assert_eq!(reports.len(), 1);
    for armed in [b, c, d] {
        let _ = join(armed);
    }
    assert_eq!(join(a).custody(), MixedTransferCustody::Complete);
}

#[test]
fn a_join_with_a_foreign_wrapper_returns_everything_undrained() {
    let mut a = arm();
    let b = arm();
    let _accepted = a
        .control
        .submit_note_on(&mut a.control_port, note_on(10, 60))
        .expect("note-on");
    let MixedTransferArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = a;
    let MixedTransferArmed {
        control: b_control,
        audio: b_audio,
        control_port: b_control_port,
        audio_port: b_audio_port,
    } = b;
    let refusal = control
        .join(audio, Some(control_port), Some(b_audio_port))
        .expect_err("a foreign wrapper refuses the whole join");
    assert_eq!(refusal.reason(), MixedTransferJoinError::ForeignEndpoint);
    let (control, audio, control_port, b_audio_port) = refusal.into_parts();
    let refusal = control
        .join(b_audio, control_port, Some(audio_port))
        .expect_err("a crossed audio half refuses too");
    assert_eq!(refusal.reason(), MixedTransferJoinError::CrossedPair);
    let (control, b_audio, control_port, audio_port) = refusal.into_parts();

    // Retrying with the correct wrappers accounts for the queued command.
    let teardown = control
        .join(audio, control_port, audio_port)
        .expect("correct join");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert_eq!(ids(teardown.unserved()), [1]);
    assert_eq!(ids(teardown.outstanding()), [1]);
    let teardown = b_control
        .join(b_audio, Some(b_control_port), b_audio_port)
        .expect("B joins");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
}

type TapArmed = MixedTransferArmed<Tap<MixedTransferCommand>, Tap<MixedTransferResult>>;

fn arm_tapped() -> (
    TapArmed,
    Tap<MixedTransferCommand>,
    Tap<MixedTransferResult>,
) {
    let commands = Tap::new(depth());
    let results = Tap::new(depth());
    let (prepared, candidate) = prepared();
    let armed = prepared
        .arm_transfer_rehearsal(candidate, &profile(), commands.clone(), results.clone())
        .expect("arm");
    (armed, commands, results)
}

#[test]
fn foreign_and_reordered_packets_fault_the_receiving_half_without_mutation() {
    // A foreign command with the expected ID.
    let (mut a, a_commands, _) = arm_tapped();
    let (mut b, b_commands, _) = arm_tapped();
    let _accepted = a
        .control
        .submit_note_on(&mut a.control_port, note_on(10, 60))
        .expect("A");
    let foreign = a_commands.take().expect("A's packet");
    assert_eq!(foreign.id().get(), 1);
    b_commands.put(foreign);
    let (block, rendered) = callback(&mut b.audio, &mut b.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::CommandOrder));
    assert!(silent(&block));
    let teardown = b
        .control
        .join(b.audio, Some(b.control_port), Some(b.audio_port))
        .expect("join");
    assert_eq!(teardown.invalid_command().map(|id| id.get()), Some(1));
    assert!(
        teardown
            .sounding_live()
            .expect("renderer snapshot")
            .is_empty(),
        "nothing was offered"
    );
    drop(a);

    // Reordered commands of one transfer.
    let (mut c, c_commands, _) = arm_tapped();
    for (frames, raw) in [(10, 60), (20, 62)] {
        let _accepted = c
            .control
            .submit_note_on(&mut c.control_port, note_on(frames, raw))
            .expect("C");
    }
    let first = c_commands.take().expect("first");
    let second = c_commands.take().expect("second");
    c_commands.put(second);
    c_commands.put(first);
    let (block, rendered) = callback(&mut c.audio, &mut c.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::CommandOrder));
    assert!(silent(&block));
    // A key-up for any handle now returns the halt rather than `NotOpen`.
    let teardown = c
        .control
        .join(c.audio, Some(c.control_port), Some(c.audio_port))
        .expect("join");
    assert_eq!(teardown.invalid_command().map(|id| id.get()), Some(2));
    assert_eq!(ids(teardown.unserved()), [1]);
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);

    // A foreign result: control keeps it in its invalid-result slot and settles nothing.
    let (mut d, _, d_results) = arm_tapped();
    let (mut e, _, e_results) = arm_tapped();
    let _accepted = d
        .control
        .submit_note_on(&mut d.control_port, note_on(10, 60))
        .expect("D");
    let (_, e_handle) = e
        .control
        .submit_note_on(&mut e.control_port, note_on(10, 60))
        .expect("E");
    callback(&mut d.audio, &mut d.audio_port, 128)
        .1
        .expect("D renders");
    callback(&mut e.audio, &mut e.audio_port, 128)
        .1
        .expect("E renders");
    let foreign = d_results.take().expect("D's result");
    let own = e_results.take().expect("E's result");
    e_results.put(foreign);
    let Err(MixedTransferCollectError::Fault(fault)) = e.control.collect(&mut e.control_port)
    else {
        panic!("E refuses D's result");
    };
    let MixedTransferFault::ResultProtocol { received, expected } = fault else {
        panic!("a foreign result is a result protocol fault: {fault:?}");
    };
    assert_eq!((received.get(), expected.map(|id| id.get())), (1, Some(1)));
    e_results.put(own);
    assert_eq!(
        e.control.collect(&mut e.control_port),
        Err(MixedTransferCollectError::Halted(fault)),
        "a later collection observes the halt and pops nothing"
    );
    assert_eq!(e_results.len(), 1);
    let late = key_up(20, e_handle);
    let Err(MixedTransferSubmitError::Refused { reason, original }) =
        e.control.submit_key_up(&mut e.control_port, late)
    else {
        panic!("a halted transfer refuses");
    };
    assert_eq!((reason, original), (MixedTransferRefusal::Halted, late));
    let (block, rendered) = callback(&mut e.audio, &mut e.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::RemoteHalt));
    assert!(silent(&block));
    let teardown = e
        .control
        .join(e.audio, Some(e.control_port), Some(e.audio_port))
        .expect("join");
    assert_eq!(teardown.invalid_result().map(|id| id.get()), Some(1));
    assert_eq!(ids(teardown.uncollected()), [1]);
    assert_eq!(
        teardown.handles(),
        [(e_handle, MixedTransferHandleState::Open)],
        "the foreign result settled nothing"
    );
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    drop(d);
}

// ---------------------------------------------------------------------------------------
// Key-up order.

#[test]
fn a_retired_handle_key_up_is_not_open_until_it_runs_backward() {
    let mut armed = arm();
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(10, 60))
        .expect("note-on");
    let up = armed
        .control
        .submit_key_up(&mut armed.control_port, key_up(50, handle))
        .expect("key-up");
    assert_eq!(up.get(), 2);
    callback(&mut armed.audio, &mut armed.audio_port, 128)
        .1
        .expect("render");
    assert_eq!(
        collect_all(&mut armed.control, &mut armed.control_port).len(),
        2
    );

    // In order: a non-terminal refusal that mints nothing and keeps the frontier.
    let repeat = key_up(60, handle);
    let Err(MixedTransferSubmitError::Refused { reason, original }) =
        armed.control.submit_key_up(&mut armed.control_port, repeat)
    else {
        panic!("the retired handle is not open");
    };
    assert_eq!((reason, original), (MixedTransferRefusal::NotOpen, repeat));
    assert!(!reason.is_terminal());
    assert_eq!(armed.control.frontier(), Some(at(50)));
    assert_eq!(armed.control.fault(), None);
    let next = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(70, 62))
        .expect("the stream continues")
        .0;
    assert_eq!(next.get(), 3, "the NotOpen refusal minted no ID");

    // Behind the frontier: the time-order fault precedes the open-handle check.
    let backward = key_up(20, handle);
    let Err(MixedTransferSubmitError::Refused { reason, original }) = armed
        .control
        .submit_key_up(&mut armed.control_port, backward)
    else {
        panic!("a backward key-up faults");
    };
    assert_eq!(
        (reason, original),
        (
            MixedTransferRefusal::SourceOrder { frontier: at(70) },
            backward
        )
    );
    assert!(reason.is_terminal());
    let (block, rendered) = callback(&mut armed.audio, &mut armed.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::RemoteHalt));
    assert!(silent(&block), "the atomic halt reaches audio");
    let teardown = join(armed);
    assert_eq!(
        teardown.control_fault(),
        Some(MixedTransferFault::SourceOrder {
            at: at(20),
            frontier: at(70)
        })
    );
    assert_eq!(ids(teardown.unserved()), [3]);
}

// ---------------------------------------------------------------------------------------
// Forward horizon.

#[test]
fn a_beyond_horizon_tombstone_key_up_is_counted_once_and_silences_audio() {
    let mut armed = arm();
    // Two holds are admitted; the third onset is refused and leaves a tombstone.
    let mut handles = Vec::new();
    for raw in [60, 62, 64] {
        handles.push(
            armed
                .control
                .submit_note_on(&mut armed.control_port, note_on(10, raw))
                .expect("note-on")
                .1,
        );
    }
    callback(&mut armed.audio, &mut armed.audio_port, 128)
        .1
        .expect("render");
    let reports = collect_all(&mut armed.control, &mut armed.control_port);
    assert_eq!(
        reports[2].outcome(),
        MixedTransferOutcome::NoteOn {
            handle: handles[2],
            result: Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Hold
            }),
        }
    );

    // The tombstone's key-up beyond the horizon still meets the one horizon check.
    let far = at(128)
        .checked_add(horizon())
        .and_then(|end| end.checked_add(FrameCount::new(1)))
        .expect("representable");
    let far_key_up = MixedSourceKeyUp {
        at: far,
        handle: handles[2],
    };
    let id = armed
        .control
        .submit_key_up(&mut armed.control_port, far_key_up)
        .expect("control does not judge the horizon");
    let (block, rendered) = callback(&mut armed.audio, &mut armed.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::BeyondHorizon));
    assert!(silent(&block));
    assert_eq!(armed.audio.ingress_counters().beyond_horizon(), 1);
    let (block, rendered) = callback(&mut armed.audio, &mut armed.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::Faulted));
    assert!(silent(&block));
    assert_eq!(
        armed.audio.ingress_counters().beyond_horizon(),
        1,
        "counted once"
    );

    let teardown = join(armed);
    assert_eq!(teardown.invalid_command(), Some(id));
    assert_eq!(
        teardown.tombstones().get(),
        1,
        "the tombstone was not consumed"
    );
    assert_eq!(
        teardown.sounding_live().expect("renderer snapshot").len(),
        2
    );
    assert!(
        teardown
            .handles()
            .contains(&(handles[2], MixedTransferHandleState::Releasing(id)))
    );
    assert_eq!(teardown.counters().beyond_horizon(), 1);
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
}

// ---------------------------------------------------------------------------------------
// Missing wrappers and lost packets.

#[test]
fn a_missing_wrapper_reports_missing_endpoint_and_unknown_queue_contents() {
    for omit_audio in [true, false] {
        let MixedTransferArmed {
            mut control,
            audio,
            mut control_port,
            audio_port,
        } = arm();
        let (_, handle) = control
            .submit_note_on(&mut control_port, note_on(10, 60))
            .expect("note-on");
        let teardown = if omit_audio {
            drop(audio_port);
            control.join::<_, _, HostConsumer<MixedTransferCommand>, HostProducer<MixedTransferResult>>(
                audio,
                Some(control_port),
                None,
            )
        } else {
            drop(control_port);
            control.join::<HostProducer<MixedTransferCommand>, HostConsumer<MixedTransferResult>, _, _>(
                audio,
                None,
                Some(audio_port),
            )
        }
        .expect("a missing wrapper still joins");
        assert_eq!(
            teardown.custody(),
            MixedTransferCustody::MissingEndpoint {
                commands_unknown: omit_audio,
                results_unknown: !omit_audio,
            }
        );
        assert_eq!(ids(teardown.outstanding()), [1]);
        assert_eq!(
            teardown.handles(),
            [(handle, MixedTransferHandleState::Open)]
        );
        assert!(
            teardown.lost().is_empty(),
            "lost contents cannot be claimed"
        );
        if omit_audio {
            assert!(teardown.unserved().is_empty());
        } else {
            assert_eq!(
                ids(teardown.unserved()),
                [1],
                "the bound consumer is drained"
            );
        }
    }
}

#[test]
fn a_lost_final_packet_is_first_reported_at_teardown_and_a_gap_faults() {
    // The second accepted command is lost by an unqualified queue.
    let lossy = || HostQueue {
        lose_push: Some(2),
        ..HostQueue::new(depth())
    };
    let mut armed = arm_with(lossy(), HostQueue::new(depth()));
    let _delivered = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(10, 60))
        .expect("delivered");
    let lost = armed
        .control
        .submit_bend(&mut armed.control_port, bend(20, 30.0))
        .expect("accepted and lost");
    assert_eq!(lost.get(), 2);
    callback(&mut armed.audio, &mut armed.audio_port, 128)
        .1
        .expect("no later packet reveals the loss");
    assert_eq!(
        collect_all(&mut armed.control, &mut armed.control_port).len(),
        1
    );
    let teardown = join(armed);
    assert_eq!(teardown.custody(), MixedTransferCustody::Lost);
    assert_eq!(ids(teardown.lost()), [2]);

    // A later packet reveals the gap: audio faults before serving it.
    let mut armed = arm_with(lossy(), HostQueue::new(depth()));
    let _delivered = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(10, 60))
        .expect("delivered");
    let _lost = armed
        .control
        .submit_bend(&mut armed.control_port, bend(20, 30.0))
        .expect("lost");
    let after_gap = armed
        .control
        .submit_bend(&mut armed.control_port, bend(30, 0.0))
        .expect("delivered after the gap");
    assert_eq!(after_gap.get(), 3);
    let (block, rendered) = callback(&mut armed.audio, &mut armed.audio_port, 128);
    assert_eq!(rendered, Err(MixedTransferRenderError::CommandOrder));
    assert!(silent(&block));
    let teardown = join(armed);
    assert_eq!(teardown.invalid_command().map(|id| id.get()), Some(3));
    assert_eq!(ids(teardown.lost()), [2]);
}
