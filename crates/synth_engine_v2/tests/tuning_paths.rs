//! One tuning through every path — `P06-S006`.
//!
//! Phase 6's gate: built-in twelve-tone equal temperament and one non-twelve-tone mapping
//! produce the same pitches through the live, sequenced, offline and analysis-facing paths.
//! `SOUND-INV-021` already makes this structural — a key becomes a frequency in exactly one
//! place, `CompiledPlan::magnitude_value`, through the prepared tuning the scope states —
//! and this file holds every consumer to it under nineteen-tone equal temperament, where a
//! path converting the key itself would be audibly wrong. The analysis-facing path is the
//! observation tap (`SOUND-INV-022`), the one surface an analyzer reads rendered signal
//! through in this crate.
//!
//! It also holds the case `P06-S005` handed here: a sampler under a seek is silent from the
//! boundary rather than playing on across it. Which mechanism lowers its trigger on the
//! compiled path — the catch-up's restore of every target, not the boundary release's
//! explicit sites — is what the combined mutations in `SOUND-INV-026`'s row established.

mod common;
use synth_engine_v2::identity::ProducerId;
use synth_engine_v2::ingress::PerformanceIngress;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use synth_engine_v2::observe::ObservationSubscriptions;
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::publish::PublicationArbiter;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, EventCount, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, Seconds,
};
use synth_engine_v2::render::AudioBlockMut;
use synth_engine_v2::sample::{
    PlayDirection, PlayMode, PlaybackRegion, PreparedSample, SUSTAIN_FADE_FRAMES, SampleFrame,
    SampleMap, SampleMapRef, SampleRef, SampleZone,
};
use synth_engine_v2::schedule::{
    AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent,
};
use synth_engine_v2::stream::{ActivationRequest, StreamControl};
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};
use synth_engine_v2::tuning::PreparedTuning;

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const MONITOR: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(5);
const Q: u64 = QUANTUM_FRAMES as u64;
const BLOCK: usize = 256;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
const ONLY_PRODUCER: ProducerId = ProducerId::new(0);
const KEY: u8 = 72;

fn key(raw: u8) -> KeyIdentity {
    KeyIdentity::new(raw).expect("a keyboard position")
}

fn live_declarations() -> PlanDeclarations {
    PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(2),
            simultaneous_holds: EventCount::measured(2),
        }],
        ..PlanDeclarations::default()
    }
}

/// The smallest pitched voice, under a stated tuning, with a monitor on its output so the
/// same render is readable through a tap.
fn voice(tuning: PreparedTuning, declarations: PlanDeclarations) -> CompiledPlan {
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
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(MONITOR, IrNodeKind::Monitor, ExecutionScope::Voice)
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
            (MONITOR, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (MONITOR, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(ExecutionScope::Voice, tuning)
        .declaring(declarations)
        .build()
        .expect("a readable plan");
    common::admit(&ir, common::profile(BLOCK as u64, ChannelLayout::Mono))
}

fn note_events(plan: &CompiledPlan, on: u64, off: u64) -> Vec<PlanEvent> {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    vec![
        PlanEvent::new(
            PlanPosition::new(on),
            CompiledPayload::NoteOn {
                slot,
                key: key(KEY),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(off),
            CompiledPayload::NoteOff {
                slot,
                key: key(KEY),
            },
        ),
    ]
}

/// The sequenced path: a compiled stream through the scheduler, block by block.
fn render_compiled(plan: &CompiledPlan, events: &[PlanEvent], frames: usize) -> Vec<f32> {
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(plan, events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter =
        PublicationArbiter::prepare(&common::profile(BLOCK as u64, ChannelLayout::Mono))
            .expect("the store is preparable");
    let mut out = Vec::new();
    let mut done = 0;
    while done < frames {
        let this = BLOCK.min(frames - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(&mut renderer, &mut arbiter, output)
            .expect("the stream renders");
        out.extend_from_slice(&samples);
        done += this;
    }
    out
}

/// The offline path: `render_offline` over the same events, which trims the stream's
/// priming quantum, so its frame `k` is the stream's frame `k + Q`.
fn render_offline(plan: &CompiledPlan, on: u64, off: u64, frames: u64) -> Vec<f32> {
    let events: Vec<synth_engine_v2::offline::OfflineEvent> = note_events(plan, on, off)
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
        FrameCount::new(frames),
        PlanPosition::ZERO,
        &events,
    )
    .expect("the offline render succeeds")
}

/// The live path: the note offered at the boundary, one quantum at a time, through the
/// ingress store; the compiled stream is empty so every edge came through the boundary.
fn render_live(plan: &CompiledPlan, on: u64, off: u64, frames: usize) -> Vec<f32> {
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let empty = AdmittedCompiledStream::admit(plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &empty).expect("the schedule is valid");
    let host = common::profile(BLOCK as u64, ChannelLayout::Mono);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("the store is preparable");
    let mut store = PerformanceIngress::prepare(&host, plan, ONLY_PRODUCER, &renderer)
        .expect("the live producer has a store");
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let mut identity = None;
    let mut out = Vec::new();
    let mut done = 0_u64;
    while (done as usize) < frames {
        if done == on {
            identity = Some(
                control
                    .offer_note_on(
                        &mut store,
                        SampleTime::new(on),
                        slot,
                        key(KEY),
                        NoteVelocity::FULL,
                    )
                    .expect("the note-on is admitted"),
            );
        }
        if done == off
            && let Some(identity) = identity
        {
            control
                .offer_note_off(&mut store, SampleTime::new(off), identity)
                .expect("the release is admitted");
        }
        let this = (Q as usize).min(frames - done as usize);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render_with_ingress(&mut renderer, &mut arbiter, Some(&mut store), output)
            .expect("the pass publishes");
        out.extend_from_slice(&samples);
        done += this as u64;
    }
    out
}

/// The analysis-facing path: the same render with a subscription on the monitor's tap,
/// read after every block, returned beside the output it observed.
fn render_observed(
    plan: &CompiledPlan,
    events: &[PlanEvent],
    frames: usize,
) -> (Vec<f32>, Vec<f32>) {
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(plan, events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let host = common::profile(BLOCK as u64, ChannelLayout::Mono);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("the store is preparable");
    let mut store = ObservationSubscriptions::prepare(&host, plan);
    let tap = plan
        .resolve_tap(MONITOR, PortId::FIRST)
        .expect("the monitor declares a tap");
    let id = store.subscribe(plan, tap).expect("the tap is admitted");
    let mut out = Vec::new();
    let mut observed = Vec::new();
    let mut done = 0;
    while done < frames {
        let this = BLOCK.min(frames - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render_observed(&mut renderer, &mut arbiter, None, Some(&mut store), output)
            .expect("the stream renders");
        out.extend_from_slice(&samples);
        done += this;
        let mut into = vec![0.0_f32; 2 * BLOCK];
        let read = store
            .read(id, &mut into)
            .expect("the handle is this store's");
        assert_eq!(read.dropped.as_u64(), 0, "the reader kept up");
        let frames_read = usize::try_from(read.frames.as_u64()).expect("fits");
        observed.extend_from_slice(&into[..frames_read]);
    }
    (out, observed)
}

fn crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count()
}

fn assert_same(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}: lengths differ");
    if let Some(frame) = a.iter().zip(b).position(|(x, y)| x != y) {
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
fn the_sequenced_offline_and_live_paths_render_one_key_the_same_under_nineteen_tet() {
    // The three paths that render: the compiled stream, the offline render over the same
    // events, and the live boundary offering the same edges. Under nineteen-tone the three
    // are the same samples, and all three differ from the twelve-tone render — the
    // control that makes the agreement say something, since three paths ignoring the
    // tuning would agree just as well.
    const FRAMES: u64 = 32 * Q;
    let on = 0;
    let off = 20 * Q;
    let nineteen = voice(common::nineteen_tet(), common::compiled_notes(2));
    let compiled = render_compiled(&nineteen, &note_events(&nineteen, on, off), FRAMES as usize);
    let offline = render_offline(&nineteen, on, off, FRAMES);
    // The offline render trims the priming quantum: its frame `k` is the stream's `k + Q`.
    assert_same(
        &offline[..(FRAMES - Q) as usize],
        &compiled[Q as usize..],
        "offline against the compiled stream",
    );
    let live = render_live(
        &voice(common::nineteen_tet(), live_declarations()),
        on,
        off,
        FRAMES as usize,
    );
    assert_same(&live, &compiled, "live against the compiled stream");
    let twelve = voice(common::twelve_tet(), common::compiled_notes(2));
    let under_twelve = render_compiled(&twelve, &note_events(&twelve, on, off), FRAMES as usize);
    assert!(
        compiled.iter().zip(&under_twelve).any(|(a, b)| a != b),
        "nineteen-tone rendered the twelve-tone samples, so the tuning reached no path"
    );
    assert!(
        crossings(&compiled) > 0 && crossings(&compiled) != crossings(&under_twelve),
        "the two tunings render two pitches: {} against {} crossings",
        crossings(&compiled),
        crossings(&under_twelve)
    );
    // And two keys under one tuning render two pitches — the control the tunings alone
    // cannot supply, because the two tables differ at every key including the reference,
    // so a resolution that ignored the key would still pass the assertion above. Held
    // exactly at the plan, where the key becomes a frequency, and in the samples.
    let slot = nineteen
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let pitch = nineteen
        .note_magnitudes_of(slot)
        .iter()
        .find(|magnitude| magnitude.magnitude == synth_engine_v2::node::NoteMagnitude::Pitch)
        .copied()
        .expect("the voice declares a pitch destination");
    let resolved = |raw: u8| {
        nineteen
            .magnitude_value(&pitch, key(raw), NoteVelocity::FULL)
            .expect("the plan resolves its own key")
            .as_f32()
    };
    assert_ne!(
        resolved(72),
        resolved(60),
        "two keys resolved to one frequency"
    );
    let lower_events = vec![
        PlanEvent::new(
            PlanPosition::new(on),
            CompiledPayload::NoteOn {
                slot,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(off),
            CompiledPayload::NoteOff { slot, key: key(60) },
        ),
    ];
    let lower = render_compiled(&nineteen, &lower_events, FRAMES as usize);
    assert!(
        crossings(&lower) > 0 && crossings(&lower) != crossings(&compiled),
        "two keys under nineteen-tone rendered one pitch: {} against {} crossings",
        crossings(&lower),
        crossings(&compiled)
    );
}

#[test]
fn the_observation_tap_reads_the_pitch_the_output_carries_under_nineteen_tet() {
    // The analysis-facing path: a tap on the voice's output, read block by block, holds
    // exactly the samples the output carried — so an analyzer reading the tap sees the
    // tuning's pitch and not a second resolution of the key.
    const FRAMES: usize = 24 * Q as usize;
    let plan = voice(common::nineteen_tet(), common::compiled_notes(2));
    let (out, observed) = render_observed(&plan, &note_events(&plan, 0, 20 * Q), FRAMES);
    assert!(out.iter().any(|s| *s != 0.0), "the voice sounds");
    // The tap holds the signal at plan time and the output is the stream's carry, one
    // quantum behind it: the tap's frame `k` is the output's frame `k + Q`.
    assert_eq!(
        observed.len() + Q as usize,
        out.len(),
        "one quantum of carry"
    );
    assert_same(&observed, &out[Q as usize..], "the tap against the output");
    let twelve = voice(common::twelve_tet(), common::compiled_notes(2));
    let (_, under_twelve) = render_observed(&twelve, &note_events(&twelve, 0, 20 * Q), FRAMES);
    assert!(
        crossings(&under_twelve) > 0,
        "the twelve-tone render reached the tap"
    );
    assert_ne!(
        crossings(&observed),
        crossings(&under_twelve),
        "the tap read one pitch under two tunings"
    );
}

/// A seek past a note released before the destination, with automation opening the gate
/// at the destination: what the catch-up restores is the last note's key, resolved through
/// the tuning the plan states (ADR-0051 clause 1, `SOUND-INV-021`).
fn render_after_seek(tuning: PreparedTuning) -> Vec<f32> {
    let plan = voice(tuning, common::compiled_notes(2));
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let host = common::profile(BLOCK as u64, ChannelLayout::Mono);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("the store is preparable");
    let quiet = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");
    let gate = plan
        .resolve_parameter(ENVELOPE, synth_engine_v2::ir::parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate parameter");
    let mut events = note_events(&plan, 0, 2 * Q);
    events.push(PlanEvent::new(
        PlanPosition::new(8 * Q),
        CompiledPayload::SetParameter {
            slot: gate,
            value: synth_engine_v2::quantities::ParameterValue::ONE,
        },
    ));
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let activation = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(4 * Q),
                position: PlanPosition::new(8 * Q),
                loop_interval: None,
            },
        )
        .expect("the seek builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");
    let mut out = Vec::new();
    let frames = 32 * Q as usize;
    let mut done = 0;
    while done < frames {
        let this = (Q as usize).min(frames - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(&mut renderer, &mut arbiter, output)
            .expect("the stream renders");
        out.extend_from_slice(&samples);
        done += this;
    }
    out
}

#[test]
fn a_locate_restores_the_pitch_through_the_tuning_the_plan_states() {
    // The activation catch-up under two tunings: the restored note sounds at the pitch its
    // key has under the plan's tuning, and the two tunings restore two pitches.
    let nineteen = render_after_seek(common::nineteen_tet());
    let twelve = render_after_seek(common::twelve_tet());
    assert!(
        crossings(&nineteen) > 0 && crossings(&twelve) > 0,
        "the automation opened the gate under both tunings"
    );
    assert_ne!(
        crossings(&nineteen),
        crossings(&twelve),
        "a seek under two tunings restored one pitch, so the catch-up resolved the key \
         through something other than the plan's tuning"
    );
    // And the restored pitch is the tuning's own: the straight render of the same key under
    // nineteen-tone crosses zero at the same rate, within the phase the seek starts at.
    let plan = voice(common::nineteen_tet(), common::compiled_notes(2));
    let straight = render_compiled(&plan, &note_events(&plan, 0, 40 * Q), 32 * Q as usize);
    let from = 9 * Q as usize;
    let seek_rate = crossings(&nineteen[from..]) as f64 / (nineteen.len() - from) as f64;
    let straight_rate = crossings(&straight[from..]) as f64 / (straight.len() - from) as f64;
    assert!(
        (seek_rate - straight_rate).abs() * (nineteen.len() - from) as f64 <= 2.0,
        "the seek restored a pitch other than the straight render's: {seek_rate} against \
         {straight_rate} crossings per frame"
    );
}

/// A sampler feeding the output directly, so what its trigger does is audible on its own.
fn sampler_voice() -> CompiledPlan {
    const FRAMES: usize = 32_768;
    let frames: Vec<f32> = (0..FRAMES)
        .map(|i| ((i * 7_919) % 1_000) as f32 / 1_000.0 - 0.5)
        .collect();
    let sample = PreparedSample::prepare(frames, ChannelLayout::Mono, common::rate(48_000.0))
        .expect("the sample prepares");
    let region =
        PlaybackRegion::new(SampleFrame::FIRST, SampleFrame::new(FRAMES as u32)).expect("a region");
    let zone = SampleZone::new(SampleRef::new(0), key(60), region);
    let ir = GraphIr::builder()
        .sample(sample)
        .sample_map(SampleMap::new(vec![zone]))
        .node(
            SOURCE,
            IrNodeKind::Sampler {
                map: SampleMapRef::new(0),
                level: Amplitude::UNITY,
                velocity_sensitivity: NormalizedLevel::FULL,
                start_offset: NormalizedLevel::ZERO,
                play_mode: PlayMode::Sustain,
                direction: PlayDirection::Forward,
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
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(ExecutionScope::Voice, common::twelve_tet())
        .declaring(common::compiled_notes(2))
        .build()
        .expect("a readable plan");
    common::admit(&ir, common::profile(BLOCK as u64, ChannelLayout::Mono))
}

#[test]
fn a_seek_past_a_sounding_sampler_note_silences_it_from_the_boundary() {
    // The case `P06-S005` handed here: a sampler whose note crosses the seek is silent from
    // the boundary — its trigger falls there and V1's 512-frame fade ends it — rather than
    // playing on across the seek. This measures the behaviour, not one mechanism: on the
    // compiled path it is the catch-up's restore of every target that lowers the trigger,
    // and the boundary release's explicit trigger sites are the fallback, as the combined
    // mutations in `SOUND-INV-026`'s row established. The control: before the seek it
    // sounds, and a straight render with no seek still sounds where this is silent.
    let plan = sampler_voice();
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let events = vec![
        PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::NoteOn {
                slot,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(60 * Q),
            CompiledPayload::NoteOff { slot, key: key(60) },
        ),
    ];
    let frames = 40 * Q as usize;
    let straight = render_compiled(&plan, &events, frames);
    assert!(
        straight[20 * Q as usize..].iter().any(|s| *s != 0.0),
        "with no seek the sampler still sounds late in the render"
    );

    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let host = common::profile(BLOCK as u64, ChannelLayout::Mono);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("the store is preparable");
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let seek_at = 8 * Q;
    let activation = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(seek_at),
                position: PlanPosition::new(20 * Q),
                loop_interval: None,
            },
        )
        .expect("the seek builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");
    let mut out = Vec::new();
    let mut done = 0;
    while done < frames {
        let this = (Q as usize).min(frames - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(&mut renderer, &mut arbiter, output)
            .expect("the stream renders");
        out.extend_from_slice(&samples);
        done += this;
    }
    assert!(
        out[..seek_at as usize].iter().any(|s| *s != 0.0),
        "before the seek the sampler sounds"
    );
    // The boundary quantum, the stream's one-quantum carry, and V1's fade, then silence.
    let silent_from = seek_at as usize + 2 * Q as usize + SUSTAIN_FADE_FRAMES as usize;
    assert!(
        out[silent_from..].iter().all(|s| *s == 0.0),
        "the sampler played on across the seek: its trigger did not fall at the boundary"
    );
}
