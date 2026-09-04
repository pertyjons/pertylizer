//! The stream's off-thread half.
//!
//! [ADR-0050](../../../plans/v2/decisions/ADR-0050-transport-activation.md) clause 9 gives a
//! stream two owners, and this is the one that never runs in a callback. It owns the current
//! anchor, the identity **minter**, and the building of anything a schedule needs; the
//! [`PreparedRenderer`] owns the clock, the carries, node state, the live-note registry, and
//! adoption.
//!
//! # Why the split exists at all
//!
//! Building a compiled schedule allocates and mints, so it cannot run on the audio thread.
//! Adopting one must run there, because only the audio thread knows which quantum is next.
//! While both halves lived in one struct, a schedule could only be prepared before the
//! stream started — `CompiledEventScheduler::prepare` needed `&mut PreparedRenderer` — which
//! is exactly why a seek during playback was unimplementable.
//!
//! The state was already split in fact: `PreparedRenderer` documented its minter as "off the
//! audio thread" and its registry as "the audio thread's half" while holding both. Nothing on
//! the audio thread ever read the minter. This module gives that division an owner.
//!
//! # One constructor, and where a crossed pair is caught
//!
//! [`StreamControl::open`] is the only way to obtain either half. It issues the epoch, builds
//! the minter, and prepares the renderer against **that** minter's table identity, so the two
//! halves it returns always answer to each other. The renderer's foreign filter compares an
//! occurrence's table against its registry's; two identities there would make every one of
//! the stream's own events look foreign.
//!
//! **What it does not prevent is a caller crossing two streams' halves**, and an independent
//! review was right that an earlier revision of this comment claimed otherwise. Two halves are
//! two values, so nothing in the type system stops someone pairing one stream's control with
//! another's renderer. That pairing is refused where it becomes wrong: a schedule carries the
//! epoch of the control that stamped it, and `CompiledEventScheduler::render` refuses a
//! renderer whose epoch differs. The refusal is the whole schedule rather than each of its
//! events discarded as stale, which is the difference between a diagnosable error and a
//! silent nothing.

use std::sync::Arc;

use thiserror::Error;

use crate::diagnostics::CompileError;
use crate::identity::{IdentityTable, NoteIdentity, ProducerId};
use crate::plan::{CompiledPlan, PlanId};
use crate::quantities::HeldNoteCount;
use crate::render::PreparedRenderer;
use crate::schedule::{AdmittedCompiledStream, CompiledPayload, SchedulePrepareError};
use crate::time::{Located, PlanPosition, SampleTime, StreamAnchor, StreamEpoch, issue_epoch};
use crate::transport::{ActivationSequence, LoopInterval, TransportActivation};

/// One repeating pass of a loop, as admitting one has to judge it.
///
/// Two quantities from one walk of one interval, answering to two records. They are carried
/// together rather than returned separately because they must describe the **same** pass: a
/// density refusal computed over one set of events and a polyphony refusal computed over
/// another would disagree without anything being able to say so.
struct RepeatingPass {
    /// Every event position inside the interval, which the window scan slides over.
    positions: Vec<PlanPosition>,
    /// The most note contracts the pass holds open at one instant.
    peak: crate::quantities::HeldNoteCount,
}

/// What a caller asks an activation to do.
///
/// Grouped because the values are one intent and are checked against each other: a position
/// without a time is not a seek.
///
/// **What a candidate supersedes is not here, and that is deliberate.** The control issues
/// it from the sequence it last promoted, so a caller cannot name one. A candidate built
/// between adoption and collection then necessarily carries the stale value and is refused
/// at the offer — which is what closes ADR-0050 clause 6's window without the control having
/// to observe an acceptance it cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ActivationRequest {
    /// The engine time the caller asks for. Adoption snaps it forward to a quantum boundary.
    pub at: SampleTime,
    /// The plan position the transport moves to. **Not** snapped: moving it would seek
    /// somewhere other than where the caller asked.
    pub position: PlanPosition,
    /// The requested loop. It is admitted off-thread, but ADR-0055 refuses it at the
    /// runtime offer until sample-exact wrapping exists.
    pub loop_interval: Option<LoopInterval>,
}

/// Why an activation could not be built.
///
/// Every one of these is off the audio thread and leaves the stream untouched, which is
/// ADR-0050 clause 3's "failure leaves everything as it was" made structural: the working
/// copy is dropped and the authoritative table never saw a mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ActivationBuildError {
    /// A note slot's gate is not a prepared parameter target, so its omitted crossing
    /// release has no row to carry its gate write.
    ///
    /// ADR-0051 clause 5 keeps that write when the note contract is dropped, and it needs a
    /// prepared row to carry it. Today's envelope exposes its gate as a parameter — the note
    /// control and the parameter are the same control — so a conforming plan does not reach
    /// this. It is a refusal rather than a skip because the alternative is silently losing the
    /// only event that could lower the gate.
    #[error("note slot {slot:?} at event {event_index} has no prepared gate parameter")]
    NoteGateNotPrepared {
        /// Which event in the caller's admitted stream.
        event_index: usize,
        /// The note slot whose gate is unprepared.
        slot: crate::plan::NoteSlot,
    },
    /// This stream has adopted a live ingress store, which ADR-0050 clause 8 puts outside
    /// this contract.
    ///
    /// Clause 5's boundary mass release ends every sounding note by freeing its index, and
    /// a live note's release hold is only discharged by the release that redeems it. An
    /// activation over an open live note would therefore leave the ingress queue reserving
    /// a slot for an event that can no longer exist. The clause scopes activation to a
    /// stream whose note producers are compiled; this is that scope refusing rather than
    /// leaking.
    #[error(
        "this stream is served by {store}, and ADR-0050 clause 8 scopes activation to compiled \
         producers"
    )]
    LiveIngressAdopted {
        /// The store this stream adopted.
        store: crate::ingress::IngressStoreId,
    },
    /// The stream has no schedule yet, so there is nothing to offer a candidate to.
    ///
    /// And none can be prepared while this candidate exists: preparation stamps against the
    /// authoritative minter, which an outstanding candidate holds still. Building one here
    /// would produce a value whose only legal next step is withdrawal.
    #[error("this stream has no schedule yet, so an activation has nothing to replace")]
    NoSchedule,

    /// The stream was admitted against a different plan.
    #[error("the stream belongs to {actual}, but this control renders {expected}")]
    ForeignPlan {
        /// The control's plan.
        expected: PlanId,
        /// The stream's.
        actual: PlanId,
    },

    /// The activation sequence space is spent.
    #[error("the activation sequence space is exhausted for this stream")]
    SequenceExhausted,

    /// The requested time has no quantum boundary at or after it.
    ///
    /// The effective point is the first boundary at or after `max(requested, clock)`, so a
    /// request in the last non-boundary frames of engine time has none. Refused here rather
    /// than at the boundary: an accepted candidate whose effective point cannot be computed
    /// occupies the exchange forever, and a wedged slot looks exactly like a stream that is
    /// simply not there yet.
    #[error("no quantum boundary exists at or after {at}")]
    BoundaryUnrepresentable {
        /// The requested engine time.
        at: SampleTime,
    },

    /// An admitted position has no representable engine time.
    #[error("event {event_index} at {position} has no representable engine time")]
    TimeUnrepresentable {
        /// Position in the admitted list.
        event_index: usize,
        /// The position that could not be placed.
        position: PlanPosition,
    },

    /// The loop is not admissible: the pass a wrap would replay exceeds a compiled bound.
    ///
    /// Two bounds from two records. ADR-0046 clause 4 checks the periodic extension of
    /// `[start, end)` against a sliding `Q`-frame window; `SOUND-INV-017` bounds the notes
    /// that pass holds at once by the compiled producer's admitted range. ADR-0050 clause 3
    /// wants the interval **already admitted** when it becomes an offer candidate, and
    /// `LoopInterval::new` proves only that the interval is positive — so without these a
    /// caller could activate a loop whose first wrap then faults at publication or
    /// over-emits, either of which ends the stream rather than refusing the state change. An
    /// independent review found the density gap; a design consultation for the wrap slice
    /// found the polyphony one.
    #[error("the loop {start}..{end} is not admissible: {source}")]
    Loop {
        /// The loop's first frame.
        start: PlanPosition,
        /// Its exclusive end.
        end: PlanPosition,
        /// Which of the two bounds the repeating pass exceeded.
        source: crate::admit::AdmissionError,
    },

    /// An occurrence the control believed the schedule in force reserved does not.
    ///
    /// The set is released into the working copy so the two schedules never compete for a
    /// producer's range; an entry that names no live note means the control's record and the
    /// allocator disagree, which is a defect in this crate rather than in a caller's plan.
    /// Refusing names it where it happened instead of letting it reappear later as an
    /// over-emission that blames the wrong list.
    #[error("the outstanding occurrence {identity:?} resolves as {resolution:?}, not live")]
    StaleReservation {
        /// The occurrence that did not resolve.
        identity: crate::identity::NoteIdentity,
        /// What the allocator says about it.
        resolution: crate::identity::Resolution,
    },

    /// Stamping refused the list.
    #[error("stamping the activation failed: {0}")]
    Stamp(#[from] SchedulePrepareError),
}

/// Why a retired activation could not be promoted.
///
/// Every variant leaves the control untouched. Promotion installs the anchor, the minter and
/// the outstanding set together, so a partial one would be worse than none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ActivationCollectError {
    /// The value was retired by another stream.
    #[error("the activation belongs to {candidate}, but this control is {stream}")]
    ForeignStream {
        /// The value's epoch.
        candidate: StreamEpoch,
        /// This control's.
        stream: StreamEpoch,
    },

    /// The value is a candidate rather than a retirement: it was never adopted.
    #[error("the activation was never adopted, so there is nothing to promote")]
    NotAdopted,

    /// The value is a retirement rather than a candidate: it cannot be withdrawn.
    ///
    /// Dropping one would destroy the only proof its control has that adoption happened,
    /// leaving it on its previous sequence with every later candidate refused as superseded.
    #[error("the activation was already adopted, so it is a retirement rather than a candidate")]
    AlreadyAdopted,

    /// Its sequence is at or behind what this control has already promoted.
    #[error("the activation is {candidate}, but {in_force} is already in force")]
    AlreadyPromoted {
        /// The value's sequence.
        candidate: ActivationSequence,
        /// What the control has already promoted.
        in_force: ActivationSequence,
    },
}

/// Refuse a note edge in a plan that declares no compiled note producer.
///
/// The suffix's edges are checked by `stamp_into`, but the history's never reach it — so
/// without this a plan with a playable node and no producer could build an activation whose
/// suffix was derived from edges nothing was entitled to emit, while preparing the very same
/// stream refused it. An independent review found the asymmetry. The index is the caller's
/// own, because history events are not rebased.
fn require_note_producer(
    plan: &CompiledPlan,
    event_index: usize,
) -> Result<(), ActivationBuildError> {
    if plan.compiled_note_producer().is_some() {
        return Ok(());
    }
    Err(ActivationBuildError::Stamp(
        SchedulePrepareError::NoCompiledNoteProducer { event_index },
    ))
}

/// Record a note edge's effect on the gate it moves, in the same history a parameter writes.
///
/// Every prepared row aliasing that physical gate is written, so two parameters addressing one
/// `(node, control)` cannot end up disagreeing about it.
fn write_gate(
    values: &mut [Option<crate::quantities::ParameterValue>],
    gate_rows: &[Vec<usize>],
    slot: crate::plan::NoteSlot,
    value: crate::quantities::ParameterValue,
) {
    let Some(rows) = gate_rows.get(slot.index()) else {
        return;
    };
    for row in rows {
        if let Some(entry) = values.get_mut(*row) {
            *entry = Some(value);
        }
    }
}

/// Record a note-on's **magnitudes** in that same history.
///
/// ADR-0051's rule is that a target carries the last write before the destination, and since
/// `SOUND-INV-021` a note-on writes more than its gate. Without this, seeking into a sounding
/// note restores the gate and leaves the pitch at the value the node was *prepared* with — the
/// note would resume, audibly, as a different note.
///
/// Written through the prepared rows the magnitude's physical `(node, control)` resolves to,
/// exactly as [`write_gate`] does and for the same aliasing reason.
fn write_magnitudes(
    values: &mut [Option<crate::quantities::ParameterValue>],
    plan: &crate::plan::CompiledPlan,
    slot: crate::plan::NoteSlot,
    key: crate::quantities::KeyIdentity,
    velocity: crate::quantities::NoteVelocity,
) {
    let targets = plan.parameter_targets();
    for magnitude in plan.note_magnitudes_of(slot) {
        let Some(value) = plan.magnitude_value(magnitude, key, velocity) else {
            continue;
        };
        for (index, target) in targets.iter().enumerate() {
            if target.node == magnitude.node
                && target.control == magnitude.control
                && let Some(entry) = values.get_mut(index)
            {
                *entry = Some(value);
            }
        }
    }
}

/// Rewrite a stamping error's event index from the suffix back to the admitted stream.
///
/// The suffix is built here — history dropped, crossing-note releases omitted — so an index
/// into it names nothing a caller can look at. Every variant that carries one is rewritten,
/// which is why this matches exhaustively rather than reaching for a field: a variant added
/// later with an index has to answer for itself here.
fn rebase(error: SchedulePrepareError, sources: &[usize]) -> SchedulePrepareError {
    let at = |index: usize| sources.get(index).copied().unwrap_or(index);
    match error {
        SchedulePrepareError::UnmatchedRelease { event_index } => {
            SchedulePrepareError::UnmatchedRelease {
                event_index: at(event_index),
            }
        }
        SchedulePrepareError::NoCompiledNoteProducer { event_index } => {
            SchedulePrepareError::NoCompiledNoteProducer {
                event_index: at(event_index),
            }
        }
        SchedulePrepareError::Identity {
            event_index,
            source,
        } => SchedulePrepareError::Identity {
            event_index: at(event_index),
            source,
        },
        SchedulePrepareError::ForeignPlan {
            event_index,
            expected,
            actual,
        } => SchedulePrepareError::ForeignPlan {
            event_index: at(event_index),
            expected,
            actual,
        },
        SchedulePrepareError::BeforeAnchor {
            event_index,
            position,
            anchor,
        } => SchedulePrepareError::BeforeAnchor {
            event_index: at(event_index),
            position,
            anchor,
        },
        SchedulePrepareError::TimeUnrepresentable {
            event_index,
            position,
        } => SchedulePrepareError::TimeUnrepresentable {
            event_index: at(event_index),
            position,
        },
        // No index to rewrite: the stream as a whole belongs to another plan, or the refusal
        // is about the control's own state rather than about any event.
        SchedulePrepareError::ForeignStream { .. }
        | SchedulePrepareError::CandidateOutstanding { .. }
        | SchedulePrepareError::SchedulerExists => error,
    }
}

/// The first quantum boundary at or after `at`, or `None` where engine time has none.
///
/// The same rule the audio thread applies to `max(requested, clock)`; the build uses it on
/// the requested time alone, which is the only half a caller can get wrong.
fn next_boundary(at: SampleTime) -> Option<SampleTime> {
    let quantum = u64::from(crate::time::QUANTUM_FRAMES);
    let remainder = at.as_u64() % quantum;
    if remainder == 0 {
        return Some(at);
    }
    at.as_u64()
        .checked_add(quantum - remainder)
        .map(SampleTime::new)
}

/// The off-thread half of one prepared stream.
///
/// Holds no audio-thread state and is never touched from a callback.
#[derive(Debug)]
#[must_use]
pub struct StreamControl {
    epoch: StreamEpoch,
    /// The same plan the renderer runs, shared rather than copied.
    ///
    /// Holding it is what lets a schedule be **built** without borrowing the renderer at
    /// all, which is the whole point of ADR-0050 clause 9: an off-thread builder cannot hold
    /// a shared borrow of a value the audio thread mutates, so a design that read the plan
    /// through the renderer would have split the ownership in name only.
    plan: Arc<CompiledPlan>,
    anchor: StreamAnchor,
    minter: IdentityTable,
    /// What the schedule **in force** left live in the allocator.
    ///
    /// The note-ons its own list never paired. ADR-0050 clause 3 releases exactly this set
    /// into the working copy a replacement is stamped against, so the two schedules never
    /// compete for a producer's range.
    ///
    /// It is held here rather than read from the scheduler because the scheduler is the
    /// audio thread's. An off-thread builder that borrowed it could not run while the stream
    /// renders, which would split the ownership in name only — clause 9's whole point, and
    /// the same defect an independent review found in `prepare`'s first shape.
    outstanding: Vec<NoteIdentity>,
    /// The sequence this control believes is in force: the last one it promoted.
    ///
    /// Every candidate supersedes it. Between adoption and collection it is stale, and that
    /// staleness is the mechanism rather than a flaw — a candidate built in that window
    /// names a superseded value and the offer refuses it.
    in_force: ActivationSequence,
    /// The highest sequence this control has issued.
    last_issued: ActivationSequence,
    /// Whether this stream's one schedule has been prepared.
    ///
    /// A stream has exactly one, because the exchange is what orders activations and two
    /// schedulers are two exchanges — a candidate accepted by each would be adopted twice.
    has_scheduler: bool,
    /// Candidates built and neither withdrawn nor collected.
    ///
    /// Each one holds a snapshot of the minter that promotion will install, so while any is
    /// outstanding the authoritative table may not move: a stamping committed in between
    /// would have its generations rewound by that promotion, and a later note could then be
    /// given an identity that is already live. An independent review found the rewind.
    live_candidates: usize,
    /// Live notes this control has minted and not yet released.
    ///
    /// **A transport activation may not happen while one is open**, and the reason is a
    /// leak rather than a taste: ADR-0050 clause 5's boundary mass release ends every
    /// sounding note through `IdentityTable::release_all`, which frees the index without
    /// passing through the ingress store. The store's release hold would then never be
    /// discharged — its note is gone, so no release can ever redeem it — and the queue
    /// would carry a permanent reservation for an event that can never arrive. ADR-0046
    /// clause 6 asks a mass release to redeem every affected hold atomically, and nothing
    /// does that yet.
    ///
    /// ADR-0050 clause 8 already scopes activation to a stream whose note producers are
    /// compiled, so this is that scope made executable rather than a new restriction. The
    /// refusal is what keeps it from being a silent leak.
    live_notes_open: HeldNoteCount,
    /// The ingress store this stream serves, latched on first use.
    ///
    /// One store per stream, refused rather than assumed: two stores for one producer each
    /// hold that producer's whole hold entitlement, so the pair admits twice what ADR-0046
    /// clause 6 partitioned, and a release offered to the wrong one spends a reservation it
    /// never made. The registry's own rule is the same shape — a second live store needs its
    /// own row and its own admitting ground.
    #[cfg_attr(
        not(feature = "simulated-ingress"),
        allow(
            dead_code,
            reason = "the offers that read it are the boundary ADR-0053 clause 5 asks for"
        )
    )]
    ingress_store: Option<crate::ingress::IngressStoreId>,
}

impl StreamControl {
    /// Open a stream, returning both halves.
    ///
    /// The epoch is issued here rather than inside the renderer because it identifies the
    /// **stream**, which is both halves, and because a value two constructors could produce
    /// is a value the two can disagree about.
    pub fn open(
        plan: CompiledPlan,
        anchor: StreamAnchor,
    ) -> Result<(Self, PreparedRenderer), CompileError> {
        let epoch = issue_epoch()?;
        // ADR-0047 clause 3's identity partition, from what admission copied into the plan.
        // A plan with no note-on producers gets an empty partition, which is right: nothing
        // can start a note, so nothing can name an occurrence.
        let minter = IdentityTable::from_admitted_ranges(plan.note_producer_ranges())?;
        let id = minter.id();
        let plan = Arc::new(plan);
        let renderer = PreparedRenderer::prepare(Arc::clone(&plan), anchor, epoch, id)?;
        let control = Self {
            epoch,
            plan,
            anchor,
            minter,
            outstanding: Vec::new(),
            in_force: ActivationSequence::INITIAL,
            last_issued: ActivationSequence::INITIAL,
            live_candidates: 0,
            live_notes_open: HeldNoteCount::NONE,
            ingress_store: None,
            has_scheduler: false,
        };
        Ok((control, renderer))
    }

    /// This stream's epoch.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// The plan this stream renders.
    pub fn plan(&self) -> &CompiledPlan {
        &self.plan
    }

    /// Its identity, for a comparison that does not need the whole plan.
    pub fn plan_id(&self) -> PlanId {
        self.plan.id()
    }

    /// The activation sequence in force, which a replacement schedule must start from.
    ///
    /// A scheduler prepared after an activation was adopted inherits this rather than
    /// starting at `INITIAL`, because the control's next candidate supersedes it.
    pub const fn in_force(&self) -> ActivationSequence {
        self.in_force
    }

    /// The transport's current pairing of plan time and engine time.
    ///
    /// This is the value a schedule is placed at. The renderer holds the same pairing and
    /// reads it for `plan_position_of`; [`Self::open`] establishes both from one argument,
    /// and ADR-0050's activation is what moves them together afterwards.
    pub const fn anchor(&self) -> StreamAnchor {
        self.anchor
    }

    /// Build an activation that moves the transport to `request.position`.
    ///
    /// ADR-0050 clauses 3, 5 and 7, and everything that can refuse happens here rather than
    /// at the boundary. The candidate is stamped against a **working copy** of the minter,
    /// with the schedule in force releasing its outstanding occurrences into it first, so
    /// that the two schedules never compete for a producer's range and so that abandoning
    /// the candidate costs nothing — not an index, not a generation. The copy travels with
    /// the candidate and is promoted at [`Self::adopted`].
    ///
    /// **`stream` is the plan's whole admitted stream, not a suffix**, and that is what
    /// makes an ordinary seek correct rather than merely buildable. Two of clause 7's rules
    /// need what lies *before* the destination: a release whose note-on is before it has
    /// nothing to pair with and is omitted, and the notes open there are what bound the
    /// history against the producer's range. A suffix carries neither, so a builder handed
    /// one refuses every seek through a held note. The suffix is derived here instead, in one
    /// pass, off the audio thread.
    ///
    /// Nothing about the stream is touched on failure: the working copy is dropped and the
    /// authoritative table never saw a mint.
    pub fn plan_activation(
        &mut self,
        stream: &AdmittedCompiledStream,
        request: ActivationRequest,
    ) -> Result<Box<TransportActivation>, ActivationBuildError> {
        // **A candidate needs a schedule to be offered to, and one that does not exist yet
        // cannot be created afterwards**: preparing it stamps against the authoritative
        // minter, and an outstanding candidate is exactly what forbids that. Building here
        // would hand back a value whose only legal next step is withdrawal. An independent
        // review found the dead end.
        if !self.has_scheduler {
            return Err(ActivationBuildError::NoSchedule);
        }
        // ADR-0050 clause 8's scope, enforced rather than assumed. Two things go wrong
        // otherwise, and the second is why this refuses on the **store** rather than on the
        // notes it currently holds open. Clause 5's boundary mass release ends a live note
        // without passing through the store, stranding that note's release hold forever. And
        // ADR-0051 clause 6 leaves a gate reached by two producers with no ownership law, so
        // the catch-up's row can cut a live note the activation does not own.
        //
        // **A count of open notes cannot see either.** An offered note-on and its offered
        // release take that count back to zero while both are still queued and neither has
        // rendered — so an activation built there is built over a live note that is about to
        // sound, and its catch-up row can cut it. An independent review found exactly that
        // sequence. A stream that has adopted a store at all is out of scope, which is the
        // conservative reading and the only one the state supports.
        if let Some(store) = self.ingress_store {
            return Err(ActivationBuildError::LiveIngressAdopted { store });
        }
        if stream.plan() != self.plan.id() {
            return Err(ActivationBuildError::ForeignPlan {
                expected: self.plan.id(),
                actual: stream.plan(),
            });
        }
        // The effective point must exist before anything is stamped for it.
        if next_boundary(request.at).is_none() {
            return Err(ActivationBuildError::BoundaryUnrepresentable { at: request.at });
        }
        let sequence = self
            .last_issued
            .next()
            .ok_or(ActivationBuildError::SequenceExhausted)?;

        // The table as it will be **after** the activation: the schedule in force releases
        // its reservations, because clause 5's boundary release ends its notes.
        let mut minter = self.minter.working_copy();
        for identity in &self.outstanding {
            // **The resolution is checked rather than discarded.** This set is what the
            // control believes the schedule in force reserves, and releasing it is a state
            // reconstruction: an entry that does not name a live note means the belief is
            // wrong, and carrying on would either promote a leaked slot or surface later as
            // an over-emission that names the wrong cause. An independent review found the
            // result thrown away.
            //
            // **The refusal is unreachable, and here is the argument rather than a
            // reassurance.** Only `stamp_compiled` writes this set, and it appends exactly
            // what its own list left unpaired — indices it minted into the authoritative
            // table and did not release. Nothing else releases into that table: a candidate
            // is stamped against a *copy*, and promotion replaces the table and this set
            // together, from a copy in which every entry here had already been released. So
            // every recorded occurrence is live until the pair of them moves at once. It is
            // written as a refusal anyway because the alternative is discarding a
            // reconstruction result, which is the rule that brought it here. It has no test,
            // which is stated rather than covered by a fixture that cannot reach it.
            match minter.release(*identity) {
                crate::identity::Resolution::Live => {}
                resolution => {
                    return Err(ActivationBuildError::StaleReservation {
                        identity: *identity,
                        resolution,
                    });
                }
            }
        }

        // Placed against the requested time. Adoption may snap that forward, and clause 1
        // makes the whole stream shift with it rather than the plan position moving.
        let anchor = StreamAnchor::new(request.at, request.position);

        // One pass over the whole stream, and the anchor itself is what separates history
        // from suffix: `locate` already answers "is this before the destination?", so a
        // second comparison here would be a second authority on the same question.
        // Note-ons before the anchor, per node. A release pairs with the most recent
        // unreleased note-on on its node, so which side that note-on is on is what decides
        // whether the release belongs in the suffix at all.
        let mut before_anchor = vec![0_u32; self.plan.note_targets().len()];
        let mut in_suffix = vec![0_u32; self.plan.note_targets().len()];
        // How many notes the history has open at once, against what the producer admits.
        // The history never reaches `stamp_into`, so without this a timeline the producer was
        // never entitled to emit would still decide which crossing releases the suffix omits —
        // and `plan_activation` would accept a stream that preparing refuses. An independent
        // review found the asymmetry.
        let mut open_now = 0_u32;
        let admitted_notes = self
            .plan
            .compiled_note_producer()
            .and_then(|producer| {
                self.plan
                    .note_producer_ranges()
                    .get(producer.as_u16() as usize)
                    .copied()
            })
            .map_or(0, crate::quantities::HeldNoteCount::get);
        let mut placed = Vec::with_capacity(stream.events().len());
        // Where each placed event came from in the caller's admitted stream. The suffix drops
        // history and crossing-note releases, so a stamping error's index would otherwise name
        // a position in a list the caller has never seen — an independent review found it
        // reporting the fourth event of a stream as its first.
        let mut sources = Vec::with_capacity(stream.events().len());
        let mut omitted_releases = 0_usize;
        // ADR-0051. The last write before the destination, per prepared target. Note edges
        // write here too: a gate raised by automation and then dropped by a note-off before
        // the destination must be restored **low**, and a history that saw only
        // `SetParameter` would restore the raised value the note-off had undone.
        let mut values: Vec<Option<crate::quantities::ParameterValue>> =
            vec![None; self.plan.parameter_targets().len()];
        // Which prepared rows each note slot's gate is, resolved once by **physical**
        // `(node, control)` rather than by slot. Two prepared parameters aliasing one gate
        // would otherwise disagree — one forced low, a later one restoring what it read —
        // and whichever published last would win.
        let gate_rows = self.gate_rows();
        // Note-ons still unpaired at the anchor, per note slot. Kept apart from
        // `before_anchor`, which the suffix decrements as it omits crossing releases: reusing
        // one counter for both erases the record that the note was open there, and the seek
        // then re-opens a gate nothing will lower.
        let mut open_at_anchor = vec![0_u32; self.plan.note_targets().len()];

        for (event_index, event) in stream.events().iter().copied().enumerate() {
            let time = match anchor.locate(event.position()) {
                Located::At(time) => time,
                Located::BeforeAnchor => {
                    // History. Only the note depth is read from it: what a locate restores
                    // to a parameter target is clause 7's catch-up batch, which this slice
                    // does not build.
                    match event.payload() {
                        CompiledPayload::SetParameter { slot, value } => {
                            if let Some(entry) = values.get_mut(slot.index()) {
                                *entry = Some(value);
                            }
                        }
                        CompiledPayload::NoteOn {
                            slot,
                            key,
                            velocity,
                        } => {
                            require_note_producer(&self.plan, event_index)?;
                            write_gate(
                                &mut values,
                                &gate_rows,
                                slot,
                                crate::quantities::ParameterValue::ONE,
                            );
                            write_magnitudes(&mut values, &self.plan, slot, key, velocity);
                            if let Some(depth) = open_at_anchor.get_mut(slot.index()) {
                                *depth = depth.saturating_add(1);
                            }
                            open_now = open_now.saturating_add(1);
                            if open_now > admitted_notes {
                                return Err(ActivationBuildError::Stamp(
                                    SchedulePrepareError::Identity {
                                        event_index,
                                        source:
                                            crate::identity::IdentityError::ProducerOverEmitted {
                                                producer: self
                                                    .plan
                                                    .compiled_note_producer()
                                                    .unwrap_or(ProducerId::new(0)),
                                                admitted:
                                                    crate::quantities::HeldNoteCount::measured(
                                                        admitted_notes,
                                                    ),
                                            },
                                    },
                                ));
                            }
                            if let Some(depth) = before_anchor.get_mut(slot.index()) {
                                *depth = depth.saturating_add(1);
                            }
                        }
                        CompiledPayload::NoteOff { slot } => {
                            require_note_producer(&self.plan, event_index)?;
                            let sounding =
                                before_anchor.get(slot.index()).copied().unwrap_or(0) > 0;
                            if sounding {
                                open_now = open_now.saturating_sub(1);
                            }
                            if !sounding {
                                // Nothing to pair it with, on either side. `AdmittedCompiledStream`
                                // does not check pairing, and the suffix stamper never sees this
                                // event — so a saturating subtract here would turn a malformed
                                // timeline into a successful activation. Clause 5 omits a release
                                // whose note-on **precedes the anchor**, which this is not.
                                return Err(ActivationBuildError::Stamp(
                                    SchedulePrepareError::UnmatchedRelease { event_index },
                                ));
                            }
                            write_gate(
                                &mut values,
                                &gate_rows,
                                slot,
                                crate::quantities::ParameterValue::ZERO,
                            );
                            if let Some(depth) = open_at_anchor.get_mut(slot.index()) {
                                *depth = depth.saturating_sub(1);
                            }
                            if let Some(depth) = before_anchor.get_mut(slot.index()) {
                                *depth = depth.saturating_sub(1);
                            }
                        }
                    }
                    continue;
                }
                Located::Unrepresentable => {
                    return Err(ActivationBuildError::TimeUnrepresentable {
                        event_index,
                        position: event.position(),
                    });
                }
            };

            match event.payload() {
                CompiledPayload::NoteOn { slot, .. } => {
                    if let Some(depth) = in_suffix.get_mut(slot.index()) {
                        *depth = depth.saturating_add(1);
                    }
                }
                CompiledPayload::NoteOff { slot } => {
                    let paired_here = in_suffix.get(slot.index()).copied().unwrap_or(0) > 0;
                    if paired_here {
                        if let Some(depth) = in_suffix.get_mut(slot.index()) {
                            *depth = depth.saturating_sub(1);
                        }
                    } else if before_anchor.get(slot.index()).copied().unwrap_or(0) > 0 {
                        // Clause 5: its note-on lies before the anchor, so after the seek
                        // that note is not sounding — the boundary release ended it and the
                        // new stream never started it. Omitted and **counted**, off the
                        // audio thread, so it is a named transformation rather than a
                        // silent drop the renderer would have to make.
                        if let Some(depth) = before_anchor.get_mut(slot.index()) {
                            *depth = depth.saturating_sub(1);
                        }
                        omitted_releases = omitted_releases.saturating_add(1);
                        // ADR-0051 clause 5: what is omitted is the note **contract**, not
                        // the gate write the plan authored. Dropping both leaves a gate that
                        // automation can raise after the destination with nothing left in the
                        // stream able to lower it — a note sounding that playing through
                        // would have ended. The identity goes; the gate write stays, at the
                        // release's own position.
                        let Some(row) = gate_rows.get(slot.index()).and_then(|rows| rows.first())
                        else {
                            return Err(ActivationBuildError::NoteGateNotPrepared {
                                event_index,
                                slot,
                            });
                        };
                        placed.push(crate::schedule::CompiledEvent::new(
                            time,
                            CompiledPayload::SetParameter {
                                slot: crate::plan::ParameterSlot::new(self.plan.id(), *row),
                                value: crate::quantities::ParameterValue::ZERO,
                            },
                        ));
                        sources.push(event_index);
                        continue;
                    }
                    // Neither side has a note-on for it. That is a malformed list rather
                    // than a seek, and stamping refuses it by name.
                }
                CompiledPayload::SetParameter { .. } => {}
            }
            placed.push(crate::schedule::CompiledEvent::new(time, event.payload()));
            sources.push(event_index);
        }

        // ADR-0046 clause 4, and **against what repeats** — which is neither the stream this
        // candidate was derived from nor the suffix it carries. This is `admit_loop`'s first
        // caller: an interval reaches the offer candidate only through here, so "already
        // admitted" is a fact about the value rather than a rule someone has to remember.
        //
        // Judging the *suffix this candidate carries* misses events in
        // `[loop_start, request.position)`: the first pass skips them because it enters the
        // loop late, but every wrap after it re-anchors at the loop's start and plays them.
        // What repeats is a pass anchored at `loop_start`, so that is what is derived here.
        //
        // Judging the *original stream* would be harmless rather than wrong, and saying so is
        // the correction a merge review forced: `admit_loop` filters its input to the interval
        // before extending it, so history outside the loop cannot reach the window either way.
        // An earlier revision of this comment claimed that history was the discriminator, and
        // built a test on it that could not fail.
        //
        // **Two rules from two records, over one derived pass.** ADR-0046 clause 4 bounds the
        // pass's events per quantum against the compiled share; `SOUND-INV-017` bounds the
        // notes it holds at once against what the compiled producer is admitted for. Neither
        // implies the other. The second is the same rule the history walk above and
        // `stamp_into` below already apply to their own timelines; the pass a wrap replays is
        // a third, and it had no enforcement point — so a loop needing more identity than the
        // producer holds could be recorded here and would over-emit at its first real wrap.
        if let Some(interval) = request.loop_interval {
            let repeating = self.repeating_pass(stream, interval);
            crate::admit::admit_loop(
                &repeating.positions,
                interval.start(),
                interval.end(),
                self.plan.compiled_event_share(),
            )
            .map_err(|source| ActivationBuildError::Loop {
                start: interval.start(),
                end: interval.end(),
                source,
            })?;
            // **Only where a producer exists to be measured against.** With none declared,
            // `admitted_notes` is zero and a pass holding any note would be refused as one
            // the producer admits nothing of — which is a different fact from there being no
            // producer at all, and it would classify one invalid note two ways depending on
            // whether a loop was supplied. That refusal has owners already:
            // `require_note_producer` above for a note in the history, and `stamp_into` below
            // for one in the suffix. An independent review found the misreport.
            if self.plan.compiled_note_producer().is_some() {
                crate::admit::admit_loop_polyphony(
                    repeating.peak,
                    crate::quantities::HeldNoteCount::measured(admitted_notes),
                    interval,
                )
                .map_err(|source| ActivationBuildError::Loop {
                    start: interval.start(),
                    end: interval.end(),
                    source,
                })?;
            }
        }

        // ADR-0051 clause 1. A gate held open by a note contract at the destination is owed
        // `ZERO`, whatever the last write before that position was. The boundary mass release
        // lowers it and the batch is published **after** those gate-downs, so a batch carrying
        // the raised value would re-raise it — and because a gate is edge-triggered, that pair
        // is a rising edge that restarts an envelope no note contract stands behind.
        //
        // **The predicate is the destination-open contract, not the mass release's scope.** A
        // forward seek can land inside a note the retired stream never sounded, and that gate
        // must still be low. The producer scope the clause also names holds structurally here
        // rather than as a branch: **no stream that can activate has a live ingress store**,
        // because `plan_activation` refuses once one is adopted and `latch_store` refuses
        // while a candidate is outstanding. So every contract this walk sees is the compiled
        // producer's. Phase 3's live ingress made those checks necessary rather than leaving
        // this as a fact about what the crate happened to contain. The store check is
        // intentionally not a count of notes currently open — that count goes to zero while
        // both edges of a live note are still queued.
        for (index, depth) in open_at_anchor.iter().copied().enumerate() {
            if depth == 0 {
                continue;
            }
            let Some(rows) = gate_rows.get(index) else {
                continue;
            };
            for row in rows {
                if let Some(entry) = values.get_mut(*row) {
                    *entry = Some(crate::quantities::ParameterValue::ZERO);
                }
            }
        }
        let catch_up = self.catch_up(request.at, &values);

        let (events, outstanding) =
            crate::schedule::stamp_into(&mut minter, &self.plan, self.epoch, &placed)
                .map_err(|error| ActivationBuildError::Stamp(rebase(error, &sources)))?;
        let ended = vec![None; self.index_space()];

        self.last_issued = sequence;
        self.live_candidates = self.live_candidates.saturating_add(1);
        Ok(Box::new(TransportActivation {
            epoch: self.epoch,
            sequence,
            supersedes: self.in_force,
            requested: request.at,
            position: request.position,
            events,
            outstanding,
            minter,
            omitted_releases,
            catch_up,
            loop_interval: request.loop_interval,
            producers: self.note_producers(),
            ended,
            late: false,
            effective: None,
            retired: None,
        }))
    }

    /// Take a retired activation, promoting what it proves.
    ///
    /// Collection is the first moment adoption is a fact rather than a plan (ADR-0050
    /// clause 3): the renderer can end the epoch before the boundary — an oversized
    /// callback, a publication fault, a clock exhaustion — and then no later call advances
    /// toward it. Promoting at the offer would spend the candidate's generations for an
    /// activation that never happened.
    ///
    /// What is promoted is the candidate's own half of the value: its minter copy, its
    /// outstanding set and its sequence. The event list the box carries is the **retired**
    /// one, and it is freed here, off the audio thread, which is the whole reason the box
    /// travels back.
    ///
    /// **Refuses a value that is not this stream's**, and returns rather than ignoring it.
    /// The promotion installs an anchor, a minter and an outstanding set, so a retirement
    /// collected from another stream's scheduler would make this control mint from a table
    /// its own renderer treats as foreign — every note it then placed would be filtered as
    /// belonging to someone else. An independent review found that the first version checked
    /// only whether the value had been adopted at all.
    pub fn adopted(
        &mut self,
        retired: Box<TransportActivation>,
    ) -> Result<(), (Box<TransportActivation>, ActivationCollectError)> {
        // **The value goes back with the refusal**, exactly as a refused candidate does at
        // the offer, and for a stronger reason: a retirement is the only proof its own
        // control has that adoption happened. Consuming one handed to the wrong control
        // would leave the right one stuck on its previous sequence with every later
        // candidate refused as superseded, and nothing to re-derive it from. An independent
        // review found the first version unboxing before it checked.
        if retired.epoch != self.epoch {
            let error = ActivationCollectError::ForeignStream {
                candidate: retired.epoch,
                stream: self.epoch,
            };
            return Err((retired, error));
        }
        let Some(effective) = retired.effective else {
            return Err((retired, ActivationCollectError::NotAdopted));
        };
        if retired.sequence <= self.in_force {
            let error = ActivationCollectError::AlreadyPromoted {
                candidate: retired.sequence,
                in_force: self.in_force,
            };
            return Err((retired, error));
        }
        let retired = *retired;
        self.anchor = StreamAnchor::new(effective, retired.position);
        self.minter = retired.minter;
        self.outstanding = retired.outstanding;
        self.in_force = retired.sequence;
        self.live_candidates = self.live_candidates.saturating_sub(1);
        Ok(())
    }

    /// Discard an unadopted candidate, restoring nothing because nothing was taken.
    ///
    /// **Dropping a candidate instead of withdrawing it strands the control**, and the
    /// limitation is stated rather than defended: nothing decrements the outstanding count,
    /// so every later `stamp_compiled` refuses with `CandidateOutstanding`. An independent
    /// review asked for `Drop`-based bookkeeping; the value cannot reach its control from
    /// `Drop`, and the crate's real-time rules forbid the indirection that would let it. The
    /// exposure is small — after preparation there is no legitimate second stamping, because
    /// a stream has one schedule — and the refusal names itself, so a caller that hits it is
    /// told what happened rather than left with a silent stream. The next slice to touch
    /// this ownership should carry a token the type system can enforce.
    ///
    /// The whole of clause 3's withdrawal: the working copy the candidate carries is dropped
    /// with it and the authoritative table never saw the mints. A candidate returned refused
    /// at the offer comes back here.
    ///
    /// Takes the box the offer returned, and frees it **here**, off the audio thread, which
    /// is the whole reason it travels back at all. By box rather than by value so that a
    /// refusal can hand it back without moving three hundred bytes through a `Result`, which
    /// is the same shape [`Self::adopted`] uses for the same reason.
    pub fn withdraw(
        &mut self,
        activation: Box<TransportActivation>,
    ) -> Result<(), (Box<TransportActivation>, ActivationCollectError)> {
        // **Refuses what is not this control's to drop, and gives it back.** A foreign
        // candidate dropped here would free the owning control's snapshot while decrementing
        // a counter that is not its own, leaving that control unable to stamp for the rest of
        // the stream; a *retirement* dropped here would destroy the only proof its owner has
        // that adoption happened. An independent review found both, and they are the same
        // mistake `adopted` was already made to refuse.
        if activation.epoch != self.epoch {
            let error = ActivationCollectError::ForeignStream {
                candidate: activation.epoch,
                stream: self.epoch,
            };
            return Err((activation, error));
        }
        if activation.effective.is_some() {
            return Err((activation, ActivationCollectError::AlreadyAdopted));
        }
        drop(activation);
        self.live_candidates = self.live_candidates.saturating_sub(1);
        Ok(())
    }

    /// Which prepared rows each note slot's gate is, resolved by **physical** target.
    ///
    /// ADR-0051 clause 3. A note edge and a `SetParameter` can address one `(node, control)`,
    /// and so can two prepared parameters. Matching on the physical pair rather than on a slot
    /// is what keeps the **substitution** single-valued when they do — not the whole catch-up:
    /// the last-write half writes each addressed value slot on its own, so two aliased slots
    /// with different pre-destination writes would still disagree. A plan lowers one prepared
    /// target per `(node, control)`, so that cannot arise today; ADR-0051 clause 3 carries the
    /// same bound, and a node kind that ever aliases has to decide the last-write half.
    fn gate_rows(&self) -> Vec<Vec<usize>> {
        let targets = self.plan.parameter_targets();
        self.plan
            .note_targets()
            .iter()
            .map(|note| {
                targets
                    .iter()
                    .enumerate()
                    .filter(|(_, target)| {
                        target.node == note.node && target.control == note.control
                    })
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect()
    }

    /// ADR-0051 clause 1's batch: one row per prepared target, at the requested time.
    ///
    /// A target with a write before the destination carries it; one with none carries the
    /// value it was **prepared** with — the target row's stored base, which is also the
    /// slot's — because a control survives an activation in node state and a skipped target
    /// would keep whatever the pre-seek position left. Either way the renderer takes the
    /// value as an **override-layer** write (`SOUND-INV-023`) and re-derives modulation from
    /// the slot, never from a flattened figure. Covering every
    /// target is also what makes the size exactly the prepared-target count, which is the
    /// quantity the plan-dependent session-share admission checks — that count plus one for
    /// ADR-0050 clause 5's boundary mass release, and `first_refusal` acts on it.
    fn catch_up(
        &self,
        at: SampleTime,
        values: &[Option<crate::quantities::ParameterValue>],
    ) -> Vec<crate::render::TimedEvent> {
        let targets = self.plan.parameter_targets();
        let mut batch = Vec::with_capacity(targets.len());
        for (index, target) in targets.iter().enumerate() {
            let value = values.get(index).copied().flatten().unwrap_or(target.base);
            batch.push(crate::render::TimedEvent::new(
                crate::render::EventEnvelope::new(
                    self.epoch,
                    at,
                    crate::time::TimeSource::Compiled,
                ),
                crate::render::EventPayload::SetParameter {
                    slot: crate::plan::ParameterSlot::new(self.plan.id(), index),
                    value,
                },
            ));
        }
        batch
    }

    /// What a wrap replays: every event the loop interval contains, and how many notes it
    /// holds open at once.
    ///
    /// **One walk for both, because they are one pass.** The two quantities answer to
    /// different records — ADR-0046 clause 4 and `SOUND-INV-017` — but they are properties of
    /// the same events, and two walks over one interval is how they come to disagree about
    /// which events those are. The same reason `producer_spans` is one function, and here the
    /// disagreement would be silent: the density check would refuse against one set of
    /// positions while the polyphony check admitted against another.
    ///
    /// # Which events the positions carry
    ///
    /// Every event inside the interval, with **no note bookkeeping applied**, and that is
    /// ADR-0051's doing rather than an omission. An earlier shape tracked open depth so it
    /// could decide whether a release whose note-on lies before the interval carried an event
    /// at all — under the bare omission it did not. Clause 5 keeps that release's gate write
    /// at its own position, so **every** event inside the interval carries one and the
    /// question the bookkeeping answered no longer has two answers. Keeping the state would
    /// have been a check that cannot fail, and an independent review of the merge is what
    /// exposed the version that still skipped the position.
    ///
    /// # How the peak is counted
    ///
    /// The pass starts at **zero** open notes rather than inheriting the depth at
    /// `loop_start`. ADR-0050 clause 5's boundary mass release is what makes that true: a
    /// wrap ends the notes the previous pass opened, so the pass a wrap replays begins with
    /// nothing sounding. Inheriting a depth would count notes twice — once where they open
    /// and once in every later pass — and refuse loops that hold nothing.
    ///
    /// A release inside the interval closes the most recent unclosed on edge for its slot,
    /// which is the pairing `stamp_into` performs. A release whose on edge lies **before**
    /// the interval is ADR-0051 clause 5's crossing release: it carries a bare gate-down and
    /// no note contract, so it lowers nothing here. Its own depth is spent, so a second
    /// release for that slot cannot claim to cross too.
    ///
    /// **A release that pairs with nothing on either side raises the peak rather than
    /// refusing.** Pairing has an owner — the anchored walk refuses one in history, and
    /// `stamp_into` refuses one in the suffix — and a second authority here is how the two
    /// come to disagree. Leaving it unpaired can only leave `live` higher than the truth, so
    /// a malformed stream is refused by this check or by its owner, never admitted by both.
    fn repeating_pass(
        &self,
        stream: &AdmittedCompiledStream,
        interval: LoopInterval,
    ) -> RepeatingPass {
        let mut positions = Vec::with_capacity(stream.events().len());
        // Open on edges per slot, on each side of the loop's start. Two tables rather than
        // one, because a release consults them in order and a shared counter could not say
        // which side the note it pairs with is on.
        let mut before = vec![0_u32; self.plan.note_targets().len()];
        let mut inside = vec![0_u32; self.plan.note_targets().len()];
        let mut live = 0_u32;
        let mut peak = 0_u32;
        for event in stream.events().iter().copied() {
            let position = event.position();
            if position < interval.start() {
                match event.payload() {
                    CompiledPayload::NoteOn { slot, .. } => {
                        if let Some(depth) = before.get_mut(slot.index()) {
                            *depth = depth.saturating_add(1);
                        }
                    }
                    CompiledPayload::NoteOff { slot } => {
                        if let Some(depth) = before.get_mut(slot.index()) {
                            *depth = depth.saturating_sub(1);
                        }
                    }
                    CompiledPayload::SetParameter { .. } => {}
                }
                continue;
            }
            if position >= interval.end() {
                break;
            }
            positions.push(position);
            match event.payload() {
                CompiledPayload::NoteOn { slot, .. } => {
                    if let Some(depth) = inside.get_mut(slot.index()) {
                        *depth = depth.saturating_add(1);
                    }
                    live = live.saturating_add(1);
                    peak = peak.max(live);
                }
                CompiledPayload::NoteOff { slot } => {
                    if inside.get(slot.index()).copied().unwrap_or(0) > 0 {
                        if let Some(depth) = inside.get_mut(slot.index()) {
                            *depth = depth.saturating_sub(1);
                        }
                        live = live.saturating_sub(1);
                    } else if before.get(slot.index()).copied().unwrap_or(0) > 0
                        && let Some(depth) = before.get_mut(slot.index())
                    {
                        *depth = depth.saturating_sub(1);
                    }
                }
                CompiledPayload::SetParameter { .. } => {}
            }
        }
        RepeatingPass {
            positions,
            peak: crate::quantities::HeldNoteCount::measured(peak),
        }
    }

    /// The producer whose schedule this activation replaces, which is the release's scope.
    ///
    /// **The compiled one, and only it.** ADR-0050 clause 5 ends the notes of "a producer
    /// whose schedule the activation replaces", and an activation replaces an
    /// `AdmittedCompiledStream` — nothing else. A scope that named every declared producer
    /// would, once a non-compiled one can sound a note, cut a performer's note on a seek: a
    /// seek moves plan time, it does not lift a finger. An independent review found the
    /// over-wide scope, and clause 8 already puts a non-compiled producer out of this
    /// record's reach.
    ///
    /// It is also what bounds the audio thread's walk. A plan may declare thousands of
    /// producers and still be admitted, because admission charges the identity **span**; an
    /// earlier revision walked the declarations and skipped the empty ones inside the loop,
    /// which left it `O(declarations)` while its comment claimed otherwise.
    fn note_producers(&self) -> Vec<ProducerId> {
        let Some(compiled) = self.plan.compiled_note_producer() else {
            return Vec::new();
        };
        let admits = self
            .plan
            .note_producer_ranges()
            .get(compiled.as_u16() as usize)
            .copied()
            .is_some_and(|range| range.get() > 0);
        if admits { vec![compiled] } else { Vec::new() }
    }

    /// How many identity indices the partition holds, which bounds one boundary release.
    fn index_space(&self) -> usize {
        self.plan
            .note_producer_ranges()
            .iter()
            .map(|range| range.get() as usize)
            .sum()
    }

    /// Whether this stream's one schedule has been prepared.
    pub const fn has_scheduler(&self) -> bool {
        self.has_scheduler
    }

    /// Record that this stream's schedule now exists.
    pub(crate) const fn scheduler_prepared(&mut self) {
        self.has_scheduler = true;
    }

    /// How many candidates are built and neither withdrawn nor collected.
    ///
    /// While any is, the authoritative minter may not move: each holds a snapshot that
    /// promotion installs, and a stamping committed in between would be rewound by it.
    pub const fn outstanding_candidates(&self) -> usize {
        self.live_candidates
    }

    /// The shared plan handle, for a caller that must keep it while the control is borrowed.
    pub(crate) const fn plan_arc(&self) -> &Arc<CompiledPlan> {
        &self.plan
    }

    /// Offer a live note-on into a producer's ingress store.
    ///
    /// The off-thread half owns the minter, so the offer is made here rather than on the
    /// store: an identity minted from any other table is one the renderer refuses as
    /// foreign, and this is what makes that unrepresentable.
    ///
    /// `HOST-INV-009`'s three resources are acquired together or not at all, and the store
    /// names which one was exhausted.
    #[cfg(feature = "simulated-ingress")]
    pub fn offer_note_on(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: crate::time::SampleTime,
        note: crate::plan::NoteSlot,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Result<NoteIdentity, crate::ingress::IngressRefused> {
        // The authoritative minter may not move while a candidate holds a snapshot of it,
        // and a mint is exactly a move. Refused rather than allowed to rewind: ADR-0050
        // clause 8 scopes activation to a stream whose note producers are compiled, so a
        // live producer beside a pending activation is out of scope, not supported.
        self.latch_store(store)?;
        // **Before the mint and before the hold**, because a slot that names another plan is
        // not a shortage to recover from — it is an offer that would play the wrong note.
        // The renderer does not re-check this: `note_target` applies the slot's index to
        // whichever plan is rendering, so nothing downstream can catch it.
        if note.plan() != self.plan.id() {
            return Err(crate::ingress::IngressRefused::ForeignSlot {
                slot: note.plan(),
                stream: self.plan.id(),
            });
        }
        let identity = store.offer_note_on(&mut self.minter, time, note, key, velocity)?;
        self.live_notes_open =
            HeldNoteCount::measured(self.live_notes_open.get().saturating_add(1));
        Ok(identity)
    }

    /// Offer a live parameter write into this stream's ingress store.
    ///
    /// It needs no minter, and it is here anyway: the stream latches one store, and an
    /// offer that bypassed that latch would let a second store fill a queue the stream never
    /// adopted — and overwrite the first store's cumulative counters in the report, because
    /// the drain mirrors those totals rather than accumulating them.
    #[cfg(feature = "simulated-ingress")]
    pub fn offer_parameter(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: crate::time::SampleTime,
        slot: crate::plan::ParameterSlot,
        value: crate::quantities::ParameterValue,
    ) -> Result<(), crate::ingress::IngressRefused> {
        self.latch_store(store)?;
        store.offer_parameter(time, slot, value)
    }

    /// Adopt this stream's one ingress store, or refuse a second.
    ///
    /// **Two checks, and they answer different questions.** This stream may serve one store,
    /// because two would each hold the producer's whole entitlement; and this store may be
    /// served by one stream, because its entitlement and its identity range would otherwise
    /// come from different plans. The mark the second check sets lives on the **store**, so
    /// the audio-thread half verifies the same adoption instead of keeping a latch of its
    /// own that could disagree with this one.
    #[cfg(feature = "simulated-ingress")]
    fn latch_store(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
    ) -> Result<(), crate::ingress::IngressRefused> {
        // **A candidate outstanding refuses every offer, and for two reasons that would
        // otherwise be checked in two places.** The authoritative minter may not move while a
        // candidate holds a snapshot of it, so a mint would have its generation rewound by
        // that candidate's promotion. And `plan_activation` refuses once a store is adopted,
        // which is what keeps a live producer out of ADR-0050 clause 8's scope — but that
        // check runs when the candidate is *built*, so adopting a store afterwards walks
        // straight past it. An independent review found that ordering: a parameter offer,
        // which mints nothing and so had no reason of its own to refuse, adopted a store
        // between a candidate's build and its offer.
        if self.live_candidates > 0 {
            return Err(crate::ingress::IngressRefused::CandidateOutstanding);
        }
        if let Some(latched) = self.ingress_store
            && latched != store.id()
        {
            return Err(crate::ingress::IngressRefused::ForeignStore {
                latched,
                offered: store.id(),
            });
        }
        // **Adopt first, record second.** Recording the id before the fallible adoption
        // poisoned the control: a refused foreign store left its id latched here, and the
        // control's own store was then rejected as foreign for the rest of the stream. A
        // refusal must leave the stream exactly as it found it, which is the same rule every
        // activation refusal follows. An independent review found the ordering.
        store.adopt(self.epoch)?;
        self.ingress_store = Some(store.id());
        Ok(())
    }

    /// Offer the release of a live note this stream's minter opened.
    ///
    /// Freeing the index is a move of the authoritative minter too, so it takes the same
    /// refusal while a candidate is outstanding.
    #[cfg(feature = "simulated-ingress")]
    pub fn offer_note_off(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: crate::time::SampleTime,
        identity: NoteIdentity,
    ) -> Result<(), crate::ingress::IngressRefused> {
        self.latch_store(store)?;
        store.offer_note_off(&mut self.minter, time, identity)?;
        self.live_notes_open =
            HeldNoteCount::measured(self.live_notes_open.get().saturating_sub(1));
        Ok(())
    }

    /// Live notes this control has minted and not yet released.
    ///
    /// **A diagnostic, not a guard.** It counts note-ons this control has minted and not yet
    /// released, which goes to zero as soon as a release is *offered* — while both edges are
    /// still queued and neither has rendered. An earlier revision gated transport activation
    /// on it and an independent review found that sequence: the count says nothing about
    /// what is about to sound. `plan_activation` refuses on the adopted store instead.
    ///
    /// A `HeldNoteCount` because that is exactly what it is: notes a source holds open at
    /// once, the same quantity `SOUND-INV-017`'s admitted producer range bounds. A raw
    /// `usize` here would be comparable with a queue length and a frame count, which the
    /// critical newtype rule exists to prevent.
    pub const fn live_notes_open(&self) -> HeldNoteCount {
        self.live_notes_open
    }

    /// The minting half, for the off-thread producer that stamps a compiled list.
    ///
    /// Crate-private because a producer outside this crate has no admitted range: the
    /// partition is the plan's, and reaching it from elsewhere would put occurrences outside
    /// what admission checked.
    pub(crate) const fn minter_mut(&mut self) -> &mut IdentityTable {
        &mut self.minter
    }

    /// Record what a committed stamping left live in the allocator.
    ///
    /// Called by [`stamp_compiled`](crate::schedule::stamp_compiled), which is the only
    /// thing that commits against the authoritative table. Holding the set here rather than
    /// in the schedule is what lets a replacement be built while that schedule renders.
    ///
    /// **It accumulates**, because stamping does: every committed list mints into the one
    /// authoritative table and releases nothing, so a second stamping's set is additional
    /// rather than a replacement. Overwriting lost the first list's reservations while the
    /// minter still held them, and an activation could then not reclaim an index no schedule
    /// admitted to holding — an independent review found it. Promotion at collection is what
    /// resets the set, and it may, because the candidate was stamped against a copy with
    /// **all** of these already released.
    pub(crate) fn add_outstanding(&mut self, outstanding: &[NoteIdentity]) {
        self.outstanding.extend_from_slice(outstanding);
    }
}
