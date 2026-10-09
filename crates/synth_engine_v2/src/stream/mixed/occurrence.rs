//! Merger-minted handles for the private mixed owner's live occurrences.
//!
//! The off-thread merger names an occurrence when it stages the onset: a
//! [`LiveOccurrenceId`] is one bounded ledger cell and that cell's generation. Every
//! later command for the occurrence names the handle, so a release can travel in the
//! same command batch as its onset, before any onset result returns. The audio half
//! keeps a table of the same width mapping each handle to the identity it accepted
//! (`Sounding`) or to the first refusal of its onset (`Refused`, a tombstone). A
//! release ends a sounding occurrence or is consumed by its tombstone; either way audio
//! retires the cell and acknowledges the retirement in the release's result. The merger
//! reuses a cell only after collecting that acknowledgement, and both halves advance
//! the cell's generation at retirement, so a handle from a reaped cell never resolves.
//!
//! Each open handle owns one command-ring place reserved for its release, so the
//! ledger can never hold more cells than the ring can carry releases for. Sustain,
//! Panic and bend are not handled here yet.

use thiserror::Error;

use super::{MixedIngressCommandId, MixedIngressQueueCount};
use crate::{identity::NoteIdentity, ingress::IngressRefused, quantities::EventCount};

mod hot;

/// One bounded cell of the occurrence ledger and of the audio table.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[must_use]
pub(super) struct LiveOccurrenceCell(u32);

impl LiveOccurrenceCell {
    fn index(self) -> Option<usize> {
        usize::try_from(self.0).ok()
    }
}

/// How often a cell has been retired; a handle names exactly one occupancy of its cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[must_use]
pub(super) struct LiveOccurrenceGeneration(u32);

impl LiveOccurrenceGeneration {
    const FIRST: Self = Self(0);
}

/// A merger-minted name for one live occurrence: a ledger cell and its generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub(super) struct LiveOccurrenceId {
    cell: LiveOccurrenceCell,
    generation: LiveOccurrenceGeneration,
}

/// What the audio half holds for one occupied cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MixedOccurrenceState {
    /// The onset was accepted; the release ends this identity.
    Sounding(NoteIdentity),
    /// A tombstone carrying the onset's first refusal; the release is consumed.
    Refused(IngressRefused),
}

/// How a release retired its cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MixedRetireCause {
    /// The release took the occurrence's reserved queue place and ends this identity.
    Released(NoteIdentity),
    /// The onset had been refused; the release was consumed and counted.
    AfterRefusal(IngressRefused),
}

/// The audio half's acknowledgement that a cell is retired and may be reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MixedRetired {
    pub(super) occurrence: LiveOccurrenceId,
    pub(super) cause: MixedRetireCause,
}

/// Why the audio half refused a command's handle. No cell changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(super) enum MixedHandleRefused {
    #[error("live occurrence cell is outside the prepared table")]
    UnknownCell,
    #[error("live occurrence generation {offered:?} is older than the cell's {current:?}")]
    Stale {
        offered: LiveOccurrenceGeneration,
        current: LiveOccurrenceGeneration,
    },
    #[error("live occurrence generation {offered:?} was never issued; the cell is at {current:?}")]
    Unissued {
        offered: LiveOccurrenceGeneration,
        current: LiveOccurrenceGeneration,
    },
    #[error("live occurrence cell already holds an onset")]
    Occupied,
    #[error("live occurrence cell holds no onset to release")]
    NotOpen,
    #[error("live occurrence cell has spent its generations")]
    Exhausted,
}

/// Why the merger could not stage an onset. The caller keeps its input; nothing was
/// minted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(super) enum MixedOnsetSubmitError {
    #[error("no command-ring place for an onset and its reserved release")]
    Full,
    #[error("mixed ingress command identity is exhausted")]
    IdentityExhausted,
}

/// Why the merger could not submit a release. The handle stays open with its
/// reserved place.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(super) enum MixedReleaseSubmitError {
    #[error("the live occurrence is not open at the merger")]
    NotOpen(LiveOccurrenceId),
    #[error("mixed ingress command identity is exhausted")]
    IdentityExhausted(LiveOccurrenceId),
}

/// One cell's use, shared by the merger ledger and the audio table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellUse<S> {
    Free(LiveOccurrenceGeneration),
    Held(LiveOccurrenceGeneration, S),
    Exhausted,
}

/// The merger's side of one cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LedgerHold {
    /// Onset staged; one ring place stays reserved for the release.
    Open,
    /// The release was submitted as this command; the cell waits for `Retired`.
    Releasing(MixedIngressCommandId),
}

/// The off-thread merger's fixed occurrence ledger. It mints handles and frees a cell
/// only when it collects the audio half's retirement of that exact handle.
#[derive(Debug)]
pub(super) struct MixedOccurrenceLedger {
    cells: Vec<CellUse<LedgerHold>>,
}

impl MixedOccurrenceLedger {
    pub(super) fn prepare(width: MixedIngressQueueCount) -> Self {
        Self {
            cells: vec![CellUse::Free(LiveOccurrenceGeneration::FIRST); width.as_usize()],
        }
    }

    /// The lowest free cell's next handle, without taking it.
    pub(super) fn vacant(&self) -> Option<LiveOccurrenceId> {
        self.cells
            .iter()
            .enumerate()
            .find_map(|(index, cell)| match cell {
                CellUse::Free(generation) => Some(LiveOccurrenceId {
                    cell: LiveOccurrenceCell(u32::try_from(index).ok()?),
                    generation: *generation,
                }),
                CellUse::Held(..) | CellUse::Exhausted => None,
            })
    }

    fn cell_mut(&mut self, occurrence: LiveOccurrenceId) -> Option<&mut CellUse<LedgerHold>> {
        self.cells.get_mut(occurrence.cell.index()?)
    }

    fn cell(&self, occurrence: LiveOccurrenceId) -> Option<CellUse<LedgerHold>> {
        self.cells.get(occurrence.cell.index()?).copied()
    }

    /// Take a handle `vacant` returned once its onset holds command custody.
    pub(super) fn open(&mut self, occurrence: LiveOccurrenceId) -> bool {
        match self.cell_mut(occurrence) {
            Some(cell) if *cell == CellUse::Free(occurrence.generation) => {
                *cell = CellUse::Held(occurrence.generation, LedgerHold::Open);
                true
            }
            Some(_) | None => false,
        }
    }

    pub(super) fn is_open(&self, occurrence: LiveOccurrenceId) -> bool {
        self.cell(occurrence) == Some(CellUse::Held(occurrence.generation, LedgerHold::Open))
    }

    /// Mark an open handle's release as submitted under `command`.
    pub(super) fn begin_release(
        &mut self,
        occurrence: LiveOccurrenceId,
        command: MixedIngressCommandId,
    ) -> bool {
        match self.cell_mut(occurrence) {
            Some(cell) if *cell == CellUse::Held(occurrence.generation, LedgerHold::Open) => {
                *cell = CellUse::Held(occurrence.generation, LedgerHold::Releasing(command));
                true
            }
            Some(_) | None => false,
        }
    }

    /// Settle the release command `command` for `occurrence` against the collected
    /// outcome. Returns `true` when the release's reserved place returns to the handle.
    /// A result that names no releasing command of this ledger changes nothing.
    pub(super) fn settle_release(
        &mut self,
        command: MixedIngressCommandId,
        occurrence: LiveOccurrenceId,
        retired: Option<MixedRetired>,
    ) -> bool {
        let Some(cell) = self.cell_mut(occurrence) else {
            return false;
        };
        if *cell != CellUse::Held(occurrence.generation, LedgerHold::Releasing(command)) {
            return false;
        }
        match retired {
            Some(retired) if retired.occurrence == occurrence => {
                *cell = CellUse::retired(occurrence.generation);
                false
            }
            // A refused release returns custody, with its reserved place, to the merger.
            Some(_) | None => {
                *cell = CellUse::Held(occurrence.generation, LedgerHold::Open);
                true
            }
        }
    }

    /// Handles the merger has opened and not yet seen retired.
    pub(super) fn held(&self) -> usize {
        self.cells
            .iter()
            .filter(|cell| matches!(cell, CellUse::Held(..)))
            .count()
    }
}

/// The audio half's fixed occurrence table: the authority on what each handle names.
#[derive(Debug)]
pub(super) struct MixedOccurrenceTable {
    cells: Vec<CellUse<MixedOccurrenceState>>,
    released_after_refusal: EventCount,
}

impl MixedOccurrenceTable {
    /// Allocated off-thread at arm, with the merger ledger's width.
    pub(super) fn prepare(width: MixedIngressQueueCount) -> Self {
        Self {
            cells: vec![CellUse::Free(LiveOccurrenceGeneration::FIRST); width.as_usize()],
            released_after_refusal: EventCount::NONE,
        }
    }

    /// Every occupied cell, in cell order, for an off-thread teardown.
    pub(super) fn occupied(&self) -> Vec<(LiveOccurrenceId, MixedOccurrenceState)> {
        self.cells
            .iter()
            .enumerate()
            .filter_map(|(index, cell)| match cell {
                CellUse::Held(generation, state) => Some((
                    LiveOccurrenceId {
                        cell: LiveOccurrenceCell(u32::try_from(index).ok()?),
                        generation: *generation,
                    },
                    *state,
                )),
                CellUse::Free(_) | CellUse::Exhausted => None,
            })
            .collect()
    }

    /// Releases consumed by a refused onset's tombstone.
    pub(super) const fn released_after_refusal(&self) -> EventCount {
        self.released_after_refusal
    }
}

#[cfg(test)]
mod tests;
