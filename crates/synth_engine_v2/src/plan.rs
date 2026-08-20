//! The prepared render plan: what the renderer executes.
//!
//! The master plan's layer boundaries fix what belongs here — ordered operations,
//! numeric slots, immutable prepared node data, fixed-size mutable state layout,
//! event-routing tables, latency metadata — and what must not: **no validation
//! branches, strings, hash maps, filesystem paths, or construction logic in the
//! render loop**.
//!
//! It also carries every capacity the renderer needs, copied at admission. That is
//! `HOST-INV-002`: the renderer reads the prepared plan and never the profile, so a
//! capacity reaching the audio thread without having passed admission is a defect
//! rather than a fallback.

use crate::ir::{NodeId, ParameterId};
use crate::node::kernels::{ControlIndex, Kernel, MAX_INPUTS, PreparedNode};
use crate::quantities::{ChannelLayout, EventCount, SampleRate};
use crate::time::FrameCount;

/// One buffer in the plan's arena, by index.
///
/// An **identity**, not a position: since
/// [ADR-0041](../../plans/v2/decisions/ADR-0041-interleaved-internal-channel-layout.md)
/// clause 2 a signal occupies one region of `c * Q` samples, so slots are no longer
/// uniform and a slot's place in the arena is the offset and length the plan records
/// for it — [`CompiledPlan::region`]. Multiplying this index by the quantum was the
/// planar arithmetic and is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct BufferSlot(usize);

impl BufferSlot {
    /// A slot.
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// The index.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Where one slot's samples live: an offset and a length within the one allocation.
///
/// ADR-0041 clause 13. Both are in **samples**, not frames: a region holds `c * Q`
/// samples of one signal, and the kernel that reads it is told the channel count
/// separately, so a length in frames would have to be multiplied back out at every
/// binding.
///
/// The plan records these; nothing derives them. That is the whole difference from the
/// planar arena, where a slot index times the quantum was the position and every slot
/// was the same width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct BufferRegion {
    offset: usize,
    length: usize,
}

impl BufferRegion {
    /// A region, or `None` if it is not one.
    ///
    /// A zero length is not a narrow region, it is the absence of storage, and a region
    /// whose end overflows is not describable at all. Both are compiler defects rather
    /// than render-time conditions, and the type refuses them here so that [`Self::end`]
    /// can be exact arithmetic rather than a saturating one that turns a malformed
    /// region into a plausible-looking alias of an unrelated range.
    #[must_use]
    pub fn new(offset: usize, length: usize) -> Option<Self> {
        match length > 0 && offset.checked_add(length).is_some() {
            true => Some(Self { offset, length }),
            false => None,
        }
    }

    /// A region the arena has already established, without re-checking it.
    ///
    /// Crate-private: the assignment builds these from a width it took from a port's
    /// layout and an offset it computed itself, so the invariant holds by construction,
    /// and a `Result` at every allocation would be the compiler checking itself.
    /// Everything outside the crate goes through [`Self::new`].
    pub(crate) const fn raw(offset: usize, length: usize) -> Self {
        Self { offset, length }
    }

    /// The first sample.
    pub const fn offset(self) -> usize {
        self.offset
    }

    /// How many samples it holds.
    pub const fn length(self) -> usize {
        self.length
    }

    /// One past the last sample. The arena's extent is the greatest of these.
    ///
    /// Exact rather than saturating: [`Self::new`] refuses a region whose end overflows,
    /// so there is nothing here to saturate away.
    pub const fn end(self) -> usize {
        self.offset + self.length
    }

    /// Whether two regions share any sample.
    ///
    /// ADR-0041 clause 14 strengthens ADR-0005 clause 8's structural check to this:
    /// with mixed widths, two *distinct* slots can still intersect, so identity is no
    /// longer the question and partial overlap is a defect the equal-slot arena could
    /// not represent.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.offset < other.end() && other.offset < self.end()
    }
}

/// A compiled plan's identity.
///
/// Issued once per compilation, and carried by every [`ParameterSlot`] the plan hands
/// out. Its job is the same as [`crate::time::StreamEpoch`]'s: make a *stale* value
/// detectable rather than merely unlikely. A slot is an index into one plan's target
/// table, so a slot resolved against another plan does not do nothing — it writes
/// whatever occupies that index here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct PlanId(u64);

impl PlanId {
    /// The identity the never-read event scratch fill carries.
    ///
    /// Distinct from every issued identity because issuing starts at 1.
    pub(crate) const FILL: Self = Self(0);

    /// The raw identity, for a report or a log.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for PlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "plan {}", self.0)
    }
}

/// Issue the next plan identity.
///
/// Saturates rather than wrapping at `u64::MAX`, which is unreachable: a compilation
/// every nanosecond would take five centuries to get there, and a wrapped identity
/// would make two plans indistinguishable — the one thing this type exists to prevent.
pub(crate) fn issue_plan_id() -> PlanId {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    PlanId(
        NEXT.fetch_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |current| Some(current.saturating_add(1)),
        )
        .map_or(u64::MAX, |previous| previous.saturating_add(1)),
    )
}

/// One addressable parameter in the plan, by index.
///
/// This is what "compile stable names and IDs to compact numeric slots" means for a
/// parameter: an event carries the **slot**, resolved once off the audio thread by
/// [`CompiledPlan::resolve_parameter`], and the renderer indexes straight into its
/// target table. Phase 1 carried the `(NodeId, ParameterId)` pair into the render loop
/// and scanned a routing table for it on every event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct ParameterSlot {
    plan: PlanId,
    index: usize,
}

impl ParameterSlot {
    /// A slot. Crate-private: the only way to obtain one is
    /// [`CompiledPlan::resolve_parameter`], which is what keeps an index and the table
    /// it indexes from drifting apart.
    pub(crate) const fn new(plan: PlanId, index: usize) -> Self {
        Self { plan, index }
    }

    /// Which plan this slot indexes.
    pub const fn plan(self) -> PlanId {
        self.plan
    }

    /// The index into that plan's target table.
    pub const fn index(self) -> usize {
        self.index
    }
}

/// One node that accepts note edges, by index.
///
/// The note-side twin of [`ParameterSlot`], and it exists for the same reason: an event
/// carries the slot, resolved once off the audio thread by
/// [`CompiledPlan::resolve_note`], and the renderer indexes rather than searching. It is
/// a separate address space because a note is not a parameter write — it names a node
/// that can be played, and the control it moves is the node kind's business rather than
/// the caller's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct NoteSlot {
    plan: PlanId,
    index: usize,
}

impl NoteSlot {
    /// A slot. Crate-private for the same reason [`ParameterSlot::new`] is.
    pub(crate) const fn new(plan: PlanId, index: usize) -> Self {
        Self { plan, index }
    }

    /// Which plan this slot indexes.
    pub const fn plan(self) -> PlanId {
        self.plan
    }

    /// The index into that plan's note-target table.
    pub const fn index(self) -> usize {
        self.index
    }
}

/// When a control a caller moves takes effect.
///
/// ADR-0001 splits this deliberately, in clause 14: *sample-positioned* effects — note-on,
/// note-off, gate, retrigger — occur at their declared sample within the quantum, while
/// the *control-rate* response to a mid-quantum event begins at the next quantum
/// boundary. The split is a property of the **effect**, not of the message that carried
/// it, so it is declared by the node kind and compiled into the target rather than being
/// chosen by whichever payload a caller happened to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ControlRate {
    /// Evaluated once per quantum, at the boundary at or after the event (clause 13).
    Quantum,
    /// Applied at the event's declared sample inside the quantum (clause 14).
    Sample,
}

/// One node instance, by index.
///
/// It indexes **both** tables: the plan's prepared data and the renderer's mutable
/// state. They are parallel by construction — admission builds one record in each per
/// node — and one index for both is what keeps a node's configuration and its state from
/// being paired by two independent counters that can drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct NodeSlot(usize);

impl NodeSlot {
    /// A slot.
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// The index.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// What one of a step's inputs resolves to, decided at admission.
///
/// The classification a renderer would otherwise redo per node per quantum: whether an
/// input is patched at all, whether the arena gave it the output's own slot, and whether
/// it reads a buffer an earlier input already borrowed. None of it can change between
/// quanta — the slots are fixed once the arena has assigned them — so deciding it here
/// is the same rule the phase's first gate bullet states: the hot path makes no topology
/// decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputBinding {
    /// Nothing is patched here.
    Unpatched,
    /// The arena gave this input the output's own slot.
    InPlace,
    /// A distinct region, to be borrowed.
    Distinct,
    /// The same region as an earlier input, whose borrow it shares.
    Mirrors(u8),
}

/// One step of the schedule: which kernel runs, over which slots.
///
/// This is what [ADR-0004](../../plans/v2/decisions/ADR-0004-native-node-representation.md)
/// means by a prepared function table. The kernel is resolved from the node's kind once,
/// at admission; the render loop calls through the pointer and never learns what kind of
/// node it just ran. Adding a node kind adds a kernel and a registry entry, and adds
/// nothing here and nothing to the loop.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct NodeStep {
    kernel: Kernel,
    node: NodeSlot,
    out: BufferSlot,
    /// The layout of the signal it writes.
    ///
    /// ADR-0041 clause 4: a kernel is **told** how many channels it has, and this is
    /// where the count comes from — the node's own output port, resolved at admission,
    /// rather than the stream's layout or the width of the region divided by the quantum.
    out_layout: ChannelLayout,
    inputs: [Option<BufferSlot>; MAX_INPUTS],
    /// What each input resolves to, decided here rather than per quantum.
    bindings: [InputBinding; MAX_INPUTS],
    /// The regions to borrow, in ascending slot order: `0` is the output and `n` is
    /// input `n - 1`. `u8::MAX` ends the list.
    ///
    /// Ascending because the borrows are handed out by walking the arena forwards and
    /// splitting each region off in turn, which is what makes one mutable and two shared
    /// borrows of one allocation safe without `unsafe`. Sorting three entries per node
    /// per quantum is not expensive; doing it at all is a decision the compiler already
    /// had the answer to.
    order: [u8; MAX_INPUTS + 1],
    in_place_safe: bool,
}

impl NodeStep {
    /// A step.
    ///
    /// Admission builds these. It is public so that a harness can build one too — a step
    /// on its own is inert, because the only way to get a [`CompiledPlan`] is to compile
    /// a graph — and the ADR-0004 evidence harness needs one to bind an arena the way the
    /// renderer does.
    pub fn new(
        kernel: Kernel,
        node: NodeSlot,
        out: BufferSlot,
        out_layout: ChannelLayout,
        inputs: [Option<BufferSlot>; MAX_INPUTS],
        in_place_safe: bool,
    ) -> Self {
        let mut step = Self {
            kernel,
            node,
            out,
            out_layout,
            inputs,
            bindings: [InputBinding::Unpatched; MAX_INPUTS],
            order: [u8::MAX; MAX_INPUTS + 1],
            in_place_safe,
        };
        // Ordered by slot index until the arena has run. Lowering's slots are virtual and
        // have no offset yet; [`Self::remap`] resolves the order again over the regions
        // they were assigned, which is the order the binding actually walks.
        step.resolve(&[]);
        step
    }

    /// Work out what each input is, and the order the regions are borrowed in.
    ///
    /// `regions` is the assignment's table, indexed by [`BufferSlot`]; it is empty while
    /// the slots are still virtual, and then the slot index stands in for the offset.
    /// After the arena has run, the order is by **offset** — with variable-width regions
    /// a higher slot index can sit lower in the arena, and the binding walks the
    /// allocation forwards.
    fn resolve(&mut self, regions: &[BufferRegion]) {
        self.bindings = [InputBinding::Unpatched; MAX_INPUTS];
        for index in 0..MAX_INPUTS {
            let Some(Some(slot)) = self.inputs.get(index).copied() else {
                continue;
            };
            let binding = if slot == self.out {
                InputBinding::InPlace
            } else {
                let mirrored = (0..index).find(|earlier| {
                    self.inputs.get(*earlier).copied().flatten() == Some(slot)
                        && matches!(self.bindings.get(*earlier), Some(InputBinding::Distinct))
                });
                match mirrored {
                    Some(earlier) => InputBinding::Mirrors(earlier as u8),
                    None => InputBinding::Distinct,
                }
            };
            if let Some(entry) = self.bindings.get_mut(index) {
                *entry = binding;
            }
        }

        // The regions to borrow, ascending by where they sit in the arena. Three entries
        // at most, so an insertion is cheaper and clearer than a sort — and this runs once
        // per compile either way.
        let mut order: [(usize, u8); MAX_INPUTS + 1] = [(usize::MAX, u8::MAX); MAX_INPUTS + 1];
        let mut count = 0;
        let push = |at: usize, role: u8, order: &mut [(usize, u8)], count: &mut usize| {
            let mut position = *count;
            while position > 0
                && order
                    .get(position - 1)
                    .is_some_and(|(other, _)| *other > at)
            {
                let previous = order
                    .get(position - 1)
                    .copied()
                    .unwrap_or((usize::MAX, u8::MAX));
                if let Some(entry) = order.get_mut(position) {
                    *entry = previous;
                }
                position -= 1;
            }
            if let Some(entry) = order.get_mut(position) {
                *entry = (at, role);
            }
            *count += 1;
        };
        let position_of = |slot: BufferSlot| -> usize {
            regions
                .get(slot.index())
                .map_or(slot.index(), |region| region.offset())
        };
        push(position_of(self.out), 0, &mut order, &mut count);
        for index in 0..MAX_INPUTS {
            if !matches!(self.bindings.get(index), Some(InputBinding::Distinct)) {
                continue;
            }
            let Some(Some(slot)) = self.inputs.get(index).copied() else {
                continue;
            };
            push(position_of(slot), index as u8 + 1, &mut order, &mut count);
        }

        self.order = [u8::MAX; MAX_INPUTS + 1];
        for (entry, (_, role)) in self.order.iter_mut().zip(order.iter()) {
            *entry = *role;
        }
    }

    /// What each input resolved to.
    pub const fn bindings(&self) -> &[InputBinding; MAX_INPUTS] {
        &self.bindings
    }

    /// The regions to borrow, in ascending slot order.
    pub const fn order(&self) -> &[u8; MAX_INPUTS + 1] {
        &self.order
    }

    /// The kernel this step calls.
    pub const fn kernel(&self) -> Kernel {
        self.kernel
    }

    /// Which prepared node and which state record.
    pub const fn node(&self) -> NodeSlot {
        self.node
    }

    /// The layout of the signal it writes.
    pub const fn out_layout(&self) -> ChannelLayout {
        self.out_layout
    }

    /// The buffer it writes.
    pub const fn out(&self) -> BufferSlot {
        self.out
    }

    /// The buffers it reads, in port order.
    pub const fn inputs(&self) -> &[Option<BufferSlot>; MAX_INPUTS] {
        &self.inputs
    }

    /// Whether the arena may give it its first input's slot.
    pub const fn in_place_safe(&self) -> bool {
        self.in_place_safe
    }

    /// Whether two steps call the same kernel.
    ///
    /// [`Kernel::is_same`] owns the comparison and records what function-pointer equality
    /// can and cannot promise.
    fn same_kernel(&self, other: &Self) -> bool {
        self.kernel.is_same(other.kernel)
    }

    /// Rewrite the slots this step names, once the arena has assigned them.
    pub(crate) fn remap(
        &mut self,
        out: BufferSlot,
        inputs: [Option<BufferSlot>; MAX_INPUTS],
        regions: &[BufferRegion],
    ) {
        self.out = out;
        self.inputs = inputs;
        // Resolved again rather than carried over: reuse is exactly what turns two
        // distinct slots into one, so a classification computed before the arena ran
        // would call an input distinct when it has just become the output's own. The
        // borrow order is resolved here too, because only now do the slots have offsets.
        self.resolve(regions);
    }
}

impl PartialEq for NodeStep {
    fn eq(&self, other: &Self) -> bool {
        self.same_kernel(other)
            && self.node == other.node
            && self.out == other.out
            // The layout is part of what a step *does*: it becomes `NodeIo::channels`,
            // and two steps that differ in it hand their kernel a different arrangement
            // of the same region.
            && self.out_layout == other.out_layout
            && self.inputs == other.inputs
            && self.bindings == other.bindings
            && self.order == other.order
            && self.in_place_safe == other.in_place_safe
    }
}

/// One operation, in execution order.
///
/// Two variants, and it stays two: a node kernel, and the renderer's own boundary. The
/// Phase 1 shape had one variant per node kind, which is what ADR-0004 clause 2 rejects —
/// a node addition was a new arm inside the quantum loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanOp {
    /// Run one prepared node kernel.
    Node(NodeStep),
    /// Write one region to the stream.
    ///
    /// **One** operation, not one per channel: since ADR-0041 clause 11 a signal whose
    /// layout is the stream's occupies one interleaved region, and matching the host's
    /// arrangement is a contiguous copy rather than the per-channel strided writes the
    /// planar renderer performed. What made a conversion visible before was that a mono
    /// signal reaching a stereo output compiled to two of these; it is now the widening
    /// operation upstream that carries it, which is a scheduled node with an identity
    /// under clause 9 rather than a shape the output happens to have.
    Output {
        /// The region to write out, `Q` frames of the stream's channels.
        source: BufferSlot,
    },
}

/// What a parameter event addresses, resolved to numeric slots at admission.
///
/// A node instance and one of its controls. Neither is an identity: the renderer indexes
/// its state table and hands the control index to the state, which is the last place the
/// meaning of "control 0" lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ParameterTarget {
    /// Which node instance.
    pub node: NodeSlot,
    /// Which of its controls.
    pub control: ControlIndex,
    /// When moving it takes effect.
    ///
    /// Compiled from what the node kind declares, so the renderer reads it rather than
    /// deciding it. A gate is [`ControlRate::Sample`] however it was addressed, which is
    /// what keeps ADR-0001 clause 14 from being violable by choosing another payload.
    pub rate: ControlRate,
}

/// What a note event addresses, resolved to numeric slots at admission.
///
/// The node, and the control on it that a note edge moves. Which control that is belongs
/// to the node kind: a caller plays a node, and the kind decides what being played means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct NoteTarget {
    /// Which node instance.
    pub node: NodeSlot,
    /// The control a note edge moves, always at [`ControlRate::Sample`].
    pub control: ControlIndex,
}

/// One row of the plan's note address table.
///
/// Read **off the audio thread only**, by [`CompiledPlan::resolve_note`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct NoteAddress {
    /// The node the caller plays.
    pub node: NodeId,
    /// The slot it compiles to.
    pub slot: NoteSlot,
}

/// One row of the plan's address table.
///
/// Read **off the audio thread only**, by [`CompiledPlan::resolve_parameter`]. The
/// renderer never sees an identity: it is handed a [`ParameterSlot`] and indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ParameterAddress {
    /// The node the caller names.
    pub node: NodeId,
    /// The parameter on that node.
    pub parameter: ParameterId,
    /// The slot it compiles to.
    pub slot: ParameterSlot,
}

/// An admitted plan, with every capacity it needs.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct CompiledPlan {
    id: PlanId,
    ops: Vec<PlanOp>,
    /// Where each slot's samples live, indexed by [`BufferSlot`].
    ///
    /// ADR-0041 clause 2: the plan records the position, because slot width is `c * Q`
    /// and multiplying an index by the quantum no longer describes anything.
    regions: Vec<BufferRegion>,
    prepared_nodes: Vec<PreparedNode>,
    parameter_targets: Vec<ParameterTarget>,
    parameter_addresses: Vec<ParameterAddress>,
    note_targets: Vec<NoteTarget>,
    note_addresses: Vec<NoteAddress>,
    channel_layout: ChannelLayout,
    sample_rate: SampleRate,
    maximum_block_size: FrameCount,
    max_events_per_quantum: EventCount,
    forward_event_horizon: FrameCount,
    added_latency: FrameCount,
}

impl CompiledPlan {
    /// Assemble a plan. Called by admission and by nothing else.
    #[allow(
        clippy::too_many_arguments,
        reason = "a prepared plan carries exactly the capacities admission copied into it; \
                  bundling them would hide which ones the renderer depends on"
    )]
    pub(crate) const fn new(
        id: PlanId,
        ops: Vec<PlanOp>,
        regions: Vec<BufferRegion>,
        prepared_nodes: Vec<PreparedNode>,
        parameter_targets: Vec<ParameterTarget>,
        parameter_addresses: Vec<ParameterAddress>,
        note_targets: Vec<NoteTarget>,
        note_addresses: Vec<NoteAddress>,
        channel_layout: ChannelLayout,
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        max_events_per_quantum: EventCount,
        forward_event_horizon: FrameCount,
        added_latency: FrameCount,
    ) -> Self {
        Self {
            id,
            ops,
            regions,
            prepared_nodes,
            parameter_targets,
            parameter_addresses,
            note_targets,
            note_addresses,
            channel_layout,
            sample_rate,
            maximum_block_size,
            max_events_per_quantum,
            forward_event_horizon,
            added_latency,
        }
    }

    /// This plan's identity, which every slot it hands out carries.
    pub const fn id(&self) -> PlanId {
        self.id
    }

    /// The operations, in execution order.
    pub fn ops(&self) -> &[PlanOp] {
        &self.ops
    }

    /// How many distinct buffers the plan needs.
    ///
    /// A **count**, not a size: since ADR-0041 the buffers differ in width, so the
    /// memory the arena takes is [`Self::arena_samples`] and this is what the schedule
    /// and the conversion accounting speak of — clause 9's "the plan's buffer count".
    pub const fn buffer_count(&self) -> usize {
        self.regions.len()
    }

    /// Where one slot's samples live, or `None` if the plan has no such slot.
    ///
    /// Off the audio thread as well as on it: the renderer resolves a step's regions
    /// through [`crate::node::kernels::bind`], which reads this table.
    #[must_use]
    pub fn region(&self, slot: BufferSlot) -> Option<BufferRegion> {
        self.regions.get(slot.index()).copied()
    }

    /// Every slot's region, indexed by [`BufferSlot`].
    pub fn regions(&self) -> &[BufferRegion] {
        &self.regions
    }

    /// How many samples the arena holds: the greatest `offset + length` assigned.
    ///
    /// ADR-0041 clause 13's **exclusive end**, which is the only reading that yields a
    /// sample count. The renderer allocates exactly this, and admission reports it.
    #[must_use]
    pub fn arena_samples(&self) -> usize {
        self.regions
            .iter()
            .map(|region| region.end())
            .max()
            .unwrap_or(0)
    }

    /// Every node's immutable prepared data, indexed by [`NodeSlot`].
    ///
    /// The renderer builds one mutable state per entry at preparation, so the two tables
    /// stay parallel without either of them being a count the other trusts.
    pub fn prepared_nodes(&self) -> &[PreparedNode] {
        &self.prepared_nodes
    }

    /// Where each parameter slot lands.
    ///
    /// Indexed by [`ParameterSlot`] on the audio thread; never searched.
    pub fn parameter_targets(&self) -> &[ParameterTarget] {
        &self.parameter_targets
    }

    /// The slot an addressed parameter compiles to, or `None` if the plan has no such
    /// parameter.
    ///
    /// **Off the audio thread.** A caller resolves once — when it builds a timeline, or
    /// when a controller is bound — and sends slots thereafter. That an unknown address
    /// returns `None` here rather than being ignored at render time is the point: the
    /// renderer can no longer receive an event it silently does nothing with.
    #[must_use]
    pub fn resolve_parameter(&self, node: NodeId, parameter: ParameterId) -> Option<ParameterSlot> {
        self.parameter_addresses
            .iter()
            .find(|address| address.node == node && address.parameter == parameter)
            .map(|address| address.slot)
    }

    /// Every addressable parameter, for a caller building a binding table.
    pub fn parameter_addresses(&self) -> &[ParameterAddress] {
        &self.parameter_addresses
    }

    /// Where each note slot lands.
    ///
    /// Indexed by [`NoteSlot`] on the audio thread; never searched.
    pub fn note_targets(&self) -> &[NoteTarget] {
        &self.note_targets
    }

    /// The slot a playable node compiles to, or `None` if the plan has no such node.
    ///
    /// **Off the audio thread**, and for the same reason [`Self::resolve_parameter`] is:
    /// a caller resolves once and sends slots thereafter, so a node that cannot be played
    /// is refused where a caller can still be told about it rather than being an event
    /// the renderer silently does nothing with.
    #[must_use]
    pub fn resolve_note(&self, node: NodeId) -> Option<NoteSlot> {
        self.note_addresses
            .iter()
            .find(|address| address.node == node)
            .map(|address| address.slot)
    }

    /// Every playable node, for a caller building a binding table.
    pub fn note_addresses(&self) -> &[NoteAddress] {
        &self.note_addresses
    }

    /// The stream's channel layout.
    pub const fn channel_layout(&self) -> ChannelLayout {
        self.channel_layout
    }

    /// The stream's sample rate.
    pub const fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// The largest callback this plan was prepared for.
    pub const fn maximum_block_size(&self) -> FrameCount {
        self.maximum_block_size
    }

    /// Events one quantum may be presented with.
    pub const fn max_events_per_quantum(&self) -> EventCount {
        self.max_events_per_quantum
    }

    /// How far ahead an ingress event may be stamped.
    pub const fn forward_event_horizon(&self) -> FrameCount {
        self.forward_event_horizon
    }

    /// The latency this plan adds, which is ADR-0001 clause 7's constant `Q`.
    ///
    /// Charged unconditionally, including to a host whose callbacks are always whole
    /// multiples of the quantum and which would not otherwise need it — because a
    /// latency that varies with the caller's block pattern cannot be declared once
    /// or compensated statically.
    pub const fn added_latency(&self) -> FrameCount {
        self.added_latency
    }
}
