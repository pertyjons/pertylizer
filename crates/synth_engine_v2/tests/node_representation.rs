//! P02-T005: prepared data is separated from mutable state, and the loop dispatches.
//!
//! [ADR-0004](../../../plans/v2/decisions/ADR-0004-native-node-representation.md) clause
//! 5 is the claim under test: a kernel takes prepared data, mutable state, and slots, and
//! **never `&self`**. Two consequences follow, and neither is visible in a render that
//! uses one plan once — which is why they are checked here rather than assumed:
//!
//! - rendering a node cannot change its configuration, so a plan is still the plan it was
//!   after a stream has run;
//! - one prepared node can serve several states, which is what Phase 6's voice pool needs
//!   and what a shared `&mut self` node would quietly prevent.
//!
//! The dispatch half — that the render loop contains no per-kind control flow — is a
//! source property, and `tests/render_loop_purity.rs` owns it.

mod common;

use common::{OUTPUT, SOURCE, admit, profile};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::{CompiledPlan, PlanOp};
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, Frequency, GainFactor, ParameterValue,
};
use synth_engine_v2::render::{
    AudioBlockMut, EventEnvelope, EventPayload, Renderer, TimedEvent, TimedEvents,
};
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor, TimeSource};

fn sine(frequency: f32) -> IrNodeKind {
    IrNodeKind::Sine {
        frequency: Frequency::new(frequency).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    }
}

/// Render one block of `frames` from a freshly prepared stream.
fn render(plan: &CompiledPlan, frames: usize) -> Vec<f32> {
    let (_control, mut renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("an admitted plan prepares");
    let mut samples = vec![0.0; frames];
    let block =
        AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("a mono block");
    renderer
        .render(block, TimedEvents::EMPTY)
        .expect("a block within the maximum renders");
    samples
}

#[test]
fn rendering_a_plan_does_not_change_the_plan() {
    // The `&self`-free signature made concrete: a kernel is handed `&PreparedNode`, so
    // the prepared table it reads is the same table afterwards. A node that rendered
    // through `&mut self` would make this untestable rather than false — there would be
    // no separate prepared data to compare.
    let plan = admit(
        &common::source_plan(sine(440.0)),
        profile(256, ChannelLayout::Mono),
    );
    let before = plan.prepared_nodes().to_vec();

    let (_control, mut renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("an admitted plan prepares");
    let mut samples = vec![0.0; 256];
    let block = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("a mono block");
    renderer.render(block, TimedEvents::EMPTY).expect("renders");

    assert_eq!(
        renderer.plan().prepared_nodes(),
        before.as_slice(),
        "a stream rendered 256 frames and the prepared data it read is unchanged"
    );
    assert_eq!(
        plan.prepared_nodes(),
        before.as_slice(),
        "the caller's own copy of the plan is untouched"
    );
}

#[test]
fn one_plan_serves_two_independent_states() {
    // Two streams over one plan advance their own phase accumulators: the second one is
    // not carried forward by the first.
    //
    // **What this establishes, and what it does not.** Each renderer is given its own
    // `CompiledPlan`, and a plan's prepared table is a `Vec`, so cloning it copies —
    // this test on its own would also pass if the phase lived in that copied table.
    // What rules that out is the *pair*: `rendering_a_plan_does_not_change_the_plan`
    // fails the moment a kernel writes state into its prepared record, and this one
    // fails if two streams share one. Neither is the zero-copy sharing ADR-0004 clause 5
    // points at — one prepared node serving many states without copying is Phase 6's
    // voice pool, and there is no pool here to check it with.
    let plan = admit(
        &common::source_plan(sine(440.0)),
        profile(256, ChannelLayout::Mono),
    );

    let (_first_control, mut first) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("prepares");
    let (_second_control, second) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("prepares");

    // Advance only the first stream.
    let mut samples = vec![0.0; 256];
    let block = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("a mono block");
    first.render(block, TimedEvents::EMPTY).expect("renders");
    let advanced = samples.clone();

    let mut second = second;
    let mut fresh = vec![0.0; 256];
    let block = AudioBlockMut::new(&mut fresh, 256, ChannelLayout::Mono).expect("a mono block");
    second.render(block, TimedEvents::EMPTY).expect("renders");

    assert_eq!(
        advanced, fresh,
        "two streams from one plan each start at phase zero, so their first blocks match"
    );

    // Now they diverge: only the first has history.
    let mut next = vec![0.0; 256];
    let block = AudioBlockMut::new(&mut next, 256, ChannelLayout::Mono).expect("a mono block");
    first.render(block, TimedEvents::EMPTY).expect("renders");
    assert_ne!(
        next, fresh,
        "the first stream's second block continues its own phase rather than restarting"
    );
}

#[test]
fn a_control_event_moves_one_node_state_and_leaves_the_other_alone() {
    // Two sines, one event. The compiled slot names a node instance, so exactly one
    // state moves — the property that makes a slot different from a name lookup, which
    // could match either.
    const SECOND: NodeId = NodeId::new(3);
    let ir = GraphIr::builder()
        .node(SOURCE, sine(440.0), ExecutionScope::Voice)
        .node(SECOND, sine(440.0), ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");
    let plan = admit(&ir, profile(256, ChannelLayout::Mono));

    let first_slot = plan
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the first sine declares a frequency");
    let second_slot = plan
        .resolve_parameter(SECOND, parameters::SINE_FREQUENCY)
        .expect("the second sine declares a frequency");
    assert_ne!(
        first_slot.index(),
        second_slot.index(),
        "two nodes of one kind are two slots, because a slot is an instance"
    );

    // Moving the *unpatched* node's frequency changes nothing that reaches the output.
    let untouched = render(&plan, 256);
    let (_control, mut renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("prepares");
    let events = [TimedEvent::new(
        EventEnvelope::new(renderer.epoch(), SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::SetParameter {
            slot: second_slot,
            value: ParameterValue::new(110.0).expect("finite"),
        },
    )];
    let mut samples = vec![0.0; 256];
    let block = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("a mono block");
    renderer
        .render(block, TimedEvents::new(&events))
        .expect("renders");
    assert_eq!(
        samples, untouched,
        "the event addressed the sine nothing is patched into, so the output is unchanged"
    );
}

#[test]
fn a_gain_reading_its_own_slot_scales_rather_than_silences() {
    // The in-place case, end to end. The arena merges a gain with its input when nothing
    // reads that input again, so the kernel is handed `InputBuffer::InPlace` — and the
    // three input states have to stay three: an in-place input treated as unpatched
    // renders silence, which is plausible audio and a silent bug.
    const GAIN: NodeId = NodeId::new(5);
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            GAIN,
            IrNodeKind::Gain {
                factor: GainFactor::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
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
        .expect("a readable plan");
    let plan = admit(&ir, profile(256, ChannelLayout::Mono));

    let merged = plan.ops().iter().any(|op| match op {
        PlanOp::Node(step) => step
            .inputs()
            .first()
            .copied()
            .flatten()
            .is_some_and(|input| input == step.out()),
        PlanOp::Output { .. } => false,
    });
    assert!(
        merged,
        "the arena should have given the gain its input's slot; without that this test \
         checks nothing about the in-place path"
    );

    // Offline, so the `Q` frames of silence ADR-0001 clause 6 primes the carry with are
    // trimmed by the alignment the offline path owns rather than counted as a result.
    let rendered = render_offline(
        plan,
        FrameCount::new(128),
        PlanPosition::ZERO,
        &[] as &[OfflineEvent],
    )
    .expect("renders");
    assert!(
        rendered.iter().all(|sample| (*sample - 0.25).abs() < 1e-6),
        "0.5 scaled by 0.5 is 0.25 on every sample, not silence"
    );
}

#[test]
fn a_widened_signal_is_copied_by_a_scheduled_kernel() {
    // ADR-0002 clause 7's operation is a node step like any other after P02-T005: it has
    // prepared data, a state record, and a kernel resolved from the same registry. What
    // it must not have is a special case in the loop.
    let plan = admit(
        &common::source_plan(IrNodeKind::Constant {
            level: Amplitude::new(0.25).expect("finite"),
        }),
        profile(256, ChannelLayout::Stereo),
    );

    let steps = plan
        .ops()
        .iter()
        .filter(|op| matches!(op, PlanOp::Node(_)))
        .count();
    assert_eq!(
        steps, 2,
        "one constant and one copy; the two output writes are the renderer's boundary"
    );
    assert_eq!(
        plan.prepared_nodes().len(),
        2,
        "the copy has prepared data of its own, so the report counts it"
    );

    let rendered = render_offline(
        plan,
        FrameCount::new(64),
        PlanPosition::ZERO,
        &[] as &[OfflineEvent],
    )
    .expect("renders");
    assert!(
        rendered
            .as_chunks::<2>()
            .0
            .iter()
            .all(|frame| (frame[0] - 0.25).abs() < 1e-6 && (frame[1] - 0.25).abs() < 1e-6),
        "both channels carry the widened signal"
    );
}

/// `P05-S001`: a declared kind is named in the registry only where it is declared, where
/// its prepared data is built, and in arms that defer to the declaration.
///
/// The registry's functions match exhaustively over `IrNodeKind`, so a declared kind still
/// appears in each of them; what the declaration removes is the *fact* those arms used to
/// carry. This holds every arm of the sawtooth to one of three forms: the declaration's own
/// (`Some(&SAW)`), `prepare`'s field destructuring (the one arm that stays until parameters
/// are slots), or an exact one-line forwarding arm — the declaration's descriptor, or one
/// declared field through the `Option`'s `0` fallback. An arm restating a port, a control or
/// a byte count, or forwarding through a second expression, matches none and fails here; `node.rs`'s own
/// `a_declared_kinds_registry_facts_derive_from_its_declaration` is the other half, that
/// the deferring arms reach the declaration rather than a copy of it.
#[test]
fn a_declared_kind_appears_in_the_registry_only_by_deferring_to_its_declaration() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/node.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    // Production code only: the test module names kinds freely.
    let production = source.split("#[cfg(test)]").next().unwrap_or("");
    let mut offending = Vec::new();
    let mut seen = 0;
    for (index, line) in production.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") || !trimmed.contains("IrNodeKind::Saw") {
            continue;
        }
        seen += 1;
        let declares = trimmed == "IrNodeKind::Saw { .. } => Some(&SAW),";
        let prepares = trimmed == "IrNodeKind::Saw {";
        // A deferring arm is one line, ends the arm, and forwards exactly one thing: the
        // declaration's descriptor, or one of its fields through the `0` fallback the
        // `Option` needs. Anything else — a literal, a second expression, a body on the
        // next line — is not a forwarding form. An independent review found the earlier
        // token check accepting `declared.map_or(16, |_| 16)`.
        let field_forward = trimmed
            .strip_prefix("IrNodeKind::Saw { .. } => return declared.map_or(0, |d| d.")
            .and_then(|rest| rest.strip_suffix("),"))
            .is_some_and(|field| {
                !field.is_empty() && field.chars().all(|c| c.is_ascii_lowercase() || c == '_')
            });
        let defers = trimmed
            == "IrNodeKind::Saw { .. } => return declared.map(NodeDeclaration::descriptor),"
            || field_forward;
        if !(declares || prepares || defers) {
            offending.push(format!("{}:{} {trimmed}", path.display(), index + 1));
        }
    }
    assert!(
        seen >= 3,
        "the scan expects the declaration, `prepare` and at least one deferring arm; it \
         found {seen} mentions, so it is reading the wrong file or the shape moved"
    );
    assert!(
        offending.is_empty(),
        "an arm of a declared kind must defer to its declaration rather than restate a fact:\n  {}",
        offending.join("\n  ")
    );
}
