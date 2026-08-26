//! Compiled admission over anchor phases and loops.
//!
//! ADR-0046 clause 4. A compiled plan is admitted against the compiled share, and the check
//! is not "how many events land in each absolute quantum" — that is the wrong question,
//! because which quantum a frame belongs to depends on where the stream was anchored.
//!
//! Two events at frames 63 and 64 sit in different quanta from an anchor at zero and in the
//! same quantum from an anchor one frame later. A plan admitted on the first reading would
//! fault at publication after an ordinary seek. The clause says so directly: admission
//! "rejects the plan if any window of `Q` consecutive integer frame positions contains more
//! events than the share", and "that sliding-window test is exactly the worst case over all
//! `Q` integer anchor phases".
//!
//! # Loops are periodic, not linear
//!
//! A loop is not a window over the plan; it is a stream that repeats. The tail of one pass
//! and the head of the next fall inside one `Q`-frame window whenever the wrap does, and a
//! loop shorter than `Q` puts several whole passes there. Clause 4 requires validating the
//! periodic extension of `[loop_start, loop_end)` over every anchor phase, repeating enough
//! cycles to cover one window — "so loops shorter than `Q` are not a special hole".
//!
//! At most `ceil(Q / loop_length) + 2` copies can intersect a `Q`-frame window: the copies
//! fully inside it, plus the one straddling each end. Extending by that many and sliding
//! over the result is exact rather than approximate.
//!
//! # Off the audio thread
//!
//! Clause 4 is explicit that this is finite but **not** real-time: its cost scales with the
//! events inside the loop interval, which no profile capacity bounds — only the *window* it
//! slides is bounded by `Q`. Running it inside a callback would put producer-sized work on
//! the audio thread, which ADR-0021 forbids. A loop change is validated where the plan is,
//! and the audio thread only ever adopts an already-admitted loop.

use thiserror::Error;

use crate::quantities::EventCount;
use crate::time::{PlanPosition, QUANTUM_FRAMES};

/// Why a compiled stream was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AdmissionError {
    /// Some `Q`-frame window holds more compiled events than the share admits.
    #[error(
        "a {quantum}-frame window at {window_start} holds {requested} compiled events against a \
         share of {share}"
    )]
    WindowOverShare {
        /// First frame of the densest window found.
        window_start: PlanPosition,
        /// How many events it holds.
        requested: EventCount,
        /// The compiled share.
        share: EventCount,
        /// `Q`, in frames.
        quantum: u32,
    },

    /// A loop whose bounds are not a positive half-open interval.
    #[error("loop [{start}, {end}) is not a positive interval")]
    EmptyLoop {
        /// The loop's first frame.
        start: PlanPosition,
        /// The loop's exclusive end.
        end: PlanPosition,
    },

    /// The periodic extension needed for the check cannot be represented.
    #[error("the periodic extension of loop [{start}, {end}) overflows plan time")]
    ExtensionUnrepresentable {
        /// The loop's first frame.
        start: PlanPosition,
        /// The loop's exclusive end.
        end: PlanPosition,
    },
}

/// The densest `Q`-frame window over a sorted position list, and where it starts.
///
/// Only windows that *begin at an event* need checking: sliding a window forward without
/// passing an event cannot add one, so every window's count equals that of the last window
/// aligned to an event at or before it. That is why `Q` anchor phases collapse to one pass.
fn densest_window(sorted: &[u64]) -> Option<(u64, usize)> {
    let quantum = u64::from(QUANTUM_FRAMES);
    let mut best: Option<(u64, usize)> = None;
    let mut end = 0_usize;
    for (start, first) in sorted.iter().copied().enumerate() {
        if end < start {
            end = start;
        }
        while end < sorted.len() && sorted[end].saturating_sub(first) < quantum {
            end += 1;
        }
        let count = end - start;
        if best.is_none_or(|(_, seen)| count > seen) {
            best = Some((first, count));
        }
    }
    best
}

/// Admit a linear compiled stream against its share.
///
/// `positions` must be sorted; a compiled schedule already is, and sorting here would hide
/// a caller that lost that ordering.
pub fn admit_linear(positions: &[PlanPosition], share: EventCount) -> Result<(), AdmissionError> {
    let frames: Vec<u64> = positions.iter().map(|position| position.as_u64()).collect();
    check_window(&frames, share)
}

/// Admit a looping compiled stream against its share.
///
/// Validates the periodic extension of `[start, end)`, which is what makes a wrap safe: once
/// this passes, no wrap can fail for compiled capacity, and the audio thread does no
/// wrap-time work at all.
pub fn admit_loop(
    positions: &[PlanPosition],
    start: PlanPosition,
    end: PlanPosition,
    share: EventCount,
) -> Result<(), AdmissionError> {
    if end <= start {
        return Err(AdmissionError::EmptyLoop { start, end });
    }
    let length = end.as_u64() - start.as_u64();
    let quantum = u64::from(QUANTUM_FRAMES);

    // The copies that can intersect one `Q`-frame window: those wholly inside it, plus the
    // one straddling each end. Clause 4's `ceil(Q / loop_length) + 2`.
    let copies = quantum.div_ceil(length).saturating_add(2);

    let inside: Vec<u64> = positions
        .iter()
        .map(|position| position.as_u64())
        .filter(|frame| *frame >= start.as_u64() && *frame < end.as_u64())
        .collect();
    if inside.is_empty() {
        return Ok(());
    }

    let mut extended: Vec<u64> = Vec::with_capacity(inside.len().saturating_mul(copies as usize));
    for copy in 0..copies {
        let Some(offset) = length.checked_mul(copy) else {
            return Err(AdmissionError::ExtensionUnrepresentable { start, end });
        };
        for frame in &inside {
            let Some(shifted) = frame.checked_add(offset) else {
                return Err(AdmissionError::ExtensionUnrepresentable { start, end });
            };
            extended.push(shifted);
        }
    }
    extended.sort_unstable();
    check_window(&extended, share)
}

fn check_window(frames: &[u64], share: EventCount) -> Result<(), AdmissionError> {
    let Some((window_start, count)) = densest_window(frames) else {
        return Ok(());
    };
    let requested = u32::try_from(count).unwrap_or(u32::MAX);
    if requested > share.get() {
        return Err(AdmissionError::WindowOverShare {
            window_start: PlanPosition::new(window_start),
            requested: EventCount::measured(requested),
            share,
            quantum: QUANTUM_FRAMES,
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/admit.rs"]
mod tests;
