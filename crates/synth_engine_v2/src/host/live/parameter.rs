//! Off-thread preparation for a separate, coalescing parameter lane.
use super::{LiveInputError, LiveInputStream};
use crate::{
    node::ParameterUnit,
    plan::{ControlRate, ParameterSlot},
    quantities::{ParameterValue, PreparedBytes},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct UpdateVersion(u64);
impl UpdateVersion {
    pub fn new(value: u64) -> Result<Self, LiveInputError> {
        if value == 0 {
            return Err(LiveInputError::Identity);
        }
        Ok(Self(value))
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStatus {
    Pending(UpdateVersion),
    Applied(UpdateVersion),
    Cancelled(UpdateVersion),
}
#[derive(Clone, Copy)]
pub(super) struct ParameterCell {
    pub slot: ParameterSlot,
    pub unit: ParameterUnit,
    pub version: Option<UpdateVersion>,
    pub pending: Option<(UpdateVersion, ParameterValue)>,
    pub staged: Option<UpdateVersion>,
    pub applied: Option<UpdateVersion>,
    pub cancelled: Option<UpdateVersion>,
}
impl LiveInputStream {
    /// Resolve an address off-callback; later writes use the compiled slot only.
    pub fn resolve_parameter(
        &self,
        node: crate::ir::NodeId,
        parameter: crate::ir::ParameterId,
    ) -> Option<ParameterSlot> {
        self.control.plan().resolve_parameter(node, parameter)
    }

    /// Call once before rendering or input admission. The grant includes existing storage.
    /// Note-owned and sample-positioned controls cannot be overridden by this lane.
    pub fn prepare_parameters(
        &mut self,
        slots: &[ParameterSlot],
        ceiling: PreparedBytes,
    ) -> Result<(), LiveInputError> {
        if !self.parameters.is_empty()
            || self.clock() != crate::time::SampleTime::ZERO
            || self.sources.iter().any(|source| source.serial != 0)
            || slots.len()
                >= usize::try_from(self.arbiter.session_share().get())
                    .map_err(|_| LiveInputError::Bytes)?
        {
            return Err(LiveInputError::Configuration);
        }
        let extra = slots
            .len()
            .checked_mul(size_of::<ParameterCell>())
            .and_then(|n| {
                n.checked_add(
                    self.control
                        .plan()
                        .parameter_targets()
                        .len()
                        .checked_mul(size_of::<Option<usize>>())?,
                )
            })
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(LiveInputError::Bytes)?;
        let bytes = self
            .bytes
            .get()
            .checked_add(extra)
            .ok_or(LiveInputError::Bytes)?;
        if bytes > ceiling.get() {
            return Err(LiveInputError::Bytes);
        }
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(slots.len())
            .map_err(|_| LiveInputError::Allocation)?;
        if cells.capacity() != slots.len() {
            return Err(LiveInputError::Bytes);
        }
        let plan = self.control.plan();
        for (index, slot) in slots.iter().copied().enumerate() {
            if slot.plan() != plan.id() || slots[..index].contains(&slot) {
                return Err(LiveInputError::Identity);
            }
            let target = plan
                .parameter_targets()
                .get(slot.index())
                .ok_or(LiveInputError::Configuration)?;
            if target.rate != ControlRate::Quantum
                || plan
                    .note_targets()
                    .iter()
                    .any(|note| note.parameter == slot)
                || plan
                    .note_magnitudes()
                    .iter()
                    .any(|magnitude| magnitude.parameter == slot)
            {
                return Err(LiveInputError::Configuration);
            }
            cells.push(ParameterCell {
                slot,
                unit: target.unit,
                version: None,
                pending: None,
                staged: None,
                applied: None,
                cancelled: None,
            });
        }
        let mut indices = super::slots(self.control.plan().parameter_targets().len())?;
        for (index, cell) in cells.iter().enumerate() {
            indices[cell.slot.index()] = Some(index);
        }
        self.parameter_indices = indices;
        self.parameters = cells.into_boxed_slice();
        self.bytes = PreparedBytes::measured(bytes);
        Ok(())
    }
}
