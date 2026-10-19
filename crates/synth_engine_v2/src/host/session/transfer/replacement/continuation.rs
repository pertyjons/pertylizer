//! Cold exact-program proof and separately charged owning event copies.
mod hot;
use super::{PreparedPlanReplacement, SessionControl};
use crate::{
    host::session::transfer::continuation::{
        AdoptedTimelineWitness, ContinuationLimits, charged_bytes, copy_events,
    },
    plan::{
        CompiledPlan, ContinuationError, NoteSlot, ParameterSlot, continuation::PlanContinuation,
    },
    quantities::PreparedBytes,
    render::{EventPayload, NoteEdge, TimedEvent},
    schedule::{AdmittedCompiledStream, CompiledPayload},
    time::{SampleTime, StreamAnchor},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContinuationStage {
    Ready,
    Rendering,
    Complete,
}

pub(super) struct ContinuingReplacement {
    pub(super) proof: PlanContinuation,
    pub(super) anchor: StreamAnchor,
    pub(super) boundary: SampleTime,
    pub(super) stage: ContinuationStage,
    pub(super) events: Vec<TimedEvent>,
    pub(super) witness: Option<Box<AdoptedTimelineWitness>>,
    pub(super) retired_catch_up: Vec<TimedEvent>,
    pub(super) bytes: PreparedBytes,
}

impl SessionControl {
    /// Unsupported programs choose reset; inconsistent bindings and resource errors surface.
    pub(super) fn try_continuation(
        &mut self,
        plan: &CompiledPlan,
        stream: &AdmittedCompiledStream,
        limits: ContinuationLimits,
    ) -> Result<Option<ContinuingReplacement>, ContinuationError> {
        let valid = self.active_timeline.as_ref().is_some_and(|witness| {
            witness.matches(
                self.origin,
                self.control.continuation_sequence(),
                self.control.anchor(),
            )
        });
        if !valid {
            // This metadata cannot authorize an installation. Retire it cold and
            // preserve forward progress through the existing reset path.
            self.active_timeline = None;
            return Ok(None);
        }
        let witness = self
            .active_timeline
            .as_ref()
            .ok_or(ContinuationError::Unavailable)?;
        if witness.bytes
            != charged_bytes(
                witness.events.capacity(),
                size_of::<AdoptedTimelineWitness>(),
            )?
            || self.control.table_id() != self.origin.table
            || self.control.epoch() != self.origin.epoch
            || self.control.plan().id() != self.origin.plan
        {
            return Err(ContinuationError::Storage);
        }
        let proof = match self.control.plan().prepare_continuation(
            plan,
            limits.transition_bytes,
            limits.maximum_transition_records,
        ) {
            Ok(proof) => proof,
            Err(ContinuationError::Plan) => return Ok(None),
            Err(error) => return Err(error),
        };
        match self.control.preflight_continuation_plan(plan, &proof.notes) {
            Ok(()) => {}
            Err(ContinuationError::Plan) => return Ok(None),
            Err(error) => return Err(error),
        }
        match same_program(self.control.plan(), plan, &self.stream, stream, &proof) {
            Ok(()) => {}
            Err(ContinuationError::Timeline) => return Ok(None),
            Err(error) => return Err(error),
        }
        // Both copies are admitted together, including actual capacities and fixed metadata.
        let required = charged_bytes(witness.events.len(), size_of::<ContinuingReplacement>())?
            .get()
            .checked_add(
                charged_bytes(witness.events.len(), size_of::<AdoptedTimelineWitness>())?.get(),
            )
            .and_then(|bytes| bytes.checked_add(proof.bytes.get()))
            .ok_or(ContinuationError::Bytes)?;
        if required > limits.transition_bytes.get() {
            return Err(ContinuationError::Bytes);
        }

        let mut events = copy_events(
            &witness.events,
            limits.maximum_stamped_events,
            limits.transition_bytes,
            size_of::<ContinuingReplacement>(),
        )?;
        for event in &mut events {
            *event = relabel(*event, self.control.plan(), plan, witness, &proof.notes)?;
        }
        let successor_events = copy_events(
            &events,
            limits.maximum_stamped_events,
            limits.witness_bytes,
            size_of::<AdoptedTimelineWitness>(),
        )?;
        let witness_bytes = charged_bytes(
            successor_events.capacity(),
            size_of::<AdoptedTimelineWitness>(),
        )?;
        let bytes = charged_bytes(events.capacity(), size_of::<ContinuingReplacement>())?
            .get()
            .checked_add(witness_bytes.get())
            .and_then(|bytes| bytes.checked_add(proof.bytes.get()))
            .ok_or(ContinuationError::Bytes)?;
        if bytes > limits.transition_bytes.get() {
            return Err(ContinuationError::Bytes);
        }
        let successor = AdoptedTimelineWitness {
            plan: plan.id(),
            epoch: witness.epoch,
            table: witness.table,
            sequence: witness.sequence,
            anchor: witness.anchor,
            events: successor_events,
            bytes: witness_bytes,
        };
        Ok(Some(ContinuingReplacement {
            proof,
            anchor: witness.anchor,
            boundary: witness.anchor.time(),
            stage: ContinuationStage::Ready,
            events,
            witness: Some(Box::new(successor)),
            retired_catch_up: Vec::new(),
            bytes: PreparedBytes::measured(bytes),
        }))
    }
}

pub(crate) fn same_program(
    old: &CompiledPlan,
    new: &CompiledPlan,
    old_stream: &AdmittedCompiledStream,
    new_stream: &AdmittedCompiledStream,
    proof: &PlanContinuation,
) -> Result<(), ContinuationError> {
    if old_stream.plan() != old.id() || new_stream.plan() != new.id() {
        return Err(ContinuationError::Binding);
    }
    if old_stream.events().len() != new_stream.events().len() {
        return Err(ContinuationError::Timeline);
    }
    for (a, b) in old_stream.events().iter().zip(new_stream.events()) {
        let old_note = match a.payload() {
            CompiledPayload::NoteOn { slot, .. }
            | CompiledPayload::NoteOff { slot, .. }
            | CompiledPayload::Expression { slot, .. }
            | CompiledPayload::Bend { slot, .. } => Some(slot),
            CompiledPayload::SetParameter { .. } | CompiledPayload::Controller(_) => None,
        };
        if old_note.is_some_and(|slot| !proof.notes.accepts_note(slot)) {
            return Err(ContinuationError::Timeline);
        }
        let payload = rebind_program(b.payload(), new, old)?;
        if a.position() != b.position()
            || a.payload() != payload
            || !same_values(a.payload(), payload)
        {
            return Err(ContinuationError::Timeline);
        }
        if let CompiledPayload::SetParameter { slot, .. } = a.payload() {
            let target = old
                .parameter_targets()
                .get(slot.index())
                .ok_or(ContinuationError::Binding)?;
            for address in old.note_addresses() {
                if proof.notes.accepts_note(address.slot) {
                    continue;
                }
                let note = old
                    .note_targets()
                    .get(address.slot.index())
                    .ok_or(ContinuationError::Binding)?;
                if note.node == target.node
                    || old
                        .note_magnitudes_of(address.slot)
                        .iter()
                        .any(|magnitude| magnitude.node == target.node)
                {
                    // Structural global note-driven DSP is fresh, never preserved.
                    // A parameter writer makes this destination active even without
                    // a note identity, so select the explicit reset path.
                    return Err(ContinuationError::Timeline);
                }
            }
        }
        if let CompiledPayload::SetParameter { slot, .. } = a.payload()
            && proof
                .trims
                .iter()
                .any(|change| change.old.index() == slot.index())
        {
            return Err(ContinuationError::Timeline);
        }
    }
    for target in old.note_magnitudes() {
        if proof
            .trims
            .iter()
            .any(|change| change.old.index() == target.parameter.index())
        {
            return Err(ContinuationError::Timeline);
        }
    }
    Ok(())
}

fn same_values(a: CompiledPayload, b: CompiledPayload) -> bool {
    match (a, b) {
        (
            CompiledPayload::SetParameter { value: a, .. },
            CompiledPayload::SetParameter { value: b, .. },
        ) => a.as_f32().to_bits() == b.as_f32().to_bits(),
        (
            CompiledPayload::NoteOn { velocity: a, .. },
            CompiledPayload::NoteOn { velocity: b, .. },
        ) => a.as_f32().to_bits() == b.as_f32().to_bits(),
        (CompiledPayload::Bend { cents: a, .. }, CompiledPayload::Bend { cents: b, .. }) => {
            a.as_f32().to_bits() == b.as_f32().to_bits()
        }
        (
            CompiledPayload::Expression { expression: a, .. },
            CompiledPayload::Expression { expression: b, .. },
        ) => a.value().as_f32().to_bits() == b.value().as_f32().to_bits(),
        (CompiledPayload::NoteOff { .. }, CompiledPayload::NoteOff { .. }) => true,
        _ => false,
    }
}

fn note_slot(
    slot: NoteSlot,
    from: &CompiledPlan,
    to: &CompiledPlan,
) -> Result<NoteSlot, ContinuationError> {
    if slot.plan() != from.id() || slot.index() >= from.note_targets().len() {
        return Err(ContinuationError::Binding);
    }
    let address = from
        .note_addresses()
        .iter()
        .find(|address| address.slot == slot)
        .ok_or(ContinuationError::Binding)?;
    to.resolve_note(address.node)
        .ok_or(ContinuationError::Timeline)
}
fn parameter_slot(
    slot: ParameterSlot,
    from: &CompiledPlan,
    to: &CompiledPlan,
) -> Result<ParameterSlot, ContinuationError> {
    if slot.plan() != from.id() || slot.index() >= from.parameter_targets().len() {
        return Err(ContinuationError::Binding);
    }
    // A ParameterSlot names a whole group. An interior ParameterRow must not
    // acquire group fan-out merely because its numeric index is in bounds.
    let address = from
        .parameter_addresses()
        .iter()
        .find(|address| address.slot == slot)
        .ok_or(ContinuationError::Binding)?;
    to.resolve_parameter(address.node, address.parameter)
        .ok_or(ContinuationError::Timeline)
}

pub(crate) fn rebind_program(
    payload: CompiledPayload,
    from: &CompiledPlan,
    to: &CompiledPlan,
) -> Result<CompiledPayload, ContinuationError> {
    Ok(match payload {
        CompiledPayload::SetParameter { slot, value } => CompiledPayload::SetParameter {
            slot: parameter_slot(slot, from, to)?,
            value,
        },
        CompiledPayload::NoteOn {
            slot,
            key,
            velocity,
        } => CompiledPayload::NoteOn {
            slot: note_slot(slot, from, to)?,
            key,
            velocity,
        },
        CompiledPayload::NoteOff { slot, key } => CompiledPayload::NoteOff {
            slot: note_slot(slot, from, to)?,
            key,
        },
        CompiledPayload::Expression {
            slot,
            key,
            expression,
        } => CompiledPayload::Expression {
            slot: note_slot(slot, from, to)?,
            key,
            expression,
        },
        CompiledPayload::Bend { slot, key, cents } => CompiledPayload::Bend {
            slot: note_slot(slot, from, to)?,
            key,
            cents,
        },
        CompiledPayload::Controller(_) => return Err(ContinuationError::Timeline),
    })
}

fn relabel(
    event: TimedEvent,
    from: &CompiledPlan,
    to: &CompiledPlan,
    witness: &AdoptedTimelineWitness,
    bindings: &crate::plan::note_binding::NoteBindings,
) -> Result<TimedEvent, ContinuationError> {
    if event.envelope().epoch() != witness.epoch
        || bindings.old_plan() != from.id()
        || bindings.new_plan() != to.id()
    {
        return Err(ContinuationError::Binding);
    }
    let checked = |identity: crate::identity::NoteIdentity| {
        if identity.table() == witness.table {
            Ok(identity)
        } else {
            Err(ContinuationError::Binding)
        }
    };
    let payload = match event.payload() {
        EventPayload::SetParameter { slot, value } => EventPayload::SetParameter {
            slot: parameter_slot(slot, from, to)?,
            value,
        },
        EventPayload::Note { identity, edge } => EventPayload::Note {
            identity: checked(identity)?,
            edge: match edge {
                NoteEdge::On {
                    slot,
                    key,
                    velocity,
                } => NoteEdge::On {
                    slot: bindings.note(slot).ok_or(ContinuationError::Binding)?,
                    key,
                    velocity,
                },
                NoteEdge::Off => NoteEdge::Off,
            },
        },
        EventPayload::Fade { identity, frames } => EventPayload::Fade {
            identity: checked(identity)?,
            frames,
        },
        EventPayload::Reset { identity } => EventPayload::Reset {
            identity: checked(identity)?,
        },
        EventPayload::Expression {
            identity,
            expression,
        } => EventPayload::Expression {
            identity: checked(identity)?,
            expression,
        },
        EventPayload::Bend { identity, cents } => EventPayload::Bend {
            identity: checked(identity)?,
            cents,
        },
        EventPayload::Controller(_)
        | EventPayload::RestoreController(_)
        | EventPayload::ScopedRestore(_)
        | EventPayload::ReleaseGroup(_) => return Err(ContinuationError::Binding),
    };
    Ok(TimedEvent::new(event.envelope(), payload))
}

impl SessionControl {
    pub(super) fn collect_continuing_replacement(
        &mut self,
        mut candidate: Box<PreparedPlanReplacement>,
        credit: usize,
        outcome: super::PlanReplacementOutcome,
    ) -> Result<
        super::PlanReplacementReceipt,
        (Box<PreparedPlanReplacement>, super::PlanReplacementError),
    > {
        use super::{PlanReplacementError, PlanReplacementOutcome, PlanReplacementReceipt};
        let snapshot = match outcome {
            PlanReplacementOutcome::Installed(snapshot)
            | PlanReplacementOutcome::Interrupted(snapshot) => snapshot,
            PlanReplacementOutcome::Refused(_) | PlanReplacementOutcome::Cancelled => {
                self.replacement_credits[credit] = None;
                return Ok(PlanReplacementReceipt {
                    id: candidate.id,
                    plan: candidate.plan,
                    outcome,
                    retired_compiled_notes: crate::quantities::HeldNoteCount::NONE,
                    observation: candidate.observation_outcome,
                });
            }
        };
        let valid = (|| {
            let continuation = candidate
                .continuation
                .as_ref()
                .ok_or(ContinuationError::Storage)?;
            let old = self
                .active_timeline
                .as_ref()
                .ok_or(ContinuationError::Unavailable)?;
            let new = continuation
                .witness
                .as_ref()
                .ok_or(ContinuationError::Storage)?;
            let mut new_origin = self.origin;
            new_origin.plan = continuation.proof.new;
            if candidate.outgoing_table != Some(self.origin.table)
                || continuation.proof.old != self.origin.plan
                || candidate.plan != continuation.proof.new
                || self.held != 0
                || self.acknowledged_serial != candidate.command_watermark
                || self
                    .replacement_credits
                    .iter()
                    .flatten()
                    .any(|id| *id != candidate.id)
                || !old.matches(
                    self.origin,
                    self.control.continuation_sequence(),
                    self.control.anchor(),
                )
                || !new.matches(
                    new_origin,
                    self.control.continuation_sequence(),
                    self.control.anchor(),
                )
                || old.events.len() != new.events.len()
            {
                return Err(ContinuationError::Storage);
            }
            Ok((continuation.proof.old, continuation.proof.new))
        })();
        let (from, to) = match valid {
            Ok(binding) => binding,
            Err(error) => {
                self.closed = true;
                return Err((candidate, PlanReplacementError::Continuation(error)));
            }
        };
        let Some(continuation) = candidate.continuation.as_ref() else {
            return Err((
                candidate,
                PlanReplacementError::Continuation(ContinuationError::Storage),
            ));
        };
        if let Err(error) = self.control.collect_continuation_plan(
            &mut candidate.control.control,
            from,
            to,
            &continuation.proof.notes,
        ) {
            self.closed = true;
            return Err((candidate, PlanReplacementError::Continuation(error)));
        }
        // No fallible work follows the authoritative minter's checked in-place rebind.
        if let Some(continuation) = candidate.continuation.as_mut() {
            core::mem::swap(&mut self.active_timeline, &mut continuation.witness);
        }
        core::mem::swap(&mut self.stream, &mut candidate.control.stream);
        self.origin.plan = to;
        self.acknowledged = snapshot;
        self.closed |= matches!(outcome, PlanReplacementOutcome::Interrupted(_));
        self.replacement_credits[credit] = None;
        Ok(PlanReplacementReceipt {
            id: candidate.id,
            plan: candidate.plan,
            outcome,
            retired_compiled_notes: crate::quantities::HeldNoteCount::NONE,
            observation: candidate.observation_outcome,
        })
    }
}

#[cfg(test)]
mod tests;
