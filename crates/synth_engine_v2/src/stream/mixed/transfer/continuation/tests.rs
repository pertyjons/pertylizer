use super::super::tests::{Armed, Ring, callback, note_on};
use super::*;
use crate::{
    compile::{RenderConfig, compile},
    ir::{
        ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations,
        PortId, SignalDomain,
    },
    profile::HostProfile,
    quantities::{
        Amplitude, ChannelLayout, DelayFeedback, DelayTime, EventCount, Frequency, HeldNoteCount,
        KeyIdentity, NormalizedLevel, NoteVelocity, SampleRate, Seconds,
    },
    render_allocation::count_allocs,
    schedule::{AdmittedCompiledStream, CompiledPayload, PlanEvent},
    stream::{MixedJoinedPrepared, MixedJoinedStream, MixedSourceKeyUp},
    time::{FrameCount, PlanPosition, StreamAnchor},
};

const SOURCE: NodeId = NodeId::new(10);
const GATE: NodeId = NodeId::new(20);
const AMP: NodeId = NodeId::new(30);
const DELAY: NodeId = NodeId::new(40);
const TRIM: NodeId = NodeId::new(50);
const FOLD: NodeId = NodeId::new(55);
const OUTPUT: NodeId = NodeId::new(60);
pub(in crate::stream::mixed::transfer) const BYTES: PreparedBytes =
    PreparedBytes::measured(2 * 1024 * 1024);
pub(in crate::stream::mixed::transfer) const RECORDS: RecordCount = RecordCount::measured(1024);

fn profile(hz: f32) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(hz).unwrap(),
        FrameCount::new(256),
        ChannelLayout::Mono,
    )
    .unwrap()
}
pub(in crate::stream::mixed::transfer) fn targets(
    profile: HostProfile,
    level: f32,
) -> MixedTargetAdmission {
    targets_with_programs(profile, level, Vec::new(), Vec::new())
}
fn targets_with_programs(
    profile: HostProfile,
    level: f32,
    runtime_parameters: Vec<crate::ir::RuntimeParameterProgram>,
    internal_producers: Vec<crate::ir::InternalProducerDeclaration>,
) -> MixedTargetAdmission {
    let producer_load = !runtime_parameters.is_empty() || !internal_producers.is_empty();
    let has_internal = !internal_producers.is_empty();
    let voice = ExecutionScope::Voice;
    let global = ExecutionScope::Global;
    let graph = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(317.0).unwrap(),
                amplitude: Amplitude::new(0.3).unwrap(),
            },
            voice,
        )
        .node(
            GATE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            voice,
        )
        .node(AMP, IrNodeKind::Amplifier, voice)
        .node(
            DELAY,
            IrNodeKind::Delay {
                time_left: DelayTime::new(0.001).unwrap(),
                time_right: DelayTime::new(0.002).unwrap(),
                feedback: DelayFeedback::new(0.3).unwrap(),
                mix: NormalizedLevel::new(0.5).unwrap(),
                tone: NormalizedLevel::FULL,
            },
            if internal_producers.is_empty() {
                voice
            } else {
                global
            },
        )
        .node(
            TRIM,
            IrNodeKind::Trim {
                level: Amplitude::new(level).unwrap(),
            },
            global,
        )
        .node(
            NodeId::new(51),
            IrNodeKind::Trim {
                level: Amplitude::UNITY,
            },
            global,
        )
        .node(
            NodeId::new(52),
            IrNodeKind::Trim {
                level: Amplitude::UNITY,
            },
            global,
        )
        .node(FOLD, IrNodeKind::StereoToMono, global)
        .node(OUTPUT, IrNodeKind::Output, global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMP, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (GATE, PortId::FIRST),
            (AMP, crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMP, PortId::FIRST),
            (DELAY, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (DELAY, PortId::FIRST),
            (TRIM, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (TRIM, PortId::FIRST),
            (FOLD, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FOLD, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(
            voice,
            crate::tuning::PreparedTuning::equal_temperament().unwrap(),
        )
        .declaring(PlanDeclarations {
            runtime_parameters,
            internal_producers,
            note_producers: vec![
                NoteProducerDeclaration {
                    compiled: true,
                    simultaneous_notes: HeldNoteCount::measured(2),
                    simultaneous_holds: EventCount::NONE,
                },
                NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: HeldNoteCount::measured(2),
                    simultaneous_holds: EventCount::measured(2),
                },
            ],
            held_notes: HeldNoteCount::measured(4),
            ..PlanDeclarations::default()
        })
        .build()
        .unwrap();
    let outcome = compile(&graph, &RenderConfig::new(profile));
    let clear_warning = outcome.warnings().iter().find_map(|warning| match warning {
        crate::diagnostics::CompileWarning::InternalClearCostUnqualified {
            samples_per_quantum,
        } => Some(*samples_per_quantum),
        _ => None,
    });
    let plan = outcome.into_plan().unwrap();
    assert_eq!(
        clear_warning,
        has_internal.then_some(plan.internal_clear_sample_writes_per_quantum())
    );
    let slot = plan.resolve_note(GATE).unwrap();
    let events = [
        PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::NoteOn {
                slot,
                key: KeyIdentity::new(60).unwrap(),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(if producer_load { 1 } else { 8192 }),
            CompiledPayload::NoteOn {
                slot,
                key: KeyIdentity::new(64).unwrap(),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(if producer_load { 128 } else { 16384 }),
            CompiledPayload::NoteOff {
                slot,
                key: KeyIdentity::new(60).unwrap(),
            },
        ),
        PlanEvent::new(
            PlanPosition::new(if producer_load { 129 } else { 16448 }),
            CompiledPayload::NoteOff {
                slot,
                key: KeyIdentity::new(64).unwrap(),
            },
        ),
    ];
    let stream = AdmittedCompiledStream::admit(&plan, &events).unwrap();
    MixedTargetAdmission::admit(plan, stream, slot).unwrap()
}
pub(in crate::stream::mixed::transfer) fn prepared(
    profile: HostProfile,
) -> (MixedJoinedPrepared, crate::stream::MixedStampedCandidate) {
    let prepared = MixedJoinedStream::open(
        targets(profile, 0.8),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .unwrap()
    .prepare_initial()
    .unwrap();
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::ZERO)
        .unwrap();
    let suffix = prepared.prepare_suffix(history).unwrap();
    let candidate = prepared.stamp_suffix(suffix).unwrap();
    (prepared, candidate)
}

fn arm(profile: HostProfile) -> Armed {
    let (prepared, candidate) = prepared(profile);
    let capacity = MixedJoinedPrepared::transfer_capacity(&profile).get();
    prepared
        .arm_transfer_rehearsal(
            candidate,
            &profile,
            Ring::new(capacity),
            Ring::new(capacity),
        )
        .unwrap()
}
fn advance(armed: &mut Armed, reference: &mut Armed, frames: usize) {
    let mut samples = vec![0.0; frames];
    let mut expected = vec![0.0; frames];
    assert_eq!(
        count_allocs(|| callback(&mut armed.audio, &mut armed.audio_port, &mut samples).unwrap()),
        0
    );
    callback(
        &mut reference.audio,
        &mut reference.audio_port,
        &mut expected,
    )
    .unwrap();
    assert_eq!(samples, expected);
    while armed
        .control
        .collect(&mut armed.control_port)
        .unwrap()
        .is_some()
    {}
    while reference
        .control
        .collect(&mut reference.control_port)
        .unwrap()
        .is_some()
    {}
}

#[test]
fn mixed_fixed_continuation_keeps_handles_warm_history_and_future_ingress_at_four_rates() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(hz);
        let mut armed = arm(profile);
        let mut reference = arm(profile);
        for _ in 0..32 {
            advance(&mut armed, &mut reference, 64);
        }
        let table = armed.audio.inner.audio.renderer.table_id();
        let epoch = armed.audio.inner.audio.renderer.epoch();
        let now = armed.audio.inner.audio.renderer.clock().as_u64();
        let (_, handle) = armed
            .control
            .submit_note_on(&mut armed.control_port, note_on(now, 67))
            .unwrap();
        let (_, reference_handle) = reference
            .control
            .submit_note_on(&mut reference.control_port, note_on(now, 67))
            .unwrap();
        advance(&mut armed, &mut reference, 64);
        let future_at = armed.audio.inner.audio.renderer.clock().as_u64() + 160;
        let (_, future_handle) = armed
            .control
            .submit_note_on(&mut armed.control_port, note_on(future_at, 71))
            .unwrap();
        let (_, reference_future_handle) = reference
            .control
            .submit_note_on(&mut reference.control_port, note_on(future_at, 71))
            .unwrap();
        for _ in 0..4 {
            let at = SampleTime::new(armed.audio.inner.audio.renderer.clock().as_u64() + 128);
            let packet = armed
                .control
                .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
                .unwrap();
            let new_plan = packet.plan_id();
            assert!(
                armed
                    .control
                    .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
                    .is_err()
            );
            armed.audio.offer_continuation(packet).unwrap();
            for frames in [1, 63, 7, 121, 64, 128, 3, 253] {
                advance(&mut armed, &mut reference, frames);
            }
            let packet = armed.audio.take_retired_continuation().unwrap();
            assert_eq!(
                packet.outcome(),
                Some(MixedContinuationOutcome::InstalledRendered {
                    plan: new_plan,
                    boundary: at
                })
            );
            let retired = armed.control.collect_continuation(packet).unwrap();
            assert!(matches!(
                retired.outcome(),
                Some(MixedContinuationOutcome::InstalledRendered { .. })
            ));
            drop(retired);
            assert_eq!(armed.audio.inner.audio.renderer.table_id(), table);
            assert_eq!(armed.audio.inner.audio.renderer.epoch(), epoch);
            assert_eq!(armed.control.inner.control.plan.id(), new_plan);
            assert!(Arc::ptr_eq(
                &armed.control.inner.control.partition,
                &armed.audio.inner.audio.partition
            ));
        }
        let now = armed.audio.inner.audio.renderer.clock().as_u64();
        let _up = armed
            .control
            .submit_key_up(
                &mut armed.control_port,
                MixedSourceKeyUp {
                    at: SampleTime::new(now),
                    handle,
                },
            )
            .unwrap();
        let _reference_up = reference
            .control
            .submit_key_up(
                &mut reference.control_port,
                MixedSourceKeyUp {
                    at: SampleTime::new(now),
                    handle: reference_handle,
                },
            )
            .unwrap();
        let _future_up = armed
            .control
            .submit_key_up(
                &mut armed.control_port,
                MixedSourceKeyUp {
                    at: SampleTime::new(now),
                    handle: future_handle,
                },
            )
            .unwrap();
        let _reference_future_up = reference
            .control
            .submit_key_up(
                &mut reference.control_port,
                MixedSourceKeyUp {
                    at: SampleTime::new(now),
                    handle: reference_future_handle,
                },
            )
            .unwrap();
        while armed.audio.inner.audio.renderer.clock().as_u64() < 16640 {
            advance(&mut armed, &mut reference, 128);
        }
        assert!(armed.audio.fault().is_none());
    }
}

#[test]
fn mixed_fixed_continuation_changes_trim_without_resetting_sounding_delay_history() {
    let profile = profile(48_000.0);
    let mut armed = arm(profile);
    let mut reference = arm(profile);
    for _ in 0..32 {
        advance(&mut armed, &mut reference, 64);
    }
    let now = armed.audio.inner.audio.renderer.clock().as_u64();
    let (_, _held) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(now, 67))
        .unwrap();
    let (_, _reference_held) = reference
        .control
        .submit_note_on(&mut reference.control_port, note_on(now, 67))
        .unwrap();
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let at = armed.audio.inner.audio.renderer.clock();
    let packet = armed
        .control
        .prepare_continuation(targets(profile, 0.4), || Some(at), BYTES, RECORDS)
        .unwrap();
    let plan = packet.plan_id();
    armed.audio.offer_continuation(packet).unwrap();
    for _ in 0..8 {
        let mut actual = [0.0; 64];
        let mut expected = [0.0; 64];
        assert_eq!(
            count_allocs(
                || callback(&mut armed.audio, &mut armed.audio_port, &mut actual).unwrap()
            ),
            0
        );
        callback(
            &mut reference.audio,
            &mut reference.audio_port,
            &mut expected,
        )
        .unwrap();
        assert!(expected.iter().any(|sample| *sample != 0.0));
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(*actual, expected * 0.5);
        }
    }
    let retired = armed
        .control
        .collect_continuation(armed.audio.take_retired_continuation().unwrap())
        .unwrap();
    assert_eq!(
        retired.outcome(),
        Some(MixedContinuationOutcome::InstalledRendered { plan, boundary: at })
    );
}

#[test]
fn mixed_fixed_continuation_post_install_fault_returns_and_promotes_actual_geometry() {
    let profile = profile(48_000.0);
    let mut armed = arm(profile);
    let mut reference = arm(profile);
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let at = armed.audio.inner.audio.renderer.clock();
    let packet = armed
        .control
        .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
        .unwrap();
    let plan = packet.plan_id();
    armed.audio.inner.test_fail_after_ingress_at = Some(at);
    armed.audio.offer_continuation(packet).unwrap();
    let mut samples = [7.0; 64];
    let mut result = None;
    assert_eq!(
        count_allocs(|| {
            result = Some(callback(
                &mut armed.audio,
                &mut armed.audio_port,
                &mut samples,
            ));
        }),
        0
    );
    assert!(result.unwrap().is_err());
    assert!(samples.iter().all(|sample| *sample == 0.0));
    let packet = armed.audio.take_retired_continuation().unwrap();
    assert_eq!(
        packet.outcome(),
        Some(MixedContinuationOutcome::InstalledFaulted { plan, boundary: at })
    );
    let retired = armed.control.collect_continuation(packet).unwrap();
    assert_eq!(
        retired.outcome(),
        Some(MixedContinuationOutcome::InstalledFaulted { plan, boundary: at })
    );
    assert_eq!(armed.control.inner.control.plan.id(), plan);
    assert_eq!(armed.audio.inner.audio.renderer.plan().id(), plan);
    assert!(armed.control.fault().is_some());
    assert!(
        armed
            .control
            .join(
                armed.audio,
                Some(armed.control_port),
                Some(armed.audio_port)
            )
            .is_ok()
    );
}

#[test]
fn mixed_fixed_continuation_cold_refusal_returns_inputs_without_spending_credit() {
    use std::cell::Cell;
    let profile = profile(48_000.0);
    let mut armed = arm(profile);
    let mut reference = arm(profile);
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let sampled = Cell::new(false);
    let incoming = targets(profile, 0.8);
    let incoming_plan = incoming.plan().id();
    let refused = armed
        .control
        .prepare_continuation(
            incoming,
            || {
                sampled.set(true);
                Some(SampleTime::ZERO)
            },
            PreparedBytes::measured(0),
            RECORDS,
        )
        .unwrap_err();
    assert_eq!(refused.1.plan().id(), incoming_plan);
    assert!(
        !sampled.get(),
        "clock sampling follows every fallible allocation and proof"
    );
    assert!(armed.control.continuation_pending.is_none());
    let refused = armed
        .control
        .prepare_continuation(targets(profile, 0.8), || None, BYTES, RECORDS)
        .unwrap_err();
    assert!(matches!(refused.0, MixedContinuationPrepareError::Boundary));
    assert!(armed.control.continuation_pending.is_none());
    advance(&mut armed, &mut reference, 64);
}

#[test]
fn mixed_fixed_continuation_invalid_successor_target_refuses_before_any_actual_mutation() {
    let profile = profile(48_000.0);
    let mut armed = arm(profile);
    let mut reference = arm(profile);
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let old_plan = armed.control.inner.control.plan.id();
    let at = armed.audio.inner.audio.renderer.clock();
    let mut packet = armed
        .control
        .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
        .unwrap();
    packet.next.live_slot = crate::plan::NoteSlot::new(packet.proof.new, usize::MAX);
    armed.audio.offer_continuation(packet).unwrap();
    advance(&mut armed, &mut reference, 64);
    let packet = armed.audio.take_retired_continuation().unwrap();
    assert_eq!(
        packet.outcome(),
        Some(MixedContinuationOutcome::Refused(
            MixedContinuationRefusal::Storage
        ))
    );
    let _retired = armed.control.collect_continuation(packet).unwrap();
    assert_eq!(armed.control.inner.control.plan.id(), old_plan);
    assert_eq!(armed.audio.inner.audio.renderer.plan().id(), old_plan);
    assert!(armed.control.continuation_pending.is_none());
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
}

#[test]
fn mixed_fixed_continuation_quiescent_withdrawal_preserves_both_joined_owners() {
    let profile = profile(48_000.0);
    let mut armed = arm(profile);
    let mut reference = arm(profile);
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let old_plan = armed.control.inner.control.plan.id();
    let at = SampleTime::new(armed.audio.inner.audio.renderer.clock().as_u64() + 1024);
    let packet = armed
        .control
        .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
        .unwrap();
    armed.audio.offer_continuation(packet).unwrap();
    assert!(armed.audio.take_retired_continuation().is_none());
    let mut returned = None;
    assert_eq!(
        count_allocs(|| {
            returned = armed.audio.withdraw_continuation();
        }),
        0
    );
    let retired = armed
        .control
        .collect_continuation(returned.unwrap())
        .unwrap();
    assert_eq!(
        retired.outcome(),
        Some(MixedContinuationOutcome::Refused(
            MixedContinuationRefusal::Cancelled
        ))
    );
    assert_eq!(armed.control.inner.control.plan.id(), old_plan);
    assert!(
        armed
            .control
            .join(
                armed.audio,
                Some(armed.control_port),
                Some(armed.audio_port)
            )
            .is_ok()
    );
}

#[test]
fn mixed_fixed_continuation_retags_running_loop_templates_before_later_wraps() {
    use crate::{stream::MixedActivationKind, transport::LoopInterval};
    let profile = profile(48_000.0);
    let mut armed = arm(profile);
    let mut reference = arm(profile);
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let at = SampleTime::new(armed.audio.inner.audio.renderer.clock().as_u64() + 128);
    let start = PlanPosition::new(128);
    let interval = LoopInterval::new(start, PlanPosition::new(228)).unwrap();
    for rig in [&mut armed, &mut reference] {
        let offer = rig
            .control
            .prepare_loop_activation(at, start, interval, at, MixedActivationKind::Seek)
            .unwrap();
        rig.audio.offer_activation(offer).unwrap();
    }
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    for rig in [&mut armed, &mut reference] {
        let retired = rig.audio.take_retired_activation().unwrap();
        let _receipt = rig.control.collect_activation(retired).unwrap();
    }
    let wraps = armed.audio.compiled_loop().wraps();
    assert!(wraps.get() > 0);
    let at = SampleTime::new(armed.audio.inner.audio.renderer.clock().as_u64() + 128);
    let packet = armed
        .control
        .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
        .unwrap();
    let plan = packet.plan_id();
    armed.audio.offer_continuation(packet).unwrap();
    for _ in 0..16 {
        for frames in [7, 57, 128] {
            advance(&mut armed, &mut reference, frames);
        }
    }
    assert!(armed.audio.compiled_loop().wraps() > wraps);
    let retired = armed
        .control
        .collect_continuation(armed.audio.take_retired_continuation().unwrap())
        .unwrap();
    assert_eq!(
        retired.outcome(),
        Some(MixedContinuationOutcome::InstalledRendered { plan, boundary: at })
    );
}

#[test]
fn mixed_fixed_continuation_precedes_same_boundary_stop_and_keeps_live_release_valid() {
    let profile = profile(48_000.0);
    let mut armed = arm(profile);
    let mut reference = arm(profile);
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let now = armed.audio.inner.audio.renderer.clock();
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(now.as_u64(), 67))
        .unwrap();
    let (_, reference_handle) = reference
        .control
        .submit_note_on(&mut reference.control_port, note_on(now.as_u64(), 67))
        .unwrap();
    advance(&mut armed, &mut reference, 64);
    let at = SampleTime::new(armed.audio.inner.audio.renderer.clock().as_u64() + 128);
    let packet = armed
        .control
        .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
        .unwrap();
    let plan = packet.plan_id();
    armed.audio.offer_continuation(packet).unwrap();
    armed.audio.schedule_stop(at).unwrap();
    reference.audio.schedule_stop(at).unwrap();
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let retired = armed
        .control
        .collect_continuation(armed.audio.take_retired_continuation().unwrap())
        .unwrap();
    assert_eq!(
        retired.outcome(),
        Some(MixedContinuationOutcome::InstalledRendered { plan, boundary: at })
    );
    assert!(
        armed
            .audio
            .inner
            .stop_ended
            .iter()
            .flatten()
            .all(|ended| ended.note.plan() == plan)
    );
    let now = armed.audio.inner.audio.renderer.clock();
    let _up = armed
        .control
        .submit_key_up(
            &mut armed.control_port,
            MixedSourceKeyUp { at: now, handle },
        )
        .unwrap();
    let _reference_up = reference
        .control
        .submit_key_up(
            &mut reference.control_port,
            MixedSourceKeyUp {
                at: now,
                handle: reference_handle,
            },
        )
        .unwrap();
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    assert!(
        armed
            .control
            .join(
                armed.audio,
                Some(armed.control_port),
                Some(armed.audio_port)
            )
            .is_ok()
    );
}

#[test]
fn mixed_fixed_continuation_refuses_uncollected_activation_before_rebinding() {
    use crate::stream::MixedActivationKind;
    for take_receipt in [false, true] {
        let profile = profile(48_000.0);
        let mut armed = arm(profile);
        let mut reference = arm(profile);
        for _ in 0..8 {
            advance(&mut armed, &mut reference, 64);
        }
        let at = SampleTime::new(armed.audio.inner.audio.renderer.clock().as_u64() + 128);
        for rig in [&mut armed, &mut reference] {
            let offer = rig
                .control
                .prepare_activation(at, PlanPosition::new(128), at, MixedActivationKind::Seek)
                .unwrap();
            rig.audio.offer_activation(offer).unwrap();
        }
        for _ in 0..8 {
            advance(&mut armed, &mut reference, 64);
        }
        let receipt = if take_receipt {
            armed.audio.take_retired_activation()
        } else {
            None
        };
        let old = armed.control.inner.control.plan.id();
        let at = armed.audio.inner.audio.renderer.clock();
        let refused = armed
            .control
            .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
            .unwrap_err();
        assert!(matches!(refused.0, MixedContinuationPrepareError::Busy));
        assert_eq!(armed.audio.inner.audio.renderer.plan().id(), old);
        let receipt = receipt
            .or_else(|| armed.audio.take_retired_activation())
            .unwrap();
        let _receipt = armed.control.collect_activation(receipt).unwrap();
        let _reference_receipt = reference
            .control
            .collect_activation(reference.audio.take_retired_activation().unwrap())
            .unwrap();
        advance(&mut armed, &mut reference, 64);
    }
}

#[test]
fn mixed_fixed_continuation_cancelled_receipt_blocks_rebinding_until_collected() {
    use crate::stream::MixedActivationKind;
    let profile = profile(48_000.0);
    let mut armed = arm(profile);
    let mut reference = arm(profile);
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let now = armed.audio.inner.audio.renderer.clock().as_u64();
    let stop = SampleTime::new(now + 128);
    let at = SampleTime::new(now + 256);
    let offer = armed
        .control
        .prepare_activation(at, PlanPosition::new(128), at, MixedActivationKind::Seek)
        .unwrap();
    armed.audio.offer_activation(offer).unwrap();
    armed.audio.schedule_stop(stop).unwrap();
    reference.audio.schedule_stop(stop).unwrap();
    for _ in 0..8 {
        advance(&mut armed, &mut reference, 64);
    }
    let retired = armed.audio.take_retired_activation().unwrap();
    let at = armed.audio.inner.audio.renderer.clock();
    let refused = armed
        .control
        .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
        .unwrap_err();
    assert!(matches!(refused.0, MixedContinuationPrepareError::Busy));
    let _receipt = armed.control.collect_activation(retired).unwrap();
    let packet = armed
        .control
        .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
        .unwrap();
    armed.audio.offer_continuation(packet).unwrap();
    advance(&mut armed, &mut reference, 64);
    let _retired = armed
        .control
        .collect_continuation(armed.audio.take_retired_continuation().unwrap())
        .unwrap();
}

#[test]
fn mixed_fixed_continuation_abandoned_packets_recover_without_replacing_actual_authorities() {
    for installed in [false, true] {
        let profile = profile(48_000.0);
        let mut armed = arm(profile);
        let mut reference = arm(profile);
        for _ in 0..8 {
            advance(&mut armed, &mut reference, 64);
        }
        let at = armed.audio.inner.audio.renderer.clock();
        let packet = armed
            .control
            .prepare_continuation(targets(profile, 0.8), || Some(at), BYTES, RECORDS)
            .unwrap();
        let new = packet.plan_id();
        let table = armed.audio.inner.audio.renderer.table_id();
        if installed {
            armed.audio.offer_continuation(packet).unwrap();
            advance(&mut armed, &mut reference, 64);
            drop(armed.audio.take_retired_continuation().unwrap());
        } else {
            drop(packet);
        }
        assert_eq!(
            armed
                .control
                .recover_abandoned_continuation(&mut armed.audio)
                .unwrap(),
            Some(if installed {
                MixedContinuationRecovery::InstalledUnconfirmed { plan: new }
            } else {
                MixedContinuationRecovery::Unpublished
            })
        );
        assert_eq!(armed.audio.inner.audio.renderer.table_id(), table);
        advance(&mut armed, &mut reference, 64);
        assert!(
            armed
                .control
                .join(
                    armed.audio,
                    Some(armed.control_port),
                    Some(armed.audio_port)
                )
                .is_ok()
        );
    }
}

mod producer_tests;
