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
use crate::ir::{GraphIr, IrNodeKind, IrObject, NodeId, PlanDeclarations, PortId};
use crate::node::kernels::{MAX_INPUTS, PreparedNode};
use crate::node::{self, NodeDescriptor, NoteMagnitude};
use crate::plan::{
    BufferSlot, CompiledPlan, NodeRole, NodeSlot, NodeStep, NoteAddress, NoteMagnitudeTarget,
    NoteSlot, NoteTarget, ParameterAddress, ParameterSlot, ParameterTarget, PlanOp,
    VoiceInstanceIndex, issue_plan_id,
};
use crate::profile::HostProfile;
use crate::quantities::{
    ChannelLayout, EdgeCount, EventCount, HeldNoteCount, InstructionCount, NodeCount,
    PreparedBytes, RecordCount, SlotCount, TapCount,
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
    for program in ir.scripts() {
        if let Some(scope) = ir.scope_of(program.node())
            && let Some(estimate) = program.audio_cost(scope, profile)
            && estimate.per_quantum > estimate.warning_threshold
        {
            warnings.push(CompileWarning::AudioScriptWork { estimate });
        }
    }

    // The report a refused plan carries, over the arena size that can be known before an
    // assignment exists. Advisory findings are collected from it on both refusal paths,
    // so a report showing an overrun is never returned with no warning to match.
    let mut preflight = build_report(
        ir,
        profile,
        arena_upper_bound(ir, profile),
        inserted_records_upper_bound(ir, profile),
        ir.tuning_bytes()
            .saturating_add(ir.sample_bytes())
            .saturating_add(ir.script_bytes()),
        None,
        0,
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

    let paths = match crate::latency::analyze(
        ir,
        validated.order(),
        profile.capabilities().sample_rate(),
        ir.declarations().compensation,
    ) {
        Ok(paths) => paths,
        Err(error) => {
            first_refusal(&preflight, &mut warnings, RefuseUpTo::Arena);
            return CompileOutcome {
                report: preflight,
                warnings,
                plan: Err(error),
            };
        }
    };
    // Rebuild the preflight with the analysis's inserted records and history. These
    // rows precede the arena, so omitting compensation can choose the wrong first
    // refusal (mutable state instead of an earlier immutable-data overrun).
    let inserted = u64::from(paths.compensation_records().get());
    preflight = build_report(
        ir,
        profile,
        arena_upper_bound(ir, profile).saturating_add(
            inserted
                .saturating_mul(profile.capabilities().channel_layout().channels() as u64)
                .saturating_mul(u64::from(QUANTUM_FRAMES)),
        ),
        inserted_records_upper_bound(ir, profile).saturating_add(inserted),
        ir.tuning_bytes()
            .saturating_add(ir.sample_bytes())
            .saturating_add(ir.script_bytes()),
        Some(&paths),
        paths.compensation_history().get(),
    )
    .with_estimated_arena();

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

    // Resource rows are recomputed after lowering; the static program workload is unchanged.
    warnings.retain(|warning| matches!(warning, CompileWarning::AudioScriptWork { .. }));
    warnings.extend_from_slice(validated.warnings());

    let lowered = lower(ir, profile, &validated, &mut warnings, policy, &paths);
    let report = build_report(
        ir,
        profile,
        lowered.arena_samples() as u64,
        lowered.inserted as u64,
        ir.tuning_bytes()
            .saturating_add(ir.sample_bytes())
            .saturating_add(ir.script_bytes()),
        Some(&paths),
        lowered.compensation_history,
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
        None => {
            let mut plan = lowered.into_plan(profile, ir.declarations(), paths);
            match ir
                .scripts()
                .iter()
                .map(|program| program.prepare(ir, &plan))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(scripts) => {
                    plan.install_scripts(scripts);
                    Ok(plan)
                }
                Err(error) => Err(error),
            }
        }
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
            Fit::NotConfigured if field.is_capture_configuration() => {}
            Fit::NotConfigured | Fit::UnitMismatch => {
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
    tuning_bytes: u64,
    paths: Option<&crate::latency::PathLatencies>,
    compensation_history: u64,
) -> ResourceReport {
    let mut latency = LatencyAccounting::default()
        .with(LatencyContributor::RenderQuantumCarry, FrameCount::QUANTUM);
    if let Some(paths) = paths {
        latency = latency.with_paths(paths.clone());
    }
    let (script_work, script_contributor) = ir.script_instructions_per_quantum();
    ResourceReport::new(
        build_rows(
            ir,
            profile,
            arena_samples,
            inserted_records,
            tuning_bytes
                .saturating_add(paths.map_or(0, crate::latency::PathLatencies::prepared_bytes)),
            compensation_history,
            paths,
        ),
        latency,
        ReportedQuantities::new(
            script_work,
            script_contributor,
            ir.declared_tail(profile.capabilities().sample_rate()),
        ),
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
    // One region per **scheduled** authored record: a voice-scope node is scheduled once per
    // instance (`P06-S001`) and every instance writes its own `Q`, so the bound counts the
    // instances, not the nodes — an independent review found it counting each authored
    // node once and understating a four-voice plan's exact arena.
    let producers = u64::from(ir.state_records(0).get());
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
    // Only a **mono** signal reaching a wider output is widened; a stereo source — a mix
    // channel's or a sum's since `P08-S001` — is copied out as it is, and charging it a
    // widening refused a plan whose own report fit its budget.
    let reached_output = ir
        .nodes()
        .iter()
        .filter(|node| matches!(node.kind(), IrNodeKind::Output))
        .any(|output| {
            ir.edges().iter().any(|edge| {
                edge.to().0 == output.id()
                    && ir
                        .node(edge.from().0)
                        .and_then(|source| ir.descriptor(source.kind()))
                        .and_then(|descriptor| {
                            descriptor
                                .ports
                                .iter()
                                .find(|port| {
                                    port.id() == edge.from().1
                                        && port.direction()
                                            == crate::validate::PortDirection::Output
                                })
                                .map(|port| port.layout())
                        })
                        .unwrap_or(ChannelLayout::Mono)
                        == ChannelLayout::Mono
            })
        });
    let widening = u64::from(reached_output && widened);
    // `P06-S001`'s voice sum: every voice-scope node whose output feeds a node outside the
    // scope is summed by one copy and `voices − 1` accumulates, when there is more than one
    // voice. An upper bound, like the widening: a node feeding the scope's outside twice is
    // summed once, and the exact figure is the lowering's.
    let voices = u64::from(ir.voice_instances().get());
    // A stealing plan sums even one voice, because the sum step is where the taken voice's
    // fade is applied (ADR-0058).
    let summed = if voices > 1 || ir.declarations().stealing.steals() {
        (voice_sum_sources(ir).len() as u64).saturating_mul(voices)
    } else {
        0
    };
    // `SOUND-INV-031` and `SOUND-INV-014`, **exact** rather than bounded, because this count
    // feeds the memory rows preflight can refuse on: an over-estimate would refuse a plan
    // whose admitted report fits its own budget, which an independent read reproduced. Per
    // declared input port: one widening per cable whose layout is narrower than the port's,
    // and, where the port sums, one accumulate per cable after the first — exactly what
    // lowering schedules. Per instance of the consuming node.
    let scopes: HashMap<NodeId, crate::ir::ExecutionScope> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.scope()))
        .collect();
    let kinds: HashMap<NodeId, IrNodeKind> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.kind()))
        .collect();
    // Per target port: cables, cables to widen, and whether the port sums.
    let mut ports: HashMap<(NodeId, PortId), (u64, u64, bool)> = HashMap::new();
    for edge in ir.edges() {
        let (from, from_port) = edge.from();
        let (to, to_port) = edge.to();
        let Some(target) = kinds
            .get(&to)
            .and_then(|kind| ir.descriptor(*kind))
            .and_then(|descriptor| {
                descriptor
                    .ports
                    .iter()
                    .find(|port| {
                        port.id() == to_port
                            && port.direction() == crate::validate::PortDirection::Input
                    })
                    .copied()
            })
        else {
            continue;
        };
        let source_layout = kinds
            .get(&from)
            .and_then(|kind| ir.descriptor(*kind))
            .and_then(|descriptor| {
                descriptor
                    .ports
                    .iter()
                    .find(|port| {
                        port.id() == from_port
                            && port.direction() == crate::validate::PortDirection::Output
                    })
                    .map(|port| port.layout())
            })
            .unwrap_or(ChannelLayout::Mono);
        let entry = ports.entry((to, to_port)).or_insert((0, 0, false));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry
            .1
            .saturating_add(u64::from(source_layout != target.layout()));
        entry.2 = target.fan_in() == crate::validate::FanIn::Summed;
    }
    let mut into_inputs = 0_u64;
    for ((to, _), (cables, widened, sums)) in ports {
        let accumulates = if sums { cables.saturating_sub(1) } else { 0 };
        let instances = if scopes.get(&to) == Some(&crate::ir::ExecutionScope::Voice) {
            voices
        } else {
            1
        };
        into_inputs = into_inputs.saturating_add(
            widened
                .saturating_add(accumulates)
                .saturating_mul(instances),
        );
    }
    widening.saturating_add(summed).saturating_add(into_inputs)
}

/// The mix channels a plan compiles: its `Channel` nodes in a channel scope
/// (`SOUND-INV-031`); a strip in a bus scope is a bus's (`SOUND-INV-034`).
pub(crate) fn compiled_channels(ir: &GraphIr) -> crate::quantities::MixChannelCount {
    let count = ir
        .nodes()
        .iter()
        .filter(|node| {
            matches!(node.kind(), IrNodeKind::Channel { .. })
                && matches!(node.scope(), crate::ir::ExecutionScope::Channel(_))
        })
        .count();
    crate::quantities::MixChannelCount::measured(u32::try_from(count).unwrap_or(u32::MAX))
}

/// The buses a plan compiles: its `Channel` nodes in a bus scope (`SOUND-INV-034`).
pub(crate) fn compiled_buses(ir: &GraphIr) -> crate::quantities::BusCount {
    let count = ir
        .nodes()
        .iter()
        .filter(|node| {
            matches!(node.kind(), IrNodeKind::Channel { .. })
                && matches!(node.scope(), crate::ir::ExecutionScope::Bus(_))
        })
        .count();
    crate::quantities::BusCount::measured(u32::try_from(count).unwrap_or(u32::MAX))
}

/// The most sends any one channel or bus of the plan has (`SOUND-INV-034`): the send nodes
/// of each tagged scope, pre-fader and post-fader together, as V1's per-channel list holds
/// both (`LIMIT-0024`).
pub(crate) fn sends_on_the_busiest_channel(ir: &GraphIr) -> crate::quantities::SendCount {
    let mut per_scope: HashMap<crate::ir::ExecutionScope, u32> = HashMap::new();
    for node in ir.nodes() {
        if matches!(
            node.kind(),
            IrNodeKind::Send { .. } | IrNodeKind::PostFaderSend { .. }
        ) {
            let count = per_scope.entry(node.scope()).or_insert(0);
            *count = count.saturating_add(1);
        }
    }
    crate::quantities::SendCount::measured(per_scope.values().copied().max().unwrap_or(0))
}

/// The voice-scope nodes whose output is read by a node outside the voice scope, each once,
/// in ascending identity.
pub(crate) fn voice_sum_sources(ir: &GraphIr) -> Vec<NodeId> {
    let scopes: HashMap<NodeId, crate::ir::ExecutionScope> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.scope()))
        .collect();
    let mut sources: Vec<NodeId> = ir
        .edges()
        .iter()
        .filter(|edge| {
            scopes.get(&edge.from().0) == Some(&crate::ir::ExecutionScope::Voice)
                && scopes
                    .get(&edge.to().0)
                    .is_some_and(|scope| *scope != crate::ir::ExecutionScope::Voice)
        })
        .map(|edge| edge.from().0)
        .collect();
    sources.sort_unstable();
    sources.dedup();
    sources
}

/// One row per field that carries an amount, in field order.
/// The amount a declared aggregate requests, named exactly.
///
/// `HOST-INV-006` requires a row to carry the amount a plan **requested**, and a declared
/// aggregate can sum past what an `EventCount` names. Saturating made the report understate
/// the request and, against a profile limit of `u32::MAX`, read `Within` for a total it
/// exceeded. [`ResourceAmount::EventsBeyondCount`] carries the real figure, so the ordinary
/// row comparison refuses the plan and names the field — no separate error, and no arithmetic
/// the reader has to trust.
fn events_requested(total: u64) -> ResourceAmount {
    u32::try_from(total).map_or_else(
        |_| {
            // The `None` arm is unreachable: `try_from` failed, so the value exceeds
            // `u32::MAX`, which is exactly what the constructor validates. Written as a
            // fallback rather than an `expect` because a report row must not panic the
            // compiler, and the saturating value is the closest honest thing left to say.
            crate::report::EventsBeyondCount::new(total).map_or(
                ResourceAmount::Events(EventCount::measured(u32::MAX)),
                ResourceAmount::EventsBeyondCount,
            )
        },
        |fits| ResourceAmount::Events(EventCount::measured(fits)),
    )
}

fn build_rows(
    ir: &GraphIr,
    profile: &HostProfile,
    arena_samples: u64,
    inserted_records: u64,
    tuning_bytes: u64,
    compensation_history: u64,
    paths: Option<&crate::latency::PathLatencies>,
) -> Vec<ResourceRow> {
    let capabilities = profile.capabilities();
    let limits = profile.limits();
    let declarations = ir.declarations();

    let (node_prepared_bytes, node_contributor) = ir.prepared_bytes(inserted_records);
    // `SOUND-INV-021`'s prepared tunings are immutable plan data like a node's prepared
    // record, so they are charged to the same row. The contributor stays the node that set
    // the record width unless the tables outweigh every node's records, in which case no
    // single authored object is responsible and the plan is — which is what `IrObject::Plan`
    // means in this report.
    let prepared_bytes =
        PreparedBytes::measured(node_prepared_bytes.get().saturating_add(tuning_bytes));
    let prepared_contributor = if tuning_bytes > node_prepared_bytes.get() {
        IrObject::Plan
    } else {
        node_contributor
    };
    let (mutable_bytes, mutable_contributor) =
        ir.mutable_bytes(inserted_records, capabilities.sample_rate());
    let mutable_bytes =
        PreparedBytes::measured(mutable_bytes.get().saturating_add(compensation_history));
    let (peak_fan_out, fan_out_contributor) = ir.peak_fan_out();
    // The same count the prepared and mutable rows are over: a node with a kernel, plus
    // whatever the compiler inserted. The renderer allocates one state — and one control
    // range — per one of these, so a second formula here could disagree with preparation.
    let declared_note_ranges: Vec<_> = declarations
        .note_producers
        .iter()
        .map(|producer| producer.simultaneous_notes)
        .collect();
    let scratch_bytes = scratch_bytes(
        profile,
        arena_samples,
        ir.state_records(inserted_records),
        &declared_note_ranges,
        ScratchWriteWidths::new(
            crate::render::release_group_writes(ir.max_writes_per_note())
                .fanned_out(ir.sample_positioned_fan_out())
                .widest(crate::quantities::WritesPerNote::at_least(
                    ir.steal_expansion().get().saturating_add(
                        if ir.declarations().stealing.steals() {
                            paths.map_or(0, crate::latency::PathLatencies::voice_groups)
                        } else {
                            0
                        },
                    ),
                )),
            ir.max_writes_per_note(),
        ),
        ir.modulated_sample_positioned_rows(),
        ir.voice_instances(),
    );

    let node_count = NodeCount::measured(u32::try_from(ir.nodes().len()).unwrap_or(u32::MAX));
    // A modulation edge is an edge of the plan (`SOUND-INV-027`): it is walked by the
    // schedule and read every quantum, so it is admitted against the same count.
    let edge_count = EdgeCount::measured(
        u32::try_from(ir.edges().len().saturating_add(ir.modulations().len())).unwrap_or(u32::MAX),
    );
    // `SOUND-INV-022`: the taps a plan carries are its nodes' declarations', and nothing
    // else's — the same walk the lowering makes, so the admitted count is the table's.
    // One row per instance of a voice-scope node (`P06-S001`): a monitor in the voice scope
    // is one signal point per voice, and each is admitted.
    let tap_count = TapCount::measured(
        u32::try_from(ir.nodes().iter().fold(0_usize, |total, node| {
            let per_instance =
                crate::node::declaration(node.kind()).map_or(0, |declared| declared.taps.len());
            let instances = if node.scope() == crate::ir::ExecutionScope::Voice {
                usize::try_from(ir.voice_instances().get()).unwrap_or(usize::MAX)
            } else {
                1
            };
            total.saturating_add(per_instance.saturating_mul(instances))
        }))
        .unwrap_or(u32::MAX),
    );

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
    // `P06-S001`: the voices a plan renders are derived from its producers' identity ranges
    // — one instance per index — and admitted here as that count, not as a declaration that
    // could disagree with what the renderer instantiates. A plan with no voice-scope node
    // instantiates nothing per voice and requests none.
    let voices = if ir.has_voice_scope() {
        ir.voice_instances()
    } else {
        crate::quantities::VoiceCount::NONE
    };
    rows.push(ResourceRow::new(
        ResourceField::MaxActiveVoices,
        ResourceAmount::Voices(voices),
        ResourceAmount::Voices(limits.voices().max_active_voices()),
        IrObject::Plan,
    ));
    // ADR-0047 clause 3's identity partition covers a **superset** of the hold partition —
    // every note-on needs an occurrence while only some need a hold — so it sums every
    // producer, compiled included. The plan's own `held_notes` is the larger of the two
    // claims it can make about itself, and a producer set that outran it would be a plan
    // disagreeing with itself before any profile was consulted.
    let declared_notes = declarations
        .note_producers
        .iter()
        .fold(0_u64, |total, producer| {
            total.saturating_add(u64::from(producer.simultaneous_notes.get()))
        });
    let held = HeldNoteCount::measured(
        u32::try_from(declared_notes.max(u64::from(declarations.held_notes.get())))
            .unwrap_or(u32::MAX),
    );
    rows.push(ResourceRow::new(
        ResourceField::MaxHeldNotes,
        ResourceAmount::HeldNotes(held),
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
        ResourceAmount::Voices(voices),
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
    // ADR-0046 clause 1's third authored relation: "the plan-wide aggregate maximum of
    // simultaneously retained authored future events fits the headroom above that floor",
    // the floor being `compiled_event_share * max_quanta_per_callback`. The row asks for the
    // **larger** of the plan's own in-flight claim and that floor plus the authored
    // retention, which is the shape [`ResourceField::MaxHeldNotes`] above already uses: two
    // claims a plan makes about one store, and the store must satisfy both. Composing them
    // by addition instead would double-count a plan that already folded its authored sources
    // into its own figure, and refuse a plan that is in fact admissible.
    //
    // The floor is recomputed here rather than read back, because profile construction
    // checks it as a relation and keeps no field for it. A capability that cannot name its
    // own callback extent has already failed construction, so the fallback below is
    // unreachable through a constructed profile; it is a saturating floor rather than an
    // `expect`, because a value this row cannot compute must not panic the compiler.
    let compiled_floor = capabilities
        .max_quanta_per_callback()
        .ok()
        .and_then(|quanta| {
            limits
                .events()
                .shares()
                .compiled_event_share()
                .checked_over(quanta)
        })
        .map_or(0_u64, |floor| u64::from(floor.get()));
    let retained = declarations
        .authored_sources
        .iter()
        .fold(compiled_floor, |total, source| {
            total.saturating_add(u64::from(source.retained_future.get()))
        });
    let in_flight = retained.max(u64::from(declarations.scheduled_events_in_flight.get()));
    rows.push(ResourceRow::new(
        ResourceField::MaxScheduledEventsInFlight,
        events_requested(in_flight),
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

    // `SOUND-INV-031`: the channels a plan renders are its `Channel` nodes, counted from the
    // IR rather than taken from a declaration that could disagree with what is compiled.
    rows.push(ResourceRow::new(
        ResourceField::MaxMixChannels,
        ResourceAmount::MixChannels(compiled_channels(ir)),
        ResourceAmount::MixChannels(limits.mixing().max_mix_channels()),
        IrObject::Plan,
    ));
    // `SOUND-INV-034`: the buses are the plan's bus strips and a channel's sends are the
    // send nodes of its tagged scope, both counted from the IR as the channels are.
    rows.push(ResourceRow::new(
        ResourceField::MaxBuses,
        ResourceAmount::Buses(compiled_buses(ir)),
        ResourceAmount::Buses(limits.mixing().max_buses()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::MaxSendsPerChannel,
        ResourceAmount::Sends(sends_on_the_busiest_channel(ir)),
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
    // ADR-0046 clause 5's plan-wide authored aggregate, **summed**. The record allows two
    // sources that fit individually to be rejected together "unless the compiler proves them
    // mutually exclusive"; no such proof exists and none is built here, so every declared
    // source counts. Summing is the conservative direction: it can refuse a plan whose
    // sources never actually coincide, never admit one whose sources do.
    let authored = declarations
        .authored_sources
        .iter()
        .fold(0_u64, |total, source| {
            total.saturating_add(u64::from(source.destination_occupancy.get()))
        });
    rows.push(ResourceRow::new(
        ResourceField::AuthoredRuntimeEventShare,
        events_requested(authored),
        ResourceAmount::Events(limits.events().shares().authored_runtime_event_share()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::LiveEventShare,
        ResourceAmount::Events(limits.events().shares().live_event_share()),
        ResourceAmount::Events(limits.events().shares().live_event_share()),
        IrObject::Plan,
    ));

    // ADR-0051 clause 1's catch-up batch, which `HOST-INV-022` makes the session share's
    // bounded contributor. A locate restores **every** prepared target at once, so the
    // batch's size *is* the plan's addressable-parameter count at every legal locate position
    // — one row per control per node, which is exactly how `parameter_addresses` is lowered
    // below; the per-instance rows behind one address are the renderer's fan-out
    // (`P06-S001`), not rows of the batch. That is why the row has one number to report
    // rather than a worst case to search
    // for, which is what the plan-dependent admission of this share compares. See the note
    // on that row below for the boundary release it is charged alongside.
    // Over the controls that **admit a write**: `SOUND-INV-023`'s not-modulatable control
    // compiles to no target, so the batch has no row for it and the charge must not either.
    let catch_up = ir.nodes().iter().fold(0_u64, |total, node| {
        total.saturating_add(ir.descriptor(node.kind()).map_or(0, |descriptor| {
            descriptor
                .controls
                .iter()
                .filter(|control| control.law.admits_writes())
                .count()
        }) as u64)
    });
    // Plus **one** for ADR-0050 clause 5's boundary mass release, which ADR-0046 clause 6
    // charges to the same share as a single bounded operation rather than as one event per
    // voice. It is published into the same quantum as the batch, so a plan whose catch-up
    // exactly filled the share would overrun at the first seek — and a share overrun is a
    // contract violation that ends the stream, not a load condition it recovers from.
    //
    // Like the compiled row above, this one reports a plan-derived request rather than the
    // profile's value on both sides, **and it is refused**. ADR-0046 clause 1 requires plan
    // admission to check "the maximum destination contribution of one complete eligible
    // session/transport snapshot, including the largest catch-up batch over every legal
    // locate position in that plan", and the paragraph above is why that maximum is one
    // number rather than a search: a locate restores every addressable parameter at once, so
    // the batch's size *is* the address count at every legal position.
    //
    // It did not refuse until this slice. `SessionEventShare` was excluded from
    // `ResourceField::is_admission_checked`, so a plan whose catch-up exceeded the share
    // compiled and faulted at its **first locate** — a share overrun is a contract violation
    // that ends the stream, which is a bad way to learn that a plan was never admissible.
    let session = catch_up.saturating_add(1);
    rows.push(ResourceRow::new(
        ResourceField::SessionEventShare,
        events_requested(session),
        ResourceAmount::Events(limits.events().shares().session_event_share()),
        IrObject::Plan,
    ));

    // ADR-0046 clause 1: "the internal share covers the sum of every **admitted** internal
    // producer's declared per-quantum maximum, which is a complete bound only because clause
    // 2 confines an internal emission to the quantum that generates it". The confinement is
    // why one number per producer suffices; without it a per-quantum rate would say nothing
    // about occupancy at a destination admitted earlier.
    let internal = declarations
        .internal_producers
        .iter()
        .fold(0_u64, |total, producer| {
            total.saturating_add(u64::from(producer.per_quantum.get()))
        });
    rows.push(ResourceRow::new(
        ResourceField::InternalEventShare,
        events_requested(internal),
        ResourceAmount::Events(limits.events().shares().internal_event_share()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::ReleaseEventShare,
        ResourceAmount::Events(limits.events().shares().release_event_share()),
        ResourceAmount::Events(limits.events().shares().release_event_share()),
        IrObject::Plan,
    ));

    // ADR-0046 clause 6's hold partition, summed across the plan's **non-compiled** note-on
    // producers, and emitted here because `ResourceField::ALL` puts it after the release
    // share. Checking one source at a time is not admission: two that each fit can together
    // exceed the capacity, which is the rule the record states for authored envelopes and
    // which holds here for the same reason. A compiled producer contributes nothing — its
    // releases use plan entitlements — and one that declared a hold was already refused.
    let declared_holds = declarations
        .note_producers
        .iter()
        .filter(|producer| !producer.compiled)
        .fold(0_u64, |total, producer| {
            total.saturating_add(u64::from(producer.simultaneous_holds.get()))
        });
    rows.push(ResourceRow::new(
        ResourceField::ReleaseHoldCapacity,
        ResourceAmount::Events(EventCount::measured(
            u32::try_from(declared_holds).unwrap_or(u32::MAX),
        )),
        ResourceAmount::Events(limits.events().shares().release_hold_capacity()),
        IrObject::Plan,
    ));
    rows.push(ResourceRow::new(
        ResourceField::PerformanceIngressCapacity,
        ResourceAmount::Events(limits.events().queues().performance_ingress_capacity()),
        ResourceAmount::Events(limits.events().queues().performance_ingress_capacity()),
        IrObject::Plan,
    ));

    // Configuration only: the graph does not reserve capture storage.
    let capture = limits.recording().capture();
    for (field, amount) in [
        (
            ResourceField::MaxTrackedInputNotes,
            capture
                .map(|value| ResourceAmount::TrackedInputNotes(value.max_tracked_input_notes()))
                .unwrap_or(ResourceAmount::NotConfigured),
        ),
        (
            ResourceField::MaxCaptureSources,
            capture
                .map(|value| ResourceAmount::CaptureSources(value.max_capture_sources()))
                .unwrap_or(ResourceAmount::NotConfigured),
        ),
        (
            ResourceField::MaxCapturePasses,
            capture
                .map(|value| ResourceAmount::CapturePasses(value.max_capture_passes()))
                .unwrap_or(ResourceAmount::NotConfigured),
        ),
        (
            ResourceField::MaxPendingCaptureResults,
            capture
                .map(|value| ResourceAmount::CaptureResults(value.max_pending_capture_results()))
                .unwrap_or(ResourceAmount::NotConfigured),
        ),
        (
            ResourceField::MaxCaptureBytes,
            capture
                .map(|value| ResourceAmount::Bytes(value.max_capture_bytes()))
                .unwrap_or(ResourceAmount::NotConfigured),
        ),
        (
            ResourceField::MaxAudioCaptureFrames,
            capture
                .map(|value| ResourceAmount::Frames(value.max_audio_capture_frames()))
                .unwrap_or(ResourceAmount::NotConfigured),
        ),
        (
            ResourceField::MaxProjectionTicks,
            capture
                .map(|value| ResourceAmount::ProjectionTicks(value.max_projection_ticks()))
                .unwrap_or(ResourceAmount::NotConfigured),
        ),
        (
            ResourceField::CaptureLatenessAllowance,
            capture
                .map(|value| ResourceAmount::Frames(value.capture_lateness_allowance()))
                .unwrap_or(ResourceAmount::NotConfigured),
        ),
    ] {
        rows.push(ResourceRow::new(field, amount, amount, IrObject::Plan));
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
    // `SOUND-INV-027`: a modulation edge into a voice-scope parameter is one Mod Matrix
    // slot per voice. Installed voice-scope programs consume actual script host slots.
    // Both counts are admitted; the floor between their capacities is validated at
    // profile construction rather than here, as
    // `HOST-INV-017` wants the relation declared once.
    let (voice_slots, voice_slots_at) = ir.voice_modulation_slots();
    rows.push(ResourceRow::new(
        ResourceField::ModMatrixSlotsPerVoice,
        ResourceAmount::Slots(voice_slots),
        ResourceAmount::Slots(script.mod_matrix_slots_per_voice()),
        voice_slots_at,
    ));
    let (script_slots, script_slots_at) = ir.voice_script_slots();
    rows.push(ResourceRow::new(
        ResourceField::ScriptHostSlotsPerVoice,
        ResourceAmount::Slots(script_slots),
        ResourceAmount::Slots(script.script_host_slots_per_voice()),
        script_slots_at,
    ));
}

/// The distinct event and boundary-release widths charged to renderer scratch.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub(crate) struct ScratchWriteWidths {
    event: crate::quantities::WritesPerNote,
    boundary: crate::quantities::WritesPerNote,
}

impl ScratchWriteWidths {
    pub(crate) const fn new(
        event: crate::quantities::WritesPerNote,
        boundary: crate::quantities::WritesPerNote,
    ) -> Self {
        Self { event, boundary }
    }
}

/// The arena and the carries this plan needs, in bytes.
///
/// The carries are the one part that computes exactly: ADR-0001 clause 5 sizes both
/// at `maximum_block_size + Q` frames, and clause 6 primes the output one. The arena is
/// its **extent** in samples — ADR-0041 clause 13 — rather than a buffer count times the
/// quantum, because since that record its regions differ in width.
pub(crate) fn scratch_bytes(
    profile: &HostProfile,
    arena_samples: u64,
    scheduled_records: RecordCount,
    note_producer_ranges: &[crate::quantities::HeldNoteCount],
    widths: ScratchWriteWidths,
    modulated_sample_positioned_rows: u32,
    voice_instances: crate::quantities::VoiceCount,
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
    let mut identity_indices: u32 = 0;
    for range in note_producer_ranges {
        identity_indices = identity_indices.saturating_add(range.get());
    }
    let controls = crate::render::timed_control_scratch_bytes(
        profile.limits().events().max_events_per_quantum(),
        scheduled_records,
        crate::quantities::HeldNoteCount::measured(identity_indices),
        widths.event,
        widths.boundary,
        modulated_sample_positioned_rows,
        voice_instances,
    );

    // And ADR-0047's two identity halves, both sized by the producer partition this plan
    // declares. Preparation allocates them, so the budget has to cover them: a plan
    // declaring a large polyphony would otherwise be reported as fitting and then allocate
    // past the ceiling, which is the defect the event scratch already taught this function.
    let identities = crate::render::identity_bytes(note_producer_ranges);

    PreparedBytes::measured(
        buffers
            .saturating_add(carries)
            .saturating_mul(sample)
            .saturating_add(events)
            .saturating_add(controls)
            .saturating_add(identities),
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
    compensation_history: u64,
    /// The first node that could not be prepared for this stream, if any.
    ///
    /// Carried out rather than returned early, because a refusal owes a report and the
    /// report is exact only once the schedule and the arena exist. `HOST-INV-006` admits
    /// no plan-shaped exception to that.
    fault: Option<CompileError>,
    prepared_nodes: Vec<PreparedNode>,
    node_timings: Vec<(NodeId, crate::node::NodeTiming)>,
    parameter_targets: Vec<ParameterTarget>,
    parameter_addresses: Vec<ParameterAddress>,
    taps: Vec<crate::plan::TapTarget>,
    tap_addresses: Vec<crate::plan::TapAddress>,
    /// The mix channels, in ascending node identity (`SOUND-INV-031`).
    channels: Vec<crate::plan::ChannelRecord>,
    /// The buses, in ascending strip identity (`SOUND-INV-034`).
    buses: Vec<crate::plan::BusRecord>,
    /// The sends, in ascending node identity (`SOUND-INV-034`).
    sends: Vec<crate::plan::SendRecord>,
    note_targets: Vec<NoteTarget>,
    note_addresses: Vec<NoteAddress>,
    note_magnitudes: Vec<NoteMagnitudeTarget>,
    prepared_tunings: Vec<crate::tuning::PreparedTuning>,
    prepared_samples: Vec<crate::sample::PreparedSample>,
    /// The first step of each `N`-instance group, and the voice-sum groups among them
    /// (ADR-0058).
    instance_groups: Vec<NodeSlot>,
    sum_groups: Vec<NodeSlot>,
    /// How many leading operations are the modulation pre-pass (`SOUND-INV-027`).
    prepass: usize,
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
    fn into_plan(
        self,
        profile: &HostProfile,
        declarations: &PlanDeclarations,
        paths: crate::latency::PathLatencies,
    ) -> CompiledPlan {
        let capabilities = profile.capabilities();
        // A producer's position in the declaration **is** its `ProducerId`, so the ranges are
        // carried in declaration order and nothing else has to agree on a numbering.
        let note_producer_ranges: Vec<_> = declarations
            .note_producers
            .iter()
            .map(|producer| producer.simultaneous_notes)
            .collect();
        // The same order, and the same reason: an entitlement is looked up by `ProducerId`.
        let note_producer_holds: Vec<_> = declarations
            .note_producers
            .iter()
            .map(|producer| producer.simultaneous_holds)
            .collect();
        let authored_sources = declarations.authored_sources.clone();
        // Validation has already refused a second one, so the first is the only one.
        let compiled_note_producer = declarations
            .note_producers
            .iter()
            .position(|producer| producer.compiled)
            .and_then(|index| u16::try_from(index).ok())
            .map(crate::identity::ProducerId::new);
        CompiledPlan::new(
            self.id,
            self.ops,
            self.regions,
            self.prepared_nodes,
            self.node_timings,
            self.parameter_targets,
            self.parameter_addresses,
            self.taps,
            self.tap_addresses,
            self.channels,
            self.buses,
            self.sends,
            self.note_targets,
            self.note_addresses,
            self.note_magnitudes,
            self.prepared_tunings,
            self.prepared_samples,
            capabilities.channel_layout(),
            capabilities.sample_rate(),
            capabilities.maximum_block_size(),
            profile.limits().events().max_events_per_quantum(),
            profile.limits().events().shares().compiled_event_share(),
            note_producer_ranges,
            note_producer_holds,
            authored_sources,
            compiled_note_producer,
            profile.limits().events().forward_event_horizon(),
            paths,
            declarations.stealing,
            self.instance_groups,
            self.sum_groups,
            self.prepass,
        )
    }
}

/// What lowering accumulates while it walks the schedule.
struct Lowering {
    ops: Vec<PlanOp>,
    prepared_nodes: Vec<PreparedNode>,
    /// Each authored node's declared timing at the stream's rate (`SOUND-INV-033`).
    node_timings: Vec<(NodeId, crate::node::NodeTiming)>,
    /// One width per virtual buffer, in samples: `c * Q` for a signal of `c` channels.
    ///
    /// ADR-0041 clause 2. Lowering is where a signal's channel count is known — it comes
    /// from the port and its edge — and the arena is handed the widths rather than
    /// deriving them, because deriving them would make it a second authority on layout.
    widths: Vec<usize>,
    inserted: usize,
    /// How many state records have been scheduled: the next [`NodeSlot`]. Distinct from the
    /// prepared count since `P06-S001`, because a voice-scope node has one prepared record and
    /// one state per instance.
    states: usize,
}

impl Lowering {
    /// Hold one prepared record, shared by every instance scheduled over it.
    fn prepare(&mut self, prepared: PreparedNode) -> crate::plan::PreparedSlot {
        let slot = crate::plan::PreparedSlot::new(self.prepared_nodes.len());
        self.prepared_nodes.push(prepared);
        slot
    }

    /// Schedule one node: its prepared record, its output buffer, its step.
    ///
    /// The one place a step is built, so an authored node and a compiler-inserted
    /// operation are scheduled by the same code and the arena cannot tell them apart.
    fn schedule(
        &mut self,
        descriptor: &NodeDescriptor,
        prepared: crate::plan::PreparedSlot,
        inputs: [Option<BufferSlot>; MAX_INPUTS],
        layout: ChannelLayout,
        role: NodeRole,
    ) -> (NodeSlot, BufferSlot) {
        let out = BufferSlot::new(self.widths.len());
        // ADR-0041 clause 2: the width is the layout's channel count times the quantum, and
        // it is decided here, where the port's layout is known.
        self.widths
            .push(layout.channels().saturating_mul(QUANTUM_FRAMES as usize));
        (
            self.schedule_into(descriptor, prepared, inputs, layout, out, role),
            out,
        )
    }

    /// Schedule the compiler's widening of a narrower signal into `layout` (`SOUND-INV-014`):
    /// one copy writing each sample into every channel of one wider region, with the
    /// conversion recorded in the warnings so a reader of the outcome need not infer it from
    /// the operation list.
    fn widen(
        &mut self,
        source: BufferSlot,
        layout: ChannelLayout,
        edge: crate::ir::EdgeId,
        conversion: crate::validate::Conversion,
        role: NodeRole,
        warnings: &mut Vec<CompileWarning>,
    ) -> BufferSlot {
        let copy = node::copy_descriptor();
        let prepared = self.prepare(node::prepare_copy());
        let (_, out) = self.schedule(
            &copy,
            prepared,
            crate::node::kernels::pair_inputs(Some(source), None),
            layout,
            role,
        );
        self.inserted += 1;
        warnings.push(CompileWarning::ConversionInserted { edge, conversion });
        out
    }

    /// Schedule one step writing an **existing** buffer: the voice sum's accumulate, whose
    /// output is the sum region its second input also names (`P06-S001`).
    fn schedule_into(
        &mut self,
        descriptor: &NodeDescriptor,
        prepared: crate::plan::PreparedSlot,
        inputs: [Option<BufferSlot>; MAX_INPUTS],
        layout: ChannelLayout,
        out: BufferSlot,
        role: NodeRole,
    ) -> NodeSlot {
        let node = NodeSlot::new(self.states);
        self.states += 1;
        self.ops.push(PlanOp::Node(
            NodeStep::new(
                descriptor.kernel,
                node,
                prepared,
                out,
                layout,
                inputs,
                descriptor.in_place_safe,
            )
            .with_role(role),
        ));
        node
    }
}

const fn instance_role(in_voice: bool, instance: usize) -> NodeRole {
    if in_voice {
        NodeRole::Local(VoiceInstanceIndex::measured(instance))
    } else {
        NodeRole::Global
    }
}

fn lower(
    ir: &GraphIr,
    profile: &HostProfile,
    validated: &Validated,
    warnings: &mut Vec<CompileWarning>,
    policy: ArenaPolicy,
    paths: &crate::latency::PathLatencies,
) -> Lowered {
    let plan_id = issue_plan_id();
    let rate = profile.capabilities().sample_rate();
    let mut state = Lowering {
        ops: Vec::new(),
        prepared_nodes: Vec::new(),
        node_timings: Vec::new(),
        widths: Vec::new(),
        inserted: 0,
        states: 0,
    };
    let mut fault = None;
    let mut compensation_history = 0_u64;
    let compensation: HashMap<_, _> = paths
        .edges()
        .iter()
        .filter(|edge| edge.compensation() != FrameCount::ZERO)
        .map(|edge| (edge.edge(), edge.compensation()))
        .collect();
    let mut parameter_targets = Vec::new();
    let mut parameter_addresses = Vec::new();
    let mut taps: Vec<crate::plan::TapTarget> = Vec::new();
    let mut tap_addresses = Vec::new();
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
    // Every cable into each input port, in ascending source identity — the order a summed
    // port adds them in, which is a function of identity and not of declaration position
    // (`SOUND-INV-008`), and the order the one cable of an ordinary port is found in
    // (validation refused a second). Keyed by port, as `source_of` is.
    let mut edges_into: HashMap<(NodeId, PortId), Vec<(crate::ir::EdgeId, NodeId)>> =
        HashMap::with_capacity(ir.edges().len());
    for edge in ir.edges() {
        edges_into
            .entry(edge.to())
            .or_default()
            .push((edge.id(), edge.from().0));
    }
    for arrivals in edges_into.values_mut() {
        arrivals.sort_by_key(|(edge, from)| (*from, *edge));
    }
    // The validator's record of which edges need the widening, which is the one authority
    // on it; lowering does not re-derive it from the layouts.
    let converted: HashMap<crate::ir::EdgeId, crate::validate::Conversion> = validated
        .conversions()
        .iter()
        .map(|conversion| (conversion.edge, conversion.conversion))
        .collect();
    let mut channels: Vec<crate::plan::ChannelRecord> = Vec::new();
    // `SOUND-INV-034`: the bus strips and the sends, with their slots, resolved into
    // records once every identity is minted below.
    let mut bus_strips: Vec<(NodeId, crate::ir::BusTag, [ParameterSlot; 3])> = Vec::new();
    let mut send_nodes: Vec<(
        NodeId,
        crate::ir::ExecutionScope,
        bool,
        ParameterSlot,
        ParameterSlot,
    )> = Vec::new();
    let mut slots: HashMap<NodeId, BufferSlot> = HashMap::with_capacity(ir.nodes().len());
    // The node's place in the *state* tables, which is what a note target addresses. Kept
    // beside the buffer slots rather than derived from them: they are two numberings, and
    // the output node has one of them and not the other.
    let mut node_slots: HashMap<NodeId, NodeSlot> = HashMap::with_capacity(ir.nodes().len());

    let kinds: HashMap<NodeId, IrNodeKind> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.kind()))
        .collect();

    // `P06-S001`: the voice scope is instantiated once per identity index of the plan's
    // note producers, over one prepared record per node. Every other scope renders once.
    let voices = usize::try_from(ir.voice_instances().get()).unwrap_or(usize::MAX);
    let scopes: HashMap<NodeId, crate::ir::ExecutionScope> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.scope()))
        .collect();
    // A voice-scope node's outputs, one per instance, in instance order.
    let mut voice_slots: HashMap<NodeId, Vec<BufferSlot>> = HashMap::new();
    // The voice-scope nodes whose output the scope's outside reads, summed below.
    let summed = voice_sum_sources(ir);
    // ADR-0058: a stealing plan sums even a single voice, so the taken voice has a sum step
    // to be faded on; and it records where each instance's steps begin.
    let steals = ir.declarations().stealing.steals();
    let mut instance_groups: Vec<NodeSlot> = Vec::new();
    let mut sum_groups: Vec<NodeSlot> = Vec::new();

    // ADR-0026 clause 3: the plan's sample table, one entry per **distinct** sample the IR
    // holds, compared by content as the tunings are and for the same reason — a digest is
    // a hash and a collision would have two zones read one buffer. Every IR reference
    // resolves to a slot here, and that resolution is what the prepare context carries.
    // Only the samples a sampler node reaches through its map are prepared into the plan:
    // `GraphIr::sample_bytes` charges exactly that set, and a sample the IR holds but no
    // sampler plays must not be allocated unpaid for. An independent read found every IR
    // sample prepared. An unreached reference resolves to no slot.
    let mut reached = vec![false; ir.samples().len()];
    for node in ir.nodes() {
        if let IrNodeKind::Sampler { map, .. } = node.kind()
            && let Some(map) = ir.map_of(map)
        {
            for zone in map.zones() {
                if let Some(flag) = reached.get_mut(zone.sample().index()) {
                    *flag = true;
                }
            }
        }
    }
    let mut prepared_samples: Vec<crate::sample::PreparedSample> = Vec::new();
    let sample_slots: Vec<Option<crate::plan::SampleSlot>> = ir
        .samples()
        .iter()
        .zip(reached)
        .map(|(sample, reached)| {
            if !reached {
                return None;
            }
            let index = prepared_samples
                .iter()
                .position(|held| held == sample)
                .unwrap_or_else(|| {
                    prepared_samples.push(sample.clone());
                    prepared_samples.len() - 1
                });
            Some(crate::plan::SampleSlot::new(plan_id, index))
        })
        .collect();
    let context = node::PrepareContext {
        rate,
        ir,
        samples: &sample_slots,
    };

    // `SOUND-INV-027`: the modulation sources are lowered first, in the validated order
    // among themselves, so their steps form the pre-pass the renderer runs ahead of the
    // quantum's positioned writes. A source depends on no cable (validation refused one
    // with an input) and on no node but an earlier source (a modulation into it), so
    // pulling it forward keeps every dependency the order established. A plan with no
    // modulation lowers in exactly the validated order, and its schedule is unchanged.
    let mut order: Vec<NodeId> = validated
        .order()
        .iter()
        .copied()
        .filter(|id| ir.is_modulation_source(*id))
        .collect();
    let sources = order.len();
    order.extend(
        validated
            .order()
            .iter()
            .copied()
            .filter(|id| !ir.is_modulation_source(*id)),
    );
    // The modulation steps for targets outside the pre-pass, gathered as their rows are
    // built and spliced in after the last source's step; the steps for targets inside it
    // are inserted ahead of the target's own step as it is lowered.
    let mut deferred_modulations: Vec<PlanOp> = Vec::new();
    let mut prepass_end: Option<usize> = None;

    for (position, id) in order.iter().enumerate() {
        if position == sources {
            prepass_end = Some(state.ops.len());
        }
        let Some(kind) = kinds.get(id).copied() else {
            continue;
        };
        let Some(descriptor) = ir.descriptor(kind) else {
            lower_output(
                ir, profile, validated, warnings, &mut state, &slots, &source_of, *id,
            );
            continue;
        };
        let in_voice = scopes.get(id) == Some(&crate::ir::ExecutionScope::Voice);
        let instances = if in_voice { voices } else { 1 };

        let mut prepared = match node::prepare(*id, kind, &context) {
            Ok(prepared) => prepared,
            Err(error) => {
                // The schedule is still built, with silence where the node would have
                // been, so the arena and the report describe a plan of the right shape.
                // Nothing renders it: the outcome carries the refusal.
                fault = fault.or(Some(error));
                PreparedNode::Silence
            }
        };
        // ADR-0026 clause 6: the zone's root is a key, and its frequency is the scope's
        // tuning's answer — resolved here, where the scope is known, and never in the
        // kernel. A scope with no tuning leaves it at zero, which renders silence; a note
        // reaching the sampler's pitch destination is refused by `bind_note_magnitudes`
        // for the want of that tuning, so the zero is never what a played note gets.
        if let PreparedNode::Sampler {
            root,
            root_frequency,
            ..
        } = &mut prepared
            && let Some(tuning) = scopes.get(id).and_then(|scope| ir.tuning_of(*scope))
        {
            *root_frequency = tuning.frequency_of(*root);
        }
        // One prepared record, whatever the instance count: shared, never cloned.
        let prepared_slot = state.prepare(prepared);
        // The declaration's timing at this rate, kept per node for the diagnostics reader
        // (`SOUND-INV-033`); the walk is in ascending identity, so the table is too.
        state.node_timings.push((*id, node::timing_of(kind, rate)));
        // ADR-0041 clause 5: the channel count is a property of the port, so the width of
        // the region the node writes comes from the port table rather than from the
        // stream. Every authored kind declares a mono output today; asking the port is
        // what makes that a fact about the node rather than an assumption here.
        let out_layout = descriptor
            .ports
            .iter()
            .find(|port| port.direction() == crate::validate::PortDirection::Output)
            .map_or(ChannelLayout::Mono, |port| port.layout());

        // Resolve every source/widening first, then schedule each compensation cable's
        // instances contiguously. These groups own reset state just like authored voice
        // nodes; compensation after a voice sum is shared and receives no steal reset.
        let mut aligned: HashMap<crate::ir::EdgeId, Vec<BufferSlot>> = HashMap::new();
        for port in descriptor
            .ports
            .iter()
            .filter(|port| port.direction() == crate::validate::PortDirection::Input)
        {
            for (edge, from) in edges_into
                .get(&(*id, port.id()))
                .map_or(&[][..], Vec::as_slice)
            {
                let Some(frames) = compensation.get(edge) else {
                    continue;
                };
                let mut sources = Vec::with_capacity(instances);
                for instance in 0..instances {
                    let source = match voice_slots.get(from) {
                        Some(per_instance) if in_voice => per_instance.get(instance).copied(),
                        _ => slots.get(from).copied(),
                    };
                    if let Some(source) = source {
                        sources.push((
                            instance,
                            match converted.get(edge) {
                                Some(conversion) => state.widen(
                                    source,
                                    port.layout(),
                                    *edge,
                                    *conversion,
                                    instance_role(in_voice, instance),
                                    warnings,
                                ),
                                None => source,
                            },
                        ));
                    }
                }
                let len = match frames.as_usize() {
                    Some(len) => len,
                    None => {
                        fault = fault.or(Some(CompileError::LatencyUnrepresentable {
                            node: *id,
                            frames: *frames,
                        }));
                        0
                    }
                };
                let mut buffers = Vec::with_capacity(instances);
                for (instance, source) in sources {
                    let prepared = state.prepare(PreparedNode::Latency { frames: len });
                    let (slot, buffer) = state.schedule(
                        &node::latency_descriptor(),
                        prepared,
                        crate::node::kernels::pair_inputs(Some(source), None),
                        port.layout(),
                        instance_role(in_voice, instance),
                    );
                    if in_voice && buffers.is_empty() {
                        instance_groups.push(slot);
                    }
                    buffers.push(buffer);
                    state.inserted += 1;
                    compensation_history = compensation_history.saturating_add(
                        frames
                            .as_u64()
                            .saturating_mul(port.layout().channels() as u64)
                            .saturating_mul(size_of::<f32>() as u64),
                    );
                }
                aligned.insert(*edge, buffers);
            }
        }

        // Every instance's inputs are resolved — and any widening scheduled — **before** the
        // first instance step, and every accumulate after the last, so the instance steps
        // occupy contiguous state slots: a parameter target and an instance group address
        // instance `k` as `first + k`, and an inserted step between two instances would be
        // what instance `k` addressed. An independent read built exactly that.
        let mut bound: Vec<([Option<BufferSlot>; MAX_INPUTS], Vec<BufferSlot>)> =
            Vec::with_capacity(instances);
        for instance in 0..instances {
            // Inputs in the order the node declares them, so port identity — not edge
            // order, and not declaration order — decides which slot a kernel reads first.
            // A source in the voice scope is read from the **same instance**; a source
            // outside it is the one shared buffer every instance reads.
            let mut inputs = [None; MAX_INPUTS];
            // The cables a summed port adds after its first, each already the port's layout
            // (`SOUND-INV-031`); accumulated into the node's own output once its step exists.
            let mut summed: Vec<BufferSlot> = Vec::new();
            let declared = descriptor
                .ports
                .iter()
                .filter(|port| port.direction() == crate::validate::PortDirection::Input);
            for (index, port) in declared.enumerate() {
                let arrivals = if matches!(kind, IrNodeKind::FeedbackDelay) {
                    &[][..]
                } else {
                    edges_into
                        .get(&(*id, port.id()))
                        .map_or(&[][..], Vec::as_slice)
                };
                let mut resolved: Vec<BufferSlot> = Vec::with_capacity(arrivals.len());
                for (edge, from) in arrivals {
                    if let Some(buffer) = aligned
                        .get(edge)
                        .and_then(|buffers| buffers.get(instance))
                        .copied()
                    {
                        resolved.push(buffer);
                        continue;
                    }
                    let buffer = match voice_slots.get(from) {
                        // Only a consumer inside the scope reads a voice-scope source
                        // per instance; one outside it reads what the scope's outside
                        // reads — the voice sum, which is what `slots` holds for it.
                        Some(per_instance) if in_voice => per_instance.get(instance).copied(),
                        _ => slots.get(from).copied(),
                    };
                    let Some(buffer) = buffer else {
                        continue;
                    };
                    // `SOUND-INV-014`: a narrower signal reaching a declared wider input is
                    // widened by a scheduled conversion, exactly as one reaching the output
                    // is, on the validator's record of the edge.
                    let buffer = match converted.get(edge) {
                        Some(conversion) => state.widen(
                            buffer,
                            port.layout(),
                            *edge,
                            *conversion,
                            instance_role(in_voice, instance),
                            warnings,
                        ),
                        None => buffer,
                    };
                    resolved.push(buffer);
                    if port.fan_in() == crate::validate::FanIn::One {
                        break;
                    }
                }
                let mut resolved = resolved.into_iter();
                let source = resolved.next();
                summed.extend(resolved);
                if let Some(entry) = inputs.get_mut(index) {
                    *entry = source;
                }
            }
            bound.push((inputs, summed));
        }
        let mut first_node = None;
        let mut outs = Vec::with_capacity(instances);
        let mut sums: Vec<(BufferSlot, Vec<BufferSlot>)> = Vec::with_capacity(instances);
        for (instance, (inputs, summed)) in bound.into_iter().enumerate() {
            let (node_slot, out) = state.schedule(
                &descriptor,
                prepared_slot,
                inputs,
                out_layout,
                instance_role(in_voice, instance),
            );
            let first = *first_node.get_or_insert(node_slot);
            // The contiguity the addressing relies on, held rather than assumed: a step
            // scheduled between two instances is a compiler defect, refused here.
            if node_slot.index() != first.index().saturating_add(outs.len()) {
                fault = fault.or(Some(CompileError::DestinationWithoutSlot { node: *id }));
            }
            outs.push(out);
            sums.push((out, summed));
        }
        // `SOUND-INV-031`: each instance's own step seeded its output with the first cable;
        // every further cable is accumulated into that region, in identity order, by the
        // voice sum's own step. Linear, in float, unclamped.
        for (instance, (out, summed)) in sums.into_iter().enumerate() {
            for source in summed {
                let accumulate = node::accumulate_descriptor();
                let accumulate_prepared = state.prepare(node::prepare_copy());
                let _ = state.schedule_into(
                    &accumulate,
                    accumulate_prepared,
                    crate::node::kernels::pair_inputs(Some(source), Some(out)),
                    out_layout,
                    out,
                    instance_role(in_voice, instance),
                );
                state.inserted += 1;
            }
        }
        let Some(node_slot) = first_node else {
            continue;
        };
        node_slots.insert(*id, node_slot);
        if in_voice {
            voice_slots.insert(*id, outs.clone());
            instance_groups.push(node_slot);
        }
        // What the scope's outside reads: the one output where there is one instance, and
        // the **voice sum** where there are several — instance 0 copied into a sum region,
        // every later instance accumulated into it. The sum is inserted work, and is only
        // inserted where something outside the scope reads the node.
        let outside_reads = if in_voice {
            if (instances > 1 || steals) && summed.binary_search(id).is_ok() {
                let mut sum = None;
                for (instance, out) in outs.iter().copied().enumerate() {
                    match sum {
                        None => {
                            let copy = node::copy_descriptor();
                            let copy_prepared = state.prepare(node::prepare_copy());
                            let (first_sum, region) = state.schedule(
                                &copy,
                                copy_prepared,
                                crate::node::kernels::pair_inputs(Some(out), None),
                                out_layout,
                                NodeRole::SharedSum,
                            );
                            state.inserted += 1;
                            instance_groups.push(first_sum);
                            sum_groups.push(first_sum);
                            sum = Some(region);
                        }
                        Some(region) => {
                            let accumulate = node::accumulate_descriptor();
                            let accumulate_prepared = state.prepare(node::prepare_copy());
                            let _ = state.schedule_into(
                                &accumulate,
                                accumulate_prepared,
                                crate::node::kernels::pair_inputs(Some(out), Some(region)),
                                out_layout,
                                region,
                                NodeRole::SharedSum,
                            );
                            state.inserted += 1;
                            let _ = instance;
                        }
                    }
                }
                sum
            } else {
                outs.first().copied()
            }
        } else {
            outs.first().copied()
        };
        if let Some(out) = outside_reads {
            slots.insert(*id, out);
        }

        // One addressable slot per control, and one **row per instance** of the node behind
        // it, grouped: a write to the slot fans out over the group, and a note's magnitude
        // lands on the row of its own instance (`P06-S001`). A not-modulatable control
        // compiles to no slot at all (`SOUND-INV-023`).
        //
        // `SOUND-INV-027`: the modulation steps into each row, one per edge landing on the
        // control, the last of them marked so the row composes once. A source in the voice
        // scope is read from the row's own instance; one outside it is read from the buffer
        // every instance shares. A step for a control of a modulation source is inserted
        // ahead of the source's own steps in the pre-pass; a step for any other control is
        // deferred and spliced in after the last source.
        let is_source = ir.is_modulation_source(*id);
        let mut own_modulations: Vec<PlanOp> = Vec::new();
        for spec in &descriptor.controls {
            if !spec.law.admits_writes() {
                continue;
            }
            let slot = ParameterSlot::new(plan_id, parameter_targets.len());
            let landing: Vec<&crate::ir::IrModulation> = ir
                .modulations()
                .iter()
                .filter(|modulation| modulation.target() == (*id, spec.parameter))
                .collect();
            for instance in 0..instances {
                let row = parameter_targets.len().saturating_add(instance);
                for (position, modulation) in landing.iter().enumerate() {
                    let (source, _) = modulation.source();
                    let buffer = match voice_slots.get(&source) {
                        Some(per_instance) => per_instance.get(instance).copied(),
                        None => slots.get(&source).copied(),
                    };
                    let Some(buffer) = buffer else {
                        continue;
                    };
                    let step = PlanOp::Modulate(crate::plan::ModulationStep::new(
                        buffer,
                        row,
                        modulation.depth().amount(),
                        position + 1 == landing.len(),
                    ));
                    if is_source {
                        own_modulations.push(step);
                    } else {
                        deferred_modulations.push(step);
                    }
                }
            }
            for instance in 0..instances {
                parameter_targets.push(ParameterTarget {
                    controller: spec.controller,
                    node: NodeSlot::new(node_slot.index().saturating_add(instance)),
                    control: spec.control,
                    law: spec.law,
                    unit: spec.default.unit(),
                    smoothing: spec.smoothing,
                    instances: crate::quantities::VoiceCount::measured(
                        u32::try_from(instances).unwrap_or(u32::MAX),
                    ),
                    // The value the node was prepared with; the declaration's resting value
                    // where the prepared record carries none — a gate released, a velocity full.
                    base: crate::node::kernels::authored_value(&prepared, spec.control)
                        .unwrap_or(spec.default.as_parameter_value()),
                    rate: spec.rate,
                });
            }
            parameter_addresses.push(ParameterAddress {
                node: *id,
                parameter: spec.parameter,
                slot,
            });
        }
        // `SOUND-INV-031`: a channel record with the slots its three controls compile to;
        // its identity is minted below, once every channel is known. `SOUND-INV-034`: the
        // same strip in a bus scope is the bus's, and a send keeps its level's slot.
        let slot_of = |parameter: crate::ir::ParameterId| {
            parameter_addresses
                .iter()
                .rev()
                .find(|address| address.node == *id && address.parameter == parameter)
                .map(|address| address.slot)
        };
        if matches!(kind, IrNodeKind::Channel { .. }) {
            match (
                slot_of(crate::ir::parameters::CHANNEL_FADER),
                slot_of(crate::ir::parameters::CHANNEL_PAN),
                slot_of(crate::ir::parameters::CHANNEL_MUTE),
                scopes.get(id).copied(),
            ) {
                (Some(fader), Some(pan), Some(mute), Some(crate::ir::ExecutionScope::Bus(tag))) => {
                    bus_strips.push((*id, tag, [fader, pan, mute]));
                }
                (
                    Some(fader),
                    Some(pan),
                    Some(mute),
                    Some(crate::ir::ExecutionScope::Channel(tag)),
                ) => {
                    channels.push(crate::plan::ChannelRecord {
                        id: crate::plan::ChannelId::new(plan_id, channels.len()),
                        tag,
                        node: *id,
                        fader,
                        pan,
                        mute,
                    });
                }
                _ => fault = fault.or(Some(CompileError::DestinationWithoutSlot { node: *id })),
            }
        }
        let send_slots = match kind {
            IrNodeKind::Send { .. } => Some((
                slot_of(crate::ir::parameters::SEND_LEVEL),
                slot_of(crate::ir::parameters::SEND_MUTE),
                false,
            )),
            IrNodeKind::PostFaderSend { .. } => Some((
                slot_of(crate::ir::parameters::POST_FADER_SEND_LEVEL),
                slot_of(crate::ir::parameters::POST_FADER_SEND_MUTE),
                true,
            )),
            _ => None,
        };
        if let Some((level, mute, post_fader)) = send_slots {
            match (level, mute, scopes.get(id).copied()) {
                (Some(level), Some(mute), Some(scope)) => {
                    send_nodes.push((*id, scope, post_fader, level, mute));
                }
                _ => fault = fault.or(Some(CompileError::DestinationWithoutSlot { node: *id })),
            }
        }
        if !own_modulations.is_empty() {
            // Ahead of this source's steps: the composition its kernel then reads. The steps
            // are the last `instances` operations scheduled, since a source has no sum.
            let at = state.ops.len().saturating_sub(instances);
            for (offset, step) in own_modulations.into_iter().enumerate() {
                state.ops.insert(at.saturating_add(offset), step);
            }
        }

        // `SOUND-INV-022`: the kind's declared taps, one row per instance, each naming that
        // instance's step and its output region — present in the plan whether or not anything
        // will subscribe. The address names instance 0's row; a per-instance tap is later
        // work. The region is the lowering's virtual slot here; the arena's mapping resolves
        // it below.
        for tap in crate::node::declaration(kind).map_or(&[][..], |declared| declared.taps) {
            let slot = crate::plan::TapSlot::new(plan_id, taps.len());
            for (instance, out) in outs.iter().enumerate() {
                taps.push(crate::plan::TapTarget {
                    node: NodeSlot::new(node_slot.index().saturating_add(instance)),
                    region: *out,
                    data: tap.data,
                    bytes_per_quantum: crate::quantities::QuantumBytes::measured(
                        (out_layout.channels() as u64)
                            .saturating_mul(u64::from(crate::time::QUANTUM_FRAMES))
                            .saturating_mul(size_of::<f32>() as u64),
                    ),
                });
            }
            tap_addresses.push(crate::plan::TapAddress {
                node: *id,
                port: tap.port,
                slot,
            });
        }

        // A playable node gets one note slot. The control it names is the kind's, so a
        // caller plays the node and never learns which control being played moves — which
        // is what lets Phase 6's voice pool address a voice without knowing its graph.
        if let Some(control) = descriptor.note_control {
            // The gate's own parameter slot, which its edges are composed through: the first
            // row of the control's group, the renderer adding the instance. Absent only if
            // the gate declared itself not-modulatable, which the declaration tests forbid
            // for a note control; refused rather than played around.
            let Some(parameter) = parameter_addresses
                .iter()
                .rev()
                .find(|address| address.node == *id)
                .and_then(|_| {
                    parameter_targets
                        .iter()
                        .position(|target| target.node == node_slot && target.control == control)
                })
                .map(|index| ParameterSlot::new(plan_id, index))
            else {
                fault = fault.or(Some(CompileError::DestinationWithoutSlot { node: *id }));
                continue;
            };
            let slot = NoteSlot::new(plan_id, note_targets.len());
            note_targets.push(NoteTarget {
                node: node_slot,
                control,
                parameter,
                // Filled by `bind_note_magnitudes` below, which needs every node scheduled
                // before it can collect a scope's destinations: a played node may be
                // lowered before the oscillator its key reaches.
                magnitudes: crate::plan::NoteMagnitudeRange::EMPTY,
            });
            note_addresses.push(NoteAddress { node: *id, slot });
        }
    }

    // `SOUND-INV-021`'s binding, and it runs here rather than inside the walk because a
    // scope's destinations are only complete once every node of that scope is scheduled.
    let mut note_magnitudes = Vec::new();
    let mut prepared_tunings = Vec::new();
    if let Err(error) = bind_note_magnitudes(
        ir,
        plan_id,
        &node_slots,
        &parameter_targets,
        &note_addresses,
        &mut note_targets,
        &mut note_magnitudes,
        &mut prepared_tunings,
    ) {
        fault = fault.or(Some(error));
    }

    // `SOUND-INV-027`: the pre-pass is the sources' steps, their own compositions, and then
    // every composition into a target outside it — all before the first main-walk step.
    let prepass_end = prepass_end.unwrap_or(state.ops.len());
    let prepass = prepass_end.saturating_add(deferred_modulations.len());
    for (offset, step) in deferred_modulations.into_iter().enumerate() {
        state.ops.insert(prepass_end.saturating_add(offset), step);
    }

    // `SOUND-INV-031`: a channel's identity is its position among the plan's channels in
    // ascending node identity — not in schedule order, which is a function of identity too
    // but of the graph's shape besides, so two plans with the same channels in another
    // topology would number them differently.
    channels.sort_by_key(|record| record.node);
    for (index, record) in channels.iter_mut().enumerate() {
        record.id = crate::plan::ChannelId::new(plan_id, index);
    }
    // `SOUND-INV-034`: a bus's identity is its position among the plan's buses in ascending
    // strip identity, for the reason a channel's is; its entry is the one sum of its scope,
    // which validation held to exactly one. A send's record names whose it is by its scope's
    // tag, where it taps by its kind and scope, and the bus it enters by its one cable's
    // target — every one a fact validation checked, resolved here to the minted identities.
    bus_strips.sort_by_key(|(strip, _, _)| *strip);
    let entry_of: HashMap<crate::ir::ExecutionScope, NodeId> = ir
        .nodes()
        .iter()
        .filter(|node| {
            matches!(node.kind(), IrNodeKind::Mix)
                && matches!(node.scope(), crate::ir::ExecutionScope::Bus(_))
        })
        .map(|node| (node.scope(), node.id()))
        .collect();
    let mut buses: Vec<crate::plan::BusRecord> = Vec::with_capacity(bus_strips.len());
    for (index, (strip, tag, [fader, pan, mute])) in bus_strips.iter().copied().enumerate() {
        match entry_of.get(&crate::ir::ExecutionScope::Bus(tag)).copied() {
            Some(entry) => buses.push(crate::plan::BusRecord {
                id: crate::plan::BusId::new(plan_id, index),
                tag,
                entry,
                strip,
                fader,
                pan,
                mute,
            }),
            None => fault = fault.or(Some(CompileError::DestinationWithoutSlot { node: strip })),
        }
    }
    let channel_by_tag: HashMap<crate::ir::ChannelTag, crate::plan::ChannelId> = channels
        .iter()
        .map(|record| (record.tag, record.id))
        .collect();
    let bus_by_tag: HashMap<crate::ir::BusTag, crate::plan::BusId> =
        buses.iter().map(|record| (record.tag, record.id)).collect();
    send_nodes.sort_by_key(|(node, _, _, _, _)| *node);
    let mut sends: Vec<crate::plan::SendRecord> = Vec::with_capacity(send_nodes.len());
    for (node, scope, post_fader, level, mute) in send_nodes {
        let from = match scope {
            crate::ir::ExecutionScope::Channel(tag) => channel_by_tag
                .get(&tag)
                .copied()
                .map(crate::plan::SendSource::Channel),
            crate::ir::ExecutionScope::Bus(tag) => bus_by_tag
                .get(&tag)
                .copied()
                .map(crate::plan::SendSource::Bus),
            _ => None,
        };
        let into = ir
            .edges()
            .iter()
            .find(|edge| edge.from().0 == node)
            .and_then(|edge| match ir.scope_of(edge.to().0) {
                Some(crate::ir::ExecutionScope::Bus(tag)) => bus_by_tag.get(&tag).copied(),
                _ => None,
            });
        let tap = if post_fader || matches!(scope, crate::ir::ExecutionScope::Bus(_)) {
            crate::plan::SendTap::PostFader
        } else {
            crate::plan::SendTap::PreFader
        };
        match (from, into) {
            (Some(from), Some(into)) => sends.push(crate::plan::SendRecord {
                node,
                from,
                tap,
                into,
                level,
                mute,
            }),
            _ => fault = fault.or(Some(CompileError::DestinationWithoutSlot { node })),
        }
    }

    // Capture boundaries after every graph read. The arena includes these reads when
    // deciding liveness and in-place reuse; history itself is outside the arena.
    for id in &order {
        if kinds.get(id) != Some(&IrNodeKind::FeedbackDelay) {
            continue;
        }
        let Some((edge, from)) = edges_into
            .get(&(*id, PortId::FIRST))
            .and_then(|arrivals| arrivals.first())
        else {
            continue;
        };
        let (Some(mut source), Some(node)) =
            (slots.get(from).copied(), node_slots.get(id).copied())
        else {
            fault = fault.or(Some(CompileError::DestinationWithoutSlot { node: *id }));
            continue;
        };
        if let Some(conversion) = converted.get(edge) {
            source = state.widen(
                source,
                ChannelLayout::Stereo,
                *edge,
                *conversion,
                NodeRole::Global,
                warnings,
            );
        }
        state.ops.push(PlanOp::FeedbackWrite { node, source });
    }

    // ADR-0005: lowering emits one buffer per value; the arena decides which of them
    // share storage, once, here. The render loop reads slot indices and learns nothing
    // about it.
    // ADR-0005 clause 6: a tapped value is read after the schedule, so its region stays
    // live to the end of the quantum. The taps' virtual slots are what the arena pins.
    let tapped: Vec<usize> = taps.iter().map(|tap| tap.region.index()).collect();
    let assignment = arena::assign(&state.ops, &state.widths, policy, &tapped);
    arena::rewrite(&mut state.ops, &assignment.mapping, &assignment.regions);
    // The same mapping the operations were rewritten through, so a tap names the physical
    // region the node's output actually occupies.
    for tap in &mut taps {
        if let Some(physical) = assignment.mapping.get(tap.region.index()).copied() {
            tap.region = physical;
        }
    }

    Lowered {
        id: plan_id,
        ops: state.ops,
        regions: assignment.regions,
        inserted: state.inserted,
        compensation_history,
        fault,
        prepared_nodes: state.prepared_nodes,
        node_timings: state.node_timings,
        instance_groups,
        sum_groups,
        parameter_targets,
        parameter_addresses,
        taps,
        tap_addresses,
        channels,
        buses,
        sends,
        note_targets,
        note_addresses,
        note_magnitudes,
        prepared_tunings,
        prepared_samples,
        prepass,
    }
}

/// Bind each note target to the magnitude destinations its execution scope declares.
///
/// `SOUND-INV-021`'s answer to how a key reaches a node that is not the played one: a
/// producer names a node, and admission collects — from every node **kind** within that
/// node's scope — the controls that kind declares as a pitch or velocity destination. The
/// caller never names a destination, so `SOUND-INV-016`'s ownership is untouched; what
/// changes is that one note-on now resolves to more than one control write.
///
/// Three refusals live here, and each is the narrowest rule that is implementable now:
///
/// - two playable nodes in one island of the voice scope, which the rule above cannot tell apart because
///   `ExecutionScope::Voice` names a kind rather than an instance;
/// - a note scope declaring no velocity destination, because the Phase 4 gate says a
///   fixed-velocity render cannot satisfy it and a typed velocity reaching nothing is that
///   render with extra steps;
/// - a pitch destination whose scope states no tuning, because a key with nothing to
///   resolve against has no frequency and this crate may not invent one.
#[allow(
    clippy::too_many_arguments,
    reason = "the binder reads the lowering's tables by name; bundling them would hide which \
              of them a destination is resolved against"
)]
fn bind_note_magnitudes(
    ir: &GraphIr,
    plan_id: crate::plan::PlanId,
    node_slots: &HashMap<NodeId, NodeSlot>,
    parameter_targets: &[ParameterTarget],
    note_addresses: &[NoteAddress],
    note_targets: &mut [NoteTarget],
    note_magnitudes: &mut Vec<NoteMagnitudeTarget>,
    prepared_tunings: &mut Vec<crate::tuning::PreparedTuning>,
) -> Result<(), CompileError> {
    let scopes: HashMap<NodeId, crate::ir::ExecutionScope> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.scope()))
        .collect();
    // `P08-S002`: a note's destinations are its **island's** — the nodes of its scope it is
    // connected to — so a whole project's instruments, each its own island in the one voice
    // scope, are bound apart.
    let islands = ir.note_islands();
    // The slot of each (node, control), indexed once: a destination is resolved by one
    // lookup rather than a search over the target table per destination, which an
    // independent review found quadratic in the node count for a scope of many pitch
    // destinations — and lowering runs before the exact budget refusal.
    let slot_of: HashMap<(NodeSlot, crate::node::kernels::ControlIndex), ParameterSlot> =
        parameter_targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                (
                    (target.node, target.control),
                    ParameterSlot::new(plan_id, index),
                )
            })
            .collect();

    // Two playable nodes in one island share one set of destinations, so playing either
    // would move the other's velocity. Checked over the note addresses, which is exactly
    // the set of playable nodes. The invariant states the rule over the voice scope — that
    // is where two instruments land — and the check is over every scope's islands because
    // the reason is: the binding merges within an island, whichever scope it is in.
    let mut played: Vec<(u32, NodeId)> = Vec::new();
    for address in note_addresses {
        let (Some(scope), Some(island)) = (
            scopes.get(&address.node).copied(),
            islands.get(&address.node).copied(),
        ) else {
            continue;
        };
        if let Some((_, first)) = played.iter().find(|(held, _)| *held == island) {
            return Err(CompileError::AmbiguousNoteScope {
                first: *first,
                second: address.node,
                scope,
            });
        }
        played.push((island, address.node));
    }

    for address in note_addresses {
        let (Some(scope), Some(island)) = (
            scopes.get(&address.node).copied(),
            islands.get(&address.node).copied(),
        ) else {
            continue;
        };
        let start = note_magnitudes.len();
        let mut has_velocity = false;
        // Every node of the island, in the plan's declaration order, so the expansion a note
        // produces is the same list on every admission of one plan.
        for node in ir.nodes() {
            if islands.get(&node.id()).copied() != Some(island) {
                continue;
            }
            let (Some(descriptor), Some(slot)) =
                (ir.descriptor(node.kind()), node_slots.get(&node.id()))
            else {
                continue;
            };
            for spec in &descriptor.controls {
                let Some(magnitude) = spec.magnitude else {
                    continue;
                };
                let tuning = match magnitude {
                    NoteMagnitude::Velocity => {
                        has_velocity = true;
                        None
                    }
                    // ADR-0026 clause 5: an edge, resolved through nothing.
                    NoteMagnitude::Trigger | NoteMagnitude::Source(_) => None,
                    NoteMagnitude::Pitch => {
                        let Some(tuning) = ir.tuning_of(scope) else {
                            return Err(CompileError::ScopeWithoutTuning {
                                node: node.id(),
                                scope,
                            });
                        };
                        // Deduplicated by comparing the tables themselves: `SOUND-INV-021`
                        // charges one prepared table once however many nodes reference it,
                        // so two scopes naming one scale must not become two tables in the
                        // report. **Not by digest** — that is a 64-bit hash, and a collision
                        // would make the second scope resolve every key through the first
                        // scale. Off the audio thread, once per destination.
                        let index = prepared_tunings
                            .iter()
                            .position(|held: &crate::tuning::PreparedTuning| held == tuning)
                            .unwrap_or_else(|| {
                                prepared_tunings.push(tuning.clone());
                                prepared_tunings.len() - 1
                            });
                        Some(crate::plan::TuningSlot::new(plan_id, index))
                    }
                };
                // Composed through the control's own slot, so a destination a note reaches
                // is one a `SetParameter` reaches the same way. A magnitude on a control
                // with no slot is a declaration the tests forbid; refused here, not skipped.
                let Some(parameter) = slot_of.get(&(*slot, spec.control)).copied() else {
                    return Err(CompileError::DestinationWithoutSlot { node: node.id() });
                };
                note_magnitudes.push(NoteMagnitudeTarget {
                    node: *slot,
                    control: spec.control,
                    parameter,
                    magnitude,
                    tuning,
                });
            }
        }
        if !has_velocity {
            return Err(CompileError::NoteScopeWithoutVelocity {
                node: address.node,
                scope,
            });
        }
        if let Some(target) = note_targets.get_mut(address.slot.index()) {
            target.magnitudes = crate::plan::NoteMagnitudeRange::new(
                start,
                note_magnitudes.len().saturating_sub(start),
            );
        }
    }
    Ok(())
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
            let prepared = state.prepare(node::prepare_copy());
            let (_, out) = state.schedule(
                &copy,
                prepared,
                crate::node::kernels::pair_inputs(Some(source), None),
                layout,
                NodeRole::Global,
            );
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
