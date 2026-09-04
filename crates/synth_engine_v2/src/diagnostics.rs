//! What compilation and rendering say when something is wrong.
//!
//! Three rules shape this module:
//!
//! - **A refusal names the field, both amounts, and the responsible object**
//!   (`HOST-INV-007`). A diagnostic a reader cannot act on is the failure mode this
//!   whole contract exists to remove.
//! - **A caller precondition returns a `Result`, not a `debug_assert`.** A debug
//!   assertion compiles out of the build that runs, so it cannot define release
//!   behaviour; it may only supplement one.
//! - **Every counter reaches the structured diagnostics report.** `LIMIT-0013`'s
//!   drop counters existed for years and reached no consumer at all, which is the
//!   specific failure the report is the control against.

use thiserror::Error;

use crate::ir::{IrError, IrObject};
use crate::profile::ProfileError;
use crate::quantities::ChannelLayout;
use crate::report::{ResourceAmount, ResourceField};
use crate::time::{FrameCount, TimeError};

/// Why a plan was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum CompileError {
    /// One execution scope holds two playable nodes.
    ///
    /// `SOUND-INV-021` binds a note's magnitudes by execution scope: a note sent to a node
    /// reaches every pitch and velocity destination its **scope** declares. Two playable
    /// nodes in one scope therefore share one set of destinations, so playing either would
    /// move the other's velocity — and, where the scope has an oscillator, contend for one
    /// pitch. The plan is refused rather than resolved by declaration order.
    ///
    /// The invariant states this over [`crate::ir::ExecutionScope::Voice`], because that is
    /// where two instruments land and `Voice` names a kind rather than an instance. The
    /// check is over **every** scope because the reason is: the binding merges within a
    /// scope, and nothing about that is special to `Voice`. Phase 6 supplies instance
    /// identity and generalises the binding.
    #[error(
        "{first} and {second} are both playable in the {scope:?} scope, which binds one \
             set of note destinations"
    )]
    AmbiguousNoteScope {
        /// The first playable node found in the scope.
        first: crate::ir::NodeId,
        /// The second.
        second: crate::ir::NodeId,
        /// The scope holding both.
        scope: crate::ir::ExecutionScope,
    },

    /// A playable node's scope declares no destination for a note's velocity.
    ///
    /// `SOUND-INV-021` requires velocity to be **audible**: the Phase 4 gate states that a
    /// fixed-velocity render cannot satisfy it, so a typed velocity reaching nothing would
    /// satisfy the letter of the invariant and none of its purpose. A scope declaring none is
    /// refused here rather than rendering every note at one loudness.
    #[error("{node} is playable, but nothing in its {scope:?} scope receives a note's velocity")]
    NoteScopeWithoutVelocity {
        /// The playable node.
        node: crate::ir::NodeId,
        /// The scope whose kinds declare no velocity destination.
        scope: crate::ir::ExecutionScope,
    },

    /// A scope with a pitch destination states no tuning to resolve keys through.
    ///
    /// `SOUND-INV-021` puts the key-to-frequency mapping in the plan, so a pitch destination
    /// with no prepared tuning has nothing to resolve against. No default is substituted:
    /// choosing a scale is the authored model's decision and Phase 10A's, and picking one
    /// here would silently decide what the plan sounds like.
    #[error("{node} receives a note's pitch, but its {scope:?} scope states no tuning")]
    ScopeWithoutTuning {
        /// The node declaring the pitch destination.
        node: crate::ir::NodeId,
        /// Its scope.
        scope: crate::ir::ExecutionScope,
    },

    /// The plan's admitted producer partition could not become an identity table.
    ///
    /// Unreachable through the ordinary path — profile construction and plan admission
    /// between them establish both relations `IdentityTable` checks — and reported rather
    /// than assumed away, because a preparation that silently built a smaller table would
    /// hand the renderer one that refuses valid identities.
    #[error("the admitted note-producer partition is not a usable identity table: {0}")]
    IdentityPartition(#[from] crate::identity::IdentityError),

    /// A note-on producer declared more holds than notes.
    ///
    /// A hold is taken *by* a note-on, so a source cannot hold more obligations than it has
    /// notes sounding. ADR-0046 clause 6 gives a hold only to a note-on "whose complete
    /// note-on/release pair is not already present in one indivisible materialized
    /// open-window batch", so holds are a subset of notes rather than a separate budget.
    #[error("note producer {index} declares {holds} holds against {notes} simultaneous notes")]
    ProducerHoldsExceedNotes {
        /// Position in the plan's producer list.
        index: usize,
        /// Holds declared.
        holds: crate::quantities::EventCount,
        /// Notes declared.
        notes: crate::quantities::HeldNoteCount,
    },

    /// A compiled note-on producer declared a hold.
    ///
    /// ADR-0046 clause 6: "Compiled releases use plan entitlements and need no hold." A
    /// compiled source that asked for one would consume `release_hold_capacity` that the
    /// non-compiled producers are entitled to, which is the partition this refusal keeps
    /// disjoint.
    #[error(
        "compiled note producer {index} declares {holds} holds, but compiled releases need none"
    )]
    CompiledProducerDeclaresHold {
        /// Position in the plan's producer list.
        index: usize,
        /// Holds declared.
        holds: crate::quantities::EventCount,
    },

    /// A plan declared a second compiled note producer.
    ///
    /// `PlanDeclarations::events_per_quantum` is one figure, admitted against
    /// `compiled_event_share`. A second compiled producer would have no separate envelope to
    /// be admitted against, and `stamp_compiled` would have no way to say which of the two a
    /// compiled note-on belongs to — so the identity it minted could land in either
    /// producer's range. Refusing here is what lets both answers be a lookup rather than a
    /// guess.
    #[error(
        "note producers {first} and {second} are both compiled, but a plan carries one compiled \
         event stream"
    )]
    SecondCompiledProducer {
        /// Position of the first compiled producer.
        first: usize,
        /// Position of the second.
        second: usize,
    },

    /// An authored runtime source named a note producer the plan does not declare.
    ///
    /// ADR-0046 clause 5 admits an authored source against envelopes, and clause 6 gives its
    /// note-ons holds from a producer entitlement. A source whose `ProducerId` resolves to
    /// nothing has no entitlement to spend, so its declared holds would be checked against
    /// an absent partition rather than a disjoint one.
    #[error(
        "authored source {index} names note producer {producer}, which the plan does not \
         declare"
    )]
    AuthoredSourceProducerUnknown {
        /// Position in the plan's authored-source list.
        index: usize,
        /// The producer it named.
        producer: crate::identity::ProducerId,
    },

    /// An authored runtime source named the plan's compiled note producer.
    ///
    /// ADR-0046 clause 6: "Compiled releases use plan entitlements and need no hold." A
    /// compiled producer therefore holds an entitlement of zero, so routing an authored
    /// source through it would spend holds the partition never granted — the same
    /// disjointness [`Self::CompiledProducerDeclaresHold`] keeps on the declaring side.
    #[error("authored source {index} names compiled note producer {producer}, which holds none")]
    AuthoredSourceProducerCompiled {
        /// Position in the plan's authored-source list.
        index: usize,
        /// The compiled producer it named.
        producer: crate::identity::ProducerId,
    },

    /// An authored runtime source declared more holds than its producer is entitled to.
    ///
    /// The producer's `simultaneous_holds` is its whole entitlement under ADR-0046 clause 6's
    /// disjoint partition. A source claiming more would be spending another producer's unused
    /// holds, which that clause forbids by name, and the overrun would surface as an
    /// exhausted entitlement at runtime rather than as a plan that was never admissible.
    #[error(
        "authored source {index} declares {holds} holds against note producer {producer}'s \
         entitlement of {entitlement}"
    )]
    AuthoredSourceHoldsAboveEntitlement {
        /// Position in the plan's authored-source list.
        index: usize,
        /// The producer whose entitlement it names.
        producer: crate::identity::ProducerId,
        /// Holds the source declared.
        holds: crate::quantities::EventCount,
        /// The entitlement the producer declared.
        entitlement: crate::quantities::EventCount,
    },

    /// A plan asked for more than a render limit allows.
    ///
    /// Admission never truncates, clamps, or drops to make a plan fit: exceeding a
    /// limit never rewrites authored data, so the only outcomes are a prepared plan
    /// and this.
    #[error(
        "{field} exceeded: {requested} requested, {available} available, contributed most by \
         {responsible}"
    )]
    LimitExceeded {
        /// Which limit.
        field: ResourceField,
        /// What the plan asked for.
        requested: ResourceAmount,
        /// What the profile allows.
        available: ResourceAmount,
        /// The authored object that contributed most to the request.
        responsible: IrObject,
    },

    /// Several signals are patched into one input.
    ///
    /// Summing them is a fan-in policy, and no accepted decision defines one. Refusing
    /// is the honest move: silently taking the first would be exactly the quiet
    /// reduction this contract exists to remove. The diagnostic names the two edges
    /// that collide, because "this input has three sources" is not something a reader
    /// can act on without knowing which.
    #[error(
        "{edges} edges reach {node} {port}, including {first} and {second}; one input takes one \
         source"
    )]
    UnsupportedFanIn {
        /// The node whose input is over-subscribed.
        node: crate::ir::NodeId,
        /// Which input.
        port: crate::ir::PortId,
        /// How many edges reach it.
        edges: u32,
        /// The first edge into it.
        first: crate::ir::EdgeId,
        /// The next edge into it.
        second: crate::ir::EdgeId,
    },

    /// A node cannot be prepared for the stream this plan is admitted against.
    ///
    /// Distinct from a limit and from a cable error: the graph is well formed, it fits,
    /// and the node's own values are legal — they are just not legal *here*. Only a
    /// property of the stream can produce it, which is why it names the node and both
    /// values rather than a resource field.
    #[error("{node} cannot be prepared for this stream: {fault}")]
    NodeNotPreparable {
        /// The node that could not be prepared.
        node: crate::ir::NodeId,
        /// Why.
        fault: PreparationFault,
    },

    /// An edge names a port the node does not declare.
    #[error("{edge} names {node} {port}, which is not an {needed} port it declares")]
    UnknownPort {
        /// The offending edge.
        edge: crate::ir::EdgeId,
        /// The node it names.
        node: crate::ir::NodeId,
        /// The port it names.
        port: crate::ir::PortId,
        /// The direction the edge needed it in.
        needed: crate::validate::PortDirection,
    },

    /// An edge uses a port that exists in the other direction.
    ///
    /// Separate from [`Self::UnknownPort`] because it is a different mistake: the port
    /// is real and the cable is backwards, which is worth saying rather than making a
    /// reader wonder whether the node has the port at all.
    #[error(
        "{edge} uses {node} {port} as an {needed}, but it is declared in the other direction; the \
         cable's other end is {other_node} {other_port}"
    )]
    PortDirection {
        /// The offending edge.
        edge: crate::ir::EdgeId,
        /// The node.
        node: crate::ir::NodeId,
        /// The port.
        port: crate::ir::PortId,
        /// The direction the edge needed.
        needed: crate::validate::PortDirection,
        /// The node at the cable's other end.
        other_node: crate::ir::NodeId,
        /// The port at the cable's other end.
        other_port: crate::ir::PortId,
    },

    /// An edge crosses two signal domains, or declares a third.
    #[error(
        "{edge} carries {declared} from {source_node} {source_port} ({source_domain}) into \
         {target_node} {target_port} ({target_domain}); all three must agree"
    )]
    DomainMismatch {
        /// The offending edge.
        edge: crate::ir::EdgeId,
        /// The source node.
        source_node: crate::ir::NodeId,
        /// The source port.
        source_port: crate::ir::PortId,
        /// What the source produces.
        source_domain: crate::ir::SignalDomain,
        /// The target node.
        target_node: crate::ir::NodeId,
        /// The target port.
        target_port: crate::ir::PortId,
        /// What the target consumes.
        target_domain: crate::ir::SignalDomain,
        /// What the edge says it carries.
        declared: crate::ir::SignalDomain,
    },

    /// An edge crosses two channel layouts with no conversion that resolves it.
    ///
    /// ADR-0002 clause 6: mono into stereo is duplicated, and the reverse is refused
    /// rather than down-mixed, because choosing a summing law is a product decision
    /// with no caller in this phase.
    #[error(
        "{edge} carries {source_layout} from {source_node} {source_port} into {target_node} \
         {target_port}, which takes {target_layout}"
    )]
    LayoutMismatch {
        /// The offending edge.
        edge: crate::ir::EdgeId,
        /// The source node.
        source_node: crate::ir::NodeId,
        /// The source port.
        source_port: crate::ir::PortId,
        /// What the source produces.
        source_layout: ChannelLayout,
        /// The target node.
        target_node: crate::ir::NodeId,
        /// The target port.
        target_port: crate::ir::PortId,
        /// What the target consumes.
        target_layout: ChannelLayout,
    },

    /// The graph has a cycle.
    ///
    /// Phase 2 refuses every cycle rather than scheduling one. ADR-0033 owns the
    /// delay-boundary rule that would relax this and is `Proposed`, so the message
    /// deliberately promises no way to break the cycle: there is not one yet.
    ///
    /// It names the edge that closes the cycle and the node re-entered rather than
    /// listing the whole cycle, because this error is `Copy` and a list would put an
    /// allocation in a type that admission returns by value. The closing edge is also
    /// the actionable half — it is the cable to remove.
    #[error("{edge} closes a cycle back into {node}, through {nodes}; a cycle is refused")]
    Cycle {
        /// The edge that closes the cycle.
        edge: crate::ir::EdgeId,
        /// The node it re-enters.
        node: crate::ir::NodeId,
        /// How many nodes were on the walk when it closed.
        nodes: crate::quantities::NodeCount,
    },

    /// The plan has sources but nowhere for them to go.
    #[error("the plan declares {sources} of sources and no output node")]
    MissingOutput {
        /// How many sources are stranded.
        sources: crate::quantities::NodeCount,
    },

    /// A plan declares more than one output.
    ///
    /// Phase 1 renders one. Taking the first and ignoring the rest would be a silent
    /// choice about which output the plan has.
    #[error("the plan declares {outputs} of outputs; this phase renders one")]
    MultipleOutputs {
        /// How many outputs the plan declares.
        outputs: crate::quantities::NodeCount,
    },

    /// The IR could not be read.
    #[error("the plan could not be read: {0}")]
    Ir(#[from] IrError),

    /// A report row compared two different units, which is an internal defect.
    ///
    /// It is surfaced rather than swallowed because a mismatch means the comparison
    /// that decides admission did not happen for that field.
    #[error("internal: the report row for {field} compares mismatched units")]
    ReportUnitMismatch {
        /// The field whose row is malformed.
        field: ResourceField,
    },

    /// Preparation could not issue a stream epoch.
    #[error("preparation failed: {0}")]
    Time(#[from] TimeError),

    /// The profile itself was invalid.
    #[error("profile invalid: {0}")]
    Profile(#[from] ProfileError),
}

/// Why a node's prepared data could not be built.
///
/// Its own enum rather than a string, because a caller that wants to fix the plan needs
/// the numbers: which value was too high, and what it was measured against.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum PreparationFault {
    /// A filter's corner frequency is at or above the stream's Nyquist frequency.
    ///
    /// Refused rather than clamped. Clamping would render a filter the caller did not
    /// ask for, and the difference is audible.
    #[error(
        "its corner frequency {cutoff} is at or above the stream's Nyquist frequency of \
         {nyquist} Hz"
    )]
    CutoffAboveNyquist {
        /// The corner frequency the node declared.
        cutoff: crate::quantities::CutoffFrequency,
        /// Half this stream's sample rate.
        nyquist: crate::quantities::Frequency,
    },

    /// A node's authored values are each legal and the filter they imply is not.
    ///
    /// The class a range check on the inputs cannot replace: a denormal quality factor
    /// passes every test one could write for it on its own, and the coefficients it
    /// implies leave the representation — after which the node admits, renders, and is
    /// silent forever with nothing in the plan or the report saying so. The derived
    /// values are therefore checked rather than the inputs guessed at, because the
    /// formula is what decides which pairs are usable.
    #[error(
        "its corner frequency {cutoff} and {resonance} produce coefficients this stream \
         cannot represent"
    )]
    CoefficientsUnusable {
        /// The corner frequency the node declared.
        cutoff: crate::quantities::CutoffFrequency,
        /// The quality factor the node declared.
        resonance: crate::quantities::Resonance,
    },

    /// A node's coefficients are representable and the filter they make is not stable.
    ///
    /// Separate from [`Self::CoefficientsUnusable`] because it is the opposite failure:
    /// nothing underflowed, and rounding moved a pole *outside* the unit circle instead.
    /// The audible result is a filter whose output climbs for as long as the stream runs,
    /// which is worth its own diagnostic — a reader told only "unusable" would look for
    /// a value that had vanished.
    #[error(
        "its corner frequency {cutoff} and {resonance} produce a filter that is not stable \
         at this rate"
    )]
    CoefficientsUnstable {
        /// The corner frequency the node declared.
        cutoff: crate::quantities::CutoffFrequency,
        /// The quality factor the node declared.
        resonance: crate::quantities::Resonance,
    },

    /// A segment lasts more frames than a frame counter can hold.
    ///
    /// Refused where the duration becomes a frame count, because that is where it stops
    /// being a legal quantity and becomes a number this node cannot use. Left to run, it
    /// would prepare a segment that never advances — a note that never finishes starting.
    #[error("a segment of {duration} is more than the {limit} frames a segment can last")]
    SegmentTooLong {
        /// The duration the node declared.
        duration: crate::quantities::Seconds,
        /// The most frames a segment may last.
        limit: u32,
    },
}

/// Something worth saying about a plan that was nevertheless admitted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompileWarning {
    /// The compiler inserted an implicit conversion on an edge.
    ///
    /// ADR-0002 clause 7 requires a conversion to appear in the schedule, in the
    /// resource report's buffer count, **and** in a diagnostic. The first two say what
    /// the plan costs; this one says what the compiler decided on the author's behalf,
    /// which is the part a reader cannot recover from an operation list.
    ConversionInserted {
        /// The edge that needed it.
        edge: crate::ir::EdgeId,
        /// What was inserted.
        conversion: crate::validate::Conversion,
    },

    /// The plan has an output node that nothing reaches.
    ///
    /// A **warning** rather than a refusal, deliberately: such a plan renders silence,
    /// which is a legitimate intermediate state for a patch under construction — and
    /// once plans can be swapped live, refusing it would mean the engine rejects a plan
    /// the moment a cable is unplugged. What it must not do is stay quiet about it.
    OutputNotReached {
        /// The output node nothing feeds.
        output: crate::ir::NodeId,
    },

    /// An advisory budget was exceeded.
    ///
    /// `HOST-INV-015`: compilation continues. The predicted and permitted values
    /// both travel with the warning, because a warning that does not say by how
    /// much is a warning nobody can act on.
    AdvisoryBudgetExceeded {
        /// Which advisory field.
        field: ResourceField,
        /// What was predicted.
        predicted: ResourceAmount,
        /// What the budget permits.
        permitted: ResourceAmount,
        /// The object contributing most to the prediction.
        contributor: IrObject,
    },
}

impl std::fmt::Display for CompileWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConversionInserted { edge, conversion } => {
                write!(f, "{edge} carries an inserted {conversion}")
            }
            Self::OutputNotReached { output } => write!(
                f,
                "{output} is the plan's output and nothing reaches it; this plan renders silence"
            ),
            Self::AdvisoryBudgetExceeded {
                field,
                predicted,
                permitted,
                contributor,
            } => write!(
                f,
                "{field} is advisory and was exceeded: {predicted} predicted against {permitted} \
                 permitted, contributed most by {contributor}"
            ),
        }
    }
}

/// Why a render call could not be served.
///
/// Every variant is returned **before** renderer state or output is mutated, except
/// the two stream-contract faults, which are terminal for the epoch and say so:
/// they silence the output, invalidate the carries, and publish `needs_reprepare`.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum RenderError {
    /// The caller delivered more frames than the profile's maximum block.
    ///
    /// ADR-0021 part 3's terminal stream-contract fault: output silence, both
    /// carries invalidated, `needs_reprepare` published, nothing allocated. The
    /// engine makes no claim that the old input epoch continues, so recovery is
    /// re-preparation.
    #[error(
        "callback of {frames} exceeds maximum_block_size {maximum}; the stream contract is broken \
         and the epoch is over"
    )]
    OversizedCallback {
        /// What the caller delivered.
        frames: FrameCount,
        /// What the profile admits.
        maximum: FrameCount,
    },

    /// The output slice does not match the frames and layout it claims.
    #[error(
        "output buffer holds {samples} samples, but {frames} frames of {layout} needs {needed}"
    )]
    OutputBufferShape {
        /// Samples the caller provided.
        samples: usize,
        /// Frames the caller asked for.
        frames: usize,
        /// The stream's layout.
        layout: ChannelLayout,
        /// Samples that layout requires for those frames.
        needed: usize,
    },

    /// One quantum was presented with more events than it admits.
    ///
    /// Rejected before anything is mutated. Phase 1 and Phase 2 accept an open caller
    /// span, so an over-full quantum is a caller-contract violation and nothing may
    /// allocate, defer, or trim to absorb it. ADR-0046 removes capacity deferral;
    /// Phase 3 replaces this boundary with sealed admitted input and a terminal
    /// response for a violated share or store declaration.
    #[error(
        "quantum {quantum_index} was presented with {requested} events against \
         max_events_per_quantum {available}"
    )]
    QuantumEventOverflow {
        /// Which quantum, absolutely.
        quantum_index: u64,
        /// Events due in it.
        requested: u32,
        /// Events it admits.
        available: u32,
    },

    /// The span holds more events than any one call could admit.
    ///
    /// Checked **before** the span is scanned, and that ordering is the point. The
    /// per-quantum tally cannot bound this call's work on its own: an event discarded for
    /// a stale epoch or a distant timestamp never reaches the tally, so a producer could
    /// present a million of them and the audio thread would walk every one. The span's
    /// *total* is still not a per-quantum limit — one call may validly cover several
    /// quanta — but it has an absolute ceiling, which is the capacity times the quanta a
    /// call can render.
    #[error("the span presents {presented} events, above the {admissible} a single call can admit")]
    EventSpanTooLarge {
        /// Events the caller presented.
        presented: usize,
        /// The most any one call can admit.
        admissible: usize,
    },

    /// An event is stamped for a quantum this call does not render.
    ///
    /// Phase 1's event input is a **prevalidated bounded span** covering the quanta
    /// this call renders. The renderer owns no future-event store, and dropping an
    /// out-of-span event silently is what ADR-0001 clause 16 forbids — so the span's
    /// contract is enforced instead of quietly bent. Phase 3 presents only the
    /// publication arbiter's sealed batches for the imminent call.
    #[error(
        "event at sample {position} falls in quantum {event_quantum}, outside the quanta \
         {first_quantum}..={last_quantum} this call renders"
    )]
    EventOutsideCallSpan {
        /// The event's render position.
        position: u64,
        /// The quantum it falls in.
        event_quantum: u64,
        /// First quantum this call renders.
        first_quantum: u64,
        /// Last quantum this call renders.
        last_quantum: u64,
    },

    /// The render clock cannot advance.
    ///
    /// ADR-0032 clause 28's terminal fault, with the same shape as the oversized
    /// callback: silence, `needs_reprepare`, a counted diagnostic, and no panic —
    /// because a clock that cannot advance can no longer place an event, and
    /// continuing would break the monotonicity every other clause rests on.
    #[error("the render clock is exhausted: {0}")]
    ClockExhausted(TimeError),

    /// The stream is over and the renderer is waiting to be re-prepared.
    #[error("this stream needs re-preparation before it can render again")]
    NeedsReprepare,
}

/// The counters a phase exit review inspects.
///
/// This is the report ADR-0021 requires drop counts to reach and ADR-0001 clause 16
/// requires the late counter to reach. It exists because of a specific failure:
/// `LIMIT-0013`'s per-priority drop counters were recorded as "published on OSC" for
/// years, and the use-site audit found `get_dropped_counts` has no caller at all
/// while the OSC feed publishes a different ring's counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct DiagnosticsReport {
    late_events: u64,
    stale_epoch_events: u64,
    out_of_horizon_events: u64,
    pre_epoch_clamps: u64,
    arrival_stamped_events: u64,
    foreign_slot_events: u64,
    orphan_note_events: u64,
    last_orphan_note: Option<crate::identity::NoteIdentity>,
    ingress_dropped_slot: u64,
    ingress_dropped_hold: u64,
    ingress_dropped_identity: u64,
    ingress_orphan_releases: u64,
    oversized_callback_faults: u64,
    clock_exhaustion_faults: u64,
    publication_faults: u64,
    displacement_faults: u64,
    late_activations: u64,
    refused_activations: u64,
    needs_reprepare: bool,
}

impl DiagnosticsReport {
    /// Events whose timestamp fell in an already-rendered quantum.
    ///
    /// ADR-0001 clause 16: such an event is clamped to the first not-yet-rendered
    /// quantum boundary and **counted**; it is never applied retroactively and never
    /// silently dropped. The counter's trigger is a *condition*, not a cause — it
    /// does not ask who was at fault.
    pub const fn late_events(&self) -> u64 {
        self.late_events
    }

    /// Events discarded because their epoch is not the renderer's.
    ///
    /// ADR-0032 clause 20. Without this an event stamped microseconds before a
    /// re-preparation would be applied against a clock that restarted at zero,
    /// producing a timing error bounded only by how long the previous stream ran.
    pub const fn stale_epoch_events(&self) -> u64 {
        self.stale_epoch_events
    }

    /// Ingress offers refused for being stamped beyond the forward horizon.
    ///
    /// ADR-0032 clause 21. Holding one would pin a queue slot — and a note-on's hold — for an
    /// unbounded time. It binds ingress provenance only: a compiled list spans the whole
    /// piece, and measuring it against this horizon would reject most of a song.
    ///
    /// **This counts offers the ingress boundary never accepted, and that is a change of
    /// meaning rather than of implementation.** Until 2026-09-01 the renderer evaluated the
    /// horizon and this counted events *it* refused from a span it had been handed;
    /// `HOST-INV-013` evaluates it exactly once, and the maintainer settled that the one site
    /// is `PerformanceIngress::admit`. The renderer's evaluation is retired, so a reader
    /// comparing this figure across that boundary compares two different populations: a
    /// caller-assembled span is no longer measured against the horizon at all.
    pub const fn out_of_horizon_events(&self) -> u64 {
        self.out_of_horizon_events
    }

    /// Ingress timestamps clamped up to the start of the epoch.
    ///
    /// ADR-0032 clause 22's counter. **Phase 1 publishes it and never increments
    /// it**, and that is deliberate rather than an omission: the envelope carries an
    /// unsigned `SampleTime`, so a pre-epoch stamp is unrepresentable by the time it
    /// reaches the renderer. The clamp belongs to the ingress mapper, which is Phase
    /// 3's, along with both of the tests that separate this counter from the late
    /// one.
    pub const fn pre_epoch_clamps(&self) -> u64 {
        self.pre_epoch_clamps
    }

    /// Events whose provenance is `Arrival`.
    ///
    /// ADR-0032 clause 19: an adapter with no hardware timestamp must declare its
    /// arrival-time fallback, and that declaration reaches this report. The
    /// uncertainty is *reported*, never used to move an event — an adapter may not
    /// compensate its own unmeasured error.
    pub const fn arrival_stamped_events(&self) -> u64 {
        self.arrival_stamped_events
    }

    /// Live events dropped at the ingress boundary because the queue was full.
    ///
    /// `HOST-INV-009` licenses a drop at the one registered live renderer-ingress store and
    /// requires the count to reach this report with **the exhausted resource named**, "so
    /// the three causes stay distinguishable". They need different fixes: a full queue is a
    /// producer outrunning the render callback, while the two below are a plan admitted for
    /// fewer notes than the performer plays.
    ///
    /// Mirrored from the store rather than incremented here, because the drop happens on the
    /// producing half **before acceptance**, which is where `HOST-INV-009` puts it and where
    /// this report cannot reach. The drain copies the store's running totals into it.
    pub const fn ingress_dropped_slot(&self) -> u64 {
        self.ingress_dropped_slot
    }

    /// Live note-ons dropped because the producer's release-hold entitlement was outstanding.
    pub const fn ingress_dropped_hold(&self) -> u64 {
        self.ingress_dropped_hold
    }

    /// Live note-ons dropped because the producer's identity range was exhausted.
    pub const fn ingress_dropped_identity(&self) -> u64 {
        self.ingress_dropped_identity
    }

    /// Releases refused at the live boundary for naming no note this producer holds open.
    ///
    /// Distinct from [`Self::orphan_note_events`], and the distinction is the refusal
    /// **point**: this one never entered a queue or spent a share, while that one was
    /// published and reached the renderer. A producer whose releases are refused at the
    /// boundary and one whose releases arrive at notes already gone need different fixes.
    /// Neither consumes `HOST-INV-009`'s drop licence: an orphan is a release for a note
    /// that does not exist, not a shortage.
    pub const fn ingress_orphan_releases(&self) -> u64 {
        self.ingress_orphan_releases
    }

    /// Every drop at the live boundary, whatever the resource.
    ///
    /// Offered beside the three causes rather than instead of them: a report carrying only
    /// the total could not tell a starved producer from an under-admitted plan.
    pub const fn ingress_dropped(&self) -> u64 {
        self.ingress_dropped_slot
            .saturating_add(self.ingress_dropped_hold)
            .saturating_add(self.ingress_dropped_identity)
    }

    /// Events whose parameter slot belongs to another compiled plan.
    ///
    /// A slot is an index into one plan's target table. After a plan swap, events
    /// resolved against the old plan are still in flight, and applying one here would
    /// write whatever occupies that index now — a wrong parameter, silently. Refusing
    /// by identity and counting is the same shape ADR-0032 gives a stale epoch.
    pub const fn foreign_slot_events(&self) -> u64 {
        self.foreign_slot_events
    }

    /// Note edges whose occurrence named no live note.
    ///
    /// `SOUND-INV-017`'s orphan, and ADR-0047 clause 4's three cases all land here: a free
    /// index, a superseded generation, and a retired one. The renderer refuses such an edge
    /// rather than resolving it to another note, and counts it here so the refusal is
    /// observable instead of a silent skip. It is deliberately **not** a drop: `HOST-INV-009`
    /// licenses drops for a shortage, and an orphan is a release for a note that does not
    /// exist. Reporting it as a drop would make a producer look starved when it is not.
    ///
    /// Distinct from [`Self::foreign_slot_events`], which is an event from *another* plan.
    /// An orphan is this stream's own occurrence, spent or superseded.
    pub const fn orphan_note_events(&self) -> u64 {
        self.orphan_note_events
    }

    /// The most recent orphan's occurrence.
    ///
    /// ADR-0047 clause 4 requires an orphan to be counted "against its offering producer with
    /// the identity named". **Naming the identity names the producer**: the ranges are
    /// disjoint by construction and a producer's position in the plan's declaration is its
    /// `ProducerId`, so the index this carries falls in exactly one producer's range. No
    /// producer tag is needed and none is carried.
    ///
    /// One identity rather than all of them, because this report is a fixed-size value the
    /// audio thread writes with no allocation. **Per-producer counts are owed**, and the
    /// count beside the identity is what cannot be attributed: two producers orphaning in one
    /// call are reported as one identity and a total. That is a property of this report's
    /// shape, not of how many producers emit.
    ///
    /// **It is not reachable here yet**, and the reason is a check rather than a habit:
    /// `PerformanceIngress::prepare` refuses a plan that also declares a compiled producer,
    /// because ADR-0051 clause 6 leaves a gate reached by more than one with no ownership
    /// law. Plans declaring both do exist in this crate's fixtures and are harmless — none
    /// builds a live store, and without one a non-compiled declaration cannot emit.
    ///
    /// Three earlier revisions justified the gap by the producer *count*, then by
    /// `stamp_compiled` being the only path that mints into a renderer's table, then by the
    /// disjoint ranges. Independent reviews refuted all three — the second because the live
    /// boundary mints too, through `StreamControl::offer_note_on`, and the third because
    /// disjoint ranges identify the **named** occurrence's producer and say nothing about
    /// the count beside it.
    pub const fn last_orphan_note(&self) -> Option<crate::identity::NoteIdentity> {
        self.last_orphan_note
    }

    /// Callbacks larger than the profile's maximum block.
    pub const fn oversized_callback_faults(&self) -> u64 {
        self.oversized_callback_faults
    }

    /// Times the render clock could not advance.
    pub const fn clock_exhaustion_faults(&self) -> u64 {
        self.clock_exhaustion_faults
    }

    /// Publications the arbiter could not seal.
    ///
    /// ADR-0046 clause 7 requires the terminal response to increment an **attributable**
    /// counter, not merely to silence: a stream that ended for a contract violation and one
    /// that ended for any other reason are indistinguishable without it, and the violation
    /// is the only one of the two a producer can be told to fix.
    ///
    /// At most one per epoch, because every producer refuses to publish into an epoch that
    /// has already faulted — a guarantee the producers keep, not one this counter enforces.
    pub const fn publication_faults(&self) -> u64 {
        self.publication_faults
    }

    /// Streams ended because an activation's displacement left engine time.
    ///
    /// Separate from [`Self::publication_faults`] because publication did not fail: the
    /// condition is found before a window is opened, and a reader sent to look for a share
    /// overrun would find none. Both are terminal; only one is a producer's contract
    /// violation.
    pub const fn displacement_faults(&self) -> u64 {
        self.displacement_faults
    }

    /// Activations offered after the engine time they named had already passed.
    ///
    /// ADR-0050 clause 1. Building a candidate is off-thread work of unbounded duration, so
    /// one can be finished too late; it then activates at the clock, and this counter is what
    /// makes a seek that felt sluggish attributable rather than leaving a delay no one can
    /// see.
    ///
    /// **The condition is `requested < clock` at the offer, and it is not "took effect later
    /// than requested".** Every request that does not fall on a quantum boundary takes effect
    /// later than it named, because the effective point snaps forward — a `T` of 65 offered
    /// against a clock of zero activates at 128 and is **not** counted here, correctly, since
    /// nothing was slow. Lateness and displacement are independent and neither implies the
    /// other: an off-grid request offered against a clock already standing on its own snapped
    /// boundary is late and displaced by nothing at all.
    pub const fn late_activations(&self) -> u64 {
        self.late_activations
    }

    /// Activations refused at the offer.
    ///
    /// A stream that has already faulted, a stale epoch, unsupported loop playback, a
    /// superseded sequence, or an occupied exchange slot. **A schedule paired with another
    /// stream's renderer refuses too and is deliberately not counted here**: these counters
    /// belong to the stream that was offered to, and that refusal has no such stream. None of
    /// the counted ones is a fault — ADR-0050 clause 3 puts every refusal at the offer so
    /// adoption has no branch that can fail — but a stream that silently refuses every seek and one
    /// that adopts them are indistinguishable without this.
    pub const fn refused_activations(&self) -> u64 {
        self.refused_activations
    }

    /// Whether the stream is waiting to be re-prepared.
    pub const fn needs_reprepare(&self) -> bool {
        self.needs_reprepare
    }

    pub(crate) fn count_late_event(&mut self) {
        self.late_events = self.late_events.saturating_add(1);
    }

    pub(crate) fn count_stale_epoch_event(&mut self) {
        self.stale_epoch_events = self.stale_epoch_events.saturating_add(1);
    }

    pub(crate) fn count_arrival_stamped_event(&mut self) {
        self.arrival_stamped_events = self.arrival_stamped_events.saturating_add(1);
    }

    pub(crate) fn count_foreign_slot_event(&mut self) {
        self.foreign_slot_events = self.foreign_slot_events.saturating_add(1);
    }

    pub(crate) fn count_orphan_note_event(&mut self, identity: crate::identity::NoteIdentity) {
        self.orphan_note_events = self.orphan_note_events.saturating_add(1);
        self.last_orphan_note = Some(identity);
    }

    pub(crate) fn count_oversized_callback(&mut self) {
        self.oversized_callback_faults = self.oversized_callback_faults.saturating_add(1);
    }

    pub(crate) fn count_clock_exhaustion(&mut self) {
        self.clock_exhaustion_faults = self.clock_exhaustion_faults.saturating_add(1);
    }

    pub(crate) fn count_publication_fault(&mut self) {
        self.publication_faults = self.publication_faults.saturating_add(1);
    }

    pub(crate) fn count_displacement_fault(&mut self) {
        self.displacement_faults = self.displacement_faults.saturating_add(1);
    }

    pub(crate) fn count_late_activation(&mut self) {
        self.late_activations = self.late_activations.saturating_add(1);
    }

    pub(crate) fn count_refused_activation(&mut self) {
        self.refused_activations = self.refused_activations.saturating_add(1);
    }

    /// Mirror an ingress store's running boundary counts into this report.
    ///
    /// **Assignment, not accumulation**, and the two are not interchangeable: the store's
    /// counters are cumulative over the stream, so adding them each pass would count every
    /// earlier drop again on every callback. One store per report is what makes assignment
    /// correct; a second live store needs its own registry row under `HOST-INV-009`, and
    /// this method would need to name which store it is carrying.
    pub(crate) const fn mirror_ingress_boundary(
        &mut self,
        slot: u64,
        hold: u64,
        identity: u64,
        orphans: u64,
        beyond_horizon: u64,
    ) {
        self.ingress_dropped_slot = slot;
        self.ingress_dropped_hold = hold;
        self.ingress_dropped_identity = identity;
        // **The horizon count moved halves rather than disappearing.** It used to be the
        // renderer's, counting events it refused from a span it was handed; `HOST-INV-013`'s
        // single evaluation is now at ingress admission, so the boundary is what counts and
        // the public accessor keeps its name. A reader of an older report saw refusals of
        // events that had already been assembled; this one sees offers that were never
        // accepted.
        self.out_of_horizon_events = beyond_horizon;
        // **The orphan keeps its own field, because that condition has two sites.** This one
        // refused before acceptance, where nothing entered a queue and no share was spent;
        // the renderer's refused an event it was handed. Adding them together would leave a
        // reader unable to tell a producer whose releases never enter the stream from one
        // whose releases arrive at notes that are already gone, and those need different
        // fixes.
        self.ingress_orphan_releases = orphans;
    }

    pub(crate) fn set_needs_reprepare(&mut self) {
        self.needs_reprepare = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_start_at_zero_and_saturate_rather_than_wrap() {
        let mut report = DiagnosticsReport::default();
        assert_eq!(report.late_events(), 0);
        assert!(!report.needs_reprepare());

        report.count_late_event();
        assert_eq!(report.late_events(), 1);

        report.late_events = u64::MAX;
        report.count_late_event();
        assert_eq!(
            report.late_events(),
            u64::MAX,
            "a counter must saturate: wrapping to zero would read as 'nothing happened'"
        );
    }

    #[test]
    fn the_pre_epoch_counter_exists_and_stays_at_zero_in_this_phase() {
        // Published, never incremented here. Phase 3's ingress mapper is what can
        // observe a pre-zero stamp at all.
        let report = DiagnosticsReport::default();
        assert_eq!(report.pre_epoch_clamps(), 0);
    }

    #[test]
    fn a_limit_refusal_names_the_field_both_amounts_and_the_object() {
        use crate::ir::NodeId;
        use crate::quantities::NodeCount;

        let error = CompileError::LimitExceeded {
            field: ResourceField::MaxNodes,
            requested: ResourceAmount::Nodes(NodeCount::measured(5)),
            available: ResourceAmount::Nodes(NodeCount::measured(4)),
            responsible: IrObject::Node(NodeId::new(3)),
        };
        let rendered = error.to_string();
        for expected in ["max_nodes", "5 nodes", "4 nodes", "node 3"] {
            assert!(
                rendered.contains(expected),
                "a refusal must name {expected}; it said {rendered:?}"
            );
        }
    }
}
