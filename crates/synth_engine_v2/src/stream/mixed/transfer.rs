//! ADR-0078's default-off, host-owned mixed transfer rehearsal.
//!
//! This is a non-shipping Rust API over the private one-shot mixed owner. It arms only
//! the bound, closed, initial one-shot plan with one compiled and one live producer and
//! one registered simulated live source, and it carries only note-on, channel bend and
//! the exact key-up of an accepted onset. Pedal, lift, Panic and plan replacement stay
//! refused here, and `PerformanceIngress::prepare` still refuses a mixed producer plan.
//! The armed candidate cannot carry a loop; only a later candidate can enter one. A
//! second source, sustain pedal edges and a host Panic are reachable only through the
//! Phase 9 Stage 6 two-source bridge in [`bridge`], which owns its transfer's control
//! half, so its stage and its Panic cell are the only offerers that transfer has. That
//! arm requires host queues with one protected Panic place beyond the admitted bound.
//!
//! The compiled transport has its own path beside the live queues: a compiled Stop, and
//! a later Start, seek or loop entry of the same bound plan that control seals and the
//! host carries to the audio half's single offer slot and back, as [`activation`]
//! describes. That path never touches a live queue, journal or occurrence.
//!
//! # Queue custody
//!
//! The host owns both bounded SPSC queues. Arm consumes one unsplit command queue and one
//! unsplit result queue, checks their reported capacities and that both are empty, and
//! returns both unchanged on any refusal. A successful arm splits them into two bound
//! wrappers: [`MixedTransferControlPort`] (command producer, result consumer) and
//! [`MixedTransferAudioPort`] (command consumer, result producer). Neither mixed half
//! retains a queue half; the host lends its control wrapper to every submit and collect
//! and its audio wrapper to every callback. Each call compares the wrapper's binding
//! before touching it, and a foreign wrapper is a terminal transfer fault. The host must
//! retain every wrapper until [`MixedTransferControl::join`] and destroy it only off the
//! audio thread; a wrapper dropped earlier may lose queued packets.
//!
//! A queue implementation must provide exclusive, unaliased halves and FIFO delivery,
//! report its true fixed capacity and occupancy, retain every accepted packet exactly
//! once, and return the exact packet from a failed `try_push`. On the audio thread its
//! `try_push` and `try_pop` must be bounded, nonblocking, allocation-free, lock-free and
//! panic-free. Packets are opaque and cannot be cloned or forged through this API.
//!
//! # Order and faults
//!
//! Control checks, in this order: the Acquire halt read, the wrapper and any key-up
//! handle binding, the observation's nominal stamp against the last consumed one, a
//! key-up's open handle, command-ID exhaustion, command capacity, bend credit and onset
//! cell vacancy. A refusal before minting returns the original observation and changes
//! no frontier or reservation; only a minted packet advances the source frontier. Audio
//! independently checks each served packet's owner, ID, nominal stamp and forward horizon
//! before any ledger or ingress mutation. Every transfer fault, from either half, is
//! published through one prepared halt; audio silences the complete callback that
//! observes it and every later one.

use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU64, Ordering},
};

mod activation;
mod bridge;
mod publication;
pub use activation::{
    MixedActivationAdopted, MixedActivationCollectError, MixedActivationCollectRefusal,
    MixedActivationKind, MixedActivationLoop, MixedActivationOffer, MixedActivationOutcome,
    MixedActivationPrepareError, MixedActivationReceipt, MixedActivationRefusal,
    MixedActivationRetired, MixedActivationSlot,
};
pub use bridge::{
    MixedBridgeAbandonRefusal, MixedBridgeArmed, MixedBridgeCommitRefusal, MixedBridgeControl,
    MixedBridgeError, MixedBridgeFault, MixedBridgeJoinError, MixedBridgeJoinParts,
    MixedBridgeJoinRefusal, MixedBridgeLifted, MixedBridgeOutcome, MixedBridgePanicEnd,
    MixedBridgePanicRefusal, MixedBridgePanicked, MixedBridgeReport, MixedBridgeResetPhase,
    MixedBridgeRetiredControl, MixedBridgeRetiredJoinRefusal, MixedBridgeRetiredTeardown,
    MixedBridgeSealRefusal, MixedBridgeSealRefused, MixedBridgeTeardown, MixedBridgeWait,
    MixedCommandPlace, MixedOnsetRefusal, MixedPumpStep, MixedRetiredHandle, MixedSourceCustody,
    MixedSourceFault, MixedSourceHandoff, MixedSourcePacketId, MixedSourcePacketIdError,
    MixedSplit, MixedStaged, MixedTransferFence, MixedTransferSources, MixedTransferSourcesError,
};
pub use publication::{
    MixedPublication, MixedPublicationFault, MixedPublicationGate, MixedPublicationPublishError,
    MixedPublicationReceiver, MixedPublicationRefusal, MixedPublicationSender,
};

use synth_core::MidiChannel;
use thiserror::Error;

#[cfg(test)]
use super::MixedBendScopeCapacity;
use super::{
    MixedBendTables, MixedControlQueues, MixedIngressCommand, MixedIngressCommandId,
    MixedIngressIdentityFault, MixedIngressOriginId, MixedIngressOutcome, MixedIngressOwner,
    MixedIngressQueueCount, MixedIngressRequest, MixedIngressResult,
    MixedIngressResultCollectError, MixedJoinedPrepared, MixedOneShotArmError, MixedOneShotAudio,
    MixedOneShotControl, MixedOneShotRenderError, MixedPanicCapability, MixedStampedCandidate,
    mixed_collection_pair,
    occurrence::{
        LedgerHold, LiveOccurrenceId, MixedBendRefused, MixedBendSubmitError, MixedLifted,
        MixedLiveScope, MixedLiveSource, MixedOccurrenceState, MixedOnsetSubmitError,
        MixedPanicEnd, MixedPanicRefused, MixedPanicked, MixedPedalDown, MixedPedalRefused,
        MixedReleaseSubmitError, MixedRetireCause,
    },
};
use crate::{
    identity::NoteIdentity,
    ingress::{IngressCounters, IngressRefused, RELEASE_GROUP_CAPACITY},
    profile::HostProfile,
    publish::ProducerClass,
    quantities::{Cents, EventCount, HeldNoteCount, KeyIdentity, NoteVelocity},
    render::{AudioBlockMut, MixedSoundingSnapshotError},
    time::{FrameCount, SampleTime},
};

/// The sole registered simulated live source; its ordinal matches the private bridge's
/// first bound source.
const REHEARSAL_SOURCE: MixedLiveSource = MixedLiveSource::new(1);

/// The channel scopes of the sole registered MIDI1 source. Both bend tables must cover
/// them, so a legal first bend never exhausts either table.
const REHEARSAL_SCOPES: MixedTransferScopeCount = MixedTransferScopeCount(16);

/// Cumulative publication charges observed by one rehearsal owner. The two unavailable
/// producers have no executable path in this host; their observations remain `None`.
/// Peaks belong to this owner, and the external total is a same-quantum peak rather than
/// the sum of the independent class peaks. The renderer-internal arena is not included.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedTransferPublicationUsage {
    high_water: [Option<EventCount>; 6],
    external_total: EventCount,
    last_quantum: Option<(SampleTime, [Option<EventCount>; 6])>,
}

impl MixedTransferPublicationUsage {
    pub(crate) fn of(audio: &MixedOneShotAudio) -> Self {
        Self {
            high_water: ProducerClass::ALL.map(|class| {
                match (class, audio.arbiter.high_water(class)) {
                    (
                        ProducerClass::AuthoredRuntime | ProducerClass::Internal,
                        EventCount::NONE,
                    ) => None,
                    (_, observed) => Some(observed),
                }
            }),
            external_total: audio.arbiter.high_water_external_total(),
            last_quantum: audio.arbiter.rehearsal_quantum().map(|(start, counts)| {
                (
                    start,
                    ProducerClass::ALL.map(|class| match (class, counts[class.index()]) {
                        (
                            ProducerClass::AuthoredRuntime | ProducerClass::Internal,
                            EventCount::NONE,
                        ) => None,
                        (_, count) => Some(count),
                    }),
                )
            }),
        }
    }

    /// The owner's highest charged occupancy for `class`, including a quantum that
    /// later failed. `None` means that producer is unavailable, not measured at zero.
    pub const fn high_water(&self, class: ProducerClass) -> Option<EventCount> {
        self.high_water[class.index()]
    }

    /// The highest total external charge reached in any single quantum for this owner.
    pub const fn high_water_external_total(&self) -> EventCount {
        self.external_total
    }

    /// Actual charges in the most recently opened single-quantum window and its
    /// opening sample. `None` means no single-quantum window is available; a carry
    /// callback must not be inferred to have produced fresh charges. Unavailable
    /// classes retain `None` under the same rule as the lifetime observations.
    pub const fn last_quantum(&self) -> Option<(SampleTime, [Option<EventCount>; 6])> {
        self.last_quantum
    }
}

/// Passive lifetime resource observations for one mixed rehearsal owner. The named
/// storage buffers exclude allocator bookkeeping and are not a total memory claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedTransferResourceUsage {
    pub renderer: crate::render::RehearsalRendererUsage,
    pub renderer_storage: crate::render::RehearsalRendererStorage,
    pub ingress_queue_high_water: EventCount,
    pub live_holds_high_water: EventCount,
    pub ingress_storage: crate::quantities::PreparedBytes,
    pub publication_storage: crate::quantities::PreparedBytes,
}

impl MixedTransferResourceUsage {
    pub(crate) fn of(
        audio: &MixedOneShotAudio,
    ) -> Result<Self, crate::render::RehearsalUsageError> {
        use crate::render::RehearsalUsageError;
        Ok(Self {
            renderer: audio.audio.renderer.rehearsal_usage()?,
            renderer_storage: audio.audio.renderer.rehearsal_storage()?,
            ingress_queue_high_water: u32::try_from(audio.ingress_queue_peak)
                .map(EventCount::measured)
                .map_err(|_| RehearsalUsageError::Counter("mixed ingress queue"))?,
            live_holds_high_water: audio.ingress_holds_peak,
            ingress_storage: audio.test_ingress.storage_bytes(),
            publication_storage: audio
                .arbiter
                .storage_bytes()
                .ok_or(RehearsalUsageError::Storage("publication arbiter"))?,
        })
    }
}

/// A number of host queue places: a queue's capacity or occupancy, the admitted
/// command/result bound, or places reserved for exact key-ups. Every count is valid;
/// arm compares a queue's reported capacity against the admitted bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[must_use]
pub struct MixedTransferPlaces(usize);

impl MixedTransferPlaces {
    /// No place.
    pub const NONE: Self = Self(0);

    /// This many packet places.
    pub const fn new(places: usize) -> Self {
        Self(places)
    }

    /// The number of places.
    pub const fn get(self) -> usize {
        self.0
    }
}

impl std::fmt::Display for MixedTransferPlaces {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} places", self.0)
    }
}

/// A number of source channel scopes: a bend table's prepared cells, or the scopes the
/// registered sources reach.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[must_use]
pub struct MixedTransferScopeCount(usize);

impl MixedTransferScopeCount {
    /// The number of scopes.
    pub const fn get(self) -> usize {
        self.0
    }
}

impl std::fmt::Display for MixedTransferScopeCount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} scopes", self.0)
    }
}

/// A count of live occurrences in a bend result or a joined tombstone table.
/// It retains the table's exact size on every supported platform.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[must_use]
pub struct MixedTransferHeldCount(usize);

impl MixedTransferHeldCount {
    /// The maximum combined live sound hold set in this rehearsal: sustain, Panic and
    /// reset each release it through one bounded group. This is not a raw source limit.
    pub const MAX_SOUND_HOLDS: Self = Self(RELEASE_GROUP_CAPACITY);

    /// No held occurrences.
    pub const NONE: Self = Self(0);

    /// The exact number of occurrences.
    pub const fn get(self) -> usize {
        self.0
    }
}

/// One unsplit, empty host queue offered to [`MixedJoinedPrepared::arm_transfer_rehearsal`].
pub trait MixedTransferQueue<T>: Sized {
    /// The half that pushes packets.
    type Producer: MixedTransferProducer<T>;
    /// The half that pops packets.
    type Consumer: MixedTransferConsumer<T>;

    /// The true fixed number of packets the queue can hold at once.
    fn capacity(&self) -> MixedTransferPlaces;

    /// The packets the queue holds now.
    fn occupied(&self) -> MixedTransferPlaces;

    /// Split into exclusive, unaliased halves. Arm calls this only after accepting the
    /// queue, so a refused queue is returned unsplit.
    fn split(self) -> (Self::Producer, Self::Consumer);
}

/// The pushing half of a host queue.
pub trait MixedTransferProducer<T> {
    /// Push one packet, or return that exact packet when the queue is full.
    ///
    /// # Errors
    ///
    /// Returns the packet when no place is free.
    fn try_push(&mut self, packet: T) -> Result<(), T>;
}

/// The popping half of a host queue.
pub trait MixedTransferConsumer<T> {
    /// Pop the oldest packet, if any.
    fn try_pop(&mut self) -> Option<T>;
}

/// One minted command, opaque to the host.
#[must_use]
pub struct MixedTransferCommand(pub(super) MixedIngressCommand);

impl std::fmt::Debug for MixedTransferCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedTransferCommand")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

impl MixedTransferCommand {
    /// The command ID this packet carries.
    pub const fn id(&self) -> MixedTransferCommandId {
        MixedTransferCommandId(self.0.id.0)
    }
}

/// One audio result for a command, opaque to the host. It carries the quantum boundary
/// that served its command, so a report can state where the late rule executed it.
#[must_use]
pub struct MixedTransferResult(pub(super) MixedIngressResult, pub(super) SampleTime);

impl std::fmt::Debug for MixedTransferResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedTransferResult")
            .field("id", &self.id())
            .finish_non_exhaustive()
    }
}

impl MixedTransferResult {
    /// The command ID this result answers.
    pub const fn id(&self) -> MixedTransferCommandId {
        MixedTransferCommandId(self.0.id.0)
    }
}

/// A gap-free, increasing command ID within one transfer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[must_use]
pub struct MixedTransferCommandId(u64);

impl MixedTransferCommandId {
    /// The ID's serial; the first command of a transfer is 1.
    pub const fn get(self) -> u64 {
        self.0
    }

    const fn of(id: MixedIngressCommandId) -> Self {
        Self(id.0)
    }
}

/// The owner, plan, epoch, table and transfer one arm bound its wrappers and handles to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MixedTransferBinding {
    owner: MixedIngressOwner,
    transfer: u64,
}

/// The bridge publishes a successor and its old transfer's last minted command before
/// offering reset. The same bridge cannot clear or republish while the offered owner is
/// in the audio wrapper; `abandon_reset` first requires that owner to be returned.
///
/// One atomic phase is the reset's only shared decision point. While it is `Sealed`,
/// the bridge's due Panic and audio at `B` each try one compare-and-swap away from it, so
/// exactly one of "the Panic cancels" and "audio decides" wins. Offer-time checks only
/// read it.
#[derive(Debug)]
pub(super) struct MixedResetSeal {
    phase: AtomicU8,
    successor: AtomicU64,
    through: AtomicU64,
}

/// The seal's phase, stored as one `u8`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MixedResetSealPhase {
    /// Nothing is sealed; the only phase the control side publishes from.
    Open,
    /// Published for one successor and undecided.
    Sealed,
    /// A due Panic won: the old owner keeps serving and the successor is refused.
    Cancelled,
    /// Audio won at `B` with every check passed and is ending the old owner's notes.
    Acquired,
    /// Audio refused the successor at `B`, before or after acquiring the seal.
    Declined,
}

impl MixedResetSealPhase {
    const fn encode(self) -> u8 {
        match self {
            Self::Open => 0,
            Self::Sealed => 1,
            Self::Cancelled => 2,
            Self::Acquired => 3,
            Self::Declined => 4,
        }
    }

    /// Only the five encodings above are ever stored; any other byte reads as spent.
    const fn decode(byte: u8) -> Self {
        match byte {
            0 => Self::Open,
            1 => Self::Sealed,
            2 => Self::Cancelled,
            3 => Self::Acquired,
            _ => Self::Declined,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MixedResetSealState {
    /// Not sealed for this successor, or the decision is already spent.
    Unsealed,
    /// A due Panic cancelled the reset sealed for this successor.
    Cancelled,
    IngressPending,
    Ready,
}

/// Audio's one decision at `B` for a seal naming this successor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MixedResetSealDecision {
    /// Audio's compare-and-swap won: `Acquired` if it took the reset, else `Declined`.
    Decided,
    /// The bridge's Panic won first.
    Cancelled,
    /// The seal does not name this successor, or is open or already decided; nothing
    /// changed.
    NotSealed,
}

impl MixedResetSeal {
    pub(super) const fn new() -> Self {
        Self {
            phase: AtomicU8::new(MixedResetSealPhase::Open.encode()),
            successor: AtomicU64::new(0),
            through: AtomicU64::new(0),
        }
    }

    fn phase(&self) -> MixedResetSealPhase {
        MixedResetSealPhase::decode(self.phase.load(Ordering::Acquire))
    }

    /// Whether the seal's published successor is `successor`. Read only after an
    /// Acquire load or compare-and-swap of the phase observed a published phase.
    fn names(&self, successor: Option<MixedTransferBinding>) -> bool {
        successor.map(|binding| binding.transfer) == Some(self.successor.load(Ordering::Relaxed))
    }

    /// Read-only: the offer's and the boundary's check. It never changes the phase.
    pub(super) fn reset_seal_state(
        &self,
        successor: Option<MixedTransferBinding>,
        served: Option<MixedIngressCommandId>,
    ) -> MixedResetSealState {
        let phase = self.phase();
        if !matches!(
            phase,
            MixedResetSealPhase::Sealed | MixedResetSealPhase::Cancelled
        ) || !self.names(successor)
        {
            return MixedResetSealState::Unsealed;
        }
        if phase == MixedResetSealPhase::Cancelled {
            MixedResetSealState::Cancelled
        } else if served.map_or(0, |id| id.0) < self.through.load(Ordering::Relaxed) {
            MixedResetSealState::IngressPending
        } else {
            MixedResetSealState::Ready
        }
    }

    /// Audio's decision at `B`: one compare-and-swap from `Sealed` to `Acquired` when
    /// `take` holds, else to `Declined`. Lock-free and allocation-free.
    pub(super) fn reset_seal_decide(
        &self,
        successor: Option<MixedTransferBinding>,
        take: bool,
    ) -> MixedResetSealDecision {
        let phase = self.phase();
        if !matches!(
            phase,
            MixedResetSealPhase::Sealed | MixedResetSealPhase::Cancelled
        ) || !self.names(successor)
        {
            return MixedResetSealDecision::NotSealed;
        }
        if phase == MixedResetSealPhase::Cancelled {
            return MixedResetSealDecision::Cancelled;
        }
        let decided = if take {
            MixedResetSealPhase::Acquired
        } else {
            MixedResetSealPhase::Declined
        };
        match self.phase.compare_exchange(
            MixedResetSealPhase::Sealed.encode(),
            decided.encode(),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => MixedResetSealDecision::Decided,
            Err(found) if MixedResetSealPhase::decode(found) == MixedResetSealPhase::Cancelled => {
                MixedResetSealDecision::Cancelled
            }
            Err(_) => MixedResetSealDecision::NotSealed,
        }
    }

    /// Audio acquired the seal and then could not end the old owner's live notes.
    pub(super) fn reset_seal_decline(&self) {
        self.phase
            .store(MixedResetSealPhase::Declined.encode(), Ordering::Release);
    }

    /// The bridge's due Panic: one compare-and-swap from `Sealed` to `Cancelled`.
    pub(super) fn cancel(&self) -> bool {
        self.phase
            .compare_exchange(
                MixedResetSealPhase::Sealed.encode(),
                MixedResetSealPhase::Cancelled.encode(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub(super) fn publish(&self, successor: MixedTransferBinding, through: u64) {
        self.successor.store(successor.transfer, Ordering::Relaxed);
        self.through.store(through, Ordering::Relaxed);
        self.phase
            .store(MixedResetSealPhase::Sealed.encode(), Ordering::Release);
    }

    pub(super) fn clear(&self) {
        self.phase
            .store(MixedResetSealPhase::Open.encode(), Ordering::Release);
    }

    #[cfg(test)]
    pub(super) fn phase_for_test(&self) -> MixedResetSealPhase {
        self.phase()
    }
}

static NEXT_TRANSFER: AtomicU64 = AtomicU64::new(1);

/// A fresh transfer serial, or `None` once the space is spent. Never reissued.
fn issue_transfer() -> Option<u64> {
    let mut serial = NEXT_TRANSFER.load(Ordering::Relaxed);
    loop {
        let next = serial.checked_add(1)?;
        match NEXT_TRANSFER.compare_exchange_weak(
            serial,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Some(serial),
            Err(current) => serial = current,
        }
    }
}

/// The host's bound command producer and result consumer, lent to each control call.
/// It exposes no push, pop or clone; only the crate moves packets through it.
#[must_use]
pub struct MixedTransferControlPort<C, R> {
    pub(super) binding: MixedTransferBinding,
    pub(super) commands: C,
    pub(super) results: R,
    /// The service boundary of the result this port popped last.
    pub(super) served_at: Option<SampleTime>,
}

/// The host's bound command consumer and result producer, lent to each callback.
/// It exposes no push, pop or clone; only the crate moves packets through it.
#[must_use]
pub struct MixedTransferAudioPort<C, R> {
    pub(super) binding: MixedTransferBinding,
    pub(super) commands: C,
    pub(super) results: R,
}

impl<C, R> std::fmt::Debug for MixedTransferControlPort<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedTransferControlPort")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl<C, R> std::fmt::Debug for MixedTransferAudioPort<C, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedTransferAudioPort")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl<C, R> MixedControlQueues for MixedTransferControlPort<C, R>
where
    C: MixedTransferProducer<MixedTransferCommand>,
    R: MixedTransferConsumer<MixedTransferResult>,
{
    fn push_command(
        &mut self,
        _control: &mut MixedOneShotControl,
        command: MixedIngressCommand,
    ) -> Result<(), MixedIngressCommand> {
        self.commands
            .try_push(MixedTransferCommand(command))
            .map_err(|packet| packet.0)
    }

    fn pop_result(&mut self, _control: &mut MixedOneShotControl) -> Option<MixedIngressResult> {
        let packet = self.results.try_pop()?;
        self.served_at = Some(packet.1);
        Some(packet.0)
    }
}

/// A source note-on for the sole registered live source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixedSourceNoteOn {
    /// The nominal stamp; never moved by the transfer.
    pub at: SampleTime,
    /// The source channel, which scopes bends.
    pub channel: MidiChannel,
    /// The key.
    pub key: KeyIdentity,
    /// The velocity.
    pub velocity: NoteVelocity,
}

/// A source channel bend for the sole registered live source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixedSourceBend {
    /// The nominal stamp; never moved or coalesced.
    pub at: SampleTime,
    /// The source channel the bend reaches.
    pub channel: MidiChannel,
    /// The channel's new pitch offset.
    pub cents: Cents,
}

/// The exact key-up of one accepted onset, named by its handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedSourceKeyUp {
    /// The nominal stamp; never moved by the transfer.
    pub at: SampleTime,
    /// The handle the onset's submit returned.
    pub handle: MixedKeyUpHandle,
}

/// The name one submitted onset's key-up must use. It carries its transfer's binding, so
/// another transfer refuses it as a foreign handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedKeyUpHandle {
    binding: MixedTransferBinding,
    occurrence: LiveOccurrenceId,
}

/// Why a submit minted nothing. The original observation is returned with it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedTransferRefusal {
    /// The call's first read observed the transfer halt.
    #[error("the mixed transfer is halted")]
    Halted,
    /// Transient: every command and result place is charged.
    #[error("no command and result place is free")]
    CommandCapacity,
    /// Transient: too few bend credits are free; served commands return them.
    #[error("too few bend credits are free")]
    BendCredit,
    /// Defensive: no bend scope cell is free. Arm sizes the table to cover the source.
    #[error("no bend scope reservation is free")]
    BendScopes,
    /// Defensive: an open handle's reserved key-up place is not available.
    #[error("the key-up's reserved place is unavailable")]
    ReleasePlace,
    /// Non-terminal: the key-up names a handle of this transfer that is not open.
    #[error("the key-up's handle is not open")]
    NotOpen,
    /// Terminal: the lent wrapper belongs to another transfer.
    #[error("the lent control endpoint belongs to another transfer")]
    ForeignEndpoint,
    /// Terminal: the key-up's handle belongs to another transfer.
    #[error("the key-up handle belongs to another transfer")]
    ForeignHandle,
    /// Terminal: the observation's stamp precedes the last consumed observation's.
    #[error("source stamp precedes the consumed frontier {frontier}")]
    SourceOrder {
        /// The nominal stamp of the last consumed observation.
        frontier: SampleTime,
    },
    /// Terminal: the next command ID cannot be minted.
    #[error("the mixed transfer command ID space is exhausted")]
    CommandIdExhausted,
    /// Terminal: an otherwise admissible onset found no vacant, unexhausted handle cell.
    #[error("the live occurrence cells are exhausted")]
    OccurrenceCellsExhausted,
    /// Terminal and defensive: the merger's reservation count disagreed with an open
    /// handle, so the key-up was refused before minting.
    #[error("the merger's handle ledger is inconsistent")]
    MergerLedger,
}

impl MixedTransferRefusal {
    /// Whether this refusal raised the transfer's terminal halt.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::ForeignEndpoint
                | Self::ForeignHandle
                | Self::SourceOrder { .. }
                | Self::CommandIdExhausted
                | Self::OccurrenceCellsExhausted
                | Self::MergerLedger
        )
    }
}

/// Why a submit did not leave a command in the host queue.
#[derive(Debug, Error)]
pub enum MixedTransferSubmitError<O> {
    /// Nothing was minted; the original observation comes back.
    #[error("{reason}")]
    Refused {
        /// Why.
        reason: MixedTransferRefusal,
        /// The unconsumed observation.
        original: O,
    },
    /// The observation was consumed into a command whose physical push failed. This is a
    /// terminal host fault: control keeps the packet, its journal entry and every charge
    /// for joined teardown, and the source frontier names it.
    #[error("the host command queue refused minted command {id:?}")]
    PushFailed {
        /// The retained command's ID.
        id: MixedTransferCommandId,
        /// The onset's handle, or the key-up's handle now releasing under `id`.
        handle: Option<MixedKeyUpHandle>,
    },
    /// Defensive: the observation was consumed into a command, but the merger's handle
    /// ledger refused the state that command needs. This is terminal: control keeps the
    /// command, its journal entry and its charges for joined teardown, and the source
    /// frontier names it. A host queue push may also have failed.
    #[error("the merger's handle ledger refused minted command {id:?}")]
    MergerLedger {
        /// The retained command's ID.
        id: MixedTransferCommandId,
        /// The handle the command names, if it names one.
        handle: Option<MixedKeyUpHandle>,
    },
}

/// A terminal transfer fault as one half recorded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedTransferFault {
    /// A lent wrapper belonged to another transfer.
    #[error("a lent endpoint belongs to another transfer")]
    ForeignEndpoint,
    /// A key-up handle belonged to another transfer.
    #[error("a key-up handle belongs to another transfer")]
    ForeignHandle,
    /// A source observation was stamped before the consumed frontier.
    #[error("source stamp {at} precedes the consumed frontier {frontier}")]
    SourceOrder {
        /// The refused stamp.
        at: SampleTime,
        /// The frontier it preceded.
        frontier: SampleTime,
    },
    /// The command ID space is spent.
    #[error("the command ID space is exhausted")]
    CommandIdExhausted,
    /// An admissible onset found no vacant, unexhausted handle cell.
    #[error("the live occurrence cells are exhausted")]
    OccurrenceCellsExhausted,
    /// The host command queue refused a minted command.
    #[error("the host command queue refused command {id:?}")]
    CommandPush {
        /// The retained command.
        id: MixedTransferCommandId,
    },
    /// Defensive: the merger's handle ledger disagreed with a command it staged, or with
    /// the reservation an open handle holds; `id` names the command when one was minted.
    #[error("the merger's handle ledger is inconsistent at command {id:?}")]
    MergerLedger {
        /// The minted command, if any.
        id: Option<MixedTransferCommandId>,
    },
    /// A popped result did not answer the oldest outstanding command.
    #[error("result {received:?} does not answer the next outstanding command {expected:?}")]
    ResultProtocol {
        /// The result's command ID.
        received: MixedTransferCommandId,
        /// The oldest outstanding command, if any.
        expected: Option<MixedTransferCommandId>,
    },
    /// The other half published the halt.
    #[error("the transfer was halted by its other half")]
    Remote,
}

/// Why a collection returned no result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedTransferCollectError {
    /// The call's first read observed the halt; nothing was popped or settled.
    #[error("the mixed transfer is halted: {0}")]
    Halted(MixedTransferFault),
    /// This call detected a terminal fault.
    #[error("mixed transfer collection faulted: {0}")]
    Fault(MixedTransferFault),
}

/// How an exact key-up retired its handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedKeyUpRetired {
    /// The accepted occurrence ended in its reserved place.
    Released(NoteIdentity),
    /// The onset had been refused; its tombstone consumed the key-up.
    AfterRefusal(IngressRefused),
    /// A host Panic had already ended the note; its tombstone consumed the key-up.
    /// Only the two-source bridge carries Panic.
    AfterPanic(NoteIdentity),
    /// The key went up under its channel's pedal: nothing was offered, the note keeps
    /// sounding and its hold stays outstanding until that channel's lift ends it. The
    /// handle stays held. Only the two-source bridge carries pedal edges.
    Sustained,
}

/// The members one applied bend wrote, each at the bend's nominal stamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedTransferBendApplied {
    members: MixedTransferHeldCount,
}

impl MixedTransferBendApplied {
    /// Live notes of the channel that took the bend.
    pub const fn members(self) -> MixedTransferHeldCount {
        self.members
    }
}

/// Why audio refused a bend; the channel's value and every entry are unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedTransferBendRefused {
    /// No audio bend cell was free for the channel.
    #[error("no audio bend cell is free")]
    BendTable,
    /// The channel holds more live notes than one bend fanout carries.
    #[error("the channel holds more live notes than one bend fanout")]
    Fanout,
    /// The command carried fewer credits than members.
    #[error("the bend carried too few credits")]
    Credits,
    /// The ingress store refused the entries.
    #[error(transparent)]
    Ingress(IngressRefused),
}

/// How audio applied an effective pedal-down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedTransferPedalDown {
    /// The channel's pedal went down.
    Pressed,
    /// The pedal was already down on audio, because the channel's last lift was refused.
    AlreadyDown,
}

/// Why audio refused a pedal edge; the pedal and every sustained note keep their state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedTransferPedalRefused {
    /// No audio pedal cell was free for the channel.
    #[error("no audio pedal cell is free")]
    PedalTable,
    /// The channel sustains more notes than one release group carries.
    #[error("the channel sustains more notes than one release group")]
    GroupCapacity,
    /// The ingress store refused the lift's release group.
    #[error(transparent)]
    Ingress(IngressRefused),
}

impl From<MixedPedalRefused> for MixedTransferPedalRefused {
    fn from(refused: MixedPedalRefused) -> Self {
        match refused {
            MixedPedalRefused::PedalTableFull => Self::PedalTable,
            MixedPedalRefused::GroupCapacity => Self::GroupCapacity,
            MixedPedalRefused::Ingress(refused) => Self::Ingress(refused),
        }
    }
}

/// The sustained notes one applied lift ended as one `SustainLift` release group, by
/// their onsets' handles, in group order. A lift of a channel that sustained nothing
/// ends none.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedTransferLifted {
    binding: MixedTransferBinding,
    members: [Option<LiveOccurrenceId>; RELEASE_GROUP_CAPACITY],
}

impl MixedTransferLifted {
    /// The ended notes' handles, in group order.
    pub fn handles(&self) -> impl Iterator<Item = MixedKeyUpHandle> + '_ {
        let binding = self.binding;
        self.members
            .iter()
            .flatten()
            .map(move |&occurrence| MixedKeyUpHandle::of(binding, occurrence))
    }

    /// How many sustained notes the lift ended.
    pub fn count(&self) -> MixedTransferHeldCount {
        MixedTransferHeldCount(self.members.iter().flatten().count())
    }
}

/// How one applied host Panic ended one live note.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedTransferPanicEnd {
    /// The key was up under its channel's pedal: the handle retired with the Panic.
    Retired(MixedKeyUpHandle),
    /// The key was down: the handle stays held on a Panic tombstone, which consumes its
    /// key-up as [`MixedKeyUpRetired::AfterPanic`].
    Tombstoned(MixedKeyUpHandle),
}

/// The live notes one applied host Panic ended as one `Panic` release group, in group
/// order, and the sample it executed at. In the same quantum the compiled producer's
/// open notes end at that sample too.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedTransferPanicked {
    executed_at: SampleTime,
    binding: MixedTransferBinding,
    ended: [Option<MixedPanicEnd>; RELEASE_GROUP_CAPACITY],
}

impl MixedTransferPanicked {
    /// The sample the Panic's release group and compiled cut execute at.
    pub const fn executed_at(&self) -> SampleTime {
        self.executed_at
    }

    /// The ended notes, in group order.
    pub fn ended(&self) -> impl Iterator<Item = MixedTransferPanicEnd> + '_ {
        let binding = self.binding;
        self.ended.iter().flatten().map(move |end| match *end {
            MixedPanicEnd::Retired(occurrence) => {
                MixedTransferPanicEnd::Retired(MixedKeyUpHandle::of(binding, occurrence))
            }
            MixedPanicEnd::Tombstoned(occurrence) => {
                MixedTransferPanicEnd::Tombstoned(MixedKeyUpHandle::of(binding, occurrence))
            }
        })
    }

    /// How many live notes the Panic ended.
    pub fn count(&self) -> MixedTransferHeldCount {
        MixedTransferHeldCount(self.ended.iter().flatten().count())
    }
}

/// Why audio refused a host Panic; every live note keeps its state and hold, and no
/// compiled note was cut.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedTransferPanicRefused {
    /// More live notes sound or are sustained than one release group carries.
    #[error("more live notes than one release group carries")]
    GroupCapacity,
    /// The ingress store refused the Panic's release group.
    #[error(transparent)]
    Ingress(IngressRefused),
}

impl From<MixedPanicRefused> for MixedTransferPanicRefused {
    fn from(refused: MixedPanicRefused) -> Self {
        match refused {
            MixedPanicRefused::GroupCapacity => Self::GroupCapacity,
            MixedPanicRefused::Ingress(refused) => Self::Ingress(refused),
        }
    }
}

/// What a host Panic has done to the compiled producer, and whether audio holds one
/// parked. Fixed-size and `Copy`; reading it allocates nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedTransferPanicState {
    live_released_after_panic: EventCount,
    compiled_ended: HeldNoteCount,
    compiled_groups: EventCount,
    compiled_tombstones: HeldNoteCount,
    compiled_released_after_panic: EventCount,
    compiled_controls_after_panic: EventCount,
    parked: Option<SampleTime>,
}

impl MixedTransferPanicState {
    /// Live key-ups a Panic tombstone consumed instead of ending a note.
    pub const fn live_released_after_panic(&self) -> EventCount {
        self.live_released_after_panic
    }

    /// Open compiled notes every applied Panic ended at its execution sample.
    pub const fn compiled_ended(&self) -> HeldNoteCount {
        self.compiled_ended
    }

    /// Release groups those compiled cuts charged.
    pub const fn compiled_groups(&self) -> EventCount {
        self.compiled_groups
    }

    /// Compiled notes a Panic ended whose own `Off` has not been reached yet.
    pub const fn compiled_tombstones(&self) -> HeldNoteCount {
        self.compiled_tombstones
    }

    /// Compiled `Off` edges a Panic tombstone consumed instead of publishing.
    pub const fn compiled_released_after_panic(&self) -> EventCount {
        self.compiled_released_after_panic
    }

    /// Compiled expression and bend edges a Panic tombstone consumed.
    pub const fn compiled_controls_after_panic(&self) -> EventCount {
        self.compiled_controls_after_panic
    }

    /// The stamp of a Panic audio holds unserved until the quantum of its execution
    /// stamp opens, if any.
    pub const fn parked(&self) -> Option<SampleTime> {
        self.parked
    }
}

/// What audio did with one command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MixedTransferOutcome {
    /// The onset's identity, or the refusal its handle's tombstone now carries.
    NoteOn {
        /// The onset's handle.
        handle: MixedKeyUpHandle,
        /// The identity or refusal.
        result: Result<NoteIdentity, IngressRefused>,
    },
    /// The key-up's retirement, or an ingress refusal that left the note sounding.
    KeyUp {
        /// The key-up's handle.
        handle: MixedKeyUpHandle,
        /// How the handle retired.
        result: Result<MixedKeyUpRetired, IngressRefused>,
    },
    /// The bend's members, or a refusal that changed nothing.
    Bend {
        /// The bent channel.
        channel: MidiChannel,
        /// The members or refusal.
        result: Result<MixedTransferBendApplied, MixedTransferBendRefused>,
    },
    /// Audio's table held no cell the command could use; nothing changed.
    HandleRefused {
        /// The command's handle.
        handle: MixedKeyUpHandle,
    },
    /// An effective pedal-down: how audio applied it, or a refusal that changed nothing.
    /// Only the two-source bridge mints one.
    PedalDown {
        /// The pedalled channel.
        channel: MidiChannel,
        /// How audio applied it, or why it refused.
        result: Result<MixedTransferPedalDown, MixedTransferPedalRefused>,
    },
    /// An effective lift: the sustained notes it ended, or a refusal that left the pedal
    /// down and every sustained note sounding. Only the two-source bridge mints one.
    Lift {
        /// The lifted channel.
        channel: MidiChannel,
        /// The ended notes, or why audio refused.
        result: Result<MixedTransferLifted, MixedTransferPedalRefused>,
    },
    /// A host Panic: the live notes it ended and its execution sample, or a refusal that
    /// changed nothing. Only the two-source bridge mints one.
    Panic {
        /// The ended notes, or why audio refused.
        result: Result<MixedTransferPanicked, MixedTransferPanicRefused>,
    },
}

/// One settled result: its command ID, the command's stamp, where audio executed it and
/// what audio did.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct MixedTransferReport {
    id: MixedTransferCommandId,
    at: SampleTime,
    executed_at: SampleTime,
    outcome: MixedTransferOutcome,
}

impl MixedTransferReport {
    /// The command this result answers.
    pub const fn id(&self) -> MixedTransferCommandId {
        self.id
    }

    /// The command's stamp, unchanged by service.
    pub const fn at(&self) -> SampleTime {
        self.at
    }

    /// The sample the late rule executed the command at: its stamp, or the quantum
    /// boundary that served it when the stamp was already behind that boundary.
    pub const fn executed_at(&self) -> SampleTime {
        self.executed_at
    }

    /// What audio did.
    pub const fn outcome(&self) -> MixedTransferOutcome {
        self.outcome
    }
}

/// Why the audio callback failed. Every variant except `OutputShape` is terminal, and
/// every terminal callback is silent.
#[derive(Clone, Copy, Debug, PartialEq, Error)]
pub enum MixedTransferRenderError {
    /// The output block does not match the bound profile; nothing changed.
    #[error("the output block does not match the bound profile")]
    OutputShape,
    /// An earlier callback already faulted.
    #[error("the mixed transfer audio half is terminally faulted")]
    Faulted,
    /// The lent wrapper belongs to another transfer; no queue was touched.
    #[error("the lent audio endpoint belongs to another transfer")]
    ForeignEndpoint,
    /// A foreign, duplicate, reordered or missing-predecessor command.
    #[error("a served command is foreign or out of ID order")]
    CommandOrder,
    /// A command stamped before the last served command.
    #[error("a served command is stamped before the last served command")]
    BackwardTime,
    /// A command beyond the forward event horizon, rejected and counted once.
    #[error("a served command lies beyond the forward event horizon")]
    BeyondHorizon,
    /// The host result queue refused a result; audio keeps it for teardown.
    #[error("the host result queue refused a result")]
    ResultPush,
    /// The control half published the halt.
    #[error("the control half halted the transfer")]
    RemoteHalt,
    /// Another terminal render, publication or boundary fault.
    #[error("the mixed stream faulted: {0}")]
    Stream(MixedTransferStreamFault),
}

/// An opaque terminal render, publication or boundary fault of the mixed stream.
#[derive(Clone, Copy, Debug, PartialEq, Error)]
#[error(transparent)]
pub struct MixedTransferStreamFault(MixedOneShotRenderError);

/// A failure to inspect the stopped renderer during joined teardown.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error(transparent)]
pub struct MixedTransferSnapshotFault(MixedSoundingSnapshotError);

impl From<MixedOneShotRenderError> for MixedTransferRenderError {
    fn from(error: MixedOneShotRenderError) -> Self {
        use MixedOneShotRenderError as Private;
        match error {
            Private::OutputShape => Self::OutputShape,
            Private::Faulted => Self::Faulted,
            Private::IngressForeignEndpoint => Self::ForeignEndpoint,
            Private::IngressCommandOrder => Self::CommandOrder,
            Private::IngressBackwardTime => Self::BackwardTime,
            Private::IngressBeyondHorizon => Self::BeyondHorizon,
            Private::IngressResultPush => Self::ResultPush,
            Private::IngressTransferHalt => Self::RemoteHalt,
            other => Self::Stream(MixedTransferStreamFault(other)),
        }
    }
}

/// Why arm refused; both host queues come back unsplit and untouched.
#[derive(Clone, Copy, Debug, PartialEq, Error)]
pub enum MixedTransferArmError {
    /// The command queue reports fewer places than the admitted command bound.
    #[error("command queue capacity {reported} is below the admitted {required}")]
    CommandCapacity {
        /// Places the admitted bound needs.
        required: MixedTransferPlaces,
        /// Places the queue reported.
        reported: MixedTransferPlaces,
    },
    /// The result queue reports fewer places than the admitted result bound.
    #[error("result queue capacity {reported} is below the admitted {required}")]
    ResultCapacity {
        /// Places the admitted bound needs.
        required: MixedTransferPlaces,
        /// Places the queue reported.
        reported: MixedTransferPlaces,
    },
    /// The command queue already holds packets.
    #[error("command queue is not empty: {occupied} occupied")]
    CommandOccupied {
        /// Packets the queue reported.
        occupied: MixedTransferPlaces,
    },
    /// The result queue already holds packets.
    #[error("result queue is not empty: {occupied} occupied")]
    ResultOccupied {
        /// Packets the queue reported.
        occupied: MixedTransferPlaces,
    },
    /// The control scope-reservation table cannot hold every channel of the sources.
    #[error("control bend table holds {width}; the sources have {required}")]
    ControlBendTable {
        /// Cells prepared.
        width: MixedTransferScopeCount,
        /// Channel scopes of the registered sources.
        required: MixedTransferScopeCount,
    },
    /// The audio bend table cannot hold every channel of the sources.
    #[error("audio bend table holds {width}; the sources have {required}")]
    AudioBendTable {
        /// Cells prepared.
        width: MixedTransferScopeCount,
        /// Channel scopes of the registered sources.
        required: MixedTransferScopeCount,
    },
    /// No transfer binding can be issued.
    #[error("the transfer binding space is exhausted")]
    BindingExhausted,
    /// The bridge's per-source stage lane cannot hold one onset and its protected
    /// key-up place, or exceeds the prepared bound.
    #[error("stage lane of {cells} is outside 2..={maximum}")]
    StageCells {
        /// Cells requested per source.
        cells: MixedTransferPlaces,
        /// The largest lane arm prepares.
        maximum: MixedTransferPlaces,
    },
    /// The bridge's declared held onsets per source are none, or exceed the pairing table
    /// arm prepares.
    #[error("a pairing table of {onsets} is outside 1..={maximum}")]
    HeldOnsets {
        /// Held onsets declared per source.
        onsets: MixedTransferPlaces,
        /// The largest pairing table arm prepares.
        maximum: MixedTransferPlaces,
    },
    /// One callback of the profile's largest block, in whole quanta, exceeds the forward
    /// horizon, so a forwarded packet could be refused as too far ahead. A valid profile
    /// never does this.
    #[error("a publication window of {block} in whole quanta exceeds the horizon {horizon}")]
    PublicationWindow {
        /// The profile's largest block.
        block: FrameCount,
        /// The profile's forward event horizon.
        horizon: FrameCount,
    },
    /// The one-shot owner refused the plan, candidate or profile.
    #[error(transparent)]
    Owner(MixedTransferOwnerRefusal),
}

/// An opaque one-shot owner arm refusal.
#[derive(Clone, Copy, Debug, PartialEq, Error)]
#[error(transparent)]
pub struct MixedTransferOwnerRefusal(MixedOneShotArmError);

/// An arm refusal with every input unchanged.
#[must_use]
pub struct MixedTransferArmRefusal<CQ, RQ> {
    reason: MixedTransferArmError,
    owner: MixedJoinedPrepared,
    candidate: MixedStampedCandidate,
    commands: CQ,
    results: RQ,
}

impl<CQ, RQ> std::fmt::Debug for MixedTransferArmRefusal<CQ, RQ> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedTransferArmRefusal")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

impl<CQ, RQ> MixedTransferArmRefusal<CQ, RQ> {
    /// Why arm refused.
    pub const fn reason(&self) -> MixedTransferArmError {
        self.reason
    }

    /// The prepared owner, the candidate and both unsplit host queues.
    pub fn into_parts(self) -> (MixedJoinedPrepared, MixedStampedCandidate, CQ, RQ) {
        (self.owner, self.candidate, self.commands, self.results)
    }
}

/// What one arm's control must cover: both bend tables, the source channel scopes they
/// must reach, and whether the arm carries Panic.
#[derive(Clone, Copy, Debug)]
struct MixedTransferShape {
    tables: MixedBendTables,
    scopes: MixedTransferScopeCount,
    panic: MixedPanicCapability,
}

/// The armed halves and the host's two bound wrappers.
#[must_use]
pub struct MixedTransferArmed<CQ, RQ>
where
    CQ: MixedTransferQueue<MixedTransferCommand>,
    RQ: MixedTransferQueue<MixedTransferResult>,
{
    /// The off-thread control half.
    pub control: MixedTransferControl,
    /// The audio half, to move to the callback thread.
    pub audio: MixedTransferAudio,
    /// The command producer and result consumer, lent to every control call.
    pub control_port: MixedTransferControlPort<CQ::Producer, RQ::Consumer>,
    /// The command consumer and result producer, lent to every callback.
    pub audio_port: MixedTransferAudioPort<CQ::Consumer, RQ::Producer>,
}

impl<CQ, RQ> std::fmt::Debug for MixedTransferArmed<CQ, RQ>
where
    CQ: MixedTransferQueue<MixedTransferCommand>,
    RQ: MixedTransferQueue<MixedTransferResult>,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedTransferArmed")
            .field("control", &self.control)
            .field("audio", &self.audio)
            .field("control_port", &self.control_port)
            .field("audio_port", &self.audio_port)
            .finish()
    }
}

impl MixedJoinedPrepared {
    /// Arm ADR-0078's host-owned transfer over the bound initial one-shot plan.
    ///
    /// Both queues must report at least the admitted command/result bound, which
    /// includes every place withheld for exact key-ups, and must be empty. Every refusal
    /// returns the owner, the candidate and both queues unsplit and untouched.
    ///
    /// # Errors
    ///
    /// A queue, bend-table, binding or one-shot owner refusal.
    pub fn arm_transfer_rehearsal<CQ, RQ>(
        self,
        candidate: MixedStampedCandidate,
        profile: &HostProfile,
        commands: CQ,
        results: RQ,
    ) -> Result<MixedTransferArmed<CQ, RQ>, Box<MixedTransferArmRefusal<CQ, RQ>>>
    where
        CQ: MixedTransferQueue<MixedTransferCommand>,
        RQ: MixedTransferQueue<MixedTransferResult>,
    {
        let tables = MixedBendTables::for_capacity(MixedIngressQueueCount(
            Self::transfer_capacity(profile).get(),
        ));
        self.arm_transfer_with_tables(candidate, profile, commands, results, tables)
    }

    /// The admitted command/result bound: every ordinary command place, including the
    /// places reserved for exact key-ups. Panic's protected place is not part of it. A
    /// bound no `usize` can hold saturates, so no reported capacity can meet it and arm
    /// refuses.
    fn transfer_capacity(profile: &HostProfile) -> MixedTransferPlaces {
        MixedTransferPlaces(
            profile
                .limits()
                .events()
                .queues()
                .performance_ingress_capacity()
                .as_usize()
                .unwrap_or(usize::MAX),
        )
    }

    /// [`Self::arm_transfer_rehearsal`] with explicit bend-table widths.
    pub(super) fn arm_transfer_with_tables<CQ, RQ>(
        self,
        candidate: MixedStampedCandidate,
        profile: &HostProfile,
        commands: CQ,
        results: RQ,
        tables: MixedBendTables,
    ) -> Result<MixedTransferArmed<CQ, RQ>, Box<MixedTransferArmRefusal<CQ, RQ>>>
    where
        CQ: MixedTransferQueue<MixedTransferCommand>,
        RQ: MixedTransferQueue<MixedTransferResult>,
    {
        self.arm_transfer_for_scopes(
            candidate,
            profile,
            commands,
            results,
            MixedTransferShape {
                tables,
                scopes: REHEARSAL_SCOPES,
                panic: MixedPanicCapability::Absent,
            },
        )
    }

    /// Arm with bend tables that must each cover the shape's source channel scopes. With
    /// Panic available both queues must also hold its protected place, one beyond the
    /// admitted bound, and the owner admits one compiled Panic cut per quantum within
    /// the release share.
    fn arm_transfer_for_scopes<CQ, RQ>(
        self,
        candidate: MixedStampedCandidate,
        profile: &HostProfile,
        commands: CQ,
        results: RQ,
        shape: MixedTransferShape,
    ) -> Result<MixedTransferArmed<CQ, RQ>, Box<MixedTransferArmRefusal<CQ, RQ>>>
    where
        CQ: MixedTransferQueue<MixedTransferCommand>,
        RQ: MixedTransferQueue<MixedTransferResult>,
    {
        let MixedTransferShape {
            tables,
            scopes,
            panic,
        } = shape;
        let ordinary = Self::transfer_capacity(profile);
        let required = match panic {
            MixedPanicCapability::Available => {
                MixedTransferPlaces(ordinary.get().saturating_add(1))
            }
            MixedPanicCapability::Absent => ordinary,
        };
        let control_width = MixedTransferScopeCount(tables.control.as_usize());
        let audio_width = MixedTransferScopeCount(tables.audio.as_usize());
        let refused = if commands.capacity() < required {
            Some(MixedTransferArmError::CommandCapacity {
                required,
                reported: commands.capacity(),
            })
        } else if results.capacity() < required {
            Some(MixedTransferArmError::ResultCapacity {
                required,
                reported: results.capacity(),
            })
        } else if commands.occupied() != MixedTransferPlaces::NONE {
            Some(MixedTransferArmError::CommandOccupied {
                occupied: commands.occupied(),
            })
        } else if results.occupied() != MixedTransferPlaces::NONE {
            Some(MixedTransferArmError::ResultOccupied {
                occupied: results.occupied(),
            })
        } else if control_width < scopes {
            Some(MixedTransferArmError::ControlBendTable {
                width: control_width,
                required: scopes,
            })
        } else if audio_width < scopes {
            Some(MixedTransferArmError::AudioBendTable {
                width: audio_width,
                required: scopes,
            })
        } else {
            None
        };
        if let Some(reason) = refused {
            return Err(Box::new(MixedTransferArmRefusal {
                reason,
                owner: self,
                candidate,
                commands,
                results,
            }));
        }
        let Some(transfer) = issue_transfer() else {
            return Err(Box::new(MixedTransferArmRefusal {
                reason: MixedTransferArmError::BindingExhausted,
                owner: self,
                candidate,
                commands,
                results,
            }));
        };
        let (control, mut audio) =
            match self.arm_one_shot_with(candidate, profile, Some(tables), panic) {
                Ok(halves) => halves,
                Err(refusal) => {
                    let refusal = *refusal;
                    return Err(Box::new(MixedTransferArmRefusal {
                        reason: MixedTransferArmError::Owner(MixedTransferOwnerRefusal(
                            refusal.reason,
                        )),
                        owner: refusal.owner,
                        candidate: refusal.candidate,
                        commands,
                        results,
                    }));
                }
            };
        let binding = MixedTransferBinding {
            owner: control.ingress_owner,
            transfer,
        };
        audio.transfer_binding = Some(binding);
        let (command_producer, command_consumer) = commands.split();
        let (result_producer, result_consumer) = results.split();
        Ok(MixedTransferArmed {
            control: MixedTransferControl {
                inner: control,
                binding,
                frontier: None,
                fault: None,
                next_origin: 0,
                reset_seal: None,
            },
            audio: MixedTransferAudio {
                boundary: Arc::new(AtomicU64::new(audio.audio.renderer.clock().as_u64())),
                inner: audio,
            },
            control_port: MixedTransferControlPort {
                binding,
                commands: command_producer,
                results: result_consumer,
                served_at: None,
            },
            audio_port: MixedTransferAudioPort {
                binding,
                commands: command_consumer,
                results: result_producer,
            },
        })
    }
}

/// The off-thread control half: the merger's ledger, journal and source frontier.
/// It cannot be sent to the audio thread.
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedTransferControl;
/// fn require_send<T: Send>() {}
/// require_send::<MixedTransferControl>();
/// ```
#[derive(Debug)]
#[must_use]
pub struct MixedTransferControl {
    inner: MixedOneShotControl,
    binding: MixedTransferBinding,
    /// The nominal stamp of the last consumed observation.
    frontier: Option<SampleTime>,
    /// The exact cause this half detected, if it detected one.
    fault: Option<MixedTransferFault>,
    next_origin: u64,
    reset_seal: Option<Arc<MixedResetSeal>>,
}

/// The audio half: renderer, occurrence table and ingress store. Send it to the callback.
#[derive(Debug)]
#[must_use]
pub struct MixedTransferAudio {
    inner: MixedOneShotAudio,
    /// The first sample no callback has rendered yet, published once per callback for
    /// the two-source bridge's peer fence. It only ever grows.
    boundary: Arc<AtomicU64>,
}

impl MixedTransferAudio {
    /// Render one callback, serving the lent command queue at each quantum boundary.
    ///
    /// # Errors
    ///
    /// A wrong output shape, or a terminal fault that silenced the complete block.
    pub fn render<C, R>(
        &mut self,
        port: &mut MixedTransferAudioPort<C, R>,
        output: AudioBlockMut<'_>,
    ) -> Result<(), MixedTransferRenderError>
    where
        C: MixedTransferConsumer<MixedTransferCommand>,
        R: MixedTransferProducer<MixedTransferResult>,
    {
        let rendered = self
            .inner
            .render_with(port, output)
            .map_err(MixedTransferRenderError::from);
        // One Release store per callback; the bridge's fence only needs a lower bound.
        self.boundary.store(
            self.inner.audio.renderer.clock().as_u64(),
            Ordering::Release,
        );
        rendered
    }

    /// The terminal fault this half recorded, if any.
    pub fn fault(&self) -> Option<MixedTransferRenderError> {
        self.inner.fault.map(MixedTransferRenderError::from)
    }

    /// The registered ingress store's counters.
    pub const fn ingress_counters(&self) -> IngressCounters {
        self.inner.test_ingress.counters()
    }

    /// Publication observations retained over this owner's complete lifetime. Reading
    /// between callbacks allocates nothing and does not reset the authoritative ledger.
    pub fn publication_usage(&self) -> MixedTransferPublicationUsage {
        MixedTransferPublicationUsage::of(&self.inner)
    }

    /// Resource peaks sampled at mutation sites, including transients spent before a
    /// callback returns. Conversion or byte arithmetic overflow is an explicit error.
    ///
    /// # Errors
    /// A counter or named storage amount cannot be represented.
    pub fn resource_usage(
        &self,
    ) -> Result<MixedTransferResourceUsage, crate::render::RehearsalUsageError> {
        MixedTransferResourceUsage::of(&self.inner)
    }

    /// Live release holds the ingress store has outstanding: one per accepted live note
    /// whose release edge has not yet been written or applied. A key-up under its pedal
    /// redeems none; the lift's release group redeems every sustained note's hold.
    pub const fn live_holds_outstanding(&self) -> EventCount {
        self.inner.test_ingress.holds_outstanding()
    }

    /// What host Panics have done so far. Callable between callbacks on the audio thread.
    pub fn panic_state(&self) -> MixedTransferPanicState {
        let report = self.inner.report();
        MixedTransferPanicState {
            live_released_after_panic: self.inner.occurrences.released_after_panic(),
            compiled_ended: report.compiled_panic_ended,
            compiled_groups: report.compiled_panic_groups,
            compiled_tombstones: report.compiled_panic_tombstones,
            compiled_released_after_panic: report.compiled_released_after_panic,
            compiled_controls_after_panic: report.compiled_controls_after_panic,
            parked: report.panic_parked,
        }
    }
}

impl MixedTransferControl {
    /// Attach the optional reset handshake when this arm becomes bridge-owned.
    pub(super) fn attach_reset_seal(&mut self, audio: &mut MixedTransferAudio) {
        let seal = Arc::new(MixedResetSeal::new());
        audio.inner.reset_seal = Some(Arc::clone(&seal));
        self.reset_seal = Some(seal);
    }

    pub(super) fn seal_reset_for(&self, successor: MixedTransferBinding) -> bool {
        let Some(seal) = &self.reset_seal else {
            return false;
        };
        let through = self.inner.last_ingress_id.map_or(0, |id| id.0);
        seal.publish(successor, through);
        true
    }

    pub(super) fn clear_reset_seal(&self) {
        if let Some(seal) = &self.reset_seal {
            seal.clear();
        }
    }

    /// The bridge's due Panic tries to cancel the published seal before audio decides
    /// it at `B`. `false` when audio already decided, or when no seal is published.
    pub(super) fn cancel_reset_seal(&self) -> bool {
        self.reset_seal.as_ref().is_some_and(|seal| seal.cancel())
    }

    /// The nominal stamp of the last consumed observation, if any.
    pub const fn frontier(&self) -> Option<SampleTime> {
        self.frontier
    }

    /// The fault this half detected, or `Remote` once it observes the other half's halt.
    pub fn fault(&self) -> Option<MixedTransferFault> {
        self.fault.or_else(|| {
            self.inner
                .ingress_halt
                .load(Ordering::Acquire)
                .then_some(MixedTransferFault::Remote)
        })
    }

    /// Record this half's exact cause and publish the halt.
    fn halt(&mut self, fault: MixedTransferFault) {
        if self.fault.is_none() {
            self.fault = Some(fault);
        }
        self.inner.ingress_halt.store(true, Ordering::Release);
    }

    fn origin(&mut self) -> MixedIngressOriginId {
        self.next_origin = self.next_origin.saturating_add(1);
        MixedIngressOriginId(self.next_origin)
    }

    /// The halt read, the endpoint and handle bindings, then the source order. Each
    /// refusal here precedes minting and every capacity, credit or handle check.
    fn precheck(
        &mut self,
        endpoint: MixedTransferBinding,
        handle: Option<MixedKeyUpHandle>,
        at: SampleTime,
    ) -> Result<(), MixedTransferRefusal> {
        if self.inner.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedTransferRefusal::Halted);
        }
        if endpoint != self.binding {
            self.halt(MixedTransferFault::ForeignEndpoint);
            return Err(MixedTransferRefusal::ForeignEndpoint);
        }
        if handle.is_some_and(|handle| handle.binding != self.binding) {
            self.halt(MixedTransferFault::ForeignHandle);
            return Err(MixedTransferRefusal::ForeignHandle);
        }
        if let Some(frontier) = self.frontier
            && at < frontier
        {
            self.halt(MixedTransferFault::SourceOrder { at, frontier });
            return Err(MixedTransferRefusal::SourceOrder { frontier });
        }
        Ok(())
    }

    /// The identity cause the inner merger recorded with its halt.
    fn identity_refusal(&mut self) -> MixedTransferRefusal {
        let (refusal, fault) = match self.inner.terminal_identity_fault {
            Some(MixedIngressIdentityFault::OccurrenceCells) => (
                MixedTransferRefusal::OccurrenceCellsExhausted,
                MixedTransferFault::OccurrenceCellsExhausted,
            ),
            Some(MixedIngressIdentityFault::CommandId) | None => (
                MixedTransferRefusal::CommandIdExhausted,
                MixedTransferFault::CommandIdExhausted,
            ),
        };
        self.halt(fault);
        refusal
    }

    /// The consumed observation's minted command, kept in the pending slot.
    fn push_failed<O>(&mut self, at: SampleTime) -> MixedTransferSubmitError<O> {
        self.frontier = Some(at);
        let pending = self
            .inner
            .ingress_command_pending
            .as_ref()
            .map(|command| (command.id, command.request));
        let id = pending.map_or(
            MixedTransferCommandId(self.inner.last_ingress_id.map_or(0, |id| id.0)),
            |(id, _)| MixedTransferCommandId::of(id),
        );
        let handle = match pending {
            Some((_, MixedIngressRequest::Onset { occurrence, .. }))
            | Some((_, MixedIngressRequest::Release { occurrence, .. })) => {
                Some(MixedKeyUpHandle {
                    binding: self.binding,
                    occurrence,
                })
            }
            _ => None,
        };
        self.halt(MixedTransferFault::CommandPush { id });
        MixedTransferSubmitError::PushFailed { id, handle }
    }

    /// The consumed observation's command `id`, which the merger's ledger refused.
    fn merger_ledger_fault<O>(
        &mut self,
        at: SampleTime,
        id: MixedIngressCommandId,
        handle: Option<MixedKeyUpHandle>,
    ) -> MixedTransferSubmitError<O> {
        self.frontier = Some(at);
        let id = MixedTransferCommandId::of(id);
        self.halt(MixedTransferFault::MergerLedger { id: Some(id) });
        MixedTransferSubmitError::MergerLedger { id, handle }
    }

    /// The handle the journaled command `id` names, if it names one.
    fn pending_handle(&self, id: MixedIngressCommandId) -> Option<MixedKeyUpHandle> {
        self.inner
            .ingress_journal
            .iter()
            .find(|entry| entry.id == id)
            .and_then(|entry| match entry.request {
                MixedIngressRequest::Onset { occurrence, .. }
                | MixedIngressRequest::Release { occurrence, .. } => Some(self.handle(occurrence)),
                _ => None,
            })
    }

    /// Submit one note-on, reserving its exact key-up's place before minting.
    ///
    /// # Errors
    ///
    /// A refusal returns the original observation; a failed physical push is terminal.
    pub fn submit_note_on<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
        note: MixedSourceNoteOn,
    ) -> Result<
        (MixedTransferCommandId, MixedKeyUpHandle),
        MixedTransferSubmitError<MixedSourceNoteOn>,
    >
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        let refused = |reason| MixedTransferSubmitError::Refused {
            reason,
            original: note,
        };
        self.precheck(port.binding, None, note.at)
            .map_err(refused)?;
        let origin = self.origin();
        let scope = MixedLiveScope::new(REHEARSAL_SOURCE, note.channel);
        match self
            .inner
            .submit_onset_via(port, origin, scope, note.at, note.key, note.velocity)
        {
            Ok((id, MixedIngressRequest::Onset { occurrence, .. })) => {
                self.frontier = Some(note.at);
                Ok((
                    MixedTransferCommandId::of(id),
                    MixedKeyUpHandle {
                        binding: self.binding,
                        occurrence,
                    },
                ))
            }
            // Defensive: the merger stages only an onset request for an onset. The
            // observation was consumed into the command, which keeps its custody.
            Ok((id, _)) => Err(self.merger_ledger_fault(note.at, id, None)),
            Err(MixedOnsetSubmitError::Halted) => Err(refused(MixedTransferRefusal::Halted)),
            Err(MixedOnsetSubmitError::Full) => Err(refused(MixedTransferRefusal::CommandCapacity)),
            // Defensive: the single-source API mints no Panic for a command to precede.
            Err(MixedOnsetSubmitError::BehindPanic(frontier)) => {
                self.halt(MixedTransferFault::SourceOrder {
                    at: note.at,
                    frontier,
                });
                Err(refused(MixedTransferRefusal::SourceOrder { frontier }))
            }
            Err(MixedOnsetSubmitError::BendCredit) => {
                Err(refused(MixedTransferRefusal::BendCredit))
            }
            Err(MixedOnsetSubmitError::IdentityExhausted) => {
                let reason = self.identity_refusal();
                Err(refused(reason))
            }
            Err(MixedOnsetSubmitError::PhysicalPush) => Err(self.push_failed(note.at)),
            Err(MixedOnsetSubmitError::LedgerDefect(id)) => {
                let handle = self.pending_handle(id);
                Err(self.merger_ledger_fault(note.at, id, handle))
            }
        }
    }

    /// Submit one channel bend. It is never coalesced or moved earlier.
    ///
    /// # Errors
    ///
    /// A refusal returns the original observation; a failed physical push is terminal.
    pub fn submit_bend<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
        bend: MixedSourceBend,
    ) -> Result<MixedTransferCommandId, MixedTransferSubmitError<MixedSourceBend>>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        let refused = |reason| MixedTransferSubmitError::Refused {
            reason,
            original: bend,
        };
        self.precheck(port.binding, None, bend.at)
            .map_err(refused)?;
        let origin = self.origin();
        let scope = MixedLiveScope::new(REHEARSAL_SOURCE, bend.channel);
        match self
            .inner
            .submit_bend_via(port, origin, scope, bend.at, bend.cents)
        {
            Ok((id, _)) => {
                self.frontier = Some(bend.at);
                Ok(MixedTransferCommandId::of(id))
            }
            Err(MixedBendSubmitError::Halted) => Err(refused(MixedTransferRefusal::Halted)),
            Err(MixedBendSubmitError::Full) => Err(refused(MixedTransferRefusal::CommandCapacity)),
            // Defensive: the single-source API mints no Panic for a command to precede.
            Err(MixedBendSubmitError::BehindPanic(frontier)) => {
                self.halt(MixedTransferFault::SourceOrder {
                    at: bend.at,
                    frontier,
                });
                Err(refused(MixedTransferRefusal::SourceOrder { frontier }))
            }
            Err(MixedBendSubmitError::ScopeCapacity) => {
                Err(refused(MixedTransferRefusal::BendScopes))
            }
            Err(MixedBendSubmitError::Credits { .. }) => {
                Err(refused(MixedTransferRefusal::BendCredit))
            }
            Err(MixedBendSubmitError::IdentityExhausted) => {
                let reason = self.identity_refusal();
                Err(refused(reason))
            }
            Err(MixedBendSubmitError::PhysicalPush) => Err(self.push_failed(bend.at)),
        }
    }

    /// Submit the exact key-up of an open handle into the place its onset reserved.
    ///
    /// # Errors
    ///
    /// A refusal returns the original observation; `NotOpen` is not terminal. A failed
    /// physical push leaves the handle releasing under the retained command.
    pub fn submit_key_up<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
        key_up: MixedSourceKeyUp,
    ) -> Result<MixedTransferCommandId, MixedTransferSubmitError<MixedSourceKeyUp>>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        let refused = |reason| MixedTransferSubmitError::Refused {
            reason,
            original: key_up,
        };
        self.precheck(port.binding, Some(key_up.handle), key_up.at)
            .map_err(refused)?;
        let origin = self.origin();
        match self
            .inner
            .submit_release_via(port, origin, key_up.handle.occurrence, key_up.at)
        {
            Ok((id, _)) => {
                self.frontier = Some(key_up.at);
                Ok(MixedTransferCommandId::of(id))
            }
            Err(MixedReleaseSubmitError::Halted) => Err(refused(MixedTransferRefusal::Halted)),
            Err(MixedReleaseSubmitError::NotOpen(_)) => Err(refused(MixedTransferRefusal::NotOpen)),
            // Defensive: the single-source API mints no Panic for a command to precede.
            Err(MixedReleaseSubmitError::BehindPanic(frontier)) => {
                self.halt(MixedTransferFault::SourceOrder {
                    at: key_up.at,
                    frontier,
                });
                Err(refused(MixedTransferRefusal::SourceOrder { frontier }))
            }
            Err(MixedReleaseSubmitError::ReservedPlaceUnavailable(_)) => {
                Err(refused(MixedTransferRefusal::ReleasePlace))
            }
            Err(MixedReleaseSubmitError::IdentityExhausted(_)) => {
                let reason = self.identity_refusal();
                Err(refused(reason))
            }
            Err(MixedReleaseSubmitError::PhysicalPush) => Err(self.push_failed(key_up.at)),
            Err(MixedReleaseSubmitError::ReservationDefect(_)) => {
                self.halt(MixedTransferFault::MergerLedger { id: None });
                Err(refused(MixedTransferRefusal::MergerLedger))
            }
            Err(MixedReleaseSubmitError::LedgerDefect(id)) => {
                Err(self.merger_ledger_fault(key_up.at, id, Some(key_up.handle)))
            }
        }
    }

    /// Pop and settle at most one result against the oldest outstanding command.
    ///
    /// # Errors
    ///
    /// The halt observed by the call's first read, or a terminal fault this call found.
    pub fn collect<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
    ) -> Result<Option<MixedTransferReport>, MixedTransferCollectError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        if self.inner.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedTransferCollectError::Halted(
                self.fault().unwrap_or(MixedTransferFault::Remote),
            ));
        }
        if port.binding != self.binding {
            self.halt(MixedTransferFault::ForeignEndpoint);
            return Err(MixedTransferCollectError::Fault(
                MixedTransferFault::ForeignEndpoint,
            ));
        }
        match self.inner.collect_ingress_result_via(port) {
            Ok(None) => Ok(None),
            Ok(Some(result)) => match self.report(&result, port.served_at.take()) {
                Some(report) => Ok(Some(report)),
                // Defensive: collection already matched the outcome kind to a request
                // this API minted, so no other pairing can settle.
                None => {
                    let id = MixedTransferCommandId::of(result.id);
                    let fault = MixedTransferFault::ResultProtocol {
                        received: id,
                        expected: Some(id),
                    };
                    self.halt(fault);
                    Err(MixedTransferCollectError::Fault(fault))
                }
            },
            Err(MixedIngressResultCollectError::Halted) => Err(MixedTransferCollectError::Halted(
                self.fault().unwrap_or(MixedTransferFault::Remote),
            )),
            Err(MixedIngressResultCollectError::Protocol { received, expected }) => {
                let fault = MixedTransferFault::ResultProtocol {
                    received: MixedTransferCommandId::of(received),
                    expected: expected.map(MixedTransferCommandId::of),
                };
                self.halt(fault);
                Err(MixedTransferCollectError::Fault(fault))
            }
        }
    }

    fn handle(&self, occurrence: LiveOccurrenceId) -> MixedKeyUpHandle {
        MixedKeyUpHandle {
            binding: self.binding,
            occurrence,
        }
    }

    fn lifted(&self, lifted: &MixedLifted) -> MixedTransferLifted {
        let mut members = [None; RELEASE_GROUP_CAPACITY];
        for (member, occurrence) in members.iter_mut().zip(lifted.retired()) {
            *member = Some(occurrence);
        }
        MixedTransferLifted {
            binding: self.binding,
            members,
        }
    }

    fn report(
        &self,
        result: &MixedIngressResult,
        served_at: Option<SampleTime>,
    ) -> Option<MixedTransferReport> {
        let outcome = match (result.request, result.outcome) {
            (MixedIngressRequest::Onset { occurrence, .. }, MixedIngressOutcome::Onset(onset)) => {
                MixedTransferOutcome::NoteOn {
                    handle: self.handle(occurrence),
                    result: onset,
                }
            }
            (
                MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Release(released),
            ) => MixedTransferOutcome::KeyUp {
                handle: self.handle(occurrence),
                result: released.map(|retired| match retired.cause {
                    MixedRetireCause::Released(identity) => MixedKeyUpRetired::Released(identity),
                    MixedRetireCause::AfterRefusal(refused) => {
                        MixedKeyUpRetired::AfterRefusal(refused)
                    }
                    MixedRetireCause::AfterPanic(identity) => {
                        MixedKeyUpRetired::AfterPanic(identity)
                    }
                }),
            },
            (
                MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Sustained(_),
            ) => MixedTransferOutcome::KeyUp {
                handle: self.handle(occurrence),
                result: Ok(MixedKeyUpRetired::Sustained),
            },
            (MixedIngressRequest::Bend { scope, .. }, MixedIngressOutcome::Bend(bent)) => {
                MixedTransferOutcome::Bend {
                    channel: scope.channel(),
                    result: bent
                        .map(|bent| MixedTransferBendApplied {
                            members: MixedTransferHeldCount(bent.written().count()),
                        })
                        .map_err(|refused| match refused {
                            MixedBendRefused::BendTableFull => MixedTransferBendRefused::BendTable,
                            MixedBendRefused::GroupCapacity => MixedTransferBendRefused::Fanout,
                            MixedBendRefused::Credits { .. } => MixedTransferBendRefused::Credits,
                            MixedBendRefused::Ingress(refused) => {
                                MixedTransferBendRefused::Ingress(refused)
                            }
                        }),
                }
            }
            (
                MixedIngressRequest::Onset { occurrence, .. }
                | MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Handle(_),
            ) => MixedTransferOutcome::HandleRefused {
                handle: self.handle(occurrence),
            },
            (
                MixedIngressRequest::Pedal {
                    scope, down: true, ..
                },
                MixedIngressOutcome::PedalDown(pressed),
            ) => MixedTransferOutcome::PedalDown {
                channel: scope.channel(),
                result: pressed
                    .map(|pressed| match pressed {
                        MixedPedalDown::Pressed => MixedTransferPedalDown::Pressed,
                        MixedPedalDown::AlreadyDown => MixedTransferPedalDown::AlreadyDown,
                    })
                    .map_err(MixedTransferPedalRefused::from),
            },
            (
                MixedIngressRequest::Pedal {
                    scope, down: false, ..
                },
                MixedIngressOutcome::Lift(lifted),
            ) => MixedTransferOutcome::Lift {
                channel: scope.channel(),
                result: lifted
                    .map(|lifted| self.lifted(&lifted))
                    .map_err(MixedTransferPedalRefused::from),
            },
            (MixedIngressRequest::Panic { .. }, MixedIngressOutcome::Panic(panicked)) => {
                MixedTransferOutcome::Panic {
                    result: panicked
                        .map(|panicked| self.panicked(&panicked))
                        .map_err(MixedTransferPanicRefused::from),
                }
            }
            _ => return None,
        };
        let at = result.request.nominal();
        // An applied Panic names its own execution sample, where its group and cut ran.
        let executed_at = match outcome {
            MixedTransferOutcome::Panic {
                result: Ok(panicked),
            } => panicked.executed_at,
            _ => served_at.map_or(at, |served| at.max(served)),
        };
        Some(MixedTransferReport {
            id: MixedTransferCommandId::of(result.id),
            at,
            executed_at,
            outcome,
        })
    }

    fn panicked(&self, panicked: &MixedPanicked) -> MixedTransferPanicked {
        let mut ended = [None; RELEASE_GROUP_CAPACITY];
        for (slot, end) in ended.iter_mut().zip(panicked.ended()) {
            *slot = Some(end);
        }
        MixedTransferPanicked {
            executed_at: panicked.executed_at(),
            binding: self.binding,
            ended,
        }
    }

    /// Stop both halves off-thread and account for every packet and handle.
    ///
    /// Every supplied wrapper's binding is checked first. A crossed pair or a foreign
    /// wrapper refuses the whole join and returns everything unchanged, with neither
    /// queue drained. With no foreign wrapper, a missing one selects the partial
    /// [`MixedTransferCustody::MissingEndpoint`] shutdown.
    ///
    /// # Errors
    ///
    /// A crossed audio half or a foreign wrapper.
    pub fn join<CP, RC, CC, RP>(
        self,
        audio: MixedTransferAudio,
        control_port: Option<MixedTransferControlPort<CP, RC>>,
        audio_port: Option<MixedTransferAudioPort<CC, RP>>,
    ) -> Result<MixedTransferTeardown, Box<MixedTransferJoinRefusal<CP, RC, CC, RP>>>
    where
        CP: MixedTransferProducer<MixedTransferCommand>,
        RC: MixedTransferConsumer<MixedTransferResult>,
        CC: MixedTransferConsumer<MixedTransferCommand>,
        RP: MixedTransferProducer<MixedTransferResult>,
    {
        let reason = if audio.inner.transfer_binding != Some(self.binding)
            || !mixed_collection_pair(&self.inner.control, &audio.inner)
        {
            Some(MixedTransferJoinError::CrossedPair)
        } else if control_port
            .as_ref()
            .is_some_and(|port| port.binding != self.binding)
            || audio_port
                .as_ref()
                .is_some_and(|port| port.binding != self.binding)
        {
            Some(MixedTransferJoinError::ForeignEndpoint)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(Box::new(MixedTransferJoinRefusal {
                reason,
                control: self,
                audio,
                control_port,
                audio_port,
            }));
        }
        Ok(self.teardown(audio, control_port, audio_port))
    }

    fn teardown<CP, RC, CC, RP>(
        mut self,
        mut audio: MixedTransferAudio,
        control_port: Option<MixedTransferControlPort<CP, RC>>,
        audio_port: Option<MixedTransferAudioPort<CC, RP>>,
    ) -> MixedTransferTeardown
    where
        RC: MixedTransferConsumer<MixedTransferResult>,
        CC: MixedTransferConsumer<MixedTransferCommand>,
    {
        let own = self.inner.ingress_owner;
        let control_fault = self.fault();
        let audio_fault = audio.fault();
        let outstanding: Vec<_> = self
            .inner
            .ingress_journal
            .iter()
            .map(|entry| MixedTransferCommandId::of(entry.id))
            .collect();
        // Own-owner packets account for journal entries; foreign ones are only named.
        let mut accounted = Vec::new();
        let mut name_command = |command: &MixedIngressCommand| {
            if command.owner == own {
                accounted.push(command.id);
            }
            MixedTransferCommandId::of(command.id)
        };
        let unsent = self.inner.ingress_command_pending.take();
        let unsent = unsent.as_ref().map(&mut name_command);
        let invalid_command = audio.inner.invalid_ingress_command.take();
        let invalid_command = invalid_command.as_ref().map(&mut name_command);
        // A Panic audio held unserved until its quantum opens leaves here, unapplied.
        let parked = audio.inner.parked_panic.take();
        let parked = parked.as_ref().map(&mut name_command);
        let mut unserved = Vec::new();
        let commands_unknown = audio_port.is_none();
        if let Some(mut port) = audio_port {
            while let Some(command) = port.commands.try_pop() {
                unserved.push(name_command(&command.0));
            }
            // The drained queue halves are destroyed here, off the audio thread.
            drop(port);
        }
        let mut name_result = |result: &MixedIngressResult| {
            if result.owner == own {
                accounted.push(result.id);
            }
            MixedTransferCommandId::of(result.id)
        };
        let pending_result = audio.inner.ingress_result_pending.take();
        let pending_result = pending_result.as_ref().map(&mut name_result);
        let invalid_result = self.inner.invalid_ingress_result.take();
        let invalid_result = invalid_result.as_ref().map(&mut name_result);
        let mut uncollected = Vec::new();
        let results_unknown = control_port.is_none();
        if let Some(mut port) = control_port {
            while let Some(result) = port.results.try_pop() {
                uncollected.push(name_result(&result.0));
            }
            drop(port);
        }
        let (custody, lost) = if commands_unknown || results_unknown {
            (
                MixedTransferCustody::MissingEndpoint {
                    commands_unknown,
                    results_unknown,
                },
                Vec::new(),
            )
        } else {
            let lost: Vec<_> = self
                .inner
                .ingress_journal
                .iter()
                .filter(|entry| !accounted.contains(&entry.id))
                .map(|entry| MixedTransferCommandId::of(entry.id))
                .collect();
            let custody = if lost.is_empty() {
                MixedTransferCustody::Complete
            } else {
                MixedTransferCustody::Lost
            };
            (custody, lost)
        };
        let handles = self
            .inner
            .occurrence_ledger
            .held_handles()
            .into_iter()
            .map(|(occurrence, hold)| {
                let state = match hold {
                    LedgerHold::Open => MixedTransferHandleState::Open,
                    LedgerHold::Releasing(id) => {
                        MixedTransferHandleState::Releasing(MixedTransferCommandId::of(id))
                    }
                    LedgerHold::Sustained => MixedTransferHandleState::Sustained,
                };
                (self.handle(occurrence), state)
            })
            .collect();
        let sounding_live = audio
            .inner
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .map(|snapshot| snapshot.live().iter().map(|note| note.identity).collect())
            .map_err(MixedTransferSnapshotFault);
        // The slot's candidate or retired list leaves with the teardown, off-thread.
        let drained = super::drain_offer_slot(&mut audio.inner);
        let activation = MixedActivationSlot::drained(&drained);
        drop(drained);
        let mut tombstones = 0_usize;
        for (_, state) in audio.inner.occurrences.occupied() {
            match state {
                MixedOccurrenceState::Sounding(_) | MixedOccurrenceState::Sustained(_) => {}
                MixedOccurrenceState::Refused(_)
                | MixedOccurrenceState::Panicked(_)
                | MixedOccurrenceState::Reset(_) => {
                    tombstones += 1;
                }
            }
        }
        let teardown = MixedTransferTeardown {
            custody,
            control_fault,
            audio_fault,
            outstanding,
            unsent,
            unserved,
            invalid_command,
            parked,
            pending_result,
            uncollected,
            invalid_result,
            lost,
            handles,
            release_places: MixedTransferPlaces(self.inner.release_reserved.as_usize()),
            sounding_live,
            tombstones: MixedTransferHeldCount(tombstones),
            counters: audio.inner.test_ingress.counters(),
            queued_entries: MixedTransferPlaces(audio.inner.test_ingress.len()),
            holds_outstanding: audio.inner.test_ingress.holds_outstanding(),
            publication_usage: audio.publication_usage(),
            resource_usage: audio.resource_usage(),
            activation,
        };
        // Both halves, their renderer, store, ledgers and event lists end here, off-thread.
        drop(audio);
        drop(self);
        teardown
    }
}

/// Why a join returned everything unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedTransferJoinError {
    /// The audio half belongs to another transfer.
    #[error("the control and audio halves belong to different transfers")]
    CrossedPair,
    /// A supplied wrapper belongs to another transfer.
    #[error("a supplied wrapper belongs to another transfer")]
    ForeignEndpoint,
}

/// A refused join with both halves and every supplied wrapper, undrained.
#[must_use]
pub struct MixedTransferJoinRefusal<CP, RC, CC, RP> {
    reason: MixedTransferJoinError,
    control: MixedTransferControl,
    audio: MixedTransferAudio,
    control_port: Option<MixedTransferControlPort<CP, RC>>,
    audio_port: Option<MixedTransferAudioPort<CC, RP>>,
}

impl<CP, RC, CC, RP> std::fmt::Debug for MixedTransferJoinRefusal<CP, RC, CC, RP> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedTransferJoinRefusal")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// Every part a refused join returns, in argument order.
pub type MixedTransferJoinParts<CP, RC, CC, RP> = (
    MixedTransferControl,
    MixedTransferAudio,
    Option<MixedTransferControlPort<CP, RC>>,
    Option<MixedTransferAudioPort<CC, RP>>,
);

impl<CP, RC, CC, RP> MixedTransferJoinRefusal<CP, RC, CC, RP> {
    /// Why the join refused.
    pub const fn reason(&self) -> MixedTransferJoinError {
        self.reason
    }

    /// Both halves and every supplied wrapper, unchanged.
    pub fn into_parts(self) -> MixedTransferJoinParts<CP, RC, CC, RP> {
        (self.control, self.audio, self.control_port, self.audio_port)
    }
}

/// Whether joined teardown could account for every outstanding command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedTransferCustody {
    /// Both wrappers returned and every outstanding command was found.
    Complete,
    /// Both wrappers returned, but some outstanding command was in no queue or slot.
    Lost,
    /// A wrapper was missing; a queue whose consumer is missing has unknown contents.
    MissingEndpoint {
        /// The command queue's consumer was not returned.
        commands_unknown: bool,
        /// The result queue's consumer was not returned.
        results_unknown: bool,
    },
}

/// What the merger still holds for one handle at teardown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedTransferHandleState {
    /// The onset holds its reserved key-up place.
    Open,
    /// The key-up was minted as this command and not yet settled.
    Releasing(MixedTransferCommandId),
    /// The key-up was served under its channel's pedal; the note sounds and holds its
    /// release hold until the lift. Only the two-source bridge carries pedal edges.
    Sustained,
}

/// Joined terminal teardown: every unsent, unserved and uncollected packet by ID, every
/// outstanding handle, and what was still sounding.
#[derive(Debug)]
#[must_use]
pub struct MixedTransferTeardown {
    custody: MixedTransferCustody,
    control_fault: Option<MixedTransferFault>,
    audio_fault: Option<MixedTransferRenderError>,
    outstanding: Vec<MixedTransferCommandId>,
    unsent: Option<MixedTransferCommandId>,
    unserved: Vec<MixedTransferCommandId>,
    invalid_command: Option<MixedTransferCommandId>,
    parked: Option<MixedTransferCommandId>,
    pending_result: Option<MixedTransferCommandId>,
    uncollected: Vec<MixedTransferCommandId>,
    invalid_result: Option<MixedTransferCommandId>,
    lost: Vec<MixedTransferCommandId>,
    handles: Vec<(MixedKeyUpHandle, MixedTransferHandleState)>,
    release_places: MixedTransferPlaces,
    sounding_live: Result<Vec<NoteIdentity>, MixedTransferSnapshotFault>,
    tombstones: MixedTransferHeldCount,
    counters: IngressCounters,
    queued_entries: MixedTransferPlaces,
    holds_outstanding: EventCount,
    publication_usage: MixedTransferPublicationUsage,
    resource_usage: Result<MixedTransferResourceUsage, crate::render::RehearsalUsageError>,
    activation: MixedActivationSlot,
}

impl MixedTransferTeardown {
    /// The owner's resource observations retained through joined destruction.
    ///
    /// # Errors
    /// A counter or named storage amount could not be represented at join.
    pub const fn resource_usage(
        &self,
    ) -> Result<MixedTransferResourceUsage, crate::render::RehearsalUsageError> {
        self.resource_usage
    }

    /// The joined owner's cumulative publication observations, retained after its
    /// renderer and arbiter have been dropped off-thread.
    pub const fn publication_usage(&self) -> MixedTransferPublicationUsage {
        self.publication_usage
    }

    /// Whether every outstanding command was accounted for.
    pub const fn custody(&self) -> MixedTransferCustody {
        self.custody
    }

    /// The control half's fault, if it detected or observed one.
    pub const fn control_fault(&self) -> Option<MixedTransferFault> {
        self.control_fault
    }

    /// The audio half's fault, if it recorded one.
    pub const fn audio_fault(&self) -> Option<MixedTransferRenderError> {
        self.audio_fault
    }

    /// Every command the control journal still expected a result for, oldest first.
    pub fn outstanding(&self) -> &[MixedTransferCommandId] {
        &self.outstanding
    }

    /// The minted command a failed physical push left in control's pending slot.
    pub const fn unsent(&self) -> Option<MixedTransferCommandId> {
        self.unsent
    }

    /// Commands drained from the command queue, oldest first.
    pub fn unserved(&self) -> &[MixedTransferCommandId] {
        &self.unserved
    }

    /// The command audio refused and kept in its invalid-command slot.
    pub const fn invalid_command(&self) -> Option<MixedTransferCommandId> {
        self.invalid_command
    }

    /// The Panic audio held unserved until the quantum of its execution stamp opened. It
    /// was never applied. Only the two-source bridge mints one.
    pub const fn parked(&self) -> Option<MixedTransferCommandId> {
        self.parked
    }

    /// The result a failed physical push left in audio's pending slot.
    pub const fn pending_result(&self) -> Option<MixedTransferCommandId> {
        self.pending_result
    }

    /// Results drained from the result queue, oldest first.
    pub fn uncollected(&self) -> &[MixedTransferCommandId] {
        &self.uncollected
    }

    /// The result control refused and kept in its invalid-result slot.
    pub const fn invalid_result(&self) -> Option<MixedTransferCommandId> {
        self.invalid_result
    }

    /// Outstanding commands found in no queue or slot. Empty unless custody is `Lost`.
    pub fn lost(&self) -> &[MixedTransferCommandId] {
        &self.lost
    }

    /// Every handle the merger still held, with its reservation state.
    pub fn handles(&self) -> &[(MixedKeyUpHandle, MixedTransferHandleState)] {
        &self.handles
    }

    /// Key-up places still reserved for open handles.
    pub const fn release_places(&self) -> MixedTransferPlaces {
        self.release_places
    }

    /// Live identities in the stopped renderer registry, including an unpublished key-up.
    /// A snapshot failure is explicit; it cannot be mistaken for no sounding notes.
    ///
    /// # Errors
    ///
    /// The renderer's stopped mixed registry could not be inspected.
    pub fn sounding_live(&self) -> Result<&[NoteIdentity], MixedTransferSnapshotFault> {
        self.sounding_live.as_deref().map_err(|error| *error)
    }

    /// Refusal and Panic tombstones still waiting for their key-ups.
    pub const fn tombstones(&self) -> MixedTransferHeldCount {
        self.tombstones
    }

    /// The registered ingress store's counters.
    pub const fn counters(&self) -> IngressCounters {
        self.counters
    }

    /// Ingress entries accepted and not yet published.
    pub const fn queued_entries(&self) -> MixedTransferPlaces {
        self.queued_entries
    }

    /// Live release holds the stopped store still had outstanding: accepted live notes,
    /// sounding or sustained, whose release edge was never written or applied.
    pub const fn holds_outstanding(&self) -> EventCount {
        self.holds_outstanding
    }

    /// What the audio half's activation slot held: a candidate that never adopted, or a
    /// retired one control never collected, with its receipt. Either was dropped here.
    pub const fn activation(&self) -> MixedActivationSlot {
        self.activation
    }
}

#[cfg(test)]
impl MixedTransferControl {
    pub(super) fn inner_mut(&mut self) -> &mut MixedOneShotControl {
        &mut self.inner
    }

    pub(super) const fn inner(&self) -> &MixedOneShotControl {
        &self.inner
    }
}

#[cfg(test)]
impl MixedTransferAudio {
    pub(super) fn inner_mut(&mut self) -> &mut MixedOneShotAudio {
        &mut self.inner
    }

    pub(super) const fn inner(&self) -> &MixedOneShotAudio {
        &self.inner
    }
}

impl MixedTransferAudio {
    /// The owner and the audio boundary its callbacks publish, for a reset wrapper's
    /// custody. The boundary moves with the owner, so a bridge sharing it keeps reading
    /// the same cell.
    pub(super) fn into_reset_parts(self) -> (MixedOneShotAudio, Arc<AtomicU64>) {
        (self.inner, self.boundary)
    }

    /// The inverse of [`Self::into_reset_parts`], off-thread, for joined teardown.
    pub(super) fn from_reset_parts(inner: MixedOneShotAudio, boundary: Arc<AtomicU64>) -> Self {
        Self { inner, boundary }
    }
}

impl MixedKeyUpHandle {
    /// The handle the arm bound to `binding` returned for `occurrence`, so a reset
    /// receipt names the fading owner's ended occurrences as its control knows them.
    pub(super) const fn of(binding: MixedTransferBinding, occurrence: LiveOccurrenceId) -> Self {
        Self {
            binding,
            occurrence,
        }
    }
}

#[cfg(test)]
impl MixedKeyUpHandle {
    pub(super) const fn occurrence(self) -> LiveOccurrenceId {
        self.occurrence
    }
}

#[cfg(test)]
impl MixedBendTables {
    pub(super) const fn widths(control: usize, audio: usize) -> Self {
        Self {
            control: MixedBendScopeCapacity(control),
            audio: MixedBendScopeCapacity(audio),
        }
    }
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
