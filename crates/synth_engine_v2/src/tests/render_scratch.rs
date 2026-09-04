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
const OSCILLATOR: NodeId = NodeId::new(3);
const ENVELOPE: NodeId = NodeId::new(4);
const AMPLIFIER: NodeId = NodeId::new(5);

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
    // **Both plan shapes**, because `SOUND-INV-021` made the charge depend on what a
    // note-on expands to: a plan with no playable node writes one control per event, and a
    // real voice writes its gate plus a pitch and a velocity. A budget still stated over one
    // write per event passes the first shape and is overrun by the second.
    for simultaneous in [1_u32, 2, 8, 64, 512] {
        for voice in [false, true] {
            check_one(simultaneous, voice);
        }
    }
}

/// One polyphony and one plan shape: what admission charges against what preparation holds.
fn check_one(simultaneous: u32, voice: bool) {
    {
        let (ir, plan) = plan_declaring(simultaneous, voice);
        let ranges = plan.note_producer_ranges().to_vec();
        // Admission charges from the IR and preparation allocates from the plan, so the two
        // figures are compared here as well: an IR bound below what the plan expands to
        // would size the scratch under what the render loop then fills.
        assert!(
            plan.max_writes_per_note() <= ir.max_writes_per_note(),
            "the IR charges {} where the plan expands to {}",
            ir.max_writes_per_note(),
            plan.max_writes_per_note()
        );
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
            ir.max_writes_per_note(),
        );
        let held = renderer.control_scratch_bytes();
        assert!(
            charged >= held as u64,
            "a polyphony of {simultaneous} on a {} plan is charged {charged} bytes of control \
             scratch but holds {held}",
            if voice { "voice" } else { "silent" }
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

fn plan_declaring(simultaneous: u32, voice: bool) -> (GraphIr, crate::plan::CompiledPlan) {
    let mut builder = GraphIr::builder();
    if voice {
        // A real voice: the note's gate on the envelope, its key on the oscillator, its
        // velocity on the envelope beside the gate. Three writes where the silent plan has
        // one, which is the whole point of running both shapes.
        builder = builder
            .node(
                OSCILLATOR,
                IrNodeKind::Sine {
                    frequency: crate::quantities::Frequency::new(220.0).expect("finite"),
                    amplitude: crate::quantities::Amplitude::UNITY,
                },
                ExecutionScope::Voice,
            )
            .node(
                ENVELOPE,
                IrNodeKind::Envelope {
                    attack: crate::quantities::Seconds::ZERO,
                    decay: crate::quantities::Seconds::ZERO,
                    sustain: crate::quantities::NormalizedLevel::FULL,
                    release: crate::quantities::Seconds::ZERO,
                },
                ExecutionScope::Voice,
            )
            .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
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
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .tuning(
                ExecutionScope::Voice,
                crate::tuning::PreparedTuning::equal_temperament().expect("12-TET prepares"),
            );
    } else {
        builder = builder
            .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Voice)
            .connect(
                (SOURCE, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            );
    }
    let ir = builder
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
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
    let plan = compile(&ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("the plan fits this profile");
    (ir, plan)
}
