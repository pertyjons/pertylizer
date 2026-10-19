//! Later compiled activations through ADR-0078's host-owned transfer rehearsal.
//!
//! Control prepares and seals one later Start or seek candidate off-thread, against the
//! sequence it holds in force and the arm profile, as an opaque
//! [`MixedActivationOffer`] bound to its transfer. The host carries that value to the
//! audio thread by any means it owns and offers it between callbacks. The audio half
//! keeps one offer slot: it accepts the box or returns it whole with the reason, and at
//! the candidate's boundary it adopts or refuses it, or an applied Stop cancels it. The
//! same allocation then waits in the slot, retired with a fixed-size receipt, until the
//! host takes it between callbacks and carries it back to control, which collects it
//! off-thread. Offering, adopting, retiring and taking move one box and allocate, lock
//! and free nothing; every list is dropped by control or joined teardown, never by a
//! callback. An offer of another transfer is refused and returned. Joined teardown
//! reports whatever the slot still holds. The reset wrapper forwards the same offer, take
//! and slot report to its active owner through the same binding check, so after a
//! committed reset an offer prepared by an earlier owner's control is foreign; while a
//! reset successor is pending the wrapper refuses an offer as
//! [`MixedActivationRefusal::ResetPending`].
//!
//! Adoption is compiled-scoped (draft Item 2): the boundary release ends only compiled
//! occurrences, so live notes, holds, the ingress store and every live command are
//! unchanged.
//!
//! A later candidate may also enter a loop of the bound stream (draft rules 2 and 3.1):
//! [`MixedTransferControl::prepare_loop_activation`] seals the entry list with its
//! repeating pass, admitted from the entry boundary through every wrap alignment, and it
//! travels through the same slot as any other candidate. A wrap never uses the slot: it
//! runs in the quantum's compiled order at its own sample and visits no live
//! occurrence, row, ramp or minter. An ordinary later seek or Start leaves the loop, and
//! its receipt names the pass it retired. The armed candidate cannot carry a loop.

use thiserror::Error;

use super::{MixedTransferAudio, MixedTransferBinding, MixedTransferControl};
use crate::{
    looping::LoopPassId,
    quantities::{EventCount, HeldNoteCount},
    stream::mixed::{
        MixedBridgeStopAccepted, MixedBridgeStopRefusal, MixedCandidateOffer, MixedDrainedOffer,
        MixedOfferCollectError, MixedOfferKind, MixedOfferOutcome, MixedOfferPrepareError,
        MixedOfferReceipt, MixedOfferRefused, MixedOfferSlot, MixedOneShotAudio, MixedRetiredOffer,
        MixedSubsequentPrepareError,
    },
    time::{PlanPosition, SampleTime, StreamAnchor},
    transport::{ActivationSequence, LoopInterval},
};

/// What a later activation does to the compiled transport at its boundary. Both kinds
/// adopt the same way: the compiled boundary release, the program and anchor swap, then
/// scoped restoration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedActivationKind {
    /// Keep the transport as it stands at the boundary. A running program publishes the
    /// new list; a stopped one publishes only its scoped restoration.
    Seek,
    /// Resume a stopped program: the new list publishes in full from the boundary.
    Start,
}

impl From<MixedActivationKind> for MixedOfferKind {
    fn from(kind: MixedActivationKind) -> Self {
        match kind {
            MixedActivationKind::Seek => Self::Seek,
            MixedActivationKind::Start => Self::Start,
        }
    }
}

impl From<MixedOfferKind> for MixedActivationKind {
    fn from(kind: MixedOfferKind) -> Self {
        match kind {
            MixedOfferKind::Seek => Self::Seek,
            MixedOfferKind::Start => Self::Start,
        }
    }
}

/// One sealed later candidate of one transfer, on its way to that transfer's audio half.
/// Only control builds one, after admitting it at its boundary against the arm profile.
/// It names no live identity and mints nothing, so the host may drop one it never offered
/// or got back, off the audio thread.
#[derive(Debug)]
#[must_use]
pub struct MixedActivationOffer {
    binding: MixedTransferBinding,
    offer: MixedCandidateOffer,
}

impl MixedActivationOffer {
    /// The sequence the candidate supersedes.
    pub fn supersedes(&self) -> ActivationSequence {
        self.offer.0.candidate.supersedes
    }

    /// The quantum boundary at which the candidate takes effect.
    pub fn effective(&self) -> SampleTime {
        self.offer.0.timing.effective()
    }

    /// Whether the candidate seeks or starts.
    pub fn kind(&self) -> MixedActivationKind {
        self.offer.0.kind.into()
    }

    /// The loop the candidate enters, if any.
    pub fn loop_interval(&self) -> Option<LoopInterval> {
        self.offer
            .0
            .loop_program
            .as_deref()
            .map(|program| program.interval)
    }
}

/// The same allocation coming back from the audio half's slot with its receipt. After
/// an adoption it holds the retired list. Control collects it off-thread.
#[derive(Debug)]
#[must_use]
pub struct MixedActivationRetired {
    binding: MixedTransferBinding,
    retired: MixedRetiredOffer,
}

impl MixedActivationRetired {
    /// Whether the candidate adopted, and if not, why.
    pub fn receipt(&self) -> MixedActivationReceipt {
        self.retired.receipt.into()
    }
}

/// An opaque refusal to prepare or seal a later candidate. Control is unchanged and
/// nothing was minted.
#[derive(Clone, Copy, Debug, PartialEq, Error)]
#[error(transparent)]
pub struct MixedActivationPrepareError(ActivationPrepare);

#[derive(Clone, Copy, Debug, PartialEq, Error)]
enum ActivationPrepare {
    #[error("activation custody identities are exhausted")]
    CustodyExhausted,
    #[error("an owning plan continuation is pending")]
    ContinuationPending,
    #[error(transparent)]
    Prepare(MixedSubsequentPrepareError),
    #[error(transparent)]
    Seal(MixedOfferPrepareError),
}

/// Why the audio half refused a later candidate, at its offer or when its boundary
/// opened. A refusal changes no program, minter, registry row, live occurrence or slot
/// content, and the renderer counts it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedActivationRefusal {
    #[error("an owning plan continuation is pending or uncollected")]
    ContinuationPending,
    /// The offer belongs to another transfer.
    #[error("the activation offer belongs to another transfer")]
    ForeignTransfer,
    /// The audio half is terminally faulted.
    #[error("the mixed audio half is terminally faulted")]
    Faulted,
    /// The offer belongs to another plan, epoch or table.
    #[error("the activation offer belongs to another plan, epoch or table")]
    ForeignCandidate,
    /// The single slot holds a candidate waiting for its boundary.
    #[error("a later activation is already pending")]
    AlreadyPending,
    /// The single slot holds a retired candidate the host has not taken.
    #[error("a retired activation has not been taken")]
    RetiredUncollected,
    /// The candidate does not supersede the sequence in force at its boundary.
    #[error("the offer supersedes {supersedes}, but {in_force} is in force at its boundary")]
    Superseded {
        /// The sequence the candidate supersedes.
        supersedes: ActivationSequence,
        /// The sequence in force at its boundary.
        in_force: ActivationSequence,
    },
    /// The boundary does not follow the transition in force or still pending from arm.
    #[error("offer boundary {effective} does not follow the in-force boundary {boundary}")]
    NotAfterInForce {
        /// The candidate's boundary.
        effective: SampleTime,
        /// The in-force transition's boundary.
        boundary: SampleTime,
    },
    /// The boundary's quantum has already opened.
    #[error("offer boundary {effective} precedes the render clock {clock}")]
    BoundaryPassed {
        /// The candidate's boundary.
        effective: SampleTime,
        /// The audio half's render clock.
        clock: SampleTime,
    },
    /// A compiled Stop is pending at or before the boundary.
    #[error("offer boundary {effective} does not precede the pending Stop at {stop}")]
    StopPending {
        /// The pending Stop's boundary.
        stop: SampleTime,
        /// The candidate's boundary.
        effective: SampleTime,
    },
    /// A Start needs a stopped compiled program at its boundary.
    #[error("a Start needs a stopped compiled program")]
    StartWhileRunning,
    /// The reset wrapper holds a successor waiting for its boundary `at`. A pending reset
    /// wins at the caller boundary, as a pending candidate refuses a reset offer; the
    /// host may offer again after withdrawing the successor.
    #[error("a reset successor is pending for boundary {at}")]
    ResetPending {
        /// The pending reset's boundary.
        at: SampleTime,
    },
}

impl From<MixedOfferRefused> for MixedActivationRefusal {
    fn from(refused: MixedOfferRefused) -> Self {
        use MixedOfferRefused as Private;
        match refused {
            Private::Faulted => Self::Faulted,
            Private::ForeignCandidate => Self::ForeignCandidate,
            Private::AlreadyPending => Self::AlreadyPending,
            Private::RetiredUncollected => Self::RetiredUncollected,
            Private::Superseded {
                supersedes,
                in_force,
            } => Self::Superseded {
                supersedes,
                in_force,
            },
            Private::NotAfterInForce {
                effective,
                boundary,
            } => Self::NotAfterInForce {
                effective,
                boundary,
            },
            Private::BoundaryPassed { effective, clock } => {
                Self::BoundaryPassed { effective, clock }
            }
            Private::StopPending { stop, effective } => Self::StopPending { stop, effective },
            Private::StartWhileRunning => Self::StartWhileRunning,
        }
    }
}

/// What the audio half reports for one retired candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedActivationReceipt {
    supersedes: ActivationSequence,
    sequence: ActivationSequence,
    late: bool,
    kind: MixedActivationKind,
    outcome: MixedActivationOutcome,
}

impl MixedActivationReceipt {
    /// The sequence the candidate superseded.
    pub const fn supersedes(&self) -> ActivationSequence {
        self.supersedes
    }

    /// The candidate's own sequence, in force after an adoption.
    pub const fn sequence(&self) -> ActivationSequence {
        self.sequence
    }

    /// Whether the clock had passed the requested time when audio accepted the offer.
    pub const fn late(&self) -> bool {
        self.late
    }

    /// Whether the candidate seeks or starts.
    pub const fn kind(&self) -> MixedActivationKind {
        self.kind
    }

    /// Whether it adopted, was refused at its boundary or was cancelled by a Stop.
    pub const fn outcome(&self) -> MixedActivationOutcome {
        self.outcome
    }
}

impl From<MixedOfferReceipt> for MixedActivationReceipt {
    fn from(receipt: MixedOfferReceipt) -> Self {
        Self {
            supersedes: receipt.supersedes,
            sequence: receipt.sequence,
            late: receipt.late,
            kind: receipt.kind.into(),
            outcome: match receipt.outcome {
                MixedOfferOutcome::Adopted(adopted) => {
                    MixedActivationOutcome::Adopted(MixedActivationAdopted {
                        effective: adopted.effective,
                        anchor: adopted.anchor,
                        retired_anchor: adopted.retired_anchor,
                        released: adopted.released,
                        stopped: adopted.stopped,
                        loop_interval: adopted.loop_interval,
                        retired_pass: adopted.retired_pass,
                    })
                }
                MixedOfferOutcome::Refused(refused) => {
                    MixedActivationOutcome::Refused(refused.into())
                }
                MixedOfferOutcome::Cancelled { stop } => MixedActivationOutcome::Cancelled { stop },
            },
        }
    }
}

/// How one retired candidate left the slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedActivationOutcome {
    /// It adopted at its boundary.
    Adopted(MixedActivationAdopted),
    /// It was refused when its boundary opened; the program in force did not change.
    Refused(MixedActivationRefusal),
    /// A compiled Stop applied at `stop`, at or before its boundary, took it out of the
    /// slot unadopted. The program in force did not change.
    Cancelled {
        /// Where the Stop applied.
        stop: SampleTime,
    },
}

/// One confirmed later adoption.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedActivationAdopted {
    effective: SampleTime,
    anchor: StreamAnchor,
    retired_anchor: StreamAnchor,
    released: HeldNoteCount,
    stopped: bool,
    loop_interval: Option<LoopInterval>,
    retired_pass: Option<LoopPassId>,
}

impl MixedActivationAdopted {
    /// The quantum boundary at which it adopted.
    pub const fn effective(&self) -> SampleTime {
        self.effective
    }

    /// The mapping now in force.
    pub const fn anchor(&self) -> StreamAnchor {
        self.anchor
    }

    /// The mapping the retired list belonged to.
    pub const fn retired_anchor(&self) -> StreamAnchor {
        self.retired_anchor
    }

    /// Sounding compiled occurrences the boundary release ended. Live notes are never
    /// among them.
    pub const fn released(&self) -> HeldNoteCount {
        self.released
    }

    /// Whether the adopted program is stopped, so it published only its restoration.
    pub const fn stopped(&self) -> bool {
        self.stopped
    }

    /// The loop the adopted program plays, if any.
    pub const fn loop_interval(&self) -> Option<LoopInterval> {
        self.loop_interval
    }

    /// The pass the retired program was in, if it was a loop. The rest of that pass
    /// never published.
    pub const fn retired_pass(&self) -> Option<LoopPassId> {
        self.retired_pass
    }
}

/// Where the audio half's compiled loop stands. Fixed-size and `Copy`; reading it
/// allocates nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedActivationLoop {
    interval: Option<LoopInterval>,
    pass: Option<LoopPassId>,
    wraps: EventCount,
    wrap_releases: HeldNoteCount,
}

impl MixedActivationLoop {
    /// The loop of the program in force, if any. It wraps only while running.
    pub const fn interval(&self) -> Option<LoopInterval> {
        self.interval
    }

    /// The pass that loop is in; the entry plays the first.
    pub const fn pass(&self) -> Option<LoopPassId> {
        self.pass
    }

    /// Applied wraps, cumulative over every loop this half played.
    pub const fn wraps(&self) -> EventCount {
        self.wraps
    }

    /// Open compiled occurrences those wraps ended. Live notes are never among them.
    pub const fn wrap_releases(&self) -> HeldNoteCount {
        self.wrap_releases
    }
}

/// What the audio half's offer slot holds, or held when joined teardown drained it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedActivationSlot {
    /// No candidate.
    Empty,
    /// A candidate waits for its boundary. In joined teardown it never adopted and is
    /// withdrawn; it minted nothing.
    Pending {
        /// The sequence it supersedes.
        supersedes: ActivationSequence,
        /// Its boundary.
        effective: SampleTime,
        /// Whether it seeks or starts.
        kind: MixedActivationKind,
        /// The loop it enters, if any.
        loop_interval: Option<LoopInterval>,
    },
    /// A retired candidate waits to be taken, with its receipt. In joined teardown an
    /// adoption here was never collected by control.
    Retired(MixedActivationReceipt),
}

impl MixedActivationSlot {
    fn of(slot: &MixedOfferSlot) -> Self {
        match slot {
            MixedOfferSlot::Empty => Self::Empty,
            MixedOfferSlot::Pending(offer) => Self::pending(offer),
            MixedOfferSlot::Retired(retired) => Self::Retired(retired.receipt.into()),
        }
    }

    fn pending(offer: &MixedCandidateOffer) -> Self {
        Self::Pending {
            supersedes: offer.0.candidate.supersedes,
            effective: offer.0.timing.effective(),
            kind: offer.0.kind.into(),
            loop_interval: offer
                .0
                .loop_program
                .as_deref()
                .map(|program| program.interval),
        }
    }

    /// The drained slot's report; the drained contents are dropped by the caller,
    /// off-thread.
    pub(super) fn drained(drained: &MixedDrainedOffer) -> Self {
        match drained {
            MixedDrainedOffer::Empty => Self::Empty,
            MixedDrainedOffer::Withdrawn(offer) => Self::pending(offer),
            MixedDrainedOffer::Retired(retired) => Self::Retired(retired.receipt.into()),
        }
    }
}

/// Why control refused to collect a retired candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedActivationCollectError {
    /// The retired candidate belongs to another transfer.
    #[error("the retired activation belongs to another transfer")]
    ForeignTransfer,
    /// The retired candidate belongs to another plan, epoch or table.
    #[error("the retired activation belongs to another plan, epoch or table")]
    ForeignCandidate,
    /// Defensive: an adoption must supersede the sequence control holds in force.
    #[error("the adoption superseded {supersedes}, but {in_force} is in force")]
    Superseded {
        /// The sequence the adoption superseded.
        supersedes: ActivationSequence,
        /// The sequence control holds in force.
        in_force: ActivationSequence,
    },
}

impl From<MixedOfferCollectError> for MixedActivationCollectError {
    fn from(error: MixedOfferCollectError) -> Self {
        match error {
            MixedOfferCollectError::ForeignCandidate => Self::ForeignCandidate,
            MixedOfferCollectError::Superseded {
                supersedes,
                in_force,
            } => Self::Superseded {
                supersedes,
                in_force,
            },
        }
    }
}

/// A collection refusal with the retired candidate unchanged.
#[derive(Debug)]
#[must_use]
pub struct MixedActivationCollectRefusal {
    reason: MixedActivationCollectError,
    retired: MixedActivationRetired,
}

impl MixedActivationCollectRefusal {
    /// Why control refused it.
    pub const fn reason(&self) -> MixedActivationCollectError {
        self.reason
    }

    /// The retired candidate, unchanged.
    pub fn into_retired(self) -> MixedActivationRetired {
        self.retired
    }
}

impl MixedTransferControl {
    /// Prepare and seal a later Start or seek that takes effect at the quantum boundary
    /// `effective`, off-thread.
    ///
    /// The candidate reconstructs the bound stream at `position` mapped to `requested`,
    /// supersedes the sequence this control holds in force, and is admitted against the
    /// arm profile at exactly `effective`'s displacement. `requested` must follow the
    /// in-force boundary and `effective` must be a quantum boundary at or after it.
    /// Preparing mints, reserves and changes nothing: until a retired adoption is
    /// collected, every candidate supersedes the same sequence and at most one adopts.
    ///
    /// # Errors
    ///
    /// The candidate could not be built, displaced or admitted at that boundary.
    pub fn prepare_activation(
        &self,
        requested: SampleTime,
        position: PlanPosition,
        effective: SampleTime,
        kind: MixedActivationKind,
    ) -> Result<MixedActivationOffer, MixedActivationPrepareError> {
        if self.continuation_pending.is_some() {
            return Err(MixedActivationPrepareError(
                ActivationPrepare::ContinuationPending,
            ));
        }
        let candidate = self
            .inner
            .prepare_subsequent(requested, position, self.inner.in_force)
            .map_err(|error| MixedActivationPrepareError(ActivationPrepare::Prepare(error)))?;
        // A refused candidate is dropped here, off-thread; it minted nothing.
        let mut offer = self
            .inner
            .seal_offer(candidate, effective, kind.into())
            .map_err(|refused| MixedActivationPrepareError(ActivationPrepare::Seal(refused.0)))?;
        offer.0.activation_custody =
            Some(ActivationCustody::acquire(&self.activation_custody).ok_or(
                MixedActivationPrepareError(ActivationPrepare::CustodyExhausted),
            )?);
        Ok(MixedActivationOffer {
            binding: self.binding,
            offer,
        })
    }

    /// Prepare and seal a later candidate that enters `interval` at `position`, taking
    /// effect at the quantum boundary `effective`, off-thread.
    ///
    /// The entry list is [`Self::prepare_activation`]'s, cut before the loop's end, and
    /// the repeating pass is the bound stream reconstructed from the loop's start. Each
    /// wrap ends the pass's open compiled notes with Session-charged `Off`s, then
    /// restores and replays the pass. The loop is admitted against the arm profile from
    /// `effective` through the first wrap and in its periodic steady state, so it holds
    /// for every quantum alignment a wrap can take; wraps charge nothing to Release or
    /// Live. A position at the loop's end enters at its start. Like any later candidate
    /// it mints, reserves and changes nothing, and an ordinary later candidate leaves it.
    ///
    /// # Errors
    ///
    /// A position outside the loop, or a candidate or loop that could not be built,
    /// displaced or admitted at that boundary.
    pub fn prepare_loop_activation(
        &self,
        requested: SampleTime,
        position: PlanPosition,
        interval: LoopInterval,
        effective: SampleTime,
        kind: MixedActivationKind,
    ) -> Result<MixedActivationOffer, MixedActivationPrepareError> {
        if self.continuation_pending.is_some() {
            return Err(MixedActivationPrepareError(
                ActivationPrepare::ContinuationPending,
            ));
        }
        let candidate = self
            .inner
            .prepare_loop(requested, position, interval, self.inner.in_force)
            .map_err(|error| MixedActivationPrepareError(ActivationPrepare::Prepare(error)))?;
        // A refused candidate is dropped here, off-thread; it minted nothing.
        let mut offer = self
            .inner
            .seal_loop_offer(candidate, effective, kind.into())
            .map_err(|refused| MixedActivationPrepareError(ActivationPrepare::Seal(refused.0)))?;
        offer.0.activation_custody =
            Some(ActivationCustody::acquire(&self.activation_custody).ok_or(
                MixedActivationPrepareError(ActivationPrepare::CustodyExhausted),
            )?);
        Ok(MixedActivationOffer {
            binding: self.binding,
            offer,
        })
    }

    /// Collect one retired candidate and its receipt, off-thread. Only an adoption
    /// advances this control's in-force sequence, boundary and anchor; the retired list
    /// is dropped here.
    ///
    /// # Errors
    ///
    /// A candidate of another transfer or owner, with the candidate unchanged.
    pub fn collect_activation(
        &mut self,
        retired: MixedActivationRetired,
    ) -> Result<MixedActivationReceipt, Box<MixedActivationCollectRefusal>> {
        if retired.binding != self.binding {
            return Err(Box::new(MixedActivationCollectRefusal {
                reason: MixedActivationCollectError::ForeignTransfer,
                retired,
            }));
        }
        let binding = retired.binding;
        self.inner
            .collect_offer(retired.retired)
            .map(MixedActivationReceipt::from)
            .map_err(|refused| {
                let (reason, retired) = *refused;
                Box::new(MixedActivationCollectRefusal {
                    reason: reason.into(),
                    retired: MixedActivationRetired { binding, retired },
                })
            })
    }
}

impl MixedOneShotAudio {
    /// The one binding check every host-armed audio half applies to a later candidate,
    /// whether the host renders it directly or through the reset wrapper: an offer of
    /// another transfer is refused, counted and returned whole before the slot's own
    /// checks run. Allocates, locks and frees nothing.
    pub(in crate::stream::mixed) fn offer_bound_activation(
        &mut self,
        offer: MixedActivationOffer,
    ) -> Result<(), (MixedActivationRefusal, MixedActivationOffer)> {
        if self.continuation.is_some() {
            self.audio.renderer.count_refused_activation();
            return Err((MixedActivationRefusal::ContinuationPending, offer));
        }
        if self.transfer_binding != Some(offer.binding) {
            self.audio.renderer.count_refused_activation();
            return Err((MixedActivationRefusal::ForeignTransfer, offer));
        }
        let binding = offer.binding;
        self.offer_candidate(offer.offer)
            .map_err(|(reason, offer)| (reason.into(), MixedActivationOffer { binding, offer }))
    }

    /// Refuse a candidate before any binding or slot check, as the reset wrapper does
    /// for its own reasons, counting it like every other refusal. Allocates nothing.
    pub(in crate::stream::mixed) fn refuse_bound_activation(
        &mut self,
        reason: MixedActivationRefusal,
        offer: MixedActivationOffer,
    ) -> (MixedActivationRefusal, MixedActivationOffer) {
        self.audio.renderer.count_refused_activation();
        (reason, offer)
    }

    /// Move a retired candidate out of the slot, bound to this owner's transfer.
    /// Allocates and frees nothing.
    pub(in crate::stream::mixed) fn take_bound_activation(
        &mut self,
    ) -> Option<MixedActivationRetired> {
        let binding = self.transfer_binding?;
        self.take_retired_offer()
            .map(|retired| MixedActivationRetired { binding, retired })
    }

    /// What the offer slot holds.
    pub(in crate::stream::mixed) fn bound_activation(&self) -> MixedActivationSlot {
        MixedActivationSlot::of(&self.offer_slot)
    }
}

impl MixedTransferAudio {
    /// Accept one later candidate into the single offer slot, or give it back whole.
    ///
    /// The checks run again when its boundary opens. A refusal leaves the program in
    /// force, both minters, the registry, every live occurrence and the slot unchanged,
    /// and is counted. Callable between callbacks on the audio thread: it moves one box
    /// and allocates, locks and frees nothing.
    ///
    /// # Errors
    ///
    /// Why the candidate was refused, with the candidate unchanged.
    pub fn offer_activation(
        &mut self,
        offer: MixedActivationOffer,
    ) -> Result<(), (MixedActivationRefusal, MixedActivationOffer)> {
        self.inner.offer_bound_activation(offer)
    }

    /// Move a retired candidate and its receipt out of the slot, emptying it for the next
    /// offer. Callable between callbacks on the audio thread: it allocates and frees
    /// nothing, and control drops the retired list.
    pub fn take_retired_activation(&mut self) -> Option<MixedActivationRetired> {
        self.inner.take_bound_activation()
    }

    /// What the offer slot holds.
    pub fn activation(&self) -> MixedActivationSlot {
        self.inner.bound_activation()
    }

    /// Where the compiled loop stands. Callable between callbacks on the audio thread.
    pub fn compiled_loop(&self) -> MixedActivationLoop {
        MixedActivationLoop {
            interval: self
                .inner
                .loop_program
                .as_deref()
                .map(|program| program.interval),
            pass: self.inner.loop_cursor.map(|cursor| cursor.pass),
            wraps: self.inner.loop_wraps,
            wrap_releases: self.inner.loop_wrap_releases,
        }
    }

    /// Schedule the compiled-scoped Stop at the quantum boundary `at`, applied before
    /// that quantum's compiled events in every callback partition. Live notes keep
    /// sounding. A candidate pending at or after `at` is cancelled when the Stop
    /// applies and retires with a cancelled receipt. Callable between callbacks on the
    /// audio thread; it allocates and frees nothing.
    ///
    /// # Errors
    ///
    /// A faulted owner, a boundary that is not a later quantum boundary after the armed
    /// transition, or a Stop already scheduled.
    pub fn schedule_stop(
        &mut self,
        at: SampleTime,
    ) -> Result<MixedBridgeStopAccepted, MixedBridgeStopRefusal> {
        self.inner
            .schedule_compiled_stop(at)
            .map(MixedBridgeStopAccepted::from)
            .map_err(MixedBridgeStopRefusal::from)
    }
}

/// One cold-prepared activation owner, retained through cancellation/refusal and
/// owning collection. Audio only moves its enclosing cell; destruction is cold.
#[derive(Debug)]
pub(in crate::stream::mixed) struct ActivationCustody(
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
);
impl ActivationCustody {
    fn acquire(counter: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Option<Self> {
        counter
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |count| count.checked_add(1),
            )
            .ok()?;
        Some(Self(std::sync::Arc::clone(counter)))
    }
}
impl Drop for ActivationCustody {
    fn drop(&mut self) {
        // The private non-Clone guard acquires exactly once and releases once.
        let _previous = self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}
