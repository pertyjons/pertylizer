//! The identity table itself: minting, resolving and retirement.

use super::{
    IdentityError, NoteIdentity, OrphanCause, ProducerId, ReleaseScope, Resolution, Slot, TableId,
    issue_table_id,
};
use crate::quantities::HeldNoteCount;

/// What one index type can address.
///
/// `u16` because an index names a *simultaneously outstanding* obligation, which
/// `max_held_notes` bounds, and 65 536 exceeds any profile this project has reason to
/// construct. The relation is checked rather than assumed: [`IdentityTable::new`] refuses a
/// profile whose `max_held_notes` exceeds this.
pub(crate) const INDEX_SPACE: u32 = 1 << 16;

/// How many producers a [`ProducerId`] can name.
const PRODUCER_SPACE: usize = 1 << 16;

/// One producer's half-open span of the index space.
#[derive(Debug, Clone, Copy)]
struct Range {
    start: u32,
    len: u32,
}

/// The identities one plan activation can mint.
#[derive(Debug)]
#[must_use]
pub struct IdentityTable {
    id: TableId,
    slots: Vec<Slot>,
    ranges: Vec<Range>,
    /// The highest generation an index may mint before it retires.
    ///
    /// A construction parameter rather than a constant, because ADR-0047 leaves the widths
    /// to Phase 3 measurement — and because retirement is otherwise unreachable in a test:
    /// walking a `u32` to its ceiling by minting would take longer than this project will
    /// exist, and a rule no test can reach is a rule nobody has checked.
    generation_ceiling: u32,
    /// Indices retired over this table's life, counted as `SOUND-INV-017` requires.
    retired: u64,
    live: u32,
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
        let mut ranges = Vec::with_capacity(producer_ranges.len());
        let mut start = 0_u32;
        for range in producer_ranges {
            ranges.push(Range {
                start,
                len: range.get(),
            });
            start = start.saturating_add(range.get());
        }

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
    /// Takes the lowest free index in the producer's range. Lowest rather than round-robin
    /// because retirement makes the two differ in a way that matters: round-robin would
    /// spread generation consumption evenly and retire every index at about the same time,
    /// while taking the lowest concentrates reuse and leaves the upper part of the range
    /// fresh for longer.
    pub fn mint(&mut self, producer: ProducerId) -> Result<NoteIdentity, IdentityError> {
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

    /// What an identity names.
    pub fn resolve(&self, identity: NoteIdentity) -> Resolution {
        if identity.table != self.id {
            return Resolution::ForeignTable {
                minted_by: identity.table,
            };
        }
        match self.slots.get(usize::from(identity.index)) {
            Some(Slot::Live { generation }) if *generation == identity.generation => {
                Resolution::Live
            }
            Some(Slot::Live { .. }) => Resolution::Orphan(OrphanCause::SupersededGeneration),
            Some(Slot::Retired) => Resolution::Orphan(OrphanCause::RetiredIndex),
            Some(Slot::Free { .. }) | None => Resolution::Orphan(OrphanCause::FreeIndex),
        }
    }

    /// Release the note an identity names, freeing its index for reuse.
    ///
    /// Returns what the identity resolved to. Only [`Resolution::Live`] releases anything:
    /// an orphan is refused, which is ADR-0046 clause 3's "counted rather than allowed to
    /// release another note" made executable.
    ///
    /// **Advancing the generation is what makes the released identity stale**, and it is
    /// where retirement happens: an index whose generation space is spent is withdrawn
    /// instead of restarted, because reusing a generation would let a stale identity match a
    /// live note.
    pub fn release(&mut self, identity: NoteIdentity) -> Resolution {
        let resolution = self.resolve(identity);
        if resolution != Resolution::Live {
            return resolution;
        }
        let index = usize::from(identity.index);
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = if identity.generation >= self.generation_ceiling {
                self.retired = self.retired.saturating_add(1);
                Slot::Retired
            } else {
                Slot::Free {
                    next_generation: identity.generation.saturating_add(1),
                }
            };
            self.live = self.live.saturating_sub(1);
        }
        resolution
    }

    /// Release every live note, as a panic, transport stop or sustain lift does.
    ///
    /// One bounded operation rather than one event per voice, per ADR-0046 clause 6. Every
    /// identity it ends becomes an orphan by the same rule an individual release uses, so a
    /// release arriving afterwards for a note this took is refused rather than double-freed.
    ///
    /// **Scoped**, because clause 6 applies the operation "to owned voices within the source
    /// event". A panic or a transport stop reaches everything; a sustain lift or a
    /// script-driven mass release reaches one producer and must not end another's notes.
    pub fn release_all(&mut self, scope: ReleaseScope) -> u32 {
        let (first, last) = match scope {
            ReleaseScope::Everything => (0_usize, self.slots.len()),
            ReleaseScope::Producer(producer) => {
                let Some(range) = self.ranges.get(usize::from(producer.as_u16())) else {
                    return 0;
                };
                let start = range.start as usize;
                (start, start.saturating_add(range.len as usize))
            }
        };
        let mut ended = 0;
        for index in first..last {
            let Some(slot) = self.slots.get_mut(index) else {
                break;
            };
            if let Slot::Live { generation } = *slot {
                *slot = if generation >= self.generation_ceiling {
                    self.retired = self.retired.saturating_add(1);
                    Slot::Retired
                } else {
                    Slot::Free {
                        next_generation: generation.saturating_add(1),
                    }
                };
                ended += 1;
            }
        }
        self.live = self.live.saturating_sub(ended);
        ended
    }
}
