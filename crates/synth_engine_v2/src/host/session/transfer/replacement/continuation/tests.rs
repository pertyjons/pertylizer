use super::*;
use crate::{
    compile::{RenderConfig, compile},
    host::session::transfer::tests::limits,
    host::session::{
        PlaybackState, SessionOutcome,
        transfer::{
            ContinuationLimits, SessionAudio, SessionControl,
            replacement::{
                PlanReplacementKind, PlanReplacementOutcome, PlanReplacementRefusal,
                RunningEditMode, RunningReplacementRequest,
            },
        },
    },
    ir::{
        ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations,
        PortId, SignalDomain,
    },
    profile::HostProfile,
    quantities::{
        Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity,
        NormalizedLevel, NoteVelocity, ParameterValue, PreparedBytes, SampleRate, Seconds,
    },
    render::AudioBlockMut,
    schedule::PlanEvent,
    time::{FrameCount, PlanPosition, SampleTime},
};

const TRIM: NodeId = NodeId::new(4);
fn profile(rate: f32) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(rate).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Stereo,
    )
    .unwrap()
}
fn plan(profile: HostProfile, level: f32) -> CompiledPlan {
    plan_with_stealing(profile, level, crate::ir::StealingPolicy::None)
}
fn plan_with_stealing(
    profile: HostProfile,
    level: f32,
    stealing: crate::ir::StealingPolicy,
) -> CompiledPlan {
    plan_with_global_stage(profile, level, stealing, None)
}

fn plan_with_global_stage(
    profile: HostProfile,
    level: f32,
    stealing: crate::ir::StealingPolicy,
    stage: Option<IrNodeKind>,
) -> CompiledPlan {
    plan_with_global_stages(profile, level, stealing, stage.as_slice())
}

fn plan_with_global_stages(
    profile: HostProfile,
    level: f32,
    stealing: crate::ir::StealingPolicy,
    stages: &[IrNodeKind],
) -> CompiledPlan {
    let stages: Vec<_> = stages
        .iter()
        .enumerate()
        .map(|(index, kind)| {
            (
                NodeId::new(6 + u32::try_from(index).unwrap()),
                *kind,
                ExecutionScope::Global,
            )
        })
        .collect();
    plan_with_keyed_global_stages(profile, level, stealing, &stages)
}

fn plan_with_keyed_global_stages(
    profile: HostProfile,
    level: f32,
    stealing: crate::ir::StealingPolicy,
    stages: &[(NodeId, IrNodeKind, ExecutionScope)],
) -> CompiledPlan {
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).unwrap(),
                amplitude: Amplitude::new(0.5).unwrap(),
            },
            ExecutionScope::Voice,
        )
        .node(
            NodeId::new(2),
            IrNodeKind::Envelope {
                attack: Seconds::new(0.005).unwrap(),
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            ExecutionScope::Voice,
        )
        .node(NodeId::new(3), IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(
            TRIM,
            IrNodeKind::Trim {
                level: Amplitude::new(level).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(NodeId::new(5), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(3), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (NodeId::new(2), PortId::FIRST),
            (NodeId::new(3), crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (NodeId::new(3), PortId::FIRST),
            (TRIM, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().unwrap(),
        )
        .declaring(PlanDeclarations {
            held_notes: HeldNoteCount::measured(2),
            stealing,
            note_producers: vec![NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(2),
                simultaneous_holds: EventCount::NONE,
            }],
            ..PlanDeclarations::default()
        });
    let mut graph = graph;
    let mut previous = TRIM;
    for (node, kind, scope) in stages {
        graph = graph.node(*node, *kind, *scope).connect(
            (previous, PortId::FIRST),
            (*node, PortId::FIRST),
            SignalDomain::Audio,
        );
        previous = *node;
    }
    let graph = graph
        .connect(
            (previous, PortId::FIRST),
            (NodeId::new(5), PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .unwrap();
    compile(&graph, &RenderConfig::new(profile))
        .into_plan()
        .unwrap()
}

#[test]
fn a_global_structural_stage_preserves_actual_held_voices_and_later_releases() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        for partitions in [&[1][..], &[64], &[256], &[512], &[7, 19, 3, 111]] {
            let old = plan(profile, 0.8);
            let program = events(&old);
            let (_, mut reference) = start(old.clone(), &program, profile);
            let (mut control, mut audio) = start(old, &program, profile);
            let table = audio.origin.table;
            let epoch = audio.epoch();
            let new = plan_with_global_stage(
                profile,
                0.8,
                crate::ir::StealingPolicy::None,
                Some(IrNodeKind::HardClamp),
            );
            assert!(matches!(
                audio.renderer.plan().prepare_continuation(
                    &new,
                    witness_limits().transition_bytes,
                    crate::quantities::RecordCount::measured(0)
                ),
                Err(ContinuationError::RecordLimit)
            ));
            let candidate = prepare(&mut control, &audio, new, profile);
            assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
            assert!(
                candidate
                    .continuation
                    .as_ref()
                    .unwrap()
                    .proof
                    .structural
                    .is_some()
            );
            let mut installed = None;
            assert_eq!(
                crate::render_allocation::count_allocs(
                    || installed = Some(audio.begin_running_replacement(candidate).unwrap())
                ),
                0
            );
            let mut candidate = installed.unwrap();
            let mut frame = 0;
            let mut part = 0;
            let mut sound = Vec::new();
            while frame < 1280 {
                let count = partitions[part % partitions.len()].min(1280 - frame);
                let mut output = vec![0.0; count * 2];
                assert_eq!(
                    crate::render_allocation::count_allocs(|| {
                        let block =
                            AudioBlockMut::new(&mut output, count, ChannelLayout::Stereo).unwrap();
                        if candidate.outcome().is_none() {
                            audio
                                .render_running_replacement(&mut candidate, block)
                                .unwrap();
                        } else {
                            audio.render(block).unwrap();
                        }
                    }),
                    0
                );
                let expected = render(&mut reference, count);
                assert_eq!(
                    output
                        .iter()
                        .map(|sample| sample.to_bits())
                        .collect::<Vec<_>>(),
                    expected
                        .iter()
                        .map(|sample| sample.to_bits())
                        .collect::<Vec<_>>(),
                    "rate={rate}, parts={partitions:?}, frame={frame}"
                );
                sound.extend(output);
                frame += count;
                part += 1;
            }
            assert!(sound.iter().any(|sample| *sample != 0.0));
            assert!(sound[2000..].iter().all(|sample| *sample == 0.0));
            let receipt = control.collect_replacement(candidate).unwrap();
            assert!(matches!(
                receipt.outcome,
                PlanReplacementOutcome::Installed(_)
            ));
            assert_eq!(audio.origin.table, table);
            assert_eq!(control.origin.table, table);
            assert_eq!(audio.epoch(), epoch);
            assert_eq!(audio.publication_faults(), 0);
        }
    }
}

#[test]
fn repeated_structural_edits_keep_warm_delay_history_and_bounded_owning_custody() {
    use crate::quantities::{DelayFeedback, DelayTime};
    let profile = profile(48_000.0);
    let delay = IrNodeKind::Delay {
        time_left: DelayTime::new(0.001).unwrap(),
        time_right: DelayTime::new(0.002).unwrap(),
        feedback: DelayFeedback::new(0.4).unwrap(),
        mix: NormalizedLevel::new(0.4).unwrap(),
        tone: NormalizedLevel::FULL,
    };
    let stages = [delay, IrNodeKind::HardClamp];
    let old = plan_with_global_stages(profile, 0.8, crate::ir::StealingPolicy::None, &stages[..1]);
    let source_plan = old.clone();
    let program: Vec<_> = events(&old)
        .into_iter()
        .filter(|event| matches!(event.payload(), CompiledPayload::NoteOn { .. }))
        .collect();
    let (_, mut reference) = start(old.clone(), &program, profile);
    let (mut control, mut audio) = start(old, &program, profile);
    let table = audio.origin.table;
    for index in 0..128 {
        let count = if index % 2 == 0 { 2 } else { 1 };
        let next = plan_with_global_stages(
            profile,
            0.8,
            crate::ir::StealingPolicy::None,
            &stages[..count],
        );
        let stream = AdmittedCompiledStream::admit(
            &next,
            &program
                .iter()
                .map(|event| {
                    let payload = rebind_program(event.payload(), &source_plan, &next).unwrap();
                    PlanEvent::new(event.position(), payload)
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
        assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
        let mut owner = None;
        assert_eq!(
            crate::render_allocation::count_allocs(
                || owner = Some(audio.begin_running_replacement(candidate).unwrap())
            ),
            0
        );
        let mut owner = owner.unwrap();
        let mut samples = [0.0; 128];
        assert_eq!(
            crate::render_allocation::count_allocs(|| audio
                .render_running_replacement(
                    &mut owner,
                    AudioBlockMut::new(&mut samples, 64, ChannelLayout::Stereo).unwrap()
                )
                .unwrap()),
            0
        );
        assert_eq!(
            samples.as_slice(),
            render(&mut reference, 64).as_slice(),
            "edit={index}"
        );
        let receipt = control.collect_replacement(owner).unwrap();
        assert!(matches!(
            receipt.outcome,
            PlanReplacementOutcome::Installed(_)
        ));
        assert!(!control.has_replacements());
        assert!(!control.has_outstanding());
        assert_eq!(audio.origin.table, table);
        assert_eq!(control.origin.table, table);
        assert_eq!(audio.publication_faults(), 0);
    }
    let boundary = audio.clock();
    audio
        .enqueue(control.prepare_stop(boundary).unwrap())
        .unwrap();
    let _stopped = render(&mut audio, 64);
    let _receipt = control.collect(audio.take_completed().unwrap()).unwrap();
    assert!(!control.has_outstanding());
    // Stop holds dormant compiled state; its two identities stay owned until
    // activation or cold teardown, rather than disappearing during an edit.
    assert_eq!(audio.renderer.readmission_retired_notes().get(), 2);
}

#[test]
fn structural_voice_extent_refusal_cannot_partially_transfer_global_state() {
    let profile = profile(48_000.0);
    let delay = IrNodeKind::Delay {
        time_left: crate::quantities::DelayTime::new(0.001).unwrap(),
        time_right: crate::quantities::DelayTime::new(0.002).unwrap(),
        feedback: crate::quantities::DelayFeedback::new(0.4).unwrap(),
        mix: NormalizedLevel::new(0.4).unwrap(),
        tone: NormalizedLevel::FULL,
    };
    let old = plan_with_global_stages(profile, 0.8, crate::ir::StealingPolicy::None, &[delay]);
    let program = events(&old);
    let (_, mut reference) = start(old.clone(), &program, profile);
    let (mut control, mut audio) = start(old, &program, profile);
    let origin = audio.origin;
    let next = plan_with_global_stages(
        profile,
        0.8,
        crate::ir::StealingPolicy::None,
        &[delay, IrNodeKind::HardClamp],
    );
    let mut candidate = prepare(&mut control, &audio, next, profile);
    let structural = candidate
        .continuation
        .as_mut()
        .unwrap()
        .proof
        .structural
        .as_mut()
        .unwrap();
    structural.voice_nodes[0].history += 1;
    let candidate = audio.begin_running_replacement(candidate).unwrap();
    assert!(matches!(
        candidate.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::Continuation(ContinuationError::Storage)
        ))
    ));
    assert_eq!(audio.origin, origin);
    assert_eq!(render(&mut audio, 512), render(&mut reference, 512));
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Refused(_)
    ));
    assert!(!control.has_replacements());
}

#[test]
fn global_automation_and_its_stamped_witness_follow_a_shifted_stable_parameter_address() {
    let profile = profile(48_000.0);
    let delay = IrNodeKind::Delay {
        time_left: crate::quantities::DelayTime::new(0.001).unwrap(),
        time_right: crate::quantities::DelayTime::new(0.002).unwrap(),
        feedback: crate::quantities::DelayFeedback::new(0.4).unwrap(),
        mix: NormalizedLevel::new(0.4).unwrap(),
        tone: NormalizedLevel::FULL,
    };
    let old = plan_with_keyed_global_stages(
        profile,
        0.8,
        crate::ir::StealingPolicy::None,
        &[(NodeId::new(6), delay, ExecutionScope::Global)],
    );
    let old_slot = old
        .resolve_parameter(NodeId::new(6), crate::ir::parameters::DELAY_MIX)
        .unwrap();
    let mut program = events(&old);
    program.push(PlanEvent::new(
        PlanPosition::new(320),
        CompiledPayload::SetParameter {
            slot: old_slot,
            value: ParameterValue::new(0.7).unwrap(),
        },
    ));
    program.sort_by_key(|event| event.position());
    let (_, mut reference) = start(old.clone(), &program, profile);
    let (mut control, mut audio) = start(old.clone(), &program, profile);
    let new = plan_with_keyed_global_stages(
        profile,
        0.8,
        crate::ir::StealingPolicy::None,
        &[
            (
                NodeId::new(7),
                IrNodeKind::Trim {
                    level: Amplitude::UNITY,
                },
                ExecutionScope::Global,
            ),
            (NodeId::new(6), delay, ExecutionScope::Global),
        ],
    );
    let shifted = new
        .resolve_parameter(NodeId::new(6), crate::ir::parameters::DELAY_MIX)
        .unwrap();
    assert_ne!(old_slot.index(), shifted.index());
    let new_program: Vec<_> = program
        .iter()
        .map(|event| {
            PlanEvent::new(
                event.position(),
                rebind_program(event.payload(), &old, &new).unwrap(),
            )
        })
        .collect();
    let stream = AdmittedCompiledStream::admit(&new, &new_program).unwrap();
    let candidate = prepare_admitted(&mut control, &audio, new, stream, profile);
    let actual = install_and_render(&mut control, &mut audio, candidate);
    assert_eq!(actual, render(&mut reference, 64));
    assert_eq!(render(&mut audio, 512), render(&mut reference, 512));
}

#[test]
fn unsupported_shape_precedes_limits_and_shifted_strip_rows_obey_them() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let changed = plan_with_global_stage(
        profile,
        0.7,
        crate::ir::StealingPolicy::None,
        Some(IrNodeKind::HardClamp),
    );
    assert!(matches!(
        old.prepare_continuation(
            &changed,
            witness_limits().transition_bytes,
            crate::quantities::RecordCount::measured(0)
        ),
        Err(ContinuationError::Plan)
    ));
    let strip = IrNodeKind::Channel {
        fader: Amplitude::UNITY,
        pan: crate::controller::BipolarLevel::new(0.0).unwrap(),
        muted: false,
    };
    let scope = ExecutionScope::Channel(crate::ir::ChannelTag::new(1));
    let old = plan_with_keyed_global_stages(
        profile,
        0.8,
        crate::ir::StealingPolicy::None,
        &[(NodeId::new(6), strip, scope)],
    );
    let added = IrNodeKind::Delay {
        time_left: crate::quantities::DelayTime::new(0.001).unwrap(),
        time_right: crate::quantities::DelayTime::new(0.002).unwrap(),
        feedback: crate::quantities::DelayFeedback::new(0.4).unwrap(),
        mix: NormalizedLevel::new(0.4).unwrap(),
        tone: NormalizedLevel::FULL,
    };
    let new = plan_with_keyed_global_stages(
        profile,
        0.8,
        crate::ir::StealingPolicy::None,
        &[
            (NodeId::new(7), added, scope),
            (NodeId::new(6), strip, scope),
        ],
    );
    let new_strip = new.channels()[0];
    assert!(new_strip.mute.index() >= old.parameter_targets().len());
    assert!(matches!(
        old.prepare_continuation(
            &new,
            witness_limits().transition_bytes,
            crate::quantities::RecordCount::measured(0)
        ),
        Err(ContinuationError::RecordLimit)
    ));
    let program = events(&old);
    let (mut control, audio) = start(old, &program, profile);
    let candidate = prepare(&mut control, &audio, new, profile);
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
    let receipt = control.cancel_replacement(candidate).unwrap();
    assert_eq!(receipt.outcome, PlanReplacementOutcome::Cancelled);
}

#[test]
fn a_voice_parameter_row_cannot_be_relabelled_as_a_group_slot() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let new = plan(profile, 0.8);
    let group = old
        .parameter_addresses()
        .iter()
        .find(|address| {
            old.parameter_targets()[address.slot.index()]
                .instances
                .get()
                > 1
        })
        .unwrap()
        .slot;
    let interior = ParameterSlot::new(old.id(), group.index() + 1);
    assert_eq!(
        parameter_slot(interior, &old, &new),
        Err(ContinuationError::Binding)
    );
    let resolved = parameter_slot(group, &old, &new).unwrap();
    assert!(
        new.parameter_addresses()
            .iter()
            .any(|address| address.slot == resolved)
    );
}
fn events(plan: &CompiledPlan) -> Vec<PlanEvent> {
    let slot = plan.resolve_note(NodeId::new(2)).unwrap();
    [
        (0, 60, true),
        (384, 72, true),
        (768, 60, false),
        (1024, 72, false),
    ]
    .map(|(position, key, on)| {
        PlanEvent::new(
            PlanPosition::new(position),
            if on {
                CompiledPayload::NoteOn {
                    slot,
                    key: KeyIdentity::new(key).unwrap(),
                    velocity: NoteVelocity::FULL,
                }
            } else {
                CompiledPayload::NoteOff {
                    slot,
                    key: KeyIdentity::new(key).unwrap(),
                }
            },
        )
    })
    .to_vec()
}
fn witness_limits() -> ContinuationLimits {
    ContinuationLimits {
        maximum_transition_records: crate::quantities::RecordCount::measured(16_384),
        maximum_stamped_events: EventCount::limit(16_384).unwrap(),
        witness_bytes: PreparedBytes::limit(4 * 1024 * 1024).unwrap(),
        transition_bytes: PreparedBytes::limit(4 * 1024 * 1024).unwrap(),
    }
}
fn start(
    plan: CompiledPlan,
    events: &[PlanEvent],
    profile: HostProfile,
) -> (SessionControl, SessionAudio) {
    let stream = AdmittedCompiledStream::admit(&plan, events).unwrap();
    let (mut control, mut audio) =
        SessionControl::prepare(plan, stream, profile, limits()).unwrap();
    let mut play = control.prepare_play(SampleTime::ZERO).unwrap();
    play.prepare_continuation_witness(witness_limits()).unwrap();
    audio.enqueue(play).unwrap();
    render(&mut audio, 256);
    let receipt = control.collect(audio.take_completed().unwrap()).unwrap();
    assert!(matches!(receipt.outcome, SessionOutcome::Applied { .. }));
    (control, audio)
}
fn render(audio: &mut SessionAudio, frames: usize) -> Vec<f32> {
    let mut output = vec![0.0; frames * 2];
    assert_eq!(
        crate::render_allocation::count_allocs(|| audio
            .render(AudioBlockMut::new(&mut output, frames, ChannelLayout::Stereo).unwrap())
            .unwrap()),
        0
    );
    output
}
fn prepare(
    control: &mut SessionControl,
    audio: &SessionAudio,
    plan: CompiledPlan,
    profile: HostProfile,
) -> Box<PreparedPlanReplacement> {
    let stream = AdmittedCompiledStream::admit(&plan, &events(&plan)).unwrap();
    prepare_admitted(control, audio, plan, stream, profile)
}
fn prepare_admitted(
    control: &mut SessionControl,
    audio: &SessionAudio,
    plan: CompiledPlan,
    stream: AdmittedCompiledStream,
    profile: HostProfile,
) -> Box<PreparedPlanReplacement> {
    control
        .prepare_running_replacement(
            plan,
            stream,
            RunningReplacementRequest {
                profile,
                transfer: limits(),
                song_end: None,
                maximum_timeline_events: EventCount::limit(16_384).unwrap(),
                mode: RunningEditMode::ContinueOrReset(witness_limits()),
            },
            |_| Some(audio.clock()),
        )
        .unwrap()
}

#[test]
fn held_voice_recompile_matches_a_single_plans_sample_exact_trim_change() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        for partitions in [&[1][..], &[64], &[256], &[512], &[7, 19, 3, 111]] {
            let old = plan(profile, 0.8);
            let original = events(&old);
            let mut reference_events = original.clone();
            reference_events.push(PlanEvent::new(
                PlanPosition::new(192),
                CompiledPayload::SetParameter {
                    slot: old
                        .resolve_parameter(TRIM, crate::ir::parameters::TRIM_LEVEL)
                        .unwrap(),
                    value: ParameterValue::new(0.4).unwrap(),
                },
            ));
            reference_events.sort_by_key(|event| event.position());
            let (_, mut reference) = start(old.clone(), &reference_events, profile);
            let (mut control, mut audio) = start(old, &original, profile);
            let table = audio.origin.table;
            let epoch = audio.epoch();
            let generation = control.generation();
            let candidate = prepare(&mut control, &audio, plan(profile, 0.4), profile);
            assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
            let mut installed = None;
            assert_eq!(
                crate::render_allocation::count_allocs(
                    || installed = Some(audio.begin_running_replacement(candidate).unwrap())
                ),
                0
            );
            let mut candidate = installed.unwrap();
            assert!(candidate.outcome().is_none());
            assert_eq!(audio.origin.table, table);
            assert_eq!(audio.epoch(), epoch);
            let mut actual = Vec::new();
            let mut expected = Vec::new();
            let mut cursor = 0;
            while actual.len() < 1280 * 2 {
                let n = partitions[cursor % partitions.len()].min(1280 - actual.len() / 2);
                let mut output = vec![0.0; n * 2];
                if candidate.outcome().is_none() {
                    assert_eq!(
                        crate::render_allocation::count_allocs(|| audio
                            .render_running_replacement(
                                &mut candidate,
                                AudioBlockMut::new(&mut output, n, ChannelLayout::Stereo).unwrap()
                            )
                            .unwrap()),
                        0
                    );
                } else {
                    output = render(&mut audio, n);
                }
                actual.extend(output);
                expected.extend(render(&mut reference, n));
                cursor += 1;
            }
            assert!(actual.iter().any(|value| *value != 0.0));
            assert_eq!(
                actual
                    .iter()
                    .map(|sample| sample.to_bits())
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|sample| sample.to_bits())
                    .collect::<Vec<_>>(),
                "rate={rate}, partitions={partitions:?}"
            );
            assert!(
                actual[2000..].iter().all(|value| *value == 0.0),
                "old releases still end their own continued voices"
            );
            let receipt = control.collect_replacement(candidate).unwrap();
            assert!(matches!(
                receipt.outcome,
                PlanReplacementOutcome::Installed(_)
            ));
            assert_eq!(control.generation(), generation);
            assert_eq!(control.origin.table, table);
            assert_eq!(receipt.retired_compiled_notes.get(), 0);
            assert!(!control.has_replacements());
            assert_eq!(audio.publication_faults(), 0);
            assert!(matches!(
                control.snapshot().playback,
                PlaybackState::Playing(_)
            ));
        }
    }
}

fn install_and_render(
    control: &mut SessionControl,
    audio: &mut SessionAudio,
    candidate: Box<PreparedPlanReplacement>,
) -> Vec<f32> {
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
    let mut candidate = audio.begin_running_replacement(candidate).unwrap();
    let mut output = [0.0; 128];
    assert_eq!(
        crate::render_allocation::count_allocs(|| audio
            .render_running_replacement(
                &mut candidate,
                AudioBlockMut::new(&mut output, 64, ChannelLayout::Stereo).unwrap()
            )
            .unwrap()),
        0
    );
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Installed(_)
    ));
    output.to_vec()
}

#[test]
fn repeated_continuations_replace_the_witness_and_keep_the_actual_authority() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let mut original = events(&old);
    let _unpaired = original.pop().unwrap();
    let mut reference_events = original.clone();
    let slot = old
        .resolve_parameter(TRIM, crate::ir::parameters::TRIM_LEVEL)
        .unwrap();
    for (at, value) in [(192, 0.4), (256, 0.6), (320, 0.2)] {
        reference_events.push(PlanEvent::new(
            PlanPosition::new(at),
            CompiledPayload::SetParameter {
                slot,
                value: ParameterValue::new(value).unwrap(),
            },
        ));
    }
    reference_events.sort_by_key(|event| event.position());
    let (_, mut reference) = start(old.clone(), &reference_events, profile);
    let (mut control, mut audio) = start(old, &original, profile);
    let table = control.origin.table;
    let generation = control.generation();
    for level in [0.4, 0.6, 0.2] {
        let next = plan(profile, level);
        let next_id = next.id();
        let mut next_events = events(&next);
        let _unpaired = next_events.pop().unwrap();
        let stream = AdmittedCompiledStream::admit(&next, &next_events).unwrap();
        let candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
        let actual = install_and_render(&mut control, &mut audio, candidate);
        assert_eq!(actual, render(&mut reference, 64));
        assert_eq!(control.plan_id(), next_id);
        assert_eq!(control.active_timeline.as_ref().unwrap().plan, next_id);
        assert_eq!(control.origin.table, table);
        assert_eq!(control.generation(), generation);
        let note_count = control.control.plan().note_targets().len();
        assert!(
            control
                .control
                .minter_mut()
                .can_continue_plan(next_id, note_count)
        );
    }
    let stop = control.prepare_stop(audio.clock()).unwrap();
    audio.enqueue(stop).unwrap();
    render(&mut audio, 64);
    let _stopped = control.collect(audio.take_completed().unwrap()).unwrap();
    let mut play = control.prepare_play(audio.clock()).unwrap();
    play.prepare_continuation_witness(witness_limits()).unwrap();
    audio.enqueue(play).unwrap();
    render(&mut audio, 64);
    let played = control.collect(audio.take_completed().unwrap()).unwrap();
    assert!(matches!(played.outcome, SessionOutcome::Applied { .. }));
    assert_eq!(control.origin.table, table);
    assert!(
        control
            .active_timeline
            .as_ref()
            .unwrap()
            .events
            .iter()
            .all(|event| match event.payload() {
                EventPayload::SetParameter { slot, .. } => slot.plan() == control.plan_id(),
                EventPayload::Note {
                    edge: NoteEdge::On { slot, .. },
                    ..
                } => slot.plan() == control.plan_id(),
                _ => true,
            })
    );
    assert_eq!(audio.publication_faults(), 0);
}

#[test]
fn changed_notes_or_an_automated_trim_choose_reset_and_keep_the_old_pair_on_cancel() {
    for automation in [false, true] {
        let profile = profile(48_000.0);
        let old = plan(profile, 0.8);
        let mut original = events(&old);
        if automation {
            original.push(PlanEvent::new(
                PlanPosition::new(32),
                CompiledPayload::SetParameter {
                    slot: old
                        .resolve_parameter(TRIM, crate::ir::parameters::TRIM_LEVEL)
                        .unwrap(),
                    value: ParameterValue::new(0.9).unwrap(),
                },
            ));
            original.sort_by_key(|event| event.position());
        }
        let (mut control, audio) = start(old, &original, profile);
        let next = plan(profile, 0.4);
        let mut updated = events(&next);
        if automation {
            updated.push(PlanEvent::new(
                PlanPosition::new(32),
                CompiledPayload::SetParameter {
                    slot: next
                        .resolve_parameter(TRIM, crate::ir::parameters::TRIM_LEVEL)
                        .unwrap(),
                    value: ParameterValue::new(0.9).unwrap(),
                },
            ));
            updated.sort_by_key(|event| event.position());
        } else {
            updated[1] = PlanEvent::new(PlanPosition::new(400), updated[1].payload());
        }
        let stream = AdmittedCompiledStream::admit(&next, &updated).unwrap();
        let candidate = control
            .prepare_running_replacement(
                next,
                stream,
                RunningReplacementRequest {
                    profile,
                    transfer: limits(),
                    song_end: None,
                    maximum_timeline_events: EventCount::limit(16_384).unwrap(),
                    mode: RunningEditMode::ContinueOrReset(witness_limits()),
                },
                |_| Some(audio.clock()),
            )
            .unwrap();
        assert_eq!(candidate.kind(), PlanReplacementKind::RunningReset);
        let _cancelled = control.cancel_replacement(candidate).unwrap();
        assert_eq!(control.origin.plan, audio.origin.plan);
        assert_eq!(control.origin.table, audio.origin.table);
        assert!(!control.has_replacements());
    }
}

#[test]
fn an_empty_first_call_keeps_custody_and_waits_for_actual_audio() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let (mut control, mut audio) = start(old.clone(), &events(&old), profile);
    let candidate = prepare(&mut control, &audio, plan(profile, 0.4), profile);
    let mut candidate = audio.begin_running_replacement(candidate).unwrap();
    let clock = audio.clock();
    let mut empty = [];
    assert_eq!(
        crate::render_allocation::count_allocs(|| audio
            .render_running_replacement(
                &mut candidate,
                AudioBlockMut::new(&mut empty, 0, ChannelLayout::Stereo).unwrap()
            )
            .unwrap()),
        0
    );
    assert!(candidate.outcome().is_none());
    assert!(candidate.is_rendering_transition());
    assert_eq!(audio.clock(), clock);
    let (candidate, _) = control.cancel_replacement(candidate).unwrap_err();
    let (mut candidate, _) = control.collect_replacement(candidate).unwrap_err();
    let mut output = [0.0; 128];
    audio
        .render_running_replacement(
            &mut candidate,
            AudioBlockMut::new(&mut output, 64, ChannelLayout::Stereo).unwrap(),
        )
        .unwrap();
    assert!(output.iter().any(|sample| *sample != 0.0));
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Installed(_)
    ));
}

#[test]
fn bypassing_continuation_render_is_terminal_and_collection_keeps_the_actual_new_binding() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let (mut control, mut audio) = start(old.clone(), &events(&old), profile);
    let next = plan(profile, 0.4);
    let new_id = next.id();
    let candidate = prepare(&mut control, &audio, next, profile);
    let mut candidate = audio.begin_running_replacement(candidate).unwrap();
    let mut output = [9.0; 128];
    assert!(
        audio
            .render(AudioBlockMut::new(&mut output, 64, ChannelLayout::Stereo).unwrap())
            .is_err()
    );
    assert!(output.iter().all(|sample| *sample == 0.0));
    assert!(
        audio
            .render_running_replacement(
                &mut candidate,
                AudioBlockMut::new(&mut output, 64, ChannelLayout::Stereo).unwrap()
            )
            .is_err()
    );
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Interrupted(_)
    ));
    assert_eq!(control.plan_id(), new_id);
    assert_eq!(control.control.plan().id(), new_id);
    assert!(control.closed);
    assert!(control.prepare_stop(audio.clock()).is_err());
}

#[test]
fn stale_witness_is_invalidated_and_the_next_edit_uses_reset() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let (mut control, audio) = start(old.clone(), &events(&old), profile);
    control.active_timeline.as_mut().unwrap().anchor =
        StreamAnchor::new(SampleTime::new(64), PlanPosition::new(64));
    let candidate = prepare(&mut control, &audio, plan(profile, 0.4), profile);
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningReset);
    assert!(control.active_timeline.is_none());
    let _cancelled = control.cancel_replacement(candidate).unwrap();
}

#[test]
fn authoritative_minter_bindings_are_checked_before_publication() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let old_id = old.id();
    let mut original = events(&old);
    let _unpaired = original.pop().unwrap();
    let (mut control, mut audio) = start(old, &original, profile);
    let next = plan(profile, 0.4);
    let mut next_events = events(&next);
    let _unpaired = next_events.pop().unwrap();
    let stream = AdmittedCompiledStream::admit(&next, &next_events).unwrap();
    // Corrupt only private test authority, leaving its real timeline/renderer untouched.
    let forward = crate::plan::note_binding::NoteBindings::prepare(
        control.control.plan(),
        &next,
        crate::plan::note_binding::NoteBindingScope::Exact,
        witness_limits().transition_bytes,
    )
    .unwrap();
    let reverse = crate::plan::note_binding::NoteBindings::prepare(
        &next,
        control.control.plan(),
        crate::plan::note_binding::NoteBindingScope::Exact,
        witness_limits().transition_bytes,
    )
    .unwrap();
    control
        .control
        .minter_mut()
        .continue_note_bindings(&forward);
    let serial = control.publication_serial;
    let result = control.prepare_running_replacement(
        next,
        stream,
        RunningReplacementRequest {
            profile,
            transfer: limits(),
            song_end: None,
            maximum_timeline_events: EventCount::limit(16_384).unwrap(),
            mode: RunningEditMode::ContinueOrReset(witness_limits()),
        },
        |_| Some(audio.clock()),
    );
    assert!(matches!(
        result,
        Err(
            crate::host::session::transfer::replacement::PlanReplacementError::Continuation(
                ContinuationError::Storage
            )
        )
    ));
    assert!(!control.has_replacements());
    assert_eq!(control.publication_serial, serial);
    assert_eq!(audio.plan_id(), old_id);
    assert_eq!(control.plan_id(), old_id);
    control
        .control
        .minter_mut()
        .continue_note_bindings(&reverse);
    assert!(render(&mut audio, 64).iter().any(|sample| *sample != 0.0));
}

#[test]
fn failed_witness_capture_keeps_the_original_play_packet_usable() {
    for too_few_events in [false, true] {
        let profile = profile(48_000.0);
        let plan = plan(profile, 0.8);
        let stream = AdmittedCompiledStream::admit(&plan, &events(&plan)).unwrap();
        let (mut control, mut audio) =
            SessionControl::prepare(plan, stream, profile, limits()).unwrap();
        let mut play = control.prepare_play(SampleTime::ZERO).unwrap();
        let mut limits = witness_limits();
        if too_few_events {
            limits.maximum_stamped_events = EventCount::limit(1).unwrap();
        } else {
            limits.witness_bytes = PreparedBytes::limit(1).unwrap();
        }
        assert!(play.prepare_continuation_witness(limits).is_err());
        audio.enqueue(play).unwrap();
        render(&mut audio, 256);
        let applied = control.collect(audio.take_completed().unwrap()).unwrap();
        assert!(matches!(applied.outcome, SessionOutcome::Applied { .. }));
        assert!(control.active_timeline.is_none());
        assert!(!control.has_outstanding());
    }
}

#[test]
fn a_bend_and_an_in_progress_steal_continue_through_the_recompiled_plan() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        let policy = crate::ir::StealingPolicy::Oldest {
            fade: FrameCount::new(8),
        };
        let old = plan_with_stealing(profile, 0.8, policy);
        let slot = old.resolve_note(NodeId::new(2)).unwrap();
        let key = |key| KeyIdentity::new(key).unwrap();
        let program = [
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(64),
                CompiledPayload::NoteOn {
                    slot,
                    key: key(62),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(188),
                CompiledPayload::NoteOn {
                    slot,
                    key: key(64),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(256),
                CompiledPayload::Bend {
                    slot,
                    key: key(64),
                    cents: crate::quantities::Cents::new(1200.0).unwrap(),
                },
            ),
            PlanEvent::new(
                PlanPosition::new(768),
                CompiledPayload::NoteOff { slot, key: key(62) },
            ),
            PlanEvent::new(
                PlanPosition::new(1024),
                CompiledPayload::NoteOff { slot, key: key(64) },
            ),
        ];
        let mut reference_program = program.to_vec();
        reference_program.push(PlanEvent::new(
            PlanPosition::new(192),
            CompiledPayload::SetParameter {
                slot: old
                    .resolve_parameter(TRIM, crate::ir::parameters::TRIM_LEVEL)
                    .unwrap(),
                value: ParameterValue::new(0.4).unwrap(),
            },
        ));
        reference_program.sort_by_key(|event| event.position());
        let (_, mut reference) = start(old.clone(), &reference_program, profile);
        let (mut control, mut audio) = start(old, &program, profile);
        let next = plan_with_stealing(profile, 0.4, policy);
        let mapped: Vec<_> = program
            .iter()
            .map(|event| {
                PlanEvent::new(
                    event.position(),
                    super::rebind_program(event.payload(), control.control.plan(), &next).unwrap(),
                )
            })
            .collect();
        let stream = AdmittedCompiledStream::admit(&next, &mapped).unwrap();
        let candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
        assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
        let mut candidate = audio.begin_running_replacement(candidate).unwrap();
        for frames in [1, 17, 64, 256, 512, 430] {
            let mut actual = vec![0.0; frames * 2];
            if candidate.outcome().is_none() {
                assert_eq!(
                    crate::render_allocation::count_allocs(|| audio
                        .render_running_replacement(
                            &mut candidate,
                            AudioBlockMut::new(&mut actual, frames, ChannelLayout::Stereo).unwrap()
                        )
                        .unwrap()),
                    0
                );
            } else {
                actual = render(&mut audio, frames);
            }
            let expected = render(&mut reference, frames);
            assert_eq!(
                actual
                    .iter()
                    .map(|sample| sample.to_bits())
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|sample| sample.to_bits())
                    .collect::<Vec<_>>(),
                "rate={rate}, frames={frames}"
            );
        }
        let receipt = control.collect_replacement(candidate).unwrap();
        assert!(matches!(
            receipt.outcome,
            PlanReplacementOutcome::Installed(_)
        ));
        assert_eq!(audio.publication_faults(), 0);
    }
}

#[test]
fn witness_limits_measure_stamped_steal_expansion_not_just_semantic_events() {
    let profile = profile(48_000.0);
    let plan = plan_with_stealing(
        profile,
        0.8,
        crate::ir::StealingPolicy::Oldest {
            fade: FrameCount::new(8),
        },
    );
    let slot = plan.resolve_note(NodeId::new(2)).unwrap();
    let events: Vec<_> = [60, 62, 64]
        .into_iter()
        .enumerate()
        .map(|(position, key)| {
            PlanEvent::new(
                PlanPosition::new(u64::try_from(position).unwrap()),
                CompiledPayload::NoteOn {
                    slot,
                    key: KeyIdentity::new(key).unwrap(),
                    velocity: NoteVelocity::FULL,
                },
            )
        })
        .collect();
    let stream = AdmittedCompiledStream::admit(&plan, &events).unwrap();
    let (mut control, _audio) = SessionControl::prepare(plan, stream, profile, limits()).unwrap();
    let mut packet = control.prepare_play(SampleTime::ZERO).unwrap();
    assert!(packet.entry.activation.as_ref().unwrap().events.len() > events.len());
    let mut budget = witness_limits();
    budget.maximum_stamped_events = EventCount::limit(3).unwrap();
    assert_eq!(
        packet.prepare_continuation_witness(budget),
        Err(ContinuationError::EventLimit)
    );
    let _cancelled = control.cancel(packet).unwrap();
    assert!(!control.has_outstanding());
}

#[test]
fn equal_time_note_order_changes_choose_reset() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let mut original = events(&old);
    original[1] = PlanEvent::new(PlanPosition::ZERO, original[1].payload());
    let (mut control, audio) = start(old, &original, profile);
    let next = plan(profile, 0.4);
    let mut reordered = events(&next);
    reordered[1] = PlanEvent::new(PlanPosition::ZERO, reordered[1].payload());
    reordered.swap(0, 1);
    let stream = AdmittedCompiledStream::admit(&next, &reordered).unwrap();
    let candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningReset);
    let _cancelled = control.cancel_replacement(candidate).unwrap();
}

#[test]
fn a_foreign_slot_in_any_witness_payload_cannot_be_relabelled_by_index_alone() {
    for note in [false, true] {
        let profile = profile(48_000.0);
        let old = plan(profile, 0.8);
        let (mut control, audio) = start(old.clone(), &events(&old), profile);
        let foreign = plan(profile, 0.8);
        let witness = control.active_timeline.as_mut().unwrap();
        let first = witness.events[0];
        let payload = if note {
            let EventPayload::Note {
                identity,
                edge: NoteEdge::On { key, velocity, .. },
            } = first.payload()
            else {
                panic!("first onset");
            };
            EventPayload::Note {
                identity,
                edge: NoteEdge::On {
                    slot: foreign.resolve_note(NodeId::new(2)).unwrap(),
                    key,
                    velocity,
                },
            }
        } else {
            EventPayload::SetParameter {
                slot: foreign
                    .resolve_parameter(TRIM, crate::ir::parameters::TRIM_LEVEL)
                    .unwrap(),
                value: ParameterValue::ONE,
            }
        };
        witness.events[0] = TimedEvent::new(first.envelope(), payload);
        let next = plan(profile, 0.4);
        let stream = AdmittedCompiledStream::admit(&next, &events(&next)).unwrap();
        let result = control.prepare_running_replacement(
            next,
            stream,
            RunningReplacementRequest {
                profile,
                transfer: limits(),
                song_end: None,
                maximum_timeline_events: EventCount::limit(16_384).unwrap(),
                mode: RunningEditMode::ContinueOrReset(witness_limits()),
            },
            |_| Some(audio.clock()),
        );
        assert!(matches!(
            result,
            Err(
                crate::host::session::transfer::replacement::PlanReplacementError::Continuation(
                    ContinuationError::Binding
                )
            )
        ));
        assert_eq!(audio.plan_id(), old.id());
        assert!(!control.has_replacements());
    }
}

#[test]
fn a_changed_transport_capacity_selects_a_fresh_owner() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let (mut control, audio) = start(old.clone(), &events(&old), profile);
    let next = plan(profile, 0.4);
    let stream = AdmittedCompiledStream::admit(&next, &events(&next)).unwrap();
    let mut changed = limits();
    changed.session.commands = crate::host::session::SessionCommandCapacity::new(3).unwrap();
    let candidate = control
        .prepare_running_replacement(
            next,
            stream,
            RunningReplacementRequest {
                profile,
                transfer: changed,
                song_end: None,
                maximum_timeline_events: EventCount::limit(16_384).unwrap(),
                mode: RunningEditMode::ContinueOrReset(witness_limits()),
            },
            |_| Some(audio.clock()),
        )
        .unwrap();
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningReset);
    let _cancelled = control.cancel_replacement(candidate).unwrap();
}

#[test]
fn optional_reset_witness_failure_is_reported_without_disabling_the_reset() {
    let profile = profile(48_000.0);
    let old = plan(profile, 0.8);
    let (mut control, mut audio) = start(old.clone(), &events(&old), profile);
    let next = plan(profile, 0.4);
    let stream = AdmittedCompiledStream::admit(&next, &events(&next)).unwrap();
    let mut budget = witness_limits();
    budget.witness_bytes = PreparedBytes::limit(1).unwrap();
    let candidate = control
        .prepare_running_replacement(
            next,
            stream,
            RunningReplacementRequest {
                profile,
                transfer: limits(),
                song_end: None,
                maximum_timeline_events: EventCount::limit(16_384).unwrap(),
                mode: RunningEditMode::ResetWithWitness(budget),
            },
            |_| Some(audio.clock()),
        )
        .unwrap();
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningReset);
    assert_eq!(
        candidate.continuation_unavailable(),
        Some(ContinuationError::Bytes)
    );
    let mut candidate = audio.begin_running_replacement(candidate).unwrap();
    let mut output = [0.0; 128];
    audio
        .render_running_replacement(
            &mut candidate,
            AudioBlockMut::new(&mut output, 64, ChannelLayout::Stereo).unwrap(),
        )
        .unwrap();
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Installed(_)
    ));
    assert!(control.active_timeline.is_none());
}

#[test]
fn observed_structural_continuation_keeps_held_voice_sound_and_later_releases() {
    use crate::{
        host::session::transfer::replacement::ObservationReplacementOutcome,
        observe::ObservationSubscriptions,
    };
    let profile = profile(48_000.0);
    for partition in [1, 37, 64, 511] {
        let old = plan_with_global_stages(
            profile,
            0.8,
            crate::ir::StealingPolicy::None,
            &[IrNodeKind::StereoMonitor],
        );
        let next = plan_with_global_stages(
            profile,
            0.8,
            crate::ir::StealingPolicy::None,
            &[IrNodeKind::StereoMonitor, IrNodeKind::HardClamp],
        );
        let mut store = ObservationSubscriptions::prepare(&profile, &old).unwrap();
        let handle = store
            .subscribe(
                &old,
                old.resolve_tap(NodeId::new(6), PortId::FIRST).unwrap(),
            )
            .unwrap();
        let budget = PreparedBytes::measured(4096);
        let handoff = store
            .configuration(budget)
            .unwrap()
            .prepare_handoff(&old, &next, budget)
            .unwrap();
        let program = events(&old);
        let (_, mut reference) = start(old.clone(), &program, profile);
        let (mut control, mut audio) = start(old, &program, profile);
        let table = audio.origin.table;
        let epoch = audio.epoch();
        let mut candidate = prepare(&mut control, &audio, next, profile);
        assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
        candidate.attach_observation(Ok(handoff).into()).unwrap();
        let mut returned = None;
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                returned = Some(
                    audio
                        .begin_running_replacement_observed(candidate, &mut store)
                        .unwrap(),
                );
            }),
            0
        );
        let mut candidate = returned.unwrap();
        let mut sound = Vec::new();
        let mut taps = vec![0.0; 2560];
        let mut total = 0;
        for start in (0..1280).step_by(partition) {
            let frames = partition.min(1280 - start);
            let mut output = vec![0.0; frames * 2];
            assert_eq!(
                crate::render_allocation::count_allocs(|| {
                    let block =
                        AudioBlockMut::new(&mut output, frames, ChannelLayout::Stereo).unwrap();
                    if candidate.outcome().is_none() {
                        audio
                            .render_running_replacement_observed(&mut candidate, block, &mut store)
                            .unwrap();
                    } else {
                        audio.render_observed(block, Some(&mut store)).unwrap();
                    }
                    let read = store.read(handle, &mut taps[total..]).unwrap();
                    total += read.frames.as_usize().unwrap() * 2;
                    assert_eq!(read.generation.as_u64(), 1);
                    assert_eq!(read.dropped.as_u64(), 0);
                    assert!(!read.ended);
                }),
                0
            );
            assert_eq!(output, render(&mut reference, frames));
            sound.extend(output);
        }
        assert_eq!(total, sound.len());
        assert_eq!(taps, sound);
        assert!(sound.iter().any(|sample| *sample != 0.0));
        assert!(sound[2000..].iter().all(|sample| *sample == 0.0));
        assert_eq!(audio.origin.table, table);
        assert_eq!(audio.epoch(), epoch);
        let receipt = control.collect_replacement(candidate).unwrap();
        assert!(matches!(
            receipt.observation,
            ObservationReplacementOutcome::HandedOff { .. }
        ));
    }
}

#[path = "relocation_tests.rs"]
mod relocation_tests;
