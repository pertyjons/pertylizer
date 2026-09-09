use super::*;
use crate::{
    compile::{RenderConfig, compile},
    ir::{
        AuthoredSourceDeclaration, ExecutionScope, GraphIr, IrNodeKind, NoteProducerDeclaration,
        PlanDeclarations, PortId, SignalDomain,
    },
    quantities::{
        Amplitude, ChannelLayout, Frequency, HeldNoteCount, NormalizedLevel, ParameterValue,
        SampleRate, Seconds,
    },
    render::AudioBlockMut,
    schedule::{CompiledPayload, PlanEvent},
    script::{ProjectSeed, ScriptBinding, ScriptIdentity, ScriptSource, ScriptStateId},
    time::{FrameCount, PlanPosition},
};
const SCRIPT: NodeId = NodeId::new(10);
const ENV: NodeId = NodeId::new(11);
const OSC: NodeId = NodeId::new(12);
const AMP: NodeId = NodeId::new(13);
const OUT: NodeId = NodeId::new(14);
const CONTROL: NodeId = NodeId::new(15);
const FRAMES: usize = 4096;
fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(FRAMES as u64),
        ChannelLayout::Mono,
    )
    .unwrap()
}
fn plan(source: &str, bindings: &[ScriptBinding], count: u32) -> CompiledPlan {
    plan_scoped(source, bindings, count, ExecutionScope::Voice)
}
fn plan_scoped(
    source: &str,
    bindings: &[ScriptBinding],
    count: u32,
    voice_scope: ExecutionScope,
) -> CompiledPlan {
    let rate = profile().capabilities().sample_rate();
    let program = ScriptIdentity::new(SCRIPT, ScriptStateId::new(42), ProjectSeed::new(99))
        .compile_note(source, rate, bindings)
        .expect("Note program");
    let control = ScriptIdentity::new(CONTROL, ScriptStateId::new(43), ProjectSeed::new(99))
        .compile_control(
            "param bias = 4 [0, 20]\nout = accum(1) + bias * 0",
            rate,
            &[],
        )
        .unwrap();
    let ir = GraphIr::builder()
        .script(program, ExecutionScope::Global)
        .script(control, ExecutionScope::Global)
        .node(
            OSC,
            IrNodeKind::Sine {
                frequency: Frequency::A4,
                amplitude: Amplitude::UNITY,
            },
            voice_scope,
        )
        .node(
            ENV,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            voice_scope,
        )
        .node(AMP, IrNodeKind::Amplifier, voice_scope)
        .node(OUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSC, PortId::FIRST),
            (AMP, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENV, PortId::FIRST),
            (AMP, crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMP, PortId::FIRST),
            (OUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(
            voice_scope,
            crate::tuning::PreparedTuning::equal_temperament().unwrap(),
        )
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(count),
                simultaneous_holds: EventCount::measured(count),
            }],
            authored_sources: vec![AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: EventCount::measured(count * 2),
                retained_future: EventCount::measured(count * 2),
                simultaneous_holds: EventCount::measured(count),
            }],
            ..PlanDeclarations::default()
        })
        .build()
        .expect("IR");
    compile(&ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("admission")
}
fn note(id: u64, start: u64, duration: Option<u16>) -> AuthoredNote {
    AuthoredNote {
        occurrence: AuthoredOccurrenceId::new(id),
        start: MusicalTick::new(start),
        tick: ScriptTick::new(7),
        duration: duration.map(ScriptDuration::new),
        cut: MusicalTick::new(start + 24),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::new(0.5).unwrap(),
    }
}
fn source(plan: &CompiledPlan, notes: Vec<AuthoredNote>) -> AuthoredNoteSource {
    AuthoredNoteSource {
        script: SCRIPT,
        producer: ProducerId::new(0),
        target: plan.resolve_note(ENV).unwrap(),
        maximum_duration: ScriptDuration::new(24),
        tempo: TempoMap::new(
            crate::tempo::Bpm::new(120.0).unwrap(),
            &[],
            plan.sample_rate(),
        )
        .unwrap(),
        notes,
    }
}
fn prepare(
    plan: CompiledPlan,
    notes: Vec<AuthoredNote>,
    events: &[PlanEvent],
) -> AuthoredNoteStream {
    let source = source(&plan, notes);
    let automation = AdmittedCompiledStream::admit(&plan, events).unwrap();
    AuthoredNoteStream::prepare_source(
        plan,
        &profile(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        source,
        &automation,
    )
    .expect("source")
}
fn rendered(
    mut stream: AuthoredNoteStream,
    blocks: &[usize],
) -> (Vec<u32>, Vec<GeneratedNote>, AuthoredUsage) {
    let mut result = Vec::new();
    for block in blocks {
        let mut output = vec![f32::NAN; *block];
        stream
            .render(AudioBlockMut::new(&mut output, *block, ChannelLayout::Mono).unwrap())
            .expect("render");
        result.extend(output.iter().map(|sample| sample.to_bits()));
    }
    assert!(!stream.diagnostics().needs_reprepare());
    assert!(
        stream
            .occurrences
            .iter()
            .all(|entry| entry.phase == Phase::Complete)
    );
    (result, stream.generated_notes().collect(), stream.usage())
}
#[test]
fn seeded_notes_and_audio_are_partition_invariant() {
    let plan = plan(
        "out.pitch = note_pitch + round(rand() * 12)\nout.vel = note_vel\nout.dur = 5",
        &[],
        4,
    );
    let notes = vec![
        note(90, 0, None),
        note(50, 3, None),
        note(700, 8, None),
        note(2, 12, None),
    ];
    let reference = rendered(prepare(plan.clone(), notes.clone(), &[]), &[FRAMES]);
    assert!(reference.0.iter().any(|bits| *bits != 0));
    assert_eq!(reference.1.len(), 4);
    for blocks in [
        vec![1; FRAMES],
        vec![64; FRAMES / 64],
        vec![7, 59, 1, 1023, 3006],
    ] {
        assert_eq!(
            rendered(prepare(plan.clone(), notes.clone(), &[]), &blocks),
            reference
        );
    }
    let mut changed = notes.clone();
    changed[0].occurrence = AuthoredOccurrenceId::new(999);
    let changed = rendered(prepare(plan, changed, &[]), &[FRAMES]);
    assert_ne!(
        changed.1[0].key, reference.1[0].key,
        "changing the occurrence must change its random value, not merely the trace's ID"
    );
    assert_ne!(changed.0, reference.0);
}
#[test]
fn note_context_captures_local_automation_and_signal_on_the_quantum_clock() {
    let binding = ScriptBinding {
        input: synth_script::compile::SourceInput::NoteInput(0),
        source: ScriptSource::Signal {
            node: CONTROL,
            port: PortId::FIRST,
        },
    };
    let plan = plan(
        "param transpose = 2 [0, 20]\nout.pitch = note_pitch + transpose + in1\nout.dur = 2",
        &[binding],
        3,
    );
    let parameter = plan
        .parameter_addresses()
        .iter()
        .find(|address| address.node == SCRIPT)
        .unwrap()
        .slot;
    let events = [PlanEvent::new(
        PlanPosition::new(65),
        CompiledPayload::SetParameter {
            slot: parameter,
            value: ParameterValue::new(10.0).unwrap(),
        },
    )];
    // At 120 BPM/48 kHz, each tick is 25 frames (960 ticks/quarter).
    let notes = vec![note(1, 0, None), note(2, 4, None), note(3, 8, None)];
    let reference = rendered(prepare(plan.clone(), notes.clone(), &events), &[FRAMES]);
    assert_eq!(
        reference
            .1
            .iter()
            .map(|n| n.key.as_u8())
            .collect::<Vec<_>>(),
        vec![62, 63, 73]
    );
    assert_eq!(
        rendered(prepare(plan, notes, &events), &vec![1; FRAMES]),
        reference
    );
}
#[test]
fn finite_until_cut_drop_and_zero_duration_have_explicit_outcomes() {
    for (code, expected_count, expected_end) in [
        ("out.dur = -1", 1, 600),
        ("out.vel = -1", 0, 0),
        ("out.dur = 0", 0, 0),
        ("out.dur = 2.5\nout.gate = 0.5", 1, 50),
        ("out.dur = 999\nout.gate = 0.5", 1, 300),
    ] {
        let plan = plan(code, &[], 1);
        let (_, trace, _) = rendered(prepare(plan, vec![note(1, 0, Some(4))], &[]), &[FRAMES]);
        assert_eq!(trace.len(), expected_count);
        if let Some(note) = trace.first() {
            assert_eq!(note.end.as_u64(), expected_end);
        }
    }
}
#[test]
fn same_batch_charges_both_authored_and_later_release_redeems_a_hold() {
    let same = rendered(
        prepare(plan("out.dur = 1", &[], 1), vec![note(1, 0, None)], &[]),
        &[FRAMES],
    )
    .2;
    assert_eq!(same.authored_per_quantum.get(), 2);
    assert_eq!(same.held_releases.get(), 0);
    assert_eq!(same.releases_per_quantum.get(), 0);
    let future = rendered(
        prepare(plan("out.dur = 4", &[], 1), vec![note(1, 0, None)], &[]),
        &[FRAMES],
    )
    .2;
    assert_eq!(future.authored_per_quantum.get(), 1);
    assert_eq!(future.held_releases.get(), 1);
    assert_eq!(future.releases_per_quantum.get(), 1);
}
#[test]
fn one_past_input_refuses_the_whole_source_before_playback() {
    let plan = plan("out.dur = 1", &[], 1);
    let source = source(&plan, vec![note(1, 0, None), note(2, 0, None)]);
    let automation = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    assert!(matches!(
        AuthoredNoteStream::prepare_source(
            plan,
            &profile(),
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
            source,
            &automation
        ),
        Err(AuthoredPrepareError::Capacity)
    ));
}
#[test]
fn plain_stream_cannot_silently_skip_a_note_program() {
    let plan = plan("out.dur = 1", &[], 1);
    assert!(matches!(
        StreamControl::open(
            plan,
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO)
        ),
        Err(crate::diagnostics::CompileError::Script {
            fault: crate::script::ScriptFault::NoteSourceRequired,
            ..
        })
    ));
}

#[test]
fn a_later_quantum_fault_silences_the_entire_callback_and_all_future_calls() {
    let plan = plan("out.dur = 20", &[], 2);
    let first = note(1, 0, None);
    let mut second = note(2, 8, None);
    second.key = KeyIdentity::new(61).unwrap();
    let mut stream = prepare(plan, vec![first, second], &[]);
    // Forge a mapping after preparation: it fails only when the second note runs in q3.
    stream.occurrences[1].raw.start = MusicalTick::new(u64::MAX);

    let mut buffer = vec![9.0; FRAMES];
    assert!(matches!(
        stream.render(AudioBlockMut::new(&mut buffer, FRAMES, ChannelLayout::Mono).unwrap()),
        Err(AuthoredRenderError::Source {
            fault: AuthoredFault::Output,
            ..
        })
    ));
    assert!(buffer.iter().all(|value| *value == 0.0));
    assert!(stream.diagnostics().needs_reprepare());
    assert_eq!(stream.diagnostics().authored_source_faults(), 1);
    assert_eq!(stream.diagnostics().publication_faults(), 0);
    buffer.fill(9.0);
    assert!(matches!(
        stream.render(AudioBlockMut::new(&mut buffer, FRAMES, ChannelLayout::Mono).unwrap()),
        Err(AuthoredRenderError::Render(
            crate::diagnostics::RenderError::NeedsReprepare
        ))
    ));
    assert!(buffer.iter().all(|value| *value == 0.0));
}
#[test]
fn forged_late_and_overfull_sources_are_terminal_not_live_drops() {
    let mut late = prepare(plan("out.dur = 1", &[], 1), vec![note(1, 8, None)], &[]);
    let mut head = [0.0; 192];
    late.render(AudioBlockMut::new(&mut head, 192, ChannelLayout::Mono).unwrap())
        .unwrap();
    late.occurrences[0].start = SampleTime::ZERO;
    let mut buffer = [9.0; 64];
    assert!(matches!(
        late.render(AudioBlockMut::new(&mut buffer, 64, ChannelLayout::Mono).unwrap()),
        Err(AuthoredRenderError::Source {
            fault: AuthoredFault::Late,
            ..
        })
    ));
    assert!(buffer.iter().all(|v| *v == 0.0));
    let count = crate::publish::ProducerClass::AuthoredRuntime
        .share_of(&profile())
        .get()
        / 2;
    let mut full = prepare(
        plan("out.dur = 1", &[], count),
        (0..count).map(|i| note(u64::from(i), 0, None)).collect(),
        &[],
    );
    let mut forged = full.occurrences.to_vec();
    let mut extra = forged[0];
    extra.raw.occurrence = AuthoredOccurrenceId::new(999);
    extra.raw.start = MusicalTick::new(1);
    extra.start = SampleTime::new(25);
    forged.push(extra);
    full.occurrences = forged.into_boxed_slice();
    // Forge past the admission boundary, including its reservation counter, so the actual arbiter detects the over-emit.
    full.usage.reserved_obligations = EventCount::measured(count + 1);
    let mut buffer = [9.0; 128];
    assert!(matches!(
        full.render(AudioBlockMut::new(&mut buffer, 128, ChannelLayout::Mono).unwrap()),
        Err(AuthoredRenderError::Source {
            fault: AuthoredFault::Publication,
            ..
        })
    ));
    assert!(full.diagnostics().needs_reprepare());
    assert_eq!(full.diagnostics().authored_source_faults(), 1);
    assert_eq!(full.diagnostics().publication_faults(), 1);
    assert!(buffer.iter().all(|v| *v == 0.0));
}
#[test]
fn compiler_note_knobs_are_opt_in_and_keep_the_note_output_grammar() {
    let old = synth_script::compile::CompileOptions {
        note_event: true,
        ..Default::default()
    };
    assert!(
        synth_script::compile::compile("param x = 1\nout.pitch = x", &old)
            .0
            .is_none()
    );
    assert!(
        synth_script::compile::compile_note_with_params("param x = 1\nout.pitch = x", 750.0)
            .0
            .is_some()
    );
    for wrong in [
        "out = 1",
        "out1 = 1",
        "out.left = 1",
        "out.pitch = first_sample",
        "out.pitch = sr",
    ] {
        assert!(
            synth_script::compile::compile_note_with_params(wrong, 750.0)
                .0
                .is_none(),
            "{wrong}"
        );
    }
}

#[test]
fn vm_nonfinite_arithmetic_keeps_its_existing_finite_store_rule() {
    let (_, trace, _) = rendered(
        prepare(
            plan("out.vel = 1e38 * 1e38\nout.dur = 1", &[], 1),
            vec![note(1, 0, None)],
            &[],
        ),
        &[FRAMES],
    );
    assert_eq!(trace[0].velocity.as_f32(), 0.0);
}

#[test]
fn first_and_repeated_authored_callbacks_allocate_nothing_at_the_admitted_maximum() {
    let count = crate::publish::ProducerClass::AuthoredRuntime
        .share_of(&profile())
        .get()
        / 2;
    let mut stream = prepare(
        plan(
            "out.pitch = note_pitch + rand() * 12\nout.dur = -1",
            &[],
            count,
        ),
        (0..count).map(|i| note(u64::from(i), 0, None)).collect(),
        &[],
    );
    let mut samples = vec![0.0; FRAMES];
    let allocations = crate::render_allocation::count_allocs(|| {
        for _ in 0..3 {
            stream
                .render(AudioBlockMut::new(&mut samples, FRAMES, ChannelLayout::Mono).unwrap())
                .unwrap();
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(stream.usage.held_releases.get(), count);
}

/// EVD-0020: measure a committed subject only, after the empty control below passes.
#[test]
#[ignore = "retained evidence run on a committed subject, not an ordinary test"]
fn evd_0020_authored_capacity() {
    let count = crate::publish::ProducerClass::AuthoredRuntime
        .share_of(&profile())
        .get()
        / 2;
    let cases = [
        ("empty", "out.dur = 1", 0, 0),
        ("drop", "out.vel = -1", count, 0),
        ("same_batch", "out.dur = 1", count, 0),
        ("held", "out.dur = 4", count, 0),
        ("until_cut", "out.dur = -1", count, 0),
        ("retained_inputs", "out.dur = -1", count, 1000),
        (
            "staggered",
            "out.pitch = note_pitch + rand() * 12\nout.dur = 20",
            count,
            3,
        ),
    ];
    println!(
        "case,inputs,pending_peak,hold_peak,future_peak,reserved_peak,eval_peak,authored_peak,release_peak,source_bytes,allocations,draws,median_ns,max_ns"
    );
    for (name, code, inputs, step) in cases {
        let compiled = plan(code, &[], count);
        let notes: Vec<_> = (0..inputs)
            .map(|i| {
                note(
                    u64::from(i),
                    u64::from(if name == "retained_inputs" {
                        step
                    } else {
                        i * step
                    }),
                    None,
                )
            })
            .collect();
        let mut times = Vec::new();
        let mut observed = None;
        for _ in 0..101 {
            let mut stream = prepare(compiled.clone(), notes.clone(), &[]);
            let mut output = vec![0.0; FRAMES];
            let start = std::time::Instant::now();
            stream
                .render(AudioBlockMut::new(&mut output, FRAMES, ChannelLayout::Mono).unwrap())
                .unwrap();
            times.push(start.elapsed().as_nanos());
            let usage = stream.usage();
            if let Some((previous, _)) = observed {
                assert_eq!(usage, previous);
            }
            if name == "empty" {
                assert_eq!(usage, AuthoredUsage::default());
                assert!(output.iter().all(|value| *value == 0.0));
            }
            observed = Some((usage, stream.source_bytes()));
        }
        let mut stream = prepare(compiled, notes, &[]);
        let mut output = vec![0.0; FRAMES];
        let allocations = crate::render_allocation::count_allocs(|| {
            stream
                .render(AudioBlockMut::new(&mut output, FRAMES, ChannelLayout::Mono).unwrap())
                .unwrap()
        });
        assert_eq!(allocations, 0);
        let (usage, bytes) = observed.unwrap();
        assert!(
            usage.authored_per_quantum
                <= crate::publish::ProducerClass::AuthoredRuntime.share_of(&profile())
        );
        assert!(usage.held_releases.get() <= count);
        if name == "same_batch" {
            assert_eq!(usage.authored_per_quantum.get(), count * 2);
            assert_eq!(usage.held_releases.get(), 0);
        }
        if matches!(name, "held" | "until_cut") {
            assert_eq!(usage.held_releases.get(), count);
            assert_eq!(usage.releases_per_quantum.get(), count);
        }
        times.sort_unstable();
        println!(
            "{name},{inputs},{},{},{},{},{},{},{},{bytes},{allocations},101,{},{}",
            usage.pending_inputs.get(),
            usage.held_releases.get(),
            usage.future_events.get(),
            usage.reserved_obligations.get(),
            usage.evaluations_per_quantum.get(),
            usage.authored_per_quantum.get(),
            usage.releases_per_quantum.get(),
            times[50],
            times[100]
        );
    }
}

#[test]
fn a_shared_gate_cannot_accept_several_potentially_overlapping_notes() {
    let plan = plan_scoped("out.dur = 20", &[], 2, ExecutionScope::Global);
    let source = source(&plan, vec![note(1, 0, None), note(2, 1, None)]);
    let automation = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    assert!(matches!(
        AuthoredNoteStream::prepare_source(
            plan,
            &profile(),
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
            source,
            &automation
        ),
        Err(AuthoredPrepareError::Capacity)
    ));
}
#[test]
fn occurrence_seed_is_independent_of_same_time_input_order() {
    let plan = plan(
        "out.pitch = note_pitch + round(rand() * 12)\nout.dur = 1",
        &[],
        4,
    );
    let notes: Vec<_> = [90, 50, 700, 2]
        .into_iter()
        .map(|id| note(id, 0, None))
        .collect();
    let mut reversed = notes.clone();
    reversed.reverse();
    let by_id = |trace: Vec<GeneratedNote>| {
        let mut values: Vec<_> = trace
            .iter()
            .map(|n| (n.occurrence.as_u64(), n.key.as_u8()))
            .collect();
        values.sort_unstable();
        values
    };
    assert_eq!(
        by_id(rendered(prepare(plan.clone(), notes, &[]), &[FRAMES]).1),
        by_id(rendered(prepare(plan, reversed, &[]), &[FRAMES]).1)
    );
}
#[test]
fn explicit_cut_bounds_a_longer_finite_output() {
    let mut input = note(1, 0, None);
    input.cut = MusicalTick::new(2);
    let (_, trace, _) = rendered(
        prepare(plan("out.dur = 20", &[], 1), vec![input], &[]),
        &[FRAMES],
    );
    assert_eq!(trace[0].end, SampleTime::new(50));
}

#[test]
fn parameter_bindings_initialize_from_base_then_capture_the_named_layer() {
    for (read, second_key) in [
        (crate::script::ParameterRead::Base, 64),
        (crate::script::ParameterRead::Automated, 67),
    ] {
        let binding = ScriptBinding {
            input: synth_script::compile::SourceInput::NoteInput(0),
            source: ScriptSource::Parameter {
                node: CONTROL,
                parameter: crate::ir::ParameterId::FIRST,
                read,
            },
        };
        let plan = plan("out.pitch = note_pitch + in1\nout.dur = 1", &[binding], 2);
        let slot = plan
            .resolve_parameter(CONTROL, crate::ir::ParameterId::FIRST)
            .unwrap();
        let events = [PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::SetParameter {
                slot,
                value: ParameterValue::new(7.0).unwrap(),
            },
        )];
        let (_, trace, _) = rendered(
            prepare(plan, vec![note(1, 0, None), note(2, 4, None)], &events),
            &[FRAMES],
        );
        assert_eq!(
            trace.iter().map(|n| n.key.as_u8()).collect::<Vec<_>>(),
            vec![64, second_key]
        );
    }
}
#[test]
fn excessive_authored_declarations_refuse_without_overflowing_the_ir_builder() {
    let program = ScriptIdentity::new(SCRIPT, ScriptStateId::new(42), ProjectSeed::new(99))
        .compile_note("out.dur = 1", profile().capabilities().sample_rate(), &[])
        .unwrap();
    let ir = GraphIr::builder()
        .script(program, ExecutionScope::Global)
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(1),
                simultaneous_holds: EventCount::NONE,
            }],
            authored_sources: vec![
                AuthoredSourceDeclaration {
                    producer: ProducerId::new(0),
                    destination_occupancy: EventCount::measured(u32::MAX),
                    retained_future: EventCount::NONE,
                    simultaneous_holds: EventCount::NONE
                };
                3
            ],
            ..PlanDeclarations::default()
        })
        .build()
        .expect("the IR builder must not panic on an excessive declaration");
    assert!(
        compile(&ir, &RenderConfig::new(profile()))
            .into_plan()
            .is_err()
    );
    assert_eq!(
        ir.declarations().programs[0].evaluations_per_quantum(),
        u32::MAX
    );
}
