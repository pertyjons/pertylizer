//! Borrowed terminal signal and fixed-storage callback delegation.
use super::{InputCaptureAudio, InputCaptureHalt};
use crate::{
    host::session::{
        LoopSessionError,
        loop_transfer::{LoopTransferCompletion, LoopTransferError, LoopTransferPacket},
    },
    looping::LoopSnapshot,
    recording::notes::CaptureStopReason,
    render::AudioBlockMut,
};
use std::sync::atomic::Ordering;

impl InputCaptureHalt {
    fn request(&self, reason: CaptureStopReason) {
        let code = match reason {
            CaptureStopReason::Stop => 1,
            CaptureStopReason::DeviceLost => 2,
            _ => 3,
        };
        // A failed exchange means another terminal request already won. It is
        // deliberately harmless: this finite signal never resets or replaces it.
        let _first_request = self
            .signal
            .compare_exchange(0, code, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
    }
    pub fn is_requested(&self) -> bool {
        self.signal.load(Ordering::Acquire) != 0
    }
    pub fn request_stop(&self) {
        self.request(CaptureStopReason::Stop);
    }
    pub fn request_device_lost(&self) {
        self.request(CaptureStopReason::DeviceLost);
    }
    /// A host delivery failure interrupts capture without impersonating user Stop.
    pub fn request_invalid(&self) {
        self.request(CaptureStopReason::SourceInvalid);
    }
    pub(super) fn reason(&self) -> Option<CaptureStopReason> {
        match self.signal.load(Ordering::Acquire) {
            0 => None,
            1 => Some(CaptureStopReason::Stop),
            2 => Some(CaptureStopReason::DeviceLost),
            _ => Some(CaptureStopReason::SourceInvalid),
        }
    }
}

impl InputCaptureAudio {
    /// Reconcile mutable raw metadata before returning a live outcome credit.
    pub fn resolve_audition(
        &mut self,
        id: crate::host::live::AuditionId,
        outcome: crate::host::live::AuditionOutcome,
    ) -> Result<crate::quantities::EventCount, LoopSessionError> {
        self.core.resolve_audition(id, outcome)
    }

    pub fn applied_end(
        &self,
    ) -> Option<(
        crate::time::SampleTime,
        crate::host::session::SessionCommand,
    )> {
        self.core.applied_end()
    }

    pub const fn acknowledged(&self) -> LoopSnapshot {
        self.core.acknowledged()
    }

    /// Poll also after callback access joins when no final callback is delivered.
    pub fn synchronize_halt(&mut self) -> Result<(), LoopSessionError> {
        if let Some(reason) = self.halt.reason() {
            self.core.halt(reason)?;
        }
        Ok(())
    }

    pub fn enqueue(
        &mut self,
        packet: LoopTransferPacket,
    ) -> Result<(), (LoopTransferPacket, LoopTransferError)> {
        // Refuse new custody immediately. Render or joined synchronize_halt applies
        // the terminal transition and reports any recorder error to its caller.
        if self.halt.reason().is_some() {
            return Err((packet, LoopTransferError::Closed));
        }
        self.core.enqueue(packet)
    }

    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), LoopSessionError> {
        if let Err(error) = self.synchronize_halt() {
            output.silence();
            return Err(error);
        }
        let result = self.core.render(output);
        if self.core.is_closed() {
            // Audio can fault independently of all producers. Publish closure
            // without replacing the recorder's existing LoopRenderFault reason.
            self.halt.request_invalid();
        }
        result
    }

    pub fn take_completed(&mut self) -> Option<LoopTransferCompletion> {
        self.core.take_completed()
    }

    pub fn drain_stopped_sources(&mut self) -> Result<(), LoopSessionError> {
        self.synchronize_halt()?;
        self.core.drain_stopped_sources()
    }
}
