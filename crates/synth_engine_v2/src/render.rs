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
use crate::node::kernels::{NodeState, TimedControl};
use crate::plan::{CompiledPlan, PlanOp};
use crate::quantities::{ChannelLayout, EventCount, ParameterValue, RecordCount};
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

/// How many bytes [`PreparedRenderer::prepare`] will allocate for the sample-positioned
/// control scratch, given a plan's per-quantum event capacity and how many records it
/// schedules.
///
/// Beside the allocation for the same reason [`event_scratch_bytes`] is: admission has to
/// state what preparation takes, and two formulas for one allocation drift. One quantum's
/// events bound how many sample-positioned changes that quantum can carry, and the two
/// index tables are one entry per scheduled record plus a terminator.
#[must_use]
pub fn timed_control_scratch_bytes(
    max_events_per_quantum: EventCount,
    scheduled_records: RecordCount,
) -> u64 {
    let controls =
        u64::from(max_events_per_quantum.get()).saturating_mul(size_of::<TimedControl>() as u64);
    let index = u64::from(scheduled_records.get())
        .saturating_mul(2)
        .saturating_add(1)
        .saturating_mul(size_of::<u32>() as u64);
    controls.saturating_add(index)
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

/// Which way a note edge goes.
///
/// An **edge**, not a level: a note is played or let go, and ADR-0001 clause 14 names both
/// as sample-positioned effects. Carrying a float here instead would invite the question
/// of what a gate of `0.5` means, which no caller has ever needed to ask.
///
/// It carries no pitch, velocity or note identity. Nothing in this phase reads any of
/// them — the envelope has no velocity input and a sine's frequency is an ordinary
/// control — and a field nothing reads is a contract the phase has not earned. Phase 3's
/// ingress and Phase 6's voice pool are where they arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum NoteEdge {
    /// The note is played.
    On,
    /// The note is let go.
    Off,
}

impl NoteEdge {
    /// The control value this edge sets.
    ///
    /// Any value above zero raises a gate and zero lowers it, so an edge is one of exactly
    /// two values. Named constants rather than a fallible conversion, because the audio
    /// thread has no way to report a failure and no fallback here would be honest.
    pub const fn value(self) -> ParameterValue {
        match self {
            Self::On => ParameterValue::ONE,
            Self::Off => ParameterValue::ZERO,
        }
    }
}

/// What an event does.
///
/// **When** it takes effect is not this enum's to say. ADR-0001 clause 14, as ADR-0043
/// restated it, splits on the *effect*: a sample-positioned one — note-on, note-off, gate,
/// retrigger — occurs at the sample its **render position** names, while a control-rate
/// response begins at the first quantum boundary at or after that position, under clause
/// 13's causality rule. The render position is the declared sample unless the preserving
/// late clamp moves a genuinely late event to the first not-yet-rendered boundary. The node
/// kind declares which of the two each of its controls is, admission compiles that into the
/// target, and the renderer reads it. So addressing a gate as a parameter and playing its
/// node as a note reach the same control under the same timing law, and neither payload can
/// be used to escape the other's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventPayload {
    /// Set one compiled parameter slot.
    ///
    /// The slot, not the `(node, parameter)` pair: [`CompiledPlan::resolve_parameter`]
    /// turns an address into one **off the audio thread**, so the render loop indexes
    /// instead of searching, and an address the plan does not have is caught where a
    /// caller can still be told about it.
    SetParameter {
        /// Which compiled parameter.
        slot: crate::plan::ParameterSlot,
        /// The new value, validated where it was built.
        value: ParameterValue,
    },
    /// Play or let go of one compiled node.
    ///
    /// The note-side twin of [`Self::SetParameter`], resolved by
    /// [`CompiledPlan::resolve_note`] under the same rule. It names the node rather than
    /// one of its controls: what being played *means* belongs to the node kind, which is
    /// what lets a caller play a voice without knowing the graph inside it.
    Note {
        /// Which compiled node is played.
        slot: crate::plan::NoteSlot,
        /// Which way the edge goes.
        edge: NoteEdge,
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
/// A **prevalidated bounded span**, per the host-profile specification's Phase 1–2
/// presentation rule. The renderer groups these by the same absolute quantum boundaries
/// it renders on, independently of how the caller partitions blocks, and rejects the
/// call if any one quantum exceeds `max_events_per_quantum`. Phase 3 instead presents
/// only sealed, admitted batches for the imminent render call. The span's **total** is
/// not a limit — one call may cover several quanta.
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
            // Never read: `scratch_len` bounds every read of the scratch. The identity
            // is the first a process can issue, so even if it were read it would name a
            // plan this renderer does not hold.
            slot: crate::plan::ParameterSlot::new(crate::plan::PlanId::FILL, 0),
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
    foreign_slot: u32,
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
    node_states: Vec<NodeState>,
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
    /// One quantum's sample-positioned control changes, grouped by the node they move.
    ///
    /// Rebuilt before each quantum renders and never grown: one quantum's declared event
    /// capacity bounds how many of these it can carry, so the length admission approved is
    /// the length this holds for the life of the stream.
    timed_controls: Vec<TimedControl>,
    /// Where each node's run of [`Self::timed_controls`] starts, plus a terminator.
    ///
    /// `control_starts[n] .. control_starts[n + 1]` is node `n`'s slice, which is what
    /// lets the schedule walk hand a kernel its own edges by indexing rather than by
    /// searching or by relying on the order node slots appear in.
    control_starts: Vec<u32>,
    /// How far each node's run has been filled, while it is being built.
    control_fill: Vec<u32>,
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

        // One state per prepared node, built from the prepared record so the two tables
        // are parallel by construction rather than by two counters agreeing.
        let node_states: Vec<NodeState> = plan
            .prepared_nodes()
            .iter()
            .map(NodeState::initial)
            .collect();

        // A call renders at most one quantum more than its frame count spans, so this
        // bounds both the per-quantum tally and the event scratch.
        let quanta_per_call = max_block.div_ceil(quantum).saturating_add(1);
        let events_per_quantum = plan.max_events_per_quantum().as_usize().unwrap_or(0);
        // One index entry per scheduled record, from the table the renderer already keeps
        // one state per — so the two cannot be counted differently.
        let records = node_states.len();

        let mut output_carry = vec![0.0; carry_frames_capacity.saturating_mul(channels)];
        output_carry.fill(0.0);

        let has_output = plan
            .ops()
            .iter()
            .any(|op| matches!(op, PlanOp::Output { .. }));

        Ok(Self {
            // ADR-0041 clause 13: the arena is one allocation of variable-width regions,
            // sized by the extent the assignment reached rather than by a count of
            // uniform slots.
            buffers: vec![0.0; plan.arena_samples()],
            output_carry,
            // Primed: `Q` frames of silence are already available to serve.
            carry_frames: quantum,
            input_carry: vec![0.0; carry_frames_capacity.saturating_mul(channels)],
            node_states,
            has_output,
            event_scratch: vec![DueEvent::FILL; events_per_quantum.saturating_mul(quanta_per_call)],
            scratch_len: 0,
            quantum_counts: vec![0; quanta_per_call],
            // ADR-0001 clause 14's storage. One quantum at a time, because an edge is
            // applied inside the quantum it falls in and nothing outlives that.
            timed_controls: vec![TimedControl::FILL; events_per_quantum],
            control_starts: vec![0; records.saturating_add(1)],
            control_fill: vec![0; records],
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
