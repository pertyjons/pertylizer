//! Cold proof of an unchanged native voice program under physical relocation.
use super::*;
use crate::node::kernels::history_frames;

#[derive(Debug)]
pub(crate) struct VoiceNodePair {
    pub(crate) old: NodeSlot,
    pub(crate) new: NodeSlot,
    pub(crate) history: usize,
}

#[derive(Debug)]
pub(crate) struct VoiceParameterPair {
    pub(crate) old: ParameterRow,
    pub(crate) new: ParameterRow,
}

/// Borrowed matching work is cold; only the admitted flat maps reach the callback.
pub(super) struct VoiceProgram<'a> {
    old: &'a CompiledPlan,
    new: &'a CompiledPlan,
}

fn steps(plan: &CompiledPlan) -> impl Iterator<Item = &NodeStep> + Clone {
    plan.ops.iter().filter_map(|op| match op {
        PlanOp::Node(step) if step.role != NodeRole::Global => Some(step),
        _ => None,
    })
}

impl<'a> VoiceProgram<'a> {
    pub(super) fn check(
        old: &'a CompiledPlan,
        new: &'a CompiledPlan,
    ) -> Result<Self, ContinuationError> {
        let program = Self { old, new };
        if steps(old).count() != steps(new).count() {
            return Err(ContinuationError::Plan);
        }
        program.check_steps()?;
        program.check_anchors()?;
        program.check_groups()?;
        program.check_parameters()?;
        program.check_notes()?;
        Ok(program)
    }

    fn check_steps(&self) -> Result<(), ContinuationError> {
        let fixed = steps(self.old).zip(steps(self.new)).all(|(a, b)| a == b);
        let mut previous = None;
        for (a, b) in steps(self.old).zip(steps(self.new)) {
            if previous.is_some_and(|(old, new)| old >= a.node || new >= b.node)
                || matches!(a.role, NodeRole::Unclassified)
                || matches!(b.role, NodeRole::Unclassified)
                || !a.same_kernel(b)
                || a.role != b.role
                || a.out_layout != b.out_layout
                || a.in_place_safe != b.in_place_safe
                || self.old.prepared_for_node(a.node) != self.new.prepared_for_node(b.node)
            {
                return Err(ContinuationError::Plan);
            }
            previous = Some((a.node, b.node));
            for (left, right) in a.inputs().iter().zip(b.inputs()) {
                match (left, right) {
                    (None, None) => {}
                    (Some(left), Some(right)) => {
                        let source = preceding_writer(self.old, a.node, *left)
                            .ok_or(ContinuationError::Plan)?;
                        let destination = preceding_writer(self.new, b.node, *right)
                            .ok_or(ContinuationError::Plan)?;
                        if self.node(source) != Some(destination) {
                            // Preserve fixed-geometry global inputs only when the
                            // actual authored source still agrees. Relocated global
                            // dependencies need a separate boundary law.
                            let same_global = fixed
                                && self.old.non_voice_state_nodes.iter().any(
                                    |(id, scope, node)| {
                                        *node == source
                                            && self.new.non_voice_state_nodes.contains(&(
                                                *id,
                                                *scope,
                                                destination,
                                            ))
                                    },
                                );
                            if !same_global {
                                return Err(ContinuationError::Plan);
                            }
                        }
                    }
                    _ => return Err(ContinuationError::Plan),
                }
            }
        }
        Ok(())
    }

    fn check_anchors(&self) -> Result<(), ContinuationError> {
        // Authored identity anchors the schedule bijection. Ordinals alone never
        // authorize copying state between different authored nodes or instruments.
        for (from, to, forward) in [(self.old, self.new, true), (self.new, self.old, false)] {
            for (id, scope, slot) in &from.authored_state_nodes {
                if *scope != crate::ir::ExecutionScope::Voice {
                    continue;
                }
                let (_, other_scope, other_slot) = to
                    .authored_state_nodes
                    .iter()
                    .find(|(other, _, _)| other == id)
                    .ok_or(ContinuationError::Plan)?;
                let mapped = if forward {
                    self.node(*slot)
                } else {
                    self.reverse_node(*slot)
                };
                if scope != other_scope || mapped != Some(*other_slot) {
                    return Err(ContinuationError::Plan);
                }
            }
        }
        Ok(())
    }

    fn check_groups(&self) -> Result<(), ContinuationError> {
        let count = usize::try_from(self.old.voice_instances().get())
            .map_err(|_| ContinuationError::Binding)?;
        for (a, b) in [
            (&self.old.instance_groups, &self.new.instance_groups),
            (&self.old.sum_groups, &self.new.sum_groups),
        ] {
            if a.len() != b.len() {
                return Err(ContinuationError::Plan);
            }
            for (a, b) in a.iter().zip(b) {
                for offset in 0..count {
                    let source = a
                        .index()
                        .checked_add(offset)
                        .ok_or(ContinuationError::Binding)?;
                    let destination = b
                        .index()
                        .checked_add(offset)
                        .ok_or(ContinuationError::Binding)?;
                    if self.node(NodeSlot::new(source)) != Some(NodeSlot::new(destination)) {
                        return Err(ContinuationError::Plan);
                    }
                }
            }
        }
        Ok(())
    }

    fn check_parameters(&self) -> Result<(), ContinuationError> {
        // Every per-instance row must match its authored control group, not just
        // the numerical position it happens to occupy in either plan.
        for (from, to, forward) in [(self.old, self.new, true), (self.new, self.old, false)] {
            let mut previous = None;
            for address in &from.parameter_addresses {
                let target = from
                    .parameter_targets
                    .get(address.slot.index())
                    .ok_or(ContinuationError::Binding)?;
                if !steps(from).any(|step| step.node == target.node) {
                    continue;
                }
                let destination = to
                    .resolve_parameter(address.node, address.parameter)
                    .ok_or(ContinuationError::Plan)?;
                let other = to
                    .parameter_targets
                    .get(destination.index())
                    .ok_or(ContinuationError::Binding)?;
                if target.instances != other.instances {
                    return Err(ContinuationError::Plan);
                }
                for instance in 0..target.instances.get() {
                    let offset =
                        usize::try_from(instance).map_err(|_| ContinuationError::Binding)?;
                    let source = address
                        .slot
                        .index()
                        .checked_add(offset)
                        .ok_or(ContinuationError::Binding)?;
                    let destination_row = destination
                        .index()
                        .checked_add(offset)
                        .ok_or(ContinuationError::Binding)?;
                    if previous.is_some_and(|(old, new)| old >= source || new >= destination_row) {
                        return Err(ContinuationError::Plan);
                    }
                    previous = Some((source, destination_row));
                    let a = from
                        .parameter_targets
                        .get(source)
                        .ok_or(ContinuationError::Binding)?;
                    let mut b = *to
                        .parameter_targets
                        .get(destination_row)
                        .ok_or(ContinuationError::Binding)?;
                    let mapped = if forward {
                        self.node(a.node)
                    } else {
                        self.reverse_node(a.node)
                    };
                    if mapped != Some(b.node) {
                        return Err(ContinuationError::Plan);
                    }
                    b.node = a.node;
                    if *a != b || a.base.as_f32().to_bits() != b.base.as_f32().to_bits() {
                        return Err(ContinuationError::Plan);
                    }
                }
            }
        }
        Ok(())
    }

    fn node(&self, node: NodeSlot) -> Option<NodeSlot> {
        steps(self.old)
            .zip(steps(self.new))
            .find_map(|(a, b)| (a.node == node).then_some(b.node))
    }
    fn reverse_node(&self, node: NodeSlot) -> Option<NodeSlot> {
        steps(self.old)
            .zip(steps(self.new))
            .find_map(|(a, b)| (b.node == node).then_some(a.node))
    }
    fn parameter(&self, slot: ParameterSlot) -> Result<ParameterSlot, ContinuationError> {
        if slot.plan() != self.old.id {
            return Err(ContinuationError::Binding);
        }
        let address = self
            .old
            .parameter_addresses
            .iter()
            .find(|address| address.slot == slot)
            .ok_or(ContinuationError::Binding)?;
        self.new
            .resolve_parameter(address.node, address.parameter)
            .ok_or(ContinuationError::Plan)
    }
    fn check_notes(&self) -> Result<(), ContinuationError> {
        for address in &self.old.note_addresses {
            let a = self
                .old
                .note_targets
                .get(address.slot.index())
                .ok_or(ContinuationError::Binding)?;
            let Some(mapped_node) = self.node(a.node) else {
                continue;
            };
            let slot = self
                .new
                .resolve_note(address.node)
                .ok_or(ContinuationError::Plan)?;
            let mut b = *self
                .new
                .note_targets
                .get(slot.index())
                .ok_or(ContinuationError::Binding)?;
            if mapped_node != b.node || self.parameter(a.parameter)? != b.parameter {
                return Err(ContinuationError::Plan);
            }
            self.check_magnitudes(a.magnitudes, b.magnitudes)?;
            b.node = a.node;
            b.parameter = a.parameter;
            b.magnitudes = a.magnitudes;
            if *a != b {
                return Err(ContinuationError::Plan);
            }
        }
        Ok(())
    }

    fn check_magnitudes(
        &self,
        old: NoteMagnitudeRange,
        new: NoteMagnitudeRange,
    ) -> Result<(), ContinuationError> {
        let left = self
            .old
            .note_magnitudes
            .get(
                old.start
                    ..old
                        .start
                        .checked_add(old.length)
                        .ok_or(ContinuationError::Binding)?,
            )
            .ok_or(ContinuationError::Binding)?;
        let right = self
            .new
            .note_magnitudes
            .get(
                new.start
                    ..new
                        .start
                        .checked_add(new.length)
                        .ok_or(ContinuationError::Binding)?,
            )
            .ok_or(ContinuationError::Binding)?;
        if left.len() != right.len() {
            return Err(ContinuationError::Plan);
        }
        for (a, b) in left.iter().zip(right) {
            let mut b = *b;
            if self.node(a.node) != Some(b.node) || self.parameter(a.parameter)? != b.parameter {
                return Err(ContinuationError::Plan);
            }
            b.node = a.node;
            b.parameter = a.parameter;
            b.tuning = match (a.tuning, b.tuning) {
                (Some(a), Some(b))
                    if a.plan() == self.old.id
                        && b.plan() == self.new.id
                        && a.index() == b.index() =>
                {
                    Some(a)
                }
                (None, None) => None,
                _ => return Err(ContinuationError::Plan),
            };
            if *a != b {
                return Err(ContinuationError::Plan);
            }
        }
        Ok(())
    }

    pub(super) fn node_count(&self) -> usize {
        steps(self.old).count()
    }
    pub(super) fn parameter_count(&self) -> usize {
        self.old
            .parameter_targets
            .iter()
            .filter(|target| self.node(target.node).is_some())
            .count()
    }
    pub(super) fn nodes(&self) -> Result<Vec<VoiceNodePair>, ContinuationError> {
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(self.node_count())
            .map_err(|_| ContinuationError::Allocation)?;
        for (a, b) in steps(self.old).zip(steps(self.new)) {
            let prepared = self
                .old
                .prepared_for_node(a.node)
                .ok_or(ContinuationError::Binding)?;
            let history = history_frames(prepared)
                .checked_mul(a.out_layout.channels())
                .ok_or(ContinuationError::Bytes)?;
            nodes.push(VoiceNodePair {
                old: a.node,
                new: b.node,
                history,
            });
        }
        Ok(nodes)
    }
    pub(super) fn parameters(&self) -> Result<Vec<VoiceParameterPair>, ContinuationError> {
        let mut pairs = Vec::new();
        pairs
            .try_reserve_exact(self.parameter_count())
            .map_err(|_| ContinuationError::Allocation)?;
        for (index, target) in self.old.parameter_targets.iter().enumerate() {
            if self.node(target.node).is_none() {
                continue;
            }
            // Contiguous group membership is established by the compiler; locate
            // its authored address and preserve the instance offset within it.
            let address = self
                .old
                .parameter_addresses
                .iter()
                .find(|address| {
                    let start = address.slot.index();
                    self.old.parameter_targets.get(start).is_some_and(|base| {
                        index >= start
                            && index - start < usize::try_from(base.instances.get()).unwrap_or(0)
                    })
                })
                .ok_or(ContinuationError::Binding)?;
            let destination = self.parameter(address.slot)?;
            pairs.push(VoiceParameterPair {
                old: ParameterRow::new(self.old.id, index),
                new: ParameterRow::new(
                    self.new.id,
                    destination.index() + index - address.slot.index(),
                ),
            });
        }
        Ok(pairs)
    }
}

pub(super) fn preceding_writer(
    plan: &CompiledPlan,
    before: NodeSlot,
    buffer: BufferSlot,
) -> Option<NodeSlot> {
    let mut writer = None;
    for op in &plan.ops {
        if let PlanOp::Node(step) = op {
            if step.node == before {
                return writer;
            }
            if step.out == buffer {
                writer = Some(step.node);
            }
        }
    }
    None
}
