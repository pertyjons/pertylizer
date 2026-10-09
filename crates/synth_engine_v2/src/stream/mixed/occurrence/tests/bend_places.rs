//! Precharged bend places for the private mixed owner.
//!
//! Every accepted live onset owns one bend place in the ingress queue until it retires.
//! These tests check the queue invariant `len + holds + reserved places <= capacity`
//! after each step through [`free_units`], that a retired note's undrained entry never
//! returns a place when it drains, that an inherited entry keeps its origin and stamp,
//! that early application sounds exactly as its result describes under every host
//! partition, and that no admitted bend is lost under a long scripted command storm.

use super::{
    super::{MixedLiveScope, MixedPanicEnd},
    LiveOccurrenceId, MixedIngressOutcome, MixedIngressRequest, MixedOccurrenceState,
    MixedOneShotAudio, MixedOneShotControl, MixedRetireCause, MixedRetired, NoteIdentity, ORIGIN,
    accepted,
    bend::{Entry, bend_at, bend_origins, entries, orphans, written},
    collect_all, free_units, key, outcome_for, release, release_request,
    service_without_allocation,
    sustain::{armed_with, holds, onset_in, onset_request_in, pedal, pedal_request, render, scope},
};
use crate::{
    ingress::{IngressRefused, MixedBendOrigin, MixedBendWrite, ReleaseCause},
    quantities::{Cents, ChannelLayout, EventCount, NoteVelocity},
    render::AudioBlockMut,
    time::SampleTime,
};

/// Reserved places and undrained entries holding one, after checking the invariant.
fn places(audio: &MixedOneShotAudio) -> (usize, usize) {
    let _free = free_units(audio);
    let places = audio.test_ingress.mixed_bend_places_for_test();
    (places.reserved, places.queued)
}

/// The identity a sounding or sustained handle names on audio.
fn identity_of(audio: &MixedOneShotAudio, occurrence: LiveOccurrenceId) -> Option<NoteIdentity> {
    audio
        .occurrences
        .occupied()
        .into_iter()
        .find_map(|(held, state)| match state {
            MixedOccurrenceState::Sounding(identity)
            | MixedOccurrenceState::Sustained(identity)
                if held == occurrence =>
            {
                Some(identity)
            }
            _ => None,
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Retirement {
    Release,
    Lift,
    Panic,
}

#[test]
fn retired_notes_never_restore_a_bend_place_when_their_entries_drain() {
    for case in [Retirement::Release, Retirement::Lift, Retirement::Panic] {
        let (bent_scope, plain_scope, kept_scope) = (scope(1, 1), scope(2, 1), scope(3, 1));
        let (mut control, audio) = armed_with(true, 4);
        let depth = audio.test_ingress.queue_capacity_for_test();
        if case == Retirement::Lift {
            pedal(&mut control, bent_scope, 90, true);
            pedal(&mut control, plain_scope, 90, true);
        }
        let bent = onset_in(&mut control, bent_scope, 100, 60);
        let plain = onset_in(&mut control, plain_scope, 100, 61);
        let kept = onset_in(&mut control, kept_scope, 100, 62);
        let bend = bend_at(&mut control, bent_scope, 110, 40.0);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        let ibent = accepted(outcome_for(
            &results,
            onset_request_in(bent, bent_scope, 100, 60),
        ));
        let iplain = accepted(outcome_for(
            &results,
            onset_request_in(plain, plain_scope, 100, 61),
        ));
        let ikept = accepted(outcome_for(
            &results,
            onset_request_in(kept, kept_scope, 100, 62),
        ));
        assert_eq!(
            written(outcome_for(&results, bend)),
            vec![(bent, MixedBendWrite::Queued)]
        );
        assert_eq!(places(&audio), (2, 1), "{case:?}");

        // Retire `bent` while its bend entry is undrained and `plain` while its place is
        // in reserve.
        match case {
            Retirement::Release => {
                release(&mut control, bent, 111);
                release(&mut control, plain, 111);
            }
            Retirement::Lift => {
                release(&mut control, bent, 111);
                release(&mut control, plain, 111);
                pedal(&mut control, bent_scope, 112, false);
                pedal(&mut control, plain_scope, 112, false);
            }
            Retirement::Panic => {
                let _panic = control
                    .submit_panic(ORIGIN, SampleTime::new(112))
                    .expect("protected place");
            }
        }
        audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        let ended_by = |identity| {
            entries(&audio).into_iter().position(|entry| match entry {
                Entry::Off(ended, _) => ended == identity,
                Entry::Group(_, members) => members.contains(&identity),
                Entry::On(..) | Entry::Bend(..) => false,
            })
        };
        match case {
            Retirement::Release => {
                for (note, identity) in [(bent, ibent), (plain, iplain)] {
                    assert_eq!(
                        outcome_for(&results, release_request(note, 111)),
                        MixedIngressOutcome::Release(Ok(MixedRetired {
                            occurrence: note,
                            cause: MixedRetireCause::Released(identity),
                        }))
                    );
                }
            }
            Retirement::Lift => {
                for scope in [bent_scope, plain_scope] {
                    assert!(matches!(
                        outcome_for(&results, pedal_request(scope, 112, false)),
                        MixedIngressOutcome::Lift(Ok(_))
                    ));
                }
                assert!(
                    entries(&audio).contains(&Entry::Group(ReleaseCause::SustainLift, vec![ibent]))
                );
            }
            Retirement::Panic => {
                let Some(MixedIngressOutcome::Panic(Ok(panicked))) = results
                    .iter()
                    .find(|result| matches!(result.request, MixedIngressRequest::Panic { .. }))
                    .map(|result| result.outcome)
                else {
                    panic!("Panic must apply");
                };
                assert_eq!(
                    panicked.ended().collect::<Vec<_>>(),
                    [bent, plain, kept].map(MixedPanicEnd::Tombstoned)
                );
            }
        }
        // The retired note's bend is still published before its end: the value is not
        // lost, but its entry no longer holds a place.
        let bend_at_position = entries(&audio)
            .into_iter()
            .position(|entry| entry == Entry::Bend(ibent, 40.0, 110))
            .expect("the retired note's bend stays queued");
        assert!(bend_at_position < ended_by(ibent).expect("the note's end is queued"));
        let live = usize::from(case != Retirement::Panic);
        assert_eq!(holds(&audio) as usize, live);
        assert_eq!(places(&audio), (live, 0), "{case:?}: places at retirement");

        render(&mut audio);
        assert!(audio.test_ingress.is_empty());
        // A drained entry of a retired note restores nothing.
        assert_eq!(
            places(&audio),
            (live, 0),
            "{case:?}: places after the drain"
        );
        assert_eq!(free_units(&audio), depth - 2 * live);
        assert_eq!(orphans(&audio), 0);

        // Panic left key-down tombstones; their key-ups are consumed without a place.
        if case == Retirement::Panic {
            for note in [bent, plain, kept] {
                release(&mut control, note, 300);
            }
            audio.service_test_ingress_queue();
            let results = collect_all(&mut control);
            for (note, identity) in [(bent, ibent), (plain, iplain), (kept, ikept)] {
                assert_eq!(
                    outcome_for(&results, release_request(note, 300)),
                    MixedIngressOutcome::Release(Ok(MixedRetired {
                        occurrence: note,
                        cause: MixedRetireCause::AfterPanic(identity),
                    }))
                );
            }
            assert_eq!(places(&audio), (0, 0));
        } else {
            // The kept note is still a member and its reserved place still serves it.
            let again = bend_at(&mut control, kept_scope, 300, -20.0);
            audio = service_without_allocation(audio);
            assert_eq!(
                written(outcome_for(&collect_all(&mut control), again)),
                vec![(kept, MixedBendWrite::Queued)]
            );
            assert_eq!(places(&audio), (0, 1));
            assert_eq!(entries(&audio), vec![Entry::Bend(ikept, -20.0, 300)]);
        }
    }
}

#[test]
fn an_inherited_entry_keeps_its_origin_and_onset_stamp_through_every_merge() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 2);
    let set = bend_at(&mut control, s, 100, 30.0);
    let note = onset_in(&mut control, s, 100, 60);
    let same = bend_at(&mut control, s, 100, 35.0);
    let within = bend_at(&mut control, s, 120, 40.0);
    let across = bend_at(&mut control, s, 200, 45.0);
    let mut audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(written(outcome_for(&results, set)), vec![]);
    let identity = accepted(outcome_for(&results, onset_request_in(note, s, 100, 60)));
    let early = MixedBendWrite::Early {
        applies_at: SampleTime::new(100),
        origin: MixedBendOrigin::Inherited,
    };
    // At the onset's own stamp the value is replaced on time; at a later stamp in the
    // same quantum or a later quantum it applies at the onset stamp, said so.
    assert_eq!(
        written(outcome_for(&results, same)),
        vec![(note, MixedBendWrite::Coalesced)]
    );
    assert_eq!(written(outcome_for(&results, within)), vec![(note, early)]);
    assert_eq!(written(outcome_for(&results, across)), vec![(note, early)]);
    assert_eq!(
        entries(&audio),
        vec![Entry::On(identity, 100), Entry::Bend(identity, 45.0, 100)]
    );
    let origins = bend_origins(&audio);
    assert_eq!(origins.len(), 1);
    assert_eq!(
        (origins[0].1, origins[0].2),
        (MixedBendOrigin::Inherited, true)
    );
    assert_eq!(
        audio.occurrences.bends_superseded(),
        EventCount::measured(3)
    );
    assert_eq!(
        audio.occurrences.bends_applied_early(),
        EventCount::measured(2)
    );
    assert_eq!(places(&audio), (0, 1));

    // Once drained, the place serves an ordinary bend at its own stamp, and a new
    // onset's inherited entry starts with its own origin.
    render(&mut audio);
    assert_eq!(places(&audio), (1, 0));
    let later = bend_at(&mut control, s, 300, 50.0);
    let second = onset_in(&mut control, s, 301, 62);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(
        written(outcome_for(&results, later)),
        vec![(note, MixedBendWrite::Queued)]
    );
    let isecond = accepted(outcome_for(&results, onset_request_in(second, s, 301, 62)));
    assert_eq!(
        bend_origins(&audio)
            .into_iter()
            .map(|(event, origin, place)| (event.envelope().time().as_u64(), origin, place))
            .collect::<Vec<_>>(),
        vec![
            (300, MixedBendOrigin::Ordinary, true),
            (301, MixedBendOrigin::Inherited, true)
        ]
    );
    assert_eq!(
        entries(&audio),
        vec![
            Entry::Bend(identity, 50.0, 300),
            Entry::On(isecond, 301),
            Entry::Bend(isecond, 50.0, 301),
        ]
    );
    render(&mut audio);
    assert_eq!(orphans(&audio), 0);
}

/// Render `partition`'s blocks in order and return the samples.
fn render_partition(audio: &mut MixedOneShotAudio, partition: &[usize]) -> Vec<f32> {
    let mut samples = vec![0.0_f32; partition.iter().sum()];
    let mut offset = 0;
    for &frames in partition {
        audio
            .render_private(
                AudioBlockMut::new(
                    &mut samples[offset..offset + frames],
                    frames,
                    ChannelLayout::Mono,
                )
                .expect("block"),
            )
            .expect("mixed render");
        offset += frames;
    }
    samples
}

/// One live note at 100 on a fresh owner, with `bends` serviced before any render.
fn bent_owner(bends: &[(u64, f32)]) -> (MixedOneShotControl, MixedOneShotAudio) {
    let s = scope(1, 1);
    let (mut control, mut audio) = armed_with(true, 2);
    let _note = onset_in(&mut control, s, 100, 60);
    for &(at, value) in bends {
        let _bend = bend_at(&mut control, s, at, value);
    }
    audio.service_test_ingress_queue();
    for result in collect_all(&mut control) {
        assert!(matches!(
            result.outcome,
            MixedIngressOutcome::Onset(Ok(_)) | MixedIngressOutcome::Bend(Ok(_))
        ));
    }
    (control, audio)
}

#[test]
fn an_early_bend_sounds_at_its_described_stamp_under_every_host_partition() {
    let partitions: [&[usize]; 4] = [&[512], &[64; 8], &[1, 63, 100, 200, 148], &[37; 13]];
    let partitions = partitions.map(|partition| {
        let mut partition = partition.to_vec();
        let total: usize = partition.iter().sum();
        if total < 512 {
            partition.push(512 - total);
        }
        partition
    });
    // A wheel at 110 then 200, both serviced before the drain: the second reaches the
    // first's entry, which keeps 110, so -300 applies from 110.
    let (mut control, mut early) = bent_owner(&[(110, 200.0)]);
    let late = bend_at(&mut control, scope(1, 1), 200, -300.0);
    early.service_test_ingress_queue();
    let MixedIngressOutcome::Bend(Ok(bent)) = outcome_for(&collect_all(&mut control), late) else {
        panic!("the bend applies");
    };
    assert_eq!(
        bent.written().map(|(_, write)| write).collect::<Vec<_>>(),
        vec![MixedBendWrite::Early {
            applies_at: SampleTime::new(110),
            origin: MixedBendOrigin::Ordinary,
        }]
    );
    let reference = render_partition(&mut early, &partitions[0]);
    for partition in &partitions[1..] {
        let (mut control, mut owner) = bent_owner(&[(110, 200.0)]);
        let _late = bend_at(&mut control, scope(1, 1), 200, -300.0);
        owner.service_test_ingress_queue();
        let _results = collect_all(&mut control);
        assert_eq!(
            render_partition(&mut owner, partition),
            reference,
            "partition {partition:?}"
        );
    }
    // The described stamp is the whole story: the same as -300 written at 110.
    for partition in &partitions {
        let (_control, mut described) = bent_owner(&[(110, -300.0)]);
        assert_eq!(
            render_partition(&mut described, partition),
            reference,
            "partition {partition:?}"
        );
    }
    // And it is audible: served on time, after the drain of 110, the bend differs
    // between 110 and 200 and nowhere before 110.
    let (mut control, mut on_time) = bent_owner(&[(110, 200.0)]);
    let mut samples = render_partition(&mut on_time, &[192]);
    let _late = bend_at(&mut control, scope(1, 1), 200, -300.0);
    on_time.service_test_ingress_queue();
    assert!(matches!(
        collect_all(&mut control)[0].outcome,
        MixedIngressOutcome::Bend(Ok(_))
    ));
    samples.extend(render_partition(&mut on_time, &[320]));
    assert_eq!(samples[..110], reference[..110]);
    assert_ne!(samples[110..200], reference[110..200]);
}

#[test]
fn the_bend_place_invariant_holds_under_a_scripted_command_storm() {
    let scopes: [MixedLiveScope; 3] = [scope(1, 1), scope(1, 2), scope(2, 1)];
    let (mut control, mut audio) = armed_with(true, 8);
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut draw = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut at = 100_u64;
    let mut open: Vec<LiveOccurrenceId> = Vec::new();
    let (mut queued, mut coalesced, mut inherited_early, mut ordinary_early) = (0, 0, 0, 0);
    let (mut onset_slot_refusals, mut lifts, mut panics) = (0_u64, 0, 0);
    let slot = IngressRefused::Dropped {
        resource: crate::ingress::ExhaustedResource::Slot,
    };
    let hold = IngressRefused::Dropped {
        resource: crate::ingress::ExhaustedResource::Hold,
    };
    for step in 0..4_000 {
        let clock = audio.audio.renderer.clock().as_u64();
        at = at.max(clock) + draw(6);
        if at > clock + 1_024 {
            render(&mut audio);
            continue;
        }
        let s = scopes[usize::try_from(draw(3)).expect("small")];
        let stamp = SampleTime::new(at);
        let mut bend = None;
        match draw(16) {
            0..=4 => {
                let raw = u8::try_from(draw(128)).expect("key");
                if let Ok((_, MixedIngressRequest::Onset { occurrence, .. })) =
                    control.submit_onset(ORIGIN, s, stamp, key(raw), NoteVelocity::FULL)
                {
                    open.push(occurrence);
                }
            }
            5..=8 => {
                if !open.is_empty() {
                    let index = usize::try_from(draw(open.len() as u64)).expect("index");
                    let handle = open.swap_remove(index);
                    if control.occurrence_ledger.is_open(handle) {
                        release(&mut control, handle, at);
                    }
                }
            }
            9..=12 => {
                let value = Cents::new(draw(401) as f32 - 200.0).expect("bend");
                if let Ok((_, request)) = control.submit_bend(ORIGIN, s, stamp, value) {
                    bend = Some((request, value));
                }
            }
            13 => {
                let _edge = control.submit_pedal(ORIGIN, s, stamp, draw(2) == 0);
            }
            14 if draw(6) == 0 => {
                if control.submit_panic(ORIGIN, stamp).is_ok() {
                    panics += 1;
                }
            }
            _ => {
                // Rare drains let ordinary traffic saturate the queue between them.
                if draw(8) == 0 {
                    render(&mut audio);
                    let _free = free_units(&audio);
                }
                continue;
            }
        }
        audio.service_test_ingress_queue();
        for result in collect_all(&mut control) {
            match result.outcome {
                MixedIngressOutcome::Onset(Err(reason)) if reason == slot => {
                    onset_slot_refusals += 1;
                }
                MixedIngressOutcome::Onset(Err(reason)) => {
                    assert_eq!(reason, hold, "step {step}: only shortage refuses an onset");
                }
                MixedIngressOutcome::Release(Err(reason)) => {
                    panic!("step {step}: a release was refused: {reason:?}");
                }
                MixedIngressOutcome::Bend(Err(refused)) => {
                    panic!("step {step}: an admitted bend was refused: {refused:?}");
                }
                MixedIngressOutcome::Lift(Ok(_)) => lifts += 1,
                MixedIngressOutcome::Bend(Ok(bent)) => {
                    let Some((request, value)) = bend else {
                        panic!("step {step}: an unexpected bend result");
                    };
                    assert_eq!(result.request, request);
                    // Every member's latest value waits in the queue, at the stamp the
                    // result names.
                    let waiting = entries(&audio);
                    for (member, write) in bent.written() {
                        let applies = match write {
                            MixedBendWrite::Queued => {
                                queued += 1;
                                at
                            }
                            MixedBendWrite::Coalesced => {
                                coalesced += 1;
                                at
                            }
                            MixedBendWrite::Early { applies_at, origin } => {
                                assert!(applies_at.as_u64() < at);
                                match origin {
                                    MixedBendOrigin::Inherited => inherited_early += 1,
                                    MixedBendOrigin::Ordinary => ordinary_early += 1,
                                }
                                applies_at.as_u64()
                            }
                        };
                        let identity = identity_of(&audio, member).expect("a live member");
                        assert!(
                            waiting.contains(&Entry::Bend(identity, value.as_f32(), applies)),
                            "step {step}: {member:?} lost its bend"
                        );
                    }
                }
                _ => {}
            }
        }
        let _free = free_units(&audio);
    }
    // The storm reached every branch it claims to cover.
    assert!(onset_slot_refusals > 0, "the queue never saturated");
    assert!(queued > 0 && coalesced > 0);
    assert!(inherited_early > 0 && ordinary_early > 0);
    assert!(lifts > 0 && panics > 0);
    assert_eq!(
        audio.test_ingress.counters().dropped_slot(),
        onset_slot_refusals
    );
    assert_eq!(
        audio.occurrences.bends_applied_early().get(),
        u32::try_from(inherited_early + ordinary_early).expect("count")
    );
    assert_eq!(
        audio.occurrences.bends_superseded().get(),
        u32::try_from(coalesced + inherited_early + ordinary_early).expect("count")
    );
    // Let the owner fall quiet: every remaining entry drains and resolves.
    for _ in 0..8 {
        render(&mut audio);
    }
    assert_eq!(orphans(&audio), 0);
    let _free = free_units(&audio);
}
