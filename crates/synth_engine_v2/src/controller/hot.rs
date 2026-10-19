//! Prevalidated allocation-free plan binding operations.
use super::{ControllerChange, ControllerRestore};
#[cfg(feature = "mixed-transfer-rehearsal")]
impl ControllerChange {
    pub(crate) fn apply_plan_bindings(
        &mut self,
        bindings: crate::plan::binding::FixedPlanBindings,
    ) {
        self.slot.slot = bindings.parameter(self.slot.slot);
    }
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl ControllerRestore {
    pub(crate) fn apply_plan_bindings(
        &mut self,
        bindings: crate::plan::binding::FixedPlanBindings,
    ) {
        self.slot.slot = bindings.parameter(self.slot.slot);
    }
}
