//! Graph debugger for analyzing module connections.
//!
//! Helps verify that signal paths are correctly configured.

use std::collections::{HashMap, HashSet, VecDeque};

use synth_core::ModuleType;
use synth_engine::{Connection, ModuleGraph, ModuleId};

/// Information about a connection.
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    /// Source module.
    pub from_module: ModuleId,
    /// Source port name.
    pub from_port: String,
    /// Destination module.
    pub to_module: ModuleId,
    /// Destination port name.
    pub to_port: String,
}

impl From<&Connection> for ConnectionInfo {
    fn from(conn: &Connection) -> Self {
        Self {
            from_module: conn.from_module,
            from_port: conn.from_port.as_str().to_string(),
            to_module: conn.to_module,
            to_port: conn.to_port.as_str().to_string(),
        }
    }
}

/// A step in a signal path trace.
#[derive(Debug, Clone)]
pub struct PathStep {
    /// Module ID at this step.
    pub module_id: ModuleId,
    /// Module type.
    pub module_type: ModuleType,
    /// Input port used (None for source modules).
    pub input_port: Option<String>,
    /// Output port used (None for sink modules).
    pub output_port: Option<String>,
}

/// A traced signal path from source to destination.
#[derive(Debug, Clone)]
pub struct PathTrace {
    /// Steps in the path from source to destination.
    pub steps: Vec<PathStep>,
    /// True if the path reaches the destination.
    pub complete: bool,
}

impl PathTrace {
    /// Get a human-readable description of the path.
    pub fn describe(&self) -> String {
        if self.steps.is_empty() {
            return "Empty path".to_string();
        }

        let mut parts = Vec::new();
        for step in &self.steps {
            let port_info = match (&step.input_port, &step.output_port) {
                (None, Some(out)) => format!(" ->{}", out),
                (Some(inp), None) => format!("{}->", inp),
                (Some(inp), Some(out)) => format!("{}->{}", inp, out),
                (None, None) => String::new(),
            };
            parts.push(format!("{}{}", step.module_id, port_info));
        }

        let status = if self.complete {
            "✓"
        } else {
            "✗ (incomplete)"
        };
        format!("{} {}", parts.join(" → "), status)
    }
}

/// Debugger for analyzing ModuleGraph connections.
pub struct GraphDebugger<'a> {
    graph: &'a ModuleGraph,
}

impl<'a> GraphDebugger<'a> {
    /// Create a new debugger for a graph.
    pub fn new(graph: &'a ModuleGraph) -> Self {
        Self { graph }
    }

    /// Get all connections in the graph.
    pub fn connections(&self) -> Vec<ConnectionInfo> {
        self.graph.connections().map(ConnectionInfo::from).collect()
    }

    /// Check if a path exists from one module to another.
    pub fn path_exists(&self, from: ModuleId, to: ModuleId) -> bool {
        if from == to {
            return true;
        }

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(from);

        while let Some(current) = queue.pop_front() {
            if current == to {
                return true;
            }

            if visited.insert(current) {
                for conn in self.graph.connections() {
                    if conn.from_module == current {
                        queue.push_back(conn.to_module);
                    }
                }
            }
        }

        false
    }

    /// Trace a path from one module to another.
    /// Returns the first path found, or an incomplete path if no path exists.
    pub fn trace_path(&self, from: ModuleId, to: ModuleId) -> PathTrace {
        if from == to {
            let module_type = from.module_type;
            return PathTrace {
                steps: vec![PathStep {
                    module_id: from,
                    module_type,
                    input_port: None,
                    output_port: None,
                }],
                complete: true,
            };
        }

        // BFS with path reconstruction
        let mut visited = HashSet::new();
        let mut parent: HashMap<ModuleId, (ModuleId, String, String)> = HashMap::new();
        let mut queue = VecDeque::new();
        queue.push_back(from);
        visited.insert(from);

        while let Some(current) = queue.pop_front() {
            if current == to {
                // Reconstruct path
                return self.reconstruct_path(from, to, &parent);
            }

            for conn in self.graph.connections() {
                if conn.from_module == current && !visited.contains(&conn.to_module) {
                    visited.insert(conn.to_module);
                    parent.insert(
                        conn.to_module,
                        (
                            current,
                            conn.from_port.as_str().to_string(),
                            conn.to_port.as_str().to_string(),
                        ),
                    );
                    queue.push_back(conn.to_module);
                }
            }
        }

        // No path found - return partial path to closest reached module
        PathTrace {
            steps: vec![PathStep {
                module_id: from,
                module_type: from.module_type,
                input_port: None,
                output_port: None,
            }],
            complete: false,
        }
    }

    fn reconstruct_path(
        &self,
        from: ModuleId,
        to: ModuleId,
        parent: &HashMap<ModuleId, (ModuleId, String, String)>,
    ) -> PathTrace {
        let mut steps = Vec::new();
        let mut current = to;

        // Walk backwards from destination to source
        while let Some((prev, from_port, to_port)) = parent.get(&current) {
            steps.push(PathStep {
                module_id: current,
                module_type: current.module_type,
                input_port: Some(to_port.clone()),
                output_port: None,
            });
            current = *prev;

            // Update the previous step's output port
            // The from_port becomes the output of the previous module
            // which we'll add in the next iteration
            // Store from_port for the previous module
            if current == from {
                steps.push(PathStep {
                    module_id: from,
                    module_type: from.module_type,
                    input_port: None,
                    output_port: Some(from_port.clone()),
                });
                break;
            } else if let Some(last) = steps.last_mut() {
                last.output_port = Some(from_port.clone());
            }
        }

        // Handle case where from == to or direct connection
        if steps.is_empty() {
            steps.push(PathStep {
                module_id: from,
                module_type: from.module_type,
                input_port: None,
                output_port: None,
            });
        }

        steps.reverse();

        PathTrace {
            steps,
            complete: true,
        }
    }

    /// Find all outputs that are not connected to anything.
    pub fn unconnected_outputs(&self) -> Vec<(ModuleId, String)> {
        let mut result = Vec::new();

        // Get all output ports from descriptors
        for module_id in self.graph.module_ids() {
            if let Some(descriptor) = self.graph.get_descriptor(module_id) {
                for port in &descriptor.ports {
                    if port.direction == synth_modules::PortDirection::Output {
                        // Check if this output is connected
                        let is_connected = self.graph.connections().any(|c| {
                            c.from_module == module_id && c.from_port.as_str() == port.name
                        });
                        if !is_connected {
                            result.push((module_id, port.name.clone()));
                        }
                    }
                }
            }
        }

        result
    }

    /// Find all inputs that are not connected to anything.
    pub fn unconnected_inputs(&self) -> Vec<(ModuleId, String)> {
        let mut result = Vec::new();

        for module_id in self.graph.module_ids() {
            if let Some(descriptor) = self.graph.get_descriptor(module_id) {
                for port in &descriptor.ports {
                    if port.direction == synth_modules::PortDirection::Input {
                        let is_connected = self
                            .graph
                            .connections()
                            .any(|c| c.to_module == module_id && c.to_port.as_str() == port.name);
                        if !is_connected {
                            result.push((module_id, port.name.clone()));
                        }
                    }
                }
            }
        }

        result
    }

    /// Find a module by type.
    pub fn find_module(&self, module_type: ModuleType) -> Option<ModuleId> {
        self.graph.find_module_by_type(module_type)
    }

    /// Find all modules of a specific type.
    pub fn find_modules(&self, module_type: ModuleType) -> Vec<ModuleId> {
        self.graph
            .module_ids()
            .filter(|id| id.module_type == module_type)
            .collect()
    }

    /// Verify that a gate signal can reach an envelope.
    /// Returns Ok with the path, or Err with a description of the problem.
    pub fn verify_gate_path(&self) -> Result<PathTrace, String> {
        // Find gate source (typically oscillator or gate input)
        // and envelope

        let envelope = self
            .find_module(ModuleType::Envelope)
            .ok_or("No envelope module found")?;

        // Check if envelope has gate input connected
        let gate_connected = self.graph.connections().any(|c| {
            c.to_module == envelope
                && (c.to_port.as_str() == "gate" || c.to_port.as_str() == "trigger")
        });

        if !gate_connected {
            return Err(format!(
                "Envelope {} has no gate/trigger input connected",
                envelope
            ));
        }

        // Find what's connected to the gate
        for conn in self.graph.connections() {
            if conn.to_module == envelope
                && (conn.to_port.as_str() == "gate" || conn.to_port.as_str() == "trigger")
            {
                return Ok(self.trace_path(conn.from_module, envelope));
            }
        }

        Err("Gate path verification failed".to_string())
    }

    /// Verify that envelope output reaches an amplifier/VCA.
    /// Returns Ok with the path, or Err with a description of the problem.
    pub fn verify_envelope_to_vca(&self) -> Result<PathTrace, String> {
        let envelope = self
            .find_module(ModuleType::Envelope)
            .ok_or("No envelope module found")?;

        let amplifier = self
            .find_module(ModuleType::Amplifier)
            .ok_or("No amplifier module found")?;

        if !self.path_exists(envelope, amplifier) {
            return Err(format!(
                "No path from envelope {} to amplifier {}",
                envelope, amplifier
            ));
        }

        Ok(self.trace_path(envelope, amplifier))
    }

    /// Generate a human-readable description of the graph.
    pub fn describe(&self) -> String {
        let mut lines = Vec::new();

        lines.push("=== Module Graph ===".to_string());
        lines.push(String::new());

        // List modules
        lines.push("Modules:".to_string());
        let mut modules: Vec<_> = self.graph.module_ids().collect();
        modules.sort_by(|a, b| {
            (a.module_type.prefix(), a.instance).cmp(&(b.module_type.prefix(), b.instance))
        });
        for id in &modules {
            let desc = self
                .graph
                .get_descriptor(*id)
                .map(|d| d.name.as_str())
                .unwrap_or("(unknown)");
            let source_sink = match (self.graph.is_source(*id), self.graph.is_sink(*id)) {
                (true, true) => " [isolated]",
                (true, false) => " [source]",
                (false, true) => " [sink]",
                (false, false) => "",
            };
            lines.push(format!("  {} - {}{}", id, desc, source_sink));
        }

        lines.push(String::new());

        // List connections
        lines.push("Connections:".to_string());
        let mut connections: Vec<_> = self.connections();
        connections.sort_by(|a, b| {
            (a.from_module.module_type.prefix(), a.from_module.instance)
                .cmp(&(b.from_module.module_type.prefix(), b.from_module.instance))
        });
        for conn in &connections {
            lines.push(format!(
                "  {}.{} → {}.{}",
                conn.from_module, conn.from_port, conn.to_module, conn.to_port
            ));
        }

        if connections.is_empty() {
            lines.push("  (none)".to_string());
        }

        lines.push(String::new());

        // List unconnected ports
        let unconnected_outputs = self.unconnected_outputs();
        let unconnected_inputs = self.unconnected_inputs();

        if !unconnected_outputs.is_empty() || !unconnected_inputs.is_empty() {
            lines.push("Unconnected ports:".to_string());

            for (id, port) in &unconnected_outputs {
                lines.push(format!("  {}.{} (output)", id, port));
            }
            for (id, port) in &unconnected_inputs {
                lines.push(format!("  {}.{} (input)", id, port));
            }
        }

        lines.join("\n")
    }

    /// Check for common wiring problems.
    pub fn diagnose(&self) -> Vec<String> {
        let mut issues = Vec::new();

        // Check for envelope without gate
        if let Some(envelope) = self.find_module(ModuleType::Envelope) {
            let has_gate = self.graph.connections().any(|c| {
                c.to_module == envelope
                    && (c.to_port.as_str() == "gate" || c.to_port.as_str() == "trigger")
            });
            if !has_gate {
                issues.push(format!("Envelope {} has no gate input connected", envelope));
            }
        }

        // Check for amplifier without CV
        if let Some(amplifier) = self.find_module(ModuleType::Amplifier) {
            let has_cv = self
                .graph
                .connections()
                .any(|c| c.to_module == amplifier && c.to_port.as_str() == "cv");
            if !has_cv {
                issues.push(format!(
                    "Amplifier {} has no CV input - will be silent unless base gain > 0",
                    amplifier
                ));
            }
        }

        // Check for isolated modules (no connections at all)
        for id in self.graph.module_ids() {
            if self.graph.is_source(id) && self.graph.is_sink(id) {
                // Exclude output modules which are expected to be sinks
                if id.module_type != ModuleType::StereoOutput {
                    issues.push(format!("Module {} is isolated (no connections)", id));
                }
            }
        }

        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_modules::Oscillator;

    #[test]
    fn test_debugger_empty_graph() {
        let graph = ModuleGraph::new();
        let dbg = GraphDebugger::new(&graph);

        assert!(dbg.connections().is_empty());
        assert!(dbg.unconnected_outputs().is_empty());
    }

    #[test]
    fn test_debugger_with_module() {
        let mut graph = ModuleGraph::new();
        let osc = Oscillator::new();
        let osc_id = graph.add_module(Box::new(osc));

        let dbg = GraphDebugger::new(&graph);

        // Oscillator should have unconnected output
        let unconnected = dbg.unconnected_outputs();
        assert!(!unconnected.is_empty());
        assert!(unconnected.iter().any(|(id, _)| *id == osc_id));
    }

    #[test]
    fn test_path_exists_self() {
        let graph = ModuleGraph::new();
        let id = ModuleId::new(ModuleType::Oscillator, 1);

        // Note: path_exists with non-existent module should return false
        // but path to self should return true if the module existed
        let dbg = GraphDebugger::new(&graph);
        assert!(dbg.path_exists(id, id)); // Self-path always exists
    }

    #[test]
    fn test_describe_output() {
        let mut graph = ModuleGraph::new();
        let osc = Oscillator::new();
        let _osc_id = graph.add_module(Box::new(osc));

        let dbg = GraphDebugger::new(&graph);
        let desc = dbg.describe();

        assert!(desc.contains("Module Graph"));
        assert!(desc.contains("osc-1"));
    }
}
