//! The audio-thread half of a subscription: one bounded copy per quantum per subscriber.
//!
//! Inside the render loop's scanned region because it runs after each quantum's schedule
//! walk. It allocates nothing, locks nothing and cannot panic: every index is checked, the
//! ring was sized at subscription, and a subscription whose plan is not the renderer's
//! receives nothing. A copy is the whole of what observation costs the render, and it is
//! the same copy whether or not anyone reads.

use super::{ObservationSubscriptions, Subscription};
use crate::plan::{BufferRegion, PlanId, TapTarget};
use crate::time::QUANTUM_FRAMES;

impl ObservationSubscriptions {
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
            let Some(tap) = taps.get(subscription.tap.index()) else {
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
