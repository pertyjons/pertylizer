//! The resource report: what a plan asked for, what was available, and who asked.
//!
//! `HOST-INV-006` is the whole of this module. Compilation returns a report naming,
//! **for every field**, the requested amount, the available amount, and the dominant
//! contributor to the request — and it returns one whether admission succeeded or
//! failed, because a refusal is a report plus an error, never an error alone.
//!
//! Two shape decisions follow from that:
//!
//! - The rows are **enumerable**: [`ResourceField`] is a closed set with an
//!   exhaustive index, so a field added without a row fails a test rather than
//!   going unreported.
//! - The amounts stay **typed**: [`ResourceAmount`] is one enum whose variants carry
//!   the profile's own newtypes. Enumerability does not require erasing units, and a
//!   generic value-plus-unit pair would let a requested count be compared against
//!   the wrong unit at runtime.

use crate::ir::IrObject;
use crate::quantities::{
    BusCount, ChannelLayout, CostRatio, EdgeCount, EventCount, FanOut, HeldNoteCount,
    InstructionCount, MixChannelCount, NodeCount, PreparedBytes, SampleRate, SampleRateRange,
    ScriptWorkPerQuantum, SendCount, SlotCount, TapCount, VoiceCount,
};
use crate::time::FrameCount;

/// Every profile field that carries an amount.
///
/// The three queried capabilities and all forty-one limits are here. The fourth
/// `HostCapabilities` member, the capability source, is **not** a row: it is a
/// provenance tag with no requested amount to compare, so it is carried in the
/// report's header by [`ResourceReport::capability_source`] instead. Reporting it
/// as a row would mean inventing an amount for a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceField {
    /// The prepared sample rate.
    SampleRate,
    /// The largest callback the host may deliver.
    MaximumBlockSize,
    /// The stream's channel layout.
    ChannelLayout,
    /// The range of rates a stream may be prepared at.
    AcceptedSampleRates,
    /// Nodes in one prepared plan.
    MaxNodes,
    /// Edges in one prepared plan.
    MaxEdges,
    /// Edges leaving one output port.
    MaxFanOutPerPort,
    /// Nodes in one modulation graph.
    MaxModGraphNodes,
    /// Nodes in one note graph.
    MaxNoteGraphNodes,
    /// Voices one instrument may declare.
    VoicesPerInstrument,
    /// Voices sounding at once.
    MaxActiveVoices,
    /// Notes held at once.
    MaxHeldNotes,
    /// Frames a retiring voice crossfades over.
    RetirementCrossfade,
    /// Voices retiring at once.
    MaxConcurrentRetiringVoices,
    /// Events one quantum may be presented with.
    MaxEventsPerQuantum,
    /// Events one tick's note expansion may produce.
    MaxNoteExpansionPerTick,
    /// The scheduler's release window.
    MaxScheduledEventsInFlight,
    /// How far ahead an ingress event may be stamped.
    ForwardEventHorizon,
    /// The live command queue's depth.
    CommandQueueCapacity,
    /// The engine-to-GUI event ring's depth.
    EventEgressCapacity,
    /// Observation taps one plan may register.
    MaxObservationTaps,
    /// The telemetry window, in frames.
    TelemetryRingFrames,
    /// The analyzer's resolution.
    AnalyzerFftSize,
    /// Mix channels in one plan.
    MaxMixChannels,
    /// Buses in one plan.
    MaxBuses,
    /// Sends from one channel.
    MaxSendsPerChannel,
    /// Immutable prepared bytes.
    PreparedImmutableBytes,
    /// Mutable state bytes.
    MutableStateBytes,
    /// Buffer and scratch bytes.
    BufferScratchBytes,
    /// Instructions in one script program.
    MaxInstructionsPerProgram,
    /// Sources one program may read.
    MaxSourcesPerProgram,
    /// State slots one program may keep.
    MaxStateSlotsPerProgram,
    /// Locals one program may declare.
    MaxLocalsPerProgram,
    /// A program's evaluation-stack depth.
    MaxEvalStackDepth,
    /// Arrays one program may declare.
    MaxArraysPerProgram,
    /// Elements in one array.
    MaxArrayElements,
    /// Emit slots one program may drive.
    MaxEmitsPerProgram,
    /// Mod Matrix slots per voice.
    ModMatrixSlotsPerVoice,
    /// Script host slots per voice.
    ScriptHostSlotsPerVoice,
    /// Notes one take may hold.
    MaxHeldNotesPerTake,
    /// Events one take may record.
    MaxRecordedEventsPerTake,
    /// The advisory quantum cost ratio.
    PredictedQuantumCostRatio,
    /// ADR-0046's compiled timeline and automation share.
    CompiledEventShare,
    /// ADR-0046's authored runtime expansion share.
    AuthoredRuntimeEventShare,
    /// ADR-0046's live ingress share.
    LiveEventShare,
    /// ADR-0046's session and transport share.
    SessionEventShare,
    /// ADR-0046's renderer-internal production share.
    InternalEventShare,
    /// ADR-0046's guaranteed release share.
    ReleaseEventShare,
    /// ADR-0046's outstanding non-compiled release obligations.
    ReleaseHoldCapacity,
    /// The live performance-event ingress queue's depth.
    PerformanceIngressCapacity,
}

impl ResourceField {
    /// How many fields carry an amount.
    pub const COUNT: usize = 50;

    /// Every field, once.
    pub const ALL: [Self; Self::COUNT] = [
        Self::SampleRate,
        Self::MaximumBlockSize,
        Self::ChannelLayout,
        Self::AcceptedSampleRates,
        Self::MaxNodes,
        Self::MaxEdges,
        Self::MaxFanOutPerPort,
        Self::MaxModGraphNodes,
        Self::MaxNoteGraphNodes,
        Self::VoicesPerInstrument,
        Self::MaxActiveVoices,
        Self::MaxHeldNotes,
        Self::RetirementCrossfade,
        Self::MaxConcurrentRetiringVoices,
        Self::MaxEventsPerQuantum,
        Self::MaxNoteExpansionPerTick,
        Self::MaxScheduledEventsInFlight,
        Self::ForwardEventHorizon,
        Self::CommandQueueCapacity,
        Self::EventEgressCapacity,
        Self::MaxObservationTaps,
        Self::TelemetryRingFrames,
        Self::AnalyzerFftSize,
        Self::MaxMixChannels,
        Self::MaxBuses,
        Self::MaxSendsPerChannel,
        Self::PreparedImmutableBytes,
        Self::MutableStateBytes,
        Self::BufferScratchBytes,
        Self::MaxInstructionsPerProgram,
        Self::MaxSourcesPerProgram,
        Self::MaxStateSlotsPerProgram,
        Self::MaxLocalsPerProgram,
        Self::MaxEvalStackDepth,
        Self::MaxArraysPerProgram,
        Self::MaxArrayElements,
        Self::MaxEmitsPerProgram,
        Self::ModMatrixSlotsPerVoice,
        Self::ScriptHostSlotsPerVoice,
        Self::MaxHeldNotesPerTake,
        Self::MaxRecordedEventsPerTake,
        Self::PredictedQuantumCostRatio,
        Self::CompiledEventShare,
        Self::AuthoredRuntimeEventShare,
        Self::LiveEventShare,
        Self::SessionEventShare,
        Self::InternalEventShare,
        Self::ReleaseEventShare,
        Self::ReleaseHoldCapacity,
        Self::PerformanceIngressCapacity,
    ];

    /// This field's position in [`Self::ALL`].
    ///
    /// The match is exhaustive on purpose: adding a variant fails the build here,
    /// and a variant added to the match but not to `ALL` fails the enumeration test.
    /// Between them a new field cannot reach the profile without reaching the report.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::SampleRate => 0,
            Self::MaximumBlockSize => 1,
            Self::ChannelLayout => 2,
            Self::AcceptedSampleRates => 3,
            Self::MaxNodes => 4,
            Self::MaxEdges => 5,
            Self::MaxFanOutPerPort => 6,
            Self::MaxModGraphNodes => 7,
            Self::MaxNoteGraphNodes => 8,
            Self::VoicesPerInstrument => 9,
            Self::MaxActiveVoices => 10,
            Self::MaxHeldNotes => 11,
            Self::RetirementCrossfade => 12,
            Self::MaxConcurrentRetiringVoices => 13,
            Self::MaxEventsPerQuantum => 14,
            Self::MaxNoteExpansionPerTick => 15,
            Self::MaxScheduledEventsInFlight => 16,
            Self::ForwardEventHorizon => 17,
            Self::CommandQueueCapacity => 18,
            Self::EventEgressCapacity => 19,
            Self::MaxObservationTaps => 20,
            Self::TelemetryRingFrames => 21,
            Self::AnalyzerFftSize => 22,
            Self::MaxMixChannels => 23,
            Self::MaxBuses => 24,
            Self::MaxSendsPerChannel => 25,
            Self::PreparedImmutableBytes => 26,
            Self::MutableStateBytes => 27,
            Self::BufferScratchBytes => 28,
            Self::MaxInstructionsPerProgram => 29,
            Self::MaxSourcesPerProgram => 30,
            Self::MaxStateSlotsPerProgram => 31,
            Self::MaxLocalsPerProgram => 32,
            Self::MaxEvalStackDepth => 33,
            Self::MaxArraysPerProgram => 34,
            Self::MaxArrayElements => 35,
            Self::MaxEmitsPerProgram => 36,
            Self::ModMatrixSlotsPerVoice => 37,
            Self::ScriptHostSlotsPerVoice => 38,
            Self::MaxHeldNotesPerTake => 39,
            Self::MaxRecordedEventsPerTake => 40,
            Self::PredictedQuantumCostRatio => 41,
            Self::CompiledEventShare => 42,
            Self::AuthoredRuntimeEventShare => 43,
            Self::LiveEventShare => 44,
            Self::SessionEventShare => 45,
            Self::InternalEventShare => 46,
            Self::ReleaseEventShare => 47,
            Self::ReleaseHoldCapacity => 48,
            Self::PerformanceIngressCapacity => 49,
        }
    }

    /// The field's name, for a diagnostic a reader has to act on.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SampleRate => "sample_rate",
            Self::MaximumBlockSize => "maximum_block_size",
            Self::ChannelLayout => "channel_layout",
            Self::AcceptedSampleRates => "accepted_sample_rates",
            Self::MaxNodes => "max_nodes",
            Self::MaxEdges => "max_edges",
            Self::MaxFanOutPerPort => "max_fan_out_per_port",
            Self::MaxModGraphNodes => "max_mod_graph_nodes",
            Self::MaxNoteGraphNodes => "max_note_graph_nodes",
            Self::VoicesPerInstrument => "voices_per_instrument",
            Self::MaxActiveVoices => "max_active_voices",
            Self::MaxHeldNotes => "max_held_notes",
            Self::RetirementCrossfade => "retirement_crossfade",
            Self::MaxConcurrentRetiringVoices => "max_concurrent_retiring_voices",
            Self::MaxEventsPerQuantum => "max_events_per_quantum",
            Self::MaxNoteExpansionPerTick => "max_note_expansion_per_tick",
            Self::MaxScheduledEventsInFlight => "max_scheduled_events_in_flight",
            Self::ForwardEventHorizon => "forward_event_horizon",
            Self::CommandQueueCapacity => "command_queue_capacity",
            Self::EventEgressCapacity => "event_egress_capacity",
            Self::MaxObservationTaps => "max_observation_taps",
            Self::TelemetryRingFrames => "telemetry_ring_frames",
            Self::AnalyzerFftSize => "analyzer_fft_size",
            Self::MaxMixChannels => "max_mix_channels",
            Self::MaxBuses => "max_buses",
            Self::MaxSendsPerChannel => "max_sends_per_channel",
            Self::PreparedImmutableBytes => "prepared_immutable_bytes",
            Self::MutableStateBytes => "mutable_state_bytes",
            Self::BufferScratchBytes => "buffer_scratch_bytes",
            Self::MaxInstructionsPerProgram => "max_instructions_per_program",
            Self::MaxSourcesPerProgram => "max_sources_per_program",
            Self::MaxStateSlotsPerProgram => "max_state_slots_per_program",
            Self::MaxLocalsPerProgram => "max_locals_per_program",
            Self::MaxEvalStackDepth => "max_eval_stack_depth",
            Self::MaxArraysPerProgram => "max_arrays_per_program",
            Self::MaxArrayElements => "max_array_elements",
            Self::MaxEmitsPerProgram => "max_emits_per_program",
            Self::ModMatrixSlotsPerVoice => "mod_matrix_slots_per_voice",
            Self::ScriptHostSlotsPerVoice => "script_host_slots_per_voice",
            Self::MaxHeldNotesPerTake => "max_held_notes_per_take",
            Self::MaxRecordedEventsPerTake => "max_recorded_events_per_take",
            Self::PredictedQuantumCostRatio => "predicted_quantum_cost_ratio",
            Self::CompiledEventShare => "compiled_event_share",
            Self::AuthoredRuntimeEventShare => "authored_runtime_event_share",
            Self::LiveEventShare => "live_event_share",
            Self::SessionEventShare => "session_event_share",
            Self::InternalEventShare => "internal_event_share",
            Self::ReleaseEventShare => "release_event_share",
            Self::ReleaseHoldCapacity => "release_hold_capacity",
            Self::PerformanceIngressCapacity => "performance_ingress_capacity",
        }
    }

    /// Whether exceeding this field warns instead of refusing.
    ///
    /// `HOST-INV-015`: an advisory budget never produces a `CompileError`. Exactly
    /// one field is advisory, and the cost model behind it is why — a prediction
    /// from V1 measurements taken offline on one machine may warn, but may not
    /// decide.
    #[must_use]
    pub const fn is_advisory(self) -> bool {
        matches!(self, Self::PredictedQuantumCostRatio)
    }

    /// Whether a *plan* can exceed this field at admission.
    ///
    /// `HOST-INV-007` binds the limits a plan can exceed, and its conformance row asks
    /// for one refusal case per such limit — so this predicate has to be exactly the
    /// set those cases can be written for. Thirty-two fields qualify. The eighteen
    /// that do not take that refusal fall into seven groups, each excluded for its own
    /// reason:
    ///
    /// - **The three queried capabilities.** A capability describes what the plan is
    ///   *prepared against*, not a budget it spends.
    /// - **`accepted_sample_rates`.** The named exception: no plan carries a rate, so
    ///   the rate and the range can only disagree inside one profile, which
    ///   construction refuses before any plan is compiled.
    /// - **The sizing fields** — `retirement_crossfade`, `telemetry_ring_frames`,
    ///   `analyzer_fft_size`. They bound nothing, so asking which behaviour they take
    ///   is a category error.
    /// - **The capacities a plan does not request**: `forward_event_horizon`, the three
    ///   queue depths, the two per-voice slot counts whose relation construction
    ///   validates, and `max_concurrent_retiring_voices`, which is derived so that it
    ///   cannot bind. Their rows report the profile's own value, so exceeding is not
    ///   reachable.
    /// - **The advisory cost budget.** `predicted_quantum_cost_ratio` may be exceeded,
    ///   but `HOST-INV-015` makes that a `CompileWarning` rather than a `CompileError`,
    ///   which is the same rule [`Self::is_advisory`] states.
    /// - **`max_events_per_quantum`**, which a plan no longer requests directly. Its
    ///   statically knowable declaration is compiled work and is checked against
    ///   `compiled_event_share`; the cap cannot be exceeded without a share being exceeded
    ///   first, because the shares sum to at most the cap.
    /// - **Two of ADR-0046's seven producer fields**, and only these two. Five have left this
    ///   list, each because a plan states the quantity the share bounds:
    ///   `compiled_event_share`, against the statically knowable events a plan declares;
    ///   `release_hold_capacity`, against its note-on producers' hold entitlements;
    ///   `session_event_share`, against the prepared targets a locate restores at once plus
    ///   ADR-0050 clause 5's boundary mass release; and — since `PlanDeclarations` gained
    ///   [`crate::ir::AuthoredSourceDeclaration`] and
    ///   [`crate::ir::InternalProducerDeclaration`] — `authored_runtime_event_share`, against
    ///   ADR-0046 clause 5's summed destination envelopes, and `internal_event_share`,
    ///   against clause 1's sum of admitted per-quantum maxima.
    /// - The **live** and **release** shares stay, for a reason a declaration would not
    ///   remove: they bound a runtime queue rather than a plan. ADR-0046 clause 6 charges
    ///   them at publication, where `Publication::charge` refuses above the share. That is
    ///   not plan admission, so claiming a plan can exceed them would leave
    ///   `HOST-INV-007`'s conformance row asking for a refusal case that cannot be written.
    ///
    /// An earlier revision of this predicate excluded only eight fields, which made
    /// `HOST-INV-007`'s conformance row unsatisfiable: six of the remaining fields
    /// compare a value against itself, and no plan can be built that exceeds one.
    ///
    /// Eighteen fields are excluded and thirty-two qualify.
    #[must_use]
    pub const fn is_admission_checked(self) -> bool {
        !matches!(
            self,
            Self::SampleRate
                | Self::MaximumBlockSize
                | Self::ChannelLayout
                | Self::AcceptedSampleRates
                | Self::RetirementCrossfade
                | Self::MaxConcurrentRetiringVoices
                | Self::ForwardEventHorizon
                | Self::CommandQueueCapacity
                | Self::EventEgressCapacity
                | Self::TelemetryRingFrames
                | Self::AnalyzerFftSize
                | Self::ModMatrixSlotsPerVoice
                | Self::ScriptHostSlotsPerVoice
                | Self::PredictedQuantumCostRatio
                | Self::MaxEventsPerQuantum
                | Self::LiveEventShare
                | Self::ReleaseEventShare
                | Self::PerformanceIngressCapacity
        )
    }
}

impl std::fmt::Display for ResourceField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A number of events larger than an [`EventCount`] can name.
///
/// The domain concept is "the same unit, past the point its counter holds", and the invariant
/// is exactly that: a value at or below `u32::MAX` is not one of these, because such a value
/// *is* an [`EventCount`] and belongs in [`ResourceAmount::Events`]. Construction validates
/// it, so a caller cannot build a wide amount that would have fit and make a report claim two
/// different things about the same figure. An independent review found the tuple variant it
/// replaces constructible with `1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct EventsBeyondCount(u64);

impl EventsBeyondCount {
    /// The amount, if it genuinely exceeds what an [`EventCount`] names.
    pub const fn new(total: u64) -> Option<Self> {
        if total > u32::MAX as u64 {
            Some(Self(total))
        } else {
            None
        }
    }

    /// The raw total.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for EventsBeyondCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} events", self.0)
    }
}

/// One amount, with its unit intact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResourceAmount {
    /// A number of nodes.
    Nodes(NodeCount),
    /// A number of edges.
    Edges(EdgeCount),
    /// Edges leaving one port.
    FanOut(FanOut),
    /// A number of voices.
    Voices(VoiceCount),
    /// A range of voice counts.
    VoiceRange(VoiceCount, VoiceCount),
    /// A number of held notes.
    HeldNotes(HeldNoteCount),
    /// A number of events.
    Events(EventCount),
    /// A number of events larger than an [`EventCount`] can name.
    ///
    /// `HOST-INV-006` requires every row to name the amount a plan **requested**, and a
    /// declared aggregate can sum past `u32::MAX` — two authored sources, or the compiled
    /// floor added to a retained-future sum. Saturating the row to `u32::MAX` was the earlier
    /// behaviour and it made the report lie twice over: it understated the request, and
    /// against a profile whose limit is `u32::MAX` — construction accepts one, since
    /// `EventCount::limit` refuses only zero — it read [`Fit::Within`] for a total it
    /// exceeded. A merge-gate review held that against the invariant.
    ///
    /// This variant is not a second unit. It is the same unit past the point its counter can
    /// hold, which is why it compares against [`Self::Events`] **in both directions** rather
    /// than mismatching, and why [`EventsBeyondCount`] validates that the value would not
    /// have fit.
    EventsBeyondCount(EventsBeyondCount),
    /// A number of frames.
    Frames(FrameCount),
    /// A number of taps.
    Taps(TapCount),
    /// A number of slots.
    Slots(SlotCount),
    /// A number of script instructions.
    Instructions(InstructionCount),
    /// A number of bytes.
    Bytes(PreparedBytes),
    /// A number of mix channels.
    MixChannels(MixChannelCount),
    /// A number of buses.
    Buses(BusCount),
    /// A number of sends.
    Sends(SendCount),
    /// A dimensionless ratio.
    Ratio(CostRatio),
    /// A channel layout.
    Layout(ChannelLayout),
    /// A sample rate.
    Rate(SampleRate),
    /// A range of sample rates.
    RateRange(SampleRateRange),
}

/// How a requested amount stands against an available one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// The request fits.
    Within,
    /// The request is larger than what is available.
    Exceeds,
    /// The two amounts are not the same unit, which is an internal defect rather
    /// than a plan's problem.
    UnitMismatch,
}

impl ResourceAmount {
    /// Whether this request fits inside `available`.
    ///
    /// A unit mismatch is reported rather than assumed away: it means the report
    /// was built with a row comparing two different things, which a test asserts
    /// never happens for any field.
    #[must_use]
    pub fn fits_within(self, available: Self) -> Fit {
        let within = match (self, available) {
            (Self::Nodes(a), Self::Nodes(b)) => a <= b,
            (Self::Edges(a), Self::Edges(b)) => a <= b,
            (Self::FanOut(a), Self::FanOut(b)) => a <= b,
            (Self::Voices(a), Self::Voices(b)) => a <= b,
            // A declared per-instrument count fits a permitted range when it is
            // inside both endpoints. A plan declaring no voices declares no
            // instrument either, so zero fits.
            (Self::Voices(declared), Self::VoiceRange(low, high)) => {
                declared.get() == 0 || (declared >= low && declared <= high)
            }
            (Self::VoiceRange(a_low, a_high), Self::VoiceRange(b_low, b_high)) => {
                a_low >= b_low && a_high <= b_high
            }
            (Self::HeldNotes(a), Self::HeldNotes(b)) => a <= b,
            (Self::Events(a), Self::Events(b)) => a <= b,
            // A request past what an `EventCount` names is past every available `EventCount`,
            // so this is `false` without inspecting either side — the available amount is a
            // profile field and cannot exceed `u32::MAX`. Stated as a comparison rather than
            // a bare `false` so a reader can check the reasoning against the types.
            (Self::EventsBeyondCount(a), Self::Events(b)) => a.get() <= u64::from(b.get()),
            // The other direction, which an independent review found missing: an ordinary
            // request against an over-large available amount is the same unit and always
            // fits. Reporting `UnitMismatch` there would have claimed an internal defect for
            // a comparison that is simply true.
            (Self::Events(a), Self::EventsBeyondCount(b)) => u64::from(a.get()) <= b.get(),
            // Two over-large requests can still be ordered, which matters if a later field
            // ever compares one against another rather than against a profile value.
            (Self::EventsBeyondCount(a), Self::EventsBeyondCount(b)) => a.get() <= b.get(),
            (Self::Frames(a), Self::Frames(b)) => a <= b,
            (Self::Taps(a), Self::Taps(b)) => a <= b,
            (Self::Slots(a), Self::Slots(b)) => a <= b,
            (Self::Instructions(a), Self::Instructions(b)) => a <= b,
            (Self::Bytes(a), Self::Bytes(b)) => a <= b,
            (Self::MixChannels(a), Self::MixChannels(b)) => a <= b,
            (Self::Buses(a), Self::Buses(b)) => a <= b,
            (Self::Sends(a), Self::Sends(b)) => a <= b,
            (Self::Ratio(a), Self::Ratio(b)) => a.as_f32() <= b.as_f32(),
            // A kind either matches or does not; there is no ordering.
            (Self::Layout(a), Self::Layout(b)) => a == b,
            (Self::Rate(a), Self::Rate(b)) => a.as_f32() <= b.as_f32(),
            (Self::Rate(rate), Self::RateRange(range)) => range.contains(rate),
            (Self::RateRange(a), Self::RateRange(b)) => a == b,
            _ => return Fit::UnitMismatch,
        };
        if within { Fit::Within } else { Fit::Exceeds }
    }
}

impl std::fmt::Display for ResourceAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nodes(v) => write!(f, "{v}"),
            Self::Edges(v) => write!(f, "{v}"),
            Self::FanOut(v) => write!(f, "{v}"),
            Self::Voices(v) => write!(f, "{v}"),
            Self::VoiceRange(low, high) => write!(f, "{low} to {high}"),
            Self::HeldNotes(v) => write!(f, "{v}"),
            Self::Events(v) => write!(f, "{v}"),
            Self::EventsBeyondCount(v) => write!(f, "{v}"),
            Self::Frames(v) => write!(f, "{v}"),
            Self::Taps(v) => write!(f, "{v}"),
            Self::Slots(v) => write!(f, "{v}"),
            Self::Instructions(v) => write!(f, "{v}"),
            Self::Bytes(v) => write!(f, "{v}"),
            Self::MixChannels(v) => write!(f, "{v}"),
            Self::Buses(v) => write!(f, "{v}"),
            Self::Sends(v) => write!(f, "{v}"),
            Self::Ratio(v) => write!(f, "{v}"),
            Self::Layout(v) => write!(f, "{v}"),
            Self::Rate(v) => write!(f, "{v}"),
            Self::RateRange(v) => write!(f, "{v}"),
        }
    }
}

/// One field's line in the report.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ResourceRow {
    field: ResourceField,
    requested: ResourceAmount,
    available: ResourceAmount,
    contributor: IrObject,
}

impl ResourceRow {
    /// A row.
    pub const fn new(
        field: ResourceField,
        requested: ResourceAmount,
        available: ResourceAmount,
        contributor: IrObject,
    ) -> Self {
        Self {
            field,
            requested,
            available,
            contributor,
        }
    }

    /// Which field this row is about.
    pub const fn field(&self) -> ResourceField {
        self.field
    }

    /// What the plan asked for.
    pub const fn requested(&self) -> ResourceAmount {
        self.requested
    }

    /// What the profile allowed.
    pub const fn available(&self) -> ResourceAmount {
        self.available
    }

    /// The object that contributed most to the request.
    pub const fn contributor(&self) -> IrObject {
        self.contributor
    }

    /// How the request stands against the allowance.
    pub fn fit(&self) -> Fit {
        self.requested.fits_within(self.available)
    }
}

/// Something that adds latency, named rather than implicit.
///
/// ADR-0001 clause 7 charges a constant `Q` frames unconditionally and requires it
/// to be "a named contributor in the plan's latency accounting", because a latency
/// that is implicit is a latency nobody compensates. Its risk control says so
/// directly: the carry latency must appear in the report ADR-0022 consumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LatencyContributor {
    /// The render quantum's carry buffers.
    RenderQuantumCarry,
}

impl std::fmt::Display for LatencyContributor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RenderQuantumCarry => f.write_str("render quantum carry"),
        }
    }
}

/// What the plan's latency is made of.
#[derive(Debug, Clone, PartialEq, Default)]
#[must_use]
pub struct LatencyAccounting {
    contributors: Vec<(LatencyContributor, FrameCount)>,
}

impl LatencyAccounting {
    /// Record a contributor.
    pub fn with(mut self, contributor: LatencyContributor, frames: FrameCount) -> Self {
        self.contributors.push((contributor, frames));
        self
    }

    /// Every contributor, in the order they were recorded.
    pub fn contributors(&self) -> &[(LatencyContributor, FrameCount)] {
        &self.contributors
    }

    /// The total, or `None` if the contributions overflow.
    pub fn total(&self) -> Option<FrameCount> {
        self.contributors
            .iter()
            .try_fold(FrameCount::ZERO, |sum, (_, frames)| {
                sum.checked_add(*frames)
            })
    }

    /// The frames one named contributor adds, if it is present.
    pub fn frames_of(&self, contributor: LatencyContributor) -> Option<FrameCount> {
        self.contributors
            .iter()
            .find(|(named, _)| *named == contributor)
            .map(|(_, frames)| *frames)
    }
}

/// Quantities the report carries with no threshold attached.
///
/// The script-work aggregate is the case this exists for. Instructions times scopes
/// times voices is what actually costs CPU, and nothing in V1 or in the evidence
/// measures what an instruction costs — so it is computed and reported from Phase 1,
/// and becomes a `RenderLimits` field when Phase 7 can justify a number. A limit
/// with no value is not a limit.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ReportedQuantities {
    script_instructions_per_quantum: ScriptWorkPerQuantum,
    script_work_contributor: IrObject,
}

impl ReportedQuantities {
    /// The reported quantities.
    pub const fn new(
        script_instructions_per_quantum: ScriptWorkPerQuantum,
        script_work_contributor: IrObject,
    ) -> Self {
        Self {
            script_instructions_per_quantum,
            script_work_contributor,
        }
    }

    /// Script instructions evaluated per quantum across the plan.
    pub const fn script_instructions_per_quantum(&self) -> ScriptWorkPerQuantum {
        self.script_instructions_per_quantum
    }

    /// The program contributing most of that work.
    pub const fn script_work_contributor(&self) -> IrObject {
        self.script_work_contributor
    }
}

/// What compilation says about a plan's resources.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct ResourceReport {
    rows: Vec<ResourceRow>,
    latency: LatencyAccounting,
    reported: ReportedQuantities,
    capability_source: crate::profile::CapabilitySource,
    arena_estimated: bool,
}

impl ResourceReport {
    /// Assemble a report.
    pub const fn new(
        rows: Vec<ResourceRow>,
        latency: LatencyAccounting,
        reported: ReportedQuantities,
        capability_source: crate::profile::CapabilitySource,
    ) -> Self {
        Self {
            rows,
            latency,
            reported,
            capability_source,
            arena_estimated: false,
        }
    }

    /// Mark the arena row as an upper bound rather than a measurement.
    ///
    /// The arena's size is a function of the *assignment*, and a report produced before
    /// one exists can only state one buffer per signal. A plan refused before lowering
    /// therefore carries a scratch row that may be far above what it would have taken —
    /// a chain of gains collapses to a single buffer — and a reader has no way to tell
    /// unless the report says so.
    pub(crate) const fn with_estimated_arena(mut self) -> Self {
        self.arena_estimated = true;
        self
    }

    /// Whether the arena row is an upper bound rather than what preparation would take.
    ///
    /// True only on a report that accompanies a refusal made before lowering.
    pub const fn arena_is_estimated(&self) -> bool {
        self.arena_estimated
    }

    /// Every row.
    pub fn rows(&self) -> &[ResourceRow] {
        &self.rows
    }

    /// The row for one field, if the report has it.
    pub fn row(&self, field: ResourceField) -> Option<&ResourceRow> {
        self.rows.iter().find(|row| row.field() == field)
    }

    /// What the plan's latency is made of.
    pub const fn latency(&self) -> &LatencyAccounting {
        &self.latency
    }

    /// Quantities carried without a threshold.
    pub const fn reported(&self) -> &ReportedQuantities {
        &self.reported
    }

    /// Whether the capability half was queried or declared.
    pub const fn capability_source(&self) -> crate::profile::CapabilitySource {
        self.capability_source
    }

    /// Every field whose request exceeds its allowance, in the order this report holds
    /// its rows.
    ///
    /// For a report the compiler built that is [`ResourceField::ALL`]'s order, which
    /// `voice_nodes`' report test asserts over the whole row sequence. It is **not** a
    /// property of every report: [`Self::new`] is public and accepts any sequence
    /// without sorting or validating it, so a caller that assembles its own rows gets
    /// them back as it supplied them. An earlier revision promised "field order" without
    /// that distinction, which was true only by coincidence of how the compiler happened
    /// to push.
    pub fn exceeded(&self) -> impl Iterator<Item = &ResourceRow> {
        self.rows.iter().filter(|row| row.fit() == Fit::Exceeds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_appears_in_all_exactly_once() {
        // `index` is exhaustive, so a new variant fails the build. This is the
        // other half: a variant added to `index` but not to `ALL` would leave a
        // profile field with no report row, which is the failure `HOST-INV-006`
        // exists to prevent.
        let mut seen = [false; ResourceField::COUNT];
        for field in ResourceField::ALL {
            let index = field.index();
            assert!(
                !seen[index],
                "{field} shares index {index} with another field"
            );
            seen[index] = true;
        }
        assert!(
            seen.iter().all(|found| *found),
            "ResourceField::ALL does not cover every index in 0..{}",
            ResourceField::COUNT
        );
    }

    #[test]
    fn field_names_are_unique() {
        for (index, field) in ResourceField::ALL.iter().enumerate() {
            for other in &ResourceField::ALL[index + 1..] {
                assert_ne!(
                    field.name(),
                    other.name(),
                    "{field} and {other} share a name, so a diagnostic cannot tell them apart"
                );
            }
        }
    }

    #[test]
    fn exactly_one_field_is_advisory() {
        let advisory: Vec<_> = ResourceField::ALL
            .into_iter()
            .filter(|field| field.is_advisory())
            .collect();
        assert_eq!(advisory, vec![ResourceField::PredictedQuantumCostRatio]);
    }

    #[test]
    fn mismatched_units_are_reported_rather_than_compared() {
        let nodes = ResourceAmount::Nodes(NodeCount::measured(1));
        let bytes = ResourceAmount::Bytes(PreparedBytes::measured(1));
        assert_eq!(nodes.fits_within(bytes), Fit::UnitMismatch);
    }

    #[test]
    fn a_request_within_its_allowance_fits() {
        let requested = ResourceAmount::Nodes(NodeCount::measured(3));
        let available = ResourceAmount::Nodes(NodeCount::measured(4));
        assert_eq!(requested.fits_within(available), Fit::Within);
        assert_eq!(available.fits_within(requested), Fit::Exceeds);
    }

    #[test]
    fn a_declared_voice_count_is_measured_against_both_endpoints() {
        let low = VoiceCount::measured(1);
        let high = VoiceCount::measured(128);
        let range = ResourceAmount::VoiceRange(low, high);
        assert_eq!(
            ResourceAmount::Voices(VoiceCount::measured(64)).fits_within(range),
            Fit::Within
        );
        assert_eq!(
            ResourceAmount::Voices(VoiceCount::measured(129)).fits_within(range),
            Fit::Exceeds
        );
        // Zero means "no instrument", not "below the minimum".
        assert_eq!(
            ResourceAmount::Voices(VoiceCount::NONE).fits_within(range),
            Fit::Within
        );
    }

    #[test]
    fn latency_names_its_contributors_and_sums_them() {
        let latency = LatencyAccounting::default()
            .with(LatencyContributor::RenderQuantumCarry, FrameCount::QUANTUM);
        assert_eq!(
            latency.frames_of(LatencyContributor::RenderQuantumCarry),
            Some(FrameCount::QUANTUM)
        );
        assert_eq!(latency.total(), Some(FrameCount::QUANTUM));
    }
}
