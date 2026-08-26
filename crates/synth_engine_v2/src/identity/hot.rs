//! The identity table's audio-thread path: resolving and releasing.
//!
//! Like the other `hot.rs` files, this one may not allocate, lock, perform I/O, log, or
//! panic. It indexes storage the table already holds and writes nothing new.
//!
//! **Minting is not here.** `HOST-INV-009` puts the atomic slot-hold-identity acquisition at
//! the live boundary, which is off the audio thread, and it allocates nothing there either —
//! but it belongs with construction rather than with the loop that consumes its output.

use super::{
    IdentityTable, LiveNote, LiveNotes, NoteIdentity, OrphanCause, ReleaseScope, Resolution, Slot,
};
use crate::plan::NoteSlot;

impl IdentityTable {
    /// The node an identity's note plays, if it names a live one.
    ///
    /// This is what lets a release carry the identity alone: the node was named at the
    /// note-on, when the occurrence was created, so nothing later has to carry it. An
    /// identity that names no live note has no node either — which is the same refusal
    /// [`Self::resolve`] gives, and not a reason to guess one.
    pub fn note_of(&self, identity: NoteIdentity) -> Option<NoteSlot> {
        if identity.table != self.id {
            return None;
        }
        match self.slots.get(usize::from(identity.index)) {
            Some(Slot::Live { generation, note }) if *generation == identity.generation => {
                Some(*note)
            }
            _ => None,
        }
    }

    /// What an identity names.
    pub fn resolve(&self, identity: NoteIdentity) -> Resolution {
        if identity.table != self.id {
            return Resolution::ForeignTable {
                minted_by: identity.table,
            };
        }
        match self.slots.get(usize::from(identity.index)) {
            Some(Slot::Live { generation, .. }) if *generation == identity.generation => {
                Resolution::Live
            }
            Some(Slot::Live { .. }) => Resolution::Orphan(OrphanCause::SupersededGeneration),
            Some(Slot::Retired) => Resolution::Orphan(OrphanCause::RetiredIndex),
            Some(Slot::Free { .. }) | None => Resolution::Orphan(OrphanCause::FreeIndex),
        }
    }

    /// Release the note an identity names, freeing its index for reuse.
    ///
    /// Returns what the identity resolved to. Only [`Resolution::Live`] releases anything:
    /// an orphan is refused, which is ADR-0046 clause 3's "counted rather than allowed to
    /// release another note" made executable.
    ///
    /// **Advancing the generation is what makes the released identity stale**, and it is
    /// where retirement happens: an index whose generation space is spent is withdrawn
    /// instead of restarted, because reusing a generation would let a stale identity match a
    /// live note.
    pub fn release(&mut self, identity: NoteIdentity) -> Resolution {
        let resolution = self.resolve(identity);
        if resolution != Resolution::Live {
            return resolution;
        }
        let index = usize::from(identity.index);
        if let Some(slot) = self.slots.get_mut(index) {
            *slot = if identity.generation >= self.generation_ceiling {
                self.retired = self.retired.saturating_add(1);
                Slot::Retired
            } else {
                Slot::Free {
                    next_generation: identity.generation.saturating_add(1),
                }
            };
            self.live = self.live.saturating_sub(1);
        }
        resolution
    }

    /// Release every live note, as a panic, transport stop or sustain lift does.
    ///
    /// One bounded operation rather than one event per voice, per ADR-0046 clause 6. Every
    /// identity it ends becomes an orphan by the same rule an individual release uses, so a
    /// release arriving afterwards for a note this took is refused rather than double-freed.
    ///
    /// **Scoped**, because clause 6 applies the operation "to owned voices within the source
    /// event". A panic or a transport stop reaches everything; a sustain lift or a
    /// script-driven mass release reaches one producer and must not end another's notes.
    pub fn release_all(&mut self, scope: ReleaseScope) -> u32 {
        let (first, last) = match scope {
            ReleaseScope::Everything => (0_usize, self.slots.len()),
            ReleaseScope::Producer(producer) => {
                let Some(range) = self.ranges.get(usize::from(producer.as_u16())) else {
                    return 0;
                };
                let start = range.start as usize;
                (start, start.saturating_add(range.len as usize))
            }
        };
        let mut ended = 0;
        for index in first..last {
            let Some(slot) = self.slots.get_mut(index) else {
                break;
            };
            if let Slot::Live { generation, .. } = *slot {
                *slot = if generation >= self.generation_ceiling {
                    self.retired = self.retired.saturating_add(1);
                    Slot::Retired
                } else {
                    Slot::Free {
                        next_generation: generation.saturating_add(1),
                    }
                };
                ended += 1;
            }
        }
        self.live = self.live.saturating_sub(ended);
        ended
    }
}

impl LiveNotes {
    /// Which table's occurrences this registry accepts.
    pub const fn id(&self) -> super::TableId {
        self.id
    }

    /// Record that an occurrence is now sounding on `note`.
    ///
    /// The node comes from the **on edge**, not from the minting table: the renderer applies
    /// an event stream, and what that stream says is played is what it plays. An index the
    /// minter has since reissued therefore reads correctly here, because this is written in
    /// the order the events are applied rather than in the order they were stamped.
    ///
    /// Silently overwrites a slot already holding an older generation. That is the reissued
    /// index, and it is the intended case: the older occurrence's release, if one is still in
    /// flight, then resolves as an orphan, which is what it is.
    pub fn admit(&mut self, identity: NoteIdentity, note: crate::plan::NoteSlot) {
        if identity.table != self.id {
            return;
        }
        if let Some(slot) = self.slots.get_mut(usize::from(identity.index)) {
            *slot = Some(LiveNote {
                generation: identity.generation,
                note,
            });
        }
    }

    /// The node an occurrence is sounding on, if it names a live one.
    pub fn note_of(&self, identity: NoteIdentity) -> Option<crate::plan::NoteSlot> {
        if identity.table != self.id {
            return None;
        }
        match self.slots.get(usize::from(identity.index)) {
            Some(Some(live)) if live.generation == identity.generation => Some(live.note),
            _ => None,
        }
    }

    /// End the note an occurrence names, and say what it was.
    ///
    /// Only a live occurrence clears anything: an orphan is refused, which is ADR-0046
    /// clause 3's "counted rather than allowed to release another note" made executable at
    /// the point the release is applied.
    pub fn release(&mut self, identity: NoteIdentity) -> Option<crate::plan::NoteSlot> {
        let note = self.note_of(identity)?;
        if let Some(slot) = self.slots.get_mut(usize::from(identity.index)) {
            *slot = None;
        }
        Some(note)
    }
}
