//! Phase 3's first vertical slice: exact compiled note edges through actual host calls.
//!
//! The renderer already proved that one preselected event span is sample-positioned.
//! These tests add the missing scheduler boundary: the same prepared plan and compiled
//! list are driven through four different callback sequences, and the scheduler releases
//! each edge only to the call that renders its absolute quantum.

mod common;

use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, NormalizedLevel, Seconds};
use synth_engine_v2::render::{
    AudioBlockMut, EventPayload, NoteEdge, PreparedRenderer, Renderer, TimedEvents,
};
use synth_engine_v2::schedule::{
    CompiledEvent, CompiledEventScheduler, SchedulePrepareError, ScheduledRenderError,
};
use synth_engine_v2::time::{PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);
const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);

const Q: u64 = QUANTUM_FRAMES as u64;
const TOTAL_FRAMES: usize = 4_096;
const ON: u64 = 2 * Q + 17;
const OFF: u64 = 40 * Q + 3;

const WHOLE: [usize; 1] = [4_096];
const BLOCKS_256: [usize; 16] = [256; 16];
const BLOCKS_64: [usize; 64] = [64; 64];
const IRREGULAR: [usize; 10] = [17, 511, 3, 64, 1_024, 1, 700, 256, 63, 1_457];

fn gated_constant() -> GraphIr {
    GraphIr::builder()
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
                sustain: NormalizedLevel::FULL,
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
        .expect("a readable plan")
}

fn compiled_note(plan: &CompiledPlan, time: u64, edge: NoteEdge) -> CompiledEvent {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope accepts note edges");
    CompiledEvent::new(SampleTime::new(time), EventPayload::Note { slot, edge })
}

fn render_partition(plan: &CompiledPlan, partition: &[usize]) -> Vec<f32> {
    assert_eq!(
        partition.iter().sum::<usize>(),
        TOTAL_FRAMES,
        "every callback family covers the same output duration"
    );

    let mut renderer = PreparedRenderer::prepare(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let events = [
        compiled_note(plan, ON, NoteEdge::On),
        compiled_note(plan, OFF, NoteEdge::Off),
    ];
    let mut scheduler =
        CompiledEventScheduler::prepare(&renderer, &events).expect("the schedule is valid");
    let mut rendered = Vec::with_capacity(TOTAL_FRAMES);

    for &frames in partition {
        let mut block = vec![0.0_f32; frames];
        let output = AudioBlockMut::new(&mut block, frames, ChannelLayout::Mono)
            .expect("the output block is shaped correctly");
        scheduler
            .render(&mut renderer, output)
            .expect("the scheduler releases only events for this call");
        rendered.extend_from_slice(&block);
    }

    assert!(scheduler.is_complete(), "both compiled edges were released");
    assert_eq!(
        renderer.diagnostics().late_events(),
        0,
        "the scheduler must never rely on the renderer's preserving late clamp"
    );
    rendered
}

#[test]
fn a_compiled_note_is_exact_and_bit_identical_across_actual_host_partitions() {
    let plan = common::admit(
        &gated_constant(),
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
    );
    let partitions: [&[usize]; 4] = [&WHOLE, &BLOCKS_256, &BLOCKS_64, &IRREGULAR];
    let renders: Vec<Vec<f32>> = partitions
        .iter()
        .map(|partition| render_partition(&plan, partition))
        .collect();
    let reference = renders.first().expect("four renders");

    for (index, rendered) in renders.iter().enumerate().skip(1) {
        assert_eq!(
            rendered, reference,
            "actual host partition {index} changed the rendered samples"
        );
    }

    let sounding = ON + Q..OFF + Q;
    for (frame, sample) in reference.iter().copied().enumerate() {
        let expected = if sounding.contains(&(frame as u64)) {
            1.0
        } else {
            0.0
        };
        assert_eq!(
            sample, expected,
            "output frame {frame} must reflect the exact compiled edge plus the declared Q carry"
        );
    }
}

#[test]
fn preparation_rejects_a_descending_compiled_list() {
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let renderer = PreparedRenderer::prepare(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let events = [
        compiled_note(&plan, Q, NoteEdge::On),
        compiled_note(&plan, 0, NoteEdge::Off),
    ];

    assert_eq!(
        CompiledEventScheduler::prepare(&renderer, &events).expect_err("order is invalid"),
        SchedulePrepareError::EventsOutOfOrder {
            event_index: 1,
            previous: SampleTime::new(Q),
            time: SampleTime::ZERO,
        }
    );
}

#[test]
fn preparation_rejects_a_slot_from_another_plan() {
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let foreign = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let renderer = PreparedRenderer::prepare(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let events = [compiled_note(&foreign, 0, NoteEdge::On)];

    assert_eq!(
        CompiledEventScheduler::prepare(&renderer, &events)
            .expect_err("a foreign plan slot is invalid"),
        SchedulePrepareError::ForeignPlan {
            event_index: 0,
            expected: plan.id(),
            actual: foreign.id(),
        }
    );
}

#[test]
fn preparation_rejects_an_overfull_quantum_before_any_host_partition_is_chosen() {
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let renderer = PreparedRenderer::prepare(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let admissible = plan
        .max_events_per_quantum()
        .as_usize()
        .expect("the event capacity fits this platform");
    let events = vec![compiled_note(&plan, 0, NoteEdge::On); admissible + 1];

    assert_eq!(
        CompiledEventScheduler::prepare(&renderer, &events)
            .expect_err("the absolute quantum is over-full"),
        SchedulePrepareError::QuantumTooDense {
            quantum_start: SampleTime::ZERO,
            admissible: plan.max_events_per_quantum(),
        }
    );
    assert_eq!(renderer.clock(), SampleTime::ZERO);
}

#[test]
fn one_multi_quantum_host_call_accepts_the_full_capacity_of_each_quantum() {
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let mut renderer = PreparedRenderer::prepare(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let admissible = plan
        .max_events_per_quantum()
        .as_usize()
        .expect("the event capacity fits this platform");
    let mut events = Vec::with_capacity(admissible * 3);
    for quantum in 0_u64..3 {
        for _ in 0..admissible {
            events.push(compiled_note(&plan, quantum * Q, NoteEdge::On));
        }
    }
    let mut scheduler =
        CompiledEventScheduler::prepare(&renderer, &events).expect("every quantum fits");
    let mut samples = [0.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");

    scheduler
        .render(&mut renderer, output)
        .expect("the total may exceed one quantum's capacity");

    assert!(scheduler.is_complete());
    assert_eq!(renderer.clock(), SampleTime::new(3 * Q));
}

#[test]
fn a_schedule_cannot_cross_a_reprepared_epoch() {
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let first = PreparedRenderer::prepare(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("first preparation succeeds");
    let event = [compiled_note(&plan, 0, NoteEdge::On)];
    let mut scheduler =
        CompiledEventScheduler::prepare(&first, &event).expect("the schedule is valid");
    let mut second = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("second preparation succeeds");
    let mut samples = [0.0_f32; 64];
    let output = AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");

    assert_eq!(
        scheduler
            .render(&mut second, output)
            .expect_err("the old schedule must be stale"),
        ScheduledRenderError::EpochMismatch {
            schedule: first.epoch(),
            renderer: second.epoch(),
        }
    );
}

#[test]
fn bypassing_the_scheduler_cannot_make_a_compiled_event_late_silently() {
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let mut renderer = PreparedRenderer::prepare(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let event = [compiled_note(&plan, 0, NoteEdge::On)];
    let mut scheduler =
        CompiledEventScheduler::prepare(&renderer, &event).expect("the schedule is valid");

    let mut first = [0.0_f32; 128];
    let output = AudioBlockMut::new(&mut first, 128, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");
    renderer
        .render(output, TimedEvents::EMPTY)
        .expect("the direct call advances past the event");

    let mut second = [0.0_f32; 64];
    let output = AudioBlockMut::new(&mut second, 64, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");
    assert_eq!(
        scheduler
            .render(&mut renderer, output)
            .expect_err("the scheduler detects its missed event"),
        ScheduledRenderError::MissedEvent {
            event: SampleTime::ZERO,
            clock: SampleTime::new(Q),
        }
    );
    assert_eq!(renderer.diagnostics().late_events(), 0);
}
