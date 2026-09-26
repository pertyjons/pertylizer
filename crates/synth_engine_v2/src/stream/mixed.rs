//! A split-born mixed stream before ingress, activation or loop commands are enabled.
//!
//! This constructor consumes one bound plan and stream. Its two range minters share a
//! table identity but never share slots. It deliberately exposes no mixed render or
//! offer method while ADR-0075's release and restoration laws remain open.

use std::{marker::PhantomData, rc::Rc, sync::Arc};

use thiserror::Error;

use crate::{
    diagnostics::CompileError,
    host::mixed_targets::MixedTargetAdmission,
    identity::{
        CompiledRangeMinter, IdentityTable, LiveRangeMinter, NoteIdentity, ProducerId, TableId,
    },
    plan::{CompiledPlan, NoteSlot},
    render::{EventPayload, PreparedRenderer, TimedEvent},
    schedule::{AdmittedCompiledStream, SchedulePrepareError},
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

/// Why the bound first schedule could not be sealed before any mixed rendering exists.
#[derive(Debug, Error)]
pub enum MixedInitialPrepareError {
    /// Placement or compiled note stamping refused.
    #[error(transparent)]
    Schedule(#[from] SchedulePrepareError),
    /// A stealing policy or steal artifact appeared despite mixed no-stealing admission.
    #[error("mixed preparation encountered a stealing policy or steal artifact")]
    UnexpectedSteal,
    /// Stamping left a different number of held indices and recorded obligations.
    #[error("compiled range holds {live} notes but the schedule records {outstanding}")]
    OutstandingMismatch { live: u32, outstanding: usize },
    /// A stamped note edge escaped the compiled producer's checked range.
    #[error("stamped event {event_index} names {identity} outside the compiled range")]
    IdentityOutsideRange {
        event_index: usize,
        identity: NoteIdentity,
    },
}

/// An off-thread joined mixed stream, before its one bound initial stamp.
/// It cannot be transferred to the audio thread or split through its public API.
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedStream;
/// fn require_clone<T: Clone>() {}
/// require_clone::<MixedJoinedStream>();
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedStream;
/// fn split(owner: MixedJoinedStream) { let _ = owner.into_parts(); }
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedStream;
/// fn mutate(owner: &mut MixedJoinedStream) { let _ = owner.control_mut(); }
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedStream;
/// fn mutate(owner: &mut MixedJoinedStream) { let _ = owner.audio_mut(); }
/// ```
#[derive(Debug)]
#[must_use]
pub struct MixedJoinedStream {
    control: MixedStreamControl,
    audio: MixedStreamAudio,
    off_thread: PhantomData<Rc<()>>,
}

/// The sealed first schedule and both owners, still off-thread and not renderable.
/// No stamped event or mutable half escapes this value.
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedPrepared;
/// fn require_clone<T: Clone>() {}
/// require_clone::<MixedJoinedPrepared>();
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedPrepared;
/// fn events(prepared: &MixedJoinedPrepared) { let _ = prepared.events(); }
/// ```
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedJoinedPrepared;
/// fn require_send<T: Send>() {}
/// require_send::<MixedJoinedPrepared>();
/// ```
#[derive(Debug)]
#[must_use]
pub struct MixedJoinedPrepared {
    owner: MixedJoinedStream,
    events: Vec<TimedEvent>,
    outstanding: Vec<NoteIdentity>,
}

/// Off-thread compiled-range custody for one bound mixed plan and stream.
/// It exposes no independent schedule or activation builder.
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
    pub(crate) fn open(
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

    /// Check the bound stream against this control's compiled range off-thread.
    /// The copied identities and events are discarded; no schedule or reservation is published.
    pub(crate) fn check_bound_stamp(&self) -> Result<(), SchedulePrepareError> {
        let placed = crate::schedule::place_admitted(&self.stream, self.anchor)?;
        let mut minter = self.minter.working_copy();
        crate::schedule::stamp_all(&mut minter, &self.plan, self.epoch, &placed)?;
        Ok(())
    }
}

impl MixedJoinedStream {
    /// Construct the joined owner directly from one checked target binding.
    pub fn open(
        binding: MixedTargetAdmission,
        anchor: StreamAnchor,
    ) -> Result<Self, Box<(MixedStreamOpenError, MixedTargetAdmission)>> {
        let (control, audio) = MixedStreamControl::open(binding, anchor)?;
        Ok(Self {
            control,
            audio,
            off_thread: PhantomData,
        })
    }

    /// Read-only compiled custody for this joined owner.
    pub const fn control(&self) -> &MixedStreamControl {
        &self.control
    }

    /// Read-only live custody for this joined owner.
    pub const fn audio(&self) -> &MixedStreamAudio {
        &self.audio
    }

    /// Check the fixed compiled stream without creating any schedule or reservation.
    pub fn check_bound_stamp(&self) -> Result<(), SchedulePrepareError> {
        self.control.check_bound_stamp()
    }

    /// Seal the bound initial schedule exactly once, off-thread.
    /// A refusal returns this owner with its compiled minter unchanged.
    ///
    /// No separate plan, stream or anchor can be supplied:
    ///
    /// ```compile_fail
    /// use synth_engine_v2::stream::MixedJoinedStream;
    /// use synth_engine_v2::schedule::AdmittedCompiledStream;
    /// fn substitute(owner: MixedJoinedStream, stream: AdmittedCompiledStream) {
    ///     let _ = owner.prepare_initial(stream);
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use synth_engine_v2::stream::MixedJoinedStream;
    /// use synth_engine_v2::plan::CompiledPlan;
    /// fn substitute(owner: MixedJoinedStream, plan: CompiledPlan) {
    ///     let _ = owner.prepare_initial(plan);
    /// }
    /// ```
    pub fn prepare_initial(
        mut self,
    ) -> Result<MixedJoinedPrepared, Box<(MixedInitialPrepareError, Self)>> {
        if self.control.plan.stealing() != crate::ir::StealingPolicy::None {
            return Err(Box::new((MixedInitialPrepareError::UnexpectedSteal, self)));
        }
        let placed =
            match crate::schedule::place_admitted(&self.control.stream, self.control.anchor) {
                Ok(placed) => placed,
                Err(error) => return Err(Box::new((error.into(), self))),
            };
        let mut minter = self.control.minter.working_copy();
        let stamped = match crate::schedule::stamp_all(
            &mut minter,
            &self.control.plan,
            self.control.epoch,
            &placed,
        ) {
            Ok(stamped) => stamped,
            Err(error) => return Err(Box::new((error.into(), self))),
        };
        if stamped.released_after_steal != 0
            || stamped.expressions_after_steal != 0
            || stamped.events.iter().any(|event| {
                matches!(
                    event.payload(),
                    EventPayload::Fade { .. } | EventPayload::Reset { .. }
                )
            })
        {
            return Err(Box::new((MixedInitialPrepareError::UnexpectedSteal, self)));
        }
        if usize::try_from(minter.live()).ok() != Some(stamped.outstanding.len()) {
            return Err(Box::new((
                MixedInitialPrepareError::OutstandingMismatch {
                    live: minter.live(),
                    outstanding: stamped.outstanding.len(),
                },
                self,
            )));
        }
        let span = minter.span();
        for (event_index, event) in stamped.events.iter().enumerate() {
            let identity = match event.payload() {
                EventPayload::Note { identity, .. }
                | EventPayload::Expression { identity, .. }
                | EventPayload::Bend { identity, .. }
                | EventPayload::Fade { identity, .. }
                | EventPayload::Reset { identity } => Some(identity),
                EventPayload::ReleaseGroup(_)
                | EventPayload::Controller(_)
                | EventPayload::RestoreController(_)
                | EventPayload::SetParameter { .. } => None,
            };
            if let Some(identity) = identity
                && (identity.table() != minter.id() || !span.contains(identity.index()))
            {
                return Err(Box::new((
                    MixedInitialPrepareError::IdentityOutsideRange {
                        event_index,
                        identity,
                    },
                    self,
                )));
            }
        }
        self.control.minter = minter;
        Ok(MixedJoinedPrepared {
            owner: self,
            events: stamped.events,
            outstanding: stamped.outstanding,
        })
    }
}

impl MixedJoinedPrepared {
    /// The bound stream epoch carried by both retained owners.
    pub const fn epoch(&self) -> StreamEpoch {
        self.owner.control.epoch
    }

    /// The table shared by the compiled and live ranges.
    pub const fn table_id(&self) -> TableId {
        self.owner.control.minter.id()
    }

    /// How many stamped events remain privately owned by this prepared stream.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// How many compiled note-ons remain unpaired after the bound initial list.
    pub fn outstanding_count(&self) -> usize {
        self.outstanding.len()
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
