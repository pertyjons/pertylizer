//! Attribution controls through the saved graph and the concrete two-source host.
//! These assert small literal charges; they do not collect workload capacity data.
use super::*;
use synth_engine_v2::publish::ProducerClass;

fn single_quantum(rig: &mut Rig, boundary: u64) -> [Option<EventCount>; 6] {
    for source in [FIRST, SECOND] {
        rig.send(source, frontier(boundary + 64));
    }
    rig.step();
    let (start, charges) = rig
        .direct_audio()
        .publication_usage()
        .last_quantum()
        .expect("one destination quantum");
    assert_eq!(start, at(boundary));
    charges
}

fn assert_only(charges: [Option<EventCount>; 6], expected: Option<ProducerClass>) {
    for class in ProducerClass::ALL {
        let expected = match class {
            ProducerClass::AuthoredRuntime | ProducerClass::Internal => None,
            _ => Some(EventCount::measured(u32::from(Some(class) == expected))),
        };
        assert_eq!(charges[class.index()], expected, "{class}");
    }
}

#[test]
fn empty_saved_graph_sources_have_no_producer_charges_before_initial_activation() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        empty_before_adoption(sustained_host::sustained_profile(hz), &default_setup);
    }
}

fn empty_before_adoption(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    // Candidate preparation is not producer occupancy. Its future adoption is
    // deliberately outside this empty control; raw Play has its own arbiter.
    let (prepared, candidate) = saved_prepared_schedule(at(4_096), profile, SavedSchedule::Empty);
    let mut rig = Rig::with_prepared(make_setup(), prepared, candidate, profile);
    rig.prime();
    for boundary in (0..256).step_by(64) {
        assert_only(single_quantum(&mut rig, boundary), None);
    }
    let joined = rig.join();
    assert_eq!(joined.teardown.fault(), None);
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

#[test]
fn empty_saved_graph_adoption_has_only_its_literal_session_restoration_charge() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        empty_adoption(sustained_host::sustained_profile(hz), &default_setup);
    }
}

fn empty_adoption(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    let (prepared, candidate) = saved_prepared_schedule(at(64), profile, SavedSchedule::Empty);
    let mut rig = Rig::with_prepared(make_setup(), prepared, candidate, profile);
    rig.prime();
    assert_only(single_quantum(&mut rig, 0), None);
    let charges = single_quantum(&mut rig, 64);
    // Sixteen scoped restoration groups, plus the adoption's mass-release
    // operation. Empty input does not eliminate mandatory transport work.
    assert_eq!(
        charges[ProducerClass::Session.index()],
        Some(EventCount::measured(17))
    );
    let mut without_session = charges;
    without_session[ProducerClass::Session.index()] = Some(EventCount::NONE);
    assert_only(without_session, None);
    assert_only(single_quantum(&mut rig, 128), None);
    let joined = rig.join();
    assert_eq!(joined.teardown.fault(), None);
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

#[test]
fn each_executable_producer_has_independent_one_unit_attribution() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        one_unit(sustained_host::sustained_profile(hz), &default_setup);
    }
}

fn one_unit(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    let (prepared, candidate) = saved_prepared_schedule(
        at(64),
        profile,
        SavedSchedule::SingleOnset(PlanPosition::new(256)),
    );
    let mut rig = Rig::with_prepared(make_setup(), prepared, candidate, profile);
    rig.prime();
    assert_only(single_quantum(&mut rig, 0), None);
    let _initial_adoption = single_quantum(&mut rig, 64);
    // No inherited bend: exactly one live onset and one later exact key-up.
    rig.send(FIRST, message(136, note_on(60)));
    assert_only(single_quantum(&mut rig, 128), Some(ProducerClass::Live));
    rig.send(FIRST, message(200, note_off(60)));
    assert_only(single_quantum(&mut rig, 192), Some(ProducerClass::Release));
    assert_only(single_quantum(&mut rig, 256), None);
    // Adoption at sample 64 translates plan position 256 to sample 320.
    assert_only(single_quantum(&mut rig, 320), Some(ProducerClass::Compiled));
    rig.direct_audio().schedule_stop(at(384)).expect("Stop");
    assert_only(single_quantum(&mut rig, 384), Some(ProducerClass::Session));
    assert_eq!(rig.holds(), EventCount::NONE);
    let joined = rig.join();
    assert_eq!(joined.teardown.fault(), None);
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

#[derive(Clone, Copy)]
enum PeerDelivery {
    KnownPrefix,
    MaximumCadence,
    Silent,
}

fn fence_onset(
    profile: HostProfile,
    make_setup: &impl Fn() -> Setup,
    source: usize,
    delivery: PeerDelivery,
) -> MixedBridgeReport {
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(make_setup(), prepared, candidate, profile);
    rig.prime();
    for boundary in [0, 64] {
        for source in [FIRST, SECOND] {
            rig.send(source, frontier(boundary + 64));
        }
        rig.step();
    }
    assert_eq!(rig.control.audio_boundary(), at(128));
    // The first possible packet after the peer's last complete prefix. The
    // baseline knows the entire next prefix; the healthy case publishes at its
    // maximum legal cadence. Neither should cross the strict fence threshold.
    rig.send(source, message(129, note_on(60)));
    rig.send(source, frontier(192));
    let peer = 1 - source;
    match delivery {
        PeerDelivery::KnownPrefix => rig.send(peer, frontier(512)),
        PeerDelivery::MaximumCadence => rig.send(peer, frontier(192)),
        PeerDelivery::Silent => {}
    }
    rig.service();
    let packet = rig.packet_of(source, 129);
    if matches!(delivery, PeerDelivery::Silent) {
        assert!(rig.forwarded.is_empty());
        assert_eq!(
            rig.waits.last(),
            Some(&MixedBridgeWait::Peer {
                source: rig.generation(peer),
            })
        );
        rig.render(Q).expect("boundary 128");
        rig.service();
        assert_eq!(rig.control.audio_boundary(), at(192));
        assert!(
            rig.forwarded.is_empty(),
            "boundary equal to F + W is not fenced"
        );
        rig.render(Q).expect("boundary 192");
        rig.service();
        assert_eq!(rig.control.audio_boundary(), at(256));
        assert_eq!(
            rig.forwarded.len(),
            1,
            "first boundary strictly beyond F + W"
        );
    }
    rig.render(Q).expect("onset callback");
    rig.collect();
    let report = rig.report_of(packet).expect("executed onset");
    assert!(matches!(
        report.outcome(),
        MixedBridgeOutcome::NoteOn(Ok(_))
    ));
    assert_eq!(report.nominal_at(), at(129));
    let expected = if matches!(delivery, PeerDelivery::Silent) {
        256
    } else {
        129
    };
    assert_eq!(report.executed_at(), at(expected));
    assert!(rig.errors.is_empty() && !rig.raw_interrupted());
    // Release and advance both sources before join, preserving raw FIFO and all
    // earlier observations. The late peer publishes a future frontier, no packet.
    let cleanup = rig.control.audio_boundary().as_u64() + 8;
    rig.send(source, message(cleanup, note_off(60)));
    for source in [FIRST, SECOND] {
        if source != peer || !matches!(delivery, PeerDelivery::KnownPrefix) {
            rig.send(source, frontier(cleanup + 64));
        }
    }
    rig.step();
    assert_eq!(rig.holds(), EventCount::NONE);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(joined.teardown.fault(), None);
    report
}

#[test]
fn candidate_fence_respects_adjacent_boundaries_and_five_milliseconds_at_every_rate() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        fence_for(sustained_host::sustained_profile(hz), &default_setup);
    }
}

fn fence_for(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    let hz = profile.capabilities().sample_rate().as_f32();
    for source in [FIRST, SECOND] {
        let direct = fence_onset(profile, make_setup, source, PeerDelivery::KnownPrefix);
        let healthy = fence_onset(profile, make_setup, source, PeerDelivery::MaximumCadence);
        let silent = fence_onset(profile, make_setup, source, PeerDelivery::Silent);
        assert_eq!(healthy.executed_at(), direct.executed_at());
        let added = silent.executed_at().as_u64() - direct.executed_at().as_u64();
        assert_eq!(added, 127);
        // Keep the additional callback handoff in the conservative bound,
        // although this host forwards at the first eligible boundary.
        let conservative_frames = added + 64;
        assert!(
            f64::from(u32::try_from(conservative_frames).expect("small bound")) / f64::from(hz)
                <= 0.005
        );
    }
}

fn default_setup() -> Setup {
    Setup {
        peer_wait: 64,
        ..Setup::default()
    }
}

/// Re-execute attribution and fence controls with the exact reference/selected
/// profile and queue configuration; success under defaults is not transferred.
pub(super) fn controls(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    empty_before_adoption(profile, make_setup);
    empty_adoption(profile, make_setup);
    one_unit(profile, make_setup);
    fence_for(profile, make_setup);
    super::capacity_boundaries::run(profile, make_setup);
    pressure_partitions(profile, make_setup);
}

fn pressure_run(
    profile: HostProfile,
    make_setup: &impl Fn() -> Setup,
    partition: &[usize],
) -> (Vec<u32>, Vec<String>, Vec<String>) {
    assert_eq!(partition.iter().sum::<usize>(), 64);
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(make_setup(), prepared, candidate, profile);
    rig.prime();
    for boundary in [0, 64] {
        for source in [FIRST, SECOND] {
            rig.send(source, frontier(boundary + 64));
        }
        rig.step();
    }
    for (source, keys) in [(FIRST, [60, 62]), (SECOND, [64, 67])] {
        for (offset, key) in keys.into_iter().enumerate() {
            rig.send(
                source,
                message(136 + u64::try_from(offset).expect("two keys"), note_on(key)),
            );
        }
        rig.send(source, frontier(192));
    }
    rig.step();
    assert_eq!(rig.holds(), EventCount::measured(4));
    for (source, keys) in [(FIRST, [60, 62]), (SECOND, [64, 67])] {
        for index in 0..16_u16 {
            rig.send(source, message(200 + u64::from(index), bend(8_193 + index)));
        }
        assert_eq!(
            rig.inboxes[source].occupied_len(),
            16,
            "ordinary ring saturated"
        );
        for (offset, key) in keys.into_iter().enumerate() {
            rig.send(
                source,
                message(
                    216 + u64::try_from(offset).expect("two keys"),
                    note_off(key),
                ),
            );
        }
        assert_eq!(
            rig.inboxes[source].occupied_len(),
            18,
            "protected releases retained"
        );
    }
    rig.stage_all();
    for source in [FIRST, SECOND] {
        rig.send(source, frontier(256));
    }
    rig.service();
    for &frames in partition {
        rig.render(frames).expect("pressure callback subdivision");
    }
    rig.collect();
    assert_eq!(rig.holds(), EventCount::NONE);
    assert!(rig.retries.iter().all(Option::is_none));
    assert!(rig.errors.is_empty());
    let messages: Vec<_> = rig.messages().map(|item| item.packet).collect();
    for packet in messages {
        let report = rig.report_of(packet).expect("every sound request executed");
        match report.outcome() {
            MixedBridgeOutcome::NoteOn(result) => assert!(result.is_ok()),
            MixedBridgeOutcome::Bend { result, .. } => assert!(result.is_ok()),
            MixedBridgeOutcome::KeyUp { result, .. } => assert!(result.is_ok()),
            other => panic!("unexpected pressure outcome: {other:?}"),
        }
    }
    finish(rig)
}

fn pressure_partitions(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    let whole = pressure_run(profile, make_setup, &[64]);
    for partition in [&[8; 8][..], &[7, 13, 1, 43][..]] {
        assert_eq!(pressure_run(profile, make_setup, partition), whole);
    }
}
