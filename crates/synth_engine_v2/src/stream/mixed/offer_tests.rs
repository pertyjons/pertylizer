//! The private one-slot offer of a later Start or seek candidate (draft Item 2,
//! rules 2.1 and 2.2).
//!
//! Control seals an admitted candidate; audio takes it at its quantum boundary, ends the
//! old program's sounding compiled occurrences, swaps the program and publishes its
//! scoped restoration before any destination edge, and parks the same allocation with a
//! bounded receipt. Live notes must render as if the adoption had not happened, the
//! slot refuses a second offer while it is occupied, and control's in-force sequence
//! moves only on a receipt confirming adoption.

use super::history_tests::{bound_with_events, key, mixed_profile};
use super::*;
use crate::quantities::{ChannelLayout, NoteVelocity};
use crate::render::{AudioBlockMut, NoteEdge, Renderer, TimedEvents};
use crate::schedule::PlanEvent;
use crate::time::{QUANTUM_FRAMES, TimeSource as TestOrigin};

/// The armed transition adopts here from a stopped clock, at plan position 10.
const ARMED: SampleTime = SampleTime::new(64);
/// The later seek's boundary; it moves to plan position 400, where key 72 starts.
const SEEK: SampleTime = SampleTime::new(256);
const SEEK_POSITION: PlanPosition = PlanPosition::new(400);
/// Output index `i` holds engine sample `i - 64`: the first output quantum is the carry.
const SEEK_OUTPUT: usize = 320;
const FRAMES: usize = 640;

const PARTITIONS: [&[usize]; 4] = [
    &[512, 128],
    &[64; 10],
    &[8; 80],
    &[7, 63, 90, 1, 256, 95, 128],
];

/// Compiled key 60 from 10 to 300 and key 72 from 400 to 500. Armed at position 10,
/// key 60 sounds from engine 64 across the seek, which must end it and start key 72
/// exactly at the boundary.
fn bound(compiled_first: bool) -> MixedJoinedPrepared {
    bound_with_events(compiled_first, |note| {
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

fn first_candidate(
    prepared: &MixedJoinedPrepared,
    requested: SampleTime,
    position: PlanPosition,
) -> MixedStampedCandidate {
    let history = prepared
        .prepare_history(requested, position)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    prepared.stamp_suffix(suffix).expect("stamp")
}

fn armed(compiled_first: bool) -> (MixedOneShotControl, MixedOneShotAudio) {
    let prepared = bound(compiled_first);
    let candidate = first_candidate(&prepared, ARMED, PlanPosition::new(10));
    prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm")
}

fn armed_successor() -> ActivationSequence {
    ActivationSequence::INITIAL.next().expect("successor")
}

fn later(control: &MixedOneShotControl, at: SampleTime, position: u64) -> MixedStampedCandidate {
    control
        .prepare_subsequent(at, PlanPosition::new(position), control.in_force)
        .expect("later candidate")
}

fn sealed(control: &MixedOneShotControl, at: SampleTime, position: u64) -> MixedCandidateOffer {
    control
        .seal_offer(later(control, at, position), at, MixedOfferKind::Seek)
        .expect("sealed offer")
}

/// Offer from the audio thread's side, asserting the call allocates and frees nothing.
fn offer(
    audio: &mut MixedOneShotAudio,
    offer: MixedCandidateOffer,
) -> Result<(), (MixedOfferRefused, MixedCandidateOffer)> {
    let mut result = None;
    let allocations = crate::render_allocation::count_allocs(|| {
        result = Some(audio.offer_candidate(offer));
    });
    assert_eq!(allocations, 0, "the offer allocated or deallocated");
    result.expect("offer ran")
}

fn take_retired(audio: &mut MixedOneShotAudio) -> Option<MixedRetiredOffer> {
    let mut retired = None;
    let allocations = crate::render_allocation::count_allocs(|| {
        retired = Some(audio.take_retired_offer());
    });
    assert_eq!(
        allocations, 0,
        "taking the retired offer allocated or deallocated"
    );
    retired.expect("take ran")
}

fn refused_offers(audio: &MixedOneShotAudio) -> u64 {
    audio.audio.renderer.diagnostics().refused_activations()
}

/// Render `frames` over `partition`, asserting every callback allocates nothing.
fn render_owner(audio: &mut MixedOneShotAudio, partition: &[usize], frames: usize) -> Vec<f32> {
    assert_eq!(partition.iter().sum::<usize>(), frames);
    let mut samples = vec![0.0_f32; frames];
    let mut offset = 0;
    for &length in partition {
        let output = AudioBlockMut::new(
            &mut samples[offset..offset + length],
            length,
            ChannelLayout::Mono,
        )
        .expect("mono block");
        let allocations = crate::render_allocation::count_allocs(|| {
            audio.render_private(output).expect("private render");
        });
        assert_eq!(allocations, 0, "callback allocated or deallocated");
        offset += length;
    }
    samples
}

fn quanta(frames: usize) -> Vec<usize> {
    vec![QUANTUM_FRAMES as usize; frames / QUANTUM_FRAMES as usize]
}

/// Everything audio's identity custody and program hold.
fn custody(audio: &MixedOneShotAudio) -> String {
    format!(
        "{:?} {:?} {:?} {:?} {:?} {:?} {} {} {:?}",
        audio.audio.compiled_minter,
        audio.audio.live_minter,
        audio.audio.compiled_tokens,
        audio.audio.compiled_ended,
        audio.audio.renderer.snapshot_mixed_sounding(),
        audio.timing,
        audio.events.len(),
        audio.next,
        audio.in_force,
    )
}

/// A renderer bound to the same plan and partition that hears only `events`.
fn live_only_reference(prepared: &MixedJoinedPrepared) -> PreparedRenderer {
    let mut reference = PreparedRenderer::prepare(
        Arc::clone(&prepared.owner.control.plan),
        prepared.owner.control.anchor,
        prepared.epoch(),
        prepared.table_id(),
    )
    .expect("reference renderer");
    assert!(reference.bind_mixed_partition(Arc::clone(&prepared.owner.control.partition)));
    reference
}

fn render_reference(reference: &mut PreparedRenderer, events: &[TimedEvent]) -> Vec<f32> {
    let mut samples = vec![0.0_f32; FRAMES];
    for chunk in samples.chunks_mut(QUANTUM_FRAMES as usize) {
        let start = reference.clock().as_u64();
        let quanta = reference.quanta_needed_for(chunk.len()) as u64;
        let end = start + quanta * u64::from(QUANTUM_FRAMES);
        let due = events
            .iter()
            .copied()
            .filter(|event| (start..end).contains(&event.envelope().time().as_u64()))
            .collect::<Vec<_>>();
        let frames = chunk.len();
        reference
            .render(
                AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).expect("reference block"),
                TimedEvents::new(&due),
            )
            .expect("reference quantum");
    }
    samples
}

fn live_edge(epoch: StreamEpoch, at: u64, identity: NoteIdentity, edge: NoteEdge) -> TimedEvent {
    TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::new(at), TestOrigin::Simulated),
        EventPayload::Note { identity, edge },
    )
}

fn live_on(
    epoch: StreamEpoch,
    at: u64,
    identity: NoteIdentity,
    slot: NoteSlot,
    raw: u8,
) -> TimedEvent {
    live_edge(
        epoch,
        at,
        identity,
        NoteEdge::On {
            slot,
            key: key(raw),
            velocity: NoteVelocity::FULL,
        },
    )
}

fn assert_sum(mixed: &[f32], compiled: &[f32], live: &[f32]) {
    assert_eq!(mixed.len(), compiled.len());
    for (index, ((mixed, compiled), live)) in mixed.iter().zip(compiled).zip(live).enumerate() {
        assert!(
            (mixed - (compiled + live)).abs() < 0.00001,
            "frame {index}: mixed {mixed}, compiled {compiled}, live {live}"
        );
    }
}

fn assert_close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() < 0.00001,
            "frame {index}: {actual} != {expected}"
        );
    }
}

struct Run {
    samples: Vec<f32>,
    live_only: Vec<f32>,
    control: MixedOneShotControl,
    audio: MixedOneShotAudio,
    live: Option<NoteIdentity>,
}

/// Arm at 64, optionally hold test live key 48 from 0, optionally offer the seek to
/// position 400 at 256 before any callback, then render `partition`.
fn seek_run(compiled_first: bool, partition: &[usize], with_live: bool, with_offer: bool) -> Run {
    let prepared = bound(compiled_first);
    let candidate = first_candidate(&prepared, ARMED, PlanPosition::new(10));
    let mut reference = live_only_reference(&prepared);
    let epoch = prepared.epoch();
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let live = with_live.then(|| {
        audio
            .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("test live onset")
    });
    if with_offer {
        offer(&mut audio, sealed(&control, SEEK, SEEK_POSITION.as_u64()))
            .map_err(|(reason, _)| reason)
            .expect("offer accepted before the armed adoption");
    }
    let samples = render_owner(&mut audio, partition, FRAMES);
    let live_events = live
        .map(|identity| vec![live_on(epoch, 0, identity, audio.audio.note, 48)])
        .unwrap_or_default();
    let live_only = render_reference(&mut reference, &live_events);
    Run {
        samples,
        live_only,
        control,
        audio,
        live,
    }
}

/// The same destination reached by the first activation: armed straight at the seek.
fn direct_seek(compiled_first: bool) -> Vec<f32> {
    let prepared = bound(compiled_first);
    let candidate = first_candidate(&prepared, SEEK, SEEK_POSITION);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("direct arm");
    render_owner(&mut audio, PARTITIONS[0], FRAMES)
}

#[test]
fn an_offered_seek_adopts_at_its_boundary_and_leaves_held_live_notes_untouched() {
    for compiled_first in [true, false] {
        let base = seek_run(compiled_first, PARTITIONS[0], true, true);
        let restoration = base
            .audio
            .capsule
            .restoration_count
            .as_usize()
            .expect("restoration count");
        assert!(restoration > 0);
        let report = base.audio.report();
        assert!(report.adopted && !report.faulted, "{report:?}");
        assert_eq!(report.sequence, armed_successor().next().expect("later"));
        assert_eq!(
            report.effective_anchor,
            StreamAnchor::new(SEEK, SEEK_POSITION)
        );
        let receipt = MixedOfferReceipt {
            supersedes: armed_successor(),
            sequence: armed_successor().next().expect("later"),
            late: false,
            kind: MixedOfferKind::Seek,
            outcome: MixedOfferOutcome::Adopted(MixedOfferAdopted {
                effective: SEEK,
                anchor: StreamAnchor::new(SEEK, SEEK_POSITION),
                retired_anchor: StreamAnchor::new(ARMED, PlanPosition::new(10)),
                // The armed list's restoration and key 60's onset had published.
                retired_unconsumed_from: restoration + 1,
                retired_event_count: restoration + 4,
                released: HeldNoteCount::measured(1),
                stopped: false,
                loop_interval: None,
                retired_pass: None,
            }),
        };
        assert_eq!(report.offer, MixedOfferSlotState::Retired(receipt));
        // Both adoptions charged their restoration to Session; each charged one release.
        assert_eq!(report.restoration_charged.as_usize(), Some(restoration * 2));
        assert_eq!(report.releases_charged, EventCount::measured(2));
        assert_eq!(refused_offers(&base.audio), 0);

        // The later list's restoration leads it, then the destination-time onset.
        let audio = &base.audio;
        assert_eq!(audio.events.len(), restoration + 2);
        let shift = audio.timing.shift();
        for event in &audio.events[..restoration] {
            assert!(matches!(event.payload(), MixedCompiledPayload::Restore(_)));
            assert_eq!(event.envelope().time().checked_add(shift), Ok(SEEK));
        }
        assert!(matches!(
            audio.events[restoration].payload(),
            MixedCompiledPayload::On { key: k, .. } if k == key(72)
        ));
        assert_eq!(
            audio.events[restoration]
                .envelope()
                .time()
                .checked_add(shift),
            Ok(SEEK)
        );
        assert_eq!(audio.next, audio.events.len());
        // Key 72 ended at 356; only the held live note remains minted.
        assert!(audio.audio.compiled_minter.live_identities().is_empty());
        assert!(
            audio
                .audio
                .compiled_tokens
                .iter()
                .all(|token| *token == super::MixedCompiledTokenState::Free)
        );
        let live = base.live.expect("live identity");
        assert_eq!(audio.audio.live_minter.resolve(live), Resolution::Live);
        let sounding = audio
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("bound registry");
        assert!(sounding.compiled().is_empty());
        assert_eq!(
            sounding
                .live()
                .iter()
                .map(|n| n.identity)
                .collect::<Vec<_>>(),
            vec![live]
        );
        assert_eq!(audio.audio.renderer.seeded_row_count(), 0);

        // The live note renders exactly as a live-only renderer renders it, before and
        // after the boundary: the mixed output is the compiled-only run plus that.
        let compiled = seek_run(compiled_first, PARTITIONS[0], false, true);
        assert_sum(&base.samples, &compiled.samples, &base.live_only);
        assert!(base.live_only[SEEK_OUTPUT..].iter().any(|s| *s != 0.0));
        // Restoration did not silence the destination onset: key 72 sounds from the
        // boundary quantum, exactly as when the first activation seeks there.
        assert!(
            compiled.samples[SEEK_OUTPUT..SEEK_OUTPUT + 64]
                .iter()
                .any(|s| *s != 0.0)
        );
        assert_close(
            &compiled.samples[SEEK_OUTPUT..],
            &direct_seek(compiled_first)[SEEK_OUTPUT..],
        );

        // Falsifier control: without the offer key 60 keeps sounding past the boundary.
        let unoffered = seek_run(compiled_first, PARTITIONS[0], true, false);
        assert_eq!(
            &base.samples[..SEEK_OUTPUT],
            &unoffered.samples[..SEEK_OUTPUT]
        );
        assert_ne!(
            &base.samples[SEEK_OUTPUT..],
            &unoffered.samples[SEEK_OUTPUT..]
        );
        assert_eq!(unoffered.audio.report().offer, MixedOfferSlotState::Empty);

        for partition in &PARTITIONS[1..] {
            let run = seek_run(compiled_first, partition, true, true);
            assert_eq!(run.samples, base.samples, "{partition:?}");
            assert_eq!(run.audio.report(), report, "{partition:?}");
        }

        // Control's sequence moves only when it collects the confirming receipt.
        let Run {
            mut control,
            mut audio,
            ..
        } = base;
        assert_eq!(control.in_force, armed_successor());
        assert_eq!(control.in_force_effective, ARMED);
        let retired = take_retired(&mut audio).expect("retired offer");
        assert_eq!(audio.report().offer, MixedOfferSlotState::Empty);
        assert!(take_retired(&mut audio).is_none());
        assert_eq!(control.collect_offer(retired).expect("collected"), receipt);
        assert_eq!(control.in_force, receipt.sequence);
        assert_eq!(control.in_force_effective, SEEK);
        assert_eq!(
            control.control.anchor,
            StreamAnchor::new(SEEK, SEEK_POSITION)
        );
    }
}

/// Live key 48 is held across the seek and released after it; live key 49 starts
/// exactly on the seek boundary. Both reach audio through the private ingress store.
#[cfg(feature = "simulated-ingress")]
fn ingress_seek_run(
    compiled_first: bool,
    partition: &[usize],
    with_live: bool,
) -> (Vec<f32>, Vec<f32>) {
    let prepared = bound(compiled_first);
    let candidate = first_candidate(&prepared, ARMED, PlanPosition::new(10));
    let mut reference = live_only_reference(&prepared);
    let epoch = prepared.epoch();
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut live_events = Vec::new();
    if with_live {
        let held = audio
            .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("held onset");
        let on_boundary = audio
            .offer_test_note_on(SEEK, key(49), NoteVelocity::FULL)
            .expect("onset on the boundary");
        audio
            .offer_live_note_off(SampleTime::new(400), on_boundary)
            .expect("boundary onset release");
        audio
            .offer_live_note_off(SampleTime::new(480), held)
            .expect("held release");
        let slot = audio.audio.note;
        live_events = vec![
            live_on(epoch, 0, held, slot, 48),
            live_on(epoch, SEEK.as_u64(), on_boundary, slot, 49),
            live_edge(epoch, 400, on_boundary, NoteEdge::Off),
            live_edge(epoch, 480, held, NoteEdge::Off),
        ];
    }
    offer(&mut audio, sealed(&control, SEEK, SEEK_POSITION.as_u64()))
        .map_err(|(reason, _)| reason)
        .expect("offer accepted");
    let samples = render_owner(&mut audio, partition, FRAMES);
    assert!(matches!(
        audio.report().offer,
        MixedOfferSlotState::Retired(MixedOfferReceipt {
            outcome: MixedOfferOutcome::Adopted(_),
            ..
        })
    ));
    let sounding = audio
        .audio
        .renderer
        .snapshot_mixed_sounding()
        .expect("bound registry");
    assert!(sounding.compiled().is_empty());
    assert!(sounding.live().is_empty());
    (samples, render_reference(&mut reference, &live_events))
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn live_ingress_held_across_and_starting_on_the_seek_boundary_is_untouched() {
    for compiled_first in [true, false] {
        let (base, live_only) = ingress_seek_run(compiled_first, PARTITIONS[0], true);
        let (compiled, _) = ingress_seek_run(compiled_first, PARTITIONS[0], false);
        assert_sum(&base, &compiled, &live_only);
        // Key 49 is heard from the boundary.
        assert!(
            live_only[SEEK_OUTPUT..SEEK_OUTPUT + 64]
                .iter()
                .any(|s| *s != 0.0)
        );
        for partition in &PARTITIONS[1..] {
            let (samples, _) = ingress_seek_run(compiled_first, partition, true);
            assert_eq!(samples, base, "{partition:?}");
        }
    }
}

#[test]
fn an_occupied_slot_refuses_a_second_offer_and_returns_it_owned() {
    for compiled_first in [true, false] {
        let (mut control, mut audio) = armed(compiled_first);
        let seek = sealed(&control, SEEK, SEEK_POSITION.as_u64());
        // Sealing changes nothing, so a second offer can supersede the same sequence.
        let restart = sealed(&control, SampleTime::new(320), 10);
        offer(&mut audio, seek)
            .map_err(|(reason, _)| reason)
            .expect("first offer");
        let pending = audio.report();
        assert_eq!(
            pending.offer,
            MixedOfferSlotState::Pending {
                supersedes: armed_successor(),
                effective: SEEK,
                kind: MixedOfferKind::Seek,
            }
        );
        let before = custody(&audio);
        let (reason, restart) = offer(&mut audio, restart).expect_err("slot pending");
        assert_eq!(reason, MixedOfferRefused::AlreadyPending);
        assert_eq!(restart.0.timing.effective(), SampleTime::new(320));
        assert_eq!(restart.0.candidate.supersedes, armed_successor());
        assert_eq!(audio.report(), pending);
        assert_eq!(custody(&audio), before);
        assert_eq!(refused_offers(&audio), 1);

        // Past the boundary the slot holds the retired allocation until control takes it.
        render_owner(&mut audio, &quanta(384), 384);
        let MixedOfferSlotState::Retired(receipt) = audio.report().offer else {
            panic!("the seek must retire into the slot");
        };
        assert!(matches!(receipt.outcome, MixedOfferOutcome::Adopted(_)));
        let (reason, restart) = offer(&mut audio, restart).expect_err("retired uncollected");
        assert_eq!(reason, MixedOfferRefused::RetiredUncollected);
        assert_eq!(refused_offers(&audio), 2);

        // Taken, the slot is free, but the returned offer now names a superseded sequence.
        let retired = take_retired(&mut audio).expect("retired");
        let (reason, restart) = offer(&mut audio, restart).expect_err("stale");
        assert_eq!(
            reason,
            MixedOfferRefused::Superseded {
                supersedes: armed_successor(),
                in_force: receipt.sequence,
            }
        );
        assert_eq!(refused_offers(&audio), 3);
        drop(restart);
        // Until control collects, it still prepares against the old sequence, which
        // audio refuses; after collection a fresh candidate adopts at a later boundary.
        let stale = sealed(&control, SampleTime::new(448), 10);
        let (reason, _) = offer(&mut audio, stale).expect_err("stale control");
        assert!(matches!(reason, MixedOfferRefused::Superseded { .. }));
        assert_eq!(refused_offers(&audio), 4);
        assert_eq!(control.collect_offer(retired).expect("collected"), receipt);
        assert_eq!(control.in_force, receipt.sequence);
        let restart = sealed(&control, SampleTime::new(448), 10);
        offer(&mut audio, restart)
            .map_err(|(reason, _)| reason)
            .expect("second later offer");
        render_owner(&mut audio, &quanta(256), 256);
        let MixedOfferSlotState::Retired(second) = audio.report().offer else {
            panic!("the restart must retire into the slot");
        };
        assert_eq!(second.supersedes, receipt.sequence);
        let MixedOfferOutcome::Adopted(adopted) = second.outcome else {
            panic!("the restart must adopt: {second:?}");
        };
        assert_eq!(adopted.effective, SampleTime::new(448));
        assert_eq!(
            adopted.retired_anchor,
            StreamAnchor::new(SEEK, SEEK_POSITION)
        );
        assert_eq!(audio.in_force, second.sequence);
        let retired = take_retired(&mut audio).expect("second retired");
        assert_eq!(
            control.collect_offer(retired).expect("second collected"),
            second
        );
        assert_eq!(control.in_force, second.sequence);
        assert_eq!(refused_offers(&audio), 4);
    }
}

#[test]
fn stale_foreign_late_stop_pending_and_faulted_offers_change_nothing() {
    for compiled_first in [true, false] {
        // Foreign: sealed by another owner's control.
        let (_, mut audio) = armed(compiled_first);
        let (other, _other_audio) = armed(!compiled_first);
        let before = (custody(&audio), audio.report());
        let (reason, returned) = offer(&mut audio, sealed(&other, SEEK, 400)).expect_err("foreign");
        assert_eq!(reason, MixedOfferRefused::ForeignCandidate);
        assert_eq!(returned.0.timing.effective(), SEEK);
        assert_eq!((custody(&audio), audio.report()), before);
        assert_eq!(refused_offers(&audio), 1);

        // Late: the boundary's quantum has already opened.
        let (control, mut audio) = armed(compiled_first);
        render_owner(&mut audio, &quanta(384), 384);
        let clock = audio.audio.renderer.clock();
        assert_eq!(clock, SampleTime::new(320));
        let before = (custody(&audio), audio.report());
        let (reason, _) = offer(&mut audio, sealed(&control, SEEK, 400)).expect_err("late");
        assert_eq!(
            reason,
            MixedOfferRefused::BoundaryPassed {
                effective: SEEK,
                clock,
            }
        );
        assert_eq!((custody(&audio), audio.report()), before);
        // The quantum at the clock itself has not opened, so it is still offerable.
        offer(&mut audio, sealed(&control, clock, 400))
            .map_err(|(reason, _)| reason)
            .expect("the unopened clock quantum");

        // A pending Stop refuses an offer at or after its boundary; the transport
        // tests cover the rest of that law.
        let (control, mut audio) = armed(compiled_first);
        assert_eq!(
            audio.schedule_compiled_stop(SampleTime::new(192)),
            Ok(MixedStopAccepted::Scheduled)
        );
        let before = (custody(&audio), audio.report());
        let (reason, _) = offer(&mut audio, sealed(&control, SEEK, 400)).expect_err("pending");
        assert_eq!(
            reason,
            MixedOfferRefused::StopPending {
                stop: SampleTime::new(192),
                effective: SEEK,
            }
        );
        assert_eq!((custody(&audio), audio.report()), before);
        assert_eq!(refused_offers(&audio), 1);

        // Faulted.
        let (control, mut audio) = armed(compiled_first);
        audio.fault = Some(MixedOneShotRenderError::RendererFaulted);
        let before = (custody(&audio), audio.report());
        let (reason, _) = offer(&mut audio, sealed(&control, SEEK, 400)).expect_err("faulted");
        assert_eq!(reason, MixedOfferRefused::Faulted);
        assert_eq!((custody(&audio), audio.report()), before);
        assert_eq!(refused_offers(&audio), 1);
    }
}

#[test]
fn a_candidate_refused_at_its_boundary_retires_without_changing_the_program() {
    for compiled_first in [true, false] {
        let unoffered = seek_run(compiled_first, PARTITIONS[0], true, false);
        for partition in PARTITIONS {
            let (mut control, mut audio) = armed(compiled_first);
            let live = audio
                .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
                .expect("test live onset");

            offer(&mut audio, sealed(&control, SEEK, SEEK_POSITION.as_u64()))
                .map_err(|(reason, _)| reason)
                .expect("offer accepted");
            // The offer passed every check; force the state a defect would leave, so the
            // boundary's own validation is what refuses it.
            let MixedOfferSlot::Pending(pending) = &mut audio.offer_slot else {
                panic!("pending offer");
            };
            pending.0.candidate.supersedes = ActivationSequence::INITIAL;
            let samples = render_owner(&mut audio, partition, FRAMES);
            assert_eq!(samples, unoffered.samples, "{partition:?}");
            let report = audio.report();
            let receipt = MixedOfferReceipt {
                supersedes: ActivationSequence::INITIAL,
                sequence: armed_successor().next().expect("later"),
                late: false,
                kind: MixedOfferKind::Seek,
                outcome: MixedOfferOutcome::Refused(MixedOfferRefused::Superseded {
                    supersedes: ActivationSequence::INITIAL,
                    in_force: armed_successor(),
                }),
            };
            assert_eq!(report.offer, MixedOfferSlotState::Retired(receipt));
            assert_eq!(
                MixedOneShotRenderReport {
                    offer: MixedOfferSlotState::Empty,
                    ..report
                },
                unoffered.audio.report()
            );
            assert_eq!(refused_offers(&audio), 1);
            assert_eq!(audio.audio.live_minter.resolve(live), Resolution::Live);
            // A refused receipt is collected, but control's sequence stays.
            let retired = take_retired(&mut audio).expect("retired");
            assert_eq!(control.collect_offer(retired).expect("collected"), receipt);
            assert_eq!(control.in_force, armed_successor());
            assert_eq!(control.in_force_effective, ARMED);
        }
    }
}

#[test]
fn sealing_refuses_stale_foreign_early_and_unadmitted_candidates_unchanged() {
    let (control, _audio) = armed(true);
    let (other, _other_audio) = armed(false);
    let unchanged = |control: &MixedOneShotControl| {
        assert_eq!(control.in_force, armed_successor());
        assert_eq!(control.in_force_effective, ARMED);
    };

    let foreign = later(&other, SEEK, 400);
    let refusal = control
        .seal_offer(foreign, SEEK, MixedOfferKind::Seek)
        .expect_err("foreign");
    assert_eq!(refusal.0, MixedOfferPrepareError::ForeignCandidate);
    assert_eq!(refusal.1.anchor(), StreamAnchor::new(SEEK, SEEK_POSITION));

    let history = control
        .control
        .prepare_history(SEEK, SEEK_POSITION)
        .expect("history");
    let suffix = control
        .control
        .prepare_suffix(history)
        .map_err(|refusal| refusal.0)
        .expect("suffix");
    let stale = control
        .control
        .stamp_suffix(suffix, ActivationSequence::INITIAL)
        .map_err(|refusal| refusal.0)
        .expect("stale stamp");
    let refusal = control
        .seal_offer(stale, SEEK, MixedOfferKind::Seek)
        .expect_err("stale");
    assert_eq!(
        refusal.0,
        MixedOfferPrepareError::Superseded {
            supersedes: ActivationSequence::INITIAL,
            in_force: armed_successor(),
        }
    );

    for effective in [SampleTime::ZERO, ARMED] {
        let refusal = control
            .seal_offer(
                later(&control, SampleTime::new(65), 400),
                effective,
                MixedOfferKind::Seek,
            )
            .expect_err("not after the armed boundary");
        assert_eq!(
            refusal.0,
            MixedOfferPrepareError::NotAfterInForce {
                effective,
                boundary: ARMED,
            }
        );
    }
    let refusal = control
        .seal_offer(
            later(&control, SampleTime::new(300), 400),
            SampleTime::new(300),
            MixedOfferKind::Seek,
        )
        .expect_err("off grid");
    assert_eq!(
        refusal.0,
        MixedOfferPrepareError::Timing(MixedEffectiveTimeError::NotQuantumBoundary {
            effective: SampleTime::new(300),
        })
    );
    let refusal = control
        .seal_offer(
            later(&control, SampleTime::new(330), 400),
            SampleTime::new(320),
            MixedOfferKind::Seek,
        )
        .expect_err("before requested");
    assert_eq!(
        refusal.0,
        MixedOfferPrepareError::Timing(MixedEffectiveTimeError::BeforeRequested {
            requested: SampleTime::new(330),
            effective: SampleTime::new(320),
        })
    );

    // Admission runs at the seal: a list whose count disagrees with itself is refused.
    let mut miscounted = later(&control, SEEK, 400);
    let count = miscounted.event_count;
    miscounted.event_count = EventCount::NONE;
    let refusal = control
        .seal_offer(miscounted, SEEK, MixedOfferKind::Seek)
        .expect_err("unadmitted");
    assert_eq!(
        refusal.0,
        MixedOfferPrepareError::Admission(MixedOneShotAdmissionError::CandidateCount)
    );
    let mut repaired = refusal.1;
    assert_eq!(repaired.event_count, EventCount::NONE);
    repaired.event_count = count;
    let offer = control
        .seal_offer(repaired, SEEK, MixedOfferKind::Seek)
        .expect("repaired seal");
    assert_eq!(offer.0.timing.effective(), SEEK);
    assert_eq!(
        offer.0.candidate.sequence,
        armed_successor().next().expect("later")
    );
    assert!(offer.0.ended.iter().all(Option::is_none));
    assert_eq!(
        offer.0.ended.len(),
        control.control.compiled_span.indices().len()
    );
    unchanged(&control);
}

#[test]
fn joined_collection_hands_a_pending_or_retired_offer_back_owned() {
    for (at, frames) in [(SampleTime::new(448), 256), (SEEK, 384)] {
        let (control, mut audio) = armed(true);
        offer(&mut audio, sealed(&control, at, 400))
            .map_err(|(reason, _)| reason)
            .expect("offer");
        render_owner(&mut audio, &quanta(frames), frames);
        let slot = audio.report().offer;
        let MixedCollection::Resumed {
            control,
            audio,
            offer,
            ..
        } = control.collect(audio).expect("adopted pair")
        else {
            panic!("an adopted pair resumes");
        };
        // The slot moved to the caller whole; the resumed half holds nothing.
        assert_eq!(audio.report().offer, MixedOfferSlotState::Empty);
        match (slot, *offer) {
            (
                MixedOfferSlotState::Pending { effective, .. },
                MixedDrainedOffer::Withdrawn(offer),
            ) => {
                assert_eq!(effective, at);
                assert_eq!(offer.0.timing.effective(), at);
                assert_eq!(control.sequence, armed_successor());
            }
            (MixedOfferSlotState::Retired(receipt), MixedDrainedOffer::Retired(retired)) => {
                assert_eq!(retired.receipt, receipt);
                assert!(matches!(receipt.outcome, MixedOfferOutcome::Adopted(_)));
                // The adoption's boundary record travels with it.
                assert_eq!(retired.cell.released, HeldNoteCount::measured(1));
                assert_eq!(control.sequence, receipt.sequence);
                assert_eq!(
                    control.control.anchor,
                    StreamAnchor::new(SEEK, SEEK_POSITION)
                );
            }
            (slot, offer) => panic!("the drained offer {offer:?} does not match {slot:?}"),
        }
    }
}

#[test]
fn offers_and_retired_offers_are_sendable() {
    fn require_send<T: Send>() {}
    require_send::<MixedCandidateOffer>();
    require_send::<MixedRetiredOffer>();
}

/// Arm with test live key 48 held from 0, offer the seek, and render up to the
/// unopened seek boundary, returning key 60's compiled occurrence just before it.
fn adopted_seek_before_terminal(
    compiled_first: bool,
) -> (
    MixedOneShotControl,
    MixedOneShotAudio,
    NoteIdentity,
    NoteIdentity,
) {
    let (control, mut audio) = armed(compiled_first);
    let live = audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("test live onset");
    offer(&mut audio, sealed(&control, SEEK, SEEK_POSITION.as_u64()))
        .map_err(|(reason, _)| reason)
        .expect("offer");
    render_owner(&mut audio, &quanta(SEEK_OUTPUT), SEEK_OUTPUT);
    let old = audio.audio.compiled_minter.live_identities();
    assert_eq!(old.len(), 1, "key 60 sounds up to the seek");
    (control, audio, old[0], live)
}

/// Fault the owner through a real callback: the capsule names another table.
fn fault_by_pairing(audio: &mut MixedOneShotAudio, compiled_first: bool) {
    let foreign = bound(compiled_first).table_id();
    assert_ne!(foreign, audio.capsule.table);
    audio.capsule.table = foreign;
    let mut samples = [1.0_f32; 1];
    let error = audio
        .render_private(AudioBlockMut::new(&mut samples, 1, ChannelLayout::Mono).expect("block"))
        .expect_err("capsule table mismatch");
    assert_eq!(error, MixedOneShotRenderError::Pairing);
    assert_eq!(samples, [0.0]);
}

fn identities(notes: &[crate::identity::EndedNote]) -> Vec<NoteIdentity> {
    notes.iter().map(|note| note.identity).collect()
}

#[test]
fn terminal_teardown_after_a_later_adoption_keeps_its_uncollected_boundary_record() {
    for compiled_first in [true, false] {
        let (control, mut audio, old, live) = adopted_seek_before_terminal(compiled_first);
        render_owner(&mut audio, &quanta(64), 64);
        let MixedOfferSlotState::Retired(receipt) = audio.report().offer else {
            panic!("the seek retired its offer");
        };
        let MixedOfferOutcome::Adopted(adopted) = receipt.outcome else {
            panic!("the seek adopted");
        };
        assert_eq!(adopted.released, HeldNoteCount::measured(1));
        // Key 72 started at the seek and still sounds; it is the only compiled custody.
        let new = audio.audio.compiled_minter.live_identities();
        assert_eq!(new.len(), 1);
        assert_ne!(new[0], old);
        fault_by_pairing(&mut audio, compiled_first);

        let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown")
        else {
            panic!("a faulted owner ends");
        };
        assert_eq!(ended.end, MixedCollectionEnd::Faulted);
        assert_eq!(ended.report.offer, MixedOfferSlotState::Retired(receipt));
        // Teardown owns the retired allocation itself, not only its receipt.
        assert!(matches!(
            &ended.offer,
            MixedDrainedOffer::Retired(retired) if retired.receipt == receipt
        ));
        // The seek's release is recorded once, under the seek. The armed adoption ended
        // the initial list's own key 60, a different occurrence, in its separate record.
        assert_eq!(identities(&ended.offer_boundary_ended), vec![old]);
        assert_eq!(ended.report.released_compiled, HeldNoteCount::measured(1));
        assert_eq!(ended.boundary_ended.len(), 1);
        assert_ne!(ended.boundary_ended[0].identity, old);
        // The ended note is neither sounding nor minted; key 72 and live 48 are both.
        assert_eq!(ended.compiled_minted, new);
        assert_eq!(identities(ended.sounding.compiled()), new);
        assert_eq!(identities(ended.sounding.live()), vec![live]);
    }
}

#[test]
fn terminal_teardown_after_a_collected_later_adoption_reports_no_offer_record() {
    for compiled_first in [true, false] {
        let (mut control, mut audio, old, _) = adopted_seek_before_terminal(compiled_first);
        render_owner(&mut audio, &quanta(64), 64);
        let retired = take_retired(&mut audio).expect("retired offer");
        let receipt = control.collect_offer(retired).expect("collected");
        assert!(matches!(receipt.outcome, MixedOfferOutcome::Adopted(_)));
        fault_by_pairing(&mut audio, compiled_first);
        let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown")
        else {
            panic!("a faulted owner ends");
        };
        // Control took the record with its receipt; teardown does not report it again.
        assert_eq!(ended.report.offer, MixedOfferSlotState::Empty);
        assert!(matches!(ended.offer, MixedDrainedOffer::Empty));
        assert!(ended.offer_boundary_ended.is_empty());
        assert_eq!(ended.boundary_ended.len(), 1);
        assert_ne!(ended.boundary_ended[0].identity, old);
    }
}

#[test]
fn a_custody_fault_after_the_seek_release_keeps_the_pending_cell_record() {
    for compiled_first in [true, false] {
        let (control, mut audio, old, _) = adopted_seek_before_terminal(compiled_first);
        // Fault injection: the minter loses key 60 before the seek's release reaches it.
        assert_eq!(audio.audio.compiled_minter.release(old), Resolution::Live);
        let mut samples = [1.0_f32; 64];
        let error = audio
            .render_private(
                AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono).expect("block"),
            )
            .expect_err("custody fault at the seek");
        assert!(
            matches!(error, MixedOneShotRenderError::CompiledRelease { .. }),
            "{error:?}"
        );
        assert!(samples.iter().all(|sample| *sample == 0.0));
        let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown")
        else {
            panic!("a faulted owner ends");
        };
        assert_eq!(ended.end, MixedCollectionEnd::Faulted);
        // The cell went back to the slot unadopted, but the renderer's release ended key
        // 60 and the record says so.
        assert!(matches!(
            ended.report.offer,
            MixedOfferSlotState::Pending {
                effective: SEEK,
                ..
            }
        ));
        assert_eq!(identities(&ended.offer_boundary_ended), vec![old]);
        assert!(matches!(
            &ended.offer,
            MixedDrainedOffer::Withdrawn(offer) if offer.0.released == HeldNoteCount::measured(1)
        ));
        assert_eq!(ended.boundary_ended.len(), 1);
        assert_ne!(ended.boundary_ended[0].identity, old);
        assert!(ended.sounding.compiled().is_empty());
    }
}
