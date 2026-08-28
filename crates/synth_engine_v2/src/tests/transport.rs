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
fn wrap_times_come_from_the_ideal_timeline_so_a_non_quantum_loop_does_not_drift() {
    // ADR-0050 clause 1, and the property a wrap derived from the previous *effective* point
    // would lose. The loop below is 100 frames, which is not a whole number of 64-frame
    // quanta, so every wrap snaps forward by a different amount.
    //
    // The falsifier is drift: under the ideal derivation the `n`-th wrap is exactly `n`
    // lengths after the first, so the error at each one is independent and bounded by a
    // quantum. Deriving each from the last snapped value would make the gap between
    // consecutive wraps at least one length and usually more, and the deficit would grow
    // without bound.
    const LENGTH: u64 = 100;
    let interval =
        LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(LENGTH)).expect("positive");
    let first = SampleTime::new(1_000);

    let mut previous_effective = snap(first);
    for pass in 1..=64_u64 {
        let requested = interval.wrap_at(first, pass).expect("representable");
        assert_eq!(
            requested.as_u64(),
            first.as_u64() + LENGTH * pass,
            "the requested time is the ideal one, whatever the previous wrap snapped to"
        );

        let effective = snap(requested);
        let error = effective.as_u64() - requested.as_u64();
        assert!(
            error < u64::from(crate::time::QUANTUM_FRAMES),
            "pass {pass} snapped by {error} frames, which is a whole quantum or more"
        );

        // The gap between consecutive wraps stays within one quantum of the ideal length.
        // Under a drifting derivation it could not: each pass would add the previous
        // pass's rounding to its own.
        let gap = effective.as_u64() - previous_effective.as_u64();
        assert!(
            gap.abs_diff(LENGTH) < u64::from(crate::time::QUANTUM_FRAMES),
            "pass {pass} landed {gap} frames after the previous wrap, against a loop of {LENGTH}"
        );
        previous_effective = effective;
    }
}

/// The first quantum boundary at or after `time`, which is clause 1's effective point once
/// the clock has been taken into account.
fn snap(time: SampleTime) -> SampleTime {
    let quantum = u64::from(crate::time::QUANTUM_FRAMES);
    let remainder = time.as_u64() % quantum;
    if remainder == 0 {
        time
    } else {
        SampleTime::new(time.as_u64() + (quantum - remainder))
    }
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
        catch_up: Vec::new(),
        loop_interval: None,
        producers: Vec::new(),
        ended: Vec::new(),
        late: false,
        effective: None,
        retired: None,
    }
}
