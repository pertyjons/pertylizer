//! The identity table's construction and minting.
//!
//! Off the audio thread. `HOST-INV-009` puts the atomic slot-hold-identity acquisition at
//! the live boundary, and construction allocates — which is why the resolving half lives in
//! `hot.rs`: the real-time purity region is file-granular and admits no mixed file.

use super::{
    INDEX_SPACE, IdentityError, IdentityTable, LiveNotes, NoteIdentity, PRODUCER_SPACE, ProducerId,
    Range, Slot, TableId, issue_table_id,
};
use crate::quantities::HeldNoteCount;

/// The disjoint index spans a producer list implies, in declaration order.
///
/// Shared by both identity halves rather than written twice. They must agree exactly —
/// [`IdentityTable`] mints inside a producer's span and [`LiveNotes`] clears inside the same
/// one — and two loops computing one partition is how they come to disagree.
fn producer_spans(producer_ranges: &[HeldNoteCount]) -> Vec<Range> {
    let mut ranges = Vec::with_capacity(producer_ranges.len());
    let mut start = 0_u32;
    for range in producer_ranges {
        ranges.push(Range {
            start,
            len: range.get(),
        });
        start = start.saturating_add(range.get());
    }
    ranges
}

impl IdentityTable {
    /// Build a table whose producers get disjoint ranges, in the order given.
    ///
    /// `max_held_notes` is the profile's, and the relation against the index space is
    /// checked here rather than assumed — `max_held_notes` is otherwise constrained only to
    /// be nonzero, so nothing else would catch a profile that can hold more notes than it
    /// can name.
    pub fn new(
        max_held_notes: HeldNoteCount,
        producer_ranges: &[HeldNoteCount],
    ) -> Result<Self, IdentityError> {
        Self::with_generation_ceiling(max_held_notes, producer_ranges, u32::MAX)
    }

    /// Build a table from ranges plan admission has already checked.
    ///
    /// No `max_held_notes` argument, because the relation it guards is already established
    /// upstream: profile construction refuses `max_held_notes` above the index space, and
    /// plan admission refuses a producer set whose ranges outrun `max_held_notes`. Asking
    /// for the figure again here would invite a caller to supply a different one, and two
    /// answers to one question is how they come to disagree.
    pub fn from_admitted_ranges(producer_ranges: &[HeldNoteCount]) -> Result<Self, IdentityError> {
        Self::with_generation_ceiling(
            HeldNoteCount::measured(INDEX_SPACE),
            producer_ranges,
            u32::MAX,
        )
    }

    /// The same, with the generation ceiling named.
    ///
    /// The ceiling is what decides how long a stream runs before indices begin retiring —
    /// a liveness property, not a safety one, since a generation value is never reused
    /// whatever the ceiling is.
    pub fn with_generation_ceiling(
        max_held_notes: HeldNoteCount,
        producer_ranges: &[HeldNoteCount],
        generation_ceiling: u32,
    ) -> Result<Self, IdentityError> {
        if producer_ranges.len() > PRODUCER_SPACE {
            return Err(IdentityError::TooManyProducers {
                count: producer_ranges.len(),
                available: PRODUCER_SPACE,
            });
        }
        if max_held_notes.get() > INDEX_SPACE {
            return Err(IdentityError::IndexSpaceBelowHeldNotes {
                available: INDEX_SPACE,
                needed: max_held_notes,
            });
        }

        let mut needed: u64 = 0;
        for range in producer_ranges {
            needed = needed.saturating_add(u64::from(range.get()));
        }
        if needed > u64::from(INDEX_SPACE) {
            return Err(IdentityError::RangesExceedIndexSpace {
                needed,
                available: INDEX_SPACE,
            });
        }

        let id = issue_table_id().ok_or(IdentityError::TableSpaceExhausted)?;
        // Refused rather than substituted with zero: an empty table would pass construction
        // and fail every mint, which is worse than a construction that says it cannot.
        let total =
            usize::try_from(needed).map_err(|_| IdentityError::ExtentUnallocatable { needed })?;
        let ranges = producer_spans(producer_ranges);

        Ok(Self {
            id,
            slots: vec![Slot::Free { next_generation: 0 }; total],
            ranges,
            generation_ceiling,
            retired: 0,
            live: 0,
        })
    }

    /// This table's identity.
    pub const fn id(&self) -> TableId {
        self.id
    }

    /// A working copy, for an operation that must either complete or leave nothing behind.
    ///
    /// **Not a general clone, and deliberately not `Clone`.** The copy carries the same
    /// [`TableId`], so two of these alive at once would both answer to occurrences the other
    /// minted. The only sanctioned use is commit-or-discard: take a copy, do the whole
    /// operation on it, and either assign it back or drop it. `stamp_compiled` is the caller,
    /// and it needs this because minting is what fails — a list can pair correctly and still
    /// exceed the producer's range, so a check before the first mint cannot catch every case
    /// without reimplementing allocation. Restoring by releasing what was minted would not
    /// restore: a release advances the generation, and the paired releases the aborted list
    /// already performed are not recoverable that way.
    pub(crate) fn working_copy(&self) -> Self {
        Self {
            id: self.id,
            slots: self.slots.clone(),
            ranges: self.ranges.clone(),
            generation_ceiling: self.generation_ceiling,
            retired: self.retired,
            live: self.live,
        }
    }

    /// How many indices this table has retired.
    pub const fn retired(&self) -> u64 {
        self.retired
    }

    /// How many notes are live.
    pub const fn live(&self) -> u32 {
        self.live
    }

    /// Mint an identity for `producer`, or say why it cannot.
    ///
    /// Records the node the occurrence plays, which is what lets a release carry the
    /// identity alone: the node was named here, at the note-on, so nothing later has to.
    ///
    /// Takes the lowest free index in the producer's range. Lowest rather than round-robin
    /// because retirement makes the two differ in a way that matters: round-robin would
    /// spread generation consumption evenly and retire every index at about the same time,
    /// while taking the lowest concentrates reuse and leaves the upper part of the range
    /// fresh for longer.
    pub fn mint(
        &mut self,
        producer: ProducerId,
        note: crate::plan::NoteSlot,
    ) -> Result<NoteIdentity, IdentityError> {
        let range = *self
            .ranges
            .get(usize::from(producer.as_u16()))
            .ok_or(IdentityError::UnknownProducer { producer })?;

        for offset in 0..range.len {
            let index = range.start.saturating_add(offset);
            let Some(slot) = self.slots.get_mut(index as usize) else {
                break;
            };
            if let Slot::Free { next_generation } = *slot {
                *slot = Slot::Live {
                    generation: next_generation,
                    note,
                };
                self.live = self.live.saturating_add(1);
                return Ok(NoteIdentity {
                    table: self.id,
                    index: u16::try_from(index).unwrap_or(u16::MAX),
                    generation: next_generation,
                });
            }
        }

        // No free index. **Which condition this is matters**, and the two are not the same
        // fault: if every index is live the producer is holding more notes at once than it
        // was admitted for, which is over-emission; if some are retired, the usable range
        // shrank underneath a producer that declared correctly. Reporting the second as the
        // first would send someone to fix a producer that is behaving.
        let mut retired = 0_u32;
        for offset in 0..range.len {
            let index = range.start.saturating_add(offset);
            if matches!(self.slots.get(index as usize), Some(Slot::Retired)) {
                retired = retired.saturating_add(1);
            }
        }
        let admitted = HeldNoteCount::measured(range.len);
        if retired == 0 {
            Err(IdentityError::ProducerOverEmitted { producer, admitted })
        } else {
            Err(IdentityError::ProducerRangeEroded {
                producer,
                admitted,
                retired,
            })
        }
    }

    /// Build the successor of this table, refusing while it still holds obligations.
    ///
    /// `SOUND-INV-017`'s rebuild rule, and ADR-0046 clause 3 is why it is a refusal rather
    /// than a reset: rejecting the eventual release would refuse an accepted obligation, and
    /// stranding it would leave a note nothing can release. Refusing the rebuild breaks
    /// neither. Lifting the limit is ADR-0048's, for Phase 9.
    /// **Borrows rather than consumes**, and that is not a style choice. A consuming
    /// signature that returns only an error drops the table on refusal — which strands
    /// exactly the obligations the refusal exists to preserve. An independent review found
    /// that in the first version of this method.
    ///
    /// What this cannot prevent is a caller dropping a live table and calling
    /// [`Self::new`]. Enforcing that needs an owner outliving both tables, which is the
    /// same gap the publication arbiter's identity records: a type can refuse the operation
    /// it is asked to perform, not one it is never told about.
    pub fn rebuild(
        &self,
        max_held_notes: HeldNoteCount,
        producer_ranges: &[HeldNoteCount],
    ) -> Result<Self, IdentityError> {
        if self.live > 0 {
            return Err(IdentityError::RebuildWithObligationsOutstanding {
                table: self.id,
                outstanding: self.live,
            });
        }
        Self::with_generation_ceiling(max_held_notes, producer_ranges, self.generation_ceiling)
    }
}

impl LiveNotes {
    /// A registry for the occurrences a table with these ranges can mint.
    ///
    /// Takes the minting table's [`TableId`] rather than issuing one: an occurrence carries
    /// the id of the table that minted it, and the renderer's foreign check compares against
    /// this. Two ids would make every one of its own events look foreign.
    ///
    /// Sized to the whole partition, so `admit` is an indexed write that cannot grow and
    /// cannot fail for want of room.
    pub fn for_ranges(
        id: TableId,
        producer_ranges: &[super::super::quantities::HeldNoteCount],
    ) -> Result<Self, IdentityError> {
        let mut needed: u64 = 0;
        for range in producer_ranges {
            needed = needed.saturating_add(u64::from(range.get()));
        }
        if needed > u64::from(INDEX_SPACE) {
            return Err(IdentityError::RangesExceedIndexSpace {
                needed,
                available: INDEX_SPACE,
            });
        }
        let total =
            usize::try_from(needed).map_err(|_| IdentityError::ExtentUnallocatable { needed })?;
        Ok(Self {
            id,
            slots: vec![None; total],
            ranges: producer_spans(producer_ranges),
        })
    }

    /// How many bytes its storage occupies, for the resource report.
    ///
    /// Both allocations, not only the slots: the registry keeps its own copy of the producer
    /// spans so a scoped mass release can resolve a producer on the audio thread, and a
    /// figure that omitted them would understate the registry by exactly that table.
    pub const fn storage_bytes(&self) -> usize {
        self.slots.len() * size_of::<Option<super::LiveNote>>()
            + self.ranges.len() * size_of::<Range>()
    }
}

impl IdentityTable {
    /// How many bytes its storage occupies, for the resource report.
    ///
    /// The twin of [`LiveNotes::storage_bytes`], and it exists so that admission's budget has
    /// something to be checked *against*. `render::identity_bytes` predicts what the two
    /// halves take; these two report what they took. Without both, the prediction can only be
    /// tested by restating its own formula, which is not a test.
    pub const fn storage_bytes(&self) -> usize {
        self.slots.len() * size_of::<Slot>() + self.ranges.len() * size_of::<Range>()
    }
}
