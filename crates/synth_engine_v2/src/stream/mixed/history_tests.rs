//! Bound, private mixed history: scoped batches carry compiled-only spans.

use super::*;
use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain, parameters,
};
use crate::plan::{CompiledPlan, ControlRate, ParameterSlot};
use crate::profile::HostProfile;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, KeyIdentity, NormalizedLevel, NoteVelocity,
    SampleRate, Seconds,
};
use crate::sample::{
    PlayDirection, PlayMode, PlaybackRegion, PreparedSample, SampleFrame, SampleMap, SampleMapRef,
    SampleRef, SampleZone,
};
use crate::schedule::PlanEvent;
use crate::time::FrameCount;

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const CONTROLLER: NodeId = NodeId::new(5);
const GLOBAL: NodeId = NodeId::new(6);
const NOTE_SOURCE: NodeId = NodeId::new(7);

fn key(raw: u8) -> KeyIdentity {
    KeyIdentity::new(raw).expect("valid key")
}

fn bound(compiled_first: bool) -> MixedJoinedPrepared {
    bound_with_events(compiled_first, |note| {
        vec![
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(10),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(72),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(20),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(72),
                },
            ),
            PlanEvent::new(
                PlanPosition::new(30),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ),
        ]
    })
}

fn bound_with_events(
    compiled_first: bool,
    events: impl FnOnce(NoteSlot) -> Vec<PlanEvent>,
) -> MixedJoinedPrepared {
    let plan = mixed_plan(compiled_first);
    let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
    let events = events(note);
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("admitted stream");
    let binding = MixedTargetAdmission::admit(plan, stream, note).expect("mixed target binding");
    MixedJoinedStream::open(
        binding,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("joined owner")
    .prepare_initial()
    .expect("stamped initial schedule")
}

fn mixed_plan(compiled_first: bool) -> CompiledPlan {
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
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(
            CONTROLLER,
            IrNodeKind::Controller {
                kind: crate::controller::ControllerKind::ModWheel,
            },
            ExecutionScope::Voice,
        )
        .node(
            NOTE_SOURCE,
            IrNodeKind::NoteSource {
                kind: crate::controller::NoteSource::Velocity,
            },
            ExecutionScope::Voice,
        )
        .node(
            GLOBAL,
            IrNodeKind::Sine {
                frequency: Frequency::new(110.0).expect("finite frequency"),
                amplitude: Amplitude::new(0.1).expect("finite amplitude"),
            },
            ExecutionScope::Global,
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
        .expect("mixed graph");
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .expect("profile");
    compile(&ir, &RenderConfig::new(profile))
        .into_plan()
        .expect("plan")
}

fn restore(candidate: &MixedHistoryCandidate, slot: ParameterSlot) -> ScopedParameterRestore {
    candidate
        .restoration
        .iter()
        .find_map(|event| match event.payload() {
            EventPayload::ScopedRestore(restore) if restore.slot() == slot => Some(restore),
            _ => None,
        })
        .expect("one scoped event for the group")
}

#[test]
fn prefix_restores_last_magnitude_and_zero_gate_in_both_producer_orders() {
    for compiled_first in [true, false] {
        let prepared = bound(compiled_first);
        let plan = &prepared.owner.control.plan;
        let partition = &prepared.owner.control.partition;
        let note = plan.resolve_note(ENVELOPE).expect("note");
        let gate = plan.note_targets()[note.index()].parameter;
        let frequency = plan
            .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
            .expect("frequency");
        let controller = plan
            .resolve_parameter(CONTROLLER, parameters::SOURCE_VALUE)
            .expect("controller");
        let note_source = plan
            .resolve_parameter(NOTE_SOURCE, parameters::SOURCE_VALUE)
            .expect("note source");
        let table_before = prepared.table_id();
        let outstanding_before = prepared.outstanding_count();
        let candidate = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(30))
            .expect("bounded prefix");
        assert_eq!(candidate.plan_id(), plan.id());
        assert_eq!(candidate.epoch(), prepared.epoch());
        assert_eq!(candidate.table_id(), table_before);
        assert_eq!(candidate.requested(), SampleTime::new(64));
        assert_eq!(candidate.position(), PlanPosition::new(30));
        assert_eq!(candidate.prefix_end(), 3);
        assert_eq!(candidate.open_at_destination_count().get(), 1);
        assert_eq!(candidate.book.entries().len(), 1);
        assert_eq!(candidate.open_at_destination.len(), 1);
        assert_eq!(prepared.table_id(), table_before);
        assert_eq!(prepared.outstanding_count(), outstanding_before);
        assert_eq!(
            candidate.restoration_count().get() as usize,
            partition.restoration_groups().len()
        );
        assert_eq!(restore(&candidate, gate).value(), ParameterValue::ZERO);
        assert_eq!(restore(&candidate, gate).controller(), None);
        let expected_pitch = ParameterValue::from_frequency(
            crate::tuning::PreparedTuning::equal_temperament()
                .expect("tuning")
                .frequency_of(key(72)),
        );
        assert_eq!(restore(&candidate, frequency).value(), expected_pitch);
        assert_eq!(restore(&candidate, frequency).controller(), None);
        assert_eq!(
            restore(&candidate, controller).value(),
            plan.parameter_targets()[controller.index()].base
        );
        assert_eq!(restore(&candidate, controller).controller(), Some(None));
        assert_eq!(
            restore(&candidate, note_source).value(),
            plan.parameter_targets()[note_source.index()].base
        );
        for event in &candidate.restoration {
            let EventPayload::ScopedRestore(scoped) = event.payload() else {
                panic!("private batch must be scoped");
            };
            assert_eq!(event.envelope().time(), SampleTime::new(64));
            assert!(partition.restoration_groups().iter().any(|group| {
                group.parameter() == scoped.slot() && group.instances() == scoped.instances()
            }));
        }
    }
}

#[test]
fn destination_events_stay_in_suffix_and_unrepresentable_time_refuses_without_mutation() {
    let prepared = bound(false);
    let at_off = prepared
        .prepare_history(SampleTime::ZERO, PlanPosition::new(20))
        .expect("off at destination belongs to suffix");
    assert_eq!(at_off.prefix_end(), 2);
    assert_eq!(at_off.open_at_destination_count().get(), 2);
    let table_before = prepared.table_id();
    let outstanding_before = prepared.outstanding_count();
    assert_eq!(
        prepared
            .prepare_history(SampleTime::new(u64::MAX), PlanPosition::new(20))
            .expect_err("no next boundary"),
        MixedHistoryPrepareError::BoundaryUnrepresentable {
            at: SampleTime::new(u64::MAX)
        }
    );
    assert_eq!(prepared.table_id(), table_before);
    assert_eq!(prepared.outstanding_count(), outstanding_before);
}

#[test]
fn equal_position_order_and_repeated_key_pairing_are_retained() {
    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![
                PlanEvent::new(
                    PlanPosition::ZERO,
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(72),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(20),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(72),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(30),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
            ]
        });
        let before = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("destination events are suffix events");
        assert_eq!(before.prefix_end(), 1);
        assert_eq!(before.open_at_destination.len(), 1);
        assert_eq!(before.open_at_destination[0].key, key(60));

        let after = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(20))
            .expect("same-position prefix is admitted in order");
        assert_eq!(after.prefix_end(), 4);
        assert_eq!(after.open_at_destination.len(), 2);
        assert_eq!(after.open_at_destination[0].key, key(60));
        assert_eq!(after.open_at_destination[0].opened, 0);
        assert_eq!(after.open_at_destination[1].key, key(72));
        let frequency = prepared
            .owner
            .control
            .plan
            .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
            .expect("frequency");
        let expected_pitch = ParameterValue::from_frequency(
            crate::tuning::PreparedTuning::equal_temperament()
                .expect("tuning")
                .frequency_of(key(72)),
        );
        assert_eq!(restore(&after, frequency).value(), expected_pitch);
    }
}

#[test]
fn sample_positioned_controller_is_refused_at_target_binding() {
    for compiled_first in [true, false] {
        let mut plan = mixed_plan(compiled_first);
        let controller = plan
            .resolve_parameter(CONTROLLER, parameters::SOURCE_VALUE)
            .expect("controller source value");
        assert!(plan.set_parameter_rate_for_test(controller, ControlRate::Sample));
        let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
        let stream = AdmittedCompiledStream::admit(&plan, &[]).expect("empty stream");
        let failure = MixedTargetAdmission::admit(plan, stream, note)
            .expect_err("sample-positioned controller cannot bind");
        assert_eq!(
            failure.reason(),
            crate::host::mixed_targets::MixedTargetError::SampleController { slot: controller }
        );
        let (plan, stream, returned_note) = failure.into_inputs();
        assert_eq!(returned_note, note);
        assert_eq!(stream.plan(), plan.id());
    }
}

#[test]
fn destination_open_sampler_trigger_is_part_of_zeroed_history() {
    for compiled_first in [true, false] {
        let rate = SampleRate::new(48_000.0).expect("rate");
        let sample = PreparedSample::prepare(vec![0.25; 4096], ChannelLayout::Mono, rate)
            .expect("finite sample");
        let region = PlaybackRegion::new(SampleFrame::new(0), SampleFrame::new(4096))
            .expect("nonempty region");
        let compiled = NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(1),
            simultaneous_holds: EventCount::NONE,
        };
        let live = NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(1),
            simultaneous_holds: EventCount::measured(1),
        };
        let ir = GraphIr::builder()
            .sample(sample)
            .sample_map(SampleMap::new(vec![SampleZone::new(
                SampleRef::new(0),
                KeyIdentity::LOWEST,
                region,
            )]))
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
                held_notes: HeldNoteCount::measured(2),
                ..PlanDeclarations::default()
            })
            .build()
            .expect("sampler graph");
        let profile =
            HostProfile::harness(rate, FrameCount::new(512), ChannelLayout::Mono).expect("profile");
        let plan = compile(&ir, &RenderConfig::new(profile))
            .into_plan()
            .expect("sampler plan");
        let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
        let trigger = plan
            .resolve_parameter(SOURCE, parameters::SAMPLER_TRIGGER)
            .expect("sampler trigger");
        assert!(super::super::gate_rows(&plan)[note.index()].contains(&trigger.index()));
        let events = [
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: KeyIdentity::LOWEST,
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(30),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: KeyIdentity::LOWEST,
                },
            ),
        ];
        let stream = AdmittedCompiledStream::admit(&plan, &events).expect("sampler stream");
        let binding = MixedTargetAdmission::admit(plan, stream, note).expect("sampler binding");
        let prepared = MixedJoinedStream::open(
            binding,
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .expect("sampler owner")
        .prepare_initial()
        .expect("initial sampler schedule");
        let candidate = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(20))
            .expect("sampler prefix");
        assert_eq!(candidate.open_at_destination_count().get(), 1);
        assert_eq!(restore(&candidate, trigger).value(), ParameterValue::ZERO);
    }
}
