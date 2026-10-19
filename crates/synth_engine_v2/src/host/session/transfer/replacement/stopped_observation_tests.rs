//! Stopped installation observes only the committed owner, preserving ordinary sound.
use super::*;
use crate::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    observe::{ObservationHandoffError, ObservationSubscriptions, SubscriptionId},
    quantities::{
        Amplitude, ChannelLayout, DelayFeedback, DelayTime, Frequency, NormalizedLevel,
        PreparedBytes, SampleRate,
    },
    render::AudioBlockMut,
    render_allocation::count_allocs,
    time::{FrameCount, SampleTime},
};
const TAP: NodeId = NodeId::new(8);
const BUDGET: PreparedBytes = PreparedBytes::measured(4096);
const AFTER: usize = 8192;
fn profile(rate: f32) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(rate).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Stereo,
    )
    .unwrap()
}
fn plan(profile: HostProfile, extra: bool) -> CompiledPlan {
    let mut graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: Frequency::new(317.0).unwrap(),
                amplitude: Amplitude::new(0.25).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(
            NodeId::new(4),
            IrNodeKind::Trim {
                level: Amplitude::UNITY,
            },
            ExecutionScope::Global,
        )
        .node(
            NodeId::new(6),
            IrNodeKind::Delay {
                time_left: DelayTime::new(0.001).unwrap(),
                time_right: DelayTime::new(0.002).unwrap(),
                feedback: DelayFeedback::new(0.4).unwrap(),
                mix: NormalizedLevel::new(0.6).unwrap(),
                tone: NormalizedLevel::FULL,
            },
            ExecutionScope::Global,
        )
        .node(TAP, IrNodeKind::StereoMonitor, ExecutionScope::Global)
        .node(NodeId::new(9), IrNodeKind::Output, ExecutionScope::Global);
    for (from, to) in [(1, 4), (4, 6), (6, 8), (8, 9)] {
        graph = graph.connect(
            (NodeId::new(from), PortId::FIRST),
            (NodeId::new(to), PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    if extra {
        graph = graph.node(
            NodeId::new(2),
            IrNodeKind::StereoMonitor,
            ExecutionScope::Global,
        );
    }
    compile(&graph.build().unwrap(), &RenderConfig::new(profile))
        .into_plan()
        .unwrap()
}
fn store(plan: &CompiledPlan, profile: HostProfile) -> (ObservationSubscriptions, SubscriptionId) {
    let mut store = ObservationSubscriptions::prepare(&profile, plan).unwrap();
    let handle = store
        .subscribe(plan, plan.resolve_tap(TAP, PortId::FIRST).unwrap())
        .unwrap();
    (store, handle)
}
fn draw(
    audio: &mut SessionAudio,
    observers: Option<&mut ObservationSubscriptions>,
    frames: usize,
) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    assert_eq!(
        count_allocs(|| audio
            .render_observed(
                AudioBlockMut::new(&mut output, frames, ChannelLayout::Stereo).unwrap(),
                observers
            )
            .unwrap()),
        0
    );
    output
}
fn settle(control: &mut SessionControl, audio: &mut SessionAudio) {
    while let Some(command) = audio.take_completed() {
        let _receipt = control.collect(command).unwrap();
    }
}
fn warm(
    plan: CompiledPlan,
    profile: HostProfile,
    observers: Option<&mut ObservationSubscriptions>,
) -> (SessionControl, SessionAudio, Vec<f32>) {
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let (mut control, mut audio) =
        SessionControl::prepare(plan, stream, profile, super::super::tests::limits()).unwrap();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    let prefix = draw(&mut audio, observers, 512);
    settle(&mut control, &mut audio);
    audio
        .enqueue(control.prepare_stop(audio.clock()).unwrap())
        .unwrap();
    assert!(
        draw(&mut audio, None, 64)
            .iter()
            .all(|sample| *sample == 0.0)
    );
    settle(&mut control, &mut audio);
    (control, audio, prefix)
}
fn prepare(
    control: &mut SessionControl,
    new: CompiledPlan,
    profile: HostProfile,
    compatible: bool,
) -> Box<PreparedPlanReplacement> {
    let stream = AdmittedCompiledStream::admit(&new, &[]).unwrap();
    let limits = super::super::tests::limits();
    if compatible {
        control
            .prepare_compatible_replacement(new, stream, profile, limits)
            .unwrap()
    } else {
        control
            .prepare_replacement(new, stream, profile, limits)
            .unwrap()
    }
}
#[derive(Clone, Copy)]
enum Subscriber {
    Absent,
    Draining,
    Stalled,
    Budget,
    Changed,
    Foreign,
    Missing,
}
fn run(
    old: &CompiledPlan,
    new: &CompiledPlan,
    profile: HostProfile,
    compatible: bool,
    partition: usize,
    subscriber: Subscriber,
) -> Vec<f32> {
    let (mut store, handle) = store(old, profile);
    let config = store.configuration(BUDGET).unwrap();
    let (mut control, mut audio, prefix) = warm(old.clone(), profile, Some(&mut store));
    let old_frames = prefix.len() / 2 - 64;
    assert_eq!(
        store.read(handle, &mut []).unwrap().behind,
        FrameCount::new(u64::try_from(old_frames).unwrap())
    );
    let map = match subscriber {
        Subscriber::Foreign => {
            let (foreign, _) = self::store(old, profile);
            foreign.prepare_handoff(old, new, BUDGET)
        }
        Subscriber::Budget => config.prepare_handoff(old, new, PreparedBytes::NONE),
        _ => config.prepare_handoff(old, new, BUDGET),
    };
    if matches!(subscriber, Subscriber::Changed | Subscriber::Absent) {
        store.end();
    }
    let mut candidate = prepare(&mut control, new.clone(), profile, compatible);
    if !matches!(subscriber, Subscriber::Absent | Subscriber::Missing) {
        candidate.attach_observation(map.into()).unwrap();
    }
    let mut returned = None;
    assert_eq!(
        count_allocs(
            || returned = Some(if matches!(subscriber, Subscriber::Absent) {
                audio.install_replacement(candidate).unwrap()
            } else {
                audio
                    .install_replacement_observed(candidate, &mut store)
                    .unwrap()
            })
        ),
        0
    );
    let receipt = control.collect_replacement(returned.unwrap()).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Installed(_)
    ));
    assert!(!control.has_replacements());
    let handed = matches!(subscriber, Subscriber::Draining | Subscriber::Stalled);
    let first = store.read(handle, &mut []).unwrap();
    let mut dropped = first.dropped.as_u64();
    if handed {
        assert_eq!(first.plan, new.id());
        assert_eq!(first.generation.as_u64(), 1);
        assert_eq!(first.behind, FrameCount::ZERO);
        assert!(!first.ended);
        assert_eq!(dropped, u64::try_from(old_frames).unwrap());
        assert!(
            matches!(receipt.observation, ObservationReplacementOutcome::HandedOff { generation, kind } if generation.as_u64() == 1 && kind == if compatible { PlanReplacementKind::StoppedCompatible } else { PlanReplacementKind::StoppedReset })
        );
    } else if !matches!(subscriber, Subscriber::Absent) {
        let error = match subscriber {
            Subscriber::Budget => ObservationHandoffError::Bytes,
            Subscriber::Changed => ObservationHandoffError::Changed,
            Subscriber::Foreign => ObservationHandoffError::Origin,
            _ => ObservationHandoffError::NotPrepared,
        };
        assert_eq!(
            receipt.observation,
            ObservationReplacementOutcome::Ended(error)
        );
        assert!(first.ended);
        assert_eq!(first.plan, old.id());
        assert_eq!(first.generation.as_u64(), 0);
    }
    // Idle time advances the engine clock but has no DSP or tap coverage.
    for start in (0..192).step_by(partition) {
        assert!(
            draw(&mut audio, Some(&mut store), partition.min(192 - start))
                .iter()
                .all(|sample| *sample == 0.0)
        );
    }
    assert_eq!(
        store.read(handle, &mut []).unwrap().behind,
        if handed {
            FrameCount::ZERO
        } else {
            FrameCount::new(u64::try_from(old_frames).unwrap())
        }
    );
    audio
        .enqueue(control.prepare_play(audio.clock()).unwrap())
        .unwrap();
    let mut output = Vec::with_capacity(AFTER * 2);
    let mut taps = vec![0.0; AFTER * 2];
    let mut total = 0;
    for start in (0..AFTER).step_by(partition) {
        output.extend(draw(
            &mut audio,
            if matches!(subscriber, Subscriber::Absent) {
                None
            } else {
                Some(&mut store)
            },
            partition.min(AFTER - start),
        ));
        if matches!(subscriber, Subscriber::Draining) {
            let read = store.read(handle, &mut taps[total..]).unwrap();
            total += read.frames.as_usize().unwrap() * 2;
            dropped += read.dropped.as_u64();
        }
    }
    let read = store.read(handle, &mut taps[total..]).unwrap();
    total += read.frames.as_usize().unwrap() * 2;
    dropped += read.dropped.as_u64();
    taps.truncate(total);
    if handed {
        assert_eq!(
            dropped + u64::try_from(total / 2).unwrap(),
            u64::try_from(old_frames + AFTER).unwrap()
        );
        assert_eq!(taps, output[output.len() - total..]);
        if matches!(subscriber, Subscriber::Draining) {
            assert_eq!(taps, output);
        } else {
            assert!(dropped > u64::try_from(old_frames).unwrap());
        }
    } else {
        assert!(read.ended);
        assert_eq!(taps, prefix[128..]);
    }
    settle(&mut control, &mut audio);
    assert!(!control.has_outstanding());
    output
}
#[test]
fn stopped_reset_and_migration_handoff_are_passive_across_rates_partitions_and_ring_pressure() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        let old = plan(profile, false);
        let new = plan(profile, true);
        assert_ne!(
            old.resolve_tap(TAP, PortId::FIRST).unwrap().index(),
            new.resolve_tap(TAP, PortId::FIRST).unwrap().index()
        );
        for compatible in [false, true] {
            let oracle = run(&old, &new, profile, compatible, 512, Subscriber::Absent);
            for partition in [1, 37, 64, 511] {
                for subscriber in [
                    Subscriber::Draining,
                    Subscriber::Stalled,
                    Subscriber::Budget,
                    Subscriber::Changed,
                    Subscriber::Foreign,
                    Subscriber::Missing,
                ] {
                    let sound = run(&old, &new, profile, compatible, partition, subscriber);
                    assert_eq!(
                        sound.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        oracle.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}
#[test]
fn migration_keeps_warm_history_and_reset_restarts_it_even_with_the_same_plan_id() {
    let profile = profile(48_000.0);
    let old = plan(profile, false);
    let reset = run(&old, &old, profile, false, 37, Subscriber::Draining);
    let migrated = run(&old, &old, profile, true, 37, Subscriber::Draining);
    assert_ne!(
        reset, migrated,
        "the migration comparison must exercise warm DSP history"
    );
    let (mut control, mut reference, _) = warm(old.clone(), profile, None);
    let _idle = draw(&mut reference, None, 192);
    reference
        .enqueue(control.prepare_play(reference.clock()).unwrap())
        .unwrap();
    let mut expected = Vec::new();
    for start in (0..AFTER).step_by(37) {
        expected.extend(draw(&mut reference, None, 37.min(AFTER - start)));
    }
    assert_eq!(
        migrated, expected,
        "unchanged ordinary owner retains exactly the same history"
    );
    let stream = AdmittedCompiledStream::admit(&old, &[]).unwrap();
    let (mut fresh_control, mut fresh) =
        SessionControl::prepare(old, stream, profile, super::super::tests::limits()).unwrap();
    fresh
        .enqueue(fresh_control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    assert!(
        draw(&mut fresh, None, 64)
            .iter()
            .all(|sample| *sample == 0.0)
    );
    let mut expected = Vec::new();
    for start in (0..AFTER).step_by(37) {
        expected.extend(draw(&mut fresh, None, 37.min(AFTER - start)));
    }
    assert_eq!(reset, expected, "fresh ordinary owner is the reset oracle");
}
#[test]
fn refused_sound_and_cancelled_or_misrouted_envelopes_never_rebind_the_store() {
    let profile = profile(48_000.0);
    for migration_fault in [false, true] {
        let old = plan(profile, false);
        let new = plan(profile, true);
        let (mut store, handle) = store(&old, profile);
        let (mut control, mut audio, _) = warm(old.clone(), profile, Some(&mut store));
        let mut candidate = prepare(&mut control, new.clone(), profile, migration_fault);
        candidate
            .attach_observation(store.prepare_handoff(&old, &new, BUDGET).into())
            .unwrap();
        let before = format!("{store:?}");
        let (candidate, error) = audio.install_replacement(candidate).unwrap_err();
        assert!(matches!(error, PlanReplacementError::State));
        assert_eq!(before, format!("{store:?}"));
        let cancelled = control.cancel_replacement(candidate).unwrap();
        assert_eq!(
            cancelled.observation,
            ObservationReplacementOutcome::NotAttempted
        );
        assert_eq!(before, format!("{store:?}"));
        let mut candidate = prepare(&mut control, new.clone(), profile, migration_fault);
        candidate
            .attach_observation(store.prepare_handoff(&old, &new, BUDGET).into())
            .unwrap();
        if migration_fault {
            let foreign = plan(profile, false);
            candidate.migration = Some(
                crate::render::PreparedGlobalMigration::prepare(&foreign, &new, BUDGET).unwrap(),
            );
        } else {
            let _carry = draw(&mut audio, Some(&mut store), 1);
        }
        let before = format!("{store:?}");
        let mut returned = None;
        assert_eq!(
            count_allocs(|| returned = Some(
                audio
                    .install_replacement_observed(candidate, &mut store)
                    .unwrap()
            )),
            0
        );
        let receipt = control.collect_replacement(returned.unwrap()).unwrap();
        assert!(
            matches!(
                receipt.outcome,
                PlanReplacementOutcome::Refused(PlanReplacementRefusal::Migration(_))
            ) && migration_fault
                || matches!(
                    receipt.outcome,
                    PlanReplacementOutcome::Refused(PlanReplacementRefusal::Carry)
                ) && !migration_fault
        );
        assert_eq!(
            receipt.observation,
            ObservationReplacementOutcome::NotAttempted
        );
        assert_eq!(before, format!("{store:?}"));
        assert!(!store.read(handle, &mut []).unwrap().ended);
        assert!(!control.has_replacements());
    }
}

#[test]
fn a_legacy_entry_returns_the_attached_envelope_for_one_observed_retry() {
    let profile = profile(48_000.0);
    for compatible in [false, true] {
        let old = plan(profile, false);
        let new = plan(profile, true);
        let (mut store, handle) = store(&old, profile);
        let (mut control, mut audio, _) = warm(old.clone(), profile, Some(&mut store));
        let mut candidate = prepare(&mut control, new.clone(), profile, compatible);
        candidate
            .attach_observation(store.prepare_handoff(&old, &new, BUDGET).into())
            .unwrap();
        let publication = candidate.id();
        let clock = audio.clock();
        let before = format!("{store:?}");
        let mut refused = None;
        assert_eq!(
            count_allocs(|| refused = Some(audio.install_replacement(candidate).unwrap_err())),
            0
        );
        let (candidate, error) = refused.unwrap();
        assert!(matches!(error, PlanReplacementError::State));
        assert_eq!(candidate.id(), publication);
        assert!(candidate.outcome().is_none());
        assert_eq!(
            candidate.observation_outcome(),
            ObservationReplacementOutcome::NotAttempted
        );
        assert_eq!(audio.clock(), clock);
        assert_eq!(audio.plan_id(), old.id());
        assert_eq!(before, format!("{store:?}"));
        let mut installed = None;
        assert_eq!(
            count_allocs(|| installed = Some(
                audio
                    .install_replacement_observed(candidate, &mut store)
                    .unwrap()
            )),
            0
        );
        let receipt = control.collect_replacement(installed.unwrap()).unwrap();
        assert_eq!(receipt.id, publication);
        assert!(matches!(
            receipt.outcome,
            PlanReplacementOutcome::Installed(_)
        ));
        assert!(matches!(
            receipt.observation,
            ObservationReplacementOutcome::HandedOff { .. }
        ));
        assert_eq!(store.read(handle, &mut []).unwrap().generation.as_u64(), 1);
        assert_eq!(audio.plan_id(), new.id());
        assert!(!control.has_replacements());
    }
}
