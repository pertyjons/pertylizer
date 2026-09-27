use super::*;
use crate::{
    recording::notes::Midi1Input,
    time::{FrameCount, issue_epoch},
};

fn ready_with_cells(uncertainty: u64, cells: u32) -> (SimulatedNoteInput, ConnectionGeneration) {
    let mut input = SimulatedNoteInput::new(
        EndpointId::new("oracle".to_owned()).unwrap(),
        InputLimits {
            cells: InputCapacity::new(cells).unwrap(),
            bytes: PreparedBytes::measured(65536),
        },
    )
    .unwrap();
    let generation = input.begin().unwrap();
    input
        .prepare(
            generation,
            SimulatedInputClock::new(
                issue_epoch().unwrap(),
                SampleTime::ZERO,
                InputTick::new(0),
                InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
                InputTickSpan::new(uncertainty),
            ),
        )
        .unwrap();
    input.start(generation).unwrap();
    (input, generation)
}
fn ready(uncertainty: u64) -> (SimulatedNoteInput, ConnectionGeneration) {
    ready_with_cells(uncertainty, 6)
}

#[test]
fn raw_pressure_counts_retained_cells_and_release_reservations() {
    let (mut input, generation) = ready_with_cells(0, 8);
    let pressure = |input: &SimulatedNoteInput| {
        let pressure = input.pressure();
        (
            pressure.occupied().as_usize(),
            pressure.release_reservations().as_usize(),
            pressure.capacity().as_u32(),
        )
    };
    assert_eq!(pressure(&input), (1, 0, 8));
    for (time, velocity) in [(10, 100), (20, 110)] {
        let _id = input
            .offer_message(
                generation,
                InputTick::new(time),
                SampleTime::new(time),
                Midi1Input::from_bytes([0x90, 60, velocity]).unwrap(),
            )
            .unwrap();
    }
    assert_eq!(pressure(&input), (3, 2, 8));
    let _release = input
        .offer_message(
            generation,
            InputTick::new(30),
            SampleTime::new(30),
            Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
        )
        .unwrap();
    assert_eq!(pressure(&input), (4, 1, 8));
    let _frontier = input
        .advance_frontier(generation, InputTick::new(40))
        .unwrap();
    assert_eq!(pressure(&input), (5, 1, 8));
    input.device_lost(generation).unwrap();
    assert_eq!(pressure(&input), (5, 0, 8));
    assert_eq!(std::iter::from_fn(|| input.collect()).count(), 5);
    assert_eq!(pressure(&input), (0, 0, 8));
}
#[test]
fn uncertainty_and_identity_exhaustion_keep_first_failure_and_accepted_prefix() {
    for uncertain in [false, true] {
        let (mut input, generation) = ready(u64::from(uncertain));
        if !uncertain {
            input.serial = u64::MAX;
        }
        let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
        let expected = if uncertain {
            InputError::Uncertain
        } else {
            InputError::IdentityExhausted
        };
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                assert_eq!(
                    input.offer_message(generation, InputTick::new(10), SampleTime::new(10), note),
                    Err(expected)
                );
            }),
            0
        );
        let failure = input.discontinuity().unwrap();
        assert_eq!(failure.reason, expected);
        assert!(matches!(
            failure.observation,
            Some(InputObservation::Message { .. })
        ));
        assert_eq!(
            input.offer_message(generation, InputTick::new(11), SampleTime::new(11), note),
            Err(InputError::State)
        );
        assert_eq!(input.discontinuity(), Some(failure));
        assert_eq!(input.collect().unwrap().id.serial(), 1);
        input.acknowledge_quiescence(generation).unwrap();
        input.retire(generation).unwrap();
        let replacement = input.begin().unwrap();
        assert_ne!(replacement, generation);
    }
}
#[test]
fn source_regression_and_nominal_before_frontier_are_discontinuities() {
    for past_frontier in [false, true] {
        let (mut input, generation) = ready(0);
        let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
        if past_frontier {
            let _id = input
                .advance_frontier(generation, InputTick::new(20))
                .unwrap();
        } else {
            let _id = input
                .offer_message(generation, InputTick::new(20), SampleTime::new(25), note)
                .unwrap();
        }
        assert_eq!(
            input.offer_message(generation, InputTick::new(19), SampleTime::new(30), note),
            Err(InputError::Order)
        );
        assert_eq!(input.state(), ConnectionState::Quiescing);
        assert_eq!(std::iter::from_fn(|| input.collect()).count(), 2);
    }
}

#[test]
fn raw_admission_preserves_a_held_notes_release_and_frontier_cells() {
    let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
    let release = Midi1Input::from_bytes([0x80, 60, 0]).unwrap();
    let (mut admitted, generation) = ready(0);
    let _id = admitted
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    assert_eq!(admitted.release_reservations, 1);
    let _id = admitted
        .offer_message(
            generation,
            InputTick::new(11),
            SampleTime::new(11),
            Midi1Input::from_bytes([0xB0, 64, 127]).unwrap(),
        )
        .unwrap();
    let _id = admitted
        .advance_frontier(generation, InputTick::new(12))
        .unwrap();
    let _id = admitted
        .offer_message(generation, InputTick::new(13), SampleTime::new(13), release)
        .unwrap();
    assert_eq!(admitted.release_reservations, 0);
    assert_eq!(admitted.state(), ConnectionState::Running);

    let (mut refused, generation) = ready_with_cells(0, 4);
    assert_eq!(
        refused.offer_message(generation, InputTick::new(10), SampleTime::new(10), note),
        Err(InputError::ProtectedCapacity)
    );
    assert_eq!(
        refused.discontinuity().unwrap().reason,
        InputError::ProtectedCapacity
    );

    let (mut frontier_only, generation) = ready_with_cells(0, 5);
    let _id = frontier_only
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    let _id = frontier_only
        .advance_frontier(generation, InputTick::new(11))
        .unwrap();
    let _id = frontier_only
        .offer_message(generation, InputTick::new(12), SampleTime::new(12), release)
        .unwrap();
    assert_eq!(frontier_only.state(), ConnectionState::Running);

    let (mut frontier_refused, generation) = ready_with_cells(0, 5);
    let _id = frontier_refused
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    let _id = frontier_refused
        .advance_frontier(generation, InputTick::new(11))
        .unwrap();
    assert_eq!(
        frontier_refused.advance_frontier(generation, InputTick::new(12)),
        Err(InputError::ProtectedCapacity)
    );

    let (mut unmatched, generation) = ready_with_cells(0, 5);
    let _id = unmatched
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    let _id = unmatched
        .advance_frontier(generation, InputTick::new(11))
        .unwrap();
    assert_eq!(
        unmatched.offer_message(
            generation,
            InputTick::new(12),
            SampleTime::new(12),
            Midi1Input::from_bytes([0x80, 61, 0]).unwrap(),
        ),
        Err(InputError::ProtectedCapacity)
    );

    let (mut refused, generation) = ready(0);
    let _id = refused
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    let _id = refused
        .advance_frontier(generation, InputTick::new(11))
        .unwrap();
    assert_eq!(
        refused.offer_message(
            generation,
            InputTick::new(12),
            SampleTime::new(12),
            Midi1Input::from_bytes([0xB0, 64, 127]).unwrap(),
        ),
        Err(InputError::ProtectedCapacity)
    );
    assert_eq!(refused.state(), ConnectionState::Quiescing);
    assert_eq!(refused.release_reservations, 0);

    let (mut full, generation) = ready_with_cells(0, 2);
    let _id = full
        .advance_frontier(generation, InputTick::new(10))
        .unwrap();
    assert_eq!(
        full.preflight_observation(
            generation,
            InputObservation::Frontier {
                tick: InputTick::new(11),
            },
        ),
        Err(InputError::Full)
    );
    assert_eq!(full.state(), ConnectionState::Running);
    assert_eq!(full.discontinuity(), None);
    assert_eq!(
        full.advance_frontier(generation, InputTick::new(11)),
        Err(InputError::Full)
    );
    assert_eq!(full.discontinuity().unwrap().reason, InputError::Full);
}

#[test]
fn raw_repeated_key_releases_redeem_the_oldest_accepted_onset() {
    let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
    let release = Midi1Input::from_bytes([0x90, 60, 0]).unwrap();
    let (mut input, generation) = ready_with_cells(0, 8);
    let first = input
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    let second = input
        .offer_message(generation, InputTick::new(11), SampleTime::new(11), note)
        .unwrap();
    assert_eq!(input.release_reservations, 2);
    let first_release = input
        .offer_message(generation, InputTick::new(12), SampleTime::new(12), release)
        .unwrap();
    assert_eq!(input.release_reservations, 1);
    assert_eq!(
        input
            .slots
            .iter()
            .flatten()
            .find(|entry| entry.id == first_release)
            .unwrap()
            .matched_onset,
        Some(first)
    );
    assert_eq!(
        input
            .held_onsets
            .iter()
            .flatten()
            .map(|hold| hold.id)
            .collect::<Vec<_>>(),
        [second]
    );
    let second_release = input
        .offer_message(generation, InputTick::new(13), SampleTime::new(13), release)
        .unwrap();
    assert_eq!(input.release_reservations, 0);
    assert_eq!(
        input
            .slots
            .iter()
            .flatten()
            .find(|entry| entry.id == second_release)
            .unwrap()
            .matched_onset,
        Some(second)
    );
    assert!(input.held_onsets.iter().all(Option::is_none));
    assert_ne!(first, second);
}

#[test]
fn cancelled_raw_receipts_keep_matched_and_unmatched_release_identity() {
    let (mut input, generation) = ready_with_cells(0, 8);
    let onset = input
        .offer_message(
            generation,
            InputTick::new(10),
            SampleTime::new(10),
            Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
        )
        .unwrap();
    let unmatched = input
        .offer_message(
            generation,
            InputTick::new(11),
            SampleTime::new(11),
            Midi1Input::from_bytes([0x80, 61, 0]).unwrap(),
        )
        .unwrap();
    let matched = input
        .offer_message(
            generation,
            InputTick::new(12),
            SampleTime::new(12),
            Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
        )
        .unwrap();
    input.device_lost(generation).unwrap();
    let receipts: Vec<_> = std::iter::from_fn(|| input.collect()).collect();
    let unmatched_receipt = receipts
        .iter()
        .find(|receipt| receipt.id == unmatched)
        .unwrap();
    let matched_receipt = receipts
        .iter()
        .find(|receipt| receipt.id == matched)
        .unwrap();
    assert_eq!(unmatched_receipt.matched_onset, None);
    assert_eq!(matched_receipt.matched_onset, Some(onset));
    assert!(matches!(unmatched_receipt.outcome, InputOutcome::Cancelled));
    assert!(matches!(matched_receipt.outcome, InputOutcome::Cancelled));
}

#[test]
fn raw_preflight_matches_immediate_offer_without_spending_credit_or_faulting() {
    let (mut input, generation) = ready_with_cells(0, 6);
    let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
    let first = InputObservation::Message {
        tick: InputTick::new(10),
        arrival: SampleTime::new(10),
        input: note,
    };
    assert_eq!(input.preflight_observation(generation, first), Ok(()));
    assert_eq!(input.serial, 1);
    assert_eq!(input.release_reservations, 0);
    let _id = input
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();

    let second = InputObservation::Message {
        tick: InputTick::new(11),
        arrival: SampleTime::new(11),
        input: note,
    };
    assert_eq!(
        input.preflight_observation(generation, second),
        Err(InputError::ProtectedCapacity)
    );
    assert_eq!(input.serial, 2);
    assert_eq!(input.release_reservations, 1);
    assert_eq!(input.state(), ConnectionState::Running);
    assert_eq!(input.discontinuity(), None);
    assert_eq!(input.slots.iter().flatten().count(), 2);

    input.slots[0].as_mut().unwrap().outcome = Some(InputOutcome::Cancelled);
    let _receipt = input.collect().unwrap();
    assert_eq!(input.preflight_observation(generation, second), Ok(()));
    let _id = input
        .offer_message(generation, InputTick::new(11), SampleTime::new(11), note)
        .unwrap();
    assert_eq!(input.release_reservations, 2);

    let old = InputObservation::Frontier {
        tick: InputTick::new(9),
    };
    assert_eq!(
        input.preflight_observation(generation, old),
        Err(InputError::Order)
    );
    assert_eq!(input.state(), ConnectionState::Running);
    assert_eq!(input.discontinuity(), None);
    assert_eq!(
        input.advance_frontier(generation, InputTick::new(9)),
        Err(InputError::Order)
    );
    assert_eq!(
        input.preflight_observation(generation, second),
        Err(InputError::State)
    );
}
#[test]
fn failed_preparation_is_retained_and_retry_is_explicit() {
    let mut input = SimulatedNoteInput::new(
        EndpointId::new("missing".to_owned()).unwrap(),
        InputLimits {
            cells: InputCapacity::new(2).unwrap(),
            bytes: PreparedBytes::measured(65536),
        },
    )
    .unwrap();
    let failed = input.begin().unwrap();
    input.fail_preparation(failed).unwrap();
    assert_eq!(input.state(), ConnectionState::Unavailable);
    assert_eq!(
        input.discontinuity().unwrap().reason,
        InputError::Preparation
    );
    let retry = input.begin().unwrap();
    assert_ne!(failed, retry);
    assert_eq!(input.fail_preparation(failed), Err(InputError::Stale));
    assert_eq!(input.state(), ConnectionState::Preparing);
}

#[test]
fn refused_mapping_quality_intersects_valid_domain_without_admitting_clipped_input() {
    let epoch = issue_epoch().unwrap();
    let clock = SimulatedInputClock::new(
        epoch,
        SampleTime::ZERO,
        InputTick::new(100),
        InputRate::new(FrameCount::new(1), InputTickSpan::new(10)).unwrap(),
        InputTickSpan::new(10),
    );
    assert_eq!(clock.map(InputTick::new(95)), Err(InputError::ClockRange));
    assert_eq!(
        clock.quality_bounds(InputTick::new(95)),
        Some((SampleTime::ZERO, SampleTime::ZERO))
    );
    assert_eq!(clock.quality_bounds(InputTick::new(80)), None);
    let overflow = SimulatedInputClock::new(
        epoch,
        SampleTime::ZERO,
        InputTick::new(0),
        InputRate::new(FrameCount::new(u64::MAX), InputTickSpan::new(1)).unwrap(),
        InputTickSpan::new(1),
    );
    assert_eq!(overflow.map(InputTick::new(1)), Err(InputError::ClockRange));
    assert_eq!(
        overflow.quality_bounds(InputTick::new(1)),
        Some((SampleTime::ZERO, SampleTime::new(u64::MAX)))
    );
    assert_eq!(overflow.quality_bounds(InputTick::new(3)), None);
}
