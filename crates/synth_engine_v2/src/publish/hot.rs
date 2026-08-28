//! The arbiter's audio-thread path: open, charge, seal.
//!
//! Like `render/hot.rs` and `schedule/hot.rs`, this file may not allocate, lock, perform
//! I/O, log, or panic. Everything it writes into was allocated by
//! [`PublicationArbiter::prepare`], every index it uses is proven in bounds before the
//! write rather than assumed, and it grows nothing — `len` says how much of the prepared
//! store is live, because the real-time rules forbid an operation that may allocate even
//! where capacity usually happens to be available.
//!
//! Two properties are structural rather than remembered.
//!
//! **The destination is read from the event.** [`Publication::charge`] takes no quantum
//! from its caller. A charge that named its own ledger row could spread one real
//! destination across several rows and overrun a share without ever tripping it, which
//! would leave the partition advisory. The row is derived from the event's own
//! `SampleTime` against the window the pass opened.
//!
//! **The seal is a type.** [`Publication::seal`] consumes the writer and returns a
//! [`SealedBatch`], so writing after sealing does not compile. That is the same reason the
//! kernel registry keeps its constructor private: a guarantee the type system carries
//! cannot be lost to a later edit in a hurry.

use super::{ProducerClass, PublicationArbiter, PublicationFault, WindowRow};
use crate::quantities::EventCount;
use crate::render::{TimedEvent, TimedEvents};
use crate::time::SampleTime;

impl PublicationArbiter {
    /// Open a window of `quanta` quanta whose first row is `start`'s quantum.
    ///
    /// Clearing is bounded work over the prepared ledger and touches no allocation. The
    /// high-water marks deliberately survive: they describe the stream, not the call.
    pub fn open(
        &mut self,
        start: SampleTime,
        quanta: usize,
    ) -> Result<Publication<'_>, PublicationFault> {
        let prepared = self.max_quanta_per_callback.as_usize().unwrap_or(0);
        if quanta > prepared {
            return Err(PublicationFault::WindowExceedsPreparedStore {
                quanta,
                prepared: self.max_quanta_per_callback,
            });
        }

        self.len = 0;
        self.start = start;

        // The **whole** prepared ledger, not just the rows this window uses. Clearing only
        // the active rows leaves a larger previous pass's occupancy readable beyond a
        // smaller window, which an observer cannot distinguish from real work — the
        // independent review found exactly that. The extra writes are bounded by the
        // prepared extent and touch no allocation.
        let mut index = 0;
        while index < self.ledger.len() {
            if let Some(slot) = self.ledger.get_mut(index) {
                *slot = EventCount::NONE;
            }
            index += 1;
        }
        let mut quantum = 0;
        while quantum < self.totals.len() {
            if let Some(slot) = self.totals.get_mut(quantum) {
                *slot = EventCount::NONE;
            }
            quantum += 1;
        }
        self.open_quanta = quanta;
        Ok(Publication { arbiter: self })
    }

    /// One class's occupancy in one row of the **open** window.
    ///
    /// A row outside the open window reads `NONE` rather than the prepared store's
    /// contents. `open` clears the whole store, so the two agree today; bounding here as
    /// well means they still agree if a later revision clears only what it uses.
    fn spent_in(&self, quantum: WindowRow, class: ProducerClass) -> EventCount {
        let quantum = quantum.offset();
        if quantum >= self.open_quanta {
            return EventCount::NONE;
        }
        let row = quantum.saturating_mul(ProducerClass::ALL.len());
        self.ledger
            .get(row.saturating_add(class.index()))
            .copied()
            .unwrap_or(EventCount::NONE)
    }

    /// One row of the open window's external total, under the same bound.
    fn external_total_in(&self, quantum: WindowRow) -> EventCount {
        let quantum = quantum.offset();
        if quantum >= self.open_quanta {
            return EventCount::NONE;
        }
        self.totals
            .get(quantum)
            .copied()
            .unwrap_or(EventCount::NONE)
    }

    /// Which row of the open window an event's own destination falls in.
    fn row_of(&self, event: TimedEvent) -> Option<WindowRow> {
        self.row_of_time(event.envelope().time())
    }

    /// The same, for a charge whose destination is a time rather than an event.
    fn row_of_time(&self, time: SampleTime) -> Option<WindowRow> {
        let offset = time
            .quantum_index()
            .checked_sub(self.start.quantum_index())?;
        let offset = usize::try_from(offset).ok()?;
        if offset < self.open_quanta {
            Some(WindowRow::new(offset))
        } else {
            None
        }
    }
}

/// An open window, being filled.
///
/// Held by the one arbiter for the duration of one publication pass. It cannot outlive its
/// store, and it cannot be observed as a batch until it is sealed.
#[derive(Debug)]
#[must_use]
pub struct Publication<'a> {
    arbiter: &'a mut PublicationArbiter,
}

impl<'a> Publication<'a> {
    /// Charge one event to one class, in the quantum the event itself names.
    ///
    /// Refuses on the **class's own share**, never on the quantum total. ADR-0046 clause 7
    /// makes an over-emit a fault "even if unusable slack means the quantum total remains
    /// below `max_events_per_quantum`", so a ledger that checked the total would silently
    /// turn a declared share into a soft limit. The store's own capacity cannot be the
    /// binding constraint first: the shares sum to at most the cap, and the store covers
    /// the cap over every quantum of the window.
    pub fn charge(
        &mut self,
        class: ProducerClass,
        event: TimedEvent,
    ) -> Result<(), PublicationFault> {
        // ADR-0046 clause 2 keeps renderer-internal emissions on the far side of the seal,
        // in their own arena, where they "never reopen or mutate the external batch".
        // Accepting one here would put it in the batch the renderer is handed as immutable.
        if class == ProducerClass::Internal {
            return Err(PublicationFault::InternalIntoExternalBatch);
        }

        let Some(quantum) = self.arbiter.row_of(event) else {
            return Err(PublicationFault::DestinationOutsideWindow {
                time: event.envelope().time(),
                start: self.arbiter.start,
                quanta: self.arbiter.open_quanta,
            });
        };

        let row = quantum.offset().saturating_mul(ProducerClass::ALL.len());
        let index = row.saturating_add(class.index());
        let (Some(spent), Some(total)) = (
            self.arbiter.ledger.get(index).copied(),
            self.arbiter.totals.get(quantum.offset()).copied(),
        ) else {
            // Unreachable while `open` bounds the window by the prepared ledger, and stated
            // as a refusal rather than an index because the audio thread cannot report a
            // panic.
            return Err(PublicationFault::DestinationOutsideWindow {
                time: event.envelope().time(),
                start: self.arbiter.start,
                quanta: self.arbiter.open_quanta,
            });
        };

        let share = self.arbiter.shares[class.index()];
        let one = EventCount::measured(1);
        let Some(requested) = spent.checked_add(one) else {
            return Err(PublicationFault::ShareOverrun {
                class,
                quantum,
                requested: spent,
                share,
            });
        };
        if requested > share {
            return Err(PublicationFault::ShareOverrun {
                class,
                quantum,
                requested,
                share,
            });
        }

        // Written by index into storage preparation already sized, never grown.
        let Some(slot) = self.arbiter.events.get_mut(self.arbiter.len) else {
            return Err(PublicationFault::ShareOverrun {
                class,
                quantum,
                requested,
                share,
            });
        };
        *slot = event;
        self.arbiter.len = self.arbiter.len.saturating_add(1);

        if let Some(slot) = self.arbiter.ledger.get_mut(index) {
            *slot = requested;
        }
        let quantum_total = total.checked_add(one).unwrap_or(total);
        if let Some(slot) = self.arbiter.totals.get_mut(quantum.offset()) {
            *slot = quantum_total;
        }

        if requested > self.arbiter.high_water[class.index()] {
            self.arbiter.high_water[class.index()] = requested;
        }
        if quantum_total > self.arbiter.high_water_external_total {
            self.arbiter.high_water_external_total = quantum_total;
        }
        Ok(())
    }

    /// Charge one **operation** to one class, in the quantum `at` falls in.
    ///
    /// The same accounting as [`Self::charge`] and no batch entry: it spends a unit of the
    /// class's share for work the renderer performs itself rather than for an event the
    /// batch carries.
    ///
    /// Its one caller is ADR-0050 clause 5's boundary mass release, which clause 6 of
    /// ADR-0046 requires to be "one operation ... charged to the session share, never
    /// expanded into one event per voice". Expanding it would make a seek's cost depend on
    /// how many notes happened to be sounding, which is exactly the unbounded quantity the
    /// bounded operation exists to replace; charging nothing at all would leave the share's
    /// occupancy and its high-water mark blind to it.
    ///
    /// The destination is taken from `at` rather than from a row the caller names, for the
    /// reason [`Self::charge`] reads it from the event: a charge that chose its own row
    /// could overrun a share without ever tripping it.
    pub fn charge_operation(
        &mut self,
        class: ProducerClass,
        at: SampleTime,
    ) -> Result<(), PublicationFault> {
        if class == ProducerClass::Internal {
            return Err(PublicationFault::InternalIntoExternalBatch);
        }

        let Some(quantum) = self.arbiter.row_of_time(at) else {
            return Err(PublicationFault::DestinationOutsideWindow {
                time: at,
                start: self.arbiter.start,
                quanta: self.arbiter.open_quanta,
            });
        };

        let row = quantum.offset().saturating_mul(ProducerClass::ALL.len());
        let index = row.saturating_add(class.index());
        let (Some(spent), Some(total)) = (
            self.arbiter.ledger.get(index).copied(),
            self.arbiter.totals.get(quantum.offset()).copied(),
        ) else {
            return Err(PublicationFault::DestinationOutsideWindow {
                time: at,
                start: self.arbiter.start,
                quanta: self.arbiter.open_quanta,
            });
        };

        let share = self.arbiter.shares[class.index()];
        let one = EventCount::measured(1);
        let Some(requested) = spent.checked_add(one) else {
            return Err(PublicationFault::ShareOverrun {
                class,
                quantum,
                requested: spent,
                share,
            });
        };
        if requested > share {
            return Err(PublicationFault::ShareOverrun {
                class,
                quantum,
                requested,
                share,
            });
        }

        if let Some(slot) = self.arbiter.ledger.get_mut(index) {
            *slot = requested;
        }
        let quantum_total = total.checked_add(one).unwrap_or(total);
        if let Some(slot) = self.arbiter.totals.get_mut(quantum.offset()) {
            *slot = quantum_total;
        }
        if requested > self.arbiter.high_water[class.index()] {
            self.arbiter.high_water[class.index()] = requested;
        }
        if quantum_total > self.arbiter.high_water_external_total {
            self.arbiter.high_water_external_total = quantum_total;
        }
        Ok(())
    }

    /// How much of one class's share one quantum of the window has spent so far.
    ///
    /// Bounded by the **open** window rather than by the prepared store, so a query past
    /// the window reads nothing rather than whatever a larger previous pass left there.
    ///
    /// **Charges, not batch entries** — see [`Publication::charge_operation`]: a bounded
    /// operation spends the share without adding an entry, so this can exceed what
    /// [`SealedBatch::len`] reports.
    pub fn spent(&self, quantum: WindowRow, class: ProducerClass) -> EventCount {
        self.arbiter.spent_in(quantum, class)
    }

    /// Seal the window. The batch is immutable from here.
    pub fn seal(self) -> SealedBatch<'a> {
        SealedBatch {
            arbiter: self.arbiter,
        }
    }
}

/// A sealed batch: exactly what one renderer call is presented with.
///
/// Read-only by construction. Producing one consumes the [`Publication`], so no path
/// exists that writes into a batch after the renderer has been handed it — which is what
/// "sealed" has to mean if clause 2's ordering guarantee is to survive a later edit.
#[derive(Debug)]
#[must_use]
pub struct SealedBatch<'a> {
    arbiter: &'a PublicationArbiter,
}

impl SealedBatch<'_> {
    /// The events, in the order they were charged.
    pub fn events(&self) -> TimedEvents<'_> {
        TimedEvents::new(self.arbiter.events.get(..self.arbiter.len).unwrap_or(&[]))
    }

    /// How many events the batch holds.
    ///
    /// **Not the same as what its quanta spent.** A bounded operation charges a share without
    /// adding an entry, so [`Self::spent`] and [`Self::external_total`] can exceed this — see
    /// [`Publication::charge_operation`].
    pub const fn len(&self) -> usize {
        self.arbiter.len
    }

    /// Whether the batch is empty.
    pub const fn is_empty(&self) -> bool {
        self.arbiter.len == 0
    }

    /// How much of one class's share one quantum of the window spent.
    ///
    /// Charges rather than entries: an operation spends without adding one.
    pub fn spent(&self, quantum: WindowRow, class: ProducerClass) -> EventCount {
        self.arbiter.spent_in(quantum, class)
    }

    /// How much of one quantum's **external** share was spent.
    ///
    /// External because the renderer-internal arena is not part of this batch; see
    /// [`PublicationArbiter::high_water_external_total`]. Charges rather than entries, for
    /// the reason [`Self::len`] gives.
    pub fn external_total(&self, quantum: WindowRow) -> EventCount {
        self.arbiter.external_total_in(quantum)
    }
}
