//! The reset pair's callback: the boundary decision, the `Q`-frame cross-fade and the
//! fading owner's discarded quantum. It works in the pair's prepared scratch and moves
//! owner boxes and their endpoints between the active place and the secondary slot. It
//! does not allocate, lock, perform I/O, log, panic or drop an owner or an endpoint.

use std::sync::atomic::Ordering;

#[cfg(all(test, feature = "simulated-ingress"))]
use ringbuf::traits::Observer;

#[cfg(feature = "mixed-transfer-rehearsal")]
use super::super::transfer::{MixedResetSealDecision, MixedResetSealState};
#[cfg(feature = "mixed-transfer-rehearsal")]
use super::super::transfer::{
    MixedTransferAudioPort, MixedTransferCommand, MixedTransferConsumer, MixedTransferProducer,
    MixedTransferResult,
};
use super::super::{
    MixedAudioQueues, MixedCompiledTransport, MixedIngressCommand, MixedIngressResult,
    MixedOfferSlot, MixedOneShotAudio,
};
#[cfg(feature = "mixed-transfer-rehearsal")]
use super::MixedResetEndpoint;
use super::{
    MixedResetPair, MixedResetReceipt, MixedResetRefused, MixedResetRenderError, MixedResetSlot,
};
use crate::{
    quantities::{ChannelLayout, EventCount, HeldNoteCount},
    render::AudioBlockMut,
    time::{FrameCount, QUANTUM_FRAMES, SampleTime, StreamAnchor},
};

/// The fading owner's command source: none. From the reset boundary the old owner
/// serves no command, so nothing waiting for it is applied after `B`.
struct MixedResetFadeQueues;

impl MixedAudioQueues for MixedResetFadeQueues {
    const TRANSFER_RULES: bool = false;

    fn bound(&self, _audio: &MixedOneShotAudio) -> bool {
        true
    }

    fn pop_command(&mut self, _audio: &mut MixedOneShotAudio) -> Option<MixedIngressCommand> {
        None
    }

    fn push_result(
        &mut self,
        _audio: &mut MixedOneShotAudio,
        result: MixedIngressResult,
    ) -> Option<MixedIngressResult> {
        Some(result)
    }
}

/// A host-armed owner's endpoint serves through the port its own arm returned, whose
/// binding check runs before any queue use, exactly as a lent port does alone.
#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> MixedAudioQueues for MixedResetEndpoint<C, R>
where
    C: MixedTransferConsumer<MixedTransferCommand>,
    R: MixedTransferProducer<MixedTransferResult>,
{
    const TRANSFER_RULES: bool = <MixedTransferAudioPort<C, R> as MixedAudioQueues>::TRANSFER_RULES;

    fn bound(&self, audio: &MixedOneShotAudio) -> bool {
        self.port.bound(audio)
    }

    fn pop_command(&mut self, audio: &mut MixedOneShotAudio) -> Option<MixedIngressCommand> {
        self.port.pop_command(audio)
    }

    fn push_result(
        &mut self,
        audio: &mut MixedOneShotAudio,
        result: MixedIngressResult,
    ) -> Option<MixedIngressResult> {
        self.port.push_result(audio, result)
    }
}

/// Why `successor` cannot take over from `old` at `at`, checked at the offer and again
/// when `at` opens. Both owners are only read.
///
/// The old owner must be stopped with nothing left to apply after `B`: no scheduled
/// Stop, an empty offer slot, no unserved command, queued entry, parked Panic, owed
/// compiled cut or unpushed result. The successor must be freshly armed for exactly `B`,
/// under another epoch and table, with the old owner's output shape. A seal a due Panic
/// cancelled is reported right after a fault, so the cancelling Panic the old owner
/// still holds is not reported as pending ingress.
pub(super) fn reset_refusal(
    old: &MixedOneShotAudio,
    successor: &MixedOneShotAudio,
    at: SampleTime,
) -> Option<MixedResetRefused> {
    let renderer = &old.audio.renderer;
    if old.fault.is_some()
        || renderer.diagnostics().needs_reprepare()
        || old.ingress_halt.load(Ordering::Acquire)
    {
        return Some(MixedResetRefused::Faulted);
    }
    #[cfg(feature = "mixed-transfer-rehearsal")]
    let sealed = seal_state(old, successor);
    #[cfg(feature = "mixed-transfer-rehearsal")]
    if sealed == Some(MixedResetSealState::Cancelled) {
        return Some(MixedResetRefused::PanicCancelled);
    }
    if old.transport == MixedCompiledTransport::Running {
        return Some(MixedResetRefused::Running);
    }
    if let Some(stop) = old.stop_pending {
        return Some(MixedResetRefused::StopPending { stop });
    }
    // The old slot must be empty, as the successor's must: an uncollected receipt would
    // otherwise fade out with an owner whose control no longer collects it.
    match old.offer_slot {
        MixedOfferSlot::Empty => {}
        MixedOfferSlot::Pending(_) => return Some(MixedResetRefused::OfferPending),
        MixedOfferSlot::Retired(_) => return Some(MixedResetRefused::ActivationUncollected),
    }
    // Input the old owner would still apply after `B`.
    if old.parked_panic.is_some()
        || old.panic_cut.is_some()
        || old.ingress_result_pending.is_some()
        || old.invalid_ingress_command.is_some()
        || !old.test_ingress.is_empty()
        || private_input_pending(old)
    {
        return Some(MixedResetRefused::IngressPending);
    }
    #[cfg(feature = "mixed-transfer-rehearsal")]
    match sealed {
        Some(MixedResetSealState::Unsealed) => return Some(MixedResetRefused::Unsealed),
        Some(MixedResetSealState::IngressPending) => {
            return Some(MixedResetRefused::IngressPending);
        }
        Some(MixedResetSealState::Ready | MixedResetSealState::Cancelled) | None => {}
    }
    let next = &successor.audio.renderer;
    if next.epoch() == renderer.epoch()
        || next.table_id() == renderer.table_id()
        || next.plan().sample_rate() != renderer.plan().sample_rate()
        || next.plan().channel_layout() != renderer.plan().channel_layout()
        || next.plan().maximum_block_size() != renderer.plan().maximum_block_size()
    {
        return Some(MixedResetRefused::ForeignSuccessor);
    }
    if successor.render_started
        || successor.adopted
        || successor.fault.is_some()
        || next.diagnostics().needs_reprepare()
        || successor.ingress_halt.load(Ordering::Acquire)
        || successor.stop_pending.is_some()
        || !matches!(successor.offer_slot, MixedOfferSlot::Empty)
        || successor.transport != MixedCompiledTransport::Running
    {
        return Some(MixedResetRefused::SuccessorNotFresh);
    }
    let effective = successor.timing.effective();
    if effective != at {
        return Some(MixedResetRefused::SuccessorBoundary { effective, at });
    }
    None
}

/// The old owner's seal for `successor`, read once per check without changing it, or
/// `None` for an owner no bridge sealed.
#[cfg(feature = "mixed-transfer-rehearsal")]
fn seal_state(
    old: &MixedOneShotAudio,
    successor: &MixedOneShotAudio,
) -> Option<MixedResetSealState> {
    let Some(seal) = &old.reset_seal else {
        return None;
    };
    Some(seal.reset_seal_state(successor.transfer_binding, old.last_served_ingress_id))
}

/// Audio's one decision at `B` on a sealed old owner: a compare-and-swap from `Sealed`
/// to `Acquired` when every check passed, else to `Declined`. Should the bridge's Panic
/// have cancelled first, the refusal is `PanicCancelled`, unless the owner faulted. An
/// owner no bridge sealed keeps the checks' own outcome.
#[cfg(feature = "mixed-transfer-rehearsal")]
fn decide_seal(
    old: &MixedOneShotAudio,
    successor: &MixedOneShotAudio,
    checked: Result<SampleTime, MixedResetRefused>,
) -> Result<SampleTime, MixedResetRefused> {
    let Some(seal) = &old.reset_seal else {
        return checked;
    };
    match seal.reset_seal_decide(successor.transfer_binding, checked.is_ok()) {
        MixedResetSealDecision::Decided => checked,
        MixedResetSealDecision::Cancelled => match checked {
            Err(MixedResetRefused::Faulted) => checked,
            _ => Err(MixedResetRefused::PanicCancelled),
        },
        // Defensive: the checks read a published seal naming this successor, and only
        // the control side reopens it, after the successor came back.
        MixedResetSealDecision::NotSealed => match checked {
            Ok(_) => Err(MixedResetRefused::Unsealed),
            Err(_) => checked,
        },
    }
}

/// Audio acquired the seal but refused at the old owner's `Reset` group.
#[cfg(feature = "mixed-transfer-rehearsal")]
fn decline_seal(old: &MixedOneShotAudio) {
    if let Some(seal) = &old.reset_seal {
        seal.reset_seal_decline();
    }
}

/// Input only the private rehearsal has: a command in the owner's own ring, or the
/// test-only live seam's identity, which lies outside the store and so outside the
/// reset's group. A host-armed owner's port is the host's queue, whose occupancy audio
/// cannot see.
#[cfg(all(test, feature = "simulated-ingress"))]
fn private_input_pending(old: &MixedOneShotAudio) -> bool {
    !old.ingress_commands.0.is_empty() || old.test_live.is_some()
}

#[cfg(not(all(test, feature = "simulated-ingress")))]
const fn private_input_pending(_old: &MixedOneShotAudio) -> bool {
    false
}

/// The successor's gain at frame `position` of the fade, `(position + 1) / Q`, so the
/// last fade frame is the successor alone. `None` outside the fade.
pub(super) fn fade_gain(position: usize) -> Option<f32> {
    let step = u16::try_from(position.checked_add(1)?).ok()?;
    let quantum = u16::try_from(QUANTUM_FRAMES).ok()?;
    if step > quantum {
        return None;
    }
    Some(f32::from(step) / f32::from(quantum))
}

impl MixedOneShotAudio {
    /// Start a reset successor at `at`. Its renderer's clock moves to the boundary with
    /// no primed carry, so it renders the same frames as the fading owner, and its
    /// compiled transport is stopped at its anchor's position, so its armed adoption at
    /// `at` publishes only scoped restoration.
    pub(super) fn install_reset_successor(&mut self, at: SampleTime) {
        let position = self.effective_anchor.position();
        self.audio
            .renderer
            .install_stopped_clock(StreamAnchor::new(at, position));
        self.transport = MixedCompiledTransport::Stopped {
            at,
            released: HeldNoteCount::NONE,
            position: Some(position),
        };
    }
}

/// Render the fading owner's quantum after its fade into scratch and discard it. Its
/// `Reset` group applies at that quantum's first sample, so its registry releases every
/// old identity there. Afterwards the old owner must hold no live obligation at all.
fn discard_quantum(
    old: &mut MixedOneShotAudio,
    scratch: &mut [f32],
    layout: ChannelLayout,
) -> Result<(), MixedResetRenderError> {
    let maximum = usize::try_from(old.audio.renderer.plan().maximum_block_size().as_u64())
        .map_err(|_| MixedResetRenderError::Scratch)?;
    let channels = layout.channels();
    let mut remaining = QUANTUM_FRAMES as usize;
    while remaining > 0 {
        let frames = remaining.min(maximum);
        if frames == 0 {
            return Err(MixedResetRenderError::Scratch);
        }
        let samples = frames
            .checked_mul(channels)
            .ok_or(MixedResetRenderError::Scratch)?;
        let discarded = scratch
            .get_mut(..samples)
            .ok_or(MixedResetRenderError::Scratch)?;
        let discarded = AudioBlockMut::new(discarded, frames, layout)
            .map_err(|_| MixedResetRenderError::Scratch)?;
        old.render_with(&mut MixedResetFadeQueues, discarded)
            .map_err(MixedResetRenderError::Fading)?;
        remaining -= frames;
    }
    if old.test_ingress.holds_outstanding() != EventCount::NONE
        || !old.test_ingress.is_empty()
        || old.audio.live_minter.live() != 0
        || old.audio.compiled_minter.live() != 0
        || old.audio.renderer.has_replacement_obligations()
    {
        return Err(MixedResetRenderError::Custody);
    }
    Ok(())
}

impl<E: MixedAudioQueues> MixedResetPair<E> {
    /// Render the active owner through its own endpoint and, from a reset's boundary,
    /// the fading one through none. Every terminal failure silences the caller's complete
    /// block and keeps both owners and both endpoints.
    pub(super) fn render_private(
        &mut self,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), MixedResetRenderError> {
        if self.fault.is_some() {
            output.silence();
            return Err(MixedResetRenderError::Faulted);
        }
        let plan = self.active.audio.renderer.plan();
        if output.layout() != plan.channel_layout()
            || u64::try_from(output.frames())
                .map_or(true, |frames| frames > plan.maximum_block_size().as_u64())
        {
            return Err(MixedResetRenderError::OutputShape);
        }
        if let Err(error) = self.render_inner(&mut output) {
            self.fault = Some(error);
            self.active.ingress_halt.store(true, Ordering::Release);
            match &self.secondary {
                MixedResetSlot::Empty => {}
                MixedResetSlot::Pending { successor, .. }
                | MixedResetSlot::Refused { successor, .. } => {
                    successor.ingress_halt.store(true, Ordering::Release);
                }
                MixedResetSlot::Fading { old, .. } | MixedResetSlot::Retired { old, .. } => {
                    old.ingress_halt.store(true, Ordering::Release);
                }
            }
            output.silence();
            return Err(error);
        }
        Ok(())
    }

    /// Split the callback at the active owner's quantum boundaries, as each owner does,
    /// so a reset begins exactly at its boundary and both owners see the same segments.
    fn render_inner(
        &mut self,
        output: &mut AudioBlockMut<'_>,
    ) -> Result<(), MixedResetRenderError> {
        let mut offset = 0;
        while offset < output.frames() {
            if self.active.audio.renderer.carry_frames() == 0 {
                self.open_boundary();
            }
            let carry = self.active.audio.renderer.carry_frames();
            let frames = (output.frames() - offset).min(if carry == 0 {
                QUANTUM_FRAMES as usize
            } else {
                carry
            });
            let Some(mut block) = output.window(offset, frames) else {
                return Err(MixedResetRenderError::Scratch);
            };
            self.active
                .render_with(&mut self.active_endpoint, block.reborrow())
                .map_err(MixedResetRenderError::Active)?;
            self.fade(&mut block)?;
            offset += frames;
        }
        Ok(())
    }

    /// At a boundary of the active owner, decide a successor due here. A refusal puts it
    /// back in the slot unchanged, with nothing else changed. Otherwise the old owner's
    /// live occurrences are ended at `B + Q`, the successor starts at `B` and becomes
    /// the active owner with its own endpoint, and the old owner moves into the slot to
    /// fade with its endpoint, which it no longer serves.
    fn open_boundary(&mut self) {
        let clock = self.active.audio.renderer.clock();
        let due = matches!(&self.secondary, MixedResetSlot::Pending { at, .. } if *at <= clock);
        if !due {
            return;
        }
        let (mut successor, mut endpoint, at) = match core::mem::take(&mut self.secondary) {
            MixedResetSlot::Pending {
                successor,
                endpoint,
                at,
            } => (successor, endpoint, at),
            other => {
                self.secondary = other;
                return;
            }
        };
        match self.begin_reset(&successor, at, clock) {
            Err(reason) => {
                self.secondary = MixedResetSlot::Refused {
                    successor,
                    endpoint,
                    at,
                    reason,
                };
            }
            Ok(receipt) => {
                successor.install_reset_successor(at);
                core::mem::swap(&mut self.active, &mut successor);
                core::mem::swap(&mut self.active_endpoint, &mut endpoint);
                self.secondary = MixedResetSlot::Fading {
                    old: successor,
                    endpoint,
                    receipt,
                    faded: 0,
                };
            }
        }
    }

    /// Every check, then a sealed old owner's one seal decision, then the old owner's
    /// `Reset` group, which is its only change. A refusal before or by the group leaves
    /// both owners as they were; only the seal records that audio declined.
    fn begin_reset(
        &mut self,
        successor: &MixedOneShotAudio,
        at: SampleTime,
        clock: SampleTime,
    ) -> Result<MixedResetReceipt, MixedResetRefused> {
        let checked = if at < clock {
            Err(MixedResetRefused::BoundaryPassed { at, clock })
        } else if let Some(reason) = reset_refusal(&self.active, successor, at) {
            Err(reason)
        } else {
            at.checked_add(FrameCount::QUANTUM)
                .map_err(|_| MixedResetRefused::FadeUnrepresentable { at })
        };
        #[cfg(feature = "mixed-transfer-rehearsal")]
        let checked = decide_seal(&self.active, successor, checked);
        let ended_at = checked?;
        let old = &mut *self.active;
        let epoch = old.audio.renderer.epoch();
        match old.occurrences.end_live_for_reset(
            &mut old.test_ingress,
            &mut old.audio.live_minter,
            epoch,
            at,
            ended_at,
        ) {
            Ok(live) => {
                #[cfg(feature = "mixed-transfer-rehearsal")]
                old.observe_ingress_usage();
                Ok(MixedResetReceipt { at, ended_at, live })
            }
            Err(refused) => {
                #[cfg(feature = "mixed-transfer-rehearsal")]
                decline_seal(old);
                Err(MixedResetRefused::Live(refused))
            }
        }
    }

    /// Cross-fade one segment. The fading owner renders the same frames into scratch,
    /// and the frame at offset `p` from the boundary sums the successor at
    /// `(p + 1) / Q` and the old owner at the rest, so only that offset decides a
    /// sample, never the callback partition. The discarded quantum follows the fade's
    /// last frame at once, and then the old owner retires into the slot.
    fn fade(&mut self, block: &mut AudioBlockMut<'_>) -> Result<(), MixedResetRenderError> {
        let MixedResetSlot::Fading { old, faded, .. } = &mut self.secondary else {
            return Ok(());
        };
        let scratch = &mut self.scratch;
        let frames = block.frames();
        let layout = block.layout();
        let channels = layout.channels();
        let quantum = QUANTUM_FRAMES as usize;
        let start = *faded;
        let end = start
            .checked_add(frames)
            .ok_or(MixedResetRenderError::Scratch)?;
        if end > quantum {
            return Err(MixedResetRenderError::Scratch);
        }
        let samples = frames
            .checked_mul(channels)
            .ok_or(MixedResetRenderError::Scratch)?;
        {
            let fading = scratch
                .get_mut(..samples)
                .ok_or(MixedResetRenderError::Scratch)?;
            let fading = AudioBlockMut::new(fading, frames, layout)
                .map_err(|_| MixedResetRenderError::Scratch)?;
            old.render_with(&mut MixedResetFadeQueues, fading)
                .map_err(MixedResetRenderError::Fading)?;
        }
        let fading = scratch
            .get(..samples)
            .ok_or(MixedResetRenderError::Scratch)?;
        let output = block.samples_mut();
        for frame in 0..frames {
            let Some(gain) = fade_gain(start + frame) else {
                return Err(MixedResetRenderError::Scratch);
            };
            for channel in 0..channels {
                let index = frame * channels + channel;
                let (Some(sample), Some(previous)) = (output.get_mut(index), fading.get(index))
                else {
                    return Err(MixedResetRenderError::Scratch);
                };
                *sample = *sample * gain + *previous * (1.0 - gain);
            }
        }
        *faded = end;
        if end < quantum {
            return Ok(());
        }
        discard_quantum(old, scratch, layout)?;
        match core::mem::take(&mut self.secondary) {
            MixedResetSlot::Fading {
                old,
                endpoint,
                receipt,
                ..
            } => {
                self.secondary = MixedResetSlot::Retired {
                    old,
                    endpoint,
                    receipt,
                };
            }
            other => self.secondary = other,
        }
        Ok(())
    }
}

impl<E> MixedResetPair<E> {
    /// The active owner, for reading its own slots between callbacks.
    pub(super) fn active(&self) -> &MixedOneShotAudio {
        &self.active
    }
}
