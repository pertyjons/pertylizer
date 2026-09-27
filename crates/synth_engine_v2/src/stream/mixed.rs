//! A split-born mixed stream before ingress, activation or loop commands are enabled.
//!
//! This constructor consumes one bound plan and stream. Its two range minters share a
//! table identity but never share slots. It exposes no production mixed render or
//! offer while ADR-0075's combined-host laws remain open.

use std::{marker::PhantomData, rc::Rc, sync::Arc};

#[cfg(all(test, feature = "simulated-ingress"))]
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};

use thiserror::Error;

#[cfg(all(test, feature = "simulated-ingress"))]
use crate::ingress::{IngressCounters, IngressPrepareError, IngressRefused, PerformanceIngress};
use crate::{
    diagnostics::CompileError,
    host::mixed_targets::{MixedInstancePartition, MixedTargetAdmission},
    identity::{
        CompiledRangeMinter, IdentityTable, LiveRangeMinter, NoteIdentity, ProducerId, Resolution,
        TableId,
    },
    plan::{CompiledPlan, NoteSlot, PlanId},
    profile::HostProfile,
    publish::PublicationArbiter,
    quantities::{EventCount, HeldNoteCount, ParameterValue, QuantumCount, VoiceCount},
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
    transport::ActivationSequence,
};

mod hot;

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
    /// Placement or compiled note stamping refused.
    #[error(transparent)]
    Schedule(#[from] SchedulePrepareError),
    /// A stealing policy or steal artifact appeared despite mixed no-stealing admission.
    #[error("mixed preparation encountered a stealing policy or steal artifact")]
    UnexpectedSteal,
    /// A scoped restoration appeared in an initial schedule built from note-only input.
    #[error("mixed initial schedule contains a scoped restoration")]
    UnexpectedScopedRestore,
    /// Stamping left a different number of held indices and recorded obligations.
    #[error("compiled range holds {live} notes but the schedule records {outstanding}")]
    OutstandingMismatch { live: u32, outstanding: usize },
    /// A stamped note edge escaped the compiled producer's checked range.
    #[error("stamped event {event_index} names {identity} outside the compiled range")]
    IdentityOutsideRange {
        event_index: usize,
        identity: NoteIdentity,
    },
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

/// Why a private bound suffix could not be placed and stamped against a compiled-range copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MixedStampPrepareError {
    /// This selection came from another prepared owner.
    #[error("mixed suffix selection belongs to another prepared owner")]
    ForeignCandidate,
    /// Its private source indices no longer select a strict suffix in source order.
    #[error("mixed suffix source selection is invalid at index {event_index}")]
    InvalidSelection {
        /// The rejected source index.
        event_index: usize,
    },
    /// An old compiled reservation was not live in the copied minter.
    #[error("old compiled reservation {identity} resolved as {resolution:?}")]
    StaleReservation {
        /// The reservation that could not be released.
        identity: NoteIdentity,
        /// Its resolution in the copied compiled range.
        resolution: Resolution,
    },
    /// Releasing the old set left another compiled index live.
    #[error("{live} compiled notes remain after releasing the old schedule")]
    OldReservationsRemain {
        /// The unexpected live count.
        live: HeldNoteCount,
    },
    /// Placement or stamping refused with the original source index preserved.
    #[error(transparent)]
    Schedule(#[from] SchedulePrepareError),
    /// A no-stealing selection emitted an extra or missing stamped event.
    #[error("mixed suffix selected {selected} source events but stamped {stamped}")]
    EventCountMismatch {
        /// Selected source events.
        selected: EventCount,
        /// Actual stamped events.
        stamped: EventCount,
    },
    /// A stealing artifact appeared despite target binding's no-stealing rule.
    #[error("mixed suffix stamping encountered a stealing artifact")]
    UnexpectedSteal,
    /// The copied minter and the stamped outstanding set disagree.
    #[error("mixed suffix holds {live} notes but stamped {outstanding}")]
    OutstandingMismatch {
        /// Live notes in the private compiled minter.
        live: HeldNoteCount,
        /// Identities held by the private stamped schedule.
        outstanding: HeldNoteCount,
    },
    /// A stamped note edge escaped the compiled producer's range.
    #[error("mixed suffix source event {event_index} names {identity} outside the compiled range")]
    IdentityOutsideRange {
        /// Position in the owner's admitted source stream.
        event_index: usize,
        /// The foreign or out-of-range identity.
        identity: NoteIdentity,
    },
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
    #[cfg(all(test, feature = "simulated-ingress"))]
    #[error(transparent)]
    Ingress(#[from] IngressPrepareError),
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
    #[error("mixed one-shot candidate event {event_index} disappeared during a checked read")]
    MissingCandidateEvent { event_index: usize },
    #[error("mixed one-shot validated output window cannot be borrowed")]
    InternalOutputWindow,
    #[error("mixed one-shot restoration count cannot be indexed")]
    RestorationCount,
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
    pub(crate) release_charged: bool,
    pub(crate) released_compiled: HeldNoteCount,
    pub(crate) restoration_charged: EventCount,
    pub(crate) suffix_charged: EventCount,
    pub(crate) completed_quanta: QuantumCount,
    /// Whether the quantum that selected the compiled release completed rendering.
    pub(crate) boundary_quantum_completed: bool,
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
    events: &'a [TimedEvent],
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
    ) -> impl ExactSizeIterator<Item = Result<TimedEvent, MixedEffectiveTimeError>> + '_ {
        self.events
            .iter()
            .copied()
            .enumerate()
            .map(|(index, event)| self.shifted(index, event))
    }
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

/// One private requested-time mixed schedule for the first activation rehearsal.
///
/// Its ordered events contain scoped restoration before any equal-time suffix edge.
/// The restoration batch moves out of the retained suffix history into that list,
/// leaving the history's restoration count at zero. Promotion is possible only
/// after a private arm, successful boundary render and off-thread collection;
/// the combined host still has no offer path.
#[must_use]
pub struct MixedStampedCandidate {
    #[allow(dead_code)]
    suffix: MixedSuffixCandidate,
    anchor: StreamAnchor,
    supersedes: ActivationSequence,
    events: Vec<TimedEvent>,
    event_count: EventCount,
    restoration_count: EventCount,
    #[allow(dead_code)]
    outstanding: Vec<NoteIdentity>,
    outstanding_count: HeldNoteCount,
    #[allow(dead_code)]
    minter: CompiledRangeMinter,
}

/// Sendable, boxed custody for one private audio-side mixed transition.
///
/// Source-selection history stays off-thread. Collection drops the retired
/// event list off-thread; the box remains with audio until final teardown.
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
    events: Vec<TimedEvent>,
    /// The candidate's admitted list size, retained as historical metadata
    /// after the list moves to audio at the boundary.
    event_count: EventCount,
    restoration_count: EventCount,
    outstanding: Vec<NoteIdentity>,
    /// The candidate's admitted obligation count, retained after promotion
    /// moves the actual outstanding vector to control.
    outstanding_count: HeldNoteCount,
    /// Present until off-thread promotion. The callback never reads it.
    minter: Option<CompiledRangeMinter>,
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

/// An off-thread joined mixed stream, before its one bound initial stamp.
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
/// No stamped event or mutable half escapes this value.
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
    events: Vec<TimedEvent>,
    outstanding: Vec<NoteIdentity>,
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

/// Retained off-thread compiled identity authority while one private transition runs.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // The one-shot retirement path is the next slice.
pub(crate) struct MixedOneShotControl {
    control: MixedStreamControl,
    outstanding: Vec<NoteIdentity>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_commands: OpaqueQueue<HeapProd<MixedIngressCommand>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_results: OpaqueQueue<HeapCons<MixedIngressResult>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_capacity: MixedIngressQueueCount,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_outstanding: MixedIngressQueueCount,
    #[cfg(all(test, feature = "simulated-ingress"))]
    last_ingress_id: Option<MixedIngressCommandId>,
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

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
struct MixedIngressCommandId(u64);

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
struct MixedIngressOriginId(u64);

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[must_use]
struct MixedIngressQueueCount(usize);

#[cfg(all(test, feature = "simulated-ingress"))]
impl MixedIngressQueueCount {
    const NONE: Self = Self(0);

    fn capacity(value: usize) -> Self {
        assert!(value > 0, "prepared ingress capacity must be nonzero");
        Self(value)
    }

    const fn as_usize(self) -> usize {
        self.0
    }
}

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
enum MixedIngressRequest {
    Onset {
        origin: MixedIngressOriginId,
        at: SampleTime,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    },
    Release {
        origin: MixedIngressOriginId,
        at: SampleTime,
        identity: NoteIdentity,
    },
}

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug)]
struct MixedIngressCommand {
    id: MixedIngressCommandId,
    request: MixedIngressRequest,
}

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MixedIngressOutcome {
    Onset(Result<NoteIdentity, IngressRefused>),
    Release(Result<(), IngressRefused>),
}

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
struct MixedIngressResult {
    id: MixedIngressCommandId,
    request: MixedIngressRequest,
    outcome: MixedIngressOutcome,
}

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Debug, Error, PartialEq)]
enum MixedIngressSubmitError {
    #[error("mixed ingress command or result capacity is full")]
    Full(MixedIngressRequest),
    #[error("mixed ingress command identity is exhausted")]
    IdentityExhausted(MixedIngressRequest),
}

#[cfg(all(test, feature = "simulated-ingress"))]
impl MixedOneShotControl {
    fn submit_ingress(
        &mut self,
        request: MixedIngressRequest,
    ) -> Result<MixedIngressCommandId, MixedIngressSubmitError> {
        if self.ingress_outstanding >= self.ingress_capacity {
            return Err(MixedIngressSubmitError::Full(request));
        }
        let serial = self
            .last_ingress_id
            .map_or(Some(1), |id| id.0.checked_add(1))
            .ok_or(MixedIngressSubmitError::IdentityExhausted(request))?;
        let id = MixedIngressCommandId(serial);
        self.ingress_commands
            .0
            .try_push(MixedIngressCommand { id, request })
            .map_err(|command| MixedIngressSubmitError::Full(command.request))?;
        self.last_ingress_id = Some(id);
        self.ingress_outstanding.0 += 1;
        Ok(id)
    }

    fn collect_ingress_result(&mut self) -> Option<MixedIngressResult> {
        let result = self.ingress_results.0.try_pop()?;
        assert!(self.ingress_outstanding > MixedIngressQueueCount::NONE);
        self.ingress_outstanding.0 -= 1;
        Some(result)
    }
}

/// The sole compiled authority after a private adopted owner has rejoined.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)] // Private collection has no production host caller.
pub(crate) struct MixedResumedControl {
    control: MixedStreamControl,
    outstanding: Vec<NoteIdentity>,
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
    PromotionRefused,
    ResumedTeardown,
}

/// Final stopped-state classification. Boundary-ended and still-sounding notes
/// remain separate, as do uncharged and charged test-live reservations that
/// never entered the renderer registry.
#[derive(Debug)]
#[must_use]
#[allow(dead_code)]
pub(crate) struct MixedOneShotTeardown {
    end: MixedCollectionEnd,
    report: MixedOneShotRenderReport,
    sounding: MixedSoundingSnapshot,
    boundary_ended: Vec<crate::identity::EndedNote>,
    unpublished_live: Option<NoteIdentity>,
    charged_unregistered_live: Option<NoteIdentity>,
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
}

#[derive(Debug)]
#[must_use]
#[allow(dead_code)]
pub(crate) enum MixedCollection {
    Resumed {
        control: MixedResumedControl,
        audio: Box<MixedOneShotAudio>,
        retired: MixedRetiredList,
    },
    Ended(MixedOneShotTeardown),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[allow(dead_code)]
pub(crate) enum MixedCollectionError {
    #[error("mixed control and audio halves were crossed")]
    CrossedPair,
    #[error(transparent)]
    Snapshot(#[from] MixedSoundingSnapshotError),
    #[error("mixed boundary-ended storage lost its recorded prefix")]
    EndedPrefix,
    #[error("mixed audio half has not completed compiled authority promotion")]
    UnpromotedAudio,
    #[cfg(all(test, feature = "simulated-ingress"))]
    #[error("private mixed ingress journal lost the registered queue")]
    IngressJournal,
    #[cfg(all(test, feature = "simulated-ingress"))]
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
    audio: MixedStreamAudio,
    arbiter: PublicationArbiter,
    events: Vec<TimedEvent>,
    next: usize,
    capsule: Box<MixedAudioCandidate>,
    timing: MixedEffectiveTiming,
    effective_anchor: StreamAnchor,
    late_at_arm: bool,
    in_force: ActivationSequence,
    adopted: bool,
    fault: Option<MixedOneShotRenderError>,
    release_charged: bool,
    released_compiled: HeldNoteCount,
    restoration_charged: EventCount,
    suffix_charged: EventCount,
    completed_quanta: QuantumCount,
    adoption_after_quanta: Option<QuantumCount>,
    render_started: bool,
    #[cfg(all(test, feature = "simulated-ingress"))]
    test_ingress: PerformanceIngress,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_commands: OpaqueQueue<HeapCons<MixedIngressCommand>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_results: OpaqueQueue<HeapProd<MixedIngressResult>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_result_pending: Option<MixedIngressResult>,
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

#[cfg(all(test, feature = "simulated-ingress"))]
impl MixedOneShotAudio {
    /// Serve only the preallocated command prefix; each result retains its request.
    fn service_test_ingress_queue(&mut self) {
        if let Some(pending) = self.ingress_result_pending.take()
            && let Err(pending) = self.ingress_results.0.try_push(pending)
        {
            self.ingress_result_pending = Some(pending);
            return;
        }
        while let Some(command) = self.ingress_commands.0.try_pop() {
            let outcome = match command.request {
                MixedIngressRequest::Onset {
                    at, key, velocity, ..
                } => MixedIngressOutcome::Onset(self.offer_test_note_on(at, key, velocity)),
                MixedIngressRequest::Release { at, identity, .. } => {
                    MixedIngressOutcome::Release(self.offer_test_note_off(at, identity))
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

    /// Private ingress acceptance; the returned identity is the release token.
    fn offer_test_note_on(
        &mut self,
        at: SampleTime,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Result<NoteIdentity, IngressRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(IngressRefused::TerminalOwner);
        }
        if self.test_live.is_some() {
            return Err(IngressRefused::MixedTestConflict);
        }
        self.test_ingress.adopt(self.audio.renderer.epoch())?;
        self.test_ingress.offer_mixed_note_on(
            &mut self.audio.minter,
            at,
            self.audio.note,
            key,
            velocity,
        )
    }

    /// The exact accepted occurrence redeems its release hold.
    fn offer_test_note_off(
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
            .offer_mixed_note_off(&mut self.audio.minter, at, identity)
    }
}

/// Off-thread compiled-range custody for one bound mixed plan and stream.
/// It exposes no independent schedule or activation builder.
#[derive(Debug)]
#[must_use]
pub struct MixedStreamControl {
    epoch: StreamEpoch,
    anchor: StreamAnchor,
    plan: Arc<CompiledPlan>,
    stream: AdmittedCompiledStream,
    minter: CompiledRangeMinter,
    compiled_slots: Vec<NoteSlot>,
    partition: Arc<MixedInstancePartition>,
}

/// Audio-side live-range custody and renderer for a split-born mixed stream.
/// It exposes no mixed rendering until release and parameter ownership are proved.
#[derive(Debug)]
#[must_use]
pub struct MixedStreamAudio {
    renderer: PreparedRenderer,
    minter: LiveRangeMinter,
    note: NoteSlot,
    partition: Arc<MixedInstancePartition>,
    compiled_ended: Vec<Option<crate::identity::EndedNote>>,
}

fn mixed_collection_pair(control: &MixedStreamControl, audio: &MixedOneShotAudio) -> bool {
    let rendered = &audio.audio;
    Arc::ptr_eq(&control.partition, &rendered.partition)
        && rendered.renderer.plan().id() == control.plan.id()
        && rendered.renderer.epoch() == control.epoch
        && rendered.renderer.table_id() == control.minter.id()
        && rendered.minter.id() == control.minter.id()
        && control.partition.compiled_producer() == control.minter.producer()
        && control.partition.spans().0 == control.minter.span()
        && control.partition.live_producer() == rendered.minter.producer()
        && control.partition.spans().1 == rendered.minter.span()
}

struct MixedTeardownParts {
    sounding: MixedSoundingSnapshot,
    boundary_ended: Vec<crate::identity::EndedNote>,
    unpublished_live: Option<NoteIdentity>,
    charged_unregistered_live: Option<NoteIdentity>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress: MixedIngressTeardown,
}

fn mixed_teardown_parts(
    audio: &MixedOneShotAudio,
) -> Result<MixedTeardownParts, MixedCollectionError> {
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
        let mut queue = vec![None; audio.test_ingress.queue_capacity_for_test()];
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
            minted_live: audio.audio.minter.live_identities_for_test(),
            holds_outstanding: audio.test_ingress.holds_outstanding(),
            counters: audio.test_ingress.counters(),
        }
    };
    Ok(MixedTeardownParts {
        sounding,
        boundary_ended,
        unpublished_live,
        charged_unregistered_live,
        #[cfg(all(test, feature = "simulated-ingress"))]
        ingress,
    })
}

fn mixed_finish_teardown(
    audio: MixedOneShotAudio,
    end: MixedCollectionEnd,
    parts: MixedTeardownParts,
) -> MixedOneShotTeardown {
    let report = audio.report();
    // Both event lists, the capsule and renderer are finally dropped here, off-thread.
    drop(audio);
    MixedOneShotTeardown {
        end,
        report,
        sounding: parts.sounding,
        boundary_ended: parts.boundary_ended,
        unpublished_live: parts.unpublished_live,
        charged_unregistered_live: parts.charged_unregistered_live,
        #[cfg(all(test, feature = "simulated-ingress"))]
        ingress: parts.ingress,
    }
}

#[allow(dead_code)] // Private off-thread collection has no production host caller.
impl MixedOneShotControl {
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
        #[cfg(all(test, feature = "simulated-ingress"))]
        if self.ingress_outstanding > MixedIngressQueueCount::NONE {
            return Err(Box::new(MixedCollectionRefusal {
                reason: MixedCollectionError::IngressResultPending,
                control: self,
                audio,
            }));
        }
        if !audio.adopted
            || audio.fault.is_some()
            || audio.audio.renderer.diagnostics().needs_reprepare()
        {
            let end =
                if audio.fault.is_some() || audio.audio.renderer.diagnostics().needs_reprepare() {
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
            drop(self);
            return Ok(MixedCollection::Ended(mixed_finish_teardown(
                audio, end, parts,
            )));
        }
        let capsule = &audio.capsule;
        let minter = capsule.minter.as_ref();
        let promotable = minter.is_some_and(|minter| {
            minter.id() == self.control.minter.id()
                && minter.producer() == self.control.minter.producer()
                && minter.span() == self.control.minter.span()
                && minter.live() == capsule.outstanding_count.get()
                && capsule.outstanding_count.as_usize() == Some(capsule.outstanding.len())
                && capsule
                    .outstanding
                    .iter()
                    .all(|identity| minter.resolve(*identity) == Resolution::Live)
        }) && capsule.plan == self.control.plan.id()
            && capsule.epoch == self.control.epoch
            && capsule.table == self.control.minter.id()
            && audio.in_force == capsule.sequence
            && capsule.supersedes == ActivationSequence::INITIAL
            && capsule.retired_anchor.is_some()
            && capsule
                .retired_next
                .is_some_and(|next| next <= capsule.events.len())
            && audio.audio.renderer.mixed_anchor() == audio.effective_anchor;
        if !promotable {
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
            return Ok(MixedCollection::Ended(mixed_finish_teardown(
                audio,
                MixedCollectionEnd::PromotionRefused,
                parts,
            )));
        }
        // The checks above prove these fields exist. Keep a defensive terminal
        // branch so a future change cannot guess an old anchor or cursor.
        let (Some(retired_anchor), Some(retired_next)) =
            (audio.capsule.retired_anchor, audio.capsule.retired_next)
        else {
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
            return Ok(MixedCollection::Ended(mixed_finish_teardown(
                audio,
                MixedCollectionEnd::PromotionRefused,
                parts,
            )));
        };
        let Some(promoted_minter) = audio.capsule.minter.take() else {
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
            return Ok(MixedCollection::Ended(mixed_finish_teardown(
                audio,
                MixedCollectionEnd::PromotionRefused,
                parts,
            )));
        };
        let old_minter = std::mem::replace(&mut self.control.minter, promoted_minter);
        drop(old_minter);
        self.control.anchor = audio.effective_anchor;
        let outstanding = std::mem::take(&mut audio.capsule.outstanding);
        let retired = MixedRetiredList {
            anchor: retired_anchor,
            unconsumed_from: retired_next,
            event_count: audio.capsule.events.len(),
        };
        audio.capsule.retired_next = None;
        drop(std::mem::take(&mut audio.capsule.events));
        drop(self.outstanding);
        Ok(MixedCollection::Resumed {
            control: MixedResumedControl {
                control: self.control,
                outstanding,
                sequence: audio.in_force,
                off_thread: PhantomData,
            },
            audio: Box::new(audio),
            retired,
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
        if audio.capsule.minter.is_some() {
            return Err(Box::new(MixedCollectionRefusal {
                reason: MixedCollectionError::UnpromotedAudio,
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
        let mut renderer = match PreparedRenderer::prepare(
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
        Ok((
            Self {
                epoch,
                anchor,
                plan: parts.plan,
                stream: parts.stream,
                minter: compiled,
                compiled_slots: parts.compiled_slots,
                partition: parts.partition,
            },
            MixedStreamAudio {
                renderer,
                minter: live,
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

    /// The only compiled stream this owner may later stamp.
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
        self.minter.id()
    }

    /// The compiled producer this control alone may mint for.
    pub const fn compiled_producer(&self) -> ProducerId {
        self.minter.producer()
    }

    /// The note slots reached by the owned compiled stream.
    pub fn compiled_slots(&self) -> &[NoteSlot] {
        &self.compiled_slots
    }

    /// The immutable local-instance partition shared with the audio half.
    pub fn instance_partition(&self) -> &MixedInstancePartition {
        &self.partition
    }

    /// Check the bound stream against this control's compiled range off-thread.
    /// The copied identities and events are discarded; no schedule or reservation is published.
    pub(crate) fn check_bound_stamp(&self) -> Result<(), SchedulePrepareError> {
        let placed = crate::schedule::place_admitted(&self.stream, self.anchor)?;
        let mut minter = self.minter.working_copy();
        crate::schedule::stamp_all(&mut minter, &self.plan, self.epoch, &placed)?;
        Ok(())
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

    /// Check the fixed compiled stream without creating any schedule or reservation.
    pub fn check_bound_stamp(&self) -> Result<(), SchedulePrepareError> {
        self.control.check_bound_stamp()
    }

    /// Seal the bound initial schedule exactly once, off-thread.
    /// A refusal returns this owner with its compiled minter unchanged.
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
        mut self,
    ) -> Result<MixedJoinedPrepared, Box<(MixedInitialPrepareError, Self)>> {
        if self.control.plan.stealing() != crate::ir::StealingPolicy::None {
            return Err(Box::new((MixedInitialPrepareError::UnexpectedSteal, self)));
        }
        let placed =
            match crate::schedule::place_admitted(&self.control.stream, self.control.anchor) {
                Ok(placed) => placed,
                Err(error) => return Err(Box::new((error.into(), self))),
            };
        let mut minter = self.control.minter.working_copy();
        let stamped = match crate::schedule::stamp_all(
            &mut minter,
            &self.control.plan,
            self.control.epoch,
            &placed,
        ) {
            Ok(stamped) => stamped,
            Err(error) => return Err(Box::new((error.into(), self))),
        };
        if stamped.released_after_steal != 0
            || stamped.expressions_after_steal != 0
            || stamped.events.iter().any(|event| {
                matches!(
                    event.payload(),
                    EventPayload::Fade { .. } | EventPayload::Reset { .. }
                )
            })
        {
            return Err(Box::new((MixedInitialPrepareError::UnexpectedSteal, self)));
        }
        if usize::try_from(minter.live()).ok() != Some(stamped.outstanding.len()) {
            return Err(Box::new((
                MixedInitialPrepareError::OutstandingMismatch {
                    live: minter.live(),
                    outstanding: stamped.outstanding.len(),
                },
                self,
            )));
        }
        let span = minter.span();
        for (event_index, event) in stamped.events.iter().enumerate() {
            let identity = match event.payload() {
                EventPayload::ScopedRestore(_) => {
                    return Err(Box::new((
                        MixedInitialPrepareError::UnexpectedScopedRestore,
                        self,
                    )));
                }
                EventPayload::Note { identity, .. }
                | EventPayload::Expression { identity, .. }
                | EventPayload::Bend { identity, .. }
                | EventPayload::Fade { identity, .. }
                | EventPayload::Reset { identity } => Some(identity),
                EventPayload::ReleaseGroup(_)
                | EventPayload::Controller(_)
                | EventPayload::RestoreController(_)
                | EventPayload::SetParameter { .. } => None,
            };
            if let Some(identity) = identity
                && (identity.table() != minter.id() || !span.contains(identity.index()))
            {
                return Err(Box::new((
                    MixedInitialPrepareError::IdentityOutsideRange {
                        event_index,
                        identity,
                    },
                    self,
                )));
            }
        }
        self.control.minter = minter;
        Ok(MixedJoinedPrepared {
            owner: self,
            events: stamped.events,
            outstanding: stamped.outstanding,
        })
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
        let control = &self.owner.control;
        let audio = &self.owner.audio;
        let history = &candidate.suffix.history;
        if history.plan != control.plan.id()
            || history.epoch != control.epoch
            || history.table != control.minter.id()
        {
            return Err(Box::new(MixedOneShotArmRefusal {
                reason: MixedOneShotArmError::ForeignCandidate,
                owner: self,
                candidate,
            }));
        }
        if audio.renderer.plan().id() != control.plan.id()
            || audio.renderer.epoch() != control.epoch
            || audio.renderer.table_id() != control.minter.id()
            || audio.table_id() != control.minter.id()
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
        #[cfg(all(test, feature = "simulated-ingress"))]
        let test_ingress = match PerformanceIngress::prepare_mixed_test(
            profile,
            &control.plan,
            &audio.minter,
            &audio.renderer,
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
        #[cfg(all(test, feature = "simulated-ingress"))]
        let ingress_capacity =
            MixedIngressQueueCount::capacity(test_ingress.queue_capacity_for_test());
        #[cfg(all(test, feature = "simulated-ingress"))]
        let (ingress_command_writer, ingress_command_reader) =
            HeapRb::<MixedIngressCommand>::new(ingress_capacity.as_usize()).split();
        #[cfg(all(test, feature = "simulated-ingress"))]
        let (ingress_result_writer, ingress_result_reader) =
            HeapRb::<MixedIngressResult>::new(ingress_capacity.as_usize()).split();
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
        Ok((
            MixedOneShotControl {
                control,
                outstanding,
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_commands: OpaqueQueue(ingress_command_writer),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_results: OpaqueQueue(ingress_result_reader),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_capacity,
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_outstanding: MixedIngressQueueCount::NONE,
                #[cfg(all(test, feature = "simulated-ingress"))]
                last_ingress_id: None,
                off_thread: PhantomData,
            },
            MixedOneShotAudio {
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
                release_charged: false,
                released_compiled: HeldNoteCount::NONE,
                restoration_charged: EventCount::NONE,
                suffix_charged: EventCount::NONE,
                completed_quanta: QuantumCount::NONE,
                adoption_after_quanta: None,
                render_started: false,
                #[cfg(all(test, feature = "simulated-ingress"))]
                test_ingress_inflight: vec![None; test_ingress.queue_capacity_for_test()],
                #[cfg(all(test, feature = "simulated-ingress"))]
                test_ingress_inflight_len: 0,
                #[cfg(all(test, feature = "simulated-ingress"))]
                test_fail_after_ingress_at: None,
                #[cfg(all(test, feature = "simulated-ingress"))]
                test_ingress,
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_commands: OpaqueQueue(ingress_command_reader),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_results: OpaqueQueue(ingress_result_writer),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_result_pending: None,
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
        self.owner.control.minter.id()
    }

    /// How many stamped events remain privately owned by this prepared stream.
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
        if super::next_boundary(requested).is_none() {
            return Err(MixedHistoryPrepareError::BoundaryUnrepresentable { at: requested });
        }
        let control = &self.owner.control;
        let plan = &control.plan;
        let stream = control.stream.events();
        let prefix_end = stream.partition_point(|event| event.position() < position);
        let span = control.minter.span().indices();
        let capacity = span.end - span.start;
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
                // full stamp already checked their pairing with a note.
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
        let groups = control.partition.restoration_groups();
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
                EventEnvelope::new(control.epoch, requested, TimeSource::Compiled),
                EventPayload::ScopedRestore(scoped),
            ));
        }
        let restoration_count = EventCount::measured(
            u32::try_from(restoration.len())
                .map_err(|_| MixedHistoryPrepareError::RestorationCountUnrepresentable)?,
        );
        Ok(MixedHistoryCandidate {
            plan: plan.id(),
            epoch: control.epoch,
            table: control.minter.id(),
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

    /// Classify only the bound suffix's note contracts against a consumed prefix candidate.
    ///
    /// A release paired to the saved prefix is counted and omitted entirely. It is never
    /// converted into the exclusive builder's whole-group `SetParameter` gate write.
    /// On refusal the private candidate is discarded and this owner stays unchanged.
    pub fn prepare_suffix(
        &self,
        mut candidate: MixedHistoryCandidate,
    ) -> Result<MixedSuffixCandidate, MixedSuffixPrepareError> {
        let control = &self.owner.control;
        if candidate.plan != control.plan.id()
            || candidate.epoch != control.epoch
            || candidate.table != control.minter.id()
        {
            return Err(MixedSuffixPrepareError::ForeignCandidate);
        }
        let stream = control.stream.events();
        if candidate.prefix_end
            != stream.partition_point(|event| event.position() < candidate.position)
        {
            return Err(MixedSuffixPrepareError::PrefixMismatch);
        }
        let span = control.minter.span().indices();
        let mut suffix = OpenNotes::new(span.end - span.start, crate::ir::StealingPolicy::None);
        let mut included = Vec::with_capacity(stream.len() - candidate.prefix_end);
        let mut omitted_releases = 0_usize;
        let mut omitted_expressions = 0_usize;
        for (event_index, event) in stream
            .iter()
            .copied()
            .enumerate()
            .skip(candidate.prefix_end)
        {
            let now = event.position().as_u64();
            match event.payload() {
                CompiledPayload::NoteOn { slot, key, .. } => {
                    if !matches!(suffix.open(now, slot, key, ()), Opened::Admitted) {
                        return Err(MixedSuffixPrepareError::ProducerCapacity { event_index });
                    }
                }
                CompiledPayload::NoteOff { slot, key } => {
                    if !matches!(suffix.close(now, slot, key), Closed::Paired(_)) {
                        if matches!(candidate.book.close(now, slot, key), Closed::Paired(_)) {
                            omitted_releases += 1;
                            continue;
                        }
                        return Err(MixedSuffixPrepareError::UnmatchedRelease { event_index });
                    }
                }
                CompiledPayload::Expression { slot, key, .. }
                | CompiledPayload::Bend { slot, key, .. } => {
                    if suffix.find(now, slot, key).is_none() {
                        if candidate.book.find(now, slot, key).is_some() {
                            omitted_expressions += 1;
                            continue;
                        }
                        return Err(MixedSuffixPrepareError::UnmatchedExpression { event_index });
                    }
                }
                CompiledPayload::Controller(_) | CompiledPayload::SetParameter { .. } => {
                    return Err(MixedSuffixPrepareError::UnexpectedWriter { event_index });
                }
            }
            included.push(event_index);
        }
        let counts = (
            u32::try_from(included.len()),
            u32::try_from(omitted_releases),
            u32::try_from(omitted_expressions),
        );
        let (Ok(included_count), Ok(omitted_releases), Ok(omitted_expressions)) = counts else {
            return Err(MixedSuffixPrepareError::CountUnrepresentable);
        };
        Ok(MixedSuffixCandidate {
            history: candidate,
            included,
            included_count: EventCount::measured(included_count),
            omitted_releases: EventCount::measured(omitted_releases),
            omitted_expressions: EventCount::measured(omitted_expressions),
        })
    }

    /// Place and stamp a private first-activation suffix against only a compiled-range copy.
    ///
    /// The old initial schedule's reservations are released in that copy before stamping.
    /// The returned list orders requested-time restoration before suffix events at the
    /// destination. There is still no producer-scoped boundary release or offer path.
    pub fn stamp_suffix(
        &self,
        mut suffix: MixedSuffixCandidate,
    ) -> Result<MixedStampedCandidate, MixedStampPrepareError> {
        let control = &self.owner.control;
        let history = &suffix.history;
        if history.plan != control.plan.id()
            || history.epoch != control.epoch
            || history.table != control.minter.id()
        {
            return Err(MixedStampPrepareError::ForeignCandidate);
        }
        let stream = control.stream.events();
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
        if suffix.history.restoration_count.as_usize() != Some(suffix.history.restoration.len())
            || suffix.history.restoration.iter().any(|event| {
                event.envelope().time() != history.requested
                    || event.envelope().epoch() != history.epoch
                    || !matches!(event.payload(), EventPayload::ScopedRestore(_))
            })
        {
            return Err(MixedStampPrepareError::InvalidRestoration);
        }

        let mut minter = control.minter.working_copy();
        for &identity in &self.outstanding {
            let resolution = minter.release(identity);
            if resolution != Resolution::Live {
                return Err(MixedStampPrepareError::StaleReservation {
                    identity,
                    resolution,
                });
            }
        }
        if minter.live() != 0 {
            return Err(MixedStampPrepareError::OldReservationsRemain {
                live: HeldNoteCount::measured(minter.live()),
            });
        }
        let stamped =
            crate::schedule::stamp_all(&mut minter, &control.plan, control.epoch, &placed)
                .map_err(|error| {
                    MixedStampPrepareError::Schedule(super::rebase(error, &suffix.included))
                })?;
        if stamped.released_after_steal != 0
            || stamped.expressions_after_steal != 0
            || stamped.events.iter().any(|event| {
                matches!(
                    event.payload(),
                    EventPayload::Fade { .. } | EventPayload::Reset { .. }
                )
            })
        {
            return Err(MixedStampPrepareError::UnexpectedSteal);
        }
        let stamped_count = EventCount::measured(
            u32::try_from(stamped.events.len())
                .map_err(|_| MixedStampPrepareError::CountUnrepresentable)?,
        );
        if stamped_count != suffix.included_count {
            return Err(MixedStampPrepareError::EventCountMismatch {
                selected: suffix.included_count,
                stamped: stamped_count,
            });
        }
        let outstanding_count = HeldNoteCount::measured(
            u32::try_from(stamped.outstanding.len())
                .map_err(|_| MixedStampPrepareError::CountUnrepresentable)?,
        );
        if minter.live() != outstanding_count.get() {
            return Err(MixedStampPrepareError::OutstandingMismatch {
                live: HeldNoteCount::measured(minter.live()),
                outstanding: outstanding_count,
            });
        }
        let span = minter.span();
        for (stamped_index, event) in stamped.events.iter().enumerate() {
            let identity = match event.payload() {
                EventPayload::Note { identity, .. }
                | EventPayload::Expression { identity, .. }
                | EventPayload::Bend { identity, .. }
                | EventPayload::Fade { identity, .. }
                | EventPayload::Reset { identity } => Some(identity),
                EventPayload::ScopedRestore(_)
                | EventPayload::ReleaseGroup(_)
                | EventPayload::Controller(_)
                | EventPayload::RestoreController(_)
                | EventPayload::SetParameter { .. } => None,
            };
            if let Some(identity) = identity
                && (identity.table() != minter.id() || !span.contains(identity.index()))
            {
                let event_index = suffix.included.get(stamped_index).copied().ok_or(
                    MixedStampPrepareError::InvalidSelection {
                        event_index: stamped_index,
                    },
                )?;
                return Err(MixedStampPrepareError::IdentityOutsideRange {
                    event_index,
                    identity,
                });
            }
        }
        let restoration_count = suffix.history.restoration_count;
        let mut events = std::mem::take(&mut suffix.history.restoration);
        suffix.history.restoration_count = EventCount::NONE;
        events.extend(stamped.events);
        check_mixed_event_order(&events)?;
        let event_count = EventCount::measured(
            u32::try_from(events.len())
                .map_err(|_| MixedStampPrepareError::CountUnrepresentable)?,
        );
        Ok(MixedStampedCandidate {
            suffix,
            anchor,
            supersedes: ActivationSequence::INITIAL,
            events,
            event_count,
            restoration_count,
            outstanding: stamped.outstanding,
            outstanding_count,
            minter,
        })
    }
}

fn check_mixed_event_order(events: &[TimedEvent]) -> Result<(), MixedStampPrepareError> {
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
            let EventPayload::ScopedRestore(scoped) = event.payload() else {
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
            if matches!(event.payload(), EventPayload::ScopedRestore(_)) || at < timing.effective()
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

    // Session and Compiled each fit their own share; their sum fits the quantum
    // cap by HostProfile construction. The renderer's storage preflight covers
    // that cap times its widest event, the compiled release and modulation.
    PublicationArbiter::prepare_one_quantum(profile).map_err(Refused::Arbiter)
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
            minter,
        } = self;
        let plan = suffix.history.plan;
        let epoch = suffix.history.epoch;
        let table = suffix.history.table;
        let omitted_releases = suffix.omitted_releases;
        let omitted_expressions = suffix.omitted_expressions;
        // `history` contains a PhantomData<Rc<()>> marker and source-selection
        // vectors. The vectors are finally dropped here, before the sendable
        // capsule can reach audio.
        drop(suffix);
        Ok(Box::new(MixedAudioCandidate {
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
            outstanding,
            outstanding_count,
            minter: Some(minter),
        }))
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

    /// The initial sequence this first-activation rehearsal would supersede.
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

    /// Notes the privately stamped suffix leaves reserved in its compiled range.
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
        self.minter.id()
    }

    /// The live producer this audio owner alone may mint for.
    pub const fn live_producer(&self) -> ProducerId {
        self.minter.producer()
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
#[path = "mixed/stage_tests.rs"]
mod stage_tests;
