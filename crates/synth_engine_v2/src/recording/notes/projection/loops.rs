//! Off-thread projection of retained loop passes; continuation is not a new attack.
use super::*;
use crate::{
    recording::notes::{CapturePassId, loop_capture::LoopCapturePass},
    time::FrameCount,
};

#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ProjectedLoopNote {
    pass: CapturePassId,
    note: ProjectedNote,
    attack: bool,
    continues: bool,
}
impl ProjectedLoopNote {
    pub const fn pass(self) -> CapturePassId {
        self.pass
    }
    pub const fn note(self) -> ProjectedNote {
        self.note
    }
    pub const fn attack(self) -> bool {
        self.attack
    }
    pub const fn continues(self) -> bool {
        self.continues
    }
}

/// Every segment keeps its pass and original occurrence. Replace/overdub intent
/// remains the immutable raw context; choosing or committing passes is off-thread.
#[must_use]
pub struct ProjectedLoopTake<'a> {
    raw: NoteCaptureResult<'a>,
    table: ProjectionTable,
    notes: Box<[ProjectedLoopNote]>,
    bytes: PreparedBytes,
}
impl Drop for ProjectedLoopTake<'_> {
    fn drop(&mut self) {} // Retain the exclusive recorder loan through storage destruction.
}
impl ProjectedLoopTake<'_> {
    pub const fn raw(&self) -> &NoteCaptureResult<'_> {
        &self.raw
    }
    pub fn notes(&self) -> &[ProjectedLoopNote] {
        &self.notes
    }
    pub const fn bytes_reserved(&self) -> PreparedBytes {
        self.bytes
    }
    pub fn certified_ticks(&self) -> ProjectionTickCount {
        ProjectionTickCount::measured(self.table.len() as u64)
    }
}

fn bytes(
    retained: PreparedBytes,
    ticks: usize,
    notes: usize,
) -> Result<PreparedBytes, NoteCaptureError> {
    let total = super::super::add_bytes(retained.get(), size_of::<ProjectedLoopTake<'_>>() as u64)?;
    let total = super::super::add_bytes(total, super::super::array_bytes::<PlanPosition>(ticks)?)?;
    Ok(PreparedBytes::measured(super::super::add_bytes(
        total,
        super::super::array_bytes::<ProjectedLoopNote>(notes)?,
    )?))
}

fn lifetime(
    cell: &NoteCell,
    raw: &NoteCaptureResult<'_>,
) -> Result<(super::super::RecordedInput, SampleTime, ProjectedEnding), ProjectionError> {
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
    if paired_release.is_some_and(|time| time <= record.stamp.nominal)
        || release.is_some_and(|time| time <= record.stamp.nominal)
    {
        return Err(ProjectionError::InvalidLifetime { occurrence });
    }
    let (end, ending) = match (release, closure) {
        (Some(time), _) if *time < raw.window().end() => (*time, ProjectedEnding::KeyRelease),
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
    if end <= record.stamp.nominal {
        return Err(ProjectionError::InvalidLifetime { occurrence });
    }
    Ok((*record, end, ending))
}

fn position(pass: &LoopCapturePass, at: SampleTime) -> Result<PlanPosition, ProjectionError> {
    let frames = at
        .as_u64()
        .checked_sub(pass.window().start().as_u64())
        .ok_or(ProjectionError::OutsideMapping { time: at })?;
    Ok(pass.position().checked_add(FrameCount::new(frames))?)
}

fn segment(
    raw: &NoteCaptureResult<'_>,
    table: &ProjectionTable,
    pass: &LoopCapturePass,
    cell: &NoteCell,
) -> Result<Option<ProjectedLoopNote>, ProjectionError> {
    let (record, raw_end, ending) = lifetime(cell, raw)?;
    let begin = record.stamp.nominal.max(pass.window().start());
    let end = raw_end.min(pass.window().end());
    if begin >= end {
        return Ok(None);
    }
    let occurrence = record
        .occurrence
        .ok_or(ProjectionError::InvalidCapturedNote)?;
    let Midi1Event::NoteOn { key, velocity } = record.input.event else {
        return Err(ProjectionError::InvalidCapturedNote);
    };
    let attack = begin == record.stamp.nominal;
    let onset = table.lookup(position(pass, begin)?)?;
    let release = table.lookup(position(pass, end)?)?;
    let duration = release
        .tick
        .as_u64()
        .checked_sub(onset.tick.as_u64())
        .filter(|n| *n > 0)
        .ok_or(ProjectionError::InvalidLifetime { occurrence })?;
    let start = if attack {
        snap(onset.tick, raw.context().quantization())
    } else {
        Some(onset.tick)
    }
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
    // A continued segment must still reach its loop edge after onset snapping.
    // Refuse the whole projection rather than invent a gap or a new attack.
    let continues = raw_end > pass.window().end();
    if continues && end != interval.end() {
        return Err(ProjectionError::OutsideTarget { occurrence });
    }
    let ending = if continues {
        ProjectedEnding::LoopContinuation
    } else {
        ending
    };
    Ok(Some(ProjectedLoopNote {
        pass: pass.id(),
        attack,
        continues,
        note: ProjectedNote {
            occurrence,
            channel: record.input.channel,
            key,
            velocity,
            onset,
            release,
            start,
            end,
            ending,
        },
    }))
}

impl SimulatedNoteRecorder {
    pub fn project_loop_notes(
        &mut self,
        ticket: TakeReservation,
    ) -> Result<ProjectedLoopTake<'_>, ProjectionError> {
        let raw = self.result(ticket)?;
        if !matches!(raw.context().mapping(), super::super::NoteMapping::Loop(_)) {
            return Err(ProjectionError::LoopMapping);
        }
        validate_material(&raw)?;
        let capture = self
            .limits
            .capture()
            .ok_or(CaptureError::MissingConfiguration)?;
        let ticks =
            ProjectionTable::count(raw.context().interval(), capture.max_projection_ticks())?;
        let mut count = 0_usize;
        for cell in selected_onsets(&raw) {
            let (record, end, _) = lifetime(cell, &raw)?;
            for pass in raw.loop_passes() {
                if record.stamp.nominal < pass.window().end() && end > pass.window().start() {
                    count = count
                        .checked_add(1)
                        .ok_or(ProjectionError::IntervalOverflow)?;
                }
            }
        }
        let total = bytes(self.bytes_reserved(), ticks, count)?;
        super::super::check_bytes(total.get(), self.limits)?;
        let table = ProjectionTable::prepare(
            raw.context(),
            ticks,
            AllocationBudget {
                occupied: bytes(self.bytes_reserved(), 0, count)?,
                limit: capture.max_capture_bytes(),
            },
        )?;
        let mut notes = AllocationBudget {
            occupied: bytes(self.bytes_reserved(), ticks, 0)?,
            limit: capture.max_capture_bytes(),
        }
        .reserve::<ProjectedLoopNote>(count)?;
        for pass in raw.loop_passes() {
            for cell in selected_onsets(&raw) {
                if let Some(note) = segment(&raw, &table, pass, cell)? {
                    notes.push(note);
                }
            }
        }
        Ok(ProjectedLoopTake {
            raw,
            table,
            notes: notes.into_boxed_slice(),
            bytes: total,
        })
    }
}
