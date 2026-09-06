//! Determinism under pressure — `P06-S007`, the phase's exit evidence.
//!
//! Phase 6's first gate bullet asks for polyphonic output that is deterministic for a fixed
//! event stream under stealing pressure, and its roadmap exit for equivalent offline and
//! live instance behaviour. This file holds the falsifiable half: a dense set of overlapping
//! notes on a two-voice plan under the oldest-voice policy, where every note-on after the
//! second takes a voice, fades it and starts when the fade ends, renders **bit-identically**
//! run to run, across every host block partition, through the offline render, and through
//! the live boundary — and the report's counts agree with the renders.
//!
//! What it does not hold is a **project seed**: nothing in V2 consumes randomness, no node
//! kind has a seed, and what a seed is belongs to Phase 7's ADR-0008. The exit review
//! carries that clause as a named residual rather than claiming it.

mod common;
use synth_engine_v2::identity::ProducerId;
use synth_engine_v2::ingress::PerformanceIngress;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain, StealingPolicy,
};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::publish::PublicationArbiter;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, EventCount, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, Seconds,
};
use synth_engine_v2::render::AudioBlockMut;
use synth_engine_v2::schedule::{
    AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent,
};
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const Q: u64 = QUANTUM_FRAMES as u64;
const TOTAL: usize = 32 * Q as usize;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
const ONLY_PRODUCER: ProducerId = ProducerId::new(0);
const FADE: FrameCount = FrameCount::new(128);

/// The host block partitions the compiled path is rendered under: one call, the harness's
/// block, the quantum, and an uneven partition that straddles every boundary shape.
const WHOLE: [usize; 1] = [TOTAL];
const BLOCKS_256: [usize; 8] = [256; 8];
const BLOCKS_64: [usize; 32] = [64; 32];
const IRREGULAR: [usize; 8] = [17, 511, 3, 64, 1024, 1, 100, 328];

/// One note's edges as both paths state them: `releases` names the `n`th note-on offered.
#[derive(Clone, Copy)]
struct Edge {
    at: u64,
    key: u8,
    releases: Option<usize>,
}

const fn on(at: u64, key: u8) -> Edge {
    Edge {
        at,
        key,
        releases: None,
    }
}

const fn off(at: u64, of: usize) -> Edge {
    Edge {
        at,
        key: 0,
        releases: Some(of),
    }
}

/// Five overlapping notes on two voices, every one after the second taking a voice while
/// the previous victim's fade is still running, then every release, then a sixth note once
/// every voice is free. The densest pressure the live-versus-compiled fixtures already hold
/// (`simulated_ingress`), reused here so the three claims are about one stream.
const PRESSURE: [Edge; 12] = [
    on(0, 60),
    on(2 * Q, 67),
    on(4 * Q + 5, 72),
    on(6 * Q + 5, 75),
    on(8 * Q + 5, 79),
    off(20 * Q, 0),
    off(21 * Q, 1),
    off(22 * Q, 2),
    off(23 * Q, 3),
    off(24 * Q, 4),
    on(26 * Q, 84),
    off(27 * Q, 5),
];

fn key(raw: u8) -> KeyIdentity {
    KeyIdentity::new(raw).expect("a keyboard position")
}

fn oldest() -> StealingPolicy {
    StealingPolicy::Oldest { fade: FADE }
}

fn compiled_declarations() -> PlanDeclarations {
    PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(2),
            simultaneous_holds: EventCount::NONE,
        }],
        held_notes: HeldNoteCount::measured(2),
        stealing: oldest(),
        ..PlanDeclarations::default()
    }
}

fn live_declarations() -> PlanDeclarations {
    PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(2),
            simultaneous_holds: EventCount::measured(2),
        }],
        held_notes: HeldNoteCount::measured(2),
        stealing: oldest(),
        ..PlanDeclarations::default()
    }
}

/// A pitched voice — sine through an envelope-driven amplifier — so a note's key, its
/// instance and its steal are all audible in the samples.
fn voice(declarations: PlanDeclarations) -> CompiledPlan {
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: synth_engine_v2::quantities::Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(0.25).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.002).expect("finite"),
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.004).expect("finite"),
                velocity_sensitivity: NormalizedLevel::FULL,
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
        .tuning(ExecutionScope::Voice, common::twelve_tet())
        .declaring(declarations)
        .build()
        .expect("a readable plan");
    common::admit(&ir, common::profile(TOTAL as u64, ChannelLayout::Mono))
}

fn compiled_events(plan: &CompiledPlan, edges: &[Edge]) -> Vec<PlanEvent> {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let mut keys: Vec<u8> = Vec::new();
    edges
        .iter()
        .map(|edge| {
            let payload = match edge.releases {
                None => {
                    keys.push(edge.key);
                    CompiledPayload::NoteOn {
                        slot,
                        key: key(edge.key),
                        velocity: NoteVelocity::FULL,
                    }
                }
                Some(of) => CompiledPayload::NoteOff {
                    slot,
                    key: key(keys[of]),
                },
            };
            PlanEvent::new(PlanPosition::new(edge.at), payload)
        })
        .collect()
}

/// The compiled path under a host partition, with the scheduler's counts.
fn render_compiled(plan: &CompiledPlan, partition: &[usize]) -> (Vec<f32>, usize) {
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let events = compiled_events(plan, &PRESSURE);
    let stream = AdmittedCompiledStream::admit(plan, &events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter =
        PublicationArbiter::prepare(&common::profile(TOTAL as u64, ChannelLayout::Mono))
            .expect("the store is preparable");
    let mut out = Vec::with_capacity(TOTAL);
    for block in partition.iter().copied() {
        let mut samples = vec![0.0_f32; block];
        let output =
            AudioBlockMut::new(&mut samples, block, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(&mut renderer, &mut arbiter, output)
            .expect("the stream renders");
        out.extend_from_slice(&samples);
    }
    assert_eq!(out.len(), TOTAL, "the partition covers the render");
    (out, scheduler.released_after_steal())
}

/// The offline path over the same events: `render_offline` drives the same renderer at
/// the plan's block size and trims the priming quantum.
fn render_offline(plan: &CompiledPlan) -> Vec<f32> {
    let events: Vec<synth_engine_v2::offline::OfflineEvent> = compiled_events(plan, &PRESSURE)
        .into_iter()
        .map(|event| {
            synth_engine_v2::offline::OfflineEvent::new(
                SampleTime::new(event.position().as_u64()),
                event.payload(),
            )
        })
        .collect();
    synth_engine_v2::offline::render_offline(
        plan.clone(),
        FrameCount::new(TOTAL as u64),
        PlanPosition::ZERO,
        &events,
    )
    .expect("the offline render succeeds")
}

/// The live path: every edge offered at its own quantum through the ingress store, one
/// quantum rendered at a time, because a taken voice becomes takeable again only once the
/// drain has published its deferred start (`simulated_ingress` records why).
fn render_live(plan: &CompiledPlan) -> (Vec<f32>, u64) {
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let empty = AdmittedCompiledStream::admit(plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(TOTAL as u64, ChannelLayout::Mono);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("the store is preparable");
    let mut store = PerformanceIngress::prepare(&host, plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let mut identities = Vec::new();
    let mut out = Vec::with_capacity(TOTAL);
    let mut done = 0_u64;
    let mut next = 0;
    while (done as usize) < TOTAL {
        while let Some(edge) = PRESSURE.get(next).copied()
            && edge.at < done + Q
        {
            next += 1;
            match edge.releases {
                None => {
                    let identity = control
                        .offer_note_on(
                            &mut store,
                            SampleTime::new(edge.at),
                            slot,
                            key(edge.key),
                            NoteVelocity::FULL,
                        )
                        .expect("the note-on is admitted");
                    identities.push(identity);
                }
                Some(of) => {
                    control
                        .offer_note_off(&mut store, SampleTime::new(edge.at), identities[of])
                        .expect("the release is admitted");
                }
            }
        }
        let this = (Q as usize).min(TOTAL - done as usize);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render_with_ingress(&mut renderer, &mut arbiter, Some(&mut store), output)
            .expect("the pass publishes");
        out.extend_from_slice(&samples);
        done += this as u64;
    }
    (out, store.counters().released_after_steal())
}

fn assert_same(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: lengths differ");
    if let Some(frame) = a
        .iter()
        .zip(b)
        .position(|(x, y)| x.to_bits() != y.to_bits())
    {
        panic!(
            "{what}: frame {frame} (quantum {}, offset {}): {} against {}",
            frame / Q as usize,
            frame % Q as usize,
            a[frame],
            b[frame]
        );
    }
}

#[test]
fn a_polyphonic_render_under_stealing_pressure_is_bit_identical_run_to_run_and_across_partitions() {
    // Two renders of one plan over one event stream are the same bits, and so is every
    // host partition's — one call, the harness's block, the quantum, and an uneven one —
    // which is what rules out a steal, a fade or a deferred start that depended on where
    // a host block boundary fell. The stream is under pressure by construction: three of
    // its six note-ons take a voice, and the count says so.
    let plan = voice(compiled_declarations());
    let (first, taken) = render_compiled(&plan, &WHOLE);
    assert_eq!(
        taken, 3,
        "the third, fourth and fifth notes each took a voice"
    );
    assert!(
        first.iter().filter(|s| **s != 0.0).count() > TOTAL / 2,
        "the voices sound through most of the render"
    );
    // The second run is of a **freshly compiled** plan, so the claim covers preparation
    // as well as rendering: a state seeded at compile time would pass a rerun of one plan.
    let (again, taken_again) = render_compiled(&voice(compiled_declarations()), &WHOLE);
    assert_same(
        &again,
        &first,
        "a second run of a freshly compiled plan and one stream",
    );
    assert_eq!(taken_again, taken);
    for (index, partition) in [&BLOCKS_256[..], &BLOCKS_64[..], &IRREGULAR[..]]
        .into_iter()
        .enumerate()
    {
        let (rendered, taken_here) = render_compiled(&plan, partition);
        assert_same(&rendered, &first, &format!("host partition {index}"));
        assert_eq!(
            taken_here, taken,
            "host partition {index} changed how many notes stole"
        );
    }
}

#[test]
fn the_offline_render_under_stealing_pressure_is_the_compiled_streams_render() {
    // The offline path drives the same renderer over the same stamped events, so under the
    // same pressure it is the same bits, one quantum of priming apart.
    let plan = voice(compiled_declarations());
    let (compiled, taken) = render_compiled(&plan, &WHOLE);
    assert_eq!(
        taken, 3,
        "the stream is under pressure: three notes took a voice"
    );
    assert!(
        compiled.iter().filter(|s| **s != 0.0).count() > TOTAL / 2,
        "the voices sound through most of the render"
    );
    let offline = render_offline(&plan);
    assert_same(
        &offline[..TOTAL - Q as usize],
        &compiled[Q as usize..],
        "offline against the compiled stream",
    );
    let again = render_offline(&plan);
    assert_same(&again, &offline, "a second offline run");
}

#[test]
fn the_live_boundary_under_stealing_pressure_is_bit_identical_run_to_run_and_to_the_compiled_stream()
 {
    // Equivalent offline and live instance behaviour: the same edges offered live take the
    // same voices, fade the same victims and start at the same displaced positions as the
    // compiled stream, so the two renders are the same bits — and a second live run is too.
    let compiled_plan = voice(compiled_declarations());
    let (compiled, taken) = render_compiled(&compiled_plan, &WHOLE);
    assert_eq!(
        taken, 3,
        "the stream is under pressure: three notes took a voice"
    );
    assert!(
        compiled.iter().filter(|s| **s != 0.0).count() > TOTAL / 2,
        "the voices sound through most of the render"
    );
    let live_plan = voice(live_declarations());
    let (live, released_after_steal) = render_live(&live_plan);
    assert_same(&live, &compiled, "live against the compiled stream");
    assert_eq!(
        released_after_steal, 3,
        "the boundary dropped one release per note a steal ended"
    );
    let (again, released_again) = render_live(&live_plan);
    assert_same(&again, &live, "a second live run");
    assert_eq!(released_again, released_after_steal);
}
