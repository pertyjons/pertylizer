//! Bounded owning observation requests between control and audio threads.
//!
//! Construction, subscriptions, buffer allocation and joined destruction are cold.
//! Audio services at most one preallocated request between render calls. A stalled
//! consumer retains owners and stops request admission without stopping sound.
use std::sync::Arc;

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use synth_engine_v2::{
    observe::{ObservationRead, ObservationSubscriptions, StoreId, SubscriptionId},
    quantities::{ChannelLayout, PreparedBytes},
    time::FrameCount,
};
use thiserror::Error;

mod hot;

const CREDITS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub struct ObservationRequestId(u64);

/// A preallocated read destination; its complete capacity is admitted cold.
#[derive(Debug)]
#[must_use]
pub struct ObservationBuffer {
    samples: Vec<f32>,
    layout: ChannelLayout,
}

impl ObservationBuffer {
    pub fn prepare(
        layout: ChannelLayout,
        frames: FrameCount,
        allowance: PreparedBytes,
    ) -> Result<Self, ObservationLaneError> {
        let count = usize::try_from(frames.as_u64())
            .ok()
            .and_then(|frames| frames.checked_mul(layout.channels()))
            .ok_or(ObservationLaneError::Bytes)?;
        let charged = |capacity: usize| {
            capacity
                .checked_mul(size_of::<f32>())
                .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
                .and_then(|bytes| u64::try_from(bytes).ok())
        };
        if count == 0 || charged(count).is_none_or(|bytes| bytes > allowance.get()) {
            return Err(ObservationLaneError::Bytes);
        }
        let mut samples = Vec::new();
        samples
            .try_reserve_exact(count)
            .map_err(|_| ObservationLaneError::Allocation)?;
        if charged(samples.capacity()).is_none_or(|bytes| bytes > allowance.get()) {
            return Err(ObservationLaneError::Bytes);
        }
        samples.resize(count, 0.0);
        Ok(Self { samples, layout })
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
}

#[derive(Debug)]
#[must_use]
pub enum ObservationRequest {
    /// The old store must match, including `None` for the initially empty lane.
    Install {
        expected: Option<StoreId>,
        incoming: Option<Box<ObservationSubscriptions>>,
    },
    Remove {
        expected: StoreId,
    },
    Read {
        subscription: SubscriptionId,
        buffer: ObservationBuffer,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ObservationLaneError {
    #[error("observation request credits are full")]
    Full,
    #[error("observation request identities are exhausted")]
    Exhausted,
    #[error("observation buffer exceeds its admitted byte allowance")]
    Bytes,
    #[error("observation buffer allocation failed")]
    Allocation,
    #[error("observation request belongs to a different store")]
    ForeignStore,
    #[error("observation store belongs to a different installed plan")]
    ForeignPlan,
    #[error("observation read buffer has a different channel layout")]
    Layout,
    #[error("an ended or previously rendered observation store cannot be installed")]
    UsedStore,
    #[error("observation installation has no incoming owner")]
    MissingStore,
    #[error("observation halves belong to different lanes")]
    ForeignLane,
}

#[derive(Debug)]
struct Envelope {
    id: ObservationRequestId,
    request: ObservationRequest,
}

/// All supplied owners return, including a refused installation or read buffer.
#[derive(Debug)]
#[must_use]
pub struct ObservationReply {
    pub id: ObservationRequestId,
    pub request: ObservationRequest,
    pub result: Result<Option<ObservationRead>, ObservationLaneError>,
    /// The ended outgoing store; its queued tail remains readable off-thread.
    pub retired: Option<Box<ObservationSubscriptions>>,
}

#[must_use]
pub struct ObservationControl {
    origin: Arc<()>,
    requests: HeapProd<Envelope>,
    replies: HeapCons<ObservationReply>,
    in_flight: usize,
    next: u64,
    maximum_owner_bytes: PreparedBytes,
}

#[must_use]
pub struct ObservationAudio {
    origin: Arc<()>,
    requests: HeapCons<Envelope>,
    replies: HeapProd<ObservationReply>,
    active: Option<Box<ObservationSubscriptions>>,
    retained: Option<ObservationReply>,
    maximum_read_frames: FrameCount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationService {
    Idle,
    Returned,
    Backpressured,
}

/// Cold-only joined teardown. Every accepted request is either completed or cancelled.
#[must_use]
pub struct ObservationRetired {
    pub active: Option<Box<ObservationSubscriptions>>,
    pub completed: Vec<ObservationReply>,
    pub cancelled: Vec<(ObservationRequestId, ObservationRequest)>,
}

impl ObservationControl {
    pub fn prepare(
        maximum_read_frames: FrameCount,
        maximum_owner_bytes: PreparedBytes,
    ) -> Result<(Self, ObservationAudio), ObservationLaneError> {
        if maximum_read_frames.as_u64() == 0 || maximum_owner_bytes.get() == 0 {
            return Err(ObservationLaneError::Bytes);
        }
        let (requests, incoming) = HeapRb::new(CREDITS).split();
        let (outgoing, replies) = HeapRb::new(CREDITS).split();
        let origin = Arc::new(());
        Ok((
            Self {
                origin: Arc::clone(&origin),
                requests,
                replies,
                in_flight: 0,
                next: 0,
                maximum_owner_bytes,
            },
            ObservationAudio {
                origin,
                requests: incoming,
                replies: outgoing,
                active: None,
                retained: None,
                maximum_read_frames,
            },
        ))
    }

    /// Refusal preserves the complete request and never advances the issuer.
    pub fn submit(
        &mut self,
        request: ObservationRequest,
    ) -> Result<ObservationRequestId, (ObservationRequest, ObservationLaneError)> {
        if let ObservationRequest::Install {
            incoming: Some(store),
            ..
        } = &request
            && !store.is_pristine()
        {
            return Err((request, ObservationLaneError::UsedStore));
        }
        let owner_bytes = match &request {
            ObservationRequest::Install {
                incoming: Some(store),
                ..
            } => store.prepared_bytes().map(|bytes| bytes.get()),
            ObservationRequest::Read { buffer, .. } => buffer
                .samples
                .capacity()
                .checked_mul(size_of::<f32>())
                .and_then(|bytes| bytes.checked_add(size_of::<ObservationBuffer>()))
                .and_then(|bytes| u64::try_from(bytes).ok()),
            _ => Some(0),
        };
        if owner_bytes.is_none_or(|bytes| bytes > self.maximum_owner_bytes.get()) {
            return Err((request, ObservationLaneError::Bytes));
        }
        if self.in_flight == CREDITS || self.requests.is_full() {
            return Err((request, ObservationLaneError::Full));
        }
        let Some(next) = self.next.checked_add(1) else {
            return Err((request, ObservationLaneError::Exhausted));
        };
        let id = ObservationRequestId(next);
        if let Err(envelope) = self.requests.try_push(Envelope { id, request }) {
            return Err((envelope.request, ObservationLaneError::Full));
        }
        self.next = next;
        self.in_flight += 1;
        Ok(id)
    }

    pub fn collect(&mut self) -> Option<ObservationReply> {
        let reply = self.replies.try_pop()?;
        self.in_flight -= 1;
        Some(reply)
    }

    /// Call only after the backend has returned the audio half off its callback.
    pub fn close(
        mut self,
        mut audio: ObservationAudio,
    ) -> Result<ObservationRetired, Box<(Self, ObservationAudio, ObservationLaneError)>> {
        if !Arc::ptr_eq(&self.origin, &audio.origin) {
            return Err(Box::new((self, audio, ObservationLaneError::ForeignLane)));
        }
        let mut completed = Vec::with_capacity(CREDITS);
        while let Some(reply) = self.collect() {
            completed.push(reply);
        }
        if let Some(reply) = audio.retained.take() {
            completed.push(reply);
        }
        let mut cancelled = Vec::with_capacity(CREDITS);
        while let Some(envelope) = audio.requests.try_pop() {
            cancelled.push((envelope.id, envelope.request));
        }
        if let Some(store) = audio.active.as_mut() {
            store.end();
        }
        Ok(ObservationRetired {
            active: audio.active.take(),
            completed,
            cancelled,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane() -> (ObservationControl, ObservationAudio) {
        ObservationControl::prepare(FrameCount::new(64), PreparedBytes::measured(4096)).unwrap()
    }

    #[test]
    fn close_refuses_foreign_halves_without_losing_the_original_pairs() {
        let (first, first_audio) = lane();
        let (second, second_audio) = lane();
        let (first, second_audio, error) = *first.close(second_audio).err().unwrap();
        assert_eq!(error, ObservationLaneError::ForeignLane);
        assert!(first.close(first_audio).is_ok());
        assert!(second.close(second_audio).is_ok());
    }

    #[test]
    fn exhausted_request_issuer_keeps_refused_owner_and_never_wraps() {
        let (mut control, audio) = lane();
        control.next = u64::MAX;
        for _ in 0..2 {
            let request = ObservationRequest::Install {
                expected: None,
                incoming: None,
            };
            let (returned, error) = control.submit(request).unwrap_err();
            assert_eq!(error, ObservationLaneError::Exhausted);
            assert!(matches!(
                returned,
                ObservationRequest::Install { incoming: None, .. }
            ));
            assert_eq!(control.next, u64::MAX);
            assert_eq!(control.in_flight, 0);
        }
        let retired = control
            .close(audio)
            .unwrap_or_else(|_| panic!("matching halves"));
        assert!(retired.completed.is_empty());
        assert!(retired.cancelled.is_empty());
    }

    #[test]
    fn close_returns_queued_commands_with_original_identities() {
        let (mut control, audio) = lane();
        let first = control
            .submit(ObservationRequest::Install {
                expected: None,
                incoming: None,
            })
            .unwrap();
        let second = control
            .submit(ObservationRequest::Install {
                expected: None,
                incoming: None,
            })
            .unwrap();
        assert!(matches!(
            control.submit(ObservationRequest::Install {
                expected: None,
                incoming: None
            }),
            Err((_, ObservationLaneError::Full))
        ));
        let retired = control
            .close(audio)
            .unwrap_or_else(|_| panic!("matching halves"));
        assert_eq!(
            retired
                .cancelled
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            [first, second]
        );
    }
}
