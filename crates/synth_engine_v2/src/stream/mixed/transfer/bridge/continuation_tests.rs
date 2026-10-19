use super::*;
use crate::render_allocation::count_allocs;
use crate::stream::mixed::transfer::continuation::tests::{BYTES, RECORDS, prepared, targets};
use crate::stream::{MixedActivationKind, MixedContinuationOutcome};
use crate::transport::LoopInterval;

fn rig() -> ResetRig {
    let first = Source::new("continuing-first");
    let second = Source::new("continuing-second");
    let sources = MixedTransferSources::new(first.generation, second.generation).unwrap();
    let profile = history_tests::mixed_profile();
    let (prepared, candidate) = prepared(profile);
    let depth = MixedJoinedPrepared::transfer_capacity(&profile).get();
    let MixedBridgeArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = prepared
        .arm_bridge_rehearsal(
            candidate,
            &profile,
            sources,
            fence(),
            Ring(HeapRb::new(depth + 1)),
            Ring(HeapRb::new(depth + 1)),
        )
        .unwrap();
    ResetRig {
        running_reset: false,
        control,
        port: control_port,
        wrapper: Wrapper::new(Owner::new(audio, audio_port).unwrap()),
        first,
        second,
        sources,
        next_control: None,
        next_port: None,
        next_owner: None,
    }
}
fn frontier(rig: &mut ResetRig, at: u64) {
    let (packet, at) = rig.first.frontier(at);
    rig.control.advance_frontier(packet, at).unwrap();
    let (packet, at) = rig.second.frontier(at.as_u64());
    rig.control.advance_frontier(packet, at).unwrap();
}
fn advance(rig: &mut ResetRig, reference: &mut ResetRig) {
    let mut actual = [0.0; 64];
    let mut expected = [0.0; 64];
    assert_eq!(
        count_allocs(|| rig
            .wrapper
            .render(AudioBlockMut::new(&mut actual, 64, ChannelLayout::Mono).unwrap())
            .unwrap()),
        0
    );
    reference
        .wrapper
        .render(AudioBlockMut::new(&mut expected, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(actual, expected);
    let _reports = rig.collect();
    let _reference_reports = reference.collect();
}
fn held_and_sustained(rig: &mut ResetRig) {
    let first = rig.first.handoff(100, ON);
    let second = rig.second.handoff(110, [0x90, 67, 100]);
    let pedal = rig.first.handoff(180, [0xB0, 64, 127]);
    let up = rig.first.handoff(200, OFF);
    for packet in [first, second, pedal, up] {
        assert_eq!(rig.stage(packet), Ok(MixedStaged::Staged));
    }
    frontier(rig, 201);
    let steps = pump_all(rig);
    assert_eq!(
        steps
            .iter()
            .filter(|step| matches!(step, MixedPumpStep::Forwarded { .. }))
            .count(),
        4
    );
}

#[test]
fn two_source_continuation_preserves_sustain_source_frontiers_and_running_loop() {
    for looping in [false, true] {
        let mut rig = rig();
        let mut reference = self::rig();
        let binding = rig.control.transfer.binding;
        held_and_sustained(&mut rig);
        held_and_sustained(&mut reference);
        for _ in 0..16 {
            advance(&mut rig, &mut reference);
        }
        if looping {
            let at = SampleTime::new(rig.control.audio_boundary().as_u64() + 128);
            let interval =
                LoopInterval::new(PlanPosition::new(128), PlanPosition::new(228)).unwrap();
            for rig in [&mut rig, &mut reference] {
                let offer = rig
                    .control
                    .prepare_loop_activation(
                        at,
                        PlanPosition::new(128),
                        interval,
                        at,
                        MixedActivationKind::Seek,
                    )
                    .unwrap();
                rig.wrapper.offer_activation(offer).unwrap();
            }
            for _ in 0..8 {
                advance(&mut rig, &mut reference);
            }
            for rig in [&mut rig, &mut reference] {
                let _receipt = rig
                    .control
                    .collect_activation(rig.wrapper.take_retired_activation().unwrap())
                    .unwrap();
            }
        }
        let at = SampleTime::new(rig.control.audio_boundary().as_u64() + 128);
        let packet = rig
            .control
            .prepare_continuation(
                targets(history_tests::mixed_profile(), 0.8),
                || Some(at),
                BYTES,
                RECORDS,
            )
            .unwrap();
        let plan = packet.plan_id();
        rig.wrapper.offer_continuation(packet).unwrap();
        for _ in 0..12 {
            advance(&mut rig, &mut reference);
        }
        let retired = rig
            .control
            .collect_continuation(rig.wrapper.take_retired_continuation().unwrap())
            .unwrap();
        assert_eq!(
            retired.outcome(),
            Some(MixedContinuationOutcome::InstalledRendered { plan, boundary: at })
        );
        assert_eq!(rig.control.transfer.binding, binding);
        let now = rig.control.audio_boundary().as_u64() + 32;
        for rig in [&mut rig, &mut reference] {
            let lift = rig.first.handoff(now, [0xB0, 64, 0]);
            let up = rig.second.handoff(now + 1, [0x80, 67, 0]);
            assert_eq!(rig.stage(lift), Ok(MixedStaged::Staged));
            assert_eq!(rig.stage(up), Ok(MixedStaged::Staged));
            frontier(rig, now + 2);
            let _steps = pump_all(rig);
        }
        for _ in 0..12 {
            advance(&mut rig, &mut reference);
        }
        assert!(rig.control.fault().is_none());
        let (active, secondary) = rig.wrapper.into_owners();
        assert!(secondary.is_none());
        let (audio, audio_port) = active.into_parts();
        assert_eq!(audio.inner.audio.renderer.table_id(), binding.owner.table);
        assert_eq!(audio.inner.audio.renderer.epoch(), binding.owner.epoch);
        assert_eq!(audio.inner.audio.live_minter.0.minted, 2);
        assert!(
            rig.control
                .join(audio, Some(rig.port), Some(audio_port))
                .is_ok()
        );
    }
}

#[test]
fn a_protected_panic_at_the_continuation_boundary_keeps_the_same_source_binding() {
    let mut rig = rig();
    let mut reference = self::rig();
    held_and_sustained(&mut rig);
    held_and_sustained(&mut reference);
    for _ in 0..16 {
        advance(&mut rig, &mut reference);
    }
    let at = SampleTime::new(rig.control.audio_boundary().as_u64() + 128);
    let packet = rig
        .control
        .prepare_continuation(
            targets(history_tests::mixed_profile(), 0.8),
            || Some(at),
            BYTES,
            RECORDS,
        )
        .unwrap();
    let plan = packet.plan_id();
    rig.wrapper.offer_continuation(packet).unwrap();
    for rig in [&mut rig, &mut reference] {
        frontier(rig, at.as_u64() + 1);
        rig.control.panic(at).unwrap();
        let _steps = pump_all(rig);
    }
    for _ in 0..8 {
        advance(&mut rig, &mut reference);
    }
    let retired = rig
        .control
        .collect_continuation(rig.wrapper.take_retired_continuation().unwrap())
        .unwrap();
    assert_eq!(
        retired.outcome(),
        Some(MixedContinuationOutcome::InstalledRendered { plan, boundary: at })
    );
    let now = rig.control.audio_boundary().as_u64() + 32;
    for rig in [&mut rig, &mut reference] {
        let up = rig.second.handoff(now, [0x80, 67, 0]);
        assert_eq!(rig.stage(up), Ok(MixedStaged::Staged));
        frontier(rig, now + 1);
        let _steps = pump_all(rig);
    }
    for _ in 0..8 {
        advance(&mut rig, &mut reference);
    }
    assert!(rig.control.fault().is_none());
}

#[test]
fn continuation_success_waits_for_the_wrappers_complete_multi_quantum_callback() {
    let mut rig = rig();
    let mut reference = self::rig();
    for _ in 0..8 {
        advance(&mut rig, &mut reference);
    }
    let at = rig.control.audio_boundary();
    let packet = rig
        .control
        .prepare_continuation(
            targets(history_tests::mixed_profile(), 0.8),
            || Some(at),
            BYTES,
            RECORDS,
        )
        .unwrap();
    let plan = packet.plan_id();
    rig.wrapper.offer_continuation(packet).unwrap();
    let (active, secondary) = rig.wrapper.into_owners();
    assert!(secondary.is_none());
    let (mut audio, port) = active.into_parts();
    audio.inner.test_fail_after_ingress_at = Some(SampleTime::new(at.as_u64() + 64));
    rig.wrapper = MixedBridgeResetAudio::new(MixedBridgeResetOwner::new(audio, port).unwrap());
    let mut output = [7.0; 128];
    let mut result = None;
    assert_eq!(
        count_allocs(|| {
            result = Some(
                rig.wrapper
                    .render(AudioBlockMut::new(&mut output, 128, ChannelLayout::Mono).unwrap()),
            );
        }),
        0
    );
    assert!(result.unwrap().is_err());
    assert!(output.iter().all(|sample| *sample == 0.0));
    let retired = rig
        .control
        .collect_continuation(rig.wrapper.take_retired_continuation().unwrap())
        .unwrap();
    assert_eq!(
        retired.outcome(),
        Some(MixedContinuationOutcome::InstalledFaulted { plan, boundary: at })
    );
    assert!(rig.control.fault().is_some());
}

#[test]
fn abandoned_bridge_continuations_recover_through_both_public_host_paths() {
    use crate::stream::MixedContinuationRecovery;
    for installed in [false, true] {
        let mut rig = rig();
        let mut reference = self::rig();
        held_and_sustained(&mut rig);
        held_and_sustained(&mut reference);
        for _ in 0..8 {
            advance(&mut rig, &mut reference);
        }
        let binding = rig.control.transfer.binding;
        let at = rig.control.audio_boundary();
        let packet = rig
            .control
            .prepare_continuation(
                targets(history_tests::mixed_profile(), 0.8),
                || Some(at),
                BYTES,
                RECORDS,
            )
            .unwrap();
        let plan = packet.plan_id();
        let recovery = if installed {
            rig.wrapper.offer_continuation(packet).unwrap();
            advance(&mut rig, &mut reference);
            drop(rig.wrapper.take_retired_continuation().unwrap());
            rig.wrapper
                .recover_abandoned_continuation(&mut rig.control)
                .unwrap()
        } else {
            drop(packet);
            let (active, secondary) = rig.wrapper.into_owners();
            assert!(secondary.is_none());
            let (mut audio, port) = active.into_parts();
            let recovery = rig
                .control
                .recover_abandoned_continuation(&mut audio)
                .unwrap();
            rig.wrapper =
                MixedBridgeResetAudio::new(MixedBridgeResetOwner::new(audio, port).unwrap());
            recovery
        };
        assert_eq!(
            recovery,
            Some(if installed {
                MixedContinuationRecovery::InstalledUnconfirmed { plan }
            } else {
                MixedContinuationRecovery::Unpublished
            })
        );
        assert_eq!(rig.control.transfer.binding, binding);
        for _ in 0..8 {
            advance(&mut rig, &mut reference);
        }
        let now = rig.control.audio_boundary().as_u64() + 32;
        for rig in [&mut rig, &mut reference] {
            let lift = rig.first.handoff(now, [0xB0, 64, 0]);
            let up = rig.second.handoff(now + 1, [0x80, 67, 0]);
            assert_eq!(rig.stage(lift), Ok(MixedStaged::Staged));
            assert_eq!(rig.stage(up), Ok(MixedStaged::Staged));
            frontier(rig, now + 2);
            let _steps = pump_all(rig);
        }
        for _ in 0..8 {
            advance(&mut rig, &mut reference);
        }
        let (active, secondary) = rig.wrapper.into_owners();
        assert!(secondary.is_none());
        let (audio, port) = active.into_parts();
        assert!(rig.control.join(audio, Some(rig.port), Some(port)).is_ok());
    }
}
