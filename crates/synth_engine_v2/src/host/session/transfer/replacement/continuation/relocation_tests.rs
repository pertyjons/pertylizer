//! Actual compiler relocation controls, with an unedited renderer as the audio oracle.
use super::*;

fn relocated_plan(
    profile: HostProfile,
    prefixes: u32,
    stealing: crate::ir::StealingPolicy,
) -> CompiledPlan {
    edited_plan(profile, prefixes, stealing, VoiceEdit::None)
}

#[derive(Clone, Copy)]
enum VoiceEdit {
    None,
    DormantGlobal,
    ConnectedGlobal,
    DetachedGlobal,
    Kernel,
    Identity,
    Routing,
    GlobalInput,
    Destination,
}

fn edited_plan(
    profile: HostProfile,
    prefixes: u32,
    stealing: crate::ir::StealingPolicy,
    edit: VoiceEdit,
) -> CompiledPlan {
    edited_plan_with_observation(profile, prefixes, stealing, edit, false)
}

fn edited_plan_with_observation(
    profile: HostProfile,
    prefixes: u32,
    stealing: crate::ir::StealingPolicy,
    edit: VoiceEdit,
    observed: bool,
) -> CompiledPlan {
    use crate::quantities::{DelayFeedback, DelayTime};
    let delay = NodeId::new(if matches!(edit, VoiceEdit::Identity) {
        41
    } else {
        40
    });
    let voice = ExecutionScope::Voice;
    let global = ExecutionScope::Global;
    let mut graph = GraphIr::builder()
        .node(
            NodeId::new(10),
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).unwrap(),
                amplitude: Amplitude::new(0.5).unwrap(),
            },
            voice,
        )
        .node(
            NodeId::new(20),
            IrNodeKind::Envelope {
                attack: Seconds::new(0.005).unwrap(),
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            voice,
        )
        .node(NodeId::new(30), IrNodeKind::Amplifier, voice)
        .node(
            delay,
            IrNodeKind::Delay {
                time_left: DelayTime::new(0.001).unwrap(),
                time_right: DelayTime::new(0.002).unwrap(),
                feedback: DelayFeedback::new(if matches!(edit, VoiceEdit::Kernel) {
                    0.3
                } else {
                    0.4
                })
                .unwrap(),
                mix: NormalizedLevel::new(0.4).unwrap(),
                tone: NormalizedLevel::FULL,
            },
            voice,
        )
        .node(
            TRIM,
            IrNodeKind::Trim {
                level: Amplitude::new(0.8).unwrap(),
            },
            global,
        )
        .node(
            NodeId::new(60),
            IrNodeKind::Channel {
                fader: Amplitude::new(0.7).unwrap(),
                pan: crate::controller::BipolarLevel::ZERO,
                muted: false,
            },
            ExecutionScope::Channel(crate::ir::ChannelTag::new(1)),
        )
        .node(
            NodeId::new(70),
            IrNodeKind::Mix,
            ExecutionScope::Bus(crate::ir::BusTag::new(1)),
        )
        .node(
            NodeId::new(80),
            IrNodeKind::Channel {
                fader: Amplitude::new(0.9).unwrap(),
                pan: crate::controller::BipolarLevel::ZERO,
                muted: false,
            },
            ExecutionScope::Bus(crate::ir::BusTag::new(1)),
        )
        .node(NodeId::new(50), IrNodeKind::Output, global)
        .connect(
            (
                NodeId::new(if matches!(edit, VoiceEdit::GlobalInput) {
                    200
                } else {
                    10
                }),
                PortId::FIRST,
            ),
            (NodeId::new(30), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (NodeId::new(20), PortId::FIRST),
            (NodeId::new(30), crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (
                NodeId::new(if matches!(edit, VoiceEdit::Routing) {
                    10
                } else {
                    30
                }),
                PortId::FIRST,
            ),
            (delay, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (delay, PortId::FIRST),
            (TRIM, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (TRIM, PortId::FIRST),
            (NodeId::new(60), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (NodeId::new(60), PortId::FIRST),
            (NodeId::new(70), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (NodeId::new(70), PortId::FIRST),
            (NodeId::new(80), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (NodeId::new(80), PortId::FIRST),
            (NodeId::new(if observed { 65 } else { 50 }), PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(
            voice,
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
    if matches!(edit, VoiceEdit::GlobalInput | VoiceEdit::Destination) {
        graph = graph.node(
            NodeId::new(200),
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).unwrap(),
                amplitude: Amplitude::new(0.5).unwrap(),
            },
            if matches!(edit, VoiceEdit::GlobalInput) {
                global
            } else {
                voice
            },
        );
    }
    for index in 0..prefixes {
        graph = graph.node(
            NodeId::new(100 + index),
            IrNodeKind::Trim {
                level: Amplitude::new(0.7).unwrap(),
            },
            global,
        );
    }
    if observed {
        graph = graph
            .node(NodeId::new(65), IrNodeKind::StereoMonitor, global)
            .connect(
                (NodeId::new(65), PortId::FIRST),
                (NodeId::new(50), PortId::FIRST),
                SignalDomain::Audio,
            );
    }
    if matches!(
        edit,
        VoiceEdit::DormantGlobal | VoiceEdit::ConnectedGlobal | VoiceEdit::DetachedGlobal
    ) {
        graph = graph.node(
            NodeId::new(2000),
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            global,
        );
    }
    if matches!(edit, VoiceEdit::ConnectedGlobal | VoiceEdit::DetachedGlobal) {
        graph = graph.node(NodeId::new(2100), IrNodeKind::Amplifier, global);
    }
    if matches!(edit, VoiceEdit::ConnectedGlobal) {
        graph = graph.connect(
            (NodeId::new(2000), PortId::FIRST),
            (NodeId::new(2100), crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        );
    }
    compile(&graph.build().unwrap(), &RenderConfig::new(profile))
        .into_plan()
        .unwrap()
}

fn program(plan: &CompiledPlan) -> Vec<PlanEvent> {
    let slot = plan.resolve_note(NodeId::new(20)).unwrap();
    let key = |key| KeyIdentity::new(key).unwrap();
    let mut result = vec![
        PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::NoteOn {
                slot,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(384),
            CompiledPayload::NoteOn {
                slot,
                key: key(72),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(512),
            CompiledPayload::Bend {
                slot,
                key: key(60),
                cents: crate::quantities::Cents::new(120.0).unwrap(),
            },
        ),
        PlanEvent::new(
            PlanPosition::new(768),
            CompiledPayload::NoteOff { slot, key: key(60) },
        ),
        PlanEvent::new(
            PlanPosition::new(1024),
            CompiledPayload::NoteOff { slot, key: key(72) },
        ),
    ];
    let address = plan
        .parameter_addresses()
        .iter()
        .find(|address| address.node == NodeId::new(10))
        .unwrap();
    for (at, value) in [(64, 330.0), (640, 550.0)] {
        result.push(PlanEvent::new(
            PlanPosition::new(at),
            CompiledPayload::SetParameter {
                slot: address.slot,
                value: ParameterValue::new(value).unwrap(),
            },
        ));
    }
    result.sort_by_key(|event| event.position());
    result
}

#[test]
fn relocated_voice_history_parameters_bends_and_releases_match_unedited_audio() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        for (before, after) in [(0, 2), (2, 0)] {
            for partitions in [&[1][..], &[64], &[256], &[512], &[7, 19, 3, 111]] {
                let old = relocated_plan(profile, before, crate::ir::StealingPolicy::None);
                let events = program(&old);
                let (_, mut reference) = start(old.clone(), &events, profile);
                let (mut control, mut audio) = start(old, &events, profile);
                let origin = audio.origin;
                let next = relocated_plan(profile, after, crate::ir::StealingPolicy::None);
                assert_eq!(next.channels().len(), 1);
                assert_eq!(next.buses().len(), 1);
                assert_ne!(
                    control.control.plan().channels()[0].fader.index(),
                    next.channels()[0].fader.index()
                );
                assert_ne!(
                    control.control.plan().buses()[0].fader.index(),
                    next.buses()[0].fader.index()
                );
                let stream = AdmittedCompiledStream::admit(&next, &program(&next)).unwrap();
                let candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
                assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
                let proof = candidate
                    .continuation
                    .as_ref()
                    .unwrap()
                    .proof
                    .structural
                    .as_ref()
                    .unwrap();
                assert!(proof.voice_nodes.iter().any(|pair| pair.old != pair.new));
                assert!(
                    proof
                        .voice_parameters
                        .iter()
                        .any(|pair| pair.old.index() != pair.new.index())
                );
                assert!(proof.voice_nodes.iter().any(|pair| pair.history > 0));
                let mut owner = None;
                assert_eq!(
                    crate::render_allocation::count_allocs(
                        || owner = Some(audio.begin_running_replacement(candidate).unwrap())
                    ),
                    0
                );
                let mut owner = owner.unwrap();
                let mut cursor = 0;
                let mut frames = 0;
                let mut sounding = false;
                while frames < 2048 {
                    let count = partitions[cursor % partitions.len()].min(2048 - frames);
                    let mut output = vec![0.0; count * 2];
                    assert_eq!(
                        crate::render_allocation::count_allocs(|| {
                            let block =
                                AudioBlockMut::new(&mut output, count, ChannelLayout::Stereo)
                                    .unwrap();
                            if owner.outcome().is_none() {
                                audio.render_running_replacement(&mut owner, block).unwrap();
                            } else {
                                audio.render(block).unwrap();
                            }
                        }),
                        0
                    );
                    sounding |= output.iter().any(|sample| *sample != 0.0);
                    let expected = render(&mut reference, count);
                    assert!(
                        output
                            .iter()
                            .zip(expected)
                            .all(|(a, b)| a.to_bits() == b.to_bits()),
                        "rate={rate}, prefix={before}->{after}, frames={frames}, parts={partitions:?}"
                    );
                    cursor += 1;
                    frames += count;
                }
                assert!(sounding);
                let receipt = control.collect_replacement(owner).unwrap();
                assert!(matches!(
                    receipt.outcome,
                    PlanReplacementOutcome::Installed(_)
                ));
                assert_eq!(receipt.retired_compiled_notes.get(), 0);
                assert_eq!(audio.origin.table, origin.table);
                assert_eq!(control.origin.table, origin.table);
                assert_eq!(audio.epoch(), origin.epoch);
                assert!(!control.has_replacements());
                assert!(!control.has_outstanding());
            }
        }
    }
}

#[test]
fn changed_voice_meaning_refuses_before_the_structural_resource_limit() {
    let profile = profile(48_000.0);
    let old = relocated_plan(profile, 0, crate::ir::StealingPolicy::None);
    for edit in [
        VoiceEdit::Kernel,
        VoiceEdit::Identity,
        VoiceEdit::Routing,
        VoiceEdit::GlobalInput,
        VoiceEdit::Destination,
    ] {
        let next = edited_plan(profile, 2, crate::ir::StealingPolicy::None, edit);
        assert!(matches!(
            old.prepare_continuation(
                &next,
                witness_limits().transition_bytes,
                crate::quantities::RecordCount::measured(0)
            ),
            Err(ContinuationError::Plan)
        ));
    }
    let next = relocated_plan(profile, 2, crate::ir::StealingPolicy::None);
    assert!(matches!(
        old.prepare_continuation(
            &next,
            witness_limits().transition_bytes,
            crate::quantities::RecordCount::measured(0)
        ),
        Err(ContinuationError::RecordLimit)
    ));
    assert!(matches!(
        old.prepare_continuation(
            &next,
            PreparedBytes::limit(1).unwrap(),
            witness_limits().maximum_transition_records
        ),
        Err(ContinuationError::Bytes)
    ));
}

#[test]
fn invalid_relocation_destinations_and_extents_refuse_without_changing_sound_or_identity() {
    let profile = profile(48_000.0);
    for corruption in 0..4 {
        let old = relocated_plan(profile, 0, crate::ir::StealingPolicy::None);
        let events = program(&old);
        let (_, mut reference) = start(old.clone(), &events, profile);
        let (mut control, mut audio) = start(old, &events, profile);
        let origin = audio.origin;
        let next = relocated_plan(profile, 2, crate::ir::StealingPolicy::None);
        let stream = AdmittedCompiledStream::admit(&next, &program(&next)).unwrap();
        let mut candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
        let proof = candidate
            .continuation
            .as_mut()
            .unwrap()
            .proof
            .structural
            .as_mut()
            .unwrap();
        match corruption {
            0 => proof.voice_nodes[0].history += 1,
            1 => proof.voice_nodes[1].new = proof.voice_nodes[0].new,
            2 => proof.voice_parameters[1].new = proof.voice_parameters[0].new,
            _ => proof.voice_nodes.last_mut().unwrap().new = crate::plan::NodeSlot::new(usize::MAX),
        }
        let mut result = None;
        assert_eq!(
            crate::render_allocation::count_allocs(
                || result = Some(audio.begin_running_replacement(candidate).unwrap())
            ),
            0
        );
        let candidate = result.unwrap();
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
        assert_eq!(control.origin.table, origin.table);
        assert!(!control.has_replacements());
    }
}

#[test]
fn repeated_relocations_preserve_the_actual_registry_during_an_in_progress_steal() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        let policy = crate::ir::StealingPolicy::Oldest {
            fade: FrameCount::new(8),
        };
        let old = relocated_plan(profile, 0, policy);
        let slot = old.resolve_note(NodeId::new(20)).unwrap();
        let mut events = program(&old);
        events.extend([(64, 62), (188, 64)].map(|(at, key)| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::NoteOn {
                    slot,
                    key: KeyIdentity::new(key).unwrap(),
                    velocity: NoteVelocity::FULL,
                },
            )
        }));
        events.sort_by_key(|event| event.position());
        let source = old.clone();
        let (_, mut reference) = start(old.clone(), &events, profile);
        let (mut control, mut audio) = start(old, &events, profile);
        let origin = audio.origin;
        // Preparation starts with one quantum of output priming. The 256 served
        // frames have therefore rendered through 192: the steal at 188 has four
        // of its eight fade frames left at the first relocation boundary.
        assert_eq!(audio.clock(), SampleTime::new(192));
        assert!(audio.renderer.continuation_has_active_steal_fade());
        for index in 0..128 {
            let next = relocated_plan(profile, if index % 2 == 0 { 2 } else { 0 }, policy);
            let mapped: Vec<_> = events
                .iter()
                .map(|event| {
                    PlanEvent::new(
                        event.position(),
                        rebind_program(event.payload(), &source, &next).unwrap(),
                    )
                })
                .collect();
            let stream = AdmittedCompiledStream::admit(&next, &mapped).unwrap();
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
            if index == 0 {
                assert_eq!(audio.clock(), SampleTime::new(192));
                assert!(audio.renderer.continuation_has_active_steal_fade());
            }
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
            let expected = render(&mut reference, 64);
            assert!(
                samples
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "rate={rate}, edit={index}"
            );
            let receipt = control.collect_replacement(owner).unwrap();
            assert!(matches!(
                receipt.outcome,
                PlanReplacementOutcome::Installed(_)
            ));
            assert_eq!(receipt.retired_compiled_notes.get(), 0);
            assert_eq!(audio.origin.table, origin.table);
            assert_eq!(control.origin.table, origin.table);
            assert_eq!(audio.epoch(), origin.epoch);
            assert!(!control.has_replacements());
            assert!(!control.has_outstanding());
        }
    }
}

fn reindex_and_compare(
    control: &mut SessionControl,
    audio: &mut SessionAudio,
    reference: &mut SessionAudio,
    next: CompiledPlan,
    events: &[PlanEvent],
    partitions: &[usize],
) {
    let old_slot = control
        .control
        .plan()
        .resolve_note(NodeId::new(20))
        .unwrap();
    let next_slot = next.resolve_note(NodeId::new(20)).unwrap();
    assert_ne!(
        old_slot.index(),
        next_slot.index(),
        "actual compiler must move the logical target"
    );
    continue_and_compare(control, audio, reference, next, events, partitions);
}

fn continue_and_compare(
    control: &mut SessionControl,
    audio: &mut SessionAudio,
    reference: &mut SessionAudio,
    next: CompiledPlan,
    events: &[PlanEvent],
    partitions: &[usize],
) {
    let next_slot = next.resolve_note(NodeId::new(20)).unwrap();
    let table = audio.origin.table;
    let stream = AdmittedCompiledStream::admit(&next, events).unwrap();
    let profile = profile(audio.renderer.plan().sample_rate().as_f32());
    let candidate = prepare_admitted(control, audio, next, stream, profile);
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningCompatible);
    for command in [
        crate::host::session::SessionCommand::Play,
        crate::host::session::SessionCommand::Stop,
        crate::host::session::SessionCommand::Panic,
    ] {
        assert!(matches!(
            control.validate(audio.clock(), command),
            Err(crate::host::session::SessionError::ReplacementOutstanding)
        ));
    }
    let mut owner = None;
    assert_eq!(
        crate::render_allocation::count_allocs(
            || owner = Some(audio.begin_running_replacement(candidate).unwrap())
        ),
        0
    );
    let mut owner = owner.unwrap();
    for count in partitions {
        let mut actual = vec![0.0; count * 2];
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                let block = AudioBlockMut::new(&mut actual, *count, ChannelLayout::Stereo).unwrap();
                if owner.outcome().is_none() {
                    audio.render_running_replacement(&mut owner, block).unwrap();
                } else {
                    audio.render(block).unwrap();
                }
            }),
            0
        );
        assert_eq!(actual, render(reference, *count));
    }
    // Every command is frozen while the owning replacement is uncollected,
    // including after B. An old-plan Stop must never cross a reindex boundary.
    assert!(matches!(
        control.prepare_stop(audio.clock()),
        Err(crate::host::session::SessionError::ReplacementOutstanding)
    ));
    assert!(matches!(
        control.prepare_play(audio.clock()),
        Err(crate::host::session::SessionError::ReplacementOutstanding)
    ));
    assert!(matches!(
        control.validate(audio.clock(), crate::host::session::SessionCommand::Panic),
        Err(crate::host::session::SessionError::ReplacementOutstanding)
    ));
    let receipt = control.collect_replacement(owner).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Installed(_)
    ));
    assert_eq!(
        control
            .control
            .plan()
            .resolve_note(NodeId::new(20))
            .unwrap(),
        next_slot
    );
    assert_eq!(audio.origin.table, table);
    assert_eq!(control.origin.table, table);
    assert!(!control.has_replacements());
}

#[test]
fn actual_global_gate_edits_reindex_held_native_voices_and_future_program_at_four_rates() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        for parts in [&[256][..], &[1, 63, 7, 121, 64]] {
            let old = relocated_plan(profile, 0, crate::ir::StealingPolicy::None);
            let events = program(&old);
            let (mut control, mut audio) = start(old.clone(), &events, profile);
            let (_, mut reference) = start(old, &events, profile);
            for index in 0..8 {
                let next = edited_plan(
                    profile,
                    0,
                    crate::ir::StealingPolicy::None,
                    if index % 2 == 0 {
                        VoiceEdit::DormantGlobal
                    } else {
                        VoiceEdit::None
                    },
                );
                let events = program(&next);
                reindex_and_compare(
                    &mut control,
                    &mut audio,
                    &mut reference,
                    next,
                    &events,
                    parts,
                );
            }
            let boundary = audio.clock();
            audio
                .enqueue(control.prepare_stop(boundary).unwrap())
                .unwrap();
            let _stopped = render(&mut audio, 64);
            let _receipt = control.collect(audio.take_completed().unwrap()).unwrap();
            assert!(!control.has_outstanding());
        }
    }
}

#[test]
fn actual_note_reindexing_preserves_an_in_progress_steal_and_its_later_releases() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        let policy = crate::ir::StealingPolicy::Oldest {
            fade: FrameCount::new(8),
        };
        let old = relocated_plan(profile, 0, policy);
        let slot = old.resolve_note(NodeId::new(20)).unwrap();
        let mut events = program(&old);
        events.extend([(64, 62), (188, 64)].map(|(at, key)| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::NoteOn {
                    slot,
                    key: KeyIdentity::new(key).unwrap(),
                    velocity: NoteVelocity::FULL,
                },
            )
        }));
        events.sort_by_key(|event| event.position());
        let source = old.clone();
        let (mut control, mut audio) = start(old.clone(), &events, profile);
        let (_, mut reference) = start(old, &events, profile);
        assert!(audio.renderer.continuation_has_active_steal_fade());
        for index in 0..8 {
            let next = edited_plan(
                profile,
                0,
                policy,
                if index % 2 == 0 {
                    VoiceEdit::DormantGlobal
                } else {
                    VoiceEdit::None
                },
            );
            let mapped: Vec<_> = events
                .iter()
                .map(|event| {
                    PlanEvent::new(
                        event.position(),
                        rebind_program(event.payload(), &source, &next).unwrap(),
                    )
                })
                .collect();
            reindex_and_compare(
                &mut control,
                &mut audio,
                &mut reference,
                next,
                &mapped,
                &[1, 63],
            );
        }
        for _ in 0..24 {
            assert_eq!(render(&mut audio, 64), render(&mut reference, 64));
        }
    }
}

#[test]
fn invalid_authored_note_maps_refuse_atomically_before_sounding_state_or_authority_moves() {
    let profile = profile(48_000.0);
    for corruption in 0..4 {
        let old = relocated_plan(profile, 0, crate::ir::StealingPolicy::None);
        let old_slot = old.resolve_note(NodeId::new(20)).unwrap();
        let events = program(&old);
        let (mut control, mut audio) = start(old.clone(), &events, profile);
        let (_, mut reference) = start(old, &events, profile);
        let origin = audio.origin;
        let next = edited_plan(
            profile,
            0,
            crate::ir::StealingPolicy::None,
            VoiceEdit::DormantGlobal,
        );
        let live = next.resolve_note(NodeId::new(20)).unwrap();
        let fresh = next.resolve_note(NodeId::new(2000)).unwrap();
        let stream = AdmittedCompiledStream::admit(&next, &program(&next)).unwrap();
        let mut candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
        let bad = match corruption {
            0 => Some(NoteSlot::new(origin.plan, live.index())),
            1 => Some(NoteSlot::new(live.plan(), usize::MAX)),
            2 => Some(fresh),
            _ => None,
        };
        assert!(
            candidate
                .continuation
                .as_mut()
                .unwrap()
                .proof
                .notes
                .replace_destination_for_test(old_slot, bad)
        );
        let mut result = None;
        assert_eq!(
            crate::render_allocation::count_allocs(
                || result = Some(audio.begin_running_replacement(candidate).unwrap())
            ),
            0
        );
        let candidate = result.unwrap();
        assert!(matches!(
            candidate.outcome(),
            Some(PlanReplacementOutcome::Refused(
                PlanReplacementRefusal::Continuation(ContinuationError::Storage)
            ))
        ));
        let _receipt = control.collect_replacement(candidate).unwrap();
        assert_eq!(audio.origin.plan, origin.plan);
        assert_eq!(audio.origin.table, origin.table);
        assert_eq!(control.origin.table, origin.table);
        for _ in 0..20 {
            assert_eq!(render(&mut audio, 64), render(&mut reference, 64));
        }
    }
}

#[test]
fn a_global_note_target_with_a_parameter_writer_is_not_dormant_and_selects_reset() {
    let profile = profile(48_000.0);
    let old = edited_plan(
        profile,
        0,
        crate::ir::StealingPolicy::None,
        VoiceEdit::DormantGlobal,
    );
    let mut events = program(&old);
    let parameter = old
        .resolve_parameter(NodeId::new(2000), crate::ir::parameters::ENVELOPE_ATTACK)
        .unwrap();
    events.push(PlanEvent::new(
        PlanPosition::new(64),
        CompiledPayload::SetParameter {
            slot: parameter,
            value: ParameterValue::new(0.5).unwrap(),
        },
    ));
    events.sort_by_key(|event| event.position());
    let source = old.clone();
    let (mut control, audio) = start(old, &events, profile);
    let next = edited_plan(
        profile,
        1,
        crate::ir::StealingPolicy::None,
        VoiceEdit::DormantGlobal,
    );
    let mapped: Vec<_> = events
        .iter()
        .map(|event| {
            PlanEvent::new(
                event.position(),
                rebind_program(event.payload(), &source, &next).unwrap(),
            )
        })
        .collect();
    let stream = AdmittedCompiledStream::admit(&next, &mapped).unwrap();
    let candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningReset);
    let _cancelled = control.cancel_replacement(candidate).unwrap();
}

#[test]
fn logical_note_reindexing_hands_the_actual_observer_to_the_retained_sounding_renderer() {
    use crate::{
        host::session::transfer::replacement::ObservationReplacementOutcome,
        observe::ObservationSubscriptions,
    };
    let profile = profile(48_000.0);
    let policy = crate::ir::StealingPolicy::None;
    let old = edited_plan_with_observation(profile, 0, policy, VoiceEdit::None, true);
    let next = edited_plan_with_observation(profile, 0, policy, VoiceEdit::DormantGlobal, true);
    assert_ne!(
        old.resolve_note(NodeId::new(20)).unwrap().index(),
        next.resolve_note(NodeId::new(20)).unwrap().index()
    );
    let mut store = ObservationSubscriptions::prepare(&profile, &old).unwrap();
    let handle = store
        .subscribe(
            &old,
            old.resolve_tap(NodeId::new(65), PortId::FIRST).unwrap(),
        )
        .unwrap();
    let budget = PreparedBytes::measured(4096);
    let handoff = store
        .configuration(budget)
        .unwrap()
        .prepare_handoff(&old, &next, budget)
        .unwrap();
    let events = program(&old);
    let (mut control, mut audio) = start(old.clone(), &events, profile);
    let (_, mut reference) = start(old, &events, profile);
    let stream = AdmittedCompiledStream::admit(&next, &program(&next)).unwrap();
    let mut candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
    candidate.attach_observation(Ok(handoff).into()).unwrap();
    let mut candidate = audio
        .begin_running_replacement_observed(candidate, &mut store)
        .unwrap();
    let mut taps = vec![0.0; 3072];
    let mut total = 0;
    let mut sound = Vec::new();
    for _ in 0..3 {
        for frames in [1, 63, 7, 57, 128, 256] {
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
    }
    assert_eq!(total, sound.len());
    assert_eq!(taps, sound);
    assert!(sound.iter().any(|sample| *sample != 0.0));
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.observation,
        ObservationReplacementOutcome::HandedOff { .. }
    ));
}

#[test]
fn a_future_global_note_reference_selects_reset_even_before_its_onset() {
    let profile = profile(48_000.0);
    let old = edited_plan(
        profile,
        0,
        crate::ir::StealingPolicy::None,
        VoiceEdit::DormantGlobal,
    );
    let slot = old.resolve_note(NodeId::new(2000)).unwrap();
    let mut events = program(&old);
    events.extend([
        PlanEvent::new(
            PlanPosition::new(2048),
            CompiledPayload::NoteOn {
                slot,
                key: KeyIdentity::new(67).unwrap(),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(2112),
            CompiledPayload::NoteOff {
                slot,
                key: KeyIdentity::new(67).unwrap(),
            },
        ),
    ]);
    events.sort_by_key(|event| event.position());
    let source = old.clone();
    let (mut control, audio) = start(old, &events, profile);
    let next = edited_plan(
        profile,
        1,
        crate::ir::StealingPolicy::None,
        VoiceEdit::DormantGlobal,
    );
    let mapped: Vec<_> = events
        .iter()
        .map(|event| {
            PlanEvent::new(
                event.position(),
                rebind_program(event.payload(), &source, &next).unwrap(),
            )
        })
        .collect();
    let stream = AdmittedCompiledStream::admit(&next, &mapped).unwrap();
    let candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningReset);
    let _cancelled = control.cancel_replacement(candidate).unwrap();
}

#[test]
fn a_global_gate_with_a_scheduled_consumer_refuses_preservation_even_without_note_events() {
    let profile = profile(48_000.0);
    let old = edited_plan(
        profile,
        0,
        crate::ir::StealingPolicy::None,
        VoiceEdit::ConnectedGlobal,
    );
    let events = program(&old);
    let (mut control, audio) = start(old, &events, profile);
    let next = edited_plan(
        profile,
        1,
        crate::ir::StealingPolicy::None,
        VoiceEdit::ConnectedGlobal,
    );
    assert!(matches!(
        control.control.plan().prepare_continuation(
            &next,
            witness_limits().transition_bytes,
            witness_limits().maximum_transition_records
        ),
        Err(ContinuationError::Plan)
    ));
    let stream = AdmittedCompiledStream::admit(&next, &program(&next)).unwrap();
    let candidate = prepare_admitted(&mut control, &audio, next, stream, profile);
    assert_eq!(candidate.kind(), PlanReplacementKind::RunningReset);
    let _cancelled = control.cancel_replacement(candidate).unwrap();
}

#[test]
fn retained_dormant_global_gates_allow_structural_continuation_with_and_without_an_unused_amplifier()
 {
    let profile = profile(48_000.0);
    for edit in [VoiceEdit::DormantGlobal, VoiceEdit::DetachedGlobal] {
        let old = edited_plan(profile, 0, crate::ir::StealingPolicy::None, edit);
        let events = program(&old);
        let slot = old.resolve_note(NodeId::new(20)).unwrap();
        let (mut control, mut audio) = start(old.clone(), &events, profile);
        let (_, mut reference) = start(old, &events, profile);
        for prefix in [1, 0, 1, 0] {
            let next = edited_plan(profile, prefix, crate::ir::StealingPolicy::None, edit);
            assert_eq!(
                slot.index(),
                next.resolve_note(NodeId::new(20)).unwrap().index()
            );
            let events = program(&next);
            continue_and_compare(
                &mut control,
                &mut audio,
                &mut reference,
                next,
                &events,
                &[1, 63, 7, 121, 64],
            );
        }
    }
}

#[test]
fn note_map_byte_and_structural_record_refusals_precede_map_allocation() {
    let profile = profile(48_000.0);
    let old = relocated_plan(profile, 0, crate::ir::StealingPolicy::None);
    let next = edited_plan(
        profile,
        0,
        crate::ir::StealingPolicy::None,
        VoiceEdit::DormantGlobal,
    );
    let mut result = None;
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            result = Some(crate::plan::note_binding::NoteBindings::prepare(
                &old,
                &next,
                crate::plan::note_binding::NoteBindingScope::StructuralVoices,
                PreparedBytes::measured(0),
            ));
        }),
        0
    );
    assert!(matches!(result.unwrap(), Err(ContinuationError::Bytes)));
    let mut result = None;
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            result = Some(old.prepare_continuation(
                &next,
                witness_limits().transition_bytes,
                crate::quantities::RecordCount::measured(1),
            ));
        }),
        0
    );
    assert!(matches!(
        result.unwrap(),
        Err(ContinuationError::RecordLimit)
    ));
}

#[test]
fn a_fresh_global_onset_in_a_witness_is_refused_even_when_the_score_does_not_name_it() {
    let profile = profile(48_000.0);
    let old = edited_plan(
        profile,
        0,
        crate::ir::StealingPolicy::None,
        VoiceEdit::DormantGlobal,
    );
    let slot = old.resolve_note(NodeId::new(2000)).unwrap();
    let events = program(&old);
    let (mut control, mut audio) = start(old.clone(), &events, profile);
    let (_, mut reference) = start(old, &events, profile);
    let witness = control.active_timeline.as_mut().unwrap();
    let onset = witness
        .events
        .iter_mut()
        .find(|event| {
            matches!(
                event.payload(),
                EventPayload::Note {
                    edge: NoteEdge::On { .. },
                    ..
                }
            )
        })
        .unwrap();
    let EventPayload::Note {
        identity,
        edge: NoteEdge::On { key, velocity, .. },
    } = onset.payload()
    else {
        panic!("onset");
    };
    *onset = TimedEvent::new(
        onset.envelope(),
        EventPayload::Note {
            identity,
            edge: NoteEdge::On {
                slot,
                key,
                velocity,
            },
        },
    );
    let next = edited_plan(
        profile,
        1,
        crate::ir::StealingPolicy::None,
        VoiceEdit::DormantGlobal,
    );
    let stream = AdmittedCompiledStream::admit(&next, &program(&next)).unwrap();
    let result = control.prepare_running_replacement(
        next,
        stream,
        RunningReplacementRequest {
            profile,
            transfer: limits(),
            song_end: None,
            maximum_timeline_events: EventCount::limit(16384).unwrap(),
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
    assert!(!control.has_replacements());
    for _ in 0..8 {
        assert_eq!(render(&mut audio, 64), render(&mut reference, 64));
    }
}
