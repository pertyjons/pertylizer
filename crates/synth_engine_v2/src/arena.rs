//! The buffer arena: which signals share storage, decided once at compile time.
//!
//! [ADR-0005](../../../plans/v2/decisions/ADR-0005-buffer-liveness-strategy.md) is what
//! this module implements. Lowering emits operations over one buffer per producing
//! node; this pass computes each buffer's **live range** over the schedule and lets two
//! whose ranges do not overlap share one physical slot. Nothing here runs on the audio
//! thread, and the render loop never learns that reuse happened: it reads slot indices.
//!
//! # Why an aliasing bug needs a check rather than an ear
//!
//! Assigning one slot to two signals that are both live produces plausible audio, not a
//! panic. That is why clause 8 asks for two checks and this module provides both: the
//! structural one, which reads the assignment and fails when two overlapping ranges
//! share a slot, and the behavioural one, which compiles the same plan with reuse
//! *disabled* and requires bit-identical output.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::node::kernels::MAX_INPUTS;
use crate::plan::{BufferRegion, BufferSlot, PlanOp};

/// Whether the arena reuses storage.
///
/// [`ArenaPolicy::NoReuse`] exists for clause 8's behavioural check and for nothing
/// else. It is not reachable from a host profile, is not a supported configuration, and
/// would go with that check if the check ever went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArenaPolicy {
    /// Non-overlapping lifetimes share storage.
    Reuse,
    /// Every value keeps its own slot.
    NoReuse,
}

/// One value's life: where it is written, where it is last read, and what it got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chain {
    /// The operation that writes the value.
    pub(crate) first: usize,
    /// The last operation that reads it, or its write when nothing reads it.
    pub(crate) last: usize,
    /// The physical slot it was assigned.
    pub(crate) slot: BufferSlot,
    /// Where that slot's samples live.
    pub(crate) region: BufferRegion,
    /// The virtual buffers merged into this chain by in-place processing.
    pub(crate) members: Vec<usize>,
}

/// What the pass decided.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub(crate) struct Assignment {
    /// Where each physical slot's samples live, indexed by [`BufferSlot`].
    pub(crate) regions: Vec<BufferRegion>,
    /// Virtual buffer index to the physical slot it maps to.
    pub(crate) mapping: Vec<BufferSlot>,
    /// One entry per value chain, in schedule order.
    pub(crate) chains: Vec<Chain>,
}

/// The free list: physical regions, ordered by ascending offset, coalescing on release.
///
/// ADR-0041 clause 13 is what this implements, and it is a contract rather than an
/// adjective because mixed widths make every question equal slots answered implicitly —
/// which hole, what happens to the remainder, what happens when none fits — change the
/// memory a plan needs, and could change it between two compilations of the same plan.
///
/// The cost of a request is the number of **holes**, which is bounded by peak liveness
/// rather than by the size of the plan: a hole exists only between a released region and
/// a later one, and adjacent holes coalesce on release. That is deliberate; the equal-slot
/// scan this replaces used two heaps to avoid a quadratic pass over the live set, and the
/// same shape of defect would be available here if the free list were rebuilt per request.
#[derive(Debug, Default)]
struct FreeList {
    /// Disjoint, non-adjacent, ascending by offset.
    holes: Vec<BufferRegion>,
    /// One past the last assigned sample.
    extent: usize,
}

impl FreeList {
    /// First fit by ascending offset, splitting a larger hole and growing when none fits.
    fn take(&mut self, length: usize) -> BufferRegion {
        if let Some(index) = self.holes.iter().position(|hole| hole.length() >= length)
            && let Some(hole) = self.holes.get(index).copied()
        {
            let taken = BufferRegion::raw(hole.offset(), length);
            // A hole strictly larger than the request splits, and the remainder stays
            // free at the higher offset — a wider region is never handed to a narrower
            // signal whole, so no slack is silently carried.
            match hole.length() > length {
                true => {
                    if let Some(entry) = self.holes.get_mut(index) {
                        *entry = BufferRegion::raw(
                            hole.offset().saturating_add(length),
                            hole.length().saturating_sub(length),
                        );
                    }
                }
                false => {
                    // The hole is consumed whole. `remove` hands it back and nothing
                    // wants it; the binding says so rather than leaving a `#[must_use]`
                    // value dropped in passing.
                    let _consumed = self.holes.remove(index);
                }
            }
            return taken;
        }

        // Nothing fits: the region is appended at the current extent, exactly `length`
        // wide. Growth is the fallback, never the first choice, which is what makes reuse
        // the normal path.
        let region = BufferRegion::raw(self.extent, length);
        self.extent = region.end();
        region
    }

    /// Return a region, coalescing with any free region it abuts on either side.
    ///
    /// Coalescing is what makes the free list **canonical**: whatever order regions are
    /// released in, the result is the same set of maximal disjoint holes, so the
    /// assignment stays a pure function of the plan — ADR-0005 clause 3, which a digest
    /// comparison depends on.
    fn release(&mut self, region: BufferRegion) {
        let at = self
            .holes
            .iter()
            .position(|hole| hole.offset() > region.offset())
            .unwrap_or(self.holes.len());
        self.holes.insert(at, region);

        // Merge with the neighbour above, then with the one below. Two checks, because a
        // released region can close a gap on both sides at once.
        if let (Some(current), Some(next)) =
            (self.holes.get(at).copied(), self.holes.get(at + 1).copied())
            && current.end() == next.offset()
        {
            if let Some(entry) = self.holes.get_mut(at) {
                *entry = BufferRegion::raw(
                    current.offset(),
                    current.length().saturating_add(next.length()),
                );
            }
            let _merged = self.holes.remove(at + 1);
        }
        if at > 0
            && let (Some(previous), Some(current)) =
                (self.holes.get(at - 1).copied(), self.holes.get(at).copied())
            && previous.end() == current.offset()
        {
            if let Some(entry) = self.holes.get_mut(at - 1) {
                *entry = BufferRegion::raw(
                    previous.offset(),
                    previous.length().saturating_add(current.length()),
                );
            }
            let _merged = self.holes.remove(at);
        }
    }
}

/// The buffers an operation reads, then the one it writes.
///
/// Read off the step rather than off the node kind: a step names its slots, and that is
/// all liveness needs. It is why adding a node kind changes nothing in this file.
fn accesses(op: PlanOp) -> (Vec<usize>, Option<usize>) {
    match op {
        PlanOp::Node(step) => (
            step.inputs()
                .iter()
                .flatten()
                .map(|slot| slot.index())
                .collect(),
            Some(step.out().index()),
        ),
        PlanOp::Output { source } => (vec![source.index()], None),
    }
}

/// Whether an operation may write over the buffer it reads.
///
/// Clause 5's *first* condition, and the node declares it: a gain scales each sample
/// independently, so reading and writing one buffer changes nothing about its result,
/// while a copy exists to produce a second buffer and writing it over its own input
/// would leave one buffer where the plan needs two. The registry holds that answer, and
/// lowering copied it into the step.
const fn in_place_safe(op: PlanOp) -> bool {
    match op {
        PlanOp::Node(step) => step.in_place_safe(),
        PlanOp::Output { .. } => false,
    }
}

/// Assign physical regions to `ops`' virtual buffers.
///
/// `widths` is one entry per virtual buffer, in samples: `c * Q` for a signal of `c`
/// channels, per ADR-0041 clause 2. A missing or zero width is treated as one quantum's
/// worth of nothing to assign, which cannot arise from lowering — every virtual buffer it
/// emits has a layout — and is a compiler defect rather than a render-time condition.
pub(crate) fn assign(ops: &[PlanOp], widths: &[usize], policy: ArenaPolicy) -> Assignment {
    let virtual_count = widths.len();
    if policy == ArenaPolicy::NoReuse {
        // Clause 9's mode: every value keeps its own region, laid out consecutively, and
        // the chains still describe the same lives so the structural check reads the same
        // shape.
        let mapping: Vec<BufferSlot> = (0..virtual_count).map(BufferSlot::new).collect();
        let mut offset = 0_usize;
        let mut regions = Vec::with_capacity(virtual_count);
        for width in widths {
            regions.push(BufferRegion::raw(offset, *width));
            offset = offset.saturating_add(*width);
        }
        let chains = live_ranges(ops, virtual_count)
            .into_iter()
            .enumerate()
            .map(|(index, (first, last))| Chain {
                first,
                last,
                slot: BufferSlot::new(index),
                region: regions
                    .get(index)
                    .copied()
                    .unwrap_or(BufferRegion::raw(0, 0)),
                members: vec![index],
            })
            .collect();
        return Assignment {
            regions,
            mapping,
            chains,
        };
    }

    let ranges = live_ranges(ops, virtual_count);

    // Clause 2's merge: an in-place operation produces one value chain rather than two
    // values that happen to overlap. `chain_of[v]` is the chain a virtual buffer belongs
    // to, and merging is what makes the alias legal instead of an exception to the
    // overlap rule.
    let mut chain_of: Vec<usize> = (0..virtual_count).collect();
    for (index, op) in ops.iter().enumerate() {
        let (reads, writes) = accesses(*op);
        let (Some(out), Some(source)) = (writes, reads.first().copied()) else {
            continue;
        };
        // Clause 5's *second* condition: the input's live range has to end here. A node
        // that declares itself safe does not get in-place treatment where its input is
        // read again later.
        let input_ends_here = ranges.get(source).is_some_and(|(_, last)| *last == index);
        // ADR-0041 clause 14's third condition: in-place processing now also requires the
        // two layouts to be identical, which here is the one thing the widths say. A node
        // cannot write a stereo output over a mono input's region, and the compiler
        // allocates separately where the widths differ.
        let same_width = widths.get(source).copied() == widths.get(out).copied();
        if in_place_safe(*op) && input_ends_here && same_width {
            let target = chain_of.get(source).copied().unwrap_or(source);
            if let Some(entry) = chain_of.get_mut(out) {
                *entry = target;
            }
        }
    }

    // One chain per root, spanning every member's range.
    let mut chains: Vec<Chain> = Vec::new();
    let mut chain_index: Vec<Option<usize>> = vec![None; virtual_count];
    for virtual_slot in 0..virtual_count {
        let root = root_of(&chain_of, virtual_slot);
        let Some((first, last)) = ranges.get(virtual_slot).copied() else {
            continue;
        };
        match chain_index.get(root).copied().flatten() {
            Some(existing) => {
                if let Some(chain) = chains.get_mut(existing) {
                    chain.first = chain.first.min(first);
                    chain.last = chain.last.max(last);
                    chain.members.push(virtual_slot);
                }
            }
            None => {
                if let Some(entry) = chain_index.get_mut(root) {
                    *entry = Some(chains.len());
                }
                chains.push(Chain {
                    first,
                    last,
                    // Both replaced by the scan below; a chain never keeps these values.
                    slot: BufferSlot::new(usize::MAX),
                    region: BufferRegion::raw(0, 0),
                    members: vec![virtual_slot],
                });
            }
        }
    }

    // ADR-0041 clause 13's **total** order: ascending index, in the compiled schedule, of
    // the operation that writes each chain's first value. Every node operation writes one
    // output and no two chains begin at the same one, so the second key never decides
    // anything — it is there because a buffer nothing writes takes its start from its
    // first read, and two of those would otherwise be ordered by whichever the enumerator
    // reached first. Without a total order the free list is deterministic and the
    // *sequence of requests* is not, which is the same non-determinism one level up.
    let mut order: Vec<usize> = (0..chains.len()).collect();
    order.sort_by_key(|index| {
        chains
            .get(*index)
            .map_or((usize::MAX, usize::MAX), |chain| (chain.first, chain.last))
    });

    // The live set is popped in end order; the free list is `FreeList`'s, ordered by
    // ascending offset. A heap for the first because the obvious form is quadratic —
    // copying the live set for every chain moves hundreds of megabytes on a plan near
    // `max_nodes` — and a vector for the second because first fit *is* a scan, bounded by
    // the number of holes rather than by the size of the plan.
    let mut free = FreeList::default();
    let mut in_flight: BinaryHeap<Reverse<(usize, usize, usize)>> = BinaryHeap::new();
    let mut regions: Vec<BufferRegion> = Vec::new();
    for index in order {
        let Some(chain) = chains.get(index) else {
            continue;
        };
        let (first, last) = (chain.first, chain.last);
        // Every member of a chain has the same width: the merge above refuses to join two
        // whose widths differ, which is what clause 14's in-place rule requires.
        let length = chain
            .members
            .first()
            .and_then(|member| widths.get(*member).copied())
            .unwrap_or(0);

        // Everything whose last read is behind this chain's write is free again. Only the
        // expired entries are visited, and each returns its region to the free list, where
        // it coalesces with whatever it abuts.
        while let Some(Reverse((end, offset, hole))) = in_flight.peek().copied() {
            if end >= first {
                break;
            }
            in_flight.pop();
            free.release(BufferRegion::raw(offset, hole));
        }

        let region = free.take(length);
        // A slot **is** a storage: two chains handed the same offset and length are two
        // tenants of one buffer and share its identity, which is what makes
        // `buffer_count` a count of buffers rather than of values. A region split off a
        // larger hole is not that region, and gets its own.
        //
        // The scan is over *distinct* regions, which is bounded by peak liveness rather
        // than by the size of the plan, for the same reason the free list's is.
        let slot = match regions.iter().position(|existing| *existing == region) {
            Some(index) => BufferSlot::new(index),
            None => {
                regions.push(region);
                BufferSlot::new(regions.len().saturating_sub(1))
            }
        };
        if let Some(chain) = chains.get_mut(index) {
            chain.slot = slot;
            chain.region = region;
        }
        in_flight.push(Reverse((last, region.offset(), region.length())));
    }

    let mut mapping = vec![BufferSlot::new(0); virtual_count];
    for chain in &chains {
        for member in &chain.members {
            if let Some(entry) = mapping.get_mut(*member) {
                *entry = chain.slot;
            }
        }
    }

    let assignment = Assignment {
        regions,
        mapping,
        chains,
    };

    // Clause 8's structural check, run **here** rather than in a test over chosen
    // fixtures: every plan any test compiles passes through this function, so every one
    // of them is checked. It defines no release behaviour, which is the only thing a
    // debug assertion may not do.
    //
    // The whole assertion is gated, not just its effect: `debug_assert!` still
    // *name-resolves* its condition in a release build, so a call to a
    // `#[cfg(debug_assertions)]` helper inside one fails to compile with the assertion
    // compiled out. That is a defect only `--release` shows, which is the shape this
    // repository's release gate has been caught by before.
    #[cfg(debug_assertions)]
    assert!(
        overlapping_chains(&assignment).is_none(),
        "the arena gave two values that are live at the same time regions that intersect"
    );

    assignment
}

/// Follow the merge links to a chain's root.
fn root_of(chain_of: &[usize], mut slot: usize) -> usize {
    let mut steps = 0;
    while let Some(next) = chain_of.get(slot).copied() {
        if next == slot || steps > chain_of.len() {
            break;
        }
        slot = next;
        steps += 1;
    }
    slot
}

/// First write and last read, per virtual buffer.
fn live_ranges(ops: &[PlanOp], virtual_count: usize) -> Vec<(usize, usize)> {
    let mut ranges = vec![(usize::MAX, 0_usize); virtual_count];
    for (index, op) in ops.iter().enumerate() {
        let (reads, writes) = accesses(*op);
        if let Some(out) = writes
            && let Some(range) = ranges.get_mut(out)
        {
            range.0 = range.0.min(index);
            range.1 = range.1.max(index);
        }
        for read in reads {
            if let Some(range) = ranges.get_mut(read) {
                range.1 = range.1.max(index);
            }
        }
    }
    for range in &mut ranges {
        if range.0 == usize::MAX {
            // A buffer nothing writes still has to occupy a slot for the reads that
            // reference it, so its range starts where those reads do.
            range.0 = range.1;
        }
    }
    ranges
}

/// Rewrite `ops` to the physical slots, and resolve each step's borrow order over the
/// regions those slots were assigned.
pub(crate) fn rewrite(ops: &mut [PlanOp], mapping: &[BufferSlot], regions: &[BufferRegion]) {
    let physical =
        |slot: BufferSlot| -> BufferSlot { mapping.get(slot.index()).copied().unwrap_or(slot) };
    for op in ops.iter_mut() {
        match op {
            PlanOp::Node(step) => {
                let mut inputs = [None; MAX_INPUTS];
                for (entry, slot) in inputs.iter_mut().zip(step.inputs().iter()) {
                    *entry = slot.map(physical);
                }
                step.remap(physical(step.out()), inputs, regions);
            }
            PlanOp::Output { source } => *source = physical(*source),
        }
    }
}

/// Two chains whose regions intersect while both are live, if the assignment has any.
///
/// ADR-0005 clause 8's structural check, strengthened by ADR-0041 clause 14: slot
/// identity is no longer the question, because with mixed widths two *distinct* slots
/// can still share samples, and partial overlap is a defect the equal-slot arena could
/// not represent. It reads the assignment rather than the audio, so it fails on the
/// defect itself instead of on a symptom that a listener might not hear.
///
/// Debug-only by construction: it is a property of the assignment, and running it in a
/// release build would be checking the compiler against itself on every compile. The
/// `test` half of the gate is what keeps `cargo test --release` compiling, where
/// `debug_assertions` is off and the test module still imports this.
#[cfg(any(test, debug_assertions))]
pub(crate) fn overlapping_chains(assignment: &Assignment) -> Option<(Chain, Chain)> {
    // A sweep over the **schedule**, not a scan over pairs. The pairwise form is
    // quadratic, and this runs on every compile in a debug or test build: 16 383
    // independent producers would be 134 million comparisons for a graph the profile
    // admits, which would make the check the reason development builds are slow.
    //
    // Sweeping by space instead is the same trap wearing a different hat, and it is the
    // one reuse walks straight into: every chain that reuses one region shares its
    // offset, so "every later region that begins before this one ends" is every later
    // chain, with no overlap among them at all.
    //
    // So: walk the chains in the order their lives begin, keep the ones still live in a
    // list ordered by offset, and ask only whether the region starting now intersects a
    // *live* one. The live set is what the arena bounds — its regions are disjoint when
    // the invariant holds — so the search is a scan of at most the neighbours a region
    // can reach, and eviction is amortized.
    let mut order: Vec<usize> = (0..assignment.chains.len()).collect();
    order.sort_unstable_by_key(|index| {
        assignment
            .chains
            .get(*index)
            .map_or((usize::MAX, usize::MAX), |chain| (chain.first, chain.last))
    });

    // (offset, end, last, chain index), ordered by offset.
    let mut live: Vec<(usize, usize, usize, usize)> = Vec::new();
    for index in order {
        let Some(chain) = assignment.chains.get(index) else {
            continue;
        };
        // Live ranges are inclusive of both endpoints — ADR-0005 clause 1 — so a chain
        // whose last read *is* this operation is still live here.
        live.retain(|(_, _, last, _)| *last >= chain.first);

        let region = chain.region;
        let at = live.partition_point(|(offset, _, _, _)| *offset <= region.offset());
        // The neighbour below can reach forwards into this region, and the ones above
        // can begin inside it. Both are checked; the scan upwards stops at the first
        // region that starts past this one's end.
        if at > 0
            && let Some((_, end, _, other)) = live.get(at - 1)
            && *end > region.offset()
            && let (Some(chain), Some(other)) =
                (assignment.chains.get(index), assignment.chains.get(*other))
        {
            return Some((chain.clone(), other.clone()));
        }
        for (offset, _, _, other) in live.iter().skip(at) {
            if *offset >= region.end() {
                break;
            }
            if let (Some(chain), Some(other)) =
                (assignment.chains.get(index), assignment.chains.get(*other))
            {
                return Some((chain.clone(), other.clone()));
            }
        }

        live.insert(at, (region.offset(), region.end(), chain.last, index));
    }
    None
}
