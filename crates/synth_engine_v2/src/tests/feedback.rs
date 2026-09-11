//! ADR-0033: recurrence, capture ownership, resources and the real-time boundary.
use crate::compile::{RenderConfig, compile};
use crate::diagnostics::CompileError;
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PlanDeclarations, PortId, SignalDomain,
};
use crate::latency::CompensationPolicy;
use crate::offline::render_offline;
use crate::plan::CompiledPlan;
use crate::profile::HostProfile;
use crate::quantities::{Amplitude, ChannelLayout, SampleRate};
use crate::render::{AudioBlockMut, Renderer, TimedEvents};
use crate::stream::StreamControl;
use crate::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor};

const CLICK: NodeId = NodeId::new(9);
const MEMORY: NodeId = NodeId::new(3);
const SUM: NodeId = NodeId::new(7);
const GAIN: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(5);

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(4096),
        ChannelLayout::Stereo,
    )
    .expect("profile")
}

fn recurrence(policy: CompensationPolicy, reverse: bool) -> GraphIr {
    let mut nodes = vec![
        (
            CLICK,
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
        ),
        (MEMORY, IrNodeKind::FeedbackDelay),
        (SUM, IrNodeKind::Mix),
        (
            GAIN,
            IrNodeKind::Trim {
                level: Amplitude::new(0.5).expect("level"),
            },
        ),
        (OUTPUT, IrNodeKind::Output),
    ];
    if reverse {
        nodes.reverse();
    }
    let mut b = GraphIr::builder().declaring(PlanDeclarations {
        compensation: policy,
        ..PlanDeclarations::default()
    });
    for (id, kind) in nodes {
        b = b.node(id, kind, ExecutionScope::Global);
    }
    for (from, to) in [
        (CLICK, SUM),
        (MEMORY, SUM),
        (SUM, GAIN),
        (GAIN, MEMORY),
        (SUM, OUTPUT),
    ] {
        b = b.connect(
            (from, PortId::FIRST),
            (to, PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    b.build().expect("graph")
}
fn admit(ir: &GraphIr) -> CompiledPlan {
    compile(ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("admitted")
}
fn render(plan: &CompiledPlan, pattern: &[usize]) -> Vec<f32> {
    let (_, mut renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("open");
    let mut out = vec![0.0; 2048];
    let mut at = 0;
    let mut part = 0;
    while at < out.len() {
        let frames = pattern[part % pattern.len()].min((out.len() - at) / 2);
        renderer
            .render(
                AudioBlockMut::new(&mut out[at..at + frames * 2], frames, ChannelLayout::Stereo)
                    .expect("shape"),
                TimedEvents::EMPTY,
            )
            .expect("render");
        at += frames * 2;
        part += 1;
    }
    out
}

#[test]
fn recurrence_has_exact_quantum_delay_under_every_partition_and_declaration_order() {
    let plan = admit(&recurrence(CompensationPolicy::Decline, false));
    let samples = render(&plan, &[1024]);
    for (frame, pair) in samples.as_chunks::<2>().0.iter().enumerate() {
        let expected = if frame >= 64 && frame % 64 == 0 {
            0.5_f32.powi(i32::try_from(frame / 64 - 1).expect("small"))
        } else {
            0.0
        };
        assert_eq!(*pair, [expected; 2], "frame {frame}");
    }
    for pattern in [&[64][..], &[256], &[1, 13, 127, 3, 251]] {
        assert_eq!(samples, render(&plan, pattern));
    }
    assert_eq!(
        samples,
        render(
            &admit(&recurrence(CompensationPolicy::Decline, true)),
            &[71, 5]
        )
    );
    assert_eq!(plan.path_latencies().feedback_boundaries(), &[MEMORY]);
    assert_eq!(plan.added_latency(), FrameCount::QUANTUM);
    assert_eq!(plan.declared_tail(), None);
    let offline =
        render_offline(plan, FrameCount::new(960), PlanPosition::ZERO, &[]).expect("offline");
    assert_eq!(offline, samples[128..]);
}

#[test]
fn deferred_capture_is_not_changed_by_a_downstream_in_place_candidate() {
    // SUM is captured while a later trim reads it. Capture must retain SUM's original
    // signal, otherwise the recurrence decays by the later trim's gain as well.
    let mut b = GraphIr::builder().declaring(PlanDeclarations {
        compensation: CompensationPolicy::Decline,
        ..PlanDeclarations::default()
    });
    for (id, kind) in [
        (
            CLICK,
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
        ),
        (MEMORY, IrNodeKind::FeedbackDelay),
        (SUM, IrNodeKind::Mix),
        (
            GAIN,
            IrNodeKind::Trim {
                level: Amplitude::new(0.25).expect("level"),
            },
        ),
        (OUTPUT, IrNodeKind::Output),
    ] {
        b = b.node(id, kind, ExecutionScope::Global);
    }
    for (from, to) in [
        (CLICK, SUM),
        (MEMORY, SUM),
        (SUM, MEMORY),
        (SUM, GAIN),
        (GAIN, OUTPUT),
    ] {
        b = b.connect(
            (from, PortId::FIRST),
            (to, PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    let samples = render(&admit(&b.build().expect("graph")), &[31, 137]);
    for (frame, pair) in samples.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(
            *pair,
            if frame >= 64 && frame % 64 == 0 {
                [0.25; 2]
            } else {
                [0.0; 2]
            },
            "frame {frame}"
        );
    }
}

#[test]
fn feedback_and_compressor_allocate_nothing_from_the_first_callback() {
    let ir = recurrence(CompensationPolicy::Decline, false);
    let mut b = GraphIr::builder().declaring(ir.declarations().clone());
    for n in ir.nodes() {
        b = b.node(n.id(), n.kind(), n.scope());
    }
    for e in ir.edges() {
        if e.to().0 != OUTPUT {
            b = b.connect(e.from(), e.to(), e.domain());
        }
    }
    let compressor = NodeId::new(10);
    b = b
        .node(
            compressor,
            IrNodeKind::Compressor {
                settings: crate::dynamics::CompressorSettings::default(),
            },
            ExecutionScope::Global,
        )
        .connect(
            (SUM, PortId::FIRST),
            (compressor, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (compressor, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        );
    let (_, mut renderer) = StreamControl::open(
        admit(&b.build().expect("graph")),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("open");
    let mut buffer = [0.0; 512];
    let allocations = crate::render_allocation::count_allocs(|| {
        for _ in 0..20 {
            renderer
                .render(
                    AudioBlockMut::new(&mut buffer, 256, ChannelLayout::Stereo).expect("shape"),
                    TimedEvents::EMPTY,
                )
                .expect("render");
        }
    });
    assert_eq!(allocations, 0);
}

#[test]
fn automatic_compensation_and_unbroken_cycles_are_refused_by_identity() {
    assert!(
        matches!(compile(&recurrence(CompensationPolicy::Compensate,false),&RenderConfig::new(profile())).plan(),Err(CompileError::FeedbackBoundary {node,..}) if *node==MEMORY)
    );
    let original = recurrence(CompensationPolicy::Decline, false);
    let mut b = GraphIr::builder();
    for n in original.nodes() {
        b = b.node(
            n.id(),
            if n.id() == MEMORY {
                IrNodeKind::Trim {
                    level: Amplitude::UNITY,
                }
            } else {
                n.kind()
            },
            n.scope(),
        );
    }
    for e in original.edges() {
        b = b.connect(e.from(), e.to(), e.domain());
    }
    assert!(matches!(
        compile(&b.build().expect("graph"), &RenderConfig::new(profile())).plan(),
        Err(CompileError::Cycle { .. })
    ));
}

#[test]
fn feedback_history_is_charged_exactly_as_allocated() {
    let outcome = compile(
        &recurrence(CompensationPolicy::Decline, false),
        &RenderConfig::new(profile()),
    );
    let reported = match outcome
        .report()
        .row(crate::report::ResourceField::MutableStateBytes)
        .expect("row")
        .requested()
    {
        crate::report::ResourceAmount::Bytes(v) => v.get(),
        other => panic!("{other:?}"),
    };
    let (_, renderer) = StreamControl::open(
        outcome.into_plan().expect("plan"),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("open");
    assert_eq!(
        reported,
        renderer.slot_bytes_held() as u64
            + renderer.ramp_table_bytes_held() as u64
            + renderer.history_bytes_held() as u64
            + u64::from(renderer.prepared_record_count().get())
                * crate::node::state_bytes_per_node()
    );
    assert_eq!(
        renderer.history_bytes_held(),
        128 * size_of::<f32>()
            + (renderer.prepared_record_count().get() as usize + 1) * size_of::<usize>()
    );
}

#[test]
fn two_boundaries_capture_the_same_quantum_and_preserve_stereo_order() {
    let second = NodeId::new(11);
    let ir = GraphIr::builder()
        .declaring(PlanDeclarations {
            compensation: CompensationPolicy::Decline,
            ..PlanDeclarations::default()
        })
        .node(
            CLICK,
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
            ExecutionScope::Global,
        )
        .node(MEMORY, IrNodeKind::FeedbackDelay, ExecutionScope::Global)
        .node(second, IrNodeKind::FeedbackDelay, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (CLICK, PortId::FIRST),
            (MEMORY, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (MEMORY, PortId::FIRST),
            (second, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (second, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("graph");
    let p = admit(&ir);
    assert_eq!(p.path_latencies().feedback_boundaries(), &[MEMORY, second]);
    let samples = render(&p, &[13, 65, 257]);
    for (frame, pair) in samples.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(
            *pair,
            if frame == 192 { [1.0; 2] } else { [0.0; 2] },
            "frame {frame}"
        );
    }
}

#[test]
fn feedback_refuses_missing_input_and_voice_lifetimes() {
    for (boundary_scope, source_scope, connected) in [
        (ExecutionScope::Global, ExecutionScope::Global, false),
        (ExecutionScope::Voice, ExecutionScope::Global, true),
        (ExecutionScope::Global, ExecutionScope::Voice, true),
    ] {
        let mut b = GraphIr::builder()
            .declaring(PlanDeclarations {
                compensation: CompensationPolicy::Decline,
                ..PlanDeclarations::default()
            })
            .node(CLICK, IrNodeKind::Silence, source_scope)
            .node(MEMORY, IrNodeKind::FeedbackDelay, boundary_scope)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (MEMORY, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            );
        if connected {
            b = b.connect(
                (CLICK, PortId::FIRST),
                (MEMORY, PortId::FIRST),
                SignalDomain::Audio,
            );
        }
        let outcome = compile(&b.build().expect("graph"), &RenderConfig::new(profile()));
        assert!(
            matches!(outcome.plan(),Err(CompileError::FeedbackBoundary {node,..}) if *node==MEMORY),
            "{:?}",
            outcome.plan()
        );
    }
}

#[test]
fn feedback_admission_holds_exact_memory_limits() {
    use crate::profile::{MemoryLimits, RenderLimits};
    use crate::quantities::PreparedBytes;
    use crate::report::{ResourceAmount, ResourceField};
    let ir = recurrence(CompensationPolicy::Decline, false);
    let baseline = compile(&ir, &RenderConfig::new(profile()));
    let bytes = |field| match baseline.report().row(field).expect("row").requested() {
        ResourceAmount::Bytes(n) => n.get(),
        other => panic!("{other:?}"),
    };
    let immutable = bytes(ResourceField::PreparedImmutableBytes);
    let mutable = bytes(ResourceField::MutableStateBytes);
    let scratch = bytes(ResourceField::BufferScratchBytes);
    for (cap, accept) in [(immutable, true), (immutable - 1, false)] {
        let h = profile();
        let limits = h.limits();
        let memory = MemoryLimits::new(
            PreparedBytes::limit(cap).expect("cap"),
            PreparedBytes::limit(mutable).expect("cap"),
            PreparedBytes::limit(scratch).expect("cap"),
        )
        .expect("memory");
        let limits = RenderLimits::new(
            limits.stream(),
            limits.graph(),
            limits.voices(),
            limits.events(),
            limits.observation(),
            limits.mixing(),
            memory,
            limits.script(),
            limits.recording(),
            limits.cost(),
        )
        .expect("limits");
        let h = HostProfile::new(h.capabilities(), limits).expect("profile");
        assert_eq!(compile(&ir, &RenderConfig::new(h)).plan().is_ok(), accept);
    }
}
