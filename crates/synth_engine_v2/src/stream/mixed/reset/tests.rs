//! The private whole-owner reset (draft rules 3.2, 3.3 and 3.6).
//!
//! The old owner is armed at 64 from plan position 10, stops at 192 and holds two live
//! notes across the reset boundary `B = 448`: key 48 down, and key 50 up under its
//! pedal. The successor is a fresh owner of the same plan armed for `B`, with one live
//! onset waiting in its own ring for `B + 10`. Output index `i` holds engine sample
//! `i - 64`, because the first output quantum is the old owner's primed carry, so `B`
//! is heard from output 512 and the fade ends at output 576.

use ringbuf::traits::Observer;

use super::super::{
    CompiledPayload, MixedCompiledTransport, MixedIngressOriginId, MixedIngressOutcome,
    MixedIngressRequest, MixedJoinedPrepared, MixedOfferKind, MixedOfferSlotState,
    MixedOneShotAudio, MixedOneShotControl, MixedStopAccepted, TEST_SCOPE,
    history_tests::{bound_with_events, key, mixed_profile},
    occurrence::{LiveOccurrenceId, MixedOccurrenceState, MixedResetEnd},
};
use super::{MixedResetPair, MixedResetRefused, MixedResetState, hot::fade_gain};
use crate::{
    identity::Resolution,
    ingress::ReleaseCause,
    quantities::{ChannelLayout, EventCount, HeldNoteCount, NoteVelocity},
    render::{AudioBlockMut, EventPayload},
    schedule::PlanEvent,
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime},
};

const ORIGIN: MixedIngressOriginId = MixedIngressOriginId(9);
const STOP: SampleTime = SampleTime::new(192);
const B: SampleTime = SampleTime::new(448);
const FADE_END: SampleTime = SampleTime::new(512);
const B_OUTPUT: usize = 512;
const FADE_END_OUTPUT: usize = 576;
const AFTER_FRAMES: usize = 256;
const QUANTUM: usize = QUANTUM_FRAMES as usize;

/// Callback partitions of the 512 output frames before `B`.
const BEFORE: [&[usize]; 4] = [&[512], &[64; 8], &[8; 64], &[7, 63, 90, 1, 256, 95]];
/// Callback partitions of the 256 frames from `B`, splitting the fade differently.
const AFTER: [&[usize]; 4] = [&[256], &[64; 4], &[8; 32], &[5, 50, 9, 1, 191]];

fn bound() -> MixedJoinedPrepared {
    bound_with_events(true, |note| {
        let on = |at, raw| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(raw),
                    velocity: NoteVelocity::FULL,
                },
            )
        };
        let off = |at, raw| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(raw),
                },
            )
        };
        vec![on(10, 60), off(300, 60), on(400, 72), off(500, 72)]
    })
}

fn onset(control: &mut MixedOneShotControl, at: u64, raw: u8) -> LiveOccurrenceId {
    let (_, request) = control
        .submit_onset(
            ORIGIN,
            TEST_SCOPE,
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

/// The old owner and the handles of its held (key 48) and sustained (key 50) notes.
fn old_owner() -> (
    MixedOneShotControl,
    MixedOneShotAudio,
    LiveOccurrenceId,
    LiveOccurrenceId,
) {
    let prepared = bound();
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("old arm");
    assert_eq!(
        audio.schedule_compiled_stop(STOP),
        Ok(MixedStopAccepted::Scheduled)
    );
    let held = onset(&mut control, 100, 48);
    let _down = control
        .submit_pedal(ORIGIN, TEST_SCOPE, SampleTime::new(110), true)
        .expect("pedal down");
    let sustained = onset(&mut control, 120, 50);
    let _up = control
        .submit_release(ORIGIN, sustained, SampleTime::new(130))
        .expect("key-up under the pedal");
    (control, audio, held, sustained)
}

/// A fresh owner armed to adopt at `requested`'s boundary, with key 55 waiting at `B + 10`.
fn successor_at(requested: SampleTime) -> (MixedOneShotControl, Box<MixedOneShotAudio>) {
    let prepared = bound();
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (mut control, audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("successor arm");
    let _later = onset(&mut control, B.as_u64() + 10, 55);
    (control, Box::new(audio))
}

fn successor() -> (MixedOneShotControl, Box<MixedOneShotAudio>) {
    successor_at(B)
}

fn block(samples: &mut [f32]) -> AudioBlockMut<'_> {
    let frames = samples.len();
    AudioBlockMut::new(samples, frames, ChannelLayout::Mono).expect("mono block")
}

/// Render `samples` through the pair over `partition`, requiring every callback to
/// allocate and free nothing.
fn render_pair(pair: &mut MixedResetPair, samples: &mut [f32], partition: &[usize]) {
    assert_eq!(partition.iter().sum::<usize>(), samples.len());
    let mut offset = 0;
    for &frames in partition {
        let output = block(&mut samples[offset..offset + frames]);
        let allocations = crate::render_allocation::count_allocs(|| {
            pair.render_private(output).expect("pair callback");
        });
        assert_eq!(allocations, 0, "pair callback allocated or freed");
        offset += frames;
    }
}

/// Render one owner alone in quantum-sized callbacks.
fn render_owner(audio: &mut MixedOneShotAudio, samples: &mut [f32]) {
    for chunk in samples.chunks_mut(QUANTUM) {
        audio.render_private(block(chunk)).expect("owner callback");
    }
}

/// Offer `successor` at `B` without allocating, requiring acceptance.
fn offer(pair: &mut MixedResetPair, successor: Box<MixedOneShotAudio>) {
    let mut offered = None;
    let allocations = crate::render_allocation::count_allocs(|| {
        offered = Some(pair.offer_reset(successor, B));
    });
    assert_eq!(allocations, 0, "offer allocated or freed");
    assert!(matches!(offered, Some(Ok(()))));
}

#[test]
fn a_reset_is_refused_and_returned_while_running_or_mistimed() {
    let (_control, old, _, _) = old_owner();
    let mut pair = MixedResetPair::new(old);
    let (_next, next) = successor();
    let epoch = next.audio.renderer.epoch();

    // Rule 3.2: before the Stop applies, the compiled program runs.
    let (reason, next) = pair.offer_reset(next, B).expect_err("running");
    assert_eq!(reason, MixedResetRefused::Running);
    assert_eq!(next.audio.renderer.epoch(), epoch);
    assert!(!next.render_started);
    assert_eq!(pair.state(), MixedResetState::Empty);

    let mut samples = vec![0.0_f32; B_OUTPUT];
    render_pair(&mut pair, &mut samples, BEFORE[1]);
    assert!(matches!(
        pair.active().report().transport,
        MixedCompiledTransport::Stopped { at, .. } if at == STOP
    ));
    assert_eq!(pair.active().audio.renderer.clock(), B);

    let odd = SampleTime::new(B.as_u64() + 1);
    let (reason, next) = pair.offer_reset(next, odd).expect_err("mid-quantum");
    assert_eq!(reason, MixedResetRefused::NotBoundary { at: odd });
    let past = SampleTime::new(384);
    let (reason, next) = pair.offer_reset(next, past).expect_err("past");
    assert_eq!(
        reason,
        MixedResetRefused::BoundaryPassed { at: past, clock: B }
    );
    // The successor's armed transition fixes its boundary; another one is refused.
    let later = SampleTime::new(B.as_u64() + 64);
    let (reason, next) = pair.offer_reset(next, later).expect_err("other boundary");
    assert_eq!(
        reason,
        MixedResetRefused::SuccessorBoundary {
            effective: B,
            at: later,
        }
    );

    // Rule 3.6: one secondary slot. A second successor is returned, not queued.
    offer(&mut pair, next);
    assert_eq!(pair.state(), MixedResetState::Pending { at: B });
    let (_other, other) = successor();
    let (reason, other) = pair.offer_reset(other, B).expect_err("slot occupied");
    assert_eq!(reason, MixedResetRefused::SecondaryOccupied);
    assert!(!other.render_started);
    let withdrawn = pair.withdraw_pending().expect("pending successor");
    assert_eq!(withdrawn.audio.renderer.epoch(), epoch);
    assert_eq!(pair.state(), MixedResetState::Empty);
    assert!(pair.withdraw_pending().is_none());
    assert_eq!(pair.fault(), None);
}

/// Old-owner output with no reset, where the late onset key 52 arrives after 512
/// output frames when `late_onset` is set.
fn old_alone(late_onset: bool) -> Vec<f32> {
    let (mut control, mut audio, _, _) = old_owner();
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render_owner(&mut audio, &mut samples[..B_OUTPUT]);
    while control.collect_ingress_result().is_some() {}
    if late_onset {
        let _late = onset(&mut control, B.as_u64() + 22, 52);
    }
    render_owner(&mut audio, &mut samples[B_OUTPUT..]);
    samples
}

#[test]
fn old_work_pending_at_the_boundary_refuses_the_reset_without_waiting() {
    for (before, after) in BEFORE.iter().zip(AFTER) {
        let (mut control, old, _, _) = old_owner();
        let mut pair = MixedResetPair::new(old);
        let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
        render_pair(&mut pair, &mut samples[..B_OUTPUT], before);
        while control.collect_ingress_result().is_some() {}
        let (_next, next) = successor();
        offer(&mut pair, next);
        // A command the old owner would serve at `B` arrives after the offer.
        let _late = onset(&mut control, B.as_u64() + 22, 52);
        render_pair(&mut pair, &mut samples[B_OUTPUT..], after);

        assert_eq!(
            pair.state(),
            MixedResetState::Refused {
                at: B,
                reason: MixedResetRefused::IngressPending,
            }
        );
        // The refusal changed nothing: the old owner served its command and played on.
        assert_eq!(samples, old_alone(true), "{before:?} {after:?}");
        let late_result = control
            .collect_ingress_result()
            .expect("old owner served the onset");
        assert!(matches!(
            late_result.fields().3,
            MixedIngressOutcome::Onset(_)
        ));
        // A refused successor still occupies the one secondary slot until taken.
        let (_other, other) = successor();
        let (reason, _other) = pair.offer_reset(other, B).expect_err("slot occupied");
        assert_eq!(reason, MixedResetRefused::SecondaryOccupied);
        let refused = pair.take_refused().expect("refused successor");
        assert_eq!(refused.at, B);
        assert_eq!(refused.reason, MixedResetRefused::IngressPending);
        assert!(!refused.successor.render_started);
        assert!(!refused.successor.adopted);
        assert_eq!(refused.successor.audio.renderer.clock(), SampleTime::ZERO);
        assert_eq!(pair.state(), MixedResetState::Empty);
        assert!(pair.take_refused().is_none());
    }
}

#[test]
fn a_pending_old_transition_refuses_the_reset_at_its_boundary() {
    let (control, old, _, _) = old_owner();
    let mut pair = MixedResetPair::new(old);
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render_pair(&mut pair, &mut samples[..B_OUTPUT], BEFORE[0]);
    let (_next, next) = successor();
    offer(&mut pair, next);
    let candidate = control
        .prepare_subsequent(B, PlanPosition::new(20), control.in_force)
        .expect("stopped seek");
    let seek = control
        .seal_offer(candidate, B, MixedOfferKind::Seek)
        .expect("sealed seek");
    pair.active_mut()
        .offer_candidate(seek)
        .expect("old owner takes the seek");
    render_pair(&mut pair, &mut samples[B_OUTPUT..], AFTER[1]);
    assert_eq!(
        pair.state(),
        MixedResetState::Refused {
            at: B,
            reason: MixedResetRefused::OfferPending,
        }
    );
    // The old owner's own transition went ahead at `B`.
    assert!(matches!(
        pair.active().report().offer,
        MixedOfferSlotState::Retired(_)
    ));
    let refused = pair.take_refused().expect("refused successor");
    assert!(!refused.successor.render_started);
}

/// Pair output for one pair of partitions, with the reset completed and retired.
fn reset_run(before: &[usize], after: &[usize]) -> Vec<f32> {
    let (_control, old, _, _) = old_owner();
    let mut pair = MixedResetPair::new(old);
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render_pair(&mut pair, &mut samples[..B_OUTPUT], before);
    let (_next, next) = successor();
    offer(&mut pair, next);
    render_pair(&mut pair, &mut samples[B_OUTPUT..], after);
    assert_eq!(pair.fault(), None);
    assert!(matches!(pair.state(), MixedResetState::Retired(_)));
    samples
}

#[test]
fn the_reset_cross_fades_over_exactly_one_quantum_in_every_partition() {
    let old = old_alone(false);
    let (_next, mut alone) = successor();
    alone.install_reset_successor(B);
    let mut new = vec![0.0_f32; AFTER_FRAMES];
    render_owner(&mut alone, &mut new);

    // Falsifier controls: both owners are audible across the fade, and the old notes
    // would keep sounding without the reset.
    assert!(old[B_OUTPUT..FADE_END_OUTPUT].iter().any(|s| *s != 0.0));
    assert!(new[10..QUANTUM].iter().any(|s| *s != 0.0));
    assert_ne!(&old[FADE_END_OUTPUT..], &new[QUANTUM..]);

    let mut expected = old[..B_OUTPUT].to_vec();
    for position in 0..QUANTUM {
        let gain = fade_gain(position).expect("fade frame");
        expected.push(new[position] * gain + old[B_OUTPUT + position] * (1.0 - gain));
    }
    expected.extend_from_slice(&new[QUANTUM..]);
    assert_eq!(fade_gain(QUANTUM - 1), Some(1.0));
    assert_eq!(fade_gain(QUANTUM), None);
    // The successor alone from the fade's last frame on.
    assert_eq!(expected[FADE_END_OUTPUT - 1], new[QUANTUM - 1]);

    for before in BEFORE {
        for after in AFTER {
            assert_eq!(reset_run(before, after), expected, "{before:?} {after:?}");
        }
    }
}

#[test]
fn the_reset_ends_old_live_notes_at_the_fade_end_and_returns_the_old_owner() {
    let (mut control, old, held, sustained) = old_owner();
    let mut pair = MixedResetPair::new(old);
    let mut samples = vec![0.0_f32; B_OUTPUT + AFTER_FRAMES];
    render_pair(&mut pair, &mut samples[..B_OUTPUT], BEFORE[0]);
    while control.collect_ingress_result().is_some() {}
    let states = pair.active().occurrences.occupied();
    let [
        (first, MixedOccurrenceState::Sounding(iheld)),
        (second, MixedOccurrenceState::Sustained(isustained)),
    ] = states.as_slice()
    else {
        panic!("one held and one sustained live note before the reset: {states:?}");
    };
    assert_eq!((*first, *second), (held, sustained));
    let (iheld, isustained) = (*iheld, *isustained);
    let old_epoch = pair.active().audio.renderer.epoch();
    let (_next, next) = successor();
    offer(&mut pair, next);

    // Half the fade: four 8-frame callbacks from `B`.
    render_pair(&mut pair, &mut samples[B_OUTPUT..B_OUTPUT + 32], &[8; 4]);
    let MixedResetState::Fading { receipt, faded } = pair.state() else {
        panic!("fading: {:?}", pair.state());
    };
    assert_eq!(faded, FrameCount::new(32));
    assert_eq!(receipt.at, B);
    assert_eq!(receipt.ended_at, FADE_END);
    assert_eq!(
        receipt.live.ended().collect::<Vec<_>>(),
        vec![
            MixedResetEnd::Tombstoned(held),
            MixedResetEnd::Retired(sustained),
        ]
    );
    // A key-up for the held note after `B` is old input: the old owner never serves it.
    let _late = control
        .submit_release(ORIGIN, held, SampleTime::new(B.as_u64() + 40))
        .expect("old merger still accepts the key-up");
    {
        let fading = pair.secondary().expect("fading owner");
        assert_eq!(fading.audio.renderer.epoch(), old_epoch);
        assert_ne!(pair.active().audio.renderer.epoch(), old_epoch);
        // Both holds were redeemed by one `Reset` group, stamped at the fade's end and
        // still queued, so both notes keep sounding through the fade.
        assert_eq!(fading.test_ingress.holds_outstanding(), EventCount::NONE);
        let mut queue = vec![None; fading.test_ingress.queue_capacity()];
        let len = fading
            .test_ingress
            .copy_queue_for_test(&mut queue)
            .expect("queue copy");
        assert_eq!(len, 1);
        let (event, _) = queue[0].expect("reset group");
        assert_eq!(event.envelope().time(), FADE_END);
        let EventPayload::ReleaseGroup(group) = event.payload() else {
            panic!("a release group: {event:?}");
        };
        assert_eq!(group.cause(), ReleaseCause::Reset);
        assert_eq!(
            group.identities().collect::<Vec<_>>(),
            vec![iheld, isustained]
        );
        let sounding = fading
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("registry");
        assert_eq!(sounding.live().len(), 2);
        assert_ne!(fading.audio.live_minter.resolve(iheld), Resolution::Live);
        assert_ne!(
            fading.audio.live_minter.resolve(isustained),
            Resolution::Live
        );
        assert_eq!(
            fading.occurrences.occupied(),
            vec![(held, MixedOccurrenceState::Reset(iheld))]
        );
    }

    render_pair(&mut pair, &mut samples[B_OUTPUT + 32..], &[8, 8, 8, 8, 192]);
    let MixedResetState::Retired(retired) = pair.state() else {
        panic!("retired: {:?}", pair.state());
    };
    assert_eq!(retired, receipt);
    // The successor is the active owner, adopted and holding its stopped playhead.
    let report = pair.active().report();
    assert!(report.adopted);
    assert_eq!(
        report.transport,
        MixedCompiledTransport::Stopped {
            at: B,
            released: HeldNoteCount::NONE,
            position: Some(PlanPosition::new(10)),
        }
    );
    assert_eq!(report.stops_charged, EventCount::NONE);

    let returned = pair.take_retired().expect("retired old owner");
    assert_eq!(returned.receipt, receipt);
    let old = returned.owner;
    // Exactly one discarded quantum after the fade, and no live obligation left.
    assert_eq!(old.audio.renderer.clock(), SampleTime::new(576));
    assert!(!old.audio.renderer.has_replacement_obligations());
    assert!(
        old.audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("registry")
            .live()
            .is_empty()
    );
    assert_eq!(old.audio.live_minter.live(), 0);
    assert!(old.test_ingress.is_empty());
    assert_eq!(old.fault, None);
    // The late key-up is still in the old ring, unserved, and the tombstone stands.
    assert_eq!(old.ingress_commands.0.occupied_len(), 1);
    assert_eq!(
        old.occurrences.occupied(),
        vec![(held, MixedOccurrenceState::Reset(iheld))]
    );
    assert_eq!(pair.state(), MixedResetState::Empty);
    assert!(pair.take_retired().is_none());

    let (active, secondary) = pair.into_owners();
    assert!(secondary.is_none());
    assert_eq!(active.report().sequence, report.sequence);
}

#[test]
fn a_successor_armed_for_another_boundary_cannot_take_over_at_this_one() {
    let (_control, old, _, _) = old_owner();
    let mut pair = MixedResetPair::new(old);
    let mut samples = vec![0.0_f32; B_OUTPUT];
    render_pair(&mut pair, &mut samples, BEFORE[2]);
    let (_next, early) = successor_at(SampleTime::new(384));
    let (reason, early) = pair.offer_reset(early, B).expect_err("armed for 384");
    assert_eq!(
        reason,
        MixedResetRefused::SuccessorBoundary {
            effective: SampleTime::new(384),
            at: B,
        }
    );
    assert!(!early.render_started);
}
