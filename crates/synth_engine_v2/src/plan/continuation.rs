//! Exact-layout proof for a compiled-only native voice continuation.
//! Every plan field is checked; only authored non-voice Trim bases may differ.
use super::*;
use crate::{
    node::{Smoothing, kernels::TRIM_LEVEL},
    quantities::{ParameterValue, PreparedBytes},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ContinuationError {
    #[error("the active Play has no matching adopted timeline witness")]
    Unavailable,
    #[error("the compiled plan is outside the exact-layout native continuation scope")]
    Plan,
    #[error("the compiled note/automation program changed")]
    Timeline,
    #[error("a plan-relative continuation binding is inconsistent")]
    Binding,
    #[error("continuation preparation exceeds its stamped-event limit")]
    EventLimit,
    #[error("continuation metadata exceeds its byte allowance")]
    Bytes,
    #[error("continuation metadata allocation failed")]
    Allocation,
    #[error("continuation runtime storage or authority is inconsistent")]
    Storage,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TrimChange {
    pub(crate) old: ParameterRow,
    pub(crate) new: ParameterRow,
    pub(crate) old_base: ParameterValue,
    pub(crate) new_base: ParameterValue,
}

#[derive(Debug)]
pub(crate) struct PlanContinuation {
    pub(crate) old: PlanId,
    pub(crate) new: PlanId,
    pub(crate) trims: Vec<TrimChange>,
    pub(crate) bytes: PreparedBytes,
}

impl CompiledPlan {
    pub(crate) fn prepare_continuation(
        &self,
        other: &Self,
        allowance: PreparedBytes,
    ) -> Result<PlanContinuation, ContinuationError> {
        // Exhaustive, without `..`: adding a plan field forces its proof classification.
        let Self {
            id: _,
            ops: _,
            regions: _,
            prepared_scripts: _,
            prepared_nodes: _,
            node_timings: _,
            non_voice_state_nodes: _,
            authored_state_nodes: _,
            parameter_targets: _,
            parameter_addresses: _,
            taps: _,
            tap_addresses: _,
            channels: _,
            buses: _,
            sends: _,
            note_targets: _,
            note_addresses: _,
            note_magnitudes: _,
            prepared_tunings: _,
            prepared_samples: _,
            channel_layout: _,
            sample_rate: _,
            maximum_block_size: _,
            max_events_per_quantum: _,
            compiled_event_share: _,
            note_producer_ranges: _,
            note_producer_holds: _,
            authored_sources: _,
            compiled_note_producer: _,
            forward_event_horizon: _,
            path_latencies: _,
            stealing: _,
            instance_groups: _,
            sum_groups: _,
            prepass: _,
        } = self;
        if !(self.regions == other.regions
            && self.node_timings == other.node_timings
            && self.non_voice_state_nodes == other.non_voice_state_nodes
            && self.authored_state_nodes == other.authored_state_nodes
            && self.taps == other.taps
            && self.prepared_tunings == other.prepared_tunings
            && self.channel_layout == other.channel_layout
            && self.sample_rate == other.sample_rate
            && self.maximum_block_size == other.maximum_block_size
            && self.max_events_per_quantum == other.max_events_per_quantum
            && self.compiled_event_share == other.compiled_event_share
            && self.note_producer_ranges == other.note_producer_ranges
            && self.note_producer_holds == other.note_producer_holds
            && self.compiled_note_producer == other.compiled_note_producer
            && self.forward_event_horizon == other.forward_event_horizon
            && self.path_latencies == other.path_latencies
            && self.stealing == other.stealing
            && self.instance_groups == other.instance_groups
            && self.sum_groups == other.sum_groups
            && self.prepass == other.prepass)
            || !self.prepared_scripts.is_empty()
            || !other.prepared_scripts.is_empty()
            || !self.prepared_samples.is_empty()
            || !other.prepared_samples.is_empty()
            || !self.authored_sources.is_empty()
            || !other.authored_sources.is_empty()
        {
            return Err(ContinuationError::Plan);
        }
        self.continuation_ops(other)?;
        self.continuation_addresses(other)?;
        self.continuation_notes(other)?;
        self.continuation_mix(other)?;
        self.continuation_nodes(other)?;
        if self.parameter_targets.len() != other.parameter_targets.len() {
            return Err(ContinuationError::Plan);
        }
        let required = self
            .parameter_targets
            .len()
            .checked_mul(size_of::<TrimChange>())
            .and_then(|bytes| bytes.checked_add(size_of::<PlanContinuation>()))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(ContinuationError::Bytes)?;
        if required > allowance.get() {
            return Err(ContinuationError::Bytes);
        }
        let mut trims = Vec::new();
        trims
            .try_reserve_exact(self.parameter_targets.len())
            .map_err(|_| ContinuationError::Allocation)?;
        for (index, (old, new)) in self
            .parameter_targets
            .iter()
            .zip(&other.parameter_targets)
            .enumerate()
        {
            if old == new && old.base.as_f32().to_bits() == new.base.as_f32().to_bits() {
                continue;
            }
            let mut normalized = *new;
            normalized.base = old.base;
            if *old != normalized || old.instances.get() != 1 || old.smoothing != Smoothing::None {
                return Err(ContinuationError::Plan);
            }
            let (old_level, new_level) = self.changed_trim(other, old.node)?;
            if old.control != TRIM_LEVEL
                || old.base != ParameterValue::from_amplitude(old_level)
                || new.base != ParameterValue::from_amplitude(new_level)
            {
                return Err(ContinuationError::Plan);
            }
            trims.push(TrimChange {
                old: ParameterRow::new(self.id, index),
                new: ParameterRow::new(other.id, index),
                old_base: old.base,
                new_base: new.base,
            });
        }
        let bytes = trims
            .capacity()
            .checked_mul(size_of::<TrimChange>())
            .and_then(|bytes| bytes.checked_add(size_of::<PlanContinuation>()))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(ContinuationError::Bytes)?;
        if bytes > allowance.get() {
            return Err(ContinuationError::Bytes);
        }
        Ok(PlanContinuation {
            old: self.id,
            new: other.id,
            trims,
            bytes: PreparedBytes::measured(bytes),
        })
    }

    fn continuation_ops(&self, other: &Self) -> Result<(), ContinuationError> {
        if self.ops.len() != other.ops.len() {
            return Err(ContinuationError::Plan);
        }
        for (old, new) in self.ops.iter().zip(&other.ops) {
            match (old, new) {
                (PlanOp::Node(a), PlanOp::Node(b)) if a == b && a.prepared == b.prepared => {}
                (PlanOp::Output { .. }, PlanOp::Output { .. }) if old == new => {}
                _ => return Err(ContinuationError::Plan),
            }
        }
        Ok(())
    }

    fn continuation_addresses(&self, other: &Self) -> Result<(), ContinuationError> {
        if self.parameter_addresses.len() != other.parameter_addresses.len()
            || self.note_addresses.len() != other.note_addresses.len()
        {
            return Err(ContinuationError::Plan);
        }
        for (old, new) in self
            .parameter_addresses
            .iter()
            .zip(&other.parameter_addresses)
        {
            if old.slot.plan() != self.id || new.slot.plan() != other.id {
                return Err(ContinuationError::Binding);
            }
            let mut normalized = *new;
            normalized.slot = ParameterSlot::new(self.id, new.slot.index());
            if *old != normalized {
                return Err(ContinuationError::Plan);
            }
        }
        for (old, new) in self.note_addresses.iter().zip(&other.note_addresses) {
            if old.slot.plan() != self.id || new.slot.plan() != other.id {
                return Err(ContinuationError::Binding);
            }
            let mut normalized = *new;
            normalized.slot = NoteSlot::new(self.id, new.slot.index());
            if *old != normalized {
                return Err(ContinuationError::Plan);
            }
        }
        Ok(())
    }

    fn continuation_notes(&self, other: &Self) -> Result<(), ContinuationError> {
        if self.note_targets.len() != other.note_targets.len()
            || self.note_magnitudes.len() != other.note_magnitudes.len()
        {
            return Err(ContinuationError::Plan);
        }
        for (old, new) in self.note_targets.iter().zip(&other.note_targets) {
            if old.parameter.plan() != self.id || new.parameter.plan() != other.id {
                return Err(ContinuationError::Binding);
            }
            let mut normalized = *new;
            normalized.parameter = ParameterSlot::new(self.id, new.parameter.index());
            if *old != normalized {
                return Err(ContinuationError::Plan);
            }
        }
        for (old, new) in self.note_magnitudes.iter().zip(&other.note_magnitudes) {
            if old.parameter.plan() != self.id || new.parameter.plan() != other.id {
                return Err(ContinuationError::Binding);
            }
            let mut normalized = *new;
            normalized.parameter = ParameterSlot::new(self.id, new.parameter.index());
            normalized.tuning = match (old.tuning, new.tuning) {
                (Some(a), Some(b)) if a.plan() == self.id && b.plan() == other.id => {
                    Some(TuningSlot::new(self.id, b.index()))
                }
                (None, None) => None,
                _ => return Err(ContinuationError::Binding),
            };
            if *old != normalized {
                return Err(ContinuationError::Plan);
            }
        }
        Ok(())
    }

    // Mixer/tap IDs are plan-relative addresses. Exact record equality after checked
    // relabelling preserves scope, physical destinations and every semantic field.
    // Split SessionAudio has no observer-subscription API; tap regions stay identical.
    fn continuation_mix(&self, other: &Self) -> Result<(), ContinuationError> {
        if self.channels.len() != other.channels.len()
            || self.buses.len() != other.buses.len()
            || self.sends.len() != other.sends.len()
            || self.tap_addresses.len() != other.tap_addresses.len()
        {
            return Err(ContinuationError::Plan);
        }
        for (old, new) in self.channels.iter().zip(&other.channels) {
            if old.id.plan() != self.id
                || new.id.plan() != other.id
                || old.id.index() >= self.channels.len()
                || new.id.index() >= other.channels.len()
            {
                return Err(ContinuationError::Binding);
            }
            let mut normalized = *new;
            normalized.id = ChannelId::new(self.id, new.id.index());
            normalized.fader = self.relabel_parameter_from(other, new.fader)?;
            normalized.pan = self.relabel_parameter_from(other, new.pan)?;
            normalized.mute = self.relabel_parameter_from(other, new.mute)?;
            if *old != normalized {
                return Err(ContinuationError::Plan);
            }
        }
        for (old, new) in self.buses.iter().zip(&other.buses) {
            if old.id.plan() != self.id
                || new.id.plan() != other.id
                || old.id.index() >= self.buses.len()
                || new.id.index() >= other.buses.len()
            {
                return Err(ContinuationError::Binding);
            }
            let mut normalized = *new;
            normalized.id = BusId::new(self.id, new.id.index());
            normalized.fader = self.relabel_parameter_from(other, new.fader)?;
            normalized.pan = self.relabel_parameter_from(other, new.pan)?;
            normalized.mute = self.relabel_parameter_from(other, new.mute)?;
            if *old != normalized {
                return Err(ContinuationError::Plan);
            }
        }
        for (old, new) in self.sends.iter().zip(&other.sends) {
            let mut normalized = *new;
            normalized.from = match new.from {
                SendSource::Channel(channel)
                    if channel.plan() == other.id && channel.index() < other.channels.len() =>
                {
                    SendSource::Channel(ChannelId::new(self.id, channel.index()))
                }
                SendSource::Bus(bus)
                    if bus.plan() == other.id && bus.index() < other.buses.len() =>
                {
                    SendSource::Bus(BusId::new(self.id, bus.index()))
                }
                _ => return Err(ContinuationError::Binding),
            };
            if new.into.plan() != other.id || new.into.index() >= other.buses.len() {
                return Err(ContinuationError::Binding);
            }
            normalized.into = BusId::new(self.id, new.into.index());
            normalized.level = self.relabel_parameter_from(other, new.level)?;
            normalized.mute = self.relabel_parameter_from(other, new.mute)?;
            if *old != normalized {
                return Err(ContinuationError::Plan);
            }
        }
        for (old, new) in self.tap_addresses.iter().zip(&other.tap_addresses) {
            if old.slot.plan() != self.id
                || new.slot.plan() != other.id
                || old.slot.index() >= self.taps.len()
                || new.slot.index() >= other.taps.len()
            {
                return Err(ContinuationError::Binding);
            }
            let mut normalized = *new;
            normalized.slot = TapSlot::new(self.id, new.slot.index());
            if *old != normalized {
                return Err(ContinuationError::Plan);
            }
        }
        Ok(())
    }

    fn relabel_parameter_from(
        &self,
        other: &Self,
        slot: ParameterSlot,
    ) -> Result<ParameterSlot, ContinuationError> {
        if slot.plan() != other.id
            || slot.index() >= self.parameter_targets.len()
            || slot.index() >= other.parameter_targets.len()
        {
            return Err(ContinuationError::Binding);
        }
        Ok(ParameterSlot::new(self.id, slot.index()))
    }

    fn continuation_nodes(&self, other: &Self) -> Result<(), ContinuationError> {
        if self.prepared_nodes.len() != other.prepared_nodes.len() {
            return Err(ContinuationError::Plan);
        }
        for (index, (old, new)) in self
            .prepared_nodes
            .iter()
            .zip(&other.prepared_nodes)
            .enumerate()
        {
            if !eligible(old) || !eligible(new) {
                return Err(ContinuationError::Plan);
            }
            if old == new {
                continue;
            }
            let mut referenced = false;
            for op in &self.ops {
                let PlanOp::Node(step) = op else {
                    continue;
                };
                if step.prepared.index() != index {
                    continue;
                }
                referenced = true;
                let _levels = self.changed_trim(other, step.node)?;
                let Some(old_target) = self
                    .parameter_targets
                    .iter()
                    .find(|target| target.node == step.node && target.control == TRIM_LEVEL)
                else {
                    return Err(ContinuationError::Plan);
                };
                let Some(new_target) = other
                    .parameter_targets
                    .iter()
                    .find(|target| target.node == step.node && target.control == TRIM_LEVEL)
                else {
                    return Err(ContinuationError::Plan);
                };
                if old_target.base.as_f32().to_bits() == new_target.base.as_f32().to_bits() {
                    return Err(ContinuationError::Plan);
                }
            }
            if !referenced {
                return Err(ContinuationError::Plan);
            }
        }
        Ok(())
    }

    fn changed_trim(
        &self,
        other: &Self,
        node: NodeSlot,
    ) -> Result<(crate::quantities::Amplitude, crate::quantities::Amplitude), ContinuationError>
    {
        let step = self
            .ops
            .iter()
            .find_map(|op| match op {
                PlanOp::Node(step) if step.node == node => Some(step),
                _ => None,
            })
            .ok_or(ContinuationError::Binding)?;
        if step.role != NodeRole::Global
            || !self
                .authored_state_nodes
                .iter()
                .any(|(_, scope, slot)| *slot == node && *scope != crate::ir::ExecutionScope::Voice)
        {
            return Err(ContinuationError::Plan);
        }
        match (
            self.prepared_nodes.get(step.prepared.index()),
            other.prepared_nodes.get(step.prepared.index()),
        ) {
            (Some(PreparedNode::Trim { level: a }), Some(PreparedNode::Trim { level: b }))
                if a.as_f32().to_bits() != b.as_f32().to_bits() =>
            {
                Ok((*a, *b))
            }
            _ => Err(ContinuationError::Plan),
        }
    }
}

const fn eligible(node: &PreparedNode) -> bool {
    matches!(
        node,
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
            | PreparedNode::Send { .. }
            | PreparedNode::PostFaderSend { .. }
    )
}
