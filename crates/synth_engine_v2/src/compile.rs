//! Admission: compiling a plan against one profile.
//!
//! Admission happens **off the audio thread, once per prepared plan**, and it has
//! exactly two outcomes: a prepared plan, or a refusal with an attributable
//! diagnostic. It never truncates, clamps, or drops to make a plan fit — exceeding a
//! limit may not rewrite authored data — and it returns a resource report either
//! way, because a refusal is a report plus an error and never an error alone.

use std::collections::HashMap;

use crate::arena::{self, ArenaPolicy};
use crate::diagnostics::{CompileError, CompileWarning};
use crate::ir::{GraphIr, IrNodeKind, IrObject, NodeId, PortId};
use crate::node::kernels::{MAX_INPUTS, PreparedNode};
use crate::node::{self, NodeDescriptor};
use crate::plan::{
    BufferSlot, CompiledPlan, NodeSlot, NodeStep, NoteAddress, NoteSlot, NoteTarget,
    ParameterAddress, ParameterSlot, ParameterTarget, PlanOp, issue_plan_id,
};
use crate::profile::HostProfile;
use crate::quantities::{
    ChannelLayout, EdgeCount, InstructionCount, NodeCount, PreparedBytes, RecordCount, SlotCount,
    TapCount,
};
use crate::report::{
    Fit, LatencyAccounting, LatencyContributor, ReportedQuantities, ResourceAmount, ResourceField,
    ResourceReport, ResourceRow,
};
use crate::time::{FrameCount, QUANTUM_FRAMES};
use crate::validate::{Validated, validate};

/// The preparation input.
///
/// It carries the profile and nothing the profile already owns. The master plan's
/// sketch also held a `sample_rate`, which the `Current` specification puts in
/// `HostCapabilities`; carrying both would give one stream two rates, and the plan is
/// updated in the same change that removed it — the same footing as ADR-0001's
/// removal of `quantum` from this struct.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct RenderConfig {
    host_profile: HostProfile,
}

impl RenderConfig {
    /// A configuration over one profile.
    pub const fn new(host_profile: HostProfile) -> Self {
        Self { host_profile }
    }

    /// The profile a plan is admitted against.
    pub const fn host_profile(&self) -> &HostProfile {
        &self.host_profile
    }
}

/// Everything compilation has to say.
///
/// The report is present whether or not a plan came out, which is `HOST-INV-006`.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct CompileOutcome {
    report: ResourceReport,
    warnings: Vec<CompileWarning>,
    plan: Result<CompiledPlan, CompileError>,
}

impl CompileOutcome {
    /// What the plan asked for and what was available.
    pub const fn report(&self) -> &ResourceReport {
        &self.report
    }

    /// Advisory findings. An admitted plan may still have them.
    pub fn warnings(&self) -> &[CompileWarning] {
        &self.warnings
    }

    /// The prepared plan, or why there is none.
    pub const fn plan(&self) -> Result<&CompiledPlan, &CompileError> {
        match &self.plan {
            Ok(plan) => Ok(plan),
            Err(error) => Err(error),
        }
    }

    /// Take the prepared plan, dropping the report.
    pub fn into_plan(self) -> Result<CompiledPlan, CompileError> {
        self.plan
    }
}

/// Compile `ir` against `config`.
///
/// The order is deliberate, and P02-T004 changed it. It is, exactly:
///
/// 1. build a **preflight** report over an arena size that can be known without an
///    assignment — one buffer per signal, an upper bound, and the report says so;
/// 2. **validate the structure**, because an invalid cable is the actionable diagnostic
///    and refusing a malformed graph on a limit instead would hide it;
/// 3. refuse on any admission-checked field **before** the arena row, so a graph the
///    profile refuses outright never reaches lowering or liveness analysis;
/// 4. lower, assign the arena, and rebuild the report over what it actually takes;
/// 5. refuse on any remaining field.
///
/// Field order decides which refusal a plan gets, so step 3 stops at the arena row: a
/// later field must not be reported ahead of a scratch overrun the exact figure would
/// have found. Structure moved ahead of the limits because the arena's size is a
/// function of the assignment — reuse means a plan allocates fewer buffers than it has
/// signals — and a report built before lowering can only state an upper bound, which
/// refuses plans that fit. Every refusal still carries a report, which `HOST-INV-006`
/// admits no exception to; a refusal from step 2 or 3 carries the preflight one, marked
/// as estimated.
pub fn compile(ir: &GraphIr, config: &RenderConfig) -> CompileOutcome {
    compile_with(ir, config, ArenaPolicy::Reuse)
}

/// Compile under a chosen arena policy.
///
/// Crate-private, and [`ArenaPolicy::NoReuse`] exists for ADR-0005 clause 8's
/// behavioural check alone: the same plan compiled both ways must render bit-identical
/// audio. It is not reachable from a host profile and is not a supported configuration.
pub(crate) fn compile_with(
    ir: &GraphIr,
    config: &RenderConfig,
    policy: ArenaPolicy,
) -> CompileOutcome {
    let profile = config.host_profile();
    let mut warnings = Vec::new();

    // The report a refused plan carries, over the arena size that can be known before an
    // assignment exists. Advisory findings are collected from it on both refusal paths,
    // so a report showing an overrun is never returned with no warning to match.
    let preflight = build_report(
        ir,
        profile,
        arena_upper_bound(ir, profile),
        inserted_records_upper_bound(ir, profile),
    )
    .with_estimated_arena();

    // **Structure first**, whatever else is wrong: an invalid cable is the actionable
    // diagnostic, and refusing a malformed graph on a limit instead would hide it. The
    // walk is proportional to a graph the caller has already built, and it is the cheap
    // half — lowering and liveness analysis are what the preflight below protects.
    let validated = match validate(ir, profile.capabilities().channel_layout()) {
        Ok(validated) => validated,
        Err(error) => {
            first_refusal(&preflight, &mut warnings, RefuseUpTo::Arena);
            return CompileOutcome {
                report: preflight,
                warnings,
                plan: Err(error),
            };
        }
    };

    // Then the limits that do not depend on the arena, so an oversized graph is refused
    // before anything is lowered. Only fields *before* the arena row can be decided
    // here: a later field must not be reported ahead of a scratch overrun the exact
    // figure would have found, because a plan over two limits is refused on the first.
    if let Some(error) = first_refusal(&preflight, &mut warnings, RefuseUpTo::Arena) {
        return CompileOutcome {
            report: preflight,
            warnings,
            plan: Err(error),
        };
    }

    warnings.clear();
    warnings.extend_from_slice(validated.warnings());

    let lowered = lower(ir, profile, &validated, &mut warnings, policy);
    let report = build_report(
        ir,
        profile,
        lowered.arena_samples() as u64,
        lowered.inserted as u64,
    );

    // The field scan runs **whatever else is wrong**, because it is also what collects
    // the advisory warnings, and a report whose warnings describe a different plan than
    // its rows is the one thing this contract cannot produce. Which refusal is *returned*
    // is the separate question: a node the stream cannot carry wins, because it is the
    // actionable diagnostic in the same way an invalid cable is — reporting a resource
    // field instead would send a reader to the profile for a plan whose corner frequency
    // is simply above its rate.
    let over_limit = first_refusal(&report, &mut warnings, RefuseUpTo::EveryField);
    let refusal = lowered.fault.or(over_limit);

    let plan = match refusal {
        Some(error) => Err(error),
        None => Ok(lowered.into_plan(profile)),
    };

    CompileOutcome {
        report,
        warnings,
        plan,
    }
}

/// How far through the field order a pass may refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefuseUpTo {
    /// Only the fields before the arena's, whose exact size needs an assignment.
    Arena,
    /// Every field.
    EveryField,
}

/// The first field the plan exceeds, in field order, collecting advisory warnings.
///
/// Field order, not check order: a plan over two limits is refused on the first rather
/// than on whichever check happened to run first. Warnings are collected over **every**
/// field regardless of how far refusal may reach, so a report and its warnings always
/// describe the same plan.
fn first_refusal(
    report: &ResourceReport,
    warnings: &mut Vec<CompileWarning>,
    scope: RefuseUpTo,
) -> Option<CompileError> {
    let mut refusal = None;
    let mut reached_arena = false;
    for field in ResourceField::ALL {
        if field == ResourceField::BufferScratchBytes {
            reached_arena = true;
        }
        let may_refuse = scope == RefuseUpTo::EveryField || !reached_arena;
        let Some(row) = report.row(field) else {
            continue;
        };
        match row.fit() {
            Fit::Within => {}
            Fit::UnitMismatch => {
                if may_refuse {
                    refusal = refusal.or(Some(CompileError::ReportUnitMismatch { field }));
                }
            }
            Fit::Exceeds => {
                if field.is_advisory() {
                    warnings.push(CompileWarning::AdvisoryBudgetExceeded {
                        field,
                        predicted: row.requested(),
                        permitted: row.available(),
                        contributor: row.contributor(),
                    });
                } else if field.is_admission_checked() && may_refuse {
                    refusal = refusal.or(Some(CompileError::LimitExceeded {
                        field,
                        requested: row.requested(),
                        available: row.available(),
                        responsible: row.contributor(),
                    }));
                }
            }
        }
    }
    refusal
}

/// The report, over an arena size and a record count the caller has established.
fn build_report(
    ir: &GraphIr,
    profile: &HostProfile,
    arena_samples: u64,
    inserted_records: u64,
) -> ResourceReport {
    let (script_work, script_contributor) = ir.script_instructions_per_quantum();
    ResourceReport::new(
        build_rows(ir, profile, arena_samples, inserted_records),
        LatencyAccounting::default().with(
            // ADR-0001 clause 7 requires this to be a *named* contributor: a latency
            // that is implicit is a latency nobody compensates, and its own risk
            // control is that it must appear in the report ADR-0022 consumes.
            LatencyContributor::RenderQuantumCarry,
            FrameCount::QUANTUM,
        ),
        ReportedQuantities::new(script_work, script_contributor),
        profile.capabilities().source(),
    )
}

/// How many **samples** the arena will hold at most, for a report built before lowering.
///
/// The only caller is the preflight report — a plan refused on structure or on an earlier
/// field never reaches an assignment, and still owes a scratch row. Once lowering has run,
/// the exact figure is the assignment's own extent and this bound is not consulted.
///
/// Samples rather than buffers since ADR-0041 clause 2: `Q` per producing node, plus
/// `c * Q` for the widening a mono signal reaching a wider output needs — one operation,
/// not one per channel, which is clause 8. Two earlier revisions of this bound were wrong
/// in the same direction: one counted only the producing nodes, so a stereo plan was
/// admitted against a budget it then allocated past, and one kept counting buffers after
/// the report started reading samples, which understated a refused plan's scratch row by
/// a factor of the quantum.
fn arena_upper_bound(ir: &GraphIr, profile: &HostProfile) -> u64 {
    let quantum = u64::from(QUANTUM_FRAMES);
    let channels = profile.capabilities().channel_layout().channels() as u64;
    let producers = ir.nodes().iter().filter(|n| n.kind().is_source()).count() as u64;
    // **Samples**, not buffers, since ADR-0041 clause 2: an authored node writes `Q` and
    // the widening writes `c * Q`, so a count of regions no longer describes an amount of
    // memory. Reporting one where the other is expected is what makes a refused plan's
    // scratch row wrong by a factor of the quantum.
    producers.saturating_mul(quantum).saturating_add(
        inserted_records_upper_bound(ir, profile)
            .saturating_mul(channels)
            .saturating_mul(quantum),
    )
}

/// How many operations the **compiler** adds beyond the authored nodes, at most.
///
/// **One**, where a mono signal is widened at the output — ADR-0041 clause 8 makes the
/// widening a single operation writing one `c * Q` region, where ADR-0002 clause 7 gave
/// each further channel its own. It is both a buffer and a prepared record, so it costs
/// what an operation costs. Only an output something reaches is widened: lowering skips
/// one with no incoming edge, so charging it would refuse a plan that fits. An unreached
/// output is admitted with a warning rather than refused, which is exactly the case that
/// would otherwise be measured against memory it never takes.
fn inserted_records_upper_bound(ir: &GraphIr, profile: &HostProfile) -> u64 {
    let widened = profile.capabilities().channel_layout().channels() > 1;
    let reached_output = ir
        .nodes()
        .iter()
        .filter(|node| matches!(node.kind(), IrNodeKind::Output))
        .any(|output| ir.edges().iter().any(|edge| edge.to().0 == output.id()));
    u64::from(reached_output && widened)
}

/// One row per field that carries an amount, in field order.
fn build_rows(
    ir: &GraphIr,
    profile: &HostProfile,
    arena_samples: u64,
    inserted_records: u64,
) -> Vec<ResourceRow> {
    let capabilities = profile.capabilities();
    let limits = profile.limits();
    let declarations = ir.declarations();

    let (prepared_bytes, prepared_contributor) = ir.prepared_bytes(inserted_records);
    let (mutable_bytes, mutable_contributor) = ir.mutable_bytes(inserted_records);
    let (peak_fan_out, fan_out_contributor) = ir.peak_fan_out();
    // The same count the prepared and mutable rows are over: a node with a kernel, plus
    // whatever the compiler inserted. The renderer allocates one state — and one control
    // range — per one of these, so a second formula here could disagree with preparation.
    let scratch_bytes = scratch_bytes(
        profile,
        arena_samples,
        ir.scheduled_records(inserted_records),
    );

    let node_count = NodeCount::measured(u32::try_from(ir.nodes().len()).unwrap_or(u32::MAX));
    let edge_count = EdgeCount::measured(u32::try_from(ir.edges().len()).unwrap_or(u32::MAX));
    let tap_count = TapCount::measured(u32::try_from(declarations.taps.len()).unwrap_or(u32::MAX));

    let mut rows = Vec::with_capacity(ResourceField::COUNT);

    // Capability rows report what the stream was prepared against. A capability is
    // not a budget a plan spends, so requested and available are the same queried
    // value; the row exists because `HOST-INV-006` covers every field, and because a
    // reader of a refusal needs to see what the plan was measured against.
    rows.push(ResourceRow::new(
        ResourceField::SampleRate,
        ResourceAmount::Rate(capabilities.sample_rate()),
        ResourceAmount::Rate(capabilities.sample_rate()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaximumBlockSize,
        ResourceAmount::Frames(capabilities.maximum_block_size()),
        ResourceAmount::Frames(capabilities.maximum_block_size()),
        IrObject::Plan,
    ));
    // Phase 1's only output operation writes whatever layout the stream has, so this
    // row reports and cannot fail. A plan that declares a layout of its own — and
    // the mismatch that becomes possible with it — arrives with ADR-0002.
    rows.push(ResourceRow::new(
        ResourceField::ChannelLayout,
        ResourceAmount::Layout(capabilities.channel_layout()),
        ResourceAmount::Layout(capabilities.channel_layout()),
        output_object(ir),
    ));
    // The informative form of the rate limit: the prepared rate against the range.
    // Construction already refused a rate outside it, so this row can only report —
    // which is `HOST-INV-007`'s narrowing, made visible in the report.
    rows.push(ResourceRow::new(
        ResourceField::AcceptedSampleRates,
        ResourceAmount::Rate(capabilities.sample_rate()),
        ResourceAmount::RateRange(limits.stream().accepted_sample_rates()),
        IrObject::Plan,
    ));

    rows.push(ResourceRow::new(
        ResourceField::MaxNodes,
        ResourceAmount::Nodes(node_count),
        ResourceAmount::Nodes(limits.graph().max_nodes()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxEdges,
        ResourceAmount::Edges(edge_count),
        ResourceAmount::Edges(limits.graph().max_edges()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxFanOutPerPort,
        ResourceAmount::FanOut(peak_fan_out),
        ResourceAmount::FanOut(limits.graph().max_fan_out_per_port()),
        fan_out_contributor,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxModGraphNodes,
        ResourceAmount::Nodes(declarations.mod_graph_nodes),
        ResourceAmount::Nodes(limits.graph().max_mod_graph_nodes()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxNoteGraphNodes,
        ResourceAmount::Nodes(declarations.note_graph_nodes),
        ResourceAmount::Nodes(limits.graph().max_note_graph_nodes()),
        IrObject::Plan,
    ));

    rows.push(ResourceRow::new(
        ResourceField::VoicesPerInstrument,
        ResourceAmount::Voices(declarations.voices_per_instrument),
        ResourceAmount::VoiceRange(
            limits.voices().minimum_voices_per_instrument(),
            limits.voices().maximum_voices_per_instrument(),
        ),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxActiveVoices,
        ResourceAmount::Voices(declarations.active_voices),
        ResourceAmount::Voices(limits.voices().max_active_voices()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxHeldNotes,
        ResourceAmount::HeldNotes(declarations.held_notes),
        ResourceAmount::HeldNotes(limits.voices().max_held_notes()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::RetirementCrossfade,
        ResourceAmount::Frames(limits.voices().retirement_crossfade()),
        ResourceAmount::Frames(limits.voices().retirement_crossfade()),
        IrObject::Plan,
    ));
    // A plan swap retires whatever is sounding, so the request is the plan's own
    // voice count. The budget is derived from `max_active_voices` precisely so this
    // row cannot exceed: a voice cannot be refused retirement.
    rows.push(ResourceRow::new(
        ResourceField::MaxConcurrentRetiringVoices,
        ResourceAmount::Voices(declarations.active_voices),
        ResourceAmount::Voices(limits.voices().max_concurrent_retiring_voices()),
        IrObject::Plan,
    ));

    // The cap itself is now reported rather than requested: no plan asks for it directly,
    // and it cannot be exceeded without a share being exceeded first, since the shares sum
    // to at most the cap.
    rows.push(ResourceRow::new(
        ResourceField::MaxEventsPerQuantum,
        ResourceAmount::Events(limits.events().max_events_per_quantum()),
        ResourceAmount::Events(limits.events().max_events_per_quantum()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxNoteExpansionPerTick,
        ResourceAmount::Events(declarations.note_expansion_per_tick),
        ResourceAmount::Events(limits.events().max_note_expansion_per_tick()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxScheduledEventsInFlight,
        ResourceAmount::Events(declarations.scheduled_events_in_flight),
        ResourceAmount::Events(limits.events().max_scheduled_events_in_flight()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::ForwardEventHorizon,
        ResourceAmount::Frames(limits.events().forward_event_horizon()),
        ResourceAmount::Frames(limits.events().forward_event_horizon()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::CommandQueueCapacity,
        ResourceAmount::Events(limits.events().queues().command_queue_capacity()),
        ResourceAmount::Events(limits.events().queues().command_queue_capacity()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::EventEgressCapacity,
        ResourceAmount::Events(limits.events().queues().event_egress_capacity()),
        ResourceAmount::Events(limits.events().queues().event_egress_capacity()),
        IrObject::Plan,
    ));

    rows.push(ResourceRow::new(
        ResourceField::MaxObservationTaps,
        ResourceAmount::Taps(tap_count),
        ResourceAmount::Taps(limits.observation().max_observation_taps()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::TelemetryRingFrames,
        ResourceAmount::Frames(limits.observation().telemetry_ring_frames()),
        ResourceAmount::Frames(limits.observation().telemetry_ring_frames()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::AnalyzerFftSize,
        ResourceAmount::Frames(limits.observation().analyzer_fft_size()),
        ResourceAmount::Frames(limits.observation().analyzer_fft_size()),
        IrObject::Plan,
    ));

    rows.push(ResourceRow::new(
        ResourceField::MaxMixChannels,
        ResourceAmount::MixChannels(declarations.mix_channels),
        ResourceAmount::MixChannels(limits.mixing().max_mix_channels()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxBuses,
        ResourceAmount::Buses(declarations.buses),
        ResourceAmount::Buses(limits.mixing().max_buses()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxSendsPerChannel,
        ResourceAmount::Sends(declarations.max_sends_on_any_channel),
        ResourceAmount::Sends(limits.mixing().max_sends_per_channel()),
        IrObject::Plan,
    ));

    rows.push(ResourceRow::new(
        ResourceField::PreparedImmutableBytes,
        ResourceAmount::Bytes(prepared_bytes),
        ResourceAmount::Bytes(limits.memory().prepared_immutable_bytes()),
        prepared_contributor,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MutableStateBytes,
        ResourceAmount::Bytes(mutable_bytes),
        ResourceAmount::Bytes(limits.memory().mutable_state_bytes()),
        mutable_contributor,
    ));
    rows.push(ResourceRow::new(
        ResourceField::BufferScratchBytes,
        ResourceAmount::Bytes(scratch_bytes),
        ResourceAmount::Bytes(limits.memory().buffer_scratch_bytes()),
        IrObject::Plan,
    ));

    push_script_rows(&mut rows, ir, profile);

    rows.push(ResourceRow::new(
        ResourceField::MaxHeldNotesPerTake,
        ResourceAmount::HeldNotes(declarations.held_notes_per_take),
        ResourceAmount::HeldNotes(limits.recording().max_held_notes_per_take()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxRecordedEventsPerTake,
        ResourceAmount::Events(declarations.recorded_events_per_take),
        ResourceAmount::Events(limits.recording().max_recorded_events_per_take()),
        IrObject::Plan,
    ));

    let predicted = ir
        .predicted_quantum_cost_ratio(capabilities.sample_rate())
        .unwrap_or(limits.cost().predicted_quantum_cost_ratio());
    rows.push(ResourceRow::new(
        ResourceField::PredictedQuantumCostRatio,
        ResourceAmount::Ratio(predicted),
        ResourceAmount::Ratio(limits.cost().predicted_quantum_cost_ratio()),
        IrObject::Plan,
    ));

    // The declaration is **compiled** work — `PlanDeclarations::events_per_quantum` is
    // statically knowable events, and data-dependent expansion is admitted separately — so
    // it is checked against the compiled producer's share rather than against the cap the
    // six shares partition. Checking it against the cap would admit a plan that exceeds its
    // entitlement and then faults at publication, which ADR-0046 clause 3 forbids: a
    // compiled runtime miss is a producer defect, so an admitted plan must not reach one.
    rows.push(ResourceRow::new(
        ResourceField::CompiledEventShare,
        ResourceAmount::Events(declarations.events_per_quantum),
        ResourceAmount::Events(limits.events().shares().compiled_event_share()),
        IrObject::Plan,
    ));

    // ADR-0046's producer partition. Each row reports the profile's own value on both
    // sides, as the other construction-checked capacities do: a plan does not request a
    // share, so there is no requested amount that could differ. What the rows carry is
    // the partition itself, so a report shows which class a later admission refusal was
    // charged against rather than only the cap it summed to.
    for (field, amount) in [
        (
            ResourceField::AuthoredRuntimeEventShare,
            limits.events().shares().authored_runtime_event_share(),
        ),
        (
            ResourceField::LiveEventShare,
            limits.events().shares().live_event_share(),
        ),
        (
            ResourceField::SessionEventShare,
            limits.events().shares().session_event_share(),
        ),
        (
            ResourceField::InternalEventShare,
            limits.events().shares().internal_event_share(),
        ),
        (
            ResourceField::ReleaseEventShare,
            limits.events().shares().release_event_share(),
        ),
        (
            ResourceField::ReleaseHoldCapacity,
            limits.events().shares().release_hold_capacity(),
        ),
        (
            ResourceField::PerformanceIngressCapacity,
            limits.events().queues().performance_ingress_capacity(),
        ),
    ] {
        rows.push(ResourceRow::new(
            field,
            ResourceAmount::Events(amount),
            ResourceAmount::Events(amount),
            IrObject::Plan,
        ));
    }

    rows
}

/// The per-program script rows, each attributed to the program that peaks.
fn push_script_rows(rows: &mut Vec<ResourceRow>, ir: &GraphIr, profile: &HostProfile) {
    let script = profile.limits().script();
    let programs = &ir.declarations().programs;

    /// The peak of one per-program quantity, and the program that reaches it.
    fn peak<T: Copy + Ord>(
        programs: &[crate::ir::IrProgram],
        of: fn(&crate::ir::IrProgram) -> T,
        none: T,
    ) -> (T, IrObject) {
        programs
            .iter()
            .fold((none, IrObject::Plan), |best, program| {
                let value = of(program);
                if value > best.0 {
                    (value, IrObject::Program(program.id()))
                } else {
                    best
                }
            })
    }

    let (instructions, instructions_at) = peak(
        programs,
        crate::ir::IrProgram::instructions,
        InstructionCount::NONE,
    );
    let (sources, sources_at) = peak(programs, crate::ir::IrProgram::sources, SlotCount::NONE);
    let (state, state_at) = peak(programs, crate::ir::IrProgram::state_slots, SlotCount::NONE);
    let (locals, locals_at) = peak(programs, crate::ir::IrProgram::locals, SlotCount::NONE);
    let (stack, stack_at) = peak(
        programs,
        crate::ir::IrProgram::eval_stack_depth,
        SlotCount::NONE,
    );
    let (arrays, arrays_at) = peak(programs, crate::ir::IrProgram::arrays, SlotCount::NONE);
    let (elements, elements_at) = peak(
        programs,
        crate::ir::IrProgram::array_elements,
        SlotCount::NONE,
    );
    let (emits, emits_at) = peak(programs, crate::ir::IrProgram::emits, SlotCount::NONE);

    rows.push(ResourceRow::new(
        ResourceField::MaxInstructionsPerProgram,
        ResourceAmount::Instructions(instructions),
        ResourceAmount::Instructions(script.max_instructions_per_program()),
        instructions_at,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxSourcesPerProgram,
        ResourceAmount::Slots(sources),
        ResourceAmount::Slots(script.max_sources_per_program()),
        sources_at,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxStateSlotsPerProgram,
        ResourceAmount::Slots(state),
        ResourceAmount::Slots(script.max_state_slots_per_program()),
        state_at,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxLocalsPerProgram,
        ResourceAmount::Slots(locals),
        ResourceAmount::Slots(script.max_locals_per_program()),
        locals_at,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxEvalStackDepth,
        ResourceAmount::Slots(stack),
        ResourceAmount::Slots(script.max_eval_stack_depth()),
        stack_at,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxArraysPerProgram,
        ResourceAmount::Slots(arrays),
        ResourceAmount::Slots(script.max_arrays_per_program()),
        arrays_at,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxArrayElements,
        ResourceAmount::Slots(elements),
        ResourceAmount::Slots(script.max_array_elements()),
        elements_at,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxEmitsPerProgram,
        ResourceAmount::Slots(emits),
        ResourceAmount::Slots(script.max_emits_per_program()),
        emits_at,
    ));
    // Both slot capacities are reported, and the floor between them is validated at
    // profile construction rather than here: `HOST-INV-017` wants the relation
    // declared once, not maintained at a use site.
    rows.push(ResourceRow::new(
        ResourceField::ModMatrixSlotsPerVoice,
        ResourceAmount::Slots(script.mod_matrix_slots_per_voice()),
        ResourceAmount::Slots(script.mod_matrix_slots_per_voice()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::ScriptHostSlotsPerVoice,
        ResourceAmount::Slots(script.script_host_slots_per_voice()),
        ResourceAmount::Slots(script.script_host_slots_per_voice()),
        IrObject::Plan,
    ));
}

/// The arena and the carries this plan needs, in bytes.
///
/// The carries are the one part that computes exactly: ADR-0001 clause 5 sizes both
/// at `maximum_block_size + Q` frames, and clause 6 primes the output one. The arena is
/// its **extent** in samples — ADR-0041 clause 13 — rather than a buffer count times the
/// quantum, because since that record its regions differ in width.
fn scratch_bytes(
    profile: &HostProfile,
    arena_samples: u64,
    scheduled_records: RecordCount,
) -> PreparedBytes {
    let channels = profile.capabilities().channel_layout().channels() as u64;
    let carry_frames = profile
        .capabilities()
        .maximum_block_size()
        .as_u64()
        .saturating_add(u64::from(QUANTUM_FRAMES));
    let sample = size_of::<f32>() as u64;

    let buffers = arena_samples;
    // Two carries, per ADR-0001 clause 5. Phase 1 has no node that consumes live
    // input, so the input carry is prepared and not read — the memory is reserved
    // because the contract sizes it, and the phase that adds an input-consuming node
    // is the one that starts reading it.
    let carries = carry_frames.saturating_mul(channels).saturating_mul(2);

    // Preparation also allocates the event scratch and the per-quantum tally, and the
    // budget has to cover what preparation actually takes. The figure comes from the
    // module that allocates it, so admission and preparation cannot disagree: an earlier
    // revision counted only the audio buffers, so a raised `max_events_per_quantum` was
    // reported as fitting and then allocated past the budget.
    let events = crate::render::event_scratch_bytes(
        profile.limits().events().max_events_per_quantum(),
        profile.capabilities().maximum_block_size(),
    );

    // And the sample-positioned control scratch, for the same reason and from the same
    // module: it is sized by the per-quantum event capacity and by how many nodes the plan
    // schedules, neither of which this function is the authority on.
    let controls = crate::render::timed_control_scratch_bytes(
        profile.limits().events().max_events_per_quantum(),
        scheduled_records,
    );

    PreparedBytes::measured(
        buffers
            .saturating_add(carries)
            .saturating_mul(sample)
            .saturating_add(events)
            .saturating_add(controls),
    )
}

/// The output node, if the plan has one, for attributing a layout row.
fn output_object(ir: &GraphIr) -> IrObject {
    ir.nodes()
        .iter()
        .find(|node| matches!(node.kind(), IrNodeKind::Output))
        .map_or(IrObject::Plan, |node| IrObject::Node(node.id()))
}

/// Turn a validated IR into the operations the renderer executes.
///
/// Lowering makes no structural decisions: [`crate::validate`] has already refused
/// every graph this could not express, and the execution order it produced is what
/// this walks. A check here would be a second authority on the same question.
///
/// It makes no *node* decisions either. What a kind declares — its ports, its controls,
/// its prepared data, its kernel, whether it may run in place — comes from
/// [`crate::node`], so this function is the same code for a plan of sines and a plan of
/// filters. That is ADR-0004 clause 2 one layer below the render loop.
struct Lowered {
    id: crate::plan::PlanId,
    ops: Vec<PlanOp>,
    /// Where each physical slot's samples live, indexed by `BufferSlot`.
    regions: Vec<crate::plan::BufferRegion>,
    /// Records the compiler added beyond the authored nodes, for the exact report.
    inserted: usize,
    /// The first node that could not be prepared for this stream, if any.
    ///
    /// Carried out rather than returned early, because a refusal owes a report and the
    /// report is exact only once the schedule and the arena exist. `HOST-INV-006` admits
    /// no plan-shaped exception to that.
    fault: Option<CompileError>,
    prepared_nodes: Vec<PreparedNode>,
    parameter_targets: Vec<ParameterTarget>,
    parameter_addresses: Vec<ParameterAddress>,
    note_targets: Vec<NoteTarget>,
    note_addresses: Vec<NoteAddress>,
}

impl Lowered {
    /// How many samples the arena holds, which is what the report's scratch row is over.
    fn arena_samples(&self) -> usize {
        self.regions
            .iter()
            .map(|region| region.end())
            .max()
            .unwrap_or(0)
    }

    /// Attach the capacities admission copied in.
    fn into_plan(self, profile: &HostProfile) -> CompiledPlan {
        let capabilities = profile.capabilities();
        CompiledPlan::new(
            self.id,
            self.ops,
            self.regions,
            self.prepared_nodes,
            self.parameter_targets,
            self.parameter_addresses,
            self.note_targets,
            self.note_addresses,
            capabilities.channel_layout(),
            capabilities.sample_rate(),
            capabilities.maximum_block_size(),
            profile.limits().events().max_events_per_quantum(),
            profile.limits().events().shares().compiled_event_share(),
            profile.limits().events().forward_event_horizon(),
            FrameCount::QUANTUM,
        )
    }
}

/// What lowering accumulates while it walks the schedule.
struct Lowering {
    ops: Vec<PlanOp>,
    prepared_nodes: Vec<PreparedNode>,
    /// One width per virtual buffer, in samples: `c * Q` for a signal of `c` channels.
    ///
    /// ADR-0041 clause 2. Lowering is where a signal's channel count is known — it comes
    /// from the port and its edge — and the arena is handed the widths rather than
    /// deriving them, because deriving them would make it a second authority on layout.
    widths: Vec<usize>,
    inserted: usize,
}

impl Lowering {
    /// Schedule one node: its prepared record, its output buffer, its step.
    ///
    /// The one place a step is built, so an authored node and a compiler-inserted
    /// operation are scheduled by the same code and the arena cannot tell them apart.
    fn schedule(
        &mut self,
        descriptor: &NodeDescriptor,
        prepared: PreparedNode,
        inputs: [Option<BufferSlot>; MAX_INPUTS],
        layout: ChannelLayout,
    ) -> (NodeSlot, BufferSlot) {
        let node = NodeSlot::new(self.prepared_nodes.len());
        self.prepared_nodes.push(prepared);
        let out = BufferSlot::new(self.widths.len());
        // The layout rather than a count: a raw number here would admit zero, or a count
        // no layout has, and the width of a region is the one place that would turn into
        // storage nobody can address.
        self.widths
            .push(layout.channels().saturating_mul(QUANTUM_FRAMES as usize));
        self.ops.push(PlanOp::Node(NodeStep::new(
            descriptor.kernel,
            node,
            out,
            layout,
            inputs,
            descriptor.in_place_safe,
        )));
        (node, out)
    }
}

fn lower(
    ir: &GraphIr,
    profile: &HostProfile,
    validated: &Validated,
    warnings: &mut Vec<CompileWarning>,
    policy: ArenaPolicy,
) -> Lowered {
    let plan_id = issue_plan_id();
    let rate = profile.capabilities().sample_rate();
    let mut state = Lowering {
        ops: Vec::new(),
        prepared_nodes: Vec::new(),
        widths: Vec::new(),
        inserted: 0,
    };
    let mut fault = None;
    let mut parameter_targets = Vec::new();
    let mut parameter_addresses = Vec::new();
    let mut note_targets = Vec::new();
    let mut note_addresses = Vec::new();

    // Indexed once, because the naive form is quadratic: a plan near `max_nodes` would
    // otherwise scan every edge for every node. Hashing off the audio thread is fine;
    // a compile that takes a billion steps for an admitted plan is not. Keyed by
    // **port**, not by node: a node with two inputs has two of them, and validation has
    // already refused a second edge into either.
    let mut source_of: HashMap<(NodeId, PortId), NodeId> = HashMap::with_capacity(ir.edges().len());
    for edge in ir.edges() {
        source_of.entry(edge.to()).or_insert(edge.from().0);
    }
    let mut slots: HashMap<NodeId, BufferSlot> = HashMap::with_capacity(ir.nodes().len());

    let kinds: HashMap<NodeId, IrNodeKind> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.kind()))
        .collect();

    for id in validated.order() {
        let Some(kind) = kinds.get(id).copied() else {
            continue;
        };
        let Some(descriptor) = node::descriptor(kind) else {
            // The output node: emitted last by construction, because the order puts a
            // node after everything that feeds it and the output is fed by everything.
            lower_output(
                ir, profile, validated, warnings, &mut state, &slots, &source_of, *id,
            );
            continue;
        };

        // Inputs in the order the node declares them, so port identity — not edge
        // order, and not declaration order — decides which slot a kernel reads first.
        let mut inputs = [None; MAX_INPUTS];
        let declared = descriptor
            .ports
            .iter()
            .filter(|port| port.direction() == crate::validate::PortDirection::Input);
        for (index, port) in declared.enumerate() {
            let source = source_of
                .get(&(*id, port.id()))
                .and_then(|from| slots.get(from).copied());
            if let Some(entry) = inputs.get_mut(index) {
                *entry = source;
            }
        }

        let prepared = match node::prepare(*id, kind, rate) {
            Ok(prepared) => prepared,
            Err(error) => {
                // The schedule is still built, with silence where the node would have
                // been, so the arena and the report describe a plan of the right shape.
                // Nothing renders it: the outcome carries the refusal.
                fault = fault.or(Some(error));
                PreparedNode::Silence
            }
        };
        // ADR-0041 clause 5: the channel count is a property of the port, so the width of
        // the region the node writes comes from the port table rather than from the
        // stream. Every authored kind declares a mono output today; asking the port is
        // what makes that a fact about the node rather than an assumption here.
        let out_layout = descriptor
            .ports
            .iter()
            .find(|port| port.direction() == crate::validate::PortDirection::Output)
            .map_or(ChannelLayout::Mono, |port| port.layout());
        let (node_slot, out) = state.schedule(&descriptor, prepared, inputs, out_layout);
        slots.insert(*id, out);

        for spec in &descriptor.controls {
            let slot = ParameterSlot::new(plan_id, parameter_targets.len());
            parameter_targets.push(ParameterTarget {
                node: node_slot,
                control: spec.control,
                rate: spec.rate,
            });
            parameter_addresses.push(ParameterAddress {
                node: *id,
                parameter: spec.parameter,
                slot,
            });
        }

        // A playable node gets one note slot. The control it names is the kind's, so a
        // caller plays the node and never learns which control being played moves — which
        // is what lets Phase 6's voice pool address a voice without knowing its graph.
        if let Some(control) = descriptor.note_control {
            let slot = NoteSlot::new(plan_id, note_targets.len());
            note_targets.push(NoteTarget {
                node: node_slot,
                control,
            });
            note_addresses.push(NoteAddress { node: *id, slot });
        }
    }

    // ADR-0005: lowering emits one buffer per value; the arena decides which of them
    // share storage, once, here. The render loop reads slot indices and learns nothing
    // about it.
    let assignment = arena::assign(&state.ops, &state.widths, policy);
    arena::rewrite(&mut state.ops, &assignment.mapping, &assignment.regions);

    Lowered {
        id: plan_id,
        ops: state.ops,
        regions: assignment.regions,
        inserted: state.inserted,
        fault,
        prepared_nodes: state.prepared_nodes,
        parameter_targets,
        parameter_addresses,
        note_targets,
        note_addresses,
    }
}

/// Schedule the plan's output: the widening it needs, then one write per channel.
#[allow(
    clippy::too_many_arguments,
    reason = "the output is lowered against the whole lowering context; bundling the arguments \
              would hide which of them it reads"
)]
fn lower_output(
    ir: &GraphIr,
    profile: &HostProfile,
    validated: &Validated,
    warnings: &mut Vec<CompileWarning>,
    state: &mut Lowering,
    slots: &HashMap<NodeId, BufferSlot>,
    source_of: &HashMap<(NodeId, PortId), NodeId>,
    id: NodeId,
) {
    let Some(source) = source_of
        .get(&(id, PortId::FIRST))
        .and_then(|from| slots.get(from).copied())
    else {
        return;
    };
    let layout = profile.capabilities().channel_layout();
    // The **validator's** record decides this, not the layout: validation is the one
    // authority on what an edge needs, and re-deriving it here would make lowering a
    // second one that could disagree.
    let widening = ir
        .edges()
        .iter()
        .find(|edge| edge.to().0 == id)
        .and_then(|edge| {
            validated
                .conversions()
                .iter()
                .find(|conversion| conversion.edge == edge.id())
        })
        .copied();

    // ADR-0041 clauses 2 and 8: a signal of `c` channels occupies **one** region of
    // `c * Q` samples, so a mono signal reaching a wider port is widened by one scheduled
    // operation that duplicates each sample into both channels of one wider region —
    // not, as ADR-0002 clause 2 had it, by one buffer and one operation per channel.
    let out = match widening {
        Some(widening) => {
            let copy = node::copy_descriptor();
            let (_, out) =
                state.schedule(&copy, node::prepare_copy(), [Some(source), None], layout);
            state.inserted += 1;
            // Clause 9's third requirement. The schedule and the buffer count carry the
            // conversion; without this a reader of the outcome would have to infer from
            // the operation list that the compiler widened their signal.
            warnings.push(CompileWarning::ConversionInserted {
                edge: widening.edge,
                conversion: widening.conversion,
            });
            out
        }
        // The signal already has the stream's layout, so the boundary is a copy and the
        // schedule holds no conversion at all.
        None => source,
    };
    state.ops.push(PlanOp::Output { source: out });
}
