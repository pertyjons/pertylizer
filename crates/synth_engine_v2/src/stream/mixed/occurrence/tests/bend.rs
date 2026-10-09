//! Channel bend tests for the private mixed owner.
//!
//! Every test inspects the registered ingress queue after service: a bend writes one
//! entry per sounding or sustained member of its scope, coalesces in place within one
//! quantum, and an onset on a bent scope writes its inherited bend right after its
//! note-on. Rendering then proves each bend resolved against a live occurrence.

use super::{
    super::{super::MixedPedalSubmitted, MixedBendRefused, MixedBendSubmitError, MixedLiveScope},
    ExhaustedResource, IngressRefused, LiveOccurrenceId, MixedIngressOutcome, MixedIngressRequest,
    MixedOccurrenceState, MixedOneShotAudio, MixedOneShotControl, MixedRetireCause, MixedRetired,
    NoteIdentity, ORIGIN, accepted, collect_all, outcome_for, release, release_request,
    service_without_allocation,
    sustain::{
        armed_with, holds, onset_in, onset_request_in, pedal, render, scope, sounding, state_of,
    },
};
use crate::{
    ingress::{MixedBendWrite, ReleaseCause},
    quantities::{Cents, EventCount},
    render::{EventPayload, NoteEdge},
    time::SampleTime,
};

fn cents(value: f32) -> Cents {
    Cents::new(value).expect("valid bend")
}

fn bend_at(
    control: &mut MixedOneShotControl,
    scope: MixedLiveScope,
    at: u64,
    value: f32,
) -> MixedIngressRequest {
    let (_, request) = control
        .submit_bend(ORIGIN, scope, SampleTime::new(at), cents(value))
        .expect("bend fits an ordinary place");
    request
}

fn written(outcome: MixedIngressOutcome) -> Vec<(LiveOccurrenceId, MixedBendWrite)> {
    let MixedIngressOutcome::Bend(Ok(bent)) = outcome else {
        panic!("bend must apply, got {outcome:?}");
    };
    bent.written().collect()
}

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    On(NoteIdentity, u64),
    Off(NoteIdentity, u64),
    Bend(NoteIdentity, f32, u64),
    Group(ReleaseCause, Vec<NoteIdentity>),
}

/// Every entry still waiting in the registered ingress queue, in queue order.
fn entries(audio: &MixedOneShotAudio) -> Vec<Entry> {
    let mut queue = vec![None; audio.test_ingress.queue_capacity_for_test()];
    let len = audio
        .test_ingress
        .copy_queue_for_test(&mut queue)
        .expect("journal copy");
    queue
        .into_iter()
        .take(len)
        .flatten()
        .map(|(event, _)| {
            let at = event.envelope().time().as_u64();
            match event.payload() {
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On { .. },
                } => Entry::On(identity, at),
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                } => Entry::Off(identity, at),
                EventPayload::Bend { identity, cents } => Entry::Bend(identity, cents.as_f32(), at),
                EventPayload::ReleaseGroup(group) => {
                    Entry::Group(group.cause(), group.identities().collect())
                }
                other => panic!("unexpected live entry {other:?}"),
            }
        })
        .collect()
}

/// Orphaned note events the renderer counted, once the whole queue has drained.
fn orphans(audio: &MixedOneShotAudio) -> u64 {
    assert!(audio.test_ingress.is_empty(), "every entry was published");
    audio.audio.renderer.diagnostics().orphan_note_events()
}

/// Submit, serve and collect one onset/release pair on `scope`.
fn pair(
    control: &mut MixedOneShotControl,
    audio: &mut MixedOneShotAudio,
    scope: MixedLiveScope,
    at: u64,
    raw: u8,
) {
    let note = onset_in(control, scope, at, raw);
    release(control, note, at);
    audio.service_test_ingress_queue();
    for result in collect_all(control) {
        assert!(matches!(
            result.outcome,
            MixedIngressOutcome::Onset(Ok(_)) | MixedIngressOutcome::Release(Ok(_))
        ));
    }
}

#[test]
fn a_bend_writes_every_sounding_and_sustained_member_of_its_scope_only() {
    for compiled_first in [true, false] {
        let (s, other) = (scope(1, 1), scope(2, 1));
        let (mut control, audio) = armed_with(compiled_first, 4);
        assert!(matches!(
            pedal(&mut control, s, 100, true),
            MixedPedalSubmitted::Command(..)
        ));
        let down = onset_in(&mut control, s, 101, 60);
        let sustained = onset_in(&mut control, s, 102, 64);
        release(&mut control, sustained, 103);
        let elsewhere = onset_in(&mut control, other, 104, 60);
        let bend = bend_at(&mut control, s, 110, 100.0);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        let idown = accepted(outcome_for(&results, onset_request_in(down, s, 101, 60)));
        let isustained = accepted(outcome_for(
            &results,
            onset_request_in(sustained, s, 102, 64),
        ));
        let ielsewhere = accepted(outcome_for(
            &results,
            onset_request_in(elsewhere, other, 104, 60),
        ));
        assert_eq!(
            outcome_for(&results, release_request(sustained, 103)),
            MixedIngressOutcome::Sustained(sustained)
        );
        assert_eq!(
            written(outcome_for(&results, bend)),
            vec![
                (down, MixedBendWrite::Queued),
                (sustained, MixedBendWrite::Queued),
            ]
        );
        assert_eq!(
            entries(&audio),
            vec![
                Entry::On(idown, 101),
                Entry::On(isustained, 102),
                Entry::On(ielsewhere, 104),
                Entry::Bend(idown, 100.0, 110),
                Entry::Bend(isustained, 100.0, 110),
            ]
        );
        // A bend opens no obligation and leaves every occurrence's state alone.
        assert_eq!(holds(&audio), 3);
        assert_eq!(
            state_of(&audio, sustained),
            MixedOccurrenceState::Sustained(isustained)
        );
        assert_eq!(audio.occurrences.bend_of(s), cents(100.0));
        assert_eq!(audio.occurrences.bend_of(other), Cents::ZERO);

        render(&mut audio);
        assert_eq!(orphans(&audio), 0, "every bend found its live note");
        let live = sounding(&audio);
        assert!(
            [idown, isustained, ielsewhere]
                .iter()
                .all(|identity| live.contains(identity))
        );

        // The lift ends the sustained member; the scope keeps its value and the next
        // bend reaches only the key still down.
        pedal(&mut control, s, 300, false);
        let after = bend_at(&mut control, s, 310, -50.0);
        audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(
            written(outcome_for(&results, after)),
            vec![(down, MixedBendWrite::Queued)]
        );
        assert_eq!(
            entries(&audio),
            vec![
                Entry::Group(ReleaseCause::SustainLift, vec![isustained]),
                Entry::Bend(idown, -50.0, 310),
            ]
        );
        assert_eq!(audio.occurrences.bend_of(s), cents(-50.0));
        render(&mut audio);
        assert_eq!(orphans(&audio), 0);
        assert_eq!(audio.occurrences.bends_superseded(), EventCount::NONE);
    }
}

#[test]
fn an_onset_inherits_its_scopes_bend_and_a_zero_bend_frees_the_scope() {
    for compiled_first in [true, false] {
        let (s, other) = (scope(1, 1), scope(1, 2));
        let (mut control, audio) = armed_with(compiled_first, 4);
        // No member yet: the bend sets the value and the order fence only.
        let set = bend_at(&mut control, s, 100, 50.0);
        let bent = onset_in(&mut control, s, 100, 60);
        let plain = onset_in(&mut control, other, 100, 60);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(written(outcome_for(&results, set)), vec![]);
        let ibent = accepted(outcome_for(&results, onset_request_in(bent, s, 100, 60)));
        let iplain = accepted(outcome_for(
            &results,
            onset_request_in(plain, other, 100, 60),
        ));
        // The inherited bend follows its own note-on at the same stamp.
        assert_eq!(
            entries(&audio),
            vec![
                Entry::On(ibent, 100),
                Entry::Bend(ibent, 50.0, 100),
                Entry::On(iplain, 100),
            ]
        );
        assert_eq!(holds(&audio), 2);

        // Returning to zero keeps the inherited value at the onset stamp. The
        // scope's cell is freed, so a later onset writes no bend.
        let zero = bend_at(&mut control, s, 101, 0.0);
        let later = onset_in(&mut control, s, 102, 62);
        audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(
            written(outcome_for(&results, zero)),
            vec![(bent, MixedBendWrite::Queued)]
        );
        let ilater = accepted(outcome_for(&results, onset_request_in(later, s, 102, 62)));
        assert_eq!(
            entries(&audio),
            vec![
                Entry::On(ibent, 100),
                Entry::Bend(ibent, 50.0, 100),
                Entry::On(iplain, 100),
                Entry::Bend(ibent, 0.0, 101),
                Entry::On(ilater, 102),
            ]
        );
        assert_eq!(audio.occurrences.bend_of(s), Cents::ZERO);
        assert_eq!(audio.occurrences.bends_superseded(), EventCount::NONE);
        render(&mut audio);
        assert_eq!(orphans(&audio), 0);
    }
}

#[test]
fn an_intervening_onset_keeps_its_inherited_bend_at_its_own_stamp() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 3);
    let first = onset_in(&mut control, s, 100, 60);
    let initial = bend_at(&mut control, s, 110, 30.0);
    let second = onset_in(&mut control, s, 115, 62);
    let later = bend_at(&mut control, s, 120, 70.0);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ifirst = accepted(outcome_for(&results, onset_request_in(first, s, 100, 60)));
    let isecond = accepted(outcome_for(&results, onset_request_in(second, s, 115, 62)));
    assert_eq!(
        written(outcome_for(&results, initial)),
        vec![(first, MixedBendWrite::Queued)]
    );
    assert_eq!(
        written(outcome_for(&results, later)),
        vec![
            (first, MixedBendWrite::Coalesced),
            (second, MixedBendWrite::Queued)
        ]
    );
    assert_eq!(
        entries(&audio),
        vec![
            Entry::On(ifirst, 100),
            Entry::Bend(ifirst, 70.0, 120),
            Entry::On(isecond, 115),
            Entry::Bend(isecond, 30.0, 115),
            Entry::Bend(isecond, 70.0, 120),
        ]
    );
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
}

#[test]
fn an_ordinary_bend_after_an_onset_still_coalesces_when_the_queue_is_full() {
    let (s, other, filler) = (scope(1, 1), scope(2, 1), scope(3, 1));
    let (mut control, audio) = armed_with(true, 3);
    let depth = audio.test_ingress.queue_capacity_for_test();
    assert!(depth >= 6 && depth % 2 == 0);
    let first = onset_in(&mut control, s, 100, 60);
    let first_bend = bend_at(&mut control, s, 100, 25.0);
    let second = onset_in(&mut control, other, 100, 62);
    let second_bend = bend_at(&mut control, other, 100, 25.0);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ifirst = accepted(outcome_for(&results, onset_request_in(first, s, 100, 60)));
    let _isecond = accepted(outcome_for(
        &results,
        onset_request_in(second, other, 100, 62),
    ));
    assert_eq!(written(outcome_for(&results, first_bend)).len(), 1);
    assert_eq!(written(outcome_for(&results, second_bend)).len(), 1);
    for raw in 0..(depth - 6) / 2 {
        pair(
            &mut control,
            &mut audio,
            filler,
            100,
            u8::try_from(raw).expect("small key"),
        );
    }
    assert_eq!(audio.test_ingress.ring_len_for_test(), depth - 2);
    assert_eq!(holds(&audio), 2);
    let later = bend_at(&mut control, s, 110, 50.0);
    audio = service_without_allocation(audio);
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), later)),
        vec![(first, MixedBendWrite::Coalesced)]
    );
    assert_eq!(audio.test_ingress.ring_len_for_test(), depth - 2);
    assert!(entries(&audio).contains(&Entry::Bend(ifirst, 50.0, 110)));
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
}

#[test]
fn commands_at_one_stamp_apply_in_command_order() {
    for compiled_first in [true, false] {
        let s = scope(1, 1);
        let (mut control, audio) = armed_with(compiled_first, 4);
        let first = onset_in(&mut control, s, 200, 60);
        let bend = bend_at(&mut control, s, 200, 30.0);
        let second = onset_in(&mut control, s, 200, 61);
        release(&mut control, first, 200);
        let again = bend_at(&mut control, s, 200, 70.0);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        let ifirst = accepted(outcome_for(&results, onset_request_in(first, s, 200, 60)));
        let isecond = accepted(outcome_for(&results, onset_request_in(second, s, 200, 61)));
        assert_eq!(
            written(outcome_for(&results, bend)),
            vec![(first, MixedBendWrite::Queued)]
        );
        // The first note's key-up preceded the second bend, so only the second note is a
        // member, and its inherited entry takes the new value. The first note keeps the
        // value it had when it was released.
        assert_eq!(
            written(outcome_for(&results, again)),
            vec![(second, MixedBendWrite::Coalesced)]
        );
        assert_eq!(
            outcome_for(&results, release_request(first, 200)),
            MixedIngressOutcome::Release(Ok(MixedRetired {
                occurrence: first,
                cause: MixedRetireCause::Released(ifirst),
            }))
        );
        assert_eq!(
            entries(&audio),
            vec![
                Entry::On(ifirst, 200),
                Entry::Bend(ifirst, 30.0, 200),
                Entry::On(isecond, 200),
                Entry::Bend(isecond, 70.0, 200),
                Entry::Off(ifirst, 200),
            ]
        );

        // A bend behind the order fence is refused whole.
        let late = bend_at(&mut control, s, 199, 90.0);
        audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(
            outcome_for(&results, late),
            MixedIngressOutcome::Bend(Err(MixedBendRefused::Ingress(
                IngressRefused::NonMonotoneStamp {
                    time: SampleTime::new(199),
                    last: SampleTime::new(200),
                }
            )))
        );
        assert_eq!(audio.occurrences.bend_of(s), cents(70.0));
        assert_eq!(entries(&audio).len(), 5);

        // Each bend resolves after its note-on and before its note-off at one sample.
        render(&mut audio);
        render(&mut audio);
        assert!(audio.test_ingress.is_empty());
        assert_eq!(orphans(&audio), 0);
        assert!(sounding(&audio).contains(&isecond));
    }
}

#[test]
fn four_wheel_updates_over_a_held_chord_spend_one_slot_per_member_and_quantum() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 8);
    let notes: Vec<_> = (0..8)
        .map(|raw| onset_in(&mut control, s, 100, 60 + raw))
        .collect();
    let mut audio = service_without_allocation(audio);
    let identities: Vec<_> = collect_all(&mut control)
        .into_iter()
        .map(|result| accepted(result.outcome))
        .collect();
    // Four wheel messages inside quantum 1 (frames 64 to 127).
    let wheel: Vec<_> = [(110, 20.0), (115, -40.0), (120, 60.0), (125, 90.0)]
        .into_iter()
        .map(|(at, value)| bend_at(&mut control, s, at, value))
        .collect();
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    for (index, bend) in wheel.into_iter().enumerate() {
        let write = if index == 0 {
            MixedBendWrite::Queued
        } else {
            MixedBendWrite::Coalesced
        };
        assert_eq!(
            written(outcome_for(&results, bend)),
            notes.iter().map(|note| (*note, write)).collect::<Vec<_>>()
        );
    }
    // Eight note-ons and one bend per member, carrying the last value at the last stamp.
    let mut expected: Vec<_> = identities
        .iter()
        .map(|identity| Entry::On(*identity, 100))
        .collect();
    expected.extend(
        identities
            .iter()
            .map(|identity| Entry::Bend(*identity, 90.0, 125)),
    );
    assert_eq!(entries(&audio), expected);
    assert_eq!(
        audio.occurrences.bends_superseded(),
        EventCount::measured(24)
    );
    assert_eq!(audio.test_ingress.counters().dropped_slot(), 0);
    assert_eq!(audio.occurrences.bend_of(s), cents(90.0));

    // A bend in the next quantum does not reach back: each member takes a new entry.
    let next = bend_at(&mut control, s, 130, 10.0);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert!(
        written(outcome_for(&results, next))
            .into_iter()
            .all(|(_, write)| write == MixedBendWrite::Queued)
    );
    assert_eq!(audio.test_ingress.ring_len_for_test(), 24);
    render(&mut audio);
    assert!(audio.test_ingress.is_empty());
    assert_eq!(orphans(&audio), 0);
    let live = sounding(&audio);
    assert!(identities.iter().all(|identity| live.contains(identity)));
}

#[test]
fn a_full_queue_takes_same_quantum_bends_and_refuses_new_slots_whole() {
    let slot = IngressRefused::Dropped {
        resource: ExhaustedResource::Slot,
    };
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (mut control, audio) = armed_with(true, 8);
    let depth = audio.test_ingress.queue_capacity_for_test();
    assert!(depth >= 18 && depth % 2 == 0, "harness depth {depth}");
    let notes: Vec<_> = (0..6)
        .map(|raw| onset_in(&mut control, s, 100, 60 + raw))
        .collect();
    let first = bend_at(&mut control, s, 100, 25.0);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(written(outcome_for(&results, first)).len(), 6);
    // Six note-ons, six bends and six holds; onset/release pairs fill the rest.
    for raw in 0..(depth - 18) / 2 {
        pair(
            &mut control,
            &mut audio,
            other,
            100,
            u8::try_from(raw).expect("small key"),
        );
    }
    assert_eq!(audio.test_ingress.ring_len_for_test(), depth - 6);
    assert_eq!(holds(&audio), 6);

    // The queue is full: an onset with a free hold is refused for its slot.
    let blocked = onset_in(&mut control, other, 100, 100);
    // Every member's entry is in this quantum, so the update needs no slot.
    let same = bend_at(&mut control, s, 110, 50.0);
    // A later quantum needs six new slots and is refused whole.
    let later = bend_at(&mut control, s, 200, 75.0);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(
        outcome_for(&results, onset_request_in(blocked, other, 100, 100)),
        MixedIngressOutcome::Onset(Err(slot))
    );
    assert_eq!(
        written(outcome_for(&results, same)),
        notes
            .iter()
            .map(|note| (*note, MixedBendWrite::Coalesced))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        outcome_for(&results, later),
        MixedIngressOutcome::Bend(Err(MixedBendRefused::Ingress(slot)))
    );
    assert_eq!(audio.occurrences.bend_of(s), cents(50.0));
    assert_eq!(audio.test_ingress.counters().dropped_slot(), 2);
    let bends: Vec<_> = entries(&audio)
        .into_iter()
        .filter(|entry| matches!(entry, Entry::Bend(..)))
        .collect();
    assert_eq!(bends.len(), 6);
    assert!(
        bends
            .iter()
            .all(|entry| matches!(entry, Entry::Bend(_, value, 110) if *value == 50.0))
    );

    // A member's key-up still owns its place, and after the drain the refused value can
    // be sent again.
    release(&mut control, notes[0], 201);
    release(&mut control, blocked, 201);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert!(matches!(
        outcome_for(&results, release_request(notes[0], 201)),
        MixedIngressOutcome::Release(Ok(_))
    ));
    render(&mut audio);
    render(&mut audio);
    assert!(audio.test_ingress.is_empty());
    let retry = bend_at(&mut control, s, 400, 75.0);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(written(outcome_for(&results, retry)).len(), 5);
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
}

#[test]
fn an_onset_whose_inherited_bend_does_not_fit_is_refused_whole() {
    let slot = IngressRefused::Dropped {
        resource: ExhaustedResource::Slot,
    };
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (mut control, audio) = armed_with(true, 2);
    let depth = audio.test_ingress.queue_capacity_for_test();
    assert!(depth % 2 == 0, "harness depth {depth}");
    let set = bend_at(&mut control, s, 100, 40.0);
    let mut audio = service_without_allocation(audio);
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), set)),
        vec![]
    );
    for raw in 0..(depth - 2) / 2 {
        pair(
            &mut control,
            &mut audio,
            other,
            100,
            u8::try_from(raw).expect("small key"),
        );
    }
    // Two units are left: enough for a plain onset and its hold, not for an onset, its
    // inherited bend and its hold.
    let bent = onset_in(&mut control, s, 100, 60);
    let plain = onset_in(&mut control, other, 100, 100);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(
        outcome_for(&results, onset_request_in(bent, s, 100, 60)),
        MixedIngressOutcome::Onset(Err(slot))
    );
    let iplain = accepted(outcome_for(
        &results,
        onset_request_in(plain, other, 100, 100),
    ));
    // Neither the note-on nor its bend was written, and no hold was taken for it.
    assert_eq!(holds(&audio), 1);
    assert_eq!(
        entries(&audio).last(),
        Some(&Entry::On(iplain, 100)),
        "the refused onset wrote nothing"
    );
    assert_eq!(state_of(&audio, bent), MixedOccurrenceState::Refused(slot));
    assert_eq!(audio.test_ingress.counters().dropped_slot(), 1);

    // The tombstone consumes its key-up and is never a bend member.
    release(&mut control, bent, 101);
    let after = bend_at(&mut control, s, 102, 45.0);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(
        outcome_for(&results, release_request(bent, 101)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: bent,
            cause: MixedRetireCause::AfterRefusal(slot),
        }))
    );
    assert_eq!(written(outcome_for(&results, after)), vec![]);
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
}

#[test]
fn tombstones_and_released_occurrences_are_not_bent() {
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 1);
    let held = onset_in(&mut control, s, 100, 60);
    let refused = onset_in(&mut control, s, 100, 61);
    let first = bend_at(&mut control, s, 110, 20.0);
    release(&mut control, held, 111);
    let second = bend_at(&mut control, s, 112, 35.0);
    let reused = onset_in(&mut control, s, 113, 60);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let iheld = accepted(outcome_for(&results, onset_request_in(held, s, 100, 60)));
    assert_eq!(
        outcome_for(&results, onset_request_in(refused, s, 100, 61)),
        MixedIngressOutcome::Onset(Err(hold))
    );
    assert_eq!(
        written(outcome_for(&results, first)),
        vec![(held, MixedBendWrite::Queued)]
    );
    // The released note and the refusal tombstone are not members; the released note's
    // queued bend keeps its value and stamp ahead of its note-off.
    assert_eq!(written(outcome_for(&results, second)), vec![]);
    let ireused = accepted(outcome_for(&results, onset_request_in(reused, s, 113, 60)));
    assert_ne!(ireused, iheld);

    let (_, panic) = control
        .submit_panic(ORIGIN, SampleTime::new(120))
        .expect("protected place");
    let third = bend_at(&mut control, s, 121, 50.0);
    release(&mut control, reused, 130);
    release(&mut control, refused, 131);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert!(matches!(
        outcome_for(&results, panic),
        MixedIngressOutcome::Panic(Ok(_))
    ));
    // The Panic tombstone is not a member either.
    assert_eq!(written(outcome_for(&results, third)), vec![]);
    assert_eq!(
        outcome_for(&results, release_request(reused, 130)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: reused,
            cause: MixedRetireCause::AfterPanic(ireused),
        }))
    );
    assert_eq!(
        outcome_for(&results, release_request(refused, 131)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: refused,
            cause: MixedRetireCause::AfterRefusal(hold),
        }))
    );
    assert_eq!(
        entries(&audio),
        vec![
            Entry::On(iheld, 100),
            Entry::Bend(iheld, 20.0, 110),
            Entry::Off(iheld, 111),
            Entry::On(ireused, 113),
            Entry::Bend(ireused, 35.0, 113),
            Entry::Group(ReleaseCause::Panic, vec![ireused]),
        ]
    );
    assert_eq!(audio.occurrences.bend_of(s), cents(50.0));
    assert_eq!(audio.occurrences.bends_superseded(), EventCount::NONE);
    assert!(audio.occurrences.occupied().is_empty());
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
}

#[test]
fn a_full_bend_table_refuses_a_new_scope_whole_and_a_zero_bend_needs_no_cell() {
    let (mut control, mut audio) = armed_with(true, 1);
    let width = control.ingress_capacity.as_usize();
    let mut serve = |control: &mut MixedOneShotControl, request: MixedIngressRequest| {
        audio.service_test_ingress_queue();
        outcome_for(&collect_all(control), request)
    };
    for source in 0..width {
        let request = bend_at(&mut control, scope(10 + source as u64, 1), 100, 5.0);
        assert_eq!(written(serve(&mut control, request)), vec![]);
    }
    let extra = scope(999, 1);
    let request = bend_at(&mut control, extra, 100, 5.0);
    assert_eq!(
        serve(&mut control, request),
        MixedIngressOutcome::Bend(Err(MixedBendRefused::BendTableFull))
    );
    let request = bend_at(&mut control, extra, 100, 0.0);
    assert_eq!(written(serve(&mut control, request)), vec![]);
    // Returning one scope to zero frees its cell for the new scope.
    let request = bend_at(&mut control, scope(10, 1), 100, 0.0);
    assert_eq!(written(serve(&mut control, request)), vec![]);
    let request = bend_at(&mut control, extra, 100, 5.0);
    assert_eq!(written(serve(&mut control, request)), vec![]);
    drop(serve);
    assert_eq!(audio.occurrences.bend_of(extra), cents(5.0));
    assert_eq!(audio.occurrences.bend_of(scope(10, 1)), Cents::ZERO);
}

#[test]
fn a_full_command_ring_refuses_a_bend_at_the_caller_boundary() {
    let s = scope(1, 1);
    let (mut control, _audio) = armed_with(true, 1);
    for _ in 0..control.ingress_capacity.as_usize() {
        let _request = bend_at(&mut control, s, 100, 5.0);
    }
    assert_eq!(control.ingress_room(), 0);
    assert_eq!(
        control.submit_bend(ORIGIN, s, SampleTime::new(100), cents(6.0)),
        Err(MixedBendSubmitError::Full)
    );
}
