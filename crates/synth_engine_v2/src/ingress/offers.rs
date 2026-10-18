//! Fixed-storage live admission; all preparation remains in the parent module.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use super::RELEASE_GROUP_CAPACITY;
use super::{ExhaustedResource, IngressEntry, IngressRefused, PendingStart, PerformanceIngress};
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use crate::{identity::LiveRangeMinter, quantities::Cents};
use crate::{
    identity::{IdentityError, IdentityTable, NoteIdentity, Resolution},
    plan::{NoteSlot, ParameterSlot},
    quantities::{EventCount, ParameterValue},
    render::{EventEnvelope, EventPayload, NoteEdge, TimedEvent},
    time::{SampleTime, StreamEpoch, TimeSource},
};
/// What wrote a test-only mixed bend entry.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MixedBendOrigin {
    /// An onset on a bent scope wrote its scope's value at its own stamp.
    Inherited,
    /// A channel bend wrote the entry at its nominal stamp.
    Ordinary,
}

/// The test-only accounting of the private mixed bend-credit budget.
#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MixedBendBudget {
    /// The budget `B` withheld from ordinary traffic.
    pub(crate) budget: usize,
    /// The store's count of undrained credit-charged entries.
    pub(crate) queued: usize,
    /// Undrained entries actually marked as credit-charged, counted by walking the ring.
    pub(crate) marked: usize,
}

impl PerformanceIngress {
    /// Test-only mixed offer through the owner's registered queue and live range.
    ///
    /// An accepted onset takes its note-on entry and its release hold from ordinary
    /// room. An onset whose scope's bend is not zero also writes that bend right after
    /// its note-on at its own stamp, charged to one bend credit the merger reserved for
    /// it. Every check precedes the mint, so the onset and its inherited bend are
    /// accepted or refused whole.
    ///
    /// The hold is checked before ordinary room. With `B = C - 2H`, ordinary room holds
    /// exactly every hold's note-on and release, so when every hold is out the room is
    /// short too and the hold is the binding cause.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) fn offer_mixed_note_on(
        &mut self,
        minter: &mut LiveRangeMinter,
        time: SampleTime,
        note: NoteSlot,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
        inherited: Option<Cents>,
    ) -> Result<NoteIdentity, IngressRefused> {
        self.admit(time)?;
        if self.holds_outstanding >= self.hold_entitlement {
            self.counters.dropped_hold = self.counters.dropped_hold.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Hold,
            });
        }
        if !self.room_for_entries(1, 1) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        if inherited.is_some() && !self.mixed_bend_credits_free(1) {
            return Err(IngressRefused::MixedBendCredit);
        }
        let identity = match minter.mint_keyed(note, key) {
            Ok(identity) => identity,
            Err(_) => {
                self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
                return Err(IngressRefused::Dropped {
                    resource: ExhaustedResource::Identity,
                });
            }
        };
        self.holds_outstanding = self
            .holds_outstanding
            .checked_add(EventCount::measured(1))
            .unwrap_or(self.holds_outstanding);
        self.enqueue_entry(
            time,
            EventPayload::Note {
                identity,
                edge: NoteEdge::On {
                    slot: note,
                    key,
                    velocity,
                },
            },
            false,
        );
        if let Some(cents) = inherited {
            self.enqueue_mixed_bend(time, identity, cents, MixedBendOrigin::Inherited);
        }
        Ok(identity)
    }

    /// An accepted release trades its reserved hold for a queue entry.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) fn offer_mixed_note_off(
        &mut self,
        minter: &mut LiveRangeMinter,
        time: SampleTime,
        identity: NoteIdentity,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;
        if self.holds_outstanding == EventCount::NONE
            || minter.release(identity) != Resolution::Live
        {
            self.counters.orphan_releases = self.counters.orphan_releases.saturating_add(1);
            return Err(IngressRefused::OrphanRelease { identity });
        }
        self.holds_outstanding =
            EventCount::measured(self.holds_outstanding.get().saturating_sub(1));
        self.enqueue_entry(
            time,
            EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            },
            true,
        );
        Ok(())
    }

    /// Test-only channel bend for the mixed owner's live members, all of one scope.
    ///
    /// Each member takes a new entry at the bend's stamp, in member order and after
    /// every earlier entry, charged to one bend credit the merger reserved before it
    /// minted the command. No earlier entry is rewritten, coalesced or moved, so every
    /// value applies at its own stamp. The budget invariant gives each entry a ring slot:
    /// `(len - queued) + holds + B <= C` with `queued + members <= B` leaves
    /// `len + members <= C - holds`.
    ///
    /// Every check precedes the first write, so a refusal changes nothing, and an
    /// accepted bend moves the order fence to its stamp even when it has no member: a
    /// later onset inheriting the value is never stamped before it.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) fn offer_mixed_bend(
        &mut self,
        minter: &LiveRangeMinter,
        time: SampleTime,
        members: &[NoteIdentity],
        cents: Cents,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;
        if members.len() > RELEASE_GROUP_CAPACITY {
            return Err(IngressRefused::MixedBendFanout);
        }
        for (index, identity) in members.iter().copied().enumerate() {
            if members
                .get(..index)
                .is_some_and(|seen| seen.contains(&identity))
            {
                return Err(IngressRefused::MixedBendFanout);
            }
            if minter.resolve(identity) != Resolution::Live {
                self.counters.orphan_expressions =
                    self.counters.orphan_expressions.saturating_add(1);
                return Err(IngressRefused::OrphanExpression { identity });
            }
        }
        if !self.mixed_bend_credits_free(members.len()) {
            return Err(IngressRefused::MixedBendCredit);
        }
        for identity in members.iter().copied() {
            self.enqueue_mixed_bend(time, identity, cents, MixedBendOrigin::Ordinary);
        }
        self.last_accepted = Some(match self.last_accepted {
            Some(last) if last > time => last,
            _ => time,
        });
        Ok(())
    }

    /// Whether `entries` more bend entries fit the bend-credit budget.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    fn mixed_bend_credits_free(&self, entries: usize) -> bool {
        self.mixed_bend_queued.saturating_add(entries) <= self.mixed_bend_budget
    }

    /// Append one credit-charged bend entry. The caller has checked the budget.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    fn enqueue_mixed_bend(
        &mut self,
        time: SampleTime,
        identity: NoteIdentity,
        cents: Cents,
        origin: MixedBendOrigin,
    ) {
        let slot = self.head;
        self.enqueue_entry(time, EventPayload::Bend { identity, cents }, false);
        if let Some(Some(entry)) = self.entries.get_mut(slot) {
            entry.bend_credit = true;
            entry.inherited_bend = origin == MixedBendOrigin::Inherited;
        }
        self.mixed_bend_queued = self.mixed_bend_queued.saturating_add(1);
    }

    /// The private mixed owner's bend-credit budget `B`.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) const fn mixed_bend_budget(&self) -> usize {
        self.mixed_bend_budget
    }

    /// Undrained entries charged to the bend-credit budget. The drain only lowers it.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) const fn mixed_bend_queued(&self) -> usize {
        self.mixed_bend_queued
    }

    /// The budget, its queued count and the marked entries, for an invariant check.
    #[cfg(all(test, feature = "simulated-ingress"))]
    pub(crate) fn mixed_bend_budget_for_test(&self) -> MixedBendBudget {
        let capacity = self.entries.len();
        let mut cursor = self.tail;
        let mut marked = 0;
        for _ in 0..self.len {
            if self
                .entries
                .get(cursor)
                .is_some_and(|entry| entry.is_some_and(|entry| entry.bend_credit))
            {
                marked += 1;
            }
            cursor = if cursor + 1 >= capacity {
                0
            } else {
                cursor + 1
            };
        }
        MixedBendBudget {
            budget: self.mixed_bend_budget,
            queued: self.mixed_bend_queued,
            marked,
        }
    }

    /// Copy every undrained bend entry in queue order into prepared storage, with its
    /// origin. `None` when the storage is too short.
    #[cfg(all(test, feature = "simulated-ingress"))]
    pub(crate) fn copy_mixed_bend_entries_for_test(
        &self,
        destination: &mut [Option<(TimedEvent, MixedBendOrigin)>],
    ) -> Option<usize> {
        let capacity = self.entries.len();
        let mut cursor = self.tail;
        let mut copied = 0;
        for _ in 0..self.len {
            if let Some(Some(entry)) = self.entries.get(cursor)
                && let EventPayload::Bend { .. } = entry.event.payload()
            {
                let origin = if entry.inherited_bend {
                    MixedBendOrigin::Inherited
                } else {
                    MixedBendOrigin::Ordinary
                };
                *destination.get_mut(copied)? = Some((entry.event, origin));
                copied += 1;
            }
            cursor = if cursor + 1 >= capacity {
                0
            } else {
                cursor + 1
            };
        }
        Some(copied)
    }

    /// Whether one new entry fits, together with `holds` further release reservations it
    /// would itself create.
    ///
    /// The invariant is `occupied + outstanding holds <= capacity`: every queued entry has
    /// a slot, and every hold has a slot kept for the release that will redeem it. A
    /// **note-on therefore needs two units** — its own entry and the reservation for the
    /// release it does not yet know about — which is why the count is a parameter rather
    /// than a constant. Asking for one unit for a note-on is the off-by-one that lets the
    /// last note-on into a queue with no room for its own release, and the promise this
    /// reservation exists to keep is exactly that release.
    ///
    /// A release asks nothing: it converts a reservation into an entry, so the sum does not
    /// move and the invariant carries it.
    fn room_for(&self, holds: usize) -> bool {
        self.room_for_entries(1, holds)
    }

    /// Room for `entries` more ring entries, with `holds` more reservations.
    ///
    /// A deferred start counts as the two live-class entries it will publish — a reset and a
    /// note-on; its release redeems the hold as any release does — so what waits outside the
    /// ring is charged as if it were inside it, and the density the ring's capacity bounds
    /// is unchanged by stealing.
    fn room_for_entries(&self, entries: usize, holds: usize) -> bool {
        let unstarted = self
            .pending
            .iter()
            .filter(|slot| slot.is_some_and(|record| !record.started))
            .count();
        let waiting_bends = self
            .pending
            .iter()
            .filter(|slot| slot.is_some_and(|record| record.bend.is_some()))
            .count();
        // Credit-charged mixed bend entries occupy the withheld budget, not ordinary room.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let ordinary = self.len.saturating_sub(self.mixed_bend_queued);
        #[cfg(not(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        )))]
        let ordinary = self.len;
        let needed = ordinary
            .saturating_add(unstarted.saturating_mul(2))
            .saturating_add(waiting_bends)
            .saturating_add(self.expression_len)
            .saturating_add(self.holds_outstanding.get() as usize)
            .saturating_add(entries)
            .saturating_add(holds);
        // The private mixed bend-credit budget is withheld from ordinary traffic.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let needed = needed.saturating_add(self.mixed_bend_budget);
        needed <= self.entries.len()
    }

    /// Offer a parameter write.
    ///
    /// It takes a slot and nothing else: a control write opens no obligation, so ADR-0046
    /// clause 6's hold does not reach it.
    ///
    /// **Crate-private for the reason the note offers are**, although it needs no minter: the
    /// stream latches one store, and an offer that bypassed the latch would let a second
    /// store fill a queue the stream never adopted — and overwrite the first store's
    /// cumulative counters in the report, since the drain mirrors rather than accumulates.
    /// An independent review found the gap.
    pub(crate) fn offer_parameter(
        &mut self,
        time: SampleTime,
        slot: ParameterSlot,
        value: ParameterValue,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;
        if !self.room_for(0) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        self.enqueue_entry(time, EventPayload::SetParameter { slot, value }, false);
        Ok(())
    }

    /// Offer a validated controller replacement under the existing live queue's entitlement.
    pub(crate) fn offer_controller(
        &mut self,
        time: SampleTime,
        change: crate::controller::ControllerChange,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;
        if !self.room_for_entries(1, 0) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        self.enqueue_entry(time, EventPayload::Controller(change), false);
        Ok(())
    }

    /// Offer a note-on, acquiring its slot, hold and identity together.
    ///
    /// Returns the occurrence, because the caller needs it to release the note: the
    /// producer, not this store, decides which of its open notes an incoming note-off
    /// belongs to.
    ///
    /// **Crate-private, and reached through `StreamControl`.** The render contract's *stream
    /// has two owners* rule puts the identity **minter** on the off-thread half, which is
    /// `StreamControl`; a public method taking any `IdentityTable` would let a caller mint
    /// from a table the renderer never heard of, and the renderer would then refuse every
    /// edge as foreign. Routing the offer through the owner makes that unrepresentable
    /// rather than merely wrong.
    pub(crate) fn offer_note_on(
        &mut self,
        table: &mut IdentityTable,
        time: SampleTime,
        note: NoteSlot,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Result<NoteIdentity, IngressRefused> {
        self.admit(time)?;
        self.sweep(table, time);

        // The two cheap resources first, and both **before** the mint. A mint that
        // succeeded into a full queue would have to be undone, and undoing it is what
        // reissues an index whose generation has already advanced.
        if !self.room_for(1) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        if self.holds_outstanding >= self.hold_entitlement {
            // ADR-0058 at the boundary, where the holds run out with the voices: a producer
            // declaring as many holds as notes holds every reservation when every voice is
            // held. The taken note's release will not be queued — it is counted at this
            // boundary — so its hold goes with its voice to the note that takes it, and no
            // reservation is spent.
            if self.stealing.steals() && table.is_full(self.producer) {
                return self.steal(table, time, note, key, velocity, true);
            }
            self.counters.dropped_hold = self.counters.dropped_hold.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Hold,
            });
        }

        // Last, because it is the only one whose failure leaves state behind if it is not.
        // `IdentityTable::mint` commits nothing on failure, so a refusal here has taken
        // neither the slot nor the hold either.
        let identity = match table.mint_keyed(self.producer, note, key) {
            Ok(identity) => identity,
            Err(IdentityError::ProducerOverEmitted { .. }) if self.stealing.steals() => {
                // ADR-0058 clause 6 at the live boundary: every index is held, so the policy
                // names the note to take. The victim is released in the minter and the mint
                // retried onto the index it freed — the only free one — and the note-on
                // expands as the compiled path's does: a retrigger of a held note on the same
                // node and key, or a fade from here and a start `fade` frames on, deferred.
                // A hold was still free, so the new note takes its own and the taken note's
                // stays outstanding until its release arrives.
                return self.steal(table, time, note, key, velocity, false);
            }
            Err(
                IdentityError::ProducerOverEmitted { .. }
                | IdentityError::ProducerRangeEroded { .. },
            ) => {
                self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
                return Err(IngressRefused::Dropped {
                    resource: ExhaustedResource::Identity,
                });
            }
            Err(_) => {
                // An unknown producer cannot occur: `prepare` refused a producer the plan
                // does not declare, and the table is built from the same declarations.
                // Counted as an identity shortage rather than ignored, because a silent
                // `Ok` here would queue a note-on with no occurrence at all.
                self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
                return Err(IngressRefused::Dropped {
                    resource: ExhaustedResource::Identity,
                });
            }
        };

        self.holds_outstanding = self
            .holds_outstanding
            .checked_add(EventCount::measured(1))
            .unwrap_or(self.holds_outstanding);
        self.enqueue_entry(
            time,
            EventPayload::Note {
                identity,
                edge: NoteEdge::On {
                    slot: note,
                    key,
                    velocity,
                },
            },
            false,
        );
        Ok(identity)
    }

    /// Take a voice for a note-on that found every admitted index held (ADR-0058).
    ///
    /// `transfer_hold`: the taken note's hold goes to the new note, because no hold was free.
    fn steal(
        &mut self,
        table: &mut IdentityTable,
        time: SampleTime,
        note: NoteSlot,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
        transfer_hold: bool,
    ) -> Result<NoteIdentity, IngressRefused> {
        // A voice waiting to start, or in the tail before its displaced release, is not
        // taken: its slot here is occupied. With none eligible the note-on is a shortage.
        // Eligible: no deferred record, or one that has started and is not in the tail
        // before a displaced release. A note that took a voice keeps its record until its
        // release is published, so its release is displaced by the fade whenever it arrives —
        // the compiled path's rule — and the record says whether the voice is committed.
        let pending = &self.pending;
        let eligible = |index: u16| match pending.get(usize::from(index)) {
            Some(None) => true,
            Some(Some(record)) => record.started && record.ends.is_none(),
            None => false,
        };
        let Some(victim) = table.victim(self.producer, self.stealing, note, key, &eligible) else {
            self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Identity,
            });
        };
        // The replacement must use the victim's index. At the generation ceiling,
        // release retires that index instead of freeing it; refuse before ending the
        // victim or consuming its hold.
        if table.release_would_retire(victim) {
            self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Identity,
            });
        }
        let victim_note = table.note_of(victim);
        let retrigger = matches!(self.stealing, crate::ir::StealingPolicy::SameNote { .. })
            && victim_note == Some(note)
            && self.stolen_key_matches(table, victim, key);
        // A retrigger is two ring entries; a fade is one and a deferred start. Checked before
        // anything is released, so a refusal leaves the minter as it was.
        let entries = if retrigger { 2 } else { 1 };
        if !self.room_for_entries(entries, usize::from(!transfer_hold)) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        if table.release(victim) != Resolution::Live {
            self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Identity,
            });
        }
        let identity = match table.mint_keyed(self.producer, note, key) {
            Ok(identity) => identity,
            Err(_) => {
                self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
                return Err(IngressRefused::Dropped {
                    resource: ExhaustedResource::Identity,
                });
            }
        };
        // The taken note's own release is still to come; when it does it is counted, not
        // treated as an orphan, and it discharges the hold its note-on reserved.
        self.record_stolen(victim, transfer_hold);
        // The taken note's own deferred record, if it took this voice itself: its release is
        // now counted rather than displaced, so the record goes with the note.
        if let Some(slot) = self.pending.get_mut(usize::from(victim.index()))
            && slot.is_some()
        {
            *slot = None;
            self.pending_len = self.pending_len.saturating_sub(1);
        }
        if !transfer_hold {
            self.holds_outstanding = self
                .holds_outstanding
                .checked_add(EventCount::measured(1))
                .unwrap_or(self.holds_outstanding);
        }
        if retrigger {
            self.enqueue_entry(
                time,
                EventPayload::Note {
                    identity: victim,
                    edge: NoteEdge::Off,
                },
                false,
            );
            self.enqueue_entry(
                time,
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On {
                        slot: note,
                        key,
                        velocity,
                    },
                },
                false,
            );
            return Ok(identity);
        }
        let fade = self
            .stealing
            .fade()
            .unwrap_or(crate::time::FrameCount::ZERO);
        let Ok(starts) = time.checked_add(fade) else {
            self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Identity,
            });
        };
        self.enqueue_entry(
            time,
            EventPayload::Fade {
                identity: victim,
                frames: fade,
            },
            false,
        );
        if let Some(slot) = self.pending.get_mut(usize::from(identity.index())) {
            *slot = Some(PendingStart {
                identity,
                note,
                key,
                velocity,
                starts,
                started: false,
                ends: None,
                bend: None,
                off_published: false,
                table_released: false,
            });
            self.pending_len = self.pending_len.saturating_add(1);
        }
        Ok(identity)
    }

    /// Remember a taken note until its release arrives.
    fn record_stolen(&mut self, victim: NoteIdentity, transfer_hold: bool) {
        if !transfer_hold {
            for slot in &mut self.stolen_with_hold {
                if slot.is_none() {
                    *slot = Some(victim);
                    break;
                }
            }
            return;
        }
        for slot in &mut self.stolen_transferred {
            if slot.is_none() {
                *slot = Some(victim);
                return;
            }
        }
        let capacity = self.stolen_transferred.len();
        if capacity == 0 {
            return;
        }
        if let Some(slot) = self.stolen_transferred.get_mut(self.stolen_next) {
            *slot = Some(victim);
        }
        self.stolen_next = (self.stolen_next + 1) % capacity;
    }

    fn take_stolen(&mut self, identity: NoteIdentity) -> Option<bool> {
        for slot in &mut self.stolen_with_hold {
            if *slot == Some(identity) {
                *slot = None;
                return Some(true);
            }
        }
        for slot in &mut self.stolen_transferred {
            if *slot == Some(identity) {
                *slot = None;
                return Some(false);
            }
        }
        None
    }

    /// Free, in the minter, every deferred note whose displaced release has passed `now`, and
    /// forget those the drain has finished with. Run at each offer, where the minter is at
    /// hand: the drain cannot touch it.
    fn sweep(&mut self, table: &mut IdentityTable, now: SampleTime) {
        for index in 0..self.pending.len() {
            let Some(Some(mut pending)) = self.pending.get(index).copied() else {
                continue;
            };
            let Some(ends) = pending.ends else {
                continue;
            };
            if ends > now {
                continue;
            }
            if !pending.table_released {
                let _ = table.release(pending.identity);
                pending.table_released = true;
            }
            if let Some(slot) = self.pending.get_mut(index) {
                if pending.off_published {
                    *slot = None;
                    self.pending_len = self.pending_len.saturating_sub(1);
                } else {
                    *slot = Some(pending);
                }
            }
        }
    }

    /// Whether the live note `victim` was minted at `key`.
    fn stolen_key_matches(
        &self,
        table: &IdentityTable,
        victim: NoteIdentity,
        key: crate::quantities::KeyIdentity,
    ) -> bool {
        table.key_of(victim) == Some(key)
    }

    /// Offer a per-note bend for a note this producer holds open (`SOUND-INV-021`).
    ///
    /// A bend opens no obligation, so it takes a slot and nothing else, as a parameter write
    /// does. One for a note whose start a steal deferred waits with that start, displaced by
    /// the same fade; one for a note that is not this producer's live occurrence is refused as
    /// an orphan and counted, never queued to be refused a pass later.
    pub(crate) fn offer_bend(
        &mut self,
        table: &mut IdentityTable,
        time: SampleTime,
        identity: NoteIdentity,
        cents: crate::quantities::Cents,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;
        self.sweep(table, time);
        let index = usize::from(identity.index());
        if let Some(Some(pending)) = self.pending.get_mut(index)
            && pending.identity == identity
            && !pending.started
        {
            let fade = self
                .stealing
                .fade()
                .unwrap_or(crate::time::FrameCount::ZERO);
            // A displaced time the clock cannot hold is past every horizon.
            let Ok(at) = time.checked_add(fade) else {
                self.counters.beyond_horizon = self.counters.beyond_horizon.saturating_add(1);
                return Err(IngressRefused::BeyondHorizon {
                    time,
                    horizon_end: time,
                });
            };
            let had_bend = pending.bend.is_some();
            pending.bend = Some((at, cents));
            // Charged as the one live-class entry the drain will publish, like the start it
            // waits with; a bend replacing one still waiting takes no more room.
            if !had_bend && !self.room_for_entries(0, 0) {
                if let Some(Some(record)) = self.pending.get_mut(index) {
                    record.bend = None;
                }
                self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
                return Err(IngressRefused::Dropped {
                    resource: ExhaustedResource::Slot,
                });
            }
            return Ok(());
        }
        if identity.table() != table.id()
            || !self.owns(identity)
            || table.resolve(identity) != Resolution::Live
        {
            self.counters.orphan_expressions = self.counters.orphan_expressions.saturating_add(1);
            return Err(IngressRefused::OrphanExpression { identity });
        }
        if !self.room_for_entries(1, 0) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        self.enqueue_entry(time, EventPayload::Bend { identity, cents }, false);
        Ok(())
    }

    /// Offer a source update for an owned live identity, retaining every accepted update.
    pub(crate) fn offer_expression(
        &mut self,
        table: &mut IdentityTable,
        time: SampleTime,
        identity: NoteIdentity,
        expression: crate::controller::NoteExpression,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;
        self.sweep(table, time);
        let pending = self
            .pending
            .get(usize::from(identity.index()))
            .copied()
            .flatten()
            .filter(|pending| pending.identity == identity);
        if identity.table() != table.id()
            || !self.owns(identity)
            || table.resolve(identity) != Resolution::Live
            || pending.is_some_and(|pending| pending.ends.is_some())
        {
            self.counters.orphan_expressions = self.counters.orphan_expressions.saturating_add(1);
            return Err(IngressRefused::OrphanExpression { identity });
        }
        if !self.room_for_entries(1, 0) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        let payload = EventPayload::Expression {
            identity,
            expression,
        };
        if pending.is_some() {
            let fade = self
                .stealing
                .fade()
                .unwrap_or(crate::time::FrameCount::ZERO);
            let Ok(at) = time.checked_add(fade) else {
                self.counters.beyond_horizon = self.counters.beyond_horizon.saturating_add(1);
                return Err(IngressRefused::BeyondHorizon {
                    time,
                    horizon_end: time,
                });
            };
            // The same capacity relation proved above covers both rings and every hold.
            if let Some(entry) = self.delayed_expressions.get_mut(self.expression_head) {
                *entry = Some(crate::render::TimedEvent::new(
                    Self::envelope_for(self.epoch, at),
                    payload,
                ));
                self.expression_head = if self.expression_head + 1 == self.delayed_expressions.len()
                {
                    0
                } else {
                    self.expression_head + 1
                };
                self.expression_len = self.expression_len.saturating_add(1);
                self.last_accepted = Some(time);
                return Ok(());
            }
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        self.enqueue_entry(time, payload, false);
        Ok(())
    }

    /// Whether an identity's index lies in this producer's admitted range.
    fn owns(&self, identity: NoteIdentity) -> bool {
        self.pending.get(usize::from(identity.index())).is_some()
    }

    /// Offer the release of a note this producer opened.
    ///
    /// **It cannot be dropped for a full queue**, and that is ADR-0046 clause 6 rather
    /// than a convenience: "Once the note-on is published, its matching release cannot be
    /// dropped by queue pressure." The hold acquired with the note-on is what reserves the
    /// room, so the release spends the hold and is written even when the queue is
    /// otherwise full — the queue is sized so that the notes a producer may hold open are
    /// exactly the releases it may owe.
    pub(crate) fn offer_note_off(
        &mut self,
        table: &mut IdentityTable,
        time: SampleTime,
        identity: NoteIdentity,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;
        self.sweep(table, time);

        // A release naming nothing open is refused here rather than queued. Queuing it
        // would spend a slot to reach a renderer that refuses it one pass later, and the
        // producer would learn about it a callback too late to correct anything.
        //
        // `release` frees the minter's index as it resolves, which is what lets the next
        // note-on reuse it. The freed index comes back at the **next** generation, so the
        // occurrence still in flight stays distinguishable from the one that reuses its
        // index — and the renderer applies this release before that note-on, because both
        // are this producer's queue entries and ADR-0023 keeps a producer's own order.
        // **This producer's own occurrence, and this store's own hold.** Checking the table
        // alone accepted any live identity in it, including a *compiled* producer's: the
        // release then ended a note this producer never opened, spent this store's hold on
        // it, and left this producer's own note occupied — so the next note-on found the
        // range full with a hold still free. `release_for` refuses an index outside the
        // producer's admitted range. An independent review found it.
        //
        // The hold check below is the second half. Checking the table alone
        // accepts a release for a note some *other* store of the same producer opened: this
        // store would then saturate a zero hold count to zero and push without the room its
        // reservation was supposed to guarantee, while the store that minted the note kept a
        // hold nothing can ever discharge. ADR-0046 clause 6 partitions entitlements so that
        // "no producer borrows another's unused holds", and an unowned release is exactly
        // that borrowing. An independent review found it.
        // ADR-0058 clause 5: the release of a note a steal ended. Nothing to release — the
        // voice belongs to the note that took it — but the hold its note-on reserved is
        // discharged, and it is counted under its own name rather than as an orphan.
        let index = usize::from(identity.index());
        if let Some(held_reservation) = self.take_stolen(identity) {
            self.counters.released_after_steal =
                self.counters.released_after_steal.saturating_add(1);
            if held_reservation {
                self.holds_outstanding =
                    EventCount::measured(self.holds_outstanding.get().saturating_sub(1));
            }
            return Ok(());
        }
        if self.holds_outstanding == EventCount::NONE {
            self.counters.orphan_releases = self.counters.orphan_releases.saturating_add(1);
            return Err(IngressRefused::OrphanRelease { identity });
        }
        // A release for a note whose start the steal deferred is displaced with it, so the
        // note keeps its authored length and its release never precedes its start.
        if let Some(Some(pending)) = self.pending.get_mut(index)
            && pending.identity == identity
            && pending.ends.is_none()
        {
            if table.resolve(identity) != Resolution::Live {
                self.counters.orphan_releases = self.counters.orphan_releases.saturating_add(1);
                return Err(IngressRefused::OrphanRelease { identity });
            }
            // The index is **not** freed here: the voice is committed until the displaced
            // release lands, and `sweep` frees it then.
            let fade = self
                .stealing
                .fade()
                .unwrap_or(crate::time::FrameCount::ZERO);
            let Ok(ends) = time.checked_add(fade) else {
                self.counters.orphan_releases = self.counters.orphan_releases.saturating_add(1);
                return Err(IngressRefused::OrphanRelease { identity });
            };
            pending.ends = Some(ends);
            self.holds_outstanding =
                EventCount::measured(self.holds_outstanding.get().saturating_sub(1));
            return Ok(());
        }
        if table.release_for(self.producer, identity) != Resolution::Live {
            self.counters.orphan_releases = self.counters.orphan_releases.saturating_add(1);
            return Err(IngressRefused::OrphanRelease { identity });
        }

        // **The hold is discharged here, not at publication**, and the two are not
        // interchangeable: the reservation exists to keep a queue slot free for this
        // release, and the release now occupies one. Holding it until the drain would
        // reserve a slot for an event that is already sitting in a slot, which shrinks the
        // usable queue by one per note in flight.
        self.holds_outstanding =
            EventCount::measured(self.holds_outstanding.get().saturating_sub(1));
        self.enqueue_entry(
            time,
            EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            },
            true,
        );
        Ok(())
    }

    /// The two checks every offer takes, whatever it carries.
    ///
    /// Monotonicity, and `HOST-INV-013`'s single evaluation of the forward horizon — see the
    /// module header for why this boundary is that site and the renderer's is retired.
    /// Nothing is checked against the stream's epoch: a store serves one epoch and
    /// re-preparation builds a new one, so a stamp from another stream cannot reach this.
    pub(super) fn admit(&mut self, time: SampleTime) -> Result<(), IngressRefused> {
        if let Some(last) = self.last_accepted
            && time < last
        {
            self.counters.non_monotone = self.counters.non_monotone.saturating_add(1);
            return Err(IngressRefused::NonMonotoneStamp { time, last });
        }
        self.admit_horizon(time)
    }

    /// Whether [`Self::admit_horizon`] would admit `time`, without counting anything.
    /// The private mixed owner parks a future Panic only inside the horizon; one beyond
    /// it is served at once, so the store refuses and counts it as before.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) fn within_horizon(&self, time: SampleTime) -> bool {
        self.clock
            .checked_add(self.forward_event_horizon)
            .map_or(true, |end| time <= end)
    }

    /// The forward-horizon half of [`Self::admit`], counted once when it refuses.
    ///
    /// ADR-0078's transfer rehearsal evaluates it for every served packet before any
    /// occurrence or queue mutation, including a key-up that only consumes a tombstone.
    /// The offer that follows re-reads the same clock and stamp, so it cannot refuse or
    /// count again.
    pub(crate) fn admit_horizon(&mut self, time: SampleTime) -> Result<(), IngressRefused> {
        // `HOST-INV-013`'s single evaluation. Measured from the render clock recorded
        // by the drain or by the private mixed owner's current opening boundary, rather
        // than from a caller's notion of now: this half does not own a clock, and a
        // horizon measured from an offer-side guess would move with the offerer.
        //
        // A store whose drain has not run yet measures from its origin, which is the
        // conservative direction — it admits less, never more.
        if let Ok(end) = self.clock.checked_add(self.forward_event_horizon)
            && time > end
        {
            self.counters.beyond_horizon = self.counters.beyond_horizon.saturating_add(1);
            return Err(IngressRefused::BeyondHorizon {
                time,
                horizon_end: end,
            });
        }
        Ok(())
    }

    /// Write one entry. Every caller has already decided it fits.
    /// The stamp every event this store publishes carries: its stream's epoch, the time the
    /// producer named, and the simulated source — the one place the source is written.
    pub(crate) const fn envelope(&self, time: SampleTime) -> EventEnvelope {
        Self::envelope_for(self.epoch, time)
    }

    /// [`Self::envelope`] for a stream epoch, where the store itself is borrowed.
    pub(crate) const fn envelope_for(epoch: StreamEpoch, time: SampleTime) -> EventEnvelope {
        EventEnvelope::new(epoch, time, TimeSource::Simulated)
    }

    pub(super) fn enqueue_entry(
        &mut self,
        time: SampleTime,
        payload: EventPayload,
        redeems_hold: bool,
    ) {
        let entry = IngressEntry {
            event: TimedEvent::new(self.envelope(time), payload),
            redeems_hold,
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            inherited_bend: false,
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            bend_credit: false,
        };
        if let Some(slot) = self.entries.get_mut(self.head) {
            *slot = Some(entry);
        }
        self.last_accepted = Some(match self.last_accepted {
            Some(last) if last > time => last,
            _ => time,
        });
        self.head = self.head.saturating_add(1);
        if self.head >= self.entries.len() {
            self.head = 0;
        }
        self.len = self.len.saturating_add(1);
    }
}
