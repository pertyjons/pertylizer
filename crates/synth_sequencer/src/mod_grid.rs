//! Mod Grid — pooled control-rate modulator graphs.
//!
//! A [`ModGraph`] is a project-level asset (pooled in `Song`, referenced by id)
//! whose nodes produce control-rate signals that write into the automation
//! target space. Where the patch graph processes audio per voice and the Note
//! Grid processes note events, the Mod Grid processes control signals —
//! monophonic and always-running.
//!
//! This module is the pure, serializable **data model** plus its non-RT editing
//! helpers (used by the GUI and MCP). The running instances — actual
//! `Box<dyn PolyModule>` graphs — live in the engine and are (re)built off the
//! audio thread from this data; see the engine's mod-grid pre-pass.
//!
//! Nodes are one of a hosted control-rate audio module ([`ModNodeConfig::Module`],
//! run mono), one of a handful of cheap grid-native sources (Macro, Transport,
//! MIDI CC, Audio tap), or a routing sink ([`ModNodeConfig::Target`]) that writes
//! its input into an [`AutomationTarget`]. Cables carry control signals between
//! named ports; a cable into a `Target` node's input identifies the source that
//! drives that routing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use synth_core::ModuleType;

use crate::automation::AutomationTarget;
use crate::ids::{ModGraphId, ModNodeId, TrackId};
use crate::note_graph::NodePosition;
use crate::track::TrackColor;

/// Hard cap on nodes in one graph. Adding past it is a non-RT-side error, never
/// an audio-thread concern.
pub const MAX_MOD_GRID_NODES: usize = 32;

/// The single input-port name a [`ModNodeConfig::Target`] node exposes.
pub const TARGET_INPUT_PORT: &str = "in";

/// Where a mod graph runs and how "this track" targets resolve.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum ModGraphScope {
    /// Exactly one running instance, always on. Relative `Track { track: None }`
    /// targets cannot resolve (no host) and are dropped.
    #[default]
    Global,
    /// Pooled and assigned to zero or more tracks; one running instance per
    /// assignment, and relative "this track" targets resolve to the host track.
    Track,
}

/// How a [`ModTarget`] contribution composes with the target's current value
/// (base value or authored lane value). Additive-only for now.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum CombineMode {
    /// `result = current + amount * source`, clamped to the target's range.
    #[default]
    Add,
}

/// A hosted control-rate audio module, run mono by the grid harness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModuleNode {
    /// Which module type to host (e.g. `Lfo`, `Mseg`, `EnvelopeFollower`).
    pub module_type: ModuleType,
    /// Parameter values, keyed by descriptor `type_id`. Stored as `f32` (the
    /// same boundary the MCP `set_parameter` uses); choice params encode as
    /// their choice index.
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
    /// Explicit persisted seed for random-family modules, so an offline render
    /// reproduces the live result bit-for-bit. `None` for deterministic modules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

/// Which transport-derived signal a [`TransportNode`] emits.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum TransportSource {
    /// Ramp 0→1 across each beat.
    #[default]
    BeatPhase,
    /// Ramp 0→1 across each bar.
    BarPhase,
    /// Current tempo, normalized against a nominal 20..300 BPM span.
    Tempo,
    /// Absolute song position in beats (unnormalized).
    PositionBeats,
}

/// A grid-native transport source node.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TransportNode {
    #[serde(default)]
    pub source: TransportSource,
}

/// A grid-native named knob — the song-level macro bank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MacroNode {
    /// Display name of the macro.
    pub name: String,
    /// Current value, `0.0..=1.0`.
    #[serde(default)]
    pub value: f32,
}

/// A grid-native MIDI CC input source node.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MidiCcNode {
    /// Controller number, `0..=127`.
    #[serde(default)]
    pub cc: u8,
    /// MIDI channel to listen on, or `None` for omni.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<u8>,
}

/// What a [`AudioTapNode`] reads a pre-fader level from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum AudioTapSource {
    /// A sequencer track's pre-fader instrument output.
    Track(TrackId),
    /// The master bus.
    Master,
}

/// A grid-native pre-fader level tap (source only; one-block latency).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AudioTapNode {
    pub source: AudioTapSource,
}

/// A routing sink: writes its input control signal into an automation target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModTarget {
    /// The address written to. Relative `Track { track: None }` resolves to the
    /// instance's host track (Track scope) or is dropped (Global scope).
    pub target: AutomationTarget,
    /// Depth in the target's own units (semitones for pitch, `0..1` for a
    /// normalized param, etc.). The input signal is multiplied by this.
    #[serde(default)]
    pub amount: f32,
    /// How the contribution composes with the target's current value.
    #[serde(default)]
    pub combine: CombineMode,
}

/// A single node's configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum ModNodeConfig {
    /// A hosted control-rate audio module (LFO, MSEG, envelope follower, …).
    Module(ModuleNode),
    /// A song-level macro knob.
    Macro(MacroNode),
    /// A transport-derived source (beat/bar phase, tempo, position).
    Transport(TransportNode),
    /// A MIDI CC input source.
    MidiCc(MidiCcNode),
    /// A pre-fader level tap of a track or the master bus.
    AudioTap(AudioTapNode),
    /// A routing sink into an [`AutomationTarget`].
    Target(ModTarget),
}

impl ModNodeConfig {
    /// A short snake_case tag for the node kind (for MCP/DTOs).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Module(_) => "module",
            Self::Macro(_) => "macro",
            Self::Transport(_) => "transport",
            Self::MidiCc(_) => "midi_cc",
            Self::AudioTap(_) => "audio_tap",
            Self::Target(_) => "target",
        }
    }

    /// Whether this node writes into the automation space (i.e. is a sink).
    #[must_use]
    pub fn is_target(&self) -> bool {
        matches!(self, Self::Target(_))
    }

    /// Whether this node can drive a cable (every non-target node is a source).
    #[must_use]
    pub fn is_source(&self) -> bool {
        !self.is_target()
    }
}

/// A control-signal cable between two nodes' named ports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModConnection {
    /// Source node.
    pub from: ModNodeId,
    /// Output port name on the source node (e.g. `"out"`).
    pub from_port: String,
    /// Destination node.
    pub to: ModNodeId,
    /// Input port name on the destination node (e.g. `"in"`, `"rate_cv"`).
    pub to_port: String,
}

impl ModConnection {
    /// Build a cable between two named ports.
    #[must_use]
    pub fn new(
        from: ModNodeId,
        from_port: impl Into<String>,
        to: ModNodeId,
        to_port: impl Into<String>,
    ) -> Self {
        Self {
            from,
            from_port: from_port.into(),
            to,
            to_port: to_port.into(),
        }
    }
}

/// Errors from editing / validating a [`ModGraph`]. Non-RT side only — the
/// audio thread never sees an invalid or cyclic graph because these are rejected
/// at edit time.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModGraphError {
    /// Adding a node would exceed [`MAX_MOD_GRID_NODES`].
    #[error("mod graph is at the {MAX_MOD_GRID_NODES}-node cap")]
    NodeCapExceeded,
    /// A connection references a node id not present in the graph.
    #[error("connection references unknown node {0:?}")]
    UnknownNode(ModNodeId),
    /// A cable originates at a `Target` node (sinks have no output).
    #[error("node {0:?} is a target and cannot drive a cable")]
    NotASource(ModNodeId),
    /// The exact same (node, port) → (node, port) cable already exists.
    #[error("cable {0:?}.{1} → {2:?}.{3} already exists")]
    DuplicateConnection(ModNodeId, String, ModNodeId, String),
    /// A second cable feeds an input port that already has a source (the grid
    /// evaluates a single source per input; a second would silently win).
    #[error("node {0:?} input port {1} already has a source")]
    DuplicateInput(ModNodeId, String),
    /// The connection set would create a cycle.
    #[error("cable would create a cycle")]
    Cycle,
}

/// Static mod-graph configuration — a pooled project asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ModGraph {
    /// Pool-unique id.
    pub id: ModGraphId,
    /// User-facing name, shown in the pool view.
    pub name: String,
    /// Global (one always-on instance) or Track (one instance per assignment).
    #[serde(default)]
    pub scope: ModGraphScope,
    /// Tracks this graph is assigned to (Track scope only). One running instance
    /// per assigned track; "this track" targets resolve to each host.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assigned_tracks: Vec<TrackId>,
    /// Free-text description.
    #[serde(default)]
    pub description: String,
    /// Sequencer-side color (RGB), never egui's `Color32` — this crate has no
    /// egui dependency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<TrackColor>,
    /// Nodes keyed by graph-local id.
    #[serde(default)]
    pub nodes: BTreeMap<ModNodeId, ModNodeConfig>,
    /// Cables, kept as a sorted `Vec` for deterministic serialization.
    #[serde(default)]
    pub connections: Vec<ModConnection>,
    /// Editor canvas positions per node (layout metadata; nodes without one are
    /// auto-laid-out). Persisted so an arranged canvas survives reload.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_positions: BTreeMap<ModNodeId, NodePosition>,
}

impl ModGraph {
    /// Create an empty graph with the given id and name.
    #[must_use]
    pub fn new(id: ModGraphId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            scope: ModGraphScope::Global,
            assigned_tracks: Vec::new(),
            description: String::new(),
            color: None,
            nodes: BTreeMap::new(),
            connections: Vec::new(),
            node_positions: BTreeMap::new(),
        }
    }

    /// Number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The smallest unused [`ModNodeId`] — used by non-RT editors (GUI / MCP) to
    /// allocate node ids. Smallest-unused rather than `max + 1` so a restored
    /// graph containing a `u32::MAX` node can't saturate into a collision.
    pub fn next_node_id(&self) -> ModNodeId {
        let mut candidate = 0u32;
        while self.nodes.contains_key(&ModNodeId::new(candidate)) {
            candidate += 1;
        }
        ModNodeId::new(candidate)
    }

    /// Mutable iterator over every node config, in id order. For non-RT resync
    /// of derived per-node state; structural edits still go through
    /// [`Self::try_insert_node`] / [`Self::remove_node`].
    pub fn nodes_mut(&mut self) -> impl Iterator<Item = &mut ModNodeConfig> {
        self.nodes.values_mut()
    }

    /// Insert (or replace) a node, enforcing the [`MAX_MOD_GRID_NODES`] cap
    /// (replacing an existing id never trips the cap). Replacing a node can turn
    /// a source into a `Target`, invalidating an outgoing cable, so the graph is
    /// re-validated and the edit rolled back on rejection.
    pub fn try_insert_node(
        &mut self,
        id: ModNodeId,
        config: ModNodeConfig,
    ) -> Result<(), ModGraphError> {
        if !self.nodes.contains_key(&id) && self.nodes.len() >= MAX_MOD_GRID_NODES {
            return Err(ModGraphError::NodeCapExceeded);
        }
        let previous = self.nodes.insert(id, config);
        if let Err(e) = self.validate() {
            match previous {
                Some(old) => {
                    self.nodes.insert(id, old);
                }
                None => {
                    self.nodes.remove(&id);
                }
            }
            return Err(e);
        }
        Ok(())
    }

    /// Remove a node and every cable touching it. Dropping edges can never
    /// introduce a cycle or duplicate, so this cannot fail. Returns the removed
    /// config, or `None` if the id was absent. Non-RT side only.
    pub fn remove_node(&mut self, id: ModNodeId) -> Option<ModNodeConfig> {
        let removed = self.nodes.remove(&id)?;
        self.connections.retain(|c| c.from != id && c.to != id);
        self.node_positions.remove(&id);
        Some(removed)
    }

    /// Add a cable, validating endpoints, single-source-per-input, and
    /// acyclicity. On error the cable is rolled back and the graph is unchanged.
    pub fn try_connect(&mut self, connection: ModConnection) -> Result<(), ModGraphError> {
        self.connections.push(connection);
        if let Err(e) = self.validate() {
            self.connections.pop();
            return Err(e);
        }
        Ok(())
    }

    /// Remove a specific cable. Returns `true` if one was removed.
    pub fn disconnect(&mut self, connection: &ModConnection) -> bool {
        let before = self.connections.len();
        self.connections.retain(|c| c != connection);
        self.connections.len() != before
    }

    /// Validate the whole graph: every cable references present nodes, sources
    /// aren't `Target` sinks, no duplicate cable or double-driven input, and no
    /// cycle. Non-RT side only.
    ///
    /// **Not checked: port names / directions.** This crate is descriptor-free (no
    /// module-factory dependency), so a cable to a non-existent or wrong-direction
    /// port passes here and then silently fails to wire at runtime-build time. The
    /// GUI only offers real descriptor ports, but MCP/import can still author an
    /// invalid cable; a descriptor-aware validator at the app layer is the TODO.
    pub fn validate(&self) -> Result<(), ModGraphError> {
        for c in &self.connections {
            let from = self
                .nodes
                .get(&c.from)
                .ok_or(ModGraphError::UnknownNode(c.from))?;
            if !self.nodes.contains_key(&c.to) {
                return Err(ModGraphError::UnknownNode(c.to));
            }
            if from.is_target() {
                return Err(ModGraphError::NotASource(c.from));
            }
        }
        // Duplicate cable and double-driven-input checks.
        for (i, a) in self.connections.iter().enumerate() {
            for b in &self.connections[i + 1..] {
                if a == b {
                    return Err(ModGraphError::DuplicateConnection(
                        a.from,
                        a.from_port.clone(),
                        a.to,
                        a.to_port.clone(),
                    ));
                }
                if a.to == b.to && a.to_port == b.to_port {
                    return Err(ModGraphError::DuplicateInput(a.to, a.to_port.clone()));
                }
            }
        }
        if self.has_cycle() {
            return Err(ModGraphError::Cycle);
        }
        Ok(())
    }

    /// Depth-first cycle detection over the node-level cable graph.
    fn has_cycle(&self) -> bool {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Unvisited,
            InProgress,
            Done,
        }
        let ids: Vec<ModNodeId> = self.nodes.keys().copied().collect();
        let mut marks: BTreeMap<ModNodeId, Mark> =
            ids.iter().map(|&id| (id, Mark::Unvisited)).collect();
        // Iterative DFS to avoid deep recursion on pathological graphs.
        for &start in &ids {
            if marks.get(&start).copied() != Some(Mark::Unvisited) {
                continue;
            }
            let mut stack: Vec<(ModNodeId, bool)> = vec![(start, false)];
            while let Some((node, children_done)) = stack.pop() {
                if children_done {
                    marks.insert(node, Mark::Done);
                    continue;
                }
                marks.insert(node, Mark::InProgress);
                stack.push((node, true));
                for c in self.connections.iter().filter(|c| c.from == node) {
                    match marks.get(&c.to).copied() {
                        Some(Mark::InProgress) => return true,
                        Some(Mark::Done) => {}
                        _ => stack.push((c.to, false)),
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module_node(mt: ModuleType) -> ModNodeConfig {
        ModNodeConfig::Module(ModuleNode {
            module_type: mt,
            params: BTreeMap::new(),
            seed: None,
        })
    }

    fn target_node() -> ModNodeConfig {
        ModNodeConfig::Target(ModTarget {
            target: AutomationTarget::Global(crate::automation::GlobalParam::MasterVolume),
            amount: 1.0,
            combine: CombineMode::Add,
        })
    }

    #[test]
    fn next_node_id_fills_gaps() {
        let mut g = ModGraph::new(ModGraphId::new(0), "g");
        g.try_insert_node(ModNodeId::new(0), module_node(ModuleType::Lfo))
            .unwrap();
        g.try_insert_node(ModNodeId::new(2), module_node(ModuleType::Lfo))
            .unwrap();
        // 1 is the smallest free id.
        assert_eq!(g.next_node_id(), ModNodeId::new(1));
    }

    #[test]
    fn node_cap_enforced() {
        let mut g = ModGraph::new(ModGraphId::new(0), "g");
        for i in 0..MAX_MOD_GRID_NODES as u32 {
            g.try_insert_node(ModNodeId::new(i), module_node(ModuleType::Lfo))
                .unwrap();
        }
        let err = g
            .try_insert_node(ModNodeId::new(999), module_node(ModuleType::Lfo))
            .unwrap_err();
        assert_eq!(err, ModGraphError::NodeCapExceeded);
        // Replacing an existing id still works at the cap.
        g.try_insert_node(ModNodeId::new(0), module_node(ModuleType::Mseg))
            .unwrap();
    }

    #[test]
    fn connect_validates_endpoints_and_source() {
        let mut g = ModGraph::new(ModGraphId::new(0), "g");
        g.try_insert_node(ModNodeId::new(0), module_node(ModuleType::Lfo))
            .unwrap();
        g.try_insert_node(ModNodeId::new(1), target_node()).unwrap();
        // Unknown destination.
        assert_eq!(
            g.try_connect(ModConnection::new(
                ModNodeId::new(0),
                "out",
                ModNodeId::new(7),
                TARGET_INPUT_PORT
            ))
            .unwrap_err(),
            ModGraphError::UnknownNode(ModNodeId::new(7))
        );
        // Valid: source module → target sink.
        g.try_connect(ModConnection::new(
            ModNodeId::new(0),
            "out",
            ModNodeId::new(1),
            TARGET_INPUT_PORT,
        ))
        .unwrap();
        // A target cannot be a source.
        assert_eq!(
            g.try_connect(ModConnection::new(
                ModNodeId::new(1),
                "out",
                ModNodeId::new(0),
                "rate_cv"
            ))
            .unwrap_err(),
            ModGraphError::NotASource(ModNodeId::new(1))
        );
    }

    #[test]
    fn duplicate_input_rejected() {
        let mut g = ModGraph::new(ModGraphId::new(0), "g");
        g.try_insert_node(ModNodeId::new(0), module_node(ModuleType::Lfo))
            .unwrap();
        g.try_insert_node(ModNodeId::new(1), module_node(ModuleType::Lfo))
            .unwrap();
        g.try_insert_node(ModNodeId::new(2), target_node()).unwrap();
        g.try_connect(ModConnection::new(
            ModNodeId::new(0),
            "out",
            ModNodeId::new(2),
            TARGET_INPUT_PORT,
        ))
        .unwrap();
        // Second cable into the same input port is rejected.
        assert_eq!(
            g.try_connect(ModConnection::new(
                ModNodeId::new(1),
                "out",
                ModNodeId::new(2),
                TARGET_INPUT_PORT
            ))
            .unwrap_err(),
            ModGraphError::DuplicateInput(ModNodeId::new(2), TARGET_INPUT_PORT.to_string())
        );
    }

    #[test]
    fn cycle_rejected() {
        let mut g = ModGraph::new(ModGraphId::new(0), "g");
        g.try_insert_node(ModNodeId::new(0), module_node(ModuleType::Lfo))
            .unwrap();
        g.try_insert_node(ModNodeId::new(1), module_node(ModuleType::Lfo))
            .unwrap();
        g.try_connect(ModConnection::new(
            ModNodeId::new(0),
            "out",
            ModNodeId::new(1),
            "rate_cv",
        ))
        .unwrap();
        assert_eq!(
            g.try_connect(ModConnection::new(
                ModNodeId::new(1),
                "out",
                ModNodeId::new(0),
                "rate_cv"
            ))
            .unwrap_err(),
            ModGraphError::Cycle
        );
    }

    #[test]
    fn remove_node_drops_cables() {
        let mut g = ModGraph::new(ModGraphId::new(0), "g");
        g.try_insert_node(ModNodeId::new(0), module_node(ModuleType::Lfo))
            .unwrap();
        g.try_insert_node(ModNodeId::new(1), target_node()).unwrap();
        g.try_connect(ModConnection::new(
            ModNodeId::new(0),
            "out",
            ModNodeId::new(1),
            TARGET_INPUT_PORT,
        ))
        .unwrap();
        g.remove_node(ModNodeId::new(0));
        assert!(g.connections.is_empty());
        assert_eq!(g.node_count(), 1);
    }
}
