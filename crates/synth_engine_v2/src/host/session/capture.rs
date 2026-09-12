//! Control-side capture setup; the callback alone dispatches queued source actions.
use super::{
    PlaybackState, SessionCommandId, SessionError, SessionSourceAction, SessionSourceId,
    SessionSourceLimits, SessionSourceReceipt, source::SourceQueue,
};
use crate::host::{ConnectionGeneration, ConnectionState, HostError, SimulatedHost};
use crate::profile::RecordingLimits;
use crate::quantities::{PreparedBytes, SampleRate};
use crate::recording::TakeReservation;
use crate::recording::notes::{ControllerSnapshot, NoteArmContext, SimulatedNoteRecorder};
use crate::time::{PlanPosition, SampleTime, StreamAnchor};

impl SimulatedHost {
    /// Prepare the retained recorder and bounded source queue off-thread, once.
    pub fn prepare_ordered_capture(
        &mut self,
        generation: ConnectionGeneration,
        recording: RecordingLimits,
        sources: SessionSourceLimits,
    ) -> Result<(), HostError> {
        if self.note_capture.is_some() {
            return Err(HostError::CaptureRetained);
        }
        let connection = self.active_mut(generation)?;
        if connection.status.state != ConnectionState::Ready {
            return Err(HostError::WrongState);
        }
        let prepared = connection.prepared.as_mut().ok_or(HostError::WrongState)?;
        let session = prepared.session.as_mut().ok_or(SessionError::NotEnabled)?;
        if session.sources.is_some() {
            return Err(HostError::CaptureRetained);
        }
        let queue = SourceQueue::prepare(generation, sources)?;
        let mut recorder =
            SimulatedNoteRecorder::prepare_fixture(prepared.control.epoch(), recording)?;
        recorder.host_generation = Some(generation);
        session.sources = Some(queue);
        self.note_capture = Some(recorder);
        Ok(())
    }

    pub fn session_source_bytes(&self) -> Option<PreparedBytes> {
        self.active
            .as_ref()?
            .prepared
            .as_ref()?
            .session
            .as_ref()?
            .sources
            .as_ref()
            .map(SourceQueue::bytes)
    }

    /// Binding and arming may allocate; neither source dispatch nor capture boundaries
    /// can bypass the callback's ordering through this view.
    pub fn session_capture_control(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<SessionCaptureControl<'_>, HostError> {
        let connection = self
            .active
            .as_mut()
            .filter(|connection| connection.status.generation == generation)
            .ok_or(HostError::StaleGeneration)?;
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(HostError::WrongState);
        }
        let prepared = connection.prepared.as_mut().ok_or(HostError::WrongState)?;
        let session = prepared.session.as_mut().ok_or(SessionError::NotEnabled)?;
        if session.held != 0 || session.sources.as_ref().is_some_and(SourceQueue::has_held) {
            return Err(SessionError::RetainedOutcomes.into());
        }
        let PlaybackState::Stopped(position) = session.state else {
            return Err(SessionError::AlreadyPlaying.into());
        };
        let recorder = self
            .note_capture
            .as_mut()
            .filter(|recorder| recorder.host_generation == Some(generation))
            .ok_or(SessionError::NoCapture)?;
        Ok(SessionCaptureControl {
            recorder,
            rate: prepared.control.plan().sample_rate(),
            position,
            clock: prepared.renderer.clock(),
        })
    }

    pub fn offer_session_recording_play(
        &mut self,
        generation: ConnectionGeneration,
        at: SampleTime,
        ticket: TakeReservation,
    ) -> Result<SessionCommandId, HostError> {
        let recorder = self
            .note_capture
            .as_ref()
            .filter(|recorder| recorder.host_generation == Some(generation))
            .ok_or(SessionError::NoCapture)?;
        let context = recorder.armed_context(ticket)?;
        let expected_anchor = context.anchor().ok_or(SessionError::CaptureContext)?;
        if context.window().start() != at {
            return Err(SessionError::CaptureContext.into());
        }
        let connection = self.active_mut(generation)?;
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(HostError::WrongState);
        }
        let prepared = connection.prepared.as_mut().ok_or(HostError::WrongState)?;
        let session = prepared.session.as_mut().ok_or(SessionError::NotEnabled)?;
        if session.state != PlaybackState::Stopped(expected_anchor.position())
            || expected_anchor.time() != at
        {
            return Err(SessionError::CaptureContext.into());
        }
        Ok(session.offer_play(
            &mut prepared.control,
            prepared.renderer.clock(),
            at,
            Some(ticket),
        )?)
    }

    pub fn offer_session_source(
        &mut self,
        generation: ConnectionGeneration,
        action: SessionSourceAction,
    ) -> Result<SessionSourceId, HostError> {
        let recorder = self
            .note_capture
            .as_ref()
            .filter(|recorder| recorder.host_generation == Some(generation))
            .ok_or(SessionError::NoCapture)?;
        let _sequence = recorder.source_sequence(action.source())?;
        let connection = self.active_mut(generation)?;
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(HostError::WrongState);
        }
        let prepared = connection.prepared.as_mut().ok_or(HostError::WrongState)?;
        if action.epoch() != prepared.control.epoch() {
            return Err(SessionError::Capture(
                crate::recording::notes::NoteCaptureError::ForeignEpoch,
            )
            .into());
        }
        let session = prepared.session.as_mut().ok_or(SessionError::NotEnabled)?;
        if session.closed {
            return Err(SessionError::Closed.into());
        }
        Ok(session
            .sources
            .as_mut()
            .ok_or(SessionError::NoCapture)?
            .offer(prepared.renderer.clock(), action)?)
    }

    pub fn collect_session_source_receipt(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<Option<SessionSourceReceipt>, HostError> {
        if self.active().is_some_and(|status| {
            status.generation == generation && status.state == ConnectionState::Quiescing
        }) {
            self.close_ordered_session(generation)?;
        }
        let connection = self.active_mut(generation)?;
        let session = connection
            .prepared
            .as_mut()
            .and_then(|prepared| prepared.session.as_mut())
            .ok_or(SessionError::NotEnabled)?;
        Ok(session
            .sources
            .as_mut()
            .ok_or(SessionError::NoCapture)?
            .collect())
    }
}

#[must_use]
pub struct SessionCaptureControl<'a> {
    recorder: &'a mut SimulatedNoteRecorder,
    rate: SampleRate,
    position: PlanPosition,
    clock: SampleTime,
}
impl SessionCaptureControl<'_> {
    pub fn bind_source(
        &mut self,
        initial: ControllerSnapshot,
    ) -> Result<ConnectionGeneration, SessionError> {
        Ok(self.recorder.bind_fixture_source(Some(initial))?)
    }
    pub fn arm(
        &mut self,
        context: NoteArmContext,
        sources: &[ConnectionGeneration],
    ) -> Result<TakeReservation, SessionError> {
        let at = context.window().start();
        if at < self.clock
            || context.anchor() != Some(StreamAnchor::new(at, self.position))
            || context.tempo().sample_rate() != self.rate
        {
            return Err(SessionError::CaptureContext);
        }
        Ok(self.recorder.arm(context, sources)?)
    }
}

#[cfg(test)]
mod tests;
