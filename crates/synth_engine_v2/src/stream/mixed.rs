//! A split-born mixed stream before ingress, activation or loop commands are enabled.
//!
//! This constructor consumes one bound plan and stream. Its two range minters share a
//! table identity but never share slots. It deliberately exposes no mixed render or
//! offer method while ADR-0075's release and restoration laws remain open.

use std::{marker::PhantomData, rc::Rc, sync::Arc};

use thiserror::Error;

use crate::{
    diagnostics::CompileError,
    host::mixed_targets::{MixedInstancePartition, MixedTargetAdmission},
    identity::{
        CompiledRangeMinter, IdentityTable, LiveRangeMinter, NoteIdentity, ProducerId, Resolution,
        TableId,
    },
    plan::{CompiledPlan, NoteSlot, PlanId},
    quantities::{EventCount, HeldNoteCount, ParameterValue},
    render::{EventEnvelope, EventPayload, PreparedRenderer, ScopedParameterRestore, TimedEvent},
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
#[allow(dead_code)] // No mixed audio-side schedule reader is connected yet.
pub(crate) struct MixedEffectiveEvents<'a> {
    events: &'a [TimedEvent],
    shift: FrameCount,
}

#[allow(dead_code)] // The private mixed activation reader is still under construction.
impl MixedEffectiveEvents<'_> {
    pub(crate) const fn len(&self) -> usize {
        self.events.len()
    }

    fn shifted(
        &self,
        event_index: usize,
        event: TimedEvent,
    ) -> Result<TimedEvent, MixedEffectiveTimeError> {
        let envelope = event.envelope();
        let time = envelope.time().checked_add(self.shift).map_err(|_| {
            MixedEffectiveTimeError::EventTimeUnrepresentable {
                event_index,
                time: envelope.time(),
                shift: self.shift,
            }
        })?;
        Ok(TimedEvent::new(
            EventEnvelope::new(envelope.epoch(), time, envelope.source()),
            event.payload(),
        ))
    }

    pub(crate) fn get(
        &self,
        event_index: usize,
    ) -> Result<Option<TimedEvent>, MixedEffectiveTimeError> {
        self.events
            .get(event_index)
            .copied()
            .map(|event| self.shifted(event_index, event))
            .transpose()
    }

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
/// leaving the history's restoration count at zero. The copied minter and
/// outstanding set cannot be promoted through this API;
/// boundary release, audio-side displacement and combined capacity remain unproved.
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
    /// Check the uniform displacement for a possible adoption boundary.
    ///
    /// This is private preparation evidence, not capacity admission or an offer. The
    /// eventual audio owner must select and validate its actual boundary again.
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
    #[allow(dead_code)] // The mixed audio-side schedule reader is not connected yet.
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
