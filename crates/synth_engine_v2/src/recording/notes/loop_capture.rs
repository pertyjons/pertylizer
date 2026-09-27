//! Serial exact-input loop capture. One owner binds actual render observations to raw input.
//! This reference consumer supplies no concurrent source merge or physical clock mapping.

mod finalize;
mod hot;
pub(crate) mod ordered;
#[cfg(test)]
pub(crate) mod tests;

use super::*;
use crate::{
    looping::{CompiledLoopStream, LoopPassId, LoopSnapshot, journal::JournaledLoopStream},
    quantities::{CapturePassCount, LoopPassCount, SampleRate},
    time::{FrameCount, PlanPosition},
    transport::LoopInterval,
};

/// Loop capture has no single linear anchor across its finite observation window.
pub struct LoopNoteArmInput {
    pub target: FixtureTargetId,
    pub expected_revision: FixtureRevision,
    pub interval: MusicalInterval,
    pub mode: CaptureMode,
    pub quantization: CaptureQuantization,
    pub tempo: TempoMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopCaptureMapping {
    initial: LoopSnapshot,
    interval: LoopInterval,
    passes: CapturePassCount,
}
impl LoopCaptureMapping {
    pub const fn initial(self) -> LoopSnapshot {
        self.initial
    }
    pub const fn interval(self) -> LoopInterval {
        self.interval
    }
    pub const fn passes(self) -> CapturePassCount {
        self.passes
    }
}

/// One logical half-open segment of the shared raw log, not a duplicate take allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopCapturePass {
    id: CapturePassId,
    rendered: LoopPassId,
    window: CaptureWindow,
    position: PlanPosition,
}
impl LoopCapturePass {
    pub const fn id(self) -> CapturePassId {
        self.id
    }
    pub const fn rendered(self) -> LoopPassId {
        self.rendered
    }
    pub const fn window(self) -> CaptureWindow {
        self.window
    }
    pub const fn position(self) -> PlanPosition {
        self.position
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopCarryDirection {
    In,
    Out,
}

/// Key continuation only. Pedals remain separate raw observations, never new attacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopCarry {
    occurrence: PerformedOccurrenceId,
    pass: CapturePassId,
    peer: CapturePassId,
    at: SampleTime,
    direction: LoopCarryDirection,
}
impl LoopCarry {
    pub const fn occurrence(self) -> PerformedOccurrenceId {
        self.occurrence
    }
    pub const fn pass(self) -> CapturePassId {
        self.pass
    }
    pub const fn peer(self) -> CapturePassId {
        self.peer
    }
    pub const fn at(self) -> SampleTime {
        self.at
    }
    pub const fn direction(self) -> LoopCarryDirection {
        self.direction
    }
}

#[derive(Debug, Error)]
pub enum LoopCaptureError {
    #[error(transparent)]
    Capture(#[from] NoteCaptureError),
    #[error(transparent)]
    Render(#[from] crate::looping::LoopFault),
    #[error(transparent)]
    Journal(#[from] crate::looping::journal::LoopJournalPrepareError),
    #[error("loop capture requires an untouched journal and one capture arm")]
    State,
    #[error("retained musical endpoints or sample rate do not match the loop")]
    Mapping,
    #[error("loop observation has not ended")]
    AwaitingAudio,
    #[error("capture sources have not fenced or quiesced at the selected endpoint")]
    AwaitingSources,
}

/// Owns both mutable authorities. Sources are dispatched serially by publication time;
/// nominal time may regress. No mutable inner owner or fabricated boundary is accepted.
#[must_use]
pub struct LoopCaptureSession {
    journal: JournaledLoopStream,
    recorder: SimulatedNoteRecorder,
    rate: SampleRate,
    ticket: Option<TakeReservation>,
    first_pass: Option<CapturePassId>,
    started: bool,
    start: SampleTime,
    carry_capacity: Option<SampleTime>,
}

impl LoopCaptureSession {
    /// Prepare off-thread. The recording ceiling charges this combined inline owner
    /// and all recording storage. Journal heap retains its explicit separate ceiling.
    /// Ordinary loop renderer/program storage retains its original profile/budget.
    pub fn prepare(
        stream: CompiledLoopStream,
        limits: RecordingLimits,
        journal_budget: PreparedBytes,
    ) -> Result<Self, LoopCaptureError> {
        let passes = limits
            .capture()
            .ok_or(CaptureError::MissingConfiguration)
            .map_err(NoteCaptureError::from)?
            .max_capture_passes();
        let rate = stream.sample_rate();
        let journal = JournaledLoopStream::prepare(
            stream,
            LoopPassCount::limit(passes.get()).map_err(NoteCaptureError::from)?,
            journal_budget,
        )?;
        let owner_bytes = u64::try_from(size_of::<Self>() - size_of::<SimulatedNoteRecorder>())
            .map_err(|_| NoteCaptureError::from(CaptureError::LayoutOverflow))?;
        let recorder = SimulatedNoteRecorder::prepare_fixture_extra(
            journal.initial().epoch,
            limits,
            PreparedBytes::measured(owner_bytes),
        )?;
        Ok(Self {
            journal,
            recorder,
            rate,
            ticket: None,
            first_pass: None,
            started: false,
            start: SampleTime::ZERO,
            carry_capacity: None,
        })
    }

    pub fn bind_source(
        &mut self,
        initial: ControllerSnapshot,
    ) -> Result<ConnectionGeneration, LoopCaptureError> {
        if self.ticket.is_some() {
            return Err(LoopCaptureError::State);
        }
        Ok(self.recorder.bind_fixture_source(Some(initial))?)
    }

    /// Reserve one continuous raw window and all recording pass identities off-thread.
    pub fn arm(
        &mut self,
        input: LoopNoteArmInput,
        sources: &[ConnectionGeneration],
    ) -> Result<TakeReservation, LoopCaptureError> {
        self.arm_at(input, sources, self.journal.initial().clock)
    }

    /// Reserve a future quantum-aligned start for an ordered session. The immutable
    /// capture window is fixed here; publishing Play later cannot move timestamps.
    pub fn arm_at(
        &mut self,
        input: LoopNoteArmInput,
        sources: &[ConnectionGeneration],
        start: SampleTime,
    ) -> Result<TakeReservation, LoopCaptureError> {
        if self.ticket.is_some()
            || self.journal.end().is_some()
            || self.journal.acknowledged() != self.journal.initial()
        {
            return Err(LoopCaptureError::State);
        }
        let mut initial = self.journal.initial();
        if start < initial.clock
            || !start
                .as_u64()
                .is_multiple_of(u64::from(crate::time::QUANTUM_FRAMES))
        {
            return Err(LoopCaptureError::Mapping);
        }
        initial.clock = start;
        let interval = self.journal.interval();
        if input.tempo.sample_rate() != self.rate
            || input.tempo.position_of(input.interval.start())? != interval.start()
            || input.tempo.position_of(input.interval.end())? != interval.end()
        {
            return Err(LoopCaptureError::Mapping);
        }
        let passes = self
            .recorder
            .limits
            .capture()
            .ok_or(CaptureError::MissingConfiguration)
            .map_err(NoteCaptureError::from)?
            .max_capture_passes();
        let last = self
            .recorder
            .last_pass
            .0
            .checked_add(u64::from(passes.get()))
            .ok_or(NoteCaptureError::IdentityExhausted)?;
        let duration = interval
            .end()
            .as_u64()
            .checked_sub(interval.start().as_u64())
            .and_then(|length| length.checked_mul(u64::from(passes.get().checked_sub(1)?)))
            .and_then(|full| full.checked_add(interval.end().as_u64() - initial.position.as_u64()))
            .ok_or(NoteCaptureError::MappingRange)?;
        let end = initial
            .clock
            .checked_add(FrameCount::new(duration))
            .map_err(|_| NoteCaptureError::MappingRange)?;
        let window = CaptureWindow::new(initial.epoch, initial.clock, end)
            .map_err(NoteCaptureError::from)?;
        let context = NoteArmContext {
            target: input.target,
            expected_revision: input.expected_revision,
            interval: input.interval,
            mode: input.mode,
            quantization: input.quantization,
            tempo: input.tempo,
            window,
            mapping: NoteMapping::Loop(LoopCaptureMapping {
                initial,
                interval,
                passes,
            }),
        };
        let ticket = self.recorder.arm(context, sources)?;
        self.first_pass = Some(self.recorder.last_pass);
        self.recorder.last_pass = CapturePassId(last);
        if let Some(active) = &mut self.recorder.active {
            active.seal_ready = false;
        }
        self.ticket = Some(ticket);
        self.start = start;
        Ok(ticket)
    }

    pub fn result(&self) -> Result<NoteCaptureResult<'_>, LoopCaptureError> {
        Ok(self
            .recorder
            .result(self.ticket.ok_or(LoopCaptureError::State)?)?)
    }

    pub fn resolve_audition(
        &mut self,
        id: crate::host::live::AuditionId,
        outcome: crate::host::live::AuditionOutcome,
    ) -> Result<crate::quantities::EventCount, LoopCaptureError> {
        Ok(self.recorder.resolve_audition(
            self.ticket.ok_or(LoopCaptureError::State)?,
            id,
            outcome,
        )?)
    }

    pub fn project_notes(
        &mut self,
    ) -> Result<projection::ProjectedLoopTake<'_>, projection::ProjectionError> {
        self.recorder
            .project_loop_notes(self.ticket.ok_or(NoteCaptureError::StartRequired)?)
    }

    /// Combined owner and recording allocations, excluding the separately admitted
    /// journal heap and ordinary compiled-loop renderer/program allocations.
    pub const fn recording_bytes(&self) -> PreparedBytes {
        self.recorder.bytes_reserved()
    }
    pub const fn journal_bytes(&self) -> PreparedBytes {
        self.journal.storage_bytes()
    }
    /// The first interior boundary refused because nominal key carry exceeded H.
    pub const fn carry_capacity(&self) -> Option<SampleTime> {
        self.carry_capacity
    }
    pub fn source_diagnostics(
        &self,
        source: ConnectionGeneration,
    ) -> Result<SourceDiagnostics, LoopCaptureError> {
        Ok(self.recorder.source_diagnostics(source)?)
    }
    /// Explicit off-thread discard still requires every source to retire its quality custody.
    pub fn discard(&mut self, quality: CaptureQuality) -> Result<(), LoopCaptureError> {
        Ok(self
            .recorder
            .discard(self.ticket.ok_or(LoopCaptureError::State)?, quality)?)
    }
    pub const fn initial(&self) -> LoopSnapshot {
        self.journal.initial()
    }
    pub const fn acknowledged(&self) -> LoopSnapshot {
        self.journal.acknowledged()
    }
    pub const fn observation_end(&self) -> Option<crate::looping::journal::LoopJournalEnd> {
        self.journal.end()
    }
}

impl From<TempoError> for LoopCaptureError {
    fn from(error: TempoError) -> Self {
        Self::Capture(NoteCaptureError::Tempo(error))
    }
}

impl NoteCaptureResult<'_> {
    pub fn loop_passes(&self) -> impl Iterator<Item = &LoopCapturePass> {
        self.raw
            .cells(CaptureBuffer::Pass)
            .iter()
            .filter_map(|cell| match cell {
                Some(NoteCell::LoopPass(pass)) => Some(pass),
                _ => None,
            })
    }
    pub fn loop_carry(&self) -> impl Iterator<Item = &LoopCarry> {
        self.raw
            .cells(CaptureBuffer::Carry)
            .iter()
            .filter_map(|cell| match cell {
                Some(NoteCell::Carry(carry)) => Some(carry),
                _ => None,
            })
    }
}
