//! Explicit terminal folding: frame/channel arithmetic, admission and host partitions.
mod common;

use synth_engine_v2::{
    controller::BipolarLevel,
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    node::kernels::{self, InputBuffer, MAX_INPUTS, NodeIo, NodeState, PreparedNode},
    offline::render_offline,
    plan::{CompiledPlan, PlanOp},
    quantities::{Amplitude, ChannelLayout, Frequency},
    render::{AudioBlockMut, Renderer, TimedEvents},
    stream::StreamControl,
    time::{FrameCount, PlanPosition, QuantumTimeline, SampleTime, StreamAnchor},
};

const BALANCE: NodeId = NodeId::new(3);
const FOLD: NodeId = NodeId::new(4);
const FRAMES: u64 = 513;

fn kernel(source: &[f32], out: &mut [f32]) -> allocation_counter::AllocationInfo {
    let prepared = PreparedNode::Copy;
    let mut state = NodeState::initial(&prepared);
    let mut inputs = [InputBuffer::Unpatched; MAX_INPUTS];
    inputs[0] = InputBuffer::Patched(source);
    let mut io = NodeIo {
        out,
        channels: ChannelLayout::Mono,
        inputs,
        timeline: QuantumTimeline::linear(None),
        controls: &[],
        ramps: &[],
        history: &mut [],
        samples: &[],
        scripts: synth_engine_v2::script::ScriptResources::default(),
    };
    allocation_counter::measure(|| kernels::STEREO_TO_MONO.run(&prepared, &mut state, &mut io))
}

#[test]
fn literal_frames_pin_half_sum_headroom_subnormals_and_channel_symmetry_without_heap() {
    let source = [
        1.0,
        1.0,
        1.0,
        0.0,
        0.0,
        1.0,
        1.0,
        -1.0,
        -4.0,
        2.0,
        4.0,
        4.0,
        f32::MAX,
        f32::MAX,
        -f32::MAX,
        -f32::MAX,
        f32::MAX,
        -f32::MAX,
        f32::from_bits(1),
        f32::from_bits(1),
        f32::from_bits(3),
        f32::from_bits(3),
        1.0,
        -0.0,
        -0.0,
        -0.0,
        f32::from_bits(1),
        0.0,
    ];
    // Wider intermediates preserve dual-mono even at the minimum subnormal.
    let expected = [
        1.0_f32,
        0.5,
        0.5,
        0.0,
        -1.0,
        4.0,
        f32::MAX,
        -f32::MAX,
        0.0,
        f32::from_bits(1),
        f32::from_bits(3),
        0.5,
        -0.0,
        0.0,
    ];
    let mut out = [99.0; 14];
    let allocations = kernel(&source, &mut out);
    assert_eq!(allocations.count_total, 0);
    assert_eq!(allocations.count_current, 0);
    assert_eq!(allocations.bytes_current, 0);
    assert_eq!(out.map(f32::to_bits), expected.map(f32::to_bits));
    let swapped: Vec<_> = source
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|[l, r]| [*r, *l])
        .collect();
    let mut reversed = [99.0; 14];
    let _allocations = kernel(&swapped, &mut reversed);
    assert_eq!(out.map(f32::to_bits), reversed.map(f32::to_bits));
}

#[test]
fn unpatched_input_writes_defined_silence() {
    let prepared = PreparedNode::Copy;
    let mut state = NodeState::initial(&prepared);
    let mut out = [99.0; 64];
    let mut io = NodeIo {
        out: &mut out,
        channels: ChannelLayout::Mono,
        inputs: [InputBuffer::Unpatched; MAX_INPUTS],
        timeline: QuantumTimeline::linear(None),
        controls: &[],
        ramps: &[],
        history: &mut [],
        samples: &[],
        scripts: synth_engine_v2::script::ScriptResources::default(),
    };
    kernels::STEREO_TO_MONO.run(&prepared, &mut state, &mut io);
    assert_eq!(out.map(f32::to_bits), [0.0_f32.to_bits(); 64]);
}

fn graph(fold: bool) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(
            common::SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(317.0).expect("frequency"),
                amplitude: Amplitude::new(3.0).expect("float headroom"),
            },
            ExecutionScope::Global,
        )
        .node(
            BALANCE,
            IrNodeKind::Balance {
                level: Amplitude::UNITY,
                pan: BipolarLevel::new(0.37).expect("pan"),
                muted: false,
            },
            ExecutionScope::Global,
        )
        .node(common::OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (common::SOURCE, PortId::FIRST),
            (BALANCE, PortId::FIRST),
            SignalDomain::Audio,
        );
    if fold {
        builder = builder
            .node(FOLD, IrNodeKind::StereoToMono, ExecutionScope::Global)
            .connect(
                (BALANCE, PortId::FIRST),
                (FOLD, PortId::FIRST),
                SignalDomain::Audio,
            );
    }
    builder
        .connect(
            (if fold { FOLD } else { BALANCE }, PortId::FIRST),
            (common::OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("graph")
}

fn render(plan: CompiledPlan) -> Vec<f32> {
    render_offline(plan, FrameCount::new(FRAMES), PlanPosition::ZERO, &[]).expect("render")
}

#[test]
#[allow(
    clippy::cast_possible_truncation,
    reason = "the finite f32 mean fits f32; oracle rounds once"
)]
fn admitted_fold_is_a_separate_scheduled_region_and_equals_the_stereo_oracle() {
    let stereo = common::admit(&graph(false), common::profile(256, ChannelLayout::Stereo));
    let mono = common::admit(&graph(true), common::profile(256, ChannelLayout::Mono));
    let folds: Vec<_> = mono
        .ops()
        .iter()
        .filter_map(|op| match op {
            PlanOp::Node(step)
                if step.out_layout() == ChannelLayout::Mono && step.inputs()[0].is_some() =>
            {
                Some(step)
            }
            _ => None,
        })
        .collect();
    assert_eq!(folds.len(), 1);
    let step = folds[0];
    assert_eq!(step.out_layout(), ChannelLayout::Mono);
    assert!(!step.in_place_safe());
    let input = mono
        .region(step.inputs()[0].expect("stereo input"))
        .expect("input region");
    let output = mono.region(step.out()).expect("output region");
    assert!(input.end() <= output.offset() || output.end() <= input.offset());
    let stereo = render(stereo);
    let expected: Vec<_> = stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[l, r]| (((f64::from(*l) + f64::from(*r)) * 0.5) as f32).to_bits())
        .collect();
    let actual = render(mono);
    assert!(
        actual.iter().any(|sample| sample.abs() > 1.0),
        "headroom is not clamped"
    );
    assert_eq!(
        actual.into_iter().map(f32::to_bits).collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn mono_audio_is_identical_under_whole_and_irregular_host_partitions() {
    let plan = common::admit(&graph(true), common::profile(256, ChannelLayout::Mono));
    let expected = render(plan.clone())
        .into_iter()
        .map(f32::to_bits)
        .collect::<Vec<_>>();
    for pattern in [&[256_usize][..], &[64][..], &[1, 7, 13, 43, 127, 3][..]] {
        let (_control, mut renderer) = StreamControl::open(
            plan.clone(),
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .expect("open");
        // Offline rendering removes the stream's initial Q-frame silent carry.
        let retained_frames = expected.len() + 64;
        let mut actual = Vec::new();
        for length in pattern.iter().cycle() {
            let frames = (*length).min(retained_frames - actual.len());
            let mut samples = vec![0.0; frames];
            renderer
                .render(
                    AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("block"),
                    TimedEvents::EMPTY,
                )
                .expect("render");
            actual.extend(samples.into_iter().map(f32::to_bits));
            if actual.len() == retained_frames {
                break;
            }
        }
        assert_eq!(&actual[..64], &[0.0_f32.to_bits(); 64]);
        assert_eq!(&actual[64..], expected, "partition {pattern:?}");
    }
}

#[test]
fn a_stereo_edge_without_the_explicit_fold_still_refuses() {
    let result = synth_engine_v2::compile::compile(
        &graph(false),
        &synth_engine_v2::compile::RenderConfig::new(common::profile(256, ChannelLayout::Mono)),
    )
    .into_plan();
    assert!(matches!(
        result,
        Err(synth_engine_v2::diagnostics::CompileError::LayoutMismatch {
            source_layout: ChannelLayout::Stereo,
            target_layout: ChannelLayout::Mono,
            ..
        })
    ));
}
