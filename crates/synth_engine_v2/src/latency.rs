//! Signal-path latency and scheduled compensation (`SOUND-INV-035`).

use std::collections::HashMap;

use crate::diagnostics::CompileError;
use crate::ir::{EdgeId, GraphIr, IrEdge, IrNodeKind, NodeId, SignalDomain};
use crate::quantities::SampleRate;
use crate::time::FrameCount;

/// Whether signal cables entering a node are aligned to the latest input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[must_use]
pub enum CompensationPolicy {
    /// Align audio, control and gate cables at each consuming node. Parameter events keep processing time.
    #[default]
    Compensate,
    /// Preserve the authored delays and report the remaining skew.
    Decline,
}

/// The earliest and latest signal arrival at a node's output, including its own latency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct PathLatency {
    node: NodeId,
    earliest: FrameCount,
    latest: FrameCount,
}

impl PathLatency {
    pub const fn node(self) -> NodeId {
        self.node
    }
    pub const fn earliest(self) -> FrameCount {
        self.earliest
    }
    pub const fn latest(self) -> FrameCount {
        self.latest
    }
}

/// A signal cable's skew at its destination and the delay actually inserted there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct EdgeLatency {
    edge: EdgeId,
    skew: FrameCount,
    compensation: FrameCount,
}

impl EdgeLatency {
    pub const fn edge(self) -> EdgeId {
        self.edge
    }
    pub const fn skew(self) -> FrameCount {
        self.skew
    }
    pub const fn compensation(self) -> FrameCount {
        self.compensation
    }
}

/// Diagnostics in identity order. Path bounds include upstream compensation under `Compensate`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct PathLatencies {
    policy: CompensationPolicy,
    paths: Vec<PathLatency>,
    edges: Vec<EdgeLatency>,
    output: FrameCount,
    voice_groups: crate::quantities::RecordCount,
    compensation_records: crate::quantities::RecordCount,
    compensation_history: crate::quantities::PreparedBytes,
}

impl Default for PathLatencies {
    fn default() -> Self {
        Self {
            policy: CompensationPolicy::default(),
            paths: Vec::new(),
            edges: Vec::new(),
            output: FrameCount::ZERO,
            voice_groups: crate::quantities::RecordCount::measured(0),
            compensation_records: crate::quantities::RecordCount::measured(0),
            compensation_history: crate::quantities::PreparedBytes::measured(0),
        }
    }
}

impl PathLatencies {
    pub const fn policy(&self) -> CompensationPolicy {
        self.policy
    }
    pub fn paths(&self) -> &[PathLatency] {
        &self.paths
    }
    pub fn edges(&self) -> &[EdgeLatency] {
        &self.edges
    }
    /// The longest signal path reaching the output; disconnected nodes do not contribute.
    pub const fn output(&self) -> FrameCount {
        self.output
    }
    pub(crate) const fn voice_groups(&self) -> u32 {
        self.voice_groups.get()
    }
    pub(crate) const fn compensation_records(&self) -> crate::quantities::RecordCount {
        self.compensation_records
    }
    pub(crate) const fn compensation_history(&self) -> crate::quantities::PreparedBytes {
        self.compensation_history
    }
    pub(crate) fn prepared_bytes(&self) -> u64 {
        (self.paths.len() as u64)
            .saturating_mul(size_of::<PathLatency>() as u64)
            .saturating_add(
                (self.edges.len() as u64).saturating_mul(size_of::<EdgeLatency>() as u64),
            )
    }
}

/// One topological walk; no path enumeration, which would be exponential on reconvergence.
pub(crate) fn analyze(
    ir: &GraphIr,
    order: &[NodeId],
    rate: SampleRate,
    policy: CompensationPolicy,
) -> Result<PathLatencies, CompileError> {
    let kinds: HashMap<_, _> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.kind()))
        .collect();
    let scopes: HashMap<_, _> = ir
        .nodes()
        .iter()
        .map(|node| (node.id(), node.scope()))
        .collect();
    let mut incoming: HashMap<NodeId, Vec<&IrEdge>> = HashMap::new();
    for edge in ir
        .edges()
        .iter()
        .filter(|edge| edge.domain() != SignalDomain::Event)
    {
        incoming.entry(edge.to().0).or_default().push(edge);
    }
    let voices = ir.voice_instances().get();
    let mut paths: HashMap<NodeId, PathLatency> = HashMap::new();
    let mut result = PathLatencies {
        policy,
        ..PathLatencies::default()
    };
    for id in order {
        let Some(kind) = kinds.get(id) else { continue };
        let arrivals = incoming.get(id).map_or(&[][..], Vec::as_slice);
        let latest = arrivals
            .iter()
            .filter_map(|edge| paths.get(&edge.from().0))
            .map(|path| path.latest)
            .max()
            .unwrap_or(FrameCount::ZERO);
        let align = policy == CompensationPolicy::Compensate;
        let earliest = arrivals
            .iter()
            .filter_map(|edge| paths.get(&edge.from().0))
            .map(|path| path.earliest)
            .min()
            .unwrap_or(FrameCount::ZERO);
        for edge in arrivals {
            let arrival = paths
                .get(&edge.from().0)
                .map_or(FrameCount::ZERO, |path| path.latest);
            // The maximum above proves this subtraction is nonnegative.
            let skew = FrameCount::new(latest.as_u64() - arrival.as_u64());
            if align && skew != FrameCount::ZERO {
                let in_voice = scopes.get(id) == Some(&crate::ir::ExecutionScope::Voice);
                let instances = if in_voice { voices } else { 1 };
                if in_voice {
                    result.voice_groups = crate::quantities::RecordCount::measured(
                        result.voice_groups.get().saturating_add(1),
                    );
                }
                result.compensation_records = crate::quantities::RecordCount::measured(
                    result.compensation_records.get().saturating_add(instances),
                );
                // Validated input identity; an output has only one cable and zero skew.
                let channels = ir
                    .descriptor(*kind)
                    .and_then(|descriptor| {
                        descriptor.ports.into_iter().find(|port| {
                            port.direction() == crate::validate::PortDirection::Input
                                && port.id() == edge.to().1
                        })
                    })
                    .map_or(0, |port| port.layout().channels());
                result.compensation_history = crate::quantities::PreparedBytes::measured(
                    result.compensation_history.get().saturating_add(
                        skew.as_u64()
                            .saturating_mul(channels as u64)
                            .saturating_mul(size_of::<f32>() as u64)
                            .saturating_mul(u64::from(instances)),
                    ),
                );
            }
            result.edges.push(EdgeLatency {
                edge: edge.id(),
                skew,
                compensation: if align { skew } else { FrameCount::ZERO },
            });
        }
        let own = crate::node::timing_of(*kind, rate).latency;
        // Preserve any skew inherited from an uncompensated multi-input node.
        let earliest = if align {
            arrivals
                .iter()
                .filter_map(|edge| paths.get(&edge.from().0))
                .map(|path| {
                    FrameCount::new(
                        latest.as_u64() - (path.latest.as_u64() - path.earliest.as_u64()),
                    )
                })
                .min()
                .unwrap_or(FrameCount::ZERO)
        } else {
            earliest
        };
        let path = PathLatency {
            node: *id,
            earliest: earliest
                .checked_add(own)
                .ok_or(CompileError::PathLatencyOverflow { node: *id })?,
            latest: latest
                .checked_add(own)
                .ok_or(CompileError::PathLatencyOverflow { node: *id })?,
        };
        if ir.is_modulation_source(*id) && path.latest != FrameCount::ZERO {
            return Err(CompileError::ModulationLatencyUnsupported { node: *id });
        }
        if matches!(kind, IrNodeKind::Output) {
            let _total = path
                .latest
                .checked_add(FrameCount::QUANTUM)
                .ok_or(CompileError::PathLatencyOverflow { node: *id })?;
            result.output = path.latest;
        }
        paths.insert(*id, path);
    }
    // Vec allocations must fit isize::MAX bytes, even when a profile permits u64::MAX.
    // Saturation here is a lower bound that necessarily exceeds the platform ceiling;
    // it cannot turn an unrepresentable history into an apparently fitting demand.
    let history = ir
        .nodes()
        .iter()
        .fold(result.compensation_history.get(), |total, node| {
            let instances = if node.scope() == crate::ir::ExecutionScope::Voice {
                voices
            } else {
                1
            };
            total.saturating_add(
                crate::node::history_bytes(node.kind(), rate).saturating_mul(u64::from(instances)),
            )
        });
    if history > u64::try_from(isize::MAX).unwrap_or(u64::MAX) {
        return Err(CompileError::HistoryStorageUnrepresentable {
            bytes: crate::quantities::PreparedBytes::measured(history),
        });
    }
    result.paths = paths.into_values().collect();
    result.paths.sort_by_key(|path| path.node);
    result.edges.sort_by_key(|edge| edge.edge);
    result.paths = result.paths.into_boxed_slice().into_vec();
    result.edges = result.edges.into_boxed_slice().into_vec();
    Ok(result)
}
