//! Sustain pedal, sustained key-up and lift tests for the private mixed owner.
//!
//! Every test asserts the outstanding release holds after each step: a key-up under
//! the pedal redeems nothing, and only the scope's lift ends a sustained occurrence.

use synth_core::MidiChannel;

use super::{
    super::{
        super::MixedPedalSubmitted, MixedLifted, MixedLiveScope, MixedLiveSource, MixedPedalDown,
        MixedPedalRefused, MixedPedalSubmitError,
    },
    ExhaustedResource, IngressRefused, LiveOccurrenceId, MixedCollection, MixedHandleRefused,
    MixedIngressOutcome, MixedIngressRequest, MixedIngressResult, MixedOccurrenceState,
    MixedOneShotArmError, MixedOneShotAudio, MixedOneShotControl, MixedReleaseSubmitError,
    MixedRetireCause, MixedRetired, NoteIdentity, ORIGIN, Resolution, accepted, collect_all, forge,
    free_units, history_tests, key, outcome_for, release, release_request,
    service_without_allocation,
};
use crate::{
    quantities::{ChannelLayout, EventCount, HeldNoteCount, NoteVelocity},
    render::{AudioBlockMut, EventPayload, NoteEdge},
    time::SampleTime,
};

pub(super) fn armed_with(
    compiled_first: bool,
    holds: u32,
) -> (MixedOneShotControl, MixedOneShotAudio) {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        compiled_first,
        HeldNoteCount::measured(holds),
        EventCount::measured(holds),
    );
    prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .expect("private arm")
}

pub(super) fn scope(source: u64, channel: u8) -> MixedLiveScope {
    MixedLiveScope::new(MixedLiveSource::new(source), MidiChannel::new(channel))
}

pub(super) fn onset_in(
    control: &mut MixedOneShotControl,
    scope: MixedLiveScope,
    at: u64,
    raw: u8,
) -> LiveOccurrenceId {
    let (_, request) = control
        .submit_onset(
            ORIGIN,
            scope,
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

pub(super) fn onset_request_in(
    occurrence: LiveOccurrenceId,
    scope: MixedLiveScope,
    at: u64,
    raw: u8,
) -> MixedIngressRequest {
    MixedIngressRequest::Onset {
        origin: ORIGIN,
        occurrence,
        scope,
        at: SampleTime::new(at),
        key: key(raw),
        velocity: NoteVelocity::FULL,
    }
}

pub(super) fn pedal(
    control: &mut MixedOneShotControl,
    scope: MixedLiveScope,
    at: u64,
    down: bool,
) -> MixedPedalSubmitted {
    control
        .submit_pedal(ORIGIN, scope, SampleTime::new(at), down)
        .expect("pedal edge fits")
}

pub(super) fn pedal_request(scope: MixedLiveScope, at: u64, down: bool) -> MixedIngressRequest {
    MixedIngressRequest::Pedal {
        origin: ORIGIN,
        scope,
        at: SampleTime::new(at),
        down,
    }
}

pub(super) fn holds(audio: &MixedOneShotAudio) -> u32 {
    audio.test_ingress.holds_outstanding().get()
}

fn lifted(outcome: MixedIngressOutcome) -> Vec<LiveOccurrenceId> {
    let MixedIngressOutcome::Lift(Ok(lifted)) = outcome else {
        panic!("lift must apply, got {outcome:?}");
    };
    lifted.retired().collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Queued {
    On(NoteIdentity),
    Off(NoteIdentity),
    Group(Vec<NoteIdentity>),
}

/// Live entries still waiting in the registered ingress queue, in queue order.
pub(super) fn queued(audio: &MixedOneShotAudio) -> Vec<Queued> {
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
            EventPayload::Note {
                identity,
                edge: NoteEdge::On { .. },
            } => Some(Queued::On(identity)),
            EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            } => Some(Queued::Off(identity)),
            EventPayload::ReleaseGroup(group) => Some(Queued::Group(group.identities().collect())),
            _ => None,
        })
        .collect()
}

pub(super) fn render(audio: &mut MixedOneShotAudio) {
    let mut samples = [0.0_f32; 256];
    audio
        .render_private(AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("block"))
        .expect("mixed render");
}

pub(super) fn sounding(audio: &MixedOneShotAudio) -> Vec<NoteIdentity> {
    audio
        .audio
        .renderer
        .snapshot_mixed_sounding()
        .expect("registry")
        .live()
        .iter()
        .map(|note| note.identity)
        .collect()
}

pub(super) fn state_of(
    audio: &MixedOneShotAudio,
    occurrence: LiveOccurrenceId,
) -> MixedOccurrenceState {
    audio
        .occurrences
        .occupied()
        .into_iter()
        .find_map(|(held, state)| (held == occurrence).then_some(state))
        .expect("the handle is held")
}

#[test]
fn a_key_up_under_the_pedal_keeps_its_hold_until_the_lift() {
    for compiled_first in [true, false] {
        let s = scope(1, 1);
        let (mut control, audio) = armed_with(compiled_first, 2);
        assert!(matches!(
            pedal(&mut control, s, 130, true),
            MixedPedalSubmitted::Command(..)
        ));
        let note = onset_in(&mut control, s, 140, 60);
        release(&mut control, note, 150);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(
            outcome_for(&results, pedal_request(s, 130, true)),
            MixedIngressOutcome::PedalDown(Ok(MixedPedalDown::Pressed))
        );
        let identity = accepted(outcome_for(&results, onset_request_in(note, s, 140, 60)));
        assert_eq!(
            outcome_for(&results, release_request(note, 150)),
            MixedIngressOutcome::Sustained(note)
        );
        // The key-up wrote no edge and redeemed nothing.
        assert_eq!(holds(&audio), 1);
        assert_eq!(queued(&audio), vec![Queued::On(identity)]);
        assert_eq!(
            state_of(&audio, note),
            MixedOccurrenceState::Sustained(identity)
        );
        assert_eq!(audio.audio.live_minter.resolve(identity), Resolution::Live);
        assert!(control.occurrence_ledger.is_sustained(note));
        assert_eq!(control.release_reserved.as_usize(), 0);
        assert_eq!(control.pedal_ledger.reserved(), 1);

        // Neither the merger nor audio accepts a second key-up for the handle.
        assert_eq!(
            control.submit_release(ORIGIN, note, SampleTime::new(160)),
            Err(MixedReleaseSubmitError::NotOpen(note))
        );
        forge(&mut control, release_request(note, 160));
        audio.service_test_ingress_queue();
        assert_eq!(
            outcome_for(&collect_all(&mut control), release_request(note, 160)),
            MixedIngressOutcome::Handle(MixedHandleRefused::Sustained)
        );
        assert_eq!(holds(&audio), 1);

        render(&mut audio);
        assert!(
            sounding(&audio).contains(&identity),
            "sustained note sounds"
        );

        assert!(matches!(
            pedal(&mut control, s, 300, false),
            MixedPedalSubmitted::Command(..)
        ));
        assert_eq!(control.pedal_ledger.reserved(), 0);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(
            lifted(outcome_for(&results, pedal_request(s, 300, false))),
            vec![note]
        );
        assert_eq!(holds(&audio), 0);
        assert_eq!(queued(&audio), vec![Queued::Group(vec![identity])]);
        assert!(matches!(
            audio.audio.live_minter.resolve(identity),
            Resolution::Orphan(_)
        ));
        assert!(audio.occurrences.occupied().is_empty());
        assert!(audio.occurrences.pedals_down().is_empty());
        assert_eq!(control.occurrence_ledger.held(), 0);
        assert!(!control.pedal_ledger.is_down(s));
        assert_eq!(control.ingress_outstanding.as_usize(), 0);

        render(&mut audio);
        assert!(!sounding(&audio).contains(&identity), "the lift ended it");
    }
}

#[test]
fn a_lift_releases_exactly_the_prior_sustained_members_of_its_scope() {
    let (a1, a2, b1) = (scope(1, 1), scope(1, 2), scope(2, 1));
    let (mut control, audio) = armed_with(true, 6);
    for s in [a1, a2, b1] {
        assert!(matches!(
            pedal(&mut control, s, 100, true),
            MixedPedalSubmitted::Command(..)
        ));
    }
    let x = onset_in(&mut control, a1, 110, 60);
    let held = onset_in(&mut control, a1, 111, 61);
    let y = onset_in(&mut control, a2, 112, 60);
    let z = onset_in(&mut control, b1, 113, 60);
    for note in [x, y, z] {
        release(&mut control, note, 120);
    }
    assert!(matches!(
        pedal(&mut control, a1, 130, false),
        MixedPedalSubmitted::Command(..)
    ));
    // Stamped after the lift: an ordinary release.
    let late = onset_in(&mut control, a1, 140, 62);
    release(&mut control, late, 150);
    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let id = |occurrence, scope, at, raw| {
        accepted(outcome_for(
            &results,
            onset_request_in(occurrence, scope, at, raw),
        ))
    };
    let (ix, iheld, iy, iz, ilate) = (
        id(x, a1, 110, 60),
        id(held, a1, 111, 61),
        id(y, a2, 112, 60),
        id(z, b1, 113, 60),
        id(late, a1, 140, 62),
    );
    for note in [x, y, z] {
        assert_eq!(
            outcome_for(&results, release_request(note, 120)),
            MixedIngressOutcome::Sustained(note)
        );
    }
    assert_eq!(
        lifted(outcome_for(&results, pedal_request(a1, 130, false))),
        vec![x]
    );
    assert_eq!(
        outcome_for(&results, release_request(late, 150)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: late,
            cause: MixedRetireCause::Released(ilate),
        }))
    );
    assert_eq!(
        queued(&audio),
        vec![
            Queued::On(ix),
            Queued::On(iheld),
            Queued::On(iy),
            Queued::On(iz),
            Queued::Group(vec![ix]),
            Queued::On(ilate),
            Queued::Off(ilate),
        ]
    );
    assert_eq!(
        state_of(&audio, held),
        MixedOccurrenceState::Sounding(iheld)
    );
    assert_eq!(state_of(&audio, y), MixedOccurrenceState::Sustained(iy));
    assert_eq!(state_of(&audio, z), MixedOccurrenceState::Sustained(iz));
    assert_eq!(holds(&audio), 3);
    assert_eq!(audio.occurrences.pedals_down(), vec![a2, b1]);

    // Each remaining scope's lift ends only its own member; the held key ends itself.
    let mut audio = audio;
    pedal(&mut control, b1, 200, false);
    audio.service_test_ingress_queue();
    assert_eq!(
        lifted(outcome_for(
            &collect_all(&mut control),
            pedal_request(b1, 200, false)
        )),
        vec![z]
    );
    assert_eq!(holds(&audio), 2);
    pedal(&mut control, a2, 210, false);
    release(&mut control, held, 220);
    audio.service_test_ingress_queue();
    let results = collect_all(&mut control);
    assert_eq!(
        lifted(outcome_for(&results, pedal_request(a2, 210, false))),
        vec![y]
    );
    assert_eq!(
        outcome_for(&results, release_request(held, 220)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: held,
            cause: MixedRetireCause::Released(iheld),
        }))
    );
    assert_eq!(holds(&audio), 0);
    assert!(audio.occurrences.occupied().is_empty());
    assert_eq!(control.occurrence_ledger.held(), 0);
    assert_eq!(control.pedal_ledger.reserved(), 0);
}

#[test]
fn repeated_keys_under_the_pedal_stay_distinct_occurrences() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(false, 4);
    pedal(&mut control, s, 100, true);
    let first = onset_in(&mut control, s, 110, 60);
    release(&mut control, first, 111);
    let second = onset_in(&mut control, s, 112, 60);
    release(&mut control, second, 113);
    let third = onset_in(&mut control, s, 114, 60);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ifirst = accepted(outcome_for(&results, onset_request_in(first, s, 110, 60)));
    let isecond = accepted(outcome_for(&results, onset_request_in(second, s, 112, 60)));
    let ithird = accepted(outcome_for(&results, onset_request_in(third, s, 114, 60)));
    assert_ne!(ifirst, isecond);
    assert_ne!(isecond, ithird);
    assert_ne!(ifirst, ithird);
    assert_eq!(holds(&audio), 3);
    // The merger still holds both sustained cells until a lift retires them.
    assert!(control.occurrence_ledger.is_sustained(first));
    assert!(control.occurrence_ledger.is_sustained(second));
    assert_eq!(control.occurrence_ledger.held(), 3);

    pedal(&mut control, s, 120, false);
    audio.service_test_ingress_queue();
    assert_eq!(
        lifted(outcome_for(
            &collect_all(&mut control),
            pedal_request(s, 120, false)
        )),
        vec![first, second]
    );
    assert_eq!(holds(&audio), 1);
    assert_eq!(
        state_of(&audio, third),
        MixedOccurrenceState::Sounding(ithird)
    );
    assert_eq!(
        queued(&audio).last(),
        Some(&Queued::Group(vec![ifirst, isecond]))
    );
    render(&mut audio);
    let live = sounding(&audio);
    assert!(live.contains(&ithird) && !live.contains(&ifirst) && !live.contains(&isecond));

    // With the pedal up, the third key's own release ends it.
    release(&mut control, third, 300);
    audio.service_test_ingress_queue();
    assert_eq!(
        outcome_for(&collect_all(&mut control), release_request(third, 300)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: third,
            cause: MixedRetireCause::Released(ithird),
        }))
    );
    assert_eq!(holds(&audio), 0);
    assert_eq!(control.occurrence_ledger.held(), 0);
}

#[test]
fn a_refused_onset_tombstone_is_consumed_under_the_pedal_and_never_ends_a_newer_note() {
    let order = IngressRefused::NonMonotoneStamp {
        time: SampleTime::new(140),
        last: SampleTime::new(150),
    };
    for compiled_first in [true, false] {
        let s = scope(1, 1);
        let (mut control, audio) = armed_with(compiled_first, 2);
        pedal(&mut control, s, 100, true);
        let peer = onset_in(&mut control, s, 150, 61);
        let refused = onset_in(&mut control, s, 140, 60);
        let newer = onset_in(&mut control, s, 150, 60);
        release(&mut control, refused, 150);
        release(&mut control, newer, 150);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        assert_eq!(
            outcome_for(&results, onset_request_in(refused, s, 140, 60)),
            MixedIngressOutcome::Onset(Err(order))
        );
        let ipeer = accepted(outcome_for(&results, onset_request_in(peer, s, 150, 61)));
        let inewer = accepted(outcome_for(&results, onset_request_in(newer, s, 150, 60)));
        // The tombstone's key-up is consumed and retires its cell even under the pedal.
        assert_eq!(
            outcome_for(&results, release_request(refused, 150)),
            MixedIngressOutcome::Release(Ok(MixedRetired {
                occurrence: refused,
                cause: MixedRetireCause::AfterRefusal(order),
            }))
        );
        assert_eq!(
            outcome_for(&results, release_request(newer, 150)),
            MixedIngressOutcome::Sustained(newer)
        );
        assert_eq!(
            audio.occurrences.released_after_refusal(),
            EventCount::measured(1)
        );
        assert_eq!(holds(&audio), 2);
        assert_eq!(
            state_of(&audio, newer),
            MixedOccurrenceState::Sustained(inewer)
        );

        // The lift ends the newer same-key note and leaves the held peer.
        pedal(&mut control, s, 160, false);
        audio.service_test_ingress_queue();
        assert_eq!(
            lifted(outcome_for(
                &collect_all(&mut control),
                pedal_request(s, 160, false)
            )),
            vec![newer]
        );
        assert_eq!(
            state_of(&audio, peer),
            MixedOccurrenceState::Sounding(ipeer)
        );
        assert_eq!(holds(&audio), 1);
    }
}

#[test]
fn a_sustained_hold_refuses_new_onsets_until_the_lift_redeems_it() {
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 2);
    pedal(&mut control, s, 100, true);
    let first = onset_in(&mut control, s, 110, 61);
    let second = onset_in(&mut control, s, 110, 62);
    let refused = onset_in(&mut control, s, 110, 60);
    release(&mut control, refused, 110);
    release(&mut control, first, 111);
    release(&mut control, second, 111);
    // Both keys are up, but the pedal keeps both holds: this onset is refused.
    let starved = onset_in(&mut control, s, 112, 63);
    pedal(&mut control, s, 113, false);
    let mut audio = service_without_allocation(audio);
    let mut results = collect_all(&mut control);
    let (ifirst, isecond) = (
        accepted(outcome_for(&results, onset_request_in(first, s, 110, 61))),
        accepted(outcome_for(&results, onset_request_in(second, s, 110, 62))),
    );
    assert_eq!(
        queued(&audio),
        vec![
            Queued::On(ifirst),
            Queued::On(isecond),
            Queued::Group(vec![ifirst, isecond]),
        ]
    );
    // The lift returned both holds; ordinary room, `C - B = 2H`, frees as the two
    // note-ons and the group drain.
    render(&mut audio);
    let after = onset_in(&mut control, s, 114, 60);
    let audio = service_without_allocation(audio);
    results.extend(collect_all(&mut control));
    assert_eq!(
        outcome_for(&results, onset_request_in(refused, s, 110, 60)),
        MixedIngressOutcome::Onset(Err(hold))
    );
    assert_eq!(
        outcome_for(&results, release_request(refused, 110)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: refused,
            cause: MixedRetireCause::AfterRefusal(hold),
        }))
    );
    assert_eq!(
        outcome_for(&results, onset_request_in(starved, s, 112, 63)),
        MixedIngressOutcome::Onset(Err(hold))
    );
    assert_eq!(
        lifted(outcome_for(&results, pedal_request(s, 113, false))),
        vec![first, second]
    );
    let iafter = accepted(outcome_for(&results, onset_request_in(after, s, 114, 60)));
    assert_eq!(holds(&audio), 1);
    assert_eq!(queued(&audio), vec![Queued::On(iafter)]);
}

#[test]
fn redundant_pedal_edges_are_consumed_at_the_merger_and_reserve_nothing() {
    let s = scope(3, 4);
    let (mut control, audio) = armed_with(true, 2);
    let capacity = control.ingress_capacity.as_usize();
    assert_eq!(
        pedal(&mut control, s, 100, false),
        MixedPedalSubmitted::Redundant
    );
    assert_eq!(control.pedal_ledger.redundant(), EventCount::measured(1));
    assert_eq!(control.ingress_room(), capacity);

    assert!(matches!(
        pedal(&mut control, s, 101, true),
        MixedPedalSubmitted::Command(..)
    ));
    assert_eq!(control.pedal_ledger.reserved(), 1);
    assert_eq!(control.ingress_room(), capacity - 2);
    assert_eq!(
        pedal(&mut control, s, 102, true),
        MixedPedalSubmitted::Redundant
    );
    assert_eq!(control.ingress_room(), capacity - 2);

    assert!(matches!(
        pedal(&mut control, s, 103, false),
        MixedPedalSubmitted::Command(..)
    ));
    assert_eq!(control.pedal_ledger.reserved(), 0);
    assert_eq!(control.ingress_room(), capacity - 2);
    assert_eq!(
        pedal(&mut control, s, 104, false),
        MixedPedalSubmitted::Redundant
    );
    // A down while the lift is still in flight is effective again.
    assert!(matches!(
        pedal(&mut control, s, 105, true),
        MixedPedalSubmitted::Command(..)
    ));
    assert_eq!(control.pedal_ledger.redundant(), EventCount::measured(3));

    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(
        results
            .iter()
            .map(|result| result.outcome)
            .collect::<Vec<_>>(),
        vec![
            MixedIngressOutcome::PedalDown(Ok(MixedPedalDown::Pressed)),
            MixedIngressOutcome::Lift(Ok(MixedLifted::NONE)),
            MixedIngressOutcome::PedalDown(Ok(MixedPedalDown::Pressed)),
        ]
    );
    assert!(control.pedal_ledger.is_down(s));
    assert_eq!(control.pedal_ledger.reserved(), 1);
    assert_eq!(control.ingress_room(), capacity - 1);
    assert_eq!(audio.occurrences.pedals_down(), vec![s]);
    assert_eq!(holds(&audio), 0);
}

#[test]
fn a_full_command_ring_refuses_a_new_pedal_down_but_never_a_key_up_or_lift() {
    let (s, other) = (scope(1, 1), scope(1, 2));
    let (mut control, audio) = armed_with(true, 6);
    let capacity = control.ingress_capacity.as_usize();
    let mut submitted = Vec::new();
    let MixedPedalSubmitted::Command(id, request) = pedal(&mut control, s, 100, true) else {
        panic!("the first pedal-down is effective");
    };
    submitted.push((id, request));
    let mut notes = Vec::new();
    for raw in 0..(capacity - 2) / 2 {
        let raw = u8::try_from(raw).expect("small key");
        let (id, request) = control
            .submit_onset(
                ORIGIN,
                s,
                SampleTime::new(110),
                key(raw),
                NoteVelocity::FULL,
            )
            .expect("onset fits");
        let MixedIngressRequest::Onset { occurrence, .. } = request else {
            panic!("onset request");
        };
        submitted.push((id, request));
        notes.push(occurrence);
    }
    assert_eq!(control.ingress_room(), 0);
    assert_eq!(
        control.submit_pedal(ORIGIN, other, SampleTime::new(111), true),
        Err(MixedPedalSubmitError::Full)
    );
    // The refused down changed no merger state, so its scope's up is redundant.
    assert!(!control.pedal_ledger.is_down(other));
    assert_eq!(
        pedal(&mut control, other, 111, false),
        MixedPedalSubmitted::Redundant
    );
    // Every key-up and the lift still fit their reserved places.
    for note in &notes {
        submitted.push(
            control
                .submit_release(ORIGIN, *note, SampleTime::new(120))
                .expect("reserved release place"),
        );
    }
    let MixedPedalSubmitted::Command(id, request) = pedal(&mut control, s, 130, false) else {
        panic!("the lift is effective");
    };
    submitted.push((id, request));
    assert_eq!(submitted.len(), capacity);
    assert_eq!(control.ingress_outstanding.as_usize(), capacity);

    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    // Results return in submission order, one per command.
    assert_eq!(
        results
            .iter()
            .map(|result| (result.id, result.request))
            .collect::<Vec<_>>(),
        submitted
    );
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    let mut identities = Vec::new();
    for (index, note) in notes.iter().enumerate() {
        let onset = results[1 + index].outcome;
        let key_up = results[1 + notes.len() + index].outcome;
        if index < 6 {
            let MixedIngressOutcome::Onset(Ok(identity)) = onset else {
                panic!("the first six onsets take the six holds");
            };
            assert_eq!(key_up, MixedIngressOutcome::Sustained(*note));
            identities.push((*note, identity));
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
    let MixedIngressResult { outcome, .. } = results[capacity - 1];
    let (handles, members): (Vec<_>, Vec<_>) = identities.into_iter().unzip();
    assert_eq!(lifted(outcome), handles);
    assert_eq!(queued(&audio).last(), Some(&Queued::Group(members)));
    assert_eq!(holds(&audio), 0);
    assert_eq!(control.occurrence_ledger.held(), 0);
    assert_eq!(control.ingress_room(), capacity);
}

#[test]
fn a_lift_needs_no_ingress_place_when_the_queue_is_full_and_every_hold_is_out() {
    let (s, other) = (scope(1, 1), scope(2, 1));
    let (mut control, mut audio) = armed_with(true, 6);
    let depth = audio.test_ingress.queue_capacity_for_test();
    let budget = audio.test_ingress.mixed_bend_budget();
    // The bend-credit budget leaves exactly the six notes' entries and holds as ordinary
    // room: `B = C - 2H`.
    assert_eq!(depth - budget, 12);
    pedal(&mut control, s, 110, true);
    let mut notes = Vec::new();
    for raw in 0..6 {
        let note = onset_in(&mut control, s, 120, 60 + raw);
        release(&mut control, note, 121);
        notes.push((note, raw));
    }
    let starved = onset_in(&mut control, other, 122, 100);
    audio.service_test_ingress_queue();
    let results = collect_all(&mut control);
    assert_eq!(holds(&audio), 6);
    assert_eq!(audio.test_ingress.len(), 6);
    // Entries and outstanding holds fill ordinary room, and every hold is out.
    assert_eq!(free_units(&audio), 0);
    assert_eq!(
        outcome_for(&results, onset_request_in(starved, other, 122, 100)),
        MixedIngressOutcome::Onset(Err(IngressRefused::Dropped {
            resource: ExhaustedResource::Hold,
        }))
    );
    let identities = notes
        .iter()
        .map(|(note, raw)| {
            (
                *note,
                accepted(outcome_for(
                    &results,
                    onset_request_in(*note, s, 120, 60 + raw),
                )),
            )
        })
        .collect::<Vec<_>>();

    pedal(&mut control, s, 130, false);
    audio.service_test_ingress_queue();
    assert_eq!(
        lifted(outcome_for(
            &collect_all(&mut control),
            pedal_request(s, 130, false)
        )),
        identities.iter().map(|(note, _)| *note).collect::<Vec<_>>()
    );
    // Six reservations became one group entry.
    assert_eq!(holds(&audio), 0);
    assert_eq!(audio.test_ingress.len(), 7);
    assert_eq!(free_units(&audio), 5);
    assert_eq!(
        queued(&audio).last(),
        Some(&Queued::Group(
            identities.iter().map(|(_, identity)| *identity).collect()
        ))
    );
    render(&mut audio);
    let live = sounding(&audio);
    assert!(
        identities
            .iter()
            .all(|(_, identity)| !live.contains(identity))
    );
    assert!(audio.test_ingress.is_empty());
}

#[test]
fn a_lift_key_up_and_down_inside_one_service_interval_apply_in_submission_order() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 4);
    pedal(&mut control, s, 100, true);
    let a = onset_in(&mut control, s, 101, 60);
    release(&mut control, a, 102);
    pedal(&mut control, s, 110, false);
    let b = onset_in(&mut control, s, 111, 61);
    release(&mut control, b, 115);
    pedal(&mut control, s, 120, true);
    let c = onset_in(&mut control, s, 121, 62);
    release(&mut control, c, 122);
    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ia = accepted(outcome_for(&results, onset_request_in(a, s, 101, 60)));
    let ib = accepted(outcome_for(&results, onset_request_in(b, s, 111, 61)));
    let ic = accepted(outcome_for(&results, onset_request_in(c, s, 121, 62)));
    assert_eq!(
        outcome_for(&results, release_request(a, 102)),
        MixedIngressOutcome::Sustained(a)
    );
    assert_eq!(
        lifted(outcome_for(&results, pedal_request(s, 110, false))),
        vec![a]
    );
    assert_eq!(
        outcome_for(&results, release_request(b, 115)),
        MixedIngressOutcome::Release(Ok(MixedRetired {
            occurrence: b,
            cause: MixedRetireCause::Released(ib),
        }))
    );
    assert_eq!(
        outcome_for(&results, release_request(c, 122)),
        MixedIngressOutcome::Sustained(c)
    );
    assert_eq!(
        queued(&audio),
        vec![
            Queued::On(ia),
            Queued::Group(vec![ia]),
            Queued::On(ib),
            Queued::Off(ib),
            Queued::On(ic),
        ]
    );
    assert_eq!(holds(&audio), 1);
    assert_eq!(audio.occurrences.pedals_down(), vec![s]);
    assert_eq!(control.pedal_ledger.reserved(), 1);
}

#[test]
fn a_refused_lift_keeps_the_pedal_down_and_every_member_sustained() {
    let order = IngressRefused::NonMonotoneStamp {
        time: SampleTime::new(140),
        last: SampleTime::new(150),
    };
    let s = scope(1, 1);
    // The refused lift is collected before the next edge.
    let (mut control, audio) = armed_with(true, 2);
    pedal(&mut control, s, 100, true);
    let a = onset_in(&mut control, s, 150, 60);
    release(&mut control, a, 150);
    pedal(&mut control, s, 140, false);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ia = accepted(outcome_for(&results, onset_request_in(a, s, 150, 60)));
    assert_eq!(
        outcome_for(&results, pedal_request(s, 140, false)),
        MixedIngressOutcome::Lift(Err(MixedPedalRefused::Ingress(order)))
    );
    assert_eq!(state_of(&audio, a), MixedOccurrenceState::Sustained(ia));
    assert_eq!(holds(&audio), 1);
    assert_eq!(audio.occurrences.pedals_down(), vec![s]);
    // The merger regained the lift reservation; a repeated down is redundant.
    assert!(control.pedal_ledger.is_down(s));
    assert_eq!(control.pedal_ledger.reserved(), 1);
    assert_eq!(
        pedal(&mut control, s, 160, true),
        MixedPedalSubmitted::Redundant
    );
    pedal(&mut control, s, 170, false);
    audio.service_test_ingress_queue();
    assert_eq!(
        lifted(outcome_for(
            &collect_all(&mut control),
            pedal_request(s, 170, false)
        )),
        vec![a]
    );
    assert_eq!(holds(&audio), 0);
    assert_eq!(control.pedal_ledger.reserved(), 0);

    // A newer down submitted while the refused lift is in flight reconciles on audio.
    let (mut control, audio) = armed_with(true, 2);
    pedal(&mut control, s, 100, true);
    let a = onset_in(&mut control, s, 150, 60);
    release(&mut control, a, 150);
    pedal(&mut control, s, 140, false);
    pedal(&mut control, s, 160, true);
    pedal(&mut control, s, 170, false);
    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ia = accepted(outcome_for(&results, onset_request_in(a, s, 150, 60)));
    assert_eq!(
        outcome_for(&results, pedal_request(s, 140, false)),
        MixedIngressOutcome::Lift(Err(MixedPedalRefused::Ingress(order)))
    );
    assert_eq!(
        outcome_for(&results, pedal_request(s, 160, true)),
        MixedIngressOutcome::PedalDown(Ok(MixedPedalDown::AlreadyDown))
    );
    assert_eq!(
        lifted(outcome_for(&results, pedal_request(s, 170, false))),
        vec![a]
    );
    assert_eq!(
        queued(&audio),
        vec![Queued::On(ia), Queued::Group(vec![ia])]
    );
    assert_eq!(holds(&audio), 0);
    assert!(audio.occurrences.pedals_down().is_empty());
    assert!(!control.pedal_ledger.is_down(s));
    assert_eq!(control.pedal_ledger.reserved(), 0);
    assert_eq!(control.ingress_room(), control.ingress_capacity.as_usize());
}

#[test]
fn a_live_hold_entitlement_beyond_one_lift_group_refuses_arm() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        HeldNoteCount::measured(9),
        EventCount::measured(9),
    );
    let refused = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .expect_err("nine sustained notes cannot share one lift group");
    assert_eq!(
        refused.reason,
        MixedOneShotArmError::SustainGroupCapacity {
            holds: EventCount::measured(9),
            group: 8,
        }
    );
    let (control, _audio) = armed_with(true, 8);
    // The occurrence tables cover the ring plus every sustained hold.
    assert_eq!(
        control.occurrence_ledger.width(),
        control.ingress_capacity.as_usize() + 8
    );
}

#[test]
fn teardown_reports_sustained_occurrences_and_pedals_still_down() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 2);
    pedal(&mut control, s, 100, true);
    let a = onset_in(&mut control, s, 110, 60);
    let b = onset_in(&mut control, s, 111, 61);
    release(&mut control, a, 120);
    let audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    let ia = accepted(outcome_for(&results, onset_request_in(a, s, 110, 60)));
    let ib = accepted(outcome_for(&results, onset_request_in(b, s, 111, 61)));
    let MixedCollection::Ended(teardown) = control.collect(audio).expect("collection") else {
        panic!("a pending one-shot owner ends at collection");
    };
    assert_eq!(
        teardown.ingress.occurrences,
        vec![
            (a, MixedOccurrenceState::Sustained(ia)),
            (b, MixedOccurrenceState::Sounding(ib)),
        ]
    );
    assert_eq!(teardown.ingress.pedals_down, vec![s]);
    assert_eq!(teardown.ingress.holds_outstanding, EventCount::measured(2));
}
