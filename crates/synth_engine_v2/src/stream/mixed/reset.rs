//! Stage 7's private whole-owner reset of the mixed audio owner (draft Item 3).
//!
//! A reset replaces the active [`MixedOneShotAudio`] with a fresh owner, epoch and table
//! at one quantum boundary `B`, and only while the active owner's compiled transport is
//! stopped through the legacy entry (rule 3.2). The explicit running entry instead
//! cuts the old compiled program with a real Stop at `B + Q`. From `B` the old owner
//! takes no input. It renders the same frames as the successor, and the two cross-fade over exactly `Q` frames by each frame's
//! offset from `B`, so every callback partition delivers the same samples. At `B + Q` one
//! `Reset` release group ends every old live occurrence and redeems its hold. The old
//! owner then renders one discarded quantum, in which its registry releases those
//! identities, and retires into the pair's single secondary slot. It is taken back from
//! there and destroyed off the audio thread (rule 3.3, as ADR-0009 and ADR-0074 do).
//!
//! The pair holds the active owner and at most one secondary: a pending successor, the
//! fading or retired old owner, or a refused successor (rule 3.6). The legacy entry
//! refuses a running old program. Both entries refuse pending transport or ingress
//! work, at offer or at `B`; neither waits for quiescence. In legacy mode the successor
//! adopts its armed transition at `B` with compiled transport stopped at its anchor's
//! position, so it publishes only scoped
//! restoration until a Start is offered to it. The explicit running entry keeps the
//! fresh successor running at its exact armed anchor; neither entry migrates state.
//!
//! Every owner in the pair carries its endpoint: the command source bound to it, which
//! moves with it through every slot transition. The active owner is served only through
//! its own endpoint, and from `B` the fading owner through none. The private rehearsal's
//! endpoint is the owner's own rings. With the transfer rehearsal,
//! [`MixedResetTransferAudio`] lends each host-armed owner the audio port its own arm
//! returned, and [`MixedBridgeResetAudio`] is its public face. Giving the old owner's held
//! keys tombstones in the successor's merger remains the bridge's integration.

#[cfg(feature = "mixed-transfer-rehearsal")]
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use thiserror::Error;

#[cfg(all(test, feature = "simulated-ingress"))]
use super::EmbeddedQueues;
#[cfg(feature = "mixed-transfer-rehearsal")]
use super::transfer::{
    MixedTransferAudio, MixedTransferAudioPort, MixedTransferCommand, MixedTransferConsumer,
    MixedTransferProducer, MixedTransferResult,
};
use super::{
    MixedOneShotAudio, MixedOneShotRenderError,
    occurrence::{MixedResetEnded, MixedResetLiveRefused},
};
#[cfg(feature = "mixed-transfer-rehearsal")]
use crate::render::AudioBlockMut;
use crate::time::{FrameCount, QUANTUM_FRAMES, QuantumOffset, SampleTime};

#[cfg(feature = "mixed-transfer-rehearsal")]
mod host;
mod hot;
#[cfg(test)]
mod tests;
#[cfg(all(test, feature = "mixed-transfer-rehearsal"))]
mod transfer_tests;

#[cfg(feature = "mixed-transfer-rehearsal")]
pub use host::{
    MixedBridgeCompiledCut, MixedBridgeResetAudio, MixedBridgeResetEnd,
    MixedBridgeResetForeignPort, MixedBridgeResetLiveRefusal, MixedBridgeResetOwner,
    MixedBridgeResetReceipt, MixedBridgeResetRefusal, MixedBridgeResetRefused,
    MixedBridgeResetRenderError, MixedBridgeResetRetired, MixedBridgeResetState,
    MixedBridgeStopAccepted, MixedBridgeStopRefusal,
};

/// The endpoint a pair names by default: the private rehearsal's own rings. Outside that
/// rehearsal every pair names its endpoint, so the default is uninhabited.
#[cfg(all(test, feature = "simulated-ingress"))]
type MixedResetDefaultEndpoint = EmbeddedQueues;
#[cfg(not(all(test, feature = "simulated-ingress")))]
type MixedResetDefaultEndpoint = core::convert::Infallible;

/// Why the pair refused a reset successor, at its offer or at its boundary. The
/// successor comes back unchanged and the active owner renders on as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(super) enum MixedResetRefused {
    #[error("the mixed reset pair or its active owner is terminally faulted")]
    Faulted,
    /// The single secondary slot holds a pending, fading, retired or refused owner.
    #[error("the mixed reset pair's secondary slot is occupied")]
    SecondaryOccupied,
    #[error("reset boundary {at} is not a quantum boundary")]
    NotBoundary { at: SampleTime },
    #[error("reset boundary {at} precedes the render clock {clock}")]
    BoundaryPassed { at: SampleTime, clock: SampleTime },
    #[error("the fade after reset boundary {at} cannot be represented")]
    FadeUnrepresentable { at: SampleTime },
    /// The legacy entry preserves draft row 3.2's stopped-only first shape.
    #[error("the active owner's compiled program is running")]
    Running,
    #[cfg(feature = "mixed-transfer-rehearsal")]
    #[error("the active owner's compiled program is not running")]
    NotRunning,
    #[cfg(feature = "mixed-transfer-rehearsal")]
    #[error("the active owner has not adopted its compiled program")]
    NotAdopted,
    #[cfg(feature = "mixed-transfer-rehearsal")]
    #[error("the successor's running schedule does not start at the reset boundary")]
    SuccessorSchedule,
    #[cfg(feature = "mixed-transfer-rehearsal")]
    #[error("the retirement Stop cannot be admitted")]
    RetirementStop,
    #[error("a compiled Stop at {stop} is still pending")]
    StopPending { stop: SampleTime },
    #[cfg(feature = "mixed-transfer-rehearsal")]
    #[error("an in-place plan continuation is pending or uncollected")]
    ContinuationPending,
    /// A later Start or seek waits in the active owner's offer slot.
    #[error("a later compiled candidate is pending")]
    OfferPending,
    /// A retired Start or seek, with its receipt, waits in the active owner's offer slot
    /// for the host to take it. Past `B` its control would no longer own the transfer,
    /// so the reset waits until the slot is empty.
    #[error("a retired compiled candidate has not been taken")]
    ActivationUncollected,
    /// A bridged command still unserved through the reset seal, queued entries, a parked
    /// Panic, an owed compiled Panic cut or an unpushed result: input the old owner would
    /// still apply after `B`.
    #[error("the active owner has live ingress work pending")]
    IngressPending,
    /// A bridge-owned transfer was not sealed for this successor.
    #[cfg(feature = "mixed-transfer-rehearsal")]
    #[error("the old bridge transfer is not sealed for this successor")]
    Unsealed,
    /// The bridge's due Panic won the seal before audio decided it at `B`, so the old
    /// owner keeps serving, the Panic included.
    #[cfg(feature = "mixed-transfer-rehearsal")]
    #[error("a due Panic cancelled the sealed reset")]
    PanicCancelled,
    #[error("the active owner's live occurrences cannot be ended: {0}")]
    Live(MixedResetLiveRefused),
    /// The successor shares the active owner's epoch or table, or renders another
    /// output shape.
    #[error("the reset successor is not a fresh owner of the same output shape")]
    ForeignSuccessor,
    /// The successor has rendered, adopted or faulted, or holds its own pending
    /// transport, so it is not a freshly armed owner.
    #[error("the reset successor is not freshly armed")]
    SuccessorNotFresh,
    #[error("the successor's armed transition takes effect at {effective}, not at {at}")]
    SuccessorBoundary {
        effective: SampleTime,
        at: SampleTime,
    },
}

/// Failure of the reset pair. All but output shape are terminal: the complete callback
/// is silenced and both owners stay in the pair for off-thread teardown.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub(super) enum MixedResetRenderError {
    #[error("mixed reset pair output does not match the active owner's profile")]
    OutputShape,
    #[error("mixed reset pair is terminally faulted")]
    Faulted,
    #[error("active mixed owner failed: {0}")]
    Active(MixedOneShotRenderError),
    #[error("fading mixed owner failed: {0}")]
    Fading(MixedOneShotRenderError),
    /// Defensive check: an output window or the prepared fade scratch cannot hold a
    /// segment, or a segment would cross the end of the fade.
    #[error("mixed reset fade storage or alignment is inconsistent")]
    Scratch,
    /// The faded owner still holds a live hold, identity, queued entry or sounding note
    /// after its discarded quantum.
    #[error("the faded mixed owner retained a live obligation")]
    Custody,
}

/// One applied reset: its boundary, the sample its `Reset` group ends the old owner's
/// live occurrences at, and which handles that group ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(super) struct MixedResetReceipt {
    pub(super) at: SampleTime,
    pub(super) ended_at: SampleTime,
    pub(super) live: MixedResetEnded,
    pub(super) compiled_cut: Option<MixedCompiledCut>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MixedCompiledCut {
    pub(super) stop_at: SampleTime,
    pub(super) old_position_at_b: Option<crate::time::PlanPosition>,
    pub(super) old_pass: Option<crate::looping::LoopPassId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MixedResetMode {
    Stopped,
    #[cfg(feature = "mixed-transfer-rehearsal")]
    Running,
}

/// One owner and the endpoint bound to it.
pub(super) type MixedResetOwner<E> = (Box<MixedOneShotAudio>, E);

/// The pair's single secondary slot. Each owner in it keeps its endpoint.
#[derive(Debug, Default)]
enum MixedResetSlot<E> {
    #[default]
    Empty,
    /// A successor waiting for its boundary.
    Pending {
        successor: Box<MixedOneShotAudio>,
        endpoint: E,
        at: SampleTime,
        mode: MixedResetMode,
    },
    /// The old owner, cross-fading out; `faded` frames of the `Q`-frame fade are done.
    Fading {
        old: Box<MixedOneShotAudio>,
        endpoint: E,
        receipt: MixedResetReceipt,
        faded: usize,
    },
    /// The old owner after its discarded quantum, holding no live obligation.
    Retired {
        old: Box<MixedOneShotAudio>,
        endpoint: E,
        receipt: MixedResetReceipt,
    },
    /// A successor refused at its boundary, unchanged.
    Refused {
        successor: Box<MixedOneShotAudio>,
        endpoint: E,
        at: SampleTime,
        reason: MixedResetRefused,
    },
}

/// What the secondary slot holds, for a report.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MixedResetState {
    Empty,
    Pending {
        at: SampleTime,
    },
    /// `faded` frames of the `Q`-frame fade are delivered.
    Fading {
        receipt: MixedResetReceipt,
        faded: FrameCount,
    },
    Retired(MixedResetReceipt),
    Refused {
        at: SampleTime,
        reason: MixedResetRefused,
    },
}

/// A retired old owner, its endpoint and its receipt, taken back for off-thread
/// destruction.
#[derive(Debug)]
#[must_use]
pub(super) struct MixedRetiredOwner<E> {
    pub(super) owner: Box<MixedOneShotAudio>,
    /// Kept for the old owner's joined teardown.
    #[cfg_attr(
        not(feature = "mixed-transfer-rehearsal"),
        allow(dead_code, reason = "only a host endpoint is read back")
    )]
    pub(super) endpoint: E,
    pub(super) receipt: MixedResetReceipt,
}

/// A successor the pair refused at its boundary, returned unchanged with its endpoint.
#[derive(Debug)]
#[must_use]
pub(super) struct MixedRefusedSuccessor<E> {
    pub(super) successor: Box<MixedOneShotAudio>,
    #[cfg_attr(
        not(feature = "mixed-transfer-rehearsal"),
        allow(dead_code, reason = "only a host endpoint is read back")
    )]
    pub(super) endpoint: E,
    pub(super) at: SampleTime,
    pub(super) reason: MixedResetRefused,
}

/// The active mixed audio owner with its endpoint and its one secondary slot, with the
/// fade's scratch.
#[derive(Debug)]
#[must_use]
pub(super) struct MixedResetPair<E = MixedResetDefaultEndpoint> {
    active: Box<MixedOneShotAudio>,
    /// The command source bound to the active owner.
    active_endpoint: E,
    secondary: MixedResetSlot<E>,
    /// One quantum of the active owner's channel layout: the fading owner's samples.
    scratch: Box<[f32]>,
    fault: Option<MixedResetRenderError>,
}

#[cfg(all(test, feature = "simulated-ingress"))]
impl MixedResetPair {
    /// Take one armed owner, which serves its own rings, as the active owner, off-thread.
    pub(super) fn new(active: MixedOneShotAudio) -> Self {
        Self::with_endpoint(Box::new(active), EmbeddedQueues)
    }

    /// [`Self::offer_with`] for a successor that serves its own rings.
    pub(super) fn offer_reset(
        &mut self,
        successor: Box<MixedOneShotAudio>,
        at: SampleTime,
    ) -> Result<(), (MixedResetRefused, Box<MixedOneShotAudio>)> {
        self.offer_with(successor, EmbeddedQueues, at)
            .map_err(|(reason, successor, _)| (reason, successor))
    }

    /// [`Self::withdraw_with`] without the successor's own rings.
    pub(super) fn withdraw_pending(&mut self) -> Option<Box<MixedOneShotAudio>> {
        self.withdraw_with().map(|(successor, _)| successor)
    }

    /// Both owners, off-thread, for teardown after the callback is joined.
    pub(super) fn into_owners(self) -> (Box<MixedOneShotAudio>, Option<Box<MixedOneShotAudio>>) {
        let ((active, _), secondary) = self.into_parts();
        (active, secondary.map(|(owner, _)| owner))
    }
}

impl<E> MixedResetPair<E> {
    /// Take one armed owner and the endpoint bound to it as the active owner, off-thread.
    /// The scratch holds one quantum, which every fade segment and discarded-quantum
    /// chunk fits.
    pub(super) fn with_endpoint(active: Box<MixedOneShotAudio>, endpoint: E) -> Self {
        let channels = active.audio.renderer.plan().channel_layout().channels();
        let samples = channels.saturating_mul(QUANTUM_FRAMES as usize);
        Self {
            active,
            active_endpoint: endpoint,
            secondary: MixedResetSlot::Empty,
            scratch: vec![0.0; samples].into_boxed_slice(),
            fault: None,
        }
    }

    /// Accept a freshly armed successor, with the endpoint bound to it, that takes over at
    /// the quantum boundary `at`, or give both back. The checks run again when `at` opens.
    /// Callable between callbacks: it moves one box and the endpoint and allocates and
    /// frees nothing.
    pub(super) fn offer_with(
        &mut self,
        successor: Box<MixedOneShotAudio>,
        endpoint: E,
        at: SampleTime,
    ) -> Result<(), (MixedResetRefused, Box<MixedOneShotAudio>, E)> {
        self.offer_with_mode(successor, endpoint, at, MixedResetMode::Stopped)
    }

    fn offer_with_mode(
        &mut self,
        successor: Box<MixedOneShotAudio>,
        endpoint: E,
        at: SampleTime,
        mode: MixedResetMode,
    ) -> Result<(), (MixedResetRefused, Box<MixedOneShotAudio>, E)> {
        let clock = self.active.audio.renderer.clock();
        let refused = if self.fault.is_some() {
            Some(MixedResetRefused::Faulted)
        } else if !matches!(self.secondary, MixedResetSlot::Empty) {
            Some(MixedResetRefused::SecondaryOccupied)
        } else if at.quantum_offset() != QuantumOffset::ZERO {
            Some(MixedResetRefused::NotBoundary { at })
        } else if at < clock {
            Some(MixedResetRefused::BoundaryPassed { at, clock })
        } else if at.checked_add(FrameCount::QUANTUM).is_err() {
            Some(MixedResetRefused::FadeUnrepresentable { at })
        } else {
            hot::reset_refusal(&self.active, &successor, at, mode)
        };
        if let Some(reason) = refused {
            return Err((reason, successor, endpoint));
        }
        self.secondary = MixedResetSlot::Pending {
            successor,
            endpoint,
            at,
            mode,
        };
        Ok(())
    }

    /// Take back a successor whose boundary has not opened, with its endpoint. Allocates
    /// and frees nothing.
    pub(super) fn withdraw_with(&mut self) -> Option<MixedResetOwner<E>> {
        match core::mem::take(&mut self.secondary) {
            MixedResetSlot::Pending {
                successor,
                endpoint,
                ..
            } => Some((successor, endpoint)),
            other => {
                self.secondary = other;
                None
            }
        }
    }

    /// Take back the retired old owner and its endpoint, emptying the slot for the next
    /// reset. The caller destroys it off the audio thread, after collecting its control
    /// half.
    pub(super) fn take_retired(&mut self) -> Option<MixedRetiredOwner<E>> {
        match core::mem::take(&mut self.secondary) {
            MixedResetSlot::Retired {
                old,
                endpoint,
                receipt,
            } => Some(MixedRetiredOwner {
                owner: old,
                endpoint,
                receipt,
            }),
            other => {
                self.secondary = other;
                None
            }
        }
    }

    /// Take back a successor refused at its boundary, emptying the slot.
    pub(super) fn take_refused(&mut self) -> Option<MixedRefusedSuccessor<E>> {
        match core::mem::take(&mut self.secondary) {
            MixedResetSlot::Refused {
                successor,
                endpoint,
                at,
                reason,
            } => Some(MixedRefusedSuccessor {
                successor,
                endpoint,
                at,
                reason,
            }),
            other => {
                self.secondary = other;
                None
            }
        }
    }

    #[cfg(test)]
    pub(super) fn state(&self) -> MixedResetState {
        match &self.secondary {
            MixedResetSlot::Empty => MixedResetState::Empty,
            MixedResetSlot::Pending { at, .. } => MixedResetState::Pending { at: *at },
            MixedResetSlot::Fading { receipt, faded, .. } => MixedResetState::Fading {
                receipt: *receipt,
                faded: FrameCount::new(*faded as u64),
            },
            MixedResetSlot::Retired { receipt, .. } => MixedResetState::Retired(*receipt),
            MixedResetSlot::Refused { at, reason, .. } => MixedResetState::Refused {
                at: *at,
                reason: *reason,
            },
        }
    }

    pub(super) const fn fault(&self) -> Option<MixedResetRenderError> {
        self.fault
    }

    /// The active owner, for scheduling its own transport between callbacks.
    pub(super) fn active_mut(&mut self) -> &mut MixedOneShotAudio {
        &mut self.active
    }

    /// The owner in the secondary slot, whatever its state.
    #[cfg(test)]
    pub(super) fn secondary(&self) -> Option<&MixedOneShotAudio> {
        let owner: &MixedOneShotAudio = match &self.secondary {
            MixedResetSlot::Empty => return None,
            MixedResetSlot::Pending { successor, .. }
            | MixedResetSlot::Refused { successor, .. } => successor,
            MixedResetSlot::Fading { old, .. } | MixedResetSlot::Retired { old, .. } => old,
        };
        Some(owner)
    }

    /// Both owners with their endpoints, off-thread, for teardown after the callback is
    /// joined.
    pub(super) fn into_parts(self) -> (MixedResetOwner<E>, Option<MixedResetOwner<E>>) {
        let secondary = match self.secondary {
            MixedResetSlot::Empty => None,
            MixedResetSlot::Pending {
                successor,
                endpoint,
                ..
            }
            | MixedResetSlot::Refused {
                successor,
                endpoint,
                ..
            } => Some((successor, endpoint)),
            MixedResetSlot::Fading { old, endpoint, .. }
            | MixedResetSlot::Retired { old, endpoint, .. } => Some((old, endpoint)),
        };
        ((self.active, self.active_endpoint), secondary)
    }
}

/// One host-armed owner's endpoint: the audio port its own arm returned, and the audio
/// boundary its callbacks publish for the bridge's peer fence.
#[cfg(feature = "mixed-transfer-rehearsal")]
#[must_use]
pub(super) struct MixedResetEndpoint<C, R> {
    port: MixedTransferAudioPort<C, R>,
    boundary: Arc<AtomicU64>,
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> std::fmt::Debug for MixedResetEndpoint<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedResetEndpoint")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// One host-armed owner boxed with the audio port its own arm returned, prepared
/// off-thread for the reset wrapper.
#[cfg(feature = "mixed-transfer-rehearsal")]
#[must_use]
pub(super) struct MixedResetTransferOwner<C, R> {
    owner: Box<MixedOneShotAudio>,
    endpoint: MixedResetEndpoint<C, R>,
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> std::fmt::Debug for MixedResetTransferOwner<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedResetTransferOwner")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// An audio half and a port of different arms, both returned unchanged.
#[cfg(feature = "mixed-transfer-rehearsal")]
#[must_use]
pub(super) struct MixedResetForeignEndpoint<C, R> {
    pub(super) audio: MixedTransferAudio,
    pub(super) port: MixedTransferAudioPort<C, R>,
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> std::fmt::Debug for MixedResetForeignEndpoint<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedResetForeignEndpoint")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> MixedResetTransferOwner<C, R> {
    /// Box `audio` with the audio port of the same arm, off-thread. A port bound to
    /// another owner is refused, and both come back unchanged.
    ///
    /// # Errors
    ///
    /// The port's binding is not the owner's.
    pub(super) fn new(
        audio: MixedTransferAudio,
        port: MixedTransferAudioPort<C, R>,
    ) -> Result<Self, Box<MixedResetForeignEndpoint<C, R>>> {
        let (owner, boundary) = audio.into_reset_parts();
        if owner.transfer_binding != Some(port.binding) {
            return Err(Box::new(MixedResetForeignEndpoint {
                audio: MixedTransferAudio::from_reset_parts(owner, boundary),
                port,
            }));
        }
        Ok(Self {
            owner: Box::new(owner),
            endpoint: MixedResetEndpoint { port, boundary },
        })
    }

    #[cfg(test)]
    pub(super) fn owner(&self) -> &MixedOneShotAudio {
        &self.owner
    }

    /// The audio half and its port, unboxed off-thread for the arm's joined teardown.
    pub(super) fn into_parts(self) -> (MixedTransferAudio, MixedTransferAudioPort<C, R>) {
        let Self {
            owner,
            endpoint: MixedResetEndpoint { port, boundary },
        } = self;
        (MixedTransferAudio::from_reset_parts(*owner, boundary), port)
    }

    fn from_parts((owner, endpoint): MixedResetOwner<MixedResetEndpoint<C, R>>) -> Self {
        Self { owner, endpoint }
    }
}

/// A retired old owner with its port, and its receipt.
#[cfg(feature = "mixed-transfer-rehearsal")]
#[derive(Debug)]
#[must_use]
pub(super) struct MixedResetRetiredTransfer<C, R> {
    pub(super) owner: MixedResetTransferOwner<C, R>,
    pub(super) receipt: MixedResetReceipt,
}

/// A successor refused at its boundary, with its port, unchanged.
#[cfg(feature = "mixed-transfer-rehearsal")]
#[derive(Debug)]
#[must_use]
pub(super) struct MixedResetRefusedTransfer<C, R> {
    pub(super) owner: MixedResetTransferOwner<C, R>,
    pub(super) at: SampleTime,
    pub(super) reason: MixedResetRefused,
}

/// The reset pair over owners armed into host-owned transfers (ADR-0078).
///
/// Each owner keeps the audio port its own arm returned, so the active owner is served
/// only through its own port and the binding check inside it still runs first. From `B`
/// the fading old owner serves no port, so a command still in the old port at `B` is
/// never served. The old port stays with the old owner, and that arm's joined teardown
/// names such a command unserved. For a bridge-owned transfer, the reset seal instead
/// refuses the offer until audio has served every command minted before the seal, and
/// audio decides the seal once at `B` with one compare-and-swap that the bridge's due
/// Panic races. A Panic that wins first refuses the successor as `PanicCancelled`. A
/// direct host-owned transfer has no seal, so its unserved commands do not refuse the
/// reset. Every terminal fault silences the complete callback and keeps both owners and
/// both ports in the pair.
#[cfg(feature = "mixed-transfer-rehearsal")]
#[must_use]
pub(super) struct MixedResetTransferAudio<C, R> {
    pair: MixedResetPair<MixedResetEndpoint<C, R>>,
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> std::fmt::Debug for MixedResetTransferAudio<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedResetTransferAudio")
            .field("pair", &self.pair)
            .finish()
    }
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> MixedResetTransferAudio<C, R> {
    /// Take one host-armed owner as the active owner, off-thread.
    pub(super) fn new(active: MixedResetTransferOwner<C, R>) -> Self {
        Self {
            pair: MixedResetPair::with_endpoint(active.owner, active.endpoint),
        }
    }

    /// [`MixedResetPair::offer_with`] for a host-armed successor and its port. Allocates
    /// and frees nothing.
    pub(super) fn offer_reset(
        &mut self,
        successor: MixedResetTransferOwner<C, R>,
        at: SampleTime,
    ) -> Result<(), (MixedResetRefused, MixedResetTransferOwner<C, R>)> {
        let MixedResetTransferOwner { owner, endpoint } = successor;
        self.pair
            .offer_with(owner, endpoint, at)
            .map_err(|(reason, owner, endpoint)| {
                (reason, MixedResetTransferOwner { owner, endpoint })
            })
    }

    pub(super) fn offer_running_reset(
        &mut self,
        successor: MixedResetTransferOwner<C, R>,
        at: SampleTime,
    ) -> Result<(), (MixedResetRefused, MixedResetTransferOwner<C, R>)> {
        let MixedResetTransferOwner { owner, endpoint } = successor;
        self.pair
            .offer_with_mode(owner, endpoint, at, MixedResetMode::Running)
            .map_err(|(reason, owner, endpoint)| {
                (reason, MixedResetTransferOwner { owner, endpoint })
            })
    }

    /// Take back a successor whose boundary has not opened, with its port.
    pub(super) fn withdraw_pending(&mut self) -> Option<MixedResetTransferOwner<C, R>> {
        self.pair
            .withdraw_with()
            .map(MixedResetTransferOwner::from_parts)
    }

    /// Take back the retired old owner with its port. Allocates and frees nothing; the
    /// caller unboxes and joins it off the audio thread.
    pub(super) fn take_retired(&mut self) -> Option<MixedResetRetiredTransfer<C, R>> {
        self.pair
            .take_retired()
            .map(|retired| MixedResetRetiredTransfer {
                owner: MixedResetTransferOwner {
                    owner: retired.owner,
                    endpoint: retired.endpoint,
                },
                receipt: retired.receipt,
            })
    }

    /// Take back a successor refused at its boundary, with its port.
    pub(super) fn take_refused(&mut self) -> Option<MixedResetRefusedTransfer<C, R>> {
        self.pair
            .take_refused()
            .map(|refused| MixedResetRefusedTransfer {
                owner: MixedResetTransferOwner {
                    owner: refused.successor,
                    endpoint: refused.endpoint,
                },
                at: refused.at,
                reason: refused.reason,
            })
    }

    #[cfg(test)]
    pub(super) fn state(&self) -> MixedResetState {
        self.pair.state()
    }

    pub(super) const fn fault(&self) -> Option<MixedResetRenderError> {
        self.pair.fault()
    }

    /// Both owners with their ports, off-thread, for each arm's joined teardown after
    /// the callback is joined.
    pub(super) fn into_owners(
        self,
    ) -> (
        MixedResetTransferOwner<C, R>,
        Option<MixedResetTransferOwner<C, R>>,
    ) {
        let (active, secondary) = self.pair.into_parts();
        (
            MixedResetTransferOwner::from_parts(active),
            secondary.map(MixedResetTransferOwner::from_parts),
        )
    }
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> MixedResetTransferAudio<C, R>
where
    C: MixedTransferConsumer<MixedTransferCommand>,
    R: MixedTransferProducer<MixedTransferResult>,
{
    /// Render one callback: the active owner through its own port and, from a reset's
    /// boundary, the fading owner through none.
    ///
    /// Then one Release store per owner publishes its audio boundary, as
    /// [`MixedTransferAudio::render`] does: the active owner's clock, and `B` for the
    /// old owner, which serves no command from `B`. A pending or refused successor has
    /// rendered nothing, so its boundary is left as it was.
    ///
    /// # Errors
    ///
    /// A wrong output shape, or a terminal fault that silenced the complete block.
    pub(super) fn render(
        &mut self,
        output: AudioBlockMut<'_>,
    ) -> Result<(), MixedResetRenderError> {
        let rendered = self.pair.render_private(output);
        let pair = &self.pair;
        pair.active_endpoint.boundary.store(
            pair.active.audio.renderer.clock().as_u64(),
            Ordering::Release,
        );
        if let MixedResetSlot::Fading {
            endpoint, receipt, ..
        }
        | MixedResetSlot::Retired {
            endpoint, receipt, ..
        } = &pair.secondary
        {
            endpoint
                .boundary
                .store(receipt.at.as_u64(), Ordering::Release);
        }
        rendered
    }
}
