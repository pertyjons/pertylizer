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
    // **Three regions rather than one**, because ADR-0049 spread the law across three
    // functions: the segment integral, the beat conversion both of the others call, and the
    // reported tempo, which clause 7 makes the reciprocal of the interpolated period.
    // Scanning only the first would have let a ramp arrive through either of the others.
    let source = include_str!("../tempo.rs");
    // **Comments are stripped before the scan**, because the check is about code and a
    // substring list this broad hits ordinary prose: the word "rising" contains `sin`, and
    // it failed a correct implementation once for exactly that reason. Stripping is the
    // repair rather than rewording the comment, because the next sentence someone writes
    // would hit it again.
    //
    // **A line holding a quote is scanned raw instead**, which is what keeps stripping
    // sound without lexing Rust. A naive split on `//` truncates a line at a `//` inside a
    // string, so `("https://x", beats.sin()).1` would have its call removed from the
    // scanned text — an independent review found exactly that hole. Leaving such a line
    // whole cannot hide anything; at worst a comment on the same line false-positives,
    // which is the safe direction and is visible immediately.
    let region = |marker: &str, closing: &str| -> String {
        let after = source
            .split(marker)
            .nth(1)
            .unwrap_or_else(|| panic!("`{marker}` names a function in this module"));
        let body = after
            .split(closing)
            .next()
            .expect("the function has a body");
        // Attributes are dropped too: `#[allow(...)]` is not a call, and leaving it in makes
        // the call allowlist below reject `allow`, which teaches a reader nothing.
        let mut kept: Vec<&str> = Vec::new();
        let mut in_attribute = false;
        for line in body.lines() {
            let trimmed = line.trim_start();
            if in_attribute {
                in_attribute = !trimmed.ends_with(")]") && !trimmed.ends_with(']');
                continue;
            }
            if trimmed.starts_with("#[") {
                in_attribute = !trimmed.ends_with(']');
                continue;
            }
            kept.push(if line.contains('"') {
                line
            } else {
                line.split("//").next().unwrap_or(line)
            });
        }
        kept.join("\n")
    };

    // **The scan proves a property of these three bodies and does not follow calls**, so the
    // regions must call nothing that is not itself scanned. An independent review pointed
    // out that `segment_seconds` could otherwise call a harmlessly named helper containing
    // `x.ln()`. The allowlist below is what closes that: every call these bodies make is
    // either to one of the three, or to a method whose name is in the arithmetic set.
    const CALLABLE: [&str; 16] = [
        // The functions that are themselves scanned below, so a call to one is covered.
        "beats_from",
        "segment_for",
        // Arithmetic and comparison on the standard types, none of it transcendental.
        "saturating_sub",
        "min",
        "max",
        "clamp",
        "is_finite",
        // Slice, iterator, `Option` and newtype accessors, which move a value without
        // computing one.
        "unwrap_or",
        "copied",
        "first",
        "rev",
        "iter",
        "as_u64",
        // Pattern, constructor and conversion names the crude `name(` match also catches.
        "Some",
        "Bpm",
        "from",
    ];

    // **An allowlisted name must not also be a function this module defines**, or the
    // allowlist becomes the hole it was added to close: `fn min(x: f64) -> f64 { x.ln() }`
    // called as `min(beats)` would pass, because the scan validates the call's name and not
    // the body behind it. An independent review found exactly that. The two entries that are
    // module functions are the ones the scan itself covers.
    const SCANNED: [&str; 5] = [
        "beats_from",
        "segment_for",
        "segment_seconds",
        "tempo_at",
        // A one-line newtype accessor, scanned rather than trusted for the same reason the
        // rule exists at all: it is a function this module defines by an allowlisted name.
        "as_u64",
    ];
    for name in CALLABLE {
        if SCANNED.contains(&name) {
            continue;
        }
        assert!(
            !source.contains(&format!("fn {name}(")),
            "`{name}` is allowlisted but this module defines a function by that name; either \
             scan it or rename the helper"
        );
    }

    // A free function closes at column zero; a method closes at the impl block's indent.
    for (marker, closing) in [
        ("fn segment_seconds", "\n}"),
        ("fn beats_from", "\n}"),
        ("fn tempo_at", "\n    }"),
        // Reached from `tempo_at`, and scanned rather than allowlisted: the allowlist is for
        // names that are arithmetic, and this one is a function whose body could grow any
        // call at all. The check forced the question, which is what it is for.
        ("fn segment_for", "\n    }"),
        ("fn as_u64", "\n    }"),
    ] {
        let body = region(marker, closing);

        // Every call the body makes must be named in the allowlist. The pattern is crude on
        // purpose — an identifier followed by `(` — because a crude over-match here fails
        // loudly and sends a reader to this list, while a clever under-match would let the
        // very call the scan exists to find slip past unexamined.
        let mut rest = body.as_str();
        while let Some(open) = rest.find('(') {
            let before = &rest[..open];
            let name: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !name.is_empty() {
                assert!(
                    CALLABLE.contains(&name.as_str()),
                    "`{marker}` calls `{name}`, which the scan does not follow; add it to \
                     the scanned regions or to CALLABLE once it is known to be arithmetic"
                );
            }
            rest = &rest[open + 1..];
        }

        for forbidden in [
            "ln_1p", "ln(", "log", "exp", "powf", "powi", "sqrt", "sin", "cos", "tan",
        ] {
            assert!(
                !body.contains(forbidden),
                "`{marker}` calls `{forbidden}`, which clause 15 forbids"
            );
        }
    }
}

#[test]
fn the_four_operation_scan_does_not_strip_a_line_holding_a_string() {
    // The rule the scan above relies on, checked directly rather than through the module's
    // own source — which contains no line where a `//` hides inside a string, so a
    // regression there would pass silently. An independent review found that hole in a
    // version that stripped every line.
    let sample = "let url = (\"https://example\", beats.sin()).1; // ordinary comment";
    let scanned: String = sample
        .lines()
        .map(|line| {
            if line.contains('"') {
                line
            } else {
                line.split("//").next().unwrap_or(line)
            }
        })
        .collect();
    assert!(
        scanned.contains("sin()"),
        "a `//` inside a string must not truncate the line: {scanned}"
    );

    // And the case the rule exists for: a comment on a quote-free line is still removed, so
    // ordinary prose cannot fail the scan.
    let prose = "    return linear; // both terms are non-negative and rising";
    let stripped: String = prose
        .lines()
        .map(|line| {
            if line.contains('"') {
                line
            } else {
                line.split("//").next().unwrap_or(line)
            }
        })
        .collect();
    assert!(!stripped.contains("rising"), "prose survived: {stripped}");
}

#[test]
fn a_ramp_with_equal_endpoints_is_bit_identical_to_a_step() {
    // ADR-0049 clause 3, and the property that removes V1's `1e-5` near-flat branch rather
    // than reproducing it: the quadratic term is a signed zero when the two periods are
    // equal, so the sum is the step law's own value bit for bit.
    //
    // **The falsifier is a second copy of the linear term, not a missing feature.** The two
    // laws share one value; a ramp that recomputed it — even into a form that is equal in
    // exact arithmetic — could disagree with the step by a rounding, and that mutation is
    // what this check fails on. Reordering *both* laws together does not break the identity
    // and this check correctly does not claim it does.
    for tempo in [60.0, 90.0, 120.0, 137.37, 6_000.0] {
        let ramped = TempoMap::new(
            bpm(tempo),
            &[
                TempoChange::ramp(MusicalTick::ZERO, bpm(tempo)),
                TempoChange::step(MusicalTick::new(16 * Q), bpm(tempo)),
            ],
            rate(),
        )
        .expect("a valid map");
        let stepped = TempoMap::new(
            bpm(tempo),
            &[
                TempoChange::step(MusicalTick::ZERO, bpm(tempo)),
                TempoChange::step(MusicalTick::new(16 * Q), bpm(tempo)),
            ],
            rate(),
        )
        .expect("a valid map");

        for tick in (0..=16 * Q).step_by(7) {
            let tick = MusicalTick::new(tick);
            assert_eq!(
                ramped.position_of(tick).expect("inside plan time"),
                stepped.position_of(tick).expect("inside plan time"),
                "an equal-endpoint ramp at {tempo} BPM must equal the step at {tick}"
            );
        }
    }
}

#[test]
fn a_ramp_lasts_its_beats_times_the_mean_of_its_two_periods() {
    // ADR-0049 clauses 1 and 2, on the fixture the corpus actually contains:
    // `tempo-map-arrangement` ramps 90 to 180 BPM over two beats. The periods are 2/3 s and
    // 1/3 s, so the segment lasts `2 * (2/3 + 1/3) / 2` = 1.000 s exactly — 48 000 frames.
    //
    // **This is also the number that differs from V1**, whose logarithmic law answers
    // 0.924196 s, or 44 361 frames. The gap is the delivered-behaviour change ADR-0049
    // accepted and Phase 4's A/B owns as an intentional difference, and asserting the V2
    // value here is what stops it drifting back by accident.
    let map = TempoMap::new(
        bpm(90.0),
        &[
            TempoChange::ramp(MusicalTick::ZERO, bpm(90.0)),
            TempoChange::step(MusicalTick::new(2 * Q), bpm(180.0)),
            TempoChange::step(MusicalTick::new(4 * Q), bpm(120.0)),
        ],
        rate(),
    )
    .expect("a valid map");

    assert_eq!(
        map.position_of(MusicalTick::new(2 * Q))
            .expect("inside plan time"),
        PlanPosition::new(48_000),
        "the ramp's own end"
    );
    assert_ne!(
        map.position_of(MusicalTick::new(2 * Q))
            .expect("inside plan time"),
        PlanPosition::new(44_361),
        "V1's logarithmic law is the value this record deliberately does not produce"
    );

    // The prefix is the same expression evaluated at the boundary, so the segment after the
    // ramp starts where the ramp ended: two more beats at 180 BPM is 2 * 16 000 frames.
    assert_eq!(
        map.position_of(MusicalTick::new(4 * Q))
            .expect("inside plan time"),
        PlanPosition::new(48_000 + 32_000)
    );
}

#[test]
fn a_ramp_does_not_move_a_position_backwards_over_a_musical_segment() {
    // ADR-0049 clause 6, and the name says "over a musical segment" because the clause makes
    // no unconditional claim: the property is checked rather than proved, and inversions are
    // constructible far outside this domain. Equal positions are legitimate, because clause 5
    // rounds to a frame and a fast segment can put two ticks on one.
    //
    // Both directions are covered: a steep acceleration and a steep deceleration, since a
    // sign error in the quadratic term breaks only one of them.
    for (from, to) in [(30.0, 300.0), (300.0, 30.0), (120.0, 121.0)] {
        let map = TempoMap::new(
            bpm(from),
            &[
                TempoChange::ramp(MusicalTick::ZERO, bpm(from)),
                TempoChange::step(MusicalTick::new(8 * Q), bpm(to)),
            ],
            rate(),
        )
        .expect("a valid map");

        let mut previous = PlanPosition::ZERO;
        for tick in 0..=8 * Q {
            let position = map
                .position_of(MusicalTick::new(tick))
                .expect("inside plan time");
            assert!(
                position.as_u64() >= previous.as_u64(),
                "{from} to {to} BPM went backwards at tick {tick}"
            );
            previous = position;
        }
    }
}

#[test]
fn a_position_does_not_move_backwards_in_sampled_windows_of_a_steep_ramp() {
    // ADR-0049 clause 6 **withdraws** the unconditional monotonicity guarantee rather than
    // claiming one this evaluation cannot keep. The accepted form subtracts a positive
    // quadratic from a positive linear term whenever the tempo rises, so it is not a
    // composition of monotone operations — a form that *is* exists, and clause 6 records why
    // it is rejected on measurement rather than overlooked.
    //
    // What this checks is **sampled**, and the name and the record both say so: adjacent
    // ticks in four windows of a steep ramp, not the whole segment. The falsifier is the
    // quadratic's sign, which this and the neighbouring check share.
    // Musical length, not wall-clock: the ramps below run between two and three hours
    // depending on their tempi, and calling it an hour was wrong in the first version.
    const SEGMENT_TICKS: u64 = 960 * 2 * 60 * 60;
    for (from, to) in [
        (20.0, 6_000.0),
        (6_000.0, 20.0),
        (30.0, 300.0),
        (300.0, 30.0),
    ] {
        let map = TempoMap::new(
            bpm(from),
            &[
                TempoChange::ramp(MusicalTick::ZERO, bpm(from)),
                TempoChange::step(MusicalTick::new(SEGMENT_TICKS), bpm(to)),
            ],
            rate(),
        )
        .expect("a valid map");

        // **Adjacent ticks, in windows, rather than a stride over the whole segment.** The
        // failure mode is a one-frame inversion between *neighbouring* ticks that recovers
        // immediately, so a stride of 97 steps straight over it — an independent review
        // found the first version of this check doing exactly that.
        for window in [
            0,
            SEGMENT_TICKS / 4,
            SEGMENT_TICKS / 2,
            3 * SEGMENT_TICKS / 4,
        ] {
            let mut previous = map
                .position_of(MusicalTick::new(window))
                .expect("inside plan time");
            for tick in window..=(window + 20_000).min(SEGMENT_TICKS) {
                let position = map
                    .position_of(MusicalTick::new(tick))
                    .expect("inside plan time");
                assert!(
                    position.as_u64() >= previous.as_u64(),
                    "{from} to {to} BPM went backwards at tick {tick}"
                );
                previous = position;
            }
        }
    }
}

#[test]
fn a_tempo_whose_period_overflows_is_refused_at_construction() {
    // An independent review found a ramp toward a tempo below `60 / f64::MAX` producing an
    // infinite period, and from it a `NaN` position and a `Bpm(NaN)` built inside
    // `tempo_at` without passing through `Bpm::new`. The repair closes it at the newtype,
    // which is where this repository puts an invariant.
    assert!(Bpm::new(f64::MIN_POSITIVE).is_err());
    assert!(
        Bpm::new(3.3e-307).is_err(),
        "60/bpm overflows just below here"
    );
    // The boundary is not moved for any tempo a caller could mean: the smallest accepted
    // value is still one beat per far longer than the age of the universe.
    assert!(Bpm::new(3.4e-307).is_ok());
    assert!(Bpm::new(1e-6).is_ok());

    // With the constructor closed, every reported tempo inside a ramp is a real tempo. The
    // falsifier is the removed check: without it this map builds and `tempo_at` answers
    // `NaN` at the ramp's own start.
    let map = TempoMap::new(
        bpm(120.0),
        &[
            TempoChange::ramp(MusicalTick::ZERO, bpm(120.0)),
            TempoChange::step(MusicalTick::new(4 * Q), bpm(3.4e-307)),
        ],
        rate(),
    )
    .expect("a valid map");
    for tick in (0..=4 * Q).step_by(97) {
        let reported = map.tempo_at(MusicalTick::new(tick)).as_f64();
        assert!(
            reported.is_finite() && reported > 0.0,
            "tempo at {tick} is {reported}"
        );
    }

    // The other end, and the reviewer's second construction reproduced exactly. A
    // 6000-to-1e100 BPM ramp spanning the whole tick range puts the elapsed fraction at
    // **exactly** `1.0` one tick before its end, because `beats_from` rounds the two to the
    // same value there. `p0 + (p1 - p0) * 1.0` then rounds to zero and the reciprocal is
    // infinite; weighting the two periods reports the tempo the ramp declares.
    //
    // A four-beat fixture cannot reach this: the fraction only reaches `1.0` at the ramp's
    // own end tick, which belongs to the next segment. A first version of this check used
    // one and passed the mutation it was written against, which is how the vacuity was
    // found.
    let span = MusicalTick::new(1_u64 << 53);
    let steep = TempoMap::new(
        bpm(6_000.0),
        &[
            TempoChange::ramp(MusicalTick::ZERO, bpm(6_000.0)),
            TempoChange::step(span, bpm(1e100)),
        ],
        rate(),
    )
    .expect("a valid map");
    let at_end = steep.tempo_at(MusicalTick::new(span.as_u64() - 1)).as_f64();
    assert!(
        at_end.is_finite() && at_end > 0.0,
        "the steep ramp reports {at_end} one tick before its end"
    );
    assert!(
        (at_end - 1e100).abs() <= 1e100 * 1e-12,
        "it should report the tempo the ramp declares, not {at_end}"
    );
}

#[test]
fn chained_ramps_each_reach_the_next_declared_tempo() {
    // V1 admits a ramp whose own destination is another ramp's start, and its tests use
    // one. Each segment ramps to the **next** declared tempo, so the two are independent
    // trapezoids that meet at the boundary rather than one curve through three points.
    let map = TempoMap::new(
        bpm(90.0),
        &[
            TempoChange::ramp(MusicalTick::ZERO, bpm(90.0)),
            TempoChange::ramp(MusicalTick::new(2 * Q), bpm(180.0)),
            TempoChange::step(MusicalTick::new(4 * Q), bpm(120.0)),
        ],
        rate(),
    )
    .expect("a valid map");

    // 90 -> 180 over two beats is 2 * (2/3 + 1/3) / 2 = 1.000 s; 180 -> 120 over the next
    // two is 2 * (1/3 + 1/2) / 2 = 0.8333... s, which is 40 000 frames.
    assert_eq!(
        map.position_of(MusicalTick::new(2 * Q))
            .expect("inside plan time"),
        PlanPosition::new(48_000)
    );
    assert_eq!(
        map.position_of(MusicalTick::new(4 * Q))
            .expect("inside plan time"),
        PlanPosition::new(48_000 + 40_000)
    );
    // The tempo is continuous at the junction: the first ramp arrives at 180 exactly where
    // the second departs from it.
    assert!((map.tempo_at(MusicalTick::new(2 * Q)).as_f64() - 180.0).abs() < 1e-9);
}

#[test]
fn a_ramp_with_no_change_after_it_is_a_step() {
    // ADR-0049 clause 4, which is V1's rule kept: a ramp toward nothing has no second
    // endpoint, so the map holds the declared tempo instead of inventing a destination.
    let ramped = TempoMap::new(
        bpm(120.0),
        &[TempoChange::ramp(MusicalTick::new(4 * Q), bpm(90.0))],
        rate(),
    )
    .expect("a valid map");
    let stepped = TempoMap::new(
        bpm(120.0),
        &[TempoChange::step(MusicalTick::new(4 * Q), bpm(90.0))],
        rate(),
    )
    .expect("a valid map");

    for tick in (0..=16 * Q).step_by(13) {
        let tick = MusicalTick::new(tick);
        assert_eq!(
            ramped.position_of(tick).expect("inside plan time"),
            stepped.position_of(tick).expect("inside plan time"),
            "a trailing ramp must behave as a step at {tick}"
        );
    }
}

#[test]
fn the_tempo_inside_a_ramp_is_the_reciprocal_of_the_interpolated_period() {
    // ADR-0049 clause 7, and the check exists because the plausible wrong answer is the one
    // a reader expects: halfway through a 90-to-180 ramp the *period* is halfway between
    // 2/3 s and 1/3 s, which is 1/2 s — 120 BPM — not the 135 BPM a straight line between
    // the two tempo numbers would give.
    let map = TempoMap::new(
        bpm(90.0),
        &[
            TempoChange::ramp(MusicalTick::ZERO, bpm(90.0)),
            TempoChange::step(MusicalTick::new(2 * Q), bpm(180.0)),
        ],
        rate(),
    )
    .expect("a valid map");

    assert!((map.tempo_at(MusicalTick::ZERO).as_f64() - 90.0).abs() < 1e-9);
    assert!((map.tempo_at(MusicalTick::new(Q)).as_f64() - 120.0).abs() < 1e-9);
    assert!(
        (map.tempo_at(MusicalTick::new(Q)).as_f64() - 135.0).abs() > 1.0,
        "a tempo-linear reading would answer 135 BPM here"
    );

    // The endpoint belongs to the next segment, which steps to the tempo the ramp reached.
    assert!((map.tempo_at(MusicalTick::new(2 * Q)).as_f64() - 180.0).abs() < 1e-9);
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
