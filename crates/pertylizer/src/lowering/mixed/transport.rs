//! One owning operation credit, returned in its original allocation.
use super::*;
use synth_engine_v2::stream::{
    MixedActivationOffer, MixedActivationRefusal, MixedActivationRetired, MixedBridgeStopRefusal,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(super) struct RequestId(std::num::NonZeroU64);
impl RequestId {
    pub(super) fn next(previous: Option<Self>) -> Option<Self> {
        previous
            .map_or(Some(1), |id| id.0.get().checked_add(1))
            .and_then(std::num::NonZeroU64::new)
            .map(Self)
    }
}

pub(super) enum Operation {
    Reset {
        owner: Option<Owner>,
        retired: Box<Option<RetiredOwner>>,
        refused: Option<synth_engine_v2::stream::MixedBridgeResetRefusal>,
        at: SampleTime,
        running: bool,
        offered: bool,
        install: super::super::app::InstallSequence,
        anchor: StreamAnchor,
        observed_stopped: Option<PlanPosition>,
    },
    Continuation {
        packet: Option<Box<synth_engine_v2::stream::MixedContinuationPacket>>,
        offered: bool,
        refusal: Option<synth_engine_v2::stream::MixedContinuationRefusal>,
        install: super::super::app::InstallSequence,
    },
    Activation {
        intent: TransportIntent,
        offer: Option<MixedActivationOffer>,
        retired: Option<MixedActivationRetired>,
        refused: Option<MixedActivationRefusal>,
    },
    Stop {
        at: SampleTime,
        accepted: bool,
        refused: Option<MixedBridgeStopRefusal>,
        position: Option<PlanPosition>,
    },
}

pub(super) struct Request {
    pub(super) id: RequestId,
    pub(super) operation: Operation,
    pub(super) complete: bool,
    /// Only the entire completing callback can establish this observation.
    pub(super) rendered: Option<bool>,
}
impl Request {
    pub(super) fn before_render(&mut self, audio: &mut Audio) {
        match &mut self.operation {
            Operation::Reset {
                owner,
                at,
                running,
                refused,
                offered,
                ..
            } => {
                if !*offered && let Some(candidate) = owner.take() {
                    let result = if *running {
                        audio.offer_running_reset(candidate, *at)
                    } else {
                        audio.offer_reset(candidate, *at)
                    };
                    match result {
                        Ok(()) => *offered = true,
                        Err((reason, candidate)) => {
                            *owner = Some(candidate);
                            *refused = Some(reason);
                            self.complete = true;
                        }
                    }
                }
            }
            Operation::Continuation {
                packet,
                offered,
                refusal,
                ..
            } => {
                if !*offered && let Some(candidate) = packet.take() {
                    match audio.offer_continuation(candidate) {
                        Ok(()) => *offered = true,
                        Err((candidate, reason)) => {
                            *packet = Some(candidate);
                            *refusal = Some(reason);
                            self.complete = true;
                        }
                    }
                }
            }
            Operation::Activation { offer, refused, .. } => {
                if let Some(candidate) = offer.take()
                    && let Err((reason, candidate)) = audio.offer_activation(candidate)
                {
                    *offer = Some(candidate);
                    *refused = Some(reason);
                    self.complete = true;
                }
            }
            Operation::Stop {
                at,
                accepted,
                refused,
                ..
            } if !*accepted => match audio.schedule_stop(*at) {
                Ok(_) => *accepted = true,
                Err(reason) => {
                    *refused = Some(reason);
                    self.complete = true;
                }
            },
            Operation::Stop { .. } => {}
        }
    }
    pub(super) fn after_render(&mut self, audio: &mut Audio, rendered: bool) {
        if self.rendered.is_some() {
            return;
        }
        if self.complete {
            self.rendered = Some(rendered);
            return;
        }
        match &mut self.operation {
            Operation::Reset {
                owner,
                retired,
                refused,
                offered,
                observed_stopped,
                ..
            } if *offered => {
                if let Some(returned) = audio.take_retired() {
                    **retired = Some(returned);
                    *observed_stopped = audio.stopped_position();
                    self.complete = true;
                } else if let Some(returned) = audio.take_refused() {
                    *refused = Some(returned.reason());
                    *owner = Some(returned.into_owner());
                    self.complete = true;
                } else if !rendered {
                    *owner = audio.withdraw_pending();
                }
            }
            Operation::Continuation {
                packet, offered, ..
            } if *offered => {
                if let Some(returned) = audio.take_retired_continuation() {
                    *packet = Some(returned);
                    self.complete = true;
                }
            }
            Operation::Activation {
                retired, refused, ..
            } if refused.is_none() => {
                if let Some(receipt) = audio.take_retired_activation() {
                    *retired = Some(receipt);
                    self.complete = true;
                }
            }
            Operation::Stop {
                at,
                accepted,
                position,
                ..
            } if *accepted && audio.clock() > *at => {
                *position = audio.stopped_position();
                self.complete = true;
            }
            _ => {}
        }
        if !rendered {
            self.complete = true;
        }
        if self.complete {
            self.rendered = Some(rendered);
        }
    }
}
