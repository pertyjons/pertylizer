//! Exclusive callback delegation and fixed-storage loss handling.
use super::super::{
    InputDiscontinuity, InputError, InputEventId, InputObservation, InputReceipt, InputTick,
    SimulatedInputClock, SourceFaultStage,
};
use super::{InputCaptureError, InputCaptureSession, LoopRecordingSession};
use crate::{
    host::{
        ConnectionGeneration,
        session::{LoopSessionError, SessionCommand, SessionCommandId, SessionReceipt},
    },
    looping::LoopSnapshot,
    recording::notes::{
        CaptureStamp, CaptureStopReason, Midi1Input, loop_capture::LoopCaptureError,
    },
    render::AudioBlockMut,
    time::SampleTime,
};

fn attribute_input_fault(
    session: &mut LoopRecordingSession,
    clock: Option<SimulatedInputClock>,
    source: Option<ConnectionGeneration>,
    fault: InputDiscontinuity,
    pre_raw: bool,
) -> Result<(), InputCaptureError> {
    // Exact late observations use their actual stamp. Uncertain observations
    // contribute conservative bounds and never acquire a fabricated exact time.
    if (fault.reason == InputError::Order
        || (pre_raw
            && matches!(
                fault.reason,
                InputError::SourceQueueFull | InputError::IdentityExhausted | InputError::Full
            )))
        && let Some(InputObservation::Message { tick, arrival, .. }) = fault.observation
        && let Some(clock) = clock
        && let Ok(nominal) = clock.map(tick)
        && let Ok(stamp) = CaptureStamp::exact_fixture(clock.epoch, nominal, arrival)
        && let Some(source) = source
    {
        session
            .capture
            .recorder
            .attribute_refused_input(source, stamp)
            .map_err(|error| {
                InputCaptureError::Recording(LoopSessionError::Capture(LoopCaptureError::Capture(
                    error,
                )))
            })?;
    }
    if matches!(fault.reason, InputError::Uncertain | InputError::ClockRange)
        && let Some(InputObservation::Message { tick, .. }) = fault.observation
        && let Some(clock) = clock
        && let Some(source) = source
        && let Some((earliest, latest)) = clock.quality_bounds(tick)
    {
        session
            .capture
            .recorder
            .attribute_uncertain_input(source, earliest, latest)
            .map_err(|error| {
                InputCaptureError::Recording(LoopSessionError::Capture(LoopCaptureError::Capture(
                    error,
                )))
            })?;
    }
    Ok(())
}

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
        for input in &mut self.inputs {
            let primary = input.discontinuity;
            if let Some(fault) = primary
                && fault.reason != InputError::PeerInterrupted
            {
                if !input.discontinuity_attributed {
                    attribute_input_fault(
                        &mut self.session,
                        input.clock,
                        input.binding,
                        fault,
                        input.discontinuity_source_stage.is_some(),
                    )?;
                    input.discontinuity_attributed = true;
                }
                if reason.is_none() {
                    reason = Some(if fault.reason == InputError::DeviceLost {
                        CaptureStopReason::DeviceLost
                    } else {
                        CaptureStopReason::SourceInvalid
                    });
                }
            }
            if let Some(fault) = input.pre_ring_failure {
                if !input.pre_ring_attributed {
                    if input.discontinuity_source_stage != Some(SourceFaultStage::BeforeSourceRing)
                    {
                        attribute_input_fault(
                            &mut self.session,
                            input.clock,
                            input.binding,
                            fault,
                            true,
                        )?;
                    }
                    input.pre_ring_attributed = true;
                }
                if reason.is_none() {
                    reason = Some(CaptureStopReason::SourceInvalid);
                }
            }
            if let Some(fault) = input.post_source_ring_failure {
                if !input.post_source_ring_attributed {
                    if input.discontinuity_source_stage != Some(SourceFaultStage::AfterSourceRing) {
                        attribute_input_fault(
                            &mut self.session,
                            input.clock,
                            input.binding,
                            fault,
                            true,
                        )?;
                    }
                    input.post_source_ring_attributed = true;
                }
                if reason.is_none() {
                    reason = Some(CaptureStopReason::SourceInvalid);
                }
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
            input.quiesce();
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
