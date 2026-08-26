//! The node vocabulary the vertical sound path needs, one node at a time.
//!
//! The master plan's Phase 2 scope is a single path — note events, an envelope, an
//! oscillator, a filter, an amplifier, an output. This file checks each node's own
//! behaviour as it arrives; the path itself is P02-T007's, and it is not assembled here.

mod common;

use common::{OUTPUT, SOURCE, profile, rate};
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::diagnostics::{CompileError, PreparationFault};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, NormalizedLevel, ParameterValue, Resonance, Seconds,
};
use synth_engine_v2::render::EventPayload;
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime};

const FILTER: NodeId = NodeId::new(7);

/// A constant into a low-pass into the output.
fn filtered_constant(level: f32, cutoff: f32) -> GraphIr {
    GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(level).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(cutoff).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan")
}

#[test]
fn a_low_pass_settles_at_unity_gain_on_a_constant() {
    // The one property of a low-pass that a wrong coefficient normalisation breaks and a
    // spectrum plot would not: its DC gain is exactly one. A filter whose coefficients
    // were not divided by `a0` still filters, still sounds like a filter, and is quietly
    // several decibels off.
    let plan = common::admit(
        &filtered_constant(0.5, 1_000.0),
        profile(256, ChannelLayout::Mono),
    );
    let rendered = render_offline(
        plan,
        FrameCount::new(4_096),
        PlanPosition::ZERO,
        &[] as &[OfflineEvent],
    )
    .expect("renders");

    let settled = rendered.last().copied().expect("frames were rendered");
    assert!(
        (settled - 0.5).abs() < 1e-3,
        "a low-pass passes a constant at unity, and this settled at {settled}"
    );
    let first = rendered.first().copied().expect("frames were rendered");
    assert!(
        first < settled,
        "the step response rises rather than starting settled: {first} then {settled}"
    );
}

#[test]
fn a_low_pass_attenuates_what_is_above_its_corner() {
    // Two filters over one source, differing only in corner frequency. Comparing them
    // rather than asserting an absolute figure is deliberate: the claim is that the
    // corner frequency *does* something, and an absolute threshold would be a claim about
    // a particular filter design that no accepted record makes.
    let low = common::admit(
        &filtered_constant(0.5, 20.0),
        profile(256, ChannelLayout::Mono),
    );
    let high = common::admit(
        &filtered_constant(0.5, 8_000.0),
        profile(256, ChannelLayout::Mono),
    );

    let after = |plan| {
        render_offline(
            plan,
            FrameCount::new(64),
            PlanPosition::ZERO,
            &[] as &[OfflineEvent],
        )
        .expect("renders")
        .last()
        .copied()
        .expect("frames were rendered")
    };

    assert!(
        after(low) < after(high),
        "the same step reaches further through the wider filter in the same 64 frames"
    );
}

#[test]
fn a_corner_frequency_of_a_few_hertz_still_passes_signal() {
    // The `1 - cos` cancellation, as a test rather than as a comment. At 1 Hz against
    // 48 kHz the angle is small enough that `cos` rounds to exactly one in `f32`, and a
    // filter whose numerator cancelled to zero compiles cleanly, admits, renders, and is
    // silent forever. Nothing about the plan or the report would say so.
    let plan = common::admit(
        &filtered_constant(0.5, 1.0),
        profile(256, ChannelLayout::Mono),
    );
    let rendered = render_offline(
        plan,
        FrameCount::new(8_192),
        PlanPosition::ZERO,
        &[] as &[OfflineEvent],
    )
    .expect("renders");

    let last = rendered.last().copied().expect("frames were rendered");
    assert!(
        last > 0.0,
        "a 1 Hz low-pass fed a constant rises towards it; this one is at {last}"
    );
    assert!(
        last < 0.5,
        "and 8 192 frames is well inside its time constant, so it has not arrived: {last}"
    );
}

#[test]
fn a_corner_frequency_above_nyquist_is_refused_rather_than_clamped() {
    // Clamping would render a filter the caller did not ask for. The diagnostic names the
    // node and both figures, because a reader can act on neither alone.
    let profile = HostProfile::harness(rate(48_000.0), FrameCount::new(256), ChannelLayout::Mono)
        .expect("a valid profile");
    let error = compile(
        &filtered_constant(0.5, 30_000.0),
        &RenderConfig::new(profile),
    )
    .into_plan()
    .expect_err("a corner frequency above Nyquist is refused");

    match error {
        CompileError::NodeNotPreparable {
            node,
            fault: PreparationFault::CutoffAboveNyquist { cutoff, nyquist },
        } => {
            assert_eq!(
                node, FILTER,
                "the diagnostic names the node that cannot be prepared"
            );
            assert!((cutoff.as_f32() - 30_000.0).abs() < f32::EPSILON);
            assert!((nyquist.as_f32() - 24_000.0).abs() < f32::EPSILON);
        }
        other => panic!("expected a preparation refusal, got {other}"),
    }
}

#[test]
fn a_refused_node_still_comes_with_a_report() {
    // `HOST-INV-006` admits no exception, including this one: a plan refused for a node
    // it cannot prepare is still a plan whose resources someone may want to read.
    let profile = HostProfile::harness(rate(48_000.0), FrameCount::new(256), ChannelLayout::Mono)
        .expect("a valid profile");
    let outcome = compile(
        &filtered_constant(0.5, 30_000.0),
        &RenderConfig::new(profile),
    );
    assert!(outcome.plan().is_err());
    assert_eq!(
        outcome.report().rows().len(),
        synth_engine_v2::report::ResourceField::COUNT,
        "every field is reported whether or not a plan came out"
    );

    // And in `ResourceField::ALL`'s order, which `exceeded()` promises its callers when
    // it says "in field order" while returning rows in the order they were pushed. The
    // two agreed only by construction until ADR-0046's fields were appended to `ALL` but
    // emitted mid-list, which silently misaligned every later row for a consumer walking
    // the canonical order. Counting the rows cannot see that; this can.
    let order: Vec<_> = outcome
        .report()
        .rows()
        .iter()
        .map(synth_engine_v2::report::ResourceRow::field)
        .collect();
    assert_eq!(
        order,
        synth_engine_v2::report::ResourceField::ALL.to_vec(),
        "report rows must follow the canonical field order"
    );
}

#[test]
fn a_pair_of_legal_values_whose_arithmetic_is_not_is_refused() {
    // A denormal quality factor is finite and above zero, so no range check on the input
    // catches it. The formula divides by twice it, `alpha` overflows to infinity, and the
    // feedback coefficient becomes `NaN` — which would poison the filter's history from
    // its first sample, with nothing downstream able to recover.
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(1_000.0).expect("positive"),
                resonance: Resonance::new(f32::from_bits(1)).expect("finite and above zero"),
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");

    let error = compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)))
        .into_plan()
        .expect_err("coefficients that are not finite are refused");
    assert!(
        matches!(
            error,
            CompileError::NodeNotPreparable {
                node: FILTER,
                fault: PreparationFault::CoefficientsUnusable { .. },
            }
        ),
        "expected a coefficient refusal, got {error}"
    );
}

#[test]
fn a_quality_factor_whose_rounded_filter_would_climb_is_refused() {
    // Representable is not stable. This pair rounds `a1` up far enough that the
    // recurrence's determinant passes one, after which an impulse response grows for as
    // long as the stream runs — slowly enough that no short render would show it, which
    // is why the check is on the coefficients rather than on a rendered tail.
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(6_840.908).expect("positive"),
                resonance: Resonance::new(54_034_396.0).expect("finite and above zero"),
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");

    let error = compile(
        &ir,
        &RenderConfig::new(
            HostProfile::harness(rate(44_100.0), FrameCount::new(256), ChannelLayout::Mono)
                .expect("a valid profile"),
        ),
    )
    .into_plan()
    .expect_err("an unstable rounded recurrence is refused");
    assert!(
        matches!(
            error,
            CompileError::NodeNotPreparable {
                node: FILTER,
                fault: PreparationFault::CoefficientsUnstable { .. },
            }
        ),
        "expected a stability refusal, got {error}"
    );
}

#[test]
fn ordinary_filters_are_not_refused_by_the_stability_check() {
    // The other half, because a check that refuses everything also passes the test above.
    // Every corner frequency and quality factor a caller might plausibly ask for, at the
    // rate the fixtures use.
    for cutoff in [1.0, 20.0, 200.0, 2_000.0, 12_000.0, 20_000.0] {
        for resonance in [0.05, 0.5, 0.707, 1.0, 8.0, 60.0, 200.0] {
            let ir = GraphIr::builder()
                .node(
                    FILTER,
                    IrNodeKind::Filter {
                        cutoff: CutoffFrequency::new(cutoff).expect("positive"),
                        resonance: Resonance::new(resonance).expect("positive"),
                    },
                    ExecutionScope::Voice,
                )
                .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
                .connect(
                    (FILTER, PortId::FIRST),
                    (OUTPUT, PortId::FIRST),
                    SignalDomain::Audio,
                )
                .build()
                .expect("a readable plan");
            assert!(
                compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)))
                    .into_plan()
                    .is_ok(),
                "a {cutoff} Hz low-pass at Q {resonance} is an ordinary filter and was refused"
            );
        }
    }
}

#[test]
fn a_filter_that_has_stopped_ringing_writes_exact_zeros() {
    // The denormal guard, from outside. A filter's history decays into the subnormal
    // range and stays there — the products that would carry it further underflow — so
    // without a flush the node keeps multiplying subnormals on the audio thread forever,
    // and its output is a tiny non-zero value rather than silence.
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(1_000.0).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");

    let rendered = render_offline(
        common::admit(&ir, profile(256, ChannelLayout::Mono)),
        FrameCount::new(48_000),
        PlanPosition::ZERO,
        &[] as &[OfflineEvent],
    )
    .expect("renders");

    assert!(
        rendered.iter().any(|sample| *sample != 0.0),
        "the impulse should have rung the filter at all"
    );
    let tail = rendered.len().saturating_sub(1_000);
    assert!(
        rendered[tail..].iter().all(|sample| *sample == 0.0),
        "a second after one impulse, the filter is writing subnormal dust rather than silence"
    );
}

#[test]
fn the_same_filter_at_two_rates_is_two_prepared_records() {
    // Prepared data is derived from the stream, so the same authored node prepares
    // differently against a different rate. If it did not, the coefficients would be
    // wrong at every rate but one — the defect that hides best, because the filter still
    // sounds like a filter.
    let ir = filtered_constant(0.5, 1_000.0);
    let at_48 = common::admit(&ir, profile(256, ChannelLayout::Mono));
    let at_96 = common::admit(
        &ir,
        HostProfile::harness(rate(96_000.0), FrameCount::new(256), ChannelLayout::Mono)
            .expect("a valid profile"),
    );

    assert_ne!(
        at_48.prepared_nodes(),
        at_96.prepared_nodes(),
        "the coefficients are a function of the rate, and these are two rates"
    );
}

/// An envelope gating a constant through an amplifier, and the gate's compiled slot.
///
/// The first plan in this phase with two signal domains in it: the envelope's control
/// output crosses into the amplifier's control input, and validation checks that against
/// the port table like any other edge.
fn gated_constant() -> (GraphIr, NodeId) {
    const ENVELOPE: NodeId = NodeId::new(11);
    const AMPLIFIER: NodeId = NodeId::new(12);
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::new(0.5).expect("within range"),
                release: Seconds::new(0.0).expect("not negative"),
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");
    (ir, ENVELOPE)
}

#[test]
fn an_amplifier_with_no_gate_is_silent() {
    // An envelope nobody gated is at zero, and an amplifier driven by zero passes
    // nothing. It is the same rendered result as a broken patch, which is why the next
    // test exists: this one alone would pass on a plan that never connected anything.
    let (ir, _) = gated_constant();
    let rendered = render_offline(
        common::admit(&ir, profile(256, ChannelLayout::Mono)),
        FrameCount::new(256),
        PlanPosition::ZERO,
        &[] as &[OfflineEvent],
    )
    .expect("renders");
    assert!(
        rendered.iter().all(|sample| *sample == 0.0),
        "an ungated voice is silent"
    );
}

#[test]
fn a_gated_amplifier_passes_the_envelope_it_is_driven_by() {
    // The path P02-T007 will render a note through, one task early and with the gate as
    // an ordinary control: a constant into an amplifier, driven by an envelope whose
    // segments are instantaneous, so the level is the sustain level from the first
    // quantum the gate is held.
    let (ir, envelope) = gated_constant();
    let plan = common::admit(&ir, profile(256, ChannelLayout::Mono));
    let gate = plan
        .resolve_parameter(envelope, parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate");

    let rendered = render_offline(
        plan,
        FrameCount::new(256),
        PlanPosition::ZERO,
        &[OfflineEvent::new(
            SampleTime::ZERO,
            EventPayload::SetParameter {
                slot: gate,
                value: ParameterValue::new(1.0).expect("finite"),
            },
        )],
    )
    .expect("renders");

    let settled = rendered.last().copied().unwrap_or(0.0);
    assert!(
        (settled - 0.5).abs() < 1e-6,
        "a held gate on an instant envelope holds the sustain level, and this is {settled}"
    );
}

#[test]
fn an_amplifier_refuses_an_audio_signal_on_its_control_port() {
    // The domain rule, with a node that has two domains to get wrong. Patching the
    // constant into the control port is a plan that means something the compiler cannot
    // deliver, and it says so rather than rendering the audio as a gain.
    const AMPLIFIER: NodeId = NodeId::new(12);
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Audio,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");

    let error = compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Mono)))
        .into_plan()
        .expect_err("audio into a control port is refused");
    assert!(
        matches!(error, CompileError::DomainMismatch { .. }),
        "expected a domain refusal, got {error}"
    );
}
