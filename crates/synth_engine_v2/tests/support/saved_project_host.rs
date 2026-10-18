//! Executable EVD-0026 readiness controls, using the checked saved-project transcription.
//! These are functional controls, not a capacity selection or callback timing result.

use super::*;
use sha2::{Digest, Sha256};

#[path = "autonomous_prefix.rs"]
mod autonomous_prefix;

#[path = "concurrent_saved_host.rs"]
mod concurrent_saved_host;

#[path = "sustained_host.rs"]
mod sustained_host;

#[path = "capacity_controls.rs"]
mod capacity_controls;

#[path = "capacity_boundaries.rs"]
mod capacity_boundaries;

#[test]
fn frozen_reference_profiles_open_sound_and_raw_controls_before_collection() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let config = crate::frozen_capacity::reference_host(
            synth_engine_v2::quantities::SampleRate::new(hz).expect("candidate rate"),
        )
        .expect("reference profile");
        capacity_controls::controls(config.profile, &|| config.setup());
    }
}

#[test]
fn saved_project_host_pins_the_same_corpus_bytes_as_the_lowering_control() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(instrument_inserts::SOURCE);
    let bytes = std::fs::read(source).expect("saved project");
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(digest, instrument_inserts::SOURCE_SHA256);
}

#[test]
fn saved_stereo_graph_without_explicit_mono_fold_refuses_narrowing() {
    let ir = instrument_inserts::ir(Some(instrument_inserts::live()));
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = HostProfile::harness(
            synth_engine_v2::quantities::SampleRate::new(hz).expect("candidate rate"),
            FrameCount::new(64),
            ChannelLayout::Mono,
        )
        .expect("mono profile");
        let error = compile(&ir, &RenderConfig::new(profile))
            .into_plan()
            .expect_err("mono graph refuses");
        assert!(
            matches!(
                error,
                synth_engine_v2::diagnostics::CompileError::LayoutMismatch {
                    source_node: instrument_inserts::MASTER_METER,
                    target_node: instrument_inserts::MASTER_OUTPUT,
                    source_layout: ChannelLayout::Stereo,
                    target_layout: ChannelLayout::Mono,
                    ..
                }
            ),
            "{error:?}"
        );
    }
}

fn saved_prepared_with_profile(
    requested: SampleTime,
    profile: HostProfile,
) -> (MixedJoinedPrepared, MixedStampedCandidate) {
    saved_prepared_schedule(requested, profile, SavedSchedule::Original)
}

enum SavedSchedule {
    Original,
    Empty,
    SingleOnset(PlanPosition),
}

fn saved_prepared_schedule(
    requested: SampleTime,
    profile: HostProfile,
    schedule: SavedSchedule,
) -> (MixedJoinedPrepared, MixedStampedCandidate) {
    let plan = common::admit(
        &instrument_inserts::ir_with_layout(
            Some(instrument_inserts::live()),
            profile.capabilities().channel_layout(),
        ),
        profile,
    );
    let slot = plan
        .resolve_note(instrument_inserts::ENVELOPE)
        .expect("saved envelope");
    let original = instrument_inserts::events_at(slot, profile.capabilities().sample_rate());
    let events: Vec<_> = match schedule {
        SavedSchedule::Original => original
            .into_iter()
            .map(|(frame, payload)| PlanEvent::new(PlanPosition::new(frame), payload))
            .collect(),
        SavedSchedule::Empty => Vec::new(),
        SavedSchedule::SingleOnset(position) => vec![PlanEvent::new(
            position,
            original.first().expect("first compiled onset").1,
        )],
    };
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("saved schedule");
    let binding = MixedTargetAdmission::admit(plan, stream, slot).expect("saved mixed partition");
    let prepared = MixedJoinedStream::open(
        binding,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("saved owner")
    .prepare_initial()
    .expect("saved initial schedule");
    let history = prepared
        .prepare_history(requested, PlanPosition::ZERO)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("candidate");
    (prepared, candidate)
}

fn candidate_profile(hz: f32) -> HostProfile {
    HostProfile::harness(
        synth_engine_v2::quantities::SampleRate::new(hz).expect("candidate rate"),
        FrameCount::new(BLOCK),
        ChannelLayout::Stereo,
    )
    .expect("candidate profile")
}

fn collect_activation(rig: &mut Rig) -> MixedActivationAdopted {
    let retired = callback_probe::without_heap(|| rig.direct_audio().take_retired_activation())
        .expect("retired activation");
    let receipt = rig
        .control
        .collect_activation(retired)
        .expect("collected activation");
    match receipt.outcome() {
        MixedActivationOutcome::Adopted(adopted) => adopted,
        other => panic!("saved activation failed: {other:?}"),
    }
}

fn finish(rig: Rig) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let audio = rig.rendered.iter().map(|sample| sample.to_bits()).collect();
    let reports = normalized_reports(&rig);
    let joined = rig.join();
    joined.assert_each_packet_once();
    joined.assert_sound_pairs_as_raw_links();
    assert!(!joined.host_halted && !joined.raw_halted);
    assert!(joined.errors.is_empty(), "{:?}", joined.errors);
    assert_eq!(joined.teardown.fault(), None);
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert_eq!(
        joined.teardown.transfer().holds_outstanding(),
        EventCount::NONE
    );
    assert!(joined.teardown.held().is_empty() && joined.teardown.open().is_empty());
    for (packet, state, _) in joined.ledger() {
        assert_eq!(
            state,
            RawState::Published(CaptureDisposition::Recorded),
            "{packet:?}"
        );
    }
    (
        audio,
        raw_trace(&joined.admitted, &joined.receipts),
        reports,
    )
}

/// The exact saved schedule runs beyond its last edge; both independent sources play
/// through its filter, velocity, balance, distortion, delay and master stages.
fn saved_timeline(
    partition: &[usize],
    live: bool,
    profile: HostProfile,
) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(Setup::default(), prepared, candidate, profile);
    rig.prime();
    if live {
        rig.send_all(
            FIRST,
            &[
                message(100, note_on(60)),
                message(120, sustain(true)),
                frontier(200),
            ],
        );
        rig.send_all(SECOND, &[message(110, note_on(64)), frontier(200)]);
        rig.service();
    } else {
        rig.send(FIRST, frontier(200));
        rig.send(SECOND, frontier(200));
        rig.service();
    }
    let mut next = 0;
    advance(&mut rig, at(16_384), partition, &mut next);
    if live {
        rig.send_all(
            FIRST,
            &[
                message(16_400, note_off(60)),
                message(16_410, sustain(false)),
                frontier(16_500),
            ],
        );
        rig.send_all(SECOND, &[message(16_420, note_off(64)), frontier(16_500)]);
        rig.service();
    }
    // Beyond the last release at every candidate rate, including 216,000 at 192 kHz.
    advance(&mut rig, at(240_064), partition, &mut next);
    assert_eq!(rig.holds(), EventCount::NONE);
    rig.control.panic(at(240_128)).expect("post-timeline Panic");
    rig.send(FIRST, frontier(240_256));
    rig.send(SECOND, frontier(240_256));
    rig.service();
    advance(&mut rig, at(240_192), partition, &mut next);
    assert_eq!(
        rig.direct_audio().panic_state().compiled_ended(),
        HeldNoteCount::NONE,
        "all compiled releases must have executed before the final Panic"
    );
    finish(rig)
}

#[test]
fn saved_project_timeline_and_two_source_capture_agree_in_four_partitions() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = candidate_profile(hz);
        for live in [false, true] {
            let reference = saved_timeline(&[Q], live, profile);
            assert!(audible(&reference.0));
            for partition in [&[1_024][..], &[8], &[37, 91, 13, 5, 110]] {
                assert_eq!(saved_timeline(partition, live, profile), reference);
            }
        }
        assert_ne!(
            saved_timeline(&[Q], true, profile).0,
            saved_timeline(&[Q], false, profile).0
        );
    }
}

fn saved_transport(
    partition: &[usize],
    transport: bool,
    profile: HostProfile,
) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(Setup::default(), prepared, candidate, profile);
    rig.prime();
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(101, note_on(62)),
            message(120, sustain(true)),
            message(130, bend(9_000)),
            frontier(150),
        ],
    );
    rig.send_all(
        SECOND,
        &[
            message(110, note_on(64)),
            message(111, note_on(67)),
            message(140, bend(8_000)),
            frontier(150),
        ],
    );
    rig.service();
    let mut next = 0;
    advance(&mut rig, at(192), partition, &mut next);
    assert_eq!(
        rig.holds(),
        EventCount::measured(4),
        "the full live span is exercised"
    );
    // Every fourth quantum: seek, short multi-wrap loop, seek out, Stop and Start.
    for (boundary, looped) in [(256, false), (512, true), (768, false)] {
        if transport {
            let offer = if looped {
                rig.control.prepare_loop_activation(
                    at(boundary),
                    PlanPosition::ZERO,
                    LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(28)).expect("loop"),
                    at(boundary),
                    MixedActivationKind::Seek,
                )
            } else {
                rig.control.prepare_activation(
                    at(boundary),
                    PlanPosition::ZERO,
                    at(boundary),
                    MixedActivationKind::Seek,
                )
            }
            .expect("saved transport prepares");
            rig.direct_audio()
                .offer_activation(offer)
                .expect("saved transport offered");
        }
        rig.send(FIRST, frontier(boundary + 150));
        rig.send(SECOND, frontier(boundary + 150));
        rig.service();
        if transport && looped {
            let before = rig.direct_audio().compiled_loop().wraps().get();
            advance(&mut rig, at(boundary + 64), partition, &mut next);
            let after = rig.direct_audio().compiled_loop().wraps().get();
            assert_eq!(
                after.checked_sub(before),
                Some(2),
                "two wraps inside the entry quantum"
            );
        }
        advance(&mut rig, at(boundary + 128), partition, &mut next);
        if transport {
            let adopted = collect_activation(&mut rig);
            assert_eq!(adopted.effective(), at(boundary));
            assert_eq!(adopted.released(), HeldNoteCount::measured(2));
            if looped {
                assert!(rig.direct_audio().compiled_loop().wraps().get() >= 2);
            }
        }
    }
    if transport {
        rig.direct_audio()
            .schedule_stop(at(1_024))
            .expect("saved Stop");
    }
    rig.send_all(
        FIRST,
        &[
            message(1_000, note_off(60)),
            message(1_001, note_off(62)),
            frontier(1_100),
        ],
    );
    rig.send(SECOND, frontier(1_100));
    rig.service();
    advance(&mut rig, at(1_152), partition, &mut next);
    if transport {
        let start = rig
            .control
            .prepare_activation(
                at(1_280),
                PlanPosition::ZERO,
                at(1_280),
                MixedActivationKind::Start,
            )
            .expect("Start");
        rig.direct_audio()
            .offer_activation(start)
            .expect("Start offer");
    }
    rig.send_all(FIRST, &[message(1_350, sustain(false)), frontier(1_400)]);
    rig.send_all(
        SECOND,
        &[
            message(1_360, note_off(64)),
            message(1_361, note_off(67)),
            frontier(1_400),
        ],
    );
    rig.service();
    advance(&mut rig, at(1_408), partition, &mut next);
    if transport {
        assert!(!collect_activation(&mut rig).stopped());
        rig.control.panic(at(1_472)).expect("saved Panic");
    }
    rig.send(FIRST, frontier(1_600));
    rig.send(SECOND, frontier(1_600));
    rig.service();
    advance(&mut rig, at(1_664), partition, &mut next);
    if transport {
        let (_, ended) = only_panic(&rig);
        assert!(ended.is_empty(), "both live notes were already released");
        assert_eq!(
            rig.direct_audio().panic_state().compiled_ended(),
            HeldNoteCount::measured(2)
        );
    }
    finish(rig)
}

fn saved_panic(
    partition: &[usize],
    panic: bool,
    profile: HostProfile,
) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(Setup::default(), prepared, candidate, profile);
    rig.prime();
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(101, note_on(62)),
            message(120, sustain(true)),
            message(130, note_off(60)),
            frontier(150),
        ],
    );
    rig.send_all(
        SECOND,
        &[
            message(110, note_on(64)),
            message(111, note_on(67)),
            frontier(150),
        ],
    );
    rig.service();
    let mut next = 0;
    advance(&mut rig, at(192), partition, &mut next);
    assert_eq!(rig.holds(), EventCount::measured(4));
    if panic {
        rig.control.panic(at(256)).expect("full-span Panic");
    }
    rig.send(FIRST, frontier(350));
    rig.send(SECOND, frontier(350));
    rig.service();
    advance(&mut rig, at(384), partition, &mut next);
    if panic {
        let (report, ended) = only_panic(&rig);
        assert_eq!(report.executed_at(), at(256));
        assert_eq!(
            ended.len(),
            4,
            "Panic ends every held or sustained live note"
        );
        assert_eq!(
            rig.direct_audio().panic_state().compiled_ended(),
            HeldNoteCount::measured(2)
        );
        assert_eq!(rig.holds(), EventCount::NONE);
    }
    // Every later key-up and lift must still settle, without reaching a newer note.
    rig.send_all(
        FIRST,
        &[
            message(400, note_off(62)),
            message(410, sustain(false)),
            frontier(500),
        ],
    );
    rig.send_all(
        SECOND,
        &[
            message(420, note_off(64)),
            message(421, note_off(67)),
            frontier(500),
        ],
    );
    rig.service();
    advance(&mut rig, at(576), partition, &mut next);
    finish(rig)
}

#[test]
fn saved_project_panic_ends_the_full_live_hold_set_and_compiled_notes_in_stereo() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = candidate_profile(hz);
        let reference = saved_panic(&[Q], true, profile);
        for partition in [&[1_024][..], &[8], &[37, 91, 13, 5, 110]] {
            assert_eq!(saved_panic(partition, true, profile), reference);
        }
        assert_eq!(
            saved_panic(&[Q], false, profile).1,
            reference.1,
            "Panic cannot change raw capture"
        );
    }
}

#[test]
fn saved_project_repeated_transport_multiwrap_and_panic_keep_two_source_capture() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = candidate_profile(hz);
        let reference = saved_transport(&[Q], true, profile);
        assert!(audible(&reference.0));
        for partition in [&[1_024][..], &[8], &[37, 91, 13, 5, 110]] {
            assert_eq!(saved_transport(partition, true, profile), reference);
        }
        assert_eq!(
            saved_transport(&[Q], false, profile).1,
            reference.1,
            "transport cannot alter raw capture"
        );
    }
}

fn saved_reset(
    partition: &[usize],
    reset: bool,
    profile: HostProfile,
) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(Setup::default(), prepared, candidate, profile);
    rig.wrap_for_reset(at(192));
    rig.prime();
    rig.send_all(FIRST, &[message(100, note_on(60)), frontier(150)]);
    rig.send_all(
        SECOND,
        &[
            message(110, sustain(true)),
            message(120, note_on(64)),
            message(130, note_off(64)),
            frontier(150),
        ],
    );
    rig.service();
    let mut next = 0;
    serve_to(&mut rig, at(320), partition, &mut next);
    let mut successor_port = None;
    if reset {
        let (prepared, candidate) = saved_prepared_with_profile(at(384), profile);
        let MixedBridgeArmed {
            control,
            audio,
            control_port,
            audio_port,
        } = arm_prepared(&rig.raw, Setup::default(), prepared, candidate, profile);
        rig.control
            .seal_reset(control, at(384))
            .expect("saved reset seal");
        rig.reset_wrapper()
            .offer_reset(
                MixedBridgeResetOwner::new(audio, audio_port).expect("saved successor"),
                at(384),
            )
            .expect("saved reset offer");
        successor_port = Some(control_port);
    }
    rig.send(FIRST, frontier(370));
    rig.send(SECOND, frontier(370));
    serve_to(&mut rig, at(392), partition, &mut next);
    if reset {
        let MixedBridgeResetState::Fading { receipt, .. } = rig.reset_wrapper().state() else {
            panic!("saved owner reset did not apply");
        };
        assert_eq!(
            receipt.ended().count(),
            2,
            "reset owns held and sustained live notes"
        );
        let old = rig
            .control
            .commit_reset(receipt)
            .expect("saved reset commit");
        rig.reset_old.push((
            old,
            std::mem::replace(
                &mut rig.control_port,
                successor_port.take().expect("successor port"),
            ),
        ));
    }
    rig.send_all(FIRST, &[message(500, note_off(60)), frontier(550)]);
    rig.send_all(SECOND, &[message(510, sustain(false)), frontier(550)]);
    serve_to(&mut rig, at(576), partition, &mut next);
    if reset {
        rig.retire_reset();
        assert_eq!(rig.retired_resets.len(), 1);
        let old = &rig.retired_resets[0];
        assert_eq!(
            old.bridge().transfer().custody(),
            MixedTransferCustody::Complete
        );
        assert_eq!(
            old.bridge().transfer().holds_outstanding(),
            EventCount::NONE
        );
        assert!(old.unclassified().is_empty() && old.unmatched().is_empty());
        let start = rig
            .control
            .prepare_activation(
                at(640),
                PlanPosition::ZERO,
                at(640),
                MixedActivationKind::Start,
            )
            .expect("successor Start");
        rig.reset_wrapper()
            .offer_activation(start)
            .expect("successor Start offered");
    }
    rig.send_all(
        FIRST,
        &[
            message(700, note_on(62)),
            message(800, note_off(62)),
            frontier(900),
        ],
    );
    rig.send(SECOND, frontier(900));
    serve_to(&mut rig, at(960), partition, &mut next);
    if reset {
        let retired = rig
            .reset_wrapper()
            .take_retired_activation()
            .expect("successor Start receipt");
        let receipt = rig
            .control
            .collect_activation(retired)
            .expect("collected successor Start");
        assert!(
            matches!(receipt.outcome(), MixedActivationOutcome::Adopted(adopted) if !adopted.stopped())
        );
    }
    finish(rig)
}

#[test]
fn saved_project_stopped_reset_preserves_both_source_captures_and_joins_old_custody() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = candidate_profile(hz);
        let reference = saved_reset(&[Q], true, profile);
        for partition in [&[1_024][..], &[8], &[37, 91, 13, 5, 110]] {
            assert_eq!(saved_reset(partition, true, profile), reference);
        }
        assert_eq!(
            saved_reset(&[Q], false, profile).1,
            reference.1,
            "reset cannot change raw capture"
        );
    }
}
