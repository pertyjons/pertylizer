//! Renderer-level falsifiers for the scoped mixed restoration rehearsal.

use std::sync::Arc;

use super::{
    AudioBlockMut, EventEnvelope, EventPayload, NoteEdge, PreparedRenderer, Renderer,
    ScopedParameterRestore, TimedEvent, TimedEvents,
};
use crate::compile::{RenderConfig, compile};
use crate::host::mixed_targets::{MixedRestorationGroup, MixedTargetAdmission};
use crate::identity::{IdentityTable, NoteIdentity};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain, parameters,
};
use crate::plan::{ParameterInstanceSpan, ParameterSlot};
use crate::profile::HostProfile;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, ParameterValue, SampleRate, Seconds, VoiceCount,
};
use crate::render::slot::SlotState;
use crate::schedule::AdmittedCompiledStream;
use crate::time::{
    FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor, StreamEpoch, TimeSource,
    issue_epoch,
};

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const GLOBAL_SOURCE: NodeId = NodeId::new(5);
const CONTROLLER: NodeId = NodeId::new(6);
const Q: usize = QUANTUM_FRAMES as usize;
const ANCHOR: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

fn value(raw: f32) -> ParameterValue {
    ParameterValue::new(raw).expect("finite parameter")
}

fn binding(compiled_first: bool) -> (MixedTargetAdmission, crate::plan::NoteSlot) {
    let compiled = NoteProducerDeclaration {
        compiled: true,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::NONE,
    };
    let live = NoteProducerDeclaration {
        compiled: false,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::measured(2),
    };
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite frequency"),
                amplitude: Amplitude::new(0.8).expect("finite amplitude"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("finite attack"),
                decay: Seconds::new(0.0).expect("finite decay"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("finite release"),
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(
            GLOBAL_SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(110.0).expect("finite frequency"),
                amplitude: Amplitude::new(0.1).expect("finite amplitude"),
            },
            ExecutionScope::Global,
        )
        .node(
            CONTROLLER,
            IrNodeKind::Controller {
                kind: crate::controller::ControllerKind::ModWheel,
            },
            ExecutionScope::Voice,
        )
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
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().expect("tuning"),
        )
        .declaring(PlanDeclarations {
            note_producers: if compiled_first {
                vec![compiled, live]
            } else {
                vec![live, compiled]
            },
            held_notes: HeldNoteCount::measured(4),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("readable mixed graph");
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .expect("profile");
    let plan = compile(&ir, &RenderConfig::new(profile))
        .into_plan()
        .expect("admitted plan");
    let slot = plan.resolve_note(ENVELOPE).expect("playable envelope");
    let stream = AdmittedCompiledStream::admit(&plan, &[]).expect("empty compiled stream");
    (
        MixedTargetAdmission::admit(plan, stream, slot).expect("disjoint mixed targets"),
        slot,
    )
}

fn renderers(
    binding: &MixedTargetAdmission,
    note: crate::plan::NoteSlot,
) -> (
    PreparedRenderer,
    PreparedRenderer,
    NoteIdentity,
    StreamEpoch,
) {
    let mut table = IdentityTable::from_admitted_ranges(binding.plan().note_producer_ranges())
        .expect("identity ranges");
    let identity = table
        .mint(binding.live_producer(), note)
        .expect("live range has credit");
    let epoch = issue_epoch().expect("stream epoch");
    let plain =
        PreparedRenderer::prepare(Arc::clone(binding.plan_arc()), ANCHOR, epoch, table.id())
            .expect("plain renderer");
    let mut mixed =
        PreparedRenderer::prepare(Arc::clone(binding.plan_arc()), ANCHOR, epoch, table.id())
            .expect("mixed renderer");
    assert!(mixed.bind_mixed_partition(Arc::clone(binding.partition_arc())));
    (plain, mixed, identity, epoch)
}

fn group(binding: &MixedTargetAdmission, slot: ParameterSlot) -> MixedRestorationGroup {
    *binding
        .instance_partition()
        .restoration_groups()
        .iter()
        .find(|group| group.parameter() == slot)
        .expect("addressable local group")
}

fn note_events(
    epoch: StreamEpoch,
    identity: NoteIdentity,
    note: crate::plan::NoteSlot,
) -> [TimedEvent; 2] {
    [
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Simulated),
            EventPayload::Note {
                identity,
                edge: NoteEdge::On {
                    slot: note,
                    key: KeyIdentity::LOWEST,
                    velocity: NoteVelocity::FULL,
                },
            },
        ),
        TimedEvent::new(
            EventEnvelope::new(
                epoch,
                SampleTime::new((2 * Q) as u64),
                TimeSource::Simulated,
            ),
            EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            },
        ),
    ]
}

fn render(renderer: &mut PreparedRenderer, events: &[TimedEvent]) -> Vec<f32> {
    let mut samples = vec![0.0_f32; 4 * Q];
    let block = AudioBlockMut::new(&mut samples, 4 * Q, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(block, TimedEvents::new(events))
        .expect("bounded render");
    samples
}

fn states(renderer: &PreparedRenderer, rows: &[crate::plan::ParameterRow]) -> Vec<SlotState> {
    rows.iter()
        .map(|row| renderer.parameter_slots[row.index()])
        .collect()
}

#[test]
fn scoped_restoration_keeps_live_rows_and_audio_in_both_producer_orders() {
    for compiled_first in [true, false] {
        let (binding, note) = binding(compiled_first);
        let frequency = binding
            .plan()
            .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
            .expect("sine frequency");
        let gate = binding.plan().note_targets()[note.index()].parameter;
        let (mut plain, mut mixed, identity, epoch) = renderers(&binding, note);
        let before_live = states(&mixed, binding.instance_partition().live_rows());
        assert!(!binding.instance_partition().global_rows().is_empty());
        let before_global = states(&mixed, binding.instance_partition().global_rows());
        let gate_span = group(&binding, gate).instances();
        for instance in gate_span.first()..gate_span.first() + gate_span.count() {
            let row = gate.index() + instance as usize;
            let _ = mixed.parameter_slots[row].write_override(value(1.0));
        }
        mixed.seed_for_adoption();
        assert_eq!(
            states(&mixed, binding.instance_partition().live_rows()),
            before_live,
            "a compiled adoption cannot mark a live row for a step"
        );
        assert_eq!(
            states(&mixed, binding.instance_partition().global_rows()),
            before_global,
            "a compiled adoption cannot seed a global row"
        );
        let notes = note_events(epoch, identity, note);
        let expected = render(&mut plain, &notes);
        let scoped = [
            notes[0],
            TimedEvent::new(
                EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
                EventPayload::ScopedRestore(ScopedParameterRestore::override_for(
                    group(&binding, frequency),
                    value(330.0),
                )),
            ),
            TimedEvent::new(
                EventEnvelope::new(epoch, SampleTime::new((Q / 2) as u64), TimeSource::Compiled),
                EventPayload::ScopedRestore(ScopedParameterRestore::override_for(
                    group(&binding, gate),
                    ParameterValue::ZERO,
                )),
            ),
            notes[1],
        ];
        let actual = render(&mut mixed, &scoped);
        assert_eq!(
            actual, expected,
            "compiled restoration changed a held live voice"
        );
        assert!(actual.iter().any(|sample| *sample != 0.0));
        assert_eq!(mixed.diagnostics().foreign_slot_events(), 0);
        assert_eq!(
            states(&mixed, binding.instance_partition().live_rows()),
            states(&plain, binding.instance_partition().live_rows()),
        );
        assert_eq!(
            states(&mixed, binding.instance_partition().global_rows()),
            states(&plain, binding.instance_partition().global_rows()),
        );
        let span = group(&binding, frequency).instances();
        for instance in span.first()..span.first() + span.count() {
            let row = frequency.index() + instance as usize;
            assert_eq!(mixed.parameter_slots[row].automated(), value(330.0));
        }
        for instance in gate_span.first()..gate_span.first() + gate_span.count() {
            let row = gate.index() + instance as usize;
            assert_eq!(mixed.parameter_slots[row].automated(), ParameterValue::ZERO);
            assert_eq!(mixed.parameter_slots[row].current(), ParameterValue::ZERO);
        }
    }
}

#[test]
fn a_scoped_event_crossing_into_live_rows_is_refused_before_writing() {
    let (binding, note) = binding(true);
    let frequency = binding
        .plan()
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("frequency");
    let (mut plain, mut mixed, identity, epoch) = renderers(&binding, note);
    let notes = note_events(epoch, identity, note);
    let expected = render(&mut plain, &notes);
    let live_span = ParameterInstanceSpan::checked(2, 2, VoiceCount::measured(4))
        .expect("the span fits the group but belongs to live input");
    let forged = ScopedParameterRestore {
        slot: frequency,
        instances: live_span,
        value: value(660.0),
        controller: None,
    };
    let events = [
        notes[0],
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
            EventPayload::ScopedRestore(forged),
        ),
        notes[1],
    ];
    let actual = render(&mut mixed, &events);
    assert_eq!(actual, expected);
    assert_eq!(mixed.diagnostics().foreign_slot_events(), 1);
    assert_eq!(
        states(&mixed, binding.instance_partition().live_rows()),
        states(&plain, binding.instance_partition().live_rows()),
    );
}

#[test]
fn an_ordinary_renderer_has_no_scoped_restoration_authority() {
    let (binding, note) = binding(true);
    let frequency = binding
        .plan()
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("frequency");
    let (mut plain, _mixed, identity, epoch) = renderers(&binding, note);
    let notes = note_events(epoch, identity, note);
    let event = TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::ScopedRestore(ScopedParameterRestore::override_for(
            group(&binding, frequency),
            value(660.0),
        )),
    );
    let before = plain.parameter_slots.clone();
    let events = [notes[0], event, notes[1]];
    let _ = render(&mut plain, &events);
    assert_eq!(plain.diagnostics().foreign_slot_events(), 1);
    let span = group(&binding, frequency).instances();
    for instance in span.first()..span.first() + span.count() {
        let row = frequency.index() + instance as usize;
        assert_eq!(
            plain.parameter_slots[row].automated(),
            before[row].automated()
        );
    }
}

#[test]
fn a_scoped_adoption_does_not_change_the_next_live_ramp() {
    let (binding, note) = binding(true);
    let (mut plain, mut mixed, _, _) = renderers(&binding, note);
    let frequency = binding
        .plan()
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("frequency");
    let live_row = frequency.index() + 2;
    plain.parameter_slots[live_row].smooth_over(2 * QUANTUM_FRAMES);
    mixed.parameter_slots[live_row].smooth_over(2 * QUANTUM_FRAMES);
    let _ = plain.parameter_slots[live_row].write_override(value(440.0));
    let _ = mixed.parameter_slots[live_row].write_override(value(440.0));
    mixed.seed_for_adoption();
    assert_eq!(
        mixed.parameter_slots[live_row],
        plain.parameter_slots[live_row]
    );
    let _ = plain.parameter_slots[live_row].write_override(value(660.0));
    let _ = mixed.parameter_slots[live_row].write_override(value(660.0));
    assert_eq!(
        mixed.parameter_slots[live_row],
        plain.parameter_slots[live_row]
    );
    let modulation = crate::node::ModulationSum::new(12.0).expect("finite modulation");
    let _ = plain.parameter_slots[live_row].modulate(modulation);
    let _ = mixed.parameter_slots[live_row].modulate(modulation);
    assert_eq!(
        mixed.parameter_slots[live_row], plain.parameter_slots[live_row],
        "a compiled adoption cannot seed the next live modulation"
    );
}

#[test]
fn scoped_controller_restoration_spends_the_compiled_seed_once() {
    let (binding, note) = binding(true);
    let controller = binding
        .plan()
        .resolve_parameter(CONTROLLER, parameters::SOURCE_VALUE)
        .expect("controller source value");
    let group = group(&binding, controller);
    let (mut plain, mut mixed, _, epoch) = renderers(&binding, note);
    let live_row = controller.index() + 2;
    let compiled_row = controller.index();
    mixed.parameter_slots[compiled_row].smooth_over(10 * QUANTUM_FRAMES);
    let _ = mixed.parameter_slots[compiled_row].control(Some(value(0.25)));
    let before_live = mixed.parameter_slots[live_row];
    mixed.seed_for_adoption();
    let event = TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::ScopedRestore(ScopedParameterRestore::controller_for(
            group,
            value(0.8),
            None,
        )),
    );
    let _ = render(&mut mixed, &[event]);
    let _ = render(&mut plain, &[]);
    assert_eq!(mixed.parameter_slots[compiled_row].current(), value(0.8));
    assert_eq!(mixed.parameter_slots[live_row], before_live);
    assert_eq!(
        mixed.parameter_slots[live_row],
        plain.parameter_slots[live_row]
    );
}

#[test]
fn scoped_render_resolves_without_audio_thread_allocation() {
    let (binding, note) = binding(true);
    let frequency = binding
        .plan()
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("frequency");
    let (_plain, mut mixed, _, epoch) = renderers(&binding, note);
    let event = TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::ScopedRestore(ScopedParameterRestore::override_for(
            group(&binding, frequency),
            value(330.0),
        )),
    );
    let events = [event];
    let mut samples = vec![0.0_f32; 2 * Q];
    let block = AudioBlockMut::new(&mut samples, 2 * Q, ChannelLayout::Mono).expect("shaped block");
    let allocations = crate::render_allocation::count_allocs(|| {
        mixed
            .render(block, TimedEvents::new(&events))
            .expect("bounded render");
    });
    assert_eq!(allocations, 0);
}
