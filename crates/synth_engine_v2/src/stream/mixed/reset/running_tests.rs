//! Explicit running mixed reset against ordinary old/successor renders.
use super::super::host::MixedBridgeStopRefusal;
use super::*;

fn running_arm(successor: bool) -> Armed {
    let prepared = bound_with_events(true, |slot| {
        let edges: &[(u64, bool)] = if successor {
            &[
                (0, true),
                (1, false),
                (1, true),
                (65, false),
                (130, true),
                (200, false),
            ]
        } else {
            // Engine stamps B-1, B, B+Q-1 and B+Q. Off precedes equal-stamp On.
            &[
                (0, true),
                (200, false),
                (383, true),
                (384, false),
                (384, true),
                (447, false),
                (447, true),
                (448, false),
                (448, true),
                (520, false),
            ]
        };
        edges
            .iter()
            .map(|&(at, on)| {
                PlanEvent::new(
                    PlanPosition::new(at),
                    if on {
                        CompiledPayload::NoteOn {
                            slot,
                            key: key(60),
                            velocity: NoteVelocity::FULL,
                        }
                    } else {
                        CompiledPayload::NoteOff { slot, key: key(60) }
                    },
                )
            })
            .collect()
    });
    let requested = if successor { B } else { SampleTime::new(64) };
    let history = prepared
        .prepare_history(requested, PlanPosition::ZERO)
        .unwrap();
    let suffix = prepared.prepare_suffix(history).unwrap();
    let candidate = prepared.stamp_suffix(suffix).unwrap();
    let mut armed = prepared
        .arm_transfer_rehearsal(candidate, &mixed_profile(), ring(), ring())
        .unwrap();
    let _held = onset(
        &mut armed.control,
        &mut armed.control_port,
        if successor { B.as_u64() + 10 } else { 100 },
        if successor { 55 } else { 48 },
    );
    armed
}

// Seed the ordinary fresh renderer at B without adopting through the reset wrapper.
// Running it from zero would warm its DSP and would not model fresh-owner reset.
fn fresh_successor_sound() -> Vec<f32> {
    let mut armed = running_arm(true);
    let inner = armed.audio.inner_mut();
    inner
        .audio
        .renderer
        .install_stopped_clock(inner.effective_anchor);
    let mut sound = vec![0.0; 1024];
    render_alone(
        &mut armed.audio,
        &mut armed.audio_port,
        &mut sound[B_OUTPUT..],
    );
    sound
}

fn public_running_prefix() -> (Public, MixedTransferControl, ControlPort) {
    let mut armed = running_arm(false);
    let mut prefix = [0.0; 256];
    render_alone(&mut armed.audio, &mut armed.audio_port, &mut prefix);
    let _reports = reports(&mut armed.control, &mut armed.control_port);
    let wrapper = MixedBridgeResetAudio::new(
        MixedBridgeResetOwner::new(armed.audio, armed.audio_port).unwrap(),
    );
    (wrapper, armed.control, armed.control_port)
}

#[test]
fn running_mixed_reset_matches_ordinary_twins_across_fade_and_onset_edges() {
    let mut old_reference = running_arm(false);
    old_reference
        .audio
        .inner_mut()
        .schedule_compiled_stop(SampleTime::new(B.as_u64() + u64::from(QUANTUM_FRAMES)))
        .unwrap();
    let mut old_sound = vec![0.0; 1024];
    render_alone(
        &mut old_reference.audio,
        &mut old_reference.audio_port,
        &mut old_sound,
    );
    let old_report = old_reference.audio.inner_mut().report();
    assert_eq!(
        old_report.suffix_charged.get(),
        7,
        "no old edge at or after B + Q publishes"
    );
    let old_released = old_reference.audio.inner_mut().stop_released;
    let new_sound = fresh_successor_sound();
    let mut expected = old_sound.clone();
    for index in B_OUTPUT..expected.len() {
        expected[index] = if index < FADE_END_OUTPUT {
            let gain = fade_gain(index - B_OUTPUT).unwrap();
            new_sound[index] * gain + old_sound[index] * (1.0 - gain)
        } else {
            new_sound[index]
        };
    }
    for partition in [1, 3, 63, 64, 65, 512] {
        let mut old = running_arm(false);
        let mut sound = vec![0.0; 1024];
        render_alone(&mut old.audio, &mut old.audio_port, &mut sound[..256]);
        let _reports = reports(&mut old.control, &mut old.control_port);
        let mut wrapper = MixedBridgeResetAudio::new(
            MixedBridgeResetOwner::new(old.audio, old.audio_port).unwrap(),
        );
        let next = running_arm(true);
        let successor = MixedBridgeResetOwner::new(next.audio, next.audio_port).unwrap();
        assert_eq!(
            count_allocs(|| wrapper.offer_running_reset(successor, B).unwrap()),
            0
        );
        assert_eq!(
            wrapper.schedule_stop(SampleTime::new(256)),
            Err(MixedBridgeStopRefusal::ResetPending)
        );
        for chunk in sound[256..].chunks_mut(partition) {
            assert_eq!(count_allocs(|| wrapper.render(block(chunk)).unwrap()), 0);
        }
        let mismatch = sound
            .iter()
            .zip(&expected)
            .position(|(a, b)| a.to_bits() != b.to_bits());
        assert_eq!(
            mismatch,
            None,
            "partition={partition}, first={:?}",
            mismatch.map(|i| (i, sound[i], expected[i]))
        );
        let retired = wrapper.take_retired().unwrap();
        let cut = retired.receipt().compiled_cut().unwrap();
        assert_eq!(cut.stop_at(), SampleTime::new(512));
        assert_eq!(cut.old_position_at_b(), Some(PlanPosition::new(384)));
        assert_eq!(cut.old_pass(), None);
        assert_eq!(retired.compiled_released(), Some(old_released));
        assert_eq!(retired.compiled_released().unwrap().get(), 1);
        let (mut old_audio, old_port) = retired.into_owner().into_parts();
        let retired_report = old_audio.inner_mut().report();
        assert_eq!(retired_report.suffix_charged, old_report.suffix_charged);
        let inner = old_audio.inner_mut();
        assert_eq!(inner.audio.compiled_minter.live(), 0);
        assert_eq!(inner.audio.live_minter.live(), 0);
        assert!(!inner.audio.renderer.has_replacement_obligations());
        assert_eq!(inner.test_ingress.holds_outstanding(), EventCount::NONE);
        let joined = old
            .control
            .join(old_audio, Some(old.control_port), Some(old_port))
            .unwrap();
        assert_eq!(joined.custody(), MixedTransferCustody::Complete);
        let (next_owner, secondary) = wrapper.into_owners();
        assert!(secondary.is_none());
        let (audio, port) = next_owner.into_parts();
        assert_eq!(
            next.control
                .join(audio, Some(next.control_port), Some(port))
                .unwrap()
                .custody(),
            MixedTransferCustody::Complete
        );
    }
}

#[test]
fn running_reset_is_explicit_and_refusal_cannot_schedule_a_future_stop() {
    let (mut wrapper, control, control_port) = public_running_prefix();
    let successor = running_arm(true);
    let successor = MixedBridgeResetOwner::new(successor.audio, successor.audio_port).unwrap();
    let (reason, successor) = wrapper.offer_reset(successor, B).unwrap_err();
    assert_eq!(reason, MixedBridgeResetRefusal::Running);
    // The successor passed through the legacy refusal unchanged and is usable.
    wrapper.offer_running_reset(successor, B).unwrap();
    let withdrawn = wrapper.withdraw_pending().unwrap();
    let _withdrawn_parts = withdrawn.into_parts();
    let (mut reference, reference_control, reference_port) = public_running_prefix();
    let mut actual = [0.0; 512];
    let mut expected = [0.0; 512];
    wrapper.render(block(&mut actual)).unwrap();
    reference.render(block(&mut expected)).unwrap();
    assert_eq!(actual, expected);
    let (owner, _) = wrapper.into_owners();
    let (audio, port) = owner.into_parts();
    let joined = control.join(audio, Some(control_port), Some(port)).unwrap();
    assert_eq!(joined.custody(), MixedTransferCustody::Complete);
    let (owner, _) = reference.into_owners();
    let (audio, port) = owner.into_parts();
    let joined = reference_control
        .join(audio, Some(reference_port), Some(port))
        .unwrap();
    assert_eq!(joined.custody(), MixedTransferCustody::Complete);
}

fn enter_loop(armed: &mut Armed, length: u64) -> Vec<f32> {
    let interval =
        crate::transport::LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(length)).unwrap();
    let offer = armed
        .control
        .prepare_loop_activation(
            SampleTime::new(128),
            PlanPosition::ZERO,
            interval,
            SampleTime::new(128),
            MixedActivationKind::Seek,
        )
        .unwrap();
    armed.audio.offer_activation(offer).unwrap();
    let mut prefix = vec![0.0; 256];
    render_alone(&mut armed.audio, &mut armed.audio_port, &mut prefix);
    let receipt = armed
        .control
        .collect_activation(armed.audio.take_retired_activation().unwrap())
        .unwrap();
    assert!(matches!(
        receipt.outcome(),
        MixedActivationOutcome::Adopted(_)
    ));
    let _reports = reports(&mut armed.control, &mut armed.control_port);
    prefix
}

#[test]
fn running_reset_cuts_old_loops_before_or_after_wrap_without_reanchoring_successor() {
    for length in [320, 321, 384, 385] {
        let mut reference = running_arm(false);
        let mut expected = enter_loop(&mut reference, length);
        reference
            .audio
            .inner_mut()
            .schedule_compiled_stop(SampleTime::new(512))
            .unwrap();
        expected.resize(1024, 0.0);
        render_alone(
            &mut reference.audio,
            &mut reference.audio_port,
            &mut expected[256..],
        );
        let released = reference.audio.inner_mut().stop_released;
        let suffix = reference.audio.inner_mut().report().suffix_charged;
        let next_sound = fresh_successor_sound();
        for index in B_OUTPUT..expected.len() {
            expected[index] = if index < FADE_END_OUTPUT {
                let gain = fade_gain(index - B_OUTPUT).unwrap();
                next_sound[index] * gain + expected[index] * (1.0 - gain)
            } else {
                next_sound[index]
            };
        }
        for partition in [1, 63, 65, 512] {
            let mut old = running_arm(false);
            let mut actual = enter_loop(&mut old, length);
            let next = running_arm(true);
            let mut wrapper = public(old.audio, old.audio_port);
            wrapper
                .offer_running_reset(
                    MixedBridgeResetOwner::new(next.audio, next.audio_port).unwrap(),
                    B,
                )
                .unwrap();
            actual.resize(1024, 0.0);
            for chunk in actual[256..].chunks_mut(partition) {
                assert_eq!(count_allocs(|| wrapper.render(block(chunk)).unwrap()), 0);
            }
            assert_eq!(actual, expected, "length={length} partition={partition}");
            let retired = wrapper.take_retired().unwrap();
            assert_eq!(retired.compiled_released(), Some(released));
            let cut = retired.receipt().compiled_cut().unwrap();
            assert_eq!(
                cut.old_position_at_b(),
                Some(PlanPosition::new(320 % length))
            );
            assert_eq!(cut.old_pass(), Some(crate::looping::LoopPassId::FIRST));
            let (mut audio, port) = retired.into_owner().into_parts();
            assert_eq!(audio.inner_mut().report().suffix_charged, suffix);
            assert_eq!(audio.inner_mut().audio.compiled_minter.live(), 0);
            assert_eq!(audio.inner_mut().audio.live_minter.live(), 0);
            let joined = old
                .control
                .join(audio, Some(old.control_port), Some(port))
                .unwrap();
            assert_eq!(joined.custody(), MixedTransferCustody::Complete);
            let (owner, secondary) = wrapper.into_owners();
            assert!(secondary.is_none());
            let (audio, port) = owner.into_parts();
            let joined = next
                .control
                .join(audio, Some(next.control_port), Some(port))
                .unwrap();
            assert_eq!(joined.custody(), MixedTransferCustody::Complete);
        }
    }
}

#[test]
fn boundary_refusals_keep_running_audio_and_transport_equal_to_an_unoffered_twin() {
    for refusal in 0..3 {
        let mut old = running_arm(false);
        old.control.attach_reset_seal(&mut old.audio);
        let mut reference = running_arm(false);
        let mut prefix = [0.0; 256];
        render_alone(&mut old.audio, &mut old.audio_port, &mut prefix);
        render_alone(&mut reference.audio, &mut reference.audio_port, &mut prefix);
        let _reports = reports(&mut old.control, &mut old.control_port);
        let next = running_arm(true);
        assert!(old.control.seal_reset_for(next.control_port.binding));
        let mut wrapper = Wrapper::new(owner(old.audio, old.audio_port));
        wrapper
            .offer_running_reset(owner(next.audio, next.audio_port), B)
            .unwrap();
        match refusal {
            0 => {
                let _old = onset(&mut old.control, &mut old.control_port, B.as_u64() + 5, 52);
                let _twin = onset(
                    &mut reference.control,
                    &mut reference.control_port,
                    B.as_u64() + 5,
                    52,
                );
            }
            1 => assert!(old.control.cancel_reset_seal()),
            _ => {
                let MixedResetSlot::Pending { successor, .. } = &mut wrapper.pair.secondary else {
                    panic!("pending");
                };
                successor.effective_anchor =
                    crate::time::StreamAnchor::new(SampleTime::new(384), PlanPosition::ZERO);
            }
        }
        let mut actual = [0.0; 512];
        let mut expected = [0.0; 512];
        assert_eq!(
            count_allocs(|| wrapper.render(block(&mut actual)).unwrap()),
            0
        );
        render_alone(
            &mut reference.audio,
            &mut reference.audio_port,
            &mut expected,
        );
        assert_eq!(actual, expected, "refusal={refusal}");
        let refused = wrapper.take_refused().unwrap();
        match refusal {
            0 => assert_eq!(refused.reason, MixedResetRefused::IngressPending),
            1 => assert_eq!(refused.reason, MixedResetRefused::PanicCancelled),
            _ => assert_eq!(refused.reason, MixedResetRefused::SuccessorSchedule),
        }
        let (owner, secondary) = wrapper.into_owners();
        assert!(secondary.is_none());
        let (mut audio, _port) = owner.into_parts();
        let actual = audio.inner_mut();
        let expected = reference.audio.inner_mut();
        assert_eq!(actual.stop_pending, expected.stop_pending);
        assert_eq!(actual.transport, expected.transport);
        assert_eq!(actual.next, expected.next);
        assert_eq!(
            actual.audio.compiled_minter.live(),
            expected.audio.compiled_minter.live()
        );
        assert_eq!(
            actual.audio.live_minter.live(),
            expected.audio.live_minter.live()
        );
        assert_eq!(
            actual.test_ingress.holds_outstanding(),
            expected.test_ingress.holds_outstanding()
        );
        assert_eq!(
            actual.report().suffix_charged,
            expected.report().suffix_charged
        );
    }
}

#[test]
fn running_entry_returns_the_fresh_successor_whole_for_unadopted_and_stopped_owners() {
    let old = running_arm(false);
    let next = running_arm(true);
    let mut wrapper = public(old.audio, old.audio_port);
    let successor = MixedBridgeResetOwner::new(next.audio, next.audio_port).unwrap();
    let binding = successor.binding();
    let (reason, successor) = wrapper.offer_running_reset(successor, B).unwrap_err();
    assert_eq!(reason, MixedBridgeResetRefusal::NotAdopted);
    assert_eq!(successor.binding(), binding);
    wrapper.schedule_stop(STOP).unwrap();
    let mut output = [0.0; 320];
    wrapper.render(block(&mut output)).unwrap();
    let (reason, successor) = wrapper.offer_running_reset(successor, B).unwrap_err();
    assert_eq!(reason, MixedBridgeResetRefusal::NotRunning);
    assert_eq!(successor.binding(), binding);
    assert!(successor.fresh());
}
