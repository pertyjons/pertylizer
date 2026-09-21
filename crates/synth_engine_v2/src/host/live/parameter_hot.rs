//! Fixed-slot parameter publication, separate from performance ingress.
use super::{LiveInputError, LiveInputStream, UpdateStatus, UpdateVersion};
use crate::{node::ParameterUnit, plan::ParameterSlot, quantities::ParameterValue};
impl LiveInputStream {
    /// Returns the superseded pending version, if any. Versions never reset in a plan.
    pub fn update_parameter(
        &mut self,
        slot: ParameterSlot,
        version: UpdateVersion,
        value: ParameterValue,
    ) -> Result<Option<UpdateVersion>, LiveInputError> {
        if self.closed {
            return Err(LiveInputError::Closed);
        }
        let index = self
            .parameter_indices
            .get(slot.index())
            .copied()
            .flatten()
            .ok_or(LiveInputError::Identity)?;
        let cell = self
            .parameters
            .get_mut(index)
            .ok_or(LiveInputError::Identity)?;
        if cell.slot != slot {
            return Err(LiveInputError::Identity);
        }
        if cell.version.is_some_and(|prior| version <= prior) {
            return Err(LiveInputError::Identity);
        }
        let value_raw = value.as_f32();
        let valid = match cell.unit {
            ParameterUnit::ScriptScalar(range) => {
                (range.minimum()..=range.maximum()).contains(&value_raw)
            }
            ParameterUnit::BipolarLevel => (-1.0..=1.0).contains(&value_raw),
            ParameterUnit::NormalizedLevel => (0.0..=1.0).contains(&value_raw),
            ParameterUnit::Seconds => value_raw >= 0.0,
            ParameterUnit::DelayFeedback => (0.0..=0.95).contains(&value_raw),
            ParameterUnit::DelayTime => (0.001..=2.0).contains(&value_raw),
            ParameterUnit::Gate => false,
            ParameterUnit::QualityFactor => value_raw > 0.0,
            ParameterUnit::Hertz | ParameterUnit::LinearAmplitude => true,
        };
        if !valid {
            return Err(LiveInputError::Configuration);
        }
        let replaced = cell.pending.map(|(version, _)| version);
        cell.pending = Some((version, value));
        cell.version = Some(version);
        Ok(replaced)
    }
    pub(super) fn cancel_pending_parameters(&mut self) {
        for cell in &mut self.parameters {
            if let Some((version, _)) = cell.pending.take() {
                cell.cancelled = Some(version);
            }
        }
    }
    pub(super) fn cancel_parameters(&mut self) {
        self.cancel_pending_parameters();
        for cell in &mut self.parameters {
            if let Some(version) = cell.staged.take() {
                cell.cancelled = Some(version);
            }
        }
    }
    pub fn parameter_status(&self, slot: ParameterSlot) -> Option<UpdateStatus> {
        let index = self
            .parameter_indices
            .get(slot.index())
            .copied()
            .flatten()?;
        let cell = self.parameters.get(index)?;
        if cell.slot != slot {
            return None;
        }
        let latest = cell.version?;
        Some(if cell.cancelled == Some(latest) {
            UpdateStatus::Cancelled(latest)
        } else if cell.applied == Some(latest) {
            UpdateStatus::Applied(latest)
        } else {
            UpdateStatus::Pending(latest)
        })
    }
    pub fn applied_parameter(&self, slot: ParameterSlot) -> Option<UpdateVersion> {
        let index = self
            .parameter_indices
            .get(slot.index())
            .copied()
            .flatten()?;
        let cell = self.parameters.get(index)?;
        if cell.slot != slot {
            return None;
        }
        cell.applied
    }
}
