//! Finite packet custody around the serial loop recorder. Concrete queues belong to the host.
mod hot;
#[cfg(test)]
mod tests;

use super::{LoopRecordingSession, LoopSessionError};
use crate::{
    host::{
        ConnectionGeneration,
        session::{
            SessionCommand, SessionCommandId, SessionError, SessionReceipt, SessionSourceAction,
            SessionSourceId, SessionSourceReceipt,
        },
    },
    looping::LoopSnapshot,
    quantities::{EventCount, PreparedBytes},
    time::{QUANTUM_FRAMES, SampleTime},
};
use thiserror::Error;

/// Identifies a transfer occurrence, independently of its eventual serial-lane identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopTransferId {
    generation: ConnectionGeneration,
    serial: u64,
}
impl LoopTransferId {
    pub const fn generation(self) -> ConnectionGeneration {
        self.generation
    }
    pub const fn serial(self) -> u64 {
        self.serial
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Action {
    Command(SampleTime, SessionCommand),
    Source(SessionSourceAction),
}

/// Ownership must reach audio or return to control cancellation after a failed send.
/// Dropping a packet does not return its reserved credit.
#[derive(Debug)]
#[must_use]
pub struct LoopTransferPacket {
    id: LoopTransferId,
    action: Action,
}
impl LoopTransferPacket {
    pub(super) const fn is_source(&self) -> bool {
        matches!(self.action, Action::Source(_))
    }

    pub const fn id(&self) -> LoopTransferId {
        self.id
    }
}

#[derive(Debug)]
pub enum LoopTransferOutcome {
    Command(SessionReceipt),
    Source(SessionSourceReceipt),
    Refused(LoopSessionError),
}

/// An identified result remains a credit until explicit off-thread collection.
#[derive(Debug)]
#[must_use]
pub struct LoopTransferCompletion {
    packet: LoopTransferPacket,
    outcome: LoopTransferOutcome,
    snapshot: LoopSnapshot,
}
impl LoopTransferCompletion {
    pub(super) const fn is_source(&self) -> bool {
        self.packet.is_source()
    }

    pub const fn id(&self) -> LoopTransferId {
        self.packet.id
    }
    pub const fn outcome(&self) -> &LoopTransferOutcome {
        &self.outcome
    }
    pub const fn snapshot(&self) -> LoopSnapshot {
        self.snapshot
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum LoopTransferError {
    #[error("packet or owner belongs to another loop session")]
    Origin,
    #[error("transfer identity did not increase")]
    Order,
    #[error("no transfer cell is available")]
    Full,
    #[error("loop transfer admission is closed")]
    Closed,
    #[error("transfer has no matching outstanding credit")]
    Unknown,
    #[error("transfer outcomes remain outstanding")]
    Outstanding,
}

#[derive(Clone, Copy)]
struct Credit {
    id: LoopTransferId,
    action: Action,
}

enum Delivery {
    Command(SessionCommandId),
    Source(SessionSourceId),
    Refused(LoopSessionError),
}
type PendingSlots = Box<[Option<Pending>]>;

struct Pending {
    packet: LoopTransferPacket,
    delivery: Delivery,
}

/// Off-thread preparation and credit collection. One ordered source merger publishes
/// through this owner; independent producers cannot infer a common fence from queue order.
#[must_use]
pub struct LoopRecordingControl {
    generation: ConnectionGeneration,
    credits: Box<[Option<Credit>]>,
    command_capacity: usize,
    source_capacity: usize,
    serial: u64,
    last_command: Option<SampleTime>,
    last_source: Option<(SampleTime, bool)>,
    play_offered: bool,
    start: SampleTime,
    closed: bool,
    share: EventCount,
    snapshot: LoopSnapshot,
    bytes: PreparedBytes,
}

/// Sole owner of the preallocated renderer, journal and raw recorder. The host retains
/// this owner and queue backing until callback access has joined, then moves it off-thread.
#[must_use]
pub struct LoopRecordingAudio {
    generation: ConnectionGeneration,
    session: LoopRecordingSession,
    pending: PendingSlots,
    last_serial: u64,
}

/// A refused split preserves the original complete session.
#[derive(Error)]
#[error("loop transfer preparation failed: {error}")]
pub struct LoopSplitError {
    session: LoopRecordingSession,
    error: LoopSessionError,
}
impl std::fmt::Debug for LoopSplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopSplitError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl LoopSplitError {
    pub const fn error(&self) -> &LoopSessionError {
        &self.error
    }
    #[must_use = "The returned owners retain recording and transfer custody"]
    pub fn into_parts(self) -> (LoopRecordingSession, LoopSessionError) {
        (self.session, self.error)
    }
}

/// A refused reunion retains both owners; no recording or outstanding credit is discarded.
#[derive(Error)]
#[error("loop transfer reunion failed: {error}")]
pub struct LoopReuniteError {
    control: LoopRecordingControl,
    audio: LoopRecordingAudio,
    error: LoopTransferError,
}
impl std::fmt::Debug for LoopReuniteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopReuniteError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl LoopReuniteError {
    pub const fn error(&self) -> LoopTransferError {
        self.error
    }
    #[must_use = "The returned owners retain recording and transfer custody"]
    pub fn into_parts(self) -> (LoopRecordingControl, LoopRecordingAudio, LoopTransferError) {
        (self.control, self.audio, self.error)
    }
}

impl LoopRecordingSession {
    /// Split only an untouched session with empty command/source lanes, off-thread.
    /// Budget covers both wrapper layouts and fixed credit/mapping allocations;
    /// existing session storage and the concrete host's queues remain separately charged.
    pub fn split(
        self,
        budget: PreparedBytes,
    ) -> Result<(LoopRecordingControl, LoopRecordingAudio), Box<LoopSplitError>> {
        let prepared = LoopRecordingControl::prepare(&self, budget);
        match prepared {
            Ok((control, pending)) => {
                let audio = LoopRecordingAudio {
                    generation: control.generation,
                    session: self,
                    pending,
                    last_serial: 0,
                };
                Ok((control, audio))
            }
            Err(error) => Err(Box::new(LoopSplitError {
                session: self,
                error,
            })),
        }
    }
}

impl LoopRecordingControl {
    fn prepare(
        session: &LoopRecordingSession,
        budget: PreparedBytes,
    ) -> Result<(Self, PendingSlots), LoopSessionError> {
        if session.commands.serial != 0
            || session.commands.closed
            || session.sources.has_held()
            || !session.capture.journal.is_fresh()
        {
            return Err(LoopSessionError::FreshCapture);
        }
        let command_capacity = session.commands.slots.len();
        let source_capacity = session.sources.capacity();
        let count = command_capacity
            .checked_add(source_capacity)
            .ok_or(SessionError::Layout)?;
        let bytes = count
            .checked_mul(size_of::<Option<Credit>>() + size_of::<Option<Pending>>())
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .and_then(|bytes| {
                bytes.checked_add(
                    size_of::<LoopRecordingAudio>() - size_of::<LoopRecordingSession>(),
                )
            })
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(SessionError::Layout)?;
        let bytes = PreparedBytes::measured(bytes);
        if bytes > budget {
            return Err(SessionError::ByteBudget {
                required: bytes,
                available: budget,
            }
            .into());
        }
        let mut credits = Vec::new();
        credits
            .try_reserve_exact(count)
            .map_err(|_| SessionError::Allocation)?;
        credits.resize_with(count, || None);
        let mut pending = Vec::new();
        pending
            .try_reserve_exact(count)
            .map_err(|_| SessionError::Allocation)?;
        pending.resize_with(count, || None);
        Ok((
            Self {
                // A reunited untouched session can split again. Its serial-lane generation
                // persists, but a new transfer namespace must never reuse historical IDs.
                generation: crate::host::issue_capture_source_generation()?,
                credits: credits.into_boxed_slice(),
                command_capacity,
                source_capacity,
                serial: 0,
                last_command: None,
                last_source: None,
                play_offered: false,
                start: session.capture.start,
                closed: false,
                share: session.commands.share,
                snapshot: session.acknowledged(),
                bytes,
            },
            pending.into_boxed_slice(),
        ))
    }

    pub const fn transfer_bytes(&self) -> PreparedBytes {
        self.bytes
    }
    pub const fn snapshot(&self) -> LoopSnapshot {
        self.snapshot
    }
    pub fn has_outstanding(&self) -> bool {
        self.credits.iter().any(Option::is_some)
    }
    pub fn close_admission(&mut self) {
        self.closed = true;
    }

    fn prepare_packet(&mut self, action: Action) -> Result<LoopTransferPacket, LoopSessionError> {
        if self.closed {
            return Err(SessionError::Closed.into());
        }
        let index = self
            .credits
            .iter()
            .position(Option::is_none)
            .ok_or(SessionError::Full)?;
        let serial = self
            .serial
            .checked_add(1)
            .ok_or(SessionError::IdentityExhausted)?;
        let id = LoopTransferId {
            generation: self.generation,
            serial,
        };
        self.credits[index] = Some(Credit { id, action });
        self.serial = serial;
        Ok(LoopTransferPacket { id, action })
    }

    pub fn prepare_command(
        &mut self,
        at: SampleTime,
        command: SessionCommand,
    ) -> Result<LoopTransferPacket, LoopSessionError> {
        if at < self.snapshot.clock
            || !at.as_u64().is_multiple_of(u64::from(QUANTUM_FRAMES))
            || self.last_command.is_some_and(|last| at < last)
        {
            return Err(SessionError::Boundary.into());
        }
        if command == SessionCommand::Play && (self.play_offered || at != self.start) {
            return Err(LoopSessionError::FinitePlay);
        }
        let mut held = 0;
        let mut same_time = 1_u64;
        for credit in self.credits.iter().flatten() {
            if let Action::Command(time, _) = credit.action {
                held += 1;
                same_time += u64::from(time == at);
            }
        }
        if held >= self.command_capacity - usize::from(command == SessionCommand::Play) {
            return Err(SessionError::Full.into());
        }
        if same_time > u64::from(self.share.get()) {
            return Err(SessionError::SessionShare.into());
        }
        let packet = self.prepare_packet(Action::Command(at, command))?;
        self.last_command = Some(at);
        self.play_offered |= command == SessionCommand::Play;
        Ok(packet)
    }

    pub fn prepare_source(
        &mut self,
        action: SessionSourceAction,
    ) -> Result<LoopTransferPacket, LoopSessionError> {
        if action.epoch() != self.snapshot.epoch {
            return Err(LoopSessionError::Capture(
                super::super::LoopCaptureError::Capture(
                    crate::recording::notes::NoteCaptureError::ForeignEpoch,
                ),
            ));
        }
        let at = action.at();
        let published = matches!(action, SessionSourceAction::Publish { .. });
        if self.last_source.is_some_and(|(last, was_published)| {
            at < last || (at == last && was_published && !published)
        }) {
            return Err(SessionError::SourceOrder.into());
        }
        let held = self
            .credits
            .iter()
            .flatten()
            .filter(|c| matches!(c.action, Action::Source(_)))
            .count();
        if held >= self.source_capacity {
            return Err(SessionError::Full.into());
        }
        let packet = self.prepare_packet(Action::Source(action))?;
        self.last_source = Some((at, published));
        Ok(packet)
    }

    fn credit(&self, packet: &LoopTransferPacket) -> Result<usize, LoopTransferError> {
        if packet.id.generation != self.generation {
            return Err(LoopTransferError::Origin);
        }
        self.credits
            .iter()
            .position(|credit| {
                credit.is_some_and(|c| c.id == packet.id && c.action == packet.action)
            })
            .ok_or(LoopTransferError::Unknown)
    }

    pub fn cancel(
        &mut self,
        packet: LoopTransferPacket,
    ) -> Result<LoopTransferId, (LoopTransferPacket, LoopTransferError)> {
        match self.credit(&packet) {
            Ok(index) => {
                self.credits[index] = None;
                Ok(packet.id)
            }
            Err(error) => Err((packet, error)),
        }
    }

    pub fn collect(
        &mut self,
        completion: LoopTransferCompletion,
    ) -> Result<
        (LoopTransferId, LoopTransferOutcome),
        Box<(LoopTransferCompletion, LoopTransferError)>,
    > {
        let index = match self.credit(&completion.packet) {
            Ok(index) => index,
            Err(error) => return Err(Box::new((completion, error))),
        };
        self.credits[index] = None;
        if completion.snapshot.clock >= self.snapshot.clock {
            self.snapshot = completion.snapshot;
        }
        Ok((completion.packet.id, completion.outcome))
    }
}

impl LoopRecordingAudio {
    /// Only after the host has joined callback access. This freezes capture and retains
    /// pending outcomes; it does not acknowledge independent source quiescence.
    pub fn device_lost_after_callback_join(&mut self) -> Result<(), LoopSessionError> {
        self.session.device_lost()
    }

    /// Recover the serial owner on the finalization worker after callbacks have joined.
    /// Every queued, unpublished, runtime and returned packet must first be resolved.
    /// Delayed source fences, source retirement and finalization then use the serial API.
    pub fn reunite(
        self,
        control: LoopRecordingControl,
    ) -> Result<LoopRecordingSession, Box<LoopReuniteError>> {
        let error = if control.generation != self.generation {
            Some(LoopTransferError::Origin)
        } else if control.has_outstanding()
            || self.pending.iter().any(Option::is_some)
            || self.session.commands.held != 0
            || self.session.sources.has_held()
        {
            Some(LoopTransferError::Outstanding)
        } else {
            None
        };
        if let Some(error) = error {
            return Err(Box::new(LoopReuniteError {
                control,
                audio: self,
                error,
            }));
        }
        Ok(self.session)
    }
}
