//! Transport activation: moving a rendering stream to a new plan mapping.
//!
//! [ADR-0050](../../../plans/v2/decisions/ADR-0050-transport-activation.md), and the types
//! here are its clauses made constructible. A seek, a loop wrap or a tempo-map replacement
//! is **one value**, built off the audio thread by
//! [`StreamControl`](crate::stream::StreamControl) and adopted whole at a quantum boundary.
//!
//! # What the shapes are for
//!
//! **The effective point is quantum-granular** (clause 1). An activation names a requested
//! engine time; it takes effect at the first quantum boundary at or after `max(requested,
//! clock)`. That is not merely what ADR-0001 permits — it is what keeps the junction
//! admissible, because shares are charged per destination quantum and a boundary-aligned
//! activation never puts two streams' events in one of them.
//!
//! **Adoption is an infallible move** (clause 3). Everything that can refuse happens while
//! the candidate is built, or at the **offer**, where there are five: a schedule paired with
//! another stream's renderer, a stream that has already faulted, a stale epoch, a superseded
//! sequence, and an occupied exchange slot. Once an offer is accepted, nothing between it and
//! the boundary can invalidate the candidate.
//!
//! **The exchange is one slot used in both directions.** It holds the pending candidate
//! until adoption and the retired value until the control collects it, so nothing is
//! deallocated on the audio thread. An offer while it is occupied is refused, and the two
//! occupancies are told apart because they need different fixes.
//!
//! # What this module does not carry yet
//!
//! The tempo map. Clause 3 puts it in the atomic set, and it belongs there — but
//! [`SessionScheduler`](crate::session::SessionScheduler) owns the only one that exists and
//! nothing yet replaces one during playback. When a caller for that arrives, the map joins
//! [`TransportActivation`] and the retired value, and this comment goes with it.

use thiserror::Error;

use crate::identity::{IdentityTable, ProducerId};
use crate::plan::NoteSlot;
use crate::render::TimedEvent;
use crate::time::{PlanPosition, SampleTime, StreamEpoch};

/// Which activation a stream is running under.
///
/// Strictly increasing, and it changes at **adoption alone** — never at issue. That is what
/// makes an abandoned candidate consume nothing: the value in force did not move, so the
/// next candidate built against it is still adoptable and a cancelled seek cannot wedge the
/// stream.
///
/// A newtype rather than a `u64` because it is compared against exactly one other thing —
/// the value a candidate supersedes — and a bare integer here would be one call away from
/// being compared with an epoch, a frame count or an event count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct ActivationSequence(u64);

impl ActivationSequence {
    /// The value a stream is opened with, before any activation has been adopted.
    pub const INITIAL: Self = Self(0);

    /// The next value, or `None` once the space is spent.
    ///
    /// Refuses permanently rather than wrapping, for the reason `issue_epoch` does: a
    /// reissued sequence would make a superseded candidate adoptable, which is the one
    /// thing this type exists to prevent.
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }

    /// The raw value, for a report.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for ActivationSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "activation {}", self.0)
    }
}

/// A half-open plan interval a transport repeats.
///
/// ADR-0046 clause 4 admits a loop by validating the periodic extension of
/// `[start, end)`, so the interval is half-open here for the same reason it is there: the
/// end frame belongs to the next pass, and a closed interval would place two events one
/// loop length apart in one window that the extension never checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopInterval {
    start: PlanPosition,
    end: PlanPosition,
}

impl LoopInterval {
    /// A loop, or `None` where the interval is not positive.
    ///
    /// **An interval carried by an activation is admitted, but not enforced.** No wrap is
    /// implemented, so nothing repeats it: the candidate's schedule is not bounded by
    /// [`Self::end`], and an event past it plays and reserves an identity where under
    /// wrapping it would be unreachable. That enforcement is the wrap slice's, and so is the
    /// identity question it turns on — a wrap replays a list whose occurrences were minted
    /// once, which ADR-0052 owes an answer before any of it can be built.
    ///
    /// What **is** proved when the candidate is built is two bounds on the pass a wrap would
    /// produce, answering to two records: ADR-0046 clause 4's event density against the
    /// compiled share, and `SOUND-INV-017`'s producer range against the notes that pass holds
    /// at once. The second is not clause 4 — a wrap "cannot fail for compiled **capacity**",
    /// and capacity is the share — and it was missing while the first stood alone, so a loop
    /// entered after one note opened could be recorded here and still over-emit at its first
    /// real wrap.
    ///
    /// Checked at construction so that no later caller has to ask. An empty or inverted
    /// loop is not a small loop: it has no periodic extension at all, so every downstream
    /// rule about wraps would be reasoning about nothing.
    pub const fn new(start: PlanPosition, end: PlanPosition) -> Option<Self> {
        if end.as_u64() <= start.as_u64() {
            return None;
        }
        Some(Self { start, end })
    }

    /// The loop's first frame.
    pub const fn start(self) -> PlanPosition {
        self.start
    }

    /// The loop's exclusive end.
    pub const fn end(self) -> PlanPosition {
        self.end
    }

    /// How long one pass is.
    ///
    /// A [`FrameCount`](crate::time::FrameCount) rather than a `u64`: it is a duration in
    /// frames, [`Self::wrap_at`] adds multiples of it to a [`SampleTime`], and the unit
    /// belongs in the type rather than in the caller's memory.
    pub const fn length(self) -> crate::time::FrameCount {
        crate::time::FrameCount::new(self.end.as_u64() - self.start.as_u64())
    }

    /// The requested engine time of the `pass`-th wrap after `first`.
    ///
    /// **Derived from the ideal timeline, never from the previous wrap's effective point.**
    /// ADR-0050 clause 1: a loop whose length is not a whole number of quanta snaps at every
    /// wrap, and deriving each wrap from the last actual one accumulates those roundings
    /// until the loop is permanently longer than the one the user set. Under this
    /// derivation each error is an independent value below one quantum, so the audible
    /// period jitters and returns.
    ///
    /// The two are indistinguishable for a loop length that happens to be a multiple of `Q`,
    /// which is exactly why the ideal derivation is a rule rather than an implementation
    /// detail.
    pub const fn wrap_at(self, first: SampleTime, pass: u64) -> Option<SampleTime> {
        let Some(offset) = self.length().as_u64().checked_mul(pass) else {
            return None;
        };
        match first.as_u64().checked_add(offset) {
            Some(time) => Some(SampleTime::new(time)),
            None => None,
        }
    }
}

/// Why an activation was not accepted at the offer.
///
/// Every variant leaves the stream running on the state in force. None of them is a fault:
/// ADR-0050 clause 3 puts every refusal here precisely so that adoption itself has no branch
/// that can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ActivationRefused {
    /// The candidate was built for another stream.
    #[error("the activation belongs to {candidate}, but the stream is {stream}")]
    StaleEpoch {
        /// The epoch the candidate carries.
        candidate: StreamEpoch,
        /// The stream's own.
        stream: StreamEpoch,
    },

    /// The candidate supersedes a state that is no longer in force.
    ///
    /// Two candidates built against one value are ordered rather than raced: whichever is
    /// adopted first moves the value, and this refuses the other. A superseded intent can
    /// therefore never activate after the intent that replaced it.
    #[error("the activation supersedes {supersedes}, but {in_force} is in force")]
    Superseded {
        /// What the candidate expected to replace.
        supersedes: ActivationSequence,
        /// What is actually in force.
        in_force: ActivationSequence,
    },

    /// The offer paired this schedule with another stream's renderer.
    ///
    /// Checked **before** anything else, and before the renderer's counters are touched:
    /// the refusal counters belong to the stream that was offered to, so attributing this
    /// one to a renderer that is not its half would put a diagnostic on the wrong stream.
    /// `render` refuses the same pairing later; this is the same rule, one call earlier.
    #[error("the schedule belongs to {schedule}, but the renderer offered is {renderer}")]
    ForeignRenderer {
        /// The schedule's epoch.
        schedule: StreamEpoch,
        /// The renderer's.
        renderer: StreamEpoch,
    },

    /// The stream has ended and is waiting to be re-prepared.
    ///
    /// After a terminal fault no later call advances toward a boundary, so an accepted
    /// candidate would sit in the exchange forever: `collect` never produces it, and the
    /// control cannot withdraw what it no longer holds. Reporting an impossible state change
    /// as accepted is the failure this refuses; an independent review found it.
    #[error("the stream has faulted and is waiting to be re-prepared")]
    StreamFaulted,

    /// A candidate is already waiting to be adopted.
    #[error("an activation is already pending")]
    AlreadyPending,

    /// A retired activation is still waiting to be collected.
    ///
    /// Told apart from [`Self::AlreadyPending`] because the two need different fixes: this
    /// one means the **off-thread half has not kept up**, while a pending candidate means
    /// the audio thread has not reached the boundary yet.
    #[error("a retired activation has not been collected")]
    RetiredUncollected,
}

/// One activation, and the **same allocation** in both directions.
///
/// Before adoption its fields are the incoming state; after adoption they are the retired
/// state, and the exchange slot it sits in is what says which. That is not a convenience:
/// adoption may not allocate, and a separate retired type would need a second box made on the
/// audio thread. So the vectors are **swapped** with the scheduler's live ones and the same
/// box travels back for the control to free.
///
/// [`Self::effective`] is the discriminator a caller reads, and it is `None` until adoption.
///
/// **After adoption the value is deliberately mixed, and reading it as one thing is the
/// mistake to avoid.** Only what the scheduler swaps is retired: the event list, plus the
/// scalars in [`Self::retired`]. The minter copy, the outstanding set, the sequence and the
/// position are the
/// *candidate's* own and are now the ones in force — which is exactly why they travel here
/// rather than in the control: collection is the first moment adoption is a fact, so the
/// promotion has to arrive with the proof.
#[derive(Debug)]
#[must_use]
pub struct TransportActivation {
    pub(crate) epoch: StreamEpoch,
    pub(crate) sequence: ActivationSequence,
    pub(crate) supersedes: ActivationSequence,
    /// The engine time the caller asked for. **Immutable**, exactly as an event's stamp is
    /// under ADR-0043: the effective point may be later, and the difference is what the
    /// late counter reports.
    pub(crate) requested: SampleTime,
    /// The plan position the transport moves to. The engine half of the anchor is snapped
    /// at adoption; this half is not, because moving it would seek somewhere other than
    /// where the caller asked.
    pub(crate) position: PlanPosition,
    /// The stamped schedule — incoming before adoption, retired after it.
    pub(crate) events: Vec<TimedEvent>,
    /// The occurrences that schedule leaves outstanding in the allocator.
    ///
    /// **Not** swapped at adoption: it describes this candidate's own schedule, and after
    /// adoption that is the schedule in force. The control takes it when it collects.
    pub(crate) outstanding: Vec<crate::identity::NoteIdentity>,
    /// The working copy of the minter this candidate was stamped against.
    ///
    /// ADR-0050 clause 3 stamps a candidate against a copy so that abandoning it costs
    /// neither an index nor a generation, and clause 6 promotes that copy at **collection**
    /// rather than at acceptance, because collection is the first moment adoption is a fact.
    /// Carrying the copy here rather than in the control is what makes the two rules one
    /// mechanism: a candidate that is never adopted is dropped with its copy inside it, and
    /// nothing in the control has to be un-done. It also lets two candidates be built
    /// against one in-force sequence, which clause 6 deliberately allows.
    ///
    /// Never read on the audio thread.
    pub(crate) minter: IdentityTable,
    /// Releases the suffix omitted because their note-on lies before the new anchor.
    ///
    /// ADR-0050 clause 5. Counted rather than dropped silently: ADR-0001 clause 16 forbids
    /// the second, and a named transformation is what tells a reader that a seek through a
    /// held note did something rather than nothing.
    pub(crate) omitted_releases: usize,
    /// ADR-0051 clause 1's catch-up batch: one row per prepared target, restoring the
    /// destination's control state before the new stream's own events at that sample.
    pub(crate) catch_up: Vec<crate::render::TimedEvent>,
    /// The loop in force after adoption.
    pub(crate) loop_interval: Option<LoopInterval>,
    /// The producers whose sounding notes the boundary ends.
    pub(crate) producers: Vec<ProducerId>,
    /// Preallocated storage for the mass release's node reports.
    ///
    /// Sized by the builder to the identity partition, because
    /// [`LiveNotes::release_all`](crate::identity::LiveNotes::release_all) is all-or-nothing
    /// against exactly that bound. Carrying it with the candidate is what keeps adoption
    /// free of allocation.
    pub(crate) ended: Vec<Option<NoteSlot>>,
    /// Whether the clock had already passed [`Self::requested`] when the offer was accepted.
    ///
    /// ADR-0050 clause 1's lateness, and it is decided **at the offer** because that is the
    /// moment the rule is about: the candidate was finished after the time it named had
    /// passed. Deciding it at adoption cannot work — by the call that adopts, the clock has
    /// reached the boundary, so every request that does not fall on one looks late. Two
    /// earlier revisions tried to derive it there and both reported quantum snapping as
    /// delayed preparation.
    pub(crate) late: bool,
    /// Where this activation took effect, once it has.
    pub(crate) effective: Option<SampleTime>,
    /// The state the activation replaced, filled at adoption.
    ///
    /// The scalars of the atomic set clause 3 exchanges. The vectors travel by being swapped
    /// into the fields above; these have nowhere else to go, and a retirement that could not
    /// report the cursor and loop it replaced would not be the state it claims to return.
    pub(crate) retired: Option<RetiredState>,
}

/// What an adopted activation replaced, beyond the storage it swapped.
///
/// ADR-0050 clause 3 exchanges an atomic set; the allocations come back by being swapped, and
/// these are the members that are values rather than allocations. An independent review found
/// the cursor and the loop being dropped instead of returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct RetiredState {
    /// The anchor the replaced mapping used.
    pub anchor: crate::time::StreamAnchor,
    /// How far its schedule had been released.
    pub cursor: usize,
    /// The loop that was in force.
    pub loop_interval: Option<LoopInterval>,
    /// The displacement its own placed events were read under.
    ///
    /// Without it the returned event list is uninterpretable: the times it carries are the
    /// ones the retired schedule was *stamped* at, and the schedule it belonged to added this
    /// at every read. Adoption overwrites the live value, so a retirement that did not carry
    /// its own would describe its events against a displacement that is no longer anywhere.
    /// An independent review found the gap.
    pub shift: crate::time::FrameCount,
}

impl TransportActivation {
    /// What this activation replaced, once it has been adopted.
    pub const fn retired(&self) -> Option<RetiredState> {
        self.retired
    }

    /// Whether the clock had passed the requested time when the offer was accepted.
    pub const fn was_late(&self) -> bool {
        self.late
    }

    /// The engine time the caller asked for.
    pub const fn requested(&self) -> SampleTime {
        self.requested
    }

    /// The plan position the transport moves to.
    pub const fn position(&self) -> PlanPosition {
        self.position
    }

    /// This activation's own sequence, which becomes the value in force when it is adopted.
    pub const fn sequence(&self) -> ActivationSequence {
        self.sequence
    }

    /// The sequence it expects to replace.
    pub const fn supersedes(&self) -> ActivationSequence {
        self.supersedes
    }

    /// The loop in force after adoption.
    pub const fn loop_interval(&self) -> Option<LoopInterval> {
        self.loop_interval
    }

    /// Where it took effect, or `None` while it is still a candidate.
    ///
    /// Later than [`Self::requested`] when the candidate was finished after that time had
    /// passed, or when the request did not fall on a quantum boundary.
    pub const fn effective(&self) -> Option<SampleTime> {
        self.effective
    }

    /// How many events its schedule holds — incoming before adoption, retired after.
    pub fn events(&self) -> usize {
        self.events.len()
    }

    /// How many occurrences that schedule reserves in the allocator.
    pub fn outstanding(&self) -> usize {
        self.outstanding.len()
    }

    /// How many rows the catch-up batch carries.
    ///
    /// One per prepared target, so this is the quantity a plan-dependent session-share
    /// admission will bound once it exists. `SessionEventShare` is reported against the
    /// share today but is not yet admission-checked; see `compile.rs`'s session row.
    #[must_use]
    pub fn catch_up(&self) -> usize {
        self.catch_up.len()
    }

    /// How many releases the suffix omitted because their note-on precedes the anchor.
    pub const fn omitted_releases(&self) -> usize {
        self.omitted_releases
    }

    /// How many producers its boundary release names.
    pub fn producers(&self) -> usize {
        self.producers.len()
    }

    /// How many notes that release can name at once.
    pub fn release_capacity(&self) -> usize {
        self.ended.len()
    }
}

/// The single-slot exchange between a stream's two halves.
///
/// One slot used in both directions: a candidate travels in, a retired value travels out.
/// An offer while it is occupied is refused rather than queued, which is
/// `HOST-INV-022`'s backpressure — and the two occupancies are distinguishable because
/// only one of them means the off-thread half fell behind.
#[derive(Debug, Default)]
#[must_use]
pub(crate) enum Exchange {
    /// Nothing in flight.
    #[default]
    Empty,
    /// A candidate accepted and not yet adopted.
    Pending(Box<TransportActivation>),
    /// A retired value adopted and not yet collected.
    ///
    /// The same box the candidate arrived in, its vectors now holding what the activation
    /// replaced. One allocation, exchanged rather than remade.
    Retired(Box<TransportActivation>),
}

impl Exchange {
    /// Why an offer cannot be accepted into this slot, if it cannot.
    pub(crate) const fn occupied(&self) -> Option<ActivationRefused> {
        match self {
            Self::Empty => None,
            Self::Pending(_) => Some(ActivationRefused::AlreadyPending),
            Self::Retired(_) => Some(ActivationRefused::RetiredUncollected),
        }
    }
}

#[cfg(test)]
#[path = "tests/transport.rs"]
mod tests;
