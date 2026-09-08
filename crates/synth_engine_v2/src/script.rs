//! Off-thread script identity and stable local parameter keys (ADR-0008).

use crate::ir::{NodeId, ParameterId};
use std::collections::BTreeMap;
use thiserror::Error;

/// Authored project seed; no implicit seed is selected by a compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct ProjectSeed(u64);
impl ProjectSeed {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Identity of one script's state, independent of its source or compiled layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct ScriptStateId(u64);
impl ScriptStateId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// One authored script node's parameter namespace. Removed keys retain their identity;
/// a different spelling receives a different ID, even after all old keys are removed.
/// Keep this object with the authored node across source edits. It is not persisted by V2.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct ScriptIdentity {
    node: NodeId,
    state: ScriptStateId,
    project_seed: ProjectSeed,
    parameters: BTreeMap<String, ParameterId>,
    next_parameter: Option<ParameterId>,
}
impl ScriptIdentity {
    pub fn new(node: NodeId, state: ScriptStateId, project_seed: ProjectSeed) -> Self {
        Self {
            node,
            state,
            project_seed,
            parameters: BTreeMap::new(),
            next_parameter: Some(ParameterId::FIRST),
        }
    }
    pub const fn node(&self) -> NodeId {
        self.node
    }
    pub const fn state(&self) -> ScriptStateId {
        self.state
    }
    pub const fn project_seed(&self) -> ProjectSeed {
        self.project_seed
    }
    /// Called only on a staged clone while compiling a program. Failed compilation must
    /// not change the active identity or consume a new key.
    fn parameter(&mut self, name: &str) -> Result<ParameterId, ScriptIdentityError> {
        if let Some(id) = self.parameters.get(name) {
            return Ok(*id);
        }
        let id = self
            .next_parameter
            .ok_or(ScriptIdentityError::ParametersExhausted { node: self.node })?;
        self.next_parameter = id.as_raw().checked_add(1).map(ParameterId::new);
        self.parameters.insert(name.to_owned(), id);
        Ok(id)
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum ScriptIdentityError {
    #[error("{node}'s script parameter identities are exhausted; removed keys are never reused")]
    ParametersExhausted { node: NodeId },
}

/// Invalid authored scalar metadata; rejected before installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScriptValueError {
    #[error("script parameter bounds must be finite and ordered")]
    Range,
    #[error("script parameter default is outside its declared range")]
    DefaultOutsideRange,
    #[error("script default must be finite")]
    NonFiniteDefault,
}

/// A script knob's finite authored domain. Bits retain exact bounds without float keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct ScriptRange {
    minimum: u32,
    maximum: u32,
}
impl ScriptRange {
    pub fn new(minimum: f32, maximum: f32) -> Result<Self, ScriptValueError> {
        if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
            return Err(ScriptValueError::Range);
        }
        Ok(Self {
            minimum: minimum.to_bits(),
            maximum: maximum.to_bits(),
        })
    }
    pub const fn minimum(self) -> f32 {
        f32::from_bits(self.minimum)
    }
    pub const fn maximum(self) -> f32 {
        f32::from_bits(self.maximum)
    }
}

/// A scalar knob default paired with the domain that validated it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct ScriptDefault {
    value: crate::quantities::ParameterValue,
    range: ScriptRange,
}
impl ScriptDefault {
    pub fn new(
        value: crate::quantities::ParameterValue,
        range: ScriptRange,
    ) -> Result<Self, ScriptValueError> {
        if value.as_f32() < range.minimum() || value.as_f32() > range.maximum() {
            return Err(ScriptValueError::DefaultOutsideRange);
        }
        Ok(Self { value, range })
    }
    pub const fn value(self) -> crate::quantities::ParameterValue {
        self.value
    }
    pub const fn range(self) -> ScriptRange {
        self.range
    }
}

/// The scalar layer explicitly selected by an external parameter read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterRead {
    Base,
    Automated,
    PreviousResolved,
}

/// Authored binding resolved to numeric buffers or parameter slots at graph compilation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScriptSource {
    Signal {
        node: NodeId,
        port: crate::ir::PortId,
    },
    Parameter {
        node: NodeId,
        parameter: ParameterId,
        read: ParameterRead,
    },
    Constant(crate::quantities::ParameterValue),
}

/// One compiler source name and its explicit meaning. No dangling name reads zero.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptBinding {
    pub input: synth_script::compile::SourceInput,
    pub source: ScriptSource,
}

/// A local knob's stable address and authored discovery metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptParameter {
    pub id: ParameterId,
    pub declaration: synth_core::script::ScriptParamDecl,
    pub default: ScriptDefault,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ProgramInput {
    Local(crate::node::kernels::ControlIndex),
    External(ScriptSource),
    SampleRate,
    ControlRate,
}

/// An immutable control program compiled off-thread, including its bound interface.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct ScriptProgram {
    pub(crate) identity: ScriptIdentity,
    pub(crate) code: synth_core::script::CompiledScript,
    pub(crate) inputs: Vec<ProgramInput>,
    pub(crate) spans: Vec<synth_script::span::Span>,
    parameters: Vec<ScriptParameter>,
    source: String,
    rate: crate::quantities::SampleRate,
    work: crate::ir::IrProgram,
    warnings: Vec<synth_script::diag::Diagnostic>,
    /// Named state cells followed by opcode state-address uses: exact layout, not a hash.
    layout: Vec<String>,
}

impl ScriptIdentity {
    /// Compile a source edit transactionally. Failure leaves this namespace untouched.
    pub fn compile_control(
        &mut self,
        source: &str,
        rate: crate::quantities::SampleRate,
        bindings: &[ScriptBinding],
    ) -> Result<ScriptProgram, Vec<synth_script::diag::Diagnostic>> {
        use synth_script::{
            compile::{CompileOptions, SourceInput},
            diag::Diagnostic,
            span::Span,
        };
        if source.len() > synth_core::script::MAX_SOURCE_LEN {
            return Err(vec![Diagnostic::error(
                Span::new(0, 0),
                "script source exceeds the compiler byte limit",
            )]);
        }
        let options = CompileOptions {
            control_rate: rate.as_f32() / f32::from(crate::time::QUANTUM_FRAMES as u16),
            control_ports: true,
            ..CompileOptions::default()
        };
        let (compiled, mut diagnostics) = synth_script::compile::compile(source, &options);
        let Some(compiled) = compiled else {
            return Err(diagnostics);
        };
        if compiled
            .script
            .code()
            .iter()
            .any(|op| matches!(op, synth_core::script::Op::StoreOut(slot) if *slot > 0))
        {
            diagnostics.push(Diagnostic::error(
                Span::new(0, 0),
                "V2 Control currently declares one output; out2..out4 require a multi-output node",
            ));
        }
        let (ast, _) = synth_script::parser::parse(source);
        let mut staged = self.clone();
        let mut parameters = Vec::new();
        for declaration in &compiled.params {
            let span = ast
                .params
                .iter()
                .find(|p| p.name.name == declaration.name_str)
                .map_or(Span::new(0, 0), |p| p.span);
            let id = match staged.parameter(declaration.name_str) {
                Ok(id) => id,
                Err(error) => {
                    diagnostics.push(Diagnostic::error(span, error.to_string()));
                    continue;
                }
            };
            let range = ScriptRange::new(declaration.min, declaration.max);
            let value = crate::quantities::ParameterValue::new(declaration.default)
                .map_err(|_| ScriptValueError::NonFiniteDefault);
            match range.and_then(|range| value.and_then(|value| ScriptDefault::new(value, range))) {
                Ok(default) => parameters.push(ScriptParameter {
                    id,
                    declaration: declaration.clone(),
                    default,
                }),
                Err(error) => diagnostics.push(Diagnostic::error(span, error.to_string())),
            }
        }
        for (index, binding) in bindings.iter().enumerate() {
            if bindings[..index]
                .iter()
                .any(|earlier| earlier.input == binding.input)
            {
                diagnostics.push(Diagnostic::error(
                    Span::new(0, 0),
                    format!("duplicate source binding {:?}", binding.input),
                ));
            }
            if !compiled.inputs.contains(&binding.input)
                || matches!(
                    binding.input,
                    SourceInput::LocalParam(_)
                        | SourceInput::Context(
                            synth_script::symbols::Context::Sr | synth_script::symbols::Context::Cr
                        )
                )
            {
                diagnostics.push(Diagnostic::error(
                    Span::new(0, 0),
                    format!(
                        "binding does not name an external source: {:?}",
                        binding.input
                    ),
                ));
            }
        }
        let mut inputs = Vec::new();
        let mut spans = Vec::new();
        for input in &compiled.inputs {
            let span = source_span(&ast, input);
            let bound = match input {
                SourceInput::LocalParam(name) => parameters
                    .iter()
                    .position(|p| p.declaration.name == *name)
                    .and_then(|index| u8::try_from(index).ok())
                    .map(|index| {
                        ProgramInput::Local(crate::node::kernels::ControlIndex::new(index))
                    }),
                SourceInput::Context(synth_script::symbols::Context::Sr) => {
                    Some(ProgramInput::SampleRate)
                }
                SourceInput::Context(synth_script::symbols::Context::Cr) => {
                    Some(ProgramInput::ControlRate)
                }
                _ => bindings
                    .iter()
                    .find(|b| &b.input == input)
                    .map(|b| ProgramInput::External(b.source)),
            };
            if let Some(bound) = bound {
                inputs.push(bound);
                spans.push(span);
            } else {
                diagnostics.push(Diagnostic::error(
                    span,
                    format!("unbound script source {input:?}"),
                ));
            }
        }
        if diagnostics.iter().any(Diagnostic::is_error) {
            return Err(diagnostics);
        }
        let work = program_work(self.node, &compiled.script, &ast);
        let layout = ast
            .states
            .iter()
            .map(|state| state.name.name.clone())
            .chain(compiled.script.code().iter().filter_map(state_layout_entry))
            .collect();
        let program = ScriptProgram {
            identity: staged.clone(),
            code: compiled.script,
            inputs,
            spans,
            parameters,
            source: source.to_owned(),
            rate,
            work,
            warnings: diagnostics,
            layout,
        };
        *self = staged;
        Ok(program)
    }
}

impl ScriptProgram {
    pub(crate) fn prepared_bytes(&self) -> u64 {
        (size_of::<PreparedScript>() as u64)
            .saturating_add(
                (self.code.code().len() as u64)
                    .saturating_mul(size_of::<synth_core::script::Op>() as u64),
            )
            .saturating_add(
                (self.code.constants().len() as u64).saturating_mul(size_of::<f32>() as u64),
            )
            .saturating_add(
                (self.inputs.len() as u64).saturating_mul(size_of::<PreparedInput>() as u64),
            )
    }

    pub const fn node(&self) -> NodeId {
        self.identity.node()
    }
    pub fn parameters(&self) -> &[ScriptParameter] {
        &self.parameters
    }
    pub fn warnings(&self) -> &[synth_script::diag::Diagnostic] {
        &self.warnings
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    pub const fn rate(&self) -> crate::quantities::SampleRate {
        self.rate
    }
    pub fn state_layout(&self) -> &[String] {
        &self.layout
    }
    pub const fn work(&self) -> crate::ir::IrProgram {
        self.work
    }
}

fn source_span(
    ast: &synth_script::ast::Program,
    input: &synth_script::compile::SourceInput,
) -> synth_script::span::Span {
    use synth_script::compile::SourceInput;
    if let SourceInput::Module {
        module,
        instance,
        member,
    } = input
    {
        return ast
            .bindings
            .iter()
            .find(|binding| {
                binding.address.module == *module
                    && binding.address.instance.unwrap_or(1) == *instance
                    && binding.address.member == *member
            })
            .map_or(synth_script::span::Span::new(0, 0), |binding| {
                binding.address.span
            });
    }
    synth_script::span::Span::new(0, 0)
}

fn state_layout_entry(op: &synth_core::script::Op) -> Option<String> {
    use synth_core::script::Op;
    match op {
        Op::Lag(_)
        | Op::Slew(_)
        | Op::Sah(_)
        | Op::Accum(_)
        | Op::AccumReset(_)
        | Op::Delta(_)
        | Op::Phasor(_)
        | Op::PhasorSync(_)
        | Op::Edge(_)
        | Op::Counter(_)
        | Op::RandSmooth(_)
        | Op::LoadState(_)
        | Op::StoreState(_) => Some(format!("{op:?}")),
        _ => None,
    }
}

fn program_work(
    node: NodeId,
    code: &synth_core::script::CompiledScript,
    ast: &synth_script::ast::Program,
) -> crate::ir::IrProgram {
    use crate::quantities::{InstructionCount, SlotCount};
    use synth_core::script::Op;
    let mut depth = 0usize;
    let mut maximum = 0usize;
    for op in code.code() {
        let (pops, pushes) = match op {
            Op::PushConst(_) | Op::PushSource(_) | Op::LoadLocal(_) | Op::LoadState(_) => (0, 1),
            Op::StoreLocal(_) | Op::StoreState(_) | Op::StoreOut(_) => (1, 0),
            Op::Call(function) => (function.arity(), 1),
            Op::Select | Op::Slew(_) => (3, 1),
            Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Rem
            | Op::Pow
            | Op::Eq
            | Op::Ne
            | Op::Lt
            | Op::Gt
            | Op::Le
            | Op::Ge
            | Op::And
            | Op::Or
            | Op::Lag(_)
            | Op::Sah(_)
            | Op::AccumReset(_)
            | Op::PhasorSync(_)
            | Op::Rand => (2, 1),
            Op::IndexConst { .. }
            | Op::Neg
            | Op::Not
            | Op::Accum(_)
            | Op::Delta(_)
            | Op::Phasor(_)
            | Op::Edge(_)
            | Op::Counter(_)
            | Op::RandSmooth(_)
            | Op::TableLin { .. }
            | Op::ScaleSnap { .. } => (1, 1),
        };
        depth = depth.saturating_sub(pops).saturating_add(pushes);
        maximum = maximum.max(depth);
    }
    let count = |value: usize| SlotCount::measured(u32::try_from(value).unwrap_or(u32::MAX));
    let locals = code
        .code()
        .iter()
        .filter_map(|op| match op {
            Op::LoadLocal(i) | Op::StoreLocal(i) => Some(usize::from(*i) + 1),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    crate::ir::IrProgram::new(
        crate::ir::ProgramId::new(node.as_raw()),
        InstructionCount::measured(u32::try_from(code.code().len()).unwrap_or(u32::MAX)),
        SlotCount::measured(u32::from(code.source_count())),
        SlotCount::measured(u32::from(code.state_count())),
        count(locals),
        count(maximum),
        count(ast.arrays.len()),
        count(ast.arrays.iter().map(|a| a.elements.len()).sum()),
        SlotCount::NONE,
        1,
    )
}

/// A resource position inside one IR, distinct from a program's stable node identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct ScriptRef(usize);
impl ScriptRef {
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }
    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

/// A prepared program's position inside its owning plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ScriptSlot(usize);
impl ScriptSlot {
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }
    pub(crate) const fn index(self) -> usize {
        self.0
    }
}

/// A stable runtime voice within its node's declared instance group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ScriptVoiceId(u32);
impl ScriptVoiceId {
    pub(crate) const ZERO: Self = Self(0);
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }
}

/// Seed after mixing the authored identities; the VM never sees plan positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ScriptSeed(u64);
impl ScriptSeed {
    pub(crate) const fn as_u64(self) -> u64 {
        self.0
    }
    pub(crate) fn for_voice(self, voice: ScriptVoiceId) -> Self {
        Self(synth_core::hash::splitmix64(
            self.0 ^ u64::from(voice.0) ^ 0x564F_4943_4500_0001,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ScriptFault {
    #[error("script resource is missing")]
    Missing,
    #[error("script identity belongs to {actual}")]
    WrongNode { actual: NodeId },
    #[error("script was compiled for {compiled}, but the stream uses {stream}")]
    RateMismatch {
        compiled: crate::quantities::SampleRate,
        stream: crate::quantities::SampleRate,
    },
}

impl ScriptProgram {
    pub(crate) fn seed(&self) -> ScriptSeed {
        use synth_core::hash::splitmix64;
        let project = splitmix64(self.identity.project_seed.as_u64() ^ 0x5052_4F4A_4543_5401);
        let node = splitmix64(project ^ u64::from(self.node().as_raw()) ^ 0x4E4F_4445_0000_0001);
        ScriptSeed(splitmix64(
            node ^ self.identity.state.as_u64() ^ 0x5354_4154_4500_0001,
        ))
    }
    pub(crate) fn signals(&self) -> impl Iterator<Item = (NodeId, crate::ir::PortId)> + '_ {
        self.inputs.iter().filter_map(|input| match input {
            ProgramInput::External(ScriptSource::Signal { node, port }) => Some((*node, *port)),
            _ => None,
        })
    }
    pub(crate) fn work_for(&self, evaluations: u32) -> crate::ir::IrProgram {
        let w = self.work;
        crate::ir::IrProgram::new(
            w.id(),
            w.instructions(),
            w.sources(),
            w.state_slots(),
            w.locals(),
            w.eval_stack_depth(),
            w.arrays(),
            w.array_elements(),
            w.emits(),
            evaluations,
        )
    }
    pub(crate) fn descriptor(&self) -> crate::node::NodeDescriptor {
        use crate::{
            ir::{PortId, SignalDomain},
            node::{ControlSpec, ModulationLaw, ParameterDefault, Smoothing},
            quantities::ChannelLayout,
            validate::{PortDirection, PortSpec},
        };
        let mut ports = vec![PortSpec::new(
            PortId::FIRST,
            PortDirection::Output,
            SignalDomain::Control,
            ChannelLayout::Mono,
        )];
        for (index, _) in self.signals().enumerate() {
            ports.push(PortSpec::new(
                PortId::new(u16::try_from(index).unwrap_or(u16::MAX)),
                PortDirection::Input,
                SignalDomain::Control,
                ChannelLayout::Mono,
            ));
        }
        let controls = self
            .parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| ControlSpec {
                controller: false,
                parameter: parameter.id,
                name: parameter.declaration.name_str,
                default: ParameterDefault::ScriptScalar(parameter.default),
                law: ModulationLaw::PhysicalLinearAdditive,
                smoothing: Smoothing::None,
                control: crate::node::kernels::ControlIndex::new(
                    u8::try_from(index).unwrap_or(u8::MAX),
                ),
                rate: crate::plan::ControlRate::Quantum,
                magnitude: None,
            })
            .collect();
        crate::node::NodeDescriptor {
            kernel: crate::node::kernels::SCRIPT,
            ports,
            controls,
            in_place_safe: false,
            note_control: None,
        }
    }
}

pub(crate) mod hot;

/// Numeric source binding; names and graph lookup remain off the audio thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PreparedInput {
    Local(crate::node::kernels::ControlIndex),
    Signal(ScriptSignalIndex),
    Parameter {
        slot: crate::plan::ParameterSlot,
        read: ParameterRead,
        per_voice: bool,
    },
    Constant(crate::quantities::ParameterValue),
}

/// Immutable executable resource. Only the compiler can construct it.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedScript {
    code: synth_core::script::CompiledScript,
    inputs: Vec<PreparedInput>,
}

impl PreparedScript {
    #[cfg(test)]
    pub(crate) fn dynamic_bytes_held(&self) -> usize {
        self.inputs.capacity() * size_of::<PreparedInput>()
            + std::mem::size_of_val(self.code.code())
            + std::mem::size_of_val(self.code.constants())
    }
}

/// Explicit parameter layers sampled on the fixed quantum clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterLayers {
    pub(crate) base: crate::quantities::ParameterValue,
    pub(crate) automated: crate::quantities::ParameterValue,
    pub(crate) previous: crate::quantities::ParameterValue,
}

/// Resources borrowed from the admitted plan and its renderer.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScriptResources<'a> {
    pub programs: &'a [PreparedScript],
    pub parameters: &'a [ParameterLayers],
}

impl ScriptProgram {
    pub(crate) fn prepare(
        &self,
        ir: &crate::ir::GraphIr,
        plan: &crate::plan::CompiledPlan,
    ) -> Result<PreparedScript, crate::diagnostics::CompileError> {
        use crate::{diagnostics::CompileError, quantities::ParameterValue};
        let mut signal = 0_u8;
        let mut inputs = Vec::with_capacity(self.inputs.len());
        for (input, span) in self.inputs.iter().zip(&self.spans) {
            let fault = |reason| CompileError::ScriptBinding {
                node: self.node(),
                span: *span,
                reason,
            };
            inputs.push(match *input {
                ProgramInput::Local(index) => PreparedInput::Local(index),
                ProgramInput::SampleRate => PreparedInput::Constant(
                    ParameterValue::new(self.rate.as_f32())
                        .map_err(|_| fault(ScriptBindingFault::InvalidRate))?,
                ),
                ProgramInput::ControlRate => PreparedInput::Constant(
                    ParameterValue::new(
                        self.rate.as_f32() / f32::from(crate::time::QUANTUM_FRAMES as u16),
                    )
                    .map_err(|_| fault(ScriptBindingFault::InvalidRate))?,
                ),
                ProgramInput::External(ScriptSource::Constant(value)) => {
                    PreparedInput::Constant(value)
                }
                ProgramInput::External(ScriptSource::Signal { .. }) => {
                    let index = ScriptSignalIndex(signal);
                    signal = signal.saturating_add(1);
                    PreparedInput::Signal(index)
                }
                ProgramInput::External(ScriptSource::Parameter {
                    node,
                    parameter,
                    read,
                }) => {
                    let slot = plan.resolve_parameter(node, parameter).ok_or_else(|| {
                        fault(ScriptBindingFault::MissingParameter {
                            source_node: node,
                            parameter,
                        })
                    })?;
                    let per_voice = ir.scope_of(node) == Some(crate::ir::ExecutionScope::Voice);
                    if per_voice
                        && ir.scope_of(self.node()) != Some(crate::ir::ExecutionScope::Voice)
                    {
                        return Err(fault(ScriptBindingFault::Scope { source_node: node }));
                    }
                    PreparedInput::Parameter {
                        slot,
                        read,
                        per_voice,
                    }
                }
            });
        }
        Ok(PreparedScript {
            code: self.code.clone(),
            inputs,
        })
    }
}

/// Source-level binding refusal, retaining the authored span without runtime names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScriptBindingFault {
    #[error("source {source_node} is not declared")]
    MissingNode { source_node: NodeId },
    #[error("source {source_node}.{port} is not a mono control output")]
    Signal {
        source_node: NodeId,
        port: crate::ir::PortId,
    },
    #[error("source {source_node} closes an algebraic dependency cycle")]
    Cycle { source_node: NodeId },
    #[error("source {source_node}.{parameter} is not a declared parameter")]
    MissingParameter {
        source_node: NodeId,
        parameter: ParameterId,
    },
    #[error("source {source_node} cannot be read from this scope")]
    Scope { source_node: NodeId },
    #[error("the evaluation rate is invalid")]
    InvalidRate,
}

impl ScriptProgram {
    pub(crate) fn validate_bindings(
        &self,
        ir: &crate::ir::GraphIr,
        stream: crate::quantities::ChannelLayout,
    ) -> Result<(), crate::diagnostics::CompileError> {
        use crate::{ir::SignalDomain, validate::PortDirection};
        for (input, span) in self.inputs.iter().zip(&self.spans) {
            let ProgramInput::External(binding) = *input else {
                continue;
            };
            let fault = |reason| crate::diagnostics::CompileError::ScriptBinding {
                node: self.node(),
                span: *span,
                reason,
            };
            let source_node = match binding {
                ScriptSource::Signal { node, port } => {
                    let Some(source) = ir.node(node) else {
                        return Err(fault(ScriptBindingFault::MissingNode { source_node: node }));
                    };
                    let ports = ir.ports_of(source.kind(), stream);
                    if !ports.iter().any(|p| {
                        p.id() == port
                            && p.direction() == PortDirection::Output
                            && p.domain() == SignalDomain::Control
                            && p.layout() == crate::quantities::ChannelLayout::Mono
                    }) {
                        return Err(fault(ScriptBindingFault::Signal {
                            source_node: node,
                            port,
                        }));
                    }
                    node
                }
                ScriptSource::Parameter {
                    node, parameter, ..
                } => {
                    if !ir
                        .node(node)
                        .and_then(|node| ir.descriptor(node.kind()))
                        .is_some_and(|descriptor| {
                            descriptor.controls.iter().any(|control| {
                                control.parameter == parameter && control.law.admits_writes()
                            })
                        })
                    {
                        return Err(fault(ScriptBindingFault::MissingParameter {
                            source_node: node,
                            parameter,
                        }));
                    }
                    node
                }
                ScriptSource::Constant(_) => continue,
            };
            if let (Some(source), Some(target)) =
                (ir.scope_of(source_node), ir.scope_of(self.node()))
                && crate::validate::scope_depth(source) > crate::validate::scope_depth(target)
            {
                return Err(fault(ScriptBindingFault::Scope { source_node }));
            }
        }
        Ok(())
    }
}

pub(crate) fn cycle_diagnostic(
    ir: &crate::ir::GraphIr,
    cycle: &[NodeId],
) -> Option<crate::diagnostics::CompileError> {
    for (index, node) in cycle.iter().enumerate() {
        let Some(program) = ir.scripts().iter().find(|program| program.node() == *node) else {
            continue;
        };
        let source_node = *cycle.get(
            index
                .checked_sub(1)
                .unwrap_or(cycle.len().saturating_sub(1)),
        )?;
        let Some(span) = program.inputs.iter().zip(&program.spans).find_map(|(input, span)| {
            matches!(input, ProgramInput::External(ScriptSource::Signal { node, .. }) if *node == source_node).then_some(*span)
        }) else { continue; };
        return Some(crate::diagnostics::CompileError::ScriptBinding {
            node: *node,
            span,
            reason: ScriptBindingFault::Cycle { source_node },
        });
    }
    None
}

/// Position in the program's declared signal-input interface, distinct from a knob index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) struct ScriptSignalIndex(u8);
impl ScriptSignalIndex {
    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }
}
