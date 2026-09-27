//! Non-shipping concrete transport for the simulated input owner.
//! The merger owns preparation and collection; callback access must join before recovery.
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::sync::Arc;
use synth_engine_v2::{
    host::{
        ConnectionGeneration,
        input::{
            InputCaptureAudio, InputCaptureControl, InputCaptureError, InputCaptureHalt,
            InputCaptureSession, InputError, InputEventId, InputObservation, InputReceipt,
            InputReuniteError,
        },
        session::{
            LoopSessionError, SessionCommand,
            loop_transfer::{
                LoopTransferCompletion, LoopTransferError, LoopTransferId, LoopTransferOutcome,
                LoopTransferPacket,
            },
        },
    },
    quantities::PreparedBytes,
    render::AudioBlockMut,
    time::SampleTime,
};
use thiserror::Error;

#[path = "v2_input_host/archive.rs"]
pub mod archive;
#[path = "v2_input_host/audition.rs"]
pub mod audition;
#[path = "v2_input_host/managed.rs"]
pub mod managed;
#[path = "v2_input_host/metronome.rs"]
mod metronome;
#[path = "v2_input_host/prepare.rs"]
pub mod prepare;
#[path = "v2_input_host/source.rs"]
pub mod source;

#[cfg(test)]
#[path = "v2_input_host/tests.rs"]
mod tests;

#[path = "v2_input_host/swaps.rs"]
pub mod swaps;

const CELLS: usize = 64;

#[derive(Debug)]
pub enum HostOutcome {
    Delivered(LoopTransferOutcome),
    Cancelled,
}

/// An accepted observation stays in core custody even if its audition handoff fails.
#[derive(Debug, Error, PartialEq)]
pub enum InputOfferError {
    #[error("input observation refused: {1}")]
    Refused(InputObservation, InputError),
    #[error("accepted input {id:?} has an audition handoff error: {error}")]
    Accepted {
        id: InputEventId,
        error: InputError,
        settlement_error: Option<InputError>,
    },
}

pub type InputOfferResult = Result<InputEventId, InputOfferError>;

#[derive(Debug, Error)]
pub enum HostError {
    #[error("input producers or their queued observations have not been closed")]
    SourcesOpen,
    #[error(transparent)]
    Audition(#[from] synth_engine_v2::host::live::LiveInputError),
    #[error(transparent)]
    Metronome(#[from] synth_engine_v2::host::pulse::PulseError),
    #[error("the pending FIFO publication must be retried first")]
    Pending,
    #[error(transparent)]
    Input(#[from] InputCaptureError),
    #[error(transparent)]
    Transfer(#[from] LoopTransferError),
    #[error(transparent)]
    Render(#[from] LoopSessionError),
    #[error("joined recovery reported multiple faults: {earlier}; {later}")]
    Recovery {
        earlier: Box<Self>,
        later: Box<Self>,
    },
}

impl HostError {
    fn combine_recovery(self, later: Self) -> Self {
        Self::Recovery {
            earlier: Box::new(self),
            later: Box::new(later),
        }
    }
}

/// A refused host-level move retains packets as well as their credit owners.
pub enum ReuniteError {
    Pending(Box<(LiveControl, LiveAudio)>),
    Core(
        Box<InputReuniteError>,
        Option<Box<audition::AuditionAudio>>,
        Option<Box<metronome::MetronomeAudio>>,
    ),
    Audition(
        Box<InputCaptureSession>,
        Box<audition::AuditionAudio>,
        Option<Box<metronome::MetronomeAudio>>,
        InputCaptureError,
    ),
}

#[must_use]
pub struct LiveControl {
    audition: Option<audition::AuditionControl>,
    core: InputCaptureControl,
    halt: InputCaptureHalt,
    packets: HeapProd<LoopTransferPacket>,
    completions: HeapCons<LoopTransferCompletion>,
    pending: Option<LoopTransferPacket>,
    failed_collection: Option<LoopTransferCompletion>,
    pending_input: Option<InputReceipt>,
    // Never let callback endpoint destruction free the ring allocation.
    _packets: Arc<HeapRb<LoopTransferPacket>>,
    _completions: Arc<HeapRb<LoopTransferCompletion>>,
}

#[must_use]
pub struct LiveAudio {
    audition: Option<audition::AuditionAudio>,
    metronome: Option<metronome::MetronomeAudio>,
    core: InputCaptureAudio,
    halt: InputCaptureHalt,
    packets: HeapCons<LoopTransferPacket>,
    completions: HeapProd<LoopTransferCompletion>,
    refused: Option<LoopTransferPacket>,
    pending: Option<LoopTransferCompletion>,
}

impl LiveControl {
    /// Attach already admitted owners off-thread. Queue capacities are finite host
    /// fixture choices, independent of the core's command/source credit ceilings.
    pub fn attach(
        core: InputCaptureControl,
        audio: InputCaptureAudio,
        halt: InputCaptureHalt,
    ) -> (Self, LiveAudio) {
        let packets = Arc::new(HeapRb::new(CELLS));
        let completions = Arc::new(HeapRb::new(CELLS));
        let (writer, reader) = Arc::clone(&packets).split();
        let (returns, receiver) = Arc::clone(&completions).split();
        (
            Self {
                audition: None,
                core,
                halt: halt.clone(),
                packets: writer,
                completions: receiver,
                pending: None,
                failed_collection: None,
                pending_input: None,
                _packets: packets,
                _completions: completions,
            },
            LiveAudio {
                audition: None,
                metronome: None,
                core: audio,
                halt,
                packets: reader,
                completions: returns,
                refused: None,
                pending: None,
            },
        )
    }

    pub fn storage_bytes() -> PreparedBytes {
        // Includes conservative Arc headers/alignment plus both inline owners.
        PreparedBytes::measured(
            (size_of::<Self>()
                + size_of::<LiveAudio>()
                + size_of::<HeapRb<LoopTransferPacket>>()
                + size_of::<HeapRb<LoopTransferCompletion>>()
                + CELLS * (size_of::<LoopTransferPacket>() + size_of::<LoopTransferCompletion>())
                + 512) as u64,
        )
    }

    pub fn halt_handle(&self) -> InputCaptureHalt {
        self.halt.clone()
    }

    fn source_failed_before_ring(
        &mut self,
        generation: ConnectionGeneration,
        observation: InputObservation,
        reason: InputError,
    ) -> Result<(), InputError> {
        self.core
            .record_pre_ring_failure(generation, observation, reason)
    }

    pub fn offer(
        &mut self,
        generation: ConnectionGeneration,
        observation: InputObservation,
    ) -> InputOfferResult {
        let prepared = match self.audition.as_ref() {
            Some(audition) => match audition.preflight(generation, observation) {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.halt.request_invalid();
                    return Err(InputOfferError::Refused(observation, error));
                }
            },
            None => None,
        };
        let id = self
            .core
            .offer_observation(generation, observation)
            .map_err(|(original, error)| InputOfferError::Refused(original, error))?;
        let trace = match (self.audition.as_mut(), prepared) {
            (Some(audition), Some(prepared)) => match audition.commit(prepared) {
                Ok(trace) => trace,
                Err(error) => {
                    self.halt.request_invalid();
                    return Err(InputOfferError::Accepted {
                        id,
                        error,
                        settlement_error: None,
                    });
                }
            },
            _ => synth_engine_v2::recording::notes::AuditionTrace::NotOffered,
        };
        if let Err(error) = self.core.set_audition(id, trace) {
            self.halt.request_invalid();
            let settlement_error = self
                .audition
                .as_mut()
                .and_then(|audition| audition.settle(trace).err());
            return Err(InputOfferError::Accepted {
                id,
                error,
                settlement_error,
            });
        }
        Ok(id)
    }

    /// The returned ID is accepted host custody even when its queue is full.
    pub fn command(
        &mut self,
        at: SampleTime,
        command: SessionCommand,
    ) -> Result<LoopTransferId, HostError> {
        if !self.retry() {
            return Err(HostError::Pending);
        }
        let packet = self.core.prepare_command(at, command)?;
        let id = packet.id();
        self.pending = self.packets.try_push(packet).err();
        Ok(id)
    }

    fn retry(&mut self) -> bool {
        if let Some(packet) = self.pending.take() {
            self.pending = self.packets.try_push(packet).err();
        }
        self.pending.is_none()
    }

    /// Prepare at most one ring's worth per service call; never overtake a retry.
    pub fn pump(&mut self) -> Result<(), HostError> {
        if !self.retry() {
            return Ok(());
        }
        for _ in 0..CELLS {
            let Some(packet) = self.core.next_packet()? else {
                break;
            };
            if let Err(packet) = self.packets.try_push(packet) {
                self.pending = Some(packet);
                break;
            }
        }
        Ok(())
    }

    /// Return one command outcome. Input outcomes stay in their original core cells.
    /// Call until the completion ring is empty; None may mean an input was collected.
    pub fn collect(&mut self) -> Result<Option<(LoopTransferId, HostOutcome)>, HostError> {
        let completion = self
            .failed_collection
            .take()
            .or_else(|| self.completions.try_pop());
        let Some(completion) = completion else {
            return Ok(None);
        };
        match self.core.collect(completion) {
            Ok(outcome) => Ok(outcome.map(|(id, outcome)| (id, HostOutcome::Delivered(outcome)))),
            Err(error) => {
                let (completion, error) = *error;
                self.failed_collection = Some(completion);
                Err(error.into())
            }
        }
    }

    pub fn has_completions(&self) -> bool {
        self.failed_collection.is_some() || !self.completions.is_empty()
    }
    /// An audition settlement error retains its raw receipt in this owner.
    /// Other generations report no receipt while this one is retained.
    pub fn collect_input(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<Option<InputReceipt>, InputError> {
        let receipt = if let Some(receipt) = self.pending_input.take() {
            if receipt.id.generation() != generation {
                self.pending_input = Some(receipt);
                return Ok(None);
            }
            Some(receipt)
        } else {
            self.core.collect_input(generation)?
        };
        if let Some(trace) = receipt.as_ref().map(|receipt| receipt.audition)
            && let Some(audition) = &mut self.audition
            && let Err(error) = audition.settle(trace)
        {
            self.pending_input = receipt;
            return Err(error);
        }
        Ok(receipt)
    }

    pub fn collect_audition(
        &mut self,
    ) -> Option<(
        synth_engine_v2::host::live::AuditionId,
        synth_engine_v2::host::live::AuditionOutcome,
    )> {
        self.audition.as_mut()?.collect()
    }

    fn cancel_cell(
        &mut self,
        cell: &mut Option<LoopTransferPacket>,
    ) -> Result<Option<LoopTransferId>, HostError> {
        if let Some(packet) = cell.take() {
            match self.core.cancel(packet) {
                Ok(id) => return Ok(Some(id)),
                Err((packet, error)) => {
                    *cell = Some(packet);
                    return Err(error.into());
                }
            }
        }
        Ok(None)
    }

    /// After callback access joins, recover one bounded batch without consuming either
    /// owner. Returned command outcomes are final even if a later step fails;
    /// retries never report them again. An audition fault does not stop command
    /// cleanup. Commands are cancelled from the oldest refused packet through
    /// the ring to the newest pending packet. A command fault keeps that packet
    /// and its later suffix. A command fault or failed core halt leaves core
    /// completions for retry. Audition, halt and the first command
    /// fault are all returned when they occur in the same attempt.
    /// Source quiescence and finalization follow reunion.
    pub fn recover(
        &mut self,
        audio: &mut LiveAudio,
        mut receive: impl FnMut(LoopTransferId, HostOutcome),
    ) -> Result<(), HostError> {
        self.halt.request_device_lost();
        let audition_error = audio
            .audition
            .as_mut()
            .and_then(|audition| audition.finish().err())
            .map(HostError::from);
        let halt_error = audio.core.synchronize_halt().err().map(HostError::from);
        let cleanup = (|| {
            if let Some(id) = self.cancel_cell(&mut audio.refused)? {
                receive(id, HostOutcome::Cancelled);
            }
            while let Some(packet) = audio.packets.try_pop() {
                audio.refused = Some(packet);
                if let Some(id) = self.cancel_cell(&mut audio.refused)? {
                    receive(id, HostOutcome::Cancelled);
                }
            }
            let mut pending = self.pending.take();
            let result = self.cancel_cell(&mut pending);
            self.pending = pending;
            if let Some(id) = result? {
                receive(id, HostOutcome::Cancelled);
            }
            if halt_error.is_none() {
                loop {
                    audio.flush();
                    if !self.has_completions() {
                        break;
                    }
                    while self.has_completions() {
                        if let Some((id, outcome)) = self.collect()? {
                            receive(id, outcome);
                        }
                    }
                }
            }
            Ok(())
        })();
        let error = audition_error
            .into_iter()
            .chain(halt_error)
            .chain(cleanup.err())
            .reduce(HostError::combine_recovery);
        error.map_or(Ok(()), Err)
    }

    /// Normal ordered Stop, after the backend and input producers have joined.
    /// No terminal signal is set: source fences may complete a healthy take.
    /// Finalization and explicit source acknowledgements follow reunion.
    pub fn finish_after_join(
        &mut self,
        audio: &mut LiveAudio,
        mut receive: impl FnMut(LoopTransferId, HostOutcome),
    ) -> Result<(), HostError> {
        // Each round consumes finite source cells or credits. This fixture admits
        // at most 64 cells per source; bounded attempts never wait for a worker.
        for _ in 0..CELLS {
            self.pump()?;
            audio.admit()?;
            audio.core.drain_stopped_sources()?;
            audio.flush();
            let progress = self.has_completions();
            while self.has_completions() {
                if let Some((id, outcome)) = self.collect()? {
                    receive(id, outcome);
                }
            }
            if !progress && self.pending.is_none() {
                if let Some(audition) = &mut audio.audition {
                    audition.finish()?;
                }
                return Ok(());
            }
        }
        Err(HostError::Pending)
    }

    /// Ownership failures are returned intact by the core. Call recover first and
    /// retain both halves until the backend has joined callback access.
    pub fn reunite(
        mut self,
        mut audio: LiveAudio,
        mut receive: impl FnMut(
            synth_engine_v2::host::live::AuditionId,
            synth_engine_v2::host::live::AuditionOutcome,
        ),
    ) -> Result<InputCaptureSession, ReuniteError> {
        if self.pending.is_some()
            || self.failed_collection.is_some()
            || self.pending_input.is_some()
            || audio.pending.is_some()
            || audio.refused.is_some()
            || !audio.packets.is_empty()
            || !self.completions.is_empty()
            || audio
                .audition
                .as_ref()
                .is_some_and(|audition| !audition.is_finished())
        {
            return Err(ReuniteError::Pending(Box::new((self, audio))));
        }
        while let Some((id, outcome)) = self.collect_audition() {
            receive(id, outcome);
        }
        let audition = audio.audition.take().map(Box::new);
        let mut owner = match audio.core.reunite(self.core) {
            Ok(owner) => owner,
            Err(error) => {
                return Err(ReuniteError::Core(
                    error,
                    audition,
                    audio.metronome.take().map(Box::new),
                ));
            }
        };
        if let Some(audition) = audition {
            let mut failure = None;
            for (id, outcome) in audition.outcomes() {
                if let Err(error) = owner.resolve_audition(id, outcome) {
                    failure = Some(error);
                    break;
                }
                receive(id, outcome);
            }
            if let Some(error) = failure {
                return Err(ReuniteError::Audition(
                    Box::new(owner),
                    audition,
                    audio.metronome.take().map(Box::new),
                    error,
                ));
            }
        }
        Ok(owner)
    }
}

impl LiveAudio {
    pub fn clock(&self) -> SampleTime {
        self.core.acknowledged().clock
    }

    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), HostError> {
        if let Some(audition) = &self.audition
            && let Err(error) = audition.validate(&output)
        {
            output.silence();
            return Err(error.into());
        }
        if let Some(metronome) = &self.metronome
            && let Err(error) = metronome.validate(&output)
        {
            output.silence();
            return Err(error.into());
        }
        if let Err(error) = self.admit() {
            output.silence();
            return Err(error);
        }
        let result = self.core.render(output.reborrow());
        if let Err(error) = result {
            output.silence();
            self.flush();
            return Err(error.into());
        }
        if self.halt.is_requested() {
            output.silence();
            self.flush();
            return Ok(());
        }
        if let Some(audition) = &mut self.audition
            && let Err(error) = audition.render(&mut output, self.core.applied_end())
        {
            output.silence();
            self.halt.request_invalid();
            return Err(error.into());
        }
        if let Some(audition) = &mut self.audition
            && let Err(error) = audition.reconcile(&mut self.core)
        {
            output.silence();
            self.halt.request_invalid();
            return Err(error);
        }
        if let Some(metronome) = &mut self.metronome
            && let Err(error) = metronome.render(&mut output, self.core.applied_end())
        {
            output.silence();
            self.halt.request_invalid();
            return Err(error.into());
        }
        self.flush();
        Ok(())
    }

    fn admit(&mut self) -> Result<(), HostError> {
        if self.refused.is_some() {
            return Err(HostError::Transfer(LoopTransferError::Closed));
        }
        let prefix = self.packets.occupied_len();
        for _ in 0..prefix {
            let Some(packet) = self.packets.try_pop() else {
                break;
            };
            if let Err((packet, error)) = self.core.enqueue(packet) {
                self.refused = Some(packet);
                self.halt.request_invalid();
                return Err(error.into());
            }
        }
        Ok(())
    }

    fn flush(&mut self) {
        if let Some(completion) = self.pending.take() {
            self.pending = self.completions.try_push(completion).err();
        }
        if self.pending.is_some() {
            return;
        }
        for _ in 0..CELLS {
            if self.completions.is_full() {
                break;
            }
            let Some(completion) = self.core.take_completed() else {
                break;
            };
            if let Err(completion) = self.completions.try_push(completion) {
                self.pending = Some(completion);
                break;
            }
        }
    }
}
