//! The private mixed audio half's live command service.
//!
//! Each command names a live-occurrence handle. This path resolves it in the table
//! prepared at arm, offers the edge to the registered ingress store, records an onset's
//! acceptance or refusal in the handle's cell, and retires the cell when its release
//! ends the occurrence or is consumed by the onset's tombstone. It works in prepared
//! storage only: it does not allocate, lock, perform I/O, log or panic.

use ringbuf::traits::{Consumer, Producer};

use super::{
    super::{MixedIngressOutcome, MixedIngressRequest, MixedIngressResult, MixedOneShotAudio},
    CellUse, LiveOccurrenceGeneration, LiveOccurrenceId, MixedHandleRefused, MixedOccurrenceState,
    MixedOccurrenceTable, MixedRetireCause, MixedRetired,
};
use crate::{
    identity::NoteIdentity,
    ingress::IngressRefused,
    quantities::{EventCount, KeyIdentity, NoteVelocity},
    time::SampleTime,
};

impl LiveOccurrenceGeneration {
    /// The generation after a retirement; `None` retires the cell for good.
    pub(crate) const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }
}

impl<S> CellUse<S> {
    /// The cell after the occupancy at `generation` retires. The merger ledger applies
    /// the same rule when it collects `Retired`, so both halves agree on the next handle.
    pub(crate) const fn retired(generation: LiveOccurrenceGeneration) -> Self {
        match generation.next() {
            Some(next) => Self::Free(next),
            None => Self::Exhausted,
        }
    }
}

impl MixedOccurrenceTable {
    /// The cell a handle names at its current generation, with that cell's use.
    fn resolve(
        &self,
        occurrence: LiveOccurrenceId,
    ) -> Result<(usize, CellUse<MixedOccurrenceState>), MixedHandleRefused> {
        let index = occurrence
            .cell
            .index()
            .ok_or(MixedHandleRefused::UnknownCell)?;
        let cell = self
            .cells
            .get(index)
            .copied()
            .ok_or(MixedHandleRefused::UnknownCell)?;
        let current = match cell {
            CellUse::Free(generation) | CellUse::Held(generation, _) => generation,
            CellUse::Exhausted => return Err(MixedHandleRefused::Exhausted),
        };
        let offered = occurrence.generation;
        if offered < current {
            return Err(MixedHandleRefused::Stale { offered, current });
        }
        if offered > current {
            return Err(MixedHandleRefused::Unissued { offered, current });
        }
        Ok((index, cell))
    }

    fn vacant_for_onset(&self, occurrence: LiveOccurrenceId) -> Result<usize, MixedHandleRefused> {
        match self.resolve(occurrence)? {
            (index, CellUse::Free(_)) => Ok(index),
            (_, CellUse::Held(..)) => Err(MixedHandleRefused::Occupied),
            (_, CellUse::Exhausted) => Err(MixedHandleRefused::Exhausted),
        }
    }

    fn held_for_release(
        &self,
        occurrence: LiveOccurrenceId,
    ) -> Result<(usize, MixedOccurrenceState), MixedHandleRefused> {
        match self.resolve(occurrence)? {
            (index, CellUse::Held(_, state)) => Ok((index, state)),
            (_, CellUse::Free(_)) => Err(MixedHandleRefused::NotOpen),
            (_, CellUse::Exhausted) => Err(MixedHandleRefused::Exhausted),
        }
    }

    /// Record an onset's first disposition; it is never overwritten before retirement.
    fn occupy(
        &mut self,
        index: usize,
        generation: LiveOccurrenceGeneration,
        state: MixedOccurrenceState,
    ) {
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = CellUse::Held(generation, state);
        }
    }

    /// Free the cell at the next generation, so the retired handle can never resolve.
    fn retire(&mut self, index: usize, generation: LiveOccurrenceGeneration) {
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = CellUse::retired(generation);
        }
    }

    fn count_release_after_refusal(&mut self) {
        self.released_after_refusal = self
            .released_after_refusal
            .checked_add(EventCount::measured(1))
            .unwrap_or(self.released_after_refusal);
    }
}

impl MixedOneShotAudio {
    /// Serve only the preallocated command prefix; each result retains its request.
    pub(crate) fn service_test_ingress_queue(&mut self) {
        if let Some(pending) = self.ingress_result_pending.take()
            && let Err(pending) = self.ingress_results.0.try_push(pending)
        {
            self.ingress_result_pending = Some(pending);
            return;
        }
        while let Some(command) = self.ingress_commands.0.try_pop() {
            let outcome = match command.request {
                MixedIngressRequest::Onset {
                    occurrence,
                    at,
                    key,
                    velocity,
                    ..
                } => self.service_onset(occurrence, at, key, velocity),
                MixedIngressRequest::Release { occurrence, at, .. } => {
                    self.service_release(occurrence, at)
                }
            };
            let result = MixedIngressResult {
                id: command.id,
                request: command.request,
                outcome,
            };
            if let Err(result) = self.ingress_results.0.try_push(result) {
                self.ingress_result_pending = Some(result);
                break;
            }
        }
    }

    /// An onset occupies its handle's cell with the identity or the first refusal.
    fn service_onset(
        &mut self,
        occurrence: LiveOccurrenceId,
        at: SampleTime,
        key: KeyIdentity,
        velocity: NoteVelocity,
    ) -> MixedIngressOutcome {
        let index = match self.occurrences.vacant_for_onset(occurrence) {
            Ok(index) => index,
            Err(refused) => return MixedIngressOutcome::Handle(refused),
        };
        let outcome = self.offer_test_note_on(at, key, velocity);
        let state = match outcome {
            Ok(identity) => MixedOccurrenceState::Sounding(identity),
            Err(reason) => MixedOccurrenceState::Refused(reason),
        };
        self.occurrences.occupy(index, occurrence.generation, state);
        MixedIngressOutcome::Onset(outcome)
    }

    /// A release ends its sounding occurrence or is consumed by the onset's tombstone,
    /// then retires the cell. A refused offer leaves the occurrence sounding and held.
    fn service_release(
        &mut self,
        occurrence: LiveOccurrenceId,
        at: SampleTime,
    ) -> MixedIngressOutcome {
        let (index, state) = match self.occurrences.held_for_release(occurrence) {
            Ok(held) => held,
            Err(refused) => return MixedIngressOutcome::Handle(refused),
        };
        let cause = match state {
            MixedOccurrenceState::Sounding(identity) => {
                if let Err(reason) = self.offer_test_note_off(at, identity) {
                    return MixedIngressOutcome::Release(Err(reason));
                }
                MixedRetireCause::Released(identity)
            }
            MixedOccurrenceState::Refused(reason) => {
                self.occurrences.count_release_after_refusal();
                MixedRetireCause::AfterRefusal(reason)
            }
        };
        self.occurrences.retire(index, occurrence.generation);
        MixedIngressOutcome::Release(Ok(MixedRetired { occurrence, cause }))
    }

    /// Private ingress acceptance; the returned identity is recorded in the handle's cell.
    pub(crate) fn offer_test_note_on(
        &mut self,
        at: SampleTime,
        key: KeyIdentity,
        velocity: NoteVelocity,
    ) -> Result<NoteIdentity, IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        if self.test_live.is_some() {
            return Err(IngressRefused::MixedTestConflict);
        }
        self.test_ingress.adopt(self.audio.renderer.epoch())?;
        self.test_ingress.offer_mixed_note_on(
            &mut self.audio.live_minter,
            at,
            self.audio.note,
            key,
            velocity,
        )
    }

    /// The exact accepted occurrence redeems its release hold.
    pub(crate) fn offer_test_note_off(
        &mut self,
        at: SampleTime,
        identity: NoteIdentity,
    ) -> Result<(), IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        if self.test_live.is_some() {
            return Err(IngressRefused::MixedTestConflict);
        }
        self.test_ingress.adopt(self.audio.renderer.epoch())?;
        self.test_ingress
            .offer_mixed_note_off(&mut self.audio.live_minter, at, identity)
    }
}
