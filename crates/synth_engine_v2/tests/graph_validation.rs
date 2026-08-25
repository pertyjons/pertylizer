//! P02-T002: one refusal case per validation rule, each asserting the fields its own
//! failure class requires.
//!
//! The Phase 2 exit gate asks for "a useful path-local diagnostic for an invalid cable
//! and for a missing output path". Useful is the load-bearing word: a cable error has
//! to name the edge, both endpoints, and both mismatched values, while a fan-in
//! violation names the competing edges and a cycle names the edge that closes it.
//! Those are different fields, so a single shape imposed on all of them would leave
//! some cases naming things a reader cannot act on.

mod common;

use common::{OUTPUT, SOURCE, profile};
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::diagnostics::{CompileError, CompileWarning};
use synth_engine_v2::ir::{
    EdgeId, ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain,
};
use synth_engine_v2::offline::render_offline;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, GainFactor, NormalizedLevel, Resonance, Seconds,
};
use synth_engine_v2::time::{FrameCount, PlanPosition};
use synth_engine_v2::validate::PortDirection;

const GAIN: NodeId = NodeId::new(3);
const SECOND_GAIN: NodeId = NodeId::new(4);

fn level(value: f32) -> Amplitude {
    Amplitude::new(value).expect("a finite test amplitude")
}

fn factor(value: f32) -> GainFactor {
    GainFactor::new(value).expect("a finite test gain factor")
}

fn constant(value: f32) -> IrNodeKind {
    IrNodeKind::Constant {
        level: level(value),
    }
}

/// Compile against a mono harness profile, expecting a refusal.
fn refuse(ir: &GraphIr) -> CompileError {
    compile(ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)))
        .into_plan()
        .expect_err("the plan must be refused")
}

#[test]
fn a_port_the_node_does_not_declare_is_refused_and_named() {
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::new(7)),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    match refuse(&ir) {
        CompileError::UnknownPort {
            edge,
            node,
            port,
            needed,
        } => {
            assert_eq!(edge, EdgeId::FIRST);
            assert_eq!(node, OUTPUT);
            assert_eq!(port, PortId::new(7));
            assert_eq!(needed, PortDirection::Input);
        }
        other => panic!("expected an unknown-port refusal, got {other:?}"),
    }
}

#[test]
fn a_cable_into_an_output_port_is_a_direction_mistake_and_says_so() {
    // The port exists; the cable is backwards. Saying "node 2 has no port 0" here would
    // send a reader looking for a missing port that is right there.
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(SECOND_GAIN, constant(0.25), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (SECOND_GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    match refuse(&ir) {
        CompileError::PortDirection {
            edge,
            node,
            port,
            needed,
            other_node,
            other_port,
        } => {
            assert_eq!(edge, EdgeId::FIRST);
            assert_eq!(node, SECOND_GAIN);
            assert_eq!(port, PortId::FIRST);
            assert_eq!(needed, PortDirection::Input);
            assert_eq!(other_node, SOURCE, "a diagnostic names the whole cable");
            assert_eq!(other_port, PortId::FIRST);
        }
        other => panic!("expected a direction refusal, got {other:?}"),
    }
}

#[test]
fn an_edge_that_declares_another_domain_is_refused_naming_all_three() {
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Control,
        )
        .build()
        .expect("readable plan");

    match refuse(&ir) {
        CompileError::DomainMismatch {
            edge,
            source_node,
            source_domain,
            target_node,
            target_domain,
            declared,
            ..
        } => {
            assert_eq!(edge, EdgeId::FIRST);
            assert_eq!(source_node, SOURCE);
            assert_eq!(target_node, OUTPUT);
            assert_eq!(source_domain, SignalDomain::Audio);
            assert_eq!(target_domain, SignalDomain::Audio);
            assert_eq!(
                declared,
                SignalDomain::Control,
                "the refusal must say what the edge claimed, not only what the ports are"
            );
        }
        other => panic!("expected a domain refusal, got {other:?}"),
    }
}

#[test]
fn several_edges_into_one_input_are_refused_and_the_competing_ones_are_named() {
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(SECOND_GAIN, constant(0.25), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_GAIN, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    match refuse(&ir) {
        CompileError::UnsupportedFanIn {
            node,
            port,
            edges,
            first,
            second,
        } => {
            assert_eq!(node, OUTPUT);
            assert_eq!(port, PortId::FIRST);
            assert_eq!(edges, 2);
            assert_eq!(first, EdgeId::FIRST);
            assert_eq!(
                second,
                EdgeId::new(1),
                "a count alone does not say which cables to look at"
            );
        }
        other => panic!("expected a fan-in refusal, got {other:?}"),
    }
}

#[test]
fn one_over_subscribed_input_produces_one_diagnostic() {
    // Three edges into one port is one mistake, not three.
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(SECOND_GAIN, constant(0.25), ExecutionScope::Global)
        .node(GAIN, constant(0.125), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_GAIN, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (GAIN, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    match refuse(&ir) {
        CompileError::UnsupportedFanIn { edges, .. } => assert_eq!(edges, 3),
        other => panic!("expected a fan-in refusal, got {other:?}"),
    }
}

#[test]
fn a_cycle_is_refused_and_the_closing_edge_is_named() {
    // Two gains feeding each other. ADR-0033 owns the delay-boundary rule that would
    // let a cycle be scheduled; it is `Proposed`, so every cycle is refused and the
    // diagnostic promises nothing about breaking it.
    let ir = GraphIr::builder()
        .node(
            GAIN,
            IrNodeKind::Gain {
                factor: factor(0.5),
            },
            ExecutionScope::Global,
        )
        .node(
            SECOND_GAIN,
            IrNodeKind::Gain {
                factor: factor(0.5),
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (GAIN, PortId::FIRST),
            (SECOND_GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_GAIN, PortId::FIRST),
            (GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    match refuse(&ir) {
        CompileError::Cycle { edge, node, nodes } => {
            assert_eq!(
                edge,
                EdgeId::new(1),
                "the closing edge is the one to remove"
            );
            assert_eq!(node, GAIN);
            assert!(
                nodes.get() >= 2,
                "a two-node cycle walks at least two nodes"
            );
        }
        other => panic!("expected a cycle refusal, got {other:?}"),
    }
}

#[test]
fn a_node_feeding_itself_is_a_cycle_too() {
    let ir = GraphIr::builder()
        .node(
            GAIN,
            IrNodeKind::Gain {
                factor: factor(0.5),
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (GAIN, PortId::FIRST),
            (GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    match refuse(&ir) {
        CompileError::Cycle { node, .. } => assert_eq!(node, GAIN),
        other => panic!("expected a cycle refusal, got {other:?}"),
    }
}

#[test]
fn sources_with_no_output_node_are_refused_and_counted() {
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(SECOND_GAIN, constant(0.25), ExecutionScope::Global)
        .build()
        .expect("readable plan");

    match refuse(&ir) {
        CompileError::MissingOutput { sources } => assert_eq!(sources.get(), 2),
        other => panic!("expected a missing-output refusal, got {other:?}"),
    }
}

#[test]
fn an_output_nothing_reaches_warns_rather_than_refusing() {
    // Refusing would mean the engine rejects a plan the moment a cable is unplugged,
    // which is a legitimate intermediate state. Saying nothing would leave a silent
    // render unexplained. So: a warning, and the plan still compiles.
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .build()
        .expect("readable plan");

    let outcome = compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)));
    assert!(
        outcome.plan().is_ok(),
        "an unreached output is a warning, not a refusal"
    );
    let warning = outcome
        .warnings()
        .iter()
        .copied()
        .find(|warning| matches!(warning, CompileWarning::OutputNotReached { .. }))
        .expect("an unreached output must be reported");
    match warning {
        CompileWarning::OutputNotReached { output } => assert_eq!(output, OUTPUT),
        other => panic!("expected the unreached-output warning, got {other:?}"),
    }
    assert!(warning.to_string().contains("renders silence"));
}

#[test]
fn the_empty_plan_stays_legal_and_says_nothing() {
    // The Phase 1 exit gate requires an empty plan to render, so "sources without an
    // output" must not catch a plan that has neither.
    let outcome = compile(
        &GraphIr::empty(),
        &RenderConfig::new(profile(256, ChannelLayout::Mono)),
    );
    assert!(outcome.plan().is_ok());
    assert!(
        outcome.warnings().is_empty(),
        "an empty plan is not an unfinished one"
    );
}

#[test]
fn a_mono_source_into_a_stereo_output_is_converted_rather_than_refused() {
    // ADR-0002 clause 6: mono into stereo is the one implicit conversion this phase
    // inserts, and both channels must carry it.
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.25), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    let plan = compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Stereo)))
        .into_plan()
        .expect("a mono source into a stereo stream is legal");
    let rendered = render_offline(plan, FrameCount::new(64), PlanPosition::ZERO, &[])
        .expect("the plan renders");

    assert_eq!(rendered.len(), 128, "64 stereo frames are 128 samples");
    for frame in rendered.as_chunks::<2>().0 {
        assert!((frame[0] - 0.25).abs() < f32::EPSILON);
        assert!(
            (frame[1] - frame[0]).abs() < f32::EPSILON,
            "the duplication must reach both channels"
        );
    }
}

/// Every node kind there is, and its pattern, from **one** definition.
///
/// A premise stated over "the catalog" is worth nothing if the catalog is a hand-written
/// list: an earlier form of this held six of the nine kinds and would have kept passing
/// while `Filter`, `Envelope` or `Amplifier` declared whatever they liked. Pairing the
/// value with its pattern here is what closes that. Adding a variant makes `is_covered`'s
/// match non-exhaustive, and the only way to make it compile again is another
/// `value => pattern` pair — which *is* the catalog entry, so the routine fix cannot
/// restore compilation while leaving the new kind untested.
macro_rules! catalog {
    ($($value:expr => $pattern:pat),+ $(,)?) => {
        /// Every `IrNodeKind`, with a representative value for each.
        fn catalog() -> Vec<IrNodeKind> {
            vec![$($value),+]
        }

        /// Exhaustive by construction: one arm per pair above.
        #[expect(dead_code, reason = "its exhaustiveness is the check, not its callers")]
        fn is_covered(kind: IrNodeKind) -> bool {
            match kind {
                $($pattern => true),+
            }
        }
    };
}

catalog! {
    IrNodeKind::Silence => IrNodeKind::Silence,
    constant(0.5) => IrNodeKind::Constant { .. },
    IrNodeKind::Sine {
        frequency: synth_engine_v2::quantities::Frequency::new(440.0).expect("finite"),
        amplitude: level(0.5),
    } => IrNodeKind::Sine { .. },
    IrNodeKind::Impulse { position: PlanPosition::ZERO } => IrNodeKind::Impulse { .. },
    IrNodeKind::Gain { factor: factor(0.5) } => IrNodeKind::Gain { .. },
    IrNodeKind::Envelope {
        attack: Seconds::new(0.01).expect("finite"),
        decay: Seconds::new(0.1).expect("finite"),
        sustain: NormalizedLevel::new(0.7).expect("in range"),
        release: Seconds::new(0.2).expect("finite"),
    } => IrNodeKind::Envelope { .. },
    IrNodeKind::Filter {
        cutoff: CutoffFrequency::new(1_000.0).expect("positive"),
        resonance: Resonance::BUTTERWORTH,
    } => IrNodeKind::Filter { .. },
    IrNodeKind::Amplifier => IrNodeKind::Amplifier,
    IrNodeKind::Output => IrNodeKind::Output,
}

#[test]
fn every_kernel_admits_exactly_one_channel_on_every_port() {
    // ADR-0041 clause 12's exemption, **checked** rather than assumed: a kernel whose
    // ports admit only one channel is tested at one, with a test asserting that its port
    // table admits only one. Today that is every authored kind — the compiler's own
    // widening is the one operation that writes more — so the obligation to test at two
    // channels does not yet reach any of them, and this is what says so.
    for kind in catalog() {
        // The output node has no kernel: it declares one input port carrying the
        // **stream's** layout, and what reads that region is the boundary copy rather
        // than a kernel that would have to step frames. Clause 12 is an obligation on
        // kernels, so this is the one kind it does not reach.
        if matches!(kind, IrNodeKind::Output) {
            continue;
        }
        for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
            for port in synth_engine_v2::node::ports(kind, layout) {
                assert_eq!(
                    port.layout(),
                    ChannelLayout::Mono,
                    "{kind:?} declares a {:?} port in a {layout:?} stream; it now admits a \
                     second channel and owes a test at every count its ports admit",
                    port.layout()
                );
            }
        }
    }

    // And the output node's own premise, stated rather than skipped: its port follows
    // the stream, which is the signal the widening produces.
    for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
        let ports = synth_engine_v2::node::ports(IrNodeKind::Output, layout);
        assert_eq!(ports.len(), 1, "the output declares one input port");
        assert_eq!(
            ports.first().map(|port| port.layout()),
            Some(layout),
            "and it carries the stream's layout"
        );
    }
}

#[test]
fn the_layout_refusal_has_no_constructible_case_in_this_phase() {
    // Every node kind produces mono, so the *refusing* direction of the layout rule —
    // anything into a narrower port — cannot be built. This test asserts that premise
    // rather than leaving the rule looking untested: the day a node declares a stereo
    // output, this fails, and the refusal case has to be written with it.
    for kind in catalog() {
        for port in synth_engine_v2::node::ports(kind, ChannelLayout::Stereo) {
            if port.direction() == PortDirection::Output {
                assert_eq!(
                    port.layout(),
                    ChannelLayout::Mono,
                    "{kind:?} declares a non-mono output; the layout refusal is now reachable \
                     and needs its own case"
                );
            }
        }
    }
}

#[test]
fn a_gain_scales_the_signal_patched_into_it() {
    let ir = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(
            GAIN,
            IrNodeKind::Gain {
                factor: factor(0.25),
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (GAIN, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    let plan = compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)))
        .into_plan()
        .expect("a source through a gain into an output is admissible");
    let rendered = render_offline(plan, FrameCount::new(64), PlanPosition::ZERO, &[])
        .expect("the plan renders");

    assert!(
        rendered
            .iter()
            .all(|sample| (*sample - 0.125).abs() < f32::EPSILON),
        "0.5 through a gain of 0.25 is 0.125, which also proves the gain ran *after* its source"
    );
}

#[test]
fn the_schedule_is_a_function_of_identity_not_declaration_order() {
    // ADR-0005 clause 3 needs the assignment to be a pure function of the plan, and the
    // schedule is what it is computed from. Two declaration orders of one graph must
    // therefore render the same bytes.
    let forward = GraphIr::builder()
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .node(
            GAIN,
            IrNodeKind::Gain {
                factor: factor(0.25),
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (GAIN, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");
    let reversed = GraphIr::builder()
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .node(
            GAIN,
            IrNodeKind::Gain {
                factor: factor(0.25),
            },
            ExecutionScope::Global,
        )
        .node(SOURCE, constant(0.5), ExecutionScope::Global)
        .connect(
            (GAIN, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    let render = |ir: &GraphIr| {
        let plan = compile(ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)))
            .into_plan()
            .expect("admissible");
        render_offline(plan, FrameCount::new(64), PlanPosition::ZERO, &[]).expect("renders")
    };

    assert_eq!(
        render(&forward).as_slice(),
        render(&reversed).as_slice(),
        "reordering the node list must not change a single sample"
    );
}

#[test]
fn a_long_chain_compiles_without_scanning_the_graph_per_node() {
    // Four thousand gains in a line — well inside the default `max_nodes` of 16 384.
    // The naive form of these passes scans every edge for every node, which is sixteen
    // million comparisons per pass for a graph the profile explicitly admits. Indexed,
    // it is linear. This test asserts the result rather than the time, because a timing
    // assertion on this machine measures the background load; what it does is make a
    // regression to the quadratic form take minutes instead of milliseconds.
    const LINKS: u32 = 4_096;

    let mut builder = GraphIr::builder().node(SOURCE, constant(0.5), ExecutionScope::Global);
    let mut previous = SOURCE;
    for step in 0..LINKS {
        let id = NodeId::new(100 + step);
        builder = builder
            .node(
                id,
                IrNodeKind::Gain {
                    factor: factor(1.0),
                },
                ExecutionScope::Global,
            )
            .connect(
                (previous, PortId::FIRST),
                (id, PortId::FIRST),
                SignalDomain::Audio,
            );
        previous = id;
    }
    let ir = builder
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (previous, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    let plan = compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)))
        .into_plan()
        .expect("a long chain is admissible");
    let rendered = render_offline(plan, FrameCount::new(64), PlanPosition::ZERO, &[])
        .expect("the plan renders");

    assert!(
        rendered
            .iter()
            .all(|sample| (*sample - 0.5).abs() < f32::EPSILON),
        "unity gains pass the source through, which also proves the schedule ordered all \
         {LINKS} of them"
    );
}

#[test]
fn fan_out_order_is_identity_ordered_too() {
    // One source feeding two gains, with the two cables declared in both orders. If the
    // schedule followed edge declaration order, these would assign different buffer
    // slots and emit a different operation list for the same graph.
    let plan_for = |swapped: bool| {
        let mut builder = GraphIr::builder()
            .node(SOURCE, constant(0.5), ExecutionScope::Global)
            .node(
                GAIN,
                IrNodeKind::Gain {
                    factor: factor(0.25),
                },
                ExecutionScope::Global,
            )
            .node(
                SECOND_GAIN,
                IrNodeKind::Gain {
                    factor: factor(0.5),
                },
                ExecutionScope::Global,
            )
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global);
        let first = (SOURCE, PortId::FIRST);
        let (a, b) = if swapped {
            (SECOND_GAIN, GAIN)
        } else {
            (GAIN, SECOND_GAIN)
        };
        builder = builder
            .connect(first, (a, PortId::FIRST), SignalDomain::Audio)
            .connect(first, (b, PortId::FIRST), SignalDomain::Audio)
            .connect(
                (GAIN, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            );
        let ir = builder.build().expect("readable plan");
        compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)))
            .into_plan()
            .expect("admissible")
    };

    assert_eq!(
        plan_for(false).ops(),
        plan_for(true).ops(),
        "swapping two equivalent cables must not change the compiled operation list"
    );
}
