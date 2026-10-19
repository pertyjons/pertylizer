//! The audio-thread half of a subscription: one bounded copy per quantum per subscriber.
//!
//! Inside the render loop's scanned region because it runs after each quantum's schedule
//! walk. It allocates nothing, locks nothing and cannot panic: every index is checked, the
//! ring was sized at subscription, and a subscription whose plan is not the renderer's
//! receives nothing. A copy is the whole of what observation costs the render, and it is
//! the same copy whether or not anyone reads.

use super::{
    ObservationGeneration, ObservationHandoff, ObservationHandoffError, ObservationRead,
    ObservationSubscriptions, Subscription, SubscriptionId,
};
use crate::plan::{BufferRegion, PlanId, TapTarget};
use crate::time::{FrameCount, QUANTUM_FRAMES};

impl ObservationSubscriptions {
    /// End every current handle without freeing its ring. Already queued frames
    /// remain readable under their old plan label; these handles never revive.
    pub fn end(&mut self) {
        for subscription in &mut self.subscriptions {
            subscription.tap = None;
        }
    }

    /// Push the quantum just rendered into every subscription's ring.
    ///
    /// `taps` and `regions` are the renderer's plan's tables and `buffers` its arena; a store
    /// prepared for another plan is skipped whole, because its indexes would name regions of
    /// a plan they were not admitted against. A reader more than a ring behind loses the
    /// oldest frames, counted for its next read (`HOST-INV-019`).
    pub(crate) fn push_quantum(
        &mut self,
        plan: PlanId,
        taps: &[TapTarget],
        regions: &[BufferRegion],
        buffers: &[f32],
    ) {
        if plan != self.plan {
            return;
        }
        for subscription in &mut self.subscriptions {
            let Some(slot) = subscription.tap else {
                continue;
            };
            let Some(tap) = taps.get(slot.index()) else {
                continue;
            };
            let Some(region) = regions.get(tap.region.index()).copied() else {
                continue;
            };
            let Some(source) = buffers.get(region.offset()..region.end()) else {
                continue;
            };
            subscription.append_quantum(source);
        }
    }
}

impl Subscription {
    /// Append one quantum of frames, wrapping the ring, and evict what the reader is too far
    /// behind to ever read.
    fn append_quantum(&mut self, source: &[f32]) {
        let quantum = QUANTUM_FRAMES as usize;
        let channels = self.channels.max(1);
        let capacity = self.capacity_frames.max(1);
        // Frame by frame rather than as one memmove, because a quantum may wrap the ring
        // and a ring shorter than a quantum — a legal profile — wraps more than once. A
        // bounded loop of `channels`-wide copies is the same work either way.
        let start = usize::try_from(self.written % capacity as u64).unwrap_or(0);
        for frame in 0..quantum {
            let slot = (start + frame) % capacity;
            let (Some(from), Some(to)) = (
                source.get(frame * channels..(frame + 1) * channels),
                self.ring.get_mut(slot * channels..(slot + 1) * channels),
            ) else {
                break;
            };
            to.copy_from_slice(from);
        }
        self.written = self.written.saturating_add(quantum as u64);
        // The oldest frames a reader this far behind can no longer reach are evicted now,
        // so the read cursor never points at a frame the ring has already overwritten.
        let held = self.written.saturating_sub(self.consumed);
        let evicted = held.saturating_sub(capacity as u64);
        if evicted > 0 {
            self.consumed = self.consumed.saturating_add(evicted);
            self.dropped_since_read = self.dropped_since_read.saturating_add(evicted);
        }
    }
}

impl ObservationSubscriptions {
    /// Apply only at the matching sound transition's boundary, before rendering
    /// the successor. Preflight all entries before touching any cursor or binding.
    /// Old unread frames are counted as dropped; every subsequent read names the
    /// successor plan, so samples from different plans are never mixed unlabelled.
    /// The descriptor is borrowed and must be returned for off-thread destruction.
    ///
    /// # Errors
    /// Foreign/stale handoff or subscription changes; the store is untouched.
    pub fn handoff(
        &mut self,
        prepared: &ObservationHandoff,
    ) -> Result<(), ObservationHandoffError> {
        if self.id != prepared.store
            || self.plan != prepared.old
            || self.generation != prepared.generation
        {
            return Err(ObservationHandoffError::Origin);
        }
        if self.subscriptions.len() != prepared.rows.len()
            || self
                .subscriptions
                .iter()
                .zip(&prepared.rows)
                .any(|(subscription, row)| {
                    subscription.tap != row.source.tap
                        || subscription.channels != row.source.channels
                        || subscription.capacity_frames != row.source.capacity_frames
                })
        {
            return Err(ObservationHandoffError::Changed);
        }
        if self.subscriptions.iter().any(|subscription| {
            subscription.capacity_frames == 0
                || subscription.channels == 0
                || subscription
                    .capacity_frames
                    .checked_mul(subscription.channels)
                    != Some(subscription.ring.len())
                || subscription.consumed > subscription.written
                || u64::try_from(subscription.capacity_frames)
                    .ok()
                    .is_none_or(|capacity| subscription.written - subscription.consumed > capacity)
        }) {
            return Err(ObservationHandoffError::Geometry);
        }
        for (subscription, row) in self.subscriptions.iter_mut().zip(&prepared.rows) {
            let unread = subscription.written.saturating_sub(subscription.consumed);
            subscription.dropped_since_read =
                subscription.dropped_since_read.saturating_add(unread);
            subscription.consumed = subscription.written;
            subscription.tap = row.destination;
        }
        self.plan = prepared.new;
        self.generation = prepared.successor;
        Ok(())
    }
}

impl ObservationHandoff {
    pub const fn new_plan(&self) -> PlanId {
        self.new
    }
    pub const fn successor_generation(&self) -> ObservationGeneration {
        self.successor
    }

    #[cfg(feature = "simulated-ingress")]
    pub(crate) fn matches_installed(&self, store: &ObservationSubscriptions) -> bool {
        store.id == self.store && store.plan == self.new && store.generation == self.successor
    }
}

impl ObservationSubscriptions {
    /// Copies at most `maximum` whole frames into preallocated caller storage.
    /// Callback hosts must separately admit that work bound. The report labels the
    /// copied tail, evicted unread frames and remaining lag. A foreign handle returns
    /// `None`; this operation neither allocates nor releases ring storage.
    pub fn read_bounded(
        &mut self,
        id: SubscriptionId,
        into: &mut [f32],
        maximum: FrameCount,
    ) -> Option<ObservationRead> {
        if id.store != self.id {
            return None;
        }
        let subscription = self.subscriptions.get_mut(id.index)?;
        let channels = subscription.channels.max(1);
        let available = subscription.written.saturating_sub(subscription.consumed);
        let wanted = u64::try_from(into.len() / channels).unwrap_or(u64::MAX);
        let frames = available.min(wanted).min(maximum.as_u64());
        let capacity = subscription.capacity_frames as u64;
        for frame in 0..frames {
            let source_frame = usize::try_from((subscription.consumed + frame) % capacity).ok()?;
            let source = source_frame * channels;
            let target = usize::try_from(frame).ok()? * channels;
            if let (Some(from), Some(to)) = (
                subscription.ring.get(source..source + channels),
                into.get_mut(target..target + channels),
            ) {
                to.copy_from_slice(from);
            }
        }
        subscription.consumed += frames;
        let dropped = core::mem::take(&mut subscription.dropped_since_read);
        Some(ObservationRead {
            plan: self.plan,
            generation: self.generation,
            ended: subscription.tap.is_none(),
            frames: FrameCount::new(frames),
            dropped: FrameCount::new(dropped),
            behind: FrameCount::new(available - frames),
        })
    }
}
