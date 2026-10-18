//! Nonblocking publication checks around the reset owner.
use super::{FrameCount, MixedBridgeResetAudio, MixedResetSlot};
use crate::stream::{
    MixedPublication, MixedPublicationFault, MixedPublicationGate, MixedPublicationRefusal,
};

impl<C, R> MixedBridgeResetAudio<C, R> {
    /// Accept a strict source-publication proof for the active arm or the exact
    /// sealed transition. Raw coverage belongs to the host's independent raw owner.
    ///
    /// # Errors
    /// A foreign or regressing proof, leaving the previous proof unchanged.
    pub fn offer_publication(
        &self,
        gate: &mut MixedPublicationGate,
        publication: MixedPublication,
        raw: crate::host::mixed_input::MixedRawPublication,
    ) -> Result<(), MixedPublicationRefusal> {
        let active = self.inner.pair.active();
        gate.offer(
            publication,
            raw,
            active.transfer_binding,
            active.audio.renderer.clock(),
        )
    }

    /// Check actual carry and quantum demand of an already shape/profile-validated
    /// callback before either sound or raw renders. Shape refusal precedes this check.
    /// The host silences and halts both halves on a strict preflight failure.
    ///
    /// # Errors
    /// A missing active/transition proof, insufficient coverage or exhausted clock.
    pub fn check_publication(
        &self,
        gate: &mut MixedPublicationGate,
        frames: FrameCount,
    ) -> Result<(), MixedPublicationFault> {
        let active = self.inner.pair.active();
        let crossing = match &self.inner.pair.secondary {
            MixedResetSlot::Pending { endpoint, at, .. } => Some((endpoint.port.binding, *at)),
            _ => None,
        };
        gate.check(
            active.transfer_binding,
            &active.audio.renderer,
            frames,
            crossing,
        )
    }
}
