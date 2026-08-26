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
            // Frame **1**, not 63: every window from `[1, 65)` to `[63, 127)` holds both
            // events, and `HOST-INV-021` asks for the first — the anchor phase at which
            // the stream first fails, rather than the first event of the cluster.
            assert_eq!(window_start, PlanPosition::new(1));
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
fn an_over_full_window_is_found_wherever_it_sits() {
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
            // The eighth event of the cluster sits at `40Q + 21`, and the earliest window
            // reaching it starts `Q - 1` frames earlier.
            assert_eq!(window_start, PlanPosition::new(40 * Q + 21 - (Q - 1)));
            assert_eq!(requested, EventCount::measured(8));
        }
        other => panic!("expected the late cluster to be found, got {other:?}"),
    }
    assert!(admit_linear(&at(&frames), share(8)).is_ok());
}

#[test]
fn the_named_window_is_the_earliest_one_and_not_merely_an_event_aligned_one() {
    // Brute force over every start frame, against the scan's answer. The scan only checks
    // event-aligned windows to decide *whether* a stream fits; naming the first one is a
    // separate step, and this is what proves the two agree.
    let cases: [(&[u64], u32); 4] = [
        (&[63, 64], 1),
        (&[0, 1, 2, 40 * Q, 40 * Q + 1, 40 * Q + 2, 40 * Q + 3], 2),
        (&[10, 11, 12, 13], 2),
        (&[0, Q - 1, 2 * Q - 2, 3 * Q - 3], 1),
    ];
    for (frames, admitted) in cases {
        let mut expected = None;
        let last = frames.last().copied().unwrap_or(0);
        for start in 0..=last + Q {
            let held = frames
                .iter()
                .filter(|frame| **frame >= start && **frame < start + Q)
                .count();
            if held > admitted as usize {
                expected = Some((start, held));
                break;
            }
        }
        let (start, held) = expected.expect("each case has an over-full window");
        match admit_linear(&at(frames), share(admitted)) {
            Err(AdmissionError::WindowOverShare {
                window_start,
                requested,
                ..
            }) => {
                assert_eq!(window_start, PlanPosition::new(start), "{frames:?}");
                assert_eq!(requested, EventCount::measured(held as u32), "{frames:?}");
            }
            other => panic!("expected a refusal for {frames:?}, got {other:?}"),
        }
    }
}

#[test]
fn the_refusal_names_the_first_over_full_window_not_the_densest() {
    // `HOST-INV-021` asks for "the exact first over-full half-open `Q`-frame window". Both
    // clusters here are over a share of two, and the later one is denser — a scan that kept
    // the maximum would name frame `40Q` and send someone past the overrun they have to fix
    // first.
    let frames = at(&[0, 1, 2, 40 * Q, 40 * Q + 1, 40 * Q + 2, 40 * Q + 3]);
    match admit_linear(&frames, share(2)) {
        Err(AdmissionError::WindowOverShare {
            window_start,
            requested,
            ..
        }) => {
            assert_eq!(window_start, PlanPosition::ZERO);
            assert_eq!(requested, EventCount::measured(3));
        }
        other => panic!("expected the earliest overrun to be named, got {other:?}"),
    }
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

/// The greatest common divisor, for the oracle's alignment cycle below.
const fn gcd(a: u64, b: u64) -> u64 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a
}

/// Whether any quantum overruns `share` when the grid's boundaries fall at `phase`.
///
/// The oracle for the loop diagnostic, and deliberately not a restatement of the scan: it
/// materialises the periodic stream over many passes and buckets by absolute quantum index,
/// which is the definition clause 4's sliding window is an optimisation *of*.
fn overruns_at_phase(frames: &[u64], start: u64, end: u64, share: u32, phase: u64) -> bool {
    let length = end - start;
    let inside: Vec<u64> = frames
        .iter()
        .copied()
        .filter(|frame| *frame >= start && *frame < end)
        .collect();
    // **A full alignment cycle, not a handful of passes.** Copy `n` sits `length * n`
    // frames along, so relative to a fixed grid its phase shifts by `length % Q` each time
    // and only returns to where it started after `Q / gcd(Q, length)` copies. A bound like
    // `Q / length + 4` covers a window straddling one wrap and nothing else: with a loop of
    // 129 frames it materialises four copies, while events at 0 and 63 first share a
    // phase-10 quantum at copy **10** — so the oracle would answer "no overrun" for a phase
    // that overruns. Found by an independent review of this test.
    //
    // The implementation is unaffected, and the reason is worth keeping: the stream is
    // periodic, so a window's content depends only on its start modulo `length`, and
    // sliding over `ceil(Q / length) + 2` copies already covers every residue. The oracle
    // asks a different question — "does *this* phase overrun" — and that one does need the
    // cycle.
    let cycle = Q / gcd(Q, length.max(1));
    let passes = cycle + (Q / length.max(1)) + 4;
    let mut counts = std::collections::BTreeMap::new();
    for pass in 0..passes {
        for frame in &inside {
            let placed = frame + length * pass;
            // The grid whose boundaries fall at `phase`: a frame belongs to the quantum
            // that starts at the greatest boundary at or below it.
            //
            // **Shifted by one whole quantum rather than saturated.** A frame before the
            // first boundary has a negative index, and `saturating_sub` folded every such
            // frame into bucket 0 together with the first *complete* quantum — which the
            // interior trim below then threw away as an edge. Loop `[22, 24)` with an event
            // at 23 and phase 38 bucketed as `{0: 40, 1: 28}`, left no interior at all, and
            // answered "no overrun" while `[38, 102)` really held 32 events. Adding `Q`
            // shifts every index by exactly one, so the grouping is untouched and nothing
            // merges. Found by an independent review.
            let bucket = (placed + Q - phase) / Q;
            *counts.entry(bucket).or_insert(0_u32) += 1;
        }
    }
    // The first and last buckets see only part of the materialised passes, so they can
    // undercount; the interior is what the periodic stream really does.
    let interior: Vec<u32> = counts
        .values()
        .copied()
        .skip(1)
        .take(counts.len().saturating_sub(2))
        .collect();
    interior.iter().any(|held| *held > share)
}

#[test]
fn the_oracle_sees_a_phase_that_only_aligns_many_copies_later() {
    // A check on the checker. A loop of 129 frames is coprime with `Q`, so each copy shifts
    // the grid by one frame and events at 0 and 63 first share a quantum at copy **10**, at
    // phase 10. An oracle that materialised only a few passes would answer "no overrun"
    // there and would then be unable to falsify anything about such loops.
    assert!(
        overruns_at_phase(&[0, 63], 0, 129, 1, 10),
        "copy 10 puts frames 1290 and 1353 in the phase-10 quantum [1290, 1354)"
    );
    // And the implementation refuses this loop regardless of which copy is looked at,
    // because the stream is periodic: the window at frame 0 already holds both.
    assert!(
        admit_loop(
            &at(&[0, 63]),
            PlanPosition::ZERO,
            PlanPosition::new(129),
            share(1),
        )
        .is_err()
    );
}

#[test]
fn the_oracle_keeps_pre_boundary_frames_out_of_the_first_complete_quantum() {
    // The second check on the checker. A short loop whose events all precede the phase's
    // first boundary used to collapse into bucket zero along with the first complete
    // quantum, which the interior trim then discarded — the oracle inspected nothing and
    // answered "no overrun".
    assert!(
        overruns_at_phase(&[23], 22, 24, 6, 38),
        "the quantum [38, 102) holds 32 events of a two-frame loop, well over a share of six"
    );
}

#[test]
fn a_loop_refusal_names_the_interval_and_a_phase_that_really_overruns() {
    // Clause 4: "the diagnostic names the loop interval, phase, requested count and
    // available count". The interval is the caller's own, so the load-bearing half is the
    // phase — and this checks it against the definition rather than against the scan.
    let start = 200_u64;
    let end = 300_u64;
    let frames = [200, 299];
    match admit_loop(
        &at(&frames),
        PlanPosition::new(start),
        PlanPosition::new(end),
        share(1),
    ) {
        Err(AdmissionError::LoopWindowOverShare {
            start: named_start,
            end: named_end,
            phase,
            requested,
            share: named_share,
        }) => {
            assert_eq!(named_start, PlanPosition::new(start));
            assert_eq!(named_end, PlanPosition::new(end));
            assert_eq!(named_share, share(1), "the refusal names both sides");
            assert!(requested > EventCount::measured(1));
            // The witness holds: at the named phase, some quantum of the periodic stream
            // really does hold more than the share.
            assert!(
                overruns_at_phase(&frames, start, end, 1, u64::from(phase.as_u16())),
                "phase {} was named but no quantum overruns there",
                phase.as_u16()
            );
        }
        other => panic!("expected a loop refusal, got {other:?}"),
    }
}

#[test]
fn an_admitted_loop_overruns_at_no_phase_at_all() {
    // The control, and the reason the witness is worth anything. Two events a full `Q`
    // apart inside a loop that is a whole number of quanta long: no grid puts them in one
    // quantum, so admission must accept and the oracle must agree at every one of the `Q`
    // phases.
    let start = 0_u64;
    let end = 4 * Q;
    let frames = [0, Q, 2 * Q, 3 * Q];
    assert!(
        admit_loop(
            &at(&frames),
            PlanPosition::new(start),
            PlanPosition::new(end),
            share(1),
        )
        .is_ok(),
        "one event per quantum, and the wrap lands on a boundary"
    );
    for phase in 0..Q {
        assert!(
            !overruns_at_phase(&frames, start, end, 1, phase),
            "phase {phase} overruns, so admission should have refused"
        );
    }
}

#[test]
fn the_named_phase_is_the_only_one_that_overruns_when_only_one_does() {
    // The witness test above is weak on its own fixture: a wrap puts two events one frame
    // apart, and 63 of the 64 phases separate nothing, so almost any phase would satisfy
    // it. This fixture has exactly **one** witnessing phase, so the named value is pinned
    // rather than merely plausible.
    //
    // Two events `Q - 1` apart sit in one quantum only when a boundary falls exactly on
    // the first of them: a quantum `[p + kQ, p + kQ + Q)` holds both `a` and `a + Q - 1`
    // iff `p + kQ == a`. The loop is a whole number of quanta long, so no copy shifts it.
    let start = 0_u64;
    let end = 100 * Q;
    let frames = [10, 10 + Q - 1];
    let witnesses: Vec<u64> = (0..Q)
        .filter(|phase| overruns_at_phase(&frames, start, end, 1, *phase))
        .collect();
    assert_eq!(
        witnesses,
        vec![10],
        "the premise: exactly one phase can overrun"
    );

    match admit_loop(
        &at(&frames),
        PlanPosition::new(start),
        PlanPosition::new(end),
        share(1),
    ) {
        Err(AdmissionError::LoopWindowOverShare { phase, .. }) => {
            assert_eq!(u64::from(phase.as_u16()), 10);
        }
        other => panic!("expected a loop refusal, got {other:?}"),
    }
}

#[test]
fn a_loop_refusal_is_not_the_linear_one() {
    // Two variants rather than one, because they answer different questions: a linear
    // stream fails at a place worth naming, a loop fails at a phase that recurs. Reusing
    // the linear variant would report a `window_start` in the extension — a frame that can
    // sit outside the loop entirely.
    let refusal = admit_loop(
        &at(&[0, 1]),
        PlanPosition::ZERO,
        PlanPosition::new(100),
        share(1),
    )
    .expect_err("two events in one window is over a share of one");
    assert!(
        matches!(refusal, AdmissionError::LoopWindowOverShare { .. }),
        "a loop refusal must not arrive as the linear variant: {refusal:?}"
    );
}

#[test]
fn an_empty_plan_is_admitted() {
    // Nothing to place cannot overrun anything, and a check that refused it would make an
    // empty plan unloadable.
    assert!(admit_linear(&[], share(1)).is_ok());
    assert!(admit_loop(&[], PlanPosition::ZERO, PlanPosition::new(Q), share(1)).is_ok());
}
