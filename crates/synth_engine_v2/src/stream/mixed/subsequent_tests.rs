//! Off-thread preparation of a later Start or seek candidate after the private arm.
//!
//! Control holds no minter (draft S1), so preparing a candidate must mint, free and
//! reserve nothing on audio. Each candidate names the sequence it supersedes, which
//! must be the one the armed transition puts in force, and must request a time after
//! the in-force adoption boundary (draft Item 2, rule 2.1). No candidate is offered here.

use super::history_tests::{bound_with_events, key, mixed_profile};
use super::*;
use crate::quantities::{ChannelLayout, NoteVelocity};
use crate::render::AudioBlockMut;
use crate::schedule::PlanEvent;
use crate::time::QUANTUM_FRAMES;

/// The armed transition's requested time; with a stopped clock at zero it adopts here.
const ARMED: SampleTime = SampleTime::new(64);
const FRAMES: usize = 640;

/// Compiled key 60 from 10 to 300 and key 72 from 400 to 500, armed at position 10.
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

fn first_candidate(prepared: &MixedJoinedPrepared, requested: SampleTime) -> MixedStampedCandidate {
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    prepared.stamp_suffix(suffix).expect("stamp")
}

fn armed_at(
    compiled_first: bool,
    requested: SampleTime,
) -> (MixedOneShotControl, MixedOneShotAudio) {
    let prepared = bound(compiled_first);
    let candidate = first_candidate(&prepared, requested);
    prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm")
}

fn armed_successor() -> ActivationSequence {
    ActivationSequence::INITIAL.next().expect("successor")
}

/// Render `frames` in quantum-sized callbacks, asserting each allocates nothing.
fn render(audio: &mut MixedOneShotAudio, frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0_f32; frames];
    for chunk in samples.chunks_mut(QUANTUM_FRAMES as usize) {
        let len = chunk.len();
        let output = AudioBlockMut::new(chunk, len, ChannelLayout::Mono).expect("mono block");
        let allocations = crate::render_allocation::count_allocs(|| {
            audio.render_private(output).expect("private render");
        });
        assert_eq!(allocations, 0, "callback allocated or deallocated");
    }
    samples
}

/// Everything audio's identity custody holds: both minters, the token table and the
/// registry's sounding set.
fn custody(audio: &MixedOneShotAudio) -> String {
    format!(
        "{:?} {:?} {:?} {:?} {:?}",
        audio.audio.compiled_minter,
        audio.audio.live_minter,
        audio.audio.compiled_tokens,
        audio.audio.compiled_ended,
        audio.audio.renderer.snapshot_mixed_sounding(),
    )
}

fn first_token(candidate: &MixedStampedCandidate) -> Option<MixedNoteToken> {
    candidate
        .events
        .iter()
        .find_map(|event| match event.payload() {
            MixedCompiledPayload::On { token, .. } => Some(token),
            _ => None,
        })
}

struct Run {
    samples: Vec<f32>,
    report: MixedOneShotRenderReport,
}

/// Adopt, render past the adoption with compiled and live notes sounding, optionally
/// prepare `prepare` from the armed control, then render the rest.
fn run(
    compiled_first: bool,
    prepare: impl FnOnce(&MixedOneShotControl, &MixedOneShotAudio),
) -> Run {
    let (control, mut audio) = armed_at(compiled_first, ARMED);
    let _live = audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("test live onset");
    let mut samples = render(&mut audio, 256);
    assert!(audio.report().adopted);
    prepare(&control, &audio);
    samples.extend(render(&mut audio, FRAMES - 256));
    Run {
        samples,
        report: audio.report(),
    }
}

#[test]
fn two_later_candidates_from_one_armed_control_mint_and_free_nothing() {
    for compiled_first in [true, false] {
        let mut prepared = Vec::new();
        let with = run(compiled_first, |control, audio| {
            // Key 60 and the test live note are both minted and sounding here.
            assert!(!audio.audio.compiled_minter.live_identities().is_empty());
            assert!(
                audio
                    .audio
                    .compiled_tokens
                    .iter()
                    .any(|token| *token != super::MixedCompiledTokenState::Free)
            );
            let before = custody(audio);
            let in_force = control.in_force;
            let requested = SampleTime::new(320);
            let first = control
                .prepare_subsequent(requested, PlanPosition::new(300), armed_successor())
                .expect("first later candidate");
            let second = control
                .prepare_subsequent(requested, PlanPosition::new(300), armed_successor())
                .expect("second later candidate");
            let other = control
                .prepare_subsequent(
                    SampleTime::new(448),
                    PlanPosition::new(10),
                    armed_successor(),
                )
                .expect("other later candidate");
            assert_eq!(custody(audio), before, "preparation touched audio custody");
            assert_eq!(control.in_force, in_force, "preparation moved control");
            prepared.extend([first, second, other]);
        });
        let without = run(compiled_first, |_, _| {});
        // Audio rendered on exactly as if nothing had been prepared.
        assert_eq!(with.samples, without.samples);
        assert_eq!(with.report, without.report);
        assert!(with.report.adopted && !with.report.faulted);

        let [first, second, other] = <[_; 3]>::try_from(prepared).expect("three candidates");
        // Preparing changes nothing, so both candidates supersede the same sequence
        // and only one could ever adopt.
        for candidate in [&first, &second, &other] {
            assert_eq!(candidate.supersedes(), armed_successor());
        }
        assert_eq!(first.events, second.events);
        assert_eq!(first.anchor(), second.anchor());
        // The seek to 300 crosses key 60's release and keeps only key 72's pair.
        assert_eq!(first.anchor().position(), PlanPosition::new(300));
        assert_eq!(
            first.suffix.omitted_release_count(),
            EventCount::measured(1)
        );
        assert_eq!(
            first.suffix.open_at_destination_count(),
            HeldNoteCount::measured(1)
        );
        assert_eq!(first.outstanding_count(), HeldNoteCount::NONE);
        let restoration = first.restoration_count();
        assert_eq!(
            first.event_count().as_usize(),
            restoration.as_usize().map(|count| count + 2)
        );
        // The source history gave its restoration batch to the sealed list.
        assert_eq!(first.suffix.history.restoration_count, EventCount::NONE);
        assert!(first.suffix.history.restoration.is_empty());
        // The Start from 10 keeps all four edges and crosses nothing.
        assert_eq!(
            other.anchor(),
            StreamAnchor::new(SampleTime::new(448), PlanPosition::new(10))
        );
        assert_eq!(other.suffix.omitted_release_count(), EventCount::NONE);
        assert_eq!(
            other.event_count().as_usize(),
            restoration.as_usize().map(|count| count + 4)
        );
        // Pairing starts from an empty scratch occupancy: token 0 is reused while
        // audio's own token 0 still names a minted occurrence, because the boundary
        // release ends every old compiled occurrence before the list publishes.
        assert_eq!(first_token(&first), Some(MixedNoteToken(0)));
        assert_eq!(first_token(&other), Some(MixedNoteToken(0)));
        // Each candidate seals into a capsule for the successor of what it supersedes.
        let capsule = other.into_audio_capsule().expect("sealed capsule");
        assert_eq!(capsule.supersedes, armed_successor());
        assert_eq!(Some(capsule.sequence), armed_successor().next());
    }
}

#[test]
fn a_later_candidate_naming_a_stale_or_unissued_sequence_is_refused() {
    let (control, audio) = armed_at(true, ARMED);
    let before = custody(&audio);
    let unissued = armed_successor().next().expect("unissued");
    for supersedes in [ActivationSequence::INITIAL, unissued] {
        assert_eq!(
            control
                .prepare_subsequent(SampleTime::new(320), PlanPosition::new(300), supersedes)
                .map(drop),
            Err(MixedSubsequentPrepareError::Superseded {
                supersedes,
                in_force: armed_successor(),
            })
        );
    }
    // The stale request is refused before any timing or build check.
    assert_eq!(
        control
            .prepare_subsequent(ARMED, PlanPosition::new(300), ActivationSequence::INITIAL)
            .map(drop),
        Err(MixedSubsequentPrepareError::Superseded {
            supersedes: ActivationSequence::INITIAL,
            in_force: armed_successor(),
        })
    );
    assert_eq!(custody(&audio), before);
    assert_eq!(control.in_force, armed_successor());
    // Falsifier control: the in-force sequence is accepted by the same control.
    let _accepted = control
        .prepare_subsequent(
            SampleTime::new(320),
            PlanPosition::new(300),
            armed_successor(),
        )
        .expect("current sequence");
}

#[test]
fn a_later_candidate_must_follow_the_armed_adoption_boundary() {
    // Requested at 32 from a stopped clock, the armed transition adopts at 64.
    for (armed_request, armed) in [(ARMED, ARMED), (SampleTime::new(32), ARMED)] {
        let (control, _audio) = armed_at(true, armed_request);
        assert_eq!(control.in_force_effective, armed);
        for requested in [SampleTime::ZERO, SampleTime::new(33), armed] {
            assert_eq!(
                control
                    .prepare_subsequent(requested, PlanPosition::new(300), armed_successor())
                    .map(drop),
                Err(MixedSubsequentPrepareError::NotAfterInForce {
                    requested,
                    boundary: armed
                })
            );
        }
        // One frame later snaps to the next boundary, so the two adoptions never share one.
        let requested = SampleTime::new(armed.as_u64() + 1);
        let later = control
            .prepare_subsequent(requested, PlanPosition::new(300), armed_successor())
            .expect("after the armed boundary");
        assert_eq!(later.anchor().time(), requested);
        let boundary = SampleTime::new(armed.as_u64() + u64::from(QUANTUM_FRAMES));
        assert_eq!(
            later.effective_timing(boundary).map(|t| t.effective()),
            Ok(boundary)
        );
        assert_eq!(
            later.effective_timing(armed).map(drop),
            Err(MixedEffectiveTimeError::BeforeRequested {
                requested,
                effective: armed,
            })
        );
    }

    let (control, _audio) = armed_at(false, ARMED);
    let at = SampleTime::new(u64::MAX);
    assert_eq!(
        control
            .prepare_subsequent(at, PlanPosition::new(300), armed_successor())
            .map(drop),
        Err(MixedSubsequentPrepareError::History(
            MixedHistoryPrepareError::BoundaryUnrepresentable { at }
        ))
    );
}

#[test]
fn the_first_activation_builders_still_supersede_initial_and_arm_refuses_a_later_one() {
    let prepared = bound(true);
    let first = first_candidate(&prepared, ARMED);
    assert_eq!(first.supersedes(), ActivationSequence::INITIAL);

    // A list sealed as a later candidate cannot be armed as the first activation.
    let history = prepared
        .prepare_history(ARMED, PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let later = prepared
        .owner
        .control
        .stamp_suffix(suffix, armed_successor())
        .expect("later seal");
    assert_eq!(later.events, first.events);
    let refusal = prepared
        .arm_one_shot(later, &mixed_profile())
        .expect_err("wrong sequence");
    assert_eq!(refusal.reason, MixedOneShotArmError::WrongSequence);
    assert!(
        refusal
            .owner
            .owner
            .audio
            .compiled_minter
            .untouched_for_test()
    );
    assert_eq!(refusal.candidate.supersedes(), armed_successor());

    // The returned owner still arms its first candidate.
    let (control, _audio) = refusal
        .owner
        .arm_one_shot(first, &mixed_profile())
        .expect("first arm");
    assert_eq!(control.in_force, armed_successor());
}

#[test]
fn shared_builders_return_their_consumed_input_unchanged_on_refusal() {
    let prepared = bound(true);
    let control = &prepared.owner.control;

    // A crossing release closes key 60 in the classification's book before key 64's
    // release, which neither the suffix nor the tampered book owns, refuses.
    let prepared_two = bound_with_events(true, |note| {
        let edge = |at, raw, on: bool| {
            let payload = if on {
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(raw),
                    velocity: NoteVelocity::FULL,
                }
            } else {
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(raw),
                }
            };
            PlanEvent::new(PlanPosition::new(at), payload)
        };
        vec![
            edge(10, 60, true),
            edge(20, 64, true),
            edge(300, 60, false),
            edge(310, 64, false),
        ]
    });
    let two = &prepared_two.owner.control;
    let only_60 = two
        .prepare_history(ARMED, PlanPosition::new(15))
        .expect("history holding key 60");
    let mut tampered = two
        .prepare_history(ARMED, PlanPosition::new(300))
        .expect("history holding both keys");
    assert_eq!(tampered.book.entries().len(), 2);
    tampered.book = only_60.book;
    let refusal = two.prepare_suffix(tampered).map(drop).expect_err("refused");
    let (error, returned) = *refusal;
    assert_eq!(
        error,
        MixedSuffixPrepareError::UnmatchedRelease { event_index: 3 }
    );
    let entries = returned.book.entries();
    assert_eq!(
        entries.len(),
        1,
        "the refused book lost key 60's crossing note"
    );
    assert_eq!(entries[0].key, key(60));
    assert_eq!(returned.position, PlanPosition::new(300));

    // A foreign history comes back whole.
    let refusal = control
        .prepare_suffix(returned)
        .map(drop)
        .expect_err("foreign");
    let (error, returned) = *refusal;
    assert_eq!(error, MixedSuffixPrepareError::ForeignCandidate);
    assert_eq!(returned.plan_id(), two.plan.id());

    // A refused stamp keeps the suffix's restoration batch and selection.
    let history = control
        .prepare_history(ARMED, PlanPosition::new(10))
        .expect("history");
    let restoration = history.restoration.clone();
    assert!(!restoration.is_empty());
    let mut suffix = control.prepare_suffix(history).expect("suffix");
    suffix.included.push(usize::MAX);
    suffix.included_count = EventCount::measured(5);
    let refusal = control
        .stamp_suffix(suffix, ActivationSequence::INITIAL)
        .map(drop)
        .expect_err("invalid selection");
    let (error, mut suffix) = *refusal;
    assert_eq!(
        error,
        MixedStampPrepareError::InvalidSelection {
            event_index: usize::MAX
        }
    );
    assert_eq!(suffix.history.restoration, restoration);
    assert_eq!(
        suffix.history.restoration_count.as_usize(),
        Some(restoration.len())
    );

    // Repaired, the same suffix seals with the same batch at the front.
    suffix.included.pop();
    suffix.included_count = EventCount::measured(4);
    let sealed = control
        .stamp_suffix(suffix, ActivationSequence::INITIAL)
        .expect("repaired stamp");
    assert_eq!(sealed.restoration_count.as_usize(), Some(restoration.len()));
    assert!(matches!(
        sealed.events[0].payload(),
        MixedCompiledPayload::Restore(_)
    ));
}
