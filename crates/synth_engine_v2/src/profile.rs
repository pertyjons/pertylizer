//! The host profile: the single immutable preparation input.
//!
//! `HOST-INV-001` and `HOST-INV-002` are the two properties everything else
//! rests on. A profile is immutable for the life of one stream epoch, no field is
//! read from a global, and **the renderer never reads it**: admission copies every
//! capacity it needs into the prepared plan, so a capacity reaching the audio
//! thread without having passed admission is a defect rather than a fallback.
//!
//! The profile splits into two halves that differ in *who may set the value*:
//! [`HostCapabilities`] is what the host and device can do, queried rather than
//! chosen, and [`RenderLimits`] is what the operator budgets.

use thiserror::Error;

use crate::quantities::{
    BusCount, ChannelLayout, CostRatio, EdgeCount, EventCount, FanOut, HeldNoteCount,
    InstructionCount, MixChannelCount, NodeCount, PreparedBytes, QuantityError, SampleRate,
    SampleRateRange, SendCount, SlotCount, TapCount, VoiceCount,
};
use crate::time::{FrameCount, QUANTUM_FRAMES};

/// Generate `const fn` accessors for `Copy` fields.
///
/// No `#[must_use]`: every type these return already carries it, and repeating the
/// attribute is what `clippy::double_must_use` objects to.
macro_rules! accessors {
    ($($field:ident: $ty:ty, $doc:expr;)+) => {
        $(
            #[doc = $doc]
            pub const fn $field(&self) -> $ty {
                self.$field
            }
        )+
    };
}

/// Refuse a zero capacity, naming the field rather than the type.
///
/// The group constructors take already-built newtypes, so a caller can hand them a
/// `measured(0)` or a `NONE` and bypass the `limit` constructor's own check. This is where
/// that hole is closed: a capacity of zero admits nothing, so a profile carrying one is a
/// profile that can compile no plan, and the report would show a plan refused against a
/// budget of nothing.
fn nonzero(field: &'static str, value: u64) -> Result<(), ProfileError> {
    if value == 0 {
        Err(ProfileError::Quantity(QuantityError::ZeroCapacity {
            quantity: field,
        }))
    } else {
        Ok(())
    }
}

/// A profile that refused to be built.
///
/// `HOST-INV-016`: construction is fallible, there is no partially valid profile,
/// and there is no clamping constructor. A profile whose fields disagree names
/// *both* fields, because naming one leaves the reader guessing which to change.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ProfileError {
    /// A single field was outside its own domain.
    #[error("profile field out of range: {0}")]
    Quantity(#[from] QuantityError),

    /// The prepared rate is outside the range the profile accepts.
    ///
    /// This is `accepted_sample_rates`' enforcement point. It is a construction
    /// failure rather than a `CompileError` because no plan carries a rate, so no
    /// plan can exceed the limit — see `HOST-INV-007`'s narrowing.
    #[error("sample_rate {rate} is outside accepted_sample_rates {range}")]
    SampleRateOutsideAcceptedRange {
        /// The rate that was asked for.
        rate: SampleRate,
        /// The range the profile accepts.
        range: SampleRateRange,
    },

    /// The forward event horizon is below `maximum_block_size + Q`.
    ///
    /// `HOST-INV-013`'s floor. A horizon shorter than one callback plus one
    /// quantum would reject events the host is entitled to deliver.
    #[error(
        "forward_event_horizon {horizon} is below maximum_block_size {block} plus the quantum \
         ({quantum} frames)"
    )]
    ForwardHorizonBelowFloor {
        /// The horizon as given.
        horizon: FrameCount,
        /// The block size it must clear.
        block: FrameCount,
        /// `Q`, in frames.
        quantum: u32,
    },

    /// Script host slots are below Mod Matrix slots.
    ///
    /// `HOST-INV-017`: V1 keeps this as a compile-time assertion in a third crate,
    /// and the relation is an inequality rather than an equality — lowering the
    /// host slots below the matrix count breaks the build, raising them alone is
    /// legal. Declaring it once in this constructor is what stops the two from
    /// drifting.
    #[error("script_host_slots_per_voice {host} is below mod_matrix_slots_per_voice {matrix}")]
    ScriptHostSlotsBelowModMatrix {
        /// Script host slots as given.
        host: SlotCount,
        /// Mod Matrix slots as given.
        matrix: SlotCount,
    },

    /// A voice range whose endpoints are the wrong way round.
    #[error("voices_per_instrument range {minimum} to {maximum} is inverted")]
    InvertedVoiceRange {
        /// The lower endpoint.
        minimum: VoiceCount,
        /// The upper endpoint.
        maximum: VoiceCount,
    },
}

/// Where the capability half came from.
///
/// An offline job has no device, so it declares its capabilities rather than
/// pretending to have queried them, and a report quoting a profile can say which
/// it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilitySource {
    /// Queried from a live device through the audio backend.
    Device,
    /// Declared by an offline render or analysis job.
    Offline,
    /// Declared by a test harness constructing IR directly.
    Harness,
}

impl std::fmt::Display for CapabilitySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Device => f.write_str("queried from a device"),
            Self::Offline => f.write_str("declared by an offline job"),
            Self::Harness => f.write_str("declared by a test harness"),
        }
    }
}

/// What the host and device can do. Queried, never chosen.
///
/// Four members: **three queried capabilities** — the set `HOST-INV-005` closes —
/// and [`CapabilitySource`], which nothing queries and which `HOST-INV-003`
/// admits instead.
///
/// # What the constructor split guarantees
///
/// `HOST-INV-003` forbids a hardcoded advertised range on the device path — the
/// `LIMIT-0057` anti-pattern, where V1 discards a queried buffer range in favour
/// of a constant. No runtime tag can prove that a query happened, so the rule is
/// enforced by API shape instead, and three narrower things hold: a capability
/// that was not queried must be **written at the call site**, where review sees a
/// literal; there is no `Default` and no `..Default::default()` tail; and a path
/// with no device need not mislabel itself, because [`Self::offline`] and
/// [`Self::harness`] exist. A caller determined to mislabel itself still can.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct HostCapabilities {
    sample_rate: SampleRate,
    maximum_block_size: FrameCount,
    channel_layout: ChannelLayout,
    source: CapabilitySource,
}

impl HostCapabilities {
    /// The device path. Every capability is an argument and none has a default.
    ///
    /// Sets [`CapabilitySource::Device`], which is not an argument, so no call can
    /// pass a tag that disagrees with the constructor it called.
    pub fn from_device(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
    ) -> Result<Self, ProfileError> {
        Self::build(
            sample_rate,
            maximum_block_size,
            channel_layout,
            CapabilitySource::Device,
        )
    }

    /// An offline render or analysis job, which has no device to query.
    pub fn offline(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
    ) -> Result<Self, ProfileError> {
        Self::build(
            sample_rate,
            maximum_block_size,
            channel_layout,
            CapabilitySource::Offline,
        )
    }

    /// A test harness constructing IR directly.
    pub fn harness(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
    ) -> Result<Self, ProfileError> {
        Self::build(
            sample_rate,
            maximum_block_size,
            channel_layout,
            CapabilitySource::Harness,
        )
    }

    fn build(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
        source: CapabilitySource,
    ) -> Result<Self, ProfileError> {
        // A device reporting a zero-frame maximum block can serve no callback.
        // There is deliberately **no upper bound and no lower bound in terms of
        // `Q`**: the queried value sizes the carries, and ADR-0001 clause 6 primes
        // the output carry so that a host whose largest block is smaller than one
        // quantum is supported unchanged.
        if maximum_block_size.as_u64() == 0 {
            return Err(ProfileError::Quantity(QuantityError::ZeroCapacity {
                quantity: "maximum_block_size",
            }));
        }
        Ok(Self {
            sample_rate,
            maximum_block_size,
            channel_layout,
            source,
        })
    }

    accessors! {
        sample_rate: SampleRate, "The rate this stream is prepared at.";
        maximum_block_size: FrameCount, "The largest callback the host may deliver.";
        channel_layout: ChannelLayout, "What the stream's channels are.";
        source: CapabilitySource, "Which constructor produced these capabilities.";
    }
}

/// The range of rates a stream may be prepared at.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct StreamLimits {
    accepted_sample_rates: SampleRateRange,
}

impl StreamLimits {
    /// Limits over the rates a stream may take.
    pub const fn new(accepted_sample_rates: SampleRateRange) -> Self {
        Self {
            accepted_sample_rates,
        }
    }

    accessors! {
        accepted_sample_rates: SampleRateRange, "The inclusive range of admissible rates.";
    }
}

/// Graph extents after polyphony expansion.
///
/// `LIMIT-0060` is why this group exists: V1 holds connections in an uncapped
/// `Vec` with unbounded fan-out, so "unbounded because nobody decided" becomes
/// bounded, reported, and raisable.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct GraphLimits {
    max_nodes: NodeCount,
    max_edges: EdgeCount,
    max_fan_out_per_port: FanOut,
    max_mod_graph_nodes: NodeCount,
    max_note_graph_nodes: NodeCount,
}

impl GraphLimits {
    /// Graph limits. Every capacity must be above zero.
    pub fn new(
        max_nodes: NodeCount,
        max_edges: EdgeCount,
        max_fan_out_per_port: FanOut,
        max_mod_graph_nodes: NodeCount,
        max_note_graph_nodes: NodeCount,
    ) -> Result<Self, ProfileError> {
        nonzero("max_nodes", u64::from(max_nodes.get()))?;
        nonzero("max_edges", u64::from(max_edges.get()))?;
        nonzero(
            "max_fan_out_per_port",
            u64::from(max_fan_out_per_port.get()),
        )?;
        nonzero("max_mod_graph_nodes", u64::from(max_mod_graph_nodes.get()))?;
        nonzero(
            "max_note_graph_nodes",
            u64::from(max_note_graph_nodes.get()),
        )?;
        Ok(Self {
            max_nodes,
            max_edges,
            max_fan_out_per_port,
            max_mod_graph_nodes,
            max_note_graph_nodes,
        })
    }

    accessors! {
        max_nodes: NodeCount, "Nodes in one prepared plan.";
        max_edges: EdgeCount, "Edges in one prepared plan.";
        max_fan_out_per_port: FanOut, "Edges leaving one output port.";
        max_mod_graph_nodes: NodeCount, "Nodes in one modulation graph.";
        max_note_graph_nodes: NodeCount, "Nodes in one note graph.";
    }
}

/// Polyphony and retirement.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct VoiceLimits {
    minimum_voices_per_instrument: VoiceCount,
    maximum_voices_per_instrument: VoiceCount,
    max_active_voices: VoiceCount,
    max_held_notes: HeldNoteCount,
    retirement_crossfade: FrameCount,
    max_concurrent_retiring_voices: VoiceCount,
}

impl VoiceLimits {
    /// Voice limits.
    ///
    /// `max_concurrent_retiring_voices` is **derived** from `max_active_voices`
    /// rather than taken, so that it cannot bind. How many voices retire at once
    /// is a runtime quantity — a plan swap retires whatever is sounding — and a
    /// voice cannot be refused retirement, so a smaller budget would be a field
    /// enforced at runtime with no defined behaviour on reaching it. What it still
    /// does is account: the crossfade buffers it implies are real prepared memory
    /// and appear in the report. ADR-0009 may lower it, but only together with a
    /// defined behaviour for reaching it.
    pub fn new(
        minimum_voices_per_instrument: VoiceCount,
        maximum_voices_per_instrument: VoiceCount,
        max_active_voices: VoiceCount,
        max_held_notes: HeldNoteCount,
        retirement_crossfade: FrameCount,
    ) -> Result<Self, ProfileError> {
        nonzero(
            "minimum_voices_per_instrument",
            u64::from(minimum_voices_per_instrument.get()),
        )?;
        nonzero(
            "maximum_voices_per_instrument",
            u64::from(maximum_voices_per_instrument.get()),
        )?;
        nonzero("max_active_voices", u64::from(max_active_voices.get()))?;
        nonzero("max_held_notes", u64::from(max_held_notes.get()))?;
        nonzero("retirement_crossfade", retirement_crossfade.as_u64())?;
        if minimum_voices_per_instrument > maximum_voices_per_instrument {
            return Err(ProfileError::InvertedVoiceRange {
                minimum: minimum_voices_per_instrument,
                maximum: maximum_voices_per_instrument,
            });
        }
        Ok(Self {
            minimum_voices_per_instrument,
            maximum_voices_per_instrument,
            max_active_voices,
            max_held_notes,
            retirement_crossfade,
            max_concurrent_retiring_voices: max_active_voices,
        })
    }

    accessors! {
        minimum_voices_per_instrument: VoiceCount, "Fewest voices an instrument may declare.";
        maximum_voices_per_instrument: VoiceCount, "Most voices an instrument may declare.";
        max_active_voices: VoiceCount, "Voices sounding across the whole plan.";
        max_held_notes: HeldNoteCount, "Notes held across the whole plan.";
        retirement_crossfade: FrameCount, "Frames a retiring voice crossfades over.";
        max_concurrent_retiring_voices: VoiceCount, "Voices retiring at once. Derived, so it cannot bind.";
    }
}

/// Event capacities.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct EventLimits {
    max_events_per_quantum: EventCount,
    max_note_expansion_per_tick: EventCount,
    max_scheduled_events_in_flight: EventCount,
    forward_event_horizon: FrameCount,
    command_queue_capacity: EventCount,
    event_egress_capacity: EventCount,
}

impl EventLimits {
    /// Event limits.
    ///
    /// `event_egress_capacity` is **one** capacity, not two: ADR-0038 part 4 split
    /// `LIMIT-0014`'s single constant, and the OSC ring it also sized became
    /// `LIMIT-0076`, owned by the protocol contract rather than by this profile.
    pub fn new(
        max_events_per_quantum: EventCount,
        max_note_expansion_per_tick: EventCount,
        max_scheduled_events_in_flight: EventCount,
        forward_event_horizon: FrameCount,
        command_queue_capacity: EventCount,
        event_egress_capacity: EventCount,
    ) -> Result<Self, ProfileError> {
        nonzero(
            "max_events_per_quantum",
            u64::from(max_events_per_quantum.get()),
        )?;
        nonzero(
            "max_note_expansion_per_tick",
            u64::from(max_note_expansion_per_tick.get()),
        )?;
        nonzero(
            "max_scheduled_events_in_flight",
            u64::from(max_scheduled_events_in_flight.get()),
        )?;
        nonzero("forward_event_horizon", forward_event_horizon.as_u64())?;
        nonzero(
            "command_queue_capacity",
            u64::from(command_queue_capacity.get()),
        )?;
        nonzero(
            "event_egress_capacity",
            u64::from(event_egress_capacity.get()),
        )?;
        Ok(Self {
            max_events_per_quantum,
            max_note_expansion_per_tick,
            max_scheduled_events_in_flight,
            forward_event_horizon,
            command_queue_capacity,
            event_egress_capacity,
        })
    }

    accessors! {
        max_events_per_quantum: EventCount, "Events one quantum may be presented with.";
        max_note_expansion_per_tick: EventCount, "Events one tick's note expansion may produce.";
        max_scheduled_events_in_flight: EventCount, "The scheduler's release window.";
        forward_event_horizon: FrameCount, "How far ahead an ingress event may be stamped.";
        command_queue_capacity: EventCount, "The live command queue's depth.";
        event_egress_capacity: EventCount, "The engine-to-GUI event ring's depth.";
    }
}

/// Observation and analysis capacities.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ObservationLimits {
    max_observation_taps: TapCount,
    telemetry_ring_frames: FrameCount,
    analyzer_fft_size: FrameCount,
}

impl ObservationLimits {
    /// Observation limits.
    pub fn new(
        max_observation_taps: TapCount,
        telemetry_ring_frames: FrameCount,
        analyzer_fft_size: FrameCount,
    ) -> Result<Self, ProfileError> {
        nonzero(
            "max_observation_taps",
            u64::from(max_observation_taps.get()),
        )?;
        nonzero("telemetry_ring_frames", telemetry_ring_frames.as_u64())?;
        nonzero("analyzer_fft_size", analyzer_fft_size.as_u64())?;
        Ok(Self {
            max_observation_taps,
            telemetry_ring_frames,
            analyzer_fft_size,
        })
    }

    accessors! {
        max_observation_taps: TapCount, "Taps one plan may register. V1 lost meters past 128 silently.";
        telemetry_ring_frames: FrameCount, "The telemetry window, in frames. Lossy by design.";
        analyzer_fft_size: FrameCount, "The analyzer's resolution budget.";
    }
}

/// Mixing extents.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct MixingLimits {
    max_mix_channels: MixChannelCount,
    max_buses: BusCount,
    max_sends_per_channel: SendCount,
}

impl MixingLimits {
    /// Mixing limits.
    pub fn new(
        max_mix_channels: MixChannelCount,
        max_buses: BusCount,
        max_sends_per_channel: SendCount,
    ) -> Result<Self, ProfileError> {
        nonzero("max_mix_channels", u64::from(max_mix_channels.get()))?;
        nonzero("max_buses", u64::from(max_buses.get()))?;
        nonzero(
            "max_sends_per_channel",
            u64::from(max_sends_per_channel.get()),
        )?;
        Ok(Self {
            max_mix_channels,
            max_buses,
            max_sends_per_channel,
        })
    }

    accessors! {
        max_mix_channels: MixChannelCount, "Mix channels in one plan.";
        max_buses: BusCount, "Buses in one plan.";
        max_sends_per_channel: SendCount, "Sends from one channel.";
    }
}

/// Prepared-memory budgets.
///
/// `HOST-INV-014`: these are checked against the compiler's computed aggregate
/// over prepared nodes, never against a process-level measurement. V1 computes no
/// such aggregate anywhere, which is `LIMIT-0073`'s finding; producing one is what
/// admission partly *is*.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct MemoryLimits {
    prepared_immutable_bytes: PreparedBytes,
    mutable_state_bytes: PreparedBytes,
    buffer_scratch_bytes: PreparedBytes,
}

impl MemoryLimits {
    /// Memory limits.
    pub fn new(
        prepared_immutable_bytes: PreparedBytes,
        mutable_state_bytes: PreparedBytes,
        buffer_scratch_bytes: PreparedBytes,
    ) -> Result<Self, ProfileError> {
        nonzero("prepared_immutable_bytes", prepared_immutable_bytes.get())?;
        nonzero("mutable_state_bytes", mutable_state_bytes.get())?;
        nonzero("buffer_scratch_bytes", buffer_scratch_bytes.get())?;
        Ok(Self {
            prepared_immutable_bytes,
            mutable_state_bytes,
            buffer_scratch_bytes,
        })
    }

    accessors! {
        prepared_immutable_bytes: PreparedBytes, "Immutable prepared data: coefficients and assets.";
        mutable_state_bytes: PreparedBytes, "Mutable node state, which scales with polyphony.";
        buffer_scratch_bytes: PreparedBytes, "Buffers and scratch, including both carries.";
    }
}

/// Per-program script capacities.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ScriptLimits {
    max_instructions_per_program: InstructionCount,
    max_sources_per_program: SlotCount,
    max_state_slots_per_program: SlotCount,
    max_locals_per_program: SlotCount,
    max_eval_stack_depth: SlotCount,
    max_arrays_per_program: SlotCount,
    max_array_elements: SlotCount,
    max_emits_per_program: SlotCount,
    mod_matrix_slots_per_voice: SlotCount,
    script_host_slots_per_voice: SlotCount,
}

impl ScriptLimits {
    /// Script limits.
    ///
    /// The last two are **two fields with a floor relation**, not one. The ledger
    /// recorded them as coupled 1:1 by a compile-time assertion; the use-site
    /// audit found an inequality and a module-agnostic script host, so collapsing
    /// them would refuse one resource or overprovision the other. The floor is
    /// validated in [`RenderLimits::new`], which is where `HOST-INV-017` wants it:
    /// declared once, not maintained by an assertion in a third crate.
    #[allow(
        clippy::too_many_arguments,
        reason = "ten per-program script capacities, each a separate profile field with its own \
                  ledger antecedent; grouping them into sub-structs would invent a taxonomy the \
                  specification does not have"
    )]
    pub fn new(
        max_instructions_per_program: InstructionCount,
        max_sources_per_program: SlotCount,
        max_state_slots_per_program: SlotCount,
        max_locals_per_program: SlotCount,
        max_eval_stack_depth: SlotCount,
        max_arrays_per_program: SlotCount,
        max_array_elements: SlotCount,
        max_emits_per_program: SlotCount,
        mod_matrix_slots_per_voice: SlotCount,
        script_host_slots_per_voice: SlotCount,
    ) -> Result<Self, ProfileError> {
        nonzero(
            "max_instructions_per_program",
            u64::from(max_instructions_per_program.get()),
        )?;
        for (field, value) in [
            ("max_sources_per_program", max_sources_per_program),
            ("max_state_slots_per_program", max_state_slots_per_program),
            ("max_locals_per_program", max_locals_per_program),
            ("max_eval_stack_depth", max_eval_stack_depth),
            ("max_arrays_per_program", max_arrays_per_program),
            ("max_array_elements", max_array_elements),
            ("max_emits_per_program", max_emits_per_program),
            ("mod_matrix_slots_per_voice", mod_matrix_slots_per_voice),
            ("script_host_slots_per_voice", script_host_slots_per_voice),
        ] {
            nonzero(field, u64::from(value.get()))?;
        }
        Ok(Self {
            max_instructions_per_program,
            max_sources_per_program,
            max_state_slots_per_program,
            max_locals_per_program,
            max_eval_stack_depth,
            max_arrays_per_program,
            max_array_elements,
            max_emits_per_program,
            mod_matrix_slots_per_voice,
            script_host_slots_per_voice,
        })
    }

    accessors! {
        max_instructions_per_program: InstructionCount, "Instructions in one script program.";
        max_sources_per_program: SlotCount, "Sources one program may read.";
        max_state_slots_per_program: SlotCount, "State slots one program may keep, times polyphony.";
        max_locals_per_program: SlotCount, "Locals one program may declare.";
        max_eval_stack_depth: SlotCount, "How deep one program's evaluation stack may go.";
        max_arrays_per_program: SlotCount, "Arrays one program may declare.";
        max_array_elements: SlotCount, "Elements in one array.";
        max_emits_per_program: SlotCount, "Emit slots one program may drive.";
        mod_matrix_slots_per_voice: SlotCount, "Mod Matrix slots per voice.";
        script_host_slots_per_voice: SlotCount, "Script host slots per voice. Floored by the above.";
    }
}

/// Recording capacities.
///
/// The profile's only **session limits** (`HOST-INV-020`): how long a take runs is
/// not knowable when the plan is compiled, so reaching one stops the activity with
/// a counted diagnostic and keeps everything already recorded. It never drops or
/// overwrites a note, because a played note is authored data the moment it exists.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct RecordingLimits {
    max_held_notes_per_take: HeldNoteCount,
    max_recorded_events_per_take: EventCount,
}

impl RecordingLimits {
    /// Recording limits.
    pub fn new(
        max_held_notes_per_take: HeldNoteCount,
        max_recorded_events_per_take: EventCount,
    ) -> Result<Self, ProfileError> {
        nonzero(
            "max_held_notes_per_take",
            u64::from(max_held_notes_per_take.get()),
        )?;
        nonzero(
            "max_recorded_events_per_take",
            u64::from(max_recorded_events_per_take.get()),
        )?;
        Ok(Self {
            max_held_notes_per_take,
            max_recorded_events_per_take,
        })
    }

    accessors! {
        max_held_notes_per_take: HeldNoteCount, "Notes one take may hold at once.";
        max_recorded_events_per_take: EventCount, "Events one take may record.";
    }
}

/// The advisory cost budget.
///
/// `HOST-INV-015`: it never refuses. Exceeding it is a `CompileWarning` carrying
/// the predicted and permitted values, and compilation continues — because the
/// cost model behind it is a prediction from V1 measurements taken offline on one
/// machine, and a prediction that weak may warn but may not decide.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct CostBudget {
    predicted_quantum_cost_ratio: CostRatio,
}

impl CostBudget {
    /// The advisory budget. Must be above zero.
    ///
    /// A budget of zero would warn on every plan, including an empty one, which is a
    /// warning that carries no information.
    pub fn new(predicted_quantum_cost_ratio: CostRatio) -> Result<Self, ProfileError> {
        if predicted_quantum_cost_ratio.as_f32() <= 0.0 {
            return Err(ProfileError::Quantity(QuantityError::NotPositive {
                quantity: "predicted_quantum_cost_ratio",
                value: predicted_quantum_cost_ratio.as_f32(),
            }));
        }
        Ok(Self {
            predicted_quantum_cost_ratio,
        })
    }

    accessors! {
        predicted_quantum_cost_ratio: CostRatio, "Predicted median quantum cost over its real-time budget.";
    }
}

/// What the operator budgets. Raisable, at a cost the resource report accounts
/// for — and only within a ceiling the engine owns, where one exists.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct RenderLimits {
    stream: StreamLimits,
    graph: GraphLimits,
    voices: VoiceLimits,
    events: EventLimits,
    observation: ObservationLimits,
    mixing: MixingLimits,
    memory: MemoryLimits,
    script: ScriptLimits,
    recording: RecordingLimits,
    cost: CostBudget,
}

impl RenderLimits {
    /// Assemble the ten groups, validating the one relation that spans two of
    /// them.
    #[allow(
        clippy::too_many_arguments,
        reason = "one argument per limit group; the groups are the specification's own partition"
    )]
    pub fn new(
        stream: StreamLimits,
        graph: GraphLimits,
        voices: VoiceLimits,
        events: EventLimits,
        observation: ObservationLimits,
        mixing: MixingLimits,
        memory: MemoryLimits,
        script: ScriptLimits,
        recording: RecordingLimits,
        cost: CostBudget,
    ) -> Result<Self, ProfileError> {
        if script.script_host_slots_per_voice() < script.mod_matrix_slots_per_voice() {
            return Err(ProfileError::ScriptHostSlotsBelowModMatrix {
                host: script.script_host_slots_per_voice(),
                matrix: script.mod_matrix_slots_per_voice(),
            });
        }
        Ok(Self {
            stream,
            graph,
            voices,
            events,
            observation,
            mixing,
            memory,
            script,
            recording,
            cost,
        })
    }

    /// The specification's defaults, for a stream at these capabilities.
    ///
    /// Two fields depend on the capability half and are computed here rather than
    /// carried as constants:
    ///
    /// - `forward_event_horizon` takes `max(one second at the prepared rate,
    ///   maximum_block_size + Q)`. A flat second was a default that could fail its
    ///   own validation, because `maximum_block_size` is queried and has no
    ///   compiled-in ceiling: a device reporting a block above a second's worth of
    ///   frames would produce a profile refused at construction. Deriving the floor
    ///   removes the case instead of relying on nobody meeting it.
    /// - `max_concurrent_retiring_voices` is derived inside [`VoiceLimits::new`].
    ///
    /// Every other value is the specification's, and each carries its basis there
    /// — queried, derived, carried over from V1, or chosen. None may be tuned to
    /// before its revisit point.
    pub fn engine_defaults(capabilities: HostCapabilities) -> Result<Self, ProfileError> {
        let one_second = capabilities.sample_rate().frames_per_second();
        let block_floor = capabilities
            .maximum_block_size()
            .checked_add(FrameCount::QUANTUM)
            .ok_or(ProfileError::ForwardHorizonBelowFloor {
                horizon: FrameCount::ZERO,
                block: capabilities.maximum_block_size(),
                quantum: QUANTUM_FRAMES,
            })?;
        let horizon = if one_second > block_floor {
            one_second
        } else {
            block_floor
        };

        Self::new(
            StreamLimits::new(SampleRateRange::engine_supported()),
            GraphLimits::new(
                NodeCount::limit(16_384)?,
                EdgeCount::limit(65_536)?,
                FanOut::limit(64)?,
                NodeCount::limit(32)?,
                NodeCount::limit(32)?,
            )?,
            VoiceLimits::new(
                VoiceCount::limit(1)?,
                VoiceCount::limit(128)?,
                VoiceCount::limit(512)?,
                HeldNoteCount::limit(512)?,
                FrameCount::new(128),
            )?,
            EventLimits::new(
                EventCount::limit(256)?,
                EventCount::limit(128)?,
                EventCount::limit(4_096)?,
                horizon,
                EventCount::limit(16_384)?,
                EventCount::limit(256)?,
            )?,
            ObservationLimits::new(
                TapCount::limit(128)?,
                FrameCount::new(4_096),
                FrameCount::new(2_048),
            )?,
            MixingLimits::new(
                MixChannelCount::limit(256)?,
                BusCount::limit(64)?,
                SendCount::limit(16)?,
            )?,
            MemoryLimits::new(
                PreparedBytes::limit(64 * 1024 * 1024)?,
                PreparedBytes::limit(32 * 1024 * 1024)?,
                PreparedBytes::limit(16 * 1024 * 1024)?,
            )?,
            ScriptLimits::new(
                InstructionCount::limit(256)?,
                SlotCount::limit(32)?,
                SlotCount::limit(16)?,
                SlotCount::limit(16)?,
                SlotCount::limit(64)?,
                SlotCount::limit(16)?,
                SlotCount::limit(256)?,
                SlotCount::limit(4)?,
                SlotCount::limit(16)?,
                SlotCount::limit(16)?,
            )?,
            RecordingLimits::new(HeldNoteCount::limit(32)?, EventCount::limit(4_096)?)?,
            CostBudget::new(CostRatio::limit(0.15)?)?,
        )
    }

    accessors! {
        stream: StreamLimits, "Which streams may be prepared.";
        graph: GraphLimits, "Graph extents.";
        voices: VoiceLimits, "Polyphony and retirement.";
        events: EventLimits, "Event capacities.";
        observation: ObservationLimits, "Observation and analysis.";
        mixing: MixingLimits, "Mixing extents.";
        memory: MemoryLimits, "Prepared-memory budgets.";
        script: ScriptLimits, "Per-program script capacities.";
        recording: RecordingLimits, "Recording session limits.";
        cost: CostBudget, "The advisory cost budget.";
    }
}

/// The single immutable preparation input.
///
/// Constructed off the audio thread, validated once, and fixed for the life of one
/// stream epoch. It is **never persisted** (`HOST-INV-010`): it describes the
/// machine and the operator's budgets, not the work, and a project that rendered
/// on one profile must load on another.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct HostProfile {
    capabilities: HostCapabilities,
    limits: RenderLimits,
}

impl HostProfile {
    /// Validate the two halves against each other.
    ///
    /// `HOST-INV-016`: a profile whose fields are mutually inconsistent fails here,
    /// before any plan is compiled, naming the two fields that disagree. There is
    /// no partially valid profile and no clamping constructor.
    pub fn new(capabilities: HostCapabilities, limits: RenderLimits) -> Result<Self, ProfileError> {
        let range = limits.stream().accepted_sample_rates();
        if !range.contains(capabilities.sample_rate()) {
            return Err(ProfileError::SampleRateOutsideAcceptedRange {
                rate: capabilities.sample_rate(),
                range,
            });
        }

        let floor = capabilities
            .maximum_block_size()
            .checked_add(FrameCount::QUANTUM)
            .ok_or(ProfileError::ForwardHorizonBelowFloor {
                horizon: limits.events().forward_event_horizon(),
                block: capabilities.maximum_block_size(),
                quantum: QUANTUM_FRAMES,
            })?;
        if limits.events().forward_event_horizon() < floor {
            return Err(ProfileError::ForwardHorizonBelowFloor {
                horizon: limits.events().forward_event_horizon(),
                block: capabilities.maximum_block_size(),
                quantum: QUANTUM_FRAMES,
            });
        }

        Ok(Self {
            capabilities,
            limits,
        })
    }

    /// A harness profile at the given rate, block size, and layout, with the
    /// specification's default limits.
    ///
    /// The offline and harness paths are why [`CapabilitySource`] exists: there is
    /// no device to query, and the tag records that the capability half was
    /// declared rather than discovered.
    pub fn harness(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
    ) -> Result<Self, ProfileError> {
        let capabilities =
            HostCapabilities::harness(sample_rate, maximum_block_size, channel_layout)?;
        let limits = RenderLimits::engine_defaults(capabilities)?;
        Self::new(capabilities, limits)
    }

    accessors! {
        capabilities: HostCapabilities, "What the host and device can do.";
        limits: RenderLimits, "What the operator budgets.";
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(hz: f32) -> SampleRate {
        SampleRate::new(hz).expect("test rate is valid")
    }

    /// `ScriptLimits::new` with the failure unwrapped, for the same reason.
    #[allow(
        clippy::too_many_arguments,
        reason = "one argument per per-program script capacity, as the constructor has"
    )]
    fn build_script(
        instructions: InstructionCount,
        sources: SlotCount,
        state: SlotCount,
        locals: SlotCount,
        stack: SlotCount,
        arrays: SlotCount,
        elements: SlotCount,
        emits: SlotCount,
        matrix: SlotCount,
        host: SlotCount,
    ) -> ScriptLimits {
        ScriptLimits::new(
            instructions,
            sources,
            state,
            locals,
            stack,
            arrays,
            elements,
            emits,
            matrix,
            host,
        )
        .expect("valid script limits")
    }

    /// `EventLimits::new` with the failure unwrapped, for a test that is not about it.
    fn build_events(
        per_quantum: EventCount,
        expansion: EventCount,
        in_flight: EventCount,
        horizon: FrameCount,
        command: EventCount,
        egress: EventCount,
    ) -> EventLimits {
        EventLimits::new(per_quantum, expansion, in_flight, horizon, command, egress)
            .expect("valid event limits")
    }

    fn harness_profile() -> HostProfile {
        HostProfile::harness(
            rate(48_000.0),
            FrameCount::new(1_024),
            ChannelLayout::Stereo,
        )
        .expect("the default harness profile is valid")
    }

    #[test]
    fn the_default_profile_is_valid() {
        let profile = harness_profile();
        assert_eq!(
            profile.capabilities().source(),
            CapabilitySource::Harness,
            "the harness path must not claim to have queried a device"
        );
    }

    #[test]
    fn a_rate_outside_the_accepted_range_fails_construction_naming_both_fields() {
        // Below the range.
        let low =
            HostCapabilities::harness(rate(4_000.0), FrameCount::new(512), ChannelLayout::Stereo)
                .expect("capabilities alone do not check the range");
        let limits = RenderLimits::engine_defaults(low).expect("defaults are valid");
        match HostProfile::new(low, limits) {
            Err(ProfileError::SampleRateOutsideAcceptedRange { rate, range }) => {
                assert_eq!(rate.as_f32(), 4_000.0);
                assert_eq!(range.minimum().as_f32(), SampleRateRange::MINIMUM_HZ);
            }
            other => panic!("expected a range refusal naming both fields, got {other:?}"),
        }
    }

    #[test]
    fn a_rate_above_the_accepted_range_also_fails() {
        let high =
            HostCapabilities::harness(rate(200_000.0), FrameCount::new(512), ChannelLayout::Stereo)
                .expect("capabilities alone do not check the range");
        let limits = RenderLimits::engine_defaults(high).expect("defaults are valid");
        assert!(matches!(
            HostProfile::new(high, limits),
            Err(ProfileError::SampleRateOutsideAcceptedRange { .. })
        ));
    }

    #[test]
    fn both_endpoints_of_the_accepted_range_are_admissible() {
        for hz in [
            SampleRateRange::MINIMUM_HZ,
            synth_core::audio::DeviceSampleRate::MAX_SUPPORTED.as_f32(),
        ] {
            let capabilities =
                HostCapabilities::harness(rate(hz), FrameCount::new(512), ChannelLayout::Stereo)
                    .expect("valid capabilities");
            let limits = RenderLimits::engine_defaults(capabilities).expect("defaults are valid");
            assert!(
                HostProfile::new(capabilities, limits).is_ok(),
                "{hz} Hz is an inclusive endpoint and must be admissible"
            );
        }
    }

    #[test]
    fn a_horizon_below_the_block_plus_quantum_floor_fails_naming_both_fields() {
        let capabilities = HostCapabilities::harness(
            rate(48_000.0),
            FrameCount::new(1_024),
            ChannelLayout::Stereo,
        )
        .expect("valid capabilities");
        let defaults = RenderLimits::engine_defaults(capabilities).expect("defaults are valid");
        let events = build_events(
            defaults.events().max_events_per_quantum(),
            defaults.events().max_note_expansion_per_tick(),
            defaults.events().max_scheduled_events_in_flight(),
            // One frame below the floor.
            FrameCount::new(1_024 + u64::from(QUANTUM_FRAMES) - 1),
            defaults.events().command_queue_capacity(),
            defaults.events().event_egress_capacity(),
        );
        let limits = RenderLimits::new(
            defaults.stream(),
            defaults.graph(),
            defaults.voices(),
            events,
            defaults.observation(),
            defaults.mixing(),
            defaults.memory(),
            defaults.script(),
            defaults.recording(),
            defaults.cost(),
        )
        .expect("only the profile checks the horizon floor");

        match HostProfile::new(capabilities, limits) {
            Err(ProfileError::ForwardHorizonBelowFloor {
                horizon,
                block,
                quantum,
            }) => {
                assert_eq!(horizon.as_u64(), 1_024 + u64::from(QUANTUM_FRAMES) - 1);
                assert_eq!(block.as_u64(), 1_024);
                assert_eq!(quantum, QUANTUM_FRAMES);
            }
            other => panic!("expected a horizon refusal naming both fields, got {other:?}"),
        }
    }

    #[test]
    fn the_default_horizon_clears_its_floor_at_every_admissible_block_size() {
        // Including a block above one second's worth of frames, which is the case
        // a flat one-second default failed. The device is implausible, which is
        // exactly why testing would not have found it.
        for block in [1_u64, 64, 63, 1_024, 4_096, 48_000, 96_000, 1_000_000] {
            let capabilities = HostCapabilities::harness(
                rate(48_000.0),
                FrameCount::new(block),
                ChannelLayout::Stereo,
            )
            .expect("valid capabilities");
            let limits = RenderLimits::engine_defaults(capabilities).expect("defaults are valid");
            assert!(
                HostProfile::new(capabilities, limits).is_ok(),
                "a {block}-frame maximum block must produce a valid default profile"
            );
        }
    }

    #[test]
    fn a_block_size_below_one_quantum_is_admitted() {
        // `HOST-INV-012`: ADR-0001 clause 6 primes the output carry with `Q`
        // frames of silence precisely so that any callback size works, so a
        // `maximum_block_size >= Q` clause would refuse a host the render model
        // was built for.
        let profile = HostProfile::harness(rate(48_000.0), FrameCount::new(1), ChannelLayout::Mono);
        assert!(profile.is_ok());
    }

    #[test]
    fn a_zero_frame_maximum_block_is_refused() {
        assert!(matches!(
            HostCapabilities::harness(rate(48_000.0), FrameCount::ZERO, ChannelLayout::Mono),
            Err(ProfileError::Quantity(QuantityError::ZeroCapacity { .. }))
        ));
    }

    #[test]
    fn script_host_slots_below_mod_matrix_slots_fail_naming_both() {
        let capabilities =
            HostCapabilities::harness(rate(48_000.0), FrameCount::new(512), ChannelLayout::Stereo)
                .expect("valid capabilities");
        let defaults = RenderLimits::engine_defaults(capabilities).expect("defaults are valid");
        let script = build_script(
            defaults.script().max_instructions_per_program(),
            defaults.script().max_sources_per_program(),
            defaults.script().max_state_slots_per_program(),
            defaults.script().max_locals_per_program(),
            defaults.script().max_eval_stack_depth(),
            defaults.script().max_arrays_per_program(),
            defaults.script().max_array_elements(),
            defaults.script().max_emits_per_program(),
            SlotCount::limit(16).expect("valid slot count"),
            SlotCount::limit(8).expect("valid slot count"),
        );
        let built = RenderLimits::new(
            defaults.stream(),
            defaults.graph(),
            defaults.voices(),
            defaults.events(),
            defaults.observation(),
            defaults.mixing(),
            defaults.memory(),
            script,
            defaults.recording(),
            defaults.cost(),
        );
        match built {
            Err(ProfileError::ScriptHostSlotsBelowModMatrix { host, matrix }) => {
                assert_eq!(host.get(), 8);
                assert_eq!(matrix.get(), 16);
            }
            other => panic!("expected a slot-floor refusal naming both fields, got {other:?}"),
        }
    }

    #[test]
    fn raising_the_host_slots_alone_is_accepted() {
        // V1's assertion is an inequality, and this is the case the single-field
        // model forbade: the script host is module-agnostic, so more host slots
        // than matrix slots is legal.
        let capabilities =
            HostCapabilities::harness(rate(48_000.0), FrameCount::new(512), ChannelLayout::Stereo)
                .expect("valid capabilities");
        let defaults = RenderLimits::engine_defaults(capabilities).expect("defaults are valid");
        let script = build_script(
            defaults.script().max_instructions_per_program(),
            defaults.script().max_sources_per_program(),
            defaults.script().max_state_slots_per_program(),
            defaults.script().max_locals_per_program(),
            defaults.script().max_eval_stack_depth(),
            defaults.script().max_arrays_per_program(),
            defaults.script().max_array_elements(),
            defaults.script().max_emits_per_program(),
            SlotCount::limit(16).expect("valid slot count"),
            SlotCount::limit(64).expect("valid slot count"),
        );
        assert!(
            RenderLimits::new(
                defaults.stream(),
                defaults.graph(),
                defaults.voices(),
                defaults.events(),
                defaults.observation(),
                defaults.mixing(),
                defaults.memory(),
                script,
                defaults.recording(),
                defaults.cost(),
            )
            .is_ok()
        );
    }

    #[test]
    fn a_zero_capacity_is_refused_by_the_group_constructor_and_named_by_field() {
        // The group constructors take already-built newtypes, so `measured(0)` and `NONE`
        // could bypass `limit`'s own zero check and reach admission — where a plan would
        // be refused against a budget of nothing. The diagnostic names the *field*, which
        // is what a reader has to change, rather than the type.
        let error = GraphLimits::new(
            NodeCount::NONE,
            EdgeCount::limit(1).expect("positive"),
            FanOut::limit(1).expect("positive"),
            NodeCount::limit(1).expect("positive"),
            NodeCount::limit(1).expect("positive"),
        );
        match error {
            Err(ProfileError::Quantity(QuantityError::ZeroCapacity { quantity })) => {
                assert_eq!(quantity, "max_nodes");
            }
            other => panic!("expected a zero-capacity refusal naming the field, got {other:?}"),
        }

        // One per group, so a group added without the check fails here.
        assert!(
            EventLimits::new(
                EventCount::NONE,
                EventCount::limit(1).expect("positive"),
                EventCount::limit(1).expect("positive"),
                FrameCount::new(1),
                EventCount::limit(1).expect("positive"),
                EventCount::limit(1).expect("positive"),
            )
            .is_err()
        );
        assert!(
            ObservationLimits::new(TapCount::NONE, FrameCount::new(1), FrameCount::new(1)).is_err()
        );
        assert!(
            MixingLimits::new(
                MixChannelCount::NONE,
                BusCount::limit(1).expect("positive"),
                SendCount::limit(1).expect("positive"),
            )
            .is_err()
        );
        assert!(
            MemoryLimits::new(
                PreparedBytes::NONE,
                PreparedBytes::limit(1).expect("positive"),
                PreparedBytes::limit(1).expect("positive"),
            )
            .is_err()
        );
        assert!(
            RecordingLimits::new(HeldNoteCount::NONE, EventCount::limit(1).expect("positive"))
                .is_err()
        );
        assert!(build_script_is_err());
        assert!(CostBudget::new(CostRatio::measured(0.0).expect("finite")).is_err());
        assert!(
            VoiceLimits::new(
                VoiceCount::NONE,
                VoiceCount::limit(1).expect("positive"),
                VoiceCount::limit(1).expect("positive"),
                HeldNoteCount::limit(1).expect("positive"),
                FrameCount::new(1),
            )
            .is_err()
        );
    }

    /// A zero slot count anywhere in the script group is refused.
    fn build_script_is_err() -> bool {
        ScriptLimits::new(
            InstructionCount::limit(1).expect("positive"),
            SlotCount::NONE,
            SlotCount::limit(1).expect("positive"),
            SlotCount::limit(1).expect("positive"),
            SlotCount::limit(1).expect("positive"),
            SlotCount::limit(1).expect("positive"),
            SlotCount::limit(1).expect("positive"),
            SlotCount::limit(1).expect("positive"),
            SlotCount::limit(1).expect("positive"),
            SlotCount::limit(1).expect("positive"),
        )
        .is_err()
    }

    #[test]
    fn the_retirement_budget_is_derived_and_cannot_bind() {
        let profile = harness_profile();
        assert_eq!(
            profile.limits().voices().max_concurrent_retiring_voices(),
            profile.limits().voices().max_active_voices(),
            "a plan swap retires whatever is sounding, so this budget must not bind"
        );
    }

    #[test]
    fn an_inverted_voice_range_is_refused() {
        assert!(matches!(
            VoiceLimits::new(
                VoiceCount::limit(128).expect("valid"),
                VoiceCount::limit(1).expect("valid"),
                VoiceCount::limit(512).expect("valid"),
                HeldNoteCount::limit(512).expect("valid"),
                FrameCount::new(128),
            ),
            Err(ProfileError::InvertedVoiceRange { .. })
        ));
    }

    #[test]
    fn held_notes_and_voices_do_not_convert() {
        // `HeldNoteCount` and `VoiceCount` are deliberately unconvertible: a held
        // note is a source obligation and a voice is a resource. Their equal
        // default of 512 is a coincidence of value, not a derivation — nothing
        // here can express "equal to `max_active_voices`", which is what keeps
        // Phase 6 from inheriting a comment instead of a rule.
        let profile = harness_profile();
        assert_eq!(profile.limits().voices().max_held_notes().get(), 512);
        assert_eq!(profile.limits().voices().max_active_voices().get(), 512);
    }
}
