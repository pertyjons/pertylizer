#![cfg(feature = "simulated-ingress")]
//! Phase 3's exit gate: a simulated ingress producer and the compiled path reach the same
//! samples.
//!
//! > Equivalent simulated-ingress and precompiled events carrying the same engine-epoch
//! > `SampleTime` reach the same sample offsets. This proves the scheduler boundary, not a
//! > physical adapter's hardware-clock mapping; ADR-0022 and Phase 9 own that later claim.
//!
//! # The fixture uses note edges, and that is not a style choice
//!
//! An earlier attempt at this gate moved a **sine's frequency** and displacing every
//! ingress event by one frame did not fail it. A frequency is control-rate, so ADR-0001
//! clause 14 makes it take effect at the next quantum boundary either way. Only a
//! sample-positioned payload — a note edge — makes a one-frame error observable, so a
//! placement test built on a control-rate parameter measures nothing.
//!
//! # One producer per plan
//!
//! ADR-0051 clause 6 leaves a scalar gate reached by more than one producer with no
//! ownership law, and the rule forbids two producers **emitting** onto a gate rather than
//! only both sounding at once. The boundary is a check rather than a convention:
//! `PerformanceIngress::prepare` refuses a plan that also declares a compiled producer, so
//! no plan can carry a live **store** beside one. Declaring both is a different thing and is
//! not forbidden — one fixture below builds such a plan precisely to assert the refusal, and
//! plans declaring both already exist elsewhere in this crate's fixtures, harmless because
//! none builds a store. Nothing here needs a mixed plan anyway: the exit gate asks whether
//! two paths reach the same offsets, which is a comparison between renders rather than
//! inside one.

mod common;

use synth_engine_v2::identity::ProducerId;
use synth_engine_v2::ingress::{
    ExhaustedResource, IngressCounters, IngressRefused, PerformanceIngress,
};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain, parameters,
};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::publish::{ProducerClass, PublicationArbiter};
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, EventCount, HeldNoteCount, NormalizedLevel, Seconds,
};
use synth_engine_v2::render::{AudioBlockMut, PreparedRenderer};
use synth_engine_v2::schedule::{
    AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent,
};
use synth_engine_v2::stream::{ActivationBuildError, ActivationRequest, StreamControl};
use synth_engine_v2::time::{PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor, TimeSource};

const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);
const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);

const Q: u64 = QUANTUM_FRAMES as u64;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
const TOTAL_FRAMES: usize = 4_096;

/// Deliberately not on a quantum boundary, in either edge.
///
/// A boundary-aligned pair would agree between the two paths even if one of them rounded to
/// a quantum, which is the error the gate exists to catch.
const ON: u64 = 2 * Q + 17;
const OFF: u64 = 40 * Q + 3;

const WHOLE: [usize; 1] = [4_096];
const BLOCKS_256: [usize; 16] = [256; 16];
const BLOCKS_64: [usize; 64] = [64; 64];
const IRREGULAR: [usize; 10] = [17, 511, 3, 64, 1_024, 1, 700, 256, 63, 1_457];

/// The producer position every plan a fixture **renders** declares, alone.
///
/// One mixed plan exists below and is never rendered: it asserts that a live store refuses
/// to join a plan that also declares a compiled producer.
const ONLY_PRODUCER: ProducerId = ProducerId::new(0);

/// A constant through a gate, so a note edge is audible as a step in the output.
fn gated_constant(declarations: PlanDeclarations) -> GraphIr {
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
                velocity_sensitivity: synth_engine_v2::quantities::NormalizedLevel::FULL,
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
        .declaring(declarations)
        .build()
        .expect("a readable plan")
}

/// A plan whose only note producer is the compiled one.
fn compiled_only() -> PlanDeclarations {
    PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(4),
            // ADR-0046 clause 6: a compiled release uses the plan entitlement clause 4
            // established, so a compiled producer declares no hold.
            simultaneous_holds: EventCount::NONE,
        }],
        ..PlanDeclarations::default()
    }
}

/// A plan whose only note producer is live.
///
/// **Deliberately a different plan from [`compiled_only`], not a second producer inside
/// one.** ADR-0051 clause 6 leaves a scalar gate reached by two producers with no ownership
/// law: ending either producer's note writes `ZERO` to the gate they share and cuts the
/// other's with it, and the rule forbids more than one producer *emitting* onto a gate
/// rather than only both sounding at once. So no plan that a fixture here renders may carry
/// a live store beside a compiled producer — which `PerformanceIngress::prepare` refuses,
/// and which one fixture below asserts by building exactly such a plan and being refused.
///
/// **What that costs is recorded rather than absorbed**: `IdentityTable::release_for`
/// refuses an occurrence outside the offering producer's range, and the fixture that would
/// prove it end to end needs a mixed plan. Its unit-level check lives in
/// `src/tests/identity.rs`, over a bare table with two producer ranges, which reaches no
/// gate and needs no plan.
///
/// Nothing is lost: the exit gate compares two renders, so one producer per plan is all it
/// ever needed. The identity range and the hold entitlement are four so the boundary
/// fixtures below can exhaust each independently.
fn live_only(notes: u32, holds: u32) -> PlanDeclarations {
    PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(notes),
            simultaneous_holds: EventCount::measured(holds),
        }],
        ..PlanDeclarations::default()
    }
}

fn plan_with(declarations: PlanDeclarations) -> CompiledPlan {
    common::admit(
        &gated_constant(declarations),
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
    )
}

/// The live-producer plan every boundary fixture uses.
fn plan() -> CompiledPlan {
    plan_with(live_only(4, 4))
}

fn arbiter() -> PublicationArbiter {
    PublicationArbiter::prepare(&common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono))
        .expect("the publication store is preparable")
}

fn compiled_note(plan: &CompiledPlan, time: u64, on: bool) -> PlanEvent {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope accepts note edges");
    let payload = if on {
        common::note_on(slot)
    } else {
        CompiledPayload::NoteOff {
            slot,
            key: common::any_key(),
        }
    };
    PlanEvent::new(PlanPosition::new(time), payload)
}

/// Render the note through the compiled path.
fn render_compiled(partition: &[usize]) -> Vec<f32> {
    let plan = plan_with(compiled_only());
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let events = [
        compiled_note(&plan, ON, true),
        compiled_note(&plan, OFF, false),
    ];
    let admitted =
        AdmittedCompiledStream::admit(&plan, &events).expect("the compiled stream fits its share");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &admitted).expect("the schedule is valid");
    let mut publication = arbiter();
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
    rendered
}

/// Render the same note through the simulated ingress producer.
///
/// The compiled stream is empty, so the scheduler contributes nothing and every edge in the
/// output came through the live boundary. That is what makes the comparison a comparison of
/// two producers rather than of one producer against itself.
fn render_ingress(partition: &[usize]) -> (Vec<f32>, PreparedRenderer, PublicationArbiter) {
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let mut publication = arbiter();
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope accepts note edges");
    // Through the control, because the off-thread half owns the minter: an identity from
    // any other table is one this renderer refuses as foreign.
    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the note-on takes a slot, a hold and an identity");
    control
        .offer_note_off(&mut store, SampleTime::new(OFF), identity)
        .expect("the release spends the hold its note-on reserved");

    let mut rendered = Vec::with_capacity(TOTAL_FRAMES);
    for &frames in partition {
        let mut block = vec![0.0_f32; frames];
        let output = AudioBlockMut::new(&mut block, frames, ChannelLayout::Mono)
            .expect("the output block is shaped correctly");
        scheduler
            .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
            .expect("the pass publishes both producers");
        rendered.extend_from_slice(&block);
    }
    assert!(store.is_empty(), "every offered edge was drained");
    assert_eq!(
        store.counters().dropped(),
        0,
        "nothing was dropped at the live boundary"
    );
    (rendered, renderer, publication)
}

/// One note's edges, as a live producer offers them and as a compiled stream states them.
#[derive(Clone, Copy)]
struct Edge {
    at: u64,
    key: u8,
    velocity: f32,
    /// `Some(n)`: the release of the `n`th note-on offered, by order; `None`: a note-on.
    releases: Option<usize>,
    /// With `releases`, a bend of that note by these cents rather than its release.
    cents: Option<f32>,
}

const fn on(at: u64, key: u8, velocity: f32) -> Edge {
    Edge {
        at,
        key,
        velocity,
        releases: None,
        cents: None,
    }
}

const fn off(at: u64, of: usize) -> Edge {
    Edge {
        at,
        key: 0,
        velocity: 0.0,
        releases: Some(of),
        cents: None,
    }
}

const fn bend(at: u64, of: usize, cents: f32) -> Edge {
    Edge {
        at,
        key: 0,
        velocity: 0.0,
        releases: Some(of),
        cents: Some(cents),
    }
}

/// A voice with a pitch destination — a sine through the envelope into the amplifier, tuned —
/// where the gated constant has none, so a bend has something to move.
fn pitched(declarations: PlanDeclarations) -> CompiledPlan {
    pitched_with(declarations, common::twelve_tet())
}

/// The pitched voice under a stated tuning (`P06-S006`): the live boundary carries a key,
/// and which frequency that is belongs to the scope's prepared tuning on this path as on
/// the compiled one.
fn pitched_with(
    declarations: PlanDeclarations,
    tuning: synth_engine_v2::tuning::PreparedTuning,
) -> CompiledPlan {
    const OSCILLATOR: NodeId = NodeId::new(31);
    let ir = GraphIr::builder()
        .node(
            OSCILLATOR,
            IrNodeKind::Sine {
                frequency: synth_engine_v2::quantities::Frequency::new(220.0).expect("finite"),
                amplitude: synth_engine_v2::quantities::Amplitude::new(0.25).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: synth_engine_v2::quantities::Seconds::ZERO,
                decay: synth_engine_v2::quantities::Seconds::ZERO,
                sustain: synth_engine_v2::quantities::NormalizedLevel::FULL,
                release: synth_engine_v2::quantities::Seconds::ZERO,
                velocity_sensitivity: synth_engine_v2::quantities::NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
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
        .tuning(ExecutionScope::Voice, tuning)
        .declaring(declarations)
        .build()
        .expect("a readable plan");
    common::admit(
        &ir,
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
    )
}

#[test]
fn a_live_note_under_nineteen_tet_reaches_the_same_samples_as_the_compiled_one() {
    // `P06-S006`: the live boundary offers a key and never a frequency, so the tuning the
    // plan states is the one both paths resolve through. Under nineteen-tone equal
    // temperament the live and the compiled renders of one key are the same samples, and
    // both differ from the twelve-tone render of that key — the second assertion is what
    // makes the first say something, since two paths ignoring the tuning would also agree.
    let plain = synth_engine_v2::ir::StealingPolicy::None;
    let edges = [on(0, 72, 1.0), off(20 * Q, 0)];
    let compiled = render_compiled_edges_on(
        &pitched_with(
            stealing(compiled_only_notes(2), plain),
            common::nineteen_tet(),
        ),
        &edges,
        24,
    );
    let (live, counters, renderer) = render_live_edges_on(
        &pitched_with(stealing(live_only(2, 2), plain), common::nineteen_tet()),
        &edges,
        24,
    );
    assert_same(
        &live,
        &compiled,
        "the live note under nineteen-tone did not render as the compiled one",
    );
    assert_eq!(counters.orphan_releases(), 0);
    assert_eq!(renderer.diagnostics().orphan_note_events(), 0);
    let twelve = render_compiled_edges_on(
        &pitched(stealing(compiled_only_notes(2), plain)),
        &edges,
        24,
    );
    assert!(
        live.iter().zip(&twelve).any(|(a, b)| a != b),
        "nineteen-tone rendered the twelve-tone samples, so the tuning reached neither path"
    );
}

/// The compiled path's render of `edges` under `declarations`, `quanta` quanta long.
fn render_compiled_edges(declarations: PlanDeclarations, edges: &[Edge], quanta: u64) -> Vec<f32> {
    render_compiled_edges_on(&plan_with(declarations), edges, quanta)
}

/// [`render_compiled_edges`] on a given plan.
fn render_compiled_edges_on(plan: &CompiledPlan, edges: &[Edge], quanta: u64) -> Vec<f32> {
    let plan = plan.clone();
    let slot = plan.resolve_note(ENVELOPE).expect("playable");
    let mut keys: Vec<u8> = Vec::new();
    let mut events: Vec<PlanEvent> = Vec::new();
    for edge in edges {
        let payload = match edge.releases {
            None => {
                keys.push(edge.key);
                CompiledPayload::NoteOn {
                    slot,
                    key: synth_engine_v2::quantities::KeyIdentity::new(edge.key).expect("key"),
                    velocity: synth_engine_v2::quantities::NoteVelocity::saturating(edge.velocity),
                }
            }
            Some(of) => match edge.cents {
                Some(cents) => CompiledPayload::Bend {
                    slot,
                    key: synth_engine_v2::quantities::KeyIdentity::new(keys[of]).expect("key"),
                    cents: synth_engine_v2::quantities::Cents::new(cents).expect("a bend"),
                },
                None => CompiledPayload::NoteOff {
                    slot,
                    key: synth_engine_v2::quantities::KeyIdentity::new(keys[of]).expect("key"),
                },
            },
        };
        events.push(PlanEvent::new(PlanPosition::new(edge.at), payload));
    }
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let admitted = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &admitted).expect("the schedule is valid");
    let mut publication = arbiter();
    let mut rendered = Vec::new();
    let mut done = 0_u64;
    while done < quanta * Q {
        let frames = 256.min((quanta * Q - done) as usize);
        let mut block = vec![0.0_f32; frames];
        let output = AudioBlockMut::new(&mut block, frames, ChannelLayout::Mono).expect("shaped");
        scheduler
            .render(&mut renderer, &mut publication, output)
            .expect("renders");
        rendered.extend_from_slice(&block);
        done += frames as u64;
    }
    rendered
}

/// The live path's render of the same edges, each offered once the render clock has reached
/// its quantum — as a live producer's offers arrive — plus the store's counters and the
/// renderer's report.
///
/// Offered live rather than all in advance because the boundary's steal is asynchronous in
/// one respect the compiled path is not: a voice a steal took becomes takeable again once the
/// drain has published its deferred start, and the drain runs with the render. Offering
/// everything before any render would find every taken voice still waiting.
fn render_live_edges(
    declarations: PlanDeclarations,
    edges: &[Edge],
    quanta: u64,
) -> (Vec<f32>, IngressCounters, PreparedRenderer) {
    render_live_edges_on(&plan_with(declarations), edges, quanta)
}

/// [`render_live_edges`] on a given plan.
fn render_live_edges_on(
    plan: &CompiledPlan,
    edges: &[Edge],
    quanta: u64,
) -> (Vec<f32>, IngressCounters, PreparedRenderer) {
    let plan = plan.clone();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let mut publication = arbiter();
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let slot = plan.resolve_note(ENVELOPE).expect("playable");
    let mut identities = Vec::new();
    let mut rendered = Vec::new();
    let mut done = 0_u64;
    let total = quanta * Q;
    let mut render_block = |scheduler: &mut CompiledEventScheduler,
                            renderer: &mut PreparedRenderer,
                            store: &mut PerformanceIngress,
                            rendered: &mut Vec<f32>,
                            done: &mut u64| {
        let frames = (Q as usize).min((total - *done) as usize);
        let mut block = vec![0.0_f32; frames];
        let output = AudioBlockMut::new(&mut block, frames, ChannelLayout::Mono).expect("shaped");
        scheduler
            .render_with_ingress(renderer, &mut publication, Some(store), output)
            .expect("the pass publishes");
        rendered.extend_from_slice(&block);
        *done += frames as u64;
    };
    for edge in edges {
        // The render clock reaches the edge's quantum before the edge is offered.
        while done < total && done + Q <= edge.at {
            render_block(
                &mut scheduler,
                &mut renderer,
                &mut store,
                &mut rendered,
                &mut done,
            );
        }
        match edge.releases {
            None => {
                let identity = control
                    .offer_note_on(
                        &mut store,
                        SampleTime::new(edge.at),
                        slot,
                        synth_engine_v2::quantities::KeyIdentity::new(edge.key).expect("key"),
                        synth_engine_v2::quantities::NoteVelocity::saturating(edge.velocity),
                    )
                    .expect("the note-on is admitted, stealing if it must");
                identities.push(identity);
            }
            Some(of) => match edge.cents {
                Some(cents) => control
                    .offer_bend(
                        &mut store,
                        SampleTime::new(edge.at),
                        identities[of],
                        synth_engine_v2::quantities::Cents::new(cents).expect("a bend"),
                    )
                    .expect("the bend is admitted"),
                None => control
                    .offer_note_off(&mut store, SampleTime::new(edge.at), identities[of])
                    .expect("the release is admitted"),
            },
        }
    }
    while done < total {
        render_block(
            &mut scheduler,
            &mut renderer,
            &mut store,
            &mut rendered,
            &mut done,
        );
    }
    (rendered, store.counters(), renderer)
}

fn stealing(
    declarations: PlanDeclarations,
    policy: synth_engine_v2::ir::StealingPolicy,
) -> PlanDeclarations {
    PlanDeclarations {
        stealing: policy,
        ..declarations
    }
}

const FADE: synth_engine_v2::time::FrameCount = synth_engine_v2::time::FrameCount::new(128);

#[test]
fn a_live_note_on_into_a_full_producer_steals_as_the_compiled_one_does() {
    // ADR-0058 clause 6's second site, held to the first: the same three notes on a two-voice
    // plan — the third arriving with both voices held — render the same samples whether a
    // live producer offers them or a compiled stream states them. The compiled path stamps
    // the fade, the reset and the delayed start; the live path queues the fade and defers the
    // start outside the ring until the window reaches it. The taken note's release, offered
    // later, is counted at the boundary and is not an orphan.
    // Two holds for two voices, and every hold is outstanding when the third note arrives: the
    // taken note's hold goes with its voice, because its release is counted here and never
    // queued, so the steal spends no reservation.
    let oldest = synth_engine_v2::ir::StealingPolicy::Oldest { fade: FADE };
    let edges = [
        on(0, 60, 1.0),
        on(2 * Q, 67, 1.0),
        on(4 * Q + 5, 72, 1.0),
        off(20 * Q, 0),
        off(22 * Q, 1),
        off(24 * Q, 2),
    ];
    let compiled = render_compiled_edges(stealing(compiled_only_notes(2), oldest), &edges, 28);
    let (live, counters, renderer) =
        render_live_edges(stealing(live_only(2, 2), oldest), &edges, 28);
    assert_eq!(
        live, compiled,
        "the live steal did not render as the compiled one"
    );
    assert!(live.iter().any(|s| *s != 0.0));
    assert_eq!(
        counters.released_after_steal(),
        1,
        "the taken note's release"
    );
    assert_eq!(counters.orphan_releases(), 0);
    assert_eq!(counters.dropped(), 0);
    assert_eq!(renderer.diagnostics().ingress_released_after_steal(), 1);
    assert_eq!(renderer.diagnostics().orphan_note_events(), 0);
}

#[test]
fn a_live_release_offered_while_the_start_is_pending_is_displaced_with_it() {
    // The note that takes a voice starts `fade` frames late; a release offered before that —
    // a note shorter than the fade — is displaced with it, as the compiled path displaces
    // its release, so the note keeps its length on both paths and never hangs.
    let oldest = synth_engine_v2::ir::StealingPolicy::Oldest { fade: FADE };
    let edges = [
        on(0, 60, 1.0),
        on(2 * Q, 67, 1.0),
        on(4 * Q + 5, 72, 1.0),
        off(4 * Q + 5 + 50, 2),
        off(20 * Q, 0),
        off(22 * Q, 1),
    ];
    let compiled = render_compiled_edges(stealing(compiled_only_notes(2), oldest), &edges, 28);
    let (live, counters, renderer) =
        render_live_edges(stealing(live_only(2, 2), oldest), &edges, 28);
    assert_same(
        &live,
        &compiled,
        "the live render differs from the compiled one",
    );
    assert_eq!(counters.released_after_steal(), 1);
    assert_eq!(renderer.diagnostics().orphan_note_events(), 0);
    // The short note ended: nothing of it sounds after its displaced release.
    let alone_b = render_compiled_edges(
        stealing(compiled_only_notes(2), oldest),
        &[on(2 * Q, 67, 1.0), off(22 * Q, 0)],
        28,
    );
    let quiet_from = (4 * Q + 5 + 128 + 50 + Q) as usize;
    let quiet_to = (20 * Q) as usize;
    assert_eq!(&live[quiet_from..quiet_to], &alone_b[quiet_from..quiet_to]);
}

#[test]
fn a_live_same_note_retrigger_matches_the_compiled_one() {
    // `SameNote` at the boundary: a held key struck again takes its own voice at once, with
    // the new velocity and no fade, exactly as the compiled path stamps it.
    let same = synth_engine_v2::ir::StealingPolicy::SameNote { fade: FADE };
    let edges = [
        on(0, 60, 1.0),
        on(2 * Q, 67, 1.0),
        on(4 * Q + 5, 60, 0.5),
        off(20 * Q, 2),
        off(22 * Q, 1),
        off(24 * Q, 0),
    ];
    let compiled = render_compiled_edges(stealing(compiled_only_notes(2), same), &edges, 28);
    let (live, counters, _) = render_live_edges(stealing(live_only(2, 2), same), &edges, 28);
    assert_eq!(
        live, compiled,
        "the live retrigger did not render as the compiled one"
    );
    assert_eq!(counters.released_after_steal(), 1);
}

#[test]
fn a_voice_taken_twice_before_its_first_victims_release_counts_every_release_and_leaks_no_hold() {
    // An independent read's scenario: on two voices, a fourth and a fifth note each take a
    // voice whose earlier occupant's release has not yet been offered. Every taken note's
    // release, when it comes, is counted and not an orphan; no hold leaks, which the next
    // note-on after every release proves by being admitted; and the live render still equals
    // the compiled one.
    let oldest = synth_engine_v2::ir::StealingPolicy::Oldest { fade: FADE };
    let edges = [
        on(0, 60, 1.0),
        on(2 * Q, 67, 1.0),
        on(4 * Q + 5, 72, 1.0),
        on(6 * Q + 5, 75, 1.0),
        on(8 * Q + 5, 79, 1.0),
        off(20 * Q, 0),
        off(21 * Q, 1),
        off(22 * Q, 2),
        off(23 * Q, 3),
        off(24 * Q, 4),
        on(26 * Q, 84, 1.0),
        off(27 * Q, 5),
    ];
    let compiled = render_compiled_edges(stealing(compiled_only_notes(2), oldest), &edges, 30);
    let (live, counters, renderer) =
        render_live_edges(stealing(live_only(2, 2), oldest), &edges, 30);
    assert_eq!(
        live, compiled,
        "repeated live steals did not render as the compiled ones"
    );
    assert_eq!(counters.released_after_steal(), 3, "A, B and C were taken");
    assert_eq!(counters.orphan_releases(), 0);
    assert_eq!(
        counters.dropped(),
        0,
        "the sixth note found a hold: none leaked"
    );
    assert_eq!(renderer.diagnostics().orphan_note_events(), 0);
}

#[test]
fn a_live_note_arriving_while_a_taken_voice_waits_to_start_takes_the_other_voice() {
    // C takes A's voice at `p` and waits to start; D arrives before it has. C's voice is
    // committed, so D takes B's — the compiled path's rule, and the render matches it. C's
    // release, offered while it waits, keeps C's index until its displaced position.
    let oldest = synth_engine_v2::ir::StealingPolicy::Oldest { fade: FADE };
    let p = 4 * Q + 5;
    let edges = [
        on(0, 60, 1.0),
        on(2 * Q, 67, 1.0),
        on(p, 72, 1.0),
        off(p + 50, 2),
        on(p + 60, 76, 1.0),
        off(20 * Q, 0),
        off(21 * Q, 1),
        off(22 * Q, 3),
    ];
    let compiled = render_compiled_edges(stealing(compiled_only_notes(2), oldest), &edges, 28);
    let (live, counters, renderer) =
        render_live_edges(stealing(live_only(2, 2), oldest), &edges, 28);
    assert_same(
        &live,
        &compiled,
        "the live render differs from the compiled one",
    );
    assert_eq!(counters.released_after_steal(), 2);
    assert_eq!(counters.orphan_releases(), 0);
    assert_eq!(counters.dropped(), 0);
    assert_eq!(renderer.diagnostics().orphan_note_events(), 0);
}

#[test]
fn a_live_note_on_finding_every_voice_waiting_to_start_is_dropped_by_name() {
    // The boundary's twin of the compiled over-emission: on one voice, C takes A and waits to
    // start, and D arrives before it has. The only voice is committed, so D is dropped with
    // the identity named — the compiled path refuses the same list at preparation.
    let oldest = synth_engine_v2::ir::StealingPolicy::Oldest { fade: FADE };
    let plan = plan_with(stealing(live_only(1, 1), oldest));
    let (mut control, renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let slot = plan.resolve_note(ENVELOPE).expect("playable");
    let offer = |control: &mut StreamControl, store: &mut PerformanceIngress, at: u64, key: u8| {
        control.offer_note_on(
            store,
            SampleTime::new(at),
            slot,
            synth_engine_v2::quantities::KeyIdentity::new(key).expect("key"),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
    };
    let p = 4 * Q + 5;
    let _a = offer(&mut control, &mut store, 0, 60).expect("admitted");
    let _c = offer(&mut control, &mut store, p, 72).expect("takes A's voice");
    let refused =
        offer(&mut control, &mut store, p + 60, 76).expect_err("C's voice waits to start");
    assert!(matches!(
        refused,
        IngressRefused::Dropped {
            resource: ExhaustedResource::Identity
        }
    ));
    assert_eq!(store.counters().dropped_identity(), 1);
}

#[test]
fn a_live_bend_moves_the_note_it_names_as_the_compiled_one_does() {
    // `SOUND-INV-021`'s bend through the live boundary, addressed by the occurrence the
    // note-on returned: the same edges bent live and stated compiled render the same samples,
    // and the untouched note is untouched.
    let plain = synth_engine_v2::ir::StealingPolicy::None;
    let edges = [
        on(0, 60, 0.5),
        on(2 * Q, 67, 1.0),
        bend(4 * Q + 5, 0, -350.0),
        bend(8 * Q, 1, 100.0),
        off(20 * Q, 0),
        off(22 * Q, 1),
    ];
    let compiled = render_compiled_edges_on(
        &pitched(stealing(compiled_only_notes(2), plain)),
        &edges,
        26,
    );
    let (live, counters, renderer) =
        render_live_edges_on(&pitched(stealing(live_only(2, 2), plain)), &edges, 26);
    assert_same(
        &live,
        &compiled,
        "the live bend did not render as the compiled one",
    );
    assert_eq!(counters.orphan_expressions(), 0);
    assert_eq!(renderer.diagnostics().orphan_note_events(), 0);
    // And the bend did something: the unbent render differs from the first bend on.
    let unbent = render_compiled_edges_on(
        &pitched(stealing(compiled_only_notes(2), plain)),
        &[
            on(0, 60, 0.5),
            on(2 * Q, 67, 1.0),
            off(20 * Q, 0),
            off(22 * Q, 1),
        ],
        26,
    );
    let from = (4 * Q + 5 + Q) as usize;
    assert_ne!(
        &compiled[from..from + 64],
        &unbent[from..from + 64],
        "the bend moved nothing"
    );
}

#[test]
fn a_live_bend_of_a_note_whose_start_is_deferred_waits_with_it() {
    // C takes a voice and waits to start; a bend of C offered before the start is displaced
    // with it, as the compiled path displaces its bend, and both render alike.
    let oldest = synth_engine_v2::ir::StealingPolicy::Oldest { fade: FADE };
    let p = 4 * Q + 5;
    let edges = [
        on(0, 60, 1.0),
        on(2 * Q, 67, 1.0),
        on(p, 72, 1.0),
        bend(p + 50, 2, 25.0),
        off(20 * Q, 0),
        off(22 * Q, 1),
        off(24 * Q, 2),
    ];
    let compiled = render_compiled_edges_on(
        &pitched(stealing(compiled_only_notes(2), oldest)),
        &edges,
        28,
    );
    let (live, counters, _) =
        render_live_edges_on(&pitched(stealing(live_only(2, 2), oldest)), &edges, 28);
    assert_same(
        &live,
        &compiled,
        "the deferred live bend did not render as the compiled one",
    );
    assert_eq!(counters.orphan_expressions(), 0);
    let unbent = render_compiled_edges_on(
        &pitched(stealing(compiled_only_notes(2), oldest)),
        &[
            on(0, 60, 1.0),
            on(2 * Q, 67, 1.0),
            on(p, 72, 1.0),
            off(20 * Q, 0),
            off(22 * Q, 1),
            off(24 * Q, 2),
        ],
        28,
    );
    let from = (p + 50 + 128 + Q) as usize;
    assert_ne!(
        &compiled[from..from + 64],
        &unbent[from..from + 64],
        "the bend moved nothing"
    );
}

#[test]
fn a_live_bend_naming_a_note_the_producer_does_not_hold_is_refused_as_an_orphan() {
    // A bend for a released occurrence, or one from another table, is refused at the boundary
    // and counted as an orphan expression — never queued to be refused a pass later.
    let plan = plan_with(live_only(2, 2));
    let (mut control, renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let slot = plan.resolve_note(ENVELOPE).expect("playable");
    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(0),
            slot,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("admitted");
    control
        .offer_note_off(&mut store, SampleTime::new(Q), identity)
        .expect("released");
    let refused = control
        .offer_bend(
            &mut store,
            SampleTime::new(2 * Q),
            identity,
            synth_engine_v2::quantities::Cents::new(10.0).expect("a bend"),
        )
        .expect_err("the note is released");
    assert!(matches!(refused, IngressRefused::OrphanExpression { .. }));
    assert_eq!(store.counters().orphan_expressions(), 1);
    assert_eq!(
        store.len(),
        2,
        "the note's two edges, and no bend queued behind them"
    );
}

#[test]
fn a_live_producer_declaring_no_stealing_still_drops_the_note_on_by_name() {
    // `None` is today's boundary: the third note-on into a full two-voice producer is dropped
    // with the resource named — the hold, which runs out with the voices since a producer
    // declares no more holds than notes — and nothing else changes.
    let plan = plan_with(live_only(2, 2));
    let (mut control, renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let slot = plan.resolve_note(ENVELOPE).expect("playable");
    for (at, key) in [(0_u64, 60_u8), (Q, 67)] {
        let _held = control
            .offer_note_on(
                &mut store,
                SampleTime::new(at),
                slot,
                synth_engine_v2::quantities::KeyIdentity::new(key).expect("key"),
                synth_engine_v2::quantities::NoteVelocity::FULL,
            )
            .expect("admitted");
    }
    let refused = control
        .offer_note_on(
            &mut store,
            SampleTime::new(2 * Q),
            slot,
            synth_engine_v2::quantities::KeyIdentity::new(72).expect("key"),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect_err("the producer is full and does not steal");
    assert!(matches!(
        refused,
        IngressRefused::Dropped {
            resource: ExhaustedResource::Hold
        }
    ));
    assert_eq!(store.counters().dropped_hold(), 1);
}

/// Sample-for-sample equality, reported at the first frame that differs.
fn assert_same(live: &[f32], compiled: &[f32], what: &str) {
    assert_eq!(live.len(), compiled.len(), "{what}: lengths differ");
    if let Some(frame) = live.iter().zip(compiled).position(|(l, c)| l != c) {
        panic!(
            "{what}: frame {frame} (quantum {}, offset {}): live {} against compiled {}",
            frame / Q as usize,
            frame % Q as usize,
            live[frame],
            compiled[frame]
        );
    }
}

/// `compiled_only` with the producer's polyphony chosen.
fn compiled_only_notes(simultaneous: u32) -> PlanDeclarations {
    PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(simultaneous),
            simultaneous_holds: EventCount::NONE,
        }],
        ..PlanDeclarations::default()
    }
}

#[test]
fn a_simulated_ingress_note_reaches_the_same_samples_as_the_compiled_one() {
    let partitions: [&[usize]; 4] = [&WHOLE, &BLOCKS_256, &BLOCKS_64, &IRREGULAR];

    for (index, partition) in partitions.iter().enumerate() {
        let compiled = render_compiled(partition);
        let (ingress, renderer, _spent) = render_ingress(partition);
        assert_eq!(
            ingress, compiled,
            "partition {index}: the ingress producer and the compiled path must reach the same \
             samples"
        );
        assert_eq!(
            renderer.diagnostics().late_events(),
            0,
            "partition {index}: an on-time offer must not reach the preserving late clamp"
        );
    }

    // And the samples are the declared ones rather than merely equal to each other: two
    // paths that both rounded to a quantum boundary would agree with each other and be
    // wrong. `Q` is the renderer's declared constant live-output carry.
    let compiled = render_compiled(&WHOLE);
    let sounding = ON + Q..OFF + Q;
    for (frame, sample) in compiled.iter().copied().enumerate() {
        let expected = if sounding.contains(&(frame as u64)) {
            1.0
        } else {
            0.0
        };
        assert_eq!(
            sample, expected,
            "frame {frame} must reflect the exact edge plus the declared carry"
        );
    }
}

#[test]
fn a_live_note_on_spends_the_live_share_and_its_release_the_guaranteed_one() {
    // ADR-0046 clause 6: a live note-on is charged to the live share, and "when an
    // individual release becomes publishable it redeems one hold into the guaranteed-release
    // share". One producer, one queue, **two** classes.
    //
    // Without this the equivalence test above is blind to attribution — charging both edges
    // to the live share renders exactly the same audio, so the whole point of the partition
    // would be unchecked. That was a real gap: the mutation passed every other test in this
    // file.
    let (_rendered, _renderer, arbiter) = render_ingress(&WHOLE);
    assert_eq!(
        arbiter.high_water(ProducerClass::Live),
        EventCount::measured(1),
        "the note-on spends the live share"
    );
    assert_eq!(
        arbiter.high_water(ProducerClass::Release),
        EventCount::measured(1),
        "the release redeems its hold into the guaranteed-release share"
    );
    assert_eq!(
        arbiter.high_water(ProducerClass::Compiled),
        EventCount::NONE,
        "the compiled stream is empty, so nothing may be charged to it"
    );
}

#[test]
fn a_release_discharges_the_hold_when_it_takes_the_slot_that_hold_reserved() {
    // ADR-0046 clause 6's hold is the room the queue keeps free for a release that is not
    // knowable yet. So it is spent the moment the release **occupies** a slot, not when
    // that slot is later published: keeping it until publication would reserve a slot for an
    // event already sitting in one, shrinking the usable queue by one per note in flight.
    //
    // Getting this wrong in the other direction is worse and was the first version's actual
    // bug: decrementing at both points took the count to zero while a note was still open.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let mut publication = arbiter();
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the note-on is admitted");
    assert_eq!(
        store.holds_outstanding(),
        EventCount::measured(1),
        "the note-on took a hold"
    );
    control
        .offer_note_off(&mut store, SampleTime::new(OFF), identity)
        .expect("the release is admitted");
    assert_eq!(
        store.holds_outstanding(),
        EventCount::NONE,
        "the release took the slot its own hold reserved, so the reservation is spent"
    );

    let mut block = vec![0.0_f32; TOTAL_FRAMES];
    let output = AudioBlockMut::new(&mut block, TOTAL_FRAMES, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the pass publishes the store");
    assert_eq!(
        store.holds_outstanding(),
        EventCount::NONE,
        "publication does not discharge it a second time"
    );
    assert!(store.is_empty(), "both edges were drained");
}

#[test]
fn a_simulated_producer_stamps_its_own_provenance_and_the_horizon_binds_it() {
    // ADR-0053: `Simulated` says both true things at once — engine-external, so the
    // horizon binds, and exact, so nothing declares an uncertainty.
    assert!(
        TimeSource::Simulated.is_ingress(),
        "a simulated producer is external, so ADR-0032 clause 21's horizon reaches it"
    );
    // The falsifier for that: `Compiled` is exempt, so a fixture stamping one would assert
    // a boundary it never crosses and would stay green with the horizon check removed.
    assert!(!TimeSource::Compiled.is_ingress());
}

#[test]
fn a_note_on_that_cannot_take_all_three_resources_is_dropped_with_the_resource_named() {
    let plan = plan_with(live_only(8, 2));
    let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    // **Each resource is exhausted on its own**, which is the whole point of naming it.
    // An earlier version declared four notes and four holds, so the fifth offer exhausted
    // both at once and the test proved only which branch runs first — an independent review
    // found that it established branch priority rather than classification.
    // Eight identities, two holds: the third note-on exhausts the **hold** while six
    // identities are still free, so nothing but the hold can explain the refusal.
    for frame in 0..2 {
        let _open = control
            .offer_note_on(
                &mut store,
                SampleTime::new(frame),
                note,
                common::any_key(),
                synth_engine_v2::quantities::NoteVelocity::FULL,
            )
            .expect("the entitlement covers two");
    }
    let refused = control
        .offer_note_on(
            &mut store,
            SampleTime::new(2),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect_err("the third exceeds the hold entitlement");
    assert_eq!(
        refused,
        IngressRefused::Dropped {
            resource: ExhaustedResource::Hold
        }
    );
    assert_eq!(store.counters().dropped_hold(), 1);
    assert_eq!(store.counters().dropped_identity(), 0);

    // **The mirror image cannot be built, and finding that out is the useful half.** An
    // identity-only exhaustion needs a plan whose identity range is smaller than its hold
    // entitlement, and admission refuses one: `ProducerHoldsExceedNotes` rejects
    // `live_only(2, 8)` at compile time, because a producer cannot owe more releases than
    // it can sound notes. With `holds <= notes` the hold is always reached first or at the
    // same moment, so the identity branch is reachable in this phase only through
    // `ProducerRangeEroded` — an index whose `u32` generation space is spent, which no test
    // can walk to.
    //
    // The branch is kept rather than removed because `HOST-INV-009` names identity as one of
    // the three causes and ADR-0048 will lift the rebuild refusal that erodes a range. What
    // is recorded here is that this phase cannot exercise it, not that it has been checked.
    assert!(
        std::panic::catch_unwind(|| plan_with(live_only(2, 8))).is_err(),
        "a plan owing more releases than it can sound notes is refused at admission"
    );
}

#[test]
fn a_stream_is_served_by_one_ingress_store() {
    // Two stores for one producer each hold that producer's whole hold entitlement, so the
    // pair admits twice what ADR-0046 clause 6 partitioned. Worse, a release offered to the
    // store that did not mint the note finds a live occurrence in the shared minter, spends
    // a reservation it never made, and leaves the minting store holding a hold nothing can
    // ever discharge. An independent review found it.
    let plan = plan();
    let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut first = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let mut second = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("a second store is constructible; using it is what is refused");
    assert_ne!(first.id(), second.id(), "store identities are not reissued");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    let identity = control
        .offer_note_on(
            &mut first,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the first store is latched by first use");
    let refused = control
        .offer_note_off(&mut second, SampleTime::new(OFF), identity)
        .expect_err("the second store may not discharge the first store's hold");
    assert!(matches!(refused, IngressRefused::ForeignStore { .. }));
    assert_eq!(
        first.holds_outstanding(),
        EventCount::measured(1),
        "the hold stays with the store that minted the note"
    );
    assert_eq!(second.holds_outstanding(), EventCount::NONE);
    // The latched store still works.
    control
        .offer_note_off(&mut first, SampleTime::new(OFF), identity)
        .expect("the latched store discharges its own hold");
}

#[test]
fn a_note_on_is_refused_when_the_queue_has_no_room_for_the_release_it_would_owe() {
    // **The boundary case, and the one an easier test cannot see.** ADR-0046 clause 6
    // promises that an accepted note-on's release "cannot be dropped by queue pressure", so
    // the queue reserves a slot for every outstanding hold. A note-on therefore needs *two*
    // units of room: its own entry and that reservation.
    //
    // Filling to a single free slot is what separates the two arithmetics. Charging a
    // note-on one unit admits it here and leaves its release nowhere to go — which either
    // overwrites an undrained entry or breaks the promise. The fixture asserts the refusal
    // at one free slot and the acceptance at two, so an off-by-one in either direction
    // fails it.
    let plan = plan();
    let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let depth = host
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity()
        .get() as usize;
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");
    let param = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
        .expect("the envelope's gate is a prepared parameter");

    // Two free slots left: a note-on fits, with room kept for its release.
    for index in 0..depth - 2 {
        control
            .offer_parameter(
                &mut store,
                SampleTime::new(index as u64),
                param,
                synth_engine_v2::quantities::ParameterValue::ZERO,
            )
            .expect("a parameter write needs one slot and reserves nothing");
    }
    let mut probe = store;
    // Stamps stay non-decreasing: the store refuses a past it has already left, so the
    // fixture's filler decides where the note has to sit.
    let identity = control
        .offer_note_on(
            &mut probe,
            SampleTime::new(depth as u64),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("two free slots hold a note-on and its reservation");
    control
        .offer_note_off(&mut probe, SampleTime::new(depth as u64 + Q), identity)
        .expect("the release takes the slot its hold reserved");

    // One free slot left: the note-on is refused, naming the slot. A fresh stream, because
    // a stream serves one store and the probe above latched the other one.
    let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    for index in 0..depth - 1 {
        control
            .offer_parameter(
                &mut store,
                SampleTime::new(index as u64),
                param,
                synth_engine_v2::quantities::ParameterValue::ZERO,
            )
            .expect("a parameter write needs one slot");
    }
    let refused = control
        .offer_note_on(
            &mut store,
            SampleTime::new(depth as u64),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect_err("one free slot cannot hold a note-on and its release");
    assert_eq!(
        refused,
        IngressRefused::Dropped {
            resource: ExhaustedResource::Slot
        }
    );
}

#[test]
fn a_release_naming_no_open_note_is_refused_at_the_boundary_rather_than_queued() {
    let plan = plan();
    let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(0),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the note-on is admitted");
    control
        .offer_note_off(&mut store, SampleTime::new(Q), identity)
        .expect("the first release resolves");
    // Replaying it names a spent occurrence. Refusing here rather than queuing it is what
    // keeps a producer's mistake from spending a slot to reach a renderer that refuses it
    // one pass later.
    let refused = control
        .offer_note_off(&mut store, SampleTime::new(2 * Q), identity)
        .expect_err("a spent occurrence releases nothing");
    assert!(matches!(refused, IngressRefused::OrphanRelease { .. }));
    assert_eq!(store.counters().orphan_releases(), 1);
    assert_eq!(store.counters().dropped(), 0, "an orphan is not a drop");
}

#[test]
fn the_compiled_producer_cannot_own_a_live_store() {
    // ADR-0046 clause 6: a compiled producer declares no hold, because its releases use the
    // plan entitlement clause 4 established. Preparing a live store against it would hand
    // every live note-on an entitlement of zero and drop all of them, reporting a shortage
    // for what is really a plan that never declared a live producer.
    let plan = plan();
    let (_control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let compiled = plan_with(compiled_only());
    let (_c, compiled_renderer) =
        StreamControl::open(compiled.clone(), ORIGIN).expect("preparation");
    assert!(
        PerformanceIngress::prepare(&host, &compiled, ONLY_PRODUCER, &compiled_renderer).is_err()
    );
    assert!(PerformanceIngress::prepare(&host, &plan, ProducerId::new(7), &renderer).is_err());
}

#[test]
fn an_activation_is_refused_once_a_stream_has_adopted_a_live_store() {
    // ADR-0050 clause 8 scopes transport activation to a stream whose note producers are
    // compiled, and two things go wrong without the scope. Clause 5's boundary mass release
    // ends a live note by freeing its index, without passing through the store — so its
    // release hold is never discharged and the queue keeps a reservation for an event that
    // can no longer exist. And ADR-0051 clause 6 leaves a gate reached by two producers with
    // no ownership law, so the catch-up's row can cut a live note the activation does not
    // own.
    //
    // **It refuses on the store, not on the notes currently open**, and that distinction is
    // the whole fixture. An earlier version gated on a count of open notes, which an offered
    // release takes back to zero **while both edges are still queued and neither has
    // rendered** — so an activation built there sits over a live note that is about to
    // sound. An independent review found that sequence, and it is the second half below.
    let plan = plan();
    let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    // Before any offer the stream has adopted nothing, so an activation builds.
    let request = ActivationRequest {
        at: SampleTime::new(8 * Q),
        position: PlanPosition::ZERO,
        loop_interval: None,
    };
    let candidate = control
        .plan_activation(&empty, request)
        .expect("a stream with no live store activates");
    control
        .withdraw(candidate)
        .expect("the candidate withdraws");

    // One offer adopts the store, and from there the stream is out of clause 8's scope.
    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the note-on is admitted");
    let request = ActivationRequest {
        at: SampleTime::new(8 * Q),
        position: PlanPosition::ZERO,
        loop_interval: None,
    };
    let refused = control
        .plan_activation(&empty, request)
        .expect_err("a stream serving a live store is outside the activation contract");
    assert!(matches!(
        refused,
        ActivationBuildError::LiveIngressAdopted { .. }
    ));

    // **And releasing the note does not reopen it**, which a count of open notes would: both
    // edges are queued and neither has rendered, so the note is still about to sound.
    control
        .offer_note_off(&mut store, SampleTime::new(OFF), identity)
        .expect("the release is admitted");
    assert_eq!(
        control.live_notes_open(),
        HeldNoteCount::NONE,
        "the count a weaker guard would have consulted is already zero here"
    );
    assert_eq!(store.len(), 2, "while both edges are still queued");
    let request = ActivationRequest {
        at: SampleTime::new(8 * Q),
        position: PlanPosition::ZERO,
        loop_interval: None,
    };
    let refused = control
        .plan_activation(&empty, request)
        .expect_err("an offered release does not put the stream back in scope");
    assert!(matches!(
        refused,
        ActivationBuildError::LiveIngressAdopted { .. }
    ));

    // The store is untouched by the refusals: both edges still render.
    let mut block = vec![0.0_f32; TOTAL_FRAMES];
    let output =
        AudioBlockMut::new(&mut block, TOTAL_FRAMES, ChannelLayout::Mono).expect("shaped block");
    let mut publication = arbiter();
    let mut renderer = renderer;
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the pass publishes the store");
    assert!(store.is_empty(), "both edges survived the refusals");
}
#[test]
fn the_live_boundary_s_drops_reach_the_structured_report_with_the_cause_kept() {
    // `HOST-INV-009` requires two things of a registered live store, and they are separate:
    // the drop is counted with the exhausted resource named, and the count **reaches the
    // structured diagnostics report**. The drop happens on the producing half before
    // acceptance, where the report cannot be reached, so the drain is the only point at
    // which the two halves meet — and without that step the second requirement is unmet
    // however carefully the first one is.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    // Four is the declared entitlement, so the fifth and sixth are dropped on the hold.
    let mut open = Vec::new();
    for frame in 0..4 {
        open.push(
            control
                .offer_note_on(
                    &mut store,
                    SampleTime::new(frame),
                    note,
                    common::any_key(),
                    synth_engine_v2::quantities::NoteVelocity::FULL,
                )
                .expect("the entitlement covers four"),
        );
    }
    for frame in 4..6 {
        assert!(
            control
                .offer_note_on(
                    &mut store,
                    SampleTime::new(frame),
                    note,
                    common::any_key(),
                    synth_engine_v2::quantities::NoteVelocity::FULL
                )
                .is_err()
        );
    }
    assert_eq!(store.counters().dropped_hold(), 2);
    assert_eq!(
        renderer.diagnostics().ingress_dropped(),
        0,
        "nothing has reached the report before a pass has run"
    );

    let mut block = vec![0.0_f32; TOTAL_FRAMES];
    let output = AudioBlockMut::new(&mut block, TOTAL_FRAMES, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");
    let mut publication = arbiter();
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the pass publishes the store");

    assert_eq!(renderer.diagnostics().ingress_dropped_hold(), 2);
    assert_eq!(renderer.diagnostics().ingress_dropped_slot(), 0);
    assert_eq!(renderer.diagnostics().ingress_dropped_identity(), 0);
    assert_eq!(renderer.diagnostics().ingress_dropped(), 2);

    // A second pass must not count the same two again. The store's counters are cumulative
    // over the stream, so the report mirrors them rather than accumulating them — adding
    // each pass would report every earlier drop again on every callback.
    let mut block = vec![0.0_f32; TOTAL_FRAMES];
    let output = AudioBlockMut::new(&mut block, TOTAL_FRAMES, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the second pass runs");
    assert_eq!(
        renderer.diagnostics().ingress_dropped(),
        2,
        "a cumulative total mirrored each pass must not be added to itself"
    );
    let _ = open;
}

/// Render a live-only plan, offering into the store through `offer`, one block at a time.
fn drive(
    offers: impl FnOnce(&mut StreamControl, &mut PerformanceIngress, synth_engine_v2::plan::NoteSlot),
    partition: &[usize],
) -> (Vec<f32>, PerformanceIngress) {
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");
    offers(&mut control, &mut store, note);

    let mut publication = arbiter();
    let mut rendered = Vec::with_capacity(TOTAL_FRAMES);
    for &frames in partition {
        let mut block = vec![0.0_f32; frames];
        let output = AudioBlockMut::new(&mut block, frames, ChannelLayout::Mono)
            .expect("the output block is shaped correctly");
        scheduler
            .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
            .expect("the pass publishes the store");
        rendered.extend_from_slice(&block);
    }
    (rendered, store)
}

#[test]
fn a_note_and_its_release_at_one_render_position_apply_in_the_order_they_were_offered() {
    // **`SOUND-INV-020`'s decisive case, and the one the equivalence render cannot see.**
    // There the two edges sit at different samples, so position sorting restores their order
    // whatever the drain did with them — an independent review found the fixture proving
    // nothing about ordering. Here they share a render position, and only the order they
    // were presented in decides.
    //
    // The falsifier is loud rather than subtle. Applied on-then-off, the gate rises and
    // falls at one sample and nothing sounds. Applied off-then-on — which is what any order
    // derived from ADR-0046's capacity classes produces, since the release is charged to the
    // guaranteed-release share and the note-on to the live one — the release finds no live
    // note, is refused as an orphan, and the note-on that follows sounds to the end of the
    // render with nothing left to end it.
    let (rendered, store) = drive(
        |control, store, note| {
            let identity = control
                .offer_note_on(
                    store,
                    SampleTime::new(ON),
                    note,
                    common::any_key(),
                    synth_engine_v2::quantities::NoteVelocity::FULL,
                )
                .expect("the note-on is admitted");
            control
                .offer_note_off(store, SampleTime::new(ON), identity)
                .expect("its release at the same sample is admitted");
        },
        &WHOLE,
    );
    assert!(store.is_empty(), "both edges were drained");
    assert!(
        rendered.iter().all(|sample| *sample == 0.0),
        "a note released at the sample it starts sounds for no frames; a non-silent render \
         means the release was applied before the note-on it discharges"
    );
}

#[test]
fn an_entry_whose_destination_is_past_the_window_waits_for_the_call_that_reaches_it() {
    // ADR-0046 clause 3: "an accepted queue entry waits only for its destination to enter
    // the horizon". A drain that charged everything it held would place a later note in this
    // call's window; one that charged nothing would strand it. The fixture renders one 64-
    // frame block and asserts the entry is still queued, then renders the rest and asserts
    // it arrived at its own sample.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");
    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the note-on is admitted");
    control
        .offer_note_off(&mut store, SampleTime::new(OFF), identity)
        .expect("the release is admitted");

    let mut publication = arbiter();
    let mut block = vec![0.0_f32; 64];
    let output = AudioBlockMut::new(&mut block, 64, ChannelLayout::Mono).expect("shaped block");
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the first call renders one quantum");
    assert_eq!(
        store.len(),
        2,
        "both destinations are past a window of one quantum, so neither is charged yet"
    );
    assert!(
        block.iter().all(|sample| *sample == 0.0),
        "nothing sounds before the note's own sample"
    );

    let mut block = vec![0.0_f32; TOTAL_FRAMES - 64];
    let output = AudioBlockMut::new(&mut block, TOTAL_FRAMES - 64, ChannelLayout::Mono)
        .expect("shaped block");
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the rest of the render reaches both destinations");
    assert!(store.is_empty(), "both entries were charged once reachable");
    // The first block covered frames 0..64 and the note starts at ON plus the declared
    // carry, so the step falls inside this second block at that offset.
    let first = block.iter().position(|sample| *sample != 0.0);
    assert_eq!(
        first,
        Some((ON + Q - 64) as usize),
        "the waiting entry landed on its own sample, not on the window it first missed"
    );
}

#[test]
fn the_ring_wraps_and_delivers_every_entry_exactly_once() {
    // The queue is a ring, so `tail` passes the end and starts again. A drain that assumed a
    // linear span would lose or repeat entries after the first wrap.
    //
    // **Every entry has to be distinguishable for loss to be observable**, and a first
    // version's were not: it wrote the same zero value from every slot and asserted only
    // emptiness and a charge count. Here each write lands in its **own** quantum and
    // alternates the gate, so the render is a square wave whose edges are the quantum
    // boundaries, and a lost entry merges two cells.
    //
    // **Duplication is invisible in that wave and is checked separately.** Charging one
    // write twice into its own quantum is idempotent — the same value wins either way — so
    // the audio cannot see it; what it consumes is the share, and the peak below is where
    // that shows. Loss and duplication need different instruments, and an earlier version of
    // this comment claimed one fixture covered both.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let depth = host
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity()
        .get() as usize;
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let param = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
        .expect("the envelope's gate is a prepared parameter");

    // One 1 024-frame block is 16 quanta, and the ring holds 32 entries, so three blocks of
    // 16 writes offer 48 — the ring wraps during the second block and again in the third.
    const PER_BLOCK: usize = 1_024 / QUANTUM_FRAMES as usize;
    let mut publication = arbiter();
    let mut rendered = Vec::new();
    let mut quantum = 0_u64;
    for _ in 0..3 {
        for _ in 0..PER_BLOCK {
            // Alternate the gate, one write per quantum boundary.
            let value = if quantum.is_multiple_of(2) {
                synth_engine_v2::quantities::ParameterValue::ONE
            } else {
                synth_engine_v2::quantities::ParameterValue::ZERO
            };
            control
                .offer_parameter(&mut store, SampleTime::new(quantum * Q), param, value)
                .expect("each block's writes fit the ring");
            quantum += 1;
        }
        let mut block = vec![0.0_f32; 1_024];
        let output =
            AudioBlockMut::new(&mut block, 1_024, ChannelLayout::Mono).expect("shaped block");
        scheduler
            .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
            .expect("the pass drains the ring");
        rendered.extend_from_slice(&block);
    }
    // A fourth block with nothing offered, to flush what the renderer's declared `Q`-frame
    // live-output carry still holds. Asserting emptiness per block would be asserting that
    // the carry does not exist.
    let mut block = vec![0.0_f32; 1_024];
    let output = AudioBlockMut::new(&mut block, 1_024, ChannelLayout::Mono).expect("shaped block");
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the flush pass renders");
    rendered.extend_from_slice(&block);
    assert!(store.is_empty(), "every entry was drained");
    assert!(
        quantum as usize > depth,
        "the fixture offered more entries than the ring holds"
    );

    // **The output is the evidence.** The gate is a sample-positioned control, so each write
    // takes effect at its own quantum boundary and the amplifier holds that level until the
    // next one. Live output carries the renderer's declared `Q` frames, so the write for
    // quantum `k` appears in output cell `k + 1`; cell zero is the gate before any write.
    // A lost entry merges two cells and breaks this comparison. A duplicate does not reach
    // it at all: see the share peak below.
    // The envelope is a voice-scope node with four instances, and a parameter write fans out
    // over every instance of its control (`P06-S001`), so an open gate sounds on all four
    // and the voice sum carries their total: the high cell is `instances`, not one.
    let instances = plan.voice_instances().get() as f32;
    for (frame, sample) in rendered.iter().copied().enumerate() {
        let cell = frame as u64 / Q;
        // Past the last write the gate holds what that write left, so the source cell is
        // clamped rather than special-cased.
        let written = cell.saturating_sub(1).min(quantum.saturating_sub(1));
        let expected = if cell == 0 {
            0.0
        } else if written.is_multiple_of(2) {
            instances
        } else {
            0.0
        };
        assert_eq!(
            sample, expected,
            "output frame {frame} is cell {cell}, which carries the write for quantum \
             {cell} minus one and no other"
        );
    }
    assert_eq!(
        renderer.diagnostics().ingress_dropped(),
        0,
        "wrapping is not a shortage"
    );

    // **Loss and duplication need different instruments, and the audio only sees one.**
    // Charging one write twice into its own quantum is idempotent — the same value wins
    // either way — so the pattern above cannot see it. What a duplicate does consume is the
    // share, and with exactly one write per quantum the live class's peak is one. A
    // mutation charging an entry twice moves it to two and fails here; the same mutation
    // leaves the render untouched.
    assert_eq!(
        publication.high_water(ProducerClass::Live),
        EventCount::measured(1),
        "one write per quantum means the live share never peaks above one"
    );
}

#[test]
fn an_orphan_release_reaches_the_structured_report_and_not_the_drop_count() {
    // `HOST-INV-009` requires the boundary's counts to reach the structured report, and the
    // one that is **not** a drop is the easy one to leave store-local. Removing that
    // argument from the mirror passed every earlier fixture, because they asserted
    // `IngressCounters` and never rendered afterwards — an independent review found it.
    //
    // The fixture also pins the classification: an orphan release is a refusal and not a
    // shortage, so it may not reach the drop count. **The store's other non-drop refusals
    // are the non-monotone stamp and the two store-identity refusals**; the out-of-horizon
    // one this fixture once also covered is gone with the check, which this slice cut.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the note-on is admitted");
    control
        .offer_note_off(&mut store, SampleTime::new(OFF), identity)
        .expect("the release is admitted");
    control
        .offer_note_off(&mut store, SampleTime::new(OFF), identity)
        .expect_err("a spent occurrence releases nothing");

    let mut block = vec![0.0_f32; TOTAL_FRAMES];
    let output = AudioBlockMut::new(&mut block, TOTAL_FRAMES, ChannelLayout::Mono)
        .expect("the output block is shaped correctly");
    let mut publication = arbiter();
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the pass publishes the store");

    // The boundary's own field. The condition has two sites — this one refused before
    // acceptance, the renderer's refused an event it was handed — so merging them would
    // report a defect at the wrong point.
    let report = renderer.diagnostics();
    assert_eq!(report.ingress_orphan_releases(), 1);
    assert_eq!(
        report.ingress_dropped(),
        0,
        "a refusal is not a shortage, so it does not consume HOST-INV-009's drop licence"
    );
    // And it is the boundary's own field, not the renderer's: nothing reached the renderer
    // to be refused there, so merging the two would report a defect at the wrong point and
    // send someone to fix the wrong half.
    assert_eq!(report.orphan_note_events(), 0);
}

#[test]
fn a_store_prepared_against_another_plan_is_refused() {
    // The store takes its hold entitlement and identity range from the plan it is given, and
    // mints through the **stream's** table. Two different plans make those halves disagree:
    // an eight-hold entitlement from one plan minting into a two-index range from another
    // spends an entitlement that plan never admitted, and makes an identity shortage
    // reachable although each plan separately satisfies `holds <= notes`. That is what would
    // have falsified this file's claim that identity exhaustion is unreachable, and an
    // independent review found it.
    let generous = plan_with(live_only(8, 8));
    let narrow = plan_with(live_only(2, 2));
    let (_control, renderer) = StreamControl::open(narrow, ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let refused = PerformanceIngress::prepare(&host, &generous, ONLY_PRODUCER, &renderer)
        .expect_err("a store may not serve a plan the renderer does not render");
    assert!(matches!(
        refused,
        synth_engine_v2::ingress::IngressPrepareError::ForeignPlan { .. }
    ));
}

#[test]
fn a_store_this_stream_never_adopted_is_refused_by_the_drain() {
    // **One mark, read by both halves.** The control marks a store when it adopts it, and
    // the drain reads that mark rather than keeping a latch of its own. Two independent
    // latches could disagree: offering into store A while rendering store B latched one on
    // each, and the stream wedged — offers reached only A, drains accepted only B, and
    // neither could ever be undone. An independent review found it.
    //
    // Refusing an unadopted store strands nothing, because every offer goes through the
    // control that sets the mark, so an unadopted store is empty. What it prevents is that
    // empty store's zero counters overwriting the adopted one's totals, since the drain
    // mirrors rather than accumulates.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut adopted = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let mut stranger = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("a second store is constructible; using it is what is refused");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");
    let mut publication = arbiter();

    let identity = control
        .offer_note_on(
            &mut adopted,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the first offer adopts the store");
    assert!(adopted.adopted_by().is_some());
    assert!(stranger.adopted_by().is_none());

    // The drain refuses the store nothing adopted.
    let mut block = vec![0.0_f32; 256];
    let output = AudioBlockMut::new(&mut block, 256, ChannelLayout::Mono).expect("shaped block");
    let refused = scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut stranger), output)
        .expect_err("an unadopted store is refused");
    assert!(matches!(
        refused,
        synth_engine_v2::schedule::ScheduledRenderError::UnadoptedIngressStore { .. }
    ));
    // A caller error, not a contract violation, and **not a wedge**: the adopted store still
    // renders afterwards. That is the property two independent latches did not have.
    assert!(!renderer.diagnostics().needs_reprepare());
    let mut block = vec![0.0_f32; TOTAL_FRAMES];
    let output =
        AudioBlockMut::new(&mut block, TOTAL_FRAMES, ChannelLayout::Mono).expect("shaped block");
    control
        .offer_note_off(&mut adopted, SampleTime::new(OFF), identity)
        .expect("the release is admitted");
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut adopted), output)
        .expect("the adopted store still renders");
    assert!(adopted.is_empty(), "both edges were drained");
}

#[test]
fn a_store_cannot_be_carried_to_a_second_stream() {
    // `prepare` binds a store to the plan its renderer renders; this binds it to the stream
    // that first used it. Without the second binding a store prepared against an eight-hold
    // plan could be offered through a control whose table holds two indices, which spends an
    // entitlement that stream never admitted — and makes the identity shortage reachable
    // although each plan separately satisfies `holds <= notes`. An independent review found
    // that `prepare`'s check alone did not close it.
    let plan = plan();
    let (mut first, first_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let (mut second, _second_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("a second stream opens");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &first_renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    let _identity = first
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the first stream adopts the store");
    let refused = second
        .offer_note_on(
            &mut store,
            SampleTime::new(OFF),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect_err("a second stream may not use another stream's store");
    assert!(matches!(refused, IngressRefused::ForeignStream { .. }));
}

#[test]
fn a_refused_store_leaves_the_stream_able_to_use_its_own() {
    // A refusal must leave the stream exactly as it found it, which is the rule every
    // activation refusal already follows. Recording the store id before the fallible
    // adoption broke it: a refused foreign store left its id latched, and the control's own
    // store was rejected as foreign for the rest of the stream — a permanent wedge from a
    // caller error. An independent review found the ordering.
    let plan = plan();
    let (mut first, first_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let (mut second, second_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("a second stream opens");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut owned = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &second_renderer)
        .expect("the second stream's own store");
    let mut foreign = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &first_renderer)
        .expect("a store belonging to the first stream");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    let _held = first
        .offer_note_on(
            &mut foreign,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the first stream adopts its own store");
    second
        .offer_note_on(
            &mut foreign,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect_err("the second stream may not use the first's store");

    // And it can still use its own, which a poisoned latch would refuse.
    let _adopted = second
        .offer_note_on(
            &mut owned,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the refusal left this stream able to adopt its own store");
}

#[test]
fn simulated_has_one_literal_construction_site_in_this_crate_s_source() {
    // A guard toward ADR-0053's fourth owed check, and **not that check**. The record asks
    // for a standing scan finding exactly one construction site "in this repository"; this
    // reads one crate and one spelling, so it is a weaker statement wearing a narrower name.
    // Closing the obligation needs a method that can carry the wider claim.
    //
    // **Its limits are wider than the record implies, and are stated rather than glossed.**
    // It establishes that one place in this crate's `src` writes the literal path
    // `TimeSource::Simulated` — the ingress store's `push`. It would **not** see an aliased
    // import, a wildcard `use`, a `Self::Simulated` inside the enum's own module, or a
    // construction in another crate, and it says nothing about whether a release build calls
    // the store or about a downstream consumer of a public enum. An independent review
    // pointed out that "this repository" and "one construction site" both claim more than a
    // scan of this shape can carry.
    //
    // What bounds release reachability is not this scan and not the crate's dependency
    // edges either — a first answer claimed the latter and it was false, since the three
    // permitted measurement examples link this crate in `--release`. It is the default-on
    // `simulated-ingress` feature, which gates the store's constructor and the three offers
    // that fill it. Clause 5's prohibition on a live adapter stamping one stays contractual
    // on top of that, because no boundary here can constrain a downstream consumer of a
    // public enum. Test source is excluded because a fixture asserting the variant's
    // behaviour is not an adapter.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sites = Vec::new();
    let mut stack = vec![root];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).expect("the crate source is readable") {
            let entry = entry.expect("a readable directory entry");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            // The crate keeps its unit tests under `src/tests/`; the scan covers what a
            // release build compiles into the library.
            if path
                .components()
                .any(|component| component.as_os_str() == "tests")
            {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("a readable source file");
            for (number, line) in source.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("TimeSource::Simulated") {
                    sites.push(format!("{}:{}", path.display(), number + 1));
                }
            }
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "this crate's production source must write the literal path exactly once; found \
         {sites:?}"
    );
    assert!(
        sites[0].contains("ingress.rs"),
        "the one site must be the ingress store, not {}",
        sites[0]
    );
}

#[test]
fn a_simulated_stamp_inside_the_horizon_is_admitted_and_moves_no_arrival_counter() {
    // ADR-0053's arrival-counter check: `Simulated` declares no arrival uncertainty, because
    // it is exact by construction, and folding it into the arrival counter would put a
    // fallback measurement in the report for a fallback that does not exist. Falsified by a
    // mutation that folds it in.
    //
    // **The record's horizon pair is owed, not covered here.** It needs the store to check
    // the forward horizon, which this slice cut: taking that check would be a second
    // evaluation site `HOST-INV-013` forbids, and which site the invariant means is the
    // decision `NOW.md` records. The check and the pair arrive together in the slice after
    // it.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("preparation succeeds");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("a stamp inside the horizon is admitted");
    control
        .offer_note_off(&mut store, SampleTime::new(OFF), identity)
        .expect("and so is its release");

    let mut block = vec![0.0_f32; TOTAL_FRAMES];
    let output =
        AudioBlockMut::new(&mut block, TOTAL_FRAMES, ChannelLayout::Mono).expect("shaped block");
    let mut publication = arbiter();
    scheduler
        .render_with_ingress(&mut renderer, &mut publication, Some(&mut store), output)
        .expect("the pass publishes both edges");
    assert!(store.is_empty(), "both edges reached the renderer");
    assert_eq!(
        renderer.diagnostics().arrival_stamped_events(),
        0,
        "a simulated stamp is exact by construction and declares no arrival fallback"
    );
}

#[test]
fn a_live_store_cannot_join_a_plan_that_also_declares_a_compiled_producer() {
    // ADR-0023's boundary, enforced rather than asserted. ADR-0051 clause 6 leaves a gate
    // reached by more than one producer with no ownership law, and nothing in a declaration
    // says which nodes a producer plays — so a live store beside a compiled producer is the
    // construction the clause forbids, conservatively read.
    //
    // **The check is here because prose was not enough.** Plans declaring both producers
    // already existed in this crate's fixtures, harmless while a non-compiled declaration
    // could not emit; this slice makes one emit, and an independent review found the claim
    // that no such plan existed to be false. Those fixtures never build a store.
    let mixed = common::admit(
        &gated_constant(PlanDeclarations {
            note_producers: vec![
                NoteProducerDeclaration {
                    compiled: true,
                    simultaneous_notes: HeldNoteCount::measured(2),
                    simultaneous_holds: EventCount::NONE,
                },
                NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: HeldNoteCount::measured(2),
                    simultaneous_holds: EventCount::measured(2),
                },
            ],
            ..PlanDeclarations::default()
        }),
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
    );
    let (_control, renderer) = StreamControl::open(mixed.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let refused = PerformanceIngress::prepare(&host, &mixed, ProducerId::new(1), &renderer)
        .expect_err("a live store may not join a plan that also declares a compiled producer");
    assert!(matches!(
        refused,
        synth_engine_v2::ingress::IngressPrepareError::MixedProducerPlan { .. }
    ));
}

#[test]
fn a_store_cannot_be_adopted_between_a_candidates_build_and_its_offer() {
    // **The mirror image of the refusal above, and it needs its own fixture.**
    // `plan_activation` refuses once a store is adopted, which keeps a live producer out of
    // ADR-0050 clause 8's scope — but that check runs when the candidate is *built*. Adopting
    // a store afterwards walks straight past it: build the candidate while the stream has no
    // store, adopt one, then offer the candidate, and the activation proceeds over a live
    // producer with its catch-up free to cut a gate the live note holds.
    //
    // A **parameter** offer is what found it, because it mints nothing and so had no reason
    // of its own to refuse while a candidate was outstanding. The check now lives at the one
    // point every offer passes through. An independent review found the ordering.
    let plan = plan();
    let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let _ = &mut scheduler;
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let param = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
        .expect("the envelope's gate is a prepared parameter");
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    // No store adopted yet, so the candidate builds.
    let request = ActivationRequest {
        at: SampleTime::new(8 * Q),
        position: PlanPosition::ZERO,
        loop_interval: None,
    };
    let candidate = control
        .plan_activation(&empty, request)
        .expect("a stream with no live store activates");

    // With that candidate outstanding, no offer may adopt a store — a parameter write least
    // of all, since it is the one that mints nothing.
    let refused = control
        .offer_parameter(
            &mut store,
            SampleTime::new(ON),
            param,
            synth_engine_v2::quantities::ParameterValue::ONE,
        )
        .expect_err("a parameter offer may not adopt a store under an outstanding candidate");
    assert!(matches!(refused, IngressRefused::CandidateOutstanding));
    let refused = control
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect_err("nor may a note offer");
    assert!(matches!(refused, IngressRefused::CandidateOutstanding));
    assert!(
        store.adopted_by().is_none(),
        "a refused offer adopts nothing"
    );

    // Withdrawing the candidate puts the stream back where it was, and the store may then be
    // adopted — after which an activation is refused, which the fixture above covers.
    control
        .withdraw(candidate)
        .expect("the candidate withdraws");
    let _open = control
        .offer_note_on(
            &mut store,
            SampleTime::new(ON),
            note,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("with no candidate outstanding the store is adopted");
}

#[test]
fn a_live_store_cannot_claim_a_producer_an_authored_source_already_holds() {
    // ADR-0046 clause 6 partitions hold entitlements so that "no producer borrows another's
    // unused holds", and an entitlement belongs to the **producer** rather than to whoever
    // claims it. Plan admission checks that a producer's authored sources fit its
    // entitlement — but admission cannot see the stores prepared afterwards.
    //
    // **What this fixture stops.** The plan reserves four holds on producer 0, an authored
    // source is admitted against all four, and then the live store asks for the same
    // producer. Both claims are individually legal: the producer resolves, it is not
    // compiled, and the authored source fits. Only comparing the two catches it, and an
    // independent review found that the authored link proved the index resolved and was
    // non-compiled — which is not the same as proving it was unclaimed.
    let claimed = common::admit(
        &gated_constant(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(8),
                simultaneous_holds: EventCount::measured(4),
            }],
            authored_sources: vec![synth_engine_v2::ir::AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: EventCount::measured(1),
                retained_future: EventCount::NONE,
                simultaneous_holds: EventCount::measured(4),
            }],
            ..PlanDeclarations::default()
        }),
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
    );
    let (_control, renderer) = StreamControl::open(claimed.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let refused = PerformanceIngress::prepare(&host, &claimed, ProducerId::new(0), &renderer)
        .expect_err("an authored source already holds this producer's entitlement");
    assert!(matches!(
        refused,
        synth_engine_v2::ingress::IngressPrepareError::AuthoredProducer { .. }
    ));
}

#[test]
fn a_live_store_may_claim_a_producer_no_authored_source_holds() {
    // The mirror, and what keeps the refusal above from being a blanket ban on plans that
    // declare an authored source at all: entitlements are disjoint per producer, so a store
    // on the *other* producer is admissible while the first is claimed.
    let claimed = common::admit(
        &gated_constant(PlanDeclarations {
            note_producers: vec![
                NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: HeldNoteCount::measured(8),
                    simultaneous_holds: EventCount::measured(4),
                },
                NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: HeldNoteCount::measured(8),
                    simultaneous_holds: EventCount::measured(4),
                },
            ],
            authored_sources: vec![synth_engine_v2::ir::AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: EventCount::measured(1),
                retained_future: EventCount::NONE,
                simultaneous_holds: EventCount::measured(4),
            }],
            ..PlanDeclarations::default()
        }),
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
    );
    let (_control, renderer) = StreamControl::open(claimed.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    assert!(
        PerformanceIngress::prepare(&host, &claimed, ProducerId::new(1), &renderer).is_ok(),
        "a producer no authored source claims is still available to the live store"
    );
}

#[test]
fn a_note_slot_from_another_plan_is_refused_before_it_can_mint() {
    // **The renderer does not re-check the slot's plan.** `NoteSlot` carries the `PlanId` it
    // was resolved against, but `note_target` applies the slot's *index* to whichever plan is
    // rendering — so a slot resolved against plan B and offered to a stream on plan A plays
    // plan A's target at B's numeric index. A different note, silently, with no error
    // anywhere downstream. An independent review found this at the merge gate: nothing on the
    // offer path compared the two ids.
    //
    // Two separately compiled plans give two distinct `PlanId`s, which is what makes the
    // comparison meaningful rather than trivially true.
    let plan_a = plan();
    let plan_b = plan();
    assert_ne!(
        plan_a.id(),
        plan_b.id(),
        "two compilations must not share an identity, or this fixture proves nothing"
    );

    let (mut control, renderer) =
        StreamControl::open(plan_a.clone(), ORIGIN).expect("preparation succeeds");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan_a, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let foreign = plan_b.resolve_note(ENVELOPE).expect("the envelope plays");

    let refused = control
        .offer_note_on(
            &mut store,
            SampleTime::ZERO,
            foreign,
            common::any_key(),
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect_err("a slot naming another plan may not mint");
    assert!(matches!(
        refused,
        synth_engine_v2::ingress::IngressRefused::ForeignSlot { .. }
    ));

    // Refused *before* the mint and *before* the hold, so nothing was spent: the store's own
    // slot still admits a legitimate offer afterwards.
    let local = plan_a.resolve_note(ENVELOPE).expect("the envelope plays");
    assert!(
        control
            .offer_note_on(
                &mut store,
                SampleTime::ZERO,
                local,
                common::any_key(),
                synth_engine_v2::quantities::NoteVelocity::FULL
            )
            .is_ok(),
        "a refused foreign slot must leave the entitlement and the queue untouched"
    );
}

#[test]
fn the_horizon_pair_a_simulated_stamp_beyond_it_is_refused_and_one_inside_admitted() {
    // **ADR-0053's owed obligation, and it is owed as a pair.** Either half alone passes with
    // the predicate inverted: a fixture that only refuses would pass an implementation that
    // refuses everything, and one that only admits would pass an implementation that checks
    // nothing. The record asked for both, and until `HOST-INV-013`'s evaluation site was
    // settled there was no check to test — the maintainer settled it on 2026-09-01 on the
    // reading that the boundary admitting into bounded source storage is the one site.
    //
    // Measured from the clock the **drain** last recorded. A store whose drain has not run
    // measures from its origin, which is why both offers here are made against a fresh store.
    let plan = plan();
    let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("preparation");
    let host = common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono);
    let mut store = PerformanceIngress::prepare(&host, &plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let horizon = plan.forward_event_horizon().as_u64();
    let note = plan.resolve_note(ENVELOPE).expect("the envelope plays");

    // One sample past the horizon: refused, and the refusal names both times.
    let beyond = SampleTime::new(horizon + 1);
    match control.offer_note_on(
        &mut store,
        beyond,
        note,
        common::any_key(),
        synth_engine_v2::quantities::NoteVelocity::FULL,
    ) {
        Err(synth_engine_v2::ingress::IngressRefused::BeyondHorizon { time, horizon_end }) => {
            assert_eq!(time, beyond);
            assert_eq!(horizon_end, SampleTime::new(horizon));
        }
        other => panic!("expected a horizon refusal, got {other:?}"),
    }
    assert_eq!(
        store.counters().beyond_horizon(),
        1,
        "HOST-INV-009 requires the refusal to be counted at the boundary"
    );

    // The last stamp inside it: admitted. This is the half that fails if the check is
    // off by one in the closing direction, or if it refuses everything.
    assert!(
        control
            .offer_note_on(
                &mut store,
                SampleTime::new(horizon),
                note,
                common::any_key(),
                synth_engine_v2::quantities::NoteVelocity::FULL
            )
            .is_ok(),
        "the horizon's own sample is inside it"
    );
}
