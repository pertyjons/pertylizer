//! Passive observations for the non-shipping mixed-transfer rehearsal only.
mod hot;
use super::{DueEvent, EventPayload, PreparedRenderer};
use crate::quantities::{EventCount, PreparedBytes, SlotCount};

/// A measurement that cannot be represented must fail explicitly, never clamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RehearsalUsageError {
    #[error("rehearsal usage counter is unrepresentable: {0}")]
    Counter(&'static str),
    #[error("rehearsal storage arithmetic overflowed: {0}")]
    Storage(&'static str),
}

/// Lifetime maxima for one renderer, retained even if a later quantum fails.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct RehearsalRendererUsage {
    /// Greatest node-run demand in one quantum, including attempted refused writes.
    pub timed_controls_required: EventCount,
    /// Greatest number of successfully stored node-run controls in one quantum.
    pub timed_controls_written: EventCount,
    /// Cumulative write refusals; observing them does not change sound fault handling.
    pub rejected_control_writes: EventCount,
    /// Highest resolved-event scratch occupancy of a renderer call. The mixed owner
    /// gives its renderer one quantum per call, even for a longer host callback.
    pub event_payloads: EventCount,
    /// Inline enum bytes, excluding event envelopes and resolved-event scratch.
    pub inline_payload_bytes: PreparedBytes,
    pub seeded_rows: SlotCount,
    pub adoption_gate_entries: EventCount,
}

/// Granted buffer storage, excluding allocator bookkeeping and the renderer's inline
/// owner. Parameter-slot bytes include their seed flags exactly once. These named
/// buffers are not a total memory or allocator-charge claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct RehearsalRendererStorage {
    pub controls: PreparedBytes,
    pub event_scratch: PreparedBytes,
    pub parameter_slots: PreparedBytes,
    /// Writable extents, independently fixed by preparation rather than observed peaks.
    pub timed_control_entries: EventCount,
    pub event_entries: EventCount,
    pub parameter_rows: SlotCount,
    pub adoption_gate_entries: EventCount,
}

#[derive(Debug, Default)]
pub(crate) struct RehearsalCounters {
    required: usize,
    written: usize,
    required_peak: usize,
    written_peak: usize,
    rejected: usize,
    payload_peak: usize,
    seeded_peak: usize,
    adoption_peak: usize,
    overflowed: bool,
}

pub(crate) fn event_count(
    value: usize,
    field: &'static str,
) -> Result<EventCount, RehearsalUsageError> {
    u32::try_from(value)
        .map(EventCount::measured)
        .map_err(|_| RehearsalUsageError::Counter(field))
}

pub(crate) fn bytes(
    value: usize,
    field: &'static str,
) -> Result<PreparedBytes, RehearsalUsageError> {
    u64::try_from(value)
        .map(PreparedBytes::measured)
        .map_err(|_| RehearsalUsageError::Storage(field))
}

impl RehearsalCounters {
    fn snapshot(&self) -> Result<RehearsalRendererUsage, RehearsalUsageError> {
        if self.overflowed {
            return Err(RehearsalUsageError::Counter("renderer internal counter"));
        }
        let payload_bytes = self
            .payload_peak
            .checked_mul(size_of::<EventPayload>())
            .ok_or(RehearsalUsageError::Storage("inline event payloads"))?;
        Ok(RehearsalRendererUsage {
            timed_controls_required: event_count(self.required_peak, "required timed controls")?,
            timed_controls_written: event_count(self.written_peak, "written timed controls")?,
            rejected_control_writes: event_count(self.rejected, "rejected timed controls")?,
            event_payloads: event_count(self.payload_peak, "event payloads")?,
            inline_payload_bytes: bytes(payload_bytes, "inline event payloads")?,
            seeded_rows: u32::try_from(self.seeded_peak)
                .map(SlotCount::measured)
                .map_err(|_| RehearsalUsageError::Counter("seeded rows"))?,
            adoption_gate_entries: event_count(self.adoption_peak, "adoption gate entries")?,
        })
    }
}

impl PreparedRenderer {
    pub(crate) fn rehearsal_usage(&self) -> Result<RehearsalRendererUsage, RehearsalUsageError> {
        self.rehearsal_usage.snapshot()
    }

    pub(crate) fn rehearsal_storage(
        &self,
    ) -> Result<RehearsalRendererStorage, RehearsalUsageError> {
        let event_bytes = self
            .event_scratch
            .capacity()
            .checked_mul(size_of::<DueEvent>())
            .and_then(|value| {
                self.quantum_counts
                    .capacity()
                    .checked_mul(size_of::<u32>())
                    .and_then(|counts| value.checked_add(counts))
            })
            .ok_or(RehearsalUsageError::Storage("event scratch"))?;
        let slots = self
            .parameter_slots
            .capacity()
            .checked_mul(size_of::<super::slot::SlotState>())
            .ok_or(RehearsalUsageError::Storage("parameter slots"))?;
        Ok(RehearsalRendererStorage {
            controls: bytes(self.control_scratch_bytes(), "control scratch")?,
            event_scratch: bytes(event_bytes, "event scratch")?,
            parameter_slots: bytes(slots, "parameter slots")?,
            timed_control_entries: event_count(self.timed_controls.len(), "timed control extent")?,
            event_entries: event_count(self.event_scratch.len(), "event scratch extent")?,
            parameter_rows: u32::try_from(self.parameter_slots.len())
                .map(SlotCount::measured)
                .map_err(|_| RehearsalUsageError::Counter("parameter row extent"))?,
            adoption_gate_entries: event_count(
                self.adoption_gate_slots.len(),
                "adoption gate extent",
            )?,
        })
    }
}
