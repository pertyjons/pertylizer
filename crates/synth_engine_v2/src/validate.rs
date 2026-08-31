//! Graph validation: what a plan must satisfy before anything is lowered.
//!
//! Phase 1 checked what its lowering happened to read — one output node, the first
//! output port, one edge into it — and everything else compiled. Phase 2 makes the
//! rules explicit and checks them against a **declared port table**, so that a plan
//! is refused where it is wrong rather than rendered silent where it is unreadable.
//!
//! The master plan's Phase 2 work list names the rules: node and port existence,
//! direction and signal-domain compatibility, channel-layout compatibility, a
//! one-source fan-in policy, cycle detection, and required-output validation.
//!
//! # What a diagnostic must carry
//!
//! Each failure class names what a reader can act on, and they are not the same
//! fields. A cable error names the edge, both endpoints, and both mismatched values;
//! a fan-in violation names the destination port and the competing edges; a cycle
//! names the edge that closes it and the node it re-enters; a missing output names
//! the plan and how many sources it stranded. `HOST-INV-007`'s shape — a field, two
//! amounts, an authored object — belongs to *resource* refusals and is not this.
//!
//! # Where it runs
//!
//! **Before every admission check and before lowering.** A preflight report is built
//! ahead of it, because a refusal has to carry one, but nothing is *decided* from that
//! report until this pass has run. P02-T004 moved validation there:
//! the arena's size is a function of the *assignment*, so a report built before
//! lowering could only state an upper bound, and an upper bound refuses plans that fit.
//! A plan refused here still carries a report — `HOST-INV-006` admits no exception —
//! and it is the one case whose arena row is that upper bound, because no assignment
//! exists to describe.

use std::collections::HashMap;

use crate::diagnostics::{CompileError, CompileWarning};
use crate::ir::{EdgeId, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use crate::quantities::{ChannelLayout, EventCount, NodeCount};

/// Which way a signal crosses a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    /// The node consumes the signal.
    Input,
    /// The node produces the signal.
    Output,
}

impl std::fmt::Display for PortDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input => f.write_str("input"),
            Self::Output => f.write_str("output"),
        }
    }
}

/// One port a node kind declares.
///
/// The layout is a property of the **port**, never of the buffer it is assigned:
/// ADR-0002 clause 2 makes an arena buffer always mono, so a stereo port occupies
/// two of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PortSpec {
    id: PortId,
    direction: PortDirection,
    domain: SignalDomain,
    layout: ChannelLayout,
}

impl PortSpec {
    /// Declare a port.
    pub const fn new(
        id: PortId,
        direction: PortDirection,
        domain: SignalDomain,
        layout: ChannelLayout,
    ) -> Self {
        Self {
            id,
            direction,
            domain,
            layout,
        }
    }

    /// The port's identity within its node.
    pub const fn id(self) -> PortId {
        self.id
    }

    /// Which way the signal crosses it.
    pub const fn direction(self) -> PortDirection {
        self.direction
    }

    /// What kind of signal it carries.
    pub const fn domain(self) -> SignalDomain {
        self.domain
    }

    /// The channel layout it carries.
    pub const fn layout(self) -> ChannelLayout {
        self.layout
    }
}

/// What the compiler must insert on an edge for it to be legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conversion {
    /// One mono signal written to every channel of a wider port.
    ///
    /// ADR-0002 clause 6: the only implicit conversion this phase inserts, and it is
    /// a *declared* operation in the schedule rather than something lowering does
    /// quietly. The reverse — stereo into mono — is refused, because picking a
    /// down-mix law would make a product decision by accident.
    MonoToStereo,
}

impl std::fmt::Display for Conversion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MonoToStereo => f.write_str("mono-to-stereo duplication"),
        }
    }
}

/// One edge that needs a conversion, resolved at validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct EdgeConversion {
    /// The edge.
    pub edge: EdgeId,
    /// What it needs.
    pub conversion: Conversion,
}

/// A graph that passed validation, with what the next stage needs from the pass.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct Validated {
    order: Vec<NodeId>,
    conversions: Vec<EdgeConversion>,
    warnings: Vec<CompileWarning>,
}

impl Validated {
    /// The nodes in an execution order: every node appears after all of its sources.
    ///
    /// The order is a **function of node identity**, not of declaration position:
    /// nodes are visited in ascending identity, so reordering the node list cannot
    /// change the schedule. That is what keeps a render digest comparable across two
    /// compilations of the same plan, which ADR-0005 clause 3 requires of the arena
    /// assignment computed from it.
    pub fn order(&self) -> &[NodeId] {
        &self.order
    }

    /// The edges whose legality depends on an inserted conversion.
    pub fn conversions(&self) -> &[EdgeConversion] {
        &self.conversions
    }

    /// Findings that do not refuse the plan.
    pub fn warnings(&self) -> &[CompileWarning] {
        &self.warnings
    }
}

/// Everything the passes need, indexed once.
///
/// Built because the naive form is **quadratic**: scanning every edge for every node
/// is fine for the five-node fixtures in the tests and is not fine for a plan near
/// `max_nodes` — 16 384 nodes against 65 536 edges is a billion comparisons for a
/// graph the profile explicitly admits. Compilation happens off the audio thread, so
/// hashing and allocating here are allowed; taking a billion steps is not, because a
/// compiler nobody can wait for is a compiler nobody uses.
struct Index<'a> {
    ir: &'a GraphIr,
    /// Node identity to its position in [`Self::ports`] and the adjacency lists.
    position: HashMap<NodeId, usize>,
    /// The ports each node declares, by position.
    ports: Vec<Vec<PortSpec>>,
    /// Edge indices leaving each node, by position, in edge order.
    outgoing: Vec<Vec<usize>>,
}

impl<'a> Index<'a> {
    fn build(ir: &'a GraphIr, stream: ChannelLayout) -> Self {
        let mut position = HashMap::with_capacity(ir.nodes().len());
        let mut ports = Vec::with_capacity(ir.nodes().len());
        for (slot, node) in ir.nodes().iter().enumerate() {
            position.insert(node.id(), slot);
            ports.push(crate::node::ports(node.kind(), stream));
        }
        let mut outgoing = vec![Vec::new(); ir.nodes().len()];
        for (index, edge) in ir.edges().iter().enumerate() {
            if let Some(slot) = position.get(&edge.from().0)
                && let Some(list) = outgoing.get_mut(*slot)
            {
                list.push(index);
            }
        }
        // Successors in identity order, not in the order someone happened to write the
        // cables. Without this the schedule — and with it the buffer assignment and the
        // operation list — would change when two equivalent `connect` calls are
        // swapped, which is exactly the "position is not identity" rule this crate is
        // built on. Ties within one successor keep edge order, which is already
        // identity-ordered.
        for list in &mut outgoing {
            list.sort_by_key(|index| {
                ir.edges()
                    .get(*index)
                    .map_or((NodeId::FIRST, PortId::FIRST), |edge| edge.to())
            });
        }
        Self {
            ir,
            position,
            ports,
            outgoing,
        }
    }

    fn port(&self, node: NodeId, id: PortId, direction: PortDirection) -> Option<PortSpec> {
        self.specs(node)?
            .iter()
            .find(|spec| spec.id() == id && spec.direction() == direction)
            .copied()
    }

    fn declares_port(&self, node: NodeId, id: PortId) -> bool {
        self.specs(node)
            .is_some_and(|specs| specs.iter().any(|spec| spec.id() == id))
    }

    fn specs(&self, node: NodeId) -> Option<&[PortSpec]> {
        self.position
            .get(&node)
            .and_then(|slot| self.ports.get(*slot))
            .map(Vec::as_slice)
    }
}

/// Check `ir` against every Phase 2 rule.
pub(crate) fn validate(ir: &GraphIr, stream: ChannelLayout) -> Result<Validated, CompileError> {
    // The two per-producer declaration rules, before anything is summed. An aggregate built
    // from a declaration that contradicts itself would be checked against the profile and
    // could pass, so the contradiction has to be caught first — and the caller told which
    // producer it was, which a sum cannot say.
    let mut first_compiled: Option<usize> = None;
    for (index, producer) in ir.declarations().note_producers.iter().enumerate() {
        if producer.compiled {
            if let Some(first) = first_compiled {
                return Err(CompileError::SecondCompiledProducer {
                    first,
                    second: index,
                });
            }
            first_compiled = Some(index);
        }
        if producer.compiled && producer.simultaneous_holds > EventCount::NONE {
            return Err(CompileError::CompiledProducerDeclaresHold {
                index,
                holds: producer.simultaneous_holds,
            });
        }
        if u64::from(producer.simultaneous_holds.get())
            > u64::from(producer.simultaneous_notes.get())
        {
            return Err(CompileError::ProducerHoldsExceedNotes {
                index,
                holds: producer.simultaneous_holds,
                notes: producer.simultaneous_notes,
            });
        }
    }

    // ADR-0046 clause 5's authored sources, linked to clause 6's hold partition. A source's
    // envelopes are summed against the shares further down; these three rules are what make
    // that sum meaningful, and they run here for the same reason the producer rules above do
    // — a declaration that contradicts itself must not reach an aggregate that could pass.
    //
    // **The link is the point.** An earlier attempt at this type carried the envelopes and
    // no producer reference, and an independent review found the hole: nothing tied a source
    // to the `note_producers` entry holding its entitlement, so a source could declare holds
    // that no partition had granted and the compiler could not tell.
    let mut holds_by_producer: std::collections::BTreeMap<u16, u64> =
        std::collections::BTreeMap::new();
    for (index, source) in ir.declarations().authored_sources.iter().enumerate() {
        // A producer's position in the declaration *is* its `ProducerId`, which is the same
        // numbering the identity ranges and hold entitlements are carried in.
        let Some(producer) = ir
            .declarations()
            .note_producers
            .get(source.producer.as_u16() as usize)
        else {
            return Err(CompileError::AuthoredSourceProducerUnknown {
                index,
                producer: source.producer,
            });
        };
        if producer.compiled {
            return Err(CompileError::AuthoredSourceProducerCompiled {
                index,
                producer: source.producer,
            });
        }
        // **Accumulated, not compared one at a time.** Several authored sources may name the
        // same producer, and ADR-0046 clause 6 partitions entitlements per *producer*, not
        // per source: two sources that each fit could together spend more holds than the one
        // entitlement they share. This is the same rule the destination aggregate below
        // follows, and checking it per source instead was a hole in this function's first
        // draft — a plan with two three-hold sources against a four-hold producer passed.
        let spent = holds_by_producer
            .entry(source.producer.as_u16())
            .or_insert(0_u64);
        *spent = spent.saturating_add(u64::from(source.simultaneous_holds.get()));
        if *spent > u64::from(producer.simultaneous_holds.get()) {
            return Err(CompileError::AuthoredSourceHoldsAboveEntitlement {
                index,
                producer: source.producer,
                holds: crate::quantities::EventCount::measured(
                    u32::try_from(*spent).unwrap_or(u32::MAX),
                ),
                entitlement: producer.simultaneous_holds,
            });
        }
    }

    let index = Index::build(ir, stream);
    let mut conversions = Vec::new();

    for edge in ir.edges() {
        let (source_node, source_port) = edge.from();
        let (target_node, target_port) = edge.to();

        // Existence and direction, in that order: a port that exists in the other
        // direction is a different mistake from one that does not exist, and saying
        // so is the difference between "check your cable" and "check your node".
        let Some(source) = index.port(source_node, source_port, PortDirection::Output) else {
            return Err(if index.declares_port(source_node, source_port) {
                CompileError::PortDirection {
                    edge: edge.id(),
                    node: source_node,
                    port: source_port,
                    needed: PortDirection::Output,
                    other_node: target_node,
                    other_port: target_port,
                }
            } else {
                CompileError::UnknownPort {
                    edge: edge.id(),
                    node: source_node,
                    port: source_port,
                    needed: PortDirection::Output,
                }
            });
        };
        let Some(target) = index.port(target_node, target_port, PortDirection::Input) else {
            return Err(if index.declares_port(target_node, target_port) {
                CompileError::PortDirection {
                    edge: edge.id(),
                    node: target_node,
                    port: target_port,
                    needed: PortDirection::Input,
                    other_node: source_node,
                    other_port: source_port,
                }
            } else {
                CompileError::UnknownPort {
                    edge: edge.id(),
                    node: target_node,
                    port: target_port,
                    needed: PortDirection::Input,
                }
            });
        };

        // The edge declares a domain too, and all three have to agree. An edge that
        // says `Control` between two audio ports is a plan that means something the
        // compiler cannot deliver.
        if source.domain() != target.domain() || edge.domain() != source.domain() {
            return Err(CompileError::DomainMismatch {
                edge: edge.id(),
                source_node,
                source_port,
                source_domain: source.domain(),
                target_node,
                target_port,
                target_domain: target.domain(),
                declared: edge.domain(),
            });
        }

        match (source.layout(), target.layout()) {
            (a, b) if a == b => {}
            (ChannelLayout::Mono, ChannelLayout::Stereo) => conversions.push(EdgeConversion {
                edge: edge.id(),
                conversion: Conversion::MonoToStereo,
            }),
            (source_layout, target_layout) => {
                return Err(CompileError::LayoutMismatch {
                    edge: edge.id(),
                    source_node,
                    source_port,
                    source_layout,
                    target_node,
                    target_port,
                    target_layout,
                });
            }
        }
    }

    fan_in(ir)?;
    let order = topological_order(&index)?;
    let warnings = outputs(ir)?;

    Ok(Validated {
        order,
        conversions,
        warnings,
    })
}

/// One source per input, and the competing edges named when there are more.
///
/// One pass to tally, then one pass in edge order to report — so a plan with three
/// edges into one input produces one diagnostic, and which one it is does not depend
/// on hash iteration order.
fn fan_in(ir: &GraphIr) -> Result<(), CompileError> {
    let mut arrivals: HashMap<(NodeId, PortId), (u32, EdgeId, Option<EdgeId>)> =
        HashMap::with_capacity(ir.edges().len());
    for edge in ir.edges() {
        arrivals
            .entry(edge.to())
            .and_modify(|(count, _, second)| {
                *count = count.saturating_add(1);
                if second.is_none() {
                    *second = Some(edge.id());
                }
            })
            .or_insert((1, edge.id(), None));
    }

    for edge in ir.edges() {
        let Some((count, first, second)) = arrivals.get(&edge.to()).copied() else {
            continue;
        };
        if count > 1 {
            let (node, port) = edge.to();
            return Err(CompileError::UnsupportedFanIn {
                node,
                port,
                edges: count,
                first,
                second: second.unwrap_or(first),
            });
        }
    }
    Ok(())
}

/// Depth-first order, refusing the first cycle it closes.
///
/// Phase 2 refuses **every** cycle rather than scheduling one. ADR-0033 owns the
/// delay-boundary rule that would relax that, it is `Proposed`, and the diagnostic
/// below deliberately does not promise a rule that does not exist yet.
fn topological_order(index: &Index<'_>) -> Result<Vec<NodeId>, CompileError> {
    /// Where a node is in the walk.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mark {
        Unvisited,
        OnStack,
        Done,
    }

    let ir = index.ir;
    // Ascending identity, so the order is a function of what the nodes *are* rather
    // than of the order someone happened to declare them in.
    let mut roots: Vec<(NodeId, usize)> = index
        .position
        .iter()
        .map(|(id, slot)| (*id, *slot))
        .collect();
    roots.sort_unstable();

    let mut marks = vec![Mark::Unvisited; ir.nodes().len()];
    let mut order = Vec::with_capacity(ir.nodes().len());
    // An explicit stack rather than recursion: a deep chain must not decide how big a
    // plan the compiler accepts, and `max_nodes` is what bounds that.
    let mut stack: Vec<(usize, usize)> = Vec::new();

    for (_, root) in &roots {
        if marks.get(*root).copied().unwrap_or(Mark::Done) != Mark::Unvisited {
            continue;
        }
        if let Some(mark) = marks.get_mut(*root) {
            *mark = Mark::OnStack;
        }
        stack.push((*root, 0));

        while let Some((slot, cursor)) = stack.pop() {
            let next = index
                .outgoing
                .get(slot)
                .and_then(|edges| edges.get(cursor))
                .copied();
            match next {
                Some(edge_index) => {
                    stack.push((slot, cursor + 1));
                    let Some(edge) = ir.edges().get(edge_index) else {
                        continue;
                    };
                    let successor = edge.to().0;
                    let Some(successor_slot) = index.position.get(&successor).copied() else {
                        continue;
                    };
                    match marks.get(successor_slot).copied().unwrap_or(Mark::Done) {
                        Mark::OnStack => {
                            return Err(CompileError::Cycle {
                                edge: edge.id(),
                                node: successor,
                                nodes: NodeCount::measured(
                                    u32::try_from(stack.len()).unwrap_or(u32::MAX),
                                ),
                            });
                        }
                        Mark::Done => {}
                        Mark::Unvisited => {
                            if let Some(mark) = marks.get_mut(successor_slot) {
                                *mark = Mark::OnStack;
                            }
                            stack.push((successor_slot, 0));
                        }
                    }
                }
                None => {
                    if let Some(mark) = marks.get_mut(slot) {
                        *mark = Mark::Done;
                    }
                    if let Some(node) = ir.nodes().get(slot) {
                        order.push(node.id());
                    }
                }
            }
        }
    }

    // Sources first, which is the order a schedule runs in: the walk finishes a node
    // after everything it feeds.
    order.reverse();
    Ok(order)
}

/// Required-output validation, and the one finding that warns rather than refuses.
fn outputs(ir: &GraphIr) -> Result<Vec<CompileWarning>, CompileError> {
    let outputs: Vec<NodeId> = ir
        .nodes()
        .iter()
        .filter(|node| matches!(node.kind(), IrNodeKind::Output))
        .map(super::ir::IrNode::id)
        .collect();
    let sources = ir
        .nodes()
        .iter()
        .filter(|node| node.kind().is_source())
        .count();

    if outputs.len() > 1 {
        // Rendering the first and ignoring the rest would be a silent choice about
        // which output a plan has. What a second output *means* belongs to the phase
        // that has buses.
        return Err(CompileError::MultipleOutputs {
            outputs: NodeCount::measured(u32::try_from(outputs.len()).unwrap_or(u32::MAX)),
        });
    }

    let Some(output) = outputs.first().copied() else {
        if sources == 0 {
            // The empty plan is legal and renders silence — the Phase 1 exit gate
            // requires exactly that.
            return Ok(Vec::new());
        }
        return Err(CompileError::MissingOutput {
            sources: NodeCount::measured(u32::try_from(sources).unwrap_or(u32::MAX)),
        });
    };

    if !ir.edges().iter().any(|edge| edge.to().0 == output) {
        // A **warning**, not a refusal, and the distinction is deliberate. A plan
        // whose output has nothing patched into it renders silence, which is a
        // legitimate intermediate state for a patch under construction — and once
        // plans can be swapped live, refusing it would mean the engine rejects a plan
        // the moment a cable is unplugged. What it must not do is stay quiet about it,
        // and that holds whether or not the plan also has sources: an output-only plan
        // renders exactly the silence this warning exists to explain.
        return Ok(vec![CompileWarning::OutputNotReached { output }]);
    }
    Ok(Vec::new())
}
