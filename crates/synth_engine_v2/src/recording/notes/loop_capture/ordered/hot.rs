//! Fixed-storage session dispatch; the journal commits only the complete callback.

use super::{CommandLane, LoopRecordingSession, LoopSessionError};
use crate::{
    host::{
        ConnectionGeneration,
        session::{
            SessionCaptureOutcome, SessionCommand, SessionError, SessionOutcome,
            source::SourceQueue,
        },
    },
    looping::{LoopFault, LoopRenderControl, LoopSnapshot},
    quantities::EventCount,
    recording::{
        CaptureOutcome, TakeReservation,
        notes::{CaptureStopReason, SimulatedNoteRecorder},
    },
    render::AudioBlockMut,
    time::SampleTime,
};

impl CommandLane {
    fn following_stop(&self, at: SampleTime) -> bool {
        for offset in self.completed + 1..self.held {
            let index = (self.head + offset) % self.slots.len();
            let Some(entry) = self.slots.get(index).and_then(Option::as_ref) else {
                return false;
            };
            if entry.boundary.at != at {
                return false;
            }
            if matches!(
                entry.boundary.command,
                SessionCommand::Stop | SessionCommand::Panic
            ) {
                return true;
            }
        }
        false
    }

    fn boundary(
        &mut self,
        snapshot: LoopSnapshot,
        recorder: &mut SimulatedNoteRecorder,
        sources: &mut SourceQueue,
        ticket: TakeReservation,
    ) -> Result<LoopRenderControl, LoopFault> {
        let mut operations = EventCount::NONE;
        let mut finish = false;
        while self.completed < self.held {
            let index = (self.head + self.completed) % self.slots.len();
            let boundary = self
                .slots
                .get(index)
                .and_then(Option::as_ref)
                .map(|entry| entry.boundary)
                .ok_or(LoopFault::PreparedState)?;
            if boundary.at > snapshot.clock {
                break;
            }
            if boundary.at < snapshot.clock {
                return Err(LoopFault::PreparedState);
            }
            let (outcome, capture) = match boundary.command {
                SessionCommand::Play if self.stopped || self.following_stop(snapshot.clock) => {
                    (SessionOutcome::Cancelled, SessionCaptureOutcome::Cancelled)
                }
                SessionCommand::Play => {
                    sources.drain(recorder, snapshot.clock, true);
                    match recorder.start(ticket) {
                        Ok(()) => {
                            self.playing = true;
                            (
                                SessionOutcome::Applied {
                                    position: snapshot.position,
                                },
                                SessionCaptureOutcome::Applied,
                            )
                        }
                        Err(error) => (
                            SessionOutcome::CaptureRefused,
                            SessionCaptureOutcome::Refused(error),
                        ),
                    }
                }
                SessionCommand::Stop | SessionCommand::Panic => {
                    let reason = if boundary.command == SessionCommand::Panic {
                        CaptureStopReason::Panic
                    } else {
                        CaptureStopReason::Stop
                    };
                    self.playing = false;
                    self.stopped = true;
                    if self.end.is_none() {
                        self.end = Some((snapshot.clock, boundary.command));
                    }
                    finish = true;
                    let capture = match recorder.stop(ticket, snapshot.clock, reason) {
                        Ok(()) => SessionCaptureOutcome::Applied,
                        Err(error) => SessionCaptureOutcome::Refused(error),
                    };
                    (
                        SessionOutcome::Applied {
                            position: snapshot.position,
                        },
                        capture,
                    )
                }
            };
            let entry = self
                .slots
                .get_mut(index)
                .and_then(Option::as_mut)
                .ok_or(LoopFault::PreparedState)?;
            entry.outcome = Some(outcome);
            entry.capture = Some(capture);
            self.completed += 1;
            // Admission bounds the complete same-time group by the u32 session share.
            operations = EventCount::measured(operations.get() + 1);
        }
        // A successful initial Play is already charged by the loop's initial restore.
        // Every other possible group is stopped and has no compiled-loop publication.
        Ok(LoopRenderControl {
            playing: self.playing,
            finish,
            idle_operations: operations,
        })
    }

    pub(super) fn close(&mut self) {
        self.closed = true;
        self.playing = false;
        for offset in self.completed..self.held {
            let index = (self.head + offset) % self.slots.len();
            if let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) {
                entry.outcome = Some(SessionOutcome::Cancelled);
                entry.capture = Some(SessionCaptureOutcome::Cancelled);
            }
        }
        self.completed = self.held;
    }
}

impl LoopRecordingSession {
    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), LoopSessionError> {
        if self.commands.closed {
            output.silence();
            return Err(LoopSessionError::Session(SessionError::Closed));
        }
        let ticket = self.capture.ticket.ok_or(LoopSessionError::FreshCapture)?;
        let commands = &mut self.commands;
        let sources = &mut self.sources;
        let recorder = &mut self.capture.recorder;
        let started = &mut self.capture.started;
        let result = self.capture.journal.render_controlled(output, |snapshot| {
            let end = snapshot
                .clock
                .checked_advance_quantum()
                .map_err(LoopFault::Time)?;
            let action = commands.boundary(snapshot, recorder, sources, ticket)?;
            *started |= action.playing;
            sources.drain(recorder, end, false);
            Ok(action)
        });
        if let Err(error) = result {
            if self.capture.journal.is_faulted() {
                self.interrupt(CaptureStopReason::LoopRenderFault)?;
            }
            return Err(LoopSessionError::Capture(super::LoopCaptureError::Render(
                error,
            )));
        }
        Ok(())
    }

    /// Consume the finite admitted source FIFO after Stop, even when its progress
    /// exceeds the last audio clock. This never extends acknowledged audio or
    /// manufactures a source acknowledgement; raw selection keeps its Stop endpoint.
    pub fn drain_stopped_sources(&mut self) -> Result<(), LoopSessionError> {
        if self.commands.closed {
            return Err(LoopSessionError::Session(SessionError::Closed));
        }
        if !self.commands.stopped {
            return Err(LoopSessionError::NotStopped);
        }
        self.sources.drain_stopped(&mut self.capture.recorder);
        Ok(())
    }

    pub(super) fn interrupt(&mut self, reason: CaptureStopReason) -> Result<(), LoopSessionError> {
        self.commands.close();
        self.sources.close();
        let terminal = self.capture.journal.finish();
        if let Some(active) = self.capture.recorder.active {
            self.capture
                .recorder
                .stop_at(
                    active.ticket,
                    terminal.at,
                    CaptureOutcome::Interrupted,
                    reason,
                )
                .map_err(super::LoopCaptureError::from)?;
        }
        self.capture
            .recorder
            .interrupt_host(reason)
            .map_err(super::LoopCaptureError::from)?;
        Ok(())
    }

    /// Exclusive serial loss notification freezes the last successful audio/source
    /// frontiers. It is not a concurrent backend fence or a hardware timestamp.
    pub fn device_lost(&mut self) -> Result<(), LoopSessionError> {
        if self.commands.closed {
            return Ok(());
        }
        self.interrupt(CaptureStopReason::DeviceLost)
    }

    pub fn source_lost(&mut self, source: ConnectionGeneration) -> Result<(), LoopSessionError> {
        let _sequence = self.capture.source_sequence(source)?;
        self.device_lost()
    }

    /// Source retirement cannot advance the already frozen selection. No render
    /// callback is required; finalization remains a separate off-thread operation.
    pub fn acknowledge_source_quiescence(
        &mut self,
        source: ConnectionGeneration,
    ) -> Result<(), LoopSessionError> {
        if !self.commands.closed {
            return Err(LoopSessionError::NotClosed);
        }
        self.capture
            .recorder
            .acknowledge_host_source(source)
            .map_err(super::LoopCaptureError::from)?;
        Ok(())
    }
}
