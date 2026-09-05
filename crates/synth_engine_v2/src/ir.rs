//! The compiler IR: what a plan says about itself before it is admitted.
//!
//! The master plan's layer boundaries fix what belongs here: resolved stable node
//! and parameter IDs, explicit signal domain and scope, explicit source and
//! destination bindings — and **no GUI layout or transport protocol objects**. The
//! IR is also where a plan *declares* what it will need, because admission has to
//! be able to refuse a plan before anything is prepared.
//!
//! Phase 1's node vocabulary is deliberately tiny: the sources its exit gate names
//! plus an output. Graph validation, topological scheduling, the buffer arena, and
//! prepared/mutable state separation are Phase 2's, and nothing here should
//! anticipate them.

use crate::quantities::{
    Amplitude, BusCount, CostRatio, CutoffFrequency, EventCount, Frequency, GainFactor,
    HeldNoteCount, InstructionCount, MixChannelCount, NodeCount, NormalizedLevel, PreparedBytes,
    RecordCount, Resonance, ScriptWorkPerQuantum, Seconds, SendCount, SlotCount, VoiceCount,
    WritesPerNote,
};
use crate::time::PlanPosition;
use crate::tuning::PreparedTuning;
use thiserror::Error;

/// Define a stable typed identifier.
///
/// These are **identities, not positions**. A node's identifier does not change
/// when the node list is reordered, which is what keeps a persisted or reported
/// reference meaning the same thing.
macro_rules! typed_id {
    ($name:ident, $inner:ty, $label:literal, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[must_use]
        pub struct $name($inner);

        impl $name {
            /// The first identifier of this kind.
            pub const FIRST: Self = Self(0);

            /// Name an identifier.
            pub const fn new(raw: $inner) -> Self {
                Self(raw)
            }

            /// The raw identifier.
            pub const fn as_raw(self) -> $inner {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{} {}", $label, self.0)
            }
        }
    };
}

typed_id!(NodeId, u32, "node", "A node's stable identity.");
typed_id!(PortId, u16, "port", "A port's identity within its node.");
typed_id!(EdgeId, u32, "edge", "An edge's stable identity.");
typed_id!(
    ParameterId,
    u16,
    "parameter",
    "A parameter's identity within its node."
);
typed_id!(BufferId, u32, "buffer", "A buffer's identity in the plan.");
typed_id!(ProgramId, u32, "program", "A script program's identity.");

/// What kind of signal crosses an edge.
///
/// The master plan's initial domains. Conversions between them are compiler
/// operations with documented latency and cost, and an invalid implicit conversion
/// is a compile error — Phase 2 inserts them; Phase 1 only carries the domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalDomain {
    /// Sample streams with a declared channel layout and rate.
    Audio,
    /// Constant, ramp, or bounded control-rate stream.
    Control,
    /// Sample-timed transitions with conventional low/high semantics.
    Gate,
    /// Timed note, controller, or transport events.
    Event,
}

impl std::fmt::Display for SignalDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Audio => f.write_str("audio"),
            Self::Control => f.write_str("control"),
            Self::Gate => f.write_str("gate"),
            Self::Event => f.write_str("event"),
        }
    }
}

/// Where in the scope hierarchy a node runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionScope {
    /// Once per plan.
    Global,
    /// Once per bus.
    Bus,
    /// Once per channel or track.
    Channel,
    /// Once per instrument instance.
    InstrumentInstance,
    /// Once per voice.
    Voice,
}

/// An object a diagnostic can point at.
///
/// `HOST-INV-007` requires a refusal to name "the authored object responsible",
/// and this is what that means in a phase whose IR is built directly: a stable
/// identity, never an index into a list and never a user-facing name, which the
/// hot path must not carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrObject {
    /// The plan as a whole, where no single object dominates the request.
    Plan,
    /// One node.
    Node(NodeId),
    /// One output port of one node.
    Port(NodeId, PortId),
    /// One edge.
    Edge(EdgeId),
    /// One script program.
    Program(ProgramId),
}

impl std::fmt::Display for IrObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Plan => f.write_str("the plan"),
            Self::Node(id) => write!(f, "{id}"),
            Self::Port(node, port) => write!(f, "{node} {port}"),
            Self::Edge(id) => write!(f, "{id}"),
            Self::Program(id) => write!(f, "{id}"),
        }
    }
}

/// What a Phase 1 node does.
///
/// The four sources are the ones this phase's gate and risk controls need: an
/// empty plan and a constant or sine source have to render deterministically, and
/// the impulse is what makes ADR-0001's offline alignment control testable — a
/// render path that forgets the trim emits audio shifted by `Q` frames, which is
/// still valid audio and which no listening test would catch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IrNodeKind {
    /// Zeros.
    Silence,
    /// A constant level on every sample.
    Constant {
        /// The level.
        level: Amplitude,
    },
    /// A sine, from a phase accumulator.
    ///
    /// Both controls are validated types rather than raw floats. A `NaN` reaching the
    /// phase accumulator is unrecoverable — every later sample is `NaN` and no later event
    /// fixes it — so it is refused where the value is built, which is the only place a
    /// diagnostic can be produced at all.
    Sine {
        /// Starting frequency.
        frequency: Frequency,
        /// Peak amplitude.
        amplitude: Amplitude,
    },
    /// A band-limited sawtooth, from the same phase accumulator a sine uses.
    ///
    /// Its own kind rather than a `waveform` field on [`Self::Sine`], because `SOUND-INV-013`
    /// forbids a kernel taking a parameter that selects between laws — and a shape selector is
    /// exactly that. Two kinds, two kernels, two sets of checks.
    ///
    /// **Band-limited by PolyBLEP**, a residual subtracted around the discontinuity rather
    /// than a filter over the result. A naive ramp folds every harmonic above Nyquist back
    /// into the audible band, and at the pitches a project authors that is not a subtle
    /// defect: it is a spray of inharmonic partials that move the wrong way as the pitch
    /// rises.
    Saw {
        /// Starting frequency.
        frequency: Frequency,
        /// Peak amplitude. The ramp runs between its negation and it.
        amplitude: Amplitude,
    },
    /// One sample of `1.0` at a declared plan position, zero elsewhere.
    Impulse {
        /// Where in the plan the click is.
        position: PlanPosition,
    },
    /// A constant gain applied to one mono input.
    ///
    /// The first kind with an **input** port, which is what makes Phase 2's
    /// validation rules testable at all: without one, no plan can have a fan-in, a
    /// cycle, a layout mismatch, or a path from a source to an output through
    /// anything. It stays inside the existing operation enum on purpose —
    /// ADR-0004 owns the node-representation question and is `Proposed`, so this
    /// phase's validation work may not pre-empt it.
    Gain {
        /// The factor applied to every sample.
        factor: GainFactor,
    },
    /// A four-segment envelope, gated by a note edge.
    ///
    /// It produces a **control** signal rather than audio, and it produces one value per
    /// *sample* rather than one per quantum. An envelope that only moved at quantum
    /// boundaries would step in 1.3 ms stairs at 48 kHz — audible on every note, and a
    /// difference from V1 that no record asks for.
    ///
    /// Its gate is the phase's one **sample-positioned** control: ADR-0001 clause 14, as
    /// ADR-0043 restated it, puts a note-on, note-off, gate or retrigger at the offset its
    /// render position names within the quantum that renders it — the declared sample
    /// unless the late clamp moved it — and since P02-T007 that is where it lands. The split is a property of the effect
    /// rather than of the message, so the gate behaves the same whether it is played as a
    /// note or addressed as a parameter; clause 13's causality still governs every
    /// *control-rate* change, which takes effect at the next boundary.
    Envelope {
        /// How long silence takes to reach full level.
        attack: Seconds,
        /// How long full level takes to reach the sustain level.
        decay: Seconds,
        /// The level a held gate settles at.
        sustain: NormalizedLevel,
        /// How long the level takes to reach silence once the gate falls.
        release: Seconds,
    },
    /// A two-pole low-pass over one mono input.
    ///
    /// The first kind whose prepared data is *derived* rather than copied: its
    /// coefficients are a function of the corner frequency, the quality factor **and
    /// the stream's sample rate**, which is why admission is where they are computed and
    /// why a corner frequency above the stream's Nyquist frequency is refused there
    /// rather than clamped here.
    ///
    /// It has no controls, and that is a consequence of the split rather than an
    /// omission: a kernel is handed `&PreparedNode`, so a parameter that moves the
    /// coefficients would have to move data the kernel cannot write. Where recomputation
    /// belongs — the state, a ramp, a control-rate law — is Phase 5's, and inventing it
    /// here would be deciding it in an implementation task.
    Filter {
        /// The corner frequency.
        cutoff: CutoffFrequency,
        /// The quality factor.
        resonance: Resonance,
    },
    /// One audio input scaled, sample by sample, by one control input.
    ///
    /// Distinct from [`Self::Gain`], which multiplies by a constant the plan carries.
    /// This one has no value of its own: what it does is decided by whatever is patched
    /// into its control port, which is how an envelope becomes an amplitude.
    ///
    /// Either input unpatched makes it silent. That is the same rule as everywhere else
    /// in this crate — an unpatched input is silence — and the alternative would be to
    /// invent a level nobody asked for.
    Amplifier,
    /// A pass-through that declares an observation tap on its output (`SOUND-INV-022`).
    ///
    /// The one authored way a plan carries a tap: a monitor placed where a project wants
    /// to observe a signal. It changes no sample — its output is its input — and its
    /// declaration is the single source of the tap's data type, rate and cost.
    Monitor,
    /// The plan's output. Takes one mono signal and writes every output channel.
    ///
    /// Writing a mono source to both channels is a **declared duplication** rather
    /// than a conversion the compiler inserted: inserting implicit conversions is
    /// Phase 2's work, and pretending to do it here would preempt it.
    Output,
}

impl IrNodeKind {
    /// Whether this kind produces a signal.
    #[must_use]
    pub const fn is_source(self) -> bool {
        !matches!(self, Self::Output)
    }
}

/// Parameter identities a node kind exposes.
///
/// **When** moving one takes effect is the node kind's declaration rather than
/// this module's: ADR-0001 clause 13 makes control evaluation causal, so a
/// control-rate change inside a quantum takes effect at the next boundary, while
/// clause 14, as ADR-0043 restated it, puts a sample-positioned one — a gate — at
/// the offset its render position names.
/// [`crate::plan::ControlRate`] is where that is compiled, and the renderer reads
/// it there rather than inferring it from the payload a caller chose.
pub mod parameters {
    use super::ParameterId;

    /// A sine's frequency in hertz.
    ///
    /// **Sample-positioned**, because `SOUND-INV-021` makes it a pitch destination: a note's
    /// key has to be in force at the sample that note's gate rises. A destination has one
    /// timing whichever payload addressed it, so automating a frequency and playing a note
    /// reach it under the same law.
    pub const SINE_FREQUENCY: ParameterId = ParameterId::new(0);
    /// A sine's peak amplitude. Control-rate.
    pub const SINE_AMPLITUDE: ParameterId = ParameterId::new(1);
    /// A sawtooth's frequency in hertz. Sample-positioned, as the sine's is and for the
    /// same reason.
    ///
    /// A `ParameterId` is scoped to its node, so sharing a number with the sine's is not a
    /// collision — it is the same statement about a different kind.
    pub const SAW_FREQUENCY: ParameterId = ParameterId::new(0);
    /// A sawtooth's peak amplitude. Control-rate.
    pub const SAW_AMPLITUDE: ParameterId = ParameterId::new(1);
    /// An envelope's gate: above zero is held, zero or below is released.
    ///
    /// **Sample-positioned.** It is the same control a note edge moves, so
    /// automating a gate and playing a note reach it under one timing law.
    pub const ENVELOPE_GATE: ParameterId = ParameterId::new(2);
    /// An envelope's velocity: the scale its emitted level is multiplied by.
    ///
    /// **Sample-positioned**, and `SOUND-INV-021`'s velocity destination. Addressable as a
    /// parameter for the reason the gate is: one destination, one timing, whichever payload
    /// moved it.
    pub const ENVELOPE_VELOCITY: ParameterId = ParameterId::new(3);
}

/// One node in the IR.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct IrNode {
    id: NodeId,
    kind: IrNodeKind,
    scope: ExecutionScope,
}

impl IrNode {
    /// A node.
    pub const fn new(id: NodeId, kind: IrNodeKind, scope: ExecutionScope) -> Self {
        Self { id, kind, scope }
    }

    /// This node's stable identity.
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// What the node does.
    pub const fn kind(&self) -> IrNodeKind {
        self.kind
    }

    /// Where the node runs.
    pub const fn scope(&self) -> ExecutionScope {
        self.scope
    }
}

/// One edge in the IR: an explicit source and destination binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct IrEdge {
    id: EdgeId,
    from: (NodeId, PortId),
    to: (NodeId, PortId),
    domain: SignalDomain,
}

impl IrEdge {
    /// An edge.
    pub const fn new(
        id: EdgeId,
        from: (NodeId, PortId),
        to: (NodeId, PortId),
        domain: SignalDomain,
    ) -> Self {
        Self {
            id,
            from,
            to,
            domain,
        }
    }

    /// This edge's stable identity.
    pub const fn id(&self) -> EdgeId {
        self.id
    }

    /// The source node and port.
    pub const fn from(&self) -> (NodeId, PortId) {
        self.from
    }

    /// The destination node and port.
    pub const fn to(&self) -> (NodeId, PortId) {
        self.to
    }

    /// What crosses the edge.
    pub const fn domain(&self) -> SignalDomain {
        self.domain
    }
}

/// One script program's declared work.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct IrProgram {
    id: ProgramId,
    instructions: InstructionCount,
    sources: SlotCount,
    state_slots: SlotCount,
    locals: SlotCount,
    eval_stack_depth: SlotCount,
    arrays: SlotCount,
    array_elements: SlotCount,
    emits: SlotCount,
    /// How many scopes evaluate this program each quantum.
    ///
    /// Instructions times scopes times polyphony is the quantity that actually
    /// costs CPU, and it is **reported without a threshold** until Phase 7 can
    /// measure a per-instruction cost that would justify one. A limit with no
    /// value is not a limit.
    evaluations_per_quantum: u32,
}

impl IrProgram {
    /// A program's declaration.
    #[allow(
        clippy::too_many_arguments,
        reason = "one argument per per-program script capacity the profile bounds"
    )]
    pub const fn new(
        id: ProgramId,
        instructions: InstructionCount,
        sources: SlotCount,
        state_slots: SlotCount,
        locals: SlotCount,
        eval_stack_depth: SlotCount,
        arrays: SlotCount,
        array_elements: SlotCount,
        emits: SlotCount,
        evaluations_per_quantum: u32,
    ) -> Self {
        Self {
            id,
            instructions,
            sources,
            state_slots,
            locals,
            eval_stack_depth,
            arrays,
            array_elements,
            emits,
            evaluations_per_quantum,
        }
    }

    /// This program's identity.
    pub const fn id(&self) -> ProgramId {
        self.id
    }

    /// Instructions in the program.
    pub const fn instructions(&self) -> InstructionCount {
        self.instructions
    }

    /// Sources the program reads.
    pub const fn sources(&self) -> SlotCount {
        self.sources
    }

    /// State slots the program keeps.
    pub const fn state_slots(&self) -> SlotCount {
        self.state_slots
    }

    /// Locals the program declares.
    pub const fn locals(&self) -> SlotCount {
        self.locals
    }

    /// The program's evaluation-stack depth.
    pub const fn eval_stack_depth(&self) -> SlotCount {
        self.eval_stack_depth
    }

    /// Arrays the program declares.
    pub const fn arrays(&self) -> SlotCount {
        self.arrays
    }

    /// Elements in the program's largest array.
    pub const fn array_elements(&self) -> SlotCount {
        self.array_elements
    }

    /// Emit slots the program drives.
    pub const fn emits(&self) -> SlotCount {
        self.emits
    }

    /// How many evaluations of this program happen per quantum.
    pub const fn evaluations_per_quantum(&self) -> u32 {
        self.evaluations_per_quantum
    }
}

/// What a plan declares it will need, beyond what its nodes and edges show.
///
/// Every field defaults to nothing, so a plan requests only what it says it
/// requests. Phase 1's IR is built directly in tests; the phases that lower a real
/// project compute these from its content.
///
/// What one note-on source may hold at once.
///
/// Two numbers rather than one, because they bound different resources and ADR-0046 gives
/// them different owners. `simultaneous_notes` sizes the source's **identity range**, which
/// every note-on consumes; `simultaneous_holds` sizes its **hold entitlement**, which only a
/// note-on whose release is not already in the same sealed batch consumes. Holds are
/// therefore at most notes, and a compiled source declares none at all — ADR-0046 clause 6:
/// "Compiled releases use plan entitlements and need no hold."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct NoteProducerDeclaration {
    /// Whether this source's releases are already in the plan.
    pub compiled: bool,
    /// Notes it may have sounding at once. Sizes its identity range.
    pub simultaneous_notes: HeldNoteCount,
    /// Release obligations it may hold at once. Zero for a compiled source.
    pub simultaneous_holds: EventCount,
}

/// The `Default` is written out rather than derived, because deriving it would
/// require a `Default` on every quantity type — and a capacity type whose default
/// is zero is the shape the two-constructor rule exists to prevent. Here the zeros
/// are *measurements*, which is a different claim.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct PlanDeclarations {
    /// Every source that can start a note, and what each may hold at once.
    ///
    /// ADR-0046 partitions hold entitlements "at plan admission" across "every admitted
    /// non-compiled note-on producer", and ADR-0047 clause 3 partitions identity ranges
    /// across a **superset** of those — every note-on producer, compiled included, because
    /// every note-on needs an occurrence while only some need a hold. Neither partition can
    /// be computed from a plan that does not say who its producers are, which is what this
    /// field supplies.
    ///
    /// An empty list is a plan that starts no notes, which is ordinary: a plan can be pure
    /// automation.
    pub note_producers: Vec<NoteProducerDeclaration>,
    /// Notes held at once across the plan.
    pub held_notes: HeldNoteCount,
    /// Mix channels.
    pub mix_channels: MixChannelCount,
    /// Buses.
    pub buses: BusCount,
    /// The most sends any one channel has.
    pub max_sends_on_any_channel: SendCount,
    /// Events the plan is known to place in one quantum.
    ///
    /// **Compiled work, and admitted against `compiled_event_share`.** Statically knowable
    /// events are the compiled producer's; ADR-0046 clause 5 admits data-dependent
    /// expansion separately, against its own declared envelopes. When an authored runtime
    /// source arrives, this field does not become the aggregate — an aggregate without
    /// producer attribution could not be checked against any share.
    ///
    /// Statically knowable only. Phase 1 and Phase 2 reject an over-full caller span
    /// before mutation; Phase 3 admits data-dependent expansion against ADR-0046's
    /// destination, future-storage, and release-hold envelopes.
    pub events_per_quantum: EventCount,
    /// Events one tick's note expansion may produce.
    pub note_expansion_per_tick: EventCount,
    /// Events the scheduler holds in its release window.
    pub scheduled_events_in_flight: EventCount,
    /// Nodes in the largest modulation graph.
    pub mod_graph_nodes: NodeCount,
    /// Nodes in the largest note graph.
    pub note_graph_nodes: NodeCount,
    /// Notes one take may hold.
    pub held_notes_per_take: HeldNoteCount,
    /// Events one take may record.
    pub recorded_events_per_take: EventCount,
    /// Script programs.
    pub programs: Vec<IrProgram>,
    /// Voices any one instrument declares.
    pub voices_per_instrument: VoiceCount,
    /// Every authored runtime source this plan admits, with its ADR-0046 clause 5 envelopes.
    ///
    /// Empty is ordinary and is what every plan in this phase declares: no authored producer
    /// exists yet. The field is not speculation about one — it is what lets admission refuse
    /// a plan that declares more than its share allows, which `HOST-INV-007` requires of the
    /// `authored_runtime_event_share` row and which a share compared against itself can never
    /// do.
    pub authored_sources: Vec<AuthoredSourceDeclaration>,
    /// Every renderer-internal producer this plan admits, with its per-quantum maximum.
    ///
    /// Empty for the same reason, and with the same purpose.
    pub internal_producers: Vec<InternalProducerDeclaration>,
}

/// One admitted authored runtime source's conservative envelopes.
///
/// ADR-0046 clause 5 admits data-dependent expansion against declarations rather than
/// against what a source turns out to emit: a runtime producer cannot be trusted to stay
/// inside a share it never stated. The three envelopes are the three ways such a source can
/// outgrow the partition — too many events at one destination, too much retained future, or
/// too many open release obligations — and each is checked separately because a source can
/// fit one and exceed another.
///
/// **The `producer` link is what makes the holds checkable**, and its absence was a contract
/// hole an independent review found in an earlier attempt at this type: without it nothing
/// tied a source to the `note_producers` entry carrying its `ProducerId` and its hold
/// entitlement, so the compiler could not verify that an authored note source had declared
/// one at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct AuthoredSourceDeclaration {
    /// The note producer this source expands through.
    ///
    /// It must name a declared **non-compiled** producer: ADR-0046 clause 6 gives compiled
    /// releases plan entitlements and no hold, so an authored source routed through a
    /// compiled producer would take holds against an entitlement of zero.
    pub producer: crate::identity::ProducerId,
    /// Most events it may place at any one destination quantum.
    pub destination_occupancy: EventCount,
    /// Most future events it may simultaneously retain.
    pub retained_future: EventCount,
    /// Most release obligations it may hold at once.
    ///
    /// Bounded by the named producer's own entitlement, which is where the disjointness
    /// ADR-0046 clause 6 requires is enforced.
    pub simultaneous_holds: EventCount,
}

/// One admitted renderer-internal producer's per-quantum maximum.
///
/// ADR-0046 clause 2 confines a renderer-internal emission to the quantum that generates it,
/// which is exactly what makes a single declared per-quantum maximum a **complete** bound
/// rather than a rate below which occupancy could still accumulate. A producer that needed a
/// future target would first need clause 5's envelopes, which is a change to that record
/// rather than a declaration this type could carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct InternalProducerDeclaration {
    /// Most events it may emit into the quantum that generates them.
    pub per_quantum: EventCount,
}

impl Default for PlanDeclarations {
    fn default() -> Self {
        Self {
            // Empty rather than one nominal producer: a plan that declares nothing starts no
            // notes, and inventing a producer here would give admission something to
            // partition that the plan never asked for.
            note_producers: Vec::new(),
            held_notes: HeldNoteCount::NONE,
            mix_channels: MixChannelCount::NONE,
            buses: BusCount::NONE,
            max_sends_on_any_channel: SendCount::NONE,
            events_per_quantum: EventCount::NONE,
            note_expansion_per_tick: EventCount::NONE,
            scheduled_events_in_flight: EventCount::NONE,
            mod_graph_nodes: NodeCount::NONE,
            note_graph_nodes: NodeCount::NONE,
            held_notes_per_take: HeldNoteCount::NONE,
            recorded_events_per_take: EventCount::NONE,
            programs: Vec::new(),
            voices_per_instrument: VoiceCount::NONE,
            // Empty for the same reason `note_producers` is: a plan that declares no
            // authored or internal producer has none, and inventing one here would give
            // admission an envelope to partition that the plan never asked for.
            authored_sources: Vec::new(),
            internal_producers: Vec::new(),
        }
    }
}

/// An IR that could not be assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IrError {
    /// Two nodes claim one identity.
    #[error("{id} is declared twice")]
    DuplicateNode {
        /// The repeated identity.
        id: NodeId,
    },
    /// An edge names a node that is not in the plan.
    #[error("{edge} refers to {node}, which the plan does not declare")]
    UnknownNode {
        /// The edge holding the dangling reference.
        edge: EdgeId,
        /// The node it names.
        node: NodeId,
    },
    /// An edge leaves a node that produces nothing.
    #[error("{edge} leaves {node}, which is not a source")]
    NotASource {
        /// The offending edge.
        edge: EdgeId,
        /// The node it leaves.
        node: NodeId,
    },
    /// Two tunings claim one execution scope.
    ///
    /// `SOUND-INV-021` requires every node of one scope to reference the same prepared
    /// tuning, so two entries for one scope have no reading — and picking the last would
    /// let declaration order decide which scale the plan sounds in.
    #[error("{scope:?} is given a tuning twice")]
    DuplicateScopeTuning {
        /// The scope declared twice.
        scope: ExecutionScope,
    },
}

/// What one execution scope declares, accumulated in one pass over a plan's nodes.
///
/// `SOUND-INV-021`'s two report figures are both per scope, and computing either by asking
/// each node about its scope is quadratic in a plan the profile admits.
#[derive(Debug, Clone, Copy)]
struct ScopeSummary {
    scope: ExecutionScope,
    /// Whether any node of the scope can be sent a note.
    playable: bool,
    /// How many pitch and velocity destinations its kinds declare, together.
    magnitudes: u32,
    /// How many of those are pitch destinations, which are what reference a tuning.
    pitch_destinations: u32,
}

/// A plan, as the compiler receives it.
#[derive(Debug, Clone, PartialEq, Default)]
#[must_use]
pub struct GraphIr {
    nodes: Vec<IrNode>,
    edges: Vec<IrEdge>,
    declarations: PlanDeclarations,
    tunings: Vec<ScopeTuning>,
}

/// The tuning one execution scope resolves its keys through.
///
/// `SOUND-INV-021` puts the reference on the node and requires every node of one scope to
/// share it, so the plan states it **per scope** and admission hands each pitch-producing
/// node the reference its scope names. That is what keeps a scope from resolving two keys
/// two ways while leaving two scopes free to use two scales.
///
/// A scope with no entry is not given a fallback. Admission refuses a plan whose note scope
/// declares a pitch destination and no tuning, because choosing one here would be this
/// crate deciding what a project sounds like — which is the authored model's job and Phase
/// 10A's.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct ScopeTuning {
    scope: ExecutionScope,
    tuning: PreparedTuning,
}

impl ScopeTuning {
    /// Bind a prepared tuning to a scope.
    pub const fn new(scope: ExecutionScope, tuning: PreparedTuning) -> Self {
        Self { scope, tuning }
    }

    /// Which scope resolves through it.
    pub const fn scope(&self) -> ExecutionScope {
        self.scope
    }

    /// The prepared table.
    pub const fn tuning(&self) -> &PreparedTuning {
        &self.tuning
    }
}

impl GraphIr {
    /// An empty plan.
    ///
    /// Not a degenerate case to be rejected: the Phase 1 exit gate requires an
    /// empty plan to render deterministically, which it does, as silence.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Start building.
    pub fn builder() -> GraphIrBuilder {
        GraphIrBuilder {
            nodes: Vec::new(),
            edges: Vec::new(),
            declarations: PlanDeclarations::default(),
            tunings: Vec::new(),
            next_edge: 0,
        }
    }

    /// The tuning each scope resolves through, in declaration order.
    pub fn tunings(&self) -> &[ScopeTuning] {
        &self.tunings
    }

    /// The tuning `scope` resolves through, if the plan states one.
    #[must_use]
    pub fn tuning_of(&self, scope: ExecutionScope) -> Option<&PreparedTuning> {
        self.tunings
            .iter()
            .find(|entry| entry.scope() == scope)
            .map(ScopeTuning::tuning)
    }

    /// The plan's nodes, in declaration order.
    pub fn nodes(&self) -> &[IrNode] {
        &self.nodes
    }

    /// The plan's edges, in declaration order.
    pub fn edges(&self) -> &[IrEdge] {
        &self.edges
    }

    /// What the plan declares it needs.
    pub const fn declarations(&self) -> &PlanDeclarations {
        &self.declarations
    }

    /// The node with this identity, if the plan has it.
    pub fn node(&self, id: NodeId) -> Option<&IrNode> {
        self.nodes.iter().find(|node| node.id() == id)
    }

    /// The immutable prepared bytes this plan's nodes occupy, with the node responsible.
    ///
    /// Counted over the nodes that are **scheduled**, which is not every node: the output
    /// has no kernel and no prepared data — it is the renderer's boundary — so lowering
    /// builds no record for it and neither does this. Counting it would overstate both
    /// memory rows and could refuse a plan whose limit falls between the two figures.
    ///
    /// `inserted` is how many records the **compiler** adds beyond the authored nodes —
    /// a mono-to-stereo widening is a scheduled operation with prepared data of its own,
    /// and a report that counted only what the caller wrote would understate the table
    /// that is actually allocated. Admission passes the exact figure once lowering has
    /// produced it, and an upper bound before that, exactly as it does for the arena.
    ///
    /// The amount is measured from the representation ADR-0004 selected: every node gets
    /// one record of the widest variant. The **contributor** is therefore the node that
    /// sets that width, not one that carries more records than the others.
    pub fn prepared_bytes(&self, inserted: u64) -> (PreparedBytes, IrObject) {
        self.aggregate_bytes(
            crate::node::prepared_bytes_per_node(),
            crate::node::prepared_payload_bytes,
            inserted,
        )
    }

    /// The bytes this plan's prepared tunings add to its immutable prepared total.
    ///
    /// **Exact, and it has to be**: the preflight report is allowed to *refuse* on the
    /// prepared row — only the arena row and what follows it are exempt — so an upper bound
    /// here would reject a plan that fits. An independent review found exactly that.
    ///
    /// Exactness comes from computing it over the same set admission binds magnitudes over:
    /// **the scopes holding a playable node**, which one linear pass over the nodes
    /// establishes. A tuning declared for a scope no note reaches is never prepared and is not
    /// charged, and a pitch destination outside those scopes is not a reference.
    /// `the_reported_tuning_charge_is_what_the_plan_holds` ties this figure to the tables the
    /// compiled plan actually carries, so the two cannot drift.
    ///
    /// The split is `SOUND-INV-021`'s and it is the point of the row: one table per
    /// **distinct** tuning, plus one reference per pitch destination — so a second scale
    /// reads as a table and a second node as a reference.
    pub fn tuning_bytes(&self) -> u64 {
        let mut distinct: Vec<&PreparedTuning> = Vec::new();
        let mut references = 0_u64;
        for summary in self.scope_summaries() {
            if !summary.playable || summary.pitch_destinations == 0 {
                continue;
            }
            references = references.saturating_add(u64::from(summary.pitch_destinations));
            let Some(tuning) = self.tuning_of(summary.scope) else {
                // Admission refuses this plan; the row still owes a figure, and the
                // references are real whether or not a table backs them.
                continue;
            };
            // Compared by **content**, not by digest: a digest is a 64-bit hash, and two
            // scales colliding on one would be charged as a single table here while
            // resolving through a single table there. At most one comparison per scope, and
            // a plan has five.
            if !distinct.contains(&tuning) {
                distinct.push(tuning);
            }
        }
        let tables =
            (distinct.len() as u64).saturating_mul(crate::tuning::PreparedTuning::prepared_bytes());
        tables
            .saturating_add(references.saturating_mul(size_of::<crate::plan::TuningSlot>() as u64))
    }

    /// The most control writes one of this plan's note-ons can expand to, gate included.
    ///
    /// `SOUND-INV-021`'s cardinality change, as the resource report needs it: the control
    /// scratch is sized on it, so a budget of one write per event is overrun by the first
    /// note whose scope declares a pitch and a velocity destination.
    ///
    /// An **upper bound**, and one is enough here: the only row it reaches is the scratch
    /// row, which `RefuseUpTo::Arena` already exempts from preflight refusal for the same
    /// reason the arena's own estimate is. `CompiledPlan::max_writes_per_note` is the exact
    /// figure preparation allocates from, and
    /// `the_control_scratch_budget_covers_what_preparation_actually_allocates` holds the two
    /// together.
    ///
    /// Never below one: a plan with no playable node still writes a gate per note event it
    /// could never receive, and a scratch of zero would be a buffer nothing can be put in.
    pub fn max_writes_per_note(&self) -> WritesPerNote {
        self.scope_summaries()
            .into_iter()
            .filter(|summary| summary.playable)
            .map(|summary| WritesPerNote::with_magnitudes(summary.magnitudes))
            .max()
            .unwrap_or(WritesPerNote::GATE_ONLY)
    }

    /// What each execution scope declares, in **one pass** over the plan's nodes.
    ///
    /// Both figures above are per scope, and the obvious form asks each node whether its scope
    /// holds a playable node — a scan inside a scan, and then a third for the magnitudes. A
    /// plan near `max_nodes` pays roughly `N²` node visits for that, twice per compile, and an
    /// *oversized* plan pays it **before** the refusal that would have rejected it. A review of
    /// the finished branch found it, and `compile.rs` already carries the same lesson about
    /// edges.
    ///
    /// Linear instead: one descriptor per node, accumulated into an entry per scope. The
    /// lookup is a scan of that list and stays constant because [`ExecutionScope`] is a closed
    /// enum of five.
    fn scope_summaries(&self) -> Vec<ScopeSummary> {
        let mut summaries: Vec<ScopeSummary> = Vec::new();
        for node in &self.nodes {
            let scope = node.scope();
            let index = match summaries.iter().position(|held| held.scope == scope) {
                Some(index) => index,
                None => {
                    summaries.push(ScopeSummary {
                        scope,
                        playable: false,
                        magnitudes: 0,
                        pitch_destinations: 0,
                    });
                    summaries.len() - 1
                }
            };
            let Some(descriptor) = crate::node::descriptor(node.kind()) else {
                continue;
            };
            let Some(summary) = summaries.get_mut(index) else {
                continue;
            };
            if descriptor.note_control.is_some() {
                summary.playable = true;
            }
            for spec in &descriptor.controls {
                match spec.magnitude {
                    Some(crate::node::NoteMagnitude::Pitch) => {
                        summary.magnitudes = summary.magnitudes.saturating_add(1);
                        summary.pitch_destinations = summary.pitch_destinations.saturating_add(1);
                    }
                    Some(crate::node::NoteMagnitude::Velocity) => {
                        summary.magnitudes = summary.magnitudes.saturating_add(1);
                    }
                    None => {}
                }
            }
        }
        summaries
    }

    /// The mutable state bytes this plan's nodes occupy, with the node responsible.
    ///
    /// One state record per scheduled record, plus the parameter slots (`SOUND-INV-023`)
    /// each node's writable controls need — slot, buffer offset and per-frame buffer — and
    /// the per-node run table of those buffers (`SOUND-INV-024`): `HOST` admits slots
    /// through this row rather than as a count of their own, and the renderer allocates
    /// exactly this much. The attribution stays the node whose state payload is widest; a
    /// slot belongs to its node and is charged with it.
    pub fn mutable_bytes(&self, inserted: u64) -> (PreparedBytes, IrObject) {
        // The attribution walks the payloads; the total is over **state** records, which is
        // what the renderer allocates one of per voice instance (`P06-S001`).
        let (_, dominant) = self.aggregate_bytes(
            crate::node::state_bytes_per_node(),
            |kind| {
                crate::node::state_payload_bytes(kind)
                    .saturating_add(crate::node::slot_payload_bytes(kind))
            },
            inserted,
        );
        let records = u64::from(self.state_records(inserted).get());
        let voices = u64::from(self.voice_instances().get());
        // The slots and buffers of a voice-scope node exist once per instance: voice-local
        // parameter state, one row per instance of each control.
        let slots = self.nodes.iter().fold(0_u64, |total, node| {
            let per_instance = crate::node::slot_payload_bytes(node.kind());
            let instances = if node.scope() == ExecutionScope::Voice {
                voices
            } else {
                1
            };
            total.saturating_add(per_instance.saturating_mul(instances))
        });
        // The per-node run table of `SOUND-INV-024`'s buffers: one entry per state record
        // and a terminator, sized exactly as preparation sizes it.
        let table = records
            .saturating_add(1)
            .saturating_mul(crate::node::ramp_table_bytes_per_record());
        (
            PreparedBytes::measured(
                records
                    .saturating_mul(crate::node::state_bytes_per_node())
                    .saturating_add(slots)
                    .saturating_add(table),
            ),
            dominant,
        )
    }

    /// How many voice instances the plan renders: one per identity index of its note
    /// producers — the sum of their `simultaneous_notes` — and at least one, so a voice scope
    /// nothing plays still renders once (`P06-S001`).
    ///
    /// Derived rather than declared: an `active_voices` declaration used to sit beside the
    /// producers and could disagree with them; the renderer instantiates exactly this many,
    /// and the report admits exactly this many against `max_active_voices`.
    /// How many rows one sample-positioned write can fan out over: the instance count where
    /// a voice-scope node declares a writable `ControlRate::Sample` control, and one where
    /// none does.
    ///
    /// What the timed-control scratch is sized on beside a note-on's expansion
    /// (`P06-S001`). A quantum-rate write reaches its rows through the slots and takes no
    /// scratch, and a note's magnitudes land on one row — so a plan whose voice scope has
    /// no sample-positioned control fans nothing out, whatever its polyphony, and charging
    /// it `N` writes per event would refuse it for scratch no event can fill. An independent
    /// review found the charge stated over every plan. [`crate::plan::CompiledPlan`] derives
    /// the same figure from its target table, and a test holds the two equal.
    pub fn sample_positioned_fan_out(&self) -> VoiceCount {
        let fans_out = self.nodes.iter().any(|node| {
            node.scope() == ExecutionScope::Voice
                && crate::node::descriptor(node.kind()).is_some_and(|descriptor| {
                    descriptor.controls.iter().any(|spec| {
                        spec.rate == crate::plan::ControlRate::Sample && spec.law.admits_writes()
                    })
                })
        });
        if fans_out {
            self.voice_instances()
        } else {
            VoiceCount::measured(1)
        }
    }

    pub fn voice_instances(&self) -> VoiceCount {
        VoiceCount::measured(self.identity_indices().max(1))
    }

    /// The identity partition's size: the sum of every producer's `simultaneous_notes`, and
    /// zero for a plan nothing plays. What the cost model prices — a plan with no producer
    /// prices no voice, though it still renders its voice scope once.
    #[must_use]
    pub fn identity_indices(&self) -> u32 {
        self.declarations
            .note_producers
            .iter()
            .fold(0_u32, |total, producer| {
                total.saturating_add(producer.simultaneous_notes.get())
            })
    }

    /// Whether any node renders in the voice scope, and so is instantiated per voice.
    #[must_use]
    pub fn has_voice_scope(&self) -> bool {
        self.nodes
            .iter()
            .any(|node| node.scope() == ExecutionScope::Voice && node.kind().is_source())
    }

    /// How many **state** records the plan schedules: a node with a kernel once per instance
    /// of its scope — `voice_instances` for the voice scope, one otherwise — plus what the
    /// compiler inserted. This is what the renderer keeps one state, one control run and one
    /// buffer run per.
    pub fn state_records(&self, inserted: u64) -> RecordCount {
        let voices = u64::from(self.voice_instances().get());
        let scheduled = self
            .nodes
            .iter()
            .filter(|node| node.kind().is_source())
            .fold(0_u64, |total, node| {
                total.saturating_add(if node.scope() == ExecutionScope::Voice {
                    voices
                } else {
                    1
                })
            });
        RecordCount::measured(u32::try_from(scheduled.saturating_add(inserted)).unwrap_or(u32::MAX))
    }

    /// How many **prepared** records the plan holds: a node with a kernel once, whatever
    /// its scope — prepared data is shared by every instance and cloned by none — plus what
    /// the compiler inserted.
    ///
    /// Every row and allocation that is *per scheduled record* is over this one figure,
    /// so a caller that counted nodes instead would over-report by exactly the outputs.
    pub fn scheduled_records(&self, inserted: u64) -> RecordCount {
        let scheduled = self
            .nodes
            .iter()
            .filter(|node| node.kind().is_source())
            .count() as u64;
        // Saturating rather than checked: a plan with more than four billion records is
        // refused by `max_nodes` long before this, and the report's job at that point is
        // to state an amount that is certainly over the limit rather than to fail.
        RecordCount::measured(u32::try_from(scheduled.saturating_add(inserted)).unwrap_or(u32::MAX))
    }

    fn aggregate_bytes(
        &self,
        per_node: u64,
        payload: fn(IrNodeKind) -> u64,
        inserted: u64,
    ) -> (PreparedBytes, IrObject) {
        let records = u64::from(self.scheduled_records(inserted).get());
        let total = records.saturating_mul(per_node);
        let mut dominant = (0_u64, IrObject::Plan);
        for node in &self.nodes {
            let bytes = payload(node.kind());
            if bytes > dominant.0 {
                dominant = (bytes, IrObject::Node(node.id()));
            }
        }
        (PreparedBytes::measured(total), dominant.1)
    }

    /// The largest number of edges leaving one output port, and that port.
    ///
    /// Sorted and counted in runs rather than compared pairwise. The naive form is
    /// quadratic in the edge count, and `max_edges` defaults to 65 536 — a compiler
    /// that takes four billion comparisons to reject a plan is not a compiler that
    /// gets used.
    pub fn peak_fan_out(&self) -> (crate::quantities::FanOut, IrObject) {
        let mut sources: Vec<(NodeId, PortId)> =
            self.edges.iter().map(super::ir::IrEdge::from).collect();
        sources.sort_unstable();

        let mut peak = (0_u32, IrObject::Plan);
        let mut run = 0_u32;
        let mut index = 0;
        while index < sources.len() {
            run = run.saturating_add(1);
            let is_last_of_run = sources
                .get(index + 1)
                .is_none_or(|next| *next != sources[index]);
            if is_last_of_run {
                if run > peak.0 {
                    let (node, port) = sources[index];
                    peak = (run, IrObject::Port(node, port));
                }
                run = 0;
            }
            index += 1;
        }
        (crate::quantities::FanOut::measured(peak.0), peak.1)
    }

    /// The script work this plan performs per quantum, and the program that
    /// contributes most.
    ///
    /// Reported, never budgeted: no threshold is attached until Phase 7 measures a
    /// per-instruction cost.
    pub fn script_instructions_per_quantum(&self) -> (ScriptWorkPerQuantum, IrObject) {
        let mut total = 0_u64;
        let mut dominant = (0_u64, IrObject::Plan);
        for program in &self.declarations.programs {
            // In `u64`, because this is a product of two separately admissible `u32`s: a
            // 256-instruction program evaluated `u32::MAX` times is over a trillion
            // instructions, and saturating at `u32::MAX` would understate it by three
            // orders of magnitude while looking exact.
            let work = u64::from(program.instructions().get())
                .saturating_mul(u64::from(program.evaluations_per_quantum()));
            total = total.saturating_add(work);
            if work > dominant.0 {
                dominant = (work, IrObject::Program(program.id()));
            }
        }
        (ScriptWorkPerQuantum::measured(total), dominant.1)
    }

    /// The predicted median cost of one quantum, as a fraction of its real-time
    /// budget.
    ///
    /// The model is EVD-0003's measured per-voice slope and intercept — 1.173 ms of
    /// CPU per second of audio per voice, plus 0.428 ms — scaled by the prepared
    /// rate against the rate it was measured at. The scaling is what `HOST-INV-011`
    /// requires: a block carries the same work at every sample rate while its
    /// real-time budget shrinks with the rate, so a policy stated in frames reads a
    /// 192 kHz plan as no more expensive than a 44.1 kHz one when it is more than
    /// four times as expensive per second.
    ///
    /// Every figure behind it was measured on V1, offline, with no host and no
    /// callback deadline, on one machine. It may warn; it may not decide.
    pub fn predicted_quantum_cost_ratio(
        &self,
        sample_rate: crate::quantities::SampleRate,
    ) -> Option<CostRatio> {
        /// Milliseconds of CPU per second of rendered audio, per voice.
        const MS_PER_SECOND_PER_VOICE: f32 = 1.173;
        /// Fixed milliseconds of CPU per second of rendered audio.
        const MS_PER_SECOND_FIXED: f32 = 0.428;
        /// The rate EVD-0003's figures were measured at.
        const MEASURED_AT_HZ: f32 = 44_100.0;

        let voices = self.identity_indices();
        if voices == 0 && self.nodes.is_empty() {
            return CostRatio::measured(0.0).ok();
        }
        let cost_ms = MS_PER_SECOND_PER_VOICE.mul_add(voices as f32, MS_PER_SECOND_FIXED);
        let ratio = cost_ms / 1_000.0 * (sample_rate.as_f32() / MEASURED_AT_HZ);
        CostRatio::measured(ratio).ok()
    }
}

/// Assembles a [`GraphIr`], refusing a plan that cannot be read.
#[must_use]
pub struct GraphIrBuilder {
    nodes: Vec<IrNode>,
    edges: Vec<IrEdge>,
    declarations: PlanDeclarations,
    tunings: Vec<ScopeTuning>,
    next_edge: u32,
}

impl GraphIrBuilder {
    /// Add a node.
    pub fn node(mut self, id: NodeId, kind: IrNodeKind, scope: ExecutionScope) -> Self {
        self.nodes.push(IrNode::new(id, kind, scope));
        self
    }

    /// State the tuning one execution scope resolves its keys through.
    ///
    /// Declaring a second one for the same scope refuses at [`Self::build`] rather than
    /// overwriting: `SOUND-INV-021` requires one prepared value per scope, and silently
    /// keeping the last of two would decide by declaration order which scale a plan sounds
    /// in.
    pub fn tuning(mut self, scope: ExecutionScope, tuning: PreparedTuning) -> Self {
        self.tunings.push(ScopeTuning::new(scope, tuning));
        self
    }

    /// Connect one node's output port to another's input port.
    pub fn connect(
        mut self,
        from: (NodeId, PortId),
        to: (NodeId, PortId),
        domain: SignalDomain,
    ) -> Self {
        self.edges
            .push(IrEdge::new(EdgeId::new(self.next_edge), from, to, domain));
        self.next_edge = self.next_edge.saturating_add(1);
        self
    }

    /// Replace what the plan declares it needs.
    pub fn declaring(mut self, declarations: PlanDeclarations) -> Self {
        self.declarations = declarations;
        self
    }

    /// Finish, or say why the plan cannot be read.
    ///
    /// Indexed rather than scanned: the obvious form compares every node against every
    /// earlier one and every edge against every node, which is quadratic in a plan the
    /// profile admits. Hashing here is free of consequence — nothing in this function
    /// runs on the audio thread — and the map is dropped before the IR is returned.
    pub fn build(self) -> Result<GraphIr, IrError> {
        let mut kinds: std::collections::HashMap<NodeId, IrNodeKind> =
            std::collections::HashMap::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if kinds.insert(node.id(), node.kind()).is_some() {
                return Err(IrError::DuplicateNode { id: node.id() });
            }
        }
        for edge in &self.edges {
            let (source_id, _) = edge.from();
            let (target_id, _) = edge.to();
            let source = kinds.get(&source_id).ok_or(IrError::UnknownNode {
                edge: edge.id(),
                node: source_id,
            })?;
            if !kinds.contains_key(&target_id) {
                return Err(IrError::UnknownNode {
                    edge: edge.id(),
                    node: target_id,
                });
            }
            if !source.is_source() {
                return Err(IrError::NotASource {
                    edge: edge.id(),
                    node: source_id,
                });
            }
        }
        for (index, entry) in self.tunings.iter().enumerate() {
            if self.tunings[..index]
                .iter()
                .any(|earlier| earlier.scope() == entry.scope())
            {
                return Err(IrError::DuplicateScopeTuning {
                    scope: entry.scope(),
                });
            }
        }
        Ok(GraphIr {
            nodes: self.nodes,
            edges: self.edges,
            declarations: self.declarations,
            tunings: self.tunings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quantities::SampleRate;

    const SOURCE: NodeId = NodeId::new(1);
    const OUTPUT: NodeId = NodeId::new(2);

    fn sine_plan() -> GraphIr {
        GraphIr::builder()
            .node(
                SOURCE,
                IrNodeKind::Sine {
                    frequency: Frequency::new(440.0).expect("a finite test frequency"),
                    amplitude: Amplitude::new(0.5).expect("a finite test amplitude"),
                },
                ExecutionScope::Voice,
            )
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()
            .expect("a source into an output is a readable plan")
    }

    #[test]
    fn an_empty_plan_is_legal() {
        let ir = GraphIr::empty();
        assert!(ir.nodes().is_empty());
        assert!(ir.edges().is_empty());
    }

    #[test]
    fn a_duplicate_node_identity_is_refused() {
        let built = GraphIr::builder()
            .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Global)
            .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Global)
            .build();
        assert_eq!(built, Err(IrError::DuplicateNode { id: SOURCE }));
    }

    #[test]
    fn a_dangling_edge_is_refused_rather_than_ignored() {
        let built = GraphIr::builder()
            .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .build();
        assert_eq!(
            built,
            Err(IrError::UnknownNode {
                edge: EdgeId::FIRST,
                node: OUTPUT
            })
        );
    }

    #[test]
    fn an_edge_out_of_the_output_node_is_refused() {
        let built = GraphIr::builder()
            .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Global)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (OUTPUT, PortId::FIRST),
                (SOURCE, PortId::FIRST),
                SignalDomain::Audio,
            )
            .build();
        assert_eq!(
            built,
            Err(IrError::NotASource {
                edge: EdgeId::FIRST,
                node: OUTPUT
            })
        );
    }

    #[test]
    fn memory_aggregates_name_the_node_that_dominates_them() {
        let ir = sine_plan();
        let (mutable, dominant) = ir.mutable_bytes(0);
        assert!(mutable.get() > 0, "the sine carries phase between quanta");
        assert_eq!(
            dominant,
            IrObject::Node(SOURCE),
            "the only stateful node must be named as the dominant contributor"
        );
    }

    #[test]
    fn fan_out_names_the_port_it_measured() {
        let ir = GraphIr::builder()
            .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Global)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (SOURCE, PortId::FIRST),
                (OUTPUT, PortId::new(1)),
                SignalDomain::Audio,
            )
            .build()
            .expect("readable plan");
        let (fan_out, object) = ir.peak_fan_out();
        assert_eq!(fan_out.get(), 2);
        assert_eq!(object, IrObject::Port(SOURCE, PortId::FIRST));
    }

    #[test]
    fn script_work_is_instructions_times_evaluations() {
        let program = IrProgram::new(
            ProgramId::FIRST,
            InstructionCount::measured(100),
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            8,
        );
        let ir = GraphIr::builder()
            .declaring(PlanDeclarations {
                programs: vec![program],
                ..PlanDeclarations::default()
            })
            .build()
            .expect("declarations alone are a readable plan");
        let (work, dominant) = ir.script_instructions_per_quantum();
        assert_eq!(work.get(), 800);
        assert_eq!(dominant, IrObject::Program(ProgramId::FIRST));
    }

    #[test]
    fn script_work_survives_a_product_that_would_overflow_a_u32() {
        // Both factors are separately admissible, and their product is not a `u32`.
        // Saturating here would report a trillion instructions as four billion.
        let program = IrProgram::new(
            ProgramId::FIRST,
            InstructionCount::measured(256),
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            u32::MAX,
        );
        let ir = GraphIr::builder()
            .declaring(PlanDeclarations {
                programs: vec![program],
                ..PlanDeclarations::default()
            })
            .build()
            .expect("declarations alone are a readable plan");
        let (work, _) = ir.script_instructions_per_quantum();
        assert_eq!(work.get(), 256 * u64::from(u32::MAX));
    }

    #[test]
    fn the_cost_model_reproduces_the_two_figures_the_evidence_states() {
        let ir = GraphIr::builder()
            .declaring(PlanDeclarations {
                // 512 voices, derived from a producer of 512 simultaneous notes.
                note_producers: vec![NoteProducerDeclaration {
                    compiled: true,
                    simultaneous_notes: HeldNoteCount::measured(512),
                    simultaneous_holds: EventCount::NONE,
                }],
                ..PlanDeclarations::default()
            })
            .build()
            .expect("declarations alone are a readable plan");

        // EVD-0003: 512 voices is "about 600 ms of CPU per second of rendered
        // audio" at 44.1 kHz — roughly 60% of one core.
        let at_cd = ir
            .predicted_quantum_cost_ratio(SampleRate::new(44_100.0).expect("valid rate"))
            .expect("the model produces a finite ratio");
        assert!(
            (at_cd.as_f32() - 0.601).abs() < 0.005,
            "expected ~0.601 of budget at 44.1 kHz, got {}",
            at_cd.as_f32()
        );

        // And "about 2.6 s per second of audio" at 192 kHz, which is not real time
        // at all. The same plan, the same counts, four times the cost: this is the
        // asymmetry a frame-based policy would invert.
        let at_high = ir
            .predicted_quantum_cost_ratio(SampleRate::new(192_000.0).expect("valid rate"))
            .expect("the model produces a finite ratio");
        assert!(
            (at_high.as_f32() - 2.617).abs() < 0.01,
            "expected ~2.617 of budget at 192 kHz, got {}",
            at_high.as_f32()
        );
    }

    #[test]
    fn an_empty_plan_predicts_no_cost() {
        let ratio = GraphIr::empty()
            .predicted_quantum_cost_ratio(SampleRate::new(48_000.0).expect("valid rate"))
            .expect("the model produces a finite ratio");
        assert_eq!(ratio.as_f32(), 0.0);
    }
}
