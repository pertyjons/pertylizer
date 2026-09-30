//! Phase 9A item 9: a live host's observation subscribers cannot alter audio or block the
//! renderer, and the plan needs no subscriber at all.
//!
//! The live stream renders on its own thread with the plan's tap subscribed. After each
//! callback that thread copies what the store pushed into a bounded SPSC ring without
//! waiting; a subscriber on a third thread drains the ring, or stalls and never does. A
//! full ring drops and counts frames instead of blocking. The allocation counter covers
//! each callback together with that forwarding copy.
use crate::compile::{RenderConfig, compile};
use crate::host::EndpointId;
use crate::host::input::{InputCapacity, InputLimits, SimulatedNoteInput};
use crate::host::live::{AuditionId, LiveInputStream};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use crate::observe::ObservationSubscriptions;
use crate::profile::HostProfile;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, NormalizedLevel, PreparedBytes,
    SampleRate, Seconds,
};
use crate::recording::notes::Midi1Input;
use crate::render::AudioBlockMut;
use crate::render_allocation::count_allocs;
use crate::time::{FrameCount, QUANTUM_FRAMES};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

const OSCILLATOR: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const MONITOR: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(5);
const Q: usize = QUANTUM_FRAMES as usize;
const CALLBACKS: usize = 400;
const STALLED_RING: usize = 4 * Q;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Subscriber {
    /// No store is handed to the render at all.
    Nobody,
    /// A thread drains the forwarding ring, during or after the render.
    Reading,
    /// A thread holds the ring and never reads it.
    Stalled,
}

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(Q as u64),
        ChannelLayout::Mono,
    )
    .expect("valid harness profile")
}

/// A live-played voice whose amplifier output passes a declared monitor tap.
fn monitored_live_voice() -> GraphIr {
    GraphIr::builder()
        .node(
            OSCILLATOR,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
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
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(4),
                simultaneous_holds: EventCount::measured(4),
            }],
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan")
}

/// Returns the rendered audio, what the subscriber thread received, and how many
/// forwarded samples a full ring dropped.
fn run(subscriber: Subscriber) -> (Vec<f32>, Vec<f32>, usize) {
    let plan = compile(&monitored_live_voice(), &RenderConfig::new(profile()))
        .into_plan()
        .expect("an admitted plan");
    let mut store = ObservationSubscriptions::prepare(&profile(), &plan);
    let tap = store
        .subscribe(
            &plan,
            plan.resolve_tap(MONITOR, PortId::FIRST)
                .expect("a declared tap"),
        )
        .expect("a subscription");
    let channels = store.channels(tap).expect("an issued handle");
    let note = plan.resolve_note(ENVELOPE).expect("a note target");
    let source = SimulatedNoteInput::new(
        EndpointId::new("observed".into()).expect("valid endpoint"),
        InputLimits {
            cells: InputCapacity::new(4).expect("valid capacity"),
            bytes: PreparedBytes::measured(65_536),
        },
    )
    .expect("valid input")
    .begin()
    .expect("a fresh generation");
    let mut live = LiveInputStream::prepare(
        plan,
        profile(),
        note,
        &[source],
        EventCount::measured(8),
        PreparedBytes::measured(4_000_000),
    )
    .expect("a live stream");

    // The audio thread never waits for the subscriber. A reading subscriber gets a ring
    // that holds the whole run, so it loses nothing however it is scheduled; a stalled one
    // gets four quanta, so its ring fills and the renderer carries on and counts the loss.
    let capacity = match subscriber {
        Subscriber::Stalled => STALLED_RING,
        Subscriber::Nobody | Subscriber::Reading => CALLBACKS * Q * channels,
    };
    let (mut forward, mut receive) = HeapRb::<f32>::new(capacity).split();
    let done = Arc::new(AtomicBool::new(false));
    let finished = Arc::clone(&done);
    let (release_stall, stalled) = mpsc::channel::<()>();
    let reader = std::thread::spawn(move || {
        let mut received = Vec::new();
        if subscriber == Subscriber::Stalled {
            // Holds the ring without reading until the audio thread has finished.
            let _hold = stalled.recv();
            return received;
        }
        loop {
            let finished = finished.load(Ordering::Acquire);
            while let Some(sample) = receive.try_pop() {
                received.push(sample);
            }
            if finished {
                return received;
            }
            std::thread::yield_now();
        }
    });
    let audio = std::thread::spawn(move || {
        let mut output = vec![0.0; CALLBACKS * Q];
        let mut scratch = vec![0.0; 4 * Q * channels];
        let mut dropped = 0;
        for (index, block) in output.chunks_mut(Q).enumerate() {
            if index == 1 {
                live.queue(
                    AuditionId::new(source, 1).expect("valid id"),
                    live.clock(),
                    Midi1Input::from_bytes([0x90, 69, 100]).expect("valid MIDI"),
                )
                .expect("an admitted onset");
            }
            let events = count_allocs(|| {
                let block = AudioBlockMut::new(block, Q, ChannelLayout::Mono).expect("valid block");
                if subscriber == Subscriber::Nobody {
                    live.render(block).expect("a rendered callback");
                    return;
                }
                live.render_observed(block, &mut store)
                    .expect("a rendered callback");
                let read = store.read(tap, &mut scratch).expect("an issued handle");
                let samples = usize::try_from(read.frames.as_u64()).unwrap_or(0) * channels;
                let pushed = forward.push_slice(&scratch[..samples]);
                dropped += samples - pushed;
            });
            assert_eq!(events, 0, "a callback or its forwarding copy allocated");
        }
        (output, dropped)
    });
    let (output, dropped) = audio.join().expect("the audio thread finished");
    done.store(true, Ordering::Release);
    let _released = release_stall.send(());
    let received = reader.join().expect("the subscriber finished");
    (output, received, dropped)
}

#[test]
fn live_observation_subscribers_change_no_sample_and_never_block_the_renderer() {
    let (reference, nothing, _) = run(Subscriber::Nobody);
    assert!(nothing.is_empty());
    assert!(
        reference.iter().any(|sample| *sample != 0.0),
        "the note sounds"
    );

    // The tap sees each quantum as it renders; the output delivers it one quantum later
    // (ADR-0001's constant latency), so the tap stream is the output advanced by Q.
    let (reading, received, dropped) = run(Subscriber::Reading);
    assert!(
        reading == reference,
        "a reading subscriber changed the audio"
    );
    assert_eq!(dropped, 0, "a reading subscriber keeps up");
    assert!(
        received[..] == reference[Q..],
        "the subscriber received the tap exactly"
    );

    let (stalled, starved, dropped) = run(Subscriber::Stalled);
    assert!(
        stalled == reference,
        "a stalled subscriber changed the audio"
    );
    assert!(starved.is_empty());
    assert_eq!(
        dropped,
        reference.len() - Q - STALLED_RING,
        "a full ring drops and counts everything beyond its capacity instead of blocking"
    );
}

#[test]
fn a_store_for_another_plan_is_refused_and_silenced() {
    let plan = compile(&monitored_live_voice(), &RenderConfig::new(profile()))
        .into_plan()
        .expect("an admitted plan");
    let other = compile(&monitored_live_voice(), &RenderConfig::new(profile()))
        .into_plan()
        .expect("a second plan");
    let mut foreign = ObservationSubscriptions::prepare(&profile(), &other);
    let note = plan.resolve_note(ENVELOPE).expect("a note target");
    let source = SimulatedNoteInput::new(
        EndpointId::new("foreign".into()).expect("valid endpoint"),
        InputLimits {
            cells: InputCapacity::new(4).expect("valid capacity"),
            bytes: PreparedBytes::measured(65_536),
        },
    )
    .expect("valid input")
    .begin()
    .expect("a fresh generation");
    let mut live = LiveInputStream::prepare(
        plan,
        profile(),
        note,
        &[source],
        EventCount::measured(8),
        PreparedBytes::measured(4_000_000),
    )
    .expect("a live stream");
    let mut output = [9.0; Q];
    assert!(
        live.render_observed(
            AudioBlockMut::new(&mut output, Q, ChannelLayout::Mono).expect("valid block"),
            &mut foreign,
        )
        .is_err()
    );
    assert_eq!(output, [0.0; Q]);
}
