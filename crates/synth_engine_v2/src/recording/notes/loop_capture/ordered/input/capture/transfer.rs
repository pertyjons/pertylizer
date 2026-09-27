//! Merger ownership around finite core transfers; concrete queues belong to the host.
mod hot;
mod recovery;
pub use recovery::*;

use super::super::{
    InputDelivery, InputError, InputEventId, InputObservation, InputOutcome, InputReceipt,
    SimulatedNoteInput,
};
use super::{InputCaptureError, InputCaptureSession, InputPortOrder, next_source};
use crate::{
    host::{
        ConnectionGeneration, ConnectionState,
        session::{
            SessionCommand, SessionError, SessionSourceOutcome,
            loop_transfer::{
                LoopRecordingAudio, LoopRecordingControl, LoopTransferCompletion,
                LoopTransferError, LoopTransferId, LoopTransferOutcome, LoopTransferPacket,
            },
        },
    },
    quantities::PreparedBytes,
    recording::notes::{CaptureDisposition, PublicationReceipt},
    time::SampleTime,
};
use std::sync::{Arc, atomic::AtomicU8};

/// A terminal signal for this split only. Clone and retire handles off-callback.
/// Stop interrupts at the acknowledged frontier; it is not an ordered Stop receipt.
#[derive(Clone)]
#[must_use]
pub struct InputCaptureHalt {
    signal: Arc<AtomicU8>,
}

#[must_use]
pub struct InputCaptureControl {
    core: LoopRecordingControl,
    inputs: Box<[SimulatedNoteInput]>,
    halt: InputCaptureHalt,
    closed: bool,
    serial_bytes: PreparedBytes,
    bytes: PreparedBytes,
}

/// The complete renderer/recorder stays here until callback access has joined.
#[must_use]
pub struct InputCaptureAudio {
    core: LoopRecordingAudio,
    halt: InputCaptureHalt,
}

pub type InputTransferCollection = Result<
    Option<(LoopTransferId, LoopTransferOutcome)>,
    Box<(LoopTransferCompletion, LoopTransferError)>,
>;

impl InputCaptureControl {
    pub const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }

    pub fn input(&self, generation: ConnectionGeneration) -> Option<&SimulatedNoteInput> {
        self.inputs
            .iter()
            .find(|input| input.generation == Some(generation))
    }

    fn port(&self, generation: ConnectionGeneration) -> Result<usize, InputError> {
        self.inputs
            .iter()
            .position(|input| input.generation == Some(generation))
            .ok_or(InputError::Stale)
    }

    pub fn start_input(&mut self, generation: ConnectionGeneration) -> Result<(), InputError> {
        let port = self.port(generation)?;
        self.synchronize_halt();
        self.inputs[port].start(generation)
    }

    /// Queue admission is not model admission. A refusal returns the original observation.
    pub fn offer_observation(
        &mut self,
        generation: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<InputEventId, (InputObservation, InputError)> {
        let port = self
            .port(generation)
            .map_err(|error| (observation, error))?;
        self.synchronize_halt();
        let result = match observation {
            InputObservation::Message {
                tick,
                arrival,
                input,
            } => self.inputs[port].offer_message(generation, tick, arrival, input),
            InputObservation::Frontier { tick } => {
                self.inputs[port].advance_frontier(generation, tick)
            }
        };
        if self.inputs[port].discontinuity.is_some() {
            self.halt.request_invalid();
            self.synchronize_halt();
        }
        result.map_err(|error| (observation, error))
    }

    /// Validate and retain a producer's terminal refusal whose original never
    /// entered the source ring. Reunion attributes accepted late or uncertain claims;
    /// source order, a retired full ring, and source-serial exhaustion are attestations
    /// when the queued prefix has not reached raw input. The pre-ring slot separates
    /// source exhaustion from raw input-ID exhaustion, but not the source attempt
    /// serial from the source queue serial.
    pub fn record_pre_ring_failure(
        &mut self,
        generation: ConnectionGeneration,
        observation: InputObservation,
        reason: InputError,
    ) -> Result<(), InputError> {
        let port = self.port(generation)?;
        let input = &mut self.inputs[port];
        if !matches!(
            input.state(),
            ConnectionState::Running | ConnectionState::Quiescing
        ) {
            return Err(InputError::State);
        }
        let clock = input.clock().ok_or(InputError::State)?;
        let (tick, arrival) = match observation {
            InputObservation::Message { tick, arrival, .. } => (tick, Some(arrival)),
            InputObservation::Frontier { tick } => (tick, None),
        };
        let mapping = clock.map(tick);
        let valid_reason = match reason {
            InputError::ClockRange | InputError::Uncertain => mapping == Err(reason),
            InputError::Future => {
                matches!((mapping, arrival), (Ok(nominal), Some(at)) if at < nominal)
            }
            InputError::Order => mapping.is_ok(),
            InputError::SourceQueueFull | InputError::IdentityExhausted => match (mapping, arrival)
            {
                (Ok(nominal), Some(at)) => at >= nominal,
                (Ok(_), None) => true,
                _ => false,
            },
            _ => false,
        };
        if !valid_reason {
            return Err(InputError::State);
        }
        input.record_pre_ring_failure(reason, observation)?;
        self.halt.request_invalid();
        Ok(())
    }

    /// Retain a terminal audition preflight refusal for a source-queued message
    /// that never acquired a raw input ID. This is separate from pre-source-ring
    /// failures because a later full-ring retirement can have its own original.
    pub fn record_post_source_ring_failure(
        &mut self,
        generation: ConnectionGeneration,
        observation: InputObservation,
        reason: InputError,
    ) -> Result<(), InputError> {
        let port = self.port(generation)?;
        self.synchronize_halt();
        let input = &mut self.inputs[port];
        if !matches!(
            input.state(),
            ConnectionState::Running | ConnectionState::Quiescing
        ) || !matches!(reason, InputError::Full | InputError::IdentityExhausted)
        {
            return Err(InputError::State);
        }
        let InputObservation::Message { tick, arrival, .. } = observation else {
            return Err(InputError::State);
        };
        let clock = input.clock().ok_or(InputError::State)?;
        let nominal = clock.map(tick).map_err(|_| InputError::State)?;
        if arrival < nominal {
            return Err(InputError::State);
        }
        input.record_post_source_ring_failure(reason, observation)?;
        self.halt.request_invalid();
        Ok(())
    }

    /// Attach a retained live-audition token before this observation is forwarded.
    pub fn set_audition(
        &mut self,
        id: InputEventId,
        trace: crate::recording::notes::AuditionTrace,
    ) -> Result<(), InputError> {
        let port = self.port(id.generation())?;
        let entry = self.inputs[port]
            .slots
            .iter_mut()
            .flatten()
            .find(|entry| entry.id == id)
            .ok_or(InputError::ReceiptOwner)?;
        if entry.forwarded.is_some() || entry.outcome.is_some() {
            return Err(InputError::ReceiptOwner);
        }
        entry.audition = trace;
        Ok(())
    }

    pub fn device_lost(&mut self, generation: ConnectionGeneration) -> Result<(), InputError> {
        let port = self.port(generation)?;
        self.inputs[port].device_lost(generation)?;
        self.halt.request_device_lost();
        self.synchronize_halt();
        Ok(())
    }

    fn synchronize_halt(&mut self) {
        if self.halt.reason().is_some() {
            self.closed = true;
            self.core.close_admission();
            for input in &mut self.inputs {
                if input.discontinuity.is_none() {
                    input.fail(InputError::PeerInterrupted, None);
                } else {
                    input.quiesce();
                }
            }
        }
    }

    pub fn prepare_command(
        &mut self,
        at: SampleTime,
        command: SessionCommand,
    ) -> Result<LoopTransferPacket, InputCaptureError> {
        self.synchronize_halt();
        Ok(self.core.prepare_command(at, command)?)
    }

    /// Prepare one eligible packet. The host retains a failed send and preserves FIFO.
    pub fn next_packet(&mut self) -> Result<Option<LoopTransferPacket>, InputCaptureError> {
        self.synchronize_halt();
        if self.closed {
            return Ok(None);
        }
        let Some((InputPortOrder(port), slot, action)) = next_source(&self.inputs)? else {
            return Ok(None);
        };
        let entry = self.inputs[port].slots[slot]
            .as_mut()
            .ok_or(InputError::ReceiptOwner)?;
        match self.core.prepare_source(action) {
            Ok(packet) => {
                entry.forwarded = Some(InputDelivery::Transfer(packet.id()));
                Ok(Some(packet))
            }
            Err(crate::host::session::LoopSessionError::Session(SessionError::Full)) => Ok(None),
            Err(error) => {
                if let Some(entry) = self.inputs[port].slots[slot].as_mut() {
                    let observation = entry.observation;
                    entry.outcome = Some(InputOutcome::Refused(error));
                    self.inputs[port].fail(InputError::Delivery, Some(observation));
                }
                self.halt.request_invalid();
                self.synchronize_halt();
                Err(InputError::Delivery.into())
            }
        }
    }

    /// Correlate an outstanding source packet with its retaining input cell.
    /// Returns `None` for commands or packets without a matching retained input
    /// in this control owner, including packets belonging to another owner.
    /// Preparation is not delivery: a host may report a queued source frontier
    /// only after this packet and every preceding FIFO packet were sent successfully.
    pub fn packet_input_id(&self, packet: &LoopTransferPacket) -> Option<InputEventId> {
        let (port, slot) = self.target(packet.id())?;
        Some(self.inputs.get(port)?.slots.get(slot)?.as_ref()?.id)
    }

    fn target(&self, id: LoopTransferId) -> Option<(usize, usize)> {
        for (port, input) in self.inputs.iter().enumerate() {
            for (slot, entry) in input.slots.iter().enumerate() {
                if entry
                    .as_ref()
                    .is_some_and(|entry| entry.forwarded == Some(InputDelivery::Transfer(id)))
                {
                    return Some((port, slot));
                }
            }
        }
        None
    }

    pub fn cancel(
        &mut self,
        packet: LoopTransferPacket,
    ) -> Result<LoopTransferId, (LoopTransferPacket, LoopTransferError)> {
        let target = self.target(packet.id());
        if packet.is_source() && target.is_none() {
            return Err((packet, LoopTransferError::Unknown));
        }
        let id = self.core.cancel(packet)?;
        if let Some((port, slot)) = target
            && let Some(entry) = self.inputs[port].slots[slot].as_mut()
        {
            entry.forwarded = None;
            let observation = entry.observation;
            entry.outcome = Some(InputOutcome::Cancelled);
            self.inputs[port].fail(InputError::Delivery, Some(observation));
            self.halt.request_invalid();
        }
        self.synchronize_halt();
        Ok(id)
    }

    /// Source outcomes stay in their input cells. Command outcomes return to the caller.
    pub fn collect(&mut self, completion: LoopTransferCompletion) -> InputTransferCollection {
        let target = self.target(completion.id());
        if completion.is_source() && target.is_none() {
            return Err(Box::new((completion, LoopTransferError::Unknown)));
        }
        let (id, outcome) = self.core.collect(completion)?;
        let Some((port, slot)) = target else {
            return Ok(Some((id, outcome)));
        };
        let failed = matches!(
            &outcome,
            LoopTransferOutcome::Refused(_)
                | LoopTransferOutcome::Source(crate::host::session::SessionSourceReceipt {
                    outcome: SessionSourceOutcome::Refused(_)
                        | SessionSourceOutcome::Published(PublicationReceipt {
                            capture: CaptureDisposition::Late | CaptureDisposition::Stopped(_),
                            ..
                        }),
                    ..
                })
        );
        let outcome = match outcome {
            LoopTransferOutcome::Source(receipt) => InputOutcome::Delivered(receipt.outcome),
            LoopTransferOutcome::Refused(error) => InputOutcome::Refused(error),
            // Core action custody guarantees source packets cannot carry command outcomes.
            LoopTransferOutcome::Command(receipt) => {
                return Ok(Some((id, LoopTransferOutcome::Command(receipt))));
            }
        };
        if let Some(entry) = self.inputs[port].slots[slot].as_mut() {
            let observation = entry.observation;
            entry.forwarded = None;
            entry.outcome = Some(outcome);
            if failed {
                self.inputs[port].fail(InputError::Delivery, Some(observation));
            }
        }
        if failed {
            self.halt.request_invalid();
        }
        self.synchronize_halt();
        Ok(None)
    }

    pub fn collect_input(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<Option<InputReceipt>, InputError> {
        let port = self.port(generation)?;
        self.synchronize_halt();
        Ok(self.inputs[port].take_receipt())
    }
}
