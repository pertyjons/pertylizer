//! Fixed-storage synthetic input callback operations.
use super::{
    InputDiscontinuity, InputEntry, InputError, InputEventId, InputObservation, InputOutcome,
    InputReceipt, InputTick, RawHeldOnset, SimulatedNoteInput, SourceFaultStage,
};
use crate::{
    host::{ConnectionGeneration, ConnectionState},
    quantities::KeyIdentity,
    recording::notes::{Midi1Event, Midi1Input},
    time::SampleTime,
};
use synth_core::MidiChannel;

#[derive(Clone, Copy)]
enum ProtectedClass {
    Onset {
        hold_index: usize,
        channel: MidiChannel,
        key: KeyIdentity,
    },
    MatchedRelease {
        hold_index: usize,
        onset_id: InputEventId,
    },
    Ordinary,
    Frontier,
}

struct AdmitPlan {
    class: ProtectedClass,
    index: usize,
    id: InputEventId,
    observation: InputObservation,
    nominal: SampleTime,
    tick: InputTick,
    arrival: Option<SampleTime>,
}

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
        self.quiesce();
    }
    pub(super) fn quiesce(&mut self) {
        self.state = ConnectionState::Quiescing;
        self.cancel_unsent();
        // Once admission closes, accepted notes and their final disposition
        // belong to the retained take; no later release can use these claims.
        self.release_reservations = 0;
        for hold in &mut self.held_onsets {
            *hold = None;
        }
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
        let first = self.discontinuity.is_none();
        self.fail(reason, Some(observation));
        if first {
            self.discontinuity_source_stage = Some(SourceFaultStage::BeforeSourceRing);
        }
        Ok(())
    }
    pub(super) fn record_post_source_ring_failure(
        &mut self,
        reason: InputError,
        observation: InputObservation,
    ) -> Result<(), InputError> {
        if self.post_source_ring_failure.is_some() {
            return Err(InputError::State);
        }
        self.post_source_ring_failure = Some(InputDiscontinuity {
            reason,
            observation: Some(observation),
        });
        self.post_source_ring_attributed = false;
        let first = self.discontinuity.is_none();
        self.fail(reason, Some(observation));
        if first {
            self.discontinuity_source_stage = Some(SourceFaultStage::AfterSourceRing);
        }
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
    /// Read-only local admission check. The answer is current only until this
    /// input changes; it reserves no raw, source-ring or recorder credit.
    pub fn preflight_observation(
        &self,
        generation: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<(), InputError> {
        self.check(generation)?;
        if self.state != ConnectionState::Running {
            return Err(InputError::State);
        }
        self.plan(generation, observation).map(|_| ())
    }
    fn admit(
        &mut self,
        generation: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<InputEventId, InputError> {
        let plan = self.plan(generation, observation)?;
        let id = plan.id;
        self.apply(plan);
        Ok(id)
    }
    fn plan(
        &self,
        generation: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<AdmitPlan, InputError> {
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
        let mut held: usize = 0;
        let mut vacant = None;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.is_some() {
                held += 1;
            } else if vacant.is_none() {
                vacant = Some(index);
            }
        }
        let class = match observation {
            InputObservation::Frontier { .. } => ProtectedClass::Frontier,
            InputObservation::Message { input, .. } => match input.event() {
                Midi1Event::NoteOn { key, .. } => ProtectedClass::Onset {
                    hold_index: self
                        .held_onsets
                        .iter()
                        .position(Option::is_none)
                        .ok_or(InputError::ProtectedCapacity)?,
                    channel: input.channel(),
                    key,
                },
                Midi1Event::KeyRelease { key, .. } => {
                    let mut selected: Option<(usize, InputEventId)> = None;
                    for (index, hold) in self.held_onsets.iter().enumerate() {
                        if let Some(hold) = hold
                            && hold.channel == input.channel()
                            && hold.key == key
                            && selected.is_none_or(|(_, id)| hold.id.serial < id.serial)
                        {
                            selected = Some((index, hold.id));
                        }
                    }
                    selected.map_or(ProtectedClass::Ordinary, |(hold_index, onset_id)| {
                        ProtectedClass::MatchedRelease {
                            hold_index,
                            onset_id,
                        }
                    })
                }
                Midi1Event::Sustain { .. } | Midi1Event::PitchBend { .. } => {
                    ProtectedClass::Ordinary
                }
            },
        };
        let total = held
            .checked_add(self.release_reservations)
            .ok_or(InputError::ProtectedCapacity)?;
        let cells = self.slots.len();
        let protected_room = match class {
            ProtectedClass::Onset { .. } => {
                cells.checked_sub(4).is_some_and(|limit| total <= limit)
            }
            ProtectedClass::Ordinary if self.release_reservations > 0 => {
                cells.checked_sub(3).is_some_and(|limit| total <= limit)
            }
            ProtectedClass::Ordinary => cells.checked_sub(2).is_some_and(|limit| held <= limit),
            ProtectedClass::Frontier if self.release_reservations > 0 => {
                cells.checked_sub(2).is_some_and(|limit| total <= limit)
            }
            ProtectedClass::Frontier => cells.checked_sub(1).is_some_and(|limit| held <= limit),
            ProtectedClass::MatchedRelease { .. } => {
                cells.checked_sub(2).is_some_and(|limit| held <= limit)
            }
        };
        if !protected_room {
            return Err(match class {
                ProtectedClass::Ordinary | ProtectedClass::Frontier
                    if self.release_reservations == 0 =>
                {
                    InputError::Full
                }
                _ => InputError::ProtectedCapacity,
            });
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
        Ok(AdmitPlan {
            class,
            index,
            id,
            observation,
            nominal,
            tick,
            arrival,
        })
    }
    fn apply(&mut self, plan: AdmitPlan) {
        let AdmitPlan {
            class,
            index,
            id,
            observation,
            nominal,
            tick,
            arrival,
        } = plan;
        self.slots[index] = Some(InputEntry {
            audition: crate::recording::notes::AuditionTrace::NotOffered,
            id,
            matched_onset: match class {
                ProtectedClass::MatchedRelease { onset_id, .. } => Some(onset_id),
                _ => None,
            },
            observation,
            nominal,
            forwarded: None,
            outcome: None,
        });
        match class {
            ProtectedClass::Onset {
                hold_index,
                channel,
                key,
            } => {
                self.held_onsets[hold_index] = Some(RawHeldOnset { id, channel, key });
                self.release_reservations += 1;
            }
            ProtectedClass::MatchedRelease { hold_index, .. } => {
                self.held_onsets[hold_index] = None;
                self.release_reservations -= 1;
            }
            ProtectedClass::Ordinary | ProtectedClass::Frontier => {}
        }
        self.serial = id.serial;
        self.last_tick = Some(tick);
        if let Some(arrival) = arrival {
            self.last_arrival = Some(arrival);
        } else {
            self.frontier = nominal;
        }
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
            matched_onset: entry.matched_onset,
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
