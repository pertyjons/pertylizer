//! Panic tests for the private mixed owner.
//!
//! Every test asserts the outstanding release holds after each step: Panic redeems
//! every sounding and sustained hold in one group, and a later key-up of an occurrence
//! it ended is consumed by its tombstone without touching the queue.

use ringbuf::traits::{Consumer, Producer};

use super::{
    super::{
        super::MixedPedalSubmitted, MixedLifted, MixedOnsetSubmitError, MixedPanicEnd,
        MixedPanicRefused, MixedPanicSubmitError, MixedPanicked,
    },
    ExhaustedResource, IngressRefused, LiveOccurrenceId, MixedCollection, MixedHandleRefused,
    MixedIngressOutcome, MixedIngressRequest, MixedOccurrenceState, MixedOneShotAudio,
    MixedOneShotControl, MixedRetireCause, MixedRetired, NoteIdentity, ORIGIN, Resolution,
    accepted, collect_all, forge, free_units, key, outcome_for, release, release_request,
    service_without_allocation,
    sustain::{
        Queued, armed_with, holds, onset_in, onset_request_in, pedal, pedal_request, queued,
        render, scope, sounding, state_of,
    },
};
use crate::{
    ingress::ReleaseCause,
    quantities::{EventCount, NoteVelocity},
    render::EventPayload,
    time::SampleTime,
};

fn panic_at(control: &mut MixedOneShotControl, at: u64) -> MixedIngressRequest {
    let (_, request) = control
        .submit_panic(ORIGIN, SampleTime::new(at))
        .expect("the protected Panic place is free");
    request
}

fn panic_request(at: u64) -> MixedIngressRequest {
    MixedIngressRequest::Panic {
        origin: ORIGIN,
        at: SampleTime::new(at),
    }
}

fn assert_invalid_panic_member_keeps_ledger_and_place(wrong_kind: bool) {
    let s = scope(1, 1);
    let (mut control, mut audio) = armed_with(true, 2);
    let open = onset_in(&mut control, s, 10, 60);
    audio.service_test_ingress_queue();
    let _onset = control.collect_ingress_result().unwrap();
    let _panic = panic_at(&mut control, 20);
    let panic_id = control.panic_inflight.unwrap();
    audio.service_test_ingress_queue();
    let actual = control.ingress_results.0.try_pop().unwrap();
    let MixedIngressOutcome::Panic(Ok(mut panicked)) = actual.outcome else {
        panic!("the actual Panic applied");
    };
    panicked.ended[0] = wrong_kind.then_some(MixedPanicEnd::Retired(open));
    let forged = super::MixedIngressResult {
        outcome: MixedIngressOutcome::Panic(Ok(panicked)),
        ..actual
    };
    let expected_result = forged.fields();
    assert!(audio.ingress_results.0.try_push(forged).is_ok());
    assert!(control.collect_ingress_result_checked().is_err());
    assert_eq!(
        control
            .invalid_ingress_result
            .as_ref()
            .map(super::MixedIngressResult::fields),
        Some(expected_result)
    );
    assert_eq!(control.panic_inflight, Some(panic_id));
    assert!(control.occurrence_ledger.is_open(open));
    assert!(control.occurrence_ledger.is_sounding(open));
    assert_eq!(
        control.ingress_journal.front().map(|packet| packet.id),
        Some(panic_id)
    );
}

#[test]
fn forged_panic_retirement_of_an_open_cell_keeps_ledger_and_place() {
    assert_invalid_panic_member_keeps_ledger_and_place(true);
}

#[test]
fn omitted_panic_tombstone_keeps_ledger_and_place() {
    assert_invalid_panic_member_keeps_ledger_and_place(false);
}

fn applied(outcome: MixedIngressOutcome) -> MixedPanicked {
    let MixedIngressOutcome::Panic(Ok(panicked)) = outcome else {
        panic!("Panic must apply, got {outcome:?}");
    };
    panicked
}

fn ended(outcome: MixedIngressOutcome) -> Vec<MixedPanicEnd> {
    applied(outcome).ended().collect()
}

/// Release groups in the registered ingress queue, in queue order, with their cause.
fn groups(audio: &MixedOneShotAudio) -> Vec<(ReleaseCause, Vec<NoteIdentity>)> {
    let mut queue = vec![None; audio.test_ingress.queue_capacity()];
    let len = audio
        .test_ingress
        .copy_queue_for_test(&mut queue)
        .expect("journal copy");
    queue
        .into_iter()
        .take(len)
        .flatten()
        .filter_map(|(event, _)| match event.payload() {
            EventPayload::ReleaseGroup(group) => {
                Some((group.cause(), group.identities().collect()))
            }
            _ => None,
        })
        .collect()
}

fn after_panic(occurrence: LiveOccurrenceId, identity: NoteIdentity) -> MixedIngressOutcome {
    MixedIngressOutcome::Release(Ok(MixedRetired {
        occurrence,
        cause: MixedRetireCause::AfterPanic(identity),
    }))
}

#[test]
fn panic_ends_sounding_and_sustained_occurrences_and_tombstones_keys_still_down() {
    for compiled_first in [true, false] {
        let (s, other) = (scope(1, 1), scope(2, 3));
        let (mut control, audio) = armed_with(compiled_first, 4);
        pedal(&mut control, s, 10, true);
        let down = onset_in(&mut control, s, 20, 60);
        let sustained = onset_in(&mut control, s, 21, 61);
        release(&mut control, sustained, 22);
        let elsewhere = onset_in(&mut control, other, 23, 60);
        let panic = panic_at(&mut control, 40);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        let idown = accepted(outcome_for(&results, onset_request_in(down, s, 20, 60)));
        let isustained = accepted(outcome_for(
            &results,
            onset_request_in(sustained, s, 21, 61),
        ));
        let ielsewhere = accepted(outcome_for(
            &results,
            onset_request_in(elsewhere, other, 23, 60),
        ));
        assert_eq!(
            outcome_for(&results, release_request(sustained, 22)),
            MixedIngressOutcome::Sustained(sustained)
        );
        let panicked = applied(outcome_for(&results, panic));
        assert_eq!(panicked.executed_at(), SampleTime::new(40));
        // Cell order across scopes, which is also the group's member order.
        assert_eq!(
            panicked.ended().collect::<Vec<_>>(),
            vec![
                MixedPanicEnd::Tombstoned(down),
                MixedPanicEnd::Retired(sustained),
                MixedPanicEnd::Tombstoned(elsewhere),
            ]
        );
        assert_eq!(holds(&audio), 0);
        assert_eq!(
            groups(&audio),
            vec![(ReleaseCause::Panic, vec![idown, isustained, ielsewhere])]
        );
        assert_eq!(
            queued(&audio),
            vec![
                Queued::On(idown),
                Queued::On(isustained),
                Queued::On(ielsewhere),
                Queued::Group(vec![idown, isustained, ielsewhere]),
            ]
        );
        assert_eq!(
            state_of(&audio, down),
            MixedOccurrenceState::Panicked(idown)
        );
        assert_eq!(
            state_of(&audio, elsewhere),
            MixedOccurrenceState::Panicked(ielsewhere)
        );
        assert_eq!(audio.occurrences.occupied().len(), 2);
        for identity in [idown, isustained, ielsewhere] {
            assert!(matches!(
                audio.audio.live_minter.resolve(identity),
                Resolution::Orphan(_)
            ));
        }
        // The merger freed the sustained cell; the tombstoned handles keep their
        // reserved key-up places. Panic changed no pedal.
        assert_eq!(control.occurrence_ledger.held(), 2);
        assert!(!control.occurrence_ledger.is_sustained(sustained));
        assert_eq!(control.release_reserved.as_usize(), 2);
        assert_eq!(audio.occurrences.pedals_down(), vec![s]);
        assert!(control.pedal_ledger.is_down(s));
        assert!(control.panic_inflight.is_none());

        render(&mut audio);
        render(&mut audio);
        let live = sounding(&audio);
        assert!(
            [idown, isustained, ielsewhere]
                .iter()
                .all(|identity| !live.contains(identity))
        );

        // The late key-ups are consumed and counted; neither writes a queue entry. The
        // pedal's lift then finds nothing sustained.
        release(&mut control, down, 300);
        release(&mut control, elsewhere, 301);
        pedal(&mut control, s, 302, false);
        audio.service_test_ingress_queue();
        let results = collect_all(&mut control);
        assert_eq!(
            outcome_for(&results, release_request(down, 300)),
            after_panic(down, idown)
        );
        assert_eq!(
            outcome_for(&results, release_request(elsewhere, 301)),
            after_panic(elsewhere, ielsewhere)
        );
        assert_eq!(
            outcome_for(&results, pedal_request(s, 302, false)),
            MixedIngressOutcome::Lift(Ok(MixedLifted::NONE))
        );
        assert_eq!(
            audio.occurrences.released_after_panic(),
            EventCount::measured(2)
        );
        assert_eq!(audio.occurrences.released_after_refusal(), EventCount::NONE);
        assert!(audio.test_ingress.is_empty());
        assert!(audio.occurrences.occupied().is_empty());
        assert!(audio.occurrences.pedals_down().is_empty());
        assert_eq!(control.occurrence_ledger.held(), 0);
        assert_eq!(control.ingress_room(), control.ingress_capacity.as_usize());
    }
}

#[test]
fn panic_takes_its_protected_place_in_a_ring_full_of_commands_and_uncollected_results() {
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 8);
    let capacity = control.ingress_capacity.as_usize();
    let mut submitted = Vec::new();
    let MixedPedalSubmitted::Command(id, request) = pedal(&mut control, s, 10, true) else {
        panic!("the first pedal-down is effective");
    };
    submitted.push((id, request));
    let mut notes = Vec::new();
    for raw in 0..(capacity - 2) / 2 {
        let raw = u8::try_from(raw).expect("small key");
        let note = onset_in(&mut control, s, 20, raw);
        submitted.push((
            control.last_ingress_id.expect("submitted"),
            onset_request_in(note, s, 20, raw),
        ));
        notes.push(note);
    }
    for note in &notes {
        submitted.push(
            control
                .submit_release(ORIGIN, *note, SampleTime::new(30))
                .expect("reserved release place"),
        );
    }
    assert!(notes.len() > 8, "more onsets than holds");
    // Only the lift's reservation is left, and no result has been collected.
    assert_eq!(control.ingress_room(), 0);
    assert_eq!(
        control.submit_onset(ORIGIN, s, SampleTime::new(40), key(100), NoteVelocity::FULL),
        Err(MixedOnsetSubmitError::Full)
    );
    let mut audio = service_without_allocation(audio);
    assert_eq!(control.ingress_outstanding.as_usize(), capacity - 1);

    let (id, request) = control
        .submit_panic(ORIGIN, SampleTime::new(40))
        .expect("the protected place is free");
    submitted.push((id, request));
    let MixedPedalSubmitted::Command(id, request) = pedal(&mut control, s, 50, false) else {
        panic!("the lift is effective");
    };
    submitted.push((id, request));
    assert_eq!(control.ingress_outstanding.as_usize(), capacity);
    assert_eq!(control.panic_inflight, Some(submitted[capacity - 1].0));
    audio = service_without_allocation(audio);
    // Both results fit beside the capacity - 1 uncollected ones: the Panic took the
    // result ring's protected place.
    assert!(audio.ingress_result_pending.is_none());

    let results = collect_all(&mut control);
    assert_eq!(
        results
            .iter()
            .map(|result| (result.id, result.request))
            .collect::<Vec<_>>(),
        submitted
    );
    let mut sustained = Vec::new();
    for (index, note) in notes.iter().enumerate() {
        let onset = results[1 + index].outcome;
        let key_up = results[1 + notes.len() + index].outcome;
        if index < 8 {
            let MixedIngressOutcome::Onset(Ok(identity)) = onset else {
                panic!("the first eight onsets take the eight holds");
            };
            assert_eq!(key_up, MixedIngressOutcome::Sustained(*note));
            sustained.push((*note, identity));
        } else {
            assert_eq!(onset, MixedIngressOutcome::Onset(Err(hold)));
            assert_eq!(
                key_up,
                MixedIngressOutcome::Release(Ok(MixedRetired {
                    occurrence: *note,
                    cause: MixedRetireCause::AfterRefusal(hold),
                }))
            );
        }
    }
    let (handles, members): (Vec<_>, Vec<_>) = sustained.into_iter().unzip();
    assert_eq!(
        ended(results[capacity - 1].outcome),
        handles
            .iter()
            .map(|note| MixedPanicEnd::Retired(*note))
            .collect::<Vec<_>>()
    );
    // A full eight-member group, and the lift after it finds nothing.
    assert_eq!(groups(&audio), vec![(ReleaseCause::Panic, members)]);
    assert_eq!(
        results[capacity].outcome,
        MixedIngressOutcome::Lift(Ok(MixedLifted::NONE))
    );
    assert_eq!(holds(&audio), 0);
    assert_eq!(control.occurrence_ledger.held(), 0);
    assert!(control.panic_inflight.is_none());
    assert_eq!(control.ingress_room(), capacity);
}

#[test]
fn panic_needs_no_ingress_place_with_the_queue_full_and_every_hold_out() {
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (mut control, audio) = armed_with(true, 8);
    let depth = audio.test_ingress.queue_capacity();
    // Eight live notes' note-on entries and holds are all the ordinary room the
    // bend-credit budget leaves: `B = C - 2H`.
    assert_eq!(depth - audio.test_ingress.mixed_bend_budget(), 16);
    pedal(&mut control, s, 10, true);
    let mut sustained = Vec::new();
    let mut down = Vec::new();
    for raw in 0..4 {
        let at = 20 + 3 * u64::from(raw);
        let note = onset_in(&mut control, s, at, 60 + raw);
        release(&mut control, note, at + 1);
        sustained.push((note, onset_request_in(note, s, at, 60 + raw)));
        let note = onset_in(&mut control, other, at + 2, 70 + raw);
        down.push((note, onset_request_in(note, other, at + 2, 70 + raw)));
    }
    let starved = onset_in(&mut control, other, 33, 100);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(holds(&audio), 8);
    assert_eq!(audio.test_ingress.len(), 8);
    assert_eq!(free_units(&audio), 0);
    assert_eq!(
        outcome_for(&results, onset_request_in(starved, other, 33, 100)),
        MixedIngressOutcome::Onset(Err(IngressRefused::Dropped {
            resource: ExhaustedResource::Hold,
        }))
    );
    let identity = |request| accepted(outcome_for(&results, request));
    let isustained = sustained
        .iter()
        .map(|(_, request)| identity(*request))
        .collect::<Vec<_>>();
    let idown = down
        .iter()
        .map(|(_, request)| identity(*request))
        .collect::<Vec<_>>();
    release(&mut control, starved, 34);

    let panic = panic_at(&mut control, 35);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let panicked = ended(outcome_for(&results, panic));
    assert_eq!(panicked.len(), 8);
    for (note, _) in &sustained {
        assert!(panicked.contains(&MixedPanicEnd::Retired(*note)));
    }
    for (note, _) in &down {
        assert!(panicked.contains(&MixedPanicEnd::Tombstoned(*note)));
    }
    // Eight reservations became one group entry.
    assert_eq!(holds(&audio), 0);
    assert_eq!(audio.test_ingress.len(), 9);
    assert_eq!(free_units(&audio), 7);
    let (cause, members) = groups(&audio).pop().expect("the Panic group");
    assert_eq!(cause, ReleaseCause::Panic);
    assert_eq!(members.len(), 8);
    assert!(
        isustained
            .iter()
            .chain(&idown)
            .all(|identity| members.contains(identity))
    );

    // The freed holds admit a new onset at once; the late key-ups need no place.
    let fresh = onset_in(&mut control, other, 40, 100);
    for (note, _) in &down {
        release(&mut control, *note, 41);
    }
    audio.service_test_ingress_queue();
    let results = collect_all(&mut control);
    let ifresh = accepted(outcome_for(
        &results,
        onset_request_in(fresh, other, 40, 100),
    ));
    for ((note, _), identity) in down.iter().zip(&idown) {
        assert_eq!(
            outcome_for(&results, release_request(*note, 41)),
            after_panic(*note, *identity)
        );
    }
    assert_eq!(holds(&audio), 1);
    assert_eq!(audio.test_ingress.len(), 10);
    assert_eq!(free_units(&audio), 5);
    render(&mut audio);
    let live = sounding(&audio);
    assert!(
        isustained
            .iter()
            .chain(&idown)
            .all(|identity| !live.contains(identity))
    );
    assert!(live.contains(&ifresh));
}

#[test]
fn a_late_key_up_after_panic_never_ends_a_newer_same_key_occurrence() {
    for compiled_first in [true, false] {
        let s = scope(1, 1);
        let (mut control, audio) = armed_with(compiled_first, 2);
        let old = onset_in(&mut control, s, 10, 60);
        let panic = panic_at(&mut control, 20);
        let newer = onset_in(&mut control, s, 30, 60);
        release(&mut control, old, 40);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        let iold = accepted(outcome_for(&results, onset_request_in(old, s, 10, 60)));
        let inewer = accepted(outcome_for(&results, onset_request_in(newer, s, 30, 60)));
        assert_ne!(iold, inewer);
        assert_eq!(
            ended(outcome_for(&results, panic)),
            vec![MixedPanicEnd::Tombstoned(old)]
        );
        assert_eq!(
            outcome_for(&results, release_request(old, 40)),
            after_panic(old, iold)
        );
        assert_eq!(
            state_of(&audio, newer),
            MixedOccurrenceState::Sounding(inewer)
        );
        assert_eq!(holds(&audio), 1);
        assert_eq!(
            queued(&audio),
            vec![
                Queued::On(iold),
                Queued::Group(vec![iold]),
                Queued::On(inewer),
            ]
        );
        render(&mut audio);
        assert!(sounding(&audio).contains(&inewer));

        release(&mut control, newer, 300);
        audio.service_test_ingress_queue();
        assert_eq!(
            outcome_for(&collect_all(&mut control), release_request(newer, 300)),
            MixedIngressOutcome::Release(Ok(MixedRetired {
                occurrence: newer,
                cause: MixedRetireCause::Released(inewer),
            }))
        );
        assert_eq!(holds(&audio), 0);
        assert_eq!(
            audio.occurrences.released_after_panic(),
            EventCount::measured(1)
        );
    }
}

#[test]
fn a_second_panic_is_refused_at_the_caller_until_the_first_result_is_collected() {
    let order = IngressRefused::NonMonotoneStamp {
        time: SampleTime::new(40),
        last: SampleTime::new(50),
    };
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 2);
    let held = onset_in(&mut control, s, 50, 61);
    let refused = onset_in(&mut control, s, 40, 60);
    let (first, first_request) = control
        .submit_panic(ORIGIN, SampleTime::new(60))
        .expect("the protected place is free");
    let last = control.last_ingress_id;
    let room = control.ingress_room();
    assert_eq!(
        control.submit_panic(ORIGIN, SampleTime::new(61)),
        Err(MixedPanicSubmitError::Pending(first))
    );
    // The refusal pushed nothing and minted no command identity.
    assert_eq!(control.last_ingress_id, last);
    assert_eq!(control.ingress_room(), room);
    let mut audio = service_without_allocation(audio);
    // Served but uncollected, the result still holds the protected place.
    assert_eq!(
        control.submit_panic(ORIGIN, SampleTime::new(62)),
        Err(MixedPanicSubmitError::Pending(first))
    );
    let results = collect_all(&mut control);
    assert_eq!(results.len(), 3);
    let iheld = accepted(outcome_for(&results, onset_request_in(held, s, 50, 61)));
    assert_eq!(
        outcome_for(&results, onset_request_in(refused, s, 40, 60)),
        MixedIngressOutcome::Onset(Err(order))
    );
    assert_eq!(
        ended(outcome_for(&results, first_request)),
        vec![MixedPanicEnd::Tombstoned(held)]
    );
    assert_eq!(
        state_of(&audio, refused),
        MixedOccurrenceState::Refused(order)
    );

    assert_eq!(groups(&audio), vec![(ReleaseCause::Panic, vec![iheld])]);

    // A second applied Panic, in a later quantum, ends nothing more and overwrites
    // neither tombstone.
    render(&mut audio);
    let second = panic_at(&mut control, 200);
    audio.service_test_ingress_queue();
    let results = collect_all(&mut control);
    assert_eq!(ended(outcome_for(&results, second)), Vec::new());
    assert_eq!(
        state_of(&audio, held),
        MixedOccurrenceState::Panicked(iheld)
    );
    assert_eq!(
        state_of(&audio, refused),
        MixedOccurrenceState::Refused(order)
    );
    assert!(groups(&audio).is_empty());

    // Each key-up is consumed under its first cause.
    release(&mut control, refused, 201);
    release(&mut control, held, 202);
    audio.service_test_ingress_queue();
    let results = collect_all(&mut control);
    assert_eq!(
        outcome_for(&results, release_request(refused, 201)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: refused,
            cause: MixedRetireCause::AfterRefusal(order),
        }))
    );
    assert_eq!(
        outcome_for(&results, release_request(held, 202)),
        after_panic(held, iheld)
    );
    assert_eq!(
        audio.occurrences.released_after_refusal(),
        EventCount::measured(1)
    );
    assert_eq!(
        audio.occurrences.released_after_panic(),
        EventCount::measured(1)
    );
    assert_eq!(control.occurrence_ledger.held(), 0);
}

#[test]
fn a_refused_panic_changes_nothing_and_frees_its_place() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 2);
    pedal(&mut control, s, 10, true);
    let down = onset_in(&mut control, s, 20, 60);
    let sustained = onset_in(&mut control, s, 21, 61);
    release(&mut control, sustained, 22);
    let far = u64::MAX / 2;
    let refused = panic_at(&mut control, far);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let idown = accepted(outcome_for(&results, onset_request_in(down, s, 20, 60)));
    let isustained = accepted(outcome_for(
        &results,
        onset_request_in(sustained, s, 21, 61),
    ));
    let MixedIngressOutcome::Panic(Err(MixedPanicRefused::Ingress(
        IngressRefused::BeyondHorizon { time, .. },
    ))) = outcome_for(&results, refused)
    else {
        panic!("a Panic beyond the horizon is refused");
    };
    assert_eq!(time, SampleTime::new(far));
    assert_eq!(holds(&audio), 2);
    assert_eq!(
        state_of(&audio, down),
        MixedOccurrenceState::Sounding(idown)
    );
    assert_eq!(
        state_of(&audio, sustained),
        MixedOccurrenceState::Sustained(isustained)
    );
    assert!(groups(&audio).is_empty());
    assert!(control.occurrence_ledger.is_sustained(sustained));
    assert!(control.panic_inflight.is_none());

    // The place is free again and a valid Panic applies.
    let panic = panic_at(&mut control, 30);
    audio.service_test_ingress_queue();
    assert_eq!(
        ended(outcome_for(&collect_all(&mut control), panic)),
        vec![
            MixedPanicEnd::Tombstoned(down),
            MixedPanicEnd::Retired(sustained),
        ]
    );
    assert_eq!(holds(&audio), 0);
}

#[test]
fn a_panic_stamped_before_the_last_accepted_edge_executes_at_that_fence() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 2);
    let note = onset_in(&mut control, s, 50, 60);
    let panic = panic_at(&mut control, 10);
    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let identity = accepted(outcome_for(&results, onset_request_in(note, s, 50, 60)));
    let panicked = applied(outcome_for(&results, panic));
    // The result retains the nominal stamp in its request and reports the executed one.
    assert_eq!(panic, panic_request(10));
    assert_eq!(panicked.executed_at(), SampleTime::new(50));
    assert_eq!(
        panicked.ended().collect::<Vec<_>>(),
        vec![MixedPanicEnd::Tombstoned(note)]
    );
    assert_eq!(groups(&audio), vec![(ReleaseCause::Panic, vec![identity])]);
    assert_eq!(audio.test_ingress.counters().non_monotone(), 0);
}

#[test]
fn panic_results_and_retirements_follow_submission_order() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(false, 4);
    pedal(&mut control, s, 10, true);
    let down = onset_in(&mut control, s, 20, 60);
    let sustained = onset_in(&mut control, s, 21, 61);
    release(&mut control, sustained, 22);
    let panic = panic_at(&mut control, 30);
    release(&mut control, down, 40);
    let after = onset_in(&mut control, s, 41, 61);
    let audio = service_without_allocation(audio);
    let order = [
        pedal_request(s, 10, true),
        onset_request_in(down, s, 20, 60),
        onset_request_in(sustained, s, 21, 61),
        release_request(sustained, 22),
        panic,
        release_request(down, 40),
        onset_request_in(after, s, 41, 61),
    ];
    let mut collected = Vec::new();
    while let Some(result) = control.collect_ingress_result() {
        // The merger frees the sustained cell exactly when it collects the Panic, and
        // the tombstoned cell only when it collects that cell's own key-up.
        let step = collected.len();
        assert_eq!(control.occurrence_ledger.is_sustained(sustained), step == 3);
        let held = match step {
            0..4 => 3,
            4 => 2,
            _ => 1,
        };
        assert_eq!(
            control.occurrence_ledger.held(),
            held,
            "after result {step}"
        );
        assert_eq!(control.panic_inflight.is_some(), step < 4);
        collected.push(result);
    }
    assert_eq!(
        collected
            .iter()
            .map(|result| result.request)
            .collect::<Vec<_>>(),
        order
    );
    let idown = accepted(collected[1].outcome);
    let isustained = accepted(collected[2].outcome);
    let iafter = accepted(collected[6].outcome);
    assert_eq!(
        ended(collected[4].outcome),
        vec![
            MixedPanicEnd::Tombstoned(down),
            MixedPanicEnd::Retired(sustained),
        ]
    );
    assert_eq!(collected[5].outcome, after_panic(down, idown));
    assert_eq!(
        queued(&audio),
        vec![
            Queued::On(idown),
            Queued::On(isustained),
            Queued::Group(vec![idown, isustained]),
            Queued::On(iafter),
        ]
    );
    // The retired sustained handle no longer resolves on audio.
    let mut audio = audio;
    forge(&mut control, release_request(sustained, 50));
    audio.service_test_ingress_queue();
    assert!(matches!(
        outcome_for(&collect_all(&mut control), release_request(sustained, 50)),
        MixedIngressOutcome::Handle(MixedHandleRefused::Stale { .. } | MixedHandleRefused::NotOpen)
    ));
    assert_eq!(holds(&audio), 1);
}

#[test]
fn teardown_waits_for_the_panic_result_and_reports_its_tombstones() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 2);
    pedal(&mut control, s, 10, true);
    let down = onset_in(&mut control, s, 20, 60);
    let sustained = onset_in(&mut control, s, 21, 61);
    release(&mut control, sustained, 22);
    let _panic = panic_at(&mut control, 30);
    let audio = service_without_allocation(audio);
    // Collect every ordinary result; only the Panic's is left.
    let mut results = Vec::new();
    for _ in 0..4 {
        results.push(control.collect_ingress_result().expect("ordinary result"));
    }
    assert_eq!(control.ingress_outstanding.as_usize(), 0);
    assert!(control.panic_inflight.is_some());
    let refused = control
        .collect(audio)
        .expect_err("an uncollected Panic blocks collection");
    assert_eq!(
        refused.reason,
        super::super::super::MixedCollectionError::IngressResultPending
    );
    let (mut control, audio) = (refused.control, refused.audio);
    let idown = accepted(outcome_for(&results, onset_request_in(down, s, 20, 60)));
    let panicked = control.collect_ingress_result().expect("the Panic result");
    assert!(matches!(
        panicked.request,
        MixedIngressRequest::Panic { .. }
    ));
    let MixedCollection::Ended(teardown) = control.collect(audio).expect("collection") else {
        panic!("a pending one-shot owner ends at collection");
    };
    assert_eq!(
        teardown.ingress.occurrences,
        vec![(down, MixedOccurrenceState::Panicked(idown))]
    );
    assert_eq!(teardown.ingress.holds_outstanding, EventCount::NONE);
    assert!(teardown.ingress.minted_live.is_empty());
    assert_eq!(teardown.ingress.pedals_down, vec![s]);
    assert_eq!(teardown.ingress.released_after_panic, EventCount::NONE);
    assert!(teardown.ingress.queued.iter().any(|(event, _)| matches!(
        event.payload(),
        EventPayload::ReleaseGroup(group) if group.cause() == ReleaseCause::Panic
    )));
}
