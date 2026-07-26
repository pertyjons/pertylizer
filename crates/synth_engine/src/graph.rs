//! Module graph - manages connections between modules.
//!
//! The graph handles:
//! - Module registration and lookup
//! - Connections between module ports
//! - Processing order calculation
//! - Signal routing during audio processing

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::ModuleId;
use synth_core::{
    AudioBuffer, InputPorts, ModuleCategory, ModuleDescriptor, PolyModule, PortDirection,
    ProcessContext,
};
use synth_core::{MidiNote, PortName, Velocity};
use synth_core::{ModuleType, Param};

/// Fold a voice id with a module identity into a per-(voice, module) PRNG seed
/// base, so two script-hosting modules in the same voice — and the same module
/// across different voices — all decorrelate. Used by
/// [`ModuleGraph::set_voice_index`].
fn script_seed_base(voice_index: u32, id: ModuleId) -> u32 {
    voice_index
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((id.module_type as u32).wrapping_mul(131))
        .wrapping_add(u32::from(id.instance))
}

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
    /// Module descriptor (cached). Held behind an `Arc` so a script edit can share
    /// one freshly-built descriptor across a template + every voice node with a
    /// cheap clone (no audio-thread allocation), then defer the replaced ones'
    /// drop — see [`ModuleGraph::set_node_descriptor`].
    descriptor: Arc<ModuleDescriptor>,
    /// Output buffers (keyed by interned port name for zero-allocation lookup).
    outputs: HashMap<PortName, AudioBuffer>,
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
    /// All modules in the graph. `BTreeMap` so iteration order is
    /// deterministic across fresh instances — `HashMap`'s per-instance
    /// `RandomState` breaks stable traversal and complicates reproducible
    /// diagnostics even though DSP randomness is now per-module.
    nodes: BTreeMap<ModuleId, GraphNode>,
    /// All connections.
    connections: HashSet<Connection>,
    /// Processing order (topologically sorted).
    processing_order: Vec<ModuleId>,
    /// Instance counters per module type for ID generation.
    instance_counters: HashMap<ModuleType, u16>,
    /// Is the processing order dirty (needs recalculation).
    order_dirty: bool,
    /// Buffer size.
    buffer_size: synth_core::BlockSize,
    /// Pre-allocated input buffers for processing (avoid allocations in audio thread).
    /// Vec of (port_name, buffer) pairs - allows creating a reference slice without allocation.
    input_buffers: Vec<(PortName, AudioBuffer)>,
    /// Cheap Mod-Grid sources injected into a hosted module's control input port
    /// as a block-constant value (`Macro → LFO.rate_cv`). Slots are pre-created
    /// off the audio thread ([`Self::push_input_injection`]); the pre-pass only
    /// updates values ([`Self::set_input_injection_value`], no allocation). Empty
    /// for every non-Mod-Grid graph, so `process_module` skips the scan.
    input_injections: Vec<(ModuleId, PortName, f32)>,
    /// Pre-built incoming connection lookup: module_id → Vec<(from_module, from_port, to_port)>.
    /// Rebuilt when graph topology changes (order_dirty), not per frame.
    /// Each Vec is sorted in `calculate_processing_order` so input-summation
    /// order is stable — fp addition is non-associative.
    incoming_map: HashMap<ModuleId, Vec<(ModuleId, PortName, PortName)>>,
    /// Pre-allocated vec for gathering incoming connections (used as temp during rebuild).
    incoming_cache: Vec<(ModuleId, PortName, PortName)>,
    /// Cached output module ID — resolved when graph topology changes, not per frame.
    cached_output_id: Option<ModuleId>,
    /// Set of bypassed modules (outputs zeroed instead of processing).
    bypassed: HashSet<ModuleId>,
    /// Pool of reusable AudioBuffers to avoid per-frame heap allocations.
    buffer_pool: Vec<AudioBuffer>,
    /// Scratch storage for topological sort: in-degree per module. `BTreeMap`
    /// to keep the Kahn-seed iteration deterministic across instances; see
    /// `nodes`.
    topo_in_degree: BTreeMap<ModuleId, usize>,
    /// Scratch storage for topological sort: adjacency list. Same reason as
    /// `topo_in_degree`; each Vec is also sorted post-build.
    topo_adjacency: BTreeMap<ModuleId, Vec<ModuleId>>,
    /// Scratch storage for topological sort: processing queue.
    topo_queue: VecDeque<ModuleId>,
}

impl ModuleGraph {
    /// Create a new empty graph.
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            connections: HashSet::new(),
            processing_order: Vec::new(),
            instance_counters: HashMap::new(),
            order_dirty: true,
            buffer_size: synth_core::BlockSize::new(256),
            input_buffers: Vec::with_capacity(8),
            input_injections: Vec::new(),
            incoming_map: HashMap::new(),
            incoming_cache: Vec::with_capacity(16),
            cached_output_id: None,
            bypassed: HashSet::new(),
            buffer_pool: Vec::with_capacity(16),
            topo_in_degree: BTreeMap::new(),
            topo_adjacency: BTreeMap::new(),
            topo_queue: VecDeque::with_capacity(16),
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
        self.input_injections.clear();
        self.incoming_map.clear();
        self.incoming_cache.clear();
        self.cached_output_id = None;
        self.bypassed.clear();
        self.buffer_pool.clear();
        self.topo_in_degree.clear();
        self.topo_adjacency.clear();
        self.topo_queue.clear();
    }

    /// Get the next instance number for a module type.
    fn next_instance(&mut self, module_type: ModuleType) -> u16 {
        let counter = self.instance_counters.entry(module_type).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Add a module to the graph.
    pub fn add_module(&mut self, mut module: Box<dyn PolyModule>) -> ModuleId {
        let module_type = module.module_type();
        let instance = self.next_instance(module_type);
        let id = ModuleId::new(module_type, instance);

        let descriptor = module.descriptor();
        // Populate the module's generic mod-offset store (if any) from its
        // descriptor — off the audio thread, once per module instance.
        if let Some(offsets) = module.mod_offsets_mut() {
            offsets.populate(&descriptor);
        }

        // Create output buffers
        let mut outputs = HashMap::new();
        for port in &descriptor.ports {
            if port.direction == PortDirection::Output {
                outputs.insert(port.name, AudioBuffer::new(self.buffer_size.as_usize()));
            }
        }

        self.nodes.insert(
            id,
            GraphNode {
                module,
                descriptor: Arc::new(descriptor),
                outputs,
            },
        );

        self.order_dirty = true;
        id
    }

    /// Add a module with a specific ID.
    pub fn add_module_with_id(&mut self, id: ModuleId, mut module: Box<dyn PolyModule>) {
        let descriptor = module.descriptor();
        // Populate the generic mod-offset store (if any) — off the audio thread.
        if let Some(offsets) = module.mod_offsets_mut() {
            offsets.populate(&descriptor);
        }

        let mut outputs = HashMap::new();
        for port in &descriptor.ports {
            if port.direction == PortDirection::Output {
                outputs.insert(port.name, AudioBuffer::new(self.buffer_size.as_usize()));
            }
        }

        self.nodes.insert(
            id,
            GraphNode {
                module,
                descriptor: Arc::new(descriptor),
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

    /// Borrow the cached descriptor for a module.
    ///
    /// Unlike [`Describable::descriptor`](synth_core::Describable::descriptor),
    /// which rebuilds a fresh descriptor and
    /// allocates (a `Vec` of parameters plus strings), this returns the
    /// descriptor cached at `add_module` time — zero-allocation, so it is safe
    /// to call on the audio thread (e.g. to denormalize an automation value via
    /// a parameter's range). Returns `None` if the module id is absent.
    pub fn module_descriptor(&self, id: ModuleId) -> Option<&ModuleDescriptor> {
        self.nodes.get(&id).map(|node| node.descriptor.as_ref())
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
        self.nodes.get(&id).map(|n| n.descriptor.as_ref())
    }

    /// Swap a module node's cached descriptor, returning the replaced one so the
    /// caller can drop it **off the audio thread** (a `ModuleDescriptor` owns
    /// `String`s/`Vec`s whose free must not run in `process()`). RT-safe — a
    /// `mem::replace`, no allocation: the engine builds the fresh descriptor
    /// off-thread (when a script edit changes a module's `param` set) and hands it
    /// in here. Output buffers are **not** touched — a script edit changes only the
    /// param list, never the fixed port set. An absent module hands the descriptor
    /// straight back.
    #[must_use = "the replaced descriptor must be dropped off the audio thread"]
    pub fn set_node_descriptor(
        &mut self,
        id: ModuleId,
        descriptor: Arc<ModuleDescriptor>,
    ) -> Option<Arc<ModuleDescriptor>> {
        match self.nodes.get_mut(&id) {
            Some(node) => Some(std::mem::replace(&mut node.descriptor, descriptor)),
            None => Some(descriptor),
        }
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

    /// Install (or clear, with `None`) a hosted control-script slot on a module.
    /// No-op when the module is absent or hosts no scripts (the default
    /// `set_script` ignores it). The `Arc` is shared, not deep-copied. Returns the
    /// **replaced** script (if any) so the caller can drop it off the audio
    /// thread — this runs during the command drain.
    #[must_use = "the replaced script must be dropped off the audio thread"]
    pub fn set_script(
        &mut self,
        module: ModuleId,
        slot: usize,
        script: Option<std::sync::Arc<synth_core::script::BoundScript>>,
    ) -> Option<std::sync::Arc<synth_core::script::BoundScript>> {
        match self.nodes.get_mut(&module) {
            Some(node) => node.module.set_script(slot, script),
            None => script,
        }
    }

    /// Propagate the per-voice PRNG seed base to every script-hosting module in
    /// the graph, folding the `voice_index` with each module's identity so two
    /// script hosts in the same voice still decorrelate. Called once per voice at
    /// allocation, after the voice id is assigned. No-op for modules that host no
    /// scripts (default [`PolyModule::set_voice_index`]).
    pub fn set_voice_index(&mut self, voice_index: u32) {
        for (id, node) in &mut self.nodes {
            node.module
                .set_voice_index(script_seed_base(voice_index, *id));
        }
    }

    /// Set bypass state for a module.
    pub fn set_bypass(&mut self, module_id: ModuleId, bypass: bool) {
        if bypass {
            self.bypassed.insert(module_id);
        } else {
            self.bypassed.remove(&module_id);
        }
    }

    /// Check if a module is bypassed.
    #[must_use]
    pub fn is_bypassed(&self, module_id: ModuleId) -> bool {
        self.bypassed.contains(&module_id)
    }

    /// Get a parameter from a module.
    pub fn get_param(&self, module: ModuleId, param: &Param) -> Option<f32> {
        self.nodes.get(&module)?.module.get_param(param)
    }

    /// Process the graph.
    pub fn process(&mut self, output: &mut AudioBuffer, context: &ProcessContext<'_>) {
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

        // Copy from cached output module (resolved during topology change, not per frame)
        if let Some(out_id) = self.cached_output_id
            && let Some(node) = self.nodes.get(&out_id)
            && let Some(out_buf) = node.outputs.get(&PortName::OUT)
        {
            output.copy_from(out_buf);
        } else {
            output.clear();
        }
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

    /// Check if all envelope-type modules have finished releasing.
    /// Returns `true` when every module reports `is_release_done()`.
    pub fn all_releases_done(&self) -> bool {
        self.nodes
            .values()
            .all(|node| node.module.is_release_done())
    }

    /// Reset all modules.
    pub fn reset(&mut self) {
        self.nodes.values_mut().for_each(|node| node.module.reset());
    }

    /// Get a module's output buffer by port name (for extracting stereo outputs).
    /// This is useful after processing to get specific output ports.
    pub fn get_module_output(
        &self,
        module_id: ModuleId,
        port_name: PortName,
    ) -> Option<&AudioBuffer> {
        self.nodes.get(&module_id)?.outputs.get(&port_name)
    }

    /// The incoming graph connections to `id`, as `(from_module, from_port,
    /// to_port)` — the wired edges feeding this module's input ports. Backs the
    /// pre-built `incoming_map` (rebuilt only when topology changes, at the start
    /// of [`process`](Self::process)), so this is a `HashMap` lookup plus a slice
    /// borrow — no per-call scan, safe on the audio thread. Empty for a module
    /// with no incoming edges or an unknown id.
    ///
    /// The `Voice` uses it to resolve the control-ports `Script` module's
    /// `in1..in4` CV ports from their wired sources' previous-block output. After
    /// a topology change the map is one control block stale until the next
    /// `process()` rebuilds it — the same one-block latency every bound source has.
    #[must_use]
    pub fn incoming_connections(&self, id: ModuleId) -> &[(ModuleId, PortName, PortName)] {
        match self.incoming_map.get(&id) {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }

    /// Find a module by type and return its ID.
    /// Useful for finding the amplifier in a voice graph.
    pub fn find_module_by_type(&self, module_type: ModuleType) -> Option<ModuleId> {
        self.nodes
            .iter()
            .find(|(id, _)| id.module_type == module_type)
            .map(|(id, _)| *id)
    }

    /// Deliver the voice's per-block note pitch to every pitch-tracking sound
    /// source in the graph. Used by Voice to inject the (modulated) note
    /// frequency before processing.
    ///
    /// Modules opt in by overriding [`PolyModule::set_voice_pitch`]; oscillators
    /// and the sampler track it, everything else ignores it. This replaces the
    /// former `== ModuleType::Oscillator` special case — pitch tracking is now a
    /// per-module capability, so any pitched source (sub-osc, wavetable, …) can
    /// follow continuous pitch simply by implementing the trait method.
    pub fn set_voice_pitch(&mut self, pitch: synth_core::VoicePitch) {
        for node in self.nodes.values_mut() {
            node.module.set_voice_pitch(pitch);
        }
    }

    /// Get all module IDs.
    pub fn module_ids(&self) -> impl Iterator<Item = ModuleId> + '_ {
        self.nodes.keys().copied()
    }

    /// Apply a modulation offset to the module addressed by `addr`.
    ///
    /// ID-based (S1.2): resolves the exact `ModuleId` from the address's module
    /// type + 1-based instance and forwards the param identifier to the module's
    /// [`set_mod_offset`](synth_core::PolyModule::set_mod_offset). No fuzzy
    /// fallback — a dangling address (the module was removed) is a silent no-op.
    pub fn apply_mod_offset_addr(&mut self, addr: synth_core::DestAddr, value: f32) {
        let target_id = crate::ModuleId::new(addr.module_type, addr.instance);
        if let Some(node) = self.nodes.get_mut(&target_id) {
            node.module.set_mod_offset(addr.param.as_str(), value);
        }
    }

    /// Clear all modulation offsets on all modules in the graph.
    pub fn clear_mod_offsets(&mut self) {
        for node in self.nodes.values_mut() {
            node.module.clear_mod_offsets();
        }
    }

    /// Pre-create an input-injection slot for `(module, port)` (off the audio
    /// thread — allocates). Idempotent: a `(module, port)` already present is left
    /// untouched. The pre-pass then only *updates* the value with
    /// [`Self::set_input_injection_value`], never inserts, so it stays RT-safe.
    pub fn push_input_injection(&mut self, module: ModuleId, port: PortName) {
        if !self
            .input_injections
            .iter()
            .any(|(m, p, _)| *m == module && *p == port)
        {
            self.input_injections.push((module, port, 0.0));
        }
    }

    /// Set the block-constant value of a pre-created injection slot (RT-safe:
    /// linear find + write, no allocation). A `(module, port)` with no slot is a
    /// silent no-op — slots are created at build time.
    pub fn set_input_injection_value(&mut self, module: ModuleId, port: PortName, value: f32) {
        if let Some((_, _, v)) = self
            .input_injections
            .iter_mut()
            .find(|(m, p, _)| *m == module && *p == port)
        {
            *v = value;
        }
    }

    /// Apply a transient automation override to a specific module.
    ///
    /// Routes to the module with the given id and replaces the targeted
    /// parameter's effective value until [`Self::clear_param_overrides`]; the
    /// module's stored base value (set via [`Self::set_param`]) is never
    /// mutated. No-op if the module id is absent or the module does not support
    /// the override (default `PolyModule` behaviour). Real-time safe.
    pub fn apply_param_override(&mut self, module: ModuleId, param: Param) {
        if let Some(node) = self.nodes.get_mut(&module) {
            node.module.set_param_override(param);
        }
    }

    /// Clear all transient automation overrides on every module in the graph,
    /// reverting affected parameters to their base values. Called on transport
    /// stop. Real-time safe.
    pub fn clear_param_overrides(&mut self) {
        for node in self.nodes.values_mut() {
            node.module.clear_param_overrides();
        }
    }

    /// Get the last output sample value from a module (first sample of "out" port).
    ///
    /// Used to read LFO/Envelope output values for the mod matrix.
    /// Returns the first sample of the previous block (one-block latency).
    pub fn get_last_output_value(&self, module_id: ModuleId) -> f32 {
        self.nodes
            .get(&module_id)
            .and_then(|n| n.outputs.get(&PortName::OUT))
            .map(|buf| if buf.is_empty() { 0.0 } else { buf[0] })
            .unwrap_or(0.0)
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
        let mut new_graph = Self::new();
        new_graph.instance_counters = self.instance_counters.clone();
        new_graph.buffer_size = self.buffer_size;

        // Clone all modules
        for (&id, node) in &self.nodes {
            new_graph.add_module_with_id(id, node.module.box_clone());
        }

        // Clone all connections
        new_graph.connections = self.connections.clone();
        // Bypass state is part of the graph structure — voices rebuilt from a
        // template after a module is bypassed must inherit that bypass, or
        // subsequent `add_module` calls (which trigger a voice rebuild) would
        // silently unmute previously bypassed modules.
        new_graph.bypassed = self.bypassed.clone();
        new_graph.order_dirty = true;
        new_graph.prepare_realtime();

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
        // Reuse scratch fields to avoid allocations on the audio thread
        self.processing_order.clear();
        self.topo_in_degree.clear();
        for val in self.topo_adjacency.values_mut() {
            val.clear();
        }
        self.topo_queue.clear();

        // Initialize
        for &id in self.nodes.keys() {
            self.topo_in_degree.insert(id, 0);
            self.topo_adjacency.entry(id).or_default();
        }

        // Build adjacency list and in-degrees
        // Only process connections where both modules exist
        for conn in &self.connections {
            if self.topo_in_degree.contains_key(&conn.from_module)
                && self.topo_in_degree.contains_key(&conn.to_module)
            {
                if let Some(adj) = self.topo_adjacency.get_mut(&conn.from_module) {
                    adj.push(conn.to_module);
                }
                if let Some(deg) = self.topo_in_degree.get_mut(&conn.to_module) {
                    *deg += 1;
                }
            }
        }

        // Sort adjacency Vecs so neighbor traversal is independent of
        // `HashSet<Connection>` iteration order — otherwise ties in the
        // topo sort resolve differently across fresh graph instances.
        for adj in self.topo_adjacency.values_mut() {
            adj.sort_unstable();
        }

        // Find all nodes with in-degree 0.
        for (&id, &deg) in &self.topo_in_degree {
            if deg == 0 {
                self.topo_queue.push_back(id);
            }
        }

        while let Some(id) = self.topo_queue.pop_front() {
            self.processing_order.push(id);

            if let Some(neighbors) = self.topo_adjacency.get(&id) {
                for &neighbor in neighbors {
                    if let Some(deg) = self.topo_in_degree.get_mut(&neighbor) {
                        *deg -= 1;
                        if *deg == 0 {
                            self.topo_queue.push_back(neighbor);
                        }
                    }
                }
            }
        }

        // If order doesn't contain all nodes, there's a cycle
        // (shouldn't happen if validate_connection works correctly)
        if self.processing_order.len() != self.nodes.len() {
            // Fallback: just use arbitrary order
            self.processing_order.clear();
            self.processing_order.extend(self.nodes.keys().copied());
        }
        self.order_dirty = false;

        // Rebuild incoming connection map (avoids per-frame O(M×C) scan)
        self.incoming_map.clear();
        for conn in &self.connections {
            self.incoming_map
                .entry(conn.to_module)
                .or_insert_with(|| Vec::with_capacity(4))
                .push((conn.from_module, conn.from_port, conn.to_port));
        }

        // Sort so input-summation order is stable — fp addition is
        // non-associative.
        for incoming in self.incoming_map.values_mut() {
            incoming.sort_unstable();
        }

        // Cache the output module ID (avoids per-frame search)
        self.cached_output_id = self.resolve_output_module();
    }

    /// Build topology caches and seed the reusable input-buffer pool before a
    /// graph reaches the audio thread.
    fn prepare_realtime(&mut self) {
        self.calculate_processing_order();

        let required_buffers = self
            .incoming_map
            .values()
            .map(|connections| {
                connections
                    .iter()
                    .map(|(_, _, to_port)| to_port)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            })
            .max()
            .unwrap_or(0);
        while self.buffer_pool.len() < required_buffers {
            self.buffer_pool
                .push(AudioBuffer::new(self.buffer_size.as_usize()));
        }
    }

    /// Resolve which module should be used as the graph output.
    /// Called once when topology changes, not per frame.
    fn resolve_output_module(&self) -> Option<ModuleId> {
        // Priority 1: Dedicated Output category module
        if let Some((&id, _)) = self
            .nodes
            .iter()
            .find(|(_, n)| n.descriptor.category == ModuleCategory::Output)
            && self
                .nodes
                .get(&id)
                .and_then(|n| n.outputs.get(&PortName::OUT))
                .is_some()
        {
            return Some(id);
        }

        // Priority 2: Sink module with incoming connections
        let found = self
            .processing_order
            .iter()
            .rev()
            .copied()
            .filter(|&id| !self.connections.iter().any(|c| c.from_module == id))
            .find(|&id| {
                self.connections.iter().any(|c| c.to_module == id)
                    && self
                        .nodes
                        .get(&id)
                        .and_then(|n| n.outputs.get(&PortName::OUT))
                        .is_some()
            });
        if found.is_some() {
            return found;
        }

        // Priority 3: Any sink with an output port
        if !self.connections.is_empty() {
            return self
                .processing_order
                .iter()
                .rev()
                .copied()
                .filter(|&id| !self.connections.iter().any(|c| c.from_module == id))
                .find(|&id| {
                    self.nodes
                        .get(&id)
                        .and_then(|n| n.outputs.get(&PortName::OUT))
                        .is_some()
                });
        }

        None
    }

    fn process_module(&mut self, module_id: ModuleId, context: &ProcessContext<'_>) {
        // Return input buffers to the pool for reuse (avoids per-frame allocation)
        for (_, buf) in self.input_buffers.drain(..) {
            self.buffer_pool.push(buf);
        }

        // Use pre-built incoming connection map (rebuilt on topology change, not per frame)
        if let Some(incoming) = self.incoming_map.get(&module_id) {
            for &(from_module, from_port, to_port) in incoming {
                if let Some(from_node) = self.nodes.get(&from_module)
                    && let Some(output_buf) = from_node.outputs.get(&from_port)
                {
                    // Sum inputs if multiple connections to same port
                    // Linear search in Vec is fast for typical 1-4 input ports
                    if let Some((_, existing)) = self
                        .input_buffers
                        .iter_mut()
                        .find(|(name, _)| *name == to_port)
                    {
                        if existing.len() < context.samples.as_usize() {
                            existing.resize(context.samples.as_usize());
                        }
                        existing.add_from(output_buf);
                    } else {
                        let mut buf = if let Some(mut pooled) = self.buffer_pool.pop() {
                            pooled.resize(context.samples.as_usize());
                            pooled.clear();
                            pooled
                        } else {
                            AudioBuffer::new(context.samples.as_usize())
                        };
                        buf.copy_from(output_buf);
                        self.input_buffers.push((to_port, buf));
                    }
                }
            }
        }

        // Cheap Mod-Grid sources injected into this module's control input ports
        // (`Macro → LFO.rate_cv`). Added like a connection — summed into an
        // existing buffer, or filled into a fresh one. The graph validator forbids
        // a second driver on a port, so a port is either connected or injected.
        // The whole vec is empty for non-Mod-Grid graphs (the common case).
        if !self.input_injections.is_empty() {
            let n = context.samples.as_usize();
            for &(m, port, value) in &self.input_injections {
                if m != module_id {
                    continue;
                }
                if let Some((_, existing)) = self
                    .input_buffers
                    .iter_mut()
                    .find(|(name, _)| *name == port)
                {
                    if existing.len() < n {
                        existing.resize(n);
                    }
                    for s in existing.as_mut_slice() {
                        *s += value;
                    }
                } else {
                    let mut buf = if let Some(mut pooled) = self.buffer_pool.pop() {
                        pooled.resize(n);
                        pooled
                    } else {
                        AudioBuffer::new(n)
                    };
                    for s in buf.as_mut_slice() {
                        *s = value;
                    }
                    self.input_buffers.push((port, buf));
                }
            }
        }

        // Build InputPorts directly from owned buffers (zero allocation)
        let inputs = InputPorts::from_owned(&self.input_buffers);

        // Get the node and process
        if let Some(node) = self.nodes.get_mut(&module_id) {
            // Clear output buffers
            for buf in node.outputs.values_mut() {
                buf.clear();
            }

            // If bypassed, leave outputs zeroed (already cleared above)
            if self.bypassed.contains(&module_id) {
                return;
            }

            // Process
            node.module.process(inputs, &mut node.outputs, context);
        }
    }

    fn resize_buffers(&mut self, size: usize) {
        if self.buffer_size.as_usize() != size {
            self.buffer_size = synth_core::BlockSize::new(size);
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
    use synth_core::{AmplifierParam, Gain, SampleCount, SampleRate};
    use synth_modules::{Amplifier, Oscillator};

    /// F2 verification: the positional `ModuleId { module_type, instance }` is
    /// stable across removal of a same-type sibling — removing one module never
    /// renumbers the survivors, and instance numbers are monotonic (never
    /// reused), so an `AutomationTarget::Module` lane keyed on a surviving
    /// instance keeps resolving to the same module. This is why F2 (a separate
    /// stable id + on-disk migration) is unnecessary: positional identity is
    /// already non-positional-in-effect.
    #[test]
    fn module_instance_identity_is_stable_across_removal() {
        let mut graph = ModuleGraph::new();
        let osc1 = graph.add_module(Box::new(Oscillator::new()));
        let osc2 = graph.add_module(Box::new(Oscillator::new()));
        assert_eq!(osc1.instance, 1);
        assert_eq!(osc2.instance, 2);

        // Remove the first same-type module.
        graph.remove_module(osc1);
        // The survivor keeps its instance (2) — not renumbered to 1.
        assert!(graph.get_module(osc1).is_none());
        assert!(
            graph.get_module(osc2).is_some(),
            "surviving module must keep instance 2 after a sibling is removed"
        );

        // A newly added same-type module gets instance 3 (monotonic), NOT a
        // reused 1 — so it can never shadow a lane that targets instance 2.
        let osc3 = graph.add_module(Box::new(Oscillator::new()));
        assert_eq!(osc3.instance, 3, "instance numbers must not be reused");
        assert!(graph.get_module(osc2).is_some());
    }

    #[test]
    fn incoming_connections_reports_wired_edges() {
        // osc.out -> amp.in. After one process() (which rebuilds the incoming
        // map on the dirty topology), the accessor reports the amp's single
        // incoming edge; the source has none, and an unknown id is empty.
        let mut graph = ModuleGraph::new();
        let osc = graph.add_module(Box::new(Oscillator::new()));
        let amp = graph.add_module(Box::new(Amplifier::new()));
        graph
            .connect(osc, "out", amp, "in")
            .expect("osc out -> amp in");

        let ctx = ProcessContext {
            samples: SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        let mut out = AudioBuffer::new(64);
        graph.process(&mut out, &ctx);

        assert_eq!(
            graph.incoming_connections(amp),
            &[(osc, PortName::OUT, PortName::IN)]
        );
        assert!(graph.incoming_connections(osc).is_empty());
        assert!(
            graph
                .incoming_connections(ModuleId::new(ModuleType::Filter, 99))
                .is_empty()
        );
    }

    #[test]
    fn input_injection_feeds_a_module_control_port() {
        use synth_modules::Lfo;
        // An LFO at 1 Hz base rate. Injecting a value into its `rate_cv` control
        // input makes `is_connected()` true and applies FM, so the block's final
        // sample diverges from the un-injected run — proving the injection reached
        // the module's input port.
        let ctx = ProcessContext {
            samples: SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        let last_sample = |inject: Option<f32>| -> f32 {
            let mut graph = ModuleGraph::new();
            let lfo = graph.add_module(Box::new(Lfo::new()));
            graph.set_param(
                lfo,
                Param::Lfo(synth_core::LfoParam::Rate(synth_core::Hertz::new(1.0))),
            );
            if let Some(v) = inject {
                graph.push_input_injection(lfo, PortName::RATE_CV);
                graph.set_input_injection_value(lfo, PortName::RATE_CV, v);
            }
            let mut out = AudioBuffer::new(256);
            // A few blocks so the phase difference accumulates.
            for _ in 0..8 {
                graph.process(&mut out, &ctx);
            }
            graph
                .get_module_output(lfo, PortName::OUT)
                .and_then(|b| b.as_slice().last().copied())
                .unwrap_or(0.0)
        };
        let base = last_sample(None);
        let injected = last_sample(Some(1.0));
        assert!(
            (base - injected).abs() > 1e-3,
            "injected rate_cv should change the LFO output: base={base}, injected={injected}"
        );
    }

    /// `set_script` installs a slot's compiled script onto a Mod Matrix
    /// node (visible via `scripts`), clears it with `None`, and is a safe
    /// no-op for an absent module — the write half of the YAMS engine channel.
    #[test]
    fn set_script_installs_clears_and_ignores_absent() {
        use std::sync::Arc;
        use synth_core::script::{BoundScript, CompiledScript, Op};
        use synth_modules::ModMatrix;

        let mut graph = ModuleGraph::new();
        let mmx = graph.add_module(Box::new(ModMatrix::new()));

        let script = Arc::new(BoundScript::new(
            CompiledScript::new(vec![Op::PushConst(0)], vec![0.5], 0, 0),
            Vec::new(),
            "out = 0.5".to_string(),
        ));
        // Installing into an empty slot replaces nothing → returns None.
        assert!(
            graph
                .set_script(mmx, 1, Some(Arc::clone(&script)))
                .is_none()
        );

        let scripts = graph.get_module(mmx).and_then(|m| m.scripts()).unwrap();
        assert_eq!(
            scripts[1].as_deref().map(|b| b.source.as_str()),
            Some("out = 0.5")
        );
        assert!(scripts[0].is_none());

        // Clearing the slot removes it and hands the old script back to drop.
        assert!(graph.set_script(mmx, 1, None).is_some());
        assert!(graph.get_module(mmx).and_then(|m| m.scripts()).unwrap()[1].is_none());

        // An absent module is a silent no-op (no panic) and returns the unused
        // script straight back rather than swallowing it.
        let ghost = ModuleId::new(ModuleType::ModMatrix, 99);
        assert!(graph.set_script(ghost, 0, Some(script)).is_some());
    }

    #[test]
    fn test_graph_param_override_routes_to_module_and_clears() {
        // Osc (source) -> Amp (sink). Overriding the amp's level to zero must
        // silence the graph output; clearing must restore the base level.
        let mut graph = ModuleGraph::new();
        let osc_id = graph.add_module(Box::new(Oscillator::new()));
        let amp_id = graph.add_module(Box::new(Amplifier::new()));
        graph
            .connect(osc_id, "out", amp_id, "in")
            .expect("osc out -> amp in");

        let ctx = ProcessContext {
            samples: SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        fn peak(graph: &mut ModuleGraph, ctx: &ProcessContext<'_>) -> f32 {
            let mut out = AudioBuffer::new(256);
            graph.process(&mut out, ctx);
            (0..256).map(|i| out[i].abs()).fold(0.0_f32, f32::max)
        }

        let base_peak = peak(&mut graph, &ctx);
        assert!(
            base_peak > 0.01,
            "expected audible base output, got {base_peak}"
        );

        graph.apply_param_override(
            amp_id,
            Param::Amplifier(AmplifierParam::Level(Gain::new(0.0))),
        );
        // The amp level is de-zippered with a per-block ramp, so the first block
        // after the override fades down; the next block is fully silent.
        let _ = peak(&mut graph, &ctx);
        let override_peak = peak(&mut graph, &ctx);
        assert!(
            override_peak < 1e-4,
            "override should silence the amp once the ramp settles"
        );

        // The oscillator free-runs, so exact peaks drift block to block; assert
        // that clearing restores audible output rather than an exact level. One
        // settling block first (the level ramps back up).
        graph.clear_param_overrides();
        let _ = peak(&mut graph, &ctx);
        let reverted_peak = peak(&mut graph, &ctx);
        assert!(
            reverted_peak > 0.01,
            "clearing must revert to the audible base level, got {reverted_peak}"
        );
    }

    #[test]
    fn test_graph_param_override_ignores_unknown_module() {
        // Applying an override to an absent module id is a harmless no-op.
        let mut graph = ModuleGraph::new();
        let osc_id = graph.add_module(Box::new(Oscillator::new()));
        let missing = crate::ModuleId::new(ModuleType::Amplifier, 99);
        graph.apply_param_override(
            missing,
            Param::Amplifier(AmplifierParam::Level(Gain::new(0.0))),
        );
        // The real module is untouched and still present.
        assert!(graph.get_module(osc_id).is_some());
    }

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

    #[test]
    fn clone_structure_preserves_bypass() {
        let mut graph = ModuleGraph::new();
        let id = graph.add_module(Box::new(Oscillator::new()));
        graph.set_bypass(id, true);
        assert!(graph.is_bypassed(id));

        let cloned = graph.clone_structure();
        assert!(
            cloned.is_bypassed(id),
            "clone_structure must carry bypass state — otherwise a voice \
             rebuilt from a bypassed template silently un-bypasses modules"
        );
    }
}
