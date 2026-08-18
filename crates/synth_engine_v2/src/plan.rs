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
use crate::quantities::{ChannelIndex, ChannelLayout, EventCount, SampleRate};
use crate::time::FrameCount;

/// One buffer in the plan's arena, by index.
///
/// Phase 1 gives every source its own quantum-sized buffer. The preallocated arena
/// with liveness analysis, so that non-overlapping signal lifetimes share storage,
/// is Phase 2's work; anticipating it here would produce an arena with nothing to
/// analyse.
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
        inputs: [Option<BufferSlot>; MAX_INPUTS],
        in_place_safe: bool,
    ) -> Self {
        let mut step = Self {
            kernel,
            node,
            out,
            inputs,
            bindings: [InputBinding::Unpatched; MAX_INPUTS],
            order: [u8::MAX; MAX_INPUTS + 1],
            in_place_safe,
        };
        step.resolve();
        step
    }

    /// Work out what each input is, and the order the regions are borrowed in.
    fn resolve(&mut self) {
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

        // The regions to borrow, ascending. Three entries at most, so an insertion is
        // cheaper and clearer than a sort — and this runs once per compile either way.
        let mut regions: [(usize, u8); MAX_INPUTS + 1] = [(usize::MAX, u8::MAX); MAX_INPUTS + 1];
        let mut count = 0;
        let push = |slot: usize, role: u8, regions: &mut [(usize, u8)], count: &mut usize| {
            let mut position = *count;
            while position > 0 && regions.get(position - 1).is_some_and(|(at, _)| *at > slot) {
                let previous = regions
                    .get(position - 1)
                    .copied()
                    .unwrap_or((usize::MAX, u8::MAX));
                if let Some(entry) = regions.get_mut(position) {
                    *entry = previous;
                }
                position -= 1;
            }
            if let Some(entry) = regions.get_mut(position) {
                *entry = (slot, role);
            }
            *count += 1;
        };
        push(self.out.index(), 0, &mut regions, &mut count);
        for index in 0..MAX_INPUTS {
            if !matches!(self.bindings.get(index), Some(InputBinding::Distinct)) {
                continue;
            }
            let Some(Some(slot)) = self.inputs.get(index).copied() else {
                continue;
            };
            push(slot.index(), index as u8 + 1, &mut regions, &mut count);
        }

        self.order = [u8::MAX; MAX_INPUTS + 1];
        for (entry, (_, role)) in self.order.iter_mut().zip(regions.iter()) {
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
    /// Function-pointer equality is what it is: two identical functions may be merged to
    /// one address, and one function may have two addresses across codegen units. It is
    /// used to compare *schedules*, where both directions are acceptable — a schedule
    /// that differs in its slots is what a test is really asking about.
    fn same_kernel(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.kernel, other.kernel)
    }

    /// Rewrite the slots this step names, once the arena has assigned them.
    pub(crate) fn remap(&mut self, out: BufferSlot, inputs: [Option<BufferSlot>; MAX_INPUTS]) {
        self.out = out;
        self.inputs = inputs;
        // Resolved again rather than carried over: reuse is exactly what turns two
        // distinct slots into one, so a classification computed before the arena ran
        // would call an input distinct when it has just become the output's own.
        self.resolve();
    }
}

impl PartialEq for NodeStep {
    fn eq(&self, other: &Self) -> bool {
        self.same_kernel(other)
            && self.node == other.node
            && self.out == other.out
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
    /// Write one buffer to one channel of the stream.
    ///
    /// One operation per channel, which is what makes a conversion **visible**: a mono
    /// signal reaching a stereo output compiles to two of these naming the same source
    /// slot, so the duplication ADR-0002 clause 6 permits appears in the schedule
    /// instead of hiding inside a single output operation the way Phase 1's did. It is
    /// also what a wider layout extends to without changing the operation.
    OutputChannel {
        /// The buffer to write out.
        source: BufferSlot,
        /// Which channel of the stream it becomes.
        channel: ChannelIndex,
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
    buffer_count: usize,
    prepared_nodes: Vec<PreparedNode>,
    parameter_targets: Vec<ParameterTarget>,
    parameter_addresses: Vec<ParameterAddress>,
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
        buffer_count: usize,
        prepared_nodes: Vec<PreparedNode>,
        parameter_targets: Vec<ParameterTarget>,
        parameter_addresses: Vec<ParameterAddress>,
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
            buffer_count,
            prepared_nodes,
            parameter_targets,
            parameter_addresses,
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

    /// How many quantum-sized buffers the plan needs.
    pub const fn buffer_count(&self) -> usize {
        self.buffer_count
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
