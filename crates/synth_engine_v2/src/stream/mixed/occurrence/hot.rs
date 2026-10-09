//! The private mixed audio half's live command service.
//!
//! Each note command names a live-occurrence handle. This path resolves it in the table
//! prepared at arm, offers the edge to the registered ingress store, records an onset's
//! acceptance or refusal in the handle's cell, and retires the cell when its release
//! ends the occurrence or is consumed by the onset's tombstone. A key-up while its
//! scope's pedal is down offers nothing and keeps the hold; the scope's lift ends every
//! sustained member as one release group. It works in prepared storage only: it does
//! not allocate, lock, perform I/O, log or panic.

use ringbuf::traits::{Consumer, Producer};

use super::{
    super::{MixedIngressOutcome, MixedIngressRequest, MixedIngressResult, MixedOneShotAudio},
    CellUse, LiveOccurrenceCell, LiveOccurrenceGeneration, LiveOccurrenceId, MixedHandleRefused,
    MixedLiftResult, MixedLifted, MixedLiveScope, MixedOccurrenceState, MixedOccurrenceTable,
    MixedPedalDown, MixedPedalRefused, MixedRetireCause, MixedRetired, Occupant,
};
use crate::{
    identity::NoteIdentity,
    ingress::{IngressRefused, RELEASE_GROUP_CAPACITY, ReleaseCause},
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
    ) -> Result<(usize, CellUse<Occupant>), MixedHandleRefused> {
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
    ) -> Result<(usize, Occupant), MixedHandleRefused> {
        match self.resolve(occurrence)? {
            (index, CellUse::Held(_, occupant)) => Ok((index, occupant)),
            (_, CellUse::Free(_)) => Err(MixedHandleRefused::NotOpen),
            (_, CellUse::Exhausted) => Err(MixedHandleRefused::Exhausted),
        }
    }

    /// Record an onset's first disposition, or a sounding occurrence's key-up under the
    /// pedal. A refusal is never overwritten before retirement.
    fn occupy(&mut self, index: usize, generation: LiveOccurrenceGeneration, occupant: Occupant) {
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = CellUse::Held(generation, occupant);
        }
    }

    fn pedal_down(&self, scope: MixedLiveScope) -> bool {
        self.pedals.contains(&Some(scope))
    }

    /// Put the scope's pedal down in a free prepared cell.
    fn press(&mut self, scope: MixedLiveScope) -> Result<MixedPedalDown, MixedPedalRefused> {
        if self.pedal_down(scope) {
            return Ok(MixedPedalDown::AlreadyDown);
        }
        for pedal in &mut self.pedals {
            if pedal.is_none() {
                *pedal = Some(scope);
                return Ok(MixedPedalDown::Pressed);
            }
        }
        Err(MixedPedalRefused::PedalTableFull)
    }

    fn unpress(&mut self, scope: MixedLiveScope) {
        for pedal in &mut self.pedals {
            if *pedal == Some(scope) {
                *pedal = None;
            }
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
                    scope,
                    at,
                    key,
                    velocity,
                    ..
                } => self.service_onset(occurrence, scope, at, key, velocity),
                MixedIngressRequest::Release { occurrence, at, .. } => {
                    self.service_release(occurrence, at)
                }
                MixedIngressRequest::Pedal {
                    scope, down: true, ..
                } => MixedIngressOutcome::PedalDown(self.occurrences.press(scope)),
                MixedIngressRequest::Pedal {
                    scope,
                    at,
                    down: false,
                    ..
                } => MixedIngressOutcome::Lift(self.service_lift(scope, at)),
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
        scope: MixedLiveScope,
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
        self.occurrences
            .occupy(index, occurrence.generation, Occupant { scope, state });
        MixedIngressOutcome::Onset(outcome)
    }

    /// A release ends its sounding occurrence or is consumed by the onset's tombstone,
    /// then retires the cell. Under its scope's pedal a sounding occurrence instead
    /// becomes sustained: nothing is offered, the hold stays outstanding and the cell
    /// stays held for the lift. A refused offer leaves the occurrence sounding and held.
    fn service_release(
        &mut self,
        occurrence: LiveOccurrenceId,
        at: SampleTime,
    ) -> MixedIngressOutcome {
        let (index, occupant) = match self.occurrences.held_for_release(occurrence) {
            Ok(held) => held,
            Err(refused) => return MixedIngressOutcome::Handle(refused),
        };
        let cause = match occupant.state {
            MixedOccurrenceState::Sounding(identity) => {
                if self.occurrences.pedal_down(occupant.scope) {
                    let state = MixedOccurrenceState::Sustained(identity);
                    self.occurrences.occupy(
                        index,
                        occurrence.generation,
                        Occupant { state, ..occupant },
                    );
                    return MixedIngressOutcome::Sustained(occurrence);
                }
                if let Err(reason) = self.offer_test_note_off(at, identity) {
                    return MixedIngressOutcome::Release(Err(reason));
                }
                MixedRetireCause::Released(identity)
            }
            MixedOccurrenceState::Sustained(_) => {
                return MixedIngressOutcome::Handle(MixedHandleRefused::Sustained);
            }
            MixedOccurrenceState::Refused(reason) => {
                self.occurrences.count_release_after_refusal();
                MixedRetireCause::AfterRefusal(reason)
            }
        };
        self.occurrences.retire(index, occurrence.generation);
        MixedIngressOutcome::Release(Ok(MixedRetired { occurrence, cause }))
    }

    /// A lift ends every sustained member of its scope as one release group, which
    /// spends their holds and needs no further queue place. A key still down stays
    /// sounding. A refused group changes nothing: the pedal stays down and every member
    /// stays sustained. A lift of a scope whose pedal is up changes nothing.
    fn service_lift(&mut self, scope: MixedLiveScope, at: SampleTime) -> MixedLiftResult {
        if !self.occurrences.pedal_down(scope) {
            return Ok(MixedLifted::NONE);
        }
        let mut members = [None; RELEASE_GROUP_CAPACITY];
        let mut count = 0;
        for (index, cell) in self.occurrences.cells.iter().enumerate() {
            if let CellUse::Held(generation, occupant) = cell
                && occupant.scope == scope
                && let MixedOccurrenceState::Sustained(identity) = occupant.state
            {
                let member = members
                    .get_mut(count)
                    .ok_or(MixedPedalRefused::GroupCapacity)?;
                let cell = u32::try_from(index).map_err(|_| MixedPedalRefused::GroupCapacity)?;
                let occurrence = LiveOccurrenceId {
                    cell: LiveOccurrenceCell(cell),
                    generation: *generation,
                };
                *member = Some((index, occurrence, identity));
                count += 1;
            }
        }
        let Some((_, _, first)) = members.first().copied().flatten() else {
            self.occurrences.unpress(scope);
            return Ok(MixedLifted::NONE);
        };
        let mut identities = [first; RELEASE_GROUP_CAPACITY];
        for (identity, member) in identities.iter_mut().zip(members.iter().flatten()) {
            *identity = member.2;
        }
        let identities = identities
            .get(..count)
            .ok_or(MixedPedalRefused::GroupCapacity)?;
        self.offer_test_release_group(at, identities)
            .map_err(MixedPedalRefused::Ingress)?;
        let mut lifted = MixedLifted::NONE;
        for (retired, member) in lifted.members.iter_mut().zip(members) {
            let Some((index, occurrence, _)) = member else {
                continue;
            };
            self.occurrences.retire(index, occurrence.generation);
            *retired = Some(occurrence);
        }
        self.occurrences.unpress(scope);
        Ok(lifted)
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

    /// The scope's sustained occurrences redeem their holds as one lift group.
    fn offer_test_release_group(
        &mut self,
        at: SampleTime,
        identities: &[NoteIdentity],
    ) -> Result<(), IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        if self.test_live.is_some() {
            return Err(IngressRefused::MixedTestConflict);
        }
        self.test_ingress.adopt(self.audio.renderer.epoch())?;
        self.test_ingress.offer_mixed_release_group(
            &mut self.audio.live_minter,
            at,
            identities,
            ReleaseCause::SustainLift,
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
