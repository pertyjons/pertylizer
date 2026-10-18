//! EVD-0026's sustained functional readiness control. No callback timing, capacity
//! collection or numeric selection is performed here.
use super::*;

#[path = "capacity_collection.rs"]
mod capacity_collection;
use synth_engine_v2::stream::MixedBridgeResetReceipt;

const WARMUP_CALLBACKS: usize = 1_024;
const EXERCISE_CALLBACKS: usize = 10_000;
const TOTAL_CALLBACKS: usize = WARMUP_CALLBACKS + EXERCISE_CALLBACKS;

#[derive(Debug)]
struct ActivationEntry {
    boundary: SampleTime,
    looped: bool,
    receipt: MixedActivationAdopted,
    first_quantum_wraps: EventCount,
}

#[derive(Debug)]
struct FunctionalTranscript {
    activations: Vec<ActivationEntry>,
    messages: Vec<(usize, InputObservation)>,
    callbacks_with_four_holds: usize,
    stops: Vec<SampleTime>,
    starts: Vec<SampleTime>,
    panics: Vec<MixedBridgeReport>,
    resets: Vec<MixedBridgeResetReceipt>,
    pressure: Vec<SampleTime>,
}

pub(super) fn sustained_profile(hz: f32) -> HostProfile {
    super::super::capacity_selection_controls::capacity_selection::candidate_profile(
        synth_engine_v2::quantities::SampleRate::new(hz).expect("candidate rate"),
        FrameCount::new(64),
        ChannelLayout::Stereo,
    )
    .expect("the candidate envelope admits fixed 64-frame stereo")
}

/// Frame-stamped requests, independent of profile capacities and sample rate.
/// Four sound holds are legal, distinct from the source rings' larger custody bounds.
fn source_requests(callback: usize) -> Vec<(usize, u64, Midi1Input)> {
    let base = u64::try_from(callback).expect("bounded callback") * 64;
    let requests = match callback {
        1 => vec![
            (FIRST, 8, note_on(60)),
            (FIRST, 9, note_on(62)),
            (SECOND, 10, note_on(64)),
            (SECOND, 11, note_on(67)),
        ],
        512 => vec![(FIRST, 8, bend(9_000)), (SECOND, 10, bend(8_000))],
        1_024 => vec![(FIRST, 8, sustain(true))],
        1_025 => vec![(FIRST, 8, note_off(60)), (FIRST, 9, note_off(62))],
        1_026 => vec![(FIRST, 8, sustain(false))],
        1_027 => vec![(FIRST, 8, note_on(60)), (FIRST, 9, note_on(62))],
        3_001 | 6_005 => vec![
            (FIRST, 8, note_off(60)),
            (FIRST, 9, note_off(62)),
            (SECOND, 10, note_off(64)),
            (SECOND, 11, note_off(67)),
        ],
        3_002 | 4_004 | 6_006 => vec![
            (FIRST, 8, note_on(60)),
            (FIRST, 9, note_on(62)),
            (SECOND, 10, note_on(64)),
            (SECOND, 11, note_on(67)),
        ],
        4_003 => {
            // Each source fills all sixteen ordinary ring places, then offers
            // both protected key-ups. Its two held keys make each bend two
            // ingress entries. The current 28-frame loop wraps three times.
            let mut pressure = Vec::new();
            for source in [FIRST, SECOND] {
                for index in 0..16_u16 {
                    pressure.push((source, 8 + u64::from(index), bend(8_193 + index)));
                }
                let keys = if source == FIRST { [60, 62] } else { [64, 67] };
                for (offset, key) in keys.into_iter().enumerate() {
                    pressure.push((
                        source,
                        24 + u64::try_from(offset).expect("two keys"),
                        note_off(key),
                    ));
                }
            }
            pressure
        }
        4_096 => vec![(SECOND, 8, sustain(true))],
        4_097 => vec![(SECOND, 8, note_off(64)), (SECOND, 9, note_off(67))],
        4_098 => vec![(SECOND, 8, sustain(false))],
        4_099 => vec![(SECOND, 8, note_on(64)), (SECOND, 9, note_on(67))],
        8_000 => vec![(FIRST, 8, bend(8_192)), (SECOND, 10, bend(8_192))],
        11_020 => vec![
            (FIRST, 8, note_off(60)),
            (FIRST, 9, note_off(62)),
            (SECOND, 10, note_off(64)),
            (SECOND, 11, note_off(67)),
        ],
        _ => vec![],
    };
    requests
        .into_iter()
        .map(|(source, offset, input)| (source, base + offset, input))
        .collect()
}

pub(super) fn assert_applied_sound_outcome(report: MixedBridgeReport) {
    assert_eq!(
        report.nominal_at(),
        report.stamp(),
        "healthy peers must not defer sound"
    );
    assert_eq!(
        report.stamp(),
        report.executed_at(),
        "no callback delivered sound late"
    );
    match report.outcome() {
        MixedBridgeOutcome::NoteOn(result) => assert!(result.is_ok(), "{report:?}"),
        MixedBridgeOutcome::KeyUp { result, .. } => assert!(result.is_ok(), "{report:?}"),
        MixedBridgeOutcome::Bend { result, .. } => assert!(result.is_ok(), "{report:?}"),
        MixedBridgeOutcome::PedalDown { result, .. } => assert!(result.is_ok(), "{report:?}"),
        MixedBridgeOutcome::Lift { result, .. } => assert!(result.is_ok(), "{report:?}"),
        other => panic!("unrequested or refused sound operation: {other:?}"),
    }
}

/// The existing generic receipt audit searches every frontier in every receipt. Keep
/// this linear by comparing each source's ordered raw-admission/receipt streams; sound
/// messages remain sparse and use the existing pairing and ledger audit.
fn finish_sustained(rig: Rig, transcript: &FunctionalTranscript) -> Joined {
    assert_eq!(
        rig.callbacks.len(),
        TOTAL_CALLBACKS,
        "priming carry counts in warm-up"
    );
    assert!(
        rig.callbacks
            .iter()
            .all(|(_, frames, outcome)| *frames == Q && outcome.is_ok())
    );
    assert!(rig.raw_callbacks.iter().all(|running| *running));
    assert_eq!(rig.frontiers.len(), 2 * (TOTAL_CALLBACKS - 1));
    assert!(rig.frontiers.iter().all(|(_, result)| result.is_ok()));
    assert!(rig.raw_refused.is_empty() && rig.pre_ring.is_empty() && rig.halted_sends.is_empty());
    assert_eq!(rig.holds(), EventCount::NONE);
    assert_eq!(rig.messages().count(), transcript.messages.len());
    assert_eq!(rig.settled.len(), transcript.messages.len());
    let mut reset_key_ups = 0;
    let mut panic_key_ups = 0;
    for (packet, settled) in &rig.settled {
        match settled {
            Settled::Report(report) => {
                assert_applied_sound_outcome(*report);
                if matches!(
                    report.outcome(),
                    MixedBridgeOutcome::KeyUp {
                        result: Ok(MixedKeyUpRetired::AfterPanic(_)),
                        ..
                    }
                ) {
                    panic_key_ups += 1;
                }
            }
            Settled::Stage(Ok(MixedStaged::Split(MixedSplit::AfterRefusedOnset {
                reason: MixedOnsetRefusal::Reset,
                ..
            })))
            | Settled::Pump(MixedSplit::AfterRefusedOnset {
                reason: MixedOnsetRefusal::Reset,
                ..
            }) => reset_key_ups += 1,
            other => panic!("sound request did not execute: {packet:?} {other:?}"),
        }
    }
    assert_eq!(panic_key_ups, 4);
    assert_eq!(reset_key_ups, 4);
    let generations = [rig.generation(FIRST), rig.generation(SECOND)];
    let joined = rig.join();
    assert!(!joined.host_halted && !joined.raw_halted);
    assert!(joined.errors.is_empty() && joined.raw_errors.is_empty());
    assert!(joined.raw_packet_errors.is_empty() && joined.raw_render_error.is_none());
    assert!(joined.raw_collect_refusals.is_empty() && joined.raw_cancel_refusals.is_empty());
    assert!(joined.raw_receipt_errors.is_empty() && joined.raw_join_error.is_none());
    assert!(!joined.raw_unexpected_command);
    assert!(matches!(
        joined.raw_command_outcomes.as_slice(),
        [(id, LoopTransferOutcome::Command(receipt))]
            if *id == joined.raw_play_id
                && receipt.boundary.command == SessionCommand::Play
                && matches!(receipt.outcome, SessionOutcome::Applied { .. })
    ));
    assert!(joined.unexamined.is_empty() && joined.retired.is_empty());
    assert_eq!(joined.teardown.fault(), None);
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert!(joined.teardown.held().is_empty() && joined.teardown.open().is_empty());
    assert_eq!(
        joined.teardown.transfer().holds_outstanding(),
        EventCount::NONE
    );
    assert_eq!(joined.retired_reset.len(), 1);
    let old = &joined.retired_reset[0];
    assert_eq!(
        old.bridge().transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert_eq!(
        old.bridge().transfer().holds_outstanding(),
        EventCount::NONE
    );
    assert!(old.unclassified().is_empty() && old.unmatched().is_empty());
    joined.assert_sound_pairs_as_raw_links();
    for generation in generations {
        let admitted: Vec<_> = joined
            .admitted
            .iter()
            .filter(|item| item.packet.connection() == generation)
            .collect();
        let receipts: Vec<_> = joined
            .receipts
            .iter()
            .filter(|receipt| receipt.id.generation() == generation)
            .collect();
        assert_eq!(
            receipts.len(),
            admitted.len() + 1,
            "one initial start fence"
        );
        assert_eq!(receipts[0].id.serial(), 1);
        assert_eq!(receipts[0].observation, frontier(0));
        for (item, receipt) in admitted.iter().zip(&receipts[1..]) {
            let RawAdmission::Admitted { id, link } = item.raw else {
                panic!("packet was not raw-admitted: {item:?}");
            };
            assert_eq!(id, receipt.id);
            assert_eq!(item.observation, receipt.observation);
            assert_eq!(link, receipt.matched_onset);
            if item.is_message() {
                assert_eq!(
                    receipt.state,
                    RawState::Published(CaptureDisposition::Recorded)
                );
            }
        }
    }
    for (_, raw, _) in joined.ledger() {
        assert_eq!(raw, RawState::Published(CaptureDisposition::Recorded));
    }
    joined
}

fn sustained_run(hz: f32) -> FunctionalTranscript {
    let profile = sustained_profile(hz);
    let config = crate::frozen_capacity::reference_host(profile.capabilities().sample_rate())
        .expect("frozen reference");
    let (transcript, _joined, _callbacks) =
        sustained_run_with(config.profile, &|| config.setup(), false, "Inconclusive");
    transcript
}

fn sustained_run_with(
    profile: HostProfile,
    make_setup: &impl Fn() -> Setup,
    collect: bool,
    runtime_failure: &str,
) -> (
    FunctionalTranscript,
    Joined,
    Vec<callback_probe::Observation>,
) {
    let hz = profile.capabilities().sample_rate().as_f32();
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(make_setup(), prepared, candidate, profile);
    if collect {
        rig.callback_observations = Some(Vec::with_capacity(TOTAL_CALLBACKS));
    }
    let mut transcript = FunctionalTranscript {
        activations: Vec::new(),
        messages: Vec::new(),
        callbacks_with_four_holds: 0,
        stops: Vec::new(),
        starts: Vec::new(),
        panics: Vec::new(),
        resets: Vec::new(),
        pressure: Vec::new(),
    };
    let mut successor_port = None;
    rig.prime();
    // The priming callback is the first of the 1,024 warm-up callbacks.
    for callback in 0..TOTAL_CALLBACKS - 1 {
        let boundary = u64::try_from(callback).expect("bounded callback") * 64;
        let activate = callback >= 4
            && callback.is_multiple_of(4)
            && ![2_048, 6_000, 6_004].contains(&callback);
        let looped = callback.is_multiple_of(8) && callback != 6_008;
        let wrapped = matches!(
            rig.callback.as_ref().expect("callback").mixed.as_ref(),
            Some(MixedHostAudio::Reset(_))
        );
        let wraps_before = if wrapped {
            0
        } else {
            rig.direct_audio().compiled_loop().wraps().get()
        };
        if [2_048, 6_000].contains(&callback) {
            callback_probe::without_heap(|| rig.direct_audio().schedule_stop(at(boundary)))
                .expect("post-warmup compiled Stop");
            transcript.stops.push(at(boundary));
        }
        if callback == 6_002 {
            let host_callback = rig.callback.as_mut().expect("callback");
            let MixedHostAudio::Direct(audio, port) = host_callback.mixed.take().expect("direct")
            else {
                panic!("direct old owner");
            };
            let owner = MixedBridgeResetOwner::new(*audio, port).expect("old port");
            host_callback.mixed = Some(MixedHostAudio::Reset(Box::new(
                MixedBridgeResetAudio::new(owner),
            )));
            let reset_at = at(6_004 * 64);
            let (prepared, candidate) = saved_prepared_with_profile(reset_at, profile);
            let setup = make_setup();
            let MixedBridgeArmed {
                control,
                audio,
                control_port,
                audio_port,
            } = arm_prepared(&rig.raw, setup, prepared, candidate, profile);
            rig.control
                .seal_reset(control, reset_at)
                .expect("sustained reset seal");
            // Boxing is preparation while the complete callback is returned to
            // maintenance. Only the already-prepared owner's offer is audio work.
            let successor = MixedBridgeResetOwner::new(audio, audio_port).expect("successor");
            callback_probe::without_heap(|| {
                rig.reset_wrapper()
                    .offer_reset(successor, reset_at)
                    .expect("reset offer");
            });
            successor_port = Some(control_port);
        }
        if activate {
            let kind = if [2_052, 6_008].contains(&callback) {
                MixedActivationKind::Start
            } else {
                MixedActivationKind::Seek
            };
            let offer = if looped {
                rig.control.prepare_loop_activation(
                    at(boundary),
                    PlanPosition::ZERO,
                    LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(28))
                        .expect("28-frame loop"),
                    at(boundary),
                    kind,
                )
            } else {
                rig.control
                    .prepare_activation(at(boundary), PlanPosition::ZERO, at(boundary), kind)
            }
            .expect("sustained later activation prepares");
            callback_probe::without_heap(|| rig.direct_audio().offer_activation(offer))
                .expect("one empty offer slot");
        }
        if callback == 3_000 {
            assert_eq!(rig.holds(), EventCount::measured(4));
            rig.control
                .panic(at(boundary + 16))
                .expect("post-warmup full-span Panic");
        }
        for (source, frame, input) in source_requests(callback) {
            let observation = message(frame, input);
            rig.send(source, observation);
            transcript.messages.push((source, observation));
        }
        if callback == 4_003 {
            assert_eq!(rig.holds(), EventCount::measured(4));
            for source in [FIRST, SECOND] {
                assert_eq!(
                    rig.inboxes[source].occupied_len(),
                    18,
                    "sixteen ordinary bends and two protected releases"
                );
            }
            // Consume the full ordinary rings before offering their frontiers.
            // This is host staging, not an extra audio callback or a retry.
            rig.stage_all();
            transcript.pressure.push(at(boundary));
        }
        // These complete prefixes advance exactly once per 64-frame callback. Every
        // source request precedes its own frontier and both sources are healthy.
        for source in [FIRST, SECOND] {
            rig.send(source, frontier(boundary + 64));
        }
        rig.service();
        rig.render(Q).unwrap_or_else(|error| {
            panic!("{runtime_failure}: runtime fault at rate {hz}, callback {callback}: {error:?}")
        });
        rig.collect();
        if callback == 4_003 {
            assert_eq!(
                rig.direct_audio().compiled_loop().wraps().get() - wraps_before,
                3,
                "full ordinary pressure coincides with three actual wraps"
            );
        }
        assert!(
            !rig.raw_interrupted(),
            "raw halted at rate {hz}, callback {callback}"
        );
        assert!(
            rig.errors.is_empty(),
            "bridge errors at callback {callback}: {:?}",
            rig.errors
        );
        if callback == 3_000 {
            let (report, ended) = only_panic(&rig);
            assert_eq!(ended.len(), 4);
            assert_eq!(report.executed_at(), at(boundary + 16));
            transcript.panics.push(report);
        }
        if callback == 6_004 {
            let MixedBridgeResetState::Retired(receipt) = rig.reset_wrapper().state() else {
                panic!("one-quantum reset must retire");
            };
            assert_eq!(receipt.ended().count(), 4);
            let old = rig.control.commit_reset(receipt).expect("commit reset");
            rig.reset_old.push((
                old,
                std::mem::replace(
                    &mut rig.control_port,
                    successor_port.take().expect("successor port"),
                ),
            ));
            transcript.resets.push(receipt);
            rig.retire_reset();
            let host_callback = rig.callback.as_mut().expect("callback");
            let MixedHostAudio::Reset(wrapper) = host_callback.mixed.take().expect("wrapper")
            else {
                panic!("reset wrapper");
            };
            let (active, secondary) = (*wrapper).into_owners();
            assert!(secondary.is_none());
            let (audio, port) = active.into_parts();
            host_callback.mixed = Some(MixedHostAudio::Direct(Box::new(audio), port));
        }
        let expected_holds = match callback {
            0 | 3_000 | 3_001 | 4_003 | 6_004 | 6_005 | 11_020.. => 0,
            1_026 | 4_098 => 2,
            _ => 4,
        };
        if ![6_002, 6_003].contains(&callback) {
            assert_eq!(
                rig.holds().get(),
                expected_holds,
                "rate {hz}, callback {callback}"
            );
        }
        if activate {
            let receipt = collect_activation(&mut rig);
            assert_eq!(receipt.effective(), at(boundary));
            if [2_052, 6_008].contains(&callback) {
                assert!(!receipt.stopped());
                assert_eq!(receipt.released(), HeldNoteCount::NONE);
                transcript.starts.push(at(boundary));
            }
            let wraps_after = rig.direct_audio().compiled_loop().wraps().get();
            let wraps = wraps_after
                .checked_sub(wraps_before)
                .expect("cumulative wraps");
            if looped {
                assert!(
                    wraps >= 2,
                    "one 64-frame quantum must contain two 28-frame wraps"
                );
            }
            transcript.activations.push(ActivationEntry {
                boundary: at(boundary),
                looped,
                receipt,
                first_quantum_wraps: EventCount::measured(wraps),
            });
        }
        if ![6_002, 6_003].contains(&callback) && rig.holds() == EventCount::measured(4) {
            transcript.callbacks_with_four_holds += 1;
        }
    }
    assert_eq!(transcript.activations.len(), (TOTAL_CALLBACKS - 2) / 4 - 3);
    assert_eq!(transcript.stops.len(), 2);
    assert_eq!(transcript.starts.len(), 2);
    assert_eq!(transcript.panics.len(), 1);
    assert_eq!(transcript.resets.len(), 1);
    assert_eq!(transcript.pressure, [at(4_003 * 64)]);
    assert!(transcript.callbacks_with_four_holds > EXERCISE_CALLBACKS / 2);
    // Read the retained operation entries as well as asserting at execution time.
    for entry in &transcript.activations {
        assert_eq!(entry.receipt.effective(), entry.boundary);
        if entry.looped {
            assert!(entry.first_quantum_wraps.get() >= 2);
        }
    }
    let callbacks = rig.callback_observations.take().unwrap_or_default();
    let joined = finish_sustained(rig, &transcript);
    (transcript, joined, callbacks)
}

#[test]
fn sustained_saved_graph_and_two_real_sources_complete_fixed_callbacks_at_all_four_rates() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let _operations = sustained_run(hz);
    }
}
