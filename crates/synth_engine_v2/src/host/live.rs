//! Bounded simulated MIDI audition through the real ingress, identity and voice path.
//! This exclusive immutable-plan owner has no activation or voice-stealing authority.
mod hot;
mod parameter;
mod parameter_hot;
mod swap;
mod swap_hot;
use crate::{
    host::ConnectionGeneration,
    identity::NoteIdentity,
    ingress::{IngressRefused, PerformanceIngress, ReleaseCause},
    plan::{CompiledPlan, NoteSlot},
    profile::HostProfile,
    publish::PublicationArbiter,
    quantities::{EventCount, KeyIdentity, PreparedBytes},
    recording::notes::Midi1Input,
    render::PreparedRenderer,
    schedule::ScheduledRenderError,
    stream::StreamControl,
    time::{SampleTime, StreamEpoch},
};
pub use parameter::{UpdateStatus, UpdateVersion};
pub use swap::{PreparedLivePlan, SwapOutcome, SwappingLiveStream};
use synth_core::MidiChannel;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct AuditionId {
    source: ConnectionGeneration,
    serial: u64,
}
impl AuditionId {
    pub fn new(source: ConnectionGeneration, serial: u64) -> Result<Self, LiveInputError> {
        if serial == 0 {
            return Err(LiveInputError::Identity);
        }
        Ok(Self { source, serial })
    }
    pub const fn source(self) -> ConnectionGeneration {
        self.source
    }
    pub const fn serial(self) -> u64 {
        self.serial
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditionOutcome {
    Executed { epoch: StreamEpoch, at: SampleTime },
    Refused(IngressRefused),
    Unsupported,
    UnmatchedRelease,
    NotSounded,
    Cancelled,
}
#[derive(Debug, Error)]
pub enum LiveInputError {
    #[error("live plan preparation or retirement is still outstanding")]
    Busy,
    #[error("live audition has closed")]
    Closed,
    #[error("invalid or repeated audition identity")]
    Identity,
    #[error("audition outstanding-cell capacity exceeded")]
    Capacity,
    #[error("live audition requires one non-stealing producer with one to eight voices")]
    Configuration,
    #[error("live audition output shape differs from preparation")]
    Shape,
    #[error("live audition preparation failed: {0}")]
    Preparation(String),
    #[error("live audition storage exceeds its preparation budget")]
    Bytes,
    #[error("live audition storage allocation failed")]
    Allocation,
    #[error(transparent)]
    Render(#[from] ScheduledRenderError),
    #[error(transparent)]
    Ingress(#[from] IngressRefused),
}
#[derive(Clone, Copy)]
struct Entry {
    id: AuditionId,
    nominal: SampleTime,
    input: Midi1Input,
    staged: Option<AuditionOutcome>,
    outcome: Option<AuditionOutcome>,
}
#[derive(Clone, Copy)]
struct Held {
    id: AuditionId,
    channel: MidiChannel,
    key: KeyIdentity,
    down: bool,
    identity: Option<NoteIdentity>,
}
#[derive(Clone, Copy)]
struct PreviewHeld {
    id: AuditionId,
    channel: MidiChannel,
    key: KeyIdentity,
    down: bool,
    identity_possible: bool,
}
struct Source {
    generation: ConnectionGeneration,
    pedal: [bool; 16],
    serial: u64,
    bend: [crate::quantities::Cents; 16],
}

#[must_use]
pub struct LiveInputStream {
    control: StreamControl,
    renderer: PreparedRenderer,
    parameters: Box<[parameter::ParameterCell]>,
    parameter_indices: Box<[Option<usize>]>,
    failed: bool,
    arbiter: PublicationArbiter,
    ingress: PerformanceIngress,
    note: NoteSlot,
    entries: Box<[Option<Entry>]>,
    held: Box<[Option<Held>]>,
    preview: Box<[Option<PreviewHeld>]>,
    sources: Box<[Source]>,
    end: Option<(SampleTime, ReleaseCause)>,
    closed: bool,
    bytes: PreparedBytes,
}
fn preparation(error: impl std::fmt::Display) -> LiveInputError {
    LiveInputError::Preparation(error.to_string())
}
fn slots<T>(count: usize) -> Result<Box<[Option<T>]>, LiveInputError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| LiveInputError::Allocation)?;
    if values.capacity() != count {
        return Err(LiveInputError::Bytes);
    }
    values.resize_with(count, || None);
    Ok(values.into_boxed_slice())
}
impl LiveInputStream {
    /// `quota` bounds outstanding observations and unmatched key occurrences.
    /// Taking a completed outcome returns its cell; source identity high-water marks persist.
    /// The caller also admits compiled DSP resources. Retained plan table capacities,
    /// publication and ingress storage are included here.
    pub fn prepare(
        plan: CompiledPlan,
        profile: HostProfile,
        note: NoteSlot,
        sources: &[ConnectionGeneration],
        quota: EventCount,
        ceiling: PreparedBytes,
    ) -> Result<Self, LiveInputError> {
        let caps = profile.capabilities();
        if plan.sample_rate() != caps.sample_rate()
            || plan.channel_layout() != caps.channel_layout()
            || plan.maximum_block_size() != caps.maximum_block_size()
            || plan.max_events_per_quantum() != profile.limits().events().max_events_per_quantum()
            || plan.forward_event_horizon() != profile.limits().events().forward_event_horizon()
        {
            return Err(LiveInputError::Configuration);
        }
        let ranges = plan.note_producer_ranges();
        if ranges.len() != 1
            || !(1..=8).contains(&ranges[0].get())
            || plan.compiled_note_producer().is_some()
            || plan.stealing() != crate::ir::StealingPolicy::None
            || note.plan() != plan.id()
            || sources.is_empty()
            || quota == EventCount::NONE
            || sources
                .iter()
                .enumerate()
                .any(|(i, source)| sources[..i].contains(source))
        {
            return Err(LiveInputError::Configuration);
        }
        let quota = usize::try_from(quota.get()).map_err(|_| LiveInputError::Bytes)?;
        let table_bytes = plan.host_table_bytes().ok_or(LiveInputError::Bytes)?;
        let metadata = quota
            .checked_mul(
                size_of::<Option<Entry>>()
                    + size_of::<Option<Held>>()
                    + size_of::<Option<PreviewHeld>>(),
            )
            .and_then(|bytes| {
                sources
                    .len()
                    .checked_mul(size_of::<Source>())
                    .and_then(|more| bytes.checked_add(more))
            })
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .and_then(|bytes| bytes.checked_add(table_bytes))
            .ok_or(LiveInputError::Bytes)?;
        if metadata > ceiling.get() {
            return Err(LiveInputError::Bytes);
        }
        let (mut control, renderer) = StreamControl::open(
            plan,
            crate::time::StreamAnchor::new(SampleTime::ZERO, crate::time::PlanPosition::ZERO),
        )
        .map_err(preparation)?;
        let arbiter = PublicationArbiter::prepare_one_quantum(&profile).map_err(preparation)?;
        let mut ingress = PerformanceIngress::prepare(
            &profile,
            control.plan(),
            crate::identity::ProducerId::new(0),
            &renderer,
        )
        .map_err(preparation)?;
        control.latch_store(&mut ingress)?;
        let bytes = PreparedBytes::measured(
            metadata
                .checked_add(ingress.storage_bytes().get())
                .and_then(|n| n.checked_add(arbiter.storage_bytes()?.get()))
                .ok_or(LiveInputError::Bytes)?,
        );
        if bytes > ceiling {
            return Err(LiveInputError::Bytes);
        }
        Ok(Self {
            control,
            renderer,
            parameters: Box::default(),
            parameter_indices: Box::default(),
            failed: false,
            arbiter,
            ingress,
            note,
            entries: slots(quota)?,
            held: slots(quota)?,
            preview: slots(quota)?,
            sources: sources
                .iter()
                .map(|generation| Source {
                    generation: *generation,
                    pedal: [false; 16],
                    serial: 0,
                    bend: [crate::quantities::Cents::ZERO; 16],
                })
                .collect(),
            end: None,
            closed: false,
            bytes,
        })
    }
    pub const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
    pub const fn epoch(&self) -> StreamEpoch {
        self.renderer.epoch()
    }
    pub const fn clock(&self) -> SampleTime {
        self.renderer.clock()
    }
    pub fn outcomes(&self) -> impl Iterator<Item = (AuditionId, AuditionOutcome)> + '_ {
        self.entries
            .iter()
            .flatten()
            .filter_map(|entry| entry.outcome.map(|outcome| (entry.id, outcome)))
    }
}
