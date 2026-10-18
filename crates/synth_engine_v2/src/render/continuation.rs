//! Bounded preflight and owner exchange for exact-layout native continuation.
use super::{PreparedRenderer, slot::SlotState};
use crate::plan::{ContinuationError, continuation::PlanContinuation};

impl PreparedRenderer {
    pub(crate) fn continuation_preflight(
        &self,
        prepared: &Self,
        proof: &PlanContinuation,
    ) -> Result<(), ContinuationError> {
        if self.plan.id() != proof.old
            || prepared.plan.id() != proof.new
            || self.carry_frames != 0
            || self.mixed_partition.is_some()
            || self.adoption_gate_len != 0
            || self.node_states.len() != prepared.node_states.len()
            || self.parameter_slots.len() != prepared.parameter_slots.len()
            || self.history.len() != prepared.history.len()
            || self.buffers.len() != prepared.buffers.len()
            || !self
                .live_notes
                .can_continue_plan(proof.old, self.plan.note_targets().len())
        {
            return Err(ContinuationError::Storage);
        }
        let mut previous = None;
        for change in &proof.trims {
            if change.old.plan() != proof.old
                || change.new.plan() != proof.new
                || change.old.index() != change.new.index()
                || previous.is_some_and(|index| index >= change.old.index())
            {
                return Err(ContinuationError::Storage);
            }
            previous = Some(change.old.index());
            let target = self
                .plan
                .parameter_targets()
                .get(change.old.index())
                .ok_or(ContinuationError::Storage)?;
            let mut expected =
                SlotState::prepared(target.law, target.unit, target.smoothing, change.old_base);
            let resting = expected;
            expected.seed();
            let _restored = expected.write_override(change.old_base);
            let current = self.parameter_slots.get(change.old.index());
            if current != Some(&expected) && current != Some(&resting) {
                return Err(ContinuationError::Storage);
            }
            let new_target = prepared
                .plan
                .parameter_targets()
                .get(change.new.index())
                .ok_or(ContinuationError::Storage)?;
            let fresh = SlotState::prepared(
                new_target.law,
                new_target.unit,
                new_target.smoothing,
                change.new_base,
            );
            if prepared.parameter_slots.get(change.new.index()) != Some(&fresh) {
                return Err(ContinuationError::Storage);
            }
        }
        Ok(())
    }

    /// The caller has preflighted the immutable proof and both fixed extents. No
    /// failure/allocation/destruction follows: only copied rows, slot retags and Arc swaps.
    pub(crate) fn install_continuation(&mut self, prepared: &mut Self, proof: &PlanContinuation) {
        // Every index was checked against these unchanged vectors in the preceding pass.
        for change in &proof.trims {
            core::mem::swap(
                &mut self.parameter_slots[change.old.index()],
                &mut prepared.parameter_slots[change.new.index()],
            );
        }
        self.live_notes.continue_plan(proof.new);
        core::mem::swap(&mut self.plan, &mut prepared.plan);
    }
}
