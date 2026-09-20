//! Exclusive compiled-loop playback. Mutable note identity never leaves this owner.

mod hot;
pub mod journal;
mod prepare;
#[cfg(test)]
mod tests;

use crate::{
    diagnostics::RenderError,
    identity::{IdentityError, NoteIdentity, ProducerId},
    plan::{NoteSlot, PlanId},
    publish::{PublicationArbiter, PublicationFault},
    quantities::{Cents, HeldNoteCount, KeyIdentity, NoteVelocity, PreparedBytes},
    render::{EventPayload, PreparedRenderer},
    stream::StreamControl,
    time::{FrameCount, PlanPosition, SampleTime, StreamEpoch, TimeError},
    transport::LoopInterval,
};
use thiserror::Error;

/// A pass occurrence within one stream epoch, never a voice or storage index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct LoopPassId(u64);
impl LoopPassId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Entry and retained loop-storage budget, validated before preparing an owner.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct LoopSettings {
    interval: LoopInterval,
    entry: PlanPosition,
    storage_bytes: PreparedBytes,
}
impl LoopSettings {
    /// Entering exactly at the end starts the first pass at the start. Other
    /// positions outside the half-open interval are rejected, never clamped.
    pub fn new(
        interval: LoopInterval,
        entry: PlanPosition,
        storage_bytes: PreparedBytes,
    ) -> Result<Self, LoopPrepareError> {
        if entry < interval.start() || entry > interval.end() {
            return Err(LoopPrepareError::Entry);
        }
        Ok(Self {
            interval,
            entry: if entry == interval.end() {
                interval.start()
            } else {
                entry
            },
            storage_bytes,
        })
    }
}

#[derive(Debug, Error)]
pub enum LoopPrepareError {
    #[error("loop entry lies outside the interval")]
    Entry,
    #[error("loop profile does not match the compiled plan")]
    Profile,
    #[error("this loop owner requires StealingPolicy::None")]
    Stealing,
    #[error("loop preparation failed: {0}")]
    Preparation(String),
    #[error("loop {class:?} admission failed: {source}")]
    Admission {
        class: crate::publish::ProducerClass,
        #[source]
        source: crate::admit::AdmissionError,
    },
    #[error("loop storage needs {required:?}, above the declared {available:?}")]
    Storage {
        required: PreparedBytes,
        available: PreparedBytes,
    },
    #[error("loop storage or timeline is not representable")]
    Layout,
    #[error("loop preparation could not reserve storage")]
    Allocation,
    #[error("a normalized loop event has no matching occurrence")]
    Occurrence,
}

/// A render refusal or terminal fault. Output-shape refusal preserves the owner;
/// terminal causes are retained by `CompiledLoopStream::fault`.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum LoopFault {
    #[error("loop publication failed: {0}")]
    Publication(PublicationFault),
    #[error("loop render failed: {0}")]
    Render(RenderError),
    #[error("loop identity failed: {0}")]
    Identity(IdentityError),
    #[error("loop release failed: {0:?}")]
    Release(crate::identity::Resolution),
    #[error("loop time exhausted: {0}")]
    Time(TimeError),
    #[error("loop pass identities are exhausted")]
    PassExhausted,
    #[error("prepared loop routing or boundary storage is inconsistent")]
    PreparedState,
}

/// A sample-exact transition in the last successful render call. The borrow of
/// the owner's boundary slice must end before another call can replace it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopBoundary {
    pub epoch: StreamEpoch,
    pub at: SampleTime,
    pub previous: LoopPassId,
    pub next: LoopPassId,
    pub interval: LoopInterval,
}

/// The next render step, not a hardware timestamp or a delivered position.
/// Position equal to the exclusive loop end means that the wrap is still pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopSnapshot {
    pub epoch: StreamEpoch,
    pub plan: PlanId,
    pub clock: SampleTime,
    pub pass: LoopPassId,
    pub position: PlanPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct NoteToken(usize);

#[derive(Debug, Clone, Copy)]
enum LoopPayload {
    Direct(EventPayload),
    On {
        token: NoteToken,
        slot: NoteSlot,
        key: KeyIdentity,
        velocity: NoteVelocity,
    },
    Off(NoteToken),
    Expression(NoteToken, crate::controller::NoteExpression),
    Bend(NoteToken, Cents),
}

#[derive(Debug, Clone, Copy)]
struct LoopEvent {
    offset: FrameCount,
    payload: LoopPayload,
}

struct LoopProgram {
    start: PlanPosition,
    events: Box<[LoopEvent]>,
    catch_up: Box<[EventPayload]>,
    note_tokens: usize,
    end_held: HeldNoteCount,
    omissions: LoopOmissions,
}

/// A counted source-history transformation in one normalized pass template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopOmissionCount(usize);
impl LoopOmissionCount {
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

/// Cross-entry note events omitted by normalization, measured once per template.
/// Releases can retain their physical gate write while losing note identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct LoopOmissions {
    pub releases: LoopOmissionCount,
    pub expressions: LoopOmissionCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PassProgram {
    Entry,
    Repeating,
}

#[derive(Debug, Clone, Copy)]
struct HeldToken {
    token: NoteToken,
    identity: NoteIdentity,
}

/// One compiled-only loop. The control/minter, renderer and normalized programs
/// are private and cannot be split into independent transport authorities.
#[must_use]
pub struct CompiledLoopStream {
    source: LoopSource,
    renderer: PreparedRenderer,
    arbiter: PublicationArbiter,
    fault: Option<LoopFault>,
    storage_bytes: PreparedBytes,
}

struct LoopSource {
    control: StreamControl,
    producer: Option<ProducerId>,
    initial: LoopProgram,
    repeating: LoopProgram,
    program: PassProgram,
    cursor: usize,
    interval: LoopInterval,
    position: PlanPosition,
    pass: LoopPassId,
    started: bool,
    tokens: Box<[Option<NoteIdentity>]>,
    held: Box<[Option<HeldToken>]>,
    held_len: usize,
    boundaries: Box<[Option<LoopBoundary>]>,
    boundary_len: usize,
}

/// Private control of the exclusive loop owner; no external renderer/minter split.
#[derive(Clone, Copy)]
pub(crate) struct LoopRenderControl {
    pub playing: bool,
    pub finish: bool,
    pub idle_operations: crate::quantities::EventCount,
}

impl LoopRenderControl {
    pub(crate) const PLAY: Self = Self {
        playing: true,
        finish: false,
        idle_operations: crate::quantities::EventCount::NONE,
    };
}

impl CompiledLoopStream {
    #[cfg(feature = "simulated-ingress")]
    pub(crate) fn sample_rate(&self) -> crate::quantities::SampleRate {
        self.renderer.plan().sample_rate()
    }

    /// Source-history transformations in the initial suffix and each full pass.
    /// These are template counts, not a saturating lifetime render counter.
    pub const fn omissions(&self) -> (LoopOmissions, LoopOmissions) {
        (
            self.source.initial.omissions,
            self.source.repeating.omissions,
        )
    }

    pub const fn fault(&self) -> Option<LoopFault> {
        self.fault
    }

    /// Additional retained loop-program/routing heap charge; ordinary renderer,
    /// minter and publication admission remains covered by the supplied profile.
    pub const fn storage_bytes(&self) -> PreparedBytes {
        self.storage_bytes
    }
}
