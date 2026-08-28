//! The control-scratch budget covers what preparation actually allocates.
//!
//! `render::timed_control_scratch_bytes` is what admission charges a plan for its
//! sample-positioned control storage, and `PreparedRenderer::prepare` is what spends it.

use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use crate::profile::HostProfile;
use crate::quantities::{ChannelLayout, EventCount, HeldNoteCount, SampleRate};
use crate::stream::StreamControl;
use crate::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor};

const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);

#[test]
fn the_control_scratch_budget_covers_what_preparation_actually_allocates() {
    // The prediction against what the object reports holding, rather than against a
    // restatement of the formula. **The declared polyphony is what varies**, because that is
    // the term successive revisions got wrong: an activation's mass release needs a gate-down
    // per note it can end, and those live in three vectors sized by the identity partition —
    // the scratch itself, the queue they wait in, and the node indices beside it. A budget
    // that charged only the first reported a ceiling preparation then allocates past, which
    // is admission passing a plan it should refuse. An independent review found it.
    //
    // The falsifier is the direction: charged must be at least held, at every polyphony.
    for simultaneous in [1_u32, 2, 8, 64, 512] {
        let plan = plan_declaring(simultaneous);
        let ranges = plan.note_producer_ranges().to_vec();
        let (_control, renderer) = StreamControl::open(
            plan,
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .expect("the stream opens");

        let charged = crate::render::timed_control_scratch_bytes(
            profile().limits().events().max_events_per_quantum(),
            renderer.prepared_record_count(),
            ranges
                .iter()
                .copied()
                .fold(HeldNoteCount::NONE, |total, range| {
                    HeldNoteCount::measured(total.get().saturating_add(range.get()))
                }),
        );
        let held = renderer.control_scratch_bytes();
        assert!(
            charged >= held as u64,
            "a polyphony of {simultaneous} is charged {charged} bytes of control scratch but \
             holds {held}"
        );
    }
}

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(256),
        ChannelLayout::Mono,
    )
    .expect("valid harness profile")
}

fn plan_declaring(simultaneous: u32) -> crate::plan::CompiledPlan {
    let ir = GraphIr::builder()
        .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(simultaneous),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(simultaneous),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a source into an output is a readable plan");
    compile(&ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("the plan fits this profile")
}
