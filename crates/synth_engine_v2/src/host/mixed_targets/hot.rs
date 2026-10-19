//! Prevalidated allocation-free plan binding operations.
use super::MixedInstancePartition;
#[cfg(feature = "mixed-transfer-rehearsal")]
impl MixedInstancePartition {
    /// Fixed native continuation changes plan labels, never producer/class geometry.
    pub(crate) fn same_fixed_geometry(&self, other: &Self) -> bool {
        if self.compiled_producer != other.compiled_producer
            || self.live_producer != other.live_producer
            || self.compiled_span != other.compiled_span
            || self.live_span != other.live_span
            || self.compiled_nodes != other.compiled_nodes
            || self.live_nodes != other.live_nodes
            || self.shared_sum_nodes != other.shared_sum_nodes
            || self.global_nodes != other.global_nodes
            || self.restoration_groups.len() != other.restoration_groups.len()
        {
            return false;
        }
        for (old, new) in [
            (&self.compiled_rows, &other.compiled_rows),
            (&self.live_rows, &other.live_rows),
            (&self.shared_sum_rows, &other.shared_sum_rows),
            (&self.global_rows, &other.global_rows),
        ] {
            if old.len() != new.len() {
                return false;
            }
            for (old, new) in old.iter().zip(new) {
                if old.plan() != self.plan || new.plan() != other.plan || old.index() != new.index()
                {
                    return false;
                }
            }
        }
        for (old, new) in self
            .restoration_groups
            .iter()
            .zip(&other.restoration_groups)
        {
            if old.parameter.plan() != self.plan
                || new.parameter.plan() != other.plan
                || old.parameter.index() != new.parameter.index()
                || old.instances != new.instances
            {
                return false;
            }
        }
        true
    }
}
