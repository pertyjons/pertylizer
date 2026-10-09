//! Handle, ledger and table tests for the private mixed owner's live occurrences.

use super::{
    super::{
        MixedCollection, MixedIngressOriginId, MixedIngressOutcome, MixedIngressRequest,
        MixedIngressResult, MixedOneShotArmError, MixedOneShotAudio, MixedOneShotControl,
        history_tests,
    },
    CellUse, LiveOccurrenceCell, LiveOccurrenceGeneration, LiveOccurrenceId, MixedHandleRefused,
    MixedOccurrenceState, MixedReleaseSubmitError, MixedRetireCause, MixedRetired,
};
use crate::{
    identity::{NoteIdentity, Resolution},
    ingress::{ExhaustedResource, IngressRefused},
    quantities::{ChannelLayout, EventCount, KeyIdentity, NoteVelocity},
    render::{AudioBlockMut, EventPayload, NoteEdge},
    time::SampleTime,
};

const ORIGIN: MixedIngressOriginId = MixedIngressOriginId(7);

fn armed(compiled_first: bool) -> (MixedOneShotControl, MixedOneShotAudio) {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(compiled_first);
    prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .expect("private arm")
}

#[test]
fn a_one_slot_ingress_profile_refuses_arm_and_preserves_both_inputs() {
    use crate::profile::{EventLimits, HostProfile, QueueCapacities, RenderLimits};

    let original = history_tests::mixed_profile();
    let limits = original.limits();
    let events = limits.events();
    let queues = events.queues();
    let queues = QueueCapacities::new(
        EventCount::measured(1),
        queues.command_queue_capacity(),
        queues.event_egress_capacity(),
    )
    .expect("valid one-slot queue");
    let events = EventLimits::new(
        events.max_events_per_quantum(),
        events.max_note_expansion_per_tick(),
        events.max_scheduled_events_in_flight(),
        events.forward_event_horizon(),
        queues,
        events.shares(),
    )
    .expect("valid one-slot event profile");
    let limits = RenderLimits::new(
        limits.stream(),
        limits.graph(),
        limits.voices(),
        events,
        limits.observation(),
        limits.mixing(),
        limits.memory(),
        limits.script(),
        limits.recording(),
        limits.cost(),
    )
    .expect("valid limits");
    let profile = HostProfile::new(original.capabilities(), limits).expect("valid profile");
    let (prepared, candidate) = history_tests::one_shot_with_one_live_hold();
    let refused = prepared
        .arm_one_shot(candidate, &profile)
        .expect_err("one slot cannot reserve onset plus release");
    assert_eq!(
        refused.reason,
        MixedOneShotArmError::OccurrenceCapacity { capacity: 1 }
    );
    let _armed = refused
        .owner
        .arm_one_shot(refused.candidate, &original)
        .expect("both inputs survive the refusal");
}

fn key(raw: u8) -> KeyIdentity {
    KeyIdentity::new(raw).expect("valid key")
}

fn onset(control: &mut MixedOneShotControl, at: u64, raw: u8) -> LiveOccurrenceId {
    let (_, request) = control
        .submit_onset(
            ORIGIN,
            super::super::TEST_SCOPE,
            SampleTime::new(at),
            key(raw),
            NoteVelocity::FULL,
        )
        .expect("onset and its release place fit");
    let MixedIngressRequest::Onset { occurrence, .. } = request else {
        panic!("an onset submit returns its onset request");
    };
    occurrence
}

fn release(control: &mut MixedOneShotControl, occurrence: LiveOccurrenceId, at: u64) {
    let _submitted = control
        .submit_release(ORIGIN, occurrence, SampleTime::new(at))
        .expect("an open handle's release fits its reserved place");
}

/// Push a request the merger never minted, so it reaches audio's own handle checks.
fn forge(control: &mut MixedOneShotControl, request: MixedIngressRequest) {
    let _command = control
        .submit_ingress(request)
        .expect("forged command fits");
}

/// Serve the command ring on another thread and require zero allocator events.
fn service_without_allocation(audio: MixedOneShotAudio) -> MixedOneShotAudio {
    std::thread::spawn(move || {
        let mut audio = audio;
        assert_eq!(
            crate::render_allocation::count_allocs(|| audio.service_test_ingress_queue()),
            0,
            "live command service allocated"
        );
        audio
    })
    .join()
    .expect("service thread")
}

fn collect_all(control: &mut MixedOneShotControl) -> Vec<MixedIngressResult> {
    std::iter::from_fn(|| control.collect_ingress_result()).collect()
}

fn outcome_for(
    results: &[MixedIngressResult],
    request: MixedIngressRequest,
) -> MixedIngressOutcome {
    let mut matching = results.iter().filter(|result| result.request == request);
    let result = matching.next().expect("the request has a result");
    assert!(matching.next().is_none(), "one result per request");
    result.outcome
}

fn onset_request(occurrence: LiveOccurrenceId, at: u64, raw: u8) -> MixedIngressRequest {
    MixedIngressRequest::Onset {
        origin: ORIGIN,
        occurrence,
        scope: super::super::TEST_SCOPE,
        at: SampleTime::new(at),
        key: key(raw),
        velocity: NoteVelocity::FULL,
    }
}

fn release_request(occurrence: LiveOccurrenceId, at: u64) -> MixedIngressRequest {
    MixedIngressRequest::Release {
        origin: ORIGIN,
        occurrence,
        at: SampleTime::new(at),
    }
}

/// Note edges still waiting in the registered ingress queue, in queue order.
fn queued_edges(audio: &MixedOneShotAudio) -> Vec<(NoteIdentity, bool)> {
    let mut queue = vec![None; audio.test_ingress.queue_capacity_for_test()];
    let len = audio
        .test_ingress
        .copy_queue_for_test(&mut queue)
        .expect("journal copy");
    queue
        .into_iter()
        .take(len)
        .flatten()
        .filter_map(|(event, _)| match event.payload() {
            EventPayload::Note { identity, edge } => {
                Some((identity, matches!(edge, NoteEdge::On { .. })))
            }
            _ => None,
        })
        .collect()
}

fn accepted(outcome: MixedIngressOutcome) -> NoteIdentity {
    let MixedIngressOutcome::Onset(Ok(identity)) = outcome else {
        panic!("onset must be accepted, got {outcome:?}");
    };
    identity
}

#[test]
fn a_release_submitted_before_its_onset_result_ends_exactly_that_occurrence() {
    for compiled_first in [true, false] {
        let (mut control, audio) = armed(compiled_first);
        let handle = onset(&mut control, 140, 60);
        // No result has been collected or even produced: the release names the handle.
        release(&mut control, handle, 141);
        assert!(control.collect_ingress_result().is_none());
        let audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(results.len(), 2);
        let identity = accepted(outcome_for(&results, onset_request(handle, 140, 60)));
        assert_eq!(
            outcome_for(&results, release_request(handle, 141)),
            MixedIngressOutcome::Release(Ok(MixedRetired {
                occurrence: handle,
                cause: MixedRetireCause::Released(identity),
            }))
        );
        assert_eq!(
            queued_edges(&audio),
            vec![(identity, true), (identity, false)]
        );
        assert_eq!(audio.test_ingress.holds_outstanding(), EventCount::NONE);
        assert!(matches!(
            audio.audio.live_minter.resolve(identity),
            Resolution::Orphan(_)
        ));
        assert!(audio.occurrences.occupied().is_empty());
        assert_eq!(control.occurrence_ledger.held(), 0);
        assert_eq!(control.release_reserved.as_usize(), 0);
    }
}

#[test]
fn a_refused_first_onset_never_releases_an_accepted_same_key_second_onset() {
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    let order = IngressRefused::NonMonotoneStamp {
        time: SampleTime::new(140),
        last: SampleTime::new(150),
    };
    for compiled_first in [true, false] {
        for expected in [hold, order] {
            let (mut control, audio) = armed(compiled_first);
            // Every command is submitted before the first service and the first
            // collection, so the merger knows no result when it names a release.
            let (refused, second, refused_release_at) = if expected == hold {
                // Two live holds: the third onset is refused, then one release
                // returns a hold and the same key's next onset is accepted.
                let first = onset(&mut control, 140, 61);
                let _peer = onset(&mut control, 140, 62);
                let refused = onset(&mut control, 140, 60);
                release(&mut control, first, 140);
                let second = onset(&mut control, 141, 60);
                (refused, second, 141)
            } else {
                let _peer = onset(&mut control, 150, 61);
                let refused = onset(&mut control, 140, 60);
                let second = onset(&mut control, 150, 60);
                (refused, second, 150)
            };
            release(&mut control, refused, refused_release_at);
            let mut audio = service_without_allocation(audio);
            let results = collect_all(&mut control);
            assert_eq!(
                outcome_for(&results, onset_request(refused, 140, 60)),
                MixedIngressOutcome::Onset(Err(expected))
            );
            let second_at = if expected == hold { 141 } else { 150 };
            let identity = accepted(outcome_for(&results, onset_request(second, second_at, 60)));
            assert_eq!(
                outcome_for(&results, release_request(refused, refused_release_at)),
                MixedIngressOutcome::Release(Ok(MixedRetired {
                    occurrence: refused,
                    cause: MixedRetireCause::AfterRefusal(expected),
                }))
            );
            assert_eq!(
                audio.occurrences.released_after_refusal(),
                EventCount::measured(1)
            );
            // The tombstone consumed its release without a queue entry.
            assert!(!queued_edges(&audio).contains(&(identity, false)));
            assert!(
                audio
                    .occurrences
                    .occupied()
                    .contains(&(second, MixedOccurrenceState::Sounding(identity)))
            );
            let mut samples = [0.0_f32; 256];
            audio
                .render_private(
                    AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("block"),
                )
                .expect("mixed render");
            let sounding = audio
                .audio
                .renderer
                .snapshot_mixed_sounding()
                .expect("registry");
            assert!(
                sounding.live().iter().any(|note| note.identity == identity),
                "the second same-key onset must still sound"
            );
            assert_eq!(audio.audio.live_minter.resolve(identity), Resolution::Live);

            // Its own release, and only that, ends it.
            release(&mut control, second, 300);
            audio.service_test_ingress_queue();
            assert_eq!(
                collect_all(&mut control)
                    .into_iter()
                    .map(|result| result.outcome)
                    .collect::<Vec<_>>(),
                vec![MixedIngressOutcome::Release(Ok(MixedRetired {
                    occurrence: second,
                    cause: MixedRetireCause::Released(identity),
                }))]
            );
            assert_eq!(queued_edges(&audio), vec![(identity, false)]);
        }
    }
}

#[test]
fn a_cell_is_reused_only_after_retired_and_a_stale_generation_refuses() {
    let (mut control, audio) = armed(true);
    let first = onset(&mut control, 140, 60);
    release(&mut control, first, 141);
    let mut audio = service_without_allocation(audio);
    // Audio has retired the cell, but the merger has not collected `Retired`.
    let unacknowledged = onset(&mut control, 142, 61);
    assert_ne!(unacknowledged.cell, first.cell);
    let results = collect_all(&mut control);
    assert!(matches!(
        outcome_for(&results, release_request(first, 141)),
        MixedIngressOutcome::Release(Ok(MixedRetired { occurrence, .. })) if occurrence == first
    ));
    let reused = onset(&mut control, 143, 60);
    assert_eq!(reused.cell, first.cell);
    assert!(reused.generation > first.generation);
    assert_eq!(
        control.submit_release(ORIGIN, first, SampleTime::new(150)),
        Err(MixedReleaseSubmitError::NotOpen(first))
    );
    audio.service_test_ingress_queue();
    let identity = accepted(outcome_for(
        &collect_all(&mut control),
        onset_request(reused, 143, 60),
    ));

    // Forged commands reach audio's own checks; none changes a cell.
    let free_cell = LiveOccurrenceId {
        cell: LiveOccurrenceCell(2),
        generation: LiveOccurrenceGeneration::FIRST,
    };
    let unissued = LiveOccurrenceId {
        generation: reused.generation.next().expect("next generation"),
        ..reused
    };
    let unknown = LiveOccurrenceId {
        cell: LiveOccurrenceCell(u32::MAX),
        generation: LiveOccurrenceGeneration::FIRST,
    };
    let forged = [
        (
            release_request(first, 150),
            MixedHandleRefused::Stale {
                offered: first.generation,
                current: reused.generation,
            },
        ),
        (
            onset_request(first, 150, 62),
            MixedHandleRefused::Stale {
                offered: first.generation,
                current: reused.generation,
            },
        ),
        (
            onset_request(unissued, 150, 62),
            MixedHandleRefused::Unissued {
                offered: unissued.generation,
                current: reused.generation,
            },
        ),
        (onset_request(reused, 150, 62), MixedHandleRefused::Occupied),
        (release_request(free_cell, 150), MixedHandleRefused::NotOpen),
        (
            release_request(unknown, 150),
            MixedHandleRefused::UnknownCell,
        ),
    ];
    for (request, _) in forged {
        forge(&mut control, request);
    }
    let holds = audio.test_ingress.holds_outstanding();
    let occupied = audio.occurrences.occupied();
    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    for (request, refused) in forged {
        assert_eq!(
            outcome_for(&results, request),
            MixedIngressOutcome::Handle(refused)
        );
    }
    assert_eq!(audio.occurrences.occupied(), occupied);
    assert!(occupied.contains(&(reused, MixedOccurrenceState::Sounding(identity))));
    assert_eq!(audio.test_ingress.holds_outstanding(), holds);
    assert_eq!(audio.audio.live_minter.resolve(identity), Resolution::Live);
    assert!(control.occurrence_ledger.is_open(reused));
    assert!(control.occurrence_ledger.is_open(unacknowledged));
}

#[test]
fn a_tombstone_keeps_its_first_refusal_until_its_release() {
    let (mut control, audio) = armed(false);
    let _peer = onset(&mut control, 150, 61);
    let refused = onset(&mut control, 140, 60);
    let mut audio = service_without_allocation(audio);
    let reason = IngressRefused::NonMonotoneStamp {
        time: SampleTime::new(140),
        last: SampleTime::new(150),
    };
    assert_eq!(
        outcome_for(&collect_all(&mut control), onset_request(refused, 140, 60)),
        MixedIngressOutcome::Onset(Err(reason))
    );
    // A second onset under the same handle cannot overwrite the tombstone, even one
    // that ingress would now accept.
    forge(&mut control, onset_request(refused, 160, 60));
    audio.service_test_ingress_queue();
    assert_eq!(
        outcome_for(&collect_all(&mut control), onset_request(refused, 160, 60)),
        MixedIngressOutcome::Handle(MixedHandleRefused::Occupied)
    );
    assert!(
        audio
            .occurrences
            .occupied()
            .contains(&(refused, MixedOccurrenceState::Refused(reason)))
    );
    release(&mut control, refused, 160);
    audio.service_test_ingress_queue();
    assert_eq!(
        outcome_for(&collect_all(&mut control), release_request(refused, 160)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: refused,
            cause: MixedRetireCause::AfterRefusal(reason),
        }))
    );
}

#[test]
fn an_exhausted_generation_retires_its_cell_for_good() {
    let (mut control, mut audio) = armed(true);
    let last = LiveOccurrenceGeneration(u32::MAX);
    control.occurrence_ledger.cells[0] = CellUse::Free(last);
    audio.occurrences.cells[0] = CellUse::Free(last);
    let handle = onset(&mut control, 140, 60);
    assert_eq!(handle.cell, LiveOccurrenceCell(0));
    assert_eq!(handle.generation, last);
    release(&mut control, handle, 141);
    audio.service_test_ingress_queue();
    let results = collect_all(&mut control);
    assert!(matches!(
        outcome_for(&results, release_request(handle, 141)),
        MixedIngressOutcome::Release(Ok(MixedRetired { occurrence, .. })) if occurrence == handle
    ));
    assert_eq!(control.occurrence_ledger.cells[0], CellUse::Exhausted);
    assert_eq!(audio.occurrences.cells[0], CellUse::Exhausted);
    let next = onset(&mut control, 142, 60);
    assert_eq!(next.cell, LiveOccurrenceCell(1));
    forge(&mut control, onset_request(handle, 143, 61));
    audio.service_test_ingress_queue();
    assert_eq!(
        outcome_for(&collect_all(&mut control), onset_request(handle, 143, 61)),
        MixedIngressOutcome::Handle(MixedHandleRefused::Exhausted)
    );
}

#[test]
fn teardown_classifies_every_held_handle_by_its_audio_state() {
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    let (mut control, audio) = armed(true);
    let first = onset(&mut control, 140, 60);
    let second = onset(&mut control, 140, 61);
    let refused = onset(&mut control, 140, 62);
    let consumed = onset(&mut control, 140, 63);
    release(&mut control, consumed, 141);
    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let first_identity = accepted(outcome_for(&results, onset_request(first, 140, 60)));
    let second_identity = accepted(outcome_for(&results, onset_request(second, 140, 61)));
    assert_eq!(control.occurrence_ledger.held(), 3);
    let MixedCollection::Ended(teardown) = control.collect(audio).expect("collection") else {
        panic!("a pending one-shot owner ends at collection");
    };
    assert_eq!(
        teardown.ingress.occurrences,
        vec![
            (first, MixedOccurrenceState::Sounding(first_identity)),
            (second, MixedOccurrenceState::Sounding(second_identity)),
            (refused, MixedOccurrenceState::Refused(hold)),
        ]
    );
    assert_eq!(
        teardown.ingress.released_after_refusal,
        EventCount::measured(1)
    );
    assert_eq!(teardown.ingress.holds_outstanding, EventCount::measured(2));
}

mod sustain;
