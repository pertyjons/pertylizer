//! Cold stable-address matching and bounded, allocation-free tap handoff.
use super::{ObservationGeneration, ObservationSubscriptions, StoreId};
use crate::{
    plan::{CompiledPlan, PlanId, TapSlot},
    quantities::PreparedBytes,
};
use thiserror::Error;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ObservationHandoffError {
    #[error("observation handoff generations are exhausted")]
    Exhausted,
    #[error("observation handoff belongs to another store or plan")]
    Origin,
    #[error("observation handoff exceeds its independent metadata budget")]
    Bytes,
    #[error("observation handoff metadata allocation failed")]
    Allocation,
    #[error("observation tap storage or channel geometry is inconsistent")]
    Geometry,
    #[error("observation subscriptions changed after handoff preparation")]
    Changed,
    #[error("the sound transition has no prepared observation mapping")]
    NotPrepared,
    #[error("the host bypassed the observed transition entry")]
    NotObserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ObservationRow {
    pub(super) tap: Option<TapSlot>,
    pub(super) channels: usize,
    pub(super) capacity_frames: usize,
}

#[derive(Debug)]
pub(super) struct HandoffRow {
    pub(super) source: ObservationRow,
    pub(super) destination: Option<TapSlot>,
}

/// Immutable cold mirror: control can prepare maps while audio owns the store.
/// It carries no ring, cursor or samples and cannot modify the actual store.
#[derive(Debug)]
#[must_use]
pub struct ObservationConfiguration {
    store: StoreId,
    generation: ObservationGeneration,
    plan: PlanId,
    rows: Vec<ObservationRow>,
}

/// Prepared off-thread, borrowed at a quantum boundary, reclaimed off-thread.
/// Its independent budget never changes a sound plan's admission.
#[derive(Debug)]
#[must_use]
pub struct ObservationHandoff {
    pub(super) store: StoreId,
    pub(super) generation: ObservationGeneration,
    pub(super) successor: ObservationGeneration,
    pub(super) old: PlanId,
    pub(super) new: PlanId,
    pub(super) rows: Vec<HandoffRow>,
}

fn metadata<T>(
    length: usize,
    overhead: usize,
    allowance: PreparedBytes,
) -> Result<Vec<T>, ObservationHandoffError> {
    let bytes = |capacity: usize| {
        capacity
            .checked_mul(size_of::<T>())
            .and_then(|bytes| bytes.checked_add(overhead))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .filter(|bytes| *bytes <= allowance.get())
            .ok_or(ObservationHandoffError::Bytes)
    };
    bytes(length)?;
    let mut rows = Vec::new();
    rows.try_reserve_exact(length)
        .map_err(|_| ObservationHandoffError::Allocation)?;
    bytes(rows.capacity())?;
    Ok(rows)
}

fn channels(plan: &CompiledPlan, slot: TapSlot) -> Option<usize> {
    if slot.plan() != plan.id() {
        return None;
    }
    let region = plan.region(plan.taps().get(slot.index())?.region)?;
    let quantum = crate::time::QUANTUM_FRAMES as usize;
    let count = region.length() / quantum;
    (count != 0 && region.length().is_multiple_of(quantum)).then_some(count)
}

fn prepare_rows(
    store: StoreId,
    generation: ObservationGeneration,
    plan: PlanId,
    source: impl ExactSizeIterator<Item = ObservationRow>,
    old: &CompiledPlan,
    new: &CompiledPlan,
    allowance: PreparedBytes,
) -> Result<ObservationHandoff, ObservationHandoffError> {
    if plan != old.id() {
        return Err(ObservationHandoffError::Origin);
    }
    if old.sample_rate() != new.sample_rate() {
        return Err(ObservationHandoffError::Geometry);
    }
    let successor = generation
        .next()
        .ok_or(ObservationHandoffError::Exhausted)?;
    let mut rows: Vec<HandoffRow> =
        metadata(source.len(), size_of::<ObservationHandoff>(), allowance)?;
    for row in source {
        if row.channels == 0 || row.capacity_frames == 0 {
            return Err(ObservationHandoffError::Geometry);
        }
        let destination = if let Some(slot) = row.tap {
            if channels(old, slot) != Some(row.channels) {
                return Err(ObservationHandoffError::Geometry);
            }
            let address = old
                .tap_addresses()
                .iter()
                .find(|address| address.slot == slot)
                .ok_or(ObservationHandoffError::Geometry)?;
            let destination = new.resolve_tap(address.node, address.port);
            if destination.is_some_and(|tap| channels(new, tap) != Some(row.channels)) {
                return Err(ObservationHandoffError::Geometry);
            }
            if let Some(tap) = destination
                && (old.taps().get(slot.index()).map(|target| target.data)
                    != new.taps().get(tap.index()).map(|target| target.data)
                    || rows
                        .iter()
                        .any(|prepared| prepared.destination == Some(tap)))
            {
                return Err(ObservationHandoffError::Geometry);
            }
            destination
        } else {
            None
        };
        rows.push(HandoffRow {
            source: row,
            destination,
        });
    }
    Ok(ObservationHandoff {
        store,
        generation,
        successor,
        old: old.id(),
        new: new.id(),
        rows,
    })
}

impl ObservationSubscriptions {
    /// Snapshot before moving the store to audio, off-thread and within its own budget.
    pub fn configuration(
        &self,
        allowance: PreparedBytes,
    ) -> Result<ObservationConfiguration, ObservationHandoffError> {
        let mut rows = metadata(
            self.subscriptions.len(),
            size_of::<ObservationConfiguration>(),
            allowance,
        )?;
        rows.extend(
            self.subscriptions
                .iter()
                .map(|subscription| ObservationRow {
                    tap: subscription.tap,
                    channels: subscription.channels,
                    capacity_frames: subscription.capacity_frames,
                }),
        );
        Ok(ObservationConfiguration {
            store: self.id,
            generation: self.generation,
            plan: self.plan,
            rows,
        })
    }

    /// Resolve surviving subscriptions by authored node/port, never by tap index.
    /// The host must establish graph identity continuity, including application
    /// lineage across deletion/recreation. Missing taps end; changed geometry
    /// refuses observation only. Same-plan resets still advance the generation.
    pub fn prepare_handoff(
        &self,
        old: &CompiledPlan,
        new: &CompiledPlan,
        allowance: PreparedBytes,
    ) -> Result<ObservationHandoff, ObservationHandoffError> {
        prepare_rows(
            self.id,
            self.generation,
            self.plan,
            self.subscriptions
                .iter()
                .map(|subscription| ObservationRow {
                    tap: subscription.tap,
                    channels: subscription.channels,
                    capacity_frames: subscription.capacity_frames,
                }),
            old,
            new,
            allowance,
        )
    }
}

impl ObservationConfiguration {
    /// The host must establish graph identity continuity before preparing this map.
    /// A stale mirror is harmless: installation checks the complete actual configuration.
    pub fn prepare_handoff(
        &self,
        old: &CompiledPlan,
        new: &CompiledPlan,
        allowance: PreparedBytes,
    ) -> Result<ObservationHandoff, ObservationHandoffError> {
        prepare_rows(
            self.store,
            self.generation,
            self.plan,
            self.rows.iter().copied(),
            old,
            new,
            allowance,
        )
    }
}

impl ObservationHandoff {
    pub const fn old_plan(&self) -> PlanId {
        self.old
    }
    /// Predict the next cold mirror without reading the audio-owned store.
    /// A refused earlier transition leaves this mirror stale; it cannot authorize a replay.
    pub fn successor_configuration(
        &self,
        allowance: PreparedBytes,
    ) -> Result<ObservationConfiguration, ObservationHandoffError> {
        let mut rows = metadata(
            self.rows.len(),
            size_of::<ObservationConfiguration>(),
            allowance,
        )?;
        rows.extend(self.rows.iter().map(|row| ObservationRow {
            tap: row.destination,
            ..row.source
        }));
        Ok(ObservationConfiguration {
            store: self.store,
            generation: self.successor,
            plan: self.new,
            rows,
        })
    }
}
