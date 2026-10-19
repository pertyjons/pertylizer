//! Cold custody of the exact stamped list that a Play actually adopted.
use super::{Origin, PreparedSessionCommand};
use crate::{
    host::session::SessionCommand,
    identity::TableId,
    plan::{ContinuationError, PlanId},
    quantities::{EventCount, PreparedBytes, RecordCount},
    render::TimedEvent,
    time::{StreamAnchor, StreamEpoch},
    transport::{ActivationSequence, TransportActivation},
};

/// Separate cold-copy limits. These select no producer share or callback capacity.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct ContinuationLimits {
    /// Bound structural preflight, copied rows and voice/history-owner exchanges.
    pub maximum_transition_records: RecordCount,
    pub maximum_stamped_events: EventCount,
    pub witness_bytes: PreparedBytes,
    pub transition_bytes: PreparedBytes,
}

#[derive(Debug)]
pub(crate) struct AdoptedTimelineWitness {
    pub(crate) plan: PlanId,
    pub(crate) epoch: StreamEpoch,
    pub(crate) table: TableId,
    pub(crate) sequence: ActivationSequence,
    pub(crate) anchor: StreamAnchor,
    pub(crate) events: Vec<TimedEvent>,
    pub(crate) bytes: PreparedBytes,
}

impl AdoptedTimelineWitness {
    fn capture(
        origin: Origin,
        activation: &TransportActivation,
        limits: ContinuationLimits,
    ) -> Result<Self, ContinuationError> {
        if activation.epoch != origin.epoch
            || activation.minter.id() != origin.table
            || activation.loop_interval.is_some()
        {
            return Err(ContinuationError::Binding);
        }
        let events = copy_events(
            &activation.events,
            limits.maximum_stamped_events,
            limits.witness_bytes,
            size_of::<Self>(),
        )?;
        let bytes = charged_bytes(events.capacity(), size_of::<Self>())?;
        Ok(Self {
            plan: origin.plan,
            epoch: origin.epoch,
            table: origin.table,
            sequence: activation.sequence,
            anchor: StreamAnchor::new(activation.requested, activation.position),
            events,
            bytes,
        })
    }

    pub(in crate::host::session::transfer) fn matches(
        &self,
        origin: Origin,
        sequence: ActivationSequence,
        anchor: StreamAnchor,
    ) -> bool {
        self.plan == origin.plan
            && self.epoch == origin.epoch
            && self.table == origin.table
            && self.sequence == sequence
            && self.anchor == anchor
    }
}

impl PreparedSessionCommand {
    /// Attach a bounded copy BEFORE publication. Failure leaves this owning command
    /// intact for ordinary submission or cancellation, with no activation credit lost.
    pub fn prepare_continuation_witness(
        &mut self,
        limits: ContinuationLimits,
    ) -> Result<(), ContinuationError> {
        if self.entry.boundary.command != SessionCommand::Play
            || self.entry.boundary.capture.is_some()
            || self.entry.timeline_witness.is_some()
        {
            return Err(ContinuationError::Unavailable);
        }
        let activation = self
            .entry
            .activation
            .as_ref()
            .ok_or(ContinuationError::Unavailable)?;
        let witness = AdoptedTimelineWitness::capture(self.origin, activation, limits)?;
        self.entry.timeline_witness = Some(Box::new(witness));
        Ok(())
    }
}

pub(crate) fn charged_bytes(
    capacity: usize,
    fixed: usize,
) -> Result<PreparedBytes, ContinuationError> {
    let bytes = capacity
        .checked_mul(size_of::<TimedEvent>())
        .and_then(|bytes| bytes.checked_add(fixed))
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(ContinuationError::Bytes)?;
    Ok(PreparedBytes::measured(bytes))
}

pub(crate) fn copy_events(
    events: &[TimedEvent],
    maximum: EventCount,
    allowance: PreparedBytes,
    fixed: usize,
) -> Result<Vec<TimedEvent>, ContinuationError> {
    let maximum = usize::try_from(maximum.get()).map_err(|_| ContinuationError::EventLimit)?;
    if maximum == 0 || events.len() > maximum {
        return Err(ContinuationError::EventLimit);
    }
    if charged_bytes(events.len(), fixed)? > allowance {
        return Err(ContinuationError::Bytes);
    }
    let mut copied = Vec::new();
    copied
        .try_reserve_exact(events.len())
        .map_err(|_| ContinuationError::Allocation)?;
    if charged_bytes(copied.capacity(), fixed)? > allowance {
        return Err(ContinuationError::Bytes);
    }
    copied.extend_from_slice(events);
    Ok(copied)
}
