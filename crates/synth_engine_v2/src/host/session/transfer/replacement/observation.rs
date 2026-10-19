//! Independent observation preparation, carried by the sound's owning envelope.
use super::{PlanReplacementError, PlanReplacementKind, PreparedPlanReplacement};
use crate::observe::{ObservationGeneration, ObservationHandoff, ObservationHandoffError};

/// A failed observation preparation is carried explicitly and never rejects sound.
#[derive(Debug)]
#[must_use]
pub enum ObservationAdmission {
    Prepared(ObservationHandoff),
    Refused(ObservationHandoffError),
}

impl From<Result<ObservationHandoff, ObservationHandoffError>> for ObservationAdmission {
    fn from(result: Result<ObservationHandoff, ObservationHandoffError>) -> Self {
        match result {
            Ok(prepared) => Self::Prepared(prepared),
            Err(error) => Self::Refused(error),
        }
    }
}

/// Observation follows the actual sound outcome; it never selects sound admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ObservationReplacementOutcome {
    NotAttached,
    NotAttempted,
    HandedOff {
        generation: ObservationGeneration,
        kind: PlanReplacementKind,
    },
    Interrupted {
        generation: ObservationGeneration,
        kind: PlanReplacementKind,
    },
    Ended(ObservationHandoffError),
    /// The host lost the paired store or bypassed observed rendering. Its actual
    /// state is unknown to this call; this does not claim that store was ended.
    Detached(ObservationHandoffError),
}

impl PreparedPlanReplacement {
    /// Attach off-thread, before publication. The host has established authored
    /// graph identity continuity. Use the observed installation entry for this kind.
    pub fn attach_observation(
        &mut self,
        admission: ObservationAdmission,
    ) -> Result<(), (ObservationAdmission, PlanReplacementError)> {
        if self.observation.is_some() || self.outcome.is_some() || self.is_rendering_transition() {
            return Err((admission, PlanReplacementError::State));
        }
        if let ObservationAdmission::Prepared(prepared) = &admission
            && (prepared.old_plan() != self.old_plan || prepared.new_plan() != self.plan)
        {
            return Err((
                admission,
                PlanReplacementError::Observation(ObservationHandoffError::Origin),
            ));
        }
        self.observation = Some(admission);
        self.observation_outcome = ObservationReplacementOutcome::NotAttempted;
        Ok(())
    }

    pub const fn observation_outcome(&self) -> ObservationReplacementOutcome {
        self.observation_outcome
    }
}
