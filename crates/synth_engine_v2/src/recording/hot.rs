//! Borrowed fixed-capacity storage operations; no allocation, ownership transfer or drop.

use super::{
    CaptureBuffer, CaptureError, CaptureOutcome, CaptureWindow, DiagnosticCount, LateAttribution,
    LateCaptureInput, SimulatedTakeStore, SourceAttribution, TakeReservation, TakeSlot,
};
use crate::host::ConnectionGeneration;
use crate::time::{SampleTime, StreamEpoch};

impl DiagnosticCount {
    fn increment(&mut self) {
        if let Some(next) = self.value.checked_add(1) {
            self.value = next;
        } else {
            self.saturated = true;
        }
    }
}

impl<T: Copy> SimulatedTakeStore<T> {
    pub(super) fn slot_mut(
        &mut self,
        ticket: TakeReservation,
    ) -> Result<&mut TakeSlot<T>, CaptureError> {
        let slot = self
            .slots
            .get_mut(ticket.slot)
            .ok_or(CaptureError::StaleReservation)?;
        match slot.state {
            Some(state) if state.id == ticket.id => Ok(slot),
            _ => Err(CaptureError::StaleReservation),
        }
    }

    /// Append one synthetic storage cell. This is not validation or publication of
    /// physical performance input. An exhausted region stops ordinary writes and leaves
    /// every accepted cell and all emergency metadata intact for later finalization.
    pub fn push_fixture(&mut self, ticket: TakeReservation, value: T) -> Result<(), CaptureError> {
        let capacity = self.layout.lengths[CaptureBuffer::Ordinary.index()];
        let slot = self.slot_mut(ticket)?;
        let Some(state) = &mut slot.state else {
            return Err(CaptureError::StaleReservation);
        };
        if state.outcome.is_some() {
            return Err(CaptureError::Sealed);
        }
        if state.forced_partial {
            return Err(CaptureError::CaptureStopped);
        }
        if state.ordinary_len >= capacity {
            state.forced_partial = true;
            return Err(CaptureError::OrdinaryFull);
        }
        let cell = slot
            .data
            .get_mut(state.ordinary_len)
            .ok_or(CaptureError::MetadataBounds)?;
        *cell = Some(value);
        state.ordinary_len += 1;
        Ok(())
    }

    /// Fill or update one already-reserved metadata cell before sealing. Ordinary
    /// exhaustion does not close this reserve. Index is a collection offset, never ID.
    pub fn write_fixture_metadata(
        &mut self,
        ticket: TakeReservation,
        buffer: CaptureBuffer,
        index: usize,
        value: T,
    ) -> Result<(), CaptureError> {
        if buffer == CaptureBuffer::Ordinary || index >= self.layout.lengths[buffer.index()] {
            return Err(CaptureError::MetadataBounds);
        }
        // Layout construction proved the entire region fits inside the data allocation.
        let at = self.layout.starts[buffer.index()] + index;
        let slot = self.slot_mut(ticket)?;
        if slot.state.is_some_and(|state| state.outcome.is_some()) {
            return Err(CaptureError::Sealed);
        }
        let cell = slot.data.get_mut(at).ok_or(CaptureError::MetadataBounds)?;
        *cell = Some(value);
        Ok(())
    }

    /// Synthetic acknowledgement of a consumed publication fence. No physical callback
    /// or worker fence is inferred from this method; the real source adapter must provide
    /// it before using watermark admission. The fence updates every take sharing this
    /// generation. Zero lateness still needs the explicit fence.
    pub fn acknowledge_fixture_fence(
        &mut self,
        ticket: TakeReservation,
        source: ConnectionGeneration,
        epoch: StreamEpoch,
        frontier: SampleTime,
    ) -> Result<(), CaptureError> {
        let watermark = SampleTime::new(
            frontier
                .as_u64()
                .saturating_sub(self.lateness_allowance.as_u64()),
        );
        let slot = self.slot_mut(ticket)?;
        check_epoch(slot, epoch)?;
        let ledger = source_mut(slot, source)?;
        if ledger.quiescent {
            return Err(CaptureError::SourceQuiescent);
        }
        if watermark < ledger.watermark || frontier < ledger.frontier {
            return Err(CaptureError::WatermarkRetreat);
        }
        self.update_source_fence(source, watermark, frontier, false);
        Ok(())
    }

    /// Record this source generation's synthetic quiescence across all retained takes
    /// without another render callback.
    /// Later fence/fault calls for this source refuse; other sources may still update
    /// take quality. Ordinary fixture writes are opaque storage operations: the real
    /// publisher must enforce source admission before appending a cell.
    pub fn acknowledge_fixture_quiescence(
        &mut self,
        ticket: TakeReservation,
        source: ConnectionGeneration,
        epoch: StreamEpoch,
        last_valid: SampleTime,
    ) -> Result<(), CaptureError> {
        let slot = self.slot_mut(ticket)?;
        check_epoch(slot, epoch)?;
        let ledger = source_mut(slot, source)?;
        if ledger.quiescent {
            return Err(CaptureError::SourceQuiescent);
        }
        if last_valid < ledger.watermark {
            return Err(CaptureError::WatermarkRetreat);
        }
        self.update_source_fence(source, last_valid, last_valid, true);
        Ok(())
    }

    fn update_source_fence(
        &mut self,
        generation: ConnectionGeneration,
        watermark: SampleTime,
        frontier: SampleTime,
        quiescent: bool,
    ) {
        for slot in &mut self.slots {
            for source in slot.sources.iter_mut().flatten() {
                if source.generation == generation {
                    source.watermark = watermark;
                    source.frontier = source.frontier.max(frontier);
                    source.fenced = true;
                    source.quiescent = quiescent;
                }
            }
        }
    }

    /// Exercise the already-reserved quality slot for one synthetic source refusal.
    /// The real publisher still owes per-source ordering and attribution to every
    /// retained interval; this method does not implement that publisher.
    pub fn attribute_fixture_late(
        &mut self,
        ticket: TakeReservation,
        source: ConnectionGeneration,
        epoch: StreamEpoch,
        time: SampleTime,
    ) -> Result<LateAttribution, CaptureError> {
        let slot = self.slot_mut(ticket)?;
        check_epoch(slot, epoch)?;
        let ledger = source_mut(slot, source)?;
        if ledger.quiescent {
            return Err(CaptureError::SourceQuiescent);
        }
        if time >= ledger.watermark {
            return Err(CaptureError::NotLate);
        }
        let Some(state) = &mut slot.state else {
            return Err(CaptureError::StaleReservation);
        };
        if time < state.window.start || time >= state.window.end {
            return Ok(LateAttribution::OutsideSelectedInterval);
        }
        if state.quality.first_late.is_none() {
            state.quality.first_late = Some(LateCaptureInput { source, time });
        }
        state.quality.late_count.increment();
        if state.outcome.is_none() {
            state.forced_partial = true;
        }
        Ok(LateAttribution::TakeFaulted)
    }

    /// Seal synthetic storage once every source's acknowledged frontier covers the
    /// selected stop. Interrupted finalization also requires source quiescence. The
    /// caller owns the semantic meaning of its cells and supplies finalization metadata;
    /// this storage layer does not assert note pairing, projection or asset validity.
    pub fn seal_fixture(
        &mut self,
        ticket: TakeReservation,
        outcome: CaptureOutcome,
        stop: SampleTime,
    ) -> Result<(), CaptureError> {
        let slot = self.slot_mut(ticket)?;
        let Some(state) = &mut slot.state else {
            return Err(CaptureError::StaleReservation);
        };
        if state.outcome.is_some() {
            return Err(CaptureError::Sealed);
        }
        if stop > state.window.end
            || (stop < state.window.start && outcome != CaptureOutcome::Interrupted)
        {
            return Err(CaptureError::InvalidStop);
        }
        for source in slot.sources.iter().flatten() {
            if !source.fenced || source.watermark < stop {
                return Err(CaptureError::AwaitingFences);
            }
            if outcome == CaptureOutcome::Interrupted && !source.quiescent {
                return Err(CaptureError::SourcesLive);
            }
        }
        state.window = CaptureWindow {
            start: state.window.start.min(stop),
            end: stop,
            ..state.window
        };
        state.outcome = Some(
            if outcome == CaptureOutcome::Complete && state.forced_partial {
                CaptureOutcome::Partial
            } else {
                outcome
            },
        );
        if self.notification.is_none() {
            self.notification = Some(ticket);
        } else {
            self.notification_misses.increment();
        }
        Ok(())
    }
}

fn check_epoch<T: Copy>(slot: &TakeSlot<T>, epoch: StreamEpoch) -> Result<(), CaptureError> {
    if slot.state.is_some_and(|state| state.window.epoch == epoch) {
        Ok(())
    } else {
        Err(CaptureError::ForeignEpoch)
    }
}

fn source_mut<T: Copy>(
    slot: &mut TakeSlot<T>,
    generation: ConnectionGeneration,
) -> Result<&mut SourceAttribution, CaptureError> {
    for source in slot.sources.iter_mut().flatten() {
        if source.generation == generation {
            return Ok(source);
        }
    }
    Err(CaptureError::ForeignSource)
}
