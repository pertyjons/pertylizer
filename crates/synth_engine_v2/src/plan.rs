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
use crate::node::NoteMagnitude;
use crate::node::kernels::{ControlIndex, Kernel, MAX_INPUTS, PreparedNode};
use crate::quantities::{ChannelLayout, EventCount, HeldNoteCount, SampleRate, VoiceCount};
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

/// One row inside a plan's parameter target table.
///
/// A [`ParameterSlot`] addresses a whole parameter group and its writes fan out across
/// instances. A row is one instance's destination and cannot be used as that group address.
///
/// ```compile_fail
/// use synth_engine_v2::plan::{ParameterRow, ParameterSlot};
/// fn group_write(row: ParameterRow) { let _: ParameterSlot = row; }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct ParameterRow {
    plan: PlanId,
    index: usize,
}

impl ParameterRow {
    pub(crate) const fn new(plan: PlanId, index: usize) -> Self {
        Self { plan, index }
    }

    /// The plan whose target table this row indexes.
    pub const fn plan(self) -> PlanId {
        self.plan
    }

    /// The index of this one target row.
    pub const fn index(self) -> usize {
        self.index
    }
}

/// A checked contiguous set of instances within one parameter group.
/// Its indices are relative to the group's first [`ParameterSlot`] row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ParameterInstanceSpan {
    first: u32,
    len: u32,
}

impl ParameterInstanceSpan {
    pub(crate) fn checked(first: u32, len: u32, instances: VoiceCount) -> Option<Self> {
        (len > 0 && first.checked_add(len)? <= instances.get()).then_some(Self { first, len })
    }

    /// The first instance index within the group.
    pub const fn first(self) -> u32 {
        self.first
    }

    /// The number of instance rows in the span.
    pub const fn count(self) -> u32 {
        self.len
    }

    pub(crate) fn indices(self) -> std::ops::Range<u32> {
        self.first..self.first + self.len
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
/// ADR-0001 splits this deliberately, in clause 14 as ADR-0043 restated it:
/// *sample-positioned* effects — note-on, note-off, gate, retrigger — occur at the offset
/// its **render position** names within the quantum that renders it, while the
/// *control-rate* response begins at the first quantum boundary at or after that position. The split is a property of the **effect**, not of the message that carried
/// it, so it is declared by the node kind and compiled into the target rather than being
/// chosen by whichever payload a caller happened to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ControlRate {
    /// Evaluated once per quantum, at the boundary at or after the event's render
    /// position (clause 13).
    Quantum,
    /// Applied at the offset the event's render position names inside the quantum that
    /// renders it (clause 14, as ADR-0043 restated it).
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

/// One local voice position within a plan's admitted identity partition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VoiceInstanceIndex(usize);

impl VoiceInstanceIndex {
    pub(crate) const fn measured(index: usize) -> Self {
        Self(index)
    }

    pub(crate) const fn as_usize(self) -> usize {
        self.0
    }
}

/// Where lowering says one scheduled node step writes state or an output region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeRole {
    Unclassified,
    Global,
    Local(VoiceInstanceIndex),
    SharedSum,
}

/// One step of the schedule: which kernel runs, over which slots.
///
/// This is what [ADR-0004](../../plans/v2/decisions/ADR-0004-native-node-representation.md)
/// means by a prepared function table. The kernel is resolved from the node's kind once,
/// at admission; the render loop calls through the pointer and never learns what kind of
/// node it just ran. Adding a node kind adds a kernel and a registry entry, and adds
/// nothing here and nothing to the loop.
#[derive(Debug, Clone)]
#[must_use]
pub struct NodeStep {
    kernel: Kernel,
    node: NodeSlot,
    /// The prepared record the kernel reads, shared by every instance of the node.
    prepared: PreparedSlot,
    out: BufferSlot,
    /// The layout of the signal it writes.
    ///
    /// ADR-0041 clause 4: a kernel is **told** how many channels it has, and this is
    /// where the count comes from — the node's own output port, resolved at admission,
    /// rather than the stream's layout or the width of the region divided by the quantum.
    out_layout: ChannelLayout,
    io: Box<StepInputs>,
    in_place_safe: bool,
    role: NodeRole,
}

/// Immutable after admission; sized for every YAMS source without widening each plan operation.
#[derive(Debug, Clone, PartialEq)]
struct StepInputs {
    inputs: [Option<BufferSlot>; MAX_INPUTS],
    /// What each input resolves to, decided here rather than per quantum.
    bindings: [InputBinding; MAX_INPUTS],
    /// The regions to borrow, in ascending slot order: `0` is the output and `n` is
    /// input `n - 1`. `u8::MAX` ends the list.
    ///
    /// Ascending because the borrows are handed out by walking the arena forwards and
    /// splitting each region off in turn, which permits one mutable and up to 32 shared
    /// borrows without `unsafe`. Admission sorts the entries once for the hot walk.
    order: [u8; MAX_INPUTS + 1],
}

impl NodeStep {
    /// Immutable input bindings allocated once for this scheduled step.
    pub const fn input_bytes() -> u64 {
        size_of::<StepInputs>() as u64
    }

    /// A step.
    ///
    /// Admission builds these. It is public so that a harness can build one too — a step
    /// on its own is inert, because the only way to get a [`CompiledPlan`] is to compile
    /// a graph — and the ADR-0004 evidence harness needs one to bind an arena the way the
    /// renderer does.
    pub fn new(
        kernel: Kernel,
        node: NodeSlot,
        prepared: PreparedSlot,
        out: BufferSlot,
        out_layout: ChannelLayout,
        inputs: [Option<BufferSlot>; MAX_INPUTS],
        in_place_safe: bool,
    ) -> Self {
        let mut step = Self {
            kernel,
            node,
            prepared,
            out,
            out_layout,
            io: Box::new(StepInputs {
                inputs,
                bindings: [InputBinding::Unpatched; MAX_INPUTS],
                order: [u8::MAX; MAX_INPUTS + 1],
            }),
            in_place_safe,
            role: NodeRole::Unclassified,
        };
        // Ordered by slot index until the arena has run. Lowering's slots are virtual and
        // have no offset yet; [`Self::remap`] resolves the order again over the regions
        // they were assigned, which is the order the binding actually walks.
        step.resolve(&[]);
        step
    }

    /// Attach lowering's instance or shared-sum classification to this step.
    #[must_use = "the returned step carries its classified role"]
    pub(crate) fn with_role(mut self, role: NodeRole) -> Self {
        self.role = role;
        self
    }

    /// The scope this step itself writes, including inserted helper steps.
    pub(crate) const fn role(&self) -> NodeRole {
        self.role
    }

    /// Work out what each input is, and the order the regions are borrowed in.
    ///
    /// `regions` is the assignment's table, indexed by [`BufferSlot`]; it is empty while
    /// the slots are still virtual, and then the slot index stands in for the offset.
    /// After the arena has run, the order is by **offset** — with variable-width regions
    /// a higher slot index can sit lower in the arena, and the binding walks the
    /// allocation forwards.
    fn resolve(&mut self, regions: &[BufferRegion]) {
        self.io.bindings = [InputBinding::Unpatched; MAX_INPUTS];
        for index in 0..MAX_INPUTS {
            let Some(Some(slot)) = self.io.inputs.get(index).copied() else {
                continue;
            };
            let binding = if slot == self.out {
                InputBinding::InPlace
            } else {
                let mirrored = (0..index).find(|earlier| {
                    self.io.inputs.get(*earlier).copied().flatten() == Some(slot)
                        && matches!(self.io.bindings.get(*earlier), Some(InputBinding::Distinct))
                });
                match mirrored {
                    Some(earlier) => InputBinding::Mirrors(earlier as u8),
                    None => InputBinding::Distinct,
                }
            };
            if let Some(entry) = self.io.bindings.get_mut(index) {
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
            if !matches!(self.io.bindings.get(index), Some(InputBinding::Distinct)) {
                continue;
            }
            let Some(Some(slot)) = self.io.inputs.get(index).copied() else {
                continue;
            };
            push(position_of(slot), index as u8 + 1, &mut order, &mut count);
        }

        self.io.order = [u8::MAX; MAX_INPUTS + 1];
        for (entry, (_, role)) in self.io.order.iter_mut().zip(order.iter()) {
            *entry = *role;
        }
    }

    /// What each input resolved to.
    pub const fn bindings(&self) -> &[InputBinding; MAX_INPUTS] {
        &self.io.bindings
    }

    /// The regions to borrow, in ascending slot order.
    pub const fn order(&self) -> &[u8; MAX_INPUTS + 1] {
        &self.io.order
    }

    /// The kernel this step calls.
    pub const fn kernel(&self) -> Kernel {
        self.kernel
    }

    /// Which prepared node and which state record.
    pub const fn node(&self) -> NodeSlot {
        self.node
    }

    /// The prepared record the step reads.
    pub const fn prepared(&self) -> PreparedSlot {
        self.prepared
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
        &self.io.inputs
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
        self.io.inputs = inputs;
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
            && self.io.inputs == other.io.inputs
            && self.io.bindings == other.io.bindings
            && self.io.order == other.io.order
            && self.in_place_safe == other.in_place_safe
            && self.role == other.role
    }
}

/// One operation, in execution order.
///
/// Three variants, and none of them is a node kind: a node kernel, the renderer's own
/// boundary, and — since `P07-S001` — the composition of one modulation edge into one
/// parameter row. The Phase 1 shape had one variant per node kind, which is what ADR-0004
/// clause 2 rejects: a node addition was a new arm inside the quantum loop.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanOp {
    /// Capture one shared source after all current-quantum reads (ADR-0033).
    FeedbackWrite {
        /// The boundary's history owner.
        node: NodeSlot,
        /// Stereo source kept live until this operation.
        source: BufferSlot,
    },
    /// Run one prepared node kernel.
    Node(NodeStep),
    /// Read one modulation source and compose it into one parameter row
    /// (`SOUND-INV-027`).
    Modulate(ModulationStep),
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

/// One modulation edge landing on one parameter row, as the renderer applies it
/// (`SOUND-INV-027`).
///
/// The source is a buffer the plan's pre-pass has already written this quantum, read at its
/// first frame; the depth is in the target law's units; and `last` marks the final edge
/// into the row, after which the row's accumulated sum is composed under its law and — for
/// a quantum-rate control — its segment advanced. Rows rather than slots, because a
/// voice-scope target has one row per instance and each instance reads its own source.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ModulationStep {
    source: BufferSlot,
    row: usize,
    depth: f32,
    last: bool,
}

impl ModulationStep {
    /// A step. Admission builds these.
    pub(crate) const fn new(source: BufferSlot, row: usize, depth: f32, last: bool) -> Self {
        Self {
            source,
            row,
            depth,
            last,
        }
    }

    /// The buffer whose first frame is the source's value this quantum.
    pub const fn source(&self) -> BufferSlot {
        self.source
    }

    /// The parameter row the contribution lands on.
    pub const fn row(&self) -> usize {
        self.row
    }

    /// The edge's depth, in the target law's units.
    pub const fn depth(&self) -> f32 {
        self.depth
    }

    /// Whether this is the last edge into its row this quantum.
    pub const fn last(&self) -> bool {
        self.last
    }

    /// Rebind the source to its physical slot, once the arena has assigned it.
    pub(crate) const fn remap(&mut self, source: BufferSlot) {
        self.source = source;
    }
}

/// A prepared record of a compiled plan, by index.
///
/// Distinct from [`NodeSlot`] since `P06-S001`: a voice-scope node has one prepared record
/// and `N` state records, one per voice instance, so the step that renders instance `k`
/// names its **state** by `NodeSlot` and its **prepared data** by this — and the prepared
/// data is shared, never cloned per voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct PreparedSlot(usize);

impl PreparedSlot {
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    /// The index into the plan's prepared records.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// One observation tap of a compiled plan, by index.
///
/// The twin of [`ParameterSlot`] for `SOUND-INV-022`: a subscription names one of these
/// rather than a node's internals, and it carries the plan identity for the same reason an
/// index resolved against another plan would read whatever occupies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct TapSlot {
    plan: PlanId,
    index: usize,
}

impl TapSlot {
    pub(crate) const fn new(plan: PlanId, index: usize) -> Self {
        Self { plan, index }
    }

    /// The plan the slot belongs to.
    pub const fn plan(self) -> PlanId {
        self.plan
    }

    /// The index into that plan's tap table.
    pub const fn index(self) -> usize {
        self.index
    }
}

/// What a declared tap names in the compiled plan: a stable signal point.
///
/// `SOUND-INV-022`: present whether or not anything subscribes, and passive — it is the
/// region the tapped node writes, read after the quantum renders. ADR-0005 clause 6 makes
/// it a **reader** of that region, so the arena keeps the region live to the end of the
/// quantum rather than handing it to a later chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct TapTarget {
    /// Which node instance.
    pub node: NodeSlot,
    /// The physical region its tapped output occupies.
    pub region: BufferSlot,
    /// What the tap carries.
    pub data: crate::node::TapData,
    /// The tap's declared cost: the bytes one quantum of it holds, from the port's layout.
    pub bytes_per_quantum: crate::quantities::QuantumBytes,
}

/// The identity of one mix channel in one plan (`SOUND-INV-031`).
///
/// Minted by the compiler, one per `Channel` node in ascending node identity, and carried
/// with the plan it names — as a [`ParameterSlot`] is — so a channel of one plan cannot
/// address another's. A mixer consumer keys a send, a meter or a sidechain by this and
/// never by an instrument's identity, which the master plan's Phase 8 requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct ChannelId {
    plan: PlanId,
    index: usize,
}

impl ChannelId {
    /// An identity. Crate-private for the reason [`ParameterSlot::new`] is.
    pub(crate) const fn new(plan: PlanId, index: usize) -> Self {
        Self { plan, index }
    }

    /// Which plan this channel belongs to.
    pub const fn plan(self) -> PlanId {
        self.plan
    }

    /// The channel's position among the plan's channels, in ascending node identity.
    pub const fn index(self) -> usize {
        self.index
    }
}

/// One mix channel the plan compiled, with the slots its three controls occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ChannelRecord {
    /// The channel's identity.
    pub id: ChannelId,
    /// The builder's tag for the channel's scope (`SOUND-INV-034`), under which its sends
    /// were placed.
    pub tag: crate::ir::ChannelTag,
    /// The node it was compiled from.
    pub node: NodeId,
    /// Its fader's slot.
    pub fader: ParameterSlot,
    /// Its pan's slot.
    pub pan: ParameterSlot,
    /// Its mute's slot.
    pub mute: ParameterSlot,
}

/// The identity of one bus in one plan (`SOUND-INV-034`).
///
/// Minted by the compiler, one per bus strip in ascending strip node identity, and carried
/// with the plan it names, as a [`ChannelId`] is. A send, a meter or a sidechain keys a bus
/// by this and never by a saved return's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct BusId {
    plan: PlanId,
    index: usize,
}

impl BusId {
    /// An identity. Crate-private for the reason [`ParameterSlot::new`] is.
    pub(crate) const fn new(plan: PlanId, index: usize) -> Self {
        Self { plan, index }
    }

    /// Which plan this bus belongs to.
    pub const fn plan(self) -> PlanId {
        self.plan
    }

    /// The bus's position among the plan's buses, in ascending strip identity.
    pub const fn index(self) -> usize {
        self.index
    }
}

/// One bus the plan compiled (`SOUND-INV-034`): its entry sum, its strip, and the slots the
/// strip's three controls occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct BusRecord {
    /// The bus's identity.
    pub id: BusId,
    /// The builder's tag for the bus's scope.
    pub tag: crate::ir::BusTag,
    /// The sum its sends enter.
    pub entry: NodeId,
    /// The strip: its fader, pan and mute.
    pub strip: NodeId,
    /// The strip's fader slot.
    pub fader: ParameterSlot,
    /// The strip's pan slot.
    pub pan: ParameterSlot,
    /// The strip's mute slot.
    pub mute: ParameterSlot,
}

/// Whose send a send is (`SOUND-INV-034`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum SendSource {
    /// A mix channel's.
    Channel(ChannelId),
    /// A bus's.
    Bus(BusId),
}

/// Where a send taps its source (`SOUND-INV-034`), as V1 names its two tap points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum SendTap {
    /// Before the fader: the signal the strip reads, times the level.
    PreFader,
    /// After the fader: a channel's gain composed with the level, or a bus's clipped output
    /// times the level.
    PostFader,
}

/// One send the plan compiled (`SOUND-INV-034`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SendRecord {
    /// The node it was compiled from.
    pub node: NodeId,
    /// Whose send it is.
    pub from: SendSource,
    /// Where it taps.
    pub tap: SendTap,
    /// The bus it enters.
    pub into: BusId,
    /// Its level's slot.
    pub level: ParameterSlot,
    /// Its mute's slot — the send's own on a `Send`, the channel's copy on a
    /// `PostFaderSend`.
    pub mute: ParameterSlot,
}

/// A declared tap by the identity a caller addresses it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct TapAddress {
    /// The node whose declaration names the tap.
    pub node: NodeId,
    /// The output port the tap names.
    pub port: crate::ir::PortId,
    /// The slot it compiled to.
    pub slot: TapSlot,
}

/// What a parameter event addresses, resolved to numeric slots at admission.
///
/// A node instance and one of its controls. Neither is an identity: the renderer indexes
/// its state table and hands the control index to the state, which is the last place the
/// meaning of "control 0" lives. Since `P05-S007a` a row also carries what the renderer's
/// parameter slot composes with — the law, the unit and the stored base — because the
/// slot is prepared from this table and the render loop reads nothing else about a kind.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ParameterTarget {
    /// Whether the declaration admits a controller replacement layer.
    pub controller: bool,
    /// Which node instance.
    pub node: NodeSlot,
    /// Which of its controls.
    pub control: ControlIndex,
    /// How its layers combine, `SOUND-INV-023`'s law, from the declaration.
    pub law: crate::node::ModulationLaw,
    /// The unit whose clamp follows the law's arithmetic.
    pub unit: crate::node::ParameterUnit,
    /// How long a new resolved value takes to be reached, `SOUND-INV-024`'s policy.
    pub smoothing: crate::node::Smoothing,
    /// How many rows this parameter's group holds — one per voice instance of its node, so
    /// `1` for a node outside the voice scope. The addressable [`ParameterSlot`] names the
    /// group's first row; a write fans out over the group, and a note's magnitude lands on
    /// the row of its own instance (`P06-S001`).
    pub instances: VoiceCount,
    /// The stored base: the value the node was prepared with, which is the authored one.
    ///
    /// What the slot's base layer starts as and what `SOUND-INV-018`'s catch-up restores
    /// for a target no write reached before the destination. One figure for both, here,
    /// so the two cannot come to differ.
    pub base: crate::quantities::ParameterValue,
    /// When moving it takes effect.
    ///
    /// Compiled from what the node kind declares, so the renderer reads it rather than
    /// deciding it. A gate is [`ControlRate::Sample`] however it was addressed, which is
    /// what keeps ADR-0001 clause 14 from being violable by choosing another payload.
    pub rate: ControlRate,
}

/// One prepared tuning of a plan, by index.
///
/// `SOUND-INV-021` requires a pitch-producing node to **reference** a prepared table rather
/// than copy it, and this is the reference: the table's bytes are charged once to the plan
/// and one of these is charged per node, so the resource report tells a second scale from a
/// second node. It carries the plan identity for the reason [`NoteSlot`] does — an index
/// resolved against another plan does not do nothing, it reads whatever occupies that index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct TuningSlot {
    plan: PlanId,
    index: usize,
}

impl TuningSlot {
    /// A slot. Crate-private for the same reason [`NoteSlot::new`] is.
    pub(crate) const fn new(plan: PlanId, index: usize) -> Self {
        Self { plan, index }
    }

    /// Which plan this slot indexes.
    pub const fn plan(self) -> PlanId {
        self.plan
    }

    /// The index into that plan's tuning table.
    pub const fn index(self) -> usize {
        self.index
    }
}

/// One prepared sample of a plan, by index (ADR-0026 clause 3).
///
/// The sampler's counterpart of [`TuningSlot`]: the sample's bytes are charged once to the
/// plan and one of these is charged per sampler node, and it carries the plan identity for
/// the same reason — an index resolved against another plan reads whatever occupies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct SampleSlot {
    plan: PlanId,
    index: usize,
}

impl SampleSlot {
    /// A slot. Crate-private for the same reason [`NoteSlot::new`] is.
    pub(crate) const fn new(plan: PlanId, index: usize) -> Self {
        Self { plan, index }
    }

    /// Which plan this slot indexes.
    pub const fn plan(self) -> PlanId {
        self.plan
    }

    /// The index into that plan's sample table.
    pub const fn index(self) -> usize {
        self.index
    }
}

/// Where one magnitude of a note-on lands, resolved at admission.
///
/// `SOUND-INV-021`'s expansion: a note-on is a gate **and** the magnitudes that describe the
/// note the gate starts, so it resolves to more than one control write. Admission collects
/// these from the node kinds within the played node's execution scope, which is what lets a
/// key reach an oscillator while the producer names only the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct NoteMagnitudeTarget {
    /// Which node instance.
    pub node: NodeSlot,
    /// Which of its controls: sample-positioned magnitudes or a quantum-rate note source.
    pub control: ControlIndex,
    /// The parameter slot of that control, through which the write is composed.
    ///
    /// `SOUND-INV-023`: a magnitude is an override-layer write like any other, and the
    /// renderer resolves it through the same slot a `SetParameter` to the control would use,
    /// so a modulation in force on the destination is not lost to a note's arrival.
    pub parameter: ParameterSlot,
    /// Which magnitude it receives.
    pub magnitude: NoteMagnitude,
    /// The tuning a [`NoteMagnitude::Pitch`] destination resolves its key through.
    ///
    /// `None` for a velocity destination, which resolves nothing: a velocity is already the
    /// value it is written as, while a key is a keyboard position the plan must map.
    pub tuning: Option<TuningSlot>,
}

/// What a note event addresses, resolved to numeric slots at admission.
///
/// The gate node and the control on it that a note edge moves, plus where in the plan's flat
/// magnitude table this note's expansion lives. Which control the gate is belongs to the node
/// kind: a caller plays a node, and the kind decides what being played means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct NoteTarget {
    /// Which node instance.
    pub node: NodeSlot,
    /// The control a note edge moves, always at [`ControlRate::Sample`].
    pub control: ControlIndex,
    /// The parameter slot of that control. A gate edge is composed through it like every
    /// other write, for the reason [`NoteMagnitudeTarget::parameter`] gives.
    pub parameter: ParameterSlot,
    /// Where this note's magnitude writes live in [`CompiledPlan::note_magnitudes`].
    ///
    /// A range into one flat table rather than a `Vec` per note, because the renderer reads
    /// it on the audio thread: an owned collection per target would put an indirection and a
    /// second allocation in a structure the render loop indexes.
    pub magnitudes: NoteMagnitudeRange,
}

/// Where one note target's magnitude writes live in the plan's flat table.
///
/// A **start and a length are two different quantities**, and exposing them as two `usize`
/// fields makes swapping them compile and puts an unchecked `start + len` at every reader.
/// Both are private here — the same shape [`BufferRegion`] uses for the arena, and for the
/// same reason — so the arithmetic exists once, inside
/// [`CompiledPlan::note_magnitudes_of`], where it is bounds-checked.
///
/// The range is still *visible* on [`NoteTarget`], and [`CompiledPlan::note_magnitudes`]
/// still hands out the whole table: a caller inspecting a plan needs both. What the private
/// fields remove is a reader **constructing an index** from them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct NoteMagnitudeRange {
    start: usize,
    length: usize,
}

impl NoteMagnitudeRange {
    /// An empty range, which is what a note target starts with and what a release expands to.
    pub const EMPTY: Self = Self {
        start: 0,
        length: 0,
    };

    /// A range the compiler has established over a table it just built.
    ///
    /// Crate-private, like [`BufferRegion::raw`] and for the same reason: the bounds come
    /// from the length of the table the entries were appended to, so the invariant holds by
    /// construction. Every read goes through [`CompiledPlan::note_magnitudes_of`], which
    /// bounds itself against the table rather than trusting this.
    pub(crate) const fn new(start: usize, length: usize) -> Self {
        Self { start, length }
    }

    /// How many magnitude writes the note expands to.
    pub const fn len(self) -> usize {
        self.length
    }

    /// Whether the note expands to none.
    pub const fn is_empty(self) -> bool {
        self.length == 0
    }
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
    prepared_scripts: Vec<crate::script::PreparedScript>,
    prepared_nodes: Vec<PreparedNode>,
    /// Each authored node's declared latency, tail and history at the plan's rate, in
    /// ascending node identity (`SOUND-INV-033`) — the diagnostics reader of the
    /// declaration's timing fields.
    node_timings: Vec<(crate::ir::NodeId, crate::node::NodeTiming)>,
    parameter_targets: Vec<ParameterTarget>,
    parameter_addresses: Vec<ParameterAddress>,
    /// `SOUND-INV-022`'s taps, derived from the nodes' declarations; indexed by [`TapSlot`].
    taps: Vec<TapTarget>,
    tap_addresses: Vec<TapAddress>,
    channels: Vec<ChannelRecord>,
    /// The buses, in ascending strip identity (`SOUND-INV-034`).
    buses: Vec<BusRecord>,
    /// The sends, in ascending node identity (`SOUND-INV-034`).
    sends: Vec<SendRecord>,
    note_targets: Vec<NoteTarget>,
    note_addresses: Vec<NoteAddress>,
    /// Every note target's magnitude writes, flattened.
    ///
    /// One table rather than a collection per target, so the renderer indexes into it with
    /// the range its note target names and never follows a per-note allocation.
    note_magnitudes: Vec<NoteMagnitudeTarget>,
    /// The distinct prepared tunings this plan resolves keys through.
    ///
    /// Deduplicated at admission by comparing the prepared tables themselves, which is what
    /// `SOUND-INV-021`'s "one prepared value exists per distinct tuning" means once two
    /// scopes may name one scale. **Not** by digest: a digest is a 64-bit hash, and two
    /// scales colliding on one would silently share a table and resolve every key of the
    /// second through the first.
    prepared_tunings: Vec<crate::tuning::PreparedTuning>,
    /// The distinct prepared samples this plan's samplers read (ADR-0026 clause 3).
    ///
    /// Deduplicated at admission by comparing the frames, as the tunings are; the frames
    /// sit behind an `Arc`, so a clone of the plan shares them.
    prepared_samples: Vec<crate::sample::PreparedSample>,
    channel_layout: ChannelLayout,
    sample_rate: SampleRate,
    maximum_block_size: FrameCount,
    max_events_per_quantum: EventCount,
    compiled_event_share: EventCount,
    /// The identity range admitted to each note-on producer, in declaration order.
    ///
    /// Carried by the plan for the reason every other capacity is: `HOST-INV-001` keeps the
    /// profile off the audio thread, so admission copies what the renderer's side needs. A
    /// renderer builds its identity table from this, and a producer's position here is its
    /// `ProducerId`.
    note_producer_ranges: Vec<HeldNoteCount>,
    /// The release-hold entitlement admitted to each note-on producer, in the same order.
    ///
    /// ADR-0046 clause 6 partitions `release_hold_capacity` into disjoint per-producer
    /// entitlements at plan admission, and "no producer borrows another's unused holds".
    /// Carried beside the identity ranges rather than derived from them, because they
    /// bound different things: a range bounds occurrences, an entitlement bounds
    /// obligations, and a compiled producer declares the first and none of the second.
    note_producer_holds: Vec<EventCount>,
    /// Admitted authored source declarations, retaining their authored order and envelopes.
    ///
    /// ADR-0046 clause 6 makes hold entitlements disjoint across admitted non-compiled
    /// producers, and an entitlement is per producer rather than per claimant. Admission
    /// checks that a producer's authored sources fit its entitlement, but admission cannot
    /// see the renderer-ingress stores prepared later — so without this list the live store
    /// could claim the same producer and spend the same holds a second time. An independent
    /// review found exactly that: the authored link proved the index resolved and was
    /// non-compiled, which is not the same as proving it was *unclaimed*.
    authored_sources: Vec<crate::ir::AuthoredSourceDeclaration>,
    compiled_note_producer: Option<crate::identity::ProducerId>,
    forward_event_horizon: FrameCount,
    path_latencies: crate::latency::PathLatencies,
    /// ADR-0058: what a note-on does when its producer holds every admitted index.
    stealing: crate::ir::StealingPolicy,
    /// The first step of every group of `N` instance steps — each voice-scope node's and
    /// each voice sum's — so a steal can address one instance's steps by `first + instance`.
    instance_groups: Vec<NodeSlot>,
    /// The subset of [`Self::instance_groups`] that are voice sums, whose steps carry the
    /// taken voice's fade.
    sum_groups: Vec<NodeSlot>,
    /// How many leading operations are the **pre-pass**: the modulation sources' steps and
    /// every [`PlanOp::Modulate`], run before the quantum's positioned writes are placed so
    /// that a write composes with this quantum's modulation (`SOUND-INV-027`).
    prepass: usize,
}

impl CompiledPlan {
    /// Additional retained table capacity for an owning host. Compiler resource rows
    /// cover DSP payloads/scratch, not every Vec backing and the shared plan container.
    /// Some outer tables overlap those rows; charging both is deliberately conservative.
    #[cfg(feature = "simulated-ingress")]
    pub(crate) fn host_table_bytes(&self) -> Option<u64> {
        let mut bytes = u64::try_from(size_of::<Self>() + 256).ok()?;
        bytes = bytes.checked_add(
            u64::try_from(self.ops.capacity().checked_mul(size_of::<PlanOp>())?).ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.regions
                    .capacity()
                    .checked_mul(size_of::<BufferRegion>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.prepared_scripts
                    .capacity()
                    .checked_mul(size_of::<crate::script::PreparedScript>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.prepared_nodes
                    .capacity()
                    .checked_mul(size_of::<PreparedNode>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.node_timings
                    .capacity()
                    .checked_mul(size_of::<(crate::ir::NodeId, crate::node::NodeTiming)>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.parameter_targets
                    .capacity()
                    .checked_mul(size_of::<ParameterTarget>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.parameter_addresses
                    .capacity()
                    .checked_mul(size_of::<ParameterAddress>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(self.taps.capacity().checked_mul(size_of::<TapTarget>())?).ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.tap_addresses
                    .capacity()
                    .checked_mul(size_of::<TapAddress>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.channels
                    .capacity()
                    .checked_mul(size_of::<ChannelRecord>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(self.buses.capacity().checked_mul(size_of::<BusRecord>())?).ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(self.sends.capacity().checked_mul(size_of::<SendRecord>())?).ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.note_targets
                    .capacity()
                    .checked_mul(size_of::<NoteTarget>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.note_addresses
                    .capacity()
                    .checked_mul(size_of::<NoteAddress>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.note_magnitudes
                    .capacity()
                    .checked_mul(size_of::<NoteMagnitudeTarget>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.prepared_tunings
                    .capacity()
                    .checked_mul(size_of::<crate::tuning::PreparedTuning>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.prepared_samples
                    .capacity()
                    .checked_mul(size_of::<crate::sample::PreparedSample>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.note_producer_ranges
                    .capacity()
                    .checked_mul(size_of::<HeldNoteCount>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.note_producer_holds
                    .capacity()
                    .checked_mul(size_of::<EventCount>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.authored_sources
                    .capacity()
                    .checked_mul(size_of::<crate::ir::AuthoredSourceDeclaration>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.instance_groups
                    .capacity()
                    .checked_mul(size_of::<NodeSlot>())?,
            )
            .ok()?,
        )?;
        bytes = bytes.checked_add(
            u64::try_from(
                self.sum_groups
                    .capacity()
                    .checked_mul(size_of::<NodeSlot>())?,
            )
            .ok()?,
        )?;
        bytes.checked_add(self.path_latencies.prepared_bytes())
    }

    pub(crate) fn install_scripts(&mut self, scripts: Vec<crate::script::PreparedScript>) {
        self.prepared_scripts = scripts.into_boxed_slice().into_vec();
    }
    pub fn prepared_scripts(&self) -> &[crate::script::PreparedScript] {
        &self.prepared_scripts
    }

    #[cfg(test)]
    pub(crate) fn script_bytes_held(&self) -> usize {
        self.prepared_scripts.capacity() * size_of::<crate::script::PreparedScript>()
            + self
                .prepared_scripts
                .iter()
                .map(crate::script::PreparedScript::dynamic_bytes_held)
                .sum::<usize>()
    }

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
        node_timings: Vec<(crate::ir::NodeId, crate::node::NodeTiming)>,
        parameter_targets: Vec<ParameterTarget>,
        parameter_addresses: Vec<ParameterAddress>,
        taps: Vec<TapTarget>,
        tap_addresses: Vec<TapAddress>,
        channels: Vec<ChannelRecord>,
        buses: Vec<BusRecord>,
        sends: Vec<SendRecord>,
        note_targets: Vec<NoteTarget>,
        note_addresses: Vec<NoteAddress>,
        note_magnitudes: Vec<NoteMagnitudeTarget>,
        prepared_tunings: Vec<crate::tuning::PreparedTuning>,
        prepared_samples: Vec<crate::sample::PreparedSample>,
        channel_layout: ChannelLayout,
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        max_events_per_quantum: EventCount,
        compiled_event_share: EventCount,
        note_producer_ranges: Vec<HeldNoteCount>,
        note_producer_holds: Vec<EventCount>,
        authored_sources: Vec<crate::ir::AuthoredSourceDeclaration>,
        compiled_note_producer: Option<crate::identity::ProducerId>,
        forward_event_horizon: FrameCount,
        path_latencies: crate::latency::PathLatencies,
        stealing: crate::ir::StealingPolicy,
        instance_groups: Vec<NodeSlot>,
        sum_groups: Vec<NodeSlot>,
        prepass: usize,
    ) -> Self {
        Self {
            id,
            ops,
            regions,
            prepared_scripts: Vec::new(),
            prepared_nodes,
            node_timings,
            parameter_targets,
            parameter_addresses,
            taps,
            tap_addresses,
            channels,
            buses,
            sends,
            note_targets,
            note_addresses,
            note_magnitudes,
            prepared_tunings,
            prepared_samples,
            channel_layout,
            sample_rate,
            maximum_block_size,
            max_events_per_quantum,
            compiled_event_share,
            note_producer_ranges,
            note_producer_holds,
            authored_sources,
            compiled_note_producer,
            forward_event_horizon,
            path_latencies,
            stealing,
            instance_groups,
            sum_groups,
            prepass,
        }
    }

    /// How many leading operations the pre-pass holds (`SOUND-INV-027`): the modulation
    /// sources and every modulation step, in dependency order. `ops()[..prepass_ops()]` runs
    /// before the quantum's positioned writes are placed; the rest runs after.
    pub const fn prepass_ops(&self) -> usize {
        self.prepass
    }

    /// How many parameter rows a modulation lands on whose control is sample-positioned —
    /// each receives one control at the quantum's first frame every quantum, and the
    /// timed-control scratch is sized on it. The renderer's side of
    /// [`crate::ir::GraphIr::modulated_sample_positioned_rows`], derived from the steps the
    /// lowering built; a test holds the two equal.
    pub fn modulated_sample_positioned_rows(&self) -> u32 {
        let rows = self
            .ops
            .iter()
            .filter_map(|op| match op {
                PlanOp::Modulate(step) if step.last() => Some(step.row()),
                _ => None,
            })
            .filter(|row| {
                self.parameter_targets
                    .get(*row)
                    .is_some_and(|target| matches!(target.rate, ControlRate::Sample))
            })
            .count();
        u32::try_from(rows).unwrap_or(u32::MAX)
    }

    /// ADR-0058's policy for a full producer.
    pub const fn stealing(&self) -> crate::ir::StealingPolicy {
        self.stealing
    }

    /// The first step of every `N`-instance group: a voice's steps are these plus its index.
    pub fn instance_groups(&self) -> &[NodeSlot] {
        &self.instance_groups
    }

    /// The voice-sum groups among [`Self::instance_groups`].
    pub fn sum_groups(&self) -> &[NodeSlot] {
        &self.sum_groups
    }

    /// The renderer's side of [`crate::ir::GraphIr::steal_expansion`]: a reset writes one
    /// control per instance group.
    pub fn steal_expansion(&self) -> crate::quantities::WritesPerNote {
        if !self.stealing.steals() {
            return crate::quantities::WritesPerNote::GATE_ONLY;
        }
        crate::quantities::WritesPerNote::at_least(
            u32::try_from(self.instance_groups.len()).unwrap_or(u32::MAX),
        )
    }

    /// This plan's identity, which every slot it hands out carries.
    pub const fn id(&self) -> PlanId {
        self.id
    }

    /// The operations, in execution order.
    pub fn ops(&self) -> &[PlanOp] {
        &self.ops
    }

    /// Resolve a runtime instance to its shared prepared record during host preparation.
    pub(crate) fn prepared_for_node(&self, node: NodeSlot) -> Option<&PreparedNode> {
        self.ops.iter().find_map(|op| match op {
            PlanOp::Node(step) if step.node() == node => {
                self.prepared_nodes.get(step.prepared().index())
            }
            _ => None,
        })
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

    /// Every authored node's declared timing at this plan's rate, in ascending node identity
    /// (`SOUND-INV-033`): what a node's kind imposes on its path and keeps across quanta.
    pub fn node_timings(&self) -> &[(crate::ir::NodeId, crate::node::NodeTiming)] {
        &self.node_timings
    }

    /// One node's declared timing, or `None` for a node the plan does not hold.
    #[must_use]
    pub fn timing_of(&self, node: crate::ir::NodeId) -> Option<crate::node::NodeTiming> {
        self.node_timings
            .iter()
            .find(|(id, _)| *id == node)
            .map(|(_, timing)| *timing)
    }

    /// The longest tail any node declares, or `None` where any node keeps signal without a
    /// stated rule (`SOUND-INV-033`); the plan-level reader of the declaration's tail.
    #[must_use]
    pub fn declared_tail(&self) -> Option<FrameCount> {
        self.node_timings
            .iter()
            .try_fold(FrameCount::ZERO, |longest, (_, timing)| {
                timing.tail.map(|tail| longest.max(tail))
            })
    }

    /// Where each parameter slot lands.
    ///
    /// Indexed by [`ParameterSlot`] on the audio thread; never searched.
    pub fn parameter_targets(&self) -> &[ParameterTarget] {
        &self.parameter_targets
    }

    /// Resolve one occurrence's destination inside an addressable parameter group.
    /// The renderer and off-thread mixed target binding use this one mapping.
    pub(crate) fn parameter_row_for_identity(
        &self,
        first: ParameterSlot,
        index: u16,
    ) -> Option<ParameterRow> {
        if first.plan() != self.id {
            return None;
        }
        let target = self.parameter_targets.get(first.index())?;
        let instances = target.instances.get() as usize;
        let row = if instances <= 1 {
            first.index()
        } else {
            let voice = usize::from(index);
            if voice >= instances {
                return None;
            }
            first.index().checked_add(voice)?
        };
        self.parameter_targets.get(row)?;
        Some(ParameterRow::new(self.id, row))
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

    /// The plan's observation taps, `SOUND-INV-022`: one per declared tap per node,
    /// present whether or not anything subscribes. Indexed by [`TapSlot`].
    pub fn taps(&self) -> &[TapTarget] {
        &self.taps
    }

    /// How many voice instances the plan renders: one per identity index of its producers,
    /// the sum of their ranges, and at least one — the same derivation the IR makes, over the
    /// ranges admission copied in (`P06-S001`).
    pub fn voice_instances(&self) -> crate::quantities::VoiceCount {
        let indices = self
            .note_producer_ranges
            .iter()
            .fold(0_u32, |total, range| total.saturating_add(range.get()));
        crate::quantities::VoiceCount::measured(indices.max(1))
    }

    /// Every declared tap by node and port, for a subscriber resolving one.
    /// The mix channels the plan compiled, in ascending node identity (`SOUND-INV-031`).
    /// Admission counted them against `max_mix_channels`.
    pub fn channels(&self) -> &[ChannelRecord] {
        &self.channels
    }

    /// The buses the plan compiled, in ascending strip identity (`SOUND-INV-034`).
    /// Admission counted them against `max_buses`.
    pub fn buses(&self) -> &[BusRecord] {
        &self.buses
    }

    /// The sends the plan compiled, in ascending node identity (`SOUND-INV-034`). Admission
    /// counted each channel's against `max_sends_per_channel`.
    pub fn sends(&self) -> &[SendRecord] {
        &self.sends
    }

    pub fn tap_addresses(&self) -> &[TapAddress] {
        &self.tap_addresses
    }

    /// A channel meter, resolved by this plan's compiled channel identity.
    #[must_use]
    pub fn channel_tap(&self, channel: ChannelId) -> Option<TapSlot> {
        self.channels()
            .iter()
            .find(|record| record.id == channel)
            .and_then(|record| self.resolve_tap(record.node, crate::ir::PortId::FIRST))
    }

    /// A return meter, resolved by this plan's compiled bus identity.
    #[must_use]
    pub fn bus_tap(&self, bus: BusId) -> Option<TapSlot> {
        self.buses()
            .iter()
            .find(|record| record.id == bus)
            .and_then(|record| self.resolve_tap(record.strip, crate::ir::PortId::FIRST))
    }

    /// The tap a node's output port compiled to, or `None` where none is declared.
    #[must_use]
    pub fn resolve_tap(&self, node: NodeId, port: crate::ir::PortId) -> Option<TapSlot> {
        self.tap_addresses
            .iter()
            .find(|address| address.node == node && address.port == port)
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

    /// Every note target's magnitude writes, flattened.
    ///
    /// Indexed on the audio thread by the range a [`NoteTarget`] names; never searched.
    pub fn note_magnitudes(&self) -> &[NoteMagnitudeTarget] {
        &self.note_magnitudes
    }

    /// The magnitude writes one note slot expands to.
    ///
    /// Takes the **slot** rather than a [`NoteTarget`], and that is what makes it safe: a
    /// slot carries the plan that issued it, so a slot resolved against another plan is
    /// refused here instead of returning an in-bounds slice of unrelated entries. A bare
    /// target carries no provenance and could not be checked at all — an independent review
    /// found the earlier signature accepting one.
    ///
    /// The start-plus-length arithmetic exists once, here, and is bounds-checked. Real-time
    /// legal: two comparisons and one checked slice of a table the plan owns, with the empty
    /// slice for anything out of range rather than a panic the audio thread could not
    /// report.
    pub fn note_magnitudes_of(&self, slot: NoteSlot) -> &[NoteMagnitudeTarget] {
        if slot.plan() != self.id {
            return &[];
        }
        let Some(range) = self
            .note_targets
            .get(slot.index())
            .map(|row| row.magnitudes)
        else {
            return &[];
        };
        let Some(end) = range.start.checked_add(range.length) else {
            return &[];
        };
        self.note_magnitudes.get(range.start..end).unwrap_or(&[])
    }

    /// The distinct prepared tunings this plan resolves keys through.
    ///
    /// Indexed by [`TuningSlot`] on the audio thread, where the read is one array index into
    /// one prepared table — which is what makes resolving a key real-time legal.
    pub fn prepared_tunings(&self) -> &[crate::tuning::PreparedTuning] {
        &self.prepared_tunings
    }

    /// The distinct prepared samples this plan's samplers read, by [`SampleSlot`] index
    /// (ADR-0026). Indexed on the audio thread through one array read.
    pub fn prepared_samples(&self) -> &[crate::sample::PreparedSample] {
        &self.prepared_samples
    }

    /// What one magnitude destination is written with, for a note naming `key` and `velocity`.
    ///
    /// **The one place a key becomes a frequency**, which is `SOUND-INV-021`'s "no node
    /// converts a key to a frequency on its own" made structural: the resolution is the
    /// plan's, through the prepared tuning the destination's node references, and a kernel
    /// receives an ordinary control value.
    ///
    /// Real-time legal: two array indexes and no arithmetic. `KeyIdentity` is `0..=127` by
    /// construction and a prepared table is 128 long, so the inner lookup cannot fail; the
    /// outer one can only fail on a slot from another plan, which is `None` rather than a
    /// substituted frequency — the audio thread has no honest fallback for "which note is
    /// this", and writing nothing leaves the previous value, which is at least a note the
    /// caller asked for at some point.
    ///
    /// Both conversions are infallible: a `Frequency` and a `NoteVelocity` are inside the
    /// finite floats a `ParameterValue` admits.
    #[must_use]
    pub fn magnitude_value(
        &self,
        magnitude: &NoteMagnitudeTarget,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Option<crate::quantities::ParameterValue> {
        match magnitude.magnitude {
            NoteMagnitude::Source(source) => Some(match source {
                crate::controller::NoteSource::Velocity => {
                    crate::quantities::ParameterValue::from_note_velocity(velocity)
                }
                crate::controller::NoteSource::NoteNumber => {
                    crate::quantities::ParameterValue::saturating(f32::from(key.as_u8()) / 127.0)
                }
                crate::controller::NoteSource::Pressure
                | crate::controller::NoteSource::ReleaseVelocity => {
                    crate::quantities::ParameterValue::ZERO
                }
            }),
            NoteMagnitude::Velocity => Some(crate::quantities::ParameterValue::from_note_velocity(
                velocity,
            )),
            // ADR-0026 clause 2: the on edge, unless the destination is a sampler whose one
            // zone the key or the velocity does not select — then nothing is written, the
            // note plays nothing on it, and the renderer counts the note as outside the
            // zone. Read from the prepared record the plan already holds, so the audio
            // thread decides it with two comparisons and no lookup elsewhere.
            NoteMagnitude::Trigger => match self.prepared_nodes.get(magnitude.node.index()) {
                Some(crate::node::kernels::PreparedNode::Sampler {
                    keys, velocities, ..
                }) if !(keys.holds(key) && velocities.holds(velocity)) => None,
                _ => Some(crate::quantities::ParameterValue::ONE),
            },
            NoteMagnitude::Pitch => {
                let slot = magnitude.tuning?;
                if slot.plan() != self.id {
                    return None;
                }
                let tuning = self.prepared_tunings.get(slot.index())?;
                Some(crate::quantities::ParameterValue::from_frequency(
                    tuning.frequency_of(key),
                ))
            }
        }
    }

    /// The most control writes any one of this plan's note-ons expands to, gate included.
    ///
    /// What admission charges the timed-control scratch with: `SOUND-INV-021` makes a note-on
    /// more than one write, and a scratch sized on one write per event would be overrun by
    /// the very first note whose scope declares a pitch and a velocity destination.
    /// How many rows one sample-positioned write fans out over, at most: the widest group
    /// among the `ControlRate::Sample` targets, and one where there is none.
    ///
    /// The renderer's side of [`crate::ir::GraphIr::sample_positioned_fan_out`]: derived from
    /// the target table the lowering built rather than copied in, so the scratch preparation
    /// takes is sized on what the renderer can actually be asked to write.
    pub fn sample_positioned_fan_out(&self) -> VoiceCount {
        self.parameter_targets
            .iter()
            .filter(|target| matches!(target.rate, ControlRate::Sample))
            .map(|target| target.instances)
            .max()
            .unwrap_or(VoiceCount::measured(1))
            .max(VoiceCount::measured(1))
    }

    pub fn max_writes_per_note(&self) -> crate::quantities::WritesPerNote {
        self.note_targets
            .iter()
            .map(|target| {
                crate::quantities::WritesPerNote::with_magnitudes(
                    u32::try_from(target.magnitudes.len()).unwrap_or(u32::MAX),
                )
            })
            .max()
            .unwrap_or(crate::quantities::WritesPerNote::GATE_ONLY)
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

    /// What the **compiled producer** may place in one destination quantum.
    ///
    /// Copied in at admission, like every other capacity the renderer's side needs, because
    /// `HOST-INV-001` keeps the profile away from the audio thread. It is not
    /// [`Self::max_events_per_quantum`]: ADR-0046 clause 1 partitions that cap across six
    /// producers, and the compiled class spends only its own share. Validating a schedule
    /// against the cap would admit a plan that faults at publication, which clause 3 forbids
    /// — a compiled runtime miss is a producer defect, so it has to be impossible for an
    /// admitted plan.
    pub const fn compiled_event_share(&self) -> EventCount {
        self.compiled_event_share
    }

    /// The identity range admitted to each note-on producer, in declaration order.
    ///
    /// A producer's position here **is** its `ProducerId`: the ranges are disjoint by
    /// construction, so an identity is attributable without carrying a producer tag.
    pub fn note_producer_ranges(&self) -> &[HeldNoteCount] {
        &self.note_producer_ranges
    }

    /// The release-hold entitlement admitted to each note-on producer, in declaration
    /// order.
    ///
    /// ADR-0046 clause 6's disjoint partition of `release_hold_capacity`. A compiled
    /// producer's entry is zero: clause 6 gives a compiled release the plan entitlement
    /// clause 4 established, so it needs no hold.
    pub fn note_producer_holds(&self) -> &[EventCount] {
        &self.note_producer_holds
    }

    /// Admitted authored source declarations, retaining their authored order and envelopes.
    ///
    /// A renderer-ingress store may not prepare against one of these: the authored source
    /// already holds that producer's entitlement, and clause 6 forbids two claimants sharing
    /// it. See the field for what an earlier revision let through.
    pub fn authored_sources(&self) -> &[crate::ir::AuthoredSourceDeclaration] {
        &self.authored_sources
    }

    /// Which producer owns the plan's compiled note events, if it has any.
    ///
    /// Validation refuses a second compiled producer, so this is a lookup rather than a
    /// search: `stamp_compiled` needs the `ProducerId` whose range a compiled note-on mints
    /// into, and inferring it from position would silently make producer 0 the compiled one
    /// in every plan that declares a runtime source first.
    pub const fn compiled_note_producer(&self) -> Option<crate::identity::ProducerId> {
        self.compiled_note_producer
    }

    /// How far ahead an ingress event may be stamped.
    pub const fn forward_event_horizon(&self) -> FrameCount {
        self.forward_event_horizon
    }

    /// Per-node path bounds and per-cable skew and compensation (`SOUND-INV-035`).
    pub const fn path_latencies(&self) -> &crate::latency::PathLatencies {
        &self.path_latencies
    }

    /// The offline presentation removes only the quantum carry. Graph delays remain
    /// audible, independently of alignment policy; positioned controls keep processing time.
    pub const fn offline_trim(&self) -> FrameCount {
        FrameCount::QUANTUM
    }

    /// The maximum latency this plan adds: quantum carry plus the longest output path.
    /// Under `Decline`, path bounds can differ, so this is not an offline trim quantity.
    ///
    /// Charged unconditionally, including to a host whose callbacks are always whole
    /// multiples of the quantum and which would not otherwise need it — because a
    /// latency that varies with the caller's block pattern cannot be declared once
    /// or compensated statically.
    pub const fn added_latency(&self) -> FrameCount {
        // Compilation refuses overflow before constructing the plan.
        FrameCount::new(FrameCount::QUANTUM.as_u64() + self.path_latencies.output().as_u64())
    }
}
