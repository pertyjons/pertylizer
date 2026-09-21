//! Serial callback ordering. All command ownership remains in preallocated slots.

use super::{PlaybackState, SessionCaptureOutcome, SessionCommand, SessionOutcome, SessionRuntime};
use crate::publish::ProducerClass;
use crate::quantities::EventCount;
use crate::recording::notes::{CaptureStopReason, NoteCaptureError, SimulatedNoteRecorder};
use crate::render::{AudioBlockMut, PreparedRenderer};
use crate::schedule::ScheduledRenderError;
use crate::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

impl SessionRuntime {
    pub(super) fn stop_at(&mut self, at: SampleTime) -> Result<PlanPosition, ScheduledRenderError> {
        let position = match self.state {
            PlaybackState::Unavailable => {
                return Err(ScheduledRenderError::CallSpanUnrepresentable { clock: at });
            }
            PlaybackState::Stopped(position) => position,
            PlaybackState::Playing(anchor) => {
                let delta = at
                    .difference(anchor.time())
                    .map_err(|_| ScheduledRenderError::CallSpanUnrepresentable { clock: at })?;
                let frames = u64::try_from(delta.as_i64())
                    .map_err(|_| ScheduledRenderError::CallSpanUnrepresentable { clock: at })?;
                anchor
                    .position()
                    .checked_add(FrameCount::new(frames))
                    .map_err(|_| ScheduledRenderError::CallSpanUnrepresentable { clock: at })?
            }
        };
        self.state = PlaybackState::Stopped(position);
        Ok(position)
    }
    fn pending_index(&self) -> Option<usize> {
        (self.completed < self.held).then_some((self.head + self.completed) % self.slots.len())
    }

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

    fn complete(&mut self, index: usize, outcome: SessionOutcome) {
        if let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) {
            if outcome == SessionOutcome::Cancelled && entry.boundary.capture.is_some() {
                entry.capture_outcome = Some(SessionCaptureOutcome::Cancelled);
            }
            entry.outcome = Some(outcome);
            self.completed += 1;
        }
    }

    fn boundary(
        &mut self,
        renderer: &mut PreparedRenderer,
        mut capture: Option<&mut SimulatedNoteRecorder>,
    ) -> Result<(), ScheduledRenderError> {
        let at = renderer.clock();
        self.operations = EventCount::measured(0);
        while let Some(index) = self.pending_index() {
            let Some(boundary) = self
                .slots
                .get(index)
                .and_then(Option::as_ref)
                .map(|entry| entry.boundary)
            else {
                return Err(ScheduledRenderError::CallSpanUnrepresentable { clock: at });
            };
            if let Some(error) = self
                .slots
                .get(index)
                .and_then(Option::as_ref)
                .and_then(|entry| entry.delivery_error)
            {
                self.complete(index, SessionOutcome::DeliveryRefused(error));
                continue;
            }
            if boundary.at > at {
                break;
            }
            if boundary.at < at {
                return Err(ScheduledRenderError::MissedEvent {
                    event: boundary.at,
                    clock: at,
                });
            }
            match boundary.command {
                SessionCommand::Stop | SessionCommand::Panic => {
                    let reason = if boundary.command == SessionCommand::Panic {
                        CaptureStopReason::Panic
                    } else {
                        CaptureStopReason::Stop
                    };
                    let position = self.stop_at(at)?;
                    // Offer admission bounded this group, including Play catch-up.
                    self.operations = EventCount::measured(self.operations.get() + 1);
                    if let Some(ticket) = boundary.capture {
                        let outcome = match capture
                            .as_deref_mut()
                            .map_or(Err(NoteCaptureError::NotActive), |recorder| {
                                recorder.stop(ticket, at, reason)
                            }) {
                            Ok(()) => SessionCaptureOutcome::Applied,
                            Err(error) => SessionCaptureOutcome::Refused(error),
                        };
                        if let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) {
                            entry.capture_outcome = Some(outcome);
                        }
                    }
                    self.complete(index, SessionOutcome::Applied { position });
                }
                SessionCommand::Play => {
                    if self.following_stop(at) {
                        self.complete(index, SessionOutcome::Cancelled);
                        continue;
                    }
                    if let Some(expected) = self
                        .slots
                        .get(index)
                        .and_then(Option::as_ref)
                        .and_then(|entry| entry.expected_position)
                    {
                        let refusal = match self.state {
                            PlaybackState::Stopped(actual) if actual == expected => None,
                            PlaybackState::Stopped(actual) => {
                                Some(super::transfer::SessionDeliveryError::PositionChanged {
                                    actual,
                                })
                            }
                            _ => Some(super::transfer::SessionDeliveryError::NotStopped),
                        };
                        if let Some(error) = refusal {
                            self.complete(index, SessionOutcome::DeliveryRefused(error));
                            continue;
                        }
                    }
                    let candidate = self
                        .slots
                        .get_mut(index)
                        .and_then(Option::as_mut)
                        .and_then(|entry| entry.activation.take());
                    let Some(candidate) = candidate else {
                        return Err(ScheduledRenderError::CallSpanUnrepresentable { clock: at });
                    };
                    let mut capture_started = false;
                    if let Some(ticket) = boundary.capture {
                        // Cancelled Plays were handled above; no same-time Stop follows
                        // a surviving Play. Earlier Stops have already executed.
                        if let (Some(sources), Some(recorder)) =
                            (self.sources.as_mut(), capture.as_deref_mut())
                        {
                            sources.drain(recorder, at, true);
                        }
                        if self.scheduler.check_offer(renderer, &candidate).is_ok() {
                            let result = capture
                                .as_deref_mut()
                                .map_or(Err(NoteCaptureError::NotActive), |recorder| {
                                    recorder.start(ticket)
                                });
                            match result {
                                Ok(()) => {
                                    capture_started = true;
                                    if let Some(entry) =
                                        self.slots.get_mut(index).and_then(Option::as_mut)
                                    {
                                        entry.capture_outcome =
                                            Some(SessionCaptureOutcome::Applied);
                                    }
                                }
                                Err(error) => {
                                    if let Some(entry) =
                                        self.slots.get_mut(index).and_then(Option::as_mut)
                                    {
                                        entry.activation = Some(candidate);
                                        entry.capture_outcome =
                                            Some(SessionCaptureOutcome::Refused(error));
                                    }
                                    self.complete(index, SessionOutcome::CaptureRefused);
                                    continue;
                                }
                            }
                        } else if let Some(entry) =
                            self.slots.get_mut(index).and_then(Option::as_mut)
                        {
                            entry.capture_outcome = Some(SessionCaptureOutcome::Cancelled);
                        }
                    }
                    let position = candidate.position();
                    match self.scheduler.offer(renderer, candidate) {
                        Ok(()) => {
                            // Offer is immediately before the one-quantum render call.
                            // No later session action can invalidate the exchange value.
                            self.state = PlaybackState::Playing(StreamAnchor::new(at, position));
                            self.adopted_slot = Some(index);
                            break;
                        }
                        Err((candidate, error)) => {
                            if let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut)
                            {
                                entry.activation = Some(candidate);
                            }
                            self.complete(index, SessionOutcome::Refused(error));
                            if capture_started {
                                return Err(ScheduledRenderError::SessionActivationRefused(error));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn retain_adopted(&mut self) {
        let Some(index) = self.adopted_slot else {
            return;
        };
        // The slot is retained throughout the call; it cannot have been collected.
        let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) else {
            return;
        };
        if let Some(retired) = self.scheduler.take_retired() {
            let position = retired.position();
            entry.activation = Some(retired);
            entry.outcome = Some(SessionOutcome::Applied { position });
            self.completed += 1;
            self.adopted_slot = None;
        }
    }

    fn idle_quantum(
        &mut self,
        renderer: &mut PreparedRenderer,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        let clock = renderer.clock();
        let mut publication = self
            .arbiter
            .open(clock, 1)
            .map_err(ScheduledRenderError::Publication)?;
        for _ in 0..self.operations.get() {
            if let Err(fault) = publication.charge_operation(ProducerClass::Session, clock) {
                renderer.terminal_fault(&mut output);
                return Err(ScheduledRenderError::Publication(fault));
            }
        }
        let _batch = publication.seal();
        renderer
            .render_idle(output)
            .map_err(ScheduledRenderError::Render)
    }

    pub(in crate::host) fn render(
        &mut self,
        renderer: &mut PreparedRenderer,
        mut output: AudioBlockMut<'_>,
        mut capture: Option<&mut SimulatedNoteRecorder>,
    ) -> Result<(), ScheduledRenderError> {
        let _quanta = renderer.validate_output(&mut output)?;
        let mut delivered = 0;
        while delivered < output.frames() {
            let carry = renderer.carry_frames();
            let frames = (output.frames() - delivered).min(if carry == 0 {
                QUANTUM_FRAMES as usize
            } else {
                carry
            });
            let Some(window) = output.window(delivered, frames) else {
                return Err(ScheduledRenderError::CallSpanUnrepresentable {
                    clock: renderer.clock(),
                });
            };
            let result = if carry != 0 {
                // No new quantum, hence no musical action or adoption on this subcall.
                renderer
                    .render_idle(window)
                    .map_err(ScheduledRenderError::Render)
            } else {
                // Validate the source window before a coupled start can take effect.
                let source_end = if self.sources.is_some() {
                    Some(renderer.clock().checked_advance_quantum().map_err(|_| {
                        ScheduledRenderError::CallSpanUnrepresentable {
                            clock: renderer.clock(),
                        }
                    })?)
                } else {
                    None
                };
                self.boundary(renderer, capture.as_deref_mut())?;
                if let (Some(sources), Some(recorder), Some(end)) =
                    (self.sources.as_mut(), capture.as_deref_mut(), source_end)
                {
                    sources.drain(recorder, end, false);
                }
                let result = match self.state {
                    PlaybackState::Unavailable => {
                        Err(ScheduledRenderError::CallSpanUnrepresentable {
                            clock: renderer.clock(),
                        })
                    }
                    PlaybackState::Stopped(_) => self.idle_quantum(renderer, window),
                    PlaybackState::Playing(_) => {
                        self.scheduler.set_session_operations(self.operations);
                        self.scheduler.render(renderer, &mut self.arbiter, window)
                    }
                };
                self.retain_adopted();
                result
            };
            if let Err(error) = result {
                output.silence();
                return Err(error);
            }
            delivered += frames;
        }
        Ok(())
    }
}

impl super::SessionCommandId {
    pub(crate) fn after(
        generation: crate::host::ConnectionGeneration,
        previous: u64,
    ) -> Result<Self, super::SessionError> {
        let serial = previous
            .checked_add(1)
            .ok_or(super::SessionError::IdentityExhausted)?;
        Ok(Self { generation, serial })
    }
}
