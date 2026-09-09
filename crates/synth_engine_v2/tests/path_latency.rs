//! `P08-S005`, `SOUND-INV-035`: observable arrival times, compensation and accounting.
mod common;

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::diagnostics::CompileError;
use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::latency::CompensationPolicy;
use synth_engine_v2::offline::render_offline;
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::quantities::ChannelLayout;
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::report::{LatencyContributor, ResourceAmount, ResourceField};
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor};

const SOURCE: NodeId = NodeId::new(90);
const FIRST: NodeId = NodeId::new(40);
const SECOND: NodeId = NodeId::new(30);
const MIX: NodeId = NodeId::new(20);
const OUTPUT: NodeId = NodeId::new(10);

fn latency(frames: u64) -> IrNodeKind {
    IrNodeKind::Latency {
        frames: FrameCount::new(frames),
    }
}

/// Identity order deliberately opposes dependency order.
fn fork(first: u64, second: u64) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
            ExecutionScope::Global,
        )
        .node(FIRST, latency(first), ExecutionScope::Global)
        .node(SECOND, latency(second), ExecutionScope::Global)
        .node(MIX, IrNodeKind::Mix, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global);
    for (from, to) in [
        (SOURCE, FIRST),
        (SOURCE, SECOND),
        (FIRST, MIX),
        (SECOND, MIX),
        (MIX, OUTPUT),
    ] {
        builder = builder.connect(
            (from, PortId::FIRST),
            (to, PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    builder.build().expect("readable fork")
}

fn outcome(ir: &GraphIr, policy: CompensationPolicy) -> synth_engine_v2::compile::CompileOutcome {
    let mut builder = GraphIr::builder().declaring(synth_engine_v2::ir::PlanDeclarations {
        compensation: policy,
        ..ir.declarations().clone()
    });
    for node in ir.nodes() {
        builder = builder.node(node.id(), node.kind(), node.scope());
    }
    for edge in ir.edges() {
        builder = builder.connect(edge.from(), edge.to(), edge.domain());
    }
    compile(
        &builder.build().expect("same graph"),
        &RenderConfig::new(common::profile(2048, ChannelLayout::Stereo)),
    )
}

fn render(plan: CompiledPlan) -> Vec<f32> {
    render_offline(plan, FrameCount::new(2048), PlanPosition::ZERO, &[]).expect("renders")
}

fn clicks(samples: &[f32]) -> Vec<(usize, f32)> {
    let mut result = Vec::new();
    for (frame, [left, right]) in samples.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(left.to_bits(), right.to_bits(), "stereo frame {frame}");
        if *left != 0.0 {
            result.push((frame, *left));
        }
    }
    result
}

#[test]
fn unequal_paths_arrive_aligned_and_report_the_scheduled_delay() {
    for (first, second) in [(0, 7), (7, 0), (17, 137), (257, 3), (0, 0)] {
        let ir = fork(first, second);
        let result = outcome(&ir, CompensationPolicy::Compensate);
        let plan = result.plan().expect("admits");
        let longest = first.max(second);
        assert_eq!(plan.added_latency(), FrameCount::new(64 + longest));
        assert_eq!(
            result.report().latency().total(),
            Some(plan.added_latency())
        );
        assert_eq!(
            result.report().latency().paths(),
            Some(plan.path_latencies())
        );
        assert_eq!(
            plan.timing_of(FIRST).expect("timing").latency,
            FrameCount::new(first)
        );
        let mix = plan
            .path_latencies()
            .paths()
            .iter()
            .find(|path| path.node() == MIX)
            .expect("mix");
        assert_eq!(mix.earliest(), FrameCount::new(longest));
        assert_eq!(mix.latest(), FrameCount::new(longest));
        let inserted: u64 = plan
            .path_latencies()
            .edges()
            .iter()
            .map(|edge| edge.compensation().as_u64())
            .sum();
        assert_eq!(inserted, first.abs_diff(second));
        assert_eq!(
            clicks(&render(plan.clone())),
            [(usize::try_from(longest).expect("small"), 2.0)]
        );
    }
}

#[test]
fn declining_compensation_preserves_and_reports_the_skew() {
    let result = outcome(&fork(7, 137), CompensationPolicy::Decline);
    let plan = result.plan().expect("admits");
    assert_eq!(plan.path_latencies().policy(), CompensationPolicy::Decline);
    assert!(
        plan.path_latencies()
            .edges()
            .iter()
            .all(|edge| edge.compensation() == FrameCount::ZERO)
    );
    assert!(
        plan.path_latencies()
            .edges()
            .iter()
            .any(|edge| edge.skew() == FrameCount::new(130))
    );
    let output = plan
        .path_latencies()
        .paths()
        .iter()
        .find(|path| path.node() == OUTPUT)
        .expect("output");
    assert_eq!(
        (output.earliest(), output.latest()),
        (FrameCount::new(7), FrameCount::new(137))
    );
    assert_eq!(
        result
            .report()
            .latency()
            .frames_of(LatencyContributor::AudioPath),
        Some(FrameCount::new(137))
    );
    assert_eq!(clicks(&render(plan.clone())), [(7, 1.0), (137, 1.0)]);
}

#[test]
fn serial_latencies_sum_and_an_unconnected_latency_does_not_reach_output() {
    let mut builder = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
            ExecutionScope::Global,
        )
        .node(FIRST, latency(7), ExecutionScope::Global)
        .node(SECOND, latency(137), ExecutionScope::Global)
        .node(NodeId::new(100), latency(1000), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global);
    for (from, to) in [(SOURCE, FIRST), (FIRST, SECOND), (SECOND, OUTPUT)] {
        builder = builder.connect(
            (from, PortId::FIRST),
            (to, PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    let ir = builder.build().expect("readable");
    let plan = outcome(&ir, CompensationPolicy::Compensate)
        .into_plan()
        .expect("admits");
    assert_eq!(plan.added_latency(), FrameCount::new(208));
    assert_eq!(clicks(&render(plan)), [(144, 1.0)]);
}

fn partitioned(plan: &CompiledPlan, pattern: &[usize]) -> Vec<u32> {
    let (_control, mut renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("opens");
    let mut samples = Vec::new();
    let mut index = 0;
    while samples.len() < 4096 {
        let frames = pattern[index % pattern.len()].min((4096 - samples.len()) / 2);
        let mut block = vec![0.0; frames * 2];
        renderer
            .render(
                AudioBlockMut::new(&mut block, frames, ChannelLayout::Stereo).expect("shape"),
                TimedEvents::EMPTY,
            )
            .expect("renders");
        samples.extend(block.iter().map(|sample| sample.to_bits()));
        index += 1;
    }
    samples
}

#[test]
fn compensation_crosses_quanta_with_identical_bits_under_every_partition() {
    let plan = outcome(&fork(3, 257), CompensationPolicy::Compensate)
        .into_plan()
        .expect("admits");
    let whole = partitioned(&plan, &[2048]);
    assert_eq!(whole[(64 + 257) * 2], 2.0_f32.to_bits());
    for partition in [&[256][..], &[64], &[1, 17, 63, 129, 5]] {
        assert_eq!(partitioned(&plan, partition), whole);
    }
}

fn bytes(result: &synth_engine_v2::compile::CompileOutcome) -> u64 {
    match result
        .report()
        .row(ResourceField::MutableStateBytes)
        .expect("row")
        .requested()
    {
        ResourceAmount::Bytes(bytes) => bytes.get(),
        other => panic!("unexpected amount: {other:?}"),
    }
}

#[test]
fn compensation_charges_its_stereo_history_and_inserted_record() {
    let ir = fork(7, 137);
    let aligned = outcome(&ir, CompensationPolicy::Compensate);
    let uncompensated = outcome(&ir, CompensationPolicy::Decline);
    assert_eq!(
        bytes(&aligned) - bytes(&uncompensated),
        130 * 2 * 4
            + synth_engine_v2::node::state_bytes_per_node()
            + synth_engine_v2::node::ramp_table_bytes_per_record()
            + synth_engine_v2::node::history_table_bytes_per_record()
    );
}

#[test]
fn enormous_history_is_refused_before_render_allocation() {
    let result = outcome(&fork(0, 1_000_000_000), CompensationPolicy::Compensate);
    assert!(
        matches!(
            result.plan(),
            Err(CompileError::LimitExceeded {
                field: ResourceField::MutableStateBytes,
                ..
            })
        ),
        "{:?}",
        result.plan()
    );
}

#[test]
fn path_latency_overflow_is_refused_by_node() {
    let result = outcome(&fork(u64::MAX, 1), CompensationPolicy::Compensate);
    assert!(
        matches!(
            result.plan(),
            Err(CompileError::PathLatencyOverflow { node: OUTPUT })
        ),
        "{:?}",
        result.plan()
    );
}

#[test]
fn zero_latency_plans_have_identical_audio_under_both_policies() {
    let ir = fork(0, 0);
    let aligned = outcome(&ir, CompensationPolicy::Compensate)
        .into_plan()
        .expect("admits");
    let uncompensated = outcome(&ir, CompensationPolicy::Decline)
        .into_plan()
        .expect("admits");
    assert_eq!(
        partitioned(&aligned, &[2048]),
        partitioned(&uncompensated, &[2048])
    );
    assert_eq!(aligned.ops().len(), uncompensated.ops().len());
}

#[test]
fn a_declined_upstream_merge_preserves_its_spread_through_a_second_merge() {
    const NEXT: NodeId = NodeId::new(5);
    let original = fork(0, 137);
    let mut builder = GraphIr::builder().node(NEXT, IrNodeKind::Mix, ExecutionScope::Global);
    for node in original.nodes() {
        builder = builder.node(node.id(), node.kind(), node.scope());
    }
    for edge in original.edges() {
        if edge.to().0 != OUTPUT {
            builder = builder.connect(edge.from(), edge.to(), edge.domain());
        }
    }
    for (from, to) in [(MIX, NEXT), (SECOND, NEXT), (NEXT, OUTPUT)] {
        builder = builder.connect(
            (from, PortId::FIRST),
            (to, PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    let ir = builder.build().expect("nested merges");
    let declined = outcome(&ir, CompensationPolicy::Decline)
        .into_plan()
        .expect("admits");
    let next = declined
        .path_latencies()
        .paths()
        .iter()
        .find(|path| path.node() == NEXT)
        .expect("second merge");
    assert_eq!(
        (next.earliest(), next.latest()),
        (FrameCount::ZERO, FrameCount::new(137))
    );
    assert_eq!(clicks(&render(declined)), [(0, 1.0), (137, 2.0)]);
    let aligned = outcome(&ir, CompensationPolicy::Compensate)
        .into_plan()
        .expect("admits");
    assert_eq!(clicks(&render(aligned)), [(137, 3.0)]);
}

#[test]
fn the_tail_summary_and_offline_trim_do_not_absorb_the_output_path() {
    let result = outcome(&fork(7, 137), CompensationPolicy::Compensate);
    let plan = result.plan().expect("admits");
    assert_eq!(plan.declared_tail(), Some(FrameCount::new(137)));
    assert_eq!(plan.offline_trim(), FrameCount::QUANTUM);
    assert_eq!(plan.added_latency(), FrameCount::new(201));
}

#[test]
fn invalid_structure_has_no_fabricated_path_latency() {
    let ir = GraphIr::builder()
        .node(SOURCE, latency(7), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (SOURCE, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable cycle");
    let result = outcome(&ir, CompensationPolicy::Compensate);
    assert!(result.plan().is_err());
    assert!(result.report().latency().paths().is_none());
}

#[test]
fn compensation_is_charged_before_selecting_the_first_memory_refusal() {
    use synth_engine_v2::profile::{HostProfile, MemoryLimits, RenderLimits};
    use synth_engine_v2::quantities::PreparedBytes;
    let ir = fork(7, 137);
    let host = common::profile(2048, ChannelLayout::Stereo);
    let baseline = compile(&ir, &RenderConfig::new(host));
    let requested = |field| match baseline.report().row(field).expect("row").requested() {
        ResourceAmount::Bytes(bytes) => bytes,
        other => panic!("unexpected amount: {other:?}"),
    };
    let prepared = requested(ResourceField::PreparedImmutableBytes).get();
    let mutable = requested(ResourceField::MutableStateBytes).get();
    let scratch = requested(ResourceField::BufferScratchBytes).get();
    for (immutable_cap, mutable_cap, admits) in
        [(prepared, mutable, true), (prepared - 1, 1, false)]
    {
        let defaults = host.limits();
        let memory = MemoryLimits::new(
            PreparedBytes::limit(immutable_cap).expect("positive"),
            PreparedBytes::limit(mutable_cap).expect("positive"),
            PreparedBytes::limit(scratch).expect("positive"),
        )
        .expect("memory limits");
        let limits = RenderLimits::new(
            defaults.stream(),
            defaults.graph(),
            defaults.voices(),
            defaults.events(),
            defaults.observation(),
            defaults.mixing(),
            memory,
            defaults.script(),
            defaults.recording(),
            defaults.cost(),
        )
        .expect("limits");
        let limited = HostProfile::new(host.capabilities(), limits).expect("profile");
        let result = compile(&ir, &RenderConfig::new(limited));
        if admits {
            assert!(
                result.plan().is_ok(),
                "its own exact report must fit: {:?}",
                result.plan()
            );
        } else {
            assert!(
                matches!(
                    result.plan(),
                    Err(CompileError::LimitExceeded {
                        field: ResourceField::PreparedImmutableBytes,
                        ..
                    })
                ),
                "the first resource row must win: {:?}",
                result.plan()
            );
            assert!(result.report().latency().paths().is_some());
        }
    }
}

#[test]
fn unrepresentable_history_is_refused_even_under_the_largest_memory_budget() {
    use synth_engine_v2::profile::{HostProfile, MemoryLimits, RenderLimits};
    use synth_engine_v2::quantities::PreparedBytes;
    let host = common::profile(2048, ChannelLayout::Stereo);
    let defaults = host.limits();
    let maximum = PreparedBytes::limit(u64::MAX).expect("positive");
    let memory = MemoryLimits::new(maximum, maximum, maximum).expect("memory limits");
    let limits = RenderLimits::new(
        defaults.stream(),
        defaults.graph(),
        defaults.voices(),
        defaults.events(),
        defaults.observation(),
        defaults.mixing(),
        memory,
        defaults.script(),
        defaults.recording(),
        defaults.cost(),
    )
    .expect("limits");
    let host = HostProfile::new(host.capabilities(), limits).expect("profile");
    // The frame count and its output latency fit u64/usize; the stereo inserted line's
    // byte length and the combined slab do not. Never open a renderer for this fixture.
    let ir = fork(0, u64::MAX / 8);
    let outcome = compile(&ir, &RenderConfig::new(host));
    assert!(
        matches!(
            outcome.plan(),
            Err(CompileError::HistoryStorageUnrepresentable { .. })
        ),
        "{:?}",
        outcome.plan()
    );
    assert!(outcome.report().latency().paths().is_none());
}
