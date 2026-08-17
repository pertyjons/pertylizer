//! Admission: compiling a plan against one profile.
//!
//! Admission happens **off the audio thread, once per prepared plan**, and it has
//! exactly two outcomes: a prepared plan, or a refusal with an attributable
//! diagnostic. It never truncates, clamps, or drops to make a plan fit — exceeding a
//! limit may not rewrite authored data — and it returns a resource report either
//! way, because a refusal is a report plus an error and never an error alone.

use crate::diagnostics::{CompileError, CompileWarning};
use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, IrObject, NodeId, PortId, parameters};
use crate::plan::{
    BufferSlot, CompiledPlan, ParameterRoute, ParameterTarget, PlanOp, SineTemplate, StateSlot,
};
use crate::profile::HostProfile;
use crate::quantities::{
    EdgeCount, InstructionCount, NodeCount, PreparedBytes, SlotCount, TapCount,
};
use crate::report::{
    Fit, LatencyAccounting, LatencyContributor, ReportedQuantities, ResourceAmount, ResourceField,
    ResourceReport, ResourceRow,
};
use crate::time::{FrameCount, QUANTUM_FRAMES};

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
/// The order is deliberate: the report is built first, because it is what a refusal
/// carries; then the admission-checked rows are examined in field order, so a plan
/// over two limits is refused on the first rather than on whichever check happened
/// to run first.
pub fn compile(ir: &GraphIr, config: &RenderConfig) -> CompileOutcome {
    let profile = config.host_profile();
    let rows = build_rows(ir, profile);
    let (script_work, script_contributor) = ir.script_instructions_per_quantum();
    let report = ResourceReport::new(
        rows,
        LatencyAccounting::default().with(
            // ADR-0001 clause 7 requires this to be a *named* contributor: a latency
            // that is implicit is a latency nobody compensates, and its own risk
            // control is that it must appear in the report ADR-0022 consumes.
            LatencyContributor::RenderQuantumCarry,
            FrameCount::QUANTUM,
        ),
        ReportedQuantities::new(script_work, script_contributor),
        profile.capabilities().source(),
    );

    let mut warnings = Vec::new();
    let mut refusal = None;

    for field in ResourceField::ALL {
        let Some(row) = report.row(field) else {
            continue;
        };
        match row.fit() {
            Fit::Within => {}
            Fit::UnitMismatch => {
                refusal = refusal.or(Some(CompileError::ReportUnitMismatch { field }));
            }
            Fit::Exceeds => {
                if field.is_advisory() {
                    warnings.push(CompileWarning::AdvisoryBudgetExceeded {
                        field,
                        predicted: row.requested(),
                        permitted: row.available(),
                        contributor: row.contributor(),
                    });
                } else if field.is_admission_checked() {
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

    let plan = match refusal {
        Some(error) => Err(error),
        None => lower(ir, profile),
    };

    CompileOutcome {
        report,
        warnings,
        plan,
    }
}

/// One row per field that carries an amount, in field order.
fn build_rows(ir: &GraphIr, profile: &HostProfile) -> Vec<ResourceRow> {
    let capabilities = profile.capabilities();
    let limits = profile.limits();
    let declarations = ir.declarations();

    let (prepared_bytes, prepared_contributor) = ir.prepared_bytes();
    let (mutable_bytes, mutable_contributor) = ir.mutable_bytes();
    let (peak_fan_out, fan_out_contributor) = ir.peak_fan_out();
    let scratch_bytes = scratch_bytes(ir, profile);

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

    rows.push(ResourceRow::new(
        ResourceField::MaxEventsPerQuantum,
        ResourceAmount::Events(declarations.events_per_quantum),
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
        ResourceAmount::Events(limits.events().command_queue_capacity()),
        ResourceAmount::Events(limits.events().command_queue_capacity()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::EventEgressCapacity,
        ResourceAmount::Events(limits.events().event_egress_capacity()),
        ResourceAmount::Events(limits.events().event_egress_capacity()),
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

/// The buffers and carries this plan needs, in bytes.
///
/// The carries are the one part that computes exactly: ADR-0001 clause 5 sizes both
/// at `maximum_block_size + Q` frames, and clause 6 primes the output one. Everything
/// else is one quantum-sized buffer per source, which is Phase 1's arena.
fn scratch_bytes(ir: &GraphIr, profile: &HostProfile) -> PreparedBytes {
    let channels = profile.capabilities().channel_layout().channels() as u64;
    let carry_frames = profile
        .capabilities()
        .maximum_block_size()
        .as_u64()
        .saturating_add(u64::from(QUANTUM_FRAMES));
    let sample = size_of::<f32>() as u64;

    let sources = ir.nodes().iter().filter(|n| n.kind().is_source()).count() as u64;
    let buffers = sources.saturating_mul(u64::from(QUANTUM_FRAMES));
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

    PreparedBytes::measured(
        buffers
            .saturating_add(carries)
            .saturating_mul(sample)
            .saturating_add(events),
    )
}

/// The output node, if the plan has one, for attributing a layout row.
fn output_object(ir: &GraphIr) -> IrObject {
    ir.nodes()
        .iter()
        .find(|node| matches!(node.kind(), IrNodeKind::Output))
        .map_or(IrObject::Plan, |node| IrObject::Node(node.id()))
}

/// Turn an admitted IR into the operations the renderer executes.
fn lower(ir: &GraphIr, profile: &HostProfile) -> Result<CompiledPlan, CompileError> {
    let mut ops = Vec::new();
    let mut sine_templates = Vec::new();
    let mut parameter_routes = Vec::new();
    // Source node -> its buffer, in declaration order. A linear scan over a handful
    // of entries at *compile* time; nothing here runs on the audio thread.
    let mut buffers: Vec<(NodeId, BufferSlot)> = Vec::new();

    for node in ir.nodes() {
        if !node.kind().is_source() {
            continue;
        }
        let out = BufferSlot::new(buffers.len());
        buffers.push((node.id(), out));
        match node.kind() {
            IrNodeKind::Silence => ops.push(PlanOp::Silence { out }),
            IrNodeKind::Constant { level } => ops.push(PlanOp::Constant { out, level }),
            IrNodeKind::Impulse { position } => ops.push(PlanOp::Impulse { out, position }),
            IrNodeKind::Sine {
                frequency,
                amplitude,
            } => {
                let state = StateSlot::new(sine_templates.len());
                sine_templates.push(SineTemplate {
                    frequency,
                    amplitude,
                });
                parameter_routes.push(ParameterRoute {
                    node: node.id(),
                    parameter: parameters::SINE_FREQUENCY,
                    target: ParameterTarget::SineFrequency(state),
                });
                parameter_routes.push(ParameterRoute {
                    node: node.id(),
                    parameter: parameters::SINE_AMPLITUDE,
                    target: ParameterTarget::SineAmplitude(state),
                });
                ops.push(PlanOp::Sine { out, state });
            }
            IrNodeKind::Output => {}
        }
    }

    let outputs = ir
        .nodes()
        .iter()
        .filter(|node| matches!(node.kind(), IrNodeKind::Output))
        .count();
    if outputs > 1 {
        // Rendering the first and ignoring the rest would be a silent choice about which
        // output a plan has. Refusing says so instead; what a second output *means* is a
        // question for the phase that has buses.
        return Err(CompileError::MultipleOutputs {
            outputs: u32::try_from(outputs).unwrap_or(u32::MAX),
        });
    }

    if let Some(output) = ir
        .nodes()
        .iter()
        .find(|node| matches!(node.kind(), IrNodeKind::Output))
    {
        // An edge into any other port of the output builds and compiles, and lowering
        // would then simply not see it — silence with no diagnostic. This phase has one
        // output port, so anything else is refused rather than dropped.
        if let Some(edge) = ir.edges().iter().find(|edge| {
            let (node, port) = edge.to();
            node == output.id() && port != PortId::FIRST
        }) {
            let (node, port) = edge.to();
            return Err(CompileError::UnsupportedOutputPort { node, port });
        }

        let incoming: Vec<_> = ir
            .edges()
            .iter()
            .filter(|edge| edge.to() == (output.id(), PortId::FIRST))
            .collect();
        match incoming.as_slice() {
            [] => {
                // An output with nothing patched into it renders silence, which is
                // what the empty plan does too. There is nothing to refuse here: a
                // plan under construction is allowed to be quiet.
            }
            [edge] => {
                let (source, _) = edge.from();
                let slot = buffers
                    .iter()
                    .find(|(id, _)| *id == source)
                    .map(|(_, slot)| *slot);
                if let Some(source) = slot {
                    ops.push(PlanOp::OutputMono { source });
                }
            }
            many => {
                // Summing several signals into one input is a fan-in policy, and
                // choosing one belongs to Phase 2's graph validation. Refusing is the
                // honest move: silently taking the first would be the kind of quiet
                // reduction this contract exists to remove.
                return Err(CompileError::UnsupportedFanIn {
                    node: output.id(),
                    port: PortId::FIRST,
                    edges: u32::try_from(many.len()).unwrap_or(u32::MAX),
                });
            }
        }
    }

    let capabilities = profile.capabilities();
    Ok(CompiledPlan::new(
        ops,
        buffers.len(),
        sine_templates,
        parameter_routes,
        capabilities.channel_layout(),
        capabilities.sample_rate(),
        capabilities.maximum_block_size(),
        profile.limits().events().max_events_per_quantum(),
        profile.limits().events().forward_event_horizon(),
        FrameCount::QUANTUM,
    ))
}

/// Scope is carried by the IR and not yet used to place work.
///
/// Phase 1 renders one global scope. Keeping the enum readable from here rather than
/// quietly dropping it is the difference between a phase that has not needed a
/// feature and a phase that lost it.
#[must_use]
pub fn declared_scopes(ir: &GraphIr) -> Vec<ExecutionScope> {
    let mut scopes = Vec::new();
    for node in ir.nodes() {
        if !scopes.contains(&node.scope()) {
            scopes.push(node.scope());
        }
    }
    scopes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IrNodeKind, NodeId, PortId, SignalDomain};
    use crate::quantities::{Amplitude, ChannelLayout, SampleRate};

    fn profile() -> HostProfile {
        HostProfile::harness(
            SampleRate::new(48_000.0).expect("valid rate"),
            FrameCount::new(1_024),
            ChannelLayout::Stereo,
        )
        .expect("valid harness profile")
    }

    #[test]
    fn every_field_has_a_row_whether_admission_succeeds_or_fails() {
        let config = RenderConfig::new(profile());
        let outcome = compile(&GraphIr::empty(), &config);
        assert!(outcome.plan().is_ok());
        for field in ResourceField::ALL {
            let row = outcome
                .report()
                .row(field)
                .unwrap_or_else(|| panic!("{field} has no row"));
            assert_eq!(row.field(), field);
        }
        assert_eq!(outcome.report().rows().len(), ResourceField::COUNT);
    }

    #[test]
    fn no_row_compares_mismatched_units() {
        let config = RenderConfig::new(profile());
        let outcome = compile(&GraphIr::empty(), &config);
        for row in outcome.report().rows() {
            assert_ne!(
                row.fit(),
                Fit::UnitMismatch,
                "the row for {} compares two different units",
                row.field()
            );
        }
    }

    #[test]
    fn the_report_names_the_carry_latency() {
        let config = RenderConfig::new(profile());
        let outcome = compile(&GraphIr::empty(), &config);
        assert_eq!(
            outcome
                .report()
                .latency()
                .frames_of(LatencyContributor::RenderQuantumCarry),
            Some(FrameCount::QUANTUM)
        );
    }

    #[test]
    fn fan_in_to_one_input_is_refused_rather_than_silently_reduced() {
        let a = NodeId::new(1);
        let b = NodeId::new(2);
        let out = NodeId::new(3);
        let ir = GraphIr::builder()
            .node(a, IrNodeKind::Silence, ExecutionScope::Global)
            .node(
                b,
                IrNodeKind::Constant {
                    level: Amplitude::new(0.5).expect("finite"),
                },
                ExecutionScope::Global,
            )
            .node(out, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (a, PortId::FIRST),
                (out, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (b, PortId::FIRST),
                (out, PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()
            .expect("readable plan");

        let outcome = compile(&ir, &RenderConfig::new(profile()));
        assert!(matches!(
            outcome.plan(),
            Err(CompileError::UnsupportedFanIn { edges: 2, .. })
        ));
        // The report is still there: a refusal is a report plus an error.
        assert_eq!(outcome.report().rows().len(), ResourceField::COUNT);
    }
}
