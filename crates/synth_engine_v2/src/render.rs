//! The renderer: the host-driven boundary and the quantum splitting contract.
//!
//! ADR-0001 is what this module implements. The renderer never processes a partial
//! quantum, control is evaluated exactly once per quantum, and a caller block of any
//! size up to the profile's maximum is served from a carry that was primed with `Q`
//! frames of silence at stream start. Partition invariance follows by construction
//! rather than by test: the sequence of quanta does not depend on how the caller
//! chops its requests.
//!
//! # What the render loop may not do
//!
//! No heap allocation, no lock, no I/O, no logging. Every buffer is allocated at
//! preparation, the event scratch is preallocated to the capacity admission approved,
//! and the one sort the loop performs is in place. A caller precondition returns a
//! [`RenderError`]; a `debug_assert` cannot define release behaviour because it
//! compiles out of the build that runs.

use crate::diagnostics::{CompileError, DiagnosticsReport, RenderError};
use crate::ir::{NodeId, ParameterId};
use crate::plan::{CompiledPlan, PlanOp};
use crate::quantities::{Amplitude, ChannelLayout, EventCount, Frequency, ParameterValue};
use crate::time::{
    FrameCount, QUANTUM_FRAMES, SampleTime, StreamAnchor, StreamEpoch, TimeSource, issue_epoch,
};

/// How many bytes [`PreparedRenderer::prepare`] will allocate for one call's event
/// resolution, given a plan's capacities.
///
/// It lives next to the allocation it describes and is called by admission, so the two
/// cannot drift: an earlier revision computed the scratch budget from the audio buffers
/// alone, so a raised `max_events_per_quantum` or a large `maximum_block_size` could be
/// reported as fitting and then allocate far past the budget at preparation.
#[must_use]
pub fn event_scratch_bytes(
    max_events_per_quantum: EventCount,
    maximum_block_size: FrameCount,
) -> u64 {
    let quantum = u64::from(QUANTUM_FRAMES);
    let quanta_per_call = maximum_block_size
        .as_u64()
        .div_ceil(quantum)
        .saturating_add(1);
    let events = u64::from(max_events_per_quantum.get()).saturating_mul(quanta_per_call);
    events
        .saturating_mul(size_of::<DueEvent>() as u64)
        .saturating_add(quanta_per_call.saturating_mul(size_of::<u32>() as u64))
}

/// A caller's output block: interleaved samples the renderer writes.
#[derive(Debug)]
#[must_use]
pub struct AudioBlockMut<'a> {
    samples: &'a mut [f32],
    frames: usize,
    layout: ChannelLayout,
}

impl<'a> AudioBlockMut<'a> {
    /// Wrap a caller's buffer, checking that its shape is what it claims.
    ///
    /// The check is here rather than in the render loop so that the loop can index
    /// without branching on shape, and so that a wrong shape is a caller error with a
    /// diagnostic rather than a panic on the audio thread.
    pub fn new(
        samples: &'a mut [f32],
        frames: usize,
        layout: ChannelLayout,
    ) -> Result<Self, RenderError> {
        let needed = frames.saturating_mul(layout.channels());
        if samples.len() != needed {
            return Err(RenderError::OutputBufferShape {
                samples: samples.len(),
                frames,
                layout,
                needed,
            });
        }
        Ok(Self {
            samples,
            frames,
            layout,
        })
    }

    /// How many frames the caller asked for.
    pub const fn frames(&self) -> usize {
        self.frames
    }

    /// The block's channel layout.
    pub const fn layout(&self) -> ChannelLayout {
        self.layout
    }
}

/// What every event carries in addition to its payload.
///
/// ADR-0032 clause 17. It is `Copy` and fixed-size, so it crosses a lock-free queue
/// under the audio thread's rules, and the `epoch` is what makes an event from a dead
/// stream detectable rather than merely unlikely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct EventEnvelope {
    epoch: StreamEpoch,
    time: SampleTime,
    source: TimeSource,
}

impl EventEnvelope {
    /// Stamp an event.
    pub const fn new(epoch: StreamEpoch, time: SampleTime, source: TimeSource) -> Self {
        Self {
            epoch,
            time,
            source,
        }
    }

    /// The epoch the event was stamped in.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// When the event says it happens.
    ///
    /// **Immutable.** Nothing the renderer does rewrites it: a clamp moves the
    /// *render position*, and the declared sample survives, so a report can say how
    /// far an event was displaced and a recorded performance is never quantized
    /// forward by an overload.
    pub const fn time(&self) -> SampleTime {
        self.time
    }

    /// Where the timestamp came from.
    pub const fn source(&self) -> TimeSource {
        self.source
    }
}

/// What an event does.
///
/// Phase 1 has one payload. It is a **control-rate** change, so ADR-0001 clause 13
/// applies: the evaluation at a quantum's start may observe only events at or before
/// that quantum's first sample, and an event inside a quantum takes effect at the next
/// boundary. Lookahead would make a value depend on the future.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventPayload {
    /// Set one parameter of one node.
    SetParameter {
        /// The node addressed.
        node: NodeId,
        /// The parameter on it.
        parameter: ParameterId,
        /// The new value, validated where it was built.
        value: ParameterValue,
    },
}

/// One stamped event.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct TimedEvent {
    envelope: EventEnvelope,
    payload: EventPayload,
}

impl TimedEvent {
    /// An event.
    pub const fn new(envelope: EventEnvelope, payload: EventPayload) -> Self {
        Self { envelope, payload }
    }

    /// The stamp.
    pub const fn envelope(&self) -> EventEnvelope {
        self.envelope
    }

    /// The payload.
    pub const fn payload(&self) -> EventPayload {
        self.payload
    }
}

/// The events one render call is presented with.
///
/// A **prevalidated bounded span**, per the host-profile specification's rule for a
/// phase in which `HOST-INV-021` is deferred: the renderer groups these by the same
/// absolute quantum boundaries it renders on, independently of how the caller
/// partitions blocks, and rejects the call if any one quantum exceeds
/// `max_events_per_quantum`. The span's **total** is not a limit — one call may cover
/// several quanta.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct TimedEvents<'a> {
    events: &'a [TimedEvent],
}

impl<'a> TimedEvents<'a> {
    /// No events.
    pub const EMPTY: Self = Self { events: &[] };

    /// A span.
    pub const fn new(events: &'a [TimedEvent]) -> Self {
        Self { events }
    }

    /// The events, in the order the caller presented them.
    pub const fn as_slice(&self) -> &'a [TimedEvent] {
        self.events
    }
}

/// The host-driven renderer boundary.
pub trait Renderer {
    /// Render one caller block.
    ///
    /// Fallible on purpose: an over-full quantum, an oversized callback, and an
    /// exhausted clock are all conditions a release build has to define, and a
    /// `debug_assert` defines none of them.
    fn render(
        &mut self,
        output: AudioBlockMut<'_>,
        events: TimedEvents<'_>,
    ) -> Result<(), RenderError>;
}

/// A sine's mutable state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SineState {
    /// Normalized phase in `[0, 1)`.
    pub(crate) phase: f64,
    pub(crate) frequency: Frequency,
    pub(crate) amplitude: Amplitude,
}

/// One event, resolved to a render position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DueEvent {
    pub(crate) position: SampleTime,
    pub(crate) arrival: u32,
    pub(crate) payload: EventPayload,
}

impl DueEvent {
    /// The value the scratch is filled with at preparation.
    ///
    /// Never read: `scratch_len` bounds every read, and the fill exists so the buffer can
    /// be allocated to its full length once. That is what makes growth *impossible* in
    /// the loop rather than merely unlikely — a `Vec::push` with spare capacity does not
    /// allocate, but a `Vec::push` is still a call that can.
    const FILL: Self = Self {
        position: SampleTime::ZERO,
        arrival: 0,
        payload: EventPayload::SetParameter {
            node: NodeId::FIRST,
            parameter: ParameterId::FIRST,
            value: ParameterValue::ZERO,
        },
    };
}

/// Counters a call accumulates before committing them.
///
/// Held locally so that a call which fails validation leaves the published counters
/// untouched: a rejected call must not move renderer state, and the diagnostics report
/// is renderer state.
#[derive(Debug, Clone, Copy, Default)]
struct PendingCounts {
    late: u32,
    stale_epoch: u32,
    out_of_horizon: u32,
    arrival_stamped: u32,
}

/// An admitted plan, prepared for one stream epoch.
#[derive(Debug)]
#[must_use]
pub struct PreparedRenderer {
    plan: CompiledPlan,
    epoch: StreamEpoch,
    anchor: StreamAnchor,
    clock: SampleTime,
    channels: usize,
    /// Per-source buffers, `Q` frames each, back to back.
    buffers: Vec<f32>,
    /// Interleaved output frames waiting to be served.
    output_carry: Vec<f32>,
    /// How many frames of [`Self::output_carry`] are live.
    carry_frames: usize,
    /// ADR-0001 clause 5's input carry.
    ///
    /// Prepared at `maximum_block_size + Q` frames and **not read in this phase**: no
    /// Phase 1 node consumes live input, and the render signature has no input block
    /// to append. It is allocated because the contract sizes it and the report
    /// accounts for it, and the phase that adds an input-consuming node is the one
    /// that starts reading it. Preparing it here is what keeps that phase from
    /// discovering it needs an allocation on the audio thread.
    input_carry: Vec<f32>,
    sine_states: Vec<SineState>,
    /// Whether the plan writes the carry at all, decided once at preparation so the
    /// loop does not re-derive a topology fact per quantum.
    has_output: bool,
    /// Fixed-length scratch for one call's resolved events.
    ///
    /// Allocated to `max_events_per_quantum` times the quanta a call can render, and
    /// written by index — never grown. [`Self::scratch_len`] is how much of it is live.
    event_scratch: Vec<DueEvent>,
    scratch_len: usize,
    quantum_counts: Vec<u32>,
    diagnostics: DiagnosticsReport,
}

impl PreparedRenderer {
    /// Prepare an admitted plan, issuing a stream epoch.
    ///
    /// Everything the render loop touches is allocated here. The output carry is
    /// primed with `Q` frames of silence, which is ADR-0001 clause 6 and the reason
    /// clause 5's loop can serve any `N` — including `N < Q` and any irregular
    /// sequence — without rendering a quantum whose input has not arrived.
    pub fn prepare(plan: CompiledPlan, anchor: StreamAnchor) -> Result<Self, CompileError> {
        let epoch = issue_epoch()?;
        let quantum = QUANTUM_FRAMES as usize;
        let channels = plan.channel_layout().channels();

        let max_block =
            plan.maximum_block_size()
                .as_usize()
                .ok_or(CompileError::ReportUnitMismatch {
                    field: crate::report::ResourceField::MaximumBlockSize,
                })?;
        let carry_frames_capacity = max_block.saturating_add(quantum);

        let sine_states = plan
            .sine_templates()
            .iter()
            .map(|template| SineState {
                phase: 0.0,
                frequency: template.frequency,
                amplitude: template.amplitude,
            })
            .collect();

        // A call renders at most one quantum more than its frame count spans, so this
        // bounds both the per-quantum tally and the event scratch.
        let quanta_per_call = max_block.div_ceil(quantum).saturating_add(1);
        let events_per_quantum = plan.max_events_per_quantum().as_usize().unwrap_or(0);

        let mut output_carry = vec![0.0; carry_frames_capacity.saturating_mul(channels)];
        output_carry.fill(0.0);

        let has_output = plan
            .ops()
            .iter()
            .any(|op| matches!(op, PlanOp::OutputMono { .. }));

        Ok(Self {
            buffers: vec![0.0; plan.buffer_count().saturating_mul(quantum)],
            output_carry,
            // Primed: `Q` frames of silence are already available to serve.
            carry_frames: quantum,
            input_carry: vec![0.0; carry_frames_capacity.saturating_mul(channels)],
            sine_states,
            has_output,
            event_scratch: vec![DueEvent::FILL; events_per_quantum.saturating_mul(quanta_per_call)],
            scratch_len: 0,
            quantum_counts: vec![0; quanta_per_call],
            plan,
            epoch,
            anchor,
            clock: SampleTime::ZERO,
            channels,
            diagnostics: DiagnosticsReport::default(),
        })
    }

    /// This stream's epoch.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// The render clock: input frames consumed so far.
    pub const fn clock(&self) -> SampleTime {
        self.clock
    }

    /// The counters this stream has accumulated.
    pub const fn diagnostics(&self) -> &DiagnosticsReport {
        &self.diagnostics
    }

    /// The latency this stream adds, which is a constant `Q` frames.
    pub const fn added_latency(&self) -> FrameCount {
        self.plan.added_latency()
    }

    /// The plan being rendered.
    pub const fn plan(&self) -> &CompiledPlan {
        &self.plan
    }
}

/// The hot path: everything that runs on the audio thread.
///
/// It is a separate file so that the real-time rules have an unambiguous region to be
/// checked against. `tests/render_loop_purity.rs` reads it and fails on a lock, an I/O
/// call, a logging macro, a panicking accessor, or an allocating construct — none of
/// which the counting-allocator test can see for the three that do not allocate.
#[path = "render/hot.rs"]
mod hot;
