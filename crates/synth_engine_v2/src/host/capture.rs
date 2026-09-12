//! Serialized capture custody for one simulated output generation (P09-S005).
//!
//! Only the control view can publish or arm, and borrowing it excludes a simultaneous
//! host transition. Once the output is Quiescing, no such view can be obtained. Source
//! acknowledgements model backend fences; they neither run callbacks nor advance time.

use super::{ConnectionGeneration, ConnectionState, HostError, HostFailure, SimulatedHost};
use crate::profile::RecordingLimits;
use crate::quantities::SampleRate;
use crate::recording::notes::session::{
    BoundaryReceipt, CaptureCommand, CaptureCommandCapacity, CaptureCommandId, SessionError,
};
use crate::recording::notes::{
    AuditionTrace, CaptureStamp, CaptureStopReason, ControllerSnapshot, Midi1Input, NoteArmContext,
    NoteCaptureError, PublicationReceipt, PublicationSequence, SimulatedNoteRecorder,
};
use crate::recording::{CaptureQuality, TakeReservation};
use crate::time::{SampleTime, StreamAnchor, StreamEpoch};

impl SimulatedHost {
    /// Prepare off-thread for an active Ready output, with the recorder's complete
    /// descriptor and storage charged by its existing byte admission. A retained owner
    /// cannot be replaced, including after a different output has been activated.
    pub fn prepare_note_capture(
        &mut self,
        generation: ConnectionGeneration,
        limits: RecordingLimits,
    ) -> Result<(), HostError> {
        let connection = self.active_mut(generation)?;
        if connection
            .prepared
            .as_ref()
            .is_some_and(|prepared| prepared.session.is_some())
        {
            return Err(super::session::SessionError::OrderedTransport.into());
        }
        if connection.status.state != ConnectionState::Ready {
            return Err(HostError::WrongState);
        }
        let epoch = connection
            .status
            .identity
            .ok_or(HostError::WrongState)?
            .epoch;
        if self.note_capture.is_some() {
            return Err(HostError::CaptureRetained);
        }
        let mut capture = SimulatedNoteRecorder::prepare_fixture(epoch, limits)?;
        capture.host_generation = Some(generation);
        self.note_capture = Some(capture);
        Ok(())
    }

    /// Read retained capture even after its output was reclaimed or replaced.
    pub const fn note_capture(&self) -> Option<&SimulatedNoteRecorder> {
        self.note_capture.as_ref()
    }

    /// A bounded serial publication view. No mutable recorder reference escapes, so
    /// callers cannot replace its owner, substitute an epoch or re-arm it after loss.
    pub fn note_capture_control(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<NoteCaptureControl<'_>, HostError> {
        let connection = self.active_mut(generation)?;
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(HostError::WrongState);
        }
        let prepared = connection.prepared.as_ref().ok_or(HostError::WrongState)?;
        if prepared.session.is_some() {
            return Err(super::session::SessionError::OrderedTransport.into());
        }
        let rate = prepared.control.plan().sample_rate();
        let anchor = prepared.control.anchor();
        Ok(NoteCaptureControl {
            recorder: self.owned_capture(generation)?,
            rate,
            anchor,
        })
    }

    fn owned_capture(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<&mut SimulatedNoteRecorder, HostError> {
        self.note_capture
            .as_mut()
            .filter(|capture| capture.host_generation == Some(generation))
            .ok_or(HostError::NoNoteCapture)
    }

    pub(super) fn interrupt_note_capture(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), HostError> {
        let failure = self.connection_mut(generation)?.status.failure;
        let reason = match failure {
            Some(HostFailure::DeviceLost | HostFailure::CaptureSourceLost(_)) => {
                CaptureStopReason::DeviceLost
            }
            _ => CaptureStopReason::DeviceReprepare,
        };
        if let Some(capture) = self
            .note_capture
            .as_mut()
            .filter(|capture| capture.host_generation == Some(generation))
        {
            capture.interrupt_host(reason)?;
        }
        Ok(())
    }

    /// Loss of an exact-input fixture source also stops the output. The failed source
    /// remains named in persistent host diagnostics. No hardware input is modeled.
    pub fn note_source_lost(
        &mut self,
        generation: ConnectionGeneration,
        source: ConnectionGeneration,
    ) -> Result<(), HostError> {
        let _validated_sequence = self.owned_capture(generation)?.source_sequence(source)?;
        let connection = self.active_mut(generation)?;
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running | ConnectionState::Quiescing
        ) {
            return Err(HostError::WrongState);
        }
        connection
            .status
            .failure
            .get_or_insert(HostFailure::CaptureSourceLost(source));
        connection.status.state = ConnectionState::Quiescing;
        self.close_ordered_session(generation)?;
        self.interrupt_note_capture(generation)
    }

    /// Acknowledge one source's shutdown off-thread, possibly long after the last
    /// callback. Every bound source must acknowledge before output resources retire.
    /// Repeated acknowledgements are harmless; foreign sources or generations refuse.
    pub fn acknowledge_note_source_quiescence(
        &mut self,
        generation: ConnectionGeneration,
        source: ConnectionGeneration,
    ) -> Result<(), HostError> {
        let _validated_sequence = self.owned_capture(generation)?.source_sequence(source)?;
        if self.active_mut(generation)?.status.state != ConnectionState::Quiescing {
            return Err(HostError::WrongState);
        }
        // Also handles terminal renderer faults, whose callback only closes the host
        // state. Publication views have been unavailable since that transition.
        self.interrupt_note_capture(generation)?;
        self.owned_capture(generation)?
            .acknowledge_host_source(source)?;
        Ok(())
    }

    /// Explicit off-thread result disposal keeps the existing quality/fence checks.
    pub fn discard_note_capture(
        &mut self,
        generation: ConnectionGeneration,
        ticket: TakeReservation,
        quality: CaptureQuality,
    ) -> Result<(), HostError> {
        if self
            .active
            .as_ref()
            .filter(|connection| connection.status.generation == generation)
            .and_then(|connection| connection.prepared.as_ref())
            .and_then(|prepared| prepared.session.as_ref())
            .is_some_and(super::session::SessionRuntime::has_retained_outcomes)
        {
            return Err(super::session::SessionError::RetainedOutcomes.into());
        }
        self.owned_capture(generation)?.discard(ticket, quality)?;
        Ok(())
    }

    /// Dispatch the next explicitly reached serial capture boundary. After loss,
    /// commands drain as cancellation receipts, including after output retirement.
    pub fn dispatch_note_boundary(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<Option<BoundaryReceipt>, HostError> {
        if self.active().is_some_and(|status| {
            status.generation == generation && status.state == ConnectionState::Quiescing
        }) {
            self.interrupt_note_capture(generation)?;
        }
        Ok(self.owned_capture(generation)?.dispatch_boundary())
    }

    /// Only resolved, quiescent storage can be reclaimed. Reconnection alone never
    /// replaces this owner or spends another recording quota beside retained results.
    pub fn release_note_capture(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), HostError> {
        let capture = self.owned_capture(generation)?;
        if !capture.host_quiescent()
            || capture.retained_results().next().is_some()
            || capture.has_pending_boundaries()
        {
            return Err(HostError::CaptureRetained);
        }
        if self.active().is_some_and(|status| {
            status.generation == generation && status.state == ConnectionState::Quiescing
        }) {
            return Err(HostError::AwaitingQuiescence);
        }
        self.note_capture = None;
        Ok(())
    }
}

/// Operations admitted while the matching output is Ready or Running. This view
/// borrows host-owned storage and cannot transfer or replace it.
#[must_use]
pub struct NoteCaptureControl<'a> {
    recorder: &'a mut SimulatedNoteRecorder,
    rate: SampleRate,
    anchor: StreamAnchor,
}

impl NoteCaptureControl<'_> {
    /// Off-thread queue preparation under the recorder's byte admission.
    pub fn enable_ordered_session(
        &mut self,
        capacity: CaptureCommandCapacity,
    ) -> Result<(), SessionError> {
        self.recorder.enable_ordered_session(capacity)
    }

    pub fn offer_boundary(
        &mut self,
        epoch: StreamEpoch,
        at: SampleTime,
        command: CaptureCommand,
    ) -> Result<CaptureCommandId, SessionError> {
        self.recorder.offer_boundary(epoch, at, command)
    }

    pub fn bind_source(
        &mut self,
        initial: ControllerSnapshot,
    ) -> Result<ConnectionGeneration, NoteCaptureError> {
        self.recorder.bind_fixture_source(Some(initial))
    }
    pub fn arm(
        &mut self,
        context: NoteArmContext,
        sources: &[ConnectionGeneration],
    ) -> Result<TakeReservation, NoteCaptureError> {
        if context.tempo().sample_rate() != self.rate {
            return Err(NoteCaptureError::HostSampleRate);
        }
        if context.anchor() != Some(self.anchor) {
            return Err(NoteCaptureError::HostAnchor);
        }
        self.recorder.arm(context, sources)
    }
    pub fn start(&mut self, ticket: TakeReservation) -> Result<(), NoteCaptureError> {
        self.recorder.start(ticket)
    }
    pub fn publish(
        &mut self,
        source: ConnectionGeneration,
        stamp: CaptureStamp,
        input: Midi1Input,
        audition: AuditionTrace,
    ) -> Result<PublicationReceipt, NoteCaptureError> {
        self.recorder.publish(source, stamp, input, audition)
    }
    pub fn fence(
        &mut self,
        source: ConnectionGeneration,
        epoch: StreamEpoch,
        frontier: SampleTime,
        through: PublicationSequence,
    ) -> Result<(), NoteCaptureError> {
        self.recorder.fence(source, epoch, frontier, through)
    }
    pub fn stop(
        &mut self,
        ticket: TakeReservation,
        at: SampleTime,
        reason: CaptureStopReason,
    ) -> Result<(), NoteCaptureError> {
        self.recorder.stop(ticket, at, reason)
    }
}

#[cfg(test)]
pub(super) mod tests;
