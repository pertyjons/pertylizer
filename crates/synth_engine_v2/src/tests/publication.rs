//! The publication arbiter's checks.

use super::*;
use crate::plan::PlanId;
use crate::profile::HostProfile;
use crate::publish::WindowRow;
use crate::quantities::{ChannelLayout, EventCount, ParameterValue, SampleRate};
use crate::render::{EventEnvelope, EventPayload, TimedEvent};
use crate::time::{FrameCount, QUANTUM_FRAMES, SampleTime, StreamEpoch, TimeSource};

fn profile(block: u64) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("a valid rate"),
        FrameCount::new(block),
        ChannelLayout::Stereo,
    )
    .expect("the default harness profile is valid")
}

/// An event whose payload does not matter; where it lands is what is under test.
fn at(time: u64) -> TimedEvent {
    TimedEvent::new(
        EventEnvelope::new(
            StreamEpoch::from_raw(1),
            SampleTime::new(time),
            TimeSource::Compiled,
        ),
        EventPayload::SetParameter {
            slot: crate::plan::ParameterSlot::new(PlanId::FILL, 0),
            value: ParameterValue::ZERO,
        },
    )
}

/// An event landing in quantum `index`, offset by one frame so it is not on the boundary.
fn in_quantum(index: u64) -> TimedEvent {
    at(index * u64::from(QUANTUM_FRAMES) + 1)
}

#[test]
fn the_store_covers_the_cap_over_every_quantum_the_callback_renders() {
    // ADR-0046 clause 1's sealed-batch relation, as an allocation rather than as prose.
    // A store smaller than this could be the thing that fails while every producer stayed
    // inside its own share, which is exactly the failure the partition exists to remove.
    for block in [64_u64, 256, 1_024, 4_096] {
        let host = profile(block);
        let arbiter = PublicationArbiter::prepare(&host).expect("the store is preparable");
        let quanta = host
            .capabilities()
            .max_quanta_per_callback()
            .expect("fits")
            .as_usize()
            .expect("fits");
        let cap = host
            .limits()
            .events()
            .max_events_per_quantum()
            .as_usize()
            .expect("fits");
        assert_eq!(
            arbiter.capacity(),
            cap * quanta,
            "a {block}-frame callback covers {quanta} quanta at {cap} events each"
        );
    }
}

#[test]
fn the_destination_comes_from_the_event_and_not_from_the_caller() {
    // The property the whole ledger rests on. `charge` takes no quantum: if a producer
    // could name its own row, it could spread one real destination across several rows and
    // overrun its share without ever tripping it, leaving the partition advisory. The
    // independent review found exactly that hole in the first draft, where an earlier
    // version of this suite charged one `SampleTime` to two different rows and passed.
    let host = profile(256);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");
    let share = ProducerClass::Live.share_of(&host);

    let mut publication = arbiter
        .open(SampleTime::ZERO, 4)
        .expect("a four-quantum window");
    // Every event names quantum 0, however many times it is offered.
    for _ in 0..share.get() {
        publication
            .charge(ProducerClass::Live, in_quantum(0))
            .expect("inside the share");
    }
    assert_eq!(
        publication.spent(WindowRow::new(0), ProducerClass::Live),
        share
    );
    for other in 1..4 {
        assert_eq!(
            publication.spent(WindowRow::new(other), ProducerClass::Live),
            EventCount::NONE,
            "no charge reached quantum {other}"
        );
    }

    publication
        .charge(ProducerClass::Live, in_quantum(0))
        .expect_err("one past the share, and no other row absorbs it");
}

#[test]
fn a_share_overrun_is_a_fault_even_while_the_quantum_total_has_room() {
    // Clause 7 in one case: "the same terminal response applies when an external producer
    // exceeds its fixed share even if unusable slack means the quantum total remains below
    // `max_events_per_quantum`". A ledger that checked the total would pass this and turn
    // a declared share into a soft limit.
    let host = profile(256);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");
    let share = ProducerClass::Live.share_of(&host);

    let mut publication = arbiter
        .open(SampleTime::ZERO, 1)
        .expect("a one-quantum window");
    for _ in 0..share.get() {
        publication
            .charge(ProducerClass::Live, in_quantum(0))
            .expect("inside the share");
    }

    let fault = publication
        .charge(ProducerClass::Live, in_quantum(0))
        .expect_err("one past the share is a fault");
    match fault {
        PublicationFault::ShareOverrun {
            class,
            quantum,
            requested,
            share: named,
        } => {
            assert_eq!(class, ProducerClass::Live);
            assert_eq!(quantum, WindowRow::FIRST);
            assert_eq!(requested.get(), share.get() + 1);
            assert_eq!(named, share, "the fault names the share that bound it");
        }
        other => panic!("expected a share overrun, got {other:?}"),
    }

    let batch = publication.seal();
    assert_eq!(
        batch.len() as u32,
        share.get(),
        "the refused charge stored nothing"
    );
    assert!(
        batch.external_total(WindowRow::new(0)) < host.limits().events().max_events_per_quantum(),
        "the premise: the quantum still had room, so the total is not what refused"
    );
}

#[test]
fn a_renderer_internal_emission_may_not_enter_the_external_batch() {
    // ADR-0046 clause 2: internal emissions live in a separate preallocated arena on the
    // renderer side of the seal and "never reopen or mutate the external batch". The class
    // exists because its share is part of the same partition, so the refusal has to be
    // explicit rather than an omission a later caller could read as an oversight.
    let host = profile(256);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");
    let mut publication = arbiter.open(SampleTime::ZERO, 1).expect("window");
    match publication.charge(ProducerClass::Internal, in_quantum(0)) {
        Err(PublicationFault::InternalIntoExternalBatch) => {}
        other => panic!("expected the internal class to be refused, got {other:?}"),
    }
    assert_eq!(publication.seal().len(), 0, "and nothing was stored");
}

#[test]
fn every_publishable_class_is_charged_against_its_own_share_and_no_other() {
    // The partition, exercised class by class. Filling one class to its share must leave
    // every other class's entitlement untouched — that is what "disjoint" means, and a
    // ledger keyed on the quantum alone would fail it.
    let host = profile(256);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");
    let mut publication = arbiter.open(SampleTime::ZERO, 1).expect("window");

    let publishable: Vec<_> = ProducerClass::ALL
        .into_iter()
        .filter(|class| *class != ProducerClass::Internal)
        .collect();

    for class in &publishable {
        let share = class.share_of(&host);
        for _ in 0..share.get() {
            publication
                .charge(*class, in_quantum(0))
                .unwrap_or_else(|error| panic!("{} within its share: {error:?}", class.name()));
        }
    }

    for class in &publishable {
        assert_eq!(
            publication.spent(WindowRow::new(0), *class),
            class.share_of(&host),
            "{} kept its own count while every other class filled",
            class.name()
        );
    }
}

#[test]
fn one_quantums_charges_do_not_reach_another() {
    // Per-quantum accounting rather than per-call: the shares bound a *destination
    // quantum*, so filling quantum 0 must leave quantum 1 empty.
    let host = profile(256);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");
    let mut publication = arbiter
        .open(SampleTime::ZERO, 2)
        .expect("a two-quantum window");
    let share = ProducerClass::Live.share_of(&host);

    for _ in 0..share.get() {
        publication
            .charge(ProducerClass::Live, in_quantum(0))
            .expect("inside the share");
    }
    assert_eq!(
        publication.spent(WindowRow::new(1), ProducerClass::Live),
        EventCount::NONE
    );
    publication
        .charge(ProducerClass::Live, in_quantum(1))
        .expect("the second quantum has its own entitlement");
}

#[test]
fn a_window_that_does_not_start_at_zero_places_events_by_offset() {
    // The stream does not restart every callback, so the window's first quantum is
    // whatever the clock reached. An arbiter that assumed zero would misplace every event
    // after the first call — and would do it silently, since the rows would still exist.
    let host = profile(256);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");
    let mut publication = arbiter
        .open(SampleTime::new(100 * u64::from(QUANTUM_FRAMES)), 2)
        .expect("window");

    publication
        .charge(ProducerClass::Live, in_quantum(101))
        .expect("the window's second quantum");
    assert_eq!(
        publication.spent(WindowRow::new(1), ProducerClass::Live),
        EventCount::measured(1),
        "quantum 101 is offset 1 of a window starting at 100"
    );
    assert_eq!(
        publication.spent(WindowRow::new(0), ProducerClass::Live),
        EventCount::NONE
    );

    // **The two directions are not symmetric, and that is ADR-0032 clause 21.** Only
    // forward has a budget. A destination past the window is refused and waits for the
    // call that can reach it. A destination *behind* the window is not outside it: ADR-0043's
    // preserving late clamp puts its render position at the first not-yet-rendered
    // boundary, which is this window's own first quantum, so row zero is where it belongs.
    // Refusing it instead would drop an event ADR-0001 clause 16 forbids dropping and
    // would strand a live producer's late entry in its queue forever, because the window
    // only ever moves further away from it.
    publication
        .charge(ProducerClass::Live, in_quantum(99))
        .expect("a late destination takes the window's first row under the late clamp");
    assert_eq!(
        publication.spent(WindowRow::new(0), ProducerClass::Live),
        EventCount::measured(1),
        "quantum 99 is late for a window starting at 100, so it spends row 0's share"
    );
    publication
        .charge(ProducerClass::Live, in_quantum(102))
        .expect_err("a destination past the window is refused");
}

#[test]
fn both_high_water_marks_survive_a_quieter_pass() {
    // Clause 1's high-water requirement is about the *stream*, and a mark reset by every
    // call could not answer what Phase 3 has to measure. The total is kept beside the six
    // class marks because it is not derivable from them: their peaks can fall in different
    // quanta, so the sum overstates and the maximum understates.
    let host = profile(256);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");

    let mut publication = arbiter.open(SampleTime::ZERO, 2).expect("window");
    for _ in 0..4 {
        publication
            .charge(ProducerClass::Session, in_quantum(0))
            .expect("inside the share");
    }
    for _ in 0..3 {
        publication
            .charge(ProducerClass::Live, in_quantum(1))
            .expect("inside the share");
    }
    drop(publication.seal());
    assert_eq!(
        arbiter.high_water(ProducerClass::Session),
        EventCount::measured(4)
    );
    assert_eq!(
        arbiter.high_water(ProducerClass::Live),
        EventCount::measured(3)
    );
    assert_eq!(
        arbiter.high_water_external_total(),
        EventCount::measured(4),
        "the peak quantum held four, not the seven the class marks sum to"
    );

    let mut publication = arbiter.open(SampleTime::ZERO, 1).expect("window");
    assert_eq!(
        publication.spent(WindowRow::new(0), ProducerClass::Session),
        EventCount::NONE,
        "the new pass starts empty"
    );
    publication
        .charge(ProducerClass::Session, in_quantum(0))
        .expect("inside the share");
    drop(publication.seal());
    assert_eq!(
        arbiter.high_water(ProducerClass::Session),
        EventCount::measured(4),
        "a quieter pass does not lower the class mark"
    );
    assert_eq!(
        arbiter.high_water_external_total(),
        EventCount::measured(4),
        "nor the total"
    );
}

#[test]
fn a_window_larger_than_the_prepared_store_is_refused() {
    // The store is prepared for the largest callback the profile admits. A window past it
    // is a caller bypassing the publication contract, which clause 7 names as one of the
    // three ways to reach a fault.
    let host = profile(256);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");
    let prepared = arbiter.max_quanta_per_callback();
    match arbiter.open(SampleTime::ZERO, prepared.as_usize().expect("fits") + 1) {
        Err(PublicationFault::WindowExceedsPreparedStore {
            quanta,
            prepared: named,
        }) => {
            assert_eq!(quanta, prepared.as_usize().expect("fits") + 1);
            assert_eq!(named, prepared);
        }
        other => panic!("expected a window refusal, got {other:?}"),
    }
}

#[test]
fn every_harness_profile_over_these_block_sizes_prepares_a_store() {
    // The premise behind `prepare`'s only refusal, and the claim is exactly what the method
    // establishes rather than a universal it cannot reach. These are harness profiles at
    // the default limits over eight block sizes; they do not vary the event cap or the
    // shares, so this says nothing about profiles that do.
    //
    // What makes the branch hard to reach at all is that `HostProfile::new` already refuses
    // a profile whose sealed-batch extent no `EventCount` can name — the same shape
    // `SOUND-INV-007`'s layout refusal takes, where the premise is asserted so the row
    // fails when the premise does.
    for block in [1_u64, 63, 64, 65, 256, 1_024, 4_096, 48_000] {
        assert!(
            PublicationArbiter::prepare(&profile(block)).is_ok(),
            "a {block}-frame harness profile admitted by construction must prepare"
        );
    }
}

#[test]
fn a_smaller_pass_does_not_expose_a_larger_ones_occupancy() {
    // The stale-row defect an independent reread found. `open` used to clear only the rows
    // its own window used, so after a wide pass a narrow one left real occupancy readable
    // beyond its window — indistinguishable to an observer from work the narrow pass did.
    //
    // **What this establishes, and what it does not.** It fixes the observable property: a
    // narrow pass reads nothing outside its window. It cannot separate the two safeguards
    // that produce it, and a later reread was right to say so — restoring the partial clear
    // alone still passes, and dropping the accessor bound alone still passes, because
    // either one suffices. The accessor bound is the load-bearing half; clearing the whole
    // store is defence the public API cannot distinguish, kept so the two agree if a later
    // revision narrows either.
    let host = profile(1_024);
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");

    let mut wide = arbiter
        .open(SampleTime::ZERO, 4)
        .expect("a four-quantum window");
    for quantum in 0..4 {
        for _ in 0..3 {
            wide.charge(ProducerClass::Live, in_quantum(quantum))
                .expect("inside the share");
        }
    }
    assert_eq!(
        wide.spent(WindowRow::new(3), ProducerClass::Live),
        EventCount::measured(3)
    );
    drop(wide.seal());

    let narrow = arbiter
        .open(SampleTime::ZERO, 1)
        .expect("a one-quantum window")
        .seal();
    assert_eq!(
        narrow.spent(WindowRow::new(3), ProducerClass::Live),
        EventCount::NONE,
        "a row outside the narrow window must read nothing, not the wide pass's three"
    );
    assert_eq!(narrow.external_total(WindowRow::new(3)), EventCount::NONE);
    assert_eq!(
        narrow.spent(WindowRow::new(0), ProducerClass::Live),
        EventCount::NONE
    );
}
