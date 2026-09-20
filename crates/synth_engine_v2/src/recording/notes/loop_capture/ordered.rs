//! A finite simulated session owns ordered transport, loop observations and capture.

mod admission;
mod hot;
pub mod input;
pub mod transfer;

use super::{LoopCaptureError, LoopCaptureSession};
use crate::{
    host::{
        ConnectionGeneration,
        session::{
            SessionBoundary, SessionCaptureOutcome, SessionError, SessionLimits, SessionOutcome,
            SessionReceipt, SessionSourceLimits, SessionSourceReceipt, source::SourceQueue,
        },
    },
    looping::LoopSnapshot,
    quantities::{EventCount, PreparedBytes},
    recording::{CaptureQuality, notes::NoteCaptureResult},
    time::SampleTime,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoopSessionError {
    #[error("ordered loop recording requires an armed, unstarted, fresh loop owner")]
    FreshCapture,
    #[error("this finite loop session admits only one Play at its initial clock")]
    FinitePlay,
    #[error("source draining requires a stopped session")]
    NotStopped,
    #[error("source retirement requires closed session admission")]
    NotClosed,
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Capture(#[from] LoopCaptureError),
    #[error(transparent)]
    Host(#[from] crate::host::HostError),
}

/// A failed conversion returns the complete original owner, including any take.
#[derive(Error)]
#[error("ordered loop session preparation failed: {error}")]
pub struct LoopSessionPrepareError {
    capture: LoopCaptureSession,
    error: LoopSessionError,
}

impl std::fmt::Debug for LoopSessionPrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopSessionPrepareError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl LoopSessionPrepareError {
    pub const fn error(&self) -> &LoopSessionError {
        &self.error
    }

    #[must_use = "The returned owner retains the recording and must be resolved explicitly"]
    pub fn into_parts(self) -> (LoopCaptureSession, LoopSessionError) {
        (self.capture, self.error)
    }
}

struct CommandEntry {
    boundary: SessionBoundary,
    outcome: Option<SessionOutcome>,
    capture: Option<SessionCaptureOutcome>,
}

struct CommandLane {
    generation: ConnectionGeneration,
    slots: Box<[Option<CommandEntry>]>,
    head: usize,
    held: usize,
    completed: usize,
    serial: u64,
    last_offer: Option<SampleTime>,
    play_offered: bool,
    playing: bool,
    stopped: bool,
    closed: bool,
    share: EventCount,
    bytes: PreparedBytes,
}

/// One armed take and one recording Play. Sources and commands have identified,
/// retained outcomes. Preparation, finalization and destruction are off-thread.
/// No mutable renderer, recorder or journal can escape this exclusive serial owner.
#[must_use]
pub struct LoopRecordingSession {
    capture: LoopCaptureSession,
    commands: CommandLane,
    sources: SourceQueue,
}

impl LoopRecordingSession {
    /// Convert an unused armed fixture off-thread. Every refusal returns its original
    /// owner. Error boxing is off-thread and does not spend a render/capture budget.
    pub fn prepare(
        capture: LoopCaptureSession,
        commands: SessionLimits,
        sources: SessionSourceLimits,
    ) -> Result<Self, Box<LoopSessionPrepareError>> {
        match Self::prepare_lanes(&capture, commands, sources) {
            Ok((commands, sources)) => Ok(Self {
                capture,
                commands,
                sources,
            }),
            Err(error) => Err(Box::new(LoopSessionPrepareError { capture, error })),
        }
    }

    fn prepare_lanes(
        capture: &LoopCaptureSession,
        commands: SessionLimits,
        sources: SessionSourceLimits,
    ) -> Result<(CommandLane, SourceQueue), LoopSessionError> {
        if capture.ticket.is_none() || capture.started || !capture.journal.is_fresh() {
            return Err(LoopSessionError::FreshCapture);
        }
        let generation = crate::host::issue_capture_source_generation()?;
        let sources = SourceQueue::prepare(generation, sources)?;
        let count =
            usize::try_from(commands.commands.as_u32()).map_err(|_| SessionError::Layout)?;
        // The capture and source budgets already charge their inline owners.
        let extra = size_of::<Self>() - size_of::<LoopCaptureSession>() - size_of::<SourceQueue>();
        let bytes = count
            .checked_mul(size_of::<Option<CommandEntry>>())
            .and_then(|bytes| bytes.checked_add(extra))
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(SessionError::Layout)?;
        let bytes = PreparedBytes::measured(bytes);
        if bytes > commands.command_bytes {
            return Err(SessionError::ByteBudget {
                required: bytes,
                available: commands.command_bytes,
            }
            .into());
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| SessionError::Allocation)?;
        slots.resize_with(count, || None);
        let lane = CommandLane {
            generation,
            slots: slots.into_boxed_slice(),
            head: 0,
            held: 0,
            completed: 0,
            serial: 0,
            last_offer: None,
            play_offered: false,
            playing: false,
            stopped: false,
            closed: false,
            share: capture.journal.session_share(),
            bytes,
        };
        Ok((lane, sources))
    }

    pub const fn command_bytes(&self) -> PreparedBytes {
        self.commands.bytes
    }

    pub fn source_bytes(&self) -> PreparedBytes {
        self.sources.bytes()
    }

    pub const fn acknowledged(&self) -> LoopSnapshot {
        self.capture.acknowledged()
    }

    pub const fn observation_end(&self) -> Option<crate::looping::journal::LoopJournalEnd> {
        self.capture.observation_end()
    }

    pub fn collect(&mut self) -> Option<SessionReceipt> {
        self.take_command_receipt()
    }
    pub fn collect_source(&mut self) -> Option<SessionSourceReceipt> {
        self.take_source_receipt()
    }

    pub fn finalize(&mut self) -> Result<(), LoopSessionError> {
        Ok(self.capture.finalize()?)
    }

    /// Close publication after a result has sealed normally, so its sources can
    /// retire quality custody and the owner can explicitly discard the result.
    pub fn close_completed(&mut self) -> Result<(), LoopSessionError> {
        let _sealed = self.capture.result()?;
        self.commands.close();
        self.sources.close();
        Ok(())
    }

    pub fn result(&self) -> Result<NoteCaptureResult<'_>, LoopSessionError> {
        Ok(self.capture.result()?)
    }

    pub fn discard(&mut self, quality: CaptureQuality) -> Result<(), LoopSessionError> {
        if self.commands.held != 0 || self.sources.has_held() {
            return Err(SessionError::RetainedOutcomes.into());
        }
        Ok(self.capture.discard(quality)?)
    }
}
