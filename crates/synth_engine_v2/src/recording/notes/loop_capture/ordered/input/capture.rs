//! Exclusive composition; allocation, merging, reconciliation and recovery are off-callback.
mod hot;
mod transfer;
use super::super::{LoopRecordingSession, LoopSessionError};
use super::{InputDelivery, InputError, InputOutcome, SimulatedNoteInput};
use crate::{
    host::{
        ConnectionGeneration, ConnectionState,
        session::{SessionError, SessionSourceOutcome},
    },
    quantities::PreparedBytes,
    recording::notes::{CaptureDisposition, NoteCaptureResult},
};
use thiserror::Error;
pub use transfer::*;

#[derive(Debug, Error)]
pub enum InputCaptureError {
    #[error(transparent)]
    Input(#[from] InputError),
    #[error(transparent)]
    Recording(#[from] LoopSessionError),
    #[error(transparent)]
    Transfer(#[from] super::super::transfer::LoopTransferError),
}

/// Failed attachment or release retains both the take and every input observation.
#[derive(Error)]
#[error("input composition refused: {error}")]
pub struct InputCaptureOwnerError {
    session: LoopRecordingSession,
    inputs: Box<[SimulatedNoteInput]>,
    error: InputCaptureError,
}
impl std::fmt::Debug for InputCaptureOwnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputCaptureOwnerError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl InputCaptureOwnerError {
    pub const fn error(&self) -> &InputCaptureError {
        &self.error
    }
    #[must_use = "The returned owners retain the take and input observations"]
    pub fn into_parts(
        self,
    ) -> (
        LoopRecordingSession,
        Box<[SimulatedNoteInput]>,
        InputCaptureError,
    ) {
        (self.session, self.inputs, self.error)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct InputPortOrder(usize);

/// One selection rule for serial admission and threaded packet preparation.
fn next_source(
    inputs: &[SimulatedNoteInput],
) -> Result<
    Option<(
        InputPortOrder,
        usize,
        crate::host::session::SessionSourceAction,
    )>,
    InputError,
> {
    if inputs
        .iter()
        .any(|input| input.state != ConnectionState::Running)
    {
        return Err(InputError::State);
    }
    let frontier = inputs
        .iter()
        .map(|input| input.frontier)
        .min()
        .ok_or(InputError::Attachment)?;
    let mut selected = None;
    for (port, input) in inputs.iter().enumerate() {
        for (slot, entry) in input.slots.iter().enumerate() {
            let Some(entry) = entry else {
                continue;
            };
            if entry.forwarded.is_some() || entry.outcome.is_some() {
                continue;
            }
            let action = input.action(entry).ok_or(InputError::Attachment)?;
            let message = matches!(
                action,
                crate::host::session::SessionSourceAction::Publish { .. }
            );
            if action.at() > frontier || (message && action.at() == frontier) {
                continue;
            }
            // Array order is only this prepared merger's deterministic tie-break.
            let key = (action.at(), message, InputPortOrder(port), entry.id.serial);
            if selected.as_ref().is_none_or(|(prior, _, _)| key < *prior) {
                selected = Some((key, slot, action));
            }
        }
    }
    Ok(selected.map(|((_, _, port, _), slot, action)| (port, slot, action)))
}

/// A finite serial oracle. No concurrent callback access or physical fence is implied.
/// Input owners cannot escape while they still participate in this retained take.
#[must_use]
pub struct InputCaptureSession {
    session: LoopRecordingSession,
    inputs: Box<[SimulatedNoteInput]>,
    closed: bool,
    bytes: PreparedBytes,
}
impl InputCaptureSession {
    pub fn prepare(
        session: LoopRecordingSession,
        inputs: Box<[SimulatedNoteInput]>,
        bytes: PreparedBytes,
    ) -> Result<Self, Box<InputCaptureOwnerError>> {
        match Self::validate(&session, &inputs, bytes) {
            Ok(bytes) => Ok(Self {
                session,
                inputs,
                closed: false,
                bytes,
            }),
            Err(error) => Err(Box::new(InputCaptureOwnerError {
                session,
                inputs,
                error,
            })),
        }
    }
    fn validate(
        session: &LoopRecordingSession,
        inputs: &[SimulatedNoteInput],
        bytes: PreparedBytes,
    ) -> Result<PreparedBytes, InputCaptureError> {
        if session.commands.held != 0
            || session.commands.play_offered
            || session.commands.closed
            || session.sources.has_held()
            || !session.capture.journal.is_fresh()
            || inputs.is_empty()
        {
            return Err(InputError::Attachment.into());
        }
        let ticket = session.capture.ticket.ok_or(InputError::Attachment)?;
        let selected = &session.capture.recorder.store.slots[ticket.slot].sources;
        if selected.iter().flatten().count() != inputs.len() {
            return Err(InputError::Attachment.into());
        }
        for (index, input) in inputs.iter().enumerate() {
            if input.state != ConnectionState::Ready
                || input.serial != 1
                || input
                    .clock
                    .is_none_or(|clock| clock.epoch != session.capture.initial().epoch)
                || !selected
                    .iter()
                    .flatten()
                    .any(|source| Some(source.generation) == input.binding)
                || inputs[..index]
                    .iter()
                    .any(|prior| prior.binding == input.binding)
            {
                return Err(InputError::Attachment.into());
            }
            if session.capture.start != session.capture.initial().clock {
                let _start_tick = input
                    .clock
                    .ok_or(InputError::Attachment)?
                    .exact_tick(session.capture.start)?;
            }
        }
        // Each input charges its inline owner, including the boxed-array payload.
        // The serial session charges itself; only composition metadata is additional.
        let required = PreparedBytes::measured(
            u64::try_from(size_of::<Self>() - size_of::<LoopRecordingSession>())
                .map_err(|_| InputError::Layout)?,
        );
        if required > bytes {
            return Err(InputError::ByteBudget {
                required,
                available: bytes,
            }
            .into());
        }
        Ok(required)
    }
    pub const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
    pub fn input(&self, generation: ConnectionGeneration) -> Option<&SimulatedNoteInput> {
        self.inputs
            .iter()
            .find(|input| input.generation == Some(generation))
    }
    pub fn result(&self) -> Result<NoteCaptureResult<'_>, InputCaptureError> {
        Ok(self.session.result()?)
    }

    pub fn resolve_audition(
        &mut self,
        id: crate::host::live::AuditionId,
        outcome: crate::host::live::AuditionOutcome,
    ) -> Result<crate::quantities::EventCount, InputCaptureError> {
        Ok(self.session.resolve_audition(id, outcome)?)
    }

    pub fn project_notes(
        &mut self,
    ) -> Result<
        crate::recording::notes::projection::ProjectedLoopTake<'_>,
        crate::recording::notes::projection::ProjectionError,
    > {
        self.session.project_notes()
    }
    pub fn finalize(&mut self) -> Result<(), InputCaptureError> {
        self.reconcile()?;
        Ok(self.session.finalize()?)
    }
    pub fn close_completed(&mut self) -> Result<(), InputCaptureError> {
        self.session.close_completed()?;
        self.close_inputs();
        self.reconcile()?;
        Ok(())
    }
    pub fn into_parts(
        self,
    ) -> Result<(LoopRecordingSession, Box<[SimulatedNoteInput]>), Box<InputCaptureOwnerError>>
    {
        if !self.retirement_ready() {
            return Err(Box::new(InputCaptureOwnerError {
                session: self.session,
                inputs: self.inputs,
                error: InputError::Retained.into(),
            }));
        }
        Ok((self.session, self.inputs))
    }

    /// All transfer outcomes have been collected and every source explicitly joined.
    pub fn retirement_ready(&self) -> bool {
        self.closed
            && self.session.commands.held == 0
            && !self.session.sources.has_held()
            && self
                .inputs
                .iter()
                .all(|input| input.quiescent && input.slots.iter().all(Option::is_none))
    }

    /// Advance only an explicitly complete prefix. SourceQueue saturation leaves
    /// the original cell queued. Fences sort before messages at the same arrival.
    pub fn pump(&mut self) -> Result<(), InputCaptureError> {
        self.reconcile()?;
        if self.closed {
            return Ok(());
        }
        while let Some((InputPortOrder(port), slot, action)) = next_source(&self.inputs)? {
            match self.session.offer_source(action) {
                Ok(id) => {
                    let entry = self.inputs[port].slots[slot]
                        .as_mut()
                        .ok_or(InputError::ReceiptOwner)?;
                    entry.forwarded = Some(InputDelivery::Serial(id));
                }
                Err(LoopSessionError::Session(SessionError::Full)) => break,
                Err(error) => {
                    let entry = self.inputs[port].slots[slot]
                        .as_mut()
                        .ok_or(InputError::ReceiptOwner)?;
                    let observation = entry.observation;
                    entry.outcome = Some(InputOutcome::Refused(error));
                    self.inputs[port].fail(InputError::Delivery, Some(observation));
                    self.synchronize_failure()?;
                    break;
                }
            }
        }
        Ok(())
    }

    /// Never consume a core receipt until its retaining input cell is located.
    fn reconcile(&mut self) -> Result<(), InputCaptureError> {
        while let Some(id) = self.session.next_source_receipt_id() {
            let mut target = None;
            for (port, input) in self.inputs.iter().enumerate() {
                for (slot, entry) in input.slots.iter().enumerate() {
                    if entry
                        .as_ref()
                        .is_some_and(|entry| entry.forwarded == Some(InputDelivery::Serial(id)))
                    {
                        target = Some((port, slot));
                    }
                }
            }
            let (port, slot) = target.ok_or(InputError::ReceiptOwner)?;
            let receipt = self
                .session
                .take_source_receipt()
                .ok_or(InputError::ReceiptOwner)?;
            let failed = matches!(
                &receipt.outcome,
                SessionSourceOutcome::Refused(_)
                    | SessionSourceOutcome::Published(
                        crate::recording::notes::PublicationReceipt {
                            capture: CaptureDisposition::Late | CaptureDisposition::Stopped(_),
                            ..
                        }
                    )
            );
            let entry = self.inputs[port].slots[slot]
                .as_mut()
                .ok_or(InputError::ReceiptOwner)?;
            let observation = entry.observation;
            entry.forwarded = None;
            entry.outcome = Some(InputOutcome::Delivered(receipt.outcome));
            if failed {
                self.inputs[port].fail(InputError::Delivery, Some(observation));
                self.synchronize_failure()?;
            }
        }
        Ok(())
    }
    pub fn drain_stopped_sources(&mut self) -> Result<(), InputCaptureError> {
        self.pump()?;
        if !self.closed {
            self.session.drain_stopped_sources()?;
        }
        self.reconcile()
    }
}
