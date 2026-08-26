//! The publication arbiter: the one normal writer of sealed renderer input.
//!
//! [ADR-0046](../../../plans/v2/decisions/ADR-0046-destination-quantum-admission.md)
//! clause 2 makes this the only path that constructs what a renderer call is presented
//! with. Producers write to their own bounded source storage; this module snapshots what
//! is eligible, fills the preallocated external quantum batches for exactly the quanta the
//! imminent call can render, and seals them before the first quantum begins.
//!
//! # What this module is for
//!
//! Two properties, and neither is obtainable by bounding producers one at a time.
//!
//! **Every event is charged to exactly one share.** Clause 1 partitions
//! `max_events_per_quantum` into six disjoint producer shares precisely so that the cap is
//! safe at the *joint* peak of every class. A ledger that counted only the quantum total
//! would admit a class overrunning its own entitlement while another class happened to be
//! quiet, which turns a declared share into a soft limit.
//!
//! **Slack is not recovery capacity.** Clause 7 is explicit that a share overrun is a
//! fault "even if unusable slack means the quantum total remains below
//! `max_events_per_quantum`". [`Publication::charge`] therefore refuses on the share, not
//! on the total, and the two are tested apart.
//!
//! # What this module does not do yet
//!
//! It does not read ingress queues, evaluate scheduler work, or drive the renderer, and
//! it holds no internal-emission arena — clause 2 keeps that separate from the external
//! batch and it arrives with the producer that needs it. A detected fault is returned to
//! the caller here; **wiring it to clause 7's terminal renderer response — silence over
//! this callback and every later one in the epoch, both carries invalidated,
//! `needs_reprepare` published — is the integration slice that routes the renderer
//! through this arbiter.** Until then a fault is reported and not yet enacted.

use thiserror::Error;

use crate::profile::HostProfile;
use crate::quantities::{EventCount, QuantumCount};
use crate::render::TimedEvent;
use crate::time::SampleTime;

/// Which row of an open publication window a quantum occupies.
///
/// Zero is the window's first quantum, whatever absolute quantum that is. A newtype rather
/// than a `usize` because it is a position in one specific window: comparing it with an
/// absolute quantum index, an event count or a frame offset is exactly the confusion the
/// critical newtype rule exists to prevent, and all four are integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct WindowRow(usize);

impl WindowRow {
    /// The window's first quantum.
    pub const FIRST: Self = Self(0);

    /// A row. Any offset names one; whether the open window *holds* it is the arbiter's.
    pub const fn new(offset: usize) -> Self {
        Self(offset)
    }

    /// The offset from the window's first quantum.
    pub const fn offset(self) -> usize {
        self.0
    }
}

impl std::fmt::Display for WindowRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "window row {}", self.0)
    }
}

/// Which producer class an event is charged to.
///
/// Exactly ADR-0046 clause 1's six, and the exhaustive match in [`Self::share_of`] is what
/// keeps that true: a class added here without a share does not compile, and a share added
/// to the profile without a class has no way to be spent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProducerClass {
    /// Compiled timeline and automation.
    Compiled,
    /// Authored runtime expansion.
    AuthoredRuntime,
    /// Live ingress.
    Live,
    /// Session and transport.
    Session,
    /// Renderer-internal production.
    ///
    /// **Not publishable into the external batch.** ADR-0046 clause 2 gives internal
    /// emissions a separate preallocated arena and ledger on the renderer side of the
    /// seal, and says they "never reopen or mutate the external batch". The class exists
    /// here because its share is part of the same partition, and
    /// [`Publication::charge`] refuses it by name rather than silently accepting it.
    Internal,
    /// Guaranteed releases.
    Release,
}

impl std::fmt::Display for ProducerClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl ProducerClass {
    /// Every class, for a ledger that must cover all of them.
    pub const ALL: [Self; 6] = [
        Self::Compiled,
        Self::AuthoredRuntime,
        Self::Live,
        Self::Session,
        Self::Internal,
        Self::Release,
    ];

    /// This class's index into a per-quantum ledger row.
    pub const fn index(self) -> usize {
        match self {
            Self::Compiled => 0,
            Self::AuthoredRuntime => 1,
            Self::Live => 2,
            Self::Session => 3,
            Self::Internal => 4,
            Self::Release => 5,
        }
    }

    /// The share this class spends, from the profile that admitted the stream.
    pub fn share_of(self, profile: &HostProfile) -> EventCount {
        let shares = profile.limits().events().shares();
        match self {
            Self::Compiled => shares.compiled_event_share(),
            Self::AuthoredRuntime => shares.authored_runtime_event_share(),
            Self::Live => shares.live_event_share(),
            Self::Session => shares.session_event_share(),
            Self::Internal => shares.internal_event_share(),
            Self::Release => shares.release_event_share(),
        }
    }

    /// The class's name, for a diagnostic a reader has to act on.
    ///
    /// Called off the audio thread only — a fault carries the [`ProducerClass`] itself and
    /// resolves the name when it is formatted, so the publication path makes no call the
    /// real-time scan would have to justify.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Compiled => "compiled",
            Self::AuthoredRuntime => "authored_runtime",
            Self::Live => "live",
            Self::Session => "session",
            Self::Internal => "internal",
            Self::Release => "release",
        }
    }
}

/// Why a publication could not be completed.
///
/// Every variant is a contract violation rather than a load condition: after ADR-0046's
/// clauses 1 to 6, no conforming producer can reach one. That is why they carry enough to
/// attribute the defect rather than merely to report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PublicationFault {
    /// A producer charged more than its own share in one quantum.
    ///
    /// Refused on the **share**, not on the quantum total. Clause 7 makes this a fault
    /// even when unusable slack leaves the total below `max_events_per_quantum`, because
    /// absorbing the over-emit would turn the declared share into a soft limit.
    #[error(
        "producer {class} charged {requested} events to quantum {quantum} against a share of \
         {share}"
    )]
    ShareOverrun {
        /// The offending class.
        class: ProducerClass,
        /// Which quantum of the open window.
        quantum: WindowRow,
        /// What the charge would have made the class's total.
        requested: EventCount,
        /// The class's entitlement.
        share: EventCount,
    },

    /// An event's own destination lies outside the open window.
    ///
    /// The destination is read from the event, never taken from the caller. A charge that
    /// named its own row could spread one real destination quantum across several ledger
    /// rows and overrun a share without ever tripping it, which would make the whole
    /// partition advisory.
    #[error("an event at {time} falls outside the open window of {quanta} quanta from {start}")]
    DestinationOutsideWindow {
        /// The event's declared time.
        time: SampleTime,
        /// The window's first sample, whose quantum is the window's first row.
        ///
        /// A [`SampleTime`] rather than a bare quantum index: the index is derived from it
        /// and a raw integer here would be one call away from being compared with a frame
        /// count or an event count.
        start: SampleTime,
        /// How many quanta the window holds.
        quanta: usize,
    },

    /// A renderer-internal emission was offered to the external batch.
    #[error("renderer-internal emissions may not enter the sealed external batch")]
    InternalIntoExternalBatch,

    /// The window asked for more quanta than the prepared store covers.
    #[error("a window of {quanta} exceeds the prepared maximum of {prepared}")]
    WindowExceedsPreparedStore {
        /// The window asked for.
        quanta: usize,
        /// What preparation covers.
        prepared: QuantumCount,
    },
}

/// The preallocated publication store.
///
/// Prepared once per stream and reused by every call. Its extent is exactly clause 1's
/// sealed-batch relation — `max_events_per_quantum * max_quanta_per_callback` — so the
/// store cannot be the thing that fails when a conforming producer fills every share of
/// every quantum the callback renders.
#[derive(Debug)]
#[must_use]
pub struct PublicationArbiter {
    /// Filled to its full extent at preparation and written **by index** thereafter.
    ///
    /// Not grown by `push`: the real-time rules forbid an operation that may allocate even
    /// where capacity usually happens to be available, and the purity scan enforces that
    /// over this file's hot path. `len` is what says how much of it is live.
    events: Vec<TimedEvent>,
    len: usize,
    /// One row per quantum, one counter per class, laid out `quantum * 6 + class`.
    ///
    /// `EventCount` rather than a bare integer throughout, not only where a value is
    /// returned: an occupancy is a number of events wherever it is stored, and a store
    /// typed loosely is what a returning accessor would have to launder.
    ledger: Vec<EventCount>,
    /// One total per quantum of the open window, so the per-quantum peak is observable.
    totals: Vec<EventCount>,
    /// The high-water mark per class over the stream, never reset by [`Self::open`].
    high_water: [EventCount; 6],
    /// The highest **external** total any one quantum reached over the stream.
    ///
    /// Not reconstructible from the six class marks: those peaks can fall in different
    /// quanta, so their sum overstates and their maximum understates.
    ///
    /// **External only, and named so rather than called the total.** `HOST-INV-021`'s
    /// total ledger also counts the renderer-internal arena, which ADR-0046 clause 2 keeps
    /// on the far side of the seal and which this slice does not build. Calling this the
    /// total would understate occupancy by exactly the internal share once that arena
    /// exists.
    high_water_external_total: EventCount,
    /// The first sample of the open window; its quantum is the window's first row.
    start: SampleTime,
    shares: [EventCount; 6],
    max_events_per_quantum: EventCount,
    max_quanta_per_callback: QuantumCount,
    open_quanta: usize,
}

impl PublicationArbiter {
    /// Prepare the store for one stream, allocating everything it will ever use.
    ///
    /// This is the only allocation. Every later call opens, fills and seals inside it.
    pub fn prepare(profile: &HostProfile) -> Result<Self, crate::profile::ProfileError> {
        let max_quanta_per_callback = profile.capabilities().max_quanta_per_callback()?;
        let max_events_per_quantum = profile.limits().events().max_events_per_quantum();

        // Clause 1's sealed-batch relation, and the reason `HostProfile::new` already
        // refuses an unnameable extent: by the time preparation runs, the product is known
        // to fit an `EventCount`, so this cannot be the allocation that surprises a caller.
        let extent = max_events_per_quantum
            .checked_over(max_quanta_per_callback)
            .ok_or(crate::profile::ProfileError::EventTotalUnrepresentable {
                total: "the sealed-batch extent for one callback",
            })?;
        let capacity =
            extent
                .as_usize()
                .ok_or(crate::profile::ProfileError::EventTotalUnrepresentable {
                    total: "the sealed-batch extent as an allocation",
                })?;
        let quanta = max_quanta_per_callback.as_usize().ok_or(
            crate::profile::ProfileError::EventTotalUnrepresentable {
                total: "max_quanta_per_callback as an allocation",
            },
        )?;

        let mut shares = [EventCount::NONE; 6];
        for class in ProducerClass::ALL {
            shares[class.index()] = class.share_of(profile);
        }

        Ok(Self {
            events: vec![fill_event(); capacity],
            len: 0,
            ledger: vec![EventCount::NONE; quanta.saturating_mul(ProducerClass::ALL.len())],
            totals: vec![EventCount::NONE; quanta],
            high_water: [EventCount::NONE; 6],
            high_water_external_total: EventCount::NONE,
            start: SampleTime::ZERO,
            shares,
            max_events_per_quantum,
            max_quanta_per_callback,
            open_quanta: 0,
        })
    }

    /// How many events the prepared store holds.
    pub fn capacity(&self) -> usize {
        self.events.len()
    }

    /// The highest occupancy one class has reached over this stream.
    ///
    /// Recorded on **every** stream rather than only after a fault, because a partition
    /// that is never approached and one that is repeatedly grazed look identical in a
    /// green run, and clause 1's numbers are exactly what Phase 3 has to measure.
    pub const fn high_water(&self, class: ProducerClass) -> EventCount {
        self.high_water[class.index()]
    }

    /// The highest **external** total any one quantum has reached over this stream.
    ///
    /// Not derivable from the six class marks: their peaks can fall in different quanta, so
    /// the sum overstates and the maximum understates. That is why it is stored rather than
    /// computed.
    ///
    /// **It does not satisfy `HOST-INV-021`'s total ledger**, in part or in whole. That
    /// ledger also counts the renderer-internal arena, which ADR-0046 clause 2 keeps on the
    /// far side of the seal and which this slice does not build. This is the external half
    /// alone, and the invariant is unmet until the other half exists.
    pub const fn high_water_external_total(&self) -> EventCount {
        self.high_water_external_total
    }

    /// The per-quantum cap this store was prepared against.
    pub const fn max_events_per_quantum(&self) -> EventCount {
        self.max_events_per_quantum
    }

    /// The largest window this store can open.
    pub const fn max_quanta_per_callback(&self) -> QuantumCount {
        self.max_quanta_per_callback
    }
}

/// The value the never-read part of the store carries.
///
/// The store is allocated at its full extent so the hot path can write by index rather
/// than grow, which means the tail holds something. It carries `PlanId::FILL` for the
/// same reason the renderer's event scratch does: a fill that resolved to a real plan
/// would be indistinguishable from an event if a length bug ever exposed it.
fn fill_event() -> TimedEvent {
    TimedEvent::new(
        crate::render::EventEnvelope::new(
            crate::time::StreamEpoch::from_raw(0),
            crate::time::SampleTime::ZERO,
            crate::time::TimeSource::Compiled,
        ),
        crate::render::EventPayload::SetParameter {
            slot: crate::plan::ParameterSlot::new(crate::plan::PlanId::FILL, 0),
            value: crate::quantities::ParameterValue::ZERO,
        },
    )
}

#[path = "publish/hot.rs"]
mod hot;

#[cfg(test)]
#[path = "tests/publication.rs"]
mod tests;

pub use hot::{Publication, SealedBatch};
