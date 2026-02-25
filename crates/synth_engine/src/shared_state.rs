//! Shared state for multi-GUI communication.
//!
//! This module provides thread-safe shared state that can be read by multiple
//! GUI clients while being updated by the audio engine.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use parking_lot::RwLock;

use super::commands::ModuleId;
use super::connectivity::ModuleConnectivityStatus;
use super::instrument::InstrumentId;
use synth_core::{BypassState, MuteState, SoloState};
use synth_core::{ModuleType, Param};

/// Atomic f32 wrapper for lock-free meter access.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    /// Create a new atomic f32 with the given initial value.
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    /// Load the current value.
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    /// Store a new value.
    pub fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }

    /// Load with acquire ordering.
    pub fn load_acquire(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Acquire))
    }

    /// Store with release ordering.
    pub fn store_release(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Release);
    }
}

impl Default for AtomicF32 {
    fn default() -> Self {
        Self::new(0.0)
    }
}

impl Clone for AtomicF32 {
    fn clone(&self) -> Self {
        Self::new(self.load())
    }
}

/// Thread-safe meter state for multi-GUI sharing.
///
/// For simpler single-GUI metering, see `engine::state::MeterState`.
#[derive(Debug, Default)]
pub struct SharedMeterState {
    pub peak_left: AtomicF32,
    pub peak_right: AtomicF32,
    pub rms_left: AtomicF32,
    pub rms_right: AtomicF32,
}

impl SharedMeterState {
    /// Create new meter state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Update peak meters.
    pub fn set_peak(&self, left: f32, right: f32) {
        self.peak_left.store(left);
        self.peak_right.store(right);
    }

    /// Update RMS meters.
    pub fn set_rms(&self, left: f32, right: f32) {
        self.rms_left.store(left);
        self.rms_right.store(right);
    }

    /// Get peak meter values.
    pub fn get_peak(&self) -> (f32, f32) {
        (self.peak_left.load(), self.peak_right.load())
    }

    /// Get RMS meter values.
    pub fn get_rms(&self) -> (f32, f32) {
        (self.rms_left.load(), self.rms_right.load())
    }
}

impl Clone for SharedMeterState {
    fn clone(&self) -> Self {
        Self {
            peak_left: self.peak_left.clone(),
            peak_right: self.peak_right.clone(),
            rms_left: self.rms_left.clone(),
            rms_right: self.rms_right.clone(),
        }
    }
}

/// Transport state for multi-GUI sharing with loop support.
///
/// This version supports play/pause/stop states and loop markers.
/// For simpler transport, see `engine::state::TransportState`.
#[derive(Debug)]
pub struct SharedTransportState {
    /// Whether playback is active.
    pub playing: AtomicU32, // 0 = stopped, 1 = playing, 2 = paused
    /// Current tempo in BPM.
    pub tempo: AtomicF32,
    /// Current position in samples.
    pub position_samples: AtomicU64,
    /// Whether loop is enabled.
    pub loop_enabled: AtomicU32,
    /// Loop start position in samples.
    pub loop_start: AtomicU64,
    /// Loop end position in samples.
    pub loop_end: AtomicU64,
}

impl SharedTransportState {
    /// Create new transport state.
    pub fn new() -> Self {
        Self {
            playing: AtomicU32::new(0),
            tempo: AtomicF32::new(120.0),
            position_samples: AtomicU64::new(0),
            loop_enabled: AtomicU32::new(0),
            loop_start: AtomicU64::new(0),
            loop_end: AtomicU64::new(0),
        }
    }

    /// Check if playing.
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed) == 1
    }

    /// Check if paused.
    pub fn is_paused(&self) -> bool {
        self.playing.load(Ordering::Relaxed) == 2
    }

    /// Get current tempo.
    pub fn tempo(&self) -> f32 {
        self.tempo.load()
    }

    /// Get current position in samples.
    pub fn position(&self) -> u64 {
        self.position_samples.load(Ordering::Relaxed)
    }
}

impl Default for SharedTransportState {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of a module's state for GUI display.
#[derive(Debug, Clone)]
pub struct ModuleStateSnapshot {
    /// Module ID.
    pub id: ModuleId,
    /// Which instrument this module belongs to.
    pub instrument_id: InstrumentId,
    /// Module type.
    pub module_type: ModuleType,
    /// Module name.
    pub name: String,
    /// Whether module is bypassed.
    pub bypass_state: BypassState,
    /// Whether module is muted.
    pub mute_state: MuteState,
    /// Whether module is soloed.
    pub solo_state: SoloState,
    /// Connectivity status.
    pub connectivity: ModuleConnectivityStatus,
    /// Current parameter values (each Param contains both type and value).
    pub parameters: Vec<Param>,
    /// Number of connections to each input port.
    pub input_connection_counts: HashMap<String, usize>,
    /// Number of connections from each output port.
    pub output_connection_counts: HashMap<String, usize>,
    /// CPU usage percentage for this module.
    pub cpu_usage: f32,
    /// Output levels per port.
    pub output_levels: HashMap<String, f32>,
}

impl ModuleStateSnapshot {
    /// Create a new module state snapshot.
    pub fn new(
        id: ModuleId,
        instrument_id: InstrumentId,
        module_type: ModuleType,
        name: String,
    ) -> Self {
        Self {
            id,
            instrument_id,
            module_type,
            name,
            bypass_state: BypassState::Active,
            mute_state: MuteState::Unmuted,
            solo_state: SoloState::Normal,
            connectivity: ModuleConnectivityStatus::Disconnected,
            parameters: Vec::new(),
            input_connection_counts: HashMap::new(),
            output_connection_counts: HashMap::new(),
            cpu_usage: 0.0,
            output_levels: HashMap::new(),
        }
    }

    /// Check if module has any input connections.
    pub fn has_inputs(&self) -> bool {
        self.input_connection_counts.values().any(|&c| c > 0)
    }

    /// Check if module has any output connections.
    pub fn has_outputs(&self) -> bool {
        self.output_connection_counts.values().any(|&c| c > 0)
    }

    /// Get total number of connections.
    pub fn total_connections(&self) -> usize {
        self.input_connection_counts.values().sum::<usize>()
            + self.output_connection_counts.values().sum::<usize>()
    }
}

/// Snapshot of a connection for GUI display.
#[derive(Debug, Clone)]
pub struct ConnectionSnapshot {
    /// Which instrument this connection belongs to.
    pub instrument_id: InstrumentId,
    /// Source module.
    pub from_module: ModuleId,
    /// Source port name.
    pub from_port: String,
    /// Destination module.
    pub to_module: ModuleId,
    /// Destination port name.
    pub to_port: String,
}

impl ConnectionSnapshot {
    /// Create a new connection snapshot.
    pub fn new(
        instrument_id: InstrumentId,
        from_module: ModuleId,
        from_port: String,
        to_module: ModuleId,
        to_port: String,
    ) -> Self {
        Self {
            instrument_id,
            from_module,
            from_port,
            to_module,
            to_port,
        }
    }
}

/// Snapshot of an instrument's metadata for MCP/GUI access.
#[derive(Debug, Clone)]
pub struct InstrumentSnapshot {
    /// Instrument ID.
    pub id: InstrumentId,
    /// Instrument name.
    pub name: String,
    /// MIDI channel (1-indexed).
    pub midi_channel: u8,
    /// Volume (0.0-2.0).
    pub volume: f32,
    /// Pan (-1.0 to 1.0).
    pub pan: f32,
    /// Whether the instrument is enabled (not muted).
    pub enabled: bool,
    /// Whether the instrument is muted.
    pub muted: bool,
    /// Whether the instrument is soloed.
    pub solo: bool,
    /// Number of modules in the voice graph.
    pub module_count: usize,
    /// Number of effects in the chain.
    pub effect_count: usize,
}

/// Thread-safe snapshot of graph topology.
/// Updated by audio thread, read by GUI threads.
#[derive(Debug)]
pub struct SharedGraphState {
    /// All modules in the graph.
    modules: RwLock<HashMap<ModuleId, ModuleStateSnapshot>>,
    /// All connections.
    connections: RwLock<Vec<ConnectionSnapshot>>,
    /// Processing order.
    processing_order: RwLock<Vec<ModuleId>>,
    /// Modules that are "live" (connected to output).
    live_modules: RwLock<HashSet<ModuleId>>,
    /// Version counter - incremented on any topology change.
    version: AtomicU64,
}

impl SharedGraphState {
    /// Create new shared graph state.
    pub fn new() -> Self {
        Self {
            modules: RwLock::new(HashMap::new()),
            connections: RwLock::new(Vec::new()),
            processing_order: RwLock::new(Vec::new()),
            live_modules: RwLock::new(HashSet::new()),
            version: AtomicU64::new(0),
        }
    }

    /// Get the current version number.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Increment the version (call after any change).
    pub fn bump_version(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Get snapshot of a specific module.
    pub fn get_module(&self, id: ModuleId) -> Option<ModuleStateSnapshot> {
        self.modules.read().get(&id).cloned()
    }

    /// Get all modules with their states.
    pub fn get_all_modules(&self) -> Vec<ModuleStateSnapshot> {
        self.modules.read().values().cloned().collect()
    }

    /// Get module count.
    pub fn module_count(&self) -> usize {
        self.modules.read().len()
    }

    /// Check if a module exists.
    pub fn has_module(&self, id: ModuleId) -> bool {
        self.modules.read().contains_key(&id)
    }

    /// Check if a module is connected to output.
    pub fn is_live(&self, id: ModuleId) -> bool {
        self.live_modules.read().contains(&id)
    }

    /// Get connectivity status for a module.
    pub fn connectivity(&self, id: ModuleId) -> ModuleConnectivityStatus {
        self.modules
            .read()
            .get(&id)
            .map(|m| m.connectivity)
            .unwrap_or(ModuleConnectivityStatus::Disconnected)
    }

    /// Get all connections.
    pub fn get_connections(&self) -> Vec<ConnectionSnapshot> {
        self.connections.read().clone()
    }

    /// Get connection count.
    pub fn connection_count(&self) -> usize {
        self.connections.read().len()
    }

    /// Get the processing order.
    pub fn get_processing_order(&self) -> Vec<ModuleId> {
        self.processing_order.read().clone()
    }

    /// Get all live modules.
    pub fn get_live_modules(&self) -> HashSet<ModuleId> {
        self.live_modules.read().clone()
    }

    // === Per-instrument query methods ===

    /// Get modules belonging to a specific instrument.
    pub fn get_modules_for_instrument(&self, id: InstrumentId) -> Vec<ModuleStateSnapshot> {
        self.modules
            .read()
            .values()
            .filter(|m| m.instrument_id == id)
            .cloned()
            .collect()
    }

    /// Get connections belonging to a specific instrument.
    pub fn get_connections_for_instrument(&self, id: InstrumentId) -> Vec<ConnectionSnapshot> {
        self.connections
            .read()
            .iter()
            .filter(|c| c.instrument_id == id)
            .cloned()
            .collect()
    }

    /// Count modules belonging to a specific instrument.
    pub fn module_count_for_instrument(&self, id: InstrumentId) -> usize {
        self.modules
            .read()
            .values()
            .filter(|m| m.instrument_id == id)
            .count()
    }

    /// Remove all modules belonging to a specific instrument.
    pub fn remove_modules_for_instrument(&self, id: InstrumentId) {
        self.modules.write().retain(|_, m| m.instrument_id != id);
        self.bump_version();
    }

    /// Replace all connections for a specific instrument, keeping connections for other instruments.
    pub fn set_connections_for_instrument(
        &self,
        id: InstrumentId,
        connections: Vec<ConnectionSnapshot>,
    ) {
        let mut all = self.connections.write();
        all.retain(|c| c.instrument_id != id);
        all.extend(connections);
        drop(all);
        self.bump_version();
    }

    // === Update methods (called from main thread after receiving engine events) ===

    /// Add or update a module.
    pub fn set_module(&self, snapshot: ModuleStateSnapshot) {
        self.modules.write().insert(snapshot.id, snapshot);
        self.bump_version();
    }

    /// Remove a module.
    pub fn remove_module(&self, id: ModuleId) {
        self.modules.write().remove(&id);
        self.bump_version();
    }

    /// Set all connections.
    pub fn set_connections(&self, connections: Vec<ConnectionSnapshot>) {
        *self.connections.write() = connections;
        self.bump_version();
    }

    /// Add a connection.
    pub fn add_connection(&self, connection: ConnectionSnapshot) {
        self.connections.write().push(connection);
        self.bump_version();
    }

    /// Remove a connection.
    pub fn remove_connection(
        &self,
        from_module: ModuleId,
        from_port: &str,
        to_module: ModuleId,
        to_port: &str,
    ) {
        self.connections.write().retain(|c| {
            !(c.from_module == from_module
                && c.from_port == from_port
                && c.to_module == to_module
                && c.to_port == to_port)
        });
        self.bump_version();
    }

    /// Set the processing order.
    pub fn set_processing_order(&self, order: Vec<ModuleId>) {
        *self.processing_order.write() = order;
        self.bump_version();
    }

    /// Set live modules.
    pub fn set_live_modules(&self, live: HashSet<ModuleId>) {
        *self.live_modules.write() = live;
        self.bump_version();
    }

    /// Update a module's connectivity status.
    pub fn update_connectivity(&self, id: ModuleId, status: ModuleConnectivityStatus) {
        if let Some(module) = self.modules.write().get_mut(&id) {
            module.connectivity = status;
            self.bump_version();
        }
    }

    /// Update a module's CPU usage.
    pub fn update_cpu_usage(&self, id: ModuleId, usage: f32) {
        if let Some(module) = self.modules.write().get_mut(&id) {
            module.cpu_usage = usage;
            // Don't bump version for CPU updates (too frequent)
        }
    }

    /// Update a module's output level.
    pub fn update_output_level(&self, id: ModuleId, port: String, level: f32) {
        if let Some(module) = self.modules.write().get_mut(&id) {
            module.output_levels.insert(port, level);
            // Don't bump version for level updates (too frequent)
        }
    }
}

impl Default for SharedGraphState {
    fn default() -> Self {
        Self::new()
    }
}

/// Complete shared engine state for multi-GUI access.
#[derive(Debug)]
pub struct SharedEngineState {
    /// Meter state (atomic, updated frequently).
    pub meters: SharedMeterState,
    /// Voice count.
    pub voice_count: AtomicU32,
    /// Overall CPU usage.
    pub cpu_usage: AtomicF32,
    /// Transport state.
    pub transport: SharedTransportState,
    /// Graph topology.
    pub graph: SharedGraphState,
    /// Instrument metadata snapshots.
    pub instrument_snapshots: RwLock<Vec<InstrumentSnapshot>>,
    /// Global version counter for any state change.
    pub version: AtomicU64,
}

impl SharedEngineState {
    /// Create new shared engine state.
    pub fn new() -> Self {
        Self {
            meters: SharedMeterState::new(),
            voice_count: AtomicU32::new(0),
            cpu_usage: AtomicF32::new(0.0),
            transport: SharedTransportState::new(),
            graph: SharedGraphState::new(),
            instrument_snapshots: RwLock::new(Vec::new()),
            version: AtomicU64::new(0),
        }
    }

    /// Create as Arc for sharing.
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Get current voice count.
    pub fn voice_count(&self) -> u32 {
        self.voice_count.load(Ordering::Relaxed)
    }

    /// Get current CPU usage.
    pub fn cpu_usage(&self) -> f32 {
        self.cpu_usage.load()
    }

    /// Check if state changed since last check.
    pub fn changed_since(&self, last_version: u64) -> bool {
        self.version.load(Ordering::Acquire) > last_version
    }

    /// Get current version.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// Bump the global version.
    pub fn bump_version(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }
}

impl Default for SharedEngineState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_f32() {
        let val = AtomicF32::new(3.14);
        assert!((val.load() - 3.14).abs() < 0.001);

        val.store(2.71);
        assert!((val.load() - 2.71).abs() < 0.001);
    }

    #[test]
    fn test_meter_state() {
        let meters = SharedMeterState::new();
        meters.set_peak(0.8, 0.7);
        let (l, r) = meters.get_peak();
        assert!((l - 0.8).abs() < 0.001);
        assert!((r - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_shared_graph_state() {
        use super::super::instrument::InstrumentId;

        let graph = SharedGraphState::new();

        let module = ModuleStateSnapshot::new(
            ModuleId::new(ModuleType::Oscillator, 1),
            InstrumentId::FIRST,
            ModuleType::Oscillator,
            "Osc 1".to_string(),
        );

        graph.set_module(module);
        assert_eq!(graph.module_count(), 1);
        assert!(graph.has_module(ModuleId::new(ModuleType::Oscillator, 1)));

        graph.remove_module(ModuleId::new(ModuleType::Oscillator, 1));
        assert_eq!(graph.module_count(), 0);
    }

    #[test]
    fn test_shared_engine_state() {
        let state = SharedEngineState::new();
        state.voice_count.store(4, Ordering::Relaxed);
        state.cpu_usage.store(25.0);

        assert_eq!(state.voice_count(), 4);
        assert!((state.cpu_usage() - 25.0).abs() < 0.001);
    }

    #[test]
    fn test_version_tracking() {
        use super::super::instrument::InstrumentId;

        let graph = SharedGraphState::new();
        let v1 = graph.version();

        graph.add_connection(ConnectionSnapshot::new(
            InstrumentId::FIRST,
            ModuleId::new(ModuleType::Oscillator, 1),
            "out".to_string(),
            ModuleId::new(ModuleType::Filter, 1),
            "in".to_string(),
        ));

        let v2 = graph.version();
        assert!(v2 > v1);
    }
}
