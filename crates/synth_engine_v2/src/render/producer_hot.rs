//! Bounded current-quantum generation. No owning storage is created or destroyed here.
use super::{EventEnvelope, EventPayload, PreparedRenderer, TimedEvent};
#[cfg(feature = "mixed-transfer-rehearsal")]
use crate::time::SampleTime;
use crate::{
    diagnostics::RenderError,
    node::kernels::TimedControl,
    quantities::{EventCount, ParameterValue},
    time::{FrameCount, QUANTUM_FRAMES, TimeSource},
};
use crate::{
    publish::{ProducerClass, Publication},
    time::QuantumOffset,
};

const AUTHORED_STORAGE: RenderError = RenderError::ProducerStorage {
    class: ProducerClass::AuthoredRuntime,
};
const INTERNAL_STORAGE: RenderError = RenderError::ProducerStorage {
    class: ProducerClass::Internal,
};

impl PreparedRenderer {
    #[cfg(all(test, feature = "mixed-transfer-rehearsal"))]
    pub(crate) fn producer_runtime_for_test(
        &mut self,
    ) -> &mut crate::producer_programs::ProducerRuntime {
        &mut self.producer_runtime
    }
    pub(crate) fn publish_runtime_parameters(
        &mut self,
        publication: &mut Publication<'_>,
    ) -> Result<(), RenderError> {
        if self.plan.producer_programs.authored.is_empty() {
            return Ok(());
        }
        self.producer_runtime.authored_observed = true;
        self.producer_runtime.authored_counts.fill(EventCount::NONE);
        let initial = !self.producer_runtime.initialized;
        let suppressed = {
            #[cfg(test)]
            {
                self.producer_runtime.suppress_authored
            }
            #[cfg(not(test))]
            {
                false
            }
        };
        // Position first, then stable declaration order. Equal-phase entries of one
        // producer remain contiguous even when another producer targets that sample.
        for offset in 0..QUANTUM_FRAMES {
            let time = self
                .clock
                .checked_add(FrameCount::new(u64::from(offset)))
                .map_err(RenderError::ClockExhausted)?;
            for (program_index, program) in self.plan.producer_programs.authored.iter().enumerate()
            {
                let bound = program.declaration.destination_occupancy();
                #[cfg(test)]
                let bound = self
                    .producer_runtime
                    .authored_limit
                    .map_or(bound, |limit| limit.min(bound));
                let spent = self
                    .producer_runtime
                    .authored_counts
                    .get_mut(program_index)
                    .ok_or(AUTHORED_STORAGE)?;
                let phase = time.as_u64() % program.declaration.period().as_u64();
                if initial && offset == 0 {
                    for target_index in &program.targets {
                        let Some(slot) = program.slots.get(*target_index).copied() else {
                            return Err(AUTHORED_STORAGE);
                        };
                        let mut latest = None;
                        let mut previous = None;
                        for (index, entry) in program.declaration.entries().iter().enumerate() {
                            if program.slots.get(index) == Some(&slot) {
                                latest = Some(entry.value());
                                if entry.phase().as_u64() <= phase {
                                    previous = Some(entry.value());
                                }
                            }
                        }
                        let value = previous.or(latest).ok_or(AUTHORED_STORAGE)?;
                        if !suppressed {
                            charge_authored(
                                publication,
                                spent,
                                bound,
                                TimedEvent::new(
                                    EventEnvelope::new(self.epoch, time, TimeSource::Authored),
                                    EventPayload::SetParameter { slot, value },
                                ),
                            )?;
                        }
                    }
                } else {
                    for (index, entry) in program.declaration.entries().iter().enumerate() {
                        if entry.phase().as_u64() == phase && !suppressed {
                            let slot = program.slots.get(index).copied().ok_or(AUTHORED_STORAGE)?;
                            charge_authored(
                                publication,
                                spent,
                                bound,
                                TimedEvent::new(
                                    EventEnvelope::new(self.epoch, time, TimeSource::Authored),
                                    EventPayload::SetParameter {
                                        slot,
                                        value: entry.value(),
                                    },
                                ),
                            )?;
                        }
                    }
                }
            }
        }
        self.producer_runtime.initialized = true;
        Ok(())
    }
    pub(crate) fn set_producer_external_total(&mut self, total: EventCount) {
        self.producer_runtime.external = total;
    }

    pub(super) fn generate_internal_records(&mut self) -> Result<(), RenderError> {
        self.producer_runtime.length = 0;
        self.producer_runtime.last_time = Some(self.clock);
        self.producer_runtime.observed = true;
        self.producer_runtime.last_internal = EventCount::NONE;
        self.producer_runtime.combined_peak = self
            .producer_runtime
            .combined_peak
            .max(self.producer_runtime.external);
        let suppressed = {
            #[cfg(test)]
            {
                self.producer_runtime.suppress_internal
            }
            #[cfg(not(test))]
            {
                false
            }
        };
        let total_limit = self.plan.max_events_per_quantum();
        #[cfg(test)]
        let total_limit = self.producer_runtime.combined_limit.unwrap_or(total_limit);
        let admitted = self.plan.producer_programs.internal_bound;
        let limit = {
            #[cfg(test)]
            {
                self.producer_runtime.internal_limit.unwrap_or(admitted)
            }
            #[cfg(not(test))]
            {
                admitted
            }
        };
        for offset in 0..if self.plan.producer_programs.internal.is_empty() {
            0
        } else {
            QUANTUM_FRAMES
        } {
            let time = self
                .clock
                .checked_add(FrameCount::new(u64::from(offset)))
                .map_err(RenderError::ClockExhausted)?;
            for program in &self.plan.producer_programs.internal {
                if time.as_u64() % program.declaration.period().as_u64()
                    == program.declaration.phase().as_u64()
                    && !suppressed
                {
                    let next = self
                        .producer_runtime
                        .length
                        .checked_add(1)
                        .ok_or(INTERNAL_STORAGE)?;
                    let observed = u32::try_from(next).map_err(|_| INTERNAL_STORAGE)?;
                    if observed > limit.get() {
                        return Err(RenderError::ProducerBound {
                            class: ProducerClass::Internal,
                        });
                    }
                    let combined = observed
                        .checked_add(self.producer_runtime.external.get())
                        .ok_or(INTERNAL_STORAGE)?;
                    if combined > total_limit.get() {
                        return Err(RenderError::ProducerBound {
                            class: ProducerClass::Internal,
                        });
                    }
                    let record = self
                        .producer_runtime
                        .arena
                        .get_mut(self.producer_runtime.length)
                        .ok_or(INTERNAL_STORAGE)?;
                    *record = Some(crate::producer_programs::InternalEvent {
                        node: program.node,
                        offset: QuantumOffset::new(
                            u16::try_from(offset).map_err(|_| INTERNAL_STORAGE)?,
                        )
                        .map_err(|_| INTERNAL_STORAGE)?,
                    });
                    self.producer_runtime.length = next;
                    self.producer_runtime.last_internal = EventCount::measured(observed);
                    self.producer_runtime.internal_peak = self
                        .producer_runtime
                        .internal_peak
                        .max(EventCount::measured(observed));
                    self.producer_runtime.combined_peak = self
                        .producer_runtime
                        .combined_peak
                        .max(EventCount::measured(combined));
                }
            }
        }
        let internal = u32::try_from(self.producer_runtime.length).map_err(|_| INTERNAL_STORAGE)?;
        let combined = internal
            .checked_add(self.producer_runtime.external.get())
            .ok_or(INTERNAL_STORAGE)?;
        if combined > total_limit.get() {
            return Err(INTERNAL_STORAGE);
        }
        self.producer_runtime.last_internal = EventCount::measured(internal);
        self.producer_runtime.combined_peak =
            EventCount::measured(self.producer_runtime.combined_peak.get().max(combined));
        self.producer_runtime.observed = true;
        Ok(())
    }
    #[cfg(feature = "mixed-transfer-rehearsal")]
    pub(crate) fn runtime_authored_usage(&self) -> Option<EventCount> {
        (!self.plan.producer_programs.authored.is_empty()
            && self.producer_runtime.authored_observed)
            .then_some(EventCount::NONE)
    }
    #[cfg(feature = "mixed-transfer-rehearsal")]
    pub(crate) fn runtime_internal_usage(&self) -> Option<EventCount> {
        (!self.plan.producer_programs.internal.is_empty() && self.producer_runtime.observed)
            .then_some(self.producer_runtime.internal_peak)
    }
    #[cfg(feature = "mixed-transfer-rehearsal")]
    pub(crate) fn runtime_combined_usage(&self) -> Option<EventCount> {
        self.producer_runtime
            .observed
            .then_some(self.producer_runtime.combined_peak)
    }
    #[cfg(feature = "mixed-transfer-rehearsal")]
    pub(crate) fn runtime_internal_at(&self, time: SampleTime) -> Option<EventCount> {
        (!self.plan.producer_programs.internal.is_empty()
            && self.producer_runtime.last_time == Some(time))
        .then_some(self.producer_runtime.last_internal)
    }
    pub(super) fn count_internal_controls(&mut self) {
        for index in 0..self.producer_runtime.length {
            if let Some(Some(event)) = self.producer_runtime.arena.get(index)
                && let Some(count) = self
                    .control_starts
                    .get_mut(event.node.index().saturating_add(1))
            {
                *count = count.saturating_add(1);
            }
        }
    }
    pub(super) fn merge_internal_controls(&mut self) -> Result<(), RenderError> {
        // The separate immutable arena remains the sole cause of each clear. This
        // derived run is sorted by offset, preserving external-first equal ties.
        for index in 0..self.producer_runtime.length {
            let event = self
                .producer_runtime
                .arena
                .get(index)
                .copied()
                .flatten()
                .ok_or(INTERNAL_STORAGE)?;
            let node = event.node.index();
            let base = self
                .control_starts
                .get(node)
                .copied()
                .ok_or(INTERNAL_STORAGE)? as usize;
            let filled = self
                .control_fill
                .get(node)
                .copied()
                .ok_or(INTERNAL_STORAGE)? as usize;
            let end = base.checked_add(filled).ok_or(INTERNAL_STORAGE)?;
            let capacity_end = self
                .control_starts
                .get(node.saturating_add(1))
                .copied()
                .ok_or(INTERNAL_STORAGE)? as usize;
            if end >= capacity_end {
                return Err(INTERNAL_STORAGE);
            }
            let mut position = end;
            while position > base {
                let previous = self
                    .timed_controls
                    .get(position - 1)
                    .copied()
                    .ok_or(INTERNAL_STORAGE)?;
                if previous.offset <= event.offset {
                    break;
                }
                *self
                    .timed_controls
                    .get_mut(position)
                    .ok_or(INTERNAL_STORAGE)? = previous;
                position -= 1;
            }
            *self
                .timed_controls
                .get_mut(position)
                .ok_or(INTERNAL_STORAGE)? = TimedControl {
                offset: event.offset,
                control: crate::node::kernels::ControlIndex::RESET,
                value: ParameterValue::ONE,
            };
            let fill = self.control_fill.get_mut(node).ok_or(INTERNAL_STORAGE)?;
            *fill = fill.checked_add(1).ok_or(INTERNAL_STORAGE)?;
        }
        Ok(())
    }
}

fn charge_authored(
    publication: &mut Publication<'_>,
    spent: &mut EventCount,
    bound: EventCount,
    event: TimedEvent,
) -> Result<(), RenderError> {
    let next = spent.get().checked_add(1).ok_or(AUTHORED_STORAGE)?;
    if next > bound.get() {
        return Err(RenderError::ProducerBound {
            class: ProducerClass::AuthoredRuntime,
        });
    }
    publication
        .charge(ProducerClass::AuthoredRuntime, event)
        .map_err(RenderError::ProducerPublication)?;
    *spent = EventCount::measured(next);
    Ok(())
}
