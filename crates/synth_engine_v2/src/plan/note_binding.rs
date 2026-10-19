//! Stable authored note bindings for preflighted native continuation.
use super::{CompiledPlan, ContinuationError, NodeRole, NoteSlot, PlanId, PlanOp};
use crate::quantities::{PreparedBytes, SlotCount};
mod hot;

#[derive(Clone, Copy)]
pub(crate) enum NoteBindingScope {
    Exact,
    StructuralVoices,
}

#[derive(Debug)]
#[must_use]
pub(crate) struct NoteBindings {
    old: PlanId,
    new: PlanId,
    destination_count: SlotCount,
    destinations: Vec<Option<NoteSlot>>,
    sources: Vec<Option<NoteSlot>>,
}

fn local_target(plan: &CompiledPlan, slot: NoteSlot) -> bool {
    let Some(target) = plan.note_targets.get(slot.index()) else {
        return false;
    };
    plan.ops.iter().any(|op| matches!(op, PlanOp::Node(step) if step.node == target.node && matches!(step.role, NodeRole::Local(_))))
}

fn resolve_binding(
    old: &CompiledPlan,
    new: &CompiledPlan,
    scope: NoteBindingScope,
    address: &super::NoteAddress,
) -> Result<Option<NoteSlot>, ContinuationError> {
    if address.slot.plan() != old.id || address.slot.index() >= old.note_targets.len() {
        return Err(ContinuationError::Binding);
    }
    if matches!(scope, NoteBindingScope::StructuralVoices) && !local_target(old, address.slot) {
        return Ok(None);
    }
    let destination = new
        .resolve_note(address.node)
        .ok_or(ContinuationError::Plan)?;
    if destination.plan() != new.id
        || destination.index() >= new.note_targets.len()
        || (matches!(scope, NoteBindingScope::StructuralVoices) && !local_target(new, destination))
    {
        return Err(ContinuationError::Binding);
    }
    Ok(Some(destination))
}

impl NoteBindings {
    pub(crate) fn required_heap_bytes(
        old: &CompiledPlan,
        new: &CompiledPlan,
    ) -> Result<PreparedBytes, ContinuationError> {
        old.note_targets
            .len()
            .checked_add(new.note_targets.len())
            .and_then(|count| count.checked_mul(size_of::<Option<NoteSlot>>()))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .map(PreparedBytes::measured)
            .ok_or(ContinuationError::Bytes)
    }

    pub(crate) fn prepare(
        old: &CompiledPlan,
        new: &CompiledPlan,
        scope: NoteBindingScope,
        allowance: PreparedBytes,
    ) -> Result<Self, ContinuationError> {
        // Unsupported shape precedes budget refusal without constructing storage.
        for address in &old.note_addresses {
            let _binding = resolve_binding(old, new, scope, address)?;
        }
        if Self::required_heap_bytes(old, new)? > allowance {
            return Err(ContinuationError::Bytes);
        }
        let count = SlotCount::measured(
            u32::try_from(new.note_targets.len()).map_err(|_| ContinuationError::Binding)?,
        );
        let mut destinations = Vec::new();
        destinations
            .try_reserve_exact(old.note_targets.len())
            .map_err(|_| ContinuationError::Allocation)?;
        destinations.resize(old.note_targets.len(), None);
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(new.note_targets.len())
            .map_err(|_| ContinuationError::Allocation)?;
        sources.resize(new.note_targets.len(), None);
        for address in &old.note_addresses {
            let Some(destination) = resolve_binding(old, new, scope, address)? else {
                continue;
            };
            let source = address.slot.index();
            if sources.get(destination.index()).is_none_or(Option::is_some)
                || destinations[source].is_some()
            {
                return Err(ContinuationError::Binding);
            }
            destinations[source] = Some(destination);
            sources[destination.index()] = Some(address.slot);
        }
        let bindings = Self {
            old: old.id,
            new: new.id,
            destination_count: count,
            destinations,
            sources,
        };
        if bindings.heap_bytes()? > allowance {
            return Err(ContinuationError::Bytes);
        }
        Ok(bindings)
    }

    pub(super) fn heap_bytes(&self) -> Result<PreparedBytes, ContinuationError> {
        self.destinations
            .capacity()
            .checked_add(self.sources.capacity())
            .and_then(|count| count.checked_mul(size_of::<Option<NoteSlot>>()))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .map(PreparedBytes::measured)
            .ok_or(ContinuationError::Bytes)
    }
}

#[cfg(test)]
impl NoteBindings {
    pub(crate) fn replace_destination_for_test(
        &mut self,
        source: NoteSlot,
        destination: Option<NoteSlot>,
    ) -> bool {
        if source.plan() != self.old {
            return false;
        }
        let Some(slot) = self.destinations.get_mut(source.index()) else {
            return false;
        };
        *slot = destination;
        true
    }
}
