use super::*;
use crate::{
    recording::notes::Midi1Input,
    time::{FrameCount, issue_epoch},
};

fn ready(uncertainty: u64) -> (SimulatedNoteInput, ConnectionGeneration) {
    let mut input = SimulatedNoteInput::new(
        EndpointId::new("oracle".to_owned()).unwrap(),
        InputLimits {
            cells: InputCapacity::new(4).unwrap(),
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
fn key_release_needs_a_free_message_cell_before_the_frontier_cell() {
    let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
    let release = Midi1Input::from_bytes([0x80, 60, 0]).unwrap();
    let (mut admitted, generation) = ready(0);
    let _id = admitted
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    let _id = admitted
        .offer_message(generation, InputTick::new(11), SampleTime::new(11), release)
        .unwrap();
    let _id = admitted
        .advance_frontier(generation, InputTick::new(12))
        .unwrap();

    let (mut refused, generation) = ready(0);
    let _id = refused
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    let _id = refused
        .offer_message(generation, InputTick::new(11), SampleTime::new(11), note)
        .unwrap();
    assert_eq!(
        refused.offer_message(generation, InputTick::new(12), SampleTime::new(12), release),
        Err(InputError::Full)
    );
    assert_eq!(refused.state(), ConnectionState::Quiescing);
    assert_eq!(refused.discontinuity().unwrap().reason, InputError::Full);

    let (mut frontier_only, generation) = ready(0);
    let _id = frontier_only
        .offer_message(generation, InputTick::new(10), SampleTime::new(10), note)
        .unwrap();
    let _id = frontier_only
        .offer_message(generation, InputTick::new(11), SampleTime::new(11), note)
        .unwrap();
    let _id = frontier_only
        .advance_frontier(generation, InputTick::new(12))
        .unwrap();
    assert_eq!(frontier_only.state(), ConnectionState::Running);
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
