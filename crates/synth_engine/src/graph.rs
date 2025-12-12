//! Module graph - manages connections between modules.
//!
//! The graph handles:
//! - Module registration and lookup
//! - Connections between module ports
//! - Processing order calculation
//! - Signal routing during audio processing

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ModuleId;
use synth_core::{
    AudioBuffer, InputPorts, ModuleCategory, ModuleDescriptor, PolyModule, PortDirection,
    ProcessContext,
};
use synth_core::{MidiNote, PortName, Velocity};
use synth_core::{ModuleType, Param};

/// A connection between two ports.
///
/// Uses `PortName` (interned strings) for zero-allocation port name handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Connection {
    /// Source module.
    pub from_module: ModuleId,
    /// Source port name (interned for zero-allocation copying).
    pub from_port: PortName,
    /// Destination module.
    pub to_module: ModuleId,
    /// Destination port name (interned for zero-allocation copying).
    pub to_port: PortName,
}

impl Connection {
    pub fn new(
        from_module: ModuleId,
        from_port: impl Into<PortName>,
        to_module: ModuleId,
        to_port: impl Into<PortName>,
    ) -> Self {
        Self {
            from_module,
            from_port: from_port.into(),
            to_module,
            to_port: to_port.into(),
        }
    }
}

/// Node in the module graph.
struct GraphNode {
    /// The module.
    module: Box<dyn PolyModule>,
    /// Module descriptor (cached).
    descriptor: ModuleDescriptor,
    /// Output buffers.
    /// Note: Uses String keys for compatibility with PolyModule trait.
    /// This will be changed to PortName when we update the trait (Problem 3).
    outputs: HashMap<String, AudioBuffer>,
}

impl std::fmt::Debug for GraphNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphNode")
            .field("descriptor", &self.descriptor)
            .field("outputs", &self.outputs.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// The module graph for a voice or effect chain.
pub struct ModuleGraph {
    /// All modules in the graph.
    nodes: HashMap<ModuleId, GraphNode>,
    /// All connections.
    connections: HashSet<Connection>,
    /// Processing order (topologically sorted).
    processing_order: Vec<ModuleId>,
    /// Instance counters per module type for ID generation.
    instance_counters: HashMap<ModuleType, u16>,
    /// Is the processing order dirty (needs recalculation).
    order_dirty: bool,
    /// Buffer size.
    buffer_size: usize,
    /// Pre-allocated input buffers for processing (avoid allocations in audio thread).
    /// Vec of (port_name, buffer) pairs - allows creating a reference slice without allocation.
    input_buffers: Vec<(PortName, AudioBuffer)>,
    /// Pre-allocated vec for gathering incoming connections.
    /// Uses PortName for zero-allocation copying of connection info.
    incoming_cache: Vec<(ModuleId, PortName, PortName)>,
}

impl ModuleGraph {
    /// Create a new empty graph.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            connections: HashSet::new(),
            processing_order: Vec::new(),
            instance_counters: HashMap::new(),
            order_dirty: true,
            buffer_size: 256,
            input_buffers: Vec::with_capacity(8),
            incoming_cache: Vec::with_capacity(16),
        }
    }

    /// Clear all modules and connections.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.connections.clear();
        self.processing_order.clear();
        self.instance_counters.clear();
        self.order_dirty = true;
        self.input_buffers.clear();
        self.incoming_cache.clear();
    }

    /// Get the next instance number for a module type.
    fn next_instance(&mut self, module_type: ModuleType) -> u16 {
        let counter = self.instance_counters.entry(module_type).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Add a module to the graph.
    pub fn add_module(&mut self, module: Box<dyn PolyModule>) -> ModuleId {
        let module_type = module.module_type();
        let instance = self.next_instance(module_type);
        let id = ModuleId::new(module_type, instance);

        let descriptor = module.descriptor();

        // Create output buffers
        let mut outputs = HashMap::new();
        for port in &descriptor.ports {
            if port.direction == PortDirection::Output {
                outputs.insert(port.name.clone(), AudioBuffer::new(self.buffer_size));
            }
        }

        self.nodes.insert(
            id,
            GraphNode {
                module,
                descriptor,
                outputs,
            },
        );

        self.order_dirty = true;
        id
    }

    /// Add a module with a specific ID.
    pub fn add_module_with_id(&mut self, id: ModuleId, module: Box<dyn PolyModule>) {
        let descriptor = module.descriptor();

        let mut outputs = HashMap::new();
        for port in &descriptor.ports {
            if port.direction == PortDirection::Output {
                outputs.insert(port.name.clone(), AudioBuffer::new(self.buffer_size));
            }
        }

        self.nodes.insert(
            id,
            GraphNode {
                module,
                descriptor,
                outputs,
            },
        );

        // Update instance counter if this ID is higher
        let counter = self.instance_counters.entry(id.module_type).or_insert(0);
        if id.instance > *counter {
            *counter = id.instance;
        }

        self.order_dirty = true;
    }

    /// Remove a module from the graph (drops the module).
    pub fn remove_module(&mut self, id: ModuleId) {
        // Remove all connections involving this module
        self.connections
            .retain(|c| c.from_module != id && c.to_module != id);
        self.order_dirty = true;
        self.nodes.remove(&id);
    }

    /// Remove a module from the graph and return it.
    /// Use this to defer dropping to a non-audio thread.
    pub fn remove_module_and_return(&mut self, id: ModuleId) -> Option<Box<dyn PolyModule>> {
        // Remove all connections involving this module
        self.connections
            .retain(|c| c.from_module != id && c.to_module != id);
        self.order_dirty = true;
        self.nodes.remove(&id).map(|n| n.module)
    }

    /// Get a module by ID.
    pub fn get_module(&self, id: ModuleId) -> Option<&dyn PolyModule> {
        self.nodes.get(&id).map(|n| n.module.as_ref())
    }

    /// Get a mutable module by ID.
    pub fn get_module_mut(&mut self, id: ModuleId) -> Option<&mut (dyn PolyModule + '_)> {
        if let Some(node) = self.nodes.get_mut(&id) {
            Some(node.module.as_mut())
        } else {
            None
        }
    }

    /// Get module descriptor by ID.
    pub fn get_descriptor(&self, id: ModuleId) -> Option<&ModuleDescriptor> {
        self.nodes.get(&id).map(|n| &n.descriptor)
    }

    /// Connect two modules.
    pub fn connect(
        &mut self,
        from_module: ModuleId,
        from_port: impl Into<String>,
        to_module: ModuleId,
        to_port: impl Into<String>,
    ) -> Result<(), GraphError> {
        let from_port = from_port.into();
        let to_port = to_port.into();

        // Validate connection
        self.validate_connection(from_module, &from_port, to_module, &to_port)?;

        let connection = Connection::new(from_module, from_port, to_module, to_port);
        self.connections.insert(connection);
        self.order_dirty = true;

        Ok(())
    }

    /// Disconnect two modules.
    pub fn disconnect(
        &mut self,
        from_module: ModuleId,
        from_port: &str,
        to_module: ModuleId,
        to_port: &str,
    ) -> bool {
        let connection = Connection::new(from_module, from_port, to_module, to_port);
        let removed = self.connections.remove(&connection);
        if removed {
            self.order_dirty = true;
        }
        removed
    }

    /// Disconnect all connections from/to a module.
    pub fn disconnect_all(&mut self, module: ModuleId) {
        let before = self.connections.len();
        self.connections
            .retain(|c| c.from_module != module && c.to_module != module);
        if self.connections.len() != before {
            self.order_dirty = true;
        }
    }

    /// Get all connections.
    pub fn connections(&self) -> impl Iterator<Item = &Connection> {
        self.connections.iter()
    }

    /// Get the processing order (topologically sorted module IDs).
    /// Returns None if the order needs recalculation.
    pub fn processing_order(&self) -> &[ModuleId] {
        &self.processing_order
    }

    /// Check if a module is a "source" (no incoming connections).
    pub fn is_source(&self, module_id: ModuleId) -> bool {
        !self.connections.iter().any(|c| c.to_module == module_id)
    }

    /// Check if a module is a "sink" (no outgoing connections).
    pub fn is_sink(&self, module_id: ModuleId) -> bool {
        !self.connections.iter().any(|c| c.from_module == module_id)
    }

    /// Get the processing position of a module (0-based index in topological order).
    /// Returns None if the module is not in the graph or order is dirty.
    pub fn processing_position(&self, module_id: ModuleId) -> Option<usize> {
        self.processing_order.iter().position(|&id| id == module_id)
    }

    /// Set a parameter on a module.
    pub fn set_param(&mut self, module: ModuleId, param: Param) {
        if let Some(node) = self.nodes.get_mut(&module) {
            node.module.set_param(param);
        }
    }

    /// Get a parameter from a module.
    pub fn get_param(&self, module: ModuleId, param: &Param) -> Option<f32> {
        self.nodes.get(&module)?.module.get_param(param)
    }

    /// Load a sample into a module (for sample-based modules like SamplePlayer).
    /// Returns true if the sample was loaded, false if the module doesn't exist
    /// or doesn't support sample loading.
    pub fn load_sample(
        &mut self,
        module: ModuleId,
        sample: std::sync::Arc<synth_core::Sample>,
    ) -> bool {
        if let Some(node) = self.nodes.get_mut(&module) {
            node.module.load_sample(sample)
        } else {
            false
        }
    }

    /// Process the graph.
    pub fn process(&mut self, output: &mut AudioBuffer, context: &ProcessContext) {
        // Ensure buffer sizes
        self.resize_buffers(context.samples.as_usize());

        // Update processing order if needed
        if self.order_dirty {
            self.calculate_processing_order();
        }

        // Process modules in order (iterate by index to avoid cloning the Vec)
        for i in 0..self.processing_order.len() {
            let module_id = self.processing_order[i];
            self.process_module(module_id, context);
        }

        // Copy from final output module (if any) to output buffer
        // Priority 1: Look for a dedicated Output category module
        if let Some((&id, _)) = self
            .nodes
            .iter()
            .find(|(_, n)| n.descriptor.category == ModuleCategory::Output)
            && let Some(node) = self.nodes.get(&id)
            && let Some(out_buf) = node.outputs.get("out")
        {
            output.copy_from(out_buf);
            return;
        }

        // Priority 2: Find a "sink" module (has connections IN but none OUT)
        // This ensures we only use modules that are actually endpoints in the graph
        let found = self
            .processing_order
            .iter()
            .rev()
            .copied()
            .filter(|&id| self.is_sink(id))
            .filter(|&id| self.connections.iter().any(|c| c.to_module == id))
            .find_map(|id| self.nodes.get(&id)?.outputs.get("out"));

        if let Some(out_buf) = found {
            output.copy_from(out_buf);
            return;
        }

        // Priority 3: If there are connections, use the last connected sink
        if !self.connections.is_empty() {
            let found = self
                .processing_order
                .iter()
                .rev()
                .copied()
                .filter(|&id| self.is_sink(id))
                .find_map(|id| self.nodes.get(&id)?.outputs.get("out"));

            if let Some(out_buf) = found {
                output.copy_from(out_buf);
                return;
            }
        }

        // No valid output found - return silence
        // (disconnected modules should not produce output)
        output.clear();
    }

    /// Trigger note on for all modules.
    pub fn note_on(&mut self, note: MidiNote, velocity: Velocity) {
        self.nodes
            .values_mut()
            .for_each(|node| node.module.note_on(note, velocity));
    }

    /// Trigger note off for all modules.
    pub fn note_off(&mut self) {
        self.nodes
            .values_mut()
            .for_each(|node| node.module.note_off());
    }

    /// Reset all modules.
    pub fn reset(&mut self) {
        self.nodes.values_mut().for_each(|node| node.module.reset());
    }

    /// Get a module's output buffer by port name (for extracting stereo outputs).
    /// This is useful after processing to get specific output ports.
    pub fn get_module_output(&self, module_id: ModuleId, port_name: &str) -> Option<&AudioBuffer> {
        self.nodes.get(&module_id)?.outputs.get(port_name)
    }

    /// Find a module by type and return its ID.
    /// Useful for finding the amplifier in a voice graph.
    pub fn find_module_by_type(&self, module_type: ModuleType) -> Option<ModuleId> {
        self.nodes
            .iter()
            .find(|(id, _)| id.module_type == module_type)
            .map(|(id, _)| *id)
    }

    /// Set the base frequency for all oscillator modules in the graph.
    /// Used by Voice to inject the note frequency before processing.
    pub fn set_oscillator_frequency(&mut self, freq: synth_core::Hertz) {
        use synth_core::{OscillatorParam, Param};

        for (&id, node) in self.nodes.iter_mut() {
            if id.module_type == ModuleType::Oscillator {
                node.module
                    .set_param(Param::Oscillator(OscillatorParam::Frequency(freq)));
            }
        }
    }

    /// Get all module IDs.
    pub fn module_ids(&self) -> impl Iterator<Item = ModuleId> + '_ {
        self.nodes.keys().copied()
    }

    /// Check if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get the number of modules in the graph.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Clone the graph structure.
    pub fn clone_structure(&self) -> Self {
        let mut new_graph = ModuleGraph::new();
        new_graph.instance_counters = self.instance_counters.clone();
        new_graph.buffer_size = self.buffer_size;

        // Clone all modules
        for (&id, node) in &self.nodes {
            new_graph.add_module_with_id(id, node.module.box_clone());
        }

        // Clone all connections
        new_graph.connections = self.connections.clone();
        new_graph.order_dirty = true;

        new_graph
    }

    // === Private methods ===

    fn validate_connection(
        &self,
        from_module: ModuleId,
        from_port: &str,
        to_module: ModuleId,
        to_port: &str,
    ) -> Result<(), GraphError> {
        // Check modules exist
        let from_node = self
            .nodes
            .get(&from_module)
            .ok_or(GraphError::ModuleNotFound(from_module))?;
        let to_node = self
            .nodes
            .get(&to_module)
            .ok_or(GraphError::ModuleNotFound(to_module))?;

        // Check ports exist and have correct directions
        let from_port_desc = from_node
            .descriptor
            .ports
            .iter()
            .find(|p| p.name == from_port)
            .ok_or_else(|| GraphError::PortNotFound(from_module, from_port.to_string()))?;

        let to_port_desc = to_node
            .descriptor
            .ports
            .iter()
            .find(|p| p.name == to_port)
            .ok_or_else(|| GraphError::PortNotFound(to_module, to_port.to_string()))?;

        if from_port_desc.direction != PortDirection::Output {
            return Err(GraphError::InvalidConnection(
                "Source port must be an output".to_string(),
            ));
        }

        if to_port_desc.direction != PortDirection::Input {
            return Err(GraphError::InvalidConnection(
                "Destination port must be an input".to_string(),
            ));
        }

        // Check for cycles (would cause infinite loop)
        if self.would_create_cycle(from_module, to_module) {
            return Err(GraphError::CycleDetected);
        }

        Ok(())
    }

    fn would_create_cycle(&self, from: ModuleId, to: ModuleId) -> bool {
        // Check if we can reach 'from' starting from 'to' using DFS
        // If we can, adding an edge from->to would create a cycle

        if from == to {
            return true; // Self-loop
        }

        let mut visited = HashSet::new();
        let mut stack = vec![to];

        while let Some(current) = stack.pop() {
            if current == from {
                return true; // Found a path from 'to' to 'from', cycle would be created
            }

            if visited.insert(current) {
                // Find all modules that 'current' connects to
                for conn in &self.connections {
                    if conn.from_module == current {
                        stack.push(conn.to_module);
                    }
                }
            }
        }

        false
    }

    fn calculate_processing_order(&mut self) {
        // Topological sort using Kahn's algorithm
        let mut order = Vec::new();
        let mut in_degree: HashMap<ModuleId, usize> = HashMap::new();
        let mut adjacency: HashMap<ModuleId, Vec<ModuleId>> = HashMap::new();

        // Initialize
        for &id in self.nodes.keys() {
            in_degree.insert(id, 0);
            adjacency.insert(id, Vec::new());
        }

        // Build adjacency list and in-degrees
        // Only process connections where both modules exist
        for conn in &self.connections {
            if let (Some(adj), Some(deg)) = (
                adjacency.get_mut(&conn.from_module),
                in_degree.get_mut(&conn.to_module),
            ) {
                adj.push(conn.to_module);
                *deg += 1;
            }
        }

        // Find all nodes with in-degree 0
        let mut queue: VecDeque<ModuleId> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        while let Some(id) = queue.pop_front() {
            order.push(id);

            if let Some(neighbors) = adjacency.get(&id) {
                for &neighbor in neighbors {
                    if let Some(deg) = in_degree.get_mut(&neighbor) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(neighbor);
                        }
                    }
                }
            }
        }

        // If order doesn't contain all nodes, there's a cycle
        // (shouldn't happen if validate_connection works correctly)
        if order.len() != self.nodes.len() {
            // Fallback: just use arbitrary order
            order = self.nodes.keys().copied().collect();
        }

        self.processing_order = order;
        self.order_dirty = false;
    }

    fn process_module(&mut self, module_id: ModuleId, context: &ProcessContext) {
        // Gather incoming connections into pre-allocated cache (avoid per-frame allocation)
        // Connection ports are PortName (Copy, no allocation)
        self.incoming_cache.clear();
        for conn in &self.connections {
            if conn.to_module == module_id {
                self.incoming_cache.push((
                    conn.from_module,
                    conn.from_port, // PortName is Copy - no allocation!
                    conn.to_port,   // PortName is Copy - no allocation!
                ));
            }
        }

        // Clear and reuse the pre-allocated input buffers Vec
        // We clear all buffers to silence and track which ports have data
        for (_, buf) in self.input_buffers.iter_mut() {
            buf.clear();
        }

        // Gather inputs from connected modules
        // Uses Vec for zero-allocation (no HashMap creation per frame)
        for (from_module, from_port, to_port) in &self.incoming_cache {
            let from_port_str = from_port.as_str();
            if let Some(from_node) = self.nodes.get(from_module)
                && let Some(output_buf) = from_node.outputs.get(from_port_str)
            {
                // Sum inputs if multiple connections to same port
                // Linear search in Vec is fast for typical 1-4 input ports
                if let Some((_, existing)) = self
                    .input_buffers
                    .iter_mut()
                    .find(|(name, _)| *name == *to_port)
                {
                    // Ensure buffer is correctly sized before adding
                    if existing.len() < context.samples.as_usize() {
                        existing.resize(context.samples.as_usize());
                    }
                    existing.add_from(output_buf);
                } else {
                    // First connection to this port - add new buffer entry
                    // Note: Vec only grows during warmup, not during steady-state processing
                    let mut buf = AudioBuffer::new(context.samples.as_usize());
                    buf.copy_from(output_buf);
                    self.input_buffers.push((*to_port, buf));
                }
            }
        }

        // Build InputPorts from the input_buffers Vec.
        // Uses a small Vec allocation for the reference slice, which is much cheaper
        // than the old HashMap<String, &AudioBuffer> approach:
        // - No String cloning (PortName is Copy - 8 bytes vs 24+ bytes for String)
        // - No hashing or bucket allocation
        // - Typical size is 1-4 entries, so Vec allocation is minimal
        //
        // Trade-off: Still allocates a small Vec per module per frame, but this is
        // acceptable for realtime audio (takes nanoseconds for 4-8 pointers).
        let input_refs: Vec<(PortName, &AudioBuffer)> = self
            .input_buffers
            .iter()
            .map(|(name, buf)| (*name, buf))
            .collect();
        let inputs = InputPorts::new(&input_refs);

        // Get the node and process
        if let Some(node) = self.nodes.get_mut(&module_id) {
            // Clear output buffers
            for buf in node.outputs.values_mut() {
                buf.clear();
            }

            // Process
            node.module.process(inputs, &mut node.outputs, context);
        }
    }

    fn resize_buffers(&mut self, size: usize) {
        if self.buffer_size != size {
            self.buffer_size = size;
            for node in self.nodes.values_mut() {
                for buf in node.outputs.values_mut() {
                    buf.resize(size);
                }
            }
            // Also resize the input buffer cache to prevent out-of-bounds access
            for (_, buf) in self.input_buffers.iter_mut() {
                buf.resize(size);
            }
        }
    }
}

impl Default for ModuleGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur in the module graph.
#[derive(Debug, Clone, thiserror::Error)]
pub enum GraphError {
    #[error("Module not found: {0:?}")]
    ModuleNotFound(ModuleId),

    #[error("Port not found: {0:?}/{1}")]
    PortNotFound(ModuleId, String),

    #[error("Invalid connection: {0}")]
    InvalidConnection(String),

    #[error("Connection would create a cycle")]
    CycleDetected,
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_modules::Oscillator;

    #[test]
    fn test_graph_creation() {
        let graph = ModuleGraph::new();
        assert_eq!(graph.nodes.len(), 0);
    }

    #[test]
    fn test_add_module() {
        let mut graph = ModuleGraph::new();
        let osc = Oscillator::new();
        let id = graph.add_module(Box::new(osc));

        assert!(graph.get_module(id).is_some());
    }

    #[test]
    fn test_connect_modules() {
        let mut graph = ModuleGraph::new();

        let osc = Oscillator::new();
        let _osc_id = graph.add_module(Box::new(osc));

        // Add a second oscillator (just for testing connections)
        let osc2 = Oscillator::new();
        let _osc2_id = graph.add_module(Box::new(osc2));

        // This should fail because oscillators don't have audio inputs
        // (they're sources, not processors)
        // A real test would use Filter or Amplifier
    }
}
