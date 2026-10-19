//! The internal producer's terminal attribution, called at most once per epoch.
use super::DiagnosticsReport;
impl DiagnosticsReport {
    pub(crate) fn count_internal_producer_fault(&mut self) {
        self.internal_producer_faults = self.internal_producer_faults.saturating_add(1);
    }
}
