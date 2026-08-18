//! P02-T003: what lowering compiles identities into, and what it makes visible.
//!
//! Two of the master plan's Phase 2 work items are checked here. **Compact numeric
//! slots**: an event carries a compiled `ParameterSlot`, resolved off the audio thread,
//! so the render loop indexes where Phase 1 searched a routing table for a
//! `(node, parameter)` pair. **Implicit operations**: a conversion appears in the
//! schedule as its own operation rather than hiding inside an output that writes
//! "every channel".

mod common;

use common::{OUTPUT, SOURCE, admit, profile, source_plan};
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::diagnostics::CompileWarning;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, ParameterId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::{ParameterSlot, PlanOp};
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, Frequency, ParameterValue};
use synth_engine_v2::render::{
    AudioBlockMut, EventEnvelope, EventPayload, PreparedRenderer, Renderer, TimedEvent, TimedEvents,
};
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor, TimeSource};

fn sine() -> IrNodeKind {
    IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    }
}

#[test]
fn an_address_compiles_to_a_slot_and_an_unknown_one_resolves_to_nothing() {
    let plan = admit(&source_plan(sine()), profile(256, ChannelLayout::Mono));

    let frequency = plan
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency");
    let amplitude = plan
        .resolve_parameter(SOURCE, parameters::SINE_AMPLITUDE)
        .expect("the sine declares an amplitude");
    assert_ne!(
        frequency, amplitude,
        "two parameters of one node are two slots"
    );

    // The point of resolving off the audio thread: an address the plan does not have is
    // caught where a caller can still be told about it, instead of becoming an event the
    // renderer silently does nothing with.
    assert_eq!(
        plan.resolve_parameter(OUTPUT, parameters::SINE_FREQUENCY),
        None
    );
    assert_eq!(
        plan.resolve_parameter(SOURCE, ParameterId::new(9)),
        None,
        "a parameter identity this node does not declare has no slot"
    );
}

#[test]
fn every_addressable_parameter_has_a_distinct_slot_into_the_target_table() {
    let plan = admit(&source_plan(sine()), profile(256, ChannelLayout::Mono));

    let mut seen: Vec<ParameterSlot> = Vec::new();
    for address in plan.parameter_addresses() {
        assert!(
            address.slot.index() < plan.parameter_targets().len(),
            "every address must resolve inside the target table the renderer indexes"
        );
        assert!(
            !seen.contains(&address.slot),
            "two addresses share a slot, so one of them writes the other's parameter"
        );
        seen.push(address.slot);
    }
    assert_eq!(seen.len(), plan.parameter_targets().len());
}

#[test]
fn a_slot_from_another_plan_is_refused_by_identity_rather_than_applied() {
    // The failure this prevents is not a panic: after a plan swap, an in-flight event
    // resolved against the old plan holds an index that is perfectly valid *here*, and
    // applying it would silence a different parameter than the one it names. Two plans
    // compiled from the same IR are the sharpest case, because their tables are
    // identical in every way except identity.
    let ir = source_plan(sine());
    let elsewhere = admit(&ir, profile(256, ChannelLayout::Mono));
    let plan = admit(&ir, profile(256, ChannelLayout::Mono));
    assert_ne!(
        elsewhere.id(),
        plan.id(),
        "two compilations are two plans, whatever they were compiled from"
    );

    let foreign = elsewhere
        .resolve_parameter(SOURCE, parameters::SINE_AMPLITUDE)
        .expect("the sine declares an amplitude");

    let rendered = render_offline(
        plan,
        FrameCount::new(128),
        PlanPosition::ZERO,
        &[OfflineEvent::new(
            SampleTime::ZERO,
            synth_engine_v2::render::EventPayload::SetParameter {
                slot: foreign,
                // Silencing the sine is what this event would do if it were applied.
                value: ParameterValue::new(0.0).expect("finite"),
            },
        )],
    )
    .expect("a foreign slot is refused, not a fault");

    assert!(
        rendered.iter().any(|sample| sample.abs() > 0.1),
        "the sine is still sounding, so the foreign slot did not reach its target"
    );
}

#[test]
fn a_mono_source_into_a_stereo_stream_compiles_one_operation_per_channel() {
    // ADR-0002 clause 7: every conversion is a scheduled operation with an identity.
    // Phase 1 hid the duplication inside an output operation that wrote "every
    // channel", which is the same audio and a different claim.
    let plan = admit(&source_plan(sine()), profile(256, ChannelLayout::Stereo));

    let outputs: Vec<(usize, usize)> = plan
        .ops()
        .iter()
        .filter_map(|op| match op {
            PlanOp::OutputChannel { source, channel } => Some((source.index(), channel.get())),
            _ => None,
        })
        .collect();
    // The widening is a node step like any other: the schedule holds one operation that
    // reads the mono buffer and writes a second one, and nothing about it says "copy" to
    // the renderer beyond the kernel the compiler resolved.
    let duplications: Vec<(usize, usize)> = plan
        .ops()
        .iter()
        .filter_map(|op| match op {
            PlanOp::Node(step) => step
                .inputs()
                .first()
                .copied()
                .flatten()
                .map(|source| (source.index(), step.out().index())),
            PlanOp::OutputChannel { .. } => None,
        })
        .collect();

    assert_eq!(
        outputs.len(),
        2,
        "a stereo stream is two output operations, one per channel"
    );
    assert_eq!(outputs[0].1, 0);
    assert_eq!(outputs[1].1, 1);
    assert_eq!(
        duplications.len(),
        1,
        "widening mono to stereo is one scheduled operation, not something the output \
         operation does quietly"
    );
    assert_eq!(
        duplications[0].0, outputs[0].0,
        "the duplication reads the mono signal that reached the port"
    );
    assert_eq!(
        duplications[0].1, outputs[1].0,
        "and writes the buffer the second channel is served from"
    );
    assert_ne!(
        outputs[0].0, outputs[1].0,
        "ADR-0002 clause 2: two channels are two buffers"
    );
}

#[test]
fn a_mono_stream_compiles_exactly_one_output_operation() {
    let plan = admit(&source_plan(sine()), profile(256, ChannelLayout::Mono));
    let outputs = plan
        .ops()
        .iter()
        .filter(|op| matches!(op, PlanOp::OutputChannel { .. }))
        .count();
    assert_eq!(outputs, 1);
}

#[test]
fn buffer_slots_are_dense_and_every_operation_indexes_inside_the_arena() {
    // "Compile stable names and IDs to compact numeric slots": every operation indexes
    // inside the arena the plan declares. A gap would mean the renderer allocates a
    // buffer nothing writes; an index past the end would mean an operation silently
    // does nothing.
    const GAIN: NodeId = NodeId::new(3);
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Voice)
        .node(
            GAIN,
            IrNodeKind::Gain {
                factor: synth_engine_v2::quantities::GainFactor::new(0.5).expect("finite"),
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
        .expect("admissible");

    // Two signals, one buffer: the sine's value dies at the gain, which declares itself
    // in-place safe, so ADR-0005's merge gives the chain one slot. Before P02-T004 this
    // was two.
    assert_eq!(
        plan.buffer_count(),
        1,
        "a source through a gain is one value passing through one buffer"
    );
    for op in plan.ops() {
        let slots = match op {
            PlanOp::Node(step) => {
                let mut slots = vec![step.out().index()];
                slots.extend(step.inputs().iter().flatten().map(|slot| slot.index()));
                slots
            }
            PlanOp::OutputChannel { source, .. } => vec![source.index()],
        };
        for slot in slots {
            assert!(
                slot < plan.buffer_count(),
                "operation {op:?} indexes buffer {slot} of {}",
                plan.buffer_count()
            );
        }
    }
}

#[test]
fn a_refused_foreign_slot_is_counted_rather_than_only_ignored() {
    // A counter nothing reads is the failure this crate keeps citing — `LIMIT-0013`'s
    // drop counters existed for years with no consumer. If a foreign slot is going to be
    // refused silently in the audio, it has to be loud in the report.
    let ir = source_plan(sine());
    let elsewhere = admit(&ir, profile(256, ChannelLayout::Mono));
    let plan = admit(&ir, profile(256, ChannelLayout::Mono));
    let foreign = elsewhere
        .resolve_parameter(SOURCE, parameters::SINE_AMPLITUDE)
        .expect("the sine declares an amplitude");

    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();

    let events = [TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::SetParameter {
            slot: foreign,
            value: ParameterValue::new(0.0).expect("finite"),
        },
    )];

    let mut samples = vec![0.0_f32; 128];
    let output = AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::new(&events))
        .expect("a foreign slot does not fail the call");

    assert_eq!(
        renderer.diagnostics().foreign_slot_events(),
        1,
        "the refusal must reach the report"
    );
    assert_eq!(
        renderer.diagnostics().late_events(),
        0,
        "and must not be conflated with any other counter"
    );
}

#[test]
fn the_scratch_budget_counts_the_buffers_a_stereo_plan_actually_allocates() {
    // A widened channel gets its own buffer, and admission has to know that before
    // preparation allocates it. Counting only the producing nodes would admit a stereo
    // plan against a budget it then exceeds — the defect the event scratch had in
    // Phase 1, in a different place.
    let mono = compile(
        &source_plan(sine()),
        &RenderConfig::new(profile(256, ChannelLayout::Mono)),
    );
    let stereo = compile(
        &source_plan(sine()),
        &RenderConfig::new(profile(256, ChannelLayout::Stereo)),
    );

    let requested = |outcome: &synth_engine_v2::compile::CompileOutcome| {
        outcome
            .report()
            .row(synth_engine_v2::report::ResourceField::BufferScratchBytes)
            .map(|row| row.requested().to_string())
            .expect("the scratch row is always present")
    };

    assert_ne!(
        requested(&mono),
        requested(&stereo),
        "the stereo plan allocates a duplication buffer the mono plan does not, so its \
         reported scratch request must differ"
    );
    assert_eq!(
        stereo.into_plan().expect("admissible").buffer_count(),
        mono.into_plan().expect("admissible").buffer_count() + 1,
        "and the difference is exactly the widened channel's buffer"
    );
}

#[test]
fn an_unreached_stereo_output_is_charged_no_widening_buffer() {
    // Validation admits an output nothing reaches, with a warning, because a patch under
    // construction is a legitimate state. Lowering then emits nothing for it — so
    // charging it a widening buffer would refuse a plan against memory it never takes.
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .build()
        .expect("readable plan");

    let plan = compile(&ir, &RenderConfig::new(profile(256, ChannelLayout::Stereo)))
        .into_plan()
        .expect("an unreached output is admitted");

    assert_eq!(
        plan.buffer_count(),
        1,
        "the sine's buffer and nothing else: no channel was widened"
    );
}

#[test]
fn an_inserted_conversion_is_reported_and_not_only_scheduled() {
    // ADR-0002 clause 7 asks for three things, and the third is the one a reader cannot
    // recover from the other two: the schedule and the buffer count say what the plan
    // costs, while the diagnostic says what the compiler decided on the author's behalf.
    let outcome = compile(
        &source_plan(sine()),
        &RenderConfig::new(profile(256, ChannelLayout::Stereo)),
    );

    let conversions: Vec<_> = outcome
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, CompileWarning::ConversionInserted { .. }))
        .collect();
    assert_eq!(conversions.len(), 1, "one edge was widened, so one report");
    assert!(
        conversions[0].to_string().contains("mono-to-stereo"),
        "the diagnostic must name what was inserted: {}",
        conversions[0]
    );

    // And a mono stream converts nothing, so it reports nothing.
    let mono = compile(
        &source_plan(sine()),
        &RenderConfig::new(profile(256, ChannelLayout::Mono)),
    );
    assert!(
        !mono
            .warnings()
            .iter()
            .any(|warning| matches!(warning, CompileWarning::ConversionInserted { .. })),
        "a plan whose layouts already agree has no conversion to report"
    );
}

#[test]
fn foreign_slots_do_not_consume_a_quantum_capacity() {
    // The ordinary post-swap case: a burst of events resolved against the old plan
    // arrives after the swap. They must be refused and counted like a stale epoch —
    // *before* the per-quantum capacity check — or a plan swap under automation would
    // fail the call rather than ignore the stragglers.
    let ir = source_plan(sine());
    let elsewhere = admit(&ir, profile(256, ChannelLayout::Mono));
    let plan = admit(&ir, profile(256, ChannelLayout::Mono));
    let capacity = plan.max_events_per_quantum().get();
    let foreign = elsewhere
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency");

    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();

    // One more than the quantum admits, all of them from the other plan.
    let events: Vec<TimedEvent> = (0..=capacity)
        .map(|index| {
            TimedEvent::new(
                EventEnvelope::new(
                    epoch,
                    SampleTime::new(u64::from(index)),
                    TimeSource::Compiled,
                ),
                EventPayload::SetParameter {
                    slot: foreign,
                    value: ParameterValue::new(300.0).expect("finite"),
                },
            )
        })
        .collect();

    let mut samples = vec![0.0_f32; 128];
    let output = AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::new(&events))
        .expect("events from another plan are refused, not a failed call");

    assert_eq!(
        renderer.diagnostics().foreign_slot_events(),
        u64::from(capacity) + 1
    );
}
