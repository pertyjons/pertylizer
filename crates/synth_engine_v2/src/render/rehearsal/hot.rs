//! Bounded rehearsal counter mutations reached from the render loop.
//! This entire file belongs to render_loop_purity's checked real-time region.
use super::{PreparedRenderer, RehearsalCounters};

impl RehearsalCounters {
    /// Existing render counters saturate for fault handling. An observation must
    /// retain the attempted overflow instead of treating that clamp as exact demand.
    pub(crate) fn require_representable_add(&mut self, left: u32, right: u32) {
        self.overflowed |= left.checked_add(right).is_none();
    }

    fn increment(value: &mut usize, overflowed: &mut bool) {
        match value.checked_add(1) {
            Some(next) => *value = next,
            None => *overflowed = true,
        }
    }

    pub(crate) fn begin_controls(&mut self) {
        self.required = 0;
        self.written = 0;
    }

    pub(crate) fn require_total(&mut self, count: u32) {
        match usize::try_from(count) {
            Ok(count) => self.required_peak = self.required_peak.max(count),
            Err(_) => self.overflowed = true,
        }
    }

    pub(crate) fn require_control(&mut self) {
        Self::increment(&mut self.required, &mut self.overflowed);
        self.required_peak = self.required_peak.max(self.required);
    }

    pub(crate) fn wrote_control(&mut self) {
        Self::increment(&mut self.written, &mut self.overflowed);
        self.written_peak = self.written_peak.max(self.written);
    }

    pub(crate) fn reject_control(&mut self) {
        Self::increment(&mut self.rejected, &mut self.overflowed);
    }

    pub(crate) fn payloads(&mut self, count: usize) {
        self.payload_peak = self.payload_peak.max(count);
    }

    pub(crate) fn seeded_rows(&mut self, count: usize) {
        self.seeded_peak = self.seeded_peak.max(count);
    }

    pub(crate) fn adoption_gates(&mut self, count: usize) {
        self.adoption_peak = self.adoption_peak.max(count);
    }
}

impl PreparedRenderer {
    pub(crate) fn observe_rehearsal_seeds(&mut self) {
        let mut seeded = 0;
        for slot in &self.parameter_slots {
            if slot.is_seeded() {
                RehearsalCounters::increment(&mut seeded, &mut self.rehearsal_usage.overflowed);
            }
        }
        self.rehearsal_usage.seeded_rows(seeded);
    }
}
