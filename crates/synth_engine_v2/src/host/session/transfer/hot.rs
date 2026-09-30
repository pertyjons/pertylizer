//! Allocation-free packet admission, rendering and owning completion transfer.

use super::{
    CompletedSessionCommand, PreparedSessionCommand, SessionAudio, SessionDeliveryError,
    SessionSnapshot, SessionTransferError,
};
use crate::host::session::{PlaybackState, SessionCommand};
use crate::plan::PlanId;
use crate::quantities::EventCount;
use crate::render::AudioBlockMut;
use crate::schedule::ScheduledRenderError;
use crate::time::{SampleTime, StreamEpoch};

impl SessionAudio {
    pub const fn clock(&self) -> SampleTime {
        self.renderer.clock()
    }
    pub const fn state(&self) -> PlaybackState {
        self.runtime.state
    }
    pub const fn plan_id(&self) -> PlanId {
        self.origin.plan
    }
    pub const fn epoch(&self) -> StreamEpoch {
        self.origin.epoch
    }
    pub const fn has_retained_commands(&self) -> bool {
        self.runtime.held != 0
    }
    /// The stream's high water for one producer class, never reset (EVD-0025).
    pub const fn high_water(&self, class: crate::publish::ProducerClass) -> EventCount {
        self.runtime.arbiter.high_water(class)
    }
    /// The stream's high water of all external classes in one window, never reset.
    pub const fn high_water_external_total(&self) -> EventCount {
        self.runtime.arbiter.high_water_external_total()
    }
    /// Publication faults the renderer has counted, which a completed run must not have.
    pub const fn publication_faults(&self) -> u64 {
        self.renderer.diagnostics().publication_faults()
    }

    /// A host establishes its ingress cut before rendering a callback. Late delivery
    /// is retained for boundary refusal; a protocol error returns the owning packet.
    pub fn enqueue(
        &mut self,
        mut packet: PreparedSessionCommand,
    ) -> Result<(), (PreparedSessionCommand, SessionTransferError)> {
        if packet.origin != self.origin {
            return Err((packet, SessionTransferError::Origin));
        }
        if self.runtime.closed || self.fault.is_some() {
            return Err((packet, SessionTransferError::Closed));
        }
        let boundary = packet.entry.boundary;
        if boundary.id.serial() <= self.runtime.serial
            || self
                .runtime
                .last_offer
                .is_some_and(|last| boundary.at < last)
        {
            return Err((packet, SessionTransferError::Order));
        }
        if self.runtime.held >= self.runtime.slots.len() {
            return Err((packet, SessionTransferError::Full));
        }
        let index = (self.runtime.head + self.runtime.held) % self.runtime.slots.len();
        let Some(slot) = self.runtime.slots.get_mut(index) else {
            return Err((packet, SessionTransferError::Full));
        };
        if slot.is_some() {
            return Err((packet, SessionTransferError::Full));
        }
        if boundary.at < self.renderer.clock() {
            packet.entry.delivery_error = Some(SessionDeliveryError::Late {
                observed: self.renderer.clock(),
            });
        }
        *slot = Some(packet.entry);
        self.runtime.held += 1;
        self.runtime.serial = boundary.id.serial();
        self.runtime.last_offer = Some(boundary.at);
        Ok(())
    }

    /// All due commands use the same quantum boundary machinery as the serial host.
    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), ScheduledRenderError> {
        if let Some(error) = self.fault {
            output.silence();
            return Err(error);
        }
        if self.runtime.closed {
            output.silence();
            return Ok(());
        }
        let result = self
            .runtime
            .render(&mut self.renderer, output.reborrow(), None);
        if let Err(error) = result {
            output.silence();
            self.fault = Some(error);
        }
        result
    }

    /// Move a completed entry without collecting its activation or freeing its box.
    /// The host checks destination capacity before taking and retains any failed send.
    pub fn take_completed(&mut self) -> Option<CompletedSessionCommand> {
        if self.runtime.completed == 0 {
            return None;
        }
        let entry = self.runtime.slots.get_mut(self.runtime.head)?.take()?;
        self.runtime.head = (self.runtime.head + 1) % self.runtime.slots.len();
        self.runtime.held -= 1;
        self.runtime.completed -= 1;
        if entry.boundary.command == SessionCommand::Play {
            self.runtime.play_outstanding = false;
        }
        Some(CompletedSessionCommand {
            origin: self.origin,
            entry,
            snapshot: SessionSnapshot {
                clock: self.renderer.clock(),
                playback: self.runtime.state,
            },
        })
    }
}
