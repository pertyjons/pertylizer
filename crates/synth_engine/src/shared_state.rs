//! Shared state for multi-GUI communication.
//!
//! This module provides thread-safe shared state that can be read by multiple
//! GUI clients while being updated by the audio engine.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use parking_lot::RwLock;

use super::commands::ModuleId;
use super::connectivity::ModuleConnectivityStatus;
use super::instrument::InstrumentId;
use super::recording::RecordingState;
use synth_core::{
    Amplitude, BipolarValue, BypassState, CpuUsage, Gain, MuteState, PortName, SoloState,
};
use synth_core::{ModuleParam, ModuleType, Param};
use synth_sequencer::ReturnBusId;

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
    pub fn set_peak(&self, left: Amplitude, right: Amplitude) {
        self.peak_left.store(left.as_f32());
        self.peak_right.store(right.as_f32());
    }

    /// Update RMS meters.
    pub fn set_rms(&self, left: Amplitude, right: Amplitude) {
        self.rms_left.store(left.as_f32());
        self.rms_right.store(right.as_f32());
    }

    /// Get peak meter values.
    pub fn get_peak(&self) -> (Amplitude, Amplitude) {
        (
            Amplitude::new(self.peak_left.load()),
            Amplitude::new(self.peak_right.load()),
        )
    }

    /// Get RMS meter values.
    pub fn get_rms(&self) -> (Amplitude, Amplitude) {
        (
            Amplitude::new(self.rms_left.load()),
            Amplitude::new(self.rms_right.load()),
        )
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
    /// Recording state: 0=off, 1=armed, 2=count_in, 3=capturing.
    pub recording: AtomicU32,
    /// Metronome state: 0=off, 1=on.
    pub metronome: AtomicU32,
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
            recording: AtomicU32::new(0),
            metronome: AtomicU32::new(0),
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
    pub fn tempo(&self) -> synth_core::Bpm {
        synth_core::Bpm::new(self.tempo.load())
    }

    /// Get current position in samples.
    pub fn position(&self) -> u64 {
        self.position_samples.load(Ordering::Relaxed)
    }

    /// Get the recording state.
    pub fn recording_state(&self) -> RecordingState {
        RecordingState::from_u32(self.recording.load(Ordering::Relaxed))
    }

    /// Set the recording state.
    pub fn set_recording_state(&self, state: RecordingState) {
        self.recording.store(state.as_u32(), Ordering::Relaxed);
    }

    /// Check if recording is armed (waiting for play).
    pub fn is_armed(&self) -> bool {
        self.recording_state() == RecordingState::Armed
    }

    /// Check if actively capturing.
    pub fn is_recording(&self) -> bool {
        self.recording_state() == RecordingState::Capturing
    }

    /// Check if in count-in phase.
    pub fn is_count_in(&self) -> bool {
        self.recording_state() == RecordingState::CountIn
    }

    /// Check if metronome is on.
    pub fn is_metronome_on(&self) -> bool {
        self.metronome.load(Ordering::Relaxed) == 1
    }

    /// Set metronome state.
    pub fn set_metronome(&self, enabled: bool) {
        self.metronome.store(u32::from(enabled), Ordering::Relaxed);
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
    /// Free-text per-instance description (read side of the description
    /// channel, mirroring the owning instrument's `module_description` map so
    /// the save path can persist it into `ModuleState.description`). Empty when
    /// no description was set or loaded.
    pub description: String,
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
    /// Per-slot YAMS control-script source text (Step 2), keyed by 1-based slot
    /// number — the read side of the script channel, mirroring the engine
    /// module's `scripts()` so the save path can persist them. Empty for
    /// every module except a script-hosting one (e.g. a Mod Matrix) with
    /// installed scripts.
    pub scripts: BTreeMap<String, String>,
    /// Number of connections to each input port.
    pub input_connection_counts: HashMap<PortName, usize>,
    /// Number of connections from each output port.
    pub output_connection_counts: HashMap<PortName, usize>,
    /// CPU usage percentage for this module.
    pub cpu_usage: CpuUsage,
    /// Output levels per port.
    pub output_levels: HashMap<PortName, Amplitude>,
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
            description: String::new(),
            bypass_state: BypassState::Active,
            mute_state: MuteState::Unmuted,
            solo_state: SoloState::Normal,
            connectivity: ModuleConnectivityStatus::Disconnected,
            parameters: Vec::new(),
            scripts: BTreeMap::new(),
            input_connection_counts: HashMap::new(),
            output_connection_counts: HashMap::new(),
            cpu_usage: CpuUsage::default(),
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
#[derive(Debug, Clone, Copy)]
pub struct ConnectionSnapshot {
    /// Which instrument this connection belongs to.
    pub instrument_id: InstrumentId,
    /// Source module.
    pub from_module: ModuleId,
    /// Source port name.
    pub from_port: PortName,
    /// Destination module.
    pub to_module: ModuleId,
    /// Destination port name.
    pub to_port: PortName,
}

impl ConnectionSnapshot {
    /// Create a new connection snapshot.
    pub fn new(
        instrument_id: InstrumentId,
        from_module: ModuleId,
        from_port: impl Into<PortName>,
        to_module: ModuleId,
        to_port: impl Into<PortName>,
    ) -> Self {
        Self {
            instrument_id,
            from_module,
            from_port: from_port.into(),
            to_module,
            to_port: to_port.into(),
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
    /// Free-text description / intent. Empty when not set.
    pub description: String,
    /// Patch-level description, separate from the per-instrument
    /// description above. `None` when not set.
    pub patch_description: Option<String>,
    /// Optional accent color as a hex string (e.g. "#FF8800FF"). `None` when no
    /// color is set ("auto" / default tint). Read side of the color channel —
    /// the save path persists it into `InstrumentState.color`.
    pub color: Option<String>,
    /// Patch-level accent color (separate from the per-instance `color` above).
    /// `None` when unset. The save path persists it into `Patch.color`.
    pub patch_color: Option<String>,
    /// Sidechain source — when set, this instrument's
    /// sidechain-capable modules read audio from the source instrument.
    pub sidechain_source_id: Option<InstrumentId>,
    /// Instrument category (drums, bass, pad, etc.).
    pub category: crate::InstrumentCategory,
    /// MIDI channel (1-indexed), or `None` for OMNI.
    pub midi_channel: Option<synth_core::MidiChannel>,
    /// Volume.
    pub volume: Gain,
    /// Pan (-1.0 to 1.0).
    pub pan: BipolarValue,
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
    /// Effect chain slot order (module IDs in processing order).
    pub effect_chain_order: Vec<ModuleId>,
    /// Key range for keyboard splitting (low/high MIDI notes inclusive).
    pub key_range: crate::instrument::KeyRange,
    /// Transpose offset in semitones.
    pub transpose: synth_core::Semitones,
    /// Oversampling factor (Off/2x/4x).
    pub oversampling: synth_dsp::OversamplingFactor,
    /// Voice allocation mode (Poly / Mono / Legato / Unison).
    pub allocation_mode: crate::voice_allocator::AllocationMode,
    /// Strategy for stealing voices when all are busy.
    pub stealing_strategy: crate::voice_allocator::StealingStrategy,
    /// Total unison detune spread (cents), used in `Unison` allocation mode.
    pub unison_detune: synth_core::Cents,
    /// Unison stereo spread (0..1), used in `Unison` allocation mode.
    pub unison_spread: synth_core::NormalizedValue,
    /// Maximum polyphony for this instrument.
    pub max_voices: synth_core::VoiceCount,
    /// Velocity → amplitude sensitivity (0 = constant, 1 = full dynamic).
    pub velocity_amp_sensitivity: synth_core::NormalizedValue,
    /// Velocity → filter cutoff sensitivity (0 = none, 1 = full).
    pub velocity_filter_sensitivity: synth_core::NormalizedValue,
}

/// Thread-safe snapshot of graph topology.
/// Updated by audio thread, read by GUI threads.
#[derive(Debug)]
pub struct SharedGraphState {
    /// All modules in the graph, keyed by `(InstrumentId, ModuleId)` since
    /// `ModuleId` alone is not unique across instruments (e.g. each instrument
    /// can have its own `osc-1`).
    modules: RwLock<HashMap<(InstrumentId, ModuleId), ModuleStateSnapshot>>,
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
    pub fn get_module(
        &self,
        instrument_id: InstrumentId,
        id: ModuleId,
    ) -> Option<ModuleStateSnapshot> {
        self.modules.read().get(&(instrument_id, id)).cloned()
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
    pub fn has_module(&self, instrument_id: InstrumentId, id: ModuleId) -> bool {
        self.modules.read().contains_key(&(instrument_id, id))
    }

    /// Check if a module is connected to output.
    ///
    /// Lock ordering: `live_modules` before `modules`. All call sites that
    /// acquire both locks must follow this order to prevent deadlocks.
    pub fn is_live(&self, instrument_id: InstrumentId, id: ModuleId) -> bool {
        // Acquire both locks before checking to avoid TOCTOU race
        let live = self.live_modules.read();
        let modules = self.modules.read();
        live.contains(&id) && modules.contains_key(&(instrument_id, id))
    }

    /// Get connectivity status for a module.
    pub fn connectivity(
        &self,
        instrument_id: InstrumentId,
        id: ModuleId,
    ) -> ModuleConnectivityStatus {
        self.modules
            .read()
            .get(&(instrument_id, id))
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
        let mut modules = self.modules.write();
        modules.retain(|&(inst_id, _), _| inst_id != id);
        self.bump_version();
        drop(modules);
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
        self.bump_version();
        drop(all);
    }

    // === Update methods (called from main thread after receiving engine events) ===

    /// Add or update a module.
    pub fn set_module(&self, snapshot: ModuleStateSnapshot) {
        let key = (snapshot.instrument_id, snapshot.id);
        let mut modules = self.modules.write();
        modules.insert(key, snapshot);
        self.bump_version();
        drop(modules);
    }

    /// Update one module's descriptive metadata from a control thread.
    pub fn set_module_description(
        &self,
        instrument_id: InstrumentId,
        id: ModuleId,
        description: String,
    ) -> bool {
        let mut modules = self.modules.write();
        let Some(module) = modules.get_mut(&(instrument_id, id)) else {
            return false;
        };
        module.description = description;
        self.bump_version();
        true
    }

    /// Remove a module.
    pub fn remove_module(&self, instrument_id: InstrumentId, id: ModuleId) {
        let mut modules = self.modules.write();
        modules.remove(&(instrument_id, id));
        self.bump_version();
        drop(modules);
    }

    /// Update one parameter in a module snapshot.
    pub fn set_module_parameter(&self, instrument_id: InstrumentId, id: ModuleId, param: Param) {
        if let Some(module) = self.modules.write().get_mut(&(instrument_id, id))
            && let Some(current) = module
                .parameters
                .iter_mut()
                .find(|current| current.same_kind(&param))
        {
            *current = param;
            self.bump_version();
        }
    }

    /// Update one script slot in a module snapshot.
    pub fn set_module_script(
        &self,
        instrument_id: InstrumentId,
        id: ModuleId,
        slot: usize,
        source: Option<String>,
        declarations: Option<&[synth_core::script::ScriptParamDecl]>,
    ) {
        if let Some(module) = self.modules.write().get_mut(&(instrument_id, id)) {
            let key = slot.to_string();
            if let Some(source) = source {
                module.scripts.insert(key, source);
            } else {
                module.scripts.remove(&key);
            }
            if matches!(
                module.module_type,
                ModuleType::Script | ModuleType::AudioScript
            ) {
                let previous = module.parameters.clone();
                module.parameters.clear();
                if let Some(declarations) = declarations {
                    module
                        .parameters
                        .extend(declarations.iter().map(|declaration| {
                            let value = previous
                                .iter()
                                .find_map(|parameter| match parameter {
                                    Param::Script(synth_core::ScriptParam::Knob(name, value))
                                        if *name == declaration.name =>
                                    {
                                        Some(*value)
                                    }
                                    _ => None,
                                })
                                .unwrap_or(declaration.default);
                            Param::Script(synth_core::ScriptParam::Knob(declaration.name, value))
                        }));
                }
            }
            self.bump_version();
        }
    }

    /// Update bypass state for one instrument, or every matching instrument.
    pub fn set_module_bypass(
        &self,
        instrument_id: Option<InstrumentId>,
        id: ModuleId,
        bypassed: bool,
    ) {
        let mut modules = self.modules.write();
        for ((owner, module_id), module) in modules.iter_mut() {
            if *module_id == id && instrument_id.is_none_or(|target| target == *owner) {
                module.bypass_state = if bypassed {
                    BypassState::Bypassed
                } else {
                    BypassState::Active
                };
            }
        }
        self.bump_version();
    }

    /// Set all connections.
    pub fn set_connections(&self, connections: Vec<ConnectionSnapshot>) {
        let mut conns = self.connections.write();
        *conns = connections;
        self.bump_version();
        drop(conns);
    }

    /// Add a connection.
    pub fn add_connection(&self, connection: ConnectionSnapshot) {
        let mut conns = self.connections.write();
        conns.push(connection);
        self.bump_version();
        drop(conns);
    }

    /// Remove a connection.
    pub fn remove_connection(
        &self,
        from_module: ModuleId,
        from_port: &str,
        to_module: ModuleId,
        to_port: &str,
    ) {
        let mut conns = self.connections.write();
        conns.retain(|c| {
            !(c.from_module == from_module
                && c.from_port == from_port
                && c.to_module == to_module
                && c.to_port == to_port)
        });
        self.bump_version();
        drop(conns);
    }

    /// Remove one exact connection owned by an instrument.
    pub fn remove_connection_for_instrument(
        &self,
        instrument_id: InstrumentId,
        from_module: ModuleId,
        from_port: PortName,
        to_module: ModuleId,
        to_port: PortName,
    ) {
        self.connections.write().retain(|connection| {
            !(connection.instrument_id == instrument_id
                && connection.from_module == from_module
                && connection.from_port == from_port
                && connection.to_module == to_module
                && connection.to_port == to_port)
        });
        self.bump_version();
    }

    /// Remove every connection touching one module in an instrument.
    pub fn disconnect_all_for_instrument(&self, instrument_id: InstrumentId, module: ModuleId) {
        self.connections.write().retain(|connection| {
            connection.instrument_id != instrument_id
                || (connection.from_module != module && connection.to_module != module)
        });
        self.bump_version();
    }

    /// Clear all topology snapshots.
    pub fn clear(&self) {
        self.modules.write().clear();
        self.connections.write().clear();
        self.processing_order.write().clear();
        self.live_modules.write().clear();
        self.bump_version();
    }

    /// Set the processing order.
    pub fn set_processing_order(&self, order: Vec<ModuleId>) {
        let mut proc_order = self.processing_order.write();
        *proc_order = order;
        self.bump_version();
        drop(proc_order);
    }

    /// Set live modules.
    pub fn set_live_modules(&self, live: HashSet<ModuleId>) {
        let mut live_mods = self.live_modules.write();
        *live_mods = live;
        self.bump_version();
        drop(live_mods);
    }

    /// Update a module's connectivity status.
    pub fn update_connectivity(
        &self,
        instrument_id: InstrumentId,
        id: ModuleId,
        status: ModuleConnectivityStatus,
    ) {
        if let Some(module) = self.modules.write().get_mut(&(instrument_id, id)) {
            module.connectivity = status;
            self.bump_version();
        }
    }

    /// Update a module's CPU usage.
    pub fn update_cpu_usage(&self, instrument_id: InstrumentId, id: ModuleId, usage: CpuUsage) {
        if let Some(module) = self.modules.write().get_mut(&(instrument_id, id)) {
            module.cpu_usage = usage;
            // Don't bump version for CPU updates (too frequent)
        }
    }

    /// Update a module's output level.
    pub fn update_output_level(
        &self,
        instrument_id: InstrumentId,
        id: ModuleId,
        port: PortName,
        level: Amplitude,
    ) {
        if let Some(module) = self.modules.write().get_mut(&(instrument_id, id)) {
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

/// One effect in a return bus's chain, captured for persistence.
#[derive(Debug, Clone)]
pub struct ReturnEffectSnapshot {
    /// Module id (type + instance).
    pub module_id: ModuleId,
    /// Effect type.
    pub module_type: ModuleType,
    /// Current parameter values (each `Param` carries its type + value).
    pub parameters: Vec<Param>,
    /// Whether the effect is bypassed.
    pub bypassed: bool,
}

/// A return bus's effect chain in processing order, captured off the audio
/// thread for persistence (the save path reads this; the audio thread
/// publishes it on every return-effect mutation).
#[derive(Debug, Clone)]
pub struct ReturnBusSnapshot {
    /// Return bus id.
    pub id: ReturnBusId,
    /// Effects in chain (processing) order.
    pub effects: Vec<ReturnEffectSnapshot>,
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
        let val = AtomicF32::new(1.5);
        assert!((val.load() - 1.5).abs() < 0.001);

        val.store(4.25);
        assert!((val.load() - 4.25).abs() < 0.001);
    }

    #[test]
    fn test_meter_state() {
        let meters = SharedMeterState::new();
        meters.set_peak(Amplitude::new(0.8), Amplitude::new(0.7));
        let (l, r) = meters.get_peak();
        assert!((l.as_f32() - 0.8).abs() < 0.001);
        assert!((r.as_f32() - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_shared_graph_state() {
        use super::super::instrument::InstrumentId;

        let graph = SharedGraphState::new();
        let osc_id = ModuleId::new(ModuleType::Oscillator, 1);

        let module = ModuleStateSnapshot::new(
            osc_id,
            InstrumentId::FIRST,
            ModuleType::Oscillator,
            "Osc 1".to_string(),
        );

        graph.set_module(module);
        assert_eq!(graph.module_count(), 1);
        assert!(graph.has_module(InstrumentId::FIRST, osc_id));

        graph.remove_module(InstrumentId::FIRST, osc_id);
        assert_eq!(graph.module_count(), 0);
    }

    #[test]
    fn test_shared_graph_modules_unique_per_instrument() {
        use super::super::instrument::InstrumentId;

        let graph = SharedGraphState::new();
        let osc_id = ModuleId::new(ModuleType::Oscillator, 1);
        let inst_a = InstrumentId::FIRST;
        let inst_b = InstrumentId::new(2);

        // Two instruments, each with osc-1
        graph.set_module(ModuleStateSnapshot::new(
            osc_id,
            inst_a,
            ModuleType::Oscillator,
            "Osc A".to_string(),
        ));
        graph.set_module(ModuleStateSnapshot::new(
            osc_id,
            inst_b,
            ModuleType::Oscillator,
            "Osc B".to_string(),
        ));

        // Both should exist independently
        assert_eq!(graph.module_count(), 2);
        assert!(graph.has_module(inst_a, osc_id));
        assert!(graph.has_module(inst_b, osc_id));

        let a = graph.get_module(inst_a, osc_id).expect("inst_a module");
        assert_eq!(a.name, "Osc A");

        let b = graph.get_module(inst_b, osc_id).expect("inst_b module");
        assert_eq!(b.name, "Osc B");

        // Remove one, other should remain
        graph.remove_module(inst_a, osc_id);
        assert_eq!(graph.module_count(), 1);
        assert!(!graph.has_module(inst_a, osc_id));
        assert!(graph.has_module(inst_b, osc_id));
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
            "out",
            ModuleId::new(ModuleType::Filter, 1),
            "in",
        ));

        let v2 = graph.version();
        assert!(v2 > v1);
    }
}
