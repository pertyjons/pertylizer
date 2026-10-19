//! In-place fixed native geometry replacement without changing source ownership.
use super::super::{MixedOfferSlot, MixedOneShotAudio};
use super::{MixedTransferAudio, MixedTransferBinding, MixedTransferControl};
use crate::{
    diagnostics::CompileError,
    host::{
        mixed_targets::{MixedTargetAdmission, MixedTargetParts},
        session::transfer::replacement::same_continuation_program,
    },
    plan::{ContinuationError, PlanId, binding::FixedPlanBindings, continuation::PlanContinuation},
    quantities::{PreparedBytes, RecordCount},
    render::PreparedRenderer,
    time::{QUANTUM_FRAMES, SampleTime},
    transport::ActivationSequence,
};
use std::sync::Arc;
use thiserror::Error;

mod hot;
#[cfg(test)]
pub(in crate::stream::mixed::transfer) mod tests;

#[derive(Debug, Error)]
pub enum MixedContinuationPrepareError {
    #[error("another plan-specific operation still owns the continuation credit")]
    Busy,
    #[error("the mixed owner is terminally faulted")]
    Faulted,
    #[error("fixed native plan proof refused: {0}")]
    Proof(ContinuationError),
    #[error("successor renderer preparation refused: {0}")]
    Renderer(CompileError),
    #[error("successor producer or parameter partition differs")]
    Partition,
    #[error("continuation metadata exceeds its byte allowance")]
    Bytes,
    #[error("continuation metadata exceeds its record allowance")]
    Records,
    #[error("continuation boundary must be quantum aligned")]
    Boundary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum MixedContinuationRefusal {
    #[error("another owning plan operation is pending or uncollected")]
    Busy,
    #[error("continuation belongs to another active plan or source owner")]
    Binding,
    #[error("continuation boundary has passed")]
    BoundaryPassed,
    #[error("initial mixed compiled adoption has not completed")]
    NotAdopted,
    #[error("the mixed owner is terminally faulted")]
    Faulted,
    #[error("actual continuation geometry or prospective bindings are inconsistent")]
    Storage,
    #[error("the unpublished continuation was cancelled")]
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixedContinuationOutcome {
    Refused(MixedContinuationRefusal),
    InstalledRendered { plan: PlanId, boundary: SampleTime },
    InstalledFaulted { plan: PlanId, boundary: SampleTime },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Ready,
    Installed,
    Finished(MixedContinuationOutcome),
}

/// The sole owning credit. Resolve by cancellation or the audio half's owning return.
/// Its metadata allowance excludes the separately admitted immutable plan and renderer
/// geometry; the active owner and at most one successor geometry coexist.
#[must_use]
pub struct MixedContinuationPacket {
    binding: MixedTransferBinding,
    expected: ActivationSequence,
    boundary: SampleTime,
    proof: PlanContinuation,
    bindings: FixedPlanBindings,
    prepared: PreparedRenderer,
    next: MixedTargetParts,
    audio_partition: Arc<crate::host::mixed_targets::MixedInstancePartition>,
    bytes: PreparedBytes,
    stage: Stage,
    // Last: weak custody expires after all packet fields have been destroyed cold.
    _custody: Arc<()>,
}
impl std::fmt::Debug for MixedContinuationPacket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MixedContinuationPacket")
            .field("old", &self.proof.old)
            .field("new", &self.proof.new)
            .field("boundary", &self.boundary)
            .field("stage", &self.stage)
            .field("metadata_bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}
/// Outgoing plan/partition and displaced controls, plus unused preparation
/// storage, after installation; untouched successor on refusal. Drop cold.
/// Sounding state and identity authorities stay in the active renderer.
#[derive(Debug)]
#[must_use]
pub struct MixedContinuationRetired(Box<MixedContinuationPacket>);
impl MixedContinuationRetired {
    pub fn outcome(&self) -> Option<MixedContinuationOutcome> {
        self.0.outcome()
    }
    pub const fn metadata_bytes(&self) -> PreparedBytes {
        self.0.bytes
    }
}

struct PreparedContinuationGeometry {
    proof: PlanContinuation,
    bindings: FixedPlanBindings,
    renderer: PreparedRenderer,
    bytes: PreparedBytes,
}

impl MixedTransferControl {
    fn prepare_continuation_geometry(
        &self,
        targets: &MixedTargetAdmission,
        metadata_allowance: PreparedBytes,
        maximum_records: RecordCount,
    ) -> Result<PreparedContinuationGeometry, MixedContinuationPrepareError> {
        if self
            .activation_custody
            .load(std::sync::atomic::Ordering::Acquire)
            != 0
            || self.continuation_pending.is_some()
            || self
                .reset_seal
                .as_ref()
                .is_some_and(|seal| !seal.continuation_is_open())
        {
            return Err(MixedContinuationPrepareError::Busy);
        }
        if self.fault.is_some()
            || self
                .inner
                .ingress_halt
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(MixedContinuationPrepareError::Faulted);
        }
        let old = &self.inner.control;
        let proof = old
            .plan
            .prepare_continuation(targets.plan(), metadata_allowance, maximum_records)
            .map_err(MixedContinuationPrepareError::Proof)?;
        if proof.structural.is_some() {
            return Err(MixedContinuationPrepareError::Proof(
                ContinuationError::Plan,
            ));
        }
        if !old
            .partition
            .same_fixed_geometry(targets.instance_partition())
        {
            return Err(MixedContinuationPrepareError::Partition);
        }
        let bindings = FixedPlanBindings::prepare(&old.plan, targets.plan())
            .map_err(MixedContinuationPrepareError::Proof)?;
        if old.compiled_slots.len() != targets.compiled_slots().len()
            || old
                .compiled_slots
                .iter()
                .zip(targets.compiled_slots())
                .any(|(a, b)| !bindings.accepts_note(*a) || bindings.note(*a) != *b)
        {
            return Err(MixedContinuationPrepareError::Partition);
        }
        same_continuation_program(
            &old.plan,
            targets.plan(),
            &old.stream,
            targets.stream(),
            &proof,
        )
        .map_err(MixedContinuationPrepareError::Proof)?;
        let records = u32::try_from(proof.trims.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or(MixedContinuationPrepareError::Records)?;
        if records > maximum_records.get() {
            return Err(MixedContinuationPrepareError::Records);
        }
        let inline_proof = u64::try_from(size_of::<PlanContinuation>())
            .map_err(|_| MixedContinuationPrepareError::Bytes)?;
        let bytes = proof
            .bytes
            .get()
            .checked_sub(inline_proof)
            .and_then(|bytes| {
                bytes.checked_add(u64::try_from(size_of::<MixedContinuationPacket>()).ok()?)
            })
            .and_then(|bytes| bytes.checked_add(targets.owned_heap_bytes()?.get()))
            .ok_or(MixedContinuationPrepareError::Bytes)?;
        if bytes > metadata_allowance.get() {
            return Err(MixedContinuationPrepareError::Bytes);
        }
        let mut renderer = PreparedRenderer::prepare_mixed(
            Arc::clone(targets.plan_arc()),
            old.anchor,
            old.epoch,
            old.table,
        )
        .map_err(MixedContinuationPrepareError::Renderer)?;
        if !renderer.bind_mixed_partition(Arc::clone(targets.partition_arc())) {
            return Err(MixedContinuationPrepareError::Partition);
        }
        Ok(PreparedContinuationGeometry {
            proof,
            bindings,
            renderer,
            bytes: PreparedBytes::measured(bytes),
        })
    }

    /// Prepare all fallible work before publication. Refusal returns the complete
    /// bound successor admission; no source, handle, frontier or minter changes.
    /// The host chooses a fresh aligned sample boundary after proof and renderer preparation.
    pub fn prepare_continuation(
        &mut self,
        targets: MixedTargetAdmission,
        choose_boundary: impl FnOnce() -> Option<SampleTime>,
        metadata_allowance: PreparedBytes,
        maximum_records: RecordCount,
    ) -> Result<
        Box<MixedContinuationPacket>,
        Box<(MixedContinuationPrepareError, MixedTargetAdmission)>,
    > {
        let geometry =
            self.prepare_continuation_geometry(&targets, metadata_allowance, maximum_records);
        let PreparedContinuationGeometry {
            proof,
            bindings,
            renderer: prepared,
            bytes,
        } = match geometry {
            Ok(geometry) => geometry,
            Err(error) => return Err(Box::new((error, targets))),
        };
        // Sample the host clock after expensive proof and renderer preparation.
        // The small owning envelope is boxed below; a delayed offer still refuses
        // explicitly rather than moving the chosen boundary.
        let Some(boundary) = choose_boundary()
            .filter(|boundary| boundary.as_u64().is_multiple_of(u64::from(QUANTUM_FRAMES)))
        else {
            return Err(Box::new((MixedContinuationPrepareError::Boundary, targets)));
        };
        let next = targets.into_parts();
        let audio_partition = Arc::clone(&next.partition);
        let custody = Arc::new(());
        self.continuation_pending = Some(proof.new);
        self.continuation_custody = Some(Arc::downgrade(&custody));
        Ok(Box::new(MixedContinuationPacket {
            binding: self.binding,
            expected: self.inner.in_force,
            boundary,
            proof,
            bindings,
            prepared,
            next,
            audio_partition,
            bytes,
            stage: Stage::Ready,
            _custody: custody,
        }))
    }

    pub fn cancel_continuation(
        &mut self,
        mut packet: Box<MixedContinuationPacket>,
    ) -> Result<MixedContinuationRetired, (Box<MixedContinuationPacket>, MixedContinuationRefusal)>
    {
        if packet.binding != self.binding
            || self.continuation_pending != Some(packet.proof.new)
            || packet.stage != Stage::Ready
        {
            return Err((packet, MixedContinuationRefusal::Binding));
        }
        packet.stage = Stage::Finished(MixedContinuationOutcome::Refused(
            MixedContinuationRefusal::Cancelled,
        ));
        self.continuation_pending = None;
        self.continuation_custody = None;
        Ok(MixedContinuationRetired(packet))
    }

    /// Promote only the actual installed metadata, including faulted installations.
    /// Only InstalledRendered is evidence of successful successor output.
    pub fn collect_continuation(
        &mut self,
        mut packet: Box<MixedContinuationPacket>,
    ) -> Result<MixedContinuationRetired, (Box<MixedContinuationPacket>, MixedContinuationRefusal)>
    {
        if packet.binding != self.binding
            || self.continuation_pending != Some(packet.proof.new)
            || self.inner.control.plan.id() != packet.proof.old
        {
            return Err((packet, MixedContinuationRefusal::Binding));
        }
        let Stage::Finished(outcome) = packet.stage else {
            return Err((packet, MixedContinuationRefusal::Busy));
        };
        if matches!(
            outcome,
            MixedContinuationOutcome::InstalledRendered { .. }
                | MixedContinuationOutcome::InstalledFaulted { .. }
        ) {
            let control = &mut self.inner.control;
            core::mem::swap(&mut control.plan, &mut packet.next.plan);
            core::mem::swap(&mut control.stream, &mut packet.next.stream);
            core::mem::swap(&mut control.compiled_slots, &mut packet.next.compiled_slots);
            core::mem::swap(&mut control.partition, &mut packet.next.partition);
        }
        self.continuation_pending = None;
        self.continuation_custody = None;
        Ok(MixedContinuationRetired(packet))
    }
}

/// Recovery reports custody, never successful rendered-output evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum MixedContinuationRecovery {
    Unpublished,
    InstalledUnconfirmed { plan: PlanId },
}

impl MixedTransferControl {
    /// Recover after a host has destroyed an owning packet without collecting it.
    /// Both halves must be quiescent. This rebuilds immutable plan/program metadata
    /// only; it never snapshots, resets or replaces either actual identity authority.
    pub fn recover_abandoned_continuation(
        &mut self,
        audio: &mut MixedTransferAudio,
    ) -> Result<Option<MixedContinuationRecovery>, MixedContinuationRefusal> {
        self.recover_abandoned_owner(&mut audio.inner)
    }

    pub(in crate::stream::mixed) fn recover_abandoned_owner(
        &mut self,
        audio: &mut MixedOneShotAudio,
    ) -> Result<Option<MixedContinuationRecovery>, MixedContinuationRefusal> {
        let Some(pending) = self.continuation_pending else {
            return Ok(None);
        };
        if audio.transfer_binding != Some(self.binding) {
            return Err(MixedContinuationRefusal::Binding);
        }
        if audio.continuation.is_some()
            || self
                .continuation_custody
                .as_ref()
                .is_none_or(|custody| custody.strong_count() != 0)
        {
            return Err(MixedContinuationRefusal::Busy);
        }
        let actual = audio.audio.renderer.plan();
        let outcome = if actual.id() == self.inner.control.plan.id() {
            MixedContinuationRecovery::Unpublished
        } else {
            if actual.id() != pending
                || actual.id() != audio.audio.partition.plan_id()
                || self.binding.owner.table != audio.audio.renderer.table_id()
                || self.binding.owner.epoch != audio.audio.renderer.epoch()
            {
                return Err(MixedContinuationRefusal::Binding);
            }
            let old = &self.inner.control;
            let bindings = FixedPlanBindings::prepare(&old.plan, actual)
                .map_err(|_| MixedContinuationRefusal::Storage)?;
            let mut events = Vec::new();
            events
                .try_reserve_exact(old.stream.events().len())
                .map_err(|_| MixedContinuationRefusal::Storage)?;
            for event in old.stream.events() {
                let payload =
                    crate::host::session::transfer::replacement::rebind_continuation_program(
                        event.payload(),
                        &old.plan,
                        actual,
                    )
                    .map_err(|_| MixedContinuationRefusal::Storage)?;
                events.push(crate::schedule::PlanEvent::new(event.position(), payload));
            }
            let stream = crate::schedule::AdmittedCompiledStream::admit(actual, &events)
                .map_err(|_| MixedContinuationRefusal::Storage)?;
            let mut slots = Vec::new();
            slots
                .try_reserve_exact(old.compiled_slots.len())
                .map_err(|_| MixedContinuationRefusal::Storage)?;
            for slot in &old.compiled_slots {
                if !bindings.accepts_note(*slot) {
                    return Err(MixedContinuationRefusal::Storage);
                }
                slots.push(bindings.note(*slot));
            }
            let plan = Arc::clone(audio.audio.renderer.plan_arc());
            let partition = Arc::clone(&audio.audio.partition);
            let control = &mut self.inner.control;
            control.plan = plan;
            control.stream = stream;
            control.compiled_slots = slots;
            control.partition = partition;
            MixedContinuationRecovery::InstalledUnconfirmed { plan: pending }
        };
        self.continuation_pending = None;
        self.continuation_custody = None;
        Ok(Some(outcome))
    }
}
