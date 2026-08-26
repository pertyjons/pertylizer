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
};
use crate::time::PlanPosition;
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
typed_id!(TapId, u32, "tap", "An observation tap's stable identity.");
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
    /// One observation tap.
    Tap(TapId),
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
            Self::Tap(id) => write!(f, "{id}"),
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

    /// A sine's frequency in hertz. Control-rate.
    pub const SINE_FREQUENCY: ParameterId = ParameterId::new(0);
    /// A sine's peak amplitude. Control-rate.
    pub const SINE_AMPLITUDE: ParameterId = ParameterId::new(1);
    /// An envelope's gate: above zero is held, zero or below is released.
    ///
    /// **Sample-positioned.** It is the same control a note edge moves, so
    /// automating a gate and playing a note reach it under one timing law.
    pub const ENVELOPE_GATE: ParameterId = ParameterId::new(2);
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
/// The `Default` is written out rather than derived, because deriving it would
/// require a `Default` on every quantity type — and a capacity type whose default
/// is zero is the shape the two-constructor rule exists to prevent. Here the zeros
/// are *measurements*, which is a different claim.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct PlanDeclarations {
    /// Voices sounding at once across the plan.
    pub active_voices: VoiceCount,
    /// Notes held at once across the plan.
    pub held_notes: HeldNoteCount,
    /// Mix channels.
    pub mix_channels: MixChannelCount,
    /// Buses.
    pub buses: BusCount,
    /// The most sends any one channel has.
    pub max_sends_on_any_channel: SendCount,
    /// Observation taps this plan registers.
    pub taps: Vec<TapId>,
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
}

impl Default for PlanDeclarations {
    fn default() -> Self {
        Self {
            active_voices: VoiceCount::NONE,
            held_notes: HeldNoteCount::NONE,
            mix_channels: MixChannelCount::NONE,
            buses: BusCount::NONE,
            max_sends_on_any_channel: SendCount::NONE,
            taps: Vec::new(),
            events_per_quantum: EventCount::NONE,
            note_expansion_per_tick: EventCount::NONE,
            scheduled_events_in_flight: EventCount::NONE,
            mod_graph_nodes: NodeCount::NONE,
            note_graph_nodes: NodeCount::NONE,
            held_notes_per_take: HeldNoteCount::NONE,
            recorded_events_per_take: EventCount::NONE,
            programs: Vec::new(),
            voices_per_instrument: VoiceCount::NONE,
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
}

/// A plan, as the compiler receives it.
#[derive(Debug, Clone, PartialEq, Default)]
#[must_use]
pub struct GraphIr {
    nodes: Vec<IrNode>,
    edges: Vec<IrEdge>,
    declarations: PlanDeclarations,
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
            next_edge: 0,
        }
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

    /// The mutable state bytes this plan's nodes occupy, with the node responsible.
    pub fn mutable_bytes(&self, inserted: u64) -> (PreparedBytes, IrObject) {
        self.aggregate_bytes(
            crate::node::state_bytes_per_node(),
            crate::node::state_payload_bytes,
            inserted,
        )
    }

    /// How many records the plan schedules: a node with a kernel, plus what the compiler
    /// inserted.
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

        let voices = self.declarations.active_voices.get();
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
    next_edge: u32,
}

impl GraphIrBuilder {
    /// Add a node.
    pub fn node(mut self, id: NodeId, kind: IrNodeKind, scope: ExecutionScope) -> Self {
        self.nodes.push(IrNode::new(id, kind, scope));
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
        Ok(GraphIr {
            nodes: self.nodes,
            edges: self.edges,
            declarations: self.declarations,
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
                active_voices: VoiceCount::measured(512),
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
