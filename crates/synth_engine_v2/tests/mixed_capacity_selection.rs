//! Arithmetic controls for the draft EVD-0026 selector. These fixed examples are not
//! measurements, workload bounds, selected profiles or production qualification.
#![cfg(feature = "mixed-transfer-rehearsal")]

#[path = "support/capacity_selection.rs"]
pub(crate) mod capacity_selection;

use capacity_selection::{QueueDemand, SelectionError, SelectionInput, select, select_queue};
use synth_engine_v2::{
    publish::ProducerClass,
    quantities::{EventCount, HeldNoteCount, QuantumCount},
    stream::MixedTransferPlaces,
};

#[test]
fn candidate_envelope_refuses_mono_even_for_a_mono_compatible_plan() {
    use synth_engine_v2::{
        compile::{RenderConfig, compile},
        ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId},
        quantities::{ChannelLayout, SampleRate},
        time::FrameCount,
    };
    let rate = SampleRate::new(48_000.0).expect("rate");
    let block = FrameCount::new(64);
    let mono = synth_engine_v2::profile::HostProfile::harness(rate, block, ChannelLayout::Mono)
        .expect("general mono profile");
    let graph = GraphIr::builder()
        .node(NodeId::new(1), IrNodeKind::Output, ExecutionScope::Global)
        .build()
        .expect("mono-compatible graph");
    let _mono_plan = compile(&graph, &RenderConfig::new(mono))
        .into_plan()
        .expect("the general core accepts this mono plan");
    assert!(matches!(
        capacity_selection::candidate_profile(rate, block, ChannelLayout::Mono),
        Err(capacity_selection::CandidateRefused::Layout(
            ChannelLayout::Mono
        ))
    ));
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let _profile = capacity_selection::candidate_profile(
            SampleRate::new(hz).expect("rate"),
            block,
            ChannelLayout::Stereo,
        )
        .expect("admitted rate");
    }
    assert!(matches!(
        capacity_selection::candidate_profile(
            SampleRate::new(32_000.0).expect("core rate"),
            block,
            ChannelLayout::Stereo
        ),
        Err(capacity_selection::CandidateRefused::Rate(_))
    ));
    for size in [63, 65] {
        assert!(matches!(
            capacity_selection::candidate_profile(
                rate,
                FrameCount::new(size),
                ChannelLayout::Stereo
            ),
            Err(capacity_selection::CandidateRefused::Block(_))
        ));
    }
}

fn queue(observed: usize, ordinary: usize, protected: usize) -> QueueDemand {
    QueueDemand {
        observed: MixedTransferPlaces::new(observed),
        ordinary_burst: Some(MixedTransferPlaces::new(ordinary)),
        protected_releases: Some(MixedTransferPlaces::new(protected)),
        guard: capacity_selection::QueueGuard::None,
    }
}

fn example() -> SelectionInput<'static> {
    SelectionInput {
        high_water: [Some(17), None, Some(5), Some(19), None, Some(6)]
            .map(|value| value.map(EventCount::measured)),
        frozen_bounds: [20, 48, 8, 64, 16, 10].map(|value| Some(EventCount::measured(value))),
        performance_store: queue(12, 10, 4),
        other_queues: &[],
        observed_holds: EventCount::measured(3),
        sound_hold_entitlement: EventCount::measured(4),
        compiled_span: HeldNoteCount::measured(9),
        default_scheduled_window: EventCount::measured(4_288),
        default_authored_headroom: EventCount::measured(4_096),
        max_quanta_per_callback: QuantumCount::measured(2),
    }
}

#[test]
fn fixed_example_applies_the_ordered_queue_hold_share_cap_and_window_rules() {
    let mut input = example();
    let queues = [queue(2, 17, 16), queue(70, 1, 0)];
    input.other_queues = &queues;
    let result = select(&input).expect("fixed arithmetic example");
    assert_eq!(result.performance_store.get(), 32);
    assert_eq!(
        result
            .other_queues
            .iter()
            .map(|depth| depth.get())
            .collect::<Vec<_>>(),
        [96, 160]
    );
    let shares = result.shares;
    assert_eq!(shares.compiled_event_share().get(), 64);
    assert_eq!(shares.authored_runtime_event_share().get(), 96);
    assert_eq!(shares.live_event_share().get(), 64);
    assert_eq!(shares.session_event_share().get(), 128);
    assert_eq!(shares.internal_event_share().get(), 32);
    assert_eq!(shares.release_event_share().get(), 96);
    assert_eq!(shares.release_hold_capacity().get(), 32);
    assert_eq!(result.max_events_per_quantum.get(), 480);
    assert_eq!(result.max_scheduled_events_in_flight.get(), 4_288);
    input.default_scheduled_window = EventCount::measured(8_192);
    assert_eq!(
        select(&input)
            .expect("larger default window")
            .max_scheduled_events_in_flight
            .get(),
        8_192
    );
}

#[test]
fn performance_store_includes_two_g_plus_one_before_doubling_and_rounding() {
    let mut input = example();
    input.performance_store = queue(0, 0, 0);
    input.sound_hold_entitlement = EventCount::measured(8);
    assert_eq!(
        select(&input).expect("hold floor").performance_store.get(),
        64
    );
}

#[test]
fn observations_and_the_compiled_panic_group_ceiling_can_raise_the_selected_shares() {
    let mut input = example();
    input.high_water[ProducerClass::Compiled.index()] = Some(EventCount::measured(33));
    input.high_water[ProducerClass::Live.index()] = Some(EventCount::measured(33));
    input.high_water[ProducerClass::Session.index()] = Some(EventCount::measured(65));
    input.high_water[ProducerClass::Release.index()] = Some(EventCount::measured(65));
    input.observed_holds = EventCount::measured(17);
    let shares = select(&input)
        .expect("observations exceed frozen bounds")
        .shares;
    assert_eq!(shares.compiled_event_share().get(), 96);
    assert_eq!(shares.live_event_share().get(), 96);
    assert_eq!(shares.session_event_share().get(), 160);
    assert_eq!(shares.release_event_share().get(), 160);
    assert_eq!(shares.release_hold_capacity().get(), 64);

    input = example();
    input.compiled_span = HeldNoteCount::measured(128);
    assert_eq!(
        select(&input)
            .expect("sixteen groups")
            .shares
            .release_event_share()
            .get(),
        96
    );
    input.compiled_span = HeldNoteCount::measured(129);
    assert_eq!(
        select(&input)
            .expect("seventeen groups")
            .shares
            .release_event_share()
            .get(),
        128
    );
}

#[test]
fn unavailable_is_distinct_from_observed_zero_for_every_class() {
    for class in [ProducerClass::AuthoredRuntime, ProducerClass::Internal] {
        let mut input = example();
        input.high_water[class.index()] = Some(EventCount::NONE);
        assert!(matches!(
            select(&input),
            Err(SelectionError::UnavailableObserved(found)) if found == class
        ));
    }
    for class in [
        ProducerClass::Compiled,
        ProducerClass::Live,
        ProducerClass::Session,
        ProducerClass::Release,
    ] {
        let mut input = example();
        input.high_water[class.index()] = Some(EventCount::NONE);
        select(&input).expect("an observed zero is valid");
        input.high_water[class.index()] = None;
        assert!(matches!(
            select(&input),
            Err(SelectionError::MissingObservation(found)) if found == class
        ));
    }
}

#[test]
fn absent_or_changed_frozen_bounds_are_refused_instead_of_inferred() {
    for class in ProducerClass::ALL {
        let mut input = example();
        input.frozen_bounds[class.index()] = None;
        assert!(matches!(
            select(&input),
            Err(SelectionError::MissingProducerBound(found)) if found == class
        ));
    }
    for (class, required) in [
        (ProducerClass::AuthoredRuntime, 48),
        (ProducerClass::Internal, 16),
    ] {
        for wrong in [required - 1, required + 1] {
            let mut input = example();
            input.frozen_bounds[class.index()] = Some(EventCount::measured(wrong));
            assert!(matches!(
                select(&input),
                Err(SelectionError::UnavailableBound { class: found, .. }) if found == class
            ));
        }
    }
    let mut demand = queue(0, 0, 0);
    demand.ordinary_burst = None;
    assert!(matches!(
        select_queue(demand),
        Err(SelectionError::MissingOrdinaryBurst)
    ));
    demand = queue(0, 0, 0);
    demand.protected_releases = None;
    assert!(matches!(
        select_queue(demand),
        Err(SelectionError::MissingProtectedDemand)
    ));
}

#[test]
fn queue_rounding_counts_protected_demand_and_refuses_arithmetic_overflow() {
    assert_eq!(select_queue(queue(0, 0, 0)).expect("minimum").get(), 32);
    assert_eq!(
        select_queue(queue(0, 16, 0)).expect("exact multiple").get(),
        32
    );
    assert_eq!(
        select_queue(queue(0, 16, 1))
            .expect("protected release")
            .get(),
        64
    );
    for demand in [queue(usize::MAX, 0, 0), queue(0, usize::MAX, 1)] {
        assert!(matches!(
            select_queue(demand),
            Err(SelectionError::Overflow(_))
        ));
    }
}

#[test]
fn illegal_sound_holds_event_cap_and_compiled_window_overflows_are_distinct_refusals() {
    let mut input = example();
    input.sound_hold_entitlement = EventCount::measured(u32::MAX);
    assert!(matches!(
        select(&input),
        Err(SelectionError::SoundHoldEntitlement(_))
    ));
    input = example();
    input.observed_holds = EventCount::measured(u32::MAX);
    assert!(matches!(
        select(&input),
        Err(SelectionError::Overflow("twice-rounded event capacity"))
    ));
    input = example();
    for class in [
        ProducerClass::Compiled,
        ProducerClass::Session,
        ProducerClass::Release,
    ] {
        input.frozen_bounds[class.index()] = Some(EventCount::measured(800_000_000));
    }
    assert!(matches!(
        select(&input),
        Err(SelectionError::Overflow("sum of the six shares"))
    ));
    input = example();
    input.frozen_bounds[ProducerClass::Compiled.index()] = Some(EventCount::measured(800_000_000));
    input.max_quanta_per_callback = QuantumCount::measured(10);
    assert!(matches!(
        select(&input),
        Err(SelectionError::Overflow("compiled callback floor"))
    ));
}

#[test]
fn a_raised_compiled_floor_preserves_the_complete_authored_headroom() {
    let mut input = example();
    input.frozen_bounds[ProducerClass::Compiled.index()] = Some(EventCount::measured(97));
    let selected = select(&input).expect("raised compiled floor");
    assert_eq!(selected.shares.compiled_event_share().get(), 224);
    assert_eq!(selected.max_scheduled_events_in_flight.get(), 4_544);
    input.default_authored_headroom = EventCount::measured(u32::MAX);
    assert!(matches!(
        select(&input),
        Err(SelectionError::Overflow(
            "compiled floor plus authored headroom"
        ))
    ));
    input = example();
    input.sound_hold_entitlement = EventCount::measured(8);
    select(&input).expect("one complete release group");
    input.sound_hold_entitlement = EventCount::measured(9);
    assert!(matches!(
        select(&input),
        Err(SelectionError::SoundHoldEntitlement(_))
    ));
}

#[test]
fn frozen_unavailable_entitlements_still_match_the_engine_defaults() {
    let profile = synth_engine_v2::profile::HostProfile::harness(
        synth_engine_v2::quantities::SampleRate::new(48_000.0).expect("reference rate"),
        synth_engine_v2::time::FrameCount::new(64),
        synth_engine_v2::quantities::ChannelLayout::Stereo,
    )
    .expect("default profile");
    for class in [ProducerClass::AuthoredRuntime, ProducerClass::Internal] {
        assert_eq!(
            example().frozen_bounds[class.index()],
            Some(class.share_of(&profile)),
            "a changed default entitlement reopens the frozen EVD method"
        );
    }
}

#[test]
fn transfer_queue_guard_is_inside_selection_and_depends_on_selected_performance() {
    let mut transfer = queue(0, 1, 0);
    transfer.guard = capacity_selection::QueueGuard::MixedTransfer;
    assert!(matches!(
        select_queue(transfer),
        Err(SelectionError::MissingPerformanceDepth)
    ));
    let queues = [transfer];
    let mut input = example();
    input.other_queues = &queues;
    let selected = select(&input).expect("C-dependent transfer queues");
    assert_eq!(selected.performance_store.get(), 32);
    assert_eq!(
        selected.other_queues[0].get(),
        64,
        "round32(max(twice demand,C+1))"
    );
    transfer.guard = capacity_selection::QueueGuard::AtLeast(MixedTransferPlaces::new(33));
    assert_eq!(select_queue(transfer).expect("explicit guard").get(), 64);
    transfer.guard = capacity_selection::QueueGuard::AtLeast(MixedTransferPlaces::new(usize::MAX));
    assert!(matches!(
        select_queue(transfer),
        Err(SelectionError::Overflow(_))
    ));
}
