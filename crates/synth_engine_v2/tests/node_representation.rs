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

/// `P05-S001`, extended by `P05-S002`: a declared kind is named in the registry only where it
/// is declared, where its prepared data is built, and in arms that forward to the declaration.
///
/// The registry's functions match exhaustively over `IrNodeKind`, so a declared kind still
/// appears in each of them; what the declaration removes is the *fact* those arms used to
/// carry. This holds every arm of every declared kind to one of three forms: the declaration's
/// own (`Some(&NAME)`), a line inside the kind's own `prepare_<kind>` function (its
/// preparation entry, which destructures its variant), or an exact one-line forwarding arm — the declaration's descriptor,
/// or one declared field through the `Option`'s `0` fallback. An arm restating a port, a
/// control or a byte count, or forwarding through a second expression, matches none and fails
/// here; `node.rs`'s own `a_declared_kinds_registry_facts_derive_from_its_declaration` is the
/// other half, that the forwarding arms reach the declaration rather than a copy of it.
///
/// The list of declared kinds is stated here rather than read from `declaration`, so adding a
/// kind to that match without adding it here is caught by the count check below.
#[test]
fn a_declared_kind_appears_in_the_registry_only_by_deferring_to_its_declaration() {
    // The variant as it is spelled in a pattern — fieldless kinds have no `{ .. }` — and
    // the declaration constant it forwards to.
    const DECLARED: [(&str, &str); 12] = [
        ("Saw { .. }", "SAW"),
        ("VelocityScaler { .. }", "VELOCITY_SCALER"),
        ("Sampler { .. }", "SAMPLER"),
        ("Envelope { .. }", "ENVELOPE"),
        ("Sine { .. }", "SINE"),
        ("Silence", "SILENCE"),
        ("Constant { .. }", "CONSTANT"),
        ("Impulse { .. }", "IMPULSE"),
        ("Amplifier", "AMPLIFIER"),
        ("Monitor", "MONITOR"),
        ("Gain { .. }", "GAIN"),
        ("Filter { .. }", "FILTER"),
    ];

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/node.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    // Production code only: the test module names kinds freely.
    let production = source.split("#[cfg(test)]").next().unwrap_or("");
    // Each kind's own `prepare_<kind>` function, by line range: the one place a declared
    // kind's variant may be destructured into a fact, because that function *is* the
    // declaration's preparation entry. A range runs from the signature to the first bare
    // `}` at column zero, so an exemption cannot leak into another function — and it is
    // keyed by the kind's name, so one kind's variant inside another kind's preparation
    // is caught. The registry's `prepare` itself gets no exemption: it forwards.
    let lines: Vec<&str> = production.lines().collect();
    let prepare_range = |name: &str| -> Option<std::ops::RangeInclusive<usize>> {
        let signature = format!("fn prepare_{}(", name.to_ascii_lowercase());
        let start = lines.iter().position(|line| line.starts_with(&signature))?;
        let end = lines
            .iter()
            .enumerate()
            .skip(start)
            .find(|(_, line)| **line == "}")?
            .0;
        Some(start..=end)
    };
    let declared_arms = production
        .lines()
        .filter(|line| line.trim().ends_with("),") && line.contains("=> Some(&"))
        .count();
    assert_eq!(
        declared_arms,
        DECLARED.len(),
        "`declaration` has {declared_arms} arms and this scan knows {}; declare the kind here too",
        DECLARED.len()
    );

    let mut offending = Vec::new();
    for (pattern, constant) in DECLARED {
        let name = pattern.split(' ').next().unwrap_or(pattern);
        let variant = format!("IrNodeKind::{name}");
        let arm = format!("IrNodeKind::{pattern} =>");
        let declares = format!("{arm} Some(&{constant}),");
        let descriptor = format!("{arm} declared.map(NodeDeclaration::descriptor),");
        let field_prefix = format!("{arm} return declared.map_or(0, |d| d.");
        let mut seen = 0;
        for (index, line) in production.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || !trimmed.contains(&variant) {
                continue;
            }
            seen += 1;
            // A kind's own `prepare_<kind>` function destructures its variant into its
            // prepared record, and is the only place allowed to hold that fact. The
            // exemption is by **position** — inside that function — not by what the line
            // says: an earlier revision accepted any line mentioning `PreparedNode::`, which
            // a trailing comment could supply, and an independent review found it.
            let prepares = prepare_range(name).is_some_and(|range| range.contains(&index));
            let field_forward = trimmed
                .strip_prefix(field_prefix.as_str())
                .and_then(|rest| rest.strip_suffix("),"))
                .is_some_and(|field| {
                    !field.is_empty() && field.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                });
            if !(trimmed == declares || prepares || trimmed == descriptor || field_forward) {
                offending.push(format!("{}:{} {trimmed}", path.display(), index + 1));
            }
        }
        assert!(
            seen >= 3,
            "the scan expects {name}'s declaration, `prepare_*` destructure and at least one forwarding \
             arm; it found {seen} mentions, so it is reading the wrong file or the shape moved"
        );
    }
    assert!(
        offending.is_empty(),
        "an arm of a declared kind must forward to its declaration rather than restate a fact:\n  {}",
        offending.join("\n  ")
    );
}

/// `P05-S006`: what discovery says a kind's ports are is what the compiler validates against.
///
/// Both surfaces derive from one declaration, and this is the check across the crate's
/// public boundary that they still agree: for every catalog entry, the port set the
/// compiler's own `ports` reports for that kind — looked up by the catalog's stable
/// identity, never its label — equals the catalog's, field by field. The output node has
/// no catalog entry, because it has no declaration.
#[test]
fn discovery_and_validation_describe_the_same_ports() {
    use synth_engine_v2::node::{NodeKindId, PortDescription, catalog, kind_id, ports};
    use synth_engine_v2::quantities::{CutoffFrequency, Resonance, Seconds};

    let sample = |id: NodeKindId| -> IrNodeKind {
        match id {
            NodeKindId::Silence => IrNodeKind::Silence,
            NodeKindId::Constant => IrNodeKind::Constant {
                level: Amplitude::new(0.5).expect("finite"),
            },
            NodeKindId::Impulse => IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
            NodeKindId::Sine => sine(440.0),
            NodeKindId::Saw => IrNodeKind::Saw {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            NodeKindId::Gain => IrNodeKind::Gain {
                factor: GainFactor::new(0.5).expect("finite"),
            },
            NodeKindId::Amplifier => IrNodeKind::Amplifier,
            NodeKindId::VelocityScaler => IrNodeKind::VelocityScaler {
                sensitivity: synth_engine_v2::quantities::NormalizedLevel::FULL,
            },
            NodeKindId::Sampler => IrNodeKind::Sampler {
                map: synth_engine_v2::sample::SampleMapRef::new(0),
                level: Amplitude::UNITY,
                velocity_sensitivity: synth_engine_v2::quantities::NormalizedLevel::FULL,
                start_offset: synth_engine_v2::quantities::NormalizedLevel::ZERO,
                play_mode: synth_engine_v2::sample::PlayMode::Sustain,
                direction: synth_engine_v2::sample::PlayDirection::Forward,
            },
            NodeKindId::Monitor => IrNodeKind::Monitor,
            NodeKindId::Filter => IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(1_000.0).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            NodeKindId::Envelope => IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: synth_engine_v2::quantities::NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: synth_engine_v2::quantities::NormalizedLevel::FULL,
            },
        }
    };

    let entries = catalog();
    assert_eq!(
        entries.len(),
        12,
        "every kind but the output node is discoverable"
    );
    for entry in entries {
        // Looked up by the stable identity, never by the display label: a renamed kind is
        // still the same kind, and this is what holds the label apart from the identity.
        let kind = sample(entry.id);
        assert_eq!(kind_id(kind), Some(entry.id), "{}", entry.name);
        let validated: Vec<PortDescription> = ports(kind, ChannelLayout::Mono)
            .iter()
            .map(|port| PortDescription {
                id: port.id(),
                direction: port.direction(),
                domain: port.domain(),
                layout: port.layout(),
            })
            .collect();
        assert_eq!(
            entry.ports, validated,
            "{}: discovery and validation disagree",
            entry.name
        );
        assert!(
            !entry.parameters.is_empty() || !entry.playable,
            "{}",
            entry.name
        );
    }
}
