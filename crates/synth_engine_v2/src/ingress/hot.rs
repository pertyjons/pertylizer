//! The ingress store's audio-thread path: draining due entries into the publication.
//!
//! Like the other `hot.rs` files, this one may not allocate, lock, perform I/O, log, or
//! panic. It reads storage `prepare` already sized and moves one index.
//!
//! # Why stopping at the head is correct, and why it was not always
//!
//! An earlier version walked the whole occupied span and compacted what it did not charge,
//! because a producer offering a later stamp before an earlier one would otherwise have a
//! due entry held behind a future one. That is a real hazard, and it is now closed at the
//! **offer** instead: `PerformanceIngress` refuses a stamp behind one it has already
//! accepted, because non-monotone offers break identity reuse in a way no drain can repair
//! — a note-on reusing a freed index with an earlier stamp starts before the release it
//! followed.
//!
//! With stamps non-decreasing, reachability is monotone too: `row_of` maps a late stamp to
//! the window's first row and refuses only stamps past its end, so once an entry is out of
//! reach every entry behind it is as well. Stopping at the first unreachable entry is
//! therefore complete, and the drain moves one index rather than rewriting the ring.
//!
//! # Late is due, not skipped
//!
//! An entry whose destination is already behind the window start is **late**, and
//! `Publication::charge` gives it row zero under ADR-0043's preserving late clamp. Treating
//! it as not-yet-due instead is the defect that leaves it in the queue forever, because the
//! window only ever moves further away from it.

use super::PerformanceIngress;
use crate::publish::{ProducerClass, Publication, PublicationFault};
use crate::time::SampleTime;

impl PerformanceIngress {
    /// Charge every entry whose destination this call can reach, and keep the rest.
    ///
    /// `clock` is the render clock this call starts at. The drain records it even when it
    /// charges nothing, because the producing half does not own the render clock and this is
    /// the only point at which it can learn one — and it is what
    /// `PerformanceIngress::admit`'s forward-horizon check measures against.
    ///
    /// **Each entry is charged to the class ADR-0046 clause 6 assigns it**, not to one class
    /// for the whole pass: a note-on spends the live share and its matching release redeems
    /// a hold into the guaranteed-release share. The pass's charges are therefore
    /// interleaved across two classes by design, and ADR-0023's producer-granular order is
    /// what makes that safe — the queue's own order is preserved, so a release can never
    /// precede the note-on it discharges.
    pub(crate) fn drain_into(
        &mut self,
        publication: &mut Publication<'_>,
        diagnostics: &mut crate::diagnostics::DiagnosticsReport,
        clock: SampleTime,
    ) -> Result<(), PublicationFault> {
        self.clock = clock;
        // `HOST-INV-009` requires the boundary's refusal counts to reach the structured
        // report, and the drain is the only place the two halves meet: they happened on the
        // producing half, before acceptance, where the report cannot be reached. Mirrored
        // first so a fault below still leaves them published — a stream that ended on a
        // contract violation is exactly when someone reads them.
        let counters = self.counters;
        diagnostics.mirror_ingress_boundary(
            counters.dropped_slot(),
            counters.dropped_hold(),
            counters.dropped_identity(),
            counters.orphan_releases(),
            counters.beyond_horizon(),
        );

        let capacity = self.entries.len();
        if capacity == 0 {
            return Ok(());
        }

        while self.len > 0 {
            let Some(entry) = self.entries.get(self.tail).copied().flatten() else {
                // Unreachable while `len` counts written slots. Stated as a skip rather
                // than an index, because the audio thread has no way to report a panic.
                self.tail = next(self.tail, capacity);
                self.len = self.len.saturating_sub(1);
                continue;
            };
            if !publication.reaches(entry.event) {
                break;
            }
            let class = if entry.redeems_hold {
                ProducerClass::Release
            } else {
                ProducerClass::Live
            };
            // **Charged before the entry is consumed.** A fault leaves the entry in place,
            // so the terminal response ends the stream without also silently losing the
            // event that caused it.
            publication.charge(class, entry.event)?;
            if let Some(slot) = self.entries.get_mut(self.tail) {
                *slot = None;
            }
            self.tail = next(self.tail, capacity);
            self.len = self.len.saturating_sub(1);
        }

        // **No hold is discharged here.** The reservation keeps a queue slot free for a
        // release, and `offer_note_off` spends it the moment the release takes that slot.
        // Discharging again at publication would decrement it twice, which lets the queue
        // admit one entry too many for every note that has been in flight.
        Ok(())
    }
}

/// The next ring index, wrapping at the capacity.
const fn next(index: usize, capacity: usize) -> usize {
    let advanced = index.saturating_add(1);
    if advanced >= capacity { 0 } else { advanced }
}
