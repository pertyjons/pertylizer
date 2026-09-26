//! Off-thread target binding for ADR-0075's first mixed-producer prerequisite.
//!
//! This value proves only target separation for one owned plan, fixed compiled stream
//! and live note slot. It does not prepare ingress, activate transport, or permit
//! mixed rendering.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::identity::{INDEX_SPACE, ProducerId};
use crate::ir::StealingPolicy;
use crate::plan::{CompiledPlan, NodeSlot, NoteSlot, ParameterSlot};
use crate::schedule::{AdmittedCompiledStream, CompiledPayload};

/// Why the first target-binding shape cannot be admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MixedTargetError {
    /// The plan does not have exactly one compiled and one live producer with usable ranges.
    #[error("mixed target binding requires one compiled and one live note producer")]
    ProducerShape,
    /// An authored source would be a second claimant on producer custody.
    #[error("mixed target binding does not admit authored sources")]
    AuthoredSource,
    /// Stealing can emit writes beyond the note target expansion.
    #[error("mixed target binding requires no voice stealing")]
    Stealing,
    /// The admitted stream or live slot belongs to another plan.
    #[error("mixed target binding contains a foreign plan identity")]
    ForeignPlan,
    /// A compiled writer outside note gate and magnitude expansion is unaccounted for.
    #[error("compiled event {event_index} writes outside note targets")]
    UnsupportedCompiledWriter {
        /// Position in the admitted compiled stream.
        event_index: usize,
    },
    /// A note slot or its parameter expansion cannot be mapped to a node instance.
    #[error("note slot {slot:?} has an unrepresentable target")]
    UnrepresentableTarget {
        /// The note slot being expanded.
        slot: NoteSlot,
    },
    /// Both producers can write the same node instance.
    #[error("both note producers can write node instance {node:?}")]
    SharedNode {
        /// The overlapping node instance.
        node: NodeSlot,
    },
}

/// A refused binding returns its plan, stream and live slot for a corrected retry.
#[derive(Debug, Error)]
#[error("{reason}")]
pub struct MixedTargetFailure {
    reason: MixedTargetError,
    plan: CompiledPlan,
    stream: AdmittedCompiledStream,
    live_slot: NoteSlot,
}

impl MixedTargetFailure {
    /// The validation rule that refused the binding.
    pub const fn reason(&self) -> MixedTargetError {
        self.reason
    }

    /// Recover every input the failed admission took.
    pub fn into_inputs(self: Box<Self>) -> (CompiledPlan, AdmittedCompiledStream, NoteSlot) {
        let failure = *self;
        (failure.plan, failure.stream, failure.live_slot)
    }
}

/// A fixed plan, compiled stream and live note slot with disjoint playable node instances.
///
/// Construct off the audio thread. The underlying mixed-ingress and activation refusals
/// remain in force; this artifact grants no runtime access by itself.
#[derive(Debug)]
#[must_use]
pub struct MixedTargetAdmission {
    plan: CompiledPlan,
    stream: AdmittedCompiledStream,
    live_slot: NoteSlot,
    live_producer: ProducerId,
    compiled_slots: Vec<NoteSlot>,
}

impl MixedTargetAdmission {
    /// Bind every note writer in the owned plan and compiled stream to the live note slot.
    pub fn admit(
        plan: CompiledPlan,
        stream: AdmittedCompiledStream,
        live_slot: NoteSlot,
    ) -> Result<Self, Box<MixedTargetFailure>> {
        let checked: Result<_, MixedTargetError> = (|| {
            if stream.plan() != plan.id() || live_slot.plan() != plan.id() {
                return Err(MixedTargetError::ForeignPlan);
            }
            if plan.stealing() != StealingPolicy::None {
                return Err(MixedTargetError::Stealing);
            }
            if !plan.authored_sources().is_empty() {
                return Err(MixedTargetError::AuthoredSource);
            }
            let ranges = plan.note_producer_ranges();
            let compiled = plan.compiled_note_producer();
            if ranges.len() != 2
                || ranges.iter().any(|range| range.get() == 0)
                || !matches!(compiled, Some(id) if id.as_u16() < 2)
            {
                return Err(MixedTargetError::ProducerShape);
            }
            let compiled = compiled.ok_or(MixedTargetError::ProducerShape)?;
            let live_index = 1 - compiled.as_u16();
            let live_producer = ProducerId::new(live_index);
            let compiled_start = if compiled.as_u16() == 0 {
                0
            } else {
                ranges[0].get()
            };
            let live_start = if live_index == 0 { 0 } else { ranges[0].get() };
            let compiled_end = compiled_start
                .checked_add(ranges[usize::from(compiled.as_u16())].get())
                .ok_or(MixedTargetError::ProducerShape)?;
            let live_end = live_start
                .checked_add(ranges[usize::from(live_index)].get())
                .ok_or(MixedTargetError::ProducerShape)?;
            if compiled_end > INDEX_SPACE || live_end > INDEX_SPACE {
                return Err(MixedTargetError::ProducerShape);
            }
            let compiled_range = compiled_start..compiled_end;
            let live_range = live_start..live_end;

            let mut compiled_slots = BTreeSet::new();
            for (event_index, event) in stream.events().iter().enumerate() {
                match event.payload() {
                    CompiledPayload::NoteOn { slot, .. }
                    | CompiledPayload::NoteOff { slot, .. }
                    | CompiledPayload::Expression { slot, .. }
                    | CompiledPayload::Bend { slot, .. } => {
                        compiled_slots.insert(slot);
                    }
                    CompiledPayload::Controller(_) | CompiledPayload::SetParameter { .. } => {
                        return Err(MixedTargetError::UnsupportedCompiledWriter { event_index });
                    }
                }
            }

            let mut compiled_nodes = BTreeSet::new();
            for slot in compiled_slots.iter().copied() {
                expand_slot(&plan, slot, compiled_range.clone(), &mut compiled_nodes)?;
            }
            let mut live_nodes = BTreeSet::new();
            expand_slot(&plan, live_slot, live_range, &mut live_nodes)?;
            if let Some(node) = compiled_nodes.intersection(&live_nodes).next().copied() {
                return Err(MixedTargetError::SharedNode { node });
            }

            Ok((live_producer, compiled_slots.into_iter().collect()))
        })();
        let (live_producer, compiled_slots) = match checked {
            Ok(bound) => bound,
            Err(reason) => {
                return Err(Box::new(MixedTargetFailure {
                    reason,
                    plan,
                    stream,
                    live_slot,
                }));
            }
        };
        Ok(Self {
            plan,
            stream,
            live_slot,
            live_producer,
            compiled_slots,
        })
    }

    /// The exact immutable plan whose target expansions were checked.
    pub const fn plan(&self) -> &CompiledPlan {
        &self.plan
    }

    /// The exact compiled stream that was checked.
    pub fn stream(&self) -> &AdmittedCompiledStream {
        &self.stream
    }

    /// The only live note slot that was checked.
    pub const fn live_slot(&self) -> NoteSlot {
        self.live_slot
    }

    /// The producer whose identity range the bound live slot occupies.
    pub const fn live_producer(&self) -> ProducerId {
        self.live_producer
    }

    /// Every note slot reached by the fixed compiled stream.
    pub fn compiled_slots(&self) -> &[NoteSlot] {
        &self.compiled_slots
    }
}

fn expand_slot(
    plan: &CompiledPlan,
    slot: NoteSlot,
    range: std::ops::Range<u32>,
    nodes: &mut BTreeSet<NodeSlot>,
) -> Result<(), MixedTargetError> {
    let invalid = || MixedTargetError::UnrepresentableTarget { slot };
    let target = plan.note_targets().get(slot.index()).ok_or_else(invalid)?;
    if slot.plan() != plan.id() {
        return Err(invalid());
    }
    expand_parameter(
        plan,
        slot,
        target.parameter,
        target.node,
        range.clone(),
        nodes,
    )?;
    let magnitudes = plan.note_magnitudes_of(slot);
    if magnitudes.len() != target.magnitudes.len() {
        return Err(invalid());
    }
    for magnitude in magnitudes {
        expand_parameter(
            plan,
            slot,
            magnitude.parameter,
            magnitude.node,
            range.clone(),
            nodes,
        )?;
    }
    Ok(())
}

fn expand_parameter(
    plan: &CompiledPlan,
    slot: NoteSlot,
    parameter: ParameterSlot,
    first_node: NodeSlot,
    range: std::ops::Range<u32>,
    nodes: &mut BTreeSet<NodeSlot>,
) -> Result<(), MixedTargetError> {
    let invalid = || MixedTargetError::UnrepresentableTarget { slot };
    if parameter.plan() != plan.id() {
        return Err(invalid());
    }
    let first = plan
        .parameter_targets()
        .get(parameter.index())
        .ok_or_else(invalid)?;
    if first.node != first_node {
        return Err(invalid());
    }
    let instances = usize::try_from(first.instances.get()).map_err(|_| invalid())?;
    if instances == 0 {
        return Err(invalid());
    }
    for index in range {
        let row = if instances <= 1 {
            parameter.index()
        } else {
            let index = usize::try_from(index).map_err(|_| invalid())?;
            if index >= instances {
                return Err(invalid());
            }
            parameter.index().checked_add(index).ok_or_else(invalid)?
        };
        let actual = plan.parameter_targets().get(row).ok_or_else(invalid)?;
        if actual.control != first.control || actual.instances != first.instances {
            return Err(invalid());
        }
        nodes.insert(actual.node);
    }
    Ok(())
}
