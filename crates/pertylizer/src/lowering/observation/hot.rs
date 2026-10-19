//! Callback-only observation service; included in the render purity region.
use super::{
    ObservationAudio, ObservationLaneError, ObservationReply, ObservationRequest,
    ObservationService,
};
use ringbuf::traits::{Consumer, Observer, Producer};
use synth_engine_v2::{observe::ObservationSubscriptions, plan::PlanId};

impl ObservationAudio {
    /// The host hands this borrow to its existing observed render/transition entry.
    pub(crate) fn store_mut(&mut self) -> Option<&mut ObservationSubscriptions> {
        self.active.as_deref_mut()
    }

    /// One bounded service at a callback boundary, never a wait or unbounded drain.
    pub fn service(&mut self, installed: PlanId) -> ObservationService {
        if let Some(reply) = self.retained.take() {
            if let Err(reply) = self.replies.try_push(reply) {
                self.retained = Some(reply);
                return ObservationService::Backpressured;
            }
            return ObservationService::Returned;
        }
        if self.replies.is_full() {
            return ObservationService::Backpressured;
        }
        let Some(envelope) = self.requests.try_pop() else {
            return ObservationService::Idle;
        };
        let mut reply = ObservationReply {
            id: envelope.id,
            request: envelope.request,
            result: Ok(None),
            retired: None,
        };
        reply.result = match &mut reply.request {
            ObservationRequest::Install { expected, incoming } => {
                if *expected != self.active.as_ref().map(|store| store.id()) {
                    Err(ObservationLaneError::ForeignStore)
                } else if incoming
                    .as_ref()
                    .is_none_or(|store| store.plan() != installed)
                {
                    Err(if incoming.is_none() {
                        ObservationLaneError::MissingStore
                    } else {
                        ObservationLaneError::ForeignPlan
                    })
                } else {
                    if let Some(store) = self.active.as_mut() {
                        store.end();
                    }
                    reply.retired = self.active.take();
                    self.active = incoming.take();
                    Ok(None)
                }
            }
            ObservationRequest::Remove { expected } => {
                if self
                    .active
                    .as_ref()
                    .is_none_or(|store| store.id() != *expected)
                {
                    Err(ObservationLaneError::ForeignStore)
                } else {
                    if let Some(store) = self.active.as_mut() {
                        store.end();
                    }
                    reply.retired = self.active.take();
                    Ok(None)
                }
            }
            ObservationRequest::Read {
                subscription,
                buffer,
            } => match self.active.as_mut() {
                Some(store) if store.id() == subscription.store() => {
                    if store.channels(*subscription) != Some(buffer.layout.channels()) {
                        Err(ObservationLaneError::Layout)
                    } else {
                        store
                            .read_bounded(
                                *subscription,
                                &mut buffer.samples,
                                self.maximum_read_frames,
                            )
                            .map(Some)
                            .ok_or(ObservationLaneError::ForeignStore)
                    }
                }
                _ => Err(ObservationLaneError::ForeignStore),
            },
        };
        match self.replies.try_push(reply) {
            Ok(()) => ObservationService::Returned,
            Err(reply) => {
                self.retained = Some(reply);
                ObservationService::Backpressured
            }
        }
    }
}
