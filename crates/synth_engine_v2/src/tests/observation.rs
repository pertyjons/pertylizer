//! `HOST-INV-023` and the Phase 5 exit gate's fifth bullet: a subscription is the host's,
//! bounded, lossy, and invisible to the plan and the render.
//!
//! Inside the crate for one refusal case — a tap index the plan has no tap for needs the
//! crate-private `TapSlot::new` to construct — and because the equivalence renders drive
//! the scheduler and renderer directly.

use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use crate::observe::{ObservationSubscriptions, SubscriptionRefused};
use crate::plan::{CompiledPlan, TapSlot};
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, SampleRate, Seconds,
};
use crate::render::{AudioBlockMut, PreparedRenderer};
use crate::schedule::{AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent};
use crate::stream::StreamControl;
use crate::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const OSCILLATOR: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const MONITOR: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(5);
const Q: usize = QUANTUM_FRAMES as usize;
const BLOCK: usize = 256;
const RING_FRAMES: u64 = 4 * QUANTUM_FRAMES as u64;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

/// How a render is observed, for the equivalence renders.
enum Observed {
    /// No store at all.
    Nobody,
    /// One subscriber that reads everything after every block.
    Reader,
    /// One subscriber that never reads, so its ring saturates and evicts.
    Saturated,
}

#[test]
fn observation_changes_no_sample_with_no_reader_one_reader_or_a_saturated_one() {
    // ADR-0027 clause 3 and the exit gate's fifth bullet: adding a subscriber, or letting one
    // saturate, changes no audio sample. The plan is the same object in all three renders —
    // a subscription reads a compiled plan and cannot change it — so what is compared is the
    // output, bit for bit, over enough quanta that the saturated ring has wrapped several
    // times.
    let plan = admit(&monitored_voice());
    let nobody = drive(&plan, Observed::Nobody, 24).output;
    let reader = drive(&plan, Observed::Reader, 24).output;
    let saturated = drive(&plan, Observed::Saturated, 24).output;
    assert!(
        nobody.iter().any(|s| *s != 0.0),
        "the voice sounds, so the comparison is not of silence"
    );
    assert_eq!(nobody, reader, "one reader changed a sample");
    assert_eq!(nobody, saturated, "a saturated reader changed a sample");
}

#[test]
fn a_reader_that_keeps_up_reads_exactly_the_frames_the_output_carried_one_quantum_early() {
    // The tap holds each quantum as it renders, and the renderer primes one quantum of
    // silence it never rendered, so the subscription is the output from frame `Q` on: one
    // quantum fewer than the caller received, and exactly those frames. Nothing is dropped
    // and nothing is left behind when the reader reads after every block.
    let plan = admit(&monitored_voice());
    let driven = drive(&plan, Observed::Reader, 24);
    let observed = driven.observed;
    let output = &driven.output;
    assert_eq!(
        observed.len(),
        output.len() - Q,
        "one frame observed per frame rendered, and the primed quantum was never rendered"
    );
    assert_eq!(
        observed,
        output[Q..],
        "the subscription is not the output from the first rendered quantum on"
    );
    assert_eq!(driven.dropped, 0, "a reader that keeps up loses nothing");
    assert_eq!(driven.behind_after_last_read, 0);
}

#[test]
fn a_saturated_reader_loses_the_oldest_frames_and_is_told_how_many() {
    // `HOST-INV-019`: lossy, and the loss exposed. The ring holds four quanta; a render of
    // twenty-four output quanta pushes twenty-three (the primed quantum is never rendered),
    // all unread. The first read then returns exactly the ring's worth — the newest four
    // quanta — and reports the nineteen evicted, which is every frame it did not keep.
    let plan = admit(&monitored_voice());
    let mut driven = drive(&plan, Observed::Saturated, 24);
    let id = driven.id.expect("the saturated render subscribed");
    let store = driven
        .store
        .as_mut()
        .expect("the saturated render has a store");
    let mut into = vec![0.0_f32; 40 * Q];
    let read = store
        .read(id, &mut into)
        .expect("the handle is this store's");
    assert_eq!(
        read.frames,
        FrameCount::new(RING_FRAMES),
        "the ring's worth, no more"
    );
    assert_eq!(
        read.dropped,
        FrameCount::new(23 * QUANTUM_FRAMES as u64 - RING_FRAMES),
        "every frame not kept was reported evicted"
    );
    assert_eq!(
        read.behind,
        FrameCount::new(0),
        "the read took everything the ring held"
    );
    // And the frames kept are the newest: the output's last ring's worth.
    let kept = &into[..RING_FRAMES as usize];
    let output = &driven.output;
    let tail_start = output.len() - RING_FRAMES as usize;
    assert_eq!(
        kept,
        &output[tail_start..],
        "the ring kept something other than the newest"
    );
    // A second read has nothing new and reports no new loss.
    let again = store
        .read(id, &mut into)
        .expect("the handle is this store's");
    assert_eq!(again.frames, FrameCount::new(0));
    assert_eq!(again.dropped, FrameCount::new(0));
}

#[test]
fn a_subscription_is_admitted_against_the_plans_taps_and_refused_by_name() {
    // A slot of another plan, an index the plan has no tap for, and a second subscriber on
    // one tap are refused rather than ignored; a refusal changes nothing — the store still
    // holds what it held.
    let plan = admit(&monitored_voice());
    let other = admit(&monitored_voice());
    let mut store = ObservationSubscriptions::prepare(&profile(), &plan);
    let slot = plan
        .resolve_tap(MONITOR, PortId::FIRST)
        .expect("the monitor declares a tap");
    let foreign = other
        .resolve_tap(MONITOR, PortId::FIRST)
        .expect("the other plan's monitor declares a tap");
    assert_eq!(
        store.subscribe(&plan, foreign),
        Err(SubscriptionRefused::ForeignPlan {
            slot: other.id(),
            plan: plan.id(),
            store: plan.id(),
        })
    );
    // And the other mismatch: the store's own slot, offered with another plan. Both plans
    // are named, so the diagnostic says which argument did not match.
    assert_eq!(
        store.subscribe(&other, slot),
        Err(SubscriptionRefused::ForeignPlan {
            slot: plan.id(),
            plan: other.id(),
            store: plan.id(),
        })
    );
    assert!(matches!(
        store.subscribe(&plan, TapSlot::new(plan.id(), 7)),
        Err(SubscriptionRefused::UnknownTap { index: 7, .. })
    ));
    assert!(store.is_empty(), "a refusal subscribed nothing");
    let id = store
        .subscribe(&plan, slot)
        .expect("the plan's own tap is admitted");
    assert_eq!(id.index(), 0);
    assert_eq!(
        store.subscribe(&plan, slot),
        Err(SubscriptionRefused::AlreadySubscribed { index: 0 })
    );
    assert_eq!(store.len(), 1);
    assert_eq!(store.channels(id), Some(1));
    // A handle is the issuing store's: a second store for the same plan issues index zero
    // too, and this store's handle names nothing in it.
    let mut second = ObservationSubscriptions::prepare(&profile(), &plan);
    let second_id = second
        .subscribe(&plan, slot)
        .expect("admitted in its own store");
    assert_eq!(second_id.index(), id.index());
    assert_ne!(second_id, id, "two stores' handles are distinct");
    assert_eq!(second.channels(id), None);
    assert_eq!(second.read(id, &mut [0.0; 4]), None);
    // A telemetry window no address space holds is refused before anything is allocated.
    let huge = profile_with_ring(FrameCount::new(u64::MAX));
    let mut unrepresentable = ObservationSubscriptions::prepare(&huge, &plan);
    assert_eq!(
        unrepresentable.subscribe(&plan, slot),
        Err(SubscriptionRefused::RingUnrepresentable {
            frames: FrameCount::new(u64::MAX),
        })
    );
    // A store prepared for the other plan receives nothing from this renderer: its handle
    // reads no frames after a render it was handed to.
    let mut foreign_store = ObservationSubscriptions::prepare(&profile(), &other);
    let foreign_id = foreign_store
        .subscribe(&other, foreign)
        .expect("the other plan's tap is admitted against its own store");
    let mut renderer = Renderer::open(&plan);
    renderer.drive(4 * Q, Some(&mut foreign_store));
    let mut into = vec![0.0_f32; Q];
    let read = foreign_store
        .read(foreign_id, &mut into)
        .expect("the handle is this store's");
    assert_eq!(
        read.frames,
        FrameCount::new(0),
        "a foreign store was pushed into"
    );
}

#[test]
fn an_activation_adopted_inside_a_block_loses_the_observer_no_quantum() {
    // The scheduler splits a host block at an activation boundary and renders the two halves
    // as two calls. The observers are handed to both: a reader that keeps up still reads
    // every rendered quantum, block boundaries and activation boundaries alike. The seek
    // lands mid-block — the block is four quanta and the boundary is at six — so the split
    // path is the one exercised.
    let plan = admit(&monitored_voice());
    let mut renderer = Renderer::open(&plan);
    let mut store = ObservationSubscriptions::prepare(&profile(), &plan);
    let slot = plan
        .resolve_tap(MONITOR, PortId::FIRST)
        .expect("the monitor declares a tap");
    let id = store.subscribe(&plan, slot).expect("the tap is admitted");

    let stream = AdmittedCompiledStream::admit(&plan, &events(&plan)).expect("the stream fits");
    let activation = renderer
        .control
        .plan_activation(
            &stream,
            crate::stream::ActivationRequest {
                at: SampleTime::new(6 * QUANTUM_FRAMES as u64),
                position: PlanPosition::new(16 * QUANTUM_FRAMES as u64),
                loop_interval: None,
            },
        )
        .expect("the seek builds");
    renderer
        .scheduler
        .offer(&mut renderer.renderer, activation)
        .expect("the offer is accepted");

    let mut output = Vec::new();
    let mut observed = Vec::new();
    for _ in 0..6 {
        output.extend_from_slice(&renderer.drive(BLOCK, Some(&mut store)));
        let mut into = vec![0.0_f32; 2 * BLOCK];
        let read = store
            .read(id, &mut into)
            .expect("the handle is this store's");
        let frames = usize::try_from(read.frames.as_u64()).expect("fits");
        observed.extend_from_slice(&into[..frames]);
        assert_eq!(read.dropped, FrameCount::new(0));
    }
    assert_eq!(
        observed.len(),
        output.len() - Q,
        "a quantum was rendered unobserved"
    );
    assert_eq!(
        observed,
        output[Q..],
        "the observer read something other than the render"
    );
}

// --- fixtures -----------------------------------------------------------------------------

fn profile() -> HostProfile {
    profile_with_ring(FrameCount::new(RING_FRAMES))
}

/// The harness profile with the telemetry window overridden.
fn profile_with_ring(ring: FrameCount) -> HostProfile {
    let base = HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(BLOCK as u64),
        ChannelLayout::Mono,
    )
    .expect("valid harness profile");
    // A four-quantum telemetry window, so a saturated reader is observable in a short render;
    // every other group keeps the profile's defaults.
    let limits = base.limits();
    let observation = crate::profile::ObservationLimits::new(
        limits.observation().max_observation_taps(),
        ring,
        limits.observation().analyzer_fft_size(),
    )
    .expect("the overridden capacities are above zero");
    let limits = crate::profile::RenderLimits::new(
        limits.stream(),
        limits.graph(),
        limits.voices(),
        limits.events(),
        observation,
        limits.mixing(),
        limits.memory(),
        limits.script(),
        limits.recording(),
        limits.cost(),
    )
    .expect("the overridden limits are internally consistent");
    HostProfile::new(base.capabilities(), limits).expect("the profile is consistent")
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    compile(ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("the plan fits this profile")
}

/// The smallest real voice with a monitor before its output.
fn monitored_voice() -> GraphIr {
    GraphIr::builder()
        .node(
            OSCILLATOR,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::UNITY,
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: crate::quantities::NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(MONITOR, IrNodeKind::Monitor, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (MONITOR, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (MONITOR, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().expect("12-TET prepares"),
        )
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(4),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(4),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan")
}

/// A note held from the start, so every quantum after the first sounds.
fn events(plan: &CompiledPlan) -> Vec<PlanEvent> {
    let note = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    vec![PlanEvent::new(
        PlanPosition::ZERO,
        CompiledPayload::NoteOn {
            slot: note,
            key: KeyIdentity::new(69).expect("A4"),
            velocity: NoteVelocity::FULL,
        },
    )]
}

/// A stream's audio half with the scheduler and arbiter that drive it.
struct Renderer {
    renderer: PreparedRenderer,
    scheduler: CompiledEventScheduler,
    arbiter: PublicationArbiter,
    control: StreamControl,
}

impl Renderer {
    fn open(plan: &CompiledPlan) -> Self {
        let (mut control, renderer) =
            StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
        let stream = AdmittedCompiledStream::admit(plan, &events(plan)).expect("the stream fits");
        let scheduler =
            CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
        let arbiter = PublicationArbiter::prepare(&profile()).expect("the store is preparable");
        Self {
            renderer,
            scheduler,
            arbiter,
            control,
        }
    }

    /// Render `frames` in host blocks, handing the store to every call.
    fn drive(
        &mut self,
        frames: usize,
        mut store: Option<&mut ObservationSubscriptions>,
    ) -> Vec<f32> {
        let mut out = Vec::new();
        let mut done = 0;
        while done < frames {
            let this = BLOCK.min(frames - done);
            let mut samples = vec![0.0_f32; this];
            let output = AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono)
                .expect("a shaped block");
            self.scheduler
                .render_observed(
                    &mut self.renderer,
                    &mut self.arbiter,
                    None,
                    store.as_deref_mut(),
                    output,
                )
                .expect("the stream renders");
            out.extend_from_slice(&samples);
            done += this;
        }
        out
    }
}

/// What one observed render produced.
struct Driven {
    output: Vec<f32>,
    /// Every frame the reader read, in order; empty for the other two modes.
    observed: Vec<f32>,
    dropped: u64,
    behind_after_last_read: u64,
    store: Option<ObservationSubscriptions>,
    id: Option<crate::observe::SubscriptionId>,
}

/// Render `quanta` quanta of the plan under one observation mode, block by block.
fn drive(plan: &CompiledPlan, mode: Observed, quanta: usize) -> Driven {
    let mut renderer = Renderer::open(plan);
    let (mut store, id) = match mode {
        Observed::Nobody => (None, None),
        Observed::Reader | Observed::Saturated => {
            let mut store = ObservationSubscriptions::prepare(&profile(), plan);
            let slot = plan
                .resolve_tap(MONITOR, PortId::FIRST)
                .expect("the monitor declares a tap");
            let id = store.subscribe(plan, slot).expect("the tap is admitted");
            (Some(store), Some(id))
        }
    };
    let mut output = Vec::new();
    let mut observed = Vec::new();
    let mut dropped = 0_u64;
    let mut behind = 0_u64;
    let mut done = 0;
    let frames = quanta * Q;
    while done < frames {
        let this = BLOCK.min(frames - done);
        output.extend_from_slice(&renderer.drive(this, store.as_mut()));
        done += this;
        if let (Observed::Reader, Some(store), Some(id)) = (&mode, store.as_mut(), id) {
            let mut into = vec![0.0_f32; 2 * BLOCK];
            let read = store
                .read(id, &mut into)
                .expect("the handle is this store's");
            let frames_read = usize::try_from(read.frames.as_u64()).expect("fits");
            observed.extend_from_slice(&into[..frames_read]);
            dropped += read.dropped.as_u64();
            behind = read.behind.as_u64();
        }
    }
    Driven {
        output,
        observed,
        dropped,
        behind_after_last_read: behind,
        store,
        id,
    }
}
