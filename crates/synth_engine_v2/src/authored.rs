//! Prepared, non-dropping Note YAMS source. One exclusive owner, no runtime input queue.
use crate::{
    identity::{NoteIdentity, ProducerId},
    ir::NodeId,
    plan::{CompiledPlan, NodeSlot, NoteSlot},
    profile::HostProfile,
    quantities::{EventCount, KeyIdentity, NoteVelocity},
    render::PreparedRenderer,
    schedule::{AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload},
    script::ScriptDomain,
    stream::StreamControl,
    tempo::{MusicalTick, TempoMap},
    time::{Located, SampleTime, StreamAnchor},
};
use thiserror::Error;

mod hot;
mod qualification;

/// Stable authored occurrence identity, independent of list or voice position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct AuthoredOccurrenceId(u64);
impl AuthoredOccurrenceId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}
/// Pattern-relative VM tick, exactly representable by the language's scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ScriptTick(u16);
impl ScriptTick {
    pub const fn new(value: u16) -> Self {
        Self(value)
    }
    pub fn as_f32(self) -> f32 {
        f32::from(self.0)
    }
}
/// Finite Note duration domain in integer ticks. `None` separately means until cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct ScriptDuration(u16);
impl ScriptDuration {
    pub const fn new(value: u16) -> Self {
        Self(value)
    }
    pub const fn as_u16(self) -> u16 {
        self.0
    }
    pub fn as_f32(self) -> f32 {
        f32::from(self.0)
    }
}
/// Raw authored input, not a compiled note event. Every input has an explicit final cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthoredNote {
    pub occurrence: AuthoredOccurrenceId,
    pub start: MusicalTick,
    pub tick: ScriptTick,
    pub duration: Option<ScriptDuration>,
    pub cut: MusicalTick,
    pub key: KeyIdentity,
    pub velocity: NoteVelocity,
}
/// Complete immutable input and mapping supplied before playback.
#[derive(Debug)]
pub struct AuthoredNoteSource {
    pub script: NodeId,
    pub producer: ProducerId,
    pub target: NoteSlot,
    pub maximum_duration: ScriptDuration,
    pub tempo: TempoMap,
    pub notes: Vec<AuthoredNote>,
}
/// Actual high-water counts, recorded by the source rather than inferred from publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthoredUsage {
    pub pending_inputs: EventCount,
    pub held_releases: EventCount,
    pub future_events: EventCount,
    pub reserved_obligations: EventCount,
    pub evaluations_per_quantum: EventCount,
    pub authored_per_quantum: EventCount,
    pub releases_per_quantum: EventCount,
}
impl Default for AuthoredUsage {
    fn default() -> Self {
        Self {
            pending_inputs: EventCount::NONE,
            held_releases: EventCount::NONE,
            future_events: EventCount::NONE,
            reserved_obligations: EventCount::NONE,
            evaluations_per_quantum: EventCount::NONE,
            authored_per_quantum: EventCount::NONE,
            releases_per_quantum: EventCount::NONE,
        }
    }
}
/// One generated note's stable trace; no stream-local identity leaks into comparisons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeneratedNote {
    pub occurrence: AuthoredOccurrenceId,
    pub start: SampleTime,
    pub end: SampleTime,
    pub key: KeyIdentity,
    pub velocity: NoteVelocity,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Pending,
    Held(NoteIdentity, bool),
    Complete,
}
#[derive(Debug, Clone, Copy)]
struct Occurrence {
    raw: AuthoredNote,
    start: SampleTime,
    cut: SampleTime,
    phase: Phase,
    generated: Option<GeneratedNote>,
}

#[derive(Debug, Error)]
pub enum AuthoredPrepareError {
    #[error("the real authored producer has not yet been qualified")]
    Unqualified,
    #[error(
        "the plan must have exactly one shared Note script and one authored, non-stealing note producer"
    )]
    SourceShape,
    #[error("the supplied profile differs from the admitted plan's publication configuration")]
    Profile,
    #[error(
        "the complete source exceeds its declared input, destination, retained, or hold envelope"
    )]
    Capacity,
    #[error(
        "input {index} has a duplicate occurrence, unordered start, invalid duration or missing forward cut"
    )]
    Input { index: usize },
    #[error("the source's tempo rate differs from the plan or its map exceeds 32 segments")]
    TempoShape,
    #[error("input {index} has a non-monotone, unrepresentable or pre-anchor mapped position")]
    Time { index: usize },
    #[error(
        "automation must be an admitted stream for this plan containing only parameter/controller writes"
    )]
    Automation,
    #[error(transparent)]
    Compile(#[from] crate::diagnostics::CompileError),
    #[error(transparent)]
    Schedule(#[from] crate::schedule::SchedulePrepareError),
    #[error(transparent)]
    ProfileBuild(#[from] crate::profile::ProfileError),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthoredFault {
    #[error("an event missed its exact destination")]
    Late,
    #[error("a Note output was non-finite or outside its prepared mapping")]
    Output,
    #[error("an admitted identity or release obligation could not be acquired or redeemed")]
    Identity,
    #[error("the shared publication failed its contract")]
    Publication,
    #[error("the prepared Note context is missing")]
    Context,
    #[error("the renderer refused a batch after producer state advanced")]
    RendererRefused,
}
#[derive(Debug, Error)]
pub enum AuthoredRenderError {
    #[error("{producer}: terminal authored source fault: {fault}")]
    Source {
        producer: ProducerId,
        fault: AuthoredFault,
    },
    #[error(transparent)]
    Render(#[from] crate::diagnostics::RenderError),
}
/// Owns both stream halves and the only arbiter. There is no activation or offer interface.
#[derive(Debug)]
pub struct AuthoredNoteStream {
    control: StreamControl,
    renderer: PreparedRenderer,
    arbiter: crate::publish::PublicationArbiter,
    automation: CompiledEventScheduler,
    producer: ProducerId,
    target: NoteSlot,
    script_node: NodeSlot,
    script: crate::script::ScriptSlot,
    maximum_duration: ScriptDuration,
    tempo: TempoMap,
    occurrences: Box<[Occurrence]>,
    usage: AuthoredUsage,
    fault: Option<AuthoredFault>,
}
impl AuthoredNoteStream {
    pub fn prepare(
        plan: CompiledPlan,
        profile: &HostProfile,
        anchor: StreamAnchor,
        source: AuthoredNoteSource,
        automation: &AdmittedCompiledStream,
    ) -> Result<Self, AuthoredPrepareError> {
        if !qualification::QUALIFIED {
            return Err(AuthoredPrepareError::Unqualified);
        }
        Self::prepare_source(plan, profile, anchor, source, automation)
    }
    pub(crate) fn prepare_source(
        plan: CompiledPlan,
        profile: &HostProfile,
        anchor: StreamAnchor,
        source: AuthoredNoteSource,
        automation: &AdmittedCompiledStream,
    ) -> Result<Self, AuthoredPrepareError> {
        let programs: Vec<_> = plan
            .prepared_scripts()
            .iter()
            .enumerate()
            .filter(|(_, p)| p.domain == ScriptDomain::Note)
            .collect();
        let [(script_index, program)] = programs.as_slice() else {
            return Err(AuthoredPrepareError::SourceShape);
        };
        let [declaration] = plan.authored_sources() else {
            return Err(AuthoredPrepareError::SourceShape);
        };
        if program.node != source.script
            || declaration.producer != source.producer
            || plan.note_producer_ranges().len() != 1
            || source.producer != ProducerId::new(0)
            || plan.compiled_note_producer().is_some()
            || plan.stealing().steals()
            || source.target.plan() != plan.id()
            || plan.note_targets().get(source.target.index()).is_none()
        {
            return Err(AuthoredPrepareError::SourceShape);
        }
        let script = crate::script::ScriptSlot::new(*script_index);
        let script_node = plan.ops().iter().find_map(|op| match op {
            crate::plan::PlanOp::Node(step) if matches!(plan.prepared_nodes().get(step.prepared().index()), Some(crate::node::kernels::PreparedNode::Script { program, .. }) if *program == script) => Some(step.node()),
            _ => None,
        }).ok_or(AuthoredPrepareError::SourceShape)?;
        let count =
            u32::try_from(source.notes.len()).map_err(|_| AuthoredPrepareError::Capacity)?;
        let doubled = count.checked_mul(2).ok_or(AuthoredPrepareError::Capacity)?;
        let target_instances = plan
            .note_targets()
            .get(source.target.index())
            .and_then(|target| plan.parameter_targets().get(target.parameter.index()))
            .map_or(0, |target| target.instances.get());
        if target_instances < count {
            return Err(AuthoredPrepareError::Capacity);
        }

        if count > declaration.simultaneous_holds.get()
            || count > plan.note_producer_ranges().first().map_or(0, |n| n.get())
            || doubled > declaration.retained_future.get()
            || doubled > declaration.destination_occupancy.get()
        {
            return Err(AuthoredPrepareError::Capacity);
        }
        if source.tempo.sample_rate() != plan.sample_rate() || source.tempo.segment_count() > 32 {
            return Err(AuthoredPrepareError::TempoShape);
        }
        if automation.plan() != plan.id()
            || automation.events().iter().any(|event| {
                !matches!(
                    event.payload(),
                    CompiledPayload::SetParameter { .. } | CompiledPayload::Controller(_)
                )
            })
        {
            return Err(AuthoredPrepareError::Automation);
        }
        let arbiter = crate::publish::PublicationArbiter::prepare(profile)?;
        if arbiter.max_events_per_quantum() != plan.max_events_per_quantum()
            || profile.capabilities().maximum_block_size() != plan.maximum_block_size()
            || crate::publish::ProducerClass::Compiled.share_of(profile)
                != plan.compiled_event_share()
            || crate::publish::ProducerClass::AuthoredRuntime.share_of(profile)
                < declaration.destination_occupancy
            || crate::publish::ProducerClass::Release.share_of(profile)
                < declaration.simultaneous_holds
        {
            return Err(AuthoredPrepareError::Profile);
        }
        let mut occurrences = Vec::with_capacity(source.notes.len());
        for (index, raw) in source.notes.iter().copied().enumerate() {
            if raw.cut <= raw.start
                || raw.duration.is_some_and(|d| d > source.maximum_duration)
                || source.notes[..index]
                    .iter()
                    .any(|earlier| earlier.occurrence == raw.occurrence)
                || source
                    .notes
                    .get(index.wrapping_sub(1))
                    .is_some_and(|earlier| earlier.start > raw.start)
            {
                return Err(AuthoredPrepareError::Input { index });
            }
            let locate = |tick| {
                source.tempo.position_of(tick).ok().and_then(|position| {
                    match anchor.locate(position) {
                        Located::At(time) => Some(time),
                        _ => None,
                    }
                })
            };
            let start = locate(raw.start).ok_or(AuthoredPrepareError::Time { index })?;
            let cut = locate(raw.cut)
                .filter(|cut| *cut > start)
                .ok_or(AuthoredPrepareError::Time { index })?;
            let mut previous = start;
            // Exhaust every representable VM result; a ramp's rounded law is not assumed monotone.
            for duration in 0..=source.maximum_duration.as_u16() {
                let tick = raw
                    .start
                    .as_u64()
                    .checked_add(u64::from(duration))
                    .map(MusicalTick::new)
                    .ok_or(AuthoredPrepareError::Time { index })?;
                let end = locate(tick)
                    .filter(|end| *end >= previous)
                    .ok_or(AuthoredPrepareError::Time { index })?;
                previous = end;
            }
            if occurrences
                .last()
                .is_some_and(|earlier: &Occurrence| earlier.start > start)
            {
                return Err(AuthoredPrepareError::Time { index });
            }
            occurrences.push(Occurrence {
                raw,
                start,
                cut,
                phase: Phase::Pending,
                generated: None,
            });
        }
        let (mut control, renderer) = StreamControl::open_authored(plan, anchor)?;
        let automation = CompiledEventScheduler::prepare(&mut control, automation)?;
        Ok(Self {
            control,
            renderer,
            arbiter,
            automation,
            producer: source.producer,
            target: source.target,
            script_node,
            script,
            maximum_duration: source.maximum_duration,
            tempo: source.tempo,
            occurrences: occurrences.into_boxed_slice(),
            usage: AuthoredUsage {
                pending_inputs: EventCount::measured(count),
                reserved_obligations: EventCount::measured(count),
                ..AuthoredUsage::default()
            },
            fault: None,
        })
    }
    pub const fn usage(&self) -> AuthoredUsage {
        self.usage
    }
    pub const fn fault(&self) -> Option<AuthoredFault> {
        self.fault
    }
    pub fn generated_notes(&self) -> impl Iterator<Item = GeneratedNote> + '_ {
        self.occurrences.iter().filter_map(|entry| entry.generated)
    }
    pub fn diagnostics(&self) -> &crate::diagnostics::DiagnosticsReport {
        self.renderer.diagnostics()
    }
    pub fn source_bytes(&self) -> usize {
        std::mem::size_of_val(self.occurrences.as_ref()) + self.tempo.bytes_held()
    }
}

#[cfg(test)]
mod tests;
