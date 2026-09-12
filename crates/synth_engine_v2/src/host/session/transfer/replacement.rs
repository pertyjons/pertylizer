//! Stopped compiled-plan readmission with owning, bounded retirement.
//!
//! Five credits bound all prepared candidates, including caller-held unpublished
//! values. Dropping an unresolved value never returns its credit. A host supplies
//! the latest-wins mailbox and an owning return lane; no queue type enters core.

mod hot;
#[cfg(test)]
mod tests;

use super::{SessionAudio, SessionControl, SessionSnapshot, SessionTransferLimits};
use crate::host::{ConnectionGeneration, HostError};
use crate::identity::TableId;
use crate::plan::{CompiledPlan, PlanId};
use crate::profile::HostProfile;
use crate::schedule::AdmittedCompiledStream;
use crate::time::{PlanPosition, StreamAnchor, StreamEpoch};
use thiserror::Error;

/// Maximum unresolved candidates, across every host and callback location.
pub const REPLACEMENT_CREDITS: usize = 5;

/// A publication occurrence; cloning a compiled plan does not clone this identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PlanPublicationId {
    generation: ConnectionGeneration,
    serial: u64,
}

/// Installation refuses atomically and returns its prepared resources for collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PlanReplacementRefusal {
    #[error("replacement belongs to another connection or device epoch")]
    Origin,
    #[error("replacement is older than an already attempted publication")]
    Superseded,
    #[error("ordinary commands changed since replacement preparation")]
    CommandsChanged,
    #[error("replacement requires the unchanged stopped position")]
    PositionChanged,
    #[error("outgoing stream still owns note obligations")]
    NoteObligations,
    #[error("outgoing renderer still has output carry")]
    Carry,
    #[error("audio session is closed or faulted")]
    Unavailable,
    #[error("replacement is not a fresh prepared pair")]
    NotFresh,
}

/// Owning packet misuse and off-thread preparation errors.
#[derive(Debug, Error)]
pub enum PlanReplacementError {
    #[error("replacement requires acknowledged stopped transport with no command credits")]
    Transport,
    #[error("outgoing control still owns note obligations")]
    NoteObligations,
    #[error("all five replacement credits are occupied")]
    Full,
    #[error("plan publication identities are exhausted")]
    IdentityExhausted,
    #[error("replacement changes device geometry")]
    Geometry,
    #[error("replacement preparation failed: {0}")]
    Prepare(#[from] HostError),
    #[error("replacement has no matching connection credit")]
    Credit,
    #[error("replacement was already attempted or has not completed")]
    State,
    #[error("collect the outgoing plan's earlier retirement first")]
    OutgoingTable,
}

/// A scalar receipt survives off-thread destruction of all retired resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PlanReplacementReceipt {
    pub id: PlanPublicationId,
    pub plan: PlanId,
    pub outcome: PlanReplacementOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanReplacementOutcome {
    Installed(SessionSnapshot),
    Refused(PlanReplacementRefusal),
    Cancelled,
}

/// Prepared off-thread. On installation `audio` receives the outgoing audio owner;
/// its accompanying old controller stays on the control thread until collection.
/// The host must retain this box on every failed send and through backend join.
#[must_use]
pub struct PreparedPlanReplacement {
    id: PlanPublicationId,
    epoch: StreamEpoch,
    plan: PlanId,
    prepared_from_table: TableId,
    expected_position: PlanPosition,
    command_watermark: u64,
    control: SessionControl,
    audio: SessionAudio,
    outcome: Option<PlanReplacementOutcome>,
    outgoing_table: Option<TableId>,
}

impl std::fmt::Debug for PreparedPlanReplacement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedPlanReplacement")
            .field("id", &self.id)
            .field("plan", &self.plan)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

impl PreparedPlanReplacement {
    pub const fn id(&self) -> PlanPublicationId {
        self.id
    }
    pub const fn plan_id(&self) -> PlanId {
        self.plan
    }
    pub const fn outcome(&self) -> Option<PlanReplacementOutcome> {
        self.outcome
    }
}

impl SessionControl {
    pub fn has_replacements(&self) -> bool {
        self.replacement_credits.iter().any(Option::is_some)
    }

    /// Compilation precedes this call. Failure never invalidates the previous plan
    /// or a host's previously published valid candidate. At most one synchronous
    /// preparation may borrow this control; the selected credit bounds that transient.
    pub fn prepare_replacement(
        &mut self,
        plan: CompiledPlan,
        stream: AdmittedCompiledStream,
        profile: HostProfile,
        limits: SessionTransferLimits,
    ) -> Result<Box<PreparedPlanReplacement>, PlanReplacementError> {
        use crate::host::session::PlaybackState;
        let PlaybackState::Stopped(position) = self.acknowledged.playback else {
            return Err(PlanReplacementError::Transport);
        };
        if self.closed || self.held != 0 {
            return Err(PlanReplacementError::Transport);
        }
        if self.control.has_replacement_obligations() {
            return Err(PlanReplacementError::NoteObligations);
        }
        let index = self
            .replacement_credits
            .iter()
            .position(Option::is_none)
            .ok_or(PlanReplacementError::Full)?;
        let serial = self
            .publication_serial
            .checked_add(1)
            .ok_or(PlanReplacementError::IdentityExhausted)?;
        let previous = self.control.plan();
        if plan.sample_rate() != previous.sample_rate()
            || plan.channel_layout() != previous.channel_layout()
            || plan.maximum_block_size() != previous.maximum_block_size()
        {
            return Err(PlanReplacementError::Geometry);
        }
        let (control, audio) = Self::prepare_pair(plan, stream, profile, limits, Some(self))?;
        let id = PlanPublicationId {
            generation: self.origin.generation,
            serial,
        };
        let candidate = Box::new(PreparedPlanReplacement {
            id,
            epoch: self.origin.epoch,
            plan: control.origin.plan,
            prepared_from_table: self.origin.table,
            expected_position: position,
            command_watermark: self.acknowledged_serial,
            control,
            audio,
            outcome: None,
            outgoing_table: None,
        });
        self.replacement_credits[index] = Some(id);
        self.publication_serial = serial;
        Ok(candidate)
    }

    fn replacement_credit(
        &self,
        candidate: &PreparedPlanReplacement,
    ) -> Result<usize, PlanReplacementError> {
        if candidate.id.generation != self.origin.generation || candidate.epoch != self.origin.epoch
        {
            return Err(PlanReplacementError::Credit);
        }
        self.replacement_credits
            .iter()
            .position(|entry| *entry == Some(candidate.id))
            .ok_or(PlanReplacementError::Credit)
    }

    /// Cancel a value that never reached installation, including a superseded
    /// mailbox cell or a value recovered after backend join. Off-thread only.
    pub fn cancel_replacement(
        &mut self,
        candidate: Box<PreparedPlanReplacement>,
    ) -> Result<PlanReplacementReceipt, (Box<PreparedPlanReplacement>, PlanReplacementError)> {
        let index = match self.replacement_credit(&candidate) {
            Ok(index) => index,
            Err(error) => return Err((candidate, error)),
        };
        if candidate.outcome.is_some() {
            return Err((candidate, PlanReplacementError::State));
        }
        self.replacement_credits[index] = None;
        Ok(PlanReplacementReceipt {
            id: candidate.id,
            plan: candidate.plan,
            outcome: PlanReplacementOutcome::Cancelled,
        })
    }

    /// Collect in installation order; a mismatched outgoing table returns the box.
    /// The whole actual snapshot and the ordinary issuer/credit ledger survive the
    /// control exchange. Old audio, control and packet are destroyed only here.
    pub fn collect_replacement(
        &mut self,
        mut candidate: Box<PreparedPlanReplacement>,
    ) -> Result<PlanReplacementReceipt, (Box<PreparedPlanReplacement>, PlanReplacementError)> {
        let index = match self.replacement_credit(&candidate) {
            Ok(index) => index,
            Err(error) => return Err((candidate, error)),
        };
        let Some(outcome) = candidate.outcome else {
            return Err((candidate, PlanReplacementError::State));
        };
        if let PlanReplacementOutcome::Installed(snapshot) = outcome {
            if candidate.outgoing_table != Some(self.origin.table) {
                return Err((candidate, PlanReplacementError::OutgoingTable));
            }
            let crate::host::session::PlaybackState::Stopped(position) = snapshot.playback else {
                return Err((candidate, PlanReplacementError::State));
            };
            candidate.control.acknowledged = snapshot;
            candidate.control.acknowledged_serial = self.acknowledged_serial;
            candidate.control.serial = self.serial;
            candidate.control.last_offer = self.last_offer;
            candidate.control.closed = self.closed;
            candidate.control.publication_serial = self.publication_serial;
            candidate.control.replacement_credits = self.replacement_credits;
            candidate
                .control
                .control
                .synchronize_replacement_anchor(StreamAnchor::new(snapshot.clock, position));
            std::mem::swap(self, &mut candidate.control);
        }
        self.replacement_credits[index] = None;
        Ok(PlanReplacementReceipt {
            id: candidate.id,
            plan: candidate.plan,
            outcome,
        })
    }
}
