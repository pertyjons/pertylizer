//! Sample-exact Panic across both producers (draft rules 1.5, 1.5a and 2.4, with the
//! maintainer's choice that Panic ends compiled occurrences at its queue position too).
//!
//! A Panic is served in the quantum that contains its execution stamp `p`. There the live
//! group ends every live occurrence, and Release-charged compiled groups end every open
//! compiled occurrence, both at `p`. Compiled events stamped at or before `p` publish
//! first, so an onset at `p` gets a zero-length gate; later ones survive and may reuse a
//! freed index. A Panic stamped in a later quantum parks on the audio half with nothing
//! applied, and the commands behind it wait with it.

use super::history_tests::{bound_with_spans, key, mixed_profile, profile_with_release_limits};
use super::occurrence::{MixedLiveSource, MixedPanicked};
use super::*;
use crate::publish::ProducerClass;
use crate::quantities::{ChannelLayout, NoteVelocity};
use crate::render::{AudioBlockMut, NoteEdge};
use crate::schedule::PlanEvent;
use crate::time::QUANTUM_FRAMES;
use std::sync::atomic::Ordering;
use synth_core::MidiChannel;

/// The armed transition adopts plan 0 here, so engine time is plan position plus this.
const ARMED: SampleTime = SampleTime::new(64);
const SHIFT: u64 = 64;
/// Panic's sample: offset 44 of the quantum that opens at 256.
const P: u64 = 300;
const PANIC_QUANTUM: SampleTime = SampleTime::new(256);
/// Compiled notes open before the Panic quantum, from engine 164; one more starts at each
/// of `P - 1`, `P` and `P + 1`.
const EARLY: u8 = 10;
const ENDED: u32 = EARLY as u32 + 2;
const COMPILED_SPAN: u32 = EARLY as u32 + 3;
/// A compiled bend and the list's own Offs, after the Panic.
const LATER_BEND: u64 = 400;
const LATER_OFF: u64 = 500;
const FRAMES: usize = 640;
const PARTITIONS: [&[usize]; 4] = [
    &[512, 128],
    &[64; 10],
    &[8; 80],
    &[7, 63, 90, 1, 256, 95, 128],
];
const ORIGIN: MixedIngressOriginId = MixedIngressOriginId(7);

/// Output index `i` holds engine sample `i - 64`: the first output quantum is the carry.
const fn output(at: u64) -> usize {
    at as usize + QUANTUM_FRAMES as usize
}

/// How the run ends what Panic ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ending {
    /// A Panic at `P`; the list keeps its own later Offs and a later bend.
    Panic,
    /// The reference: every note Panic ends gets its own Off at `P`, and the live notes
    /// their key-ups at `P`.
    OwnOffs,
    /// Neither: everything keeps sounding past `P`.
    Neither,
}

fn compiled_events(note: NoteSlot, ending: Ending) -> Vec<PlanEvent> {
    let plan = |engine: u64| PlanPosition::new(engine - SHIFT);
    let on = |engine, raw| {
        PlanEvent::new(
            plan(engine),
            CompiledPayload::NoteOn {
                slot: note,
                key: key(raw),
                velocity: NoteVelocity::FULL,
            },
        )
    };
    let off = |engine, raw| {
        PlanEvent::new(
            plan(engine),
            CompiledPayload::NoteOff {
                slot: note,
                key: key(raw),
            },
        )
    };
    let ended = (0..EARLY).map(|index| 40 + index).chain([60, 61]);
    let mut events = (0..EARLY)
        .map(|index| on(164 + u64::from(index), 40 + index))
        .collect::<Vec<_>>();
    events.push(on(P - 1, 60));
    events.push(on(P, 61));
    if ending == Ending::OwnOffs {
        events.extend(ended.clone().map(|raw| off(P, raw)));
    }
    events.push(on(P + 1, 62));
    if ending != Ending::OwnOffs {
        events.push(PlanEvent::new(
            plan(LATER_BEND),
            CompiledPayload::Bend {
                slot: note,
                key: key(40),
                cents: Cents::new(50.0).expect("finite"),
            },
        ));
        events.extend(ended.map(|raw| off(LATER_OFF, raw)));
    }
    events.push(off(LATER_OFF, 62));
    events
}

fn armed_with(
    compiled_first: bool,
    ending: Ending,
    profile: &HostProfile,
    live_notes: u32,
) -> Result<(MixedOneShotControl, MixedOneShotAudio), Box<MixedOneShotArmRefusal>> {
    let prepared = bound_with_spans(
        compiled_first,
        HeldNoteCount::measured(COMPILED_SPAN),
        HeldNoteCount::measured(live_notes),
        EventCount::measured(live_notes),
        |note| compiled_events(note, ending),
    );
    let history = prepared
        .prepare_history(ARMED, PlanPosition::ZERO)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    prepared.arm_one_shot(candidate, profile)
}

fn armed(compiled_first: bool, ending: Ending) -> (MixedOneShotControl, MixedOneShotAudio) {
    armed_with(compiled_first, ending, &mixed_profile(), 2).expect("private arm")
}

fn scope() -> MixedLiveScope {
    MixedLiveScope::new(MixedLiveSource::new(1), MidiChannel::new(1))
}

fn onset(control: &mut MixedOneShotControl, at: u64, raw: u8) -> LiveOccurrenceId {
    let (_, request) = control
        .submit_onset(
            ORIGIN,
            scope(),
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
    let _ = control
        .submit_release(ORIGIN, occurrence, SampleTime::new(at))
        .expect("reserved release place");
}

/// The live script every run shares: two held notes, then what `ending` does at `P`,
/// then, when asked, a live onset queued after it at `P`. A key-up frees its identity
/// only when it drains, so the explicit-Offs reference cannot take that onset.
struct Live {
    held: [LiveOccurrenceId; 2],
    after: Option<LiveOccurrenceId>,
    panic: Option<MixedIngressCommandId>,
}

fn submit_live(control: &mut MixedOneShotControl, ending: Ending, after: bool) -> Live {
    let held = [onset(control, 100, 70), onset(control, 280, 71)];
    let mut panic = None;
    match ending {
        Ending::Panic => {
            let (id, _) = control
                .submit_panic(ORIGIN, SampleTime::new(P))
                .expect("the protected place is free");
            panic = Some(id);
        }
        Ending::OwnOffs => {
            for note in held {
                release(control, note, P);
            }
        }
        Ending::Neither => {}
    }
    let after = after.then(|| onset(control, P, 72));
    Live { held, after, panic }
}

/// Render `frames` in callbacks of the repeating `pattern`; no callback may allocate.
fn render_partitioned(audio: &mut MixedOneShotAudio, pattern: &[usize], frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0_f32; frames];
    let (mut start, mut index) = (0, 0);
    while start < frames {
        let len = pattern[index % pattern.len()].min(frames - start);
        index += 1;
        let block = AudioBlockMut::new(&mut samples[start..start + len], len, ChannelLayout::Mono)
            .expect("mono block");
        let allocations = crate::render_allocation::count_allocs(|| {
            audio.render_private(block).expect("private render");
        });
        assert_eq!(allocations, 0, "callback allocated or deallocated");
        start += len;
    }
    samples
}

/// Render one quantum per callback until the clock reaches `at`.
fn render_until(audio: &mut MixedOneShotAudio, at: SampleTime) {
    let mut samples = [0.0_f32; QUANTUM_FRAMES as usize];
    while audio.audio.renderer.clock() < at {
        let frames = samples.len();
        audio
            .render_private(
                AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("block"),
            )
            .expect("private render");
    }
}

fn collect_all(control: &mut MixedOneShotControl) -> Vec<MixedIngressResult> {
    std::iter::from_fn(|| control.collect_ingress_result()).collect()
}

fn panicked(result: &MixedIngressResult) -> MixedPanicked {
    let MixedIngressOutcome::Panic(Ok(panicked)) = result.outcome else {
        panic!("Panic must apply, got {:?}", result.outcome);
    };
    panicked
}

fn compiled_sounding(audio: &MixedOneShotAudio) -> Vec<NoteIdentity> {
    let snapshot = audio
        .audio
        .renderer
        .snapshot_mixed_sounding()
        .expect("registry");
    snapshot
        .compiled()
        .iter()
        .map(|note| note.identity)
        .collect()
}

fn live_sounding(audio: &MixedOneShotAudio) -> Vec<NoteIdentity> {
    let snapshot = audio
        .audio
        .renderer
        .snapshot_mixed_sounding()
        .expect("registry");
    snapshot.live().iter().map(|note| note.identity).collect()
}

fn tokens(audio: &MixedOneShotAudio, state: fn(&MixedCompiledTokenState) -> bool) -> usize {
    audio
        .audio
        .compiled_tokens
        .iter()
        .filter(|token| state(token))
        .count()
}

fn orphans(audio: &MixedOneShotAudio) -> u64 {
    audio.audio.renderer.diagnostics().orphan_note_events()
}

fn release_share_profile(share: u32) -> HostProfile {
    let holds = mixed_profile()
        .limits()
        .events()
        .shares()
        .release_hold_capacity();
    profile_with_release_limits(
        EventCount::measured(share),
        EventCount::measured(holds.get().min(share)),
    )
}

fn ingress_capacity() -> EventCount {
    mixed_profile()
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity()
}

#[test]
fn panic_ends_both_producers_at_its_sample_in_every_partition_and_producer_order() {
    for compiled_first in [true, false] {
        let mut first = None;
        for pattern in PARTITIONS {
            let run = |ending| {
                let (mut control, mut audio) = armed(compiled_first, ending);
                let live = submit_live(&mut control, ending, false);
                let samples = render_partitioned(&mut audio, pattern, FRAMES);
                (control, audio, live, samples)
            };
            let (mut control, audio, live, samples) = run(Ending::Panic);
            let (_, reference, _, own_offs) = run(Ending::OwnOffs);
            let (_, _, _, neither) = run(Ending::Neither);
            let case = format!("compiled_first {compiled_first}, partition {pattern:?}");

            // Every gate both producers own falls exactly where explicit Offs at `P`
            // put it, and nothing before `P` changed.
            assert_eq!(samples, own_offs, "{case}");
            assert_eq!(samples[..output(P)], neither[..output(P)], "{case}");
            assert_ne!(
                samples[output(P)..output(P + 8)],
                neither[output(P)..output(P + 8)],
                "{case}: Panic is audible from its own sample"
            );
            match &first {
                None => first = Some(samples.clone()),
                Some(first) => assert_eq!(&samples, first, "{case}: partition dependence"),
            }

            let report = audio.report();
            assert!(!report.faulted, "{case}: {:?}", report.fault);
            // Ten early notes plus the onsets at `P - 1` and `P`, in two groups.
            assert_eq!(report.compiled_panic_ended, HeldNoteCount::measured(ENDED));
            assert_eq!(report.compiled_panic_groups, EventCount::measured(2));
            // The list's own later Offs and its bend found tombstones.
            assert_eq!(
                report.compiled_released_after_panic,
                EventCount::measured(ENDED)
            );
            assert_eq!(
                report.compiled_controls_after_panic,
                EventCount::measured(1)
            );
            assert_eq!(report.compiled_panic_tombstones, HeldNoteCount::NONE);
            assert_eq!(report.panic_parked, None);
            // Only the compiled groups spend the release share; the live group is Live.
            assert_eq!(
                audio.arbiter.high_water(ProducerClass::Release),
                EventCount::measured(2),
                "{case}"
            );
            assert_eq!(orphans(&audio), 0, "{case}");
            assert_eq!(orphans(&reference), 0, "{case}");
            assert!(compiled_sounding(&audio).is_empty(), "{case}");
            assert!(audio.audio.compiled_minter.live_identities().is_empty());
            assert_eq!(tokens(&audio, |t| *t != MixedCompiledTokenState::Free), 0);

            // The Panic ended both held live notes.
            let results = collect_all(&mut control);
            let panic = results
                .iter()
                .find(|result| Some(result.id) == live.panic)
                .expect("the Panic result");
            let applied = panicked(panic);
            assert_eq!(applied.executed_at(), SampleTime::new(P));
            assert_eq!(
                applied.ended().collect::<Vec<_>>(),
                live.held.map(occurrence::MixedPanicEnd::Tombstoned)
            );
            assert!(live_sounding(&audio).is_empty(), "{case}");
        }
    }
}

#[test]
fn a_compiled_onset_after_the_panic_takes_a_freed_index_at_its_next_generation() {
    for compiled_first in [true, false] {
        let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
        let _live = submit_live(&mut control, Ending::Panic, true);
        render_until(&mut audio, PANIC_QUANTUM);
        let early = compiled_sounding(&audio);
        assert_eq!(early.len(), usize::from(EARLY));
        let lowest = *early
            .iter()
            .min_by_key(|identity| identity.index())
            .expect("an early note");
        assert_eq!(
            tokens(&audio, |t| matches!(t, MixedCompiledTokenState::Open(_))),
            10
        );

        // The Panic quantum: `P - 1` and `P` open and end at `P`; `P + 1` reuses.
        render_until(
            &mut audio,
            PANIC_QUANTUM
                .checked_add(FrameCount::QUANTUM)
                .expect("time"),
        );
        let survivor = compiled_sounding(&audio);
        assert_eq!(survivor.len(), 1, "only the onset at P + 1 sounds");
        assert_eq!(survivor[0].index(), lowest.index());
        assert!(survivor[0].generation() > lowest.generation());
        assert_eq!(audio.audio.compiled_minter.live_identities(), survivor);
        assert_eq!(
            tokens(&audio, |t| *t == MixedCompiledTokenState::PanicEnded),
            ENDED as usize
        );
        assert_eq!(
            tokens(&audio, |t| matches!(t, MixedCompiledTokenState::Open(_))),
            1
        );
        assert_eq!(
            audio.report().compiled_panic_tombstones,
            HeldNoteCount::measured(ENDED)
        );
        assert_eq!(orphans(&audio), 0);
    }
}

#[test]
fn arm_requires_release_share_for_the_live_backlog_plus_one_compiled_cut() {
    let capacity = ingress_capacity();
    // Thirteen compiled identities are two groups of eight.
    let required = capacity.get() + 2;
    for compiled_first in [true, false] {
        for share in [required - 1, required, required + 1] {
            match armed_with(
                compiled_first,
                Ending::Panic,
                &release_share_profile(share),
                2,
            ) {
                Ok(_) => assert!(share >= required, "share {share} armed"),
                Err(refusal) => {
                    assert_eq!(share, required - 1, "share {share} refused");
                    assert_eq!(
                        refusal.reason,
                        MixedOneShotArmError::CompiledPanicReleaseShare {
                            share: EventCount::measured(share),
                            capacity,
                            compiled_span: HeldNoteCount::measured(COMPILED_SPAN),
                            required: EventCount::measured(required),
                        }
                    );
                    assert!(refusal.reason.to_string().contains("compiled Panic cut"));
                    // Both inputs come back unconsumed and still arm.
                    let refusal = *refusal;
                    let _armed = refusal
                        .owner
                        .arm_one_shot(refusal.candidate, &mixed_profile())
                        .expect("returned owner and candidate arm");
                }
            }
        }
    }
}

#[test]
fn a_panic_incapable_arm_needs_only_the_live_release_backlog() {
    let prepared = bound_with_spans(
        true,
        HeldNoteCount::measured(COMPILED_SPAN),
        HeldNoteCount::measured(2),
        EventCount::measured(2),
        |note| compiled_events(note, Ending::Panic),
    );
    let history = prepared
        .prepare_history(ARMED, PlanPosition::ZERO)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let _halves = prepared
        .arm_one_shot_with(
            candidate,
            &release_share_profile(ingress_capacity().get()),
            None,
            MixedPanicCapability::Absent,
        )
        .expect("transfer without Panic needs no compiled cut reservation");
}

#[test]
fn panic_with_the_live_queue_saturated_fits_a_release_share_at_the_bound() {
    // Eight live identities and holds: the most one release group can end, and the
    // widest release backlog this live range can queue for one quantum.
    const LIVE: u32 = 8;
    let share = ingress_capacity().get() + 2;
    for compiled_first in [true, false] {
        let (mut control, mut audio) = armed_with(
            compiled_first,
            Ending::Panic,
            &release_share_profile(share),
            LIVE,
        )
        .expect("arm at the bound");
        let held = (0..LIVE)
            .map(|index| onset(&mut control, 100 + u64::from(index), 70 + index as u8))
            .collect::<Vec<_>>();
        render_until(&mut audio, PANIC_QUANTUM);
        assert!(
            collect_all(&mut control)
                .iter()
                .all(|result| matches!(result.outcome, MixedIngressOutcome::Onset(Ok(_))))
        );
        assert_eq!(
            audio.test_ingress.holds_outstanding(),
            EventCount::measured(LIVE)
        );
        // Every hold is released in the Panic quantum; further onsets at that stamp find
        // no identity and their key-ups are consumed. Then the Panic takes its protected
        // place in a command ring with no ordinary room left.
        for note in held {
            release(&mut control, note, 290);
        }
        let mut raw = 0_u8;
        while let Ok((_, MixedIngressRequest::Onset { occurrence, .. })) = control.submit_onset(
            ORIGIN,
            scope(),
            SampleTime::new(290),
            key(raw),
            NoteVelocity::FULL,
        ) {
            release(&mut control, occurrence, 290);
            raw += 1;
        }
        assert_eq!(control.ingress_room(), 0, "the command ring is full");
        let _panic = control
            .submit_panic(ORIGIN, SampleTime::new(P))
            .expect("the protected place is free");
        render_until(
            &mut audio,
            PANIC_QUANTUM
                .checked_add(FrameCount::QUANTUM)
                .expect("time"),
        );
        let results = collect_all(&mut control);
        let released = results
            .iter()
            .filter(|result| {
                matches!(
                    result.outcome,
                    MixedIngressOutcome::Release(Ok(MixedRetired {
                        cause: occurrence::MixedRetireCause::Released(_),
                        ..
                    }))
                )
            })
            .count();
        // The held notes, plus the pairs whose onsets found an identity a key-up freed.
        assert!(released >= LIVE as usize, "{released} releases");
        let report = audio.report();
        assert!(!report.faulted, "{:?}", report.fault);
        assert_eq!(report.compiled_panic_ended, HeldNoteCount::measured(ENDED));
        // Every live release and both compiled groups were charged in the one quantum,
        // within the share arm proved.
        let high = audio.arbiter.high_water(ProducerClass::Release);
        assert_eq!(
            high,
            EventCount::measured(u32::try_from(released).expect("small") + 2)
        );
        assert!(high <= EventCount::measured(share));
        assert_eq!(orphans(&audio), 0);
    }
}

#[test]
fn a_panic_stamped_in_a_later_quantum_parks_until_its_quantum_opens() {
    for compiled_first in [true, false] {
        let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
        let live = submit_live(&mut control, Ending::Panic, true);
        let first = live.panic.expect("the Panic");
        audio.service_test_ingress_queue();
        // The two onsets before it were served; it and everything behind it wait.
        let results = collect_all(&mut control);
        assert_eq!(
            results
                .iter()
                .map(|result| result.request)
                .collect::<Vec<_>>(),
            [(100, 70), (280, 71)]
                .iter()
                .zip(live.held)
                .map(|((at, raw), occurrence)| MixedIngressRequest::Onset {
                    origin: ORIGIN,
                    occurrence,
                    scope: scope(),
                    at: SampleTime::new(*at),
                    key: key(*raw),
                    velocity: NoteVelocity::FULL,
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(audio.report().panic_parked, Some(SampleTime::new(P)));
        assert_eq!(
            audio.test_ingress.holds_outstanding(),
            EventCount::measured(2)
        );
        // Its merger cell stays occupied, so a second Panic is refused at the caller.
        assert_eq!(control.panic_inflight, Some(first));
        assert_eq!(
            control.submit_panic(ORIGIN, SampleTime::new(P + 1)),
            Err(occurrence::MixedPanicSubmitError::Pending(first))
        );

        // Each boundary before its quantum serves nothing.
        while audio.audio.renderer.clock() < PANIC_QUANTUM {
            let next = audio
                .audio
                .renderer
                .clock()
                .checked_add(FrameCount::QUANTUM)
                .expect("time");
            render_until(&mut audio, next);
            assert!(collect_all(&mut control).is_empty());
            assert_eq!(audio.report().panic_parked, Some(SampleTime::new(P)));
            assert_eq!(audio.report().compiled_panic_ended, HeldNoteCount::NONE);
        }
        render_until(
            &mut audio,
            PANIC_QUANTUM
                .checked_add(FrameCount::QUANTUM)
                .expect("time"),
        );
        // Its quantum serves it first, then what waited behind it, in command order.
        let results = collect_all(&mut control);
        assert_eq!(results.len(), 2);
        assert_eq!(Some(results[0].id), live.panic);
        assert_eq!(panicked(&results[0]).executed_at(), SampleTime::new(P));
        assert!(matches!(
            results[1].request,
            MixedIngressRequest::Onset { occurrence, .. } if Some(occurrence) == live.after
        ));
        // Queued after the Panic at its own stamp, the onset survives it.
        let MixedIngressOutcome::Onset(Ok(after)) = results[1].outcome else {
            panic!("the onset after Panic was accepted");
        };
        assert_eq!(live_sounding(&audio), vec![after]);
        let report = audio.report();
        assert_eq!(report.panic_parked, None);
        assert_eq!(report.compiled_panic_ended, HeldNoteCount::measured(ENDED));
        assert!(control.panic_inflight.is_none());
        assert_eq!(
            control.submit_onset(
                ORIGIN,
                scope(),
                SampleTime::new(100),
                key(72),
                NoteVelocity::FULL,
            ),
            Err(occurrence::MixedOnsetSubmitError::BehindPanic(
                SampleTime::new(P)
            ))
        );
        // The receipt frees the place, but this is still the quantum that cut
        // the first Panic. A new audio service boundary reopens it.
        assert_eq!(
            control.submit_panic(ORIGIN, SampleTime::new(400)),
            Err(MixedPanicSubmitError::SameServiceQuantum {
                previous: SampleTime::new(P),
                at: SampleTime::new(400),
            })
        );
        render_until(&mut audio, SampleTime::new(384));
        let _ = control
            .submit_panic(ORIGIN, SampleTime::new(400))
            .expect("the next audio service quantum opens the cell");
    }
}

#[test]
fn a_future_empty_panic_refuses_an_earlier_onset_before_it_takes_custody() {
    for compiled_first in [true, false] {
        let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
        let _panic = control
            .submit_panic(ORIGIN, SampleTime::new(P))
            .expect("protected Panic place");
        let reserved = control.release_reserved;
        assert_eq!(
            control.submit_onset(
                ORIGIN,
                scope(),
                SampleTime::new(100),
                key(70),
                NoteVelocity::FULL
            ),
            Err(occurrence::MixedOnsetSubmitError::BehindPanic(
                SampleTime::new(P)
            ))
        );
        assert_eq!(control.release_reserved, reserved);
        audio.service_test_ingress_queue();
        assert_eq!(audio.report().panic_parked, Some(SampleTime::new(P)));
        assert!(collect_all(&mut control).is_empty());
    }
}

#[test]
fn a_stop_at_the_panic_boundary_leaves_panic_no_compiled_occurrence_to_end() {
    for compiled_first in [true, false] {
        let mut first = None;
        for pattern in PARTITIONS {
            let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
            let live = submit_live(&mut control, Ending::Panic, true);
            assert_eq!(
                audio.schedule_compiled_stop(PANIC_QUANTUM),
                Ok(MixedStopAccepted::Scheduled)
            );
            let samples = render_partitioned(&mut audio, pattern, FRAMES);
            let report = audio.report();
            assert!(!report.faulted, "{:?}", report.fault);
            assert_eq!(
                report.transport,
                MixedCompiledTransport::Stopped {
                    at: PANIC_QUANTUM,
                    released: HeldNoteCount::measured(u32::from(EARLY)),
                    position: Some(PlanPosition::new(192)),
                }
            );
            // The Stop applied first and ended every compiled note; a stopped program
            // publishes nothing, so the Panic emitted no compiled group.
            assert_eq!(report.compiled_panic_ended, HeldNoteCount::NONE);
            assert_eq!(report.compiled_panic_groups, EventCount::NONE);
            assert_eq!(report.compiled_panic_tombstones, HeldNoteCount::NONE);
            assert_eq!(
                audio.arbiter.high_water(ProducerClass::Release),
                EventCount::NONE
            );
            assert!(compiled_sounding(&audio).is_empty());
            let results = collect_all(&mut control);
            let panic = results
                .iter()
                .find(|result| Some(result.id) == live.panic)
                .expect("the Panic result");
            assert_eq!(
                panicked(panic).ended().collect::<Vec<_>>(),
                live.held.map(occurrence::MixedPanicEnd::Tombstoned)
            );
            assert_eq!(live_sounding(&audio).len(), 1, "the later onset sounds");
            assert_eq!(orphans(&audio), 0);
            match &first {
                None => first = Some(samples),
                Some(first) => assert_eq!(&samples, first, "partition {pattern:?}"),
            }
        }
    }
}

#[test]
fn a_seek_adopted_at_the_panic_boundary_leaves_its_new_notes_to_the_panic() {
    for compiled_first in [true, false] {
        let mut first = None;
        for pattern in PARTITIONS {
            let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
            let live = submit_live(&mut control, Ending::Panic, true);
            // The destination maps the early notes to engine 256..266.
            let candidate = control
                .prepare_subsequent(
                    PANIC_QUANTUM,
                    PlanPosition::new(164 - SHIFT),
                    control.in_force,
                )
                .expect("later candidate");
            let offer = control
                .seal_offer(candidate, PANIC_QUANTUM, MixedOfferKind::Seek)
                .expect("sealed offer");
            audio
                .offer_candidate(offer)
                .map_err(|refusal| refusal.0)
                .expect("offer");
            let samples = render_partitioned(&mut audio, pattern, FRAMES);
            let report = audio.report();
            assert!(!report.faulted, "{:?}", report.fault);
            // The boundary release ended the old program's ten notes; the Panic ended
            // only the new program's ten, which began at or before `P`.
            let MixedOfferSlotState::Retired(MixedOfferReceipt {
                outcome: MixedOfferOutcome::Adopted(adopted),
                ..
            }) = report.offer
            else {
                panic!("the seek adopted: {:?}", report.offer);
            };
            assert_eq!(adopted.released, HeldNoteCount::measured(u32::from(EARLY)));
            assert_eq!(
                report.compiled_panic_ended,
                HeldNoteCount::measured(u32::from(EARLY))
            );
            assert_eq!(report.compiled_panic_groups, EventCount::measured(2));
            // The new program's bend for a Panic-ended note was consumed; its notes at
            // `P - 1 + 156` and later started after the Panic and still sound.
            assert_eq!(
                report.compiled_controls_after_panic,
                EventCount::measured(1)
            );
            assert_eq!(
                report.compiled_panic_tombstones,
                HeldNoteCount::measured(u32::from(EARLY))
            );
            assert_eq!(compiled_sounding(&audio).len(), 3);
            let results = collect_all(&mut control);
            let panic = results
                .iter()
                .find(|result| Some(result.id) == live.panic)
                .expect("the Panic result");
            assert_eq!(panicked(panic).ended().count(), 2);
            assert_eq!(orphans(&audio), 0);
            match &first {
                None => first = Some(samples),
                Some(first) => assert_eq!(&samples, first, "partition {pattern:?}"),
            }
        }
    }
}

#[test]
fn the_compiled_cut_refuses_before_changing_custody_when_the_share_is_spent() {
    let (mut control, mut audio) = armed(true, Ending::Panic);
    let _live = submit_live(&mut control, Ending::Panic, true);
    render_until(&mut audio, PANIC_QUANTUM);
    let clock = audio.audio.renderer.clock();
    let tokens_before = audio.audio.compiled_tokens.clone();
    let minted = audio.audio.compiled_minter.live_identities();
    assert_eq!(minted.len(), usize::from(EARLY));
    let share = mixed_profile()
        .limits()
        .events()
        .shares()
        .release_event_share();
    let filler = TimedEvent::new(
        EventEnvelope::new(
            audio.audio.renderer.epoch(),
            clock,
            crate::time::TimeSource::Compiled,
        ),
        EventPayload::Note {
            identity: minted[0],
            edge: NoteEdge::Off,
        },
    );
    let mut publication = audio.arbiter.open(clock, 1).expect("window");
    for _ in 1..share.get() {
        publication
            .charge(ProducerClass::Release, filler)
            .expect("below the share");
    }
    assert_eq!(
        audio.audio.end_compiled_for_panic(clock, &mut publication),
        Err(MixedOneShotRenderError::CompiledPanicShare {
            spent: EventCount::measured(share.get() - 1),
            groups: EventCount::measured(2),
            share,
        })
    );
    drop(publication);
    assert_eq!(audio.audio.compiled_tokens, tokens_before);
    assert_eq!(audio.audio.compiled_minter.live_identities(), minted);
}

#[test]
fn a_second_panic_owed_in_one_quantum_waits_for_the_next_boundary() {
    for compiled_first in [true, false] {
        let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
        let _live = submit_live(&mut control, Ending::Panic, true);
        // Only a collector that turns the merger cell around inside one service pass can
        // reach this; the forged command stands in for it.
        let _ = control
            .submit_ingress(MixedIngressRequest::Panic {
                origin: ORIGIN,
                at: SampleTime::new(310),
            })
            .expect("forged second Panic fits");
        render_until(
            &mut audio,
            PANIC_QUANTUM
                .checked_add(FrameCount::QUANTUM)
                .expect("time"),
        );
        // One compiled cut per quantum: the second waits, with nothing applied.
        let report = audio.report();
        assert_eq!(report.panic_parked, Some(SampleTime::new(310)));
        assert_eq!(report.compiled_panic_ended, HeldNoteCount::measured(ENDED));
        let next = SampleTime::new(320);
        render_until(
            &mut audio,
            next.checked_add(FrameCount::QUANTUM).expect("time"),
        );
        // Served late at the next boundary, it ends the note that began at `P + 1`
        // there, where the late clamp also puts its live group.
        let report = audio.report();
        assert!(!report.faulted, "{:?}", report.fault);
        assert_eq!(report.panic_parked, None);
        assert_eq!(
            report.compiled_panic_ended,
            HeldNoteCount::measured(ENDED + 1)
        );
        assert_eq!(report.compiled_panic_groups, EventCount::measured(3));
        assert!(compiled_sounding(&audio).is_empty());
        let mut results = Vec::new();
        while let Some(result) = control.ingress_results.0.try_pop() {
            results.push(result);
        }
        let second = results.last().expect("the second Panic's result");
        assert_eq!(panicked(second).executed_at(), next);
        assert_eq!(orphans(&audio), 0);
    }
}

#[test]
fn a_collected_late_panic_cannot_reopen_the_cell_in_the_same_quantum() {
    for compiled_first in [true, false] {
        let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
        render_until(&mut audio, SampleTime::new(320));
        let (first, _) = control
            .submit_panic(ORIGIN, SampleTime::new(310))
            .expect("late Panic fits the protected place");
        // Collection frees the ring place, but the audio clock has not entered
        // another quantum. The caller must be refused before minting another ID.
        audio.service_test_ingress_queue();
        let results = collect_all(&mut control);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, first);
        assert_eq!(panicked(&results[0]).executed_at(), SampleTime::new(320));
        let last_id = control.last_ingress_id;
        assert_eq!(
            control.submit_panic(ORIGIN, SampleTime::new(321)),
            Err(MixedPanicSubmitError::SameServiceQuantum {
                previous: SampleTime::new(320),
                at: SampleTime::new(321),
            })
        );
        assert_eq!(control.last_ingress_id, last_id);
        assert_eq!(control.panic_inflight, None);
        render_until(&mut audio, SampleTime::new(448));
        let _ = control
            .submit_panic(ORIGIN, SampleTime::new(400))
            .expect("a new audio quantum reopens the protected place");
    }
}

#[test]
fn a_partial_render_keeps_the_current_panic_service_quantum_published() {
    for compiled_first in [true, false] {
        let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
        render_until(&mut audio, PANIC_QUANTUM);
        let _first = control
            .submit_panic(ORIGIN, SampleTime::new(P))
            .expect("first Panic");
        let mut samples = [0.0_f32; 17];
        audio
            .render_private(
                AudioBlockMut::new(&mut samples, 17, ChannelLayout::Mono).expect("block"),
            )
            .expect("first partial render");
        let results = collect_all(&mut control);
        assert_eq!(results.len(), 1);
        assert_eq!(panicked(&results[0]).executed_at(), SampleTime::new(P));
        let last_id = control.last_ingress_id;
        assert_eq!(
            audio.panic_current_service_at.load(Ordering::Acquire),
            PANIC_QUANTUM.as_u64()
        );
        audio
            .render_private(
                AudioBlockMut::new(&mut samples[..5], 5, ChannelLayout::Mono).expect("block"),
            )
            .expect("carry render");
        assert_eq!(
            audio.panic_current_service_at.load(Ordering::Acquire),
            PANIC_QUANTUM.as_u64()
        );
        assert_eq!(
            control.submit_panic(ORIGIN, SampleTime::new(P + 1)),
            Err(MixedPanicSubmitError::SameServiceQuantum {
                previous: SampleTime::new(P),
                at: SampleTime::new(P + 1),
            })
        );
        assert_eq!(control.last_ingress_id, last_id);
        assert_eq!(control.panic_inflight, None);
        render_until(&mut audio, SampleTime::new(384));
        assert_eq!(audio.panic_current_service_at.load(Ordering::Acquire), 320);
        let _ = control
            .submit_panic(ORIGIN, SampleTime::new(P + 1))
            .expect("late stamp is accepted after the next quantum opens");
    }
}

#[test]
fn teardown_returns_a_parked_panic_unapplied_and_reports_compiled_tombstones() {
    for compiled_first in [true, false] {
        // A fault while the Panic is parked: it ends cancelled, with nothing applied.
        let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
        let live = submit_live(&mut control, Ending::Panic, true);
        audio.test_fail_after_ingress_at = Some(SampleTime::new(128));
        let mut samples = [0.0_f32; QUANTUM_FRAMES as usize];
        let error = loop {
            let frames = samples.len();
            if let Err(error) = audio.render_private(
                AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("block"),
            ) {
                break error;
            }
        };
        assert_eq!(error, MixedOneShotRenderError::InjectedAfterIngress);
        let MixedCollection::Ended(teardown) = control.collect(audio).expect("collection") else {
            panic!("a faulted owner ends at collection");
        };
        let parked = teardown
            .ingress
            .parked_panic
            .as_ref()
            .expect("the parked Panic");
        assert_eq!(Some(parked.id), live.panic);
        assert_ne!(teardown.ingress.last_served_id, Some(parked.id));
        assert_eq!(
            parked.request,
            MixedIngressRequest::Panic {
                origin: ORIGIN,
                at: SampleTime::new(P),
            }
        );
        assert_eq!(teardown.report.panic_parked, Some(SampleTime::new(P)));
        assert_eq!(teardown.report.compiled_panic_ended, HeldNoteCount::NONE);
        assert_eq!(teardown.ingress.holds_outstanding, EventCount::measured(2));
        assert!(
            teardown
                .ingress
                .occurrences
                .iter()
                .all(|(_, state)| matches!(state, occurrence::MixedOccurrenceState::Sounding(_)))
        );
        // The onset behind it was never popped.
        assert!(
            teardown
                .ingress
                .unserved_commands
                .iter()
                .any(|command| matches!(
                    command.request,
                    MixedIngressRequest::Onset { occurrence, .. } if Some(occurrence) == live.after
                ))
        );
        assert_eq!(
            teardown
                .ingress
                .terminal_control
                .as_ref()
                .and_then(|custody| custody.panic_inflight),
            live.panic
        );

        // A fault after the Panic applied: its compiled tombstones are reported, and the
        // minter holds only the note that began after it.
        let (mut control, mut audio) = armed(compiled_first, Ending::Panic);
        let _live = submit_live(&mut control, Ending::Panic, true);
        // In the quantum after the Panic's, before the list's own Offs at 500.
        audio.test_fail_after_ingress_at = Some(SampleTime::new(384));
        let error = loop {
            let frames = samples.len();
            if let Err(error) = audio.render_private(
                AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("block"),
            ) {
                break error;
            }
        };
        assert_eq!(error, MixedOneShotRenderError::InjectedAfterIngress);
        let MixedCollection::Ended(teardown) = control.collect(audio).expect("collection") else {
            panic!("a faulted owner ends at collection");
        };
        assert!(teardown.ingress.parked_panic.is_none());
        assert_eq!(
            teardown.report.compiled_panic_tombstones,
            HeldNoteCount::measured(ENDED)
        );
        assert_eq!(
            teardown.report.compiled_panic_ended,
            HeldNoteCount::measured(ENDED)
        );
        assert_eq!(teardown.compiled_minted.len(), 1);
        assert_eq!(
            teardown
                .sounding
                .compiled()
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            teardown.compiled_minted
        );
    }
}
