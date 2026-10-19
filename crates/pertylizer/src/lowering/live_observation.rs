//! Application ownership and acknowledged cold mirrors for threaded observers.
use std::collections::VecDeque;

use super::*;
use crate::lowering::observation::{
    ObservationAudio, ObservationBuffer, ObservationControl, ObservationReply, ObservationRequest,
    ObservationRequestId,
};
use synth_engine_v2::{
    host::session::transfer::replacement::{ObservationAdmission, ObservationReplacementOutcome},
    observe::{ObservationConfiguration, ObservationSubscriptions, StoreId, SubscriptionId},
    plan::{CompiledPlan, TapSlot},
};

const OWNER_BYTES: PreparedBytes = PreparedBytes::measured(4 * 1024 * 1024);
const REPLIES: usize = 2;

pub(super) struct SongObservationControl {
    lane: ObservationControl,
    mirror: Option<ObservationConfiguration>,
    store: Option<StoreId>,
    pending: Option<(
        ObservationRequestId,
        Option<ObservationConfiguration>,
        Option<StoreId>,
    )>,
    replies: VecDeque<ObservationReply>,
    edit_mirror: Option<ObservationConfiguration>,
}

impl SongObservationControl {
    pub(super) fn prepare(
        profile: &HostProfile,
    ) -> Result<(Self, ObservationAudio), SongPlaybackError> {
        let (lane, audio) =
            ObservationControl::prepare(profile.capabilities().maximum_block_size(), OWNER_BYTES)
                .map_err(|error| SongPlaybackError::Preparation(error.to_string()))?;
        Ok((
            Self {
                lane,
                mirror: None,
                store: None,
                pending: None,
                replies: VecDeque::with_capacity(REPLIES),
                edit_mirror: None,
            },
            audio,
        ))
    }

    pub(super) fn poll(&mut self) {
        while self.replies.len() < REPLIES {
            let Some(reply) = self.lane.collect() else {
                break;
            };
            if self
                .pending
                .as_ref()
                .is_some_and(|(id, _, _)| *id == reply.id)
                && let Some((_, mirror, store)) = self.pending.take()
                && reply.result.is_ok()
            {
                self.mirror = mirror;
                self.store = store;
            }
            self.replies.push_back(reply);
        }
    }

    pub(super) fn changing(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn refuse_edit(
        &self,
    ) -> Option<(ObservationAdmission, Option<ObservationConfiguration>)> {
        self.store.map(|_| {
            (
                ObservationAdmission::Refused(
                    synth_engine_v2::observe::ObservationHandoffError::NotPrepared,
                ),
                None,
            )
        })
    }

    pub(super) fn prepare_edit(
        &self,
        old: &CompiledPlan,
        new: &CompiledPlan,
    ) -> Option<(ObservationAdmission, Option<ObservationConfiguration>)> {
        let mirror = self.mirror.as_ref()?;
        let result = mirror
            .prepare_handoff(old, new, OWNER_BYTES)
            .and_then(|map| {
                let next = map.successor_configuration(OWNER_BYTES)?;
                Ok((map, next))
            });
        Some(match result {
            Ok((map, next)) => (ObservationAdmission::Prepared(map), Some(next)),
            Err(error) => (ObservationAdmission::Refused(error), None),
        })
    }

    pub(super) fn attach(
        packet: &mut PreparedPlanReplacement,
        prepared: Option<(ObservationAdmission, Option<ObservationConfiguration>)>,
    ) -> Result<Option<ObservationConfiguration>, SongPlaybackError> {
        let Some((admission, mirror)) = prepared else {
            return Ok(None);
        };
        packet
            .attach_observation(admission)
            .map_err(|(_, error)| SongPlaybackError::Preparation(error.to_string()))?;
        Ok(mirror)
    }

    pub(super) fn published(&mut self, mirror: Option<ObservationConfiguration>) {
        self.edit_mirror = mirror;
    }

    pub(super) fn edited(&mut self, outcome: ObservationReplacementOutcome) {
        match outcome {
            ObservationReplacementOutcome::HandedOff { .. } => {
                self.mirror = self.edit_mirror.take()
            }
            ObservationReplacementOutcome::Ended(_)
            | ObservationReplacementOutcome::Detached(_)
            | ObservationReplacementOutcome::Interrupted { .. } => {
                self.mirror = None;
                self.edit_mirror = None;
            }
            ObservationReplacementOutcome::NotAttached
            | ObservationReplacementOutcome::NotAttempted => self.edit_mirror = None,
        }
    }
}

impl SongPlayback {
    /// Queue a cold-prepared complete subscription set. Old handles end on installation.
    /// The returned request still needs its owning acknowledgement from `take_observation_reply`.
    pub fn subscribe_observations(
        &mut self,
        profile: &HostProfile,
        taps: &[TapSlot],
    ) -> Result<(ObservationRequestId, Vec<SubscriptionId>), SongPlaybackError> {
        self.collect();
        if self.is_editing() || self.observers.changing() || self.observers.replies.len() == REPLIES
        {
            return Err(SongPlaybackError::Command(
                "observation ownership is busy".to_owned(),
            ));
        }
        let plan = self.control.plan();
        let mut store = ObservationSubscriptions::prepare(profile, plan)
            .map_err(|error| SongPlaybackError::Preparation(error.to_string()))?;
        let mut handles = Vec::with_capacity(taps.len());
        for tap in taps {
            handles.push(
                store
                    .subscribe(plan, *tap)
                    .map_err(|error| SongPlaybackError::Preparation(error.to_string()))?,
            );
        }
        let mirror = store
            .configuration(OWNER_BYTES)
            .map_err(|error| SongPlaybackError::Preparation(error.to_string()))?;
        let store_id = store.id();
        let id = self
            .observers
            .lane
            .submit(ObservationRequest::Install {
                expected: self.observers.store,
                incoming: Some(Box::new(store)),
            })
            .map_err(|(_, error)| SongPlaybackError::Command(error.to_string()))?;
        self.observers.pending = Some((id, Some(mirror), Some(store_id)));
        Ok((id, handles))
    }

    pub fn remove_observations(&mut self) -> Result<ObservationRequestId, SongPlaybackError> {
        self.collect();
        if self.is_editing() || self.observers.changing() || self.observers.replies.len() == REPLIES
        {
            return Err(SongPlaybackError::Command(
                "observation ownership is busy".to_owned(),
            ));
        }
        let expected = self.observers.store.ok_or_else(|| {
            SongPlaybackError::Command("no observation store is installed".to_owned())
        })?;
        let id = self
            .observers
            .lane
            .submit(ObservationRequest::Remove { expected })
            .map_err(|(_, error)| SongPlaybackError::Command(error.to_string()))?;
        self.observers.pending = Some((id, None, None));
        Ok(id)
    }

    pub fn read_observations(
        &mut self,
        subscription: SubscriptionId,
        buffer: ObservationBuffer,
    ) -> Result<ObservationRequestId, (ObservationRequest, SongPlaybackError)> {
        let request = ObservationRequest::Read {
            subscription,
            buffer,
        };
        self.collect();
        if self.observers.changing() || self.observers.replies.len() == REPLIES {
            return Err((
                request,
                SongPlaybackError::Command("observation ownership is busy".to_owned()),
            ));
        }
        self.observers
            .lane
            .submit(request)
            .map_err(|(request, error)| (request, SongPlaybackError::Command(error.to_string())))
    }

    pub fn take_observation_reply(&mut self) -> Option<ObservationReply> {
        self.observers.poll();
        self.observers.replies.pop_front()
    }
}

impl SongPlayback {
    /// Consume a matching pair after the backend has relinquished audio ownership.
    /// Outstanding observation owners return as completed or explicitly cancelled.
    pub fn close_observations_after_quiescence(
        mut self,
        mut audio: SongAudio,
    ) -> Result<
        crate::lowering::observation::ObservationRetired,
        Box<(
            Self,
            SongAudio,
            crate::lowering::observation::ObservationLaneError,
        )>,
    > {
        match self.observers.lane.close(audio.observers) {
            Ok(mut retired) => {
                retired.completed.extend(self.observers.replies.drain(..));
                Ok(retired)
            }
            Err(owners) => {
                let (lane, observer_audio, error) = *owners;
                self.observers.lane = lane;
                audio.observers = observer_audio;
                Err(Box::new((self, audio, error)))
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/threaded_observation.rs"]
mod tests;
