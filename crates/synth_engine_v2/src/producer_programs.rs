//! Cold proofs and fixed records for the two real mixed load consumers.
use crate::{
    diagnostics::CompileError,
    ir::{ExecutionScope, InternalProducerDeclaration, PlanDeclarations, RuntimeParameterProgram},
    node::kernels::{PreparedNode, TRIM_LEVEL},
    plan::{CompiledPlan, NodeRole, NodeSlot, ParameterSlot, PlanOp},
    quantities::EventCount,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParameterProgram {
    pub(crate) declaration: RuntimeParameterProgram,
    pub(crate) slots: Box<[ParameterSlot]>,
    /// Entry indices, one per distinct stable target, in stable target order.
    pub(crate) targets: Box<[usize]>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InternalProgram {
    pub(crate) declaration: InternalProducerDeclaration,
    pub(crate) node: NodeSlot,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProducerPrograms {
    pub(crate) authored: Box<[ParameterProgram]>,
    pub(crate) internal: Box<[InternalProgram]>,
    pub(crate) internal_bound: EventCount,
    pub(crate) internal_clear_work: synth_core::SampleCount,
}
fn refuse(reason: &'static str) -> CompileError {
    CompileError::ProducerProgram { reason }
}
impl ProducerPrograms {
    pub(crate) fn present(&self) -> bool {
        !self.authored.is_empty() || !self.internal.is_empty()
    }
    pub(crate) fn prepare(
        declarations: &PlanDeclarations,
        plan: &CompiledPlan,
    ) -> Result<Self, CompileError> {
        if declarations.runtime_parameters.is_empty() && declarations.internal_producers.is_empty()
        {
            return Ok(Self::default());
        }
        // A deliberately narrow influence proof: no control, feedback or script path
        // is permitted beside these first consumers. Audio dependencies below use the
        // last preceding writer, rather than equating reused buffer regions with nodes.
        if !plan.prepared_scripts().is_empty()
            || plan
                .prepared_nodes()
                .iter()
                .any(|node| matches!(node, PreparedNode::Controller { .. }))
            || plan
                .ops()
                .iter()
                .any(|op| matches!(op, PlanOp::Modulate(_) | PlanOp::FeedbackWrite { .. }))
        {
            return Err(refuse(
                "first consumers refuse modulation, feedback and scripts",
            ));
        }
        let mut authored = Vec::new();
        let mut declarations_authored = declarations.runtime_parameters.clone();
        declarations_authored.sort_by_key(RuntimeParameterProgram::id);
        if declarations_authored
            .windows(2)
            .any(|pair| pair[0].id() == pair[1].id())
        {
            return Err(refuse("duplicate runtime program identity"));
        }
        let mut claimed = Vec::new();
        for declaration in declarations_authored {
            let mut slots = Vec::new();
            let mut targets = Vec::new();
            for (index, entry) in declaration.entries().iter().enumerate() {
                let slot = plan
                    .resolve_parameter(entry.node(), entry.parameter())
                    .ok_or_else(|| refuse("runtime target does not resolve"))?;
                let target = plan
                    .parameter_targets()
                    .get(slot.index())
                    .ok_or_else(|| refuse("runtime row missing"))?;
                if target.instances.get() != 1
                    || target.control != TRIM_LEVEL
                    || !matches!(
                        plan.prepared_for_node(target.node),
                        Some(PreparedNode::Trim { .. })
                    )
                {
                    return Err(refuse("runtime target must be a global native Trim level"));
                }
                if !plan.authored_steps().iter().any(|(_, scope, node)| {
                    *scope == ExecutionScope::Global && *node == target.node
                }) {
                    return Err(refuse("runtime target must have Global execution scope"));
                }
                prove_downstream(plan, target.node)?;
                // This first consumer admits only nonnegative Trim amplitudes.
                // Amplitude itself also permits negative values (signal inversion).
                if entry.value().as_f32() < 0.0 {
                    return Err(refuse("runtime Trim amplitude is negative"));
                }
                if !slots.contains(&slot) {
                    if claimed.contains(&slot) {
                        return Err(refuse("two runtime programs claim one target"));
                    }
                    targets.push(index);
                }
                slots.push(slot);
            }
            targets.sort_by_key(|index| {
                let entry = declaration.entries()[*index];
                (entry.node(), entry.parameter())
            });
            for index in &targets {
                claimed.push(slots[*index]);
            }
            authored.push(ParameterProgram {
                declaration,
                slots: slots.into_boxed_slice(),
                targets: targets.into_boxed_slice(),
            });
        }
        let mut declarations_internal = declarations.internal_producers.clone();
        declarations_internal.sort_by_key(|declaration| declaration.node());
        if declarations_internal
            .windows(2)
            .any(|pair| pair[0].node() == pair[1].node())
        {
            return Err(refuse("duplicate internal Delay emitter"));
        }
        let mut internal = Vec::new();
        let mut internal_bound = 0_u32;
        let mut internal_clear_work = 0_usize;
        for declaration in declarations_internal {
            if !plan
                .authored_steps()
                .iter()
                .any(|(node, _, _)| *node == declaration.node())
            {
                return Err(refuse("internal target node does not exist"));
            }
            let mut matches = plan.authored_steps().iter().filter(|(node, scope, _)| {
                *node == declaration.node() && *scope == ExecutionScope::Global
            });
            let (_, _, node) = matches
                .next()
                .ok_or_else(|| refuse("internal target is not global"))?;
            if matches.next().is_some()
                || !matches!(
                    plan.prepared_for_node(*node),
                    Some(PreparedNode::Delay { .. })
                )
            {
                return Err(refuse("internal target must be one global native Delay"));
            }
            prove_downstream(plan, *node)?;
            internal_bound = internal_bound
                .checked_add(declaration.per_quantum().get())
                .ok_or_else(|| refuse("internal bound overflow"))?;
            let Some(PreparedNode::Delay { line, .. }) = plan.prepared_for_node(*node) else {
                return Err(refuse("internal target must be a native Delay"));
            };
            let writes = line
                .checked_mul(2)
                .and_then(|samples| {
                    samples.checked_mul(usize::try_from(declaration.per_quantum().get()).ok()?)
                })
                .ok_or_else(|| refuse("internal clear work overflow"))?;
            internal_clear_work = internal_clear_work
                .checked_add(writes)
                .ok_or_else(|| refuse("internal clear work overflow"))?;
            internal.push(InternalProgram {
                declaration,
                node: *node,
            });
        }
        Ok(Self {
            authored: authored.into_boxed_slice(),
            internal: internal.into_boxed_slice(),
            internal_bound: EventCount::measured(internal_bound),
            internal_clear_work: synth_core::SampleCount::new(internal_clear_work),
        })
    }
    #[cfg(feature = "simulated-ingress")]
    pub(crate) fn same_program(&self, other: &Self) -> bool {
        self.authored.len() == other.authored.len()
            && self.authored.iter().zip(&other.authored).all(|(a, b)| {
                a.declaration == b.declaration
                    && a.slots
                        .iter()
                        .map(|slot| slot.index())
                        .eq(b.slots.iter().map(|slot| slot.index()))
            })
            && self.internal == other.internal
    }
    #[cfg(feature = "simulated-ingress")]
    pub(crate) fn writes(&self, slot: ParameterSlot) -> bool {
        self.authored
            .iter()
            .any(|program| program.slots.contains(&slot))
    }
    #[cfg(feature = "simulated-ingress")]
    pub(crate) fn heap_bytes(&self) -> u64 {
        let authored = self
            .authored
            .len()
            .saturating_mul(size_of::<ParameterProgram>());
        let internal = self
            .internal
            .len()
            .saturating_mul(size_of::<InternalProgram>());
        self.authored.iter().fold(
            authored.saturating_add(internal) as u64,
            |bytes, program| {
                bytes
                    .saturating_add(
                        program
                            .declaration
                            .entries()
                            .len()
                            .saturating_mul(size_of::<crate::ir::RuntimeParameterEntry>())
                            as u64,
                    )
                    .saturating_add(
                        program
                            .slots
                            .len()
                            .saturating_mul(size_of::<ParameterSlot>())
                            as u64,
                    )
                    .saturating_add(program.targets.len().saturating_mul(size_of::<usize>()) as u64)
            },
        )
    }
}
fn prove_downstream(plan: &CompiledPlan, target: NodeSlot) -> Result<(), CompileError> {
    let mut affected = Vec::new();
    let mut writers = Vec::new();
    let mut found = false;
    for op in plan.ops() {
        if let PlanOp::Node(step) = op {
            let depends = step.node() == target
                || step.inputs().iter().flatten().any(|input| {
                    writers
                        .iter()
                        .rev()
                        .find(|(buffer, _)| buffer == input)
                        .is_some_and(|(_, node)| affected.contains(node))
                });
            if depends {
                if step.role() != NodeRole::Global {
                    return Err(refuse("producer can influence a voice or shared sum"));
                }
                affected.push(step.node());
            }
            if step.node() == target {
                found = true;
            }
            writers.push((step.out(), step.node()));
        }
    }
    if !found {
        return Err(refuse("producer target missing from schedule"));
    }
    Ok(())
}
/// Conservative exact-size cold allocation charge, independent of allocator capacity.
pub(crate) fn declaration_bytes(declarations: &PlanDeclarations) -> u64 {
    declarations
        .runtime_parameters
        .iter()
        .fold(0_u64, |bytes, program| {
            bytes
                .saturating_add(size_of::<ParameterProgram>() as u64)
                .saturating_add((program.entries().len() as u64).saturating_mul(
                    (size_of::<crate::ir::RuntimeParameterEntry>()
                        + size_of::<ParameterSlot>()
                        + size_of::<usize>()) as u64,
                ))
        })
        .saturating_add(
            (declarations.internal_producers.len() as u64)
                .saturating_mul(size_of::<InternalProgram>() as u64),
        )
}

impl Default for ProducerPrograms {
    fn default() -> Self {
        Self {
            authored: Box::default(),
            internal: Box::default(),
            internal_bound: EventCount::NONE,
            internal_clear_work: synth_core::SampleCount::ZERO,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub(crate) struct InternalEvent {
    pub(crate) node: NodeSlot,
    pub(crate) offset: crate::time::QuantumOffset,
}
#[derive(Debug)]
pub(crate) struct ProducerRuntime {
    pub(crate) initialized: bool,
    pub(crate) authored_observed: bool,
    pub(crate) authored_counts: Box<[EventCount]>,
    #[cfg(test)]
    pub(crate) authored_limit: Option<EventCount>,
    pub(crate) arena: Box<[Option<InternalEvent>]>,
    pub(crate) length: usize,
    pub(crate) observed: bool,
    pub(crate) last_time: Option<crate::time::SampleTime>,
    pub(crate) internal_peak: EventCount,
    pub(crate) combined_peak: EventCount,
    pub(crate) last_internal: EventCount,
    pub(crate) external: EventCount,
    #[cfg(test)]
    pub(crate) suppress_authored: bool,
    #[cfg(test)]
    pub(crate) suppress_internal: bool,
    #[cfg(test)]
    pub(crate) internal_limit: Option<EventCount>,
    #[cfg(test)]
    pub(crate) combined_limit: Option<EventCount>,
}
impl ProducerRuntime {
    pub(crate) fn prepare(programs: &ProducerPrograms) -> Self {
        Self {
            initialized: false,
            authored_observed: false,
            authored_counts: vec![EventCount::NONE; programs.authored.len()].into_boxed_slice(),
            #[cfg(test)]
            authored_limit: None,
            arena: vec![None; programs.internal_bound.get() as usize].into_boxed_slice(),
            length: 0,
            observed: false,
            last_time: None,
            internal_peak: EventCount::NONE,
            combined_peak: EventCount::NONE,
            last_internal: EventCount::NONE,
            external: EventCount::NONE,
            #[cfg(test)]
            suppress_authored: false,
            #[cfg(test)]
            suppress_internal: false,
            #[cfg(test)]
            internal_limit: None,
            #[cfg(test)]
            combined_limit: None,
        }
    }
}
pub(crate) fn internal_scratch_bytes(bound: EventCount) -> u64 {
    u64::from(bound.get()).saturating_mul(
        (size_of::<Option<InternalEvent>>() + size_of::<crate::node::kernels::TimedControl>())
            as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        compile::{RenderConfig, compile},
        ir::{
            ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PortId,
            RuntimeParameterEntry, RuntimeProgramId, SignalDomain, parameters,
        },
        profile::HostProfile,
        quantities::{
            Amplitude, ChannelLayout, Frequency, HeldNoteCount, NormalizedLevel, ParameterValue,
            SampleRate, Seconds,
        },
        time::FrameCount,
    };
    #[test]
    fn a_global_trim_that_feeds_a_voice_is_not_a_downstream_target() {
        let profile = HostProfile::harness(
            SampleRate::new(48_000.0).unwrap(),
            FrameCount::new(256),
            ChannelLayout::Mono,
        )
        .unwrap();
        let source = NodeId::new(1);
        let trim = NodeId::new(2);
        let gate = NodeId::new(3);
        let amp = NodeId::new(4);
        let output = NodeId::new(5);
        let fold = NodeId::new(6);
        let program = RuntimeParameterProgram::new(
            RuntimeProgramId::new(1),
            FrameCount::new(128),
            vec![RuntimeParameterEntry::new(
                trim,
                parameters::TRIM_LEVEL,
                FrameCount::new(0),
                ParameterValue::ONE,
            )],
        )
        .unwrap();
        let graph = |programs| {
            GraphIr::builder()
                .node(
                    source,
                    IrNodeKind::Sine {
                        frequency: Frequency::new(317.0).unwrap(),
                        amplitude: Amplitude::UNITY,
                    },
                    ExecutionScope::Global,
                )
                .node(
                    trim,
                    IrNodeKind::Trim {
                        level: Amplitude::UNITY,
                    },
                    ExecutionScope::Global,
                )
                .node(
                    gate,
                    IrNodeKind::Envelope {
                        attack: Seconds::ZERO,
                        decay: Seconds::ZERO,
                        sustain: NormalizedLevel::FULL,
                        release: Seconds::ZERO,
                        velocity_sensitivity: NormalizedLevel::ZERO,
                    },
                    ExecutionScope::Voice,
                )
                .node(amp, IrNodeKind::Amplifier, ExecutionScope::Voice)
                .node(output, IrNodeKind::Output, ExecutionScope::Global)
                .node(fold, IrNodeKind::StereoToMono, ExecutionScope::Global)
                .connect(
                    (source, PortId::FIRST),
                    (trim, PortId::FIRST),
                    SignalDomain::Audio,
                )
                .connect(
                    (trim, PortId::FIRST),
                    (fold, PortId::FIRST),
                    SignalDomain::Audio,
                )
                .connect(
                    (fold, PortId::FIRST),
                    (amp, PortId::FIRST),
                    SignalDomain::Audio,
                )
                .connect(
                    (gate, PortId::FIRST),
                    (amp, crate::node::AMPLIFIER_CONTROL),
                    SignalDomain::Control,
                )
                .connect(
                    (amp, PortId::FIRST),
                    (output, PortId::FIRST),
                    SignalDomain::Audio,
                )
                .declaring(PlanDeclarations {
                    runtime_parameters: programs,
                    note_producers: vec![
                        NoteProducerDeclaration {
                            compiled: true,
                            simultaneous_notes: HeldNoteCount::measured(2),
                            simultaneous_holds: EventCount::NONE,
                        },
                        NoteProducerDeclaration {
                            compiled: false,
                            simultaneous_notes: HeldNoteCount::measured(2),
                            simultaneous_holds: EventCount::measured(2),
                        },
                    ],
                    held_notes: HeldNoteCount::measured(4),
                    ..PlanDeclarations::default()
                })
                .build()
                .unwrap()
        };
        let _baseline = compile(&graph(vec![]), &RenderConfig::new(profile))
            .into_plan()
            .unwrap();
        assert!(matches!(
            compile(&graph(vec![program]), &RenderConfig::new(profile)).into_plan(),
            Err(CompileError::ProducerProgram {
                reason: "producer can influence a voice or shared sum"
            })
        ));
    }
}
