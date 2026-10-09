//! The private mixed audio half's live command service.
//!
//! Each note command names a live-occurrence handle. This path resolves it in the table
//! prepared at arm, offers the edge to the registered ingress store, records an onset's
//! acceptance or refusal in the handle's cell, and retires the cell when its release
//! ends the occurrence or is consumed by the onset's tombstone. A key-up while its
//! scope's pedal is down offers nothing and keeps the hold; the scope's lift ends every
//! sustained member as one release group. Panic ends every sounding and sustained
//! occurrence as one release group and leaves a tombstone for each key still down. A
//! bend appends one entry per sounding and sustained member of its scope at its
//! nominal stamp, and an onset inherits its scope's non-zero bend whole. Each command
//! spends at most the bend credits it carries; the rest return to the merger at once
//! through the lock-free credit counter. It works in prepared storage only: it does not
//! allocate, lock, perform I/O, log or panic.

use ringbuf::traits::{Consumer, Producer};

use super::{
    super::{
        MixedIngressOutcome, MixedIngressQueueCount, MixedIngressRequest, MixedIngressResult,
        MixedOneShotAudio,
    },
    CellUse, LiveOccurrenceCell, LiveOccurrenceGeneration, LiveOccurrenceId, MixedBendRefused,
    MixedBendResult, MixedBent, MixedHandleRefused, MixedLiftResult, MixedLifted, MixedLiveScope,
    MixedOccurrenceState, MixedOccurrenceTable, MixedPanicEnd, MixedPanicRefused, MixedPanicResult,
    MixedPanicked, MixedPedalDown, MixedPedalRefused, MixedRetireCause, MixedRetired, Occupant,
};
use crate::{
    identity::NoteIdentity,
    ingress::{IngressRefused, RELEASE_GROUP_CAPACITY, ReleaseCause},
    quantities::{Cents, EventCount, KeyIdentity, NoteVelocity},
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

    /// Record an onset's first disposition, a sounding occurrence's key-up under the
    /// pedal, or Panic ending a key-down occurrence. A refusal or Panic tombstone is never
    /// overwritten before retirement.
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

    /// Whether the scope can take `cents`: a zero bend needs no cell, and a non-zero one
    /// needs the scope's cell or a free one.
    fn can_bend(&self, scope: MixedLiveScope, cents: Cents) -> bool {
        cents == Cents::ZERO
            || self
                .bends
                .iter()
                .any(|cell| cell.is_none_or(|(held, _)| held == scope))
    }

    /// The scope's bend: the value of the last bend audio applied to it.
    pub(crate) fn bend_of(&self, scope: MixedLiveScope) -> Cents {
        for (held, cents) in self.bends.iter().flatten() {
            if *held == scope {
                return *cents;
            }
        }
        Cents::ZERO
    }

    /// Record the scope's bend after `can_bend` admitted it; zero frees the cell.
    fn set_bend(&mut self, scope: MixedLiveScope, cents: Cents) {
        let value = (cents != Cents::ZERO).then_some((scope, cents));
        for cell in &mut self.bends {
            if cell.is_some_and(|(held, _)| held == scope) {
                *cell = value;
                return;
            }
        }
        if value.is_none() {
            return;
        }
        for cell in &mut self.bends {
            if cell.is_none() {
                *cell = value;
                return;
            }
        }
    }

    fn count_release_after_refusal(&mut self) {
        self.released_after_refusal = self
            .released_after_refusal
            .checked_add(EventCount::measured(1))
            .unwrap_or(self.released_after_refusal);
    }

    fn count_release_after_panic(&mut self) {
        self.released_after_panic = self
            .released_after_panic
            .checked_add(EventCount::measured(1))
            .unwrap_or(self.released_after_panic);
    }
}

impl MixedOneShotAudio {
    /// Serve only the preallocated command prefix; each result retains its request.
    pub(crate) fn service_test_ingress_queue(&mut self) {
        if self.fault.is_some() || self.invalid_ingress_command.is_some() {
            return;
        }
        if let Some(pending) = self.ingress_result_pending.take()
            && let Err(pending) = self.ingress_results.0.try_push(pending)
        {
            self.ingress_result_pending = Some(pending);
            return;
        }
        while let Some(command) = self.ingress_commands.0.try_pop() {
            let expected = self
                .last_served_ingress_id
                .map_or(Some(1), |id| id.0.checked_add(1));
            if expected != Some(command.id.0) {
                self.invalid_ingress_command = Some(command);
                return;
            }
            self.last_served_ingress_id = Some(command.id);
            let granted = command.bend_credits;
            let none = MixedIngressQueueCount::NONE;
            let (outcome, spent) = match command.request {
                MixedIngressRequest::Onset {
                    occurrence,
                    scope,
                    at,
                    key,
                    velocity,
                    ..
                } => self.service_onset(occurrence, scope, at, key, velocity, granted),
                MixedIngressRequest::Release { occurrence, at, .. } => {
                    (self.service_release(occurrence, at), none)
                }
                MixedIngressRequest::Pedal {
                    scope, down: true, ..
                } => (
                    MixedIngressOutcome::PedalDown(self.occurrences.press(scope)),
                    none,
                ),
                MixedIngressRequest::Pedal {
                    scope,
                    at,
                    down: false,
                    ..
                } => (
                    MixedIngressOutcome::Lift(self.service_lift(scope, at)),
                    none,
                ),
                MixedIngressRequest::Panic { at, .. } => {
                    (MixedIngressOutcome::Panic(self.service_panic(at)), none)
                }
                MixedIngressRequest::Bend {
                    scope, at, cents, ..
                } => {
                    let bent = self.service_bend(scope, at, cents, granted);
                    let spent = bent.as_ref().map_or(0, |bent| bent.written().count());
                    (
                        MixedIngressOutcome::Bend(bent),
                        MixedIngressQueueCount(spent),
                    )
                }
            };
            // Unused credits return at once; each written one returns when its entry
            // drains.
            self.bend_credit_return.give_back(MixedIngressQueueCount(
                granted.as_usize().saturating_sub(spent.as_usize()),
            ));
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

    /// An onset occupies its handle's cell with the identity or the first refusal. On a
    /// scope whose bend is not zero it is offered with that bend, charged to the credit
    /// its command carries, and refused whole when the two do not fit together. Returns
    /// the outcome and the credits it spent.
    fn service_onset(
        &mut self,
        occurrence: LiveOccurrenceId,
        scope: MixedLiveScope,
        at: SampleTime,
        key: KeyIdentity,
        velocity: NoteVelocity,
        granted: MixedIngressQueueCount,
    ) -> (MixedIngressOutcome, MixedIngressQueueCount) {
        let none = MixedIngressQueueCount::NONE;
        let index = match self.occurrences.vacant_for_onset(occurrence) {
            Ok(index) => index,
            Err(refused) => return (MixedIngressOutcome::Handle(refused), none),
        };
        let bend = self.occurrences.bend_of(scope);
        let inherited = (bend != Cents::ZERO).then_some(bend);
        // The merger reserves a credit whenever the scope may be bent, so only a forged
        // onset reaches a bent scope without one.
        let outcome = if inherited.is_some() && granted == none {
            Err(IngressRefused::MixedBendCredit)
        } else {
            self.offer_test_note_on_inheriting(at, key, velocity, inherited)
        };
        let spent = if outcome.is_ok() && inherited.is_some() {
            MixedIngressQueueCount(1)
        } else {
            none
        };
        let state = match outcome {
            Ok(identity) => MixedOccurrenceState::Sounding(identity),
            Err(reason) => MixedOccurrenceState::Refused(reason),
        };
        self.occurrences
            .occupy(index, occurrence.generation, Occupant { scope, state });
        (MixedIngressOutcome::Onset(outcome), spent)
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
            MixedOccurrenceState::Panicked(identity) => {
                self.occurrences.count_release_after_panic();
                MixedRetireCause::AfterPanic(identity)
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
        self.offer_test_release_group(at, identities, ReleaseCause::SustainLift)
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

    /// Panic ends every sounding and sustained occurrence of every scope as one release
    /// group. It is queued at its execution stamp, so the store's order fence never
    /// refuses it. A sustained cell retires; a key-down cell becomes a tombstone that
    /// consumes its key-up. Refusal and Panic tombstones, pedals and compiled occurrences
    /// are untouched. A refused group changes nothing: every occurrence keeps its state
    /// and hold. Arm bounds the live holds, and so the members, by one group.
    fn service_panic(&mut self, at: SampleTime) -> MixedPanicResult {
        let mut members = [None; RELEASE_GROUP_CAPACITY];
        let mut count = 0;
        for (index, cell) in self.occurrences.cells.iter().enumerate() {
            let CellUse::Held(generation, occupant) = cell else {
                continue;
            };
            let identity = match occupant.state {
                MixedOccurrenceState::Sounding(identity)
                | MixedOccurrenceState::Sustained(identity) => identity,
                MixedOccurrenceState::Refused(_) | MixedOccurrenceState::Panicked(_) => continue,
            };
            let member = members
                .get_mut(count)
                .ok_or(MixedPanicRefused::GroupCapacity)?;
            let cell = u32::try_from(index).map_err(|_| MixedPanicRefused::GroupCapacity)?;
            let occurrence = LiveOccurrenceId {
                cell: LiveOccurrenceCell(cell),
                generation: *generation,
            };
            *member = Some((index, occurrence, *occupant, identity));
            count += 1;
        }
        let executed_at = self.test_ingress.mixed_execution_stamp(at);
        let offered = match members.first().copied().flatten() {
            Some((_, _, _, first)) => {
                let mut identities = [first; RELEASE_GROUP_CAPACITY];
                for (identity, member) in identities.iter_mut().zip(members.iter().flatten()) {
                    *identity = member.3;
                }
                let identities = identities
                    .get(..count)
                    .ok_or(MixedPanicRefused::GroupCapacity)?;
                self.offer_test_release_group(executed_at, identities, ReleaseCause::Panic)
            }
            // An empty Panic still passes the owner's checks and queues nothing.
            None => self.offer_test_release_group(executed_at, &[], ReleaseCause::Panic),
        };
        offered.map_err(MixedPanicRefused::Ingress)?;
        let mut panicked = MixedPanicked {
            executed_at,
            ended: [None; RELEASE_GROUP_CAPACITY],
        };
        for (ended, member) in panicked.ended.iter_mut().zip(members) {
            let Some((index, occurrence, occupant, identity)) = member else {
                continue;
            };
            if let MixedOccurrenceState::Sustained(_) = occupant.state {
                self.occurrences.retire(index, occurrence.generation);
                *ended = Some(MixedPanicEnd::Retired(occurrence));
            } else {
                let state = MixedOccurrenceState::Panicked(identity);
                self.occurrences.occupy(
                    index,
                    occurrence.generation,
                    Occupant { state, ..occupant },
                );
                *ended = Some(MixedPanicEnd::Tombstoned(occurrence));
            }
        }
        Ok(panicked)
    }

    /// A bend appends one entry for every sounding and sustained occurrence of its scope,
    /// in cell order, then becomes the scope's value for later onsets. Refusal and Panic
    /// tombstones are not members. Each entry spends one of the credits the command
    /// carries, so no bend is refused for a queue slot. The offer keeps its nominal
    /// stamp; an earlier accepted event with a later stamp refuses it. A refused bend
    /// changes neither the value nor any entry.
    fn service_bend(
        &mut self,
        scope: MixedLiveScope,
        at: SampleTime,
        cents: Cents,
        granted: MixedIngressQueueCount,
    ) -> MixedBendResult {
        if !self.occurrences.can_bend(scope, cents) {
            return Err(MixedBendRefused::BendTableFull);
        }
        let mut members = [None; RELEASE_GROUP_CAPACITY];
        let mut count = 0;
        for (index, cell) in self.occurrences.cells.iter().enumerate() {
            let CellUse::Held(generation, occupant) = cell else {
                continue;
            };
            if occupant.scope != scope {
                continue;
            }
            let identity = match occupant.state {
                MixedOccurrenceState::Sounding(identity)
                | MixedOccurrenceState::Sustained(identity) => identity,
                MixedOccurrenceState::Refused(_) | MixedOccurrenceState::Panicked(_) => continue,
            };
            let member = members
                .get_mut(count)
                .ok_or(MixedBendRefused::GroupCapacity)?;
            let cell = u32::try_from(index).map_err(|_| MixedBendRefused::GroupCapacity)?;
            let occurrence = LiveOccurrenceId {
                cell: LiveOccurrenceCell(cell),
                generation: *generation,
            };
            *member = Some((occurrence, identity));
            count += 1;
        }
        if count > granted.as_usize() {
            return Err(MixedBendRefused::Credits {
                granted,
                members: count,
            });
        }
        let executed_at = at;
        let offered = match members.first().copied().flatten() {
            Some((_, first)) => {
                let mut identities = [first; RELEASE_GROUP_CAPACITY];
                for (identity, member) in identities.iter_mut().zip(members.iter().flatten()) {
                    *identity = member.1;
                }
                let identities = identities
                    .get(..count)
                    .ok_or(MixedBendRefused::GroupCapacity)?;
                self.offer_test_bend(executed_at, identities, cents)
            }
            // A bend with no member still sets the value and the order fence.
            None => self.offer_test_bend(executed_at, &[], cents),
        };
        offered.map_err(MixedBendRefused::Ingress)?;
        self.occurrences.set_bend(scope, cents);
        let mut bent = MixedBent {
            executed_at,
            members: [None; RELEASE_GROUP_CAPACITY],
        };
        for (written, member) in bent.members.iter_mut().zip(members) {
            *written = member.map(|(occurrence, _)| occurrence);
        }
        Ok(bent)
    }

    /// Private ingress acceptance; the returned identity is recorded in the handle's cell.
    pub(crate) fn offer_test_note_on(
        &mut self,
        at: SampleTime,
        key: KeyIdentity,
        velocity: NoteVelocity,
    ) -> Result<NoteIdentity, IngressRefused> {
        self.offer_test_note_on_inheriting(at, key, velocity, None)
    }

    /// [`Self::offer_test_note_on`] with the scope's non-zero bend written after it.
    pub(crate) fn offer_test_note_on_inheriting(
        &mut self,
        at: SampleTime,
        key: KeyIdentity,
        velocity: NoteVelocity,
        inherited: Option<Cents>,
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
            inherited,
        )
    }

    /// A bend's live members take their entries together, or none does.
    fn offer_test_bend(
        &mut self,
        at: SampleTime,
        identities: &[NoteIdentity],
        cents: Cents,
    ) -> Result<(), IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        if self.test_live.is_some() {
            return Err(IngressRefused::MixedTestConflict);
        }
        self.test_ingress.adopt(self.audio.renderer.epoch())?;
        self.test_ingress
            .offer_mixed_bend(&self.audio.live_minter, at, identities, cents)
    }

    /// A lift's or Panic's members redeem their holds as one release group.
    fn offer_test_release_group(
        &mut self,
        at: SampleTime,
        identities: &[NoteIdentity],
        cause: ReleaseCause,
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
            cause,
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
