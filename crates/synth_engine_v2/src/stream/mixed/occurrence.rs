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
//! Each open handle owns one command-ring place reserved for its release.
//!
//! Sustain is scoped to one source's one channel. The audio half owns the pedal state
//! and decides, when it serves a key-up, whether the pedal holds the occurrence: a
//! held occurrence becomes `Sustained`, keeps its release hold and its cell, and spends
//! the key-up's ring place. Each effective pedal-down therefore reserves one ring place
//! for its own lift, and the lift ends every sustained member of its scope as one
//! release group, retiring their cells in its result. A sustained cell owns a release
//! hold rather than a ring place, so both tables are as wide as the ring plus the live
//! hold entitlement. Panic and bend are not handled here yet.

use synth_core::MidiChannel;
use thiserror::Error;

use super::{MixedIngressCommandId, MixedIngressQueueCount};
use crate::{
    identity::NoteIdentity,
    ingress::{IngressRefused, RELEASE_GROUP_CAPACITY},
    quantities::EventCount,
};

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

/// One live input source as the merger orders it. Pedal state is kept per source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub(super) struct MixedLiveSource(u64);

impl MixedLiveSource {
    pub(super) const fn new(source: u64) -> Self {
        Self(source)
    }
}

/// What a sustain pedal reaches: one source's one channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub(super) struct MixedLiveScope {
    source: MixedLiveSource,
    channel: MidiChannel,
}

impl MixedLiveScope {
    pub(super) const fn new(source: MixedLiveSource, channel: MidiChannel) -> Self {
        Self { source, channel }
    }
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
    /// The key is up but its scope's pedal holds the identity and its release hold.
    /// Only that scope's lift ends it.
    Sustained(NoteIdentity),
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

/// The sustained handles one lift retired, in cell order, which is also the order of
/// their identities in the lift's release group. A lift whose scope held no sustained
/// occurrence retires none.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MixedLifted {
    members: [Option<LiveOccurrenceId>; RELEASE_GROUP_CAPACITY],
}

impl MixedLifted {
    pub(super) const NONE: Self = Self {
        members: [None; RELEASE_GROUP_CAPACITY],
    };

    pub(super) fn retired(&self) -> impl Iterator<Item = LiveOccurrenceId> + '_ {
        self.members.iter().flatten().copied()
    }
}

/// A lift's retirements, or a refusal that leaves the pedal down and every member
/// sustained.
pub(super) type MixedLiftResult = Result<MixedLifted, MixedPedalRefused>;

/// How the audio half applied a pedal-down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MixedPedalDown {
    /// The scope's pedal went down.
    Pressed,
    /// The pedal was already down because the scope's last lift was refused.
    AlreadyDown,
}

/// Why the audio half refused a pedal edge. The pedal and every member keep their state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(super) enum MixedPedalRefused {
    #[error("no prepared pedal cell is free for another scope")]
    PedalTableFull,
    #[error("more sustained occurrences than one release group carries")]
    GroupCapacity,
    #[error(transparent)]
    Ingress(IngressRefused),
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
    #[error("live occurrence key is already up under the sustain pedal")]
    Sustained,
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

/// Why the merger could not submit a pedal edge. The caller keeps its input and the
/// merger's pedal state is unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(super) enum MixedPedalSubmitError {
    #[error("no command-ring place for a pedal-down and its reserved lift")]
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
    /// Audio served the release under the pedal; the cell waits for a lift's `Retired`
    /// and owns no ring place.
    Sustained,
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

    /// Settle the release command `command` that audio served under the pedal.
    pub(super) fn sustain(
        &mut self,
        command: MixedIngressCommandId,
        occurrence: LiveOccurrenceId,
    ) -> bool {
        match self.cell_mut(occurrence) {
            Some(cell)
                if *cell
                    == CellUse::Held(occurrence.generation, LedgerHold::Releasing(command)) =>
            {
                *cell = CellUse::Held(occurrence.generation, LedgerHold::Sustained);
                true
            }
            Some(_) | None => false,
        }
    }

    /// Free a sustained cell a collected lift retired.
    pub(super) fn retire_lifted(&mut self, occurrence: LiveOccurrenceId) -> bool {
        match self.cell_mut(occurrence) {
            Some(cell) if *cell == CellUse::Held(occurrence.generation, LedgerHold::Sustained) => {
                *cell = CellUse::retired(occurrence.generation);
                true
            }
            Some(_) | None => false,
        }
    }

    pub(super) fn is_sustained(&self, occurrence: LiveOccurrenceId) -> bool {
        self.cell(occurrence) == Some(CellUse::Held(occurrence.generation, LedgerHold::Sustained))
    }

    /// Cells the ledger was prepared with.
    pub(super) fn width(&self) -> usize {
        self.cells.len()
    }

    /// Handles the merger has opened and not yet seen retired.
    pub(super) fn held(&self) -> usize {
        self.cells
            .iter()
            .filter(|cell| matches!(cell, CellUse::Held(..)))
            .count()
    }
}

/// The merger's view of one scope's pedal. A scope with neither latch is up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PedalLatch {
    /// The effective pedal-down `pressed_by`, or a refused lift that returned the
    /// reservation, holds one ring place for the scope's lift.
    Down { pressed_by: MixedIngressCommandId },
    /// The lift was submitted as this command in the reserved place.
    Lifting(MixedIngressCommandId),
}

/// What the merger does with one pedal edge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MixedPedalEdge {
    /// The edge changes the merger's pedal state and must be submitted.
    Effective,
    /// A down while down or an up while up: consumed and counted, no place spent.
    Redundant,
}

/// The off-thread merger's pedal state per scope. It decides which edges are effective
/// and accounts the ring place each effective pedal-down reserves for its lift. Audio
/// keeps its own pedal state; this one only orders and reserves.
#[derive(Debug)]
pub(super) struct MixedPedalLedger {
    cells: Vec<Option<(MixedLiveScope, PedalLatch)>>,
    reserved: usize,
    redundant: EventCount,
}

impl MixedPedalLedger {
    /// One cell per ring place: every latched scope owns a reserved place or an
    /// in-flight lift command.
    pub(super) fn prepare(width: MixedIngressQueueCount) -> Self {
        Self {
            cells: vec![None; width.as_usize()],
            reserved: 0,
            redundant: EventCount::NONE,
        }
    }

    fn latch(&self, scope: MixedLiveScope) -> Option<PedalLatch> {
        self.cells
            .iter()
            .flatten()
            .find_map(|(held, latch)| (*held == scope).then_some(*latch))
    }

    fn latch_mut(&mut self, scope: MixedLiveScope) -> Option<&mut PedalLatch> {
        self.cells
            .iter_mut()
            .flatten()
            .find_map(|(held, latch)| (*held == scope).then_some(latch))
    }

    /// Classify an edge against the merger's pedal state, counting a redundant one.
    pub(super) fn classify(&mut self, scope: MixedLiveScope, down: bool) -> MixedPedalEdge {
        let effective = match self.latch(scope) {
            Some(PedalLatch::Down { .. }) => !down,
            Some(PedalLatch::Lifting(_)) | None => down,
        };
        if effective {
            MixedPedalEdge::Effective
        } else {
            self.redundant = self
                .redundant
                .checked_add(EventCount::measured(1))
                .unwrap_or(self.redundant);
            MixedPedalEdge::Redundant
        }
    }

    /// Whether an effective down can latch: its scope already has a cell, or one is free.
    pub(super) fn can_press(&self, scope: MixedLiveScope) -> bool {
        self.latch(scope).is_some() || self.cells.iter().any(Option::is_none)
    }

    /// Latch an effective pedal-down submitted as `command` and reserve its lift's place.
    pub(super) fn press(&mut self, scope: MixedLiveScope, command: MixedIngressCommandId) {
        let latch = PedalLatch::Down {
            pressed_by: command,
        };
        if let Some(held) = self.latch_mut(scope) {
            *held = latch;
        } else if let Some(cell) = self.cells.iter_mut().find(|cell| cell.is_none()) {
            *cell = Some((scope, latch));
        } else {
            debug_assert!(false, "can_press admitted the pedal-down");
            return;
        }
        self.reserved += 1;
    }

    /// Release one lift reservation so the lift itself can take the place.
    pub(super) fn take_reservation(&mut self) {
        self.reserved -= 1;
    }

    /// Undo [`Self::take_reservation`] when the lift could not be pushed.
    pub(super) fn return_reservation(&mut self) {
        self.reserved += 1;
    }

    /// Record the lift pushed as `command` into the scope's former reservation.
    pub(super) fn lift(&mut self, scope: MixedLiveScope, command: MixedIngressCommandId) {
        if let Some(latch) = self.latch_mut(scope) {
            *latch = PedalLatch::Lifting(command);
        }
    }

    /// The pedal-down command that latched the scope's current reservation.
    fn pressed_by(&self, scope: MixedLiveScope) -> Option<MixedIngressCommandId> {
        match self.latch(scope)? {
            PedalLatch::Down { pressed_by } => Some(pressed_by),
            PedalLatch::Lifting(_) => None,
        }
    }

    /// Settle a collected pedal-down. Audio refuses a merger's down only when its pedal
    /// table is full, which the one-cell-per-place width excludes; if it happens while
    /// this down is still the scope's latch, the reservation is returned and the scope
    /// is up again, as it is on audio.
    pub(super) fn settle_down(
        &mut self,
        command: MixedIngressCommandId,
        scope: MixedLiveScope,
        refused: bool,
    ) {
        if refused && self.pressed_by(scope) == Some(command) {
            self.clear(scope);
            self.reserved -= 1;
        }
    }

    /// Settle a collected lift. An applied lift unlatches its scope. A refused lift
    /// leaves the pedal down on audio: if it is still the scope's latest edge, its place
    /// becomes the scope's lift reservation again; otherwise a newer pedal-down already
    /// reserves for the scope and the place returns to the ring.
    pub(super) fn settle_lift(
        &mut self,
        command: MixedIngressCommandId,
        scope: MixedLiveScope,
        refused: bool,
    ) {
        if self.latch(scope) != Some(PedalLatch::Lifting(command)) {
            return;
        }
        if refused {
            if let Some(latch) = self.latch_mut(scope) {
                *latch = PedalLatch::Down {
                    pressed_by: command,
                };
            }
            self.reserved += 1;
        } else {
            self.clear(scope);
        }
    }

    fn clear(&mut self, scope: MixedLiveScope) {
        if let Some(cell) = self
            .cells
            .iter_mut()
            .find(|cell| cell.is_some_and(|(held, _)| held == scope))
        {
            *cell = None;
        }
    }

    /// Ring places reserved for lifts.
    pub(super) const fn reserved(&self) -> usize {
        self.reserved
    }

    /// Redundant pedal edges consumed at the merger.
    pub(super) const fn redundant(&self) -> EventCount {
        self.redundant
    }

    /// Whether the merger considers the scope's pedal down.
    pub(super) fn is_down(&self, scope: MixedLiveScope) -> bool {
        matches!(self.latch(scope), Some(PedalLatch::Down { .. }))
    }
}

/// One occupied audio cell: the occurrence's scope and what it names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Occupant {
    scope: MixedLiveScope,
    state: MixedOccurrenceState,
}

/// The audio half's fixed occurrence table: the authority on what each handle names,
/// and on which scopes' pedals are down.
#[derive(Debug)]
pub(super) struct MixedOccurrenceTable {
    cells: Vec<CellUse<Occupant>>,
    pedals: Vec<Option<MixedLiveScope>>,
    released_after_refusal: EventCount,
}

impl MixedOccurrenceTable {
    /// Allocated off-thread at arm, with the merger ledgers' widths.
    pub(super) fn prepare(width: MixedIngressQueueCount, pedals: MixedIngressQueueCount) -> Self {
        Self {
            cells: vec![CellUse::Free(LiveOccurrenceGeneration::FIRST); width.as_usize()],
            pedals: vec![None; pedals.as_usize()],
            released_after_refusal: EventCount::NONE,
        }
    }

    /// Scopes whose pedal is down, in cell order, for an off-thread teardown.
    pub(super) fn pedals_down(&self) -> Vec<MixedLiveScope> {
        self.pedals.iter().flatten().copied().collect()
    }

    /// Every occupied cell, in cell order, for an off-thread teardown.
    pub(super) fn occupied(&self) -> Vec<(LiveOccurrenceId, MixedOccurrenceState)> {
        self.cells
            .iter()
            .enumerate()
            .filter_map(|(index, cell)| match cell {
                CellUse::Held(generation, occupant) => Some((
                    LiveOccurrenceId {
                        cell: LiveOccurrenceCell(u32::try_from(index).ok()?),
                        generation: *generation,
                    },
                    occupant.state,
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
