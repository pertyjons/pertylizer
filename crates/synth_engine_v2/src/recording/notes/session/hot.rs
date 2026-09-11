//! Fixed-storage command admission and dispatch, before the serial source publisher.

use super::{
    BoundaryOutcome, BoundaryReceipt, CaptureBoundary, CaptureCommand, CaptureCommandId,
    CaptureEnd, CaptureStopReason, SessionError, SessionLane,
};
use crate::recording::notes::{NoteCaptureError, SimulatedNoteRecorder};
use crate::time::{SampleTime, StreamEpoch};

impl CaptureEnd {
    const fn reason(self) -> CaptureStopReason {
        match self {
            Self::Stop => CaptureStopReason::Stop,
            Self::Disarm => CaptureStopReason::Disarm,
            Self::Panic => CaptureStopReason::Panic,
        }
    }
}

impl SessionLane {
    fn first(&self) -> Option<CaptureBoundary> {
        self.slots.get(self.head).copied().flatten()
    }

    fn pop(&mut self) {
        if let Some(slot) = self.slots.get_mut(self.head) {
            *slot = None;
        }
        // Construction admits at least two slots; head is always within that array.
        self.head = (self.head + 1) % self.slots.len();
        self.len -= 1;
    }
}

impl SimulatedNoteRecorder {
    pub fn has_pending_boundaries(&self) -> bool {
        self.session_lane.as_ref().is_some_and(|lane| lane.len != 0)
    }

    pub fn next_boundary(&self) -> Option<CaptureBoundary> {
        self.session_lane.as_ref().and_then(SessionLane::first)
    }

    /// FIFO for equal times, monotone requested times, no replacement/coalescing.
    /// A start cannot spend the final free slot reserved for an ending command.
    pub fn offer_boundary(
        &mut self,
        epoch: StreamEpoch,
        at: SampleTime,
        command: CaptureCommand,
    ) -> Result<CaptureCommandId, SessionError> {
        if epoch != self.epoch {
            return Err(SessionError::ForeignEpoch);
        }
        if self.host_interrupted {
            return Err(SessionError::Interrupted);
        }
        let lane = self.session_lane.as_ref().ok_or(SessionError::NotEnabled)?;
        if at < self.observed
            || lane.last_offer.is_some_and(|last| at < last)
            || self.sources.iter().flatten().any(|source| {
                (source.sequence.as_u64() != 0 && source.observed >= at)
                    || source.fence.is_some_and(|fence| fence > at)
            })
        {
            return Err(SessionError::PastBoundary);
        }
        let ticket = match command {
            CaptureCommand::Start(ticket) | CaptureCommand::End(ticket, _) => ticket,
        };
        let active = self.active.ok_or(NoteCaptureError::NotActive)?;
        if active.ticket != ticket {
            return Err(SessionError::Capture(NoteCaptureError::NotActive));
        }
        if matches!(command, CaptureCommand::Start(_)) {
            let context = self
                .contexts
                .get(ticket.slot)
                .and_then(Option::as_ref)
                .ok_or(NoteCaptureError::NotActive)?;
            if at != context.window().start() {
                return Err(SessionError::StartTime);
            }
        }
        let reserved = usize::from(matches!(command, CaptureCommand::Start(_)));
        if lane.len >= lane.slots.len() - reserved {
            return Err(SessionError::Full);
        }
        let serial = lane
            .serial
            .checked_add(1)
            .ok_or(SessionError::IdentityExhausted)?;
        let id = CaptureCommandId {
            session: self.session,
            serial,
        };
        let lane = self.session_lane.as_mut().ok_or(SessionError::NotEnabled)?;
        let index = (lane.head + lane.len) % lane.slots.len();
        let slot = lane.slots.get_mut(index).ok_or(SessionError::Capacity)?;
        *slot = Some(CaptureBoundary { id, at, command });
        lane.len += 1;
        lane.serial = serial;
        lane.last_offer = Some(at);
        Ok(id)
    }

    /// The fixture driver declares this explicit boundary reached. This never derives
    /// capture time from callback size or the renderer's quantum-ahead clock.
    /// Waiting returns the same command; every other receipt consumes exactly one.
    pub fn dispatch_boundary(&mut self) -> Option<BoundaryReceipt> {
        let boundary = self.next_boundary()?;
        let outcome = if self.host_interrupted {
            BoundaryOutcome::Cancelled
        } else {
            let result = match boundary.command {
                CaptureCommand::Start(ticket) => self.start_boundary(ticket),
                CaptureCommand::End(ticket, end) => {
                    self.stop_boundary(ticket, boundary.at, end.reason())
                }
            };
            match result {
                Ok(()) => BoundaryOutcome::Applied,
                Err(NoteCaptureError::StartFence) => BoundaryOutcome::WaitingForSources,
                Err(error) => BoundaryOutcome::Refused(error),
            }
        };
        if outcome != BoundaryOutcome::WaitingForSources
            && let Some(lane) = &mut self.session_lane
        {
            lane.dispatched = Some(boundary.at);
            lane.pop();
        }
        Some(BoundaryReceipt { boundary, outcome })
    }

    pub(crate) fn check_session_publication(&self, at: SampleTime) -> Result<(), NoteCaptureError> {
        if self
            .next_boundary()
            .is_some_and(|boundary| at >= boundary.at)
        {
            return Err(NoteCaptureError::PendingBoundary);
        }
        if self
            .session_lane
            .as_ref()
            .and_then(|lane| lane.dispatched)
            .is_some_and(|last| at < last)
        {
            return Err(NoteCaptureError::PastBoundary);
        }
        Ok(())
    }

    pub(crate) fn check_session_fence(&self, at: SampleTime) -> Result<(), NoteCaptureError> {
        if self.next_boundary().is_some_and(|boundary| {
            at > boundary.at
                || (at == boundary.at && matches!(boundary.command, CaptureCommand::End(..)))
        }) {
            return Err(NoteCaptureError::PendingBoundary);
        }
        Ok(())
    }
}
