//! `P08-S003`, `SOUND-INV-033`: what the crate's own tables say about the two insert kinds —
//! the report's charge against the renderer's allocation, and the declared timing.

use crate::compile::{RenderConfig, compile};
use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use crate::profile::HostProfile;
use crate::quantities::{
    Amplitude, ChannelLayout, DelayFeedback, DelayTime, NormalizedLevel, SampleRate,
};
use crate::stream::StreamControl;
use crate::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor};

const SOURCE: NodeId = NodeId::new(1);
const INSERT: NodeId = NodeId::new(2);
const OUTPUT: NodeId = NodeId::new(3);

fn rate() -> SampleRate {
    SampleRate::new(48_000.0).expect("a rate")
}

fn profile() -> HostProfile {
    HostProfile::harness(rate(), FrameCount::new(256), ChannelLayout::Stereo)
        .expect("the harness profile is valid")
}

fn delay() -> IrNodeKind {
    IrNodeKind::Delay {
        time_left: DelayTime::new(0.25).expect("in range"),
        time_right: DelayTime::new(0.25).expect("in range"),
        feedback: DelayFeedback::new(0.45).expect("in range"),
        mix: NormalizedLevel::new(0.5).expect("a level"),
        tone: NormalizedLevel::new(0.4).expect("a level"),
    }
}

fn through(insert: IrNodeKind, scope: ExecutionScope) -> GraphIr {
    GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(0.25).expect("finite"),
            },
            ExecutionScope::Global,
        )
        .node(INSERT, insert, scope)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (INSERT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (INSERT, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan")
}

/// The report's mutable-bytes row charges the delay's history exactly as the renderer
/// allocates it: two lines of `2 s` at the rate, at stereo width, plus the per-record index —
/// the same equation the slot tests hold, with the history added on both sides.
#[test]
fn a_delays_history_is_charged_as_the_renderer_allocates_it() {
    let ir = through(delay(), ExecutionScope::Global);
    let outcome = compile(&ir, &RenderConfig::new(profile()));
    let reported = match outcome
        .report()
        .row(crate::report::ResourceField::MutableStateBytes)
        .expect("row")
        .requested()
    {
        crate::report::ResourceAmount::Bytes(bytes) => bytes.get(),
        other => panic!("mutable bytes: {other:?}"),
    };
    let plan = outcome.into_plan().expect("admits");
    let (_, renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("stream");
    assert_eq!(
        renderer.slot_bytes_held() as u64
            + renderer.ramp_table_bytes_held() as u64
            + renderer.history_bytes_held() as u64
            + u64::from(renderer.prepared_record_count().get())
                * crate::node::state_bytes_per_node(),
        reported,
        "the history slab and its index are charged exactly"
    );
    // And the slab is what V1 keeps: `(2.0 × rate) as usize` frames per side, two sides.
    let line = crate::node::delay_line_frames(rate());
    assert_eq!(line, 96_000);
    assert_eq!(
        renderer.history_bytes_held(),
        line * 2 * size_of::<f32>()
            + (renderer.prepared_record_count().get() as usize + 1) * size_of::<usize>(),
    );
    assert_eq!(
        crate::node::history_bytes(delay(), rate()),
        (line * 2 * size_of::<f32>()) as u64
    );
}

/// A delay in the voice scope keeps one line per instance: the charge and the allocation
/// both scale with the instance count.
#[test]
fn a_voice_scope_delay_keeps_one_line_per_instance() {
    use crate::ir::{NoteProducerDeclaration, PlanDeclarations};
    use crate::quantities::{EventCount, HeldNoteCount};
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: crate::quantities::Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(INSERT, delay(), ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (INSERT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (INSERT, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().expect("prepares"),
        )
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(3),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(3),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan");
    let outcome = compile(&ir, &RenderConfig::new(profile()));
    let reported = match outcome
        .report()
        .row(crate::report::ResourceField::MutableStateBytes)
        .expect("row")
        .requested()
    {
        crate::report::ResourceAmount::Bytes(bytes) => bytes.get(),
        other => panic!("mutable bytes: {other:?}"),
    };
    let plan = outcome.into_plan().expect("admits");
    let (_, renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("stream");
    let line = crate::node::delay_line_frames(rate());
    assert!(
        renderer.history_bytes_held() >= 3 * line * 2 * size_of::<f32>(),
        "three instances keep three pairs of lines"
    );
    assert_eq!(
        renderer.slot_bytes_held() as u64
            + renderer.ramp_table_bytes_held() as u64
            + renderer.history_bytes_held() as u64
            + u64::from(renderer.prepared_record_count().get())
                * crate::node::state_bytes_per_node(),
        reported,
    );
}
