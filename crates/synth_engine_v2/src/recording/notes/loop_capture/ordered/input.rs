//! Finite serial simulated input ownership and independent clock merging (ADR-0069).
mod capture;
mod clock;
mod hot;
#[cfg(test)]
mod tests;
mod types;
pub use capture::*;
pub use clock::*;
pub use types::*;

use crate::{
    host::{
        ConnectionGeneration, ConnectionState, EndpointId,
        session::{SessionSourceAction, SessionSourceId},
    },
    quantities::{KeyIdentity, PreparedBytes},
    recording::notes::{
        ControllerSnapshot,
        loop_capture::{LoopCaptureError, LoopCaptureSession},
    },
    time::SampleTime,
};
use synth_core::MidiChannel;

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputDelivery {
    Serial(SessionSourceId),
    Transfer(super::transfer::LoopTransferId),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceFaultStage {
    BeforeSourceRing,
    AfterSourceRing,
}

struct InputEntry {
    audition: crate::recording::notes::AuditionTrace,
    id: InputEventId,
    matched_onset: Option<InputEventId>,
    observation: InputObservation,
    nominal: SampleTime,
    forwarded: Option<InputDelivery>,
    outcome: Option<InputOutcome>,
}

#[derive(Clone, Copy)]
struct RawHeldOnset {
    id: InputEventId,
    channel: MidiChannel,
    key: KeyIdentity,
}

/// One independently generated connection. Endpoint settings are off-thread;
/// `bytes` charges this inline owner, every observation/receipt cell and the held-key ledger.
#[must_use]
pub struct SimulatedNoteInput {
    endpoint: EndpointId,
    generation: Option<ConnectionGeneration>,
    state: ConnectionState,
    clock: Option<SimulatedInputClock>,
    binding: Option<ConnectionGeneration>,
    capacity: InputCapacity,
    slots: Box<[Option<InputEntry>]>,
    held_onsets: Box<[Option<RawHeldOnset>]>,
    release_reservations: usize,
    serial: u64,
    last_tick: Option<InputTick>,
    last_arrival: Option<SampleTime>,
    frontier: SampleTime,
    discontinuity: Option<InputDiscontinuity>,
    discontinuity_source_stage: Option<SourceFaultStage>,
    pre_ring_failure: Option<InputDiscontinuity>,
    post_source_ring_failure: Option<InputDiscontinuity>,
    discontinuity_attributed: bool,
    pre_ring_attributed: bool,
    post_source_ring_attributed: bool,
    quiescent: bool,
    bytes: PreparedBytes,
}
impl SimulatedNoteInput {
    pub fn new(endpoint: EndpointId, limits: InputLimits) -> Result<Self, InputError> {
        let count = usize::try_from(limits.cells.as_u32()).map_err(|_| InputError::Layout)?;
        let bytes = count
            .checked_mul(size_of::<Option<InputEntry>>())
            .and_then(|bytes| {
                count
                    .checked_mul(size_of::<Option<RawHeldOnset>>())
                    .and_then(|holds| bytes.checked_add(holds))
            })
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(InputError::Layout)?;
        let required = PreparedBytes::measured(bytes);
        if required > limits.bytes {
            return Err(InputError::ByteBudget {
                required,
                available: limits.bytes,
            });
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| InputError::Allocation)?;
        slots.resize_with(count, || None);
        let mut held_onsets = Vec::new();
        held_onsets
            .try_reserve_exact(count)
            .map_err(|_| InputError::Allocation)?;
        held_onsets.resize_with(count, || None);
        Ok(Self {
            endpoint,
            generation: None,
            state: ConnectionState::Stopped,
            clock: None,
            binding: None,
            capacity: limits.cells,
            slots: slots.into_boxed_slice(),
            held_onsets: held_onsets.into_boxed_slice(),
            release_reservations: 0,
            serial: 0,
            last_tick: None,
            last_arrival: None,
            frontier: SampleTime::ZERO,
            discontinuity: None,
            discontinuity_source_stage: None,
            pre_ring_failure: None,
            post_source_ring_failure: None,
            discontinuity_attributed: false,
            pre_ring_attributed: false,
            post_source_ring_attributed: false,
            quiescent: true,
            bytes: required,
        })
    }
    pub fn collect(&mut self) -> Option<InputReceipt> {
        self.take_receipt()
    }

    pub fn endpoint(&self) -> &EndpointId {
        &self.endpoint
    }
    pub const fn generation(&self) -> Option<ConnectionGeneration> {
        self.generation
    }
    pub const fn state(&self) -> ConnectionState {
        self.state
    }
    pub const fn clock(&self) -> Option<SimulatedInputClock> {
        self.clock
    }
    pub const fn source(&self) -> Option<ConnectionGeneration> {
        self.binding
    }
    pub const fn discontinuity(&self) -> Option<InputDiscontinuity> {
        self.discontinuity
    }
    pub const fn pre_ring_failure(&self) -> Option<InputDiscontinuity> {
        self.pre_ring_failure
    }
    pub const fn post_source_ring_failure(&self) -> Option<InputDiscontinuity> {
        self.post_source_ring_failure
    }
    pub const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }

    /// Current raw-cell occupancy and matched-release reservations. This
    /// off-thread snapshot does not reserve capacity for a later source offer.
    pub fn pressure(&self) -> InputPressure {
        InputPressure::new(
            InputCellCount::measured(self.slots.iter().filter(|slot| slot.is_some()).count()),
            InputCellCount::measured(self.release_reservations),
            self.capacity,
        )
    }

    /// An explicit same-endpoint attempt. Retirement must precede another attempt.
    pub fn begin(&mut self) -> Result<ConnectionGeneration, InputError> {
        if !matches!(
            self.state,
            ConnectionState::Stopped | ConnectionState::Unavailable
        ) || !self.quiescent
            || self.slots.iter().any(Option::is_some)
            || self.release_reservations != 0
            || self.binding.is_some()
        {
            return Err(InputError::State);
        }
        let generation = crate::host::issue_capture_source_generation()
            .map_err(|_| InputError::IdentityExhausted)?;
        self.generation = Some(generation);
        self.state = ConnectionState::Preparing;
        self.clock = None;
        self.serial = 0;
        self.release_reservations = 0;
        for hold in &mut self.held_onsets {
            *hold = None;
        }
        self.last_tick = None;
        self.last_arrival = None;
        self.frontier = SampleTime::ZERO;
        self.discontinuity = None;
        self.discontinuity_source_stage = None;
        self.pre_ring_failure = None;
        self.post_source_ring_failure = None;
        self.discontinuity_attributed = false;
        self.pre_ring_attributed = false;
        self.post_source_ring_attributed = false;
        Ok(generation)
    }
    pub fn prepare(
        &mut self,
        generation: ConnectionGeneration,
        clock: SimulatedInputClock,
    ) -> Result<(), InputError> {
        self.check(generation)?;
        if self.state != ConnectionState::Preparing || clock.origin != SampleTime::ZERO {
            return Err(InputError::State);
        }
        // Initial synchronization is a declared exact prefix at the oracle anchor,
        // not a timestamp observation subject to message uncertainty.
        self.clock = Some(clock);
        self.slots[0] = Some(InputEntry {
            audition: crate::recording::notes::AuditionTrace::NotOffered,
            id: InputEventId {
                generation,
                serial: 1,
            },
            matched_onset: None,
            observation: InputObservation::Frontier {
                tick: clock.tick_origin,
            },
            nominal: SampleTime::ZERO,
            forwarded: None,
            outcome: None,
        });
        self.serial = 1;
        self.quiescent = false;
        self.state = ConnectionState::Ready;
        Ok(())
    }
    pub fn fail_preparation(&mut self, generation: ConnectionGeneration) -> Result<(), InputError> {
        self.check(generation)?;
        if self.state != ConnectionState::Preparing {
            return Err(InputError::State);
        }
        self.discontinuity = Some(InputDiscontinuity {
            reason: InputError::Preparation,
            observation: None,
        });
        self.state = ConnectionState::Unavailable;
        Ok(())
    }
    /// Bind once, before arm. The recording source identity differs from the input
    /// connection and is minted by the recording owner, never supplied by callers.
    pub fn bind_capture(
        &mut self,
        generation: ConnectionGeneration,
        capture: &mut LoopCaptureSession,
        controllers: ControllerSnapshot,
    ) -> Result<ConnectionGeneration, InputBindError> {
        self.check(generation)?;
        if self.state != ConnectionState::Ready
            || self.binding.is_some()
            || self
                .clock
                .is_none_or(|clock| clock.epoch != capture.initial().epoch)
        {
            return Err(InputError::Attachment.into());
        }
        let source = capture.bind_source(controllers)?;
        self.binding = Some(source);
        Ok(source)
    }

    /// Only a released, quiescent input with every outcome collected can retire.
    /// The caller retains the old recording separately before reconnecting.
    pub fn retire(&mut self, generation: ConnectionGeneration) -> Result<(), InputError> {
        self.check(generation)?;
        if self.state != ConnectionState::Quiescing
            || !self.quiescent
            || self.slots.iter().any(Option::is_some)
            || self.release_reservations != 0
        {
            return Err(InputError::Retained);
        }
        self.binding = None;
        self.state = ConnectionState::Unavailable;
        Ok(())
    }

    fn action(&self, entry: &InputEntry) -> Option<SessionSourceAction> {
        let source = self.binding?;
        let clock = self.clock?;
        Some(match entry.observation {
            InputObservation::Frontier { .. } => SessionSourceAction::Fence {
                source,
                epoch: clock.epoch,
                frontier: entry.nominal,
            },
            InputObservation::Message { arrival, input, .. } => SessionSourceAction::Publish {
                source,
                stamp: crate::recording::notes::CaptureStamp::exact_fixture(
                    clock.epoch,
                    entry.nominal,
                    arrival,
                )
                .ok()?,
                input,
                audition: entry.audition,
            },
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InputBindError {
    #[error(transparent)]
    Input(#[from] InputError),
    #[error(transparent)]
    Capture(#[from] LoopCaptureError),
}
