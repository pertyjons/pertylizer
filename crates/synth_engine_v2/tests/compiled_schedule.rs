//! Phase 3's first vertical slice: exact compiled note edges through actual host calls.
//!
//! The renderer already proved that one preselected event span is sample-positioned.
//! These tests add the missing scheduler boundary: the same prepared plan and compiled
//! list are driven through four different callback sequences, and the scheduler releases
//! each edge only to the call that renders its absolute quantum.

mod common;

use synth_engine_v2::admit::AdmissionError;
use synth_engine_v2::diagnostics::RenderError;
use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::profile::{EventLimits, HostProfile, ProducerShares, RenderLimits};
use synth_engine_v2::publish::{ProducerClass, PublicationArbiter};
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, EventCount, NormalizedLevel, Seconds};
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::schedule::{
    AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, CompiledStreamError,
    PlanEvent, SchedulePrepareError, ScheduledRenderError,
};
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);
const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);

const Q: u64 = QUANTUM_FRAMES as u64;

/// The anchor every fixture that does not seek uses.
///
/// Named once because it is passed twice — the renderer is prepared with it and the
/// schedule is placed at it — and two literals drifting apart is exactly the mistake the
/// placement argument exists to make visible.
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
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
        // Large enough for the density fixture below, which mints one occurrence per
        // compiled event across three quanta and releases none — 288 note-ons against the
        // profile's 512 held notes. A smaller declaration is refused, which is the identity
        // partition working: a producer cannot sound more notes than it declared.
        .declaring(common::compiled_notes(512))
        .build()
        .expect("a readable plan")
}

/// The one arbiter a stream publishes through.
///
/// ADR-0046 clause 2 admits exactly one per stream, so every test that renders a schedule
/// builds one and passes it in rather than letting the scheduler own a private store.
fn arbiter(block: u64) -> PublicationArbiter {
    PublicationArbiter::prepare(&common::profile(block, ChannelLayout::Mono))
        .expect("the publication store is preparable")
}

/// Admit a stream, expecting it to fit.
///
/// Preparation accepts only an admitted stream, so every fixture that used to hand
/// `prepare` a bare list now proves the list fits its share first — which is the boundary
/// this file is testing the far side of.
fn admitted(plan: &CompiledPlan, events: &[PlanEvent]) -> AdmittedCompiledStream {
    AdmittedCompiledStream::admit(plan, events).expect("the compiled stream fits its share")
}

fn compiled_note(plan: &CompiledPlan, time: u64, on: bool) -> PlanEvent {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope accepts note edges");
    // A compiled list names the node on **both** edges, which is how preparation pairs
    // them; the stamped event names it on the on edge alone.
    let payload = if on {
        CompiledPayload::NoteOn { slot }
    } else {
        CompiledPayload::NoteOff { slot }
    };
    PlanEvent::new(PlanPosition::new(time), payload)
}

fn render_partition(plan: &CompiledPlan, partition: &[usize]) -> Vec<f32> {
    assert_eq!(
        partition.iter().sum::<usize>(),
        TOTAL_FRAMES,
        "every callback family covers the same output duration"
    );

    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [
        compiled_note(plan, ON, true),
        compiled_note(plan, OFF, false),
    ];
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &admitted(plan, &events))
        .expect("the schedule is valid");
    let mut publication = arbiter(TOTAL_FRAMES as u64);
    let mut rendered = Vec::with_capacity(TOTAL_FRAMES);

    for &frames in partition {
        let mut block = vec![0.0_f32; frames];
        let output = AudioBlockMut::new(&mut block, frames, ChannelLayout::Mono)
            .expect("the output block is shaped correctly");
        scheduler
            .render(&mut renderer, &mut publication, output)
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
fn admission_rejects_a_descending_compiled_stream() {
    // Refused, not sorted: ADR-0023 has not selected a same-sample ordering policy, and the
    // window scan assumes ascending input — an unsorted list would be measured against
    // windows that do not exist in it.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let events = [
        compiled_note(&plan, Q, true),
        compiled_note(&plan, 0, false),
    ];

    assert_eq!(
        AdmittedCompiledStream::admit(&plan, &events).expect_err("order is invalid"),
        CompiledStreamError::EventsOutOfOrder {
            event_index: 1,
            previous: PlanPosition::new(Q),
            position: PlanPosition::ZERO,
        }
    );
}

#[test]
fn admission_rejects_a_slot_from_another_plan() {
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let foreign = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let events = [compiled_note(&foreign, 0, true)];

    assert_eq!(
        AdmittedCompiledStream::admit(&plan, &events).expect_err("a foreign plan slot is invalid"),
        CompiledStreamError::ForeignPlan {
            event_index: 0,
            expected: plan.id(),
            actual: foreign.id(),
        }
    );
}

#[test]
fn preparation_refuses_a_stream_admitted_against_another_plan() {
    // The stream-level counterpart of the check above. An admitted stream already proved
    // every slot belongs to *its* plan, so preparation compares one identity rather than
    // re-walking the list.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let foreign = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [compiled_note(&foreign, 0, true)];
    let stream = admitted(&foreign, &events);

    assert_eq!(
        CompiledEventScheduler::prepare(&mut control, &stream)
            .expect_err("the stream belongs to another plan"),
        SchedulePrepareError::ForeignStream {
            expected: plan.id(),
            actual: foreign.id(),
        }
    );
}

#[test]
fn admission_rejects_a_window_over_the_compiled_share() {
    // **Against the share, not the cap.** Routing this producer through the arbiter made
    // its runtime bound `compiled_event_share`, and ADR-0046 clause 3 makes a compiled
    // runtime miss a producer defect rather than a load condition. So the refusal has to
    // happen where a caller can still be told, or an admitted plan could fault at
    // publication — which is the state the whole admission design exists to remove.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let admissible = plan
        .compiled_event_share()
        .as_usize()
        .expect("the compiled share fits this platform");
    assert!(
        plan.compiled_event_share() < plan.max_events_per_quantum(),
        "the premise: the share is narrower than the cap, so the two refusals differ"
    );
    let events = vec![compiled_note(&plan, 0, true); admissible + 1];

    assert_eq!(
        AdmittedCompiledStream::admit(&plan, &events).expect_err("the window is over its share"),
        CompiledStreamError::Window(AdmissionError::WindowOverShare {
            window_start: PlanPosition::ZERO,
            requested: EventCount::measured(admissible as u32 + 1),
            share: plan.compiled_event_share(),
            quantum: QUANTUM_FRAMES,
        })
    );
}

#[test]
fn admission_is_the_worst_case_over_every_anchor_phase() {
    // ADR-0046 clause 4, and the whole reason a per-absolute-quantum count is the wrong
    // question. These events straddle a quantum boundary: from an anchor at zero they fall
    // in two different quanta, each comfortably under the share, and a per-quantum count
    // admits them. From an anchor one frame later they are in the **same** quantum and
    // overrun it — so an ordinary seek would turn an admitted plan into a publication
    // fault. The sliding window sees it before either anchor exists.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let share = plan
        .compiled_event_share()
        .as_usize()
        .expect("the compiled share fits this platform");
    let before = share / 2;
    let after = share + 1 - before;

    let mut straddling = vec![compiled_note(&plan, Q - 1, true); before];
    straddling.extend(vec![compiled_note(&plan, Q, true); after]);
    assert!(
        before < share && after < share,
        "the premise: neither absolute quantum is over the share on its own"
    );

    assert_eq!(
        AdmittedCompiledStream::admit(&plan, &straddling)
            .expect_err("one Q-frame window holds more than the share"),
        CompiledStreamError::Window(AdmissionError::WindowOverShare {
            // Frame 1, not `Q - 1`: that is the earliest anchor phase at which the two
            // clusters fall in one quantum, which is the phase the plan first fails at.
            window_start: PlanPosition::new(1),
            requested: EventCount::measured(share as u32 + 1),
            share: plan.compiled_event_share(),
            quantum: QUANTUM_FRAMES,
        })
    );

    // The control, and the half-open boundary it pins. A full share at frame 0 and another
    // full share exactly `Q` frames later is admissible: no window of `Q` consecutive
    // frames contains both, because the window starting at 0 ends *before* frame `Q`. A
    // closed window would refuse this and a real plan with it.
    let mut adjacent = vec![compiled_note(&plan, 0, true); share];
    adjacent.extend(vec![compiled_note(&plan, Q, true); share]);
    assert!(
        AdmittedCompiledStream::admit(&plan, &adjacent).is_ok(),
        "two full shares exactly Q apart share no Q-frame window"
    );
}

#[test]
fn preparation_refuses_a_position_before_the_anchor() {
    // The stream begins at the anchor, so a position before it is one this stream does not
    // render. Skipping it would be a silent drop, which ADR-0001 clause 16 forbids; the
    // caller that meant to start here admits the suffix instead, which always fits because
    // every window of a suffix is a window of the whole.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let anchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::new(4 * Q));
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), anchor).expect("preparation succeeds");
    let events = [
        compiled_note(&plan, Q, true),
        compiled_note(&plan, 8 * Q, false),
    ];
    let stream = admitted(&plan, &events);

    assert_eq!(
        CompiledEventScheduler::prepare(&mut control, &stream)
            .expect_err("the first event is behind the anchor"),
        SchedulePrepareError::BeforeAnchor {
            event_index: 0,
            position: PlanPosition::new(Q),
            anchor: PlanPosition::new(4 * Q),
        }
    );
}

#[test]
fn preparation_tells_an_unrepresentable_time_from_a_pre_anchor_one() {
    // The two refusals mean different things and must not share a name. This position is
    // *after* the anchor — it is the engine time that does not fit, because the distance
    // overflows a signed frame delta. Reporting it as `BeforeAnchor` would send someone
    // looking for a seek that never happened.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [compiled_note(&plan, u64::MAX, true)];
    let stream = admitted(&plan, &events);

    assert_eq!(
        CompiledEventScheduler::prepare(&mut control, &stream)
            .expect_err("the engine time does not fit"),
        SchedulePrepareError::TimeUnrepresentable {
            event_index: 0,
            position: PlanPosition::new(u64::MAX),
        }
    );
}

#[test]
fn preparation_places_an_admitted_stream_at_the_anchor() {
    // The other side of the refusal above, and the shape a seek actually has: the plan
    // position moves, the epoch's clock still starts at zero, and placement subtracts the
    // same distance from every event. That uniform shift is what makes admission
    // anchor-independent in the first place.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let anchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::new(4 * Q));
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), anchor).expect("preparation succeeds");
    let events = [
        compiled_note(&plan, 4 * Q + 17, true),
        compiled_note(&plan, 6 * Q + 3, false),
    ];
    let stream = admitted(&plan, &events);
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &stream)
        .expect("the stream starts at the anchor");

    let mut samples = [0.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");
    let mut publication = arbiter(256);
    scheduler
        .render(&mut renderer, &mut publication, output)
        .expect("both placed events fall inside this call");

    assert!(scheduler.is_complete());
    // Plan sample `4Q + 17` is engine time `17` and plan `6Q + 3` is engine `2Q + 3`: the
    // anchor equates plan `4Q` with engine zero, and placement moves both by that distance.
    // The clock lands on `3Q` rather than 256 because a live call's first `Q` output frames
    // are the carry, so 256 output frames consume 192 input frames (ADR-0001 clause 7).
    assert_eq!(renderer.clock(), SampleTime::new(3 * Q));
}

#[test]
fn one_multi_quantum_host_call_accepts_the_full_share_of_each_quantum() {
    // **The compiled share, not the whole cap.** Routing this path through the arbiter
    // changed what one producer may fill: ADR-0046 clause 1 partitions
    // `max_events_per_quantum`, and the compiled class spends `compiled_event_share`. The
    // test's point is unchanged — one call spanning several quanta may present more than
    // any single quantum admits — but the per-quantum bound is now the class's own.
    let host = common::profile(256, ChannelLayout::Mono);
    let plan = common::admit(&gated_constant(), host);
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let admissible = host
        .limits()
        .events()
        .shares()
        .compiled_event_share()
        .as_usize()
        .expect("the compiled share fits this platform");
    let mut events = Vec::with_capacity(admissible * 3);
    for quantum in 0_u64..3 {
        for _ in 0..admissible {
            events.push(compiled_note(&plan, quantum * Q, true));
        }
    }
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &admitted(&plan, &events))
        .expect("every quantum fits");
    let mut samples = [0.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");

    let mut publication = arbiter(256);
    scheduler
        .render(&mut renderer, &mut publication, output)
        .expect("the total may exceed one quantum's share");

    assert!(scheduler.is_complete());
    assert_eq!(renderer.clock(), SampleTime::new(3 * Q));
}

#[test]
fn a_schedule_cannot_cross_a_reprepared_epoch() {
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let (mut first_control, first) =
        StreamControl::open(plan.clone(), ORIGIN).expect("first preparation succeeds");
    let event = [compiled_note(&plan, 0, true)];
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut first_control, &admitted(&plan, &event))
            .expect("the schedule is valid");
    let (_second_control, mut second) =
        StreamControl::open(plan, ORIGIN).expect("second preparation succeeds");
    let mut samples = [0.0_f32; 64];
    let output = AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");

    assert_eq!(
        scheduler
            .render(&mut second, &mut arbiter(256), output)
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
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let event = [compiled_note(&plan, 0, true)];
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &admitted(&plan, &event))
        .expect("the schedule is valid");

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
            .render(&mut renderer, &mut arbiter(256), output)
            .expect_err("the scheduler detects its missed event"),
        ScheduledRenderError::MissedEvent {
            event: SampleTime::ZERO,
            clock: SampleTime::new(Q),
        }
    );
    assert_eq!(renderer.diagnostics().late_events(), 0);
}

#[test]
fn compiled_events_are_charged_to_the_compiled_share() {
    // ADR-0046 clause 2 makes the arbiter "the only normal path that constructs renderer
    // input", and clause 1 charges every event to exactly one class. Before this routing
    // the compiled producer handed the renderer a borrowed slice and spent nothing, so the
    // partition was true of every producer except the one the crate already had.
    let host = common::profile(256, ChannelLayout::Mono);
    let plan = common::admit(&gated_constant(), host);
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [
        compiled_note(&plan, 1, true),
        compiled_note(&plan, 3, false),
    ];
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &admitted(&plan, &events))
        .expect("the schedule is valid");
    let mut publication = arbiter(256);

    // 256 frames, not 64: the stream starts with a full quantum of primed carry, so a
    // 64-frame first call renders no quantum at all and would charge nothing — which would
    // make this assertion pass against a producer that published nothing.
    let mut samples = [0.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped");
    scheduler
        .render(&mut renderer, &mut publication, output)
        .expect("the call renders");
    assert!(scheduler.is_complete(), "both edges were released");

    assert_eq!(
        publication.high_water(ProducerClass::Compiled),
        EventCount::measured(2),
        "both edges were charged to the compiled class"
    );
    for class in ProducerClass::ALL {
        if class == ProducerClass::Compiled {
            continue;
        }
        assert_eq!(
            publication.high_water(class),
            EventCount::NONE,
            "{} spent nothing on a compiled-only stream",
            class.name()
        );
    }
}

#[test]
fn a_forged_publication_fault_ends_the_stream_rather_than_the_call() {
    // ADR-0046 clause 7, enacted rather than reported. The forge is the clause's own third
    // cause — "a caller bypassed the publication contract" — built by handing the scheduler
    // an arbiter prepared for a smaller callback than the one it is asked to render. No
    // conforming caller can reach it, which is why it has to be forged.
    //
    // What the clause demands is not an error return: it is silence over the complete
    // current callback **and every later one in the epoch**, both carries invalidated and
    // `needs_reprepare` published. A test that only checked the `Err` would pass against an
    // implementation that recovered on the next call.
    let host = common::profile(256, ChannelLayout::Mono);
    let plan = common::admit(&gated_constant(), host);
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [compiled_note(&plan, 1, true)];
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &admitted(&plan, &events))
        .expect("the schedule is valid");

    // Prepared for a single quantum; the call below needs four.
    let mut undersized = arbiter(64);

    let mut samples = [9.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped");
    let error = scheduler
        .render(&mut renderer, &mut undersized, output)
        .expect_err("a window past the prepared store is a contract violation");
    assert!(
        matches!(error, ScheduledRenderError::Publication(_)),
        "the fault names publication, got {error:?}"
    );

    assert!(
        samples.iter().all(|sample| *sample == 0.0),
        "the faulting callback is silenced over its complete length"
    );
    assert!(
        renderer.diagnostics().needs_reprepare(),
        "the epoch is over, not the call"
    );
    assert_eq!(
        renderer.diagnostics().publication_faults(),
        1,
        "clause 7 asks for an attributable counter, not only for silence"
    );
    assert_eq!(
        renderer.clock(),
        SampleTime::ZERO,
        "no quantum was rendered, so the clock did not advance"
    );

    // And every later callback, whatever it is asked for.
    let mut later = [9.0_f32; 64];
    let output = AudioBlockMut::new(&mut later, 64, ChannelLayout::Mono).expect("shaped");
    let error = renderer
        .render(output, TimedEvents::EMPTY)
        .expect_err("a faulted epoch renders nothing further");
    assert!(matches!(error, RenderError::NeedsReprepare));
    assert!(
        later.iter().all(|sample| *sample == 0.0),
        "later callbacks are silenced too, which is what makes the fault terminal"
    );
}

#[test]
fn a_forged_charge_overrun_ends_the_stream_on_its_own_branch() {
    // The `charge` half of clause 7's terminal response. The window-refusal case above
    // exercises only `open`, so this branch could return without faulting and leave that
    // test green — an independent review found exactly that gap.
    //
    // The forge is again a caller bypassing the contract: the plan is admitted against a
    // profile whose compiled share fits its events, and then rendered through an arbiter
    // prepared from a profile whose compiled share is one event. No conforming caller pairs
    // a plan with an arbiter from a different profile.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [
        compiled_note(&plan, 65, true),
        compiled_note(&plan, 70, false),
    ];
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &admitted(&plan, &events))
        .expect("two events fit the share");

    let mut narrow =
        PublicationArbiter::prepare(&narrow_compiled_share()).expect("the store is preparable");

    let mut samples = [9.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped");
    let error = scheduler
        .render(&mut renderer, &mut narrow, output)
        .expect_err("the second event overruns a one-event compiled share");
    assert!(
        matches!(error, ScheduledRenderError::Publication(_)),
        "the fault names publication, got {error:?}"
    );

    assert!(
        samples.iter().all(|sample| *sample == 0.0),
        "the faulting callback is silenced over its complete length"
    );
    assert!(renderer.diagnostics().needs_reprepare());
    assert_eq!(renderer.diagnostics().publication_faults(), 1);
    assert_eq!(
        renderer.clock(),
        SampleTime::ZERO,
        "no quantum was rendered, so the clock did not advance"
    );
}

/// A profile identical to the harness one except that compiled work gets one event.
fn narrow_compiled_share() -> HostProfile {
    let host = common::profile(256, ChannelLayout::Mono);
    let defaults = common::defaults_for(&host);
    let one = EventCount::limit(1).expect("positive");
    let shares = ProducerShares::new(
        one,
        defaults.events().shares().authored_runtime_event_share(),
        defaults.events().shares().live_event_share(),
        defaults.events().shares().session_event_share(),
        defaults.events().shares().internal_event_share(),
        defaults.events().shares().release_event_share(),
        defaults.events().shares().release_hold_capacity(),
    )
    .expect("a narrower compiled share still satisfies every relation");
    let events = EventLimits::new(
        defaults.events().max_events_per_quantum(),
        defaults.events().max_note_expansion_per_tick(),
        defaults.events().max_scheduled_events_in_flight(),
        defaults.events().forward_event_horizon(),
        defaults.events().queues(),
        shares,
    )
    .expect("only the compiled share changed");
    let limits = RenderLimits::new(
        defaults.stream(),
        defaults.graph(),
        defaults.voices(),
        events,
        defaults.observation(),
        defaults.mixing(),
        defaults.memory(),
        defaults.script(),
        defaults.recording(),
        defaults.cost(),
    )
    .expect("only the compiled share changed");
    HostProfile::new(host.capabilities(), limits).expect("a valid profile")
}

#[test]
fn a_schedule_refuses_a_second_arbiter() {
    // ADR-0046 clause 2 admits exactly one arbiter per stream, and a bare parameter does
    // not establish that on its own — an independent review pointed out that a caller could
    // hand each callback a fresh, equally sized store, satisfy every capacity bound, and
    // silently restart the high-water history that Phase 3's measurement is going to read.
    // The schedule latches the first identity it publishes through.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [compiled_note(&plan, 65, true)];
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &admitted(&plan, &events))
        .expect("the schedule is valid");

    let mut first = arbiter(256);
    let mut second = arbiter(256);
    assert_ne!(first.id(), second.id(), "identities are never reused");

    let mut samples = [0.0_f32; 128];
    let output = AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("shaped");
    scheduler
        .render(&mut renderer, &mut first, output)
        .expect("the first call adopts this arbiter");

    let mut more = [0.0_f32; 128];
    let output = AudioBlockMut::new(&mut more, 128, ChannelLayout::Mono).expect("shaped");
    match scheduler.render(&mut renderer, &mut second, output) {
        Err(ScheduledRenderError::ForeignArbiter { latched, offered }) => {
            assert_eq!(latched, first.id());
            assert_eq!(offered, second.id(), "the refusal names both stores");
        }
        other => panic!("expected a foreign-arbiter refusal, got {other:?}"),
    }

    // Refused before publication, so the substitution costs the stream nothing.
    assert!(
        !renderer.diagnostics().needs_reprepare(),
        "a caller error is not a contract violation; the stream is still alive"
    );
    let output = AudioBlockMut::new(&mut more, 128, ChannelLayout::Mono).expect("shaped");
    scheduler
        .render(&mut renderer, &mut first, output)
        .expect("the adopted arbiter still works");
}

#[test]
fn a_faulted_epoch_is_not_faulted_a_second_time() {
    // `publication_faults` says the stream ended once and why. A scheduler that kept
    // publishing into a dead epoch would increment it on every retry, and "at most one per
    // epoch" would be prose rather than a property — an independent reread found exactly
    // that, because the existing tests retried through `PreparedRenderer::render`, which has
    // its own guard, and never through the scheduler.
    let plan = common::admit(&gated_constant(), common::profile(256, ChannelLayout::Mono));
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [compiled_note(&plan, 1, true)];
    let mut scheduler = CompiledEventScheduler::prepare(&mut control, &admitted(&plan, &events))
        .expect("the schedule is valid");
    let mut undersized = arbiter(64);

    let mut samples = [9.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped");
    scheduler
        .render(&mut renderer, &mut undersized, output)
        .expect_err("the window is past the prepared store");
    assert_eq!(renderer.diagnostics().publication_faults(), 1);

    // The same call again, through the scheduler rather than the renderer.
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped");
    let error = scheduler
        .render(&mut renderer, &mut undersized, output)
        .expect_err("a dead epoch renders nothing");
    assert!(
        matches!(
            error,
            ScheduledRenderError::Render(RenderError::NeedsReprepare)
        ),
        "a retry is a dead epoch, not a fresh contract violation: {error:?}"
    );
    assert_eq!(
        renderer.diagnostics().publication_faults(),
        1,
        "the counter says the stream ended once"
    );
    assert!(samples.iter().all(|sample| *sample == 0.0));
}
