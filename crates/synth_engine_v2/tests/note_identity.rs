//! ADR-0047's contract at the seam: what a release names, and what happens when it names
//! nothing.
//!
//! [ADR-0047](../../plans/v2/decisions/ADR-0047-note-identity-in-the-event-contract.md)
//! makes a note-on carry a node and an occurrence while a release carries the occurrence
//! **alone** — `SOUND-INV-017`. `src/tests/identity.rs` already covers the table's own
//! arithmetic. What is untested there is the wiring: that the renderer resolves a release
//! through the occurrence rather than through anything it kept on the side, and that the
//! three orphan cases ADR-0047 clause 4 admits move no node at all.
//!
//! Every check below is written so that the obvious wrong implementation *fails* it. Two
//! nodes at deliberately unequal levels are what make "the release ended the other note"
//! observable; with equal levels a release resolved to the wrong node renders the same
//! sum and every assertion here would pass.

mod common;

use common::{OUTPUT, SOURCE, profile};
use synth_engine_v2::diagnostics::CompileError;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::publish::PublicationArbiter;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, EventCount, HeldNoteCount, NormalizedLevel, Seconds,
};
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::schedule::{
    AdmittedCompiledStream, CompiledEvent, CompiledEventScheduler, CompiledPayload, PlanEvent,
    SchedulePrepareError, ScheduledRenderError,
};
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const Q: u64 = QUANTUM_FRAMES as u64;
/// One quantum as a buffer length.
const QF: usize = QUANTUM_FRAMES as usize;

const FAST_ENVELOPE: NodeId = NodeId::new(11);
const FAST_AMPLIFIER: NodeId = NodeId::new(12);
const SLOW_ENVELOPE: NodeId = NodeId::new(13);
const SLOW_AMPLIFIER: NodeId = NodeId::new(14);

/// How long the slow envelope takes to fall silent, in frames at the fixture's rate.
///
/// The two envelopes are told apart by the **shape** of what their release does, not by a
/// level: they are chained in series, so either release drives the product to silence and
/// two different sustain levels would be indistinguishable. An instantaneous release
/// reaches exactly `0.0` on its own sample; a slow one is still above zero there and stays
/// above zero for frames afterwards. That is the pair of observations every check below
/// turns on, and it survives whatever ramp shape ADR-0042 gives the segment.
fn slow_release() -> Seconds {
    Seconds::new(0.01).expect("not negative")
}

/// Two gates in series over one constant: `out = 1.0 * fast_gate * slow_gate`.
///
/// Series rather than a sum because the IR has no mixer — `Output` refuses fan-in — and
/// because series is enough: what distinguishes the two notes here is which release shape
/// the output takes, and that is visible through a product.
fn two_voices() -> GraphIr {
    two_voices_declaring(common::compiled_notes(16))
}

/// The same graph under caller-chosen declarations, for the two refusals that turn on them.
fn two_voices_declaring(declarations: PlanDeclarations) -> GraphIr {
    let envelope = |release: Seconds| IrNodeKind::Envelope {
        attack: Seconds::new(0.0).expect("not negative"),
        decay: Seconds::new(0.0).expect("not negative"),
        sustain: NormalizedLevel::FULL,
        release,
    };
    GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FAST_ENVELOPE,
            envelope(Seconds::new(0.0).expect("not negative")),
            ExecutionScope::Voice,
        )
        .node(FAST_AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        // The slow chain is in **another scope**, and it has to be: `SOUND-INV-021` binds a
        // note's magnitudes by execution scope, so two playable nodes sharing one scope
        // would each move the other's velocity, and admission refuses that plan. The scopes
        // are otherwise inert here — nothing in this file plays a magnitude — so this is
        // what keeps the fixture two independently addressable notes.
        .node(
            SLOW_ENVELOPE,
            envelope(slow_release()),
            ExecutionScope::InstrumentInstance,
        )
        .node(
            SLOW_AMPLIFIER,
            IrNodeKind::Amplifier,
            ExecutionScope::InstrumentInstance,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FAST_AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FAST_ENVELOPE, PortId::FIRST),
            (FAST_AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (FAST_AMPLIFIER, PortId::FIRST),
            (SLOW_AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SLOW_ENVELOPE, PortId::FIRST),
            (SLOW_AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (SLOW_AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(declarations)
        .build()
        .expect("a readable plan")
}

fn edge(plan: &CompiledPlan, node: NodeId, at: u64, on: bool) -> OfflineEvent {
    let slot = plan.resolve_note(node).expect("an envelope can be played");
    let payload = if on {
        common::note_on(slot)
    } else {
        CompiledPayload::NoteOff {
            slot,
            key: common::any_key(),
        }
    };
    OfflineEvent::new(SampleTime::new(at), payload)
}

/// Silence from `at` onward, and nothing before it, as the fast release produces.
fn is_instant_silence(rendered: &[f32], at: usize) -> bool {
    rendered[at..].iter().all(|sample| *sample == 0.0) && rendered[at - 1] != 0.0
}

/// Still sounding on the release frame and for a while after, as the slow release produces.
///
/// The level on the release frame itself is the sustain — the segment starts there and
/// falls from it — so what separates this from the instant release is that the frame is not
/// zero, that later frames are still not zero, and that the level has moved down by then.
fn is_gradual_fall(rendered: &[f32], at: usize) -> bool {
    rendered[at] != 0.0 && rendered[at + 8] != 0.0 && rendered[at + 8] < rendered[at]
}

#[test]
fn a_release_resolves_through_its_occurrence_to_the_note_it_opened() {
    // Both notes open; the **fast** one is released. Its release is instantaneous, so the
    // output must reach exactly zero on that sample. A release resolved to the slow note
    // instead renders a ramp there — still sounding on the release frame and for hundreds
    // of frames after — which is why the two shapes rather than two levels are what this
    // asserts. The slow note is the one released *last*, so a resolver that pairs by
    // recency picks it.
    const FAST_ON: u64 = Q + 5;
    const SLOW_ON: u64 = Q + 40;
    const OFF: u64 = 3 * Q + 11;

    let plan = common::admit(&two_voices(), profile(256, ChannelLayout::Mono));
    let events = [
        edge(&plan, FAST_ENVELOPE, FAST_ON, true),
        edge(&plan, SLOW_ENVELOPE, SLOW_ON, true),
        edge(&plan, FAST_ENVELOPE, OFF, false),
    ];
    let rendered =
        render_offline(plan, FrameCount::new(1_024), PlanPosition::ZERO, &events).expect("renders");

    assert!(
        rendered[..(SLOW_ON as usize)].iter().all(|s| *s == 0.0),
        "the chain is silent until both gates are up: it is a product, not a sum"
    );
    assert_eq!(
        rendered[SLOW_ON as usize], 1.0,
        "with both gates up the constant passes through untouched"
    );
    assert!(
        is_instant_silence(&rendered, OFF as usize),
        "the fast note was released, so the output falls to zero on that very sample; a \
         ramp here means the release ended the slow note instead: {:?}",
        &rendered[(OFF as usize - 1)..(OFF as usize + 4)]
    );
}

#[test]
fn the_same_list_with_the_other_release_renders_the_other_shape() {
    // The control arm for the check above, and the reason it is falsifiable: the identical
    // list with the *other* node released must render the other shape. Without this, an
    // implementation that always resolved a release to the fast note would pass the first
    // test and be wrong.
    const FAST_ON: u64 = Q + 5;
    const SLOW_ON: u64 = Q + 40;
    const OFF: u64 = 3 * Q + 11;

    let plan = common::admit(&two_voices(), profile(256, ChannelLayout::Mono));
    let events = [
        edge(&plan, FAST_ENVELOPE, FAST_ON, true),
        edge(&plan, SLOW_ENVELOPE, SLOW_ON, true),
        edge(&plan, SLOW_ENVELOPE, OFF, false),
    ];
    let rendered =
        render_offline(plan, FrameCount::new(1_024), PlanPosition::ZERO, &events).expect("renders");

    assert!(
        is_gradual_fall(&rendered, OFF as usize),
        "the slow note was released, so the output falls over its release time rather than \
         reaching zero on the sample: {:?}",
        &rendered[(OFF as usize - 1)..(OFF as usize + 4)]
    );
}

#[test]
fn releases_interleaved_across_two_nodes_each_end_their_own_note() {
    // The slow note opens **first** and is released first, so the first release names the
    // *older* of the two outstanding notes and a resolver that pairs by recency ends the
    // fast one instead — which would cut the chain to silence on that sample rather than
    // starting a ramp. The fast release then cuts the ramp mid-fall, so both releases are
    // observed and in order.
    const SLOW_ON: u64 = 10;
    const FAST_ON: u64 = 20;
    const SLOW_OFF: u64 = 2 * Q;
    const FAST_OFF: u64 = SLOW_OFF + 32;

    let plan = common::admit(&two_voices(), profile(256, ChannelLayout::Mono));
    let events = [
        edge(&plan, SLOW_ENVELOPE, SLOW_ON, true),
        edge(&plan, FAST_ENVELOPE, FAST_ON, true),
        edge(&plan, SLOW_ENVELOPE, SLOW_OFF, false),
        edge(&plan, FAST_ENVELOPE, FAST_OFF, false),
    ];
    let rendered =
        render_offline(plan, FrameCount::new(1_024), PlanPosition::ZERO, &events).expect("renders");

    let fall = &rendered[(SLOW_OFF as usize)..(FAST_OFF as usize)];
    assert!(
        fall.iter().all(|sample| *sample > 0.0),
        "the slow release is a ramp, so the whole span before the fast release still \
         sounds: {fall:?}"
    );
    assert!(
        fall.windows(2).all(|pair| pair[1] < pair[0]),
        "and it is falling throughout, which is what says the *slow* note was the one \
         released: {fall:?}"
    );
    assert!(
        rendered[(FAST_OFF as usize)..]
            .iter()
            .all(|sample| *sample == 0.0),
        "the fast release then cuts the ramp on its own sample"
    );
}

#[test]
fn a_release_replayed_after_its_note_ended_moves_no_node() {
    // ADR-0047 clause 4's first orphan case: the index is free, so the occurrence names no
    // live note. The replayed release still *names the fast node* — that is what a bare
    // slot carried — and by the time it is replayed that node has a new note on it. An
    // implementation that resolved a release by node would cut the chain to silence here;
    // one that resolves through the occurrence leaves it sounding.
    let plan = common::admit(&two_voices(), profile(256, ChannelLayout::Mono));
    let (mut control, mut renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();
    let fast = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");
    let slow = renderer
        .plan()
        .resolve_note(SLOW_ENVELOPE)
        .expect("an envelope can be played");

    // Quantum 0: the fast note opens and closes. Its index is free afterwards.
    let first = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[
            CompiledEvent::new(SampleTime::new(0), common::note_on(fast)),
            CompiledEvent::new(
                SampleTime::new(1),
                CompiledPayload::NoteOff {
                    slot: fast,
                    key: common::any_key(),
                },
            ),
        ],
    )
    .expect("the plan declares a compiled note producer");
    let orphaned = first[1].payload();

    // ADR-0001 clause 11: the output carry is primed with `Q` frames of silence, so a call
    // has to ask for `2Q` frames before it renders a quantum at all. Asking for `Q` would
    // serve the carry and render nothing, and the events would fall outside the call span.
    let mut samples = vec![0.0_f32; 2 * QF];
    let block =
        AudioBlockMut::new(&mut samples, 2 * QF, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(block, TimedEvents::new(&first))
        .expect("the first quantum renders");

    // Quantum 1: both notes open, and the spent release is replayed on time alongside them.
    let opened = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[
            CompiledEvent::new(SampleTime::new(Q), common::note_on(fast)),
            CompiledEvent::new(SampleTime::new(Q), common::note_on(slow)),
        ],
    )
    .expect("the plan declares a compiled note producer");
    // Restamped at this quantum so the late clamp is not what this test measures.
    let replayed = synth_engine_v2::render::TimedEvent::new(
        synth_engine_v2::render::EventEnvelope::new(
            epoch,
            SampleTime::new(Q),
            synth_engine_v2::time::TimeSource::Compiled,
        ),
        orphaned,
    );
    let events = [opened[0], opened[1], replayed];

    let mut samples = vec![0.0_f32; QF];
    let block = AudioBlockMut::new(&mut samples, QF, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(block, TimedEvents::new(&events))
        .expect("an orphan release is not a render failure");

    assert!(
        samples.iter().all(|sample| *sample == 1.0),
        "both gates stay up for the whole quantum; the replayed release names a spent \
         occurrence and must move nothing: {:?}",
        &samples[..8]
    );
    assert_eq!(
        renderer.diagnostics().foreign_slot_events(),
        0,
        "an orphan from this table is not foreign — ADR-0047 clause 4 distinguishes them"
    );
}

#[test]
fn an_occurrence_from_another_plan_is_refused_rather_than_applied() {
    // Two renderers, two identity tables. A stamped release carries no node at all, so the
    // only thing that can tell the renderer this event is not its own is the occurrence's
    // table — which is why the foreign filter compares tables and not slots.
    let host = profile(256, ChannelLayout::Mono);
    let (_mine_control, mut mine) = StreamControl::open(
        common::admit(&two_voices(), host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let (mut theirs_control, theirs) = StreamControl::open(
        common::admit(&two_voices(), host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");

    let slot = theirs
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");
    let _epoch = theirs.epoch();
    let foreign = synth_engine_v2::schedule::stamp_compiled(
        &mut theirs_control,
        &[CompiledEvent::new(
            SampleTime::new(0),
            common::note_on(slot),
        )],
    )
    .expect("the plan declares a compiled note producer");

    // Restamped onto this renderer's epoch, so the stale-epoch filter cannot be what
    // catches it. The occurrence is the only foreign thing left.
    let restamped = [synth_engine_v2::render::TimedEvent::new(
        synth_engine_v2::render::EventEnvelope::new(
            mine.epoch(),
            SampleTime::new(0),
            synth_engine_v2::time::TimeSource::Compiled,
        ),
        foreign[0].payload(),
    )];

    let mut samples = vec![0.0_f32; QF];
    let block = AudioBlockMut::new(&mut samples, QF, ChannelLayout::Mono).expect("shaped block");
    mine.render(block, TimedEvents::new(&restamped))
        .expect("a foreign occurrence is filtered, not a render failure");

    assert!(
        samples.iter().all(|sample| *sample == 0.0),
        "nothing sounds: the note-on named another table's occurrence"
    );
    assert_eq!(
        mine.diagnostics().foreign_slot_events(),
        1,
        "and it is counted, so the refusal is observable rather than a silent drop"
    );
}

#[test]
fn a_compiled_release_with_no_matching_note_on_is_refused_at_stamping() {
    // Pairing happens at stamping or nowhere: after it, a release carries only an
    // occurrence. A list whose release opens nothing therefore has to be refused here,
    // because no later stage can tell it from an orphan the renderer must ignore.
    let (mut control, renderer) = StreamControl::open(
        common::admit(&two_voices(), profile(256, ChannelLayout::Mono)),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let slot = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");
    let _epoch = renderer.epoch();

    let refused = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[CompiledEvent::new(
            SampleTime::new(0),
            CompiledPayload::NoteOff {
                slot,
                key: common::any_key(),
            },
        )],
    )
    .expect_err("a release that opens nothing is not a pairing this can make");
    assert!(
        matches!(
            refused,
            SchedulePrepareError::UnmatchedRelease { event_index: 0 }
        ),
        "the caller is told which event it was: {refused:?}"
    );
}

#[test]
fn a_plan_that_declares_no_note_producer_cannot_stamp_a_note() {
    // A plan with no declared producer has no admitted identity range, so there is nowhere
    // for an occurrence to come from. Minting one anyway would put it outside the partition
    // every disjointness check relies on.
    let ir = two_voices_declaring(PlanDeclarations::default());
    let (mut control, renderer) = StreamControl::open(
        common::admit(&ir, profile(256, ChannelLayout::Mono)),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let slot = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");
    let _epoch = renderer.epoch();

    let refused = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[CompiledEvent::new(
            SampleTime::new(0),
            common::note_on(slot),
        )],
    )
    .expect_err("no producer, no occurrence");
    assert!(
        matches!(
            refused,
            SchedulePrepareError::NoCompiledNoteProducer { event_index: 0 }
        ),
        "{refused:?}"
    );
}

#[test]
fn a_second_compiled_producer_is_refused_at_admission() {
    // `PlanDeclarations::events_per_quantum` is one figure against one share, and
    // `stamp_compiled` resolves one producer. A second compiled producer would leave both
    // of those a guess, so it is refused where the declaration is read.
    let compiled = NoteProducerDeclaration {
        compiled: true,
        simultaneous_notes: HeldNoteCount::measured(4),
        simultaneous_holds: EventCount::NONE,
    };
    let declarations = PlanDeclarations {
        note_producers: vec![compiled, compiled],
        ..PlanDeclarations::default()
    };
    let ir = two_voices_declaring(declarations);

    let refused = common::refuse(&ir, profile(256, ChannelLayout::Mono));
    assert!(
        matches!(
            refused,
            CompileError::SecondCompiledProducer {
                first: 0,
                second: 1
            }
        ),
        "{refused:?}"
    );
}

#[test]
fn the_compiled_producer_is_looked_up_rather_than_assumed_to_be_the_first() {
    // A plan may declare a runtime source before its compiled one, and then producer 0 is
    // not the compiled producer. Inferring it from position would mint compiled occurrences
    // out of the runtime producer's range: disjointness would still *hold*, so nothing but
    // the range's size can catch it. The runtime producer is given one index and the
    // compiled one eight, so a compiled list of eight note-ons stamps cleanly against the
    // right range and exhausts the wrong one on its second event.
    const COMPILED_RANGE: u32 = 8;
    let declarations = PlanDeclarations {
        note_producers: vec![
            NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(1),
                simultaneous_holds: EventCount::measured(1),
            },
            NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(COMPILED_RANGE),
                simultaneous_holds: EventCount::NONE,
            },
        ],
        ..PlanDeclarations::default()
    };
    let ir = two_voices_declaring(declarations);
    let (mut control, renderer) = StreamControl::open(
        common::admit(&ir, profile(256, ChannelLayout::Mono)),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let slot = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");
    let _epoch = renderer.epoch();

    let opens: Vec<CompiledEvent> = (0..COMPILED_RANGE)
        .map(|index| CompiledEvent::new(SampleTime::new(u64::from(index)), common::note_on(slot)))
        .collect();
    synth_engine_v2::schedule::stamp_compiled(&mut control, &opens)
        .expect("eight simultaneous notes fit the compiled producer's eight admitted indices");

    // And the range really is that producer's eight rather than something unbounded: the
    // ninth has nowhere to go, and the error names the producer it was charged to.
    let refused = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[CompiledEvent::new(
            SampleTime::new(u64::from(COMPILED_RANGE)),
            common::note_on(slot),
        )],
    )
    .expect_err("a ninth simultaneous note is over-emission against an eight-wide range");
    assert!(
        matches!(
            refused,
            SchedulePrepareError::Identity {
                event_index: 0,
                source: synth_engine_v2::identity::IdentityError::ProducerOverEmitted { .. }
            }
        ),
        "{refused:?}"
    );
}

#[test]
fn a_note_slot_from_another_plan_is_refused_at_stamping() {
    // The renderer's foreign filter compares a note edge's **table**, not a node address —
    // `SOUND-INV-017` leaves a release no node to compare. So a foreign node address on a
    // note-on, stamped with *this* table's occurrence, would sail past that filter and index
    // this plan's note targets. Stamping is the last point at which the node is present, and
    // `render_offline` reaches it without going through `CompiledEventScheduler::prepare`,
    // whose own list-wide check would otherwise be the only one.
    let host = profile(256, ChannelLayout::Mono);
    let mine = common::admit(&two_voices(), host);
    let theirs = common::admit(&two_voices(), host);
    let foreign = theirs
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");
    assert_ne!(
        mine.id(),
        theirs.id(),
        "two admissions are two plans, or this test proves nothing"
    );

    let (mut control, renderer) = StreamControl::open(
        mine,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let _epoch = renderer.epoch();
    let refused = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[CompiledEvent::new(
            SampleTime::ZERO,
            common::note_on(foreign),
        )],
    )
    .expect_err("a node address from another plan is not this plan's to play");
    assert!(
        matches!(
            refused,
            SchedulePrepareError::ForeignPlan { event_index: 0, .. }
        ),
        "{refused:?}"
    );
}

#[test]
fn a_producers_range_bounds_its_polyphony_rather_than_a_pieces_note_count() {
    // The defect an independent review found in the first version of this slice: stamping
    // minted every note-on in the list before anything could be released, so a one-note
    // producer could not play two notes in sequence and a whole piece needed a range equal
    // to its total note count. `simultaneous_notes` means *simultaneous*; a release frees
    // its index at stamping, and the sequence below is the smallest case that tells the two
    // readings apart — one note at a time, played eight times over.
    const PLAYS: u64 = 8;
    let declarations = PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(1),
            simultaneous_holds: EventCount::NONE,
        }],
        ..PlanDeclarations::default()
    };
    let ir = two_voices_declaring(declarations);
    let (mut control, renderer) = StreamControl::open(
        common::admit(&ir, profile(256, ChannelLayout::Mono)),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let slot = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");

    let mut events = Vec::new();
    for play in 0..PLAYS {
        events.push(CompiledEvent::new(
            SampleTime::new(play * Q),
            common::note_on(slot),
        ));
        events.push(CompiledEvent::new(
            SampleTime::new(play * Q + 1),
            CompiledPayload::NoteOff {
                slot,
                key: common::any_key(),
            },
        ));
    }
    let stamped = synth_engine_v2::schedule::stamp_compiled(&mut control, &events)
        .expect("one note at a time fits a one-note producer, however many times it is played");
    assert_eq!(stamped.len(), events.len(), "every edge is stamped");

    // Two at once is what the range actually refuses.
    let refused = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[
            CompiledEvent::new(SampleTime::ZERO, common::note_on(slot)),
            CompiledEvent::new(SampleTime::new(1), common::note_on(slot)),
        ],
    )
    .expect_err("two sounding at once is over-emission against a one-note producer");
    assert!(
        matches!(
            refused,
            SchedulePrepareError::Identity {
                event_index: 1,
                source: synth_engine_v2::identity::IdentityError::ProducerOverEmitted { .. }
            }
        ),
        "{refused:?}"
    );

    // And that refusal kept nothing: the first of those two note-ons did mint before the
    // second failed, so a stamping that was not all-or-nothing would have left the range
    // spent and this retry would fail as over-emission with one note in flight.
    synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[
            CompiledEvent::new(SampleTime::ZERO, common::note_on(slot)),
            CompiledEvent::new(
                SampleTime::new(1),
                CompiledPayload::NoteOff {
                    slot,
                    key: common::any_key(),
                },
            ),
        ],
    )
    .expect("a valid list after a mint that failed part-way through");
}

#[test]
fn a_reissued_index_still_resolves_both_of_the_notes_that_used_it() {
    // The consequence of the fix above, and the reason the renderer keeps a registry of its
    // own rather than reading the minting table. With a one-note producer every note reuses
    // index 0, so the minter's state after stamping describes the *last* occurrence only. If
    // the renderer resolved releases through that table, the first note would never end.
    const FIRST_ON: u64 = 4;
    const FIRST_OFF: u64 = 2 * Q;
    const SECOND_ON: u64 = 3 * Q;
    const SECOND_OFF: u64 = 5 * Q;

    // Two indices, both in use: the slow note holds index 0 for the whole render, so the
    // fast note takes index 1, frees it, and takes it again at the next generation. One
    // index would refuse the pair outright; three would let the second note take a fresh
    // index and the test would prove nothing.
    let declarations = PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(2),
            simultaneous_holds: EventCount::NONE,
        }],
        ..PlanDeclarations::default()
    };
    let plan = common::admit(
        &two_voices_declaring(declarations),
        profile(256, ChannelLayout::Mono),
    );
    // The chain is a product, so the slow gate is held up for the whole render and what the
    // output follows is the fast one.
    let events = [
        edge(&plan, SLOW_ENVELOPE, 0, true),
        edge(&plan, FAST_ENVELOPE, FIRST_ON, true),
        edge(&plan, FAST_ENVELOPE, FIRST_OFF, false),
        edge(&plan, FAST_ENVELOPE, SECOND_ON, true),
        edge(&plan, FAST_ENVELOPE, SECOND_OFF, false),
    ];
    let rendered =
        render_offline(plan, FrameCount::new(512), PlanPosition::ZERO, &events).expect("renders");

    for (frame, sample) in rendered.iter().enumerate() {
        let frame = frame as u64;
        let sounding =
            (FIRST_ON..FIRST_OFF).contains(&frame) || (SECOND_ON..SECOND_OFF).contains(&frame);
        let expected = if sounding { 1.0 } else { 0.0 };
        assert_eq!(
            *sample, expected,
            "frame {frame} is {sample} rather than {expected}: both notes reuse index 0, so \
             this is where resolving a release through the minter loses the first one"
        );
    }
}

#[test]
fn a_refused_list_leaves_the_minter_as_it_found_it() {
    // Stamping validates the whole list before it mints anything. Minting as it walked would
    // leave the earlier note-on of `On, Off, Off` reserved for a list that was never
    // returned, and the next attempt — a list that is perfectly valid — would fail as false
    // over-emission. A one-note producer makes that visible in two events.
    let declarations = PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(1),
            simultaneous_holds: EventCount::NONE,
        }],
        ..PlanDeclarations::default()
    };
    let (mut control, renderer) = StreamControl::open(
        common::admit(
            &two_voices_declaring(declarations),
            profile(256, ChannelLayout::Mono),
        ),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let slot = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");

    let refused = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[
            CompiledEvent::new(SampleTime::ZERO, common::note_on(slot)),
            CompiledEvent::new(
                SampleTime::new(1),
                CompiledPayload::NoteOff {
                    slot,
                    key: common::any_key(),
                },
            ),
            CompiledEvent::new(
                SampleTime::new(2),
                CompiledPayload::NoteOff {
                    slot,
                    key: common::any_key(),
                },
            ),
        ],
    )
    .expect_err("the third event releases a node with nothing sounding");
    assert!(
        matches!(
            refused,
            SchedulePrepareError::UnmatchedRelease { event_index: 2 }
        ),
        "{refused:?}"
    );

    // The retry is the falsifier: it needs the index the refused list would have kept.
    synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[
            CompiledEvent::new(SampleTime::ZERO, common::note_on(slot)),
            CompiledEvent::new(
                SampleTime::new(1),
                CompiledPayload::NoteOff {
                    slot,
                    key: common::any_key(),
                },
            ),
        ],
    )
    .expect("a valid list after a refused one, which a partial mint would have starved");
}

#[test]
fn an_orphan_names_the_occurrence_it_refused() {
    // ADR-0047 clause 4: an orphan is counted "against its offering producer with the
    // identity named". Naming the identity names the producer — the ranges are disjoint and a
    // producer's position in the declaration is its `ProducerId`, so the index falls in
    // exactly one range. A bare count would say a release was refused and nothing about which.
    let plan = common::admit(&two_voices(), profile(256, ChannelLayout::Mono));
    let (mut control, mut renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();
    let slot = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");

    let first = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[
            CompiledEvent::new(SampleTime::ZERO, common::note_on(slot)),
            CompiledEvent::new(
                SampleTime::new(1),
                CompiledPayload::NoteOff {
                    slot,
                    key: common::any_key(),
                },
            ),
        ],
    )
    .expect("the plan declares a compiled note producer");
    let spent = first[1].payload();
    let synth_engine_v2::render::EventPayload::Note { identity, .. } = spent else {
        panic!("a stamped note edge is a note payload");
    };

    let mut samples = vec![0.0_f32; 2 * QF];
    let block =
        AudioBlockMut::new(&mut samples, 2 * QF, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(block, TimedEvents::new(&first))
        .expect("the first quantum renders");
    assert_eq!(
        renderer.diagnostics().last_orphan_note(),
        None,
        "nothing has been refused yet"
    );

    let replayed = [synth_engine_v2::render::TimedEvent::new(
        synth_engine_v2::render::EventEnvelope::new(
            epoch,
            SampleTime::new(Q),
            synth_engine_v2::time::TimeSource::Compiled,
        ),
        spent,
    )];
    let mut samples = vec![0.0_f32; QF];
    let block = AudioBlockMut::new(&mut samples, QF, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(block, TimedEvents::new(&replayed))
        .expect("an orphan release is not a render failure");

    assert_eq!(
        renderer.diagnostics().last_orphan_note(),
        Some(identity),
        "the report names the occurrence that was refused, not merely that one was"
    );
}

#[test]
fn an_orphan_release_is_counted_rather_than_silently_skipped() {
    // `SOUND-INV-017` requires an identity naming no live note to be **refused and counted**.
    // Refusing alone is not enough: a producer replaying spent releases would look like a
    // producer sending nothing, and nothing in the report would say otherwise. The orphan is
    // deliberately not a drop — `HOST-INV-009` licenses drops for a shortage, and this is a
    // release for a note that does not exist.
    let plan = common::admit(&two_voices(), profile(256, ChannelLayout::Mono));
    let (mut control, mut renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();
    let slot = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");

    let first = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[
            CompiledEvent::new(SampleTime::ZERO, common::note_on(slot)),
            CompiledEvent::new(
                SampleTime::new(1),
                CompiledPayload::NoteOff {
                    slot,
                    key: common::any_key(),
                },
            ),
        ],
    )
    .expect("the plan declares a compiled note producer");
    let spent = first[1].payload();

    let mut samples = vec![0.0_f32; 2 * QF];
    let block =
        AudioBlockMut::new(&mut samples, 2 * QF, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(block, TimedEvents::new(&first))
        .expect("the first quantum renders");
    assert_eq!(
        renderer.diagnostics().orphan_note_events(),
        0,
        "a release that ends a live note is not an orphan"
    );

    let replayed = [synth_engine_v2::render::TimedEvent::new(
        synth_engine_v2::render::EventEnvelope::new(
            epoch,
            SampleTime::new(Q),
            synth_engine_v2::time::TimeSource::Compiled,
        ),
        spent,
    )];
    let mut samples = vec![0.0_f32; QF];
    let block = AudioBlockMut::new(&mut samples, QF, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(block, TimedEvents::new(&replayed))
        .expect("an orphan release is not a render failure");

    assert_eq!(
        renderer.diagnostics().orphan_note_events(),
        1,
        "the spent release is counted where it is refused"
    );
    assert_eq!(
        renderer.diagnostics().foreign_slot_events(),
        0,
        "and it is not reported as an event from another plan, which it is not"
    );
}

#[test]
fn stamping_uses_the_streams_own_epoch_and_a_foreign_renderer_refuses_the_schedule() {
    // Taking the epoch from the caller would let a list be stamped against another stream's:
    // it would succeed, reserve this producer's occurrences, and then be discarded event by
    // event as stale — a producer's whole range spent on a render that never happens. The
    // signature no longer admits the mistake.
    //
    // The second half is what an earlier version of this test could not establish. Since
    // ADR-0050 clause 9 the two halves of a stream are separate values, so a caller **can**
    // pair a control with another stream's renderer. That pairing is refused where it becomes
    // wrong rather than at preparation: the stamp carries the control's epoch, and the
    // renderer refuses the whole schedule instead of silently discarding every event as
    // stale, which is the difference between a diagnosable error and a silent nothing.
    let plan = common::admit(&two_voices(), profile(256, ChannelLayout::Mono));
    let (mut control, renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let slot = renderer
        .plan()
        .resolve_note(FAST_ENVELOPE)
        .expect("an envelope can be played");
    let epoch = renderer.epoch();

    let stamped = synth_engine_v2::schedule::stamp_compiled(
        &mut control,
        &[CompiledEvent::new(SampleTime::ZERO, common::note_on(slot))],
    )
    .expect("the plan declares a compiled note producer");
    assert_eq!(
        stamped[0].envelope().epoch(),
        epoch,
        "the stamp carries this stream's epoch and no other"
    );

    // A second stream over the same plan. Its renderer is a legitimate value and the pairing
    // compiles, which is exactly why the refusal has to exist at render.
    let (other_control, mut other_renderer) = StreamControl::open(
        renderer.plan().clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("a second stream over the same plan prepares");
    assert_ne!(
        other_renderer.epoch(),
        epoch,
        "two streams never share an epoch"
    );
    let stream = AdmittedCompiledStream::admit(
        other_control.plan(),
        &[PlanEvent::new(PlanPosition::ZERO, common::note_on(slot))],
    )
    .expect("one note fits the compiled share");
    let mut schedule = CompiledEventScheduler::prepare(&mut control, &stream)
        .expect("the first control prepares its own schedule");

    let mut arbiter = PublicationArbiter::prepare(&profile(256, ChannelLayout::Mono))
        .expect("the publication store is preparable");
    let mut block = vec![0.0_f32; 64];
    let output =
        AudioBlockMut::new(&mut block, 64, ChannelLayout::Mono).expect("a well-shaped block");
    assert_eq!(
        schedule
            .render(&mut other_renderer, &mut arbiter, output)
            .expect_err("a schedule from another stream is refused"),
        ScheduledRenderError::EpochMismatch {
            schedule: epoch,
            renderer: other_renderer.epoch(),
        },
        "the whole schedule is refused rather than each of its events discarded as stale"
    );
}
