//! Fixed input observations, capacity and retained dispositions.
use super::{InputTick, SimulatedInputClock};
use crate::{
    host::{ConnectionGeneration, session::SessionSourceOutcome},
    quantities::PreparedBytes,
    recording::notes::Midi1Input,
    time::SampleTime,
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum InputError {
    #[error("input generation is stale")]
    Stale,
    #[error("input lifecycle does not permit this operation")]
    State,
    #[error("input clock arithmetic is outside its declared domain")]
    ClockRange,
    #[error("input uncertainty spans more than one engine frame")]
    Uncertain,
    #[error("input time precedes an admitted frontier or regresses")]
    Order,
    #[error("input arrived before its nominal engine time")]
    Future,
    #[error("input receipt storage is full")]
    Full,
    #[error("input admission cannot preserve note-release and frontier reservations")]
    ProtectedCapacity,
    #[error("source ring retry was terminally retired while full")]
    SourceQueueFull,
    #[error("input identity space is exhausted")]
    IdentityExhausted,
    #[error("input requires at least two cells")]
    Capacity,
    #[error("input storage layout cannot be represented")]
    Layout,
    #[error("input storage requires {required:?}, available {available:?}")]
    ByteBudget {
        required: PreparedBytes,
        available: PreparedBytes,
    },
    #[error("input storage allocation failed")]
    Allocation,
    #[error("input preparation failed")]
    Preparation,
    #[error("input device was lost")]
    DeviceLost,
    #[error("a participating source interrupted this input")]
    PeerInterrupted,
    #[error("recording refused or stopped input delivery")]
    Delivery,
    #[error("recording receipt has no input owner")]
    ReceiptOwner,
    #[error("input attachment requires fresh matching owners and every armed source")]
    Attachment,
    #[error("input outcomes or source quiescence remain unresolved")]
    Retained,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct InputCapacity(u32);
impl InputCapacity {
    pub fn new(cells: u32) -> Result<Self, InputError> {
        if cells < 2 {
            return Err(InputError::Capacity);
        }
        Ok(Self(cells))
    }
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

/// Measured raw observation or release-reservation cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct InputCellCount(usize);
impl InputCellCount {
    pub(super) const fn measured(cells: usize) -> Self {
        Self(cells)
    }
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// Read-only pressure on one raw input owner's configured cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct InputPressure {
    occupied: InputCellCount,
    release_reservations: InputCellCount,
    capacity: InputCapacity,
}
impl InputPressure {
    pub(super) const fn new(
        occupied: InputCellCount,
        release_reservations: InputCellCount,
        capacity: InputCapacity,
    ) -> Self {
        Self {
            occupied,
            release_reservations,
            capacity,
        }
    }
    pub const fn occupied(self) -> InputCellCount {
        self.occupied
    }
    pub const fn release_reservations(self) -> InputCellCount {
        self.release_reservations
    }
    pub const fn capacity(self) -> InputCapacity {
        self.capacity
    }
}

#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct InputLimits {
    pub cells: InputCapacity,
    pub bytes: PreparedBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct InputEventId {
    pub(super) generation: ConnectionGeneration,
    pub(super) serial: u64,
}
impl InputEventId {
    pub const fn generation(self) -> ConnectionGeneration {
        self.generation
    }
    pub const fn serial(self) -> u64 {
        self.serial
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputObservation {
    Message {
        tick: InputTick,
        arrival: SampleTime,
        input: Midi1Input,
    },
    /// Explicit complete prefix in the synthetic source clock.
    Frontier { tick: InputTick },
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputDiscontinuity {
    pub reason: InputError,
    pub observation: Option<InputObservation>,
}
#[derive(Debug)]
pub enum InputOutcome {
    Delivered(SessionSourceOutcome),
    Refused(crate::host::session::LoopSessionError),
    Cancelled,
}
#[derive(Debug)]
#[must_use]
pub struct InputReceipt {
    pub audition: crate::recording::notes::AuditionTrace,
    pub id: InputEventId,
    /// The oldest held onset redeemed by this raw release at admission.
    /// This is independent of the eventual recording or audition outcome.
    pub matched_onset: Option<InputEventId>,
    pub observation: InputObservation,
    pub clock: SimulatedInputClock,
    pub outcome: InputOutcome,
}
