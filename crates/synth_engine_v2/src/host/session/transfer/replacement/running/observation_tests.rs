//! Actual owning transitions: successor taps are pre-fade, labelled and passive.
use super::*;
use crate::{
    compile::{RenderConfig, compile},
    host::session::transfer::{
        ContinuationLimits,
        replacement::{
            ObservationAdmission, ObservationReplacementOutcome, PlanReplacementKind,
            PlanReplacementOutcome,
        },
    },
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    observe::{ObservationHandoffError, ObservationSubscriptions},
    plan::CompiledPlan,
    quantities::{Amplitude, ChannelLayout, RecordCount, SampleRate},
    render::AudioBlockMut,
    render_allocation::count_allocs,
};

const TAP: NodeId = NodeId::new(8);
const BUDGET: PreparedBytes = PreparedBytes::measured(4096);
const Q: usize = QUANTUM_FRAMES as usize;
const AFTER: usize = 8192;
fn profile(rate: f32) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(rate).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Stereo,
    )
    .unwrap()
}
fn limits() -> SessionTransferLimits {
    crate::host::session::transfer::tests::limits()
}
fn witness() -> ContinuationLimits {
    ContinuationLimits {
        maximum_transition_records: RecordCount::measured(16_384),
        maximum_stamped_events: EventCount::measured(16_384),
        witness_bytes: PreparedBytes::measured(4 * 1024 * 1024),
        transition_bytes: PreparedBytes::measured(4 * 1024 * 1024),
    }
}
fn plan(profile: HostProfile, source: f32, trim: f32, extra: bool) -> CompiledPlan {
    let mut graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Constant {
                level: Amplitude::new(source).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(
            NodeId::new(4),
            IrNodeKind::Trim {
                level: Amplitude::new(trim).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(TAP, IrNodeKind::StereoMonitor, ExecutionScope::Global)
        .node(NodeId::new(9), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(4), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (NodeId::new(4), PortId::FIRST),
            (TAP, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (TAP, PortId::FIRST),
            (NodeId::new(9), PortId::FIRST),
            SignalDomain::Audio,
        );
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
fn start(plan: CompiledPlan, profile: HostProfile) -> (SessionControl, SessionAudio) {
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let (mut control, mut audio) =
        SessionControl::prepare(plan, stream, profile, limits()).unwrap();
    let mut play = control.prepare_play(SampleTime::ZERO).unwrap();
    play.prepare_continuation_witness(witness()).unwrap();
    audio.enqueue(play).unwrap();
    (control, audio)
}
fn settle(control: &mut SessionControl, audio: &mut SessionAudio) {
    while let Some(command) = audio.take_completed() {
        let _receipt = control.collect(command).unwrap();
    }
}
#[derive(Clone, Copy)]
enum Subscriber {
    None,
    Draining,
    Stalled,
    Refused,
    Changed,
}
struct Trace {
    audio: Vec<f32>,
    tapped: Vec<f32>,
    dropped: u64,
    ended: bool,
}

fn run(
    rate: f32,
    partition: usize,
    continued: bool,
    subscriber: Subscriber,
    same_plan: bool,
) -> Trace {
    let profile = profile(rate);
    let old = plan(profile, 0.25, 1.0, false);
    let new = if same_plan {
        old.clone()
    } else if continued {
        plan(profile, 0.25, 0.5, false)
    } else {
        plan(profile, 1.0, 1.0, true)
    };
    let new_level = if same_plan {
        0.25
    } else if continued {
        0.125
    } else {
        1.0
    };
    let mut store = ObservationSubscriptions::prepare(&profile, &old).unwrap();
    let handle = store
        .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).unwrap())
        .unwrap();
    let config = store.configuration(BUDGET).unwrap();
    let admission = if matches!(subscriber, Subscriber::Refused) {
        config.prepare_handoff(&old, &new, PreparedBytes::measured(0))
    } else {
        config.prepare_handoff(&old, &new, BUDGET)
    };
    let (mut control, mut audio) = start(old.clone(), profile);
    let mut prefix = [0.0; 512];
    assert_eq!(
        count_allocs(|| audio
            .render_observed(
                AudioBlockMut::new(&mut prefix, 256, ChannelLayout::Stereo).unwrap(),
                Some(&mut store)
            )
            .unwrap()),
        0
    );
    settle(&mut control, &mut audio);
    let prefix_frames = store
        .read(handle, &mut [])
        .unwrap()
        .behind
        .as_usize()
        .unwrap();
    assert_eq!(
        prefix_frames,
        256 - Q,
        "initial priming carry produces no tap frames"
    );
    if matches!(subscriber, Subscriber::None) {
        store.end();
    }
    if matches!(subscriber, Subscriber::Changed) {
        store.end();
    }
    let at = audio.clock();
    let stream = AdmittedCompiledStream::admit(&new, &[]).unwrap();
    let request = RunningReplacementRequest {
        profile,
        transfer: limits(),
        song_end: None,
        maximum_timeline_events: EventCount::measured(16_384),
        mode: if continued {
            RunningEditMode::ContinueOrReset(witness())
        } else {
            RunningEditMode::Reset
        },
    };
    let mut candidate = control
        .prepare_running_replacement(new.clone(), stream, request, |_| Some(at))
        .unwrap();
    assert_eq!(
        candidate.kind(),
        if continued {
            PlanReplacementKind::RunningCompatible
        } else {
            PlanReplacementKind::RunningReset
        }
    );
    if !matches!(subscriber, Subscriber::None) {
        candidate
            .attach_observation(ObservationAdmission::from(admission))
            .unwrap();
    }
    let mut returned = None;
    assert_eq!(
        count_allocs(
            || returned = Some(if matches!(subscriber, Subscriber::None) {
                audio.begin_running_replacement(candidate).unwrap()
            } else {
                audio
                    .begin_running_replacement_observed(candidate, &mut store)
                    .unwrap()
            })
        ),
        0
    );
    let mut candidate = returned.unwrap();
    let handed = matches!(subscriber, Subscriber::Draining | Subscriber::Stalled);
    if handed {
        assert!(matches!(
            candidate.observation_outcome(),
            ObservationReplacementOutcome::HandedOff { .. }
        ));
    }
    if matches!(subscriber, Subscriber::Refused) {
        assert_eq!(
            candidate.observation_outcome(),
            ObservationReplacementOutcome::Ended(ObservationHandoffError::Bytes)
        );
    }
    if matches!(subscriber, Subscriber::Changed) {
        assert_eq!(
            candidate.observation_outcome(),
            ObservationReplacementOutcome::Ended(ObservationHandoffError::Changed)
        );
    }
    // An ordinary successor is the tap oracle. Its signal excludes the reset fade.
    let stream = AdmittedCompiledStream::admit(&new, &[]).unwrap();
    let (_reference_control, mut reference) = prepare_initial(
        new.clone(),
        stream,
        profile,
        limits(),
        StreamAnchor::new(at, PlanPosition::new(at.as_u64())),
    )
    .unwrap();
    let mut reference_store = ObservationSubscriptions::prepare(&profile, &new).unwrap();
    let reference_handle = reference_store
        .subscribe(&new, new.resolve_tap(TAP, PortId::FIRST).unwrap())
        .unwrap();
    let mut output = vec![0.0; AFTER * 2];
    let mut tapped = vec![0.0; (AFTER + Q) * 2];
    let mut total = 0;
    let mut dropped = 0;
    let mut expected_taps = vec![0.0; (AFTER + Q) * 2];
    let mut expected_total = 0;
    for chunk in output.chunks_mut(partition * 2) {
        let frames = chunk.len() / 2;
        let mut reference_output = vec![0.0; frames * 2];
        reference
            .render_observed(
                AudioBlockMut::new(&mut reference_output, frames, ChannelLayout::Stereo).unwrap(),
                Some(&mut reference_store),
            )
            .unwrap();
        let read = reference_store
            .read(reference_handle, &mut expected_taps[expected_total * 2..])
            .unwrap();
        expected_total += read.frames.as_usize().unwrap();
        assert_eq!(
            count_allocs(|| {
                let block = AudioBlockMut::new(chunk, frames, ChannelLayout::Stereo).unwrap();
                if candidate.outcome().is_none() {
                    if matches!(subscriber, Subscriber::None) {
                        audio
                            .render_running_replacement(&mut candidate, block)
                            .unwrap();
                    } else {
                        audio
                            .render_running_replacement_observed(&mut candidate, block, &mut store)
                            .unwrap();
                    }
                } else if matches!(subscriber, Subscriber::None) {
                    audio.render(block).unwrap();
                } else {
                    audio.render_observed(block, Some(&mut store)).unwrap();
                }
                if matches!(subscriber, Subscriber::Draining) {
                    let read = store.read(handle, &mut tapped[total * 2..]).unwrap();
                    total += read.frames.as_usize().unwrap();
                    dropped += read.dropped.as_u64();
                    assert_eq!(read.plan, new.id());
                    assert_eq!(read.generation.as_u64(), 1);
                    assert!(!read.ended);
                }
            }),
            0
        );
    }
    let read = store.read(handle, &mut tapped[total * 2..]).unwrap();
    total += read.frames.as_usize().unwrap();
    dropped += read.dropped.as_u64();
    let ended = read.ended;
    tapped.truncate(total * 2);
    expected_taps.truncate(expected_total * 2);
    if matches!(subscriber, Subscriber::Draining) {
        assert_eq!(tapped, expected_taps);
        assert_eq!(total, AFTER);
    }
    if matches!(subscriber, Subscriber::Stalled) {
        assert!(dropped > prefix_frames as u64);
        assert!(
            total < AFTER,
            "stalled subscriber actually saturated its ring"
        );
    }
    if handed {
        assert!(tapped.iter().all(|sample| *sample == new_level));
        assert_eq!(dropped + total as u64, (prefix_frames + AFTER) as u64);
    }
    if matches!(subscriber, Subscriber::Refused | Subscriber::Changed) {
        assert!(ended);
        assert_eq!(read.plan, old.id());
        assert_eq!(read.generation.as_u64(), 0);
        assert_eq!(tapped, vec![0.25; prefix_frames * 2]);
    }
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Installed(_)
    ));
    if handed {
        assert!(matches!(
            receipt.observation,
            ObservationReplacementOutcome::HandedOff { .. }
        ));
    }
    Trace {
        audio: output,
        tapped,
        dropped,
        ended,
    }
}

#[test]
fn reset_and_continuation_observers_follow_actual_successor_quanta_without_changing_sound() {
    for rate in [44_100.0, 48_000.0, 88_200.0, 96_000.0] {
        for continued in [false, true] {
            let oracle = run(rate, 512, continued, Subscriber::None, false);
            for partition in [1, 37, 64, 511] {
                for subscriber in [
                    Subscriber::Draining,
                    Subscriber::Stalled,
                    Subscriber::Refused,
                    Subscriber::Changed,
                ] {
                    let trace = run(rate, partition, continued, subscriber, false);
                    assert_eq!(
                        trace.audio, oracle.audio,
                        "rate={rate} partition={partition} continuation={continued}"
                    );
                    if matches!(subscriber, Subscriber::Draining) {
                        assert_eq!(trace.dropped, 192);
                        assert!(!trace.ended);
                    }
                }
            }
        }
    }
}

#[test]
fn fresh_owner_with_the_same_plan_id_still_changes_observation_generation() {
    let trace = run(48_000.0, 1, false, Subscriber::Draining, true);
    assert_eq!(trace.tapped, vec![0.25; AFTER * 2]);
    assert_eq!(trace.dropped, 192);
}

#[test]
fn a_refused_sound_install_never_changes_the_observation_store() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.25, 1.0, false);
    let new = plan(profile, 1.0, 1.0, true);
    let mut store = ObservationSubscriptions::prepare(&profile, &old).unwrap();
    let handle = store
        .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).unwrap())
        .unwrap();
    let config = store.configuration(BUDGET).unwrap();
    let (mut control, mut audio) = start(old.clone(), profile);
    let mut prefix = [0.0; 512];
    audio
        .render_observed(
            AudioBlockMut::new(&mut prefix, 256, ChannelLayout::Stereo).unwrap(),
            Some(&mut store),
        )
        .unwrap();
    settle(&mut control, &mut audio);
    let at = audio.clock();
    let stream = AdmittedCompiledStream::admit(&new, &[]).unwrap();
    let request = RunningReplacementRequest {
        profile,
        transfer: limits(),
        song_end: None,
        maximum_timeline_events: EventCount::measured(16_384),
        mode: RunningEditMode::Reset,
    };
    let mut candidate = control
        .prepare_running_replacement(new.clone(), stream, request, |_| Some(at))
        .unwrap();
    candidate
        .attach_observation(config.prepare_handoff(&old, &new, BUDGET).into())
        .unwrap();
    let mut later = [0.0; Q * 2];
    audio
        .render_observed(
            AudioBlockMut::new(&mut later, Q, ChannelLayout::Stereo).unwrap(),
            Some(&mut store),
        )
        .unwrap();
    let before = format!("{store:?}");
    let mut returned = None;
    assert_eq!(
        count_allocs(|| returned = Some(
            audio
                .begin_running_replacement_observed(candidate, &mut store)
                .unwrap()
        )),
        0
    );
    assert_eq!(format!("{store:?}"), before);
    let receipt = control.collect_replacement(returned.unwrap()).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Refused(_)
    ));
    assert_eq!(
        receipt.observation,
        ObservationReplacementOutcome::NotAttempted
    );
    assert!(!store.read(handle, &mut []).unwrap().ended);
    audio
        .render_observed(
            AudioBlockMut::new(&mut later, Q, ChannelLayout::Stereo).unwrap(),
            Some(&mut store),
        )
        .unwrap();
    assert!(later.iter().all(|sample| *sample == 0.25));
}

#[test]
fn interrupted_reset_and_continuation_end_the_installed_observation_generation() {
    for continued in [false, true] {
        let profile = profile(48_000.0);
        let old = plan(profile, 0.25, 1.0, false);
        let new = if continued {
            plan(profile, 0.25, 0.5, false)
        } else {
            plan(profile, 1.0, 1.0, true)
        };
        let mut store = ObservationSubscriptions::prepare(&profile, &old).unwrap();
        let handle = store
            .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).unwrap())
            .unwrap();
        let config = store.configuration(BUDGET).unwrap();
        let (mut control, mut audio) = start(old.clone(), profile);
        let mut prefix = [0.0; 512];
        audio
            .render_observed(
                AudioBlockMut::new(&mut prefix, 256, ChannelLayout::Stereo).unwrap(),
                Some(&mut store),
            )
            .unwrap();
        settle(&mut control, &mut audio);
        let at = audio.clock();
        let stream = AdmittedCompiledStream::admit(&new, &[]).unwrap();
        let request = RunningReplacementRequest {
            profile,
            transfer: limits(),
            song_end: None,
            maximum_timeline_events: EventCount::measured(16_384),
            mode: if continued {
                RunningEditMode::ContinueOrReset(witness())
            } else {
                RunningEditMode::Reset
            },
        };
        let mut candidate = control
            .prepare_running_replacement(new.clone(), stream, request, |_| Some(at))
            .unwrap();
        candidate
            .attach_observation(config.prepare_handoff(&old, &new, BUDGET).into())
            .unwrap();
        let before = format!("{store:?}");
        let (candidate, error) = audio.begin_running_replacement(candidate).unwrap_err();
        assert!(matches!(error, PlanReplacementError::State));
        assert_eq!(before, format!("{store:?}"));
        assert_eq!(
            candidate.observation_outcome(),
            ObservationReplacementOutcome::NotAttempted
        );
        let mut candidate = audio
            .begin_running_replacement_observed(candidate, &mut store)
            .unwrap();
        let mut wrong_layout = [9.0; 1];
        assert_eq!(
            count_allocs(|| {
                assert!(
                    audio
                        .render_running_replacement_observed(
                            &mut candidate,
                            AudioBlockMut::new(&mut wrong_layout, 1, ChannelLayout::Mono).unwrap(),
                            &mut store
                        )
                        .is_err()
                );
            }),
            0
        );
        assert_eq!(wrong_layout, [0.0]);
        let read = store.read(handle, &mut []).unwrap();
        assert!(read.ended);
        assert_eq!(read.plan, new.id());
        assert_eq!(read.generation.as_u64(), 1);
        let receipt = control.collect_replacement(candidate).unwrap();
        assert!(matches!(
            receipt.outcome,
            PlanReplacementOutcome::Interrupted(_)
        ));
        assert!(matches!(
            receipt.observation,
            ObservationReplacementOutcome::Interrupted { .. }
        ));
    }
}

#[test]
fn a_lost_paired_store_or_bypassed_observed_render_reports_detachment() {
    for continued in [false, true] {
        for bypass in [false, true] {
            let profile = profile(48_000.0);
            let old = plan(profile, 0.25, 1.0, false);
            let new = if continued {
                plan(profile, 0.25, 0.5, false)
            } else {
                plan(profile, 1.0, 1.0, true)
            };
            let mut store = ObservationSubscriptions::prepare(&profile, &old).unwrap();
            let handle = store
                .subscribe(&old, old.resolve_tap(TAP, PortId::FIRST).unwrap())
                .unwrap();
            let config = store.configuration(BUDGET).unwrap();
            let (mut control, mut audio) = start(old.clone(), profile);
            let mut prefix = [0.0; 512];
            audio
                .render_observed(
                    AudioBlockMut::new(&mut prefix, 256, ChannelLayout::Stereo).unwrap(),
                    Some(&mut store),
                )
                .unwrap();
            settle(&mut control, &mut audio);
            let at = audio.clock();
            let stream = AdmittedCompiledStream::admit(&new, &[]).unwrap();
            let request = RunningReplacementRequest {
                profile,
                transfer: limits(),
                song_end: None,
                maximum_timeline_events: EventCount::measured(16_384),
                mode: if continued {
                    RunningEditMode::ContinueOrReset(witness())
                } else {
                    RunningEditMode::Reset
                },
            };
            let mut candidate = control
                .prepare_running_replacement(new.clone(), stream, request, |_| Some(at))
                .unwrap();
            candidate
                .attach_observation(config.prepare_handoff(&old, &new, BUDGET).into())
                .unwrap();
            let before = format!("{store:?}");
            let (candidate, error) = audio.begin_running_replacement(candidate).unwrap_err();
            assert!(matches!(error, PlanReplacementError::State));
            assert_eq!(before, format!("{store:?}"));
            assert_eq!(
                candidate.observation_outcome(),
                ObservationReplacementOutcome::NotAttempted
            );
            let mut candidate = audio
                .begin_running_replacement_observed(candidate, &mut store)
                .unwrap();
            let mut foreign = ObservationSubscriptions::prepare(&profile, &new).unwrap();
            let foreign_handle = foreign
                .subscribe(&new, new.resolve_tap(TAP, PortId::FIRST).unwrap())
                .unwrap();
            let mut output = [0.0; Q * 2];
            assert_eq!(
                count_allocs(|| {
                    let block = AudioBlockMut::new(&mut output, Q, ChannelLayout::Stereo).unwrap();
                    if bypass {
                        audio
                            .render_running_replacement(&mut candidate, block)
                            .unwrap();
                    } else {
                        audio
                            .render_running_replacement_observed(
                                &mut candidate,
                                block,
                                &mut foreign,
                            )
                            .unwrap();
                    }
                }),
                0
            );
            for (index, frame) in output.as_chunks::<2>().0.iter().enumerate() {
                let gain = f32::from(u16::try_from(index + 1).unwrap())
                    / f32::from(u16::try_from(Q).unwrap());
                let expected = if continued {
                    0.125
                } else {
                    0.25 * (1.0 - gain) + gain
                };
                assert_eq!(frame, &[expected, expected]);
            }
            assert!(
                !store.read(handle, &mut []).unwrap().ended,
                "the inaccessible original was not ended"
            );
            if !bypass {
                assert!(foreign.read(foreign_handle, &mut []).unwrap().ended);
            }
            let receipt = control.collect_replacement(candidate).unwrap();
            assert!(matches!(
                receipt.outcome,
                PlanReplacementOutcome::Installed(_)
            ));
            assert_eq!(
                receipt.observation,
                ObservationReplacementOutcome::Detached(if bypass {
                    ObservationHandoffError::NotObserved
                } else {
                    ObservationHandoffError::Origin
                })
            );
        }
    }
}
