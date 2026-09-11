//! Ordered, serialized capture boundaries (P09-S006).
//!
//! The fixture driver dispatches explicit engine-epoch boundaries before publishing
//! source input at the same sample. This lane controls capture only: it is not an
//! audible transport, a metronome, a renderer panic or a concurrent command queue.
//! Arm remains off-thread preparation and reserves the result before any start offer.

mod hot;
#[cfg(test)]
mod tests;

use super::{
    CaptureSessionId, CaptureStopReason, NoteCaptureError, SampleTime, SimulatedNoteRecorder,
    TakeReservation, add_bytes, array_bytes, check_bytes, empty_slots,
};
use crate::quantities::PreparedBytes;
use thiserror::Error;

/// Queue slots, including one reserved for an ending boundary.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureCommandCapacity(u32);
impl CaptureCommandCapacity {
    pub fn new(slots: u32) -> Result<Self, SessionError> {
        if slots < 2 {
            return Err(SessionError::Capacity);
        }
        Ok(Self(slots))
    }
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

/// An identity belongs to one recorder, even after its queue has been drained.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureCommandId {
    session: CaptureSessionId,
    serial: u64,
}
impl CaptureCommandId {
    pub const fn session(self) -> CaptureSessionId {
        self.session
    }
    pub const fn serial(self) -> u64 {
        self.serial
    }
}

/// An ending command records intent; it does not perform the renderer operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureEnd {
    Stop,
    Disarm,
    Panic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureCommand {
    Start(TakeReservation),
    End(TakeReservation, CaptureEnd),
}

#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureBoundary {
    pub id: CaptureCommandId,
    pub at: SampleTime,
    pub command: CaptureCommand,
}

#[derive(Debug, PartialEq)]
pub enum BoundaryOutcome {
    /// The boundary has been applied; ending may still await source fences to seal.
    Applied,
    /// No source has been consumed past the boundary; the same command remains first.
    WaitingForSources,
    /// This command was consumed without applying its requested transition.
    Refused(NoteCaptureError),
    /// Host interruption closed admission. The original command is returned unchanged.
    Cancelled,
}

#[must_use]
#[derive(Debug, PartialEq)]
pub struct BoundaryReceipt {
    pub boundary: CaptureBoundary,
    pub outcome: BoundaryOutcome,
}

pub(super) struct SessionLane {
    slots: Box<[Option<CaptureBoundary>]>,
    head: usize,
    len: usize,
    serial: u64,
    last_offer: Option<SampleTime>,
    dispatched: Option<SampleTime>,
}

impl SimulatedNoteRecorder {
    /// Enable once, before arming. Storage and descriptor are included in the existing
    /// recording byte ceiling. Failure changes neither the mode nor its charge.
    pub fn enable_ordered_session(
        &mut self,
        capacity: CaptureCommandCapacity,
    ) -> Result<(), SessionError> {
        if self.session_lane.is_some() {
            return Err(SessionError::AlreadyEnabled);
        }
        if self.host_interrupted {
            return Err(SessionError::Interrupted);
        }
        if self.is_active() {
            return Err(NoteCaptureError::AlreadyArmed.into());
        }
        let count = usize::try_from(capacity.0).map_err(|_| SessionError::Capacity)?;
        let base = add_bytes(
            self.base_bytes.get(),
            array_bytes::<Option<CaptureBoundary>>(count)?,
        )?;
        check_bytes(add_bytes(base, self.map_bytes.get())?, self.limits)?;
        let slots = empty_slots(count)?;
        self.session_lane = Some(SessionLane {
            slots,
            head: 0,
            len: 0,
            serial: 0,
            last_offer: None,
            dispatched: None,
        });
        self.base_bytes = PreparedBytes::measured(base);
        Ok(())
    }
}

#[derive(Debug, PartialEq, Error)]
pub enum SessionError {
    #[error("capture command capacity must fit the platform and contain at least two slots")]
    Capacity,
    #[error("ordered capture session is already enabled")]
    AlreadyEnabled,
    #[error("ordered capture session is not enabled")]
    NotEnabled,
    #[error("host interruption closed capture command admission")]
    Interrupted,
    #[error("capture boundary belongs to another epoch")]
    ForeignEpoch,
    #[error("capture command queue is full; accepted commands were retained")]
    Full,
    #[error("capture command identity exhausted")]
    IdentityExhausted,
    #[error("capture commands must be offered in time order before same-sample source input")]
    PastBoundary,
    #[error("capture start must name its reserved interval's start")]
    StartTime,
    #[error(transparent)]
    Capture(#[from] NoteCaptureError),
}
