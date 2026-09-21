//! Two-slot simulated live plan replacement. Preparation and collection are off-thread.
use super::{LiveInputError, LiveInputStream};
use crate::{
    compile::{RenderConfig, compile},
    host::ConnectionGeneration,
    ir::{GraphIr, NodeId, ParameterId},
    plan::PlanId,
    profile::HostProfile,
    quantities::{EventCount, PreparedBytes},
    report::{ResourceAmount, ResourceField},
    time::SampleTime,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapOutcome {
    Installed { plan: PlanId, at: SampleTime },
    Cancelled,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SwapState {
    Vacant,
    Pending,
    Fading,
    Retired,
}
#[must_use]
pub struct SwappingLiveStream {
    pub(super) active: LiveInputStream,
    pub(super) secondary: Option<LiveInputStream>,
    pub(super) state: SwapState,
    pub(super) profile: HostProfile,
    pub(super) ceiling: PreparedBytes,
    pub(super) active_bytes: PreparedBytes,
    pub(super) secondary_bytes: PreparedBytes,
    pub(super) scratch: Box<[f32]>,
    pub(super) fade_position: usize,
    pub(super) outcome: Option<SwapOutcome>,
    pub(super) failed: bool,
}
pub(super) fn build(
    graph: &GraphIr,
    profile: HostProfile,
    note: NodeId,
    sources: &[ConnectionGeneration],
    parameters: &[(NodeId, ParameterId)],
    capacity: EventCount,
    ceiling: PreparedBytes,
) -> Result<(LiveInputStream, PreparedBytes), LiveInputError> {
    let compiled = compile(graph, &RenderConfig::new(profile));
    let mut bytes = 0_u64;
    for field in [
        ResourceField::PreparedImmutableBytes,
        ResourceField::MutableStateBytes,
        ResourceField::BufferScratchBytes,
    ] {
        let Some(ResourceAmount::Bytes(amount)) =
            compiled.report().row(field).map(|row| row.requested())
        else {
            return Err(LiveInputError::Configuration);
        };
        bytes = bytes
            .checked_add(amount.get())
            .ok_or(LiveInputError::Bytes)?;
    }
    let plan = compiled.into_plan().map_err(super::preparation)?;
    let slots: Vec<_> = parameters
        .iter()
        .map(|(node, parameter)| {
            plan.resolve_parameter(*node, *parameter)
                .ok_or(LiveInputError::Configuration)
        })
        .collect::<Result<_, _>>()?;
    let note = plan
        .resolve_note(note)
        .ok_or(LiveInputError::Configuration)?;
    let available = ceiling
        .get()
        .checked_sub(bytes)
        .ok_or(LiveInputError::Bytes)?;
    let mut live = LiveInputStream::prepare(
        plan,
        profile,
        note,
        sources,
        capacity,
        PreparedBytes::measured(available),
    )?;
    live.prepare_parameters(&slots, PreparedBytes::measured(available))?;
    let bytes = bytes
        .checked_add(live.bytes().get())
        .ok_or(LiveInputError::Bytes)?;
    Ok((live, PreparedBytes::measured(bytes)))
}
/// Off-thread prepared payload. Transport owns a fixed number of these values.
#[must_use]
pub struct PreparedLivePlan {
    pub(super) fresh: bool,
    pub(super) live: LiveInputStream,
    pub(super) bytes: PreparedBytes,
    pub(super) profile: HostProfile,
}
impl PreparedLivePlan {
    pub fn prepare(
        graph: &GraphIr,
        profile: HostProfile,
        note: NodeId,
        sources: &[ConnectionGeneration],
        parameters: &[(NodeId, ParameterId)],
        capacity: EventCount,
        ceiling: PreparedBytes,
    ) -> Result<Self, LiveInputError> {
        let (live, bytes) = build(graph, profile, note, sources, parameters, capacity, ceiling)?;
        Ok(Self {
            fresh: true,
            live,
            bytes,
            profile,
        })
    }
    pub fn plan_id(&self) -> PlanId {
        self.live.plan_id()
    }
}
impl SwappingLiveStream {
    pub fn prepare(
        graph: &GraphIr,
        profile: HostProfile,
        note: NodeId,
        sources: &[ConnectionGeneration],
        parameters: &[(NodeId, ParameterId)],
        capacity: EventCount,
        ceiling: PreparedBytes,
    ) -> Result<Self, LiveInputError> {
        let samples = profile
            .capabilities()
            .channel_layout()
            .channels()
            .checked_mul(crate::time::QUANTUM_FRAMES as usize)
            .ok_or(LiveInputError::Bytes)?;
        let overhead = samples
            .checked_mul(size_of::<f32>())
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(LiveInputError::Bytes)?;
        let (active, active_bytes) = build(
            graph,
            profile,
            note,
            sources,
            parameters,
            capacity,
            PreparedBytes::measured(
                ceiling
                    .get()
                    .checked_sub(overhead)
                    .ok_or(LiveInputError::Bytes)?,
            ),
        )?;
        Ok(Self {
            active,
            secondary: None,
            state: SwapState::Vacant,
            profile,
            ceiling,
            active_bytes,
            secondary_bytes: PreparedBytes::measured(0),
            scratch: vec![0.0; samples].into_boxed_slice(),
            fade_position: 0,
            outcome: None,
            failed: false,
        })
    }
    /// No externally owned candidate exists. An occupied secondary slot refuses BEFORE
    /// compilation, including when a completed retirement has not yet been collected.
    pub fn prepare_candidate(
        &mut self,
        graph: &GraphIr,
        note: NodeId,
        parameters: &[(NodeId, ParameterId)],
    ) -> Result<PlanId, LiveInputError> {
        if self.state != SwapState::Vacant
            || self.active.closed
            || self.active.end.is_some()
            || self
                .active
                .entries
                .iter()
                .flatten()
                .any(|entry| entry.outcome.is_none())
        {
            return Err(LiveInputError::Busy);
        }
        let overhead = u64::try_from(self.scratch.len() * size_of::<f32>() + size_of::<Self>())
            .map_err(|_| LiveInputError::Bytes)?;
        let available = self
            .ceiling
            .get()
            .checked_sub(self.active_bytes.get())
            .and_then(|n| n.checked_sub(overhead))
            .ok_or(LiveInputError::Bytes)?;
        let sources: Vec<_> = self
            .active
            .sources
            .iter()
            .map(|source| source.generation)
            .collect();
        let capacity = EventCount::measured(
            u32::try_from(self.active.entries.len()).map_err(|_| LiveInputError::Bytes)?,
        );
        let (candidate, bytes) = build(
            graph,
            self.profile,
            note,
            &sources,
            parameters,
            capacity,
            PreparedBytes::measured(available),
        )?;
        let plan = candidate.control.plan_id();
        self.secondary = Some(candidate);
        self.secondary_bytes = bytes;
        self.state = SwapState::Pending;
        self.outcome = None;
        Ok(plan)
    }
    /// The observer borrows the retirement; destruction finishes here before a new
    /// candidate credit exists. Calling this method on an audio callback is forbidden.
    pub fn collect_retired(&mut self, mut receive: impl FnMut(&LiveInputStream)) -> bool {
        if self.state != SwapState::Retired {
            return false;
        }
        if let Some(owner) = self.secondary.take() {
            receive(&owner);
        }
        self.secondary_bytes = PreparedBytes::measured(0);
        self.state = SwapState::Vacant;
        true
    }
    pub const fn swap_outcome(&self) -> Option<SwapOutcome> {
        self.outcome
    }
    pub fn active(&self) -> &LiveInputStream {
        &self.active
    }
}
