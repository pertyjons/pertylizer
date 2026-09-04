//! The performance-event ingress store: the live producer's bounded source storage.
//!
//! `HOST-INV-009`'s closed
//! [renderer-ingress source-store registry](../../../plans/v2/specs/spec-host-profile-and-render-limits.md)
//! has one row, and this is it: the performance-event ingress queue, marked *Live bounded
//! queue*, which is what licenses a counted drop at this boundary and at no other.
//!
//! # What a note-on has to acquire, and why all three at once
//!
//! [ADR-0046](../../../plans/v2/decisions/ADR-0046-destination-quantum-admission.md)
//! clause 6 gives every non-compiled note-on a **release hold**, "because a future
//! external note-off is not yet knowable", and clause 3 requires the hold to be acquired
//! atomically with the event slot. `HOST-INV-009` adds the third: an identity from the
//! producer's admitted range. Taking any one without the others is a defect with a name —
//! a queued note-on with no hold is a note whose release can be dropped by queue pressure,
//! which is exactly what clause 6 says holds exist to prevent; a hold with no identity is
//! an obligation nothing can discharge.
//!
//! So an offer decides the two cheap resources **before** minting,
//! and mints last. That ordering is what makes the acquisition atomic without a rollback
//! path: a refusal on the slot or the hold has taken nothing, and a failed mint leaves the
//! table exactly as it found it, which `IdentityTable::mint` already guarantees.
//!
//! # The two halves, and what this crate does and does not model
//!
//! The store has an off-thread half — the offers, which mint, take the hold and write the
//! entry, reached through `StreamControl` because it owns the minter — and an audio-thread
//! half, the drain in `ingress/hot.rs`
//! that the real-time purity scan covers. That is the split the render contract's *stream
//! has two owners* rule describes.
//!
//! **What this crate does not model is the thread handoff itself**, and that is
//! deliberate rather than an oversight: no boundary in `synth_engine_v2` does.
//! `CompiledEventScheduler::offer` takes an activation built off-thread into a value the
//! audio thread owns, and the transport exchange hands a retired box back the same way;
//! in both, the host owns the actual handoff and the crate owns the contract on either
//! side of it. Ingress is not made the exception, because the first producer that really
//! runs on another thread is a hardware adapter, and Phase 9 owns those. A lock-free
//! encoding here would be machinery Phase 3 never executes across two threads.
//!
//! # The forward horizon is checked here, and this is `HOST-INV-013`'s one evaluation
//!
//! The invariant places the check at ingress admission into bounded source storage, which
//! this store is, and says it is evaluated *exactly once*. When this store was first built
//! the renderer had checked ingress provenance since Phase 1 — when it was the admission
//! point — so taking the check here created a **second** site, and the slice cut it rather
//! than guess which one the invariant meant.
//!
//! Four ways of having both were tried in that slice and every one was refused by
//! independent review: exempting `TimeSource::Simulated` at the renderer, which a public
//! `EventEnvelope::new` forges; arguing the second site harmless because it can never reject
//! what the first accepted, which is true and still a second evaluation; skipping the check
//! for an arbiter-sealed span, which the arbiter's necessarily public API forges just as
//! easily, and which left a forged event's horizon evaluated *zero* times; and removing the
//! renderer's site, which leaves an in-span ingress event handed straight to
//! `Renderer::render` evaluated zero times too.
//!
//! **A merge-gate review then supplied the argument those rounds had missed**, and it is why
//! the renderer's site could not be the survivor: the drain releases an entry only once
//! `Publication::reaches` it, so a far-future event never arrives at the renderer while it is
//! still far-future. The renderer's check can never see it, while the entry — and a note-on's
//! hold — occupies the queue until its own timestamp comes round. The maintainer settled the
//! reading on 2026-09-01: this boundary is the site, and the renderer's evaluation is
//! retired. The accepted cost is that a caller-assembled span carrying ingress provenance now
//! meets only the span check.
//!
//! A stamp **behind** the clock is not refused here either. ADR-0032 clause 21 is explicit
//! that the backward direction has no budget: ADR-0043's preserving late clamp moves such an
//! event to the first not-yet-rendered boundary and counts it, and dropping it here would be
//! the one thing ADR-0001 clause 16 forbids.

use thiserror::Error;

use crate::identity::{IdentityError, IdentityTable, NoteIdentity, ProducerId, Resolution};
#[cfg(feature = "simulated-ingress")]
use crate::plan::CompiledPlan;
use crate::plan::{NoteSlot, ParameterSlot};
#[cfg(feature = "simulated-ingress")]
use crate::profile::HostProfile;
use crate::quantities::{EventCount, ParameterValue};
use crate::render::{EventEnvelope, EventPayload, NoteEdge, TimedEvent};
use crate::time::{SampleTime, StreamEpoch, TimeSource};

pub(crate) mod hot;

/// One ingress store's identity.
///
/// Strictly increasing and never reissued, the same shape `ArbiterId` uses and for the same
/// reason: a stream must be able to tell "the store I have been serving" from "an equally
/// valid store someone just built". Two stores for one producer each hold the producer's
/// whole hold entitlement, so the pair can admit twice what ADR-0046 clause 6 partitioned —
/// and a release offered to the wrong one finds a live occurrence in the shared table,
/// spends a reservation it never made, and leaves the store that minted the note holding a
/// hold nothing can discharge. An independent review found that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[must_use]
pub struct IngressStoreId(u64);

impl IngressStoreId {
    /// The raw value, for a diagnostic.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[cfg(feature = "simulated-ingress")]
static NEXT_INGRESS_STORE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Issue the next store identity, or `None` once the space is spent.
///
/// Refuses **permanently** rather than wrapping, exactly as `issue_arbiter_id` does and for
/// the same reason: a reissued identity makes two stores indistinguishable, which is the one
/// thing the latch relies on. `fetch_add` is wrong here and a first draft used it — at
/// `u64::MAX` it wraps to zero, so the refusal would be one hiccup rather than an end, and
/// an independent review found it.
#[cfg(feature = "simulated-ingress")]
fn issue_ingress_store_id() -> Option<IngressStoreId> {
    use std::sync::atomic::Ordering::Relaxed;
    NEXT_INGRESS_STORE_ID
        .fetch_update(Relaxed, Relaxed, |id| id.checked_add(1))
        .ok()
        .map(IngressStoreId)
}

impl std::fmt::Display for IngressStoreId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ingress store {}", self.0)
    }
}

/// Which resource an offer could not acquire.
///
/// `HOST-INV-009` requires the exhausted resource to be named, "so the three causes stay
/// distinguishable in the report". They need different fixes: a full queue is a producer
/// outrunning the render callback, an exhausted hold entitlement is a plan admitted for
/// fewer simultaneous notes than the performer plays, and an exhausted identity range is
/// the same shortage seen from `SOUND-INV-017`'s side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum ExhaustedResource {
    /// No free entry in the bounded queue.
    Slot,
    /// The producer's release-hold entitlement is fully outstanding.
    Hold,
    /// The producer's admitted identity range holds no free index.
    Identity,
}

impl std::fmt::Display for ExhaustedResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Slot => "queue slot",
            Self::Hold => "release hold",
            Self::Identity => "note identity",
        })
    }
}

/// Why an offer was not accepted.
///
/// **A drop and a refusal are different outcomes and are counted apart.** `HOST-INV-009`
/// licenses the drop for a shortage at this boundary; the other two variants are not
/// shortages. An orphan release is `SOUND-INV-017`'s refusal and a non-monotone stamp is a
/// producer offering a past it already left — neither consumes the drop licence, and
/// reporting either as a drop would make a producer look starved when it is in fact offering
/// something the stream cannot accept at all.
///
/// **There is no foreign-epoch variant, and its absence is checked rather than assumed.**
/// A producer does not supply an epoch: [`PerformanceIngress::prepare`] takes the stream's,
/// and every entry is stamped with it. ADR-0032 clause 20's stale-epoch filter still
/// applies where an epoch can differ — at publication, after a re-preparation — which is
/// where ADR-0046 clause 3 already places it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IngressRefused {
    /// A shortage at the live boundary. The event is dropped and counted.
    #[error("the live boundary has no free {resource}")]
    Dropped {
        /// Which of the three resources was exhausted.
        resource: ExhaustedResource,
    },
    /// The stamp is behind one this store has already accepted.
    ///
    /// **A live producer's stamps are non-decreasing, and refusing otherwise is a
    /// correctness rule rather than tidiness.** A release frees its occurrence's index when
    /// it is offered, so the next note-on can reuse that index at the following generation.
    /// If the reused note could carry an *earlier* stamp than the queued release, the
    /// renderer — which applies by render position — would start it before the release it
    /// followed, and the accepted release would arrive at a note that had already been
    /// replaced and be refused as an orphan. Polyphony would then exceed the range the
    /// producer was admitted for, at an instant nothing checked.
    ///
    /// Refusing is not a drop: nothing was short. It is a producer offering a past it
    /// already left.
    #[error("stamp {time} is behind {last}, which this store has already accepted")]
    NonMonotoneStamp {
        /// The offered stamp.
        time: SampleTime,
        /// The latest stamp already accepted.
        last: SampleTime,
    },
    /// This store belongs to, or was adopted by, a different stream.
    ///
    /// A store takes its hold entitlement and identity range from one plan and mints through
    /// one stream's table. Handing it to a second stream makes those halves disagree: an
    /// eight-hold entitlement minting into a two-index range spends an entitlement the
    /// second stream never admitted and makes an identity shortage reachable although each
    /// plan separately satisfies `holds <= notes`. `prepare` binds the store to the plan it
    /// was prepared against; this binds it to the stream that first used it.
    #[error("this store belongs to {adopted}, but {offered} tried to use it")]
    ForeignStream {
        /// The stream that owns the store.
        adopted: StreamEpoch,
        /// The stream that tried to use it.
        offered: StreamEpoch,
    },
    /// This stream is already served by another ingress store.
    ///
    /// The host-profile specification's registry has **one** live renderer-ingress row, and
    /// ADR-0046 clause 6 partitions hold entitlements so that "no producer borrows another's
    /// unused holds". Two stores would each hold the producer's whole entitlement. A second
    /// live store is not forbidden in principle — it needs its own registry row and its own
    /// admitting ground first, which is the registry's own rule.
    #[error("this stream is served by {latched}, but {offered} was supplied")]
    ForeignStore {
        /// The store this stream adopted.
        latched: IngressStoreId,
        /// The store that was offered.
        offered: IngressStoreId,
    },
    /// The stamp is further ahead than the forward horizon allows.
    ///
    /// `HOST-INV-013` evaluates the horizon exactly once, at the boundary that admits into
    /// bounded source storage. **This is that boundary**, and the renderer's check is retired
    /// rather than kept alongside: four ways of having both were tried in the ingress slice
    /// and each was refused by independent review, and the maintainer settled the reading on
    /// 2026-09-01.
    ///
    /// Why it must be here rather than downstream: the drain releases an entry only once
    /// `Publication::reaches` it, so a far-future event never reaches the renderer while it
    /// is still far-future. A check downstream therefore cannot reject against the horizon
    /// the offer was made under, and the entry — with a note-on's hold — occupies the queue
    /// until its own timestamp comes round.
    #[error("stamp {time} is past the forward horizon ending at {horizon_end}")]
    BeyondHorizon {
        /// The stamp offered.
        time: SampleTime,
        /// The last stamp this store would admit.
        horizon_end: SampleTime,
    },
    /// The note slot indexes a different plan than the stream renders.
    ///
    /// [`crate::plan::NoteSlot`] carries the `PlanId` it was resolved against, and the
    /// renderer does **not** re-check it: `note_target` takes the slot's index and applies it
    /// to the plan currently rendering. A slot resolved against plan B and offered to a
    /// stream on plan A therefore plays plan A's target at B's numeric index — a different
    /// note, silently, with no error anywhere. An independent review found this at the merge
    /// gate; nothing on the offer path compared the two ids.
    ///
    /// Refused before the mint and before the hold, so a rejected offer leaves no identity
    /// and spends no entitlement.
    #[error("note slot indexes {slot}, but this stream renders {stream}")]
    ForeignSlot {
        /// The plan the slot was resolved against.
        slot: crate::plan::PlanId,
        /// The plan this stream renders.
        stream: crate::plan::PlanId,
    },
    /// A transport-activation candidate is outstanding, so the minter may not move.
    ///
    /// Each candidate holds a snapshot of the minter that its promotion will install, so a
    /// mint committed in between would have its generation rewound by that promotion and a
    /// later note could be handed an identity that is already live. ADR-0050 clause 8
    /// scopes transport activation to a stream whose note producers are compiled, so this
    /// combination is out of scope rather than supported — refused by name here instead of
    /// left to corrupt the snapshot.
    #[error("a transport-activation candidate is outstanding, so no live note may be minted")]
    CandidateOutstanding,
    /// A release naming a note this producer does not hold open.
    ///
    /// Refused rather than queued, because the renderer would refuse it as an orphan one
    /// pass later — and by then the hold it should have redeemed is already spent. Counted
    /// as an orphan rather than a drop, exactly as `HOST-INV-009` distinguishes them.
    #[error("release names {identity}, which this producer does not hold open")]
    OrphanRelease {
        /// The occurrence the release named.
        identity: NoteIdentity,
    },
}

/// What one queue entry is.
///
/// A stamped [`TimedEvent`] plus whether it redeems a hold, which is the one thing the
/// drain cannot recompute: ADR-0046 clause 6 charges a live note-on to the live share and
/// its matching release to the guaranteed-release share, and the entry is the only place
/// that pairing survives.
#[derive(Debug, Clone, Copy, PartialEq)]
struct IngressEntry {
    event: TimedEvent,
    redeems_hold: bool,
}

/// Counted outcomes at the live boundary.
///
/// Kept on the store rather than on the renderer's report because the producing half is
/// where they happen — before acceptance, as `HOST-INV-009` requires — and the renderer has
/// not seen the event at all. `HOST-INV-009` also requires the counts to reach the
/// structured report, so the drain mirrors them there: it is the only point at which the two
/// halves meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub struct IngressCounters {
    dropped_slot: u64,
    dropped_hold: u64,
    dropped_identity: u64,
    orphan_releases: u64,
    non_monotone: u64,
    beyond_horizon: u64,
}

impl IngressCounters {
    /// Events refused because their stamp is past the forward horizon.
    ///
    /// `HOST-INV-013` evaluates the horizon **exactly once**, and this is that site: the
    /// boundary admitting into bounded source storage. The renderer's own check is retired,
    /// which is the reading the maintainer selected on 2026-09-01 from the option space four
    /// refuted mechanisms produced.
    pub const fn beyond_horizon(&self) -> u64 {
        self.beyond_horizon
    }

    /// Events dropped because the queue was full.
    pub const fn dropped_slot(&self) -> u64 {
        self.dropped_slot
    }

    /// Note-ons dropped because the producer's hold entitlement was outstanding.
    pub const fn dropped_hold(&self) -> u64 {
        self.dropped_hold
    }

    /// Note-ons dropped because the producer's identity range was exhausted.
    pub const fn dropped_identity(&self) -> u64 {
        self.dropped_identity
    }

    /// Every drop at this boundary, whatever the resource.
    ///
    /// The sum is offered beside the three causes rather than instead of them: a report
    /// that carried only the total could not tell a producer outrunning the callback from
    /// a plan admitted for too few notes.
    pub const fn dropped(&self) -> u64 {
        self.dropped_slot
            .saturating_add(self.dropped_hold)
            .saturating_add(self.dropped_identity)
    }

    /// Releases refused because this producer holds no such note open.
    pub const fn orphan_releases(&self) -> u64 {
        self.orphan_releases
    }

    /// Offers refused for carrying a stamp behind one already accepted.
    pub const fn non_monotone(&self) -> u64 {
        self.non_monotone
    }
}

/// One live producer's bounded ingress store.
///
/// Preallocated to `performance_ingress_capacity` and written by index. It is a ring:
/// `head` is where the next entry is written and `tail` where the next one is read, both
/// modulo the capacity, so a drained entry's slot is reusable without moving anything.
///
/// # A release always has room, and that is arithmetic rather than a promise
///
/// ADR-0046 clause 6: "Once the note-on is published, its matching release cannot be
/// dropped by queue pressure." A store that merely wrote the release into whatever space
/// was left would overwrite an undrained entry the moment the queue filled, which loses a
/// different event to keep a promise about this one.
///
/// So the queue reserves one slot for every outstanding hold. A note-on or a parameter
/// write needs a free slot **above** that reservation; a release consumes the reservation
/// its own note-on made. `prepare` refuses a store whose depth cannot cover the producer's
/// entitlement, so the reservation can never exceed the queue.
#[derive(Debug)]
#[must_use]
pub struct PerformanceIngress {
    id: IngressStoreId,
    entries: Vec<Option<IngressEntry>>,
    head: usize,
    tail: usize,
    len: usize,
    /// The producer whose range this store mints from and whose entitlement it spends.
    producer: ProducerId,
    /// ADR-0046 clause 6's disjoint entitlement for this producer.
    hold_entitlement: EventCount,
    /// How far past the drain's clock a stamp may be, from the profile this store prepared
    /// against. `HOST-INV-013`'s single evaluation is made against it in `admit`.
    forward_event_horizon: crate::time::FrameCount,
    /// Release obligations this store has taken and not yet queued a release for.
    ///
    /// An `EventCount` because that is what `release_hold_capacity` and the producer's
    /// entitlement are: comparing an obligation count against a raw `u32` is the confusion
    /// the newtype rule exists to prevent, and an independent review found the raw one here.
    holds_outstanding: EventCount,
    epoch: StreamEpoch,
    /// The clock the drain last saw.
    ///
    /// What `admit` measures the forward horizon against: the horizon is measured
    /// from a clock this half does not own, and the drain is the only point at which it can
    /// record one. It is this rather than a caller's notion of "now", which would let a
    /// caller name a later one and smuggle an entry past. A store whose drain has not run
    /// measures from its origin, which admits less rather than more.
    clock: SampleTime,
    /// The latest stamp this store has accepted, so a later offer cannot go behind it.
    last_accepted: Option<SampleTime>,
    /// The stream that adopted this store, once one has.
    ///
    /// **One mark, read by both halves.** The off-thread half sets it when it first accepts
    /// an offer, and the audio-thread half verifies it before draining. Two independent
    /// latches were the first design and they could disagree: a caller offering into store A
    /// while rendering store B latched one on each, and the stream wedged — offers only
    /// reached A and drains only accepted B, permanently. An independent review found it.
    /// One authority cannot disagree with itself, and it also refuses a store already
    /// adopted by a different stream, which keeps one plan's entitlement out of another
    /// plan's identity table.
    adopted_by: Option<StreamEpoch>,
    counters: IngressCounters,
}

/// Why a store could not be prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IngressPrepareError {
    /// The plan does not declare the named producer.
    #[error("the plan declares no note producer {producer}")]
    UnknownProducer {
        /// The producer that was asked for.
        producer: ProducerId,
    },
    /// The plan also declares a compiled note producer.
    ///
    /// ADR-0051 clause 6 leaves a scalar gate reached by more than one producer with no
    /// ownership law, and ADR-0023 makes that the boundary bounding what may be built:
    /// ending one producer's note writes `ZERO` to a gate another holds and cuts its note
    /// with it. A live store beside a compiled producer is that plan, because nothing in a
    /// declaration says which nodes a producer plays — so the conservative reading, and the
    /// only one the types support, is that the two can reach one gate.
    ///
    /// **Refused here rather than asserted in prose**, which is the repair an independent
    /// review forced: plans declaring both producers already existed in this crate's
    /// fixtures, and before live ingress they were harmless because a non-compiled
    /// declaration could not emit. This slice makes one emit, so the boundary has to be a
    /// check rather than a claim about what the repository happens to contain. Those
    /// fixtures never build a store and are unaffected.
    #[error("the plan declares compiled note producer {compiled}, which no live store may join")]
    MixedProducerPlan {
        /// The plan's compiled producer.
        compiled: ProducerId,
    },
    /// The plan's compiled producer cannot own a live ingress store.
    ///
    /// ADR-0046 clause 6: "Compiled releases use plan entitlements and need no hold", so a
    /// compiled producer declares none. Preparing a live store against it would give live
    /// note-ons an entitlement of zero and drop every one of them, which reports a
    /// shortage for what is really a plan that never declared a live producer.
    #[error(
        "note producer {producer} is the plan's compiled producer and holds no live entitlement"
    )]
    CompiledProducer {
        /// The producer that was asked for.
        producer: ProducerId,
    },
    /// The producer's entitlement is already claimed by an authored runtime source.
    ///
    /// ADR-0046 clause 6 partitions hold entitlements so that "no producer borrows another's
    /// unused holds", and an entitlement belongs to a producer rather than to a claimant.
    /// Plan admission checks that a producer's authored sources fit its entitlement; it
    /// cannot see the stores prepared afterwards, so this is where the second claim is
    /// refused. Without it a plan could reserve four holds, admit an authored source against
    /// all four, and then hand the same four to the live store — which an independent review
    /// found reachable, because the authored link proved only that the index resolved and was
    /// non-compiled.
    #[error("note producer {producer} is claimed by an authored runtime source")]
    AuthoredProducer {
        /// The producer that was asked for.
        producer: ProducerId,
    },
    /// The store's depth cannot be represented on this machine.
    #[error("performance_ingress_capacity {capacity} does not fit an index")]
    CapacityUnrepresentable {
        /// The requested depth.
        capacity: EventCount,
    },
    /// The store would serve a plan the renderer does not render.
    ///
    /// A store takes its hold entitlement and its identity range from the plan it is given
    /// and mints through the **stream's** table. Two different plans make those two halves
    /// disagree: a store could hold plan A's eight-hold entitlement while minting into plan
    /// B's two-index range, which spends an entitlement plan B never admitted and makes an
    /// identity shortage reachable although each plan separately satisfies `holds <= notes`.
    /// An independent review found it.
    #[error("the store's plan {store} is not the renderer's {renderer}")]
    ForeignPlan {
        /// The plan the store was prepared from.
        store: crate::plan::PlanId,
        /// The plan the renderer renders.
        renderer: crate::plan::PlanId,
    },
    /// The store identity space is spent.
    ///
    /// Refused rather than reissued: the latch that keeps one store per stream compares
    /// identities, and a reissued one makes two stores indistinguishable.
    #[error("the ingress store identity space is exhausted")]
    IdentitySpaceExhausted,
    /// The queue cannot reserve a release slot for every hold the producer may take.
    ///
    /// ADR-0046 clause 6 promises that an accepted note-on's release cannot be dropped by
    /// queue pressure. This store keeps that promise by reserving a slot per outstanding
    /// hold, which is only possible while the depth covers the entitlement. Refused at
    /// preparation, where the profile and the plan can both still be changed.
    #[error(
        "performance_ingress_capacity {capacity} cannot reserve a release slot for each of \
         producer {producer}'s {entitlement} holds"
    )]
    CapacityBelowEntitlement {
        /// The requested depth.
        capacity: EventCount,
        /// The producer's hold entitlement.
        entitlement: EventCount,
        /// The producer that was asked for.
        producer: ProducerId,
    },
}

impl PerformanceIngress {
    /// Prepare the store for one declared live producer.
    ///
    /// Every allocation this store will ever need happens here, which is the same rule
    /// `PublicationArbiter::prepare` follows and for the same reason: the drain may not
    /// allocate.
    /// The epoch and the origin come from the renderer rather than from the caller, so the
    /// store cannot be prepared against a stream it does not serve: a stamp carrying
    /// another epoch would be filtered at publication, and a clock taken from a different
    /// origin would be the wrong reference for `admit`'s horizon check.
    #[cfg(feature = "simulated-ingress")]
    pub fn prepare(
        profile: &HostProfile,
        plan: &CompiledPlan,
        producer: ProducerId,
        renderer: &crate::render::PreparedRenderer,
    ) -> Result<Self, IngressPrepareError> {
        let epoch = renderer.epoch();
        // **The store's plan must be the renderer's**, because the entitlement comes from
        // one and the identity range is minted through the other.
        if plan.id() != renderer.plan().id() {
            return Err(IngressPrepareError::ForeignPlan {
                store: plan.id(),
                renderer: renderer.plan().id(),
            });
        }

        let index = usize::from(producer.as_u16());
        let Some(hold_entitlement) = plan.note_producer_holds().get(index).copied() else {
            return Err(IngressPrepareError::UnknownProducer { producer });
        };
        if plan.compiled_note_producer() == Some(producer) {
            return Err(IngressPrepareError::CompiledProducer { producer });
        }
        if plan.authored_note_producers().contains(&producer) {
            return Err(IngressPrepareError::AuthoredProducer { producer });
        }
        if let Some(compiled) = plan.compiled_note_producer() {
            return Err(IngressPrepareError::MixedProducerPlan { compiled });
        }

        let Some(id) = issue_ingress_store_id() else {
            return Err(IngressPrepareError::IdentitySpaceExhausted);
        };

        let capacity = profile
            .limits()
            .events()
            .queues()
            .performance_ingress_capacity();
        let Some(depth) = capacity.as_usize() else {
            return Err(IngressPrepareError::CapacityUnrepresentable { capacity });
        };
        if capacity < hold_entitlement {
            return Err(IngressPrepareError::CapacityBelowEntitlement {
                capacity,
                entitlement: hold_entitlement,
                producer,
            });
        }

        Ok(Self {
            id,
            entries: vec![None; depth],
            head: 0,
            tail: 0,
            len: 0,
            producer,
            hold_entitlement,
            forward_event_horizon: profile.limits().events().forward_event_horizon(),
            holds_outstanding: EventCount::NONE,
            epoch,
            clock: renderer.clock(),
            last_accepted: None,
            adopted_by: None,
            counters: IngressCounters::default(),
        })
    }

    /// The producer this store belongs to.
    pub const fn producer(&self) -> ProducerId {
        self.producer
    }

    /// This store's identity, which the stream adopts on first use.
    pub const fn id(&self) -> IngressStoreId {
        self.id
    }

    /// The stream that adopted this store, if one has.
    pub const fn adopted_by(&self) -> Option<StreamEpoch> {
        self.adopted_by
    }

    /// The stream epoch this store serves.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// Adopt this store into a stream, or refuse it to a second one.
    ///
    /// `pub(crate)` because the off-thread half owns the decision: `StreamControl` calls it
    /// from every offer, so a store nothing offered into is a store no stream adopted — and
    /// an unadopted store is necessarily empty, which is why the drain can refuse one
    /// without stranding anything.
    pub(crate) fn adopt(&mut self, stream: StreamEpoch) -> Result<(), IngressRefused> {
        match self.adopted_by {
            Some(adopted) if adopted != stream => Err(IngressRefused::ForeignStream {
                adopted,
                offered: stream,
            }),
            Some(_) => Ok(()),
            None if self.epoch != stream => Err(IngressRefused::ForeignStream {
                adopted: self.epoch,
                offered: stream,
            }),
            None => {
                self.adopted_by = Some(stream);
                Ok(())
            }
        }
    }

    /// What this boundary has counted.
    pub const fn counters(&self) -> IngressCounters {
        self.counters
    }

    /// Entries waiting to be drained.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether nothing is waiting.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Holds this producer has outstanding.
    pub const fn holds_outstanding(&self) -> EventCount {
        self.holds_outstanding
    }

    /// Whether one new entry fits, together with `holds` further release reservations it
    /// would itself create.
    ///
    /// The invariant is `occupied + outstanding holds <= capacity`: every queued entry has
    /// a slot, and every hold has a slot kept for the release that will redeem it. A
    /// **note-on therefore needs two units** — its own entry and the reservation for the
    /// release it does not yet know about — which is why the count is a parameter rather
    /// than a constant. Asking for one unit for a note-on is the off-by-one that lets the
    /// last note-on into a queue with no room for its own release, and the promise this
    /// reservation exists to keep is exactly that release.
    ///
    /// A release asks nothing: it converts a reservation into an entry, so the sum does not
    /// move and the invariant carries it.
    fn room_for(&self, holds: usize) -> bool {
        let needed = self
            .len
            .saturating_add(self.holds_outstanding.get() as usize)
            .saturating_add(1)
            .saturating_add(holds);
        needed <= self.entries.len()
    }

    /// Offer a parameter write.
    ///
    /// It takes a slot and nothing else: a control write opens no obligation, so ADR-0046
    /// clause 6's hold does not reach it.
    ///
    /// **Crate-private for the reason the note offers are**, although it needs no minter: the
    /// stream latches one store, and an offer that bypassed the latch would let a second
    /// store fill a queue the stream never adopted — and overwrite the first store's
    /// cumulative counters in the report, since the drain mirrors rather than accumulates.
    /// An independent review found the gap.
    pub(crate) fn offer_parameter(
        &mut self,
        time: SampleTime,
        slot: ParameterSlot,
        value: ParameterValue,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;
        if !self.room_for(0) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        self.push(time, EventPayload::SetParameter { slot, value }, false);
        Ok(())
    }

    /// Offer a note-on, acquiring its slot, hold and identity together.
    ///
    /// Returns the occurrence, because the caller needs it to release the note: the
    /// producer, not this store, decides which of its open notes an incoming note-off
    /// belongs to.
    ///
    /// **Crate-private, and reached through `StreamControl`.** The render contract's *stream
    /// has two owners* rule puts the identity **minter** on the off-thread half, which is
    /// `StreamControl`; a public method taking any `IdentityTable` would let a caller mint
    /// from a table the renderer never heard of, and the renderer would then refuse every
    /// edge as foreign. Routing the offer through the owner makes that unrepresentable
    /// rather than merely wrong.
    pub(crate) fn offer_note_on(
        &mut self,
        table: &mut IdentityTable,
        time: SampleTime,
        note: NoteSlot,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Result<NoteIdentity, IngressRefused> {
        self.admit(time)?;

        // The two cheap resources first, and both **before** the mint. A mint that
        // succeeded into a full queue would have to be undone, and undoing it is what
        // reissues an index whose generation has already advanced.
        if !self.room_for(1) {
            self.counters.dropped_slot = self.counters.dropped_slot.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot,
            });
        }
        if self.holds_outstanding >= self.hold_entitlement {
            self.counters.dropped_hold = self.counters.dropped_hold.saturating_add(1);
            return Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Hold,
            });
        }

        // Last, because it is the only one whose failure leaves state behind if it is not.
        // `IdentityTable::mint` commits nothing on failure, so a refusal here has taken
        // neither the slot nor the hold either.
        let identity = match table.mint(self.producer, note) {
            Ok(identity) => identity,
            Err(
                IdentityError::ProducerOverEmitted { .. }
                | IdentityError::ProducerRangeEroded { .. },
            ) => {
                self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
                return Err(IngressRefused::Dropped {
                    resource: ExhaustedResource::Identity,
                });
            }
            Err(_) => {
                // An unknown producer cannot occur: `prepare` refused a producer the plan
                // does not declare, and the table is built from the same declarations.
                // Counted as an identity shortage rather than ignored, because a silent
                // `Ok` here would queue a note-on with no occurrence at all.
                self.counters.dropped_identity = self.counters.dropped_identity.saturating_add(1);
                return Err(IngressRefused::Dropped {
                    resource: ExhaustedResource::Identity,
                });
            }
        };

        self.holds_outstanding = self
            .holds_outstanding
            .checked_add(EventCount::measured(1))
            .unwrap_or(self.holds_outstanding);
        self.push(
            time,
            EventPayload::Note {
                identity,
                edge: NoteEdge::On {
                    slot: note,
                    key,
                    velocity,
                },
            },
            false,
        );
        Ok(identity)
    }

    /// Offer the release of a note this producer opened.
    ///
    /// **It cannot be dropped for a full queue**, and that is ADR-0046 clause 6 rather
    /// than a convenience: "Once the note-on is published, its matching release cannot be
    /// dropped by queue pressure." The hold acquired with the note-on is what reserves the
    /// room, so the release spends the hold and is written even when the queue is
    /// otherwise full — the queue is sized so that the notes a producer may hold open are
    /// exactly the releases it may owe.
    pub(crate) fn offer_note_off(
        &mut self,
        table: &mut IdentityTable,
        time: SampleTime,
        identity: NoteIdentity,
    ) -> Result<(), IngressRefused> {
        self.admit(time)?;

        // A release naming nothing open is refused here rather than queued. Queuing it
        // would spend a slot to reach a renderer that refuses it one pass later, and the
        // producer would learn about it a callback too late to correct anything.
        //
        // `release` frees the minter's index as it resolves, which is what lets the next
        // note-on reuse it. The freed index comes back at the **next** generation, so the
        // occurrence still in flight stays distinguishable from the one that reuses its
        // index — and the renderer applies this release before that note-on, because both
        // are this producer's queue entries and ADR-0023 keeps a producer's own order.
        // **This producer's own occurrence, and this store's own hold.** Checking the table
        // alone accepted any live identity in it, including a *compiled* producer's: the
        // release then ended a note this producer never opened, spent this store's hold on
        // it, and left this producer's own note occupied — so the next note-on found the
        // range full with a hold still free. `release_for` refuses an index outside the
        // producer's admitted range. An independent review found it.
        //
        // The hold check below is the second half. Checking the table alone
        // accepts a release for a note some *other* store of the same producer opened: this
        // store would then saturate a zero hold count to zero and push without the room its
        // reservation was supposed to guarantee, while the store that minted the note kept a
        // hold nothing can ever discharge. ADR-0046 clause 6 partitions entitlements so that
        // "no producer borrows another's unused holds", and an unowned release is exactly
        // that borrowing. An independent review found it.
        if self.holds_outstanding == EventCount::NONE {
            self.counters.orphan_releases = self.counters.orphan_releases.saturating_add(1);
            return Err(IngressRefused::OrphanRelease { identity });
        }
        if table.release_for(self.producer, identity) != Resolution::Live {
            self.counters.orphan_releases = self.counters.orphan_releases.saturating_add(1);
            return Err(IngressRefused::OrphanRelease { identity });
        }

        // **The hold is discharged here, not at publication**, and the two are not
        // interchangeable: the reservation exists to keep a queue slot free for this
        // release, and the release now occupies one. Holding it until the drain would
        // reserve a slot for an event that is already sitting in a slot, which shrinks the
        // usable queue by one per note in flight.
        self.holds_outstanding =
            EventCount::measured(self.holds_outstanding.get().saturating_sub(1));
        self.push(
            time,
            EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            },
            true,
        );
        Ok(())
    }

    /// The two checks every offer takes, whatever it carries.
    ///
    /// Monotonicity, and `HOST-INV-013`'s single evaluation of the forward horizon — see the
    /// module header for why this boundary is that site and the renderer's is retired.
    /// Nothing is checked against the stream's epoch: a store serves one epoch and
    /// re-preparation builds a new one, so a stamp from another stream cannot reach this.
    fn admit(&mut self, time: SampleTime) -> Result<(), IngressRefused> {
        if let Some(last) = self.last_accepted
            && time < last
        {
            self.counters.non_monotone = self.counters.non_monotone.saturating_add(1);
            return Err(IngressRefused::NonMonotoneStamp { time, last });
        }
        // `HOST-INV-013`'s single evaluation. Measured from the clock the **drain** last
        // recorded rather than from a caller's notion of now: this half does not own a clock,
        // and a horizon measured from an offer-side guess would move with the offerer.
        //
        // A store whose drain has not run yet measures from its origin, which is the
        // conservative direction — it admits less, never more.
        if let Ok(end) = self.clock.checked_add(self.forward_event_horizon)
            && time > end
        {
            self.counters.beyond_horizon = self.counters.beyond_horizon.saturating_add(1);
            return Err(IngressRefused::BeyondHorizon {
                time,
                horizon_end: end,
            });
        }
        Ok(())
    }

    /// Write one entry. Every caller has already decided it fits.
    fn push(&mut self, time: SampleTime, payload: EventPayload, redeems_hold: bool) {
        let entry = IngressEntry {
            event: TimedEvent::new(
                EventEnvelope::new(self.epoch, time, TimeSource::Simulated),
                payload,
            ),
            redeems_hold,
        };
        if let Some(slot) = self.entries.get_mut(self.head) {
            *slot = Some(entry);
        }
        self.last_accepted = Some(match self.last_accepted {
            Some(last) if last > time => last,
            _ => time,
        });
        self.head = self.head.saturating_add(1);
        if self.head >= self.entries.len() {
            self.head = 0;
        }
        self.len = self.len.saturating_add(1);
    }
}
