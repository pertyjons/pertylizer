//! P07-S005: fixed-quantum YAMS, stable local keys and explicit scalar layers.
mod common;
use synth_engine_v2::{
    ir::{ExecutionScope, GraphIr, GraphIrBuilder, IrNodeKind, NodeId, PortId, SignalDomain},
    node::AMPLIFIER_CONTROL,
    offline::render_offline,
    quantities::{Amplitude, ChannelLayout, SampleRate},
    script::{ProjectSeed, ScriptIdentity, ScriptProgram, ScriptStateId},
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES},
};
const SCRIPT: NodeId = NodeId::new(10);
const ONE: NodeId = NodeId::new(11);
const AMP: NodeId = NodeId::new(12);
const OUT: NodeId = NodeId::new(13);
const Q: usize = QUANTUM_FRAMES as usize;
const FRAMES: u64 = 1024;
fn identity() -> ScriptIdentity {
    ScriptIdentity::new(SCRIPT, ScriptStateId::new(21), ProjectSeed::new(34))
}
fn program(source: &str) -> ScriptProgram {
    identity()
        .compile_control(source, SampleRate::new(48_000.0).expect("rate"), &[])
        .expect("program")
}
fn reading(program: ScriptProgram) -> GraphIrBuilder {
    GraphIr::builder()
        .script(program, ExecutionScope::Global)
        .node(
            ONE,
            IrNodeKind::Constant {
                level: Amplitude::UNITY,
            },
            ExecutionScope::Global,
        )
        .node(AMP, IrNodeKind::Amplifier, ExecutionScope::Global)
        .node(OUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (ONE, PortId::FIRST),
            (AMP, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SCRIPT, PortId::FIRST),
            (AMP, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMP, PortId::FIRST),
            (OUT, PortId::FIRST),
            SignalDomain::Audio,
        )
}
#[test]
fn local_parameter_default_and_fixed_clock_are_observable() {
    let ir = reading(program("param step = 0.125 [0, 1]\nout = accum(step)"))
        .build()
        .expect("IR");
    let plan = common::admit(&ir, common::profile(FRAMES, ChannelLayout::Mono));
    let output =
        render_offline(plan, FrameCount::new(FRAMES), PlanPosition::ZERO, &[]).expect("render");
    assert!(output.iter().any(|sample| *sample != 0.0));
    for (index, quantum) in output.as_chunks::<Q>().0.iter().enumerate() {
        assert!(quantum.iter().all(|sample| *sample == quantum[0]));
        assert_eq!(quantum[0], (index + 1) as f32 * 0.125);
    }
}
#[test]
fn local_parameter_identity_survives_reordering_and_never_retargets_renames() {
    let mut identity = identity();
    let rate = SampleRate::new(48_000.0).expect("rate");
    let before = identity
        .compile_control("param a = 0.2\nparam b = 0.3\nout = a + b", rate, &[])
        .expect("before");
    let a = before.parameters()[0].id;
    let b = before.parameters()[1].id;
    let reordered = identity
        .compile_control("param b = 0.4\nparam a = 0.5\nout = a + b", rate, &[])
        .expect("reordered");
    assert_eq!(reordered.parameters()[0].id, b);
    assert_eq!(reordered.parameters()[1].id, a);
    let renamed = identity
        .compile_control("param c = 0.5\nout = c", rate, &[])
        .expect("renamed");
    assert_ne!(renamed.parameters()[0].id, a);
    assert_ne!(renamed.parameters()[0].id, b);
    let plan = common::admit(
        &reading(renamed).build().expect("IR"),
        common::profile(FRAMES, ChannelLayout::Mono),
    );
    assert!(plan.resolve_parameter(SCRIPT, a).is_none());
    assert!(plan.resolve_parameter(SCRIPT, b).is_none());
}

fn module_binding(
    source: synth_engine_v2::script::ScriptSource,
) -> synth_engine_v2::script::ScriptBinding {
    synth_engine_v2::script::ScriptBinding {
        input: synth_script::compile::SourceInput::Module {
            module: "lfo".to_owned(),
            instance: 1,
            member: "out".to_owned(),
        },
        source,
    }
}
fn bound(source: &str, binding: synth_engine_v2::script::ScriptSource) -> ScriptProgram {
    identity()
        .compile_control(
            source,
            SampleRate::new(48_000.0).expect("rate"),
            &[module_binding(binding)],
        )
        .expect("bound program")
}
fn partitioned(
    plan: &synth_engine_v2::plan::CompiledPlan,
    events: &[synth_engine_v2::schedule::PlanEvent],
    blocks: &[usize],
) -> Vec<u32> {
    use synth_engine_v2::{
        publish::PublicationArbiter,
        render::AudioBlockMut,
        schedule::{AdmittedCompiledStream, CompiledEventScheduler},
        stream::StreamControl,
        time::{SampleTime, StreamAnchor},
    };
    let (mut control, mut renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("stream");
    let stream = AdmittedCompiledStream::admit(plan, events).expect("events");
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &stream).expect("schedule");
    let mut arbiter = PublicationArbiter::prepare(&common::profile(FRAMES, ChannelLayout::Mono))
        .expect("publisher");
    let mut output = Vec::new();
    for block in blocks {
        let mut samples = vec![0.0; *block];
        scheduler
            .render(
                &mut renderer,
                &mut arbiter,
                AudioBlockMut::new(&mut samples, *block, ChannelLayout::Mono).expect("block"),
            )
            .expect("render");
        output.extend(samples.into_iter().map(f32::to_bits));
    }
    output
}
#[test]
fn time_varying_control_and_local_automation_are_partition_invariant() {
    use synth_engine_v2::{
        quantities::ParameterValue,
        schedule::{CompiledPayload, PlanEvent},
    };
    let program = program("param step = 0.125 [0, 1]\nout = accum(step)");
    let parameter = program.parameters()[0].id;
    let plan = common::admit(
        &reading(program).build().expect("IR"),
        common::profile(FRAMES, ChannelLayout::Mono),
    );
    let events = [PlanEvent::new(
        PlanPosition::new(65),
        CompiledPayload::SetParameter {
            slot: plan.resolve_parameter(SCRIPT, parameter).expect("knob"),
            value: ParameterValue::new(0.25).expect("value"),
        },
    )];
    let reference = partitioned(&plan, &events, &[FRAMES as usize]);
    for blocks in [
        vec![Q; FRAMES as usize / Q],
        vec![1; FRAMES as usize],
        vec![7, 65, 1, 300, 651],
    ] {
        assert_eq!(partitioned(&plan, &events, &blocks), reference);
    }
    assert_eq!(f32::from_bits(reference[Q]), 0.125);
    assert_eq!(f32::from_bits(reference[2 * Q]), 0.25);
    assert_eq!(f32::from_bits(reference[3 * Q]), 0.5);
}
#[test]
fn previous_resolved_reads_last_quantum_before_future_boundary_writes() {
    use synth_engine_v2::{
        quantities::ParameterValue,
        schedule::{CompiledPayload, PlanEvent},
        script::{ParameterRead, ScriptSource},
    };
    let source = "src previous = lfo-1.out\nparam value = 0.125 [0, 1]\nout = previous";
    let initial = identity()
        .compile_control(
            "param value = 0.125 [0, 1]\nout = value",
            SampleRate::new(48_000.0).expect("rate"),
            &[],
        )
        .expect("parameter key");
    let parameter = initial.parameters()[0].id;
    let program = bound(
        source,
        ScriptSource::Parameter {
            node: SCRIPT,
            parameter,
            read: ParameterRead::PreviousResolved,
        },
    );
    let plan = common::admit(
        &reading(program).build().expect("IR"),
        common::profile(FRAMES, ChannelLayout::Mono),
    );
    let events = [PlanEvent::new(
        PlanPosition::new(64),
        CompiledPayload::SetParameter {
            slot: plan.resolve_parameter(SCRIPT, parameter).expect("knob"),
            value: ParameterValue::new(0.75).expect("value"),
        },
    )];
    let reference = partitioned(&plan, &events, &[FRAMES as usize]);
    assert_eq!(
        partitioned(&plan, &events, &vec![1; FRAMES as usize]),
        reference
    );
    assert_eq!(f32::from_bits(reference[Q]), 0.125);
    assert_eq!(f32::from_bits(reference[2 * Q]), 0.125);
    assert_eq!(f32::from_bits(reference[3 * Q]), 0.75);
}
#[test]
fn signals_schedule_before_a_script_modulating_a_native_parameter() {
    use synth_engine_v2::{
        controller::ControllerKind,
        ir::{ModulationDepth, ModulationUnit, parameters},
        quantities::Frequency,
        script::ScriptSource,
    };
    let controller = NodeId::new(40);
    let sine = NodeId::new(41);
    let program = bound(
        "src x = lfo-1.out\nout = x + 1",
        ScriptSource::Signal {
            node: controller,
            port: PortId::FIRST,
        },
    );
    let kind = IrNodeKind::Sine {
        frequency: Frequency::new(220.0).expect("frequency"),
        amplitude: Amplitude::UNITY,
    };
    let ir = GraphIr::builder()
        .script(program, ExecutionScope::Global)
        .node(
            controller,
            IrNodeKind::Controller {
                kind: ControllerKind::ModWheel,
            },
            ExecutionScope::Global,
        )
        .node(sine, kind, ExecutionScope::Global)
        .node(OUT, IrNodeKind::Output, ExecutionScope::Global)
        .modulate(
            (SCRIPT, PortId::FIRST),
            (sine, parameters::SINE_FREQUENCY),
            ModulationDepth::new(ModulationUnit::Semitones, 12.0).expect("depth"),
        )
        .connect(
            (sine, PortId::FIRST),
            (OUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("IR");
    let plan = common::admit(&ir, common::profile(FRAMES, ChannelLayout::Mono));
    let expected = common::admit(
        &common::source_plan(IrNodeKind::Sine {
            frequency: Frequency::new(440.0).expect("frequency"),
            amplitude: Amplitude::UNITY,
        }),
        common::profile(FRAMES, ChannelLayout::Mono),
    );
    assert_eq!(
        partitioned(&plan, &[], &[FRAMES as usize]),
        partitioned(&expected, &[], &[FRAMES as usize])
    );
}

#[test]
fn binding_failures_name_the_authored_source() {
    use synth_engine_v2::{
        diagnostics::CompileError,
        ir::IrError,
        script::{ScriptBindingFault, ScriptSource},
    };
    let source = "src x = lfo-1.out\nout = x";
    let unknown_parameter_node = bound(
        source,
        ScriptSource::Parameter {
            node: NodeId::new(99),
            parameter: synth_engine_v2::ir::ParameterId::new(0),
            read: synth_engine_v2::script::ParameterRead::Base,
        },
    );
    assert!(
        matches!(reading(unknown_parameter_node).build(), Err(IrError::ScriptBinding {
        span, reason: ScriptBindingFault::MissingNode { source_node }, ..
    }) if source_node == NodeId::new(99) && span.start > 0)
    );
    let unknown = bound(
        source,
        ScriptSource::Signal {
            node: NodeId::new(99),
            port: PortId::FIRST,
        },
    );
    assert!(
        matches!(reading(unknown).build(), Err(IrError::ScriptBinding { node: SCRIPT, span, reason: ScriptBindingFault::MissingNode { .. } }) if span.start > 0 && span.end > span.start)
    );
    let cycle = bound(
        source,
        ScriptSource::Signal {
            node: SCRIPT,
            port: PortId::FIRST,
        },
    );
    let ir = reading(cycle).build().expect("IR");
    assert!(
        matches!(common::refuse(&ir, common::profile(FRAMES, ChannelLayout::Mono)), CompileError::ScriptBinding { node: SCRIPT, span, reason: ScriptBindingFault::Cycle { .. } } if span.start > 0)
    );
    let narrow = NodeId::new(99);
    let scope = bound(
        source,
        ScriptSource::Signal {
            node: narrow,
            port: PortId::FIRST,
        },
    );
    let ir = reading(scope)
        .node(
            narrow,
            IrNodeKind::Controller {
                kind: synth_engine_v2::controller::ControllerKind::ModWheel,
            },
            ExecutionScope::Voice,
        )
        .build()
        .expect("IR");
    assert!(
        matches!(common::refuse(&ir, common::profile(FRAMES, ChannelLayout::Mono)), CompileError::ScriptBinding { node: SCRIPT, span, reason: ScriptBindingFault::Scope { .. } } if span.start > 0)
    );
}
#[test]
fn explicit_base_and_automated_sources_select_distinct_layers() {
    use synth_engine_v2::{
        ir::ParameterId,
        quantities::ParameterValue,
        schedule::{CompiledPayload, PlanEvent},
        script::{ParameterRead, ScriptSource},
    };
    for (read, expected) in [
        (ParameterRead::Base, 0.125),
        (ParameterRead::Automated, 0.75),
    ] {
        let program = bound(
            "src x = lfo-1.out\nparam value = 0.125 [0, 1]\nout = x",
            ScriptSource::Parameter {
                node: SCRIPT,
                parameter: ParameterId::FIRST,
                read,
            },
        );
        let plan = common::admit(
            &reading(program).build().expect("IR"),
            common::profile(FRAMES, ChannelLayout::Mono),
        );
        let events = [PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::SetParameter {
                slot: plan
                    .resolve_parameter(SCRIPT, ParameterId::FIRST)
                    .expect("knob"),
                value: ParameterValue::new(0.75).expect("value"),
            },
        )];
        let out = partitioned(&plan, &events, &[FRAMES as usize]);
        assert_eq!(f32::from_bits(out[Q]), expected);
    }
}

#[test]
fn a_modulation_cycle_without_a_script_source_keeps_its_graph_diagnostic() {
    use synth_engine_v2::{
        diagnostics::CompileError,
        ir::{ModulationDepth, ModulationUnit},
    };
    let script = program("param value = 0.125 [0, 1]\nout = value");
    let parameter = script.parameters()[0].id;
    let ir = reading(script)
        .modulate(
            (SCRIPT, PortId::FIRST),
            (SCRIPT, parameter),
            ModulationDepth::new(ModulationUnit::Physical, 1.0).expect("depth"),
        )
        .build()
        .expect("IR");
    assert!(matches!(
        common::refuse(&ir, common::profile(FRAMES, ChannelLayout::Mono)),
        CompileError::ModulationCycle { .. }
    ));
}
#[test]
fn random_program_seed_survives_recompile_and_declaration_order() {
    let first = program("out = rand(0, 1)");
    let ir = reading(first.clone())
        .node(NodeId::new(99), IrNodeKind::Silence, ExecutionScope::Global)
        .build()
        .expect("IR");
    let make = || common::admit(&ir, common::profile(FRAMES, ChannelLayout::Mono));
    let reference = partitioned(&make(), &[], &[FRAMES as usize]);
    assert_eq!(
        partitioned(&make(), &[], &vec![1; FRAMES as usize]),
        reference
    );
    let mut reordered = GraphIr::builder();
    for node in ir.nodes().iter().rev().filter(|node| node.id() != SCRIPT) {
        reordered = reordered.node(node.id(), node.kind(), node.scope());
    }
    reordered = reordered.script(first, ExecutionScope::Global);
    // script() restores only script source dependencies; this fixture has none.
    for edge in ir.edges() {
        reordered = reordered.connect(edge.from(), edge.to(), edge.domain());
    }
    let plan = common::admit(
        &reordered.build().expect("IR"),
        common::profile(FRAMES, ChannelLayout::Mono),
    );
    assert_eq!(partitioned(&plan, &[], &[FRAMES as usize]), reference);
    assert_ne!(reference[Q], reference[2 * Q]);
}

#[test]
fn local_knobs_compose_automation_and_modulation_in_the_declared_range() {
    use synth_engine_v2::{
        ir::{ModulationDepth, ModulationUnit},
        quantities::ParameterValue,
        schedule::{CompiledPayload, PlanEvent},
    };
    let program = program("param value = 0.125 [0, 0.8]\nout = value");
    let parameter = program.parameters()[0].id;
    let other = NodeId::new(90);
    let mut identity = ScriptIdentity::new(other, ScriptStateId::new(1), ProjectSeed::new(1));
    let modulation = identity
        .compile_control("out = 1", SampleRate::new(48000.0).expect("rate"), &[])
        .expect("modulation");
    let ir = reading(program)
        .script(modulation, ExecutionScope::Global)
        .modulate(
            (other, PortId::FIRST),
            (SCRIPT, parameter),
            ModulationDepth::new(ModulationUnit::Physical, 0.25).expect("depth"),
        )
        .build()
        .expect("IR");
    let plan = common::admit(&ir, common::profile(FRAMES, ChannelLayout::Mono));
    let events = [PlanEvent::new(
        PlanPosition::new(64),
        CompiledPayload::SetParameter {
            slot: plan.resolve_parameter(SCRIPT, parameter).expect("knob"),
            value: ParameterValue::new(0.75).expect("value"),
        },
    )];
    let result = partitioned(&plan, &events, &[FRAMES as usize]);
    assert_eq!(f32::from_bits(result[Q]), 0.375);
    assert_eq!(f32::from_bits(result[2 * Q]), 0.8);
}
#[test]
fn installed_bytecode_supplies_its_actual_resource_declaration() {
    use synth_engine_v2::{
        compile::{RenderConfig, compile},
        ir::{IrProgram, PlanDeclarations, ProgramId},
        quantities::{InstructionCount, SlotCount},
        report::{ResourceAmount, ResourceField},
    };
    let program = program("param step = 0.125\nout = accum(step) + rand(0, 1)");
    let actual = program.work();
    assert!(actual.instructions().get() > 1);
    let forged = IrProgram::new(
        ProgramId::new(SCRIPT.as_raw()),
        InstructionCount::NONE,
        SlotCount::NONE,
        SlotCount::NONE,
        SlotCount::NONE,
        SlotCount::NONE,
        SlotCount::NONE,
        SlotCount::NONE,
        SlotCount::NONE,
        0,
    );
    let ir = reading(program)
        .declaring(PlanDeclarations {
            programs: vec![forged],
            ..PlanDeclarations::default()
        })
        .build()
        .expect("IR");
    let outcome = compile(
        &ir,
        &RenderConfig::new(common::profile(FRAMES, ChannelLayout::Mono)),
    );
    assert!(outcome.plan().is_ok());
    assert_eq!(
        outcome
            .report()
            .row(ResourceField::MaxInstructionsPerProgram)
            .expect("row")
            .requested(),
        ResourceAmount::Instructions(actual.instructions())
    );
    assert_eq!(ir.declarations().programs.len(), 1);
    assert_eq!(
        ir.declarations().programs[0].instructions(),
        actual.instructions()
    );
}
#[test]
fn failed_compile_preserves_the_parameter_namespace() {
    let mut identity = identity();
    let initial = identity.clone();
    assert!(
        identity
            .compile_control(
                "param new_name = 0.5\nout = velocity",
                SampleRate::new(48000.0).expect("rate"),
                &[]
            )
            .is_err()
    );
    assert_eq!(identity, initial);
    let program = identity
        .compile_control(
            "param accepted = 0.5\nout = accepted",
            SampleRate::new(48000.0).expect("rate"),
            &[],
        )
        .expect("valid");
    assert_eq!(
        program.parameters()[0].id,
        synth_engine_v2::ir::ParameterId::FIRST
    );
}

#[test]
fn all_32_declared_signal_sources_are_bound_and_evaluated() {
    use synth_engine_v2::{
        controller::ControllerKind,
        ir::parameters,
        quantities::ParameterValue,
        schedule::{CompiledPayload, PlanEvent},
        script::{ScriptBinding, ScriptSource},
    };
    let controller = NodeId::new(80);
    let mut source = String::new();
    let mut bindings = Vec::new();
    for index in 1..=32 {
        source.push_str(&format!("src x{index} = lfo-{index}.out\n"));
        bindings.push(ScriptBinding {
            input: synth_script::compile::SourceInput::Module {
                module: "lfo".to_owned(),
                instance: index,
                member: "out".to_owned(),
            },
            source: ScriptSource::Signal {
                node: controller,
                port: PortId::FIRST,
            },
        });
    }
    source.push_str("out = ");
    source.push_str(
        &(1..=32)
            .map(|index| format!("x{index}"))
            .collect::<Vec<_>>()
            .join(" + "),
    );
    let program = identity()
        .compile_control(&source, SampleRate::new(48000.0).expect("rate"), &bindings)
        .expect("all sources");
    assert_eq!(program.work().sources().get(), 32);
    let ir = reading(program)
        .node(
            controller,
            IrNodeKind::Controller {
                kind: ControllerKind::ModWheel,
            },
            ExecutionScope::Global,
        )
        .build()
        .expect("IR");
    let plan = common::admit(&ir, common::profile(FRAMES, ChannelLayout::Mono));
    let event = PlanEvent::new(
        PlanPosition::ZERO,
        CompiledPayload::SetParameter {
            slot: plan
                .resolve_parameter(controller, parameters::SOURCE_VALUE)
                .expect("source"),
            value: ParameterValue::new(0.125).expect("value"),
        },
    );
    let samples = partitioned(&plan, &[event], &[FRAMES as usize]);
    assert_eq!(f32::from_bits(samples[Q]), 4.0);
}
