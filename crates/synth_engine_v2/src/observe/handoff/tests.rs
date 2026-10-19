use super::*;
use crate::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    profile::HostProfile,
    quantities::{ChannelLayout, SampleRate},
    render_allocation::count_allocs,
    time::{FrameCount, QUANTUM_FRAMES},
};

const TAP: NodeId = NodeId::new(8);
const EXTRA: NodeId = NodeId::new(2);
const Q: usize = QUANTUM_FRAMES as usize;
const BUDGET: PreparedBytes = PreparedBytes::measured(4096);

fn profile(layout: ChannelLayout) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(256),
        layout,
    )
    .expect("profile")
}

fn plan(extra: bool, keep: bool, layout: ChannelLayout) -> CompiledPlan {
    let mut graph = GraphIr::builder()
        .node(NodeId::new(1), IrNodeKind::Silence, ExecutionScope::Global)
        .node(NodeId::new(9), IrNodeKind::Output, ExecutionScope::Global);
    if extra {
        graph = graph.node(EXTRA, IrNodeKind::Monitor, ExecutionScope::Global);
    }
    if keep {
        graph = graph
            .node(TAP, IrNodeKind::Monitor, ExecutionScope::Global)
            .connect(
                (NodeId::new(1), PortId::FIRST),
                (TAP, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (TAP, PortId::FIRST),
                (NodeId::new(9), PortId::FIRST),
                SignalDomain::Audio,
            );
    } else {
        graph = graph.connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(9), PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    compile(
        &graph.build().expect("graph"),
        &RenderConfig::new(profile(layout)),
    )
    .into_plan()
    .expect("plan")
}

#[test]
fn reordered_taps_keep_the_handle_and_never_mix_old_and_new_plan_frames() {
    let old = plan(false, true, ChannelLayout::Mono);
    let new = plan(true, true, ChannelLayout::Mono);
    let old_tap = old.resolve_tap(TAP, PortId::FIRST).expect("old tap");
    let new_tap = new.resolve_tap(TAP, PortId::FIRST).expect("new tap");
    assert_ne!(
        old_tap.index(),
        new_tap.index(),
        "the fixture must move the tap index"
    );
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &old).expect("store");
    let handle = store.subscribe(&old, old_tap).expect("subscribe");
    let old_buffers = vec![0.25; old.arena_samples()];
    let mut new_buffers = vec![-1.0; new.arena_samples()];
    let region = new
        .region(new.taps()[new_tap.index()].region)
        .expect("region");
    new_buffers[region.offset()..region.end()].fill(0.75);
    store.push_quantum(old.id(), old.taps(), old.regions(), &old_buffers);
    let handoff = store.prepare_handoff(&old, &new, BUDGET).expect("prepare");
    assert_eq!(
        count_allocs(|| store.handoff(&handoff).expect("handoff")),
        0
    );
    // A late outgoing callback cannot append stale samples to the new binding.
    store.push_quantum(old.id(), old.taps(), old.regions(), &old_buffers);
    store.push_quantum(new.id(), new.taps(), new.regions(), &new_buffers);
    let mut samples = [0.0; Q * 2];
    let read = store.read(handle, &mut samples).expect("same handle");
    assert_eq!(read.plan, new.id());
    assert!(!read.ended);
    assert_eq!(read.dropped, FrameCount::QUANTUM);
    assert_eq!(read.frames, FrameCount::QUANTUM);
    assert_eq!(&samples[..Q], &[0.75; Q]);
    assert_eq!(
        store.handoff(&handoff),
        Err(ObservationHandoffError::Origin)
    );
}

#[test]
fn a_removed_tap_ends_and_same_id_recreation_does_not_revive_the_old_handle() {
    let old = plan(false, true, ChannelLayout::Mono);
    let removed = plan(true, false, ChannelLayout::Mono);
    let recreated = plan(true, true, ChannelLayout::Mono);
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &old).expect("store");
    let handle = store
        .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).expect("tap"))
        .expect("subscribe");
    let remove = store
        .prepare_handoff(&old, &removed, BUDGET)
        .expect("prepare removal");
    store.handoff(&remove).expect("remove");
    let recreate = store
        .prepare_handoff(&removed, &recreated, BUDGET)
        .expect("prepare recreation");
    store.handoff(&recreate).expect("recreate");
    let buffers = vec![0.5; recreated.arena_samples()];
    store.push_quantum(
        recreated.id(),
        recreated.taps(),
        recreated.regions(),
        &buffers,
    );
    let read = store
        .read(handle, &mut [0.0; Q])
        .expect("old handle retained");
    assert!(read.ended);
    assert_eq!(read.frames, FrameCount::ZERO);
    let fresh = store
        .subscribe(
            &recreated,
            recreated.resolve_tap(TAP, PortId::FIRST).expect("new tap"),
        )
        .expect("fresh subscription");
    assert_ne!(handle, fresh);
}

#[test]
fn foreign_changed_and_underbudget_handoffs_leave_every_binding_untouched() {
    let old = plan(true, true, ChannelLayout::Mono);
    let new = plan(true, true, ChannelLayout::Mono);
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &old).expect("store");
    let first = store
        .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).expect("tap"))
        .expect("subscribe");
    assert!(matches!(
        store.prepare_handoff(&old, &new, PreparedBytes::measured(0)),
        Err(ObservationHandoffError::Bytes)
    ));
    let prepared = store.prepare_handoff(&old, &new, BUDGET).expect("prepare");
    let mut foreign =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &old).expect("store");
    assert_eq!(
        foreign.handoff(&prepared),
        Err(ObservationHandoffError::Origin)
    );
    let second = store
        .subscribe(&old, old.resolve_tap(EXTRA, PortId::FIRST).expect("extra"))
        .expect("second subscription");
    assert_eq!(
        store.handoff(&prepared),
        Err(ObservationHandoffError::Changed)
    );
    assert_eq!(store.plan(), old.id());
    assert_eq!(foreign.plan(), old.id());
    assert!(store.read(first, &mut [0.0; Q]).is_some());
    assert!(store.read(second, &mut [0.0; Q]).is_some());
}

#[test]
fn cycling_back_to_a_plan_cannot_revive_a_stale_handoff() {
    let old = plan(false, true, ChannelLayout::Mono);
    let new = plan(true, true, ChannelLayout::Mono);
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &old).expect("store");
    let handle = store
        .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).expect("tap"))
        .expect("subscription");
    let first = store.prepare_handoff(&old, &new, BUDGET).expect("first");
    store.handoff(&first).expect("install first");
    let back = store.prepare_handoff(&new, &old, BUDGET).expect("back");
    store.handoff(&back).expect("install back");
    assert_eq!(store.handoff(&first), Err(ObservationHandoffError::Origin));
    let read = store.read(handle, &mut [0.0; Q]).expect("handle");
    assert_eq!(read.plan, old.id());
    assert_eq!(read.generation.as_u64(), 2);
    store.generation = super::super::ObservationGeneration(u64::MAX);
    assert!(matches!(
        store.prepare_handoff(&old, &new, BUDGET),
        Err(ObservationHandoffError::Exhausted)
    ));
    assert_eq!(store.plan(), old.id());
}

#[test]
fn ending_a_store_retains_its_labelled_tail_without_appending_or_freeing() {
    let plan = plan(false, true, ChannelLayout::Mono);
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &plan).expect("store");
    let handle = store
        .subscribe(&plan, plan.resolve_tap(TAP, PortId::FIRST).expect("tap"))
        .expect("subscribe");
    let buffers = vec![0.5; plan.arena_samples()];
    store.push_quantum(plan.id(), plan.taps(), plan.regions(), &buffers);
    assert_eq!(count_allocs(|| store.end()), 0);
    store.push_quantum(plan.id(), plan.taps(), plan.regions(), &buffers);
    let mut tail = [0.0; Q * 2];
    let read = store.read(handle, &mut tail).expect("tail");
    assert_eq!(read.plan, plan.id());
    assert!(read.ended);
    assert_eq!(read.frames, FrameCount::QUANTUM);
    assert_eq!(read.dropped, FrameCount::ZERO);
    assert_eq!(&tail[..Q], &[0.5; Q]);
    assert_eq!(
        store.read(handle, &mut tail).expect("ended").frames,
        FrameCount::ZERO
    );
}

#[test]
fn cold_configuration_chains_without_borrowing_the_audio_owned_store() {
    let old = plan(false, true, ChannelLayout::Mono);
    let new = plan(true, true, ChannelLayout::Mono);
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &old).expect("store");
    let handle = store
        .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).expect("tap"))
        .expect("subscribe");
    let config = store.configuration(BUDGET).expect("cold mirror");
    let first = config.prepare_handoff(&old, &new, BUDGET).expect("first");
    let next = first
        .successor_configuration(BUDGET)
        .expect("next cold mirror");
    let back = next.prepare_handoff(&new, &old, BUDGET).expect("back");
    let (mut store, first, back) = std::thread::spawn(move || {
        assert_eq!(
            count_allocs(|| {
                store.handoff(&first).expect("first install");
                store.handoff(&back).expect("back install");
            }),
            0
        );
        (store, first, back)
    })
    .join()
    .expect("audio joined");
    assert_eq!(store.handoff(&first), Err(ObservationHandoffError::Origin));
    assert_eq!(store.handoff(&back), Err(ObservationHandoffError::Origin));
    let read = store.read(handle, &mut []).expect("handle");
    assert_eq!(read.plan, old.id());
    assert_eq!(read.generation.as_u64(), 2);
}

#[test]
fn same_plan_reset_advances_generation_and_counts_every_unread_old_frame() {
    let plan = plan(false, true, ChannelLayout::Mono);
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &plan).expect("store");
    let handle = store
        .subscribe(&plan, plan.resolve_tap(TAP, PortId::FIRST).expect("tap"))
        .expect("subscribe");
    let prepared = store
        .configuration(BUDGET)
        .expect("mirror")
        .prepare_handoff(&plan, &plan, BUDGET)
        .expect("same plan map");
    let old = vec![0.25; plan.arena_samples()];
    store.push_quantum(plan.id(), plan.taps(), plan.regions(), &old);
    assert_eq!(
        count_allocs(|| store.handoff(&prepared).expect("reset boundary")),
        0
    );
    let new = vec![0.75; plan.arena_samples()];
    store.push_quantum(plan.id(), plan.taps(), plan.regions(), &new);
    let mut samples = [0.0; Q];
    let read = store.read(handle, &mut samples).expect("read");
    assert_eq!(read.plan, plan.id());
    assert_eq!(read.generation.as_u64(), 1);
    assert_eq!(read.dropped, FrameCount::QUANTUM);
    assert_eq!(samples, [0.75; Q]);
    assert_eq!(
        store.handoff(&prepared),
        Err(ObservationHandoffError::Origin)
    );
}

#[test]
fn a_cold_map_checks_channel_capacity_and_actual_ring_geometry_before_mutation() {
    let old = plan(false, true, ChannelLayout::Mono);
    let new = plan(true, true, ChannelLayout::Mono);
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &old).expect("store");
    let _handle = store
        .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).expect("tap"))
        .expect("subscribe");
    assert!(matches!(
        store.configuration(PreparedBytes::measured(0)),
        Err(ObservationHandoffError::Bytes)
    ));
    let config = store.configuration(BUDGET).expect("mirror");
    let prepared = config.prepare_handoff(&old, &new, BUDGET).expect("map");
    assert!(matches!(
        prepared.successor_configuration(PreparedBytes::measured(0)),
        Err(ObservationHandoffError::Bytes)
    ));
    let channels = store.subscriptions[0].channels;
    store.subscriptions[0].channels += 1;
    assert_eq!(
        store.handoff(&prepared),
        Err(ObservationHandoffError::Changed)
    );
    store.subscriptions[0].channels = channels;
    let capacity = store.subscriptions[0].capacity_frames;
    store.subscriptions[0].capacity_frames -= 1;
    assert_eq!(
        store.handoff(&prepared),
        Err(ObservationHandoffError::Changed)
    );
    store.subscriptions[0].capacity_frames = capacity;
    let last = store.subscriptions[0].ring.pop().expect("one sample");
    assert_eq!(
        store.handoff(&prepared),
        Err(ObservationHandoffError::Geometry)
    );
    store.subscriptions[0].ring.push(last);
    assert_eq!(store.plan(), old.id());
    assert_eq!(store.generation.as_u64(), 0);
    store
        .handoff(&prepared)
        .expect("unchanged after failed checks");
}

#[test]
fn bounded_read_preserves_the_unread_tail_and_charges_actual_storage() {
    let plan = plan(false, true, ChannelLayout::Mono);
    let tap = plan.resolve_tap(TAP, PortId::FIRST).unwrap();
    let mut store =
        ObservationSubscriptions::prepare(&profile(ChannelLayout::Mono), &plan).unwrap();
    let empty_bytes = store.prepared_bytes().unwrap();
    let handle = store.subscribe(&plan, tap).unwrap();
    assert!(store.is_pristine());
    assert!(store.prepared_bytes().unwrap().get() > empty_bytes.get());
    let buffers = vec![0.25; plan.arena_samples()];
    store.push_quantum(plan.id(), plan.taps(), plan.regions(), &buffers);
    assert!(!store.is_pristine());
    let mut into = [7.0; Q];
    let mut read = None;
    assert_eq!(
        count_allocs(|| {
            read = store.read_bounded(handle, &mut into, FrameCount::new(3));
        }),
        0
    );
    let read = read.unwrap();
    assert_eq!(read.frames, FrameCount::new(3));
    assert_eq!(read.behind, FrameCount::new(u64::from(QUANTUM_FRAMES - 3)));
    assert_eq!(&into[..3], &[0.25; 3]);
    assert!(into[3..].iter().all(|sample| *sample == 7.0));
    let tail = store.read(handle, &mut into).unwrap();
    assert_eq!(tail.frames, FrameCount::new(u64::from(QUANTUM_FRAMES - 3)));
    store.end();
    assert!(!store.is_pristine());
}
