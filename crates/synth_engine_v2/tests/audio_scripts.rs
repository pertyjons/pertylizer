//! P07-S006: sample cadence, explicit channels and countable maximum-polyphony work.
mod common;
use synth_engine_v2::render::Renderer;
use synth_engine_v2::{
    ir::{ExecutionScope, GraphIr, GraphIrBuilder, IrNodeKind, NodeId, PortId, SignalDomain},
    quantities::ChannelLayout,
    script::{
        ProjectSeed, ScriptBinding, ScriptChannel, ScriptIdentity, ScriptProgram, ScriptSource,
        ScriptStateId,
    },
    time::{FrameCount, PlanPosition},
};
const SCRIPT: NodeId = NodeId::new(30);
const OUTPUT: NodeId = NodeId::new(31);
const INPUT: NodeId = NodeId::new(32);
const FRAMES: usize = 512;
fn audio(
    node: NodeId,
    source: &str,
    layout: ChannelLayout,
    bindings: &[ScriptBinding],
) -> ScriptProgram {
    ScriptIdentity::new(node, ScriptStateId::new(1), ProjectSeed::new(2))
        .compile_audio(source, common::rate(48000.0), layout, bindings)
        .expect("program")
}
fn graph(program: ScriptProgram, scope: ExecutionScope) -> GraphIrBuilder {
    GraphIr::builder()
        .script(program, scope)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SCRIPT, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
}
fn render(ir: &GraphIr, layout: ChannelLayout) -> Vec<f32> {
    synth_engine_v2::offline::render_offline(
        common::admit(ir, common::profile(FRAMES as u64, layout)),
        FrameCount::new(FRAMES as u64),
        PlanPosition::ZERO,
        &[],
    )
    .expect("render")
}
#[test]
fn audio_state_advances_per_sample_under_every_partition() {
    use synth_engine_v2::{
        render::{AudioBlockMut, TimedEvents},
        stream::StreamControl,
        time::{SampleTime, StreamAnchor},
    };
    let ir = graph(
        audio(SCRIPT, "out = accum(0.125)", ChannelLayout::Mono, &[]),
        ExecutionScope::Global,
    )
    .build()
    .expect("IR");
    let plan = common::admit(&ir, common::profile(FRAMES as u64, ChannelLayout::Mono));
    let expected: Vec<_> = (0..FRAMES)
        .map(|i| {
            if i < 64 {
                0.0_f32.to_bits()
            } else {
                ((i - 63) as f32 * 0.125).to_bits()
            }
        })
        .collect();
    for chunks in [
        vec![FRAMES],
        vec![1; FRAMES],
        vec![64; 8],
        vec![7, 59, 1, 128, 317],
    ] {
        let (_, mut renderer) = StreamControl::open(
            plan.clone(),
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .expect("stream");
        let mut output = Vec::new();
        for frames in chunks {
            let mut buffer = vec![0.0; frames];
            renderer
                .render(
                    AudioBlockMut::new(&mut buffer, frames, ChannelLayout::Mono).expect("block"),
                    TimedEvents::EMPTY,
                )
                .expect("render");
            output.extend(buffer.into_iter().map(f32::to_bits));
        }
        assert_eq!(output, expected);
    }
}
#[test]
fn stereo_bindings_read_both_channels_at_each_sample() {
    use synth_core::script::AudioInputChannel;
    let bindings = [
        ScriptBinding {
            input: synth_script::compile::SourceInput::AudioIn(AudioInputChannel::Left),
            source: ScriptSource::AudioSignal {
                node: INPUT,
                port: PortId::FIRST,
                layout: ChannelLayout::Stereo,
                channel: ScriptChannel::Left,
            },
        },
        ScriptBinding {
            input: synth_script::compile::SourceInput::AudioIn(AudioInputChannel::Right),
            source: ScriptSource::AudioSignal {
                node: INPUT,
                port: PortId::FIRST,
                layout: ChannelLayout::Stereo,
                channel: ScriptChannel::Right,
            },
        },
    ];
    let ir = graph(
        audio(
            SCRIPT,
            "out.left = in_r * 2\nout.right = in_l + 1",
            ChannelLayout::Stereo,
            &bindings,
        ),
        ExecutionScope::Global,
    )
    .script(
        audio(
            INPUT,
            "out.left = accum(0.125)\nout.right = accum(0.25)",
            ChannelLayout::Stereo,
            &[],
        ),
        ExecutionScope::Global,
    )
    .build()
    .expect("IR");
    for (index, frame) in render(&ir, ChannelLayout::Stereo)
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
    {
        assert_eq!(
            *frame,
            [(index + 1) as f32 * 0.5, (index + 1) as f32 * 0.125 + 1.0]
        );
    }
}
#[test]
fn bare_stereo_duplicates_but_an_unwritten_channel_is_silent() {
    for (source, expected) in [
        ("out = 0.25", [0.25, 0.25]),
        ("out.left = 0.25", [0.25, 0.0]),
        ("out.right = 0.25", [0.0, 0.25]),
    ] {
        let ir = graph(
            audio(SCRIPT, source, ChannelLayout::Stereo, &[]),
            ExecutionScope::Global,
        )
        .build()
        .expect("IR");
        assert!(
            render(&ir, ChannelLayout::Stereo)
                .as_chunks::<2>()
                .0
                .iter()
                .all(|frame| *frame == expected)
        );
    }
}
#[test]
fn first_sample_and_time_integration_use_the_audio_clock() {
    let ir = graph(
        audio(
            SCRIPT,
            "out = first_sample + phasor(375)",
            ChannelLayout::Mono,
            &[],
        ),
        ExecutionScope::Global,
    )
    .build()
    .expect("IR");
    let result = render(&ir, ChannelLayout::Mono);
    assert_eq!(result[0], 1.0 + 1.0 / 128.0);
    // An exact binary phase increment, independent of libm rounding.
    for (index, pair) in result.windows(2).enumerate().skip(1) {
        let expected = if pair[0] + 1.0 / 128.0 >= 1.0 {
            pair[0] + 1.0 / 128.0 - 1.0
        } else {
            pair[0] + 1.0 / 128.0
        };
        assert_eq!(pair[1], expected, "frame {}", index + 1);
    }
}
#[test]
fn audio_work_uses_actual_cadence_and_the_configured_voice_ceiling() {
    use synth_engine_v2::{
        compile::{RenderConfig, compile},
        diagnostics::CompileWarning,
        ir::{NoteProducerDeclaration, PlanDeclarations},
        quantities::{EventCount, HeldNoteCount},
    };
    let program = audio(
        SCRIPT,
        "arr scale = [0, 2, 4, 5, 7, 9, 11]\nout = scale_snap(accum(0.125), scale)",
        ChannelLayout::Mono,
        &[],
    );
    let instructions = program.work().instructions().get();
    assert_eq!(
        program.vm_work_per_evaluation().get(),
        u64::from(instructions) + 3 * 7
    );
    let profile = common::profile(FRAMES as u64, ChannelLayout::Mono);
    let estimate = program
        .audio_cost(ExecutionScope::Voice, &profile)
        .expect("Audio cost");
    assert_eq!(
        estimate.voices,
        profile.limits().voices().maximum_voices_per_instrument()
    );
    assert_eq!(
        estimate.per_quantum.get(),
        (u64::from(instructions) + 21) * 64 * u64::from(estimate.voices.get())
    );
    let ir = graph(program, ExecutionScope::Voice)
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(2),
                simultaneous_holds: EventCount::NONE,
            }],
            ..PlanDeclarations::default()
        })
        .build()
        .expect("IR");
    assert_eq!(
        ir.script_instructions_per_quantum().0.get(),
        u64::from(instructions) * 64 * 2
    );
    let outcome = compile(&ir, &RenderConfig::new(profile));
    assert!(outcome.warnings().iter().any(|warning| matches!(warning, CompileWarning::AudioScriptWork { estimate: found } if *found == estimate)));
    assert!(outcome.into_plan().is_ok(), "the work warning is advisory");
}
#[test]
fn output_and_input_shape_mismatches_are_refused() {
    let mut identity = ScriptIdentity::new(SCRIPT, ScriptStateId::new(1), ProjectSeed::new(2));
    assert!(
        identity
            .compile_audio(
                "out.right = 1",
                common::rate(48000.0),
                ChannelLayout::Mono,
                &[]
            )
            .is_err()
    );
    let bindings = [ScriptBinding {
        input: synth_script::compile::SourceInput::AudioIn(
            synth_core::script::AudioInputChannel::Right,
        ),
        source: ScriptSource::AudioSignal {
            node: INPUT,
            port: PortId::FIRST,
            layout: ChannelLayout::Mono,
            channel: ScriptChannel::Right,
        },
    }];
    let ir = graph(
        audio(SCRIPT, "out = in_r", ChannelLayout::Mono, &bindings),
        ExecutionScope::Global,
    )
    .script(
        audio(INPUT, "out = 1", ChannelLayout::Mono, &[]),
        ExecutionScope::Global,
    )
    .build()
    .expect("IR");
    assert!(matches!(
        common::refuse(&ir, common::profile(FRAMES as u64, ChannelLayout::Mono)),
        synth_engine_v2::diagnostics::CompileError::ScriptBinding {
            reason: synth_engine_v2::script::ScriptBindingFault::AudioSignal { .. },
            ..
        }
    ));
}

#[test]
fn audio_local_automation_uses_the_central_quantum_parameter_slot() {
    use synth_engine_v2::{quantities::ParameterValue, schedule::CompiledPayload};
    let program = audio(
        SCRIPT,
        "param step = 0.125 [0, 1]\nout = accum(step)",
        ChannelLayout::Mono,
        &[],
    );
    let parameter = program.parameters()[0].id;
    let ir = graph(program, ExecutionScope::Global).build().expect("IR");
    let plan = common::admit(&ir, common::profile(FRAMES as u64, ChannelLayout::Mono));
    let event = synth_engine_v2::offline::OfflineEvent::new(
        synth_engine_v2::time::SampleTime::new(65),
        CompiledPayload::SetParameter {
            slot: plan
                .resolve_parameter(SCRIPT, parameter)
                .expect("ordinary parameter slot"),
            value: ParameterValue::new(0.25).expect("value"),
        },
    );
    let output = synth_engine_v2::offline::render_offline(
        plan,
        FrameCount::new(FRAMES as u64),
        PlanPosition::ZERO,
        &[event],
    )
    .expect("render");
    for (frame, value) in output.into_iter().enumerate() {
        let expected = if frame < 128 {
            (frame + 1) as f32 * 0.125
        } else {
            16.0 + (frame - 127) as f32 * 0.25
        };
        assert_eq!(value, expected, "frame {frame}");
    }
}

#[test]
fn audio_signal_cycles_and_scope_errors_retain_the_source_span() {
    use synth_engine_v2::{diagnostics::CompileError, script::ScriptBindingFault};
    let binding = |node| ScriptBinding {
        input: synth_script::compile::SourceInput::Module {
            module: "lfo".to_owned(),
            instance: 1,
            member: "out".to_owned(),
        },
        source: ScriptSource::AudioSignal {
            node,
            port: PortId::FIRST,
            layout: ChannelLayout::Mono,
            channel: ScriptChannel::Left,
        },
    };
    let source = "src x = lfo-1.out\nout = x";
    let cycle = graph(
        audio(SCRIPT, source, ChannelLayout::Mono, &[binding(SCRIPT)]),
        ExecutionScope::Global,
    )
    .build()
    .expect("IR");
    assert!(
        matches!(common::refuse(&cycle, common::profile(FRAMES as u64, ChannelLayout::Mono)), CompileError::ScriptBinding { span, reason: ScriptBindingFault::Cycle { .. }, .. } if span.start > 0)
    );
    let scope = graph(
        audio(SCRIPT, source, ChannelLayout::Mono, &[binding(INPUT)]),
        ExecutionScope::Global,
    )
    .script(
        audio(INPUT, "out = 1", ChannelLayout::Mono, &[]),
        ExecutionScope::Voice,
    )
    .build()
    .expect("IR");
    assert!(
        matches!(common::refuse(&scope, common::profile(FRAMES as u64, ChannelLayout::Mono)), CompileError::ScriptBinding { span, reason: ScriptBindingFault::Scope { .. }, .. } if span.start > 0)
    );
}
