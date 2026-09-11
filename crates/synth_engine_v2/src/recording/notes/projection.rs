//! P09-S004: finite, certified note-only projection of a sealed exact-input take.
//!
//! Preparation runs off-thread and borrows the recorder exclusively for the lifetime
//! of the result. Its table can therefore only use the retained immutable map, rate,
//! interval, epoch and session; there is no API for substituting a current map. The
//! borrow also prevents simultaneous uncharged projections and stale quality reads.
//! A later concurrent worker/commit consumer needs its own custody and quality protocol.

mod table;
#[cfg(test)]
mod tests;

use super::{
    CaptureQuantization, CaptureSessionId, CaptureStamp, CaptureStopReason, ControllerSnapshot,
    Midi1Event, NoteCaptureError, NoteCaptureResult, NoteCell, PerformedOccurrenceId,
    SimulatedNoteRecorder,
};
use crate::host::ConnectionGeneration;
use crate::quantities::{KeyIdentity, NoteVelocity, PreparedBytes, ProjectionTickCount};
use crate::recording::{CaptureBuffer, CaptureError, CaptureOutcome, TakeReservation};
use crate::tempo::{MusicalTick, TempoError};
use crate::time::{FrameDelta, PlanPosition, SampleTime, TimeError};
use table::ProjectionTable;
use thiserror::Error;

/// A nearest musical tick and its signed forward-frame error, before grid snapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct TickProjection {
    tick: MusicalTick,
    error: FrameDelta,
}
impl TickProjection {
    pub const fn tick(self) -> MusicalTick {
        self.tick
    }
    pub const fn error(self) -> FrameDelta {
        self.error
    }
}

/// The closure's provenance, without carrying an engine timestamp into musical data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectedEnding {
    KeyRelease,
    Synthetic {
        reason: CaptureStopReason,
        key_held: bool,
        pedal_held: Option<bool>,
    },
}

/// One whole, validated note. Raw timing and release velocity remain in the take.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ProjectedNote {
    occurrence: PerformedOccurrenceId,
    channel: synth_core::MidiChannel,
    key: KeyIdentity,
    velocity: NoteVelocity,
    onset: TickProjection,
    release: TickProjection,
    start: MusicalTick,
    end: MusicalTick,
    ending: ProjectedEnding,
}
impl ProjectedNote {
    pub const fn occurrence(self) -> PerformedOccurrenceId {
        self.occurrence
    }
    pub const fn channel(self) -> synth_core::MidiChannel {
        self.channel
    }
    pub const fn key(self) -> KeyIdentity {
        self.key
    }
    pub const fn velocity(self) -> NoteVelocity {
        self.velocity
    }
    pub const fn onset(self) -> TickProjection {
        self.onset
    }
    pub const fn release(self) -> TickProjection {
        self.release
    }
    /// Musical start after optional grid snapping; `onset` retains the unsnapped tick.
    pub const fn start(self) -> MusicalTick {
        self.start
    }
    pub const fn end(self) -> MusicalTick {
        self.end
    }
    pub const fn ending(self) -> ProjectedEnding {
        self.ending
    }
}

/// All-or-error derived notes and their retained source. This is not a project commit.
#[must_use]
pub struct ProjectedTake<'a> {
    raw: NoteCaptureResult<'a>,
    table: ProjectionTable,
    notes: Box<[ProjectedNote]>,
    bytes: PreparedBytes,
}
impl Drop for ProjectedTake<'_> {
    fn drop(&mut self) {
        // Drop checking must hold the recorder borrow until the owned arrays are freed.
        // Without this destructor, NLL ends the borrow at the last use of the view,
        // allowing another projection while these allocations still await scope exit.
    }
}
impl ProjectedTake<'_> {
    pub const fn raw(&self) -> &NoteCaptureResult<'_> {
        &self.raw
    }
    pub fn notes(&self) -> &[ProjectedNote] {
        &self.notes
    }
    /// Aggregate recorder, table, derived-note storage and projection descriptor bytes.
    pub const fn bytes_reserved(&self) -> PreparedBytes {
        self.bytes
    }
    pub fn certified_ticks(&self) -> ProjectionTickCount {
        ProjectionTickCount::measured(self.table.len() as u64)
    }
    /// A scoped lookup for diagnostics, using original nominal time, never audition time.
    pub fn project_stamp(
        &self,
        session: CaptureSessionId,
        stamp: CaptureStamp,
    ) -> Result<TickProjection, ProjectionError> {
        if session != self.raw.session() || stamp.epoch() != self.raw.window().epoch() {
            return Err(ProjectionError::ForeignContext);
        }
        self.table.lookup_time(self.raw.context(), stamp.nominal())
    }
}

impl SimulatedNoteRecorder {
    /// Certify and project a sealed take off-thread. Failure changes neither raw capture
    /// nor its reservations. Partial/interrupted takes require a future explicit recovery
    /// selection; sustain/expression require a target that can represent them.
    /// No second projection or capture mutation can coexist with this retained view:
    ///
    /// ```compile_fail
    /// use synth_engine_v2::recording::{TakeReservation, notes::SimulatedNoteRecorder};
    /// fn two(recorder: &mut SimulatedNoteRecorder, ticket: TakeReservation) {
    ///     let first = recorder.project_notes(ticket).unwrap();
    ///     let second = recorder.project_notes(ticket).unwrap();
    ///     println!("{} {}", first.notes().len(), second.notes().len());
    /// }
    /// ```
    /// The loan also survives the view's last use, until its actual destruction:
    ///
    /// ```compile_fail
    /// use synth_engine_v2::recording::{TakeReservation, notes::SimulatedNoteRecorder};
    /// fn retained_allocation(recorder: &mut SimulatedNoteRecorder, ticket: TakeReservation) {
    ///     let first = recorder.project_notes(ticket).unwrap();
    ///     println!("{}", first.notes().len());
    ///     let second = recorder.project_notes(ticket).unwrap();
    ///     println!("{}", second.notes().len());
    /// }
    /// ```
    pub fn project_notes(
        &mut self,
        ticket: TakeReservation,
    ) -> Result<ProjectedTake<'_>, ProjectionError> {
        let raw = self.result(ticket)?;
        validate_material(&raw)?;
        let count = selected_onsets(&raw).count();
        let capture = self
            .limits
            .capture()
            .ok_or(CaptureError::MissingConfiguration)?;
        let ticks =
            ProjectionTable::count(raw.context().interval(), capture.max_projection_ticks())?;
        let bytes = projection_bytes(self.bytes_reserved(), ticks, count)?;
        super::check_bytes(bytes.get(), self.limits)?;
        let table = ProjectionTable::prepare(
            raw.context(),
            ticks,
            AllocationBudget {
                occupied: projection_bytes(self.bytes_reserved(), 0, count)?,
                limit: capture.max_capture_bytes(),
            },
        )?;
        let mut notes = AllocationBudget {
            occupied: projection_bytes(self.bytes_reserved(), ticks, 0)?,
            limit: capture.max_capture_bytes(),
        }
        .reserve::<ProjectedNote>(count)?;
        for cell in selected_onsets(&raw) {
            notes.push(project_note(&raw, &table, cell)?);
        }
        Ok(ProjectedTake {
            raw,
            table,
            notes: notes.into_boxed_slice(),
            bytes,
        })
    }
}

/// Both allocations check requested and granted capacity against the same aggregate law.
#[derive(Clone, Copy)]
struct AllocationBudget {
    occupied: PreparedBytes,
    limit: PreparedBytes,
}
impl AllocationBudget {
    fn check<T>(self, capacity: usize) -> Result<(), ProjectionError> {
        let required = PreparedBytes::measured(super::add_bytes(
            self.occupied.get(),
            super::array_bytes::<T>(capacity)?,
        )?);
        if required > self.limit {
            return Err(CaptureError::ByteBudget {
                required,
                available: self.limit,
            }
            .into());
        }
        Ok(())
    }
    fn reserve<T>(self, count: usize) -> Result<Vec<T>, ProjectionError> {
        self.check::<T>(count)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| CaptureError::Allocation)?;
        self.check::<T>(values.capacity())?;
        Ok(values)
    }
}

fn projection_bytes(
    retained: PreparedBytes,
    ticks: usize,
    notes: usize,
) -> Result<PreparedBytes, NoteCaptureError> {
    let bytes = super::add_bytes(retained.get(), size_of::<ProjectedTake<'_>>() as u64)?;
    let bytes = super::add_bytes(bytes, super::array_bytes::<PlanPosition>(ticks)?)?;
    Ok(PreparedBytes::measured(super::add_bytes(
        bytes,
        super::array_bytes::<ProjectedNote>(notes)?,
    )?))
}

fn validate_material(raw: &NoteCaptureResult<'_>) -> Result<(), ProjectionError> {
    if raw.effective_outcome() != CaptureOutcome::Complete {
        return Err(ProjectionError::Incomplete {
            outcome: raw.effective_outcome(),
        });
    }
    // An empty cancelled count-in does not consume its arm-time controller snapshot.
    if raw.window().start() == raw.window().end() {
        return Ok(());
    }
    for source in raw.initial_sources() {
        if source.controls != ControllerSnapshot::neutral() {
            return Err(ProjectionError::UnsupportedControllers {
                connection: source.source,
            });
        }
    }
    for record in raw.selected_records() {
        if matches!(
            record.input().event(),
            Midi1Event::Sustain { .. } | Midi1Event::PitchBend { .. }
        ) {
            return Err(ProjectionError::UnsupportedControllers {
                connection: record.source(),
            });
        }
    }
    Ok(())
}

fn selected_onsets<'a>(raw: &'a NoteCaptureResult<'_>) -> impl Iterator<Item = &'a NoteCell> {
    let window = raw.window();
    raw.raw
        .cells(CaptureBuffer::Ordinary)
        .iter()
        .flatten()
        .filter(move |cell| {
            matches!(cell, NoteCell::Input { record, .. }
            if matches!(record.input.event, Midi1Event::NoteOn { .. })
                && record.stamp.nominal >= window.start()
                && record.stamp.nominal < window.end())
        })
}

fn project_note(
    raw: &NoteCaptureResult<'_>,
    table: &ProjectionTable,
    cell: &NoteCell,
) -> Result<ProjectedNote, ProjectionError> {
    let NoteCell::Input {
        record,
        release,
        paired_release,
        closure,
        ..
    } = cell
    else {
        return Err(ProjectionError::InvalidCapturedNote);
    };
    let occurrence = record
        .occurrence
        .ok_or(ProjectionError::InvalidCapturedNote)?;
    let Midi1Event::NoteOn { key, velocity } = record.input.event else {
        return Err(ProjectionError::InvalidCapturedNote);
    };
    // A backwards paired release must not be hidden by finalization's synthetic cut.
    if paired_release.is_some_and(|at| at <= record.stamp.nominal)
        || release.is_some_and(|at| at <= record.stamp.nominal)
    {
        return Err(ProjectionError::InvalidLifetime { occurrence });
    }
    let (end, ending) = match (release, closure) {
        (Some(at), _) if *at < raw.window().end() => (*at, ProjectedEnding::KeyRelease),
        (_, Some(cut)) => (
            cut.time,
            ProjectedEnding::Synthetic {
                reason: cut.reason,
                key_held: cut.key_held,
                pedal_held: cut.pedal_held,
            },
        ),
        _ => return Err(ProjectionError::MissingRelease { occurrence }),
    };
    let onset = table
        .lookup_time(raw.context(), record.stamp.nominal)
        .map_err(|_| ProjectionError::NoteTiming {
            occurrence,
            time: record.stamp.nominal,
        })?;
    let release =
        table
            .lookup_time(raw.context(), end)
            .map_err(|_| ProjectionError::NoteTiming {
                occurrence,
                time: end,
            })?;
    let duration = release
        .tick
        .as_u64()
        .checked_sub(onset.tick.as_u64())
        .filter(|duration| *duration > 0)
        .ok_or(ProjectionError::InvalidLifetime { occurrence })?;
    let start = snap(onset.tick, raw.context().quantization())
        .ok_or(ProjectionError::UnrepresentableNote { occurrence })?;
    let end = start
        .as_u64()
        .checked_add(duration)
        .map(MusicalTick::new)
        .ok_or(ProjectionError::UnrepresentableNote { occurrence })?;
    let interval = raw.context().interval();
    if start < interval.start() || start >= interval.end() || end > interval.end() {
        return Err(ProjectionError::OutsideTarget { occurrence });
    }
    Ok(ProjectedNote {
        occurrence,
        channel: record.input.channel,
        key,
        velocity,
        onset,
        release,
        start,
        end,
        ending,
    })
}

fn snap(tick: MusicalTick, quantization: CaptureQuantization) -> Option<MusicalTick> {
    let CaptureQuantization::Grid(grid) = quantization else {
        return Some(tick);
    };
    let step = grid.ticks().as_u64();
    let remainder = tick.as_u64() % step;
    let lower = tick.as_u64() - remainder;
    if remainder <= step - remainder {
        Some(MusicalTick::new(lower))
    } else {
        lower.checked_add(step).map(MusicalTick::new)
    }
}

#[derive(Debug, PartialEq, Error)]
pub enum ProjectionError {
    #[error(transparent)]
    Capture(#[from] NoteCaptureError),
    #[error(transparent)]
    Storage(#[from] CaptureError),
    #[error(transparent)]
    Time(#[from] TimeError),
    #[error("projection interval count or layout cannot be represented")]
    IntervalOverflow,
    #[error("projection requires {required}, exceeding max_projection_ticks {available}")]
    TickBudget {
        required: ProjectionTickCount,
        available: ProjectionTickCount,
    },
    #[error("tempo conversion failed at {tick}: {error}")]
    Conversion {
        tick: MusicalTick,
        error: TempoError,
    },
    #[error("tempo position decreases from {previous_tick} ({previous}) to {tick} ({position})")]
    Decreasing {
        previous_tick: MusicalTick,
        previous: PlanPosition,
        tick: MusicalTick,
        position: PlanPosition,
    },
    #[error("projection timestamp belongs to another session or epoch")]
    ForeignContext,
    #[error("time {time} is outside the retained projection mapping")]
    OutsideMapping { time: SampleTime },
    #[error("position {position} is outside the certified frame range")]
    OutsideTable { position: PlanPosition },
    #[error("automatic note projection requires complete capture, found {outcome:?}")]
    Incomplete { outcome: CaptureOutcome },
    #[error("note-only projection cannot represent sustain or expression from {connection:?}")]
    UnsupportedControllers { connection: ConnectionGeneration },
    #[error("captured onset has no valid note or occurrence")]
    InvalidCapturedNote,
    #[error("occurrence {occurrence:?} has a nonpositive raw or projected lifetime")]
    InvalidLifetime { occurrence: PerformedOccurrenceId },
    #[error("occurrence {occurrence:?} has neither a selected release nor a synthetic closure")]
    MissingRelease { occurrence: PerformedOccurrenceId },
    #[error("occurrence {occurrence:?} has unrepresentable musical timing")]
    UnrepresentableNote { occurrence: PerformedOccurrenceId },
    #[error(
        "occurrence {occurrence:?} lies outside the allowed musical interval after quantization"
    )]
    OutsideTarget { occurrence: PerformedOccurrenceId },
    #[error("occurrence {occurrence:?}: time {time} cannot be projected in the retained mapping")]
    NoteTiming {
        occurrence: PerformedOccurrenceId,
        time: SampleTime,
    },
}
