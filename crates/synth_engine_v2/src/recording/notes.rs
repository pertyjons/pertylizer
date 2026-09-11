//! P09-S003: serialized exact-input note capture, before physical or concurrent adapters.
//!
//! The fixture supplies explicit session-before-source boundaries and zero lateness.
//! Capture retains its own immutable tempo/target context; audition is a supplied trace,
//! never a condition of capture acceptance. Certified off-thread note projection lives
//! in [`projection`]; loop passes and project commit are not exposed here.
//! All owned memory, including source state and tempo maps, is
//! charged to the recording byte budget. Preparation, arm, rebind and discard run off-thread.

mod hot;
pub mod projection;
#[cfg(test)]
mod tests;
mod types;
pub use types::*;

use super::{
    CaptureBuffer, CaptureError, CaptureLayout, CaptureOutcome, CaptureQuality, CaptureResult,
    CaptureWindow, DiagnosticCount, SimulatedTakeStore, TakeReservation,
};
use crate::host::ConnectionGeneration;
use crate::profile::RecordingLimits;
use crate::quantities::{PreparedBytes, QuantityError};
use crate::tempo::{MusicalTick, TempoError, TempoMap};
use crate::time::{SampleTime, StreamAnchor, StreamEpoch};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

static NEXT_SESSION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureMode {
    #[default]
    Overdub,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct MusicalInterval {
    start: MusicalTick,
    end: MusicalTick,
}
impl MusicalInterval {
    pub fn new(start: MusicalTick, end: MusicalTick) -> Result<Self, NoteCaptureError> {
        if start >= end {
            return Err(NoteCaptureError::InvalidInterval);
        }
        Ok(Self { start, end })
    }
    pub const fn start(self) -> MusicalTick {
        self.start
    }
    pub const fn end(self) -> MusicalTick {
        self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct QuantizationGrid(MusicalTick);
impl QuantizationGrid {
    pub fn new(ticks: MusicalTick) -> Result<Self, NoteCaptureError> {
        if ticks.as_u64() == 0 {
            return Err(NoteCaptureError::InvalidGrid);
        }
        Ok(Self(ticks))
    }
    pub const fn ticks(self) -> MusicalTick {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureQuantization {
    #[default]
    Off,
    Grid(QuantizationGrid),
}

/// Owned preparation input. Canonical project targets and transactions remain Phase 10.
pub struct NoteArmInput {
    pub target: FixtureTargetId,
    pub expected_revision: FixtureRevision,
    pub interval: MusicalInterval,
    pub mode: CaptureMode,
    pub quantization: CaptureQuantization,
    pub epoch: StreamEpoch,
    pub anchor: StreamAnchor,
    pub tempo: TempoMap,
}

/// Immutable capture context. Preparing the endpoints is not a projection certificate.
#[must_use]
pub struct NoteArmContext {
    input: NoteArmInput,
    window: CaptureWindow,
}
impl NoteArmContext {
    pub fn prepare(input: NoteArmInput) -> Result<Self, NoteCaptureError> {
        let start = input
            .anchor
            .time_of(input.tempo.position_of(input.interval.start)?)
            .ok_or(NoteCaptureError::MappingRange)?;
        let end = input
            .anchor
            .time_of(input.tempo.position_of(input.interval.end)?)
            .ok_or(NoteCaptureError::MappingRange)?;
        let window = CaptureWindow::new(input.epoch, start, end)?;
        Ok(Self { input, window })
    }
    pub const fn target(&self) -> FixtureTargetId {
        self.input.target
    }
    pub const fn expected_revision(&self) -> FixtureRevision {
        self.input.expected_revision
    }
    pub const fn interval(&self) -> MusicalInterval {
        self.input.interval
    }
    pub const fn mode(&self) -> CaptureMode {
        self.input.mode
    }
    pub const fn quantization(&self) -> CaptureQuantization {
        self.input.quantization
    }
    pub const fn window(&self) -> CaptureWindow {
        self.window
    }
    pub const fn anchor(&self) -> StreamAnchor {
        self.input.anchor
    }
    pub const fn tempo(&self) -> &TempoMap {
        &self.input.tempo
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStopReason {
    Stop,
    Disarm,
    Panic,
    Seek,
    TempoChange,
    DeviceReprepare,
    Capacity,
    LateInput,
    SourceInvalid,
    DeviceLost,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureDisposition {
    NotRecording,
    OutsideInterval,
    Recorded,
    Late,
    Stopped(CaptureStopReason),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PublicationReceipt {
    pub sequence: PublicationSequence,
    pub occurrence: Option<PerformedOccurrenceId>,
    pub capture: CaptureDisposition,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[must_use]
pub struct SourceDiagnostics {
    pub unmatched_releases: DiagnosticCount,
    pub late_refusals: DiagnosticCount,
    pub timing_anomalies: DiagnosticCount,
    pub pairing_failures: DiagnosticCount,
    pub first_late: Option<super::LateCaptureInput>,
}

/// Off-thread transfer of the retired generation's diagnostics to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SourceRebind {
    pub generation: ConnectionGeneration,
    pub retired_generation: ConnectionGeneration,
    pub diagnostics: SourceDiagnostics,
}

#[derive(Debug, Clone, Copy)]
struct SourceState {
    generation: ConnectionGeneration,
    synchronized: bool,
    quiescent: bool,
    controls: ControllerSnapshot,
    sequence: PublicationSequence,
    last_occurrence: Option<PerformedOccurrenceId>,
    last_nominal: Option<SampleTime>,
    observed: SampleTime,
    fence: Option<SampleTime>,
    diagnostics: SourceDiagnostics,
}
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct HeldInput {
    pub occurrence: PerformedOccurrenceId,
    pub channel: synth_core::MidiChannel,
    pub key: crate::quantities::KeyIdentity,
}
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct RecordedSourceState {
    pub source: ConnectionGeneration,
    pub through: PublicationSequence,
    /// State from accepted capture records; refused controllers are separate observations.
    pub controls: ControllerSnapshot,
    boundary: SampleTime,
    refused_pedals: Option<RefusedPedals>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RefusedPedals {
    earliest: SampleTime,
    latest: SampleTime,
    seen: [bool; 16],
    last_down: [bool; 16],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SyntheticClosure {
    pub occurrence: PerformedOccurrenceId,
    pub time: SampleTime,
    pub key_held: bool,
    /// None means a backwards cut intersects coalesced refused pedal observations.
    pub pedal_held: Option<bool>,
    pub reason: CaptureStopReason,
}
#[derive(Debug, Clone, Copy)]
enum NoteCell {
    Input {
        record: RecordedInput,
        release: Option<SampleTime>,
        paired_release: Option<SampleTime>,
        closure: Option<SyntheticClosure>,
    },
    Held(HeldInput),
    Source(RecordedSourceState),
    Pass(CapturePassId),
}
#[derive(Debug, Clone, Copy)]
enum CaptureStage {
    Armed,
    Capturing,
    Stopping {
        at: SampleTime,
        outcome: CaptureOutcome,
        reason: CaptureStopReason,
    },
}
#[derive(Debug, Clone, Copy)]
struct ActiveCapture {
    ticket: TakeReservation,
    stage: CaptureStage,
}

/// One bounded serial publisher; all source generations belong to this owner.
#[must_use]
pub struct SimulatedNoteRecorder {
    store: SimulatedTakeStore<NoteCell>,
    sources: Box<[Option<SourceState>]>,
    tracker: Box<[Option<HeldInput>]>,
    contexts: Box<[Option<NoteArmContext>]>,
    active: Option<ActiveCapture>,
    limits: RecordingLimits,
    base_bytes: PreparedBytes,
    map_bytes: PreparedBytes,
    session: CaptureSessionId,
    last_pass: CapturePassId,
    epoch: StreamEpoch,
    observed: SampleTime,
    pub(crate) host_generation: Option<ConnectionGeneration>,
    host_interrupted: bool,
}

impl SimulatedNoteRecorder {
    pub fn prepare_fixture(
        epoch: StreamEpoch,
        limits: RecordingLimits,
    ) -> Result<Self, NoteCaptureError> {
        let capture = limits.capture().ok_or(CaptureError::MissingConfiguration)?;
        if capture.capture_lateness_allowance().as_u64() != 0 {
            return Err(NoteCaptureError::NonzeroLateness);
        }
        let layout = CaptureLayout::for_payload::<NoteCell>(limits)?;
        let sources = capture
            .max_capture_sources()
            .as_usize()
            .ok_or(CaptureError::LayoutOverflow)?;
        let tracked = capture
            .max_tracked_input_notes()
            .as_usize()
            .ok_or(CaptureError::LayoutOverflow)?;
        let results = capture
            .max_pending_capture_results()
            .as_usize()
            .ok_or(CaptureError::LayoutOverflow)?;
        let base = layout
            .bytes()
            .get()
            .checked_sub(size_of::<SimulatedTakeStore<NoteCell>>() as u64)
            .and_then(|n| n.checked_add(size_of::<Self>() as u64))
            .ok_or(CaptureError::LayoutOverflow)?;
        let base = add_bytes(base, array_bytes::<Option<SourceState>>(sources)?)?;
        let base = add_bytes(base, array_bytes::<Option<HeldInput>>(tracked)?)?;
        let base = add_bytes(base, array_bytes::<Option<NoteArmContext>>(results)?)?;
        check_bytes(base, limits)?;
        let session = issue_session(&NEXT_SESSION)?;
        Ok(Self {
            store: SimulatedTakeStore::prepare_fixture(limits)?,
            sources: empty_slots(sources)?,
            tracker: empty_slots(tracked)?,
            contexts: empty_slots(results)?,
            active: None,
            limits,
            base_bytes: PreparedBytes::measured(base),
            map_bytes: PreparedBytes::NONE,
            session,
            last_pass: CapturePassId(0),
            host_generation: None,
            host_interrupted: false,
            epoch,
            observed: SampleTime::ZERO,
        })
    }
    pub const fn session(&self) -> CaptureSessionId {
        self.session
    }
    pub const fn bytes_reserved(&self) -> PreparedBytes {
        // Admission checked this sum before either field changed.
        PreparedBytes::measured(self.base_bytes.get() + self.map_bytes.get())
    }
    /// Establish a synthetic source. A supplied controller snapshot asserts known empty
    /// physical key state; publish pre-arm input to establish any subsequently held keys.
    pub fn bind_fixture_source(
        &mut self,
        initial: Option<ControllerSnapshot>,
    ) -> Result<ConnectionGeneration, NoteCaptureError> {
        let at = self
            .sources
            .iter()
            .position(Option::is_none)
            .ok_or(NoteCaptureError::SourcesFull)?;
        let generation = crate::host::issue_capture_source_generation()
            .map_err(|_| NoteCaptureError::IdentityExhausted)?;
        self.sources[at] = Some(source_state(generation, initial, self.observed));
        Ok(generation)
    }
    pub fn synchronize_fixture_source(
        &mut self,
        generation: ConnectionGeneration,
        initial: ControllerSnapshot,
    ) -> Result<(), NoteCaptureError> {
        if self.active.is_some() {
            return Err(NoteCaptureError::AlreadyArmed);
        }
        let at = self.source_index(generation)?;
        let source = self.sources[at]
            .as_mut()
            .ok_or(NoteCaptureError::ForeignSource)?;
        if source.quiescent || source.sequence.0 != 0 || source.synchronized {
            return Err(NoteCaptureError::RebindRequired);
        }
        source.controls = initial;
        source.synchronized = true;
        Ok(())
    }
    pub fn rebind_fixture_source(
        &mut self,
        old: ConnectionGeneration,
        initial: Option<ControllerSnapshot>,
    ) -> Result<SourceRebind, NoteCaptureError> {
        if self.active.is_some() {
            return Err(NoteCaptureError::AlreadyArmed);
        }
        let at = self.source_index(old)?;
        if self.sources[at].is_none_or(|source| !source.quiescent) {
            return Err(NoteCaptureError::SourcesLive);
        }
        let generation = crate::host::issue_capture_source_generation()
            .map_err(|_| NoteCaptureError::IdentityExhausted)?;
        for held in &mut self.tracker {
            if held.is_some_and(|held| held.occurrence.source == old) {
                *held = None;
            }
        }
        let diagnostics = self.sources[at]
            .ok_or(NoteCaptureError::ForeignSource)?
            .diagnostics;
        self.sources[at] = Some(source_state(generation, initial, self.observed));
        Ok(SourceRebind {
            generation,
            retired_generation: old,
            diagnostics,
        })
    }
    pub fn arm(
        &mut self,
        context: NoteArmContext,
        sources: &[ConnectionGeneration],
    ) -> Result<TakeReservation, NoteCaptureError> {
        if self.active.is_some() {
            return Err(NoteCaptureError::AlreadyArmed);
        }
        if context.window.epoch() != self.epoch {
            return Err(NoteCaptureError::ForeignEpoch);
        }
        if context.window.start() < self.observed {
            return Err(NoteCaptureError::PastBoundary);
        }
        for generation in sources {
            let source = self.sources[self.source_index(*generation)?]
                .ok_or(NoteCaptureError::ForeignSource)?;
            if !source.synchronized || source.quiescent {
                return Err(NoteCaptureError::Unsynchronized);
            }
            if source.fence.is_some_and(|at| at > context.window.start())
                || (source.sequence.0 != 0 && source.observed >= context.window.start())
            {
                return Err(NoteCaptureError::PastBoundary);
            }
        }
        let occupied = self.tracker.iter().flatten().count();
        let held = self
            .limits
            .max_held_notes_per_take()
            .as_usize()
            .ok_or(CaptureError::LayoutOverflow)?;
        if occupied
            .checked_add(held)
            .is_none_or(|n| n > self.tracker.len())
        {
            return Err(NoteCaptureError::TrackerReservation);
        }
        let map_bytes = add_bytes(
            self.map_bytes.get(),
            context.input.tempo.bytes_held() as u64,
        )?;
        check_bytes(add_bytes(self.base_bytes.get(), map_bytes)?, self.limits)?;
        let pass = self
            .last_pass
            .0
            .checked_add(1)
            .map(CapturePassId)
            .ok_or(NoteCaptureError::IdentityExhausted)?;
        let ticket = self.store.reserve_owned_sources(context.window, sources)?;
        self.contexts[ticket.slot] = Some(context);
        self.map_bytes = PreparedBytes::measured(map_bytes);
        self.last_pass = pass;
        self.active = Some(ActiveCapture {
            ticket,
            stage: CaptureStage::Armed,
        });
        self.store
            .write_fixture_metadata(ticket, CaptureBuffer::Pass, 0, NoteCell::Pass(pass))?;
        for generation in sources {
            if let Some(frontier) =
                self.sources[self.source_index(*generation)?].and_then(|s| s.fence)
            {
                self.store
                    .acknowledge_fixture_fence(ticket, *generation, self.epoch, frontier)?;
            }
        }
        self.snapshot_sources(ticket)?;
        Ok(ticket)
    }
    pub fn result(
        &self,
        ticket: TakeReservation,
    ) -> Result<NoteCaptureResult<'_>, NoteCaptureError> {
        let raw = self.store.result(ticket)?;
        let context = self
            .contexts
            .get(ticket.slot)
            .and_then(Option::as_ref)
            .ok_or(CaptureError::StaleReservation)?;
        Ok(NoteCaptureResult {
            raw,
            context,
            session: self.session,
        })
    }
    pub fn retained_results(&self) -> impl Iterator<Item = TakeReservation> + '_ {
        self.store.retained_results()
    }
    pub fn take_notification(&mut self) -> Option<TakeReservation> {
        self.store.take_notification()
    }
    pub const fn notification_misses(&self) -> DiagnosticCount {
        self.store.notification_misses()
    }
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }
    pub fn source_sequence(
        &self,
        source: ConnectionGeneration,
    ) -> Result<PublicationSequence, NoteCaptureError> {
        Ok(self.sources[self.source_index(source)?]
            .ok_or(NoteCaptureError::ForeignSource)?
            .sequence)
    }
    pub fn source_diagnostics(
        &self,
        source: ConnectionGeneration,
    ) -> Result<SourceDiagnostics, NoteCaptureError> {
        Ok(self.sources[self.source_index(source)?]
            .ok_or(NoteCaptureError::ForeignSource)?
            .diagnostics)
    }
    pub fn discard(
        &mut self,
        ticket: TakeReservation,
        quality: CaptureQuality,
    ) -> Result<(), NoteCaptureError> {
        self.store.discard(ticket, quality)?;
        if let Some(context) = self.contexts[ticket.slot].take() {
            self.map_bytes = PreparedBytes::measured(
                self.map_bytes.get() - context.input.tempo.bytes_held() as u64,
            );
        }
        Ok(())
    }
}

#[must_use]
pub struct NoteCaptureResult<'a> {
    raw: CaptureResult<'a, NoteCell>,
    context: &'a NoteArmContext,
    session: CaptureSessionId,
}
impl NoteCaptureResult<'_> {
    pub const fn context(&self) -> &NoteArmContext {
        self.context
    }
    pub const fn session(&self) -> CaptureSessionId {
        self.session
    }
    pub const fn window(&self) -> CaptureWindow {
        self.raw.window()
    }
    pub const fn sealed_outcome(&self) -> CaptureOutcome {
        self.raw.sealed_outcome()
    }
    pub const fn effective_outcome(&self) -> CaptureOutcome {
        self.raw.effective_outcome()
    }
    pub const fn quality(&self) -> CaptureQuality {
        self.raw.quality()
    }
    /// Raw accepted input, including observations outside a subsequently shortened window.
    pub fn records(&self) -> impl Iterator<Item = &RecordedInput> {
        self.raw
            .cells(CaptureBuffer::Ordinary)
            .iter()
            .filter_map(|cell| match cell {
                Some(NoteCell::Input { record, .. }) => Some(record),
                _ => None,
            })
    }
    /// Accepted input whose original timestamp lies in the final selected interval.
    /// This is not a projection certificate: negative or zero lifetimes still need refusal.
    pub fn selected_records(&self) -> impl Iterator<Item = &RecordedInput> {
        let window = self.window();
        self.records().filter(move |record| {
            record.stamp.nominal >= window.start() && record.stamp.nominal < window.end()
        })
    }
    pub fn closures(&self) -> impl Iterator<Item = &SyntheticClosure> {
        self.raw
            .cells(CaptureBuffer::Ordinary)
            .iter()
            .filter_map(|cell| match cell {
                Some(NoteCell::Input {
                    closure: Some(closure),
                    ..
                }) => Some(closure),
                _ => None,
            })
    }
    pub fn initial_held(&self) -> impl Iterator<Item = &HeldInput> {
        self.raw
            .cells(CaptureBuffer::TrackedInput)
            .iter()
            .filter_map(|cell| match cell {
                Some(NoteCell::Held(held)) => Some(held),
                _ => None,
            })
    }
    pub fn initial_sources(&self) -> impl Iterator<Item = &RecordedSourceState> {
        self.raw
            .cells(CaptureBuffer::InitialSourceState)
            .iter()
            .filter_map(|cell| match cell {
                Some(NoteCell::Source(source)) => Some(source),
                _ => None,
            })
    }
    pub fn terminal_sources(&self) -> impl Iterator<Item = &RecordedSourceState> {
        self.raw
            .cells(CaptureBuffer::TerminalSourceState)
            .iter()
            .filter_map(|cell| match cell {
                Some(NoteCell::Source(source)) => Some(source),
                _ => None,
            })
    }
    pub fn pass(&self) -> Option<CapturePassId> {
        match self.raw.cells(CaptureBuffer::Pass).first() {
            Some(Some(NoteCell::Pass(id))) => Some(*id),
            _ => None,
        }
    }
}

fn issue_session(counter: &AtomicU64) -> Result<CaptureSessionId, NoteCaptureError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map(|n| CaptureSessionId(n + 1))
        .map_err(|_| NoteCaptureError::IdentityExhausted)
}

fn source_state(
    generation: ConnectionGeneration,
    initial: Option<ControllerSnapshot>,
    observed: SampleTime,
) -> SourceState {
    SourceState {
        generation,
        synchronized: initial.is_some(),
        quiescent: false,
        controls: initial.unwrap_or(ControllerSnapshot::neutral()),
        sequence: PublicationSequence(0),
        last_occurrence: None,
        last_nominal: None,
        observed,
        fence: None,
        diagnostics: SourceDiagnostics::default(),
    }
}
fn empty_slots<T>(count: usize) -> Result<Box<[Option<T>]>, NoteCaptureError> {
    let mut slots = Vec::new();
    slots
        .try_reserve_exact(count)
        .map_err(|_| CaptureError::Allocation)?;
    slots.resize_with(count, || None);
    Ok(slots.into_boxed_slice())
}
fn array_bytes<T>(count: usize) -> Result<u64, NoteCaptureError> {
    let bytes = count
        .checked_mul(size_of::<T>())
        .filter(|n| *n <= isize::MAX as usize)
        .ok_or(CaptureError::LayoutOverflow)?;
    Ok(bytes as u64)
}
fn add_bytes(a: u64, b: u64) -> Result<u64, NoteCaptureError> {
    a.checked_add(b).ok_or(CaptureError::LayoutOverflow.into())
}
fn check_bytes(bytes: u64, limits: RecordingLimits) -> Result<(), NoteCaptureError> {
    let available = limits
        .capture()
        .ok_or(CaptureError::MissingConfiguration)?
        .max_capture_bytes();
    if bytes > available.get() {
        return Err(CaptureError::ByteBudget {
            required: PreparedBytes::measured(bytes),
            available,
        }
        .into());
    }
    Ok(())
}

#[derive(Debug, PartialEq, Error)]
pub enum NoteCaptureError {
    #[error("capture tempo map uses a different rate from the prepared output")]
    HostSampleRate,
    #[error("capture anchor differs from the prepared output stream")]
    HostAnchor,
    #[error(transparent)]
    Storage(#[from] CaptureError),
    #[error(transparent)]
    Quantity(#[from] QuantityError),
    #[error(transparent)]
    Tempo(#[from] TempoError),
    #[error("fixture target must be nonzero")]
    InvalidTarget,
    #[error("musical interval must have positive length")]
    InvalidInterval,
    #[error("quantization grid must be positive")]
    InvalidGrid,
    #[error("capture endpoints lie outside the retained mapping")]
    MappingRange,
    #[error("invalid MIDI 1 data byte")]
    InvalidMidi,
    #[error("unsupported MIDI 1 message; only notes, sustain and pitch bend are admitted")]
    UnsupportedMidi,
    #[error("initial controller state requires a supported controller message")]
    NotController,
    #[error("a nominal timestamp may not be later than its simulated publication boundary")]
    FutureInput,
    #[error(
        "this exact-input fixture requires explicit zero lateness; a reorder worker is not implemented"
    )]
    NonzeroLateness,
    #[error("identity counter exhausted")]
    IdentityExhausted,
    #[error("all prepared source slots are occupied")]
    SourcesFull,
    #[error("an active capture must be finalized before another arm or mapping change")]
    AlreadyArmed,
    #[error("source does not belong to this recorder")]
    ForeignSource,
    #[error("timestamp or context belongs to another epoch")]
    ForeignEpoch,
    #[error("source state is unknown or invalid")]
    Unsynchronized,
    #[error("source needs quiescence and a new generation before reset")]
    RebindRequired,
    #[error("source is still live")]
    SourcesLive,
    #[error("a session boundary cannot move behind already consumed input")]
    PastBoundary,
    #[error("known held input plus capture reserve exceeds the admitted tracker")]
    TrackerReservation,
    #[error("source tracker exhausted; pairing is invalid until rebind")]
    TrackerFull,
    #[error("uncaptured input would consume the protected capture tracker reserve")]
    TrackerReserved,
    #[error("the requested capture is not the active reservation")]
    NotActive,
    #[error("an explicit ordered start is required before interval input")]
    StartRequired,
    #[error("source fences have not reached the explicit capture start")]
    StartFence,
    #[error("internal failure reasons cannot be selected as a session stop")]
    InvalidStopReason,
    #[error("publication fence must name the current consumed sequence")]
    InvalidFence,
}
