//! Off-thread target binding for ADR-0075's first mixed-producer prerequisite.
//!
//! This value proves only target separation for one owned plan, fixed compiled stream
//! and live note slot. It does not prepare ingress, activate transport, or permit
//! mixed rendering.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use thiserror::Error;

use crate::identity::{INDEX_SPACE, ProducerId, Range};
use crate::ir::StealingPolicy;
use crate::plan::{
    CompiledPlan, NodeRole, NodeSlot, NoteSlot, ParameterInstanceSpan, ParameterRow, ParameterSlot,
    PlanId, PlanOp, VoiceInstanceIndex,
};
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
    /// The plan's voice groups, parameter rows and admitted identity spans disagree.
    #[error("mixed voice instance partition cannot represent the admitted plan")]
    InstancePartition,
    /// A note destination is outside the producer's instance-local rows.
    #[error("note destination row {row:?} is outside its producer's instance partition")]
    DestinationOutsidePartition {
        /// The gate or magnitude destination that could not be owned.
        row: ParameterRow,
    },
}

/// The local node instances and rows each producer can address in one immutable plan.
///
/// Shared voice-sum steps and global nodes stay separate. This partition does not classify
/// influence from a global source into a local instance, and grants no mixed rendering or
/// activation.
#[derive(Debug)]
#[must_use]
pub struct MixedInstancePartition {
    plan: PlanId,
    compiled_producer: ProducerId,
    live_producer: ProducerId,
    compiled_span: Range,
    live_span: Range,
    compiled_nodes: Vec<NodeSlot>,
    live_nodes: Vec<NodeSlot>,
    shared_sum_nodes: Vec<NodeSlot>,
    global_nodes: Vec<NodeSlot>,
    compiled_rows: Vec<ParameterRow>,
    live_rows: Vec<ParameterRow>,
    shared_sum_rows: Vec<ParameterRow>,
    global_rows: Vec<ParameterRow>,
    restoration_groups: Vec<MixedRestorationGroup>,
}

/// One addressable group and the compiled producer's checked instance span in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct MixedRestorationGroup {
    parameter: ParameterSlot,
    instances: ParameterInstanceSpan,
}

impl MixedRestorationGroup {
    /// The group's first parameter row.
    pub const fn parameter(self) -> ParameterSlot {
        self.parameter
    }

    /// The compiled producer's relative instance span within that group.
    pub const fn instances(self) -> ParameterInstanceSpan {
        self.instances
    }
}

impl MixedInstancePartition {
    /// The plan whose nodes and parameter rows are partitioned.
    pub const fn plan_id(&self) -> PlanId {
        self.plan
    }

    /// The compiled producer whose identity span selects local instances.
    pub const fn compiled_producer(&self) -> ProducerId {
        self.compiled_producer
    }

    /// The live producer whose identity span selects local instances.
    pub const fn live_producer(&self) -> ProducerId {
        self.live_producer
    }

    /// Every local node instance selected by the compiled span, including rowless steps.
    pub fn compiled_nodes(&self) -> &[NodeSlot] {
        &self.compiled_nodes
    }

    /// Every local node instance selected by the live span, including rowless steps.
    pub fn live_nodes(&self) -> &[NodeSlot] {
        &self.live_nodes
    }

    /// Voice-sum steps that write one output shared by both producers.
    pub fn shared_sum_nodes(&self) -> &[NodeSlot] {
        &self.shared_sum_nodes
    }

    /// Nodes outside the voice instance groups.
    pub fn global_nodes(&self) -> &[NodeSlot] {
        &self.global_nodes
    }

    /// All parameter rows on compiled local node instances.
    pub fn compiled_rows(&self) -> &[ParameterRow] {
        &self.compiled_rows
    }

    /// All parameter rows on live local node instances.
    pub fn live_rows(&self) -> &[ParameterRow] {
        &self.live_rows
    }

    /// Parameter rows on shared voice-sum steps, if a future lowering creates any.
    pub fn shared_sum_rows(&self) -> &[ParameterRow] {
        &self.shared_sum_rows
    }

    /// Parameter rows on nodes outside the voice instance groups.
    pub fn global_rows(&self) -> &[ParameterRow] {
        &self.global_rows
    }

    /// Every addressable group whose compiled rows need scoped restoration.
    /// Each group appears once; event payload and Session-share costs still need measurement.
    pub fn restoration_groups(&self) -> &[MixedRestorationGroup] {
        &self.restoration_groups
    }

    pub(crate) const fn spans(&self) -> (Range, Range) {
        (self.compiled_span, self.live_span)
    }
}

/// Named transfer of the bound values into their two range owners.
pub(crate) struct MixedTargetParts {
    pub(crate) plan: Arc<CompiledPlan>,
    pub(crate) stream: AdmittedCompiledStream,
    pub(crate) live_slot: NoteSlot,
    pub(crate) compiled_slots: Vec<NoteSlot>,
    pub(crate) partition: Arc<MixedInstancePartition>,
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
    plan: Arc<CompiledPlan>,
    stream: AdmittedCompiledStream,
    live_slot: NoteSlot,
    live_producer: ProducerId,
    compiled_span: Range,
    live_span: Range,
    compiled_slots: Vec<NoteSlot>,
    partition: Arc<MixedInstancePartition>,
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
            let compiled_span = Range::checked(compiled_start, compiled_end - compiled_start)
                .ok_or(MixedTargetError::ProducerShape)?;
            let live_span = Range::checked(live_start, live_end - live_start)
                .ok_or(MixedTargetError::ProducerShape)?;
            let partition =
                build_partition(&plan, compiled, live_producer, compiled_span, live_span)?;

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
            let mut compiled_rows = BTreeSet::new();
            for slot in compiled_slots.iter().copied() {
                expand_slot(
                    &plan,
                    slot,
                    compiled_span,
                    &mut compiled_nodes,
                    &mut compiled_rows,
                )?;
            }
            let mut live_nodes = BTreeSet::new();
            let mut live_rows = BTreeSet::new();
            expand_slot(&plan, live_slot, live_span, &mut live_nodes, &mut live_rows)?;
            if let Some(node) = compiled_nodes.intersection(&live_nodes).next().copied() {
                return Err(MixedTargetError::SharedNode { node });
            }
            if let Some(row) = compiled_rows
                .iter()
                .find(|row| partition.compiled_rows.binary_search(row).is_err())
            {
                return Err(MixedTargetError::DestinationOutsidePartition { row: *row });
            }
            if let Some(row) = live_rows
                .iter()
                .find(|row| partition.live_rows.binary_search(row).is_err())
            {
                return Err(MixedTargetError::DestinationOutsidePartition { row: *row });
            }

            Ok((
                live_producer,
                compiled_span,
                live_span,
                compiled_slots.into_iter().collect(),
                partition,
            ))
        })();
        let (live_producer, compiled_span, live_span, compiled_slots, partition) = match checked {
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
            plan: Arc::new(plan),
            stream,
            live_slot,
            live_producer,
            compiled_span,
            live_span,
            compiled_slots,
            partition: Arc::new(partition),
        })
    }

    /// The exact immutable plan whose target expansions were checked.
    pub fn plan(&self) -> &CompiledPlan {
        &self.plan
    }

    pub(crate) const fn plan_arc(&self) -> &Arc<CompiledPlan> {
        &self.plan
    }

    pub(crate) const fn spans(&self) -> (Range, Range) {
        (self.compiled_span, self.live_span)
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

    /// The local instance and row partition derived from this bound plan.
    pub fn instance_partition(&self) -> &MixedInstancePartition {
        &self.partition
    }

    /// Move the checked values into the split stream constructor exactly once.
    pub(crate) fn into_parts(self) -> MixedTargetParts {
        MixedTargetParts {
            plan: self.plan,
            stream: self.stream,
            live_slot: self.live_slot,
            compiled_slots: self.compiled_slots,
            partition: self.partition,
        }
    }
}

fn build_partition(
    plan: &CompiledPlan,
    compiled_producer: ProducerId,
    live_producer: ProducerId,
    compiled_span: Range,
    live_span: Range,
) -> Result<MixedInstancePartition, MixedTargetError> {
    let roles: BTreeMap<_, _> = plan
        .ops()
        .iter()
        .filter_map(|op| match op {
            PlanOp::Node(step) => Some((step.node(), step.role())),
            _ => None,
        })
        .collect();
    let node_count = plan
        .ops()
        .iter()
        .filter(|op| matches!(op, PlanOp::Node(_)))
        .count();
    if roles.len() != node_count {
        return Err(MixedTargetError::InstancePartition);
    }
    let mut compiled_nodes = BTreeSet::new();
    let mut live_nodes = BTreeSet::new();
    let mut shared_sum_nodes = BTreeSet::new();
    let mut global_nodes = BTreeSet::new();
    let voices = usize::try_from(plan.voice_instances().get())
        .map_err(|_| MixedTargetError::InstancePartition)?;
    for (node, role) in &roles {
        match role {
            NodeRole::Local(index) => {
                if index.as_usize() >= voices {
                    return Err(MixedTargetError::InstancePartition);
                }
                let identity = u16::try_from(index.as_usize())
                    .map_err(|_| MixedTargetError::InstancePartition)?;
                match (
                    compiled_span.contains(identity),
                    live_span.contains(identity),
                ) {
                    (true, false) => {
                        compiled_nodes.insert(*node);
                    }
                    (false, true) => {
                        live_nodes.insert(*node);
                    }
                    _ => return Err(MixedTargetError::InstancePartition),
                }
            }
            NodeRole::SharedSum => {
                shared_sum_nodes.insert(*node);
            }
            NodeRole::Global => {
                global_nodes.insert(*node);
            }
            NodeRole::Unclassified => return Err(MixedTargetError::InstancePartition),
        }
    }
    let mut declared_sums = BTreeSet::new();
    for first in plan.instance_groups() {
        let shared = plan.sum_groups().contains(first);
        for index in 0..voices {
            let node = NodeSlot::new(
                first
                    .index()
                    .checked_add(index)
                    .ok_or(MixedTargetError::InstancePartition)?,
            );
            let expected = if shared {
                NodeRole::SharedSum
            } else {
                NodeRole::Local(VoiceInstanceIndex::measured(index))
            };
            if roles.get(&node) != Some(&expected) {
                return Err(MixedTargetError::InstancePartition);
            }
            if shared {
                declared_sums.insert(node);
            }
        }
    }
    if shared_sum_nodes != declared_sums {
        return Err(MixedTargetError::InstancePartition);
    }
    let mut compiled_rows = Vec::new();
    let mut live_rows = Vec::new();
    let mut shared_sum_rows = Vec::new();
    let mut global_rows = Vec::new();
    for (index, target) in plan.parameter_targets().iter().enumerate() {
        let row = ParameterRow::new(plan.id(), index);
        if compiled_nodes.contains(&target.node) {
            compiled_rows.push(row);
        } else if live_nodes.contains(&target.node) {
            live_rows.push(row);
        } else if shared_sum_nodes.contains(&target.node) {
            shared_sum_rows.push(row);
        } else if global_nodes.contains(&target.node) {
            global_rows.push(row);
        } else {
            return Err(MixedTargetError::InstancePartition);
        }
    }
    let mut restoration_groups = Vec::new();
    let mut restored_rows = BTreeSet::new();
    let compiled_indices = compiled_span.indices();
    for address in plan.parameter_addresses() {
        let target = plan
            .parameter_targets()
            .get(address.slot.index())
            .ok_or(MixedTargetError::InstancePartition)?;
        if target.instances != plan.voice_instances() {
            continue;
        }
        let span = ParameterInstanceSpan::checked(
            compiled_indices.start,
            compiled_indices.end - compiled_indices.start,
            target.instances,
        )
        .ok_or(MixedTargetError::InstancePartition)?;
        let first_index =
            u16::try_from(span.first()).map_err(|_| MixedTargetError::InstancePartition)?;
        let first_row = plan
            .parameter_row_for_identity(address.slot, first_index)
            .ok_or(MixedTargetError::InstancePartition)?;
        if compiled_rows.binary_search(&first_row).is_err() {
            continue;
        }
        for index in span.indices() {
            let index = u16::try_from(index).map_err(|_| MixedTargetError::InstancePartition)?;
            let row = plan
                .parameter_row_for_identity(address.slot, index)
                .ok_or(MixedTargetError::InstancePartition)?;
            if compiled_rows.binary_search(&row).is_err() || !restored_rows.insert(row) {
                return Err(MixedTargetError::InstancePartition);
            }
        }
        for index in live_span.indices() {
            let index = u16::try_from(index).map_err(|_| MixedTargetError::InstancePartition)?;
            let row = plan
                .parameter_row_for_identity(address.slot, index)
                .ok_or(MixedTargetError::InstancePartition)?;
            if live_rows.binary_search(&row).is_err() {
                return Err(MixedTargetError::InstancePartition);
            }
        }
        restoration_groups.push(MixedRestorationGroup {
            parameter: address.slot,
            instances: span,
        });
    }
    if restored_rows.len() != compiled_rows.len() {
        return Err(MixedTargetError::InstancePartition);
    }
    Ok(MixedInstancePartition {
        plan: plan.id(),
        compiled_producer,
        live_producer,
        compiled_span,
        live_span,
        compiled_nodes: compiled_nodes.into_iter().collect(),
        live_nodes: live_nodes.into_iter().collect(),
        shared_sum_nodes: shared_sum_nodes.into_iter().collect(),
        global_nodes: global_nodes.into_iter().collect(),
        compiled_rows,
        live_rows,
        shared_sum_rows,
        global_rows,
        restoration_groups,
    })
}

fn expand_slot(
    plan: &CompiledPlan,
    slot: NoteSlot,
    range: Range,
    nodes: &mut BTreeSet<NodeSlot>,
    rows: &mut BTreeSet<ParameterRow>,
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
        range,
        nodes,
        rows,
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
            range,
            nodes,
            rows,
        )?;
    }
    Ok(())
}

fn expand_parameter(
    plan: &CompiledPlan,
    slot: NoteSlot,
    parameter: ParameterSlot,
    first_node: NodeSlot,
    range: Range,
    nodes: &mut BTreeSet<NodeSlot>,
    rows: &mut BTreeSet<ParameterRow>,
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
    for index in range.indices() {
        let identity = u16::try_from(index).map_err(|_| invalid())?;
        let row = plan
            .parameter_row_for_identity(parameter, identity)
            .ok_or_else(invalid)?;
        let actual = plan
            .parameter_targets()
            .get(row.index())
            .ok_or_else(invalid)?;
        if actual.control != first.control || actual.instances != first.instances {
            return Err(invalid());
        }
        nodes.insert(actual.node);
        rows.insert(row);
    }
    Ok(())
}
