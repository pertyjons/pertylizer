//! `SynthSession` — shared control layer for GUI, MCP, and future interfaces.
//!
//! Owns the module lifecycle (create, remove, connect) and provides a
//! thread-safe API used by both the GUI thread and the MCP bridge.
//! In headless mode (`--mcp`) there is no GUI, so `SynthSession` is the
//! sole owner of module state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use synth_core::{BipolarValue, Gain, ModuleCategory, ModuleDescriptor, ModuleType, PortName};
use synth_engine::commands::{EffectType, PortId};
use synth_engine::instrument::{Instrument, InstrumentId, MidiChannel};
use synth_engine::shared_state::InstrumentSnapshot;
use synth_engine::state::EngineState;
use synth_engine::{CommandSender, EngineCommand, ModuleId};

use crate::module_factory;
use crate::patch::{ParamValue, Patch};

/// Error type for session operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("unsupported module type: {0}")]
    UnsupportedModuleType(String),

    #[error("module not found: {0}")]
    ModuleNotFound(String),

    #[error("instrument not found: {0}")]
    InstrumentNotFound(u64),

    #[error("visualizer modules require GUI (VisualizationBuffer)")]
    VisualizerRequiresGui,

    #[error("parameter not found: {0}")]
    ParameterNotFound(String),

    #[error("failed to send engine command")]
    SendFailed,
}

/// Thread-safe session that owns the module lifecycle.
///
/// Both GUI and MCP call into this to add/remove modules and connections.
/// The session keeps its own registry so that callers get immediate feedback
/// (e.g. the assigned `ModuleId`) without waiting for the audio thread.
pub struct SynthSession {
    command_sender: CommandSender,
    state: Arc<EngineState>,
    /// Per-instrument instance counters for ID generation.
    /// Key: (instrument_id, module_type) → next instance number.
    /// This means each instrument has its own counter per type,
    /// so instrument 0 can have osc-1 and instrument 1 can also have osc-1.
    counters: Mutex<HashMap<(InstrumentId, ModuleType), u16>>,
    /// Registry of all modules currently managed by this session.
    /// Key: (instrument_id, module_id) → descriptor.
    registry: Mutex<HashMap<(InstrumentId, ModuleId), ModuleDescriptor>>,
    /// Next instrument ID (starts at 1 since 0 is the default).
    instrument_counter: Mutex<u64>,
}

impl SynthSession {
    /// Create a new session.
    pub fn new(command_sender: CommandSender, state: Arc<EngineState>) -> Self {
        Self {
            command_sender,
            state,
            counters: Mutex::new(HashMap::new()),
            registry: Mutex::new(HashMap::new()),
            instrument_counter: Mutex::new(1), // 0 is reserved for default
        }
    }

    // ------------------------------------------------------------------
    // Instrument lifecycle
    // ------------------------------------------------------------------

    /// Create a new instrument and send it to the engine.
    /// Returns the assigned `InstrumentId`.
    pub fn add_instrument(&self, name: &str) -> Result<InstrumentId, SessionError> {
        let id = {
            let mut counter = self
                .instrument_counter
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let id = InstrumentId::new(*counter);
            *counter += 1;
            id
        };

        let instrument = Box::new(Instrument::new(id, name));

        if !self
            .command_sender
            .send(EngineCommand::AddInstrument { instrument })
        {
            return Err(SessionError::SendFailed);
        }

        Ok(id)
    }

    /// Create an instrument with a predetermined ID.
    ///
    /// Used when loading projects to preserve saved instrument IDs.
    /// Updates the instrument counter so future `add_instrument` calls
    /// won't collide.
    pub fn add_instrument_with_id(&self, id: InstrumentId, name: &str) -> Result<(), SessionError> {
        // Ensure counter is above this ID
        {
            let mut counter = self
                .instrument_counter
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let val = id.as_u64() + 1;
            if val > *counter {
                *counter = val;
            }
        }

        let instrument = Box::new(Instrument::new(id, name));

        if !self
            .command_sender
            .send(EngineCommand::AddInstrument { instrument })
        {
            return Err(SessionError::SendFailed);
        }

        Ok(())
    }

    /// Reset module instance counters for an instrument.
    ///
    /// Call this before reloading a patch into an instrument so that
    /// the counter state is clean and `add_module_with_id` updates
    /// them correctly from the loaded module IDs.
    pub fn reset_counters_for_instrument(&self, instrument_id: InstrumentId) {
        let mut counters = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        counters.retain(|&(inst_id, _), _| inst_id != instrument_id);
    }

    /// Remove an instrument from the engine.
    pub fn remove_instrument(&self, instrument_id: InstrumentId) -> Result<(), SessionError> {
        // Remove all modules belonging to this instrument from the registry
        {
            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry.retain(|&(inst_id, _), _| inst_id != instrument_id);
        }
        // Remove per-instrument counters
        {
            let mut counters = self.counters.lock().unwrap_or_else(|e| e.into_inner());
            counters.retain(|&(inst_id, _), _| inst_id != instrument_id);
        }

        if !self
            .command_sender
            .send(EngineCommand::RemoveInstrument { instrument_id })
        {
            return Err(SessionError::SendFailed);
        }

        Ok(())
    }

    /// Rename an instrument via engine command.
    pub fn rename_instrument(
        &self,
        instrument_id: InstrumentId,
        name: &str,
    ) -> Result<(), SessionError> {
        if !self.command_sender.send(EngineCommand::RenameInstrument {
            instrument_id,
            name: name.to_string(),
        }) {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set instrument volume.
    pub fn set_instrument_volume(
        &self,
        instrument_id: InstrumentId,
        volume: Gain,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::Volume(volume),
            })
        {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set instrument pan.
    pub fn set_instrument_pan(
        &self,
        instrument_id: InstrumentId,
        pan: BipolarValue,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::Pan(pan),
            })
        {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set instrument mute state.
    pub fn set_instrument_mute(
        &self,
        instrument_id: InstrumentId,
        muted: bool,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentEnabled {
                instrument_id,
                enabled: !muted,
            })
        {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set instrument enabled state.
    pub fn set_instrument_enabled(
        &self,
        instrument_id: InstrumentId,
        enabled: bool,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentEnabled {
                instrument_id,
                enabled,
            })
        {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set instrument category.
    pub fn set_instrument_category(
        &self,
        instrument_id: InstrumentId,
        category: synth_engine::InstrumentCategory,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentCategory {
                instrument_id,
                category,
            })
        {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set instrument solo state.
    pub fn set_instrument_solo(
        &self,
        instrument_id: InstrumentId,
        solo: bool,
    ) -> Result<(), SessionError> {
        if !self.command_sender.send(EngineCommand::SetInstrumentSolo {
            instrument_id,
            solo,
        }) {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set instrument MIDI channel.
    pub fn set_instrument_midi_channel(
        &self,
        instrument_id: InstrumentId,
        channel: MidiChannel,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentMidiChannel {
                instrument_id,
                channel,
            })
        {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// List all instruments from shared state.
    pub fn list_instruments(&self) -> Vec<InstrumentSnapshot> {
        self.state.instrument_snapshots.read().clone()
    }

    // ------------------------------------------------------------------
    // Module lifecycle
    // ------------------------------------------------------------------

    /// Add a module, auto-generating its ID.
    ///
    /// Returns the assigned `ModuleId` and its `ModuleDescriptor`.
    /// Visualizer modules are rejected — they need a `VisualizationBuffer`
    /// that only the GUI can provide.
    pub fn add_module(
        &self,
        instrument_id: InstrumentId,
        module_type: ModuleType,
    ) -> Result<(ModuleId, ModuleDescriptor), SessionError> {
        if module_type.is_visualizer() {
            return Err(SessionError::VisualizerRequiresGui);
        }

        let module_id = self.next_module_id(instrument_id, module_type);

        let descriptor = self.create_and_send(instrument_id, module_id, module_type)?;

        Ok((module_id, descriptor))
    }

    /// Add a module with a pre-determined ID (used when loading patches).
    ///
    /// Also updates the instance counter so that future `add_module` calls
    /// won't collide with this ID.
    pub fn add_module_with_id(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
        module_type: ModuleType,
    ) -> Result<ModuleDescriptor, SessionError> {
        if module_type.is_visualizer() {
            return Err(SessionError::VisualizerRequiresGui);
        }

        // Ensure counter is at least as high as this instance
        {
            let mut counters = self.counters.lock().unwrap_or_else(|e| e.into_inner());
            let counter = counters.entry((instrument_id, module_type)).or_insert(0);
            if module_id.instance > *counter {
                *counter = module_id.instance;
            }
        }

        self.create_and_send(instrument_id, module_id, module_type)
    }

    /// Remove a module. Looks up its category in the registry to send
    /// the correct engine command (`RemoveModule` / `RemoveEffect`).
    pub fn remove_module(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
    ) -> Result<(), SessionError> {
        let category = {
            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry
                .remove(&(instrument_id, module_id))
                .map(|d| d.category)
                .ok_or_else(|| SessionError::ModuleNotFound(module_id.to_string()))?
        };

        let cmd = match category {
            ModuleCategory::Effect => EngineCommand::RemoveEffect {
                instrument_id: Some(instrument_id),
                id: module_id,
            },
            ModuleCategory::Visualizer => EngineCommand::RemoveVisualizer {
                instrument_id: Some(instrument_id),
                id: module_id,
            },
            _ => EngineCommand::RemoveModule {
                instrument_id: Some(instrument_id),
                id: module_id,
            },
        };

        if !self.command_sender.send(cmd) {
            return Err(SessionError::SendFailed);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Connections
    // ------------------------------------------------------------------

    /// Connect two ports.
    pub fn connect(
        &self,
        instrument_id: InstrumentId,
        from_module: ModuleId,
        from_port: impl Into<PortName>,
        to_module: ModuleId,
        to_port: impl Into<PortName>,
    ) -> Result<(), SessionError> {
        if !self.command_sender.send(EngineCommand::Connect {
            instrument_id: Some(instrument_id),
            from: PortId::new(from_module, from_port),
            to: PortId::new(to_module, to_port),
        }) {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Disconnect two ports.
    pub fn disconnect(
        &self,
        instrument_id: InstrumentId,
        from_module: ModuleId,
        from_port: impl Into<PortName>,
        to_module: ModuleId,
        to_port: impl Into<PortName>,
    ) -> Result<(), SessionError> {
        if !self.command_sender.send(EngineCommand::Disconnect {
            instrument_id: Some(instrument_id),
            from: PortId::new(from_module, from_port),
            to: PortId::new(to_module, to_port),
        }) {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Clear the entire graph for an instrument — removes only modules for that instrument.
    pub fn clear_graph(&self, instrument_id: InstrumentId) -> Result<(), SessionError> {
        let modules: Vec<(ModuleId, ModuleCategory)> = {
            let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry
                .iter()
                .filter(|&(&(inst_id, _), _)| inst_id == instrument_id)
                .map(|(&(_, mod_id), desc)| (mod_id, desc.category))
                .collect()
        };

        for (module_id, category) in &modules {
            let cmd = match category {
                ModuleCategory::Effect => EngineCommand::RemoveEffect {
                    instrument_id: Some(instrument_id),
                    id: *module_id,
                },
                ModuleCategory::Visualizer => EngineCommand::RemoveVisualizer {
                    instrument_id: Some(instrument_id),
                    id: *module_id,
                },
                _ => EngineCommand::RemoveModule {
                    instrument_id: Some(instrument_id),
                    id: *module_id,
                },
            };
            self.command_sender.send(cmd);
        }

        // Clear registry entries for this instrument
        {
            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry.retain(|&(inst_id, _), _| inst_id != instrument_id);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Register a module descriptor in the session registry without creating it.
    ///
    /// Use this for GUI-created modules (visualizers, signal monitors) that bypass
    /// the normal `add_module` flow but still need to be tracked by the session
    /// so that reconciliation doesn't remove them.
    pub fn register_descriptor(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
        descriptor: ModuleDescriptor,
    ) {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.insert((instrument_id, module_id), descriptor);
    }

    /// Check if a module exists in the registry for a specific instrument.
    pub fn has_module(&self, instrument_id: InstrumentId, module_id: ModuleId) -> bool {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.contains_key(&(instrument_id, module_id))
    }

    /// Get the descriptor for a module in a specific instrument.
    pub fn module_descriptor(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
    ) -> Option<ModuleDescriptor> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.get(&(instrument_id, module_id)).cloned()
    }

    /// Get modules belonging to a specific instrument.
    pub fn all_modules_for_instrument(
        &self,
        instrument_id: InstrumentId,
    ) -> HashMap<ModuleId, ModuleDescriptor> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .iter()
            .filter(|&(&(inst_id, _), _)| inst_id == instrument_id)
            .map(|(&(_, mod_id), desc)| (mod_id, desc.clone()))
            .collect()
    }

    /// Read-access to the shared engine state (meters, transport, etc.).
    pub fn state(&self) -> &Arc<EngineState> {
        &self.state
    }

    /// Get a clone of the command sender for operations not managed by session
    /// (note_on, set_tempo, etc.).
    pub fn command_sender(&self) -> CommandSender {
        self.command_sender.clone()
    }

    /// Check if an instrument exists in the shared snapshots.
    pub fn instrument_exists(&self, instrument_id: InstrumentId) -> bool {
        self.state
            .instrument_snapshots
            .read()
            .iter()
            .any(|s| s.id == instrument_id)
    }

    // ------------------------------------------------------------------
    // Parameter setting (GUI-independent)
    // ------------------------------------------------------------------

    /// Set a module parameter by name, resolving `ParamValue` to f32.
    ///
    /// Uses the same logic as `apply_module_parameters` in `patch_bridge.rs`
    /// but without any GUI dependencies.
    pub fn set_parameter(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
        param_name: &str,
        value: &ParamValue,
    ) -> Result<(), SessionError> {
        let descriptor = self
            .module_descriptor(instrument_id, module_id)
            .ok_or_else(|| SessionError::ModuleNotFound(module_id.to_string()))?;

        // Normalize: lowercase and replace underscores with spaces for fuzzy matching.
        // This allows patch files to use snake_case ("key_tracking") while engine
        // uses Title Case with spaces ("Key Tracking").
        let normalize = |s: &str| s.to_lowercase().replace('_', " ");
        let needle = normalize(param_name);
        let param_desc = descriptor
            .parameters
            .iter()
            .find(|p| normalize(&p.type_id) == needle)
            .or_else(|| {
                descriptor
                    .parameters
                    .iter()
                    .find(|p| normalize(&p.name) == needle)
            })
            .ok_or_else(|| SessionError::ParameterNotFound(param_name.to_string()))?;

        let f32_value = match value {
            ParamValue::Float(f) => *f,
            ParamValue::Int(i) => *i as f32,
            ParamValue::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            ParamValue::Choice(s) => {
                if let Some(ref choices) = param_desc.choices {
                    choices
                        .iter()
                        .position(|c| c.id == *s)
                        .map(|i| i as f32)
                        .unwrap_or(0.0)
                } else {
                    0.0
                }
            }
        };

        let param = param_desc.id.with_f32(f32_value);

        let cmd = if let Some(effect_type) = EffectType::from_module_type(module_id.module_type) {
            EngineCommand::SetEffectParameter {
                instrument_id: Some(instrument_id),
                effect_type,
                param,
            }
        } else {
            EngineCommand::SetModuleParameter {
                instrument_id: Some(instrument_id),
                module_id,
                param,
            }
        };

        if !self.command_sender.send(cmd) {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Patch loading (GUI-independent)
    // ------------------------------------------------------------------

    /// Apply a patch to an instrument without GUI dependencies.
    ///
    /// Skips visualizer modules (Oscilloscope, SignalMonitor, etc.) since
    /// they require `VisualizationBuffer`. The GUI picks up modules via
    /// `reconcile_with_session()`.
    pub fn apply_patch(&self, instrument_id: InstrumentId, patch: &Patch) -> ApplyPatchResult {
        let mut result = ApplyPatchResult {
            module_count: 0,
            connection_count: 0,
            module_ids: Vec::new(),
            errors: Vec::new(),
        };

        // Clear existing modules
        if let Err(e) = self.clear_graph(instrument_id) {
            result.errors.push(format!("clear_graph failed: {e}"));
            return result;
        }

        // Add modules
        for module_state in &patch.modules {
            let module_type = module_state.module_type;
            if module_type.is_visualizer() || module_type == ModuleType::SignalMonitor {
                // Visualizer/signal monitor — skip (requires GUI-specific setup)
                result.module_ids.push(None);
                continue;
            }

            let module_id: ModuleId = match module_state.id.parse() {
                Ok(id) => id,
                Err(_) => {
                    result
                        .errors
                        .push(format!("invalid module ID: {}", module_state.id));
                    result.module_ids.push(None);
                    continue;
                }
            };

            match self.add_module_with_id(instrument_id, module_id, module_type) {
                Ok(descriptor) => {
                    result.module_count += 1;
                    result.module_ids.push(Some(module_id.to_string()));

                    // Apply parameters
                    for (param_name, value) in &module_state.parameters {
                        if let Err(e) =
                            self.set_parameter(instrument_id, module_id, param_name, value)
                        {
                            result
                                .errors
                                .push(format!("{} param '{}': {e}", module_id, param_name));
                        }
                    }
                    drop(descriptor);
                }
                Err(e) => {
                    result
                        .errors
                        .push(format!("add module {}: {e}", module_state.id));
                    result.module_ids.push(None);
                }
            }
        }

        // Add connections
        for conn in &patch.connections {
            let from_id: ModuleId = match conn.from.0.parse() {
                Ok(id) => id,
                Err(_) => {
                    result
                        .errors
                        .push(format!("invalid from module: {}", conn.from.0));
                    continue;
                }
            };
            let to_id: ModuleId = match conn.to.0.parse() {
                Ok(id) => id,
                Err(_) => {
                    result
                        .errors
                        .push(format!("invalid to module: {}", conn.to.0));
                    continue;
                }
            };

            match self.connect(
                instrument_id,
                from_id,
                conn.from.1.clone(),
                to_id,
                conn.to.1.clone(),
            ) {
                Ok(()) => result.connection_count += 1,
                Err(e) => result.errors.push(format!(
                    "connect {}:{} → {}:{}: {e}",
                    conn.from.0, conn.from.1, conn.to.0, conn.to.1
                )),
            }
        }

        result
    }

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Direct access to counters for GUI-only modules (visualizers, signal monitors)
    /// that cannot go through `add_module` because they need special setup.
    pub fn counters_lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<(InstrumentId, ModuleType), u16>> {
        self.counters.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Generate the next `ModuleId` for a given type within an instrument.
    ///
    /// Each instrument has its own counter per module type, so both instrument 0
    /// and instrument 1 can have `osc-1`.
    fn next_module_id(&self, instrument_id: InstrumentId, module_type: ModuleType) -> ModuleId {
        let mut counters = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        let counter = counters.entry((instrument_id, module_type)).or_insert(0);
        *counter += 1;
        ModuleId::new(module_type, *counter)
    }

    /// Create the module/effect via the factory, send it to the engine,
    /// and register it in the registry.
    fn create_and_send(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
        module_type: ModuleType,
    ) -> Result<ModuleDescriptor, SessionError> {
        // Try voice module
        if module_type.is_voice_module() {
            let (module, descriptor) = module_factory::create_voice_module(module_type)
                .ok_or_else(|| {
                    SessionError::UnsupportedModuleType(module_type.name().to_string())
                })?;

            if !self.command_sender.send(EngineCommand::AddModuleInstance {
                instrument_id: Some(instrument_id),
                id: module_id,
                module,
            }) {
                return Err(SessionError::SendFailed);
            }

            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry.insert((instrument_id, module_id), descriptor.clone());
            return Ok(descriptor);
        }

        // Try effect
        if module_type.is_effect() {
            let (effect, descriptor) =
                module_factory::create_effect(module_type).ok_or_else(|| {
                    SessionError::UnsupportedModuleType(module_type.name().to_string())
                })?;

            if !self.command_sender.send(EngineCommand::AddEffectInstance {
                instrument_id: Some(instrument_id),
                id: module_id,
                effect,
            }) {
                return Err(SessionError::SendFailed);
            }

            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry.insert((instrument_id, module_id), descriptor.clone());
            return Ok(descriptor);
        }

        Err(SessionError::UnsupportedModuleType(
            module_type.name().to_string(),
        ))
    }
}

/// Result of applying a patch to an instrument.
pub struct ApplyPatchResult {
    /// Number of modules successfully created.
    pub module_count: usize,
    /// Number of connections successfully created.
    pub connection_count: usize,
    /// Module IDs in the same order as the patch's module array.
    /// `None` for skipped modules (visualizers, invalid IDs).
    pub module_ids: Vec<Option<String>>,
    /// Non-fatal errors encountered during loading.
    pub errors: Vec<String>,
}
