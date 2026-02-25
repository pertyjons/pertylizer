//! `SynthSession` — shared control layer for GUI, MCP, and future interfaces.
//!
//! Owns the module lifecycle (create, remove, connect) and provides a
//! thread-safe API used by both the GUI thread and the MCP bridge.
//! In headless mode (`--mcp`) there is no GUI, so `SynthSession` is the
//! sole owner of module state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use synth_core::{BipolarValue, Gain, ModuleCategory, ModuleDescriptor, ModuleType};
use synth_engine::commands::PortId;
use synth_engine::instrument::{Instrument, InstrumentId, MidiChannel};
use synth_engine::shared_state::InstrumentSnapshot;
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

    #[error("instrument not found: {0}")]
    InstrumentNotFound(u64),

    #[error("visualizer modules require GUI (VisualizationBuffer)")]
    VisualizerRequiresGui,

    #[error("failed to send engine command")]
    SendFailed,
}

/// Registry entry tracking which instrument a module belongs to.
struct RegistryEntry {
    instrument_id: InstrumentId,
    descriptor: ModuleDescriptor,
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
    registry: Mutex<HashMap<ModuleId, RegistryEntry>>,
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

    /// Remove an instrument from the engine.
    pub fn remove_instrument(&self, instrument_id: InstrumentId) -> Result<(), SessionError> {
        // Remove all modules belonging to this instrument from the registry
        {
            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry.retain(|_, entry| entry.instrument_id != instrument_id);
        }

        if !self
            .command_sender
            .send(EngineCommand::RemoveInstrument { instrument_id })
        {
            return Err(SessionError::SendFailed);
        }

        Ok(())
    }

    /// Rename an instrument. Name is stored directly in shared state
    /// (no engine command needed since name doesn't affect audio).
    pub fn rename_instrument(
        &self,
        instrument_id: InstrumentId,
        name: &str,
    ) -> Result<(), SessionError> {
        let mut snapshots = self.state.instrument_snapshots.write();
        if let Some(snap) = snapshots.iter_mut().find(|s| s.id == instrument_id) {
            snap.name = name.to_string();
            Ok(())
        } else {
            Err(SessionError::InstrumentNotFound(instrument_id.as_u64()))
        }
    }

    /// Set instrument volume.
    pub fn set_instrument_volume(
        &self,
        instrument_id: InstrumentId,
        volume: f32,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::Volume(Gain::new(volume)),
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
        pan: f32,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::Pan(BipolarValue::new(pan)),
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
                .map(|e| e.descriptor.category)
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

    /// Clear the entire graph for an instrument — removes only modules for that instrument.
    pub fn clear_graph(&self, instrument_id: InstrumentId) -> Result<(), SessionError> {
        let modules: Vec<(ModuleId, ModuleCategory)> = {
            let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry
                .iter()
                .filter(|(_, entry)| entry.instrument_id == instrument_id)
                .map(|(id, entry)| (*id, entry.descriptor.category))
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
            registry.retain(|_, entry| entry.instrument_id != instrument_id);
        }

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
        registry.get(&module_id).map(|e| e.descriptor.clone())
    }

    /// Get all modules currently in the session (across all instruments).
    pub fn all_modules(&self) -> HashMap<ModuleId, ModuleDescriptor> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .iter()
            .map(|(id, entry)| (*id, entry.descriptor.clone()))
            .collect()
    }

    /// Get modules belonging to a specific instrument.
    pub fn all_modules_for_instrument(
        &self,
        instrument_id: InstrumentId,
    ) -> HashMap<ModuleId, ModuleDescriptor> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .iter()
            .filter(|(_, entry)| entry.instrument_id == instrument_id)
            .map(|(id, entry)| (*id, entry.descriptor.clone()))
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
            registry.insert(
                module_id,
                RegistryEntry {
                    instrument_id,
                    descriptor: descriptor.clone(),
                },
            );
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
            registry.insert(
                module_id,
                RegistryEntry {
                    instrument_id,
                    descriptor: descriptor.clone(),
                },
            );
            return Ok(descriptor);
        }

        Err(SessionError::UnsupportedModuleType(
            module_type.name().to_string(),
        ))
    }
}
