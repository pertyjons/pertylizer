//! `SynthSession` — shared control layer for GUI, MCP, and future interfaces.
//!
//! Owns the module lifecycle (create, remove, connect) and provides a
//! thread-safe API used by both the GUI thread and the MCP bridge.
//! In headless mode (`--mcp`) there is no GUI, so `SynthSession` is the
//! sole owner of module state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use synth_core::{ModuleCategory, ModuleDescriptor, ModuleType};
use synth_engine::commands::PortId;
use synth_engine::instrument::InstrumentId;
use synth_engine::state::EngineState;
use synth_engine::{CommandSender, EngineCommand, ModuleId};

use crate::module_factory;

/// Error type for session operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("unsupported module type: {0}")]
    UnsupportedModuleType(String),

    #[error("module not found: {0}")]
    ModuleNotFound(String),

    #[error("visualizer modules require GUI (VisualizationBuffer)")]
    VisualizerRequiresGui,

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
    /// Instance counters for ID generation (module_type → next instance number).
    counters: Mutex<HashMap<ModuleType, u16>>,
    /// Registry of all modules currently managed by this session.
    registry: Mutex<HashMap<ModuleId, ModuleDescriptor>>,
}

impl SynthSession {
    /// Create a new session.
    pub fn new(command_sender: CommandSender, state: Arc<EngineState>) -> Self {
        Self {
            command_sender,
            state,
            counters: Mutex::new(HashMap::new()),
            registry: Mutex::new(HashMap::new()),
        }
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

        let module_id = self.next_module_id(module_type);

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
            let counter = counters.entry(module_type).or_insert(0);
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
                .remove(&module_id)
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
        from_port: String,
        to_module: ModuleId,
        to_port: String,
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
        from_port: String,
        to_module: ModuleId,
        to_port: String,
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

    /// Clear the entire graph for an instrument — removes all modules.
    pub fn clear_graph(&self, instrument_id: InstrumentId) -> Result<(), SessionError> {
        let modules: Vec<(ModuleId, ModuleCategory)> = {
            let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry
                .iter()
                .map(|(id, desc)| (*id, desc.category))
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

        // Clear internal state
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.clear();
        let mut counters = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        counters.clear();

        Ok(())
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Check if a module exists in the registry.
    pub fn has_module(&self, module_id: ModuleId) -> bool {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.contains_key(&module_id)
    }

    /// Get the descriptor for a module.
    pub fn module_descriptor(&self, module_id: ModuleId) -> Option<ModuleDescriptor> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.get(&module_id).cloned()
    }

    /// Get all modules currently in the session.
    pub fn all_modules(&self) -> HashMap<ModuleId, ModuleDescriptor> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.clone()
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

    // ------------------------------------------------------------------
    // Internal helpers
    // ------------------------------------------------------------------

    /// Direct access to counters for GUI-only modules (visualizers, signal monitors)
    /// that cannot go through `add_module` because they need special setup.
    pub fn counters_lock(&self) -> std::sync::MutexGuard<'_, HashMap<ModuleType, u16>> {
        self.counters.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Generate the next `ModuleId` for a given type.
    fn next_module_id(&self, module_type: ModuleType) -> ModuleId {
        let mut counters = self.counters.lock().unwrap_or_else(|e| e.into_inner());
        let counter = counters.entry(module_type).or_insert(0);
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
            registry.insert(module_id, descriptor.clone());
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
            registry.insert(module_id, descriptor.clone());
            return Ok(descriptor);
        }

        Err(SessionError::UnsupportedModuleType(
            module_type.name().to_string(),
        ))
    }
}
