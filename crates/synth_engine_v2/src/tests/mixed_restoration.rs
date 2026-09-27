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
use crate::node::kernels::{NodeState, Playback};
use crate::plan::{ParameterInstanceSpan, ParameterSlot};
use crate::profile::HostProfile;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, ParameterValue, SampleRate, Seconds, VoiceCount,
};
use crate::render::slot::SlotState;
use crate::sample::{
    PlayDirection, PlayMode, PlaybackRegion, PreparedSample, SampleFrame, SampleMap, SampleMapRef,
    SampleRef, SampleZone,
};
use crate::schedule::{AdmittedCompiledStream, CompiledPayload, PlanEvent};
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

#[test]
fn mixed_boundary_storage_preflight_reads_actual_registry_and_prepared_buffers() {
    for compiled_first in [true, false] {
        let (binding, _) = binding(compiled_first);
        let table = IdentityTable::from_admitted_ranges(binding.plan().note_producer_ranges())
            .expect("identity ranges");
        let epoch = issue_epoch().expect("epoch");
        let mut renderer =
            PreparedRenderer::prepare(Arc::clone(binding.plan_arc()), ANCHOR, epoch, table.id())
                .expect("mixed renderer");
        assert!(renderer.bind_mixed_partition(Arc::clone(binding.partition_arc())));
        let compiled = binding.instance_partition().compiled_producer();
        let span = binding.instance_partition().spans().0;
        let ended = span.indices().len();
        let plan = binding.plan();
        let gate_needed = ended * plan.max_writes_per_note().get() as usize;
        let event_width = super::release_group_writes(plan.max_writes_per_note())
            .fanned_out(plan.sample_positioned_fan_out())
            .widest(plan.steal_expansion())
            .get() as usize;
        let timed_needed = plan
            .max_events_per_quantum()
            .as_usize()
            .expect("event count")
            * event_width
            + gate_needed
            + plan.modulated_sample_positioned_rows() as usize;
        assert!(gate_needed > 0);
        assert!(timed_needed > gate_needed);
        assert!(renderer.adoption_gates.len() >= gate_needed);
        assert!(renderer.adoption_gate_slots.len() >= gate_needed);
        assert!(renderer.timed_controls.len() >= timed_needed);
        assert_eq!(renderer.live_notes.producer_range(compiled), Some(span));
        assert_eq!(renderer.check_mixed_boundary_storage(ended), Ok(()));
        renderer.adoption_gate_len = 1;
        assert_eq!(
            renderer.check_mixed_boundary_storage(ended),
            Err(super::MixedBoundaryStorageError::PendingBoundary)
        );
        renderer.adoption_gate_len = 0;
        assert_eq!(
            renderer.check_mixed_boundary_storage(ended - 1),
            Err(super::MixedBoundaryStorageError::Ended {
                needed: ended,
                available: ended - 1,
            })
        );

        let gates = renderer.adoption_gates.clone();
        renderer.adoption_gates.truncate(gate_needed);
        assert_eq!(renderer.check_mixed_boundary_storage(ended), Ok(()));
        renderer.adoption_gates.truncate(gate_needed - 1);
        assert_eq!(
            renderer.check_mixed_boundary_storage(ended),
            Err(super::MixedBoundaryStorageError::Gate {
                needed: gate_needed,
                available: gate_needed - 1,
            })
        );
        renderer.adoption_gates = gates;
        assert_eq!(renderer.check_mixed_boundary_storage(ended), Ok(()));

        let slots = renderer.adoption_gate_slots.clone();
        renderer.adoption_gate_slots.truncate(gate_needed);
        assert_eq!(renderer.check_mixed_boundary_storage(ended), Ok(()));
        renderer.adoption_gate_slots.truncate(gate_needed - 1);
        assert_eq!(
            renderer.check_mixed_boundary_storage(ended),
            Err(super::MixedBoundaryStorageError::Gate {
                needed: gate_needed,
                available: gate_needed - 1,
            })
        );
        renderer.adoption_gate_slots = slots;
        assert_eq!(renderer.check_mixed_boundary_storage(ended), Ok(()));

        let controls = renderer.timed_controls.clone();
        renderer.timed_controls.truncate(timed_needed);
        assert_eq!(renderer.check_mixed_boundary_storage(ended), Ok(()));
        renderer.timed_controls.truncate(timed_needed - 1);
        assert_eq!(
            renderer.check_mixed_boundary_storage(ended),
            Err(super::MixedBoundaryStorageError::Timed {
                needed: timed_needed,
                available: timed_needed - 1,
            })
        );
        renderer.timed_controls = controls;
        assert_eq!(renderer.check_mixed_boundary_storage(ended), Ok(()));

        renderer
            .live_notes
            .shorten_producer_range_for_test(compiled);
        assert_eq!(
            renderer.check_mixed_boundary_storage(ended),
            Err(super::MixedBoundaryStorageError::Partition)
        );
    }
}

fn value(raw: f32) -> ParameterValue {
    ParameterValue::new(raw).expect("finite parameter")
}

fn producers(compiled_first: bool) -> Vec<NoteProducerDeclaration> {
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
    if compiled_first {
        vec![compiled, live]
    } else {
        vec![live, compiled]
    }
}

fn binding(compiled_first: bool) -> (MixedTargetAdmission, crate::plan::NoteSlot) {
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
            note_producers: producers(compiled_first),
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
    // Bind the same compiled target the rendered compiled-provenance onset names.
    let stream = AdmittedCompiledStream::admit(
        &plan,
        &[PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::NoteOn {
                slot,
                key: KeyIdentity::LOWEST,
                velocity: NoteVelocity::FULL,
            },
        )],
    )
    .expect("compiled note stream");
    (
        MixedTargetAdmission::admit(plan, stream, slot).expect("disjoint mixed targets"),
        slot,
    )
}

fn sampler_binding(compiled_first: bool) -> (MixedTargetAdmission, crate::plan::NoteSlot) {
    let rate = SampleRate::new(48_000.0).expect("sample rate");
    let sample = PreparedSample::prepare(vec![0.25; 4096], ChannelLayout::Mono, rate)
        .expect("finite mono sample");
    let region =
        PlaybackRegion::new(SampleFrame::new(0), SampleFrame::new(4096)).expect("nonempty region");
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
            note_producers: producers(compiled_first),
            held_notes: HeldNoteCount::measured(4),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("sampler mixed graph");
    let profile =
        HostProfile::harness(rate, FrameCount::new(512), ChannelLayout::Mono).expect("profile");
    let plan = compile(&ir, &RenderConfig::new(profile))
        .into_plan()
        .expect("admitted sampler plan");
    let slot = plan.resolve_note(ENVELOPE).expect("playable envelope");
    // Bind the same compiled target the rendered compiled-provenance onset names.
    let stream = AdmittedCompiledStream::admit(
        &plan,
        &[PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::NoteOn {
                slot,
                key: KeyIdentity::LOWEST,
                velocity: NoteVelocity::FULL,
            },
        )],
    )
    .expect("compiled sampler note stream");
    (
        MixedTargetAdmission::admit(plan, stream, slot).expect("disjoint sampler targets"),
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

fn render_quantum(renderer: &mut PreparedRenderer, events: &[TimedEvent]) -> Vec<f32> {
    let mut samples = vec![0.0_f32; Q];
    let block = AudioBlockMut::new(&mut samples, Q, ChannelLayout::Mono).expect("quantum block");
    renderer
        .render(block, TimedEvents::new(events))
        .expect("bounded quantum render");
    samples
}

#[test]
fn mixed_boundary_release_ends_only_sounding_compiled_notes() {
    for compiled_first in [true, false] {
        for sampler in [false, true] {
            let (binding, note) = if sampler {
                sampler_binding(compiled_first)
            } else {
                binding(compiled_first)
            };
            let mut table =
                IdentityTable::from_admitted_ranges(binding.plan().note_producer_ranges())
                    .expect("identity ranges");
            let compiled = table
                .mint(binding.instance_partition().compiled_producer(), note)
                .expect("compiled identity");
            let live = table
                .mint(binding.live_producer(), note)
                .expect("live identity");
            let epoch = issue_epoch().expect("epoch");
            let mut mixed = PreparedRenderer::prepare(
                Arc::clone(binding.plan_arc()),
                ANCHOR,
                epoch,
                table.id(),
            )
            .expect("mixed renderer");
            let mut live_only = PreparedRenderer::prepare(
                Arc::clone(binding.plan_arc()),
                ANCHOR,
                epoch,
                table.id(),
            )
            .expect("live reference");
            assert!(mixed.bind_mixed_partition(Arc::clone(binding.partition_arc())));
            assert!(live_only.bind_mixed_partition(Arc::clone(binding.partition_arc())));
            let onset = |identity, source| {
                TimedEvent::new(
                    EventEnvelope::new(epoch, SampleTime::ZERO, source),
                    EventPayload::Note {
                        identity,
                        edge: NoteEdge::On {
                            slot: note,
                            key: KeyIdentity::LOWEST,
                            velocity: NoteVelocity::FULL,
                        },
                    },
                )
            };
            let live_on = onset(live, TimeSource::Simulated);
            let compiled_on = onset(compiled, TimeSource::Compiled);
            let _ = render_quantum(&mut mixed, &[]);
            let _ = render_quantum(&mut live_only, &[]);
            let _ = render_quantum(&mut mixed, &[compiled_on, live_on]);
            let _ = render_quantum(&mut live_only, &[live_on]);
            assert_eq!(mixed.live_notes.note_of(compiled), Some(note));
            assert_eq!(mixed.live_notes.note_of(live), Some(note));
            let gate = binding.plan().note_targets()[note.index()].parameter;
            let gate_row = binding
                .plan()
                .parameter_row_for_identity(gate, compiled.index())
                .expect("compiled gate row");
            mixed.parameter_slots[gate_row.index()].smooth_over(2 * QUANTUM_FRAMES);
            for magnitude in binding.plan().note_magnitudes_of(note) {
                if magnitude.magnitude == crate::node::NoteMagnitude::Trigger {
                    let row = binding
                        .plan()
                        .parameter_row_for_identity(magnitude.parameter, compiled.index())
                        .expect("compiled trigger row");
                    mixed.parameter_slots[row.index()].smooth_over(2 * QUANTUM_FRAMES);
                }
            }
            let invalid_note = crate::plan::NoteSlot::new(binding.plan().id(), usize::MAX);
            mixed
                .live_notes
                .admit(compiled, invalid_note, KeyIdentity::LOWEST);
            let mut ended = [None; 2];
            assert_eq!(
                mixed.release_mixed_compiled_boundary(
                    binding.instance_partition().compiled_producer(),
                    &mut ended,
                ),
                Err(super::MixedBoundaryReleaseError::UnboundTarget { note: invalid_note })
            );
            assert_eq!(mixed.live_notes.note_of(compiled), Some(invalid_note));
            assert_eq!(mixed.adoption_gate_len, 0);
            mixed.live_notes.admit(compiled, note, KeyIdentity::LOWEST);
            let before_live = states(&mixed, binding.instance_partition().live_rows());
            let mut too_short = [None; 1];
            assert_eq!(
                mixed.release_mixed_compiled_boundary(
                    binding.instance_partition().compiled_producer(),
                    &mut too_short,
                ),
                Err(super::MixedBoundaryReleaseError::EndedStorage)
            );
            assert_eq!(
                mixed.release_mixed_compiled_boundary(binding.live_producer(), &mut ended),
                Err(super::MixedBoundaryReleaseError::WrongProducer {
                    expected: binding.instance_partition().compiled_producer(),
                    offered: binding.live_producer(),
                })
            );
            assert_eq!(mixed.live_notes.note_of(compiled), Some(note));
            assert_eq!(mixed.live_notes.note_of(live), Some(note));
            assert_eq!(mixed.adoption_gate_len, 0);
            let saved_gates = std::mem::take(&mut mixed.adoption_gates);
            assert_eq!(
                mixed.release_mixed_compiled_boundary(
                    binding.instance_partition().compiled_producer(),
                    &mut ended,
                ),
                Err(super::MixedBoundaryReleaseError::GateStorage {
                    needed: 1 + usize::from(sampler),
                    available: 0,
                })
            );
            assert_eq!(mixed.live_notes.note_of(compiled), Some(note));
            assert_eq!(mixed.live_notes.note_of(live), Some(note));
            mixed.adoption_gates = saved_gates;
            let mut released = HeldNoteCount::NONE;
            let allocations = crate::render_allocation::count_allocs(|| {
                released = mixed
                    .release_mixed_compiled_boundary(
                        binding.instance_partition().compiled_producer(),
                        &mut ended,
                    )
                    .expect("bound compiled release");
            });
            assert_eq!(allocations, 0);
            assert_eq!(released.get(), 1);
            assert_eq!(ended[0].map(|entry| entry.index), Some(compiled.index()));
            assert_eq!(mixed.live_notes.note_of(compiled), None);
            assert_eq!(mixed.live_notes.note_of(live), Some(note));
            assert_eq!(
                states(&mixed, binding.instance_partition().live_rows()),
                before_live
            );
            let trigger_count = binding
                .plan()
                .note_magnitudes_of(note)
                .iter()
                .filter(|magnitude| magnitude.magnitude == crate::node::NoteMagnitude::Trigger)
                .count();
            assert_eq!(mixed.adoption_gate_len, 1 + trigger_count);
            assert_eq!(sampler, trigger_count > 0);
            let mut released_rows = Vec::new();
            for index in 0..mixed.adoption_gate_len {
                let row = crate::plan::ParameterRow::new(
                    binding.plan().id(),
                    mixed.adoption_gate_slots[index],
                );
                released_rows.push(row);
                assert!(
                    binding
                        .instance_partition()
                        .compiled_rows()
                        .binary_search(&row)
                        .is_ok()
                );
                assert_eq!(mixed.adoption_gates[index].value, ParameterValue::ZERO);
            }
            assert_eq!(
                mixed.release_mixed_compiled_boundary(
                    binding.instance_partition().compiled_producer(),
                    &mut ended,
                ),
                Err(super::MixedBoundaryReleaseError::PendingBoundary)
            );
            let _ = render_quantum(&mut mixed, &[]);
            let _ = render_quantum(&mut live_only, &[]);
            assert_eq!(mixed.adoption_gate_len, 0);
            for row in released_rows {
                assert_eq!(
                    mixed.parameter_slots[row.index()].current(),
                    ParameterValue::ZERO
                );
            }
            assert_eq!(
                states(&mixed, binding.instance_partition().live_rows()),
                states(&live_only, binding.instance_partition().live_rows())
            );
            let actual = render_quantum(&mut mixed, &[]);
            let expected = render_quantum(&mut live_only, &[]);
            assert!(actual.iter().any(|sample| *sample != 0.0));
            assert_eq!(actual, expected, "compiled release changed live output");
        }
    }
}

#[test]
fn mixed_boundary_release_refuses_missing_compiled_gate_or_trigger_row() {
    for compiled_first in [true, false] {
        for sampler in [false, true] {
            let (mut binding, note) = if sampler {
                sampler_binding(compiled_first)
            } else {
                binding(compiled_first)
            };
            let mut table =
                IdentityTable::from_admitted_ranges(binding.plan().note_producer_ranges())
                    .expect("identity ranges");
            let compiled = table
                .mint(binding.instance_partition().compiled_producer(), note)
                .expect("compiled identity");
            let parameter = if sampler {
                binding
                    .plan()
                    .note_magnitudes_of(note)
                    .iter()
                    .find(|magnitude| magnitude.magnitude == crate::node::NoteMagnitude::Trigger)
                    .expect("sampler trigger")
                    .parameter
            } else {
                binding.plan().note_targets()[note.index()].parameter
            };
            let row = binding
                .plan()
                .parameter_row_for_identity(parameter, compiled.index())
                .expect("compiled row");
            assert!(binding.omit_compiled_row_for_test(row));
            let epoch = issue_epoch().expect("epoch");
            let mut renderer = PreparedRenderer::prepare(
                Arc::clone(binding.plan_arc()),
                ANCHOR,
                epoch,
                table.id(),
            )
            .expect("renderer");
            assert!(renderer.bind_mixed_partition(Arc::clone(binding.partition_arc())));
            let _ = render_quantum(&mut renderer, &[]);
            let onset = TimedEvent::new(
                EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
                EventPayload::Note {
                    identity: compiled,
                    edge: NoteEdge::On {
                        slot: note,
                        key: KeyIdentity::LOWEST,
                        velocity: NoteVelocity::FULL,
                    },
                },
            );
            let _ = render_quantum(&mut renderer, &[onset]);
            assert_eq!(renderer.live_notes.note_of(compiled), Some(note));
            let mut ended = [None; 2];
            assert_eq!(
                renderer.release_mixed_compiled_boundary(
                    binding.instance_partition().compiled_producer(),
                    &mut ended,
                ),
                Err(super::MixedBoundaryReleaseError::UnboundTarget { note })
            );
            assert_eq!(renderer.live_notes.note_of(compiled), Some(note));
            assert_eq!(renderer.adoption_gate_len, 0);
        }
    }
}

#[test]
fn mixed_boundary_release_and_restoration_spend_frequency_seed() {
    for compiled_first in [true, false] {
        let (binding, note) = binding(compiled_first);
        let mut table = IdentityTable::from_admitted_ranges(binding.plan().note_producer_ranges())
            .expect("identity ranges");
        let compiled = table
            .mint(binding.instance_partition().compiled_producer(), note)
            .expect("compiled identity");
        let live = table
            .mint(binding.live_producer(), note)
            .expect("live identity");
        let epoch = issue_epoch().expect("epoch");
        let mut mixed =
            PreparedRenderer::prepare(Arc::clone(binding.plan_arc()), ANCHOR, epoch, table.id())
                .expect("mixed renderer");
        let mut live_only =
            PreparedRenderer::prepare(Arc::clone(binding.plan_arc()), ANCHOR, epoch, table.id())
                .expect("live reference");
        assert!(mixed.bind_mixed_partition(Arc::clone(binding.partition_arc())));
        assert!(live_only.bind_mixed_partition(Arc::clone(binding.partition_arc())));
        let frequency = binding
            .plan()
            .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
            .expect("frequency");
        let compiled_frequency = binding
            .plan()
            .parameter_row_for_identity(frequency, compiled.index())
            .expect("compiled frequency row");
        mixed.parameter_slots[compiled_frequency.index()].smooth_over(2 * QUANTUM_FRAMES);
        let _ = render_quantum(&mut mixed, &[]);
        let _ = render_quantum(&mut live_only, &[]);
        assert_eq!(mixed.clock, live_only.clock);
        let onset_time = mixed.clock;
        let onset = |identity, source| {
            TimedEvent::new(
                EventEnvelope::new(epoch, onset_time, source),
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On {
                        slot: note,
                        key: KeyIdentity::LOWEST,
                        velocity: NoteVelocity::FULL,
                    },
                },
            )
        };
        let _ = render_quantum(
            &mut mixed,
            &[
                onset(compiled, TimeSource::Compiled),
                onset(live, TimeSource::Simulated),
            ],
        );
        let _ = render_quantum(&mut live_only, &[onset(live, TimeSource::Simulated)]);
        assert_eq!(mixed.clock, live_only.clock);
        let boundary = mixed.clock;
        assert_eq!(boundary.quantum_offset(), crate::time::QuantumOffset::ZERO);
        let mut restoration = Vec::new();
        for group in binding.instance_partition().restoration_groups() {
            let target = binding.plan().parameter_targets()[group.parameter().index()];
            let scoped = if target.controller {
                ScopedParameterRestore::controller_for(*group, target.base, None)
            } else {
                ScopedParameterRestore::override_for(*group, target.base)
            };
            restoration.push(TimedEvent::new(
                EventEnvelope::new(epoch, boundary, TimeSource::Compiled),
                EventPayload::ScopedRestore(scoped),
            ));
        }
        let live_before = states(&mixed, binding.instance_partition().live_rows());
        let mut ended = [None; 2];
        assert_eq!(
            mixed
                .release_mixed_compiled_boundary(
                    binding.instance_partition().compiled_producer(),
                    &mut ended,
                )
                .expect("bound release")
                .get(),
            1
        );
        assert_eq!(
            states(&mixed, binding.instance_partition().live_rows()),
            live_before
        );
        let actual = render_quantum(&mut mixed, &restoration);
        let expected = render_quantum(&mut live_only, &[]);
        assert!(actual.iter().any(|sample| *sample != 0.0));
        assert_eq!(actual, expected, "boundary changed live output");
        assert_eq!(
            states(&mixed, binding.instance_partition().live_rows()),
            states(&live_only, binding.instance_partition().live_rows())
        );
        assert_eq!(
            mixed.parameter_slots[compiled_frequency.index()].current(),
            binding.plan().parameter_targets()[frequency.index()].base,
            "restoration must step at the boundary"
        );
        let restored = mixed.parameter_slots[compiled_frequency.index()].current();
        let _ = mixed.parameter_slots[compiled_frequency.index()].write_override(value(440.0));
        assert_eq!(
            mixed.parameter_slots[compiled_frequency.index()].current(),
            restored,
            "frequency restoration must spend its seed before the next ordinary ramp"
        );
        let mut next = [0.0_f32; 1];
        mixed.parameter_slots[compiled_frequency.index()].advance(&mut next);
        assert!(next[0] > restored.as_f32() && next[0] < 440.0);
    }
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
        assert!(!binding.instance_partition().shared_sum_nodes().is_empty());
        assert!(
            binding.instance_partition().shared_sum_rows().is_empty(),
            "inserted voice-sum steps currently declare no controls"
        );
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
fn scoped_trigger_restoration_falls_without_retriggering_or_moving_live_state() {
    for compiled_first in [true, false] {
        let (binding, note) = sampler_binding(compiled_first);
        let trigger = binding
            .plan()
            .resolve_parameter(SOURCE, parameters::SAMPLER_TRIGGER)
            .expect("sampler trigger");
        let span = group(&binding, trigger).instances();
        let live_instance = binding.instance_partition().spans().1.indices().start as usize;
        let live_node = binding.plan().parameter_targets()[trigger.index() + live_instance]
            .node
            .index();
        let (mut reference, mut mixed, identity, epoch) = renderers(&binding, note);
        assert!(reference.bind_mixed_partition(Arc::clone(binding.partition_arc())));
        let live_on = TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Simulated),
            EventPayload::Note {
                identity,
                edge: NoteEdge::On {
                    slot: note,
                    key: KeyIdentity::LOWEST,
                    velocity: NoteVelocity::FULL,
                },
            },
        );
        let compiled_on = TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
            EventPayload::ScopedRestore(ScopedParameterRestore::override_for(
                group(&binding, trigger),
                ParameterValue::ONE,
            )),
        );
        let pitch = binding
            .plan()
            .resolve_parameter(SOURCE, parameters::SAMPLER_PITCH)
            .expect("sampler pitch");
        let root = crate::tuning::PreparedTuning::equal_temperament()
            .expect("tuning")
            .frequency_of(KeyIdentity::LOWEST);
        let compiled_pitch = TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
            EventPayload::ScopedRestore(ScopedParameterRestore::override_for(
                group(&binding, pitch),
                ParameterValue::from_frequency(root),
            )),
        );
        let start = [live_on, compiled_on, compiled_pitch];
        assert!(
            render(&mut mixed, &start)
                .iter()
                .any(|sample| *sample != 0.0)
        );
        let _ = render(&mut reference, &start);
        let before_positions: Vec<_> = (span.first()..span.first() + span.count())
            .map(|instance| {
                let row = trigger.index() + instance as usize;
                let node = binding.plan().parameter_targets()[row].node.index();
                let position = match mixed.node_states()[node] {
                    NodeState::Sampler {
                        position,
                        playback: Playback::Playing,
                        held: true,
                        ..
                    } => position,
                    state => panic!("compiled trigger did not start: {state:?}"),
                };
                (row, node, position)
            })
            .collect();
        mixed.seed_for_adoption();
        let restore = TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::new((4 * Q) as u64), TimeSource::Compiled),
            EventPayload::ScopedRestore(ScopedParameterRestore::override_for(
                group(&binding, trigger),
                ParameterValue::ZERO,
            )),
        );
        let _ = render(&mut mixed, &[restore]);
        let _ = render(&mut reference, &[]);
        assert_eq!(mixed.diagnostics().foreign_slot_events(), 0);
        for (row, node, before_position) in before_positions {
            match mixed.node_states()[node] {
                NodeState::Sampler {
                    position,
                    playback: Playback::Fading,
                    held: false,
                    ..
                } => assert!(
                    position > before_position,
                    "a rising edge would restart compiled instance {node}"
                ),
                state => panic!("compiled trigger did not fall: {state:?}"),
            }
            assert_eq!(mixed.parameter_slots[row].automated(), ParameterValue::ZERO);
        }
        assert_eq!(
            states(&mixed, binding.instance_partition().live_rows()),
            states(&reference, binding.instance_partition().live_rows())
        );
        for node in binding.instance_partition().live_nodes() {
            assert_eq!(
                mixed.node_states()[node.index()],
                reference.node_states()[node.index()]
            );
        }
        assert!(matches!(
            mixed.node_states()[live_node],
            NodeState::Sampler {
                playback: Playback::Playing,
                held: true,
                ..
            }
        ));
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
fn a_scoped_event_with_the_wrong_payload_kind_is_refused_before_writing() {
    let (binding, note) = binding(false);
    let gate = binding.plan().note_targets()[note.index()].parameter;
    let controller = binding
        .plan()
        .resolve_parameter(CONTROLLER, parameters::SOURCE_VALUE)
        .expect("controller source value");
    let (mut plain, mut mixed, _, epoch) = renderers(&binding, note);
    let before_compiled = states(&mixed, binding.instance_partition().compiled_rows());
    let bad_gate = TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::ScopedRestore(ScopedParameterRestore::controller_for(
            group(&binding, gate),
            ParameterValue::ONE,
            None,
        )),
    );
    let bad_controller = TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::ScopedRestore(ScopedParameterRestore::override_for(
            group(&binding, controller),
            value(0.75),
        )),
    );
    assert_eq!(
        render(&mut mixed, &[bad_gate, bad_controller]),
        render(&mut plain, &[])
    );
    assert_eq!(mixed.diagnostics().foreign_slot_events(), 2);
    assert_eq!(
        states(&mixed, binding.instance_partition().compiled_rows()),
        before_compiled
    );
    assert_eq!(
        states(&mixed, binding.instance_partition().live_rows()),
        states(&plain, binding.instance_partition().live_rows())
    );
    assert_eq!(
        states(&mixed, binding.instance_partition().global_rows()),
        states(&plain, binding.instance_partition().global_rows())
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
