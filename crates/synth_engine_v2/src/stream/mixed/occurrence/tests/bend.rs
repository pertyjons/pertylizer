//! Channel bend tests for the private mixed owner.
//!
//! Bend behavior tests inspect the registered ingress queue after service: a bend
//! appends one entry per sounding or sustained member at its nominal stamp, and an
//! onset on a bent scope writes its inherited bend right after its note-on. No entry is
//! rewritten or coalesced. Each bend reserves `H` credits and a possibly bent onset one
//! before its command identity is minted. Render tests check live bend resolution;
//! result-ring pressure tests check ordered packet custody without a live member.

use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};
use std::sync::atomic::Ordering;

use super::{
    super::{
        super::{
            MixedCollection, MixedCollectionEnd, MixedIngressQueueCount, MixedOneShotRenderError,
            MixedPedalSubmitted, OpaqueQueue,
        },
        MixedBendRefused, MixedBendSubmitError, MixedLiveScope, MixedOnsetSubmitError,
    },
    ExhaustedResource, IngressRefused, LiveOccurrenceId, MixedIngressOutcome, MixedIngressRequest,
    MixedIngressResult, MixedOccurrenceState, MixedOneShotAudio, MixedOneShotControl,
    MixedRetireCause, MixedRetired, NoteIdentity, ORIGIN, accepted, collect_all, free_units,
    history_tests, key, outcome_for, release, release_request, service_without_allocation,
    settled_credits,
    sustain::{
        armed_with, holds, onset_in, onset_request_in, pedal, render, scope, sounding, state_of,
    },
};
use crate::{
    ingress::{MixedBendOrigin, ReleaseCause},
    quantities::{Cents, ChannelLayout, EventCount, HeldNoteCount, NoteVelocity},
    render::{AudioBlockMut, EventPayload, NoteEdge},
    time::SampleTime,
};

pub(super) fn cents(value: f32) -> Cents {
    Cents::new(value).expect("valid bend")
}

/// A count of bend credits.
pub(super) fn credits(count: usize) -> MixedIngressQueueCount {
    MixedIngressQueueCount(count)
}

#[test]
fn full_result_ring_terminally_retains_bends_and_unserved_commands() {
    let (mut control, mut audio) = armed_with(true, 1);
    // Deliberately make the physical result ring smaller than the arm-time
    // reservation to exercise the otherwise unreachable failed-push path.
    let (writer, reader) = HeapRb::<MixedIngressResult>::new(1).split();
    control.ingress_results = OpaqueQueue(reader);
    audio.ingress_results = OpaqueQueue(writer);

    let selected_scope = scope(1, 1);
    let mut submitted = Vec::new();
    for (at, value) in [(100, 10.0), (101, 20.0), (102, 0.0)] {
        submitted.push(
            control
                .submit_bend(ORIGIN, selected_scope, SampleTime::new(at), cents(value))
                .expect("all three bends have reserved command places"),
        );
    }

    audio.service_test_ingress_queue();
    assert_eq!(audio.last_served_ingress_id, Some(submitted[1].0));
    let pending = audio
        .ingress_result_pending
        .as_ref()
        .expect("second result has one owner");
    assert_eq!((pending.id, pending.request), submitted[1]);
    assert!(
        matches!(pending.outcome, MixedIngressOutcome::Bend(Ok(bent)) if bent.executed_at() == SampleTime::new(101))
    );
    assert_eq!(
        audio
            .ingress_commands
            .0
            .try_peek()
            .map(|command| (command.id, command.request)),
        Some(submitted[2])
    );

    assert!(audio.ingress_halt.load(Ordering::Acquire));
    audio.service_test_ingress_queue();
    assert_eq!(audio.last_served_ingress_id, Some(submitted[1].0));
    assert_eq!(
        audio
            .ingress_result_pending
            .as_ref()
            .map(|result| (result.id, result.request)),
        Some(submitted[1])
    );
    assert_eq!(
        audio
            .ingress_commands
            .0
            .try_peek()
            .map(|command| (command.id, command.request)),
        Some(submitted[2])
    );

    let mut observing = [1.0_f32; 128];
    assert_eq!(
        audio
            .render_private(AudioBlockMut::new(&mut observing, 64, ChannelLayout::Stereo).unwrap()),
        Err(MixedOneShotRenderError::IngressResultPush)
    );
    assert!(observing.iter().all(|sample| *sample == 0.0));
    let mut later = [1.0_f32; 64];
    assert_eq!(
        audio.render_private(AudioBlockMut::new(&mut later, 64, ChannelLayout::Mono).unwrap()),
        Err(MixedOneShotRenderError::Faulted)
    );
    assert!(later.iter().all(|sample| *sample == 0.0));

    let MixedCollection::Ended(ended) = control.collect(audio).unwrap() else {
        panic!("result push fault must end the pair");
    };
    assert_eq!(ended.end, MixedCollectionEnd::Faulted);
    assert_eq!(
        ended.report.fault,
        Some(MixedOneShotRenderError::IngressResultPush)
    );
    assert_eq!(
        ended
            .ingress
            .uncollected_results
            .iter()
            .map(|result| (result.id, result.request))
            .collect::<Vec<_>>(),
        vec![submitted[0]]
    );
    assert_eq!(
        ended
            .ingress
            .pending_result
            .as_ref()
            .map(|result| (result.id, result.request)),
        Some(submitted[1])
    );
    assert_eq!(
        ended
            .ingress
            .unserved_commands
            .iter()
            .map(|command| (command.id, command.request))
            .collect::<Vec<_>>(),
        vec![submitted[2]]
    );
    assert_eq!(
        ended
            .ingress
            .terminal_control
            .as_ref()
            .unwrap()
            .outstanding_commands
            .iter()
            .map(|entry| (entry.id, entry.request))
            .collect::<Vec<_>>(),
        submitted
    );
}

#[test]
fn result_push_fault_in_later_quantum_silences_whole_callback() {
    let (mut control, mut audio) = armed_with(true, 1);
    let (writer, reader) = HeapRb::<MixedIngressResult>::new(1).split();
    control.ingress_results = OpaqueQueue(reader);
    audio.ingress_results = OpaqueQueue(writer);

    let mut first_half = [0.0_f32; 100];
    audio
        .render_private(AudioBlockMut::new(&mut first_half, 100, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(audio.audio.renderer.carry_frames(), 28);

    let (_reference_control, mut reference_audio) = armed_with(true, 1);
    let mut reference_first = [0.0_f32; 100];
    reference_audio
        .render_private(AudioBlockMut::new(&mut reference_first, 100, ChannelLayout::Mono).unwrap())
        .unwrap();
    let mut reference_observing = [0.0_f32; 256];
    reference_audio
        .render_private(
            AudioBlockMut::new(&mut reference_observing, 256, ChannelLayout::Mono).unwrap(),
        )
        .unwrap();
    assert!(
        reference_observing[..28]
            .iter()
            .any(|sample| *sample != 0.0)
    );

    for (at, value) in [(300, 10.0), (301, 20.0), (302, 0.0)] {
        let (_id, _request) = control
            .submit_bend(ORIGIN, scope(1, 1), SampleTime::new(at), cents(value))
            .unwrap();
    }
    let mut observing = [1.0_f32; 256];
    assert_eq!(
        audio.render_private(AudioBlockMut::new(&mut observing, 256, ChannelLayout::Mono).unwrap()),
        Err(MixedOneShotRenderError::IngressResultPush)
    );
    assert!(observing.iter().all(|sample| *sample == 0.0));
}

pub(super) fn bend_at(
    control: &mut MixedOneShotControl,
    scope: MixedLiveScope,
    at: u64,
    value: f32,
) -> MixedIngressRequest {
    let (_, request) = control
        .submit_bend(ORIGIN, scope, SampleTime::new(at), cents(value))
        .expect("bend fits a ring place and its credits");
    request
}

pub(super) fn written(outcome: MixedIngressOutcome) -> Vec<LiveOccurrenceId> {
    let MixedIngressOutcome::Bend(Ok(bent)) = outcome else {
        panic!("bend must apply, got {outcome:?}");
    };
    bent.written().collect()
}

#[test]
fn mixed_service_measures_the_horizon_from_the_current_quantum() {
    for direct_service in [false, true] {
        let (mut control, mut audio) = armed_with(true, 1);
        render(&mut audio);
        let clock = audio.audio.renderer.clock();
        let horizon = history_tests::mixed_profile()
            .limits()
            .events()
            .forward_event_horizon();
        let edge = clock.checked_add(horizon).expect("horizon end");
        let request = control
            .submit_bend(ORIGIN, scope(1, 1), edge, cents(5.0))
            .expect("bend command fits")
            .1;
        let beyond = edge
            .checked_add(crate::time::FrameCount::new(1))
            .expect("next sample");
        let refused = control
            .submit_bend(ORIGIN, scope(1, 1), beyond, cents(10.0))
            .expect("second bend command fits")
            .1;

        if direct_service {
            audio.service_test_ingress_queue();
        } else {
            render(&mut audio);
        }
        let results = collect_all(&mut control);
        assert_eq!(written(outcome_for(&results, request)), vec![]);
        assert_eq!(
            outcome_for(&results, refused),
            MixedIngressOutcome::Bend(Err(MixedBendRefused::Ingress(
                IngressRefused::BeyondHorizon {
                    time: beyond,
                    horizon_end: edge,
                }
            )))
        );
        assert_eq!(audio.test_ingress.counters().beyond_horizon(), 1);
    }
}

#[test]
fn malformed_bend_result_stamp_scope_shape_or_members_keeps_command_custody() {
    for fault in 0..5 {
        let (scope_a, scope_b) = (scope(1, 1), scope(2, 1));
        let (mut control, mut audio) = armed_with(true, 3);
        let a = onset_in(&mut control, scope_a, 100, 60);
        let b = onset_in(&mut control, scope_b, 101, 61);
        let c = onset_in(&mut control, scope_a, 102, 62);
        audio.service_test_ingress_queue();
        let _onsets = collect_all(&mut control);
        let request = bend_at(&mut control, scope_a, 120, 30.0);
        let id = control.last_ingress_id.unwrap();
        audio.service_test_ingress_queue();
        let actual = control.ingress_results.0.try_pop().unwrap();
        let MixedIngressOutcome::Bend(Ok(mut bent)) = actual.outcome else {
            panic!("the actual bend applied");
        };
        match fault {
            0 => bent.executed_at = SampleTime::new(121),
            1 => bent.members[0] = Some(b),
            2 => {
                bent.members[0] = None;
                bent.members[1] = Some(a);
            }
            3 => {
                assert_eq!(bent.written().collect::<Vec<_>>(), vec![a, c]);
                bent.members[1] = None;
            }
            4 => {
                assert_eq!(bent.written().collect::<Vec<_>>(), vec![a, c]);
                bent.members[1] = None;
                bent.members[2] = Some(c);
                assert_eq!(bent.written().collect::<Vec<_>>(), vec![a, c]);
            }
            _ => unreachable!(),
        }
        let forged = MixedIngressResult {
            outcome: MixedIngressOutcome::Bend(Ok(bent)),
            ..actual
        };
        let expected_result = forged.fields();
        assert!(audio.ingress_results.0.try_push(forged).is_ok());
        assert!(control.collect_ingress_result_checked().is_err());
        assert_eq!(
            control
                .invalid_ingress_result
                .as_ref()
                .map(MixedIngressResult::fields),
            Some(expected_result)
        );
        assert_eq!(control.ingress_outstanding, credits(1));
        assert_eq!(
            control.ingress_journal.front().map(|packet| packet.id),
            Some(id)
        );
        assert_eq!(
            control.ingress_journal.front().map(|packet| packet.request),
            Some(request)
        );
        assert!(
            control
                .bend_scopes
                .iter()
                .flatten()
                .any(|cell| cell.scope == scope_a && !cell.accepted_nonzero)
        );
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Entry {
    On(NoteIdentity, u64),
    Off(NoteIdentity, u64),
    Bend(NoteIdentity, f32, u64),
    Group(ReleaseCause, Vec<NoteIdentity>),
}

/// Every entry still waiting in the registered ingress queue, in queue order.
pub(super) fn entries(audio: &MixedOneShotAudio) -> Vec<Entry> {
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

/// Every undrained bend entry's event and origin.
pub(super) fn bend_origins(
    audio: &MixedOneShotAudio,
) -> Vec<(crate::render::TimedEvent, MixedBendOrigin)> {
    let mut copied = vec![None; audio.test_ingress.queue_capacity_for_test()];
    let len = audio
        .test_ingress
        .copy_mixed_bend_entries_for_test(&mut copied)
        .expect("bend copy");
    copied.into_iter().take(len).flatten().collect()
}

/// Orphaned note events the renderer counted, once the whole queue has drained.
pub(super) fn orphans(audio: &MixedOneShotAudio) -> u64 {
    assert!(audio.test_ingress.is_empty(), "every entry was published");
    audio.audio.renderer.diagnostics().orphan_note_events()
}

/// Submit, serve and collect one onset/release pair on `scope`.
pub(super) fn pair(
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

/// Pair onsets and releases on `scope` until no further onset fits: an onset needs two
/// free ordinary units, for its note-on and its hold.
pub(super) fn saturate(
    control: &mut MixedOneShotControl,
    audio: &mut MixedOneShotAudio,
    scope: MixedLiveScope,
    at: u64,
) {
    let mut raw = 0_u8;
    while free_units(audio) >= 2 {
        pair(control, audio, scope, at, raw);
        raw += 1;
    }
}

/// Offer one onset the saturated queue must refuse, then its key-up, which the
/// tombstone consumes. Returns the onset's refusal.
pub(super) fn refused_probe(
    control: &mut MixedOneShotControl,
    audio: &mut MixedOneShotAudio,
    scope: MixedLiveScope,
    at: u64,
) -> IngressRefused {
    let probe = onset_in(control, scope, at, 127);
    release(control, probe, at);
    audio.service_test_ingress_queue();
    let results = collect_all(control);
    let MixedIngressOutcome::Onset(Err(reason)) =
        outcome_for(&results, onset_request_in(probe, scope, at, 127))
    else {
        panic!("the saturated queue must refuse the probe onset");
    };
    reason
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
        assert_eq!(written(outcome_for(&results, bend)), vec![down, sustained]);
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
        // The bend reserved four credits, spent one per member and returned the rest.
        let budget = audio.test_ingress.mixed_bend_budget();
        assert_eq!(control.bend_credit_return.pending(), credits(2));
        assert_eq!(settled_credits(&control, &audio), budget - 2);
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
        // The drain returned both written credits.
        assert_eq!(settled_credits(&control, &audio), budget);
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
        assert_eq!(written(outcome_for(&results, after)), vec![down]);
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
        assert_eq!(settled_credits(&control, &audio), budget);
    }
}

#[test]
fn an_onset_inherits_its_scopes_bend_and_a_zero_bend_appends_and_frees_the_scope() {
    for compiled_first in [true, false] {
        let (s, other) = (scope(1, 1), scope(1, 2));
        let (mut control, audio) = armed_with(compiled_first, 4);
        let depth = audio.test_ingress.queue_capacity_for_test();
        let budget = audio.test_ingress.mixed_bend_budget();
        // No member yet: the bend sets the value and the order fence only. Its scope
        // reservation makes the next onset on it reserve a credit; the other scope's
        // onset reserves none.
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
        assert_eq!(
            bend_origins(&audio)
                .into_iter()
                .map(|(_, origin)| origin)
                .collect::<Vec<_>>(),
            vec![MixedBendOrigin::Inherited]
        );
        assert_eq!(holds(&audio), 2);
        // Two note-ons and two holds take ordinary room; the inherited bend a credit.
        assert_eq!(free_units(&audio), depth - budget - 4);
        assert_eq!(settled_credits(&control, &audio), budget - 1);

        // Returning to zero one frame later appends its own entry: the inherited value
        // applies from the onset until 101. The zero's result is not collected when the
        // later onset is submitted, so that onset reserves a credit it does not spend.
        let zero = bend_at(&mut control, s, 101, 0.0);
        let later = onset_in(&mut control, s, 102, 62);
        audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(written(outcome_for(&results, zero)), vec![bent]);
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
        // The collected zero freed the reservation: a later onset needs no credit.
        assert!(!control.scope_may_bend(s));
        assert_eq!(free_units(&audio), depth - budget - 6);
        assert_eq!(settled_credits(&control, &audio), budget - 2);
        render(&mut audio);
        assert_eq!(orphans(&audio), 0);
        assert_eq!(settled_credits(&control, &audio), budget);
    }
}

#[test]
fn a_later_bend_appends_after_an_intervening_onsets_inherited_entry() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 3);
    let budget = audio.test_ingress.mixed_bend_budget();
    let first = onset_in(&mut control, s, 100, 60);
    let initial = bend_at(&mut control, s, 110, 30.0);
    let second = onset_in(&mut control, s, 115, 62);
    let later = bend_at(&mut control, s, 120, 70.0);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ifirst = accepted(outcome_for(&results, onset_request_in(first, s, 100, 60)));
    let isecond = accepted(outcome_for(&results, onset_request_in(second, s, 115, 62)));
    assert_eq!(written(outcome_for(&results, initial)), vec![first]);
    // The later bend appends one entry per member at its own stamp. The second note's
    // inherited entry keeps its value at the onset stamp; nothing applies early.
    assert_eq!(written(outcome_for(&results, later)), vec![first, second]);
    assert_eq!(
        entries(&audio),
        vec![
            Entry::On(ifirst, 100),
            Entry::Bend(ifirst, 30.0, 110),
            Entry::On(isecond, 115),
            Entry::Bend(isecond, 30.0, 115),
            Entry::Bend(ifirst, 70.0, 120),
            Entry::Bend(isecond, 70.0, 120),
        ]
    );
    assert_eq!(
        bend_origins(&audio)
            .into_iter()
            .map(|(_, origin)| origin)
            .collect::<Vec<_>>(),
        vec![
            MixedBendOrigin::Ordinary,
            MixedBendOrigin::Inherited,
            MixedBendOrigin::Ordinary,
            MixedBendOrigin::Ordinary,
        ]
    );
    let _free = free_units(&audio);
    assert_eq!(settled_credits(&control, &audio), budget - 4);
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
    assert_eq!(settled_credits(&control, &audio), budget);
}

#[test]
fn a_bend_needs_no_ordinary_unit_when_no_onset_fits() {
    let slot = IngressRefused::Dropped {
        resource: ExhaustedResource::Slot,
    };
    let (s, filler) = (scope(1, 1), scope(3, 1));
    let (mut control, audio) = armed_with(true, 3);
    let first = onset_in(&mut control, s, 100, 60);
    let mut audio = service_without_allocation(audio);
    let ifirst = accepted(outcome_for(
        &collect_all(&mut control),
        onset_request_in(first, s, 100, 60),
    ));
    saturate(&mut control, &mut audio, filler, 100);
    assert_eq!(refused_probe(&mut control, &mut audio, filler, 100), slot);
    let free = free_units(&audio);
    assert!(free < 2);
    let len = audio.test_ingress.ring_len_for_test();
    let later = bend_at(&mut control, s, 110, 50.0);
    audio = service_without_allocation(audio);
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), later)),
        vec![first]
    );
    // The bend took a new entry charged to a credit, not to ordinary room.
    assert_eq!(audio.test_ingress.ring_len_for_test(), len + 1);
    assert!(entries(&audio).ends_with(&[Entry::Bend(ifirst, 50.0, 110)]));
    assert_eq!(free_units(&audio), free);
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
        assert_eq!(written(outcome_for(&results, bend)), vec![first]);
        // The first note's key-up preceded the second bend, so only the second note is a
        // member, and it takes a new entry after its inherited one. The first note keeps
        // the value it had when it was released.
        assert_eq!(written(outcome_for(&results, again)), vec![second]);
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
                Entry::Bend(isecond, 30.0, 200),
                Entry::Off(ifirst, 200),
                Entry::Bend(isecond, 70.0, 200),
            ]
        );

        // A bend behind the order fence is refused at its nominal stamp. It
        // cannot silently take a later place or change the scope's value.
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
        assert_eq!(audio.test_ingress.counters().non_monotone(), 1);
        assert_eq!(audio.occurrences.bend_of(s), cents(70.0));
        assert_eq!(entries(&audio).len(), 6);

        // Each bend resolves after its note-on and before its note-off at one sample.
        render(&mut audio);
        render(&mut audio);
        assert!(audio.test_ingress.is_empty());
        assert_eq!(orphans(&audio), 0);
        assert!(sounding(&audio).contains(&isecond));
    }
}

#[test]
fn late_bends_on_held_and_empty_scopes_keep_their_stamps_and_are_refused() {
    let (held_scope, empty_scope) = (scope(1, 1), scope(2, 1));
    let (mut control, audio) = armed_with(true, 1);
    let note = onset_in(&mut control, held_scope, 200, 60);
    let held_bend = bend_at(&mut control, held_scope, 100, 35.0);
    let empty_bend = bend_at(&mut control, empty_scope, 50, 45.0);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let identity = accepted(outcome_for(
        &results,
        onset_request_in(note, held_scope, 200, 60),
    ));
    for (request, time) in [(held_bend, 100), (empty_bend, 50)] {
        assert_eq!(
            outcome_for(&results, request),
            MixedIngressOutcome::Bend(Err(MixedBendRefused::Ingress(
                IngressRefused::NonMonotoneStamp {
                    time: SampleTime::new(time),
                    last: SampleTime::new(200),
                }
            )))
        );
    }
    assert_eq!(audio.occurrences.bend_of(held_scope), Cents::ZERO);
    assert_eq!(audio.occurrences.bend_of(empty_scope), Cents::ZERO);
    assert!(!control.scope_may_bend(held_scope));
    assert!(!control.scope_may_bend(empty_scope));
    assert_eq!(audio.test_ingress.counters().non_monotone(), 2);
    assert_eq!(entries(&audio), vec![Entry::On(identity, 200)]);
    let _credits = settled_credits(&control, &audio);
    render(&mut audio);
    render(&mut audio);
    assert!(sounding(&audio).contains(&identity));
}

#[test]
fn a_refused_latest_bend_releases_only_a_scope_without_an_accepted_value() {
    let (zero_scope, bent_scope) = (scope(1, 1), scope(2, 1));
    let (mut control, audio) = armed_with(true, 1);
    let zero = bend_at(&mut control, zero_scope, 200, 0.0);
    let late_zero_scope = bend_at(&mut control, zero_scope, 199, 45.0);
    let bent = bend_at(&mut control, bent_scope, 200, 35.0);
    let late_bent_scope = bend_at(&mut control, bent_scope, 199, 0.0);
    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    for request in [zero, bent] {
        assert!(matches!(
            outcome_for(&results, request),
            MixedIngressOutcome::Bend(Ok(_))
        ));
    }
    for request in [late_zero_scope, late_bent_scope] {
        assert_eq!(
            outcome_for(&results, request),
            MixedIngressOutcome::Bend(Err(MixedBendRefused::Ingress(
                IngressRefused::NonMonotoneStamp {
                    time: SampleTime::new(199),
                    last: SampleTime::new(200),
                }
            )))
        );
    }
    assert!(!control.scope_may_bend(zero_scope));
    assert!(control.scope_may_bend(bent_scope));
    assert_eq!(audio.occurrences.bend_of(zero_scope), Cents::ZERO);
    assert_eq!(audio.occurrences.bend_of(bent_scope), cents(35.0));
    let _credits = settled_credits(&control, &audio);
}

/// The value `identity` carries at `frame` from the bends in `published`, in queue order.
fn bend_value_at(published: &[Entry], identity: NoteIdentity, frame: u64) -> Option<f32> {
    published.iter().rev().find_map(|entry| match entry {
        Entry::Bend(bent, value, at) if *bent == identity && *at <= frame => Some(*value),
        _ => None,
    })
}

#[test]
fn the_drained_entry_variant_keeps_one_value_across_its_scope_at_every_sample() {
    let s = scope(1, 1);
    let (mut control, mut audio) = armed_with(true, 2);
    let budget = audio.test_ingress.mixed_bend_budget();
    let old = onset_in(&mut control, s, 100, 60);
    let first = bend_at(&mut control, s, 110, 30.0);
    audio.service_test_ingress_queue();
    let results = collect_all(&mut control);
    let old_identity = accepted(outcome_for(&results, onset_request_in(old, s, 100, 60)));
    assert_eq!(written(outcome_for(&results, first)), vec![old]);
    // Every entry is recorded before it drains, so `published` is the whole history.
    let mut published = entries(&audio);
    render(&mut audio);
    assert!(audio.test_ingress.is_empty());
    assert_eq!(audio.occurrences.bend_of(s), cents(30.0));

    let newer = onset_in(&mut control, s, 300, 61);
    audio.service_test_ingress_queue();
    let new_identity = accepted(outcome_for(
        &collect_all(&mut control),
        onset_request_in(newer, s, 300, 61),
    ));
    assert_eq!(
        entries(&audio),
        vec![
            Entry::On(new_identity, 300),
            Entry::Bend(new_identity, 30.0, 300)
        ]
    );
    // The later bend arrives before the inherited entry drains. It appends one entry
    // per member at 320 and leaves the inherited 30 cents at 300.
    let later = bend_at(&mut control, s, 320, 50.0);
    audio.service_test_ingress_queue();
    let results = collect_all(&mut control);
    assert_eq!(written(outcome_for(&results, later)), vec![old, newer]);
    assert_eq!(
        entries(&audio),
        vec![
            Entry::On(new_identity, 300),
            Entry::Bend(new_identity, 30.0, 300),
            Entry::Bend(old_identity, 50.0, 320),
            Entry::Bend(new_identity, 50.0, 320),
        ]
    );
    published.extend(entries(&audio));
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
    assert_eq!(settled_credits(&control, &audio), budget);

    // Both notes carry one value at every sample both sound, 300 to 319 included.
    for frame in 300..512 {
        assert_eq!(
            bend_value_at(&published, old_identity, frame),
            bend_value_at(&published, new_identity, frame),
            "frame {frame}"
        );
    }
    assert_eq!(bend_value_at(&published, new_identity, 319), Some(30.0));
    assert_eq!(bend_value_at(&published, new_identity, 320), Some(50.0));
}

#[test]
fn wheel_updates_over_a_full_chord_refuse_at_the_caller_until_the_drain_returns_credits() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 8);
    let budget = audio.test_ingress.mixed_bend_budget();
    assert_eq!(budget, 16);
    let notes: Vec<_> = (0..8)
        .map(|raw| onset_in(&mut control, s, 100, 60 + raw))
        .collect();
    let mut audio = service_without_allocation(audio);
    let identities: Vec<_> = collect_all(&mut control)
        .into_iter()
        .map(|result| accepted(result.outcome))
        .collect();
    // Each bend reserves `H` = 8 credits before its command identity is minted, so two
    // wheel messages in one service interval reserve the whole budget.
    let wheel: Vec<_> = [(110, 20.0), (115, -40.0)]
        .into_iter()
        .map(|(at, value)| bend_at(&mut control, s, at, value))
        .collect();
    let last = control.last_ingress_id;
    let exhausted = Err(MixedBendSubmitError::Credits {
        needed: credits(8),
        available: credits(0),
    });
    assert_eq!(
        control.submit_bend(ORIGIN, s, SampleTime::new(120), cents(60.0)),
        exhausted
    );
    assert_eq!(control.last_ingress_id, last);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    for bend in wheel {
        assert_eq!(written(outcome_for(&results, bend)), notes);
    }
    // Each member took one entry per bend at the bend's own stamp.
    let mut expected: Vec<_> = identities
        .iter()
        .map(|identity| Entry::On(*identity, 100))
        .collect();
    for (value, at) in [(20.0, 110), (-40.0, 115)] {
        expected.extend(
            identities
                .iter()
                .map(|identity| Entry::Bend(*identity, value, at)),
        );
    }
    assert_eq!(entries(&audio), expected);
    assert_eq!(audio.test_ingress.counters().dropped_slot(), 0);
    // Every written credit waits for its entry to drain, so the caller is still refused.
    assert_eq!(settled_credits(&control, &audio), 0);
    assert_eq!(
        control.submit_bend(ORIGIN, s, SampleTime::new(120), cents(60.0)),
        exhausted
    );
    assert_eq!(control.last_ingress_id, last);

    // The drain alone returns every credit; no command is served in between.
    render(&mut audio);
    assert!(audio.test_ingress.is_empty());
    assert_eq!(orphans(&audio), 0);
    assert_eq!(control.bend_credit_return.pending(), credits(budget));
    // The source kept its input and resubmits it unchanged; it executes at its stamp.
    let retry = bend_at(&mut control, s, 120, 60.0);
    audio = service_without_allocation(audio);
    let MixedIngressOutcome::Bend(Ok(bent)) = outcome_for(&collect_all(&mut control), retry) else {
        panic!("the retried bend applies");
    };
    assert_eq!(bent.executed_at(), SampleTime::new(120));
    assert_eq!(bent.written().collect::<Vec<_>>(), notes);
    assert_eq!(
        entries(&audio),
        identities
            .iter()
            .map(|identity| Entry::Bend(*identity, 60.0, 120))
            .collect::<Vec<_>>()
    );
    assert_eq!(settled_credits(&control, &audio), budget - 8);
    let live = sounding(&audio);
    assert!(identities.iter().all(|identity| live.contains(identity)));
    let _free = free_units(&audio);
}

#[test]
fn a_saturated_queue_still_appends_every_members_bend() {
    let slot = IngressRefused::Dropped {
        resource: ExhaustedResource::Slot,
    };
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (mut control, audio) = armed_with(true, 8);
    let budget = audio.test_ingress.mixed_bend_budget();
    let notes: Vec<_> = (0..6)
        .map(|raw| onset_in(&mut control, s, 100, 60 + raw))
        .collect();
    let first = bend_at(&mut control, s, 100, 25.0);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let identities: Vec<_> = notes
        .iter()
        .zip(60..)
        .map(|(note, raw)| accepted(outcome_for(&results, onset_request_in(*note, s, 100, raw))))
        .collect();
    assert_eq!(written(outcome_for(&results, first)), notes);
    // Ordinary onset/release pairs saturate the queue: an onset with a free hold is
    // refused for its slot.
    saturate(&mut control, &mut audio, other, 100);
    assert_eq!(refused_probe(&mut control, &mut audio, other, 100), slot);
    assert_eq!(holds(&audio), 6);

    // Every member's earlier entry is undrained, and a later bend appends a new entry
    // per member at its own stamp: no ordinary unit is spent and nothing applies early.
    let free = free_units(&audio);
    let same = bend_at(&mut control, s, 110, 50.0);
    audio = service_without_allocation(audio);
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), same)),
        notes
    );
    assert_eq!(free_units(&audio), free);
    let bends = |audio: &MixedOneShotAudio| {
        entries(audio)
            .into_iter()
            .filter(|entry| matches!(entry, Entry::Bend(..)))
            .collect::<Vec<_>>()
    };
    let mut expected: Vec<_> = identities
        .iter()
        .map(|identity| Entry::Bend(*identity, 25.0, 100))
        .collect();
    expected.extend(
        identities
            .iter()
            .map(|identity| Entry::Bend(*identity, 50.0, 110)),
    );
    assert_eq!(bends(&audio), expected);
    // Twelve credits wait for their entries to drain; a third bend is refused at the
    // caller and keeps its input.
    assert_eq!(settled_credits(&control, &audio), budget - 12);
    assert_eq!(
        control.submit_bend(ORIGIN, s, SampleTime::new(200), cents(75.0)),
        Err(MixedBendSubmitError::Credits {
            needed: credits(8),
            available: credits(budget - 12),
        })
    );
    assert_eq!(audio.occurrences.bend_of(s), cents(50.0));
    assert_eq!(audio.test_ingress.counters().dropped_slot(), 1);

    // After the drain every credit is back. Saturate again: the queue refuses an
    // onset, yet a bend writes every member at its own stamp without a free unit.
    render(&mut audio);
    render(&mut audio);
    assert!(audio.test_ingress.is_empty());
    assert_eq!(settled_credits(&control, &audio), budget);
    saturate(&mut control, &mut audio, other, 520);
    assert_eq!(refused_probe(&mut control, &mut audio, other, 520), slot);
    let free = free_units(&audio);
    assert!(free < 6, "fewer free units than members");
    let retry = bend_at(&mut control, s, 530, -25.0);
    audio = service_without_allocation(audio);
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), retry)),
        notes
    );
    assert_eq!(free_units(&audio), free);
    assert_eq!(audio.test_ingress.counters().dropped_slot(), 2);
    assert_eq!(
        bends(&audio),
        identities
            .iter()
            .map(|identity| Entry::Bend(*identity, -25.0, 530))
            .collect::<Vec<_>>()
    );
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
    let live = sounding(&audio);
    assert!(identities.iter().all(|identity| live.contains(identity)));
}

#[test]
fn an_onset_takes_two_ordinary_units_and_its_inherited_bend_one_credit() {
    let slot = IngressRefused::Dropped {
        resource: ExhaustedResource::Slot,
    };
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (mut control, audio) = armed_with(true, 2);
    let budget = audio.test_ingress.mixed_bend_budget();
    let set = bend_at(&mut control, s, 100, 40.0);
    let mut audio = service_without_allocation(audio);
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), set)),
        vec![]
    );
    saturate(&mut control, &mut audio, other, 100);
    // No ordinary unit is left: neither a bent onset nor a plain one fits, because
    // each needs its note-on and its hold.
    assert_eq!(free_units(&audio), 0);
    let bent = onset_in(&mut control, s, 100, 60);
    let plain = onset_in(&mut control, other, 100, 100);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(
        outcome_for(&results, onset_request_in(bent, s, 100, 60)),
        MixedIngressOutcome::Onset(Err(slot))
    );
    assert_eq!(
        outcome_for(&results, onset_request_in(plain, other, 100, 100)),
        MixedIngressOutcome::Onset(Err(slot))
    );
    assert_eq!(holds(&audio), 0);
    assert_eq!(free_units(&audio), 0);
    assert_eq!(state_of(&audio, bent), MixedOccurrenceState::Refused(slot));
    assert_eq!(audio.test_ingress.counters().dropped_slot(), 2);
    // The refused bent onset returned its unused credit at service.
    assert_eq!(settled_credits(&control, &audio), budget);

    // The tombstone consumes its key-up and is never a bend member.
    release(&mut control, bent, 101);
    release(&mut control, plain, 101);
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

    // With two units left, a bent onset takes both for its note-on and hold and one
    // credit for its inherited bend. A plain onset then finds no unit and is refused.
    pair(&mut control, &mut audio, other, 300, 0);
    assert_eq!(free_units(&audio), 2);
    let bent = onset_in(&mut control, s, 300, 60);
    let plain = onset_in(&mut control, other, 300, 100);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ibent = accepted(outcome_for(&results, onset_request_in(bent, s, 300, 60)));
    assert_eq!(
        outcome_for(&results, onset_request_in(plain, other, 300, 100)),
        MixedIngressOutcome::Onset(Err(slot))
    );
    assert!(entries(&audio).ends_with(&[Entry::On(ibent, 300), Entry::Bend(ibent, 45.0, 300)]));
    assert_eq!(audio.test_ingress.mixed_bend_queued(), 1);
    assert_eq!(free_units(&audio), 0);
    assert_eq!(settled_credits(&control, &audio), budget - 1);
    release(&mut control, plain, 301);
    audio.service_test_ingress_queue();
    let _results = collect_all(&mut control);
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
    assert_eq!(settled_credits(&control, &audio), budget);
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
    let mut audio = service_without_allocation(audio);
    let mut results = collect_all(&mut control);
    let iheld = accepted(outcome_for(&results, onset_request_in(held, s, 100, 60)));
    assert_eq!(
        outcome_for(&results, onset_request_in(refused, s, 100, 61)),
        MixedIngressOutcome::Onset(Err(hold))
    );
    assert_eq!(written(outcome_for(&results, first)), vec![held]);
    // The released note and the refusal tombstone are not members; the released note's
    // queued bend keeps its value and stamp ahead of its note-off.
    assert_eq!(written(outcome_for(&results, second)), vec![]);
    assert_eq!(
        entries(&audio),
        vec![
            Entry::On(iheld, 100),
            Entry::Bend(iheld, 20.0, 110),
            Entry::Off(iheld, 111),
        ]
    );
    // One hold leaves two ordinary units, which the note-on and note-off still take.
    render(&mut audio);
    let reused = onset_in(&mut control, s, 113, 60);
    audio = service_without_allocation(audio);
    results.extend(collect_all(&mut control));
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
            Entry::On(ireused, 113),
            Entry::Bend(ireused, 35.0, 113),
            Entry::Group(ReleaseCause::Panic, vec![ireused]),
        ]
    );
    assert_eq!(audio.occurrences.bend_of(s), cents(50.0));
    assert!(audio.occurrences.occupied().is_empty());
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
}

#[test]
fn two_source_bend_scope_floor_covers_all_channels_at_shallow_ingress_depth() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(
            candidate,
            &history_tests::profile_with_ingress_depth(EventCount::measured(5)),
        )
        .unwrap();
    assert_eq!(control.ingress_capacity.as_usize(), 5);
    assert_eq!(control.bend_scopes.len(), 32);
    assert_eq!(audio.occurrences.bends.len(), 32);
    for source in 1..=2 {
        for channel in 1..=16 {
            let scope = scope(source, channel);
            let request = bend_at(&mut control, scope, 100, 5.0);
            audio.service_test_ingress_queue();
            assert_eq!(
                written(outcome_for(&collect_all(&mut control), request)),
                vec![]
            );
            assert_eq!(audio.occurrences.bend_of(scope), cents(5.0));
        }
    }
    assert_eq!(control.bend_scopes.iter().flatten().count(), 32);
    let first = scope(1, 1);
    let zero = bend_at(&mut control, first, 100, 0.0);
    audio.service_test_ingress_queue();
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), zero)),
        vec![]
    );
    assert_eq!(audio.occurrences.bend_of(first), Cents::ZERO);
    assert_eq!(control.bend_scopes.iter().flatten().count(), 31);
}

#[test]
fn a_full_bend_table_refuses_at_the_caller_and_frees_only_after_a_zero_result() {
    let (mut control, mut audio) = armed_with(true, 1);
    let width = control.bend_scopes.len();
    let mut serve = |control: &mut MixedOneShotControl, request: MixedIngressRequest| {
        audio.service_test_ingress_queue();
        outcome_for(&collect_all(control), request)
    };
    for source in 0..width {
        let request = bend_at(&mut control, scope(10 + source as u64, 1), 100, 5.0);
        assert_eq!(written(serve(&mut control, request)), vec![]);
    }
    let extra = scope(999, 1);
    let last = control.last_ingress_id;
    assert_eq!(
        control.submit_bend(ORIGIN, extra, SampleTime::new(100), cents(5.0)),
        Err(MixedBendSubmitError::ScopeCapacity)
    );
    assert_eq!(control.last_ingress_id, last);
    assert_eq!(control.bend_scopes.iter().flatten().count(), width);
    drop(serve);
    // A forged internal request bypasses the typed caller reservation. Audio still
    // refuses it without changing the table or a queued entry.
    let forged = MixedIngressRequest::Bend {
        origin: ORIGIN,
        scope: extra,
        at: SampleTime::new(100),
        cents: cents(5.0),
    };
    let _forged_command = control.submit_ingress(forged).expect("forged command fits");
    audio.service_test_ingress_queue();
    assert_eq!(
        outcome_for(&collect_all(&mut control), forged),
        MixedIngressOutcome::Bend(Err(MixedBendRefused::BendTableFull))
    );
    assert_eq!(audio.occurrences.bend_of(extra), Cents::ZERO);
    for source in 0..width {
        assert_eq!(
            audio.occurrences.bend_of(scope(10 + source as u64, 1)),
            cents(5.0)
        );
    }
    assert!(entries(&audio).is_empty());
    let request = bend_at(&mut control, extra, 100, 0.0);
    audio.service_test_ingress_queue();
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), request)),
        vec![]
    );
    // The zero is in the command ring, then on audio, but the caller's cell stays
    // reserved until its successful result is collected.
    let zero = bend_at(&mut control, scope(10, 1), 100, 0.0);
    assert_eq!(
        control.submit_bend(ORIGIN, extra, SampleTime::new(100), cents(5.0)),
        Err(MixedBendSubmitError::ScopeCapacity)
    );
    audio.service_test_ingress_queue();
    assert_eq!(
        control.submit_bend(ORIGIN, extra, SampleTime::new(100), cents(5.0)),
        Err(MixedBendSubmitError::ScopeCapacity)
    );
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), zero)),
        vec![]
    );
    let request = bend_at(&mut control, extra, 100, 5.0);
    audio.service_test_ingress_queue();
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), request)),
        vec![]
    );
    assert_eq!(audio.occurrences.bend_of(extra), cents(5.0));
    assert_eq!(audio.occurrences.bend_of(scope(10, 1)), Cents::ZERO);
}

#[test]
fn a_refused_zero_keeps_its_scope_reservation() {
    let (mut control, mut audio) = armed_with(true, 1);
    let width = control.bend_scopes.len();
    for source in 0..width {
        let held = scope(10 + source as u64, 1);
        let request = bend_at(&mut control, held, 100, 5.0);
        audio.service_test_ingress_queue();
        assert_eq!(
            written(outcome_for(&collect_all(&mut control), request)),
            vec![]
        );
    }
    let held = scope(10, 1);
    let extra = scope(999, 1);
    let refused = bend_at(&mut control, held, u64::MAX, 0.0);
    audio.service_test_ingress_queue();
    assert!(matches!(
        outcome_for(&collect_all(&mut control), refused),
        MixedIngressOutcome::Bend(Err(MixedBendRefused::Ingress(
            IngressRefused::BeyondHorizon { .. }
        )))
    ));
    assert_eq!(audio.occurrences.bend_of(held), cents(5.0));
    assert_eq!(control.bend_scopes.iter().flatten().count(), width);
    assert_eq!(
        control.submit_bend(ORIGIN, extra, SampleTime::new(100), cents(5.0)),
        Err(MixedBendSubmitError::ScopeCapacity)
    );
}

#[test]
fn a_zero_result_does_not_release_a_scope_with_a_later_bend_pending() {
    let (mut control, mut audio) = armed_with(true, 1);
    let width = control.bend_scopes.len();
    for source in 0..width {
        let scope = scope(10 + source as u64, 1);
        let request = bend_at(&mut control, scope, 100, 5.0);
        audio.service_test_ingress_queue();
        assert_eq!(
            written(outcome_for(&collect_all(&mut control), request)),
            vec![]
        );
    }
    let held = scope(10, 1);
    let extra = scope(999, 1);
    let zero = bend_at(&mut control, held, 100, 0.0);
    let again = bend_at(&mut control, held, 100, 7.0);
    audio.service_test_ingress_queue();
    let first = control.collect_ingress_result().expect("zero result");
    assert_eq!(first.request, zero);
    assert_eq!(written(first.outcome), vec![]);
    assert_eq!(audio.occurrences.bend_of(held), cents(7.0));
    assert_eq!(
        control.submit_bend(ORIGIN, extra, SampleTime::new(100), cents(5.0)),
        Err(MixedBendSubmitError::ScopeCapacity)
    );
    assert_eq!(
        written(outcome_for(&collect_all(&mut control), again)),
        vec![]
    );
    assert_eq!(audio.occurrences.bend_of(held), cents(7.0));
}

#[test]
fn a_full_command_ring_refuses_a_bend_at_the_caller_boundary() {
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (mut control, _audio) = armed_with(true, 1);
    // Unbent onsets fill the ring, each with its reserved release place, and reserve no
    // bend credit.
    for raw in 0..control.ingress_capacity.as_usize() / 2 {
        let _note = onset_in(
            &mut control,
            other,
            100,
            u8::try_from(raw).expect("small key"),
        );
    }
    assert_eq!(control.ingress_room(), 0);
    let (last, free) = (control.last_ingress_id, control.bend_credits_free);
    assert_eq!(
        control.submit_bend(ORIGIN, s, SampleTime::new(100), cents(6.0)),
        Err(MixedBendSubmitError::Full)
    );
    // The refusal minted no identity and reserved no credit or scope cell.
    assert_eq!(control.last_ingress_id, last);
    assert_eq!(control.bend_credits_free, free);
    assert!(!control.scope_may_bend(s));
}

#[test]
fn at_minimum_depth_an_inflight_bend_refuses_a_bent_onset_until_credits_return() {
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(
            candidate,
            &history_tests::profile_with_ingress_depth(EventCount::measured(5)),
        )
        .expect("depth 2H + 1 arms");
    // `B = max(H, C - 2H) = H`: one bend reserves the whole budget.
    assert_eq!(audio.test_ingress.mixed_bend_budget(), 2);
    let held = onset_in(&mut control, s, 100, 60);
    audio.service_test_ingress_queue();
    let iheld = accepted(outcome_for(
        &collect_all(&mut control),
        onset_request_in(held, s, 100, 60),
    ));
    render(&mut audio);

    let bend = bend_at(&mut control, s, 300, 30.0);
    // The bent scope's onset needs a credit and none is free: the caller is refused
    // before a handle or command identity is minted, and keeps its input.
    let (last, open) = (control.last_ingress_id, control.occurrence_ledger.held());
    assert_eq!(
        control.submit_onset(
            ORIGIN,
            s,
            SampleTime::new(300),
            key(100),
            NoteVelocity::FULL
        ),
        Err(MixedOnsetSubmitError::BendCredit)
    );
    assert_eq!(control.last_ingress_id, last);
    assert_eq!(control.occurrence_ledger.held(), open);
    // An onset on an unbent scope needs no credit and is admitted.
    let plain = onset_in(&mut control, other, 300, 62);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(written(outcome_for(&results, bend)), vec![held]);
    let iplain = accepted(outcome_for(
        &results,
        onset_request_in(plain, other, 300, 62),
    ));
    assert_eq!(
        entries(&audio),
        vec![Entry::Bend(iheld, 30.0, 300), Entry::On(iplain, 300)]
    );
    // The bend spent one credit; audio returned the unused one when it served it.
    assert_eq!(control.bend_credit_return.pending(), credits(1));
    assert_eq!(settled_credits(&control, &audio), 1);
    assert_eq!(free_units(&audio), 0);
    release(&mut control, plain, 300);
    audio.service_test_ingress_queue();
    let _released = collect_all(&mut control);

    // The drain alone returns the written credit; no other command is served.
    render(&mut audio);
    assert!(audio.test_ingress.is_empty());
    assert_eq!(control.bend_credit_return.pending(), credits(2));
    // The resubmitted onset reserves a credit and inherits the scope's bend.
    let bent = onset_in(&mut control, s, 300, 100);
    audio = service_without_allocation(audio);
    let ibent = accepted(outcome_for(
        &collect_all(&mut control),
        onset_request_in(bent, s, 300, 100),
    ));
    assert_eq!(
        entries(&audio),
        vec![Entry::On(ibent, 300), Entry::Bend(ibent, 30.0, 300)]
    );
    assert_eq!(settled_credits(&control, &audio), 1);
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
    assert_eq!(settled_credits(&control, &audio), 2);
}

#[test]
fn panic_applies_with_the_ordinary_room_and_the_credit_budget_full() {
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (mut control, audio) = armed_with(true, 8);
    let budget = audio.test_ingress.mixed_bend_budget();
    let notes: Vec<_> = (0..8)
        .map(|raw| onset_in(&mut control, s, 100, 60 + raw))
        .collect();
    let mut audio = service_without_allocation(audio);
    let identities: Vec<_> = collect_all(&mut control)
        .into_iter()
        .map(|result| accepted(result.outcome))
        .collect();
    let wheel = [
        bend_at(&mut control, s, 110, 20.0),
        bend_at(&mut control, s, 115, 40.0),
    ];
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    for bend in wheel {
        assert_eq!(written(outcome_for(&results, bend)), notes);
    }
    // Undrained bend entries hold every credit, and the eight note-ons and holds fill
    // ordinary room.
    assert_eq!(settled_credits(&control, &audio), 0);
    assert_eq!(free_units(&audio), 0);
    let last = control.last_ingress_id;
    assert_eq!(
        control.submit_bend(ORIGIN, s, SampleTime::new(120), cents(60.0)),
        Err(MixedBendSubmitError::Credits {
            needed: credits(8),
            available: credits(0),
        })
    );
    assert_eq!(control.last_ingress_id, last);
    // Unbent onsets, each with its reserved release place, fill the command ring.
    for raw in 0..control.ingress_room() / 2 {
        let _note = onset_in(
            &mut control,
            other,
            120,
            u8::try_from(raw).expect("small key"),
        );
    }
    assert_eq!(control.ingress_room(), 0);

    let (_, panic) = control
        .submit_panic(ORIGIN, SampleTime::new(130))
        .expect("the protected place is free");
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let MixedIngressOutcome::Panic(Ok(panicked)) = outcome_for(&results, panic) else {
        panic!("Panic must apply");
    };
    assert_eq!(panicked.executed_at(), SampleTime::new(130));
    assert_eq!(panicked.ended().count(), 8);
    for result in &results {
        if let MixedIngressOutcome::Onset(outcome) = result.outcome {
            assert_eq!(outcome, Err(hold));
        }
    }
    assert_eq!(holds(&audio), 0);
    let Some(Entry::Group(ReleaseCause::Panic, members)) = entries(&audio).pop() else {
        panic!("the Panic group is queued last");
    };
    assert_eq!(members.len(), 8);
    assert!(identities.iter().all(|identity| members.contains(identity)));
    // Panic spent no credit; the drain returns them all.
    assert_eq!(settled_credits(&control, &audio), 0);
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
    assert_eq!(settled_credits(&control, &audio), budget);
}
