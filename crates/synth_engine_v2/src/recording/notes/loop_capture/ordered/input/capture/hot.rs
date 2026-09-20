//! Exclusive callback delegation and fixed-storage loss handling.
use super::super::{InputError, InputEventId, InputObservation, InputReceipt, InputTick};
use super::{InputCaptureError, InputCaptureSession};
use crate::{
    host::{
        ConnectionGeneration, ConnectionState,
        session::{LoopSessionError, SessionCommand, SessionCommandId, SessionReceipt},
    },
    looping::LoopSnapshot,
    recording::notes::{
        CaptureStamp, CaptureStopReason, Midi1Input, loop_capture::LoopCaptureError,
    },
    render::AudioBlockMut,
    time::SampleTime,
};

impl InputCaptureSession {
    fn port(&self, generation: ConnectionGeneration) -> Result<usize, InputError> {
        for (index, input) in self.inputs.iter().enumerate() {
            if input.generation == Some(generation) {
                return Ok(index);
            }
        }
        Err(InputError::Stale)
    }
    pub fn start_input(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), InputCaptureError> {
        let port = self.port(generation)?;
        Ok(self.inputs[port].start(generation)?)
    }
    pub fn offer_message(
        &mut self,
        generation: ConnectionGeneration,
        tick: InputTick,
        arrival: SampleTime,
        input: Midi1Input,
    ) -> Result<InputEventId, InputCaptureError> {
        let port = self.port(generation)?;
        let result = self.inputs[port].offer_message(generation, tick, arrival, input);
        self.synchronize_failure()?;
        Ok(result?)
    }
    pub fn advance_frontier(
        &mut self,
        generation: ConnectionGeneration,
        tick: InputTick,
    ) -> Result<InputEventId, InputCaptureError> {
        let port = self.port(generation)?;
        let result = self.inputs[port].advance_frontier(generation, tick);
        self.synchronize_failure()?;
        Ok(result?)
    }
    pub fn device_lost(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), InputCaptureError> {
        let port = self.port(generation)?;
        self.inputs[port].device_lost(generation)?;
        self.synchronize_failure()
    }
    pub(super) fn synchronize_failure(&mut self) -> Result<(), InputCaptureError> {
        if self.closed {
            return Ok(());
        }
        let mut reason = None;
        for input in &self.inputs {
            if let Some(fault) = input.discontinuity
                && fault.reason != InputError::PeerInterrupted
            {
                // Known exact late refusals still owe the recorder's quality
                // attribution, even after normal sealing. Mapping failures use only
                // conservative diagnostic bounds, never a fabricated exact stamp.
                if fault.reason == InputError::Order
                    && let Some(InputObservation::Message { tick, arrival, .. }) = fault.observation
                    && let Some(clock) = input.clock
                    && let Ok(nominal) = clock.map(tick)
                    && let Ok(stamp) = CaptureStamp::exact_fixture(clock.epoch, nominal, arrival)
                    && let Some(source) = input.binding
                {
                    self.session
                        .capture
                        .recorder
                        .attribute_refused_input(source, stamp)
                        .map_err(|error| {
                            InputCaptureError::Recording(LoopSessionError::Capture(
                                LoopCaptureError::Capture(error),
                            ))
                        })?;
                }
                if matches!(fault.reason, InputError::Uncertain | InputError::ClockRange)
                    && let Some(InputObservation::Message { tick, .. }) = fault.observation
                    && let Some(clock) = input.clock
                    && let Some(source) = input.binding
                    && let Some((earliest, latest)) = clock.quality_bounds(tick)
                {
                    self.session
                        .capture
                        .recorder
                        .attribute_uncertain_input(source, earliest, latest)
                        .map_err(|error| {
                            InputCaptureError::Recording(LoopSessionError::Capture(
                                LoopCaptureError::Capture(error),
                            ))
                        })?;
                }
                reason = Some(if fault.reason == InputError::DeviceLost {
                    CaptureStopReason::DeviceLost
                } else {
                    CaptureStopReason::SourceInvalid
                });
                break;
            }
        }
        if let Some(reason) = reason {
            // A split owner may already have halted audio before joined quality attribution.
            if !self.session.commands.closed {
                self.session.interrupt(reason)?;
            }
            for input in &mut self.inputs {
                if input.discontinuity.is_none() {
                    input.fail(InputError::PeerInterrupted, None);
                }
            }
            self.close_inputs();
        }
        Ok(())
    }
    pub(super) fn close_inputs(&mut self) {
        self.closed = true;
        for input in &mut self.inputs {
            input.state = ConnectionState::Quiescing;
            input.cancel_unsent();
        }
    }
    pub fn acknowledge_input_quiescence(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), InputCaptureError> {
        let port = self.port(generation)?;
        if !self.closed {
            return Err(InputCaptureError::Input(InputError::State));
        }
        let source = self.inputs[port].binding.ok_or(InputError::Attachment)?;
        self.session.acknowledge_source_quiescence(source)?;
        self.inputs[port].acknowledge_quiescence(generation)?;
        Ok(())
    }
    pub fn offer(
        &mut self,
        at: SampleTime,
        command: SessionCommand,
    ) -> Result<SessionCommandId, InputCaptureError> {
        Ok(self.session.offer(at, command)?)
    }
    pub fn collect(&mut self) -> Option<SessionReceipt> {
        self.session.take_command_receipt()
    }
    pub fn collect_input(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<Option<InputReceipt>, InputCaptureError> {
        let port = self.port(generation)?;
        Ok(self.inputs[port].take_receipt())
    }
    pub const fn acknowledged(&self) -> LoopSnapshot {
        self.session.acknowledged()
    }
    pub fn render(&mut self, output: AudioBlockMut<'_>) -> Result<(), InputCaptureError> {
        let result = self.session.render(output);
        if self.session.commands.closed {
            self.close_inputs();
        }
        Ok(result?)
    }
}
