//! Compiled admission over anchor phases and loops.
//!
//! The window scan itself. Its linear half is what
//! [`AdmittedCompiledStream`](crate::schedule::AdmittedCompiledStream) is built on, and that
//! type — not this module — is what preparation accepts, so the proof travels with the
//! stream instead of being repeated per call. The loop half's caller is
//! [`StreamControl::plan_activation`](crate::stream::StreamControl::plan_activation): an
//! interval joins ADR-0050's atomic set only by passing through it, so "already admitted" is
//! a fact about the value rather than a rule someone has to remember.
//! [`SessionScheduler`](crate::session::SessionScheduler) still re-anchors at a wrap without
//! carrying one, because a wrap is not implemented — the activation records the interval in
//! force and nothing repeats it yet.
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
//! # Admitting a loop proves two things, and only one of them is clause 4's
//!
//! Clause 4 is the window test: **events per quantum** against the compiled share, so that
//! "a wrap cannot fail for compiled capacity". It says nothing about how many notes a pass
//! holds open at once, and it is not the rule that would: that is `SOUND-INV-017`'s producer
//! range, which ADR-0047 partitions across admitted note-on producers and which a producer
//! may not out-emit. The two are independent — a pass of two events per second can hold a
//! thousand notes, and a pass that opens one note can be dense enough to fail the scan.
//!
//! That second rule already has two enforcement points, both in
//! [`StreamControl::plan_activation`](crate::stream::StreamControl::plan_activation): the
//! history it walks, and the suffix `stamp_into` mints. The pass a wrap replays is a **third
//! timeline** the same producer emits, and it had none — so a loop needing more identity than
//! its producer holds could be recorded and would over-emit at its first real wrap.
//! [`admit_loop_polyphony`] is that refusal, and it sits beside clause 4's because the two
//! are what admitting a loop has to prove, not because they are one rule.
//!
//! # Off the audio thread
//!
//! Clause 4 is explicit that this is finite but **not** real-time: its cost scales with the
//! events inside the loop interval, which no profile capacity bounds — only the *window* it
//! slides is bounded by `Q`. Running it inside a callback would put producer-sized work on
//! the audio thread, which ADR-0021 forbids. A loop request is validated where the plan is;
//! ADR-0055 then refuses it at the runtime offer until sample-exact adoption exists.

use thiserror::Error;

use crate::quantities::{EventCount, HeldNoteCount};
use crate::time::{AnchorPhase, FrameCount, PlanPosition, QUANTUM_FRAMES};

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

    /// A loop's periodic extension puts more events in one window than the share admits.
    ///
    /// Separate from [`Self::WindowOverShare`] because clause 4 asks a loop's refusal to
    /// name different things: "the diagnostic names the loop interval, phase, requested
    /// count and available count".
    ///
    /// **A loop's overrun has no single window to name.** A linear stream fails at one
    /// place, and naming its first over-full window points at it. A loop is a stream that
    /// repeats, so an over-full window recurs at every copy — and because a copy is
    /// `length` frames away, its phase moves with each copy unless `Q` divides the loop
    /// length. The frame is never unique and the phase **need not be**: a loop whose length
    /// is a whole number of quanta keeps one phase, while any other length spreads the same
    /// overrun across several. So this reports a **witness**: one phase at which one
    /// quantum of the periodic extension holds more than the share. That is enough to
    /// refuse, and it is what clause 4 asks to be told.
    ///
    /// The witness is the earliest over-full window of the checked extension. A start frame
    /// is deliberately not reported beside it: it is a position on the plan's axis that can
    /// fall outside `[start, end)` entirely, so it names no place in the looped material.
    ///
    /// `Q` is not a field either, where [`Self::WindowOverShare`] carries one. Clause 4's
    /// list for a loop is the interval, phase, requested and available counts, and `Q` is
    /// on none of them — it is a crate constant the message names from
    /// [`FrameCount::QUANTUM`]. The two variants therefore differ because the record asks
    /// them to, not by oversight.
    #[error(
        "loop [{start}, {end}) holds {requested} compiled events in one {} window at anchor \
         phase {} against a share of {share}",
        FrameCount::QUANTUM, .phase.as_u16()
    )]
    LoopWindowOverShare {
        /// The loop's first frame.
        start: PlanPosition,
        /// The loop's exclusive end.
        end: PlanPosition,
        /// One anchor phase whose grid makes an over-full window a quantum.
        ///
        /// A witness rather than the phase: see the variant's own documentation.
        ///
        /// An [`AnchorPhase`] and deliberately not a `QuantumOffset`: both are `0..Q`, but
        /// one describes a gridding of plan time and the other where a sample sits in the
        /// render quantum carrying it. They disagree whenever the anchor is not the
        /// identity, so one type for both would let a cross-timeline substitution
        /// type-check.
        phase: AnchorPhase,
        /// How many events that window holds.
        requested: EventCount,
        /// The compiled share.
        share: EventCount,
    },

    /// A loop's repeating pass holds more notes at once than its producer admits.
    ///
    /// **Not clause 4**, which is why this variant exists rather than another window one.
    /// Every variant above compares **events in a window** against the compiled event share;
    /// this compares **note contracts open at one instant** against the compiled producer's
    /// admitted simultaneous notes, which is `SOUND-INV-017`'s range and ADR-0047's
    /// partition. A pass can be sparse enough for every window and still hold more notes than
    /// the producer has identity indices for, because a note occupies its index from its on
    /// edge to its release rather than for the quantum it arrives in.
    ///
    /// **The subject is the pass a wrap replays, not the one an activation carries.** An
    /// activation that enters a loop late skips `[loop_start, request.position)`, and every
    /// wrap after it plays those events; a check against the suffix would therefore admit a
    /// loop whose skipped prefix opens the notes that collide.
    ///
    /// No phase and no window start, unlike [`Self::LoopWindowOverShare`]. Polyphony is not
    /// a property of a gridding of plan time: the same notes are open at the same instants
    /// whatever phase the anchor gives the loop, so there is no witness to name and
    /// reporting one would suggest a dependence that does not exist.
    #[error(
        "loop [{start}, {end}) holds {requested} at once against a producer admitting \
         {admitted}"
    )]
    LoopPolyphonyOverProducer {
        /// The loop's first frame.
        start: PlanPosition,
        /// The loop's exclusive end.
        end: PlanPosition,
        /// The most notes the repeating pass holds at one instant.
        requested: HeldNoteCount,
        /// What the compiled note producer is admitted for.
        admitted: HeldNoteCount,
    },
}

/// The **first** `Q`-frame window over `share`, and where it starts.
///
/// Only windows that *begin at an event* need checking to decide **whether** a stream is
/// admissible: sliding a window forward without passing an event cannot add one, so every
/// window's count equals that of the last window aligned to an event at or before it. That
/// is why `Q` anchor phases collapse to one pass.
///
/// **Which window is named is a different question**, and an event-aligned answer is the
/// wrong one. `HOST-INV-021` asks the refusal to name "the exact first over-full half-open
/// `Q`-frame window", and the first such window rarely begins at an event: with `Q` = 64,
/// a share of one and events at 63 and 64, every window from `[1, 65)` to `[63, 127)`
/// holds both, so the first begins at frame **1**. Naming 63 names a real over-full window
/// and not the earliest one — that is, not the anchor phase at which the stream first
/// fails.
///
/// So the scan finds the earliest event-aligned overrun and then walks its start back to
/// the earliest window holding the same overrun. Let `e_i` be the first event whose window
/// is over-full; the window must hold `share + 1` events, so it must reach `e_(i+share)`,
/// and the earliest start that does is `e_(i+share) - Q + 1`. Nothing earlier can be
/// over-full: any over-full window holds `share + 1` consecutive events `e_k..e_(k+share)`
/// with `k >= i`, and its start is therefore at least that same bound.
///
/// No event before `e_i` falls in that window either — if one did, the window aligned to
/// *it* would hold `share + 1` events and would have been found first — so the count is
/// taken from `e_i` forward.
fn first_window_over(sorted: &[u64], share: EventCount) -> Option<(u64, usize)> {
    let quantum = u64::from(QUANTUM_FRAMES);
    let mut end = 0_usize;
    for (start, first) in sorted.iter().copied().enumerate() {
        if end < start {
            end = start;
        }
        while end < sorted.len() && sorted[end].saturating_sub(first) < quantum {
            end += 1;
        }
        let count = end - start;
        if u32::try_from(count).unwrap_or(u32::MAX) <= share.get() {
            continue;
        }
        // The `share + 1`-th event in this window: the one whose presence makes it
        // over-full, and the one the earliest window has to reach.
        let Some(decisive) = share
            .as_usize()
            .and_then(|share| start.checked_add(share))
            .and_then(|index| sorted.get(index))
            .copied()
        else {
            // `count > share` means the index exists; a platform that cannot name it is
            // reported at the event-aligned start rather than answered with a guess.
            return Some((first, count));
        };
        let window_start = decisive.saturating_sub(quantum - 1);
        let mut held = 0_usize;
        for position in &sorted[start..] {
            if position.saturating_sub(window_start) >= quantum {
                break;
            }
            held += 1;
        }
        return Some((window_start, held));
    }
    None
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

    let Some((window_start, count)) = first_window_over(&extended, share) else {
        return Ok(());
    };
    // **The phase, and deliberately not the position.** The extension shares the plan's
    // origin — a copy sits at `frame + length * copy` — so this residue is a real anchor
    // phase on the plan's own axis. It is a witness rather than *the* phase: the same
    // overrun recurs at every copy, at a phase shifted by `length` each time, so a loop
    // whose length is not a multiple of `Q` fails at several phases and this names the
    // first the scan reaches.
    Err(AdmissionError::LoopWindowOverShare {
        start,
        end,
        phase: AnchorPhase::of(PlanPosition::new(window_start)),
        requested: EventCount::measured(u32::try_from(count).unwrap_or(u32::MAX)),
        share,
    })
}

/// Admit a looping compiled stream against its producer's simultaneous notes.
///
/// [`admit_loop`] proves ADR-0046 clause 4: no `Q`-frame window of the periodic extension
/// holds more **events** than the compiled share. This proves a different record's rule —
/// `SOUND-INV-017`'s, that a producer emits no more simultaneous notes than its admitted
/// range holds — over the one timeline that had no enforcement point, the pass a wrap
/// replays. Neither implies the other: a pass of two events per second can hold a thousand
/// notes open, and a pass that opens one note can be dense enough to fail the window scan.
///
/// The peak is derived by the caller rather than here, because deriving it is note-contract
/// reasoning — which release pairs with which on edge, and which release crosses into the
/// pass from before it — and that already has an owner in
/// [`StreamControl::plan_activation`](crate::stream::StreamControl::plan_activation). What
/// this owns is the comparison and the diagnostic, beside the sibling whose refusal it
/// stands next to.
///
/// **It takes a [`LoopInterval`](crate::transport::LoopInterval) where [`admit_loop`] takes
/// two positions**, and the asymmetry is deliberate rather than an oversight. `admit_loop`
/// predates that type and answers [`AdmissionError::EmptyLoop`] for a non-positive interval;
/// this entry point is new, so the case can be removed instead of adjudicated. A rule no
/// caller can reach is a rule nobody has checked, and an `EmptyLoop` branch here would be
/// exactly that. An independent review found the unvalidated pair.
///
/// **A caller with no compiled note producer must not reach this.** `admitted` would be zero
/// and every pass holding a note would be refused as one the producer admits nothing of,
/// which is a different fact from there being no producer. That refusal has its own owners —
/// `require_note_producer` for the history and `stamp_into` for the suffix — and
/// `plan_activation` skips this comparison so they keep it.
pub fn admit_loop_polyphony(
    peak: HeldNoteCount,
    admitted: HeldNoteCount,
    interval: crate::transport::LoopInterval,
) -> Result<(), AdmissionError> {
    if peak.get() <= admitted.get() {
        return Ok(());
    }
    Err(AdmissionError::LoopPolyphonyOverProducer {
        start: interval.start(),
        end: interval.end(),
        requested: peak,
        admitted,
    })
}

fn check_window(frames: &[u64], share: EventCount) -> Result<(), AdmissionError> {
    let Some((window_start, count)) = first_window_over(frames, share) else {
        return Ok(());
    };
    Err(AdmissionError::WindowOverShare {
        window_start: PlanPosition::new(window_start),
        requested: EventCount::measured(u32::try_from(count).unwrap_or(u32::MAX)),
        share,
        quantum: QUANTUM_FRAMES,
    })
}

#[cfg(test)]
#[path = "tests/admit.rs"]
mod tests;
