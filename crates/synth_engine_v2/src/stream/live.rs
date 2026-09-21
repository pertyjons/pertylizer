//! Live offers under the exclusive stream minter; no preparation or retirement.
use super::StreamControl;
use crate::{identity::NoteIdentity, quantities::HeldNoteCount, time::SampleTime};
impl StreamControl {
    /// The new stopped pair has already installed this anchor on audio.
    #[cfg(feature = "simulated-ingress")]
    pub(crate) fn synchronize_replacement_anchor(&mut self, anchor: crate::time::StreamAnchor) {
        self.anchor = anchor;
    }

    /// Offer a live note-on into a producer's ingress store.
    ///
    /// The off-thread half owns the minter, so the offer is made here rather than on the
    /// store: an identity minted from any other table is one the renderer refuses as
    /// foreign, and this is what makes that unrepresentable.
    ///
    /// `HOST-INV-009`'s three resources are acquired together or not at all, and the store
    /// names which one was exhausted.
    #[cfg(feature = "simulated-ingress")]
    pub fn offer_note_on(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: crate::time::SampleTime,
        note: crate::plan::NoteSlot,
        key: crate::quantities::KeyIdentity,
        velocity: crate::quantities::NoteVelocity,
    ) -> Result<NoteIdentity, crate::ingress::IngressRefused> {
        // The authoritative minter may not move while a candidate holds a snapshot of it,
        // and a mint is exactly a move. Refused rather than allowed to rewind: ADR-0050
        // clause 8 scopes activation to a stream whose note producers are compiled, so a
        // live producer beside a pending activation is out of scope, not supported.
        self.latch_store(store)?;
        // **Before the mint and before the hold**, because a slot that names another plan is
        // not a shortage to recover from — it is an offer that would play the wrong note.
        // The renderer does not re-check this: `note_target` applies the slot's index to
        // whichever plan is rendering, so nothing downstream can catch it.
        if note.plan() != self.plan.id() {
            return Err(crate::ingress::IngressRefused::ForeignSlot {
                slot: note.plan(),
                stream: self.plan.id(),
            });
        }
        let identity = store.offer_note_on(&mut self.minter, time, note, key, velocity)?;
        self.live_notes_open =
            HeldNoteCount::measured(self.live_notes_open.get().saturating_add(1));
        Ok(identity)
    }

    /// Offer a live parameter write into this stream's ingress store.
    ///
    /// It needs no minter, and it is here anyway: the stream latches one store, and an
    /// offer that bypassed that latch would let a second store fill a queue the stream never
    /// adopted — and overwrite the first store's cumulative counters in the report, because
    /// the drain mirrors those totals rather than accumulating them.
    #[cfg(feature = "simulated-ingress")]
    pub fn offer_parameter(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: crate::time::SampleTime,
        slot: crate::plan::ParameterSlot,
        value: crate::quantities::ParameterValue,
    ) -> Result<(), crate::ingress::IngressRefused> {
        self.latch_store(store)?;
        store.offer_parameter(time, slot, value)
    }

    /// Offer a validated controller write through this stream's one ingress store.
    pub fn offer_controller(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: SampleTime,
        change: crate::controller::ControllerChange,
    ) -> Result<(), crate::ingress::IngressRefused> {
        self.latch_store(store)?;
        store.offer_controller(time, change)
    }

    /// Offer per-note pressure or release velocity through this stream's identity owner.
    pub fn offer_expression(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: SampleTime,
        identity: NoteIdentity,
        expression: crate::controller::NoteExpression,
    ) -> Result<(), crate::ingress::IngressRefused> {
        self.latch_store(store)?;
        store.offer_expression(&mut self.minter, time, identity, expression)
    }

    /// Offer a per-note bend through this stream's minter (`SOUND-INV-021`).
    pub fn offer_bend(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: crate::time::SampleTime,
        identity: NoteIdentity,
        cents: crate::quantities::Cents,
    ) -> Result<(), crate::ingress::IngressRefused> {
        self.latch_store(store)?;
        store.offer_bend(&mut self.minter, time, identity, cents)
    }

    /// Adopt this stream's one ingress store, or refuse a second.
    ///
    /// **Two checks, and they answer different questions.** This stream may serve one store,
    /// because two would each hold the producer's whole entitlement; and this store may be
    /// served by one stream, because its entitlement and its identity range would otherwise
    /// come from different plans. The mark the second check sets lives on the **store**, so
    /// the audio-thread half verifies the same adoption instead of keeping a latch of its
    /// own that could disagree with this one.
    pub(crate) fn latch_store(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
    ) -> Result<(), crate::ingress::IngressRefused> {
        // **A candidate outstanding refuses every offer, and for two reasons that would
        // otherwise be checked in two places.** The authoritative minter may not move while a
        // candidate holds a snapshot of it, so a mint would have its generation rewound by
        // that candidate's promotion. And `plan_activation` refuses once a store is adopted,
        // which is what keeps a live producer out of ADR-0050 clause 8's scope — but that
        // check runs when the candidate is *built*, so adopting a store afterwards walks
        // straight past it. An independent review found that ordering: a parameter offer,
        // which mints nothing and so had no reason of its own to refuse, adopted a store
        // between a candidate's build and its offer.
        if self.live_candidates > 0 {
            return Err(crate::ingress::IngressRefused::CandidateOutstanding);
        }
        if let Some(latched) = self.ingress_store
            && latched != store.id()
        {
            return Err(crate::ingress::IngressRefused::ForeignStore {
                latched,
                offered: store.id(),
            });
        }
        // **Adopt first, record second.** Recording the id before the fallible adoption
        // poisoned the control: a refused foreign store left its id latched here, and the
        // control's own store was then rejected as foreign for the rest of the stream. A
        // refusal must leave the stream exactly as it found it, which is the same rule every
        // activation refusal follows. An independent review found the ordering.
        store.adopt(self.epoch)?;
        self.ingress_store = Some(store.id());
        Ok(())
    }

    /// Offer the release of a live note this stream's minter opened.
    ///
    /// Freeing the index is a move of the authoritative minter too, so it takes the same
    /// refusal while a candidate is outstanding.
    #[cfg(feature = "simulated-ingress")]
    pub fn offer_note_off(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        time: crate::time::SampleTime,
        identity: NoteIdentity,
    ) -> Result<(), crate::ingress::IngressRefused> {
        self.latch_store(store)?;
        store.offer_note_off(&mut self.minter, time, identity)?;
        self.live_notes_open =
            HeldNoteCount::measured(self.live_notes_open.get().saturating_sub(1));
        Ok(())
    }

    /// Admit one bounded release operation for this stream's immutable live plan.
    /// Deferred voice stealing is deliberately outside this group API.
    pub fn offer_release_group(
        &mut self,
        store: &mut crate::ingress::PerformanceIngress,
        at: SampleTime,
        identities: &[NoteIdentity],
        cause: crate::ingress::ReleaseCause,
    ) -> Result<(), crate::ingress::IngressRefused> {
        self.latch_store(store)?;
        store.offer_release_group(&mut self.minter, at, identities, cause)?;
        let count = u32::try_from(identities.len())
            .map_err(|_| crate::ingress::IngressRefused::ReleaseGroup)?;
        self.live_notes_open =
            HeldNoteCount::measured(self.live_notes_open.get().saturating_sub(count));
        Ok(())
    }
}
