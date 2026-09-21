//! Bounded group release through the latched live producer, with no voice stealing.
use super::{IngressRefused, PerformanceIngress};
use crate::{
    identity::{IdentityTable, NoteIdentity, Resolution, TableId},
    publish::ProducerClass,
    quantities::EventCount,
    render::EventPayload,
    time::SampleTime,
};

pub(crate) const RELEASE_GROUP_CAPACITY: usize = 8;

/// The source operation owns one publication charge, regardless of its voice fanout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseCause {
    SustainLift,
    Panic,
    Stop,
}
impl ReleaseCause {
    pub(crate) const fn class(self) -> ProducerClass {
        match self {
            Self::Stop => ProducerClass::Session,
            Self::SustainLift | Self::Panic => ProducerClass::Live,
        }
    }
}

/// Created only after the live store atomically admits all affected obligations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ReleaseGroup {
    table: TableId,
    indices: [u16; RELEASE_GROUP_CAPACITY],
    generations: [u32; RELEASE_GROUP_CAPACITY],
    len: u8,
    cause: ReleaseCause,
}
impl ReleaseGroup {
    pub fn identities(&self) -> impl Iterator<Item = NoteIdentity> + '_ {
        (0..usize::from(self.len)).filter_map(|index| self.identity(index))
    }
    pub(crate) fn identity(&self, index: usize) -> Option<NoteIdentity> {
        if index >= usize::from(self.len) {
            return None;
        }
        Some(NoteIdentity {
            table: self.table,
            index: *self.indices.get(index)?,
            generation: *self.generations.get(index)?,
        })
    }
    pub const fn cause(self) -> ReleaseCause {
        self.cause
    }
}

impl PerformanceIngress {
    pub(crate) fn offer_release_group(
        &mut self,
        table: &mut IdentityTable,
        at: SampleTime,
        identities: &[NoteIdentity],
        cause: ReleaseCause,
    ) -> Result<(), IngressRefused> {
        if identities.len() > RELEASE_GROUP_CAPACITY
            || self.stealing != crate::ir::StealingPolicy::None
            || self.pending_len != 0
            || identities.len() > self.holds_outstanding.get() as usize
        {
            return Err(IngressRefused::ReleaseGroup);
        }
        self.admit(at)?;
        // No mutation until every member, count and ownership check has succeeded.
        // The exclusive caller prevents any intervening mint or release.
        for (index, identity) in identities.iter().copied().enumerate() {
            if identities[..index].contains(&identity)
                || table.resolve_for(self.producer, identity) != Resolution::Live
            {
                return Err(IngressRefused::ReleaseGroup);
            }
        }
        let Some(first) = identities.first().copied() else {
            return Ok(());
        };
        let len = u8::try_from(identities.len()).map_err(|_| IngressRefused::ReleaseGroup)?;
        let mut group = ReleaseGroup {
            table: first.table(),
            indices: [0; RELEASE_GROUP_CAPACITY],
            generations: [0; RELEASE_GROUP_CAPACITY],
            len,
            cause,
        };
        for (index, identity) in identities.iter().copied().enumerate() {
            // Preflight established this same identity's producer membership and live
            // state. Each is distinct; releasing one cannot invalidate another.
            let resolution = table.release_for(self.producer, identity);
            if resolution != Resolution::Live {
                return Err(IngressRefused::ReleaseGroup);
            }
            group.indices[index] = identity.index;
            group.generations[index] = identity.generation;
        }
        let count = u32::from(len);
        self.holds_outstanding = EventCount::measured(self.holds_outstanding.get() - count);
        // N reserved release slots become one source event; no extra queue capacity
        // is requested, even when all ordinary slots were full before this operation.
        self.enqueue_entry(at, EventPayload::ReleaseGroup(group), false);
        Ok(())
    }
}
