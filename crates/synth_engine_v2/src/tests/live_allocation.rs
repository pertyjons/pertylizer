//! Phase 9A: live event delivery, parameter updates and a running plan swap allocate
//! nothing on the audio thread.
//!
//! Uses [`count_allocs`] from the render-loop allocation harness, which owns the process's
//! only counting allocator. Deallocation counts too, so no callback performs a final drop.
//! Only the operations a callback performs are counted; candidate preparation and
//! retirement collection are control-thread work and stay outside the counted regions.

use crate::compile::{RenderConfig, compile};
use crate::host::ConnectionGeneration;
use crate::host::EndpointId;
use crate::host::input::{InputCapacity, InputLimits, SimulatedNoteInput};
use crate::host::live::{
    AuditionId, AuditionOutcome, LiveInputStream, PreparedLivePlan, SwapOutcome,
    SwappingLiveStream, UpdateVersion,
};
use crate::ingress::ReleaseCause;
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain, parameters,
};
use crate::profile::HostProfile;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, HeldNoteCount, NormalizedLevel, ParameterValue,
    PreparedBytes, SampleRate, Seconds,
};
use crate::recording::notes::Midi1Input;
use crate::render::AudioBlockMut;
use crate::render_allocation::count_allocs;
use crate::time::{FrameCount, QUANTUM_FRAMES};

const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);
const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);
const Q: usize = QUANTUM_FRAMES as usize;

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(4_096),
        ChannelLayout::Mono,
    )
    .expect("valid harness profile")
}

/// A constant through a gated envelope, whose sustain is the live parameter.
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
            (AMPLIFIER, crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
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

fn source() -> ConnectionGeneration {
    SimulatedNoteInput::new(
        EndpointId::new("allocation".into()).expect("valid endpoint"),
        InputLimits {
            cells: InputCapacity::new(4).expect("valid capacity"),
            bytes: PreparedBytes::measured(65_536),
        },
    )
    .expect("valid input")
    .begin()
    .expect("a fresh generation")
}

fn midi(bytes: [u8; 3]) -> Midi1Input {
    Midi1Input::from_bytes(bytes).expect("valid MIDI")
}

#[test]
fn live_ingress_delivery_and_parameter_updates_allocate_nothing() {
    let source = source();
    let plan = compile(&gated_constant(), &RenderConfig::new(profile()))
        .into_plan()
        .expect("an admitted plan");
    let slot = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_SUSTAIN)
        .expect("a sustain slot");
    let note = plan.resolve_note(ENVELOPE).expect("a note target");
    let mut live = LiveInputStream::prepare(
        plan,
        profile(),
        note,
        &[source],
        EventCount::measured(8),
        PreparedBytes::measured(2_000_000),
    )
    .expect("a live stream");
    live.prepare_parameters(&[slot], PreparedBytes::measured(2_000_000))
        .expect("parameter cells");
    let mut pcm = [0.0; Q];
    let on = AuditionId::new(source, 1).expect("valid id");
    let off = AuditionId::new(source, 2).expect("valid id");

    let mut outcomes = [None; 2];
    let events = count_allocs(|| {
        let render = |live: &mut LiveInputStream, pcm: &mut [f32; Q]| {
            live.render(AudioBlockMut::new(pcm, Q, ChannelLayout::Mono).expect("valid block"))
                .expect("a rendered quantum");
        };
        render(&mut live, &mut pcm);
        live.queue(on, live.clock(), midi([0x90, 60, 127]))
            .expect("an admitted onset");
        for version in 1..=64 {
            live.update_parameter(
                slot,
                UpdateVersion::new(version).expect("valid version"),
                ParameterValue::new(0.5).expect("finite"),
            )
            .expect("an accepted update");
        }
        render(&mut live, &mut pcm);
        live.queue(off, live.clock(), midi([0x80, 60, 0]))
            .expect("an admitted release");
        render(&mut live, &mut pcm);
        outcomes = [live.take_outcome(on), live.take_outcome(off)];
        live.end_at(live.clock(), ReleaseCause::Stop)
            .expect("an ordered stop");
        render(&mut live, &mut pcm);
    });
    assert_eq!(
        events, 0,
        "live delivery allocated or freed on the callback"
    );
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, Some(AuditionOutcome::Executed { .. })))
    );
}

#[test]
fn a_running_plan_swap_allocates_nothing_on_the_callback() {
    let source = source();
    let graph = gated_constant();
    let mut live = SwappingLiveStream::prepare(
        &graph,
        profile(),
        ENVELOPE,
        &[source],
        &[],
        EventCount::measured(16),
        PreparedBytes::measured(8_000_000),
    )
    .expect("a swapping stream");
    let render = |live: &mut SwappingLiveStream| {
        let mut pcm = [0.0; Q];
        let events = count_allocs(|| {
            live.render(AudioBlockMut::new(&mut pcm, Q, ChannelLayout::Mono).expect("valid block"))
                .expect("a rendered quantum");
        });
        assert_eq!(events, 0, "a callback allocated or freed");
        pcm
    };
    render(&mut live);
    let events = count_allocs(|| {
        live.queue(
            AuditionId::new(source, 1).expect("valid id"),
            live.clock(),
            midi([0x90, 60, 127]),
        )
        .expect("an admitted onset");
    });
    assert_eq!(events, 0);
    assert_eq!(render(&mut live), [1.0; Q]);

    let candidate = live
        .prepare_candidate(&graph, ENVELOPE, &[])
        .expect("an off-thread candidate");
    let faded = render(&mut live);
    assert!(matches!(
        live.swap_outcome(),
        Some(SwapOutcome::Installed { plan, .. }) if plan == candidate
    ));
    assert!(
        faded[0] > 0.0 && faded[Q - 1] == 0.0,
        "the old plan fades once"
    );
    render(&mut live);
    assert!(
        live.collect_retired(|_| {}),
        "retirement is collected off the callback"
    );
}

/// The audio-side handoff of an off-thread prepared plan: admission from the transport
/// cell, installation with its fade, and moving the retired owner back out. Only the
/// preparation before and the destruction after are control-thread work.
#[test]
fn a_prepared_plan_handoff_and_retirement_move_allocate_nothing() {
    let source = source();
    let graph = gated_constant();
    let mut live = SwappingLiveStream::prepare(
        &graph,
        profile(),
        ENVELOPE,
        &[source],
        &[],
        EventCount::measured(16),
        PreparedBytes::measured(8_000_000),
    )
    .expect("a swapping stream");
    let mut cell = Some(
        PreparedLivePlan::prepare(
            &graph,
            profile(),
            ENVELOPE,
            &[source],
            &[],
            EventCount::measured(16),
            PreparedBytes::measured(4_000_000),
        )
        .expect("an off-thread prepared plan"),
    );
    let candidate = cell.as_ref().map(PreparedLivePlan::plan_id);
    let onset = AuditionId::new(source, 1).expect("valid id");
    let mut pcm = [0.0; Q];
    let mut retired = None;
    let mut accepted = false;
    let events = count_allocs(|| {
        let render = |live: &mut SwappingLiveStream, pcm: &mut [f32; Q]| {
            live.render(AudioBlockMut::new(pcm, Q, ChannelLayout::Mono).expect("valid block"))
                .expect("a rendered quantum");
        };
        render(&mut live, &mut pcm);
        live.queue(onset, live.clock(), midi([0x90, 60, 127]))
            .expect("an admitted onset");
        render(&mut live, &mut pcm);
        accepted = live
            .accept_prepared(&mut cell)
            .expect("an admissible payload");
        render(&mut live, &mut pcm);
        render(&mut live, &mut pcm);
        let _ = live.take_outcome(onset);
        retired = live.take_retired();
    });
    assert_eq!(events, 0, "the handoff allocated or freed on the callback");
    assert!(
        accepted && cell.is_none(),
        "the payload moved into the stream"
    );
    assert!(matches!(
        live.swap_outcome(),
        Some(SwapOutcome::Installed { plan, .. }) if Some(plan) == candidate
    ));
    assert!(
        retired.is_some(),
        "the retired owner leaves the callback intact"
    );
    drop(retired);
}
