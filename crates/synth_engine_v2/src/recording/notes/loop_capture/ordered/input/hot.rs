//! Fixed-storage synthetic input callback operations.
use super::{
    InputDiscontinuity, InputEntry, InputError, InputEventId, InputObservation, InputOutcome,
    InputReceipt, InputTick, SimulatedNoteInput,
};
use crate::{
    host::{ConnectionGeneration, ConnectionState},
    recording::notes::Midi1Input,
    time::SampleTime,
};

impl SimulatedNoteInput {
    pub(super) fn check(&self, generation: ConnectionGeneration) -> Result<(), InputError> {
        if self.generation != Some(generation) {
            return Err(InputError::Stale);
        }
        Ok(())
    }
    pub fn start(&mut self, generation: ConnectionGeneration) -> Result<(), InputError> {
        self.check(generation)?;
        if self.state != ConnectionState::Ready {
            return Err(InputError::State);
        }
        self.state = ConnectionState::Running;
        self.quiescent = false;
        Ok(())
    }
    pub(super) fn fail(&mut self, reason: InputError, observation: Option<InputObservation>) {
        if self.discontinuity.is_none() {
            self.discontinuity = Some(InputDiscontinuity {
                reason,
                observation,
            });
            self.discontinuity_attributed = false;
        }
        self.state = ConnectionState::Quiescing;
        self.cancel_unsent();
    }
    pub(super) fn record_pre_ring_failure(
        &mut self,
        reason: InputError,
        observation: InputObservation,
    ) -> Result<(), InputError> {
        let fault = InputDiscontinuity {
            reason,
            observation: Some(observation),
        };
        if let Some(existing) = self.pre_ring_failure {
            return if existing == fault {
                Ok(())
            } else {
                Err(InputError::State)
            };
        }
        self.pre_ring_failure = Some(fault);
        self.pre_ring_attributed = false;
        self.fail(reason, Some(observation));
        Ok(())
    }
    pub fn device_lost(&mut self, generation: ConnectionGeneration) -> Result<(), InputError> {
        self.check(generation)?;
        if !matches!(
            self.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(InputError::State);
        }
        self.fail(InputError::DeviceLost, None);
        Ok(())
    }
    /// Exclusive simulated callback fence, not a physical backend or cross-thread fence.
    pub fn acknowledge_quiescence(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), InputError> {
        self.check(generation)?;
        if self.state != ConnectionState::Quiescing {
            return Err(InputError::State);
        }
        self.quiescent = true;
        Ok(())
    }
    pub fn offer_message(
        &mut self,
        generation: ConnectionGeneration,
        tick: InputTick,
        arrival: SampleTime,
        input: Midi1Input,
    ) -> Result<InputEventId, InputError> {
        self.offer_observation(
            generation,
            InputObservation::Message {
                tick,
                arrival,
                input,
            },
        )
    }
    pub fn advance_frontier(
        &mut self,
        generation: ConnectionGeneration,
        tick: InputTick,
    ) -> Result<InputEventId, InputError> {
        self.offer_observation(generation, InputObservation::Frontier { tick })
    }
    fn offer_observation(
        &mut self,
        generation: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<InputEventId, InputError> {
        self.check(generation)?;
        if self.state != ConnectionState::Running {
            return Err(InputError::State);
        }
        let result = self.admit(generation, observation);
        if let Err(error) = result {
            self.fail(error, Some(observation));
        }
        result
    }
    fn admit(
        &mut self,
        generation: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<InputEventId, InputError> {
        let clock = self.clock.ok_or(InputError::State)?;
        let (tick, arrival) = match observation {
            InputObservation::Message { tick, arrival, .. } => (tick, Some(arrival)),
            InputObservation::Frontier { tick } => (tick, None),
        };
        let nominal = clock.map(tick)?;
        if self.last_tick.is_some_and(|last| tick < last) || nominal < self.frontier {
            return Err(InputError::Order);
        }
        if let Some(arrival) = arrival {
            if arrival < nominal {
                return Err(InputError::Future);
            }
            if self.last_arrival.is_some_and(|last| arrival < last) {
                return Err(InputError::Order);
            }
        } else if nominal <= self.frontier || self.last_arrival.is_some_and(|last| nominal <= last)
        {
            // A frontier must follow every admitted arrival. Otherwise sorting fences
            // first could overtake an already accepted delayed message.
            return Err(InputError::Order);
        }
        let mut held = 0;
        let mut vacant = None;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.is_some() {
                held += 1;
            } else if vacant.is_none() {
                vacant = Some(index);
            }
        }
        let reserve = usize::from(arrival.is_some());
        if held >= self.slots.len() - reserve {
            return Err(InputError::Full);
        }
        let index = vacant.ok_or(InputError::Full)?;
        let serial = self
            .serial
            .checked_add(1)
            .ok_or(InputError::IdentityExhausted)?;
        let id = InputEventId { generation, serial };
        self.slots[index] = Some(InputEntry {
            audition: crate::recording::notes::AuditionTrace::NotOffered,
            id,
            observation,
            nominal,
            forwarded: None,
            outcome: None,
        });
        self.serial = serial;
        self.last_tick = Some(tick);
        if let Some(arrival) = arrival {
            self.last_arrival = Some(arrival);
        } else {
            self.frontier = nominal;
        }
        Ok(id)
    }
    pub(super) fn take_receipt(&mut self) -> Option<InputReceipt> {
        let clock = self.clock?;
        let mut selected: Option<(usize, u64)> = None;
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(entry) = slot
                && entry.outcome.is_some()
                && selected.is_none_or(|(_, serial)| entry.id.serial < serial)
            {
                selected = Some((index, entry.id.serial));
            }
        }
        let (index, _) = selected?;
        let entry = self.slots.get_mut(index)?.as_mut()?;
        let outcome = entry.outcome.take()?;
        let receipt = InputReceipt {
            audition: entry.audition,
            id: entry.id,
            observation: entry.observation,
            clock,
            outcome,
        };
        self.slots[index] = None;
        Some(receipt)
    }
    pub(super) fn cancel_unsent(&mut self) {
        for entry in self.slots.iter_mut().flatten() {
            if entry.forwarded.is_none() && entry.outcome.is_none() {
                entry.outcome = Some(InputOutcome::Cancelled);
            }
        }
    }
}
