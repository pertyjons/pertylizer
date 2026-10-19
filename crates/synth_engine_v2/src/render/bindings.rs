//! Prevalidated allocation-free plan binding operations.
use super::ScopedParameterRestore;
#[cfg(feature = "mixed-transfer-rehearsal")]
impl ScopedParameterRestore {
    pub(crate) fn apply_plan_bindings(
        &mut self,
        bindings: crate::plan::binding::FixedPlanBindings,
    ) {
        self.slot = bindings.parameter(self.slot);
    }
}

impl super::PreparedRenderer {
    pub(crate) fn continuation_partition_matches(
        &self,
        partition: &std::sync::Arc<crate::host::mixed_targets::MixedInstancePartition>,
    ) -> bool {
        self.mixed_partition
            .as_ref()
            .is_some_and(|bound| std::sync::Arc::ptr_eq(bound, partition))
    }
}
