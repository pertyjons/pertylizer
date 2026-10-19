//! A split-born mixed stream before ingress, activation or loop commands are enabled.
//!
//! This constructor consumes one bound plan and stream. Its two range minters share a
//! table identity but never share slots, and both belong to the audio half for the
//! owner's whole life. The off-thread control holds no generation state: it pairs
//! compiled note edges by bounded tokens, and the audio half mints each compiled
//! occurrence when its note-on publishes. It exposes no production mixed render or
//! offer while ADR-0075's combined-host laws remain open.

use std::{marker::PhantomData, rc::Rc, sync::Arc};

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use std::sync::atomic::AtomicU64;

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(all(test, feature = "simulated-ingress"))]
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};

use thiserror::Error;

#[cfg(all(test, feature = "simulated-ingress"))]
use crate::ingress::IngressCounters;
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use crate::ingress::{IngressPrepareError, IngressRefused, PerformanceIngress};
use crate::{
    admit::AdmissionError,
    diagnostics::CompileError,
    host::mixed_targets::{MixedInstancePartition, MixedTargetAdmission},
    identity::{
        CompiledRangeMinter, IdentityError, IdentityTable, LiveRangeMinter, NoteIdentity,
        ProducerId, Range, Resolution, TableId,
    },
    looping::LoopPassId,
    plan::{CompiledPlan, NoteSlot, PlanId},
    profile::HostProfile,
    publish::{ProducerClass, PublicationArbiter},
    quantities::{
        Cents, EventCount, HeldNoteCount, KeyIdentity, NoteVelocity, ParameterValue, QuantumCount,
        VoiceCount,
    },
    render::{
        EventEnvelope, EventPayload, MixedSoundingSnapshot, MixedSoundingSnapshotError,
        PreparedRenderer, ScopedParameterRestore, TimedEvent,
    },
    schedule::{
        AdmittedCompiledStream, Closed, CompiledEvent, CompiledPayload, OpenNote, OpenNotes,
        Opened, SchedulePrepareError,
    },
    time::{
        FrameCount, Located, PlanPosition, QuantumOffset, SampleTime, StreamAnchor, StreamEpoch,
        TimeSource, issue_epoch,
    },
    transport::{ActivationSequence, LoopInterval},
};

mod hot;
#[cfg(feature = "mixed-transfer-rehearsal")]
pub use transfer::continuation::{
    MixedContinuationOutcome, MixedContinuationPacket, MixedContinuationPrepareError,
    MixedContinuationRecovery, MixedContinuationRefusal, MixedContinuationRetired,
};
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
mod occurrence;
// Stage 7's whole-owner reset. Each owner carries its own endpoint: the private
// rehearsal's own rings, or, through the transfer rehearsal's public wrapper, the audio
// port each host-armed owner's own arm returned.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
mod reset;
#[cfg(feature = "mixed-transfer-rehearsal")]
mod transfer;

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use hot::MixedAudioQueues;
#[cfg(feature = "mixed-transfer-rehearsal")]
pub use reset::{
    MixedBridgeCompiledCut, MixedBridgeResetAudio, MixedBridgeResetEnd,
    MixedBridgeResetForeignPort, MixedBridgeResetLiveRefusal, MixedBridgeResetOwner,
    MixedBridgeResetReceipt, MixedBridgeResetRefusal, MixedBridgeResetRefused,
    MixedBridgeResetRenderError, MixedBridgeResetRetired, MixedBridgeResetState,
    MixedBridgeStopAccepted, MixedBridgeStopRefusal,
};
#[cfg(feature = "mixed-transfer-rehearsal")]
pub use transfer::{
    MixedActivationAdopted, MixedActivationCollectError, MixedActivationCollectRefusal,
    MixedActivationKind, MixedActivationLoop, MixedActivationOffer, MixedActivationOutcome,
    MixedActivationPrepareError, MixedActivationReceipt, MixedActivationRefusal,
    MixedActivationRetired, MixedActivationSlot, MixedBridgeAbandonRefusal, MixedBridgeArmed,
    MixedBridgeCommitRefusal, MixedBridgeControl, MixedBridgeError, MixedBridgeFault,
    MixedBridgeJoinError, MixedBridgeJoinParts, MixedBridgeJoinRefusal, MixedBridgeLifted,
    MixedBridgeOutcome, MixedBridgePanicEnd, MixedBridgePanicRefusal, MixedBridgePanicked,
    MixedBridgeReport, MixedBridgeResetPhase, MixedBridgeRetiredControl,
    MixedBridgeRetiredJoinRefusal, MixedBridgeRetiredTeardown, MixedBridgeSealRefusal,
    MixedBridgeSealRefused, MixedBridgeTeardown, MixedBridgeWait, MixedCommandPlace,
    MixedKeyUpHandle, MixedKeyUpRetired, MixedOnsetRefusal, MixedPublication,
    MixedPublicationFault, MixedPublicationGate, MixedPublicationPublishError,
    MixedPublicationReceiver, MixedPublicationRefusal, MixedPublicationSender, MixedPumpStep,
    MixedRetiredHandle, MixedSourceBend, MixedSourceCustody, MixedSourceFault, MixedSourceHandoff,
    MixedSourceKeyUp, MixedSourceNoteOn, MixedSourcePacketId, MixedSourcePacketIdError, MixedSplit,
    MixedStaged, MixedTransferArmError, MixedTransferArmRefusal, MixedTransferArmed,
    MixedTransferAudio, MixedTransferAudioPort, MixedTransferBendApplied, MixedTransferBendRefused,
    MixedTransferCollectError, MixedTransferCommand, MixedTransferCommandId, MixedTransferConsumer,
    MixedTransferControl, MixedTransferControlPort, MixedTransferCustody, MixedTransferFault,
    MixedTransferFence, MixedTransferHandleState, MixedTransferHeldCount, MixedTransferJoinError,
    MixedTransferJoinParts, MixedTransferJoinRefusal, MixedTransferLifted, MixedTransferOutcome,
    MixedTransferOwnerRefusal, MixedTransferPanicEnd, MixedTransferPanicRefused,
    MixedTransferPanicState, MixedTransferPanicked, MixedTransferPedalDown,
    MixedTransferPedalRefused, MixedTransferPlaces, MixedTransferProducer,
    MixedTransferPublicationUsage, MixedTransferQueue, MixedTransferRefusal,
    MixedTransferRenderError, MixedTransferReport, MixedTransferResourceUsage, MixedTransferResult,
    MixedTransferScopeCount, MixedTransferSnapshotFault, MixedTransferSources,
    MixedTransferSourcesError, MixedTransferStreamFault, MixedTransferSubmitError,
    MixedTransferTeardown,
};
#[cfg(feature = "mixed-transfer-rehearsal")]
use transfer::{MixedResetSeal, MixedTransferBinding};
#[allow(dead_code, reason = "the production mixed host has not been activated")]
mod source_set;

#[cfg(all(test, feature = "simulated-ingress"))]
use occurrence::MixedOccurrenceState;
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use occurrence::MixedPanicSubmitError;
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use occurrence::{
    LiveOccurrenceId, MixedBendCreditReturn, MixedBendResult, MixedBendSubmitError,
    MixedLiftResult, MixedLiveScope, MixedOccurrenceLedger, MixedOccurrenceTable,
    MixedOnsetSubmitError, MixedPanicResult, MixedPedalDown, MixedPedalEdge, MixedPedalLedger,
    MixedPedalRefused, MixedPedalSubmitError, MixedReleaseSubmitError, MixedRetired,
};

/// Why a bound mixed stream could not be prepared.
#[derive(Debug, Error)]
pub enum MixedStreamOpenError {
    /// Ordinary renderer, epoch or identity preparation refused.
    #[error(transparent)]
    Compile(#[from] CompileError),
    /// The identity partition disagrees with the target binding.
    #[error("mixed producer spans differ from the admitted target binding")]
    Partition,
}

/// Why the bound first schedule could not be sealed before any mixed rendering exists.
#[derive(Debug, Error)]
pub enum MixedInitialPrepareError {
    /// Placement or compiled note pairing refused, including over-emission of the
    /// compiled span.
    #[error(transparent)]
    Schedule(#[from] SchedulePrepareError),
    /// A stealing policy appeared despite mixed no-stealing admission.
    #[error("mixed preparation encountered a stealing policy")]
    UnexpectedSteal,
    /// Defensive check: a compiled parameter or controller writer escaped target admission.
    #[error("compiled event {event_index} writes outside note targets")]
    UnexpectedWriter { event_index: usize },
}

/// A refusal while reconstructing only the bound compiled history before a seek.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MixedHistoryPrepareError {
    /// No complete quantum boundary can follow the requested sample time.
    #[error("no quantum boundary follows requested time {at}")]
    BoundaryUnrepresentable { at: SampleTime },
    /// The checked plan and parameter partition no longer agree.
    #[error("mixed restoration partition disagrees with the bound plan")]
    Partition,
    /// A compiled parameter or controller writer appeared despite target admission.
    #[error("compiled history event {event_index} writes outside note targets")]
    UnexpectedWriter { event_index: usize },
    /// Defensive reconstruction check: the sealed prefix exceeded its admitted note range.
    #[error("compiled history event {event_index} exceeds its admitted note range")]
    ProducerCapacity { event_index: usize },
    /// Defensive reconstruction check: a sealed prefix release has no matching note-on.
    #[error("compiled history release {event_index} has no matching note-on")]
    UnmatchedRelease { event_index: usize },
    /// The destination-open snapshot exceeds the held-note count's range.
    #[error("mixed history open-note count cannot be represented")]
    OpenCountUnrepresentable,
    /// The scoped restoration batch exceeds the event count's range.
    #[error("mixed history restoration-event count cannot be represented")]
    RestorationCountUnrepresentable,
}

/// Why the bound compiled suffix could not be classified without publishing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MixedSuffixPrepareError {
    /// The candidate belongs to another bound plan, epoch or identity table.
    #[error("mixed suffix candidate belongs to another prepared owner")]
    ForeignCandidate,
    /// Defensive check: its saved strict-prefix index disagrees with the bound stream.
    #[error("mixed suffix prefix index disagrees with the bound stream")]
    PrefixMismatch,
    /// Defensive check: a compiled writer escaped target admission.
    #[error("compiled suffix event {event_index} writes outside note targets")]
    UnexpectedWriter { event_index: usize },
    /// Defensive check: the no-stealing suffix exceeded the compiled range capacity.
    #[error("compiled suffix event {event_index} exceeds its admitted note range")]
    ProducerCapacity { event_index: usize },
    /// Defensive check: neither the suffix nor the sealed prefix owns this release.
    #[error("compiled suffix release {event_index} has no matching note-on")]
    UnmatchedRelease { event_index: usize },
    /// Defensive check: neither the suffix nor the sealed prefix owns this expression.
    #[error("compiled suffix expression {event_index} has no matching note-on")]
    UnmatchedExpression { event_index: usize },
    /// One of the private suffix counts cannot fit its event-count type.
    #[error("mixed suffix count cannot be represented")]
    CountUnrepresentable,
}

/// Why a private bound suffix could not be placed and paired into compiled tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MixedStampPrepareError {
    /// This selection came from another prepared owner.
    #[error("mixed suffix selection belongs to another prepared owner")]
    ForeignCandidate,
    /// Its private source indices no longer select a strict suffix in source order,
    /// or select a writer the suffix classifier refuses.
    #[error("mixed suffix source selection is invalid at index {event_index}")]
    InvalidSelection {
        /// The rejected source index.
        event_index: usize,
    },
    /// Placement or token pairing refused with the original source index preserved.
    #[error(transparent)]
    Schedule(#[from] SchedulePrepareError),
    /// A private count exceeds its typed representation.
    #[error("mixed suffix stamped count cannot be represented")]
    CountUnrepresentable,
    /// The private restoration batch no longer has its admitted requested-time shape.
    #[error("mixed scoped restoration batch disagrees with the requested-time candidate")]
    InvalidRestoration,
    /// The complete private list lost its nondecreasing stamped-time order.
    /// The index names the combined restoration-plus-suffix list.
    #[error("mixed event {event_index} precedes the previous stamped event")]
    EventOrder { event_index: usize },
}

/// Why a privately stamped list cannot become the one-shot audio capsule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum MixedCapsulePrepareError {
    /// Defensive check: the privately stamped `INITIAL` baseline must have a successor.
    #[error("mixed activation sequence has no successor")]
    SequenceExhausted,
}

/// Why an armed control refused to prepare a later Start or seek candidate.
/// Every refusal leaves the control, and both audio-owned minters, unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(dead_code)] // Only private tests and the transfer rehearsal prepare a later candidate.
pub(crate) enum MixedSubsequentPrepareError {
    /// The caller's superseded sequence is not the one this control holds in force.
    #[error("the candidate supersedes {supersedes}, but {in_force} is in force")]
    Superseded {
        supersedes: ActivationSequence,
        in_force: ActivationSequence,
    },
    /// Defensive check: the in-force sequence has no successor to issue.
    #[error("mixed activation sequence has no successor")]
    SequenceExhausted,
    /// The request would share or precede the boundary of the transition in force,
    /// and one boundary carries at most one transport change.
    #[error("candidate at {requested} does not follow the in-force boundary {boundary}")]
    NotAfterInForce {
        requested: SampleTime,
        boundary: SampleTime,
    },
    /// A loop is entered inside `[start, end]`; entering exactly at the end starts the
    /// first pass at the start, as the exclusive loop owner does. Other positions are
    /// refused, never clamped.
    #[error("loop entry {position} lies outside [{start}, {end}]")]
    LoopEntry {
        position: PlanPosition,
        start: PlanPosition,
        end: PlanPosition,
    },
    #[error(transparent)]
    History(#[from] MixedHistoryPrepareError),
    #[error(transparent)]
    Suffix(#[from] MixedSuffixPrepareError),
    #[error(transparent)]
    Stamp(#[from] MixedStampPrepareError),
}

/// Why an armed control refused to seal a later candidate as an offer.
/// Every refusal returns the candidate unchanged and leaves the control unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
#[allow(dead_code)] // Only private tests and the transfer rehearsal seal an offer.
pub(crate) enum MixedOfferPrepareError {
    /// The candidate belongs to another bound plan, epoch or table.
    #[error("mixed offer candidate belongs to another prepared owner")]
    ForeignCandidate,
    /// The candidate does not supersede the sequence this control holds in force.
    #[error("the candidate supersedes {supersedes}, but {in_force} is in force")]
    Superseded {
        supersedes: ActivationSequence,
        in_force: ActivationSequence,
    },
    /// The offer would share or precede the boundary of the transition in force.
    #[error("offer boundary {effective} does not follow the in-force boundary {boundary}")]
    NotAfterInForce {
        effective: SampleTime,
        boundary: SampleTime,
    },
    /// The candidate cannot be displaced to the requested boundary.
    #[error(transparent)]
    Timing(#[from] MixedEffectiveTimeError),
    /// The candidate does not fit the arm profile at that boundary's displacement.
    #[error(transparent)]
    Admission(#[from] MixedOneShotAdmissionError),
    /// Defensive check: the superseded sequence has no successor.
    #[error(transparent)]
    Capsule(#[from] MixedCapsulePrepareError),
    /// The loop's entry junction or periodic steady state does not fit the arm profile.
    #[error("mixed loop {class:?} admission failed: {source}")]
    LoopAdmission {
        class: ProducerClass,
        source: AdmissionError,
    },
    /// The loop's template, entry junction or first wrap cannot be represented, or its
    /// template does not match the candidate it was prepared with.
    #[error("mixed loop shape is not representable")]
    LoopShape,
}

/// Why the audio half refused a later candidate, at its offer or when its boundary
/// opened. A refusal changes no program, minter, registry row or slot; the owned
/// candidate goes back to the caller, and the renderer counts the refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(dead_code)] // Only private tests and the transfer rehearsal offer a later candidate.
pub(crate) enum MixedOfferRefused {
    #[error("mixed audio owner is terminally faulted")]
    Faulted,
    #[error("mixed offer belongs to another plan, epoch or table")]
    ForeignCandidate,
    /// The single slot already holds a candidate waiting for its boundary.
    #[error("a later mixed candidate is already pending")]
    AlreadyPending,
    /// The single slot still holds a retired candidate control has not taken.
    #[error("a retired mixed candidate has not been collected")]
    RetiredUncollected,
    /// The candidate does not supersede the sequence in force at its boundary.
    #[error("the offer supersedes {supersedes}, but {in_force} is in force at its boundary")]
    Superseded {
        supersedes: ActivationSequence,
        in_force: ActivationSequence,
    },
    /// The offer would share or precede the boundary of the transition in force or
    /// still pending from arm.
    #[error("offer boundary {effective} does not follow the in-force boundary {boundary}")]
    NotAfterInForce {
        effective: SampleTime,
        boundary: SampleTime,
    },
    /// The offer's boundary quantum has already opened.
    #[error("offer boundary {effective} precedes the render clock {clock}")]
    BoundaryPassed {
        effective: SampleTime,
        clock: SampleTime,
    },
    /// A compiled Stop is pending at or before the offer's boundary. At the same
    /// boundary the two would be two transport changes; at a later one the Stop would
    /// cancel the candidate before it could adopt.
    #[error("offer boundary {effective} does not precede the pending Stop at {stop}")]
    StopPending {
        stop: SampleTime,
        effective: SampleTime,
    },
    /// A Start needs a stopped compiled program at its boundary; a running one takes
    /// a seek instead.
    #[error("a Start needs a stopped compiled program")]
    StartWhileRunning,
}

/// What an adopted later candidate does to the compiled transport at its boundary.
/// Both kinds adopt the same way: boundary release, program and anchor swap, then
/// scoped restoration, all compiled-scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Only private tests and the transfer rehearsal offer a later candidate.
pub(crate) enum MixedOfferKind {
    /// Keep the transport as it stands at the boundary. A running program publishes
    /// the new list; a stopped one publishes only its scoped restoration.
    Seek,
    /// Resume a stopped program: the new list publishes in full from the boundary.
    Start,
}

/// Whether this private arm exposes a Panic command and therefore needs one
/// compiled cut's release-share reservation per quantum.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MixedPanicCapability {
    Available,
    Absent,
}

/// One sealed later candidate on its way to the audio half's single offer slot.
///
/// Only [`MixedOneShotControl::seal_offer`] builds one, after admitting it at exactly
/// its boundary's displacement, so audio never holds an unadmitted candidate. Its
/// tokens name no identity, so dropping it off-thread changes no minter.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // Only private tests and the transfer rehearsal offer a later candidate.
pub(crate) struct MixedCandidateOffer(Box<MixedOfferCell>);

/// The same allocation coming back from the slot, with the bounded receipt that says
/// whether it adopted. After an adoption its event list is the retired one.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // Only private tests and the transfer rehearsal collect an offer.
pub(crate) struct MixedRetiredOffer {
    cell: Box<MixedOfferCell>,
    receipt: MixedOfferReceipt,
}

/// The boxed custody of one later candidate. Audio swaps its event list at adoption
/// and releases the old compiled occurrences into its own ended-note storage, so the
/// boundary allocates nothing and overwrites no earlier record.
#[derive(Debug)]
struct MixedOfferCell {
    #[cfg(feature = "mixed-transfer-rehearsal")]
    activation_custody: Option<transfer::activation::ActivationCustody>,
    candidate: MixedAudioCandidate,
    timing: MixedEffectiveTiming,
    ended: Vec<Option<crate::identity::EndedNote>>,
    /// The prefix of `ended` the renderer's boundary release filled, recorded before
    /// custody ends so a fault after the release cannot hide it from teardown.
    released: HeldNoteCount,
    /// Whether the clock had passed the requested time when audio accepted the offer.
    late: bool,
    kind: MixedOfferKind,
    /// The loop the candidate enters, if any. Adoption swaps it with the program in
    /// force, so after an adoption this holds the retired loop, dropped off-thread.
    loop_program: Option<Box<MixedLoopProgram>>,
}

/// What the audio half reports for one offered candidate. Fixed-size and `Copy`, so
/// the boundary that writes it allocates nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
#[allow(dead_code)] // Only private tests and the transfer rehearsal read a receipt.
pub(crate) struct MixedOfferReceipt {
    pub(crate) supersedes: ActivationSequence,
    pub(crate) sequence: ActivationSequence,
    pub(crate) late: bool,
    pub(crate) kind: MixedOfferKind,
    pub(crate) outcome: MixedOfferOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Only private tests and the transfer rehearsal read a receipt.
pub(crate) enum MixedOfferOutcome {
    Adopted(MixedOfferAdopted),
    /// Refused when its boundary opened; the program in force did not change.
    Refused(MixedOfferRefused),
    /// A compiled Stop applied at `stop`, at or before the candidate's boundary, took
    /// it out of the slot unadopted. The program in force did not change.
    Cancelled {
        stop: SampleTime,
    },
}

/// One confirmed later adoption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Only private tests and the transfer rehearsal read a receipt.
pub(crate) struct MixedOfferAdopted {
    pub(crate) effective: SampleTime,
    /// The mapping now in force.
    pub(crate) anchor: StreamAnchor,
    /// The mapping the retired list belonged to.
    pub(crate) retired_anchor: StreamAnchor,
    /// The first retired event that never published.
    pub(crate) retired_unconsumed_from: usize,
    pub(crate) retired_event_count: usize,
    /// Sounding compiled occurrences the boundary release ended.
    pub(crate) released: HeldNoteCount,
    /// Whether the adopted program is stopped, so it published only its restoration.
    pub(crate) stopped: bool,
    /// The loop the adopted program plays, if any.
    pub(crate) loop_interval: Option<LoopInterval>,
    /// The pass the retired program was in, if it was a loop. Once that loop had
    /// wrapped, `retired_unconsumed_from` is the entry list's length: the entry list
    /// was spent, and the rest of the pass it was in never published.
    pub(crate) retired_pass: Option<LoopPassId>,
}

/// One loop's repeating pass, prepared off-thread with its entry candidate.
///
/// The template is the bound stream's `[start, end)` reconstructed from `start` and
/// stamped from engine time zero: scoped restoration first, then the pass's tokenized
/// events, each before the loop's length. Every pass starts from an empty token table,
/// because a wrap ends or frees every token, so the template's own pairing is the
/// per-pass polyphony check. Notes still open at `end` are ended by the wrap with
/// Session-charged `Off`s, as ADR-0065's exclusive loop does.
#[derive(Debug)]
pub(crate) struct MixedLoopProgram {
    interval: LoopInterval,
    events: Vec<MixedCompiledEvent>,
    restoration_count: EventCount,
    /// Tokens the template leaves open at `end`: the `Off`s each later wrap charges.
    end_open: HeldNoteCount,
    /// Crossing releases and expressions the template omits, per pass.
    omitted_releases: EventCount,
    omitted_expressions: EventCount,
}

/// A later candidate that enters a loop: an ordinary entry list cut at the loop's end,
/// plus its repeating template. Only [`MixedOneShotControl::prepare_loop`] builds one,
/// and only [`MixedOneShotControl::seal_loop_offer`] admits it, so the armed first
/// transition can never carry a loop.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // Only private tests and the transfer rehearsal enter a loop.
pub(crate) struct MixedLoopCandidate {
    entry: MixedStampedCandidate,
    program: Box<MixedLoopProgram>,
}

impl MixedLoopCandidate {
    /// Crossing releases and expressions each repeating pass omits, as counted when
    /// its template was reconstructed from the loop start.
    #[allow(dead_code)] // Only private tests read the template's omissions so far.
    pub(crate) const fn pass_omissions(&self) -> (EventCount, EventCount) {
        (
            self.program.omitted_releases,
            self.program.omitted_expressions,
        )
    }
}

/// Where the loop in force stands on the audio half. `Copy`, so a wrap writes no heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MixedLoopCursor {
    pass: LoopPassId,
    /// The sample at which the current pass ends and the next starts.
    next_wrap: SampleTime,
    /// Whether the list cursor reads the repeating template rather than the entry list.
    repeating: bool,
    /// The engine time the current repeating pass's template is shifted to.
    pass_shift: FrameCount,
}

/// The single offer slot, used in both directions: a candidate waits in it for its
/// boundary, and the same allocation waits in it, retired, until control takes it.
#[derive(Debug, Default)]
enum MixedOfferSlot {
    #[default]
    Empty,
    Pending(MixedCandidateOffer),
    Retired(MixedRetiredOffer),
}

/// What the offer slot holds, for a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Only private tests and the transfer rehearsal read the slot.
pub(crate) enum MixedOfferSlotState {
    Empty,
    Pending {
        supersedes: ActivationSequence,
        effective: SampleTime,
        kind: MixedOfferKind,
    },
    Retired(MixedOfferReceipt),
}

/// The offer slot's contents, moved out whole when a joined pair is collected, so no
/// candidate and no boundary record is dropped without its owner seeing it.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // Only private tests collect a joined pair with an offer.
pub(crate) enum MixedDrainedOffer {
    Empty,
    /// A candidate that never adopted. It minted nothing, so withdrawing it is free.
    Withdrawn(MixedCandidateOffer),
    /// A retired candidate and its receipt that control had not taken. An adoption is
    /// already reflected in the sequence and anchor collection hands to control.
    Retired(MixedRetiredOffer),
}

/// Why control refused to collect a retired offer. The offer comes back unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(dead_code)] // Only private tests and the transfer rehearsal collect an offer.
pub(crate) enum MixedOfferCollectError {
    #[error("retired mixed offer belongs to another prepared owner")]
    ForeignCandidate,
    /// Defensive check: an adoption must supersede what this control holds in force.
    #[error("the adoption superseded {supersedes}, but {in_force} is in force")]
    Superseded {
        supersedes: ActivationSequence,
        in_force: ActivationSequence,
    },
}

/// Why the private one-shot audio owner could not be armed off-thread.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub(crate) enum MixedOneShotArmError {
    /// The candidate belongs to another bound plan, stream or table.
    #[error("mixed one-shot candidate belongs to another prepared owner")]
    ForeignCandidate,
    /// Defensive check: bound halves born from one constructor cannot disagree.
    #[error("mixed one-shot control and audio halves disagree")]
    ForeignAudio,
    /// Defensive check: the joined owner has no render path that can fault it.
    #[error("mixed one-shot renderer needs reprepare")]
    FaultedRenderer,
    /// Defensive check: private stamping always supplies `INITIAL`.
    #[error("mixed one-shot candidate does not supersede INITIAL")]
    WrongSequence,
    /// Defensive check: private history preparation and a quantum renderer clock
    /// each have a representable following boundary.
    #[error("no mixed one-shot quantum boundary follows {at}")]
    BoundaryUnrepresentable { at: SampleTime },
    /// The complete candidate cannot be displaced to the selected boundary.
    #[error(transparent)]
    Timing(#[from] MixedEffectiveTimeError),
    /// Defensive check: a stamped `INITIAL` candidate lost its successor.
    #[error(transparent)]
    Capsule(#[from] MixedCapsulePrepareError),
    /// Prepared release and timed-control storage cannot cover the bound span.
    #[error(transparent)]
    Storage(#[from] crate::render::MixedBoundaryStorageError),
    /// The closed private render schedule cannot be admitted before arm.
    #[error(transparent)]
    Admission(#[from] MixedOneShotAdmissionError),
    /// The private ingress queue could not be bound to this owner.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error(transparent)]
    Ingress(#[from] IngressPrepareError),
    /// The ingress queue cannot withhold a bend-credit budget for every live hold and
    /// still realize every declared hold with its release plus the final onset's entry.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error(
        "mixed ingress depth {capacity} cannot serve {holds} live holds; need {required}",
        capacity = .capacity.get(),
        holds = .holds.get(),
        required = .required.get()
    )]
    BendCreditCapacity {
        capacity: EventCount,
        holds: EventCount,
        required: EventCount,
    },
    /// A sustain lift ends its scope's sustained occurrences, and Panic every live
    /// occurrence, as one release group, so the live hold entitlement may not exceed
    /// one group's members.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("{holds} live holds exceed the {group}-member sustain lift group")]
    SustainGroupCapacity { holds: EventCount, group: usize },
    /// Defensive check: the prepared ingress store has no place, or its depth plus the
    /// live hold entitlement cannot be represented as an occurrence-table width.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("the prepared mixed ingress depth cannot size the occurrence tables")]
    IngressWidth,
    /// Panic ends every open compiled occurrence in `ceil(span / 8)` Release-charged
    /// groups beside a full live release backlog in its quantum, so the release share
    /// must cover the ingress depth plus that many groups.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error(
        "release share {share} cannot cover ingress depth {capacity} plus a compiled Panic \
         cut of {span} occurrences; need {required}",
        share = .share.get(),
        capacity = .capacity.get(),
        span = .compiled_span.get(),
        required = .required.get()
    )]
    CompiledPanicReleaseShare {
        share: EventCount,
        capacity: EventCount,
        compiled_span: HeldNoteCount,
        required: EventCount,
    },
}

/// Off-thread admission for the closed private one-shot schedule.
/// No production live ingress or other Session contributor is attached yet.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub(crate) enum MixedOneShotAdmissionError {
    #[error("mixed one-shot profile differs from the bound render plan")]
    ProfileMismatch,
    #[error("mixed one-shot candidate count disagrees with its event list")]
    CandidateCount,
    #[error("mixed one-shot event {event_index} violates the restoration/suffix split")]
    CandidateShape { event_index: usize },
    #[error("mixed restoration needs {needed} Session credits, but has {available}")]
    SessionShare {
        needed: EventCount,
        available: EventCount,
    },
    #[error("mixed suffix at {at} needs {needed} Compiled credits, but has {available}")]
    CompiledShare {
        at: SampleTime,
        needed: EventCount,
        available: EventCount,
    },
    #[error("mixed restoration spans {rows:?} rows, above fanout {admitted:?}")]
    TimedFanout {
        rows: VoiceCount,
        admitted: VoiceCount,
    },
    #[error(transparent)]
    Arbiter(#[from] crate::profile::ProfileError),
}

/// Failure of the private one-shot audio owner. All but output shape are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
#[allow(dead_code)] // Only the private rehearsal returns this until mixed hosting lands.
pub(crate) enum MixedOneShotRenderError {
    #[error("mixed one-shot output does not match the bound profile")]
    OutputShape,
    #[error("mixed one-shot audio owner is terminally faulted")]
    Faulted,
    #[error("mixed one-shot renderer and capsule plan, epoch or table disagree")]
    Pairing,
    #[error("mixed one-shot renderer was already faulted")]
    RendererFaulted,
    #[cfg(all(test, feature = "simulated-ingress"))]
    #[error("private mixed ingress journal cannot hold the registered queue")]
    IngressJournal,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("private mixed ingress command identity is out of order")]
    IngressCommandOrder,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("private mixed ingress transfer halted by its peer")]
    IngressTransferHalt,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("private mixed ingress result queue refused a prepared packet")]
    IngressResultPush,
    #[error("mixed transfer endpoint belongs to another transfer")]
    IngressForeignEndpoint,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("mixed transfer command is stamped before the last served command")]
    IngressBackwardTime,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("mixed transfer command lies beyond the forward event horizon")]
    IngressBeyondHorizon,
    #[cfg(all(test, feature = "simulated-ingress"))]
    #[error("injected private mixed fault after ingress charge")]
    InjectedAfterIngress,
    #[error("mixed one-shot compiled event at {event} missed clock {clock}")]
    MissedEvent {
        event: SampleTime,
        clock: SampleTime,
    },
    #[error(transparent)]
    Displacement(#[from] MixedEffectiveTimeError),
    #[error("mixed one-shot validated output window cannot be borrowed")]
    InternalOutputWindow,
    #[error("mixed one-shot restoration count cannot be indexed")]
    RestorationCount,
    /// A tokenized compiled edge disagrees with the audio token table.
    #[error("mixed compiled token {token:?} disagrees with the audio token table")]
    CompiledToken { token: MixedNoteToken },
    /// The audio-owned compiled minter refused a publishing note-on.
    #[error("mixed compiled note-on could not mint: {source}")]
    CompiledMint { source: IdentityError },
    /// The audio-owned compiled minter did not hold an occurrence being ended.
    #[error("mixed compiled release resolved as {resolution:?}")]
    CompiledRelease { resolution: Resolution },
    /// The boundary release and the audio-owned compiled minter disagree.
    #[error("{live} compiled occurrences remain minted after the boundary release")]
    CompiledCustody { live: HeldNoteCount },
    /// A scheduled Stop reached its boundary before the one-shot activation adopted.
    #[error("mixed compiled Stop reached its boundary before the activation adopted")]
    StopBeforeAdoption,
    /// A loop wrap found no pass identity left (ADR-0065's terminal exhaustion).
    #[error("mixed loop pass identities are exhausted")]
    LoopPassExhausted,
    /// A loop's next wrap or plan position cannot be represented.
    #[error("mixed loop timeline cannot be represented")]
    LoopTimeline,
    /// Defensive check: a pass event would publish at or after the wrap that ends it.
    #[error("mixed loop event at {event} does not precede its wrap at {wrap}")]
    LoopEventPastWrap { event: SampleTime, wrap: SampleTime },
    /// Defensive check: a repeating cursor without the loop program it reads.
    #[error("mixed loop cursor has no loop program")]
    LoopProgram,
    /// A Panic's compiled cut would overrun the release share arm proved sufficient.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error(
        "compiled Panic cut needs {groups} release charges after {spent} of share {share}",
        groups = .groups.get(),
        spent = .spent.get(),
        share = .share.get()
    )]
    CompiledPanicShare {
        spent: EventCount,
        groups: EventCount,
        share: EventCount,
    },
    /// Defensive check: the compiled cut's members could not form one release group.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("compiled Panic cut members do not form one release group")]
    CompiledPanicGroup,
    #[error(transparent)]
    Boundary(#[from] crate::render::MixedBoundaryReleaseError),
    #[error(transparent)]
    Publication(#[from] crate::publish::PublicationFault),
    #[error(transparent)]
    Render(#[from] crate::diagnostics::RenderError),
}

/// Release, restoration and cumulative suffix charges reached by the owned arbiter,
/// including charges in a quantum that later failed, plus completed renderer quanta.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
#[allow(dead_code)] // Off-thread collection is the next private slice.
pub(crate) struct MixedOneShotRenderReport {
    pub(crate) effective_anchor: StreamAnchor,
    pub(crate) retired_anchor: Option<StreamAnchor>,
    pub(crate) sequence: ActivationSequence,
    pub(crate) adopted: bool,
    pub(crate) faulted: bool,
    pub(crate) fault: Option<MixedOneShotRenderError>,
    /// Boundary releases charged, one Session operation per adoption, armed or later.
    pub(crate) releases_charged: EventCount,
    /// Compiled occurrences the armed adoption's boundary release ended. A later
    /// adoption reports its own in its receipt.
    pub(crate) released_compiled: HeldNoteCount,
    pub(crate) restoration_charged: EventCount,
    pub(crate) suffix_charged: EventCount,
    pub(crate) completed_quanta: QuantumCount,
    /// Whether the quantum that selected the compiled release completed rendering.
    pub(crate) boundary_quantum_completed: bool,
    pub(crate) transport: MixedCompiledTransport,
    /// A scheduled Stop that has not applied yet.
    pub(crate) stop_pending: Option<SampleTime>,
    /// Applied Stops whose release reached its Session charge, one per Stop that ended
    /// a running program. A Stop applied while stopped charges nothing.
    pub(crate) stops_charged: EventCount,
    /// What the single later-candidate slot holds.
    pub(crate) offer: MixedOfferSlotState,
    /// Compiled occurrences every applied Panic ended at its execution sample.
    pub(crate) compiled_panic_ended: HeldNoteCount,
    /// Release-charged groups those compiled cuts published.
    pub(crate) compiled_panic_groups: EventCount,
    /// Compiled tokens a Panic ended whose own `Off` has not published yet.
    pub(crate) compiled_panic_tombstones: HeldNoteCount,
    /// Compiled `Off` edges a Panic tombstone consumed instead of publishing, including
    /// the `Off` a loop wrap owes a token a Panic ended before the wrap.
    pub(crate) compiled_released_after_panic: EventCount,
    /// Compiled expression and bend edges a Panic tombstone consumed.
    pub(crate) compiled_controls_after_panic: EventCount,
    /// The nominal stamp of a Panic audio holds unserved until its execution quantum
    /// opens. Nothing of it has applied.
    pub(crate) panic_parked: Option<SampleTime>,
    /// The loop of the program in force, if any; it wraps only while running.
    pub(crate) loop_interval: Option<LoopInterval>,
    /// The pass that loop is in.
    pub(crate) loop_pass: Option<LoopPassId>,
    /// Applied wraps, cumulative over every loop this owner played.
    pub(crate) loop_wraps: EventCount,
    /// Open compiled occurrences those wraps ended with Session-charged `Off`s.
    pub(crate) loop_wrap_releases: HeldNoteCount,
}

/// Where the private owner's compiled program stands, in force. A scheduled Stop is
/// separate state until its boundary applies it. Live notes, rows and release tails
/// are outside this state; no transport change ends them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MixedCompiledTransport {
    Running,
    /// Stop applied at `at`, ending `released` sounding compiled occurrences. No
    /// compiled note edge publishes until a Start adopts; a seek while stopped
    /// publishes only its scoped restoration.
    ///
    /// Every quantum renders held at `position`: the plan position the Stop found at
    /// `at`, or the latest stopped seek's destination. It is `None` only where the
    /// anchor did not reach `at`, so the running quantum there had no position either.
    Stopped {
        at: SampleTime,
        released: HeldNoteCount,
        position: Option<PlanPosition>,
    },
}

/// Why the private owner refused to schedule a compiled-scoped Stop. A refusal
/// changes no state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(dead_code)] // Only the private mixed callback tests schedule a Stop so far.
pub(crate) enum MixedStopRefused {
    #[error("mixed one-shot audio owner is terminally faulted")]
    Faulted,
    #[error("compiled Stop at {at} is not a quantum boundary")]
    NotBoundary { at: SampleTime },
    #[error("compiled Stop at {at} precedes the first unrendered boundary {clock}")]
    Past { at: SampleTime, clock: SampleTime },
    /// Stop at or before the armed activation's boundary. The armed transition has no
    /// slot to retire into, so a Stop cannot cancel it.
    #[error("compiled Stop at {at} does not follow the pending activation at {effective}")]
    ActivationPending {
        at: SampleTime,
        effective: SampleTime,
    },
    #[error("a compiled Stop at {pending} is already scheduled")]
    Pending { pending: SampleTime },
}

/// How a compiled-scoped Stop request was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Only the private mixed callback tests schedule a Stop so far.
pub(crate) enum MixedStopAccepted {
    /// Applies at its boundary: it cancels a candidate pending at or after that
    /// boundary, then ends a running program's compiled custody.
    Scheduled,
    /// Stop while stopped, with no candidate to cancel and no Start before it, is
    /// acknowledged and changes nothing.
    AlreadyStopped,
}

#[cfg(test)]
#[derive(Debug, Error)]
#[allow(dead_code)] // The test-only live seam is exercised by the callback tests.
pub(crate) enum MixedOneShotTestLiveError {
    #[error("test live onset must be admitted before rendering, before the boundary")]
    Timing,
    #[error("test live onset exceeds the profile live share")]
    Share,
    #[error(transparent)]
    Identity(#[from] crate::identity::IdentityError),
}

/// Why a private mixed schedule cannot be read at an effective render boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MixedEffectiveTimeError {
    /// An activation cannot take effect before its requested time.
    #[error("effective time {effective} precedes requested time {requested}")]
    BeforeRequested {
        requested: SampleTime,
        effective: SampleTime,
    },
    /// The audio owner adopts only at a complete quantum boundary.
    #[error("effective time {effective} is not a quantum boundary")]
    NotQuantumBoundary { effective: SampleTime },
    /// The signed difference cannot represent this effective boundary.
    #[error("displacement from {requested} to {effective} cannot be represented")]
    DisplacementUnrepresentable {
        requested: SampleTime,
        effective: SampleTime,
    },
    /// A placed event would leave the representable engine timeline. Whole-candidate
    /// preflight names the latest event; a checked individual read names that read.
    #[error("event {event_index} at {time} cannot be displaced by {shift}")]
    EventTimeUnrepresentable {
        event_index: usize,
        time: SampleTime,
        shift: FrameCount,
    },
}

/// Checked timing for one possible effective boundary of a private mixed candidate.
///
/// The event list remains stamped at the requested time. A later scheduler can use this
/// single shift at every read, including restoration, rather than rewriting the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct MixedEffectiveTiming {
    effective: SampleTime,
    shift: FrameCount,
}

impl MixedEffectiveTiming {
    /// The complete quantum boundary at which the candidate would take effect.
    pub const fn effective(self) -> SampleTime {
        self.effective
    }

    /// One displacement for both restoration and suffix events.
    pub const fn shift(self) -> FrameCount {
        self.shift
    }
}

/// Read one checked mixed candidate at an effective boundary without rewriting its stamps.
/// Each read applies the same displacement to restoration and compiled suffix events.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub(crate) struct MixedEffectiveEvents<'a> {
    events: &'a [MixedCompiledEvent],
    shift: FrameCount,
}

impl MixedEffectiveEvents<'_> {
    #[allow(dead_code)] // List diagnostics are exercised only by private tests.
    pub(crate) const fn len(&self) -> usize {
        self.events.len()
    }

    #[allow(dead_code)] // Full-list inspection is exercised only by private tests.
    pub(crate) fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<MixedCompiledEvent, MixedEffectiveTimeError>> + '_
    {
        self.events
            .iter()
            .copied()
            .enumerate()
            .map(|(index, event)| self.shifted(index, event))
    }
}

/// A compiled note's pairing key inside one tokenized list.
///
/// Bounded by the compiled span rather than by the note count: preparation frees a token
/// when it places the paired release, and reuses the lowest free one. A token is never an
/// index or a generation. The audio half chooses the occurrence by ADR-0058 clause 2's
/// lowest-free rule when the note-on publishes, and its release, expression and bend name
/// the same token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) struct MixedNoteToken(u16);

/// What one tokenized compiled event does. It names no [`NoteIdentity`]: off-thread
/// preparation holds no generation state, so it cannot know one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MixedCompiledPayload {
    /// Restore only the compiled instances of one admitted parameter group.
    Restore(ScopedParameterRestore),
    /// Mint and play one compiled occurrence, keyed by its token.
    On {
        token: MixedNoteToken,
        slot: NoteSlot,
        key: KeyIdentity,
        velocity: NoteVelocity,
    },
    /// End the occurrence the token names.
    Off(MixedNoteToken),
    /// A typed per-note update for the occurrence the token names.
    Expression(MixedNoteToken, crate::controller::NoteExpression),
    /// A pitch offset for the occurrence the token names.
    Bend(MixedNoteToken, Cents),
}

/// One placed compiled event of a mixed owner, before the audio half resolves its token.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub(crate) struct MixedCompiledEvent {
    envelope: EventEnvelope,
    payload: MixedCompiledPayload,
}

/// A tokenized compiled list and the tokens it leaves open at its end.
struct MixedTokenized {
    events: Vec<MixedCompiledEvent>,
    open: Vec<MixedNoteToken>,
}

/// Why a compiled list could not be paired into tokens off-thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MixedTokenizeError {
    Schedule(SchedulePrepareError),
    UnexpectedWriter { event_index: usize },
}

/// Pair one no-stealing compiled list by token, minting nothing.
///
/// A scratch occupancy of the compiled span stands in for the minter: a note-on takes the
/// lowest free token and its paired release frees it, so a list that would over-emit the
/// span is refused here, at the event where stamping refused it. Pass one refuses foreign,
/// unpaired and producer-less edges before any token is taken, as stamping does.
fn tokenize_compiled(
    plan: &CompiledPlan,
    producer: ProducerId,
    span: Range,
    epoch: StreamEpoch,
    events: &[CompiledEvent],
) -> Result<MixedTokenized, MixedTokenizeError> {
    use MixedTokenizeError::{Schedule, UnexpectedWriter};

    let expected = plan.id();
    let capacity = span.len;
    let compiled_producer = plan.compiled_note_producer();
    if let Some(planned) = compiled_producer {
        let planned_len = plan
            .note_producer_ranges()
            .get(usize::from(planned.as_u16()))
            .map_or(0, |range| range.get());
        if planned != producer || planned_len != capacity {
            return Err(Schedule(SchedulePrepareError::CompiledRangeMismatch {
                producer: planned,
                owned: producer,
                planned: HeldNoteCount::measured(planned_len),
                actual: HeldNoteCount::measured(capacity),
            }));
        }
    }
    let foreign = |event_index, actual: PlanId| {
        Schedule(SchedulePrepareError::ForeignPlan {
            event_index,
            expected,
            actual,
        })
    };

    let mut sounding: OpenNotes<()> = OpenNotes::new(capacity, crate::ir::StealingPolicy::None);
    for (event_index, event) in events.iter().copied().enumerate() {
        let now = event.time().as_u64();
        match event.payload() {
            CompiledPayload::Controller(_) | CompiledPayload::SetParameter { .. } => {
                return Err(UnexpectedWriter { event_index });
            }
            CompiledPayload::NoteOn { slot, key, .. } => {
                if slot.plan() != expected {
                    return Err(foreign(event_index, slot.plan()));
                }
                if compiled_producer.is_none() {
                    return Err(Schedule(SchedulePrepareError::NoCompiledNoteProducer {
                        event_index,
                    }));
                }
                // Over-emission is pass two's to report; pairing continues as stamping's does.
                if sounding.open(now, slot, key, ()) == Opened::Refused {
                    sounding.record(now, slot, key, ());
                }
            }
            CompiledPayload::NoteOff { slot, key } => {
                if slot.plan() != expected {
                    return Err(foreign(event_index, slot.plan()));
                }
                if sounding.close(now, slot, key) == Closed::Unmatched {
                    return Err(Schedule(SchedulePrepareError::UnmatchedRelease {
                        event_index,
                    }));
                }
            }
            CompiledPayload::Expression { slot, key, .. }
            | CompiledPayload::Bend { slot, key, .. } => {
                if slot.plan() != expected {
                    return Err(foreign(event_index, slot.plan()));
                }
                if sounding.find(now, slot, key).is_none() {
                    return Err(Schedule(SchedulePrepareError::UnmatchedExpression {
                        event_index,
                    }));
                }
            }
        }
    }

    let over_emitted = |event_index| {
        Schedule(SchedulePrepareError::Identity {
            event_index,
            source: IdentityError::ProducerOverEmitted {
                producer,
                admitted: HeldNoteCount::measured(capacity),
            },
        })
    };
    let mut open: OpenNotes<MixedNoteToken> =
        OpenNotes::new(capacity, crate::ir::StealingPolicy::None);
    let mut occupied = vec![false; span.indices().len()];
    let mut tokenized = Vec::with_capacity(events.len());
    for (event_index, event) in events.iter().copied().enumerate() {
        let now = event.time().as_u64();
        let unmatched = Schedule(SchedulePrepareError::UnmatchedExpression { event_index });
        let payload = match event.payload() {
            CompiledPayload::Controller(_) | CompiledPayload::SetParameter { .. } => {
                return Err(UnexpectedWriter { event_index });
            }
            CompiledPayload::NoteOn {
                slot,
                key,
                velocity,
            } => {
                let free = occupied.iter().position(|held| !held);
                let Some((cell, token)) =
                    free.and_then(|free| Some((free, MixedNoteToken(u16::try_from(free).ok()?))))
                else {
                    return Err(over_emitted(event_index));
                };
                if open.open(now, slot, key, token) != Opened::Admitted {
                    return Err(over_emitted(event_index));
                }
                if let Some(held) = occupied.get_mut(cell) {
                    *held = true;
                }
                MixedCompiledPayload::On {
                    token,
                    slot,
                    key,
                    velocity,
                }
            }
            CompiledPayload::NoteOff { slot, key } => {
                let Closed::Paired(note) = open.close(now, slot, key) else {
                    return Err(Schedule(SchedulePrepareError::UnmatchedRelease {
                        event_index,
                    }));
                };
                if let Some(held) = occupied.get_mut(note.tag.as_usize()) {
                    *held = false;
                }
                MixedCompiledPayload::Off(note.tag)
            }
            CompiledPayload::Expression {
                slot,
                key,
                expression,
            } => match open.find(now, slot, key) {
                Some(note) => MixedCompiledPayload::Expression(note.tag, expression),
                None => return Err(unmatched),
            },
            CompiledPayload::Bend { slot, key, cents } => match open.find(now, slot, key) {
                Some(note) => MixedCompiledPayload::Bend(note.tag, cents),
                None => return Err(unmatched),
            },
        };
        tokenized.push(MixedCompiledEvent::new(
            EventEnvelope::new(epoch, event.time(), TimeSource::Compiled),
            payload,
        ));
    }
    Ok(MixedTokenized {
        events: tokenized,
        open: open.entries().into_iter().map(|note| note.tag).collect(),
    })
}

/// One private, off-thread compiled prefix and scoped restoration batch.
///
/// It is bound to one prepared mixed owner and destination. Its event and note books do
/// not escape through the public API. The suffix classifier consumes it; later
/// placement and activation still must prove boundary release, capacity and
/// effective-time displacement before any offer.
#[must_use]
pub struct MixedHistoryCandidate {
    plan: PlanId,
    epoch: StreamEpoch,
    table: TableId,
    requested: SampleTime,
    position: PlanPosition,
    prefix_end: usize,
    // The suffix classifier closes crossing notes in this book. The separate snapshot
    // below remains the authority for notes open at the destination.
    #[allow(dead_code)]
    book: OpenNotes<()>,
    #[allow(dead_code)]
    open_at_destination: Vec<OpenNote<()>>,
    open_count: HeldNoteCount,
    #[allow(dead_code)]
    restoration: Vec<TimedEvent>,
    restoration_count: EventCount,
    off_thread: PhantomData<Rc<()>>,
}

/// A private source-index selection and counted omissions over one bound mixed suffix.
///
/// It does not place, stamp, release, render or offer events. Its retained history
/// book reflects crossing releases consumed during classification; its separate
/// destination-open snapshot retains the original boundary set. The book,
/// snapshot and source indices remain private to a later off-thread builder.
#[must_use]
pub struct MixedSuffixCandidate {
    #[allow(dead_code)]
    history: MixedHistoryCandidate,
    #[allow(dead_code)]
    included: Vec<usize>,
    included_count: EventCount,
    omitted_releases: EventCount,
    omitted_expressions: EventCount,
}

/// One private requested-time mixed schedule: the first activation rehearsal's, or a
/// later Start or seek prepared by the armed control.
///
/// Its ordered events contain scoped restoration before any equal-time suffix edge.
/// The restoration batch moves out of the retained suffix history into that list,
/// leaving the history's restoration count at zero. Its note edges carry pairing
/// tokens, not identities, so withdrawing or refusing it changes no minter. Only
/// the anchor and activation sequence pass to control at collection; the combined
/// host still has no offer path.
#[must_use]
pub struct MixedStampedCandidate {
    #[allow(dead_code)]
    suffix: MixedSuffixCandidate,
    anchor: StreamAnchor,
    supersedes: ActivationSequence,
    events: Vec<MixedCompiledEvent>,
    event_count: EventCount,
    restoration_count: EventCount,
    #[allow(dead_code)]
    outstanding: Vec<MixedNoteToken>,
    outstanding_count: HeldNoteCount,
}

/// Sendable, boxed custody for one private audio-side mixed transition.
///
/// Source-selection history stays off-thread. Its events carry tokens; the audio
/// half's own minter gives them identities as they publish. Collection drops the
/// retired event list off-thread; the box remains with audio until final teardown.
#[must_use]
#[allow(dead_code)] // Private collection has no production host caller.
pub(crate) struct MixedAudioCandidate {
    plan: PlanId,
    epoch: StreamEpoch,
    table: TableId,
    anchor: StreamAnchor,
    retired_anchor: Option<StreamAnchor>,
    retired_next: Option<usize>,
    supersedes: ActivationSequence,
    sequence: ActivationSequence,
    omitted_releases: EventCount,
    omitted_expressions: EventCount,
    events: Vec<MixedCompiledEvent>,
    /// The candidate's admitted list size, retained as historical metadata
    /// after the list moves to audio at the boundary.
    event_count: EventCount,
    restoration_count: EventCount,
    /// Tokens the candidate's list leaves open at its end, as admitted metadata.
    outstanding_count: HeldNoteCount,
}

impl std::fmt::Debug for MixedAudioCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MixedAudioCandidate")
            .field("plan", &self.plan)
            .field("epoch", &self.epoch)
            .field("table", &self.table)
            .field("anchor", &self.anchor)
            .field("supersedes", &self.supersedes)
            .field("sequence", &self.sequence)
            .field("event_count", &self.event_count)
            .field("restoration_count", &self.restoration_count)
            .field("outstanding_count", &self.outstanding_count)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MixedStampedCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MixedStampedCandidate")
            .field("anchor", &self.anchor)
            .field("supersedes", &self.supersedes)
            .field("event_count", &self.event_count)
            .field("restoration_count", &self.restoration_count)
            .field("outstanding_count", &self.outstanding_count)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MixedSuffixCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MixedSuffixCandidate")
            .field("included_count", &self.included_count)
            .field("omitted_releases", &self.omitted_releases)
            .field("omitted_expressions", &self.omitted_expressions)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MixedHistoryCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MixedHistoryCandidate")
            .field("plan", &self.plan)
            .field("epoch", &self.epoch)
            .field("table", &self.table)
            .field("requested", &self.requested)
            .field("position", &self.position)
            .field("prefix_end", &self.prefix_end)
            .field("open_count", &self.open_count)
            .field("restoration_count", &self.restoration_count)
            .finish_non_exhaustive()
    }
}

/// An off-thread joined mixed stream, before its one bound initial schedule is sealed.
/// It cannot be transferred to the audio thread or split through its public API.
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedStream;
/// fn require_clone<T: Clone>() {}
/// require_clone::<MixedJoinedStream>();
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedStream;
/// fn split(owner: MixedJoinedStream) { let _ = owner.into_parts(); }
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedStream;
/// fn mutate(owner: &mut MixedJoinedStream) { let _ = owner.control_mut(); }
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedStream;
/// fn mutate(owner: &mut MixedJoinedStream) { let _ = owner.audio_mut(); }
/// ```
#[derive(Debug)]
#[must_use]
pub struct MixedJoinedStream {
    control: MixedStreamControl,
    audio: MixedStreamAudio,
    off_thread: PhantomData<Rc<()>>,
}

/// The sealed first schedule and both owners, still off-thread and not renderable.
/// No tokenized event or mutable half escapes this value, and sealing minted nothing.
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedPrepared;
/// fn require_clone<T: Clone>() {}
/// require_clone::<MixedJoinedPrepared>();
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedPrepared;
/// fn events(prepared: &MixedJoinedPrepared) { let _ = prepared.events(); }
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedPrepared;
/// fn require_send<T: Send>() {}
/// require_send::<MixedJoinedPrepared>();
/// ```
#[derive(Debug)]
#[must_use]
pub struct MixedJoinedPrepared {
    owner: MixedJoinedStream,
    events: Vec<MixedCompiledEvent>,
    outstanding: Vec<MixedNoteToken>,
}

/// An off-thread arm refusal retains both inputs for correction or disposal.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // The private arm has no production host caller.
pub(crate) struct MixedOneShotArmRefusal {
    reason: MixedOneShotArmError,
    owner: MixedJoinedPrepared,
    candidate: MixedStampedCandidate,
}

/// Retained off-thread control while one private transition runs. It holds no note
/// identity or identity-generation state; both minters stay with the audio half. Its
/// test-only occurrence ledger names live onsets by merger-minted handles, whose cell
/// generations say nothing about the identity audio mints for them.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // The one-shot retirement path is the next slice.
pub(crate) struct MixedOneShotControl {
    control: MixedStreamControl,
    /// The sequence the armed transition puts in force at its boundary, until a later
    /// adoption's receipt is collected; every later candidate must supersede it. For the
    /// armed transition this does not wait for collection: under S1 collection promotes
    /// no minter state, and one boundary takes one transport change, so a later
    /// candidate can only follow the armed adoption. A later candidate advances it only
    /// on a receipt confirming adoption. Audio's own in-force sequence stays the
    /// authority at any offer.
    in_force: ActivationSequence,
    /// The quantum boundary of the transition `in_force` names: fixed at arm, then
    /// moved by each confirmed later adoption.
    in_force_effective: SampleTime,
    /// The profile the audio arbiter was prepared from. Every later offer is admitted
    /// against it, so audio never runs a candidate its arbiter was not proven to fit.
    profile: HostProfile,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_owner: MixedIngressOwner,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_commands: OpaqueQueue<HeapProd<MixedIngressCommand>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_results: OpaqueQueue<HeapCons<MixedIngressResult>>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_journal: VecDeque<MixedIngressJournalEntry>,
    /// A minted command returned by a failed physical push. The terminal owner
    /// retains it until joined off-thread teardown.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_command_pending: Option<MixedIngressCommand>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    terminal_identity_fault: Option<MixedIngressIdentityFault>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    invalid_ingress_result: Option<MixedIngressResult>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_halt: Arc<AtomicBool>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_capacity: MixedIngressQueueCount,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_outstanding: MixedIngressQueueCount,
    /// Ring places reserved for the releases of open handles.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    release_reserved: MixedIngressQueueCount,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    last_ingress_id: Option<MixedIngressCommandId>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    occurrence_ledger: MixedOccurrenceLedger,
    /// Effective pedal state per scope and the ring places reserved for lifts.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pedal_ledger: MixedPedalLedger,
    /// Scopes entitled to audio's bounded bend table. Results track the last
    /// accepted value so a refused latest bend releases a never-bent scope.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    bend_scopes: Vec<Option<MixedBendScopeCell>>,
    /// Credits one bend reserves: the live hold entitlement `H`, which bounds the
    /// sounding and sustained members audio can find at service.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    bend_credits_per_bend: MixedIngressQueueCount,
    /// Bend credits held by the merger: the budget `B` less those carried by unserved
    /// commands, held by undrained entries, or returned by audio and not yet taken.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    bend_credits_free: MixedIngressQueueCount,
    /// Audio's lock-free return of unused and drained bend credits.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    bend_credit_return: MixedBendCreditReturn,
    /// The Panic holding the protected ring place, until its result is collected. That
    /// place lies outside `ingress_capacity`, so ordinary traffic can never take it.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    panic_inflight: Option<MixedIngressCommandId>,
    #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
    panic_pending_at: Option<SampleTime>,
    #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
    last_panic_applied_at: Option<SampleTime>,
    /// The last applied Panic's actual sample. A collected result frees the merger
    /// cell, but cannot grant another compiled cut in the same quantum.
    #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
    last_panic_executed_at: Option<SampleTime>,
    /// Audio's most recently opened command-service quantum. A collected Panic
    /// cannot reopen its protected place until this advances.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    panic_current_service_at: Arc<AtomicU64>,
    off_thread: PhantomData<Rc<()>>,
}

#[cfg(all(test, feature = "simulated-ingress"))]
struct OpaqueQueue<T>(T);

#[cfg(all(test, feature = "simulated-ingress"))]
impl<T> std::fmt::Debug for OpaqueQueue<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("bounded ingress queue")
    }
}

/// How one control call reaches its command producer and result consumer: the
/// private rehearsal's own rings, or a host's lent transfer endpoint. The control
/// half is passed in so the embedded rings can stay fields of the half itself.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
trait MixedControlQueues {
    fn push_command(
        &mut self,
        control: &mut MixedOneShotControl,
        command: MixedIngressCommand,
    ) -> Result<(), MixedIngressCommand>;

    fn pop_result(&mut self, control: &mut MixedOneShotControl) -> Option<MixedIngressResult>;
}

/// The private rehearsal's rings, owned by the two halves since arm.
#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Debug)]
struct EmbeddedQueues;

#[cfg(all(test, feature = "simulated-ingress"))]
impl MixedControlQueues for EmbeddedQueues {
    fn push_command(
        &mut self,
        control: &mut MixedOneShotControl,
        command: MixedIngressCommand,
    ) -> Result<(), MixedIngressCommand> {
        control.ingress_commands.0.try_push(command)
    }

    fn pop_result(&mut self, control: &mut MixedOneShotControl) -> Option<MixedIngressResult> {
        control.ingress_results.0.try_pop()
    }
}

#[cfg(all(test, feature = "simulated-ingress"))]
impl MixedAudioQueues for EmbeddedQueues {
    const TRANSFER_RULES: bool = false;

    fn bound(&self, _audio: &MixedOneShotAudio) -> bool {
        true
    }

    fn pop_command(&mut self, audio: &mut MixedOneShotAudio) -> Option<MixedIngressCommand> {
        audio.ingress_commands.0.try_pop()
    }

    fn push_result(
        &mut self,
        audio: &mut MixedOneShotAudio,
        result: MixedIngressResult,
    ) -> Option<MixedIngressResult> {
        audio.ingress_results.0.try_push(result).err()
    }
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
struct MixedIngressCommandId(u64);

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MixedIngressIdentityFault {
    CommandId,
    OccurrenceCells,
}

/// The stream and authoritative note table that own one private transfer.
/// Geometry may change while that source/handle lifetime continues; a PlanId
/// belongs to prospective destinations, never to this stable ownership identity.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
struct MixedIngressOwner {
    epoch: StreamEpoch,
    table: TableId,
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
struct MixedIngressOriginId(u64);

/// The one scope tests use where sustain scoping is not what they examine.
#[cfg(all(test, feature = "simulated-ingress"))]
const TEST_SCOPE: MixedLiveScope = MixedLiveScope::new(
    occurrence::MixedLiveSource::new(1),
    synth_core::MidiChannel::CH1,
);

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[must_use]
struct MixedIngressQueueCount(usize);

/// Prepared bend cells modeled on the private bridge's two MIDI1 sources.
/// The owner does not itself bind source IDs; ingress depth remains a floor
/// so direct private callers keep their existing table capacity.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
struct MixedBendScopeCapacity(usize);

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
impl MixedBendScopeCapacity {
    const TWO_SOURCE_MIDI1_FLOOR: usize = source_set::MixedSourceSet::MAX_SCOPES;

    fn for_two_source_midi1(ingress: MixedIngressQueueCount) -> Self {
        Self(ingress.as_usize().max(Self::TWO_SOURCE_MIDI1_FLOOR))
    }

    const fn as_usize(self) -> usize {
        self.0
    }
}

/// The merger's scope-reservation table and audio's bend table, which arm sizes
/// separately so a rehearsal can prove each covers its source's channel scopes.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MixedBendTables {
    control: MixedBendScopeCapacity,
    audio: MixedBendScopeCapacity,
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
impl MixedBendTables {
    fn for_capacity(ingress: MixedIngressQueueCount) -> Self {
        let width = MixedBendScopeCapacity::for_two_source_midi1(ingress);
        Self {
            control: width,
            audio: width,
        }
    }
}

/// Why the audio half refused to serve the command queue. Each one is terminal: it
/// is published through the shared halt before audio changes any ledger or entry.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MixedIngressCommandFault {
    /// The lent endpoint belongs to another transfer; no queue was touched.
    ForeignEndpoint,
    /// The packet names another owner.
    ForeignCommand,
    /// The packet's ID is not the successor of the last served one.
    Sequence,
    /// The packet's nominal stamp precedes the last served packet's.
    BackwardTime,
    /// The packet lies beyond the profile's forward horizon; counted once.
    BeyondHorizon,
}

/// One merger reservation for a scope that is bent or has an uncollected bend.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MixedBendScopeCell {
    scope: MixedLiveScope,
    latest: MixedIngressCommandId,
    accepted_nonzero: bool,
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
impl MixedIngressQueueCount {
    const NONE: Self = Self(0);

    /// A prepared ring or table width, which must hold at least one place.
    fn capacity(value: usize) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }

    const fn as_usize(self) -> usize {
        self.0
    }
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
enum MixedIngressRequest {
    /// Stage one live onset under the handle the merger minted for it.
    Onset {
        origin: MixedIngressOriginId,
        occurrence: LiveOccurrenceId,
        scope: MixedLiveScope,
        at: SampleTime,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    },
    /// End the occurrence the handle names; it never names an identity. Under its
    /// scope's pedal it sustains the occurrence instead.
    Release {
        origin: MixedIngressOriginId,
        occurrence: LiveOccurrenceId,
        at: SampleTime,
    },
    /// An effective sustain edge for one scope. A lift names no occurrence: audio
    /// resolves the scope's sustained members from its own table.
    Pedal {
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        down: bool,
    },
    /// A host Panic in the protected ring place. It names no scope or occurrence: audio
    /// ends every live occurrence its own table holds.
    Panic {
        origin: MixedIngressOriginId,
        at: SampleTime,
    },
    /// A channel bend for one scope. It names no occurrence: audio resolves the scope's
    /// sounding and sustained members from its own table.
    Bend {
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        cents: Cents,
    },
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Debug)]
#[must_use]
pub(crate) struct MixedIngressCommand {
    owner: MixedIngressOwner,
    id: MixedIngressCommandId,
    request: MixedIngressRequest,
    /// Bend credits the merger reserved for this command before minting `id`. Audio
    /// spends at most these and returns the rest when it serves the command.
    bend_credits: MixedIngressQueueCount,
}

/// Copyable expectations for one outstanding command, without packet custody.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug)]
struct MixedIngressJournalEntry {
    owner: MixedIngressOwner,
    id: MixedIngressCommandId,
    request: MixedIngressRequest,
    bend_credits: MixedIngressQueueCount,
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MixedIngressOutcome {
    /// The onset's identity, or the refusal its handle's tombstone now carries.
    Onset(Result<NoteIdentity, IngressRefused>),
    /// The audio half's retirement of the handle's cell, or a refused release offer
    /// that leaves the occurrence sounding and its cell held.
    Release(Result<MixedRetired, IngressRefused>),
    /// The handle named no cell the command could use; no cell changed.
    Handle(occurrence::MixedHandleRefused),
    /// The key-up found its scope's pedal down: the occurrence keeps sounding and its
    /// hold stays outstanding until the scope's lift retires it.
    Sustained(LiveOccurrenceId),
    /// The scope's pedal is down on audio, or the pedal table refused it.
    PedalDown(Result<MixedPedalDown, MixedPedalRefused>),
    /// The cells the lift retired, or a refusal that changed nothing.
    Lift(MixedLiftResult),
    /// The occurrences Panic ended and its execution stamp, or a refusal that changed
    /// nothing.
    Panic(MixedPanicResult),
    /// How the bend wrote each member, or a refusal that changed nothing.
    Bend(MixedBendResult),
}

/// What the merger did with one pedal edge.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, PartialEq)]
enum MixedPedalSubmitted {
    /// The effective edge was pushed as this command.
    Command(MixedIngressCommandId, MixedIngressRequest),
    /// The edge repeated the merger's pedal state and was consumed and counted.
    Redundant,
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Debug, PartialEq)]
#[must_use]
pub(crate) struct MixedIngressResult {
    owner: MixedIngressOwner,
    id: MixedIngressCommandId,
    request: MixedIngressRequest,
    outcome: MixedIngressOutcome,
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
impl MixedIngressResult {
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn fields(
        &self,
    ) -> (
        MixedIngressOwner,
        MixedIngressCommandId,
        MixedIngressRequest,
        MixedIngressOutcome,
    ) {
        (self.owner, self.id, self.request, self.outcome)
    }

    fn kind_matches_request(&self) -> bool {
        matches!(
            (self.request, self.outcome),
            (
                MixedIngressRequest::Onset { .. },
                MixedIngressOutcome::Onset(_) | MixedIngressOutcome::Handle(_)
            ) | (
                MixedIngressRequest::Release { .. },
                MixedIngressOutcome::Release(_)
                    | MixedIngressOutcome::Sustained(_)
                    | MixedIngressOutcome::Handle(_)
            ) | (
                MixedIngressRequest::Pedal { down: true, .. },
                MixedIngressOutcome::PedalDown(_)
            ) | (
                MixedIngressRequest::Pedal { down: false, .. },
                MixedIngressOutcome::Lift(_)
            ) | (
                MixedIngressRequest::Panic { .. },
                MixedIngressOutcome::Panic(_)
            ) | (
                MixedIngressRequest::Bend { .. },
                MixedIngressOutcome::Bend(_)
            )
        )
    }
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Clone, Copy, Debug, PartialEq, Error)]
enum MixedIngressResultCollectError {
    #[error("mixed ingress transfer is halted")]
    Halted,
    #[error("mixed ingress result does not match the next submitted command")]
    Protocol {
        received: MixedIngressCommandId,
        expected: Option<MixedIngressCommandId>,
    },
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Debug, Error, PartialEq)]
enum MixedIngressSubmitError {
    #[error("mixed ingress transfer is halted")]
    Halted(MixedIngressRequest),
    #[error("mixed ingress command or result capacity is full")]
    Full(MixedIngressRequest),
    #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
    #[error("command precedes a queued Panic at {panic_at:?}")]
    BehindPanic {
        request: MixedIngressRequest,
        panic_at: SampleTime,
    },
    #[error("mixed ingress command identity is exhausted")]
    IdentityExhausted(MixedIngressRequest),
    #[error("mixed ingress command queue refused a minted packet")]
    PhysicalPush {
        id: MixedIngressCommandId,
        request: MixedIngressRequest,
    },
}

/// A physical push can fail only after minting; logical capacity is checked by
/// the ordinary submit path before it calls this layer.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Debug)]
enum MixedIngressPushError {
    Halted(MixedIngressRequest),
    IdentityExhausted(MixedIngressRequest),
    PhysicalPush {
        id: MixedIngressCommandId,
        request: MixedIngressRequest,
    },
}

#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
impl MixedOneShotControl {
    fn command_id_exhausted(&self) -> bool {
        self.last_ingress_id == Some(MixedIngressCommandId(u64::MAX))
    }

    fn terminal_identity_refusal(&mut self, cause: MixedIngressIdentityFault) {
        self.terminal_identity_fault = Some(cause);
        self.ingress_halt.store(true, Ordering::Release);
    }

    /// Ring places neither carrying a command or result nor reserved for a release or
    /// a lift.
    fn ingress_room(&self) -> usize {
        self.ingress_capacity
            .as_usize()
            .saturating_sub(self.ingress_outstanding.as_usize())
            .saturating_sub(self.release_reserved.as_usize())
            .saturating_sub(self.pedal_ledger.reserved())
    }

    /// Push one command carrying no bend credit. Callers other than the typed submits
    /// below forge requests.
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn submit_ingress(
        &mut self,
        request: MixedIngressRequest,
    ) -> Result<MixedIngressCommandId, MixedIngressSubmitError> {
        self.submit_ingress_with(&mut EmbeddedQueues, request, MixedIngressQueueCount::NONE)
    }

    /// Push one command carrying `bend_credits` the caller has already reserved.
    fn submit_ingress_with<Q: MixedControlQueues>(
        &mut self,
        queues: &mut Q,
        request: MixedIngressRequest,
        bend_credits: MixedIngressQueueCount,
    ) -> Result<MixedIngressCommandId, MixedIngressSubmitError> {
        if self.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedIngressSubmitError::Halted(request));
        }
        #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
        if let Some(panic_at) = self.panic_pending_at.or(self.last_panic_applied_at)
            && request.nominal() < panic_at
        {
            return Err(MixedIngressSubmitError::BehindPanic { request, panic_at });
        }
        if self.command_id_exhausted() {
            self.terminal_identity_refusal(MixedIngressIdentityFault::CommandId);
            return Err(MixedIngressSubmitError::IdentityExhausted(request));
        }
        if self.ingress_room() == 0 {
            return Err(MixedIngressSubmitError::Full(request));
        }
        match self.push_ingress(queues, request, bend_credits) {
            Ok(id) => {
                self.ingress_outstanding.0 += 1;
                Ok(id)
            }
            Err(MixedIngressPushError::PhysicalPush { id, request }) => {
                self.ingress_outstanding.0 += 1;
                Err(MixedIngressSubmitError::PhysicalPush { id, request })
            }
            Err(MixedIngressPushError::Halted(request)) => {
                Err(MixedIngressSubmitError::Halted(request))
            }
            Err(MixedIngressPushError::IdentityExhausted(request)) => {
                Err(MixedIngressSubmitError::IdentityExhausted(request))
            }
        }
    }

    /// Mint the next command identity and push the command. Room is the caller's check.
    fn push_ingress<Q: MixedControlQueues>(
        &mut self,
        queues: &mut Q,
        request: MixedIngressRequest,
        bend_credits: MixedIngressQueueCount,
    ) -> Result<MixedIngressCommandId, MixedIngressPushError> {
        if self.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedIngressPushError::Halted(request));
        }
        let serial = self
            .last_ingress_id
            .map_or(Some(1), |id| id.0.checked_add(1));
        let Some(serial) = serial else {
            self.terminal_identity_refusal(MixedIngressIdentityFault::CommandId);
            return Err(MixedIngressPushError::IdentityExhausted(request));
        };
        let id = MixedIngressCommandId(serial);
        let packet = MixedIngressCommand {
            owner: self.ingress_owner,
            id,
            request,
            bend_credits,
        };
        let journal = MixedIngressJournalEntry {
            owner: self.ingress_owner,
            id,
            request,
            bend_credits,
        };
        let pushed = queues.push_command(self, packet);
        self.ingress_journal.push_back(journal);
        self.last_ingress_id = Some(id);
        match pushed {
            Ok(()) => Ok(id),
            Err(command) => {
                self.ingress_command_pending = Some(command);
                self.ingress_halt.store(true, Ordering::Release);
                Err(MixedIngressPushError::PhysicalPush { id, request })
            }
        }
    }

    /// Take `needed` bend credits after collecting audio's returns, or report how many
    /// are free. Called before a command identity is minted.
    fn reserve_bend_credits(
        &mut self,
        needed: MixedIngressQueueCount,
    ) -> Result<(), MixedIngressQueueCount> {
        self.bend_credits_free.0 += self.bend_credit_return.take().as_usize();
        if self.bend_credits_free < needed {
            return Err(self.bend_credits_free);
        }
        self.bend_credits_free.0 -= needed.as_usize();
        Ok(())
    }

    /// Undo [`Self::reserve_bend_credits`] when the command could not be pushed.
    fn unreserve_bend_credits(&mut self, credits: MixedIngressQueueCount) {
        self.bend_credits_free.0 += credits.as_usize();
    }

    /// Whether the scope holds a bend-table reservation. Without one, every bend the
    /// merger submitted for the scope ended in a collected successful zero, so audio
    /// serves a later onset on it with no inherited bend.
    fn scope_may_bend(&self, scope: MixedLiveScope) -> bool {
        self.bend_scopes
            .iter()
            .any(|cell| cell.is_some_and(|cell| cell.scope == scope))
    }

    /// Submit a host Panic into the protected ring place. Both rings carry one place
    /// beyond `ingress_capacity` for it, so a ring full of ordinary commands and
    /// uncollected results never refuses it. While an earlier Panic is uncollected the
    /// place is taken and this one is refused here, before acceptance, rather than
    /// dropped later. Of ADR-0078's host-owned transfers only the two-source bridge's
    /// carries Panic, and its arm requires host queues with the protected place.
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn submit_panic(
        &mut self,
        origin: MixedIngressOriginId,
        at: SampleTime,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedPanicSubmitError> {
        self.submit_panic_via(&mut EmbeddedQueues, origin, at)
    }

    /// [`Self::submit_panic`] through the queues `queues` reaches.
    fn submit_panic_via<Q: MixedControlQueues>(
        &mut self,
        queues: &mut Q,
        origin: MixedIngressOriginId,
        at: SampleTime,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedPanicSubmitError> {
        if self.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedPanicSubmitError::Halted);
        }
        if self.command_id_exhausted() {
            self.terminal_identity_refusal(MixedIngressIdentityFault::CommandId);
            return Err(MixedPanicSubmitError::IdentityExhausted);
        }
        if let Some(pending) = self.panic_inflight {
            return Err(MixedPanicSubmitError::Pending(pending));
        }
        if let Some(previous) = self.last_panic_applied_at
            && at < previous
        {
            return Err(MixedPanicSubmitError::BehindPanic(previous));
        }
        if let Some(previous) = self.last_panic_executed_at {
            // The audio side publishes its opened quantum before serving commands.
            // Collection frees the cell but does not permit a second Panic before
            // another quantum opens, regardless of the new command's future stamp.
            let current_service =
                SampleTime::new(self.panic_current_service_at.load(Ordering::Acquire));
            if current_service.quantum_index() == previous.quantum_index() {
                return Err(MixedPanicSubmitError::SameServiceQuantum { previous, at });
            }
        }
        let request = MixedIngressRequest::Panic { origin, at };
        // Panic carries no bend credit, so an exhausted budget never refuses it.
        let command = match self.push_ingress(queues, request, MixedIngressQueueCount::NONE) {
            Ok(command) => command,
            Err(MixedIngressPushError::PhysicalPush { id, .. }) => {
                self.panic_inflight = Some(id);
                self.panic_pending_at = Some(at);
                return Err(MixedPanicSubmitError::PhysicalPush);
            }
            Err(MixedIngressPushError::Halted(_)) => return Err(MixedPanicSubmitError::Halted),
            Err(MixedIngressPushError::IdentityExhausted(_)) => {
                return Err(MixedPanicSubmitError::IdentityExhausted);
            }
        };
        self.panic_inflight = Some(command);
        self.panic_pending_at = Some(at);
        Ok((command, request))
    }

    /// Mint a handle and stage its onset, reserving one ring place for its release so
    /// the release can follow at once, before the onset's result is collected. An onset
    /// on a scope that may be bent also reserves one bend credit for its inherited
    /// entry before its command identity is minted; an unbent scope needs none.
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn submit_onset(
        &mut self,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedOnsetSubmitError> {
        self.submit_onset_via(&mut EmbeddedQueues, origin, scope, at, key, velocity)
    }

    /// [`Self::submit_onset`] through the queues `queues` reaches.
    fn submit_onset_via<Q: MixedControlQueues>(
        &mut self,
        queues: &mut Q,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedOnsetSubmitError> {
        if self.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedOnsetSubmitError::Halted);
        }
        if self.command_id_exhausted() {
            self.terminal_identity_refusal(MixedIngressIdentityFault::CommandId);
            return Err(MixedOnsetSubmitError::IdentityExhausted);
        }
        if self.ingress_room() < 2 {
            return Err(MixedOnsetSubmitError::Full);
        }
        // The ledger is as wide as the ring plus the live hold entitlement. A held cell
        // owns a ring place, or is sustained and owns an outstanding hold. If the ring
        // has two free places but no vacant cell, its generations are exhausted rather
        // than the onset being transiently blocked by ring capacity.
        let credits = if self.scope_may_bend(scope) {
            MixedIngressQueueCount(1)
        } else {
            MixedIngressQueueCount::NONE
        };
        self.reserve_bend_credits(credits)
            .map_err(|_| MixedOnsetSubmitError::BendCredit)?;
        let Some(occurrence) = self.occurrence_ledger.vacant() else {
            self.unreserve_bend_credits(credits);
            self.terminal_identity_refusal(MixedIngressIdentityFault::OccurrenceCells);
            return Err(MixedOnsetSubmitError::IdentityExhausted);
        };
        let request = MixedIngressRequest::Onset {
            origin,
            occurrence,
            scope,
            at,
            key,
            velocity,
        };
        let (command, physical_push_fault) =
            match self.submit_ingress_with(queues, request, credits) {
                Ok(command) => (command, false),
                Err(MixedIngressSubmitError::PhysicalPush { id, .. }) => (id, true),
                Err(MixedIngressSubmitError::Halted(_)) => {
                    self.unreserve_bend_credits(credits);
                    return Err(MixedOnsetSubmitError::Halted);
                }
                Err(MixedIngressSubmitError::Full(_)) => {
                    self.unreserve_bend_credits(credits);
                    return Err(MixedOnsetSubmitError::Full);
                }
                #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
                Err(MixedIngressSubmitError::BehindPanic { panic_at, .. }) => {
                    self.unreserve_bend_credits(credits);
                    return Err(MixedOnsetSubmitError::BehindPanic(panic_at));
                }
                Err(MixedIngressSubmitError::IdentityExhausted(_)) => {
                    self.unreserve_bend_credits(credits);
                    return Err(MixedOnsetSubmitError::IdentityExhausted);
                }
            };
        if !self.occurrence_ledger.open(occurrence, scope, command) {
            // Defensive: `vacant` named this free cell and nothing has run since. The
            // minted command keeps its custody for joined teardown.
            self.ingress_halt.store(true, Ordering::Release);
            return Err(MixedOnsetSubmitError::LedgerDefect(command));
        }
        self.release_reserved.0 += 1;
        if physical_push_fault {
            return Err(MixedOnsetSubmitError::PhysicalPush);
        }
        Ok((command, request))
    }

    /// Submit the release of an open handle into the place its onset reserved.
    /// A preflight refusal leaves the handle open. A physical push failure
    /// retains the command and moves the handle to releasing before halting.
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn submit_release(
        &mut self,
        origin: MixedIngressOriginId,
        occurrence: LiveOccurrenceId,
        at: SampleTime,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedReleaseSubmitError> {
        self.submit_release_via(&mut EmbeddedQueues, origin, occurrence, at)
    }

    /// [`Self::submit_release`] through the queues `queues` reaches.
    fn submit_release_via<Q: MixedControlQueues>(
        &mut self,
        queues: &mut Q,
        origin: MixedIngressOriginId,
        occurrence: LiveOccurrenceId,
        at: SampleTime,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedReleaseSubmitError> {
        if self.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedReleaseSubmitError::Halted);
        }
        if !self.occurrence_ledger.is_open(occurrence) {
            return Err(MixedReleaseSubmitError::NotOpen(occurrence));
        }
        if self.command_id_exhausted() {
            self.terminal_identity_refusal(MixedIngressIdentityFault::CommandId);
            return Err(MixedReleaseSubmitError::IdentityExhausted(occurrence));
        }
        let request = MixedIngressRequest::Release {
            origin,
            occurrence,
            at,
        };
        // Every open handle holds one reserved place; a ledger that disagrees halts
        // rather than spending a place it does not have.
        let Some(reserved) = self.release_reserved.as_usize().checked_sub(1) else {
            self.ingress_halt.store(true, Ordering::Release);
            return Err(MixedReleaseSubmitError::ReservationDefect(occurrence));
        };
        self.release_reserved.0 = reserved;
        let (command, physical_push_fault) =
            match self.submit_ingress_with(queues, request, MixedIngressQueueCount::NONE) {
                Ok(command) => (command, false),
                Err(MixedIngressSubmitError::PhysicalPush { id, .. }) => (id, true),
                Err(MixedIngressSubmitError::Halted(_)) => {
                    self.release_reserved.0 += 1;
                    return Err(MixedReleaseSubmitError::Halted);
                }
                Err(MixedIngressSubmitError::Full(_)) => {
                    self.release_reserved.0 += 1;
                    return Err(MixedReleaseSubmitError::ReservedPlaceUnavailable(
                        occurrence,
                    ));
                }
                #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
                Err(MixedIngressSubmitError::BehindPanic { panic_at, .. }) => {
                    self.release_reserved.0 += 1;
                    return Err(MixedReleaseSubmitError::BehindPanic(panic_at));
                }
                Err(MixedIngressSubmitError::IdentityExhausted(_)) => {
                    self.release_reserved.0 += 1;
                    return Err(MixedReleaseSubmitError::IdentityExhausted(occurrence));
                }
            };
        if !self.occurrence_ledger.begin_release(occurrence, command) {
            // Defensive: `is_open` held above and nothing has run since. The minted
            // command keeps its custody for joined teardown.
            self.ingress_halt.store(true, Ordering::Release);
            return Err(MixedReleaseSubmitError::LedgerDefect(command));
        }
        if physical_push_fault {
            return Err(MixedReleaseSubmitError::PhysicalPush);
        }
        Ok((command, request))
    }

    /// Submit one channel bend for `scope` into an ordinary ring place. A new
    /// non-zero scope reserves one audio bend-table cell, and every bend reserves `H`
    /// bend credits, before its command identity is minted. Bends are not coalesced: a
    /// full logical ring, scope table or credit budget refuses before minting.
    /// A failed physical push retains the minted bend and halts the stream.
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn submit_bend(
        &mut self,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        cents: Cents,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedBendSubmitError> {
        self.submit_bend_via(&mut EmbeddedQueues, origin, scope, at, cents)
    }

    /// [`Self::submit_bend`] through the queues `queues` reaches.
    fn submit_bend_via<Q: MixedControlQueues>(
        &mut self,
        queues: &mut Q,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        cents: Cents,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedBendSubmitError> {
        if self.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedBendSubmitError::Halted);
        }
        if self.command_id_exhausted() {
            self.terminal_identity_refusal(MixedIngressIdentityFault::CommandId);
            return Err(MixedBendSubmitError::IdentityExhausted);
        }
        if self.ingress_room() == 0 {
            return Err(MixedBendSubmitError::Full);
        }
        let existing = self
            .bend_scopes
            .iter()
            .position(|cell| cell.is_some_and(|cell| cell.scope == scope));
        let vacant = if cents == Cents::ZERO || existing.is_some() {
            None
        } else {
            Some(
                self.bend_scopes
                    .iter()
                    .position(Option::is_none)
                    .ok_or(MixedBendSubmitError::ScopeCapacity)?,
            )
        };
        let needed = self.bend_credits_per_bend;
        self.reserve_bend_credits(needed)
            .map_err(|available| MixedBendSubmitError::Credits { needed, available })?;
        let request = MixedIngressRequest::Bend {
            origin,
            scope,
            at,
            cents,
        };
        let (command, physical_push_fault) = match self.submit_ingress_with(queues, request, needed)
        {
            Ok(command) => (command, false),
            Err(MixedIngressSubmitError::PhysicalPush { id, .. }) => (id, true),
            Err(MixedIngressSubmitError::Halted(_)) => {
                self.unreserve_bend_credits(needed);
                return Err(MixedBendSubmitError::Halted);
            }
            Err(MixedIngressSubmitError::Full(_)) => {
                self.unreserve_bend_credits(needed);
                return Err(MixedBendSubmitError::Full);
            }
            #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
            Err(MixedIngressSubmitError::BehindPanic { panic_at, .. }) => {
                self.unreserve_bend_credits(needed);
                return Err(MixedBendSubmitError::BehindPanic(panic_at));
            }
            Err(MixedIngressSubmitError::IdentityExhausted(_)) => {
                self.unreserve_bend_credits(needed);
                return Err(MixedBendSubmitError::IdentityExhausted);
            }
        };
        if let Some(index) = existing.or(vacant)
            && let Some(cell) = self.bend_scopes.get_mut(index)
        {
            *cell = Some(MixedBendScopeCell {
                scope,
                latest: command,
                accepted_nonzero: cell.is_some_and(|cell| cell.accepted_nonzero),
            });
        }
        if physical_push_fault {
            return Err(MixedBendSubmitError::PhysicalPush);
        }
        Ok((command, request))
    }

    /// Submit one sustain edge for `scope`. Only an edge that changes the merger's
    /// pedal state is pushed. An effective pedal-down needs its own place and one more
    /// that it reserves for the scope's lift. Preflight refusals leave the pedal
    /// state unchanged. A failed physical push retains the command and its
    /// corresponding pedal state before halting. ADR-0078's single-source transfer
    /// exposes no pedal; only the two-source bridge reaches [`Self::submit_pedal_via`].
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn submit_pedal(
        &mut self,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        down: bool,
    ) -> Result<MixedPedalSubmitted, MixedPedalSubmitError> {
        self.submit_pedal_via(&mut EmbeddedQueues, origin, scope, at, down)
    }

    /// [`Self::submit_pedal`] through the queues `queues` reaches.
    fn submit_pedal_via<Q: MixedControlQueues>(
        &mut self,
        queues: &mut Q,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        down: bool,
    ) -> Result<MixedPedalSubmitted, MixedPedalSubmitError> {
        if self.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedPedalSubmitError::Halted);
        }
        if self.pedal_ledger.classify(scope, down) == MixedPedalEdge::Redundant {
            return Ok(MixedPedalSubmitted::Redundant);
        }
        if self.command_id_exhausted() {
            self.terminal_identity_refusal(MixedIngressIdentityFault::CommandId);
            return Err(MixedPedalSubmitError::IdentityExhausted);
        }
        let request = MixedIngressRequest::Pedal {
            origin,
            scope,
            at,
            down,
        };
        let none = MixedIngressQueueCount::NONE;
        if down {
            if self.ingress_room() < 2 || !self.pedal_ledger.can_press(scope) {
                return Err(MixedPedalSubmitError::Full);
            }
            let (command, physical_push_fault) =
                match self.submit_ingress_with(queues, request, none) {
                    Ok(command) => (command, false),
                    Err(MixedIngressSubmitError::PhysicalPush { id, .. }) => (id, true),
                    Err(MixedIngressSubmitError::Halted(_)) => {
                        return Err(MixedPedalSubmitError::Halted);
                    }
                    Err(MixedIngressSubmitError::Full(_)) => {
                        return Err(MixedPedalSubmitError::Full);
                    }
                    #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
                    Err(MixedIngressSubmitError::BehindPanic { panic_at, .. }) => {
                        return Err(MixedPedalSubmitError::BehindPanic(panic_at));
                    }
                    Err(MixedIngressSubmitError::IdentityExhausted(_)) => {
                        return Err(MixedPedalSubmitError::IdentityExhausted);
                    }
                };
            self.pedal_ledger.press(scope, command);
            if physical_push_fault {
                return Err(MixedPedalSubmitError::PhysicalPush);
            }
            return Ok(MixedPedalSubmitted::Command(command, request));
        }
        // The scope's reserved place becomes room for the lift itself.
        self.pedal_ledger.take_reservation();
        match self.submit_ingress_with(queues, request, none) {
            Ok(command) => {
                self.pedal_ledger.lift(scope, command);
                Ok(MixedPedalSubmitted::Command(command, request))
            }
            Err(MixedIngressSubmitError::PhysicalPush { id, .. }) => {
                self.pedal_ledger.lift(scope, id);
                Err(MixedPedalSubmitError::PhysicalPush)
            }
            Err(MixedIngressSubmitError::Halted(_)) => {
                self.pedal_ledger.return_reservation();
                Err(MixedPedalSubmitError::Halted)
            }
            Err(MixedIngressSubmitError::Full(_)) => {
                self.pedal_ledger.return_reservation();
                Err(MixedPedalSubmitError::ReservedPlaceUnavailable)
            }
            #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
            Err(MixedIngressSubmitError::BehindPanic { panic_at, .. }) => {
                self.pedal_ledger.return_reservation();
                Err(MixedPedalSubmitError::BehindPanic(panic_at))
            }
            Err(MixedIngressSubmitError::IdentityExhausted(_)) => {
                self.pedal_ledger.return_reservation();
                Err(MixedPedalSubmitError::IdentityExhausted)
            }
        }
    }

    /// Collect one result. An onset records whether its handle sounds or holds a
    /// refusal tombstone. A release result settles its handle: `Retired` frees the
    /// ledger cell for reuse; `Sustained` leaves the cell for the scope's lift and
    /// spends the release's place; a refused release returns custody and its reserved
    /// place to the merger. A lift result frees every cell it retired and settles the
    /// scope's lift reservation; a pedal-down result settles its reservation. A Panic
    /// result frees every sustained cell it retired and the protected place; the cells
    /// it tombstoned stay open for their key-ups. The latest bend result releases
    /// its scope cell when the last accepted value is zero.
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn collect_ingress_result(&mut self) -> Option<MixedIngressResult> {
        match self.collect_ingress_result_checked() {
            Ok(result) => result,
            Err(MixedIngressResultCollectError::Halted) => None,
            Err(error) => panic!(
                "private mixed result protocol fault: {error}; received={:?}; expected={:?}",
                self.invalid_ingress_result,
                self.ingress_journal.front()
            ),
        }
    }

    fn result_payload_matches_ledger(
        &self,
        result: &MixedIngressResult,
        expected: MixedIngressJournalEntry,
    ) -> bool {
        match (result.request, result.outcome) {
            (MixedIngressRequest::Onset { occurrence, .. }, MixedIngressOutcome::Onset(_)) => self
                .occurrence_ledger
                .is_pending_from(occurrence, result.id),
            (MixedIngressRequest::Onset { occurrence, .. }, MixedIngressOutcome::Handle(_)) => {
                !self
                    .occurrence_ledger
                    .is_pending_from(occurrence, result.id)
            }
            (
                MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Release(Ok(retired)),
            ) => {
                retired.occurrence == occurrence
                    && self.occurrence_ledger.is_releasing(occurrence, result.id)
                    && self
                        .occurrence_ledger
                        .release_result_matches_sound(occurrence, retired.cause)
            }
            (
                MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Release(Err(_)),
            ) => {
                self.occurrence_ledger.is_releasing(occurrence, result.id)
                    && self.occurrence_ledger.is_sounding(occurrence)
            }
            (
                MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Sustained(sustained),
            ) => {
                sustained == occurrence
                    && self.occurrence_ledger.is_releasing(occurrence, result.id)
                    && self.occurrence_ledger.is_sounding(occurrence)
            }
            (MixedIngressRequest::Release { occurrence, .. }, MixedIngressOutcome::Handle(_)) => {
                !self.occurrence_ledger.is_releasing(occurrence, result.id)
            }
            (
                MixedIngressRequest::Pedal {
                    scope, down: false, ..
                },
                MixedIngressOutcome::Lift(Ok(lifted)),
            ) => self.occurrence_ledger.can_retire_lift(scope, &lifted),
            (MixedIngressRequest::Panic { .. }, MixedIngressOutcome::Panic(Ok(panicked))) => {
                self.panic_inflight == Some(result.id)
                    && self.occurrence_ledger.can_settle_panic(&panicked)
            }
            (MixedIngressRequest::Panic { .. }, MixedIngressOutcome::Panic(Err(_))) => {
                self.panic_inflight == Some(result.id)
            }
            (MixedIngressRequest::Bend { scope, at, .. }, MixedIngressOutcome::Bend(Ok(bent))) => {
                self.occurrence_ledger
                    .can_report_bend(scope, at, expected.bend_credits, &bent)
            }
            _ => true,
        }
    }

    /// Validate identity, request and outcome kind before changing any ledger or
    /// reservation. The test-only wrapper above fails loudly for existing fixtures;
    /// a host-facing collector will return this typed fault to its caller.
    #[cfg(all(test, feature = "simulated-ingress"))]
    fn collect_ingress_result_checked(
        &mut self,
    ) -> Result<Option<MixedIngressResult>, MixedIngressResultCollectError> {
        self.collect_ingress_result_via(&mut EmbeddedQueues)
    }

    /// [`Self::collect_ingress_result_checked`] through the queues `queues` reaches.
    fn collect_ingress_result_via<Q: MixedControlQueues>(
        &mut self,
        queues: &mut Q,
    ) -> Result<Option<MixedIngressResult>, MixedIngressResultCollectError> {
        let halted = self.ingress_halt.load(Ordering::Acquire);
        if let Some(received) = self.invalid_ingress_result.as_ref() {
            return Err(MixedIngressResultCollectError::Protocol {
                received: received.id,
                expected: self.ingress_journal.front().map(|packet| packet.id),
            });
        }
        if halted {
            return Err(MixedIngressResultCollectError::Halted);
        }
        let Some(result) = queues.pop_result(self) else {
            return Ok(None);
        };
        let expected = self.ingress_journal.front().copied();
        let place_accounted = match result.request {
            MixedIngressRequest::Panic { .. } => self.panic_inflight == Some(result.id),
            _ => self.ingress_outstanding > MixedIngressQueueCount::NONE,
        };
        if expected.is_none_or(|packet| {
            packet.owner != self.ingress_owner
                || result.owner != self.ingress_owner
                || packet.id != result.id
                || packet.request != result.request
                || !result.kind_matches_request()
                || !self.result_payload_matches_ledger(&result, packet)
        }) || !place_accounted
        {
            let received = result.id;
            self.invalid_ingress_result = Some(result);
            self.ingress_halt.store(true, Ordering::Release);
            return Err(MixedIngressResultCollectError::Protocol {
                received,
                expected: expected.map(|packet| packet.id),
            });
        }
        let Some(_settled) = self.ingress_journal.pop_front() else {
            let received = result.id;
            self.invalid_ingress_result = Some(result);
            self.ingress_halt.store(true, Ordering::Release);
            return Err(MixedIngressResultCollectError::Protocol {
                received,
                expected: None,
            });
        };
        if let MixedIngressRequest::Panic { .. } = result.request {
            self.panic_inflight = None;
            #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
            {
                self.panic_pending_at = None;
            }
            if let MixedIngressOutcome::Panic(Ok(panicked)) = result.outcome {
                #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
                {
                    self.last_panic_applied_at = Some(result.request.nominal());
                    self.last_panic_executed_at = Some(panicked.executed_at());
                }
                for ended in panicked.ended() {
                    let settled = match ended {
                        occurrence::MixedPanicEnd::Retired(retired) => {
                            self.occurrence_ledger.retire_sustained(retired)
                        }
                        occurrence::MixedPanicEnd::Tombstoned(tombstone) => {
                            self.occurrence_ledger.tombstone(tombstone)
                        }
                    };
                    debug_assert!(settled, "validated Panic members remain held");
                }
            }
            return Ok(Some(result));
        }
        self.ingress_outstanding.0 -= 1;
        match result.request {
            MixedIngressRequest::Release { occurrence, .. } => {
                if let MixedIngressOutcome::Sustained(sustained) = result.outcome {
                    let settled = sustained == occurrence
                        && self.occurrence_ledger.sustain(result.id, occurrence);
                    debug_assert!(settled, "audio sustains only the released handle");
                    return Ok(Some(result));
                }
                let retired = match result.outcome {
                    MixedIngressOutcome::Release(Ok(retired)) => Some(retired),
                    MixedIngressOutcome::Release(Err(_))
                    | MixedIngressOutcome::Handle(_)
                    | MixedIngressOutcome::Onset(_)
                    | MixedIngressOutcome::Sustained(_)
                    | MixedIngressOutcome::PedalDown(_)
                    | MixedIngressOutcome::Lift(_)
                    | MixedIngressOutcome::Panic(_)
                    | MixedIngressOutcome::Bend(_) => None,
                };
                if self
                    .occurrence_ledger
                    .settle_release(result.id, occurrence, retired)
                {
                    self.release_reserved.0 += 1;
                }
            }
            MixedIngressRequest::Pedal {
                scope, down: true, ..
            } => {
                let refused = !matches!(result.outcome, MixedIngressOutcome::PedalDown(Ok(_)));
                self.pedal_ledger.settle_down(result.id, scope, refused);
            }
            MixedIngressRequest::Pedal {
                scope, down: false, ..
            } => {
                let lifted = match result.outcome {
                    MixedIngressOutcome::Lift(Ok(lifted)) => Some(lifted),
                    _ => None,
                };
                for retired in lifted.iter().flat_map(|lifted| lifted.retired()) {
                    let freed = self.occurrence_ledger.retire_sustained(retired);
                    debug_assert!(freed, "a lift retires only sustained cells");
                }
                self.pedal_ledger
                    .settle_lift(result.id, scope, lifted.is_none());
            }
            MixedIngressRequest::Bend { scope, cents, .. } => {
                if let Some(cell) = self
                    .bend_scopes
                    .iter_mut()
                    .find(|cell| cell.is_some_and(|cell| cell.scope == scope))
                    && let Some(held) = cell.as_mut()
                {
                    if matches!(result.outcome, MixedIngressOutcome::Bend(Ok(_))) {
                        held.accepted_nonzero = cents != Cents::ZERO;
                    }
                    if held.latest == result.id && !held.accepted_nonzero {
                        *cell = None;
                    }
                }
            }
            MixedIngressRequest::Onset { occurrence, .. } => {
                if let MixedIngressOutcome::Onset(outcome) = result.outcome {
                    let settled =
                        self.occurrence_ledger
                            .settle_onset(occurrence, result.id, outcome.is_ok());
                    debug_assert!(settled, "validated onset remains pending in the ledger");
                }
            }
            MixedIngressRequest::Panic { .. } => {}
        }
        Ok(Some(result))
    }
}

/// Control after a private adopted owner has rejoined: the collected anchor and
/// activation sequence, and still no identity or generation state.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // Private collection has no production host caller.
pub(crate) struct MixedResumedControl {
    control: MixedStreamControl,
    sequence: ActivationSequence,
    off_thread: PhantomData<Rc<()>>,
}

/// Old compiled list metadata, copied before its vector is dropped off-thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
#[allow(dead_code)]
pub(crate) struct MixedRetiredList {
    anchor: StreamAnchor,
    unconsumed_from: usize,
    event_count: usize,
}

/// Why a correctly paired private owner stopped without a resumable control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum MixedCollectionEnd {
    Pending,
    Faulted,
    /// The adopted capsule's plan, epoch, table, sequence or anchor disagreed, so
    /// control received neither its anchor nor its sequence.
    PromotionRefused,
    ResumedTeardown,
}

/// Final stopped-state classification. Boundary-ended and still-sounding notes
/// remain separate, as do uncharged and charged test-live reservations that
/// never entered the renderer registry. `boundary_ended` is the armed adoption's
/// record; `offer_boundary_ended` is the record of a later adoption whose offer cell
/// was still in audio's slot, so each ended note appears once, under the boundary
/// that ended it. `stop_boundary_ended` is the latest applied Stop's record, filled
/// before custody ends, so a fault inside the Stop still shows what it released; an
/// earlier Stop's notes are no longer in any custody. `offer` owns whatever the slot
/// held, including a cancelled or refused candidate's receipt.
/// `compiled_minted` lists the compiled occurrences the audio minter
/// still holds; a note-on charged in a callback that then faulted appears there but
/// not in `sounding`.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)]
pub(crate) struct MixedOneShotTeardown {
    end: MixedCollectionEnd,
    report: MixedOneShotRenderReport,
    sounding: MixedSoundingSnapshot,
    boundary_ended: Vec<crate::identity::EndedNote>,
    offer_boundary_ended: Vec<crate::identity::EndedNote>,
    stop_boundary_ended: Vec<crate::identity::EndedNote>,
    offer: MixedDrainedOffer,
    unpublished_live: Option<NoteIdentity>,
    charged_unregistered_live: Option<NoteIdentity>,
    compiled_minted: Vec<NoteIdentity>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress: MixedIngressTeardown,
}

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Debug)]
struct MixedIngressTeardown {
    queued: Vec<(TimedEvent, bool)>,
    charged_in_faulted_callback: Vec<(TimedEvent, bool)>,
    minted_live: Vec<NoteIdentity>,
    holds_outstanding: EventCount,
    counters: IngressCounters,
    /// Every handle the audio table still holds, sounding, sustained or tombstoned.
    occurrences: Vec<(LiveOccurrenceId, MixedOccurrenceState)>,
    /// Scopes whose pedal audio still holds down.
    pedals_down: Vec<MixedLiveScope>,
    released_after_refusal: EventCount,
    released_after_panic: EventCount,
    /// The command that failed the audio-side sequence check, if any.
    invalid_command: Option<MixedIngressCommand>,
    invalid_result: Option<MixedIngressResult>,
    last_served_id: Option<MixedIngressCommandId>,
    unserved_commands: Vec<MixedIngressCommand>,
    pending_command: Option<MixedIngressCommand>,
    /// A Panic audio held unserved: nothing of it applied, so it ends cancelled.
    parked_panic: Option<MixedIngressCommand>,
    uncollected_results: Vec<MixedIngressResult>,
    pending_result: Option<MixedIngressResult>,
    terminal_control: Option<MixedTerminalControlCustody>,
}

/// Control-side places and bend credits retained at a terminal mixed teardown.
#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Debug)]
struct MixedTerminalControlCustody {
    terminal_identity_fault: Option<MixedIngressIdentityFault>,
    ingress_outstanding: MixedIngressQueueCount,
    release_reserved: MixedIngressQueueCount,
    pedal_reserved: MixedIngressQueueCount,
    panic_inflight: Option<MixedIngressCommandId>,
    bend_credits_free: MixedIngressQueueCount,
    bend_credits_return_pending: MixedIngressQueueCount,
    bend_scopes: Vec<MixedBendScopeCell>,
    /// Outstanding expectations, separate from owning packets in queues or
    /// the invalid-command and pending-command slots, and from packets already
    /// served by audio.
    outstanding_commands: Vec<MixedIngressJournalEntry>,
}

#[derive(Debug)]
#[must_use]
#[allow(dead_code)]
pub(crate) enum MixedCollection {
    Resumed {
        control: MixedResumedControl,
        audio: Box<MixedOneShotAudio>,
        retired: Box<MixedRetiredList>,
        /// The offer slot's contents, owned; the resumed audio half's slot is empty.
        offer: Box<MixedDrainedOffer>,
    },
    Ended(Box<MixedOneShotTeardown>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(dead_code)]
pub(crate) enum MixedCollectionError {
    #[cfg(feature = "mixed-transfer-rehearsal")]
    #[error("an owning continuation is pending or uncollected")]
    ContinuationPending,
    #[error("mixed control and audio halves were crossed")]
    CrossedPair,
    #[error(transparent)]
    Snapshot(#[from] MixedSoundingSnapshotError),
    #[error("mixed boundary-ended storage lost its recorded prefix")]
    EndedPrefix,
    #[error("mixed audio half has not been collected")]
    UncollectedAudio,
    #[cfg(all(test, feature = "simulated-ingress"))]
    #[error("private mixed ingress journal lost the registered queue")]
    IngressJournal,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    #[error("private mixed ingress command or result remains uncollected")]
    IngressResultPending,
}

/// Every refusal returns both stopped halves before consuming either one.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)]
pub(crate) struct MixedCollectionRefusal<C> {
    reason: MixedCollectionError,
    control: C,
    audio: MixedOneShotAudio,
}

/// One bound audio half, its old list and a single fixed-boundary capsule.
/// Only a private closed-schedule rehearsal can render; no host offer path exists.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // No production host may call the private scheduler.
pub(crate) struct MixedOneShotAudio {
    #[cfg(feature = "mixed-transfer-rehearsal")]
    continuation: Option<Box<transfer::continuation::MixedContinuationPacket>>,
    #[cfg(feature = "mixed-transfer-rehearsal")]
    activation_custody: Option<Arc<std::sync::atomic::AtomicUsize>>,
    audio: MixedStreamAudio,
    arbiter: PublicationArbiter,
    events: Vec<MixedCompiledEvent>,
    next: usize,
    capsule: Box<MixedAudioCandidate>,
    timing: MixedEffectiveTiming,
    effective_anchor: StreamAnchor,
    late_at_arm: bool,
    in_force: ActivationSequence,
    adopted: bool,
    fault: Option<MixedOneShotRenderError>,
    releases_charged: EventCount,
    released_compiled: HeldNoteCount,
    restoration_charged: EventCount,
    suffix_charged: EventCount,
    completed_quanta: QuantumCount,
    adoption_after_quanta: Option<QuantumCount>,
    render_started: bool,
    transport: MixedCompiledTransport,
    /// At most one scheduled Stop, applied when its quantum boundary opens.
    stop_pending: Option<SampleTime>,
    /// Retirement Stop has no playhead consumer and skips its fallible projection.
    retiring_stop: bool,
    /// A Stop's own ended-note storage, sized like `compiled_ended`, so it never
    /// overwrites the adoption's boundary-ended record that teardown reads. Each
    /// applied Stop reuses it; `stop_released` names the latest Stop's prefix.
    stop_ended: Vec<Option<crate::identity::EndedNote>>,
    /// Recorded from the renderer's release before custody ends.
    stop_released: HeldNoteCount,
    /// An applied Stop's release still owes its one Session operation.
    stop_due: bool,
    stops_charged: EventCount,
    /// Scoped restoration events at the front of the list in force; zero before the
    /// armed adoption, whose initial list has none.
    in_force_restoration: EventCount,
    /// An adoption's boundary release still owes its one Session operation.
    release_due: bool,
    /// The single later-candidate slot.
    offer_slot: MixedOfferSlot,
    /// The loop of the program in force; only a later adoption installs or retires one.
    loop_program: Option<Box<MixedLoopProgram>>,
    /// Set with `loop_program` at adoption and moved by every wrap.
    loop_cursor: Option<MixedLoopCursor>,
    loop_wraps: EventCount,
    loop_wrap_releases: HeldNoteCount,
    compiled_panic_ended: HeldNoteCount,
    compiled_panic_groups: EventCount,
    compiled_released_after_panic: EventCount,
    compiled_controls_after_panic: EventCount,
    /// The sample at which a Panic served at this boundary ends the compiled producer's
    /// open occurrences. The quantum's compiled loop takes it; at most one per quantum.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    panic_cut: Option<SampleTime>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    panic_current_service_at: Arc<AtomicU64>,
    /// A popped Panic whose execution stamp lies in a later quantum, or behind a cut
    /// already owed in this one. Service pops nothing more until it is served, so
    /// results keep command order; teardown returns it unapplied.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    parked_panic: Option<MixedIngressCommand>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    test_ingress: PerformanceIngress,
    #[cfg(feature = "mixed-transfer-rehearsal")]
    ingress_queue_peak: usize,
    #[cfg(feature = "mixed-transfer-rehearsal")]
    ingress_holds_peak: EventCount,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_owner: MixedIngressOwner,
    /// What each live handle names; the merger's ledger has the same width.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    occurrences: MixedOccurrenceTable,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_commands: OpaqueQueue<HeapCons<MixedIngressCommand>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_results: OpaqueQueue<HeapProd<MixedIngressResult>>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_result_pending: Option<MixedIngressResult>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_halt: Arc<AtomicBool>,
    /// The last served command ID; a gap, duplicate or wrap faults before the
    /// offending command changes audio state. Its packet remains owned for teardown.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    last_served_ingress_id: Option<MixedIngressCommandId>,
    /// The nominal stamp of the last served command. Only a host-owned transfer
    /// faults a packet stamped before it (ADR-0078 clause 3).
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    last_served_ingress_at: Option<SampleTime>,
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    invalid_ingress_command: Option<MixedIngressCommand>,
    /// Why audio refused command service; the packet, when there is one, is
    /// retained in `invalid_ingress_command`.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    ingress_command_fault: Option<MixedIngressCommandFault>,
    /// The host-owned transfer this half was armed into; `None` for embedded rings.
    #[cfg(feature = "mixed-transfer-rehearsal")]
    transfer_binding: Option<MixedTransferBinding>,
    /// Present only when the two-source bridge owns this transfer.
    #[cfg(feature = "mixed-transfer-rehearsal")]
    reset_seal: Option<Arc<MixedResetSeal>>,
    /// Returns each command's unused bend credits after service, and each written one
    /// after its entry drains.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    bend_credit_return: MixedBendCreditReturn,
    #[cfg(all(test, feature = "simulated-ingress"))]
    test_ingress_inflight: Vec<Option<(TimedEvent, bool)>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    test_ingress_inflight_len: usize,
    #[cfg(all(test, feature = "simulated-ingress"))]
    test_fail_after_ingress_at: Option<SampleTime>,
    #[cfg(test)]
    test_live: Option<TimedEvent>,
    #[cfg(test)]
    test_live_spent: bool,
    #[cfg(test)]
    test_live_share: EventCount,
}

/// Off-thread control for one bound mixed plan and stream.
///
/// It holds the compiled range's table, producer and span as plain values and no
/// generation state, so it can pair edges by token but cannot mint, stamp or release.
/// It exposes no independent schedule or activation builder.
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedStreamControl;
/// fn mint(control: &mut MixedStreamControl) { let _ = control.minter(); }
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedStreamControl;
/// use synth_engine_v2::plan::NoteSlot;
/// use synth_engine_v2::quantities::KeyIdentity;
/// fn mint(control: &mut MixedStreamControl, slot: NoteSlot, key: KeyIdentity) {
///     let _ = control.mint_keyed(slot, key);
/// }
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedStreamControl;
/// use synth_engine_v2::identity::NoteIdentity;
/// fn release(control: &mut MixedStreamControl, identity: NoteIdentity) {
///     let _ = control.release(identity);
/// }
/// ```
#[derive(Debug)]
#[must_use]
pub struct MixedStreamControl {
    epoch: StreamEpoch,
    anchor: StreamAnchor,
    plan: Arc<CompiledPlan>,
    stream: AdmittedCompiledStream,
    table: TableId,
    compiled_producer: ProducerId,
    compiled_span: Range,
    compiled_slots: Vec<NoteSlot>,
    partition: Arc<MixedInstancePartition>,
}

/// Audio-side custody and renderer for a split-born mixed stream: both range minters
/// for the owner's whole life, and the compiled token table bounded by the compiled
/// span. It exposes no mixed rendering until release and parameter ownership are proved.
#[derive(Debug)]
#[must_use]
pub struct MixedStreamAudio {
    renderer: PreparedRenderer,
    live_minter: LiveRangeMinter,
    compiled_minter: CompiledRangeMinter,
    /// What each compiled token of the list in force names.
    compiled_tokens: Vec<MixedCompiledTokenState>,
    note: NoteSlot,
    partition: Arc<MixedInstancePartition>,
    compiled_ended: Vec<Option<crate::identity::EndedNote>>,
}

/// What one compiled token names on the audio half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MixedCompiledTokenState {
    Free,
    Open(NoteIdentity),
    /// A Panic ended the occurrence at its execution sample and freed its identity. The
    /// list's own `Off` frees the token; its expression and bend edges until then are
    /// consumed and counted rather than faulted.
    #[cfg_attr(
        not(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        )),
        allow(dead_code, reason = "only a served Panic ends a compiled token")
    )]
    PanicEnded,
}

fn mixed_collection_pair(control: &MixedStreamControl, audio: &MixedOneShotAudio) -> bool {
    let rendered = &audio.audio;
    Arc::ptr_eq(&control.partition, &rendered.partition)
        && rendered.renderer.plan().id() == control.plan.id()
        && rendered.renderer.epoch() == control.epoch
        && rendered.renderer.table_id() == control.table
        && rendered.live_minter.id() == control.table
        && rendered.compiled_minter.id() == control.table
        && control.partition.compiled_producer() == control.compiled_producer
        && rendered.compiled_minter.producer() == control.compiled_producer
        && control.partition.spans().0 == control.compiled_span
        && rendered.compiled_minter.span() == control.compiled_span
        && control.partition.live_producer() == rendered.live_minter.producer()
        && control.partition.spans().1 == rendered.live_minter.span()
}

struct MixedTeardownParts {
    sounding: MixedSoundingSnapshot,
    boundary_ended: Vec<crate::identity::EndedNote>,
    offer_boundary_ended: Vec<crate::identity::EndedNote>,
    stop_boundary_ended: Vec<crate::identity::EndedNote>,
    unpublished_live: Option<NoteIdentity>,
    charged_unregistered_live: Option<NoteIdentity>,
    compiled_minted: Vec<NoteIdentity>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress: MixedIngressTeardown,
}

fn mixed_teardown_parts(
    audio: &MixedOneShotAudio,
) -> Result<MixedTeardownParts, MixedCollectionError> {
    #[cfg(feature = "mixed-transfer-rehearsal")]
    if audio.continuation.is_some() {
        return Err(MixedCollectionError::ContinuationPending);
    }
    let sounding = audio.audio.renderer.snapshot_mixed_sounding()?;
    let released = audio
        .released_compiled
        .as_usize()
        .ok_or(MixedCollectionError::EndedPrefix)?;
    let prefix = audio
        .audio
        .compiled_ended
        .get(..released)
        .ok_or(MixedCollectionError::EndedPrefix)?;
    let boundary_ended = prefix
        .iter()
        .copied()
        .collect::<Option<Vec<_>>>()
        .ok_or(MixedCollectionError::EndedPrefix)?;
    // A later adoption's release lives in its own offer cell until control collects
    // it; a pending cell holds one only if custody faulted after the release.
    let offer_cell = match &audio.offer_slot {
        MixedOfferSlot::Empty => None,
        MixedOfferSlot::Pending(offer) => Some(&*offer.0),
        MixedOfferSlot::Retired(retired) => Some(&*retired.cell),
    };
    let offer_boundary_ended = match offer_cell {
        None => Vec::new(),
        Some(cell) => ended_prefix(&cell.ended, cell.released)?,
    };
    let stop_boundary_ended = ended_prefix(&audio.stop_ended, audio.stop_released)?;
    #[cfg(test)]
    let unregistered_live = audio.test_live.and_then(|event| match event.payload() {
        EventPayload::Note { identity, .. }
            if !sounding.live().iter().any(|note| note.identity == identity) =>
        {
            Some(identity)
        }
        _ => None,
    });
    #[cfg(test)]
    let (unpublished_live, charged_unregistered_live) = if audio.test_live_spent {
        (None, unregistered_live)
    } else {
        (unregistered_live, None)
    };
    #[cfg(not(test))]
    let (unpublished_live, charged_unregistered_live) = (None, None);
    #[cfg(all(test, feature = "simulated-ingress"))]
    let ingress = {
        let mut queue = vec![None; audio.test_ingress.queue_capacity()];
        let len = audio
            .test_ingress
            .copy_queue_for_test(&mut queue)
            .ok_or(MixedCollectionError::IngressJournal)?;
        MixedIngressTeardown {
            queued: queue.into_iter().take(len).flatten().collect(),
            charged_in_faulted_callback: audio
                .test_ingress_inflight
                .iter()
                .take(audio.test_ingress_inflight_len)
                .copied()
                .flatten()
                .collect(),
            minted_live: audio.audio.live_minter.live_identities_for_test(),
            holds_outstanding: audio.test_ingress.holds_outstanding(),
            counters: audio.test_ingress.counters(),
            occurrences: audio.occurrences.occupied(),
            pedals_down: audio.occurrences.pedals_down(),
            released_after_refusal: audio.occurrences.released_after_refusal(),
            released_after_panic: audio.occurrences.released_after_panic(),
            invalid_command: None,
            invalid_result: None,
            last_served_id: audio.last_served_ingress_id,
            unserved_commands: Vec::new(),
            pending_command: None,
            parked_panic: None,
            uncollected_results: Vec::new(),
            pending_result: None,
            terminal_control: None,
        }
    };
    Ok(MixedTeardownParts {
        sounding,
        boundary_ended,
        offer_boundary_ended,
        stop_boundary_ended,
        unpublished_live,
        charged_unregistered_live,
        compiled_minted: audio.audio.compiled_minter.live_identities(),
        #[cfg(all(test, feature = "simulated-ingress"))]
        ingress,
    })
}

/// The recorded `released` prefix of one boundary's ended-note storage.
fn ended_prefix(
    ended: &[Option<crate::identity::EndedNote>],
    released: HeldNoteCount,
) -> Result<Vec<crate::identity::EndedNote>, MixedCollectionError> {
    released
        .as_usize()
        .and_then(|released| ended.get(..released))
        .ok_or(MixedCollectionError::EndedPrefix)?
        .iter()
        .copied()
        .collect::<Option<Vec<_>>>()
        .ok_or(MixedCollectionError::EndedPrefix)
}

/// Move the offer slot's contents out whole, off-thread, leaving it empty.
fn drain_offer_slot(audio: &mut MixedOneShotAudio) -> MixedDrainedOffer {
    match std::mem::take(&mut audio.offer_slot) {
        MixedOfferSlot::Empty => MixedDrainedOffer::Empty,
        MixedOfferSlot::Pending(offer) => MixedDrainedOffer::Withdrawn(offer),
        MixedOfferSlot::Retired(retired) => MixedDrainedOffer::Retired(retired),
    }
}

fn mixed_finish_teardown(
    mut audio: MixedOneShotAudio,
    end: MixedCollectionEnd,
    parts: MixedTeardownParts,
) -> MixedOneShotTeardown {
    let report = audio.report();
    // After the report, which names its stamp: a Panic audio held unserved leaves with
    // the teardown, unapplied. Collection refuses a live pair while its result is owed,
    // so only a terminal pair can hold one.
    #[cfg(all(test, feature = "simulated-ingress"))]
    let mut parts = parts;
    #[cfg(all(test, feature = "simulated-ingress"))]
    {
        parts.ingress.parked_panic = audio.parked_panic.take();
    }
    let offer = drain_offer_slot(&mut audio);
    // Both event lists, the capsule and renderer are finally dropped here, off-thread.
    drop(audio);
    MixedOneShotTeardown {
        end,
        report,
        sounding: parts.sounding,
        boundary_ended: parts.boundary_ended,
        offer_boundary_ended: parts.offer_boundary_ended,
        stop_boundary_ended: parts.stop_boundary_ended,
        offer,
        unpublished_live: parts.unpublished_live,
        charged_unregistered_live: parts.charged_unregistered_live,
        compiled_minted: parts.compiled_minted,
        #[cfg(all(test, feature = "simulated-ingress"))]
        ingress: parts.ingress,
    }
}

#[allow(dead_code)] // Private off-thread collection has no production host caller.
impl MixedOneShotControl {
    /// Prepare a later Start or seek candidate from the bound plan, off-thread.
    ///
    /// It runs the first activation's history, suffix and stamping builders against
    /// this control's bound stream and seals the list as superseding `supersedes`. That
    /// must be the sequence this control holds in force, and `requested` must follow the
    /// in-force transition's boundary. Control holds no minter, so preparing mints, frees
    /// and reserves nothing, and changes no control state: two candidates prepared in
    /// turn supersede the same sequence, and at most one of them can ever adopt.
    #[allow(dead_code)] // Only private tests and the transfer rehearsal prepare a later candidate.
    pub(crate) fn prepare_subsequent(
        &self,
        requested: SampleTime,
        position: PlanPosition,
        supersedes: ActivationSequence,
    ) -> Result<MixedStampedCandidate, MixedSubsequentPrepareError> {
        self.check_subsequent(requested, supersedes)?;
        self.stamp_subsequent(requested, position, None, supersedes)
    }

    /// Prepare a later candidate that enters `interval` at `position`, off-thread.
    ///
    /// The entry list is [`Self::prepare_subsequent`]'s, cut before the loop's end, so
    /// its first pass plays from `position` and nothing at or after `end`. The repeating
    /// template is the same builders' reconstruction from `start`, cut the same way and
    /// stamped from engine time zero; a wrap shifts it to its own sample. Like every
    /// later candidate it mints, frees and reserves nothing and changes no control state.
    #[allow(dead_code)] // Only private tests and the transfer rehearsal enter a loop.
    pub(crate) fn prepare_loop(
        &self,
        requested: SampleTime,
        position: PlanPosition,
        interval: LoopInterval,
        supersedes: ActivationSequence,
    ) -> Result<MixedLoopCandidate, MixedSubsequentPrepareError> {
        self.check_subsequent(requested, supersedes)?;
        if position < interval.start() || position > interval.end() {
            return Err(MixedSubsequentPrepareError::LoopEntry {
                position,
                start: interval.start(),
                end: interval.end(),
            });
        }
        let entry_at = if position == interval.end() {
            interval.start()
        } else {
            position
        };
        let entry = self.stamp_subsequent(requested, entry_at, Some(interval.end()), supersedes)?;
        let history = self
            .control
            .prepare_history(SampleTime::ZERO, interval.start())?;
        let suffix = self
            .control
            .prepare_suffix_until(history, Some(interval.end()))
            .map_err(|refusal| refusal.0)?;
        let template = self.control.stamp_events(&suffix)?;
        let program = MixedLoopProgram {
            interval,
            events: template.events,
            restoration_count: suffix.history.restoration_count,
            end_open: template.outstanding_count,
            omitted_releases: suffix.omitted_releases,
            omitted_expressions: suffix.omitted_expressions,
        };
        Ok(MixedLoopCandidate {
            entry,
            program: Box::new(program),
        })
    }

    /// The sequence and boundary checks every later candidate passes before building.
    fn check_subsequent(
        &self,
        requested: SampleTime,
        supersedes: ActivationSequence,
    ) -> Result<(), MixedSubsequentPrepareError> {
        if supersedes != self.in_force {
            return Err(MixedSubsequentPrepareError::Superseded {
                supersedes,
                in_force: self.in_force,
            });
        }
        if supersedes.next().is_none() {
            return Err(MixedSubsequentPrepareError::SequenceExhausted);
        }
        if requested <= self.in_force_effective {
            return Err(MixedSubsequentPrepareError::NotAfterInForce {
                requested,
                boundary: self.in_force_effective,
            });
        }
        Ok(())
    }

    /// History, suffix and stamping for one later candidate; `end` cuts the suffix.
    fn stamp_subsequent(
        &self,
        requested: SampleTime,
        position: PlanPosition,
        end: Option<PlanPosition>,
        supersedes: ActivationSequence,
    ) -> Result<MixedStampedCandidate, MixedSubsequentPrepareError> {
        let history = self.control.prepare_history(requested, position)?;
        let suffix = self
            .control
            .prepare_suffix_until(history, end)
            .map_err(|refusal| refusal.0)?;
        self.control
            .stamp_suffix(suffix, supersedes)
            .map_err(|refusal| MixedSubsequentPrepareError::Stamp(refusal.0))
    }

    /// Seal one later candidate as an offer that takes effect at `effective`, off-thread.
    ///
    /// The candidate must supersede the sequence this control holds in force, and
    /// `effective` must be a quantum boundary at or after its requested time and after
    /// the in-force transition's boundary. It is admitted against the arm profile at
    /// exactly that boundary's displacement, by the check that admitted the armed
    /// transition, so audio never holds an unadmitted candidate. The offer carries its
    /// own ended-note storage for the boundary release. `kind` says whether it resumes
    /// a stopped program; audio checks it against the transport at the boundary, which
    /// control does not track. Sealing changes no control state and no minter; every
    /// refusal returns the candidate unchanged.
    #[allow(dead_code)] // Only private tests and the transfer rehearsal seal an offer.
    pub(crate) fn seal_offer(
        &self,
        candidate: MixedStampedCandidate,
        effective: SampleTime,
        kind: MixedOfferKind,
    ) -> Result<MixedCandidateOffer, Box<(MixedOfferPrepareError, MixedStampedCandidate)>> {
        let control = &self.control;
        let history = &candidate.suffix.history;
        if history.plan != control.plan.id()
            || history.epoch != control.epoch
            || history.table != control.table
        {
            return Err(Box::new((
                MixedOfferPrepareError::ForeignCandidate,
                candidate,
            )));
        }
        if candidate.supersedes != self.in_force {
            return Err(Box::new((
                MixedOfferPrepareError::Superseded {
                    supersedes: candidate.supersedes,
                    in_force: self.in_force,
                },
                candidate,
            )));
        }
        if effective <= self.in_force_effective {
            return Err(Box::new((
                MixedOfferPrepareError::NotAfterInForce {
                    effective,
                    boundary: self.in_force_effective,
                },
                candidate,
            )));
        }
        let timing = match candidate.effective_timing(effective) {
            Ok(timing) => timing,
            Err(error) => return Err(Box::new((error.into(), candidate))),
        };
        if let Err(error) = admit_one_shot_candidate(
            &control.plan,
            &control.partition,
            &candidate,
            timing,
            &self.profile,
        ) {
            return Err(Box::new((error.into(), candidate)));
        }
        let ended = vec![None; control.compiled_span.indices().len()];
        let candidate = match candidate.into_audio_parts() {
            Ok(candidate) => candidate,
            Err(boxed) => {
                let (error, candidate) = *boxed;
                return Err(Box::new((error.into(), candidate)));
            }
        };
        Ok(MixedCandidateOffer(Box::new(MixedOfferCell {
            #[cfg(feature = "mixed-transfer-rehearsal")]
            activation_custody: None,
            candidate,
            timing,
            ended,
            released: HeldNoteCount::NONE,
            late: false,
            kind,
            loop_program: None,
        })))
    }

    /// Seal a loop entry as an offer that takes effect at `effective`, off-thread.
    ///
    /// The loop is admitted first, from the entry boundary: Session carries the boundary
    /// release and restoration at entry, and at every wrap one `Off` per note the pass
    /// leaves open, one operation and restoration again; Compiled carries the entry list
    /// and every later pass. The entry-to-first-wrap junction and the periodic steady
    /// state are both checked over sliding windows, so the result holds for every quantum
    /// alignment a wrap can take. The entry list then passes [`Self::seal_offer`]
    /// unchanged. Wraps charge nothing to Release or Live. Every refusal returns the
    /// candidate whole and changes no control state.
    ///
    /// A loop sealed as a `Seek` that adopts while the program is stopped stays inert:
    /// a stopped program never wraps, and the Start that resumes it brings its own loop
    /// or none.
    #[allow(dead_code)] // Only private tests and the transfer rehearsal enter a loop.
    pub(crate) fn seal_loop_offer(
        &self,
        candidate: MixedLoopCandidate,
        effective: SampleTime,
        kind: MixedOfferKind,
    ) -> Result<MixedCandidateOffer, Box<(MixedOfferPrepareError, MixedLoopCandidate)>> {
        if let Err(error) = self.admit_loop(&candidate, effective) {
            return Err(Box::new((error, candidate)));
        }
        let MixedLoopCandidate { entry, program } = candidate;
        match self.seal_offer(entry, effective, kind) {
            Ok(mut offer) => {
                offer.0.loop_program = Some(program);
                Ok(offer)
            }
            Err(boxed) => {
                let (error, entry) = *boxed;
                Err(Box::new((error, MixedLoopCandidate { entry, program })))
            }
        }
    }

    /// The loop half of [`Self::seal_loop_offer`]: shape checks the hot path relies on,
    /// then Session and Compiled admission against the arm profile.
    fn admit_loop(
        &self,
        candidate: &MixedLoopCandidate,
        effective: SampleTime,
    ) -> Result<(), MixedOfferPrepareError> {
        use MixedOfferPrepareError::LoopShape;

        let entry = &candidate.entry;
        let program = &candidate.program;
        let groups = self.control.partition.restoration_groups().len();
        let restoration = program.restoration_count.as_usize().ok_or(LoopShape)?;
        if restoration != groups
            || entry.restoration_count.as_usize() != Some(groups)
            || restoration > program.events.len()
            || groups > entry.events.len()
        {
            return Err(LoopShape);
        }
        let start = program.interval.start().as_u64();
        let length = program.interval.end().as_u64() - start;
        let position = entry.anchor.position().as_u64();
        let first_wrap = position
            .checked_sub(start)
            .filter(|into| *into < length)
            .map(|into| length - into)
            .ok_or(LoopShape)?;
        if effective.checked_add(FrameCount::new(first_wrap)).is_err() {
            return Err(LoopShape);
        }
        // The hot path ends each pass at its wrap, so every pass event must precede it.
        let requested = entry.anchor.time().as_u64();
        let entry_compiled = entry
            .events
            .get(groups..)
            .ok_or(LoopShape)?
            .iter()
            .map(|event| {
                event
                    .envelope()
                    .time()
                    .as_u64()
                    .checked_sub(requested)
                    .filter(|offset| *offset < first_wrap)
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(LoopShape)?;
        let (template_restoration, template) = program.events.split_at(restoration);
        if template_restoration.iter().any(|event| {
            event.envelope().time() != SampleTime::ZERO
                || !matches!(event.payload(), MixedCompiledPayload::Restore(_))
        }) {
            return Err(LoopShape);
        }
        let repeating_compiled = template
            .iter()
            .map(|event| {
                let offset = event.envelope().time().as_u64();
                (offset < length && !matches!(event.payload(), MixedCompiledPayload::Restore(_)))
                    .then_some(offset)
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(LoopShape)?;
        let wrap_session = |open: HeldNoteCount| {
            open.as_usize()
                .and_then(|open| open.checked_add(1))
                .and_then(|cost| cost.checked_add(groups))
                .ok_or(LoopShape)
        };
        crate::looping::admit_loop_shape(
            &crate::looping::LoopAdmissionShape {
                length,
                first_wrap,
                entry_session: groups.checked_add(1).ok_or(LoopShape)?,
                entry_compiled: &entry_compiled,
                first_wrap_session: wrap_session(entry.outstanding_count)?,
                wrap_session: wrap_session(program.end_open)?,
                repeating_compiled: &repeating_compiled,
            },
            &self.profile,
        )
        .map_err(|error| match error {
            crate::looping::LoopPrepareError::Admission { class, source } => {
                MixedOfferPrepareError::LoopAdmission { class, source }
            }
            _ => LoopShape,
        })
    }

    /// Take back one retired offer and its receipt, off-thread.
    ///
    /// Only a receipt confirming adoption advances this control's in-force sequence,
    /// boundary and anchor; a refused or cancelled one changes nothing. The retired list
    /// and the offer's storage are dropped here.
    #[allow(dead_code)] // Only private tests and the transfer rehearsal collect an offer.
    pub(crate) fn collect_offer(
        &mut self,
        retired: MixedRetiredOffer,
    ) -> Result<MixedOfferReceipt, Box<(MixedOfferCollectError, MixedRetiredOffer)>> {
        let candidate = &retired.cell.candidate;
        if candidate.plan != self.control.plan.id()
            || candidate.epoch != self.control.epoch
            || candidate.table != self.control.table
        {
            return Err(Box::new((
                MixedOfferCollectError::ForeignCandidate,
                retired,
            )));
        }
        let receipt = retired.receipt;
        if let MixedOfferOutcome::Adopted(adopted) = receipt.outcome {
            if receipt.supersedes != self.in_force {
                return Err(Box::new((
                    MixedOfferCollectError::Superseded {
                        supersedes: receipt.supersedes,
                        in_force: self.in_force,
                    },
                    retired,
                )));
            }
            self.in_force = receipt.sequence;
            self.in_force_effective = adopted.effective;
            self.control.anchor = adopted.anchor;
        }
        drop(retired);
        Ok(receipt)
    }

    /// Rejoin a stopped adopted pair or tear down a pending or terminal pair.
    /// A crossed pair and a classification refusal return both halves untouched.
    pub(crate) fn collect(
        mut self,
        mut audio: MixedOneShotAudio,
    ) -> Result<MixedCollection, Box<MixedCollectionRefusal<Self>>> {
        if !mixed_collection_pair(&self.control, &audio) {
            return Err(Box::new(MixedCollectionRefusal {
                reason: MixedCollectionError::CrossedPair,
                control: self,
                audio,
            }));
        }
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let ingress_faulted = audio.invalid_ingress_command.is_some()
            || self.invalid_ingress_result.is_some()
            || self.ingress_halt.load(Ordering::Acquire);
        #[cfg(not(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        )))]
        let ingress_faulted = false;
        let terminal = ingress_faulted
            || audio.fault.is_some()
            || audio.audio.renderer.diagnostics().needs_reprepare();
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        if !terminal
            && (self.ingress_outstanding > MixedIngressQueueCount::NONE
                || self.panic_inflight.is_some())
        {
            return Err(Box::new(MixedCollectionRefusal {
                reason: MixedCollectionError::IngressResultPending,
                control: self,
                audio,
            }));
        }
        if !audio.adopted || terminal {
            let end = if terminal {
                MixedCollectionEnd::Faulted
            } else {
                MixedCollectionEnd::Pending
            };
            let parts = match mixed_teardown_parts(&audio) {
                Ok(parts) => parts,
                Err(reason) => {
                    return Err(Box::new(MixedCollectionRefusal {
                        reason,
                        control: self,
                        audio,
                    }));
                }
            };
            #[cfg(all(test, feature = "simulated-ingress"))]
            let mut parts = parts;
            #[cfg(all(test, feature = "simulated-ingress"))]
            if terminal {
                parts.ingress.invalid_command = audio.invalid_ingress_command.take();
                parts.ingress.invalid_result = self.invalid_ingress_result.take();
                parts.ingress.pending_command = self.ingress_command_pending.take();
                parts.ingress.terminal_control = Some(MixedTerminalControlCustody {
                    terminal_identity_fault: self.terminal_identity_fault,
                    ingress_outstanding: self.ingress_outstanding,
                    release_reserved: self.release_reserved,
                    pedal_reserved: MixedIngressQueueCount(self.pedal_ledger.reserved()),
                    panic_inflight: self.panic_inflight,
                    bend_credits_free: self.bend_credits_free,
                    bend_credits_return_pending: self.bend_credit_return.pending(),
                    bend_scopes: self.bend_scopes.iter().flatten().copied().collect(),
                    outstanding_commands: self.ingress_journal.iter().copied().collect(),
                });
                parts.ingress.pending_result = audio.ingress_result_pending.take();
                while let Some(command) = audio.ingress_commands.0.try_pop() {
                    parts.ingress.unserved_commands.push(command);
                }
                while let Some(result) = self.ingress_results.0.try_pop() {
                    parts.ingress.uncollected_results.push(result);
                }
            }
            drop(self);
            return Ok(MixedCollection::Ended(Box::new(mixed_finish_teardown(
                audio, end, parts,
            ))));
        }
        // Both minters stayed with audio, so collection moves only plain values to
        // control: the effective anchor and the in-force activation sequence. A later
        // adoption moves audio's sequence past the armed one, never below it.
        let capsule = &audio.capsule;
        let consistent = capsule.plan == self.control.plan.id()
            && capsule.epoch == self.control.epoch
            && capsule.table == self.control.table
            && audio.in_force >= capsule.sequence
            && capsule.supersedes == ActivationSequence::INITIAL
            && audio.audio.renderer.mixed_anchor() == audio.effective_anchor;
        let retired = match (capsule.retired_anchor, capsule.retired_next) {
            (Some(anchor), Some(next)) if consistent && next <= capsule.events.len() => {
                Some((anchor, next))
            }
            _ => None,
        };
        let Some((retired_anchor, retired_next)) = retired else {
            let parts = match mixed_teardown_parts(&audio) {
                Ok(parts) => parts,
                Err(reason) => {
                    return Err(Box::new(MixedCollectionRefusal {
                        reason,
                        control: self,
                        audio,
                    }));
                }
            };
            drop(self);
            return Ok(MixedCollection::Ended(Box::new(mixed_finish_teardown(
                audio,
                MixedCollectionEnd::PromotionRefused,
                parts,
            ))));
        };
        self.control.anchor = audio.effective_anchor;
        let retired = Box::new(MixedRetiredList {
            anchor: retired_anchor,
            unconsumed_from: retired_next,
            event_count: audio.capsule.events.len(),
        });
        audio.capsule.retired_next = None;
        drop(std::mem::take(&mut audio.capsule.events));
        // The slot moves to the caller whole: a pending candidate minted nothing, so it
        // is withdrawn, and a retired one keeps its receipt and boundary record.
        let offer = Box::new(drain_offer_slot(&mut audio));
        Ok(MixedCollection::Resumed {
            control: MixedResumedControl {
                control: self.control,
                sequence: audio.in_force,
                off_thread: PhantomData,
            },
            audio: Box::new(audio),
            retired,
            offer,
        })
    }
}

#[allow(dead_code)] // Private resumed stream has no production host caller.
impl MixedResumedControl {
    /// Final teardown after a resumed adopted stream stops rendering.
    pub(crate) fn teardown(
        self,
        audio: MixedOneShotAudio,
    ) -> Result<MixedOneShotTeardown, Box<MixedCollectionRefusal<Self>>> {
        if !mixed_collection_pair(&self.control, &audio) {
            return Err(Box::new(MixedCollectionRefusal {
                reason: MixedCollectionError::CrossedPair,
                control: self,
                audio,
            }));
        }
        if !audio.adopted || audio.capsule.retired_next.is_some() {
            return Err(Box::new(MixedCollectionRefusal {
                reason: MixedCollectionError::UncollectedAudio,
                control: self,
                audio,
            }));
        }
        let parts = match mixed_teardown_parts(&audio) {
            Ok(parts) => parts,
            Err(reason) => {
                return Err(Box::new(MixedCollectionRefusal {
                    reason,
                    control: self,
                    audio,
                }));
            }
        };
        drop(self);
        Ok(mixed_finish_teardown(
            audio,
            MixedCollectionEnd::ResumedTeardown,
            parts,
        ))
    }
}

impl MixedStreamControl {
    /// Prepare both split owners from one target binding before exposing either one.
    /// A refusal returns the binding so the caller can retry or retain the plan.
    pub(crate) fn open(
        binding: MixedTargetAdmission,
        anchor: StreamAnchor,
    ) -> Result<(Self, MixedStreamAudio), Box<(MixedStreamOpenError, MixedTargetAdmission)>> {
        if let Some(program) = binding
            .plan()
            .prepared_scripts()
            .iter()
            .find(|program| program.domain == crate::script::ScriptDomain::Note)
        {
            return Err(Box::new((
                MixedStreamOpenError::Compile(CompileError::Script {
                    node: program.node,
                    fault: crate::script::ScriptFault::NoteSourceRequired,
                }),
                binding,
            )));
        }
        let epoch = match issue_epoch() {
            Ok(epoch) => epoch,
            Err(error) => {
                return Err(Box::new((
                    MixedStreamOpenError::Compile(error.into()),
                    binding,
                )));
            }
        };
        let table = match IdentityTable::from_admitted_ranges(binding.plan().note_producer_ranges())
        {
            Ok(table) => table,
            Err(error) => {
                return Err(Box::new((
                    MixedStreamOpenError::Compile(error.into()),
                    binding,
                )));
            }
        };
        let table_id = table.id();
        let Ok((first, second)) = table.split_fresh_two() else {
            // This table was just constructed, so it holds no obligations if split refuses.
            return Err(Box::new((MixedStreamOpenError::Partition, binding)));
        };
        let (compiled, live) = if binding.live_producer() == ProducerId::new(0) {
            (second, first)
        } else {
            (first, second)
        };
        let compiled = CompiledRangeMinter::from_partition(compiled);
        let live = LiveRangeMinter::from_partition(live);
        let (compiled_span, live_span) = binding.spans();
        if compiled.span() != compiled_span
            || live.span() != live_span
            || live.producer() != binding.live_producer()
            || compiled.id() != table_id
            || live.id() != table_id
            || binding.instance_partition().plan_id() != binding.plan().id()
            || binding.instance_partition().compiled_producer() != compiled.producer()
            || binding.instance_partition().live_producer() != live.producer()
            || binding.instance_partition().spans() != (compiled.span(), live.span())
        {
            return Err(Box::new((MixedStreamOpenError::Partition, binding)));
        }
        let mut renderer = match PreparedRenderer::prepare_mixed(
            Arc::clone(binding.plan_arc()),
            anchor,
            epoch,
            table_id,
        ) {
            Ok(renderer) => renderer,
            Err(error) => {
                return Err(Box::new((MixedStreamOpenError::Compile(error), binding)));
            }
        };
        if !renderer.bind_mixed_partition(Arc::clone(binding.partition_arc())) {
            return Err(Box::new((MixedStreamOpenError::Partition, binding)));
        }
        let parts = binding.into_parts();
        let audio_partition = Arc::clone(&parts.partition);
        let compiled_ended = vec![None; compiled_span.indices().len()];
        let compiled_tokens = vec![MixedCompiledTokenState::Free; compiled_span.indices().len()];
        Ok((
            Self {
                epoch,
                anchor,
                plan: parts.plan,
                stream: parts.stream,
                table: table_id,
                compiled_producer: compiled.producer(),
                compiled_span,
                compiled_slots: parts.compiled_slots,
                partition: parts.partition,
            },
            MixedStreamAudio {
                renderer,
                live_minter: live,
                compiled_minter: compiled,
                compiled_tokens,
                note: parts.live_slot,
                partition: audio_partition,
                compiled_ended,
            },
        ))
    }

    /// The immutable plan checked by the target binding.
    pub fn plan(&self) -> &CompiledPlan {
        &self.plan
    }

    /// The only compiled stream this owner may later place and pair by token.
    pub const fn stream(&self) -> &AdmittedCompiledStream {
        &self.stream
    }

    /// The stream epoch issued at construction.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// The initial transport anchor.
    pub const fn anchor(&self) -> StreamAnchor {
        self.anchor
    }

    /// The single table identity shared with the audio owner.
    pub const fn table_id(&self) -> TableId {
        self.table
    }

    /// The compiled producer whose edges this control pairs by token. Only the audio
    /// half mints for it.
    pub const fn compiled_producer(&self) -> ProducerId {
        self.compiled_producer
    }

    /// The note slots reached by the owned compiled stream.
    pub fn compiled_slots(&self) -> &[NoteSlot] {
        &self.compiled_slots
    }

    /// The immutable local-instance partition shared with the audio half.
    pub fn instance_partition(&self) -> &MixedInstancePartition {
        &self.partition
    }

    /// Place the bound stream at this control's anchor and pair it by token.
    /// Nothing is minted; the result is discarded or sealed by the caller.
    fn tokenize_bound(&self) -> Result<MixedTokenized, MixedInitialPrepareError> {
        if self.plan.stealing() != crate::ir::StealingPolicy::None {
            return Err(MixedInitialPrepareError::UnexpectedSteal);
        }
        let placed = crate::schedule::place_admitted(&self.stream, self.anchor)?;
        tokenize_compiled(
            &self.plan,
            self.compiled_producer,
            self.compiled_span,
            self.epoch,
            &placed,
        )
        .map_err(|error| match error {
            MixedTokenizeError::Schedule(error) => error.into(),
            MixedTokenizeError::UnexpectedWriter { event_index } => {
                MixedInitialPrepareError::UnexpectedWriter { event_index }
            }
        })
    }
}

impl MixedJoinedStream {
    /// Construct the joined owner directly from one checked target binding.
    pub fn open(
        binding: MixedTargetAdmission,
        anchor: StreamAnchor,
    ) -> Result<Self, Box<(MixedStreamOpenError, MixedTargetAdmission)>> {
        let (control, audio) = MixedStreamControl::open(binding, anchor)?;
        Ok(Self {
            control,
            audio,
            off_thread: PhantomData,
        })
    }

    /// Read-only compiled custody for this joined owner.
    pub const fn control(&self) -> &MixedStreamControl {
        &self.control
    }

    /// Read-only live custody for this joined owner.
    pub const fn audio(&self) -> &MixedStreamAudio {
        &self.audio
    }

    /// Check the fixed compiled stream's pairing and compiled-span occupancy without
    /// creating any schedule or reservation.
    pub fn check_bound_stamp(&self) -> Result<(), MixedInitialPrepareError> {
        self.control.tokenize_bound().map(drop)
    }

    /// Seal the bound initial schedule exactly once, off-thread, as a tokenized list.
    /// Nothing is minted, so a refusal returns this owner unchanged.
    ///
    /// No separate plan, stream or anchor can be supplied:
    ///
    /// ```compile_fail
    /// use synth_engine_v2::stream::MixedJoinedStream;
    /// use synth_engine_v2::schedule::AdmittedCompiledStream;
    /// fn substitute(owner: MixedJoinedStream, stream: AdmittedCompiledStream) {
    ///     let _ = owner.prepare_initial(stream);
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use synth_engine_v2::stream::MixedJoinedStream;
    /// use synth_engine_v2::plan::CompiledPlan;
    /// fn substitute(owner: MixedJoinedStream, plan: CompiledPlan) {
    ///     let _ = owner.prepare_initial(plan);
    /// }
    /// ```
    pub fn prepare_initial(
        self,
    ) -> Result<MixedJoinedPrepared, Box<(MixedInitialPrepareError, Self)>> {
        match self.control.tokenize_bound() {
            Ok(tokenized) => Ok(MixedJoinedPrepared {
                owner: self,
                events: tokenized.events,
                outstanding: tokenized.open,
            }),
            Err(error) => Err(Box::new((error, self))),
        }
    }
}

impl MixedJoinedPrepared {
    /// Arm exactly one private transition while both halves are stopped off-thread.
    /// A refusal returns the sealed owner and candidate without changing either minter.
    #[allow(dead_code)] // The private arm has no production host caller.
    pub(crate) fn arm_one_shot(
        self,
        candidate: MixedStampedCandidate,
        profile: &HostProfile,
    ) -> Result<(MixedOneShotControl, MixedOneShotAudio), Box<MixedOneShotArmRefusal>> {
        self.arm_one_shot_with(
            candidate,
            profile,
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            None,
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            MixedPanicCapability::Available,
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            crate::ingress::IngressOrigin::Simulated,
        )
    }

    /// [`Self::arm_one_shot`] with explicit bend-table widths; `None` keeps the
    /// two-source MIDI1 floor.
    #[allow(dead_code)] // The private arm has no production host caller.
    fn arm_one_shot_with(
        self,
        candidate: MixedStampedCandidate,
        profile: &HostProfile,
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        bend_tables: Option<MixedBendTables>,
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        panic_capability: MixedPanicCapability,
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        ingress_origin: crate::ingress::IngressOrigin,
    ) -> Result<(MixedOneShotControl, MixedOneShotAudio), Box<MixedOneShotArmRefusal>> {
        let control = &self.owner.control;
        let audio = &self.owner.audio;
        let history = &candidate.suffix.history;
        if history.plan != control.plan.id()
            || history.epoch != control.epoch
            || history.table != control.table
        {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::ForeignCandidate,
                owner: self,
                candidate,
            }));
        }
        if audio.renderer.plan().id() != control.plan.id()
            || audio.renderer.epoch() != control.epoch
            || audio.renderer.table_id() != control.table
            || audio.table_id() != control.table
            || audio.compiled_minter.id() != control.table
            || audio.compiled_minter.producer() != control.compiled_producer
            || audio.compiled_minter.span() != control.compiled_span
            || audio.compiled_minter.live() != 0
            || audio.compiled_tokens.len() != control.compiled_span.indices().len()
            || audio
                .compiled_tokens
                .iter()
                .any(|token| *token != MixedCompiledTokenState::Free)
        {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::ForeignAudio,
                owner: self,
                candidate,
            }));
        }
        if audio.renderer.diagnostics().needs_reprepare() {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::FaultedRenderer,
                owner: self,
                candidate,
            }));
        }
        if let Err(error) = audio
            .renderer
            .check_mixed_boundary_storage(audio.compiled_ended.len())
        {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: error.into(),
                owner: self,
                candidate,
            }));
        }
        if candidate.supersedes != ActivationSequence::INITIAL {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::WrongSequence,
                owner: self,
                candidate,
            }));
        }
        let arm_clock = audio.renderer.clock();
        let at = candidate.anchor.time().max(arm_clock);
        let Some(effective) = super::next_boundary(at) else {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::BoundaryUnrepresentable { at },
                owner: self,
                candidate,
            }));
        };
        let timing = match candidate.effective_timing(effective) {
            Ok(timing) => timing,
            Err(error) => {
                return Err(Box::new(MixedOneShotArmRefusal {
                    reason: error.into(),
                    owner: self,
                    candidate,
                }));
            }
        };
        let arbiter = match prepare_one_shot_arbiter(
            &control.plan,
            &control.partition,
            &candidate,
            timing,
            profile,
        ) {
            Ok(arbiter) => arbiter,
            Err(error) => {
                return Err(Box::new(MixedOneShotArmRefusal {
                    reason: error.into(),
                    owner: self,
                    candidate,
                }));
            }
        };
        // The private mixed bound is composed before either owner is consumed:
        // profile admission covers the sum of all class shares and queue depth <= Live;
        // the arbiter checks the plan's release-hold sum, actual Session
        // release/restoration, shifted Compiled suffix, scoped fanout and
        // full-quantum event storage. The renderer check above
        // covers the widest payload's timed writes, compiled boundary queue and seed
        // rows. The ingress constructor below additionally requires a full queued
        // Release backlog to fit its share, then allocates the sole registered queue.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let test_ingress = match PerformanceIngress::prepare_mixed(
            profile,
            &control.plan,
            &audio.live_minter,
            &audio.renderer,
            ingress_origin,
        ) {
            Ok(ingress) => ingress,
            Err(error) => {
                return Err(Box::new(MixedOneShotArmRefusal {
                    reason: error.into(),
                    owner: self,
                    candidate,
                }));
            }
        };
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let Some(ingress_capacity) =
            MixedIngressQueueCount::capacity(test_ingress.queue_capacity())
        else {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::IngressWidth,
                owner: self,
                candidate,
            }));
        };
        // A sustained occurrence owns a release hold instead of a ring place, so the
        // occurrence tables cover the ring plus the live hold entitlement. A lift ends
        // its scope's sustained occurrences, and Panic every live occurrence, as one
        // group, which bounds that entitlement.
        // The ingress preparation above already refused a producer the plan lacks.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let live_holds = control
            .plan
            .note_producer_holds()
            .get(usize::from(audio.live_minter.producer().as_u16()))
            .copied()
            .unwrap_or(EventCount::NONE);
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let sustain_group = crate::ingress::RELEASE_GROUP_CAPACITY;
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let Some(holds) = live_holds
            .as_usize()
            .filter(|holds| *holds <= sustain_group)
        else {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::SustainGroupCapacity {
                    holds: live_holds,
                    group: sustain_group,
                },
                owner: self,
                candidate,
            }));
        };
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let Some(occurrence_width) = ingress_capacity
            .as_usize()
            .checked_add(holds)
            .and_then(MixedIngressQueueCount::capacity)
        else {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::IngressWidth,
                owner: self,
                candidate,
            }));
        };
        // The store withholds a bend-credit budget B = max(H, C - 2H) from ordinary
        // traffic, so B >= H covers one bend's every member. With H-1 live notes held
        // after their onsets drain, their holds use H-1 of the C - B ordinary places,
        // and the Hth onset needs its note-on and hold. At B = H that needs
        // C - H >= H + 1, so 2H + 1 is the smallest ingress depth that can realize every
        // declared live hold. H is at most one group here, so the arithmetic is bounded
        // by 17.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let required = EventCount::measured(live_holds.get() * 2 + 1);
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        if ingress_capacity.as_usize() < required.as_usize().unwrap_or(usize::MAX) {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::BendCreditCapacity {
                    capacity: profile
                        .limits()
                        .events()
                        .queues()
                        .performance_ingress_capacity(),
                    holds: live_holds,
                    required,
                },
                owner: self,
                candidate,
            }));
        }
        // Panic ends every open compiled occurrence too. None of them reserved a live
        // release place, so the cut spends the release share beside the full queued
        // live backlog the ingress preparation already proved: at most one cut per
        // quantum, of at most the compiled span, in groups of eight.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        if panic_capability == MixedPanicCapability::Available {
            let span = control.compiled_span.indices().len();
            let groups = span.div_ceil(crate::ingress::RELEASE_GROUP_CAPACITY);
            let capacity = profile
                .limits()
                .events()
                .queues()
                .performance_ingress_capacity();
            let share = profile.limits().events().shares().release_event_share();
            let required = u32::try_from(groups)
                .ok()
                .and_then(|groups| capacity.get().checked_add(groups));
            if required.is_none_or(|required| share.get() < required) {
                return Err(Box::new(MixedOneShotArmRefusal {
                    reason: MixedOneShotArmError::CompiledPanicReleaseShare {
                        share,
                        capacity,
                        compiled_span: HeldNoteCount::measured(
                            u32::try_from(span).unwrap_or(u32::MAX),
                        ),
                        required: EventCount::measured(required.unwrap_or(u32::MAX)),
                    },
                    owner: self,
                    candidate,
                }));
            }
        }
        // Both rings carry one protected place beyond the ordinary capacity: the merger
        // keeps it for one Panic, which ordinary commands and results cannot take.
        // A host-owned transfer's arm requires host queues with the same protected place
        // when it carries Panic, as the two-source bridge's does; the single-source
        // rehearsal carries none. The journal is sized for the protected place either way.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let ring_places = ingress_capacity.as_usize() + 1;
        #[cfg(all(test, feature = "simulated-ingress"))]
        let (ingress_command_writer, ingress_command_reader) =
            HeapRb::<MixedIngressCommand>::new(ring_places).split();
        #[cfg(all(test, feature = "simulated-ingress"))]
        let (ingress_result_writer, ingress_result_reader) =
            HeapRb::<MixedIngressResult>::new(ring_places).split();
        // Every bend credit starts with the merger; audio returns them through this.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let bend_credit_return = MixedBendCreditReturn::new();
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let ingress_halt = Arc::new(AtomicBool::new(false));
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let panic_current_service_at = Arc::new(AtomicU64::new(0));
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let bend_credit_budget = MixedIngressQueueCount(test_ingress.mixed_bend_budget());
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let bend_tables =
            bend_tables.unwrap_or_else(|| MixedBendTables::for_capacity(ingress_capacity));
        // Each quantum drains at most one full queue. A Panic parked until a later
        // quantum of one callback lets the commands behind it drain in that same
        // callback, so the faulted-callback journal covers one queue per quantum.
        #[cfg(all(test, feature = "simulated-ingress"))]
        let inflight_journal = test_ingress.queue_capacity().saturating_mul(
            profile
                .capabilities()
                .max_quanta_per_callback()
                .ok()
                .and_then(|quanta| usize::try_from(quanta.get()).ok())
                .unwrap_or(usize::MAX)
                .saturating_add(1),
        );
        let late_at_arm = candidate.anchor.time() < arm_clock;
        let effective_anchor = StreamAnchor::new(effective, candidate.anchor.position());
        let capsule = match candidate.into_audio_capsule() {
            Ok(capsule) => capsule,
            Err(boxed) => {
                let (error, candidate) = *boxed;
                return Err(Box::new(MixedOneShotArmRefusal {
                    reason: error.into(),
                    owner: self,
                    candidate,
                }));
            }
        };
        let Self {
            owner:
                MixedJoinedStream {
                    control,
                    audio,
                    off_thread: _,
                },
            events,
            outstanding,
        } = self;
        // The initial list's open tokens are pairing state only; audio publishes
        // that list and mints its occurrences itself.
        drop(outstanding);
        // Arm's storage preflight already proved this width for the boundary release.
        let stop_ended = vec![None; audio.compiled_ended.len()];
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let ingress_owner = MixedIngressOwner {
            epoch: control.epoch,
            table: control.table,
        };
        Ok((
            MixedOneShotControl {
                control,
                in_force: capsule.sequence,
                in_force_effective: effective,
                profile: *profile,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_owner,
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_commands: OpaqueQueue(ingress_command_writer),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_results: OpaqueQueue(ingress_result_reader),
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_journal: VecDeque::with_capacity(ring_places),
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_command_pending: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                terminal_identity_fault: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                invalid_ingress_result: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_halt: ingress_halt.clone(),
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_capacity,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_outstanding: MixedIngressQueueCount::NONE,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                release_reserved: MixedIngressQueueCount::NONE,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                last_ingress_id: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                occurrence_ledger: MixedOccurrenceLedger::prepare(occurrence_width),
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                pedal_ledger: MixedPedalLedger::prepare(ingress_capacity),
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                bend_scopes: vec![None; bend_tables.control.as_usize()],
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                bend_credits_per_bend: MixedIngressQueueCount(holds),
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                bend_credits_free: bend_credit_budget,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                bend_credit_return: bend_credit_return.clone(),
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                panic_inflight: None,
                #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
                panic_pending_at: None,
                #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
                last_panic_applied_at: None,
                #[cfg(any(test, feature = "mixed-transfer-rehearsal"))]
                last_panic_executed_at: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                panic_current_service_at: panic_current_service_at.clone(),
                off_thread: PhantomData,
            },
            MixedOneShotAudio {
                #[cfg(feature = "mixed-transfer-rehearsal")]
                continuation: None,
                #[cfg(feature = "mixed-transfer-rehearsal")]
                activation_custody: None,
                audio,
                arbiter,
                events,
                next: 0,
                capsule,
                timing,
                effective_anchor,
                late_at_arm,
                in_force: ActivationSequence::INITIAL,
                adopted: false,
                fault: None,
                releases_charged: EventCount::NONE,
                released_compiled: HeldNoteCount::NONE,
                restoration_charged: EventCount::NONE,
                suffix_charged: EventCount::NONE,
                completed_quanta: QuantumCount::NONE,
                adoption_after_quanta: None,
                render_started: false,
                transport: MixedCompiledTransport::Running,
                stop_pending: None,
                retiring_stop: false,
                stop_ended,
                stop_released: HeldNoteCount::NONE,
                stop_due: false,
                stops_charged: EventCount::NONE,
                in_force_restoration: EventCount::NONE,
                release_due: false,
                offer_slot: MixedOfferSlot::Empty,
                compiled_panic_ended: HeldNoteCount::NONE,
                compiled_panic_groups: EventCount::NONE,
                compiled_released_after_panic: EventCount::NONE,
                compiled_controls_after_panic: EventCount::NONE,
                loop_program: None,
                loop_cursor: None,
                loop_wraps: EventCount::NONE,
                loop_wrap_releases: HeldNoteCount::NONE,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                panic_cut: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                panic_current_service_at,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                parked_panic: None,
                #[cfg(all(test, feature = "simulated-ingress"))]
                test_ingress_inflight: vec![None; inflight_journal],
                #[cfg(all(test, feature = "simulated-ingress"))]
                test_ingress_inflight_len: 0,
                #[cfg(all(test, feature = "simulated-ingress"))]
                test_fail_after_ingress_at: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                test_ingress,
                #[cfg(feature = "mixed-transfer-rehearsal")]
                ingress_queue_peak: 0,
                #[cfg(feature = "mixed-transfer-rehearsal")]
                ingress_holds_peak: EventCount::NONE,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_owner,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                occurrences: MixedOccurrenceTable::prepare(
                    occurrence_width,
                    ingress_capacity,
                    bend_tables.audio,
                ),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_commands: OpaqueQueue(ingress_command_reader),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_results: OpaqueQueue(ingress_result_writer),
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_result_pending: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_halt,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                last_served_ingress_id: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                last_served_ingress_at: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                invalid_ingress_command: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                ingress_command_fault: None,
                #[cfg(feature = "mixed-transfer-rehearsal")]
                transfer_binding: None,
                #[cfg(feature = "mixed-transfer-rehearsal")]
                reset_seal: None,
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                bend_credit_return,
                #[cfg(test)]
                test_live: None,
                #[cfg(test)]
                test_live_spent: false,
                #[cfg(test)]
                test_live_share: profile.limits().events().shares().live_event_share(),
            },
        ))
    }

    /// The bound stream epoch carried by both retained owners.
    pub const fn epoch(&self) -> StreamEpoch {
        self.owner.control.epoch
    }

    /// The table shared by the compiled and live ranges.
    pub const fn table_id(&self) -> TableId {
        self.owner.control.table
    }

    /// How many tokenized events remain privately owned by this prepared stream.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// How many compiled note-ons remain unpaired after the bound initial list.
    pub fn outstanding_count(&self) -> usize {
        self.outstanding.len()
    }

    /// Reconstruct a bound compiled prefix without minting, publishing or exposing events.
    ///
    /// The result is a private ingredient for a later mixed activation. Its scoped zero
    /// gates are invalid without a producer-scoped release before the batch is rendered.
    pub fn prepare_history(
        &self,
        requested: SampleTime,
        position: PlanPosition,
    ) -> Result<MixedHistoryCandidate, MixedHistoryPrepareError> {
        self.owner.control.prepare_history(requested, position)
    }

    /// Classify only the bound suffix's note contracts against a consumed prefix candidate.
    ///
    /// A release paired to the saved prefix is counted and omitted entirely. It is never
    /// converted into the exclusive builder's whole-group `SetParameter` gate write.
    /// On refusal the private candidate is discarded and this owner stays unchanged.
    pub fn prepare_suffix(
        &self,
        candidate: MixedHistoryCandidate,
    ) -> Result<MixedSuffixCandidate, MixedSuffixPrepareError> {
        self.owner
            .control
            .prepare_suffix(candidate)
            .map_err(|refusal| refusal.0)
    }

    /// Place a private first-activation suffix and pair its note edges by token.
    ///
    /// Pairing runs against a scratch occupancy of the compiled span and refuses
    /// over-emission there; nothing is minted. The returned list orders requested-time
    /// restoration before suffix events at the destination. There is still no offer path.
    pub fn stamp_suffix(
        &self,
        suffix: MixedSuffixCandidate,
    ) -> Result<MixedStampedCandidate, MixedStampPrepareError> {
        self.owner
            .control
            .stamp_suffix(suffix, ActivationSequence::INITIAL)
            .map_err(|refusal| refusal.0)
    }
}

impl MixedStreamControl {
    /// Reconstruct a bound compiled prefix at `requested` and `position`, minting nothing.
    ///
    /// The one history builder for every candidate this control prepares, the first
    /// activation's and any later one's alike.
    fn prepare_history(
        &self,
        requested: SampleTime,
        position: PlanPosition,
    ) -> Result<MixedHistoryCandidate, MixedHistoryPrepareError> {
        if super::next_boundary(requested).is_none() {
            return Err(MixedHistoryPrepareError::BoundaryUnrepresentable { at: requested });
        }
        let plan = &self.plan;
        let stream = self.stream.events();
        let prefix_end = stream.partition_point(|event| event.position() < position);
        let capacity = self.compiled_span.len;
        let mut book = OpenNotes::new(capacity, crate::ir::StealingPolicy::None);
        let mut values = vec![None; plan.parameter_targets().len()];
        let gates = super::gate_rows(plan);
        for (event_index, event) in stream[..prefix_end].iter().copied().enumerate() {
            match event.payload() {
                CompiledPayload::NoteOn {
                    slot,
                    key,
                    velocity,
                } => {
                    match book.open(event.position().as_u64(), slot, key, ()) {
                        Opened::Admitted => {}
                        Opened::Refused | Opened::Stole(_) => {
                            return Err(MixedHistoryPrepareError::ProducerCapacity { event_index });
                        }
                    }
                    super::write_gate(&mut values, &gates, slot, ParameterValue::ONE);
                    super::write_magnitudes(&mut values, plan, slot, key, velocity);
                }
                CompiledPayload::NoteOff { slot, key } => {
                    if !matches!(
                        book.close(event.position().as_u64(), slot, key),
                        Closed::Paired(_)
                    ) {
                        return Err(MixedHistoryPrepareError::UnmatchedRelease { event_index });
                    }
                    super::write_gate(&mut values, &gates, slot, ParameterValue::ZERO);
                }
                // An occurrence's expression and bend end at the boundary. The initial
                // token pairing already checked their pairing with a note.
                CompiledPayload::Expression { .. } | CompiledPayload::Bend { .. } => {}
                CompiledPayload::Controller(_) | CompiledPayload::SetParameter { .. } => {
                    return Err(MixedHistoryPrepareError::UnexpectedWriter { event_index });
                }
            }
        }
        let open_at_destination = book.entries();
        let open_count = HeldNoteCount::measured(
            u32::try_from(open_at_destination.len())
                .map_err(|_| MixedHistoryPrepareError::OpenCountUnrepresentable)?,
        );
        for open in &open_at_destination {
            super::write_gate(&mut values, &gates, open.slot, ParameterValue::ZERO);
        }
        let groups = self.partition.restoration_groups();
        let mut restoration = Vec::with_capacity(groups.len());
        for group in groups {
            let slot = group.parameter();
            let target = plan
                .parameter_targets()
                .get(slot.index())
                .ok_or(MixedHistoryPrepareError::Partition)?;
            if slot.plan() != plan.id() {
                return Err(MixedHistoryPrepareError::Partition);
            }
            let value = if matches!(
                plan.prepared_for_node(target.node),
                Some(crate::node::kernels::PreparedNode::NoteSource)
            ) {
                target.base
            } else {
                values
                    .get(slot.index())
                    .copied()
                    .flatten()
                    .unwrap_or(target.base)
            };
            let scoped = if target.controller {
                ScopedParameterRestore::controller_for(*group, value, None)
            } else {
                ScopedParameterRestore::override_for(*group, value)
            };
            restoration.push(TimedEvent::new(
                EventEnvelope::new(self.epoch, requested, TimeSource::Compiled),
                EventPayload::ScopedRestore(scoped),
            ));
        }
        let restoration_count = EventCount::measured(
            u32::try_from(restoration.len())
                .map_err(|_| MixedHistoryPrepareError::RestorationCountUnrepresentable)?,
        );
        Ok(MixedHistoryCandidate {
            plan: plan.id(),
            epoch: self.epoch,
            table: self.table,
            requested,
            position,
            prefix_end,
            book,
            open_at_destination,
            open_count,
            restoration,
            restoration_count,
            off_thread: PhantomData,
        })
    }

    /// Classify the bound suffix against one consumed history candidate.
    ///
    /// Classification closes crossing releases in a copy of the history's book, so a
    /// refusal returns the candidate exactly as it was given.
    fn prepare_suffix(
        &self,
        candidate: MixedHistoryCandidate,
    ) -> Result<MixedSuffixCandidate, Box<(MixedSuffixPrepareError, MixedHistoryCandidate)>> {
        self.prepare_suffix_until(candidate, None)
    }

    /// [`Self::prepare_suffix`] over source positions before `end` only. A loop's lists
    /// stop there: its suffix after `end` can never play, so it is neither classified
    /// nor counted, and a note still open at `end` leaves its token open.
    fn prepare_suffix_until(
        &self,
        candidate: MixedHistoryCandidate,
        end: Option<PlanPosition>,
    ) -> Result<MixedSuffixCandidate, Box<(MixedSuffixPrepareError, MixedHistoryCandidate)>> {
        if candidate.plan != self.plan.id()
            || candidate.epoch != self.epoch
            || candidate.table != self.table
        {
            return Err(Box::new((
                MixedSuffixPrepareError::ForeignCandidate,
                candidate,
            )));
        }
        let stream = self.stream.events();
        if candidate.prefix_end
            != stream.partition_point(|event| event.position() < candidate.position)
        {
            return Err(Box::new((
                MixedSuffixPrepareError::PrefixMismatch,
                candidate,
            )));
        }
        let mut book = candidate.book.clone();
        let mut suffix = OpenNotes::new(self.compiled_span.len, crate::ir::StealingPolicy::None);
        let mut included = Vec::with_capacity(stream.len() - candidate.prefix_end);
        let mut omitted_releases = 0_usize;
        let mut omitted_expressions = 0_usize;
        for (event_index, event) in stream
            .iter()
            .copied()
            .enumerate()
            .skip(candidate.prefix_end)
            .take_while(|(_, event)| end.is_none_or(|end| event.position() < end))
        {
            let now = event.position().as_u64();
            let refused = match event.payload() {
                CompiledPayload::NoteOn { slot, key, .. } => {
                    if matches!(suffix.open(now, slot, key, ()), Opened::Admitted) {
                        None
                    } else {
                        Some(MixedSuffixPrepareError::ProducerCapacity { event_index })
                    }
                }
                CompiledPayload::NoteOff { slot, key } => {
                    if matches!(suffix.close(now, slot, key), Closed::Paired(_)) {
                        None
                    } else if matches!(book.close(now, slot, key), Closed::Paired(_)) {
                        omitted_releases += 1;
                        continue;
                    } else {
                        Some(MixedSuffixPrepareError::UnmatchedRelease { event_index })
                    }
                }
                CompiledPayload::Expression { slot, key, .. }
                | CompiledPayload::Bend { slot, key, .. } => {
                    if suffix.find(now, slot, key).is_some() {
                        None
                    } else if book.find(now, slot, key).is_some() {
                        omitted_expressions += 1;
                        continue;
                    } else {
                        Some(MixedSuffixPrepareError::UnmatchedExpression { event_index })
                    }
                }
                CompiledPayload::Controller(_) | CompiledPayload::SetParameter { .. } => {
                    Some(MixedSuffixPrepareError::UnexpectedWriter { event_index })
                }
            };
            if let Some(error) = refused {
                return Err(Box::new((error, candidate)));
            }
            included.push(event_index);
        }
        let counts = (
            u32::try_from(included.len()),
            u32::try_from(omitted_releases),
            u32::try_from(omitted_expressions),
        );
        let (Ok(included_count), Ok(omitted_releases), Ok(omitted_expressions)) = counts else {
            return Err(Box::new((
                MixedSuffixPrepareError::CountUnrepresentable,
                candidate,
            )));
        };
        let mut history = candidate;
        history.book = book;
        Ok(MixedSuffixCandidate {
            history,
            included,
            included_count: EventCount::measured(included_count),
            omitted_releases: EventCount::measured(omitted_releases),
            omitted_expressions: EventCount::measured(omitted_expressions),
        })
    }

    /// Place a classified suffix, pair it by token and seal it as superseding `supersedes`.
    ///
    /// Nothing is minted or freed: control holds no generation state. Every check runs
    /// before the restoration batch moves out of the suffix's history, so a refusal
    /// returns the suffix exactly as it was given.
    fn stamp_suffix(
        &self,
        suffix: MixedSuffixCandidate,
        supersedes: ActivationSequence,
    ) -> Result<MixedStampedCandidate, Box<(MixedStampPrepareError, MixedSuffixCandidate)>> {
        match self.stamp_events(&suffix) {
            Ok(stamped) => Ok(stamped.seal(suffix, supersedes)),
            Err(error) => Err(Box::new((error, suffix))),
        }
    }

    /// Every fallible part of [`Self::stamp_suffix`], reading the suffix only.
    fn stamp_events(
        &self,
        suffix: &MixedSuffixCandidate,
    ) -> Result<MixedStampedEvents, MixedStampPrepareError> {
        let history = &suffix.history;
        if history.plan != self.plan.id()
            || history.epoch != self.epoch
            || history.table != self.table
        {
            return Err(MixedStampPrepareError::ForeignCandidate);
        }
        let stream = self.stream.events();
        if suffix.included_count.as_usize() != Some(suffix.included.len()) {
            return Err(MixedStampPrepareError::CountUnrepresentable);
        }
        let anchor = StreamAnchor::new(history.requested, history.position);
        let mut placed = Vec::with_capacity(suffix.included.len());
        let mut previous = None;
        for &event_index in &suffix.included {
            let Some(event) = stream.get(event_index) else {
                return Err(MixedStampPrepareError::InvalidSelection { event_index });
            };
            if event_index < history.prefix_end || previous.is_some_and(|last| event_index <= last)
            {
                return Err(MixedStampPrepareError::InvalidSelection { event_index });
            }
            previous = Some(event_index);
            let time = match anchor.locate(event.position()) {
                Located::At(time) => time,
                Located::BeforeAnchor => {
                    return Err(MixedStampPrepareError::Schedule(
                        SchedulePrepareError::BeforeAnchor {
                            event_index,
                            position: event.position(),
                            anchor: anchor.position(),
                        },
                    ));
                }
                Located::Unrepresentable => {
                    return Err(MixedStampPrepareError::Schedule(
                        SchedulePrepareError::TimeUnrepresentable {
                            event_index,
                            position: event.position(),
                        },
                    ));
                }
            };
            placed.push(CompiledEvent::new(time, event.payload()));
        }
        if history.restoration_count.as_usize() != Some(history.restoration.len())
            || history.restoration.iter().any(|event| {
                event.envelope().time() != history.requested
                    || event.envelope().epoch() != history.epoch
                    || !matches!(event.payload(), EventPayload::ScopedRestore(_))
            })
        {
            return Err(MixedStampPrepareError::InvalidRestoration);
        }

        // The boundary release ends every old compiled occurrence on audio before
        // this list publishes, so pairing starts from an empty scratch occupancy.
        // No old reservation, copy or generation is consulted or changed.
        let tokenized = tokenize_compiled(
            &self.plan,
            self.compiled_producer,
            self.compiled_span,
            self.epoch,
            &placed,
        )
        .map_err(|error| match error {
            MixedTokenizeError::Schedule(error) => {
                MixedStampPrepareError::Schedule(super::rebase(error, &suffix.included))
            }
            MixedTokenizeError::UnexpectedWriter { event_index } => {
                MixedStampPrepareError::InvalidSelection {
                    event_index: suffix
                        .included
                        .get(event_index)
                        .copied()
                        .unwrap_or(event_index),
                }
            }
        })?;
        let outstanding_count = HeldNoteCount::measured(
            u32::try_from(tokenized.open.len())
                .map_err(|_| MixedStampPrepareError::CountUnrepresentable)?,
        );
        let mut events = Vec::with_capacity(history.restoration.len() + tokenized.events.len());
        for event in &history.restoration {
            let EventPayload::ScopedRestore(scoped) = event.payload() else {
                return Err(MixedStampPrepareError::InvalidRestoration);
            };
            events.push(MixedCompiledEvent::new(
                event.envelope(),
                MixedCompiledPayload::Restore(scoped),
            ));
        }
        events.extend(tokenized.events);
        check_mixed_event_order(&events)?;
        let event_count = EventCount::measured(
            u32::try_from(events.len())
                .map_err(|_| MixedStampPrepareError::CountUnrepresentable)?,
        );
        Ok(MixedStampedEvents {
            anchor,
            events,
            event_count,
            outstanding: tokenized.open,
            outstanding_count,
        })
    }
}

/// A placed, token-paired list that has passed every stamping check but not yet taken
/// its suffix's restoration batch.
struct MixedStampedEvents {
    anchor: StreamAnchor,
    events: Vec<MixedCompiledEvent>,
    event_count: EventCount,
    outstanding: Vec<MixedNoteToken>,
    outstanding_count: HeldNoteCount,
}

impl MixedStampedEvents {
    /// Infallibly move the restoration batch out of the suffix's history, whose
    /// restoration count drops to zero, and seal the candidate.
    fn seal(
        self,
        mut suffix: MixedSuffixCandidate,
        supersedes: ActivationSequence,
    ) -> MixedStampedCandidate {
        let restoration_count = suffix.history.restoration_count;
        // The batch is already copied into `events`; its source vector is dropped here,
        // off-thread.
        drop(std::mem::take(&mut suffix.history.restoration));
        suffix.history.restoration_count = EventCount::NONE;
        MixedStampedCandidate {
            suffix,
            anchor: self.anchor,
            supersedes,
            events: self.events,
            event_count: self.event_count,
            restoration_count,
            outstanding: self.outstanding,
            outstanding_count: self.outstanding_count,
        }
    }
}

fn check_mixed_event_order(events: &[MixedCompiledEvent]) -> Result<(), MixedStampPrepareError> {
    for (index, pair) in events.windows(2).enumerate() {
        if pair[1].envelope().time() < pair[0].envelope().time() {
            return Err(MixedStampPrepareError::EventOrder {
                event_index: index + 1,
            });
        }
    }
    Ok(())
}

/// Bind one closed private publication arbiter to the profile that will run it.
/// The candidate is still owned by the caller, so every refusal precedes boxing and
/// leaves the one arm attempt available. A future mixed host must admit other Session
/// contributors and live ingress in this same arbiter before lifting the offer refusal.
fn prepare_one_shot_arbiter(
    plan: &CompiledPlan,
    partition: &MixedInstancePartition,
    candidate: &MixedStampedCandidate,
    timing: MixedEffectiveTiming,
    profile: &HostProfile,
) -> Result<PublicationArbiter, MixedOneShotAdmissionError> {
    admit_one_shot_candidate(plan, partition, candidate, timing, profile)?;
    // Session and Compiled each fit their own share; their sum fits the quantum
    // cap by HostProfile construction. The renderer's storage preflight covers
    // that cap times its widest event, the compiled release and modulation.
    PublicationArbiter::prepare_one_quantum(profile).map_err(MixedOneShotAdmissionError::Arbiter)
}

/// Admit one stamped candidate at one effective boundary against `profile`: the armed
/// transition at arm, and every later one when control seals its offer. One check for
/// both is what leaves no admission gap between them.
fn admit_one_shot_candidate(
    plan: &CompiledPlan,
    partition: &MixedInstancePartition,
    candidate: &MixedStampedCandidate,
    timing: MixedEffectiveTiming,
    profile: &HostProfile,
) -> Result<(), MixedOneShotAdmissionError> {
    use MixedOneShotAdmissionError as Refused;

    let caps = profile.capabilities();
    let events = profile.limits().events();
    let shares = events.shares();
    let declared_holds = plan
        .note_producer_holds()
        .iter()
        .try_fold(EventCount::NONE, |sum, count| sum.checked_add(*count))
        .ok_or(Refused::ProfileMismatch)?;
    if plan.sample_rate() != caps.sample_rate()
        || plan.channel_layout() != caps.channel_layout()
        || plan.maximum_block_size() != caps.maximum_block_size()
        || plan.max_events_per_quantum() != events.max_events_per_quantum()
        || plan.compiled_event_share() != shares.compiled_event_share()
        || plan.forward_event_horizon() != events.forward_event_horizon()
        || declared_holds > shares.release_hold_capacity()
    {
        return Err(Refused::ProfileMismatch);
    }

    let restoration = candidate
        .restoration_count
        .as_usize()
        .ok_or(Refused::CandidateCount)?;
    if candidate.event_count.as_usize() != Some(candidate.events.len())
        || restoration > candidate.events.len()
        || restoration != partition.restoration_groups().len()
    {
        return Err(Refused::CandidateCount);
    }
    let fanout = plan.sample_positioned_fan_out();
    let mut last_time = None;
    let mut last_quantum = None;
    let mut compiled_count = EventCount::NONE;
    let compiled_share = shares.compiled_event_share();
    for (index, event) in candidate.events.iter().copied().enumerate() {
        let at = event
            .envelope()
            .time()
            .checked_add(timing.shift())
            .map_err(|_| Refused::CandidateShape { event_index: index })?;
        if event.envelope().source() != TimeSource::Compiled
            || last_time.is_some_and(|previous| at < previous)
        {
            return Err(Refused::CandidateShape { event_index: index });
        }
        last_time = Some(at);
        if index < restoration {
            let MixedCompiledPayload::Restore(scoped) = event.payload() else {
                return Err(Refused::CandidateShape { event_index: index });
            };
            let Some(group) = partition.restoration_groups().get(index) else {
                return Err(Refused::CandidateShape { event_index: index });
            };
            if at != timing.effective()
                || scoped.slot() != group.parameter()
                || scoped.instances() != group.instances()
            {
                return Err(Refused::CandidateShape { event_index: index });
            }
            let Some(target) = plan.parameter_targets().get(scoped.slot().index()) else {
                return Err(Refused::CandidateShape { event_index: index });
            };
            if matches!(target.rate, crate::plan::ControlRate::Sample) {
                if scoped.controller().is_some() {
                    return Err(Refused::CandidateShape { event_index: index });
                }
                let rows = VoiceCount::measured(scoped.instances().count());
                if rows > fanout {
                    return Err(Refused::TimedFanout {
                        rows,
                        admitted: fanout,
                    });
                }
            }
        } else {
            if matches!(event.payload(), MixedCompiledPayload::Restore(_))
                || at < timing.effective()
            {
                return Err(Refused::CandidateShape { event_index: index });
            }
            let quantum = at.quantum_index();
            if last_quantum != Some(quantum) {
                last_quantum = Some(quantum);
                compiled_count = EventCount::NONE;
            }
            compiled_count = compiled_count
                .checked_add(EventCount::measured(1))
                .ok_or(Refused::CandidateCount)?;
            if compiled_count > compiled_share {
                return Err(Refused::CompiledShare {
                    at,
                    needed: compiled_count,
                    available: compiled_share,
                });
            }
        }
    }

    let session_needed = candidate
        .restoration_count
        .checked_add(EventCount::measured(1))
        .ok_or(Refused::CandidateCount)?;
    let session_share = shares.session_event_share();
    if session_needed > session_share {
        return Err(Refused::SessionShare {
            needed: session_needed,
            available: session_share,
        });
    }
    Ok(())
}

impl MixedHistoryCandidate {
    /// The immutable plan this prefix belongs to.
    pub const fn plan_id(&self) -> PlanId {
        self.plan
    }

    /// The stream epoch that stamped its private restoration events.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// The identity table whose compiled range the prefix book describes.
    pub const fn table_id(&self) -> TableId {
        self.table
    }

    /// The requested engine time; a later scheduler must apply its displacement.
    pub const fn requested(&self) -> SampleTime {
        self.requested
    }

    /// The destination in the bound compiled stream.
    pub const fn position(&self) -> PlanPosition {
        self.position
    }

    /// The first bound source event at or after the destination.
    pub const fn prefix_end(&self) -> usize {
        self.prefix_end
    }

    /// Events in the private scoped restoration batch.
    pub const fn restoration_count(&self) -> EventCount {
        self.restoration_count
    }

    /// Notes open in the new destination timeline whose restoration gates must be zero.
    /// This is separate from the old renderer's sounding notes, which need boundary release.
    pub const fn open_at_destination_count(&self) -> HeldNoteCount {
        self.open_count
    }
}

impl MixedSuffixCandidate {
    /// Source events still eligible for later placement and stamping.
    pub const fn included_event_count(&self) -> EventCount {
        self.included_count
    }

    /// Crossing releases that must emit no parameter or trigger write at their old time.
    pub const fn omitted_release_count(&self) -> EventCount {
        self.omitted_releases
    }

    /// Bend or expression updates whose prefix occurrence ended at the boundary.
    pub const fn omitted_expression_count(&self) -> EventCount {
        self.omitted_expressions
    }

    /// The immutable boundary-open snapshot, retained separately from suffix pairing.
    pub const fn open_at_destination_count(&self) -> HeldNoteCount {
        self.history.open_count
    }
}

impl MixedStampedCandidate {
    /// Strip the non-sendable source history and box the audio payload off-thread.
    /// A sequence refusal gives the complete candidate back for off-thread disposal.
    #[allow(dead_code)] // Only the private arm constructs this capsule.
    pub(crate) fn into_audio_capsule(
        self,
    ) -> Result<Box<MixedAudioCandidate>, Box<(MixedCapsulePrepareError, Self)>> {
        self.into_audio_parts().map(Box::new)
    }

    /// [`Self::into_audio_capsule`] before boxing, for a later offer's own cell.
    fn into_audio_parts(
        self,
    ) -> Result<MixedAudioCandidate, Box<(MixedCapsulePrepareError, Self)>> {
        let Some(sequence) = self.supersedes.next() else {
            return Err(Box::new((
                MixedCapsulePrepareError::SequenceExhausted,
                self,
            )));
        };
        let Self {
            suffix,
            anchor,
            supersedes,
            events,
            event_count,
            restoration_count,
            outstanding,
            outstanding_count,
        } = self;
        let plan = suffix.history.plan;
        let epoch = suffix.history.epoch;
        let table = suffix.history.table;
        let omitted_releases = suffix.omitted_releases;
        let omitted_expressions = suffix.omitted_expressions;
        // `history` contains a PhantomData<Rc<()>> marker and source-selection
        // vectors. They and the open-token list are finally dropped here, before
        // the sendable capsule can reach audio.
        drop(suffix);
        drop(outstanding);
        Ok(MixedAudioCandidate {
            plan,
            epoch,
            table,
            anchor,
            retired_anchor: None,
            retired_next: None,
            supersedes,
            sequence,
            omitted_releases,
            omitted_expressions,
            events,
            event_count,
            restoration_count,
            outstanding_count,
        })
    }

    /// Check the uniform displacement for a possible adoption boundary.
    ///
    /// This is private preparation evidence, not capacity admission or an offer. The
    /// private arm selects and validates its actual boundary again.
    pub fn effective_timing(
        &self,
        effective: SampleTime,
    ) -> Result<MixedEffectiveTiming, MixedEffectiveTimeError> {
        let requested = self.anchor.time();
        if effective < requested {
            return Err(MixedEffectiveTimeError::BeforeRequested {
                requested,
                effective,
            });
        }
        if effective.quantum_offset() != QuantumOffset::ZERO {
            return Err(MixedEffectiveTimeError::NotQuantumBoundary { effective });
        }
        let displacement = effective.difference(requested).map_err(|_| {
            MixedEffectiveTimeError::DisplacementUnrepresentable {
                requested,
                effective,
            }
        })?;
        let frames = u64::try_from(displacement.as_i64()).map_err(|_| {
            MixedEffectiveTimeError::DisplacementUnrepresentable {
                requested,
                effective,
            }
        })?;
        let shift = FrameCount::new(frames);
        // Stamping checked nondecreasing times off-thread. The last event therefore
        // bounds every shifted event, so the later audio offer needs one check.
        if let Some((event_index, event)) = self.events.iter().enumerate().next_back() {
            let time = event.envelope().time();
            if time.checked_add(shift).is_err() {
                return Err(MixedEffectiveTimeError::EventTimeUnrepresentable {
                    event_index,
                    time,
                    shift,
                });
            }
        }
        Ok(MixedEffectiveTiming { effective, shift })
    }

    /// Inspect the private list at one checked effective boundary. The source list and
    /// its requested-time stamps stay intact, including on a timing refusal.
    #[allow(dead_code)] // This complete-list view is exercised by private tests.
    pub(crate) fn effective_events(
        &self,
        effective: SampleTime,
    ) -> Result<MixedEffectiveEvents<'_>, MixedEffectiveTimeError> {
        let timing = self.effective_timing(effective)?;
        Ok(MixedEffectiveEvents {
            events: &self.events,
            shift: timing.shift(),
        })
    }

    /// The requested-time anchor used to place the private suffix.
    pub const fn anchor(&self) -> StreamAnchor {
        self.anchor
    }

    /// The sequence this candidate would supersede: `INITIAL` for the first activation.
    /// Any later offer must compare it with the audio owner's in-force sequence.
    pub const fn supersedes(&self) -> ActivationSequence {
        self.supersedes
    }

    /// Scoped restoration and suffix events in their requested-time order.
    pub const fn event_count(&self) -> EventCount {
        self.event_count
    }

    /// Scoped restoration events at the front of the private ordered list.
    pub const fn restoration_count(&self) -> EventCount {
        self.restoration_count
    }

    /// Tokens the privately paired suffix leaves open at its end.
    pub const fn outstanding_count(&self) -> HeldNoteCount {
        self.outstanding_count
    }
}

impl MixedStreamAudio {
    /// The immutable local-instance partition shared with the control half.
    pub fn instance_partition(&self) -> &MixedInstancePartition {
        &self.partition
    }

    /// The renderer and live owner share this epoch with the compiled control.
    pub const fn epoch(&self) -> StreamEpoch {
        self.renderer.epoch()
    }

    /// The same table identity as the compiled control.
    pub const fn table_id(&self) -> TableId {
        self.live_minter.id()
    }

    /// The live producer this audio owner alone may mint for. It alone also mints for
    /// the compiled producer.
    pub const fn live_producer(&self) -> ProducerId {
        self.live_minter.producer()
    }

    /// The one live note target proven disjoint from compiled targets.
    pub const fn live_slot(&self) -> NoteSlot {
        self.note
    }
}

#[cfg(test)]
#[path = "mixed/history_tests.rs"]
mod history_tests;
#[cfg(all(test, feature = "simulated-ingress"))]
#[path = "mixed/ledger_tests.rs"]
mod ledger_tests;
#[cfg(all(test, feature = "simulated-ingress"))]
#[path = "mixed/loop_tests.rs"]
mod loop_tests;
#[cfg(test)]
#[path = "mixed/offer_tests.rs"]
mod offer_tests;
#[cfg(all(test, feature = "simulated-ingress"))]
#[path = "mixed/panic_tests.rs"]
mod panic_tests;
#[cfg(all(test, feature = "simulated-ingress"))]
#[path = "mixed/stage_tests.rs"]
mod stage_tests;
#[cfg(test)]
#[path = "mixed/stop_tests.rs"]
mod stop_tests;
#[cfg(test)]
#[path = "mixed/subsequent_tests.rs"]
mod subsequent_tests;
#[cfg(all(test, feature = "simulated-ingress"))]
#[path = "mixed/timeline_tests.rs"]
mod timeline_tests;
#[cfg(test)]
#[path = "mixed/transport_tests.rs"]
mod transport_tests;
