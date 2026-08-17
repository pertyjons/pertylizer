//! The prepared render plan: what the renderer executes.
//!
//! The master plan's layer boundaries fix what belongs here — ordered operations,
//! numeric slots, immutable prepared node data, fixed-size mutable state layout,
//! event-routing tables, latency metadata — and what must not: **no validation
//! branches, strings, hash maps, filesystem paths, or construction logic in the
//! render loop**.
//!
//! It also carries every capacity the renderer needs, copied at admission. That is
//! `HOST-INV-002`: the renderer reads the prepared plan and never the profile, so a
//! capacity reaching the audio thread without having passed admission is a defect
//! rather than a fallback.

use crate::ir::{NodeId, ParameterId};
use crate::quantities::{Amplitude, ChannelLayout, EventCount, Frequency, SampleRate};
use crate::time::{FrameCount, PlanPosition};

/// One buffer in the plan's arena, by index.
///
/// Phase 1 gives every source its own quantum-sized buffer. The preallocated arena
/// with liveness analysis, so that non-overlapping signal lifetimes share storage,
/// is Phase 2's work; anticipating it here would produce an arena with nothing to
/// analyse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct BufferSlot(usize);

impl BufferSlot {
    /// A slot.
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// The index.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// One mutable-state record in the plan, by index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct StateSlot(usize);

impl StateSlot {
    /// A slot.
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// The index.
    pub const fn index(self) -> usize {
        self.0
    }
}

/// One operation, in execution order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlanOp {
    /// Fill a buffer with zeros.
    Silence {
        /// Where the zeros go.
        out: BufferSlot,
    },
    /// Fill a buffer with one level.
    Constant {
        /// Where the level goes.
        out: BufferSlot,
        /// The level.
        level: Amplitude,
    },
    /// Advance a phase accumulator and write a sine.
    Sine {
        /// Where the sine goes.
        out: BufferSlot,
        /// The phase and control values it advances.
        state: StateSlot,
    },
    /// Write one sample of `1.0` where the plan position falls inside this quantum.
    Impulse {
        /// Where the click goes.
        out: BufferSlot,
        /// Where in the plan the click is.
        position: PlanPosition,
    },
    /// Copy one mono buffer to every output channel.
    ///
    /// The duplication is **declared**, not inferred: Phase 2 owns inserting
    /// implicit conversions, and a compiler that quietly widened a signal here would
    /// be doing that work under another name.
    OutputMono {
        /// The buffer to write out.
        source: BufferSlot,
    },
}

/// A sine's initial state, prepared once.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct SineTemplate {
    /// Starting frequency.
    pub frequency: Frequency,
    /// Starting peak amplitude.
    pub amplitude: Amplitude,
}

/// What a parameter event addresses, resolved to a numeric slot at admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterTarget {
    /// A sine's frequency.
    SineFrequency(StateSlot),
    /// A sine's amplitude.
    SineAmplitude(StateSlot),
}

/// One row of the plan's event-routing table.
///
/// The renderer resolves an addressed parameter by scanning this table, which is
/// preallocated, bounded by the plan's parameter count, and free of strings and
/// hashing. Phase 2 compiles the address itself into a numeric slot so the scan
/// disappears; a scan over a handful of entries is not the defect its gate names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ParameterRoute {
    /// The node the event addresses.
    pub node: NodeId,
    /// The parameter on that node.
    pub parameter: ParameterId,
    /// Where the value lands.
    pub target: ParameterTarget,
}

/// An admitted plan, with every capacity it needs.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct CompiledPlan {
    ops: Vec<PlanOp>,
    buffer_count: usize,
    sine_templates: Vec<SineTemplate>,
    parameter_routes: Vec<ParameterRoute>,
    channel_layout: ChannelLayout,
    sample_rate: SampleRate,
    maximum_block_size: FrameCount,
    max_events_per_quantum: EventCount,
    forward_event_horizon: FrameCount,
    added_latency: FrameCount,
}

impl CompiledPlan {
    /// Assemble a plan. Called by admission and by nothing else.
    #[allow(
        clippy::too_many_arguments,
        reason = "a prepared plan carries exactly the capacities admission copied into it; \
                  bundling them would hide which ones the renderer depends on"
    )]
    pub(crate) const fn new(
        ops: Vec<PlanOp>,
        buffer_count: usize,
        sine_templates: Vec<SineTemplate>,
        parameter_routes: Vec<ParameterRoute>,
        channel_layout: ChannelLayout,
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        max_events_per_quantum: EventCount,
        forward_event_horizon: FrameCount,
        added_latency: FrameCount,
    ) -> Self {
        Self {
            ops,
            buffer_count,
            sine_templates,
            parameter_routes,
            channel_layout,
            sample_rate,
            maximum_block_size,
            max_events_per_quantum,
            forward_event_horizon,
            added_latency,
        }
    }

    /// The operations, in execution order.
    pub fn ops(&self) -> &[PlanOp] {
        &self.ops
    }

    /// How many quantum-sized buffers the plan needs.
    pub const fn buffer_count(&self) -> usize {
        self.buffer_count
    }

    /// The initial state of every sine.
    pub fn sine_templates(&self) -> &[SineTemplate] {
        &self.sine_templates
    }

    /// The event-routing table.
    pub fn parameter_routes(&self) -> &[ParameterRoute] {
        &self.parameter_routes
    }

    /// The stream's channel layout.
    pub const fn channel_layout(&self) -> ChannelLayout {
        self.channel_layout
    }

    /// The stream's sample rate.
    pub const fn sample_rate(&self) -> SampleRate {
        self.sample_rate
    }

    /// The largest callback this plan was prepared for.
    pub const fn maximum_block_size(&self) -> FrameCount {
        self.maximum_block_size
    }

    /// Events one quantum may be presented with.
    pub const fn max_events_per_quantum(&self) -> EventCount {
        self.max_events_per_quantum
    }

    /// How far ahead an ingress event may be stamped.
    pub const fn forward_event_horizon(&self) -> FrameCount {
        self.forward_event_horizon
    }

    /// The latency this plan adds, which is ADR-0001 clause 7's constant `Q`.
    ///
    /// Charged unconditionally, including to a host whose callbacks are always whole
    /// multiples of the quantum and which would not otherwise need it — because a
    /// latency that varies with the caller's block pattern cannot be declared once
    /// or compensated statically.
    pub const fn added_latency(&self) -> FrameCount {
        self.added_latency
    }
}
