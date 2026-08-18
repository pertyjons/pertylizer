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
use crate::plan::{BufferSlot, PlanOp};

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
    /// The virtual buffers merged into this chain by in-place processing.
    pub(crate) members: Vec<usize>,
}

/// What the pass decided.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub(crate) struct Assignment {
    /// How many physical slots the arena needs.
    pub(crate) physical: usize,
    /// Virtual buffer index to the physical slot it maps to.
    pub(crate) mapping: Vec<BufferSlot>,
    /// One entry per value chain, in schedule order.
    pub(crate) chains: Vec<Chain>,
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
        PlanOp::OutputChannel { source, .. } => (vec![source.index()], None),
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
        PlanOp::OutputChannel { .. } => false,
    }
}

/// Assign physical slots to `ops`' virtual buffers.
pub(crate) fn assign(ops: &[PlanOp], virtual_count: usize, policy: ArenaPolicy) -> Assignment {
    if policy == ArenaPolicy::NoReuse {
        // Clause 9's mode: every value keeps its own slot, and the chains still describe
        // the same lives so the structural check reads the same shape.
        let mapping: Vec<BufferSlot> = (0..virtual_count).map(BufferSlot::new).collect();
        let chains = live_ranges(ops, virtual_count)
            .into_iter()
            .enumerate()
            .map(|(index, (first, last))| Chain {
                first,
                last,
                slot: BufferSlot::new(index),
                members: vec![index],
            })
            .collect();
        return Assignment {
            physical: virtual_count,
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
        if in_place_safe(*op) && input_ends_here {
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
                    // Replaced by the scan below; a chain never keeps this value.
                    slot: BufferSlot::new(usize::MAX),
                    members: vec![virtual_slot],
                });
            }
        }
    }

    // A linear scan with a free list, over chains ordered by where they begin. The order
    // is a function of the schedule, which is a function of node identity, so two
    // compilations of one plan assign identically — clause 3, which a digest comparison
    // depends on.
    let mut order: Vec<usize> = (0..chains.len()).collect();
    order.sort_by_key(|index| {
        chains
            .get(*index)
            .map_or((usize::MAX, usize::MAX), |chain| (chain.first, chain.last))
    });

    // Two heaps rather than two vectors, because the obvious form is quadratic: copying
    // the live set for every chain turns a "linear scan" into one that moves hundreds of
    // megabytes on a plan near `max_nodes`. The live set is popped in end order and the
    // free list in slot order, so both are `O(log n)` per chain and the assignment stays
    // deterministic — the lowest free slot is always chosen.
    let mut free: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    let mut in_flight: BinaryHeap<Reverse<(usize, usize)>> = BinaryHeap::new(); // (last, slot)
    let mut physical = 0_usize;
    for index in order {
        let Some(chain) = chains.get(index) else {
            continue;
        };
        let (first, last) = (chain.first, chain.last);

        // Everything whose last read is behind this chain's write is free again. Only
        // the expired entries are visited.
        while let Some(Reverse((end, slot))) = in_flight.peek().copied() {
            if end >= first {
                break;
            }
            in_flight.pop();
            free.push(Reverse(slot));
        }

        let slot = match free.pop() {
            Some(Reverse(slot)) => slot,
            None => {
                let slot = physical;
                physical += 1;
                slot
            }
        };
        if let Some(chain) = chains.get_mut(index) {
            chain.slot = BufferSlot::new(slot);
        }
        in_flight.push(Reverse((last, slot)));
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
        physical,
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
        "the arena assigned one slot to two values that are live at the same time"
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

/// Rewrite `ops` to the physical slots.
pub(crate) fn rewrite(ops: &mut [PlanOp], mapping: &[BufferSlot]) {
    let physical =
        |slot: BufferSlot| -> BufferSlot { mapping.get(slot.index()).copied().unwrap_or(slot) };
    for op in ops.iter_mut() {
        match op {
            PlanOp::Node(step) => {
                let mut inputs = [None; MAX_INPUTS];
                for (entry, slot) in inputs.iter_mut().zip(step.inputs().iter()) {
                    *entry = slot.map(physical);
                }
                step.remap(physical(step.out()), inputs);
            }
            PlanOp::OutputChannel { source, .. } => *source = physical(*source),
        }
    }
}

/// Two chains that share a slot while both are live, if the assignment has any.
///
/// Clause 8's structural check. It reads the assignment rather than the audio, so it
/// fails on the defect itself instead of on a symptom that a listener might not hear.
/// Debug-only by construction: it is a property of the assignment, and running it in a
/// release build would be checking the compiler against itself on every compile. The
/// `test` half of the gate is what keeps `cargo test --release` compiling, where
/// `debug_assertions` is off and the test module still imports this.
#[cfg(any(test, debug_assertions))]
pub(crate) fn overlapping_chains(assignment: &Assignment) -> Option<(Chain, Chain)> {
    // Grouped by slot and sorted, not compared pairwise. The pairwise form is quadratic,
    // and this runs on **every** compile in a debug or test build — 16 383 independent
    // producers would be 134 million comparisons for a graph the profile admits, which
    // would make the check the reason development builds are slow.
    let mut by_slot: Vec<(usize, usize, usize)> = assignment
        .chains
        .iter()
        .enumerate()
        .map(|(index, chain)| (chain.slot.index(), chain.first, index))
        .collect();
    by_slot.sort_unstable();

    for pair in by_slot.windows(2) {
        let [
            (slot, _, first_index),
            (next_slot, next_first, second_index),
        ] = pair
        else {
            continue;
        };
        if slot != next_slot {
            continue;
        }
        let (Some(chain), Some(other)) = (
            assignment.chains.get(*first_index),
            assignment.chains.get(*second_index),
        ) else {
            continue;
        };
        // Sorted by start within a slot, so the only overlap that can exist is with the
        // neighbour: if this chain is still live where the next one begins, they share
        // storage while both hold a value.
        if chain.last >= *next_first {
            return Some((chain.clone(), other.clone()));
        }
    }
    None
}
