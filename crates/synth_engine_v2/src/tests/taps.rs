//! `SOUND-INV-022` in the compiler and the renderer: a tap is a declared, passive artifact.
//!
//! Inside the crate because the read a subscription will make — the tapped region after a
//! quantum renders — has no production consumer yet and is exposed test-only.

use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use crate::node::TapData;
use crate::plan::CompiledPlan;
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, SampleRate, Seconds,
};
use crate::render::{AudioBlockMut, PreparedRenderer};
use crate::schedule::{AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent};
use crate::stream::StreamControl;
use crate::time::{PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const OSCILLATOR: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const MONITOR: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(5);
const Q: u64 = QUANTUM_FRAMES as u64;
const BLOCK: usize = 256;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

#[test]
fn a_tap_exists_only_through_a_declaration_and_names_the_node_and_port() {
    // A monitor's declaration is the one source of a tap: the monitored voice has exactly
    // one, addressed by the monitor and its output port, resolving nothing for a node whose
    // kind declares none — and the same voice without the monitor carries no tap at all.
    let monitored = admit(&voice(true));
    let plain = admit(&voice(false));
    assert!(plain.taps().is_empty() && plain.tap_addresses().is_empty());
    // One tap row per voice instance of the monitor — the voice declares four simultaneous
    // notes — and one address, naming instance 0's row (`P06-S001`).
    assert_eq!(
        monitored.taps().len(),
        usize::try_from(monitored.voice_instances().get()).expect("fits")
    );
    assert_eq!(monitored.tap_addresses().len(), 1);
    // Each row names its **own** instance's step, in instance order, and the region that
    // step writes — a consumer correlating a tap with a step must not be sent to instance 0
    // for every row. An independent read found every row naming the first instance.
    let first = monitored.taps()[0].node.index();
    for (instance, tap) in monitored.taps().iter().enumerate() {
        assert_eq!(
            tap.node.index(),
            first + instance,
            "tap row {instance} names another instance's step"
        );
        let step = monitored
            .ops()
            .iter()
            .find_map(|op| match op {
                crate::plan::PlanOp::Node(step) if step.node() == tap.node => Some(*step),
                _ => None,
            })
            .expect("every instance is scheduled");
        assert_eq!(
            step.out(),
            tap.region,
            "tap row {instance} names a region its own step does not write"
        );
    }
    let slot = monitored
        .resolve_tap(MONITOR, PortId::FIRST)
        .expect("the monitor declares a tap on its output");
    assert_eq!(slot.plan(), monitored.id());
    assert_eq!(
        monitored.resolve_tap(AMPLIFIER, PortId::FIRST),
        None,
        "an amplifier declares no tap, so nothing can subscribe to its output"
    );
    assert_eq!(
        monitored.resolve_tap(MONITOR, PortId::new(7)),
        None,
        "a port the monitor does not declare a tap on resolves nothing"
    );
    let tap = monitored.taps()[slot.index()];
    assert_eq!(tap.data, TapData::Audio);
    // One quantum of mono `f32`: the declared cost, from the port's layout.
    assert_eq!(tap.bytes_per_quantum.get(), Q * 4);
    // The tap names the monitor's own output region, the one its step writes.
    let monitor_step = monitored
        .ops()
        .iter()
        .filter_map(|op| match op {
            crate::plan::PlanOp::Node(step) if step.node() == tap.node => Some(*step),
            _ => None,
        })
        .next()
        .expect("the monitor is scheduled");
    assert_eq!(monitor_step.out(), tap.region);
}

#[test]
fn a_monitor_is_passive_and_its_tap_reads_the_signal_that_passed_through() {
    // Passivity, bit for bit: the voice with a monitor before its output renders exactly
    // what the voice without one renders. And the tap is the signal point it names: after
    // a render the tapped region holds the last quantum rendered, which — the renderer's
    // primed quantum having been spent by the whole-quantum call — is the last quantum the
    // caller received. The quantum after it is different, which is what makes the
    // equality one of a signal point rather than of a periodic waveform.
    let plain = render(&admit(&voice(false)), 6);
    let (mut renderer, monitored, plan) = open_and_render(&admit(&voice(true)), 6);
    assert_eq!(plain, monitored, "a monitor changed a sample");

    let slot = plan
        .resolve_tap(MONITOR, PortId::FIRST)
        .expect("the monitor declares a tap");
    let tapped: Vec<f32> = renderer.tap_block(slot).to_vec();
    assert_eq!(tapped.len(), Q as usize);
    let last = &monitored[monitored.len() - Q as usize..];
    assert_eq!(
        tapped, last,
        "the tap did not hold the signal the output carried"
    );
    assert!(
        tapped.iter().any(|s| *s != 0.0),
        "the tapped quantum is not silence"
    );
    let next = renderer.drive(Q as usize);
    assert_ne!(tapped, next, "the control: the following quantum differs");
}

#[test]
fn the_monitor_kernel_passes_its_input_through_in_every_input_state() {
    // The monitored voice above runs the monitor in place — its input's last reader — so
    // only one of the kernel's three branches runs there. At the kernel: a patched input is
    // copied sample for sample, an in-place input is left as it is, and an unpatched one
    // is silence, which is what every other kind reads an unpatched input as.
    use crate::node::kernels::{InputBuffer, MAX_INPUTS, NodeIo, NodeState, PreparedNode, monitor};
    let source: Vec<f32> = (0..QUANTUM_FRAMES)
        .map(|i| (i as f32) * 0.25 - 3.0)
        .collect();
    let run = |input: InputBuffer<'_>, seed: f32| -> Vec<f32> {
        let mut out = vec![seed; QUANTUM_FRAMES as usize];
        let mut inputs = [InputBuffer::Unpatched; MAX_INPUTS];
        inputs[0] = input;
        let mut io = NodeIo {
            out: &mut out,
            channels: ChannelLayout::Mono,
            inputs,
            position: None,
            controls: &[],
            ramps: &[],
        };
        monitor(&PreparedNode::Copy, &mut NodeState::Stateless, &mut io);
        out
    };
    assert_eq!(run(InputBuffer::Patched(&source), -9.0), source);
    assert_eq!(
        run(InputBuffer::InPlace, 0.75),
        vec![0.75; QUANTUM_FRAMES as usize],
        "in place, the output already holds the input"
    );
    assert_eq!(
        run(InputBuffer::Unpatched, 0.75),
        vec![0.0; QUANTUM_FRAMES as usize]
    );
}

// --- fixtures -----------------------------------------------------------------------------

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        crate::time::FrameCount::new(BLOCK as u64),
        ChannelLayout::Mono,
    )
    .expect("valid harness profile")
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    compile(ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("the plan fits this profile")
}

/// The smallest real voice, with or without a monitor between the amplifier and the output.
fn voice(monitored: bool) -> GraphIr {
    let mut builder = GraphIr::builder()
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
        );
    if monitored {
        builder = builder
            .node(MONITOR, IrNodeKind::Monitor, ExecutionScope::Voice)
            .connect(
                (AMPLIFIER, PortId::FIRST),
                (MONITOR, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (MONITOR, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            );
    } else {
        builder = builder.connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    builder
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

struct Driven {
    scheduler: CompiledEventScheduler,
    arbiter: PublicationArbiter,
}

fn render(plan: &CompiledPlan, quanta: u64) -> Vec<f32> {
    open_and_render(plan, quanta).1
}

fn open_and_render(plan: &CompiledPlan, quanta: u64) -> (Renderer, Vec<f32>, CompiledPlan) {
    let (mut control, renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(plan, &events(plan)).expect("the stream fits");
    let scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let arbiter = PublicationArbiter::prepare(&profile()).expect("the store is preparable");
    let mut driven = Renderer {
        renderer,
        driven: Driven { scheduler, arbiter },
    };
    let out = driven.drive((quanta * Q) as usize);
    (driven, out, plan.clone())
}

/// A renderer with the scheduler and arbiter that drive it.
struct Renderer {
    renderer: PreparedRenderer,
    driven: Driven,
}

impl Renderer {
    fn drive(&mut self, frames: usize) -> Vec<f32> {
        let mut out = Vec::new();
        let mut done = 0;
        while done < frames {
            let this = BLOCK.min(frames - done);
            let mut samples = vec![0.0_f32; this];
            let output = AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono)
                .expect("a shaped block");
            self.driven
                .scheduler
                .render(&mut self.renderer, &mut self.driven.arbiter, output)
                .expect("the stream renders");
            out.extend_from_slice(&samples);
            done += this;
        }
        out
    }

    fn tap_block(&self, slot: crate::plan::TapSlot) -> &[f32] {
        self.renderer.tap_block(slot)
    }
}
