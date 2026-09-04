//! Phase 3's exit gate: the reference V2 renders are invariant to host block partitioning.
//!
//! > The reference V2 renders are invariant to host block partitioning.
//!
//! # Why a stateful voice and not a gated constant
//!
//! `note_events`' partition test already renders a **constant** through a gate across three
//! block sizes, which proves an edge lands on its sample whatever the caller's blocks are.
//! It cannot prove this gate, and the reason is what the gate is about: a constant has no
//! state, so every quantum's output depends only on that quantum's events. Partitioning
//! could hardly disturb it.
//!
//! The reference render is the phase's own deliverable voice — a sine through a **filter**,
//! an envelope and an amplifier. A biquad carries two samples of history and an envelope
//! carries its phase and level, so a block boundary that falls mid-envelope or mid-ring is
//! where a renderer that reset, re-prepared or double-advanced anything would diverge. That
//! is the claim: the same frames, rendered as one call or as sixty-four, are bit-identical.
//!
//! # And through the live path
//!
//! `note_events` renders offline, which is latency-compensated and drives its own blocks.
//! This drives `CompiledEventScheduler::render` with the caller's actual block sizes, so the
//! carry, the publication window and the quantum split are all exercised — the three things
//! that make a partition visible to the renderer at all.

mod common;

use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::publish::PublicationArbiter;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, NormalizedLevel, Resonance, Seconds,
};
use synth_engine_v2::render::AudioBlockMut;
use synth_engine_v2::schedule::{
    AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent,
};
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);
const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);
const FILTER: NodeId = NodeId::new(13);

const Q: u64 = QUANTUM_FRAMES as u64;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
const TOTAL_FRAMES: usize = 4_096;

/// Both edges deliberately off a quantum boundary, and the note long enough to outlast its
/// attack and decay so the sustain and the release both fall inside the render.
const ON: u64 = 2 * Q + 17;
const OFF: u64 = 30 * Q + 3;

const WHOLE: [usize; 1] = [4_096];
const BLOCKS_256: [usize; 16] = [256; 16];
const BLOCKS_64: [usize; 64] = [64; 64];
const IRREGULAR: [usize; 10] = [17, 511, 3, 64, 1_024, 1, 700, 256, 63, 1_457];

/// The phase's reference voice: a sine through a filter, an envelope and an amplifier.
fn voice() -> GraphIr {
    GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(0.8).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(2_000.0).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.002).expect("not negative"),
                decay: Seconds::new(0.010).expect("not negative"),
                sustain: NormalizedLevel::new(0.6).expect("within range"),
                release: Seconds::new(0.020).expect("not negative"),
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
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
        // `SOUND-INV-021`: the sine is a pitch destination in the played node's scope, so
        // the scope states what a key resolves to.
        .tuning(ExecutionScope::Voice, common::twelve_tet())
        .declaring(common::compiled_notes(4))
        .build()
        .expect("a readable plan")
}

fn plan() -> CompiledPlan {
    common::admit(
        &voice(),
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
    )
}

fn compiled_note(plan: &CompiledPlan, time: u64, on: bool) -> PlanEvent {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope accepts note edges");
    let payload = if on {
        common::note_on(slot)
    } else {
        CompiledPayload::NoteOff { slot }
    };
    PlanEvent::new(PlanPosition::new(time), payload)
}

/// Render the reference voice through the live path with the caller's block pattern.
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
    let admitted =
        AdmittedCompiledStream::admit(plan, &events).expect("the compiled stream fits its share");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &admitted).expect("the schedule is valid");
    let mut publication =
        PublicationArbiter::prepare(&common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono))
            .expect("the publication store is preparable");

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
        "an admitted stream must never reach the preserving late clamp"
    );
    rendered
}

#[test]
fn the_reference_voice_is_bit_identical_across_host_block_partitions() {
    let plan = plan();
    let partitions: [&[usize]; 4] = [&WHOLE, &BLOCKS_256, &BLOCKS_64, &IRREGULAR];
    let renders: Vec<Vec<f32>> = partitions
        .iter()
        .map(|partition| render_partition(&plan, partition))
        .collect();
    let reference = renders.first().expect("four renders");

    // **Comparing two silences would pass**, which is the failure mode this file exists to
    // avoid rather than demonstrate. So the reference render is checked for the shape a
    // working voice has before anything is compared against it: silent until the note plus
    // the declared carry, then sounding, then silent again after the release.
    let start = (ON + Q) as usize;
    assert!(
        reference[..start].iter().all(|sample| *sample == 0.0),
        "the voice must be silent before its note"
    );
    let sounding = reference[start..(OFF + Q) as usize]
        .iter()
        .filter(|sample| **sample != 0.0)
        .count();
    assert!(
        sounding > 1_000,
        "the reference render sounds for only {sounding} frames, so a bit-identity \
         comparison against it would be close to comparing silences"
    );
    // The release is 20 ms — 960 frames at 48 kHz — so a margin well past it, with room
    // left in the render for the tail to be a real span rather than a handful of frames.
    let tail = &reference[(OFF + Q) as usize + 1_200..];
    assert!(
        tail.len() > 800,
        "the tail is only {} frames, which is too short to mean much",
        tail.len()
    );
    assert!(
        tail.iter().all(|sample| *sample == 0.0),
        "the voice must fall silent well past its 20 ms release"
    );

    // **The state is what makes this a claim.** A biquad carries two samples of history and
    // the envelope carries its phase, so a boundary falling mid-ring or mid-attack is where
    // a renderer that reset, re-prepared or double-advanced anything diverges. `IRREGULAR`
    // is the strongest of the four: its blocks are 17, 511, 3, 1 and 1 457 frames, so almost
    // every boundary lands at a different offset inside a quantum.
    for (index, rendered) in renders.iter().enumerate().skip(1) {
        assert_eq!(
            rendered.len(),
            reference.len(),
            "partition {index} rendered a different number of frames"
        );
        let first_difference = rendered
            .iter()
            .zip(reference.iter())
            .position(|(left, right)| left != right);
        assert_eq!(
            first_difference, None,
            "partition {index} diverges from the whole-block render at frame {:?}",
            first_difference
        );
    }
}
