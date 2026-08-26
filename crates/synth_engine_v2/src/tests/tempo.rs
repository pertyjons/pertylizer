//! The tempo map's checks.

use super::*;
use crate::quantities::SampleRate;

const RATE: f64 = 48_000.0;
const Q: u64 = TICKS_PER_QUARTER as u64;

fn rate() -> SampleRate {
    SampleRate::new(RATE as f32).expect("a valid rate")
}

fn bpm(value: f64) -> Bpm {
    Bpm::new(value).expect("a valid tempo")
}

#[test]
fn a_constant_tempo_maps_a_beat_to_its_exact_frame() {
    // The anchor case, and the one every other answer is measured against: at 120 BPM a
    // beat is half a second, so beat n lands on frame n * 24 000 exactly. An off-by-one in
    // the rounding or a beats-per-tick slip shows up here before anything subtler can hide
    // it.
    let map = TempoMap::new(bpm(120.0), &[], rate()).expect("a valid map");
    for beat in 0..8_u64 {
        assert_eq!(
            map.position_of(MusicalTick::new(beat * Q))
                .expect("inside plan time"),
            PlanPosition::new(beat * 24_000),
            "beat {beat} at 120 BPM"
        );
    }
}

#[test]
fn a_step_holds_the_old_tempo_up_to_the_change_and_the_new_one_after() {
    // A step is not a ramp of length zero: everything before the change runs at the old
    // tempo and everything after at the new one, with the change itself belonging to the
    // new segment.
    let map = TempoMap::new(
        bpm(120.0),
        &[TempoChange::step(MusicalTick::new(4 * Q), bpm(60.0))],
        rate(),
    )
    .expect("a valid map");

    assert_eq!(map.segment_count(), 2);
    assert_eq!(map.tempo_at(MusicalTick::new(4 * Q - 1)), bpm(120.0));
    assert_eq!(map.tempo_at(MusicalTick::new(4 * Q)), bpm(60.0));

    // Four beats at 120 BPM is two seconds; four more at 60 BPM is four.
    assert_eq!(
        map.position_of(MusicalTick::new(4 * Q))
            .expect("inside plan time"),
        PlanPosition::new(96_000)
    );
    assert_eq!(
        map.position_of(MusicalTick::new(8 * Q))
            .expect("inside plan time"),
        PlanPosition::new(96_000 + 192_000)
    );
}

#[test]
fn a_position_does_not_depend_on_what_was_asked_before_it() {
    // The exit gate's word is **stable**, and this is what it has to mean: the same tick
    // answers the same way whether it is asked directly, asked after every intermediate
    // tick, or asked on a freshly built identical map. A map that accumulated rounding
    // across segments, or cached a cursor, would fail this while passing every value check
    // above.
    let changes = [
        TempoChange::step(MusicalTick::new(2 * Q), bpm(90.0)),
        TempoChange::step(MusicalTick::new(6 * Q), bpm(144.0)),
        TempoChange::step(MusicalTick::new(9 * Q), bpm(144.0)),
        TempoChange::step(MusicalTick::new(13 * Q), bpm(72.0)),
    ];
    let map = TempoMap::new(bpm(120.0), &changes, rate()).expect("a valid map");
    let target = MusicalTick::new(15 * Q + 137);

    let direct = map.position_of(target).expect("inside plan time");

    let mut walked = None;
    for tick in 0..=(15 * Q + 137) {
        walked = Some(
            map.position_of(MusicalTick::new(tick))
                .expect("inside plan time"),
        );
    }
    assert_eq!(
        walked,
        Some(direct),
        "walking every tick must not change the answer"
    );

    let rebuilt = TempoMap::new(bpm(120.0), &changes, rate()).expect("a valid map");
    assert_eq!(
        rebuilt.position_of(target).expect("inside plan time"),
        direct,
        "a freshly built identical map must agree"
    );
}

#[test]
fn positions_never_go_backwards_across_a_dense_map() {
    // Monotonicity is the property the scheduler leans on: a later musical position is a
    // later plan position, or a plan's events could be ordered one way musically and
    // another way in frames. A sign error inside the ramp integral shows up here even where
    // its magnitude looks plausible.
    let changes = [
        TempoChange::step(MusicalTick::new(Q), bpm(200.0)),
        TempoChange::step(MusicalTick::new(3 * Q), bpm(30.0)),
        TempoChange::step(MusicalTick::new(4 * Q), bpm(30.0)),
        TempoChange::step(MusicalTick::new(11 * Q), bpm(180.0)),
    ];
    let map = TempoMap::new(bpm(120.0), &changes, rate()).expect("a valid map");

    let mut previous = PlanPosition::ZERO;
    for tick in 0..(14 * Q) {
        let position = map
            .position_of(MusicalTick::new(tick))
            .expect("inside plan time");
        assert!(
            position >= previous,
            "tick {tick} went backwards: {position:?} after {previous:?}"
        );
        previous = position;
    }
}

#[test]
fn changes_out_of_order_are_refused_rather_than_sorted() {
    // Two changes at one tick have no defined order, and sorting here would pick one
    // silently — the same reason the compiled scheduler refuses a descending event list
    // instead of sorting it.
    let out_of_order = [
        TempoChange::step(MusicalTick::new(4 * Q), bpm(90.0)),
        TempoChange::step(MusicalTick::new(2 * Q), bpm(100.0)),
    ];
    match TempoMap::new(bpm(120.0), &out_of_order, rate()) {
        Err(TempoError::ChangesOutOfOrder {
            index,
            tick,
            previous,
        }) => {
            assert_eq!(index, 1);
            assert_eq!(tick, MusicalTick::new(2 * Q));
            assert_eq!(previous, MusicalTick::new(4 * Q));
        }
        other => panic!("expected an ordering refusal, got {other:?}"),
    }

    let repeated = [
        TempoChange::step(MusicalTick::new(2 * Q), bpm(90.0)),
        TempoChange::step(MusicalTick::new(2 * Q), bpm(100.0)),
    ];
    assert!(
        TempoMap::new(bpm(120.0), &repeated, rate()).is_err(),
        "two changes at one tick have no order to pick"
    );
}

#[test]
fn a_tempo_outside_its_domain_is_refused_at_construction() {
    // Zero and negative make the ramp integral meaningless, and a clamping constructor
    // would let an invalid persisted tempo reach the map as a different, valid one.
    for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        match Bpm::new(value) {
            Err(TempoError::TempoOutsideDomain { bpm: named }) => {
                assert!(
                    named.is_nan() || named == value,
                    "the refusal names the value it refused"
                );
            }
            other => panic!("expected {value} to be refused, got {other:?}"),
        }
    }
}

#[test]
fn a_change_at_tick_zero_replaces_the_initial_tempo() {
    // Otherwise the map would carry a zero-length segment at the start, and a zero-length
    // ramp is the `0/0` case arriving by a different door.
    let map = TempoMap::new(
        bpm(120.0),
        &[TempoChange::step(MusicalTick::ZERO, bpm(60.0))],
        rate(),
    )
    .expect("a valid map");
    assert_eq!(map.segment_count(), 1, "no empty leading segment");
    assert_eq!(map.tempo_at(MusicalTick::ZERO), bpm(60.0));
    assert_eq!(
        map.position_of(MusicalTick::new(Q)).expect("inside"),
        PlanPosition::new(48_000),
        "one beat at 60 BPM is one second"
    );
}

#[test]
fn a_position_inside_a_later_segment_is_the_prefix_plus_its_own_offset() {
    // The value check an independent review found missing: every other assertion lands on a
    // segment boundary, where an offset computed from the wrong origin — `tick` rather than
    // `tick - start_tick` — still agrees. This one sits in the middle of the third segment,
    // where it does not.
    let map = TempoMap::new(
        bpm(120.0),
        &[
            TempoChange::step(MusicalTick::new(4 * Q), bpm(60.0)),
            TempoChange::step(MusicalTick::new(6 * Q), bpm(240.0)),
        ],
        rate(),
    )
    .expect("a valid map");

    // 4 beats at 120 = 2 s; 2 beats at 60 = 2 s; then half a beat at 240 = 0.125 s.
    let half_in = ((2.0 + 2.0 + 0.125) * RATE).round() as u64;
    assert_eq!(
        map.position_of(MusicalTick::new(6 * Q + Q / 2))
            .expect("inside plan time"),
        PlanPosition::new(half_in)
    );

    // The origin really is the segment's start. Counting beats from tick zero instead would
    // put this at 6.5 beats of 240 BPM — 78 000 frames rather than 198 000 — so the two
    // readings are nowhere near each other.
    assert_ne!(half_in, (6.5 * 60.0 / 240.0 * RATE).round() as u64);

    let quarter_in = ((2.0 + 2.0 + 0.0625) * RATE).round() as u64;
    assert_eq!(
        map.position_of(MusicalTick::new(6 * Q + Q / 4))
            .expect("inside plan time"),
        PlanPosition::new(quarter_in)
    );
}

#[test]
fn a_tick_past_exact_integer_range_is_refused_rather_than_answered() {
    // Past `2^53` an `f64` no longer represents consecutive integers, so two distinct ticks
    // would convert to one position. A timing law that silently merges two samples is worse
    // than one that says it cannot, and an independent review found the first draft doing
    // exactly that while guarding only near `2^64`.
    let map = TempoMap::new(bpm(120.0), &[], rate()).expect("a valid map");
    let limit = 1_u64 << 53;

    // The tick guard, on its own: one past the limit is refused before any arithmetic.
    match map.position_of(MusicalTick::new(limit + 1)) {
        Err(TempoError::PositionNotExactlyRepresentable { tick }) => {
            assert_eq!(tick, MusicalTick::new(limit + 1));
        }
        other => panic!("expected a refusal past exact range, got {other:?}"),
    }

    // The position guard, on its own: this tick is inside the limit, but at 48 kHz its
    // position is not — 2^53 ticks would land near 2.25e17 frames. Both guards are needed,
    // and neither subsumes the other.
    match map.position_of(MusicalTick::new(limit)) {
        Err(TempoError::PositionNotExactlyRepresentable { tick }) => {
            assert_eq!(tick, MusicalTick::new(limit));
        }
        other => panic!("expected the position to be refused, got {other:?}"),
    }

    // An ordinary musical span is nowhere near either bound: an hour at 120 BPM.
    assert!(map.position_of(MusicalTick::new(7_200 * Q)).is_ok());

    // The premise both guards protect: at the limit, consecutive integers stop being
    // distinct in `f64`, so answering would return one position for two samples.
    #[allow(
        clippy::cast_precision_loss,
        reason = "demonstrating the loss is the point"
    )]
    let collapsed = ((limit + 1) as f64) == (limit as f64);
    assert!(
        collapsed,
        "the boundary is where consecutive integers stop being distinct"
    );
}

#[test]
fn a_change_past_exact_integer_range_is_refused_at_construction() {
    // Caught where a caller can still be told, rather than on the query that happens to
    // reach it.
    let far = MusicalTick::new((1_u64 << 53) + 1);
    match TempoMap::new(bpm(120.0), &[TempoChange::step(far, bpm(90.0))], rate()) {
        Err(TempoError::PositionNotExactlyRepresentable { tick }) => assert_eq!(tick, far),
        other => panic!("expected a construction refusal, got {other:?}"),
    }
}

#[test]
fn the_conversion_uses_only_the_four_operations() {
    // ADR-0032 clause 15 requires the law to be "expressible in operations whose results are
    // identical on every supported target — the four IEEE-754 arithmetic operations,
    // comparison, and rounding", because a transcendental would make the frame a note lands
    // on depend on the platform's libm.
    //
    // The first draft of this module ported V1's `ln_1p` ramp and violated that. This is the
    // standing check: a source scan over the conversion, in the same spirit as the render
    // loop's purity scan, so a future ramp cannot arrive by quietly calling a library.
    let source = include_str!("../tempo.rs");
    let body = source
        .split("fn segment_seconds")
        .nth(1)
        .expect("the conversion function is named");
    let body = body.split("\n}").next().expect("the function has a body");
    for forbidden in [
        "ln_1p", "ln(", "log", "exp", "powf", "powi", "sqrt", "sin", "cos", "tan",
    ] {
        assert!(
            !body.contains(forbidden),
            "the conversion calls `{forbidden}`, which clause 15 forbids"
        );
    }
}

#[test]
fn a_half_frame_position_rounds_away_from_zero_rather_than_truncating() {
    // Clause 15 names round-half-away-from-zero, and every other value in this suite lands
    // on an exact frame — so a mutation to truncation passed all of them. This is the case
    // that separates the two.
    //
    // At 48 kHz one tick spans `3000 / bpm` frames, so 6 000 BPM puts a tick at exactly half
    // a frame. The tempo is absurd musically and that is fine: it is the smallest fixture
    // that makes the rounding rule observable, and the rule is what is under test.
    let map = TempoMap::new(bpm(6_000.0), &[], rate()).expect("a valid map");

    for (tick, expected) in [(1_u64, 1_u64), (3, 2), (5, 3), (2, 1), (4, 2)] {
        assert_eq!(
            map.position_of(MusicalTick::new(tick))
                .expect("inside plan time"),
            PlanPosition::new(expected),
            "tick {tick} spans {} frames",
            f64::from(tick as u32) * 0.5
        );
    }

    // Stated as the difference it makes: truncation would answer 0 where the law answers 1,
    // and a uniform early bias is what a listener hears as a drag against a click.
    assert_ne!(
        map.position_of(MusicalTick::new(1))
            .expect("inside plan time"),
        PlanPosition::ZERO
    );
}
