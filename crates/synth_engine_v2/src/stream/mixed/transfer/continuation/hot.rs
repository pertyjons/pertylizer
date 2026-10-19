//! Single-quantum validation and actual-owner rebinding; no reconstruction or drop.
use super::{
    MixedContinuationOutcome, MixedContinuationPacket, MixedContinuationRefusal, MixedOfferSlot,
    MixedOneShotAudio, MixedTransferAudio, Stage,
};
use crate::{
    plan::{PlanId, binding::FixedPlanBindings},
    quantities::PreparedBytes,
    stream::mixed::{
        MixedCompiledEvent, MixedCompiledPayload,
        transfer::{MixedResetSeal, OPEN_RESET_SEAL_PHASE},
    },
    time::SampleTime,
};
use std::sync::{Arc, atomic::Ordering};

fn accepts_compiled(bindings: FixedPlanBindings, event: MixedCompiledEvent) -> bool {
    match event.payload {
        MixedCompiledPayload::On { slot, .. } => bindings.accepts_note(slot),
        MixedCompiledPayload::Restore(restore) => bindings.accepts_parameter(restore.slot()),
        MixedCompiledPayload::Off(_)
        | MixedCompiledPayload::Expression(_, _)
        | MixedCompiledPayload::Bend(_, _) => true,
    }
}
fn apply_compiled(bindings: FixedPlanBindings, event: &mut MixedCompiledEvent) {
    match &mut event.payload {
        MixedCompiledPayload::On { slot, .. } => *slot = bindings.note(*slot),
        MixedCompiledPayload::Restore(restore) => restore.apply_plan_bindings(bindings),
        MixedCompiledPayload::Off(_)
        | MixedCompiledPayload::Expression(_, _)
        | MixedCompiledPayload::Bend(_, _) => {}
    }
}

impl MixedOneShotAudio {
    /// This runs before adoption, Stop, later activation and command service at B.
    pub(in crate::stream::mixed) fn service_continuation(&mut self, clock: SampleTime) {
        let Some(mut packet) = self.continuation.take() else {
            return;
        };
        if packet.stage != Stage::Ready || clock < packet.boundary {
            self.continuation = Some(packet);
            return;
        }
        if clock > packet.boundary {
            packet.stage = Stage::Finished(MixedContinuationOutcome::Refused(
                MixedContinuationRefusal::BoundaryPassed,
            ));
            self.continuation = Some(packet);
            return;
        }
        match self.continuation_preflight(&packet) {
            Err(error) => packet.stage = Stage::Finished(MixedContinuationOutcome::Refused(error)),
            Ok(()) => {
                let bindings = packet.bindings;
                self.audio.compiled_minter.0.apply_plan_bindings(bindings);
                self.audio.live_minter.0.apply_plan_bindings(bindings);
                self.test_ingress.apply_plan_bindings(bindings);
                self.audio.note = bindings.note(self.audio.note);
                for event in &mut self.events {
                    apply_compiled(bindings, event);
                }
                for event in &mut self.capsule.events {
                    apply_compiled(bindings, event);
                }
                self.capsule.plan = bindings.new_plan();
                if let Some(program) = self.loop_program.as_mut() {
                    for event in &mut program.events {
                        apply_compiled(bindings, event);
                    }
                }
                #[cfg(test)]
                if let Some(event) = self.test_live.as_mut() {
                    *event = bindings.event(*event);
                }
                #[cfg(all(test, feature = "simulated-ingress"))]
                for (event, _) in self.test_ingress_inflight.iter_mut().flatten() {
                    *event = bindings.event(*event);
                }
                self.audio
                    .renderer
                    .install_mixed_continuation(&mut packet.prepared, &packet.proof);
                core::mem::swap(&mut self.audio.partition, &mut packet.audio_partition);
                // compiled_ended and stop_ended were synchronously redeemed at their
                // original boundaries. They retain explicit historical plan labels;
                // no prospective settlement consults those labels after redemption.
                packet.stage = Stage::Installed;
            }
        }
        self.continuation = Some(packet);
    }

    fn continuation_preflight(
        &self,
        packet: &MixedContinuationPacket,
    ) -> Result<(), MixedContinuationRefusal> {
        if self.fault.is_some() || self.ingress_halt.load(Ordering::Acquire) {
            return Err(MixedContinuationRefusal::Faulted);
        }
        if !self.adopted {
            return Err(MixedContinuationRefusal::NotAdopted);
        }
        if self
            .activation_custody
            .as_ref()
            .is_some_and(|custody| custody.load(Ordering::Acquire) != 0)
            || !matches!(self.offer_slot, MixedOfferSlot::Empty)
            || self
                .reset_seal
                .as_ref()
                .is_some_and(|seal| !seal.continuation_is_open())
        {
            return Err(MixedContinuationRefusal::Busy);
        }
        if self.transfer_binding != Some(packet.binding)
            || packet.expected != self.in_force
            || packet.proof.old != self.audio.renderer.plan().id()
            || packet.proof.new != packet.next.plan.id()
            || packet.bindings.old_plan() != packet.proof.old
            || packet.bindings.new_plan() != packet.proof.new
            || packet.binding.owner.epoch != self.audio.renderer.epoch()
            || packet.binding.owner.table != self.audio.renderer.table_id()
            || self.audio.compiled_minter.id() != packet.binding.owner.table
            || self.audio.live_minter.id() != packet.binding.owner.table
            || !Arc::ptr_eq(&packet.audio_partition, &packet.next.partition)
            || !self
                .audio
                .renderer
                .continuation_partition_matches(&self.audio.partition)
            || !packet
                .prepared
                .continuation_partition_matches(&packet.next.partition)
        {
            return Err(MixedContinuationRefusal::Binding);
        }
        let bindings = packet.bindings;
        if !bindings.accepts_note(self.audio.note)
            || bindings.note(self.audio.note) != packet.next.live_slot
            || !self.audio.compiled_minter.0.accepts_plan_bindings(bindings)
            || !self.audio.live_minter.0.accepts_plan_bindings(bindings)
            || !self.test_ingress.accepts_plan_bindings(bindings)
            || self
                .events
                .iter()
                .any(|event| !accepts_compiled(bindings, *event))
            || self.capsule.plan != bindings.old_plan()
            || self
                .capsule
                .events
                .iter()
                .any(|event| !accepts_compiled(bindings, *event))
            || self.loop_program.as_ref().is_some_and(|program| {
                program
                    .events
                    .iter()
                    .any(|event| !accepts_compiled(bindings, *event))
            })
        {
            return Err(MixedContinuationRefusal::Storage);
        }
        #[cfg(test)]
        if self
            .test_live
            .is_some_and(|event| !bindings.accepts_event(event))
        {
            return Err(MixedContinuationRefusal::Storage);
        }
        #[cfg(all(test, feature = "simulated-ingress"))]
        for entry in &self.test_ingress_inflight {
            if let Some((event, _)) = entry
                && !bindings.accepts_event(*event)
            {
                return Err(MixedContinuationRefusal::Storage);
            }
        }
        self.audio
            .renderer
            .mixed_continuation_preflight(&packet.prepared, &packet.proof)
            .map_err(|_| MixedContinuationRefusal::Storage)
    }

    /// Called after the complete outer callback has either rendered or faulted.
    pub(in crate::stream::mixed) fn finish_continuation_render(&mut self, succeeded: bool) {
        if let Some(packet) = self.continuation.as_mut() {
            if packet.stage == Stage::Installed {
                packet.stage = Stage::Finished(if succeeded {
                    MixedContinuationOutcome::InstalledRendered {
                        plan: packet.proof.new,
                        boundary: packet.boundary,
                    }
                } else {
                    // This callback is terminal; let the existing control-side
                    // Remote fault stop new source admission without another render.
                    self.ingress_halt.store(true, Ordering::Release);
                    MixedContinuationOutcome::InstalledFaulted {
                        plan: packet.proof.new,
                        boundary: packet.boundary,
                    }
                });
            } else if !succeeded && self.fault.is_some() && packet.stage == Stage::Ready {
                packet.stage = Stage::Finished(MixedContinuationOutcome::Refused(
                    MixedContinuationRefusal::Faulted,
                ));
            }
        }
    }
}

impl MixedResetSeal {
    pub(in crate::stream::mixed) fn continuation_is_open(&self) -> bool {
        self.phase.load(Ordering::Acquire) == OPEN_RESET_SEAL_PHASE
    }
}
impl MixedOneShotAudio {
    /// Admit one owning packet. Sound and raw input continue if this offer refuses.
    pub(in crate::stream::mixed) fn offer_bound_continuation(
        &mut self,
        packet: Box<MixedContinuationPacket>,
    ) -> Result<(), (Box<MixedContinuationPacket>, MixedContinuationRefusal)> {
        if self
            .reset_seal
            .as_ref()
            .is_some_and(|seal| !seal.continuation_is_open())
            || self
                .activation_custody
                .as_ref()
                .is_some_and(|custody| custody.load(Ordering::Acquire) != 0)
            || self.continuation.is_some()
            || !matches!(self.offer_slot, MixedOfferSlot::Empty)
        {
            return Err((packet, MixedContinuationRefusal::Busy));
        }
        if self.transfer_binding != Some(packet.binding)
            || packet.proof.old != self.audio.renderer.plan().id()
            || packet.stage != Stage::Ready
            || packet.expected != self.in_force
        {
            return Err((packet, MixedContinuationRefusal::Binding));
        }
        if self.audio.renderer.clock() > packet.boundary {
            return Err((packet, MixedContinuationRefusal::BoundaryPassed));
        }
        if !self.adopted {
            return Err((packet, MixedContinuationRefusal::NotAdopted));
        }
        if self.fault.is_some() {
            return Err((packet, MixedContinuationRefusal::Faulted));
        }
        self.continuation = Some(packet);
        Ok(())
    }

    /// Withdraw an uninstalled offer without another callback. A completed
    /// installation returns its actual outcome; the host must collect it cold.
    pub(in crate::stream::mixed) fn withdraw_bound_continuation(
        &mut self,
    ) -> Option<Box<MixedContinuationPacket>> {
        if let Some(packet) = self.continuation.as_mut()
            && packet.stage == Stage::Ready
        {
            packet.stage = Stage::Finished(MixedContinuationOutcome::Refused(
                MixedContinuationRefusal::Cancelled,
            ));
        }
        self.take_bound_continuation()
    }

    pub(in crate::stream::mixed) fn take_bound_continuation(
        &mut self,
    ) -> Option<Box<MixedContinuationPacket>> {
        if self
            .continuation
            .as_ref()
            .is_some_and(|packet| packet.outcome().is_some())
        {
            self.continuation.take()
        } else {
            None
        }
    }
}

impl MixedTransferAudio {
    /// Publish one owning in-place geometry change, or return it unchanged.
    pub fn offer_continuation(
        &mut self,
        packet: Box<MixedContinuationPacket>,
    ) -> Result<(), (Box<MixedContinuationPacket>, MixedContinuationRefusal)> {
        self.inner.offer_bound_continuation(packet)
    }
    /// Withdraw an uninstalled offer, or return a completed installation's actual outcome.
    pub fn withdraw_continuation(&mut self) -> Option<Box<MixedContinuationPacket>> {
        self.inner.withdraw_bound_continuation()
    }
    /// Move a completed owner back to control; unresolved packets remain retained.
    pub fn take_retired_continuation(&mut self) -> Option<Box<MixedContinuationPacket>> {
        self.inner.take_bound_continuation()
    }
}

impl MixedContinuationPacket {
    pub const fn plan_id(&self) -> PlanId {
        self.proof.new
    }
    pub const fn boundary(&self) -> SampleTime {
        self.boundary
    }
    pub const fn metadata_bytes(&self) -> PreparedBytes {
        self.bytes
    }
    pub const fn outcome(&self) -> Option<MixedContinuationOutcome> {
        match self.stage {
            Stage::Finished(outcome) => Some(outcome),
            Stage::Ready | Stage::Installed => None,
        }
    }
}
