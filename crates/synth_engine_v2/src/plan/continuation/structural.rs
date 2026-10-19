//! Structural continuation with an unchanged logical native voice program.
use super::voice::{VoiceNodePair, VoiceParameterPair, VoiceProgram};
use super::*;
use crate::render::migration::{GlobalMigrationError, PreparedGlobalMigration};

#[derive(Debug)]
pub(crate) struct StructuralContinuation {
    pub(crate) global: PreparedGlobalMigration,
    /// Monotonic source/destination pairs, including compiler-inserted fade/sum state.
    pub(crate) voice_nodes: Vec<VoiceNodePair>,
    pub(crate) voice_parameters: Vec<VoiceParameterPair>,
}

fn migration_error(error: GlobalMigrationError) -> ContinuationError {
    match error {
        GlobalMigrationError::Bytes => ContinuationError::Bytes,
        GlobalMigrationError::Allocation => ContinuationError::Allocation,
        GlobalMigrationError::Origin
        | GlobalMigrationError::Boundary
        | GlobalMigrationError::Storage => ContinuationError::Binding,
        GlobalMigrationError::Geometry
        | GlobalMigrationError::Routing
        | GlobalMigrationError::UnidentifiedState { .. } => ContinuationError::Plan,
    }
}

impl CompiledPlan {
    pub(super) fn prepare_structural_continuation(
        &self,
        other: &Self,
        allowance: PreparedBytes,
        records: crate::quantities::RecordCount,
    ) -> Result<PlanContinuation, ContinuationError> {
        if self.channel_layout != other.channel_layout
            || self.sample_rate != other.sample_rate
            || self.maximum_block_size != other.maximum_block_size
            || self.max_events_per_quantum != other.max_events_per_quantum
            || self.compiled_event_share != other.compiled_event_share
            || self.note_producer_ranges != other.note_producer_ranges
            || self.note_producer_holds != other.note_producer_holds
            || self.compiled_note_producer != other.compiled_note_producer
            || self.forward_event_horizon != other.forward_event_horizon
            || self.prepared_tunings != other.prepared_tunings
            || self.stealing != other.stealing
            || self.prepass != other.prepass
            || self.path_latencies.output() != other.path_latencies.output()
            || self.path_latencies.compensation_records().get() != 0
            || other.path_latencies.compensation_records().get() != 0
            || !self.prepared_scripts.is_empty()
            || !other.prepared_scripts.is_empty()
            || !self.prepared_samples.is_empty()
            || !other.prepared_samples.is_empty()
            || self.producer_programs.present()
            || other.producer_programs.present()
            || !self.authored_sources.is_empty()
            || !other.authored_sources.is_empty()
            || !self.sends.is_empty()
            || !other.sends.is_empty()
        {
            return Err(ContinuationError::Plan);
        }
        // Match actual voice targets by stable authored identity. Global note-driven
        // state stays fresh and cannot carry a surviving identity or timeline event.
        let voice = VoiceProgram::check(self, other)?;
        for plan in [self, other] {
            for address in &plan.note_addresses {
                let target = plan
                    .note_targets
                    .get(address.slot.index())
                    .ok_or(ContinuationError::Binding)?;
                let local = plan.ops.iter().any(|op| matches!(op, PlanOp::Node(step) if step.node == target.node && matches!(step.role, NodeRole::Local(_))));
                if !local && !dormant_global_gate(plan, address.node, address.slot) {
                    return Err(ContinuationError::Plan);
                }
            }
        }
        self.continuation_mix(other)?;
        if self.taps.iter().zip(&other.taps).any(|(old, new)| {
            old.data != new.data || old.bytes_per_quantum != new.bytes_per_quantum
        }) {
            return Err(ContinuationError::Plan);
        }
        // Matched authored kernels cannot silently acquire fresh cached note state.
        for (id, scope, slot) in &self.authored_state_nodes {
            if let Some((_, new_scope, new_slot)) = other
                .authored_state_nodes
                .iter()
                .find(|(node, _, _)| node == id)
                && (scope != new_scope
                    || self.prepared_for_node(*slot) != other.prepared_for_node(*new_slot))
            {
                return Err(ContinuationError::Plan);
            }
        }
        for address in &self.parameter_addresses {
            if let Some(slot) = other.resolve_parameter(address.node, address.parameter) {
                let old = self
                    .parameter_targets
                    .get(address.slot.index())
                    .ok_or(ContinuationError::Binding)?;
                let new = other
                    .parameter_targets
                    .get(slot.index())
                    .ok_or(ContinuationError::Binding)?;
                if old.base.as_f32().to_bits() != new.base.as_f32().to_bits() {
                    return Err(ContinuationError::Plan);
                }
            }
        }
        for plan in [self, other] {
            crate::render::migration::check_plan(plan).map_err(migration_error)?;
            if plan.prepared_nodes.iter().any(|node| !eligible(node)) {
                return Err(ContinuationError::Plan);
            }
        }
        for (id, scope, old_slot) in &self.authored_state_nodes {
            let Some((_, new_scope, new_slot)) = other
                .authored_state_nodes
                .iter()
                .find(|(node, _, _)| node == id)
            else {
                continue;
            };
            if *scope == crate::ir::ExecutionScope::Voice {
                continue;
            }
            let old_step = self
                .ops
                .iter()
                .find_map(|op| match op {
                    PlanOp::Node(step) if step.node == *old_slot => Some(step),
                    _ => None,
                })
                .ok_or(ContinuationError::Binding)?;
            let new_step = other
                .ops
                .iter()
                .find_map(|op| match op {
                    PlanOp::Node(step) if step.node == *new_slot => Some(step),
                    _ => None,
                })
                .ok_or(ContinuationError::Binding)?;
            if scope != new_scope
                || old_step.role != new_step.role
                || old_step.out_layout != new_step.out_layout
            {
                return Err(ContinuationError::Plan);
            }
            let mut old_count = 0;
            for address in self
                .parameter_addresses
                .iter()
                .filter(|address| address.node == *id)
            {
                let destination = other
                    .resolve_parameter(*id, address.parameter)
                    .ok_or(ContinuationError::Plan)?;
                let old = self
                    .parameter_targets
                    .get(address.slot.index())
                    .ok_or(ContinuationError::Binding)?;
                let mut new = *other
                    .parameter_targets
                    .get(destination.index())
                    .ok_or(ContinuationError::Binding)?;
                new.node = old.node;
                if *old != new {
                    return Err(ContinuationError::Plan);
                }
                old_count += 1;
            }
            if other
                .parameter_addresses
                .iter()
                .filter(|address| address.node == *id)
                .count()
                != old_count
            {
                return Err(ContinuationError::Plan);
            }
        }
        let work = self
            .ops
            .len()
            .checked_add(other.ops.len())
            .and_then(|n| n.checked_add(self.parameter_targets.len()))
            .and_then(|n| n.checked_add(other.parameter_targets.len()))
            .and_then(|n| n.checked_add(self.note_targets.len()))
            .and_then(|n| n.checked_add(other.note_targets.len()))
            .and_then(|n| {
                self.note_producer_ranges
                    .iter()
                    .try_fold(n, |total, range| {
                        total.checked_add(usize::try_from(range.get()).ok()?)
                    })
            })
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(ContinuationError::RecordLimit)?;
        if work > records.get() {
            return Err(ContinuationError::RecordLimit);
        }
        let note_bytes =
            super::super::note_binding::NoteBindings::required_heap_bytes(self, other)?.get();
        let global =
            PreparedGlobalMigration::prepare(self, other, allowance).map_err(migration_error)?;
        let count = voice.node_count();
        let parameter_count = voice.parameter_count();
        let requested = count
            .checked_mul(size_of::<VoiceNodePair>())
            .and_then(|n| {
                parameter_count
                    .checked_mul(size_of::<VoiceParameterPair>())
                    .and_then(|p| n.checked_add(p))
            })
            .and_then(|n| n.checked_add(size_of::<PlanContinuation>()))
            .and_then(|n| u64::try_from(n).ok())
            .and_then(|n| n.checked_add(global.bytes().get()))
            .and_then(|n| n.checked_add(note_bytes))
            .ok_or(ContinuationError::Bytes)?;
        if requested > allowance.get() {
            return Err(ContinuationError::Bytes);
        }
        let notes = super::super::note_binding::NoteBindings::prepare(
            self,
            other,
            super::super::note_binding::NoteBindingScope::StructuralVoices,
            allowance,
        )?;
        let voice_nodes = voice.nodes()?;
        let voice_parameters = voice.parameters()?;
        let bytes = voice_nodes
            .capacity()
            .checked_mul(size_of::<VoiceNodePair>())
            .and_then(|n| {
                voice_parameters
                    .capacity()
                    .checked_mul(size_of::<VoiceParameterPair>())
                    .and_then(|p| n.checked_add(p))
            })
            .and_then(|n| n.checked_add(size_of::<PlanContinuation>()))
            .and_then(|n| u64::try_from(n).ok())
            .and_then(|n| n.checked_add(global.bytes().get()))
            .and_then(|n| n.checked_add(notes.heap_bytes().ok()?.get()))
            .ok_or(ContinuationError::Bytes)?;
        if bytes > allowance.get() {
            return Err(ContinuationError::Bytes);
        }
        Ok(PlanContinuation {
            old: self.id,
            new: other.id,
            trims: Vec::new(),
            notes,
            bytes: PreparedBytes::measured(bytes),
            structural: Some(StructuralContinuation {
                global,
                voice_nodes,
                voice_parameters,
            }),
        })
    }
}

/// A fresh global gate must have no consumer and no magnitude state on another
/// node. Mere absence of note events cannot prove an audible global oscillator
/// or filter safe to reset. Modulation/feedback/controllers remain refused above.
fn dormant_global_gate(plan: &CompiledPlan, authored: crate::ir::NodeId, slot: NoteSlot) -> bool {
    let Some(target) = plan.note_targets.get(slot.index()) else {
        return false;
    };
    if !plan.authored_state_nodes.contains(&(
        authored,
        crate::ir::ExecutionScope::Global,
        target.node,
    )) || !matches!(
        plan.prepared_for_node(target.node),
        Some(PreparedNode::Envelope { .. })
    ) || plan
        .note_magnitudes_of(slot)
        .iter()
        .any(|magnitude| magnitude.node != target.node)
    {
        return false;
    }
    for op in &plan.ops {
        if let PlanOp::Node(step) = op {
            for input in step.inputs().iter().flatten() {
                if super::voice::preceding_writer(plan, step.node, *input) == Some(target.node) {
                    return false;
                }
            }
        }
    }
    true
}
