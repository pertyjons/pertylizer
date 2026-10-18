//! Strict, promised-source publication for the non-shipping combined host.
//!
//! These certificates concern admitted source observations, not a future host Panic
//! or transport request. They are suitable for sources promising complete prefixes.
//! They do not replace the bridge's tolerant peer-fence or ADR-0079 raw-only policy.
use super::{MixedTransferAudio, MixedTransferBinding};
use crate::host::{ConnectionGeneration, mixed_input::MixedRawPublication};
use crate::{
    render::PreparedRenderer,
    time::{FrameCount, SampleTime, TimeError},
};
mod errors;
pub use errors::{MixedPublicationFault, MixedPublicationRefusal};

mod mailbox;
pub use mailbox::{MixedPublicationPublishError, MixedPublicationReceiver, MixedPublicationSender};

/// The globally issued, non-reused transfer serial is sufficient for publication
/// identity. Unlike the transfer's full binding it has an atomic representation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MixedPublicationOwner(u64);
impl MixedPublicationOwner {
    pub(super) const fn of(binding: MixedTransferBinding) -> Self {
        Self(binding.transfer)
    }
}

/// An immutable source prefix proved by the bridge's actual stage and frontiers.
/// Hosts publish it after their command pushes, and audio reads it BEFORE draining
/// those commands. It cannot be constructed or relabelled by a caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedPublication {
    pub(super) root: MixedPublicationOwner,
    pub(super) binding: MixedPublicationOwner,
    pub(super) successor: Option<(MixedPublicationOwner, SampleTime)>,
    pub(super) through: SampleTime,
    pub(super) sources: [ConnectionGeneration; 2],
}
impl MixedPublication {
    /// Every promised-source observation before this exclusive bound was forwarded
    /// or explicitly settled. A reset transition instead proves nothing is due there.
    pub const fn through(self) -> SampleTime {
        self.through
    }

    pub(crate) fn accepts(self, binding: Option<MixedTransferBinding>, clock: SampleTime) -> bool {
        let binding = binding.map(MixedPublicationOwner::of);
        binding == Some(self.binding)
            || self
                .successor
                .is_some_and(|(next, at)| binding == Some(next) && clock >= at)
    }
}

/// Audio-owned strict prefix guard. Raw coverage is supplied by the host only after
/// both raw frontier packets have successfully reached its FIFO audio queue.
/// The guard neither classifies raw self-halts nor changes source-fence policy.
#[derive(Debug)]
#[must_use]
pub struct MixedPublicationGate {
    root: MixedPublicationOwner,
    current: Option<(MixedPublication, MixedRawPublication)>,
    fault: Option<MixedPublicationFault>,
}
impl MixedPublicationGate {
    pub(super) const fn new(root: MixedTransferBinding) -> Self {
        Self {
            root: MixedPublicationOwner::of(root),
            current: None,
            fault: None,
        }
    }
    /// The first failed callback preflight, which no later update clears.
    pub const fn fault(&self) -> Option<MixedPublicationFault> {
        self.fault
    }

    pub(crate) fn offer(
        &mut self,
        publication: MixedPublication,
        raw: MixedRawPublication,
        binding: Option<MixedTransferBinding>,
        clock: SampleTime,
    ) -> Result<(), MixedPublicationRefusal> {
        if publication.root != self.root
            || raw.sources() != publication.sources
            || !publication.accepts(binding, clock)
        {
            return Err(MixedPublicationRefusal::Foreign);
        }
        let ready = raw.ready_through(publication.through);
        if self.current.is_some_and(|(old, old_raw)| {
            old.accepts(binding, clock) && ready < old_raw.ready_through(old.through)
        }) {
            return Err(MixedPublicationRefusal::Regressed);
        }
        // Store one complete proof, never maxima assembled from different snapshots.
        self.current = Some((publication, raw));
        Ok(())
    }
    pub(crate) fn check(
        &mut self,
        binding: Option<MixedTransferBinding>,
        renderer: &PreparedRenderer,
        frames: FrameCount,
        crossing: Option<(MixedTransferBinding, SampleTime)>,
    ) -> Result<(), MixedPublicationFault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        let checked = (|| {
            let frames =
                usize::try_from(frames.as_u64()).map_err(|_| TimeError::ClockExhausted {
                    position: renderer.clock().as_u64(),
                    frames: frames.as_u64(),
                })?;
            let quanta = u64::try_from(renderer.quanta_needed_for(frames)).map_err(|_| {
                TimeError::ClockExhausted {
                    position: renderer.clock().as_u64(),
                    frames: u64::MAX,
                }
            })?;
            let needed = quanta.checked_mul(FrameCount::QUANTUM.as_u64()).ok_or(
                TimeError::ClockExhausted {
                    position: renderer.clock().as_u64(),
                    frames: u64::MAX,
                },
            )?;
            let required = renderer.clock().checked_add(FrameCount::new(needed))?;
            let Some((publication, raw)) = self
                .current
                .filter(|(p, _)| p.accepts(binding, renderer.clock()))
            else {
                return Err(MixedPublicationFault::MissingArm);
            };
            if crossing.is_some_and(|(next, at)| {
                required > at
                    && publication.successor != Some((MixedPublicationOwner::of(next), at))
            }) {
                return Err(MixedPublicationFault::MissingArm);
            }
            let ready = raw.ready_through(publication.through);
            if required > ready {
                return Err(MixedPublicationFault::Unprepared { ready, required });
            }
            Ok(())
        })();
        if let Err(fault) = checked {
            self.fault = Some(fault);
        }
        checked
    }
}

impl MixedTransferAudio {
    /// Accept a complete sound/raw update before draining this callback's commands.
    ///
    /// # Errors
    /// A foreign or regressing certificate; the previous proof remains unchanged.
    pub fn offer_publication(
        &self,
        gate: &mut MixedPublicationGate,
        publication: MixedPublication,
        raw: MixedRawPublication,
    ) -> Result<(), MixedPublicationRefusal> {
        gate.offer(
            publication,
            raw,
            self.inner.transfer_binding,
            self.inner.audio.renderer.clock(),
        )
    }
    /// Preflight an already shape/profile-validated callback's opened quanta,
    /// including actual output carry. The host rejects invalid output shape before
    /// this source preflight, without changing the guard.
    /// On failure the host silences the whole callback and requests its shared halt
    /// before rendering raw. The gate retains the reason. This does not render audio.
    ///
    /// # Errors
    /// A missing active-arm proof, insufficient prefix or exhausted clock.
    pub fn check_publication(
        &self,
        gate: &mut MixedPublicationGate,
        frames: FrameCount,
    ) -> Result<(), MixedPublicationFault> {
        gate.check(
            self.inner.transfer_binding,
            &self.inner.audio.renderer,
            frames,
            None,
        )
    }
}
