use super::{AllocationBudget, ProjectionError, TickProjection};
use crate::quantities::ProjectionTickCount;
use crate::recording::notes::{MusicalInterval, NoteArmContext};
use crate::tempo::{MusicalTick, TempoError};
use crate::time::{FrameCount, PlanPosition, SampleTime};

pub(super) struct ProjectionTable {
    start: MusicalTick,
    positions: Box<[PlanPosition]>,
}
impl ProjectionTable {
    pub(super) fn count(
        interval: MusicalInterval,
        limit: ProjectionTickCount,
    ) -> Result<usize, ProjectionError> {
        let count = interval
            .end()
            .as_u64()
            .checked_sub(interval.start().as_u64())
            .and_then(|n| n.checked_add(1))
            .ok_or(ProjectionError::IntervalOverflow)?;
        if count > limit.get() {
            return Err(ProjectionError::TickBudget {
                required: ProjectionTickCount::measured(count),
                available: limit,
            });
        }
        let count = usize::try_from(count).map_err(|_| ProjectionError::IntervalOverflow)?;
        super::super::array_bytes::<PlanPosition>(count)?;
        Ok(count)
    }
    pub(super) fn prepare(
        context: &NoteArmContext,
        count: usize,
        budget: AllocationBudget,
    ) -> Result<Self, ProjectionError> {
        Self::enumerate(context.interval().start(), count, budget, |tick| {
            context.tempo().position_of(tick)
        })
    }
    // The production forward law and injected inversion tests use this same exhaustive pass.
    fn enumerate(
        start: MusicalTick,
        count: usize,
        budget: AllocationBudget,
        mut forward: impl FnMut(MusicalTick) -> Result<PlanPosition, TempoError>,
    ) -> Result<Self, ProjectionError> {
        let mut positions = budget.reserve::<PlanPosition>(count)?;
        for offset in 0..count {
            let tick = MusicalTick::new(
                start
                    .as_u64()
                    .checked_add(offset as u64)
                    .ok_or(ProjectionError::IntervalOverflow)?,
            );
            let position =
                forward(tick).map_err(|error| ProjectionError::Conversion { tick, error })?;
            if let Some(&previous) = positions.last()
                && position < previous
            {
                return Err(ProjectionError::Decreasing {
                    previous_tick: MusicalTick::new(tick.as_u64() - 1),
                    previous,
                    tick,
                    position,
                });
            }
            positions.push(position);
        }
        Ok(Self {
            start,
            positions: positions.into_boxed_slice(),
        })
    }
    pub(super) fn len(&self) -> usize {
        self.positions.len()
    }
    pub(super) fn lookup_time(
        &self,
        context: &NoteArmContext,
        time: SampleTime,
    ) -> Result<TickProjection, ProjectionError> {
        let anchor = context.anchor().ok_or(ProjectionError::LoopMapping)?;
        let frames = time
            .as_u64()
            .checked_sub(anchor.time().as_u64())
            .ok_or(ProjectionError::OutsideMapping { time })?;
        let position = anchor.position().checked_add(FrameCount::new(frames))?;
        // Hold StreamAnchor's signed-difference limit as well as its forward-only rule.
        if anchor.time_of(position) != Some(time) {
            return Err(ProjectionError::OutsideMapping { time });
        }
        self.lookup(position)
    }
    fn lookup(&self, position: PlanPosition) -> Result<TickProjection, ProjectionError> {
        let Some((&first, &last)) = self.positions.first().zip(self.positions.last()) else {
            return Err(ProjectionError::OutsideTable { position });
        };
        if position < first || position > last {
            return Err(ProjectionError::OutsideTable { position });
        }
        let upper = self.positions.partition_point(|frame| *frame < position);
        // The range check proves upper exists; get keeps that proof explicit at access.
        let high = self
            .positions
            .get(upper)
            .copied()
            .ok_or(ProjectionError::OutsideTable { position })?;
        let selected = if let Some(low) = upper.checked_sub(1).and_then(|i| self.positions.get(i)) {
            if position.as_u64() - low.as_u64() <= high.as_u64() - position.as_u64() {
                *low
            } else {
                high
            }
        } else {
            high
        };
        // The lower candidate may be the *last* tick in a plateau. Resolve both tie laws.
        let earliest = self.positions.partition_point(|frame| *frame < selected);
        Ok(TickProjection {
            tick: MusicalTick::new(self.start.as_u64() + earliest as u64),
            error: selected.difference(position)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quantities::PreparedBytes;

    fn budget() -> AllocationBudget {
        AllocationBudget {
            occupied: PreparedBytes::NONE,
            limit: PreparedBytes::measured(1024),
        }
    }

    #[test]
    fn exhaustive_pass_refuses_interior_inversion_and_conversion_failure() {
        let positions = [0, 10, 20, 19, 40, 50, 60, 70, 80];
        let result =
            ProjectionTable::enumerate(MusicalTick::ZERO, positions.len(), budget(), |tick| {
                Ok(PlanPosition::new(
                    positions[usize::try_from(tick.as_u64()).unwrap()],
                ))
            });
        assert!(
            matches!(result, Err(ProjectionError::Decreasing { previous_tick, tick, .. })
            if previous_tick == MusicalTick::new(2) && tick == MusicalTick::new(3))
        );
        let result = ProjectionTable::enumerate(MusicalTick::new(10), 9, budget(), |tick| {
            if tick == MusicalTick::new(13) {
                Err(TempoError::PositionNotExactlyRepresentable { tick })
            } else {
                Ok(PlanPosition::new(tick.as_u64()))
            }
        });
        assert!(
            matches!(result, Err(ProjectionError::Conversion { tick, .. })
            if tick == MusicalTick::new(13))
        );
    }

    #[test]
    fn nearest_ties_and_plateaus_choose_earliest_tick_in_interval() {
        let frames = [2, 2, 4, 4, 4, 8, 8];
        let table =
            ProjectionTable::enumerate(MusicalTick::new(5), frames.len(), budget(), |tick| {
                Ok(PlanPosition::new(
                    frames[usize::try_from(tick.as_u64() - 5).unwrap()],
                ))
            })
            .unwrap();
        for frame in 2..=8 {
            let (index, best) = frames
                .iter()
                .enumerate()
                .min_by_key(|(index, at)| (at.abs_diff(frame), *index))
                .unwrap();
            let actual = table.lookup(PlanPosition::new(frame)).unwrap();
            assert_eq!(actual.tick(), MusicalTick::new(5 + index as u64));
            assert_eq!(
                actual.error(),
                PlanPosition::new(*best)
                    .difference(PlanPosition::new(frame))
                    .unwrap()
            );
        }
        for frame in [1, 9] {
            assert!(matches!(
                table.lookup(PlanPosition::new(frame)),
                Err(ProjectionError::OutsideTable { .. })
            ));
        }
    }
}
