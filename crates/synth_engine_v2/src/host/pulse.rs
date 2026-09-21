//! Finite, prepared metronome pulses through an exclusive Core V2 parameter renderer.
mod hot;
use crate::{
    plan::{CompiledPlan, ParameterSlot},
    profile::HostProfile,
    publish::PublicationArbiter,
    quantities::{ParameterValue, PreparedBytes},
    render::{EventEnvelope, EventPayload, PreparedRenderer, TimedEvent},
    stream::StreamControl,
    time::{FrameCount, PlanPosition, SampleTime, StreamAnchor, TimeSource},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PulseError {
    #[error("invalid metronome geometry, gate, pulse order, duration or density")]
    Configuration,
    #[error("metronome preparation failed: {0}")]
    Preparation(String),
    #[error("metronome storage exceeds its admitted ceiling")]
    Bytes,
    #[error(transparent)]
    Render(#[from] crate::diagnostics::RenderError),
    #[error(transparent)]
    Publication(#[from] crate::publish::PublicationFault),
}
#[must_use]
pub struct PulseStream {
    _control: StreamControl,
    renderer: PreparedRenderer,
    arbiter: PublicationArbiter,
    gate: ParameterSlot,
    edges: Box<[TimedEvent]>,
    next: usize,
    end: Option<SampleTime>,
    closed: bool,
    bytes: PreparedBytes,
}
impl PulseStream {
    /// Pulses are immutable engine timeline positions. One extra Session cell in
    /// every destination quantum is reserved for an ordered stop's gate-down.
    /// The caller charges the compiled plan and renderer in addition to `bytes()`.
    pub fn prepare(
        plan: CompiledPlan,
        profile: HostProfile,
        gate: ParameterSlot,
        pulses: &[SampleTime],
        width: FrameCount,
        ceiling: PreparedBytes,
    ) -> Result<Self, PulseError> {
        let caps = profile.capabilities();
        if width == FrameCount::ZERO
            || gate.plan() != plan.id()
            || !plan.parameter_addresses().iter().any(|address| {
                address.slot == gate && address.parameter == crate::ir::parameters::ENVELOPE_GATE
            })
            || plan
                .parameter_targets()
                .get(gate.index())
                .is_none_or(|target| {
                    target.instances.get() != 1 || target.rate != crate::plan::ControlRate::Sample
                })
            || !plan.note_producer_ranges().is_empty()
            || plan.sample_rate() != caps.sample_rate()
            || plan.channel_layout() != caps.channel_layout()
            || plan.maximum_block_size() != caps.maximum_block_size()
            || plan.max_events_per_quantum() != profile.limits().events().max_events_per_quantum()
        {
            return Err(PulseError::Configuration);
        }
        let arbiter = PublicationArbiter::prepare_one_quantum(&profile)
            .map_err(|e| PulseError::Preparation(e.to_string()))?;
        let bytes = pulses
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_mul(size_of::<TimedEvent>()))
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .and_then(|n| u64::try_from(n).ok())
            .and_then(|n| n.checked_add(arbiter.storage_bytes()?.get()))
            .ok_or(PulseError::Bytes)?;
        if bytes > ceiling.get() {
            return Err(PulseError::Bytes);
        }
        let (control, renderer) = StreamControl::open(
            plan,
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .map_err(|e| PulseError::Preparation(e.to_string()))?;
        let mut edges = Vec::new();
        edges
            .try_reserve_exact(pulses.len() * 2)
            .map_err(|_| PulseError::Bytes)?;
        if edges.capacity() != pulses.len() * 2 {
            return Err(PulseError::Bytes);
        }
        let mut previous_end = None;
        let mut quantum = None;
        let mut density = 0_u32;
        for &at in pulses {
            if previous_end.is_some_and(|end| at <= end) {
                return Err(PulseError::Configuration);
            }
            let end = at
                .checked_add(width)
                .map_err(|_| PulseError::Configuration)?;
            for (time, value) in [
                (
                    at,
                    ParameterValue::new(1.0).map_err(|_| PulseError::Configuration)?,
                ),
                (end, ParameterValue::ZERO),
            ] {
                let destination = time.as_u64() / u64::from(crate::time::QUANTUM_FRAMES);
                if quantum != Some(destination) {
                    quantum = Some(destination);
                    density = 0;
                }
                density = density.checked_add(1).ok_or(PulseError::Configuration)?;
                if density >= arbiter.session_share().get() {
                    return Err(PulseError::Configuration);
                }
                edges.push(TimedEvent::new(
                    EventEnvelope::new(renderer.epoch(), time, TimeSource::Compiled),
                    EventPayload::SetParameter { slot: gate, value },
                ));
            }
            previous_end = Some(end);
        }
        Ok(Self {
            _control: control,
            renderer,
            arbiter,
            gate,
            edges: edges.into_boxed_slice(),
            next: 0,
            end: None,
            closed: false,
            bytes: PreparedBytes::measured(bytes),
        })
    }
    pub const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
}
