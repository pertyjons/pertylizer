//! A split-born mixed stream before ingress, activation or loop commands are enabled.
//!
//! This constructor consumes one bound plan and stream. Its two range minters share a
//! table identity but never share slots. It deliberately exposes no mixed render or
//! offer method while ADR-0075's release and restoration laws remain open.

use std::sync::Arc;

use thiserror::Error;

use crate::{
    diagnostics::CompileError,
    host::mixed_targets::MixedTargetAdmission,
    identity::{CompiledRangeMinter, IdentityTable, LiveRangeMinter, ProducerId, TableId},
    plan::{CompiledPlan, NoteSlot},
    render::PreparedRenderer,
    schedule::AdmittedCompiledStream,
    time::{StreamAnchor, StreamEpoch, issue_epoch},
};

/// Why a bound mixed stream could not be prepared.
#[derive(Debug, Error)]
pub enum MixedStreamOpenError {
    /// Ordinary renderer, epoch or identity preparation refused.
    #[error(transparent)]
    Compile(#[from] CompileError),
    /// The identity partition disagrees with the target binding.
    #[error("mixed producer spans differ from the admitted target binding")]
    Partition,
}

/// Off-thread compiled-range custody for one bound mixed plan and stream.
/// No schedule or activation can yet be built from it.
#[derive(Debug)]
#[must_use]
pub struct MixedStreamControl {
    epoch: StreamEpoch,
    anchor: StreamAnchor,
    plan: Arc<CompiledPlan>,
    stream: AdmittedCompiledStream,
    minter: CompiledRangeMinter,
    compiled_slots: Vec<NoteSlot>,
}

/// Audio-side live-range custody and renderer for a split-born mixed stream.
/// It exposes no mixed rendering until release and parameter ownership are proved.
#[derive(Debug)]
#[must_use]
pub struct MixedStreamAudio {
    renderer: PreparedRenderer,
    minter: LiveRangeMinter,
    note: NoteSlot,
}

impl MixedStreamControl {
    /// Prepare both split owners from one target binding before exposing either one.
    /// A refusal returns the binding so the caller can retry or retain the plan.
    pub fn open(
        binding: MixedTargetAdmission,
        anchor: StreamAnchor,
    ) -> Result<(Self, MixedStreamAudio), Box<(MixedStreamOpenError, MixedTargetAdmission)>> {
        if let Some(program) = binding
            .plan()
            .prepared_scripts()
            .iter()
            .find(|program| program.domain == crate::script::ScriptDomain::Note)
        {
            return Err(Box::new((
                MixedStreamOpenError::Compile(CompileError::Script {
                    node: program.node,
                    fault: crate::script::ScriptFault::NoteSourceRequired,
                }),
                binding,
            )));
        }
        let epoch = match issue_epoch() {
            Ok(epoch) => epoch,
            Err(error) => {
                return Err(Box::new((
                    MixedStreamOpenError::Compile(error.into()),
                    binding,
                )));
            }
        };
        let table = match IdentityTable::from_admitted_ranges(binding.plan().note_producer_ranges())
        {
            Ok(table) => table,
            Err(error) => {
                return Err(Box::new((
                    MixedStreamOpenError::Compile(error.into()),
                    binding,
                )));
            }
        };
        let table_id = table.id();
        let Ok((first, second)) = table.split_fresh_two() else {
            // This table was just constructed, so it holds no obligations if split refuses.
            return Err(Box::new((MixedStreamOpenError::Partition, binding)));
        };
        let (compiled, live) = if binding.live_producer() == ProducerId::new(0) {
            (second, first)
        } else {
            (first, second)
        };
        let compiled = CompiledRangeMinter::from_partition(compiled);
        let live = LiveRangeMinter::from_partition(live);
        let (compiled_span, live_span) = binding.spans();
        if compiled.span() != compiled_span
            || live.span() != live_span
            || live.producer() != binding.live_producer()
            || compiled.id() != table_id
            || live.id() != table_id
        {
            return Err(Box::new((MixedStreamOpenError::Partition, binding)));
        }
        let renderer = match PreparedRenderer::prepare(
            Arc::clone(binding.plan_arc()),
            anchor,
            epoch,
            table_id,
        ) {
            Ok(renderer) => renderer,
            Err(error) => {
                return Err(Box::new((MixedStreamOpenError::Compile(error), binding)));
            }
        };
        let (plan, stream, note, _producer, compiled_slots) = binding.into_parts();
        Ok((
            Self {
                epoch,
                anchor,
                plan,
                stream,
                minter: compiled,
                compiled_slots,
            },
            MixedStreamAudio {
                renderer,
                minter: live,
                note,
            },
        ))
    }

    /// The immutable plan checked by the target binding.
    pub fn plan(&self) -> &CompiledPlan {
        &self.plan
    }

    /// The only compiled stream this owner may later stamp.
    pub const fn stream(&self) -> &AdmittedCompiledStream {
        &self.stream
    }

    /// The stream epoch issued at construction.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// The initial transport anchor.
    pub const fn anchor(&self) -> StreamAnchor {
        self.anchor
    }

    /// The single table identity shared with the audio owner.
    pub const fn table_id(&self) -> TableId {
        self.minter.id()
    }

    /// The compiled producer this control alone may mint for.
    pub const fn compiled_producer(&self) -> ProducerId {
        self.minter.producer()
    }

    /// The note slots reached by the owned compiled stream.
    pub fn compiled_slots(&self) -> &[NoteSlot] {
        &self.compiled_slots
    }
}

impl MixedStreamAudio {
    /// The renderer and live owner share this epoch with the compiled control.
    pub const fn epoch(&self) -> StreamEpoch {
        self.renderer.epoch()
    }

    /// The same table identity as the compiled control.
    pub const fn table_id(&self) -> TableId {
        self.minter.id()
    }

    /// The live producer this audio owner alone may mint for.
    pub const fn live_producer(&self) -> ProducerId {
        self.minter.producer()
    }

    /// The one live note target proven disjoint from compiled targets.
    pub const fn live_slot(&self) -> NoteSlot {
        self.note
    }
}
