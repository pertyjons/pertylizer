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
use crate::render::{AudioBlockMut, NoteEdge, Renderer, TimedEvents};
use crate::sample::{
    PlayDirection, PlayMode, PlaybackRegion, PreparedSample, SampleFrame, SampleMap, SampleMapRef,
    SampleRef, SampleZone,
};
use crate::schedule::PlanEvent;
use crate::time::{FrameCount, TimeSource as TestOrigin};

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

fn render_quantum(renderer: &mut PreparedRenderer, events: &[TimedEvent]) -> Vec<f32> {
    let mut samples = vec![0.0_f32; crate::time::QUANTUM_FRAMES as usize];
    let frames = samples.len();
    let output = AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("mono block");
    renderer
        .render(output, TimedEvents::new(events))
        .expect("admitted quantum");
    samples
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
fn private_audio_capsule_is_sendable_without_moving_control_or_source_history() {
    fn require_send<T: Send>() {}
    require_send::<MixedAudioCandidate>();

    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        });
        let old = prepared.outstanding[0];
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("prefix");
        let suffix = prepared.prepare_suffix(history).expect("suffix");
        let omitted = (
            suffix.omitted_release_count(),
            suffix.omitted_expression_count(),
        );
        let stamped = prepared.stamp_suffix(suffix).expect("private stamp");
        let event_count = stamped.event_count();
        let restoration_count = stamped.restoration_count();
        let outstanding_count = stamped.outstanding_count();
        let capsule = stamped.into_audio_capsule().expect("successor sequence");
        assert_eq!(capsule.plan, prepared.owner.control.plan.id());
        assert_eq!(capsule.epoch, prepared.epoch());
        assert_eq!(capsule.table, prepared.table_id());
        assert_eq!(
            capsule.anchor,
            StreamAnchor::new(SampleTime::new(64), PlanPosition::new(10))
        );
        assert_eq!(capsule.supersedes, ActivationSequence::INITIAL);
        assert_eq!(
            capsule.sequence,
            ActivationSequence::INITIAL.next().unwrap()
        );
        assert_eq!(capsule.event_count, event_count);
        assert_eq!(capsule.events.len(), event_count.as_usize().unwrap());
        assert_eq!(capsule.restoration_count, restoration_count);
        assert_eq!(capsule.outstanding_count, outstanding_count);
        assert_eq!(
            capsule.outstanding.len(),
            outstanding_count.as_usize().unwrap()
        );
        assert_eq!(
            (capsule.omitted_releases, capsule.omitted_expressions),
            omitted
        );
        assert_eq!(capsule.minter.live(), outstanding_count.get());
        assert_eq!(
            prepared.owner.control.minter.resolve(old),
            Resolution::Live,
            "boxing a copied minter must not alter authoritative custody"
        );
        drop(capsule);
    }
}

#[test]
fn private_one_shot_arm_fixes_boundary_and_splits_identity_custody() {
    fn require_send<T: Send>() {}
    require_send::<MixedOneShotAudio>();

    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        });
        let old = prepared.outstanding[0];
        let history = prepared
            .prepare_history(SampleTime::new(65), PlanPosition::new(10))
            .expect("bound history");
        let suffix = prepared.prepare_suffix(history).expect("bound suffix");
        let omitted = (
            suffix.omitted_release_count(),
            suffix.omitted_expression_count(),
        );
        let candidate = prepared.stamp_suffix(suffix).expect("private stamp");
        let restoration_count = candidate.restoration_count();
        let (control, audio) = prepared.arm_one_shot(candidate).expect("one arm");
        assert_eq!(control.control.minter.resolve(old), Resolution::Live);
        assert_eq!(control.outstanding, vec![old]);
        assert_eq!(audio.audio.renderer.clock(), SampleTime::ZERO);
        assert_eq!(audio.events[0].envelope().time(), SampleTime::ZERO);
        assert_eq!(audio.next, 0);
        assert_eq!(audio.timing.effective(), SampleTime::new(128));
        assert_eq!(audio.timing.shift(), FrameCount::new(63));
        assert_eq!(
            audio.effective_anchor,
            StreamAnchor::new(SampleTime::new(128), PlanPosition::new(10))
        );
        assert!(!audio.late_at_arm);
        assert_eq!(audio.in_force, ActivationSequence::INITIAL);
        assert_eq!(audio.capsule.supersedes, audio.in_force);
        assert_eq!(audio.capsule.sequence, audio.in_force.next().unwrap());
        assert_eq!(audio.capsule.plan, control.control.plan.id());
        assert_eq!(audio.capsule.epoch, control.control.epoch());
        assert_eq!(audio.capsule.table, control.control.table_id());
        assert_eq!(
            audio.capsule.anchor,
            StreamAnchor::new(SampleTime::new(65), PlanPosition::new(10))
        );
        assert_eq!(
            audio.capsule.events.len(),
            audio.capsule.event_count.as_usize().unwrap()
        );
        assert_eq!(
            audio.capsule.outstanding.len(),
            audio.capsule.outstanding_count.as_usize().unwrap()
        );
        assert_eq!(
            audio.capsule.minter.live(),
            audio.capsule.outstanding_count.get()
        );
        assert_eq!(audio.capsule.restoration_count, restoration_count);
        assert_eq!(
            (
                audio.capsule.omitted_releases,
                audio.capsule.omitted_expressions
            ),
            omitted
        );
    }
}

#[test]
fn private_one_shot_arm_returns_foreign_candidate_without_changing_owner() {
    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        });
        let foreign = bound(!compiled_first);
        let history = foreign
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("foreign history");
        let suffix = foreign.prepare_suffix(history).expect("foreign suffix");
        let candidate = foreign.stamp_suffix(suffix).expect("foreign stamp");
        let epoch = prepared.epoch();
        let table = prepared.table_id();
        let original = prepared.outstanding[0];
        let count = prepared.event_count();
        let refused = prepared.arm_one_shot(candidate).expect_err("foreign epoch");
        assert_eq!(refused.reason, MixedOneShotArmError::ForeignCandidate);
        assert_eq!(refused.owner.epoch(), epoch);
        assert_eq!(refused.owner.table_id(), table);
        assert_eq!(refused.owner.event_count(), count);
        assert_eq!(refused.owner.owner.audio.renderer.clock(), SampleTime::ZERO);
        assert_eq!(
            refused.owner.owner.control.minter.resolve(original),
            Resolution::Live
        );
        assert_eq!(
            refused.owner.owner.audio.renderer.anchor_for_test(),
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO)
        );
        assert_eq!(refused.candidate.anchor.time(), SampleTime::new(64));
        assert_eq!(
            refused.candidate.event_count().as_usize(),
            Some(refused.candidate.events.len())
        );
        assert_eq!(
            refused.candidate.outstanding_count().as_usize(),
            Some(refused.candidate.outstanding.len())
        );
        let returned = refused.owner;
        let history = returned
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("retry history");
        let suffix = returned.prepare_suffix(history).expect("retry suffix");
        let candidate = returned.stamp_suffix(suffix).expect("retry stamp");
        let (control, audio) = returned.arm_one_shot(candidate).expect("retry arm");
        assert_eq!(control.control.minter.resolve(original), Resolution::Live);
        assert_eq!(audio.timing.effective(), SampleTime::new(64));
    }
}

#[test]
fn private_one_shot_arm_uses_on_grid_request_without_extra_quantum() {
    let prepared = bound(true);
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, audio) = prepared.arm_one_shot(candidate).expect("arm");
    assert_eq!(audio.timing.effective(), SampleTime::new(64));
    assert_eq!(audio.timing.shift(), FrameCount::ZERO);
    assert!(!audio.late_at_arm);
}

#[test]
fn private_one_shot_arm_uses_next_unrendered_quantum_after_request() {
    let mut prepared = bound_with_events(true, |note| {
        vec![PlanEvent::new(
            PlanPosition::new(128),
            CompiledPayload::NoteOn {
                slot: note,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        )]
    });
    let _ = render_quantum(&mut prepared.owner.audio.renderer, &[]);
    let _ = render_quantum(&mut prepared.owner.audio.renderer, &[]);
    assert_eq!(prepared.owner.audio.renderer.clock(), SampleTime::new(64));
    let history = prepared
        .prepare_history(SampleTime::new(32), PlanPosition::ZERO)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, audio) = prepared.arm_one_shot(candidate).expect("arm");
    assert_eq!(audio.timing.effective(), SampleTime::new(64));
    assert_eq!(audio.timing.shift(), FrameCount::new(32));
    assert!(audio.late_at_arm);
    assert_eq!(audio.capsule.anchor.time(), SampleTime::new(32));
}

#[test]
fn private_one_shot_arm_timing_refusal_preserves_both_inputs() {
    let prepared = bound_with_events(false, |note| {
        vec![PlanEvent::new(
            PlanPosition::new(64),
            CompiledPayload::NoteOn {
                slot: note,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        )]
    });
    let boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
    let requested = SampleTime::new(boundary - 1);
    let history = prepared
        .prepare_history(requested, PlanPosition::ZERO)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared
        .stamp_suffix(suffix)
        .expect("stamp at final sample");
    let epoch = prepared.epoch();
    let count = prepared.event_count();
    let original = prepared.outstanding[0];
    let refused = prepared
        .arm_one_shot(candidate)
        .expect_err("shift overflows");
    assert!(matches!(
        refused.reason,
        MixedOneShotArmError::Timing(MixedEffectiveTimeError::EventTimeUnrepresentable { .. })
    ));
    assert_eq!(refused.owner.epoch(), epoch);
    assert_eq!(refused.owner.event_count(), count);
    assert_eq!(refused.owner.owner.audio.renderer.clock(), SampleTime::ZERO);
    assert_eq!(
        refused.owner.owner.control.minter.resolve(original),
        Resolution::Live
    );
    assert_eq!(refused.candidate.anchor.time(), requested);
    assert_eq!(
        refused
            .candidate
            .events
            .last()
            .map(|event| event.envelope().time()),
        Some(SampleTime::new(u64::MAX))
    );
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
    assert!(
        stamped
            .events
            .windows(2)
            .all(|pair| { pair[0].envelope().time() <= pair[1].envelope().time() })
    );
    let overflowing_index = stamped.events.len() - 1;
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
fn private_event_order_check_refuses_a_reversed_combined_list() {
    let prepared = bound(true);
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let stamped = prepared.stamp_suffix(suffix).expect("ordered stamp");
    let mut events = stamped.events.clone();
    let last = events.len() - 1;
    events.swap(0, last);
    assert_eq!(
        check_mixed_event_order(&events),
        Err(MixedStampPrepareError::EventOrder { event_index: 1 })
    );
}

#[test]
fn private_effective_timing_accepts_latest_event_at_timeline_end() {
    let prepared = bound_with_events(false, |note| {
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
                PlanPosition::new(73),
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
    let timing = stamped
        .effective_timing(SampleTime::new(last_boundary))
        .expect("latest event reaches the final frame exactly");
    assert_eq!(timing.shift(), FrameCount::new(128));
    let shifted = stamped
        .effective_events(timing.effective())
        .expect("checked view");
    assert_eq!(
        shifted
            .get(shifted.len() - 1)
            .expect("last read")
            .expect("last event")
            .envelope()
            .time(),
        SampleTime::new(u64::MAX)
    );
}

#[test]
fn private_effective_list_releases_old_compiled_notes_before_a_destination_on() {
    for compiled_first in [true, false] {
        let mut prepared = bound_with_events(compiled_first, |note| {
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
                    PlanPosition::new(256),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(72),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(384),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(448),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(72),
                    },
                ),
            ]
        });
        let note = prepared
            .owner
            .control
            .plan
            .resolve_note(ENVELOPE)
            .expect("note");
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(256))
            .expect("old note is open at destination");
        let suffix = prepared.prepare_suffix(history).expect("bound suffix");
        assert_eq!(suffix.omitted_release_count().get(), 1);
        let stamped = prepared.stamp_suffix(suffix).expect("private candidate");
        let old_on = prepared.events[0];
        let EventPayload::Note {
            identity: old_identity,
            edge: NoteEdge::On { .. },
        } = old_on.payload()
        else {
            panic!("initial compiled onset");
        };
        let epoch = prepared.epoch();
        let table = prepared.table_id();
        let plan = std::sync::Arc::clone(&prepared.owner.control.plan);
        let partition = std::sync::Arc::clone(&prepared.owner.control.partition);
        let anchor = prepared.owner.control.anchor;
        let mut live_only =
            PreparedRenderer::prepare(std::sync::Arc::clone(&plan), anchor, epoch, table)
                .expect("live renderer");
        let mut compiled_only =
            PreparedRenderer::prepare(std::sync::Arc::clone(&plan), anchor, epoch, table)
                .expect("compiled renderer");
        let mut released_only =
            PreparedRenderer::prepare(plan, anchor, epoch, table).expect("released reference");
        assert!(live_only.bind_mixed_partition(std::sync::Arc::clone(&partition)));
        assert!(compiled_only.bind_mixed_partition(std::sync::Arc::clone(&partition)));
        assert!(released_only.bind_mixed_partition(partition));
        let audio = &mut prepared.owner.audio;
        let live_identity = audio
            .minter
            .mint_keyed(note, key(48))
            .expect("live range credit");
        let live_on = TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TestOrigin::Simulated),
            EventPayload::Note {
                identity: live_identity,
                edge: NoteEdge::On {
                    slot: note,
                    key: key(48),
                    velocity: NoteVelocity::FULL,
                },
            },
        );
        for renderer in [
            &mut audio.renderer,
            &mut live_only,
            &mut compiled_only,
            &mut released_only,
        ] {
            let _ = render_quantum(renderer, &[]);
        }
        let _ = render_quantum(&mut audio.renderer, &[old_on, live_on]);
        let _ = render_quantum(&mut live_only, &[live_on]);
        let _ = render_quantum(&mut compiled_only, &[old_on]);
        let _ = render_quantum(&mut released_only, &[old_on]);
        for renderer in [
            &mut audio.renderer,
            &mut live_only,
            &mut compiled_only,
            &mut released_only,
        ] {
            let _ = render_quantum(renderer, &[]);
        }
        let effective = audio.renderer.clock();
        assert_eq!(effective, SampleTime::new(128));
        let shifted = stamped
            .effective_events(effective)
            .expect("all candidate events fit the effective boundary");
        assert_eq!(
            shifted.len(),
            stamped.event_count().as_usize().expect("event count")
        );
        let events = shifted
            .iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("checked events");
        assert_eq!(shifted.get(0).expect("first read"), events.first().copied());
        assert_eq!(shifted.get(events.len()).expect("end read"), None);
        assert_eq!(stamped.events[0].envelope().time(), SampleTime::new(64));
        let restoration_count = stamped
            .restoration_count()
            .as_usize()
            .expect("restoration count");
        assert!(events[..restoration_count].iter().all(|event| {
            matches!(event.payload(), EventPayload::ScopedRestore(_))
                && event.envelope().time() == effective
        }));
        let EventPayload::Note {
            identity: new_identity,
            edge: NoteEdge::On { key: new_key, .. },
        } = events[restoration_count].payload()
        else {
            panic!("destination onset follows restoration");
        };
        assert_eq!(new_key, key(72));
        assert_eq!(events[restoration_count].envelope().time(), effective);
        assert_ne!(new_identity, old_identity);
        assert_eq!(events.len(), restoration_count + 2);
        assert_eq!(
            events.last().map(|event| event.envelope().time()),
            Some(SampleTime::new(320))
        );
        assert!(matches!(
            events.last().map(TimedEvent::payload),
            Some(EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            }) if identity == new_identity
        ));
        let boundary_events: Vec<_> = events
            .iter()
            .copied()
            .filter(|event| event.envelope().time() == effective)
            .collect();
        assert_eq!(boundary_events.len(), restoration_count + 1);
        let old_anchor = audio.renderer.anchor_for_test();
        assert_ne!(
            old_anchor.time_of(stamped.suffix.history.position),
            Some(effective),
            "the seek must change the musical mapping"
        );
        assert_eq!(
            audio.adopt_compiled_boundary(SampleTime::new(64), stamped.suffix.history.position),
            Err(crate::render::MixedBoundaryReleaseError::ClockMismatch {
                clock: effective,
                offered: SampleTime::new(64),
            })
        );
        assert_eq!(audio.renderer.anchor_for_test(), old_anchor);
        let ended_storage = std::mem::take(&mut audio.compiled_ended);
        assert_eq!(
            audio.adopt_compiled_boundary(effective, stamped.suffix.history.position),
            Err(crate::render::MixedBoundaryReleaseError::EndedStorage)
        );
        assert_eq!(audio.renderer.anchor_for_test(), old_anchor);
        audio.compiled_ended = ended_storage;
        let mut adopted = None;
        let allocations = crate::render_allocation::count_allocs(|| {
            adopted = Some(
                audio
                    .adopt_compiled_boundary(effective, stamped.suffix.history.position)
                    .expect("compiled-only boundary adoption"),
            );
        });
        assert_eq!(allocations, 0);
        let adopted = adopted.expect("boundary result");
        assert_eq!(adopted.released().get(), 1);
        assert_eq!(adopted.retired_anchor(), old_anchor);
        assert_eq!(
            audio.renderer.anchor_for_test(),
            StreamAnchor::new(effective, stamped.suffix.history.position)
        );
        assert_eq!(
            audio.compiled_ended[0].map(|entry| entry.index),
            Some(old_identity.index())
        );
        let mut compiled_ended = [None; 2];
        assert_eq!(
            compiled_only
                .release_mixed_compiled_boundary(
                    audio.partition.compiled_producer(),
                    &mut compiled_ended,
                )
                .expect("reference compiled release")
                .get(),
            1
        );
        let mut released_ended = [None; 2];
        assert_eq!(
            released_only
                .release_mixed_compiled_boundary(
                    audio.partition.compiled_producer(),
                    &mut released_ended,
                )
                .expect("reference release without new onset")
                .get(),
            1
        );
        let actual = render_quantum(&mut audio.renderer, &boundary_events);
        let live = render_quantum(&mut live_only, &[]);
        let new = render_quantum(&mut compiled_only, &boundary_events);
        let released = render_quantum(&mut released_only, &boundary_events[..restoration_count]);
        assert!(live.iter().any(|sample| *sample != 0.0));
        assert!(new.iter().any(|sample| *sample != 0.0));
        assert!(released.iter().all(|sample| *sample == 0.0));
        for (index, ((actual, live), new)) in actual.iter().zip(&live).zip(&new).enumerate() {
            assert!(
                (actual - (live + new)).abs() < 0.00001,
                "boundary frame {index}: mixed {actual}, live {live}, new {new}"
            );
        }
        let actual_next = render_quantum(&mut audio.renderer, &[]);
        let live_next = render_quantum(&mut live_only, &[]);
        let new_next = render_quantum(&mut compiled_only, &[]);
        let released_next = render_quantum(&mut released_only, &[]);
        assert!(released_next.iter().all(|sample| *sample == 0.0));
        assert!(new_next.iter().any(|sample| *sample != 0.0));
        for ((actual, live), new) in actual_next.iter().zip(&live_next).zip(&new_next) {
            assert!((actual - (live + new)).abs() < 0.00001);
        }
    }
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
