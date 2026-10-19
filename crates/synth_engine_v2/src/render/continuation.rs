//! Bounded preflight and owner exchange for exact-layout native continuation.
use super::{PreparedRenderer, slot::SlotState};
use crate::plan::{ContinuationError, continuation::PlanContinuation};

impl PreparedRenderer {
    pub(crate) fn continuation_preflight(
        &self,
        prepared: &Self,
        proof: &PlanContinuation,
    ) -> Result<(), ContinuationError> {
        if self.mixed_partition.is_some() || prepared.mixed_partition.is_some() {
            return Err(ContinuationError::Storage);
        }
        self.continuation_preflight_geometry(prepared, proof)
    }

    #[cfg(feature = "mixed-transfer-rehearsal")]
    pub(crate) fn mixed_continuation_preflight(
        &self,
        prepared: &Self,
        proof: &PlanContinuation,
    ) -> Result<(), ContinuationError> {
        let (Some(old), Some(new)) = (&self.mixed_partition, &prepared.mixed_partition) else {
            return Err(ContinuationError::Storage);
        };
        if proof.structural.is_some()
            || old.plan_id() != proof.old
            || new.plan_id() != proof.new
            || !old.same_fixed_geometry(new)
        {
            return Err(ContinuationError::Storage);
        }
        self.continuation_preflight_geometry(prepared, proof)
    }

    fn continuation_preflight_geometry(
        &self,
        prepared: &Self,
        proof: &PlanContinuation,
    ) -> Result<(), ContinuationError> {
        if self.plan.id() != proof.old
            || prepared.plan.id() != proof.new
            || proof.notes.old_plan() != proof.old
            || proof.notes.new_plan() != proof.new
            || self.carry_frames != 0
            || self.adoption_gate_len != 0
            || (proof.structural.is_none()
                && (self.node_states.len() != prepared.node_states.len()
                    || self.parameter_slots.len() != prepared.parameter_slots.len()
                    || self.history.len() != prepared.history.len()
                    || self.buffers.len() != prepared.buffers.len()))
            || !self.live_notes.can_continue_notes(&proof.notes)
        {
            return Err(ContinuationError::Storage);
        }
        if let Some(structural) = &proof.structural {
            structural
                .global
                .preflight_into(self, prepared)
                .map_err(|_| ContinuationError::Storage)?;
            let mut previous = None;
            for pair in &structural.voice_nodes {
                if previous.is_some_and(|(old, new)| old >= pair.old || new >= pair.new)
                    || self.node_states.get(pair.old.index()).is_none()
                    || prepared.node_states.get(pair.new.index()).is_none()
                    || self
                        .history
                        .get(pair.old.index())
                        .map(|buffer| buffer.len())
                        != Some(pair.history)
                    || prepared
                        .history
                        .get(pair.new.index())
                        .map(|buffer| buffer.len())
                        != Some(pair.history)
                {
                    return Err(ContinuationError::Storage);
                }
                previous = Some((pair.old, pair.new));
            }
            let mut previous = None;
            for pair in &structural.voice_parameters {
                if pair.old.plan() != proof.old
                    || pair.new.plan() != proof.new
                    || previous.is_some_and(|(old, new)| {
                        old >= pair.old.index() || new >= pair.new.index()
                    })
                    || self.parameter_slots.get(pair.old.index()).is_none()
                    || prepared.parameter_slots.get(pair.new.index()).is_none()
                {
                    return Err(ContinuationError::Storage);
                }
                previous = Some((pair.old.index(), pair.new.index()));
            }
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
        if let Some(structural) = &proof.structural {
            structural.global.install_into(self, prepared);
            for pair in &structural.voice_nodes {
                prepared.node_states[pair.new.index()] = self.node_states[pair.old.index()];
                core::mem::swap(
                    &mut self.history[pair.old.index()],
                    &mut prepared.history[pair.new.index()],
                );
            }
            for pair in &structural.voice_parameters {
                prepared.parameter_slots[pair.new.index()] = self.parameter_slots[pair.old.index()];
            }
            self.exchange_continuation_geometry(prepared);
        }
        // Every index was checked against these unchanged vectors in the preceding pass.
        for change in &proof.trims {
            core::mem::swap(
                &mut self.parameter_slots[change.old.index()],
                &mut prepared.parameter_slots[change.new.index()],
            );
        }
        self.live_notes.continue_note_bindings(&proof.notes);
        core::mem::swap(&mut self.plan, &mut prepared.plan);
    }

    #[cfg(feature = "mixed-transfer-rehearsal")]
    pub(crate) fn install_mixed_continuation(
        &mut self,
        prepared: &mut Self,
        proof: &PlanContinuation,
    ) {
        self.install_continuation(prepared, proof);
        core::mem::swap(&mut self.mixed_partition, &mut prepared.mixed_partition);
    }

    #[cfg(test)]
    pub(crate) fn continuation_has_active_steal_fade(&self) -> bool {
        self.node_states.iter().any(|state| {
            matches!(state,
            crate::node::kernels::NodeState::Sum { fade_remaining, .. } if *fade_remaining > 0)
        })
    }

    fn exchange_continuation_geometry(&mut self, prepared: &mut Self) {
        // Exhaustive classification: identity/time, carry, diagnostics and the actual
        // live registry stay in this renderer; prepared scratch and mapped state move.
        let Self {
            plan: _,
            mixed_partition: _,
            epoch: _,
            anchor: _,
            clock: _,
            channels: _,
            buffers: _,
            output_carry: _,
            carry_frames: _,
            live_notes: _,
            input_carry: _,
            node_states: _,
            parameter_slots: _,
            script_values: _,
            ramp_buffers: _,
            ramp_offsets: _,
            ramp_starts: _,
            history: _,
            modulated: _,
            prepass_resets: _,
            prepass_reset_due: _,
            record_instance: _,
            has_output: _,
            event_scratch: _,
            scratch_len: _,
            quantum_counts: _,
            timed_controls: _,
            adoption_gates: _,
            adoption_gate_slots: _,
            adoption_gate_len: _,
            control_starts: _,
            control_fill: _,
            diagnostics: _,
            // Structural plans with producer programs refuse cold. Exact continuation
            // keeps the actual arena, phase initialization and cumulative ledger here.
            producer_runtime: _,
            #[cfg(feature = "mixed-transfer-rehearsal")]
                rehearsal_usage: _,
        } = self;
        core::mem::swap(&mut self.buffers, &mut prepared.buffers);
        core::mem::swap(&mut self.node_states, &mut prepared.node_states);
        core::mem::swap(&mut self.parameter_slots, &mut prepared.parameter_slots);
        core::mem::swap(&mut self.script_values, &mut prepared.script_values);
        core::mem::swap(&mut self.ramp_buffers, &mut prepared.ramp_buffers);
        core::mem::swap(&mut self.ramp_offsets, &mut prepared.ramp_offsets);
        core::mem::swap(&mut self.ramp_starts, &mut prepared.ramp_starts);
        core::mem::swap(&mut self.history, &mut prepared.history);
        core::mem::swap(&mut self.modulated, &mut prepared.modulated);
        core::mem::swap(&mut self.prepass_resets, &mut prepared.prepass_resets);
        core::mem::swap(&mut self.prepass_reset_due, &mut prepared.prepass_reset_due);
        core::mem::swap(&mut self.record_instance, &mut prepared.record_instance);
        core::mem::swap(&mut self.has_output, &mut prepared.has_output);
        core::mem::swap(&mut self.event_scratch, &mut prepared.event_scratch);
        core::mem::swap(&mut self.quantum_counts, &mut prepared.quantum_counts);
        core::mem::swap(&mut self.timed_controls, &mut prepared.timed_controls);
        core::mem::swap(&mut self.adoption_gates, &mut prepared.adoption_gates);
        core::mem::swap(
            &mut self.adoption_gate_slots,
            &mut prepared.adoption_gate_slots,
        );
        core::mem::swap(&mut self.control_starts, &mut prepared.control_starts);
        core::mem::swap(&mut self.control_fill, &mut prepared.control_fill);
        // All old resolved/control scratch was consumed at B; successor uses fresh geometry.
        self.scratch_len = 0;
    }
}
