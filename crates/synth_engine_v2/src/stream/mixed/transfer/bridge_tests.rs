//! Crate-internal bridge fixtures: what the public API cannot reach or practically
//! exhaust. The audio boundary is set directly to place a head exactly at the window
//! edge or at the end of the clock, the merger's identity spaces are spent directly, and
//! the custody lists are prepared smaller than the merger's bounds.
//!
//! The reset fixtures run the old bridge's audio half in the public reset wrapper: the
//! old arm takes effect at 64, its compiled Stop applies at 192, and the successor is a
//! fresh bridge armed for `B = 448` over the same two sources.

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};

use super::super::{
    super::{
        CompiledPayload, MixedBridgeResetAudio, MixedBridgeResetRefusal,
        MixedBridgeResetRenderError, MixedBridgeResetState, MixedBridgeStopAccepted,
        MixedIngressCommandId, MixedOneShotArmError, history_tests,
    },
    MixedTransferCustody, MixedTransferOwnerRefusal, MixedTransferRenderError,
};
use super::*;
use crate::{
    host::{
        EndpointId,
        input::{InputCapacity, InputLimits, SimulatedNoteInput},
    },
    quantities::{ChannelLayout, EventCount, PreparedBytes},
    render::AudioBlockMut,
    schedule::PlanEvent,
    time::PlanPosition,
};

struct Ring<T>(HeapRb<T>);

struct Prod<T>(HeapProd<T>);

struct Cons<T>(HeapCons<T>);

impl<T> MixedTransferQueue<T> for Ring<T> {
    type Producer = Prod<T>;
    type Consumer = Cons<T>;

    fn capacity(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.0.capacity().get())
    }

    fn occupied(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.0.occupied_len())
    }

    fn split(self) -> (Prod<T>, Cons<T>) {
        let (producer, consumer) = self.0.split();
        (Prod(producer), Cons(consumer))
    }
}

impl<T> MixedTransferProducer<T> for Prod<T> {
    fn try_push(&mut self, packet: T) -> Result<(), T> {
        self.0.try_push(packet)
    }
}

impl<T> MixedTransferConsumer<T> for Cons<T> {
    fn try_pop(&mut self) -> Option<T> {
        self.0.try_pop()
    }
}

type Armed = MixedBridgeArmed<Ring<MixedTransferCommand>, Ring<MixedTransferResult>>;

/// One bound source: a connection generation a simulated input issued, and the serials
/// its source ring gives its packets, messages and frontiers alike.
struct Source {
    generation: ConnectionGeneration,
    serial: u64,
}

impl Source {
    fn new(name: &str) -> Self {
        let mut input = SimulatedNoteInput::new(
            EndpointId::new(name.to_owned()).expect("endpoint"),
            InputLimits {
                cells: InputCapacity::new(64).expect("cells"),
                bytes: PreparedBytes::measured(1 << 20),
            },
        )
        .expect("input");
        let generation = input.begin().expect("generation");
        Self {
            generation,
            serial: 0,
        }
    }

    fn packet(&mut self) -> MixedSourcePacketId {
        self.serial += 1;
        MixedSourcePacketId::new(self.generation, self.serial).expect("nonzero packet serial")
    }

    fn handoff(&mut self, frames: u64, bytes: [u8; 3]) -> MixedSourceHandoff {
        MixedSourceHandoff {
            packet: self.packet(),
            at: SampleTime::new(frames),
            input: Midi1Input::from_bytes(bytes).expect("message"),
        }
    }

    fn frontier(&mut self, frames: u64) -> (MixedSourcePacketId, SampleTime) {
        (self.packet(), SampleTime::new(frames))
    }
}

struct Rig {
    armed: Armed,
    first: Source,
    second: Source,
}

impl Rig {
    fn new() -> Self {
        let profile = history_tests::mixed_profile();
        let depth = MixedJoinedPrepared::transfer_capacity(&profile).get();
        let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
        let first = Source::new("first");
        let second = Source::new("second");
        let armed = prepared
            .arm_bridge_rehearsal(
                candidate,
                &profile,
                MixedTransferSources::new(first.generation, second.generation).expect("sources"),
                MixedTransferFence {
                    stage_cells: MixedTransferPlaces::new(8),
                    held_onsets: MixedTransferPlaces::new(64),
                    peer_wait: FrameCount::new(1 << 20),
                },
                Ring(HeapRb::new(depth + 1)),
                Ring(HeapRb::new(depth + 1)),
            )
            .expect("bridge arms");
        Self {
            armed,
            first,
            second,
        }
    }

    fn stage(&mut self, handoff: MixedSourceHandoff) -> Result<MixedStaged, MixedBridgeError> {
        self.armed.control.stage(handoff)
    }

    /// The second source promises nothing earlier than `frames`.
    fn release_peer(&mut self, frames: u64) {
        let (packet, at) = self.second.frontier(frames);
        self.armed
            .control
            .advance_frontier(packet, at)
            .expect("frontier");
    }

    fn pump(&mut self) -> Result<MixedPumpStep, MixedBridgeError> {
        self.armed.control.pump(&mut self.armed.control_port)
    }

    fn set_boundary(&self, frames: u64) {
        self.armed
            .control
            .state
            .boundary
            .store(frames, Ordering::Release);
    }

    fn render(&mut self, frames: usize) -> Result<(), MixedTransferRenderError> {
        let mut samples = vec![0.5_f32; frames];
        let output = AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("block");
        self.armed.audio.render(&mut self.armed.audio_port, output)
    }

    fn collect(&mut self) -> Vec<MixedBridgeReport> {
        let mut reports = Vec::new();
        while let Some(report) = self
            .armed
            .control
            .collect(&mut self.armed.control_port)
            .expect("collect")
        {
            reports.push(report);
        }
        reports
    }

    fn join(self) -> MixedBridgeTeardown {
        let MixedBridgeArmed {
            control,
            audio,
            control_port,
            audio_port,
        } = self.armed;
        control
            .join(audio, Some(control_port), Some(audio_port))
            .expect("bound wrappers join")
    }
}

const ON: [u8; 3] = [0x90, 60, 100];
const OFF: [u8; 3] = [0x80, 60, 0];

/// One callback of the 512-frame harness profile, in whole quanta.
const WINDOW: u64 = 512;

#[test]
fn the_window_is_one_largest_callback_in_whole_quanta_inside_the_horizon() {
    let rig = Rig::new();
    let profile = history_tests::mixed_profile();
    assert_eq!(rig.armed.control.state.window, FrameCount::new(WINDOW));
    assert!(
        profile.limits().events().forward_event_horizon() > FrameCount::new(WINDOW),
        "the window gate is stricter than the horizon audio enforces"
    );
}

#[test]
fn a_head_at_the_window_end_waits_and_one_frame_earlier_forwards_at_its_own_stamp() {
    let mut rig = Rig::new();
    let stamp = 4 * WINDOW;
    let on = rig.first.handoff(stamp, ON);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    rig.release_peer(stamp + 1);

    // The window is `[boundary, boundary + window)`: a head at its end waits in its cell.
    rig.set_boundary(stamp - WINDOW);
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::Window {
            stamp: SampleTime::new(stamp),
            end: SampleTime::new(stamp),
        }))
    );
    assert_eq!(
        rig.armed.control.lane_charge(rig.first.generation),
        Some((MixedTransferPlaces::new(1), MixedTransferPlaces::new(1))),
        "the head and its protected key-up place stay staged"
    );
    assert_eq!(
        rig.armed.control.transfer.inner.ingress_outstanding,
        MixedIngressQueueCount::NONE,
        "nothing waits in the command ring or the ingress queue"
    );
    assert_eq!(rig.armed.control.fault(), None, "waiting is not a fault");

    // One frame later the head is inside and keeps its nominal stamp.
    rig.set_boundary(stamp - WINDOW + 1);
    assert!(matches!(
        rig.pump(),
        Ok(MixedPumpStep::Forwarded { stamp: forwarded, .. }) if forwarded == SampleTime::new(stamp)
    ));
}

#[test]
fn an_unrepresentable_window_is_terminal_clock_exhaustion_and_keeps_the_head() {
    let mut rig = Rig::new();
    let on = rig.first.handoff(100, ON);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    rig.release_peer(200);
    let boundary = u64::MAX - WINDOW + 1;
    rig.set_boundary(boundary);
    let fault = MixedBridgeFault::ClockExhausted {
        boundary: SampleTime::new(boundary),
    };
    assert_eq!(rig.pump(), Err(MixedBridgeError::Fault(fault)));
    assert_eq!(rig.pump(), Err(MixedBridgeError::Halted(fault)));
    assert_eq!(rig.armed.control.fault(), Some(fault));
    assert_eq!(
        rig.render(64),
        Err(MixedTransferRenderError::RemoteHalt),
        "the halt silences audio"
    );
    let teardown = rig.join();
    assert_eq!(teardown.fault(), Some(fault));
    assert_eq!(teardown.held(), [(on.packet, MixedSourceCustody::Staged)]);
}

#[test]
fn spent_command_ids_and_cell_generations_are_terminal_and_keep_the_head() {
    // The command-ID space.
    let mut rig = Rig::new();
    let on = rig.first.handoff(100, ON);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    rig.release_peer(200);
    rig.armed.control.transfer.inner.last_ingress_id = Some(MixedIngressCommandId(u64::MAX));
    let fault = MixedBridgeFault::Transfer(MixedTransferFault::CommandIdExhausted);
    assert_eq!(rig.pump(), Err(MixedBridgeError::Fault(fault)));
    let teardown = rig.join();
    assert_eq!(teardown.fault(), Some(fault));
    assert_eq!(teardown.held(), [(on.packet, MixedSourceCustody::Staged)]);

    // Every occurrence cell's generations.
    let mut rig = Rig::new();
    let on = rig.first.handoff(100, ON);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    rig.release_peer(200);
    rig.armed
        .control
        .transfer
        .inner
        .occurrence_ledger
        .exhaust_cells_after(0);
    let fault = MixedBridgeFault::Transfer(MixedTransferFault::OccurrenceCellsExhausted);
    assert_eq!(rig.pump(), Err(MixedBridgeError::Fault(fault)));
    let teardown = rig.join();
    assert_eq!(teardown.fault(), Some(fault));
    assert_eq!(teardown.held(), [(on.packet, MixedSourceCustody::Staged)]);
}

#[test]
fn a_full_journal_makes_the_head_wait_until_collection_frees_an_entry() {
    let mut rig = Rig::new();
    rig.armed.control.state.forwarded.limit = 1;
    let first = rig.first.handoff(100, ON);
    let second = rig.first.handoff(110, [0x90, 61, 100]);
    assert_eq!(rig.stage(first), Ok(MixedStaged::Staged));
    assert_eq!(rig.stage(second), Ok(MixedStaged::Staged));
    rig.release_peer(200);
    assert!(matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { .. })));
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::Commands)),
        "an outstanding command can still free the journal"
    );
    assert_eq!(rig.armed.control.state.forwarded.len(), 1);
    assert_eq!(
        rig.armed.control.lane_charge(rig.first.generation),
        Some((MixedTransferPlaces::new(1), MixedTransferPlaces::new(2))),
        "the waiting head kept its cell"
    );
    rig.render(128).expect("serving callback");
    assert_eq!(rig.collect().len(), 1);
    assert!(
        matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { packet, .. }) if packet == second.packet)
    );
    assert_eq!(rig.armed.control.fault(), None);
}

#[test]
fn a_full_handle_table_with_nothing_outstanding_splits_the_onset_and_consumes_its_key_up() {
    let mut rig = Rig::new();
    rig.armed.control.state.handles.limit = 1;
    let held = rig.first.handoff(100, ON);
    assert_eq!(rig.stage(held), Ok(MixedStaged::Staged));
    rig.release_peer(1_000);
    assert!(matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { .. })));
    rig.render(128).expect("serving callback");
    assert_eq!(rig.collect().len(), 1, "the held note's handle stays open");

    let refused = rig.first.handoff(110, [0x90, 61, 100]);
    assert_eq!(rig.stage(refused), Ok(MixedStaged::Staged));
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Split {
            packet: refused.packet,
            split: MixedSplit::CommandsFull,
        })
    );
    assert_eq!(
        rig.armed.control.state.handles.len(),
        1,
        "never past its bound"
    );
    assert_eq!(
        rig.armed.control.tombstone_count(rig.first.generation),
        Some(MixedTransferPlaces::new(1))
    );
    let key_up = rig.first.handoff(120, [0x80, 61, 0]);
    assert_eq!(
        rig.stage(key_up),
        Ok(MixedStaged::Split(MixedSplit::AfterRefusedOnset {
            onset: refused.packet,
            reason: MixedOnsetRefusal::CommandsFull,
        }))
    );
    // The held note's own key-up still forwards into its reserved place.
    let held_up = rig.first.handoff(130, OFF);
    assert_eq!(rig.stage(held_up), Ok(MixedStaged::Staged));
    assert!(
        matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { packet, .. }) if packet == held_up.packet)
    );
    rig.render(128).expect("serving callback");
    assert_eq!(rig.collect().len(), 1);
    let teardown = rig.join();
    assert_eq!(teardown.fault(), None);
    assert!(teardown.tombstones().is_empty() && teardown.open().is_empty());
}

#[test]
fn a_split_onset_with_a_staged_key_up_leaves_no_tombstone() {
    let mut rig = Rig::new();
    rig.armed.control.state.handles.limit = 1;
    rig.armed.control.state.lanes[0].held.limit = 2;
    let held = rig.first.handoff(100, ON);
    assert_eq!(rig.stage(held), Ok(MixedStaged::Staged));
    rig.release_peer(1_000);
    assert!(matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { .. })));
    rig.render(128).expect("serving callback");
    assert_eq!(rig.collect().len(), 1);

    let refused = rig.first.handoff(140, [0x90, 61, 100]);
    let refused_up = rig.first.handoff(150, [0x80, 61, 0]);
    assert_eq!(rig.stage(refused), Ok(MixedStaged::Staged));
    assert_eq!(rig.stage(refused_up), Ok(MixedStaged::Staged));
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Split {
            packet: refused.packet,
            split: MixedSplit::CommandsFull,
        })
    );
    assert_eq!(
        rig.armed.control.tombstone_count(rig.first.generation),
        Some(MixedTransferPlaces::NONE),
        "the staged key-up already left the pairing table"
    );
    let next = rig.first.handoff(160, [0x90, 62, 100]);
    assert_eq!(rig.stage(next), Ok(MixedStaged::Staged));
    assert_eq!(rig.armed.control.fault(), None);
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Split {
            packet: refused_up.packet,
            split: MixedSplit::AfterRefusedOnset {
                onset: refused.packet,
                reason: MixedOnsetRefusal::CommandsFull,
            },
        })
    );
    assert_eq!(rig.armed.control.fault(), None);
}

#[test]
fn a_source_beyond_the_declared_holds_is_terminal_before_any_table_grows() {
    let mut rig = Rig::new();
    // The bound counts every onset whose key-up has not arrived, whether staged,
    // forwarded or refused, so two staged onsets reach a declared bound of two.
    rig.armed.control.state.lanes[0].held.limit = 2;
    rig.release_peer(10_000);
    let first = rig.first.handoff(100, ON);
    let second = rig.first.handoff(110, [0x90, 61, 100]);
    assert_eq!(rig.stage(first), Ok(MixedStaged::Staged));
    assert_eq!(rig.stage(second), Ok(MixedStaged::Staged));
    let third = rig.first.handoff(120, [0x90, 62, 100]);
    let fault = MixedBridgeFault::SourceHolds {
        connection: rig.first.generation,
        onset: third.packet,
    };
    assert_eq!(rig.stage(third), Err(MixedBridgeError::Fault(fault)));
    assert_eq!(
        rig.armed.control.lane_charge(rig.first.generation),
        Some((MixedTransferPlaces::new(2), MixedTransferPlaces::new(2))),
        "the refused onset took no cell"
    );
    assert_eq!(
        rig.armed.control.held_onsets(rig.first.generation),
        Some(MixedTransferPlaces::new(2))
    );
    assert_eq!(
        rig.armed.control.tombstone_count(rig.first.generation),
        Some(MixedTransferPlaces::NONE)
    );
    let teardown = rig.join();
    assert_eq!(teardown.fault(), Some(fault));
    assert_eq!(teardown.held().len(), 2);
}

#[test]
fn same_key_onsets_pair_their_key_ups_first_in_first_out_on_their_own_channel() {
    let mut rig = Rig::new();
    let first_on = rig.first.handoff(100, ON);
    let second_on = rig.first.handoff(110, ON);
    let first_up = rig.first.handoff(120, OFF);
    let second_up = rig.first.handoff(130, OFF);
    for handoff in [first_on, second_on, first_up, second_up] {
        assert_eq!(rig.stage(handoff), Ok(MixedStaged::Staged));
    }
    // Key 60 on another channel, and a third key-up of key 60, find no onset.
    let other_channel = rig.first.handoff(140, [0x81, 60, 0]);
    let stray = rig.first.handoff(150, OFF);
    for unpaired in [other_channel, stray] {
        assert_eq!(
            rig.stage(unpaired),
            Ok(MixedStaged::Split(MixedSplit::Unpaired))
        );
    }
    assert_eq!(
        rig.armed.control.held_onsets(rig.first.generation),
        Some(MixedTransferPlaces::NONE)
    );
    rig.release_peer(1_000);
    let mut forwarded = Vec::new();
    while let Ok(MixedPumpStep::Forwarded { packet, .. }) = rig.pump() {
        forwarded.push(packet);
    }
    assert_eq!(forwarded.len(), 4);
    assert_eq!(forwarded[..2], [first_on.packet, second_on.packet]);
    assert_eq!(forwarded[2..], [first_up.packet, second_up.packet]);
    rig.render(512).expect("serving callback");
    let pairs: Vec<_> = rig
        .collect()
        .iter()
        .filter_map(|report| match report.outcome() {
            MixedBridgeOutcome::KeyUp { onset, .. } => {
                Some((report.packet().expect("a source report"), onset))
            }
            _ => None,
        })
        .collect();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0], (first_up.packet, first_on.packet));
    assert_eq!(pairs[1], (second_up.packet, second_on.packet));
    let teardown = rig.join();
    assert_eq!(teardown.fault(), None);
    assert!(teardown.open().is_empty(), "both same-key notes retired");
}

#[test]
fn a_repeated_source_serial_is_terminal_and_stages_nothing() {
    let mut rig = Rig::new();
    let on = rig.first.handoff(100, ON);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    let repeated = MixedSourceHandoff {
        at: SampleTime::new(110),
        input: Midi1Input::from_bytes([0x90, 61, 100]).expect("message"),
        ..on
    };
    let fault = MixedBridgeFault::SourceSerial {
        packet: on.packet,
        previous: Some(on.packet),
    };
    assert_eq!(rig.stage(repeated), Err(MixedBridgeError::Fault(fault)));
    assert_eq!(
        rig.armed.control.lane_charge(rig.first.generation),
        Some((MixedTransferPlaces::new(1), MixedTransferPlaces::new(1)))
    );
    let teardown = rig.join();
    assert_eq!(teardown.fault(), Some(fault));
    assert_eq!(teardown.held(), [(on.packet, MixedSourceCustody::Staged)]);
}

#[test]
fn a_missing_first_or_middle_source_packet_is_terminal() {
    let mut missing_first = Rig::new();
    let first_packet =
        MixedSourcePacketId::new(missing_first.first.generation, 2).expect("nonzero packet serial");
    let handoff = MixedSourceHandoff {
        packet: first_packet,
        at: SampleTime::new(100),
        input: Midi1Input::from_bytes(ON).expect("message"),
    };
    let fault = MixedBridgeFault::SourceSerial {
        packet: first_packet,
        previous: None,
    };
    assert_eq!(
        missing_first.stage(handoff),
        Err(MixedBridgeError::Fault(fault))
    );
    assert_eq!(missing_first.join().fault(), Some(fault));

    let mut missing_middle = Rig::new();
    let first = missing_middle.first.handoff(100, ON);
    assert_eq!(missing_middle.stage(first), Ok(MixedStaged::Staged));
    let skipped = MixedSourceHandoff {
        packet: MixedSourcePacketId::new(missing_middle.first.generation, 3)
            .expect("nonzero packet serial"),
        at: SampleTime::new(110),
        input: Midi1Input::from_bytes([0x90, 61, 100]).expect("message"),
    };
    let fault = MixedBridgeFault::SourceSerial {
        packet: skipped.packet,
        previous: Some(first.packet),
    };
    assert_eq!(
        missing_middle.stage(skipped),
        Err(MixedBridgeError::Fault(fault))
    );
    assert_eq!(missing_middle.join().fault(), Some(fault));
}

#[test]
fn source_packet_identity_refuses_zero_serial() {
    let source = Source::new("zero-serial");
    assert_eq!(
        MixedSourcePacketId::new(source.generation, 0),
        Err(MixedSourcePacketIdError::ZeroSerial)
    );
}

type ControlPort = MixedTransferControlPort<Prod<MixedTransferCommand>, Cons<MixedTransferResult>>;
type Owner = MixedBridgeResetOwner<Cons<MixedTransferCommand>, Prod<MixedTransferResult>>;
type Wrapper = MixedBridgeResetAudio<Cons<MixedTransferCommand>, Prod<MixedTransferResult>>;

/// The reset boundary, after the old program's Stop at 192.
const B_FRAMES: u64 = 448;
const B: SampleTime = SampleTime::new(B_FRAMES);
const STOP: SampleTime = SampleTime::new(192);
const Q: u64 = QUANTUM_FRAMES as u64;

fn fence() -> MixedTransferFence {
    MixedTransferFence {
        stage_cells: MixedTransferPlaces::new(8),
        held_onsets: MixedTransferPlaces::new(64),
        peer_wait: FrameCount::new(1 << 20),
    }
}

/// A bridge over a fresh arm whose armed transition takes effect at `requested`.
fn arm_at(
    requested: SampleTime,
    sources: MixedTransferSources,
    fence: MixedTransferFence,
) -> Armed {
    let profile = history_tests::mixed_profile();
    let depth = MixedJoinedPrepared::transfer_capacity(&profile).get();
    let prepared = history_tests::bound_with_events(true, |note| {
        vec![PlanEvent::new(
            PlanPosition::new(10),
            CompiledPayload::NoteOn {
                slot: note,
                key: history_tests::key(60),
                velocity: NoteVelocity::FULL,
            },
        )]
    });
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    prepared
        .arm_bridge_rehearsal(
            candidate,
            &profile,
            sources,
            fence,
            Ring(HeapRb::new(depth + 1)),
            Ring(HeapRb::new(depth + 1)),
        )
        .expect("bridge arms")
}

/// A fresh successor bridge for `B`: its control half, control port and boxed audio half.
fn successor(sources: MixedTransferSources) -> (MixedBridgeControl, ControlPort, Owner) {
    let MixedBridgeArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = arm_at(B, sources, fence());
    let owner = Owner::new(audio, audio_port).expect("the arm's own port");
    (control, control_port, owner)
}

/// The old bridge with its audio half in the reset wrapper, and an armed successor whose
/// parts wait to be sealed and offered.
struct ResetRig {
    control: MixedBridgeControl,
    /// The live control port: the old arm's until commit, then the successor's.
    port: ControlPort,
    wrapper: Wrapper,
    first: Source,
    second: Source,
    sources: MixedTransferSources,
    next_control: Option<MixedBridgeControl>,
    next_port: Option<ControlPort>,
    next_owner: Option<Owner>,
}

impl ResetRig {
    fn new() -> Self {
        let first = Source::new("first");
        let second = Source::new("second");
        let sources =
            MixedTransferSources::new(first.generation, second.generation).expect("sources");
        let MixedBridgeArmed {
            control,
            audio,
            control_port,
            audio_port,
        } = arm_at(SampleTime::new(64), sources, fence());
        let mut wrapper = Wrapper::new(Owner::new(audio, audio_port).expect("the arm's own port"));
        assert_eq!(
            wrapper.schedule_stop(STOP),
            Ok(MixedBridgeStopAccepted::Scheduled)
        );
        let (next_control, next_port, next_owner) = successor(sources);
        Self {
            control,
            port: control_port,
            wrapper,
            first,
            second,
            sources,
            next_control: Some(next_control),
            next_port: Some(next_port),
            next_owner: Some(next_owner),
        }
    }

    fn stage(&mut self, handoff: MixedSourceHandoff) -> Result<MixedStaged, MixedBridgeError> {
        self.control.stage(handoff)
    }

    /// The second source promises nothing earlier than `frames`.
    fn release_peer(&mut self, frames: u64) {
        let (packet, at) = self.second.frontier(frames);
        self.control.advance_frontier(packet, at).expect("frontier");
    }

    fn pump(&mut self) -> Result<MixedPumpStep, MixedBridgeError> {
        self.control.pump(&mut self.port)
    }

    fn collect(&mut self) -> Vec<MixedBridgeReport> {
        let mut reports = Vec::new();
        while let Some(report) = self.control.collect(&mut self.port).expect("collect") {
            reports.push(report);
        }
        reports
    }

    fn render(&mut self, frames: u64) -> Result<(), MixedBridgeResetRenderError> {
        let frames = usize::try_from(frames).expect("block fits");
        let mut samples = vec![0.0_f32; frames];
        let output = AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("block");
        self.wrapper.render(output)
    }

    /// Render whole quanta until the live bridge's audio boundary reaches `at`.
    fn render_to(&mut self, at: u64) {
        while self.control.audio_boundary() < SampleTime::new(at) {
            self.render(Q).expect("callback");
        }
        assert_eq!(self.control.audio_boundary(), SampleTime::new(at));
    }

    fn seal(&mut self) -> Result<(), Box<MixedBridgeSealRefused>> {
        let next = self.next_control.take().expect("an unsealed successor");
        self.control.seal_reset(next, B)
    }

    fn offer(&mut self) -> Result<(), MixedBridgeResetRefusal> {
        let owner = self.next_owner.take().expect("an unoffered successor");
        self.wrapper
            .offer_reset(owner, B)
            .map_err(|(reason, owner)| {
                self.next_owner = Some(owner);
                reason
            })
    }

    /// Render whole quanta up to `B`, then open it with an eight-frame callback, so an
    /// applied reset is still fading.
    fn open_b(&mut self) {
        while self.control.audio_boundary() < B {
            self.render(Q).expect("callback");
        }
        self.render(8).expect("the callback opening B");
        assert!(!matches!(
            self.wrapper.state(),
            MixedBridgeResetState::Pending { .. }
        ));
    }

    fn receipt(&self) -> MixedBridgeResetReceipt {
        match self.wrapper.state() {
            MixedBridgeResetState::Fading { receipt, .. }
            | MixedBridgeResetState::Retired(receipt) => receipt,
            other => panic!("no applied reset: {other:?}"),
        }
    }

    /// Commit the applied reset; the live port becomes the successor's.
    fn commit(&mut self) -> (MixedBridgeRetiredControl, ControlPort) {
        let retired = self
            .control
            .commit_reset(self.receipt())
            .expect("the applied reset commits");
        let next_port = self.next_port.take().expect("the successor's port");
        (retired, core::mem::replace(&mut self.port, next_port))
    }

    /// Finish the fade, take the old owner back and join both arms.
    fn join_committed(
        mut self,
        retired: MixedBridgeRetiredControl,
        old_port: ControlPort,
    ) -> (MixedBridgeTeardown, MixedBridgeRetiredTeardown) {
        while !matches!(self.wrapper.state(), MixedBridgeResetState::Retired(_)) {
            self.render(Q).expect("fade");
        }
        let old = self.wrapper.take_retired().expect("the retired old owner");
        let (old_audio, old_audio_port) = old.into_owner().into_parts();
        let old = retired
            .join(old_audio, Some(old_port), Some(old_audio_port))
            .expect("the old arm joins with its own ports");
        let (active, secondary) = self.wrapper.into_owners();
        assert!(secondary.is_none());
        let (audio, audio_port) = active.into_parts();
        let live = self
            .control
            .join(audio, Some(self.port), Some(audio_port))
            .expect("the successor joins with its own ports");
        (live, old)
    }
}

/// A forwarded, served and collected old onset of key 60 on the first source at 100,
/// with the second source's frontier released past every stamp the test uses.
fn sounding_old_note(rig: &mut ResetRig) -> MixedSourceHandoff {
    let on = rig.first.handoff(100, ON);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    rig.release_peer(1 << 16);
    assert!(
        matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { packet, .. }) if packet == on.packet)
    );
    rig.render_to(B_FRAMES - Q);
    let reports = rig.collect();
    assert_eq!(reports.len(), 1);
    assert!(matches!(
        reports[0].outcome(),
        MixedBridgeOutcome::NoteOn(Ok(_))
    ));
    on
}

/// Seal, offer and open `B`, then commit.
fn reset_at_b(rig: &mut ResetRig) -> (MixedBridgeRetiredControl, ControlPort) {
    if rig.next_control.is_some() {
        rig.seal().expect("seal");
    }
    rig.offer().expect("the sealed successor is accepted");
    rig.open_b();
    let state = rig.wrapper.state();
    assert!(
        matches!(state, MixedBridgeResetState::Fading { .. }),
        "{state:?}"
    );
    rig.commit()
}

/// Pump until the live bridge forwards nothing more, returning every step.
fn pump_all(rig: &mut ResetRig) -> Vec<MixedPumpStep> {
    let mut steps = Vec::new();
    loop {
        match rig.pump().expect("pump") {
            MixedPumpStep::Idle(_) => return steps,
            step => steps.push(step),
        }
    }
}

#[test]
fn an_old_onset_key_up_staged_while_sealed_is_consumed_as_reset_and_never_reaches_the_successor() {
    let mut rig = ResetRig::new();
    let on = sounding_old_note(&mut rig);
    rig.seal().expect("seal");
    assert_eq!(rig.control.reset_ready(), Some(B));

    // The key-up arrives before `B`, but the old transfer takes nothing while sealed.
    let off = rig.first.handoff(B_FRAMES - 20, OFF);
    assert_eq!(rig.stage(off), Ok(MixedStaged::Staged));
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: B }))
    );
    assert_eq!(rig.control.last_stamp(), Some(SampleTime::new(100)));

    let (retired, old_port) = reset_at_b(&mut rig);
    let receipt = retired.receipt();
    assert_eq!(receipt.at(), B);
    assert_eq!(receipt.ended_at(), SampleTime::new(B_FRAMES + Q));
    let ended: Vec<_> = receipt.ended().collect();
    assert!(matches!(ended[..], [MixedBridgeResetEnd::Tombstoned(_)]));

    // At commit the staged key-up carries the reset; it leaves its lane as a split.
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Split {
            packet: off.packet,
            split: MixedSplit::AfterRefusedOnset {
                onset: on.packet,
                reason: MixedOnsetRefusal::Reset,
            },
        })
    );
    assert_eq!(rig.pump(), Ok(MixedPumpStep::Idle(MixedBridgeWait::Empty)));
    assert_eq!(
        rig.control.lane_charge(rig.first.generation),
        Some((MixedTransferPlaces::NONE, MixedTransferPlaces::NONE))
    );

    let (live, old) = rig.join_committed(retired, old_port);
    assert_eq!(
        old.handles(),
        [(on.packet, MixedRetiredHandle::Ended(ended[0]))],
        "the old handle is ended by the reset group, not by a key-up"
    );
    assert!(old.unclassified().is_empty() && old.unmatched().is_empty());
    assert!(old.bridge().held().is_empty());
    assert_eq!(live.fault(), None);
    assert!(live.held().is_empty() && live.open().is_empty());
    assert!(
        live.transfer().outstanding().is_empty() && live.transfer().handles().is_empty(),
        "the successor never received a release"
    );
}

#[test]
fn an_onset_staged_while_sealed_sounds_in_the_successor_and_its_key_up_releases_it() {
    let mut rig = ResetRig::new();
    rig.release_peer(1 << 16);
    rig.render_to(B_FRAMES - Q);
    rig.seal().expect("seal");
    let on = rig.first.handoff(B_FRAMES - 30, ON);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: B }))
    );
    let (retired, old_port) = reset_at_b(&mut rig);
    assert_eq!(retired.receipt().ended().count(), 0);

    let boundary = rig.control.audio_boundary();
    assert!(boundary > B, "the live clock is now the successor's");
    let Ok(MixedPumpStep::Forwarded { packet, stamp, .. }) = rig.pump() else {
        panic!("the staged onset goes to the successor");
    };
    assert_eq!((packet, stamp), (on.packet, boundary));
    rig.render(Q).expect("successor callback");
    let reports = rig.collect();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].nominal_at(), SampleTime::new(B_FRAMES - 30));
    assert!(reports[0].executed_at() >= B);
    assert!(matches!(
        reports[0].outcome(),
        MixedBridgeOutcome::NoteOn(Ok(_))
    ));

    let off = rig
        .first
        .handoff(rig.control.audio_boundary().as_u64() + 10, OFF);
    assert_eq!(rig.stage(off), Ok(MixedStaged::Staged));
    assert!(
        matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { packet, .. }) if packet == off.packet)
    );
    rig.render(Q).expect("successor callback");
    let reports = rig.collect();
    assert!(matches!(
        reports[..],
        [MixedBridgeReport {
            outcome: MixedBridgeOutcome::KeyUp {
                result: Ok(MixedKeyUpRetired::Released(_)),
                onset,
            },
            ..
        }] if onset == on.packet
    ));
    let (live, old) = rig.join_committed(retired, old_port);
    assert!(live.open().is_empty() && live.tombstones().is_empty());
    assert!(old.handles().is_empty() && old.unclassified().is_empty());
}

#[test]
fn same_key_old_up_new_down_new_up_pair_each_key_up_with_its_own_onset_across_commit() {
    let mut rig = ResetRig::new();
    let old_on = sounding_old_note(&mut rig);
    rig.seal().expect("seal");
    let old_up = rig.first.handoff(B_FRAMES - 30, OFF);
    let new_on = rig.first.handoff(B_FRAMES - 20, ON);
    let new_up = rig.first.handoff(B_FRAMES - 10, OFF);
    for handoff in [old_up, new_on, new_up] {
        assert_eq!(rig.stage(handoff), Ok(MixedStaged::Staged));
    }
    let (retired, old_port) = reset_at_b(&mut rig);
    let steps = pump_all(&mut rig);
    assert_eq!(steps.len(), 3);
    assert_eq!(
        steps[0],
        MixedPumpStep::Split {
            packet: old_up.packet,
            split: MixedSplit::AfterRefusedOnset {
                onset: old_on.packet,
                reason: MixedOnsetRefusal::Reset,
            },
        }
    );
    assert!(matches!(steps[1], MixedPumpStep::Forwarded { packet, .. } if packet == new_on.packet));
    assert!(matches!(steps[2], MixedPumpStep::Forwarded { packet, .. } if packet == new_up.packet));
    rig.render(Q).expect("successor callback");
    let pairs: Vec<_> = rig
        .collect()
        .iter()
        .filter_map(|report| match report.outcome() {
            MixedBridgeOutcome::KeyUp { onset, result } => Some((
                report.packet().expect("a source report"),
                onset,
                result.is_ok(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(pairs, [(new_up.packet, new_on.packet, true)]);
    let (live, old) = rig.join_committed(retired, old_port);
    assert!(live.open().is_empty());
    assert_eq!(old.handles().len(), 1);
    assert_eq!(old.handles()[0].0, old_on.packet);
}

#[test]
fn a_sealed_command_unserved_by_the_old_owner_refuses_the_offer_until_audio_serves_it() {
    let mut rig = ResetRig::new();
    rig.release_peer(1 << 16);
    rig.render_to(B_FRAMES - Q);
    let late = rig.first.handoff(B_FRAMES - 10, ON);
    assert_eq!(rig.stage(late), Ok(MixedStaged::Staged));
    assert!(matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { .. })));
    rig.seal().expect("a stamp before B may be sealed");
    assert_eq!(
        rig.control.reset_ready(),
        None,
        "the command is outstanding"
    );
    assert_eq!(rig.offer(), Err(MixedBridgeResetRefusal::IngressPending));

    // The callback before `B` serves it; once collected the bridge reports ready.
    rig.render(Q).expect("callback before B");
    assert_eq!(rig.control.reset_ready(), None, "not yet collected");
    assert_eq!(rig.collect().len(), 1);
    assert_eq!(rig.control.reset_ready(), Some(B));
    let (retired, old_port) = reset_at_b(&mut rig);
    assert_eq!(
        retired.receipt().ended().count(),
        1,
        "the late onset sounded"
    );
    let (_, old) = rig.join_committed(retired, old_port);
    assert!(matches!(
        old.handles(),
        [(packet, MixedRetiredHandle::Ended(_))] if *packet == late.packet
    ));
}

#[test]
fn an_unsealed_or_foreign_sealed_successor_is_refused_at_offer_and_cannot_be_abandoned() {
    let mut rig = ResetRig::new();
    rig.render_to(B_FRAMES - Q);
    assert_eq!(rig.offer(), Err(MixedBridgeResetRefusal::Unsealed));

    // Sealed for another arm: the offered owner is not the one the seal names.
    let (other, other_port, other_owner) = successor(rig.sources);
    rig.control
        .seal_reset(other, B)
        .expect("seal for the other arm");
    assert_eq!(rig.offer(), Err(MixedBridgeResetRefusal::Unsealed));
    let foreign = rig.next_owner.as_ref().expect("refused owner");
    assert_eq!(
        rig.control.abandon_reset(foreign).map(drop),
        Err(MixedBridgeAbandonRefusal::ForeignOwner)
    );
    let other = rig
        .control
        .abandon_reset(&other_owner)
        .expect("the sealed arm comes back");
    assert_eq!(
        rig.control.abandon_reset(&other_owner).map(drop),
        Err(MixedBridgeAbandonRefusal::NotSealed)
    );
    assert_eq!(
        rig.offer(),
        Err(MixedBridgeResetRefusal::Unsealed),
        "abandon cleared the seal"
    );
    let (audio, audio_port) = other_owner.into_parts();
    let teardown = other
        .join(audio, Some(other_port), Some(audio_port))
        .expect("the abandoned arm joins");
    assert_eq!(
        teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

#[test]
fn a_withdrawn_successor_is_abandoned_and_waiting_heads_forward_to_the_old_owner_in_order() {
    let mut rig = ResetRig::new();
    rig.release_peer(1 << 16);
    rig.render_to(B_FRAMES - 2 * Q);
    rig.seal().expect("seal");
    let first_on = rig.first.handoff(B_FRAMES - 100, ON);
    let second_on = rig.first.handoff(B_FRAMES - 90, [0x90, 62, 100]);
    for handoff in [first_on, second_on] {
        assert_eq!(rig.stage(handoff), Ok(MixedStaged::Staged));
    }
    rig.offer().expect("offer");
    let returned = rig.wrapper.withdraw_pending().expect("pending successor");
    let next = rig
        .control
        .abandon_reset(&returned)
        .expect("a withdrawn successor is fresh");
    assert_eq!(rig.control.reset_ready(), None);
    let steps = pump_all(&mut rig);
    let forwarded: Vec<_> = steps
        .iter()
        .map(|step| match step {
            MixedPumpStep::Forwarded { packet, .. } => *packet,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(forwarded, [first_on.packet, second_on.packet]);
    rig.render(Q).expect("old callback");
    assert_eq!(rig.collect().len(), 2, "the old owner served both");

    let (audio, audio_port) = returned.into_parts();
    let next_port = rig.next_port.take().expect("successor port");
    let teardown = next
        .join(audio, Some(next_port), Some(audio_port))
        .expect("the abandoned successor joins");
    assert_eq!(
        teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert!(teardown.held().is_empty() && teardown.transfer().unserved().is_empty());
}

#[test]
fn a_foreign_receipt_or_an_installed_successor_changes_nothing() {
    // Another reset's receipt.
    let mut other = ResetRig::new();
    other.render_to(B_FRAMES - Q);
    other.seal().expect("seal");
    other.offer().expect("offer");
    other.open_b();
    let foreign = other.receipt();

    let mut rig = ResetRig::new();
    let on = sounding_old_note(&mut rig);
    assert_eq!(
        rig.control.commit_reset(foreign).map(drop),
        Err(MixedBridgeCommitRefusal::NotSealed)
    );
    rig.seal().expect("seal");
    let off = rig.first.handoff(B_FRAMES - 20, OFF);
    assert_eq!(rig.stage(off), Ok(MixedStaged::Staged));
    let before = (
        rig.control.lane_charge(rig.first.generation),
        rig.control.held_onsets(rig.first.generation),
        rig.control.tombstone_count(rig.first.generation),
    );
    assert_eq!(
        rig.control.commit_reset(foreign).map(drop),
        Err(MixedBridgeCommitRefusal::ForeignReceipt)
    );
    let after = (
        rig.control.lane_charge(rig.first.generation),
        rig.control.held_onsets(rig.first.generation),
        rig.control.tombstone_count(rig.first.generation),
    );
    assert_eq!(before, after);
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: B }))
    );

    // Applied at `B`: the installed successor cannot be abandoned, only committed.
    rig.offer().expect("offer");
    rig.open_b();
    let receipt = rig.receipt();
    let ResetRig {
        mut control,
        mut port,
        wrapper,
        next_port,
        ..
    } = rig;
    let (active, secondary) = wrapper.into_owners();
    assert_eq!(
        control.abandon_reset(&active).map(drop),
        Err(MixedBridgeAbandonRefusal::Installed)
    );
    assert_eq!(
        control.pump(&mut port),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: B })),
        "a refused abandon left the bridge sealed"
    );
    let retired = control.commit_reset(receipt).expect("commit at teardown");
    assert_eq!(
        control.tombstone_count(on.packet.connection()),
        Some(MixedTransferPlaces::NONE),
        "the staged key-up, not the pairing table, carries the reset"
    );
    let (audio, audio_port) = active.into_parts();
    let live = control
        .join(audio, next_port, Some(audio_port))
        .expect("successor joins");
    assert_eq!(live.held(), [(off.packet, MixedSourceCustody::Staged)]);
    let (old_audio, old_audio_port) = secondary.expect("fading old owner").into_parts();
    let old = retired
        .join(old_audio, Some(port), Some(old_audio_port))
        .expect("old arm joins");
    assert!(
        matches!(old.handles(), [(packet, MixedRetiredHandle::Ended(_))] if *packet == on.packet)
    );
}

#[test]
fn a_fault_of_the_fading_old_owner_halts_the_live_bridge() {
    let mut rig = ResetRig::new();
    let _on = sounding_old_note(&mut rig);
    let (mut retired, old_port) = reset_at_b(&mut rig);
    // A foreign port is a terminal fault of the old transfer while its owner fades.
    let fault = MixedBridgeFault::Transfer(MixedTransferFault::ForeignEndpoint);
    assert_eq!(
        retired.collect(&mut rig.port),
        Err(MixedBridgeError::Fault(fault))
    );
    assert!(matches!(
        rig.render(Q),
        Err(
            MixedBridgeResetRenderError::Active(MixedTransferRenderError::RemoteHalt)
                | MixedBridgeResetRenderError::Fading(MixedTransferRenderError::RemoteHalt)
        )
    ));
    assert_eq!(
        rig.pump(),
        Err(MixedBridgeError::Halted(MixedBridgeFault::Transfer(
            MixedTransferFault::Remote
        ))),
        "the wrapper's fault reaches the successor's control (rule 1.8)"
    );
    assert_eq!(retired.fault(), Some(fault));
    // Teardown stays reachable.
    let ResetRig {
        control,
        port,
        wrapper,
        ..
    } = rig;
    let (active, secondary) = wrapper.into_owners();
    let (audio, audio_port) = active.into_parts();
    let live = control
        .join(audio, Some(port), Some(audio_port))
        .expect("successor joins");
    assert!(live.fault().is_some());
    let (old_audio, old_audio_port) = secondary.expect("fading old owner").into_parts();
    let old = retired
        .join(old_audio, Some(old_port), Some(old_audio_port))
        .expect("old arm joins");
    assert_eq!(old.bridge().fault(), Some(fault));
    assert!(old.unclassified().is_empty());
}

#[test]
fn a_fault_after_the_old_owner_retires_halts_the_live_bridge() {
    let mut rig = ResetRig::new();
    let _on = sounding_old_note(&mut rig);
    let (mut retired, _old_port) = reset_at_b(&mut rig);
    while !matches!(rig.wrapper.state(), MixedBridgeResetState::Retired(_)) {
        rig.render(Q).expect("the old owner retires");
    }

    let fault = MixedBridgeFault::Transfer(MixedTransferFault::ForeignEndpoint);
    assert_eq!(
        retired.collect(&mut rig.port),
        Err(MixedBridgeError::Fault(fault))
    );
    assert_eq!(
        rig.pump(),
        Err(MixedBridgeError::Halted(MixedBridgeFault::Transfer(
            MixedTransferFault::Remote
        )))
    );
    assert!(matches!(
        rig.render(Q),
        Err(MixedBridgeResetRenderError::Active(
            MixedTransferRenderError::RemoteHalt
        ))
    ));
}

#[test]
fn a_fault_after_retirement_before_commit_halts_the_sealed_successor() {
    let mut rig = ResetRig::new();
    rig.seal().expect("seal");
    rig.render_to(B.as_u64());
    rig.offer().expect("offer");
    rig.open_b();
    while !matches!(rig.wrapper.state(), MixedBridgeResetState::Retired(_)) {
        rig.render(Q).expect("the old owner retires");
    }
    let receipt = rig.receipt();

    let foreign = rig.next_port.as_mut().expect("successor port");
    let fault = MixedBridgeFault::Transfer(MixedTransferFault::ForeignEndpoint);
    assert_eq!(
        rig.control.collect(foreign),
        Err(MixedBridgeError::Fault(fault))
    );
    assert_eq!(rig.control.fault(), Some(fault));
    let BridgeReset::Sealed { successor, .. } = &rig.control.state.reset else {
        panic!("the reset remains sealed before commit");
    };
    assert_eq!(
        successor.fault(),
        Some(MixedBridgeFault::Transfer(MixedTransferFault::Remote))
    );
    assert!(matches!(
        rig.render(Q),
        Err(MixedBridgeResetRenderError::Active(
            MixedTransferRenderError::RemoteHalt
        ))
    ));
    let retired = rig
        .control
        .commit_reset(receipt)
        .expect("fault does not prevent custody commit");
    let old_port = core::mem::replace(&mut rig.port, rig.next_port.take().expect("successor port"));
    let (live, old) = rig.join_committed(retired, old_port);
    assert_eq!(live.fault(), Some(fault));
    assert_eq!(old.bridge().fault(), Some(fault));
    assert!(old.unclassified().is_empty());
}

#[test]
fn a_foreign_pump_while_sealed_halts_the_successor_before_commit() {
    let mut rig = ResetRig::new();
    rig.seal().expect("seal");
    rig.render_to(B.as_u64());
    rig.offer().expect("offer");
    rig.open_b();
    while !matches!(rig.wrapper.state(), MixedBridgeResetState::Retired(_)) {
        rig.render(Q).expect("the old owner retires");
    }

    let foreign = rig.next_port.as_mut().expect("successor port");
    let fault = MixedBridgeFault::Transfer(MixedTransferFault::ForeignEndpoint);
    assert_eq!(
        rig.control.pump(foreign),
        Err(MixedBridgeError::Fault(fault))
    );
    assert_eq!(rig.control.fault(), Some(fault));
    let BridgeReset::Sealed { successor, .. } = &rig.control.state.reset else {
        panic!("the reset remains sealed before commit");
    };
    assert_eq!(
        successor.fault(),
        Some(MixedBridgeFault::Transfer(MixedTransferFault::Remote))
    );
    assert!(matches!(
        rig.render(Q),
        Err(MixedBridgeResetRenderError::Active(
            MixedTransferRenderError::RemoteHalt
        ))
    ));
    let retired = rig
        .control
        .commit_reset(rig.receipt())
        .expect("the faulted reset still commits");
    let old_port = core::mem::replace(&mut rig.port, rig.next_port.take().expect("successor port"));
    let (live, old) = rig.join_committed(retired, old_port);
    assert_eq!(live.fault(), Some(fault));
    assert_eq!(old.bridge().fault(), Some(fault));
    assert!(old.unclassified().is_empty());
}

#[test]
fn an_inconsistent_old_handle_at_commit_keeps_the_ledger_cause_on_both_arms() {
    let mut rig = ResetRig::new();
    let on = sounding_old_note(&mut rig);
    rig.seal().expect("seal");
    rig.offer().expect("offer");
    rig.open_b();
    while !matches!(rig.wrapper.state(), MixedBridgeResetState::Retired(_)) {
        rig.render(Q).expect("the old owner retires");
    }
    let receipt = rig.receipt();
    rig.control.state.handles.items.clear();

    let retired = rig
        .control
        .commit_reset(receipt)
        .expect("custody still moves");
    assert_eq!(rig.control.fault(), Some(MixedBridgeFault::Ledger));
    assert_eq!(retired.fault(), Some(MixedBridgeFault::Ledger));
    let old_port = core::mem::replace(&mut rig.port, rig.next_port.take().expect("successor port"));
    let (live, old) = rig.join_committed(retired, old_port);
    assert_eq!(live.fault(), Some(MixedBridgeFault::Ledger));
    assert_eq!(live.tombstones(), &[(on.packet, MixedOnsetRefusal::Reset)]);
    assert_eq!(old.bridge().fault(), Some(MixedBridgeFault::Ledger));
    assert_eq!(old.unclassified().len(), 1);
    assert!(old.unmatched().is_empty());
    assert!(old.handles().is_empty());
}

#[test]
fn a_successor_fault_before_an_inconsistent_commit_keeps_its_first_cause() {
    let mut rig = ResetRig::new();
    let on = sounding_old_note(&mut rig);
    rig.seal().expect("seal");
    rig.offer().expect("offer");
    rig.open_b();
    while !matches!(rig.wrapper.state(), MixedBridgeResetState::Retired(_)) {
        rig.render(Q).expect("the old owner retires");
    }
    let receipt = rig.receipt();
    let BridgeReset::Sealed { successor, .. } = &rig.control.state.reset else {
        panic!("reset remains sealed");
    };
    successor
        .transfer
        .inner
        .ingress_halt
        .store(true, Ordering::Release);
    rig.control.state.handles.items.clear();

    let retired = rig.control.commit_reset(receipt).expect("custody moves");
    assert_eq!(
        rig.control.fault(),
        Some(MixedBridgeFault::Transfer(MixedTransferFault::Remote))
    );
    assert_eq!(retired.fault(), Some(MixedBridgeFault::Ledger));
    let old_port = core::mem::replace(&mut rig.port, rig.next_port.take().expect("successor port"));
    let (live, old) = rig.join_committed(retired, old_port);
    assert_eq!(
        live.fault(),
        Some(MixedBridgeFault::Transfer(MixedTransferFault::Remote))
    );
    assert_eq!(live.tombstones(), &[(on.packet, MixedOnsetRefusal::Reset)]);
    assert_eq!(old.bridge().fault(), Some(MixedBridgeFault::Ledger));
    assert_eq!(old.unclassified().len(), 1);
}

#[test]
fn a_successor_fault_before_an_old_fault_keeps_the_live_cause_at_commit() {
    let mut rig = ResetRig::new();
    rig.seal().expect("seal");
    rig.render_to(B.as_u64());
    rig.offer().expect("offer");
    rig.open_b();
    while !matches!(rig.wrapper.state(), MixedBridgeResetState::Retired(_)) {
        rig.render(Q).expect("the old owner retires");
    }
    let receipt = rig.receipt();
    let BridgeReset::Sealed { successor, .. } = &rig.control.state.reset else {
        panic!("reset remains sealed");
    };
    successor
        .transfer
        .inner
        .ingress_halt
        .store(true, Ordering::Release);

    let old_cause = MixedBridgeFault::Transfer(MixedTransferFault::ForeignEndpoint);
    assert_eq!(
        rig.control
            .pump(rig.next_port.as_mut().expect("successor port")),
        Err(MixedBridgeError::Fault(old_cause))
    );
    let live_cause = MixedBridgeFault::Transfer(MixedTransferFault::Remote);
    assert_eq!(rig.control.fault(), Some(old_cause));
    let retired = rig.control.commit_reset(receipt).expect("custody moves");
    assert_eq!(rig.control.fault(), Some(live_cause));
    assert_eq!(retired.fault(), Some(old_cause));
    let old_port = core::mem::replace(&mut rig.port, rig.next_port.take().expect("successor port"));
    let (live, old) = rig.join_committed(retired, old_port);
    assert_eq!(live.fault(), Some(live_cause));
    assert_eq!(old.bridge().fault(), Some(old_cause));
}

#[test]
fn a_successor_halted_at_collect_entry_keeps_its_live_cause() {
    let mut rig = ResetRig::new();
    rig.seal().expect("seal");
    rig.render_to(B.as_u64());
    rig.offer().expect("offer");
    rig.open_b();
    while !matches!(rig.wrapper.state(), MixedBridgeResetState::Retired(_)) {
        rig.render(Q).expect("the old owner retires");
    }
    let receipt = rig.receipt();
    let BridgeReset::Sealed { successor, .. } = &rig.control.state.reset else {
        panic!("reset remains sealed");
    };
    successor
        .transfer
        .inner
        .ingress_halt
        .store(true, Ordering::Release);

    let old_cause = MixedBridgeFault::Transfer(MixedTransferFault::ForeignEndpoint);
    assert_eq!(
        rig.control
            .collect(rig.next_port.as_mut().expect("successor port")),
        Err(MixedBridgeError::Fault(old_cause))
    );
    let BridgeReset::Sealed {
        successor_first, ..
    } = rig.control.state.reset
    else {
        panic!("reset remains sealed");
    };
    assert!(successor_first);
    let retired = rig.control.commit_reset(receipt).expect("custody moves");
    let live_cause = MixedBridgeFault::Transfer(MixedTransferFault::Remote);
    assert_eq!(rig.control.fault(), Some(live_cause));
    assert_eq!(retired.fault(), Some(old_cause));
    let old_port = core::mem::replace(&mut rig.port, rig.next_port.take().expect("successor port"));
    let (live, old) = rig.join_committed(retired, old_port);
    assert_eq!(live.fault(), Some(live_cause));
    assert_eq!(old.bridge().fault(), Some(old_cause));
}

#[test]
fn a_prior_old_audio_halt_wins_over_a_later_ledger_fault_while_sealed() {
    let mut rig = ResetRig::new();
    rig.seal().expect("seal");
    rig.control
        .transfer
        .inner
        .ingress_halt
        .store(true, Ordering::Release);
    let prior = MixedBridgeFault::Transfer(MixedTransferFault::Remote);
    assert_eq!(
        rig.control.halt(MixedBridgeFault::Ledger),
        MixedBridgeError::Fault(prior)
    );
    assert_eq!(rig.control.fault(), Some(prior));
    let BridgeReset::Sealed { successor, .. } = &rig.control.state.reset else {
        panic!("reset remains sealed");
    };
    assert_eq!(successor.fault(), Some(prior));
}

#[test]
fn a_successor_halt_after_collect_entry_does_not_relabel_the_old_fault() {
    let mut rig = ResetRig::new();
    rig.seal().expect("seal");
    let successor_was_halted = rig.control.sealed_successor_halted();
    assert!(!successor_was_halted);

    let old_cause = MixedTransferFault::ForeignEndpoint;
    rig.control.transfer.halt(old_cause);
    let BridgeReset::Sealed { successor, .. } = &rig.control.state.reset else {
        panic!("reset remains sealed");
    };
    successor
        .transfer
        .inner
        .ingress_halt
        .store(true, Ordering::Release);
    rig.control.record_sealed_fault(
        MixedBridgeFault::Transfer(old_cause),
        successor_was_halted,
        Some(MixedBridgeFault::Transfer(old_cause)),
    );
    assert_eq!(
        rig.control.fault(),
        Some(MixedBridgeFault::Transfer(old_cause))
    );
    let BridgeReset::Sealed {
        successor_first, ..
    } = rig.control.state.reset
    else {
        panic!("reset remains sealed");
    };
    assert!(!successor_first);
}

#[test]
fn pumping_with_the_old_port_after_commit_is_a_terminal_foreign_endpoint() {
    let mut rig = ResetRig::new();
    rig.render_to(B_FRAMES - Q);
    let (retired, mut old_port) = reset_at_b(&mut rig);
    let fault = MixedBridgeFault::Transfer(MixedTransferFault::ForeignEndpoint);
    assert_eq!(
        rig.control.pump(&mut old_port),
        Err(MixedBridgeError::Fault(fault))
    );
    assert_eq!(rig.pump(), Err(MixedBridgeError::Halted(fault)));
    assert_eq!(retired.fault(), None, "the old transfer is not blamed");
}

#[test]
fn staging_never_waits_while_sealed_and_only_a_refused_key_up_leaves_its_lane() {
    let mut rig = ResetRig::new();
    rig.release_peer(1 << 16);
    rig.render_to(B_FRAMES - Q);
    rig.seal().expect("seal");
    // Eight cells hold four onsets with their protected key-up places.
    let onsets: Vec<_> = (0..4_u8)
        .map(|offset| {
            rig.first
                .handoff(400 + u64::from(offset), [0x90, 60 + offset, 100])
        })
        .collect();
    for &onset in &onsets {
        assert_eq!(rig.stage(onset), Ok(MixedStaged::Staged));
    }
    let refused = rig.first.handoff(410, [0x90, 70, 100]);
    assert_eq!(
        rig.stage(refused),
        Ok(MixedStaged::Split(MixedSplit::StageFull))
    );
    let bend = rig.first.handoff(411, [0xE0, 0, 0x40]);
    assert_eq!(
        rig.stage(bend),
        Ok(MixedStaged::Split(MixedSplit::StageFull))
    );
    // Key-ups spend their onsets' protected places and are never refused for capacity.
    for (offset, onset) in onsets.iter().enumerate() {
        let key = 60 + u8::try_from(offset).expect("key");
        let up = rig.first.handoff(420 + offset as u64, [0x80, key, 0]);
        assert_eq!(rig.stage(up), Ok(MixedStaged::Staged), "{onset:?}");
    }
    let refused_up = rig.first.handoff(430, [0x80, 70, 0]);
    assert_eq!(
        rig.stage(refused_up),
        Ok(MixedStaged::Split(MixedSplit::AfterRefusedOnset {
            onset: refused.packet,
            reason: MixedOnsetRefusal::StageFull,
        }))
    );
    assert_eq!(
        rig.control.lane_charge(rig.first.generation),
        Some((MixedTransferPlaces::new(8), MixedTransferPlaces::NONE))
    );
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: B }))
    );
    assert_eq!(rig.control.last_stamp(), None, "nothing was forwarded");
}

#[test]
fn a_refused_key_up_head_leaves_its_lane_while_sealed() {
    let mut rig = ResetRig::new();
    rig.release_peer(1 << 16);
    rig.render_to(B_FRAMES - Q);
    // The handle table holds no cell, so the onset is split with its key-up staged.
    let limit = rig.control.state.handles.limit;
    rig.control.state.handles.limit = 0;
    let on = rig.first.handoff(400, ON);
    let up = rig.first.handoff(401, OFF);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    assert_eq!(rig.stage(up), Ok(MixedStaged::Staged));
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Split {
            packet: on.packet,
            split: MixedSplit::CommandsFull,
        })
    );
    rig.control.state.handles.limit = limit;
    rig.seal().expect("seal");
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Split {
            packet: up.packet,
            split: MixedSplit::AfterRefusedOnset {
                onset: on.packet,
                reason: MixedOnsetRefusal::CommandsFull,
            },
        })
    );
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: B }))
    );
}

#[test]
fn a_seal_refusal_returns_the_successor_and_leaves_the_bridge_open() {
    let mut rig = ResetRig::new();
    let sources = rig.sources;
    let refuse = |rig: &mut ResetRig, next: MixedBridgeControl, at: SampleTime| {
        let refused = rig.control.seal_reset(next, at).expect_err("refused seal");
        (refused.reason(), refused.into_successor())
    };
    let next = rig.next_control.take().expect("successor");

    let (reason, next) = refuse(&mut rig, next, SampleTime::new(B_FRAMES + 1));
    assert_eq!(
        reason,
        MixedBridgeSealRefusal::NotBoundary {
            at: SampleTime::new(B_FRAMES + 1)
        }
    );

    // Swapped tie order.
    let swapped = MixedTransferSources::new(sources.second(), sources.first()).expect("sources");
    let (other, _, _) = {
        let MixedBridgeArmed {
            control,
            audio,
            control_port,
            audio_port,
        } = arm_at(B, swapped, fence());
        (
            control,
            control_port,
            Owner::new(audio, audio_port).expect("own port"),
        )
    };
    assert_eq!(
        refuse(&mut rig, other, B).0,
        MixedBridgeSealRefusal::Sources
    );

    // Other bounds.
    let narrower = MixedTransferFence {
        stage_cells: MixedTransferPlaces::new(4),
        ..fence()
    };
    let MixedBridgeArmed { control: other, .. } = arm_at(B, sources, narrower);
    assert_eq!(refuse(&mut rig, other, B).0, MixedBridgeSealRefusal::Bounds);

    // A successor that consumed a source packet.
    let (mut busy, _, _) = successor(sources);
    let packet = MixedSourcePacketId::new(sources.first(), 1).expect("first serial");
    busy.advance_frontier(packet, SampleTime::new(10))
        .expect("frontier");
    assert_eq!(
        refuse(&mut rig, busy, B).0,
        MixedBridgeSealRefusal::SuccessorBusy
    );

    // A command already forwarded at or after `B`.
    rig.release_peer(1 << 16);
    let on = rig.first.handoff(B_FRAMES, ON);
    assert_eq!(rig.stage(on), Ok(MixedStaged::Staged));
    rig.render_to(B_FRAMES - Q);
    assert!(matches!(rig.pump(), Ok(MixedPumpStep::Forwarded { .. })));
    let (reason, next) = refuse(&mut rig, next, B);
    assert_eq!(reason, MixedBridgeSealRefusal::Forwarded { last: B, at: B });
    rig.render(Q).expect("callback before B");
    assert_eq!(rig.collect().len(), 1);

    // The audio clock has passed the boundary.
    rig.render(Q).expect("callback after B");
    let (reason, next) = refuse(&mut rig, next, B);
    assert_eq!(
        reason,
        MixedBridgeSealRefusal::Passed {
            at: B,
            boundary: SampleTime::new(B_FRAMES + Q)
        }
    );

    // A later boundary seals once; a second seal is refused.
    let later = SampleTime::new(B_FRAMES + 4 * Q);
    rig.control.seal_reset(next, later).expect("seal");
    let (third, _, _) = successor(sources);
    assert_eq!(
        refuse(&mut rig, third, later).0,
        MixedBridgeSealRefusal::AlreadySealed { at: later }
    );
    assert_eq!(rig.control.reset_ready(), Some(later));

    // A faulted bridge.
    let mut faulted = ResetRig::new();
    let _halt = faulted.control.report_raw_self_halt();
    let next = faulted.next_control.take().expect("successor");
    assert_eq!(
        refuse(&mut faulted, next, B).0,
        MixedBridgeSealRefusal::Faulted
    );
}

#[test]
fn a_pending_panic_refuses_the_seal_and_a_due_panic_cancels_a_sealed_reset() {
    let mut rig = ResetRig::new();
    rig.release_peer(1 << 16);
    let (packet, frontier) = rig.first.frontier(1 << 16);
    rig.control
        .advance_frontier(packet, frontier)
        .expect("frontier");
    rig.render_to(256);
    let at = SampleTime::new(300);
    assert_eq!(rig.control.panic(at), Ok(()));
    let refuse = |rig: &mut ResetRig| {
        let next = rig.next_control.take().expect("successor");
        let refused = rig
            .control
            .seal_reset(next, B)
            .expect_err("a pending Panic");
        let reason = refused.reason();
        rig.next_control = Some(refused.into_successor());
        reason
    };
    assert_eq!(
        refuse(&mut rig),
        MixedBridgeSealRefusal::PanicPending { at }
    );
    assert!(matches!(rig.pump(), Ok(MixedPumpStep::Panic { stamp, .. }) if stamp == at));
    assert_eq!(
        refuse(&mut rig),
        MixedBridgeSealRefusal::PanicPending { at }
    );
    rig.render(Q).expect("the Panic's callback");
    assert_eq!(
        refuse(&mut rig),
        MixedBridgeSealRefusal::PanicPending { at }
    );
    let reports = rig.collect();
    assert!(
        matches!(
            reports.as_slice(),
            [report] if report.packet().is_none()
                && matches!(report.outcome(), MixedBridgeOutcome::Panic(Ok(_)))
        ),
        "{reports:?}"
    );
    // Collection frees the cell, so the reset seals.
    rig.seal().expect("the Panic cell is free");
    rig.render(Q).expect("a later quantum opens");
    assert_eq!(
        rig.control.reset_phase(),
        MixedBridgeResetPhase::Sealed { at: B }
    );
    // Stamped inside `B`'s own quantum, so still due there: it cancels the reset.
    let due = SampleTime::new(B_FRAMES + 10);
    assert_eq!(rig.control.panic(due), Ok(()));
    assert_eq!(
        rig.control.reset_phase(),
        MixedBridgeResetPhase::Cancelled { at: B, panic: due }
    );
    assert_eq!(rig.control.reset_ready(), None);
    assert_eq!(rig.control.panic_pending(), Some(due));
    assert_eq!(rig.offer(), Err(MixedBridgeResetRefusal::PanicCancelled));
    let mut applied = ResetRig::new();
    applied.render_to(B_FRAMES - Q);
    applied.seal().expect("seal");
    applied.offer().expect("offer");
    applied.open_b();
    assert_eq!(
        rig.control.commit_reset(applied.receipt()).map(drop),
        Err(MixedBridgeCommitRefusal::Cancelled { at: B })
    );
    let (third, _, _) = successor(rig.sources);
    let refused = rig.control.seal_reset(third, B).expect_err("cancelled");
    assert_eq!(
        refused.reason(),
        MixedBridgeSealRefusal::Cancelled { at: B }
    );
    assert_eq!(
        rig.control.reset_phase(),
        MixedBridgeResetPhase::Cancelled { at: B, panic: due },
        "every refusal left the cancelled reset as it was"
    );

    // The Panic forwards to the old transfer, which serves it at its stamp inside `B`.
    let Ok(MixedPumpStep::Panic { id, stamp }) = rig.pump() else {
        panic!("the Panic forwards to the old transfer");
    };
    assert_eq!(stamp, due);
    rig.render_to(B_FRAMES + Q);
    let reports = rig.collect();
    assert!(
        matches!(
            reports.as_slice(),
            [report] if report.id() == id
                && report.executed_at() == due
                && matches!(report.outcome(), MixedBridgeOutcome::Panic(Ok(_)))
        ),
        "{reports:?}"
    );
    assert_eq!(rig.wrapper.state(), MixedBridgeResetState::Empty);

    // A cancelled reset refuses the join until abandon returns the successor.
    let ResetRig {
        control,
        port,
        wrapper,
        next_port,
        next_owner,
        ..
    } = rig;
    let (active, secondary) = wrapper.into_owners();
    assert!(secondary.is_none());
    let (audio, audio_port) = active.into_parts();
    let refused = control
        .join(audio, Some(port), Some(audio_port))
        .expect_err("cancelled");
    assert_eq!(
        refused.reason(),
        MixedBridgeJoinError::ResetCancelled { at: B }
    );
    let (mut control, audio, port, audio_port) = refused.into_parts();
    let owner = next_owner.expect("the successor the offer refused");
    let next = control
        .abandon_reset(&owner)
        .expect("a cancelled reset is abandoned");
    assert_eq!(control.reset_phase(), MixedBridgeResetPhase::Open);
    let teardown = control
        .join(audio, port, audio_port)
        .expect("the old arm joins");
    assert_eq!(teardown.panic(), None);
    assert_eq!(
        teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    let (next_audio, next_audio_port) = owner.into_parts();
    let teardown = next
        .join(next_audio, next_port, Some(next_audio_port))
        .expect("the abandoned successor joins");
    assert_eq!(teardown.panic(), None);
    assert!(teardown.held().is_empty());
    assert_eq!(
        teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

/// Assert that one collected Panic report ended exactly `expected`, in any group order.
fn assert_panic_ended(report: &MixedBridgeReport, expected: &[MixedSourcePacketId]) {
    let MixedBridgeOutcome::Panic(Ok(panicked)) = report.outcome() else {
        panic!("not an applied Panic: {report:?}");
    };
    let ended: Vec<_> = panicked.ended().map(MixedBridgePanicEnd::onset).collect();
    assert_eq!(ended.len(), expected.len(), "{ended:?}");
    assert!(
        expected.iter().all(|onset| ended.contains(onset)),
        "{ended:?} is not {expected:?}"
    );
}

#[test]
fn a_panic_stamped_after_the_fade_quantum_follows_the_reset_outcome() {
    for commit in [true, false] {
        let mut rig = ResetRig::new();
        let old_on = sounding_old_note(&mut rig);
        rig.seal().expect("seal");
        let new_on = rig.first.handoff(B_FRAMES + 20, [0x90, 64, 100]);
        assert_eq!(rig.stage(new_on), Ok(MixedStaged::Staged));
        rig.offer().expect("offer");
        // The first sample after `B`'s quantum: never due at `B`, so it never cancels.
        let late = SampleTime::new(B_FRAMES + Q);
        assert_eq!(rig.control.panic(late), Ok(()));
        assert_eq!(
            rig.control.reset_phase(),
            MixedBridgeResetPhase::Sealed { at: B }
        );
        assert_eq!(
            rig.pump(),
            Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: B }))
        );
        let (packet, at) = rig.first.frontier(B_FRAMES + 2 * Q);
        rig.control.advance_frontier(packet, at).expect("frontier");
        if commit {
            rig.open_b();
            let (retired, old_port) = rig.commit();
            assert_eq!(
                retired.receipt().ended().count(),
                1,
                "the Reset group ends the old note at B + Q"
            );
            let steps = pump_all(&mut rig);
            assert!(
                matches!(
                    steps.as_slice(),
                    [
                        MixedPumpStep::Forwarded { packet, .. },
                        MixedPumpStep::Panic { stamp, .. },
                    ] if *packet == new_on.packet && *stamp == late
                ),
                "{steps:?}"
            );
            for _ in 0..2 {
                rig.render(Q).expect("successor callback");
            }
            let reports = rig.collect();
            let [onset, panic] = reports.as_slice() else {
                panic!("the successor served both: {reports:?}");
            };
            assert_eq!(onset.packet(), Some(new_on.packet));
            assert_eq!(panic.executed_at(), late);
            // The Panic ends only the successor's note.
            assert_panic_ended(panic, &[new_on.packet]);
            let (live, old) = rig.join_committed(retired, old_port);
            assert_eq!(live.panic(), None);
            assert_eq!(old.bridge().panic(), None);
            assert!(matches!(
                old.handles(),
                [(packet, MixedRetiredHandle::Ended(_))] if *packet == old_on.packet
            ));
        } else {
            let returned = rig.wrapper.withdraw_pending().expect("pending successor");
            let next = rig
                .control
                .abandon_reset(&returned)
                .expect("the withdrawn successor is fresh");
            let steps = pump_all(&mut rig);
            assert!(
                matches!(
                    steps.as_slice(),
                    [
                        MixedPumpStep::Forwarded { packet, .. },
                        MixedPumpStep::Panic { stamp, .. },
                    ] if *packet == new_on.packet && *stamp == late
                ),
                "{steps:?}"
            );
            rig.render_to(B_FRAMES + 2 * Q);
            let reports = rig.collect();
            let [onset, panic] = reports.as_slice() else {
                panic!("the old owner served both: {reports:?}");
            };
            assert_eq!(onset.packet(), Some(new_on.packet));
            assert_eq!(panic.executed_at(), late);
            // The Panic ends both notes on the old owner.
            assert_panic_ended(panic, &[old_on.packet, new_on.packet]);
            let ResetRig {
                control,
                port,
                wrapper,
                next_port,
                ..
            } = rig;
            let (active, secondary) = wrapper.into_owners();
            assert!(secondary.is_none());
            let (audio, audio_port) = active.into_parts();
            let teardown = control
                .join(audio, Some(port), Some(audio_port))
                .expect("the old arm joins");
            assert_eq!(teardown.panic(), None);
            let (next_audio, next_audio_port) = returned.into_parts();
            let teardown = next
                .join(next_audio, next_port, Some(next_audio_port))
                .expect("the abandoned successor joins");
            assert_eq!(teardown.panic(), None);
        }
    }
}

/// Audio refuses the successor at `B` for another reason first, and declines the seal. A
/// due Panic accepted after that loses its cancel, waits for the abandon and reaches the
/// old owner. The successor's halt is injected as the successor-fault tests above do:
/// with the old owner stopped and nothing forwarded while sealed, no other boundary
/// refusal can follow an accepted offer through the bridge.
#[test]
fn a_panic_after_a_boundary_refusal_waits_for_abandon_and_reaches_the_old_owner() {
    let mut rig = ResetRig::new();
    let old_on = sounding_old_note(&mut rig);
    let (packet, at) = rig.first.frontier(B_FRAMES + Q);
    rig.control.advance_frontier(packet, at).expect("frontier");
    rig.seal().expect("seal");
    rig.offer().expect("offer");
    let BridgeReset::Sealed { successor, .. } = &rig.control.state.reset else {
        panic!("reset remains sealed");
    };
    successor
        .transfer
        .inner
        .ingress_halt
        .store(true, Ordering::Release);
    rig.open_b();
    assert_eq!(
        rig.wrapper.state(),
        MixedBridgeResetState::Refused {
            at: B,
            reason: MixedBridgeResetRefusal::SuccessorNotFresh,
        }
    );

    // Due inside `B`'s quantum, but audio already declined the seal: the cancel loses.
    let due = SampleTime::new(B_FRAMES + 10);
    assert_eq!(rig.control.panic(due), Ok(()));
    assert_eq!(
        rig.control.reset_phase(),
        MixedBridgeResetPhase::Sealed { at: B }
    );
    assert_eq!(
        rig.pump(),
        Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at: B }))
    );
    let owner = rig
        .wrapper
        .take_refused()
        .expect("refused successor")
        .into_owner();
    let next = rig
        .control
        .abandon_reset(&owner)
        .expect("the refused successor is fresh");
    assert_eq!(rig.control.reset_phase(), MixedBridgeResetPhase::Open);
    // Audio served `B`'s quantum when it opened it, so the first sample the bridge can
    // still stamp is the next quantum's: the Panic is late, not sample-exact.
    let served = rig.control.audio_boundary();
    assert_eq!(served, SampleTime::new(B_FRAMES + Q));
    let steps = pump_all(&mut rig);
    assert!(
        matches!(steps.as_slice(), [MixedPumpStep::Panic { stamp, .. }] if *stamp == served),
        "{steps:?}"
    );
    rig.render(Q).expect("old callback");
    let reports = rig.collect();
    let [panic] = reports.as_slice() else {
        panic!("the old owner served the Panic: {reports:?}");
    };
    assert_eq!(panic.nominal_at(), due);
    assert_eq!(panic.executed_at(), served);
    assert_panic_ended(panic, &[old_on.packet]);

    let ResetRig {
        control,
        port,
        wrapper,
        next_port,
        ..
    } = rig;
    let (active, secondary) = wrapper.into_owners();
    assert!(secondary.is_none());
    let (audio, audio_port) = active.into_parts();
    let teardown = control
        .join(audio, Some(port), Some(audio_port))
        .expect("the old arm joins");
    assert_eq!(teardown.panic(), None);
    assert_eq!(
        teardown.fault(),
        None,
        "the successor's halt stayed its own"
    );
    let (next_audio, next_audio_port) = owner.into_parts();
    let teardown = next
        .join(next_audio, next_port, Some(next_audio_port))
        .expect("the abandoned successor joins");
    assert_eq!(teardown.panic(), None);
}

/// No successor was offered, so `B` passes undecided and a Panic due there still wins
/// the seal. Accepted after audio rendered its stamp, it reaches the old owner late: the
/// report keeps its nominal stamp and names the later sample it executed at.
#[test]
fn a_due_panic_accepted_after_an_unoffered_boundary_cancels_and_executes_late() {
    let mut rig = ResetRig::new();
    let old_on = sounding_old_note(&mut rig);
    let (packet, at) = rig.first.frontier(B_FRAMES + Q);
    rig.control.advance_frontier(packet, at).expect("frontier");
    rig.seal().expect("seal");
    rig.render_to(B_FRAMES + Q);
    let nominal = SampleTime::new(B_FRAMES - 8);
    assert_eq!(rig.control.panic(nominal), Ok(()));
    assert_eq!(
        rig.control.reset_phase(),
        MixedBridgeResetPhase::Cancelled {
            at: B,
            panic: nominal
        }
    );
    assert!(matches!(
        rig.offer(),
        Err(MixedBridgeResetRefusal::BoundaryPassed { at: B, .. })
    ));
    let opened = SampleTime::new(B_FRAMES + Q);
    let steps = pump_all(&mut rig);
    assert!(
        matches!(steps.as_slice(), [MixedPumpStep::Panic { stamp, .. }] if *stamp == opened),
        "{steps:?}"
    );
    rig.render(Q).expect("old callback");
    let reports = rig.collect();
    let [panic] = reports.as_slice() else {
        panic!("the old owner served the Panic: {reports:?}");
    };
    assert_eq!(panic.nominal_at(), nominal);
    assert_eq!(
        panic.executed_at(),
        opened,
        "accepted after audio rendered its stamp, the Panic is late, not sample-exact"
    );
    assert_panic_ended(panic, &[old_on.packet]);
    let owner = rig.next_owner.take().expect("the never-offered successor");
    let next = rig
        .control
        .abandon_reset(&owner)
        .expect("the never-offered successor is fresh");
    let (audio, audio_port) = owner.into_parts();
    let teardown = next
        .join(audio, rig.next_port.take(), Some(audio_port))
        .expect("the abandoned successor joins");
    assert_eq!(teardown.panic(), None);
}

/// Where a cancelling Panic waits when its two arms are torn down.
#[derive(Clone, Copy, Debug)]
enum CancelledPanic {
    /// Waiting for a silent peer, never forwarded.
    Staged,
    /// Forwarded and not yet served.
    Unserved,
    /// Served into audio's park for a later quantum.
    Parked,
    /// Served with its result never collected.
    Uncollected,
}

#[test]
fn a_cancelled_reset_refuses_join_until_abandon_and_every_owner_tears_down_with_the_panic_once() {
    for place in [
        CancelledPanic::Staged,
        CancelledPanic::Unserved,
        CancelledPanic::Parked,
        CancelledPanic::Uncollected,
    ] {
        let mut rig = ResetRig::new();
        if !matches!(place, CancelledPanic::Staged) {
            rig.release_peer(1 << 16);
            let (packet, at) = rig.first.frontier(1 << 16);
            rig.control.advance_frontier(packet, at).expect("frontier");
        }
        rig.render_to(B_FRAMES - 2 * Q);
        rig.seal().expect("seal");
        rig.offer().expect("offer");
        let at = match place {
            CancelledPanic::Parked => SampleTime::new(B_FRAMES + 10),
            _ => SampleTime::new(B_FRAMES - 48),
        };
        assert_eq!(rig.control.panic(at), Ok(()));
        assert!(matches!(
            rig.control.reset_phase(),
            MixedBridgeResetPhase::Cancelled { at: B, .. }
        ));
        let forwarded = match rig.pump() {
            Ok(MixedPumpStep::Panic { id, .. }) => Some(id),
            Ok(MixedPumpStep::Idle(MixedBridgeWait::Peer { .. })) => None,
            other => panic!("{place:?}: {other:?}"),
        };
        let expected = match place {
            CancelledPanic::Staged => MixedSourceCustody::Staged,
            CancelledPanic::Unserved => MixedSourceCustody::Command {
                id: forwarded.expect("forwarded"),
                place: MixedCommandPlace::Unserved,
            },
            CancelledPanic::Parked => {
                rig.render(Q).expect("the callback that parks it");
                MixedSourceCustody::Command {
                    id: forwarded.expect("forwarded"),
                    place: MixedCommandPlace::Parked,
                }
            }
            CancelledPanic::Uncollected => {
                rig.render_to(B_FRAMES + Q);
                assert_eq!(
                    rig.wrapper.state(),
                    MixedBridgeResetState::Refused {
                        at: B,
                        reason: MixedBridgeResetRefusal::PanicCancelled,
                    }
                );
                MixedSourceCustody::Command {
                    id: forwarded.expect("forwarded"),
                    place: MixedCommandPlace::Uncollected,
                }
            }
        };

        let ResetRig {
            control,
            port,
            wrapper,
            next_port,
            ..
        } = rig;
        let (active, secondary) = wrapper.into_owners();
        let successor = secondary.expect("the pending or refused successor");
        let (audio, audio_port) = active.into_parts();
        let refused = control
            .join(audio, Some(port), Some(audio_port))
            .expect_err("cancelled");
        assert_eq!(
            refused.reason(),
            MixedBridgeJoinError::ResetCancelled { at: B }
        );
        let (mut control, audio, port, audio_port) = refused.into_parts();
        let next = control
            .abandon_reset(&successor)
            .expect("the successor is fresh");
        let old = control
            .join(audio, port, audio_port)
            .expect("the old arm joins");
        let (next_audio, next_audio_port) = successor.into_parts();
        let new = next
            .join(next_audio, next_port, Some(next_audio_port))
            .expect("the successor arm joins");
        assert_eq!(old.panic(), Some(expected), "{place:?}");
        assert_eq!(new.panic(), None, "{place:?}: the successor never had it");
        assert!(new.held().is_empty() && new.transfer().outstanding().is_empty());
        for teardown in [&old, &new] {
            assert_eq!(
                teardown.transfer().custody(),
                MixedTransferCustody::Complete,
                "{place:?}"
            );
        }
        let ids: Vec<_> = old
            .transfer()
            .outstanding()
            .iter()
            .chain(new.transfer().outstanding())
            .copied()
            .collect();
        assert_eq!(ids, forwarded.into_iter().collect::<Vec<_>>(), "{place:?}");
    }
}

#[test]
fn the_bridge_arm_requires_the_release_share_for_one_compiled_panic_cut() {
    let profile = history_tests::mixed_profile();
    let depth = MixedJoinedPrepared::transfer_capacity(&profile).get();
    let events = profile.limits().events();
    let capacity = events.queues().performance_ingress_capacity();
    let holds = events.shares().release_hold_capacity();
    // The live backlog's share alone, with nothing left for a compiled Panic cut.
    let below = history_tests::profile_with_release_limits(
        capacity,
        EventCount::measured(holds.get().min(capacity.get())),
    );
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let first = Source::new("first");
    let second = Source::new("second");
    let sources = MixedTransferSources::new(first.generation, second.generation).expect("sources");
    let refusal = prepared
        .arm_bridge_rehearsal(
            candidate,
            &below,
            sources,
            fence(),
            Ring(HeapRb::new(depth + 1)),
            Ring(HeapRb::new(depth + 1)),
        )
        .expect_err("below the Panic share");
    assert!(
        matches!(
            refusal.reason(),
            MixedTransferArmError::Owner(MixedTransferOwnerRefusal(
                MixedOneShotArmError::CompiledPanicReleaseShare { share, .. }
            )) if share == capacity
        ),
        "{:?}",
        refusal.reason()
    );
    // The single-source rehearsal carries no Panic and arms under the same share.
    let (prepared, candidate, commands, results) = refusal.into_parts();
    let _armed = prepared
        .arm_transfer_rehearsal(candidate, &below, commands, results)
        .expect("the single-source arm needs no Panic share");
}

#[test]
fn a_sealed_bridge_refuses_to_join_and_returns_every_part() {
    let mut rig = ResetRig::new();
    rig.seal().expect("seal");
    let ResetRig {
        control,
        port,
        wrapper,
        ..
    } = rig;
    let (active, _) = wrapper.into_owners();
    let (audio, audio_port) = active.into_parts();
    let refused = control
        .join(audio, Some(port), Some(audio_port))
        .expect_err("sealed");
    assert_eq!(
        refused.reason(),
        MixedBridgeJoinError::ResetSealed { at: B }
    );
    let (control, audio, port, audio_port) = refused.into_parts();
    assert_eq!(
        control.reset_ready(),
        Some(B),
        "the bridge came back still sealed"
    );
    assert!(port.is_some() && audio_port.is_some());
    drop(audio);
}

/// Every staged source ID is settled exactly once across stage and pump returns, both
/// collections and both joined teardowns, including an old onset whose result is still
/// uncollected when the old arm joins.
#[test]
fn every_staged_packet_is_accounted_exactly_once_across_both_teardowns() {
    let mut rig = ResetRig::new();
    let mut staged = Vec::new();
    let mut settled = Vec::new();
    let mut stage = |rig: &mut ResetRig, handoff: MixedSourceHandoff, settled: &mut Vec<_>| {
        staged.push(handoff.packet);
        match rig.stage(handoff).expect("stage") {
            MixedStaged::Staged => {}
            MixedStaged::Split(_) => settled.push(handoff.packet),
        }
    };
    let record = |steps: Vec<MixedPumpStep>, settled: &mut Vec<_>| {
        for step in steps {
            if let MixedPumpStep::Split { packet, .. } = step {
                settled.push(packet);
            }
        }
    };
    rig.release_peer(300);
    let held_60 = rig.first.handoff(100, ON);
    let held_62 = rig.first.handoff(110, [0x90, 62, 100]);
    stage(&mut rig, held_60, &mut settled);
    stage(&mut rig, held_62, &mut settled);
    let steps = pump_all(&mut rig);
    record(steps, &mut settled);
    rig.render_to(B_FRAMES - 2 * Q);
    // One old result is collected before the reset, the other never by the bridge.
    let first = rig
        .control
        .collect(&mut rig.port)
        .expect("collect")
        .expect("a served onset");
    settled.push(first.packet().expect("a source report"));

    rig.seal().expect("seal");
    rig.render(Q).expect("callback before B");
    // Staged while sealed: the old key 60's key-up, a new onset on the second source
    // and a bend that will wait outside the successor's window.
    let old_up = rig.first.handoff(420, OFF);
    let new_on = rig.second.handoff(430, [0x91, 64, 100]);
    stage(&mut rig, old_up, &mut settled);
    stage(&mut rig, new_on, &mut settled);
    let (retired, old_port) = reset_at_b(&mut rig);
    // The first source promises nothing before the live clock, so the second's onset
    // need not wait for it.
    let live_at = rig.control.audio_boundary().as_u64();
    let (packet, at) = rig.first.frontier(live_at);
    rig.control.advance_frontier(packet, at).expect("frontier");
    let steps = pump_all(&mut rig);
    record(steps, &mut settled);
    // After commit: key 62's key-up meets its reset tombstone at stage, the new note's
    // key-up goes to the successor and a far onset stays staged.
    let old_up_62 = rig.first.handoff(live_at + 1, [0x80, 62, 0]);
    stage(&mut rig, old_up_62, &mut settled);
    let new_up = rig.second.handoff(live_at + 2, [0x81, 64, 0]);
    stage(&mut rig, new_up, &mut settled);
    let far = rig.first.handoff(live_at + 8 * WINDOW, [0x90, 65, 100]);
    stage(&mut rig, far, &mut settled);
    let steps = pump_all(&mut rig);
    record(steps, &mut settled);
    rig.render(Q).expect("successor callback");
    settled.extend(rig.collect().iter().filter_map(MixedBridgeReport::packet));
    assert_eq!(retired.outstanding(), MixedTransferPlaces::new(1));
    assert_eq!(
        rig.control.held_onsets(rig.first.generation),
        Some(MixedTransferPlaces::new(1))
    );

    let (live, old) = rig.join_committed(retired, old_port);
    settled.extend(live.held().iter().map(|(packet, _)| *packet));
    settled.extend(old.bridge().held().iter().map(|(packet, _)| *packet));
    assert_eq!(staged.len(), 7);
    assert_eq!(settled.len(), staged.len(), "{settled:?}");
    for packet in &staged {
        assert_eq!(
            settled.iter().filter(|settled| *settled == packet).count(),
            1,
            "{packet:?} is settled exactly once"
        );
    }
    assert!(matches!(
        old.bridge().held(),
        [(packet, MixedSourceCustody::Command { place: MixedCommandPlace::Uncollected, .. })]
            if *packet == held_62.packet
    ));
    let mut old_handles: Vec<_> = old
        .handles()
        .iter()
        .map(|(packet, class)| (packet.serial(), *class))
        .collect();
    old_handles.sort_by_key(|(serial, _)| *serial);
    assert_eq!(old_handles.len(), 2);
    assert!(
        old_handles
            .iter()
            .all(|(_, class)| matches!(class, MixedRetiredHandle::Ended(_)))
    );
    assert!(old.unclassified().is_empty() && old.unmatched().is_empty());
    assert_eq!(live.held(), [(far.packet, MixedSourceCustody::Staged)]);
    assert!(live.open().is_empty() && live.tombstones().is_empty());
}
