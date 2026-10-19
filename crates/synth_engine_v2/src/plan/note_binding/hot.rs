//! Actual note binding lookup and prevalidated application without allocation.
use super::{NoteBindings, NoteSlot, PlanId};
impl NoteBindings {
    pub(crate) const fn old_plan(&self) -> PlanId {
        self.old
    }
    pub(crate) const fn new_plan(&self) -> PlanId {
        self.new
    }
    pub(crate) fn note(&self, old: NoteSlot) -> Option<NoteSlot> {
        if old.plan() != self.old {
            return None;
        }
        let mapped = *self.destinations.get(old.index())?.as_ref()?;
        let inverse = *self.sources.get(mapped.index())?.as_ref()?;
        if inverse != old {
            return None;
        }
        let index = u32::try_from(mapped.index()).ok()?;
        (mapped.plan() == self.new && index < self.destination_count.get()).then_some(mapped)
    }
    pub(crate) fn accepts_note(&self, note: NoteSlot) -> bool {
        self.note(note).is_some()
    }
    /// The caller has checked every actual entry against this immutable map.
    pub(crate) fn apply_note(&self, note: &mut NoteSlot) {
        if let Some(mapped) = self.note(*note) {
            *note = mapped;
        }
    }
}
