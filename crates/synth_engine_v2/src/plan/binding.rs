//! Complete fixed-index plan relabelling, separately validated before any mutation.
use super::{CompiledPlan, ContinuationError, NoteSlot, ParameterSlot, PlanId};
use crate::{
    quantities::SlotCount,
    render::{EventPayload, NoteEdge, TimedEvent},
};

#[derive(Clone, Copy, Debug)]
#[must_use]
pub(crate) struct FixedPlanBindings {
    from: PlanId,
    to: PlanId,
    notes: SlotCount,
    parameters: SlotCount,
}

impl FixedPlanBindings {
    pub(crate) fn prepare(
        from: &CompiledPlan,
        to: &CompiledPlan,
    ) -> Result<Self, ContinuationError> {
        if from.id() == to.id()
            || from.note_targets().len() != to.note_targets().len()
            || from.parameter_targets().len() != to.parameter_targets().len()
        {
            return Err(ContinuationError::Binding);
        }
        Ok(Self {
            from: from.id(),
            to: to.id(),
            notes: SlotCount::measured(
                u32::try_from(from.note_targets().len()).map_err(|_| ContinuationError::Binding)?,
            ),
            parameters: SlotCount::measured(
                u32::try_from(from.parameter_targets().len())
                    .map_err(|_| ContinuationError::Binding)?,
            ),
        })
    }

    pub(crate) const fn old_plan(self) -> PlanId {
        self.from
    }
    pub(crate) const fn new_plan(self) -> PlanId {
        self.to
    }
    pub(crate) fn accepts_note(self, slot: NoteSlot) -> bool {
        let Ok(index) = u32::try_from(slot.index()) else {
            return false;
        };
        slot.plan() == self.from && index < self.notes.get()
    }
    pub(crate) fn accepts_parameter(self, slot: ParameterSlot) -> bool {
        let Ok(index) = u32::try_from(slot.index()) else {
            return false;
        };
        slot.plan() == self.from && index < self.parameters.get()
    }
    /// The caller preflighted this actual slot; no new authority is constructed.
    pub(crate) const fn note(self, slot: NoteSlot) -> NoteSlot {
        NoteSlot::new(self.to, slot.index())
    }
    /// The caller preflighted this actual slot and its unchanged parameter group.
    pub(crate) const fn parameter(self, slot: ParameterSlot) -> ParameterSlot {
        ParameterSlot::new(self.to, slot.index())
    }

    pub(crate) fn accepts_event(self, event: TimedEvent) -> bool {
        match event.payload() {
            EventPayload::SetParameter { slot, .. } => self.accepts_parameter(slot),
            EventPayload::Controller(change) => self.accepts_parameter(change.slot().parameter()),
            EventPayload::RestoreController(restore) => {
                self.accepts_parameter(restore.slot.parameter())
            }
            EventPayload::ScopedRestore(restore) => self.accepts_parameter(restore.slot()),
            EventPayload::Note {
                edge: NoteEdge::On { slot, .. },
                ..
            } => self.accepts_note(slot),
            EventPayload::Note {
                edge: NoteEdge::Off,
                ..
            }
            | EventPayload::ReleaseGroup(_)
            | EventPayload::Fade { .. }
            | EventPayload::Reset { .. }
            | EventPayload::Expression { .. }
            | EventPayload::Bend { .. } => true,
        }
    }

    /// Apply only after every prospective owner has passed preflight.
    pub(crate) fn event(self, event: TimedEvent) -> TimedEvent {
        let payload = match event.payload() {
            EventPayload::SetParameter { slot, value } => EventPayload::SetParameter {
                slot: self.parameter(slot),
                value,
            },
            EventPayload::Controller(mut change) => {
                change.apply_plan_bindings(self);
                EventPayload::Controller(change)
            }
            EventPayload::RestoreController(mut restore) => {
                restore.apply_plan_bindings(self);
                EventPayload::RestoreController(restore)
            }
            EventPayload::ScopedRestore(mut restore) => {
                restore.apply_plan_bindings(self);
                EventPayload::ScopedRestore(restore)
            }
            EventPayload::Note {
                identity,
                edge:
                    NoteEdge::On {
                        slot,
                        key,
                        velocity,
                    },
            } => EventPayload::Note {
                identity,
                edge: NoteEdge::On {
                    slot: self.note(slot),
                    key,
                    velocity,
                },
            },
            unchanged @ (EventPayload::Note {
                edge: NoteEdge::Off,
                ..
            }
            | EventPayload::ReleaseGroup(_)
            | EventPayload::Fade { .. }
            | EventPayload::Reset { .. }
            | EventPayload::Expression { .. }
            | EventPayload::Bend { .. }) => unchanged,
        };
        TimedEvent::new(event.envelope(), payload)
    }
}
