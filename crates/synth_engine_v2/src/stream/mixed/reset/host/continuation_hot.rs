//! In-place continuation never overlaps whole-owner reset custody.
use super::{MixedBridgeResetAudio, MixedResetSlot};
use crate::stream::mixed::transfer::continuation::{
    MixedContinuationPacket, MixedContinuationRefusal,
};

impl<C, R> MixedBridgeResetAudio<C, R> {
    /// Actual active renderer identity, including an installed but uncollected edit.
    pub fn plan_id(&self) -> crate::plan::PlanId {
        self.inner.pair.active().audio.renderer.plan().id()
    }

    /// First unrendered engine sample; this is not a physical output timestamp.
    pub fn clock(&self) -> crate::time::SampleTime {
        self.inner.pair.active().audio.renderer.clock()
    }

    /// The actual compiled Stop cursor. Running and unmapped stopped cursors return None.
    pub fn stopped_position(&self) -> Option<crate::time::PlanPosition> {
        match self.inner.pair.active().transport {
            crate::stream::mixed::MixedCompiledTransport::Stopped { position, .. } => position,
            crate::stream::mixed::MixedCompiledTransport::Running => None,
        }
    }

    /// Preserve the active source owner, or return the complete candidate unchanged.
    pub fn offer_continuation(
        &mut self,
        packet: Box<MixedContinuationPacket>,
    ) -> Result<(), (Box<MixedContinuationPacket>, MixedContinuationRefusal)> {
        if self.inner.fault().is_some() {
            return Err((packet, MixedContinuationRefusal::Faulted));
        }
        if !matches!(self.inner.pair.secondary, MixedResetSlot::Empty) {
            return Err((packet, MixedContinuationRefusal::Busy));
        }
        self.inner.pair.active.offer_bound_continuation(packet)
    }
    /// The host moves the completed owner back to cold control.
    pub fn take_retired_continuation(&mut self) -> Option<Box<MixedContinuationPacket>> {
        self.inner.pair.active.take_bound_continuation()
    }
    /// Withdraw an uninstalled offer without needing a final callback.
    pub fn withdraw_continuation(&mut self) -> Option<Box<MixedContinuationPacket>> {
        self.inner.pair.active.withdraw_bound_continuation()
    }
}
