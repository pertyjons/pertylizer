//! Note identity: which occurrence an event names.
//!
//! `SOUND-INV-017`. A note-on names an occurrence as well as a node; a release and a
//! per-note expression event name the occurrence alone, and the occurrence is the sole
//! authority for which note an event resolves to.
//!
//! # Why this exists at all
//!
//! ADR-0046 clause 3 already promises that an orphan note edge "is counted rather than
//! allowed to release another note". With `{ node, edge }` that sentence has no
//! implementable meaning: a release whose note-on was dropped is indistinguishable from a
//! legitimate one, so it releases whatever is sounding on that node. Identity is what makes
//! the promise executable, which is why it is Phase 3's and not Phase 6's.
//!
//! # An orphan is a definition, not a list
//!
//! **An identity that names no live note is an orphan.** Three cases reach that, and the
//! third is why the definition is stated generally: a **free** index, a **superseded**
//! generation at a live index, and a **retired** index. A rule listing only the first two
//! leaves no classification for the state retirement creates — which an earlier draft of
//! the specification did, and which three review rounds were spent finding.
//!
//! # What is bought, and what is not
//!
//! A generation value is never reused, so no stale identity can match the generation live at
//! its index, whatever retained it and for however long. **No finite identity is
//! unconditionally alias-free**, and this module does not pretend otherwise: what the rule
//! buys is the *direction* of failure. Exhausting an index's generations retires that index
//! and is counted; a producer whose remaining range falls below its admitted demand raises a
//! named exhaustion. A width chosen too small costs a reported exhaustion, never a wrongly
//! released note.

use thiserror::Error;

use crate::quantities::HeldNoteCount;

/// One identity table's own identity.
///
/// A table is rebuilt whenever the ranges it holds are established, and neither existing
/// value scopes an identity alone: a re-admission changes the plan without changing the
/// epoch, and a re-preparation changes the epoch without changing the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct TableId(u64);

impl TableId {
    /// The raw identity, for a report.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for TableId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "identity table {}", self.0)
    }
}

static NEXT_TABLE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Issue the next table identity, or `None` once the space is spent.
///
/// **Refuses permanently rather than saturating.** `fetch_update` leaves the counter at its
/// ceiling, so every later call refuses too; `fetch_add` would wrap at `u64::MAX` and the
/// refusal would be one hiccup followed by reissued identities, which is the one thing this
/// type exists to prevent. `Relaxed` is sufficient — the identities must differ, not order
/// anything.
fn issue_table_id() -> Option<TableId> {
    use std::sync::atomic::Ordering::Relaxed;
    NEXT_TABLE_ID
        .fetch_update(Relaxed, Relaxed, |id| id.checked_add(1))
        .ok()
        .map(TableId)
}

/// Which producer a range belongs to.
///
/// Ranges are disjoint at admission, so an identity is attributable to one producer without
/// carrying a producer tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct ProducerId(u16);

impl ProducerId {
    /// A producer's index among those admitted.
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    /// The raw index.
    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

impl std::fmt::Display for ProducerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "producer {}", self.0)
    }
}

/// Which occurrence a note event names.
///
/// Three private fields, and none is separately meaningful: the table that minted it, an
/// index into the minting producer's range, and a generation that advances every time the
/// index is reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct NoteIdentity {
    pub(super) table: TableId,
    pub(super) index: u16,
    pub(super) generation: u32,
}

impl NoteIdentity {
    /// Which table minted it.
    pub const fn table(self) -> TableId {
        self.table
    }
}

impl std::fmt::Display for NoteIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "note {}/{} in {}",
            self.index, self.generation, self.table
        )
    }
}

/// What resolving an identity found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// The identity names a live note.
    Live,
    /// The identity names no live note; see [`OrphanCause`] for which way.
    Orphan(OrphanCause),
    /// The identity was minted by a different table.
    ///
    /// Not an orphan: it says nothing about whether the note it named is live, only that
    /// this table cannot answer. ADR-0032 clause 20's epoch rejection is a separate and
    /// earlier filter, so an event from a dead stream never reaches this at all.
    ForeignTable {
        /// The table this identity came from.
        minted_by: TableId,
    },
}

/// Which of the three ways an identity can name no live note.
///
/// Distinguished rather than merged because they point at different producer defects: a
/// free index means the note was already released, a superseded generation means the index
/// has been reused since, and a retired index means it will never hold a note again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanCause {
    /// The index holds no note.
    FreeIndex,
    /// The index holds a note, but a later one than this identity named.
    SupersededGeneration,
    /// The index was withdrawn after its generation space ran out.
    RetiredIndex,
}

/// Why the table could not be built, or could not mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IdentityError {
    /// The table identity space is spent.
    #[error("the identity-table space is exhausted")]
    TableSpaceExhausted,

    /// The index space cannot address every simultaneously held note.
    ///
    /// `SOUND-INV-017`'s construction relation. `max_held_notes` bounds how many
    /// obligations exist at once, and an index space below it would let a profile name more
    /// held notes than it can identify.
    #[error("an index space of {available} cannot address max_held_notes {needed}")]
    IndexSpaceBelowHeldNotes {
        /// What the index type can address.
        available: u32,
        /// What the profile admits.
        needed: HeldNoteCount,
    },

    /// The producers' ranges do not fit the index space.
    #[error("producer ranges need {needed} indices against an index space of {available}")]
    RangesExceedIndexSpace {
        /// The sum of the requested ranges.
        needed: u64,
        /// What the index type can address.
        available: u32,
    },

    /// More producers than a [`ProducerId`] can name.
    #[error("{count} producers exceed the {available} a ProducerId can address")]
    TooManyProducers {
        /// How many were offered.
        count: usize,
        /// How many can be named.
        available: usize,
    },

    /// The admitted extent cannot be allocated on this target.
    ///
    /// Refused rather than substituted with an empty table. `unwrap_or(0)` here would build
    /// a table that passes construction and fails every mint, which is the shape of defect
    /// the repository's rules exist to prevent.
    #[error("an extent of {needed} indices cannot be allocated on this target")]
    ExtentUnallocatable {
        /// The extent asked for.
        needed: u64,
    },

    /// A table was rebuilt while it still held obligations.
    ///
    /// `SOUND-INV-017`, and ADR-0046 clause 3's guarantee is why: rejecting the eventual
    /// release would refuse an accepted obligation, and stranding it would leave a note
    /// nothing can release. Refusing the *rebuild* is the only outcome that breaks neither.
    #[error("{outstanding} obligations are still outstanding in {table}")]
    RebuildWithObligationsOutstanding {
        /// The table asked to rebuild.
        table: TableId,
        /// How many notes it still holds.
        outstanding: u32,
    },

    /// A producer that was never admitted.
    #[error("{producer} has no admitted range")]
    UnknownProducer {
        /// The producer named.
        producer: ProducerId,
    },

    /// Every index in a producer's range is live.
    ///
    /// **A producer defect.** Nothing was lost to retirement; the producer is simply holding
    /// more notes at once than it was admitted for, which is over-emission.
    #[error("{producer} holds all {admitted} of its admitted indices")]
    ProducerOverEmitted {
        /// The producer that ran out.
        producer: ProducerId,
        /// The range it was admitted with.
        admitted: HeldNoteCount,
    },

    /// Retirement has taken a producer's range below its admitted demand.
    ///
    /// **Not a producer defect**, and named separately for that reason. The producer
    /// declared correctly and did not over-emit — its indices were consumed by retirement,
    /// which is the price of never reusing a generation. Reporting it as over-emission
    /// would send someone to fix a producer that is behaving correctly.
    #[error("{producer} has {retired} of its {admitted} indices retired and none free")]
    ProducerRangeEroded {
        /// The producer that ran out.
        producer: ProducerId,
        /// The range it was admitted with.
        admitted: HeldNoteCount,
        /// How many of its indices are gone for good.
        retired: u32,
    },
}

/// What a mass release reaches.
///
/// ADR-0046 clause 6 says the allocator applies the operation "to owned voices within the
/// source event", so the scope is part of the operation rather than always everything. A
/// sustain lift on one source must not end another source's notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseScope {
    /// Every producer's obligations: a panic or a transport stop.
    Everything,
    /// One producer's obligations: a sustain lift or a script-driven mass release.
    Producer(ProducerId),
}

/// One index's state.
///
/// A live index records **which node its note plays**, and that is what lets a release carry
/// the identity alone. `SOUND-INV-017` removes the node address from the release rather than
/// carrying it and requiring agreement, so something has to remember it — and the occurrence
/// is the only thing that can, because the node was named when the occurrence was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Holds no note; the generation is the next one it will mint.
    Free { next_generation: u32 },
    /// Holds a note minted at this generation, playing this node.
    Live {
        generation: u32,
        note: crate::plan::NoteSlot,
    },
    /// Withdrawn after its generation space ran out.
    Retired,
}

/// What one index type can address.
///
/// `u16` because an index names a *simultaneously outstanding* obligation, which
/// `max_held_notes` bounds, and 65 536 exceeds any profile this project has reason to
/// construct. The relation is checked rather than assumed: [`IdentityTable::new`] refuses a
/// profile whose `max_held_notes` exceeds this.
pub(crate) const INDEX_SPACE: u32 = 1 << 16;

/// How many producers a [`ProducerId`] can name.
pub(super) const PRODUCER_SPACE: usize = 1 << 16;

/// One producer's half-open span of the index space.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Range {
    pub(super) start: u32,
    pub(super) len: u32,
}

/// The identities one plan activation can mint.
#[derive(Debug)]
#[must_use]
pub struct IdentityTable {
    pub(super) id: TableId,
    pub(super) slots: Vec<Slot>,
    pub(super) ranges: Vec<Range>,
    /// The highest generation an index may mint before it retires.
    ///
    /// A construction parameter rather than a constant, because ADR-0047 leaves the widths
    /// to Phase 3 measurement — and because retirement is otherwise unreachable in a test:
    /// walking a `u32` to its ceiling by minting would take longer than this project will
    /// exist, and a rule no test can reach is a rule nobody has checked.
    pub(super) generation_ceiling: u32,
    /// Indices retired over this table's life, counted as `SOUND-INV-017` requires.
    pub(super) retired: u64,
    pub(super) live: u32,
}

/// What the renderer knows about a live note.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LiveNote {
    pub(super) generation: u32,
    pub(super) note: crate::plan::NoteSlot,
}

/// Which occurrences are currently sounding, and on which node.
///
/// **A registry, not a minter, and the split is load-bearing.** [`IdentityTable`] models a
/// producer's *occupancy*: it allocates an index, advances its generation, and frees it when
/// the note ends. A compiled schedule is stamped ahead of the render — for a whole piece at
/// once — so the table's occupancy at stamping time is the schedule's polyphony, which is
/// what `simultaneous_notes` bounds. It is **not** what is sounding at any moment during the
/// render, and an index the table has already freed and reissued is exactly the case where
/// the two disagree.
///
/// This is what the audio thread reads instead. It mints nothing and owns no generation
/// counter: a note-on **admits** its occurrence together with the node the edge names, and a
/// release resolves through it and clears it. So the renderer's answer to "which note does
/// this release end" comes from the event stream it is applying, in the order it applies it,
/// rather than from a table that ran ahead.
#[derive(Debug)]
#[must_use]
pub struct LiveNotes {
    pub(super) id: TableId,
    pub(super) slots: Vec<Option<LiveNote>>,
}

#[path = "identity/table.rs"]
mod table;

/// The audio-thread half: resolving and releasing.
///
/// A separate file because the real-time purity region is **file-granular** and admits no
/// mixed hot/preparation file — `table.rs` allocates in its constructor, so the two cannot
/// share one. Minting stays there: `HOST-INV-009` puts the atomic slot-hold-identity
/// acquisition at the live boundary, which is off the audio thread.
#[path = "identity/hot.rs"]
mod hot;

#[cfg(test)]
#[path = "tests/identity.rs"]
mod tests;
