//! Conservative, off-thread matching for stopped-session non-voice DSP migration.
//! This is not voice/identity migration or a mixed-reset policy.
mod hot;
#[cfg(test)]
mod tests;

use super::PreparedRenderer;
use crate::{
    node::kernels::{NodeState, PreparedNode, history_frames},
    plan::{CompiledPlan, NodeRole, NodeSlot, NodeStep, ParameterSlot, PlanId, PlanOp},
    quantities::{PreparedBytes, RecordCount},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GlobalMigrationError {
    #[error("global state migration requires unchanged device geometry")]
    Geometry,
    #[error("modulation, scripts, controllers and feedback need a separate migration law")]
    Routing,
    #[error("stateful global step {node:?} in plan {plan:?} has no eligible authored identity")]
    UnidentifiedState { plan: PlanId, node: NodeSlot },
    #[error("global migration metadata exceeds the prepared byte budget")]
    Bytes,
    #[error("global migration metadata allocation failed")]
    Allocation,
    #[error("migration belongs to different source or destination plans")]
    Origin,
    #[error("migration requires empty source carry and a fresh destination")]
    Boundary,
    #[error("global migration storage is inconsistent")]
    Storage,
}

#[derive(Debug, Clone, Copy)]
struct NodePair {
    old: NodeSlot,
    new: NodeSlot,
    /// An admitted storage extent, in interleaved samples.
    history_len: usize,
    inverse_index: usize,
}
#[derive(Debug, Clone, Copy)]
struct ParameterPair {
    old: ParameterSlot,
    new: ParameterSlot,
}
#[derive(Debug)]
pub(crate) struct PreparedGlobalMigration {
    old: PlanId,
    new: PlanId,
    nodes: Vec<NodePair>,
    parameters: Vec<ParameterPair>,
    /// Inverse node mapping, sorted by destination; proves non-aliasing before a move.
    destinations: Vec<(NodeSlot, NodeSlot)>,
    bytes: PreparedBytes,
    records: RecordCount,
}

fn step(plan: &CompiledPlan, node: NodeSlot) -> Option<&NodeStep> {
    plan.ops().iter().find_map(|op| match op {
        PlanOp::Node(step) if step.node() == node => Some(step),
        _ => None,
    })
}

// Positive list: newly added kernels need an explicit migration decision. In particular,
// equal plan-relative script/sample slot indices never establish resource equivalence.
// Send IDs are positional in current application lowering, so neither send kind
// carries state or parameter overrides across a plan replacement.
const fn eligible(prepared: &PreparedNode) -> bool {
    matches!(
        prepared,
        PreparedNode::Silence
            | PreparedNode::Constant { .. }
            | PreparedNode::Impulse { .. }
            | PreparedNode::Sine { .. }
            | PreparedNode::Saw { .. }
            | PreparedNode::Gain { .. }
            | PreparedNode::Envelope { .. }
            | PreparedNode::VelocityScaler { .. }
            | PreparedNode::Channel { .. }
            | PreparedNode::Mix
            | PreparedNode::Balance { .. }
            | PreparedNode::Trim { .. }
            | PreparedNode::SoftClip
            | PreparedNode::HardClamp
            | PreparedNode::Distortion { .. }
            | PreparedNode::Delay { .. }
            | PreparedNode::Compressor { .. }
            | PreparedNode::Latency { .. }
            | PreparedNode::Filter { .. }
            | PreparedNode::Amplifier
            | PreparedNode::Copy
            | PreparedNode::Lfo { .. }
            | PreparedNode::VoiceAmplifier { .. }
            | PreparedNode::VoiceOutput { .. }
    )
}

// Retiring a compiled occurrence must not orphan its held gate or inherited
// pitch/velocity state. Those destinations stay fresh even outside Voice scope.
fn note_driven(plan: &CompiledPlan, node: NodeSlot) -> bool {
    plan.note_targets().iter().any(|target| target.node == node)
        || plan
            .note_magnitudes()
            .iter()
            .any(|target| target.node == node)
}

fn check_plan(plan: &CompiledPlan) -> Result<(), GlobalMigrationError> {
    for op in plan.ops() {
        match op {
            PlanOp::Modulate(_) | PlanOp::FeedbackWrite { .. } => {
                return Err(GlobalMigrationError::Routing);
            }
            PlanOp::Node(step) => {
                let prepared = plan
                    .prepared_nodes()
                    .get(step.prepared().index())
                    .ok_or(GlobalMigrationError::Storage)?;
                if matches!(
                    prepared,
                    PreparedNode::Script { .. }
                        | PreparedNode::Controller { .. }
                        | PreparedNode::NoteSource
                ) {
                    return Err(GlobalMigrationError::Routing);
                }
                if step.role() == NodeRole::Global
                    && !plan
                        .non_voice_state_nodes()
                        .iter()
                        .any(|(_, _, node)| *node == step.node())
                    // Global conversions cannot receive Local/SharedSum fade or
                    // reset controls. Their Sum representation stays inert and has
                    // no history; the group-first checks are additional exclusions.
                    && !(matches!(prepared, PreparedNode::Copy)
                        && !plan.sum_groups().contains(&step.node())
                        && !plan.instance_groups().contains(&step.node()))
                    && (NodeState::initial(prepared) != NodeState::Stateless
                        || history_frames(prepared) != 0)
                {
                    return Err(GlobalMigrationError::UnidentifiedState {
                        plan: plan.id(),
                        node: step.node(),
                    });
                }
            }
            PlanOp::Output { .. } => {}
        }
    }
    Ok(())
}

impl PreparedGlobalMigration {
    pub(crate) fn prepare(
        old: &CompiledPlan,
        new: &CompiledPlan,
        ceiling: PreparedBytes,
    ) -> Result<Self, GlobalMigrationError> {
        if old.sample_rate() != new.sample_rate()
            || old.channel_layout() != new.channel_layout()
            || old.maximum_block_size() != new.maximum_block_size()
        {
            return Err(GlobalMigrationError::Geometry);
        }
        check_plan(old)?;
        check_plan(new)?;
        let max_nodes = old
            .non_voice_state_nodes()
            .len()
            .min(new.non_voice_state_nodes().len());
        let max_parameters = old.parameter_addresses().len();
        let required = max_nodes
            .checked_mul(size_of::<NodePair>() + size_of::<(NodeSlot, NodeSlot)>())
            .and_then(|n| {
                max_parameters
                    .checked_mul(size_of::<ParameterPair>())
                    .and_then(|p| n.checked_add(p))
            })
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(GlobalMigrationError::Bytes)?;
        if required > ceiling.get() {
            return Err(GlobalMigrationError::Bytes);
        }
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(max_nodes)
            .map_err(|_| GlobalMigrationError::Allocation)?;
        let mut destinations = Vec::new();
        destinations
            .try_reserve_exact(max_nodes)
            .map_err(|_| GlobalMigrationError::Allocation)?;
        let mut parameters = Vec::new();
        parameters
            .try_reserve_exact(max_parameters)
            .map_err(|_| GlobalMigrationError::Allocation)?;
        let retained = nodes
            .capacity()
            .checked_mul(size_of::<NodePair>())
            .and_then(|n| {
                destinations
                    .capacity()
                    .checked_mul(size_of::<(NodeSlot, NodeSlot)>())
                    .and_then(|d| n.checked_add(d))
            })
            .and_then(|n| {
                parameters
                    .capacity()
                    .checked_mul(size_of::<ParameterPair>())
                    .and_then(|p| n.checked_add(p))
            })
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(GlobalMigrationError::Bytes)?;
        if retained > ceiling.get() {
            return Err(GlobalMigrationError::Bytes);
        }
        for (id, old_scope, old_node) in old.non_voice_state_nodes() {
            let Some((_, new_scope, new_node)) = new
                .non_voice_state_nodes()
                .iter()
                .find(|(candidate, _, _)| candidate == id)
            else {
                continue;
            };
            let left = step(old, *old_node).ok_or(GlobalMigrationError::Storage)?;
            let right = step(new, *new_node).ok_or(GlobalMigrationError::Storage)?;
            let prepared = old
                .prepared_for_node(*old_node)
                .ok_or(GlobalMigrationError::Storage)?;
            if old_scope != new_scope
                || left.role() != right.role()
                || !eligible(prepared)
                || note_driven(old, *old_node)
                || note_driven(new, *new_node)
                || Some(prepared) != new.prepared_for_node(*new_node)
                || left.out_layout() != right.out_layout()
            {
                continue;
            }
            let start = parameters.len();
            let mut compatible = true;
            for address in old
                .parameter_addresses()
                .iter()
                .filter(|address| address.node == *id)
            {
                let Some(target_slot) = new.resolve_parameter(*id, address.parameter) else {
                    compatible = false;
                    break;
                };
                let old_target = old
                    .parameter_targets()
                    .get(address.slot.index())
                    .ok_or(GlobalMigrationError::Storage)?;
                let mut new_target = *new
                    .parameter_targets()
                    .get(target_slot.index())
                    .ok_or(GlobalMigrationError::Storage)?;
                if old_target.node != *old_node
                    || new_target.node != *new_node
                    || old_target.instances.get() != 1
                {
                    compatible = false;
                    break;
                }
                new_target.node = old_target.node;
                if *old_target != new_target {
                    compatible = false;
                    break;
                }
                parameters.push(ParameterPair {
                    old: address.slot,
                    new: target_slot,
                });
            }
            let new_count = new
                .parameter_addresses()
                .iter()
                .filter(|address| address.node == *id)
                .count();
            if !compatible || new_count != parameters.len() - start {
                parameters.truncate(start);
                continue;
            }
            nodes.push(NodePair {
                old: *old_node,
                new: *new_node,
                inverse_index: 0,
                history_len: history_frames(prepared)
                    .checked_mul(left.out_layout().channels())
                    .ok_or(GlobalMigrationError::Storage)?,
            });
            destinations.push((*new_node, *old_node));
        }
        nodes.sort_unstable_by_key(|pair| pair.old);
        destinations.sort_unstable_by_key(|(new, _)| *new);
        if nodes.windows(2).any(|pair| pair[0].old >= pair[1].old)
            || destinations.windows(2).any(|pair| pair[0].0 >= pair[1].0)
        {
            return Err(GlobalMigrationError::Storage);
        }
        for pair in &mut nodes {
            pair.inverse_index = destinations
                .binary_search_by_key(&pair.new, |(new, _)| *new)
                .map_err(|_| GlobalMigrationError::Storage)?;
        }
        // Node records are compiler-bounded, but do not silently saturate a diagnostic.
        let records = RecordCount::measured(
            u32::try_from(nodes.len()).map_err(|_| GlobalMigrationError::Bytes)?,
        );
        Ok(Self {
            old: old.id(),
            new: new.id(),
            nodes,
            parameters,
            destinations,
            bytes: PreparedBytes::measured(retained),
            records,
        })
    }
    pub(crate) const fn records(&self) -> RecordCount {
        self.records
    }
    pub(crate) const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
}
