//! The private mixed audio half's live command service.
//!
//! Each note command names a live-occurrence handle. This path resolves it in the table
//! prepared at arm, offers the edge to the registered ingress store, records an onset's
//! acceptance or refusal in the handle's cell, and retires the cell when its release
//! ends the occurrence or is consumed by the onset's tombstone. A key-up while its
//! scope's pedal is down offers nothing and keeps the hold; the scope's lift ends every
//! sustained member as one release group. Panic ends every sounding and sustained
//! occurrence as one release group, leaves a tombstone for each key still down and owes
//! the quantum's compiled loop one cut at the same sample; one stamped in a later quantum
//! waits parked, with nothing applied, and holds back every command behind it. A
//! bend appends one entry per sounding and sustained member of its scope at its
//! nominal stamp, and an onset inherits its scope's non-zero bend whole. Each command
//! spends at most the bend credits it carries; the rest return to the merger at once
//! through the lock-free credit counter. It works in prepared storage only: it does not
//! allocate, lock, perform I/O, log or panic.

use std::sync::atomic::Ordering;

#[cfg(all(test, feature = "simulated-ingress"))]
use super::super::EmbeddedQueues;
use super::{
    super::{
        MixedAudioQueues, MixedIngressCommand, MixedIngressCommandFault, MixedIngressOutcome,
        MixedIngressQueueCount, MixedIngressRequest, MixedIngressResult, MixedOneShotAudio,
    },
    CellUse, LiveOccurrenceCell, LiveOccurrenceGeneration, LiveOccurrenceId, MixedBendRefused,
    MixedBendResult, MixedBent, MixedHandleRefused, MixedLiftResult, MixedLifted, MixedLiveScope,
    MixedOccurrenceState, MixedOccurrenceTable, MixedPanicEnd, MixedPanicRefused, MixedPanicResult,
    MixedPanicked, MixedPedalDown, MixedPedalRefused, MixedRetireCause, MixedRetired, Occupant,
};
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use super::{MixedResetEnd, MixedResetEnded, MixedResetLiveRefused};
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use crate::{identity::LiveRangeMinter, ingress::PerformanceIngress, time::StreamEpoch};
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

    /// A whole-owner reset ends every sounding and sustained live occurrence as one
    /// `Reset` release group stamped `ended_at`, the end of the reset's fade (draft rule
    /// 3.3). It is offered to the owner's `ingress` at the reset boundary `at`, before
    /// the fading quantum opens, so a refusal leaves every occurrence, hold and entry as
    /// it was and the reset is refused rather than started. The members must account for
    /// every outstanding hold, so the group ends every live obligation the store holds.
    /// A sustained cell retires; a key-down cell becomes a `Reset` tombstone. Refusal and
    /// Panic tombstones and pedals are untouched. Arm bounds the live holds, and so the
    /// members, by one group. The caller has already refused a faulted owner.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) fn end_live_for_reset(
        &mut self,
        ingress: &mut PerformanceIngress,
        minter: &mut LiveRangeMinter,
        epoch: StreamEpoch,
        at: SampleTime,
        ended_at: SampleTime,
    ) -> Result<MixedResetEnded, MixedResetLiveRefused> {
        let mut members = [None; RELEASE_GROUP_CAPACITY];
        let mut count = 0;
        for (index, cell) in self.cells.iter().enumerate() {
            let CellUse::Held(generation, occupant) = cell else {
                continue;
            };
            let identity = match occupant.state {
                MixedOccurrenceState::Sounding(identity)
                | MixedOccurrenceState::Sustained(identity) => identity,
                MixedOccurrenceState::Refused(_)
                | MixedOccurrenceState::Panicked(_)
                | MixedOccurrenceState::Reset(_) => continue,
            };
            let member = members
                .get_mut(count)
                .ok_or(MixedResetLiveRefused::GroupCapacity)?;
            let cell = u32::try_from(index).map_err(|_| MixedResetLiveRefused::GroupCapacity)?;
            let occurrence = LiveOccurrenceId {
                cell: LiveOccurrenceCell(cell),
                generation: *generation,
            };
            *member = Some((index, occurrence, *occupant, identity));
            count += 1;
        }
        let holds = ingress.holds_outstanding();
        if holds.as_usize() != Some(count) {
            return Err(MixedResetLiveRefused::Unaccounted {
                members: count,
                holds,
            });
        }
        // The horizon is measured from the reset boundary, as this boundary's command
        // service would have measured it.
        ingress.set_mixed_admission_clock(at);
        if let Some((_, _, _, first)) = members.first().copied().flatten() {
            let mut identities = [first; RELEASE_GROUP_CAPACITY];
            for (identity, member) in identities.iter_mut().zip(members.iter().flatten()) {
                *identity = member.3;
            }
            let identities = identities
                .get(..count)
                .ok_or(MixedResetLiveRefused::GroupCapacity)?;
            if let Err(refused) = ingress.adopt(epoch) {
                return Err(MixedResetLiveRefused::Ingress(refused));
            }
            ingress
                .offer_mixed_release_group(minter, ended_at, identities, ReleaseCause::Reset)
                .map_err(MixedResetLiveRefused::Ingress)?;
        }
        let mut ended = MixedResetEnded::NONE;
        for (slot, member) in ended.ended.iter_mut().zip(members) {
            let Some((index, occurrence, occupant, identity)) = member else {
                continue;
            };
            if let MixedOccurrenceState::Sustained(_) = occupant.state {
                self.retire(index, occurrence.generation);
                *slot = Some(MixedResetEnd::Retired(occurrence));
            } else {
                let state = MixedOccurrenceState::Reset(identity);
                self.occupy(index, occurrence.generation, Occupant { state, ..occupant });
                *slot = Some(MixedResetEnd::Tombstoned(occurrence));
            }
        }
        Ok(ended)
    }
}

impl MixedIngressRequest {
    /// The stamp the source observation named; serving never moves it.
    pub(crate) const fn nominal(self) -> SampleTime {
        match self {
            Self::Onset { at, .. }
            | Self::Release { at, .. }
            | Self::Pedal { at, .. }
            | Self::Panic { at, .. }
            | Self::Bend { at, .. } => at,
        }
    }
}

impl MixedOneShotAudio {
    /// Serve the private rehearsal's own command ring.
    #[cfg(all(test, feature = "simulated-ingress"))]
    pub(crate) fn service_test_ingress_queue(&mut self) {
        let clock = self.audio.renderer.clock();
        self.panic_current_service_at
            .store(clock.as_u64(), Ordering::Release);
        self.service_ingress_queue(&mut EmbeddedQueues);
    }

    /// Stop command service for good: keep the offending packet, if any, record why, and
    /// publish the transfer halt before any ledger, occurrence or entry changes.
    pub(crate) fn refuse_ingress_service(
        &mut self,
        fault: MixedIngressCommandFault,
        command: Option<MixedIngressCommand>,
    ) {
        if let Some(command) = command {
            self.invalid_ingress_command = Some(command);
        }
        if self.ingress_command_fault.is_none() {
            self.ingress_command_fault = Some(fault);
        }
        self.ingress_halt.store(true, Ordering::Release);
    }

    /// Why audio must refuse `command` before serving it, if it must. Owner and ID come
    /// first; a host-owned transfer then faults a stamp before the last served one, and
    /// one beyond the forward horizon, which the store counts here exactly once.
    fn command_fault(
        &mut self,
        command: &MixedIngressCommand,
        transfer_rules: bool,
    ) -> Option<MixedIngressCommandFault> {
        let expected = self
            .last_served_ingress_id
            .map_or(Some(1), |id| id.0.checked_add(1));
        let at = command.request.nominal();
        if command.owner != self.ingress_owner {
            Some(MixedIngressCommandFault::ForeignCommand)
        } else if expected != Some(command.id.0) {
            Some(MixedIngressCommandFault::Sequence)
        } else if !transfer_rules {
            None
        } else if self.last_served_ingress_at.is_some_and(|last| at < last) {
            Some(MixedIngressCommandFault::BackwardTime)
        } else if self.test_ingress.admit_horizon(at).is_err() {
            Some(MixedIngressCommandFault::BeyondHorizon)
        } else {
            None
        }
    }

    /// Whether a Panic stamped `at` is served at the boundary `clock` opens rather than
    /// parked: no other Panic's compiled cut is already owed in this quantum, and its
    /// execution stamp lies in this quantum or before it, or beyond the forward horizon,
    /// where the store refuses it at once with nothing applied, as before.
    fn panic_due(&self, at: SampleTime, clock: SampleTime) -> bool {
        let executed = self.test_ingress.mixed_execution_stamp(at);
        self.panic_cut.is_none()
            && (executed.quantum_index() <= clock.quantum_index()
                || !self.test_ingress.within_horizon(executed))
    }

    /// Serve only the preallocated command prefix; each result retains its request.
    ///
    /// A Panic whose execution stamp lies in a later quantum is parked, with nothing
    /// applied, and no later command is popped until the quantum that contains that
    /// stamp opens and serves it first. Every command behind it is stamped at or after
    /// it, so waiting moves no execution position, and the results keep command order.
    /// Its merger cell stays occupied meanwhile, so a second Panic is still refused at
    /// the caller.
    pub(crate) fn service_ingress_queue<Q: MixedAudioQueues>(&mut self, queues: &mut Q) {
        if self.fault.is_some()
            || self.invalid_ingress_command.is_some()
            || self.ingress_halt.load(Ordering::Acquire)
        {
            return;
        }
        if !queues.bound(self) {
            self.refuse_ingress_service(MixedIngressCommandFault::ForeignEndpoint, None);
            return;
        }
        let clock = self.audio.renderer.clock();
        self.test_ingress.set_mixed_admission_clock(clock);
        // A concurrent control thread may collect results and refill the ring while
        // this callback runs. Occupancy alone cannot bound a drain loop, so service
        // at most one prepared ring's worth of commands at this boundary. The
        // private ring, and the two-source bridge's host queues, have one additional
        // protected Panic place; the single-source public rehearsal carries no Panic.
        let mut budget = self.test_ingress.mixed_service_capacity().saturating_add(1);
        if let Some(parked) = &self.parked_panic {
            if !self.panic_due(parked.request.nominal(), clock) {
                return;
            }
            if let Some(command) = self.parked_panic.take() {
                budget = budget.saturating_sub(1);
                if !self.serve_command(queues, command) {
                    return;
                }
            }
        }
        for _ in 0..budget {
            if self.ingress_halt.load(Ordering::Acquire) {
                return;
            }
            let Some(command) = queues.pop_command(self) else {
                break;
            };
            if let Some(fault) = self.command_fault(&command, Q::TRANSFER_RULES) {
                self.refuse_ingress_service(fault, Some(command));
                return;
            }
            if let MixedIngressRequest::Panic { at, .. } = command.request
                && !self.panic_due(at, clock)
            {
                self.parked_panic = Some(command);
                return;
            }
            if !self.serve_command(queues, command) {
                return;
            }
        }
    }

    /// Serve one command whose identity, owner and stamp already passed, and push its
    /// result. Returns whether service may continue: a refused push halts it.
    fn serve_command<Q: MixedAudioQueues>(
        &mut self,
        queues: &mut Q,
        command: MixedIngressCommand,
    ) -> bool {
        // A parked Panic has passed the packet checks but has not been served. Record
        // its position in the command stream only when it actually applies.
        self.last_served_ingress_id = Some(command.id);
        self.last_served_ingress_at = Some(command.request.nominal());
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
        #[cfg(feature = "mixed-transfer-rehearsal")]
        self.observe_ingress_usage();
        // Unused credits return at once; each written one returns when its entry
        // drains.
        self.bend_credit_return.give_back(MixedIngressQueueCount(
            granted.as_usize().saturating_sub(spent.as_usize()),
        ));
        let result = MixedIngressResult {
            owner: command.owner,
            id: command.id,
            request: command.request,
            outcome,
        };
        if let Some(result) = queues.push_result(self, result) {
            self.ingress_result_pending = Some(result);
            self.ingress_halt.store(true, Ordering::Release);
            return false;
        }
        true
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
            self.offer_live_note_on_inheriting(at, key, velocity, inherited)
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
                if let Err(reason) = self.offer_live_note_off(at, identity) {
                    return MixedIngressOutcome::Release(Err(reason));
                }
                MixedRetireCause::Released(identity)
            }
            MixedOccurrenceState::Sustained(_) => {
                return MixedIngressOutcome::Handle(MixedHandleRefused::Sustained);
            }
            // Defensive: a reset owner serves no command, so only a defect reaches here.
            MixedOccurrenceState::Reset(_) => {
                return MixedIngressOutcome::Handle(MixedHandleRefused::Reset);
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
        self.offer_live_release_group(at, identities, ReleaseCause::SustainLift)
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
    /// consumes its key-up. Refusal and Panic tombstones and pedals are untouched. A
    /// refused group changes nothing: every occurrence keeps its state and hold. Arm
    /// bounds the live holds, and so the members, by one group.
    ///
    /// An applied Panic also owes this quantum's compiled loop one cut of the compiled
    /// producer's open occurrences at the same sample: its execution stamp, or this
    /// boundary for a late one, where the renderer's late clamp puts the live group.
    /// Service only reaches here in the quantum that contains that sample.
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
                MixedOccurrenceState::Refused(_)
                | MixedOccurrenceState::Panicked(_)
                | MixedOccurrenceState::Reset(_) => continue,
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
        let offered_at = self.test_ingress.mixed_execution_stamp(at);
        let executed_at = offered_at.max(self.audio.renderer.clock());
        let offered = match members.first().copied().flatten() {
            Some((_, _, _, first)) => {
                let mut identities = [first; RELEASE_GROUP_CAPACITY];
                for (identity, member) in identities.iter_mut().zip(members.iter().flatten()) {
                    *identity = member.3;
                }
                let identities = identities
                    .get(..count)
                    .ok_or(MixedPanicRefused::GroupCapacity)?;
                self.offer_live_release_group(offered_at, identities, ReleaseCause::Panic)
            }
            // An empty Panic still passes the owner's checks and queues nothing.
            None => self.offer_live_release_group(offered_at, &[], ReleaseCause::Panic),
        };
        offered.map_err(MixedPanicRefused::Ingress)?;
        self.panic_cut = Some(executed_at);
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
                MixedOccurrenceState::Refused(_)
                | MixedOccurrenceState::Panicked(_)
                | MixedOccurrenceState::Reset(_) => continue,
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
                self.offer_live_bend(executed_at, identities, cents)
            }
            // A bend with no member still sets the value and the order fence.
            None => self.offer_live_bend(executed_at, &[], cents),
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
    #[cfg(test)]
    pub(crate) fn offer_test_note_on(
        &mut self,
        at: SampleTime,
        key: KeyIdentity,
        velocity: NoteVelocity,
    ) -> Result<NoteIdentity, IngressRefused> {
        self.offer_live_note_on_inheriting(at, key, velocity, None)
    }

    #[cfg(feature = "mixed-transfer-rehearsal")]
    pub(crate) fn observe_ingress_usage(&mut self) {
        self.ingress_queue_peak = self.ingress_queue_peak.max(self.test_ingress.len());
        self.ingress_holds_peak = self
            .ingress_holds_peak
            .max(self.test_ingress.holds_outstanding());
    }

    /// Private ingress acceptance of an onset, with the scope's non-zero bend written
    /// after it; the returned identity is recorded in the handle's cell.
    pub(crate) fn offer_live_note_on_inheriting(
        &mut self,
        at: SampleTime,
        key: KeyIdentity,
        velocity: NoteVelocity,
        inherited: Option<Cents>,
    ) -> Result<NoteIdentity, IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        #[cfg(test)]
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
    fn offer_live_bend(
        &mut self,
        at: SampleTime,
        identities: &[NoteIdentity],
        cents: Cents,
    ) -> Result<(), IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        #[cfg(test)]
        if self.test_live.is_some() {
            return Err(IngressRefused::MixedTestConflict);
        }
        self.test_ingress.adopt(self.audio.renderer.epoch())?;
        self.test_ingress
            .offer_mixed_bend(&self.audio.live_minter, at, identities, cents)
    }

    /// A lift's or Panic's members redeem their holds as one release group.
    fn offer_live_release_group(
        &mut self,
        at: SampleTime,
        identities: &[NoteIdentity],
        cause: ReleaseCause,
    ) -> Result<(), IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        #[cfg(test)]
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
    pub(crate) fn offer_live_note_off(
        &mut self,
        at: SampleTime,
        identity: NoteIdentity,
    ) -> Result<(), IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        #[cfg(test)]
        if self.test_live.is_some() {
            return Err(IngressRefused::MixedTestConflict);
        }
        self.test_ingress.adopt(self.audio.renderer.epoch())?;
        self.test_ingress
            .offer_mixed_note_off(&mut self.audio.live_minter, at, identity)
    }
}
