//! The public face of the reset wrapper over host-armed transfer owners (ADR-0078's
//! rehearsal, draft Item 3).
//!
//! A host boxes each armed audio half with the audio port its own arm returned, wraps the
//! active one, schedules its compiled Stop, and offers a freshly armed successor for a
//! quantum boundary `B` once that Stop has applied (rule 3.2). From `B` the old owner
//! serves no port, cross-fades out over `Q` frames, ends its live occurrences in one
//! `Reset` group at `B + Q` and retires (rule 3.3). The retired old owner and a successor
//! refused at `B` are taken back with their ports, and both owners come back for each
//! arm's joined teardown off the audio thread (rule 3.6). Boxing, construction, unboxing
//! and teardown run off-thread; every other call allocates and frees nothing.
//!
//! A command still in the old port at `B` is never served. A bridge-owned transfer's
//! seal refuses the reset until audio has served every command minted before sealing.
//! At `B` audio decides the seal with one compare-and-swap. A due Panic the bridge
//! accepted first wins it instead, and the successor is refused as
//! [`MixedBridgeResetRefusal::PanicCancelled`] while the old owner keeps serving.
//! A direct host-owned transfer has no seal; its unserved command does not refuse the
//! reset and joined teardown names it unserved. The bridge owns the handoff of old held
//! keys to reset tombstones after the wrapper reports an applied reset.
//!
//! Later Starts and seeks reach the active owner through the wrapper with the same
//! binding check as a directly rendered audio half, so after each committed reset only
//! the successor's own offers are accepted. A pending reset refuses an offer, and a
//! reset is refused until the old owner's offer slot is empty, so no candidate or
//! uncollected receipt crosses `B` into the fading owner.

use thiserror::Error;
mod publication_hot;

use super::super::{
    MixedStopAccepted, MixedStopRefused,
    occurrence::{MixedResetEnd, MixedResetLiveRefused},
    transfer::{
        MixedActivationOffer, MixedActivationRefusal, MixedActivationRetired, MixedActivationSlot,
        MixedKeyUpHandle, MixedTransferAudio, MixedTransferAudioPort, MixedTransferBinding,
        MixedTransferCommand, MixedTransferConsumer, MixedTransferProducer,
        MixedTransferPublicationUsage, MixedTransferRenderError, MixedTransferResult,
    },
};
use super::{
    MixedResetForeignEndpoint, MixedResetReceipt, MixedResetRefused, MixedResetRenderError,
    MixedResetSlot, MixedResetTransferAudio, MixedResetTransferOwner,
};
use crate::{
    render::AudioBlockMut,
    time::{FrameCount, SampleTime},
};

/// One host-armed audio half boxed with the audio port its own arm returned, prepared
/// off-thread for [`MixedBridgeResetAudio`].
#[must_use]
pub struct MixedBridgeResetOwner<C, R>(MixedResetTransferOwner<C, R>);

impl<C, R> std::fmt::Debug for MixedBridgeResetOwner<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("MixedBridgeResetOwner")
            .field(&self.0)
            .finish()
    }
}

impl<C, R> MixedBridgeResetOwner<C, R> {
    /// This owner's cumulative publication observations, including after retirement.
    pub fn publication_usage(&self) -> MixedTransferPublicationUsage {
        MixedTransferPublicationUsage::of(&self.0.owner)
    }

    /// The same resource observations retained while this owner waits or retires.
    ///
    /// # Errors
    /// A counter or storage amount cannot be represented.
    pub fn resource_usage(
        &self,
    ) -> Result<
        super::super::transfer::MixedTransferResourceUsage,
        crate::render::RehearsalUsageError,
    > {
        super::super::transfer::MixedTransferResourceUsage::of(&self.0.owner)
    }

    pub(crate) const fn binding(&self) -> MixedTransferBinding {
        self.0.endpoint.port.binding
    }

    pub(crate) fn fresh(&self) -> bool {
        let owner = &self.0.owner;
        !owner.render_started
            && !owner.adopted
            && owner.fault.is_none()
            && owner.stop_pending.is_none()
            && matches!(
                owner.transport,
                super::super::MixedCompiledTransport::Running
            )
    }

    /// Box `audio` with the audio port of its own arm, off-thread.
    ///
    /// # Errors
    ///
    /// The port belongs to another arm; both come back unchanged.
    pub fn new(
        audio: MixedTransferAudio,
        port: MixedTransferAudioPort<C, R>,
    ) -> Result<Self, Box<MixedBridgeResetForeignPort<C, R>>> {
        MixedResetTransferOwner::new(audio, port)
            .map(Self)
            .map_err(|foreign| Box::new(MixedBridgeResetForeignPort(*foreign)))
    }

    /// The audio half and its port, unboxed off-thread for the arm's
    /// [`MixedTransferControl::join`](crate::stream::MixedTransferControl::join).
    pub fn into_parts(self) -> (MixedTransferAudio, MixedTransferAudioPort<C, R>) {
        self.0.into_parts()
    }
}

/// An audio half and an audio port of different arms, both returned unchanged.
#[must_use]
pub struct MixedBridgeResetForeignPort<C, R>(MixedResetForeignEndpoint<C, R>);

impl<C, R> std::fmt::Debug for MixedBridgeResetForeignPort<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("MixedBridgeResetForeignPort")
            .field(&self.0)
            .finish()
    }
}

impl<C, R> MixedBridgeResetForeignPort<C, R> {
    /// The audio half and the foreign port, unchanged.
    pub fn into_parts(self) -> (MixedTransferAudio, MixedTransferAudioPort<C, R>) {
        (self.0.audio, self.0.port)
    }
}

/// Why the wrapper refused a reset successor, at its offer or at its boundary. The
/// successor comes back unchanged and the active owner renders on as before.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgeResetRefusal {
    /// The wrapper or its active owner is terminally faulted.
    #[error("the mixed reset wrapper or its active owner is terminally faulted")]
    Faulted,
    /// The single secondary slot holds a pending, fading, retired or refused owner.
    #[error("the mixed reset wrapper's secondary slot is occupied")]
    SecondaryOccupied,
    /// The boundary is not a quantum boundary.
    #[error("reset boundary {at} is not a quantum boundary")]
    NotBoundary {
        /// The refused boundary.
        at: SampleTime,
    },
    /// The boundary's quantum has already opened.
    #[error("reset boundary {at} precedes the render clock {clock}")]
    BoundaryPassed {
        /// The refused boundary.
        at: SampleTime,
        /// The active owner's render clock.
        clock: SampleTime,
    },
    /// The end of the fade after the boundary cannot be represented.
    #[error("the fade after reset boundary {at} cannot be represented")]
    FadeUnrepresentable {
        /// The refused boundary.
        at: SampleTime,
    },
    /// Rule 3.2: the active owner's compiled program is running.
    #[error("the active owner's compiled program is running")]
    Running,
    /// A compiled Stop is scheduled and has not applied.
    #[error("a compiled Stop at {stop} is still pending")]
    StopPending {
        /// The pending Stop's boundary.
        stop: SampleTime,
    },
    /// A later Start or seek waits in the active owner's offer slot.
    #[error("a later compiled candidate is pending")]
    OfferPending,
    /// A retired Start or seek waits in the active owner's offer slot; take it with
    /// [`MixedBridgeResetAudio::take_retired_activation`] and collect it before the reset.
    #[error("a retired compiled candidate has not been taken")]
    ActivationUncollected,
    /// The active owner still holds input it would apply after the boundary: a bridged
    /// command not yet served through the seal, queued entries, a parked Panic, an owed
    /// compiled cut, an unpushed result or a retained invalid command.
    #[error("the active owner has live ingress work pending")]
    IngressPending,
    /// The bridge has not sealed forwarding to this successor, or audio already decided
    /// that seal at an earlier boundary.
    #[error("the old bridge transfer is not sealed for this successor")]
    Unsealed,
    /// The bridge accepted a Panic due no later than the boundary's quantum before audio
    /// decided the seal there, so the Panic cancelled the reset: the old owner keeps
    /// serving, the Panic included.
    #[error("a due Panic cancelled the sealed reset")]
    PanicCancelled,
    /// The active owner's live occurrences cannot be ended in one `Reset` group.
    #[error("the active owner's live occurrences cannot be ended: {0}")]
    Live(MixedBridgeResetLiveRefusal),
    /// The successor shares the active owner's epoch or table, or renders another
    /// output shape.
    #[error("the reset successor is not a fresh owner of the same output shape")]
    ForeignSuccessor,
    /// The successor has rendered, adopted or faulted, or holds its own pending
    /// transport, so it is not a freshly armed owner.
    #[error("the reset successor is not freshly armed")]
    SuccessorNotFresh,
    /// The successor's armed transition does not take effect at the boundary.
    #[error("the successor's armed transition takes effect at {effective}, not at {at}")]
    SuccessorBoundary {
        /// Where the successor's armed transition takes effect.
        effective: SampleTime,
        /// The offered boundary.
        at: SampleTime,
    },
}

/// An opaque refusal to end the active owner's live occurrences in one `Reset` group.
/// Every occurrence, hold and queued entry is unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error(transparent)]
pub struct MixedBridgeResetLiveRefusal(MixedResetLiveRefused);

impl From<MixedResetRefused> for MixedBridgeResetRefusal {
    fn from(refused: MixedResetRefused) -> Self {
        use MixedResetRefused as Private;
        match refused {
            Private::Faulted => Self::Faulted,
            Private::SecondaryOccupied => Self::SecondaryOccupied,
            Private::NotBoundary { at } => Self::NotBoundary { at },
            Private::BoundaryPassed { at, clock } => Self::BoundaryPassed { at, clock },
            Private::FadeUnrepresentable { at } => Self::FadeUnrepresentable { at },
            Private::Running => Self::Running,
            Private::StopPending { stop } => Self::StopPending { stop },
            Private::OfferPending => Self::OfferPending,
            Private::ActivationUncollected => Self::ActivationUncollected,
            Private::IngressPending => Self::IngressPending,
            Private::Unsealed => Self::Unsealed,
            Private::PanicCancelled => Self::PanicCancelled,
            Private::Live(live) => Self::Live(MixedBridgeResetLiveRefusal(live)),
            Private::ForeignSuccessor => Self::ForeignSuccessor,
            Private::SuccessorNotFresh => Self::SuccessorNotFresh,
            Private::SuccessorBoundary { effective, at } => {
                Self::SuccessorBoundary { effective, at }
            }
        }
    }
}

/// Why a wrapper callback failed. Every variant except `OutputShape` is terminal: the
/// complete callback and every later one are silent, and both owners and both ports
/// stay in the wrapper for teardown.
#[derive(Clone, Copy, Debug, PartialEq, Error)]
pub enum MixedBridgeResetRenderError {
    /// The output block does not match the active owner's profile; nothing changed.
    #[error("the output block does not match the active owner's profile")]
    OutputShape,
    /// An earlier callback already faulted.
    #[error("the mixed reset wrapper is terminally faulted")]
    Faulted,
    /// The active owner failed, as its arm's callback would.
    #[error("the active mixed owner failed: {0}")]
    Active(MixedTransferRenderError),
    /// The fading old owner failed, including when its arm's control halted.
    #[error("the fading mixed owner failed: {0}")]
    Fading(MixedTransferRenderError),
    /// Defensive: an output window or the prepared fade storage cannot hold a segment,
    /// or a segment would cross the end of the fade.
    #[error("the mixed reset fade storage or alignment is inconsistent")]
    Storage,
    /// The faded old owner still held a live hold, identity, queued entry or sounding
    /// note after its discarded quantum.
    #[error("the faded mixed owner retained a live obligation")]
    Custody,
}

impl From<MixedResetRenderError> for MixedBridgeResetRenderError {
    fn from(error: MixedResetRenderError) -> Self {
        use MixedResetRenderError as Private;
        match error {
            Private::OutputShape => Self::OutputShape,
            Private::Faulted => Self::Faulted,
            Private::Active(error) => Self::Active(error.into()),
            Private::Fading(error) => Self::Fading(error.into()),
            Private::Scratch => Self::Storage,
            Private::Custody => Self::Custody,
        }
    }
}

/// One applied reset: its boundary, the sample its `Reset` group ends the old owner's
/// live occurrences at, and the handles that group ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedBridgeResetReceipt {
    /// The old arm's binding, which names its handles.
    binding: MixedTransferBinding,
    /// The active arm after the reset, for control-side binding verification.
    successor_binding: MixedTransferBinding,
    receipt: MixedResetReceipt,
}

impl MixedBridgeResetReceipt {
    pub(crate) const fn old_binding(&self) -> MixedTransferBinding {
        self.binding
    }

    pub(crate) const fn successor_binding(&self) -> MixedTransferBinding {
        self.successor_binding
    }

    /// The reset boundary `B`: the first sample the old owner served no command at.
    pub const fn at(&self) -> SampleTime {
        self.receipt.at
    }

    /// `B + Q`: the end of the fade, where the `Reset` group ends every old live
    /// occurrence and redeems its hold.
    pub const fn ended_at(&self) -> SampleTime {
        self.receipt.ended_at
    }

    /// Every old live occurrence the `Reset` group ended, in the group's order, named by
    /// the handle the old arm's control returned for its onset.
    pub fn ended(&self) -> impl Iterator<Item = MixedBridgeResetEnd> + '_ {
        let binding = self.binding;
        self.receipt.live.ended().map(move |end| match end {
            MixedResetEnd::Retired(occurrence) => {
                MixedBridgeResetEnd::Retired(MixedKeyUpHandle::of(binding, occurrence))
            }
            MixedResetEnd::Tombstoned(occurrence) => {
                MixedBridgeResetEnd::Tombstoned(MixedKeyUpHandle::of(binding, occurrence))
            }
        })
    }
}

/// How the `Reset` group ended one old live occurrence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedBridgeResetEnd {
    /// The key was up under its pedal: the old owner's sustained cell retired with the
    /// reset, and the old arm's control still holds the handle as sustained.
    Retired(MixedKeyUpHandle),
    /// The key was down: the old owner keeps a `Reset` tombstone, and the old arm's
    /// control still holds the handle open. Its key-up never reaches the old owner.
    Tombstoned(MixedKeyUpHandle),
}

/// What the wrapper's secondary slot holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedBridgeResetState {
    /// No successor and no old owner.
    Empty,
    /// A successor waits for its boundary.
    Pending {
        /// The successor's boundary.
        at: SampleTime,
    },
    /// The old owner is cross-fading out.
    Fading {
        /// The applied reset.
        receipt: MixedBridgeResetReceipt,
        /// Frames of the `Q`-frame fade already delivered.
        faded: FrameCount,
    },
    /// The old owner rendered its discarded quantum, holds no live obligation and waits
    /// to be taken.
    Retired(MixedBridgeResetReceipt),
    /// A successor refused at its boundary waits, unchanged, to be taken.
    Refused {
        /// The successor's boundary.
        at: SampleTime,
        /// Why it was refused there.
        reason: MixedBridgeResetRefusal,
    },
}

/// The retired old owner with its own port, and its receipt.
#[must_use]
pub struct MixedBridgeResetRetired<C, R> {
    owner: MixedBridgeResetOwner<C, R>,
    receipt: MixedBridgeResetReceipt,
}

impl<C, R> std::fmt::Debug for MixedBridgeResetRetired<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedBridgeResetRetired")
            .field("owner", &self.owner)
            .field("receipt", &self.receipt)
            .finish()
    }
}

impl<C, R> MixedBridgeResetRetired<C, R> {
    /// The reset that retired the owner.
    pub const fn receipt(&self) -> MixedBridgeResetReceipt {
        self.receipt
    }

    /// The old owner with its port, for its arm's joined teardown off the audio thread.
    pub fn into_owner(self) -> MixedBridgeResetOwner<C, R> {
        self.owner
    }
}

/// A successor refused at its boundary, unchanged, with its own port.
#[must_use]
pub struct MixedBridgeResetRefused<C, R> {
    owner: MixedBridgeResetOwner<C, R>,
    at: SampleTime,
    reason: MixedBridgeResetRefusal,
}

impl<C, R> std::fmt::Debug for MixedBridgeResetRefused<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedBridgeResetRefused")
            .field("owner", &self.owner)
            .field("at", &self.at)
            .field("reason", &self.reason)
            .finish()
    }
}

impl<C, R> MixedBridgeResetRefused<C, R> {
    /// The successor's boundary.
    pub const fn at(&self) -> SampleTime {
        self.at
    }

    /// Why it was refused there.
    pub const fn reason(&self) -> MixedBridgeResetRefusal {
        self.reason
    }

    /// The successor with its port, for its arm's joined teardown off the audio thread.
    pub fn into_owner(self) -> MixedBridgeResetOwner<C, R> {
        self.owner
    }
}

/// How the active owner took a compiled Stop request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedBridgeStopAccepted {
    /// The Stop applies at its boundary. It cancels a later candidate pending at or
    /// after that boundary, then ends a running program's compiled notes.
    Scheduled,
    /// The program is already stopped and nothing pending interacts with the Stop, so
    /// it changes nothing.
    AlreadyStopped,
}

impl From<MixedStopAccepted> for MixedBridgeStopAccepted {
    fn from(accepted: MixedStopAccepted) -> Self {
        match accepted {
            MixedStopAccepted::Scheduled => Self::Scheduled,
            MixedStopAccepted::AlreadyStopped => Self::AlreadyStopped,
        }
    }
}

/// Why the active owner refused a compiled Stop. A refusal changes nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgeStopRefusal {
    /// The wrapper or its active owner is terminally faulted.
    #[error("the mixed reset wrapper or its active owner is terminally faulted")]
    Faulted,
    /// The Stop is not at a quantum boundary.
    #[error("compiled Stop at {at} is not a quantum boundary")]
    NotBoundary {
        /// The refused boundary.
        at: SampleTime,
    },
    /// The Stop's quantum has already opened.
    #[error("compiled Stop at {at} precedes the first unrendered boundary {clock}")]
    Past {
        /// The refused boundary.
        at: SampleTime,
        /// The active owner's render clock.
        clock: SampleTime,
    },
    /// The Stop is at or before the armed transition, which it cannot cancel.
    #[error("compiled Stop at {at} does not follow the pending activation at {effective}")]
    ActivationPending {
        /// The refused boundary.
        at: SampleTime,
        /// Where the armed transition takes effect.
        effective: SampleTime,
    },
    /// Another Stop is already scheduled.
    #[error("a compiled Stop at {pending} is already scheduled")]
    Pending {
        /// The scheduled Stop's boundary.
        pending: SampleTime,
    },
}

impl From<MixedStopRefused> for MixedBridgeStopRefusal {
    fn from(refused: MixedStopRefused) -> Self {
        match refused {
            MixedStopRefused::Faulted => Self::Faulted,
            MixedStopRefused::NotBoundary { at } => Self::NotBoundary { at },
            MixedStopRefused::Past { at, clock } => Self::Past { at, clock },
            MixedStopRefused::ActivationPending { at, effective } => {
                Self::ActivationPending { at, effective }
            }
            MixedStopRefused::Pending { pending } => Self::Pending { pending },
        }
    }
}

/// The reset wrapper's audio half: the active host-armed owner with its own port, and one
/// secondary slot for a pending successor, the fading or retired old owner, or a refused
/// successor. Send it to the callback thread.
///
/// The active owner is served only through its own port. Each callback publishes each
/// owner's audio boundary for its bridge's peer fence: the active owner's clock, and `B`
/// for the old owner, which serves no command from `B`.
#[must_use]
pub struct MixedBridgeResetAudio<C, R> {
    inner: MixedResetTransferAudio<C, R>,
}

impl<C, R> std::fmt::Debug for MixedBridgeResetAudio<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedBridgeResetAudio")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<C, R> MixedBridgeResetAudio<C, R> {
    /// The active owner's cumulative publication observations. Reset does not merge
    /// independent owners' peaks; the retiring owner remains separately observable.
    pub fn active_publication_usage(&self) -> MixedTransferPublicationUsage {
        MixedTransferPublicationUsage::of(self.inner.pair.active())
    }

    /// The secondary owner's observations, whether pending, fading, retired or refused.
    /// [`Self::state`] identifies which lifecycle state these observations belong to.
    pub fn secondary_publication_usage(&self) -> Option<MixedTransferPublicationUsage> {
        let owner = match &self.inner.pair.secondary {
            MixedResetSlot::Empty => return None,
            MixedResetSlot::Pending { successor, .. }
            | MixedResetSlot::Refused { successor, .. } => successor,
            MixedResetSlot::Fading { old, .. } | MixedResetSlot::Retired { old, .. } => old,
        };
        Some(MixedTransferPublicationUsage::of(owner))
    }

    /// Resource observations for the active owner, kept separate from the secondary.
    ///
    /// # Errors
    /// A counter or storage amount cannot be represented.
    pub fn active_resource_usage(
        &self,
    ) -> Result<
        super::super::transfer::MixedTransferResourceUsage,
        crate::render::RehearsalUsageError,
    > {
        super::super::transfer::MixedTransferResourceUsage::of(self.inner.pair.active())
    }

    /// Resource observations for the pending, fading, retired or refused owner.
    /// `None` means the secondary slot is empty. Inspect `state()` for its lifecycle.
    ///
    /// # Errors
    /// A counter or storage amount cannot be represented.
    pub fn secondary_resource_usage(
        &self,
    ) -> Option<
        Result<
            super::super::transfer::MixedTransferResourceUsage,
            crate::render::RehearsalUsageError,
        >,
    > {
        let owner = match &self.inner.pair.secondary {
            MixedResetSlot::Empty => return None,
            MixedResetSlot::Pending { successor, .. }
            | MixedResetSlot::Refused { successor, .. } => successor,
            MixedResetSlot::Fading { old, .. } | MixedResetSlot::Retired { old, .. } => old,
        };
        Some(super::super::transfer::MixedTransferResourceUsage::of(
            owner,
        ))
    }

    /// Take one host-armed owner as the active owner, off-thread. This prepares the
    /// fade's one-quantum scratch.
    pub fn new(active: MixedBridgeResetOwner<C, R>) -> Self {
        Self {
            inner: MixedResetTransferAudio::new(active.0),
        }
    }

    /// Schedule the active owner's compiled-scoped Stop at the quantum boundary `at`,
    /// applied before that quantum's compiled events in every callback partition. Live
    /// notes keep sounding. A reset is refused until the Stop has applied (rule 3.2).
    /// Callable between callbacks on the audio thread; it allocates and frees nothing.
    ///
    /// # Errors
    ///
    /// A faulted wrapper or owner, a boundary that is not a later quantum boundary after
    /// the armed transition, or a Stop already scheduled.
    pub fn schedule_stop(
        &mut self,
        at: SampleTime,
    ) -> Result<MixedBridgeStopAccepted, MixedBridgeStopRefusal> {
        if self.inner.fault().is_some() {
            return Err(MixedBridgeStopRefusal::Faulted);
        }
        self.inner
            .pair
            .active_mut()
            .schedule_compiled_stop(at)
            .map(MixedBridgeStopAccepted::from)
            .map_err(MixedBridgeStopRefusal::from)
    }

    /// Accept one later Start or seek for the active owner into its single offer slot, or
    /// give it back whole, with the same binding and slot checks as
    /// [`MixedTransferAudio::offer_activation`]. After a committed reset only the
    /// successor's control prepares offers this accepts; an offer of an earlier owner is
    /// foreign to the successor's transfer even when it names the same superseded
    /// sequence. While a reset successor is pending the offer is refused, so no candidate
    /// enters the slot the reset requires empty at its boundary. Callable between
    /// callbacks on the audio thread: it moves one box and allocates, locks and frees
    /// nothing.
    ///
    /// # Errors
    ///
    /// A faulted wrapper, a pending reset, or any refusal of the active owner, with the
    /// candidate unchanged.
    pub fn offer_activation(
        &mut self,
        offer: MixedActivationOffer,
    ) -> Result<(), (MixedActivationRefusal, MixedActivationOffer)> {
        let refused = if self.inner.fault().is_some() {
            Some(MixedActivationRefusal::Faulted)
        } else if let MixedResetSlot::Pending { at, .. } = &self.inner.pair.secondary {
            Some(MixedActivationRefusal::ResetPending { at: *at })
        } else {
            None
        };
        let active = self.inner.pair.active_mut();
        match refused {
            Some(reason) => Err(active.refuse_bound_activation(reason, offer)),
            None => active.offer_bound_activation(offer),
        }
    }

    /// Move the active owner's retired candidate and its receipt out of its slot, for
    /// its own control to collect off-thread. Callable between callbacks on the audio
    /// thread: it allocates and frees nothing.
    pub fn take_retired_activation(&mut self) -> Option<MixedActivationRetired> {
        self.inner.pair.active_mut().take_bound_activation()
    }

    /// What the active owner's offer slot holds.
    pub fn activation(&self) -> MixedActivationSlot {
        self.inner.pair.active().bound_activation()
    }

    /// Accept a freshly armed successor, boxed with its own arm's port, to take over at
    /// the quantum boundary `at`, or give it back. The active owner's compiled program
    /// must be stopped with nothing left to apply after `at`, and the successor must be
    /// an unrendered arm of another epoch and table whose armed transition takes effect
    /// at exactly `at`. The checks run again when `at` opens. Callable between callbacks
    /// on the audio thread: it moves one box and the port and allocates and frees
    /// nothing.
    ///
    /// # Errors
    ///
    /// Why the successor was refused, with the successor unchanged.
    pub fn offer_reset(
        &mut self,
        successor: MixedBridgeResetOwner<C, R>,
        at: SampleTime,
    ) -> Result<(), (MixedBridgeResetRefusal, MixedBridgeResetOwner<C, R>)> {
        self.inner
            .offer_reset(successor.0, at)
            .map_err(|(reason, owner)| (reason.into(), MixedBridgeResetOwner(owner)))
    }

    /// Take back a successor whose boundary has not opened, with its port. Allocates and
    /// frees nothing.
    pub fn withdraw_pending(&mut self) -> Option<MixedBridgeResetOwner<C, R>> {
        self.inner.withdraw_pending().map(MixedBridgeResetOwner)
    }

    /// Take back the retired old owner with its port and receipt, emptying the slot for
    /// the next reset. Allocates and frees nothing; the caller joins it off the audio
    /// thread.
    pub fn take_retired(&mut self) -> Option<MixedBridgeResetRetired<C, R>> {
        let successor_binding = self.inner.pair.active_endpoint.port.binding;
        self.inner.take_retired().map(|retired| {
            let receipt = MixedBridgeResetReceipt {
                binding: retired.owner.endpoint.port.binding,
                successor_binding,
                receipt: retired.receipt,
            };
            MixedBridgeResetRetired {
                owner: MixedBridgeResetOwner(retired.owner),
                receipt,
            }
        })
    }

    /// Take back a successor refused at its boundary, with its port, emptying the slot.
    /// Allocates and frees nothing.
    pub fn take_refused(&mut self) -> Option<MixedBridgeResetRefused<C, R>> {
        self.inner
            .take_refused()
            .map(|refused| MixedBridgeResetRefused {
                owner: MixedBridgeResetOwner(refused.owner),
                at: refused.at,
                reason: refused.reason.into(),
            })
    }

    /// What the secondary slot holds.
    pub fn state(&self) -> MixedBridgeResetState {
        match &self.inner.pair.secondary {
            MixedResetSlot::Empty => MixedBridgeResetState::Empty,
            MixedResetSlot::Pending { at, .. } => MixedBridgeResetState::Pending { at: *at },
            MixedResetSlot::Fading {
                endpoint,
                receipt,
                faded,
                ..
            } => MixedBridgeResetState::Fading {
                receipt: MixedBridgeResetReceipt {
                    binding: endpoint.port.binding,
                    successor_binding: self.inner.pair.active_endpoint.port.binding,
                    receipt: *receipt,
                },
                faded: FrameCount::new(u64::try_from(*faded).unwrap_or(u64::MAX)),
            },
            MixedResetSlot::Retired {
                endpoint, receipt, ..
            } => MixedBridgeResetState::Retired(MixedBridgeResetReceipt {
                binding: endpoint.port.binding,
                successor_binding: self.inner.pair.active_endpoint.port.binding,
                receipt: *receipt,
            }),
            MixedResetSlot::Refused { at, reason, .. } => MixedBridgeResetState::Refused {
                at: *at,
                reason: (*reason).into(),
            },
        }
    }

    /// The terminal fault the wrapper recorded, if any.
    pub fn fault(&self) -> Option<MixedBridgeResetRenderError> {
        self.inner.fault().map(MixedBridgeResetRenderError::from)
    }

    /// Both owners with their own ports, off-thread, for each arm's joined teardown after
    /// the callback is joined: the active owner, and whatever [`Self::state`] reported in
    /// the secondary slot.
    pub fn into_owners(
        self,
    ) -> (
        MixedBridgeResetOwner<C, R>,
        Option<MixedBridgeResetOwner<C, R>>,
    ) {
        let (active, secondary) = self.inner.into_owners();
        (
            MixedBridgeResetOwner(active),
            secondary.map(MixedBridgeResetOwner),
        )
    }
}

impl<C, R> MixedBridgeResetAudio<C, R>
where
    C: MixedTransferConsumer<MixedTransferCommand>,
    R: MixedTransferProducer<MixedTransferResult>,
{
    /// Render one callback: the active owner through its own port and, from a reset's
    /// boundary, the fading owner through none, cross-faded by each frame's offset from
    /// the boundary, so every callback partition delivers the same samples.
    ///
    /// # Errors
    ///
    /// A wrong output shape, or a terminal fault that silenced the complete block.
    pub fn render(&mut self, output: AudioBlockMut<'_>) -> Result<(), MixedBridgeResetRenderError> {
        self.inner
            .render(output)
            .map_err(MixedBridgeResetRenderError::from)
    }
}
