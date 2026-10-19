//! The reset wrapper over host-armed transfer owners (ADR-0078, draft rules 3.2, 3.3 and
//! 3.6).
//!
//! The old arm is armed at 64 from plan position 10, stops at 192 and holds key 48 from
//! 100 across the reset boundary `B = 448`. The successor is a fresh arm of the same plan
//! for `B`, with key 55 waiting in its own host queue for `B + 10`. After the offer the
//! old arm's control submits key 52 for `B + 22` into the old host queue, which the old
//! owner would serve at `B` without the reset. Output index `i` holds engine sample
//! `i - 64`, so `B` is heard from output 512 and the fade ends at output 576.

use std::sync::atomic::Ordering;

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use synth_core::MidiChannel;

use super::super::{
    CompiledPayload, MixedJoinedPrepared, MixedOneShotRenderError, MixedStopAccepted,
    history_tests::{bound_with_events, key, mixed_profile},
    transfer::{
        MixedActivationKind, MixedActivationOffer, MixedActivationOutcome, MixedActivationRefusal,
        MixedActivationSlot, MixedSourceNoteOn, MixedTransferArmed, MixedTransferAudio,
        MixedTransferAudioPort, MixedTransferCommand, MixedTransferCommandId,
        MixedTransferConsumer, MixedTransferControl, MixedTransferControlPort,
        MixedTransferCustody, MixedTransferFault, MixedTransferOutcome, MixedTransferPlaces,
        MixedTransferProducer, MixedTransferQueue, MixedTransferRefusal, MixedTransferRenderError,
        MixedTransferReport, MixedTransferResult, MixedTransferSubmitError,
    },
};
use super::{
    MixedResetForeignEndpoint, MixedResetRefused, MixedResetRenderError, MixedResetSlot,
    MixedResetState, MixedResetTransferAudio, MixedResetTransferOwner,
    host::{
        MixedBridgeResetAudio, MixedBridgeResetEnd, MixedBridgeResetOwner, MixedBridgeResetRefusal,
        MixedBridgeResetState,
    },
    hot::fade_gain,
};
use crate::{
    quantities::{ChannelLayout, EventCount, NoteVelocity},
    render::AudioBlockMut,
    render_allocation::count_allocs,
    schedule::PlanEvent,
    time::{PlanPosition, QUANTUM_FRAMES, SampleTime},
    transport::ActivationSequence,
};

const STOP: SampleTime = SampleTime::new(192);
const B: SampleTime = SampleTime::new(448);
/// The late old command's stamp, `B + 22`.
const LATE: u64 = 470;
const B_OUTPUT: usize = 512;
const FADE_END_OUTPUT: usize = 576;
const AFTER_FRAMES: usize = 256;
const QUANTUM: usize = QUANTUM_FRAMES as usize;

/// Callback partitions of the 512 output frames before `B`.
const BEFORE: [&[usize]; 4] = [&[512], &[64; 8], &[8; 64], &[7, 63, 90, 1, 256, 95]];
/// Callback partitions of the 256 frames from `B`, splitting the fade differently.
const AFTER: [&[usize]; 4] = [&[256], &[64; 4], &[8; 32], &[5, 50, 9, 1, 191]];

/// A preallocated lock-free host ring.
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

type Armed = MixedTransferArmed<Ring<MixedTransferCommand>, Ring<MixedTransferResult>>;
type ControlPort = MixedTransferControlPort<Prod<MixedTransferCommand>, Cons<MixedTransferResult>>;
type AudioPort = MixedTransferAudioPort<Cons<MixedTransferCommand>, Prod<MixedTransferResult>>;
type Owner = MixedResetTransferOwner<Cons<MixedTransferCommand>, Prod<MixedTransferResult>>;
type Wrapper = MixedResetTransferAudio<Cons<MixedTransferCommand>, Prod<MixedTransferResult>>;

fn bound() -> MixedJoinedPrepared {
    bound_with_events(true, |note| {
        let on = |at, raw| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(raw),
                    velocity: NoteVelocity::FULL,
                },
            )
        };
        let off = |at, raw| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(raw),
                },
            )
        };
        vec![on(10, 60), off(300, 60), on(400, 72), off(500, 72)]
    })
}

/// Both queues hold the admitted command/result bound.
fn ring<T>() -> Ring<T> {
    let depth = mixed_profile()
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity()
        .as_usize()
        .expect("the admitted bound fits");
    Ring(HeapRb::new(depth))
}

/// A host-owned transfer whose armed transition takes effect at `requested`.
fn arm(requested: SampleTime) -> Armed {
    let prepared = bound();
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    prepared
        .arm_transfer_rehearsal(candidate, &mixed_profile(), ring(), ring())
        .expect("transfer arms")
}

fn note_on(at: u64, raw: u8) -> MixedSourceNoteOn {
    MixedSourceNoteOn {
        at: SampleTime::new(at),
        channel: MidiChannel::CH1,
        key: key(raw),
        velocity: NoteVelocity::FULL,
    }
}

fn onset(
    control: &mut MixedTransferControl,
    port: &mut ControlPort,
    at: u64,
    raw: u8,
) -> MixedTransferCommandId {
    let (id, _) = control
        .submit_note_on(port, note_on(at, raw))
        .expect("the onset and its key-up place fit");
    id
}

fn reports(control: &mut MixedTransferControl, port: &mut ControlPort) -> Vec<MixedTransferReport> {
    let mut reports = Vec::new();
    while let Some(report) = control.collect(port).expect("ordered results") {
        reports.push(report);
    }
    reports
}

/// The old arm, stopping at 192 and holding key 48 from 100.
fn old_transfer() -> Armed {
    let mut old = arm(SampleTime::new(64));
    assert_eq!(
        old.audio.inner_mut().schedule_compiled_stop(STOP),
        Ok(MixedStopAccepted::Scheduled)
    );
    let _held = onset(&mut old.control, &mut old.control_port, 100, 48);
    old
}

/// A fresh arm for `B`, with key 55 waiting in its own host queue for `B + 10`.
fn successor_transfer() -> (Armed, MixedTransferCommandId) {
    let mut next = arm(B);
    let id = onset(
        &mut next.control,
        &mut next.control_port,
        B.as_u64() + 10,
        55,
    );
    (next, id)
}

fn owner(audio: MixedTransferAudio, port: AudioPort) -> Owner {
    MixedResetTransferOwner::new(audio, port).expect("the arm's own port")
}

fn block(samples: &mut [f32]) -> AudioBlockMut<'_> {
    let frames = samples.len();
    AudioBlockMut::new(samples, frames, ChannelLayout::Mono).expect("mono block")
}

/// Render `samples` through the wrapper over `partition`, requiring every callback to
/// allocate and free nothing.
fn render(wrapper: &mut Wrapper, samples: &mut [f32], partition: &[usize]) {
    assert_eq!(partition.iter().sum::<usize>(), samples.len());
    let mut offset = 0;
    for &frames in partition {
        let output = block(&mut samples[offset..offset + frames]);
        let allocations = count_allocs(|| {
            wrapper.render(output).expect("wrapper callback");
        });
        assert_eq!(allocations, 0, "wrapper callback allocated or freed");
        offset += frames;
    }
}

/// Render one arm alone through its own port in quantum-sized callbacks.
fn render_alone(audio: &mut MixedTransferAudio, port: &mut AudioPort, samples: &mut [f32]) {
    for chunk in samples.chunks_mut(QUANTUM) {
        audio.render(port, block(chunk)).expect("owner callback");
    }
}

/// Offer `successor` at `B` without allocating, requiring acceptance.
fn offer(wrapper: &mut Wrapper, successor: Owner) {
    let mut offered = None;
    let allocations = count_allocs(|| {
        offered = Some(wrapper.offer_reset(successor, B));
    });
    assert_eq!(allocations, 0, "offer allocated or freed");
    assert!(matches!(offered, Some(Ok(()))));
}

/// The old arm alone, with the late command when `late` is set.
fn old_alone(late: bool) -> Vec<f32> {
    let mut old = old_transfer();
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render_alone(
        &mut old.audio,
        &mut old.audio_port,
        &mut samples[..B_OUTPUT],
    );
    if late {
        let _late = onset(&mut old.control, &mut old.control_port, LATE, 52);
    }
    render_alone(
        &mut old.audio,
        &mut old.audio_port,
        &mut samples[B_OUTPUT..],
    );
    samples
}

/// The successor alone from `B`, through its own port.
fn successor_alone() -> Vec<f32> {
    let (mut next, _) = successor_transfer();
    next.audio.inner_mut().install_reset_successor(B);
    let mut samples = vec![0.0_f32; AFTER_FRAMES];
    render_alone(&mut next.audio, &mut next.audio_port, &mut samples);
    samples
}

/// Wrapper output for one pair of partitions, with the late old command in the old port.
fn reset_run(before: &[usize], after: &[usize]) -> Vec<f32> {
    let mut old = old_transfer();
    let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render(&mut wrapper, &mut samples[..B_OUTPUT], before);
    let (next, _) = successor_transfer();
    offer(&mut wrapper, owner(next.audio, next.audio_port));
    let _late = onset(&mut old.control, &mut old.control_port, LATE, 52);
    render(&mut wrapper, &mut samples[B_OUTPUT..], after);
    assert_eq!(wrapper.fault(), None);
    assert!(matches!(wrapper.state(), MixedResetState::Retired(_)));
    samples
}

#[test]
fn public_reset_wrapper_exposes_the_applied_release_and_returns_both_bound_owners() {
    let mut old = arm(SampleTime::new(64));
    let (_, held) = old
        .control
        .submit_note_on(&mut old.control_port, note_on(100, 48))
        .expect("old onset");
    let old_binding = old.control_port.binding;
    let mut wrapper = MixedBridgeResetAudio::new(
        MixedBridgeResetOwner::new(old.audio, old.audio_port).expect("bound old port"),
    );
    assert!(wrapper.secondary_publication_usage().is_none());
    let mut stopped = None;
    assert_eq!(
        count_allocs(|| stopped = Some(wrapper.schedule_stop(STOP))),
        0
    );
    assert!(matches!(
        stopped,
        Some(Ok(super::host::MixedBridgeStopAccepted::Scheduled))
    ));
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    for chunk in samples[..B_OUTPUT].chunks_mut(QUANTUM) {
        let allocations = count_allocs(|| wrapper.render(block(chunk)).expect("old callback"));
        assert_eq!(allocations, 0);
    }
    let (mut next, next_id) = successor_transfer();
    let next_binding = next.control_port.binding;
    let successor =
        MixedBridgeResetOwner::new(next.audio, next.audio_port).expect("bound successor port");
    let initial_successor_usage = successor.publication_usage();
    assert_eq!(
        count_allocs(|| wrapper.offer_reset(successor, B).expect("reset offer")),
        0
    );
    assert_eq!(
        wrapper.secondary_publication_usage(),
        Some(initial_successor_usage)
    );
    for chunk in samples[B_OUTPUT..].chunks_mut(QUANTUM) {
        let allocations = count_allocs(|| wrapper.render(block(chunk)).expect("reset callback"));
        assert_eq!(allocations, 0);
    }
    let MixedBridgeResetState::Retired(receipt) = wrapper.state() else {
        panic!("old owner must retire");
    };
    assert_eq!(receipt.at(), B);
    assert_eq!(receipt.old_binding(), old_binding);
    assert_eq!(receipt.successor_binding(), next_binding);
    assert_eq!(
        receipt.ended_at(),
        SampleTime::new(B.as_u64() + u64::from(QUANTUM_FRAMES))
    );
    assert!(matches!(
        receipt.ended().next(),
        Some(MixedBridgeResetEnd::Tombstoned(handle)) if handle == held
    ));
    assert_eq!(receipt.ended().count(), 1);
    assert_eq!(samples, reset_run(BEFORE[1], AFTER[1]));
    let old_usage = wrapper
        .secondary_publication_usage()
        .expect("retiring owner observations");
    let new_usage = wrapper.active_publication_usage();
    let old_resources = wrapper
        .secondary_resource_usage()
        .expect("retiring resource counters")
        .expect("representable");
    let new_resources = wrapper
        .active_resource_usage()
        .expect("active resource counters");
    for usage in [old_usage, new_usage] {
        assert!(usage.high_water_external_total().get() > 0);
    }
    let mut retired = None;
    assert_eq!(count_allocs(|| retired = wrapper.take_retired()), 0);
    let retired = retired.expect("retired old owner");
    assert_eq!(retired.receipt().at(), B);
    let old_owner = retired.into_owner();
    assert_eq!(old_owner.publication_usage(), old_usage);
    assert_eq!(
        old_owner.resource_usage().expect("old resources"),
        old_resources
    );
    assert!(wrapper.secondary_publication_usage().is_none());
    let (old_audio, old_port) = old_owner.into_parts();
    assert_eq!(old_audio.publication_usage(), old_usage);
    assert_eq!(old_port.binding, old_binding);
    let (new_owner, secondary) = wrapper.into_owners();
    assert_eq!(new_owner.publication_usage(), new_usage);
    assert_eq!(
        new_owner.resource_usage().expect("new resources"),
        new_resources
    );
    assert!(secondary.is_none());
    let (new_audio, new_port) = new_owner.into_parts();
    assert_eq!(new_port.binding, next_binding);
    let served = reports(&mut next.control, &mut next.control_port);
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].id(), next_id);
    let _old_reports = reports(&mut old.control, &mut old.control_port);
    let old_teardown = old
        .control
        .join(old_audio, Some(old.control_port), Some(old_port))
        .expect("old owner joins");
    assert_eq!(old_teardown.custody(), MixedTransferCustody::Complete);
    assert_eq!(old_teardown.tombstones().get(), 1);
    assert_eq!(old_teardown.publication_usage(), old_usage);
    assert_eq!(
        old_teardown.resource_usage().expect("old joined resources"),
        old_resources
    );
    let new_teardown = next
        .control
        .join(new_audio, Some(next.control_port), Some(new_port))
        .expect("successor joins");
    assert_eq!(new_teardown.custody(), MixedTransferCustody::Complete);
    assert_eq!(new_teardown.publication_usage(), new_usage);
    assert_eq!(
        new_teardown.resource_usage().expect("new joined resources"),
        new_resources
    );
}

#[test]
fn the_wrapper_cross_fades_identically_in_every_partition_without_the_old_ports_command() {
    let old = old_alone(false);
    let late = old_alone(true);
    let new = successor_alone();

    // Falsifier controls: the late command sounds inside the fade if the old owner serves
    // it, and the successor's onset, served only from its own port, is audible.
    assert_ne!(
        &old[B_OUTPUT..FADE_END_OUTPUT],
        &late[B_OUTPUT..FADE_END_OUTPUT]
    );
    assert!(new[10..QUANTUM].iter().any(|s| *s != 0.0));

    let mut expected = old[..B_OUTPUT].to_vec();
    for position in 0..QUANTUM {
        let gain = fade_gain(position).expect("fade frame");
        expected.push(new[position] * gain + old[B_OUTPUT + position] * (1.0 - gain));
    }
    expected.extend_from_slice(&new[QUANTUM..]);

    for before in BEFORE {
        for after in AFTER {
            assert_eq!(reset_run(before, after), expected, "{before:?} {after:?}");
        }
    }
}

#[test]
fn each_owner_keeps_its_own_port_and_the_retired_arm_joins_with_its_unserved_command() {
    let mut old = old_transfer();
    let old_binding = old.control_port.binding;
    let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render(&mut wrapper, &mut samples[..B_OUTPUT], BEFORE[1]);
    assert_eq!(reports(&mut old.control, &mut old.control_port).len(), 1);
    let (mut next, next_id) = successor_transfer();
    let next_binding = next.control_port.binding;
    offer(&mut wrapper, owner(next.audio, next.audio_port));
    let late = onset(&mut old.control, &mut old.control_port, LATE, 52);
    render(&mut wrapper, &mut samples[B_OUTPUT..], AFTER[1]);

    assert_eq!(wrapper.fault(), None);
    let MixedResetState::Retired(receipt) = wrapper.state() else {
        panic!("retired: {:?}", wrapper.state());
    };
    assert_eq!(receipt.at, B);
    {
        // The successor is active with its own port; the old owner keeps the old one.
        let pair = &wrapper.pair;
        assert_eq!(pair.active().transfer_binding, Some(next_binding));
        assert_eq!(pair.active_endpoint.port.binding, next_binding);
        let MixedResetSlot::Retired {
            old: retired_old,
            endpoint,
            ..
        } = &pair.secondary
        else {
            panic!("the retired old owner holds the secondary slot");
        };
        assert_eq!(retired_old.transfer_binding, Some(old_binding));
        assert_eq!(endpoint.port.binding, old_binding);
        // The old owner served nothing from `B`; the successor follows its own clock.
        assert_eq!(endpoint.boundary.load(Ordering::Acquire), B.as_u64());
        assert_eq!(
            pair.active_endpoint.boundary.load(Ordering::Acquire),
            pair.active().audio.renderer.clock().as_u64()
        );
    }

    // The successor served its own onset at `B`; the old arm's late command was not served.
    let served = reports(&mut next.control, &mut next.control_port);
    let [report] = served.as_slice() else {
        panic!("one successor result: {served:?}");
    };
    assert_eq!(report.id(), next_id);
    assert_eq!(report.executed_at(), SampleTime::new(B.as_u64() + 10));
    assert!(matches!(
        report.outcome(),
        MixedTransferOutcome::NoteOn { result: Ok(_), .. }
    ));
    assert!(reports(&mut old.control, &mut old.control_port).is_empty());

    let mut retired = None;
    let allocations = count_allocs(|| {
        retired = wrapper.take_retired();
    });
    assert_eq!(
        allocations, 0,
        "taking the retired owner allocated or freed"
    );
    let retired = retired.expect("retired old owner");
    assert_eq!(retired.receipt, receipt);
    assert_eq!(wrapper.state(), MixedResetState::Empty);
    assert!(wrapper.take_retired().is_none());

    let (old_audio, old_port) = retired.owner.into_parts();
    let teardown = old
        .control
        .join(old_audio, Some(old.control_port), Some(old_port))
        .expect("the old arm joins with its own port");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert_eq!(teardown.unserved(), &[late]);
    assert_eq!(teardown.audio_fault(), None);
    assert!(teardown.sounding_live().expect("registry").is_empty());
    // The held onset ended in the reset group and left its tombstone.
    assert_eq!(teardown.tombstones().get(), 1);

    let (active, secondary) = wrapper.into_owners();
    assert!(secondary.is_none());
    let (next_audio, next_port) = active.into_parts();
    let teardown = next
        .control
        .join(next_audio, Some(next.control_port), Some(next_port))
        .expect("the successor arm joins with its own port");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert!(teardown.unserved().is_empty());
    assert_eq!(teardown.audio_fault(), None);
}

#[test]
fn a_fault_of_the_fading_arm_silences_the_complete_callback_and_keeps_both_ports() {
    let mut old = old_transfer();
    let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
    let mut samples = vec![0.0_f32; B_OUTPUT + 32];
    render(&mut wrapper, &mut samples[..B_OUTPUT], BEFORE[0]);
    let _held = reports(&mut old.control, &mut old.control_port);
    let (mut next, _) = successor_transfer();
    offer(&mut wrapper, owner(next.audio, next.audio_port));
    render(&mut wrapper, &mut samples[B_OUTPUT..], &[8; 4]);
    assert!(matches!(wrapper.state(), MixedResetState::Fading { .. }));
    assert!(samples[B_OUTPUT..].iter().any(|s| *s != 0.0));

    // The old arm's control half is lent the successor's port, which halts the old arm.
    let refused = old
        .control
        .submit_note_on(&mut next.control_port, note_on(LATE, 52));
    assert!(matches!(
        refused,
        Err(MixedTransferSubmitError::Refused {
            reason: MixedTransferRefusal::ForeignEndpoint,
            ..
        })
    ));

    // The active owner renders its segment first; the fading owner's halt still silences
    // the complete callback, and every later one.
    let mut faulted = vec![0.5_f32; 16];
    assert_eq!(
        wrapper.render(block(&mut faulted)),
        Err(MixedResetRenderError::Fading(
            MixedOneShotRenderError::IngressTransferHalt
        ))
    );
    assert!(faulted.iter().all(|s| *s == 0.0), "{faulted:?}");
    faulted.fill(0.5);
    assert_eq!(
        wrapper.render(block(&mut faulted)),
        Err(MixedResetRenderError::Faulted)
    );
    assert!(faulted.iter().all(|s| *s == 0.0), "{faulted:?}");
    assert!(matches!(wrapper.state(), MixedResetState::Fading { .. }));
    assert!(wrapper.take_retired().is_none());
    assert!(wrapper.pair.active.ingress_halt.load(Ordering::Acquire));
    let MixedResetSlot::Fading { old: fading, .. } = &wrapper.pair.secondary else {
        panic!("the fading owner remains held");
    };
    assert!(fading.ingress_halt.load(Ordering::Acquire));
    assert_eq!(next.control.fault(), Some(MixedTransferFault::Remote));

    // Both owners come back with their own ports, and both arms join.
    let (active, secondary) = wrapper.into_owners();
    let (old_audio, old_port) = secondary.expect("the fading old owner").into_parts();
    let teardown = old
        .control
        .join(old_audio, Some(old.control_port), Some(old_port))
        .expect("the old arm joins with its own port");
    assert_eq!(
        teardown.control_fault(),
        Some(MixedTransferFault::ForeignEndpoint)
    );
    assert_eq!(
        teardown.audio_fault(),
        Some(MixedTransferRenderError::RemoteHalt)
    );
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    let (next_audio, next_port) = active.into_parts();
    let teardown = next
        .control
        .join(next_audio, Some(next.control_port), Some(next_port))
        .expect("the successor arm joins with its own port");
    assert_eq!(teardown.audio_fault(), None);
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
}

#[test]
fn bridge_seal_refuses_an_unserved_old_command_until_audio_serves_it() {
    let mut old = old_transfer();
    old.control.attach_reset_seal(&mut old.audio);
    let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
    let mut before = vec![0.0_f32; B_OUTPUT - QUANTUM];
    render(&mut wrapper, &mut before, &[QUANTUM; 7]);

    let (next, _) = successor_transfer();
    let successor_binding = next.control_port.binding;
    let successor = owner(next.audio, next.audio_port);
    let (reason, successor) = wrapper
        .offer_reset(successor, B)
        .expect_err("bridge reset requires a seal");
    assert_eq!(reason, MixedResetRefused::Unsealed);

    let _late = onset(&mut old.control, &mut old.control_port, B.as_u64() - 10, 52);
    assert!(old.control.seal_reset_for(successor_binding));
    let (reason, successor) = wrapper
        .offer_reset(successor, B)
        .expect_err("the sealed command has not reached audio");
    assert_eq!(reason, MixedResetRefused::IngressPending);

    let mut last = [0.0_f32; QUANTUM];
    render(&mut wrapper, &mut last, &[QUANTUM]);
    wrapper
        .offer_reset(successor, B)
        .expect("every sealed old command was served before B");
}

#[test]
fn every_refused_or_withdrawn_successor_comes_back_with_its_own_port() {
    let mut old = old_transfer();
    let (next, next_id) = successor_transfer();
    let old_binding = old.control_port.binding;
    let next_binding = next.control_port.binding;

    // An audio half and another arm's port are refused at boxing, both unchanged.
    let crossed =
        MixedResetTransferOwner::new(old.audio, next.audio_port).expect_err("crossed arms");
    let MixedResetForeignEndpoint {
        audio: old_audio,
        port: next_port,
    } = *crossed;
    assert_eq!(old_audio.inner().transfer_binding, Some(old_binding));
    assert_eq!(next_port.binding, next_binding);
    let mut wrapper = Wrapper::new(owner(old_audio, old.audio_port));
    let successor = owner(next.audio, next_port);

    // Rule 3.2: refused while the old program runs.
    let (reason, successor) = wrapper.offer_reset(successor, B).expect_err("running");
    assert_eq!(reason, MixedResetRefused::Running);
    assert_eq!(successor.endpoint.port.binding, next_binding);
    assert_eq!(successor.owner().transfer_binding, Some(next_binding));

    // Withdrawn before its boundary, with its port.
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render(&mut wrapper, &mut samples[..B_OUTPUT - QUANTUM], &[448]);
    offer(&mut wrapper, successor);
    let successor = wrapper.withdraw_pending().expect("pending successor");
    assert_eq!(successor.endpoint.port.binding, next_binding);
    assert_eq!(wrapper.state(), MixedResetState::Empty);

    // Refused at `B`: the old owner queued a command for `B + 5` in the last quantum
    // before `B`, so it still has input to apply after `B`.
    offer(&mut wrapper, successor);
    let queued = onset(&mut old.control, &mut old.control_port, B.as_u64() + 5, 52);
    render(&mut wrapper, &mut samples[B_OUTPUT - QUANTUM..], &[64, 256]);
    assert_eq!(
        wrapper.state(),
        MixedResetState::Refused {
            at: B,
            reason: MixedResetRefused::IngressPending,
        }
    );
    // The old owner served it and played on.
    let old_reports = reports(&mut old.control, &mut old.control_port);
    assert_eq!(
        old_reports.last().map(MixedTransferReport::id),
        Some(queued)
    );
    let refused = wrapper.take_refused().expect("refused successor");
    assert_eq!(refused.at, B);
    assert_eq!(refused.reason, MixedResetRefused::IngressPending);
    assert!(!refused.owner.owner().render_started);
    assert_eq!(refused.owner.endpoint.port.binding, next_binding);
    assert_eq!(wrapper.state(), MixedResetState::Empty);

    // The refused arm joins with its own port; its onset was never served.
    let (next_audio, next_port) = refused.owner.into_parts();
    let teardown = next
        .control
        .join(next_audio, Some(next.control_port), Some(next_port))
        .expect("the refused arm joins with its own port");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert_eq!(teardown.unserved(), &[next_id]);

    let (active, secondary) = wrapper.into_owners();
    assert!(secondary.is_none());
    let (old_audio, old_port) = active.into_parts();
    let teardown = old
        .control
        .join(old_audio, Some(old.control_port), Some(old_port))
        .expect("the old arm joins with its own port");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert!(teardown.unserved().is_empty());
}

#[test]
fn reset_seal_admits_exactly_one_of_panic_cancel_and_audio_acquire() {
    use super::super::{
        MixedIngressCommandId,
        transfer::{
            MixedResetSeal, MixedResetSealDecision, MixedResetSealPhase, MixedResetSealState,
        },
    };
    let named = arm(B).control_port.binding;
    let other = arm(B).control_port.binding;
    let seal = MixedResetSeal::new();
    assert!(!seal.cancel(), "an open seal cannot be cancelled");
    assert_eq!(
        seal.reset_seal_decide(Some(named), true),
        MixedResetSealDecision::NotSealed
    );
    assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Open);

    // The Panic first: audio then finds the cancel and changes nothing.
    seal.publish(named, 0);
    for _ in 0..2 {
        assert_eq!(
            seal.reset_seal_state(Some(named), None),
            MixedResetSealState::Ready
        );
        assert_eq!(
            seal.phase_for_test(),
            MixedResetSealPhase::Sealed,
            "an offer-time read changes nothing"
        );
    }
    assert!(seal.cancel());
    assert!(!seal.cancel(), "a second cancel loses");
    assert_eq!(
        seal.reset_seal_state(Some(named), None),
        MixedResetSealState::Cancelled
    );
    assert_eq!(
        seal.reset_seal_state(Some(other), None),
        MixedResetSealState::Unsealed
    );
    for take in [true, false] {
        assert_eq!(
            seal.reset_seal_decide(Some(named), take),
            MixedResetSealDecision::Cancelled
        );
    }
    assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Cancelled);
    seal.clear();
    assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Open);

    // Audio first: it acquires, and a later cancel loses. A foreign successor decides
    // nothing.
    seal.publish(named, 0);
    assert_eq!(
        seal.reset_seal_decide(Some(other), true),
        MixedResetSealDecision::NotSealed
    );
    assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Sealed);
    assert_eq!(
        seal.reset_seal_decide(Some(named), true),
        MixedResetSealDecision::Decided
    );
    assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Acquired);
    assert!(!seal.cancel());
    assert_eq!(
        seal.reset_seal_decide(Some(named), true),
        MixedResetSealDecision::NotSealed,
        "the decision is spent"
    );
    assert_eq!(
        seal.reset_seal_state(Some(named), None),
        MixedResetSealState::Unsealed
    );
    // `end_live` refused after the acquire.
    seal.reset_seal_decline();
    assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Declined);
    assert!(!seal.cancel());
    seal.clear();

    // A refusal at `B` declines; a later cancel loses and abandon reopens.
    seal.publish(named, 3);
    assert_eq!(
        seal.reset_seal_state(Some(named), Some(MixedIngressCommandId(2))),
        MixedResetSealState::IngressPending
    );
    assert_eq!(
        seal.reset_seal_decide(Some(named), false),
        MixedResetSealDecision::Decided
    );
    assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Declined);
    assert!(!seal.cancel());
    assert_eq!(
        seal.reset_seal_state(Some(named), Some(MixedIngressCommandId(3))),
        MixedResetSealState::Unsealed,
        "a declined seal cannot be offered again"
    );
    seal.clear();
    assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Open);

    // Both threads race on one fresh seal: exactly one compare-and-swap wins, and the
    // loser observes the winner's phase.
    let barrier = std::sync::Barrier::new(2);
    let (mut cancels, mut acquires) = (0_u32, 0_u32);
    for _ in 0..2_000 {
        seal.publish(named, 0);
        let (cancelled, decided) = std::thread::scope(|scope| {
            let control = scope.spawn(|| {
                barrier.wait();
                seal.cancel()
            });
            barrier.wait();
            let decided = seal.reset_seal_decide(Some(named), true);
            (control.join().expect("control thread"), decided)
        });
        match (cancelled, decided) {
            (true, MixedResetSealDecision::Cancelled) => {
                cancels += 1;
                assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Cancelled);
            }
            (false, MixedResetSealDecision::Decided) => {
                acquires += 1;
                assert_eq!(seal.phase_for_test(), MixedResetSealPhase::Acquired);
            }
            other => panic!("both or neither won: {other:?}"),
        }
        seal.clear();
    }
    assert_eq!(cancels + acquires, 2_000);
}

/// A cancelled seal refuses the successor at the offer and at `B`, ahead of the
/// cancelling Panic's own unserved command, and a seal audio already declined or
/// acquired cannot be cancelled.
#[test]
fn a_cancelled_seal_refuses_at_offer_and_at_b_before_pending_ingress() {
    for offered_first in [false, true] {
        let mut old = old_transfer();
        old.control.attach_reset_seal(&mut old.audio);
        let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
        let mut samples = vec![0.0_f32; B_OUTPUT + QUANTUM];
        render(
            &mut wrapper,
            &mut samples[..B_OUTPUT - QUANTUM],
            &[QUANTUM; 7],
        );
        let (next, _) = successor_transfer();
        assert!(old.control.seal_reset_for(next.control_port.binding));
        let mut successor = Some(owner(next.audio, next.audio_port));
        if offered_first {
            offer(&mut wrapper, successor.take().expect("successor"));
        }
        // The cancel wins; a command for the old owner after `B` is then unserved at `B`.
        assert!(old.control.cancel_reset_seal());
        let _after = onset(&mut old.control, &mut old.control_port, B.as_u64() + 5, 52);
        if let Some(successor) = successor.take() {
            let (reason, successor) = wrapper
                .offer_reset(successor, B)
                .expect_err("a cancelled seal");
            assert_eq!(reason, MixedResetRefused::PanicCancelled);
            assert!(!successor.owner().render_started);
            continue;
        }
        render(
            &mut wrapper,
            &mut samples[B_OUTPUT - QUANTUM..],
            &[QUANTUM, 8, 56],
        );
        assert_eq!(
            wrapper.state(),
            MixedResetState::Refused {
                at: B,
                reason: MixedResetRefused::PanicCancelled,
            },
            "the cancel is reported, not the old owner's pending input"
        );
        assert!(!old.control.cancel_reset_seal(), "already cancelled");
        let refused = wrapper.take_refused().expect("refused successor");
        assert!(!refused.owner.owner().render_started);
    }

    // Audio refuses at `B` for another reason first: a command minted after the seal is
    // still queued for `B + 5`. Audio declines the seal, and a later cancel loses.
    let mut old = old_transfer();
    old.control.attach_reset_seal(&mut old.audio);
    let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
    let mut samples = vec![0.0_f32; B_OUTPUT + QUANTUM];
    render(
        &mut wrapper,
        &mut samples[..B_OUTPUT - QUANTUM],
        &[QUANTUM; 7],
    );
    let (next, _) = successor_transfer();
    assert!(old.control.seal_reset_for(next.control_port.binding));
    offer(&mut wrapper, owner(next.audio, next.audio_port));
    let _after = onset(&mut old.control, &mut old.control_port, B.as_u64() + 5, 52);
    render(
        &mut wrapper,
        &mut samples[B_OUTPUT - QUANTUM..],
        &[QUANTUM, 8, 56],
    );
    assert_eq!(
        wrapper.state(),
        MixedResetState::Refused {
            at: B,
            reason: MixedResetRefused::IngressPending,
        }
    );
    assert!(
        !old.control.cancel_reset_seal(),
        "audio declined the seal at B"
    );

    // Audio decides first: the reset applies and a later cancel loses.
    let mut old = old_transfer();
    old.control.attach_reset_seal(&mut old.audio);
    let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
    let mut samples = vec![0.0_f32; B_OUTPUT + QUANTUM];
    render(
        &mut wrapper,
        &mut samples[..B_OUTPUT - QUANTUM],
        &[QUANTUM; 7],
    );
    let (next, _) = successor_transfer();
    assert!(old.control.seal_reset_for(next.control_port.binding));
    offer(&mut wrapper, owner(next.audio, next.audio_port));
    render(
        &mut wrapper,
        &mut samples[B_OUTPUT - QUANTUM..],
        &[QUANTUM, 8, 56],
    );
    assert!(matches!(wrapper.state(), MixedResetState::Retired(_)));
    assert!(
        !old.control.cancel_reset_seal(),
        "audio acquired the seal at B"
    );
}

// ---------------------------------------------------------------------------------------
// Later activations through the public wrapper.

type Public = MixedBridgeResetAudio<Cons<MixedTransferCommand>, Prod<MixedTransferResult>>;

fn public(audio: MixedTransferAudio, port: AudioPort) -> Public {
    MixedBridgeResetAudio::new(MixedBridgeResetOwner::new(audio, port).expect("own port"))
}

fn public_owner(
    armed: Armed,
) -> (
    MixedBridgeResetOwner<Cons<MixedTransferCommand>, Prod<MixedTransferResult>>,
    MixedTransferControl,
    ControlPort,
) {
    let owner = MixedBridgeResetOwner::new(armed.audio, armed.audio_port).expect("own port");
    (owner, armed.control, armed.control_port)
}

/// Render through the public wrapper over `partition`, allocating and freeing nothing.
fn render_public(wrapper: &mut Public, samples: &mut [f32], partition: &[usize]) {
    assert_eq!(partition.iter().sum::<usize>(), samples.len());
    let mut offset = 0;
    for &frames in partition {
        let output = block(&mut samples[offset..offset + frames]);
        let allocations = count_allocs(|| {
            wrapper.render(output).expect("wrapper callback");
        });
        assert_eq!(allocations, 0, "wrapper callback allocated or freed");
        offset += frames;
    }
}

fn prepare(
    control: &MixedTransferControl,
    at: u64,
    position: u64,
    kind: MixedActivationKind,
) -> MixedActivationOffer {
    control
        .prepare_activation(
            SampleTime::new(at),
            PlanPosition::new(position),
            SampleTime::new(at),
            kind,
        )
        .expect("the candidate prepares")
}

/// Offer through the public wrapper, allocating and freeing nothing.
fn offer_activation(
    wrapper: &mut Public,
    offer: MixedActivationOffer,
) -> Result<(), (MixedActivationRefusal, MixedActivationOffer)> {
    let mut offered = None;
    assert_eq!(
        count_allocs(|| offered = Some(wrapper.offer_activation(offer))),
        0,
        "the offer allocated or freed"
    );
    offered.expect("offered")
}

/// The wrapper forwards a later seek to its active owner through the owner's binding
/// check. An adopted receipt left in the old owner's slot refuses the reset until the host
/// takes it, so it never fades out with an owner whose control no longer collects it. A
/// pending reset then refuses a Start whole, and once the successor is withdrawn the same
/// Start is accepted and in turn refuses the reset.
#[test]
fn the_wrapper_forwards_activation_and_a_reset_waits_for_an_empty_old_slot() {
    let initial = ActivationSequence::INITIAL.next().expect("armed sequence");
    let seek_sequence = initial.next().expect("seek sequence");
    let mut old = arm(SampleTime::new(64));
    let mut wrapper = public(old.audio, old.audio_port);
    let mut samples = vec![0.0_f32; B_OUTPUT - QUANTUM];
    render_public(&mut wrapper, &mut samples[..192], &[QUANTUM; 3]);

    let (next, _) = successor_transfer();
    let foreign = prepare(&next.control, 512, 10, MixedActivationKind::Seek);
    let (reason, foreign) = offer_activation(&mut wrapper, foreign).expect_err("foreign");
    assert_eq!(reason, MixedActivationRefusal::ForeignTransfer);
    assert_eq!(foreign.effective(), SampleTime::new(512), "returned whole");
    let seek = prepare(&old.control, 256, 10, MixedActivationKind::Seek);
    assert_eq!(seek.supersedes(), initial);
    assert_eq!(
        foreign.supersedes(),
        initial,
        "the same superseded sequence"
    );
    offer_activation(&mut wrapper, seek).expect("the active owner takes its own seek");
    assert_eq!(
        wrapper.activation(),
        MixedActivationSlot::Pending {
            supersedes: initial,
            effective: SampleTime::new(256),
            kind: MixedActivationKind::Seek,
            loop_interval: None,
        }
    );
    assert_eq!(
        wrapper.schedule_stop(SampleTime::new(320)),
        Ok(super::host::MixedBridgeStopAccepted::Scheduled)
    );
    render_public(&mut wrapper, &mut samples[192..], &[QUANTUM; 4]);
    let MixedActivationSlot::Retired(receipt) = wrapper.activation() else {
        panic!("the seek retired: {:?}", wrapper.activation());
    };
    let MixedActivationOutcome::Adopted(adopted) = receipt.outcome() else {
        panic!("the seek adopted: {receipt:?}");
    };
    assert_eq!(adopted.effective(), SampleTime::new(256));
    assert_eq!(receipt.sequence(), seek_sequence);

    // Rule 3.2 holds, but the retired seek still waits in the old slot.
    let (successor, next_control, next_port) = public_owner(next);
    let (reason, successor) = wrapper
        .offer_reset(successor, B)
        .expect_err("an uncollected receipt refuses the reset");
    assert_eq!(reason, MixedBridgeResetRefusal::ActivationUncollected);
    assert_eq!(wrapper.state(), MixedBridgeResetState::Empty);
    let mut retired = None;
    assert_eq!(
        count_allocs(|| retired = wrapper.take_retired_activation()),
        0
    );
    assert_eq!(wrapper.activation(), MixedActivationSlot::Empty);
    let collected = old
        .control
        .collect_activation(retired.expect("retired seek"))
        .expect("its own control collects it");
    assert_eq!(collected, receipt);
    assert!(wrapper.take_retired_activation().is_none());

    // The reset is accepted now. While it is pending, a Start is refused whole.
    assert_eq!(
        count_allocs(|| wrapper.offer_reset(successor, B).expect("reset offer")),
        0
    );
    let start = prepare(&old.control, 512, 10, MixedActivationKind::Start);
    assert_eq!(start.supersedes(), seek_sequence);
    let (reason, start) = offer_activation(&mut wrapper, start).expect_err("reset pending");
    assert_eq!(reason, MixedActivationRefusal::ResetPending { at: B });
    assert_eq!(
        (start.effective(), start.kind(), start.supersedes()),
        (
            SampleTime::new(512),
            MixedActivationKind::Start,
            seek_sequence
        ),
        "the Start came back whole"
    );
    assert_eq!(wrapper.activation(), MixedActivationSlot::Empty);
    assert_eq!(wrapper.state(), MixedBridgeResetState::Pending { at: B });

    // Withdrawn, the successor no longer refuses the Start, which then refuses it.
    let successor = wrapper.withdraw_pending().expect("pending successor");
    offer_activation(&mut wrapper, start).expect("the same Start is accepted");
    let (reason, successor) = wrapper
        .offer_reset(successor, B)
        .expect_err("a pending Start refuses the reset");
    assert_eq!(reason, MixedBridgeResetRefusal::OfferPending);

    let (active, secondary) = wrapper.into_owners();
    assert!(secondary.is_none());
    let (old_audio, old_audio_port) = active.into_parts();
    let teardown = old
        .control
        .join(old_audio, Some(old.control_port), Some(old_audio_port))
        .expect("the old arm joins");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert!(matches!(
        teardown.activation(),
        MixedActivationSlot::Pending {
            kind: MixedActivationKind::Start,
            ..
        }
    ));
    let (next_audio, next_audio_port) = successor.into_parts();
    let teardown = next_control
        .join(next_audio, Some(next_port), Some(next_audio_port))
        .expect("the withdrawn successor joins");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert_eq!(teardown.activation(), MixedActivationSlot::Empty);
    drop(foreign);
}

/// One stopped reset through the public wrapper, with or without a Start the old control
/// prepared for after `B` offered while the reset is pending. Returns the output and
/// the offer the pending reset refused.
fn pending_probe_run(probe: bool) -> (Vec<f32>, Option<MixedActivationOffer>) {
    let mut old = old_transfer();
    let mut wrapper = public(old.audio, old.audio_port);
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render_public(&mut wrapper, &mut samples[..B_OUTPUT], BEFORE[1]);
    let (next, _) = successor_transfer();
    let next_binding = next.control_port.binding;
    let (successor, _next_control, _next_port) = public_owner(next);
    wrapper.offer_reset(successor, B).expect("reset offer");
    let refused = probe.then(|| {
        let start = prepare(&old.control, 512, 10, MixedActivationKind::Start);
        let (reason, start) = offer_activation(&mut wrapper, start).expect_err("reset pending");
        assert_eq!(reason, MixedActivationRefusal::ResetPending { at: B });
        start
    });
    render_public(&mut wrapper, &mut samples[B_OUTPUT..], AFTER[3]);
    let MixedBridgeResetState::Retired(receipt) = wrapper.state() else {
        panic!("the reset applied: {:?}", wrapper.state());
    };
    assert_eq!(receipt.successor_binding(), next_binding);
    let refused = refused.map(|start| {
        // After `B` the old control's Start is foreign to the successor's transfer.
        let (reason, start) = offer_activation(&mut wrapper, start).expect_err("foreign");
        assert_eq!(reason, MixedActivationRefusal::ForeignTransfer);
        start
    });
    assert_eq!(wrapper.activation(), MixedActivationSlot::Empty);
    let _old_reports = reports(&mut old.control, &mut old.control_port);
    (samples, refused)
}

#[test]
fn a_pending_reset_returns_an_activation_whole_and_changes_no_sample() {
    let (reference, none) = pending_probe_run(false);
    assert!(none.is_none());
    let (probed, refused) = pending_probe_run(true);
    let refused = refused.expect("the refused Start");
    assert_eq!(
        (refused.effective(), refused.kind()),
        (SampleTime::new(512), MixedActivationKind::Start),
        "the Start came back whole twice"
    );
    assert_eq!(probed, reference, "the refused offers changed the output");
}

#[test]
fn a_faulted_wrapper_refuses_an_activation_whole() {
    let mut old = arm(SampleTime::new(64));
    let offer = prepare(&old.control, 256, 10, MixedActivationKind::Seek);
    let (mut next, _) = successor_transfer();
    let mut wrapper = public(old.audio, old.audio_port);
    let mut samples = vec![0.0_f32; QUANTUM];
    render_public(&mut wrapper, &mut samples, &[QUANTUM]);
    // The old control is lent another arm's port, which halts the old arm.
    assert!(
        old.control
            .submit_note_on(&mut next.control_port, note_on(150, 52))
            .is_err()
    );
    assert!(wrapper.render(block(&mut samples)).is_err());
    assert!(wrapper.fault().is_some());
    let (reason, offer) = offer_activation(&mut wrapper, offer).expect_err("faulted");
    assert_eq!(reason, MixedActivationRefusal::Faulted);
    assert_eq!(offer.effective(), SampleTime::new(256), "returned whole");
    assert_eq!(wrapper.activation(), MixedActivationSlot::Empty);
    assert!(wrapper.take_retired_activation().is_none());
}

#[test]
fn reset_release_queue_peak_is_retained_after_the_discarded_quantum_drains_it() {
    let mut old = old_transfer();
    let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
    let mut before = [0.0; B_OUTPUT];
    render(&mut wrapper, &mut before, BEFORE[1]);
    assert_eq!(wrapper.pair.active().test_ingress.len(), 0);
    // Isolate the reset's contribution from the earlier onset's already measured peak.
    wrapper.pair.active_mut().ingress_queue_peak = 0;
    let next = arm(B);
    offer(&mut wrapper, owner(next.audio, next.audio_port));
    let mut after = [0.0; QUANTUM * 2];
    render(&mut wrapper, &mut after, &[QUANTUM; 2]);
    let MixedResetSlot::Retired { old: old_audio, .. } = &wrapper.pair.secondary else {
        panic!("retired old owner");
    };
    assert_eq!(old_audio.test_ingress.len(), 0);
    assert_eq!(old_audio.test_ingress.holds_outstanding(), EventCount::NONE);
    let usage = super::super::transfer::MixedTransferResourceUsage::of(old_audio)
        .expect("reset resource counters");
    assert_eq!(usage.ingress_queue_high_water, EventCount::measured(1));
    assert_eq!(usage.live_holds_high_water, EventCount::measured(1));
    let retired = wrapper.take_retired().expect("retired");
    let (old_audio, old_port) = retired.owner.into_parts();
    let _reports = reports(&mut old.control, &mut old.control_port);
    let old_join = old
        .control
        .join(old_audio, Some(old.control_port), Some(old_port))
        .expect("old joins");
    assert_eq!(old_join.resource_usage().expect("retained"), usage);
    let (active, secondary) = wrapper.into_owners();
    assert!(secondary.is_none());
    let (audio, port) = active.into_parts();
    let joined = next
        .control
        .join(audio, Some(next.control_port), Some(port))
        .expect("successor joins");
    assert_eq!(joined.custody(), MixedTransferCustody::Complete);
}

#[path = "running_tests.rs"]
mod running_tests;
