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

/// Compile a YAMS `source` to a shared [`BoundScript`], **off the audio thread**.
/// On failure returns the joined error diagnostics as a single message. Shared by
/// the live load path ([`SynthSession::set_mod_script`]) and the offline renderer
/// so both report compile errors identically.
///
/// `audio_rate` / `control_ports` select the language dialect (the caller derives
/// both from the target module's type — [`ModuleType::script_is_audio_rate`] /
/// [`ModuleType::script_uses_control_ports`]): both `false` for the Mod Matrix
/// `scr` scripts (single `out`, no ports); `audio_rate` for the per-sample
/// `AudioScript` (`in`/`in_l`/`in_r`, `first_sample`, `out.left`/`out.right`);
/// `control_ports` for the `Script` module (`in1..in4` / `out1..out4`). The two
/// are mutually exclusive.
pub(crate) fn compile_mod_script(
    source: &str,
    audio_rate: bool,
    control_ports: bool,
) -> Result<Arc<synth_core::script::BoundScript>, String> {
    compile_script_with(
        source,
        synth_script::CompileOptions {
            audio_rate,
            control_ports,
            ..synth_script::CompileOptions::default()
        },
    )
}

/// Compile a YAMS `note_event` (`NoteScriptTransform`) script to a shared
/// [`BoundScript`], **off the audio thread**. Same failure contract as
/// [`compile_mod_script`] — the joined error diagnostics on failure — but for
/// the note-event dialect (`note_pitch`/`note_vel`/`note_dur`/`tick` reads,
/// `in1..in4` value inputs, `out.pitch`/`out.vel`/`out.dur`/`out.gate` writes).
/// Shared by the Note Grid GUI editor, the MCP script tool, and the load-time
/// recompile so all three report compile errors identically.
pub(crate) fn compile_note_event_script(
    source: &str,
) -> Result<Arc<synth_core::script::BoundScript>, String> {
    compile_script_with(
        source,
        synth_script::CompileOptions {
            note_event: true,
            ..synth_script::CompileOptions::default()
        },
    )
}

/// Compile `source` under `opts`, returning the bound program or the joined
/// error diagnostics. The single implementation behind the per-dialect helpers.
fn compile_script_with(
    source: &str,
    opts: synth_script::CompileOptions,
) -> Result<Arc<synth_core::script::BoundScript>, String> {
    let (program, diags) = synth_script::compile(source, &opts);
    let Some(program) = program else {
        let msg = diags
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(msg);
    };
    Ok(Arc::new(program.into_bound(source.to_string())))
}

/// Build the fresh [`ModuleDescriptor`] a script module (`Script`/`AudioScript`)
/// advertises with `params` installed: its base descriptor plus one knob
/// parameter per declared `param`. Returns `None` for any other module type — a
/// Mod Matrix `scr` script never changes its descriptor. Built **off the audio
/// thread**; the engine only shares the `Arc` into each graph node and defers the
/// old one's drop (§3.C). Shared by the live authoring path and the offline
/// renderer.
pub(crate) fn build_script_descriptor(
    module_type: ModuleType,
    params: &[synth_core::script::ScriptParamDecl],
) -> Option<Arc<ModuleDescriptor>> {
    if !module_type.script_is_audio_rate() && !module_type.script_uses_control_ports() {
        return None;
    }
    let mut desc = crate::module_factory::get_descriptor(module_type)?;
    for decl in params {
        desc = desc.parameter(synth_core::script::knob_descriptor(decl, decl.default));
    }
    Some(Arc::new(desc))
}

/// Parse a 1-based mod-script slot key into a 0-based slot index, range-checked
/// against [`synth_core::MAX_MOD_MATRIX_SLOTS`]. Returns an error message naming
/// the valid range on a non-numeric or out-of-range key. Shared by the live load
/// path and the offline renderer so they accept exactly the same slot keys.
pub(crate) fn parse_mod_slot_key(slot_key: &str) -> Result<u8, String> {
    let max_slot = synth_core::MAX_MOD_MATRIX_SLOTS as u8;
    match slot_key.parse::<u8>() {
        Ok(n) if (1..=max_slot).contains(&n) => Ok(n - 1),
        _ => Err(format!(
            "invalid slot key '{slot_key}' (expected 1..={max_slot})"
        )),
    }
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

    /// Write-through a field on the cached instrument snapshot so a read-back
    /// (`list_instruments` / `get_instrument_info`) inside the same
    /// `batch_execute` sees the change before the audio thread rebuilds the
    /// snapshot (~one buffer later). The queued `EngineCommand` stays the source
    /// of truth for the audio itself; this only keeps the read-side metadata from
    /// lagging during that window. Same race rationale as
    /// [`Self::alive_instruments`].
    ///
    /// No-op when the instrument is not in the snapshot yet — it will pick up the
    /// value on the next audio-thread rebuild.
    fn patch_instrument_snapshot(
        &self,
        instrument_id: InstrumentId,
        patch: impl FnOnce(&mut InstrumentSnapshot),
    ) {
        let mut snapshots = self.state.instrument_snapshots.write();
        if let Some(snapshot) = snapshots.iter_mut().find(|s| s.id == instrument_id) {
            patch(snapshot);
        }
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
        self.patch_instrument_snapshot(instrument_id, |s| {
            s.description = description.to_string();
        });
        Ok(())
    }

    /// Set or clear an instrument's accent color (hex string). `None` clears.
    pub fn set_instrument_color(
        &self,
        instrument_id: InstrumentId,
        color: Option<&str>,
    ) -> Result<(), SessionError> {
        if !self.command_sender.send(EngineCommand::SetInstrumentColor {
            instrument_id,
            color: color.map(str::to_string),
        }) {
            return Err(SessionError::SendFailed);
        }
        self.patch_instrument_snapshot(instrument_id, |s| {
            s.color = color.map(str::to_string);
        });
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
        self.patch_instrument_snapshot(instrument_id, |s| {
            s.patch_description = description.map(str::to_owned);
        });
        Ok(())
    }

    /// Set or clear an instrument's patch-level accent color (hex string).
    /// `None` clears.
    pub fn set_patch_color(
        &self,
        instrument_id: InstrumentId,
        color: Option<&str>,
    ) -> Result<(), SessionError> {
        if !self.command_sender.send(EngineCommand::SetPatchColor {
            instrument_id,
            color: color.map(str::to_owned),
        }) {
            return Err(SessionError::SendFailed);
        }
        self.patch_instrument_snapshot(instrument_id, |s| {
            s.patch_color = color.map(str::to_owned);
        });
        Ok(())
    }

    /// Set or clear the free-text description on a specific module instance.
    /// `None` (or an empty string) clears it.
    pub fn set_module_description(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
        description: Option<&str>,
    ) -> Result<(), SessionError> {
        let normalized = description
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        if !self
            .command_sender
            .send(EngineCommand::SetModuleDescription {
                instrument_id,
                module_id,
                description: description.map(str::to_owned),
            })
        {
            return Err(SessionError::SendFailed);
        }
        self.state.shared_graph.set_module_description(
            instrument_id,
            module_id,
            normalized.to_owned(),
        );
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
        self.patch_instrument_snapshot(instrument_id, |s| {
            s.sidechain_source_id = source;
        });
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
        self.patch_instrument_snapshot(instrument_id, |s| s.volume = volume);
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
        self.patch_instrument_snapshot(instrument_id, |s| s.pan = pan);
        Ok(())
    }

    /// Set the voice allocation mode (Polyphonic / Mono / Legato / Unison).
    pub fn set_instrument_allocation_mode(
        &self,
        instrument_id: InstrumentId,
        mode: synth_engine::voice_allocator::AllocationMode,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::AllocationMode(mode),
            })
        {
            return Err(SessionError::SendFailed);
        }
        self.patch_instrument_snapshot(instrument_id, |s| s.allocation_mode = mode);
        Ok(())
    }

    /// Set the voice-stealing strategy used when all voices are busy.
    pub fn set_instrument_stealing_strategy(
        &self,
        instrument_id: InstrumentId,
        strategy: synth_engine::voice_allocator::StealingStrategy,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::StealingStrategy(strategy),
            })
        {
            return Err(SessionError::SendFailed);
        }
        self.patch_instrument_snapshot(instrument_id, |s| s.stealing_strategy = strategy);
        Ok(())
    }

    /// Set the unison detune spread (total cents across all `Unison`-mode voices).
    pub fn set_instrument_unison_detune(
        &self,
        instrument_id: InstrumentId,
        detune: synth_core::Cents,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::UnisonDetune(detune),
            })
        {
            return Err(SessionError::SendFailed);
        }
        self.patch_instrument_snapshot(instrument_id, |s| s.unison_detune = detune);
        Ok(())
    }

    /// Set the unison stereo spread (0..1) for `Unison`-mode voices.
    pub fn set_instrument_unison_spread(
        &self,
        instrument_id: InstrumentId,
        spread: synth_core::NormalizedValue,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::UnisonSpread(spread),
            })
        {
            return Err(SessionError::SendFailed);
        }
        self.patch_instrument_snapshot(instrument_id, |s| s.unison_spread = spread);
        Ok(())
    }

    /// Set the maximum polyphony (voice count).
    ///
    /// Applied to the stored config; the live voice pool is *not* resized (that
    /// would allocate on the audio thread). The new count takes effect on the
    /// next voice-graph reconstruction — e.g. on project load. The snapshot is
    /// patched immediately so readers reflect the requested value.
    pub fn set_instrument_max_voices(
        &self,
        instrument_id: InstrumentId,
        max_voices: synth_core::VoiceCount,
    ) -> Result<(), SessionError> {
        if !self
            .command_sender
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param: synth_engine::commands::InstrumentParam::MaxVoices(max_voices),
            })
        {
            return Err(SessionError::SendFailed);
        }
        self.patch_instrument_snapshot(instrument_id, |s| s.max_voices = max_voices);
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
        self.patch_instrument_snapshot(instrument_id, |s| {
            s.enabled = !muted;
            s.muted = muted;
        });
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
        self.patch_instrument_snapshot(instrument_id, |s| {
            s.enabled = enabled;
            s.muted = !enabled;
        });
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
        self.patch_instrument_snapshot(instrument_id, |s| s.category = category);
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
        self.patch_instrument_snapshot(instrument_id, |s| s.solo = solo);
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
        // Mirror the engine's snapshot conversion exactly (1-indexed display).
        self.patch_instrument_snapshot(instrument_id, |s| {
            s.midi_channel = synth_core::MidiChannel::new(channel.as_zero_indexed() + 1);
        });
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

    /// Block until the audio thread has drained every command submitted so far,
    /// or `timeout_ms` elapses. Returns `true` if the queue caught up, `false` on
    /// timeout.
    ///
    /// Use this before reading the async-mirrored `shared_graph` (e.g. a save)
    /// right after queuing graph mutations: `add_module`/`connect` only reach the
    /// snapshot once the audio thread pops and applies them, so a save issued in
    /// the same `batch_execute` would otherwise read a stale/truncated graph. The
    /// command ring is FIFO, so once `processed` reaches the `enqueued` snapshot
    /// taken here, all earlier mutations are applied *and* mirrored.
    ///
    /// If the audio thread is not running (e.g. an undriven test engine) this
    /// waits out the full timeout — same bounded-wait behaviour as the
    /// instrument-mirror wait in `project_apply`.
    #[must_use]
    pub fn wait_for_pending_commands(&self, timeout_ms: u64) -> bool {
        let target = self.state.command_sync.enqueued();
        if self.state.command_sync.processed() >= target {
            return true;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        while std::time::Instant::now() < deadline {
            if self.state.command_sync.processed() >= target {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.state.command_sync.processed() >= target
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
        // rate change is a later refinement. The `AudioScript` module compiles its
        // program in the audio-rate dialect (per-sample `in`/`out.left`/…).
        let mt = module_id.module_type;
        let bound = compile_mod_script(
            source,
            mt.script_is_audio_rate(),
            mt.script_uses_control_ports(),
        )
        .map_err(SessionError::ScriptCompile)?;
        // Rebuild the descriptor off-thread for a script module whose knob set may
        // have changed; register it into the session registry (feeds the GUI knob
        // grid, MCP discovery, and the save path) and carry it to the engine to
        // swap into each voice node (feeds cross-script reads). `None` for the Mod
        // Matrix, whose descriptor never changes on a script edit.
        let descriptor = build_script_descriptor(mt, &bound.params);
        if let Some(d) = &descriptor {
            self.register_descriptor(instrument_id, module_id, (**d).clone());
        }
        let cmd = EngineCommand::SetModScript {
            instrument_id: Some(instrument_id),
            module_id,
            slot,
            script: Some(bound),
            descriptor,
        };
        if !self.command_sender.send(cmd) {
            return Err(SessionError::SendFailed);
        }
        Ok(())
    }

    /// Clear a Mod Matrix `slot`'s control script (0-based), reverting the slot
    /// to its scalar `amount × source` behaviour. Sends `SetModScript` with
    /// `None` — an empty source can't take the compile path ([`Self::set_mod_script`])
    /// because YAMS requires an `out` statement.
    pub fn clear_mod_script(
        &self,
        instrument_id: InstrumentId,
        module_id: ModuleId,
        slot: u8,
    ) -> Result<(), SessionError> {
        // Clearing reverts a script module's descriptor to its base (no knobs);
        // register + carry it so the GUI/MCP/save and the voice nodes drop the
        // now-gone knobs. `None` for the Mod Matrix.
        let descriptor = build_script_descriptor(module_id.module_type, &[]);
        if let Some(d) = &descriptor {
            self.register_descriptor(instrument_id, module_id, (**d).clone());
        }
        let cmd = EngineCommand::SetModScript {
            instrument_id: Some(instrument_id),
            module_id,
            slot,
            script: None,
            descriptor,
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

                    // A script module's knobs are declared by the program that
                    // hasn't been installed yet, so its params can't be applied
                    // until after the script loop below (the descriptor + knob
                    // store don't exist here). Every other module applies params
                    // first, so a Mod Matrix routing exists before its script.
                    let is_script_module = module_type.script_is_audio_rate()
                        || module_type.script_uses_control_ports();
                    if !is_script_module {
                        for (param_name, value) in &module_state.parameters {
                            if let Err(e) =
                                self.set_parameter(instrument_id, module_id, param_name, value)
                            {
                                result
                                    .errors
                                    .push(format!("{} param '{}': {e}", module_id, param_name));
                            }
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
                        let slot = match parse_mod_slot_key(slot_key) {
                            Ok(slot) => slot,
                            Err(msg) => {
                                result.errors.push(format!("{module_id} script: {msg}"));
                                continue;
                            }
                        };
                        // The Script module is now a single program (slot 1). A
                        // legacy multi-slot patch keeps slot 1 and drops the rest
                        // with a note — graceful migration, no panic (the engine's
                        // set_script would silently no-op the higher slots anyway).
                        if matches!(
                            module_id.module_type,
                            ModuleType::Script | ModuleType::AudioScript
                        ) && slot != 0
                        {
                            result.errors.push(format!(
                                "{module_id} slot {slot_key} script dropped: this module has \
                                 one program (slot 1 only)"
                            ));
                            continue;
                        }
                        if let Err(e) = self.set_mod_script(instrument_id, module_id, slot, source)
                        {
                            result
                                .errors
                                .push(format!("{module_id} slot {slot_key} script: {e}"));
                        }
                    }

                    // Now that the script is installed (its knobs exist in the
                    // descriptor + store), apply a script module's saved knob
                    // values — deferred from the bulk restore above (§3.B).
                    if is_script_module {
                        for (param_name, value) in &module_state.parameters {
                            if let Err(e) =
                                self.set_parameter(instrument_id, module_id, param_name, value)
                            {
                                result
                                    .errors
                                    .push(format!("{} knob '{}': {e}", module_id, param_name));
                            }
                        }
                    }

                    // Restore the per-instance description into the engine
                    // mirror so subsequent MCP/GUI reads see the loaded intent.
                    if !module_state.description.is_empty()
                        && let Err(e) = self.set_module_description(
                            instrument_id,
                            module_id,
                            Some(&module_state.description),
                        )
                    {
                        result.errors.push(format!("{module_id} description: {e}"));
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

        // Add connections (source port migrated for legacy stereo-out names —
        // see `migrate_stereo_out_port`).
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
                migrate_stereo_out_port(from_id.module_type, &conn.from.1),
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

/// Migrate a connection's source port name when loading a patch.
///
/// The Amplifier and Stereo Output modules renamed their stereo outputs
/// `"left"`/`"right"` → `"out_l"`/`"out_r"` for consistency with every other
/// stereo module. Projects/instruments saved before that change still reference
/// the old names; this remaps them on load so their connections survive. The
/// remap is scoped to those two module types so no other port named "left"/
/// "right" (e.g. effect-chain modules) is affected, and canonical names pass
/// through unchanged.
pub(crate) fn migrate_stereo_out_port(module_type: ModuleType, port: &str) -> PortName {
    match (module_type, port) {
        (ModuleType::Amplifier | ModuleType::StereoOutput, "left") => PortName::OUT_L,
        (ModuleType::Amplifier | ModuleType::StereoOutput, "right") => PortName::OUT_R,
        _ => PortName::intern(port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_engine::SynthEngine;

    /// Legacy "left"/"right" stereo-output port names are remapped to
    /// "out_l"/"out_r" on load — but only for Amplifier / Stereo Output, and
    /// canonical / unrelated names pass through untouched.
    #[test]
    fn migrates_legacy_stereo_out_ports() {
        assert_eq!(
            migrate_stereo_out_port(ModuleType::Amplifier, "left"),
            PortName::OUT_L
        );
        assert_eq!(
            migrate_stereo_out_port(ModuleType::Amplifier, "right"),
            PortName::OUT_R
        );
        assert_eq!(
            migrate_stereo_out_port(ModuleType::StereoOutput, "left"),
            PortName::OUT_L
        );
        // Canonical names are unchanged.
        assert_eq!(
            migrate_stereo_out_port(ModuleType::Amplifier, "out_l"),
            PortName::OUT_L
        );
        // "left" on any other module type is NOT a stereo-out alias — left as-is.
        assert_eq!(
            migrate_stereo_out_port(ModuleType::Reverb, "left"),
            PortName::intern("left")
        );
        // An unrelated port passes through.
        assert_eq!(
            migrate_stereo_out_port(ModuleType::Amplifier, "out"),
            PortName::OUT
        );
    }

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

    /// A legacy multi-slot Script patch loads best-effort: slot 1 (the single
    /// program) installs, and any higher slot is dropped with a note — graceful
    /// migration to the one-program Script module, no panic.
    #[test]
    fn apply_patch_migrates_legacy_multislot_script() {
        use crate::patch::{ModuleBuilder, Patch};
        let (_engine, session) = test_session();
        session
            .add_instrument_with_id(InstrumentId::FIRST, "T")
            .expect("add instrument");

        let mut patch = Patch::new("Legacy");
        let mut scr = ModuleBuilder::new(1, ModuleType::Script).build();
        scr.scripts
            .insert("1".to_string(), "out1 = in1".to_string());
        scr.scripts
            .insert("2".to_string(), "out2 = in2".to_string());
        patch.add_module(scr);
        let result = session.apply_patch(InstrumentId::FIRST, &patch);

        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("slot 2") && e.contains("dropped")),
            "legacy slot 2 must be dropped with a note: {:?}",
            result.errors
        );
        // Only the slot-2 drop note — slot 1 installs without an error of its own.
        assert!(
            !result.errors.iter().any(|e| e.contains("slot 1 script:")),
            "slot 1 must install cleanly: {:?}",
            result.errors
        );

        let mut patch = Patch::new("Legacy audio");
        let mut asc = ModuleBuilder::new(1, ModuleType::AudioScript).build();
        asc.scripts.insert("1".to_string(), "out = in".to_string());
        asc.scripts.insert("2".to_string(), "out = in".to_string());
        patch.add_module(asc);
        let result = session.apply_patch(InstrumentId::FIRST, &patch);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.contains("asc-1") && e.contains("slot 2") && e.contains("dropped")),
            "legacy AudioScript slot 2 must be dropped with a note: {:?}",
            result.errors
        );
    }

    /// Installing a program that declares a `param` registers the knob into the
    /// session-registry descriptor (feeds the GUI / MCP discovery / save), and
    /// clearing the program reverts the descriptor to its knob-free base.
    #[test]
    fn set_mod_script_registers_and_clears_knob_descriptor() {
        let (_engine, session) = test_session();
        session
            .add_instrument_with_id(InstrumentId::FIRST, "T")
            .expect("add instrument");
        let (scr, _) = session
            .add_module(InstrumentId::FIRST, ModuleType::Script)
            .expect("add scr");

        session
            .set_mod_script(
                InstrumentId::FIRST,
                scr,
                0,
                "param drive = 0.5\nout1 = in1 * drive",
            )
            .expect("install script with a knob");
        let desc = session
            .module_descriptor(InstrumentId::FIRST, scr)
            .expect("registered descriptor");
        assert!(
            desc.parameters.iter().any(|p| p.type_id == "drive"),
            "knob 'drive' must appear in the registered descriptor"
        );

        session
            .clear_mod_script(InstrumentId::FIRST, scr, 0)
            .expect("clear script");
        let desc = session
            .module_descriptor(InstrumentId::FIRST, scr)
            .expect("registered descriptor");
        assert!(
            !desc.parameters.iter().any(|p| p.type_id == "drive"),
            "clearing the program must drop the knob from the descriptor"
        );
    }

    /// The compile dialect follows the target module type: an `AudioScript`
    /// program (audio-rate grammar — `in`, multi-out) compiles for an `AudioScript`
    /// module but is a compile error for a control-rate module, and vice-versa.
    #[test]
    fn mod_script_dialect_follows_module_type() {
        // Audio-rate grammar compiles only in the audio dialect.
        assert!(compile_mod_script("out = tanh(in * 4)", true, false).is_ok());
        assert!(compile_mod_script("out = tanh(in * 4)", false, false).is_err());
        assert!(compile_mod_script("out.left = in_l\nout.right = in_r", true, false).is_ok());

        // A plain control-rate program compiles in every dialect.
        assert!(compile_mod_script("out = velocity * 0.5", false, false).is_ok());
        assert!(compile_mod_script("out = velocity * 0.5", true, false).is_ok());
        assert!(compile_mod_script("out = velocity * 0.5", false, true).is_ok());

        // Control-ports grammar (in1..in4 / out1..out4) compiles only with
        // control_ports; it is a compile error in the plain control dialect.
        assert!(compile_mod_script("out1 = in1 * in2", false, true).is_ok());
        assert!(compile_mod_script("out1 = in1 * in2", false, false).is_err());

        // set_mod_script picks the dialect from the module type: the same audio
        // program is accepted for an AudioScript module, rejected for the Mod Matrix.
        let (_engine, session) = test_session();
        let asc = ModuleId::new(ModuleType::AudioScript, 1);
        let mmx = ModuleId::new(ModuleType::ModMatrix, 1);
        assert!(
            session
                .set_mod_script(InstrumentId::FIRST, asc, 0, "out = tanh(in * 4)")
                .is_ok()
        );
        assert!(matches!(
            session.set_mod_script(InstrumentId::FIRST, mmx, 0, "out = tanh(in * 4)"),
            Err(SessionError::ScriptCompile(_))
        ));
    }
}
