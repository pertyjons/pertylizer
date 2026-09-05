//! Transport activation's own checks: the sequence, the loop interval, and the exchange.
//!
//! The parts that need a renderer are in the integration suite; these are the value types,
//! where a property can be pinned without building a stream.

use super::*;

#[test]
fn a_sequence_changes_only_by_being_taken_and_refuses_permanently_at_its_ceiling() {
    // ADR-0050 clause 6: the value in force moves at adoption alone, and a reissued
    // sequence would make a superseded candidate adoptable again — the one thing the type
    // exists to prevent. So exhaustion refuses rather than wraps.
    let first = ActivationSequence::INITIAL;
    let second = first.next().expect("the space is not spent");
    assert!(second > first, "sequences are strictly increasing");
    assert_eq!(second.next().expect("still room").as_u64(), 2);

    let ceiling = ActivationSequence(u64::MAX);
    assert_eq!(
        ceiling.next(),
        None,
        "the ceiling refuses rather than wrapping to a value already used"
    );
}

#[test]
fn a_loop_interval_is_half_open_and_positive() {
    // Half-open for ADR-0046 clause 4's reason: the end frame belongs to the next pass, so
    // a closed interval would place two events one loop length apart in a window the
    // periodic extension never checks.
    let start = PlanPosition::new(100);
    let end = PlanPosition::new(164);
    let interval = LoopInterval::new(start, end).expect("a positive interval");
    assert_eq!(interval.length(), crate::time::FrameCount::new(64));

    assert_eq!(
        LoopInterval::new(start, start),
        None,
        "an empty loop has no periodic extension at all"
    );
    assert_eq!(
        LoopInterval::new(end, start),
        None,
        "and neither has an inverted one"
    );
}

#[test]
fn the_exchange_tells_a_pending_candidate_from_an_uncollected_retirement() {
    // `HOST-INV-022`: an offer is refused either way, but only one of the two means the
    // off-thread half has not collected — and a diagnostic that reported one as the other
    // would send someone to fix the wrong half.
    assert_eq!(Exchange::Empty.occupied(), None);
    assert_eq!(
        Exchange::Pending(Box::new(activation())).occupied(),
        Some(ActivationRefused::AlreadyPending)
    );
    let mut adopted = activation();
    adopted.effective = Some(SampleTime::new(64));
    assert_eq!(
        Exchange::Retired(Box::new(adopted)).occupied(),
        Some(ActivationRefused::RetiredUncollected)
    );
}

/// A candidate with nothing in it. Only the exchange's own bookkeeping is under test here.
fn activation() -> TransportActivation {
    TransportActivation {
        epoch: StreamEpoch::from_raw(1),
        sequence: ActivationSequence(1),
        supersedes: ActivationSequence::INITIAL,
        requested: SampleTime::ZERO,
        position: PlanPosition::ZERO,
        events: Vec::new(),
        outstanding: Vec::new(),
        minter: crate::identity::IdentityTable::from_admitted_ranges(&[])
            .expect("an empty partition"),
        omitted_releases: 0,
        released_after_steal: 0,
        catch_up: Vec::new(),
        loop_interval: None,
        producers: Vec::new(),
        ended: Vec::new(),
        late: false,
        effective: None,
        retired: None,
    }
}
