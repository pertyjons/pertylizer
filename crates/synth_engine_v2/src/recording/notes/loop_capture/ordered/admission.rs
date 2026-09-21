//! Fixed-storage admission and receipt moves shared by serial and split owners.

use super::{CommandEntry, LoopCaptureError, LoopRecordingSession, LoopSessionError};
use crate::{
    host::{
        ConnectionGeneration,
        session::{
            SessionBoundary, SessionCommand, SessionCommandId, SessionError, SessionReceipt,
            SessionSourceAction, SessionSourceId, SessionSourceReceipt,
        },
    },
    time::{QUANTUM_FRAMES, SampleTime},
};

impl LoopRecordingSession {
    pub const fn generation(&self) -> ConnectionGeneration {
        self.commands.generation
    }

    pub fn offer(
        &mut self,
        at: SampleTime,
        command: SessionCommand,
    ) -> Result<SessionCommandId, LoopSessionError> {
        let lane = &mut self.commands;
        if lane.closed {
            return Err(LoopSessionError::Session(SessionError::Closed));
        }
        if at < self.capture.acknowledged().clock
            || !at.as_u64().is_multiple_of(u64::from(QUANTUM_FRAMES))
            || lane.last_offer.is_some_and(|last| at < last)
        {
            return Err(LoopSessionError::Session(SessionError::Boundary));
        }
        if command == SessionCommand::Play
            && (lane.play_offered || lane.stopped || at != self.capture.start)
        {
            return Err(LoopSessionError::FinitePlay);
        }
        let reserve = usize::from(command == SessionCommand::Play);
        if lane.held >= lane.slots.len() - reserve {
            return Err(LoopSessionError::Session(SessionError::Full));
        }
        let mut charge = 1_u64;
        for offset in lane.completed..lane.held {
            let index = (lane.head + offset) % lane.slots.len();
            if lane
                .slots
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|entry| entry.boundary.at == at)
            {
                charge += 1;
            }
        }
        if charge > u64::from(lane.share.get()) {
            return Err(LoopSessionError::Session(SessionError::SessionShare));
        }
        let id = SessionCommandId::after(lane.generation, lane.serial)?;
        let serial = id.serial();
        let index = (lane.head + lane.held) % lane.slots.len();
        lane.slots[index] = Some(CommandEntry {
            boundary: SessionBoundary {
                id,
                at,
                command,
                capture: self.capture.ticket,
            },
            outcome: None,
            capture: None,
        });
        lane.held += 1;
        lane.serial = serial;
        lane.last_offer = Some(at);
        lane.play_offered |= command == SessionCommand::Play;
        Ok(id)
    }

    pub fn offer_source(
        &mut self,
        action: SessionSourceAction,
    ) -> Result<SessionSourceId, LoopSessionError> {
        if self.commands.closed {
            return Err(LoopSessionError::Session(SessionError::Closed));
        }
        let _sequence = self.capture.source_sequence(action.source())?;
        if action.epoch() != self.capture.initial().epoch {
            return Err(LoopSessionError::Capture(LoopCaptureError::Capture(
                crate::recording::notes::NoteCaptureError::ForeignEpoch,
            )));
        }
        // Stopped workers may deliver a delayed fence for the stopped endpoint.
        // SourceQueue and the recorder still enforce consumed FIFO/frontier order.
        let earliest = if self.commands.stopped {
            SampleTime::ZERO
        } else {
            self.acknowledged().clock
        };
        Ok(self.sources.offer(earliest, action)?)
    }

    pub(crate) fn next_command_receipt_id(&self) -> Option<SessionCommandId> {
        let lane = &self.commands;
        if lane.completed == 0 {
            return None;
        }
        let entry = lane.slots.get(lane.head)?.as_ref()?;
        entry.outcome?;
        Some(entry.boundary.id)
    }
    pub(crate) fn next_source_receipt_id(&self) -> Option<SessionSourceId> {
        self.sources.next_receipt_id()
    }
    pub(crate) fn take_command_receipt(&mut self) -> Option<SessionReceipt> {
        let lane = &mut self.commands;
        if lane.completed == 0 {
            return None;
        }
        let entry = lane.slots.get_mut(lane.head)?.as_mut()?;
        let outcome = entry.outcome?;
        let receipt = SessionReceipt {
            boundary: entry.boundary,
            outcome,
            capture: entry.capture.take(),
        };
        lane.slots[lane.head] = None;
        lane.head = (lane.head + 1) % lane.slots.len();
        lane.held -= 1;
        lane.completed -= 1;
        Some(receipt)
    }

    pub(crate) fn take_source_receipt(&mut self) -> Option<SessionSourceReceipt> {
        self.sources.take_receipt()
    }
}
