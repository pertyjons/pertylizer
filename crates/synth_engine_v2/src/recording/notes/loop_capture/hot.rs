//! Serial operations over fixed storage; no pass reconstruction or source-time merge.
use super::{LoopCaptureError, LoopCaptureSession};
use crate::{
    host::ConnectionGeneration,
    looping::journal::LoopJournalEnd,
    recording::notes::{
        AuditionTrace, CaptureStamp, Midi1Input, PublicationReceipt, PublicationSequence,
    },
    render::AudioBlockMut,
    time::SampleTime,
};

impl LoopCaptureSession {
    pub fn start(&mut self) -> Result<(), LoopCaptureError> {
        if self.started || self.start != self.journal.acknowledged().clock {
            return Err(LoopCaptureError::State);
        }
        self.recorder
            .start(self.ticket.ok_or(LoopCaptureError::State)?)?;
        self.started = true;
        Ok(())
    }
    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), LoopCaptureError> {
        if !self.started {
            output.silence();
            return Err(LoopCaptureError::State);
        }
        Ok(self.journal.render(output)?)
    }
    pub fn publish(
        &mut self,
        source: ConnectionGeneration,
        stamp: CaptureStamp,
        input: Midi1Input,
        audition: AuditionTrace,
    ) -> Result<PublicationReceipt, LoopCaptureError> {
        Ok(self.recorder.publish(source, stamp, input, audition)?)
    }
    pub fn source_sequence(
        &self,
        source: ConnectionGeneration,
    ) -> Result<PublicationSequence, LoopCaptureError> {
        Ok(self.recorder.source_sequence(source)?)
    }

    pub fn fence(
        &mut self,
        source: ConnectionGeneration,
        frontier: SampleTime,
        through: PublicationSequence,
    ) -> Result<(), LoopCaptureError> {
        Ok(self
            .recorder
            .fence(source, self.journal.initial().epoch, frontier, through)?)
    }
    pub fn quiesce(
        &mut self,
        source: ConnectionGeneration,
        last_valid: SampleTime,
    ) -> Result<(), LoopCaptureError> {
        Ok(self
            .recorder
            .quiesce(source, self.journal.initial().epoch, last_valid)?)
    }
    /// Close the exclusive serial observation; no final render invocation is required.
    /// This is not an input-source fence or a concurrent backend join.
    pub fn finish_observation(&mut self) -> LoopJournalEnd {
        self.journal.finish()
    }
}
