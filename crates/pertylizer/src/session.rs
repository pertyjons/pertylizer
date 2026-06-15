//! `SynthSession` — shared control layer for GUI, MCP, and future interfaces.
//!
//! Owns the module lifecycle (create, remove, connect) and provides a
//! thread-safe API used by both the GUI thread and the MCP bridge.
//! In headless mode (`--mcp`) there is no GUI, so `SynthSession` is the
//! sole owner of module state.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use synth_core::{BipolarValue, Gain, ModuleCategory, ModuleDescriptor, ModuleType, PortName};
use synth_engine::commands::PortId;
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

    #[error("control-script compile error: {0}")]
    ScriptCompile(String),
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
    /// Synchronous mirror of instrument IDs currently owned by this session.
    /// Why: `EngineState::instrument_snapshots` is only rebuilt on the audio
    /// thread after it pops an `EngineCommand`, so a write-then-validate inside
    /// one `batch_execute` would race the audio tick. This set is updated
    /// inline at allocation and removal so `instrument_exists` answers
    /// correctly during that window.
    alive_instruments: Mutex<HashSet<InstrumentId>>,
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
            alive_instruments: Mutex::new(HashSet::new()),
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

        self.alive_instruments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id);

        let instrument = Box::new(Instrument::new(id, name));

        if !self
            .command_sender
            .send(EngineCommand::AddInstrument { instrument })
        {
            self.alive_instruments
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
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
        self.add_instrument_with_id_and_config(id, name, None)
    }

    /// Add an instrument under a specific ID with an optional allocator
    /// config. The config governs `max_voices` / `mode` / `stealing` —
    /// these can't be changed at runtime because the voice pool is
    /// pre-allocated, so this constructor is the only way to honour a
    /// loaded project's polyphony settings.
    pub fn add_instrument_with_id_and_config(
        &self,
        id: InstrumentId,
        name: &str,
        config: Option<synth_engine::voice_allocator::AllocatorConfig>,
    ) -> Result<(), SessionError> {
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

        self.alive_instruments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id);

        let instrument = Box::new(match config {
            Some(cfg) => Instrument::with_config(id, name, cfg),
            None => Instrument::new(id, name),
        });

        if !self
            .command_sender
            .send(EngineCommand::AddInstrument { instrument })
        {
            self.alive_instruments
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
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

    /// Reset the instrument-ID counter back to its initial value.
    ///
    /// Call this after tearing down all instruments (New Project / load) so the
    /// next `add_instrument` starts from a low ID again instead of continuing
    /// from the previous project's high-water mark. `add_instrument_with_id`
    /// bumps it back up for each loaded instrument, so a subsequent project
    /// load still preserves saved IDs.
    pub fn reset_instrument_counter(&self) {
        *self
            .instrument_counter
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = 1; // 0 is reserved for the default
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
        self.alive_instruments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&instrument_id);
        // Drop any per-instrument metadata held in shared engine state.
        self.state().clear_octave_offset(instrument_id);

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

    /// Set an instrument's free-text description / intent.
    pub fn set_instrument_description(
        &self,
        instrument_id: InstrumentId,
        description: &str,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentDescription {
                instrument_id,
                description: description.to_string(),
            })
        {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set or clear the patch-level description on an instrument.
    pub fn set_patch_description(
        &self,
        instrument_id: InstrumentId,
        description: Option<&str>,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetPatchDescription {
                instrument_id,
                description: description.map(str::to_owned),
            })
        {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Set or clear the transport loop region.
    ///
    /// Routes through `EngineCommand::SetLoop`. The engine mirrors the new
    /// state into `EngineState::transport`, so a follow-up call to
    /// `transport_loop_state()` reflects the change once the command is
    /// processed (next audio callback).
    pub fn set_transport_loop(
        &self,
        start: synth_sequencer::Tick,
        end: synth_sequencer::Tick,
        enabled: bool,
    ) -> Result<(), SessionError> {
        if !self.command_sender.send(EngineCommand::SetLoop {
            start,
            end,
            enabled,
        }) {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Read the current transport loop region as `(enabled, start, end)`.
    pub fn transport_loop_state(&self) -> (bool, synth_sequencer::Tick, synth_sequencer::Tick) {
        self.state.transport.loop_state()
    }

    /// Set or clear an instrument's sidechain source.
    pub fn set_sidechain_source(
        &self,
        instrument_id: InstrumentId,
        source: Option<InstrumentId>,
    ) -> Result<(), SessionError> {
        if !self.command_sender.send(EngineCommand::SetSidechainSource {
            instrument_id,
            source,
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

        let mut failed_modules = Vec::new();
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
            if !self.command_sender.send(cmd) {
                eprintln!(
                    "clear_graph: failed to send remove command for module {} in instrument {}",
                    module_id, instrument_id
                );
                failed_modules.push(*module_id);
            }
        }

        // Clear registry entries for this instrument, keeping modules whose removal failed
        {
            let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            registry.retain(|&(inst_id, mod_id), _| {
                inst_id != instrument_id || failed_modules.contains(&mod_id)
            });
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

    /// Check if an instrument is known to this session. See
    /// [`Self::alive_instruments`] for why this reads the synchronous mirror
    /// rather than `EngineState::instrument_snapshots`.
    pub fn instrument_exists(&self, instrument_id: InstrumentId) -> bool {
        self.alive_instruments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&instrument_id)
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

        let param = value.to_param(param_desc);

        let cmd = if module_id.module_type.is_effect() {
            EngineCommand::SetEffectParameter {
                instrument_id: Some(instrument_id),
                module_id,
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

    /// Compile a YAMS control script and install it on a Mod Matrix `slot`
    /// (0-based), replacing the slot's scalar `source × amount` with the script's
    /// output (Step 2, decision #1). Compilation runs here — **off the audio
    /// thread** — and only the shared `Arc<BoundScript>` crosses to the engine
    /// via [`EngineCommand::SetModScript`]. A compile error returns
    /// [`SessionError::ScriptCompile`] with all diagnostics so the caller can
    /// disable-and-keep (skip this slot, keep the rest of the project).
    pub fn set_mod_script(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
        slot: u8,
        source: &str,
    ) -> Result<(), SessionError> {
        // The control rate the engine actually runs at drives `lag` coefficient
        // folding; the default (48 kHz / 64-sample blocks) is the common case and
        // a mismatch only nudges constant-time smoothing — recompile on a real
        // rate change is a later refinement.
        let (program, diags) =
            synth_script::compile(source, &synth_script::CompileOptions::default());
        let Some(program) = program else {
            let msg = diags
                .iter()
                .filter(|d| d.is_error())
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(SessionError::ScriptCompile(msg));
        };
        let bound = Arc::new(program.into_bound(source.to_string()));
        let cmd = EngineCommand::SetModScript {
            instrument_id: Some(instrument_id),
            module_id,
            slot,
            script: Some(bound),
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

        // Reset counters so stale state from a previous patch doesn't cause ID collisions
        self.reset_counters_for_instrument(instrument_id);

        // Clear existing modules
        if let Err(e) = self.clear_graph(instrument_id) {
            result.errors.push(format!("clear_graph failed: {e}"));
            return result;
        }

        // Upgrade legacy positional mod-matrix ids ("env2") to absolute
        // addresses, resolved against this instrument's real instances — old
        // projects on non-canonical instance numbers would otherwise modulate
        // the wrong (or no) module after the address refactor.
        let mut modules = patch.modules.clone();
        crate::patch::upgrade_legacy_mod_matrix(&mut modules);

        // Add modules
        for module_state in &modules {
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
                Ok(_descriptor) => {
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

                    // Install Step-2 control scripts (compiled here, off the
                    // audio thread). Keys are 1-based slot numbers (matching the
                    // `slot_N_*` params); a bad key or a compile error is recorded
                    // and skipped (disable-and-keep — the rest of the project still
                    // loads). Applied after params so the routing already exists.
                    for (slot_key, source) in &module_state.scripts {
                        // Range-check against the real slot count so a numeric but
                        // out-of-[1,16] key is reported, not silently dropped by the
                        // engine's bounds guard.
                        let max_slot = synth_core::MAX_MOD_MATRIX_SLOTS as u8;
                        let slot = match slot_key.parse::<u8>() {
                            Ok(n) if (1..=max_slot).contains(&n) => n - 1,
                            _ => {
                                result.errors.push(format!(
                                    "{module_id} script: invalid slot key '{slot_key}' (expected 1..={max_slot})"
                                ));
                                continue;
                            }
                        };
                        if let Err(e) = self.set_mod_script(instrument_id, module_id, slot, source)
                        {
                            result
                                .errors
                                .push(format!("{module_id} slot {slot_key} script: {e}"));
                        }
                    }
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

        // Mirror keyboard-relevant patch settings into engine state so non-GUI
        // note-rendering paths (MCP `preview_note`) match live keyboard behavior.
        self.state()
            .set_octave_offset(instrument_id, patch.settings.octave_offset);

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

#[cfg(test)]
mod tests {
    use super::*;
    use synth_engine::SynthEngine;

    /// A session wired to a live (but undriven) engine — enough to exercise the
    /// command-sending API. The engine is kept alive so the command ring buffer
    /// has a consumer.
    fn test_session() -> (SynthEngine, SynthSession) {
        let (engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        (engine, session)
    }

    /// A valid YAMS script compiles off-thread and the install command is sent;
    /// an invalid one fails with `ScriptCompile` (carrying diagnostics) *before*
    /// anything is sent, so the caller can disable-and-keep.
    #[test]
    fn set_mod_script_compiles_then_sends_or_reports() {
        let (_engine, session) = test_session();
        let mmx = ModuleId::new(ModuleType::ModMatrix, 1);

        // Valid: queued (the engine no-ops if the module is absent — the write
        // channel was tested separately).
        assert!(
            session
                .set_mod_script(InstrumentId::FIRST, mmx, 0, "out = velocity * 0.5")
                .is_ok()
        );

        // Invalid: surfaced as a compile error, not a silent drop.
        let err = session.set_mod_script(InstrumentId::FIRST, mmx, 0, "out = velocity *");
        assert!(
            matches!(err, Err(SessionError::ScriptCompile(_))),
            "expected ScriptCompile, got {err:?}"
        );
    }
}
