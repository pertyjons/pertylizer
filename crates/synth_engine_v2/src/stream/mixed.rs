//! A split-born mixed stream before ingress, activation or loop commands are enabled.
//!
//! This constructor consumes one bound plan and stream. Its two range minters share a
//! table identity but never share slots, and both belong to the audio half for the
//! owner's whole life. The off-thread control holds no generation state: it pairs
//! compiled note edges by bounded tokens, and the audio half mints each compiled
//! occurrence when its note-on publishes. It exposes no production mixed render or
//! offer while ADR-0075's combined-host laws remain open.

use std::{marker::PhantomData, rc::Rc, sync::Arc};

#[cfg(all(test, feature = "simulated-ingress"))]
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
use crate::ingress::{IngressCounters, IngressPrepareError, IngressRefused, PerformanceIngress};
use crate::{
    diagnostics::CompileError,
    host::mixed_targets::{MixedInstancePartition, MixedTargetAdmission},
    identity::{
        CompiledRangeMinter, IdentityError, IdentityTable, LiveRangeMinter, NoteIdentity,
        ProducerId, Range, Resolution, TableId,
    },
    plan::{CompiledPlan, NoteSlot, PlanId},
    profile::HostProfile,
    publish::PublicationArbiter,
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
    transport::ActivationSequence,
};

mod hot;
#[cfg(all(test, feature = "simulated-ingress"))]
mod occurrence;
#[allow(dead_code, reason = "the production mixed host has not been activated")]
mod source_set;

#[cfg(all(test, feature = "simulated-ingress"))]
use occurrence::{
    LiveOccurrenceId, MixedBendCreditReturn, MixedBendResult, MixedBendSubmitError,
    MixedLiftResult, MixedLiveScope, MixedOccurrenceLedger, MixedOccurrenceState,
    MixedOccurrenceTable, MixedOnsetSubmitError, MixedPanicResult, MixedPanicSubmitError,
    MixedPedalDown, MixedPedalEdge, MixedPedalLedger, MixedPedalRefused, MixedPedalSubmitError,
    MixedReleaseSubmitError, MixedRetired,
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
    /// The ingress queue cannot withhold a bend-credit budget for every live hold and
    /// still realize every declared hold with its release plus the final onset's entry.
    #[cfg(all(test, feature = "simulated-ingress"))]
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
    #[cfg(all(test, feature = "simulated-ingress"))]
    #[error("{holds} live holds exceed the {group}-member sustain lift group")]
    SustainGroupCapacity { holds: EventCount, group: usize },
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
    #[error("private mixed ingress command identity is out of order")]
    IngressCommandOrder,
    #[cfg(all(test, feature = "simulated-ingress"))]
    #[error("private mixed ingress result does not match the next command")]
    IngressResultOrder,
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

/// One private requested-time mixed schedule for the first activation rehearsal.
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
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_commands: OpaqueQueue<HeapProd<MixedIngressCommand>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_results: OpaqueQueue<HeapCons<MixedIngressResult>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_journal: VecDeque<MixedIngressCommand>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    invalid_ingress_result: Option<MixedIngressResult>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_halt: Arc<AtomicBool>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_capacity: MixedIngressQueueCount,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_outstanding: MixedIngressQueueCount,
    /// Ring places reserved for the releases of open handles.
    #[cfg(all(test, feature = "simulated-ingress"))]
    release_reserved: MixedIngressQueueCount,
    #[cfg(all(test, feature = "simulated-ingress"))]
    last_ingress_id: Option<MixedIngressCommandId>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    occurrence_ledger: MixedOccurrenceLedger,
    /// Effective pedal state per scope and the ring places reserved for lifts.
    #[cfg(all(test, feature = "simulated-ingress"))]
    pedal_ledger: MixedPedalLedger,
    /// Scopes entitled to audio's bounded bend table. Results track the last
    /// accepted value so a refused latest bend releases a never-bent scope.
    #[cfg(all(test, feature = "simulated-ingress"))]
    bend_scopes: Vec<Option<MixedBendScopeCell>>,
    /// Credits one bend reserves: the live hold entitlement `H`, which bounds the
    /// sounding and sustained members audio can find at service.
    #[cfg(all(test, feature = "simulated-ingress"))]
    bend_credits_per_bend: MixedIngressQueueCount,
    /// Bend credits held by the merger: the budget `B` less those carried by unserved
    /// commands, held by undrained entries, or returned by audio and not yet taken.
    #[cfg(all(test, feature = "simulated-ingress"))]
    bend_credits_free: MixedIngressQueueCount,
    /// Audio's lock-free return of unused and drained bend credits.
    #[cfg(all(test, feature = "simulated-ingress"))]
    bend_credit_return: MixedBendCreditReturn,
    /// The Panic holding the protected ring place, until its result is collected. That
    /// place lies outside `ingress_capacity`, so ordinary traffic can never take it.
    #[cfg(all(test, feature = "simulated-ingress"))]
    panic_inflight: Option<MixedIngressCommandId>,
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

/// The one scope tests use where sustain scoping is not what they examine.
#[cfg(all(test, feature = "simulated-ingress"))]
const TEST_SCOPE: MixedLiveScope = MixedLiveScope::new(
    occurrence::MixedLiveSource::new(1),
    synth_core::MidiChannel::CH1,
);

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[must_use]
struct MixedIngressQueueCount(usize);

/// Prepared bend cells modeled on the private bridge's two MIDI1 sources.
/// The owner does not itself bind source IDs; ingress depth remains a floor
/// so direct private callers keep their existing table capacity.
#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
struct MixedBendScopeCapacity(usize);

#[cfg(all(test, feature = "simulated-ingress"))]
impl MixedBendScopeCapacity {
    const TWO_SOURCE_MIDI1_FLOOR: usize = source_set::MixedSourceSet::MAX_SCOPES;

    fn for_two_source_midi1(ingress: MixedIngressQueueCount) -> Self {
        Self(ingress.as_usize().max(Self::TWO_SOURCE_MIDI1_FLOOR))
    }

    const fn as_usize(self) -> usize {
        self.0
    }
}

/// One merger reservation for a scope that is bent or has an uncollected bend.
#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MixedBendScopeCell {
    scope: MixedLiveScope,
    latest: MixedIngressCommandId,
    accepted_nonzero: bool,
}

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

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug)]
struct MixedIngressCommand {
    id: MixedIngressCommandId,
    request: MixedIngressRequest,
    /// Bend credits the merger reserved for this command before minting `id`. Audio
    /// spends at most these and returns the rest when it serves the command.
    bend_credits: MixedIngressQueueCount,
}

#[cfg(all(test, feature = "simulated-ingress"))]
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
#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, PartialEq)]
enum MixedPedalSubmitted {
    /// The effective edge was pushed as this command.
    Command(MixedIngressCommandId, MixedIngressRequest),
    /// The edge repeated the merger's pedal state and was consumed and counted.
    Redundant,
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
impl MixedIngressResult {
    fn kind_matches_request(self) -> bool {
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

#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Clone, Copy, Debug, PartialEq, Error)]
enum MixedIngressResultCollectError {
    #[error("mixed ingress result does not match the next submitted command")]
    Protocol {
        received: MixedIngressCommandId,
        expected: Option<MixedIngressCommandId>,
    },
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
    fn submit_ingress(
        &mut self,
        request: MixedIngressRequest,
    ) -> Result<MixedIngressCommandId, MixedIngressSubmitError> {
        self.submit_ingress_with(request, MixedIngressQueueCount::NONE)
    }

    /// Push one command carrying `bend_credits` the caller has already reserved.
    fn submit_ingress_with(
        &mut self,
        request: MixedIngressRequest,
        bend_credits: MixedIngressQueueCount,
    ) -> Result<MixedIngressCommandId, MixedIngressSubmitError> {
        if self.ingress_room() == 0 {
            return Err(MixedIngressSubmitError::Full(request));
        }
        let id = self.push_ingress(request, bend_credits)?;
        self.ingress_outstanding.0 += 1;
        Ok(id)
    }

    /// Mint the next command identity and push the command. Room is the caller's check.
    fn push_ingress(
        &mut self,
        request: MixedIngressRequest,
        bend_credits: MixedIngressQueueCount,
    ) -> Result<MixedIngressCommandId, MixedIngressSubmitError> {
        let serial = self
            .last_ingress_id
            .map_or(Some(1), |id| id.0.checked_add(1))
            .ok_or(MixedIngressSubmitError::IdentityExhausted(request))?;
        let id = MixedIngressCommandId(serial);
        let packet = MixedIngressCommand {
            id,
            request,
            bend_credits,
        };
        self.ingress_commands
            .0
            .try_push(packet)
            .map_err(|command| MixedIngressSubmitError::Full(command.request))?;
        self.ingress_journal.push_back(packet);
        self.last_ingress_id = Some(id);
        Ok(id)
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
    /// dropped later.
    fn submit_panic(
        &mut self,
        origin: MixedIngressOriginId,
        at: SampleTime,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedPanicSubmitError> {
        if let Some(pending) = self.panic_inflight {
            return Err(MixedPanicSubmitError::Pending(pending));
        }
        let request = MixedIngressRequest::Panic { origin, at };
        // Panic carries no bend credit, so an exhausted budget never refuses it.
        let command = self
            .push_ingress(request, MixedIngressQueueCount::NONE)
            .map_err(|error| match error {
                // Ordinary commands never exceed `ingress_capacity`, so this needs a defect.
                MixedIngressSubmitError::Full(_) => MixedPanicSubmitError::RingFull,
                MixedIngressSubmitError::IdentityExhausted(_) => {
                    MixedPanicSubmitError::IdentityExhausted
                }
            })?;
        self.panic_inflight = Some(command);
        Ok((command, request))
    }

    /// Mint a handle and stage its onset, reserving one ring place for its release so
    /// the release can follow at once, before the onset's result is collected. An onset
    /// on a scope that may be bent also reserves one bend credit for its inherited
    /// entry before its command identity is minted; an unbent scope needs none.
    fn submit_onset(
        &mut self,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedOnsetSubmitError> {
        if self.ingress_room() < 2 {
            return Err(MixedOnsetSubmitError::Full);
        }
        // The ledger is as wide as the ring plus the live hold entitlement. A held cell
        // owns a ring place, or is sustained and owns an outstanding hold. If the ring
        // has two free places but no vacant cell, its generations are exhausted rather
        // than the onset being transiently blocked by ring capacity.
        let occurrence = self
            .occurrence_ledger
            .vacant()
            .ok_or(MixedOnsetSubmitError::IdentityExhausted)?;
        let credits = if self.scope_may_bend(scope) {
            MixedIngressQueueCount(1)
        } else {
            MixedIngressQueueCount::NONE
        };
        self.reserve_bend_credits(credits)
            .map_err(|_| MixedOnsetSubmitError::BendCredit)?;
        let request = MixedIngressRequest::Onset {
            origin,
            occurrence,
            scope,
            at,
            key,
            velocity,
        };
        let command = match self.submit_ingress_with(request, credits) {
            Ok(command) => command,
            Err(error) => {
                self.unreserve_bend_credits(credits);
                return Err(match error {
                    MixedIngressSubmitError::Full(_) => MixedOnsetSubmitError::Full,
                    MixedIngressSubmitError::IdentityExhausted(_) => {
                        MixedOnsetSubmitError::IdentityExhausted
                    }
                });
            }
        };
        let opened = self.occurrence_ledger.open(occurrence, scope);
        debug_assert!(opened, "the vacant handle stays vacant until opened");
        self.release_reserved.0 += 1;
        Ok((command, request))
    }

    /// Submit the release of an open handle into the place its onset reserved.
    /// If the queue or its accounting has lost that place, leave the handle open
    /// and report the broken reservation to the caller.
    fn submit_release(
        &mut self,
        origin: MixedIngressOriginId,
        occurrence: LiveOccurrenceId,
        at: SampleTime,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedReleaseSubmitError> {
        if !self.occurrence_ledger.is_open(occurrence) {
            return Err(MixedReleaseSubmitError::NotOpen(occurrence));
        }
        let request = MixedIngressRequest::Release {
            origin,
            occurrence,
            at,
        };
        self.release_reserved.0 -= 1;
        let command = match self.submit_ingress(request) {
            Ok(command) => command,
            Err(error) => {
                self.release_reserved.0 += 1;
                return Err(match error {
                    MixedIngressSubmitError::Full(_) => {
                        MixedReleaseSubmitError::ReservedPlaceUnavailable(occurrence)
                    }
                    MixedIngressSubmitError::IdentityExhausted(_) => {
                        MixedReleaseSubmitError::IdentityExhausted(occurrence)
                    }
                });
            }
        };
        let releasing = self.occurrence_ledger.begin_release(occurrence, command);
        debug_assert!(releasing, "an open handle accepts its release");
        Ok((command, request))
    }

    /// Submit one channel bend for `scope` into an ordinary ring place. A new
    /// non-zero scope reserves one audio bend-table cell, and every bend reserves `H`
    /// bend credits, before its command identity is minted. Bends are not coalesced: a
    /// full ring, scope table or credit budget refuses the caller, which keeps its input.
    fn submit_bend(
        &mut self,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        cents: Cents,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedBendSubmitError> {
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
        let command = match self.submit_ingress_with(request, needed) {
            Ok(command) => command,
            Err(error) => {
                self.unreserve_bend_credits(needed);
                return Err(match error {
                    MixedIngressSubmitError::Full(_) => MixedBendSubmitError::Full,
                    MixedIngressSubmitError::IdentityExhausted(_) => {
                        MixedBendSubmitError::IdentityExhausted
                    }
                });
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
        Ok((command, request))
    }

    /// Submit one sustain edge for `scope`. Only an edge that changes the merger's
    /// pedal state is pushed. An effective pedal-down needs its own place and one more
    /// that it reserves for the scope's lift. A broken queue or accounting
    /// reservation is reported without changing the scope's pedal state.
    fn submit_pedal(
        &mut self,
        origin: MixedIngressOriginId,
        scope: MixedLiveScope,
        at: SampleTime,
        down: bool,
    ) -> Result<MixedPedalSubmitted, MixedPedalSubmitError> {
        if self.pedal_ledger.classify(scope, down) == MixedPedalEdge::Redundant {
            return Ok(MixedPedalSubmitted::Redundant);
        }
        let request = MixedIngressRequest::Pedal {
            origin,
            scope,
            at,
            down,
        };
        if down {
            if self.ingress_room() < 2 || !self.pedal_ledger.can_press(scope) {
                return Err(MixedPedalSubmitError::Full);
            }
            let command = self.submit_ingress(request).map_err(|error| match error {
                MixedIngressSubmitError::Full(_) => MixedPedalSubmitError::Full,
                MixedIngressSubmitError::IdentityExhausted(_) => {
                    MixedPedalSubmitError::IdentityExhausted
                }
            })?;
            self.pedal_ledger.press(scope, command);
            return Ok(MixedPedalSubmitted::Command(command, request));
        }
        // The scope's reserved place becomes room for the lift itself.
        self.pedal_ledger.take_reservation();
        match self.submit_ingress(request) {
            Ok(command) => {
                self.pedal_ledger.lift(scope, command);
                Ok(MixedPedalSubmitted::Command(command, request))
            }
            Err(error) => {
                self.pedal_ledger.return_reservation();
                Err(match error {
                    MixedIngressSubmitError::Full(_) => {
                        MixedPedalSubmitError::ReservedPlaceUnavailable
                    }
                    MixedIngressSubmitError::IdentityExhausted(_) => {
                        MixedPedalSubmitError::IdentityExhausted
                    }
                })
            }
        }
    }

    /// Collect one result. A release result settles its handle: `Retired` frees the
    /// ledger cell for reuse; `Sustained` leaves the cell for the scope's lift and
    /// spends the release's place; a refused release returns custody and its reserved
    /// place to the merger. A lift result frees every cell it retired and settles the
    /// scope's lift reservation; a pedal-down result settles its reservation. A Panic
    /// result frees every sustained cell it retired and the protected place; the cells
    /// it tombstoned stay open for their key-ups. The latest bend result releases
    /// its scope cell when the last accepted value is zero.
    fn collect_ingress_result(&mut self) -> Option<MixedIngressResult> {
        match self.collect_ingress_result_checked() {
            Ok(result) => result,
            Err(error) => panic!(
                "private mixed result protocol fault: {error}; received={:?}; expected={:?}",
                self.invalid_ingress_result,
                self.ingress_journal.front()
            ),
        }
    }

    fn result_payload_matches_ledger(
        &self,
        result: MixedIngressResult,
        expected: MixedIngressCommand,
    ) -> bool {
        match (result.request, result.outcome) {
            (
                MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Release(Ok(retired)),
            ) => {
                retired.occurrence == occurrence
                    && self.occurrence_ledger.is_releasing(occurrence, result.id)
            }
            (
                MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Release(Err(_)),
            ) => self.occurrence_ledger.is_releasing(occurrence, result.id),
            (
                MixedIngressRequest::Release { occurrence, .. },
                MixedIngressOutcome::Sustained(sustained),
            ) => {
                sustained == occurrence
                    && self.occurrence_ledger.is_releasing(occurrence, result.id)
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
    fn collect_ingress_result_checked(
        &mut self,
    ) -> Result<Option<MixedIngressResult>, MixedIngressResultCollectError> {
        if let Some(received) = self.invalid_ingress_result {
            return Err(MixedIngressResultCollectError::Protocol {
                received: received.id,
                expected: self.ingress_journal.front().map(|packet| packet.id),
            });
        }
        let Some(result) = self.ingress_results.0.try_pop() else {
            return Ok(None);
        };
        let expected = self.ingress_journal.front().copied();
        let place_accounted = match result.request {
            MixedIngressRequest::Panic { .. } => self.panic_inflight == Some(result.id),
            _ => self.ingress_outstanding > MixedIngressQueueCount::NONE,
        };
        if expected.is_none_or(|packet| {
            packet.id != result.id
                || packet.request != result.request
                || !result.kind_matches_request()
                || !self.result_payload_matches_ledger(result, packet)
        }) || !place_accounted
        {
            self.invalid_ingress_result = Some(result);
            self.ingress_halt.store(true, Ordering::Release);
            return Err(MixedIngressResultCollectError::Protocol {
                received: result.id,
                expected: expected.map(|packet| packet.id),
            });
        }
        let Some(_settled) = self.ingress_journal.pop_front() else {
            self.invalid_ingress_result = Some(result);
            self.ingress_halt.store(true, Ordering::Release);
            return Err(MixedIngressResultCollectError::Protocol {
                received: result.id,
                expected: None,
            });
        };
        if let MixedIngressRequest::Panic { .. } = result.request {
            self.panic_inflight = None;
            if let MixedIngressOutcome::Panic(Ok(panicked)) = result.outcome {
                for retired in panicked.retired() {
                    let freed = self.occurrence_ledger.retire_sustained(retired);
                    debug_assert!(freed, "Panic retires only sustained cells");
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
            MixedIngressRequest::Onset { .. } | MixedIngressRequest::Panic { .. } => {}
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
/// never entered the renderer registry. `compiled_minted` lists the compiled
/// occurrences the audio minter still holds; a note-on charged in a callback that
/// then faulted appears there but not in `sounding`.
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
    uncollected_results: Vec<MixedIngressResult>,
    pending_result: Option<MixedIngressResult>,
    terminal_control: Option<MixedTerminalControlCustody>,
}

/// Control-side places and bend credits retained at a terminal mixed teardown.
#[cfg(all(test, feature = "simulated-ingress"))]
#[derive(Debug)]
struct MixedTerminalControlCustody {
    ingress_outstanding: MixedIngressQueueCount,
    release_reserved: MixedIngressQueueCount,
    pedal_reserved: MixedIngressQueueCount,
    panic_inflight: Option<MixedIngressCommandId>,
    bend_credits_free: MixedIngressQueueCount,
    bend_credits_return_pending: MixedIngressQueueCount,
    bend_scopes: Vec<MixedBendScopeCell>,
    outstanding_commands: Vec<MixedIngressCommand>,
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
    Ended(Box<MixedOneShotTeardown>),
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
    #[error("mixed audio half has not been collected")]
    UncollectedAudio,
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
    events: Vec<MixedCompiledEvent>,
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
    /// What each live handle names; the merger's ledger has the same width.
    #[cfg(all(test, feature = "simulated-ingress"))]
    occurrences: MixedOccurrenceTable,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_commands: OpaqueQueue<HeapCons<MixedIngressCommand>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_results: OpaqueQueue<HeapProd<MixedIngressResult>>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_result_pending: Option<MixedIngressResult>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    ingress_halt: Arc<AtomicBool>,
    /// The last served command ID; a gap, duplicate or wrap faults before the
    /// offending command changes audio state. Its packet remains owned for teardown.
    #[cfg(all(test, feature = "simulated-ingress"))]
    last_served_ingress_id: Option<MixedIngressCommandId>,
    #[cfg(all(test, feature = "simulated-ingress"))]
    invalid_ingress_command: Option<MixedIngressCommand>,
    /// Returns each command's unused bend credits after service, and each written one
    /// after its entry drains.
    #[cfg(all(test, feature = "simulated-ingress"))]
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
    /// The occurrence each open compiled token names; `None` for a free token.
    compiled_tokens: Vec<Option<NoteIdentity>>,
    note: NoteSlot,
    partition: Arc<MixedInstancePartition>,
    compiled_ended: Vec<Option<crate::identity::EndedNote>>,
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
    unpublished_live: Option<NoteIdentity>,
    charged_unregistered_live: Option<NoteIdentity>,
    compiled_minted: Vec<NoteIdentity>,
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
            minted_live: audio.audio.live_minter.live_identities_for_test(),
            holds_outstanding: audio.test_ingress.holds_outstanding(),
            counters: audio.test_ingress.counters(),
            occurrences: audio.occurrences.occupied(),
            pedals_down: audio.occurrences.pedals_down(),
            released_after_refusal: audio.occurrences.released_after_refusal(),
            released_after_panic: audio.occurrences.released_after_panic(),
            invalid_command: audio.invalid_ingress_command,
            invalid_result: None,
            last_served_id: audio.last_served_ingress_id,
            unserved_commands: Vec::new(),
            uncollected_results: Vec::new(),
            pending_result: None,
            terminal_control: None,
        }
    };
    Ok(MixedTeardownParts {
        sounding,
        boundary_ended,
        unpublished_live,
        charged_unregistered_live,
        compiled_minted: audio.audio.compiled_minter.live_identities(),
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
        compiled_minted: parts.compiled_minted,
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
        let ingress_faulted = audio.invalid_ingress_command.is_some()
            || self.invalid_ingress_result.is_some()
            || self.ingress_halt.load(Ordering::Acquire);
        #[cfg(not(all(test, feature = "simulated-ingress")))]
        let ingress_faulted = false;
        let terminal = ingress_faulted
            || audio.fault.is_some()
            || audio.audio.renderer.diagnostics().needs_reprepare();
        #[cfg(all(test, feature = "simulated-ingress"))]
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
                parts.ingress.invalid_result = self.invalid_ingress_result;
                parts.ingress.terminal_control = Some(MixedTerminalControlCustody {
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
        // control: the effective anchor and the in-force activation sequence.
        let capsule = &audio.capsule;
        let consistent = capsule.plan == self.control.plan.id()
            && capsule.epoch == self.control.epoch
            && capsule.table == self.control.table
            && audio.in_force == capsule.sequence
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
        let retired = MixedRetiredList {
            anchor: retired_anchor,
            unconsumed_from: retired_next,
            event_count: audio.capsule.events.len(),
        };
        audio.capsule.retired_next = None;
        drop(std::mem::take(&mut audio.capsule.events));
        Ok(MixedCollection::Resumed {
            control: MixedResumedControl {
                control: self.control,
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
        let compiled_tokens = vec![None; compiled_span.indices().len()];
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
            || audio.compiled_tokens.iter().any(Option::is_some)
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
            &audio.live_minter,
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
        // A sustained occurrence owns a release hold instead of a ring place, so the
        // occurrence tables cover the ring plus the live hold entitlement. A lift ends
        // its scope's sustained occurrences, and Panic every live occurrence, as one
        // group, which bounds that entitlement.
        // The ingress preparation above already refused a producer the plan lacks.
        #[cfg(all(test, feature = "simulated-ingress"))]
        let live_holds = control
            .plan
            .note_producer_holds()
            .get(usize::from(audio.live_minter.producer().as_u16()))
            .copied()
            .unwrap_or(EventCount::NONE);
        #[cfg(all(test, feature = "simulated-ingress"))]
        let sustain_group = crate::ingress::RELEASE_GROUP_CAPACITY;
        #[cfg(all(test, feature = "simulated-ingress"))]
        let Some((holds, occurrence_width)) = live_holds
            .as_usize()
            .filter(|holds| *holds <= sustain_group)
            .map(|holds| {
                (
                    holds,
                    MixedIngressQueueCount::capacity(ingress_capacity.as_usize() + holds),
                )
            })
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
        // The store withholds a bend-credit budget B = max(H, C - 2H) from ordinary
        // traffic, so B >= H covers one bend's every member. With H-1 live notes held
        // after their onsets drain, their holds use H-1 of the C - B ordinary places,
        // and the Hth onset needs its note-on and hold. At B = H that needs
        // C - H >= H + 1, so 2H + 1 is the smallest ingress depth that can realize every
        // declared live hold. H is at most one group here, so the arithmetic is bounded
        // by 17.
        #[cfg(all(test, feature = "simulated-ingress"))]
        let required = EventCount::measured(live_holds.get() * 2 + 1);
        #[cfg(all(test, feature = "simulated-ingress"))]
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
        // Both rings carry one protected place beyond the ordinary capacity: the merger
        // keeps it for one Panic, which ordinary commands and results cannot take.
        #[cfg(all(test, feature = "simulated-ingress"))]
        let ring_places = ingress_capacity.as_usize() + 1;
        #[cfg(all(test, feature = "simulated-ingress"))]
        let (ingress_command_writer, ingress_command_reader) =
            HeapRb::<MixedIngressCommand>::new(ring_places).split();
        #[cfg(all(test, feature = "simulated-ingress"))]
        let (ingress_result_writer, ingress_result_reader) =
            HeapRb::<MixedIngressResult>::new(ring_places).split();
        // Every bend credit starts with the merger; audio returns them through this.
        #[cfg(all(test, feature = "simulated-ingress"))]
        let bend_credit_return = MixedBendCreditReturn::new();
        #[cfg(all(test, feature = "simulated-ingress"))]
        let ingress_halt = Arc::new(AtomicBool::new(false));
        #[cfg(all(test, feature = "simulated-ingress"))]
        let bend_credit_budget = MixedIngressQueueCount(test_ingress.mixed_bend_budget());
        #[cfg(all(test, feature = "simulated-ingress"))]
        let bend_scope_capacity = MixedBendScopeCapacity::for_two_source_midi1(ingress_capacity);
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
        Ok((
            MixedOneShotControl {
                control,
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_commands: OpaqueQueue(ingress_command_writer),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_results: OpaqueQueue(ingress_result_reader),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_journal: VecDeque::with_capacity(ring_places),
                #[cfg(all(test, feature = "simulated-ingress"))]
                invalid_ingress_result: None,
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_halt: ingress_halt.clone(),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_capacity,
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_outstanding: MixedIngressQueueCount::NONE,
                #[cfg(all(test, feature = "simulated-ingress"))]
                release_reserved: MixedIngressQueueCount::NONE,
                #[cfg(all(test, feature = "simulated-ingress"))]
                last_ingress_id: None,
                #[cfg(all(test, feature = "simulated-ingress"))]
                occurrence_ledger: MixedOccurrenceLedger::prepare(occurrence_width),
                #[cfg(all(test, feature = "simulated-ingress"))]
                pedal_ledger: MixedPedalLedger::prepare(ingress_capacity),
                #[cfg(all(test, feature = "simulated-ingress"))]
                bend_scopes: vec![None; bend_scope_capacity.as_usize()],
                #[cfg(all(test, feature = "simulated-ingress"))]
                bend_credits_per_bend: MixedIngressQueueCount(holds),
                #[cfg(all(test, feature = "simulated-ingress"))]
                bend_credits_free: bend_credit_budget,
                #[cfg(all(test, feature = "simulated-ingress"))]
                bend_credit_return: bend_credit_return.clone(),
                #[cfg(all(test, feature = "simulated-ingress"))]
                panic_inflight: None,
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
                occurrences: MixedOccurrenceTable::prepare(
                    occurrence_width,
                    ingress_capacity,
                    bend_scope_capacity,
                ),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_commands: OpaqueQueue(ingress_command_reader),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_results: OpaqueQueue(ingress_result_writer),
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_result_pending: None,
                #[cfg(all(test, feature = "simulated-ingress"))]
                ingress_halt,
                #[cfg(all(test, feature = "simulated-ingress"))]
                last_served_ingress_id: None,
                #[cfg(all(test, feature = "simulated-ingress"))]
                invalid_ingress_command: None,
                #[cfg(all(test, feature = "simulated-ingress"))]
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
        if super::next_boundary(requested).is_none() {
            return Err(MixedHistoryPrepareError::BoundaryUnrepresentable { at: requested });
        }
        let control = &self.owner.control;
        let plan = &control.plan;
        let stream = control.stream.events();
        let prefix_end = stream.partition_point(|event| event.position() < position);
        let capacity = control.compiled_span.len;
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
            table: control.table,
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
            || candidate.table != control.table
        {
            return Err(MixedSuffixPrepareError::ForeignCandidate);
        }
        let stream = control.stream.events();
        if candidate.prefix_end
            != stream.partition_point(|event| event.position() < candidate.position)
        {
            return Err(MixedSuffixPrepareError::PrefixMismatch);
        }
        let mut suffix = OpenNotes::new(control.compiled_span.len, crate::ir::StealingPolicy::None);
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

    /// Place a private first-activation suffix and pair its note edges by token.
    ///
    /// Pairing runs against a scratch occupancy of the compiled span and refuses
    /// over-emission there; nothing is minted. The returned list orders requested-time
    /// restoration before suffix events at the destination. There is still no offer path.
    pub fn stamp_suffix(
        &self,
        mut suffix: MixedSuffixCandidate,
    ) -> Result<MixedStampedCandidate, MixedStampPrepareError> {
        let control = &self.owner.control;
        let history = &suffix.history;
        if history.plan != control.plan.id()
            || history.epoch != control.epoch
            || history.table != control.table
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

        // The boundary release ends every old compiled occurrence on audio before
        // this list publishes, so pairing starts from an empty scratch occupancy.
        // No old reservation, copy or generation is consulted or changed.
        let tokenized = tokenize_compiled(
            &control.plan,
            control.compiled_producer,
            control.compiled_span,
            control.epoch,
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
        let restoration_count = suffix.history.restoration_count;
        let restoration = std::mem::take(&mut suffix.history.restoration);
        suffix.history.restoration_count = EventCount::NONE;
        let mut events = Vec::with_capacity(restoration.len() + tokenized.events.len());
        for event in restoration {
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
        Ok(MixedStampedCandidate {
            suffix,
            anchor,
            supersedes: ActivationSequence::INITIAL,
            events,
            event_count,
            restoration_count,
            outstanding: tokenized.open,
            outstanding_count,
        })
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
            outstanding_count,
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
#[path = "mixed/stage_tests.rs"]
mod stage_tests;
