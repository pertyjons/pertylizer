//! Compiled admission's checks.

use super::*;

const Q: u64 = QUANTUM_FRAMES as u64;

fn share(n: u32) -> EventCount {
    EventCount::limit(n).expect("a positive share")
}

fn at(frames: &[u64]) -> Vec<PlanPosition> {
    frames.iter().copied().map(PlanPosition::new).collect()
}

#[test]
fn two_events_astride_a_quantum_boundary_are_one_window() {
    // The gap this module exists to close, and the reason a per-absolute-quantum count is
    // the wrong question. Frames 63 and 64 sit in different quanta from an anchor at zero
    // and in the *same* quantum from an anchor one frame later — so a plan admitted by
    // counting quanta would fault at publication after an ordinary seek.
    let positions = at(&[63, 64]);
    assert!(
        admit_linear(&positions, share(2)).is_ok(),
        "two events fit a share of two"
    );
    match admit_linear(&positions, share(1)) {
        Err(AdmissionError::WindowOverShare {
            window_start,
            requested,
            share: named,
            quantum,
        }) => {
            assert_eq!(window_start, PlanPosition::new(63));
            assert_eq!(requested, EventCount::measured(2));
            assert_eq!(named, share(1), "the refusal names both sides");
            assert_eq!(quantum, QUANTUM_FRAMES);
        }
        other => panic!("expected a window refusal, got {other:?}"),
    }
}

#[test]
fn events_a_full_quantum_apart_are_two_windows() {
    // The boundary of the half-open window: `Q` frames apart is outside, `Q - 1` is inside.
    // An off-by-one here would either admit a plan that overruns or refuse one that fits.
    assert!(
        admit_linear(&at(&[0, Q]), share(1)).is_ok(),
        "exactly Q apart"
    );
    assert!(
        admit_linear(&at(&[0, Q - 1]), share(1)).is_err(),
        "one frame closer is one window"
    );
}

#[test]
fn the_densest_window_is_found_wherever_it_sits() {
    // A sparse plan with one dense cluster late in it. Scanning only from the start, or
    // only on quantum boundaries, would miss it.
    let mut frames = vec![0, 5 * Q, 10 * Q];
    for offset in 0..8 {
        frames.push(40 * Q + offset * 3);
    }
    frames.sort_unstable();
    match admit_linear(&at(&frames), share(7)) {
        Err(AdmissionError::WindowOverShare {
            window_start,
            requested,
            ..
        }) => {
            assert_eq!(window_start, PlanPosition::new(40 * Q));
            assert_eq!(requested, EventCount::measured(8));
        }
        other => panic!("expected the late cluster to be found, got {other:?}"),
    }
    assert!(admit_linear(&at(&frames), share(8)).is_ok());
}

#[test]
fn a_loop_wrap_puts_the_tail_and_the_head_in_one_window() {
    // The property a linear check cannot have. These four events are spread across a loop
    // and no `Q`-frame window of the *linear* plan holds more than two — but at the wrap the
    // tail and the head meet, and the periodic extension sees all four.
    let length = 4 * Q;
    let frames = [0_u64, 10, 4 * Q - 20, 4 * Q - 10];
    let positions = at(&frames);

    assert!(
        admit_linear(&positions, share(2)).is_ok(),
        "linearly, two per window is enough"
    );
    assert!(
        admit_loop(
            &positions,
            PlanPosition::ZERO,
            PlanPosition::new(length),
            share(2)
        )
        .is_err(),
        "at the wrap the tail and the head are one window"
    );
    assert!(
        admit_loop(
            &positions,
            PlanPosition::ZERO,
            PlanPosition::new(length),
            share(4)
        )
        .is_ok()
    );
}

#[test]
fn a_loop_shorter_than_a_quantum_is_not_a_special_hole() {
    // Clause 4's words. With a loop of `Q / 4` a single window holds four whole passes, so
    // an extension of one or two copies would understate it by a factor of two. This is the
    // case a naive "check one wrap" implementation gets wrong.
    let length = Q / 4;
    let positions = at(&[0, 1]);

    // Four passes of two events each fall inside one `Q`-frame window.
    assert!(
        admit_loop(
            &positions,
            PlanPosition::ZERO,
            PlanPosition::new(length),
            share(7)
        )
        .is_err(),
        "eight events from four passes exceed a share of seven"
    );
    assert!(
        admit_loop(
            &positions,
            PlanPosition::ZERO,
            PlanPosition::new(length),
            share(8)
        )
        .is_ok(),
        "and fit a share of eight"
    );
}

#[test]
fn only_events_inside_the_loop_interval_repeat() {
    // The interval is half-open, and the **end frame itself** is the case that matters: it
    // is one loop length after the start, so treating it as inside would duplicate the
    // loop's first frame on every pass. An earlier version of this test put every event
    // *past* the end, where `<` and `<=` agree, and a mutation to `<=` passed it.
    let length = 4 * Q;
    let positions = at(&[0, 1, 4 * Q]);

    assert!(
        admit_loop(
            &positions,
            PlanPosition::ZERO,
            PlanPosition::new(length),
            share(2)
        )
        .is_ok(),
        "the loop plays two events per pass: the frame at its exclusive end is not its own"
    );

    // And the same events with the end frame counted in would collide with the next pass's
    // start, which is exactly the overrun the half-open reading avoids.
    assert!(
        admit_loop(
            &positions,
            PlanPosition::ZERO,
            PlanPosition::new(length + 1),
            share(2)
        )
        .is_err(),
        "extending the loop by one frame does bring that event inside, and it collides"
    );
}

#[test]
fn an_empty_loop_is_refused_rather_than_extended() {
    // A zero or inverted interval has no period, so `ceil(Q / length)` is a division by
    // zero. Refusing names the bounds; extending would need a period to invent.
    for (start, end) in [(4 * Q, 4 * Q), (8 * Q, 4 * Q)] {
        match admit_loop(
            &at(&[0]),
            PlanPosition::new(start),
            PlanPosition::new(end),
            share(4),
        ) {
            Err(AdmissionError::EmptyLoop { start: a, end: b }) => {
                assert_eq!(a, PlanPosition::new(start));
                assert_eq!(b, PlanPosition::new(end));
            }
            other => panic!("expected an empty-loop refusal, got {other:?}"),
        }
    }
}

#[test]
fn an_empty_plan_is_admitted() {
    // Nothing to place cannot overrun anything, and a check that refused it would make an
    // empty plan unloadable.
    assert!(admit_linear(&[], share(1)).is_ok());
    assert!(admit_loop(&[], PlanPosition::ZERO, PlanPosition::new(Q), share(1)).is_ok());
}
