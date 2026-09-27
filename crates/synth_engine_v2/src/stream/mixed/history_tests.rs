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
    Amplitude, Cents, ChannelLayout, EventCount, Frequency, KeyIdentity, NormalizedLevel,
    NoteVelocity, SampleRate, Seconds,
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

#[test]
fn suffix_pairs_repeated_keys_first_and_counts_only_crossing_obligations() {
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
                    PlanPosition::new(15),
                    CompiledPayload::Bend {
                        slot: note,
                        key: key(60),
                        cents: Cents::new(50.0).expect("finite bend"),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(20),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(25),
                    CompiledPayload::Bend {
                        slot: note,
                        key: key(60),
                        cents: Cents::new(25.0).expect("finite bend"),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(26),
                    CompiledPayload::Expression {
                        slot: note,
                        key: key(60),
                        expression: crate::controller::NoteExpression::Pressure(
                            NormalizedLevel::FULL,
                        ),
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
        let table_before = prepared.table_id();
        let outstanding_before = prepared.outstanding_count();
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("prefix note is open");
        let destination_open = history.open_at_destination.clone();
        let suffix = prepared.prepare_suffix(history).expect("bound suffix");
        assert_eq!(suffix.included, vec![1, 2, 3]);
        assert_eq!(suffix.included_event_count().get(), 3);
        assert_eq!(suffix.omitted_expression_count().get(), 2);
        assert_eq!(suffix.omitted_release_count().get(), 1);
        assert_eq!(suffix.open_at_destination_count().get(), 1);
        assert_eq!(suffix.history.open_at_destination, destination_open);
        assert!(suffix.history.book.entries().is_empty());
        assert_eq!(prepared.table_id(), table_before);
        assert_eq!(prepared.outstanding_count(), outstanding_before);
    }
}

#[test]
fn a_suffix_candidate_cannot_be_used_by_another_prepared_owner() {
    let prepared = bound(true);
    let other = bound(true);
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(20))
        .expect("prefix");
    assert_eq!(
        other
            .prepare_suffix(history)
            .expect_err("foreign table and epoch"),
        MixedSuffixPrepareError::ForeignCandidate
    );
    assert_eq!(other.outstanding_count(), 0);
}

#[test]
fn crossing_release_is_absent_from_private_suffix_selection() {
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
                        key: key(72),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(20),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(30),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(72),
                    },
                ),
            ]
        });
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("one prefix note");
        let suffix = prepared.prepare_suffix(history).expect("bound suffix");
        assert_eq!(suffix.included, vec![1, 3]);
        assert_eq!(suffix.omitted_release_count().get(), 1);
        assert_eq!(suffix.omitted_expression_count().get(), 0);
        assert_eq!(suffix.open_at_destination_count().get(), 1);
    }
}

#[test]
fn private_stamp_releases_only_its_copy_and_orders_restoration_before_destination_on() {
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
                        key: key(72),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(20),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
            ]
        });
        let original = prepared.outstanding[0];
        assert_eq!(
            prepared.owner.control.minter.resolve(original),
            Resolution::Live
        );
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("prefix");
        let suffix = prepared.prepare_suffix(history).expect("suffix selection");
        assert_eq!(suffix.included, vec![1]);
        let stamped = prepared.stamp_suffix(suffix).expect("private stamp");
        let restoration_count = prepared.owner.control.partition.restoration_groups().len();
        assert_eq!(
            stamped.anchor(),
            StreamAnchor::new(SampleTime::new(64), PlanPosition::new(10))
        );
        assert_eq!(stamped.supersedes(), ActivationSequence::INITIAL);
        assert_eq!(
            stamped.restoration_count().as_usize(),
            Some(restoration_count)
        );
        assert!(stamped.suffix.history.restoration.is_empty());
        assert_eq!(stamped.suffix.history.restoration_count(), EventCount::NONE);
        assert_eq!(
            stamped.event_count().as_usize(),
            Some(restoration_count + 1)
        );
        assert_eq!(stamped.outstanding_count().get(), 1);
        assert_eq!(stamped.minter.live(), 1);
        assert_eq!(stamped.outstanding.len(), 1);
        assert_eq!(
            stamped.minter.resolve(stamped.outstanding[0]),
            Resolution::Live
        );
        assert_eq!(
            prepared.owner.control.minter.resolve(original),
            Resolution::Live,
            "the authoritative compiled range cannot be released by rehearsal"
        );
        for event in &stamped.events[..restoration_count] {
            assert!(matches!(event.payload(), EventPayload::ScopedRestore(_)));
            assert_eq!(event.envelope().time(), SampleTime::new(64));
        }
        let last = stamped.events[restoration_count];
        assert!(matches!(
            last.payload(),
            EventPayload::Note {
                edge: crate::render::NoteEdge::On { key: on_key, .. },
                ..
            } if on_key == key(72)
        ));
        assert_eq!(last.envelope().time(), SampleTime::new(64));
        let timing = stamped
            .effective_timing(SampleTime::new(128))
            .expect("next boundary fits the complete private list");
        assert_eq!(timing.effective(), SampleTime::new(128));
        assert_eq!(timing.shift(), FrameCount::new(64));
        assert!(stamped.events.iter().all(|event| {
            event.envelope().time().checked_add(timing.shift()) == Ok(SampleTime::new(128))
        }));
        assert_eq!(stamped.events[0].envelope().time(), SampleTime::new(64));
        assert_eq!(
            stamped
                .effective_timing(SampleTime::new(0))
                .expect_err("earlier boundary"),
            MixedEffectiveTimeError::BeforeRequested {
                requested: SampleTime::new(64),
                effective: SampleTime::ZERO,
            }
        );
        assert_eq!(
            stamped
                .effective_timing(SampleTime::new(65))
                .expect_err("incomplete quantum"),
            MixedEffectiveTimeError::NotQuantumBoundary {
                effective: SampleTime::new(65),
            }
        );
        let last_boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
        assert_eq!(
            stamped.effective_timing(SampleTime::new(last_boundary)),
            Err(MixedEffectiveTimeError::DisplacementUnrepresentable {
                requested: SampleTime::new(64),
                effective: SampleTime::new(last_boundary),
            })
        );
        assert!(
            stamped
                .events
                .iter()
                .all(|event| !matches!(event.payload(), EventPayload::SetParameter { .. }))
        );
        assert!(stamped.outstanding.iter().all(|identity| {
            prepared
                .owner
                .control
                .minter
                .span()
                .contains(identity.index())
        }));
    }
}

#[test]
fn private_effective_timing_refuses_suffix_overflow_after_restoration_fits() {
    let prepared = bound_with_events(true, |note| {
        vec![
            PlanEvent::new(
                PlanPosition::new(10),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(200),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ),
        ]
    });
    let last_boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
    let requested = SampleTime::new(last_boundary - 128);
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("prefix");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let stamped = prepared.stamp_suffix(suffix).expect("private stamp");
    let shift = FrameCount::new(128);
    let restoration_count = stamped
        .restoration_count()
        .as_usize()
        .expect("represented restoration count");
    assert!(restoration_count > 0);
    for event in &stamped.events[..restoration_count] {
        assert!(matches!(event.payload(), EventPayload::ScopedRestore(_)));
        assert_eq!(
            event.envelope().time().checked_add(shift),
            Ok(SampleTime::new(last_boundary))
        );
    }
    let overflowing_index = stamped
        .events
        .iter()
        .position(|event| event.envelope().time() > requested)
        .expect("suffix has a later edge");
    assert!(overflowing_index > restoration_count);
    assert_eq!(
        stamped.effective_timing(SampleTime::new(last_boundary)),
        Err(MixedEffectiveTimeError::EventTimeUnrepresentable {
            event_index: overflowing_index,
            time: stamped.events[overflowing_index].envelope().time(),
            shift,
        })
    );
    assert_eq!(stamped.anchor().time(), requested);
}

#[test]
fn private_stamp_refuses_unrepresentable_suffix_time_and_invalid_selection() {
    let prepared = bound_with_events(true, |note| {
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
                    key: key(60),
                },
            ),
            PlanEvent::new(
                PlanPosition::new(200),
                CompiledPayload::Bend {
                    slot: note,
                    key: key(72),
                    cents: Cents::new(25.0).expect("finite bend"),
                },
            ),
        ]
    });
    let original = prepared.outstanding[0];
    assert_eq!(
        prepared.owner.control.minter.resolve(original),
        Resolution::Live
    );
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let mut suffix = prepared.prepare_suffix(history).expect("suffix selection");
    suffix.included[0] = usize::MAX;
    assert_eq!(
        prepared
            .stamp_suffix(suffix)
            .expect_err("invalid private index"),
        MixedStampPrepareError::InvalidSelection {
            event_index: usize::MAX
        }
    );
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let mut suffix = prepared.prepare_suffix(history).expect("suffix selection");
    suffix.included.swap(0, 1);
    assert_eq!(
        prepared
            .stamp_suffix(suffix)
            .expect_err("reordered indices"),
        MixedStampPrepareError::InvalidSelection { event_index: 1 }
    );
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let mut suffix = prepared.prepare_suffix(history).expect("suffix selection");
    suffix.included[0] = 0;
    assert_eq!(
        prepared
            .stamp_suffix(suffix)
            .expect_err("prefix source selected"),
        MixedStampPrepareError::InvalidSelection { event_index: 0 }
    );

    let last_boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
    let history = prepared
        .prepare_history(SampleTime::new(last_boundary), PlanPosition::new(10))
        .expect("representable boundary");
    let suffix = prepared.prepare_suffix(history).expect("suffix selection");
    assert_eq!(
        prepared
            .stamp_suffix(suffix)
            .expect_err("placed bend exceeds engine time"),
        MixedStampPrepareError::Schedule(SchedulePrepareError::TimeUnrepresentable {
            event_index: 3,
            position: PlanPosition::new(200),
        })
    );
    assert_eq!(prepared.outstanding_count(), 1);
    assert_eq!(
        prepared.owner.control.minter.resolve(original),
        Resolution::Live
    );
}

#[test]
fn private_stamp_refuses_inconsistent_old_reservation_custody() {
    let make_prepared = || {
        bound_with_events(true, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        })
    };
    let mut missing = make_prepared();
    missing.outstanding.clear();
    let history = missing
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let suffix = missing.prepare_suffix(history).expect("empty suffix");
    assert_eq!(
        missing
            .stamp_suffix(suffix)
            .expect_err("unlisted live index"),
        MixedStampPrepareError::OldReservationsRemain {
            live: HeldNoteCount::measured(1),
        }
    );

    let mut duplicate = make_prepared();
    let identity = duplicate.outstanding[0];
    duplicate.outstanding.push(identity);
    let history = duplicate
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let suffix = duplicate.prepare_suffix(history).expect("empty suffix");
    assert_eq!(
        duplicate
            .stamp_suffix(suffix)
            .expect_err("duplicate reservation"),
        MixedStampPrepareError::StaleReservation {
            identity,
            resolution: Resolution::Orphan(crate::identity::OrphanCause::FreeIndex),
        }
    );
}
