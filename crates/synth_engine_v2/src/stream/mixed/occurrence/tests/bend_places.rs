//! The bend-credit budget of the private mixed owner.
//!
//! Bend entries are charged to a fixed budget withheld from ordinary ingress traffic.
//! These tests check `(len - queued bends) + holds + budget <= capacity` after each
//! step through [`free_units`], and that every credit is free at the merger, waiting in
//! audio's return counter or held by one undrained entry through [`settled_credits`].
//! They check that a retired note's undrained entry keeps its credit until it drains,
//! that every later bend appends its own entry after an inherited one, that a bend
//! served before the drain sounds at its own stamp under every host partition, and
//! that no admitted bend is lost or rewritten under a long scripted command storm.

use super::{
    super::{MixedBendSubmitError, MixedLiveScope, MixedPanicEnd},
    LiveOccurrenceId, MixedIngressOutcome, MixedIngressRequest, MixedOccurrenceState,
    MixedOneShotAudio, MixedOneShotControl, MixedRetireCause, MixedRetired, NoteIdentity, ORIGIN,
    accepted,
    bend::{Entry, bend_at, bend_origins, credits, entries, orphans, written},
    collect_all, free_units, key, outcome_for, release, release_request,
    service_without_allocation, settled_credits,
    sustain::{armed_with, holds, onset_in, onset_request_in, pedal, pedal_request, render, scope},
};
use crate::{
    ingress::{IngressRefused, MixedBendOrigin, ReleaseCause},
    quantities::{Cents, ChannelLayout, NoteVelocity},
    render::AudioBlockMut,
    time::SampleTime,
};

/// Undrained credit-charged bend entries, after checking both invariants.
fn queued_bends(control: &MixedOneShotControl, audio: &MixedOneShotAudio) -> usize {
    let _free = free_units(audio);
    let _credits = settled_credits(control, audio);
    audio.test_ingress.mixed_bend_queued()
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
fn a_retired_notes_undrained_bend_entry_keeps_its_credit_until_it_drains() {
    for case in [Retirement::Release, Retirement::Lift, Retirement::Panic] {
        let (bent_scope, plain_scope, kept_scope) = (scope(1, 1), scope(2, 1), scope(3, 1));
        let (mut control, audio) = armed_with(true, 4);
        let depth = audio.test_ingress.queue_capacity();
        let budget = audio.test_ingress.mixed_bend_budget();
        if case == Retirement::Lift {
            pedal(&mut control, bent_scope, 5, true);
            pedal(&mut control, plain_scope, 5, true);
        }
        let bent = onset_in(&mut control, bent_scope, 10, 60);
        let plain = onset_in(&mut control, plain_scope, 10, 61);
        let kept = onset_in(&mut control, kept_scope, 10, 62);
        let bend = bend_at(&mut control, bent_scope, 20, 40.0);
        let mut audio = service_without_allocation(audio);
        let results = collect_all(&mut control);
        let ibent = accepted(outcome_for(
            &results,
            onset_request_in(bent, bent_scope, 10, 60),
        ));
        let iplain = accepted(outcome_for(
            &results,
            onset_request_in(plain, plain_scope, 10, 61),
        ));
        let ikept = accepted(outcome_for(
            &results,
            onset_request_in(kept, kept_scope, 10, 62),
        ));
        assert_eq!(written(outcome_for(&results, bend)), vec![bent]);
        assert_eq!(queued_bends(&control, &audio), 1, "{case:?}");

        // Retire `bent` while its bend entry is undrained, and `plain`, which has none.
        match case {
            Retirement::Release => {
                release(&mut control, bent, 21);
                release(&mut control, plain, 21);
            }
            Retirement::Lift => {
                release(&mut control, bent, 21);
                release(&mut control, plain, 21);
                pedal(&mut control, bent_scope, 22, false);
                pedal(&mut control, plain_scope, 22, false);
            }
            Retirement::Panic => {
                let _panic = control
                    .submit_panic(ORIGIN, SampleTime::new(22))
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
                        outcome_for(&results, release_request(note, 21)),
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
                        outcome_for(&results, pedal_request(scope, 22, false)),
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
        // The retired note's bend is still published before its end, and its entry still
        // holds its credit: retirement returns none.
        let bend_at_position = entries(&audio)
            .into_iter()
            .position(|entry| entry == Entry::Bend(ibent, 40.0, 20))
            .expect("the retired note's bend stays queued");
        assert!(bend_at_position < ended_by(ibent).expect("the note's end is queued"));
        let live = usize::from(case != Retirement::Panic);
        assert_eq!(holds(&audio) as usize, live);
        assert_eq!(
            queued_bends(&control, &audio),
            1,
            "{case:?}: credits at retirement"
        );
        assert_eq!(settled_credits(&control, &audio), budget - 1);

        // The drain alone returns the credit.
        render(&mut audio);
        assert!(audio.test_ingress.is_empty());
        assert_eq!(
            queued_bends(&control, &audio),
            0,
            "{case:?}: credits after the drain"
        );
        assert_eq!(settled_credits(&control, &audio), budget);
        assert_eq!(free_units(&audio), depth - budget - live);
        assert_eq!(orphans(&audio), 0);

        // Panic left key-down tombstones; their key-ups are consumed without an entry.
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
            assert_eq!(queued_bends(&control, &audio), 0);
        } else {
            // The kept note is still a member and takes a new entry at the bend's stamp.
            let again = bend_at(&mut control, kept_scope, 300, -20.0);
            audio = service_without_allocation(audio);
            assert_eq!(
                written(outcome_for(&collect_all(&mut control), again)),
                vec![kept]
            );
            assert_eq!(queued_bends(&control, &audio), 1);
            assert_eq!(entries(&audio), vec![Entry::Bend(ikept, -20.0, 300)]);
        }
    }
}

#[test]
fn unused_credits_return_at_service_and_written_ones_only_at_the_drain_without_allocation() {
    let s = scope(1, 1);
    let (mut control, audio) = armed_with(true, 4);
    let budget = audio.test_ingress.mixed_bend_budget();
    let note = onset_in(&mut control, s, 100, 60);
    let bend = bend_at(&mut control, s, 110, 25.0);
    // The bend carries its four credits; none has returned yet.
    assert_eq!(control.bend_credits_free, credits(budget - 4));
    assert_eq!(control.bend_credit_return.pending(), credits(0));
    // Audio serves on another thread without allocating, and returns the three unused
    // credits through the shared counter at once.
    let audio = service_without_allocation(audio);
    assert_eq!(control.bend_credit_return.pending(), credits(3));
    let results = collect_all(&mut control);
    assert_eq!(written(outcome_for(&results, bend)), vec![note]);
    // Collecting the result returns nothing more: the written credit waits for its entry.
    assert_eq!(control.bend_credit_return.pending(), credits(3));
    assert_eq!(queued_bends(&control, &audio), 1);
    // The drain returns it, on the audio thread, without allocating.
    let audio = std::thread::spawn(move || {
        let mut audio = audio;
        let mut samples = [0.0_f32; 256];
        let allocations = crate::render_allocation::count_allocs(|| {
            audio
                .render_private(
                    AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("block"),
                )
                .expect("mixed render");
        });
        assert_eq!(allocations, 0, "the drain allocated");
        audio
    })
    .join()
    .expect("render thread");
    assert_eq!(control.bend_credit_return.pending(), credits(4));
    assert_eq!(queued_bends(&control, &audio), 0);
    // The merger takes every returned credit at its next reservation.
    let _next = bend_at(&mut control, s, 300, 0.0);
    assert_eq!(control.bend_credit_return.pending(), credits(0));
    assert_eq!(control.bend_credits_free, credits(budget - 4));
}

#[test]
fn every_bend_after_an_inherited_entry_appends_its_own_entry() {
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
    // At the onset's own stamp, later in its quantum and in a later quantum, each bend
    // appends its own entry after the inherited one, which keeps its value and stamp.
    for bend in [same, within, across] {
        assert_eq!(written(outcome_for(&results, bend)), vec![note]);
    }
    assert_eq!(
        entries(&audio),
        vec![
            Entry::On(identity, 100),
            Entry::Bend(identity, 30.0, 100),
            Entry::Bend(identity, 35.0, 100),
            Entry::Bend(identity, 40.0, 120),
            Entry::Bend(identity, 45.0, 200),
        ]
    );
    assert_eq!(
        bend_origins(&audio)
            .into_iter()
            .map(|(_, origin)| origin)
            .collect::<Vec<_>>(),
        vec![
            MixedBendOrigin::Inherited,
            MixedBendOrigin::Ordinary,
            MixedBendOrigin::Ordinary,
            MixedBendOrigin::Ordinary,
        ]
    );
    assert_eq!(queued_bends(&control, &audio), 4);

    // Once drained, every credit is back; a later bend and a new onset's inherited
    // entry each start with their own origin.
    render(&mut audio);
    // This first callback leaves the renderer at 192, before the frame-200 entry.
    assert_eq!(entries(&audio), vec![Entry::Bend(identity, 45.0, 200)]);
    render(&mut audio);
    assert_eq!(queued_bends(&control, &audio), 0);
    let later = bend_at(&mut control, s, 300, 50.0);
    let second = onset_in(&mut control, s, 301, 62);
    audio = service_without_allocation(audio);
    let results = collect_all(&mut control);
    assert_eq!(written(outcome_for(&results, later)), vec![note]);
    let isecond = accepted(outcome_for(&results, onset_request_in(second, s, 301, 62)));
    assert_eq!(
        bend_origins(&audio)
            .into_iter()
            .map(|(event, origin)| (event.envelope().time().as_u64(), origin))
            .collect::<Vec<_>>(),
        vec![
            (300, MixedBendOrigin::Ordinary),
            (301, MixedBendOrigin::Inherited)
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
fn callback_services_bends_at_quantum_open_and_preserves_carry_order() {
    let s = scope(1, 1);
    let (mut control, mut audio) = armed_with(true, 2);
    let onset = onset_in(&mut control, s, 70, 60);
    assert!(control.collect_ingress_result().is_none());

    // The armed renderer begins with one quantum of priming silence. That carry
    // does not service the onset; the clock-zero boundary does.
    let mut actual = render_partition(&mut audio, &[64]);
    assert!(control.collect_ingress_result().is_none());
    actual.extend(render_partition(&mut audio, &[1]));
    assert!(matches!(
        outcome_for(
            &collect_all(&mut control),
            onset_request_in(onset, s, 70, 60)
        ),
        MixedIngressOutcome::Onset(Ok(_))
    ));
    let first = bend_at(&mut control, s, 80, 200.0);
    let second = bend_at(&mut control, s, 81, -200.0);
    actual.extend(render_partition(&mut audio, &[63]));
    assert!(control.collect_ingress_result().is_none());

    actual.extend(render_partition(&mut audio, &[64, 128]));
    let results = collect_all(&mut control);
    for request in [first, second] {
        assert!(matches!(
            outcome_for(&results, request),
            MixedIngressOutcome::Bend(Ok(_))
        ));
    }

    let (mut reference_control, mut reference) = armed_with(true, 2);
    let _onset = onset_in(&mut reference_control, s, 70, 60);
    let _first = bend_at(&mut reference_control, s, 80, 200.0);
    let _second = bend_at(&mut reference_control, s, 81, -200.0);
    reference.service_test_ingress_queue();
    let _results = collect_all(&mut reference_control);
    assert_eq!(actual, render_partition(&mut reference, &[320]));
}

#[test]
fn a_bend_served_before_the_drain_sounds_at_its_own_stamp_under_every_host_partition() {
    let partitions: [&[usize]; 4] = [&[512], &[64; 8], &[1, 63, 100, 200, 148], &[37; 13]];
    let partitions = partitions.map(|partition| {
        let mut partition = partition.to_vec();
        let total: usize = partition.iter().sum();
        if total < 512 {
            partition.push(512 - total);
        }
        partition
    });
    // A wheel at 110 then 200, both serviced before the drain: the second appends its
    // own entry at 200 behind the first's undrained entry at 110.
    let (mut control, mut queued) = bent_owner(&[(110, 200.0)]);
    let late = bend_at(&mut control, scope(1, 1), 200, -300.0);
    queued.service_test_ingress_queue();
    let MixedIngressOutcome::Bend(Ok(bent)) = outcome_for(&collect_all(&mut control), late) else {
        panic!("the bend applies");
    };
    assert_eq!(bent.executed_at(), SampleTime::new(200));
    assert_eq!(bent.written().count(), 1);
    let reference = render_partition(&mut queued, &partitions[0]);
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
    // Served on time, after the drain of 110, the bend sounds the same: queueing it
    // before the drain applied nothing early.
    let (mut control, mut on_time) = bent_owner(&[(110, 200.0)]);
    let mut samples = render_partition(&mut on_time, &[192]);
    let _late = bend_at(&mut control, scope(1, 1), 200, -300.0);
    on_time.service_test_ingress_queue();
    assert!(matches!(
        collect_all(&mut control)[0].outcome,
        MixedIngressOutcome::Bend(Ok(_))
    ));
    samples.extend(render_partition(&mut on_time, &[320]));
    assert_eq!(samples, reference);
    // And the stamp is audible: -300 written at 110 differs between 110 and 200 and
    // nowhere before 110.
    let (_control, mut early) = bent_owner(&[(110, -300.0)]);
    let early = render_partition(&mut early, &partitions[0]);
    assert_eq!(early[..110], reference[..110]);
    assert_ne!(early[110..200], reference[110..200]);
}

#[test]
fn the_bend_credit_invariant_holds_under_a_scripted_command_storm() {
    let scopes: [MixedLiveScope; 3] = [scope(1, 1), scope(1, 2), scope(2, 1)];
    let (mut control, mut audio) = armed_with(true, 8);
    let budget = audio.test_ingress.mixed_bend_budget();
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut draw = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut at = 100_u64;
    let mut open: Vec<LiveOccurrenceId> = Vec::new();
    let (mut bend_entries, mut bend_credit_refusals) = (0, 0);
    let (mut onset_slot_refusals, mut lifts, mut panics) = (0_u64, 0, 0);
    // Submitted bends in command order. One behind a parked Panic is served by a later
    // render rather than by its own step's service.
    let mut pending_bends = std::collections::VecDeque::new();
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
                // A full ring, or a possibly bent scope without a free credit,
                // refuses at the caller.
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
                match control.submit_bend(ORIGIN, s, stamp, value) {
                    Ok((_, request)) => {
                        bend = Some((request, value));
                        pending_bends.push_back((request, value));
                    }
                    Err(MixedBendSubmitError::Credits { needed, .. }) => {
                        assert_eq!(needed, credits(8));
                        bend_credit_refusals += 1;
                    }
                    Err(_) => {}
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
        // Service only appends: every entry already queued keeps its place and value.
        let served_now = audio.parked_panic.is_none();
        let before = entries(&audio);
        audio.service_test_ingress_queue();
        let after = entries(&audio);
        assert!(
            after.starts_with(&before),
            "step {step}: service rewrote the queue"
        );
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
                    let Some((request, value)) = pending_bends.pop_front() else {
                        panic!("step {step}: an unexpected bend result");
                    };
                    assert_eq!(result.request, request);
                    let MixedIngressRequest::Bend { at: bent_at, .. } = request else {
                        panic!("step {step}: a bend result for another request");
                    };
                    // The bend executed at its own stamp.
                    assert_eq!(bent.executed_at(), bent_at);
                    // Served in this step, it appended exactly one entry per member, in
                    // member order, at its own stamp.
                    if served_now && bend == Some((request, value)) {
                        let expected = bent
                            .written()
                            .map(|member| {
                                let identity = identity_of(&audio, member).expect("a live member");
                                Entry::Bend(identity, value.as_f32(), at)
                            })
                            .collect::<Vec<_>>();
                        assert_eq!(
                            after.get(before.len()..),
                            Some(expected.as_slice()),
                            "step {step}"
                        );
                    }
                    bend_entries += bent.written().count();
                }
                _ => {}
            }
        }
        let _free = free_units(&audio);
        // Commands behind a parked Panic stay in flight with their credits.
        if audio.parked_panic.is_none() {
            let _credits = settled_credits(&control, &audio);
        }
    }
    // The storm reached every branch it claims to cover.
    assert!(onset_slot_refusals > 0, "the queue never saturated");
    assert!(bend_entries > 0, "no bend reached a member");
    assert!(bend_credit_refusals > 0, "the credit budget never ran out");
    assert!(lifts > 0 && panics > 0);
    assert_eq!(
        audio.test_ingress.counters().dropped_slot(),
        onset_slot_refusals
    );
    // Let the owner fall quiet: every remaining entry drains, resolves and returns its
    // credit.
    for _ in 0..8 {
        render(&mut audio);
    }
    assert_eq!(orphans(&audio), 0);
    let _free = free_units(&audio);
    assert_eq!(settled_credits(&control, &audio), budget);
}
