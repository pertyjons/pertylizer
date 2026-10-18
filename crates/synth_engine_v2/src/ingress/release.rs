//! Bounded group release through the latched live producer, with no voice stealing.
use super::{IngressRefused, PerformanceIngress};
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use crate::identity::LiveRangeMinter;
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
    /// One group of a mixed Panic's cut of the compiled producer's open occurrences.
    /// No compiled occurrence reserved a live release place, so it spends the
    /// guaranteed-release share, which the mixed arm sizes for the full live backlog plus
    /// one cut of the whole compiled span. A live store never offers it.
    CompiledPanic,
    /// A mixed whole-owner reset's group: every live occurrence of the fading owner,
    /// ended at the end of its fade. It is neither a transport Stop nor a Panic. Each
    /// member redeems the hold it reserved, so the group spends the guaranteed-release
    /// share, in a quantum where the fading owner takes no other input. Only the mixed
    /// owner offers it; the exclusive live path refuses it.
    Reset,
}
impl ReleaseCause {
    pub(crate) const fn class(self) -> ProducerClass {
        match self {
            Self::Stop => ProducerClass::Session,
            Self::SustainLift | Self::Panic => ProducerClass::Live,
            Self::CompiledPanic | Self::Reset => ProducerClass::Release,
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

    /// One group of a mixed Panic's compiled cut: one to eight distinct identities of
    /// one table, which the audio-owned compiled minter has already released.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) fn compiled_panic(identities: &[NoteIdentity]) -> Option<Self> {
        let first = identities.first().copied()?;
        let len = u8::try_from(identities.len()).ok()?;
        let mut group = Self {
            table: first.table(),
            indices: [0; RELEASE_GROUP_CAPACITY],
            generations: [0; RELEASE_GROUP_CAPACITY],
            len,
            cause: ReleaseCause::CompiledPanic,
        };
        for (index, identity) in identities.iter().copied().enumerate() {
            if identity.table() != first.table() || identities[..index].contains(&identity) {
                return None;
            }
            *group.indices.get_mut(index)? = identity.index;
            *group.generations.get_mut(index)? = identity.generation;
        }
        Some(group)
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
            || cause == ReleaseCause::CompiledPanic
            || cause == ReleaseCause::Reset
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

    /// The stamp a mixed Panic is queued at: its nominal stamp, or the last stamp
    /// this store accepted when that is later, so the order fence cannot refuse it.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) const fn mixed_execution_stamp(&self, nominal: SampleTime) -> SampleTime {
        match self.last_accepted {
            Some(last) if last.as_u64() > nominal.as_u64() => last,
            Some(_) | None => nominal,
        }
    }

    /// Test-only mixed group release through the audio owner's live range, with the
    /// same preflight, hold conversion and single queue entry as the exclusive path.
    /// A member's undrained bend entries keep their credits until they drain.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(crate) fn offer_mixed_release_group(
        &mut self,
        minter: &mut LiveRangeMinter,
        at: SampleTime,
        identities: &[NoteIdentity],
        cause: ReleaseCause,
    ) -> Result<(), IngressRefused> {
        if identities.len() > RELEASE_GROUP_CAPACITY
            || cause == ReleaseCause::CompiledPanic
            || self.stealing != crate::ir::StealingPolicy::None
            || self.pending_len != 0
            || identities.len() > self.holds_outstanding.get() as usize
        {
            return Err(IngressRefused::ReleaseGroup);
        }
        self.admit(at)?;
        for (index, identity) in identities.iter().copied().enumerate() {
            if identities[..index].contains(&identity)
                || minter.resolve(identity) != Resolution::Live
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
            if minter.release(identity) != Resolution::Live {
                return Err(IngressRefused::ReleaseGroup);
            }
            group.indices[index] = identity.index;
            group.generations[index] = identity.generation;
        }
        let count = u32::from(len);
        self.holds_outstanding = EventCount::measured(self.holds_outstanding.get() - count);
        self.enqueue_entry(at, EventPayload::ReleaseGroup(group), false);
        Ok(())
    }
}
