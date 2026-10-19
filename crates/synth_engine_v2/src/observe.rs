//! The host's subscription over a declared tap — `HOST-INV-023`'s reachable half.
//!
//! ADR-0027 splits observation four ways, and this module is the third: a **runtime
//! subscription** owns the bounded ring, the counters and the consumer's lifetime, and
//! nothing else. The tap it names is the compiled plan's (`SOUND-INV-022`) and exists whether
//! or not this module ever does; the plan, the compilation and every audio sample are the
//! same with a subscriber, without one, and with one that never reads — which is the exit
//! gate's fifth bullet, and what `tests/observation.rs` renders three ways to check.
//!
//! **Handed to the render call, like the ingress store.** The scheduler's
//! [`crate::schedule::CompiledEventScheduler::render_observed`] takes the store by `&mut`
//! for one call, and the renderer pushes each rendered quantum of each subscribed tap into
//! its ring after the schedule walk. The store is therefore the host's to hold and to hand
//! over, and the audio thread neither allocates nor owns it; the copy it performs is bounded
//! by the ring and the tap's layout, both fixed at subscription.
//!
//! **Lossy, and the loss is counted.** The ring holds `telemetry_ring_frames` frames
//! (`LIMIT-0021`). A reader that falls further behind than that loses the **oldest** frames —
//! V1 dropped the newest, and the limit's row records the change — and the next read reports
//! how many were evicted and how far behind the newest quantum it still stands
//! (`HOST-INV-019`'s expose-the-loss condition, at the API).
//!
//! A live stream takes the store the same way, through `LiveInputStream::render_observed`
//! (behind `simulated-ingress`); moving what a read returns to another thread is the
//! host's, bounded and non-blocking.
//!
//! What is **not** here, and owed: subscribing and unsubscribing while the stream renders
//! across a thread boundary, a store-level cross-thread transport, decimation, and the
//! versioned telemetry facade — Phase 10E's. Here a subscription is made between render calls
//! by the same caller that hands the store over.

use crate::plan::{CompiledPlan, PlanId, TapSlot};
use crate::profile::HostProfile;
use crate::quantities::{ChannelLayout, TapCount};
use crate::time::FrameCount;

mod handoff;
pub(crate) mod hot;
pub use handoff::{ObservationConfiguration, ObservationHandoff, ObservationHandoffError};

/// One store's checked plan-handoff generation, independent of its subscription IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ObservationGeneration(u64);

impl ObservationGeneration {
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

#[cfg(test)]
#[path = "tests/observation.rs"]
mod tests;

/// A store's identity, issued once per store from a process-wide counter.
///
/// What makes a [`SubscriptionId`] a handle rather than a position: two stores each issue
/// index zero, and a handle from one names nothing in the other because the store checks
/// this first. An independent review found the position-only form reading the wrong store's
/// samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct StoreId(u64);

static NEXT_STORE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl StoreId {
    fn issue(counter: &std::sync::atomic::AtomicU64) -> Result<Self, SubscriptionRefused> {
        use std::sync::atomic::Ordering::Relaxed;
        counter
            .try_update(Relaxed, Relaxed, |current| current.checked_add(1))
            .map(Self)
            .map_err(|_| SubscriptionRefused::IdentityExhausted)
    }
}

/// One subscription's handle: the store that issued it and its index there.
///
/// A handle from another store is refused by `read` and `channels` rather than indexing
/// this store's table, and the store checks the plan it was prepared for on every push, so
/// a handle cannot be used to read a tap of a plan it was not admitted against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct SubscriptionId {
    store: StoreId,
    index: usize,
}

impl SubscriptionId {
    /// The store that issued the handle.
    pub const fn store(self) -> StoreId {
        self.store
    }

    /// The index into that store's table.
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }
}

/// Why a subscription was refused. Refused, not ignored: a subscriber that believed it was
/// reading a tap would otherwise read nothing and conclude the signal was silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubscriptionRefused {
    /// Store identities never wrap or saturate into a previously issued handle.
    #[error("observation store identity space is exhausted")]
    IdentityExhausted,
    /// The slot or the plan offered belongs to another plan than the one the store was
    /// prepared for. All three are named, because the API takes the plan and the slot
    /// separately and either can be the one that does not match.
    #[error("tap slot of plan {slot} and plan {plan} offered to a store prepared for plan {store}")]
    ForeignPlan {
        /// The plan the slot names.
        slot: PlanId,
        /// The plan the caller supplied.
        plan: PlanId,
        /// The plan the store admits against.
        store: PlanId,
    },
    /// The plan has no tap at that index.
    #[error("the plan holds {taps} taps and has none at index {index}")]
    UnknownTap {
        /// The index offered.
        index: usize,
        /// How many taps the plan holds.
        taps: TapCount,
    },
    /// The tap already has a subscriber; one ring per tap is the budget.
    #[error("tap {index} already has a subscriber")]
    AlreadySubscribed {
        /// The tap's index.
        index: usize,
    },
    /// The profile's ring length does not fit the address space.
    #[error("a telemetry ring of {frames} frames is not representable here")]
    RingUnrepresentable {
        /// The profile's ring length.
        frames: FrameCount,
    },
}

/// What one read returned, beside the samples it copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ObservationRead {
    /// All returned frames belong to this plan. Handoff discards unread old frames.
    pub plan: PlanId,
    /// Changes even when a host later reinstalls an earlier immutable plan.
    pub generation: ObservationGeneration,
    /// A removed tap stays ended; a later same-ID declaration does not revive it.
    pub ended: bool,
    /// Frames copied into the caller's buffer.
    pub frames: FrameCount,
    /// Unread frames lost since the previous read, through ring eviction or a plan
    /// handoff. Zero when every frame was read before eviction or handoff.
    pub dropped: FrameCount,
    /// Frames still unread after this read: how far behind the newest rendered quantum the
    /// reader stands. Zero means the reader has everything the renderer has pushed.
    pub behind: FrameCount,
}

/// One tap's ring and cursors.
///
/// Both cursors are **totals** — frames ever written and frames ever consumed or evicted —
/// so `written - consumed` is what the ring holds and neither wraps within any session a
/// host can run: at 48 kHz a `u64` of frames is twelve million years.
#[derive(Debug)]
struct Subscription {
    tap: Option<TapSlot>,
    channels: usize,
    capacity_frames: usize,
    /// `capacity_frames × channels` samples, frame-major, written at `written % capacity`.
    ring: Vec<f32>,
    written: u64,
    consumed: u64,
    dropped_since_read: u64,
}

/// The host's subscriptions over one plan's declared taps.
#[derive(Debug)]
pub struct ObservationSubscriptions {
    id: StoreId,
    generation: ObservationGeneration,
    plan: PlanId,
    ring_frames: FrameCount,
    subscriptions: Vec<Subscription>,
}

impl ObservationSubscriptions {
    /// A store for one compiled plan, sized by the profile's telemetry window.
    ///
    /// Off the audio thread, and holding nothing yet: a ring is allocated when a tap is
    /// subscribed, which is also off the audio thread — the store is only ever handed to a
    /// render call between subscriptions.
    pub fn prepare(
        profile: &HostProfile,
        plan: &CompiledPlan,
    ) -> Result<Self, SubscriptionRefused> {
        Ok(Self {
            id: StoreId::issue(&NEXT_STORE_ID)?,
            generation: ObservationGeneration(0),
            plan: plan.id(),
            ring_frames: profile.limits().observation().telemetry_ring_frames(),
            subscriptions: Vec::new(),
        })
    }

    /// This store's identity, which every handle it issues carries.
    pub const fn id(&self) -> StoreId {
        self.id
    }

    /// The plan this store admits against.
    pub const fn plan(&self) -> PlanId {
        self.plan
    }

    /// Subscribe to one of the plan's declared taps.
    ///
    /// Admitted against the taps the compiled plan already holds and nothing else: a slot of
    /// another plan, an index the plan has no tap for, and a second subscriber on one tap are
    /// refused by name. It can neither fail nor change a compilation or a plan — it reads one
    /// — and it allocates the ring here, off the audio thread.
    pub fn subscribe(
        &mut self,
        plan: &CompiledPlan,
        slot: TapSlot,
    ) -> Result<SubscriptionId, SubscriptionRefused> {
        if slot.plan() != self.plan || plan.id() != self.plan {
            return Err(SubscriptionRefused::ForeignPlan {
                slot: slot.plan(),
                plan: plan.id(),
                store: self.plan,
            });
        }
        let taps = plan.taps();
        let Some(tap) = taps.get(slot.index()) else {
            return Err(SubscriptionRefused::UnknownTap {
                index: slot.index(),
                taps: TapCount::measured(u32::try_from(taps.len()).unwrap_or(u32::MAX)),
            });
        };
        if self
            .subscriptions
            .iter()
            .any(|existing| existing.tap == Some(slot))
        {
            return Err(SubscriptionRefused::AlreadySubscribed {
                index: slot.index(),
            });
        }
        let Some(capacity_frames) = self.ring_frames.as_usize() else {
            return Err(SubscriptionRefused::RingUnrepresentable {
                frames: self.ring_frames,
            });
        };
        // The tapped port's layout is the region's width over the quantum: the region the
        // tap names is one quantum of the port's channels, frame-major.
        let channels = plan
            .region(tap.region)
            .map_or(ChannelLayout::Mono.channels(), |region| {
                region.length() / (crate::time::QUANTUM_FRAMES as usize).max(1)
            })
            .max(1);
        // The ring's extent is checked before anything is allocated: a legal profile may
        // state a window no address space holds, and that is refused here rather than
        // reaching an allocator that panics on it.
        let samples = capacity_frames
            .checked_mul(channels)
            .filter(|samples| *samples <= isize::MAX as usize / size_of::<f32>())
            .ok_or(SubscriptionRefused::RingUnrepresentable {
                frames: self.ring_frames,
            })?;
        let mut ring: Vec<f32> = Vec::new();
        ring.try_reserve_exact(samples)
            .map_err(|_| SubscriptionRefused::RingUnrepresentable {
                frames: self.ring_frames,
            })?;
        ring.resize(samples, 0.0);
        let id = SubscriptionId {
            store: self.id,
            index: self.subscriptions.len(),
        };
        self.subscriptions.push(Subscription {
            tap: Some(slot),
            channels,
            capacity_frames,
            ring,
            written: 0,
            consumed: 0,
            dropped_since_read: 0,
        });
        Ok(id)
    }

    /// A host may install only handles with no prior samples or ending boundary.
    /// Reusing an ended or previously rendered handle could hide a coverage gap.
    pub fn is_pristine(&self) -> bool {
        self.generation.as_u64() == 0
            && self.subscriptions.iter().all(|subscription| {
                subscription.tap.is_some()
                    && subscription.written == 0
                    && subscription.consumed == 0
                    && subscription.dropped_since_read == 0
            })
    }

    /// Actual owned store and ring capacities, for a cold host transport budget.
    pub fn prepared_bytes(&self) -> Option<crate::quantities::PreparedBytes> {
        let mut bytes = self
            .subscriptions
            .capacity()
            .checked_mul(size_of::<Subscription>())?
            .checked_add(size_of::<Self>())?;
        for subscription in &self.subscriptions {
            bytes =
                bytes.checked_add(subscription.ring.capacity().checked_mul(size_of::<f32>())?)?;
        }
        Some(crate::quantities::PreparedBytes::measured(
            u64::try_from(bytes).ok()?,
        ))
    }

    /// How many subscriptions the store holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.subscriptions.len()
    }

    /// Whether the store holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
    }

    /// The channels one subscription's frames carry.
    #[must_use]
    pub fn channels(&self, id: SubscriptionId) -> Option<usize> {
        if id.store != self.id {
            return None;
        }
        self.subscriptions.get(id.index).map(|s| s.channels)
    }
    /// Cold convenience read of the available labelled tail into caller-owned storage.
    /// Callback users must call `read_bounded` with a separately admitted frame cap.
    pub fn read(&mut self, id: SubscriptionId, into: &mut [f32]) -> Option<ObservationRead> {
        self.read_bounded(id, into, FrameCount::new(u64::MAX))
    }
}
